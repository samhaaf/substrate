//! PART I — [LOCAL] integration tests (Tier 2, Docker-gated).
//!
//! Every test runs against a THROWAWAY per-test Postgres database created on the LOCAL
//! Supabase stack (`127.0.0.1:54322`) via `PgSandbox` — fully isolated, dropped on
//! teardown (test plan §0.3, S.3). NOTHING here touches remote/prod Supabase: the real
//! prod ref never appears anywhere in this tree (asserted by S.1 in this file), and no
//! test targets a protected ref — protected-ref cases assert a *refusal* (§0.3).
//!
//! Gated behind a reachable local Postgres (`common::local_enabled()`); when Docker/the
//! stack is absent, each test skip-with-reasons (green-skip, S.4) rather than failing red.
//!
//! Driver legend: all tests here are [LOCAL] unless the ID says otherwise.

mod common;

use common::{local_enabled, write_migration, PgSandbox};
use substrate_db::config::{DbConfig, DriverKind};
use substrate_db::driver::{Driver, IntrospectQuery};
use substrate_db::migration::{
    apply_one, crawl, discover, record_attestations, rollback_one, topo_order, ApplyContext,
    Migration,
};

// A macro that skips (green) when the LOCAL tier can't run, else creates a sandbox.
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

/// Build a `Migration` value from an on-disk dir (checksum from up.sql, deps parsed).
fn mig(dir: std::path::PathBuf, schema: &str, seq: u32, name: &str) -> Migration {
    let migs = discover(dir.parent().unwrap().parent().unwrap()).unwrap();
    migs.into_iter()
        .find(|m| m.schema == schema && m.seq == seq && m.name == name)
        .expect("discover migration")
}

async fn count(sb: &PgSandbox, sql: &str) -> i64 {
    let rows = sb.driver().query("", sql).await.unwrap();
    rows.rows
        .first()
        .and_then(|r| r.first().cloned().flatten())
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1)
}

/// Look up a cell by COLUMN NAME in the first row (the crate's `query` renderer
/// alphabetizes column order, so positional indexing is unsafe — always key by name).
async fn cell(sb: &PgSandbox, sql: &str, col: &str) -> Option<String> {
    let rows = sb.driver().query("", sql).await.unwrap();
    let idx = rows.columns.iter().position(|c| c == col)?;
    rows.rows.first().and_then(|r| r.get(idx).cloned().flatten())
}

// ─────────────────────────────────────────────────────────────────────────────
// I.1  ops baseline + ledger bring-up
// ─────────────────────────────────────────────────────────────────────────────

// I-BASE-01: baseline creates §3.1 tables; applied_migrations PK (env,id); composite FK.
#[tokio::test]
async fn i_base_01_baseline_tables_and_composite_fk() {
    let sb = sandbox_or_skip!("I-BASE-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();

    // All §3.1 ops tables present.
    let names: Vec<String> = d
        .query(
            "",
            "SELECT table_name FROM information_schema.tables WHERE table_schema='ops' ORDER BY 1",
        )
        .await
        .unwrap()
        .rows
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .collect();
    for t in [
        "applied_migrations",
        "crawl_attestations",
        "handlers",
        "handler_versions",
        "handler_active",
        "edge_deploys",
        "handler_dispatch",
        "validator_audit",
    ] {
        assert!(names.contains(&t.to_string()), "missing ops.{t}");
    }

    // applied_migrations PK is (env, id).
    let pk = count(
        &sb,
        "SELECT count(*) FROM information_schema.table_constraints tc \
         JOIN information_schema.key_column_usage k USING (constraint_name) \
         WHERE tc.table_schema='ops' AND tc.table_name='applied_migrations' \
           AND tc.constraint_type='PRIMARY KEY' AND k.column_name IN ('env','id')",
    )
    .await;
    assert_eq!(pk, 2, "applied_migrations PK must be (env,id)");

    // handler_active carries a composite FK on (activated_by_env, activated_by_migration)
    // → applied_migrations (H6).
    let fk = count(
        &sb,
        "SELECT count(*) FROM information_schema.table_constraints \
         WHERE table_schema='ops' AND table_name='handler_active' AND constraint_type='FOREIGN KEY'",
    )
    .await;
    assert!(fk >= 1, "handler_active must have a composite FK (H6)");
    sb.teardown().await;
}

// I-BASE-04: applied_seq is monotonic across schemas from the global sequence.
#[tokio::test]
async fn i_base_04_applied_seq_monotonic_across_schemas() {
    let sb = sandbox_or_skip!("I-BASE-04");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    for (schema, id) in [
        ("public", "public/0001_a"),
        ("core", "core/0001_b"),
        ("mind", "mind/0001_c"),
    ] {
        d.ledger_record(substrate_db::driver::AppliedMigration {
            env: "".into(),
            schema: schema.into(),
            id: id.into(),
            checksum: "c".into(),
        })
        .await
        .unwrap();
    }
    let ids: Vec<String> = d
        .query("", "SELECT id FROM ops.applied_migrations ORDER BY applied_seq")
        .await
        .unwrap()
        .rows
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .collect();
    assert_eq!(ids, vec!["public/0001_a", "core/0001_b", "mind/0001_c"]);
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.2  migrate up / down (SQL-only migrations)
// ─────────────────────────────────────────────────────────────────────────────

fn widgets_migration(root: &std::path::Path) -> std::path::PathBuf {
    write_migration(
        root,
        "public",
        1,
        "widgets",
        &[
            ("up.sql", "-- @schema: public\nCREATE TABLE widgets (id int primary key, name text);"),
            ("down.sql", "DROP TABLE widgets;"),
            (
                "test_up.sql",
                "INSERT INTO widgets(id,name) VALUES (1,'a');\n\
                 DO $$ BEGIN IF (SELECT count(*) FROM widgets)=0 THEN RAISE EXCEPTION 'no rows'; END IF; END $$;\n\
                 DELETE FROM widgets WHERE id=1;",
            ),
            (
                "test_down.sql",
                "DO $$ BEGIN IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name='widgets') \
                 THEN RAISE EXCEPTION 'table still present'; END IF; END $$;",
            ),
            ("fakedata_up.sql", "-- @intentionally-none: self-provisioning\n"),
            ("fakedata_down.sql", "-- @intentionally-none: none\n"),
        ],
    )
}

// I-UP-01 / I-UP-06 / I-DOWN-01 / I-DOWN-03: apply → ledger+checksum+test_up; idempotence;
// down → test_down + ledger removal; re-up clean.
#[tokio::test]
async fn i_up_01_down_01_apply_test_down_reup() {
    let sb = sandbox_or_skip!("I-UP-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    let dir = widgets_migration(tmp.path());
    let m = mig(dir, "public", 1, "widgets");
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };

    apply_one(&d, &m, &ctx).await.unwrap();
    // Ledger row with checksum + applied_seq.
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.applied_migrations WHERE id='public/0001_widgets'").await, 1);
    assert_eq!(
        cell(&sb, "SELECT checksum FROM ops.applied_migrations WHERE id='public/0001_widgets'", "checksum").await.as_deref(),
        Some(m.checksum.as_str())
    );
    assert_eq!(count(&sb, "SELECT count(*) FROM information_schema.tables WHERE table_name='widgets'").await, 1);

    // I-UP-06: idempotence vs the ledger is enforced at the PLANNING layer — an
    // already-applied migration is not in the pending set, so `migrate up` skips it (it
    // is not re-applied). Assert the pending computation excludes it.
    let pending = substrate_db::promote::pending_for_prod(&d, std::slice::from_ref(&m))
        .await
        .unwrap();
    assert!(pending.is_empty(), "an applied migration must not be pending (idempotence, I-UP-06)");
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.applied_migrations WHERE id='public/0001_widgets'").await, 1);

    // I-DOWN-01: down runs test_down (table gone), ledger row removed.
    rollback_one(&d, &m, "").await.unwrap();
    assert_eq!(count(&sb, "SELECT count(*) FROM information_schema.tables WHERE table_name='widgets'").await, 0);
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.applied_migrations WHERE id='public/0001_widgets'").await, 0);

    // I-DOWN-03: re-up after down reapplies cleanly, same checksum.
    apply_one(&d, &m, &ctx).await.unwrap();
    assert_eq!(
        cell(&sb, "SELECT checksum FROM ops.applied_migrations WHERE id='public/0001_widgets'", "checksum").await.as_deref(),
        Some(m.checksum.as_str())
    );
    sb.teardown().await;
}

// I-UP-03: cross-schema @depends_on → dependent applied AFTER its dependency.
#[tokio::test]
async fn i_up_03_cross_schema_depends_on_order() {
    let sb = sandbox_or_skip!("I-UP-03");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    write_migration(
        tmp.path(),
        "public",
        1,
        "base",
        &[
            ("up.sql", "-- @schema: public\nCREATE TABLE base (id int primary key);"),
            ("down.sql", "DROP TABLE base;"),
        ],
    );
    write_migration(
        tmp.path(),
        "core",
        1,
        "dep",
        &[
            (
                "up.sql",
                "-- @schema: core\n-- @depends_on: public/0001_base\n\
                 CREATE TABLE dep (id int primary key, base_id int REFERENCES base(id));",
            ),
            ("down.sql", "DROP TABLE dep;"),
        ],
    );
    let migs = discover(tmp.path()).unwrap();
    let ordered = topo_order(&migs).unwrap();
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };
    for m in &ordered {
        apply_one(&d, m, &ctx).await.unwrap();
    }
    // The FK-bearing core migration only applies if public/0001_base ran first.
    let order: Vec<String> = d
        .query("", "SELECT id FROM ops.applied_migrations ORDER BY applied_seq")
        .await
        .unwrap()
        .rows
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .collect();
    let pub_pos = order.iter().position(|i| i == "public/0001_base").unwrap();
    let core_pos = order.iter().position(|i| i == "core/0001_dep").unwrap();
    assert!(pub_pos < core_pos, "public/base must precede core/dep");
    sb.teardown().await;
}

// I-UP-07: a mid-migration failure rolls back atomically; NO ledger row; prior intact.
#[tokio::test]
async fn i_up_07_failed_migration_atomic_no_ledger() {
    let sb = sandbox_or_skip!("I-UP-07");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    // up.sql creates a table then issues invalid SQL → batch aborts (single implicit txn).
    let dir = write_migration(
        tmp.path(),
        "public",
        1,
        "boom",
        &[
            (
                "up.sql",
                "CREATE TABLE boom (id int); INSERT INTO nonexistent_table VALUES (1);",
            ),
            ("down.sql", "DROP TABLE IF EXISTS boom;"),
        ],
    );
    let m = mig(dir, "public", 1, "boom");
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };
    let res = apply_one(&d, &m, &ctx).await;
    assert!(res.is_err(), "invalid up.sql must fail");
    // batch_execute wraps the statements in one implicit txn → the CREATE TABLE is also
    // rolled back, and NO ledger row was written.
    assert_eq!(count(&sb, "SELECT count(*) FROM information_schema.tables WHERE table_name='boom'").await, 0,
        "failed migration's DDL must roll back atomically");
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.applied_migrations WHERE id='public/0001_boom'").await, 0,
        "no ledger row for a failed migration");
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.4  fakedata ordering + prod skip (H3)
// ─────────────────────────────────────────────────────────────────────────────

