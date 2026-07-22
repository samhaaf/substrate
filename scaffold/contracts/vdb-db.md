# Contract: vdb-db

> **⚠ SESSION HOME DECIDED — `db serve` SUPERSEDED (re-spoken round,
> 2026-07-21/22, INTENT #167: the operator delegated the db-vs-vdb
> session-home call — "just pick one").** The pick, for simplicity:
> **sessions live in VDB; db returns to a pure CLI tool with no daemon.**
> Rationale in one paragraph: INTENT #128 had already made vdb the owner of
> statement tracking, cleanup, and session management — leaving the session
> daemon in db split one concern across two co-located processes and cost
> two local WS hops per handler action; collapsing sessions (and the warm
> driver connections they key) INTO the vdb daemon gives the concept one
> home, returns db to the operator's original instinct (INTENT #115: "db =
> a standalone crate you call as a tool; VDB = the mesh-accessed service"
> — daemons are services, db is a tool), and deletes a public surface
> instead of adding one. **This contract is no longer a cross-app wire
> protocol.** Its session-verb vocabulary (OpenSession/ApplySql/
> InstallChangelog/promotion primitives/provenance-atomicity, below)
> survives as the shape of vdb's INTERNAL session surface; db's side of
> the reconciliations (provenance threading, the write gate, `ContentHash`)
> survives as CLI/lib capability requirements. vdb reaches db as a CLI
> subprocess for cold-path actions (db-control-plane path (a)); the
> warm-path driver mechanism is fill-time latitude recorded in vdb.md
> concern 7. Retained below as record. → `db.md` header note, `vdb.md`
> concern 7.

