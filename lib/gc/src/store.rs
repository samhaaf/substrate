//! SQLite store for the garbage collector (its own `gc.db` file).
//!
//! All access is serialized through a `Mutex<Connection>` (SQLite ops are fast
//! and bounded). The schema is applied idempotently on [`GcStore::open`].

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection, Row};

use crate::config::DirPolicy;

/// Default TTL (7 days, in seconds) used when an entry has no `ttl_override_secs`.
pub const DEFAULT_TTL_SECS: i64 = 604_800;

/// A row of the `gc_dirs` table — a registered managed directory.
#[derive(Debug, Clone)]
pub struct GcDirRow {
    pub root: String,
    pub policy: DirPolicy,
    pub registered_at: i64,
    pub last_swept_at: Option<i64>,
}

/// A row of the `gc_entries` table — a managed file or directory.
#[derive(Debug, Clone)]
pub struct GcEntryRow {
    pub path: String,
    pub dir_root: String,
    /// `"file"` or `"directory"`.
    pub kind: String,
    pub size_bytes: u64,
    pub registered_at: i64,
    pub last_touched_at: i64,
    pub touch_count: u64,
    pub lock_expires_at: Option<i64>,
    pub ttl_override_secs: Option<u64>,
    pub recovery_hint: Option<String>,
    /// `"present"`, `"evicting"`, or `"absent"`.
    pub state: String,
}

const SCHEMA_SQL: &str = "\
PRAGMA journal_mode=WAL;

CREATE TABLE IF NOT EXISTS gc_dirs (
    root TEXT PRIMARY KEY,
    policy_json TEXT NOT NULL,
    registered_at INTEGER NOT NULL,
    last_swept_at INTEGER
);

CREATE TABLE IF NOT EXISTS gc_entries (
    path TEXT PRIMARY KEY,
    dir_root TEXT NOT NULL,
    kind TEXT NOT NULL,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    registered_at INTEGER NOT NULL,
    last_touched_at INTEGER NOT NULL,
    touch_count INTEGER NOT NULL DEFAULT 0,
    lock_expires_at INTEGER,
    ttl_override_secs INTEGER,
    recovery_hint TEXT,
    state TEXT NOT NULL DEFAULT 'present'
);
CREATE INDEX IF NOT EXISTS idx_entries_dir ON gc_entries(dir_root);
CREATE INDEX IF NOT EXISTS idx_entries_lru ON gc_entries(dir_root, last_touched_at);
";

/// The garbage-collector SQLite store. Cheap to clone (wraps an `Arc`).
#[derive(Clone)]
pub struct GcStore {
    conn: Arc<Mutex<Connection>>,
}

