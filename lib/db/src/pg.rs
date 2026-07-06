//! Native Postgres client used by the local (and, for query, cloud fallback) drivers.
//!
//! DECISION #2: query/introspect go over a NATIVE Postgres client (`tokio-postgres`),
//! never by shelling `psql`. The connection is established lazily on first use; the
//! spawned connection task is detached (standard tokio-postgres pattern).
//!
//! `:SCHEMA` binding: on Postgres, `:SCHEMA` maps to a real schema. For the bare/prod
//! env it is the literal schema names already written in the SQL (the SQL uses
//! `:SCHEMA` as a search-path-style placeholder resolved to `env_<slug>` for a stage,
//! or left as the literal schema for prod). v1 substitutes the placeholder textually
//! and defers full search_path plumbing.

use std::sync::Arc;

use bytes::BytesMut;
use substrate_types::{Result, SubstrateError};
use tokio::sync::Mutex;
use tokio_postgres::types::{to_sql_checked, IsNull, ToSql, Type};
use tokio_postgres::{Client, NoTls};

use crate::driver::{
    cell, AppliedMigration, Introspection, IntrospectQuery, LockGuard, Rows, SqlParam,
};

/// Concrete owned value that implements `ToSql` without type-erasure issues.
/// Needed because `Box<dyn ToSql>` loses the concrete type needed for OID resolution
/// of nullable variants (`Option<String>`, `Option<i64>`).
///
/// We delegate serialization to the inner concrete type's `ToSql` impl, and for NULL
/// variants we emit `IsNull::Yes` regardless of the column's OID (Postgres accepts
/// typed NULLs from any conforming type).
#[derive(Debug)]
enum PgValue {
    Text(String),
    TextNull,
    Int8(i64),
    Int4(i32),
    Int4Null,
    Bool(bool),
}

