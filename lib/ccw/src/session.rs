//! Session runtime — the daemon OWNS claude invocation (INTENT #208).
//!
//! A CCW *session* is a thread: an addressable claude conversation CCW spawns,
//! meters, and governs. This module holds the [`SessionManager`] (the daemon's
//! session registry + firehose) and the per-turn runner that:
//!
//! 1. admits the turn against its budget (pre-spawn, emitting `budget_pending`);
//! 2. spawns `claude` with the canonical streaming recipe ([`crate::spawn`]);
//! 3. parses the live stream into [`CcwEvent`]s ([`crate::stream`]), persists
//!    each to the event log AND pushes it to subscribers;
//! 4. tracks **subagent liveness** — a session reports BUSY while the parent
//!    process runs OR any async subagent's transcript is still churning, so a
//!    wrapper never mistakes "parent result" for "thread idle" (the operator's
//!    critical requirement, recon Deliverable 2);
//! 5. handles retries per policy ([`crate::retry`]) — 429 → switch-account or
//!    wait-to-reset; transient API errors → bounded backoff — with meta-events.
//!
//! Observe-don't-kill stays the mid-run default (INTENT #207): overage debits
//! the budget so the NEXT turn's admission blocks.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{broadcast, Notify};
use tokio::process::Command;

use substrate_types::{Result, SubstrateError};

use crate::accounts::{config_dir_of, Registry};
use crate::budget::{decide, AccountUsage, Decision};
use crate::config::Config;
use crate::event::{CcwEvent, SeqEvent};
use crate::retry::{backoff_ms, classify_message, classify_result, RetryDecision, TransientKind};
use crate::spawn::{claude_bin, SpawnConfig, ToolConfig};
use crate::store::{SessionRow, Store};

/// How a session picks its account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AccountChoice {
    /// Choose among the budget's candidates at admission time.
    Auto,
    /// A specific registered account name.
    Named(String),
}

impl Default for AccountChoice {
    fn default() -> Self {
        AccountChoice::Auto
    }
}

/// The full session-start spec (WS `sessions.start` params).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSpec {
    pub cwd: String,
    pub model: String,
    #[serde(default)]
    pub account: AccountChoice,
    #[serde(default)]
    pub budget_id: Option<String>,
    #[serde(default)]
    pub append_system_prompt: Option<String>,
    #[serde(default)]
    pub add_dirs: Vec<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub tools: ToolConfig,
    #[serde(default)]
    pub max_budget_usd: Option<f64>,
    #[serde(default)]
    pub fallback_model: Option<String>,
    /// ROLLUP STUB (INTENT #208): a rollup ref names a prompt/plugin assembly
    /// CCW will build in a future version. v1 REJECTS a non-null ref with
    /// not-implemented — the named seam, documented, unbuilt.
    #[serde(default)]
    pub rollup: Option<String>,
}

/// Live liveness state for a session.
#[derive(Debug, Default)]
struct Liveness {
    parent_running: bool,
    /// agentId → its live-transcript output file (for mtime-churn heartbeat).
    outstanding: HashMap<String, Option<String>>,
    /// last observed mtime per outstanding agent (for idle detection).
    last_mtime: HashMap<String, SystemTime>,
    result_seen: bool,
}

impl Liveness {
    fn busy(&self) -> bool {
        self.parent_running || !self.outstanding.is_empty()
    }
    fn running(&self) -> Vec<String> {
        let mut v: Vec<String> = self.outstanding.keys().cloned().collect();
        v.sort();
        v
    }
}

/// Per-session runtime state.
struct SessionState {
    id: String,
    spec: SessionSpec,
    account: StdMutex<String>,
    config_dir: StdMutex<Option<String>>,
    seq: AtomicU64,
    resumable: AtomicBool,
    turn_running: AtomicBool,
    live: StdMutex<Liveness>,
    cancel: Notify,
}

