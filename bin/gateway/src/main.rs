//! `substrate-gateway` — unified mesh entry point.
//!
//! Listens on port 8400. Proxies REST API calls to the node (8420) and GC (8430).
//! Aggregates their WebSocket event streams into a single fan-out hub served on
//! `GET /events`, with per-client topic filtering.
//!
//! Optionally serves the compiled `ui/dashboard/dist/` frontend.
//!
//! ## Usage
//!
//! ```sh
//! substrate-gateway                   # default local config (no file needed)
//! substrate-gateway gateway.toml      # load from TOML file
//! ```
//!
//! ## Routes
//!
//! | Route             | Description                                      |
//! |-------------------|--------------------------------------------------|
//! | `GET /events`     | WebSocket: subscribe to gateway event stream     |
//! | `ANY /api/inference/*` | Proxy to inference REST API (strip `/api/inference` prefix)|
//! | `ANY /api/gc/*`   | Proxy to GC REST API (strip `/api/gc` prefix)    |
//! | `GET /*`          | Static frontend files (if `static_dir` set)      |
//! | `GET /health`     | Liveness probe                                   |

mod config;
mod hub;
mod proxy;
mod state;
mod stats;
mod topics;
mod upstream;
mod ws;

use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{any, get},
    Json, Router,
};
use stats::local_stats;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use config::GatewayConfig;
use state::GatewayState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Step 1: Load config — first CLI arg is optional path to gateway.toml.
    let config = match std::env::args().nth(1) {
        Some(path) => {
            let p = std::path::Path::new(&path);
            GatewayConfig::from_file(p)
                .with_context(|| format!("failed to load config from {path}"))?
        }
        None => GatewayConfig::default_local(),
    };

    // Step 2: Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "substrate_gateway=info".into()),
        )
        .init();

    tracing::info!("gateway starting on {}", config.bind_addr);
    tracing::info!("inference_url = {}", config.inference_url);
    tracing::info!("gc_url   = {}", config.gc_url);
    tracing::info!("node_id  = {}", config.node_id);

    // Step 3: Create shared state (hub + http client).
    let state = GatewayState::new(config);

    // Step 4: Spawn upstream connectors (these reconnect automatically).
    let inference_handle = upstream::connect_inference(
        &state.config.inference_url,
        &state.config.node_id,
        Arc::new(state.hub.clone()),
    )
    .await;

    let gc_handle = upstream::connect_gc(
        &state.config.gc_url,
        &state.config.node_id,
        Arc::new(state.hub.clone()),
    )
    .await;

    // Step 5: Build Axum router.
    let mut router: Router = Router::new()
        // WebSocket event stream.
        .route("/events", get(ws::ws_handler))
        // Gateway-native endpoints (not proxied).
        .route("/api/nodes/local/stats", get(local_stats))
        // REST proxy routes.
        .route("/api/inference/*path", any(proxy::proxy_inference))
        .route("/api/gc/*path", any(proxy::proxy_gc))
        // Liveness probe.
        .route("/health", get(health))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state.clone());

    // Optional: serve static frontend assets from disk.
    if let Some(static_dir) = &state.config.static_dir {
        use tower_http::services::ServeDir;
        let serve = ServeDir::new(static_dir).fallback(ServeDir::new(static_dir));
        router = router.fallback_service(serve);
        tracing::info!("serving static files from {}", static_dir);
    }

    // Step 6: Bind and serve.
    let bind_addr = state.config.bind_addr.parse::<std::net::SocketAddr>()
        .with_context(|| format!("invalid bind_addr: {}", state.config.bind_addr))?;

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("failed to bind {bind_addr}"))?;

    tracing::info!("listening on {}", bind_addr);

    // Step 7: Graceful shutdown on Ctrl-C.
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("gateway server error")?;

    // Cancel upstream tasks on shutdown.
    inference_handle.abort();
    gc_handle.abort();

    tracing::info!("gateway stopped");
    Ok(())
}

/// Health check endpoint.
async fn health(State(state): State<Arc<GatewayState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "node_id": state.config.node_id,
        })),
    )
}

/// Resolves when Ctrl-C (SIGINT) is received.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl-C handler");
    tracing::info!("shutdown signal received");
}
