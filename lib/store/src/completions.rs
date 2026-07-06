//! Completion persistence: CRUD + queue view over the `completions` table.

use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

use substrate_types::{
    CompletionId, CompletionRow, CompletionState, ModelId, Result, SubstrateError,
};

use crate::{sqlite_err, Store};

// ---------------------------------------------------------------------------
// Helpers shared within this module
// ---------------------------------------------------------------------------

fn parse_uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap_or(Uuid::nil())
}

fn parse_dt(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_default()
}

fn state_from_str(s: &str) -> CompletionState {
    CompletionState::from_str_lossy(s).unwrap_or(CompletionState::Pending)
}

/// Column order must match every SELECT that uses this mapper:
///   0: id
///   1: model_id
///   2: prompt
///   3: params_json
///   4: priority
///   5: preemption_threshold
///   6: json_schema
///   7: collection_id
///   8: metrics_flags
///   9: metadata_json
///  10: state
///  11: preemption_count
///  12: error_retry_count
///  13: created_at
///  14: started_at
///  15: completed_at
///  16: recovered_at
fn map_completion_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompletionRow> {
    Ok(CompletionRow {
        id:                  parse_uuid(&row.get::<_, String>(0)?),
        model_id:            row.get(1)?,
        prompt:              row.get(2)?,
        params_json:         row.get(3)?,
        priority:            row.get(4)?,
        preemption_threshold: row.get::<_, i64>(5)? as i32,
        json_schema:         row.get(6)?,
        collection_id:       row.get::<_, Option<String>>(7)?
                                 .as_deref()
                                 .map(parse_uuid),
        metrics_flags:       row.get::<_, i64>(8)? as u32,
        metadata_json:       row.get(9)?,
        state:               state_from_str(&row.get::<_, String>(10)?),
        preemption_count:    row.get::<_, i64>(11)? as u32,
        error_retry_count:   row.get::<_, i64>(12)? as u32,
        created_at:          parse_dt(&row.get::<_, String>(13)?),
        started_at:          row.get::<_, Option<String>>(14)?
                                 .as_deref()
                                 .map(parse_dt),
        completed_at:        row.get::<_, Option<String>>(15)?
                                 .as_deref()
                                 .map(parse_dt),
        recovered_at:        row.get::<_, Option<String>>(16)?
                                 .as_deref()
                                 .map(parse_dt),
    })
}

const SELECT_COLS: &str =
    "id, model_id, prompt, params_json, priority, preemption_threshold, json_schema,
     collection_id, metrics_flags, metadata_json, state, preemption_count, error_retry_count,
     created_at, started_at, completed_at, recovered_at";

// ---------------------------------------------------------------------------
// Store impl
// ---------------------------------------------------------------------------

