//! `db` — substrate's database control-plane binary (design §11 / §12).
//!
//! Thin dispatcher: parse the `clap` noun-verb tree (`cli.rs`), init tracing, load
//! `db.toml`, resolve the active env, open a [`Db`], and route each command group into
//! `substrate-db`. All real logic lives in the library; this file is one screen of glue
//! per convention (`anyhow` + `.context` at the binary layer, per recon §3).

mod cli;

use std::path::Path;

use anyhow::{Context, Result};
use clap::Parser;

use substrate_db::config::DbConfig;
use substrate_db::driver::{IntrospectQuery, LocalOp, Rows, Since};
use substrate_db::{migration, Db};

use cli::{
    Cli, Command, ConfigCmd, EdgeCmd, FakedataCmd, HandlerCmd, InspectCmd, LocalCmd, LogsCmd,
    MigrateCmd, OutboxCmd, OutputFormat, SeedCmd, TreeCmd,
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "db=info,substrate_db=info".into()),
        )
        .init();

    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        // one place to render the anyhow chain to the operator
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
    // `db init` needs no config on disk yet.
    if matches!(cli.command, Command::Init) {
        let created = substrate_db::init_project(Path::new("."))
            .context("scaffolding project")?;
        for p in &created {
            println!("created {}", p.display());
        }
        if created.is_empty() {
            println!("db.toml already present; nothing to scaffold");
        }
        return Ok(());
    }

    let cfg = DbConfig::from_file(Path::new("db.toml"))
        .context("loading db.toml (run `db init` first?)")?;
    let env_name = cfg.resolve_env(cli.env.as_deref());

    // config inspection is available without opening a driver.
    if let Command::Config(c) = &cli.command {
        return config_cmd(&cfg, &env_name, c);
    }

    // The PAT (cloud auth) comes from the keychain; v1 reads DB_PAT env as the bridge.
    let pat = std::env::var("DB_PAT").ok();
    let db = Db::open(cfg, &env_name, pat).context("opening db handle")?;

    dispatch(&db, &cli, cli.dry_run, cli.format).await
}

fn config_cmd(cfg: &DbConfig, env_name: &str, c: &ConfigCmd) -> Result<()> {
    match c {
        ConfigCmd::Show => println!("{}", cfg.to_toml().context("rendering config")?),
        ConfigCmd::Env => println!("{env_name}"),
        ConfigCmd::Check => {
            cfg.env(env_name).context("active env not defined in db.toml")?;
            println!("ok: active env `{env_name}` resolves to a defined driver");
        }
    }
    Ok(())
}

