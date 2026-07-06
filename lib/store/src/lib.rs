//! # substrate-store
//!
//! SQLite persistence layer — the system of record.
//!
//! All database access is serialized through a `Mutex<Connection>`. There is
//! intentionally no async I/O here: SQLite operations are fast and bounded,
//! and the mutex ensures consistency without complexity.
//!
//! ## Schema
//!
//! The schema is embedded at compile time from `schema.sql` and applied on
//! `Store::open`. Migrations are append-only ALTER TABLE statements added at
//! the bottom of schema.sql (SQLite `IF NOT EXISTS` semantics).
//!
//! ## Module Structure
//!
//! - [`completions`] — Completion CRUD and queue view
//! - [`collections`] — Collection lifecycle
//! - [`models`]      — Model registry and download state
//! - [`results`]     — Completion result blobs
//! - [`benchmarks`]  — Benchmark run records
//! - [`kv_cache`]    — KV cache entry metadata
//!
//! ## Observer Seam
//!
//! [`StoreObserver`] is a no-op trait that can be injected into `Store` to observe
//! every mutation. The Knowledge Graph crate will implement this trait and inject
//! itself at `Node::start()` time. Default is `None` (no-op).

pub mod completions;
pub mod collections;
pub mod models;
pub mod results;
pub mod benchmarks;
pub mod kv_cache;

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use substrate_types::{CompletionId, CompletionState, ModelId, Result, SubstrateError};

// Re-export row/entry types defined in sub-modules for convenience.
pub use benchmarks::BenchmarkRun;
pub use kv_cache::KvCacheEntry;

/// Convert rusqlite errors to SubstrateError::Store.
pub(crate) fn sqlite_err(e: rusqlite::Error) -> SubstrateError {
    SubstrateError::Store(e.to_string())
}

/// Convert serde_json errors to SubstrateError::Internal.
pub(crate) fn json_err(e: serde_json::Error) -> SubstrateError {
    SubstrateError::Internal(format!("json serialization error: {e}"))
}

/// Embedded schema SQL — applied on every `Store::open`.
const SCHEMA_SQL: &str = include_str!("../schema.sql");

// ---------------------------------------------------------------------------
// Observer seam (Knowledge Graph hook)
// ---------------------------------------------------------------------------

/// Called on every store mutation. Implement to build derived indexes.
///
/// All methods have default no-op implementations — implement only what you need.
/// The store holds `Option<Arc<dyn StoreObserver>>`.
///
/// # Future
/// The Knowledge Graph crate implements this trait and is injected at `Node::start()`.
pub trait StoreObserver: Send + Sync {
    fn on_completion_inserted(&self, _id: CompletionId) {}
    fn on_completion_state_changed(
        &self,
        _id: CompletionId,
        _old: CompletionState,
        _new: CompletionState,
    ) {}
    fn on_model_registered(&self, _id: &ModelId) {}
    fn on_benchmark_recorded(&self, _model: &ModelId, _run_id: i64) {}
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// The substrate SQLite store. All database access goes through this type.
///
/// Cheap to clone (wraps an `Arc`). All methods take `&self`.
#[derive(Clone)]
pub struct Store {
    pub(crate) conn: Arc<Mutex<Connection>>,
    pub(crate) observer: Option<Arc<dyn StoreObserver>>,
}

impl Store {
    /// Open a persistent store at `path`. Creates the file if it does not exist.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).map_err(sqlite_err)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            observer: None,
        };
        store.apply_schema()?;
        Ok(store)
    }

    /// Open an in-memory store (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(sqlite_err)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            observer: None,
        };
        store.apply_schema()?;
        Ok(store)
    }

    /// Inject a [`StoreObserver`] for change notification.
    pub fn with_observer(mut self, observer: Arc<dyn StoreObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// On startup: move any completions stuck in `Running` state back to `Pending`.
    /// Also sets `recovered_at` so the history shows the recovery event.
    /// Returns the number of completions recovered.
    pub fn recover_running(&self) -> Result<u64> {
        let conn = self.conn()?;
        let now = chrono::Utc::now().to_rfc3339();
        let recovered = conn.execute(
            "UPDATE completions
             SET state = 'pending', started_at = NULL, recovered_at = ?1
             WHERE state = 'running'",
            rusqlite::params![now],
        ).map_err(sqlite_err)? as u64;
        tracing::info!(recovered, "crash recovery: requeued running completions");
        Ok(recovered)
    }

    /// Acquire the mutex-guarded connection.
    pub(crate) fn conn(
        &self,
    ) -> std::result::Result<std::sync::MutexGuard<'_, Connection>, SubstrateError> {
        self.conn
            .lock()
            .map_err(|_| SubstrateError::Internal("store mutex poisoned".into()))
    }

    fn apply_schema(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(|_| {
            SubstrateError::Internal("mutex poisoned in apply_schema".into())
        })?;
        conn.execute_batch(SCHEMA_SQL).map_err(sqlite_err)
    }
}
