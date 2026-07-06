//! Validator audit read (design §8.4, §11 `audit`). Deny-audit is written OUT OF
//! TRANSACTION by the client layer (C2); this module is the read/observability side.

use substrate_types::Result;

use crate::config::DriverKind;
use crate::driver::{Driver, Rows, SqlParam};

/// Query validator audit rows, optionally by handler / decision (design §11).
pub async fn list(
    driver: &dyn Driver,
    env: &str,
    handler: Option<&str>,
    decision: Option<&str>,
) -> Result<Rows> {
    if driver.kind() == DriverKind::Sqlite {
        return Err(substrate_types::SubstrateError::NotImplemented {
            command: "audit",
            driver: "sqlite",
            reason: "no validator_audit table in v1 (sql validators only)",
        });
    }
    // Parameterized filters (finding 5): env/handler/decision are free-form CLI values.
    let mut wheres = vec!["env = $1".to_string()];
    let mut params = vec![SqlParam::Text(env.to_string())];
    if let Some(h) = handler {
        params.push(SqlParam::Text(h.to_string()));
        wheres.push(format!("handler = ${}", params.len()));
    }
    if let Some(d) = decision {
        params.push(SqlParam::Text(d.to_string()));
        wheres.push(format!("decision = ${}", params.len()));
    }
    let sql = format!(
        "SELECT handler, decision, reason, code, latency_ms, timed_out, at \
         FROM ops.validator_audit WHERE {} ORDER BY at DESC LIMIT 200",
        wheres.join(" AND ")
    );
    driver.query_params(env, &sql, &params).await
}

/// Write a deny audit row out-of-band (C2) — called by the client layer after it
/// catches a validator deny error, in a SEPARATE committed transaction.
pub async fn write_deny(
    driver: &dyn Driver,
    env: &str,
    handler: &str,
    version_id: i64,
    reason: &str,
    code: Option<&str>,
    latency_ms: Option<i32>,
    timed_out: bool,
) -> Result<()> {
    // Parameterized: reason/code/handler/env are free-form and MUST NOT be interpolated
    // (finding 5). A `'` in a deny reason can no longer break or inject SQL.
    let sql = "INSERT INTO ops.validator_audit \
         (env, handler, version_id, decision, reason, code, latency_ms, timed_out) \
         VALUES ($1, $2, $3, 'deny', $4, $5, $6, $7)";
    let params = [
        SqlParam::Text(env.to_string()),
        SqlParam::Text(handler.to_string()),
        SqlParam::Int(version_id),
        SqlParam::Text(reason.to_string()),
        SqlParam::TextOpt(code.map(|c| c.to_string())),
        SqlParam::IntOpt(latency_ms.map(|l| l as i64)),
        SqlParam::Bool(timed_out),
    ];
    driver.apply_params(env, sql, &params).await
}
