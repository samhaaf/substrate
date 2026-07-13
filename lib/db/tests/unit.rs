//! PART A — UNIT TESTS ([U]). No DB/process; exercise the crate's pure logic against
//! its public API. IDs map to the test plan.

use std::collections::BTreeMap;

use substrate_db::config::{DbConfig, DriverKind};
use substrate_db::driver::supabase_cloud::SupabaseCloudDriver;
use substrate_db::driver::supabase_local::SupabaseLocalDriver;
use substrate_db::driver::sqlite::SqliteDriver;
use substrate_db::driver::Driver;
use substrate_db::handler::{codegen, Contract};
use substrate_db::lint::{self, classify_change, CompatClass, Level};
use substrate_db::migration::{self, is_sentinel_only, Migration};

mod common;
use common::{write_migration, FX_TOML_BASE};

// ── A.1 Config parse ─────────────────────────────────────────────────────────

#[test]
fn u_cfg_01_parse_base() {
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    assert_eq!(c.default_env, "local");
    assert_eq!(c.env.len(), 3);
    assert_eq!(c.env("local").unwrap().driver, DriverKind::SupabaseLocal);
    assert_eq!(c.env("sqlite").unwrap().driver, DriverKind::Sqlite);
    assert_eq!(c.env("prod").unwrap().driver, DriverKind::SupabaseCloud);
}

#[test]
fn u_cfg_02_missing_dirs_defaults() {
    // No [dirs] section at all → serde(default) fills spec defaults.
    let c = DbConfig::from_str("default_env=\"local\"\n[env.sqlite]\ndriver=\"sqlite\"\n").unwrap();
    assert_eq!(c.dirs.migrations, "db/migrations");
    assert_eq!(c.dirs.seeds, "db/seeds");
    assert_eq!(c.dirs.handlers, "db/handlers");
}

#[test]
fn u_cfg_03_env_precedence() {
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    // --env wins over everything.
    std::env::remove_var("DB_ENV");
    assert_eq!(c.resolve_env(Some("prod")), "prod");
    // DB_ENV wins when no --env.
    std::env::set_var("DB_ENV", "sqlite");
    assert_eq!(c.resolve_env(None), "sqlite");
    std::env::remove_var("DB_ENV");
    // default_env is the floor (no .db.env in the crate cwd).
    assert_eq!(c.resolve_env(None), "local");
}

#[test]
fn u_cfg_05_secrets_not_in_toml() {
    // A PAT/password field must NOT be a recognized config key; deserialize ignores it.
    let c = DbConfig::from_str(
        "default_env=\"local\"\n[env.prod]\ndriver=\"supabase-cloud\"\nproject_ref=\"r\"\npat=\"SHOULD_BE_IGNORED\"\n",
    )
    .unwrap();
    // The struct has no pat/password field; round-tripping must not surface it.
    let round = c.to_toml().unwrap();
    assert!(!round.contains("SHOULD_BE_IGNORED"), "PAT must never be sourced from toml");
    assert!(!round.to_lowercase().contains("password"));
}

#[test]
fn u_cfg_08_unknown_driver_kind_errors() {
    let r = DbConfig::from_str("[env.x]\ndriver=\"mysql\"\n");
    assert!(r.is_err(), "unknown driver kind must error, not silently default");
}

#[test]
fn u_cfg_09_env_token_semantics() {
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    // A protected ref maps to the bare '' token; a non-protected stage keeps its name.
    // 'prod' points at the (fake) protected ref → token ''.
    assert_eq!(c.env_token("prod"), "");
    assert_eq!(c.env_token("local"), "local");
    assert!(c.env_is_protected("prod"));
    assert!(!c.env_is_protected("local"));
}

#[test]
fn u_safe_01_prod_refuse_predicate_is_by_ref() {
    // M5: protection keys off project_ref ∈ protected_refs, not driver kind, not a flag.
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    assert!(c.is_protected_ref("TEST_ONLY_ref_never_real"));
    assert!(!c.is_protected_ref("some_other_ref"));
    // A supabase-local env whose ref is in the set would be refused; a supabase-cloud
    // env whose ref is NOT in the set is allowed. (predicate-level check)
    assert!(!c.is_protected_ref("unlisted_cloud_ref"));
}

