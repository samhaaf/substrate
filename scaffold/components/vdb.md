# vdb

**Status:** NEW FILE (wave 2, batch 4) — the FULL design of the module the
round-8 lock named `vdb`. **This file SUPERSEDES `components/stack.md` as the
component design of record** (stack.md is retained as requirements history and
carries a superseded banner; see the naming resolution below — "stack" survives
as the PATTERN name). **Nesting:** top-level L4 app-crate (`bin/vdb` daemon +
`lib/vdb`), one daemon leg per database-hosting node
(`AddressingClass::NodeScoped`), plus one *virtual supervised entity per
managed database* (concern 2). **Layer:** L4 data & execution plane, in the
LOCKED order **VFS < VDB < KG** (INTENT #96). **Consumes (over the wire, never
linked — INTENT #29):** `db` via `vdb-db` (the action-runner execution arm),
`vfs` via `vdb-vfs` (SQLite files are NodeAnchored vfs files), mesh via
`vdb-mesh` (+ the cross-cutting `queues-api`/`cron-api`/`locks-api`/
`restart-protocol`/`pubsub-protocol`/`surface-schema`), `secrets` via
`vdb-secrets`, `aws` via `aws-vdb` (cloud target, design-only v1).
**Compiled-in shared libs:** `mesh-client`, `substrate-types`, and the
`execution-engine` (stack-tables adapter — an internal library dependency,
NOT a contract edge, locked rounds 4–5). **Consumed by:** `kg` via `kg-vdb`
(KG is built ON VDB), `projects`/`environments` (L6 stub-track edges).
Grounded in INTENT #61/#65/#70/#73/#74/#81/#85/#86/#91/#92/#93/#96/#98, the
batch-1/2/3 designs (queues' trigger data model, supervision's restart ladder +
port-handoff choreography, locks' `vdb.promote.*` example slugs, vfs'
NodeAnchored/`OpenAnchored`/`Snapshot` surface, secrets' `vdb-secrets` half,
aws' `aws-vdb` half, cron's pg_cron-leg decomposition), and the REAL `lib/db`
code (Driver trait + Capabilities, handler Contract/registry, outbox,
migration ledger, `apply_atomic`, `vault.rs`).

> **NAMING RESOLUTION (the wave2-plan §5.1 ambiguity — recommendation).**
> **`vdb` is the crate and the module** (round-8 lock: "VDB becomes the
> daemon"); crate naming is `bin/vdb` + `lib/vdb`, registry slug `vdb`.
> **"stack" survives as the PATTERN name only** — the *stack pattern* is the
> operator's database-centric paradigm; a *stack definition* is the
> declarative bundle that describes one application of it (concern 1); there
> is no `stack` crate and no `stack` service slug. **stack.md's content folds
> into this file** (all of its requirements are carried in the concerns
> below); stack.md itself is marked superseded-pending-harmonizer, not
> deleted. **Contract-stub renames flagged for the harmonizer (NOT performed
> here):** `contracts/stack-vfs.md` → `vdb-vfs`, `contracts/stack-mesh.md` →
> `vdb-mesh`. Rationale for folding rather than keeping two files: the
> pattern and its implementation vehicle have exactly one design between them
> (INTENT #93: "ideally VDB perfectly implements our stack") — two files
> would force every future edit to decide which one it belongs to, which is
> precisely the ambiguity the round-8 lock closed.

## Charter

`vdb` is **the deploy-anywhere implementation of the stack pattern** — the
daemon that TRACKS and EXECUTES the operator's database-centric backend
paradigm (INTENT #61: tables as intermediate data structures; SQL and Deno/TS
handlers firing on row/table changes; "I've almost entirely eliminated my need
for FastAPI") against **three adapter targets**: **local** (the SQLite stack
daemon — a daemon around a single SQLite file living in the VFS, INTENT #73;
SQLite-locally is LOCKED, no local Postgres ever, no Docker ever), **Supabase**,
and **AWS RDS + Lambda** (through the `aws` crate; design-only in v1). It owns:
the **stack-definition model** (schema migrations + declarative triggers +
handler code + config, as data — concern 1); the **database catalog** (every
managed database, its target, its home, its status — replicated mesh-wide via
`replicated-kv`, concern 10); **databases-as-services** (each managed database
is a registered, supervised entity inheriting the LOCKED 4-level restart
ladder and the port-handoff/registry-flip choreography — INTENT #86/#96,
concern 2); the **local change-capture + handler-execution loop** (the
changelog, the execution-engine's stack-tables adapter, the Deno runtime —
concerns 4/6); **copy/verify/switch-under-a-lock promotion** with
let-edge-functions-finish semantics (INTENT #86, concern 9); and — its
PRIMARY-HOME responsibility — **first-order, healthcare-data-engineer-grade
provenance**: every handler touch of data traced from the very beginning,
configured per-project/per-database (INTENT #85/#92, concern 8).

**Boundary — what vdb does NOT own.** It does not run SQL itself against any
engine — **`db` is its execution arm** (round-8 lock: "it takes advantage of
the db crate to actually run the actions against specific databases — we
extend db as necessary"; concern 7). It does not own file placement,
replication, or durability of the SQLite bytes — the files are **NodeAnchored
vfs files** and VFS snapshot-replicates them (`vdb-vfs`; VFS < VDB). It does
not own the trigger *data model* — that is `types::trigger`, authored by
`queues` (batch 2), consumed UNCHANGED with a row-change subject binding
(concern 6). It does not own trigger/handler *evaluation mechanics* — that is
the shared `execution-engine` lib (its stack-tables adapter runs inside the
vdb daemon; loop detection, causal-chain tracking, and the cc escalation hook
live there, not here). It does not own scheduling (`cron`), delivery
(`queues`), distributed semaphores (`locks`), replication of its catalog
(`replicated-kv`), cloud mechanics (`aws`), or secret material (`secrets`). It
does not own the graph layer — `kg` builds ON it (`kg-vdb`; VDB < KG). It is
not a general database product: it manages **stack databases** (databases
described by stack definitions) and deliberately nothing else — `inference`'s
per-node `store`, mesh's kernel SQLite, and gc's store are NOT vdb databases
(each owns its own file; deliberately so, per store.md). And it is **not a
distributed/multi-master database**: every managed database has exactly ONE
authoritative home at a time (concern 10) — cross-database consistency is
achieved by events and handlers riding the mesh, never by row-level
multi-master merge.

## Primary design concerns

### 1. The vocabulary: stack definition → database → target (the deploy-anywhere model)

Everything vdb does is a function of three declarative nouns, kept strictly
apart because "deploy-anywhere" is exactly the ability to rebind them:

- **Stack definition** — the portable, declarative description of one
  application of the stack pattern: an ordered set of **schema migrations**
  (db's migration format, ledger-tracked), a set of **declarative triggers**
  (`types::trigger` data — concern 6), a set of **handlers** (SQL text and
  Deno/TS modules — the ONLY place code lives), and **config** (provenance
  level, default queue names, handler resource limits). It is pure data +
  code-as-content: migrations and handler sources are content-addressed blobs
  in VFS; the definition manifest references them by hash. A stack definition
  has a version (the migration ledger head + a manifest hash). It knows
  nothing about where it runs.
- **Database** — one deployed instance of a stack definition:
  `{ project, name }` → a concrete engine-level database on some target, plus
  its runtime state (anchor node, status, provenance store, changelog cursor).
  Identity is `DbId = "<project>/<name>"` (e.g. `broomstick/analytics`).
  Multiple databases may deploy the same definition (a dev and a prod copy).
- **Target** — where a database physically runs: `LocalSqlite { node }`,
  `Supabase { project_ref }`, or `AwsRdsLambda { target_id }`. The routing
  rule is the operator's own (INTENT #98): **local environment → SQLite;
  cloud → promote**. In v1 the target is explicit per-database config;
  environment-driven routing arrives when `environments` leaves the stub
  track (`environments-vdb`, anticipated).

```rust
// catalog record — replicated mesh-wide via replicated-kv (concern 10)
pub struct DatabaseRecord {
    pub db_id: DbId,                      // "<project>/<name>"
    pub definition: StackDefRef,          // manifest hash + ledger head (VFS content refs)
    pub target: StackTargetKind,          // LocalSqlite{node} | Supabase{..} | AwsRdsLambda{..}
    pub status: DbStatus,                 // Provisioning | Running | Promoting{..} | Degraded | Retired
    pub provenance: ProvenanceConfig,     // per-database (INTENT #92) — concern 8
    pub version: LwwVersion,              // (wall_clock, node_id), KV discipline
}
pub enum StackTargetKind {
    LocalSqlite { node: NodeId, vfs_path: String },   // "vfs://vdb/<project>/<name>.sqlite"
    Supabase    { project_ref: String },
    AwsRdsLambda{ target_id: String },
    #[serde(other)] Unknown,                          // older daemon tolerates a newer target kind
}
```

### 2. Databases-as-services — virtual supervised entities under the restart ladder (INTENT #86/#96)

"Databases get treated like services under the same mesh restart/upgrade
protocol." supervision (batch 2) deliberately designed its whole protocol
against a *generic supervised service* = `{ endpoint, version,
Interruptibility, restart-protocol participation }` (supervision.md concern
10) precisely so vdb could inherit it. The design decision here — flagged as
the batch's first controversial call — is **how a database becomes such an
entity**:

**One vdb daemon per node, multiplexing N databases as VIRTUAL supervised
entities — NOT one OS process per database.** The vdb daemon is the single
process; each managed database it hosts is registered *individually* in the
service registry and speaks the restart protocol *individually*, through the
daemon:

- **Registry:** the daemon registers `vdb` (NodeScoped, like vfs) once per
  node; each database additionally registers the slug `vdb/<project>/<name>`
  whose `Endpoint` is the local daemon plus a database route. Resolving a
  database from any node is an ordinary `service-lookup` — the wiring seam is
  reused, not extended. On promotion, the registry flip of THIS entry is the
  "switch" (concern 9).
- **Restart-protocol, per database:** the daemon maintains one
  `Interruptibility` state per database entity and answers `RestartRequest`s
  addressed to a database slug: a database mid-write-transaction or
  mid-migration is `CriticalSection`; one with in-flight handler invocations
  is `Interruptible` and answers L2 `FinishAndRelinquish` by draining those
  invocations (exactly the let-edge-functions-finish primitive concern 9
  reuses); an idle database is `Idle`. L3 `SaveWindow` = flush WAL checkpoint
  + `Snapshot` to VFS within the ~10s window. L4 kill of a *database* closes
  its connections and suspends its trigger loop without killing the daemon;
  L4 kill of the *daemon* is mesh-core's ordinary kill (SQLite's crash-safety
  + the changelog cursor make it recoverable — concern 4).
- **Why not process-per-database:** SQLite is in-process (there is no server
  to supervise separately); N processes would multiply Deno runtimes and
  registry churn on a Pi hosting a dozen small databases; and supervision's
  ladder needs an *addressable participant*, not an OS process — the protocol
  is satisfied by the multiplexed entity. The cost — one wedged handler
  runtime can degrade sibling databases on the same node — is bounded by
  per-database Deno worker pools with hard limits (concern 4) and is flagged
  honestly rather than hidden.

Version tracking rides the same inheritance: a database entity's "version" in
its `ServiceManifest` is its stack-definition version, so supervision's
pairwise-requirement machinery can express "service X requires
`vdb/broomstick/analytics` ≥ ledger-head H" if it ever needs to.

### 3. The adapter matrix — `StackTarget`, capability-gated, seeded by db's drivers

The deploy-anywhere seam is one trait, deliberately shaped like `db`'s proven
`Driver` + `Capabilities` pattern (db.md: "the natural seed of VDB's adapter
matrix" — the incapable-adapter-degrades-early-with-typed-`NotSupported`
discipline is inherited wholesale):

```rust
#[async_trait]
pub trait StackTarget: Send + Sync {
    fn kind(&self) -> StackTargetKind;
    fn capabilities(&self) -> StackCapabilities;
    // lifecycle
    async fn provision(&self, db: &DbId, def: &StackDef) -> Result<(), VdbError>;
    async fn apply_definition(&self, db: &DbId, def: &StackDef) -> Result<(), VdbError>; // migrations via db, triggers/handlers installed
    async fn teardown(&self, db: &DbId) -> Result<(), VdbError>;
    // data plane (all delegate to `db` over vdb-db — concern 7)
    async fn query(&self, db: &DbId, sql: &str, params: &[SqlParam]) -> Result<Rows, VdbError>;
    async fn exec (&self, db: &DbId, sql: &str, params: &[SqlParam]) -> Result<ExecReceipt, VdbError>;
    // handler plane
    async fn invoke(&self, db: &DbId, handler: &HandlerName, payload: Value, prov: Provenance)
        -> Result<InvokeResult, VdbError>;
    // promotion plane (concern 9)
    async fn copy_from(&self, db: &DbId, source: &SourceRef, mode: CopyMode) -> Result<CopyReport, VdbError>;
    async fn verify_against(&self, db: &DbId, source: &SourceRef) -> Result<VerifyReport, VdbError>;
}
pub struct StackCapabilities {
    pub triggers_native: bool,   // engine-level triggers (pg) vs vdb-generated changelog (sqlite)
    pub edge_handlers: bool,     // platform handler hosting (Supabase edge fns / Lambda)
    pub promote_target: bool,    // can be promoted TO (cloud targets true; LocalSqlite false in v1)
    pub listen_notify: bool,     // native LISTEN/NOTIFY (pg) vs mesh pubsub tee (sqlite)
    pub vector: bool, pub fts: bool,          // capability probes that feed promote-vs-stay advice
}
```

- **`local-sqlite` — BUILD, the heart of v1** (concern 4). Handlers run in
  vdb's own Deno runtime + SQL via db; triggers are vdb-evaluated off the
  changelog; the file is a NodeAnchored vfs file.
- **`supabase` — BUILD, by leverage.** Maps almost 1:1 onto machinery `db`
  already has in production: migrations via db's supabase drivers; Deno
  handlers deploy as **Supabase edge functions** (db's `edge.rs`
  build/deploy/activate with pointer-flip rollback); triggers compile to pg
  triggers writing db's existing `handler_dispatch` **outbox**, drained by
  db's drainer; handler registry = db's existing `handler_versions`/
  `handler_active` ops schema. vdb *orchestrates*; db executes. (Note db's
  `supabase-local` Docker path is dead under the no-Docker lock — flagged in
  db.md already; the Supabase target here is cloud.)
- **`aws-rds-lambda` — DESIGN-ONLY v1**, through `aws` (`aws-vdb`, whose
  aws-side half is already proposed in aws.md and accepted here): RDS
  Postgres provisioning, Deno handlers as Lambda (`LambdaRuntime::Deno`),
  triggers as pg triggers → outbox → a drainer Lambda. Every call answers
  `AwsDisabled` until the adapter goes Live — vdb treats that as
  "promotion unavailable, stay local," which is the normal v1 state.

**One stack definition, three targets** is the conformance bar: the same
definition must behave identically (same trigger firings, same handler
payloads, same provenance records) on local-SQLite and Supabase — that
cross-target fixture suite is the deploy-anywhere property made testable
(non-obvious tests, below).

### 4. The local target — the SQLite stack daemon (the operator's whole backend paradigm)

INTENT #73, made concrete. Per hosted database on the anchor node:

- **The file.** `OpenAnchored("vfs://vdb/<project>/<name>.sqlite")` — vfs
  hands vdb a real local OS path plus the `vfs.anchor.<path>` exclusive-writer
  lock (vfs.md concern 2; vdb holds it for the life of the hosting). SQLite
  runs in **WAL mode**: the daemon is the single writer; concurrent local
  reads are free. Durability beyond the node = **snapshot-driven**: at
  transaction-consistent checkpoints (`wal_checkpoint(TRUNCATE)` — after
  migrations, after promotion steps, on L3 SaveWindow, and on a periodic
  cadence from `ProvenanceConfig`/db config) vdb calls `Snapshot(path)` and
  vfs content-addresses + replicates that snapshot as an immutable blob at
  the directory's replication factor. Point-in-time recovery = restore
  snapshot + (optionally) replay the changelog tail (which is IN the file and
  therefore in the snapshot).

- **Change capture: the changelog table is the durable truth.** How do
  handlers fire on row changes in an embedded engine with no LISTEN/NOTIFY?
  The boring, durable answer — and the one that is *portable to Postgres
  targets by construction*: **vdb generates engine-level AFTER
  INSERT/UPDATE/DELETE triggers on every stack table** (via db, at
  `apply_definition` time, regenerated on every migration) **that append a
  row to `_vdb_changelog`**:

  ```sql
  CREATE TABLE _vdb_changelog (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    txn_id      TEXT NOT NULL,          -- groups rows of one transaction
    tbl         TEXT NOT NULL,
    op          TEXT NOT NULL,          -- insert | update | delete
    pk          TEXT NOT NULL,          -- json-encoded primary key
    old_row     TEXT,                   -- json (NULL on insert)
    new_row     TEXT,                   -- json (NULL on delete)
    changed_at  TEXT NOT NULL,          -- wall clock
    causation_id TEXT                   -- provenance: set when the write came from a handler (concern 8)
  );
  ```

  The daemon **tails the changelog by cursor** (`processed_seq`, persisted in
  `_vdb_meta` in the same file — same transactional domain, so a crash never
  loses or forgets its place). `rusqlite`'s `update_hook` is used only as a
  *wakeup* (poll-suppression) optimization, never as truth — hooks are
  connection-local and lossy across restarts; the changelog is neither.

- **Trigger evaluation + dispatch.** For each unprocessed changelog row the
  daemon binds the **row-change subject document** exactly as queues.md
  concern 2 specifies for the execution-engine —
  `{ "table": tbl, "op": op, "old": old_row, "new": new_row,
  "meta": { event_id, occurred_at, provenance } }` (deterministic
  `event_id = uuid_v5(VDB_NS, db_id ++ seq)`) — and hands it to the
  **execution-engine's stack-tables adapter** (compiled in), which evaluates
  the database's registered `types::trigger` set (same `FilterExpr` /
  `AssemblyTemplate` AST as queues, different subject binding), assembles
  each matched handler's payload (rollup references resolve via
  `rollup-mesh`, with the `llm_safe` degradation queues already asserted),
  records the causal chain, runs loop detection, and dispatches. Delivery is
  at-least-once (cursor advances only after dispatch outcomes are recorded);
  handlers carry the same idempotency contract queues locks in
  (`event_id` + `correlation_id` in every payload).

- **Handler execution.** Two kinds, exactly db's existing handler `Kind`
  vocabulary (`Sql` | `Edge`→Deno), plus its `Invocation` split
  (`EffectAsync` — the normal fire-and-record path — and `ValidatorSync` —
  evaluated inline before commit, capability-gated):
  - **SQL handlers** — a SQL statement/script run against the same database
    through db (`vdb-db`), parameterized from the assembled payload
    (`apply_params`, injection-safe).
  - **Deno handlers** — TS modules (content-addressed in VFS) run in a
    per-database **Deno worker pool** with a hard sandbox: `--allow-net`
    restricted to the local mesh daemon `:3649` plus an explicit per-handler
    allowlist from the stack definition; no ambient fs (temp dir only); env
    empty except the injected context. Each invocation receives a **context
    object**: `{ payload, db: { query, exec }, mesh: { publish, enqueue },
    secrets: { use(ref, sink) }, provenance: { correlation_id, causation_id } }`
    — `db.query/exec` round-trips through vdb (so every handler write lands
    in the changelog *with `causation_id` set*, concern 8), `secrets.use`
    is the use-without-seeing verb (a handler wields a `SecretRef`, secrets
    performs the authenticated action — secrets.md concern 3), and third-party
    calls are plain `fetch` under the allowlist. Timeouts/fail policy come
    from the handler contract (db's `timeout_ms`/`FailPolicy` reused).

- **The "backend without FastAPI" surface.** Apps drive their stack over the
  database entity's mesh surface (resolve `vdb/<project>/<name>`, then):
  `Query`/`Exec` (data), and `Invoke { handler, payload }` — the RPC/edge
  -function-equivalent for logic that isn't change-driven. Everything else
  *is* change-driven: insert a row, the handlers ripple the transformation
  through the tables. That sentence, minus the reserved word, is the paradigm.

### 5. SQLite sufficiency — the daemon-level equivalence table (INTENT #91, the required design input)

The local stack MUST functionally cover what Postgres would have provided,
because SQLite-locally is locked and there is no local fallback. The
definitive mapping (the analysis stack.md kept as a required input, now
concrete — each right-hand side names the real designed mechanism):

| Postgres capability | Daemon-level equivalent (designed, not aspirational) |
|---|---|
| `pg_cron` | **mesh `cron`** — a `cron-api` job emits `stack.<db>.<name>.tick` into queue `vdb.<project>.<db>.jobs`; a declarative trigger assembles the payload; a stack handler runs the SQL/Deno work. cron.md concern 6 designs this leg end-to-end (its worked example is literally `vdb/analytics/nightly-rollup`). |
| `LISTEN/NOTIFY` | **mesh `pubsub-relay`** — vdb tees change events onto topic prefix `vdb/<project>/<db>/…` (lossy, observability-grade, like queues' tee); durable consumers register a trigger with `HandlerRef::Service`/queue delivery instead (guaranteed). Two consumers, two channels, both standard. |
| Procedural triggers (PL/pgSQL) | **execution-engine handlers** — declarative `types::trigger` + SQL/Deno handler off the changelog (concern 4). Strictly more capable (handlers can call the mesh) and uniformly provenance-traced. |
| Edge functions / RPC | **`Invoke` on the database entity** + Deno handler pool (concern 4). |
| Advisory locks / `SELECT … FOR UPDATE` coordination | **mesh `locks`** (`locks-api`) for cross-process/cross-node coordination; SQLite's single-writer daemon serialization covers in-database write ordering by construction. |
| Roles / GRANTs / RLS | **Not replicated locally, deliberately.** Access = mesh identity (which service resolved the entity) + secrets brokerage; a personal mesh has one operator (access control deprioritized, INTENT #39). Promotion to Supabase re-enters RLS territory — the definition may carry RLS policies as migrations that are no-ops locally (capability-gated). Flagged, not hidden. |
| `jsonb` | SQLite JSON1 (`json_extract` etc.) — db's sqlite driver already rewrites per-engine SQL (design §3.4 seam). |
| Full-text search | SQLite FTS5 (`StackCapabilities.fts = true` locally). |
| Vector search (pgvector) | **Gap, honestly flagged**: `sqlite-vec` as the local candidate, else the need is a *promotion trigger* (a database that needs pgvector is a database that has outgrown local). `StackCapabilities.vector` carries the probe; the promote-vs-stay advice surfaces it. OPEN question for the operator. |
| Concurrent writers / connection pools | WAL + the single-writer daemon — correct at personal-mesh scale by construction; a write-throughput ceiling is a promotion signal, not a local engineering project. |
| Replication / HA | **VFS snapshot replication** (factor-N durable snapshots) + the promotion path. Deliberately NOT streaming replication (concern 10). |

The operator's own counter-question — "why would we ever upgrade if the full
stack works on SQLite?" — gets its honest answer from this table: the residual
reasons to promote are *internet-facing serving, write concurrency beyond one
node, and engine-exclusive capabilities (vector today)* — all cloud-shaped,
which is why the upgrade target is only ever cloud (round-9 lock).

### 6. Triggers and handlers — one declarative data model, registered as data, travelling with the database

- **The data model is `types::trigger`, consumed UNCHANGED** (queues.md
  concern 2 authored it; the wave2-plan binds execution-engine and queues to
  one model, and vdb rides the execution-engine). A stack trigger is a
  `Trigger` whose subject binding is the row-change document; the
  `queue: QueueName` field carries the stack binding (`table:<tbl>` scope) in
  the adapter-extension struct queues prescribed for adapter-specific needs —
  the core struct stays boring. Per-trigger `SemaphoreChoice` and
  `IdempotencyMode` carry over with identical semantics.
- **Registration is data, never code (INTENT #103), and lives IN the
  database.** Triggers + handler-contract rows are installed into the
  database's own ops schema (extending db's existing `handler_versions` /
  `handler_active` registry — real code, reused) by `apply_definition`.
  Handler *source* is content-addressed in VFS; the registry row references
  the hash. Consequences, all deliberate: (a) a database is
  **self-describing** — its triggers/handlers promote WITH it (copy the
  database, the behavior comes along; the Supabase adapter re-compiles the
  same rows to pg-native form); (b) registration is **statically validated**
  (a malformed `FilterExpr` is rejected at `apply_definition`, never at
  dispatch — the queues property, inherited); (c) trigger edits are ordinary
  definition-version bumps, migration-ledger-tracked, provenance-stamped.
- **Distributed coordination — mostly unnecessary by construction (a
  deliberate simplification of the stack-mesh stub's framing, flagged).** The
  old `stack-mesh` stub said "distributed-handler coordination via locks."
  Because a database has ONE home (anchor or cloud) and only the home
  evaluates its changelog, **stack-table trigger firing needs no distributed
  lock at all in the common case** — single-fire is by construction. `locks`
  is needed exactly where the mesh enters: (a) the **promotion mutex**
  `vdb.promote.<db_id>` (locks.md's own example slug); (b) the **anchor
  lock** (held via vfs); (c) **event-ID semaphores** when a stack handler
  consumes a *mesh queue* trigger or an `Anywhere` cron fire — and those are
  queues'/cron's own designed mechanisms, riding through unchanged. The
  `vdb-mesh` contract below reflects this reduced, honest scope.

### 7. `db` as the execution arm — the call mechanism (co-designed with db, batch 4; ACCEPTED)

> **[UNFROZEN — ACCEPTED (friction-round 3, 2026-07-20, INTENT #128;
> resolves the INTENT #115 drift flag).]** The operator accepts the `db serve`
> headless stateful session daemon as part of db, with the division
> clarified, verbatim: "VDB is basically just a virtualized layer to our
> databases that allows the same access regardless of environment or
> underlying technology. If it wants to delegate to the db CLI so you don't
> have to redefine the same tools twice, that makes sense. That's okay with
> me. VDB just has to keep track of which statements it has running and do
> proper cleanup and session management." So: **vdb = the virtualization
> layer** (same access to databases regardless of environment/technology),
> **delegating to db** (so the same tools aren't defined twice), and **vdb
> owns statement tracking, cleanup, and session management** over the
> sessions it opens. See `db.md` concern 1 and `contracts/vdb-db.md`.

The locked decomposition makes `db` the crate that "actually runs the actions
against specific databases," and INTENT #29 forbids linking it. The mechanism
proposal (db's designer runs concurrently — this is vdb's preferred shape,
flagged for mid-batch reconciliation):

**`db` grows a daemon mode — `db serve` — registering slug `db` (NodeScoped)
on the mesh, speaking the `db-control-plane` protocol over the local `:3649`
daemon; vdb keeps a session per hosted database.** Concretely:

- **Session-oriented WS surface**, mapping ~1:1 onto db's existing `Driver`
  trait (the real code): `OpenSession { target: DriverTarget } → session_id`,
  then `ApplySql`/`ApplyParams`/`Query`/`QueryParams`/`Introspect`/
  `ApplyAtomic`/`LedgerRecord`/`AdvisoryLock`/`EdgeDeploy`/`OutboxDrain` —
  each an existing `lib/db` function exposed, not new behavior. `bin/db`'s
  CLI is untouched; the daemon is a second thin shell over the same lib
  (db's own dual-role shape, extended).
- **Co-location rule:** a `db serve` instance runs on every database-hosting
  node (a supervision boot-order fact: `vdb` requires `db` locally), so the
  hot path is vdb → local mesh daemon → local db → SQLite file — one local
  relay hop, no cross-node traffic for local work. Cross-node db access is
  never needed: vdb work happens at the database's home by construction.
- **Extensions db needs (the operator-authorized "extend db as necessary"
  list, for db's designer):** (1) the `db serve` session surface itself; (2)
  **changelog codegen** for the sqlite driver — generate/regenerate the
  `_vdb_changelog` triggers per table at migration-apply time (mirrors its
  existing handler codegen); (3) **copy/verify verbs** for promotion —
  `DumpTo { format }`, `RestoreFrom`, `ContentHash { table } → hash` (schema
  replay already exists via the ledger; these add the data legs); (4) sqlite
  **outbox parity** (its `outbox` capability flag turned on for sqlite where
  applicable) so the Supabase and local dispatch stories stay symmetric; (5)
  the already-flagged reconciliation of its keychain/vault reads toward
  `secrets` (db.md round-8/9 notes — not vdb's to design).
- **Performance flag, stated honestly:** every handler-context `db.query`
  crosses two local WS hops (vdb→mesh→db). At personal-mesh scale this is
  fine (sub-ms local relay); if it ever measures, the *sanctioned* escape is
  the same skeleton-time latitude vfs took with gc — db's sqlite driver is a
  lib db itself compiles; an in-process fast path would be a db-owned
  embedding decision, never vdb importing db's internals. Default is the
  wire; the contract shape is identical either way.

### 8. Provenance — the PRIMARY HOME (INTENT #85/#92): every handler touch, healthcare-grade

"I want to see everything that led to the current state of our database,
every time data gets touched by a handler." vdb is where that promise is
kept. The design:

- **Storage is IN the database** (`_vdb_provenance`, `_vdb_changelog` —
  ops-schema tables in the same engine-level database), so provenance
  **travels with the data**: a promotion copies it; a snapshot captures it; a
  restored database still explains itself. This is the healthcare-data
  -engineer instinct made structural — the audit trail is never in a side
  system that can drift from the data it describes.
- **The record.** Every handler invocation writes one provenance record;
  every changelog row a handler causes carries that invocation's id as
  `causation_id` (concern 4's context object routes handler writes through
  vdb precisely to guarantee this):

  ```rust
  pub struct HandlerInvocationRecord {
      pub invocation_id: Uuid,                 // = the causation_id of every write it makes
      pub db_id: DbId,
      pub trigger_id: TriggerId,
      pub handler: { name: String, version: String, kind: Sql|Deno, source_hash: Hash },
      pub cause: Provenance,                   // types::Provenance of the triggering change/event
      pub correlation_id: Uuid,                // the causal-chain root, stable across hops
      pub subject: { table: String, op: Op, pk: Json },   // what fired it
      pub assembled_payload_hash: Hash,        // what the handler actually received (hash, not body)
      pub started_at / finished_at: DateTime<Utc>,
      pub outcome: Succeeded { rows_touched: u32 } | Failed { error } | LoopStopped { depth },
  }
  ```

- **The chain is walkable in both directions.** Row-level lineage: for any
  row, its last changelog entries → their `causation_id`s → invocation
  records → *their* `cause` changelog entries → … back to the external write
  or cron/queue event at the root (`correlation_id` stitches across
  databases and planes — the same triple `types::Provenance` gives the whole
  OS). Forward: an invocation's writes are every changelog row bearing its
  id. This one chain is simultaneously: the operator's audit surface
  (`vdb provenance <db> --row <table>/<pk>` and `--trace <correlation_id>` on
  the CLI + dashboard), **the execution-engine's loop-detection substrate**
  (loop depth = repeated (table, pk) occurrences along a causation chain,
  INTENT #70 — one chain, two consumers, by design), and the cc
  escalation's investigation context.
- **Configured per-project/per-database (INTENT #92):**
  `ProvenanceConfig { level: Full | Touch | Off, retention: Duration,
  row_images: bool }`. `Full` (default for stack databases): changelog keeps
  old/new row images — total reconstruction. `Touch`: who/what/when per row,
  no images (bulk-heavy databases). `Off` exists for scratch databases but is
  loud (surface-schema shows it). Retention compacts old changelog/provenance
  into VFS-archived segments (immutable blobs) rather than deleting —
  "from the very beginning" means the chain may leave the hot file, never the
  system.
- **Cross-target invariance:** the Supabase/RDS adapters install the same two
  ops tables and their outbox-drainer path writes the same records — the
  provenance schema is part of the stack definition's conformance fixtures,
  so healthcare-grade does not degrade at promotion.

### 9. Promotion — copy/verify/switch under a lock, let-edge-functions-finish (INTENT #86)

Local→cloud only (round-9: upgrading past SQLite MEANS promoting to cloud).
This is supervision's port-handoff choreography (spawn-new → verify → flip →
relinquish-old → down-old) specialized with a `locks`-held write barrier —
supervision.md concern 10 anticipated exactly this specialization. The
sequence, per database:

1. **Lock.** Acquire the `locks` mutex `vdb.promote.<project>/<name>`
   (threshold 1, `Durable` class). Partition honesty: a
   `PartitionMergeThresholdExceeded` on this slug aborts the promotion — two
   half-promotions are the one unrecoverable state, so the CAP-honest error
   is fatal-to-the-attempt by policy (INTENT #84, handled per-application:
   this application's handling is "abort loudly").
2. **Provision + prepare target.** `StackTarget::provision` +
   `apply_definition` (schema via ledger replay through db; triggers/handlers
   compiled to the target's native form; ops tables installed). Status →
   `Promoting{phase}` in the catalog (replicated — every node sees it).
3. **Bulk copy, source still live.** Initial data copy (db's `DumpTo`/
   `RestoreFrom`) while the local database keeps serving and firing handlers;
   record the changelog high-water mark at copy start; then **catch-up
   passes** replay changelog segments accumulated during the copy. Repeat
   until the tail is small.
4. **Verify.** `verify_against`: per-table row counts + content hashes (db's
   `ContentHash`), migration-ledger identity, trigger/handler registry
   identity, provenance-table presence. ANY mismatch → **abort**: release the
   lock, tear down or park the target, local database untouched and never
   disturbed (the same never-disturb-the-old abort discipline as
   supervision's `HandoffStalled`).
5. **Barrier + drain (the let-edge-functions-finish semantics, verbatim
   INTENT #86).** Enter the write barrier: the entity's interruptibility goes
   `CriticalSection`; **new** trigger firings and external `Exec`/`Invoke`
   are paused (queued, bounded); **running handler invocations finish against
   the OLD database** — vdb drains the in-flight set exactly as an L2
   `FinishAndRelinquish` (same code path, concern 2). Reads continue against
   the old database throughout (proposed; flagged as an operator question).
6. **Final delta + switch.** Copy the changelog tail the drained handlers
   just wrote ("write results back" — the finished edge functions' writes are
   in the tail by construction, so swapping the database underneath loses
   nothing); re-verify the tail; **flip the registry entry**
   `vdb/<project>/<name>` → the cloud target's route (the LWW registry flip
   IS the switch — resolves route to the new home from the next lookup);
   update the catalog (`target`, `status: Running`).
7. **Resume + retire.** Release the barrier — queued firings and calls
   dispatch against the NEW database (their subject events were captured as
   data, so nothing fired twice and nothing was lost); the old SQLite file
   gets a final `Snapshot` to VFS, is marked `Retired` (kept read-only for a
   confirmation window), then handed to normal VFS/gc lifecycle. Release the
   promotion lock. Every step emits `vdb.promote.*` events + provenance
   records (the promotion itself is a traced touch of the data).

Failure at any step before 6 = abort with the local database authoritative
and untouched. Failure *during* step 6's flip window is the one delicate
spot: the registry flip is a single LWW write (atomic per-key), and the
barrier holds until the flip is confirmed replicated to reachable nodes —
the same distribute-before-confirm discipline locks uses for acquisition.

### 10. Eventual consistency across databases — what replicates vs what stays put (the brief's question, answered)

- **Replicates mesh-wide via `replicated-kv`** (small LWW control-plane
  state, keyspace `vdb/`): the **database catalog** (`vdb/db/<db_id>` →
  `DatabaseRecord`), **stack-definition manifests' refs** (`vdb/def/<hash>` →
  pointer + metadata; bodies are VFS blobs), and **promotion state**. So any
  node answers "what databases exist, where does each live, what version,
  what status" from a local read — the same fully-replicated-directory
  property vfs built for files. Catalog writes are LWW `(wall_clock,
  node_id)`; the catalog is *directory*, not data.
- **Does NOT replicate: the data.** A database has **one authoritative home**
  (anchor node or cloud target). vdb is deliberately not a multi-master
  database — row-level LWW merge on relational data is semantically wrong
  (the KG designer gets the "superset" problem; vdb refuses it). Durability
  is VFS snapshot replication; availability of a database whose home is
  offline is *honest unavailability* (resolve fails; a walk-along Pi's
  database is reachable again when the Pi is — INTENT #84 physics).
- **Cross-database consistency is the stack pattern itself, riding mesh.**
  A handler in database A emits an event (`mesh.enqueue` in the handler
  context) → a queue trigger assembles a payload → a handler in database B
  applies it. At-least-once + idempotent handlers + event-ID semaphores where
  chosen — eventually consistent BY the designed queue fabric, with every hop
  provenance-linked by `correlation_id`. No second replication mechanism is
  invented: mesh-mediated events ARE the inter-database consistency story,
  which is exactly INTENT #90's contracts-mediated-by-mesh philosophy applied
  to data.
- **Access from anywhere** = resolve the entity slug, mesh relays to the
  home (single-port locality; the caller never knows which node). Reading
  hot remote data repeatedly is a smell that the database wants its anchor
  moved (open question: anchor relocation, below) or wants promoting.

### 11. KG on VDB — the `kg-vdb` seam (VDB's side, proposed; KG designs concurrently)

KG builds ON vdb (locked). From vdb's side the seam is deliberately thin: **a
knowledge graph's storage is an ordinary vdb database whose stack definition
KG authors** (nodes/edges/templates tables; schema-locking IS migrations +
ledger; KG's trigger/handler support IS the execution-engine's kg-nodes
adapter evaluating the same changelog with a node-change subject binding).
vdb offers KG no bespoke graph verbs — KG gets: `EnsureDatabase` (definition
in, database out), the data plane (`Query`/`Exec`), `Invoke`, the changelog
subject stream (via its adapter), snapshots, and promotion — the same surface
every consumer gets. The graph-merge consistency model (the wave's hardest
open problem) is KG's to solve ABOVE this seam; vdb contributes exactly one
useful primitive to it: the per-database single-home rule means a graph
partition/merge is a KG-layer event operating over vdb databases as
replicas-by-KG's-choosing, never a vdb data merge. Proposed shape below
(`kg-vdb`); flagged for mid-batch reconciliation with the KG designer.

### 12. Surface — CLI, dashboard, events (boring on purpose)

- **CLI** (`bin/vdb`, noun-verb like db/mesh): `vdb db create <project>/<name>
  --def <path> [--target local|supabase|aws]`, `vdb db ls|status|open`,
  `vdb def validate|push`, `vdb promote <db> --to supabase --dry-run|--run`,
  `vdb provenance <db> --row <t>/<pk> | --trace <id>`, `vdb handler ls|logs`,
  `vdb snapshot <db>`.
- **Surface schema** (INTENT #46): databases table (target, status, anchor,
  definition version, provenance level, changelog lag, last snapshot),
  per-database handler activity, promotion progress — rendered by the boring
  dashboard, agent-drivable, no bespoke UI.
- **Events** (`pubsub-protocol`, topic prefix `vdb/…`): `vdb.db.created`,
  `vdb.change.applied` (the LISTEN/NOTIFY tee, per-database opt-in),
  `vdb.handler.invoked|failed`, `vdb.promote.phase`, `vdb.snapshot.taken`,
  `vdb.loop.stopped` (mirror of the engine's escalation). Lossy,
  observability-grade; durable integration is queues.

## Relationships / edges

Contract edges (cross-process, via the local `:3649` daemon):

- **vfs** via `vdb-vfs` (rename of `stack-vfs` — flagged) — SQLite files as
  NodeAnchored vfs files: `OpenAnchored` (real OS path + anchor lock),
  `Snapshot` at transaction-consistent checkpoints, retirement. vfs.md
  already proposed the storage side; accepted + refined below.
  *(scaffold/contracts/stack-vfs.md → vdb-vfs)*
- **mesh** via `vdb-mesh` (rename of `stack-mesh` — flagged) — daemon +
  per-database-entity registration, the catalog keyspace tenancy, and the
  (reduced — concern 6) locks usage: promotion mutex + event-ID semaphores.
  *(scaffold/contracts/stack-mesh.md → vdb-mesh)*
- **db** via `vdb-db` — the execution arm: the `db serve` session protocol
  (Driver-trait-shaped verbs) + the promotion copy/verify verbs + changelog
  codegen. NEW pair (wave2-plan §3b). Co-designed with db this batch.
  *(authored: scaffold/contracts/vdb-db.md)*
- **secrets** via `vdb-secrets` — (a) the local-mesh-database push adapter
  (secrets → a stack database's encrypted secret facility — the v1-BUILD
  adapter); (b) cloud-target credentials, use-without-seeing. secrets.md
  proposed its half; accepted with one refinement (below).
  *(authored: scaffold/contracts/vdb-secrets.md)*
- **aws** via `aws-vdb` — the RDS+Lambda cloud target (design-only v1;
  `AwsDisabled` until Live). aws.md proposed its half; accepted (below).
  *(authored: scaffold/contracts/aws-vdb.md)*
- **kg** via `kg-vdb` — KG's graph storage/routing through vdb (concern 11).
  Reconciled with KG mid-batch.
  *(authored: scaffold/contracts/kg-vdb.md)*
- **cc** via `cc-escalation` — consumed, not authored: the
  execution-engine's `LoopDepthExceeded` arm fires from inside vdb's engine
  adapter (queues/cc own the shape; execution-engine authors that arm).

Cross-cutting protocols (one shared document, vdb a party; consumed, not
authored): `queues-api` (stack handlers as `HandlerRef` targets; handlers
enqueue events), `cron-api` (the pg_cron leg — cron.md's worked example),
`locks-api` (promotion mutex; event-ID semaphores), `restart-protocol`
(per-database-entity participation — concern 2), `pubsub-protocol`
(`vdb/…` topics), `surface-schema`, `service-lookup` (daemon NodeScoped +
per-database entities).

Internal-lib seams (compiled in, NOT contract edges — INTENT #29/#45):
`mesh-client`, `substrate-types` (`trigger`, `event`, `provenance`, `error`
vocabulary + the `types::vdb` structs recorded in the authored contracts), **`execution-engine`**
(the stack-tables adapter runs in-process; shares `types::trigger` with
queues by construction), and the Deno embedding (`deno_core`/subprocess pool —
a third-party dependency decision left to fill, either satisfies the sandbox
spec in concern 4).

Stub-track anticipated (named, content deferred with their L6 partners):
`projects-vdb` (databases attached to projects — the catalog's `project`
field is already the join key), `environments-vdb` (environment routes
storage: local→SQLite, cloud→promote — v1's explicit per-database `target`
config is the placeholder this replaces).

## Nesting

Parent: none (top-level app-crate `bin/vdb` + `lib/vdb`). Children (internal
libs compiled into the daemon, never standalone — INTENT #22): **`catalog`**
(DatabaseRecord/StackDef management + the replicated-kv keyspace client),
**`local`** (the SQLite stack daemon: anchor/WAL/changelog tailer/cursor),
**`runtime`** (the Deno worker pools + handler context + SQL-handler
dispatch), **`targets`** (the `StackTarget` trait + local-sqlite/supabase/
aws-rds-lambda adapters), **`promote`** (the copy/verify/switch state
machine), **`provenance`** (the record writer + lineage walker + retention/
archival). The execution-engine is a SHARED lib (sibling of `types`), not a
vdb child — vdb hosts its stack-tables adapter.

## Thoroughness level

**implementation-ready** for: the vocabulary/catalog model (concern 1);
databases-as-virtual-supervised-entities + per-database restart-ladder
participation (concern 2); the `StackTarget` capability matrix (concern 3);
the local target's changelog change-capture, cursor semantics, Deno sandbox +
handler context, and Invoke surface (concern 4); the SQLite-sufficiency
mapping (concern 5); trigger/handler registration-as-data in the ops schema +
the reduced locks scope (concern 6); the in-database provenance schema,
causation threading, and lineage surface (concern 8); the 7-step promotion
state machine with barrier/drain semantics (concern 9); and the
replicate-catalog-not-data consistency split (concern 10).
**approach-sketched** for: the `vdb-db` wire (preferred shape proposed;
db's designer co-resolves this batch); `kg-vdb` (vdb side proposed; KG
co-resolves); the exact Deno embedding (in-process `deno_core` vs subprocess
pool — fill-time, sandbox spec governs either); the Supabase trigger→outbox
compilation details (db's existing machinery bounds it); and everything
behind `aws-vdb` (design-only by mandate). Genuinely open → Open questions.

## Assigned design-depth

**Fable** (wave2-plan tier: one of the six Fable seats; "the highest-stakes
design in this batch"), single Component-Designer pass (this file), grounded
in the batch-1/2/3 designs on disk, INTENT #44–107, and the real `lib/db`
source.

## Suggested fill-model

**implementation-ready + high complexity → strong-mid model, with two
surfaces reserved for a strong hand.** Boring transcription (mid model OK):
the catalog/KV keyspace, the `StackTarget` trait + capability gates, CLI,
surface schema, event tees, the SQLite-sufficiency wiring (each leg is a
sibling lib's designed API). **Tests-first, strong model, never a cheap
tier:** (1) the **changelog tailer + cursor + at-least-once dispatch** loop
(crash-replay correctness; causation_id threading — a silent gap here
corrupts provenance, the module's first-order promise); (2) the **promotion
state machine** (barrier/drain/delta/flip — its failure mode is a split-brain
database; write the abort-path and flip-window tests before the happy path).
The Deno sandbox is transcription against the spec but wants a security
review pass. Fill AFTER db (its `serve` surface), vfs, locks, queues,
replicated-kv, and the execution-engine are filled; alongside kg's early
fixtures.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `vdb-mesh` (vdb ↔ mesh) — registration (per-database supervised entities) + catalog keyspace + locks; RENAMED from `stack-mesh` at harmonization. → `scaffold/contracts/vdb-mesh.md`
- `vdb-vfs` (vdb ↔ vfs) — the SQLite-file-in-VFS anchor/snapshot surface (vfs.md's surface accepted); RENAMED from `stack-vfs` at harmonization. → `scaffold/contracts/vdb-vfs.md`
- `vdb-db` (vdb → db) — the execution arm: the `db serve` session protocol. → `scaffold/contracts/vdb-db.md`
- `vdb-secrets` (vdb ↔ secrets) — push adapter + cloud credentials (secrets.md's half accepted, one refinement). → `scaffold/contracts/vdb-secrets.md`
- `aws-vdb` (vdb → aws) — the RDS+Lambda cloud target; aws.md's half accepted (design-only v1). → `scaffold/contracts/aws-vdb.md`
- `kg-vdb` (kg ↔ vdb) — KG's storage/routing through vdb (the locked-layering edge). → `scaffold/contracts/kg-vdb.md`
- `projects-vdb` / `environments-vdb` — stub-track (anticipated, content deferred). → `scaffold/contracts/projects-vdb.md`, `scaffold/contracts/environments-vdb.md`

Also a party to (cross-cutting): `locks-api`, `queues-api`, `cron-api`, `restart-protocol`, `pubsub-protocol`, `surface-schema`, `service-lookup` — see `scaffold/contracts/`.

## Non-obvious tests (conformance + correctness)

- **Deploy-anywhere conformance (the pattern's promise):** one fixture stack
  definition (tables + 2 triggers + 1 SQL + 1 Deno handler) deployed to
  local-SQLite and Supabase produces IDENTICAL trigger firings, handler
  payloads (per `AssemblyTemplate`), and provenance records for the same
  input row-writes — the cross-target suite from concern 3.
- **Causation threading:** external insert → trigger → Deno handler writes
  table B → second trigger → SQL handler updates table C. Lineage walk from
  C's row returns the FULL chain to the external insert; every changelog row
  the handlers caused carries the correct `causation_id`; `correlation_id` is
  constant across the chain; the execution-engine's loop detector reads the
  same chain and reports depth correctly.
- **Loop guardrail end-to-end:** a deliberately cyclic pair of triggers
  (A-change → handler writes A) stops at the loop-depth threshold, records
  `LoopStopped`, fires ONE `cc-escalation` (LoopDepthExceeded arm, deduped
  by escalation_id), and the database remains serviceable.
- **Crash-replay of the tailer:** kill the daemon mid-dispatch (after
  changelog write, before cursor advance) → on restart the row re-dispatches;
  an idempotent handler yields a single effect; provenance shows one
  completed invocation (the duplicate attempt is visible as a retried
  dispatch, not a second completed record).
- **Promotion barrier semantics (INTENT #86 verbatim):** start a slow Deno
  handler, begin promotion; the handler FINISHES against the old DB; its
  writes appear in the final delta and exist in the cloud target post-switch;
  a trigger firing that arrived during the barrier dispatches exactly once,
  against the NEW database; reads during the barrier served from old until
  the flip.
- **Promotion abort honesty:** corrupt one row in the target between copy and
  verify → `PromotionConflict`, lock released, target parked, local database
  bit-identical to pre-promotion (checksum), status back to `Running`.
- **Flip-window atomicity:** kill vdb between verify and registry flip → on
  restart, promotion state machine resumes or aborts cleanly from the
  replicated `Promoting{phase}` record; at no point do two homes both accept
  writes (the barrier + anchor lock guarantee).
- **Restart ladder on a database entity:** L2 request during active
  invocations drains then yields; L1 during a migration waits
  (`CriticalSection`); L3 produces a consistent VFS snapshot within the
  window; L4 on the entity leaves sibling databases on the same daemon
  untouched.
- **Split-brain anchor guard:** a second vdb leg attempting `OpenAnchored` on
  a hosted database gets `AnchorLocked` and does NOT start a tailer;
  partition-twin anchor merge demotes the LWW-loser to read-only + alarm,
  with the losing side's split-window writes surfaced for reconciliation
  (never silently merged, never silently dropped).
- **Snapshot consistency:** restore the latest VFS snapshot into a fresh
  node → SQLite integrity_check passes, ledger head matches the catalog's
  definition ref, changelog cursor is consistent (no gap, no double-fire on
  resumed tailing).
- **Changelog codegen completeness:** every table added by any migration path
  (including ALTER) carries the three triggers afterward; a table missing
  them is caught by the `apply_definition` post-check, not discovered by a
  silent non-firing trigger.
- **llm_safe pass-through:** a trigger whose assembly includes a secrets
  raw reference, feeding an LLM-classified handler, degrades to
  SecretRef-plus-warning (queues' asserted constraint) — the assembled
  payload NEVER contains plaintext; a Deno handler's context exposes no verb
  that returns a raw secret.
- **Mixed-version tolerance:** an older daemon reading a catalog record with
  an Unknown `StackTargetKind` neither hosts nor deletes it; a newer trigger
  variant fails that trigger's registration loudly and locally (queues'
  rule), leaving sibling triggers live.
- **pg_cron leg end-to-end (cron.md's worked example):** the
  `vdb/analytics/nightly-rollup` job fires once fleet-wide, the trigger
  assembles, the handler runs against the right database, provenance traces
  handler-write → trigger → cron fire event.
