//! `substrate-inference` — the inference daemon binary.
//!
//! Runs on each machine that has a GPU or accelerator. One instance per machine.
//! Reads `substrate.toml`, initializes all subsystems, and serves the `/v1/` API.
//!
//! ## Usage
//!
//! ```sh
//! substrate-inference                    # uses ./substrate.toml
//! substrate-inference /etc/substrate.toml
//! substrate-inference ~/.config/substrate/node.toml
//! ```
//!
//! ## Subsystem Init Order
//!
//! 1. Parse config from TOML file
//! 2. Initialize tracing (RUST_LOG env var controls verbosity)
//! 3. Open SQLite store at `data_dir/substrate.db`
//! 4. Crash recovery: requeue Running completions back to Pending
//! 5. Start telemetry polling (background)
//! 6. Sync model registry from config
//! 7. Initialize model manager (download pipeline + LRU eviction)
//! 8. Initialize KV cache manager
//! 9. Initialize execution engine (llama-server started on first request)
//! 10. Initialize scheduler (default swap evaluator + FIFO selection)
//! 11. Start benchmark orchestrator (background, priority-0 idle sweeps)
//! 12. Start scheduler loop (background)
//! 13. Bind Axum API server → block until shutdown

use anyhow::Context;
use substrate_inference::config::InferenceConfig;
use substrate_inference::InferenceService;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Step 1: Parse config path from args (default: "substrate.toml").
    let config_path = std::env::args().nth(1).unwrap_or_else(|| "substrate.toml".into());

    // Step 2: Initialize tracing.
    // RUST_LOG controls log level. Default: "substrate=info".
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "substrate=info,substrate_inference=info".into()),
        )
        .init();

    tracing::info!("loading config from {}", config_path);

    // Step 3: Load config.
    let config = InferenceConfig::from_file(&config_path)
        .with_context(|| format!("failed to load config from {config_path}"))?;

    tracing::info!("data_dir = {}", config.data_dir);
    tracing::info!("api_bind = {}", config.api_bind);
    tracing::info!("models: {}", config.models.len());

    // Steps 4–12: Initialize all subsystems (delegated to InferenceService::start).
    let node = InferenceService::start(config).await
        .context("failed to start substrate inference service")?;

    // Step 13: Bind API server and block until shutdown.
    node.serve().await
        .context("node server exited with error")?;

    Ok(())
}
