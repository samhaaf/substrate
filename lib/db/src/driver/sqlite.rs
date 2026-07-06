//! `SqliteDriver` — introspect / query / SQL-migration only (design §3.4, §10.2).
//!
//! Backed by `rusqlite` (already a workspace dep). No schemas in sqlite, so the
//! `:SCHEMA` placeholder binds to a **table-name prefix** and `ops.` becomes the
//! `ops_` prefix. Everything edge / registry / outbox raises the trait's default
//! `NotImplemented`. Access is serialized through an `Arc<Mutex<Connection>>`, the
//! same pattern `substrate-store` uses.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rusqlite::Connection;
use substrate_types::{Result, SubstrateError};

use super::{
    AppliedMigration, Capabilities, Driver, Introspection, IntrospectQuery, LockGuard, Rows,
};
use crate::config::DriverKind;

/// SQLite driver. `:SCHEMA` → table-name prefix; `ops.` → `ops_`.
#[derive(Clone)]
pub struct SqliteDriver {
    conn: Arc<Mutex<Connection>>,
    path: PathBuf,
}

fn sqlite_err(e: rusqlite::Error) -> SubstrateError {
    SubstrateError::Db(format!("sqlite: {e}"))
}

impl SqliteDriver {
    /// Open (creating parent dirs as needed) a sqlite database at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    SubstrateError::Db(format!("creating sqlite dir {}: {e}", parent.display()))
                })?;
            }
        }
        let conn = Connection::open(&path).map_err(sqlite_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    /// In-memory database — the test/throwaway ctor (mirrors `Store::open_in_memory`).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(sqlite_err)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
        })
    }

    /// Rewrite `:SCHEMA`/`ops.` for the sqlite table-name-prefix dialect (design §3.4).
    /// A stage env `env_<slug>` binds `:SCHEMA` to `env_<slug>_`; the bare/prod env
    /// binds it to the empty prefix.
    fn rewrite(env: &str, sql: &str) -> String {
        let prefix = if env.is_empty() {
            String::new()
        } else {
            format!("{env}_")
        };
        sql.replace(":SCHEMA.", &prefix)
            .replace(":SCHEMA", &prefix)
            .replace("ops.", "ops_")
    }
}

#[async_trait]
impl Driver for SqliteDriver {
    fn kind(&self) -> DriverKind {
        DriverKind::Sqlite
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            edge: false,
            rls: false,
            local_stack: false,
            pull: false,
            promote_target: false,
            outbox: false,
            registry: false,
        }
    }

    async fn apply_sql(&self, env: &str, sql: &str) -> Result<()> {
        let sql = Self::rewrite(env, sql);
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(&sql).map_err(sqlite_err)?;
        Ok(())
    }

    async fn query(&self, env: &str, sql: &str) -> Result<Rows> {
        let sql = Self::rewrite(env, sql);
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
        let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let ncols = columns.len();
        let mut out = Rows {
            columns,
            rows: Vec::new(),
        };
        let mut rows = stmt.query([]).map_err(sqlite_err)?;
        while let Some(row) = rows.next().map_err(sqlite_err)? {
            let mut cells = Vec::with_capacity(ncols);
            for i in 0..ncols {
                // Render any column type as text via rusqlite's ValueRef.
                let v: rusqlite::types::Value = row.get(i).map_err(sqlite_err)?;
                cells.push(match v {
                    rusqlite::types::Value::Null => None,
                    rusqlite::types::Value::Integer(n) => Some(n.to_string()),
                    rusqlite::types::Value::Real(f) => Some(f.to_string()),
                    rusqlite::types::Value::Text(s) => Some(s),
                    rusqlite::types::Value::Blob(b) => Some(format!("<blob {} bytes>", b.len())),
                });
            }
            out.rows.push(cells);
        }
        Ok(out)
    }

    async fn introspect(&self, env: &str, q: IntrospectQuery) -> Result<Introspection> {
        // sqlite introspects via sqlite_master; policies/triggers-as-Postgres → empty.
        let sql = match q {
            IntrospectQuery::Tables => {
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name".to_string()
            }
            IntrospectQuery::Describe { table } => {
                format!("PRAGMA table_info({})", Self::rewrite(env, &table))
            }
            IntrospectQuery::Functions | IntrospectQuery::Handlers => {
                // No stored functions / handler registry on sqlite v1.
                return Ok(Introspection::default());
            }
            IntrospectQuery::Triggers => {
                "SELECT name FROM sqlite_master WHERE type='trigger' ORDER BY name".to_string()
            }
            IntrospectQuery::Policies => return Ok(Introspection::default()),
            IntrospectQuery::Size => {
                "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name".to_string()
            }
        };
        let rows = self.query(env, &sql).await?;
        Ok(Introspection { rows })
    }

    async fn ledger_record(&self, row: AppliedMigration) -> Result<()> {
        // Ledger-only baseline on sqlite: ops_applied_migrations (design §3.4).
        let conn = self.conn.lock().unwrap();
        // Assign monotonic applied_seq ourselves (no bigserial).
        let next_seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(applied_seq), 0) + 1 FROM ops_applied_migrations",
                [],
                |r| r.get(0),
            )
            .unwrap_or(1);
        conn.execute(
            "INSERT OR REPLACE INTO ops_applied_migrations \
             (env, schema, id, checksum, applied_seq) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![row.env, row.schema, row.id, row.checksum, next_seq],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    async fn advisory_lock(&self, key: &str) -> Result<LockGuard> {
        // sqlite serializes through the connection mutex already; the guard is a no-op.
        Ok(LockGuard::noop(key))
    }

    async fn apply_atomic(
        &self,
        env: &str,
        _lock_key: &str,
        statements: &[String],
        ledger: AppliedMigration,
    ) -> Result<()> {
        // Genuinely atomic on sqlite (findings 4 + 6): wrap the up→fakedata→test_up
        // statements AND the ledger INSERT in ONE transaction. The connection mutex
        // already serializes access, so no advisory lock is needed. Any failure rolls
        // the whole unit back — the up-DDL reverts and no ledger row is written.
        let conn = self.conn.lock().unwrap();
        conn.execute_batch("BEGIN").map_err(sqlite_err)?;
        let run = || -> Result<()> {
            for s in statements {
                let sql = Self::rewrite(env, s);
                conn.execute_batch(&sql).map_err(sqlite_err)?;
            }
            // Assign a monotonic applied_seq ourselves (no bigserial on sqlite).
            let next_seq: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(applied_seq), 0) + 1 FROM ops_applied_migrations",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            conn.execute(
                "INSERT OR REPLACE INTO ops_applied_migrations \
                 (env, schema, id, checksum, applied_seq) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![ledger.env, ledger.schema, ledger.id, ledger.checksum, next_seq],
            )
            .map_err(sqlite_err)?;
            Ok(())
        };
        match run() {
            Ok(()) => {
                conn.execute_batch("COMMIT").map_err(sqlite_err)?;
                Ok(())
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // edge / registry / outbox / local_lifecycle / pull all use the trait's
    // NotImplemented defaults — sqlite is introspect/query/SQL-migration only in v1.
}

impl SqliteDriver {
    /// Path this driver opened (for tracing / diagnostics).
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}