async fn dispatch(db: &Db, cli: &Cli, dry_run: bool, fmt: OutputFormat) -> Result<()> {
    let d = db.driver();
    let env = db.env_token.as_str();

    match &cli.command {
        Command::Init | Command::Config(_) => unreachable!("handled in run()"),

        // ── local stack ────────────────────────────────────────────────────
        Command::Local(l) => local_cmd(db, l).await?,
        Command::Reset => db.local(LocalOp::Reset).await.map_err(anyhow_err)?,

        // ── tree (worktree stages) ──────────────────────────────────────────
        Command::Tree(t) => tree_cmd(db, t).await?,

        // ── migrations ──────────────────────────────────────────────────────
        Command::Migrate(m) => migrate_cmd(db, m, dry_run, fmt).await?,
        Command::Up { to, one } => {
            migrate_cmd(db, &MigrateCmd::Up { to: *to, one: *one }, dry_run, fmt).await?
        }
        Command::Status => migrate_cmd(db, &MigrateCmd::Status, dry_run, fmt).await?,

        Command::Test { migration, all } => test_cmd(db, migration.as_deref(), *all).await?,

        // ── seed / fakedata ─────────────────────────────────────────────────
        Command::Seed(s) => seed_cmd(db, s).await?,
        Command::Fakedata(f) => fakedata_cmd(db, f).await?,

        // ── handlers / edge ─────────────────────────────────────────────────
        Command::Handler(h) => handler_cmd(db, h).await?,
        Command::Edge(e) => edge_cmd(db, e).await?,

        // ── query / inspect ─────────────────────────────────────────────────
        Command::Query { sql, file, write } => {
            let text = match (sql, file) {
                (Some(s), _) => s.clone(),
                (None, Some(f)) => std::fs::read_to_string(f).context("reading -f query file")?,
                (None, None) => anyhow::bail!("provide SQL or -f <file>"),
            };
            if *write {
                db.refuse_if_protected("query --write").map_err(anyhow_err)?;
            }
            let rows = substrate_db::query::run(d, env, &text, *write)
                .await
                .map_err(anyhow_err)?;
            print_rows(&rows, fmt);
        }
        Command::Inspect(i) => {
            let q = match i {
                InspectCmd::Tables => IntrospectQuery::Tables,
                InspectCmd::Describe { table } => IntrospectQuery::Describe { table: table.clone() },
                InspectCmd::Functions => IntrospectQuery::Functions,
                InspectCmd::Triggers => IntrospectQuery::Triggers,
                InspectCmd::Policies => IntrospectQuery::Policies,
                InspectCmd::Size => IntrospectQuery::Size,
                InspectCmd::Handlers => IntrospectQuery::Handlers,
            };
            let out = d.introspect(env, q).await.map_err(anyhow_err)?;
            print_rows(&out.rows, fmt);
        }

        // ── logs / outbox / audit ───────────────────────────────────────────
        Command::Logs(l) => match l {
            LogsCmd::Db => {
                let logs = d.edge_logs("", Since("1h".into())).await.map_err(anyhow_err)?;
                for line in logs.lines {
                    println!("{line}");
                }
            }
            LogsCmd::Edge { function, since } => {
                let logs = substrate_db::logs::edge(d, function, since)
                    .await
                    .map_err(anyhow_err)?;
                for line in logs.lines {
                    println!("{line}");
                }
            }
        },
        Command::Outbox(o) => outbox_cmd(db, o, fmt).await?,
        Command::Audit { handler, decision, since: _ } => {
            let rows = substrate_db::audit::list(d, env, handler.as_deref(), decision.as_deref())
                .await
                .map_err(anyhow_err)?;
            print_rows(&rows, fmt);
        }

        // ── promote / doctor ────────────────────────────────────────────────
        Command::Promote { to, confirm } => promote_cmd(db, *to, confirm.as_deref(), dry_run).await?,
        Command::Doctor { fix: _ } => {
            let report = substrate_db::promote::doctor(d, &db.config)
                .await
                .map_err(anyhow_err)?;
            for c in &report.checks {
                println!("[{}] {}: {}", if c.ok { "ok" } else { "FAIL" }, c.name, c.detail);
            }
            if !report.all_ok() {
                anyhow::bail!("doctor found failing checks");
            }
        }

        // ── snapshot (full catastrophic-recovery backup) ────────────────────
        Command::Snapshot { out, schema } => {
            let opts = substrate_db::snapshot::SnapshotOptions {
                out: out.as_ref().map(std::path::PathBuf::from),
                app_schemas: schema.clone(),
            };
            let report = substrate_db::snapshot::run(db, &opts).await.map_err(anyhow_err)?;
            let m = &report.manifest;
            println!("snapshot written to {}", report.out_dir.display());
            println!(
                "  env={} driver={} taken_at={} ({}) total_bytes={}",
                m.env, m.driver, m.taken_at, m.taken_at_source, m.total_bytes
            );
            println!("  app schemas captured: {}", m.app_schemas_captured.join(", "));
            for c in &m.components {
                let tag = match c.status {
                    substrate_db::snapshot::Status::Captured => "ok",
                    substrate_db::snapshot::Status::Skipped => "skip",
                    substrate_db::snapshot::Status::Failed => "FAIL",
                };
                println!("  [{tag}] {} ({} bytes) — {}", c.name, c.bytes, c.detail);
            }
            if !m.gaps.is_empty() {
                println!("  RECOVERY GAPS:");
                for g in &m.gaps {
                    println!("    - {g}");
                }
            }
            println!("  manifest: {}", report.manifest_md_path.display());
        }
    }
    Ok(())
}

// ── command-group handlers ──────────────────────────────────────────────────

async fn local_cmd(db: &Db, l: &LocalCmd) -> Result<()> {
    let op = match l {
        LocalCmd::Up => LocalOp::Up,
        LocalCmd::Down => LocalOp::Down,
        LocalCmd::Status => LocalOp::Status,
        LocalCmd::Restart => LocalOp::Restart,
        LocalCmd::Reset => LocalOp::Reset,
        LocalCmd::Ensure => LocalOp::Ensure,
        LocalCmd::Pull { .. } => {
            anyhow::bail!("`db local pull` is deferred in v1 (seed + fakedata cover it)")
        }
    };
    db.local(op).await.map_err(anyhow_err)
}