// I-CRAWL-05 / I-SAFE-02 / I-SAFE-01 (refusal side): a `Db` opened on a protected-ref env
// REFUSES destructive ops outright (asserts a *refusal*, never an execution — §0.3). No
// live DB is touched: the refusal fires before any driver call.
#[test]
fn u_safe_03_protected_env_refuses_destructive_ops() {
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    // 'prod' points at the FAKE protected ref TEST_ONLY_ref_never_real (never the real one).
    let db = substrate_db::Db::open(c, "prod", Some("fake-pat".into())).unwrap();
    assert!(db.protected, "prod env must resolve as protected (M5)");
    // crawl (I-CRAWL-05) and any --write on a protected ref (I-SAFE-01) are refused up front.
    assert!(db.refuse_if_protected("migrate crawl").is_err(), "crawl must be refused on a protected ref");
    assert!(db.refuse_if_protected("query --write").is_err(), "write must be refused on a protected ref");
}

// I-SAFE-02: `local reset` against a protected ref is refused (the refusal short-circuits
// before local_lifecycle shells anything).
#[tokio::test]
async fn u_safe_04_local_reset_refused_on_protected() {
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    let db = substrate_db::Db::open(c, "prod", Some("fake-pat".into())).unwrap();
    let res = db.local(substrate_db::driver::LocalOp::Reset).await;
    assert!(res.is_err(), "local reset must be refused outright on a protected ref (§9.2)");
}

// ── A.2 Migration discovery / ordering / graph ───────────────────────────────

fn discover(dir: &std::path::Path) -> Vec<Migration> {
    migration::discover(dir).unwrap()
}

#[test]
fn u_mig_01_discovery_ids() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path();
    write_migration(m, "ops", 1, "baseline", &[("up.sql", "select 1;")]);
    write_migration(m, "public", 1, "widgets", &[("up.sql", "create table w(id int);")]);
    write_migration(m, "core", 1, "x", &[("up.sql", "select 1;")]);
    write_migration(m, "mind", 1, "y", &[("up.sql", "select 1;")]);
    let migs = discover(m);
    assert_eq!(migs.len(), 4);
    assert!(migs.iter().any(|x| x.id == "public/0001_widgets"));
    assert!(migs.iter().any(|x| x.id == "ops/0001_baseline"));
}

#[test]
fn u_mig_05_06_topo_order() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path();
    write_migration(m, "public", 1, "a", &[("up.sql", "select 1;")]);
    write_migration(m, "public", 2, "b", &[("up.sql", "select 1;")]);
    write_migration(
        m,
        "core",
        1,
        "c",
        &[("up.sql", "-- @depends_on: public/0002_b\nselect 1;")],
    );
    let migs = discover(m);
    let order = migration::topo_order(&migs).unwrap();
    let pos = |id: &str| order.iter().position(|x| x.id == id).unwrap();
    assert!(pos("public/0001_a") < pos("public/0002_b"), "intra-schema seq order");
    assert!(pos("public/0002_b") < pos("core/0001_c"), "cross-schema depends_on order");
}

#[test]
fn u_mig_07_missing_dependency() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path();
    write_migration(
        m,
        "core",
        1,
        "c",
        &[("up.sql", "-- @depends_on: public/0099_nope\nselect 1;")],
    );
    let migs = discover(m);
    assert!(migration::topo_order(&migs).is_err(), "unresolved dependency must error");
}

#[test]
fn u_mig_08_cycle_detected() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path();
    write_migration(
        m,
        "public",
        1,
        "a",
        &[("up.sql", "-- @depends_on: core/0001_b\nselect 1;")],
    );
    write_migration(
        m,
        "core",
        1,
        "b",
        &[("up.sql", "-- @depends_on: public/0001_a\nselect 1;")],
    );
    let migs = discover(m);
    let e = migration::topo_order(&migs).unwrap_err();
    assert!(format!("{e}").contains("cycle"), "cycle must be named: {e}");
}

#[test]
fn u_mig_09_deterministic_tiebreak() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path();
    // independent migrations across schemas — order must be ops<public<core<mind.
    write_migration(m, "mind", 1, "m", &[("up.sql", "select 1;")]);
    write_migration(m, "core", 1, "c", &[("up.sql", "select 1;")]);
    write_migration(m, "public", 1, "p", &[("up.sql", "select 1;")]);
    write_migration(m, "ops", 1, "o", &[("up.sql", "select 1;")]);
    let order = migration::topo_order(&discover(m)).unwrap();
    let schemas: Vec<&str> = order.iter().map(|x| x.schema.as_str()).collect();
    assert_eq!(schemas, vec!["ops", "public", "core", "mind"]);
}

#[test]
fn u_mig_13_checksum_content_sensitive() {
    let tmp = tempfile::tempdir().unwrap();
    let m = tmp.path();
    write_migration(m, "public", 1, "a", &[("up.sql", "select 1;")]);
    let c1 = discover(m)[0].checksum.clone();
    // same bytes → same checksum
    let c1b = discover(m)[0].checksum.clone();
    assert_eq!(c1, c1b);
    // 1-byte change → different
    let tmp2 = tempfile::tempdir().unwrap();
    write_migration(tmp2.path(), "public", 1, "a", &[("up.sql", "select 2;")]);
    let c2 = discover(tmp2.path())[0].checksum.clone();
    assert_ne!(c1, c2);
}

