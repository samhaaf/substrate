//! Mesh proxy API server: the same API surface as the node, forwarded.
//!
//! The proxy catches all requests under `/v1/` and `/wiki/` and forwards them
//! to the router. This maintains the transparency guarantee: clients connecting
//! to the mesh see the exact same API as clients connecting directly to a node.
//!
//! ## WebSocket streaming
//!
//! WebSocket upgrade is transparent: the proxy upgrades, establishes a WS connection
//! to the node, and relays frames bidirectionally.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, Response},
    Router,
};

use crate::router::MeshRouter;

/// Shared state for the proxy Axum app.
#[derive(Clone)]
pub struct ProxyState {
    pub router: Arc<dyn MeshRouter>,
}

/// Build the Axum router for the mesh proxy.
///
/// All routes are catch-all handlers that delegate to `MeshRouter::forward`.
pub fn router(state: ProxyState) -> Router {
    todo!(
        "Mount catch-all handler: Router::new().fallback(forward_handler). \
         Apply tower_http CorsLayer and TraceLayer. \
         Handle WS upgrade for /v1/completions/:id/stream specially."
    )
}

/// Catch-all handler: forwards any HTTP request to the backing node.
async fn forward_handler(
    State(state): State<ProxyState>,
    req: Request<Body>,
) -> Response<Body> {
    todo!(
        "Extract path + method + body from req, \
         call state.router.forward(path, method, body_bytes), \
         wrap response bytes as axum Response<Body>"
    )
}
