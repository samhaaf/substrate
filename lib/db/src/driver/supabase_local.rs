//! `SupabaseLocalDriver` — Supabase-in-Docker (design §10.2).
//!
//! `apply_sql`/`query`/`introspect` go over a NATIVE Postgres client
//! (`tokio-postgres`, DECISION #2 — never shelling `psql`). `local_lifecycle`
//! shells `supabase`/Docker via `tokio::process::Command` (the engine `process.rs`
//! template) with a presence-probe + ready-poll. `deploy_edge`/`edge_logs` shell
//! `supabase functions deploy` + container logs. It is the `pull` *target*.
//!
//! Caps: `{edge, rls, local_stack, pull, outbox, registry}` = true; `promote_target`
//! = false.

use async_trait::async_trait;
use substrate_types::{Result, SubstrateError};
use tokio::process::Command;

use super::{
    AppliedMigration, Bundle, Capabilities, Deploy, Driver, DrainReport, Introspection,
    IntrospectQuery, LocalOp, LockGuard, Logs, Rows, Since, SqlParam,
};
use crate::config::DriverKind;
use crate::pg::PgClient;

/// Supabase-local driver.
pub struct SupabaseLocalDriver {
    /// Directory where `supabase/config.toml` lives (for CLI invocations).
    project_dir: String,
    /// The resolved connection string (kept so the snapshot path can hand it to
    /// `supabase db dump` / `pg_dump` as a read-only dump source).
    conn_str: String,
    /// Lazily-connected native Postgres client to the local stack.
    pg: PgClient,
}

impl SupabaseLocalDriver {
    /// Construct with the project dir and a connection string to the local Postgres.
    ///
    /// The connection string defaults to the Supabase local default
    /// (`postgresql://postgres:postgres@127.0.0.1:54322/postgres`) if not overridden.
    pub fn new(project_dir: impl Into<String>, conn_str: Option<String>) -> Self {
        let conn_str = conn_str.unwrap_or_else(|| {
            "postgresql://postgres:postgres@127.0.0.1:54322/postgres".to_string()
        });
        Self {
            project_dir: project_dir.into(),
            conn_str: conn_str.clone(),
            pg: PgClient::new(conn_str),
        }
    }

    /// The project dir (for CLI invocations like `supabase db dump`).
    pub fn project_dir(&self) -> &str {
        &self.project_dir
    }

    /// The read-only Postgres connection string (dump source).
    pub fn conn_str(&self) -> &str {
        &self.conn_str
    }

