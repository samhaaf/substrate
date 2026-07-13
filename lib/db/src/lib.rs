//! # substrate-db
//!
//! `db` is the single operational point of contact for everything database in
//! substrate: local stack lifecycle, migration authoring/apply/rollback, seed +
//! per-migration fake-data, edge-function deploy/activate with pointer-flip rollback,
//! sync validators, schema introspection, ad-hoc query, the async outbox + validator
//! audit, and forward-only promotion to prod. It wraps the Supabase CLI, the Supabase
//! Management API, Docker, and a native Postgres client behind a `clap` noun-verb
//! surface (in `bin/db`), over three drivers (`supabase-cloud`, `supabase-local`,
//! `sqlite`). The control plane is one `ops` schema (the migration ledger + handler
//! registry + outbox + audit); the ledger is the sole source of truth.
//!
//! ## Module structure (mirrors `lib/store`)
//!
//! - [`config`]      — `db.toml` (serde + toml)
//! - [`driver`]      — the `Driver` trait + capability flags + three impls
//! - [`pg`]          — native Postgres client (tokio-postgres, DECISION #2)
//! - [`migration`]   — discovery / ordering / apply / rollback / status / crawl
//! - [`lint`]        — the lint gates (sentinels, forbidden patterns, sequence, graph)
//! - [`handler`]     — contract parsing + registry + codegen
//! - [`edge`]        — bundle build + content-addressed storage (M3)
//! - [`outbox`]      — outbox read + drain (C3)
//! - [`audit`]       — validator audit (C2)
//! - [`query`]       — ad-hoc query (read-only default, `--write` gated)
//! - [`introspect`]  — schema introspection SQL
//! - [`logs`]        — edge / db logs (deferred-heavy)
//! - [`promote`]     — the promote invariant gate + `doctor`
//! - [`snapshot`]    — full-database catastrophic-recovery snapshot (create-only)
//! - [`tree`]        — worktree stage envs + reap-safety (M7)

pub mod audit;
pub mod config;
pub mod driver;
pub mod edge;
pub mod handler;
pub mod introspect;
pub mod lint;
pub mod logs;
pub mod migration;
pub mod outbox;
pub mod pg;
pub mod promote;
pub mod query;
pub mod snapshot;
pub mod tree;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use substrate_types::{Result, SubstrateError};

use crate::config::{DbConfig, DriverKind};
use crate::driver::{Driver, LocalOp};

/// The embedded `ops` baseline for Postgres (design §3.1).
pub const OPS_BASELINE_PG: &str = include_str!("../sql/0001_baseline_ops.sql");
/// The embedded `ops` baseline for sqlite — ledger-only (design §3.4).
pub const OPS_BASELINE_SQLITE: &str = include_str!("../sql/0001_baseline_ops.sqlite.sql");

/// The central `db` handle: a loaded config + a selected driver for the active env.
///
/// Constructed once per invocation. `driver` is behind an `Arc<dyn Driver>` so command
/// handlers can hold it cheaply.
#[derive(Clone)]
pub struct Db {
    pub config: DbConfig,
    /// The active env name (post-precedence resolution).
    pub env_name: String,
    /// The `ops.env` token for the active env (`''` for prod/bare, else the name).
    pub env_token: String,
    driver: Arc<dyn Driver>,
    /// Whether the active env resolves to a protected (prod) identity (M5).
    pub protected: bool,
}