// ── A.3 Lint gates ───────────────────────────────────────────────────────────

fn lint_one(files: &[(&str, &str)]) -> lint::LintReport {
    let tmp = tempfile::tempdir().unwrap();
    write_migration(tmp.path(), "public", 1, "m", files);
    lint::lint(&discover(tmp.path()))
}

fn full_six(extra: &[(&str, &str)]) -> Vec<(&'static str, String)> {
    // A complete six-file baseline (all sentinels valid) that we then override.
    let mut v: Vec<(&'static str, String)> = vec![
        ("up.sql", "create table t(id int);\n".to_string()),
        ("down.sql", "drop table t;\n".to_string()),
        ("test_up.sql", "-- @intentionally-none: grants only\n".to_string()),
        ("test_down.sql", "-- @intentionally-none: grants only\n".to_string()),
        ("fakedata_up.sql", "-- @intentionally-none: none\n".to_string()),
        ("fakedata_down.sql", "-- @intentionally-none: none\n".to_string()),
    ];
    for (k, val) in extra {
        if let Some(slot) = v.iter_mut().find(|(f, _)| f == k) {
            slot.1 = val.to_string();
        }
    }
    v
}

fn as_refs<'a>(v: &'a [(&'static str, String)]) -> Vec<(&'static str, &'a str)> {
    v.iter().map(|(k, val)| (*k, val.as_str())).collect()
}

#[test]
fn u_lint_s01_missing_test_up_no_sentinel_fails() {
    // test_up.sql absent entirely, and the other optional files present with sentinels.
    let files = vec![
        ("up.sql", "create table t(id int);"),
        ("down.sql", "drop table t;"),
        ("test_down.sql", "-- @intentionally-none: grants only"),
        ("fakedata_up.sql", "-- @intentionally-none: none"),
        ("fakedata_down.sql", "-- @intentionally-none: none"),
    ];
    let r = lint_one(&files);
    assert!(!r.is_clean(), "missing test_up with no sentinel must FAIL");
}

#[test]
fn u_lint_s02_sentinel_with_reason_passes() {
    let f = full_six(&[]);
    let r = lint_one(&as_refs(&f));
    assert!(r.is_clean(), "sentinel-with-reason must PASS: {:?}", r.findings);
}

#[test]
fn u_lint_s03_bare_sentinel_no_reason_fails() {
    let f = full_six(&[("test_up.sql", "-- @intentionally-none\n")]);
    let r = lint_one(&as_refs(&f));
    assert!(!r.is_clean(), "bare @intentionally-none with no reason must FAIL");
}

#[test]
fn u_lint_s04_typo_token_fails() {
    // Wrong token → treated as missing/empty → FAIL.
    let f = full_six(&[("test_up.sql", "-- @intentional-none: x\n")]);
    let r = lint_one(&as_refs(&f));
    assert!(!r.is_clean(), "typo'd sentinel token must not be recognized");
}

#[test]
fn u_lint_s09_lookalike_unicode_dash_fails() {
    // en-dash instead of ':' style lookalike — must be byte-exact, so FAIL.
    let f = full_six(&[("test_up.sql", "-- @intentionally-none\u{2013} grants only\n")]);
    let r = lint_one(&as_refs(&f));
    assert!(!r.is_clean(), "unicode look-alike sentinel must FAIL (byte-exact)");
}

#[test]
fn u_lint_q02_gap_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let f = full_six(&[]);
    write_migration(tmp.path(), "public", 1, "a", &as_refs(&f));
    write_migration(tmp.path(), "public", 3, "c", &as_refs(&f)); // gap at 0002
    let r = lint::lint(&discover(tmp.path()));
    assert!(!r.is_clean(), "sequence gap must FAIL");
    assert!(r.errors().any(|e| e.message.contains("gap")));
}

#[test]
fn u_lint_q03_dup_seq_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let f = full_six(&[]);
    write_migration(tmp.path(), "public", 42, "a", &as_refs(&f));
    write_migration(tmp.path(), "public", 42, "b", &as_refs(&f));
    let r = lint::lint(&discover(tmp.path()));
    assert!(!r.is_clean(), "duplicate seq must FAIL");
    assert!(r.errors().any(|e| e.message.contains("duplicate")));
}

