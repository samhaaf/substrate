//! The `clap` noun-verb command tree (design §11). `bin/db` is the only place `clap`
//! enters substrate (net-new, DECISION #5).

use clap::{Parser, Subcommand};

/// `db` — substrate's single operational point of contact for the database.
#[derive(Debug, Parser)]
#[command(name = "db", version, about = "substrate database control plane")]
pub struct Cli {
    /// Target env (overrides DB_ENV / .db.env / db.toml default_env).
    #[arg(long, global = true)]
    pub env: Option<String>,

    /// Print the plan without mutating (mutating commands).
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Output format.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Scaffold db.toml + db/{migrations,seeds,handlers}/.
    Init,

    /// Config inspection.
    #[command(subcommand)]
    Config(ConfigCmd),

    /// Local Supabase stack lifecycle.
    #[command(subcommand)]
    Local(LocalCmd),

    /// Worktree stage envs.
    #[command(subcommand)]
    Tree(TreeCmd),

    /// Migration authoring / apply / rollback / status / crawl / lint.
    #[command(subcommand)]
    Migrate(MigrateCmd),

    /// Run migration tests.
    Test {
        #[arg(long)]
        migration: Option<String>,
        #[arg(long)]
        all: bool,
    },

    /// Apply / revert curated seed data.
    #[command(subcommand)]
    Seed(SeedCmd),

    /// Per-migration NON-PROD fake data (never run by promote).
    #[command(subcommand)]
    Fakedata(FakedataCmd),

    /// Handler registry + codegen.
    #[command(subcommand)]
    Handler(HandlerCmd),

    /// Edge function deploy / activate / rollback.
    #[command(subcommand)]
    Edge(EdgeCmd),

    /// Ad-hoc query (read-only default; --write crosses the safety gate).
    Query {
        /// The SQL to run (or use -f).
        sql: Option<String>,
        #[arg(short = 'f', long)]
        file: Option<String>,
        #[arg(long)]
        write: bool,
    },

    /// Schema introspection.
    #[command(subcommand)]
    Inspect(InspectCmd),

    /// Logs.
    #[command(subcommand)]
    Logs(LogsCmd),

    /// Async outbox.
    #[command(subcommand)]
    Outbox(OutboxCmd),

    /// Validator audit.
    Audit {
        #[arg(long)]
        handler: Option<String>,
        #[arg(long)]
        decision: Option<String>,
        #[arg(long)]
        since: Option<String>,
    },

    /// Forward-only promotion to prod (invariant gate + typed confirm).
    Promote {
        #[arg(long)]
        to: Option<u32>,
        /// The typed-confirmation phrase.
        #[arg(long)]
        confirm: Option<String>,
    },

    /// Preflight / health checks.
    Doctor {
        #[arg(long)]
        fix: bool,
    },

    // ── flat aliases on the hot path (design §11) ──────────────────────────
    /// Alias for `migrate up`.
    Up {
        #[arg(long)]
        to: Option<u32>,
        #[arg(long)]
        one: bool,
    },
    /// Alias for `migrate status`.
    Status,
    /// Alias for `local reset`.
    Reset,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    Show,
    Env,
    Check,
}

#[derive(Debug, Subcommand)]
pub enum LocalCmd {
    Up,
    Down,
    Status,
    Restart,
    Reset,
    Ensure,
    Pull {
        #[arg(long)]
        with_prod_data: bool,
        #[arg(long)]
        no_blob: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum TreeCmd {
    New { branch: String },
    List,
    Rm { branch: String },
    Env,
}

#[derive(Debug, Subcommand)]
pub enum MigrateCmd {
    New {
        name: String,
        #[arg(long)]
        schema: Option<String>,
        #[arg(long)]
        edge: bool,
        #[arg(long)]
        no_test: bool,
        #[arg(long)]
        no_data: bool,
    },
    Up {
        #[arg(long)]
        to: Option<u32>,
        #[arg(long)]
        one: bool,
    },
    Down {
        #[arg(long)]
        to: Option<u32>,
        #[arg(long)]
        one: bool,
    },
    Redo,
    Status,
    Crawl {
        #[arg(long)]
        from: Option<u32>,
    },
    Lint,
    Verify,
}

#[derive(Debug, Subcommand)]
pub enum SeedCmd {
    Up {
        #[arg(long)]
        schema: Option<String>,
    },
    Down {
        #[arg(long)]
        schema: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum FakedataCmd {
    Up { seq: String },
    Down { seq: String },
}

#[derive(Debug, Subcommand)]
pub enum HandlerCmd {
    New {
        name: String,
        #[arg(long, default_value = "sql")]
        kind: String,
        #[arg(long, default_value = "effect-async")]
        invocation: String,
    },
    List,
    Show { name: String },
    Codegen {
        name: Option<String>,
        #[arg(long)]
        check: bool,
    },
    Activate { name: String, version: String },
    Rollback { name: String },
}

#[derive(Debug, Subcommand)]
pub enum EdgeCmd {
    New { name: String },
    Deploy { name: String },
    List { name: Option<String> },
    Activate { name: String, version: String },
    Rollback { name: String },
    Sync { name: String, version: String },
    Logs {
        name: String,
        #[arg(long, default_value = "1h")]
        since: String,
        #[arg(long)]
        version: Option<String>,
        #[arg(long)]
        tail: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum InspectCmd {
    Tables,
    Describe { table: String },
    Functions,
    Triggers,
    Policies,
    Size,
    Handlers,
}

#[derive(Debug, Subcommand)]
pub enum LogsCmd {
    /// db logs.
    Db,
    /// edge function logs.
    Edge {
        function: String,
        #[arg(long, default_value = "1h")]
        since: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum OutboxCmd {
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        handler: Option<String>,
    },
    Retry {
        id: Option<i64>,
        #[arg(long)]
        all_failed: bool,
    },
    Drain,
}
