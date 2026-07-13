//! `db snapshot` — a COMPLETE local backup of a target database (a catastrophic-recovery
//! net), NOT a restore tool. It writes, into one timestamped directory:
//!
//!   - `schema.sql`            — full DDL for the app schemas (`--schema-only`)
//!   - `data.sql`             — all rows for the app schemas (`--data-only --use-copy`)
//!   - `cron.sql`             — the `cron.job` schedule (pg_cron), so it is recoverable
//!   - `roles.sql`            — cluster roles / grants DDL (`--role-only`)
//!   - `extensions.txt`       — installed extensions + versions
//!   - `schemas.txt`          — EVERY schema present, app-vs-managed classified
//!   - `grants.txt`           — table grants relevant to the app schemas
//!   - `edge_deploys.json`    — the `ops.edge_deploys` registry (names + versions)
//!   - `edge_functions/`      — a copy of the on-disk edge source tree
//!   - `manifest.json` / `MANIFEST.md` — what WAS and was NOT captured, with sizes
//!
//! It is READ-ONLY against the source: every capture is a dump/SELECT/file-copy; nothing
//! is ever written back to the database. Schema+data+roles go through `supabase db dump`
//! (a version-matched, dockerised `pg_dump`), so a v17 server is dumped correctly even
//! when the host `pg_dump` is older. If a component cannot be captured, the snapshot does
//! not abort — the manifest records the failure and the rest still runs.
//!
//! Timestamps come from the DB clock (`now()`) — the crate forbids `Date::now()` — with a
//! `date -u` shell fallback.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use substrate_types::{Result, SubstrateError};
use tokio::process::Command;

use crate::config::DriverKind;
use crate::Db;

/// App schemas captured IN FULL (DDL + data). Absent ones are silently skipped after the
/// present-schema intersection. Broomstick's live app schemas plus the control plane.
pub const DEFAULT_APP_SCHEMAS: &[&str] =
    &["public", "core", "mind", "workshop", "ops", "actions"];

/// Supabase-managed schemas whose INTERNALS we never dump (design requirement) — their
/// presence is still recorded so a human knows they existed. `cron` is here for the
/// classifier but is captured specially (its `cron.job` schedule is recoverable).
pub const MANAGED_SCHEMAS: &[&str] = &[
    "auth",
    "storage",
    "realtime",
    "_realtime",
    "vault",
    "supabase_functions",
    "graphql",
    "graphql_public",
    "net",
    "extensions",
    "supabase_migrations",
    "pgbouncer",
    "cron",
];

/// Options for a snapshot run.
#[derive(Debug, Clone, Default)]
pub struct SnapshotOptions {
    /// Explicit output dir (`--out`). When `None`, defaults to
    /// `./.db-snapshots/<env>-<UTCstamp>/`.
    pub out: Option<PathBuf>,
    /// App schemas to capture in full. When empty, [`DEFAULT_APP_SCHEMAS`] is used.
    pub app_schemas: Vec<String>,
}

/// Per-component capture outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Captured successfully.
    Captured,
    /// Deliberately skipped (e.g. component absent on this target).
    Skipped,
    /// Attempted but failed — the reason is in `detail`. NOT fatal to the snapshot.
    Failed,
}

/// One captured component + its outcome (serialised into the manifest).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub name: String,
    pub status: Status,
    pub detail: String,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub bytes: u64,
}

impl Component {
    fn captured(name: &str, detail: impl Into<String>, files: Vec<String>, bytes: u64) -> Self {
        Self { name: name.into(), status: Status::Captured, detail: detail.into(), files, bytes }
    }
    fn skipped(name: &str, detail: impl Into<String>) -> Self {
        Self { name: name.into(), status: Status::Skipped, detail: detail.into(), files: vec![], bytes: 0 }
    }
    fn failed(name: &str, detail: impl Into<String>) -> Self {
        Self { name: name.into(), status: Status::Failed, detail: detail.into(), files: vec![], bytes: 0 }
    }
}

/// The top-level manifest, written as both `manifest.json` and `MANIFEST.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub tool: String,
    pub env: String,
    pub driver: String,
    pub project_ref: Option<String>,
    pub taken_at: String,
    pub taken_at_source: String,
    pub read_only: bool,
    pub app_schemas_requested: Vec<String>,
    pub app_schemas_captured: Vec<String>,
    pub managed_schemas_present: Vec<String>,
    pub total_bytes: u64,
    pub components: Vec<Component>,
    pub gaps: Vec<String>,
}