#[test]
fn u_lint_q04_gap_isolated_per_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let f = full_six(&[]);
    // core has a gap; public is contiguous → only one gap error, scoped to core.
    write_migration(tmp.path(), "public", 1, "a", &as_refs(&f));
    write_migration(tmp.path(), "public", 2, "b", &as_refs(&f));
    write_migration(tmp.path(), "core", 1, "c", &as_refs(&f));
    write_migration(tmp.path(), "core", 3, "d", &as_refs(&f));
    let r = lint::lint(&discover(tmp.path()));
    let gaps: Vec<_> = r.errors().filter(|e| e.message.contains("gap")).collect();
    assert_eq!(gaps.len(), 1, "public must not be falsely flagged");
    assert!(gaps[0].message.contains("core"));
}

#[test]
fn u_lint_f01_f04_forbidden_patterns_fail() {
    for pat in [
        "CREATE OR REPLACE FUNCTION f() RETURNS void AS $$ $$;",
        "CREATE TABLE IF NOT EXISTS t(id int);",
        "DROP TABLE IF EXISTS t;",
        "ALTER FUNCTION f() OWNER TO postgres;",
        "ALTER EXTENSION http UPDATE;",
    ] {
        let f = full_six(&[("up.sql", pat)]);
        let r = lint_one(&as_refs(&f));
        assert!(!r.is_clean(), "forbidden pattern must FAIL: {pat}");
    }
}

#[test]
fn u_lint_f05_fenced_codegen_exempt() {
    let up = "-- @codegen:begin\nCREATE OR REPLACE FUNCTION f() RETURNS void AS $$ $$;\n-- @codegen:end\ncreate table t(id int);\n";
    let f = full_six(&[("up.sql", up)]);
    let r = lint_one(&as_refs(&f));
    assert!(r.is_clean(), "fenced codegen output must be exempt: {:?}", r.findings);
}

#[test]
fn u_lint_d01_d02_fakedata_dependency_warning() {
    // D01: test_up references a table only fakedata populates → WARNING.
    let f = full_six(&[
        ("up.sql", "create table gadgets(id int);\n"),
        ("fakedata_up.sql", "insert into gadgets(id) values (1);\n"),
        ("test_up.sql", "select count(*) from gadgets;\n"),
    ]);
    let r = lint_one(&as_refs(&f));
    assert!(
        r.findings.iter().any(|x| x.level == Level::Warning),
        "fakedata-dependency should WARN"
    );

    // D02: self-provisioning test → no warning.
    let f2 = full_six(&[
        ("up.sql", "create table gadgets(id int);\n"),
        ("fakedata_up.sql", "-- @intentionally-none: none\n"),
        ("test_up.sql", "insert into gadgets(id) values (1); select count(*) from gadgets;\n"),
    ]);
    let r2 = lint_one(&as_refs(&f2));
    assert!(
        !r2.findings.iter().any(|x| x.level == Level::Warning),
        "self-provisioning test should not warn"
    );
}

#[test]
fn u_lint_b_breaking_signature_classification() {
    let mut v2 = BTreeMap::new();
    v2.insert("user_id".to_string(), "uuid".to_string());
    v2.insert("room_id".to_string(), "uuid".to_string());

    // B01: v3 removes a param → Breaking.
    let mut removed = BTreeMap::new();
    removed.insert("user_id".to_string(), "uuid".to_string());
    assert_eq!(classify_change(&v2, &removed), CompatClass::Breaking);

    // B03: v3 adds an OPTIONAL param → Compatible.
    let mut added = v2.clone();
    added.insert("note".to_string(), "text".to_string());
    assert_eq!(classify_change(&v2, &added), CompatClass::Compatible);

    // B05: retype uuid→text → Breaking.
    let mut retyped = v2.clone();
    retyped.insert("user_id".to_string(), "text".to_string());
    assert_eq!(classify_change(&v2, &retyped), CompatClass::Breaking);
}

#[test]
fn u_lint_k01_effect_async_edge_requires_idempotency_key() {
    // H5: missing idempotency_key on effect-async edge → parse/validate FAILS.
    let c = Contract::parse(
        "handler: on_user_created\nversion: v1\nkind: edge\ninvocation: effect-async\n",
    );
    assert!(c.is_err(), "effect-async edge without idempotency_key must FAIL");
}

#[test]
fn u_lint_k03_validator_sync_edge_requires_timeout() {
    let c = Contract::parse(
        "handler: validate_membership\nversion: v3\nkind: edge\ninvocation: validator-sync\nfail_policy: fail-closed\n",
    );
    assert!(c.is_err(), "validator-sync edge without timeout_ms must FAIL");
}

// ── A.4 Codegen (golden-ish / determinism) ───────────────────────────────────

