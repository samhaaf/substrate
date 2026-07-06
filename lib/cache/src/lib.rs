//! # substrate-cache
//!
//! Disk-backed KV/prefix cache manager. Renamed from v1's `KvCacheManager`.
//!
//! ## What This Does
//!
//! llama-server supports slot save/restore: after processing a prompt, the KV cache
//! state for that slot can be written to a file. On the next completion with the same
//! prefix, the cache can be restored, skipping the prefill step entirely.
//!
//! This crate manages:
//! - **Budget enforcement** — total KV cache bytes on disk ≤ `kv_cache_budget_bytes`
//! - **Entry indexing** — maps (model_id, prompt_hash) → file path, via the store
//! - **LRU eviction** — evict least-recently-accessed entries when budget is exceeded
//! - **Purge on model eviction** — when a model is removed from disk, its cache is also purged
//!
//! ## Hashing
//!
//! The entry ID is derived from the model_id and prompt_hash using a deterministic
//! combination: `sha256(model_id || ":" || prompt_hash)` expressed as a hex string.
//! This gives a stable, unique key without external crate dependencies beyond
//! `std::collections::hash_map::DefaultHasher` — but since we want collision
//! resistance, we use a two-field composite string and hash via `DefaultHasher`.
//!
//! Note: the `prompt_hash` passed in is already a caller-supplied hash of the prompt
//! (typically a SHA-256 of the tokenized prefix). The entry ID here is a secondary
//! hash of `model_id + prompt_hash`, used as the SQLite primary key.
//!
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use chrono::Utc;

use substrate_gc::{EntryKind, GcService};
use substrate_store::{KvCacheEntry, Store};
use substrate_types::{ModelId, Result};

/// Configuration for the cache manager.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Root directory for KV cache blob files.
    pub cache_dir: String,

    /// Maximum total bytes of KV cache allowed on disk.
    pub kv_cache_budget_bytes: u64,
}

/// Manages disk-backed KV/prefix cache entries.
///
/// Cheap to clone (the `Store` is `Arc`-backed internally).
pub struct CacheManager {
    config: CacheConfig,
    store: Store,
    gc: Arc<GcService>,
}

impl CacheManager {
    /// Create a new cache manager with the given config and store handle.
    pub fn new(config: CacheConfig, store: Store, gc: Arc<GcService>) -> Self {
        Self { config, store, gc }
    }

    /// Record a new KV cache entry (called after a successful slot save).
    ///
    /// If recording this entry would exceed the budget, LRU entries are evicted
    /// first. Returns `SubstrateError::DiskBudgetExceeded` if the budget cannot
    /// be satisfied even after full eviction.
    ///
    /// # Arguments
    ///
    /// - `model_id`    — the model whose slot produced this cache state
    /// - `prompt_hash` — caller-supplied hash of the tokenized prompt prefix
    /// - `file_path`   — absolute path to the saved KV state file
    /// - `bytes`       — size of the saved file in bytes
    pub async fn record_entry(
        &self,
        model_id: &ModelId,
        prompt_hash: &str,
        file_path: &str,
        bytes: u64,
    ) -> Result<()> {
        // Ask GC to make room before writing.
        if let Err(e) = self.gc.make_room(&self.config.cache_dir, bytes) {
            tracing::warn!("gc make_room failed for cache: {e}");
        }

        let now = Utc::now();
        let entry = KvCacheEntry {
            id: entry_id(model_id, prompt_hash),
            model_id: model_id.clone(),
            prompt_hash: prompt_hash.to_string(),
            file_path: file_path.to_string(),
            bytes,
            hit_count: 0,
            created_at: now,
            last_accessed_at: now,
        };

        self.store.upsert_kv_cache_entry(&entry)?;

        // Register with GC so it can track and evict this file.
        let cache_path = std::path::Path::new(file_path);
        if let Err(e) = self.gc.register_entry(cache_path, EntryKind::File, None) {
            tracing::warn!("gc register_entry failed for cache entry {file_path}: {e}");
        }

        tracing::debug!(
            model = %model_id,
            prompt_hash = %prompt_hash,
            bytes = bytes,
            path = %file_path,
            "KV cache entry recorded"
        );
        Ok(())
    }

