//! # substrate-gc
//!
//! A filesystem garbage collector. It tracks managed files and directories,
//! enforces TTL and size budgets, and evicts least-recently-used (or FIFO)
//! items when a directory exceeds its budget.
//!
//! ## Model
//!
//! - A **managed directory** is registered via [`GcService::register_dir`]. It
//!   gets a `.gc/` subfolder holding a [`config::DirPolicy`] (`config.toml`).
//! - **Entries** (files or directories) inside it are registered via
//!   [`GcService::register_entry`]. Each entry can have a per-item override
//!   (`.gc/{name}.toml`) and is tracked in the [`store::GcStore`] SQLite db.
//! - [`GcService::touch`] / [`GcService::lock`] update LRU and lock state.
//! - [`GcService::sweep`] reclaims TTL-expired and over-budget entries using a
//!   [`reclaim::Reclaimer`].

pub mod config;
pub mod eviction;
pub mod events;
pub mod reclaim;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use tokio::sync::broadcast;

pub use config::{DirPolicy, EvictionPolicy, ItemConfig, OnFullAction, UnitMode};
pub use eviction::{make_room, sweep, SweepReport};
pub use events::{EvictionReason, GcEvent};
pub use reclaim::{DeleteReclaimer, MigrateReclaimer, Reclaimer};
pub use store::{GcDirRow, GcEntryRow, GcStore};

/// Whether a managed entry is a file or a directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Directory,
}

impl EntryKind {
    fn as_str(self) -> &'static str {
        match self {
            EntryKind::File => "file",
            EntryKind::Directory => "directory",
        }
    }
}

/// Service configuration.
pub struct GcConfig {
    /// Where to store the `gc.db` SQLite file.
    pub db_path: PathBuf,
    /// Interval (secs) for the background sweep loop. Default 3600.
    pub sweep_interval_secs: u64,
}

impl Default for GcConfig {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("gc.db"),
            sweep_interval_secs: 3600,
        }
    }
}

/// The garbage-collector service: the public entry point of this crate.
pub struct GcService {
    store: Arc<GcStore>,
    reclaimer: Arc<dyn Reclaimer>,
    event_tx: broadcast::Sender<GcEvent>,
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Compute the on-disk size of a path: file length, or recursive sum for a dir.
fn compute_size(path: &Path) -> Result<u64> {
    let meta = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)
        .with_context(|| format!("reading dir {}", path.display()))?
    {
        let entry = entry?;
        let p = entry.path();
        // Skip the .gc control folder.
        if p.file_name().map(|n| n == ".gc").unwrap_or(false) {
            continue;
        }
        let md = entry.metadata()?;
        if md.is_dir() {
            total += compute_size(&p)?;
        } else {
            total += md.len();
        }
    }
    Ok(total)
}

impl GcService {
    /// Construct a service: open/create the db and wire up the reclaimer.
    pub fn new(config: GcConfig, reclaimer: Box<dyn Reclaimer>) -> Result<Self> {
        let store = GcStore::open(&config.db_path)?;
        let (event_tx, _) = broadcast::channel(256);
        Ok(Self {
            store: Arc::new(store),
            reclaimer: Arc::from(reclaimer),
            event_tx,
        })
    }

    /// Construct a service backed by an in-memory db (tests).
    pub fn new_in_memory(reclaimer: Box<dyn Reclaimer>) -> Result<Self> {
        let store = GcStore::open_in_memory()?;
        let (event_tx, _) = broadcast::channel(256);
        Ok(Self {
            store: Arc::new(store),
            reclaimer: Arc::from(reclaimer),
            event_tx,
        })
    }

    /// Subscribe to GC events. Each subscriber receives a clone of every event
    /// emitted after the subscription is created.
    pub fn subscribe_events(&self) -> broadcast::Receiver<GcEvent> {
        self.event_tx.subscribe()
    }

    /// Emit an event, ignoring errors if no subscribers are listening.
    fn emit(&self, event: GcEvent) {
        let _ = self.event_tx.send(event);
    }

    fn gc_dir_for(path: &Path) -> PathBuf {
        path.join(".gc")
    }

