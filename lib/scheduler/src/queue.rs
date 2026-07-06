//! `Queue` — a thin view over the store's pending completions.
//!
//! The queue is not a separate data structure — it is a query against the SQLite
//! `completions` table. This keeps the store as the single source of truth, which
//! means the queue survives crashes without any separate recovery step.

use substrate_store::Store;
use substrate_types::{CompletionId, CompletionRow, CompletionState, ModelId, Result};

/// A view over the store's pending queue.
///
/// `CompletionQueue` is deliberately thin: every method maps 1:1 to a store
/// query. There is no in-memory state, no caching, no coordination logic.
/// The scheduler drives all decisions; the queue is only a read/write API.
pub struct CompletionQueue {
    store: Store,
}

impl CompletionQueue {
    /// Construct a new queue view backed by `store`.
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    /// Return all pending completions across all models, ordered by
    /// `(priority DESC, created_at ASC)`.
    pub fn pending_all(&self) -> Result<Vec<CompletionRow>> {
        self.store.select_pending()
    }

    /// Return pending completions for a specific model, highest priority first.
    pub fn pending_for_model(&self, model_id: &ModelId) -> Result<Vec<CompletionRow>> {
        self.store.select_pending_for_model(model_id)
    }

    /// Return the highest-priority pending completion across all models, or `None`
    /// if the queue is empty.
    ///
    /// Used by the swap evaluator to decide whether a model swap is warranted.
    pub fn peek_highest_priority(&self) -> Result<Option<CompletionRow>> {
        Ok(self.store.select_pending()?.into_iter().next())
    }

    /// Put a preempted completion back into the Pending state.
    ///
    /// Increments `preemption_count` in the store so the circuit breaker in the
    /// scheduler can detect thrashing completions.
    pub fn requeue(&self, id: CompletionId) -> Result<()> {
        self.store.requeue_preempted(id)
    }

    /// Transition a completion to Cancelled state.
    ///
    /// Pending completions: cancelled immediately.
    /// Running completions: the scheduler is responsible for aborting the engine
    /// slot first; this call records the terminal state.
    pub fn cancel(&self, id: CompletionId) -> Result<()> {
        self.store.mark_cancelled(id)
    }

    /// How many completions are currently in the given state.
    ///
    /// Iterates the `count_by_state` result to find the requested state.
    pub fn count_in_state(&self, target: CompletionState) -> Result<u64> {
        let counts = self.store.count_by_state()?;
        Ok(counts
            .into_iter()
            .find(|(state, _)| *state == target)
            .map(|(_, count)| count)
            .unwrap_or(0))
    }

    /// Total number of pending completions across all models.
    pub fn pending_count(&self) -> Result<u64> {
        self.count_in_state(CompletionState::Pending)
    }
}
