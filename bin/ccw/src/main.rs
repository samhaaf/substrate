//! `ccw` — the Claude Code Wrapper binary (thin dispatcher).
//!
//! All logic lives in `substrate-ccw`; this file is glue: parse the `clap` tree,
//! open the [`Ccw`] context (state home + registry, first-run default), and route
//! each command. `anyhow` + rendered error chain at the binary layer.

mod cli;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;

use substrate_ccw::accounts::{config_dir_of, Account, DEFAULT_ACCOUNT};
use substrate_ccw::budget::{AccountSelector, BudgetRules};
use substrate_ccw::run::RunOptions;
use substrate_ccw::store::AccountSeen;
use substrate_ccw::{claude, home, Ccw};

use cli::{AccountCmd, BudgetCmd, Cli, Command};

const AUTH_TIMEOUT: Duration = Duration::from_secs(15);
const USAGE_TIMEOUT: Duration = Duration::from_secs(30);

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ccw=info,substrate_ccw=info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32> {
    let cli = Cli::parse();
    let mut ctx = Ccw::open().context("opening ccw state home")?;

    match cli.command {
        Command::Account(cmd) => account_cmd(&mut ctx, cmd)?,
        Command::Budget(cmd) => budget_cmd(&ctx, cmd)?,
        Command::Status { json } => status_cmd(&ctx, json)?,
        Command::Usage { budget, limit } => usage_cmd(&ctx, budget.as_deref(), limit)?,
        Command::Run {
            budget,
            account,
            no_wait,
            claude_args,
        } => {
            let opts = RunOptions {
                budget_id: budget,
                account,
                no_wait,
                claude_args,
                ..Default::default()
            };
            let code = substrate_ccw::run::run(&ctx.store, &ctx.registry, opts)
                .context("running claude under budget")?;
            return Ok(code);
        }
    }
    Ok(0)
}

fn account_cmd(ctx: &mut Ccw, cmd: AccountCmd) -> Result<()> {
    match cmd {
        AccountCmd::Add {
            name,
            config_dir,
            machine_default,
        } => {
            let config_dir = if machine_default {
                None
            } else {
                let dir = match config_dir {
                    Some(d) => PathBuf::from(d),
                    None => home::generated_config_dir(&name)?,
                };
                std::fs::create_dir_all(&dir)
                    .with_context(|| format!("creating config dir {}", dir.display()))?;
                Some(dir.to_string_lossy().to_string())
            };
            ctx.registry.add(&name, config_dir.clone());
            ctx.registry.save(&home::accounts_path()?)?;
            match &config_dir {
                Some(d) => println!("added account `{name}` (CLAUDE_CONFIG_DIR={d})"),
                None => println!("added account `{name}` (machine default; env unset)"),
            }
            if config_dir.is_some() {
                println!(
                    "next: log it in with `CLAUDE_CONFIG_DIR={} claude auth login`",
                    config_dir.unwrap()
                );
            }
        }
        AccountCmd::List => {
            for name in ctx.registry.names() {
                let acct = ctx.registry.get(&name).cloned().unwrap_or(Account {
                    config_dir: None,
                });
                let dir = config_dir_of(&acct);
                let status = claude::auth_status(dir.as_deref(), AUTH_TIMEOUT).ok();
                let (logged, email, sub) = match &status {
                    Some(s) => (
                        s.logged_in,
                        s.email.clone().unwrap_or_default(),
                        s.subscription_type.clone().unwrap_or_default(),
                    ),
                    None => (false, String::new(), String::new()),
                };
                let default_tag = if name == DEFAULT_ACCOUNT { " [default]" } else { "" };
                let dir_str = acct
                    .config_dir
                    .clone()
                    .unwrap_or_else(|| "~/.claude (env unset)".to_string());
                println!(
                    "{name}{default_tag}\n  config_dir: {dir_str}\n  logged_in: {logged}  email: {email}  plan: {sub}"
                );
                // Persist what we saw.
                ctx.store.upsert_account_seen(&AccountSeen {
                    name: name.clone(),
                    config_dir: acct.config_dir.clone(),
                    subscription_type: status.as_ref().and_then(|s| s.subscription_type.clone()),
                    email: status.as_ref().and_then(|s| s.email.clone()),
                    logged_in: logged,
                    last_checked_at: Some(chrono::Utc::now().to_rfc3339()),
                })?;
            }
        }
    }
    Ok(())
}