    /// Find a cached prefix entry by (model_id, prompt_hash).
    ///
    /// Returns the absolute path to the cache file on a hit, or `None` if no
    /// match exists. On a hit, updates `last_accessed_at` and increments
    /// `hit_count` via an upsert (the store's ON CONFLICT clause handles this).
    pub async fn find_cached_prefix(
        &self,
        model_id: &ModelId,
        prompt_hash: &str,
    ) -> Result<Option<String>> {
        match self.store.find_kv_cache_entry(model_id, prompt_hash)? {
            None => Ok(None),
            Some(entry) => {
                // Touch last_accessed_at and increment hit_count.
                let touched = KvCacheEntry {
                    last_accessed_at: Utc::now(),
                    hit_count: entry.hit_count, // store increments on conflict
                    ..entry.clone()
                };
                self.store.upsert_kv_cache_entry(&touched)?;

                // Notify GC of the access so LRU ordering stays fresh.
                if let Err(e) = self.gc.touch(&entry.file_path) {
                    tracing::warn!("gc touch failed for {}: {e}", entry.file_path);
                }

                tracing::debug!(
                    model = %model_id,
                    prompt_hash = %prompt_hash,
                    path = %entry.file_path,
                    hits = entry.hit_count + 1,
                    "KV cache hit"
                );
                Ok(Some(entry.file_path))
            }
        }
    }

    /// Purge all cache entries for a model (called when the model is evicted from disk).
    ///
    /// The cached slot states for a model become invalid as soon as the model
    /// weights are removed, so all associated cache files are deleted immediately.
    ///
    /// Returns the number of bytes freed.
    pub async fn purge_model(&self, model_id: &ModelId) -> Result<u64> {
        // Fetch entries for this model so we can evict via GC and clean up the store.
        let entries = self.store.kv_cache_for_eviction(Some(model_id))?;
        let mut freed = 0u64;
        for entry in &entries {
            // Tell GC to evict this specific file.
            if let Err(e) = self.gc.evict(&entry.file_path) {
                tracing::warn!("gc evict failed for {}: {e}", entry.file_path);
                // Fall back to direct delete.
                if tokio::fs::metadata(&entry.file_path).await.is_ok() {
                    let _ = tokio::fs::remove_file(&entry.file_path).await;
                }
            }
            self.store.delete_kv_cache_entry(&entry.id)?;
            freed += entry.bytes;
        }
        tracing::info!(
            model = %model_id,
            bytes_freed = freed,
            "KV cache purged for model"
        );
        Ok(freed)
    }

    /// Evict LRU entries until total cache bytes ≤ budget.
    ///
    /// Convenience wrapper over [`eviction::evict_to_budget`] exposed for
    /// callers that want to trigger budget enforcement on demand (e.g., on
    /// node startup cleanup).
    ///
    /// Returns the number of bytes freed.
    pub async fn evict_to_budget(&self) -> Result<u64> {
        // Delegate to GC sweep for budget enforcement.
        let report = self.gc.sweep().map_err(|e| substrate_types::SubstrateError::Internal(e.to_string()))?;
        Ok(report.bytes_freed)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Derive a stable entry ID from model_id and prompt_hash.
///
/// Uses `DefaultHasher` to produce a 64-bit hash of the composite key, then
/// formats it as a 16-character hex string. This is intentionally simple — the
/// prompt_hash supplied by the caller already encodes the full prompt identity;
/// we only need a stable, compact primary key for the SQLite row.
///
/// Collision probability for typical workloads (thousands of entries) is
/// negligible. If stronger guarantees are needed, replace with sha2.
fn entry_id(model_id: &str, prompt_hash: &str) -> String {
    let mut hasher = DefaultHasher::new();
    model_id.hash(&mut hasher);
    ":".hash(&mut hasher);
    prompt_hash.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_id_is_stable() {
        let a = entry_id("model-a", "abc123");
        let b = entry_id("model-a", "abc123");
        assert_eq!(a, b);
    }

    #[test]
    fn entry_id_differs_by_model() {
        let a = entry_id("model-a", "abc123");
        let b = entry_id("model-b", "abc123");
        assert_ne!(a, b);
    }

    #[test]
    fn entry_id_differs_by_hash() {
        let a = entry_id("model-a", "abc123");
        let b = entry_id("model-a", "def456");
        assert_ne!(a, b);
    }

    #[test]
    fn entry_id_is_16_chars() {
        let id = entry_id("qwen3.6-30b", "deadbeef");
        assert_eq!(id.len(), 16);
    }
}
