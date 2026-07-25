//! Budget rules, budget math, and admission decisions.
//!
//! ## Rules schema (v1)
//!
//! A budget is a schema-versioned JSON document (stored in `budgets.rules_json`).
//! v1 encodes the standard template from INTENT #202/#207: **20% weekly of ONE
//! account + 50% session backoff**. The `schema_version` field leaves room for
//! Cursor / other SDK budget shapes later (third-party API limits, in-app model
//! limits, usage credits) WITHOUT implementing them now — a future v2 adds
//! fields; readers switch on `schema_version`.
//!
//! ## No estimates, ever (INTENT #202)
//!
//! Weekly enforcement is expressed in TOKENS and only bites once a
//! tokens-per-percent calibration exists for the `(account, pool)`. The
//! calibration is learned from observed `/usage` percentage deltas paired with
//! observed token consumption — never guessed. Until calibrated, weekly
//! enforcement reports `calibrating` and only the session-backoff rule applies.

use serde::{Deserialize, Serialize};

use crate::reset::{earliest_reset, ResetTime};

/// Default weekly budget percentage (of one account's weekly pool).
pub const DEFAULT_WEEKLY_PCT: f64 = 20.0;
/// Default session-backoff percentage — back off once the 5h session pool hits this.
pub const DEFAULT_SESSION_BACKOFF_PCT: f64 = 50.0;
/// Current budget-rules schema version.
pub const RULES_SCHEMA_VERSION: u32 = 1;

/// The account selector for a budget.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AccountSelector {
    /// Choose among `accounts` at run time (headroom-aware).
    Auto,
    /// A fixed allow-list of account names.
    Named(Vec<String>),
}

impl Default for AccountSelector {
    fn default() -> Self {
        AccountSelector::Auto
    }
}

/// A budget's rules (schema v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetRules {
    /// Schema discriminator — always present, enables future SDK shapes.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// The SDK this budget governs. v0 only implements `claude-code`; the field
    /// exists so `cursor` (and others) can be added under new schema versions.
    #[serde(default = "default_sdk")]
    pub sdk: String,
    /// Weekly percentage cap (of one account's weekly pool).
    #[serde(default = "default_weekly_pct")]
    pub weekly_pct: f64,
    /// Session-backoff percentage (of the 5h session pool).
    #[serde(default = "default_session_backoff_pct")]
    pub session_backoff_pct: f64,
    /// Which accounts this budget may run on.
    #[serde(default)]
    pub accounts: AccountSelector,
    /// Optional per-model-class weekly caps (e.g. `{"Fable": 10.0}`), keyed by
    /// the model-scoped weekly pool label. Empty = no per-class override.
    #[serde(default)]
    pub model_class_pcts: std::collections::BTreeMap<String, f64>,
}

fn default_schema_version() -> u32 {
    RULES_SCHEMA_VERSION
}
fn default_sdk() -> String {
    "claude-code".to_string()
}
fn default_weekly_pct() -> f64 {
    DEFAULT_WEEKLY_PCT
}
fn default_session_backoff_pct() -> f64 {
    DEFAULT_SESSION_BACKOFF_PCT
}

impl Default for BudgetRules {
    fn default() -> Self {
        Self {
            schema_version: RULES_SCHEMA_VERSION,
            sdk: default_sdk(),
            weekly_pct: DEFAULT_WEEKLY_PCT,
            session_backoff_pct: DEFAULT_SESSION_BACKOFF_PCT,
            accounts: AccountSelector::Auto,
            model_class_pcts: Default::default(),
        }
    }
}

impl BudgetRules {
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("budget rules serialize")
    }
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("parsing budget rules: {e}"))
    }
    /// The candidate account names this budget may use, given a full account list
    /// (used when the selector is `Auto`).
    pub fn candidate_accounts<'a>(&'a self, all: &'a [String]) -> Vec<String> {
        match &self.accounts {
            AccountSelector::Auto => all.to_vec(),
            AccountSelector::Named(names) => names.clone(),
        }
    }
}

// ── Pure budget math ───────────────────────────────────────────────────────

/// Session-backoff decision: `true` = BLOCKED (session pool at/above backoff).
pub fn session_blocked(session_pct: f64, backoff_pct: f64) -> bool {
    session_pct >= backoff_pct
}

/// A single tokens-per-percent calibration sample from an observation pair.
/// Returns `None` when the percentage did not increase (no signal).
pub fn tokens_per_percent(tokens_delta: u64, pct_before: f64, pct_after: f64) -> Option<f64> {
    let dp = pct_after - pct_before;
    if dp <= 0.0 || tokens_delta == 0 {
        return None;
    }
    Some(tokens_delta as f64 / dp)
}

/// Merge a new tokens-per-percent sample into an existing running mean.
/// Returns `(new_mean, new_sample_count)`.
pub fn merge_calibration(
    existing: Option<(f64, u64)>,
    new_sample: f64,
) -> (f64, u64) {
    match existing {
        Some((mean, n)) if n > 0 => {
            let nf = n as f64;
            ((mean * nf + new_sample) / (nf + 1.0), n + 1)
        }
        _ => (new_sample, 1),
    }
}