fn sql_validator_contract() -> Contract {
    Contract::parse(
        "handler: validate_room\nversion: v1\nkind: sql\ninvocation: validator-sync\nreturns: validator_result\nparams:\n  user_id: uuid\n  room_id: uuid\n",
    )
    .unwrap()
}

#[test]
fn u_gen_01_sql_wrapper_shape() {
    let g = codegen(&sql_validator_contract());
    assert!(g.sql_wrapper.contains("call_validate_room"));
    assert!(g.sql_wrapper.contains("ops.call_handler('validate_room'"));
    assert!(g.sql_wrapper.contains("jsonb_build_object"));
    // never a _vN literal in the wrapper (dispatch is by logical name).
    assert!(!g.sql_wrapper.contains("_v1"));
}

#[test]
fn u_gen_02_determinism() {
    let a = codegen(&sql_validator_contract());
    let b = codegen(&sql_validator_contract());
    assert_eq!(a.sql_wrapper, b.sql_wrapper);
    assert_eq!(a.trigger_sql, b.trigger_sql);
    assert_eq!(a.contract_jsonb, b.contract_jsonb);
}

#[test]
fn u_gen_04_validator_trigger_return_side_guard_order() {
    let g = codegen(&sql_validator_contract());
    let t = &g.trigger_sql;
    assert!(t.contains("trg_validate_room"));
    assert!(t.contains("check_violation"));
    assert!(t.contains("DETAIL = r->>'code'") || t.contains("DETAIL ="));
    // M4: assert_row_shape must come BEFORE jsonb_populate_record.
    let shape = t.find("assert_row_shape").expect("assert_row_shape present");
    let populate = t.find("jsonb_populate_record").expect("populate present");
    assert!(shape < populate, "row-shape guard must precede populate (M4)");
}

#[test]
fn u_gen_06_edge_test_down_is_resolution_assert() {
    let td = substrate_db::handler::gen_edge_test_down("on_user_created", "", "v1");
    assert!(td.contains("handler_active"));
    assert!(td.contains("'v1'"));
    // It's a resolution assert, not artifact deletion.
    assert!(!td.to_uppercase().contains("DROP FUNCTION"));
    assert!(!td.to_uppercase().contains("DELETE FROM"));
}

#[test]
fn u_gen_11_blocking_derived_no_mode() {
    let sync = sql_validator_contract();
    assert!(sync.blocking(), "validator-sync ⇒ blocking=true");
    let jsonb = sync.to_jsonb();
    assert_eq!(jsonb["blocking"], serde_json::json!(true));
    assert!(jsonb.get("mode").is_none(), "no `mode` field is written");

    let async_c = Contract::parse(
        "handler: on_user_created\nversion: v1\nkind: edge\ninvocation: effect-async\nidempotency_key: \"{user_id}\"\nparams:\n  user_id: uuid\n",
    )
    .unwrap();
    assert!(!async_c.blocking(), "effect-async ⇒ blocking=false");
}

#[test]
fn u_gen_03_ts_guard_for_edge() {
    let c = Contract::parse(
        "handler: validate_membership\nversion: v3\nkind: edge\ninvocation: validator-sync\ntimeout_ms: 800\nfail_policy: fail-closed\nparams:\n  user_id: uuid\n  n: int\n",
    )
    .unwrap();
    let g = codegen(&c);
    assert!(!g.ts_guard.is_empty(), "edge handler emits a TS guard");
    assert!(g.ts_guard.contains("ValidateMembershipParams"));
    assert!(g.ts_guard.contains("user_id: string"));
    assert!(g.ts_guard.contains("n: number"));
}

// ── A.5 Driver capability degradation ────────────────────────────────────────

#[test]
fn u_cap_01_02_03_capability_matrix() {
    let cloud = SupabaseCloudDriver::new("TEST_ONLY_ref_never_real", None);
    let cc = cloud.capabilities();
    assert!(cc.edge && cc.rls && cc.promote_target && cc.outbox && cc.registry);
    assert!(!cc.local_stack && !cc.pull);

    let local = SupabaseLocalDriver::new(".", None);
    let lc = local.capabilities();
    assert!(lc.edge && lc.rls && lc.local_stack && lc.pull && lc.outbox && lc.registry);
    assert!(!lc.promote_target);

    let sqlite = SqliteDriver::open_in_memory().unwrap();
    let sc = sqlite.capabilities();
    assert!(!sc.edge && !sc.rls && !sc.local_stack && !sc.pull && !sc.promote_target && !sc.outbox && !sc.registry);
}

