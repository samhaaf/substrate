//! Migration discovery, ordering (topological cross-schema, C1), apply / rollback /
//! status, and the crawl harness (design §4–§6). A migration is a *directory* of up to
//! six files under `db/migrations/<schema>/<seq>_<name>/` plus optional headers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use substrate_types::{Result, SubstrateError};

use crate::config::DriverKind;
use crate::driver::{AppliedMigration, Driver};

/// Deterministic schema tie-break order for independent migrations (design §4.5).
pub const SCHEMA_ORDER: &[&str] = &["ops", "public", "core", "mind"];

/// The six well-known files in a migration directory (design §5.1).
pub const MIGRATION_FILES: &[&str] = &[
    "up.sql",
    "down.sql",
    "test_up.sql",
    "test_down.sql",
    "fakedata_up.sql",
    "fakedata_down.sql",
];

/// A discovered migration on disk.
#[derive(Debug, Clone)]
pub struct Migration {
    /// e.g. `core`.
    pub schema: String,
    /// Per-schema monotonic sequence, zero-padded 4 digits.
    pub seq: u32,
    /// e.g. `cohort_weeks_window`.
    pub name: String,
    /// `<schema>/<seq>_<name>` — the ledger + attestation id (design §5.2).
    pub id: String,
    /// Directory of the migration.
    pub dir: PathBuf,
    /// Cross-schema dependencies declared via `@depends_on` (design §4.5).
    pub depends_on: Vec<String>,
    /// sha256 of `up.sql` (drift + M2 attest).
    pub checksum: String,
}

impl Migration {
    fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Read a migration file, returning `None` if absent.
    pub fn read(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.file(name)).ok()
    }

    /// Whether the migration deploys/activates an edge handler (parsed from headers).
    pub fn is_edge(&self) -> bool {
        self.read("up.sql")
            .map(|s| s.contains("@activate") && (self.dir.join("handler.yaml").exists()))
            .unwrap_or(false)
            || self.dir.join("handler.yaml").exists()
    }
}

