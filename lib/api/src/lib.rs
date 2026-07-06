//! # substrate-api
//!
//! HTTP REST + WebSocket API server built on Axum.
//!
//! ## Routes
//!
//! ### Completions
//! - `POST   /v1/completions`          — submit a completion request
//! - `GET    /v1/completions/:id`      — get completion status
//! - `DELETE /v1/completions/:id`      — cancel a completion
//! - `PATCH  /v1/completions/:id/priority` — update priority
//! - `GET    /v1/completions/:id/result`   — get completion result (after terminal state)
//! - `GET    /v1/completions/:id/stream`   — WebSocket: stream tokens
//!
//! ### Collections
//! - `POST   /v1/collections`          — create a collection
//! - `GET    /v1/collections/:id`      — get collection state + metrics
//! - `DELETE /v1/collections/:id`      — cancel a collection
//!
//! ### Models
//! - `GET    /v1/models`               — list registered models
//! - `POST   /v1/models/:id/download`  — trigger model download (returns immediately)
//!
//! ### Estimation
//! - `POST   /v1/estimate`             — estimate throughput for a completion shape
//!
//! ### System
//! - `GET    /v1/system/state`         — current system snapshot (alias: /metrics)
//! - `GET    /health`                  — liveness probe
//!
//! ### Wiki (read-only knowledge endpoints)
//! - `GET    /wiki/models`             — model registry with download state
//! - `GET    /wiki/queue`              — current queue state
//! - `GET    /wiki/benchmarks`         — benchmark run history
//!
//! ## Module Structure
//!
//! - [`rest`] — REST handler implementations
//! - [`ws`]   — WebSocket streaming handlers
//! - [`wiki`] — Wiki read-only endpoints

pub mod rest;
pub mod wiki;
pub mod ws;

use std::sync::{
    atomic::AtomicBool,
    Arc,
};

use axum::Router;
use substrate_scheduler::Scheduler;
use substrate_store::Store;
use substrate_telemetry::Telemetry;
use substrate_types::LifecycleEvent;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Shared application state injected into every handler.
#[derive(Clone)]
pub struct ApiState {
    pub store: Store,
    pub scheduler: Arc<Scheduler>,
    pub telemetry: Arc<Telemetry>,
    /// Sender side of the node-level lifecycle event broadcast channel.
    ///
    /// WebSocket handlers call `.subscribe()` on this to receive all subsequent
    /// lifecycle events (model, backend, queue, pause/resume).
    pub event_tx: broadcast::Sender<LifecycleEvent>,
    /// Shared pause flag — `true` means the scheduler will stop admitting new
    /// completions. The REST pause/resume endpoints toggle this directly.
    pub paused: Arc<AtomicBool>,
}

/// Build the full Axum router with all routes mounted.
pub fn router(state: ApiState) -> Router {
    Router::new()
        .merge(rest::routes())
        .merge(ws::routes())
        .merge(wiki::routes())
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
}
