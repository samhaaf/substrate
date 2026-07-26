//! WS protocol round-trip tests. Each starts the CCW daemon in-process on an
//! ephemeral port with a FAKE `claude` binary (canned stream-json — NO real
//! invocation), connects a real WebSocket client, and asserts the contract:
//! event-stream correctness, subagent-liveness-while-busy, retry meta-events,
//! budget_pending shape, history paging, and the budgets/accounts surfaces.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use futures::{stream::SplitSink, stream::SplitStream, SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

use substrate_ccw::{build_manager, server, Config};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Absolute path to the committed fake claude emitter.
fn fake_claude() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fake-claude")
        .to_string_lossy()
        .to_string()
}

/// Set the process-global test env ONCE to fixed values (identical across every
/// test, so parallel tests never race). Turn MODE is chosen from the prompt, not
/// env, precisely so this can stay shared.
fn set_env() {
    std::env::set_var("CCW_CLAUDE_BIN", fake_claude());
    std::env::set_var("CCW_RETRY_WAIT_CAP_MS", "50");
    std::env::set_var("CCW_HEARTBEAT_MS", "100");
    std::env::set_var("CCW_BUDGET_MAX_POLLS", "0");
    std::env::set_var("CCW_BUDGET_POLL_MS", "50");
    std::env::set_var("FAKE_SESSION_PCT", "95");
    std::env::set_var("FAKE_STATE_DIR", std::env::temp_dir());
}

/// Start the daemon on 127.0.0.1:0; returns its bound address + the temp data
/// dir (kept alive for the test's lifetime).
async fn start() -> (SocketAddr, tempfile::TempDir) {
    set_env();
    let tmp = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: Some(tmp.path().to_string_lossy().to_string()),
        ..Default::default()
    };
    let manager = build_manager(config).unwrap();
    let (addr, _handle) = server::serve(manager, "127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    (addr, tmp)
}

/// A minimal WS test client: request/response + a buffered event stream.
struct Client {
    wr: SplitSink<Ws, Message>,
    rd: SplitStream<Ws>,
    events: Vec<Value>, // buffered `event` frames not yet consumed
}

impl Client {
    async fn connect(addr: SocketAddr) -> Self {
        let (ws, _) = connect_async(format!("ws://{addr}/ws")).await.unwrap();
        let (wr, rd) = ws.split();
        let mut c = Client { wr, rd, events: Vec::new() };
        // First frame must be the version-stamped welcome.
        let w = c.next_frame().await;
        assert_eq!(w["type"], "welcome");
        assert_eq!(w["daemon"], "ccw");
        assert_eq!(w["proto"], 1);
        c
    }

