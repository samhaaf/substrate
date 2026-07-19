# db

**Status:** EXISTING app-crate (`lib/db` = `substrate-db`, `bin/db`), substantial
real code. **FULL DESIGN — wave 2, batch 4 (L4).** This pass SUPERSEDES the prior
Sonnet `approach-sketched` file, which is now stale in two load-bearing ways: (1)
it framed `db`'s consumer edges (`db-control-plane`, `db-inference-init`) as
**Cargo library-dependency edges** (`org`/`inference` link `substrate-db`) — that
is directly reversed by **INTENT #29** (no in-process linking across app
boundaries, ever — the rule the operator coined *in response to `db`'s own
designer proposing exactly that*); and (2) it recorded `db` as needing **"no
implementation changes this pass"** — superseded by **INTENT #96** ("we extend db
as necessary to support VDB"), which authorizes the wave-2 refit designed here.
Grounded in the real source (`lib/db/src/{lib,driver/*,handler,outbox,audit,vault,
promote,introspect,config,query}.rs`), `stack.md`/`vdb` (batch-4 neighbor),
`secrets.md` (batch 3), `queues.md`/`locks.md`/`supervision.md` (batch 2),
`vfs.md`/`aws.md` (batch 3), and `types.md` (`Provenance`, `DbError`).

## Charter

`db` is Mind OS's **boring database action-runner**: the single crate that
actually *runs actions against one specific database* — migrations
(author/apply/rollback/status/crawl/lint), the `ops` control-plane baseline,
seed + per-migration fake-data, edge-function bundle/deploy/activate with
pointer-flip rollback, handler-contract parse + codegen (SQL wrapper, TS guard,
validator-sync trigger body), the async outbox + validator audit, ad-hoc query,
schema introspection, catastrophic-recovery snapshot, and forward-only
copy/verify/switch promotion. It presents ONE noun-verb surface over a
`Capabilities`-gated `Driver` trait spanning multiple backends, so an incapable
backend degrades **early and typed** (`DbError::NotImplemented`) rather than
deep. The control plane is one `ops` schema (migration ledger + handler registry
+ outbox + audit + — new this wave — a **provenance ledger**); the ledger is the
sole source of truth.

`db`'s wave-2 role is fixed by the **VFS < VDB < KG** lock (INTENT #96): **`db`
is the execution arm the `vdb` daemon drives to run actions against specific
databases.** VDB *tracks and decides* (the stack pattern, trigger matching,
promotion orchestration, distributed coordination); `db` *runs and records*
(SQL/DDL/edge/handler execution against a resolved backend, with atomic
provenance). `db` stays boring **under** VDB — it gains capabilities, never
orchestration intelligence.

