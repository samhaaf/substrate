//! REST API handlers.
//!
//! All handlers follow the Axum pattern: extract `State<ApiState>` and typed
//! path/query/body parameters, then delegate to the scheduler or store.
//!
//! Error mapping:
//! - `SubstrateError::CompletionNotFound` / `CollectionNotFound` / `ModelNotFound` → 404
//! - `SubstrateError::CompletionTerminal` / `CollectionTerminal` → 409 Conflict
//! - `SubstrateError::InvalidRequest` / `EmptyCollection` → 422
//! - Everything else → 500

use std::sync::{atomic::Ordering, Arc};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, patch, post},
    Json, Router,
};
use serde::Deserialize;
use substrate_types::{
    CollectionId, CollectionOptions, CollectionRow, CollectionState, CompletionId,
    CompletionRequest, CompletionState, SubstrateError,
};
use uuid::Uuid;

use crate::ApiState;

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

/// Convert a SubstrateError into an appropriate (StatusCode, JSON) response.
fn err_response(e: SubstrateError) -> (StatusCode, Json<serde_json::Value>) {
    let status = match &e {
        SubstrateError::CompletionNotFound(_)
        | SubstrateError::CollectionNotFound(_)
        | SubstrateError::ModelNotFound(_) => StatusCode::NOT_FOUND,

        SubstrateError::CompletionTerminal { .. } | SubstrateError::CollectionTerminal { .. } => {
            StatusCode::CONFLICT
        }

        SubstrateError::InvalidRequest(_) | SubstrateError::EmptyCollection => {
            StatusCode::UNPROCESSABLE_ENTITY
        }

        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::json!({ "error": e.to_string() })))
}

// ---------------------------------------------------------------------------
// Route table
// ---------------------------------------------------------------------------

/// Mount all REST routes onto a router.
pub fn routes() -> Router<ApiState> {
    Router::new()
        // Completions
        .route("/v1/completions", get(list_completions))
        .route("/v1/completions", post(submit_completion))
        .route("/v1/completions/:id", get(get_completion))
        .route("/v1/completions/:id", delete(cancel_completion))
        .route("/v1/completions/:id/priority", patch(update_priority))
        .route("/v1/completions/:id/result", get(get_result))
        // Collections
        .route("/v1/collections", post(create_collection))
        .route("/v1/collections/:id", get(get_collection))
        .route("/v1/collections/:id", delete(cancel_collection))
        // Models
        .route("/v1/models", get(list_models))
        .route("/v1/models/:id/download", post(download_model))
        // Estimation
        .route("/v1/estimate", post(estimate))
        // Benchmarks
        .route("/v1/benchmark/kernel", get(benchmark_kernel))
        .route("/v1/benchmark/run", post(benchmark_run))
        // Execution control
        .route("/v1/execution/pause", post(pause_handler))
        .route("/v1/execution/resume", post(resume_handler))
        // System / observability
        .route("/v1/system/state", get(system_state))
        .route("/metrics", get(system_state))
        .route("/health", get(health))
}

// ---------------------------------------------------------------------------
// Completions
// ---------------------------------------------------------------------------

/// Query parameters for `GET /v1/completions`.
#[derive(Deserialize, Default)]
pub struct ListCompletionsQuery {
    /// Comma-separated state names to filter by (e.g. "pending,running").
    /// If omitted, all states are returned.
    state: Option<String>,
    /// Maximum number of rows to return (default 50, capped at 1000).
    limit: Option<u32>,
}

