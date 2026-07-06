//! WebSocket streaming: `/v1/completions/:id/stream`
//!
//! When a client connects, the handler:
//! 1. Verifies the completion exists in the store (returns 404 as a close if not)
//! 2. Upgrades to WebSocket
//! 3. Polls the store every 100ms for state changes
//! 4. Sends `StreamEvent::Heartbeat` every 5 seconds to keep the connection alive
//! 5. Closes when the completion reaches a terminal state
//!
//! ## Polling Fallback
//!
//! The engine does not yet expose a broadcast channel for token events. This
//! implementation uses a polling loop over the store's completion state as a
//! lightweight placeholder. A future phase will wire up the engine's
//! `mpsc::channel::<StreamEvent>` (created in `Scheduler::admit_pending`) to
//! a `tokio::sync::broadcast` sender and replace the poll loop with a
//! subscriber.
//!
//! ## Token Events
//!
//! Actual token text is not yet persisted to the store (tokens flow through
//! the engine's channel only). Until the broadcast seam is in place, clients
//! receive lifecycle events (Started, Heartbeat, terminal) but not individual
//! Token events.

use std::time::{Duration, Instant};

use axum::{
    extract::{Path, State, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Router,
};
use axum::extract::ws::{Message, WebSocket};
use substrate_types::{CompletionId, CompletionState, LifecycleEvent, StreamEvent};

use crate::ApiState;

/// How often to poll the store for state changes.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How often to send a heartbeat to keep the connection alive through proxies.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);

/// Mount all WebSocket routes onto a router.
pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/v1/completions/:id/stream", get(completion_stream))
        .route("/events", get(lifecycle_events))
}

/// GET /v1/completions/:id/stream — WebSocket token stream.
///
/// Upgrades the connection if the completion exists. If the completion is not
/// found the upgrade still proceeds (WebSocket protocol requires it), but the
/// handler immediately sends a terminal event and closes.
pub async fn completion_stream(
    State(state): State<ApiState>,
    Path(id): Path<CompletionId>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_stream(state, id, socket))
}

async fn handle_stream(state: ApiState, id: CompletionId, mut socket: WebSocket) {
    // Verify the completion exists before entering the event loop.
    match state.store.get_completion(id) {
        Err(_) => {
            // Completion not found — send an error event and close.
            let msg = serde_json::json!({
                "type": "error",
                "id": id,
                "message": "completion not found"
            });
            let _ = socket
                .send(Message::Text(msg.to_string().into()))
                .await;
            let _ = socket.close().await;
            return;
        }
        Ok(row) if row.state.is_terminal() => {
            // Already terminal — send a single lifecycle close event and exit.
            send_terminal_event(&mut socket, id, row.state).await;
            let _ = socket.close().await;
            return;
        }
        Ok(row) if row.state == CompletionState::Running => {
            // Send a synthetic Started so the client knows generation is active.
            let event = StreamEvent::Started { id };
            let _ = send_event(&mut socket, &event).await;
        }
        Ok(_) => {
            // Pending — do not send Started yet; the poll loop will catch it.
        }
    }

    // --- Poll loop -----------------------------------------------------------

    let mut last_state = CompletionState::Pending;
    let mut last_heartbeat = Instant::now();

    loop {
        tokio::time::sleep(POLL_INTERVAL).await;

        // Send a heartbeat if enough time has elapsed.
        if last_heartbeat.elapsed() >= HEARTBEAT_INTERVAL {
            let event = StreamEvent::Heartbeat { id };
            if send_event(&mut socket, &event).await.is_err() {
                // Client disconnected.
                return;
            }
            last_heartbeat = Instant::now();
        }

        // Poll the store for the current state.
        let row = match state.store.get_completion(id) {
            Ok(r) => r,
            Err(_) => {
                // Row disappeared — close.
                let _ = socket.close().await;
                return;
            }
        };

        // Emit Started if the completion just transitioned to Running.
        if last_state != CompletionState::Running
            && row.state == CompletionState::Running
        {
            let event = StreamEvent::Started { id };
            if send_event(&mut socket, &event).await.is_err() {
                return;
            }
        }

        last_state = row.state;

        if row.state.is_terminal() {
            send_terminal_event(&mut socket, id, row.state).await;
            let _ = socket.close().await;
            return;
        }
    }
}