async fn tree_cmd(db: &Db, t: &TreeCmd) -> Result<()> {
    match t {
        TreeCmd::New { branch } => {
            let slug = substrate_db::tree::env_slug(branch);
            let token = format!("env_{slug}");
            substrate_db::tree::write_db_env(&token).map_err(anyhow_err)?;
            println!("bound worktree to env {token} (.db.env written)");
        }
        TreeCmd::List => {
            for e in substrate_db::tree::list(db.driver()).await.map_err(anyhow_err)? {
                println!("{e}");
            }
        }
        TreeCmd::Rm { branch } => {
            db.refuse_if_protected("tree rm").map_err(anyhow_err)?;
            let token = format!("env_{}", substrate_db::tree::env_slug(branch));
            substrate_db::tree::reap(db.driver(), &token).await.map_err(anyhow_err)?;
            println!("reaped {token}");
        }
        TreeCmd::Env => println!("{}", db.env_token),
    }
    Ok(())
}

async fn migrate_cmd(db: &Db, m: &MigrateCmd, dry_run: bool, fmt: OutputFormat) -> Result<()> {
    let d = db.driver();
    let env = db.env_token.as_str();
    let mig_dir = Path::new(&db.config.dirs.migrations);

    match m {
        MigrateCmd::New { name, schema, edge, no_test, no_data } => {
            let schema = schema.as_deref().unwrap_or("public");
            let dir = migration::scaffold_new(mig_dir, schema, name, *edge, *no_test, *no_data)
                .map_err(anyhow_err)?;
            println!("created {}", dir.display());
        }
        MigrateCmd::Up { to, one } => {
            let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
            let ordered = migration::topo_order(&migs).map_err(anyhow_err)?;
            let applied = migration::applied_ids(d, env).await.map_err(anyhow_err)?;
            let ctx = migration::ApplyContext {
                env,
                allow_fakedata: !db.protected,
                edge_incapable: db.driver_kind() == substrate_db::config::DriverKind::Sqlite,
            };
            let mut n = 0;
            for mm in ordered.iter().filter(|mm| !applied.contains(&mm.id)) {
                if let Some(to) = to {
                    if mm.seq > *to {
                        break;
                    }
                }
                if dry_run {
                    println!("would apply {}", mm.id);
                } else {
                    migration::apply_one(d, mm, &ctx).await.map_err(anyhow_err)?;
                    println!("applied {}", mm.id);
                }
                n += 1;
                if *one {
                    break;
                }
            }
            if n == 0 {
                println!("nothing to apply");
            }
        }
        MigrateCmd::Down { to, one } => {
            let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
            let ordered = migration::topo_order(&migs).map_err(anyhow_err)?;
            let applied = migration::applied_ids(d, env).await.map_err(anyhow_err)?;
            for mm in ordered.iter().rev().filter(|mm| applied.contains(&mm.id)) {
                if let Some(to) = to {
                    if mm.seq <= *to {
                        break;
                    }
                }
                if dry_run {
                    println!("would roll back {}", mm.id);
                } else {
                    migration::rollback_one(d, mm, env).await.map_err(anyhow_err)?;
                    println!("rolled back {}", mm.id);
                }
                if *one {
                    break;
                }
            }
        }
        MigrateCmd::Redo => {
            Box::pin(migrate_cmd(db, &MigrateCmd::Down { to: None, one: true }, dry_run, fmt)).await?;
            Box::pin(migrate_cmd(db, &MigrateCmd::Up { to: None, one: true }, dry_run, fmt)).await?;
        }
        MigrateCmd::Status => {
            let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
            let ordered = migration::topo_order(&migs).map_err(anyhow_err)?;
            let applied = migration::applied_ids(d, env).await.map_err(anyhow_err)?;
            let rows = Rows {
                columns: vec!["id".into(), "state".into()],
                rows: ordered
                    .iter()
                    .map(|mm| {
                        vec![
                            Some(mm.id.clone()),
                            Some(if applied.contains(&mm.id) { "applied" } else { "pending" }.into()),
                        ]
                    })
                    .collect(),
            };
            print_rows(&rows, fmt);
        }
        MigrateCmd::Crawl { from } => {
            db.refuse_if_protected("migrate crawl").map_err(anyhow_err)?;
            let mut migs = migration::discover(mig_dir).map_err(anyhow_err)?;
            if let Some(from) = from {
                migs.retain(|mm| mm.seq >= *from);
            }
            let atts = migration::crawl(d, &migs, env).await.map_err(anyhow_err)?;
            migration::record_attestations(d, &atts, "operator").await.map_err(anyhow_err)?;
            println!("crawl passed: {} migration(s) attested", atts.len());
        }
        MigrateCmd::Lint => {
            let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
            let report = substrate_db::lint::lint(&migs);
            for f in &report.findings {
                let tag = match f.level {
                    substrate_db::lint::Level::Error => "ERROR",
                    substrate_db::lint::Level::Warning => "warn",
                };
                println!("[{tag}] {}: {}", f.migration, f.message);
            }
            if !report.is_clean() {
                anyhow::bail!("lint failed");
            }
            println!("lint clean");
        }
        MigrateCmd::Verify => {
            let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
            migration::topo_order(&migs).map_err(anyhow_err)?;
            let report = substrate_db::lint::lint(&migs);
            if !report.is_clean() {
                anyhow::bail!("verify: lint not clean");
            }
            println!("verify ok: {} migration(s), graph acyclic, lint clean", migs.len());
        }
    }
    Ok(())
}