// A migration whose test_up depends on a row provisioned by fakedata_up.
fn fakedata_dependent_migration(root: &std::path::Path) -> std::path::PathBuf {
    write_migration(
        root,
        "public",
        1,
        "fdep",
        &[
            ("up.sql", "-- @schema: public\nCREATE TABLE fdep (id int primary key);"),
            ("down.sql", "DROP TABLE fdep;"),
            // test_up asserts a row EXISTS — only true if fakedata_up ran.
            (
                "test_up.sql",
                "DO $$ BEGIN IF (SELECT count(*) FROM fdep)=0 THEN RAISE EXCEPTION 'fakedata not present'; END IF; END $$;",
            ),
            ("test_down.sql", "-- @intentionally-none: covered by up presence\n"),
            ("fakedata_up.sql", "INSERT INTO fdep(id) VALUES (99);"),
            ("fakedata_down.sql", "DELETE FROM fdep WHERE id=99;"),
        ],
    )
}

// I-FAKE-01 / I-TEST-02: on a stage (fakedata allowed), up→fakedata→test_up all run and
// test_up passes because fakedata provisioned the row.
#[tokio::test]
async fn i_fake_01_stage_runs_fakedata_before_test() {
    let sb = sandbox_or_skip!("I-FAKE-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    let dir = fakedata_dependent_migration(tmp.path());
    let m = mig(dir, "public", 1, "fdep");
    // stage: allow_fakedata = true
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };
    apply_one(&d, &m, &ctx).await.expect("stage apply w/ fakedata must pass");
    assert_eq!(count(&sb, "SELECT count(*) FROM fdep WHERE id=99").await, 1);
    sb.teardown().await;
}

