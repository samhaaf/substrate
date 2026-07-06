//! `SupabaseCloudDriver` — the prod / promote target (design §10.2).
//!
//! `apply_sql`/`query` go through the Supabase **Management API** query endpoint with
//! a keychain PAT; `deploy_edge`/`edge_logs` via the Management API; `outbox_drain`
//! via the cloud transport (a `pg_cron` job runs `db outbox drain`, with `pg_net`
//! best-effort post-commit fire). `local_lifecycle`/`pull` stay NotImplemented — the
//! cloud has no local stack and is a pull *source*, not a target.
//!
//! Caps: `{edge, rls, promote_target, outbox, registry}` = true;
//! `{local_stack, pull}` = false.
//!
//! NOTE: nothing here is applied to remote/prod Supabase by the build; the request
//! wiring is present but every mutating call still goes through the safety gate at the
//! command layer (protected-ref refusal + typed confirm).

use async_trait::async_trait;
use serde_json::json;
use substrate_types::{Result, SubstrateError};

use super::{
    cell, substitute_params, AppliedMigration, Bundle, Capabilities, Deploy, Driver, DrainReport,
    Introspection, IntrospectQuery, LockGuard, Logs, Rows, Since, SqlParam,
};
use crate::config::DriverKind;

const MGMT_API_BASE: &str = "https://api.supabase.com";

/// Supabase-cloud driver.
pub struct SupabaseCloudDriver {
    project_ref: String,
    http: reqwest::Client,
    /// Management API PAT. In production this is fetched from the OS keychain
    /// (go-keyring parity); here it is injected at construction.
    pat: Option<String>,
}

impl SupabaseCloudDriver {
    pub fn new(project_ref: impl Into<String>, pat: Option<String>) -> Self {
        Self {
            project_ref: project_ref.into(),
            http: reqwest::Client::new(),
            pat,
        }
    }

    fn pat(&self) -> Result<&str> {
        self.pat
            .as_deref()
            .ok_or_else(|| SubstrateError::Db("no Management API PAT (expected from keychain)".into()))
    }