/// Parse `@depends_on:` headers out of an `up.sql` body.
fn parse_depends_on(up: &str) -> Vec<String> {
    up.lines()
        .filter_map(|l| {
            let l = l.trim();
            l.strip_prefix("-- @depends_on:")
                .map(|rest| rest.trim().to_string())
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn sha256_hex(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    format!("{:x}", h.finalize())
}

/// Discover all migrations under `migrations_dir`, grouped by schema, seq-ordered.
pub fn discover(migrations_dir: &Path) -> Result<Vec<Migration>> {
    let mut out = Vec::new();
    if !migrations_dir.exists() {
        return Ok(out);
    }
    for schema_entry in read_dir_sorted(migrations_dir)? {
        if !schema_entry.is_dir() {
            continue;
        }
        let schema = schema_entry
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        for mig_dir in read_dir_sorted(&schema_entry)? {
            if !mig_dir.is_dir() {
                continue;
            }
            let dir_name = mig_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            // `<seq>_<name>`
            let (seq_str, name) = match dir_name.split_once('_') {
                Some((s, n)) => (s, n.to_string()),
                None => continue,
            };
            let seq: u32 = match seq_str.parse() {
                Ok(n) => n,
                Err(_) => continue,
            };
            let up = std::fs::read_to_string(mig_dir.join("up.sql")).unwrap_or_default();
            let id = format!("{schema}/{dir_name}");
            out.push(Migration {
                schema: schema.clone(),
                seq,
                name,
                id,
                depends_on: parse_depends_on(&up),
                checksum: sha256_hex(&up),
                dir: mig_dir,
            });
        }
    }
    Ok(out)
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| SubstrateError::Db(format!("reading {}: {e}", dir.display())))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    Ok(entries)
}

/// Topologically sort migrations into apply order (design §4.5 / §6.1).
///
/// Edges: intra-schema per-seq order + cross-schema `@depends_on`. Tie-break for
/// independent nodes: `SCHEMA_ORDER` then per-schema seq. A cycle or a missing
/// dependency is a lint error (`SubstrateError::Db`).
pub fn topo_order(migs: &[Migration]) -> Result<Vec<Migration>> {
    let by_id: HashMap<&str, &Migration> = migs.iter().map(|m| (m.id.as_str(), m)).collect();

    // Build adjacency: dep -> dependents (so we can Kahn from roots).
    let mut indegree: HashMap<&str, usize> = migs.iter().map(|m| (m.id.as_str(), 0)).collect();
    let mut edges: HashMap<&str, Vec<&str>> = HashMap::new();

    // intra-schema seq edges: within a schema, seq N depends on N-1 (nearest prior).
    let mut by_schema: BTreeMap<&str, Vec<&Migration>> = BTreeMap::new();
    for m in migs {
        by_schema.entry(m.schema.as_str()).or_default().push(m);
    }
    for list in by_schema.values_mut() {
        list.sort_by_key(|m| m.seq);
        for w in list.windows(2) {
            let (a, b) = (w[0], w[1]);
            edges.entry(a.id.as_str()).or_default().push(b.id.as_str());
            *indegree.get_mut(b.id.as_str()).unwrap() += 1;
        }
    }

    // cross-schema @depends_on edges.
    for m in migs {
        for dep in &m.depends_on {
            let dep_key = by_id.keys().find(|k| dep_matches(k, dep)).copied();
            match dep_key {
                Some(dk) => {
                    edges.entry(dk).or_default().push(m.id.as_str());
                    *indegree.get_mut(m.id.as_str()).unwrap() += 1;
                }
                None => {
                    return Err(SubstrateError::Db(format!(
                        "migration {} depends_on {} which does not exist",
                        m.id, dep
                    )));
                }
            }
        }
    }

    // Kahn with deterministic tie-break.
    let mut ready: Vec<&str> = indegree
        .iter()
        .filter(|(_, &d)| d == 0)
        .map(|(k, _)| *k)
        .collect();
    sort_ready(&mut ready, &by_id);

    let mut order: Vec<Migration> = Vec::with_capacity(migs.len());
    while let Some(next) = ready.pop() {
        order.push((*by_id[next]).clone());
        if let Some(deps) = edges.get(next) {
            for &d in deps {
                let e = indegree.get_mut(d).unwrap();
                *e -= 1;
                if *e == 0 {
                    ready.push(d);
                }
            }
            sort_ready(&mut ready, &by_id);
        }
    }

    if order.len() != migs.len() {
        return Err(SubstrateError::Db(
            "migration dependency graph has a cycle".to_string(),
        ));
    }
    Ok(order)
}

/// `@depends_on: mind/0009_meta_markdown` may omit the `_<name>` suffix; match on the
/// `<schema>/<seq>` prefix.
fn dep_matches(id: &str, dep: &str) -> bool {
    id == dep || id.starts_with(&format!("{dep}_")) || {
        // dep might be `mind/0009` (schema/seq only)
        if let Some((dschema, dseq)) = dep.split_once('/') {
            if let Some((ischema, rest)) = id.split_once('/') {
                ischema == dschema && rest.starts_with(dseq)
            } else {
                false
            }
        } else {
            false
        }
    }
}

/// Deterministic tie-break: schema order then seq. We `pop()` from the end so sort
/// descending to pop the smallest first.
fn sort_ready(ready: &mut [&str], by_id: &HashMap<&str, &Migration>) {
    ready.sort_by(|a, b| {
        let ma = by_id[a];
        let mb = by_id[b];
        let sa = SCHEMA_ORDER.iter().position(|s| *s == ma.schema).unwrap_or(usize::MAX);
        let sb = SCHEMA_ORDER.iter().position(|s| *s == mb.schema).unwrap_or(usize::MAX);
        // reverse so pop() yields ascending
        (sb, mb.seq).cmp(&(sa, ma.seq))
    });
}

/// The set of migration ids already applied for `env` (from the ledger).
pub async fn applied_ids(driver: &dyn Driver, env: &str) -> Result<HashSet<String>> {
    let table = if driver.kind() == DriverKind::Sqlite {
        "ops_applied_migrations"
    } else {
        "ops.applied_migrations"
    };
    let sql = format!("SELECT id FROM {table} WHERE env = '{env}'");
    let rows = driver.query(env, &sql).await.unwrap_or_default();
    Ok(rows
        .rows
        .into_iter()
        .filter_map(|r| r.into_iter().next().flatten())
        .collect())
}

/// Whether `test_up.sql` should run for this env: on prod (`env==''` + protected),
/// fakedata is skipped and only `up → test_up` runs (design §6.2).
pub struct ApplyContext<'a> {
    pub env: &'a str,
    /// True on local / stage (fakedata allowed). False on promote/prod (skip fakedata).
    pub allow_fakedata: bool,
    /// True when the driver is edge-incapable (sqlite) — edge migrations NI at apply.
    pub edge_incapable: bool,
}

/// Apply a single migration ATOMICALLY (design §6.1/§6.2/§6.5, findings 4 + 6).
///
/// Collects `up.sql` → (fakedata_up on non-prod) → `test_up.sql` and hands the whole
/// sequence + the ledger INSERT to [`Driver::apply_atomic`], which runs them in ONE
/// transaction under a transaction-scoped advisory lock
/// (`pg_advisory_xact_lock(hashtext('db:'||env))`). If ANY step fails — e.g. a `test_up`
/// assertion RAISEs — the transaction rolls back: the up-DDL is reverted and NO ledger
/// row is written (no partial state). The xact lock auto-releases at COMMIT/ROLLBACK, so
/// it can never leak. Edge migrations on an edge-incapable driver raise NotImplemented.
pub async fn apply_one(
    driver: &dyn Driver,
    m: &Migration,
    ctx: &ApplyContext<'_>,
) -> Result<()> {
    if m.is_edge() && ctx.edge_incapable {
        return Err(SubstrateError::NotImplemented {
            command: "migrate up (edge handler)",
            driver: driver.kind().as_str(),
            reason: "edge handlers are not supported by this driver",
        });
    }

    // Build the ordered statement list for the single atomic unit (§6.2 apply graph).
    let mut statements: Vec<String> = Vec::with_capacity(3);
    match m.read("up.sql") {
        Some(up) => statements.push(up),
        None => return Err(SubstrateError::Db(format!("{}: missing up.sql", m.id))),
    }
    if ctx.allow_fakedata {
        if let Some(fd) = m.read("fakedata_up.sql") {
            if !is_sentinel_only(&fd) {
                statements.push(fd);
            }
        }
    }
    if let Some(tu) = m.read("test_up.sql") {
        if !is_sentinel_only(&tu) {
            statements.push(tu);
        }
    }

    // Per-env advisory lock key (§6.5): serializes apply/promote/activate for this env.
    let lock_key = format!("db:{}", ctx.env);
    let ledger = AppliedMigration {
        env: ctx.env.to_string(),
        schema: m.schema.clone(),
        id: m.id.clone(),
        checksum: m.checksum.clone(),
    };
    driver
        .apply_atomic(ctx.env, &lock_key, &statements, ledger)
        .await
}

/// Roll back a single migration: `down.sql` → `test_down.sql` (design §6.3).
pub async fn rollback_one(driver: &dyn Driver, m: &Migration, env: &str) -> Result<()> {
    if let Some(down) = m.read("down.sql") {
        driver.apply_sql(env, &down).await?;
    }
    if let Some(td) = m.read("test_down.sql") {
        if !is_sentinel_only(&td) {
            driver.apply_sql(env, &td).await?;
        }
    }
    // Remove the ledger row — UNLESS the `down.sql` we just ran dropped the ledger itself.
    //
    // The `ops` baseline is a migration like any other, and its `down.sql` is
    // `DROP SCHEMA IF EXISTS ops CASCADE`, which takes `ops.applied_migrations` with it.
    // Issuing `DELETE FROM ops.applied_migrations` after that would error against a
    // non-existent relation and fail the per-migration crawl on the ledger's own
    // self-reference (the ledger cannot record the rollback of the ledger). Guard on the
    // table's actual existence: for an ordinary migration the ledger is intact so the
    // DELETE runs exactly as before (normal behavior is UNCHANGED); for a ledger-dropping
    // down the row is already gone with the table, so the DELETE is correctly skipped.
    if !ledger_table_exists(driver, env).await {
        return Ok(());
    }
    let table = if driver.kind() == DriverKind::Sqlite {
        "ops_applied_migrations"
    } else {
        "ops.applied_migrations"
    };
    driver
        .apply_sql(
            env,
            &format!("DELETE FROM {table} WHERE env='{env}' AND id='{}'", m.id),
        )
        .await?;
    Ok(())
}

/// Whether the migration ledger table still exists in `env` (design §3.1.a).
///
/// Used by [`rollback_one`] to decide whether the ledger-delete can run: a migration whose
/// `down.sql` drops the `ops` schema (the `ops` baseline) removes the ledger itself, so the
/// follow-up delete must be skipped. Returns `true` (delete proceeds) for every ordinary
/// migration, which leaves the ledger intact. A query error is treated as "absent" (safe:
/// the delete is skipped rather than erroring the rollback).
async fn ledger_table_exists(driver: &dyn Driver, env: &str) -> bool {
    let sql = if driver.kind() == DriverKind::Sqlite {
        "SELECT 1 AS present FROM sqlite_master \
         WHERE type='table' AND name='ops_applied_migrations'"
    } else {
        "SELECT 1 AS present WHERE to_regclass('ops.applied_migrations') IS NOT NULL"
    };
    driver
        .query(env, sql)
        .await
        .map(|rows| !rows.rows.is_empty())
        .unwrap_or(false)
}

/// A file that contains ONLY an `@intentionally-none` sentinel (or is whitespace).
pub fn is_sentinel_only(body: &str) -> bool {
    let non_comment: Vec<&str> = body
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with("--"))
        .collect();
    non_comment.is_empty()
}

