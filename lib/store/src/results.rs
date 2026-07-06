//! Completion result persistence: the `results` table stores output blobs.
//!
//! Results are stored separately from completions to keep the completions table
//! narrow (the queue view only scans completions, not results).

use chrono::Utc;
use rusqlite::params;

use substrate_types::{
    CollectionId, CompletionId, CompletionMetrics, CompletionResult, CompletionState,
    Result,
};

use crate::{json_err, sqlite_err, Store};

// ---------------------------------------------------------------------------
// Store impl
// ---------------------------------------------------------------------------

impl Store {
    /// Insert a completion result.
    ///
    /// Serializes `termination` and `metrics` to JSON for storage.
    pub fn insert_result(&self, result: &CompletionResult) -> Result<()> {
        let conn = self.conn()?;
        let now = Utc::now().to_rfc3339();
        let termination_json = result.termination
            .as_ref()
            .map(|t| serde_json::to_string(t))
            .transpose()
            .map_err(json_err)?;
        let metrics_json = serde_json::to_string(&result.metrics).map_err(json_err)?;

        conn.execute(
            "INSERT OR REPLACE INTO results
             (completion_id, text, termination_json, metrics_json, stored_at)
             VALUES (?1,?2,?3,?4,?5)",
            params![
                result.id.to_string(),
                result.text,
                termination_json,
                metrics_json,
                now,
            ],
        ).map_err(sqlite_err)?;
        Ok(())
    }

    /// Fetch a completion result by completion ID.
    /// Returns `None` if no result has been stored yet.
    pub fn get_result(&self, id: CompletionId) -> Result<Option<CompletionResult>> {
        let conn = self.conn()?;
        let result = conn.query_row(
            "SELECT r.completion_id, r.text, r.termination_json, r.metrics_json, c.state, c.completed_at
             FROM results r
             JOIN completions c ON c.id = r.completion_id
             WHERE r.completion_id = ?1",
            params![id.to_string()],
            |row| {
                let id_str: String = row.get(0)?;
                let text: Option<String> = row.get(1)?;
                let term_json: Option<String> = row.get(2)?;
                let metrics_json: Option<String> = row.get(3)?;
                let state_str: String = row.get(4)?;
                let completed_at_str: Option<String> = row.get(5)?;
                Ok((id_str, text, term_json, metrics_json, state_str, completed_at_str))
            },
        );

        match result {
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(sqlite_err(e)),
            Ok((id_str, text, term_json, metrics_json, state_str, completed_at_str)) => {
                let parsed_id = uuid::Uuid::parse_str(&id_str)
                    .unwrap_or(uuid::Uuid::nil());

                let termination = term_json
                    .as_deref()
                    .map(|s| serde_json::from_str(s))
                    .transpose()
                    .unwrap_or(None);

                let metrics: CompletionMetrics = metrics_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();

                let state = match state_str.as_str() {
                    "completed" => CompletionState::Completed,
                    "failed"    => CompletionState::Failed,
                    "cancelled" => CompletionState::Cancelled,
                    _           => CompletionState::Completed,
                };

                let completed_at = completed_at_str
                    .as_deref()
                    .map(|s| chrono::DateTime::parse_from_rfc3339(s)
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_default())
                    .unwrap_or_else(chrono::Utc::now);

                Ok(Some(CompletionResult {
                    id: parsed_id,
                    state,
                    text,
                    termination,
                    metrics,
                    completed_at,
                }))
            }
        }
    }

    /// Delete all results belonging to completions in a given collection.
    /// Called when a collection is deleted for storage reclamation.
    pub fn delete_collection_results(&self, collection_id: CollectionId) -> Result<u64> {
        let conn = self.conn()?;
        let deleted = conn.execute(
            "DELETE FROM results WHERE completion_id IN
             (SELECT id FROM completions WHERE collection_id = ?1)",
            params![collection_id.to_string()],
        ).map_err(sqlite_err)? as u64;
        Ok(deleted)
    }
}
