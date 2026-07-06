//! Benchmark run persistence: the `benchmark_runs` table.
//!
//! Each benchmark sweep point produces one row. The telemetry estimator queries
//! these rows to fit its polynomial throughput model.

use rusqlite::params;

use substrate_types::{CollectionId, ModelId, Result};

use crate::{sqlite_err, Store};

/// A single benchmark run record.
#[derive(Debug, Clone)]
pub struct BenchmarkRun {
    /// Auto-assigned row ID (0 before insertion).
    pub id: i64,
    /// The model being benchmarked.
    pub model_id: ModelId,
    /// The collection this run belongs to (benchmark sweep collection).
    pub collection_id: Option<CollectionId>,
    /// Number of prompt tokens in the benchmark completion.
    pub prompt_tokens: u32,
    /// Max tokens requested (generation target).
    pub max_tokens: u32,
    /// Number of concurrent completions running during this measurement.
    pub concurrency: u32,
    /// Measured throughput in tokens/second (None if the run did not complete).
    pub tokens_per_second: Option<f32>,
    /// Wall clock time of the completion in milliseconds.
    pub wall_time_ms: Option<u64>,
    /// When this data point was recorded.
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Store impl
// ---------------------------------------------------------------------------

impl Store {
    /// Insert a benchmark run record. Returns the auto-assigned row ID.
    pub fn insert_benchmark_run(&self, run: &BenchmarkRun) -> Result<i64> {
        let conn = self.conn()?;
        let now = run.recorded_at.to_rfc3339();
        conn.execute(
            "INSERT INTO benchmark_runs
             (model_id, collection_id, prompt_tokens, max_tokens, concurrency,
              tokens_per_second, wall_time_ms, recorded_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                run.model_id,
                run.collection_id.map(|c| c.to_string()),
                run.prompt_tokens as i64,
                run.max_tokens as i64,
                run.concurrency as i64,
                run.tokens_per_second.map(|v| v as f64),
                run.wall_time_ms.map(|v| v as i64),
                now,
            ],
        ).map_err(sqlite_err)?;

        let row_id = conn.last_insert_rowid();

        if let Some(obs) = &self.observer {
            obs.on_benchmark_recorded(&run.model_id, row_id);
        }

        Ok(row_id)
    }

    /// Return all benchmark runs for a given model, newest first.
    pub fn benchmark_runs_for_model(&self, model_id: &ModelId) -> Result<Vec<BenchmarkRun>> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, model_id, collection_id, prompt_tokens, max_tokens, concurrency,
                    tokens_per_second, wall_time_ms, recorded_at
             FROM benchmark_runs
             WHERE model_id = ?1
             ORDER BY recorded_at DESC",
        ).map_err(sqlite_err)?;

        let rows = stmt
            .query_map(params![model_id], map_benchmark_row)
            .map_err(sqlite_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(sqlite_err)?;
        Ok(rows)
    }

    /// Return the total number of benchmark runs for a model.
    pub fn benchmark_count(&self, model_id: &ModelId) -> Result<u64> {
        let conn = self.conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM benchmark_runs WHERE model_id = ?1",
            params![model_id],
            |row| row.get(0),
        ).map_err(sqlite_err)?;
        Ok(count as u64)
    }
}

fn map_benchmark_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BenchmarkRun> {
    let collection_id_str: Option<String> = row.get(2)?;
    let recorded_at_str: String = row.get(8)?;

    Ok(BenchmarkRun {
        id:               row.get(0)?,
        model_id:         row.get(1)?,
        collection_id:    collection_id_str.as_deref()
                            .and_then(|s| uuid::Uuid::parse_str(s).ok()),
        prompt_tokens:    row.get::<_, i64>(3)? as u32,
        max_tokens:       row.get::<_, i64>(4)? as u32,
        concurrency:      row.get::<_, i64>(5)? as u32,
        tokens_per_second: row.get::<_, Option<f64>>(6)?.map(|v| v as f32),
        wall_time_ms:     row.get::<_, Option<i64>>(7)?.map(|v| v as u64),
        recorded_at:      chrono::DateTime::parse_from_rfc3339(&recorded_at_str)
                            .map(|d| d.with_timezone(&chrono::Utc))
                            .unwrap_or_default(),
    })
}