impl Store {
    /// Insert a new completion in Pending state.
    ///
    /// The `row.id` is used as-is — callers must assign the ID before calling.
    /// `row.state` is ignored; the row is always inserted as `pending`.
    pub fn insert_completion(&self, row: &CompletionRow) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO completions
             (id, model_id, prompt, params_json, priority, preemption_threshold, json_schema,
              collection_id, metrics_flags, metadata_json, state, preemption_count,
              error_retry_count, created_at, started_at, completed_at, recovered_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'pending',0,0,?11,NULL,NULL,NULL)",
            params![
                row.id.to_string(),
                row.model_id,
                row.prompt,
                row.params_json,
                row.priority,
                row.preemption_threshold,
                row.json_schema,
                row.collection_id.map(|c| c.to_string()),
                row.metrics_flags as i64,
                row.metadata_json,
                row.created_at.to_rfc3339(),
            ],
        ).map_err(sqlite_err)?;

        if let Some(obs) = &self.observer {
            obs.on_completion_inserted(row.id);
        }
        Ok(())
    }

    /// Fetch a completion row by ID.
    pub fn get_completion(&self, id: CompletionId) -> Result<CompletionRow> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {SELECT_COLS} FROM completions WHERE id = ?1"),
            params![id.to_string()],
            map_completion_row,
        ).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => SubstrateError::CompletionNotFound(id),
            other => sqlite_err(other),
        })
    }

    /// Transition a completion from Pending to Running. Records `started_at`.
    pub fn mark_running(&self, id: CompletionId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE completions SET state = 'running', started_at = ?1
             WHERE id = ?2 AND state = 'pending'",
            params![now, id.to_string()],
        ).map_err(sqlite_err)?;
        if let Some(obs) = &self.observer {
            obs.on_completion_state_changed(id, CompletionState::Pending, CompletionState::Running);
        }
        Ok(())
    }

    /// Transition a completion to Completed. Records `completed_at`.
    pub fn mark_completed(&self, id: CompletionId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE completions SET state = 'completed', completed_at = ?1 WHERE id = ?2",
            params![now, id.to_string()],
        ).map_err(sqlite_err)?;
        if let Some(obs) = &self.observer {
            obs.on_completion_state_changed(id, CompletionState::Running, CompletionState::Completed);
        }
        Ok(())
    }

    /// Transition a completion to Failed. Records `completed_at`.
    /// `_reason` is informational only; persist the full reason via `insert_result`.
    pub fn mark_failed(&self, id: CompletionId, _reason: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE completions SET state = 'failed', completed_at = ?1 WHERE id = ?2",
            params![now, id.to_string()],
        ).map_err(sqlite_err)?;
        if let Some(obs) = &self.observer {
            obs.on_completion_state_changed(id, CompletionState::Running, CompletionState::Failed);
        }
        Ok(())
    }

    /// Transition a completion to Cancelled. Records `completed_at`.
    pub fn mark_cancelled(&self, id: CompletionId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;
        conn.execute(
            "UPDATE completions SET state = 'cancelled', completed_at = ?1 WHERE id = ?2",
            params![now, id.to_string()],
        ).map_err(sqlite_err)?;
        if let Some(obs) = &self.observer {
            obs.on_completion_state_changed(id, CompletionState::Running, CompletionState::Cancelled);
        }
        Ok(())
    }

    /// Move a Running completion back to Pending (preemption). Increments preemption_count.
    pub fn requeue_preempted(&self, id: CompletionId) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE completions
             SET state = 'pending',
                 started_at = NULL,
                 preemption_count = preemption_count + 1
             WHERE id = ?1",
            params![id.to_string()],
        ).map_err(sqlite_err)?;
        if let Some(obs) = &self.observer {
            obs.on_completion_state_changed(id, CompletionState::Running, CompletionState::Pending);
        }
        Ok(())
    }

    /// Move a Failed completion back to Pending for retry. Increments error_retry_count.
    pub fn requeue_error_retry(&self, id: CompletionId) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE completions
             SET state = 'pending',
                 started_at = NULL,
                 completed_at = NULL,
                 error_retry_count = error_retry_count + 1
             WHERE id = ?1",
            params![id.to_string()],
        ).map_err(sqlite_err)?;
        if let Some(obs) = &self.observer {
            obs.on_completion_state_changed(id, CompletionState::Failed, CompletionState::Pending);
        }
        Ok(())
    }

    /// Update a completion's priority in place.
    pub fn update_priority(&self, id: CompletionId, priority: i32) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE completions SET priority = ?1 WHERE id = ?2",
            params![priority, id.to_string()],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Return pending completions ordered by (priority DESC, created_at ASC).
    /// The scheduler calls this on every loop tick.
    pub fn select_pending(&self) -> Result<Vec<CompletionRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM completions
             WHERE state = 'pending'
             ORDER BY priority DESC, created_at ASC",
        )).map_err(sqlite_err)?;

        let rows = stmt
            .query_map([], map_completion_row)
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(rows)
    }

    /// Return pending completions filtered to a specific model.
    pub fn select_pending_for_model(&self, model_id: &ModelId) -> Result<Vec<CompletionRow>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM completions
             WHERE state = 'pending' AND model_id = ?1
             ORDER BY priority DESC, created_at ASC",
        )).map_err(sqlite_err)?;

        let rows = stmt
            .query_map(params![model_id], map_completion_row)
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(rows)
    }

    /// Count completions grouped by state (for system state reporting).
    pub fn count_by_state(&self) -> Result<Vec<(CompletionState, u64)>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT state, COUNT(*) FROM completions GROUP BY state",
        ).map_err(sqlite_err)?;

        let pairs = stmt
            .query_map([], |row| {
                let state_str: String = row.get(0)?;
                let count: i64 = row.get(1)?;
                Ok((state_from_str(&state_str), count as u64))
            })
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(pairs)
    }

    /// Return completions filtered by a set of states, ordered by priority DESC then created_at ASC.
    ///
    /// Pass an empty `states` slice to return completions in any state.
    /// `limit` caps the number of rows returned (max 1000 to protect the caller).
    /// Return completions filtered by a set of states, ordered by priority DESC then created_at ASC.
    ///
    /// Pass an empty `states` slice to return completions in any state.
    /// `limit` caps the number of rows returned (max 1000 to protect the caller).
    pub fn list_completions_by_state(
        &self,
        states: &[CompletionState],
        limit: u32,
    ) -> Result<Vec<CompletionRow>> {
        let limit = limit.min(1000);
        let conn = self.conn()?;

        if states.is_empty() {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLS} FROM completions
                 ORDER BY priority DESC, created_at ASC
                 LIMIT ?1",
            )).map_err(sqlite_err)?;
            let rows = stmt.query_map(params![limit as i64], map_completion_row)
                .map_err(sqlite_err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(sqlite_err)?;
            return Ok(rows);
        }

        // Embed state enum values as SQL literals (safe: values are controlled enum strings,
        // never user-supplied text). This sidesteps rusqlite borrow-checker friction when
        // building a variable-length params slice.
        let state_literals: Vec<String> =
            states.iter().map(|s| format!("'{}'", s.as_str())).collect();
        let sql = format!(
            "SELECT {SELECT_COLS} FROM completions
             WHERE state IN ({})
             ORDER BY priority DESC, created_at ASC
             LIMIT {}",
            state_literals.join(", "),
            limit as i64,
        );

        let mut stmt = conn.prepare(&sql).map_err(sqlite_err)?;
        let rows = stmt.query_map([], map_completion_row)
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(rows)
    }

    /// Return the minimum preemption_count among currently running completions.
    /// Returns None if no completions are running.
    pub fn select_running_min_preemption_count(&self) -> Result<Option<u32>> {
        let conn = self.conn()?;
        let result: Option<i64> = conn.query_row(
            "SELECT MIN(preemption_count) FROM completions WHERE state = 'running'",
            [],
            |row| row.get(0),
        ).map_err(sqlite_err)?;
        Ok(result.map(|v| v as u32))
    }
}