    fn path_str(path: &Path) -> Result<String> {
        path.to_str()
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("non-utf8 path: {}", path.display()))
    }

    // ── Directory management ───────────────────────────────────────────────

    /// Register (or re-register) a managed directory. Creates `path/.gc/` and a
    /// `config.toml` (default policy if none exists), then records it in the db.
    pub fn register_dir(&self, path: &Path) -> Result<()> {
        let gc_dir = Self::gc_dir_for(path);
        std::fs::create_dir_all(&gc_dir)
            .with_context(|| format!("creating {}", gc_dir.display()))?;
        let policy = DirPolicy::load(&gc_dir)?;
        // Persist the (possibly defaulted) policy so config.toml exists.
        policy.save(&gc_dir)?;
        let root = Self::path_str(path)?;
        self.store.register_dir(&root, &policy)?;
        self.emit(GcEvent::DirRegistered {
            root,
            max_size_bytes: policy.max_size_bytes,
            default_ttl_secs: policy.default_ttl_secs,
        });
        Ok(())
    }

    /// Deregister a managed directory and forget all of its entries.
    pub fn deregister_dir(&self, path: &Path) -> Result<()> {
        let root = Self::path_str(path)?;
        self.store.deregister_dir(&root)
    }

    /// List all registered directories.
    pub fn list_dirs(&self) -> Result<Vec<GcDirRow>> {
        self.store.list_dirs()
    }

    /// Total bytes currently occupied by `present` entries under `dir_root`.
    pub fn dir_used_bytes(&self, dir_root: &str) -> Result<u64> {
        self.store.total_size(dir_root)
    }

    // ── Entry management ───────────────────────────────────────────────────

    /// Register an entry under its managed directory. Computes its size, reads
    /// any per-item `.gc/{name}.toml` override, and upserts it into the db.
    ///
    /// The entry's `dir_root` is its parent directory.
    pub fn register_entry(
        &self,
        path: &Path,
        kind: EntryKind,
        recovery_hint: Option<String>,
    ) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("entry has no parent: {}", path.display()))?;
        let dir_root = Self::path_str(parent)?;
        let path_str = Self::path_str(path)?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("entry has no file name: {}", path.display()))?;

        let size_bytes = compute_size(path)?;

        // Read per-item override, if present.
        let gc_dir = Self::gc_dir_for(parent);
        let item_cfg = ItemConfig::load(&gc_dir, name)?;
        let ttl_override_secs = item_cfg.as_ref().and_then(|c| c.ttl_secs);
        let hint = recovery_hint.or_else(|| item_cfg.and_then(|c| c.recovery_hint));

        let now = now_secs();
        let entry = GcEntryRow {
            path: path_str,
            dir_root,
            kind: kind.as_str().to_string(),
            size_bytes,
            registered_at: now,
            last_touched_at: now,
            touch_count: 0,
            lock_expires_at: None,
            ttl_override_secs,
            recovery_hint: hint,
            state: "present".to_string(),
        };
        self.store.upsert_entry(&entry)?;
        self.emit(GcEvent::EntryRegistered {
            path: entry.path,
            kind: entry.kind,
            size_bytes: entry.size_bytes,
            recovery_hint: entry.recovery_hint,
        });
        Ok(())
    }

    // ── Touch and lock ─────────────────────────────────────────────────────

    /// Mark an entry as recently used (refreshes LRU and TTL).
    pub fn touch(&self, path: &str) -> Result<()> {
        self.store.touch(path)?;
        // Fetch the updated touch_count to include in the event.
        let touch_count = self
            .store
            .get_entry(path)?
            .map(|e| e.touch_count)
            .unwrap_or(0);
        self.emit(GcEvent::EntryTouched {
            path: path.to_string(),
            touch_count,
        });
        Ok(())
    }

    /// Lock an entry against eviction until `now + ttl_secs`.
    pub fn lock(&self, path: &str, ttl_secs: u64) -> Result<()> {
        let expires = now_secs() + ttl_secs as i64;
        self.store.set_lock(path, expires)?;
        self.emit(GcEvent::EntryLocked {
            path: path.to_string(),
            lock_expires_at: expires,
        });
        Ok(())
    }

    /// Remove any lock on an entry.
    pub fn unlock(&self, path: &str) -> Result<()> {
        self.store.clear_lock(path)?;
        self.emit(GcEvent::EntryUnlocked {
            path: path.to_string(),
        });
        Ok(())
    }

    // ── Operations ─────────────────────────────────────────────────────────

    /// Evict LRU entries from `dir` until `bytes_needed` of headroom exists.
    pub fn make_room(&self, dir: &str, bytes_needed: u64) -> Result<u64> {
        let freed =
            eviction::make_room(&self.store, self.reclaimer.as_ref(), dir, bytes_needed, Some(&self.event_tx))?;
        if freed > 0 {
            self.emit(GcEvent::MakeRoomCompleted {
                dir: dir.to_string(),
                bytes_freed: freed,
            });
        }
        Ok(freed)
    }

    /// Move a managed path on disk and update all registrations beneath it.
    pub fn move_path(&self, from: &Path, to: &Path) -> Result<()> {
        std::fs::rename(from, to)
            .with_context(|| format!("rename {} -> {}", from.display(), to.display()))?;
        let from_str = Self::path_str(from)?;
        let to_str = Self::path_str(to)?;
        self.store.rename_prefix(&from_str, &to_str)?;
        self.emit(GcEvent::PathMoved {
            from: from_str,
            to: to_str,
        });
        Ok(())
    }

    /// Run a full sweep (TTL expiry + budget enforcement).
    pub fn sweep(&self) -> Result<SweepReport> {
        let report = eviction::sweep(&self.store, self.reclaimer.as_ref(), Some(&self.event_tx))?;
        self.emit(GcEvent::SweepCompleted {
            expired_evicted: report.expired_evicted,
            budget_evicted: report.budget_evicted,
            bytes_freed: report.bytes_freed,
        });
        Ok(report)
    }

    /// Evict a single entry by path.
    pub fn evict(&self, path: &str) -> Result<()> {
        eviction::evict_one(&self.store, self.reclaimer.as_ref(), path, Some(&self.event_tx))
    }

    // ── Query ──────────────────────────────────────────────────────────────

    /// Look up a single entry.
    pub fn query(&self, path: &str) -> Result<Option<GcEntryRow>> {
        self.store.get_entry(path)
    }

    /// List all entries belonging to a directory.
    pub fn list_entries(&self, dir: &str) -> Result<Vec<GcEntryRow>> {
        self.store.list_entries(dir)
    }

    /// List every entry across all managed directories.
    pub fn list_all_entries(&self) -> Result<Vec<GcEntryRow>> {
        self.store.list_all_entries()
    }

    // ── Background sweep loop ───────────────────────────────────────────────

    /// Spawn a background task that sweeps every `interval_secs`.
    pub fn start_sweep_loop(
        self: Arc<Self>,
        interval_secs: u64,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                ticker.tick().await;
                match self.sweep() {
                    Ok(report) => {
                        tracing::info!(
                            expired = report.expired_evicted,
                            budget = report.budget_evicted,
                            bytes = report.bytes_freed,
                            errors = report.errors.len(),
                            "gc sweep complete"
                        );
                    }
                    Err(e) => tracing::error!("gc sweep failed: {e}"),
                }
            }
        })
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a service over an in-memory store with a DeleteReclaimer.
    fn service() -> GcService {
        GcService::new_in_memory(Box::new(DeleteReclaimer)).unwrap()
    }

    fn write_file(path: &Path, bytes: usize) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(&vec![b'x'; bytes]).unwrap();
    }

    /// Register a dir with a custom max_size_bytes budget.
    fn register_dir_with_budget(svc: &GcService, dir: &Path, max_size_bytes: u64) {
        let gc_dir = dir.join(".gc");
        std::fs::create_dir_all(&gc_dir).unwrap();
        let policy = DirPolicy {
            max_size_bytes,
            ..Default::default()
        };
        policy.save(&gc_dir).unwrap();
        svc.register_dir(dir).unwrap();
    }

    #[tokio::test]
    async fn test_register_and_touch() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let svc = service();
        svc.register_dir(dir).unwrap();

        let file = dir.join("model.bin");
        write_file(&file, 128);
        svc.register_entry(&file, EntryKind::File, None).unwrap();

        let file_str = file.to_str().unwrap();
        let before = svc.query(file_str).unwrap().unwrap();
        assert_eq!(before.touch_count, 0);
        assert_eq!(before.size_bytes, 128);

        // Force a measurable time difference.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        svc.touch(file_str).unwrap();

        let after = svc.query(file_str).unwrap().unwrap();
        assert_eq!(after.touch_count, 1);
        assert!(
            after.last_touched_at >= before.last_touched_at,
            "last_touched_at should advance: {} -> {}",
            before.last_touched_at,
            after.last_touched_at
        );
    }

    #[tokio::test]
    async fn test_lock_prevents_eviction() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let svc = service();
        register_dir_with_budget(&svc, dir, 100);

        let file = dir.join("big.bin");
        write_file(&file, 150);
        svc.register_entry(&file, EntryKind::File, None).unwrap();
        let file_str = file.to_str().unwrap();

        // Lock for an hour so it cannot be evicted.
        svc.lock(file_str, 3600).unwrap();

        // make_room cannot free anything -> error (no candidates).
        let dir_str = dir.to_str().unwrap();
        let res = svc.make_room(dir_str, 100);
        assert!(res.is_err(), "locked entry must block make_room");

        // File still on disk and still present.
        assert!(file.exists());
        let e = svc.query(file_str).unwrap().unwrap();
        assert_eq!(e.state, "present");
    }

    #[tokio::test]
    async fn test_expired_lock_allows_eviction() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let svc = service();
        register_dir_with_budget(&svc, dir, 100);

        let file = dir.join("expired.bin");
        write_file(&file, 150);
        svc.register_entry(&file, EntryKind::File, None).unwrap();
        let file_str = file.to_str().unwrap();

        // Lock with 0 ttl -> expires at "now", which is < now by the time we check.
        svc.lock(file_str, 0).unwrap();
        // Ensure clock moves past the lock expiry.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        let dir_str = dir.to_str().unwrap();
        let freed = svc.make_room(dir_str, 100).unwrap();
        assert_eq!(freed, 150, "expired-lock entry should be evicted");
        assert!(!file.exists());
        let e = svc.query(file_str).unwrap().unwrap();
        assert_eq!(e.state, "absent");
    }

    #[tokio::test]
    async fn test_make_room_lru_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let svc = service();
        register_dir_with_budget(&svc, dir, 1000);

        // Three 400B entries -> total 1200B, over budget by 200B.
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        let c = dir.join("c.bin");
        for p in [&a, &b, &c] {
            write_file(p, 400);
        }
        // Register + touch in order A, B, C with spacing so LRU is deterministic.
        for p in [&a, &b, &c] {
            svc.register_entry(p, EntryKind::File, None).unwrap();
            svc.touch(p.to_str().unwrap()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }

        let dir_str = dir.to_str().unwrap();
        let freed = svc.make_room(dir_str, 200).unwrap();
        assert_eq!(freed, 400, "should evict exactly one 400B entry");

        // A (oldest touch) evicted; B and C survive.
        assert!(!a.exists(), "A should be evicted");
        assert!(b.exists(), "B should survive");
        assert!(c.exists(), "C should survive");

        assert_eq!(svc.query(a.to_str().unwrap()).unwrap().unwrap().state, "absent");
        assert_eq!(svc.query(b.to_str().unwrap()).unwrap().unwrap().state, "present");
        assert_eq!(svc.query(c.to_str().unwrap()).unwrap().unwrap().state, "present");
    }

    #[tokio::test]
    async fn test_move_updates_registrations() {
        let tmp = tempfile::tempdir().unwrap();
        let old_dir = tmp.path().join("old");
        std::fs::create_dir_all(&old_dir).unwrap();
        let svc = service();
        svc.register_dir(&old_dir).unwrap();

        let model = old_dir.join("model");
        write_file(&model, 64);
        svc.register_entry(&model, EntryKind::File, None).unwrap();

        let new_dir = tmp.path().join("new");
        svc.move_path(&old_dir, &new_dir).unwrap();

        // Dir registration updated.
        let dirs = svc.list_dirs().unwrap();
        let new_dir_str = new_dir.to_str().unwrap();
        assert!(
            dirs.iter().any(|d| d.root == new_dir_str),
            "dir root should be updated to new path; got {:?}",
            dirs.iter().map(|d| &d.root).collect::<Vec<_>>()
        );
        assert!(
            !dirs.iter().any(|d| d.root == old_dir.to_str().unwrap()),
            "old dir root should be gone"
        );

        // Entry path updated to new_dir/model.
        let new_model_str = new_dir.join("model").to_str().unwrap().to_string();
        let e = svc.query(&new_model_str).unwrap();
        assert!(e.is_some(), "entry should be queryable under its new path");
        assert_eq!(e.unwrap().dir_root, new_dir_str);

        // Old entry path no longer present.
        let old_model_str = model.to_str().unwrap();
        assert!(svc.query(old_model_str).unwrap().is_none());

        // And the file actually moved on disk.
        assert!(new_dir.join("model").exists());
    }
}