/// GET /v1/completions — list completions, optionally filtered by state.
pub async fn list_completions(
    State(state): State<ApiState>,
    Query(params): Query<ListCompletionsQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(50);

    // Parse comma-separated state names.
    let states: Vec<CompletionState> = params
        .state
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| CompletionState::from_str_lossy(s.trim()))
        .collect();

    match state.store.list_completions_by_state(&states, limit) {
        Ok(rows) => {
            let list: Vec<_> = rows
                .iter()
                .map(|row| {
                    serde_json::json!({
                        "id": row.id,
                        "model_id": row.model_id,
                        "state": row.state.as_str(),
                        "priority": row.priority,
                        "preemption_threshold": row.preemption_threshold,
                        "preemption_count": row.preemption_count,
                        "error_retry_count": row.error_retry_count,
                        "collection_id": row.collection_id,
                        "created_at": row.created_at.to_rfc3339(),
                        "started_at": row.started_at.map(|t| t.to_rfc3339()),
                        "completed_at": row.completed_at.map(|t| t.to_rfc3339()),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(list))).into_response()
        }
        Err(e) => err_response(e).into_response(),
    }
}

/// POST /v1/completions — submit a new completion.
///
/// The caller may omit `id`; if nil it will be assigned here.
/// Returns 201 Created with `{ "id": "<uuid>" }`.
pub async fn submit_completion(
    State(state): State<ApiState>,
    Json(mut req): Json<CompletionRequest>,
) -> impl IntoResponse {
    // Assign ID and timestamp if not pre-set by the caller.
    if req.id == Uuid::nil() {
        req.id = CompletionRequest::new_id();
    }
    if req.created_at == chrono::DateTime::<chrono::Utc>::default() {
        req.created_at = chrono::Utc::now();
    }

    match state.scheduler.submit(req) {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => err_response(e).into_response(),
    }
}

/// GET /v1/completions/:id — get completion status.
pub async fn get_completion(
    State(state): State<ApiState>,
    Path(id): Path<CompletionId>,
) -> impl IntoResponse {
    match state.store.get_completion(id) {
        Ok(row) => {
            let body = serde_json::json!({
                "id": row.id,
                "model_id": row.model_id,
                "state": row.state.as_str(),
                "priority": row.priority,
                "preemption_threshold": row.preemption_threshold,
                "preemption_count": row.preemption_count,
                "error_retry_count": row.error_retry_count,
                "collection_id": row.collection_id,
                "created_at": row.created_at.to_rfc3339(),
                "started_at": row.started_at.map(|t| t.to_rfc3339()),
                "completed_at": row.completed_at.map(|t| t.to_rfc3339()),
            });
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(e) => err_response(e).into_response(),
    }
}

/// DELETE /v1/completions/:id — cancel a completion.
///
/// Returns 204 No Content on success.
/// Returns 409 if already in a terminal state.
pub async fn cancel_completion(
    State(state): State<ApiState>,
    Path(id): Path<CompletionId>,
) -> impl IntoResponse {
    match state.scheduler.cancel(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_response(e).into_response(),
    }
}

/// GET /v1/completions/:id/result — fetch the result of a terminal completion.
///
/// Returns 409 if the completion is not yet in a terminal state.
/// Returns 404 if no result blob is stored (shouldn't happen for terminal completions
/// that completed normally, but can happen for cancelled/failed without a result).
pub async fn get_result(
    State(state): State<ApiState>,
    Path(id): Path<CompletionId>,
) -> impl IntoResponse {
    // Verify existence and terminal state first.
    match state.store.get_completion(id) {
        Err(e) => return err_response(e).into_response(),
        Ok(row) if !row.state.is_terminal() => {
            return (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error": format!(
                        "completion {} is not yet terminal (state: {})",
                        id,
                        row.state.as_str()
                    )
                })),
            )
                .into_response();
        }
        Ok(_) => {}
    }

    match state.store.get_result(id) {
        Ok(Some(result)) => (StatusCode::OK, Json(serde_json::json!(result))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "no result stored for this completion" })),
        )
            .into_response(),
        Err(e) => err_response(e).into_response(),
    }
}

/// PATCH /v1/completions/:id/priority — update scheduling priority in place.
///
/// Body: `{ "priority": <i32> }`
/// Returns 204 on success. Returns 409 if the completion is already terminal.
#[derive(Deserialize)]
pub(crate) struct PriorityBody {
    priority: i32,
}

