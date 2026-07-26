//! # substrate-ccw — the Claude Code Wrapper (CCW)
//!
//! **CCW v1 is a pure WebSocket daemon** (INTENT #208). The v0 CLI-wrapper shape
//! was rejected — "we don't need a command line tool, we need a service... one
//! service running on a port... always watching and handling retries. Just kill
//! the CLI. Assume I'm never going to directly call Cloud Code again." CCW OWNS
//! claude invocation; every other Leverage service reaches Claude Code through
//! CCW's WS contract ([`protocol`], `scaffold/contracts/ccw-api.md`).
//!
//! ## The daemon
//!
//! - **[`server`]** — the axum WS API on the configured port (default
//!   [`config::DEFAULT_PORT`]): sessions (start/send/resume/list/history/cancel),
//!   a live per-session/firehose event stream, budgets/accounts/ledger surfaces.
//! - **[`session`]** — the runtime that spawns `claude` with the canonical
//!   streaming recipe, parses the live stream into [`event::CcwEvent`]s
//!   (persisted to the event log AND pushed to subscribers), tracks **subagent
//!   liveness** (busy while any async subagent runs), and handles retries.
//! - **[`spawn`]** — the canonical flag recipe + tool clean-slate construction.
//! - **[`stream`]** — the stream-json → CcwEvent taxonomy mapping (nothing
//!   dropped; recon Deliverable 1).
//! - **[`retry`]** — transient-vs-deterministic classification (recon
//!   Deliverable 5).
//!
//! ## Surviving v0 internals (kept as libraries, CLI surface removed)
//!
//! - **[`accounts`]** — `accounts.toml` registry (name → `CLAUDE_CONFIG_DIR`).
//! - **[`budget`]** — schema-versioned rules + budget math + admission.
//! - **[`usage`]** / **[`reset`]** — zero-cost `/usage` introspection + reset prose.
//! - **[`transcript`]** — per-model usage + 429 extraction (calibration source).
//! - **[`store`]** — SQLite (WAL) — extended with the `sessions` + `events`
//!   tables that back history/resume.
//!
//! ## No estimates (INTENT #202)
//!
//! Weekly enforcement is expressed in tokens and only bites once a
//! tokens-per-percent calibration exists for the `(account, pool)`. Until then,
//! weekly enforcement reports `calibrating` and only session-backoff applies.
//!
//! ## Stubbed seam (INTENT #208)
//!
//! - **rollup** — a session-start `rollup` ref names a future prompt/plugin
//!   assembly; v1 REJECTS it with not-implemented. See [`session::SessionSpec`].
//! - **Mid-run kill-switch** — observe-don't-kill stays the default (overage
//!   debits the budget so the NEXT turn's admission blocks).

pub mod accounts;
pub mod budget;
pub mod claude;
pub mod config;
pub mod event;
pub mod home;
pub mod protocol;
pub mod reset;
pub mod retry;
pub mod run;
pub mod server;
pub mod session;
pub mod spawn;
pub mod store;
pub mod stream;
pub mod transcript;
pub mod usage;

use std::net::SocketAddr;

use substrate_types::{Result, SubstrateError};

pub use accounts::Registry;
pub use budget::BudgetRules;
pub use config::Config;
pub use event::CcwEvent;
pub use session::SessionManager;
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

    /// Open the context at a specific data dir (config override / tests).
    pub fn open_at(data_dir: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| SubstrateError::Config(format!("creating {}: {e}", data_dir.display())))?;
        let store = Store::open(data_dir.join("ccw.db"))?;
        let mut registry = Registry::load(&data_dir.join("accounts.toml"))?;
        if registry.ensure_default() {
            registry.save(&data_dir.join("accounts.toml"))?;
        }
        Ok(Self { store, registry })
    }
}

/// Build a [`SessionManager`] from a config: open the context at the config's
/// data dir, merge inline account seeds into the registry, apply `claude_bin`.
pub fn build_manager(config: Config) -> Result<SessionManager> {
    config.apply_claude_bin();
    let data_dir = config.resolved_data_dir()?;
    let ctx = Ccw::open_at(&data_dir)?;
    let mut registry = ctx.registry;
    // Merge inline account seeds from config.toml (empty string = machine default).
    let mut changed = false;
    for (name, dir) in &config.accounts {
        let cd = if dir.is_empty() { None } else { Some(dir.clone()) };
        if registry.get(name).map(|a| a.config_dir.clone()) != Some(cd.clone()) {
            registry.add(name, cd);
            changed = true;
        }
    }
    if changed {
        let _ = registry.save(&data_dir.join("accounts.toml"));
    }
    Ok(SessionManager::new(ctx.store, registry, config))
}

/// Run the daemon to completion (until Ctrl-C). The binary's whole job.
pub async fn run_daemon(config: Config) -> Result<()> {
    let host = config.host.clone();
    let port = config.port;
    let manager = build_manager(config)?;
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| SubstrateError::Config(format!("invalid bind addr {host}:{port}: {e}")))?;
    let (bound, handle) = server::serve(manager, addr).await?;
    tracing::info!("ccw daemon listening on ws://{bound}/ws");
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("ccw daemon shutting down");
    handle.abort();
    Ok(())
}
