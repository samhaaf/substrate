//! `CcwEvent` — the full event taxonomy CCW surfaces over WebSocket.
//!
//! CCW v1's whole job (INTENT #208) is to expose EVERYTHING about Claude Code
//! activity: every in-thread event (assistant text, tool_use/tool_result,
//! thinking, subagent spawn/launch/completion, per-message + per-turn usage,
//! compaction, the raw `stream_event`/`agent_listing_delta`/`init` frames aui
//! drops), the wrapper's own liveness heartbeats, AND the wrapper meta-events
//! (budget pending, account switch, retry, limit hit, calibration).
//!
//! The taxonomy is derived from `reports/ccw-v1-recon.md` Deliverable 1 (the
//! stream-json line shapes) and the operator's "full event exposure" mandate.
//! Every stream line maps to a concrete variant — nothing falls into an
//! "unknown, dropped" bucket (aui's blind spot); genuinely-unrecognized lines
//! surface as [`CcwEvent::Unknown`] carrying the raw JSON verbatim.
//!
//! Wire form: internally tagged on `kind`, so a subscriber switches on one
//! discriminator (`{"kind":"assistant_text","text":"…"}`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One CCW event. Every event streamed over WS and persisted to the event log
/// is one of these. `#[serde(other)]`-style tolerance is provided by the
/// [`CcwEvent::Unknown`] arm for forward compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CcwEvent {
    // ── Session lifecycle (CCW-native) ──────────────────────────────────
    /// A session (thread) has started; the first event on any session stream.
    SessionStarted {
        session_id: String,
        account: String,
        cwd: String,
        model: String,
    },
    /// A user turn was accepted and claude is being spawned.
    TurnStarted { prompt_preview: String, attempt: u32 },
    /// The session's claude process exited (turn or session end).
    SessionEnded {
        session_id: String,
        exit_code: Option<i32>,
    },

    // ── system events ───────────────────────────────────────────────────
    /// `type:"system", subtype:"init"` — the full tool/model/mcp roster aui
    /// logs only at debug. CCW surfaces it whole.
    SystemInit {
        #[serde(default)]
        claude_session_id: Option<String>,
        #[serde(default)]
        cwd: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        tools: Vec<String>,
        #[serde(default)]
        mcp_servers: Value,
        #[serde(default)]
        permission_mode: Option<String>,
        #[serde(default)]
        slash_commands: Vec<String>,
        #[serde(default)]
        api_key_source: Option<String>,
        #[serde(default)]
        agents: Value,
    },
    /// `type:"system", subtype:"compact_boundary"` — a compaction happened.
    Compacted {
        trigger: String,
        #[serde(default)]
        pre_tokens: Option<u64>,
    },

    // ── assistant content blocks ────────────────────────────────────────
    /// A complete assistant text block (verbatim; no partial prefixes since we
    /// run without `--include-partial-messages`).
    AssistantText {
        text: String,
        /// Non-null when this text was emitted by a subagent sidechain.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    /// An extended-thinking block (aui drops these — CCW keeps them).
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    /// A `tool_use` block.
    ToolUse {
        id: String,
        name: String,
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    /// A `tool_result` fed back in (full content, not a 200-char preview).
    ToolResult {
        tool_use_id: String,
        content: Value,
        #[serde(default)]
        is_error: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_use_id: Option<String>,
    },
    /// Per-assistant-message usage (streams before the turn aggregate).
    MessageUsage {
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_creation_tokens: u64,
    },

    // ── subagent liveness (the operator's CRITICAL requirement) ─────────
    /// An `Agent` (subagent) tool_use was observed in the parent stream.
    AgentSpawn {
        tool_use_id: String,
        #[serde(default)]
        subagent_type: Option<String>,
        #[serde(default)]
        description: Option<String>,
        run_in_background: bool,
    },
    /// The tool_result for an `Agent`: sync `completed`, or async
    /// `async_launched` (the parent turn can finish while this keeps running).
    AgentLaunched {
        tool_use_id: String,
        agent_id: String,
        is_async: bool,
        status: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_file: Option<String>,
    },
    /// A tracked async subagent has gone quiet (transcript mtime stable past the
    /// idle window) — CCW's derived "this subagent is done" signal.
    AgentCompleted { agent_id: String },
    /// The push-form background-agent roster delta (aui drops it).
    AgentListingDelta { raw: Value },
    /// Liveness heartbeat: is this session still doing work? `busy` stays true
    /// while the parent process runs OR any async subagent is outstanding, so a
    /// wrapper never mistakes "parent result arrived" for "thread idle."
    Liveness {
        busy: bool,
        parent_running: bool,
        running_subagents: Vec<String>,
    },

    // ── raw passthrough (nothing dropped) ───────────────────────────────
    /// `type:"stream_event"` partial deltas (only with partial-messages; we run
    /// without, but the arm exists so nothing is ever silently dropped).
    StreamEvent { raw: Value },
    /// A line whose `type` CCW does not recognize — surfaced, never dropped.
    Unknown { raw: Value },

    // ── turn result / telemetry ─────────────────────────────────────────
    /// `type:"result"` — once per turn. Bill/telemetry aggregate.
    ResultTurn {
        subtype: String,
        is_error: bool,
        #[serde(default)]
        total_cost_usd: Option<f64>,
        #[serde(default)]
        num_turns: Option<u64>,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        stop_reason: Option<String>,
        /// Per-model usage from `modelUsage` (input/output/cache totals).
        #[serde(default)]
        model_usage: Value,
    },

    // ── wrapper meta-events (CCW's own governance surface) ──────────────
    /// Admission blocked on budget; the loop UI shows `eta`.
    BudgetPending {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        eta: Option<String>,
    },
    /// CCW switched the session to a different account (limit/budget driven).
    AccountSwitched {
        from: String,
        to: String,
        reason: String,
    },
    /// A retry began after a transient failure (429 / 5xx / conn-drop).
    RetryStarted {
        reason: String,
        attempt: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        wait_ms: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        eta: Option<String>,
    },
    /// A rate/usage limit was hit (surfaced the moment it streams, not only at
    /// turn end — aui's blind spot).
    LimitHit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pool: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reset: Option<String>,
        #[serde(default)]
        status: Option<u64>,
        #[serde(default)]
        text: String,
    },
    /// The tokens-per-percent calibration for an (account, pool) was updated.
    CalibrationUpdated {
        account: String,
        pool: String,
        tokens_per_percent: f64,
        sample_count: u64,
    },

    // ── errors ──────────────────────────────────────────────────────────
    /// A non-retryable failure surfaced to subscribers.
    Error { message: String },
}

