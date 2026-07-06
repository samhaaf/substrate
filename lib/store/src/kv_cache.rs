//! KV cache entry persistence: the `kv_cache_entries` table.
//!
//! Metadata for disk-backed prefix cache blobs. The blobs themselves are
//! opaque files managed by `substrate-cache`. This module stores the index
//! (prompt hash → file path) and LRU tracking.

use rusqlite::params;

use substrate_types::{ModelId, Result};

use crate::{sqlite_err, Store};

/// A KV cache entry record.
#[derive(Debug, Clone)]
pub struct KvCacheEntry {
    /// Hash-based ID (SHA-256 of model + prompt hash, typically).
    pub id: String,
    /// The model this cache entry belongs to.
    pub model_id: ModelId,
    /// SHA-256 of the tokenized prompt prefix.
    pub prompt_hash: String,
    /// Absolute path to the saved KV state file on disk.
    pub file_path: String,
    /// Size of the file in bytes.
    pub bytes: u64,
    /// Number of times this entry has been used (cache hit count).
    pub hit_count: u64,
    /// When this entry was first created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When this entry was last accessed (LRU eviction key).
    pub last_accessed_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_dt(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_default()
}

fn map_kv_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<KvCacheEntry> {
    Ok(KvCacheEntry {
        id:               row.get(0)?,
        model_id:         row.get(1)?,
        prompt_hash:      row.get(2)?,
        file_path:        row.get(3)?,
        bytes:            row.get::<_, i64>(4)? as u64,
        hit_count:        row.get::<_, i64>(5)? as u64,
        created_at:       parse_dt(&row.get::<_, String>(6)?),
        last_accessed_at: parse_dt(&row.get::<_, String>(7)?),
    })
}

const SELECT_COLS: &str =
    "id, model_id, prompt_hash, file_path, bytes, hit_count, created_at, last_accessed_at";

// ---------------------------------------------------------------------------
// Store impl
// ---------------------------------------------------------------------------

impl Store {
    /// Insert or update a KV cache entry (upsert by id).
    ///
    /// On conflict, updates `last_accessed_at` and increments `hit_count`.
    pub fn upsert_kv_cache_entry(&self, entry: &KvCacheEntry) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO kv_cache_entries
             (id, model_id, prompt_hash, file_path, bytes, hit_count, created_at, last_accessed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(id) DO UPDATE SET
                last_accessed_at = excluded.last_accessed_at,
                hit_count        = hit_count + 1",
            params![
                entry.id,
                entry.model_id,
                entry.prompt_hash,
                entry.file_path,
                entry.bytes as i64,
                entry.hit_count as i64,
                entry.created_at.to_rfc3339(),
                entry.last_accessed_at.to_rfc3339(),
            ],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Find a cached prefix entry by (model_id, prompt_hash).
    /// Returns None if no match exists.
    pub fn find_kv_cache_entry(
        &self,
        model_id: &ModelId,
        prompt_hash: &str,
    ) -> Result<Option<KvCacheEntry>> {
        let conn = self.conn()?;
        let result = conn.query_row(
            &format!("SELECT {SELECT_COLS} FROM kv_cache_entries
                      WHERE model_id = ?1 AND prompt_hash = ?2"),
            params![model_id, prompt_hash],
            map_kv_row,
        );
        match result {
            Ok(entry) => Ok(Some(entry)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(sqlite_err(e)),
        }
    }

    /// Return total bytes of all KV cache entries.
    pub fn total_kv_cache_bytes(&self) -> Result<u64> {
        let conn = self.conn()?;
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(bytes), 0) FROM kv_cache_entries",
            [],
            |row| row.get(0),
        ).map_err(sqlite_err)?;
        Ok(total as u64)
    }

    /// Return entries eligible for eviction, ordered by LRU (least recently accessed first).
    /// If `model_id` is provided, only returns entries for that model.
    pub fn kv_cache_for_eviction(&self, model_id: Option<&ModelId>) -> Result<Vec<KvCacheEntry>> {
        let conn = self.conn()?;
        if let Some(model) = model_id {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLS} FROM kv_cache_entries
                 WHERE model_id = ?1
                 ORDER BY last_accessed_at ASC",
            )).map_err(sqlite_err)?;
            let rows = stmt
                .query_map(params![model], map_kv_row)
                .map_err(sqlite_err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_err)?;
            Ok(rows)
        } else {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLS} FROM kv_cache_entries ORDER BY last_accessed_at ASC",
            )).map_err(sqlite_err)?;
            let rows = stmt
                .query_map([], map_kv_row)
                .map_err(sqlite_err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_err)?;
            Ok(rows)
        }
    }

    /// Delete a single KV cache entry by ID.
    pub fn delete_kv_cache_entry(&self, id: &str) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "DELETE FROM kv_cache_entries WHERE id = ?1",
            params![id],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Delete all KV cache entries for a given model (called on model eviction).
    /// Returns the number of entries deleted.
    pub fn delete_kv_cache_for_model(&self, model_id: &ModelId) -> Result<u64> {
        let conn = self.conn()?;
        let deleted = conn.execute(
            "DELETE FROM kv_cache_entries WHERE model_id = ?1",
            params![model_id],
        ).map_err(sqlite_err)? as u64;
        Ok(deleted)
    }
}
