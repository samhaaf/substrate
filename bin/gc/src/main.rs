//! `substrate-gc` — HTTP API server wrapping the `substrate-gc` library.
//!
//! ## Usage
//!
//! ```sh
//! substrate-gc                      # uses default local config
//! substrate-gc --config /etc/gc.toml
//! ```
//!
//! ## Start-up sequence
//!
//! 1. Parse `--config <path>` from args.
//! 2. Load [`GcServerConfig`] from file, or fall back to [`GcServerConfig::default_local`].
//! 3. Initialise tracing (RUST_LOG controls verbosity).
//! 4. Create the data directory if it doesn't exist.
//! 5. Construct [`GcService`] over `data_dir/gc.db` with a [`DeleteReclaimer`].
//! 6. Spawn the background sweep loop.
//! 7. Build the Axum router and bind.
//! 8. Block until Ctrl-C.

mod api;
mod config;

use std::sync::Arc;

use anyhow::Context;
use substrate_gc::{DeleteReclaimer, GcConfig, GcService};
use tokio::net::TcpListener;

use config::GcServerConfig;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // ── Step 1: parse --config <path> ──────────────────────────────────────
    let cfg = parse_config_from_args()?;

    // ── Step 2: init tracing ───────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "substrate_gc=info,substrate_gc_bin=info".into()),
        )
        .init();

    tracing::info!("data_dir  = {}", cfg.data_dir);
    tracing::info!("bind_addr = {}", cfg.bind_addr);
    tracing::info!("sweep_interval_secs = {}", cfg.sweep_interval_secs);

    // ── Step 3: ensure data directory exists ──────────────────────────────
    std::fs::create_dir_all(&cfg.data_dir)
        .with_context(|| format!("creating data_dir {}", cfg.data_dir))?;

    // ── Step 4: open GcService ────────────────────────────────────────────
    let db_path = std::path::PathBuf::from(&cfg.data_dir).join("gc.db");
    let gc_cfg = GcConfig {
        db_path,
        sweep_interval_secs: cfg.sweep_interval_secs,
    };
    let svc = Arc::new(
        GcService::new(gc_cfg, Box::new(DeleteReclaimer))
            .context("failed to open GcService")?,
    );

    // ── Step 5: start background sweep loop ───────────────────────────────
    let _sweep_handle = Arc::clone(&svc).start_sweep_loop(cfg.sweep_interval_secs);

    // ── Step 6: build Axum router ─────────────────────────────────────────
    let app = api::routes().with_state(Arc::clone(&svc));

    // ── Step 7: bind and serve ────────────────────────────────────────────
    let listener = TcpListener::bind(&cfg.bind_addr)
        .await
        .with_context(|| format!("binding to {}", cfg.bind_addr))?;
    tracing::info!("listening on {}", cfg.bind_addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("server error")?;

    tracing::info!("server shut down cleanly");
    Ok(())
}

/// Wait for Ctrl-C.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    tracing::info!("received shutdown signal");
}

/// Parse `--config <path>` from argv. Returns loaded config or default.
fn parse_config_from_args() -> anyhow::Result<GcServerConfig> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            let path = args.next().context("--config requires a path argument")?;
            return GcServerConfig::from_file(std::path::Path::new(&path));
        }
    }
    Ok(GcServerConfig::default_local())
}
