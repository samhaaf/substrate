//! Collection types: groups of related completions submitted as a unit.
//!
//! A collection is a named batch of completions that share a lifecycle. Collections
//! are used by the benchmark orchestrator (priority-0 sweep collections) and by
//! callers who want to submit a batch and wait for all results.
//!
//! Collections complete atomically from the scheduler's perspective: when all member
//! completions are in a terminal state, the collection transitions to Completed or Failed.
//!
//! ## V1 compatibility note
//!
//! V1 [`CollectionOptions`] carried scheduling fields (`priority`, `preemption_threshold`,
//! `request_full_system`, `start_with_no_model_loaded`, `save_partial_results`). In v2
//! those concerns are separated: scheduling priority is set per-completion, and the
//! collection record focuses on lifecycle management. This is a deliberate semantic change
//! documented in the architecture design (§4.1 "No logic changes" means type-level, not
//! field-level identity).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── Identity ────────────────────────────────────────────────────────────

/// Stable identity for a collection. Wraps UUID v4.
pub type CollectionId = Uuid;

// ── CollectionOptions ───────────────────────────────────────────────────

/// Options provided when creating a collection.
///
/// All member completions must target the same model. The scheduler enforces
/// this invariant at submission time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionOptions {
    /// Human-readable name (e.g., `"benchmark-sweep-qwen3-2026-06-26"`).
    pub name: String,

    /// Optional description.
    pub description: Option<String>,

    /// If `true`, the collection transitions to Failed and all pending members
    /// are cancelled when any member reaches Failed state.
    pub cancel_on_failure: bool,

    /// If `true`, no other work runs while this collection is active.
    /// Used by the benchmark orchestrator for full-system sweep collections.
    pub request_full_system: bool,

    /// If `true`, the current model is unloaded before the first member starts.
    /// Produces a cold-load measurement useful for benchmarking model load time.
    pub start_with_no_model_loaded: bool,

    /// If `true`, partial results from completed members are retained when the
    /// collection is preempted. Only unfinished members re-run on requeue.
    /// If `false`, all member results are discarded and every member re-runs.
    pub save_partial_results: bool,

    /// Arbitrary caller metadata (not interpreted by substrate).
    pub metadata: Option<serde_json::Value>,
}

impl Default for CollectionOptions {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: None,
            cancel_on_failure: false,
            request_full_system: false,
            start_with_no_model_loaded: false,
            save_partial_results: true,
            metadata: None,
        }
    }
}

// ── CollectionState ─────────────────────────────────────────────────────

/// Current lifecycle state of a collection.
///
/// ```text
/// Active ──► Completed
///        ├──► Failed
///        └──► Cancelled
///
/// // Backward requeue edge on preemption:
/// Active ──► Active   (re-admitted after preemption, resets Running sub-states)
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectionState {
    /// One or more members are still pending or running.
    Active,
    /// All members completed successfully.
    Completed,
    /// One or more members failed (and `cancel_on_failure` was true, or all failed).
    Failed,
    /// The collection was explicitly cancelled.
    Cancelled,
}

impl CollectionState {
    /// Returns `true` for states that permit no further transitions.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Canonical string form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    /// Parse from the canonical string. Returns `None` on unrecognised input.
    pub fn from_str_lossy(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

impl std::fmt::Display for CollectionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ── CollectionMetrics ───────────────────────────────────────────────────

/// Aggregate metrics over all members of a collection.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CollectionMetrics {
    /// Total number of member completions.
    pub total: u32,

    /// Number of members that reached Completed state.
    pub completed: u32,

    /// Number of members that reached Failed state.
    pub failed: u32,

    /// Number of members that reached Cancelled state.
    pub cancelled: u32,

    /// Sum of prompt tokens across all completed members.
    pub total_prompt_tokens: u64,

    /// Sum of generated tokens across all completed members.
    pub total_completion_tokens: u64,

    /// Average tokens per second across completed members. `None` if none completed.
    pub avg_tokens_per_second: Option<f32>,

    /// Wall-clock time from collection creation to terminal state. `None` if still active.
    pub wall_time_ms: Option<u64>,

    /// Milliseconds spent loading the model (cold-start collections only).
    pub model_load_ms: Option<u64>,
}

// ── CollectionRow ───────────────────────────────────────────────────────

/// Full collection record as stored in SQLite.
///
/// Maps 1:1 to the `collections` table columns. Used by `substrate-store`
/// for all persistence operations. Not serialised to external clients directly.
#[derive(Debug, Clone)]
pub struct CollectionRow {
    pub id: CollectionId,
    pub name: String,
    pub description: Option<String>,
    pub cancel_on_failure: bool,
    pub request_full_system: bool,
    pub start_with_no_model_loaded: bool,
    pub save_partial_results: bool,
    pub metadata_json: Option<String>,
    pub state: CollectionState,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}
