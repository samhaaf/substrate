//! Budget math: backoff decision table, calibration, weekly-limit, admission,
//! ETA selection, and the pending JSON shape.

use chrono::{NaiveDate, NaiveDateTime};
use substrate_ccw::budget::{
    decide, merge_calibration, session_blocked, tokens_per_percent, weekly_blocked,
    weekly_limit_tokens, AccountSelector, AccountUsage, BudgetRules, Decision, PendingLine,
    RULES_SCHEMA_VERSION,
};
use substrate_ccw::reset::parse_reset;

fn now() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 7, 25).unwrap().and_hms_opt(15, 0, 0).unwrap()
}

#[test]
fn default_template_is_20_weekly_50_session() {
    let r = BudgetRules::default();
    assert_eq!(r.weekly_pct, 20.0);
    assert_eq!(r.session_backoff_pct, 50.0);
    assert_eq!(r.schema_version, RULES_SCHEMA_VERSION);
    assert_eq!(r.sdk, "claude-code");
    assert_eq!(r.accounts, AccountSelector::Auto);
}

#[test]
fn rules_json_roundtrip_and_schema_version_present() {
    let r = BudgetRules::default();
    let json = r.to_json();
    assert!(json.contains("\"schema_version\""));
    let back = BudgetRules::from_json(&json).unwrap();
    assert_eq!(back.weekly_pct, r.weekly_pct);
}

#[test]
fn session_backoff_decision_table() {
    assert!(!session_blocked(49.0, 50.0));
    assert!(session_blocked(50.0, 50.0)); // >= backoff blocks
    assert!(session_blocked(75.0, 50.0));
    assert!(!session_blocked(0.0, 50.0));
}

#[test]
fn tokens_per_percent_from_observation_pair() {
    // 50k tokens moved the weekly pool from 10% to 15% → 10k tokens/percent.
    assert_eq!(tokens_per_percent(50_000, 10.0, 15.0), Some(10_000.0));
    // No percentage movement → no signal.
    assert_eq!(tokens_per_percent(50_000, 10.0, 10.0), None);
    // No token movement → no signal.
    assert_eq!(tokens_per_percent(0, 10.0, 15.0), None);
}

#[test]
fn calibration_merges_as_running_mean() {
    let (m1, n1) = merge_calibration(None, 10_000.0);
    assert_eq!((m1, n1), (10_000.0, 1));
    let (m2, n2) = merge_calibration(Some((m1, n1)), 20_000.0);
    assert_eq!((m2, n2), (15_000.0, 2));
    let (m3, n3) = merge_calibration(Some((m2, n2)), 30_000.0);
    assert_eq!(n3, 3);
    assert_eq!(m3, 20_000.0);
}

#[test]
fn weekly_limit_and_blocked() {
    // 20% cap × 10k tok/% = 200k token weekly limit.
    let limit = weekly_limit_tokens(20.0, Some(10_000.0));
    assert_eq!(limit, Some(200_000.0));
    assert!(!weekly_blocked(199_999.0, limit));
    assert!(weekly_blocked(200_000.0, limit));
    // Uncalibrated → never blocks (calibrating).
    assert!(!weekly_blocked(9_999_999.0, None));
    assert_eq!(weekly_limit_tokens(20.0, None), None);
}

#[test]
fn admissible_respects_session_and_weekly() {
    let rules = BudgetRules::default();
    let ok = AccountUsage {
        account: "a".into(),
        session_pct: Some(40.0),
        weekly_consumed_tokens: 100.0,
        weekly_limit_tokens: Some(200_000.0),
        resets: vec![],
    };
    assert!(ok.admissible(&rules));

    let session_over = AccountUsage { session_pct: Some(55.0), ..ok.clone() };
    assert!(!session_over.admissible(&rules));

    let weekly_over = AccountUsage { weekly_consumed_tokens: 250_000.0, ..ok.clone() };
    assert!(!weekly_over.admissible(&rules));

    // Calibrating account: only session governs.
    let calibrating = AccountUsage { weekly_limit_tokens: None, weekly_consumed_tokens: 9e9, ..ok.clone() };
    assert!(calibrating.admissible(&rules));
    assert!(calibrating.weekly_calibrating());
}

#[test]
fn decide_proceeds_on_most_session_headroom() {
    let rules = BudgetRules::default();
    let candidates = vec![
        AccountUsage { account: "hot".into(), session_pct: Some(45.0), weekly_consumed_tokens: 0.0, weekly_limit_tokens: None, resets: vec![] },
        AccountUsage { account: "cool".into(), session_pct: Some(10.0), weekly_consumed_tokens: 0.0, weekly_limit_tokens: None, resets: vec![] },
    ];
    match decide(&rules, &candidates, now()) {
        Decision::Proceed { account } => assert_eq!(account, "cool"),
        d => panic!("expected proceed, got {d:?}"),
    }
}

#[test]
fn decide_blocks_all_and_reports_eta() {
    let rules = BudgetRules::default();
    let candidates = vec![AccountUsage {
        account: "a".into(),
        session_pct: Some(80.0),
        weekly_consumed_tokens: 0.0,
        weekly_limit_tokens: None,
        resets: vec![
            parse_reset("Jul 25 at 11pm (America/Chicago)").unwrap(),
            parse_reset("Jul 25 at 6pm (America/Chicago)").unwrap(),
        ],
    }];
    match decide(&rules, &candidates, now()) {
        Decision::Blocked { reason, eta } => {
            assert!(reason.contains("session"), "reason: {reason}");
            assert!(eta.as_deref().unwrap().contains("6pm"), "eta: {eta:?}");
        }
        d => panic!("expected blocked, got {d:?}"),
    }
}

#[test]
fn decide_empty_candidates_blocks() {
    let rules = BudgetRules::default();
    assert!(matches!(decide(&rules, &[], now()), Decision::Blocked { .. }));
}

#[test]
fn pending_line_json_shape() {
    let line = PendingLine::new("session 80% ≥ backoff 50%", Some("Jul 25 at 6pm (America/Chicago)"));
    let json = line.to_json();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["ccw"], "pending");
    assert_eq!(v["reason"], "session 80% ≥ backoff 50%");
    assert_eq!(v["eta"], "Jul 25 at 6pm (America/Chicago)");

    // Null eta when unknown.
    let line2 = PendingLine::new("no candidates", None);
    let v2: serde_json::Value = serde_json::from_str(&line2.to_json()).unwrap();
    assert!(v2["eta"].is_null());
}

#[test]
fn account_selector_named_candidates() {
    let rules = BudgetRules { accounts: AccountSelector::Named(vec!["x".into()]), ..Default::default() };
    let all = vec!["x".to_string(), "y".to_string()];
    assert_eq!(rules.candidate_accounts(&all), vec!["x".to_string()]);
    let auto = BudgetRules::default();
    assert_eq!(auto.candidate_accounts(&all), all);
}