impl Manifest {
    /// Serialise to pretty JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// Render a human-readable MANIFEST.md (what a recovering operator reads first).
    pub fn to_markdown(&self) -> String {
        let mut m = String::new();
        m.push_str(&format!("# Database snapshot — env `{}`\n\n", self.env));
        m.push_str(&format!("- Taken at: **{}** (source: {})\n", self.taken_at, self.taken_at_source));
        m.push_str(&format!("- Driver: `{}`\n", self.driver));
        if let Some(r) = &self.project_ref {
            m.push_str(&format!("- Project ref: `{r}`\n"));
        }
        m.push_str(&format!("- Read-only against source: {}\n", self.read_only));
        m.push_str(&format!("- Total captured size: {} bytes\n", self.total_bytes));
        m.push_str(&format!("- App schemas captured: {}\n", join_or_none(&self.app_schemas_captured)));
        m.push_str(&format!("- Managed schemas present (not dumped): {}\n\n", join_or_none(&self.managed_schemas_present)));

        m.push_str("## Components\n\n");
        m.push_str("| component | status | bytes | detail |\n|---|---|---|---|\n");
        for c in &self.components {
            let status = match c.status {
                Status::Captured => "captured",
                Status::Skipped => "skipped",
                Status::Failed => "FAILED",
            };
            m.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                c.name,
                status,
                c.bytes,
                c.detail.replace('\n', " ").replace('|', "\\|")
            ));
        }
        m.push('\n');

        if !self.gaps.is_empty() {
            m.push_str("## Recovery gaps (READ THIS)\n\n");
            for g in &self.gaps {
                m.push_str(&format!("- {g}\n"));
            }
            m.push('\n');
        }
        m.push_str("## How to recover (manual)\n\n");
        m.push_str("1. Provision a fresh Postgres/Supabase target.\n");
        m.push_str("2. Apply `roles.sql`, then `schema.sql`, then `data.sql` (in that order).\n");
        m.push_str("3. Reinstall the extensions listed in `extensions.txt`.\n");
        m.push_str("4. Recreate the pg_cron schedule from `cron.sql`.\n");
        m.push_str("5. Redeploy the edge functions in `edge_functions/` (see `edge_deploys.json`).\n");
        m
    }
}

fn join_or_none(v: &[String]) -> String {
    if v.is_empty() {
        "(none)".to_string()
    } else {
        v.join(", ")
    }
}

/// The full result handed back to the CLI.
#[derive(Debug, Clone)]
pub struct SnapshotReport {
    pub out_dir: PathBuf,
    pub manifest: Manifest,
    pub manifest_json_path: PathBuf,
    pub manifest_md_path: PathBuf,
}

// ── pure helpers (unit-testable, no DB/process) ──────────────────────────────

/// Compact an ISO-8601 UTC timestamp (`2026-07-07T17:23:49Z`) into a filesystem-safe
/// stamp (`20260707T172349Z`) for the default output directory name.
pub fn compact_stamp(iso: &str) -> String {
    let s: String = iso
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    // e.g. "20260707T172349Z"; if parsing produced nothing, fall back to a marker.
    if s.is_empty() {
        "unknown".to_string()
    } else {
        s
    }
}

/// The default output directory: `./.db-snapshots/<env>-<compact-stamp>/`.
pub fn default_out_dir(env: &str, iso_stamp: &str) -> PathBuf {
    PathBuf::from(".db-snapshots").join(format!("{env}-{}", compact_stamp(iso_stamp)))
}

/// Build the comma-separated `--schema` list `supabase db dump` expects.
pub fn schema_csv(schemas: &[String]) -> String {
    schemas.join(",")
}

/// Classify a schema as app (captured in full) or managed (presence only).
pub fn is_managed_schema(name: &str) -> bool {
    MANAGED_SCHEMAS.contains(&name)
}

// ── the snapshot itself ──────────────────────────────────────────────────────