pub(crate) async fn update_priority(
    State(state): State<ApiState>,
    Path(id): Path<CompletionId>,
    Json(body): Json<PriorityBody>,
) -> impl IntoResponse {
    // Reject mutation on terminal completions.
    match state.store.get_completion(id) {
        Err(e) => return err_response(e).into_response(),
        Ok(row) if row.state.is_terminal() => {
            return err_response(SubstrateError::CompletionTerminal {
                id,
                state: row.state,
            })
            .into_response();
        }
        Ok(_) => {}
    }

    match state.store.update_priority(id, body.priority) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_response(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Collections
// ---------------------------------------------------------------------------

/// POST /v1/collections — create a new collection.
///
/// Body: `CollectionOptions` JSON.
/// Returns 201 with `{ "id": "<uuid>" }`.
pub async fn create_collection(
    State(state): State<ApiState>,
    Json(opts): Json<CollectionOptions>,
) -> impl IntoResponse {
    let id = Uuid::new_v4();
    let row = CollectionRow {
        id,
        name: opts.name,
        description: opts.description,
        cancel_on_failure: opts.cancel_on_failure,
        request_full_system: opts.request_full_system,
        start_with_no_model_loaded: opts.start_with_no_model_loaded,
        save_partial_results: opts.save_partial_results,
        metadata_json: opts.metadata.as_ref().map(|v| v.to_string()),
        state: CollectionState::Active,
        created_at: chrono::Utc::now(),
        started_at: None,
        completed_at: None,
    };

    match state.store.insert_collection(&row) {
        Ok(()) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => err_response(e).into_response(),
    }
}

/// GET /v1/collections/:id — get collection state and member counts.
pub async fn get_collection(
    State(state): State<ApiState>,
    Path(id): Path<CollectionId>,
) -> impl IntoResponse {
    match state.store.get_collection(id) {
        Err(e) => err_response(e).into_response(),
        Ok(row) => {
            // (total, completed, failed, cancelled)
            let counts = state.store.collection_member_counts(id).ok();
            let body = serde_json::json!({
                "id": row.id,
                "name": row.name,
                "description": row.description,
                "state": row.state.as_str(),
                "cancel_on_failure": row.cancel_on_failure,
                "request_full_system": row.request_full_system,
                "start_with_no_model_loaded": row.start_with_no_model_loaded,
                "save_partial_results": row.save_partial_results,
                "created_at": row.created_at.to_rfc3339(),
                "started_at": row.started_at.map(|t| t.to_rfc3339()),
                "completed_at": row.completed_at.map(|t| t.to_rfc3339()),
                "member_counts": counts.map(|(total, completed, failed, cancelled)| {
                    serde_json::json!({
                        "total": total,
                        "completed": completed,
                        "failed": failed,
                        "cancelled": cancelled,
                        "active": total.saturating_sub(completed + failed + cancelled),
                    })
                }),
            });
            (StatusCode::OK, Json(body)).into_response()
        }
    }
}

/// DELETE /v1/collections/:id — cancel a collection and all non-terminal members.
///
/// Returns 204 on success, 409 if already terminal, 404 if not found.
pub async fn cancel_collection(
    State(state): State<ApiState>,
    Path(id): Path<CollectionId>,
) -> impl IntoResponse {
    // Check existence and guard against terminal state.
    match state.store.get_collection(id) {
        Err(e) => return err_response(e).into_response(),
        Ok(row) if row.state.is_terminal() => {
            return err_response(SubstrateError::CollectionTerminal {
                id,
                state: row.state,
            })
            .into_response();
        }
        Ok(_) => {}
    }

    match state.store.cancel_collection(id) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => err_response(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

/// GET /v1/models — list all registered models.
pub async fn list_models(State(state): State<ApiState>) -> impl IntoResponse {
    match state.store.list_models() {
        Ok(models) => {
            let list: Vec<_> = models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "source": m.source,
                        "context_length": m.context_length,
                        "status": format!("{:?}", m.status()),
                        "is_downloaded": m.is_downloaded,
                        "is_loaded": m.is_loaded,
                        "file_bytes": m.file_bytes,
                        "file_path": m.file_path,
                        "n_gpu_layers": m.n_gpu_layers,
                        "max_slots": m.max_slots,
                        "last_used_at": m.last_used_at.map(|t| t.to_rfc3339()),
                        "registered_at": m.registered_at.to_rfc3339(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(list))).into_response()
        }
        Err(e) => err_response(e).into_response(),
    }
}

/// POST /v1/models/:id/download — trigger model download (returns 202 immediately).
///
/// This is a stub. A full implementation would call `ModelManager::ensure_available`.
/// The model manager is not in `ApiState`; this endpoint acknowledges the request and
/// returns the current model status. The seam for wiring the manager is preserved.
pub async fn download_model(
    State(state): State<ApiState>,
    Path(model_id): Path<String>,
) -> impl IntoResponse {
    match state.store.get_model(&model_id) {
        Ok(row) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "id": row.id,
                "status": format!("{:?}", row.status()),
                "is_downloaded": row.is_downloaded,
                "message": "download request accepted",
            })),
        )
            .into_response(),
        Err(e) => err_response(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Estimation
// ---------------------------------------------------------------------------

/// POST /v1/estimate — estimate throughput for a completion shape.
///
/// Body: `CompletionShape` JSON.
/// Returns 200 with duration and confidence annotation.
///
/// Uses `ThroughputEstimator` from `substrate-telemetry`. In production the estimator
/// would be pre-warmed from historical benchmark data. This endpoint creates a fresh
/// estimator instance, so results are always cold-start estimates at this time.
/// The seam for passing a pre-warmed estimator through `ApiState` is clear.
pub async fn estimate(
    State(_state): State<ApiState>,
    Json(shape): Json<substrate_types::CompletionShape>,
) -> impl IntoResponse {
    let estimator = substrate_telemetry::ThroughputEstimator::new();
    let duration = estimator.estimate(shape.prompt_tokens, shape.max_tokens, shape.concurrency);

    let tokens_per_second = if duration.millis > 0 {
        shape.max_tokens as f32 / (duration.millis as f32 / 1000.0)
    } else {
        0.0
    };

    let cold_start = matches!(
        duration.confidence,
        substrate_telemetry::Confidence::Extrapolated
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "shape": {
                "model_id": shape.model_id,
                "prompt_tokens": shape.prompt_tokens,
                "max_tokens": shape.max_tokens,
                "concurrency": shape.concurrency,
            },
            "tokens_per_second": tokens_per_second,
            "estimated_ms": duration.millis,
            "cold_start": cold_start,
            "confidence": match duration.confidence {
                substrate_telemetry::Confidence::InEnvelope => "in_envelope",
                substrate_telemetry::Confidence::Extrapolated => "extrapolated",
            },
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Execution control
// ---------------------------------------------------------------------------

/// POST /v1/execution/pause — stop admitting new completions.
///
/// In-flight completions continue to run. Returns `{ "ok": true }`.
pub async fn pause_handler(State(state): State<ApiState>) -> impl IntoResponse {
    state.paused.store(true, Ordering::SeqCst);
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

/// POST /v1/execution/resume — resume admitting new completions.
///
/// Returns `{ "ok": true }`.
pub async fn resume_handler(State(state): State<ApiState>) -> impl IntoResponse {
    state.paused.store(false, Ordering::SeqCst);
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

// ---------------------------------------------------------------------------
// System state / observability
// ---------------------------------------------------------------------------

/// GET /v1/system/state (also aliased to /metrics) — current system resource snapshot.
pub async fn system_state(State(state): State<ApiState>) -> impl IntoResponse {
    let sys = state.telemetry.current_state().await;
    (StatusCode::OK, Json(serde_json::json!(sys))).into_response()
}

/// GET /health — liveness probe.
///
/// Returns `{ "status": "ok", "version": "0.1.0" }`.
pub async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Benchmark
// ---------------------------------------------------------------------------

/// GET /v1/benchmark/kernel — per-model throughput kernel data.
///
/// For each model that has benchmark data, returns:
/// - `model_id`, `model_name`, `is_current` (whether model is currently loaded)
/// - `x_axis`: "output_tokens" — the primary independent variable
/// - Quadratic polynomial coefficients `[a, b, c]` for `tps = a + b*x + c*x²`
///   fitted over all benchmark runs for that model (parallelism=1 only for the
///   simple 1-D kernel curve)
/// - `x_min_measured`, `x_max_measured`: observed output-token range
/// - `data_points`: raw `[{x, y}]` scatter data for the model
///
/// Returns `{ "models": [] }` when no benchmark data exists.
pub async fn benchmark_kernel(State(state): State<ApiState>) -> impl IntoResponse {
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

    // Determine the currently resident model for the `is_current` flag.
    let current_state = state.telemetry.current_state().await;
    let resident = current_state.resident_model.clone();

    let mut model_kernels = Vec::new();

    for model in &models {
        let runs = match state.store.benchmark_runs_for_model(&model.id) {
            Ok(r) => r,
            Err(_) => continue,
        };

        // Filter to runs that completed (have tokens_per_second) and use
        // parallelism=1 for the simple 1-D throughput-vs-output-tokens curve.
        let valid: Vec<(u32, f32)> = runs
            .iter()
            .filter(|r| r.concurrency == 1)
            .filter_map(|r| r.tokens_per_second.map(|tps| (r.max_tokens, tps)))
            .collect();

        if valid.is_empty() {
            continue;
        }

        // Fit quadratic: tps = a + b*x + c*x²  via ordinary least squares.
        let (a, b, c) = fit_quadratic_tps(&valid);

        let x_min = valid.iter().map(|(x, _)| *x).min().unwrap_or(0);
        let x_max = valid.iter().map(|(x, _)| *x).max().unwrap_or(0);

        let data_points: Vec<_> = valid
            .iter()
            .map(|(x, y)| serde_json::json!({ "x": x, "y": y }))
            .collect();

        model_kernels.push(serde_json::json!({
            "model_id":        model.id,
            "model_name":      model.id,   // name field not separate in ModelRow
            "is_current":      resident.as_deref() == Some(&model.id),
            "x_axis":          "output_tokens",
            "x_label":         "Output tokens",
            "y_label":         "tokens/sec",
            "coefficients":    [a, b, c],
            "x_min_measured":  x_min,
            "x_max_measured":  x_max,
            "data_points":     data_points,
        }));
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({ "models": model_kernels })),
    )
        .into_response()
}

/// POST /v1/benchmark/run — trigger a benchmark sweep for a given model.
///
/// Body: `{ "model_id": "<id>" }` (optional — if omitted, uses the currently
/// loaded model).
///
/// Submits a priority-0 full-system sweep collection asynchronously and returns
/// a `run_id` (the collection UUID). Returns 422 if no model is loaded and no
/// `model_id` is provided.
#[derive(Deserialize)]
pub struct BenchmarkRunBody {
    pub model_id: Option<String>,
}

pub async fn benchmark_run(
    State(state): State<ApiState>,
    Json(body): Json<BenchmarkRunBody>,
) -> impl IntoResponse {
    use substrate_benchmark::{BenchmarkConfig, BenchmarkOrchestrator};

    // Resolve target model.
    let model_id = match body.model_id {
        Some(id) => id,
        None => {
            // Use the currently resident model.
            let sys = state.telemetry.current_state().await;
            match sys.resident_model {
                Some(id) => id,
                None => {
                    return (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(serde_json::json!({
                            "error": "no model_id provided and no model is currently loaded"
                        })),
                    )
                        .into_response();
                }
            }
        }
    };

    // Verify model exists.
    if let Err(e) = state.store.get_model(&model_id) {
        return err_response(e).into_response();
    }

    let orch = BenchmarkOrchestrator::new(
        state.store.clone(),
        BenchmarkConfig::default(),
        Arc::clone(&state.scheduler),
    );

    let queue_empty = orch.is_queue_empty();
    match orch.schedule_if_idle(&model_id, queue_empty) {
        Ok(Some(collection_id)) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({ "run_id": collection_id })),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "run_id": null,
                "message": "all benchmark cells already satisfied; no sweep needed"
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Quadratic least-squares helper
// ---------------------------------------------------------------------------

/// Fit `y = a + b*x + c*x²` over the provided `(x, y)` pairs.
///
/// Uses the normal equations (3×3 linear system). Returns `(0.0, 0.0, 0.0)`
/// when fewer than 3 points are available or the system is singular.
fn fit_quadratic_tps(data: &[(u32, f32)]) -> (f64, f64, f64) {
    if data.len() < 3 {
        return (0.0, 0.0, 0.0);
    }

    // Build 3×3 normal equations for design matrix [1, x, x²].
    let n = data.len() as f64;
    let mut s1 = 0.0f64;  // Σ x
    let mut s2 = 0.0f64;  // Σ x²
    let mut s3 = 0.0f64;  // Σ x³
    let mut s4 = 0.0f64;  // Σ x⁴
    let mut sy = 0.0f64;  // Σ y
    let mut sxy = 0.0f64; // Σ x·y
    let mut sx2y = 0.0f64; // Σ x²·y

    for &(xi, yi) in data {
        let x = xi as f64;
        let y = yi as f64;
        s1 += x;
        s2 += x * x;
        s3 += x * x * x;
        s4 += x * x * x * x;
        sy += y;
        sxy += x * y;
        sx2y += x * x * y;
    }

    // Solve the 3×3 system via Cramer's rule (small enough to be fine).
    //   [n    s1   s2 ] [a]   [sy  ]
    //   [s1   s2   s3 ] [b] = [sxy ]
    //   [s2   s3   s4 ] [c]   [sx2y]
    let det = n * (s2 * s4 - s3 * s3)
            - s1 * (s1 * s4 - s3 * s2)
            + s2 * (s1 * s3 - s2 * s2);

    if det.abs() < 1e-12 {
        // Singular — fall back to linear fit (c=0).
        let denom = n * s2 - s1 * s1;
        if denom.abs() < 1e-12 {
            return (0.0, 0.0, 0.0);
        }
        let b = (n * sxy - s1 * sy) / denom;
        let a = (sy - b * s1) / n;
        return (a, b, 0.0);
    }

    let a = (sy  * (s2 * s4 - s3 * s3)
           - s1  * (sxy * s4 - s3 * sx2y)
           + s2  * (sxy * s3 - s2 * sx2y)) / det;

    let b = (n   * (sxy * s4 - s3 * sx2y)
           - sy  * (s1 * s4 - s3 * s2)
           + s2  * (s1 * sx2y - sxy * s2)) / det;

    let c = (n   * (s2 * sx2y - sxy * s3)
           - s1  * (s1 * sx2y - sxy * s2)
           + sy  * (s1 * s3 - s2 * s2)) / det;

    (a, b, c)
}
