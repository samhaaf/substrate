# db

> **SESSION HOME DECIDED — re-spoken round (2026-07-21/22, INTENT #167):
> sessions live in VDB; `db` returns to a PURE CLI TOOL with NO DAEMON.
> `db serve` is SUPERSEDED.** The operator delegated the db-vs-vdb
> session-home call ("just pick one" — stop re-raising), and this is the
> pick, taken as the simplest shape: INTENT #128 already gave vdb
> "statement tracking, cleanup, and session management," so putting the
> session/daemon concept in db too split one concern across two processes
> on the same node; collapsing it into vdb gives the session concept ONE
> home, deletes db's second public surface (one crate, one noun-verb CLI —
> maximally boring), and honors the operator's original instinct (INTENT
> #115: "db = a standalone crate you call as a tool; VDB = the
> mesh-accessed service" — daemons are services, db is a tool). vdb holds
> the warm driver connections itself (see vdb.md concern 7's superseding
> note for the mechanism latitude); db stays exactly what it is today — a
> boring CLI action-runner invoked as a subprocess (path (a) below), used
> by vdb for cold-path actions, by inference bootstrap, by CI, and by mesh
> for its own database. The `db serve` design below (concern 1 path (b),
> and the INTENT #128 acceptance notes) is retained as record; the
> capability extensions the daemon motivated (changelog codegen,
> copy/verify verbs, outbox parity) survive as CLI/lib capabilities.
> → `contracts/vdb-db.md` (superseded-as-daemon-protocol note), `vdb.md`
> concern 7.

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

`db` is Substrate's **boring database action-runner**: the single crate that
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
app** — it is reached **only as a `bin/db` CLI subprocess** (INTENT #29; the
`db serve` daemon is retired — concern 1).

Dual-crate like `gc`/`secrets`: `lib/db` (`substrate-db`, every real behavior) +
`bin/db` (the `clap` dispatcher). **No daemon, no network surface, no mesh
dependency** — a pure tool (INTENT #167).

## Primary design concerns

### 1. How VDB (and everyone) calls `db` — the standing "linked vs subprocess vs daemon" question, resolved

The task's central open question — **RESOLVED at the re-spoken round (INTENT
#167): `db` is a pure CLI tool, no daemon; the warm sessions live in `vdb`.**
INTENT #29 forecloses the linked-lib option (it was coined against `db`
specifically: "then we'll have to restart the importer in order to utilize the
most recent version of that tool"); `db` is an **app-crate**, not a shared-lib,
so it is NOT a "blessed compiled-in exception" like `chassis`/`types`/
`execution-engine`. And the round-1 `db serve` daemon is now **retired** — its
only reason to exist was to hold warm driver connections for `vdb`'s hot path,
and those connections now live in `vdb` itself (`vdb.md` concern 7). So there is
exactly **one** shape `db` presents, and the boring split across the `vdb→db`
boundary is stated from vdb's side:

- **(a) The cold path — `bin/db` CLI subprocess (standalone, mesh-free,
  boot-safe) — the ONLY shape `db` presents.** Every invocation constructs
  `Db::open` fresh, opens the driver, runs, exits — exactly today's model.
  `vdb` spawns it for every control-plane action that reuses `db`'s proven,
  engine-aware logic: migrations (apply/rollback/status/lint/crawl), **changelog
  codegen** (`install-changelog`), promotion primitives (`dump-to`/
  `restore-from`/`content-hash`/`snapshot`), structured introspection
  (`introspect --schema`), edge/handler deploy to cloud targets. Also used —
  unchanged — by **operator/CI** ops; `inference` **fresh-node bootstrap**
  (INTENT #23: the node may come up before mesh relays — a subprocess `db
  migrate up` has no mesh dependency, so it is the boot-safe path — see
  `db-inference-init`); `secrets`' occasional Supabase push (`db vault set`);
  and **mesh** managing its own kernel database. Structured output via `--format
  json`. This is "call it as a command-line tool" — explicitly blessed by #29,
  and it is the whole of the `vdb-db` contract.

- **(b) The hot path is NOT db's — it is `vdb`'s warm session.** The per-row
  runtime (drain changelog / run SQL handler / serve a Deno handler's
  `db.query/exec` / write the provenance row) runs against a **warm driver
  connection `vdb` holds itself**, keyed by hosted database — "the session,"
  now living in `vdb` (INTENT #167). `vdb` opens it through the same public
  driver crates `db` uses (`rusqlite` / `tokio-postgres`), NOT by linking
  `substrate-db` (INTENT #29). So `db` pays no per-action process-spawn cost by
  *not being on the hot path at all*; the warm-handle rationale that once
  motivated `db serve` is satisfied by the session's new home. The capability
  extensions the daemon once motivated (changelog codegen, copy/verify verbs,
  outbox parity, structured introspection) survive intact — as **CLI verbs /
  lib capabilities** reached over path (a), never a socket.

  **`db serve` is retired (record).** The round-3 acceptance (INTENT #128) of a
  headless stateful session daemon "as part of db" was superseded at the
  re-spoken round: the session concept has ONE home (vdb), which deletes db's
  second public surface, its mesh registration, and the two-WS-hops-per-action
  cost. The operator's division still holds verbatim — *"VDB is basically just a
  virtualized layer… VDB just has to keep track of which statements it has
  running and do proper cleanup and session management"* (#128) — it is simply
  realized by vdb holding the connections rather than delegating to a db daemon.

**Single-writer discipline (the load-bearing rule).** **SQLite is single-writer.**
For a mesh-managed database, **`vdb` is the sole owner of the file** — it holds
the `vfs` anchor lock (no other node's `vdb` can open it) and its warm session
is the one hot-path writer. A cold-path `bin/db` subprocess (a migration) opens
its own transient connection to the *same* OS path; the rule that keeps them
safe is **`vdb` quiesces its own writes for the subprocess's duration** (drives
the entity to `CriticalSection`, drains in-flight, pauses the warm connection —
`vdb.md` concern 7). So two connections never write concurrently; `db`'s
existing `advisory_lock`/`apply_atomic` xact discipline plus SQLite's own file
locking are the backstop. The standalone CLI is otherwise for un-daemonized
local `db.toml` databases and boot-time bootstrap. *(No operator sign-off is
pending here — the re-spoken decision settled it; the old "mesh-managed ⇒
daemon-only" friction flag is dissolved along with the daemon.)*

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
  promote-gated for staleness. No change needed beyond exposing it as a `bin/db`
  CLI verb vdb invokes as a subprocess.

### 3. Provenance is FIRST-ORDER and it lands atomically in `db` (INTENT #85/#92)

This is the wave's most important *new* first-class provenance surface. VDB is
the provenance **primary home** (per-project/per-database — #92). Healthcare-grade
provenance ("every time data gets touched by a handler" — #85) demands the
provenance record be written **in the same transaction as the mutation** —
otherwise a crash between the write and its trace loses provenance, which is
exactly the failure a data engineer will not accept.

**Who owns the transaction, after the wave-3 db split (concern 1).** The atomic
guarantee moves with the *connection owner*: (i) on the **runtime hot path**,
the mutation runs on **vdb's own warm session**, so vdb opens the transaction
and INSERTs the `ops.provenance` / `ops_provenance` row before commit (`vdb.md`
concern 8); (ii) on the **cold path**, a mutating `bin/db` action (apply-SQL,
apply-migration, edge-fire, outbox-drain step) writes its provenance/ledger row
atomically inside its own subprocess transaction. **`db` owns the ops-schema
*shape*, the atomic-write *discipline*, and the read/query surface** — installed
and reused identically by both owners — even though the per-handler-touch write
is now *executed* by vdb. The invariant is one and the same regardless of owner.
So:

- Every mutating action (whether run by vdb's warm session or a `bin/db`
  subprocess) carries a **`Provenance`** context (`types::provenance::Provenance`
  — `origin_node`, `origin_service`, `emitted_at`, `causation_id`,
  `correlation_id`, `hops`); vdb stamps it, the event/envelope already carries
  it — one vocabulary. `db`'s cold-path verbs accept it as an argument; vdb's
  hot-path session sets it on the connection.
- The row lands in the **`ops.provenance` / `ops_provenance`** ledger **inside
  the same transaction** as the mutation: `(seq, env, table, op, correlation_id,
  causation_id, actor_service, handler, at, summary)`. This makes provenance
  **non-optional and non-losable** by construction — the atomic guarantee is
  independent of which process owns the transaction (concern 1 split).
- `db` exposes read/query over the provenance ledger (`db provenance …` CLI verb)
  so vdb can assemble the causal chain and the dashboard can render "how did this
  row get here." VDB's loop-detection (INTENT #70) reads `causation_id` chains
  from the same ledger.
- This is the deep reason `db` cannot stay "no changes": it must define the
  provenance **ops-schema + the atomic-write pattern** its cold-path verbs use
  and vdb's session reuses. Scoped: the **ledger shape + db's own atomic write +
  read** are implementation-ready here; the *runtime per-handler-touch write* is
  vdb's session (`vdb.md` concern 8) and the *cross-database aggregation /
  causal-chain assembly* are VDB's.

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
**Bearer after the wave-3 split:** the `db query` verb is a `bin/db` subprocess —
the operator/CI/ad-hoc virtualization surface (query any backend by database
name). The **runtime handler hot path does NOT use it** — that query/exec runs
on vdb's own warm session (concern 1); `db query` is for out-of-band access, not
per-row dispatch.

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
**not a selectable local target** in a Substrate deployment. Flagged: the operator
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

### 7. Secrets reconciliation — `vault.rs` becomes secrets' Supabase adapter, consumed as a subprocess

`lib/db/src/vault.rs` is a **real, working Supabase-Vault module** (not a stub):
`set`/`list`/`get`/`remove`/`exists` as bound-param SQL against the
`supabase_vault` extension's `vault` schema, plus `reference_sql` (renders the
canonical `(SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name=…)`
lookup so migrations/handlers reference a secret by name and never store
plaintext). It flows over the `Driver` seam (drives `supabase-local` +
`supabase-cloud`; `sqlite` degrades to typed `NotImplemented`). Reconciliation
(INTENT #99/#105, aligned with `secrets.md` concern 5):

- **`secrets` becomes the owner of secret material; `db`'s `vault.rs` IS secrets'
  Supabase push adapter** — leverage, do NOT rebuild. `secrets` drives it **as a
  `bin/db` CLI subprocess** (`db vault set …`), **never by linking `substrate-db`**
  (INTENT #29; db has no daemon to call — concern 1). See `db-secrets` below.
- **`db`'s own keychain reads reconcile toward secrets-mediated.** Today
  `supabase_cloud.rs` fetches the Management-API PAT "from the OS keychain at
  runtime" and `config.rs` documents "secrets never live here." Direction: those
  direct keychain reads become **`secrets`-mediated resolve-into-sink** (secrets
  injects the PAT/connection credential; `db` receives a `SecretRef` and a
  use-verb, never the raw value in an LLM-reachable path). Because `db` is a
  non-LLM caller, raw resolution is allowed for it — but routing through
  `secrets` gives one audited source of truth. This is the operator-authorized
  "one of the times we actually update the db crate" (INTENT #99). Scoped as
  **direction** here; the keychain→secrets cutover lands when `secrets` is live.

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

- **vdb** (consumer) via **`vdb-db`** *(authored: scaffold/contracts/vdb-db.md)* — **wave-3
  reshaped to a CLI-subprocess contract:** vdb spawns `bin/db <verb> --format
  json` for cold-path control-plane actions against a specific database — apply
  SQL/DDL, run migrations, deploy/activate cloud handlers, install change-capture
  (changelog codegen), structured introspection, promotion primitives. **CLI
  subprocess, never linked** (INTENT #29). The runtime hot path is NOT this
  contract — it is vdb's own warm session (concern 1).
- **consumers** (operators/CI) via **`db-control-plane`** *(exists —
  authored: scaffold/contracts/db-control-plane.md)* — the noun-verb control plane (migrate/edge/query/
  seed/outbox/audit/handler/promote/provenance). **CLI subprocess only, never
  linked, never a daemon WS** (supersedes both the prior file's "Cargo dependency
  edge" framing and the interim `db serve` framing).
- **inference** (consumer) via **`db-inference-init`** *(exists — content
  authored: scaffold/contracts/db-inference-init.md)* — fresh-node DB bootstrap (sqlite driver, ledger-only baseline
  `OPS_BASELINE_SQLITE` + `migration::apply`). **`bin/db` CLI subprocess** (the
  boot-safe path, no mesh dependency), superseding the prior "library dependency"
  framing.
- **secrets** (consumer) via **`db-secrets`** *(authored: scaffold/contracts/db-secrets.md)* —
  `secrets` drives `db`'s `vault.rs` as its Supabase push adapter, **as a `bin/db`
  CLI subprocess** (no daemon), never linked. `db` also *becomes a consumer of `secrets`* for its own
  credential/keychain resolution (concern 7, direction) — the same edge, both
  directions of trust flow.
- **mesh** — **no registration, no edge.** `db` is a pure CLI tool with no
  daemon (INTENT #167), so it never registers on the mesh, publishes no surface
  schema, and participates in no `restart-protocol`. Anything that needs `db`
  spawns `bin/db`. (This deletes the interim `db serve` mesh-participation of the
  prior draft.)
- **NON-edges (deliberate, flagged so the harmonizer does not invent them):**
  `db`↔`vfs` (VDB materializes the SQLite file locally and hands `db` a path —
  concern 6); `db`↔`aws` directly (the AWS RDS/Lambda target is reached *through*
  the `aws` crate, surfaced to `db` as another driver whose network calls route
  via `aws` over `aws-vdb`, not a `db`-owned edge).

## Nesting

Parent: none (top-level L4 app-crate). Children: none. `lib/db` (`substrate-db`,
all behavior incl. `vault.rs` adapter, drivers, the new provenance/change-capture
modules) + `bin/db` (the `clap` CLI — **no daemon**). The lib/bin split is the
existing internal convention, not a scaffold nesting.

## Thoroughness level

**implementation-ready** for the refit's *shape* — the access model (**CLI
subprocess only, never linked, no daemon** — INTENT #167 retired the `db serve`
daemon; the warm sessions live in vdb — concern 1), the driver-matrix
disposition, the provenance-atomic ops-schema design (executed per concern 1's
transaction-owner split), change-capture on sqlite, structured
introspection, the query/virtualization formalization, the `supabase-local`
deprecation, the SQLite-in-VFS non-edge, and the secrets reconciliation are all
specified against real code and buildable as written. **approach-sketched** for:
the `aws-rds` driver (design-only by mandate — INTENT #105), the exact
provenance-context threading through every existing call site (a mechanical but
wide edit — flagged as migration cost), and the keychain→secrets credential
cutover (direction, lands when `secrets` is live).

## Assigned design-depth

Opus (single Component-Designer pass, this file), per the wave2-plan model tier
(`db` = Opus), grounded in the real `lib/db`/`bin/db` source, the batch-4
neighbor `stack.md`/`vdb`, batch-3 `secrets.md`, and batch-2
`queues.md`/`locks.md`/`supervision.md`.

## Suggested fill-model

**implementation-ready + medium complexity → strong-mid model.** The bulk is
transcription-grade against existing patterns (the new verbs are `clap`
subcommands + `--format json` output over the already-factored `lib/db` API;
change-capture mirrors the existing outbox; structured introspection mirrors
existing introspection). **Two areas need care and tests-first:** (1)
**provenance atomicity** — the provenance row MUST commit-or-rollback with its
mutation; conformance-test crash-between-write-and-trace to prove no orphaned
mutation lacks provenance (db's own cold-path write; vdb owns the hot-path
mirror per `vdb.md`); (2) **single-writer discipline** — test that a mesh-managed
SQLite database serializes a cold-path `bin/db` subprocess against vdb's warm
session (vdb quiesces; no concurrent writers). Do NOT let a cheap model
hand-wave the provenance transaction boundary — a lost trace is a silent
correctness failure the operator explicitly will not tolerate.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `vdb-db` (vdb → db) — the execution arm, **wave-3 reshaped to a CLI-subprocess control-plane contract** (INTENT #167 retired `db serve`; sessions live in vdb). db's second public surface is deleted, not added. → `scaffold/contracts/vdb-db.md`
- `db-control-plane` (db ↔ consumers) — the noun-verb control plane, **CLI subprocess only** (the interim `db serve` WS bearer is retired per INTENT #167). → `scaffold/contracts/db-control-plane.md`
- `db-inference-init` (inference → db) — fresh-node bootstrap. → `scaffold/contracts/db-inference-init.md`
- `db-secrets` (db ↔ secrets) — the Supabase Vault push adapter, reused as a `bin/db` CLI subprocess (no daemon). → `scaffold/contracts/db-secrets.md`

**No cross-cutting mesh protocols** — `db` is a pure CLI with no daemon (INTENT
#167), so it is a party to **no** `pubsub-protocol`/`restart-protocol`/
`locks-api`/`surface-schema` (those were the retired `db serve` daemon's; the
interim draft listed `locks-api`/`pubsub-protocol` here — removed). Distributed
coordination that once looked like a db concern is vdb's: vdb acquires `locks`
and orchestrates. (`vdb-secrets` is related but is NOT this component's edge — it
is secrets' own local-mesh-db adapter, parties `secrets` ↔ `vdb`; db's edge is
`db-secrets`.)

