//! Axum REST handlers for the GC API server.
//!
//! State: `Arc<GcService>`.
//!
//! Error mapping:
//! - "not found" / "not registered" messages → 404
//! - "locked" / "DiskBudgetExceeded" → 409 Conflict
//! - everything else → 500

use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use substrate_gc::{
    DirPolicy, EntryKind, GcDirRow, GcEntryRow, GcService, SweepReport,
};
use tokio::sync::broadcast;

// ---------------------------------------------------------------------------
// App state
// ---------------------------------------------------------------------------

pub type AppState = Arc<GcService>;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RegisterDirRequest {
    pub path: String,
    /// Optional policy overrides. If absent the directory's existing (or default) policy is used.
    pub policy: Option<DirPolicy>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterEntryRequest {
    pub path: String,
    /// `"file"` or `"directory"`.
    pub kind: String,
    pub recovery_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TouchRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct LockRequest {
    pub path: String,
    pub ttl_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct DeregisterDirRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct UnlockRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct GetEntryQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct MakeRoomRequest {
    pub dir: String,
    pub bytes_needed: u64,
}

#[derive(Debug, Deserialize)]
pub struct MoveRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
pub struct EvictRequest {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ListEntriesQuery {
    pub dir: Option<String>,
}

// ---------------------------------------------------------------------------
// Serialisable response shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct DirResponse {
    root: String,
    policy: DirPolicy,
    registered_at: i64,
    last_swept_at: Option<i64>,
    /// Sum of `size_bytes` across all `present` entries under this dir.
    /// The ceiling is `policy.max_size_bytes`.
    used_bytes: u64,
}

impl DirResponse {
    fn from_row(r: GcDirRow, used_bytes: u64) -> Self {
        Self {
            root: r.root,
            policy: r.policy,
            registered_at: r.registered_at,
            last_swept_at: r.last_swept_at,
            used_bytes,
        }
    }
}

#[derive(Debug, Serialize)]
struct EntryResponse {
    path: String,
    dir_root: String,
    kind: String,
    size_bytes: u64,
    registered_at: i64,
    last_touched_at: i64,
    touch_count: u64,
    lock_expires_at: Option<i64>,
    ttl_override_secs: Option<u64>,
    recovery_hint: Option<String>,
    state: String,
}

impl From<GcEntryRow> for EntryResponse {
    fn from(r: GcEntryRow) -> Self {
        Self {
            path: r.path,
            dir_root: r.dir_root,
            kind: r.kind,
            size_bytes: r.size_bytes,
            registered_at: r.registered_at,
            last_touched_at: r.last_touched_at,
            touch_count: r.touch_count,
            lock_expires_at: r.lock_expires_at,
            ttl_override_secs: r.ttl_override_secs,
            recovery_hint: r.recovery_hint,
            state: r.state,
        }
    }
}

#[derive(Debug, Serialize)]
struct SweepResponse {
    expired_evicted: u64,
    budget_evicted: u64,
    bytes_freed: u64,
    errors: Vec<String>,
}

impl From<SweepReport> for SweepResponse {
    fn from(r: SweepReport) -> Self {
        Self {
            expired_evicted: r.expired_evicted,
            budget_evicted: r.budget_evicted,
            bytes_freed: r.bytes_freed,
            errors: r.errors,
        }
    }
}

// ---------------------------------------------------------------------------
// Error helper
// ---------------------------------------------------------------------------

fn gc_err(e: anyhow::Error) -> (StatusCode, Json<serde_json::Value>) {
    let msg = e.to_string();
    let status = if msg.contains("not found")
        || msg.contains("not registered")
        || msg.contains("entry not found")
        || msg.contains("directory not registered")
    {
        StatusCode::NOT_FOUND
    } else if msg.contains("locked")
        || msg.contains("DiskBudgetExceeded")
        || msg.contains("is locked")
    {
        StatusCode::CONFLICT
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, Json(serde_json::json!({ "error": msg })))
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

pub fn routes() -> Router<AppState> {
    Router::new()
        // Dirs
        .route("/dirs/register", post(register_dir))
        .route("/dirs/deregister", post(deregister_dir))
        .route("/dirs", get(list_dirs))
        // Entries
        .route("/entries/register", post(register_entry))
        .route("/entries/touch", post(touch_entry))
        .route("/entries/lock", post(lock_entry))
        .route("/entries/unlock", post(unlock_entry))
        .route("/entries/get", get(get_entry))
        .route("/entries", get(list_entries))
        // Ops
        .route("/ops/make-room", post(make_room))
        .route("/ops/move", post(move_path))
        .route("/ops/evict", post(evict_entry))
        .route("/ops/sweep", post(sweep))
        // Events
        .route("/events", get(events_ws))
        // Health
        .route("/health", get(health))
}

// ---------------------------------------------------------------------------
// Dir handlers
// ---------------------------------------------------------------------------

async fn register_dir(
    State(svc): State<AppState>,
    Json(body): Json<RegisterDirRequest>,
) -> impl IntoResponse {
    let path = std::path::Path::new(&body.path);

    // If a policy override was provided, write it into the .gc/ dir first so
    // that GcService::register_dir picks it up.
    if let Some(policy) = body.policy {
        let gc_dir = path.join(".gc");
        if let Err(e) = std::fs::create_dir_all(&gc_dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| policy.save(&gc_dir))
        {
            return gc_err(e).into_response();
        }
    }

    match svc.register_dir(path) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn deregister_dir(
    State(svc): State<AppState>,
    Json(body): Json<DeregisterDirRequest>,
) -> impl IntoResponse {
    match svc.deregister_dir(std::path::Path::new(&body.path)) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn list_dirs(State(svc): State<AppState>) -> impl IntoResponse {
    match svc.list_dirs() {
        Ok(dirs) => {
            let list: Vec<DirResponse> = dirs
                .into_iter()
                .map(|d| {
                    let used_bytes = svc.dir_used_bytes(&d.root).unwrap_or(0);
                    DirResponse::from_row(d, used_bytes)
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!(list))).into_response()
        }
        Err(e) => gc_err(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Entry handlers
// ---------------------------------------------------------------------------

async fn register_entry(
    State(svc): State<AppState>,
    Json(body): Json<RegisterEntryRequest>,
) -> impl IntoResponse {
    let kind = match body.kind.as_str() {
        "file" => EntryKind::File,
        "directory" => EntryKind::Directory,
        other => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": format!("unknown kind {other:?}; expected \"file\" or \"directory\"")
                })),
            )
                .into_response();
        }
    };

    match svc.register_entry(std::path::Path::new(&body.path), kind, body.recovery_hint) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn touch_entry(
    State(svc): State<AppState>,
    Json(body): Json<TouchRequest>,
) -> impl IntoResponse {
    match svc.touch(&body.path) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn lock_entry(
    State(svc): State<AppState>,
    Json(body): Json<LockRequest>,
) -> impl IntoResponse {
    match svc.lock(&body.path, body.ttl_secs) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn unlock_entry(
    State(svc): State<AppState>,
    Json(body): Json<UnlockRequest>,
) -> impl IntoResponse {
    match svc.unlock(&body.path) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn get_entry(
    State(svc): State<AppState>,
    Query(params): Query<GetEntryQuery>,
) -> impl IntoResponse {
    match svc.query(&params.path) {
        Ok(Some(entry)) => {
            let resp: EntryResponse = entry.into();
            (StatusCode::OK, Json(serde_json::json!(resp))).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("entry not found: {}", params.path) })),
        )
            .into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn list_entries(
    State(svc): State<AppState>,
    Query(params): Query<ListEntriesQuery>,
) -> impl IntoResponse {
    let result = match params.dir {
        Some(dir) if !dir.is_empty() => svc.list_entries(&dir),
        _ => svc.list_all_entries(),
    };
    match result {
        Ok(entries) => {
            let list: Vec<EntryResponse> = entries.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(serde_json::json!(list))).into_response()
        }
        Err(e) => gc_err(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Ops handlers
// ---------------------------------------------------------------------------

async fn make_room(
    State(svc): State<AppState>,
    Json(body): Json<MakeRoomRequest>,
) -> impl IntoResponse {
    match svc.make_room(&body.dir, body.bytes_needed) {
        Ok(freed) => (
            StatusCode::OK,
            Json(serde_json::json!({ "bytes_freed": freed })),
        )
            .into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn move_path(
    State(svc): State<AppState>,
    Json(body): Json<MoveRequest>,
) -> impl IntoResponse {
    match svc.move_path(
        std::path::Path::new(&body.from),
        std::path::Path::new(&body.to),
    ) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn evict_entry(
    State(svc): State<AppState>,
    Json(body): Json<EvictRequest>,
) -> impl IntoResponse {
    match svc.evict(&body.path) {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => gc_err(e).into_response(),
    }
}

async fn sweep(State(svc): State<AppState>) -> impl IntoResponse {
    match svc.sweep() {
        Ok(report) => {
            let resp: SweepResponse = report.into();
            (StatusCode::OK, Json(serde_json::json!(resp))).into_response()
        }
        Err(e) => gc_err(e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// WebSocket event stream
// ---------------------------------------------------------------------------

/// Heartbeat interval — keeps the connection alive through proxies.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

/// GET /events — upgrade to WebSocket, stream GcEvent as JSON text frames.
async fn events_ws(
    ws: WebSocketUpgrade,
    State(svc): State<AppState>,
) -> impl IntoResponse {
    let rx = svc.subscribe_events();
    ws.on_upgrade(move |socket| drive_events_socket(socket, rx))
}

async fn drive_events_socket(
    socket: WebSocket,
    mut rx: broadcast::Receiver<substrate_gc::GcEvent>,
) {
    if let Err(e) = run_events_socket(socket, &mut rx).await {
        tracing::debug!(error = %e, "gc events WebSocket disconnected");
    }
}

async fn run_events_socket(
    socket: WebSocket,
    rx: &mut broadcast::Receiver<substrate_gc::GcEvent>,
) -> anyhow::Result<()> {
    let (mut sender, mut receiver) = socket.split();

    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.tick().await; // consume the immediate first tick

    loop {
        tokio::select! {
            // Inbound message from client (detect disconnect).
            client_msg = receiver.next() => {
                match client_msg {
                    None | Some(Ok(Message::Close(_))) => {
                        tracing::debug!("gc events WebSocket client disconnected");
                        break;
                    }
                    Some(Err(_)) => break,
                    Some(Ok(_)) => {} // ignore pong / other frames
                }
            }

            // GcEvent from the broadcast channel.
            event_result = rx.recv() => {
                match event_result {
                    Ok(event) => {
                        let json = serde_json::to_string(&event)?;
                        if sender.send(Message::Text(json.into())).await.is_err() {
                            break; // client gone
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(missed = n, "gc events WebSocket client lagged; {n} events dropped");
                        // Continue — client stays connected but missed events.
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!("gc event channel closed");
                        break;
                    }
                }
            }

            // Periodic heartbeat ping.
            _ = heartbeat.tick() => {
                if sender.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

async fn health() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
        })),
    )
        .into_response()
}
