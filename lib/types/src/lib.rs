//! # substrate-types
//!
//! Zero-dependency foundation crate for the substrate inference runtime.
//!
//! Every other crate in the workspace depends on this one. It contains only data
//! structures, type aliases, enums, and trivial impls — no I/O, no async logic,
//! no business logic.
//!
//! ## Module Structure
//!
//! - [`completion`] — `CompletionRequest`, `CompletionResult`, state machine enums
//! - [`collection`] — `CollectionOptions`, `CollectionState`, metrics aggregates
//! - [`model`]      — `ModelId`, `ModelConfig`, `ModelRow`, `ModelStatus`
//! - [`estimate`]   — `CompletionShape`, `Estimate`, `EstimateRegion`, `EstimateWarning`
//! - [`stream`]     — `StreamEvent` variants for WebSocket token streaming
//! - [`system`]     — `SystemState`, `NodeId`, `NodeInfo`
//! - [`error`]      — `SubstrateError`, `Result<T>` alias
//! - [`promise`]    — `Promise`, `PromiseSender`, `promise_pair`

pub mod completion;
pub mod collection;
pub mod model;
pub mod estimate;
pub mod stream;
pub mod system;
pub mod error;
pub mod promise;

// ── Flat re-exports ──────────────────────────────────────────────────────
//
// The most-used types are re-exported at crate root for ergonomics.
// Downstream crates can write `substrate_types::CompletionRequest` instead of
// `substrate_types::completion::CompletionRequest`.

pub use completion::{
    CompletionId,
    CompletionRequest,
    CompletionResult,
    CompletionState,
    CompletionRow,
    CompletionMetrics,
    TerminationReason,
    StopReason,
    PreemptionReason,
    ErrorKind,
    MetricsFlags,
};

pub use collection::{
    CollectionId,
    CollectionOptions,
    CollectionState,
    CollectionMetrics,
    CollectionRow,
};

pub use model::{
    ModelId,
    ModelConfig,
    ModelRow,
    ModelStatus,
};

pub use estimate::{
    CompletionShape,
    Estimate,
    EstimateRegion,
    EstimateWarning,
};

pub use stream::{StreamEvent, LifecycleEvent};

pub use system::{SystemState, NodeId, NodeInfo};

pub use error::{SubstrateError, Result};

pub use promise::{Promise, PromiseSender, promise_pair};
