//! SQLite persistence — the coordination point for concurrent wrappers.
//!
//! WAL mode (set on open) lets multiple `ccw` processes coordinate through the
//! one `ccw.db`. Access is serialized through an `Arc<Mutex<Connection>>`, the
//! same pattern `substrate-store` uses. The schema is embedded from
//! `schema.sql` and applied (append-only, `IF NOT EXISTS`) on every open.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use substrate_types::{Result, SubstrateError};

use crate::budget::BudgetRules;
use crate::event::{CcwEvent, SeqEvent};

const SCHEMA_SQL: &str = include_str!("../schema.sql");

fn sqlite_err(e: rusqlite::Error) -> SubstrateError {
    SubstrateError::Store(format!("ccw sqlite: {e}"))
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// The ccw store handle. Cheap to clone (wraps an `Arc`).
#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

/// A ledger row for one wrapped invocation.
#[derive(Debug, Clone, Default)]
pub struct InvocationRecord {
    pub budget_id: Option<String>,
    pub account: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub usage_json: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub limit_hit: bool,
    pub reset_prose: Option<String>,
    pub exit_code: Option<i64>,
}

/// A `/usage` pool observation row.
#[derive(Debug, Clone)]
pub struct ObservationRecord {
    pub account: String,
    pub pool: String,
    pub pct: f64,
    pub reset_at_raw: Option<String>,
    pub observed_at: String,
}

/// A tokens-per-percent calibration row.
#[derive(Debug, Clone)]
pub struct CalibrationRecord {
    pub account: String,
    pub pool: String,
    pub tokens_per_percent: f64,
    pub updated_at: Option<String>,
    pub sample_count: u64,
}

/// A persisted session (thread) row.
#[derive(Debug, Clone)]
pub struct SessionRow {
    pub id: String,
    pub account: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub budget_id: Option<String>,
    pub spec_json: String,
    pub status: String,
    pub last_seq: u64,
    pub created_at: String,
    pub updated_at: String,
}

/// An account-seen row (from `claude auth status`).
#[derive(Debug, Clone, Default)]
pub struct AccountSeen {
    pub name: String,
    pub config_dir: Option<String>,
    pub subscription_type: Option<String>,
    pub email: Option<String>,
    pub logged_in: bool,
    pub last_checked_at: Option<String>,
}

impl Store {
    /// Open a persistent store at `path` (WAL). Creates parent dirs + the file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    SubstrateError::Store(format!("creating {}: {e}", parent.display()))
                })?;
            }
        }
        let conn = Connection::open(path).map_err(sqlite_err)?;
        // WAL: concurrent wrappers coordinate through the one db.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(sqlite_err)?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(sqlite_err)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.apply_schema()?;
        Ok(store)
    }

    /// Open an in-memory store (tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(sqlite_err)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.apply_schema()?;
        Ok(store)
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|_| SubstrateError::Internal("ccw store mutex poisoned".into()))
    }

    fn apply_schema(&self) -> Result<()> {
        let conn = self.conn()?;
        conn.execute_batch(SCHEMA_SQL).map_err(sqlite_err)
    }

    // ── accounts_seen ──────────────────────────────────────────────────────

    pub fn upsert_account_seen(&self, a: &AccountSeen) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO accounts_seen
                (name, config_dir, subscription_type, email, logged_in, last_checked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET
                config_dir=excluded.config_dir,
                subscription_type=excluded.subscription_type,
                email=excluded.email,
                logged_in=excluded.logged_in,
                last_checked_at=excluded.last_checked_at",
            params![
                a.name,
                a.config_dir,
                a.subscription_type,
                a.email,
                a.logged_in as i64,
                a.last_checked_at,
            ],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    pub fn list_accounts_seen(&self) -> Result<Vec<AccountSeen>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT name, config_dir, subscription_type, email, logged_in, last_checked_at
                 FROM accounts_seen ORDER BY name",
            )
            .map_err(sqlite_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AccountSeen {
                    name: r.get(0)?,
                    config_dir: r.get(1)?,
                    subscription_type: r.get(2)?,
                    email: r.get(3)?,
                    logged_in: r.get::<_, i64>(4)? != 0,
                    last_checked_at: r.get(5)?,
                })
            })
            .map_err(sqlite_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(sqlite_err)
    }

    // ── budgets ────────────────────────────────────────────────────────────

    pub fn upsert_budget(&self, id: &str, rules: &BudgetRules) -> Result<()> {
        let conn = self.conn()?;
        let now = now_rfc3339();
        conn.execute(
            "INSERT INTO budgets (id, rules_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET rules_json=excluded.rules_json, updated_at=excluded.updated_at",
            params![id, rules.to_json(), now],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    pub fn get_budget(&self, id: &str) -> Result<Option<BudgetRules>> {
        let conn = self.conn()?;
        let json: Option<String> = conn
            .query_row(
                "SELECT rules_json FROM budgets WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .ok();
        match json {
            Some(j) => Ok(Some(
                BudgetRules::from_json(&j).map_err(SubstrateError::Config)?,
            )),
            None => Ok(None),
        }
    }

    pub fn list_budgets(&self) -> Result<Vec<(String, BudgetRules)>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, rules_json FROM budgets ORDER BY id")
            .map_err(sqlite_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(sqlite_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, json) = row.map_err(sqlite_err)?;
            let rules = BudgetRules::from_json(&json).map_err(SubstrateError::Config)?;
            out.push((id, rules));
        }
        Ok(out)
    }

    // ── invocations ────────────────────────────────────────────────────────

    pub fn record_invocation(&self, rec: &InvocationRecord) -> Result<i64> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO invocations
                (budget_id, account, session_id, cwd, started_at, ended_at, usage_json,
                 input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                 total_tokens, cost_usd, limit_hit, reset_prose, exit_code)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                rec.budget_id,
                rec.account,
                rec.session_id,
                rec.cwd,
                rec.started_at,
                rec.ended_at,
                rec.usage_json,
                rec.input_tokens as i64,
                rec.output_tokens as i64,
                rec.cache_read_tokens as i64,
                rec.cache_creation_tokens as i64,
                rec.total_tokens as i64,
                rec.cost_usd,
                rec.limit_hit as i64,
                rec.reset_prose,
                rec.exit_code,
            ],
        )
        .map_err(sqlite_err)?;
        Ok(conn.last_insert_rowid())
    }

    pub fn list_invocations(&self, budget_id: Option<&str>, limit: usize) -> Result<Vec<InvocationRecord>> {
        let conn = self.conn()?;
        let (sql, has_filter) = match budget_id {
            Some(_) => (
                "SELECT budget_id, account, session_id, cwd, started_at, ended_at, usage_json,
                        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                        total_tokens, cost_usd, limit_hit, reset_prose, exit_code
                 FROM invocations WHERE budget_id = ?1 ORDER BY id DESC LIMIT ?2",
                true,
            ),
            None => (
                "SELECT budget_id, account, session_id, cwd, started_at, ended_at, usage_json,
                        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                        total_tokens, cost_usd, limit_hit, reset_prose, exit_code
                 FROM invocations ORDER BY id DESC LIMIT ?1",
                false,
            ),
        };
        let mut stmt = conn.prepare(sql).map_err(sqlite_err)?;
        let map = |r: &rusqlite::Row| -> rusqlite::Result<InvocationRecord> {
            Ok(InvocationRecord {
                budget_id: r.get(0)?,
                account: r.get(1)?,
                session_id: r.get(2)?,
                cwd: r.get(3)?,
                started_at: r.get(4)?,
                ended_at: r.get(5)?,
                usage_json: r.get(6)?,
                input_tokens: r.get::<_, i64>(7)? as u64,
                output_tokens: r.get::<_, i64>(8)? as u64,
                cache_read_tokens: r.get::<_, i64>(9)? as u64,
                cache_creation_tokens: r.get::<_, i64>(10)? as u64,
                total_tokens: r.get::<_, i64>(11)? as u64,
                cost_usd: r.get(12)?,
                limit_hit: r.get::<_, i64>(13)? != 0,
                reset_prose: r.get(14)?,
                exit_code: r.get(15)?,
            })
        };
        let rows = if has_filter {
            stmt.query_map(params![budget_id.unwrap(), limit as i64], map)
        } else {
            stmt.query_map(params![limit as i64], map)
        }
        .map_err(sqlite_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(sqlite_err)
    }

    /// Total tokens consumed by a budget since `since_rfc3339` (weekly window).
    pub fn weekly_consumed_tokens(&self, budget_id: &str, since_rfc3339: &str) -> Result<u64> {
        let conn = self.conn()?;
        let total: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(total_tokens), 0) FROM invocations
                 WHERE budget_id = ?1 AND (started_at IS NULL OR started_at >= ?2)",
                params![budget_id, since_rfc3339],
                |r| r.get(0),
            )
            .map_err(sqlite_err)?;
        Ok(total.max(0) as u64)
    }

    // ── limit_observations ─────────────────────────────────────────────────

    pub fn record_observation(&self, o: &ObservationRecord) -> Result<i64> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO limit_observations (account, pool, pct, reset_at_raw, observed_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![o.account, o.pool, o.pct, o.reset_at_raw, o.observed_at],
        )
        .map_err(sqlite_err)?;
        Ok(conn.last_insert_rowid())
    }

    /// The most recent observation for `(account, pool)`, if any.
    pub fn latest_observation(&self, account: &str, pool: &str) -> Result<Option<ObservationRecord>> {
        let conn = self.conn()?;
        let r = conn
            .query_row(
                "SELECT account, pool, pct, reset_at_raw, observed_at FROM limit_observations
                 WHERE account = ?1 AND pool = ?2 ORDER BY observed_at DESC, id DESC LIMIT 1",
                params![account, pool],
                |r| {
                    Ok(ObservationRecord {
                        account: r.get(0)?,
                        pool: r.get(1)?,
                        pct: r.get(2)?,
                        reset_at_raw: r.get(3)?,
                        observed_at: r.get(4)?,
                    })
                },
            )
            .ok();
        Ok(r)
    }

    // ── calibrations ───────────────────────────────────────────────────────

    pub fn get_calibration(&self, account: &str, pool: &str) -> Result<Option<CalibrationRecord>> {
        let conn = self.conn()?;
        let r = conn
            .query_row(
                "SELECT account, pool, tokens_per_percent, updated_at, sample_count
                 FROM calibrations WHERE account = ?1 AND pool = ?2",
                params![account, pool],
                |r| {
                    Ok(CalibrationRecord {
                        account: r.get(0)?,
                        pool: r.get(1)?,
                        tokens_per_percent: r.get(2)?,
                        updated_at: r.get(3)?,
                        sample_count: r.get::<_, i64>(4)? as u64,
                    })
                },
            )
            .ok();
        Ok(r)
    }

    pub fn upsert_calibration(&self, c: &CalibrationRecord) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO calibrations (account, pool, tokens_per_percent, updated_at, sample_count)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(account, pool) DO UPDATE SET
                tokens_per_percent=excluded.tokens_per_percent,
                updated_at=excluded.updated_at,
                sample_count=excluded.sample_count",
            params![
                c.account,
                c.pool,
                c.tokens_per_percent,
                c.updated_at,
                c.sample_count as i64
            ],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    // ── sessions (v1 daemon) ─────────────────────────────────────────────

    /// Insert a new session row (idempotent on id).
    pub fn insert_session(&self, s: &SessionRow) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO sessions
                (id, account, cwd, model, budget_id, spec_json, status, last_seq, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(id) DO UPDATE SET
                account=excluded.account, cwd=excluded.cwd, model=excluded.model,
                budget_id=excluded.budget_id, spec_json=excluded.spec_json,
                updated_at=excluded.updated_at",
            params![
                s.id, s.account, s.cwd, s.model, s.budget_id, s.spec_json,
                s.status, s.last_seq as i64, s.created_at, s.updated_at,
            ],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    /// Update a session's status + last_seq watermark.
    pub fn update_session_status(&self, id: &str, status: &str, last_seq: u64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE sessions SET status=?2, last_seq=?3, updated_at=?4 WHERE id=?1",
            params![id, status, last_seq as i64, now_rfc3339()],
        )
        .map_err(sqlite_err)?;
        Ok(())
    }

    fn map_session(r: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
        Ok(SessionRow {
            id: r.get(0)?,
            account: r.get(1)?,
            cwd: r.get(2)?,
            model: r.get(3)?,
            budget_id: r.get(4)?,
            spec_json: r.get(5)?,
            status: r.get(6)?,
            last_seq: r.get::<_, i64>(7)? as u64,
            created_at: r.get(8)?,
            updated_at: r.get(9)?,
        })
    }

    pub fn get_session(&self, id: &str) -> Result<Option<SessionRow>> {
        let conn = self.conn()?;
        let r = conn
            .query_row(
                "SELECT id, account, cwd, model, budget_id, spec_json, status, last_seq, created_at, updated_at
                 FROM sessions WHERE id = ?1",
                params![id],
                Self::map_session,
            )
            .ok();
        Ok(r)
    }

    pub fn list_sessions(&self, limit: usize) -> Result<Vec<SessionRow>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, account, cwd, model, budget_id, spec_json, status, last_seq, created_at, updated_at
                 FROM sessions ORDER BY updated_at DESC LIMIT ?1",
            )
            .map_err(sqlite_err)?;
        let rows = stmt
            .query_map(params![limit as i64], Self::map_session)
            .map_err(sqlite_err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(sqlite_err)
    }

    // ── event log (v1 daemon) ────────────────────────────────────────────

    /// Append one sequenced event to the log (system of record for history +
    /// resume). Returns the row id.
    pub fn append_event(&self, ev: &SeqEvent) -> Result<i64> {
        let conn = self.conn()?;
        let payload = serde_json::to_string(&ev.event).map_err(SubstrateError::from)?;
        conn.execute(
            "INSERT INTO events (session_id, seq, kind, payload_json, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id, seq) DO NOTHING",
            params![ev.session_id, ev.seq as i64, ev.event.kind(), payload, now_rfc3339()],
        )
        .map_err(sqlite_err)?;
        Ok(conn.last_insert_rowid())
    }

    /// Paged history for a session: events with `seq > after_seq`, ascending,
    /// capped at `limit`. Serves the WS `sessions.history` call.
    pub fn list_events(&self, session_id: &str, after_seq: u64, limit: usize) -> Result<Vec<SeqEvent>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT seq, payload_json FROM events
                 WHERE session_id = ?1 AND seq > ?2 ORDER BY seq ASC LIMIT ?3",
            )
            .map_err(sqlite_err)?;
        let rows = stmt
            .query_map(params![session_id, after_seq as i64, limit as i64], |r| {
                Ok((r.get::<_, i64>(0)? as u64, r.get::<_, String>(1)?))
            })
            .map_err(sqlite_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, payload) = row.map_err(sqlite_err)?;
            let event: CcwEvent = serde_json::from_str(&payload).map_err(SubstrateError::from)?;
            out.push(SeqEvent {
                session_id: session_id.to_string(),
                seq,
                event,
            });
        }
        Ok(out)
    }

    /// The highest seq recorded for a session (0 if none) — resume watermark.
    pub fn max_seq(&self, session_id: &str) -> Result<u64> {
        let conn = self.conn()?;
        let seq: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM events WHERE session_id = ?1",
                params![session_id],
                |r| r.get(0),
            )
            .map_err(sqlite_err)?;
        Ok(seq.max(0) as u64)
    }
}
