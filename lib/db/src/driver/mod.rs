//! The `Driver` trait + capability flags — the substrate "seam" for the three
//! backends (design §10). Every driver implements the SQL core; edge / registry /
//! local-stack / pull default to a typed `NotImplemented` so an incapable driver
//! (sqlite) degrades cleanly and early, never with a confusing deep runtime error.

pub mod sqlite;
pub mod supabase_cloud;
pub mod supabase_local;

use async_trait::async_trait;
use serde_json::Value;
use substrate_types::{Result, SubstrateError};

use crate::config::DriverKind;

/// Construct the typed `NotImplemented` error for a command/driver pair.
pub(crate) fn not_impl(command: &'static str, driver: DriverKind) -> SubstrateError {
    SubstrateError::NotImplemented {
        command,
        driver: driver.as_str(),
        reason: "capability not supported by this driver",
    }
}

/// Per-driver capability flags (design §10.1). Commands consult these up front and
/// emit `NotImplemented` before doing any work.
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    /// deploy/activate/codegen-TS/validator-sync-edge.
    pub edge: bool,
    /// RLS policy introspection.
    pub rls: bool,
    /// start/stop/reset a local stack.
    pub local_stack: bool,
    /// pull prod data down.
    pub pull: bool,
    /// can be promoted TO.
    pub promote_target: bool,
    /// has the `handler_dispatch` outbox table + a drainer.
    pub outbox: bool,
    /// has the `handler_versions` / `handler_active` / `edge_deploys` registry.
    pub registry: bool,
}

/// A row set returned from `query` — column names + string-rendered cells.
///
/// v1 renders every cell as a `String` (Postgres text protocol) so the CLI can print
/// table / json / csv without knowing the concrete types. Structured typing is EXT.
#[derive(Debug, Clone, Default)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// A parameterized introspection request (design §10.1).
#[derive(Debug, Clone)]
pub enum IntrospectQuery {
    Tables,
    Describe { table: String },
    Functions,
    Triggers,
    Policies,
    Handlers,
    Size,
}

/// The result of an introspection — v1 just carries a `Rows` grid.
#[derive(Debug, Clone, Default)]
pub struct Introspection {
    pub rows: Rows,
}

/// A bound SQL parameter for parameterized (`$1`, `$2`, …) execution (finding 5).
///
/// Free-form / handler-supplied values MUST flow through these rather than string
/// interpolation, so a `'` in a reason/handler/code can neither break nor inject SQL.
#[derive(Debug, Clone)]
pub enum SqlParam {
    Text(String),
    /// A nullable text value (`NULL` when `None`).
    TextOpt(Option<String>),
    Int(i64),
    /// A nullable integer value (`NULL` when `None`).
    IntOpt(Option<i64>),
    Bool(bool),
}

/// A ledger row to record after a successful apply (design §3.1.a).
#[derive(Debug, Clone)]
pub struct AppliedMigration {
    pub env: String,
    pub schema: String,
    /// `<schema>/<seq>_<name>`.
    pub id: String,
    pub checksum: String,
}

/// A built edge bundle to deploy (design §7.2). `bytes` is the built artifact;
/// `source_hash` is its sha256 (M3).
#[derive(Debug, Clone)]
pub struct Bundle {
    pub bytes: Vec<u8>,
    pub source_hash: String,
    /// Content-addressed path the bundle was stored at (`bundle_ref`).
    pub bundle_ref: String,
}

/// Result of a `deploy_edge` call.
#[derive(Debug, Clone)]
pub struct Deploy {
    pub slug: String,
    pub source_hash: String,
}

/// Time window for log/edge queries.
#[derive(Debug, Clone)]
pub struct Since(pub String);

/// Edge / db log lines.
#[derive(Debug, Clone, Default)]
pub struct Logs {
    pub lines: Vec<String>,
}

/// Report from a single `outbox drain` pass.
#[derive(Debug, Clone, Default)]
pub struct DrainReport {
    pub fired: usize,
    pub failed: usize,
    pub skipped_quarantined: usize,
    /// Rows collapsed because another row with the same `idempotency_key` already fired
    /// (idempotent dedup — at-least-once without duplicate delivery, design §8.2).
    pub deduped: usize,
}

/// Local-stack lifecycle operations (design §10).
#[derive(Debug, Clone, Copy)]
pub enum LocalOp {
    Up,
    Down,
    Status,
    Restart,
    Reset,
    Ensure,
}

/// Options for `pull` (deferred in v1; the seam is reserved).
#[derive(Debug, Clone, Default)]
pub struct PullOpts {
    pub with_prod_data: bool,
    pub no_blob: bool,
}

