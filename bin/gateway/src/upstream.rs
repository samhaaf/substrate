//! Upstream WebSocket connectors.
//!
//! Connects to each upstream service's `/events` WebSocket endpoint and
//! republishes all received messages to the local Hub. Reconnects automatically
//! with exponential back-off when the upstream drops the connection.

use std::sync::Arc;
use std::time::Duration;

use tokio_tungstenite::tungstenite::Message;
use futures::StreamExt;

use crate::hub::Hub;
use crate::topics::{GatewayEvent, Topic};

/// Back-off parameters for reconnect loop.
const BACKOFF_INITIAL: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(30);
const BACKOFF_MULTIPLIER: u32 = 2;

/// Derive a best-effort Topic from an upstream event payload.
///
/// Upstream services are expected to include a `"type"` or `"topic"` field.
/// Falls back to the provided `default_topic` if the field is absent.
///
/// Completion-specific streaming events (token, completed, failed) carry a
/// `completion_id` field and are routed to `Topic::Completion` so subscribers
/// to a single completion receive only their own events.
fn infer_topic(event: &serde_json::Value, default_topic: Topic) -> Topic {
    let type_str = event
        .get("type")
        .or_else(|| event.get("topic"))
        .and_then(|v| v.as_str());

    // Completion-specific streaming events carry a completion_id.
    // Route these to Topic::Completion so per-completion subscribers work.
    if let Some(id) = event.get("completion_id").and_then(|v| v.as_str()) {
        if matches!(type_str, Some("token") | Some("completed") | Some("failed")) {
            return Topic::Completion(id.to_string());
        }
    }

    match type_str {
        Some(s) if s.contains("lifecycle") || s.contains("model") || s.contains("backend") => Topic::Lifecycle,
        Some("queue_depth_changed") | Some("execution_paused") | Some("execution_resumed") => Topic::Queue,
        Some(s) if s.contains("gc") || s.contains("garbage") => Topic::Gc,
        Some(s) if s.contains("system") || s.contains("telemetry") || s.contains("cpu") || s.contains("gpu") => Topic::System,
        _ => default_topic,
    }
}

/// Connect to the inference service's `/events` WebSocket and republish all events to the Hub.
///
/// The returned `JoinHandle` represents the reconnect loop — it runs until the
/// process exits. Dropping the handle cancels the loop.
pub async fn connect_inference(
    inference_url: &str,
    inference_id: &str,
    hub: Arc<Hub>,
) -> tokio::task::JoinHandle<()> {
    // Convert http(s):// to ws(s)://
    let ws_url = format!("{}/events", inference_url.replacen("http://", "ws://", 1).replacen("https://", "wss://", 1));
    let inference_id = inference_id.to_string();

    tokio::spawn(async move {
        reconnect_loop(&ws_url, "inference", &inference_id, Topic::Lifecycle, hub).await;
    })
}

/// Connect to the GC's `/events` WebSocket and republish all events to the Hub.
pub async fn connect_gc(
    gc_url: &str,
    node_id: &str,
    hub: Arc<Hub>,
) -> tokio::task::JoinHandle<()> {
    let ws_url = format!("{}/events", gc_url.replacen("http://", "ws://", 1).replacen("https://", "wss://", 1));
    let node_id = node_id.to_string();

    tokio::spawn(async move {
        reconnect_loop(&ws_url, "gc", &node_id, Topic::Gc, hub).await;
    })
}

/// Inner reconnect loop. Connects, reads until disconnect, then waits and retries.
async fn reconnect_loop(
    ws_url: &str,
    service: &str,
    node_id: &str,
    default_topic: Topic,
    hub: Arc<Hub>,
) {
    let mut backoff = BACKOFF_INITIAL;

    loop {
        tracing::info!(service, url = ws_url, "connecting to upstream WebSocket");

        match tokio_tungstenite::connect_async(ws_url).await {
            Ok((mut stream, _response)) => {
                tracing::info!(service, url = ws_url, "upstream WebSocket connected");
                backoff = BACKOFF_INITIAL; // reset on successful connect

                // Drain messages until disconnect.
                while let Some(msg_result) = stream.next().await {
                    match msg_result {
                        Ok(Message::Text(text)) => {
                            match serde_json::from_str::<serde_json::Value>(&text) {
                                Ok(event) => {
                                    let topic = infer_topic(&event, default_topic.clone());
                                    let gateway_event = GatewayEvent {
                                        topic,
                                        service: service.to_string(),
                                        node_id: node_id.to_string(),
                                        ts: GatewayEvent::now_millis(),
                                        event,
                                    };
                                    hub.publish(gateway_event);
                                }
                                Err(e) => {
                                    tracing::warn!(service, error = %e, "failed to parse upstream event JSON");
                                }
                            }
                        }
                        Ok(Message::Binary(bytes)) => {
                            // Try to parse binary frames as UTF-8 JSON too.
                            if let Ok(text) = std::str::from_utf8(&bytes) {
                                if let Ok(event) = serde_json::from_str::<serde_json::Value>(text) {
                                    let topic = infer_topic(&event, default_topic.clone());
                                    hub.publish(GatewayEvent {
                                        topic,
                                        service: service.to_string(),
                                        node_id: node_id.to_string(),
                                        ts: GatewayEvent::now_millis(),
                                        event,
                                    });
                                }
                            }
                        }
                        Ok(Message::Ping(data)) => {
                            // Tungstenite auto-responds to pings; log at trace only.
                            tracing::trace!(service, "received ping ({} bytes)", data.len());
                        }
                        Ok(Message::Close(frame)) => {
                            tracing::info!(service, ?frame, "upstream WebSocket closed cleanly");
                            break;
                        }
                        Ok(_) => {} // Pong, Frame — ignore
                        Err(e) => {
                            tracing::warn!(service, error = %e, "upstream WebSocket error");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(service, url = ws_url, error = %e, "failed to connect to upstream WebSocket");
            }
        }

        tracing::info!(service, delay_secs = backoff.as_secs(), "reconnecting after delay");
        tokio::time::sleep(backoff).await;
        backoff = (backoff * BACKOFF_MULTIPLIER).min(BACKOFF_MAX);
    }
}
