//! Reclaimers: pluggable strategies for actually freeing an evicted entry.

use anyhow::Result;

use crate::store::GcEntryRow;

/// A strategy for reclaiming (freeing) a managed entry once it is evicted.
pub trait Reclaimer: Send + Sync {
    /// Reclaim the on-disk resource backing `entry`.
    fn reclaim(&self, entry: &GcEntryRow) -> Result<()>;
}

/// Reclaimer that deletes the entry from the filesystem.
pub struct DeleteReclaimer;

impl Reclaimer for DeleteReclaimer {
    fn reclaim(&self, entry: &GcEntryRow) -> Result<()> {
        if entry.kind == "directory" {
            std::fs::remove_dir_all(&entry.path).map_err(|e| anyhow::anyhow!(e))?;
        } else {
            std::fs::remove_file(&entry.path).map_err(|e| anyhow::anyhow!(e))?;
        }
        if let Some(hint) = &entry.recovery_hint {
            tracing::info!("Evicted {}. Recovery hint: {}", entry.path, hint);
        }
        Ok(())
    }
}

/// Stub reclaimer that would migrate entries elsewhere; falls back to delete.
pub struct MigrateReclaimer;

impl Reclaimer for MigrateReclaimer {
    fn reclaim(&self, entry: &GcEntryRow) -> Result<()> {
        tracing::warn!(
            "MigrateReclaimer not implemented, falling back to delete for {}",
            entry.path
        );
        DeleteReclaimer.reclaim(entry)
    }
}
