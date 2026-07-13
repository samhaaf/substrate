//! Supabase Vault secret management (a fundamental for edge functions / DB-side secrets).
//!
//! Vault's public surface is plain SQL against the `vault` schema installed by the
//! `supabase_vault` extension:
//!
//! * create  — `select vault.create_secret(<value>, <name>, <description>)` → uuid
//! * read     — `select decrypted_secret from vault.decrypted_secrets where name = …`
//! * metadata — `select name, description from vault.secrets` (NEVER the plaintext)
//! * update   — `select vault.update_secret(<uuid>, <value>, <name>, <description>)`
//! * delete   — `delete from vault.secrets where name = …`
//!
//! Every operation flows over the [`Driver`] SQL seam, so the identical code drives the
//! `supabase-local` and `supabase-cloud` backends; `sqlite` has no Vault and degrades to a
//! typed [`SubstrateError::NotImplemented`] (mirrors the `outbox` module).
//!
//! SAFETY: all free-form values (secret name, plaintext value, description) are passed as
//! bound parameters (`$1`, …) — never string-interpolated — so a `'` in any of them can
//! neither break nor inject SQL (crate finding 5). The COMMAND layer (`bin/db`) gates the
//! two MUTATING verbs (`set`/`rm`) through the protected-ref guard and honours `--dry-run`.
//!
//! ## Referencing a secret from a migration / handler
//!
//! A migration or handler NEVER stores plaintext. It reads the secret at runtime, by name,
//! from the decrypted view. [`reference_sql`] renders the canonical lookup expression, e.g.
//! `(SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name = 'stripe_key')`,
//! which composes into a larger statement:
//!
//! ```sql
//! -- inside a handler / migration:
//! PERFORM net.http_post(
//!   url := 'https://api.example.com/charge',
//!   headers := jsonb_build_object(
//!     'Authorization',
//!     'Bearer ' || (SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name = 'api_key')
//!   )
//! );
//! ```

use substrate_types::{Result, SubstrateError};

use crate::config::DriverKind;
use crate::driver::{Driver, Rows, SqlParam};

/// The outcome of a [`set`]: whether the secret was newly created or an existing one (by
/// name) was updated in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    Created,
    Updated,
}

impl std::fmt::Display for SetOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SetOutcome::Created => "created",
            SetOutcome::Updated => "updated",
        })
    }
}

/// Fail early with a typed `NotImplemented` on a driver that has no Vault (sqlite).
fn require_vault(driver: &dyn Driver, command: &'static str) -> Result<()> {
    if driver.kind() == DriverKind::Sqlite {
        return Err(SubstrateError::NotImplemented {
            command,
            driver: "sqlite",
            reason: "sqlite has no Supabase Vault",
        });
    }
    Ok(())
}

/// Whether a secret named `name` already exists (metadata-only; no decryption).
pub async fn exists(driver: &dyn Driver, env: &str, name: &str) -> Result<bool> {
    require_vault(driver, "vault exists")?;
    let rows = driver
        .query_params(
            env,
            "SELECT id FROM vault.secrets WHERE name = $1",
            &[SqlParam::Text(name.to_string())],
        )
        .await?;
    Ok(!rows.rows.is_empty())
}

/// Create a new secret, or update the existing one with the same `name`, in place.
///
/// The value + description are bound (never interpolated). Updating by name resolves the
/// uuid inside the same statement (`vault.update_secret((SELECT id …), …)`).
pub async fn set(
    driver: &dyn Driver,
    env: &str,
    name: &str,
    value: &str,
    description: Option<&str>,
) -> Result<SetOutcome> {
    require_vault(driver, "vault set")?;
    if exists(driver, env, name).await? {
        // update_secret(uuid, new_secret, new_name, new_description). A NULL description
        // means "leave the existing description unchanged" (Vault COALESCEs it), so pass
        // TextOpt through verbatim.
        driver
            .apply_params(
                env,
                "SELECT vault.update_secret(\
                   (SELECT id FROM vault.secrets WHERE name = $1), $2, $1, $3)",
                &[
                    SqlParam::Text(name.to_string()),
                    SqlParam::Text(value.to_string()),
                    SqlParam::TextOpt(description.map(str::to_string)),
                ],
            )
            .await?;
        Ok(SetOutcome::Updated)
    } else {
        // create_secret(new_secret, new_name, new_description). `vault.secrets.description`
        // is NOT NULL: the function's `''` default only applies when the arg is OMITTED, so
        // an explicit NULL would violate the constraint. Default None → '' ourselves.
        driver
            .apply_params(
                env,
                "SELECT vault.create_secret($1, $2, $3)",
                &[
                    SqlParam::Text(value.to_string()),
                    SqlParam::Text(name.to_string()),
                    SqlParam::Text(description.unwrap_or_default().to_string()),
                ],
            )
            .await?;
        Ok(SetOutcome::Created)
    }
}

/// List secret metadata: `name`, `description`, `updated_at`. NEVER the decrypted value —
/// `db vault get --reveal` is the only path that decrypts.
pub async fn list(driver: &dyn Driver, env: &str) -> Result<Rows> {
    require_vault(driver, "vault list")?;
    driver
        .query(
            env,
            "SELECT name, description, updated_at FROM vault.secrets ORDER BY name",
        )
        .await
}

/// Read a secret's DECRYPTED plaintext by name. `None` when no such secret exists.
pub async fn get(driver: &dyn Driver, env: &str, name: &str) -> Result<Option<String>> {
    require_vault(driver, "vault get")?;
    let rows = driver
        .query_params(
            env,
            "SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name = $1",
            &[SqlParam::Text(name.to_string())],
        )
        .await?;
    Ok(rows
        .rows
        .first()
        .and_then(|r| r.first().cloned().flatten()))
}

/// Remove a secret by name. Returns whether a row actually existed (so the caller can
/// report "removed" vs "nothing to remove").
pub async fn remove(driver: &dyn Driver, env: &str, name: &str) -> Result<bool> {
    require_vault(driver, "vault rm")?;
    let existed = exists(driver, env, name).await?;
    driver
        .apply_params(
            env,
            "DELETE FROM vault.secrets WHERE name = $1",
            &[SqlParam::Text(name.to_string())],
        )
        .await?;
    Ok(existed)
}

/// The canonical SQL expression a migration / handler embeds to REFERENCE a vault secret
/// by name at runtime (reading the decrypted value from `vault.decrypted_secrets`), so the
/// plaintext never lands in a migration file. `name` is rendered as a SQL literal with `'`
/// escaped — this is doc/codegen output, not a bound-param path.
pub fn reference_sql(name: &str) -> String {
    format!(
        "(SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name = '{}')",
        name.replace('\'', "''")
    )
}
