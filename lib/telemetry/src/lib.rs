//! # substrate-telemetry
//!
//! System state sampling (CPU, memory, GPU stub) and the polynomial throughput
//! estimator trained on observed completions.
//!
//! ## Two Components
//!
//! ### `SystemSampler` (runtime system state)
//!
//! Polls system resources at a configurable interval using the `sysinfo` crate.
//! Start with [`SystemSampler::start`]; get the latest reading with
//! [`SystemSampler::snapshot`] (cheap clone, no async needed).
//!
//! GPU utilization is currently stubbed at `0.0` pending Metal/IOKit or NVML integration.
//!
//! ### `ThroughputEstimator` (throughput prediction)
//!
//! Fits a degree-2 polynomial regression model to completion observations recorded
//! via [`ThroughputEstimator::record`]. Given `(input_tokens, output_tokens, parallelism)`,
//! predicts duration in milliseconds with a [`Confidence`] annotation.
//!
//! Not async — pure computation. Thread-safe via internal `Mutex`.
//!
//! ## Module Structure
//!
//! - [`sampler`]   — `SystemSampler`: sysinfo wrapper + GPU stub
//! - [`estimator`] — `ThroughputEstimator`: polynomial regression, `EstimatedDuration`, `Confidence`

pub mod estimator;
pub mod sampler;

// Re-export the primary public surface.
pub use estimator::{Confidence, EstimatedDuration, ThroughputEstimator};
pub use sampler::SystemSampler;

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use substrate_types::SystemState;

/// The telemetry service: manages periodic system sampling via `SystemSampler`.
///
/// `Telemetry` wraps `SystemSampler` and provides an async-friendly
/// `current_state()` method. The scheduler and API layer call this rather than
/// holding the sampler directly.
pub struct Telemetry {
    poll_interval: Duration,
    state: Arc<RwLock<SystemState>>,
}

impl Telemetry {
    pub fn new(poll_interval_ms: u64) -> Self {
        Self {
            poll_interval: Duration::from_millis(poll_interval_ms),
            state: Arc::new(RwLock::new(SystemState {
                sampled_at: chrono::Utc::now(),
                total_memory_bytes: 0,
                used_memory_bytes: 0,
                memory_pressure: 0.0,
                cpu_utilization: 0.0,
                gpu_memory_used_bytes: 0,
                gpu_memory_total_bytes: 0,
                gpu_utilization: 0.0,
                is_gpu_estimate: false,
                resident_model: None,
                running_count: 0,
                pending_count: 0,
                weights_on_disk_bytes: 0,
                kv_cache_bytes: 0,
            })),
        }
    }

    /// Return the most recent system state snapshot.
    pub async fn current_state(&self) -> SystemState {
        self.state.read().await.clone()
    }

    /// Start the background polling loop.
    ///
    /// Spawns a tokio task that calls `SystemSampler` every `poll_interval` and
    /// writes the result into `self.state`. Errors are logged and skipped — the
    /// sampler never crashes the daemon.
    pub fn start_polling(self: Arc<Self>) -> JoinHandle<()> {
        let interval = self.poll_interval;
        let state_handle = Arc::clone(&self.state);

        tokio::spawn(async move {
            let (sampler, _sampler_bg) = SystemSampler::start(interval);

            // Drive the read-side: on each tick the SystemSampler already updated
            // its internal state in the background task it owns. We just copy it
            // into our RwLock so the async `current_state()` callers see fresh data.
            loop {
                tokio::time::sleep(interval).await;

                let snap = sampler.snapshot();
                let mut guard = state_handle.write().await;
                // Preserve scheduling fields that are injected externally.
                let resident = guard.resident_model.clone();
                let running = guard.running_count;
                let pending = guard.pending_count;
                let weights = guard.weights_on_disk_bytes;
                let kv = guard.kv_cache_bytes;

                *guard = snap;
                guard.resident_model = resident;
                guard.running_count = running;
                guard.pending_count = pending;
                guard.weights_on_disk_bytes = weights;
                guard.kv_cache_bytes = kv;
            }
        })
    }
}
