# Contract: vdb-db

## Parties
`vdb` daemon (`bin/vdb` on a database-hosting node) → `db` daemon (`db serve`
on the SAME node). Cross-app, **WS over the local mesh daemon `:3649`, never
linked** (INTENT #29 — the rule the operator coined against `db` specifically).
Co-located by a supervision boot-order fact: `vdb` requires a local `db`
(`meta.requires: ["db"]`), so the hot path is `vdb → local mesh → local db →
SQLite file` — one local relay hop, no cross-node traffic for local work.
Authored from `db.md` (authoritative for the `db` surface — owns `lib/db`'s
real `Driver` trait) and `vdb.md` (authoritative for the caller shape — the
Fable L4 stack daemon), co-designed in batch 4.

## Purpose
The **execution-arm hot path**: the session protocol by which the `vdb` daemon
runs every engine-level action against a specific stack database — apply
SQL/DDL, run migrations, deploy/activate handlers per target, install + drain
**change-capture**, **structured introspection**, atomic **provenance** write,
and the **promotion primitives** (dump/restore/hash/snapshot). `vdb` *tracks
and decides* (stack pattern, trigger matching, promotion orchestration); `db`
*runs and records* (SQL/DDL/edge execution against a resolved backend, with
atomic provenance). This is the reason `db serve` exists: it holds **warm
driver connections** keyed by session, so `vdb`'s handler hot path (an action
per row change) never pays process-spawn + connect + config-parse per action.

## Schema
Structs land in `types::db` (the DB surface) and `types::vdb` (`VdbError`);
`Provenance` is `types::provenance::Provenance` (frozen triple). The `db serve`
daemon exposes `lib/db`'s real `Driver`/`Capabilities` surface — an **internal
`lib/db` boundary, not itself a contract** — over these WS frames.

```rust
// ── canonical identity ────────────────────────────────────────────────
// RECONCILED: db.md's `DatabaseId { project, db }` is the canonical structured
// type in types::db; vdb.md's `DbId(String)` is its "<project>/<db>" slug
// rendering (used as the registry-slug segment). Both denote the same database.
pub struct DatabaseId { pub project: String, pub db: String }   // to_string() = "<project>/<db>"

// ── session lifecycle (vdb -> db) ─────────────────────────────────────
struct OpenSession  { target: DbDriverTarget } // -> OpenSessionAck { session_id: Uuid }
struct CloseSession { session_id: Uuid }
enum DbDriverTarget {                          // vdb SUPPLIES the resolved handle — see Reconciliation
    Sqlite        { os_path: String, database: DatabaseId },  // vdb+vfs materialized the file locally
    SupabaseCloud { project_ref: String, database: DatabaseId },
    AwsRds        { target_id: String, database: DatabaseId }, // design-only v1
    #[serde(other)] Unknown,
}

// ── the existing Driver surface, exposed (no new db behavior) ─────────
struct ApplySql    { session_id: Uuid, sql: String, prov: Provenance }              // mutating -> prov REQUIRED
struct ApplyParams { session_id: Uuid, sql: String, params: Vec<SqlParam>, prov: Provenance }
struct Query       { session_id: Uuid, sql: String, params: Vec<SqlParam>, write: bool } // -> Rows; write-gate
struct Introspect  { session_id: Uuid, q: IntrospectQuery }  // -> Introspection | TableSchema (structured, concern 2)
struct ApplyAtomic { session_id: Uuid, lock_key: String, statements: Vec<String>,
                     ledger: AppliedMigration, prov: Provenance }                    // migrations, xact-locked
struct EdgeDeploy  { session_id: Uuid, bundle: BundleRef }   // Supabase target only; NotImplemented on sqlite
struct OutboxDrain { session_id: Uuid } // -> DrainReport

// ── change-capture: the SQLite procedural-trigger equivalent (concern 2) ──
struct InstallChangelog { session_id: Uuid, tables: Vec<String>, events: Vec<ChangeOp> }
                        // (re)generate AFTER INSERT/UPDATE/DELETE triggers writing _vdb_changelog
struct ReadChanges { session_id: Uuid, since_seq: i64, limit: u32 } // -> Vec<ChangeRow>
struct AckChanges  { session_id: Uuid, seqs: Vec<i64> }

// ── promotion primitives (vdb orchestrates; db provides) ──────────────
struct Snapshot    { session_id: Uuid, out: String } // -> { content_hash: Hash }  (catastrophic snapshot)
struct DumpTo      { session_id: Uuid, format: DumpFormat } // -> Stream<Bytes>     (bulk copy source)
struct RestoreFrom { session_id: Uuid, format: DumpFormat, body: Stream<Bytes> }
struct ContentHash { session_id: Uuid, table: String } // -> Hash                   (per-table verify)

// ── provenance read (atomic write happens inside ApplySql/ApplyAtomic) ──
struct ReadProvenance { session_id: Uuid, filter: ProvFilter } // -> Vec<ProvenanceRow>

// value shapes (types::db)
struct Rows        { columns: Vec<String>, rows: Vec<Vec<Option<String>>> } // v1 string cells; typed cells EXT
struct ChangeRow   { seq: i64, table: String, op: ChangeOp, pk: Value,
                     before: Option<Value>, after: Option<Value>, at: DateTime<Utc>, provenance: Provenance }
struct ProvenanceRow { seq: i64, table: String, op: ChangeOp, correlation_id: Option<Uuid>,
                       causation_id: Option<Uuid>, actor_service: String, handler: Option<String>,
                       at: DateTime<Utc>, summary: String }
enum ChangeOp { Insert, Update, Delete }
enum DumpFormat { SqliteFile, Sql, Jsonl, #[serde(other)] Unknown }
```

