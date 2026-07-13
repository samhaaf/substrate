//! PART V — [LOCAL] Vault integration tests (Tier 2, Docker-gated).
//!
//! Exercises `db vault` (set/list/get/rm) against a REAL Supabase Vault on the local
//! Postgres, using a THROWAWAY per-test database (`PgSandbox`) into which we install the
//! `supabase_vault` extension. Nothing here touches remote/prod. When Docker/the stack is
//! absent — or the Vault extension is unavailable on the cluster — each test skips green
//! (S.4) rather than failing red.

mod common;

use common::{local_enabled, PgSandbox};
use substrate_db::driver::Driver;
use substrate_db::vault;

/// Skip (green) when the LOCAL tier can't run, else create a sandbox.
macro_rules! sandbox_or_skip {
    ($name:expr) => {{
        if !local_enabled() {
            eprintln!(
                "SKIP {}: local Supabase Postgres not reachable on 127.0.0.1:54322 \
                 (set DB_IT_LOCAL=1 and bring up the stack). S.4 green-skip.",
                $name
            );
            return;
        }
        PgSandbox::create().await
    }};
}

/// Install the `supabase_vault` extension into the sandbox DB. Returns false (with a
/// green-skip reason) when the extension is not available on the cluster — Vault is a
/// Supabase add-on and a bare Postgres won't have it.
async fn vault_ready(sb: &PgSandbox, name: &str) -> bool {
    match sb
        .driver()
        .apply_sql("", "CREATE EXTENSION IF NOT EXISTS supabase_vault CASCADE;")
        .await
    {
        Ok(()) => true,
        Err(e) => {
            eprintln!("SKIP {name}: supabase_vault extension unavailable ({e}). S.4 green-skip.");
            false
        }
    }
}

async fn count(sb: &PgSandbox, sql: &str) -> i64 {
    let rows = sb.driver().query("", sql).await.unwrap();
    rows.rows
        .first()
        .and_then(|r| r.first().cloned().flatten())
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1)
}

// I-VAULT-01: full round-trip — set → list (name+desc, NOT the plaintext) → get (decrypted)
// → set again updates in place (by name) → rm removes it.
#[tokio::test]
async fn i_vault_01_set_list_get_update_rm_roundtrip() {
    let sb = sandbox_or_skip!("I-VAULT-01");
    if !vault_ready(&sb, "I-VAULT-01").await {
        sb.teardown().await;
        return;
    }
    let d = sb.driver();

    // set (create) — value is bound, never interpolated.
    let outcome = vault::set(&d, "", "api_key", "s3cr3t-value", Some("external api key"))
        .await
        .unwrap();
    assert_eq!(outcome, vault::SetOutcome::Created, "first set creates");
    assert!(vault::exists(&d, "", "api_key").await.unwrap());

    // list surfaces name + description but NEVER the plaintext.
    let listed = vault::list(&d, "").await.unwrap();
    let name_col = listed.columns.iter().position(|c| c == "name").unwrap();
    let names: Vec<String> = listed
        .rows
        .iter()
        .filter_map(|r| r.get(name_col).cloned().flatten())
        .collect();
    assert!(names.contains(&"api_key".to_string()), "listed names include api_key");
    let flat = format!("{:?}", listed.rows);
    assert!(!flat.contains("s3cr3t-value"), "list must NEVER expose the decrypted value");

    // get decrypts by name.
    assert_eq!(
        vault::get(&d, "", "api_key").await.unwrap().as_deref(),
        Some("s3cr3t-value"),
        "get returns the decrypted plaintext"
    );

    // set again UPDATES in place (same name → update_secret path), not a duplicate row.
    let outcome2 = vault::set(&d, "", "api_key", "rotated-value", Some("rotated"))
        .await
        .unwrap();
    assert_eq!(outcome2, vault::SetOutcome::Updated, "second set updates by name");
    assert_eq!(count(&sb, "SELECT count(*) FROM vault.secrets WHERE name='api_key'").await, 1,
        "update in place — exactly one row for the name");
    assert_eq!(
        vault::get(&d, "", "api_key").await.unwrap().as_deref(),
        Some("rotated-value"),
        "get reflects the rotated value"
    );

    // rm removes it; a second rm reports it never existed.
    assert!(vault::remove(&d, "", "api_key").await.unwrap(), "rm reports the secret existed");
    assert_eq!(count(&sb, "SELECT count(*) FROM vault.secrets WHERE name='api_key'").await, 0);
    assert_eq!(vault::get(&d, "", "api_key").await.unwrap(), None, "get is None after rm");
    assert!(!vault::remove(&d, "", "api_key").await.unwrap(), "second rm reports nothing removed");

    sb.teardown().await;
}

// I-VAULT-02: a value containing a single quote round-trips intact — proof that the value
// is BOUND, not string-interpolated (an interpolated `'` would break or inject SQL).
#[tokio::test]
async fn i_vault_02_quote_in_value_is_bound_not_interpolated() {
    let sb = sandbox_or_skip!("I-VAULT-02");
    if !vault_ready(&sb, "I-VAULT-02").await {
        sb.teardown().await;
        return;
    }
    let d = sb.driver();
    let tricky = "O'Brien'; DROP TABLE vault.secrets; --";
    vault::set(&d, "", "quoted", tricky, None).await.unwrap();
    assert_eq!(
        vault::get(&d, "", "quoted").await.unwrap().as_deref(),
        Some(tricky),
        "a quote-laden value round-trips verbatim (bound param, not interpolated)"
    );
    // The table still exists (the injection attempt did nothing).
    assert_eq!(count(&sb, "SELECT count(*) FROM vault.secrets WHERE name='quoted'").await, 1);
    sb.teardown().await;
}