fn map_dir_row(row: &Row<'_>) -> rusqlite::Result<(String, String, i64, Option<i64>)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}

fn map_entry_row(row: &Row<'_>) -> rusqlite::Result<GcEntryRow> {
    Ok(GcEntryRow {
        path: row.get(0)?,
        dir_root: row.get(1)?,
        kind: row.get(2)?,
        size_bytes: row.get::<_, i64>(3)? as u64,
        registered_at: row.get(4)?,
        last_touched_at: row.get(5)?,
        touch_count: row.get::<_, i64>(6)? as u64,
        lock_expires_at: row.get(7)?,
        ttl_override_secs: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        recovery_hint: row.get(9)?,
        state: row.get(10)?,
    })
}

const ENTRY_COLS: &str = "path, dir_root, kind, size_bytes, registered_at, \
last_touched_at, touch_count, lock_expires_at, ttl_override_secs, recovery_hint, state";

impl GcStore {
    /// Open (or create) the db at `path` and apply the schema.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening gc db {}", path.display()))?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.apply_schema()?;
        Ok(store)
    }

    /// Open an in-memory store (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().context("opening in-memory gc db")?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.apply_schema()?;
        Ok(store)
    }

    fn apply_schema(&self) -> Result<()> {
        let conn = self.lock()?;
        conn.execute_batch(SCHEMA_SQL).context("applying gc schema")?;
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| anyhow!("gc store mutex poisoned"))
    }

    // ── Directory management ───────────────────────────────────────────────

    /// Upsert a managed directory and its policy.
    pub fn register_dir(&self, root: &str, policy: &DirPolicy) -> Result<()> {
        let policy_json = serde_json::to_string(policy).context("serializing dir policy")?;
        let now = chrono::Utc::now().timestamp();
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO gc_dirs (root, policy_json, registered_at, last_swept_at)
             VALUES (?1, ?2, ?3, NULL)
             ON CONFLICT(root) DO UPDATE SET policy_json = excluded.policy_json",
            params![root, policy_json, now],
        )?;
        Ok(())
    }

    /// Delete a directory registration and all of its entries.
    pub fn deregister_dir(&self, root: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute("DELETE FROM gc_entries WHERE dir_root = ?1", params![root])?;
        conn.execute("DELETE FROM gc_dirs WHERE root = ?1", params![root])?;
        Ok(())
    }

    /// List all registered directories.
    pub fn list_dirs(&self) -> Result<Vec<GcDirRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT root, policy_json, registered_at, last_swept_at FROM gc_dirs",
        )?;
        let rows = stmt.query_map([], map_dir_row)?;
        let mut out = Vec::new();
        for r in rows {
            let (root, policy_json, registered_at, last_swept_at) = r?;
            let policy: DirPolicy =
                serde_json::from_str(&policy_json).context("parsing stored dir policy")?;
            out.push(GcDirRow {
                root,
                policy,
                registered_at,
                last_swept_at,
            });
        }
        Ok(out)
    }

    /// Record the time a directory was last swept.
    pub fn mark_swept(&self, root: &str, now_secs: i64) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE gc_dirs SET last_swept_at = ?2 WHERE root = ?1",
            params![root, now_secs],
        )?;
        Ok(())
    }

    // ── Entry management ───────────────────────────────────────────────────

    /// Insert or update an entry (upsert by path).
    pub fn upsert_entry(&self, entry: &GcEntryRow) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "INSERT INTO gc_entries
             (path, dir_root, kind, size_bytes, registered_at, last_touched_at,
              touch_count, lock_expires_at, ttl_override_secs, recovery_hint, state)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(path) DO UPDATE SET
                dir_root          = excluded.dir_root,
                kind              = excluded.kind,
                size_bytes        = excluded.size_bytes,
                last_touched_at   = excluded.last_touched_at,
                lock_expires_at   = excluded.lock_expires_at,
                ttl_override_secs = excluded.ttl_override_secs,
                recovery_hint     = excluded.recovery_hint,
                state             = excluded.state",
            params![
                entry.path,
                entry.dir_root,
                entry.kind,
                entry.size_bytes as i64,
                entry.registered_at,
                entry.last_touched_at,
                entry.touch_count as i64,
                entry.lock_expires_at,
                entry.ttl_override_secs.map(|v| v as i64),
                entry.recovery_hint,
                entry.state,
            ],
        )?;
        Ok(())
    }

    /// Mark an entry as touched: set `last_touched_at = now`, bump `touch_count`.
    pub fn touch(&self, path: &str) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.lock()?;
        conn.execute(
            "UPDATE gc_entries
             SET last_touched_at = ?2, touch_count = touch_count + 1
             WHERE path = ?1",
            params![path, now],
        )?;
        Ok(())
    }

    /// Set a lock that expires at `expires_at` (unix secs).
    pub fn set_lock(&self, path: &str, expires_at: i64) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE gc_entries SET lock_expires_at = ?2 WHERE path = ?1",
            params![path, expires_at],
        )?;
        Ok(())
    }

    /// Clear any lock on an entry.
    pub fn clear_lock(&self, path: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE gc_entries SET lock_expires_at = NULL WHERE path = ?1",
            params![path],
        )?;
        Ok(())
    }

    /// Fetch a single entry by path.
    pub fn get_entry(&self, path: &str) -> Result<Option<GcEntryRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLS} FROM gc_entries WHERE path = ?1"
        ))?;
        let mut rows = stmt.query_map(params![path], map_entry_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// List all entries belonging to a directory.
    pub fn list_entries(&self, dir_root: &str) -> Result<Vec<GcEntryRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLS} FROM gc_entries WHERE dir_root = ?1"
        ))?;
        let rows = stmt.query_map(params![dir_root], map_entry_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// List every entry across all managed directories.
    pub fn list_all_entries(&self) -> Result<Vec<GcEntryRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!("SELECT {ENTRY_COLS} FROM gc_entries"))?;
        let rows = stmt.query_map([], map_entry_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// LRU eviction candidates for a directory: present, not currently locked,
    /// ordered oldest-touched first.
    pub fn lru_eviction_candidates(
        &self,
        dir_root: &str,
        now_secs: i64,
    ) -> Result<Vec<GcEntryRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLS} FROM gc_entries
             WHERE dir_root = ?1
               AND state = 'present'
               AND (lock_expires_at IS NULL OR lock_expires_at < ?2)
             ORDER BY last_touched_at ASC"
        ))?;
        let rows = stmt.query_map(params![dir_root, now_secs], map_entry_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Entries whose TTL has elapsed: present, not locked, and
    /// `last_touched_at + ttl < now` (ttl defaults to 7 days).
    pub fn expired_entries(&self, now_secs: i64) -> Result<Vec<GcEntryRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {ENTRY_COLS} FROM gc_entries
             WHERE state = 'present'
               AND (lock_expires_at IS NULL OR lock_expires_at < ?1)
               AND last_touched_at + COALESCE(ttl_override_secs, {DEFAULT_TTL_SECS}) < ?1"
        ))?;
        let rows = stmt.query_map(params![now_secs], map_entry_row)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    /// Total size of present entries in a directory.
    pub fn total_size(&self, dir_root: &str) -> Result<u64> {
        let conn = self.lock()?;
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM gc_entries
             WHERE dir_root = ?1 AND state = 'present'",
            params![dir_root],
            |row| row.get(0),
        )?;
        Ok(total as u64)
    }

    /// Set the `state` of an entry (`present` / `evicting` / `absent`).
    pub fn update_state(&self, path: &str, state: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE gc_entries SET state = ?2 WHERE path = ?1",
            params![path, state],
        )?;
        Ok(())
    }

    /// Rename a single entry's path.
    pub fn rename_path(&self, old: &str, new: &str) -> Result<()> {
        let conn = self.lock()?;
        conn.execute(
            "UPDATE gc_entries SET path = ?2 WHERE path = ?1",
            params![old, new],
        )?;
        Ok(())
    }

    /// Rename every path and dir_root under `old_prefix` to `new_prefix`
    /// (used when a managed directory is moved). Also moves the dir row itself.
    pub fn rename_prefix(&self, old_prefix: &str, new_prefix: &str) -> Result<()> {
        let conn = self.lock()?;
        let like = format!("{old_prefix}%");
        let old_len = old_prefix.len() as i64;
        // Update entry paths.
        conn.execute(
            "UPDATE gc_entries
             SET path = ?2 || SUBSTR(path, ?3 + 1)
             WHERE path = ?1 OR path LIKE ?4",
            params![old_prefix, new_prefix, old_len, like],
        )?;
        // Update entry dir_roots.
        conn.execute(
            "UPDATE gc_entries
             SET dir_root = ?2 || SUBSTR(dir_root, ?3 + 1)
             WHERE dir_root = ?1 OR dir_root LIKE ?4",
            params![old_prefix, new_prefix, old_len, like],
        )?;
        // Update dir registrations.
        conn.execute(
            "UPDATE gc_dirs
             SET root = ?2 || SUBSTR(root, ?3 + 1)
             WHERE root = ?1 OR root LIKE ?4",
            params![old_prefix, new_prefix, old_len, like],
        )?;
        Ok(())
    }
}
