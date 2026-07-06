//! Promise mechanism for in-process completion delivery.
//!
//! When a caller submits a completion through the in-process API (`Node::submit`),
//! they receive a [`Promise`] that resolves when the completion reaches a terminal
//! state. This is a thin wrapper over `tokio::sync::oneshot` that carries a typed
//! `CompletionResult`.
//!
//! The sender half ([`PromiseSender`]) lives in the scheduler; the receiver half
//! ([`Promise`]) is returned to the caller. Use [`promise_pair`] to create a linked
//! `(Promise, PromiseSender)` pair.
//!
//! ## Note on streaming
//!
//! `Promise` supports only the final result. Token streaming is handled separately
//! by the WebSocket layer in `substrate-api` via the event bus — not through this
//! mechanism. This keeps `Promise` simple and avoids coupling the scheduler to the
//! WS transport.

use tokio::sync::oneshot;

use crate::completion::CompletionResult;
use crate::error::{Result, SubstrateError};

// ── Promise ─────────────────────────────────────────────────────────────

/// The receiver half of a completion promise.
///
/// Obtained from `Node::submit` (in-process API). Awaiting this resolves when
/// the completion reaches any terminal state (Completed, Failed, or Cancelled).
pub struct Promise {
    /// Stable completion identity (survives preemption/requeue cycles).
    pub id: crate::completion::CompletionId,
    receiver: oneshot::Receiver<CompletionResult>,
}

impl Promise {
    /// Wait for the completion to finish and return its result.
    ///
    /// Returns `Err(SubstrateError::Internal)` only if the sender was dropped
    /// without fulfilling — which indicates a scheduler bug.
    pub async fn await_result(self) -> Result<CompletionResult> {
        self.receiver.await.map_err(|_| {
            SubstrateError::Internal(
                "promise sender dropped before fulfillment".into(),
            )
        })
    }
}

// ── PromiseSender ────────────────────────────────────────────────────────

/// The sender half of a completion promise. Held by the scheduler.
///
/// When a completion reaches a terminal state, the scheduler calls
/// [`PromiseSender::fulfill`] to deliver the result to the waiting caller.
pub struct PromiseSender {
    pub id: crate::completion::CompletionId,
    sender: oneshot::Sender<CompletionResult>,
}

impl PromiseSender {
    /// Fulfill the promise with a completion result.
    ///
    /// Returns the `result` inside `Err(result)` if the receiver was already
    /// dropped (i.e., the caller abandoned the promise before it resolved).
    /// This is not an error from the system's perspective — callers may cancel
    /// interest at any time.
    pub fn fulfill(self, result: CompletionResult) -> std::result::Result<(), CompletionResult> {
        self.sender.send(result)
    }
}

// ── Constructor ──────────────────────────────────────────────────────────

/// Create a linked `(Promise, PromiseSender)` pair for a given completion ID.
///
/// The scheduler calls this at submission time and retains the `PromiseSender`.
/// The `Promise` is returned to the API caller.
pub fn promise_pair(id: crate::completion::CompletionId) -> (Promise, PromiseSender) {
    let (sender, receiver) = oneshot::channel();
    (
        Promise { id, receiver },
        PromiseSender { id, sender },
    )
}