/// `db test [--migration <seq>] [--all]` (finding 7): run the in-DB test files
/// (`test_up.sql`) for one migration or all of them against the active env. Each file is a
/// SQL-assert harness — a failing assertion RAISEs and aborts, which surfaces as an error
/// here (nonzero exit). Sentinel-only test files count as trivially-passing.
async fn test_cmd(db: &Db, migration: Option<&str>, all: bool) -> Result<()> {
    let d = db.driver();
    let env = db.env_token.as_str();
    let mig_dir = Path::new(&db.config.dirs.migrations);
    let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
    let _ = all; // `--all` is the explicit broad form; the default (no --migration) is also all.
    let selected: Vec<_> = migs
        .iter()
        .filter(|m| match migration {
            Some(seq) => m.id.ends_with(seq) || format!("{:04}", m.seq) == seq,
            None => true,
        })
        .collect();
    if selected.is_empty() {
        anyhow::bail!("no migrations matched (dir {})", mig_dir.display());
    }
    let mut ran = 0;
    let mut failed = 0;
    for m in selected {
        if let Some(body) = m.read("test_up.sql") {
            if migration::is_sentinel_only(&body) {
                println!("ok - {} (no test)", m.id);
                continue;
            }
            match d.apply_sql(env, &body).await {
                Ok(()) => {
                    ran += 1;
                    println!("ok - {}", m.id);
                }
                Err(e) => {
                    failed += 1;
                    println!("not ok - {} : {e}", m.id);
                }
            }
        }
    }
    println!("# tests {} passed, {} failed", ran, failed);
    if failed > 0 {
        anyhow::bail!("{failed} test(s) failed");
    }
    Ok(())
}

async fn seed_cmd(db: &Db, s: &SeedCmd) -> Result<()> {
    let d = db.driver();
    let env = db.env_token.as_str();
    let seeds_dir = Path::new(&db.config.dirs.seeds);
    let (schema, down) = match s {
        SeedCmd::Up { schema } => (schema.clone(), false),
        SeedCmd::Down { schema } => (schema.clone(), true),
    };
    let schemas = match &schema {
        Some(s) => vec![s.clone()],
        None => vec!["public".into(), "core".into(), "mind".into()],
    };
    for sc in schemas {
        if down {
            // Finding 9: `seed down` runs the companion `<schema>.down.sql` (the reversal
            // of the curated dataset). If absent, say so explicitly rather than silently
            // no-op'ing — the seed has no declared reversal.
            let down_file = seeds_dir.join(format!("{sc}.down.sql"));
            match std::fs::read_to_string(&down_file) {
                Ok(sql) => {
                    d.apply_sql(env, &sql).await.map_err(anyhow_err)?;
                    println!("seed down applied for {sc}");
                }
                Err(_) => {
                    println!(
                        "seed down for {sc}: no {sc}.down.sql present (this seed declares no reversal)"
                    );
                }
            }
        } else {
            let file = seeds_dir.join(format!("{sc}.sql"));
            if let Ok(sql) = std::fs::read_to_string(&file) {
                d.apply_sql(env, &sql).await.map_err(anyhow_err)?;
                println!("seeded {sc}");
            }
        }
    }
    Ok(())
}

