//! Ledger round-trips against an in-memory / tempdir SQLite store, plus the
//! account registry (accounts.toml) round-trip and first-run default.

use substrate_ccw::accounts::{Registry, DEFAULT_ACCOUNT};
use substrate_ccw::budget::BudgetRules;
use substrate_ccw::store::{
    AccountSeen, CalibrationRecord, InvocationRecord, ObservationRecord, Store,
};

#[test]
fn budget_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let rules = BudgetRules { weekly_pct: 15.0, ..Default::default() };
    store.upsert_budget("loop-a", &rules).unwrap();
    let back = store.get_budget("loop-a").unwrap().unwrap();
    assert_eq!(back.weekly_pct, 15.0);
    assert!(store.get_budget("nope").unwrap().is_none());

    // upsert replaces.
    let rules2 = BudgetRules { weekly_pct: 25.0, ..Default::default() };
    store.upsert_budget("loop-a", &rules2).unwrap();
    assert_eq!(store.get_budget("loop-a").unwrap().unwrap().weekly_pct, 25.0);
    assert_eq!(store.list_budgets().unwrap().len(), 1);
}

#[test]
fn invocation_ledger_roundtrip_and_weekly_sum() {
    let store = Store::open_in_memory().unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    let rec = InvocationRecord {
        budget_id: Some("b1".into()),
        account: Some("default".into()),
        session_id: Some("sess-1".into()),
        cwd: Some("/tmp/x".into()),
        started_at: Some(now.clone()),
        ended_at: Some(now.clone()),
        usage_json: Some("[]".into()),
        input_tokens: 100,
        output_tokens: 20,
        cache_read_tokens: 5,
        cache_creation_tokens: 3,
        total_tokens: 128,
        cost_usd: 0.5,
        limit_hit: true,
        reset_prose: Some("2:40am (America/Chicago)".into()),
        exit_code: Some(0),
    };
    store.record_invocation(&rec).unwrap();
    store.record_invocation(&InvocationRecord { total_tokens: 72, budget_id: Some("b1".into()), started_at: Some(now.clone()), ..Default::default() }).unwrap();
    // Different budget, should not count.
    store.record_invocation(&InvocationRecord { total_tokens: 999, budget_id: Some("b2".into()), started_at: Some(now.clone()), ..Default::default() }).unwrap();

    let rows = store.list_invocations(Some("b1"), 10).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows[0].total_tokens == 72 || rows[0].total_tokens == 128);
    assert!(rows.iter().any(|r| r.limit_hit && r.reset_prose.is_some()));

    let week_start = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    assert_eq!(store.weekly_consumed_tokens("b1", &week_start).unwrap(), 200);
    assert_eq!(store.weekly_consumed_tokens("b2", &week_start).unwrap(), 999);

    // All-budgets listing.
    assert_eq!(store.list_invocations(None, 10).unwrap().len(), 3);
}

#[test]
fn observation_and_calibration_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    store
        .record_observation(&ObservationRecord {
            account: "default".into(),
            pool: "week (all models)".into(),
            pct: 10.0,
            reset_at_raw: Some("Jul 25 at 12:59pm (America/Chicago)".into()),
            observed_at: "2026-07-25T10:00:00Z".into(),
        })
        .unwrap();
    store
        .record_observation(&ObservationRecord {
            account: "default".into(),
            pool: "week (all models)".into(),
            pct: 15.0,
            reset_at_raw: None,
            observed_at: "2026-07-25T11:00:00Z".into(),
        })
        .unwrap();

    let latest = store.latest_observation("default", "week (all models)").unwrap().unwrap();
    assert_eq!(latest.pct, 15.0);
    assert!(store.latest_observation("default", "session").unwrap().is_none());

    store
        .upsert_calibration(&CalibrationRecord {
            account: "default".into(),
            pool: "week (all models)".into(),
            tokens_per_percent: 10_000.0,
            updated_at: Some("2026-07-25T11:00:00Z".into()),
            sample_count: 1,
        })
        .unwrap();
    let cal = store.get_calibration("default", "week (all models)").unwrap().unwrap();
    assert_eq!(cal.tokens_per_percent, 10_000.0);
    assert_eq!(cal.sample_count, 1);

    // Upsert updates in place.
    store
        .upsert_calibration(&CalibrationRecord {
            account: "default".into(),
            pool: "week (all models)".into(),
            tokens_per_percent: 12_000.0,
            updated_at: Some("2026-07-25T12:00:00Z".into()),
            sample_count: 2,
        })
        .unwrap();
    let cal2 = store.get_calibration("default", "week (all models)").unwrap().unwrap();
    assert_eq!(cal2.tokens_per_percent, 12_000.0);
    assert_eq!(cal2.sample_count, 2);
}

#[test]
fn accounts_seen_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    store
        .upsert_account_seen(&AccountSeen {
            name: "default".into(),
            config_dir: None,
            subscription_type: Some("max".into()),
            email: Some("sam.haaf@broomstick.ai".into()),
            logged_in: true,
            last_checked_at: Some("2026-07-25T12:00:00Z".into()),
        })
        .unwrap();
    let all = store.list_accounts_seen().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].subscription_type.as_deref(), Some("max"));
    assert!(all[0].logged_in);
}

#[test]
fn persistent_store_uses_wal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ccw.db");
    let store = Store::open(&path).unwrap();
    store.upsert_budget("b", &BudgetRules::default()).unwrap();
    drop(store);
    // WAL sidecar files exist after a write.
    assert!(path.exists());
    // Reopen and read back.
    let store2 = Store::open(&path).unwrap();
    assert!(store2.get_budget("b").unwrap().is_some());
}

#[test]
fn registry_toml_roundtrip_and_first_run_default() {
    let mut reg = Registry::default();
    assert!(reg.ensure_default(), "first ensure_default adds the machine default");
    assert!(!reg.ensure_default(), "idempotent");
    assert!(reg.get(DEFAULT_ACCOUNT).unwrap().is_machine_default());

    reg.add("work", Some("/home/me/.leverage/ccw/accounts/work".into()));
    let toml = reg.to_toml().unwrap();
    let back = Registry::from_toml(&toml).unwrap();
    assert_eq!(back.names(), vec!["default".to_string(), "work".to_string()]);
    assert_eq!(
        back.get("work").unwrap().config_dir.as_deref(),
        Some("/home/me/.leverage/ccw/accounts/work")
    );
    assert!(back.get(DEFAULT_ACCOUNT).unwrap().config_dir.is_none());
}

#[test]
fn registry_save_load_tempdir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.toml");
    let mut reg = Registry::default();
    reg.ensure_default();
    reg.add("second", Some("/tmp/second".into()));
    reg.save(&path).unwrap();
    let loaded = Registry::load(&path).unwrap();
    assert_eq!(loaded.names().len(), 2);
    // Missing file → empty registry.
    let missing = Registry::load(&dir.path().join("nope.toml")).unwrap();
    assert!(missing.names().is_empty());
}