/// Tunables (env-overridable so tests run fast; production keeps the defaults).
#[derive(Debug, Clone)]
struct Tunables {
    heartbeat: Duration,
    subagent_idle: Duration,
    retry_wait_cap: Duration,
    max_attempts: u32,
    budget_max_polls: u32,
    budget_poll: Duration,
}

fn env_ms(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn env_u32(key: &str, default: u32) -> u32 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

impl Default for Tunables {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_millis(env_ms("CCW_HEARTBEAT_MS", 1000)),
            subagent_idle: Duration::from_millis(env_ms("CCW_SUBAGENT_IDLE_MS", 15_000)),
            retry_wait_cap: Duration::from_millis(env_ms("CCW_RETRY_WAIT_CAP_MS", 60_000)),
            max_attempts: env_u32("CCW_MAX_ATTEMPTS", 3),
            budget_max_polls: env_u32("CCW_BUDGET_MAX_POLLS", 0),
            budget_poll: Duration::from_millis(env_ms("CCW_BUDGET_POLL_MS", 60_000)),
        }
    }
}

/// The daemon's session registry + event firehose.
#[derive(Clone)]
pub struct SessionManager {
    pub store: Store,
    pub registry: Arc<Registry>,
    pub config: Arc<Config>,
    pub firehose: broadcast::Sender<SeqEvent>,
    sessions: Arc<StdMutex<HashMap<String, Arc<SessionState>>>>,
    tunables: Tunables,
}

impl SessionManager {
    pub fn new(store: Store, registry: Registry, config: Config) -> Self {
        let (firehose, _rx) = broadcast::channel(4096);
        Self {
            store,
            registry: Arc::new(registry),
            config: Arc::new(config),
            firehose,
            sessions: Arc::new(StdMutex::new(HashMap::new())),
            tunables: Tunables::default(),
        }
    }

    /// Subscribe to the event firehose (subscribers filter by session id).
    pub fn subscribe(&self) -> broadcast::Receiver<SeqEvent> {
        self.firehose.subscribe()
    }

    /// Resolve an [`AccountChoice`] to `(name, config_dir)`.
    fn resolve_account(&self, choice: &AccountChoice, budget_id: Option<&str>) -> Result<(String, Option<String>)> {
        let names = self.registry.names();
        let name = match choice {
            AccountChoice::Named(n) => n.clone(),
            AccountChoice::Auto => {
                // Prefer the budget's first candidate, else the first registered.
                let candidates = match budget_id.and_then(|b| self.store.get_budget(b).ok().flatten()) {
                    Some(rules) => rules.candidate_accounts(&names),
                    None => names.clone(),
                };
                candidates
                    .into_iter()
                    .next()
                    .or_else(|| names.first().cloned())
                    .ok_or_else(|| SubstrateError::Config("no registered accounts".into()))?
            }
        };
        let cd = self
            .registry
            .get(&name)
            .and_then(|a| config_dir_of(a))
            .map(|p| p.to_string_lossy().to_string());
        Ok((name, cd))
    }

    /// Create a session (persist row, emit `session_started`). Does not run a
    /// turn. Rejects a non-null rollup ref (v1 stub, INTENT #208).
    pub fn start_session(&self, spec: SessionSpec) -> Result<String> {
        if spec.rollup.is_some() {
            return Err(SubstrateError::Config(
                "rollup assembly is not implemented in CCW v1 (stubbed seam — INTENT #208)".into(),
            ));
        }
        let (account, config_dir) = self.resolve_account(&spec.account, spec.budget_id.as_deref())?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.store.insert_session(&SessionRow {
            id: id.clone(),
            account: Some(account.clone()),
            cwd: Some(spec.cwd.clone()),
            model: Some(spec.model.clone()),
            budget_id: spec.budget_id.clone(),
            spec_json: serde_json::to_string(&spec).map_err(SubstrateError::from)?,
            status: "idle".into(),
            last_seq: 0,
            created_at: now.clone(),
            updated_at: now,
        })?;
        let state = Arc::new(SessionState {
            id: id.clone(),
            spec: spec.clone(),
            account: StdMutex::new(account.clone()),
            config_dir: StdMutex::new(config_dir),
            seq: AtomicU64::new(0),
            resumable: AtomicBool::new(false),
            turn_running: AtomicBool::new(false),
            live: StdMutex::new(Liveness::default()),
            cancel: Notify::new(),
        });
        self.sessions.lock().unwrap().insert(id.clone(), state.clone());
        self.emit(
            &state,
            CcwEvent::SessionStarted {
                session_id: id.clone(),
                account,
                cwd: spec.cwd.clone(),
                model: spec.model.clone(),
            },
        );
        Ok(id)
    }

