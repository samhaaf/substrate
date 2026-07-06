//! Completion request/result types and state machine.
//!
//! A [`CompletionRequest`] is the primitive unit of work in substrate. It contains
//! the prompt, generation parameters, and scheduling metadata. All scheduling
//! decisions — admission, preemption, retry — are expressed as state transitions
//! on a [`CompletionState`].
//!
//! The completion lifecycle is linear: Pending → Running → (Completed | Failed | Cancelled).
//! Preemption moves a Running completion back to Pending with an incremented preemption count.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::collection::CollectionId;
use crate::model::ModelId;

// ── Identity ────────────────────────────────────────────────────────────

/// Stable identity for a completion. Wraps UUID v4.
pub type CompletionId = Uuid;

// ── MetricsFlags ────────────────────────────────────────────────────────

/// Bitmask for which metrics fields to collect in the result.
///
/// Using a `u32` bitfield allows extensible flags without schema changes.
/// Downstream code tests individual bits with the provided helper methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MetricsFlags(pub u32);

impl MetricsFlags {
    /// Capture no optional metrics (only mandatory fields).
    pub const NONE: Self = Self(0);
    /// Capture token counts (prompt_tokens, completion_tokens).
    pub const TOKENS: Self = Self(1 << 0);
    /// Capture timing fields (queue_latency_ms, generation_ms).
    pub const TIMING: Self = Self(1 << 1);
    /// Capture all metrics.
    pub const ALL: Self = Self(u32::MAX);

    /// True if token-count metrics should be captured.
    pub fn tokens(self) -> bool {
        self.0 & Self::TOKENS.0 != 0
    }

    /// True if timing metrics should be captured.
    pub fn timing(self) -> bool {
        self.0 & Self::TIMING.0 != 0
    }

    /// Combine two flag sets (bitwise OR).
    pub fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

// ── CompletionRequest ───────────────────────────────────────────────────

/// A completion request as submitted by a client.
///
/// All sampling fields map 1:1 to llama-server's `/completion` payload where
/// they overlap. Substrate does NOT normalize parameters — it passes them
/// through verbatim (see architecture invariant §1.6).
///
/// The `id` field is assigned at submission time by the scheduler; callers
/// submitting via the HTTP API leave it blank and receive the assigned ID.
/// In-process callers (e.g. benchmark orchestrator) may pre-assign an ID
/// using [`CompletionRequest::new_id`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    /// Unique identifier. Assigned at submission time.
    pub id: CompletionId,

    /// The model to run this completion on. Must match a configured model id.
    pub model_id: ModelId,

    /// The fully-resolved prompt string.
    ///
    /// Substrate applies the model's `prompt_template` (wrapping `{prompt}`)
    /// before forwarding to llama-server. The value here is the raw caller text.
    pub prompt: String,

    /// Maximum number of tokens to generate. `None` uses the node default.
    pub max_tokens: Option<u32>,

    /// Temperature (0.0 = deterministic). `None` uses llama-server default.
    pub temperature: Option<f32>,

    /// Top-p sampling threshold. `None` uses llama-server default.
    pub top_p: Option<f32>,

    /// Top-k candidates. `None` uses llama-server default.
    pub top_k: Option<u32>,

    /// Repeat penalty. `None` uses llama-server default.
    pub repeat_penalty: Option<f32>,

    /// Stop sequences. Generation halts when any are produced.
    pub stop: Option<Vec<String>>,

    /// Optional JSON-Schema for grammar-constrained decoding.
    pub json_schema: Option<serde_json::Value>,

    /// Scheduling priority. Higher values run first.
    ///
    /// Priority 0 is reserved for benchmark/background collections.
    /// Normal user requests default to priority 10. The scheduler selects
    /// by `(priority DESC, created_at ASC)`.
    pub priority: i32,

    /// Priority threshold at which an incoming request on a *different* model
    /// triggers a model swap that preempts this completion.
    ///
    /// `None` uses `priority + 2` as the default.
    pub preemption_threshold: Option<i32>,

    /// Collection this completion belongs to, if any.
    pub collection_id: Option<CollectionId>,

    /// Which optional metrics to include in the result.
    pub metrics: MetricsFlags,

    /// Arbitrary caller-provided metadata (not interpreted by substrate).
    pub metadata: Option<serde_json::Value>,

    /// When the request was received / created.
    pub created_at: DateTime<Utc>,
}

impl CompletionRequest {
    /// Generate a new random completion ID (UUID v4).
    pub fn new_id() -> CompletionId {
        Uuid::new_v4()
    }

    /// Effective preemption threshold: caller-supplied or `priority + 2`.
    pub fn effective_preemption_threshold(&self) -> i32 {
        self.preemption_threshold
            .unwrap_or(self.priority + 2)
    }
}

// ── CompletionState ─────────────────────────────────────────────────────