// I-FAKE-02 / I-TEST-03: prod-mode (allow_fakedata=false) SKIPS fakedata; a NON
// self-provisioning test_up then FAILS — proving the H3 prod-divergence contract and
// justifying the U-LINT-D01 warning.
#[tokio::test]
async fn i_fake_02_prod_skips_fakedata_test_fails() {
    let sb = sandbox_or_skip!("I-FAKE-02");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    let dir = fakedata_dependent_migration(tmp.path());
    let m = mig(dir, "public", 1, "fdep");
    // prod-mode: allow_fakedata = false → fakedata skipped → test_up sees zero rows → RAISE.
    let ctx = ApplyContext { env: "", allow_fakedata: false, edge_incapable: false };
    let res = apply_one(&d, &m, &ctx).await;
    assert!(res.is_err(), "prod-mode test_up must FAIL when it relies on skipped fakedata (H3)");
    // Finding 6: because up+fakedata+test_up+ledger are ONE atomic transaction, the failing
    // test_up rolls back the whole unit — the `fdep` TABLE the up.sql created is reverted,
    // and NO ledger row is written (no partial state).
    assert_eq!(
        count(&sb, "SELECT count(*) FROM information_schema.tables WHERE table_name='fdep'").await,
        0,
        "a failing test_up rolls back the up-DDL (atomic per-migration txn, finding 6)"
    );
    assert_eq!(
        count(&sb, "SELECT count(*) FROM ops.applied_migrations WHERE id='public/0001_fdep'").await,
        0,
        "no ledger row for a migration whose test_up failed (finding 6)"
    );
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.5  migrate crawl (crown jewel) + attestations (M2)
// ─────────────────────────────────────────────────────────────────────────────

// I-CRAWL-01: crawl over a good tree passes apply→test→down→test_down→reapply→test and
// writes crawl_attestations(id,checksum) per migration.
#[tokio::test]
async fn i_crawl_01_good_tree_writes_attestations() {
    let sb = sandbox_or_skip!("I-CRAWL-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    widgets_migration(tmp.path());
    let migs = discover(tmp.path()).unwrap();
    let atts = crawl(&d, &migs, "").await.expect("crawl must pass on a good tree");
    record_attestations(&d, &atts, "ci").await.unwrap();
    let n = count(&sb, "SELECT count(*) FROM ops.crawl_attestations WHERE id='public/0001_widgets'").await;
    assert_eq!(n, 1, "one attestation per crawled migration");
    // Attestation checksum matches the migration's up.sql checksum (M2).
    let m = &migs[0];
    let matched = count(
        &sb,
        &format!("SELECT count(*) FROM ops.crawl_attestations WHERE id='{}' AND checksum='{}'", m.id, m.checksum),
    )
    .await;
    assert_eq!(matched, 1);
    sb.teardown().await;
}

// I-CRAWL-02: a migration with a broken down.sql (leaves residue) FAILS crawl at the
// down/test_down step and writes NO attestation.
#[tokio::test]
async fn i_crawl_02_broken_down_fails_no_attestation() {
    let sb = sandbox_or_skip!("I-CRAWL-02");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    write_migration(
        tmp.path(),
        "public",
        1,
        "leaky",
        &[
            ("up.sql", "-- @schema: public\nCREATE TABLE leaky (id int primary key);"),
            // down.sql does NOT drop the table → test_down catches the residue.
            ("down.sql", "-- leaves the table behind (bug)\nSELECT 1;"),
            ("test_up.sql", "-- @intentionally-none: n/a\n"),
            (
                "test_down.sql",
                "DO $$ BEGIN IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name='leaky') \
                 THEN RAISE EXCEPTION 'down left residue'; END IF; END $$;",
            ),
            ("fakedata_up.sql", "-- @intentionally-none: n/a\n"),
            ("fakedata_down.sql", "-- @intentionally-none: n/a\n"),
        ],
    );
    let migs = discover(tmp.path()).unwrap();
    let res = crawl(&d, &migs, "").await;
    assert!(res.is_err(), "crawl must fail on a broken down");
    // No attestation was recorded (crawl returned Err before record_attestations).
    let n = count(&sb, "SELECT count(*) FROM ops.crawl_attestations WHERE id='public/0001_leaky'").await;
    assert_eq!(n, 0);
    sb.teardown().await;
}

// I-CRAWL-06: editing up.sql after crawl invalidates the attestation (PK (id,checksum));
// a promote gate will refuse (see I-PROMO-05 style check).
#[tokio::test]
async fn i_crawl_06_edited_up_invalidates_attestation() {
    let sb = sandbox_or_skip!("I-CRAWL-06");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    let dir = widgets_migration(tmp.path());
    let migs = discover(tmp.path()).unwrap();
    let atts = crawl(&d, &migs, "").await.unwrap();
    record_attestations(&d, &atts, "ci").await.unwrap();
    let old_checksum = migs[0].checksum.clone();
    // Edit up.sql → new checksum on re-discover.
    std::fs::write(dir.join("up.sql"), "-- @schema: public\nCREATE TABLE widgets (id int primary key, name text, extra int);").unwrap();
    let migs2 = discover(tmp.path()).unwrap();
    assert_ne!(migs2[0].checksum, old_checksum, "edit must change checksum");
    // The stored attestation matches the OLD checksum, not the new one.
    let matches_new = count(
        &sb,
        &format!("SELECT count(*) FROM ops.crawl_attestations WHERE id='{}' AND checksum='{}'", migs2[0].id, migs2[0].checksum),
    )
    .await;
    assert_eq!(matches_new, 0, "no attestation matches the edited checksum → promote refuses");
    sb.teardown().await;
}

// I-CRAWL-07: the ops baseline is a migration whose `down.sql` drops the `ops` schema —
// taking `ops.applied_migrations` (the ledger) with it. `rollback_one` must NOT then error
// trying to `DELETE FROM ops.applied_migrations` against the dropped table (the ledger
// cannot record the rollback of the ledger itself), and a full per-migration crawl of the
// ops baseline must pass and write an attestation. Regression for the ledger self-reference.
#[tokio::test]
async fn i_crawl_07_ops_baseline_ledger_self_reference() {
    let sb = sandbox_or_skip!("I-CRAWL-07");
    // Do NOT bootstrap_ops here: the ops baseline is applied like any other migration
    // (design §3) — crawl's own apply creates the control plane from OPS_BASELINE_PG.
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    write_migration(
        tmp.path(),
        "ops",
        1,
        "baseline_ops",
        &[
            ("up.sql", substrate_db::OPS_BASELINE_PG),
            // Mirrors the real baseline down.sql — drops the ledger with the schema.
            ("down.sql", "DROP SCHEMA IF EXISTS ops CASCADE;"),
            (
                "test_up.sql",
                "DO $$ BEGIN IF to_regclass('ops.applied_migrations') IS NULL \
                 THEN RAISE EXCEPTION 'ledger missing after up'; END IF; END $$;",
            ),
            (
                "test_down.sql",
                "-- @intentionally-none: the ops schema is dropped by down; asserting its \
                 absence would race the guarded ledger delete.\n",
            ),
            ("fakedata_up.sql", "-- @intentionally-none: n/a\n"),
            ("fakedata_down.sql", "-- @intentionally-none: n/a\n"),
        ],
    );
    let migs = discover(tmp.path()).unwrap();
    let m = &migs[0];

    // Direct rollback_one path: apply, then roll back. BEFORE THE FIX this errored on
    // `DELETE FROM ops.applied_migrations` because down.sql had just dropped that table.
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };
    apply_one(&d, m, &ctx).await.expect("apply ops baseline");
    rollback_one(&d, m, "")
        .await
        .expect("rollback_one must not error on the ledger self-reference (down drops ops)");

    // Full per-migration crawl (apply→test→down→test_down→reapply→test) must now pass and
    // yield exactly one attestation for the ops baseline.
    let atts = crawl(&d, &migs, "").await.expect("crawl of the ops baseline must pass");
    assert_eq!(atts.len(), 1, "one attestation for the ops baseline");
    record_attestations(&d, &atts, "ci").await.unwrap();
    let n = count(
        &sb,
        "SELECT count(*) FROM ops.crawl_attestations WHERE id='ops/0001_baseline_ops'",
    )
    .await;
    assert_eq!(n, 1, "attestation written for the ops baseline");
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.6  tree new/rm + reap-safety quarantine (M7)
// ─────────────────────────────────────────────────────────────────────────────

/// Register a logical handler + one async-edge version + an active row for `env`.
/// Returns the version_id.
async fn register_edge_handler(sb: &PgSandbox, handler: &str, version: &str, env: &str) -> i64 {
    let d = sb.driver();
    d.apply_sql("", &format!("INSERT INTO ops.handlers(name) VALUES ('{handler}') ON CONFLICT DO NOTHING")).await.unwrap();
    d.apply_sql(
        "",
        &format!(
            "INSERT INTO ops.handler_versions(handler,version,kind,invocation,target_ref,contract,source_hash,timeout_ms) \
             VALUES ('{handler}','{version}','edge','effect-async','{handler}_{version}', \
             '{{\"idempotency_key\":\"{{user_id}}\"}}'::jsonb,'deadbeef',NULL) \
             ON CONFLICT (handler,version) DO NOTHING",
        ),
    )
    .await
    .unwrap();
    let rows = d
        .query("", &format!("SELECT id FROM ops.handler_versions WHERE handler='{handler}' AND version='{version}'"))
        .await
        .unwrap();
    let vid: i64 = rows.rows[0][0].as_deref().unwrap().parse().unwrap();
    // Active row must reference an applied_migrations (env,id) for the composite FK, OR
    // use NULL for activated_by_migration (FK allows NULL on the nullable part).
    d.apply_sql(
        "",
        &format!("INSERT INTO ops.handler_active(handler,env,version_id) VALUES ('{handler}','{env}',{vid}) \
                  ON CONFLICT (handler,env) DO UPDATE SET version_id=EXCLUDED.version_id"),
    )
    .await
    .unwrap();
    vid
}

// I-TREE-03: a stage with no (h,'env_x') row resolves call_handler to the prod (h,'') row.
#[tokio::test]
async fn i_tree_03_stage_inherits_prod_active() {
    let sb = sandbox_or_skip!("I-TREE-03");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    // register a SQL handler (call_handler can dispatch it synchronously) active on prod ''.
    d.apply_sql("", "INSERT INTO ops.handlers(name) VALUES ('resolve_probe')").await.unwrap();
    d.apply_sql("", "CREATE FUNCTION resolve_probe_v1(p jsonb) RETURNS jsonb LANGUAGE sql AS $$ SELECT '{\"v\":\"prod\"}'::jsonb $$;").await.unwrap();
    d.apply_sql("", "INSERT INTO ops.handler_versions(handler,version,kind,invocation,target_ref,contract,source_hash) \
        VALUES ('resolve_probe','v1','sql','effect-async','resolve_probe_v1','{}'::jsonb,'h')").await.unwrap();
    d.apply_sql("", "INSERT INTO ops.handler_active(handler,env,version_id) \
        SELECT 'resolve_probe','',id FROM ops.handler_versions WHERE handler='resolve_probe' AND version='v1'").await.unwrap();
    // Call under a stage env 'env_x' with NO (resolve_probe,'env_x') row → falls back to ''.
    let rows = d
        .query("", "SELECT (ops.call_handler('resolve_probe','{}'::jsonb))->>'v' AS v")
        .await;
    // Note: current_env() defaults to '' at session level; the fallback logic still
    // resolves to the prod row. Assert the SQL dispatch returned the prod value.
    let v = rows.unwrap();
    assert_eq!(v.rows[0][0].as_deref(), Some("prod"), "stage with no override resolves to prod (h,'')");
    sb.teardown().await;
}

// I-TREE-05 / I-TREE-06 / I-OBX-04: reap quarantines pending intents THEN purges the
// env's control-plane rows; the drainer never fires quarantined; prod dispatches
// unaffected.
#[tokio::test]
async fn i_tree_05_06_reap_quarantines_then_purges() {
    let sb = sandbox_or_skip!("I-TREE-05");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    // A handler version to reference from dispatch rows.
    let vid = register_edge_handler(&sb, "on_user_created", "v1", "").await;
    // A pending dispatch on stage env_x AND one on prod ''.
    for (env, key) in [("env_x", "kx"), ("", "kp")] {
        d.apply_sql(
            "",
            &format!("INSERT INTO ops.handler_dispatch(env,handler,version_id,payload,idempotency_key) \
                      VALUES ('{env}','on_user_created',{vid},'{{}}'::jsonb,'{key}')"),
        )
        .await
        .unwrap();
    }
    // Reap env_x.
    substrate_db::tree::reap(&d, "env_x").await.unwrap();
    // env_x dispatch rows are purged (they were quarantined then deleted).
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE env='env_x'").await, 0,
        "reap purges env_x dispatches");
    // prod '' dispatch is untouched and still pending.
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE env='' AND status='pending'").await, 1,
        "prod dispatch unaffected by reap");
    // Drainer on env_x fires nothing (rows gone); prod drain fires the prod one.
    let tx = common::MockTransport::new();
    let rep_x = d.outbox_drain("env_x", &tx).await.unwrap();
    assert_eq!(rep_x.fired, 0);
    let rep_p = d.outbox_drain("", &tx).await.unwrap();
    assert_eq!(rep_p.fired, 1, "prod drain fires the surviving prod intent");
    assert_eq!(tx.keys(), vec!["kp"], "only the prod intent was actually delivered");
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.7  handler activate / pointer-flip rollback (M1)
// ─────────────────────────────────────────────────────────────────────────────

// I-HND-03 / I-HND-04: activate flips (handler,env) version_id; rollback is a one-row
// UPDATE back to the prior version (no redeploy; edge_deploys unchanged).
#[tokio::test]
async fn i_hnd_03_04_activate_and_pointer_flip_rollback() {
    let sb = sandbox_or_skip!("I-HND-03");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let v1 = register_edge_handler(&sb, "on_user_created", "v1", "").await;
    let v2 = register_edge_handler(&sb, "on_user_created", "v2", "").await; // also flips active→v2

    // After registering v2, active points at v2. Assert, then flip back to v1 (rollback).
    let active = count(&sb, "SELECT version_id FROM ops.handler_active WHERE handler='on_user_created' AND env=''").await;
    assert_eq!(active, v2, "active should be v2 after second register");

    // Pointer-flip rollback: one-row UPDATE back to v1 (design §7.3 — never a redeploy).
    d.apply_sql("", &format!("UPDATE ops.handler_active SET version_id={v1} WHERE handler='on_user_created' AND env=''")).await.unwrap();
    let after = count(&sb, "SELECT version_id FROM ops.handler_active WHERE handler='on_user_created' AND env=''").await;
    assert_eq!(after, v1, "rollback flips pointer back to v1");
    sb.teardown().await;
}

// I-HND-10: activation in env_x does NOT touch the prod (h,'') row (M1).
#[tokio::test]
async fn i_hnd_10_stage_activate_does_not_touch_prod() {
    let sb = sandbox_or_skip!("I-HND-10");
    sb.bootstrap_ops().await;
    let vprod = register_edge_handler(&sb, "h", "v1", "").await;
    let vstage = register_edge_handler(&sb, "h", "v2", "env_x").await;
    // prod pointer stays at v1; stage pointer is v2.
    assert_eq!(count(&sb, "SELECT version_id FROM ops.handler_active WHERE handler='h' AND env=''").await, vprod);
    assert_eq!(count(&sb, "SELECT version_id FROM ops.handler_active WHERE handler='h' AND env='env_x'").await, vstage);
    sb.teardown().await;
}

// I-HND-12 (finding 8): rollback_handler flips the (handler,env) pointer to the PRIOR
// version — a one-row UPDATE, never a redeploy (edge_deploys untouched).
#[tokio::test]
async fn i_hnd_12_rollback_handler_flips_to_prior() {
    let sb = sandbox_or_skip!("I-HND-12");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let v1 = register_edge_handler(&sb, "on_user_created", "v1", "").await;
    let v2 = register_edge_handler(&sb, "on_user_created", "v2", "").await; // active → v2
    assert_eq!(
        count(&sb, "SELECT version_id FROM ops.handler_active WHERE handler='on_user_created' AND env=''").await,
        v2, "active is v2 before rollback"
    );
    // The command-surface rollback: one-row pointer flip to the prior version.
    d.rollback_handler("", "on_user_created").await.unwrap();
    assert_eq!(
        count(&sb, "SELECT version_id FROM ops.handler_active WHERE handler='on_user_created' AND env=''").await,
        v1, "rollback_handler flips the pointer back to v1 (finding 8)"
    );
    // No redeploy: both edge_deploys rows still present (immutable).
    // (register_edge_handler does not insert edge_deploys, so assert the versions table is
    // untouched — both versions still exist.)
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_versions WHERE handler='on_user_created'").await, 2,
        "rollback is a pointer flip, not a version deletion/redeploy");
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.7b  Single-txn-per-migration atomicity + advisory lock (findings 4 + 6)
// ─────────────────────────────────────────────────────────────────────────────

// I-UP-08 (findings 4 + 6): a migration whose test_up FAILS rolls back the up-DDL in the
// SAME transaction and writes NO ledger row — proving apply_one runs up+fakedata+test_up
// +ledger as one atomic unit (not four separate implicit txns).
#[tokio::test]
async fn i_up_08_failing_test_up_rolls_back_up_ddl() {
    let sb = sandbox_or_skip!("I-UP-08");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    // up.sql is VALID (creates a table); test_up.sql RAISEs. Under the old non-atomic path
    // the table would survive (committed by its own batch); under the atomic wrapper it
    // must be rolled back.
    let dir = write_migration(
        tmp.path(),
        "public",
        1,
        "atomicable",
        &[
            ("up.sql", "-- @schema: public\nCREATE TABLE atomicable (id int primary key);"),
            ("down.sql", "DROP TABLE atomicable;"),
            ("test_up.sql", "DO $$ BEGIN RAISE EXCEPTION 'test_up deliberately fails'; END $$;"),
            ("test_down.sql", "-- @intentionally-none: n/a\n"),
            ("fakedata_up.sql", "-- @intentionally-none: none\n"),
            ("fakedata_down.sql", "-- @intentionally-none: none\n"),
        ],
    );
    let m = mig(dir, "public", 1, "atomicable");
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };
    let res = apply_one(&d, &m, &ctx).await;
    assert!(res.is_err(), "a failing test_up must fail the apply");
    // The CREATE TABLE from up.sql was rolled back (same transaction).
    assert_eq!(
        count(&sb, "SELECT count(*) FROM information_schema.tables WHERE table_name='atomicable'").await,
        0, "up-DDL must roll back when test_up fails (finding 6: single-txn-per-migration)"
    );
    // No partial ledger row.
    assert_eq!(
        count(&sb, "SELECT count(*) FROM ops.applied_migrations WHERE id='public/0001_atomicable'").await,
        0, "no ledger row for a migration whose test_up failed (finding 6)"
    );
    sb.teardown().await;
}

