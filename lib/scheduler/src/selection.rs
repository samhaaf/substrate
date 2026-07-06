//! Selection policy: which pending completion to admit next?
//!
//! The `SelectionPolicy` trait is a stub seam for cache-locality ordering.
//! The `FifoSelection` impl mirrors v1: pick the highest priority, then oldest.
//!
//! # Future: `LocalitySelection`
//!
//! Pick the completion whose prompt has the most prefix overlap with recently
//! processed prompts (using the KV cache index). This improves cache hit rates
//! and reduces wasted prefill computation.
//!
//! The selection policy operates on a slice of candidates that the scheduler
//! has already narrowed to the resident model. Ordering within that slice
//! is `(priority DESC, created_at ASC)` as produced by `Store::select_pending_for_model`.

use substrate_types::CompletionRow;

// ---------------------------------------------------------------------------
// SelectionPolicy trait
// ---------------------------------------------------------------------------

/// Selects which pending completion to admit to the engine next.
///
/// The candidates are already filtered to the resident model and ordered by
/// `(priority DESC, created_at ASC)`. The policy may reorder or skip candidates
/// based on domain-specific criteria (e.g., cache locality).
///
/// Returns `None` if no candidate should be admitted (policy-specific decision).
pub trait SelectionPolicy: Send + Sync {
    /// From a list of candidates (already in priority order), pick one.
    ///
    /// Returns `None` if no candidate should be admitted.
    fn select<'a>(&self, candidates: &'a [CompletionRow]) -> Option<&'a CompletionRow>;
}

// ---------------------------------------------------------------------------
// FifoSelection
// ---------------------------------------------------------------------------

/// Default: pick the first candidate (highest priority, then oldest within priority).
///
/// This is equivalent to the v1 FIFO-within-priority behavior. Because the store
/// returns candidates pre-sorted as `(priority DESC, created_at ASC)`, selecting
/// the first element implements strict priority queuing with FIFO tie-breaking.
pub struct FifoSelection;

impl SelectionPolicy for FifoSelection {
    fn select<'a>(&self, candidates: &'a [CompletionRow]) -> Option<&'a CompletionRow> {
        candidates.first()
    }
}

// ---------------------------------------------------------------------------
// LocalitySelection (stub seam)
// ---------------------------------------------------------------------------

/// Stub: prefer completions with shared prompt prefixes (graph-local, cache-locality selection).
///
/// # Future
/// Query the KV cache index for prefix overlaps between candidate prompts and
/// recently served completions. Reorder candidates by
/// `(cache_hit_priority DESC, original_priority DESC, created_at ASC)`.
///
/// Integration point: the `substrate-cache` crate exposes a `find_cached_prefix`
/// method keyed by `(model_id, prompt_hash)`. `LocalitySelection` should accept
/// an `Arc<CacheManager>` at construction and query it during `select()`.
///
/// This stub is intentionally left unimplemented so that the cache integration
/// can be added without restructuring the scheduler.
pub struct LocalitySelection;

impl SelectionPolicy for LocalitySelection {
    fn select<'a>(&self, _candidates: &'a [CompletionRow]) -> Option<&'a CompletionRow> {
        todo!(
            "Graph-local / cache-locality selection: \
             query KV cache index for prompt hash overlaps, \
             then reorder candidates by (cache_hit_priority DESC, priority DESC, created_at ASC)"
        )
    }
}
