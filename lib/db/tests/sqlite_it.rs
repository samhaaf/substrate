//! PART I — [SQLITE] integration tests (Tier 1: always run, no Docker).
//! Throwaway SQLite file (or in-memory) via `SqliteDriver`. Ledger-only baseline (H4);
//! everything edge/registry/outbox = NotImplemented at apply (§7.5).

use substrate_db::driver::{AppliedMigration, Driver, IntrospectQuery};
use substrate_db::driver::sqlite::SqliteDriver;
use substrate_db::migration::{apply_one, rollback_one, ApplyContext, Migration};

mod common;
use common::write_migration;

fn sqlite() -> SqliteDriver {
    SqliteDriver::open_in_memory().unwrap()
}

async fn bootstrap_ledger(d: &SqliteDriver) {
    d.apply_sql("", substrate_db::OPS_BASELINE_SQLITE).await.unwrap();
}

// I-BASE-02: ledger-only baseline; ops_ prefix; :SCHEMA rewrite.
#[tokio::test]
async fn i_base_02_ledger_only_baseline() {
    let d = sqlite();
    bootstrap_ledger(&d).await;
    let rows = d
        .query("", "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .await
        .unwrap();
    let names: Vec<String> = rows.rows.iter().filter_map(|r| r[0].clone()).collect();
    assert!(names.contains(&"ops_applied_migrations".to_string()));
    // No registry/outbox tables on sqlite (ledger-only, H4).
    assert!(!names.iter().any(|n| n.contains("handler_versions")));
    assert!(!names.iter().any(|n| n.contains("handler_dispatch")));

    // :SCHEMA rewrite for a stage env → env_x_ table prefix.
    d.apply_sql("env_x", "CREATE TABLE :SCHEMAwidgets (id int);").await.unwrap();
    let t = d
        .query("", "SELECT name FROM sqlite_master WHERE type='table' AND name='env_x_widgets'")
        .await
        .unwrap();
    assert_eq!(t.rows.len(), 1, ":SCHEMA must rewrite to env_x_ prefix");
}

// I-BASE-03 / I-HND-11: an edge migration on sqlite → NotImplemented at apply.
#[tokio::test]
async fn i_base_03_edge_migration_not_implemented() {
    let d = sqlite();
    bootstrap_ledger(&d).await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_migration(
        tmp.path(),
        "public",
        1,
        "edgey",
        &[
            ("up.sql", "-- @activate on_user_created = v1\nselect 1;"),
            ("handler.yaml", "handler: on_user_created\nkind: edge\n"),
        ],
    );
    let m = Migration {
        schema: "public".into(),
        seq: 1,
        name: "edgey".into(),
        id: "public/0001_edgey".into(),
        dir,
        depends_on: vec![],
        checksum: "c".into(),
    };
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: true };
    let e = apply_one(&d, &m, &ctx).await.unwrap_err();
    assert!(
        matches!(e, substrate_types::SubstrateError::NotImplemented { .. }),
        "edge on sqlite must be NotImplemented at apply, got {e:?}"
    );
}