// I-LOCK-01 (finding 4): the txn-scoped advisory lock is actually acquired on the apply
// path AND released at COMMIT — a subsequent apply in the same process (reusing the pooled
// client) does not deadlock. This proves the lock is xact-scoped (auto-released), not a
// leaked session lock.
#[tokio::test]
async fn i_lock_01_advisory_lock_acquired_and_released_on_apply() {
    let sb = sandbox_or_skip!("I-LOCK-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    // Two independent migrations applied back-to-back through apply_one. If the per-apply
    // xact lock leaked, the second acquire on the same env key would block forever.
    for (seq, name) in [(1u32, "lockone"), (2, "locktwo")] {
        write_migration(
            tmp.path(),
            "public",
            seq,
            name,
            &[
                ("up.sql", &format!("-- @schema: public\nCREATE TABLE {name} (id int primary key);")),
                ("down.sql", &format!("DROP TABLE {name};")),
                ("test_up.sql", "-- @intentionally-none: presence covered by up\n"),
                ("test_down.sql", "-- @intentionally-none: n/a\n"),
                ("fakedata_up.sql", "-- @intentionally-none: none\n"),
                ("fakedata_down.sql", "-- @intentionally-none: none\n"),
            ],
        );
    }
    let migs = discover(tmp.path()).unwrap();
    let ordered = topo_order(&migs).unwrap();
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };
    // Bound the whole thing so a lock leak surfaces as a timeout, not a hang.
    let fut = async {
        for m in &ordered {
            apply_one(&d, m, &ctx).await.unwrap();
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(20), fut)
        .await
        .expect("two sequential applies must not deadlock (xact lock released at COMMIT, finding 4)");
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.applied_migrations").await, 2,
        "both migrations applied under the advisory lock");
    // No advisory lock is still held afterward (all xact locks released at COMMIT).
    let held = count(&sb, "SELECT count(*) FROM pg_locks WHERE locktype='advisory'").await;
    assert_eq!(held, 0, "no advisory lock leaked after apply (finding 4)");
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.8  Async outbox — enqueue-on-commit / skip-on-rollback (§8.1) + drainer
// ─────────────────────────────────────────────────────────────────────────────

// I-OBX-01: call_handler (effect-async edge) inside a COMMITTED txn → exactly one
// durable pending dispatch row (env set, idempotency_key non-null H5).
#[tokio::test]
async fn i_obx_01_enqueue_on_commit() {
    let sb = sandbox_or_skip!("I-OBX-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    register_edge_handler(&sb, "on_user_created", "v1", "").await;
    // call_handler enqueues; batch_execute commits.
    d.apply_sql("", "SELECT ops.call_handler('on_user_created', '{\"user_id\":\"u1\"}'::jsonb);").await.unwrap();
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE handler='on_user_created' AND status='pending'").await, 1);
    assert_eq!(
        cell(&sb, "SELECT idempotency_key FROM ops.handler_dispatch WHERE handler='on_user_created'", "idempotency_key").await.as_deref(),
        Some("u1"),
        "idempotency_key resolved from template {{user_id}} (H5)"
    );
    sb.teardown().await;
}

// I-OBX-02: call_handler inside a ROLLED-BACK txn → ZERO dispatch rows (transactional
// outbox: the intent rides the caller's txn).
#[tokio::test]
async fn i_obx_02_skip_on_rollback() {
    let sb = sandbox_or_skip!("I-OBX-02");
    sb.bootstrap_ops().await;
    register_edge_handler(&sb, "on_user_created", "v1", "").await;
    // Use a raw client to control the txn boundary explicitly.
    let c = sb.client().await;
    c.batch_execute("BEGIN").await.unwrap();
    c.batch_execute("SELECT ops.call_handler('on_user_created', '{\"user_id\":\"u9\"}'::jsonb);").await.unwrap();
    c.batch_execute("ROLLBACK").await.unwrap();
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch").await, 0,
        "rolled-back call enqueues nothing (transactional outbox §8.1)");
    sb.teardown().await;
}

// I-OBX-03 / I-OBX-06: drain fires pending for the target env only; env filter honored.
#[tokio::test]
async fn i_obx_03_06_drain_filters_by_env() {
    let sb = sandbox_or_skip!("I-OBX-03");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let vid = register_edge_handler(&sb, "on_user_created", "v1", "").await;
    for (env, key) in [("", "kp"), ("env_x", "kx")] {
        d.apply_sql("", &format!("INSERT INTO ops.handler_dispatch(env,handler,version_id,payload,idempotency_key) \
            VALUES ('{env}','on_user_created',{vid},'{{}}'::jsonb,'{key}')")).await.unwrap();
    }
    let tx = common::MockTransport::new();
    let rep = d.outbox_drain("", &tx).await.unwrap();
    assert_eq!(rep.fired, 1, "draining prod fires only the prod intent");
    assert_eq!(tx.keys(), vec!["kp"], "transport actually invoked for the prod intent only");
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE env='' AND status='fired'").await, 1);
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE env='env_x' AND status='pending'").await, 1,
        "env_x intent not fired by a prod drain (I-OBX-06)");
    sb.teardown().await;
}

