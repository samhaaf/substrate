//! Substrate error taxonomy.
//!
//! [`SubstrateError`] is the canonical error type for the entire workspace.
//! All fallible public APIs return `substrate_types::Result<T>`, which aliases
//! `std::result::Result<T, SubstrateError>`.
//!
//! ## Zero-dependency invariant
//!
//! This crate has no dependency on `rusqlite`, `reqwest`, `axum`, or any other
//! I/O library. Variants that wrap errors from those layers carry a `String`
//! payload (the `.to_string()` of the original error). Downstream crates convert
//! their native errors using `map_err(|e| SubstrateError::Store(e.to_string()))`,
//! etc.
//!
//! This keeps `substrate-types` at the bottom of the dependency graph — nothing
//! else depends on it, and it depends on nothing (beyond `thiserror`).

use thiserror::Error;

use crate::completion::{CompletionId, CompletionState};
use crate::collection::{CollectionId, CollectionState};
use crate::model::ModelId;

/// The canonical substrate error type.
///
/// Variants are grouped by domain: entity lookup, state conflicts, resource
/// pressure, engine errors, persistence, configuration, and a catch-all.
#[derive(Debug, Error)]
pub enum SubstrateError {
    // ── Entity not found ───────────────────────────────────────────────

    #[error("completion not found: {0}")]
    CompletionNotFound(CompletionId),

    #[error("collection not found: {0}")]
    CollectionNotFound(CollectionId),

    #[error("model not found: {0}")]
    ModelNotFound(ModelId),

    // ── Conflict / invalid state ───────────────────────────────────────

    #[error("completion {id} is in terminal state {state} and cannot be modified")]
    CompletionTerminal {
        id: CompletionId,
        state: CompletionState,
    },

    #[error("collection {id} is in terminal state {state} and cannot be modified")]
    CollectionTerminal {
        id: CollectionId,
        state: CollectionState,
    },

    #[error("collection {collection_id} requires all members to target model {expected}, but completion targets {actual}")]
    CollectionModelMismatch {
        collection_id: CollectionId,
        expected: ModelId,
        actual: ModelId,
    },

    #[error("empty collection: collections must have at least one member")]
    EmptyCollection,

    #[error("model is not downloaded: {0}")]
    ModelNotDownloaded(ModelId),

    #[error("model is already loaded: {0}")]
    ModelAlreadyLoaded(ModelId),

    #[error("model swap is in progress; try again shortly")]
    SwapInProgress,

    #[error("invalid request: {0}")]
    InvalidRequest(String),

    // ── Resource pressure ──────────────────────────────────────────────

    #[error("prompt exceeds model context length ({prompt_tokens} tokens > {context_length} token limit)")]
    ContextLengthExceeded {
        prompt_tokens: u32,
        context_length: u32,
    },

    #[error("completion {0} exceeded max preemption count ({1}); resource exhausted")]
    ResourceExhausted(CompletionId, u32),

    #[error("disk budget exhausted: cannot accommodate {requested_bytes} bytes")]
    DiskBudgetExceeded { requested_bytes: u64 },

    #[error("memory pressure too high to admit new work: {pressure:.1}%")]
    MemoryPressure { pressure: f32 },

    #[error("model download failed for {model}: {message}")]
    DownloadFailed { model: ModelId, message: String },

    // ── Engine errors ──────────────────────────────────────────────────

    /// llama-server returned an HTTP error or unexpected response.
    ///
    /// The payload is the stringified error from `reqwest` (which `substrate-engine`
    /// converts before constructing this variant).
    #[error("engine error: {0}")]
    Engine(String),

    /// Transport/network error communicating with llama-server.
    #[error("engine transport error: {0}")]
    Transport(String),

    /// llama-server health check failed after N attempts.
    #[error("llama-server health check failed after {attempts} attempts")]
    ServerUnhealthy { attempts: u32 },

    /// llama-server child process exited unexpectedly.
    #[error("llama-server process crashed: {0}")]
    ServerCrashed(String),

    // ── Persistence ────────────────────────────────────────────────────

    /// SQLite/store error. Payload is `rusqlite::Error::to_string()`.
    ///
    /// `substrate-store` converts `rusqlite::Error` into this variant so
    /// callers do not need to import `rusqlite`.
    #[error("store error: {0}")]
    Store(String),

    // ── Database utility (`substrate-db`) ─────────────────────────────

    /// Generic error from the `db` migration/control-plane utility.
    ///
    /// Payload is the stringified underlying error (Postgres client, toml,
    /// Management API, Supabase CLI, etc.) so `substrate-types` stays free of
    /// those dependencies.
    #[error("db error: {0}")]
    Db(String),

    /// A `db` command/driver-capability combination that is not implemented
    /// (e.g. edge deploy on the sqlite driver). Emitted early, before any work,
    /// when a command is run against a driver that lacks the capability.
    #[error("`{command}` is not implemented for the {driver} driver: {reason}")]
    NotImplemented {
        command: &'static str,
        driver: &'static str,
        reason: &'static str,
    },

    // ── Config / parse ─────────────────────────────────────────────────

    #[error("config error: {0}")]
    Config(String),

    // ── I/O ───────────────────────────────────────────────────────────

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    // ── Catch-all ─────────────────────────────────────────────────────

    #[error("internal error: {0}")]
    Internal(String),
}

/// Workspace-wide result alias.
///
/// All fallible substrate APIs return this type.
pub type Result<T> = std::result::Result<T, SubstrateError>;
