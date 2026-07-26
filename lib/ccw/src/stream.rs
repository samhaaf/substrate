//! stream-json line parser: one claude `--output-format stream-json` line →
//! zero or more [`CcwEvent`]s.
//!
//! This is the pure translation layer (`reports/ccw-v1-recon.md` Deliverable 1).
//! It is deliberately **stateless**: it maps a single parsed JSON `Value` to the
//! events it implies. The session runtime ([`crate::session`]) owns the stateful
//! interpretation (liveness accounting, retry classification) by inspecting the
//! events this emits — which keeps the taxonomy mapping trivially unit-testable.
//!
//! Every `type` discriminator the recon enumerated has an arm; anything else
//! becomes [`CcwEvent::Unknown`] so nothing is dropped (aui's blind spot).

use serde_json::Value;

use crate::event::CcwEvent;

/// True when an event is the synthetic 429 / API-error assistant message
/// (`reports/cc-recon.md` §3): `isApiErrorMessage:true` / `error:"rate_limit"` /
/// `apiErrorStatus:429` / `message.model:"<synthetic>"`.
pub fn is_api_error(v: &Value) -> bool {
    v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true)
        || v.get("error").and_then(Value::as_str) == Some("rate_limit")
        || v.get("apiErrorStatus").is_some()
}

fn u64_at(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Flatten a `message.content` (array of blocks or a bare string) to text.
fn flatten_text(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => {
            let mut buf = String::new();
            for it in items {
                if let Some(t) = it.get("text").and_then(Value::as_str) {
                    if !buf.is_empty() {
                        buf.push(' ');
                    }
                    buf.push_str(t);
                }
            }
            buf
        }
        _ => String::new(),
    }
}

/// Parse one stream-json line (already JSON-decoded) into CCW events.
pub fn parse_line(v: &Value) -> Vec<CcwEvent> {
    let ty = v.get("type").and_then(Value::as_str).unwrap_or("");
    let parent_tool_use_id = str_at(v, "parent_tool_use_id");

    match ty {
        "system" => parse_system(v),
        "assistant" => parse_assistant(v, parent_tool_use_id),
        "user" => parse_user(v, parent_tool_use_id),
        "result" => vec![parse_result(v)],
        "stream_event" => vec![CcwEvent::StreamEvent { raw: v.clone() }],
        "agent_listing_delta" => vec![CcwEvent::AgentListingDelta { raw: v.clone() }],
        _ => vec![CcwEvent::Unknown { raw: v.clone() }],
    }
}