**The provenance-atomicity rule (load-bearing, `db.md` concern 3):** every
mutating verb writes its `ProvenanceRow` into the `ops.provenance` /
`ops_provenance` ledger **inside the same `apply_atomic` transaction as the
mutation**. A crash between the write and its trace is impossible by
construction — the reason provenance lives in `db`, not only in vdb's tracker.

## Error cases
`db`'s errors are `DbError`; `vdb` **wraps them at its boundary into
`VdbError::ExecutionArm(String)`** — one crate's error enum never becomes
another's (types.md guardrail).

- `DbError::NotImplemented { command, driver, reason }` — e.g. `EdgeDeploy` on
  the `sqlite` driver (the TS handler is a vdb-hosted Deno handler, NOT a
  DB-side edge function; vdb's capability gate should prevent reaching this —
  hitting it is a vdb bug, logged loudly).
- `SessionNotFound` / `SessionBusy` — unknown or serialized session.
- `LedgerConflict` — `ApplyAtomic` lost its advisory-lock/ledger race.
- `DbError::MigrationFailed { id, detail }`.
- `DbError::ProtectedRefused { op, env }` — the prod guard; a write `Query`
  without `write: true` is refused here (the read/write gate, INTENT #60).
- `DbError::Backend(String)` — driver/SQL error, stringified at the boundary.
- Distributed single-writer / partition coordination surfaces as a `locks`
  error when vdb asked for it — carried by `locks-api`, **not re-typed here**.

## Version sensitivity
MEDIUM — a **node-local** edge (no cross-node version skew), but the session
protocol is `db`'s second public surface, so full types.md wire discipline:

- **Additive-safe:** new verbs; new `#[serde(default)]` fields; new
  `DbDriverTarget`/`DumpFormat`/`ChangeOp` variants (each enum reserves
  `#[serde(other)]`); typed `TableSchema` cells and typed `Rows` cells are the
  named EXT growth path (additive over the frozen v1 string-cell rendering).
- **Breaking:** the `AppliedMigration`/ledger shape is `db`'s existing on-disk
  truth and therefore **effectively frozen**; changing the provenance-atomicity
  transaction boundary or the `_vdb_changelog` row shape is breaking. The WS
  envelope versions via the standard `pubsub-protocol` `v` field.

## Reconciliation notes
1. **Session-oriented (vdb) vs `DatabaseId`-stateless (db) addressing — vdb's
   session model WINS.** `db.md` proposed a stateless `VdbDbReq` carrying
   `db: DatabaseId` per message, with the daemon holding warm handles keyed by
   database identity. But `db.md` concern 6 also states **`db` never talks to
   VFS** — VDB (with VFS) materializes the SQLite file locally and hands `db` a
   real OS path; `db` cannot resolve `DatabaseId → os_path` itself (VFS
   residency is vdb's knowledge). Therefore vdb must SUPPLY the path, which is
   exactly `OpenSession { target: DbDriverTarget::Sqlite { os_path, .. } }`.
   `db.md`'s warm-handle rationale is preserved intact: **`session_id` keys the
   warm driver connection** (open once, reuse across the handler hot path).
   `DatabaseId` still rides every target for logging/routing/registry-slug
   derivation.
2. **Provenance threading + the read/write gate (db.md, must-have) preserved
   over vdb's plainer verbs.** `vdb.md`'s sketch (`ApplySql { sql }`) omitted
   `prov` and the `write` gate; `db.md` concern 3 makes provenance
   **non-optional** on every mutating action and INTENT #60 makes the write
   gate the safety spine. This contract carries `prov: Provenance` on all
   mutating verbs and `write: bool` on `Query`.
3. **Verify primitive: vdb's `ContentHash` WINS over db's `VerifyParity`.**
   `db.md` proposed `VerifyParity { src, dst }` — but src (local) and dst
   (cloud) are two DIFFERENT sessions/connections; a single `db` call cannot
   see both. So verification is **vdb-orchestrated**: `ContentHash` per table
   per session, compared by vdb across its two sessions. `VerifyParity` is
   vdb's `verify_against` orchestration (concern 9), not a `db` verb.
4. **Verb-name reconciliation:** `db.md`'s `install_change_capture` /
   `InstallChangeCapture` and `vdb.md`'s `InstallChangelog` name one thing;
   adopted **`InstallChangelog`** (the target table is `_vdb_changelog`). vdb
   tails the changelog by its own persisted cursor (`_vdb_meta.processed_seq`,
   vdb.md concern 4) and MAY also drain via the typed `ReadChanges`/`AckChanges`
   convenience above — both are supported; the cursor is the source of truth.
5. **Type unification flagged to the harmonizer:** `types::db::DatabaseId
   { project, db }` (structured) is canonical; `types::vdb::DbId(String)` is
   its `"<project>/<db>"` slug rendering. The harmonizer should collapse these
   to one type + a `Display`/`FromStr` pair rather than two structs.
6. **Deviation from the stale stub:** there was no `vdb-db` stub — this is a
   NEW pair (wave2-plan §3b). db.md's earlier note that "org/inference consume
   `lib/db` as a Cargo dependency" is contradicted by INTENT #29 (which
   post-dates it); this contract assumes **wire-only** consumption fleet-wide.

## Example data
World: nodes **macbook** and **pi**; project **demo**; the stack database
**demo/main** is anchored on **macbook** (a `LocalSqlite` target). vdb hosts
`demo/main`'s handler loop; every handler write and every migration crosses
this edge to the co-located `db serve`.

**1. Open a warm session** (vdb boots the `demo/main` entity, gets a path from
vfs via `vdb-vfs`, opens the driver once):

```jsonc
// OpenSession  vdb -> db   (mesh WS, slug "db", node-local)
{ "target": { "Sqlite": {
    "os_path": "/Users/sam/Library/mind/vfs/vdb/demo/main.sqlite",
    "database": { "project": "demo", "db": "main" } } } }
// OpenSessionAck  db -> vdb
{ "session_id": "9b1e…-s1" }
```

**2. Install change-capture + a provenance-atomic handler write.** A row lands
in `demo.orders`; vdb's trigger fires a Deno handler that writes `demo.invoices`
— the write and its provenance row commit together:

```jsonc
// InstallChangelog  vdb -> db   (once, at apply_definition time)
{ "session_id": "9b1e…-s1", "tables": ["orders","invoices"],
  "events": ["Insert","Update","Delete"] }

// ApplyParams  vdb -> db   (the handler's write, causation-stamped)
{ "session_id": "9b1e…-s1",
  "sql": "INSERT INTO invoices(order_id, cents) VALUES (?1, ?2)",
  "params": [ { "Text": "ord-1042" }, { "Int": 4200 } ],
  "prov": { "origin_node": "macbook", "origin_service": "vdb",
            "correlation_id": "corr-77", "causation_id": "inv-77-h1",
            "emitted_at": 1752969600500, "hops": 2 } }
// -> the invoices INSERT and its ops_provenance row commit in ONE transaction
```

**3. Drain the changelog** (vdb's tailer, for the next dispatch cycle):

```jsonc
// ReadChanges  vdb -> db
{ "session_id": "9b1e…-s1", "since_seq": 118, "limit": 500 }
// -> Vec<ChangeRow>  db -> vdb
[ { "seq": 119, "table": "invoices", "op": "Insert", "pk": { "id": 88 },
    "before": null, "after": { "order_id": "ord-1042", "cents": 4200 },
    "at": 1752969600500,
    "provenance": { "origin_node": "macbook", "origin_service": "vdb",
                    "correlation_id": "corr-77", "causation_id": "inv-77-h1",
                    "emitted_at": 1752969600500, "hops": 2 } } ]
// AckChanges  vdb -> db   (after dispatch outcomes recorded)
{ "session_id": "9b1e…-s1", "seqs": [119] }
```

**4. Promotion verify leg** (vdb comparing local `demo/main` to a fresh cloud
`demo/analytics` target during copy/verify — two sessions, vdb compares):

```jsonc
// ContentHash  vdb -> db (local session s1)
{ "session_id": "9b1e…-s1", "table": "orders" }        // -> "sha256:aa11…"
// ContentHash  vdb -> db (cloud session s2, SupabaseCloud target)
{ "session_id": "9b1e…-s2", "table": "orders" }        // -> "sha256:aa11…"
// vdb: hashes match -> table parity confirmed; mismatch -> PromotionConflict, abort
```
