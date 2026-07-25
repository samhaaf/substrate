//! Transcript usage extraction and 429 limit detection.
//!
//! After a wrapped run exits, we locate the newest `.jsonl` under
//! `<configdir>/projects/<escaped-cwd>/` (`cc-recon.md` §4) and parse it line by
//! line. Two things matter for the ledger:
//!
//! * **Per-model usage** — `assistant` events carry `message.usage` (snake_case,
//!   `cc-recon.md` §2) keyed by `message.model`; the terminal `result` event (when
//!   present) carries per-model `modelUsage` (camelCase) plus `total_cost_usd`.
//! * **429 rate-limit hits** — synthetic `assistant` events with
//!   `error:"rate_limit"`, `isApiErrorMessage:true`, `apiErrorStatus:429`, whose
//!   text is `You've hit your <limit> · resets <local time> (<tz>)`.
//!
//! We prefer `modelUsage` (the authoritative per-turn aggregate `aui-nav` bills
//! from) and fall back to summing `message.usage` when no result event is seen.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::reset::{parse_reset, ResetTime};

/// Per-model token + cost usage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cost_usd: f64,
}

impl ModelUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }
}

/// A detected 429 limit hit.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitHit {
    /// The synthetic message text.
    pub text: String,
    /// Parsed reset descriptor, if the prose carried one.
    pub reset: Option<ResetTime>,
}

/// The aggregate extracted from a transcript.
#[derive(Debug, Clone, Default)]
pub struct TranscriptSummary {
    /// Per-model usage keyed by model id.
    pub per_model: BTreeMap<String, ModelUsage>,
    pub limit_hits: Vec<LimitHit>,
    pub session_id: Option<String>,
    /// `total_cost_usd` from the terminal result event, if present.
    pub total_cost_usd: Option<f64>,
    /// Whether a `result`/`modelUsage` event supplied the per-model totals
    /// (vs. having been summed from assistant `message.usage`).
    pub from_result_event: bool,
}

impl TranscriptSummary {
    pub fn total_input_tokens(&self) -> u64 {
        self.per_model.values().map(|m| m.input_tokens).sum()
    }
    pub fn total_output_tokens(&self) -> u64 {
        self.per_model.values().map(|m| m.output_tokens).sum()
    }
    pub fn total_cache_read_tokens(&self) -> u64 {
        self.per_model.values().map(|m| m.cache_read_tokens).sum()
    }
    pub fn total_cache_creation_tokens(&self) -> u64 {
        self.per_model.values().map(|m| m.cache_creation_tokens).sum()
    }
    pub fn total_tokens(&self) -> u64 {
        self.per_model.values().map(|m| m.total_tokens()).sum()
    }
    /// Cost: the result event's `total_cost_usd` if present, else summed per-model.
    pub fn cost_usd(&self) -> f64 {
        self.total_cost_usd
            .unwrap_or_else(|| self.per_model.values().map(|m| m.cost_usd).sum())
    }
    pub fn limit_hit(&self) -> bool {
        !self.limit_hits.is_empty()
    }
}

fn u64_field(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// True when an event is a 429 rate-limit synthetic assistant message.
fn is_limit_event(v: &Value) -> bool {
    v.get("error").and_then(Value::as_str) == Some("rate_limit")
        || v.get("apiErrorStatus").and_then(Value::as_u64) == Some(429)
        || (v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true)
            && v.get("message")
                .and_then(|m| m.get("model"))
                .and_then(Value::as_str)
                == Some("<synthetic>"))
}

/// Extract the flat text from a `message.content` array (or string).
fn message_text(msg: &Value) -> Option<String> {
    match msg.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
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
            if buf.is_empty() {
                None
            } else {
                Some(buf)
            }
        }
        _ => None,
    }
}

/// Parse the local-time reset prose out of a 429 message body:
/// `You've hit your session limit · resets 2:40am (America/Chicago)`.
pub fn parse_limit_reset(text: &str) -> Option<ResetTime> {
    let idx = text.find("resets ")?;
    parse_reset(text[idx + "resets ".len()..].trim())
}

/// Parse a whole transcript (`.jsonl` body) into a [`TranscriptSummary`].
pub fn parse_transcript(body: &str) -> TranscriptSummary {
    let mut summary = TranscriptSummary::default();
    let mut summed: BTreeMap<String, ModelUsage> = BTreeMap::new();

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if let Some(sid) = v.get("sessionId").and_then(Value::as_str) {
            summary.session_id.get_or_insert_with(|| sid.to_string());
        }
        if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
            summary.session_id.get_or_insert_with(|| sid.to_string());
        }

        let ty = v.get("type").and_then(Value::as_str).unwrap_or("");

        // 429 limit detection.
        if is_limit_event(&v) {
            let text = v
                .get("message")
                .and_then(message_text)
                .or_else(|| v.get("text").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            let reset = parse_limit_reset(&text);
            summary.limit_hits.push(LimitHit { text, reset });
            continue;
        }

        // Terminal result event — authoritative per-model aggregate.
        if ty == "result" {
            if let Some(cost) = v.get("total_cost_usd").and_then(Value::as_f64) {
                summary.total_cost_usd = Some(cost);
            }
            if let Some(mu) = v.get("modelUsage").and_then(Value::as_object) {
                if !mu.is_empty() {
                    summary.from_result_event = true;
                    for (model, u) in mu {
                        let e = summary.per_model.entry(model.clone()).or_default();
                        e.input_tokens += u64_field(u, "inputTokens");
                        e.output_tokens += u64_field(u, "outputTokens");
                        e.cache_read_tokens += u64_field(u, "cacheReadInputTokens");
                        e.cache_creation_tokens += u64_field(u, "cacheCreationInputTokens");
                        e.cost_usd += u.get("costUSD").and_then(Value::as_f64).unwrap_or(0.0);
                    }
                }
            }
            continue;
        }

        // Assistant message usage — summed as the fallback aggregate.
        if ty == "assistant" {
            let Some(msg) = v.get("message") else { continue };
            let model = msg
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            if model == "<synthetic>" {
                continue;
            }
            if let Some(u) = msg.get("usage") {
                let e = summed.entry(model).or_default();
                e.input_tokens += u64_field(u, "input_tokens");
                e.output_tokens += u64_field(u, "output_tokens");
                e.cache_read_tokens += u64_field(u, "cache_read_input_tokens");
                e.cache_creation_tokens += u64_field(u, "cache_creation_input_tokens");
            }
        }
    }

    // Prefer the result-event aggregate; otherwise use the summed assistant usage.
    if !summary.from_result_event {
        summary.per_model = summed;
    }
    summary
}
