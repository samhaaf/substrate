//! `db.toml` — the single project-root config mapping env → driver → connection.
//!
//! Deserialized with `serde` + `toml`; every field carries `#[serde(default)]` so a
//! partial file (or none) still yields a usable config. Secrets never live here —
//! PATs / service-role keys come from the keychain at runtime. Shipped as
//! `etc/db.example.toml`. See design §2.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use substrate_types::{Result, SubstrateError};

/// Root `db.toml` document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbConfig {
    /// Env selected when nothing else overrides (`--env` > `DB_ENV` > `.db.env` > this).
    #[serde(default = "default_env")]
    pub default_env: String,

    #[serde(default)]
    pub dirs: Dirs,

    /// Inlined schema-map (FINAL-design schema-map.yaml). schema name → flags.
    #[serde(default)]
    pub schema_map: BTreeMap<String, SchemaFlags>,

    #[serde(default)]
    pub safety: Safety,

    /// env name → env definition (driver + connection details).
    #[serde(default)]
    pub env: BTreeMap<String, EnvConfig>,
}

fn default_env() -> String {
    "local".to_string()
}

impl Default for DbConfig {
    fn default() -> Self {
        Self {
            default_env: default_env(),
            dirs: Dirs::default(),
            schema_map: BTreeMap::new(),
            safety: Safety::default(),
            env: BTreeMap::new(),
        }
    }
}

/// On-disk directory layout (design §5.1). All relative to the project root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dirs {
    #[serde(default = "d_migrations")]
    pub migrations: String,
    #[serde(default = "d_seeds")]
    pub seeds: String,
    #[serde(default = "d_handlers")]
    pub handlers: String,
    #[serde(default = "d_edge")]
    pub edge: String,
    #[serde(default = "d_bundles")]
    pub bundles: String,
    #[serde(default = "d_generated_sql")]
    pub generated_sql: String,
    #[serde(default = "d_generated_ts")]
    pub generated_ts: String,
}

fn d_migrations() -> String { "db/migrations".into() }
fn d_seeds() -> String { "db/seeds".into() }
fn d_handlers() -> String { "db/handlers".into() }
fn d_edge() -> String { "supabase/functions".into() }
fn d_bundles() -> String { "db/.bundles".into() }
fn d_generated_sql() -> String { "db/generated".into() }
fn d_generated_ts() -> String { "supabase/functions/_generated".into() }

impl Default for Dirs {
    fn default() -> Self {
        Self {
            migrations: d_migrations(),
            seeds: d_seeds(),
            handlers: d_handlers(),
            edge: d_edge(),
            bundles: d_bundles(),
            generated_sql: d_generated_sql(),
            generated_ts: d_generated_ts(),
        }
    }
}

/// Per-schema flags from the inlined schema-map.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SchemaFlags {
    #[serde(default)]
    pub rls: bool,
}

/// Safety config — the prod identity allow-list (M5) and the typed-confirm phrase.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Safety {
    /// project_refs that are PROD. Destructive/refuse decisions key off THIS set
    /// (M5) — never the driver kind and never a mutable `prod=true` flag.
    #[serde(default)]
    pub protected_refs: Vec<String>,
    /// Exact phrase the operator must type to proceed with a prod-targeting op.
    #[serde(default = "default_confirm_phrase")]
    pub confirm_phrase: String,
}

fn default_confirm_phrase() -> String {
    "promote prod".to_string()
}

impl Default for Safety {
    fn default() -> Self {
        Self {
            protected_refs: Vec::new(),
            confirm_phrase: default_confirm_phrase(),
        }
    }
}

/// The three driver kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DriverKind {
    SupabaseCloud,
    SupabaseLocal,
    Sqlite,
}

impl DriverKind {
    /// Stable, human-facing name used in errors / `NotImplemented.driver`.
    pub fn as_str(&self) -> &'static str {
        match self {
            DriverKind::SupabaseCloud => "supabase-cloud",
            DriverKind::SupabaseLocal => "supabase-local",
            DriverKind::Sqlite => "sqlite",
        }
    }
}

impl std::fmt::Display for DriverKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single `[env.<name>]` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvConfig {
    pub driver: DriverKind,

    // supabase-local
    #[serde(default)]
    pub project_dir: Option<String>,

    // sqlite
    #[serde(default)]
    pub path: Option<String>,

    // supabase-cloud
    #[serde(default)]
    pub project_ref: Option<String>,
    #[serde(default)]
    pub mgmt_api: bool,
}

impl DbConfig {
    /// Load `db.toml` from `path`. A missing file is an error; use [`DbConfig::default`]
    /// for the no-file case (e.g. `db init`).
    pub fn from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| {
            SubstrateError::Config(format!("reading {}: {e}", path.display()))
        })?;
        Self::from_str(&text)
    }

    /// Parse `db.toml` from a string.
    pub fn from_str(text: &str) -> Result<Self> {
        toml::from_str(text).map_err(|e| SubstrateError::Config(format!("parsing db.toml: {e}")))
    }

    /// Serialize back to TOML (used by `db init` to scaffold `db.toml`).
    pub fn to_toml(&self) -> Result<String> {
        toml::to_string_pretty(self)
            .map_err(|e| SubstrateError::Config(format!("serializing db.toml: {e}")))
    }

    /// Resolve the active env name, applying the precedence
    /// `--env` > `DB_ENV` > per-worktree `.db.env` > `default_env` (design §2).
    ///
    /// `cli_env` is the `--env` flag (if any). The `.db.env` file is read from the
    /// current directory if present.
    pub fn resolve_env(&self, cli_env: Option<&str>) -> String {
        if let Some(e) = cli_env {
            return e.to_string();
        }
        if let Ok(e) = std::env::var("DB_ENV") {
            if !e.is_empty() {
                return e;
            }
        }
        if let Ok(text) = std::fs::read_to_string(".db.env") {
            for line in text.lines() {
                let line = line.trim();
                if let Some(rest) = line.strip_prefix("DB_ENV=") {
                    let v = rest.trim().trim_matches('"');
                    if !v.is_empty() {
                        return v.to_string();
                    }
                }
            }
        }
        self.default_env.clone()
    }

    /// Look up an env definition by name.
    pub fn env(&self, name: &str) -> Result<&EnvConfig> {
        self.env
            .get(name)
            .ok_or_else(|| SubstrateError::Config(format!("no such env in db.toml: {name}")))
    }

    /// Whether the given project_ref is a protected (prod) identity (M5).
    pub fn is_protected_ref(&self, project_ref: &str) -> bool {
        self.safety.protected_refs.iter().any(|r| r == project_ref)
    }

    /// Whether an env resolves to a protected (prod) identity.
    pub fn env_is_protected(&self, name: &str) -> bool {
        match self.env.get(name) {
            Some(e) => e
                .project_ref
                .as_deref()
                .map(|r| self.is_protected_ref(r))
                .unwrap_or(false),
            None => false,
        }
    }

    /// The `ops.env` token for a named env: `''` for the default/prod-bare env, else
    /// the env name itself (design §2 env-token semantics).
    pub fn env_token(&self, name: &str) -> String {
        // A protected/prod cloud env maps to the bare '' schemas; a stage clone
        // (`env_<slug>`) keeps its name. We treat the default_env-as-prod convention
        // by returning '' for protected refs; callers can override for stages.
        if self.env_is_protected(name) {
            String::new()
        } else {
            name.to_string()
        }
    }
}

/// The example `db.toml` scaffolded by `db init` and shipped as `etc/db.example.toml`.
pub const EXAMPLE_TOML: &str = include_str!("../../../etc/db.example.toml");
