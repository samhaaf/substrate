//! Swap evaluation: when should the resident model be replaced?
//!
//! The `SwapEvaluator` trait is a stub seam for future anti-thrash logic. The
//! `DefaultSwapEvaluator` mirrors the v1 logic: swap when the highest-priority
//! pending completion targets a different model than the one currently loaded.
//!
//! # Anti-thrash (future)
//!
//! When two models compete for the same slot at similar priority, the scheduler
//! can oscillate (swap A→B→A→B). `DebouncedSwapEvaluator` wraps any `SwapEvaluator`
//! with a minimum inter-swap interval to suppress this oscillation.

use std::time::{Duration, Instant};
use std::sync::Mutex;

use substrate_types::{CompletionRow, ModelId};

// ---------------------------------------------------------------------------
// SwapEvaluator trait
// ---------------------------------------------------------------------------

/// Decides whether to swap the resident model.
///
/// The trait takes the currently-loaded model (None if none is loaded) and the
/// full list of pending completions in priority order. This signature lets
/// future evaluators inspect the entire queue, not just the top candidate.
///
/// # Contract
///
/// - Return `true` if the scheduler should initiate a model swap.
/// - Return `false` to keep the current model resident and continue admitting
///   from its queue.
/// - An evaluator that returns `true` when `candidates` is empty will cause the
///   engine to drain without a target; the scheduler guards against this.
pub trait SwapEvaluator: Send + Sync {
    /// Returns true if the scheduler should preempt current work and swap models.
    ///
    /// # Arguments
    /// - `resident`: the model currently loaded in the engine (`None` if no model loaded)
    /// - `candidates`: pending completions in `(priority DESC, created_at ASC)` order
    fn should_swap(
        &self,
        resident: Option<&ModelId>,
        candidates: &[CompletionRow],
    ) -> bool;
}

// ---------------------------------------------------------------------------
// DefaultSwapEvaluator
// ---------------------------------------------------------------------------

/// Default swap evaluator: swap when the top pending completion targets a different model.
///
/// This is the v1 logic ported unchanged. It does not debounce — if two models are
/// competing, it will recommend a swap on every tick until one model drains the other's
/// queue. Use [`DebouncedSwapEvaluator`] to suppress that oscillation.
///
/// Edge cases:
/// - No model loaded + pending work → swap (load the model for the top candidate).
/// - Any model loaded + no pending work → no swap.
/// - Resident == top candidate model → no swap.
#[derive(Default)]
pub struct DefaultSwapEvaluator;

impl SwapEvaluator for DefaultSwapEvaluator {
    fn should_swap(&self, resident: Option<&ModelId>, candidates: &[CompletionRow]) -> bool {
        match (resident, candidates.first()) {
            (Some(loaded), Some(top)) => &top.model_id != loaded,
            (None, Some(_)) => true,    // No model loaded; must load something.
            (_, None) => false,         // No pending work; nothing to swap to.
        }
    }
}

// ---------------------------------------------------------------------------
// PreemptionPolicy
// ---------------------------------------------------------------------------

/// Wraps a `SwapEvaluator` with hard-memory-pressure override logic.
///
/// When memory pressure is above `hard_pct` and the highest-priority pending
/// completion has priority greater than the lowest-priority running completion,
/// the policy forces a swap (and the scheduler will cancel running work to free
/// memory before swapping).
pub struct PreemptionPolicy {
    evaluator: Box<dyn SwapEvaluator>,
    hard_pct: f32,
}

impl PreemptionPolicy {
    pub fn new(evaluator: impl SwapEvaluator + 'static) -> Self {
        Self {
            evaluator: Box::new(evaluator),
            hard_pct: 0.95,
        }
    }

    pub fn with_hard_pct(mut self, hard_pct: f32) -> Self {
        self.hard_pct = hard_pct;
        self
    }

    /// Evaluate a swap given queue state and current memory pressure.
    ///
    /// If memory is above `hard_pct` and the top pending completion has higher
    /// priority than `min_running_priority`, returns true to force a preemptive swap.
    pub fn should_swap(
        &self,
        resident: Option<&ModelId>,
        candidates: &[CompletionRow],
        memory_pressure: f32,
        min_running_priority: Option<i32>,
    ) -> bool {
        // Hard memory pressure override: preempt if top candidate priority
        // exceeds the lowest-priority running completion.
        if memory_pressure >= self.hard_pct {
            if let (Some(top), Some(min_prio)) = (candidates.first(), min_running_priority) {
                if top.priority > min_prio {
                    return true;
                }
            }
        }

        self.evaluator.should_swap(resident, candidates)
    }
}

// ---------------------------------------------------------------------------
// DebouncedSwapEvaluator (anti-thrash stub)
// ---------------------------------------------------------------------------

/// Suppresses rapid swap oscillations by enforcing a minimum interval between swaps.
///
/// # Future
/// Set `min_interval` to a value that allows the resident model to amortize
/// its swap cost before being replaced. For example, if a model swap costs ~5 s,
/// a 10 s minimum interval ensures each model runs for at least one swap-cost
/// worth of time before being replaced.
///
/// The `last_swap` timestamp is updated by the *caller* (the scheduler) via
/// [`DebouncedSwapEvaluator::record_swap`] whenever an actual swap occurs.
pub struct DebouncedSwapEvaluator {
    inner: Box<dyn SwapEvaluator>,
    min_interval: Duration,
    last_swap: Mutex<Option<Instant>>,
}

impl DebouncedSwapEvaluator {
    pub fn new(inner: impl SwapEvaluator + 'static, min_interval: Duration) -> Self {
        Self {
            inner: Box::new(inner),
            min_interval,
            last_swap: Mutex::new(None),
        }
    }

    /// Record that a swap just occurred. Call this after every actual model swap.
    pub fn record_swap(&self) {
        if let Ok(mut guard) = self.last_swap.lock() {
            *guard = Some(Instant::now());
        }
    }
}

impl SwapEvaluator for DebouncedSwapEvaluator {
    fn should_swap(&self, resident: Option<&ModelId>, candidates: &[CompletionRow]) -> bool {
        // TODO: add debounce/hysteresis here
        // Current stub: delegate directly to inner without enforcing min_interval.
        // Full implementation should check whether Instant::now() >= last_swap + min_interval
        // before returning true. This prevents oscillation when two models have similar priority.
        self.inner.should_swap(resident, candidates)
    }
}

// ---------------------------------------------------------------------------
// LocalityAwareSwapEvaluator (stub seam for KV cache integration)
// ---------------------------------------------------------------------------

/// Stub: locality-aware swap evaluator.
///
/// # Future
/// Consider the KV cache hit rate of the current resident model. Avoid swapping
/// if the current model has strong cache locality for the pending queue (i.e.,
/// many pending completions share prompt prefixes with recently cached completions
/// for the resident model).
///
/// This stub seam is left for the cache-aware scheduling integration point.
/// The KV cache crate can implement this trait and inject it at `Node::start()` time.
pub struct LocalityAwareSwapEvaluator;

impl SwapEvaluator for LocalityAwareSwapEvaluator {
    fn should_swap(&self, _resident: Option<&ModelId>, _candidates: &[CompletionRow]) -> bool {
        todo!("Locality-aware swap: weight cache hit rate vs priority delta")
    }
}
