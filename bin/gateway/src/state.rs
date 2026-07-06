//! Shared gateway state threaded through Axum via `State<Arc<GatewayState>>`.

use std::sync::Arc;

use crate::config::GatewayConfig;
use crate::hub::Hub;

/// Shared application state. Cheap to clone — all heavy objects are behind `Arc`.
pub struct GatewayState {
    pub config: GatewayConfig,
    pub hub: Hub,
    /// HTTP client for REST proxying.
    pub http: reqwest::Client,
}

impl GatewayState {
    pub fn new(config: GatewayConfig) -> Arc<Self> {
        Arc::new(Self {
            hub: Hub::new(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("failed to build HTTP client"),
            config,
        })
    }
}