fn budget_cmd(ctx: &Ccw, cmd: BudgetCmd) -> Result<()> {
    match cmd {
        BudgetCmd::Add {
            id,
            weekly_pct,
            session_backoff_pct,
            account,
            model_class_pcts,
        } => {
            let accounts = if account.is_empty()
                || (account.len() == 1 && account[0].eq_ignore_ascii_case("auto"))
            {
                AccountSelector::Auto
            } else {
                AccountSelector::Named(account)
            };
            let model_class_pcts: BTreeMap<String, f64> = match model_class_pcts {
                Some(j) => serde_json::from_str(&j)
                    .context("parsing --model-class-pcts JSON")?,
                None => BTreeMap::new(),
            };
            let rules = BudgetRules {
                weekly_pct,
                session_backoff_pct,
                accounts,
                model_class_pcts,
                ..Default::default()
            };
            ctx.store.upsert_budget(&id, &rules)?;
            println!("added budget `{id}`:\n{}", rules.to_json());
        }
        BudgetCmd::List => {
            for (id, rules) in ctx.store.list_budgets()? {
                println!(
                    "{id}: weekly {}% / session-backoff {}% / accounts {:?}",
                    rules.weekly_pct, rules.session_backoff_pct, rules.accounts
                );
            }
        }
        BudgetCmd::Status { id } => {
            let rules = ctx
                .store
                .get_budget(&id)?
                .with_context(|| format!("no such budget: {id}"))?;
            let week_start = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
            let consumed = ctx.store.weekly_consumed_tokens(&id, &week_start)?;
            let names = rules.candidate_accounts(&ctx.registry.names());
            println!("budget {id}");
            println!("  weekly_pct: {}%", rules.weekly_pct);
            println!("  session_backoff_pct: {}%", rules.session_backoff_pct);
            println!("  accounts: {:?}", rules.accounts);
            println!("  weekly consumed (last 7d): {consumed} tokens");
            for acct in &names {
                match ctx.store.get_calibration(acct, substrate_ccw::run::WEEKLY_POOL)? {
                    Some(c) => {
                        let limit = rules.weekly_pct * c.tokens_per_percent;
                        println!(
                            "  [{acct}] weekly limit: {limit:.0} tokens (calibrated, {:.0} tok/%, n={})",
                            c.tokens_per_percent, c.sample_count
                        );
                    }
                    None => println!("  [{acct}] weekly enforcement: calibrating (no token/% data yet)"),
                }
            }
        }
    }
    Ok(())
}

fn status_cmd(ctx: &Ccw, json: bool) -> Result<()> {
    let mut accounts_out = Vec::new();
    for name in ctx.registry.names() {
        let acct = ctx.registry.get(&name).cloned().unwrap();
        let dir = config_dir_of(&acct);
        let snapshot = claude::usage_snapshot(dir.as_deref(), USAGE_TIMEOUT).ok();
        let pools: Vec<_> = snapshot
            .as_ref()
            .map(|s| {
                s.pools
                    .iter()
                    .map(|p| {
                        serde_json::json!({
                            "pool": p.pool,
                            "pct": p.pct,
                            "reset": p.reset.as_ref().map(|r| r.raw.clone()),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        accounts_out.push(serde_json::json!({
            "account": name,
            "config_dir": acct.config_dir,
            "pools": pools,
        }));
    }

    let mut budgets_out = Vec::new();
    let week_start = (chrono::Utc::now() - chrono::Duration::days(7)).to_rfc3339();
    for (id, rules) in ctx.store.list_budgets()? {
        let consumed = ctx.store.weekly_consumed_tokens(&id, &week_start)?;
        let names = rules.candidate_accounts(&ctx.registry.names());
        let cal: Vec<_> = names
            .iter()
            .map(|a| {
                let c = ctx.store.get_calibration(a, substrate_ccw::run::WEEKLY_POOL).ok().flatten();
                match c {
                    Some(c) => serde_json::json!({
                        "account": a,
                        "state": "calibrated",
                        "tokens_per_percent": c.tokens_per_percent,
                        "weekly_limit_tokens": rules.weekly_pct * c.tokens_per_percent,
                        "sample_count": c.sample_count,
                    }),
                    None => serde_json::json!({"account": a, "state": "calibrating"}),
                }
            })
            .collect();
        budgets_out.push(serde_json::json!({
            "id": id,
            "weekly_pct": rules.weekly_pct,
            "session_backoff_pct": rules.session_backoff_pct,
            "weekly_consumed_tokens": consumed,
            "calibration": cal,
        }));
    }

    if json {
        let out = serde_json::json!({"accounts": accounts_out, "budgets": budgets_out});
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("== accounts ==");
        for a in &accounts_out {
            println!("{}", a["account"].as_str().unwrap_or("?"));
            if let Some(pools) = a["pools"].as_array() {
                if pools.is_empty() {
                    println!("  (no /usage data — logged out or claude unavailable)");
                }
                for p in pools {
                    println!(
                        "  {}: {}% used · resets {}",
                        p["pool"].as_str().unwrap_or("?"),
                        p["pct"],
                        p["reset"].as_str().unwrap_or("?")
                    );
                }
            }
        }
        println!("== budgets ==");
        for b in &budgets_out {
            println!(
                "{}: weekly {}% / session-backoff {}% / consumed {} tok (7d)",
                b["id"].as_str().unwrap_or("?"),
                b["weekly_pct"],
                b["session_backoff_pct"],
                b["weekly_consumed_tokens"]
            );
            if let Some(cal) = b["calibration"].as_array() {
                for c in cal {
                    println!(
                        "  [{}] {}",
                        c["account"].as_str().unwrap_or("?"),
                        c["state"].as_str().unwrap_or("?")
                    );
                }
            }
        }
    }
    Ok(())
}

fn usage_cmd(ctx: &Ccw, budget: Option<&str>, limit: usize) -> Result<()> {
    let rows = ctx.store.list_invocations(budget, limit)?;
    if rows.is_empty() {
        println!("(no invocations recorded)");
        return Ok(());
    }
    for r in rows {
        println!(
            "{} | budget={} account={} tokens={} cost=${:.4} exit={} limit_hit={}{}",
            r.started_at.as_deref().unwrap_or("?"),
            r.budget_id.as_deref().unwrap_or("-"),
            r.account.as_deref().unwrap_or("-"),
            r.total_tokens,
            r.cost_usd,
            r.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "-".into()),
            r.limit_hit,
            r.reset_prose
                .as_deref()
                .map(|p| format!(" reset={p}"))
                .unwrap_or_default(),
        );
    }
    Ok(())
}