#[tokio::test]
async fn u_cap_04_sqlite_edge_not_implemented() {
    let d = SqliteDriver::open_in_memory().unwrap();
    let bundle = substrate_db::driver::Bundle {
        bytes: vec![],
        source_hash: "x".into(),
        bundle_ref: "x".into(),
    };
    let e = d.deploy_edge("", "h", "v1", &bundle).await.unwrap_err();
    assert!(
        matches!(e, substrate_types::SubstrateError::NotImplemented { driver, .. } if driver == "sqlite"),
        "sqlite edge must be typed NotImplemented, got {e:?}"
    );
    let e2 = d.activate_handler("", "h", "v1").await.unwrap_err();
    assert!(matches!(e2, substrate_types::SubstrateError::NotImplemented { .. }));
    let tx = substrate_db::outbox::PgNetTransport { base_url: String::new() };
    let e3 = d.outbox_drain("", &tx).await.unwrap_err();
    assert!(matches!(e3, substrate_types::SubstrateError::NotImplemented { .. }));
}

#[tokio::test]
async fn u_cap_05_cloud_local_lifecycle_not_implemented() {
    let d = SupabaseCloudDriver::new("TEST_ONLY_ref_never_real", None);
    let e = d.local_lifecycle(substrate_db::driver::LocalOp::Up).await.unwrap_err();
    assert!(matches!(e, substrate_types::SubstrateError::NotImplemented { .. }));
    let e2 = d.pull(Default::default()).await.unwrap_err();
    assert!(matches!(e2, substrate_types::SubstrateError::NotImplemented { .. }));
}

#[test]
fn u_cap_07_error_display_actionable() {
    let e = substrate_types::SubstrateError::NotImplemented {
        command: "deploy_edge",
        driver: "sqlite",
        reason: "no edge runtime",
    };
    let s = format!("{e}");
    assert!(s.contains("deploy_edge") && s.contains("sqlite") && s.contains("no edge runtime"), "got: {s}");
}

// ── A.6 Misc unit (safety, tree, outbox key, promote confirm) ────────────────

#[test]
fn u_safe_02_typed_confirm_phrase() {
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    assert!(substrate_db::promote::check_confirm(&c, "promote prod").is_ok());
    assert!(substrate_db::promote::check_confirm(&c, "-y").is_err());
    assert!(substrate_db::promote::check_confirm(&c, "promote  prod").is_err());
}

// Finding 1: the promote gate BLOCKS on a stale generated wrapper. codegen_is_stale
// returns true when the checked-in wrapper differs from a fresh codegen; the gate then
// records a blocker and is_clear() is false. (No hardcoded false — main.rs computes the
// real staleness and feeds it here.)
#[test]
fn u_promo_codegen_stale_blocks_gate() {
    // A tree with no migrations to attest; the ONLY blocker we exercise is codegen.
    let migs: Vec<Migration> = vec![];
    // gate() is async but touches no driver when pending is empty; drive it on a runtime.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let d = SqliteDriver::open_in_memory().unwrap();
    // stale = true → blocked.
    let stale = rt.block_on(substrate_db::promote::gate(&d, &migs, &migs, true)).unwrap();
    assert!(stale.stale_codegen, "stale codegen flagged");
    assert!(!stale.is_clear(), "a stale wrapper MUST block promote (finding 1)");
    // stale = false → not blocked by codegen.
    let fresh = rt.block_on(substrate_db::promote::gate(&d, &migs, &migs, false)).unwrap();
    assert!(!fresh.stale_codegen);
    assert!(fresh.is_clear(), "fresh codegen + no pending → gate clear");
}

// Finding 1 (the real staleness computation): codegen_is_stale detects a
// missing/divergent generated wrapper on disk and reports true.
#[test]
fn u_promo_codegen_is_stale_detects_divergence() {
    let tmp = tempfile::tempdir().unwrap();
    let handlers = tmp.path().join("handlers");
    let generated = tmp.path().join("generated");
    std::fs::create_dir_all(handlers.join("h")).unwrap();
    std::fs::write(
        handlers.join("h").join("v1.yaml"),
        "handler: h\nversion: v1\nkind: sql\ninvocation: effect-async\nparams:\n  x: int\nreturns: void\n",
    )
    .unwrap();
    // No generated file yet → stale (missing wrapper blocks promote).
    assert!(
        substrate_db::handler::codegen_is_stale(&handlers, &generated).unwrap(),
        "missing generated wrapper is stale"
    );
    // Write the FRESH wrapper → not stale.
    let c = substrate_db::handler::load_contract(&handlers, "h", "v1").unwrap();
    let gen = substrate_db::handler::codegen(&c);
    std::fs::create_dir_all(&generated).unwrap();
    std::fs::write(generated.join("call_h.sql"), &gen.sql_wrapper).unwrap();
    assert!(
        !substrate_db::handler::codegen_is_stale(&handlers, &generated).unwrap(),
        "fresh wrapper is not stale"
    );
    // Corrupt it → stale again.
    std::fs::write(generated.join("call_h.sql"), "-- stale\n").unwrap();
    assert!(
        substrate_db::handler::codegen_is_stale(&handlers, &generated).unwrap(),
        "divergent wrapper is stale (finding 1)"
    );
}