> **[Previously UNFROZEN — ACCEPTED (friction-round 3, 2026-07-20, INTENT
> #128; resolves the friction-round-1 INTENT #115 drift flag).]** The operator
> accepts the `db serve` daemon mode this contract rides, verbatim: "Now
> that you mention there are drivers for sessions that need to run, it
> actually does make sense to have a session concept with a headless
> stateful daemon running as part of db... VDB is basically just a
> virtualized layer to our databases that allows the same access regardless
> of environment or underlying technology. If it wants to delegate to the
> db CLI so you don't have to redefine the same tools twice, that makes
> sense. That's okay with me. VDB just has to keep track of which
> statements it has running and do proper cleanup and session management."
> The division on this edge, clarified: **db owns the headless stateful
> session daemon; vdb is the virtualization layer** (same access to
> databases regardless of environment/underlying technology), **delegating
> to db so the same tools aren't defined twice — and vdb owns statement
> tracking, cleanup, and session management** over the sessions it opens
> here. Note also (unchanged): mesh may use db via **direct CLI execution**
> for its own database (no daemon, no mesh dependency).

## Parties (WAVE-3 RESHAPED)
`vdb` daemon (`bin/vdb` on a database-hosting node) → `db` **CLI** (`bin/db` on
the SAME node), invoked as a **subprocess**. Cross-app, **never linked** (INTENT
#29 — the rule the operator coined against `db` specifically); and, after INTENT
#167, **never a wire protocol either** — `db` has no daemon. vdb spawns `bin/db
<verb> --format json`, hands it the resolved OS path + args, and parses the
structured JSON result. Authored from `db.md` (authoritative for the `db`
surface — owns `lib/db`'s real `Driver` trait) and `vdb.md` (authoritative for
the caller shape — the L4 stack daemon), co-designed in batch 4, **reshaped in
wave 3** to the subprocess split.

## Purpose (WAVE-3 RESHAPED)
The **cold-path control-plane arm**: the set of `db` verbs vdb invokes as a
subprocess for actions that reuse `db`'s proven, engine-aware logic —
migrations, handler/edge deploy per cloud target, **changelog codegen**
(installing the `_vdb_changelog` AFTER-triggers), **structured introspection**,
and the **promotion primitives** (dump/restore/hash/snapshot). `vdb` *tracks and
decides* (stack pattern, trigger matching, promotion orchestration); `db` *runs
and records* (SQL/DDL/edge execution against a resolved backend). **The runtime
hot path is NOT this contract** — the per-row-change handler actions and the
provenance-atomic writes run against **vdb's own warm driver connection** (the
"session," now living in vdb — INTENT #167), speaking SQL directly through
`rusqlite`/`tokio-postgres`, no `db` involvement (`vdb.md` concern 7). The
session-verb vocabulary below is retained as the shape of **vdb's internal
session surface**, not a cross-app wire.

## Schema
The **wire is now CLI invocation**: `bin/db <verb> [--format json] [args]`,
structured input/output as JSON. Value structs still land in `types::db` (the
DB surface, shared vocabulary) and `types::vdb` (`VdbError`); `Provenance` is
`types::provenance::Provenance` (frozen triple). The struct block below is the
**session-verb vocabulary retained as record** — it is now (a) the CLI verb set
vdb invokes via subprocess for cold-path actions, and (b) the shape of vdb's
INTERNAL warm-session surface for the hot path. It is NOT a WS frame protocol
any more (`db serve` is retired).

```rust
// WAVE-3 ROUTING KEY (which side of the split each verb lands on):
//   [CLI]      = cold path — vdb spawns `bin/db <verb> --format json`   (THIS contract)
//   [INTERNAL] = hot path  — vdb's own warm session speaks SQL directly (NOT a db call)
// The block is retained verbatim as record; the tags map it onto the decided split.

// ── canonical identity ────────────────────────────────────────────────
// RECONCILED: db.md's `DatabaseId { project, db }` is the canonical structured
// type in types::db; vdb.md's `DbId(String)` is its "<project>/<db>" slug
// rendering (used as the registry-slug segment). Both denote the same database.
pub struct DatabaseId { pub project: String, pub db: String }   // to_string() = "<project>/<db>"

// ── session lifecycle — [INTERNAL] vdb opens/closes its OWN warm connection ──
struct OpenSession  { target: DbDriverTarget } // -> OpenSessionAck { session_id: Uuid }
struct CloseSession { session_id: Uuid }
enum DbDriverTarget {                          // vdb SUPPLIES the resolved handle — see Reconciliation
    Sqlite        { os_path: String, database: DatabaseId },  // vdb+vfs materialized the file locally
    SupabaseCloud { project_ref: String, database: DatabaseId },
    AwsRds        { target_id: String, database: DatabaseId }, // design-only v1
    #[serde(other)] Unknown,
}

// ── the Driver surface (no new db behavior) — SPLIT by path ───────────
struct ApplySql    { session_id: Uuid, sql: String, prov: Provenance }              // [INTERNAL] mutating -> prov REQUIRED
struct ApplyParams { session_id: Uuid, sql: String, params: Vec<SqlParam>, prov: Provenance } // [INTERNAL] handler write
struct Query       { session_id: Uuid, sql: String, params: Vec<SqlParam>, write: bool } // [INTERNAL] -> Rows; write-gate
struct Introspect  { session_id: Uuid, q: IntrospectQuery }  // [CLI] `db introspect` -> Introspection | TableSchema (concern 2)
struct ApplyAtomic { session_id: Uuid, lock_key: String, statements: Vec<String>,
                     ledger: AppliedMigration, prov: Provenance }                    // [CLI] `db migrate` — xact-locked
struct EdgeDeploy  { session_id: Uuid, bundle: BundleRef }   // [CLI] `db edge deploy` — Supabase target only
struct OutboxDrain { session_id: Uuid } // [CLI] `db outbox drain` -> DrainReport (cloud outbox)

// ── change-capture: the SQLite procedural-trigger equivalent (concern 2) ──
struct InstallChangelog { session_id: Uuid, tables: Vec<String>, events: Vec<ChangeOp> }
                        // [CLI] `db install-changelog` — (re)gen AFTER INSERT/UPDATE/DELETE triggers @ apply_definition
struct ReadChanges { session_id: Uuid, since_seq: i64, limit: u32 } // [INTERNAL] vdb tails its own changelog
struct AckChanges  { session_id: Uuid, seqs: Vec<i64> }            // [INTERNAL] cursor advance in vdb's connection

// ── promotion primitives ([CLI] — vdb orchestrates; db provides) ──────
struct Snapshot    { session_id: Uuid, out: String } // [CLI] `db snapshot`   -> { content_hash: Hash }
struct DumpTo      { session_id: Uuid, format: DumpFormat } // [CLI] `db dump-to`     -> Stream<Bytes>  (bulk copy source)
struct RestoreFrom { session_id: Uuid, format: DumpFormat, body: Stream<Bytes> }      // [CLI] `db restore-from`
struct ContentHash { session_id: Uuid, table: String } // [CLI] `db content-hash`     -> Hash  (per-table verify)

// ── provenance read ([CLI] `db provenance`; the atomic WRITE is vdb's session, hot path) ──
struct ReadProvenance { session_id: Uuid, filter: ProvFilter } // [CLI] -> Vec<ProvenanceRow>

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

**The provenance-atomicity rule (load-bearing, `db.md` concern 3 / `vdb.md`
concern 8):** every mutating action writes its `ProvenanceRow` into the
`ops.provenance` / `ops_provenance` ledger **inside the same transaction as the
mutation**. After the wave-3 split the atomic write is executed by the
*connection owner*: **vdb's warm session** for `[INTERNAL]` hot-path handler
writes (`ApplyParams`/`ApplySql`), and **`bin/db`'s subprocess transaction** for
`[CLI]` cold-path migrations (`ApplyAtomic`). `db` owns the ops-schema shape +
the atomic-write discipline (installed via `[CLI]`); the invariant — no
committed mutation ever lacks its provenance — is identical on both sides. A
crash between the write and its trace is impossible by construction.

## Error cases
`db`'s errors are `DbError`; `vdb` **wraps them at its boundary into
`VdbError::ExecutionArm(String)`** — one crate's error enum never becomes
another's (types.md guardrail).

- `DbError::NotImplemented { command, driver, reason }` — e.g. `EdgeDeploy` on
  the `sqlite` driver (the TS handler is a vdb-hosted Deno handler, NOT a
  DB-side edge function; vdb's capability gate should prevent reaching this —
  hitting it is a vdb bug, logged loudly).
- **Cold-path `[CLI]` failures surface as `bin/db`'s process exit code + a
  `{ "error": DbError }` JSON on stderr** (structured `--format json`), which vdb
  parses and wraps. `SessionNotFound`/`SessionBusy` apply only to vdb's
  `[INTERNAL]` warm session (its own connection pool), not to the subprocess.
- `LedgerConflict` — `ApplyAtomic` (`db migrate`) lost its advisory-lock/ledger race.
- `DbError::MigrationFailed { id, detail }`.
- `DbError::ProtectedRefused { op, env }` — the prod guard; a write `Query`
  without `write: true` is refused here (the read/write gate, INTENT #60).
- `DbError::Backend(String)` — driver/SQL error, stringified at the boundary.
- Distributed single-writer / partition coordination surfaces as a `locks`
  error when vdb asked for it — carried by `locks-api`, **not re-typed here**.

## Version sensitivity
LOW — a **node-local, in-process-spawn** edge (no cross-node version skew, no
wire). Compatibility is **CLI-contract + on-disk-shape**, not envelope
versioning:

- **Additive-safe:** new `bin/db` subcommands/flags; new `--format json` output
  fields (vdb ignores unknown); new `DbDriverTarget`/`DumpFormat`/`ChangeOp`
  variants (each enum reserves `#[serde(other)]`); typed `TableSchema`/`Rows`
  cells are the named EXT growth path (additive over the frozen v1 string cells).
- **Breaking:** the `AppliedMigration`/ledger shape is `db`'s existing on-disk
  truth and therefore **effectively frozen**; changing the provenance-atomicity
  transaction boundary, the `_vdb_changelog` row shape, or the `--format json`
  result contract of a verb is breaking. vdb pins a **min `bin/db` version**
  (queried via `db --version`) as its boot-order compatibility check — the CLI
  analogue of a version floor, no live envelope needed.

## Reconciliation notes
1. **Where the warm handle lives — WAVE-3: it lives in vdb, not db.** The
   earlier drafts debated a stateless `db` vs a session-keyed `db serve` daemon
   holding warm handles. INTENT #167 settled it: **vdb holds the warm driver
   connection itself** (the `[INTERNAL]` hot path), so `session_id` keys a
   connection inside the vdb daemon, never a db-side one. This is consistent with
   `db.md` concern 6 (**`db` never talks to VFS** — vdb materializes the SQLite
   file locally and supplies the OS path): for the `[CLI]` cold path vdb passes
   the resolved `os_path` on the `bin/db` command line; for the hot path vdb
   opens `rusqlite`/`tokio-postgres` against that same path directly.
   `DatabaseId` still rides every invocation for logging/routing/registry-slug
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
   `lib/db` as a Cargo dependency" is contradicted by INTENT #29; this contract
   assumes **subprocess-only** consumption fleet-wide.
7. **WAVE-3: `db serve` retired, contract reshaped (INTENT #167).** The prior
   revision framed this whole edge as a WS session protocol over a `db serve`
   daemon. That is superseded: `db` has no daemon; the `[CLI]`-tagged verbs are
   `bin/db` subprocess invocations (this contract), and the `[INTERNAL]`-tagged
   verbs describe vdb's OWN warm session (not a db call at all). No cross-app
   wire remains on this edge. See `## Proposed contracts (wave 3)` below.

## Example data
World: nodes **macbook** and **pi**; project **demo**; the stack database
**demo/main** is anchored on **macbook** (a `LocalSqlite` target). vdb hosts
`demo/main`'s handler loop. **Cold-path actions are `bin/db` subprocesses;
hot-path handler writes/reads are vdb's OWN warm connection — no `db` call.**

**1. `[CLI]` Boot-time codegen** (vdb, at `apply_definition`, spawns `bin/db` to
install the changelog triggers — once):

```console
$ bin/db install-changelog \
    --path /Users/sam/Library/mind/vfs/vdb/demo/main.sqlite \
    --tables orders,invoices --events insert,update,delete --format json
{ "ok": true, "tables_covered": ["orders","invoices"] }
```

Then vdb opens its warm session **internally** (`rusqlite` against that same
path; no subprocess): `session_id = 9b1e…-s1`.

**2. `[INTERNAL]` A provenance-atomic handler write.** A row lands in
`demo.orders`; vdb's trigger fires a Deno handler that writes `demo.invoices`.
The write runs on vdb's warm connection; vdb sets the causation context and
INSERTs the `ops_provenance` row in the SAME transaction — no `db` involvement:

```rust
// inside the vdb daemon (session s1), one transaction:
tx.execute("INSERT INTO invoices(order_id, cents) VALUES (?1, ?2)",
           params!["ord-1042", 4200])?;                      // the handler's write
tx.execute("INSERT INTO ops_provenance(correlation_id, causation_id, …) VALUES (…)",
           params!["corr-77", "inv-77-h1", …])?;             // the atomic trace
tx.commit()?;   // invoices row + provenance row commit together
```

**3. `[INTERNAL]` Drain the changelog** (vdb's tailer reads its own connection):

```rust
let rows = session_s1.read_changes(/*since_seq*/118, /*limit*/500)?;
// rows[0] = ChangeRow { seq:119, table:"invoices", op:Insert, pk:{id:88},
//   after:{order_id:"ord-1042", cents:4200}, provenance:{correlation_id:"corr-77", …} }
session_s1.ack_changes(&[119]);   // cursor advance in _vdb_meta.processed_seq
```

**4. `[CLI]` Promotion verify leg** (vdb compares local `demo/main` to a fresh
cloud target — two `bin/db content-hash` subprocesses, vdb compares):

```console
$ bin/db content-hash --path /…/demo/main.sqlite --table orders --format json
{ "hash": "sha256:aa11…" }
$ bin/db content-hash --supabase demo-analytics --table orders --format json
{ "hash": "sha256:aa11…" }
# vdb: hashes match -> table parity confirmed; mismatch -> PromotionConflict, abort
```

## Proposed contracts (wave 3)

This edge's **shape changed this wave** (INTENT #167 retired `db serve`), so
the reshaped contract is stated here for the harmonizer. vdb owns the caller
side; db owns the verb surface; the reshape touches both.

### P1. `vdb-db` becomes a CLI-subprocess contract, not a WS protocol

- **Bearer:** vdb spawns `bin/db <verb> [--path <os_path> | --supabase <ref>]
  [args] --format json`, reads structured JSON from stdout, `DbError` JSON +
  non-zero exit on failure. No mesh, no daemon, no session frames on the wire.
- **Verb set (the `[CLI]` rows above):** `migrate` (apply/rollback/status/lint/
  crawl), `install-changelog`, `introspect --schema`, `edge deploy`
  (cloud-target only), `outbox drain` (cloud), `dump-to`, `restore-from`,
  `content-hash`, `snapshot`, `provenance` (read). Each already exists in
  `lib/db` or is the operator-authorized "extend db as necessary" set (db.md
  concern 1); wave-3 only fixes the *bearer* (subprocess, not socket).
- **Compatibility:** CLI-contract + on-disk-shape, not envelope versioning; vdb
  pins a **min `bin/db` version** (`db --version`) as its boot-order check.

### P2. The `[INTERNAL]` session surface leaves this contract

The `OpenSession`/`ApplySql`/`ApplyParams`/`Query`/`ReadChanges`/`AckChanges`
verbs are **no longer a cross-app contract** — they are the shape of **vdb's
own warm-session surface** (its `rusqlite`/`tokio-postgres` connections). The
harmonizer should **move these structs out of the `vdb-db` wire surface** and
into `types::vdb` (or a vdb-internal module) as vdb's session vocabulary. The
provenance-atomicity rule (write the `ops_provenance` row in the same
transaction as the mutation) rides here, executed by vdb.

### P3. Reconfirm the driver-crate boundary honors INTENT #29

vdb linking `rusqlite`/`tokio-postgres` directly does **not** violate #29 (which
forbids linking the `db` app-crate, not the shared public driver crates). Flag
for the harmonizer: ensure `vdb`'s `Cargo.toml` depends on the driver crates but
**never** on `substrate-db`; a build-time check (no `substrate-db` symbol in
`vdb`) is the enforceable form of the rule.
