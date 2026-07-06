//! Model registry persistence: CRUD over the `models` table.

use chrono::Utc;
use rusqlite::params;

use substrate_types::{ModelId, ModelRow, Result, SubstrateError};

use crate::{sqlite_err, Store};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_dt(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_default()
}

fn map_model_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRow> {
    let registered_at_str: String = row.get(11)?;
    let last_used_at_str: Option<String> = row.get(12)?;

    Ok(ModelRow {
        id:              row.get(0)?,
        source:          row.get(1)?,
        prompt_template: row.get(2)?,
        context_length:  row.get::<_, i64>(3)? as u32,
        n_gpu_layers:    row.get::<_, Option<i64>>(4)?.map(|v| v as i32),
        max_slots:       row.get::<_, Option<i64>>(5)?.map(|v| v as u32),
        is_downloaded:   row.get::<_, i64>(6)? != 0,
        is_loaded:       row.get::<_, i64>(7)? != 0,
        file_path:       row.get(8)?,
        file_bytes:      row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        registered_at:   parse_dt(&registered_at_str),
        last_used_at:    last_used_at_str.as_deref().map(parse_dt),
    })
}

// ---------------------------------------------------------------------------
// Store impl
// ---------------------------------------------------------------------------

impl Store {
    /// Insert or update a model record (upsert by id).
    ///
    /// On conflict, updates config fields (source, prompt_template, context_length,
    /// n_gpu_layers, max_slots) but leaves runtime state (is_downloaded, file_path, etc.)
    /// untouched.
    pub fn upsert_model(&self, row: &ModelRow) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO models
             (id, source, prompt_template, context_length, n_gpu_layers, max_slots,
              is_downloaded, is_loaded, file_path, file_bytes, registered_at, last_used_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
             ON CONFLICT(id) DO UPDATE SET
                source          = excluded.source,
                prompt_template = excluded.prompt_template,
                context_length  = excluded.context_length,
                n_gpu_layers    = excluded.n_gpu_layers,
                max_slots       = excluded.max_slots",
            params![
                row.id,
                row.source,
                row.prompt_template,
                row.context_length as i64,
                row.n_gpu_layers.map(|v| v as i64),
                row.max_slots.map(|v| v as i64),
                row.is_downloaded as i64,
                row.is_loaded as i64,
                row.file_path,
                row.file_bytes.map(|v| v as i64),
                row.registered_at.to_rfc3339(),
                row.last_used_at.map(|dt| dt.to_rfc3339()),
            ],
        ).map_err(sqlite_err)?;

        if let Some(obs) = &self.observer {
            obs.on_model_registered(&row.id);
        }
        Ok(())
    }

    /// Fetch a model row by ID.
    pub fn get_model(&self, id: &ModelId) -> Result<ModelRow> {
        let conn = self.conn()?;
        conn.query_row(
            "SELECT id, source, prompt_template, context_length, n_gpu_layers, max_slots,
                    is_downloaded, is_loaded, file_path, file_bytes, registered_at, last_used_at
             FROM models WHERE id = ?1",
            params![id],
            map_model_row,
        ).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => SubstrateError::ModelNotFound(id.clone()),
            other => sqlite_err(other),
        })
    }

    /// List all registered models.
    pub fn list_models(&self) -> Result<Vec<ModelRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, prompt_template, context_length, n_gpu_layers, max_slots,
                    is_downloaded, is_loaded, file_path, file_bytes, registered_at, last_used_at
             FROM models ORDER BY id",
        ).map_err(sqlite_err)?;

        let rows = stmt
            .query_map([], map_model_row)
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(rows)
    }

    /// Mark a model as fully downloaded. Sets file_path, file_bytes, and is_downloaded = true.
    pub fn set_model_downloaded(&self, id: &ModelId, path: &str, bytes: u64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE models SET is_downloaded = 1, file_path = ?1, file_bytes = ?2 WHERE id = ?3",
            params![path, bytes as i64, id],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Mark a model as currently loaded in the engine.
    pub fn set_model_loaded(&self, id: &ModelId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE models SET is_loaded = 1, last_used_at = ?1 WHERE id = ?2",
            params![now, id],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Mark a model as unloaded (engine no longer running it).
    pub fn set_model_unloaded(&self, id: &ModelId) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE models SET is_loaded = 0 WHERE id = ?1",
            params![id],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Update `last_used_at` for LRU eviction ordering.
    pub fn touch_model(&self, id: &ModelId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE models SET last_used_at = ?1 WHERE id = ?2",
            params![now, id],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Clear the file_path and file_bytes fields (model file was evicted from disk).
    pub fn clear_model_file(&self, id: &ModelId) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE models SET is_downloaded = 0, file_path = NULL, file_bytes = NULL WHERE id = ?1",
            params![id],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Return total bytes of all model weight files on disk.
    pub fn total_weights_bytes(&self) -> Result<u64> {
        let conn = self.conn()?;
        let total: i64 = conn.query_row(
            "SELECT COALESCE(SUM(file_bytes), 0) FROM models WHERE file_path IS NOT NULL",
            [],
            |row| row.get(0),
        ).map_err(sqlite_err)?;
        Ok(total as u64)
    }

    /// Return models eligible for disk eviction, ordered by LRU (least recently used first).
    /// Only returns models that have a file on disk and are not currently loaded.
    pub fn models_for_eviction(&self) -> Result<Vec<ModelRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, source, prompt_template, context_length, n_gpu_layers, max_slots,
                    is_downloaded, is_loaded, file_path, file_bytes, registered_at, last_used_at
             FROM models
             WHERE file_path IS NOT NULL AND is_loaded = 0
             ORDER BY last_used_at ASC NULLS FIRST",
        ).map_err(sqlite_err)?;

        let rows = stmt
            .query_map([], map_model_row)
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(rows)
    }
}