/// Run a full snapshot of `db`'s active env into an output directory. Never writes to the
/// source database. Returns a [`SnapshotReport`]; individual component failures are
/// recorded in the manifest rather than aborting the whole snapshot.
pub async fn run(db: &Db, opts: &SnapshotOptions) -> Result<SnapshotReport> {
    let env = db.env_name.clone();
    let driver = db.driver();

    // 1. Timestamp (DB clock, then `date -u`).
    let (taken_at, taken_at_source) = resolve_timestamp(db).await;

    // 2. Output dir.
    let out_dir = opts
        .out
        .clone()
        .unwrap_or_else(|| default_out_dir(&env, &taken_at));
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| SubstrateError::Db(format!("creating snapshot dir {}: {e}", out_dir.display())))?;

    let requested: Vec<String> = if opts.app_schemas.is_empty() {
        DEFAULT_APP_SCHEMAS.iter().map(|s| s.to_string()).collect()
    } else {
        opts.app_schemas.clone()
    };

    let mut components: Vec<Component> = Vec::new();
    let mut gaps: Vec<String> = Vec::new();

    // 3. Discover the schemas actually present.
    let (all_schemas, present_err) = list_schemas(db).await;
    if let Some(e) = present_err {
        gaps.push(format!("could not enumerate schemas: {e}"));
    }
    let app_present: Vec<String> = requested
        .iter()
        .filter(|s| all_schemas.iter().any(|p| p == *s))
        .cloned()
        .collect();
    let managed_present: Vec<String> = all_schemas
        .iter()
        .filter(|s| is_managed_schema(s))
        .cloned()
        .collect();

    // Write schemas.txt (always — a plain record even if dumps later fail).
    match write_schemas_txt(&out_dir, &all_schemas, &app_present, &managed_present) {
        Ok((f, b)) => components.push(Component::captured("schemas", "all schemas classified", vec![f], b)),
        Err(e) => components.push(Component::failed("schemas", format!("{e}"))),
    }

    // 4. Resolve the dump URL (local exposes it; cloud needs DB_DUMP_URL).
    let dump_url = std::env::var("DB_DUMP_URL").ok().or_else(|| driver.dump_url());
    let cli_present = supabase_cli_present();
    let project_dir = project_dir_for(db);

    if dump_url.is_none() {
        gaps.push(
            "no dump URL available (DB_DUMP_URL unset and driver exposes none): schema.sql, \
             data.sql, cron.sql and roles.sql were NOT captured — the SQL-queryable parts \
             (extensions, grants, cron listing, edge registry) still were"
                .to_string(),
        );
    } else if !cli_present {
        gaps.push(
            "supabase CLI not on PATH: the version-matched pg_dump captures (schema/data/\
             cron/roles) were skipped"
                .to_string(),
        );
    }

    // 5. pg_dump-backed captures (schema / data / cron / roles) via `supabase db dump`.
    if let (Some(url), true) = (dump_url.as_deref(), cli_present) {
        if app_present.is_empty() {
            components.push(Component::skipped("schema", "no requested app schema present on target"));
            components.push(Component::skipped("data", "no requested app schema present on target"));
        } else {
            let csv = schema_csv(&app_present);
            // schema.sql (schema-only is the default).
            let schema_path = out_dir.join("schema.sql");
            components.push(
                capture_dump(&project_dir, url, &["--schema", &csv], &schema_path, "schema")
                    .await,
            );
            // data.sql (COPY form — completeness over readability, incl. blob tables).
            let data_path = out_dir.join("data.sql");
            let data_comp = capture_dump(
                &project_dir,
                url,
                &["--data-only", "--use-copy", "--schema", &csv],
                &data_path,
                "data",
            )
            .await;
            if data_comp.status == Status::Captured && data_comp.bytes > 50_000_000 {
                gaps.push(format!(
                    "data.sql is large ({} bytes) — includes blob tables like public.generated_assets",
                    data_comp.bytes
                ));
            }
            components.push(data_comp);
        }

        // cron.sql — only if the cron schema is present.
        if all_schemas.iter().any(|s| s == "cron") {
            let cron_path = out_dir.join("cron.sql");
            components.push(
                capture_dump(
                    &project_dir,
                    url,
                    &["--data-only", "--schema", "cron", "-x", "cron.job_run_details"],
                    &cron_path,
                    "cron",
                )
                .await,
            );
        } else {
            components.push(Component::skipped("cron", "no cron schema (pg_cron not installed)"));
        }

        // roles.sql — cluster roles + grants DDL.
        let roles_path = out_dir.join("roles.sql");
        components.push(
            capture_dump(&project_dir, url, &["--role-only"], &roles_path, "roles").await,
        );
    } else {
        for n in ["schema", "data", "cron", "roles"] {
            components.push(Component::skipped(n, "no dump URL / supabase CLI (see gaps)"));
        }
    }

    // 6. SQL-queryable captures (work over ANY Postgres driver: local pg or cloud mgmt-api).
    components.push(capture_query_file(db, &out_dir, "extensions.txt", "extensions", EXTENSIONS_SQL).await);
    components.push(capture_query_file(db, &out_dir, "grants.txt", "grants", GRANTS_SQL).await);
    components.push(capture_cron_listing(db, &out_dir).await);
    components.push(capture_edge_registry(db, &out_dir).await);

    // 7. Edge function SOURCE tree (on-disk copy).
    components.push(capture_edge_source(db, &out_dir));

    // 8. Manifest.
    let total_bytes: u64 = components.iter().map(|c| c.bytes).sum();
    if components.iter().any(|c| c.status == Status::Failed) {
        for c in components.iter().filter(|c| c.status == Status::Failed) {
            gaps.push(format!("component `{}` FAILED: {}", c.name, c.detail));
        }
    }
    let manifest = Manifest {
        tool: format!("db snapshot v{}", env!("CARGO_PKG_VERSION")),
        env: env.clone(),
        driver: driver.kind().as_str().to_string(),
        project_ref: db.config.env(&env).ok().and_then(|e| e.project_ref.clone()),
        taken_at,
        taken_at_source,
        read_only: true,
        app_schemas_requested: requested,
        app_schemas_captured: app_present,
        managed_schemas_present: managed_present,
        total_bytes,
        components,
        gaps,
    };

    let manifest_json_path = out_dir.join("manifest.json");
    let manifest_md_path = out_dir.join("MANIFEST.md");
    std::fs::write(&manifest_json_path, manifest.to_json())
        .map_err(|e| SubstrateError::Db(format!("writing manifest.json: {e}")))?;
    std::fs::write(&manifest_md_path, manifest.to_markdown())
        .map_err(|e| SubstrateError::Db(format!("writing MANIFEST.md: {e}")))?;

    Ok(SnapshotReport {
        out_dir,
        manifest,
        manifest_json_path,
        manifest_md_path,
    })
}

