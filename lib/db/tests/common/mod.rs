//! Shared test harness (test plan §0). Provides:
//! - Tier gating for [LOCAL] (Docker/local-Postgres) and [CLOUD-MOCK].
//! - A throwaway-database factory so every [LOCAL] test is fully isolated and
//!   re-runnable (test plan §0.3, S.3): each test gets its own `db_test_<uuid>`
//!   database, so the crate's literal `ops.` schema is fresh per test and NOTHING
//!   touches the real broomstick-platform data on the shared stack.
//! - Fixture builders (FX-TOML-BASE, FX-MIG-TREE, handler contracts) built into a
//!   tempdir so no prod dir is written (test plan Part F).
//!
//! SAFETY: the throwaway DBs are created on the LOCAL Supabase Postgres (127.0.0.1:54322).
//! The real prod ref NEVER appears anywhere in this tree (asserted by S.1 in local_it.rs);
//! test fixtures use only the FAKE `TEST_ONLY_ref_never_real` protected ref.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use substrate_db::driver::supabase_local::SupabaseLocalDriver;
use substrate_db::outbox::EdgeTransport;
use substrate_types::{Result, SubstrateError};

/// A test-double edge transport (finding 2). Records every delivery so a test can assert
/// exactly-once semantics, and can be flipped to fail so failed-retry paths are provable.
#[derive(Clone, Default)]
pub struct MockTransport {
    /// Every (handler, idempotency_key) actually delivered, in order.
    pub delivered: Arc<Mutex<Vec<(String, String)>>>,
    /// When true, every delivery returns an error (row stays `failed`).
    pub fail: Arc<Mutex<bool>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }
    /// Keys delivered so far (deduped-away rows never appear here).
    pub fn keys(&self) -> Vec<String> {
        self.delivered.lock().unwrap().iter().map(|(_, k)| k.clone()).collect()
    }
    pub fn count(&self) -> usize {
        self.delivered.lock().unwrap().len()
    }
    pub fn set_fail(&self, fail: bool) {
        *self.fail.lock().unwrap() = fail;
    }
}

#[async_trait]
impl EdgeTransport for MockTransport {
    async fn deliver(
        &self,
        _env: &str,
        handler: &str,
        idempotency_key: &str,
        _payload: &str,
    ) -> Result<()> {
        if *self.fail.lock().unwrap() {
            return Err(SubstrateError::Db("mock transport forced failure".into()));
        }
        self.delivered
            .lock()
            .unwrap()
            .push((handler.to_string(), idempotency_key.to_string()));
        Ok(())
    }
}

/// Admin connection string to the LOCAL Supabase Postgres (the `postgres` maintenance
/// DB), used only to CREATE/DROP the throwaway per-test databases.
pub const ADMIN_CONN: &str = "postgresql://postgres:postgres@127.0.0.1:54322/postgres";
pub const PG_HOST: &str = "127.0.0.1";
pub const PG_PORT: u16 = 54322;

/// Whether the [LOCAL] tier should run. Requires the env flag AND a reachable local
/// Postgres. When it returns false, [LOCAL] tests skip-with-reason (never false-red).
pub fn local_enabled() -> bool {
    // Opt-in flag (test plan §0.2). Default ON here because the environment provides a
    // reachable stack; set DB_IT_LOCAL=0 to force-skip.
    if std::env::var("DB_IT_LOCAL").as_deref() == Ok("0") {
        return false;
    }
    can_reach_local_pg()
}

/// Cheap TCP reachability probe for the local Postgres port.
pub fn can_reach_local_pg() -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    TcpStream::connect_timeout(
        &format!("{PG_HOST}:{PG_PORT}").parse().unwrap(),
        Duration::from_millis(500),
    )
    .is_ok()
}

/// Skip macro: prints a clear reason and returns from the test (green-skip, S.4).
#[macro_export]
macro_rules! skip_if {
    ($cond:expr, $reason:expr) => {
        if $cond {
            eprintln!("SKIP: {}", $reason);
            return;
        }
    };
}

/// A throwaway Postgres database for one [LOCAL] test. Created on construction, dropped
/// on `Drop` (teardown / reap — test plan §0.3, S.3). Fully isolated: fresh `ops`.
pub struct PgSandbox {
    pub dbname: String,
    pub conn_str: String,
    admin: tokio_postgres::Client,
}

impl PgSandbox {
    /// Create a fresh throwaway database and return a sandbox handle.
    pub async fn create() -> Self {
        let dbname = format!("db_test_{}", uuid::Uuid::new_v4().simple());
        let (admin, connection) = tokio_postgres::connect(ADMIN_CONN, tokio_postgres::NoTls)
            .await
            .expect("connect admin");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        admin
            .batch_execute(&format!("CREATE DATABASE {dbname}"))
            .await
            .expect("create throwaway db");
        let conn_str = format!("postgresql://postgres:postgres@127.0.0.1:54322/{dbname}");
        Self {
            dbname,
            conn_str,
            admin,
        }
    }

