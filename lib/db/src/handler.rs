//! Handler contract + registry + codegen (design §4, §7).
//!
//! The handler contract file (`db/handlers/<name>/<vN>.yaml`) is parsed with a minimal
//! line-based parser (the contract is a flat, well-known shape — no full YAML dep is
//! pulled in for v1). Codegen emits, deterministically: a typed SQL wrapper
//! (`call_<handler>`), a TS edge signature + runtime guard, the stored contract jsonb,
//! and (for validator-sync) the trigger body + return-side guard (M4) + edge test_down
//! (H1).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};
use substrate_types::{Result, SubstrateError};

/// A parsed handler contract (design §4.1).
#[derive(Debug, Clone)]
pub struct Contract {
    pub handler: String,
    pub version: String,
    pub kind: Kind,
    pub invocation: Invocation,
    pub timeout_ms: Option<u32>,
    pub fail_policy: FailPolicy,
    pub idempotency_key: Option<String>,
    pub params: BTreeMap<String, String>,
    pub returns: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sql,
    Edge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Invocation {
    EffectAsync,
    ValidatorSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailPolicy {
    FailClosed,
    FailOpen,
}

impl Contract {
    /// `blocking` is DERIVED: true iff invocation == validator-sync (design §4.1).
    pub fn blocking(&self) -> bool {
        self.invocation == Invocation::ValidatorSync
    }

    /// Parse a contract from the flat line-based format.
    pub fn parse(text: &str) -> Result<Self> {
        let mut handler = None;
        let mut version = None;
        let mut kind = Kind::Sql;
        let mut invocation = Invocation::EffectAsync;
        let mut timeout_ms = None;
        let mut fail_policy = FailPolicy::FailClosed;
        let mut idempotency_key = None;
        let mut returns = "void".to_string();
        let mut params = BTreeMap::new();
        let mut in_params = false;

        for raw in text.lines() {
            let line = raw.split('#').next().unwrap_or("");
            if line.trim().is_empty() {
                continue;
            }
            // params: block — indented `  name: type`.
            if line.starts_with("params:") {
                in_params = true;
                continue;
            }
            if in_params && (raw.starts_with("  ") || raw.starts_with('\t')) {
                if let Some((k, v)) = line.trim().split_once(':') {
                    params.insert(k.trim().to_string(), v.trim().to_string());
                }
                continue;
            }
            in_params = false;
            let (key, val) = match line.split_once(':') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => continue,
            };
            match key {
                "handler" => handler = Some(val.to_string()),
                "version" => version = Some(val.to_string()),
                "kind" => {
                    kind = match val {
                        "sql" => Kind::Sql,
                        "edge" => Kind::Edge,
                        other => return Err(bad(format!("bad kind: {other}"))),
                    }
                }
                "invocation" => {
                    invocation = match val {
                        "effect-async" => Invocation::EffectAsync,
                        "validator-sync" => Invocation::ValidatorSync,
                        other => return Err(bad(format!("bad invocation: {other}"))),
                    }
                }
                "timeout_ms" => timeout_ms = val.parse().ok(),
                "fail_policy" => {
                    fail_policy = match val {
                        "fail-open" => FailPolicy::FailOpen,
                        _ => FailPolicy::FailClosed,
                    }
                }
                "idempotency_key" => {
                    let v = val.trim_matches('"');
                    if !v.is_empty() && v != "null" {
                        idempotency_key = Some(v.to_string());
                    }
                }
                "returns" => returns = val.to_string(),
                _ => {}
            }
        }

        let contract = Contract {
            handler: handler.ok_or_else(|| bad("contract missing `handler`"))?,
            version: version.ok_or_else(|| bad("contract missing `version`"))?,
            kind,
            invocation,
            timeout_ms,
            fail_policy,
            idempotency_key,
            params,
            returns,
        };
        contract.validate()?;
        Ok(contract)
    }

    /// Contract-level validation enforcing the mandatory-field lint rules (H5 / §4.4).
    pub fn validate(&self) -> Result<()> {
        // H5: idempotency_key MANDATORY for effect-async edge.
        if self.kind == Kind::Edge
            && self.invocation == Invocation::EffectAsync
            && self.idempotency_key.as_deref().unwrap_or("").is_empty()
        {
            return Err(bad(format!(
                "handler {}: effect-async edge REQUIRES a non-empty idempotency_key (H5)",
                self.handler
            )));
        }
        // timeout_ms MANDATORY for validator-sync edge (§4.1).
        if self.kind == Kind::Edge
            && self.invocation == Invocation::ValidatorSync
            && self.timeout_ms.is_none()
        {
            return Err(bad(format!(
                "handler {}: validator-sync edge REQUIRES timeout_ms",
                self.handler
            )));
        }
        Ok(())
    }

    /// The stored contract jsonb (design §7.1 — one source of truth in the registry).
    pub fn to_jsonb(&self) -> Value {
        json!({
            "handler": self.handler,
            "version": self.version,
            "kind": match self.kind { Kind::Sql => "sql", Kind::Edge => "edge" },
            "invocation": match self.invocation {
                Invocation::EffectAsync => "effect-async",
                Invocation::ValidatorSync => "validator-sync",
            },
            "blocking": self.blocking(),
            "timeout_ms": self.timeout_ms,
            "fail_policy": match self.fail_policy {
                FailPolicy::FailClosed => "fail-closed",
                FailPolicy::FailOpen => "fail-open",
            },
            "idempotency_key": self.idempotency_key,
            "params": Value::Object(self.params.iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect()),
            "returns": self.returns,
        })
    }
}

fn bad(msg: impl Into<String>) -> SubstrateError {
    SubstrateError::Db(msg.into())
}

/// Whether ANY handler's checked-in generated SQL wrapper differs from a fresh codegen
/// (the `db handler codegen --check` gate, wired into the promote invariant gate §9.3,
/// finding 1). Returns `true` if a stale (or missing) generated wrapper is found — a stale
/// wrapper MUST block promote.
///
/// Scans every `<handlers_dir>/<name>/v1.yaml` contract, regenerates its wrapper, and
/// compares byte-for-byte against `<generated_sql_dir>/call_<name>.sql`.
pub fn codegen_is_stale(handlers_dir: &Path, generated_sql_dir: &Path) -> Result<bool> {
    let entries = match std::fs::read_dir(handlers_dir) {
        Ok(e) => e,
        // No handlers dir → nothing to generate → not stale.
        Err(_) => return Ok(false),
    };
    for e in entries.flatten() {
        if !e.path().is_dir() {
            continue;
        }
        let name = e.file_name().to_string_lossy().into_owned();
        // v1 codegen keys off the v1 contract (the CLI's codegen does the same).
        let contract = match load_contract(handlers_dir, &name, "v1") {
            Ok(c) => c,
            Err(_) => continue, // not a handler contract dir
        };
        let gen = codegen(&contract);
        let out = generated_sql_dir.join(format!("call_{name}.sql"));
        let existing = std::fs::read_to_string(&out).unwrap_or_default();
        if existing != gen.sql_wrapper {
            return Ok(true); // stale (or missing) generated wrapper → block promote.
        }
    }
    Ok(false)
}

/// Load a contract from `db/handlers/<name>/<vN>.yaml`.
pub fn load_contract(handlers_dir: &Path, name: &str, version: &str) -> Result<Contract> {
    let path = handlers_dir.join(name).join(format!("{version}.yaml"));
    let text = std::fs::read_to_string(&path)
        .map_err(|e| SubstrateError::Db(format!("reading {}: {e}", path.display())))?;
    Contract::parse(&text)
}

/// Deterministic codegen output for a contract (design §7.1).
#[derive(Debug, Clone)]
pub struct Generated {
    /// Typed SQL wrapper `call_<handler>(...)` funneling through `call_handler`.
    pub sql_wrapper: String,
    /// Typed TS edge signature + runtime guard (empty for pure-sql handlers).
    pub ts_guard: String,
    /// For validator-sync: the trigger body + return-side guard (M4). Empty otherwise.
    pub trigger_sql: String,
    /// The stored contract jsonb.
    pub contract_jsonb: Value,
}

/// Generate all codegen artifacts for a contract, deterministically (design §7.1, M4,
/// H1). Output is fenced with `-- @codegen:begin/end` so lint exempts it (§9.5).
pub fn codegen(contract: &Contract) -> Generated {
    let sql_wrapper = gen_sql_wrapper(contract);
    let ts_guard = if contract.kind == Kind::Edge {
        gen_ts_guard(contract)
    } else {
        String::new()
    };
    let trigger_sql = if contract.invocation == Invocation::ValidatorSync {
        gen_trigger(contract)
    } else {
        String::new()
    };
    Generated {
        sql_wrapper,
        ts_guard,
        trigger_sql,
        contract_jsonb: contract.to_jsonb(),
    }
}

/// pg → SQL type mapping for wrapper args (v1 passes contract types through verbatim).
fn gen_sql_wrapper(c: &Contract) -> String {
    let args: Vec<String> = c
        .params
        .iter()
        .map(|(name, ty)| format!("{name} {ty}"))
        .collect();
    let jsonb_build: Vec<String> = c
        .params
        .keys()
        .map(|name| format!("'{name}', {name}"))
        .collect();
    format!(
        "-- @codegen:begin call_{handler}\n\
         CREATE OR REPLACE FUNCTION call_{handler}({args}) RETURNS {ret}\n\
         LANGUAGE sql AS $$\n\
         \x20 SELECT ops.call_handler('{handler}', jsonb_build_object({obj}))\n\
         $$;\n\
         -- @codegen:end\n",
        handler = c.handler,
        args = args.join(", "),
        ret = pg_return_type(&c.returns),
        obj = jsonb_build.join(", "),
    )
}

fn pg_return_type(returns: &str) -> &str {
    match returns {
        "validator_result" => "jsonb",
        "void" => "void",
        other => other,
    }
}

/// TS edge signature + runtime guard (valibot/zod-style) from the same contract.
fn gen_ts_guard(c: &Contract) -> String {
    let fields: Vec<String> = c
        .params
        .iter()
        .map(|(name, ty)| format!("  {name}: {}", ts_type(ty)))
        .collect();
    format!(
        "// @codegen:begin {handler}_{version}\n\
         export interface {Handler}Params {{\n{fields}\n}}\n\
         export function assert{Handler}(p: unknown): {Handler}Params {{\n\
         \x20 const o = p as {Handler}Params;\n\
         {checks}\
         \x20 return o;\n\
         }}\n\
         // @codegen:end\n",
        handler = c.handler,
        version = c.version,
        Handler = pascal(&c.handler),
        fields = fields.join(",\n"),
        checks = c
            .params
            .keys()
            .map(|n| format!("  if (o.{n} === undefined) throw new Error('missing {n}');\n"))
            .collect::<String>(),
    )
}

fn ts_type(pg: &str) -> &str {
    match pg {
        "uuid" | "text" | "varchar" => "string",
        "int" | "int4" | "int8" | "bigint" | "numeric" | "float8" => "number",
        "bool" | "boolean" => "boolean",
        "jsonb" | "json" => "unknown",
        _ => "unknown",
    }
}

/// The validator-sync trigger body + return-side guard (design §8.6, M4). This is a
/// template; the target table is not known from the contract alone, so the
/// `assert_row_shape` call is emitted parameterized and the operator wires the table in
/// the migration (`CREATE TRIGGER ... ON <table>`).
fn gen_trigger(c: &Contract) -> String {
    format!(
        "-- @codegen:begin trg_{handler}\n\
         CREATE OR REPLACE FUNCTION trg_{handler}() RETURNS trigger LANGUAGE plpgsql AS $$\n\
         DECLARE r jsonb;\n\
         BEGIN\n\
         \x20 r := call_{handler}(NEW);\n\
         \x20 IF r->>'decision' = 'deny' THEN\n\
         \x20   RAISE EXCEPTION 'validator % denied: %', '{handler}', r->>'reason'\n\
         \x20     USING ERRCODE='check_violation', DETAIL = r->>'code';\n\
         \x20 END IF;\n\
         \x20 IF r ? 'row' THEN\n\
         \x20   -- M4 return-side guard: validate keys+types before populating NEW.\n\
         \x20   PERFORM ops.assert_row_shape(TG_TABLE_SCHEMA, TG_TABLE_NAME, r->'row');\n\
         \x20   NEW := jsonb_populate_record(NEW, r->'row');\n\
         \x20 END IF;\n\
         \x20 RETURN NEW;\n\
         END $$;\n\
         -- @codegen:end\n",
        handler = c.handler,
    )
}

/// The edge `test_down.sql` codegen (H1) — asserts the pointer reverted to the prior
/// version AND dispatch resolves to the prior `_vN` (design §7.6).
pub fn gen_edge_test_down(handler: &str, env: &str, prior_version: &str) -> String {
    format!(
        "-- @codegen:begin test_down_{handler}\n\
         DO $$\n\
         DECLARE v text;\n\
         BEGIN\n\
         \x20 SELECT hv.version INTO v FROM ops.handler_active ha\n\
         \x20   JOIN ops.handler_versions hv ON hv.id = ha.version_id\n\
         \x20  WHERE ha.handler = '{handler}' AND ha.env = '{env}';\n\
         \x20 IF v IS DISTINCT FROM '{prior}' THEN\n\
         \x20   RAISE EXCEPTION 'pointer not reverted: active=% expected=%', v, '{prior}';\n\
         \x20 END IF;\n\
         END $$;\n\
         -- @codegen:end\n",
        handler = handler,
        env = env,
        prior = prior_version,
    )
}

fn pascal(s: &str) -> String {
    s.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect()
}
