//! Eviction logic: making room, sweeping expired entries, and evicting one item.

use anyhow::{anyhow, Result};
use tokio::sync::broadcast;

use crate::events::{EvictionReason, GcEvent};
use crate::reclaim::Reclaimer;
use crate::store::{GcEntryRow, GcStore};

/// Emit a GcEvent if a sender is provided, ignoring errors (no subscribers).
fn emit(tx: Option<&broadcast::Sender<GcEvent>>, event: GcEvent) {
    if let Some(tx) = tx {
        let _ = tx.send(event);
    }
}

/// Summary of a [`sweep`] pass.
#[derive(Debug, Default, Clone)]
pub struct SweepReport {
    /// Number of entries evicted because their TTL expired.
    pub expired_evicted: u64,
    /// Number of entries evicted to bring a directory under its size budget.
    pub budget_evicted: u64,
    /// Total bytes freed across both phases.
    pub bytes_freed: u64,
    /// Per-entry errors encountered (the sweep continues past failures).
    pub errors: Vec<String>,
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Reclaim one entry: mark evicting, reclaim on disk, mark absent.
fn reclaim_entry(
    store: &GcStore,
    reclaimer: &dyn Reclaimer,
    entry: &GcEntryRow,
    reason: EvictionReason,
    tx: Option<&broadcast::Sender<GcEvent>>,
) -> Result<()> {
    emit(tx, GcEvent::EntryEvicting {
        path: entry.path.clone(),
        reason,
    });
    store.update_state(&entry.path, "evicting")?;
    match reclaimer.reclaim(entry) {
        Ok(()) => {
            store.update_state(&entry.path, "absent")?;
            emit(tx, GcEvent::EntryEvicted {
                path: entry.path.clone(),
                bytes_freed: entry.size_bytes,
                recovery_hint: entry.recovery_hint.clone(),
            });
            Ok(())
        }
        Err(e) => {
            emit(tx, GcEvent::EntryEvictionFailed {
                path: entry.path.clone(),
                error: e.to_string(),
            });
            Err(e)
        }
    }
}

/// Evict LRU candidates from `dir` until at least `bytes_needed` bytes of
/// headroom exist beneath the directory's budget.
///
/// Returns the number of bytes freed (0 if there was already room).
/// Errors with `DiskBudgetExceeded` if candidates are exhausted first.
pub fn make_room(
    store: &GcStore,
    reclaimer: &dyn Reclaimer,
    dir: &str,
    bytes_needed: u64,
    tx: Option<&broadcast::Sender<GcEvent>>,
) -> Result<u64> {
    // Determine the dir's max budget.
    let max_size = store
        .list_dirs()?
        .into_iter()
        .find(|d| d.root == dir)
        .map(|d| d.policy.max_size_bytes)
        .ok_or_else(|| anyhow!("directory not registered: {dir}"))?;

    let total = store.total_size(dir)?;

    // Headroom already available beneath the budget.
    let available = max_size.saturating_sub(total);
    if available >= bytes_needed {
        return Ok(0);
    }

    let mut to_free = bytes_needed - available;
    let mut freed = 0u64;

    let candidates = store.lru_eviction_candidates(dir, now_secs())?;
    for entry in candidates {
        if freed >= to_free {
            break;
        }
        reclaim_entry(store, reclaimer, &entry, EvictionReason::BudgetPressure, tx)?;
        freed += entry.size_bytes;
        // `to_free` is fixed; loop until freed covers it.
        let _ = &mut to_free;
    }

    if freed < to_free {
        return Err(anyhow!(
            "DiskBudgetExceeded: need {bytes_needed}, freed {freed}"
        ));
    }

    Ok(freed)
}

/// Sweep all registered directories.
///
/// Phase 1 evicts TTL-expired entries. Phase 2 evicts LRU entries from any
/// directory that is over its size budget. Reclaim failures are logged and
/// recorded in [`SweepReport::errors`] rather than aborting the sweep.
pub fn sweep(
    store: &GcStore,
    reclaimer: &dyn Reclaimer,
    tx: Option<&broadcast::Sender<GcEvent>>,
) -> Result<SweepReport> {
    let mut report = SweepReport::default();
    let now = now_secs();

    let dirs = store.list_dirs()?;

    // ── Phase 1: TTL expiry (across all dirs) ──────────────────────────────
    let expired = store.expired_entries(now)?;
    for entry in expired {
        match reclaim_entry(store, reclaimer, &entry, EvictionReason::TtlExpired, tx) {
            Ok(()) => {
                report.expired_evicted += 1;
                report.bytes_freed += entry.size_bytes;
            }
            Err(e) => {
                let msg = format!("expired-evict {} failed: {e}", entry.path);
                tracing::warn!("{msg}");
                report.errors.push(msg);
            }
        }
    }

    // ── Phase 2: size budget per dir ───────────────────────────────────────
    for dir in &dirs {
        let total = store.total_size(&dir.root)?;
        let max = dir.policy.max_size_bytes;
        if total <= max {
            store.mark_swept(&dir.root, now)?;
            continue;
        }
        let overflow = total - max;

        emit(tx, GcEvent::BudgetExceeded {
            dir: dir.root.clone(),
            current_bytes: total,
            max_bytes: max,
        });

        // Evict LRU candidates until the overflow is cleared.
        let candidates = match store.lru_eviction_candidates(&dir.root, now) {
            Ok(c) => c,
            Err(e) => {
                report.errors.push(format!(
                    "listing candidates for {} failed: {e}",
                    dir.root
                ));
                continue;
            }
        };
        let mut freed = 0u64;
        for entry in candidates {
            if freed >= overflow {
                break;
            }
            match reclaim_entry(store, reclaimer, &entry, EvictionReason::BudgetPressure, tx) {
                Ok(()) => {
                    report.budget_evicted += 1;
                    report.bytes_freed += entry.size_bytes;
                    freed += entry.size_bytes;
                }
                Err(e) => {
                    let msg = format!("budget-evict {} failed: {e}", entry.path);
                    tracing::warn!("{msg}");
                    report.errors.push(msg);
                }
            }
        }

        store.mark_swept(&dir.root, now)?;
    }

    Ok(report)
}

/// Evict a single entry by path. Fails if the entry is currently locked.
pub fn evict_one(
    store: &GcStore,
    reclaimer: &dyn Reclaimer,
    path: &str,
    tx: Option<&broadcast::Sender<GcEvent>>,
) -> Result<()> {
    let entry = store
        .get_entry(path)?
        .ok_or_else(|| anyhow!("entry not found: {path}"))?;

    let now = now_secs();
    if let Some(expires) = entry.lock_expires_at {
        if expires >= now {
            return Err(anyhow!("entry is locked: {path}"));
        }
    }

    reclaim_entry(store, reclaimer, &entry, EvictionReason::Forced, tx)
}