async fn fakedata_cmd(db: &Db, f: &FakedataCmd) -> Result<()> {
    let d = db.driver();
    let env = db.env_token.as_str();
    db.refuse_if_protected("fakedata").map_err(anyhow_err)?;
    let mig_dir = Path::new(&db.config.dirs.migrations);
    let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
    let (seq, file) = match f {
        FakedataCmd::Up { seq } => (seq, "fakedata_up.sql"),
        FakedataCmd::Down { seq } => (seq, "fakedata_down.sql"),
    };
    let m = migs
        .iter()
        .find(|m| m.id.ends_with(seq.as_str()) || format!("{:04}", m.seq) == *seq)
        .with_context(|| format!("no migration matching seq {seq}"))?;
    if let Some(sql) = m.read(file) {
        if !migration::is_sentinel_only(&sql) {
            d.apply_sql(env, &sql).await.map_err(anyhow_err)?;
        }
    }
    println!("fakedata {file} applied for {}", m.id);
    Ok(())
}

async fn handler_cmd(db: &Db, h: &HandlerCmd) -> Result<()> {
    let handlers_dir = Path::new(&db.config.dirs.handlers);
    match h {
        HandlerCmd::New { name, kind, invocation } => {
            let dir = handlers_dir.join(name);
            std::fs::create_dir_all(&dir).context("creating handler dir")?;
            let contract = format!(
                "handler: {name}\nversion: v1\nkind: {kind}\ninvocation: {invocation}\nparams: {{}}\nreturns: void\n"
            );
            std::fs::write(dir.join("v1.yaml"), contract).context("writing contract")?;
            println!("created {}/v1.yaml", dir.display());
        }
        HandlerCmd::List => {
            if let Ok(entries) = std::fs::read_dir(handlers_dir) {
                for e in entries.flatten() {
                    println!("{}", e.file_name().to_string_lossy());
                }
            }
        }
        HandlerCmd::Show { name } => {
            let c = substrate_db::handler::load_contract(handlers_dir, name, "v1")
                .map_err(anyhow_err)?;
            println!("{}", serde_json::to_string_pretty(&c.to_jsonb())?);
        }
        HandlerCmd::Codegen { name, check } => {
            let names: Vec<String> = match name {
                Some(n) => vec![n.clone()],
                None => std::fs::read_dir(handlers_dir)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect(),
            };
            for n in names {
                let c = substrate_db::handler::load_contract(handlers_dir, &n, "v1")
                    .map_err(anyhow_err)?;
                let gen = substrate_db::handler::codegen(&c);
                let sql_out = Path::new(&db.config.dirs.generated_sql).join(format!("call_{n}.sql"));
                if *check {
                    let existing = std::fs::read_to_string(&sql_out).unwrap_or_default();
                    if existing != gen.sql_wrapper {
                        anyhow::bail!("codegen stale for {n} (run without --check to regenerate)");
                    }
                } else {
                    std::fs::create_dir_all(&db.config.dirs.generated_sql).ok();
                    std::fs::write(&sql_out, &gen.sql_wrapper).context("writing generated sql")?;
                    if !gen.ts_guard.is_empty() {
                        std::fs::create_dir_all(&db.config.dirs.generated_ts).ok();
                        std::fs::write(
                            Path::new(&db.config.dirs.generated_ts).join(format!("{n}.ts")),
                            &gen.ts_guard,
                        )
                        .context("writing generated ts")?;
                    }
                    println!("codegen wrote {}", sql_out.display());
                }
            }
        }
        HandlerCmd::Activate { name, version } => {
            db.driver().activate_handler(&db.env_token, name, version).await.map_err(anyhow_err)?;
            println!("activated {name} = {version} in env {}", db.env_token);
        }
        HandlerCmd::Rollback { name } => {
            // Finding 8: a real one-row pointer-flip to the prior version (§7.3), for the
            // active env. Refused on a protected ref unless the safety gate is crossed.
            db.refuse_if_protected("handler rollback").map_err(anyhow_err)?;
            db.driver()
                .rollback_handler(&db.env_token, name)
                .await
                .map_err(anyhow_err)?;
            println!("rolled {name} back to its prior version in env {}", db.env_token);
        }
    }
    Ok(())
}

