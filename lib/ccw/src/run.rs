//! The core `ccw run` orchestration.
//!
//! Ties the pieces together (INTENT #202/#207, BUILD SPEC §run):
//!
//! 1. Resolve the account (explicit / `auto` / the budget's allow-list).
//! 2. Pre-flight: zero-cost `/usage` per candidate, record observations, decide
//!    admission (session backoff + weekly token cap). Blocked → emit the
//!    machine-readable `pending` line + a human ETA line, then WAIT (poll) unless
//!    `--no-wait` (exit 75, `EX_TEMPFAIL`).
//! 3. Spawn `claude` transparently with the account's `CLAUDE_CONFIG_DIR`,
//!    stdio inherited, args verbatim.
//! 4. After exit: locate + parse the transcript (per-model usage + 429 hits),
//!    poll `/usage` again (a consumed-tokens↔pct calibration pair), record the
//!    invocation to the ledger.
//!
//! ## v0 posture: observe-don't-kill (INTENT #207)
//!
//! Mid-run overage is NOT interrupted. Everything is recorded; the overage
//! debits the budget so the NEXT run blocks. The kill-switch is a documented
//! future config seam (see [`RunOptions`] and `chassis`-level supervision), not
//! built here.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use substrate_types::{Result, SubstrateError};

use crate::accounts::{config_dir_of, Registry};
use crate::budget::{
    decide, merge_calibration, tokens_per_percent, weekly_limit_tokens, AccountUsage, BudgetRules,
    Decision, PendingLine,
};
use crate::claude;
use crate::store::{CalibrationRecord, InvocationRecord, ObservationRecord, Store};
use crate::usage::UsageSnapshot;

/// The canonical `EX_TEMPFAIL` exit code for a blocked `--no-wait` run.
pub const EX_TEMPFAIL: i32 = 75;

/// The as-reported all-models weekly pool label used for the weekly token cap
/// and its calibration key.
pub const WEEKLY_POOL: &str = "week (all models)";

/// Options for [`run`].
pub struct RunOptions {
    pub budget_id: String,
    /// `None` = use the budget's own selector; `Some("auto")` = auto; else a
    /// specific account name.
    pub account: Option<String>,
    /// Do not wait when blocked — exit `EX_TEMPFAIL` immediately.
    pub no_wait: bool,
    /// The verbatim `claude` args (everything after `--`).
    pub claude_args: Vec<String>,
    /// Poll interval while waiting on budget.
    pub poll_interval: Duration,
    /// Timeout for each zero-cost `/usage` introspection.
    pub usage_timeout: Duration,
    // FUTURE (kill-switch seam, INTENT #207): a `mid_run: MidRunPolicy` field
    // would select observe-don't-kill (v0 default) vs. a streaming kill-switch
    // that tails stream-json usage and terminates the child on threshold cross.
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            budget_id: String::new(),
            account: None,
            no_wait: false,
            claude_args: Vec::new(),
            poll_interval: Duration::from_secs(60),
            usage_timeout: Duration::from_secs(30),
        }
    }
}

/// The RFC3339 timestamp 7 days before now (the weekly window start).
fn week_start() -> String {
    (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339()
}

/// Assemble an [`AccountUsage`] for the admission decision from a fresh snapshot,
/// the ledger's weekly consumption, and the learned calibration.
fn assemble_usage(
    store: &Store,
    rules: &BudgetRules,
    budget_id: &str,
    account: &str,
    snapshot: &UsageSnapshot,
) -> Result<AccountUsage> {
    let consumed = store.weekly_consumed_tokens(budget_id, &week_start())? as f64;
    let calibration = store.get_calibration(account, WEEKLY_POOL)?;
    let tpp = calibration.map(|c| c.tokens_per_percent);
    let limit = weekly_limit_tokens(rules.weekly_pct, tpp);
    Ok(AccountUsage {
        account: account.to_string(),
        session_pct: snapshot.session_pct(),
        weekly_consumed_tokens: consumed,
        weekly_limit_tokens: limit,
        resets: snapshot.resets(),
    })
}

/// Record every pool row of a snapshot as an observation.
fn record_snapshot(store: &Store, account: &str, snapshot: &UsageSnapshot) -> Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    for p in &snapshot.pools {
        store.record_observation(&ObservationRecord {
            account: account.to_string(),
            pool: p.pool.clone(),
            pct: p.pct,
            reset_at_raw: p.reset.as_ref().map(|r| r.raw.clone()),
            observed_at: now.clone(),
        })?;
    }
    Ok(())
}