/// Current lifecycle state of a completion.
///
/// ```text
/// Pending ──► Running ──► Completed
///                    ├──► Failed
///                    └──► Cancelled
///
/// // Backward requeue edges:
/// Running ──► Pending   (preemption or error retry)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionState {
    /// Queued, waiting for the scheduler to admit it.
    Pending,
    /// Currently executing on the engine.
    Running,
    /// Generation finished successfully.
    Completed,
    /// Terminal failure (error count exhausted or non-retryable error).
    Failed,
    /// Cancelled by the caller or by collection cancellation.
    Cancelled,
}

impl CompletionState {
    /// Returns `true` for states that permit no further transitions.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Canonical string form (matches the serde `rename_all = "snake_case"`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse from the canonical string. Returns `None` on unrecognised input.
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

impl std::fmt::Display for CompletionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ── Termination taxonomy ────────────────────────────────────────────────

/// Why a completion left the Running state.
///
/// Carried in [`CompletionResult`] and in [`crate::stream::StreamEvent`] terminal
/// variants so callers know exactly what happened.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminationReason {
    /// Normal completion: the model stopped generating.
    Completed(StopReason),
    /// Preempted back to Pending (may be requeued).
    Preempted(PreemptionReason),
    /// Terminal failure.
    Failed(ErrorKind),
    /// Explicit cancellation.
    Cancelled,
}

/// Why the model stopped generating tokens during a successful completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Hit `max_tokens` limit.
    Length,
    /// A stop sequence was produced.
    Stop,
    /// End-of-sequence token.
    Eos,
}

/// Why a running completion was preempted back to pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreemptionReason {
    /// A higher-priority request arrived requiring a model swap.
    ModelSwap,
    /// System memory pressure exceeded the hard threshold.
    MemoryPressure,
    /// Node shutdown or daemon restart.
    Shutdown,
}

/// Classification of a terminal completion failure.
///
/// Variants that carry a `String` payload include the underlying error message.
/// `substrate-types` uses `String` here (not wrapped error types) so this crate
/// remains zero-dependency — downstream crates convert their native errors before
/// constructing these variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The engine returned an error response (non-retryable as classified by engine).
    EngineError(String),
    /// Network or transport error communicating with the engine.
    TransportError(String),
    /// The prompt exceeds the model's context window.
    ContextLengthExceeded,
    /// Preemption count exceeded `max_preemption_count`.
    PreemptionLimitExceeded,
    /// Retry limit exceeded after repeated transient errors.
    RetryLimitExceeded,
    /// An internal substrate error.
    Internal(String),
}

// ── CompletionMetrics ───────────────────────────────────────────────────

/// Per-completion timing and token metrics.
///
/// Fields are `Option` because they are gated by [`MetricsFlags`]. Only fields
/// corresponding to enabled flags will be populated; all others remain `None`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompletionMetrics {
    /// Milliseconds from queue entry to engine start.
    pub queue_latency_ms: Option<u64>,

    /// Milliseconds from first token request to last token received.
    pub generation_ms: Option<u64>,

    /// Number of prompt tokens processed.
    pub prompt_tokens: Option<u32>,

    /// Number of tokens generated.
    pub completion_tokens: Option<u32>,

    /// Tokens per second (completion_tokens / generation_ms * 1000).
    pub tokens_per_second: Option<f32>,

    /// Number of times this completion was preempted and requeued.
    pub preemption_count: u32,

    /// Number of times this completion was retried after a transient error.
    pub error_retry_count: u32,
}

// ── CompletionResult ────────────────────────────────────────────────────

/// The result of a completion that has reached a terminal state.
///
/// Returned by `Store::get_result` and delivered via [`crate::promise::Promise`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResult {
    /// The completion's stable identity.
    pub id: CompletionId,

    /// Terminal lifecycle state.
    pub state: CompletionState,

    /// Generated text. `None` for cancelled or immediately-failed completions
    /// that produced no output.
    pub text: Option<String>,

    /// Why the completion terminated.
    pub termination: Option<TerminationReason>,

    /// Captured metrics (fields populated based on `MetricsFlags` at submit time).
    pub metrics: CompletionMetrics,

    /// When the completion transitioned to its terminal state.
    pub completed_at: DateTime<Utc>,
}

// ── CompletionRow ───────────────────────────────────────────────────────

/// Full completion record as stored in SQLite.
///
/// Maps 1:1 to the `completions` table columns. Used by `substrate-store`
/// for all persistence operations. Not serialised to external clients.
#[derive(Debug, Clone)]
pub struct CompletionRow {
    pub id: CompletionId,
    pub model_id: ModelId,
    pub prompt: String,
    /// Serialised sampling parameters (temperature, top_p, etc.) as JSON.
    pub params_json: String,
    pub priority: i32,
    pub preemption_threshold: i32,
    pub json_schema: Option<String>,
    pub collection_id: Option<CollectionId>,
    pub metrics_flags: u32,
    pub metadata_json: Option<String>,
    pub state: CompletionState,
    pub preemption_count: u32,
    pub error_retry_count: u32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub recovered_at: Option<DateTime<Utc>>,
}
