# Contract: db-control-plane

## Parties
`db` ↔ consumers (operators / CI; `org`'s self-restructuring KG as ordinary
migrations+queries; any service that drives the DB control plane). Reached
**two boring ways — a `bin/db` CLI subprocess (standalone, mesh-free) OR the
`db serve` daemon over WS — NEVER by linking `substrate-db`** (INTENT #29).
Authored from `db.md` (authoritative; owns `lib/db`/`bin/db`).

> **SUPERSEDED (re-spoken round, 2026-07-21/22, INTENT #167): the `db serve`
> WS bearer is GONE — db is a pure CLI tool with no daemon; sessions live
> in vdb.** The session-home call was delegated to AUI ("just pick one")
> and picked for simplicity: vdb already owned statement tracking/cleanup/
> session management (INTENT #128), so the sessions and warm connections
> move into the vdb daemon, and this contract's ONE remaining bearer is the
> **`bin/db` CLI subprocess** (`--format json`), standalone and mesh-free —
> which honors the operator's original instinct that daemons are services
> and db is a tool (INTENT #115). The noun-verb vocabulary below is
> unchanged; read "WS surface" as historical. See `db.md` header note,
> `vdb.md` concern 7, `vdb-db.md`.

> **[Previously: the `db serve` WS bearer ACCEPTED — friction-round 3,
> INTENT #128, resolving the INTENT #115 drift flag].** The operator accepts "a session
> concept with a headless stateful daemon running as part of db"; VDB is the
> virtualization layer that delegates to db (so the same tools aren't
> defined twice) and owns statement tracking / cleanup / session management
> on its side (see `vdb-db`). The CLI-subprocess half stands unchanged (the
> boot-safe, mesh-free access path; mesh may use db this way for its own
> database — direct CLI execution, no daemon, no mesh dependency).

**REWRITES the stale stub.** The wave-1 stub framed this edge as the
"DB thing in a local stack for the Org/game demo" with a "programmatic access"
note that read as a library-dependency edge. That framing is **superseded** by
INTENT #29 (no in-process linking across app boundaries, ever) and INTENT #96
("extend db as necessary to support VDB"): the surface is a wire surface, and
the query facet is the standardized virtualization layer, not a demo utility.

## Purpose
The full **noun-verb control plane** over a `Capabilities`-gated `Driver` trait
spanning multiple backends (sqlite / supabase-cloud; `aws-rds` design-only):
migrate (author/apply/rollback/status/crawl/lint), edge
(bundle/deploy/activate/rollback/sync), **query — the standardized
virtualization surface (INTENT #60)**, seed/fakedata, outbox
(list/retry/drain), audit, handler (new/codegen/activate/rollback), promote,
snapshot, provenance read, doctor. One `ops` schema (migration ledger +
handler registry + outbox + audit + provenance) is the sole source of truth.
The CLI shape (the real `clap` tree in `bin/db/src/cli.rs`) is authoritative;
the WS surface is the same verbs as a request/response envelope.

## Schema
Structs land in `types::db`; `DbError` in `types::error::db`. The
`Driver`/`Capabilities` traits are an internal `lib/db` boundary, NOT a
contract.

```rust
enum DbCtlReq {
    Migrate(MigrateReq),      // Author | Apply | Rollback | Status | Crawl | Lint
    Edge(EdgeReq),            // Bundle | Deploy | Activate | Rollback | Sync   (Supabase target)
    Query(QueryReq),          // the standardized virtualization surface (INTENT #60)
    Seed(SeedReq),
    Outbox(OutboxReq),        // List | Retry | Drain
    Audit(AuditReq),
    Handler(HandlerReq),      // New | Codegen | Activate | Rollback
    Promote(PromoteReq),      // forward-only copy/verify/switch gate
    Snapshot(SnapshotReq),
    Provenance(ProvReq),      // read the ops.provenance ledger
    Doctor,
    #[serde(other)] Unknown,
}

// the query/virtualization facet — a caller names a DATABASE, never a backend
struct QueryReq {
    database: String,             // db.toml env name (standalone) or "<project>/<db>" (daemon)
    sql: String,
    params: Vec<SqlParam>,
    write: bool,                  // the read/write gate; a write without write:true is refused
}                                 // -> Rows { columns, rows }   (v1 string cells; typed cells EXT)

struct MigrateReq  { database: String, verb: MigrateVerb, i_understand_prod: Option<String> }
enum   MigrateVerb { Author { name: String }, Apply, Rollback { to: String }, Status, Crawl, Lint }

struct Rows { columns: Vec<String>, rows: Vec<Vec<Option<String>>> }
```

**Structured output:** the CLI form takes `--format json` and returns the same
typed shapes (`Rows`, `MigrateReport`, `DrainReport`, …); the WS form returns
them directly. `org`'s metacognitive KG (INTENT #19) is ordinary migrations +
queries against `org`-owned schema over this edge — `db` does not model the
graph; `org`/`kg` do.

## Error cases
- `DbError::ProtectedRefused { op, env }` — a mutating migrate on a protected
  ref without the exact `--i-understand-prod <ref>` token.
- `DbError::PromoteGateBlocked { blockers }` — lint-not-clean /
  codegen-stale / crawl-attestation-mismatch / wrong promote target.
- `DbError::LintNotClean(Vec<String>)`.
- `DbError::NotImplemented { command, driver, reason }` — incapable backend
  degrades **early and typed** (e.g. `edge deploy` on `sqlite`).
- `DbError::NoSuchDatabase { id }`, `DbError::Config(String)`.
- A write `Query` without `write: true` is refused (the read/write gate).

## Version sensitivity
LOW-MEDIUM. The noun-verb surface is stable and grows **additively**; every
enum reserves `#[serde(other)]`; the WS envelope versions via the standard
`pubsub-protocol` `v` field. The `Rows` string-cell rendering (the
Postgres-text-protocol shape) is **frozen for v1**; structured typed cells are
an additive EXT. The `ops`-schema ledger shape is `db`'s on-disk truth and
effectively frozen.

## Reconciliation notes
- **Only `db` proposed this edge; no cross-party disagreement.** Consumers
  (operators, CI, `org`) do not author a counter-shape — they consume the
  noun-verb surface.
- **Deviation from the stub (the required rewrite):** the wave-1 stub's
  "library dependency / programmatic access" framing is replaced with the
  **wire-only** access model (CLI subprocess OR `db serve` WS), per INTENT #29.
  The stub's genuine content — a stable noun-verb DB surface over
  Postgres/SQLite drivers with query/seed/outbox/audit — is fully carried
  forward; only the linkage framing changed.
- **Net-new this wave:** the **query facet as the INTENT #60 virtualization
  surface** (a caller names a `database`, `db` resolves handle → driver → runs;
  the caller never names a backend) and the **`db serve` daemon** as the WS
  bearer. `secrets.md` and `vdb.md` already assumed a "`db-control-plane` WS
  call," so the direction is cross-consistent.
- **Shared-vocabulary flag:** the `database` addressing string doubles as a
  `db.toml` env name (standalone CLI) and a `"<project>/<db>"` slug (daemon);
  the harmonizer should unify against `types::db::DatabaseId` (see `vdb-db`).

## Example data
World: nodes **macbook** and **pi**; project **demo**; database **demo/main**.

**1. A standardized virtualization query** (an operator, or `org`, asks
`demo/main` a question without naming a backend — SQLite here, but the caller
cannot tell):

```jsonc
// DbCtlReq::Query   (db serve WS, or `db --env demo/main query … --format json`)
{ "Query": {
    "database": "demo/main",
    "sql": "SELECT order_id, cents FROM invoices WHERE cents > ?1 ORDER BY cents DESC",
    "params": [ { "Int": 1000 } ],
    "write": false } }
// -> Rows
{ "columns": ["order_id", "cents"],
  "rows": [ ["ord-1042", "4200"], ["ord-1039", "1500"] ] }
```

**2. Apply a migration** (CLI subprocess form, boot-safe / operator-driven):

```jsonc
// $ db --env demo/main migrate up --format json   -> MigrateReport
{ "applied": ["0007_add_invoices"], "already_current": false,
  "ledger_head": "0007_add_invoices" }
```

**3. The prod guard fires** (a mutating migrate against a protected ref):

```jsonc
// DbCtlReq::Migrate { database: "demo/main@prod", verb: Apply, i_understand_prod: None }
// -> DbError::ProtectedRefused { op: "migrate apply", env: "demo/main@prod" }
```