/// Resolve the candidate accounts to their `(name, config_dir)` pairs.
fn resolve_candidates(
    registry: &Registry,
    rules: &BudgetRules,
    account_arg: Option<&str>,
) -> Vec<(String, Option<PathBuf>)> {
    let names: Vec<String> = match account_arg {
        Some("auto") | None => rules.candidate_accounts(&registry.names()),
        Some(name) => vec![name.to_string()],
    };
    names
        .into_iter()
        .filter_map(|n| registry.get(&n).map(|a| (n.clone(), config_dir_of(a))))
        .collect()
}

/// One pre-flight pass: snapshot every candidate, record observations, decide.
/// Returns the decision plus the chosen candidate's snapshot (on Proceed).
fn preflight(
    store: &Store,
    candidates: &[(String, Option<PathBuf>)],
    rules: &BudgetRules,
    budget_id: &str,
    usage_timeout: Duration,
) -> Result<(Decision, Vec<(String, UsageSnapshot)>)> {
    let mut usages = Vec::new();
    let mut snapshots = Vec::new();
    for (name, config_dir) in candidates {
        // Fail-open on the introspection: a timeout/err yields an empty
        // snapshot (session unknown → not blocked on session; weekly stays
        // calibrating). We never block the run on a flaky /usage call.
        let snapshot = claude::usage_snapshot(config_dir.as_deref(), usage_timeout)
            .unwrap_or_default();
        record_snapshot(store, name, &snapshot)?;
        let usage = assemble_usage(store, rules, budget_id, name, &snapshot)?;
        usages.push(usage);
        snapshots.push((name.clone(), snapshot));
    }
    let decision = decide(rules, &usages, chrono::Local::now().naive_local());
    Ok((decision, snapshots))
}

/// Emit the blocked/pending signal: a single machine-readable JSON line to
/// stdout and a human ETA line to stderr.
fn emit_pending(reason: &str, eta: Option<&str>) {
    let line = PendingLine::new(reason, eta);
    println!("{}", line.to_json());
    match eta {
        Some(e) => eprintln!("ccw: pending — {reason}; earliest reset: {e}"),
        None => eprintln!("ccw: pending — {reason}"),
    }
}