**Boundary — what `db` does NOT own.** It does not decide *which* handler fires
on a row change or assemble handler payloads (that is the declarative
trigger/execution-engine layer in `vdb` — INTENT #101/#103); it does not run
Deno/TS handler code (that is `vdb`/execution-engine + Deno); it does not do
cross-database eventual consistency or the mesh-wide "which node owns this
database" question (that is `vdb` + mesh); it does not own `store` (inference's
own per-node SQLite system-of-record — a separate database, self-migrating, see
`store.md`); it does not own the service registry (mesh); it does not own secret
*material* (that is `secrets` — `db`'s `vault.rs` becomes secrets' Supabase push
adapter, *consumed by* secrets, see below); and it is **never linked into another
app** — it is reached as a **CLI subprocess or a mesh-relayed WS daemon**
(INTENT #29).

Dual-crate like `gc`/`secrets`: `lib/db` (`substrate-db`, every real behavior) +
`bin/db` (the `clap` dispatcher, and — new this wave — the `db serve` daemon).

## Primary design concerns

### 1. How VDB (and everyone) calls `db` — the standing "linked vs subprocess vs daemon" question, resolved

The task's central open question. **INTENT #29 forecloses the linked-lib option**
(it was coined against `db` specifically: "then we'll have to restart the importer
in order to utilize the most recent version of that tool"). `db` is an
**app-crate**, not a shared-lib — so it is NOT the "blessed compiled-in exception"
that `mesh-client`/`types`/`execution-engine` are (those are shared-libs by
kind). That leaves two *wire* shapes, and the resolution is **both, chosen by
call-frequency and boot-safety — never linking:**

- **(a) `bin/db` CLI subprocess (standalone, mesh-free, boot-safe).** Every
  invocation constructs `Db::open` fresh, opens the driver, runs, exits — exactly
  today's model. Used for: **operator/CI** ops; `inference` **fresh-node
  bootstrap** (INTENT #23: the node may come up before mesh relays — a subprocess
  `db migrate up` has no mesh dependency, so it is the boot-safe path — see
  `db-inference-init`); `secrets`' occasional Supabase push (`db vault set`);
  `promote` runs. Structured output via `--format json`. This is "call it as a
  command-line tool" — explicitly blessed by #29.

- **(b) `db serve` — a thin daemon over the mesh (warm, hot-path).** New this
  wave. `db serve` registers the `db` slug with the **local** mesh daemon
  (`mesh-client`, single-port locality `:3649`), publishes a boring
  `SurfaceSchema`, participates in `restart-protocol`/`pubsub-protocol`, and
  relays the **same `lib/db` noun-verb surface over WS** (`db-control-plane`).
  Its reason to exist: it **holds warm `Db` handles** (open driver connections —
  `PgClient`, the `Arc<Mutex<Connection>>` sqlite handle) keyed by database
  identity, so VDB's **handler hot path** (an action per row change) does not pay
  process-spawn + connect + config-parse per action. VDB reaches it over the
  mesh (`vdb-db`); because VDB and its local `db` daemon co-reside on the node,
  this is a local relay through the one port.

  **This is the net-new surface of the wave.** `db` today has no daemon and no
  network surface; `secrets.md` (batch 3) and `stack.md` already *assume* a
  "`db-control-plane` WS call," so the direction is cross-consistent — this
  design makes it real and concrete.

**Single-writer discipline (the load-bearing rule that makes two access shapes
safe).** For any database the daemon has opened (especially **SQLite, which is
single-writer**), the daemon is the sole connection owner; a concurrent
standalone `bin/db` opening the *same* file would create two writers and lock
contention. Rule: **a database that is under mesh management (VDB-registered) is
mutated only through the daemon**; the standalone CLI is for un-daemonized local
`db.toml` databases and for boot-time (pre-daemon) bootstrap. The daemon
enforces its own internal serialization per database (extending the existing
`advisory_lock`/`apply_atomic` xact-lock discipline, and coordinating
*distributed* single-firing via mesh `locks` when VDB asks). Flagged as friction:
the operator should bless the "mesh-managed ⇒ daemon-only writes" rule.

### 2. New driver capabilities VDB needs — the adapter matrix refit

VDB is the deploy-anywhere stack runtime with **three targets: local SQLite /
Supabase / AWS RDS+Lambda** (INTENT #93/#96). `db`'s existing three
`Capabilities`-gated drivers are the **seed of that matrix**; the refit:

| Driver | Wave-2 disposition |
|--------|--------------------|
| `sqlite` | **KEEP + EXTEND** — the local stack backend (the file lives in VFS, materialized locally; see concern 6). Grows **change-capture** + **provenance** + richer **structured introspection**. This is where most new code lands. |
| `supabase-cloud` | **KEEP** — the Supabase VDB target. Already has edge/rls/outbox/registry/promote_target. |
| `supabase-local` | **DEPRECATE** — it wraps Docker (`supabase start/stop`), violating the HARD no-Docker rule (INTENT #72). Its local-stack role is replaced by the **`sqlite` local stack daemon** (VDB local target). See concern 5. |
| `aws-rds` (NEW) | **DESIGN-ONLY / approach-sketched** — a Postgres client to an RDS instance; "edge functions" become **Lambda** deploys performed *through the `aws` crate* over WS (`aws-vdb`), not by `db` shelling AWS. Not built v1 ("we're not doing pretty much anything in AWS right now" — INTENT #105). Its `Capabilities` mirror `supabase-cloud` minus `local_stack`. |

Concretely, the `Driver` trait grows (all defaulting to typed `NotImplemented`,
so incapable drivers degrade cleanly — the existing pattern):

- **Structured introspection.** Today `IntrospectQuery`/`Introspection` return a
  string-rendered `Rows` grid. VDB needs *typed* schema facts — column
  name/type/nullability, PK/FK, unique constraints, indexes — for its stack-table
  model and for **copy/verify** promotion (schema parity + row-count/checksum
  compare). Add `IntrospectQuery::Schema { table } -> TableSchema` (typed struct)
  alongside the existing string grids. Postgres reads the catalog; sqlite reads
  `PRAGMA table_info`/`foreign_key_list`/`index_list`. New `Capabilities` flag:
  none needed — introspection is a core method every driver already implements.

- **Change-capture (the SQLite procedural-trigger equivalent).** SQLite has no
  `LISTEN/NOTIFY` and no plpgsql. Per the SQLite-sufficiency mapping
  (`stack.md`: *procedural triggers → daemon-level handlers*), `db` installs
  **`AFTER INSERT/UPDATE/DELETE` SQLite triggers that write a row into an
  `ops_change_log` table** (op, table, rowid/pk, old/new snapshot, timestamp,
  provenance columns), and exposes **read + acknowledge** over the driver. VDB
  drains `ops_change_log`, matches declarative triggers, and fires Deno handlers —
  acquiring the per-event-ID semaphore via mesh `locks` for ~exactly-once
  (INTENT #95). This mirrors the existing Postgres `ops.handler_dispatch`
  outbox exactly — same shape, sqlite dialect. New: `Capabilities::change_capture`;
  methods `install_change_capture(table, events, mode)`,
  `read_changes(since_seq, limit) -> Vec<ChangeRow>`, `ack_changes(seqs)`. On
  Postgres this is satisfiable by the existing trigger+outbox machinery, so the
  cloud drivers can either keep native triggers or expose the same change-log
  view — the interface is uniform, the dialect is the driver's.

- **Handler-deploy per target (already mostly present).** The existing
  `deploy_edge`/`activate_handler`/`rollback_handler`/`codegen` are exactly
  "deploy a handler to a specific backend." VDB drives them per target: on
  Supabase, `supabase functions deploy` (existing); on SQLite, **SQL handlers
  run in-process and Deno/TS handlers run in VDB's Deno runtime** (so on sqlite
  `deploy_edge` stays `NotImplemented` — the TS handler is NOT a DB-side edge
  function, it's a VDB-hosted Deno handler keyed off `ops_change_log`); on AWS,
  a Lambda deploy through `aws`. The **db-side artifact** VDB consumes is
  `handler::codegen` (the deterministic SQL wrapper + TS guard + trigger body +
  stored contract jsonb) — already built, already deterministic, already
  promote-gated for staleness. No change needed beyond exposing it over the
  daemon.

### 3. Provenance is FIRST-ORDER and it lands atomically in `db` (INTENT #85/#92)

This is the wave's most important *new* first-class `db` surface. VDB is the
provenance **primary home** (per-project/per-database — #92), but the *data touch
itself* happens through `db`. Healthcare-grade provenance ("every time data gets
touched by a handler" — #85) demands the provenance record be written **in the
same transaction as the mutation** — otherwise a crash between the write and its
trace loses provenance, which is exactly the failure a data engineer will not
accept. So:

- Every mutating `db` action (apply-SQL, apply-migration, run-handler-effect,
  edge-fire, outbox-drain step) accepts a **`Provenance`** context
  (`types::provenance::Provenance` — `origin_node`, `origin_service`,
  `emitted_at`, `causation_id`, `correlation_id`, `hops`) threaded from the
  caller (VDB stamps it; the event/envelope already carries it — one vocabulary).
- `db` writes a row into a new **`ops.provenance` / `ops_provenance`** ledger
  **inside the same `apply_atomic` transaction** as the mutation: `(seq, env,
  table, op, correlation_id, causation_id, actor_service, handler, at, summary)`.
  This makes provenance **non-optional and non-losable** — the atomic guarantee
  is the reason it lives in `db`, not only in VDB's tracker.
- `db` exposes read/query over the provenance ledger (`db provenance …` verb /
  WS) so VDB can assemble the causal chain and the dashboard can render "how did
  this row get here." VDB's loop-detection (INTENT #70) reads `causation_id`
  chains; `db` supplies the atomic per-touch facts.
- This is the deep reason `db` cannot stay "no changes": provenance threading is
  a new argument on the core action surface + a new ops table. Scoped: the
  **ledger + atomic write + read** are implementation-ready here; the *cross-
  database aggregation and causal-chain assembly* are VDB's (`vdb.md`).

### 4. Query / virtualization surface — the one standardized query interface (INTENT #60)

INTENT #60 upgraded "db grows a query layer" to in-scope direction: "query
through the db crate and get a standardized interface to the different backends —
SQLite files in the VFS, Supabase projects, RDS instances." The mechanism is
already latent: `Driver::query -> Rows` is a uniform result across backends; the
`Capabilities` gate is the "decide it only needs SQLite" decision. Wave-2
formalizes it as the **`db-control-plane` query facet**: a caller sends
`{ database, sql, params, write }` and gets back a `Rows` grid (v1: cells as
`Option<String>`, the Postgres-text-protocol rendering; **structured typed cells
are the named growth path, EXT**). "Virtualization" = `db` resolves the
**database handle → driver → runs** — the caller never names a backend, only a
database. The read/write gate (`query::is_write`, `--write`) and the protected-ref
guard remain the safety spine. This is a *formalization*, not new engine work.

### 5. The `supabase-local` Docker deprecation (INTENT #72)

`supabase_local.rs` shells `supabase start`/`stop`/`db reset` → Docker; the
`vault_it`/`local_it` tests are Docker-gated. The HARD no-Docker rule (INTENT
#72, reaffirmed round-9) means **the local stack is the SQLite daemon, not
Supabase-in-Docker.** Disposition: **retire `supabase-local` as the local dev
backend** — its capabilities (edge deploy, real Postgres locally) are either (a)
served by the **SQLite local stack** (SQL + VDB-hosted Deno handlers, the whole
point of the stack pattern being FastAPI/Supabase-weight-free), or (b) deferred
to a **cloud** Supabase target for the genuinely Postgres-only needs (INTENT #82
"never need local Postgres" — going past SQLite means promoting to cloud). The
driver code can remain in-tree marked deprecated (so existing Postgres-dialect
introspection/edge SQL that `supabase-cloud` shares stays available), but it is
**not a selectable local target** in a Mind OS deployment. Flagged: the operator
may want it fully removed vs. kept behind a `--allow-docker` dev escape hatch —
recommend removed, matching the emphatic rule.

### 6. SQLite-file-in-VFS: `db` stays a path-opener; VFS residency is VDB's job

The stack pattern stores each SQLite file **in the VFS** (`vdb-vfs`). But SQLite
must `mmap`/lock a **real local file** — it cannot run against a distributed
object store directly. Layering resolution (keeps `db` boring and preserves L4>L3
direction, avoids `db` gaining a VFS edge): **VDB (with VFS) materializes the
database file on the local node's disk and hands `db` a local path**; `db`'s
`sqlite` driver opens that path exactly as it does today (`Connection::open`).
`db` does **not** talk to VFS. Replication/eviction/overflow of the file is
VFS's concern, coordinated by VDB. So the `db`↔VFS edge does **not exist** — a
deliberate non-edge, flagged so the harmonizer does not invent one. (Consistency:
`store.md` and `stack.md` treat the local SQLite path the same way.)

### 7. Secrets reconciliation — `vault.rs` becomes secrets' Supabase adapter, consumed over the wire

`lib/db/src/vault.rs` is a **real, working Supabase-Vault module** (not a stub):
`set`/`list`/`get`/`remove`/`exists` as bound-param SQL against the
`supabase_vault` extension's `vault` schema, plus `reference_sql` (renders the
canonical `(SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name=…)`
lookup so migrations/handlers reference a secret by name and never store
plaintext). It flows over the `Driver` seam (drives `supabase-local` +
`supabase-cloud`; `sqlite` degrades to typed `NotImplemented`). Reconciliation
(INTENT #99/#105, aligned with `secrets.md` concern 5):

- **`secrets` becomes the owner of secret material; `db`'s `vault.rs` IS secrets'
  Supabase push adapter** — leverage, do NOT rebuild. `secrets` drives it **over
  the wire** (`db vault set` CLI / a `db-secrets` WS call), **never by linking
  `substrate-db`** (INTENT #29). See `db-secrets` below.
- **`db`'s own keychain reads reconcile toward secrets-mediated.** Today
  `supabase_cloud.rs` fetches the Management-API PAT "from the OS keychain at
  runtime" and `config.rs` documents "secrets never live here." Direction: those
  direct keychain reads become **`secrets`-mediated resolve-into-sink** (secrets
  injects the PAT/connection credential; `db` receives a `SecretRef` and a
  use-verb, never the raw value in an LLM-reachable path). Because `db` is a
  non-LLM caller, raw resolution is allowed for it — but routing through
  `secrets` gives one audited source of truth. This is the operator-authorized
  "one of the times we actually update the db crate" (INTENT #99). Scoped as
  **direction** here; the keychain→secrets cutover lands when both daemons exist.

### 8. Promotion primitives (copy/verify/switch) — `db` provides, VDB orchestrates

INTENT #86/#96: SQLite→cloud promotion is copy/verify/switch under a lock; during
a critical upgrade, let running edge functions finish, swap underneath, write
back, resume. That **orchestration is VDB's** (it treats databases like services
under mesh's restart/upgrade protocol — `supervision.md`, mesh `locks`). `db`
supplies the **primitives**: `snapshot` (exists — full-DB catastrophic snapshot),
apply-to-target (migrations against the cloud target), **verify** (structured
introspection parity + row-count/checksum compare, concern 2), and the
`advisory_lock`/`apply_atomic` transaction discipline. The existing forward-only
`promote` gate (lint-clean + codegen-fresh + crawl-attestation-matched +
`assert_promote_target`) is retained as the safety gate on the cloud write. `db`
never decides *when* to switch — it exposes the verbs; VDB sequences them under a
mesh lock and flips the mesh registry entry (local→cloud). Local-only promotion
(SQLite→SQLite) is out of scope — promotion always means local→cloud (round-9).

## Relationships / edges

- **vdb** (consumer) via **`vdb-db`** *(MISSING — proposed below)* — the VDB
  daemon drives `db` to run actions against a specific database: apply SQL/DDL,
  run migrations, deploy/activate handlers, install + drain change-capture,
  structured introspection, provenance write/read, promotion primitives. **Over
  the mesh WS (`db serve`), never linked** (INTENT #29). The hot path; the reason
  for the daemon.
- **consumers** (operators/CI/`org`) via **`db-control-plane`** *(exists —
  content proposed below)* — the noun-verb control plane (migrate/edge/query/
  seed/outbox/audit/handler/promote/provenance). **CLI subprocess or daemon WS,
  never linked** (supersedes the prior file's "Cargo dependency edge" framing).
- **inference** (consumer) via **`db-inference-init`** *(exists — content
  proposed below)* — fresh-node DB bootstrap (sqlite driver, ledger-only baseline
  `OPS_BASELINE_SQLITE` + `migration::apply`). **`bin/db` CLI subprocess** (the
  boot-safe path, no mesh dependency), superseding the prior "library dependency"
  framing.
- **secrets** (consumer) via **`db-secrets`** *(MISSING — proposed below)* —
  `secrets` drives `db`'s `vault.rs` as its Supabase push adapter, over WS/CLI,
  never linked. `db` also *becomes a consumer of `secrets`* for its own
  credential/keychain resolution (concern 7, direction) — the same edge, both
  directions of trust flow.
- **mesh** (registration) — `db serve` registers via `mesh-client`/`service-lookup`
  and rides `pubsub-protocol`/`restart-protocol`/`surface-schema` like every
  daemon. This is the universal seam, not a `db`-specific contract (no `db-mesh`
  stub — carried by the cross-cutting protocol contracts).
- **NON-edges (deliberate, flagged so the harmonizer does not invent them):**
  `db`↔`vfs` (VDB materializes the SQLite file locally and hands `db` a path —
  concern 6); `db`↔`aws` directly (the AWS RDS/Lambda target is reached *through*
  the `aws` crate, surfaced to `db` as another driver whose network calls route
  via `aws` over `aws-vdb`, not a `db`-owned edge).

## Nesting

Parent: none (top-level L4 app-crate). Children: none. `lib/db` (`substrate-db`,
all behavior incl. `vault.rs` adapter, drivers, the new provenance/change-capture
modules) + `bin/db` (the `clap` CLI **and** the `db serve` daemon). The lib/bin
split is the existing internal convention, not a scaffold nesting.

## Thoroughness level

**implementation-ready** for the refit's *shape* — the access model (CLI
subprocess + `db serve` daemon, never linked), the driver-matrix disposition, the
provenance-atomic-in-`ops` design, change-capture on sqlite, structured
introspection, the query/virtualization formalization, the `supabase-local`
deprecation, the SQLite-in-VFS non-edge, and the secrets reconciliation are all
specified against real code and buildable as written. **approach-sketched** for:
the `aws-rds` driver (design-only by mandate — INTENT #105), the exact
provenance-context threading through every existing call site (a mechanical but
wide edit — flagged as migration cost), and the keychain→secrets credential
cutover (direction, lands when both daemons exist).

## Assigned design-depth

Opus (single Component-Designer pass, this file), per the wave2-plan model tier
(`db` = Opus), grounded in the real `lib/db`/`bin/db` source, the batch-4
neighbor `stack.md`/`vdb`, batch-3 `secrets.md`, and batch-2
`queues.md`/`locks.md`/`supervision.md`.

## Suggested fill-model

**implementation-ready + medium complexity → strong-mid model.** The bulk is
transcription-grade against existing patterns (the daemon is a `mesh-client`
register + a WS dispatch over the already-factored `lib/db` API; change-capture
mirrors the existing outbox; structured introspection mirrors existing
introspection). **Two areas need care and tests-first:** (1) **provenance
atomicity** — the provenance row MUST commit-or-rollback with its mutation;
conformance-test crash-between-write-and-trace to prove no orphaned mutation
lacks provenance; (2) **single-writer discipline** — test that a mesh-managed
SQLite database rejects/serializes a concurrent standalone writer. Do NOT let a
cheap model hand-wave the provenance transaction boundary — a lost trace is a
silent correctness failure the operator explicitly will not tolerate.

---

## Proposed contracts (wave 2)

Proposals only — the per-pair round reconciles; I do NOT edit
`scaffold/contracts/*`. Shared vocabulary lands in `types` (`types::db` for the
DB surface structs; `DbError` in `types::error::db`). The `Driver`/`Capabilities`
traits are an **internal `lib/db` boundary, NOT a contract**. `db` is reached
only over CLI/WS (INTENT #29); every edge below is a wire edge.

Shared vocabulary referenced by the edges (proposed additions to `types::db`):

```rust
// types::db
pub struct DatabaseId { pub project: String, pub db: String }   // VDB-managed database
// (a plain db.toml env name is the standalone-CLI addressing form; the daemon maps both)
pub struct Rows { pub columns: Vec<String>, pub rows: Vec<Vec<Option<String>>> } // v1 string cells; typed cells EXT
pub struct TableSchema {                                         // structured introspection (concern 2)
    pub table: String,
    pub columns: Vec<ColumnDef>,       // name, sql_type, nullable, default
    pub primary_key: Vec<String>,
    pub foreign_keys: Vec<ForeignKey>,
    pub unique: Vec<Vec<String>>,
    pub indexes: Vec<IndexDef>,
}
pub struct ChangeRow {                                          // sqlite change-capture (concern 2)
    pub seq: i64, pub table: String, pub op: ChangeOp,          // Insert|Update|Delete
    pub pk: serde_json::Value, pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>, pub at: DateTime<Utc>,
    pub provenance: Provenance,
}
pub struct ProvenanceRow {                                      // the ops.provenance ledger row (concern 3)
    pub seq: i64, pub table: String, pub op: ChangeOp,
    pub correlation_id: Option<Uuid>, pub causation_id: Option<Uuid>,
    pub actor_service: String, pub handler: Option<String>,
    pub at: DateTime<Utc>, pub summary: String,
}
```

```rust
// types::error::db  (DbError — replaces today's flat SubstrateError::Db(String) leaves)
pub enum DbError {
    NotImplemented { command: &'static str, driver: &'static str, reason: &'static str },
    NoSuchDatabase { id: String },
    ProtectedRefused { op: String, env: String },          // the prod guard (existing behavior, typed)
    PromoteGateBlocked { blockers: Vec<String> },
    MigrationFailed { id: String, detail: String },
    LintNotClean(Vec<String>),
    Backend(String),                                       // driver/SQL error, stringified at the boundary
    Config(String),
}
```

### `vdb-db` (vdb → db) — the execution arm (NEW)

- **Purpose.** The VDB daemon drives `db` (over the local mesh, `db serve`) to run
  actions against a specific `DatabaseId`: apply SQL/DDL + migrations, deploy/
  activate/rollback handlers per target, install + drain **change-capture**,
  **structured introspection**, **provenance** write/read, and the promotion
  primitives (snapshot/apply/verify). Cross-app, **WS over mesh, never linked**
  (INTENT #29). This is the hot path — the reason `db serve` holds warm driver
  connections.
- **Message/struct sketch** (WS request/response over `pubsub-protocol`):
  ```rust
  enum VdbDbReq {
      Apply    { db: DatabaseId, sql: String, params: Vec<SqlParam>, prov: Provenance },
      ApplyAtomic { db: DatabaseId, statements: Vec<String>, ledger: AppliedMigration, prov: Provenance },
      Query    { db: DatabaseId, sql: String, params: Vec<SqlParam>, write: bool } , // -> Rows
      Introspect { db: DatabaseId, q: IntrospectReq },                                // -> TableSchema | Rows
      InstallChangeCapture { db: DatabaseId, table: String, events: Vec<ChangeOp> },
      ReadChanges { db: DatabaseId, since_seq: i64, limit: u32 },                     // -> Vec<ChangeRow>
      AckChanges  { db: DatabaseId, seqs: Vec<i64> },
      DeployHandler { db: DatabaseId, contract_jsonb: Value, bundle: Option<Bundle> },// per-target
      ActivateHandler { db: DatabaseId, handler: String, version: String },
      ReadProvenance { db: DatabaseId, filter: ProvFilter },                          // -> Vec<ProvenanceRow>
      // promotion primitives:
      Snapshot { db: DatabaseId, out: String },
      VerifyParity { src: DatabaseId, dst: DatabaseId },                              // -> ParityReport
  }
  ```
- **Error cases.** `DbError::NoSuchDatabase` (VDB named an unopened DB),
  `NotImplemented { command, driver }` (e.g. `deploy_edge` on `sqlite` — the TS
  handler is VDB-hosted Deno, not a DB edge; VDB expects and handles this),
  `MigrationFailed`, `Backend`. The **partition/single-writer** concern surfaces
  as a `locks`-domain error when VDB requested distributed coordination — carried
  by `locks-api`, not re-typed here.
- **Version-sensitivity.** Medium. `SqlParam`/`Rows`/`AppliedMigration` are
  stable existing shapes; `VdbDbReq` grows additively (`#[serde(other)]`
  reserved) as VDB's target matrix widens (RDS/Lambda). Provenance rides
  `types::Provenance`'s frozen shape. Structured `TableSchema` typed-cells are the
  EXT growth path — additive.

### `db-control-plane` (db ↔ consumers) — the noun-verb control plane (EXISTS; content proposed)

- **Purpose.** The full operator/CI/`org` control plane: migrate (author/apply/
  rollback/status/crawl/lint), edge (bundle/deploy/activate/rollback/sync),
  **query (the standardized virtualization surface, INTENT #60)**, seed/fakedata,
  outbox (list/retry/drain), audit, handler (new/codegen/activate/rollback),
  promote, snapshot, provenance read, doctor. **Reached two boring ways — `bin/db`
  CLI subprocess (standalone, mesh-free) or the `db serve` daemon over WS —
  NEVER by linking `substrate-db`** (INTENT #29; this supersedes the prior file's
  "Cargo dependency edge" framing).
- **Message/struct sketch.** The CLI shape is authoritative (the existing `clap`
  tree in `bin/db/src/cli.rs`); the WS surface is the same verbs as a
  request/response envelope. `org`'s self-restructuring KG (INTENT #19) is
  ordinary migrations + queries against `db`-owned schema over this edge — `db`
  does not model the graph; `org`/`kg` do.
  ```rust
  enum DbCtlReq { Migrate(MigrateReq), Edge(EdgeReq), Query(QueryReq),
                  Seed(SeedReq), Outbox(OutboxReq), Audit(AuditReq),
                  Handler(HandlerReq), Promote(PromoteReq), Snapshot(SnapshotReq),
                  Provenance(ProvReq), Doctor }
  struct QueryReq { database: String, sql: String, params: Vec<SqlParam>, write: bool } // -> Rows
  ```
- **Error cases.** `DbError::ProtectedRefused` (the prod guard — mutating migrate
  on a protected ref without the exact `--i-understand-prod <ref>`),
  `PromoteGateBlocked { blockers }`, `LintNotClean`, `NotImplemented`, `Config`.
  The read/write gate refuses a write `Query` without `write:true`.
- **Version-sensitivity.** Low-medium. The noun-verb surface is stable and
  additive; the WS envelope versions via the standard `pubsub` `v` field. The
  `Rows` string-cell rendering is frozen for v1; typed cells are an additive EXT.

### `db-inference-init` (inference → db) — fresh-node bootstrap (EXISTS; content proposed)

- **Purpose.** When an `inference` node stands up on a **fresh mesh node**, it
  initializes its local database through `db` rather than hand-rolling bootstrap:
  the **`sqlite` driver, ledger-only baseline** (`OPS_BASELINE_SQLITE`) +
  `migration::apply`. Narrow slice — no edge functions, no local stack, no
  promote (those `Capabilities` are already `false` for sqlite).
- **Access shape — the boot-safe correction.** This is a **`bin/db` CLI
  subprocess** call (`db --env <node> migrate up`), **not** a linked-lib call
  (superseding the existing stub's "library dependency edge" wording — INTENT #29)
  and **not** necessarily a daemon WS call: on a cold node, mesh may not yet
  relay, and the db daemon may not yet be up, so the subprocess (which needs no
  mesh) is the reliable bootstrap path. Once the node is warm, subsequent DB
  access can move to the daemon. Relationship to `store`'s own self-migrating
  schema (`store.md`) is flagged for the harmonizer — they are distinct databases
  (control-plane `ops` vs. the inference system-of-record); this edge is the
  `ops`/control-plane bootstrap, not `store`'s.
- **Error cases.** `MigrationFailed { id, detail }` on a failed first-boot apply
  (inference decides degrade-to-standalone per `inference.md`); a second
  invocation against an already-migrated node is an **idempotent no-op** (ledger
  dedups) — the conformance requirement.
- **Version-sensitivity.** Low. The sqlite baseline + `migration::apply` path is
  stable; the edge exercises the narrowest, most stable slice of `db`.

### `db-secrets` (db ↔ secrets) — the Supabase Vault push adapter, reused over the wire (NEW)

- **Purpose.** Two flows over one edge. **(a) secrets → db:** `secrets` drives
  `db`'s existing `vault.rs` as its **Supabase push adapter** (INTENT #105 —
  reuse, do NOT rebuild), pushing a secret's value into a Supabase project's
  `vault.secrets` via `db vault set`/`rm`/`exists` and rendering `reference_sql`
  so handlers reference the secret at runtime without storing plaintext — **over
  `db-control-plane` WS or the `db vault set` CLI, never by linking
  `substrate-db`** (INTENT #29). **(b) db → secrets (direction, concern 7):** `db`
  reconciles its own credential/keychain reads (the `supabase-cloud` PAT, cloud
  connection strings) to be **`secrets`-mediated**; `db` is a non-LLM caller so
  raw resolution is permitted, giving one audited source of truth.
- **Message/struct sketch.**
  ```rust
  // (a) secrets -> db  (maps onto db vault set/rm/exists over db-control-plane)
  struct DbVaultPush   { env: String, name: String, value: SecureValue, description: Option<String> } // -> SetOutcome
  struct DbVaultRemove { env: String, name: String }
  struct DbVaultExists { env: String, name: String }  // -> bool
  // reference_sql is pure codegen: (name) -> String, no wire value
  // (b) db -> secrets  (concern 7 direction): db resolves a SecretRef into a sink
  //     — the CallerContext/SecretRef vocabulary is owned by `secrets` (secrets.md concern 4)
  ```
  `SecureValue` = the plaintext crossing a non-LLM, in-mesh confidential WS
  channel (secrets→db-vault); it never transits an LLM path (aligns with
  `secrets.md` `db-secrets`).
- **Error cases.** `DbError::NotImplemented { command:"vault set", driver:"sqlite" }`
  wrapped by secrets as `AdapterFailed{Supabase,…}` (the sqlite driver has no
  Vault — the SQLite equivalent is secrets' own local-mesh-db adapter, `vdb-secrets`,
  NOT this edge); SQL/connection errors stringified as `DbError::Backend`.
- **Version-sensitivity.** Low. `vault.rs`'s surface is stable bound-param SQL; the
  adapter is a thin WS/CLI shim. The (b) keychain→secrets cutover is an additive
  `db` change (operator-authorized, INTENT #99), tracked here as direction.
