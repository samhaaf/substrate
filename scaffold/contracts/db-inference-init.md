# Contract: db-inference-init

## Parties
`inference` → `db`. A **`bin/db` CLI subprocess** call (`db --env <node>
migrate up`), **NOT** a linked-lib call and **not** necessarily a daemon WS
call. Authored from `db.md` and `inference.md` (both batch parties propose a
consistent shape — see Reconciliation notes).

**REWRITES the stale stub.** The wave-1 stub declared this a "**library
dependency edge** — `inference` depends on `substrate-db` and calls into it."
That is directly reversed by **INTENT #29** (no in-process linking across app
boundaries): a top-level app is never imported. The corrected shape is a
subprocess, which is also the **boot-safe** path (below).

## Purpose
When an `inference` node stands up on a **fresh mesh node**, it initializes its
**control-plane `ops` database** through `db` rather than hand-rolling
bootstrap: the **`sqlite` driver, ledger-only baseline** (`OPS_BASELINE_SQLITE`)
+ `migration::apply`. A narrow, stable slice — no edge functions, no local
stack, no promote (those `Capabilities` are already `false` for the `sqlite`
driver and need no new gating).

## Schema
The subprocess CLI is authoritative; structured output via `--format json`
(`types::db`).

```rust
// inference invokes at Step 0 (pre-store, boot-safe):
//   db --env <node_id> migrate up --format json   ->  MigrateReport
struct MigrateReport {
    applied:         Vec<String>,   // migration ids applied this run (empty on a warm node)
    already_current: bool,          // true == idempotent no-op
    ledger_head:     String,        // the ops-schema ledger head after apply
}
```

**Boot-safe rationale:** on a cold node, mesh may not yet relay and the `db
serve` daemon may not be up. A `bin/db` subprocess needs **no mesh** — so it is
the reliable bootstrap path. Once the node is warm, later control-plane access
can move to the `db serve` daemon over WS (`db-control-plane`) — though note
the `db serve` daemon mode itself is FROZEN pending the INTENT #115 drift
discussion; this contract's CLI-subprocess path is unaffected (it is the
blessed standalone-tool access model).

## Error cases
- `DbError::MigrationFailed { id, detail }` on a failed first-boot apply →
  `inference` **degrades to standalone** (its `store` system-of-record
  `substrate.db` alone; `inference.md` concern 5), never crash-loops.
- **Missing `db` binary** → `inference` warns and continues standalone.
- A **second invocation** against an already-migrated node is an **idempotent
  no-op** (the ledger dedups; `already_current: true`) — the conformance
  requirement.

## Version sensitivity
LOW. This edge exercises the **narrowest, most stable slice** of `db` (the
sqlite baseline + `migration::apply`); the CLI contract is stable and
additive-only. `MigrateReport` grows via `#[serde(default)]` fields.

## Reconciliation notes
- **Both parties converge; no disagreement.** `db.md` (concern 1a, the
  boot-safe subprocess) and `inference.md` (concern 5 / its `db-inference-init`
  proposal, "Step 0, pre-store, boot-safe") independently specify the SAME
  shape — `bin/db` subprocess, sqlite ledger-only baseline, idempotent, degrade
  to standalone on failure. This file adopts it verbatim.
- **Deviation from the stub (required rewrite):** the "library dependency edge"
  wording is replaced by the subprocess model per INTENT #29. The stub's
  conformance preview ("a fresh node reaches a migrated/ready state without
  manual intervention; a second invocation is a safe no-op") is retained
  verbatim as the conformance bar above.
- **Distinctness from `store`, flagged for the harmonizer (both parties agree):**
  this edge bootstraps the **control-plane `ops`** database; `inference`'s
  `store` is a **separate** system-of-record (`substrate.db`) that
  **self-migrates at `Store::open`** with zero external dependency (store.md).
  The two never collide — `db-inference-init` does not touch `store`'s schema.

## Example data
World: nodes **macbook** and **pi**; project **demo**; model **qwen3-4b**.
The always-on inference leg for `qwen3-4b` is brought up on a **fresh `pi`**.

**1. Cold boot — the ops database does not exist yet:**

```jsonc
// $ db --env pi migrate up --format json         (subprocess, no mesh needed)
// -> MigrateReport
{ "applied": ["0001_ops_baseline"], "already_current": false,
  "ledger_head": "0001_ops_baseline" }
// inference proceeds to Step 1+ (store open, engine, mesh registration)
```

**2. Warm reboot — idempotent no-op:**

```jsonc
// $ db --env pi migrate up --format json
// -> MigrateReport
{ "applied": [], "already_current": true, "ledger_head": "0001_ops_baseline" }
```

**3. Failure — degrade to standalone:**

```jsonc
// -> DbError::MigrationFailed { id: "0001_ops_baseline",
//      detail: "disk I/O error opening /Users/…/mind/ops/pi.sqlite" }
// inference logs, runs on store's substrate.db alone; qwen3-4b still serves locally.
```