// ── component captures ───────────────────────────────────────────────────────

const EXTENSIONS_SQL: &str =
    "SELECT e.extname AS extension, e.extversion AS version, n.nspname AS schema \
     FROM pg_extension e JOIN pg_namespace n ON n.oid = e.extnamespace ORDER BY 1";

const GRANTS_SQL: &str = "SELECT table_schema, table_name, grantee, privilege_type \
     FROM information_schema.role_table_grants \
     WHERE table_schema IN ('public','core','mind','workshop','ops','actions') \
     ORDER BY 1,2,3,4";

/// Run `supabase db dump --db-url <url> -f <out> <extra…>` (a version-matched pg_dump) and
/// turn the result into a [`Component`]. Read-only against the source.
async fn capture_dump(
    project_dir: &str,
    db_url: &str,
    extra: &[&str],
    out: &Path,
    name: &str,
) -> Component {
    let out_str = match out.to_str() {
        Some(s) => s.to_string(),
        None => return Component::failed(name, "non-utf8 output path"),
    };
    let mut args: Vec<String> = vec![
        "db".into(),
        "dump".into(),
        "--db-url".into(),
        db_url.into(),
        "-f".into(),
        out_str,
    ];
    args.extend(extra.iter().map(|s| s.to_string()));

    let output = Command::new("supabase")
        .args(&args)
        .current_dir(project_dir)
        .output()
        .await;
    match output {
        Ok(o) if o.status.success() => {
            let bytes = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
            Component::captured(
                name,
                format!("supabase db dump {}", extra.join(" ")),
                vec![out.file_name().unwrap_or_default().to_string_lossy().into_owned()],
                bytes,
            )
        }
        Ok(o) => Component::failed(
            name,
            format!("supabase db dump failed: {}", String::from_utf8_lossy(&o.stderr).trim()),
        ),
        Err(e) => Component::failed(name, format!("spawning supabase db dump: {e}")),
    }
}

