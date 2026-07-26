//! The axum WebSocket daemon (INTENT #208 — "one service running on a port").
//!
//! One route: `GET /ws` upgrades to the CCW protocol ([`crate::protocol`]).
//! `GET /health` is a liveness probe. Every connection gets a version-stamped
//! `welcome`, then drives request/response RPC + a filtered push event stream.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use substrate_types::{Result, SubstrateError};

use crate::budget::{AccountSelector, BudgetRules};
use crate::protocol::{ClientMsg, Outcome, ServerMsg, PROTO};
use crate::session::{SessionManager, SessionSpec};

const DAEMON_NAME: &str = "ccw";
const DAEMON_VERSION: &str = env!("CARGO_PKG_VERSION");
const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Build the router.
pub fn router(manager: SessionManager) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(health))
        .with_state(Arc::new(manager))
}

/// Bind on `addr`, returning the actually-bound address (so callers passing
/// port 0 learn the ephemeral port) and the serve task handle.
pub async fn serve(
    manager: SessionManager,
    addr: SocketAddr,
) -> Result<(SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| SubstrateError::Config(format!("binding {addr}: {e}")))?;
    let bound = listener
        .local_addr()
        .map_err(|e| SubstrateError::Config(format!("local_addr: {e}")))?;
    let app = router(manager);
    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, "ccw server error");
        }
    });
    Ok((bound, handle))
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "daemon": DAEMON_NAME, "version": DAEMON_VERSION, "proto": PROTO }))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(manager): State<Arc<SessionManager>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| drive(socket, manager))
}

