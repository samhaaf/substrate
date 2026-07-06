//! `NodeDiscovery` trait — enumerates available substrate nodes on the network.
//!
//! ## Trait Contract
//!
//! `discover() -> Vec<NodeEndpoint>` returns all currently reachable nodes.
//! The mesh router calls this before each routing decision (implementations should cache).
//!
//! ## Implementations
//!
//! - [`StaticDiscovery`] — reads a static list of nodes from config. The v2 stub.
//!   Functional for deployments with a fixed, known set of nodes.
//!
//! - [`TailscaleDiscovery`] — auto-discovers substrate nodes on a Tailscale network
//!   by running `tailscale status --json` and filtering by node name prefix.
//!   **Currently `todo!()`** — the trait exists as a contract for future implementation.

use serde::{Deserialize, Serialize};
use substrate_types::Result;

use crate::config::MeshConfig;

/// A substrate node endpoint: the information the mesh needs to forward to a node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeEndpoint {
    /// Human-readable name (used in logs and load balancer decisions).
    pub name: String,

    /// Base URL of the node's API server. Example: `"http://gpu-box-1:8420"`
    pub url: String,

    /// Cached node capabilities. Populated lazily by health-check polling.
    pub capabilities: Option<NodeCapabilities>,
}

/// Capabilities reported by a node (from its `/v1/system/state`).
///
/// Populated lazily; `None` means we haven't polled this node yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCapabilities {
    /// Models currently registered on this node.
    pub registered_models: Vec<String>,

    /// Model currently loaded in the engine (if any).
    pub resident_model: Option<String>,

    /// GPU memory available in bytes (0 if no GPU or not yet sampled).
    pub gpu_memory_bytes: u64,

    /// Current memory pressure (0.0–1.0).
    pub memory_pressure: f32,

    /// Number of completions currently running.
    pub running_count: usize,
}

/// Discovers substrate nodes on the network.
pub trait NodeDiscovery: Send + Sync {
    /// Return all currently known/reachable nodes.
    fn discover(&self) -> Result<Vec<NodeEndpoint>>;
}

/// Stub: returns a static list of nodes from the mesh config.
///
/// This is the v2 functional implementation. It does not do any network
/// probing — it returns exactly what is in `[[nodes]]` in `mesh.toml`.
/// Node capabilities are `None` until health-check polling is implemented.
pub struct StaticDiscovery {
    nodes: Vec<NodeEndpoint>,
}

impl StaticDiscovery {
    /// Build from the static `[[nodes]]` list in config.
    pub fn from_config(config: &MeshConfig) -> Self {
        let nodes = config.nodes.iter().map(|n| NodeEndpoint {
            name: n.name.clone(),
            url: n.url.clone(),
            capabilities: None,
        }).collect();
        Self { nodes }
    }
}

impl NodeDiscovery for StaticDiscovery {
    fn discover(&self) -> Result<Vec<NodeEndpoint>> {
        Ok(self.nodes.clone())
    }
}

/// Stub: discovers substrate nodes via Tailscale auto-discovery.
///
/// # Future Implementation
///
/// Run `tailscale status --json`, parse the peer list, filter by a naming
/// convention (e.g., peers named `substrate-*` or in a specific Tailscale tag group),
/// and return their Tailscale IPs with the default substrate port.
///
/// This enables zero-config multi-node mesh: add a new GPU machine to the Tailscale
/// network with the right name and it will be discovered automatically.
pub struct TailscaleDiscovery {
    /// The Tailscale network name to scope discovery to.
    pub network: String,

    /// Default port to use for discovered nodes.
    pub default_port: u16,
}

impl NodeDiscovery for TailscaleDiscovery {
    fn discover(&self) -> Result<Vec<NodeEndpoint>> {
        todo!(
            "Run `tailscale status --json`, parse peers, \
             filter by substrate naming convention, \
             return NodeEndpoints with Tailscale IPs and self.default_port"
        )
    }
}