/// Run a read query through the driver and write its rows to a text file.
async fn capture_query_file(
    db: &Db,
    out_dir: &Path,
    file: &str,
    name: &str,
    sql: &str,
) -> Component {
    match db.driver().query(db.env_token.as_str(), sql).await {
        Ok(rows) => {
            let mut body = String::new();
            body.push_str(&rows.columns.join("\t"));
            body.push('\n');
            for r in &rows.rows {
                let cells: Vec<String> =
                    r.iter().map(|c| c.clone().unwrap_or_else(|| "NULL".into())).collect();
                body.push_str(&cells.join("\t"));
                body.push('\n');
            }
            let path = out_dir.join(file);
            match std::fs::write(&path, &body) {
                Ok(()) => Component::captured(
                    name,
                    format!("{} row(s)", rows.rows.len()),
                    vec![file.to_string()],
                    body.len() as u64,
                ),
                Err(e) => Component::failed(name, format!("writing {file}: {e}")),
            }
        }
        Err(e) => Component::failed(name, format!("query failed: {e}")),
    }
}

/// Capture the pg_cron schedule as a human-readable listing (belt-and-suspenders alongside
/// the SQL `cron.sql` dump — this one works over the cloud mgmt-api seam too).
async fn capture_cron_listing(db: &Db, out_dir: &Path) -> Component {
    let sql = "SELECT jobid, schedule, command, nodename, database, username, active, jobname \
               FROM cron.job ORDER BY jobid";
    match db.driver().query(db.env_token.as_str(), sql).await {
        Ok(rows) => {
            let mut body = String::from("# pg_cron schedule (cron.job)\n");
            body.push_str(&rows.columns.join("\t"));
            body.push('\n');
            for r in &rows.rows {
                let cells: Vec<String> =
                    r.iter().map(|c| c.clone().unwrap_or_else(|| "NULL".into())).collect();
                body.push_str(&cells.join("\t"));
                body.push('\n');
            }
            let path = out_dir.join("cron_jobs.txt");
            match std::fs::write(&path, &body) {
                Ok(()) => Component::captured(
                    "cron_listing",
                    format!("{} cron job(s)", rows.rows.len()),
                    vec!["cron_jobs.txt".into()],
                    body.len() as u64,
                ),
                Err(e) => Component::failed("cron_listing", format!("writing cron_jobs.txt: {e}")),
            }
        }
        // cron may not be installed on this target — that is a Skip, not a Failure.
        Err(e) => Component::skipped("cron_listing", format!("cron.job not queryable: {e}")),
    }
}

/// Capture the edge-deploy registry (`ops.edge_deploys`) as JSON: names + versions + the
/// content-addressed bundle refs, so a human knows exactly what was deployed.
async fn capture_edge_registry(db: &Db, out_dir: &Path) -> Component {
    let sql = "SELECT slug, handler, version, env, source_hash, bundle_ref, deployed_at \
               FROM ops.edge_deploys ORDER BY deployed_at";
    match db.driver().query(db.env_token.as_str(), sql).await {
        Ok(rows) => {
            let objs: Vec<serde_json::Value> = rows
                .rows
                .iter()
                .map(|r| {
                    let m: serde_json::Map<String, serde_json::Value> = rows
                        .columns
                        .iter()
                        .zip(r)
                        .map(|(c, v)| {
                            (
                                c.clone(),
                                v.clone()
                                    .map(serde_json::Value::String)
                                    .unwrap_or(serde_json::Value::Null),
                            )
                        })
                        .collect();
                    serde_json::Value::Object(m)
                })
                .collect();
            let body = serde_json::to_string_pretty(&objs).unwrap_or_else(|_| "[]".into());
            let path = out_dir.join("edge_deploys.json");
            match std::fs::write(&path, &body) {
                Ok(()) => Component::captured(
                    "edge_registry",
                    format!("{} deploy row(s)", objs.len()),
                    vec!["edge_deploys.json".into()],
                    body.len() as u64,
                ),
                Err(e) => Component::failed("edge_registry", format!("writing edge_deploys.json: {e}")),
            }
        }
        Err(e) => Component::skipped("edge_registry", format!("ops.edge_deploys not queryable: {e}")),
    }
}