/// One client connection.
async fn drive(socket: WebSocket, manager: Arc<SessionManager>) {
    let (mut tx, mut rx) = socket.split();
    let mut events = manager.subscribe();

    // Subscription filter.
    let mut firehose = false;
    let mut sessions: HashSet<String> = HashSet::new();

    // Welcome (version stamp).
    let welcome = ServerMsg::Welcome {
        proto: PROTO,
        daemon: DAEMON_NAME,
        daemon_version: DAEMON_VERSION,
    };
    if send(&mut tx, &welcome).await.is_err() {
        return;
    }

    let mut ping = tokio::time::interval(PING_INTERVAL);
    ping.tick().await;

    loop {
        tokio::select! {
            incoming = rx.next() => {
                match incoming {
                    None | Some(Err(_)) => break,
                    Some(Ok(Message::Close(_))) => break,
                    Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) | Some(Ok(Message::Binary(_))) => {}
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ClientMsg>(&text) {
                            Ok(ClientMsg::Request { id, method, params }) => {
                                let outcome = dispatch(&manager, &method, params).await;
                                let msg = ServerMsg::Response { correlate: id, outcome };
                                if send(&mut tx, &msg).await.is_err() { break; }
                            }
                            Ok(ClientMsg::Subscribe { sessions: s }) => {
                                match s {
                                    None => firehose = true,
                                    Some(list) => sessions.extend(list),
                                }
                                let ack = ServerMsg::SubAck {
                                    firehose,
                                    sessions: sessions.iter().cloned().collect(),
                                };
                                if send(&mut tx, &ack).await.is_err() { break; }
                            }
                            Ok(ClientMsg::Unsubscribe { sessions: s }) => {
                                match s {
                                    None => { firehose = false; sessions.clear(); }
                                    Some(list) => { for id in list { sessions.remove(&id); } }
                                }
                                let ack = ServerMsg::SubAck {
                                    firehose,
                                    sessions: sessions.iter().cloned().collect(),
                                };
                                if send(&mut tx, &ack).await.is_err() { break; }
                            }
                            Err(e) => {
                                let msg = ServerMsg::Response {
                                    correlate: String::new(),
                                    outcome: Outcome::err(format!("invalid frame: {e}")),
                                };
                                if send(&mut tx, &msg).await.is_err() { break; }
                            }
                        }
                    }
                }
            }
            ev = events.recv() => {
                match ev {
                    Ok(se) => {
                        if firehose || sessions.contains(&se.session_id) {
                            let msg = ServerMsg::Event {
                                session_id: se.session_id,
                                seq: se.seq,
                                event: se.event,
                            };
                            if send(&mut tx, &msg).await.is_err() { break; }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            _ = ping.tick() => {
                if tx.send(Message::Ping(Vec::new().into())).await.is_err() { break; }
            }
        }
    }
}

async fn send(
    tx: &mut futures::stream::SplitSink<WebSocket, Message>,
    msg: &ServerMsg,
) -> std::result::Result<(), ()> {
    let json = serde_json::to_string(msg).map_err(|_| ())?;
    tx.send(Message::Text(json.into())).await.map_err(|_| ())
}

/// Route one request `method` + `params` to an [`Outcome`].
async fn dispatch(manager: &SessionManager, method: &str, params: Value) -> Outcome {
    match handle(manager, method, params).await {
        Ok(payload) => Outcome::ok(payload),
        Err(e) => Outcome::err(e.to_string()),
    }
}

async fn handle(manager: &SessionManager, method: &str, params: Value) -> Result<Value> {
    match method {
        // ── sessions ─────────────────────────────────────────────────────
        "sessions.start" => {
            let spec: SessionSpec = serde_json::from_value(params.clone())
                .map_err(|e| SubstrateError::Config(format!("bad session spec: {e}")))?;
            let id = manager.start_session(spec)?;
            // Optional inline first-turn prompt.
            if let Some(prompt) = params.get("prompt").and_then(Value::as_str) {
                if !prompt.is_empty() {
                    manager.send_turn(&id, prompt.to_string())?;
                }
            }
            Ok(json!({ "session_id": id }))
        }
        "sessions.send" => {
            let id = req_str(&params, "session_id")?;
            let prompt = req_str(&params, "prompt")?;
            manager.send_turn(&id, prompt)?;
            Ok(json!({ "accepted": true, "session_id": id }))
        }
        "sessions.resume" => {
            let id = req_str(&params, "session_id")?;
            if let Some(prompt) = params.get("prompt").and_then(Value::as_str) {
                if !prompt.is_empty() {
                    manager.send_turn(&id, prompt.to_string())?;
                }
            }
            Ok(json!({ "resumed": true, "session_id": id }))
        }
        "sessions.list" => {
            let limit = opt_usize(&params, "limit").unwrap_or(100);
            let rows = manager.store.list_sessions(limit)?;
            let out: Vec<Value> = rows
                .into_iter()
                .map(|r| {
                    json!({
                        "session_id": r.id, "account": r.account, "cwd": r.cwd,
                        "model": r.model, "budget_id": r.budget_id, "status": r.status,
                        "last_seq": r.last_seq, "created_at": r.created_at, "updated_at": r.updated_at,
                    })
                })
                .collect();
            Ok(json!({ "sessions": out }))
        }
        "sessions.history" => {
            let id = req_str(&params, "session_id")?;
            let after = opt_u64(&params, "after_seq").unwrap_or(0);
            let limit = opt_usize(&params, "limit").unwrap_or(200);
            let events = manager.store.list_events(&id, after, limit)?;
            let next_after = events.last().map(|e| e.seq).unwrap_or(after);
            let out: Vec<Value> = events
                .into_iter()
                .map(|e| json!({ "seq": e.seq, "event": e.event }))
                .collect();
            Ok(json!({ "session_id": id, "events": out, "next_after": next_after }))
        }
        "sessions.cancel" => {
            let id = req_str(&params, "session_id")?;
            manager.cancel(&id)?;
            Ok(json!({ "cancelled": true, "session_id": id }))
        }

        // ── budgets ──────────────────────────────────────────────────────
        "budgets.list" => {
            let out: Vec<Value> = manager
                .store
                .list_budgets()?
                .into_iter()
                .map(|(id, r)| {
                    json!({
                        "id": id, "weekly_pct": r.weekly_pct,
                        "session_backoff_pct": r.session_backoff_pct,
                        "accounts": r.accounts,
                    })
                })
                .collect();
            Ok(json!({ "budgets": out }))
        }
        "budgets.add" => {
            let id = req_str(&params, "id")?;
            let mut rules = BudgetRules::default();
            if let Some(w) = params.get("weekly_pct").and_then(Value::as_f64) {
                rules.weekly_pct = w;
            }
            if let Some(s) = params.get("session_backoff_pct").and_then(Value::as_f64) {
                rules.session_backoff_pct = s;
            }
            if let Some(accts) = params.get("accounts").and_then(Value::as_array) {
                let names: Vec<String> =
                    accts.iter().filter_map(|a| a.as_str().map(str::to_string)).collect();
                if !names.is_empty() && !(names.len() == 1 && names[0].eq_ignore_ascii_case("auto")) {
                    rules.accounts = AccountSelector::Named(names);
                }
            }
            manager.store.upsert_budget(&id, &rules)?;
            Ok(json!({ "id": id, "rules": serde_json::from_str::<Value>(&rules.to_json()).unwrap_or(Value::Null) }))
        }

        // ── accounts ─────────────────────────────────────────────────────
        "accounts.list" => {
            let out: Vec<Value> = manager
                .registry
                .names()
                .into_iter()
                .map(|n| {
                    let cd = manager.registry.get(&n).and_then(|a| a.config_dir.clone());
                    json!({ "account": n, "config_dir": cd })
                })
                .collect();
            Ok(json!({ "accounts": out }))
        }
        "accounts.status" => {
            // Live, zero-cost `/usage` per account (blocking child → off-reactor).
            let registry = manager.registry.clone();
            let out = tokio::task::spawn_blocking(move || {
                let mut accounts = Vec::new();
                for n in registry.names() {
                    let cd = registry
                        .get(&n)
                        .and_then(|a| crate::accounts::config_dir_of(a));
                    let snap = crate::claude::usage_snapshot(cd.as_deref(), Duration::from_secs(30)).ok();
                    let pools: Vec<Value> = snap
                        .as_ref()
                        .map(|s| {
                            s.pools
                                .iter()
                                .map(|p| {
                                    json!({
                                        "pool": p.pool, "pct": p.pct,
                                        "reset": p.reset.as_ref().map(|r| r.raw.clone()),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    accounts.push(json!({ "account": n, "pools": pools }));
                }
                accounts
            })
            .await
            .map_err(|e| SubstrateError::Internal(format!("usage probe join: {e}")))?;
            Ok(json!({ "accounts": out }))
        }

        // ── ledger ───────────────────────────────────────────────────────
        "ledger.query" => {
            let budget = params.get("budget").and_then(Value::as_str);
            let limit = opt_usize(&params, "limit").unwrap_or(50);
            let rows = manager.store.list_invocations(budget, limit)?;
            let out: Vec<Value> = rows
                .into_iter()
                .map(|r| {
                    json!({
                        "budget_id": r.budget_id, "account": r.account, "session_id": r.session_id,
                        "started_at": r.started_at, "total_tokens": r.total_tokens,
                        "cost_usd": r.cost_usd, "limit_hit": r.limit_hit, "exit_code": r.exit_code,
                    })
                })
                .collect();
            Ok(json!({ "invocations": out }))
        }

        other => Err(SubstrateError::Config(format!("unknown method: {other}"))),
    }
}

fn req_str(params: &Value, key: &str) -> Result<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| SubstrateError::Config(format!("missing required param: {key}")))
}
fn opt_u64(params: &Value, key: &str) -> Option<u64> {
    params.get(key).and_then(Value::as_u64)
}
fn opt_usize(params: &Value, key: &str) -> Option<usize> {
    params.get(key).and_then(Value::as_u64).map(|v| v as usize)
}