/// The weekly token limit for a given percentage cap and calibration.
/// `None` when uncalibrated (weekly enforcement is `calibrating`).
pub fn weekly_limit_tokens(weekly_pct: f64, tokens_per_percent: Option<f64>) -> Option<f64> {
    tokens_per_percent.map(|tpp| weekly_pct * tpp)
}

/// Weekly consumption decision: `true` = BLOCKED (consumed at/above limit).
/// Uncalibrated (`limit_tokens = None`) never blocks on the weekly axis.
pub fn weekly_blocked(consumed_tokens: f64, limit_tokens: Option<f64>) -> bool {
    match limit_tokens {
        Some(limit) => consumed_tokens >= limit,
        None => false,
    }
}

// ── Admission decision ─────────────────────────────────────────────────────

/// A per-account usage snapshot assembled for the admission decision.
#[derive(Debug, Clone)]
pub struct AccountUsage {
    pub account: String,
    /// Session pool percent, if observed.
    pub session_pct: Option<f64>,
    /// Weekly tokens consumed by this budget on this account (from the ledger).
    pub weekly_consumed_tokens: f64,
    /// Weekly token limit, or `None` if calibrating.
    pub weekly_limit_tokens: Option<f64>,
    /// Reset descriptors observed for this account (for ETA selection).
    pub resets: Vec<ResetTime>,
}

impl AccountUsage {
    /// Is this account admissible under the budget's session + weekly rules?
    pub fn admissible(&self, rules: &BudgetRules) -> bool {
        let session_ok = match self.session_pct {
            Some(p) => !session_blocked(p, rules.session_backoff_pct),
            None => true, // no observation yet → do not block on session
        };
        let weekly_ok = !weekly_blocked(self.weekly_consumed_tokens, self.weekly_limit_tokens);
        session_ok && weekly_ok
    }

    /// Is weekly enforcement still calibrating (no token limit yet)?
    pub fn weekly_calibrating(&self) -> bool {
        self.weekly_limit_tokens.is_none()
    }
}

/// The outcome of a pre-flight admission decision.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Proceed on this account.
    Proceed { account: String },
    /// All candidate accounts are blocked; wait until `eta` (or forever).
    Blocked { reason: String, eta: Option<String> },
}

/// Decide which candidate account (if any) may run.
///
/// Prefers an admissible account with the LOWEST session percentage (most
/// session headroom). If none are admissible, returns `Blocked` with the
/// earliest reset across all candidates as the ETA.
pub fn decide(
    rules: &BudgetRules,
    candidates: &[AccountUsage],
    now: chrono::NaiveDateTime,
) -> Decision {
    if candidates.is_empty() {
        return Decision::Blocked {
            reason: "no candidate accounts available for this budget".to_string(),
            eta: None,
        };
    }

    let mut admissible: Vec<&AccountUsage> =
        candidates.iter().filter(|c| c.admissible(rules)).collect();
    if !admissible.is_empty() {
        // Prefer most session headroom (lowest session pct; unknown treated as 0).
        admissible.sort_by(|a, b| {
            let ap = a.session_pct.unwrap_or(0.0);
            let bp = b.session_pct.unwrap_or(0.0);
            ap.partial_cmp(&bp).unwrap_or(std::cmp::Ordering::Equal)
        });
        return Decision::Proceed {
            account: admissible[0].account.clone(),
        };
    }

    // All blocked — describe why and pick the earliest unblocking reset.
    let all_resets: Vec<ResetTime> = candidates
        .iter()
        .flat_map(|c| c.resets.clone())
        .collect();
    let eta = earliest_reset(&all_resets, now);

    let reason = if candidates.len() == 1 {
        let c = &candidates[0];
        blocked_reason(rules, c)
    } else {
        format!(
            "all {} candidate accounts are over budget (session backoff {}% / weekly {}%)",
            candidates.len(),
            rules.session_backoff_pct,
            rules.weekly_pct
        )
    };

    Decision::Blocked { reason, eta }
}

fn blocked_reason(rules: &BudgetRules, c: &AccountUsage) -> String {
    let mut parts = Vec::new();
    if let Some(p) = c.session_pct {
        if session_blocked(p, rules.session_backoff_pct) {
            parts.push(format!(
                "session {p:.0}% ≥ backoff {}%",
                rules.session_backoff_pct
            ));
        }
    }
    if weekly_blocked(c.weekly_consumed_tokens, c.weekly_limit_tokens) {
        if let Some(limit) = c.weekly_limit_tokens {
            parts.push(format!(
                "weekly {:.0} ≥ {:.0} tokens ({}% cap)",
                c.weekly_consumed_tokens, limit, rules.weekly_pct
            ));
        }
    }
    if parts.is_empty() {
        format!("account {} not admissible", c.account)
    } else {
        format!("account {}: {}", c.account, parts.join(", "))
    }
}

/// The machine-readable pending line emitted to stdout when blocked.
#[derive(Debug, Serialize)]
pub struct PendingLine<'a> {
    pub ccw: &'static str,
    pub reason: &'a str,
    pub eta: Option<&'a str>,
}

impl<'a> PendingLine<'a> {
    pub fn new(reason: &'a str, eta: Option<&'a str>) -> Self {
        Self {
            ccw: "pending",
            reason,
            eta,
        }
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("pending line serialize")
    }
}
