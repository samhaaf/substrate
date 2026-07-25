//! The `clap` command tree for `ccw` (the Claude Code Wrapper).

use clap::{Parser, Subcommand};

/// `ccw` — the Claude Code Wrapper: budget-governed, multi-account `claude`.
#[derive(Debug, Parser)]
#[command(name = "ccw", version, about = "Claude Code Wrapper — accounts, budgets, usage ledger")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Account registry (name -> CLAUDE_CONFIG_DIR).
    #[command(subcommand)]
    Account(AccountCmd),

    /// Budget definitions.
    #[command(subcommand)]
    Budget(BudgetCmd),

    /// Run `claude` under a budget. Everything after `--` is passed verbatim to
    /// `claude` (do NOT repeat the word `claude`):
    /// `ccw run --budget b -- -p "hi"` runs `claude -p "hi"`.
    Run {
        /// Budget id to govern this run.
        #[arg(long)]
        budget: String,
        /// Account to use: a name, or `auto` (default: the budget's selector).
        #[arg(long)]
        account: Option<String>,
        /// Do not wait when blocked; exit 75 (EX_TEMPFAIL) immediately.
        #[arg(long)]
        no_wait: bool,
        /// The verbatim `claude` args (after `--`).
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        claude_args: Vec<String>,
    },

    /// Accounts (live pool %) + budgets (consumption / limits / calibration).
    Status {
        #[arg(long)]
        json: bool,
    },

    /// Recent invocations from the ledger.
    Usage {
        #[arg(long)]
        budget: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Debug, Subcommand)]
pub enum AccountCmd {
    /// Register an account. Without --config-dir, a config dir is generated at
    /// ~/.leverage/ccw/accounts/<name>.
    Add {
        name: String,
        #[arg(long)]
        config_dir: Option<String>,
        /// Register as the machine default (no CLAUDE_CONFIG_DIR).
        #[arg(long)]
        machine_default: bool,
    },
    /// List accounts with a live `claude auth status` check per account.
    List,
}

#[derive(Debug, Subcommand)]
pub enum BudgetCmd {
    /// Create (or replace) a budget.
    Add {
        id: String,
        #[arg(long, default_value_t = substrate_ccw::budget::DEFAULT_WEEKLY_PCT)]
        weekly_pct: f64,
        #[arg(long, default_value_t = substrate_ccw::budget::DEFAULT_SESSION_BACKOFF_PCT)]
        session_backoff_pct: f64,
        /// Allowed account: a name (repeatable) or `auto`. Default: auto.
        #[arg(long)]
        account: Vec<String>,
        /// Per-model-class weekly caps as JSON, e.g. '{"week (Fable)": 10}'.
        #[arg(long)]
        model_class_pcts: Option<String>,
    },
    /// List budgets.
    List,
    /// Show one budget's consumption / limits / calibration state.
    Status { id: String },
}