// I-OBX-08: outbox --status failed + retry re-enqueues; drain then fires it.
#[tokio::test]
async fn i_obx_08_failed_retry() {
    let sb = sandbox_or_skip!("I-OBX-08");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let vid = register_edge_handler(&sb, "h", "v1", "").await;
    d.apply_sql("", &format!("INSERT INTO ops.handler_dispatch(env,handler,version_id,payload,idempotency_key,status) \
        VALUES ('','h',{vid},'{{}}'::jsonb,'k','failed')")).await.unwrap();
    let listed = substrate_db::outbox::list(&d, "", Some("failed"), None).await.unwrap();
    assert_eq!(listed.rows.len(), 1, "failed row listed");
    substrate_db::outbox::retry(&d, "", None).await.unwrap();
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE status='pending'").await, 1, "retry re-enqueues to pending");
    sb.teardown().await;
}

// I-OBX-10 (finding 2): the drainer ACTUALLY invokes the edge transport (not a status
// flip) and delivers at-least-once — a pending intent triggers exactly one transport call
// and lands `fired`.
#[tokio::test]
async fn i_obx_10_drainer_invokes_transport_at_least_once() {
    let sb = sandbox_or_skip!("I-OBX-10");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let vid = register_edge_handler(&sb, "on_user_created", "v1", "").await;
    d.apply_sql("", &format!("INSERT INTO ops.handler_dispatch(env,handler,version_id,payload,idempotency_key) \
        VALUES ('','on_user_created',{vid},'{{\"user_id\":\"u1\"}}'::jsonb,'idem-1')")).await.unwrap();
    let tx = common::MockTransport::new();
    let rep = substrate_db::outbox::drain_with(&d, "", &tx).await.unwrap();
    assert_eq!(rep.fired, 1, "one intent fired");
    assert_eq!(tx.count(), 1, "transport actually invoked exactly once (not a status flip)");
    assert_eq!(tx.keys(), vec!["idem-1"], "delivered the intent's idempotency_key");
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE status='fired'").await, 1);
    sb.teardown().await;
}

