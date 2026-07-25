//! State-home resolution: `~/.leverage/ccw/` (INTENT #207).
//!
//! Layout:
//! ```text
//! ~/.leverage/ccw/
//!   ccw.db          # SQLite (WAL) — concurrent wrappers coordinate through it
//!   accounts.toml   # account registry: name -> CLAUDE_CONFIG_DIR
//!   accounts/<name> # generated config dirs for `ccw account add` (when no path given)
//! ```

use std::path::PathBuf;

use substrate_types::{Result, SubstrateError};

/// The state-home root, honoring `CCW_HOME` for tests/overrides, else
/// `~/.leverage/ccw`.
pub fn home() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("CCW_HOME") {
        if !p.is_empty() {
            return Ok(PathBuf::from(p));
        }
    }
    let home = std::env::var("HOME")
        .map_err(|_| SubstrateError::Config("HOME is not set; cannot locate ~/.leverage".into()))?;
    Ok(PathBuf::from(home).join(".leverage").join("ccw"))
}

/// Ensure the state home exists, returning its path.
pub fn ensure_home() -> Result<PathBuf> {
    let h = home()?;
    std::fs::create_dir_all(&h)
        .map_err(|e| SubstrateError::Config(format!("creating {}: {e}", h.display())))?;
    Ok(h)
}

/// Path to the SQLite database.
pub fn db_path() -> Result<PathBuf> {
    Ok(home()?.join("ccw.db"))
}

/// Path to `accounts.toml`.
pub fn accounts_path() -> Result<PathBuf> {
    Ok(home()?.join("accounts.toml"))
}

/// The generated config-dir path for an account added without an explicit
/// `--config-dir` (`~/.leverage/ccw/accounts/<name>`).
pub fn generated_config_dir(name: &str) -> Result<PathBuf> {
    Ok(home()?.join("accounts").join(name))
}
