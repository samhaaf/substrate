//! Wiki read-only endpoints: `/wiki/`
//!
//! These endpoints expose structured knowledge about the node's state for
//! inspection and tooling. They are read-only and have no side effects.
//!
//! - `GET /wiki/models`     — model registry with download/load state
//! - `GET /wiki/queue`      — pending completion counts by model and priority
//! - `GET /wiki/benchmarks` — recent benchmark run history

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};

use crate::ApiState;

/// Mount all wiki routes onto a router.
pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/wiki/models", get(wiki_models))
        .route("/wiki/queue", get(wiki_queue))
        .route("/wiki/benchmarks", get(wiki_benchmarks))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /wiki/models — model registry with download and load state.
pub async fn wiki_models(State(state): State<ApiState>) -> impl IntoResponse {
    match state.store.list_models() {
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Ok(models) => {
            let list: Vec<_> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "source": m.source,
                        "context_length": m.context_length,
                        "n_gpu_layers": m.n_gpu_layers,
                        "max_slots": m.max_slots,
                        "is_downloaded": m.is_downloaded,
                        "is_loaded": m.is_loaded,
                        "file_bytes": m.file_bytes,
                        "file_path": m.file_path,
                        "registered_at": m.registered_at.to_rfc3339(),
                        "last_used_at": m.last_used_at.map(|t| t.to_rfc3339()),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(list))).into_response()
        }
    }
}

/// GET /wiki/queue — current queue state: counts grouped by completion state.
pub async fn wiki_queue(State(state): State<ApiState>) -> impl IntoResponse {
    match state.store.count_by_state() {
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
        Ok(counts) => {
            // Build a map from state name -> count.
            let mut by_state = serde_json::Map::new();
            let mut total: u64 = 0;
            let mut pending: u64 = 0;
            let mut running: u64 = 0;

            for (state, count) in &counts {
                by_state.insert(state.as_str().to_owned(), serde_json::json!(count));
                total += count;
                match state {
                    substrate_types::CompletionState::Pending => pending += count,
                    substrate_types::CompletionState::Running => running += count,
                    _ => {}
                }
            }

            let body = serde_json::json!({
                "total": total,
                "pending": pending,
                "running": running,
                "by_state": by_state,
            });
            (StatusCode::OK, Json(body)).into_response()
        }
    }
}

/// GET /wiki/benchmarks — benchmark run history across all models.
pub async fn wiki_benchmarks(State(state): State<ApiState>) -> impl IntoResponse {
    // List all models, then collect benchmark runs for each.
    let models = match state.store.list_models() {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let mut results = Vec::new();
    for model in &models {
        let runs = state
            .store
            .benchmark_runs_for_model(&model.id)
            .unwrap_or_default();

        if runs.is_empty() {
            continue;
        }

        let serialized: Vec<_> = runs
            .iter()
            .map(|r| {
                serde_json::json!({
                    "id": r.id,
                    "model_id": r.model_id,
                    "prompt_tokens": r.prompt_tokens,
                    "max_tokens": r.max_tokens,
                    "concurrency": r.concurrency,
                    "tokens_per_second": r.tokens_per_second,
                    "wall_time_ms": r.wall_time_ms,
                    "recorded_at": r.recorded_at.to_rfc3339(),
                })
            })
            .collect();

        results.push(serde_json::json!({
            "model_id": model.id,
            "run_count": serialized.len(),
            "runs": serialized,
        }));
    }

    (StatusCode::OK, Json(serde_json::json!(results))).into_response()
}
