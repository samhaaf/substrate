//! Event hub — fan-out broadcaster for gateway events.
//!
//! All upstream connectors publish to the Hub; all WebSocket client handlers
//! subscribe to it. Uses a tokio broadcast channel so slow clients are dropped
//! at the channel boundary rather than blocking fast producers.

use tokio::sync::broadcast;

use crate::topics::GatewayEvent;

/// Channel capacity: how many events can be buffered before lagging receivers
/// start missing events. 1024 is generous for a local-first deployment.
const CHANNEL_CAPACITY: usize = 1024;

/// The shared event hub. Cheap to clone (Arc-backed internally).
#[derive(Clone)]
pub struct Hub {
    tx: broadcast::Sender<GatewayEvent>,
}

impl Hub {
    /// Create a new Hub with a fresh broadcast channel.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx }
    }

    /// Publish an event to all active subscribers.
    ///
    /// Returns the number of receivers that got the message.
    /// Silently drops the send if there are no subscribers (this is normal at startup).
    pub fn publish(&self, event: GatewayEvent) -> usize {
        self.tx.send(event).unwrap_or(0)
    }

    /// Subscribe to the hub. The returned receiver will receive all future
    /// events published after the subscription is created.
    ///
    /// If the receiver falls more than `CHANNEL_CAPACITY` events behind it
    /// will receive a `RecvError::Lagged` and can re-subscribe.
    pub fn subscribe(&self) -> broadcast::Receiver<GatewayEvent> {
        self.tx.subscribe()
    }
}

impl Default for Hub {
    fn default() -> Self {
        Self::new()
    }
}