// Finding 3: assert_promote_target refuses a non-cloud / non-prod-ref target so promote
// can never write env='' rows into the wrong backend. Typed-confirm alone is insufficient.
#[test]
fn u_promo_target_guard_refuses_wrong_backend() {
    let c = DbConfig::from_str(FX_TOML_BASE).unwrap();
    // `local` resolves to supabase-local (promote_target=false) → refused.
    let local = SupabaseLocalDriver::new(".", None);
    let r_local = substrate_db::promote::assert_promote_target(&c, "local", &local);
    assert!(r_local.is_err(), "a local target must be refused (not a promote target)");

    // A cloud driver whose ref is NOT protected → refused (non-prod cloud project).
    let cloud_unprotected = SupabaseCloudDriver::new("unlisted_cloud_ref", Some("pat".into()));
    // Point the `prod` env's ref at an unprotected ref for this check by re-parsing a cfg.
    let c2 = DbConfig::from_str(
        "default_env=\"local\"\n[safety]\nprotected_refs=[\"TEST_ONLY_ref_never_real\"]\nconfirm_phrase=\"promote prod\"\n\
         [env.prod]\ndriver=\"supabase-cloud\"\nproject_ref=\"unlisted_cloud_ref\"\n",
    )
    .unwrap();
    let r_unprot = substrate_db::promote::assert_promote_target(&c2, "prod", &cloud_unprotected);
    assert!(r_unprot.is_err(), "a cloud project not in protected_refs must be refused (finding 3)");

    // The intended prod (cloud + protected ref) is ALLOWED.
    let cloud_prod = SupabaseCloudDriver::new("TEST_ONLY_ref_never_real", Some("pat".into()));
    let r_ok = substrate_db::promote::assert_promote_target(&c, "prod", &cloud_prod);
    assert!(r_ok.is_ok(), "the intended cloud prod target is allowed: {r_ok:?}");
}

#[test]
fn u_tree_01_env_slug_derivation() {
    assert_eq!(substrate_db::tree::env_slug("feature/x"), "env_feature_x");
    assert_eq!(substrate_db::tree::env_slug("main"), "env_main");
    // trailing/leading non-alnum trimmed.
    assert_eq!(substrate_db::tree::env_slug("/foo/"), "env_foo");
}

#[test]
fn u_sentinel_only_helper() {
    assert!(is_sentinel_only("-- @intentionally-none: none\n"));
    assert!(is_sentinel_only("   \n-- a comment\n"));
    assert!(!is_sentinel_only("select 1;"));
}

#[test]
fn u_query_is_write_classification() {
    use substrate_db::query::is_write;
    assert!(!is_write("SELECT * FROM t"));
    assert!(!is_write("  with x as (select 1) select * from x"));
    assert!(is_write("INSERT INTO t VALUES (1)"));
    assert!(is_write("update t set a=1"));
    assert!(is_write("DROP TABLE t"));
}

// ── A.7 Snapshot (full catastrophic-recovery backup) ─────────────────────────

use substrate_db::snapshot::{
    self, compact_stamp, default_out_dir, is_managed_schema, schema_csv, Component, Manifest,
    Status,
};

#[test]
fn u_snap_01_compact_stamp_strips_punctuation() {
    // An ISO-8601 UTC stamp collapses to a filesystem-safe alnum run.
    assert_eq!(compact_stamp("2026-07-13T17:23:49Z"), "20260713T172349Z");
    // Degenerate input never yields an empty (would-be-invalid) directory name.
    assert_eq!(compact_stamp("::::"), "unknown");
}

#[test]
fn u_snap_02_default_out_dir_shape() {
    let d = default_out_dir("prod", "2026-07-13T17:23:49Z");
    assert_eq!(
        d,
        std::path::PathBuf::from(".db-snapshots").join("prod-20260713T172349Z")
    );
}