impl Db {
    /// Open a `Db` for `env_name` using `config`. Selects and constructs the driver.
    ///
    /// A missing `PAT` for a cloud env is tolerated at construction (it surfaces at the
    /// first mutating call); this keeps `db doctor` / `config show` usable offline.
    pub fn open(config: DbConfig, env_name: &str, pat: Option<String>) -> Result<Self> {
        let env_cfg = config.env(env_name)?;
        let protected = config.env_is_protected(env_name);
        let env_token = config.env_token(env_name);

        let driver: Arc<dyn Driver> = match env_cfg.driver {
            DriverKind::Sqlite => {
                let path = env_cfg
                    .path
                    .clone()
                    .unwrap_or_else(|| ".db/local.sqlite".to_string());
                Arc::new(driver::sqlite::SqliteDriver::open(path)?)
            }
            DriverKind::SupabaseLocal => {
                let dir = env_cfg.project_dir.clone().unwrap_or_else(|| ".".to_string());
                Arc::new(driver::supabase_local::SupabaseLocalDriver::new(dir, None))
            }
            DriverKind::SupabaseCloud => {
                let project_ref = env_cfg.project_ref.clone().ok_or_else(|| {
                    SubstrateError::Config(format!("env {env_name}: supabase-cloud requires project_ref"))
                })?;
                Arc::new(driver::supabase_cloud::SupabaseCloudDriver::new(project_ref, pat))
            }
        };

        Ok(Self {
            config,
            env_name: env_name.to_string(),
            env_token,
            driver,
            protected,
        })
    }

    /// The selected driver.
    pub fn driver(&self) -> &dyn Driver {
        self.driver.as_ref()
    }

    /// The driver kind.
    pub fn driver_kind(&self) -> DriverKind {
        self.driver.kind()
    }

    /// Refuse an operation against a protected (prod) ref unless explicitly confirmed
    /// (design §9.1). Returns an error when the env is protected.
    pub fn refuse_if_protected(&self, op: &'static str) -> Result<()> {
        if self.protected {
            Err(SubstrateError::Db(format!(
                "{op} is refused against protected (prod) ref for env {}",
                self.env_name
            )))
        } else {
            Ok(())
        }
    }

    /// Bootstrap the `ops` control-plane baseline (design §3.1 / §3.4). Idempotent.
    pub async fn bootstrap_ops(&self) -> Result<()> {
        let sql = match self.driver_kind() {
            DriverKind::Sqlite => OPS_BASELINE_SQLITE,
            _ => OPS_BASELINE_PG,
        };
        self.driver.apply_sql(&self.env_token, sql).await
    }

    // ── Local stack lifecycle ──────────────────────────────────────────────

    pub async fn local(&self, op: LocalOp) -> Result<()> {
        if matches!(op, LocalOp::Reset) {
            self.refuse_if_protected("local reset")?;
        }
        self.driver.local_lifecycle(op).await
    }
}

/// Scaffold a project: write `db.toml` + `db/{migrations,seeds,handlers}/` (design §11
/// `init`). Does not overwrite an existing `db.toml`.
pub fn init_project(root: &Path) -> Result<Vec<PathBuf>> {
    let mut created = Vec::new();
    let toml_path = root.join("db.toml");
    if !toml_path.exists() {
        std::fs::write(&toml_path, config::EXAMPLE_TOML)
            .map_err(|e| SubstrateError::Db(format!("writing db.toml: {e}")))?;
        created.push(toml_path);
    }
    for sub in ["migrations/ops", "migrations/public", "migrations/core", "migrations/mind", "seeds", "handlers"] {
        let dir = root.join("db").join(sub);
        std::fs::create_dir_all(&dir)
            .map_err(|e| SubstrateError::Db(format!("creating {}: {e}", dir.display())))?;
    }
    // Drop the ops baseline migration in place so it applies like any other.
    let ops_dir = root.join("db/migrations/ops/0001_baseline_ops");
    if !ops_dir.exists() {
        std::fs::create_dir_all(&ops_dir)
            .map_err(|e| SubstrateError::Db(format!("creating {}: {e}", ops_dir.display())))?;
        std::fs::write(ops_dir.join("up.sql"), OPS_BASELINE_PG)
            .map_err(|e| SubstrateError::Db(format!("writing ops baseline: {e}")))?;
        std::fs::write(
            ops_dir.join("down.sql"),
            "-- @intentionally-none: the ops control plane is not rolled back\nDROP SCHEMA IF EXISTS ops CASCADE;\n",
        )
        .map_err(|e| SubstrateError::Db(format!("writing ops down: {e}")))?;
        created.push(ops_dir);
    }
    Ok(created)
}