/// The crawl harness (design §6.4): for each migration in apply order,
/// apply → test_up → down → test_down → reapply → test_up, then write an attestation.
/// REFUSED against protected refs at the command layer (not here).
pub async fn crawl(
    driver: &dyn Driver,
    migs: &[Migration],
    env: &str,
) -> Result<Vec<Attestation>> {
    let ordered = topo_order(migs)?;
    let ctx = ApplyContext {
        env,
        allow_fakedata: true,
        edge_incapable: driver.kind() == DriverKind::Sqlite,
    };
    let mut attestations = Vec::new();
    for m in &ordered {
        apply_one(driver, m, &ctx).await?;
        rollback_one(driver, m, env).await?;
        apply_one(driver, m, &ctx).await?;
        attestations.push(Attestation {
            schema: m.schema.clone(),
            id: m.id.clone(),
            checksum: m.checksum.clone(),
        });
    }
    Ok(attestations)
}

/// A crawl attestation to persist into `ops.crawl_attestations` (M2).
#[derive(Debug, Clone)]
pub struct Attestation {
    pub schema: String,
    pub id: String,
    pub checksum: String,
}

/// Write attestations to the ledger (Postgres only; sqlite has no attestations table).
pub async fn record_attestations(
    driver: &dyn Driver,
    atts: &[Attestation],
    by: &str,
) -> Result<()> {
    if driver.kind() == DriverKind::Sqlite {
        return Ok(()); // sqlite ledger-only; no attestations table in v1.
    }
    for a in atts {
        let sql = format!(
            "INSERT INTO ops.crawl_attestations (schema, id, checksum, crawled_by) \
             VALUES ('{}', '{}', '{}', '{}') \
             ON CONFLICT (id, checksum) DO UPDATE SET crawled_at = now(), crawled_by = EXCLUDED.crawled_by",
            a.schema, a.id, a.checksum, by
        );
        driver.apply_sql("", &sql).await?;
    }
    Ok(())
}