    /// Run SQL via the Management API query endpoint. Returns the raw JSON result.
    async fn run_query(&self, sql: &str) -> Result<serde_json::Value> {
        let pat = self.pat()?;
        let url = format!(
            "{MGMT_API_BASE}/v1/projects/{}/database/query",
            self.project_ref
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(pat)
            .json(&json!({ "query": sql }))
            .send()
            .await
            .map_err(|e| SubstrateError::Db(format!("mgmt-api query request: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SubstrateError::Db(format!(
                "mgmt-api query failed ({status}): {body}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| SubstrateError::Db(format!("mgmt-api query decode: {e}")))
    }

    fn rows_from_json(v: serde_json::Value) -> Rows {
        let mut out = Rows::default();
        if let serde_json::Value::Array(arr) = v {
            for (i, item) in arr.iter().enumerate() {
                if let serde_json::Value::Object(map) = item {
                    if i == 0 {
                        out.columns = map.keys().cloned().collect();
                    }
                    out.rows
                        .push(out.columns.iter().map(|c| cell(&map[c])).collect());
                }
            }
        }
        out
    }
}

#[async_trait]
impl Driver for SupabaseCloudDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::SupabaseCloud
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edge: true,
            rls: true,
            local_stack: false,
            pull: false, // source-only
            promote_target: true,
            outbox: true,
            registry: true,
        }
    }

    async fn apply_sql(&self, _env: &str, sql: &str) -> Result<()> {
        self.run_query(sql).await?;
        Ok(())
    }

    async fn apply_params(&self, _env: &str, sql: &str, params: &[SqlParam]) -> Result<()> {
        // The Management API query endpoint accepts a SQL string, not a bind array, so we
        // safely single-quote-escape each param into a literal (finding 5 — never raw
        // interpolation of caller-supplied text) before sending.
        self.run_query(&substitute_params(sql, params)).await?;
        Ok(())
    }

    async fn query(&self, _env: &str, sql: &str) -> Result<Rows> {
        let v = self.run_query(sql).await?;
        Ok(Self::rows_from_json(v))
    }

    async fn introspect(&self, env: &str, q: IntrospectQuery) -> Result<Introspection> {
        // Reuse the same catalog queries as the local driver's pg introspection.
        let sql = crate::introspect::introspect_sql(&q);
        let rows = self.query(env, &sql).await?;
        Ok(Introspection { rows })
    }

    async fn ledger_record(&self, row: AppliedMigration) -> Result<()> {
        let sql = format!(
            "INSERT INTO ops.applied_migrations (env, schema, id, checksum) \
             VALUES ('{}', '{}', '{}', '{}') \
             ON CONFLICT (env, id) DO UPDATE SET checksum = EXCLUDED.checksum, applied_at = now()",
            row.env, row.schema, row.id, row.checksum
        );
        self.apply_sql(&row.env, &sql).await
    }

    async fn advisory_lock(&self, key: &str) -> Result<LockGuard> {
        // Lock acquired via HTTP Management API; session-scoped on the Supabase side.
        // No explicit unlock hook (the Supabase HTTP query endpoint has no persistent
        // session to release against; the lock expires with the backend session).
        self.apply_sql("", &format!("SELECT pg_advisory_lock(hashtext('{key}'))"))
            .await?;
        Ok(LockGuard::noop(key))
    }

    async fn apply_atomic(
        &self,
        env: &str,
        lock_key: &str,
        statements: &[String],
        ledger: AppliedMigration,
    ) -> Result<()> {
        // The Management API query endpoint runs a SQL string in one round-trip; we make
        // the whole apply unit atomic (findings 4 + 6) by wrapping it in an explicit
        // transaction with a txn-scoped advisory lock (§6.5). Any failure aborts the
        // transaction server-side — the up-DDL reverts and no ledger row is written.
        // On prod (env==''), `:SCHEMA` binds to the literal schema names.
        let mut sql = String::new();
        sql.push_str("BEGIN;\n");
        sql.push_str(&format!(
            "SELECT pg_advisory_xact_lock(hashtext('{}'));\n",
            lock_key.replace('\'', "''")
        ));
        for s in statements {
            let rewritten = if env.is_empty() {
                s.replace(":SCHEMA.", "").replace(":SCHEMA", "")
            } else {
                s.replace(":SCHEMA.", &format!("{env}.")).replace(":SCHEMA", env)
            };
            sql.push_str(&rewritten);
            sql.push_str(";\n");
        }
        sql.push_str(&format!(
            "INSERT INTO ops.applied_migrations (env, schema, id, checksum) \
             VALUES ('{}', '{}', '{}', '{}') \
             ON CONFLICT (env, id) DO UPDATE SET checksum = EXCLUDED.checksum, applied_at = now();\n",
            ledger.env.replace('\'', "''"),
            ledger.schema.replace('\'', "''"),
            ledger.id.replace('\'', "''"),
            ledger.checksum.replace('\'', "''"),
        ));
        sql.push_str("COMMIT;\n");
        self.run_query(&sql).await?;
        Ok(())
    }

    async fn deploy_edge(
        &self,
        _env: &str,
        name: &str,
        v: &str,
        bundle: &Bundle,
    ) -> Result<Deploy> {
        let pat = self.pat()?;
        let slug = format!("{name}_{v}");
        let url = format!(
            "{MGMT_API_BASE}/v1/projects/{}/functions/{slug}",
            self.project_ref
        );
        // Management API function deploy (body carries the built bundle). v1 sends the
        // stored content-addressed bundle bytes; a rebuild is never on this path (M3).
        let resp = self
            .http
            .put(&url)
            .bearer_auth(pat)
            .header("Content-Type", "application/octet-stream")
            .body(bundle.bytes.clone())
            .send()
            .await
            .map_err(|e| SubstrateError::Db(format!("mgmt-api deploy: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(SubstrateError::Db(format!(
                "mgmt-api deploy {slug} failed ({status}): {body}"
            )));
        }
        Ok(Deploy {
            slug,
            source_hash: bundle.source_hash.clone(),
        })
    }

    async fn activate_handler(&self, env: &str, handler: &str, v: &str) -> Result<()> {
        let sql = format!(
            "UPDATE ops.handler_active ha SET version_id = hv.id \
             FROM ops.handler_versions hv \
             WHERE hv.handler = '{handler}' AND hv.version = '{v}' \
               AND ha.handler = '{handler}' AND ha.env = '{env}'"
        );
        self.apply_sql(env, &sql).await
    }

    async fn rollback_handler(&self, env: &str, handler: &str) -> Result<()> {
        // Pointer-flip rollback to the prior version (finding 8) — the same one-row
        // UPDATE the local driver runs, bound over the Management-API query endpoint.
        self.apply_params(
            env,
            super::ROLLBACK_HANDLER_SQL,
            &super::rollback_handler_params(handler, env),
        )
        .await
    }

    async fn edge_logs(&self, _name: &str, _since: Since) -> Result<Logs> {
        // Rich edge log tailing is deferred (§13). v1 returns empty; wiring reserved.
        Ok(Logs::default())
    }

    async fn outbox_drain(
        &self,
        env: &str,
        transport: &dyn crate::outbox::EdgeTransport,
    ) -> Result<DrainReport> {
        // Cloud: the same driver-agnostic drain runs over the Management API SQL seam,
        // firing each intent through the transport and deduping on idempotency_key (finding
        // 2). The pg_cron job (shipped in the ops baseline) invokes this on a schedule.
        crate::outbox::drain_env(self, env, transport).await
    }

    // local_lifecycle / pull: NotImplemented (cloud has no local stack; pull source).
}
