//! `db migrate lint` — sequence + forbidden-pattern + the three sentinel gates +
//! breaking-sig + fakedata-dependency checks (design §5.3, §9.4, §9.5).
//!
//! The three sentinel strings are LOCKED byte-for-byte here because lint correctness
//! depends on them exactly.

use std::collections::BTreeMap;

use crate::migration::{is_sentinel_only, Migration};

/// LOCKED sentinel tokens (design §5.3 / §14). Each requires a `: <reason>` suffix.
pub const SENTINEL_INTENTIONALLY_NONE: &str = "@intentionally-none";
pub const SENTINEL_DEPLOYED_NOT_ACTIVATED: &str = "@deployed-not-activated";
pub const SENTINEL_FAIL_OPEN_JUSTIFIED: &str = "@fail-open-justified";
pub const SENTINEL_BREAKING_SIG_ROLLBACK: &str = "@breaking-sig-rollback";

/// Forbidden non-reversible DDL patterns in HAND-WRITTEN SQL (codegen output is
/// exempt/fenced — design §9.5).
const FORBIDDEN_PATTERNS: &[&str] = &[
    "CREATE OR REPLACE",
    "IF NOT EXISTS",
    "IF EXISTS",
    "ALTER FUNCTION",
    "ALTER EXTENSION",
];

/// Fence markers delimiting codegen output, which is exempt from forbidden-pattern lint.
const CODEGEN_FENCE_BEGIN: &str = "-- @codegen:begin";
const CODEGEN_FENCE_END: &str = "-- @codegen:end";

/// A single lint diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub level: Level,
    pub migration: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

/// The result of a lint pass.
#[derive(Debug, Clone, Default)]
pub struct LintReport {
    pub findings: Vec<Finding>,
}

impl LintReport {
    pub fn errors(&self) -> impl Iterator<Item = &Finding> {
        self.findings.iter().filter(|f| f.level == Level::Error)
    }

    pub fn is_clean(&self) -> bool {
        self.errors().next().is_none()
    }

    fn err(&mut self, migration: &str, message: impl Into<String>) {
        self.findings.push(Finding {
            level: Level::Error,
            migration: migration.to_string(),
            message: message.into(),
        });
    }

    fn warn(&mut self, migration: &str, message: impl Into<String>) {
        self.findings.push(Finding {
            level: Level::Warning,
            migration: migration.to_string(),
            message: message.into(),
        });
    }
}

/// Run all lint gates over the discovered migrations (design §9.4 / §9.5).
pub fn lint(migs: &[Migration]) -> LintReport {
    let mut report = LintReport::default();
    lint_sequence(migs, &mut report);
    for m in migs {
        lint_missing_files(m, &mut report);
        lint_forbidden_patterns(m, &mut report);
        lint_activation_gate(m, &mut report);
        lint_fail_policy_gate(m, &mut report);
        lint_fakedata_dependency(m, &mut report);
    }
    lint_depends_on_graph(migs, &mut report);
    report
}

/// Monotonic-per-schema sequence: no gaps, no dupes (design §9.5).
fn lint_sequence(migs: &[Migration], report: &mut LintReport) {
    let mut by_schema: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    for m in migs {
        by_schema.entry(m.schema.as_str()).or_default().push(m.seq);
    }
    for (schema, mut seqs) in by_schema {
        seqs.sort_unstable();
        for w in seqs.windows(2) {
            if w[0] == w[1] {
                report.err(schema, format!("duplicate seq {:04} in schema {schema}", w[0]));
            } else if w[1] != w[0] + 1 {
                report.err(
                    schema,
                    format!("gap in schema {schema}: {:04} → {:04}", w[0], w[1]),
                );
            }
        }
    }
}

/// missing-file gate: every migration has all six files OR the absent/empty ones carry
/// `@intentionally-none: <reason>` (design §9.4).
fn lint_missing_files(m: &Migration, report: &mut LintReport) {
    for file in crate::migration::MIGRATION_FILES {
        match m.read(file) {
            None => report.err(
                &m.id,
                format!("missing {file} (add it or `-- {SENTINEL_INTENTIONALLY_NONE}: <reason>`)"),
            ),
            Some(body) => {
                if is_sentinel_only(&body) {
                    // Empty-ish: must carry the sentinel WITH a reason.
                    if !has_sentinel_with_reason(&body, SENTINEL_INTENTIONALLY_NONE) {
                        // An empty test/data file without a reason is a fail; up/down
                        // may be legitimately short so only gate the optional files.
                        if is_optional_file(file) && !body.contains(SENTINEL_INTENTIONALLY_NONE) {
                            report.err(
                                &m.id,
                                format!(
                                    "{file} is empty; add content or `-- {SENTINEL_INTENTIONALLY_NONE}: <reason>`"
                                ),
                            );
                        } else if body.contains(SENTINEL_INTENTIONALLY_NONE) {
                            report.err(
                                &m.id,
                                format!("{file} has a bare {SENTINEL_INTENTIONALLY_NONE} with no `: <reason>`"),
                            );
                        }
                    }
                }
            }
        }
    }
}

fn is_optional_file(file: &str) -> bool {
    matches!(
        file,
        "test_up.sql" | "test_down.sql" | "fakedata_up.sql" | "fakedata_down.sql"
    )
}

