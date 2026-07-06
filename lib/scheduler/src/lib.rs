//! # substrate-scheduler
//!
//! Central scheduling loop: queue management, admission control, swap evaluation,
//! preemption, crash recovery, and pluggable selection/swap policies.
//!
//! ## Scheduling Loop
//!
//! The scheduler runs a tight async loop (`Scheduler::run`). On each tick:
//!
//! 1. Pull the current system state from telemetry.
//! 2. Check hard memory pressure — if exceeded, cancel in-flight work and requeue.
//! 3. Check soft memory pressure — if exceeded, pause admission; in-flight drains naturally.
//! 4. Evaluate whether a model swap is needed (`SwapEvaluator::should_swap`).
//! 5. If swap: drain engine, swap model.
//! 6. Admit pending completions up to the concurrency target.
//! 7. Sleep `tick_interval_ms` or until woken by `notify_new_work()`.
//!
//! ## Pluggable Policies
//!
//! - [`swap::SwapEvaluator`] — decides when to swap models (default: `DefaultSwapEvaluator`)
//! - [`selection::SelectionPolicy`] — which pending completion to admit next (default: `FifoSelection`)
//!
//! ## Module Structure
//!
//! - [`queue`]     — `CompletionQueue`: thin view over the store's pending completions
//! - [`admission`] — `AdmissionController`: concurrency targeting + memory-pressure gates
//! - [`swap`]      — `SwapEvaluator` trait + `DefaultSwapEvaluator` + stubs
//! - [`selection`] — `SelectionPolicy` trait + `FifoSelection` + `LocalitySelection` stub

pub mod admission;
pub mod queue;
pub mod selection;
pub mod swap;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, Notify};

use substrate_engine::ExecutionEngine;
use substrate_store::Store;
use substrate_telemetry::Telemetry;
use substrate_types::{
    CompletionId, CompletionRequest, CompletionRow, CompletionState,
    LifecycleEvent, ModelId, Result, StreamEvent, SubstrateError,
};

use crate::admission::AdmissionController;
use crate::queue::CompletionQueue;
use crate::selection::{FifoSelection, SelectionPolicy};
use crate::swap::{DefaultSwapEvaluator, SwapEvaluator};

// ---------------------------------------------------------------------------
// SchedulerConfig
// ---------------------------------------------------------------------------

/// Tunable parameters for the scheduling loop.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum number of completions to run in parallel.
    pub max_concurrent: u32,

    /// How long to sleep between scheduling ticks (milliseconds).
    /// The loop also wakes immediately when `notify_new_work` fires.
    pub tick_interval_ms: u64,

    /// Memory soft threshold (0.0–1.0). Above this, concurrency target is reduced.
    pub memory_soft_pct: f32,

    /// Memory hard threshold (0.0–1.0). Above this, in-flight work is preempted.
    pub memory_hard_pct: f32,

    /// Maximum number of times a completion can be preempted before it is failed.
    pub max_preemption_count: u32,

    /// How long (seconds) to wait for the engine to drain before forcing a swap.
    pub drain_timeout_secs: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_concurrent: 4,
            tick_interval_ms: 100,
            memory_soft_pct: 0.90,
            memory_hard_pct: 0.95,
            max_preemption_count: 100,
            drain_timeout_secs: 60,
        }
    }
}

// ---------------------------------------------------------------------------
// Scheduler
// ---------------------------------------------------------------------------

/// The substrate scheduler.
///
/// Owns the queue view, the engine reference, and the swap/selection policies.
/// One scheduler per node.
///
/// The scheduling loop runs as a background `tokio::spawn`ed task (see [`Scheduler::run`]).
/// The API layer calls `submit` and `cancel` concurrently from HTTP handler tasks.
pub struct Scheduler {
    config: SchedulerConfig,
    store: Store,
    queue: CompletionQueue,
    engine: Arc<ExecutionEngine>,
    telemetry: Arc<Telemetry>,
    admission: AdmissionController,
    swap_evaluator: Box<dyn SwapEvaluator>,
    selection_policy: Box<dyn SelectionPolicy>,
    /// Notified when new work is submitted or a slot frees up.
    new_work: Arc<Notify>,
    /// Optional broadcast channel for lifecycle events (QueueDepthChanged).
    event_tx: Option<broadcast::Sender<LifecycleEvent>>,
}