fn parse_system(v: &Value) -> Vec<CcwEvent> {
    match v.get("subtype").and_then(Value::as_str) {
        Some("init") => vec![CcwEvent::SystemInit {
            claude_session_id: str_at(v, "session_id"),
            cwd: str_at(v, "cwd"),
            model: str_at(v, "model"),
            tools: v
                .get("tools")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            mcp_servers: v.get("mcp_servers").cloned().unwrap_or(Value::Null),
            permission_mode: str_at(v, "permissionMode"),
            slash_commands: v
                .get("slash_commands")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            api_key_source: str_at(v, "apiKeySource"),
            agents: v.get("agents").cloned().unwrap_or(Value::Null),
        }],
        Some("compact_boundary") => {
            let meta = v.get("compactMetadata");
            vec![CcwEvent::Compacted {
                trigger: meta
                    .and_then(|m| m.get("trigger"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string(),
                pre_tokens: meta.and_then(|m| m.get("preTokens")).and_then(Value::as_u64),
            }]
        }
        _ => vec![CcwEvent::Unknown { raw: v.clone() }],
    }
}

fn parse_assistant(v: &Value, parent: Option<String>) -> Vec<CcwEvent> {
    // The synthetic 429 / API error arrives as an assistant event.
    if is_api_error(v) {
        let text = v.get("message").map(flatten_text).unwrap_or_default();
        let reset = crate::transcript::parse_limit_reset(&text).map(|r| r.raw);
        return vec![CcwEvent::LimitHit {
            pool: None,
            reset,
            status: v.get("apiErrorStatus").and_then(Value::as_u64),
            text,
        }];
    }

    let mut out = Vec::new();
    let Some(msg) = v.get("message") else {
        return vec![CcwEvent::Unknown { raw: v.clone() }];
    };

    if let Some(blocks) = msg.get("content").and_then(Value::as_array) {
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(Value::as_str) {
                        out.push(CcwEvent::AssistantText {
                            text: t.to_string(),
                            parent_tool_use_id: parent.clone(),
                        });
                    }
                }
                Some("thinking") => {
                    if let Some(t) = b.get("thinking").and_then(Value::as_str) {
                        out.push(CcwEvent::Thinking {
                            text: t.to_string(),
                            parent_tool_use_id: parent.clone(),
                        });
                    }
                }
                Some("tool_use") => {
                    let name = b.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                    let id = b.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    // Subagent spawn: `Agent` (v2.1.201) or legacy `Task`.
                    if name == "Agent" || name == "Task" {
                        out.push(CcwEvent::AgentSpawn {
                            tool_use_id: id.clone(),
                            subagent_type: str_at(&input, "subagent_type"),
                            description: str_at(&input, "description"),
                            run_in_background: input
                                .get("run_in_background")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        });
                    }
                    out.push(CcwEvent::ToolUse {
                        id,
                        name,
                        input,
                        parent_tool_use_id: parent.clone(),
                    });
                }
                _ => {}
            }
        }
    }

    // Per-message usage (before the turn aggregate).
    if let Some(u) = msg.get("usage") {
        out.push(CcwEvent::MessageUsage {
            model: str_at(msg, "model").unwrap_or_else(|| "unknown".into()),
            input_tokens: u64_at(u, "input_tokens"),
            output_tokens: u64_at(u, "output_tokens"),
            cache_read_tokens: u64_at(u, "cache_read_input_tokens"),
            cache_creation_tokens: u64_at(u, "cache_creation_input_tokens"),
        });
    }

    if out.is_empty() {
        out.push(CcwEvent::Unknown { raw: v.clone() });
    }
    out
}

fn parse_user(v: &Value, parent: Option<String>) -> Vec<CcwEvent> {
    let mut out = Vec::new();

    // The rich `toolUseResult` distinguishes sync-completed vs async-launched
    // Agent results (the liveness-critical shape, recon Deliverable 2).
    if let Some(tur) = v.get("toolUseResult") {
        let status = str_at(tur, "status").unwrap_or_default();
        let is_async = tur.get("isAsync").and_then(Value::as_bool).unwrap_or(false);
        if let Some(agent_id) = str_at(tur, "agentId") {
            out.push(CcwEvent::AgentLaunched {
                tool_use_id: tur
                    .get("toolUseId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_default(),
                agent_id,
                is_async,
                status,
                output_file: str_at(tur, "outputFile"),
            });
        }
    }

    if let Some(blocks) = v.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) {
        for b in blocks {
            if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                out.push(CcwEvent::ToolResult {
                    tool_use_id: b
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    content: b.get("content").cloned().unwrap_or(Value::Null),
                    is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    parent_tool_use_id: parent.clone(),
                });
            }
        }
    }

    if out.is_empty() {
        out.push(CcwEvent::Unknown { raw: v.clone() });
    }
    out
}

fn parse_result(v: &Value) -> CcwEvent {
    CcwEvent::ResultTurn {
        subtype: str_at(v, "subtype").unwrap_or_default(),
        is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        total_cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
        num_turns: v.get("num_turns").and_then(Value::as_u64),
        duration_ms: v.get("duration_ms").and_then(Value::as_u64),
        stop_reason: str_at(v, "stop_reason"),
        model_usage: v.get("modelUsage").cloned().unwrap_or(Value::Null),
    }
}

/// Parse a raw stream-json line (string). Returns the CCW events, or a single
/// [`CcwEvent::Unknown`] wrapping the raw text when the line is not valid JSON
/// (never dropped).
pub fn parse_raw_line(line: &str) -> Vec<CcwEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str::<Value>(line) {
        Ok(v) => parse_line(&v),
        Err(_) => vec![CcwEvent::Unknown {
            raw: Value::String(line.to_string()),
        }],
    }
}
