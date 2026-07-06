//! # substrate-mesh
//!
//! Mesh proxy core: routes completion requests across a network of substrate
//! node daemons. This is the v2 two-tier boundary crate.
//!
//! ## Design Invariant
//!
//! `substrate-mesh` does NOT depend on `substrate-inference`, `substrate-engine`,
//! `substrate-store`, or any other node-internal crate. It communicates with
//! nodes exclusively over HTTP. This is the clean tier boundary that allows
//! the mesh proxy to run on any machine, including machines without a GPU.
//!
//! ## Architecture
//!
//! The mesh proxy exposes **the same REST + WebSocket API surface as the node**.
//! Clients cannot distinguish whether they are talking to a node directly or
//! through the mesh. When multi-node routing is implemented, no client changes
//! are required.
//!
//! ```text
//!   Client → mesh-proxy:8419 → node-daemon:8420 → llama-server:8421
//! ```
//!
//! ## Three Stub Seams
//!
//! 1. [`balancer::LoadBalancer`] — selects which node handles a request
//!    - Default: [`balancer::PassthroughBalancer`] (always picks the first node)
//!    - Future: `ModelAffinityBalancer`, `LeastLoadedBalancer`
//!
//! 2. [`discovery::NodeDiscovery`] — enumerates available nodes
//!    - Default: [`discovery::StaticDiscovery`] (reads list from config)
//!    - Future: [`discovery::TailscaleDiscovery`] (via `tailscale status --json`)
//!
//! 3. [`router::MeshRouter`] — picks a node and forwards the request
//!    - Default: [`router::SingleNodeRouter`] (routes everything to one URL)
//!    - Future: `MultiNodeRouter` (uses LoadBalancer + NodeDiscovery)
//!
//! ## Module Structure
//!
//! - [`config`]    — `MeshConfig` parsed from `mesh.toml`
//! - [`balancer`]  — `LoadBalancer` trait + stub implementations
//! - [`discovery`] — `NodeDiscovery` trait + stub implementations
//! - [`router`]    — `MeshRouter` trait + `SingleNodeRouter`
//! - [`proxy`]     — `MeshProxy` server: Axum app that forwards to the router

pub mod balancer;
pub mod config;
pub mod discovery;
pub mod proxy;
pub mod router;

use crate::config::MeshConfig;
use substrate_types::Result;

/// A fully initialized mesh proxy. One per deployment (can run on any machine).
pub struct MeshProxy {
    config: MeshConfig,
    router: Box<dyn router::MeshRouter>,
}

impl MeshProxy {
    /// Initialize the mesh proxy from config.
    pub async fn start(config: MeshConfig) -> Result<Self> {
        tracing::info!("substrate-mesh starting");

        // Build the discovery source.
        let discovery = Box::new(discovery::StaticDiscovery::from_config(&config));

        // Build the load balancer.
        let balancer = Box::new(balancer::PassthroughBalancer);

        // Build the router.
        let router = Box::new(router::SingleNodeRouter::new(
            config.nodes.first()
                .map(|n| n.url.clone())
                .unwrap_or_else(|| "http://127.0.0.1:8420".into()),
        ));

        tracing::info!("substrate-mesh ready, routing to {} node(s)", config.nodes.len());

        Ok(Self { config, router })
    }

    /// Bind the proxy API server and block until shutdown.
    pub async fn serve(&self) -> Result<()> {
        let addr: std::net::SocketAddr = self.config.api_bind.parse()
            .map_err(|e| substrate_types::SubstrateError::Config(
                format!("invalid mesh api_bind: {e}")
            ))?;

        tracing::info!("mesh proxy listening on {}", addr);
        todo!(
            "Build Axum router (proxy::router()), bind TcpListener, axum::serve()"
        )
    }
}