/// Scaffold a new migration directory with the six files + three sentinels (design §5,
/// `migrate new`). Returns the created directory.
pub fn scaffold_new(
    migrations_dir: &Path,
    schema: &str,
    name: &str,
    edge: bool,
    no_test: bool,
    no_data: bool,
) -> Result<PathBuf> {
    let schema_dir = migrations_dir.join(schema);
    std::fs::create_dir_all(&schema_dir)
        .map_err(|e| SubstrateError::Db(format!("creating {}: {e}", schema_dir.display())))?;
    let next = next_seq(&schema_dir)?;
    let dir_name = format!("{next:04}_{name}");
    let dir = schema_dir.join(&dir_name);
    std::fs::create_dir_all(&dir)
        .map_err(|e| SubstrateError::Db(format!("creating {}: {e}", dir.display())))?;

    let up_header = format!("-- @schema: {schema}\n-- forward DDL. Use :SCHEMA, never a literal prefix.\n");
    write_file(&dir.join("up.sql"), &up_header)?;
    write_file(&dir.join("down.sql"), "-- rollback DDL\n")?;

    let test_body = if no_test {
        "-- @intentionally-none: scaffolded with --no-test\n"
    } else {
        "-- assertions proving up worked; MUST be self-provisioning (H3)\n"
    };
    write_file(&dir.join("test_up.sql"), test_body)?;
    write_file(
        &dir.join("test_down.sql"),
        if no_test {
            "-- @intentionally-none: scaffolded with --no-test\n"
        } else {
            "-- assertions proving down worked\n"
        },
    )?;

    let data_body = if no_data {
        "-- @intentionally-none: scaffolded with --no-data\n"
    } else {
        "-- NON-PROD fake data (never runs on promote)\n"
    };
    write_file(&dir.join("fakedata_up.sql"), data_body)?;
    write_file(&dir.join("fakedata_down.sql"), data_body)?;

    if edge {
        write_file(
            &dir.join("handler.yaml"),
            "# edge handler contract — see db/handlers/<name>/<vN>.yaml\n",
        )?;
    }
    Ok(dir)
}

fn write_file(path: &Path, body: &str) -> Result<()> {
    std::fs::write(path, body)
        .map_err(|e| SubstrateError::Db(format!("writing {}: {e}", path.display())))
}

/// Next per-schema seq (monotonic, 4-digit; design §5.1).
pub fn next_seq(schema_dir: &Path) -> Result<u32> {
    let mut max = 0u32;
    if let Ok(rd) = std::fs::read_dir(schema_dir) {
        for e in rd.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some((s, _)) = name.split_once('_') {
                    if let Ok(n) = s.parse::<u32>() {
                        max = max.max(n);
                    }
                }
            }
        }
    }
    Ok(max + 1)
}
