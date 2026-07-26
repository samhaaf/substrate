//! Retry classification (`reports/ccw-v1-recon.md` Deliverable 5).
//!
//! A failure is retry-worthy when it is TRANSIENT (rate/capacity/network), not
//! when it is deterministic (bad flags, a configured ceiling, expired resume).
//! aui does not classify in-stream 429/5xx nor back off; CCW must. This module
//! is the pure decision — the session runtime acts on it (wait-for-reset,
//! switch-account, bounded backoff) and emits the `retry_started` meta-event.

use serde::{Deserialize, Serialize};

/// What kind of transient failure was seen (drives the wait strategy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransientKind {
    /// 429 rate/session/weekly limit — back off to reset time OR switch account.
    RateLimit,
    /// 5xx / overload (`API Error: 529 Overloaded`) — backoff / fallback-model.
    Overload,
    /// Connection drop (`ECONNRESET` and friends) — backoff.
    ConnectionDrop,
    /// `result` subtype `error_during_execution` — retry once.
    ErrorDuringExecution,
}

/// The retry decision for an observed failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryDecision {
    /// Transient — retry (subject to the runtime's attempt ceiling).
    Retry { kind: TransientKind, reason: String },
    /// The resume id expired — restart fresh (NOT a same-resume retry).
    RestartFresh { reason: String },
    /// Deterministic — surface the error, do not retry.
    Fatal { reason: String },
}

/// Classify a `result` event's subtype (`success` never reaches here).
pub fn classify_result(subtype: &str) -> RetryDecision {
    match subtype {
        "error_during_execution" => RetryDecision::Retry {
            kind: TransientKind::ErrorDuringExecution,
            reason: "result error_during_execution".into(),
        },
        "error_max_turns" | "error_max_budget_usd" | "error_max_structured_output_retries" => {
            RetryDecision::Fatal {
                reason: format!("hit configured ceiling: {subtype}"),
            }
        }
        other => RetryDecision::Fatal {
            reason: format!("result subtype {other}"),
        },
    }
}

/// Classify a free-text error message (in-stream synthetic text or stderr).
pub fn classify_message(text: &str) -> RetryDecision {
    let t = text.to_ascii_lowercase();
    if t.contains("no conversation found") {
        return RetryDecision::RestartFresh {
            reason: "expired resume session".into(),
        };
    }
    if t.contains("rate_limit")
        || t.contains("rate limit")
        || t.contains("hit your")
        || t.contains("429")
        || t.contains("resets")
    {
        return RetryDecision::Retry {
            kind: TransientKind::RateLimit,
            reason: "rate limit (429)".into(),
        };
    }
    if t.contains("overloaded") || t.contains("529") || t.contains("503") || t.contains("500") {
        return RetryDecision::Retry {
            kind: TransientKind::Overload,
            reason: "server overload (5xx)".into(),
        };
    }
    if t.contains("econnreset") || t.contains("unable to connect") || t.contains("connection") {
        return RetryDecision::Retry {
            kind: TransientKind::ConnectionDrop,
            reason: "connection drop".into(),
        };
    }
    RetryDecision::Fatal {
        reason: format!("unclassified error: {text}"),
    }
}

/// Bounded exponential backoff (ms) for attempt `n` (1-based), capped.
pub fn backoff_ms(attempt: u32) -> u64 {
    let base = 500u64;
    let cap = 30_000u64;
    base.saturating_mul(1u64 << attempt.min(6)).min(cap)
}
