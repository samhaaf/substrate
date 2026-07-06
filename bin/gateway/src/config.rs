//! Gateway configuration.
//!
//! Loaded from a TOML file or defaulted to local single-node setup.

#[derive(Debug, serde::Deserialize)]
pub struct GatewayConfig {
    /// Bind address for the gateway HTTP/WebSocket server. Default: "0.0.0.0:8400"
    pub bind_addr: String,
    /// URL of the upstream inference service. Default: "http://127.0.0.1:8420"
    pub inference_url: String,
    /// URL of the upstream GC service. Default: "http://127.0.0.1:8430"
    pub gc_url: String,
    /// Path to ui/dashboard/dist/ for serving the frontend. None disables static serving.
    pub static_dir: Option<String>,
    /// Logical node identifier included in every event envelope. Default: "local"
    pub node_id: String,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:8400".to_string(),
            inference_url: "http://127.0.0.1:8420".to_string(),
            gc_url: "http://127.0.0.1:8430".to_string(),
            static_dir: None,
            node_id: "local".to_string(),
        }
    }
}

impl GatewayConfig {
    /// Load config from a TOML file.
    pub fn from_file(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("failed to read config {:?}: {}", path, e))?;
        let config: Self = toml::from_str(&text)
            .map_err(|e| anyhow::anyhow!("failed to parse config {:?}: {}", path, e))?;
        Ok(config)
    }

    /// Default single-node local config (no file required).
    pub fn default_local() -> Self {
        Self::default()
    }
}
