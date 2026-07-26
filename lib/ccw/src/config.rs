//! Daemon configuration — `~/.leverage/ccw/config.toml` (INTENT #207/#208).
//!
//! CCW v1 is a service, not a CLI: `ccw` with no args starts the daemon, reading
//! this file. Everything is optional with boring defaults so a fresh machine
//! runs with no config at all.
//!
//! ```toml
//! # ~/.leverage/ccw/config.toml
//! port = 3651                      # WS API port (see DEFAULT_PORT rationale)
//! data_dir = "~/.leverage/ccw"     # SQLite + accounts registry live here
//! claude_bin = "claude"            # overridable; CCW_CLAUDE_BIN env wins
//!
//! [accounts]
//! default = ""                     # machine default (CLAUDE_CONFIG_DIR unset)
//! work = "/Users/me/.leverage/ccw/accounts/work"
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use substrate_types::{Result, SubstrateError};

/// The fixed default WS port for the CCW daemon.
///
/// Rationale: the mesh daemon reserves **3649** (INTENT #58 single-port
/// locality). CCW is a distinct local daemon and must not collide with it, so it
/// takes **3651**, leaving 3650 as a deliberate gap for a future sibling daemon.
/// Documented here as the one source of truth (also in `contracts/ccw-api.md`).
pub const DEFAULT_PORT: u16 = 3651;

/// Parsed daemon config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// WS API port. Default [`DEFAULT_PORT`].
    #[serde(default = "default_port")]
    pub port: u16,
    /// Bind host. Default `127.0.0.1` (local-only; mesh is the remote plane).
    #[serde(default = "default_host")]
    pub host: String,
    /// State/data dir. `None` → the resolved state home (`~/.leverage/ccw`).
    #[serde(default)]
    pub data_dir: Option<String>,
    /// claude binary path override. Env `CCW_CLAUDE_BIN` takes precedence.
    #[serde(default)]
    pub claude_bin: Option<String>,
    /// Optional inline account seeds (name → CLAUDE_CONFIG_DIR; `""` = machine
    /// default). Merged into the `accounts.toml` registry on daemon open.
    #[serde(default)]
    pub accounts: BTreeMap<String, String>,
}

fn default_port() -> u16 {
    DEFAULT_PORT
}
fn default_host() -> String {
    "127.0.0.1".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            host: default_host(),
            data_dir: None,
            claude_bin: None,
            accounts: BTreeMap::new(),
        }
    }
}

impl Config {
    /// Parse from TOML text.
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(|e| SubstrateError::Config(format!("parsing config.toml: {e}")))
    }

    /// Load from a path; a missing file yields the default config.
    pub fn load(path: &std::path::Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => Self::from_toml(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(SubstrateError::Config(format!(
                "reading {}: {e}",
                path.display()
            ))),
        }
    }

    /// The resolved data dir (config override, else the state home).
    pub fn resolved_data_dir(&self) -> Result<PathBuf> {
        match &self.data_dir {
            Some(d) if !d.is_empty() => Ok(PathBuf::from(shellexpand_home(d))),
            _ => crate::home::home(),
        }
    }

    /// Apply `claude_bin` to the process env so [`crate::spawn::claude_bin`] and
    /// [`crate::claude`] pick it up, unless `CCW_CLAUDE_BIN` is already set.
    pub fn apply_claude_bin(&self) {
        if std::env::var("CCW_CLAUDE_BIN").is_err() {
            if let Some(b) = &self.claude_bin {
                if !b.is_empty() {
                    std::env::set_var("CCW_CLAUDE_BIN", b);
                }
            }
        }
    }
}

/// Expand a leading `~/` to `$HOME` (boring, no external crate).
fn shellexpand_home(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}
