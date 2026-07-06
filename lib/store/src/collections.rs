//! Collection persistence: CRUD over the `collections` table.

use chrono::Utc;
use rusqlite::params;

use substrate_types::{
    CollectionId, CollectionRow, CollectionState, CompletionId, Result, SubstrateError,
};

use crate::{sqlite_err, Store};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_uuid(s: &str) -> uuid::Uuid {
    uuid::Uuid::parse_str(s).unwrap_or(uuid::Uuid::nil())
}

fn parse_dt(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&chrono::Utc))
        .unwrap_or_default()
}

fn state_as_str(state: CollectionState) -> &'static str {
    match state {
        CollectionState::Active    => "active",
        CollectionState::Completed => "completed",
        CollectionState::Failed    => "failed",
        CollectionState::Cancelled => "cancelled",
    }
}

fn state_from_str(s: &str) -> CollectionState {
    CollectionState::from_str_lossy(s).unwrap_or(CollectionState::Active)
}

/// Column order must match every SELECT that uses this mapper:
///   0: id, 1: name, 2: description, 3: cancel_on_failure,
///   4: request_full_system, 5: start_with_no_model_loaded,
///   6: save_partial_results, 7: metadata_json, 8: state,
///   9: created_at, 10: started_at, 11: completed_at
fn map_collection_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CollectionRow> {
    Ok(CollectionRow {
        id:                        parse_uuid(&row.get::<_, String>(0)?),
        name:                      row.get(1)?,
        description:               row.get(2)?,
        cancel_on_failure:         row.get::<_, i64>(3)? != 0,
        request_full_system:       row.get::<_, i64>(4)? != 0,
        start_with_no_model_loaded: row.get::<_, i64>(5)? != 0,
        save_partial_results:      row.get::<_, i64>(6)? != 0,
        metadata_json:             row.get(7)?,
        state:                     state_from_str(&row.get::<_, String>(8)?),
        created_at:                parse_dt(&row.get::<_, String>(9)?),
        started_at:                row.get::<_, Option<String>>(10)?
                                       .as_deref()
                                       .map(parse_dt),
        completed_at:              row.get::<_, Option<String>>(11)?
                                       .as_deref()
                                       .map(parse_dt),
    })
}

const SELECT_COLS: &str =
    "id, name, description, cancel_on_failure, request_full_system, start_with_no_model_loaded,
     save_partial_results, metadata_json, state, created_at, started_at, completed_at";

// ---------------------------------------------------------------------------
// Store impl
// ---------------------------------------------------------------------------

impl Store {
    /// Insert a new collection in Active state.
    ///
    /// `row.id` is used as-is. `row.state` is ignored; always inserted as `active`.
    pub fn insert_collection(&self, row: &CollectionRow) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO collections
             (id, name, description, cancel_on_failure, request_full_system,
              start_with_no_model_loaded, save_partial_results, metadata_json,
              state, created_at, started_at, completed_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'active',?9,NULL,NULL)",
            params![
                row.id.to_string(),
                row.name,
                row.description,
                row.cancel_on_failure as i64,
                row.request_full_system as i64,
                row.start_with_no_model_loaded as i64,
                row.save_partial_results as i64,
                row.metadata_json,
                row.created_at.to_rfc3339(),
            ],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Fetch a collection row by ID.
    pub fn get_collection(&self, id: CollectionId) -> Result<CollectionRow> {
        let conn = self.conn()?;
        conn.query_row(
            &format!("SELECT {SELECT_COLS} FROM collections WHERE id = ?1"),
            params![id.to_string()],
            map_collection_row,
        ).map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => SubstrateError::CollectionNotFound(id),
            other => sqlite_err(other),
        })
    }

    /// Update a collection's state.
    ///
    /// Records `started_at` on transition to Active-and-running (if not already set).
    /// Records `completed_at` on terminal transitions.
    pub fn set_collection_state(&self, id: CollectionId, state: CollectionState) -> Result<()> {
        let conn = self.conn()?;
        match state {
            CollectionState::Completed | CollectionState::Failed | CollectionState::Cancelled => {
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "UPDATE collections SET state = ?1, completed_at = ?2 WHERE id = ?3",
                    params![state_as_str(state), now, id.to_string()],
                ).map_err(sqlite_err)?;
            }
            CollectionState::Active => {
                // Mark started_at the first time we go to active-running.
                let now = Utc::now().to_rfc3339();
                conn.execute(
                    "UPDATE collections
                     SET state = ?1, started_at = COALESCE(started_at, ?2)
                     WHERE id = ?3",
                    params![state_as_str(state), now, id.to_string()],
                ).map_err(sqlite_err)?;
            }
        }
        Ok(())
    }

    /// Cancel a collection and all its pending/running completions.
    pub fn cancel_collection(&self, id: CollectionId) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn()?;

        conn.execute(
            "UPDATE collections SET state = 'cancelled', completed_at = ?1 WHERE id = ?2",
            params![now, id.to_string()],
        ).map_err(sqlite_err)?;

        conn.execute(
            "UPDATE completions SET state = 'cancelled', completed_at = ?1
             WHERE collection_id = ?2 AND state IN ('pending', 'running')",
            params![now, id.to_string()],
        ).map_err(sqlite_err)?;

        Ok(())
    }

    /// Return the IDs of all completions belonging to a collection.
    pub fn collection_member_ids(&self, id: CollectionId) -> Result<Vec<CompletionId>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id FROM completions WHERE collection_id = ?1",
        ).map_err(sqlite_err)?;

        let ids = stmt
            .query_map(params![id.to_string()], |row| {
                Ok(parse_uuid(&row.get::<_, String>(0)?))
            })
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(ids)
    }

    /// Return (total, completed, failed, cancelled) counts for a collection.
    pub fn collection_member_counts(&self, id: CollectionId) -> Result<(u32, u32, u32, u32)> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT state, COUNT(*) FROM completions WHERE collection_id = ?1 GROUP BY state",
        ).map_err(sqlite_err)?;

        let mut total: u32 = 0;
        let mut completed: u32 = 0;
        let mut failed: u32 = 0;
        let mut cancelled: u32 = 0;

        let rows = stmt
            .query_map(params![id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u32))
            })
            .map_err(sqlite_err)?;

        for r in rows {
            let (state_str, count) = r.map_err(sqlite_err)?;
            total += count;
            match state_str.as_str() {
                "completed" => completed += count,
                "failed"    => failed += count,
                "cancelled" => cancelled += count,
                _           => {}
            }
        }

        Ok((total, completed, failed, cancelled))
    }
}