impl CcwEvent {
    /// The variant tag string (matches the serialized `kind`), for the event
    /// log's cheap-filter column.
    pub fn kind(&self) -> &'static str {
        match self {
            CcwEvent::SessionStarted { .. } => "session_started",
            CcwEvent::TurnStarted { .. } => "turn_started",
            CcwEvent::SessionEnded { .. } => "session_ended",
            CcwEvent::SystemInit { .. } => "system_init",
            CcwEvent::Compacted { .. } => "compacted",
            CcwEvent::AssistantText { .. } => "assistant_text",
            CcwEvent::Thinking { .. } => "thinking",
            CcwEvent::ToolUse { .. } => "tool_use",
            CcwEvent::ToolResult { .. } => "tool_result",
            CcwEvent::MessageUsage { .. } => "message_usage",
            CcwEvent::AgentSpawn { .. } => "agent_spawn",
            CcwEvent::AgentLaunched { .. } => "agent_launched",
            CcwEvent::AgentCompleted { .. } => "agent_completed",
            CcwEvent::AgentListingDelta { .. } => "agent_listing_delta",
            CcwEvent::Liveness { .. } => "liveness",
            CcwEvent::StreamEvent { .. } => "stream_event",
            CcwEvent::Unknown { .. } => "unknown",
            CcwEvent::ResultTurn { .. } => "result_turn",
            CcwEvent::BudgetPending { .. } => "budget_pending",
            CcwEvent::AccountSwitched { .. } => "account_switched",
            CcwEvent::RetryStarted { .. } => "retry_started",
            CcwEvent::LimitHit { .. } => "limit_hit",
            CcwEvent::CalibrationUpdated { .. } => "calibration_updated",
            CcwEvent::Error { .. } => "error",
        }
    }
}

/// An event stamped with its session id + sequence number — the unit pushed to
/// subscribers and stored in the event log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeqEvent {
    pub session_id: String,
    pub seq: u64,
    pub event: CcwEvent,
}
