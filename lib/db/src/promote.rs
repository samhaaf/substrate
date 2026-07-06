//! Promote invariant gate (design §9.3, M2/M5/M7/M8) and `doctor` support (§9.6).
//!
//! Nothing here is applied to remote/prod Supabase by the build; the gate is the
//! sequence of pre-write checks that MUST pass before any prod write.

use substrate_types::{Result, SubstrateError};

use crate::config::DbConfig;
use crate::driver::Driver;
use crate::lint::{self, LintReport};
use crate::migration::{self, Migration};

/// The outcome of a promote gate evaluation.
#[derive(Debug, Clone, Default)]
pub struct GateReport {
    pub lint: Vec<String>,
    /// Migrations with a missing/stale crawl attestation (M2).
    pub missing_attestations: Vec<String>,
    pub stale_codegen: bool,
    /// Fatal reasons that block the promote.
    pub blockers: Vec<String>,
}

impl GateReport {
    pub fn is_clear(&self) -> bool {
        self.blockers.is_empty()
    }
}

/// Run the promote invariant gate (design §9.3), WITHOUT applying anything. Steps:
/// 1. lint clean; 2. codegen --check clean (caller supplies `codegen_stale`);
/// 3. crawl attestation present + checksum-matched per to-be-applied migration (M2).
/// Steps 4–6 (timeout sanity, deployed-not-activated, object parity) are folded into
/// lint + the attestation check for v1.
pub async fn gate(
    driver: &dyn Driver,
    migs: &[Migration],
    pending: &[Migration],
    codegen_stale: bool,
) -> Result<GateReport> {
    let mut report = GateReport::default();

    // 1. lint
    let lint_report: LintReport = lint::lint(migs);
    if !lint_report.is_clean() {
        for f in lint_report.errors() {
            report.lint.push(format!("{}: {}", f.migration, f.message));
        }
        report
            .blockers
            .push("migrate lint is not clean".to_string());
    }

    // 2. codegen --check
    if codegen_stale {
        report.stale_codegen = true;
        report
            .blockers
            .push("checked-in generated files are stale (run `db handler codegen`)".to_string());
    }

    // 3. crawl attestation present + checksum-matched, per pending migration (M2).
    for m in pending {
        let sql = format!(
            "SELECT 1 FROM ops.crawl_attestations WHERE id = '{}' AND checksum = '{}'",
            m.id, m.checksum
        );
        let rows = driver.query("", &sql).await.unwrap_or_default();
        if rows.rows.is_empty() {
            report.missing_attestations.push(m.id.clone());
        }
    }
    if !report.missing_attestations.is_empty() {
        report.blockers.push(format!(
            "{} migration(s) lack a matching crawl attestation",
            report.missing_attestations.len()
        ));
    }

    Ok(report)
}

/// Bind promote to the intended cloud prod target (design §9.3, M5, finding 3).
///
/// Typed-confirm alone is insufficient — the target must be the intended backend. This
/// refuses unless BOTH hold:
/// 1. the resolved driver advertises `capabilities().promote_target` (only supabase-cloud
///    does — a local/sqlite target is refused so promote can't write `env=''` rows into
///    the wrong backend), AND
/// 2. the env's `project_ref` is in `[safety].protected_refs` (the real prod identity, M5)
///    — a non-prod cloud project is refused too.
pub fn assert_promote_target(cfg: &DbConfig, env_name: &str, driver: &dyn Driver) -> Result<()> {
    if !driver.capabilities().promote_target {
        return Err(SubstrateError::Db(format!(
            "promote refused: env `{env_name}` resolves to driver `{}`, which is not a promote \
             target (only supabase-cloud prod can be promoted TO)",
            driver.kind()
        )));
    }
    let env_cfg = cfg.env(env_name)?;
    let is_prod_ref = env_cfg
        .project_ref
        .as_deref()
        .map(|r| cfg.is_protected_ref(r))
        .unwrap_or(false);
    if !is_prod_ref {
        return Err(SubstrateError::Db(format!(
            "promote refused: env `{env_name}` is not bound to a protected/prod project_ref \
             (target must be the intended prod in [safety].protected_refs)"
        )));
    }
    Ok(())
}

/// Verify the typed-confirmation phrase (design §9.2). Returns Ok only if `typed`
/// exactly equals the config's `confirm_phrase`.
pub fn check_confirm(cfg: &DbConfig, typed: &str) -> Result<()> {
    if typed == cfg.safety.confirm_phrase {
        Ok(())
    } else {
        Err(SubstrateError::Db(format!(
            "typed-confirmation mismatch: expected exactly \"{}\"",
            cfg.safety.confirm_phrase
        )))
    }
}

/// Compute the set of pending migrations for the prod env (topo-ordered).
pub async fn pending_for_prod(driver: &dyn Driver, migs: &[Migration]) -> Result<Vec<Migration>> {
    let applied = migration::applied_ids(driver, "").await?;
    let ordered = migration::topo_order(migs)?;
    Ok(ordered
        .into_iter()
        .filter(|m| !applied.contains(&m.id))
        .collect())
}

/// `db doctor` health/preflight report (design §9.6, M6).
#[derive(Debug, Clone, Default)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

#[derive(Debug, Clone)]
pub struct DoctorCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl DoctorReport {
    pub fn add(&mut self, name: impl Into<String>, ok: bool, detail: impl Into<String>) {
        self.checks.push(DoctorCheck {
            name: name.into(),
            ok,
            detail: detail.into(),
        });
    }

    pub fn all_ok(&self) -> bool {
        self.checks.iter().all(|c| c.ok)
    }
}

/// Run doctor checks against a driver (design §9.6). v1 covers: driver reachable,
/// ledger present, http/pg_net extensions (Postgres), and a codegen staleness flag
/// passed by the caller.
pub async fn doctor(driver: &dyn Driver, cfg: &DbConfig) -> Result<DoctorReport> {
    let mut report = DoctorReport::default();

    // driver reachable (a trivial query)
    match driver.query("", "SELECT 1").await {
        Ok(_) => report.add("driver-reachable", true, driver.kind().to_string()),
        Err(e) => report.add("driver-reachable", false, e.to_string()),
    }

    if driver.kind() != crate::config::DriverKind::Sqlite {
        // ledger present
        match driver.query("", "SELECT count(*) FROM ops.applied_migrations").await {
            Ok(_) => report.add("ledger-present", true, "ops.applied_migrations"),
            Err(e) => report.add("ledger-present", false, e.to_string()),
        }
        // http + pg_net extensions (M6)
        for ext in ["http", "pg_net"] {
            let sql = format!("SELECT 1 FROM pg_extension WHERE extname = '{ext}'");
            match driver.query("", &sql).await {
                Ok(rows) if !rows.rows.is_empty() => {
                    report.add(format!("ext-{ext}"), true, "present")
                }
                Ok(_) => report.add(format!("ext-{ext}"), false, "missing"),
                Err(e) => report.add(format!("ext-{ext}"), false, e.to_string()),
            }
        }
    }

    report.add(
        "protected-refs-configured",
        !cfg.safety.protected_refs.is_empty(),
        format!("{} protected ref(s)", cfg.safety.protected_refs.len()),
    );

    Ok(report)
}