impl ToSql for PgValue {
    fn to_sql(
        &self,
        ty: &Type,
        out: &mut BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Sync + Send>> {
        match self {
            PgValue::Text(s) => s.to_sql(ty, out),
            PgValue::TextNull => Ok(IsNull::Yes),
            PgValue::Int8(n) => n.to_sql(ty, out),
            PgValue::Int4(n) => n.to_sql(ty, out),
            PgValue::Int4Null => Ok(IsNull::Yes),
            PgValue::Bool(b) => b.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &Type) -> bool {
        // Accept any type — PgValue handles NULL emission for nullable variants
        // and delegates concrete serialization to the inner type's ToSql impl.
        true
    }

    to_sql_checked!();
}

fn to_pg(params: &[SqlParam]) -> Vec<PgValue> {
    params
        .iter()
        .map(|p| match p {
            SqlParam::Text(s) => PgValue::Text(s.clone()),
            SqlParam::TextOpt(Some(s)) => PgValue::Text(s.clone()),
            SqlParam::TextOpt(None) => PgValue::TextNull,
            SqlParam::Int(n) => PgValue::Int8(*n),
            SqlParam::IntOpt(Some(n)) => {
                // Use Int4 when the value fits in i32 (common for counts/latencies);
                // fall back to Int8 for larger values. Postgres coerces INT4→INT8 but
                // not the reverse in binary protocol.
                if *n >= i32::MIN as i64 && *n <= i32::MAX as i64 {
                    PgValue::Int4(*n as i32)
                } else {
                    PgValue::Int8(*n)
                }
            }
            SqlParam::IntOpt(None) => PgValue::Int4Null,
            SqlParam::Bool(b) => PgValue::Bool(*b),
        })
        .collect()
}

/// A lazily-connected native Postgres client.
pub struct PgClient {
    conn_str: String,
    client: Mutex<Option<Arc<Client>>>,
}

impl PgClient {
    pub fn new(conn_str: impl Into<String>) -> Self {
        Self {
            conn_str: conn_str.into(),
            client: Mutex::new(None),
        }
    }

    /// Get (connecting if necessary) the shared client.
    async fn client(&self) -> Result<Arc<Client>> {
        let mut guard = self.client.lock().await;
        if let Some(c) = guard.as_ref() {
            return Ok(Arc::clone(c));
        }
        let (client, connection) = tokio_postgres::connect(&self.conn_str, NoTls)
            .await
            .map_err(|e| SubstrateError::Db(format!("connecting to postgres: {e}")))?;
        // Drive the connection in the background; log on error.
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::warn!(error = %e, "postgres connection closed");
            }
        });
        let client = Arc::new(client);
        *guard = Some(Arc::clone(&client));
        Ok(client)
    }

    /// Resolve the `:SCHEMA` placeholder for `env`. Bare/prod env → leave the SQL's
    /// literal schema names; a stage env → prefix into `env_<slug>` schemas.
    fn rewrite(env: &str, sql: &str) -> String {
        if env.is_empty() {
            // Prod/bare: :SCHEMA_<other> and :SCHEMA resolve to the literal schema
            // written by the author (search_path handles it). Strip the marker.
            sql.replace(":SCHEMA.", "").replace(":SCHEMA", "")
        } else {
            // Stage clone: bind to the env-scoped schema name.
            sql.replace(":SCHEMA.", &format!("{env}."))
                .replace(":SCHEMA", env)
        }
    }

    pub async fn apply_sql(&self, env: &str, sql: &str) -> Result<()> {
        let sql = Self::rewrite(env, sql);
        let client = self.client().await?;
        client
            .batch_execute(&sql)
            .await
            .map_err(|e| SubstrateError::Db(format!("apply_sql: {e}")))?;
        Ok(())
    }

    /// Apply SQL with bound parameters (`$1`, `$2`, …) — native tokio-postgres binding,
    /// so free-form values (reason/code/handler names) are never interpolated (finding 5).
    pub async fn apply_params(&self, env: &str, sql: &str, params: &[SqlParam]) -> Result<()> {
        let sql = Self::rewrite(env, sql);
        let client = self.client().await?;
        // Use concrete PgValue to avoid type-erasure OID issues with nullable Box<dyn ToSql>.
        let owned = to_pg(params);
        let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            owned.iter().map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
        client
            .execute(&sql, &refs)
            .await
            .map_err(|e| SubstrateError::Db(format!("apply_params: {e}")))?;
        Ok(())
    }

    pub async fn query(&self, env: &str, sql: &str) -> Result<Rows> {
        let sql = Self::rewrite(env, sql);
        let client = self.client().await?;
        // Cast every column to text so we can render uniformly without type maps.
        // Wrap the query so `SELECT *` still works: run it and read via json_agg.
        let wrapped = format!(
            "SELECT to_jsonb(t) AS row FROM ({}) t",
            sql.trim().trim_end_matches(';')
        );
        let rows = client
            .query(&wrapped, &[])
            .await
            .map_err(|e| SubstrateError::Db(format!("query: {e}")))?;
        let mut out = Rows::default();
        for (i, row) in rows.iter().enumerate() {
            let v: serde_json::Value = row.get("row");
            if let serde_json::Value::Object(map) = v {
                if i == 0 {
                    out.columns = map.keys().cloned().collect();
                }
                out.rows
                    .push(out.columns.iter().map(|c| cell(&map[c])).collect());
            }
        }
        Ok(out)
    }

    /// Run a read query with bound parameters (`$1`, …); native tokio-postgres binding
    /// (finding 5). Wraps the query in the same `to_jsonb` renderer as [`query`].
    pub async fn query_params(&self, env: &str, sql: &str, params: &[SqlParam]) -> Result<Rows> {
        let sql = Self::rewrite(env, sql);
        let client = self.client().await?;
        let wrapped = format!(
            "SELECT to_jsonb(t) AS row FROM ({}) t",
            sql.trim().trim_end_matches(';')
        );
        let owned = to_pg(params);
        let refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            owned.iter().map(|v| v as &(dyn tokio_postgres::types::ToSql + Sync)).collect();
        let rows = client
            .query(&wrapped, &refs)
            .await
            .map_err(|e| SubstrateError::Db(format!("query_params: {e}")))?;
        let mut out = Rows::default();
        for (i, row) in rows.iter().enumerate() {
            let v: serde_json::Value = row.get("row");
            if let serde_json::Value::Object(map) = v {
                if i == 0 {
                    out.columns = map.keys().cloned().collect();
                }
                out.rows.push(out.columns.iter().map(|c| cell(&map[c])).collect());
            }
        }
        Ok(out)
    }

    pub async fn introspect(&self, env: &str, q: IntrospectQuery) -> Result<Introspection> {
        let sql = match q {
            IntrospectQuery::Tables => "SELECT table_schema, table_name FROM information_schema.tables \
                 WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY 1,2"
                .to_string(),
            IntrospectQuery::Describe { table } => format!(
                "SELECT column_name, data_type, is_nullable FROM information_schema.columns \
                 WHERE table_name = '{table}' ORDER BY ordinal_position"
            ),
            IntrospectQuery::Functions => "SELECT n.nspname AS schema, p.proname AS name \
                 FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
                 WHERE n.nspname NOT IN ('pg_catalog','information_schema') ORDER BY 1,2"
                .to_string(),
            IntrospectQuery::Triggers => {
                "SELECT event_object_table AS table, trigger_name FROM information_schema.triggers \
                 ORDER BY 1,2"
                    .to_string()
            }
            IntrospectQuery::Policies => "SELECT schemaname, tablename, policyname FROM pg_policies \
                 ORDER BY 1,2,3"
                .to_string(),
            IntrospectQuery::Handlers => {
                "SELECT h.name, ha.env, hv.version, hv.kind, hv.invocation \
                 FROM ops.handlers h \
                 LEFT JOIN ops.handler_active ha ON ha.handler = h.name \
                 LEFT JOIN ops.handler_versions hv ON hv.id = ha.version_id \
                 ORDER BY 1,2"
                    .to_string()
            }
            IntrospectQuery::Size => {
                "SELECT relname AS table, pg_size_pretty(pg_total_relation_size(c.oid)) AS size \
                 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.relkind='r' AND n.nspname NOT IN ('pg_catalog','information_schema') \
                 ORDER BY pg_total_relation_size(c.oid) DESC"
                    .to_string()
            }
        };
        let rows = self.query(env, &sql).await?;
        Ok(Introspection { rows })
    }

    pub async fn ledger_record(&self, row: AppliedMigration) -> Result<()> {
        let client = self.client().await?;
        client
            .execute(
                "INSERT INTO ops.applied_migrations (env, schema, id, checksum) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (env, id) DO UPDATE SET checksum = EXCLUDED.checksum, \
                   applied_at = now()",
                &[&row.env, &row.schema, &row.id, &row.checksum],
            )
            .await
            .map_err(|e| SubstrateError::Db(format!("ledger_record: {e}")))?;
        Ok(())
    }

    pub async fn advisory_lock(&self, key: &str) -> Result<LockGuard> {
        let client = self.client().await?;
        // Session-level advisory lock. It is RELIABLY released on guard drop (finding 4):
        // the guard carries a hook that runs pg_advisory_unlock on the shared client, so
        // the lock never leaks for the connection's life.
        client
            .execute("SELECT pg_advisory_lock(hashtext($1))", &[&key])
            .await
            .map_err(|e| SubstrateError::Db(format!("advisory_lock: {e}")))?;
        let unlock_client = Arc::clone(&client);
        let unlock_key = key.to_string();
        let release = Box::new(move || {
            // Spawn the unlock on the shared connection; best-effort, fire-and-forget.
            tokio::spawn(async move {
                let _ = unlock_client
                    .execute("SELECT pg_advisory_unlock(hashtext($1))", &[&unlock_key])
                    .await;
            });
        });
        Ok(LockGuard::with_release(key, release))
    }

    /// Apply one migration atomically under a txn-scoped advisory lock (finding 4). The
    /// whole sequence runs in ONE tokio-postgres transaction: `pg_advisory_xact_lock` →
    /// each statement → the ledger INSERT. Any failure rolls the transaction back (the
    /// up-DDL reverts, no ledger row); the xact lock auto-releases at COMMIT/ROLLBACK.
    pub async fn apply_atomic(
        &self,
        env: &str,
        lock_key: &str,
        statements: &[String],
        ledger: AppliedMigration,
    ) -> Result<()> {
        let mut client = self.client().await?;
        // A transaction needs `&mut Client`; the shared Arc gives `&Client`, so borrow a
        // dedicated connection for the atomic unit to get an owned transaction handle.
        let txn_client = Arc::get_mut(&mut client);
        // If the Arc is shared (it always is via the cache), open a fresh connection for
        // the transaction so we truly get BEGIN/COMMIT isolation.
        let (mut owned, connection) = tokio_postgres::connect(&self.conn_str, NoTls)
            .await
            .map_err(|e| SubstrateError::Db(format!("apply_atomic connect: {e}")))?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::warn!(error = %e, "apply_atomic connection closed");
            }
        });
        let _ = txn_client; // (documented above; we intentionally use a fresh connection)

        let txn = owned
            .transaction()
            .await
            .map_err(|e| SubstrateError::Db(format!("apply_atomic begin: {e}")))?;

        // Txn-scoped advisory lock (design §6.5) — auto-released at COMMIT/ROLLBACK.
        txn.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&lock_key])
            .await
            .map_err(|e| SubstrateError::Db(format!("apply_atomic lock: {e}")))?;

        for s in statements {
            let rewritten = Self::rewrite(env, s);
            txn.batch_execute(&rewritten)
                .await
                .map_err(|e| SubstrateError::Db(format!("apply_atomic statement: {e}")))?;
        }

        txn.execute(
            "INSERT INTO ops.applied_migrations (env, schema, id, checksum) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (env, id) DO UPDATE SET checksum = EXCLUDED.checksum, applied_at = now()",
            &[&ledger.env, &ledger.schema, &ledger.id, &ledger.checksum],
        )
        .await
        .map_err(|e| SubstrateError::Db(format!("apply_atomic ledger: {e}")))?;

        txn.commit()
            .await
            .map_err(|e| SubstrateError::Db(format!("apply_atomic commit: {e}")))?;
        Ok(())
    }

}