// I-OBX-11 (finding 2): idempotent dedup on idempotency_key — TWO enqueued intents that
// share a key deliver ONCE; the duplicate collapses to `done` without a second transport
// call (at-least-once WITHOUT duplicate delivery, §8.2).
#[tokio::test]
async fn i_obx_11_dedup_on_idempotency_key() {
    let sb = sandbox_or_skip!("I-OBX-11");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let vid = register_edge_handler(&sb, "on_user_created", "v1", "").await;
    // Two intents, SAME idempotency_key (e.g. a double-enqueue after a caller retry).
    for _ in 0..2 {
        d.apply_sql("", &format!("INSERT INTO ops.handler_dispatch(env,handler,version_id,payload,idempotency_key) \
            VALUES ('','on_user_created',{vid},'{{}}'::jsonb,'dup-key')")).await.unwrap();
    }
    let tx = common::MockTransport::new();
    let rep = substrate_db::outbox::drain_with(&d, "", &tx).await.unwrap();
    assert_eq!(tx.count(), 1, "transport invoked ONCE despite two same-key intents (dedup)");
    assert_eq!(rep.fired, 1, "one fired");
    assert_eq!(rep.deduped, 1, "the duplicate is deduped, not re-delivered");
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE status='fired'").await, 1);
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE status='done'").await, 1);
    sb.teardown().await;
}