/// Execute a wrapped `claude` run under budget governance. Returns the process
/// exit code to propagate.
pub fn run(store: &Store, registry: &Registry, opts: RunOptions) -> Result<i32> {
    let rules = store
        .get_budget(&opts.budget_id)?
        .ok_or_else(|| SubstrateError::Config(format!("no such budget: {}", opts.budget_id)))?;

    let candidates = resolve_candidates(registry, &rules, opts.account.as_deref());
    if candidates.is_empty() {
        return Err(SubstrateError::Config(format!(
            "budget {} has no registered candidate accounts",
            opts.budget_id
        )));
    }

    // ── Pre-flight, with the wait loop ──────────────────────────────────────
    let (chosen_account, chosen_config_dir, pre_snapshot) = loop {
        let (decision, snapshots) =
            preflight(store, &candidates, &rules, &opts.budget_id, opts.usage_timeout)?;
        match decision {
            Decision::Proceed { account } => {
                let config_dir = candidates
                    .iter()
                    .find(|(n, _)| *n == account)
                    .and_then(|(_, c)| c.clone());
                let snap = snapshots
                    .into_iter()
                    .find(|(n, _)| *n == account)
                    .map(|(_, s)| s)
                    .unwrap_or_default();
                break (account, config_dir, snap);
            }
            Decision::Blocked { reason, eta } => {
                emit_pending(&reason, eta.as_deref());
                if opts.no_wait {
                    return Ok(EX_TEMPFAIL);
                }
                std::thread::sleep(opts.poll_interval);
            }
        }
    };

    let cwd = std::env::current_dir()
        .map_err(|e| SubstrateError::Io(e))?;
    let pre_weekly_pct = pre_snapshot.weekly_all().map(|p| p.pct);
    let started_at = chrono::Utc::now().to_rfc3339();
    let run_start = SystemTime::now();

    // ── Spawn claude transparently ──────────────────────────────────────────
    let exit_code = claude::spawn_passthrough(chosen_config_dir.as_deref(), &opts.claude_args)?;
    let ended_at = chrono::Utc::now().to_rfc3339();

    // ── Post-run: transcript extraction ─────────────────────────────────────
    let summary = match claude::find_transcript(chosen_config_dir.as_deref(), &cwd, run_start)? {
        Some(path) => {
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            crate::transcript::parse_transcript(&body)
        }
        None => Default::default(),
    };

    // ── Post-run: second /usage poll → calibration pair ─────────────────────
    let post_snapshot = claude::usage_snapshot(chosen_config_dir.as_deref(), opts.usage_timeout)
        .unwrap_or_default();
    record_snapshot(store, &chosen_account, &post_snapshot)?;

    if let (Some(before), Some(after)) = (pre_weekly_pct, post_snapshot.weekly_all().map(|p| p.pct))
    {
        if let Some(sample) = tokens_per_percent(summary.total_tokens(), before, after) {
            let existing = store
                .get_calibration(&chosen_account, WEEKLY_POOL)?
                .map(|c| (c.tokens_per_percent, c.sample_count));
            let (mean, n) = merge_calibration(existing, sample);
            store.upsert_calibration(&CalibrationRecord {
                account: chosen_account.clone(),
                pool: WEEKLY_POOL.to_string(),
                tokens_per_percent: mean,
                updated_at: Some(chrono::Utc::now().to_rfc3339()),
                sample_count: n,
            })?;
        }
    }

    // ── Ledger ──────────────────────────────────────────────────────────────
    let usage_json = serde_json::to_string(
        &summary
            .per_model
            .iter()
            .map(|(m, u)| {
                serde_json::json!({
                    "model": m,
                    "input_tokens": u.input_tokens,
                    "output_tokens": u.output_tokens,
                    "cache_read_tokens": u.cache_read_tokens,
                    "cache_creation_tokens": u.cache_creation_tokens,
                    "cost_usd": u.cost_usd,
                })
            })
            .collect::<Vec<_>>(),
    )
    .ok();

    let reset_prose = summary
        .limit_hits
        .iter()
        .find_map(|h| h.reset.as_ref().map(|r| r.raw.clone()))
        .or_else(|| summary.limit_hits.first().map(|h| h.text.clone()));

    store.record_invocation(&InvocationRecord {
        budget_id: Some(opts.budget_id.clone()),
        account: Some(chosen_account.clone()),
        session_id: summary.session_id.clone(),
        cwd: Some(cwd.to_string_lossy().to_string()),
        started_at: Some(started_at),
        ended_at: Some(ended_at),
        usage_json,
        input_tokens: summary.total_input_tokens(),
        output_tokens: summary.total_output_tokens(),
        cache_read_tokens: summary.total_cache_read_tokens(),
        cache_creation_tokens: summary.total_cache_creation_tokens(),
        total_tokens: summary.total_tokens(),
        cost_usd: summary.cost_usd(),
        limit_hit: summary.limit_hit(),
        reset_prose,
        exit_code: Some(exit_code as i64),
    })?;

    Ok(exit_code)
}