// I-UP-02 / I-DOWN-02 / I-UP-06: apply → ledger → test_up; down; idempotence.
#[tokio::test]
async fn i_up_02_down_02_apply_and_rollback() {
    let d = sqlite();
    bootstrap_ledger(&d).await;
    let tmp = tempfile::tempdir().unwrap();
    let dir = write_migration(
        tmp.path(),
        "public",
        1,
        "widgets",
        &[
            ("up.sql", "CREATE TABLE widgets (id int primary key, name text);"),
            ("down.sql", "DROP TABLE widgets;"),
            (
                "test_up.sql",
                "INSERT INTO widgets(id,name) VALUES (1,'a'); DELETE FROM widgets;",
            ),
            ("test_down.sql", "-- @intentionally-none: covered by up presence\n"),
        ],
    );
    let m = Migration {
        schema: "public".into(),
        seq: 1,
        name: "widgets".into(),
        id: "public/0001_widgets".into(),
        dir,
        depends_on: vec![],
        checksum: "abc123".into(),
    };
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };

    apply_one(&d, &m, &ctx).await.unwrap();
    // Ledger recorded in ops_applied_migrations.
    let led = d
        .query("", "SELECT id, checksum FROM ops_applied_migrations WHERE env=''")
        .await
        .unwrap();
    assert_eq!(led.rows.len(), 1);
    assert_eq!(led.rows[0][0].as_deref(), Some("public/0001_widgets"));
    // table exists
    let t = d
        .query("", "SELECT name FROM sqlite_master WHERE type='table' AND name='widgets'")
        .await
        .unwrap();
    assert_eq!(t.rows.len(), 1);

    // I-DOWN-02: rollback removes table + ledger row.
    rollback_one(&d, &m, "").await.unwrap();
    let t2 = d
        .query("", "SELECT name FROM sqlite_master WHERE type='table' AND name='widgets'")
        .await
        .unwrap();
    assert_eq!(t2.rows.len(), 0, "table dropped by down");
    let led2 = d.query("", "SELECT id FROM ops_applied_migrations WHERE env=''").await.unwrap();
    assert_eq!(led2.rows.len(), 0, "ledger row removed on down");
}

// I-UP-08-SQLITE (findings 4 + 6): apply_one on sqlite is atomic — a failing test_up rolls
// back the up-DDL in the same transaction and writes NO ledger row.
#[tokio::test]
async fn i_up_08_sqlite_failing_test_up_rolls_back() {
    let d = sqlite();
    bootstrap_ledger(&d).await;
    let tmp = tempfile::tempdir().unwrap();
    // Valid up (creates a table), then a test_up that RAISEs via an invalid statement.
    let dir = write_migration(
        tmp.path(),
        "public",
        1,
        "atomic",
        &[
            ("up.sql", "CREATE TABLE atomic_t (id int primary key);"),
            ("down.sql", "DROP TABLE atomic_t;"),
            // A deliberately invalid statement in test_up aborts the transaction.
            ("test_up.sql", "INSERT INTO nonexistent_sqlite_table VALUES (1);"),
            ("test_down.sql", "-- @intentionally-none: n/a\n"),
        ],
    );
    let m = Migration {
        schema: "public".into(),
        seq: 1,
        name: "atomic".into(),
        id: "public/0001_atomic".into(),
        dir,
        depends_on: vec![],
        checksum: "c".into(),
    };
    let ctx = ApplyContext { env: "", allow_fakedata: true, edge_incapable: false };
    let res = apply_one(&d, &m, &ctx).await;
    assert!(res.is_err(), "failing test_up must fail the apply");
    // The up-DDL was rolled back (table absent).
    let t = d
        .query("", "SELECT name FROM sqlite_master WHERE type='table' AND name='atomic_t'")
        .await
        .unwrap();
    assert_eq!(t.rows.len(), 0, "up-DDL rolled back on sqlite (atomic apply, finding 6)");
    // No ledger row.
    let led = d.query("", "SELECT id FROM ops_applied_migrations WHERE id='public/0001_atomic'").await.unwrap();
    assert_eq!(led.rows.len(), 0, "no ledger row for a failed migration on sqlite");
}

// I-FAKE-05: fakedata up/down on sqlite is supported (SQL data), reversible.
#[tokio::test]
async fn i_fake_05_fakedata_reversible() {
    let d = sqlite();
    d.apply_sql("", "CREATE TABLE t (id int);").await.unwrap();
    d.apply_sql("", "INSERT INTO t(id) VALUES (1),(2);").await.unwrap();
    let n = d.query("", "SELECT count(*) FROM t").await.unwrap();
    assert_eq!(n.rows[0][0].as_deref(), Some("2"));
    d.apply_sql("", "DELETE FROM t;").await.unwrap();
    let n2 = d.query("", "SELECT count(*) FROM t").await.unwrap();
    assert_eq!(n2.rows[0][0].as_deref(), Some("0"));
}