/// An acquired advisory lock. Dropping it RELIABLY releases the lock (RAII, finding 4):
/// a session-level Postgres lock carries a release hook that actually runs
/// `pg_advisory_unlock(hashtext(key))` on drop, so the lock never leaks for the
/// connection's life. sqlite serializes via its process mutex, so its guard is a no-op.
///
/// NOTE: the load-bearing apply/promote/activate concurrency guard uses
/// `pg_advisory_xact_lock` INSIDE the driving transaction (see `apply_atomic`), which the
/// database auto-releases at COMMIT/ROLLBACK — leak-proof by construction. This guard is
/// for the session-level `advisory_lock` API only.
pub struct LockGuard {
    pub key: String,
    /// Release hook: runs `pg_advisory_unlock` on drop. `None` for sqlite / xact locks.
    #[allow(clippy::type_complexity)]
    release: Option<Box<dyn FnOnce() + Send>>,
}

impl LockGuard {
    /// A guard with a release hook (Postgres session lock).
    pub fn with_release(key: impl Into<String>, release: Box<dyn FnOnce() + Send>) -> Self {
        Self {
            key: key.into(),
            release: Some(release),
        }
    }

    /// A no-op guard (sqlite / advisory-only).
    pub fn noop(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            release: None,
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
        tracing::debug!(key = %self.key, "released advisory lock");
    }
}

/// The database driver seam. All fallible methods return `substrate_types::Result`.
///
/// The SQL core (`apply_sql`/`query`/`introspect`/`ledger_record`/`advisory_lock`) is
/// implemented by every driver. Edge/registry/local-stack/pull methods default to a
/// typed `NotImplemented`; sqlite leaves those defaults (design §10.2).
#[async_trait]
pub trait Driver: Send + Sync {
    fn kind(&self) -> DriverKind;
    fn capabilities(&self) -> Capabilities;

    /// A direct Postgres connection URL suitable for `pg_dump` / `supabase db dump`, if
    /// this driver can expose one WITHOUT a secret it does not hold. The local driver
    /// returns its Docker Postgres URL; cloud/sqlite return `None` (the cloud DB password
    /// is not in this crate — the snapshot path resolves it from `DB_DUMP_URL` instead and
    /// records the gap). Read-only: the caller only ever dumps FROM this URL.
    fn dump_url(&self) -> Option<String> {
        None
    }

    // ── SQL core — implemented by ALL drivers ──────────────────────────────

    /// Apply raw SQL against `env` (binds `:SCHEMA` per env; sqlite rewrites to a
    /// table-name prefix — design §3.4).
    async fn apply_sql(&self, env: &str, sql: &str) -> Result<()>;

    /// Apply SQL with bound parameters (`$1`, `$2`, … in `sql`), for any statement that
    /// carries free-form / handler-supplied values (finding 5 — closes injection /
    /// escaping risks). Postgres drivers bind natively; the default here safely quotes
    /// each parameter as a SQL literal and delegates to [`apply_sql`], so no driver can
    /// silently fall back to unescaped interpolation.
    async fn apply_params(&self, env: &str, sql: &str, params: &[SqlParam]) -> Result<()> {
        self.apply_sql(env, &substitute_params(sql, params)).await
    }

    /// Run a read query and return a rendered grid.
    async fn query(&self, env: &str, sql: &str) -> Result<Rows>;

    /// Run a read query with bound parameters (`$1`, …) for any filter that carries
    /// free-form values (e.g. a `--handler` / `--status` CLI arg) — finding 5. Default
    /// safely substitutes literals and delegates to [`query`]; Postgres binds natively.
    async fn query_params(&self, env: &str, sql: &str, params: &[SqlParam]) -> Result<Rows> {
        self.query(env, &substitute_params(sql, params)).await
    }

    /// Structured schema introspection.
    async fn introspect(&self, env: &str, q: IntrospectQuery) -> Result<Introspection>;

    /// Record a ledger row after a successful apply.
    async fn ledger_record(&self, row: AppliedMigration) -> Result<()>;

    /// Acquire an advisory lock keyed by `key`.
    async fn advisory_lock(&self, key: &str) -> Result<LockGuard>;

    /// Apply one migration ATOMICALLY, under a transaction-scoped advisory lock
    /// (design §6.1/§6.5, finding 4). Runs, in ONE transaction:
    /// `pg_advisory_xact_lock(hashtext(lock_key))` → each statement in `statements` (in
    /// order: up → fakedata → test_up) → the ledger INSERT. If ANY step fails, the whole
    /// transaction ROLLS BACK — the up-DDL is reverted and NO ledger row is written (no
    /// partial state). The xact lock releases automatically at COMMIT/ROLLBACK.
    ///
    /// Default impl (sqlite): wraps the same sequence in the driver's own transaction
    /// (the process mutex already serializes, so no advisory lock is needed).
    async fn apply_atomic(
        &self,
        env: &str,
        lock_key: &str,
        statements: &[String],
        ledger: AppliedMigration,
    ) -> Result<()> {
        // Fallback (no native txn seam): best-effort sequential apply + ledger. Postgres
        // and sqlite BOTH override this with a genuinely-atomic implementation, so this
        // default is never taken on a real driver.
        let _ = lock_key;
        for s in statements {
            self.apply_sql(env, s).await?;
        }
        self.ledger_record(ledger).await
    }