    /// Look up (or rehydrate from the ledger) a session by id.
    fn get_state(&self, id: &str) -> Result<Arc<SessionState>> {
        if let Some(s) = self.sessions.lock().unwrap().get(id) {
            return Ok(s.clone());
        }
        // Rehydrate a persisted session (resume after a daemon restart).
        let row = self
            .store
            .get_session(id)?
            .ok_or_else(|| SubstrateError::Config(format!("no such session: {id}")))?;
        let spec: SessionSpec = serde_json::from_str(&row.spec_json).map_err(SubstrateError::from)?;
        let max_seq = self.store.max_seq(id)?;
        let (account, config_dir) = (
            row.account.clone().unwrap_or_default(),
            self.registry
                .get(&row.account.clone().unwrap_or_default())
                .and_then(|a| config_dir_of(a))
                .map(|p| p.to_string_lossy().to_string()),
        );
        let state = Arc::new(SessionState {
            id: id.to_string(),
            spec,
            account: StdMutex::new(account),
            config_dir: StdMutex::new(config_dir),
            seq: AtomicU64::new(max_seq),
            resumable: AtomicBool::new(max_seq > 0),
            turn_running: AtomicBool::new(false),
            live: StdMutex::new(Liveness::default()),
            cancel: Notify::new(),
        });
        self.sessions.lock().unwrap().insert(id.to_string(), state.clone());
        Ok(state)
    }

    /// Cancel a running turn (kills the child on the next stream select).
    pub fn cancel(&self, id: &str) -> Result<()> {
        let state = self.get_state(id)?;
        state.cancel.notify_waiters();
        Ok(())
    }

    /// Kick off a user turn on a session (spawns the runner as a background
    /// task; events stream asynchronously). Returns immediately.
    pub fn send_turn(&self, id: &str, prompt: String) -> Result<()> {
        let state = self.get_state(id)?;
        if state.turn_running.swap(true, Ordering::SeqCst) {
            return Err(SubstrateError::Config(format!(
                "session {id} already has a turn in flight"
            )));
        }
        let mgr = self.clone();
        tokio::spawn(async move {
            mgr.run_turn(state, prompt).await;
        });
        Ok(())
    }

    // ── event emission ───────────────────────────────────────────────────

    fn emit(&self, state: &SessionState, event: CcwEvent) {
        let seq = state.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let ev = SeqEvent {
            session_id: state.id.clone(),
            seq,
            event,
        };
        if let Err(e) = self.store.append_event(&ev) {
            tracing::warn!(error = %e, "failed to persist event");
        }
        let status = self
            .live_status(state);
        let _ = self.store.update_session_status(&state.id, status, seq);
        let _ = self.firehose.send(ev); // ok if no subscribers
    }