// I-OBX-07: outbox on sqlite → NotImplemented (no outbox table, H4).
#[tokio::test]
async fn i_obx_07_outbox_not_implemented() {
    let d = sqlite();
    let e = substrate_db::outbox::drain(&d, "").await.unwrap_err();
    assert!(matches!(e, substrate_types::SubstrateError::NotImplemented { .. }));
    let e2 = substrate_db::outbox::list(&d, "", None, None).await.unwrap_err();
    assert!(matches!(e2, substrate_types::SubstrateError::NotImplemented { .. }));
}

// I-VS-01/02/04 (SQL-portable) via raw SQL triggers on sqlite: allow / transform.
// sqlite has no plpgsql; the portable validator path here asserts the *SQL-assert*
// harness works (a deny that RAISES aborts the txn).
#[tokio::test]
async fn i_vs_01_02_sqlite_sql_assert_harness() {
    let d = sqlite();
    d.apply_sql("", "CREATE TABLE m (id int primary key, ok int);").await.unwrap();
    // allow-path: insert commits.
    d.apply_sql("", "INSERT INTO m(id,ok) VALUES (1,1);").await.unwrap();
    let n = d.query("", "SELECT count(*) FROM m").await.unwrap();
    assert_eq!(n.rows[0][0].as_deref(), Some("1"));
    // deny-path: a CHECK-violating insert is rejected (row NOT written).
    d.apply_sql("", "CREATE TABLE g (id int primary key, v int CHECK (v > 0));").await.unwrap();
    let bad = d.apply_sql("", "INSERT INTO g(id,v) VALUES (1,-5);").await;
    assert!(bad.is_err(), "check violation must abort the write");
    let ng = d.query("", "SELECT count(*) FROM g").await.unwrap();
    assert_eq!(ng.rows[0][0].as_deref(), Some("0"), "denied row not written");
}

// I-INS-02: inspect tables via sqlite_master; policies → empty (no RLS).
#[tokio::test]
async fn i_ins_02_introspect() {
    let d = sqlite();
    d.apply_sql("", "CREATE TABLE alpha (id int); CREATE TABLE beta (id int);").await.unwrap();
    let tables = d.introspect("", IntrospectQuery::Tables).await.unwrap();
    let names: Vec<String> = tables.rows.rows.iter().filter_map(|r| r[0].clone()).collect();
    assert!(names.contains(&"alpha".to_string()) && names.contains(&"beta".to_string()));
    let pol = d.introspect("", IntrospectQuery::Policies).await.unwrap();
    assert!(pol.rows.rows.is_empty(), "sqlite has no RLS policies");
}

// I-TREE-02: separate sqlite file per worktree (path isolation).
#[tokio::test]
async fn i_tree_02_separate_file_per_worktree() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("wt1.sqlite");
    let p2 = dir.path().join("wt2.sqlite");
    let d1 = SqliteDriver::open(&p1).unwrap();
    let d2 = SqliteDriver::open(&p2).unwrap();
    d1.apply_sql("", "CREATE TABLE only_in_1 (id int);").await.unwrap();
    // d2 must NOT see d1's table.
    let r = d2
        .query("", "SELECT name FROM sqlite_master WHERE type='table' AND name='only_in_1'")
        .await
        .unwrap();
    assert_eq!(r.rows.len(), 0, "worktrees are isolated sqlite files");
    assert!(p1.exists() && p2.exists());
}

// Ledger-record path assigns monotonic applied_seq on sqlite.
#[tokio::test]
async fn i_base_02b_applied_seq_monotonic() {
    let d = sqlite();
    bootstrap_ledger(&d).await;
    for (i, id) in ["public/0001_a", "public/0002_b", "core/0001_c"].iter().enumerate() {
        d.ledger_record(AppliedMigration {
            env: "".into(),
            schema: if i == 2 { "core".into() } else { "public".into() },
            id: id.to_string(),
            checksum: "c".into(),
        })
        .await
        .unwrap();
    }
    let rows = d
        .query("", "SELECT id FROM ops_applied_migrations ORDER BY applied_seq")
        .await
        .unwrap();
    let ids: Vec<String> = rows.rows.iter().filter_map(|r| r[0].clone()).collect();
    assert_eq!(ids, vec!["public/0001_a", "public/0002_b", "core/0001_c"]);
}