    async fn next_frame(&mut self) -> Value {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(15), self.rd.next())
                .await
                .expect("timed out waiting for frame")
                .expect("ws stream ended")
                .expect("ws error");
            match msg {
                Message::Text(t) => return serde_json::from_str(&t).unwrap(),
                Message::Close(_) => panic!("server closed connection"),
                _ => continue, // ping/pong/binary
            }
        }
    }

    async fn send(&mut self, v: Value) {
        self.wr.send(Message::Text(v.to_string())).await.unwrap();
    }

    async fn subscribe_firehose(&mut self) {
        self.send(json!({ "type": "subscribe", "sessions": null })).await;
        loop {
            let f = self.next_frame().await;
            if f["type"] == "sub_ack" {
                assert_eq!(f["firehose"], true);
                return;
            }
            self.buffer(f);
        }
    }

    /// Send a request and return its response `outcome`, buffering any events.
    async fn request(&mut self, id: &str, method: &str, params: Value) -> Value {
        self.send(json!({ "type": "request", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let f = self.next_frame().await;
            if f["type"] == "response" && f["correlate"] == id {
                return f["outcome"].clone();
            }
            self.buffer(f);
        }
    }

    fn buffer(&mut self, f: Value) {
        if f["type"] == "event" {
            self.events.push(f);
        }
    }

    /// Read (from buffer, then the wire) until an event whose `kind` matches,
    /// returning the inner event. Panics on timeout.
    async fn wait_kind(&mut self, kind: &str) -> Value {
        if let Some(pos) = self.events.iter().position(|f| f["event"]["kind"] == kind) {
            return self.events.remove(pos)["event"].clone();
        }
        loop {
            let f = self.next_frame().await;
            if f["type"] == "event" && f["event"]["kind"] == kind {
                return f["event"].clone();
            }
            self.buffer(f);
        }
    }

    /// Collect every event seen until `terminal` kind arrives (inclusive).
    async fn drain_until(&mut self, terminal: &str) -> Vec<Value> {
        let mut seen: Vec<Value> = self.events.drain(..).map(|f| f["event"].clone()).collect();
        if seen.iter().any(|e| e["kind"] == terminal) {
            return seen;
        }
        loop {
            let f = self.next_frame().await;
            if f["type"] == "event" {
                let ev = f["event"].clone();
                let is_terminal = ev["kind"] == terminal;
                seen.push(ev);
                if is_terminal {
                    return seen;
                }
            }
        }
    }
}

fn kinds(events: &[Value]) -> Vec<String> {
    events.iter().map(|e| e["kind"].as_str().unwrap_or("?").to_string()).collect()
}

fn base_spec(cwd: &std::path::Path, prompt: &str) -> Value {
    json!({
        "cwd": cwd.to_string_lossy(),
        "model": "fable",
        "account": "auto",
        "tools": { "clean_slate": true },
        "permission_mode": "bypassPermissions",
        "prompt": prompt,
    })
}

// ── Test 1: session start, full event stream, history paging ──────────────

#[tokio::test]
async fn basic_turn_streams_events_and_history_pages() {
    let (addr, tmp) = start().await;
    let mut c = Client::connect(addr).await;
    c.subscribe_firehose().await;

    let out = c
        .request("r1", "sessions.start", base_spec(tmp.path(), "hello basic"))
        .await;
    assert_eq!(out["type"], "success");
    let sid = out["payload"]["session_id"].as_str().unwrap().to_string();

    let events = c.drain_until("session_ended").await;
    let ks = kinds(&events);
    // The full in-thread stream is surfaced.
    assert!(ks.contains(&"session_started".to_string()), "{ks:?}");
    assert!(ks.contains(&"system_init".to_string()), "{ks:?}");
    assert!(ks.contains(&"assistant_text".to_string()), "{ks:?}");
    assert!(ks.contains(&"result_turn".to_string()), "{ks:?}");
    assert!(events.iter().any(|e| e["kind"] == "assistant_text"
        && e["text"].as_str().unwrap().contains("Hello from fake claude")));

    // History serves from CCW's own event log, paged by seq.
    let h1 = c
        .request("h1", "sessions.history", json!({ "session_id": sid, "after_seq": 0, "limit": 3 }))
        .await;
    assert_eq!(h1["type"], "success");
    let page1 = h1["payload"]["events"].as_array().unwrap();
    assert_eq!(page1.len(), 3, "first page should honor limit");
    let next_after = h1["payload"]["next_after"].as_u64().unwrap();
    assert_eq!(next_after, page1.last().unwrap()["seq"].as_u64().unwrap());

    let h2 = c
        .request("h2", "sessions.history", json!({ "session_id": sid, "after_seq": next_after, "limit": 100 }))
        .await;
    let page2 = h2["payload"]["events"].as_array().unwrap();
    assert!(!page2.is_empty(), "second page continues from the cursor");
    assert!(page2[0]["seq"].as_u64().unwrap() > next_after);
}

// ── Test 2: subagent liveness — BUSY while an async subagent runs ──────────

#[tokio::test]
async fn session_reports_busy_while_subagent_runs() {
    let (addr, tmp) = start().await;
    let mut c = Client::connect(addr).await;
    c.subscribe_firehose().await;

    let out = c
        .request("r1", "sessions.start", base_spec(tmp.path(), "please run a subagent"))
        .await;
    assert_eq!(out["type"], "success");

    // The Agent spawn + async launch are surfaced.
    let spawn = c.wait_kind("agent_spawn").await;
    assert_eq!(spawn["run_in_background"], true);
    let launched = c.wait_kind("agent_launched").await;
    assert_eq!(launched["is_async"], true);
    assert_eq!(launched["agent_id"], "agent-abc");

    // The CRITICAL assertion (INTENT #208): a liveness heartbeat reports BUSY
    // with the running subagent, even though the parent `result` has arrived —
    // the thread is NOT idle.
    let mut busy_with_agent = false;
    for _ in 0..50 {
        let live = c.wait_kind("liveness").await;
        if live["busy"] == true {
            let running = live["running_subagents"].as_array().unwrap();
            if running.iter().any(|a| a == "agent-abc") {
                busy_with_agent = true;
                break;
            }
        }
    }
    assert!(busy_with_agent, "session must report busy while the async subagent runs");
}

// ── Test 3: 429 → retry meta-event, then success ──────────────────────────

#[tokio::test]
async fn rate_limit_emits_retry_and_recovers() {
    let (addr, tmp) = start().await;
    let mut c = Client::connect(addr).await;
    c.subscribe_firehose().await;

    // Unique per run so the fake's per-prompt 429 marker (in the shared temp
    // dir) is fresh — a stale marker from a prior run would skip the 429.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let prompt = format!("please ratelimit this turn {nonce}");
    let out = c
        .request("r1", "sessions.start", base_spec(tmp.path(), &prompt))
        .await;
    assert_eq!(out["type"], "success");

    // The limit hit is surfaced the moment it streams (aui's blind spot).
    let hit = c.wait_kind("limit_hit").await;
    assert_eq!(hit["status"], 429);

    // A retry meta-event fires (no alternate account → wait-to-reset, capped).
    let retry = c.wait_kind("retry_started").await;
    assert!(retry["attempt"].as_u64().unwrap() >= 2);
    assert!(retry["reason"].as_str().unwrap().to_lowercase().contains("rate"));

    // The retried attempt succeeds.
    let text = c.wait_kind("assistant_text").await;
    assert!(text["text"].as_str().unwrap().contains("Hello from fake claude"));
}

// ── Test 4: budget admission → budget_pending{eta} shape ──────────────────

#[tokio::test]
async fn over_budget_turn_emits_budget_pending() {
    let (addr, tmp) = start().await;
    let mut c = Client::connect(addr).await;
    c.subscribe_firehose().await;

    // A budget whose session backoff (10%) is far below the fake /usage session
    // reading (95%) → admission blocks pre-spawn.
    let b = c
        .request("b1", "budgets.add", json!({ "id": "tight", "session_backoff_pct": 10.0 }))
        .await;
    assert_eq!(b["type"], "success");

    let mut spec = base_spec(tmp.path(), "do work under budget");
    spec["budget_id"] = json!("tight");
    let out = c.request("r1", "sessions.start", spec).await;
    assert_eq!(out["type"], "success");

    let pending = c.wait_kind("budget_pending").await;
    assert!(pending["reason"].as_str().unwrap().to_lowercase().contains("session"), "{pending}");
    // eta is present (the reset prose the loop UI shows).
    assert!(pending.get("eta").is_some());
}

// ── Test 5: budgets + accounts surfaces round-trip ────────────────────────

#[tokio::test]
async fn budgets_and_accounts_surfaces() {
    let (addr, _tmp) = start().await;
    let mut c = Client::connect(addr).await;

    // The machine default account is auto-registered on first run.
    let accts = c.request("a1", "accounts.list", json!({})).await;
    assert_eq!(accts["type"], "success");
    let list = accts["payload"]["accounts"].as_array().unwrap();
    assert!(list.iter().any(|a| a["account"] == "default"));

    // Add + list a budget.
    let add = c
        .request("a2", "budgets.add", json!({ "id": "std", "weekly_pct": 20.0, "session_backoff_pct": 50.0 }))
        .await;
    assert_eq!(add["type"], "success");
    let listed = c.request("a3", "budgets.list", json!({})).await;
    let budgets = listed["payload"]["budgets"].as_array().unwrap();
    assert!(budgets.iter().any(|b| b["id"] == "std" && b["weekly_pct"] == 20.0));

    // Unknown method → Error arm of the success/error/promise switch.
    let bad = c.request("a4", "does.not.exist", json!({})).await;
    assert_eq!(bad["type"], "error");
}

// ── Test 6: rollup ref is the documented not-implemented seam ──────────────

#[tokio::test]
async fn rollup_ref_is_rejected_not_implemented() {
    let (addr, tmp) = start().await;
    let mut c = Client::connect(addr).await;

    let mut spec = base_spec(tmp.path(), "");
    spec["rollup"] = json!("some/rollup/ref");
    let out = c.request("r1", "sessions.start", spec).await;
    assert_eq!(out["type"], "error");
    assert!(out["error"].as_str().unwrap().to_lowercase().contains("rollup"));
}