    fn live_status(&self, state: &SessionState) -> &'static str {
        if state.live.lock().unwrap().busy() {
            "busy"
        } else if state.turn_running.load(Ordering::SeqCst) {
            "busy"
        } else {
            "idle"
        }
    }

    fn emit_liveness(&self, state: &SessionState) {
        let (busy, parent, running) = {
            let l = state.live.lock().unwrap();
            (l.busy(), l.parent_running, l.running())
        };
        self.emit(
            state,
            CcwEvent::Liveness {
                busy,
                parent_running: parent,
                running_subagents: running,
            },
        );
    }

    // ── the turn runner ──────────────────────────────────────────────────

    async fn run_turn(&self, state: Arc<SessionState>, prompt: String) {
        // Pre-spawn budget admission.
        match self.admit(&state).await {
            AdmitOutcome::Proceed => {}
            AdmitOutcome::Pending => {
                state.turn_running.store(false, Ordering::SeqCst);
                let _ = self.store.update_session_status(&state.id, "idle", state.seq.load(Ordering::SeqCst));
                return;
            }
        }

        let mut attempt: u32 = 1;
        loop {
            let first_turn = !state.resumable.load(Ordering::SeqCst);
            self.emit(
                &state,
                CcwEvent::TurnStarted {
                    prompt_preview: prompt.chars().take(120).collect(),
                    attempt,
                },
            );

            let outcome = self.run_once(&state, &prompt, first_turn, attempt).await;

            match outcome.retry {
                Some(RetryDecision::Retry { kind, reason }) if attempt < self.tunables.max_attempts => {
                    // 429: prefer switching to an alternate account with headroom.
                    if kind == TransientKind::RateLimit && self.try_switch_account(&state, &reason) {
                        attempt += 1;
                        continue;
                    }
                    let wait = self.retry_wait(&kind, outcome.reset.as_deref());
                    self.emit(
                        &state,
                        CcwEvent::RetryStarted {
                            reason,
                            attempt: attempt + 1,
                            wait_ms: Some(wait.as_millis() as u64),
                            eta: outcome.reset.clone(),
                        },
                    );
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                    continue;
                }
                Some(RetryDecision::RestartFresh { reason }) if attempt < self.tunables.max_attempts => {
                    state.resumable.store(false, Ordering::SeqCst);
                    self.emit(
                        &state,
                        CcwEvent::RetryStarted {
                            reason,
                            attempt: attempt + 1,
                            wait_ms: Some(0),
                            eta: None,
                        },
                    );
                    attempt += 1;
                    continue;
                }
                Some(RetryDecision::Fatal { reason }) => {
                    self.emit(&state, CcwEvent::Error { message: reason });
                    break;
                }
                _ => break, // success, or attempts exhausted
            }
        }

        // Turn task done. If async subagents are still outstanding, keep a
        // detached liveness poller running until they go quiet (recon rule:
        // thread is IDLE only when parent result arrived AND every async agent
        // is mtime-stable). Otherwise mark idle now.
        state.turn_running.store(false, Ordering::SeqCst);
        let still_busy = state.live.lock().unwrap().busy();
        if still_busy {
            let mgr = self.clone();
            let st = state.clone();
            tokio::spawn(async move {
                mgr.poll_subagents_until_idle(st).await;
            });
        } else {
            let _ = self.store.update_session_status(&state.id, "idle", state.seq.load(Ordering::SeqCst));
        }
    }

    /// One spawn+parse+wait cycle.
    async fn run_once(&self, state: &SessionState, prompt: &str, first_turn: bool, attempt: u32) -> TurnOutcome {
        let cfg = self.spawn_config(state);
        let config_dir = state.config_dir.lock().unwrap().clone();
        let args = cfg.build_args(first_turn);

        let mut cmd = Command::new(claude_bin());
        cmd.args(&args);
        cmd.current_dir(&state.spec.cwd);
        for (k, v) in cfg.env(config_dir.as_deref()) {
            match v {
                Some(val) => {
                    cmd.env(k, val);
                }
                None => {
                    cmd.env_remove(k);
                }
            }
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return TurnOutcome {
                    exit_code: None,
                    retry: Some(RetryDecision::Fatal {
                        reason: format!("spawning claude: {e}"),
                    }),
                    reset: None,
                };
            }
        };

        // Feed the prompt on stdin, then close it.
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.write_all(b"\n").await;
            drop(stdin);
        }

        // Drain stderr concurrently (flag/parse errors land here).
        let stderr_buf = Arc::new(StdMutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            let buf = stderr_buf.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    let mut b = buf.lock().unwrap();
                    b.push_str(&l);
                    b.push('\n');
                }
            });
        }

        // Mark parent running, start the liveness heartbeat.
        {
            let mut l = state.live.lock().unwrap();
            l.parent_running = true;
            l.result_seen = false;
        }
        self.emit_liveness(state);
        let heartbeat = self.spawn_heartbeat(state);

        let stdout = child.stdout.take().expect("piped stdout");
        let mut lines = BufReader::new(stdout).lines();
        let mut retry: Option<RetryDecision> = None;
        let mut last_reset: Option<String> = None;
        let mut cancelled = false;

        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(l)) => {
                            for ev in crate::stream::parse_raw_line(&l) {
                                // Interpret liveness / retry signals before emit.
                                self.interpret(state, &ev, &mut retry, &mut last_reset);
                                self.emit(state, ev);
                            }
                        }
                        Ok(None) => break, // EOF
                        Err(_) => break,
                    }
                }
                _ = state.cancel.notified() => {
                    cancelled = true;
                    let _ = child.start_kill();
                    break;
                }
            }
        }

        let status = child.wait().await.ok();
        heartbeat.abort();

        // Parent no longer running.
        {
            let mut l = state.live.lock().unwrap();
            l.parent_running = false;
        }
        self.emit_liveness(state);

        let exit_code = status.and_then(|s| s.code());
        self.emit(
            state,
            CcwEvent::SessionEnded {
                session_id: state.id.clone(),
                exit_code,
            },
        );

        // First successful spawn → future turns resume.
        state.resumable.store(true, Ordering::SeqCst);

        if cancelled {
            return TurnOutcome { exit_code, retry: None, reset: None };
        }

        // Classify a non-zero exit with no in-stream retry signal via stderr.
        if retry.is_none() {
            if let Some(code) = exit_code {
                if code != 0 {
                    let err = stderr_buf.lock().unwrap().clone();
                    let text = if err.trim().is_empty() {
                        format!("claude exited with code {code}")
                    } else {
                        err
                    };
                    if attempt < self.tunables.max_attempts {
                        retry = Some(classify_message(&text));
                    } else {
                        retry = Some(RetryDecision::Fatal { reason: text });
                    }
                }
            }
        }

        TurnOutcome { exit_code, retry, reset: last_reset }
    }

    /// Update liveness + capture retry signals from one event (before emit).
    fn interpret(
        &self,
        state: &SessionState,
        ev: &CcwEvent,
        retry: &mut Option<RetryDecision>,
        last_reset: &mut Option<String>,
    ) {
        match ev {
            CcwEvent::AgentLaunched { agent_id, is_async, status: st, output_file, .. } => {
                if *is_async && st != "completed" {
                    let mut l = state.live.lock().unwrap();
                    l.outstanding.insert(agent_id.clone(), output_file.clone());
                    l.last_mtime.insert(agent_id.clone(), SystemTime::now());
                }
                // Sync completion needs no tracking (result already returned).
            }
            CcwEvent::AgentCompleted { agent_id } => {
                let mut l = state.live.lock().unwrap();
                l.outstanding.remove(agent_id);
                l.last_mtime.remove(agent_id);
            }
            CcwEvent::LimitHit { reset, .. } => {
                *last_reset = reset.clone();
                *retry = Some(RetryDecision::Retry {
                    kind: TransientKind::RateLimit,
                    reason: "rate limit (429)".into(),
                });
            }
            CcwEvent::ResultTurn { subtype, is_error, .. } => {
                {
                    let mut l = state.live.lock().unwrap();
                    l.result_seen = true;
                }
                // Emit a liveness beat right at result so subscribers see BUSY
                // if async subagents are still outstanding (the idle-illusion
                // guard) — done after this fn returns via emit_liveness below.
                if *is_error && retry.is_none() {
                    *retry = Some(classify_result(subtype));
                }
                // Force a liveness beat now.
                self.emit_liveness(state);
            }
            _ => {}
        }
    }

    fn spawn_config(&self, state: &SessionState) -> SpawnConfig {
        SpawnConfig {
            model: state.spec.model.clone(),
            session_id: state.id.clone(),
            cwd: state.spec.cwd.clone(),
            append_system_prompt: state.spec.append_system_prompt.clone(),
            add_dirs: state.spec.add_dirs.clone(),
            permission_mode: state.spec.permission_mode.clone(),
            tools: state.spec.tools.clone(),
            max_budget_usd: state.spec.max_budget_usd,
            fallback_model: state.spec.fallback_model.clone(),
        }
    }

    /// Heartbeat: emit a Liveness event every `heartbeat` while the turn runs,
    /// and check outstanding async subagents' transcript mtime for idle.
    fn spawn_heartbeat(&self, state: &SessionState) -> tokio::task::JoinHandle<()> {
        let mgr = self.clone();
        // We can't move &SessionState into the task; clone the Arc via registry.
        let sid = state.id.clone();
        let interval = self.tunables.heartbeat;
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let st = match mgr.sessions.lock().unwrap().get(&sid).cloned() {
                    Some(s) => s,
                    None => break,
                };
                mgr.sweep_subagents(&st);
                mgr.emit_liveness(&st);
                let running = st.turn_running.load(Ordering::SeqCst);
                let busy = st.live.lock().unwrap().busy();
                if !running && !busy {
                    break;
                }
            }
        })
    }

    /// After the turn task ends but async subagents remain, poll until idle.
    async fn poll_subagents_until_idle(&self, state: Arc<SessionState>) {
        loop {
            tokio::time::sleep(self.tunables.heartbeat).await;
            self.sweep_subagents(&state);
            let busy = state.live.lock().unwrap().busy();
            self.emit_liveness(&state);
            if !busy {
                let _ = self
                    .store
                    .update_session_status(&state.id, "idle", state.seq.load(Ordering::SeqCst));
                break;
            }
        }
    }

    /// Mark an async subagent complete when its transcript stops churning
    /// (mtime stable past the idle window), or when its output file is gone.
    fn sweep_subagents(&self, state: &SessionState) {
        let now = SystemTime::now();
        let idle = self.tunables.subagent_idle;
        let mut completed: Vec<String> = Vec::new();
        {
            let mut l = state.live.lock().unwrap();
            let agents: Vec<(String, Option<String>)> =
                l.outstanding.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for (agent_id, out) in agents {
                let mtime = out
                    .as_ref()
                    .and_then(|p| std::fs::metadata(p).ok())
                    .and_then(|m| m.modified().ok());
                match mtime {
                    Some(mt) => {
                        let prev = l.last_mtime.get(&agent_id).copied();
                        if prev != Some(mt) {
                            l.last_mtime.insert(agent_id.clone(), mt);
                        }
                        // Stable past the idle window ⇒ done.
                        if now.duration_since(mt).map(|d| d > idle).unwrap_or(false) {
                            completed.push(agent_id.clone());
                        }
                    }
                    None => {
                        // No readable transcript: use our own last_mtime marker.
                        let started = l.last_mtime.get(&agent_id).copied().unwrap_or(now);
                        if now.duration_since(started).map(|d| d > idle).unwrap_or(false) {
                            completed.push(agent_id.clone());
                        }
                    }
                }
            }
        }
        for agent_id in completed {
            {
                let mut l = state.live.lock().unwrap();
                l.outstanding.remove(&agent_id);
                l.last_mtime.remove(&agent_id);
            }
            self.emit(state, CcwEvent::AgentCompleted { agent_id });
        }
    }

    // ── retry policy ─────────────────────────────────────────────────────

    fn retry_wait(&self, kind: &TransientKind, reset: Option<&str>) -> Duration {
        let cap = self.tunables.retry_wait_cap;
        match kind {
            TransientKind::RateLimit => {
                // Honor the parsed reset time when known, capped for safety.
                let by_reset = reset
                    .and_then(crate::reset::parse_reset)
                    .and_then(|r| r.resolve(chrono::Local::now().naive_local()))
                    .and_then(|t| (t - chrono::Local::now().naive_local()).to_std().ok());
                by_reset.unwrap_or(cap).min(cap)
            }
            _ => Duration::from_millis(backoff_ms(1)).min(cap),
        }
    }

    /// Switch to an alternate registered account (429 mitigation). Returns true
    /// when a switch happened (an `account_switched` event is emitted).
    fn try_switch_account(&self, state: &SessionState, reason: &str) -> bool {
        let current = state.account.lock().unwrap().clone();
        let budget = state.spec.budget_id.as_deref();
        let names = self.registry.names();
        let candidates = match budget.and_then(|b| self.store.get_budget(b).ok().flatten()) {
            Some(rules) => rules.candidate_accounts(&names),
            None => names.clone(),
        };
        let next = candidates.into_iter().find(|n| *n != current);
        if let Some(next) = next {
            let cd = self
                .registry
                .get(&next)
                .and_then(|a| config_dir_of(a))
                .map(|p| p.to_string_lossy().to_string());
            *state.account.lock().unwrap() = next.clone();
            *state.config_dir.lock().unwrap() = cd;
            // A fresh account has no prior session for this id — start fresh.
            state.resumable.store(false, Ordering::SeqCst);
            self.emit(
                state,
                CcwEvent::AccountSwitched {
                    from: current,
                    to: next,
                    reason: reason.to_string(),
                },
            );
            true
        } else {
            false
        }
    }

    // ── budget admission (pre-spawn) ─────────────────────────────────────

    async fn admit(&self, state: &Arc<SessionState>) -> AdmitOutcome {
        let Some(budget_id) = state.spec.budget_id.clone() else {
            return AdmitOutcome::Proceed;
        };
        let Some(rules) = self.store.get_budget(&budget_id).ok().flatten() else {
            return AdmitOutcome::Proceed; // unknown budget → fail-open
        };

        let mut polls: u32 = 0;
        loop {
            let account = state.account.lock().unwrap().clone();
            let config_dir = state.config_dir.lock().unwrap().clone();
            let store = self.store.clone();
            let budget_id2 = budget_id.clone();
            let acct2 = account.clone();
            // The /usage probe is a blocking child call — run it off the reactor.
            let usage = tokio::task::spawn_blocking(move || {
                let snap = crate::claude::usage_snapshot(
                    config_dir.as_deref().map(std::path::Path::new),
                    Duration::from_secs(30),
                )
                .unwrap_or_default();
                let week_start = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
                let consumed = store.weekly_consumed_tokens(&budget_id2, &week_start).unwrap_or(0) as f64;
                let cal = store.get_calibration(&acct2, crate::run::WEEKLY_POOL).ok().flatten();
                let tpp = cal.map(|c| c.tokens_per_percent);
                AccountUsage {
                    account: acct2,
                    session_pct: snap.session_pct(),
                    weekly_consumed_tokens: consumed,
                    weekly_limit_tokens: crate::budget::weekly_limit_tokens(rules.weekly_pct, tpp),
                    resets: snap.resets(),
                }
            })
            .await;

            let usage = match usage {
                Ok(u) => u,
                Err(_) => return AdmitOutcome::Proceed, // join error → fail-open
            };

            match decide(&rules, std::slice::from_ref(&usage), chrono::Local::now().naive_local()) {
                Decision::Proceed { account } => {
                    // decide() may have chosen a different candidate name; keep ours.
                    let _ = account;
                    return AdmitOutcome::Proceed;
                }
                Decision::Blocked { reason, eta } => {
                    self.emit(state, CcwEvent::BudgetPending { reason, eta });
                    if polls >= self.tunables.budget_max_polls {
                        return AdmitOutcome::Pending;
                    }
                    polls += 1;
                    tokio::time::sleep(self.tunables.budget_poll).await;
                }
            }
        }
    }
}

enum AdmitOutcome {
    Proceed,
    Pending,
}

struct TurnOutcome {
    #[allow(dead_code)]
    exit_code: Option<i32>,
    retry: Option<RetryDecision>,
    reset: Option<String>,
}