async fn edge_cmd(db: &Db, e: &EdgeCmd) -> Result<()> {
    let d = db.driver();
    let env = db.env_token.as_str();
    if !d.capabilities().edge {
        anyhow::bail!(
            "edge commands are NotImplemented on driver {} (sqlite has no edge runtime)",
            db.driver_kind()
        );
    }
    match e {
        EdgeCmd::New { name } => {
            let dir = Path::new(&db.config.dirs.edge).join(name);
            std::fs::create_dir_all(&dir).context("creating edge dir")?;
            std::fs::write(dir.join("index.ts"), "// edge function\n").context("writing index.ts")?;
            println!("created {}", dir.display());
        }
        EdgeCmd::Deploy { name } => {
            let src = Path::new(&db.config.dirs.edge).join(name);
            let bundle = substrate_db::edge::build_and_store(&src, Path::new(&db.config.dirs.bundles))
                .map_err(anyhow_err)?;
            let deploy = d.deploy_edge(env, name, "v1", &bundle).await.map_err(anyhow_err)?;
            println!("deployed {} (source_hash {})", deploy.slug, deploy.source_hash);
        }
        EdgeCmd::List { name } => {
            let sql = match name {
                Some(n) => format!(
                    "SELECT slug, handler, version, env, deployed_at FROM ops.edge_deploys WHERE handler = '{n}' ORDER BY deployed_at"
                ),
                None => "SELECT slug, handler, version, env, deployed_at FROM ops.edge_deploys ORDER BY deployed_at".into(),
            };
            let rows = d.query(env, &sql).await.map_err(anyhow_err)?;
            print_rows(&rows, OutputFormat::Table);
        }
        EdgeCmd::Activate { name, version } => {
            d.activate_handler(env, name, version).await.map_err(anyhow_err)?;
            println!("activated edge {name} = {version}");
        }
        EdgeCmd::Rollback { name } => {
            // Finding 8: real pointer-flip rollback to the prior version (§7.3) — the same
            // one-row UPDATE as `handler rollback`, on the edge-kind handler.
            db.refuse_if_protected("edge rollback").map_err(anyhow_err)?;
            d.rollback_handler(env, name).await.map_err(anyhow_err)?;
            println!("rolled edge {name} back to its prior version in env {env}");
        }
        EdgeCmd::Sync { name, version } => {
            // Finding 8: disaster recovery — re-push the STORED content-addressed bundle
            // (M3, §7.3), never a rebuild. Look up the deploy's bundle_ref, load the stored
            // bytes, and re-deploy the immutable slug.
            let slug = format!("{name}_{version}");
            let rows = d
                .query(
                    env,
                    &format!(
                        "SELECT bundle_ref FROM ops.edge_deploys WHERE slug = '{}'",
                        slug.replace('\'', "''")
                    ),
                )
                .await
                .map_err(anyhow_err)?;
            let bundle_ref = rows
                .rows
                .first()
                .and_then(|r| r.first().cloned().flatten())
                .with_context(|| format!("no stored deploy for {slug} (nothing to sync)"))?;
            let bundle = substrate_db::edge::load_stored(&bundle_ref).map_err(anyhow_err)?;
            let deploy = d.deploy_edge(env, name, version, &bundle).await.map_err(anyhow_err)?;
            println!("re-pushed stored bundle for {} (source_hash {})", deploy.slug, deploy.source_hash);
        }
        EdgeCmd::Logs { name, since, version: _, tail: _ } => {
            let logs = substrate_db::logs::edge(d, name, since).await.map_err(anyhow_err)?;
            for line in logs.lines {
                println!("{line}");
            }
        }
    }
    Ok(())
}

async fn outbox_cmd(db: &Db, o: &OutboxCmd, fmt: OutputFormat) -> Result<()> {
    let d = db.driver();
    let env = db.env_token.as_str();
    match o {
        OutboxCmd::List { status, handler } => {
            let rows = substrate_db::outbox::list(d, env, status.as_deref(), handler.as_deref())
                .await
                .map_err(anyhow_err)?;
            print_rows(&rows, fmt);
        }
        OutboxCmd::Retry { id, all_failed: _ } => {
            substrate_db::outbox::retry(d, env, *id).await.map_err(anyhow_err)?;
            println!("retry enqueued");
        }
        OutboxCmd::Drain => {
            let report = substrate_db::outbox::drain(d, env).await.map_err(anyhow_err)?;
            println!(
                "drained: fired={} failed={} skipped_quarantined={}",
                report.fired, report.failed, report.skipped_quarantined
            );
        }
    }
    Ok(())
}

