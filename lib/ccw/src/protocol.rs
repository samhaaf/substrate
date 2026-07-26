//! The WebSocket wire protocol — THE data contract other Leverage services use
//! (INTENT #208). Shaped promise/event-based, consistent with the scaffold's
//! `mesh-transport` philosophy (the #154 success/error/promise `ResponseOutcome`
//! switch; a version-stamped welcome) but kept **pre-mesh boring**: one socket,
//! request/response + a pushed event stream, no relay/addressing yet.
//!
//! Full taxonomy + examples: `scaffold/contracts/ccw-api.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::event::CcwEvent;

/// Transport proto version (bumped on a breaking envelope change).
pub const PROTO: u16 = 1;

/// Client → daemon frames.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// An RPC request. `id` correlates the response; `method` routes; `params`
    /// is the per-method schema (opaque here).
    Request {
        id: String,
        method: String,
        #[serde(default)]
        params: Value,
    },
    /// Subscribe to the event stream. `sessions: None` = firehose (every
    /// session); `Some([...])` = only those session ids.
    Subscribe {
        #[serde(default)]
        sessions: Option<Vec<String>>,
    },
    /// Narrow/clear the subscription.
    Unsubscribe {
        #[serde(default)]
        sessions: Option<Vec<String>>,
    },
}

/// Daemon → client frames.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// First frame on a fresh connection: proto + daemon version (sender
    /// stamping, INTENT #113 — kept minimal pre-mesh).
    Welcome {
        proto: u16,
        daemon: &'static str,
        daemon_version: &'static str,
    },
    /// Answers a `Request.id`. Carries the #154 three-arm outcome switch.
    Response {
        correlate: String,
        outcome: Outcome,
    },
    /// A pushed session event (seq-ordered per session).
    Event {
        session_id: String,
        seq: u64,
        event: CcwEvent,
    },
    /// Acknowledges a subscribe/unsubscribe (with the resulting filter).
    SubAck {
        firehose: bool,
        sessions: Vec<String>,
    },
}

/// THE success / error / promise switch (INTENT #154). Every response is exactly
/// one arm; callers MUST handle all three (a `Promise` is a normal success, not
/// an error). v1 never emits `Promise` for its own methods yet — the arm exists
/// so the contract is mesh-ready without a wire change.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outcome {
    Success { payload: Value },
    Error { error: String },
    Promise { promise: String },
}

impl Outcome {
    pub fn ok(payload: Value) -> Self {
        Outcome::Success { payload }
    }
    pub fn err(msg: impl Into<String>) -> Self {
        Outcome::Error { error: msg.into() }
    }
}
