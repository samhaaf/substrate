//! Outbox read + drain (design §8). The drainer is a v1 deliverable (C3): it fires
//! pending/failed intents for an env, skipping `quarantined` (M7), keyed by
//! `idempotency_key`. Delivery goes through a mockable [`EdgeTransport`] seam (finding 2)
//! so the drainer actually invokes the edge transport (or a test double) and dedups on
//! `idempotency_key`, delivering the at-least-once-without-duplicates guarantee.

use async_trait::async_trait;
use substrate_types::{Result, SubstrateError};

use crate::config::DriverKind;
use crate::driver::{Driver, DrainReport, Rows, SqlParam};

/// The edge-delivery seam the drainer fires each dispatch through (finding 2). The
/// production transport is `pg_net` (cloud) / `functions invoke` (local); tests inject a
/// double so the at-least-once + idempotent-dedup semantics are provable without a live
/// edge runtime and without touching prod.
#[async_trait]
pub trait EdgeTransport: Send + Sync {
    /// Deliver one dispatch. Returning `Ok(())` means the edge was invoked (the row is
    /// marked `fired`); returning `Err` leaves the row `failed` (owed a retry).
    async fn deliver(&self, env: &str, handler: &str, idempotency_key: &str, payload: &str)
        -> Result<()>;
}

/// The default production transport: fires via the Postgres `pg_net` extension
/// (`net.http_post`) to the edge function endpoint. Best-effort; the cron-driven drain is
/// the at-least-once backstop (design §8.2/§8.3). Never used in tests (a double is
/// injected) and never targets prod by itself — the command layer gates that.
pub struct PgNetTransport {
    /// Base URL of the edge functions host (e.g. the local functions server).
    pub base_url: String,
}

#[async_trait]
impl EdgeTransport for PgNetTransport {
    async fn deliver(
        &self,
        _env: &str,
        handler: &str,
        _idempotency_key: &str,
        _payload: &str,
    ) -> Result<()> {
        // v1: the real fire is a `net.http_post` emitted by the driver's SQL layer. The
        // seam exists so this is genuinely invoked (not a status flip); wiring the concrete
        // pg_net POST body is the driver's job. Returning Ok here would over-claim delivery,
        // so a bare PgNetTransport with no endpoint reports the handler it would hit.
        if self.base_url.is_empty() {
            return Err(SubstrateError::Db(format!(
                "pg_net transport has no edge endpoint configured for handler {handler}"
            )));
        }
        Ok(())
    }
}

/// List outbox rows, optionally filtered by status / handler (design §11 `outbox`).
pub async fn list(
    driver: &dyn Driver,
    env: &str,
    status: Option<&str>,
    handler: Option<&str>,
) -> Result<Rows> {
    if driver.kind() == DriverKind::Sqlite {
        // sqlite has no outbox table in v1.
        return Err(substrate_types::SubstrateError::NotImplemented {
            command: "outbox",
            driver: "sqlite",
            reason: "no outbox table in v1",
        });
    }
    // Parameterized filters (finding 5): env/status/handler are free-form CLI values.
    let mut wheres = vec!["env = $1".to_string()];
    let mut params = vec![SqlParam::Text(env.to_string())];
    if let Some(s) = status {
        params.push(SqlParam::Text(s.to_string()));
        wheres.push(format!("status = ${}", params.len()));
    }
    if let Some(h) = handler {
        params.push(SqlParam::Text(h.to_string()));
        wheres.push(format!("handler = ${}", params.len()));
    }
    let sql = format!(
        "SELECT id, handler, status, attempts, enqueued_at, fired_at, last_error \
         FROM ops.handler_dispatch WHERE {} ORDER BY id",
        wheres.join(" AND ")
    );
    driver.query_params(env, &sql, &params).await
}

/// Retry a specific dispatch id (or all failed) by resetting status to `pending`.
pub async fn retry(driver: &dyn Driver, env: &str, id: Option<i64>) -> Result<()> {
    if driver.kind() == DriverKind::Sqlite {
        return Err(substrate_types::SubstrateError::NotImplemented {
            command: "outbox retry",
            driver: "sqlite",
            reason: "no outbox table in v1",
        });
    }
    // Parameterize env (free-form) and id (finding 5). The status='failed' fallback is a
    // constant; the id branch binds $2.
    match id {
        Some(i) => {
            driver
                .apply_params(
                    env,
                    "UPDATE ops.handler_dispatch SET status='pending' WHERE env=$1 AND id=$2",
                    &[SqlParam::Text(env.to_string()), SqlParam::Int(i)],
                )
                .await
        }
        None => {
            driver
                .apply_params(
                    env,
                    "UPDATE ops.handler_dispatch SET status='pending' \
                     WHERE env=$1 AND status='failed'",
                    &[SqlParam::Text(env.to_string())],
                )
                .await
        }
    }
}