    // ── Edge / registry — default = NotImplemented ─────────────────────────

    async fn deploy_edge(
        &self,
        _env: &str,
        _name: &str,
        _v: &str,
        _bundle: &Bundle,
    ) -> Result<Deploy> {
        Err(SubstrateError::NotImplemented {
            command: "deploy_edge",
            driver: self.kind().as_str(),
            reason: "no edge runtime",
        })
    }

    async fn activate_handler(&self, _env: &str, _handler: &str, _v: &str) -> Result<()> {
        Err(not_impl("activate_handler", self.kind()))
    }

    /// Roll an active handler back to its PRIOR version for `env` — a one-row pointer
    /// flip (design §7.3, finding 8), never a redeploy. The prior version is the
    /// highest-`id` `handler_versions` row for this handler below the currently-active
    /// one. Default = NotImplemented (sqlite has no registry).
    async fn rollback_handler(&self, _env: &str, _handler: &str) -> Result<()> {
        Err(not_impl("rollback_handler", self.kind()))
    }

    async fn edge_logs(&self, _name: &str, _since: Since) -> Result<Logs> {
        Err(not_impl("edge_logs", self.kind()))
    }

    /// Drain the outbox for `env`, firing each dispatch through `transport` and deduping
    /// on `idempotency_key` (finding 2). Default = NotImplemented (sqlite has no outbox);
    /// Postgres drivers keep the default and the driver-agnostic drain in
    /// [`crate::outbox::drain_env`] runs over the SQL seam instead.
    async fn outbox_drain(
        &self,
        _env: &str,
        _transport: &dyn crate::outbox::EdgeTransport,
    ) -> Result<DrainReport> {
        Err(not_impl("outbox_drain", self.kind()))
    }

    // ── Local stack / pull — default = NotImplemented ──────────────────────

    async fn local_lifecycle(&self, _op: LocalOp) -> Result<()> {
        Err(not_impl("local_lifecycle", self.kind()))
    }

    async fn pull(&self, _opts: PullOpts) -> Result<()> {
        Err(not_impl("pull", self.kind()))
    }
}

/// Safely substitute `$1`, `$2`, … placeholders in `sql` with SQL literals for a driver
/// that does not bind natively (the trait default / sqlite). Text is single-quote-escaped
/// (`'` → `''`); ints/bools/NULL render without quotes. This is the escaping backstop; the
/// Postgres drivers override `apply_params` to bind over the wire and never build literals.
pub(crate) fn substitute_params(sql: &str, params: &[SqlParam]) -> String {
    let lit = |p: &SqlParam| -> String {
        match p {
            SqlParam::Text(s) => format!("'{}'", s.replace('\'', "''")),
            SqlParam::TextOpt(Some(s)) => format!("'{}'", s.replace('\'', "''")),
            SqlParam::TextOpt(None) => "NULL".to_string(),
            SqlParam::Int(n) => n.to_string(),
            SqlParam::IntOpt(Some(n)) => n.to_string(),
            SqlParam::IntOpt(None) => "NULL".to_string(),
            SqlParam::Bool(b) => b.to_string(),
        }
    };
    // Replace the highest indices first so `$1` does not clobber `$10`.
    let mut out = sql.to_string();
    for (i, p) in params.iter().enumerate().rev() {
        out = out.replace(&format!("${}", i + 1), &lit(p));
    }
    out
}

/// The prior-version pointer-flip statement + its bound params (finding 8). Flips
/// `handler_active.version_id` for `(handler, env)` to the PRIOR version — the
/// highest-`id` `handler_versions` row for this handler strictly below the currently
/// active one. A one-row UPDATE, never a redeploy (§7.3). Shared by the local + cloud
/// drivers so the rollback SQL lives in exactly one place; the values are bound (`$1`,
/// `$2`) so a handler name with a `'` can neither break nor inject (finding 5).
pub(crate) const ROLLBACK_HANDLER_SQL: &str = "\
    UPDATE ops.handler_active ha \
       SET version_id = ( \
         SELECT hv.id FROM ops.handler_versions hv \
          WHERE hv.handler = $1 AND hv.id < ha.version_id \
          ORDER BY hv.id DESC LIMIT 1 \
       ) \
     WHERE ha.handler = $1 AND ha.env = $2 \
       AND EXISTS ( \
         SELECT 1 FROM ops.handler_versions hv \
          WHERE hv.handler = $1 AND hv.id < ha.version_id \
       )";

pub(crate) fn rollback_handler_params(handler: &str, env: &str) -> [SqlParam; 2] {
    [
        SqlParam::Text(handler.to_string()),
        SqlParam::Text(env.to_string()),
    ]
}

/// Render a `serde_json::Value` cell as an `Option<String>` for the `Rows` grid.
pub(crate) fn cell(v: &Value) -> Option<String> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}