/// activation gate: a migration that deploys an edge `_vN` either `@activate`s it OR
/// carries `-- @deployed-not-activated: <reason>` (design §9.4).
fn lint_activation_gate(m: &Migration, report: &mut LintReport) {
    let up = m.read("up.sql").unwrap_or_default();
    let handler_yaml = m
        .dir
        .join("handler.yaml")
        .exists()
        .then(|| std::fs::read_to_string(m.dir.join("handler.yaml")).unwrap_or_default())
        .unwrap_or_default();
    let deploys_edge = up.contains("deploy") && m.is_edge();
    if deploys_edge {
        let activated = up.contains("@activate") || handler_yaml.contains("@activate");
        let sentinel = has_sentinel_with_reason(&up, SENTINEL_DEPLOYED_NOT_ACTIVATED)
            || has_sentinel_with_reason(&handler_yaml, SENTINEL_DEPLOYED_NOT_ACTIVATED);
        if !activated && !sentinel {
            report.err(
                &m.id,
                format!("deploys an edge fn but neither @activates it nor carries `-- {SENTINEL_DEPLOYED_NOT_ACTIVATED}: <reason>`"),
            );
        }
    }
}

/// fail-policy gate: a `fail_policy: fail-open` handler carries
/// `-- @fail-open-justified: <reason>` (design §9.4).
fn lint_fail_policy_gate(m: &Migration, report: &mut LintReport) {
    let hy = m.dir.join("handler.yaml");
    if hy.exists() {
        let body = std::fs::read_to_string(&hy).unwrap_or_default();
        if body.contains("fail-open")
            && !has_sentinel_with_reason(&body, SENTINEL_FAIL_OPEN_JUSTIFIED)
        {
            report.err(
                &m.id,
                format!("fail_policy: fail-open requires `-- {SENTINEL_FAIL_OPEN_JUSTIFIED}: <reason>`"),
            );
        }
    }
}

/// forbidden-pattern lint over hand-written DDL (codegen output fenced/exempt).
fn lint_forbidden_patterns(m: &Migration, report: &mut LintReport) {
    for file in ["up.sql", "down.sql"] {
        if let Some(body) = m.read(file) {
            let hand_written = strip_codegen_fences(&body);
            let upper = hand_written.to_uppercase();
            for pat in FORBIDDEN_PATTERNS {
                if upper.contains(pat) {
                    report.err(
                        &m.id,
                        format!("{file}: forbidden non-reversible pattern `{pat}` in hand-written DDL"),
                    );
                }
            }
        }
    }
}

/// Remove text between codegen fences so it is exempt from forbidden-pattern lint.
fn strip_codegen_fences(body: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with(CODEGEN_FENCE_BEGIN) {
            in_fence = true;
            continue;
        }
        if t.starts_with(CODEGEN_FENCE_END) {
            in_fence = false;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// fakedata-dependency check (design §6.2): warn if `test_up.sql` asserts against a
/// table that only `fakedata_up.sql` populates (heuristic) — the test may not be
/// prod-safe. This is a warning, resolved by making the test self-provisioning.
fn lint_fakedata_dependency(m: &Migration, report: &mut LintReport) {
    let test_up = m.read("test_up.sql").unwrap_or_default();
    let fakedata = m.read("fakedata_up.sql").unwrap_or_default();
    let up = m.read("up.sql").unwrap_or_default();
    if is_sentinel_only(&test_up) || is_sentinel_only(&fakedata) {
        return;
    }
    // Heuristic: a table INSERTed by fakedata but not by up, referenced in a test SELECT.
    for tbl in inserted_tables(&fakedata) {
        if !inserted_tables(&up).contains(&tbl)
            && test_up.to_lowercase().contains(&tbl.to_lowercase())
        {
            report.warn(
                &m.id,
                format!("test_up asserts against `{tbl}` populated only by fakedata; may not be prod-safe (make it self-provisioning)"),
            );
        }
    }
}

/// Very rough extraction of INSERT target tables (heuristic for the fakedata check).
fn inserted_tables(sql: &str) -> Vec<String> {
    let lower = sql.to_lowercase();
    let mut out = Vec::new();
    let mut idx = 0;
    while let Some(pos) = lower[idx..].find("insert into ") {
        let start = idx + pos + "insert into ".len();
        let rest = &sql[start..];
        let tbl: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '.')
            .collect();
        if !tbl.is_empty() {
            out.push(tbl);
        }
        idx = start;
    }
    out
}

/// `@depends_on` references resolve and the graph is acyclic (design §9.5). We reuse
/// the topo sort as the acyclicity check.
fn lint_depends_on_graph(migs: &[Migration], report: &mut LintReport) {
    if let Err(e) = crate::migration::topo_order(migs) {
        report.err("<graph>", e.to_string());
    }
}

/// A sentinel line `-- @name: <reason>` with a NON-EMPTY reason.
fn has_sentinel_with_reason(body: &str, sentinel: &str) -> bool {
    let needle = format!("-- {sentinel}:");
    body.lines().any(|l| {
        let t = l.trim();
        if let Some(rest) = t.strip_prefix(&needle) {
            !rest.trim().is_empty()
        } else {
            false
        }
    })
}

/// Compatibility-class of a signature change (design §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatClass {
    Compatible,
    Breaking,
}

/// Classify a handler signature change from prior params to new params (design §4.3):
/// removed/renamed/retyped param = Breaking; added optional / widened = Compatible.
pub fn classify_change(
    prior_params: &BTreeMap<String, String>,
    new_params: &BTreeMap<String, String>,
) -> CompatClass {
    for (name, ty) in prior_params {
        match new_params.get(name) {
            None => return CompatClass::Breaking,        // removed
            Some(new_ty) if new_ty != ty => return CompatClass::Breaking, // retyped
            _ => {}
        }
    }
    CompatClass::Compatible
}