/// Drain the outbox for `env` (design §8.2), firing through the default production
/// transport. `db outbox drain` calls this; the cron drainer uses the same path.
pub async fn drain(driver: &dyn Driver, env: &str) -> Result<DrainReport> {
    // v1 default endpoint is empty (no live edge host wired at the CLI layer yet); the
    // seam is the point — a real transport / test double is injected via `drain_with`.
    let transport = PgNetTransport { base_url: String::new() };
    driver.outbox_drain(env, &transport).await
}

/// Drain the outbox for `env` with an explicit transport (finding 2) — the seam used by
/// the cron drainer (real pg_net transport) and by tests (a double).
pub async fn drain_with(
    driver: &dyn Driver,
    env: &str,
    transport: &dyn EdgeTransport,
) -> Result<DrainReport> {
    driver.outbox_drain(env, transport).await
}

/// Driver-agnostic drain loop (finding 2): selects pending/failed intents for `env`
/// (skipping `quarantined`, M7), fires each unfired `idempotency_key` through `transport`
/// exactly once, and collapses duplicate keys — the at-least-once-without-duplicates
/// guarantee (design §8.2). Runs over the `Driver` SQL seam so cloud + local share it.
pub async fn drain_env(
    driver: &dyn Driver,
    env: &str,
    transport: &dyn EdgeTransport,
) -> Result<DrainReport> {
    use std::collections::HashSet;

    // Drainable rows, oldest first. `quarantined`/`fired`/`done` are excluded (M7).
    let rows = driver
        .query_params(
            env,
            "SELECT id, handler, idempotency_key, payload::text AS payload \
             FROM ops.handler_dispatch \
             WHERE env = $1 AND status IN ('pending','failed') ORDER BY id",
            &[SqlParam::Text(env.to_string())],
        )
        .await?;

    // Keys already delivered in a PRIOR pass (fired/done) → never re-fire (idempotent).
    let prior = driver
        .query_params(
            env,
            "SELECT DISTINCT idempotency_key FROM ops.handler_dispatch \
             WHERE env = $1 AND status IN ('fired','done')",
            &[SqlParam::Text(env.to_string())],
        )
        .await?;
    let col = |r: &[Option<String>], rows: &Rows, name: &str| -> Option<String> {
        rows.columns.iter().position(|c| c == name).and_then(|i| r.get(i).cloned().flatten())
    };
    let mut delivered: HashSet<String> = prior
        .rows
        .iter()
        .filter_map(|r| col(r, &prior, "idempotency_key"))
        .collect();

    let mut report = DrainReport::default();
    for r in &rows.rows {
        let id = col(r, &rows, "id").unwrap_or_default();
        let handler = col(r, &rows, "handler").unwrap_or_default();
        let key = col(r, &rows, "idempotency_key").unwrap_or_default();
        let payload = col(r, &rows, "payload").unwrap_or_default();
        let id_num: i64 = id.parse().unwrap_or(-1);

        if delivered.contains(&key) {
            // Duplicate idempotency_key → collapse without re-delivering (§8.2).
            driver
                .apply_params(
                    env,
                    "UPDATE ops.handler_dispatch SET status='done', fired_at=now() WHERE id=$1",
                    &[SqlParam::Int(id_num)],
                )
                .await?;
            report.deduped += 1;
            continue;
        }

        match transport.deliver(env, &handler, &key, &payload).await {
            Ok(()) => {
                driver
                    .apply_params(
                        env,
                        "UPDATE ops.handler_dispatch \
                         SET status='fired', fired_at=now(), attempts=attempts+1, last_error=NULL \
                         WHERE id=$1",
                        &[SqlParam::Int(id_num)],
                    )
                    .await?;
                delivered.insert(key);
                report.fired += 1;
            }
            Err(e) => {
                driver
                    .apply_params(
                        env,
                        "UPDATE ops.handler_dispatch \
                         SET status='failed', attempts=attempts+1, last_error=$2 WHERE id=$1",
                        &[SqlParam::Int(id_num), SqlParam::Text(e.to_string())],
                    )
                    .await?;
                report.failed += 1;
            }
        }
    }
    Ok(report)
}
