//! Worktree stage envs (design §11 `tree`, §3.5 reap-safety). A stage is an
//! `env_<slug>` schema clone; `tree new` writes a `.db.env` pinning `DB_ENV`.
//! `tree rm`/reaper quarantines-then-purges the stage's pending intents before drop
//! (M7).

use substrate_types::{Result, SubstrateError};

use crate::config::DriverKind;
use crate::driver::Driver;

/// Derive an `env_<slug>` token from a branch name (FINAL-design §3.1).
pub fn env_slug(branch: &str) -> String {
    let slug: String = branch
        .chars()
        .map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    format!("env_{}", slug.trim_matches('_'))
}

/// Write the per-worktree `.db.env` binding (auto-bind ON, design §2 [DEFAULT]).
pub fn write_db_env(env_token: &str) -> Result<()> {
    std::fs::write(".db.env", format!("DB_ENV={env_token}\n"))
        .map_err(|e| SubstrateError::Db(format!("writing .db.env: {e}")))
}

/// Reap-safety sequence (design §3.5, M7): quarantine → purge → drop, under the env
/// advisory lock (the caller holds the lock). Refused against protected refs at the
/// command layer.
pub async fn reap(driver: &dyn Driver, env: &str) -> Result<()> {
    if driver.kind() == DriverKind::Sqlite {
        // sqlite has no registry/outbox; only the ledger rows + the file exist.
        driver
            .apply_sql(
                env,
                &format!("DELETE FROM ops_applied_migrations WHERE env='{env}'"),
            )
            .await?;
        return Ok(());
    }
    // 1. quarantine pending/failed intents.
    driver
        .apply_sql(
            env,
            &format!(
                "UPDATE ops.handler_dispatch SET status='quarantined' \
                 WHERE env='{env}' AND status IN ('pending','failed')"
            ),
        )
        .await?;
    // 2. purge this env's control-plane rows.
    for tbl in [
        "handler_dispatch",
        "validator_audit",
        "handler_active",
        "edge_deploys",
        "applied_migrations",
    ] {
        driver
            .apply_sql(env, &format!("DELETE FROM ops.{tbl} WHERE env='{env}'"))
            .await?;
    }
    // 3. drop the stage's schemas (env_<slug>_* — the driver knows the mapping).
    driver
        .apply_sql(env, &format!("DROP SCHEMA IF EXISTS {env} CASCADE"))
        .await
        .ok();
    Ok(())
}

/// List stage envs from the ledger's distinct non-empty env tokens.
pub async fn list(driver: &dyn Driver) -> Result<Vec<String>> {
    let table = if driver.kind() == DriverKind::Sqlite {
        "ops_applied_migrations"
    } else {
        "ops.applied_migrations"
    };
    let rows = driver
        .query("", &format!("SELECT DISTINCT env FROM {table} WHERE env <> ''"))
        .await
        .unwrap_or_default();
    Ok(rows
        .rows
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .collect())
}