impl Scheduler {
    /// Construct a scheduler with default policies and a concurrency ceiling.
    ///
    /// Use [`Scheduler::with_swap_evaluator`] and [`Scheduler::with_selection_policy`]
    /// to override default policies after construction.
    pub fn new(
        store: Store,
        engine: Arc<ExecutionEngine>,
        telemetry: Arc<Telemetry>,
        max_concurrent: u32,
    ) -> Self {
        let config = SchedulerConfig {
            max_concurrent,
            ..Default::default()
        };
        let admission = AdmissionController::new(max_concurrent)
            .with_memory_limits(config.memory_soft_pct, config.memory_hard_pct);
        let queue = CompletionQueue::new(store.clone());

        Self {
            config,
            store,
            queue,
            engine,
            telemetry,
            admission,
            swap_evaluator: Box::new(DefaultSwapEvaluator::default()),
            selection_policy: Box::new(FifoSelection),
            new_work: Arc::new(Notify::new()),
            event_tx: None,
        }
    }

    /// Construct a scheduler with a fully specified [`SchedulerConfig`].
    pub fn with_config(
        config: SchedulerConfig,
        store: Store,
        engine: Arc<ExecutionEngine>,
        telemetry: Arc<Telemetry>,
    ) -> Self {
        let admission = AdmissionController::new(config.max_concurrent)
            .with_memory_limits(config.memory_soft_pct, config.memory_hard_pct);
        let queue = CompletionQueue::new(store.clone());

        Self {
            config,
            store,
            queue,
            engine,
            telemetry,
            admission,
            swap_evaluator: Box::new(DefaultSwapEvaluator::default()),
            selection_policy: Box::new(FifoSelection),
            new_work: Arc::new(Notify::new()),
            event_tx: None,
        }
    }

    /// Replace the swap evaluator (for testing or advanced configs).
    pub fn with_swap_evaluator(mut self, evaluator: impl SwapEvaluator + 'static) -> Self {
        self.swap_evaluator = Box::new(evaluator);
        self
    }

    /// Replace the selection policy.
    pub fn with_selection_policy(mut self, policy: impl SelectionPolicy + 'static) -> Self {
        self.selection_policy = Box::new(policy);
        self
    }