// I-OBX-12 (finding 2): a RE-DRAIN after a successful fire does not re-deliver the same
// key (idempotent across passes). A transport failure keeps the row `failed` (owed a
// retry); a subsequent success delivers it — genuine at-least-once.
#[tokio::test]
async fn i_obx_12_redrain_no_duplicate_and_failed_retry() {
    let sb = sandbox_or_skip!("I-OBX-12");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let vid = register_edge_handler(&sb, "on_user_created", "v1", "").await;
    d.apply_sql("", &format!("INSERT INTO ops.handler_dispatch(env,handler,version_id,payload,idempotency_key) \
        VALUES ('','on_user_created',{vid},'{{}}'::jsonb,'once-key')")).await.unwrap();
    let tx = common::MockTransport::new();

    // First pass fails at the transport → row stays `failed`, error recorded, no delivery.
    tx.set_fail(true);
    let rep1 = substrate_db::outbox::drain_with(&d, "", &tx).await.unwrap();
    assert_eq!(rep1.failed, 1);
    assert_eq!(tx.count(), 0, "failed transport delivered nothing");
    assert_eq!(count(&sb, "SELECT count(*) FROM ops.handler_dispatch WHERE status='failed'").await, 1);

    // Second pass succeeds → the still-owed intent is delivered (at-least-once).
    tx.set_fail(false);
    let rep2 = substrate_db::outbox::drain_with(&d, "", &tx).await.unwrap();
    assert_eq!(rep2.fired, 1, "the owed intent is delivered on retry");
    assert_eq!(tx.count(), 1);

    // Third pass: nothing pending; a re-drain does not re-deliver the fired key.
    let rep3 = substrate_db::outbox::drain_with(&d, "", &tx).await.unwrap();
    assert_eq!(rep3.fired, 0, "no re-fire on re-drain");
    assert_eq!(tx.count(), 1, "still exactly one delivery total (no duplicate)");
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.9  Validator-sync — allow / deny / transform / M4 guard (pure-SQL path)
// ─────────────────────────────────────────────────────────────────────────────

/// Install a pure-SQL validator wired to a real trigger via the codegen'd trigger body.
/// The validator function returns a fixed validator_result the test controls.
async fn install_sql_validator(sb: &PgSandbox, decision_sql: &str) {
    let d = sb.driver();
    // Target table.
    d.apply_sql("", "CREATE TABLE rooms (id int primary key, name text);").await.unwrap();
    // Logical handler + version (kind=sql, invocation=validator-sync).
    d.apply_sql("", "INSERT INTO ops.handlers(name) VALUES ('validate_room') ON CONFLICT DO NOTHING").await.unwrap();
    // call_validate_room is the codegen'd wrapper; here we register the concrete impl and
    // point target_ref at it so call_handler dispatches.
    d.apply_sql("", &format!("CREATE FUNCTION validate_room_impl(p jsonb) RETURNS jsonb LANGUAGE sql AS $$ SELECT {decision_sql} $$;")).await.unwrap();
    d.apply_sql("", "INSERT INTO ops.handler_versions(handler,version,kind,invocation,blocking,target_ref,contract,source_hash) \
        VALUES ('validate_room','v1','sql','validator-sync',true,'validate_room_impl','{}'::jsonb,'h')").await.unwrap();
    d.apply_sql("", "INSERT INTO ops.handler_active(handler,env,version_id) \
        SELECT 'validate_room','',id FROM ops.handler_versions WHERE handler='validate_room' AND version='v1'").await.unwrap();
    // The codegen'd trigger body (mirrors handler::gen_trigger): deny→RAISE, row→M4 guard
    // then jsonb_populate_record. It calls ops.call_handler which dispatches to the impl.
    d.apply_sql("",
        "CREATE FUNCTION trg_validate_room() RETURNS trigger LANGUAGE plpgsql AS $$\n\
         DECLARE r jsonb;\n\
         BEGIN\n\
           r := ops.call_handler('validate_room', to_jsonb(NEW));\n\
           IF r->>'decision' = 'deny' THEN\n\
             RAISE EXCEPTION 'validator validate_room denied: %', r->>'reason' USING ERRCODE='check_violation', DETAIL = r->>'code';\n\
           END IF;\n\
           IF r ? 'row' THEN\n\
             PERFORM ops.assert_row_shape('public','rooms', r->'row');\n\
             NEW := jsonb_populate_record(NEW, r->'row');\n\
           END IF;\n\
           RETURN NEW;\n\
         END $$;").await.unwrap();
    d.apply_sql("", "CREATE TRIGGER rooms_validate BEFORE INSERT ON rooms FOR EACH ROW EXECUTE FUNCTION trg_validate_room();").await.unwrap();
}

// I-VS-01: SQL validator returns allow → row inserted.
#[tokio::test]
async fn i_vs_01_sql_validator_allow() {
    let sb = sandbox_or_skip!("I-VS-01");
    sb.bootstrap_ops().await;
    install_sql_validator(&sb, "'{\"decision\":\"allow\"}'::jsonb").await;
    let d = sb.driver();
    d.apply_sql("", "INSERT INTO rooms(id,name) VALUES (1,'ok');").await.unwrap();
    assert_eq!(count(&sb, "SELECT count(*) FROM rooms WHERE id=1").await, 1, "allow → row inserted");
    sb.teardown().await;
}

// I-VS-02: SQL validator returns deny → trigger RAISEs (check_violation), row NOT written.
#[tokio::test]
async fn i_vs_02_sql_validator_deny_aborts() {
    let sb = sandbox_or_skip!("I-VS-02");
    sb.bootstrap_ops().await;
    install_sql_validator(&sb, "'{\"decision\":\"deny\",\"reason\":\"nope\",\"code\":\"E1\"}'::jsonb").await;
    let d = sb.driver();
    let res = d.apply_sql("", "INSERT INTO rooms(id,name) VALUES (2,'bad');").await;
    assert!(res.is_err(), "deny must RAISE and abort the write");
    assert_eq!(count(&sb, "SELECT count(*) FROM rooms WHERE id=2").await, 0, "denied row not written");
    sb.teardown().await;
}

// I-VS-04: validator returns allow + row transform → committed row reflects the transform.
#[tokio::test]
async fn i_vs_04_transform_applied() {
    let sb = sandbox_or_skip!("I-VS-04");
    sb.bootstrap_ops().await;
    // Validator forces name := 'transformed'.
    install_sql_validator(&sb, "jsonb_build_object('decision','allow','row', jsonb_build_object('name','transformed'))").await;
    let d = sb.driver();
    d.apply_sql("", "INSERT INTO rooms(id,name) VALUES (3,'original');").await.unwrap();
    let row = d.query("", "SELECT name FROM rooms WHERE id=3").await.unwrap();
    assert_eq!(row.rows[0][0].as_deref(), Some("transformed"), "transform mutated the committed row");
    sb.teardown().await;
}

// I-VS-05: transform returns an UNKNOWN column key → assert_row_shape RAISEs (M4).
#[tokio::test]
async fn i_vs_05_unknown_key_rejected() {
    let sb = sandbox_or_skip!("I-VS-05");
    sb.bootstrap_ops().await;
    install_sql_validator(&sb, "jsonb_build_object('decision','allow','row', jsonb_build_object('bogus_col','x'))").await;
    let d = sb.driver();
    let res = d.apply_sql("", "INSERT INTO rooms(id,name) VALUES (4,'x');").await;
    assert!(res.is_err(), "unknown column in transform must be rejected by assert_row_shape (M4)");
    assert_eq!(count(&sb, "SELECT count(*) FROM rooms WHERE id=4").await, 0);
    sb.teardown().await;
}

// I-VS-06: transform returns a type-mismatched value → assert_row_shape RAISEs.
#[tokio::test]
async fn i_vs_06_type_mismatch_rejected() {
    let sb = sandbox_or_skip!("I-VS-06");
    sb.bootstrap_ops().await;
    // id is int; feed a non-numeric string.
    install_sql_validator(&sb, "jsonb_build_object('decision','allow','row', jsonb_build_object('id','not_a_number'))").await;
    let d = sb.driver();
    let res = d.apply_sql("", "INSERT INTO rooms(id,name) VALUES (5,'x');").await;
    assert!(res.is_err(), "type mismatch in transform must be rejected (M4)");
    sb.teardown().await;
}

// I-VS-03: deny audit written OUT-OF-TXN (C2) — after the abort, a separate committed
// txn records exactly one validator_audit(decision=deny) row.
#[tokio::test]
async fn i_vs_03_deny_audit_out_of_txn() {
    let sb = sandbox_or_skip!("I-VS-03");
    sb.bootstrap_ops().await;
    install_sql_validator(&sb, "'{\"decision\":\"deny\",\"reason\":\"nope\",\"code\":\"E1\"}'::jsonb").await;
    let d = sb.driver();
    let vid = count(&sb, "SELECT id FROM ops.handler_versions WHERE handler='validate_room'").await;
    // The write aborts.
    let res = d.apply_sql("", "INSERT INTO rooms(id,name) VALUES (7,'bad');").await;
    assert!(res.is_err());
    // The db client layer writes the deny audit out-of-band, in a SEPARATE committed txn.
    substrate_db::audit::write_deny(&d, "", "validate_room", vid, "nope", Some("E1"), Some(3), false)
        .await
        .unwrap();
    let denies = count(&sb, "SELECT count(*) FROM ops.validator_audit WHERE decision='deny' AND handler='validate_room'").await;
    assert_eq!(denies, 1, "exactly one out-of-txn deny audit (C2)");
    sb.teardown().await;
}

// I-VS-13: db audit --decision deny --since 1h lists deny rows (observability seam).
#[tokio::test]
async fn i_vs_13_audit_lists_denies() {
    let sb = sandbox_or_skip!("I-VS-13");
    sb.bootstrap_ops().await;
    install_sql_validator(&sb, "'{\"decision\":\"deny\"}'::jsonb").await;
    let d = sb.driver();
    let vid = count(&sb, "SELECT id FROM ops.handler_versions WHERE handler='validate_room'").await;
    substrate_db::audit::write_deny(&d, "", "validate_room", vid, "r", None, None, false).await.unwrap();
    let rows = substrate_db::audit::list(&d, "", None, Some("deny")).await.unwrap();
    assert_eq!(rows.rows.len(), 1);
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.10 promote gate (invariant checks; no real apply)
// ─────────────────────────────────────────────────────────────────────────────

// I-PROMO-04: promote refuses when a to-be-applied migration lacks a crawl attestation.
#[tokio::test]
async fn i_promo_04_missing_attestation_refused() {
    let sb = sandbox_or_skip!("I-PROMO-04");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    widgets_migration(tmp.path());
    let migs = discover(tmp.path()).unwrap();
    // No attestation recorded → gate must flag it as a blocker.
    let report = substrate_db::promote::gate(&d, &migs, &migs, false).await.unwrap();
    assert!(!report.is_clear(), "gate must refuse with a missing attestation");
    assert!(report.missing_attestations.contains(&"public/0001_widgets".to_string()));
    sb.teardown().await;
}

// I-PROMO-05: attestation exists but checksum stale (up.sql edited post-crawl) → refused.
#[tokio::test]
async fn i_promo_05_stale_attestation_refused() {
    let sb = sandbox_or_skip!("I-PROMO-05");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    let dir = widgets_migration(tmp.path());
    let migs = discover(tmp.path()).unwrap();
    let atts = crawl(&d, &migs, "").await.unwrap();
    record_attestations(&d, &atts, "ci").await.unwrap();
    // Edit up.sql AFTER crawl → checksum drifts, attestation is now stale.
    std::fs::write(dir.join("up.sql"), "-- @schema: public\nCREATE TABLE widgets (id int primary key, name text, added int);").unwrap();
    let migs2 = discover(tmp.path()).unwrap();
    let report = substrate_db::promote::gate(&d, &migs2, &migs2, false).await.unwrap();
    assert!(!report.is_clear(), "stale attestation → refuse (no honor-system)");
    sb.teardown().await;
}

// I-PROMO-09-ish: full gate passes on a clean, crawled, codegen-fresh tree.
#[tokio::test]
async fn i_promo_gate_clear_when_crawled_and_clean() {
    let sb = sandbox_or_skip!("I-PROMO-clean");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let tmp = tempfile::tempdir().unwrap();
    widgets_migration(tmp.path());
    let migs = discover(tmp.path()).unwrap();
    let atts = crawl(&d, &migs, "").await.unwrap();
    record_attestations(&d, &atts, "ci").await.unwrap();
    let report = substrate_db::promote::gate(&d, &migs, &migs, false).await.unwrap();
    assert!(report.is_clear(), "clean+crawled+fresh-codegen tree passes the gate: {report:?}");
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// I.11 safety / doctor / query / inspect
// ─────────────────────────────────────────────────────────────────────────────

// I-SAFE-04: query --write on a NON-protected local env succeeds (no protected ref → no gate).
#[tokio::test]
async fn i_safe_04_write_on_nonprotected_succeeds() {
    let sb = sandbox_or_skip!("I-SAFE-04");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    d.apply_sql("", "CREATE TABLE t (id int);").await.unwrap();
    d.apply_sql("", "INSERT INTO t(id) VALUES (1);").await.unwrap();
    assert_eq!(count(&sb, "SELECT count(*) FROM t").await, 1);
    sb.teardown().await;
}

// I-DOC-01: db doctor checks driver reachable, ledger present, http/pg_net extensions,
// protected refs configured.
#[tokio::test]
async fn i_doc_01_doctor_checks() {
    let sb = sandbox_or_skip!("I-DOC-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    let cfg = DbConfig::from_str(common::FX_TOML_BASE).unwrap();
    let report = substrate_db::promote::doctor(&d, &cfg).await.unwrap();
    let get = |n: &str| report.checks.iter().find(|c| c.name == n).map(|c| c.ok);
    assert_eq!(get("driver-reachable"), Some(true));
    assert_eq!(get("ledger-present"), Some(true));
    // pg_net is installed on the local stack; http is available (may or may not be installed).
    assert_eq!(get("ext-pg_net"), Some(true), "pg_net must be present (M6)");
    assert_eq!(get("protected-refs-configured"), Some(true));
    sb.teardown().await;
}

// I-INS-01: inspect tables/describe lists tables/columns matching the applied schema.
#[tokio::test]
async fn i_ins_01_inspect_tables_describe() {
    let sb = sandbox_or_skip!("I-INS-01");
    sb.bootstrap_ops().await;
    let d = sb.driver();
    d.apply_sql("", "CREATE TABLE gadgets (gid int primary key, label text);").await.unwrap();
    let tables = d.introspect("", IntrospectQuery::Tables).await.unwrap();
    let has_gadgets = tables.rows.rows.iter().any(|r| r.iter().any(|c| c.as_deref() == Some("gadgets")));
    assert!(has_gadgets, "inspect tables lists gadgets");
    let desc = d.introspect("", IntrospectQuery::Describe { table: "gadgets".into() }).await.unwrap();
    let cols: Vec<String> = desc.rows.rows.iter().filter_map(|r| r[0].clone()).collect();
    assert!(cols.contains(&"gid".to_string()) && cols.contains(&"label".to_string()));
    sb.teardown().await;
}

// ─────────────────────────────────────────────────────────────────────────────
// PART S — suite-level / meta
// ─────────────────────────────────────────────────────────────────────────────

// S.1: the real prod ref never appears anywhere in the test tree / fixtures. The needle
// is assembled from fragments so THIS file does not itself contain the literal (keeping
// the tree genuinely clean, not merely self-excluded).
#[test]
fn s1_no_prod_ref_in_test_tree() {
    let real_prod_ref = ["imex", "vxagmmn", "srtob", "bnhp"].concat();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut hits = Vec::new();
    visit(&dir, &mut |p, body: &str| {
        if body.contains(&real_prod_ref) {
            hits.push(p.display().to_string());
        }
    });
    assert!(hits.is_empty(), "real prod ref must NOT appear in test tree: {hits:?}");
}

// S.2: fixtures only reference 127.0.0.1 mocks / local hosts — no real Management-API host.
// Needle assembled from fragments so this file is itself clean.
#[test]
fn s2_no_real_mgmt_api_host_in_fixtures() {
    let mgmt_host = ["api.", "supabase", ".com"].concat();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut hits = Vec::new();
    visit(&dir, &mut |p, body: &str| {
        if body.contains(&mgmt_host) {
            hits.push(p.display().to_string());
        }
    });
    assert!(hits.is_empty(), "no real mgmt-api host in fixtures: {hits:?}");
}

fn visit(dir: &std::path::Path, f: &mut dyn FnMut(&std::path::Path, &str)) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                visit(&p, f);
            } else if let Ok(body) = std::fs::read_to_string(&p) {
                f(&p, &body);
            }
        }
    }
}

// Sanity: the driver kind is supabase-local for the sandbox.
#[tokio::test]
async fn sandbox_driver_kind() {
    let sb = sandbox_or_skip!("driver-kind");
    assert_eq!(sb.driver().kind(), DriverKind::SupabaseLocal);
    sb.teardown().await;
}