#[test]
fn u_snap_03_schema_csv_and_managed_classification() {
    assert_eq!(
        schema_csv(&["public".into(), "core".into(), "ops".into()]),
        "public,core,ops"
    );
    // Supabase-managed schemas are presence-only (never dumped); app schemas are not.
    assert!(is_managed_schema("auth"));
    assert!(is_managed_schema("storage"));
    assert!(is_managed_schema("cron")); // classified managed; captured specially
    assert!(!is_managed_schema("public"));
    assert!(!is_managed_schema("core"));
    assert!(!is_managed_schema("mind"));
}

#[test]
fn u_snap_04_manifest_json_roundtrips() {
    let m = sample_manifest();
    let json = m.to_json();
    let back: Manifest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.env, "local");
    assert_eq!(back.components.len(), m.components.len());
    assert!(back.read_only, "a snapshot is always read-only against source");
}

#[test]
fn u_snap_05_manifest_markdown_surfaces_failures_and_recovery() {
    let md = sample_manifest().to_markdown();
    // The human recovering reads this: a FAILED component is loud, gaps are called out,
    // and the ordered manual recovery steps are present.
    assert!(md.contains("FAILED"), "a failed component must be visible in the table");
    assert!(md.contains("Recovery gaps"), "gaps section must render");
    assert!(md.contains("Apply `roles.sql`, then `schema.sql`, then `data.sql`"));
    assert!(md.contains("edge_functions/"));
}

fn sample_manifest() -> Manifest {
    Manifest {
        tool: "db snapshot vX".into(),
        env: "local".into(),
        driver: "supabase-local".into(),
        project_ref: Some("some_ref".into()),
        taken_at: "2026-07-13T17:23:49Z".into(),
        taken_at_source: "db now()".into(),
        read_only: true,
        app_schemas_requested: vec!["public".into(), "core".into()],
        app_schemas_captured: vec!["public".into()],
        managed_schemas_present: vec!["auth".into(), "storage".into()],
        total_bytes: 1234,
        components: vec![
            component("schemas", Status::Captured, "all schemas classified", 100),
            component("data", Status::Captured, "supabase db dump --data-only", 1000),
            component("cron", Status::Failed, "cron.job not queryable", 0),
        ],
        gaps: vec!["cron dump missing".into()],
    }
}

fn component(name: &str, status: Status, detail: &str, bytes: u64) -> Component {
    Component {
        name: name.into(),
        status,
        detail: detail.into(),
        files: vec![],
        bytes,
    }
}

// A driver-backed end-to-end run WITHOUT Docker/network: the sqlite driver exposes no
// dump URL and rejects the Postgres-catalog queries, so every component either skips or
// fails — yet `run()` MUST still complete (create-only, robust) and leave a fully-formed
// output dir with both manifests. This locks in the "record the gap, never abort" contract.
#[tokio::test]
async fn u_snap_06_run_is_robust_and_records_gaps() {
    let tmp = tempfile::tempdir().unwrap();
    let sqlite_path = tmp.path().join("snap.sqlite");
    let edge_missing = tmp.path().join("no-edge-here");
    let cfg_toml = format!(
        "default_env = \"sqlite\"\n\
         [dirs]\n\
         edge = \"{}\"\n\
         [env.sqlite]\n\
         driver = \"sqlite\"\n\
         path = \"{}\"\n",
        edge_missing.display(),
        sqlite_path.display()
    );
    let cfg = DbConfig::from_str(&cfg_toml).unwrap();
    let db = substrate_db::Db::open(cfg, "sqlite", None).unwrap();

    let out = tmp.path().join("snapshot-out");
    let opts = snapshot::SnapshotOptions { out: Some(out.clone()), app_schemas: vec![] };
    let report = snapshot::run(&db, &opts).await.expect("snapshot run must not abort");

    // The output dir and BOTH manifests exist.
    assert!(report.out_dir.exists());
    assert!(report.manifest_json_path.exists(), "manifest.json written");
    assert!(report.manifest_md_path.exists(), "MANIFEST.md written");
    assert!(out.join("schemas.txt").exists(), "schemas.txt always written");

    let m = &report.manifest;
    assert!(m.read_only, "read-only against source");
    // No dump URL on sqlite → the pg_dump-backed components are skipped and the gap noted.
    assert!(
        m.gaps.iter().any(|g| g.contains("dump URL")),
        "missing dump URL must be recorded as a gap: {:?}",
        m.gaps
    );
    // The dump components are present as skipped (not silently dropped).
    for name in ["schema", "data", "cron", "roles"] {
        let c = m.components.iter().find(|c| c.name == name).expect("component present");
        assert_eq!(c.status, Status::Skipped, "{name} skipped without a dump URL");
    }
    // Edge source dir absent → skipped, recorded.
    let edge = m.components.iter().find(|c| c.name == "edge_source").unwrap();
    assert_eq!(edge.status, Status::Skipped);
}