/// Copy the on-disk edge-function source tree (config `dirs.edge`) into the snapshot so the
/// TypeScript sources are recoverable. Records names + total bytes.
fn capture_edge_source(db: &Db, out_dir: &Path) -> Component {
    let src = Path::new(&db.config.dirs.edge);
    if !src.exists() {
        return Component::skipped(
            "edge_source",
            format!("no edge source dir at {}", src.display()),
        );
    }
    let dest = out_dir.join("edge_functions");
    match copy_tree(src, &dest) {
        Ok((files, bytes)) => {
            let names: Vec<String> = std::fs::read_dir(src)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            Component::captured(
                "edge_source",
                format!("{} file(s) from {}: {}", files, src.display(), names.join(", ")),
                vec!["edge_functions/".into()],
                bytes,
            )
        }
        Err(e) => Component::failed("edge_source", format!("copying {}: {e}", src.display())),
    }
}

/// Recursively copy `src` into `dest`, returning (file count, total bytes).
fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<(u64, u64)> {
    std::fs::create_dir_all(dest)?;
    let mut files = 0u64;
    let mut bytes = 0u64;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        let target = dest.join(entry.file_name());
        if ft.is_dir() {
            let (f, b) = copy_tree(&entry.path(), &target)?;
            files += f;
            bytes += b;
        } else if ft.is_file() {
            let n = std::fs::copy(entry.path(), &target)?;
            files += 1;
            bytes += n;
        }
    }
    Ok((files, bytes))
}

// ── small internals ──────────────────────────────────────────────────────────

/// Best-effort UTC timestamp: DB clock first (crate forbids `Date::now()`), then `date -u`.
async fn resolve_timestamp(db: &Db) -> (String, String) {
    let sql = "SELECT to_char(now() AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS ts";
    if let Ok(rows) = db.driver().query(db.env_token.as_str(), sql).await {
        if let Some(ts) = rows.rows.first().and_then(|r| r.first().cloned().flatten()) {
            if !ts.is_empty() {
                return (ts, "db now()".into());
            }
        }
    }
    if let Ok(out) = std::process::Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
    {
        if out.status.success() {
            let ts = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !ts.is_empty() {
                return (ts, "date -u".into());
            }
        }
    }
    ("unknown".into(), "unavailable".into())
}

/// Every schema present on the target (best-effort; returns any query error alongside).
async fn list_schemas(db: &Db) -> (Vec<String>, Option<String>) {
    let sql = "SELECT nspname FROM pg_namespace WHERE nspname NOT LIKE 'pg\\_%' \
               AND nspname <> 'information_schema' ORDER BY 1";
    match db.driver().query(db.env_token.as_str(), sql).await {
        Ok(rows) => (
            rows.rows
                .into_iter()
                .filter_map(|r| r.into_iter().next().flatten())
                .collect(),
            None,
        ),
        Err(e) => (Vec::new(), Some(format!("{e}"))),
    }
}

fn write_schemas_txt(
    out_dir: &Path,
    all: &[String],
    app_present: &[String],
    managed_present: &[String],
) -> Result<(String, u64)> {
    let mut body = String::from("# Schemas present on target (app vs managed)\n\n");
    body.push_str("## App schemas (captured in full)\n");
    for s in app_present {
        body.push_str(&format!("- {s}\n"));
    }
    body.push_str("\n## Managed schemas (presence recorded; internals NOT dumped)\n");
    for s in managed_present {
        body.push_str(&format!("- {s}\n"));
    }
    body.push_str("\n## All schemas\n");
    for s in all {
        body.push_str(&format!("- {s}\n"));
    }
    let path = out_dir.join("schemas.txt");
    std::fs::write(&path, &body)
        .map_err(|e| SubstrateError::Db(format!("writing schemas.txt: {e}")))?;
    Ok(("schemas.txt".to_string(), body.len() as u64))
}

fn supabase_cli_present() -> bool {
    std::process::Command::new("supabase")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// The dir to run `supabase db dump` from — the project dir for a supabase-local env,
/// else the current dir.
fn project_dir_for(db: &Db) -> String {
    if db.driver_kind() == DriverKind::SupabaseLocal {
        db.config
            .env(&db.env_name)
            .ok()
            .and_then(|e| e.project_dir.clone())
            .unwrap_or_else(|| ".".to_string())
    } else {
        ".".to_string()
    }
}
