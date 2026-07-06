//! `MeshConfig` — parsed from `mesh.toml`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use substrate_types::{Result, SubstrateError};

/// Top-level mesh proxy configuration. Parsed from `mesh.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshConfig {
    /// Address to bind the mesh proxy API server.
    /// Format: `"0.0.0.0:8419"` (all interfaces) or `"127.0.0.1:8419"` (local).
    #[serde(default = "default_api_bind")]
    pub api_bind: String,

    /// Routing strategy (controls which `MeshRouter` implementation is used).
    /// Currently only `"single"` is supported.
    #[serde(default)]
    pub routing: RoutingConfig,

    /// Static list of known nodes. Used by `StaticDiscovery`.
    #[serde(default)]
    pub nodes: Vec<NodeEntry>,

    /// (Optional) Tailscale auto-discovery config.
    /// Requires `TailscaleDiscovery` to be implemented.
    pub discovery: Option<DiscoveryConfig>,
}

/// Routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// Strategy name: `"single"`, `"round_robin"`, `"model_affinity"`, `"least_loaded"`.
    /// Only `"single"` is implemented in v2.
    #[serde(default = "default_strategy")]
    pub strategy: String,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self { strategy: default_strategy() }
    }
}

/// A statically configured node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEntry {
    /// Human-readable name for this node (used in logs).
    pub name: String,

    /// Base URL of the node's API server.
    /// Example: `"http://127.0.0.1:8420"`, `"http://gpu-box-1:8420"`
    pub url: String,
}

/// Discovery configuration (for future Tailscale integration).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    /// Discovery method: `"static"` or `"tailscale"`.
    pub method: String,

    /// Tailscale network name (used when `method = "tailscale"`).
    pub network: Option<String>,
}

impl MeshConfig {
    /// Parse a `MeshConfig` from a TOML file at `path`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| SubstrateError::Config(format!("cannot read mesh config: {e}")))?;
        toml::from_str(&content)
            .map_err(|e| SubstrateError::Config(format!("invalid mesh config TOML: {e}")))
    }
}

fn default_api_bind() -> String   { "0.0.0.0:8419".into() }
fn default_strategy() -> String   { "single".into() }
