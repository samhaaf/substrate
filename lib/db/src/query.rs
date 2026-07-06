//! Ad-hoc query (design §11 `query`). Read-only by default; `--write` crosses the
//! safety gate (checked at the command layer via the protected-ref rule).

use substrate_types::{Result, SubstrateError};

use crate::driver::{Driver, Rows};

/// Very rough read/write classification: anything that isn't a bare SELECT/WITH/EXPLAIN
/// /SHOW is treated as a write and must be gated by `--write`.
pub fn is_write(sql: &str) -> bool {
    let head = sql
        .trim_start()
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_uppercase();
    !matches!(head.as_str(), "SELECT" | "WITH" | "EXPLAIN" | "SHOW" | "TABLE" | "VALUES")
}

/// Run a query. If it's a write and `allow_write` is false, refuse.
pub async fn run(driver: &dyn Driver, env: &str, sql: &str, allow_write: bool) -> Result<Rows> {
    if is_write(sql) && !allow_write {
        return Err(SubstrateError::Db(
            "refusing a write query without --write".to_string(),
        ));
    }
    if is_write(sql) {
        driver.apply_sql(env, sql).await?;
        Ok(Rows::default())
    } else {
        driver.query(env, sql).await
    }
}