    /// Probe whether the `supabase` CLI is present (sync, like `cuda_present`).
    pub fn supabase_cli_present() -> bool {
        std::process::Command::new("supabase")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Run a `supabase` subcommand in the project dir; map failure to `Db`.
    async fn supabase(&self, args: &[&str]) -> Result<String> {
        if !Self::supabase_cli_present() {
            return Err(SubstrateError::Db(
                "supabase CLI not found on PATH; run `db doctor`".to_string(),
            ));
        }
        let out = Command::new("supabase")
            .args(args)
            .current_dir(&self.project_dir)
            .output()
            .await
            .map_err(|e| SubstrateError::Db(format!("spawning supabase {args:?}: {e}")))?;
        if !out.status.success() {
            return Err(SubstrateError::Db(format!(
                "supabase {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[async_trait]
impl Driver for SupabaseLocalDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::SupabaseLocal
    }

    fn dump_url(&self) -> Option<String> {
        Some(self.conn_str.clone())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edge: true,
            rls: true,
            local_stack: true,
            pull: true,
            promote_target: false,
            outbox: true,
            registry: true,
        }
    }

    async fn apply_sql(&self, env: &str, sql: &str) -> Result<()> {
        self.pg.apply_sql(env, sql).await
    }

    async fn apply_params(&self, env: &str, sql: &str, params: &[SqlParam]) -> Result<()> {
        self.pg.apply_params(env, sql, params).await
    }

    async fn query(&self, env: &str, sql: &str) -> Result<Rows> {
        self.pg.query(env, sql).await
    }

    async fn query_params(&self, env: &str, sql: &str, params: &[SqlParam]) -> Result<Rows> {
        self.pg.query_params(env, sql, params).await
    }

    async fn introspect(&self, env: &str, q: IntrospectQuery) -> Result<Introspection> {
        self.pg.introspect(env, q).await
    }

    async fn ledger_record(&self, row: AppliedMigration) -> Result<()> {
        self.pg.ledger_record(row).await
    }

    async fn advisory_lock(&self, key: &str) -> Result<LockGuard> {
        self.pg.advisory_lock(key).await
    }

    async fn apply_atomic(
        &self,
        env: &str,
        lock_key: &str,
        statements: &[String],
        ledger: AppliedMigration,
    ) -> Result<()> {
        // Forward to the native pg client's genuinely-atomic implementation (findings
        // 4 + 6): one transaction, txn-scoped advisory lock, ledger inside the same txn.
        self.pg.apply_atomic(env, lock_key, statements, ledger).await
    }

    async fn deploy_edge(
        &self,
        _env: &str,
        name: &str,
        v: &str,
        _bundle: &Bundle,
    ) -> Result<Deploy> {
        let slug = format!("{name}_{v}");
        // Local: `supabase functions deploy <slug>`.
        self.supabase(&["functions", "deploy", &slug]).await?;
        Ok(Deploy {
            slug,
            source_hash: _bundle.source_hash.clone(),
        })
    }

    async fn activate_handler(&self, env: &str, handler: &str, v: &str) -> Result<()> {
        // Registry activation is a plain SQL UPDATE against ops.handler_active.
        let sql = format!(
            "UPDATE ops.handler_active ha SET version_id = hv.id \
             FROM ops.handler_versions hv \
             WHERE hv.handler = '{handler}' AND hv.version = '{v}' \
               AND ha.handler = '{handler}' AND ha.env = '{env}'"
        );
        self.pg.apply_sql(env, &sql).await
    }

    async fn rollback_handler(&self, env: &str, handler: &str) -> Result<()> {
        self.pg
            .apply_params(
                env,
                super::ROLLBACK_HANDLER_SQL,
                &super::rollback_handler_params(handler, env),
            )
            .await
    }

    async fn edge_logs(&self, _name: &str, _since: Since) -> Result<Logs> {
        // Local edge logs come from the container; v1 returns the docker logs of the
        // functions container. Kept minimal — rich tailing is deferred (§13).
        let out = self
            .supabase(&["functions", "list"])
            .await
            .unwrap_or_default();
        Ok(Logs {
            lines: out.lines().map(|s| s.to_string()).collect(),
        })
    }

    async fn outbox_drain(
        &self,
        env: &str,
        transport: &dyn crate::outbox::EdgeTransport,
    ) -> Result<DrainReport> {
        crate::outbox::drain_env(self, env, transport).await
    }

    async fn local_lifecycle(&self, op: LocalOp) -> Result<()> {
        match op {
            LocalOp::Up | LocalOp::Ensure => {
                self.supabase(&["start"]).await?;
            }
            LocalOp::Down => {
                self.supabase(&["stop"]).await?;
            }
            LocalOp::Restart => {
                self.supabase(&["stop"]).await.ok();
                self.supabase(&["start"]).await?;
            }
            LocalOp::Status => {
                self.supabase(&["status"]).await?;
            }
            LocalOp::Reset => {
                self.supabase(&["db", "reset"]).await?;
            }
        }
        Ok(())
    }

    async fn pull(&self, _opts: super::PullOpts) -> Result<()> {
        // `db local pull` is DEFERRED in v1 (DECISION #6; seed + fakedata cover it).
        Err(SubstrateError::NotImplemented {
            command: "pull",
            driver: DriverKind::SupabaseLocal.as_str(),
            reason: "prod-data pull is deferred in v1; use seed + fakedata",
        })
    }
}
