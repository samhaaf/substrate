//! Client-facing WebSocket handler.
//!
//! Clients connect to `GET /events`, send subscribe/unsubscribe messages,
//! and receive `GatewayEvent` JSON frames filtered to their subscribed topics.
//!
//! ## Protocol
//!
//! Client → Gateway:
//! ```json
//! { "action": "subscribe",   "topics": ["lifecycle", "queue"] }
//! { "action": "unsubscribe", "topics": ["queue"] }
//! ```
//!
//! Gateway → Client:
//! ```json
//! { "topic": "lifecycle", "service": "inference", "node_id": "local", "ts": 1234567890000, "event": {...} }
//! ```
//!
//! On connect the gateway immediately sends a synthetic `gateway.connected` event.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use tokio::sync::broadcast;
use futures::{SinkExt, StreamExt};

use crate::state::GatewayState;
use crate::topics::{ClientMessage, GatewayEvent, Topic};

/// Ping interval — keeps the connection alive through proxies.
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Axum extractor handler: upgrades HTTP GET /events to a WebSocket.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Drives a single client WebSocket connection.
async fn handle_socket(socket: WebSocket, state: Arc<GatewayState>) {
    let hub_rx = state.hub.subscribe();
    let node_id = state.config.node_id.clone();

    if let Err(e) = drive_socket(socket, hub_rx, node_id).await {
        tracing::debug!(error = %e, "WebSocket client disconnected");
    }
}

async fn drive_socket(
    socket: WebSocket,
    mut hub_rx: broadcast::Receiver<GatewayEvent>,
    node_id: String,
) -> anyhow::Result<()> {
    let (mut sender, mut receiver) = socket.split();

    // Topics this client is currently subscribed to.
    let mut subscriptions: HashSet<Topic> = HashSet::new();

    // Send initial "connected" event.
    let connected_event = GatewayEvent {
        topic: Topic::Lifecycle,
        service: "gateway".to_string(),
        node_id: node_id.clone(),
        ts: GatewayEvent::now_millis(),
        event: serde_json::json!({
            "type": "gateway.connected",
            "node_id": node_id,
        }),
    };
    let connected_json = serde_json::to_string(&connected_event)?;
    sender.send(Message::Text(connected_json.into())).await?;

    // Ping ticker.
    let mut ping_interval = tokio::time::interval(PING_INTERVAL);
    ping_interval.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            // Inbound message from client.
            client_msg = receiver.next() => {
                match client_msg {
                    None => {
                        // Client closed the connection.
                        tracing::debug!("WebSocket client disconnected");
                        break;
                    }
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "WebSocket receive error");
                        break;
                    }
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMessage>(&text) {
                            Ok(ClientMessage::Subscribe { topics }) => {
                                tracing::debug!(?topics, "client subscribed");
                                subscriptions.extend(topics);
                            }
                            Ok(ClientMessage::Unsubscribe { topics }) => {
                                tracing::debug!(?topics, "client unsubscribed");
                                for t in topics {
                                    subscriptions.remove(&t);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(error = %e, raw = %text, "invalid client message");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        tracing::debug!("WebSocket client sent close frame");
                        break;
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(_)) => {} // Ping, Binary — ignore
                }
            }

            // Hub event from upstream services.
            hub_result = hub_rx.recv() => {
                match hub_result {
                    Ok(event) => {
                        if should_forward(&event.topic, &subscriptions) {
                            let json = serde_json::to_string(&event)?;
                            if sender.send(Message::Text(json.into())).await.is_err() {
                                break; // client gone
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "WebSocket client lagged; {} events dropped", n);
                        // Continue — client will miss those events but stays connected.
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!("Hub channel closed");
                        break;
                    }
                }
            }

            // Periodic ping.
            _ = ping_interval.tick() => {
                if sender.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

/// Returns true if the event topic matches any of the client's subscriptions.
fn should_forward(topic: &Topic, subscriptions: &HashSet<Topic>) -> bool {
    if subscriptions.contains(&Topic::All) {
        return true;
    }
    subscriptions.contains(topic)
}