    /// A `SupabaseLocalDriver` pointed at this sandbox's database.
    pub fn driver(&self) -> SupabaseLocalDriver {
        SupabaseLocalDriver::new(".", Some(self.conn_str.clone()))
    }

    /// Apply the Postgres `ops` baseline (all control-plane tables + call_handler +
    /// assert_row_shape) into this sandbox. The single bring-up used by every [LOCAL]
    /// integration test.
    pub async fn bootstrap_ops(&self) {
        let d = self.driver();
        use substrate_db::driver::Driver;
        // Mirror the real local Supabase stack: the http + pg_net extensions the M6
        // doctor check requires are present. A fresh throwaway DB starts without them,
        // so we install them here (they exist on the shared cluster, best-effort).
        let _ = d
            .apply_sql(
                "",
                "CREATE EXTENSION IF NOT EXISTS pg_net; CREATE EXTENSION IF NOT EXISTS http;",
            )
            .await;
        d.apply_sql("", substrate_db::OPS_BASELINE_PG)
            .await
            .expect("apply ops baseline");
    }

    /// A raw client into the sandbox DB for assertions.
    pub async fn client(&self) -> tokio_postgres::Client {
        let (c, connection) = tokio_postgres::connect(&self.conn_str, tokio_postgres::NoTls)
            .await
            .expect("connect sandbox");
        tokio::spawn(async move {
            let _ = connection.await;
        });
        c
    }

    /// Drop the throwaway DB now (idempotent). Also called on Drop.
    pub async fn teardown(&self) {
        // Terminate any lingering backends then drop.
        let _ = self
            .admin
            .batch_execute(&format!(
                "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                 WHERE datname = '{}' AND pid <> pg_backend_pid();",
                self.dbname
            ))
            .await;
        let _ = self
            .admin
            .batch_execute(&format!("DROP DATABASE IF EXISTS {} (FORCE)", self.dbname))
            .await;
    }
}

impl Drop for PgSandbox {
    fn drop(&mut self) {
        // Best-effort synchronous drop so no orphan DB survives the suite (S.3).
        let dbname = self.dbname.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                if let Ok((admin, connection)) =
                    tokio_postgres::connect(ADMIN_CONN, tokio_postgres::NoTls).await
                {
                    tokio::spawn(async move {
                        let _ = connection.await;
                    });
                    let _ = admin
                        .batch_execute(&format!(
                            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
                             WHERE datname = '{dbname}' AND pid <> pg_backend_pid();"
                        ))
                        .await;
                    let _ = admin
                        .batch_execute(&format!("DROP DATABASE IF EXISTS {dbname} (FORCE)"))
                        .await;
                }
            });
        })
        .join()
        .ok();
    }
}

// ── Fixtures ────────────────────────────────────────────────────────────────

/// FX-TOML-BASE — a minimal valid `db.toml` string. Uses a FAKE protected ref; the
/// real prod ref never appears (S.1).
pub const FX_TOML_BASE: &str = r#"
default_env = "local"

[dirs]
migrations = "db/migrations"

[schema_map]
public = { rls = true }
core   = { rls = false }

[safety]
protected_refs = ["TEST_ONLY_ref_never_real"]
confirm_phrase = "promote prod"

[env.local]
driver = "supabase-local"
project_dir = "."

[env.sqlite]
driver = "sqlite"
path = ".db/test.sqlite"

[env.prod]
driver = "supabase-cloud"
project_ref = "TEST_ONLY_ref_never_real"
mgmt_api = true
"#;

/// Write a migration directory with the given files under `root/<schema>/<seq>_<name>`.
pub fn write_migration(
    migrations_dir: &Path,
    schema: &str,
    seq: u32,
    name: &str,
    files: &[(&str, &str)],
) -> PathBuf {
    let dir = migrations_dir
        .join(schema)
        .join(format!("{seq:04}_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    for (fname, body) in files {
        std::fs::write(dir.join(fname), body).unwrap();
    }
    dir
}

/// A "good" six-file, self-provisioning SQL migration (FX-MIG-TREE `good/`).
pub fn good_files(table: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "up.sql",
            format!("-- @schema: public\nCREATE TABLE {table} (id int primary key, name text);\n"),
        ),
        ("down.sql", format!("DROP TABLE {table};\n")),
        (
            "test_up.sql",
            // self-provisioning: inserts its own row then asserts.
            format!(
                "INSERT INTO {table}(id,name) VALUES (1,'a');\n\
                 DO $$ BEGIN IF (SELECT count(*) FROM {table})=0 THEN RAISE EXCEPTION 'no rows'; END IF; END $$;\n\
                 DELETE FROM {table} WHERE id=1;\n"
            ),
        ),
        (
            "test_down.sql",
            format!(
                "DO $$ BEGIN IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name='{table}') \
                 THEN RAISE EXCEPTION 'table still present'; END IF; END $$;\n"
            ),
        ),
        (
            "fakedata_up.sql",
            "-- @intentionally-none: self-provisioning test needs no fakedata\n".to_string(),
        ),
        (
            "fakedata_down.sql",
            "-- @intentionally-none: none\n".to_string(),
        ),
    ]
}
