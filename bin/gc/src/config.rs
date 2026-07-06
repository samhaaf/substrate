//! Configuration for the gc API server.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Top-level server configuration, loaded from a TOML file.
#[derive(Debug, Deserialize)]
pub struct GcServerConfig {
    /// Directory where `gc.db` lives.
    pub data_dir: String,
    /// Address to bind the HTTP server to, e.g. `"127.0.0.1:8430"`.
    pub bind_addr: String,
    /// How often the background sweep loop runs, in seconds. Default 3600.
    pub sweep_interval_secs: u64,
}

impl GcServerConfig {
    /// Load config from a TOML file at `path`.
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading gc config {}", path.display()))?;
        toml::from_str(&text)
            .with_context(|| format!("parsing gc config {}", path.display()))
    }

    /// Sensible local defaults: `~/.substrate` db dir, port 8430, hourly sweep.
    pub fn default_local() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Self {
            data_dir: format!("{home}/.substrate"),
            bind_addr: "127.0.0.1:8430".into(),
            sweep_interval_secs: 3600,
        }
    }
}
