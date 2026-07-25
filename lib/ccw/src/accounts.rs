//! Account registry — `accounts.toml`: name → `CLAUDE_CONFIG_DIR` path.
//!
//! `CLAUDE_CONFIG_DIR` is Claude Code's isolation mechanism (`cc-recon.md` §1):
//! it isolates config, transcripts, AND the macOS Keychain entry (service name
//! suffixed with `sha256(dir)[0:8]`), giving true side-by-side auth.
//!
//! The machine default account (`~/.claude`, no `CLAUDE_CONFIG_DIR`) is
//! represented with `config_dir = None` and is auto-registered as `default` on
//! first run (INTENT #207-(3)): detect the already-logged-in account, make it
//! first, verify everything works on it before a second account is added.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use substrate_types::{Result, SubstrateError};

/// The name reserved for the machine default (env unset) account.
pub const DEFAULT_ACCOUNT: &str = "default";

/// One registered account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Account {
    /// `CLAUDE_CONFIG_DIR` for this account. Absent = the machine default
    /// (env var left unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
}

impl Account {
    /// The machine-default account (no config dir).
    pub fn default_machine() -> Self {
        Self { config_dir: None }
    }

    /// True if this is the machine default (env unset).
    pub fn is_machine_default(&self) -> bool {
        self.config_dir.is_none()
    }
}

/// The full registry, serialized to/from `accounts.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default)]
    pub accounts: BTreeMap<String, Account>,
}

impl Registry {
    /// Parse from TOML text.
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).map_err(|e| SubstrateError::Config(format!("parsing accounts.toml: {e}")))
    }

    /// Render to TOML text.
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self)
            .map_err(|e| SubstrateError::Config(format!("rendering accounts.toml: {e}")))
    }

    /// Load from `path`; a missing file yields an empty registry.
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

    /// Save to `path` (creating parent dirs).
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SubstrateError::Config(format!("creating {}: {e}", parent.display())))?;
        }
        std::fs::write(path, self.to_toml()?)
            .map_err(|e| SubstrateError::Config(format!("writing {}: {e}", path.display())))
    }

    /// Ensure the machine default is registered (idempotent). Returns true if it
    /// was just added (first-run behavior, INTENT #207).
    pub fn ensure_default(&mut self) -> bool {
        if self.accounts.contains_key(DEFAULT_ACCOUNT) {
            return false;
        }
        self.accounts
            .insert(DEFAULT_ACCOUNT.to_string(), Account::default_machine());
        true
    }

    /// Add (or replace) an account. `config_dir = None` means the machine default.
    pub fn add(&mut self, name: &str, config_dir: Option<String>) {
        self.accounts
            .insert(name.to_string(), Account { config_dir });
    }

    /// Look up an account by name.
    pub fn get(&self, name: &str) -> Option<&Account> {
        self.accounts.get(name)
    }

    /// All registered account names, sorted.
    pub fn names(&self) -> Vec<String> {
        self.accounts.keys().cloned().collect()
    }
}

/// The `CLAUDE_CONFIG_DIR` value for an account, if any (None = env unset).
pub fn config_dir_of(acct: &Account) -> Option<PathBuf> {
    acct.config_dir.as_ref().map(PathBuf::from)
}
