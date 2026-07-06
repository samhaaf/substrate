//! `substrate-mesh` — the mesh proxy binary.
//!
//! User-facing entry point. Can run on any machine (does not need a GPU).
//! Reads `mesh.toml`, discovers nodes, and serves the same `/v1/` API surface
//! as the node — forwarding all requests to the appropriate backend node.
//!
//! ## Usage
//!
//! ```sh
//! substrate-mesh                  # uses ./mesh.toml
//! substrate-mesh /etc/mesh.toml
//! ```
//!
//! ## Subsystem Init Order
//!
//! 1. Parse config from TOML file
//! 2. Initialize tracing (RUST_LOG env var)
//! 3. Node discovery: read static node list from config (or Tailscale, when implemented)
//! 4. Load balancer: initialize `PassthroughBalancer` (or configured strategy)
//! 5. Router: initialize `SingleNodeRouter` with the first configured node
//! 6. Bind Axum proxy server → block until shutdown
//!
//! ## Transparency Guarantee
//!
//! The mesh proxy exposes the exact same REST + WebSocket API as the node.
//! Clients do not know whether they are talking to a node directly or through
//! the mesh. Routing strategy can be changed in config without client changes.
//!
//! ## Multi-Node Future
//!
//! When `MultiNodeRouter` is implemented:
//! - `StaticDiscovery` → `TailscaleDiscovery` (zero-config node enumeration)
//! - `PassthroughBalancer` → `ModelAffinityBalancer` (route to node with model loaded)
//! - No client changes required. Only `mesh.toml` routing strategy changes.

use anyhow::Context;
use substrate_mesh::{config::MeshConfig, MeshProxy};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Step 1: Parse config path from args (default: "mesh.toml").
    let config_path = std::env::args().nth(1).unwrap_or_else(|| "mesh.toml".into());

    // Step 2: Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "substrate_mesh=info".into()),
        )
        .init();

    tracing::info!("loading mesh config from {}", config_path);

    // Step 3: Load config.
    let config = MeshConfig::from_file(&config_path)
        .with_context(|| format!("failed to load mesh config from {config_path}"))?;

    tracing::info!("api_bind = {}", config.api_bind);
    tracing::info!("nodes: {}", config.nodes.len());
    for node in &config.nodes {
        tracing::info!("  node: {} → {}", node.name, node.url);
    }

    // Steps 4–5: Initialize node discovery, balancer, and router (in MeshProxy::start).
    let mesh = MeshProxy::start(config).await
        .context("failed to start substrate mesh proxy")?;

    // Step 6: Bind proxy server and block until shutdown.
    mesh.serve().await
        .context("mesh proxy exited with error")?;

    Ok(())
}