async fn promote_cmd(db: &Db, to: Option<u32>, confirm: Option<&str>, dry_run: bool) -> Result<()> {
    let d = db.driver();

    // Finding 3: promote may ONLY apply against the intended cloud prod target. Bind the
    // target explicitly — typed-confirm alone is insufficient. Refuse if the resolved
    // driver cannot be promoted TO, or if the env's project_ref is not a protected/prod
    // ref (a wrong/local target must be refused before any write).
    substrate_db::promote::assert_promote_target(&db.config, &db.env_name, d)
        .map_err(anyhow_err)?;

    let mig_dir = Path::new(&db.config.dirs.migrations);
    let migs = migration::discover(mig_dir).map_err(anyhow_err)?;
    let mut pending = substrate_db::promote::pending_for_prod(d, &migs).await.map_err(anyhow_err)?;
    if let Some(to) = to {
        pending.retain(|m| m.seq <= to);
    }

    // Finding 1: run the REAL codegen `--check` staleness test (design §9.3 step 2). A
    // stale wrapper BLOCKS promote — no more hardcoded `false`.
    let codegen_stale = substrate_db::handler::codegen_is_stale(
        Path::new(&db.config.dirs.handlers),
        Path::new(&db.config.dirs.generated_sql),
    )
    .map_err(anyhow_err)?;

    let report = substrate_db::promote::gate(d, &migs, &pending, codegen_stale)
        .await
        .map_err(anyhow_err)?;
    for l in &report.lint {
        println!("lint: {l}");
    }
    for a in &report.missing_attestations {
        println!("missing attestation: {a}");
    }
    if !report.is_clear() {
        for b in &report.blockers {
            println!("BLOCKER: {b}");
        }
        anyhow::bail!("promote gate not clear");
    }
    if dry_run {
        println!("promote --dry-run: gate clear; {} migration(s) would apply", pending.len());
        return Ok(());
    }
    let typed = confirm.context("promote requires --confirm \"<phrase>\"")?;
    substrate_db::promote::check_confirm(&db.config, typed).map_err(anyhow_err)?;
    // edge_incapable derives from the resolved target's capabilities (finding 12).
    let ctx = migration::ApplyContext {
        env: "",
        allow_fakedata: false,
        edge_incapable: !d.capabilities().edge,
    };
    for m in &pending {
        migration::apply_one(d, m, &ctx).await.map_err(anyhow_err)?;
        println!("promoted {}", m.id);
    }
    Ok(())
}

// ── output rendering ────────────────────────────────────────────────────────

fn print_rows(rows: &Rows, fmt: OutputFormat) {
    match fmt {
        OutputFormat::Json => {
            let objs: Vec<serde_json::Value> = rows
                .rows
                .iter()
                .map(|r| {
                    let m: serde_json::Map<String, serde_json::Value> = rows
                        .columns
                        .iter()
                        .zip(r)
                        .map(|(c, v)| {
                            (c.clone(), v.clone().map(serde_json::Value::String).unwrap_or(serde_json::Value::Null))
                        })
                        .collect();
                    serde_json::Value::Object(m)
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&objs).unwrap_or_default());
        }
        OutputFormat::Csv => {
            println!("{}", rows.columns.join(","));
            for r in &rows.rows {
                let cells: Vec<String> =
                    r.iter().map(|c| c.clone().unwrap_or_default()).collect();
                println!("{}", cells.join(","));
            }
        }
        OutputFormat::Table => {
            println!("{}", rows.columns.join("\t"));
            for r in &rows.rows {
                let cells: Vec<String> =
                    r.iter().map(|c| c.clone().unwrap_or_else(|| "NULL".into())).collect();
                println!("{}", cells.join("\t"));
            }
        }
    }
}

/// Bridge a `SubstrateError` into the binary's `anyhow` chain (recon §3: libs return
/// `SubstrateError`, the bin wraps with `anyhow` + context).
fn anyhow_err(e: substrate_types::SubstrateError) -> anyhow::Error {
    anyhow::Error::new(e)
}
