//! `ccw` — the Claude Code Wrapper daemon (INTENT #208).
//!
//! CCW v1 is a **pure service**: `ccw` with no args starts the WebSocket daemon,
//! reading `~/.leverage/ccw/config.toml` (or `$CCW_CONFIG`). There are no
//! subcommands — the v0 `run`/`status`/`budget` CLI surface was killed. Every
//! capability is reached over the WS API (`scaffold/contracts/ccw-api.md`); the
//! only argv this binary accepts is an optional config path.
//!
//! ```sh
//! ccw                     # start the daemon (default config)
//! ccw /path/config.toml   # start the daemon with an explicit config
//! ```

use std::path::PathBuf;

use substrate_ccw::config::Config;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ccw=info,substrate_ccw=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: building tokio runtime: {e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = rt.block_on(run()) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> anyhow::Result<()> {
    let config_path = resolve_config_path();
    let config = Config::load(&config_path)?;
    tracing::info!("ccw config: {} (port {})", config_path.display(), config.port);
    substrate_ccw::run_daemon(config).await?;
    Ok(())
}

/// Config path: first argv, else `$CCW_CONFIG`, else `<state home>/config.toml`.
fn resolve_config_path() -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    if let Ok(p) = std::env::var("CCW_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    substrate_ccw::home::home()
        .map(|h| h.join("config.toml"))
        .unwrap_or_else(|_| PathBuf::from("config.toml"))
}
