//! WebSocket streaming event types.
//!
//! When a client opens a WebSocket connection to `/v1/completions/:id/stream`,
//! the server sends a sequence of [`StreamEvent`] messages serialized as JSON.
//!
//! ## Event ordering guarantee
//!
//! 1. `Started` arrives exactly once at the beginning of each run.
//! 2. Zero or more `Token` events follow in order.
//! 3. Exactly one terminal event closes the stream:
//!    `Completed`, `Failed`, `Cancelled`, or `Preempted`.
//!
//! If a completion is preempted and requeued, the client receives a `Preempted`
//! event and the stream closes. The client should reconnect to the same
//! `/v1/completions/:id/stream` endpoint to receive events for the next run.
//! The completion ID is stable across requeue cycles.
//!
//! `Heartbeat` events are sent every ~30 seconds to keep the WebSocket alive
//! through proxies and load balancers.

use serde::{Deserialize, Serialize};

use crate::completion::{CompletionId, CompletionMetrics, PreemptionReason, TerminationReason};
use crate::model::ModelId;

/// A single event emitted over the WebSocket stream for a completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    /// Emitted once when the engine begins processing this completion.
    Started {
        id: CompletionId,
    },

    /// A generated token fragment. Multiple tokens may be batched into one event.
    Token {
        id: CompletionId,
        /// The token text.
        text: String,
        /// Cumulative token count so far in this run.
        token_count: u32,
    },

    /// Terminal: generation finished successfully.
    Completed {
        id: CompletionId,
        termination: TerminationReason,
        metrics: CompletionMetrics,
    },

    /// Terminal: generation failed.
    Failed {
        id: CompletionId,
        termination: TerminationReason,
        metrics: CompletionMetrics,
    },

    /// Terminal: the completion was explicitly cancelled by the caller.
    Cancelled {
        id: CompletionId,
    },

    /// Terminal: the completion was preempted back to the pending queue.
    ///
    /// The client should reconnect to the same stream URL after a short delay;
    /// the completion will emit a new `Started` event when it is re-admitted.
    Preempted {
        id: CompletionId,
        reason: PreemptionReason,
        /// True if the completion will be automatically requeued.
        /// False means it transitioned to Cancelled (preemption limit exceeded).
        requeued: bool,
    },

    /// Heartbeat event sent periodically to keep the WebSocket alive.
    Heartbeat {
        id: CompletionId,
    },
}

// ── LifecycleEvent ──────────────────────────────────────────────────────

/// Internal state-transition events emitted by the scheduler, engine, store,
/// and provisioner.
///
/// The completion/collection variants are NOT sent to external WebSocket clients.
/// The model, backend, queue, and execution-control variants ARE forwarded over
/// the `/events` WebSocket to connected observers. `CompletionMetricsRecorded`
/// is a deliberate exception to the completion-variant rule: it carries no
/// completion-scoped lifecycle state, only an aggregate throughput sample, and
/// IS forwarded so dashboards can render live per-inference throughput without
/// subscribing to every completion's own stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LifecycleEvent {
    // ── Completion / Collection ──────────────────────────────────────────
    CompletionSubmitted { id: CompletionId },
    CompletionStarted { id: CompletionId },
    CompletionCompleted { id: CompletionId },
    CompletionFailed { id: CompletionId },
    CompletionCancelled { id: CompletionId },
    CompletionPreempted { id: CompletionId },
    CollectionCompleted { id: crate::collection::CollectionId },
    CollectionFailed { id: crate::collection::CollectionId },
    CollectionCancelled { id: crate::collection::CollectionId },

    /// Emitted when a completion finishes and throughput metrics were
    /// captured for it. Forwarded to external `/events` WebSocket clients
    /// (see `is_external_event` in `substrate-api`) so dashboards can plot
    /// live inference throughput.
    CompletionMetricsRecorded {
        id: CompletionId,
        model_id: ModelId,
        output_tokens: u32,
        tokens_per_second: f32,
    },

    // ── Model lifecycle ──────────────────────────────────────────────────

    /// The engine is beginning to load a model (process about to start).
    ModelLoading { model_id: ModelId, path: String },
    /// The model is loaded and the backend is healthy.
    ModelLoaded { model_id: ModelId, path: String },
    /// The engine is about to unload the current model.
    ModelUnloading { model_id: ModelId },
    /// The model process has stopped; no model is resident.
    ModelUnloaded { model_id: ModelId },
    /// A model swap is beginning (old → new; emitted before unload).
    ModelSwapping { from: ModelId, to: ModelId },

    // ── Backend lifecycle ────────────────────────────────────────────────

    /// The llama-server process is being launched.
    BackendStarting { binary_path: String, port: u16 },
    /// The llama-server process passed its health check.
    BackendReady { binary_path: String, port: u16 },
    /// The llama-server process is being shut down.
    BackendStopping { binary_path: String },
    /// The llama-server process has exited.
    BackendStopped,
    /// The provisioner is downloading/extracting a llama.cpp release.
    BackendInstalling { version: String, platform: String },
    /// The backend binary has been installed at `binary_path`.
    BackendInstalled { version: String, binary_path: String },

    // ── Queue state ──────────────────────────────────────────────────────

    /// Queue depth changed (emitted after every submit or completion).
    QueueDepthChanged { pending: u32, running: u32 },
    /// New completions will not be admitted until `ExecutionResumed`.
    ExecutionPaused,
    /// Admission is open again.
    ExecutionResumed,
}
