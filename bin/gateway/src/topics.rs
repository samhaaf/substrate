//! Topic definitions for the gateway pub/sub system.
//!
//! Clients subscribe to one or more topics over WebSocket and receive
//! `GatewayEvent` messages only for matching topics.

/// Topics clients can subscribe to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    /// Model/backend lifecycle events (load, unload, health changes).
    Lifecycle,
    /// Queue state changes (submitted, started, completed, failed).
    Queue,
    /// Garbage-collector events.
    Gc,
    /// CPU/GPU telemetry snapshots.
    System,
    /// Events for a specific completion ID: "completions/{id}".
    Completion(String),
    /// All topics — receive every event regardless of type.
    All,
}

/// Wire protocol message from client → gateway.
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Subscribe to additional topics.
    Subscribe { topics: Vec<Topic> },
    /// Remove a subscription.
    Unsubscribe { topics: Vec<Topic> },
}

/// Wire protocol message from gateway → client.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GatewayEvent {
    /// Which topic this event belongs to.
    pub topic: Topic,
    /// Originating service: "inference", "gc", or "gateway".
    pub service: String,
    /// Logical node identifier from config.
    pub node_id: String,
    /// Unix timestamp in milliseconds.
    pub ts: i64,
    /// Raw event payload from the upstream service.
    pub event: serde_json::Value,
}

impl GatewayEvent {
    /// Current wall-clock timestamp in milliseconds since epoch.
    pub fn now_millis() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64
    }
}
