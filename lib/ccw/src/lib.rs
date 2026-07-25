//! # substrate-ccw — the Claude Code Wrapper (CCW)
//!
//! CCW v0 is a **boring, standalone** budget-governed wrapper around the external
//! `claude` CLI (INTENT #202/#205/#207: "everything is a daemon; this one wraps
//! an external service"). It is a drop-in prefix — `ccw run --budget x -- claude
//! …` behaves like `claude …` — that regulates spend across multiple Max
//! accounts using ONLY observed usage data (no estimates, ever).
//!
//! ## What v0 does
//!
//! - **Accounts** ([`accounts`]) — `accounts.toml` registry (name →
//!   `CLAUDE_CONFIG_DIR`); the machine default (`~/.claude`, env unset) is
//!   auto-registered as `default` on first run.
//! - **Budgets** ([`budget`]) — schema-versioned rules JSON (v1 = 20% weekly of
//!   one account + 50% session backoff); room for Cursor/other SDKs later.
//! - **Run** ([`run`]) — resolve account → zero-cost `/usage` pre-flight →
//!   block/wait or spawn `claude` transparently → post-run transcript + `/usage`
//!   extraction → ledger.
//! - **Usage introspection** ([`usage`]) — parses `claude -p "/usage"
//!   --output-format json` (zero cost, `cc-recon.md` §5).
//! - **Transcript extraction** ([`transcript`]) — per-model usage + 429
//!   rate-limit detection from `<configdir>/projects/<escaped-cwd>/*.jsonl`.
//! - **Store** ([`store`]) — SQLite (WAL) coordinating concurrent wrappers.
//!
//! ## No estimates (INTENT #202)
//!
//! Weekly enforcement is expressed in tokens and only bites once a
//! tokens-per-percent calibration exists for the `(account, pool)`, learned from
//! observed `/usage` percentage deltas paired with observed consumption. Until
//! then, weekly enforcement reports `calibrating` and only the session-backoff
//! rule applies. See [`budget`].
//!
//! ## Future seams (documented, NOT built in v0)
//!
//! - **Mid-run kill-switch** — v0 is observe-don't-kill (overage debits the
//!   budget so the NEXT run blocks). A streaming kill-switch that tails
//!   stream-json usage and terminates the child on threshold cross is a future
//!   [`run::RunOptions`] policy field + `chassis`-level supervision seam.
//! - **Other SDKs** — [`budget::BudgetRules::schema_version`] gates future v2
//!   shapes (Cursor: third-party API limits, in-app model limits, usage
//!   credits) without touching v1 readers.

pub mod accounts;
pub mod budget;
pub mod claude;
pub mod home;
pub mod reset;
pub mod run;
pub mod store;
pub mod transcript;
pub mod usage;

use substrate_types::Result;

pub use accounts::Registry;
pub use budget::BudgetRules;
pub use store::Store;

/// A ready-to-use CCW context: the opened store + loaded account registry,
/// with the state home and machine-default account ensured (first-run).
pub struct Ccw {
    pub store: Store,
    pub registry: Registry,
}

impl Ccw {
    /// Open (or create) the CCW context at the resolved state home
    /// (`~/.leverage/ccw`, honoring `CCW_HOME`). Auto-registers the machine
    /// default account on first run (INTENT #207-(3)).
    pub fn open() -> Result<Self> {
        home::ensure_home()?;
        let store = Store::open(home::db_path()?)?;
        let mut registry = Registry::load(&home::accounts_path()?)?;
        if registry.ensure_default() {
            registry.save(&home::accounts_path()?)?;
        }
        Ok(Self { store, registry })
    }
}