    /// Attach a lifecycle event broadcast channel to this scheduler.
    ///
    /// Once set, the scheduler emits `QueueDepthChanged` events after each
    /// `submit()` and after each completion finishes running.
    pub fn with_events(mut self, tx: broadcast::Sender<LifecycleEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Emit a lifecycle event, silently dropping it when there are no subscribers.
    fn emit(&self, event: LifecycleEvent) {
        if let Some(tx) = &self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// Emit a QueueDepthChanged event reflecting current queue state.
    fn emit_queue_depth(&self) {
        let pending = self.queue.pending_count().unwrap_or(0) as u32;
        let running = self.engine.running_count() as u32;
        self.emit(LifecycleEvent::QueueDepthChanged { pending, running });
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Submit a completion request for scheduling.
    ///
    /// Validates that the model exists in the registry, persists the request as a
    /// `Pending` row in the store, and wakes the scheduling loop. Returns the
    /// assigned completion ID.
    pub fn submit(&self, req: CompletionRequest) -> Result<CompletionId> {
        // Validate that the requested model exists in the registry.
        let _ = self.store.get_model(&req.model_id)?;

        // Compute effective preemption threshold before any field moves.
        let preemption_threshold = req.effective_preemption_threshold();

        // Serialize sampling parameters to JSON for storage.
        let params = serde_json::json!({
            "max_tokens":     req.max_tokens,
            "temperature":    req.temperature,
            "top_p":          req.top_p,
            "top_k":          req.top_k,
            "repeat_penalty": req.repeat_penalty,
            "stop":           req.stop,
        });
        let params_json = serde_json::to_string(&params)
            .map_err(SubstrateError::Serialization)?;

        let row = CompletionRow {
            id:                   req.id,
            model_id:             req.model_id,
            prompt:               req.prompt,
            params_json,
            priority:             req.priority,
            preemption_threshold,
            json_schema:          req.json_schema.as_ref().map(|v| v.to_string()),
            collection_id:        req.collection_id,
            metrics_flags:        req.metrics.0,
            metadata_json:        req.metadata.as_ref().map(|v| v.to_string()),
            // insert_completion always overrides state to 'pending' regardless of this field.
            state:                CompletionState::Pending,
            preemption_count:     0,
            error_retry_count:    0,
            created_at:           req.created_at,
            started_at:           None,
            completed_at:         None,
            recovered_at:         None,
        };

        self.store.insert_completion(&row)?;
        self.emit_queue_depth();
        self.notify_new_work();
        Ok(row.id)
    }

    /// Cancel a pending or running completion.
    ///
    /// - `Pending`: transitions to `Cancelled` immediately (no engine interaction).
    /// - `Running`: aborts the engine slot, then transitions to `Cancelled`.
    /// - Terminal states (`Completed`, `Failed`, `Cancelled`): returns `CompletionTerminal`.
    pub async fn cancel(&self, id: CompletionId) -> Result<()> {
        let row = self.store.get_completion(id)?;
        match row.state {
            CompletionState::Pending => {
                self.store.mark_cancelled(id)?;
                Ok(())
            }
            CompletionState::Running => {
                self.engine.abort_slot(id).await?;
                self.store.mark_cancelled(id)?;
                Ok(())
            }
            state => Err(SubstrateError::CompletionTerminal { id, state }),
        }
    }

    /// Wake the scheduling loop.
    ///
    /// Called automatically by `submit`. Also useful when external state changes
    /// (e.g., a model finishes downloading and is now available to run).
    pub fn notify_new_work(&self) {
        self.new_work.notify_one();
    }

    /// The main scheduling loop. Run this as a background task via `tokio::spawn`.
    ///
    /// Runs indefinitely, sleeping `tick_interval_ms` between ticks. The loop also
    /// wakes immediately when `notify_new_work()` fires, so new submissions are picked
    /// up without waiting for the full tick interval.
    pub async fn run(&self) -> Result<()> {
        let tick = Duration::from_millis(self.config.tick_interval_ms);
        loop {
            // Wait for a notification or a tick timeout, whichever comes first.
            tokio::select! {
                _ = self.new_work.notified() => {},
                _ = tokio::time::sleep(tick) => {},
            }

            if let Err(e) = self.tick().await {
                tracing::error!(err = %e, "scheduler tick error");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal scheduling logic
    // -----------------------------------------------------------------------

    /// One scheduling pass.
    async fn tick(&self) -> Result<()> {
        let sys = self.telemetry.current_state().await;

        // ── Hard pressure: cancel in-flight work and requeue ─────────────────
        if sys.memory_pressure >= self.config.memory_hard_pct {
            tracing::error!(
                pressure = sys.memory_pressure,
                "memory above hard threshold — cancelling running completions"
            );
            self.handle_hard_pressure().await?;
            return Ok(());
        }

        // ── Soft pressure: stop admitting, let in-flight drain naturally ─────
        if sys.memory_pressure >= self.config.memory_soft_pct {
            tracing::warn!(
                pressure = sys.memory_pressure,
                "memory above soft threshold — pausing admission"
            );
            return Ok(());
        }

        // ── Evaluate model swap ──────────────────────────────────────────────
        let all_pending = self.queue.pending_all()?;
        let resident = self.engine.resident_model();

        if self.swap_evaluator.should_swap(resident.as_ref(), &all_pending) {
            if let Some(top) = all_pending.first() {
                let target = top.model_id.clone();
                self.execute_model_swap(&target).await?;
            }
        }

        // ── Admit pending completions ────────────────────────────────────────
        self.admit_pending(&sys).await?;

        Ok(())
    }

    /// Admit as many pending completions as the concurrency target allows.
    async fn admit_pending(&self, sys: &substrate_types::SystemState) -> Result<()> {
        let resident = match self.engine.resident_model() {
            Some(m) => m,
            None => return Ok(()), // No model loaded; nothing to admit.
        };

        let running = self.engine.running_count();
        let slots = self.admission.slots_to_admit(sys, running);
        if slots == 0 {
            return Ok(());
        }

        // Fetch candidates for the resident model, already sorted by (priority DESC, created_at ASC).
        let candidates = self.queue.pending_for_model(&resident)?;
        if candidates.is_empty() {
            return Ok(());
        }

        // Admit up to `slots` completions using the selection policy.
        // Re-query on each iteration so the slice stays consistent with store state.
        let mut admitted = 0u32;
        for _ in 0..slots {
            let candidates = self.queue.pending_for_model(&resident)?;
            if candidates.is_empty() {
                break;
            }
            match self.selection_policy.select(&candidates) {
                None => break,
                Some(chosen) => {
                    let row = chosen.clone();
                    let id = row.id;
                    self.store.mark_running(id)?;

                    // Create a streaming channel for token events.
                    // The receiver is currently dropped here; the WebSocket layer
                    // will wire this up in a future phase.
                    let (token_tx, _token_rx) = mpsc::channel::<StreamEvent>(64);

                    if let Err(e) = self.engine.submit(row, token_tx).await {
                        tracing::error!(
                            completion_id = %id,
                            err = %e,
                            "engine submit failed; marking completion failed"
                        );
                        self.store.mark_failed(id, &e.to_string())?;
                    }
                    self.emit_queue_depth();
                    admitted += 1;
                }
            }
        }

        // If we admitted something and there may be more room, wake the next tick early.
        if admitted > 0 && (running + admitted as usize) < self.config.max_concurrent as usize {
            self.new_work.notify_one();
        }

        Ok(())
    }

    /// Execute a model swap: drain the engine, then load the target model.
    async fn execute_model_swap(&self, target_model: &ModelId) -> Result<()> {
        tracing::info!(model_id = target_model, "initiating model swap");

        // Look up the file path for the target model.
        let model_row = self.store.get_model(target_model)?;
        let file_path = model_row.file_path.as_deref().ok_or_else(|| {
            SubstrateError::ModelNotDownloaded(target_model.clone())
        })?;
        let file_path = file_path.to_owned();

        // Drain or cancel in-flight work.
        let sys = self.telemetry.current_state().await;
        if sys.memory_pressure >= self.config.memory_hard_pct {
            // Under hard pressure: cancel immediately and requeue.
            self.engine.cancel_all_running().await?;
            let ids = self.engine.take_running_ids();
            for id in ids {
                self.requeue_with_circuit_breaker(id).await?;
            }
        } else {
            // Normal path: wait for in-flight work to finish.
            if let Err(e) = self.engine.drain(self.config.drain_timeout_secs).await {
                tracing::warn!(
                    err = %e,
                    "drain timed out during model swap — requeueing remaining work"
                );
                let ids = self.engine.take_running_ids();
                for id in ids {
                    self.requeue_with_circuit_breaker(id).await?;
                }
            }
        }

        // Swap to the new model.
        self.engine
            .swap_model(target_model.clone(), Path::new(&file_path))
            .await?;

        tracing::info!(model_id = target_model, "model swap complete");
        Ok(())
    }

    /// Cancel and requeue all running completions under hard memory pressure.
    async fn handle_hard_pressure(&self) -> Result<()> {
        self.engine.cancel_all_running().await?;
        let ids = self.engine.take_running_ids();
        for id in ids {
            self.requeue_with_circuit_breaker(id).await?;
        }
        Ok(())
    }

    /// Requeue a preempted completion, checking the circuit breaker first.
    ///
    /// If `preemption_count >= max_preemption_count`, the completion is failed
    /// instead of requeued, preventing infinite retry cycles.
    async fn requeue_with_circuit_breaker(&self, id: CompletionId) -> Result<()> {
        let row = self.store.get_completion(id)?;
        if row.preemption_count >= self.config.max_preemption_count {
            tracing::error!(
                completion_id = %id,
                preemption_count = row.preemption_count,
                max = self.config.max_preemption_count,
                "completion exceeded max_preemption_count — failing"
            );
            self.store.mark_failed(
                id,
                &format!(
                    "exceeded max_preemption_count ({})",
                    self.config.max_preemption_count
                ),
            )?;
            return Err(SubstrateError::ResourceExhausted(id, row.preemption_count));
        }
        self.store.requeue_preempted(id)?;
        Ok(())
    }
}