/// Serialize a `StreamEvent` as a JSON Text WebSocket message.
/// Returns `Err(())` if the socket send failed (client disconnected).
async fn send_event(socket: &mut WebSocket, event: &StreamEvent) -> Result<(), ()> {
    match serde_json::to_string(event) {
        Ok(json) => socket
            .send(Message::Text(json.into()))
            .await
            .map_err(|_| ()),
        Err(_) => Err(()),
    }
}

/// Send the appropriate terminal StreamEvent for a completion's final state.
async fn send_terminal_event(
    socket: &mut WebSocket,
    id: CompletionId,
    state: CompletionState,
) {
    use substrate_types::{CompletionMetrics, ErrorKind, StopReason, TerminationReason};

    let event = match state {
        CompletionState::Completed => StreamEvent::Completed {
            id,
            termination: TerminationReason::Completed(StopReason::Eos),
            metrics: CompletionMetrics::default(),
        },
        CompletionState::Failed => StreamEvent::Failed {
            id,
            termination: TerminationReason::Failed(ErrorKind::Internal(
                "completion failed".into(),
            )),
            metrics: CompletionMetrics::default(),
        },
        CompletionState::Cancelled => StreamEvent::Cancelled { id },
        // Pending/Running shouldn't reach here, but handle defensively.
        _ => StreamEvent::Cancelled { id },
    };

    let _ = send_event(socket, &event).await;
}

// ── /events — lifecycle event stream ─────────────────────────────────────

/// GET /events — WebSocket stream of node-level lifecycle events.
///
/// Any client that connects receives all [`LifecycleEvent`]s that occur after
/// the connection is established, serialized as JSON. The stream is infinite;
/// it closes only when the client disconnects or the server shuts down.
///
/// ## Events forwarded
///
/// - Model lifecycle: `ModelLoading`, `ModelLoaded`, `ModelUnloading`,
///   `ModelUnloaded`, `ModelSwapping`
/// - Backend lifecycle: `BackendStarting`, `BackendReady`, `BackendStopping`,
///   `BackendStopped`, `BackendInstalling`, `BackendInstalled`
/// - Queue state: `QueueDepthChanged`, `ExecutionPaused`, `ExecutionResumed`
/// - Throughput samples: `CompletionMetricsRecorded` (an aggregate throughput
///   sample, not completion-scoped lifecycle state — forwarded so dashboards
///   can render live per-inference throughput)
///
/// Completion/collection lifecycle events (e.g. `CompletionSubmitted`,
/// `CompletionCompleted`) are intentionally NOT forwarded here — use
/// `/v1/completions/:id/stream` for those.
pub async fn lifecycle_events(
    State(state): State<ApiState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_lifecycle_events(state, socket))
}

async fn handle_lifecycle_events(state: ApiState, mut socket: WebSocket) {
    let mut rx = state.event_tx.subscribe();

    loop {
        match rx.recv().await {
            Ok(event) => {
                // Skip completion/collection events — those belong to /v1/completions/:id/stream.
                if is_external_event(&event) {
                    match serde_json::to_string(&event) {
                        Ok(json) => {
                            if socket
                                .send(Message::Text(json.into()))
                                .await
                                .is_err()
                            {
                                // Client disconnected.
                                return;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("failed to serialize LifecycleEvent: {e}");
                        }
                    }
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                // Slow consumer; some events were dropped. Notify the client.
                tracing::warn!("lifecycle_events WebSocket receiver lagged by {n} events");
                let msg = serde_json::json!({
                    "type": "lagged",
                    "dropped": n,
                });
                if socket
                    .send(Message::Text(msg.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                // The broadcast channel was closed (server shutdown).
                let _ = socket.close().await;
                return;
            }
        }
    }
}

/// Returns `true` for events that should be forwarded to external WebSocket clients.
///
/// Completion/collection state transitions are excluded because they are scoped
/// to a specific completion ID and are handled by `/v1/completions/:id/stream`.
fn is_external_event(event: &LifecycleEvent) -> bool {
    !matches!(
        event,
        LifecycleEvent::CompletionSubmitted { .. }
            | LifecycleEvent::CompletionStarted { .. }
            | LifecycleEvent::CompletionCompleted { .. }
            | LifecycleEvent::CompletionFailed { .. }
            | LifecycleEvent::CompletionCancelled { .. }
            | LifecycleEvent::CompletionPreempted { .. }
            | LifecycleEvent::CollectionCompleted { .. }
            | LifecycleEvent::CollectionFailed { .. }
            | LifecycleEvent::CollectionCancelled { .. }
    )
}
