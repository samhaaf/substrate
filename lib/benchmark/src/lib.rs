//! # substrate-benchmark
//!
//! Benchmark orchestrator: idle-period detection, priority-0 sweep collections,
//! and throughput characterization.
//!
//! ## Design
//!
//! Benchmarks are priority-0 [`CollectionRow`]s with `request_full_system: true`.
//! This means:
//!
//! - They only run when the system is otherwise idle (scheduler sees no higher-priority work).
//! - Any real work (priority ≥ 1) immediately preempts them — benchmark completions carry
//!   `preemption_threshold: Some(1)`.
//! - Full-system exclusion guarantees measurement isolation: no competing completions inflate
//!   latency during the sweep.
//!
//! The orchestrator detects idle periods by checking that neither pending nor
//! running completions exist for non-benchmark work. When idle, it inspects
//! which (output_length × parallelism × input_length) cells still need samples
//! and submits only the missing ones.
//!
//! ## Module Structure
//!
//! - [`suite`] — `BenchmarkSuite`, `SweepPoint`, axis constants, `generate_benchmark_prompt`

pub mod suite;

use std::sync::Arc;

use chrono::Utc;
use tracing::{info, warn};
use uuid::Uuid;

use substrate_scheduler::Scheduler;
use substrate_store::Store;
use substrate_types::{
    CollectionId, CollectionRow, CollectionState, CompletionState, ModelId, Result,
};

use crate::suite::BenchmarkSuite;

// ---------------------------------------------------------------------------
// BenchmarkConfig
// ---------------------------------------------------------------------------

/// Configuration for the [`BenchmarkOrchestrator`].
#[derive(Debug, Clone)]
pub struct BenchmarkConfig {
    /// When `false`, the orchestrator does nothing. Useful in test environments
    /// or when benchmarking is unwanted on a production node.
    pub enabled: bool,

    /// Number of completed samples required per (output, parallelism, input)
    /// cell before that cell is considered confident.
    ///
    /// Default: 3.
    pub target_samples_per_cell: u32,

    /// How long (in seconds) the queue must be idle before a sweep is triggered.
    ///
    /// Default: 30.
    pub idle_threshold_secs: u64,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_samples_per_cell: 3,
            idle_threshold_secs: 30,
        }
    }
}

// ---------------------------------------------------------------------------
// BenchmarkOrchestrator
// ---------------------------------------------------------------------------

/// The benchmark orchestrator.
///
/// Runs as a background task (call [`Self::run`] in a `tokio::spawn`).
/// Submits priority-0 full-system sweep collections when the node is idle.
pub struct BenchmarkOrchestrator {
    config: BenchmarkConfig,
    store: Store,
    scheduler: Arc<Scheduler>,
}

impl BenchmarkOrchestrator {
    /// Create a new orchestrator.
    pub fn new(store: Store, config: BenchmarkConfig, scheduler: Arc<Scheduler>) -> Self {
        Self { config, store, scheduler }
    }

    /// The main orchestrator loop. Spawn this as a background `tokio` task.
    ///
    /// Every `idle_threshold_secs` seconds the loop checks whether the node
    /// is idle. If it is, and benchmarking is enabled, it calls
    /// [`Self::schedule_if_idle`] for the first model that still needs samples.
    pub async fn run(&self) -> Result<()> {
        if !self.config.enabled {
            info!("benchmark orchestrator disabled — exiting loop");
            return Ok(());
        }

        let interval = std::time::Duration::from_secs(self.config.idle_threshold_secs);

        loop {
            tokio::time::sleep(interval).await;

            // Pick the first registered downloaded model and check whether it
            // needs benchmark samples. In a typical single-model node there
            // will be exactly one candidate.
            let models = match self.store.list_models() {
                Ok(m) => m,
                Err(e) => {
                    warn!("benchmark: failed to list models: {e}");
                    continue;
                }
            };

            for model in &models {
                if !model.is_downloaded {
                    continue;
                }

                let queue_empty = self.is_queue_empty();

                match self.schedule_if_idle(&model.id, queue_empty) {
                    Ok(Some(col_id)) => {
                        info!(
                            model = %model.id,
                            collection = %col_id,
                            "benchmark: submitted sweep collection"
                        );
                        // Submit one sweep at a time; next idle window picks
                        // up the next model if needed.
                        break;
                    }
                    Ok(None) => {
                        // Queue non-empty or all cells already satisfied.
                    }
                    Err(e) => {
                        warn!(model = %model.id, "benchmark sweep error: {e}");
                    }
                }
            }
        }
    }

    /// If the queue is empty and the given model has underpopulated cells,
    /// create a collection and submit the missing completions.
    ///
    /// Returns `Some(CollectionId)` if a sweep was submitted, `None` if the
    /// queue is non-empty or all cells are already satisfied.
    pub fn schedule_if_idle(
        &self,
        current_model: &ModelId,
        queue_empty: bool,
    ) -> Result<Option<CollectionId>> {
        if !self.config.enabled || !queue_empty {
            return Ok(None);
        }

        let suite = BenchmarkSuite::new(current_model.clone(), self.store.clone());
        let existing = suite.existing_runs()?;

        // Create a temporary collection ID to compute missing samples.
        let collection_id = Uuid::new_v4();

        let missing = suite.missing_samples(collection_id, &existing)?;
        if missing.is_empty() {
            return Ok(None);
        }

        // Insert the collection record into the store.
        let col_row = CollectionRow {
            id: collection_id,
            name: format!(
                "benchmark-sweep-{}-{}",
                current_model,
                Utc::now().format("%Y-%m-%dT%H%M%S")
            ),
            description: Some(format!(
                "Automated benchmark sweep: {} missing samples across \
                 output×parallelism×input grid",
                missing.len()
            )),
            cancel_on_failure: false,
            request_full_system: true,
            start_with_no_model_loaded: false,
            save_partial_results: true,
            metadata_json: None,
            state: CollectionState::Active,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
        };
        self.store.insert_collection(&col_row)?;

        // Submit each missing-sample completion through the scheduler.
        for req in missing {
            self.scheduler.submit(req)?;
        }

        Ok(Some(collection_id))
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Return `true` if there are no pending or running completions in the store.
    pub fn is_queue_empty(&self) -> bool {
        let counts = match self.store.count_by_state() {
            Ok(c) => c,
            Err(_) => return false,
        };

        let pending = counts
            .iter()
            .find(|(s, _)| *s == CompletionState::Pending)
            .map(|(_, n)| *n)
            .unwrap_or(0);

        let running = counts
            .iter()
            .find(|(s, _)| *s == CompletionState::Running)
            .map(|(_, n)| *n)
            .unwrap_or(0);

        pending == 0 && running == 0
    }
}
