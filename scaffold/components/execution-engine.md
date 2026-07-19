# execution-engine

**Status:** NEW (wave 2, batch 4 — Fable seat). **Kind:** shared-lib
(`lib/execution-engine`, crate `substrate-execution-engine`; slug/keyspace
prefix `ee.` — already in use by `locks.md`'s example slugs). **Naming note:**
the wave2-plan row says "name TBD — NEVER the reserved word (INTENT #74)."
This design proposes `execution-engine`/`ee` as the boring, literal choice
(alternatives considered: `handlers` — too generic against the LOCKED
four-term vocabulary where "handler" is one term of four; `reactor` — implies
an event-loop framework and invites confusion with tokio; both rejected).
Flagged for operator confirmation, not silently locked.

## Charter

`execution-engine` is the ONE shared library implementing the operator's
database-centric trigger/handler paradigm (INTENT #61/#62) for both of its
hosts: **VDB** (row/table changes on stack-pattern databases) and **KG**
(node/edge changes on graphs), via two thin **adapters** over one identical
core. It consumes the LOCKED declarative-trigger data model **unchanged** from
`types::trigger` (authored by `queues`, batch 2 — one trigger data model, two
engines, two subject bindings), executes the two handler kinds — **SQL
handlers** and **Deno/TS handlers** (Deno CONFIRMED, INTENT #65) — and carries
the OS's three hard guarantees as designed-in structure, not policy:

1. **Traced** — every invocation and every data touch a handler makes is
   recorded in per-database provenance tables with a full causal chain
   (INTENT #85/#92 — healthcare-data-engineer-grade; this plane is
   provenance's primary home).
2. **Idempotent** — delivery is at-least-once by contract; handlers must
   tolerate replay, and the engine supplies deterministic delivery identity
   plus a dedup affordance so tolerating it is cheap.
3. **Loop-bounded** — causal loop detection with a loop-depth threshold on
   repeats of `(subject, handler)` in the chain (INTENT #70 — loop depth, not
   naive cascade depth), with the escalation hook publishing an incident that
   a ccd agent investigates.

**Boundary — what execution-engine does NOT own.** It is an internal library
dependency of its host apps, **NOT a contract edge** (locked rounds 4–5;
wave2-plan §3 footer) — it never opens a socket, never registers with mesh,
and reaches mesh facilities (queues, locks, pub/sub, secrets) only through its
host's `mesh-client` via the `HostSeam` (below). It does not own the trigger
*data model* (that is `types::trigger`, shape authored by `queues`); it does
not run SQL itself (all statements flow through the host seam — VDB executes
them via the `db` crate over `vdb-db`, per INTENT #29/#96); it does not do
change *capture* on cloud targets (VDB's Supabase/RDS adapters materialize
equivalent artifacts — the engine defines the portable semantics and the
conformance suite); it does not decide database placement, replication, or
promotion (VDB), graph schema validation (KG-core — schema rejection pushes
back to the caller before the engine ever sees a change), queue semantics
(`queues`), or lock semantics (`locks`). It is the execution discipline,
compiled in twice.

## Primary design concerns

### 1. One core, two adapters — the adapter seam, precisely

The engine's core is subject-agnostic: it evaluates `types::trigger`
`FilterExpr`s over a **subject document**, renders `AssemblyTemplate`s from
it, and runs the invocation state machine. An adapter contributes exactly two
things — the subject binding and the loop-detection identity:

```rust
// lib/execution-engine — the adapter seam (two impls, ever: Tables, Graph)
pub enum AdapterKind { Tables, Graph }

pub trait ChangeAdapter: Send + Sync {
    /// Bind a captured change into the generic subject document the shared
    /// trigger model evaluates over (queues.md concern 2 — same AST, same
    /// template, different binding):
    ///   Tables: { "table", "op": insert|update|delete, "old", "new",
    ///             "meta": { database, change_id, txn_id, provenance } }
    ///   Graph:  { "node_type"|"edge_type", "op", "old", "new",
    ///             "meta": { graph, change_id, provenance } }
    fn bind(&self, change: &Change) -> SubjectDoc;

    /// The identity loop detection counts on (concern 4):
    ///   Tables: SubjectKey::Row  { database, table, pk }
    ///   Graph:  SubjectKey::Node { graph, node_id } | SubjectKey::Edge { graph, edge_id }
    fn subject_key(&self, change: &Change) -> SubjectKey;
}
```

Adapter-specific trigger needs live in an **extension struct wrapping the
unchanged core `Trigger`** (the caveat queues.md carried to this batch —
nothing is ever bolted onto the shared struct):

```rust
pub struct EngineTrigger {
    pub core: types::trigger::Trigger,   // consumed UNCHANGED — filter, assembly,
                                         // handler ref, SemaphoreChoice, IdempotencyMode,
                                         // redrive, LwwVersion
    pub binding: SubjectBinding,         // the adapter extension
    pub retry: RetryPolicy,              // max_attempts + backoff (engine-local, pre-DLQ)
    pub loop_threshold: Option<u32>,     // per-trigger override of the database default
}
pub enum SubjectBinding {
    Table { database: DbRef, table: String, ops: Vec<ChangeOp> },
    Node  { graph: GraphRef, node_type: Option<String>, ops: Vec<ChangeOp> },
    Edge  { graph: GraphRef, edge_type: Option<String>, ops: Vec<ChangeOp> },
}
```

`core.queue` (a `QueueName` in the shared struct) is set to the canonical
pseudo-queue name derived from the binding (`ee.<database>.<table>` /
`ee.<graph>.<node_type>`), so generic tooling (dashboard trigger listings,
`types` validators) renders engine triggers without a special case. Flagged
for the types/queues harmonization (see Friction #2).

Registration is static-validated exactly like queues' (`InvalidFilterExpr` /
`InvalidAssemblyTemplate` at registration, never at dispatch — the whole
point of declarative triggers, INTENT #103) plus binding validation: the
table/graph must exist, the ops list non-empty, the handler registered.

### 2. Change capture: the transactional outbox (`ee_changes`) — at-least-once from crash, by construction

The engine never scrapes WALs and never trusts an in-memory notification to
be durable. The host records every data change into an **`ee_changes` outbox
table inside the same database, in the same transaction as the change
itself** (VDB's write path does this for tables; KG's mutation path does it
for nodes/edges at graph semantics). Post-commit, the host calls
`engine.on_commit(db_ref)` as a cheap nudge; the engine's dispatcher **drains
the outbox**, evaluates triggers, invokes handlers, and marks rows terminal.
Consequences, all deliberate:

- **Crash-safe at-least-once.** A daemon death between commit and dispatch
  loses nothing — the undrained outbox row is picked up on restart. A death
  between handler completion and outbox ack causes a replay — which is why
  idempotency is a contract, not a nicety (concern 5).
- **The outbox row IS the change-provenance record** (concern 6) — captured
  once, in-band, atomic with the data. No second bookkeeping path to drift.
- **This is `lib/db`'s proven pattern, promoted.** The real `outbox.rs` in
  `lib/db` already delivers at-least-once-without-duplicates keyed by
  `idempotency_key` through a mockable transport seam; the engine generalizes
  that exact discipline (same shape, same guarantees) rather than inventing a
  parallel one.
- **SQLite locally, LOCKED** (round-9): the local implementation is plain
  tables + the host daemon's single-writer discipline. No LISTEN/NOTIFY
  needed — the on-commit nudge plus a sweep interval covers it (the
  SQLite-sufficiency line item: procedural triggers → daemon-level handlers;
  this is that, concretely).

```
ee_changes:  change_id (uuid, pk) | subject_key | op | old_image | new_image
           | txn_id | committed_at | provenance (json: types::Provenance)
           | drained_at (null = pending)
```

`old_image`/`new_image` retention is governed by the per-database provenance
level (concern 6) — `Full` keeps both images, `Standard` keeps pk + changed
column/property set + content hashes.

### 3. The handler model — SQL and Deno/TS, with the host holding every capability

**Handler definition is data + a code artifact**, registered like a trigger
and versioned. The shape deliberately **extends `lib/db`'s existing handler
`Contract`** (real code: `handler.rs` — `handler, version, kind, invocation,
timeout_ms, fail_policy, idempotency_key, params, returns`) rather than
inventing a second vocabulary:

```rust
pub struct HandlerDef {
    pub name: String, pub version: String,
    pub kind: HandlerKind,               // Sql | Deno   (db's Sql | Edge, evolved)
    pub entry: ArtifactRef,              // SQL text ref | TS module entry (VFS-resident, host-fetched)
    pub timeout: Duration,               // default 30s Sql / 60s Deno; capped per-database
    pub fail_policy: FailPolicy,         // FailClosed | FailOpen (carried from db)
    pub params: BTreeMap<String, String>,// declared param schema (assembly output is checked against it)
    pub egress: Vec<EgressRule>,         // per-handler host allowlist — empty = NO network (concern 7)
    pub secrets: Vec<SecretRef>,         // use-without-seeing refs the host may resolve for egress
}
```

**SQL handlers** are parameterized statements/scripts executed by the host
through the seam (VDB → `db` crate over `vdb-db`) inside their own
transaction, with the invocation's `TraceCtx` attached so every affected row
lands in provenance attributed to this invocation. Their writes re-enter
`ee_changes` like any write — that is how cascades happen, and why loop
detection exists.

**Deno/TS handlers — the process model, decided: ONE long-lived Deno process
per attached database** (not per node, not per handler, not per invocation):

- **Per-invocation spawn** is rejected on latency (cold V8 + module graph per
  fire is tens of ms and thrashes under cascades).
- **Per-node single process** is rejected on blast radius and permission
  mixing: databases are the provenance/config/secrets boundary (INTENT #92 —
  per-project/per-database), so they must be the sandbox boundary too; a
  wedged or compromised handler in database A must be killable without
  touching database B.
- **Per-database** matches all three boundaries, keeps isolates warm, and
  gives supervision a clean unit: the host registers each Deno child with its
  own supervision as an internal supervised process (restart-on-wedge; the
  4-level ladder applies to the *host*, which drains the engine at
  `FinishAndRelinquish` — a database in a handler transaction is exactly
  supervision.md's `CriticalSection` case).

**Sandboxing — the capability model (decided, and deliberately strict):** the
Deno child is spawned with `--allow-read=<vendored module dir>` ONLY. **No
`--allow-net`. No `--allow-env`. No `--allow-write`. No DB credentials in the
process, ever.** The child speaks line-delimited JSON-RPC over **stdio** (a
child process, not a mesh service — no port, no registry entry, nothing to
zombie-kill beyond the child itself). Every capability a handler uses is a
`ctx` call marshalled to the host:

- `ctx.sql(stmt, params)` — host executes via the seam, attributed to the
  invocation; **this is what makes provenance total**: handlers physically
  cannot write except through the traced path.
- `ctx.emit(event)` / `ctx.enqueue(queue, event)` — host publishes via
  mesh-client (pub/sub Envelope with `causal_parent` = this invocation, per
  pubsub-relay concern 1; or `queues-api` SendEvent) — the causal chain
  crosses the bus intact.
- `ctx.fetch(req)` — **all network egress executes host-side** against the
  handler's declared `EgressRule` allowlist; secrets referenced as
  `SecretRef`s are resolved by the host (via the host app's secrets edge,
  `llm_safe` discipline intact) and injected into the outbound request
  **without ever crossing the stdio boundary into the Deno process** —
  use-without-seeing (INTENT #78/#94) extended structurally to all
  third-party calls, and every egress call lands in provenance as an
  `External` touch. This is the "call third-party systems" half of the
  operator's paradigm, traced.

**Module resolution:** handler modules and their dependency graphs are
**vendored at registration** (VFS-resident artifact, lockfile-pinned; the
host fetches them local before spawn). Remote imports at run time are
impossible by construction (no net) and rejected at registration (the
registration-time validation mirrors trigger static validation). Deterministic
code, deterministic imports — no left-pad moments mid-cascade.

**Invocation kinds:** v1 executes **effect-async handlers only** (post-commit,
from the outbox). `lib/db`'s `ValidatorSync` (pre-commit gates) is explicitly
**deferred**: KG's schema validation covers the main pre-commit need at
graph level, `db` keeps its working validator machinery for its own surface,
and putting sync gates in the daemon write path is a latency/deadlock
minefield we refuse to enter silently. Flagged (Friction #4), not dropped.

### 4. The causal-trace contract + loop detection (INTENT #70/#85) — the guardrail of record

Every invocation carries a `TraceCtx`; it is the concrete instance of
first-order provenance and the substrate loop detection runs on:

```rust
pub struct TraceCtx {
    pub invocation_id: Uuid,
    pub correlation_id: Uuid,          // root of the causal chain (types::Provenance)
    pub causation_id: Uuid,            // the change_id (or event_id) that fired this
    pub depth: u32,                    // total chain length (observability, NOT the guardrail)
    pub chain: CausalChain,            // the loop-detection evidence
}
pub struct CausalChain {
    pub recent: Vec<ChainLink>,        // last K links verbatim (K fill-time, ~32)
    pub counts: BTreeMap<LoopKey, u32>,// full-chain repeat counts (never truncated)
}
pub struct ChainLink {
    pub adapter: AdapterKind, pub subject: SubjectKey,
    pub trigger_id: TriggerId, pub handler: String,
    pub invocation_id: Uuid, pub at: DateTime<Utc>,
}
pub type LoopKey = (SubjectKey, /*handler*/ String);   // "(table,row,handler)" per the lock
```

- **The loop metric is LOCKED-shaped:** `loop_depth = counts[(subject_key,
  handler)]` — repeats of the same handler touching the same row/node in one
  causal chain. NOT total cascade depth (a long legitimate pipeline of
  distinct subjects never trips it — the operator's explicit rejection of
  arbitrary cascade depth), and NOT `(table, handler)` (a batch normalizer
  legally touching many rows of one table stays clean).
- **At threshold** (per-database default, per-trigger override; default
  constant proposed **3**, fill-time-tunable): the would-be invocation is
  **parked, not run** — recorded in provenance as `outcome: LoopParked`, the
  trigger's delivery routed to the failure path (concern 5's DLQ flow), and
  ONE `EscalationRequest { kind: LoopDepthExceeded }` published toward ccd
  (deduped by the loop's identity — `(correlation_id, loop_key)` — so a hot
  loop produces one investigation, not a thousand). Park-don't-run is the
  conservative choice: by the time depth N repeats on one row, running once
  more adds information for nobody; the chain is already in provenance for
  the agent to read.
- **Optional circuit-breaker** (per-trigger `on_loop: Park | ParkAndDisable`):
  `ParkAndDisable` flips the trigger inactive until explicitly re-enabled
  (an operator/ccd action), for triggers whose loops are known-destructive.
  Default `Park`. Flagged for operator taste.
- **The chain is bounded in memory, complete on disk.** `recent` truncates at
  K links; `counts` never truncates (it is the guardrail); the FULL chain is
  always reconstructible by walking `ee_invocations.causation_id` links in
  provenance — which is exactly what the investigating ccd agent does.
- **The chain crosses app boundaries.** `TraceCtx` serializes into the meta
  of every engine-mediated effect: `ctx.sql` calls carry it down `vdb-db`;
  `ctx.emit` stamps `causal_parent` on the Envelope; KG's writes through VDB
  carry it in `kg-vdb` meta. A KG-level handler whose write fires a VDB
  table-level trigger continues ONE chain across two engine instances — loop
  detection is mesh-wide, not per-engine. (This is why `Provenance`'s
  `causation_id`/`correlation_id` live in `types` — one vocabulary, many
  hops.)

### 5. Idempotency + delivery lifecycle + failure → DLQ (the queues alignment)

Delivery from the outbox mirrors queues' `(message × trigger)` model: each
`(change, matching trigger)` pair is an independent **delivery** with its own
retry and dead-letter state. The contract, stated plainly (same sentence as
queues'): **a handler that is not idempotent is a bug against this
contract.**

- **Deterministic identity:** `delivery_id = uuid_v5(change_id, trigger_id)`
  — a replay after a crash *shares identity* with the original attempt, so
  dedup is possible at all.
- **Engine-local dedup:** `ee_deliveries` records terminal outcomes; on
  replay, a delivery already `Completed` is skipped when the trigger declares
  `IdempotencyMode::DedupByEventId` (best-effort, same honesty as queues';
  `HandlerIdempotent` skips the table check and trusts the handler).
- **Distributed dedup:** for `Replicated` databases (KG graphs replicated
  across nodes — each node's engine sees the same change), a trigger with
  `SemaphoreChoice::EventId` acquires the `locks` Ephemeral semaphore
  `ee.<database>.<change_id>.<trigger_id>` (threshold 1, lease =
  invocation timeout) before invoking — locks.md concern 7's named consumer,
  unchanged. A `PartitionMergeExceeded` at merge (both partitions ran it)
  surfaces as `outcome: PartitionConflict` into the per-trigger seam and the
  provenance record — never silently absorbed (INTENT #84). Databases the
  host declares `Locality::SingleWriter` (the common local-VDB case: one
  daemon owns the SQLite file) skip lock acquisition entirely — the cheap
  default, no cross-node race exists.
- **Retry → DLQ:** failures retry per `RetryPolicy` (engine-local, against
  the outbox row). On exhaustion the delivery — change snapshot, trigger id,
  failure history, full `TraceCtx` — is `SendEvent`-ed (through the host's
  mesh-client, `queues-api`) onto the database's dead-letter queue
  `ee.dlq.<database>` (lazily ensured; an ordinary mesh queue, INTENT #95).
  From there the standard fabric takes over: the DLQ's own declarative
  trigger escalates to ccd exactly as queues.md concern 8 designed. **The
  engine builds no escalation transport of its own** — both of its escape
  hatches (dead-letter AND loop-park) ride the same queue fabric they guard,
  and both arrive at ccd through the ONE shared `ccd-escalation` shape.
- `FailPolicy` (from db's contract): `FailClosed` deliveries follow the
  retry→DLQ path; `FailOpen` marks the delivery failed-and-done (logged in
  provenance, no DLQ) for advisory handlers whose failure must never gum the
  works.

### 6. Provenance emission — per-database `ee_*` tables, in-band (INTENT #85/#92)

Provenance lives **inside the database it describes**, in engine-owned tables
(`ee_` prefix; VDB hosts the schema real estate — co-batch seam with the vdb
designer):

```
ee_changes      — concern 2's outbox; the change-provenance record
ee_invocations  — invocation_id | delivery_id | trigger_id | handler+version
                | subject_key | causation_id | correlation_id | depth
                | chain_counts (json) | started_at | finished_at
                | outcome: Completed|Failed{err}|TimedOut|LoopParked
                          |PartitionConflict|DeadLettered
ee_touches      — touch_id | invocation_id | kind: Sql{table,op,pk,changed_cols,
                  before_hash,after_hash [images at level Full]}
                          | External{host,method,status}   // ctx.fetch egress
                          | Emit{topic|queue,event_id}      // ctx.emit/enqueue
                | at
```

Why in-band and not a central store: (a) **provenance travels with the
database** — local→cloud promotion (copy/verify/switch under a lock, INTENT
#86) carries the full history with the data, no orphaned lineage; (b)
`ee_touches` rows commit **atomically with the writes they describe** (same
transaction through the seam) — provenance cannot lie by omission after a
crash; (c) scoping is exactly INTENT #92's per-project/per-database
configuration: `ProvenanceLevel { Full | Standard | Lean }` is a per-database
setting VDB stores, governing image retention and touch granularity (`Lean`
still records every invocation and touch — the *level* tunes payload, never
whether a touch is recorded; "every time data gets touched by a handler" is
not configurable away). Retention/compaction of old provenance is VDB's
policy call (its databases, its GC discipline), flagged to the vdb designer.

The full lineage query — "everything that led to the current state" — is a
walk: row → `ee_touches` (which invocations wrote it) → `ee_invocations`
(what fired them, `causation_id`) → `ee_changes` (the originating changes) →
recurse to the root user action. This is the query the investigating ccd
agent, the dashboard's provenance view, and the healthcare-grade audit all
share.

### 7. Boring guardrails — decided values, flagged where taste matters

- **Timeouts:** per-handler (`HandlerDef.timeout`), defaults 30s SQL / 60s
  Deno, hard-capped per-database (default cap 5min). Timeout → the Deno job
  is cancelled; a job that ignores cancellation gets its **process** killed
  and restarted by the supervisor (per-database process = per-database blast
  radius), the delivery outcome `TimedOut` → retry ladder.
- **Concurrency:** per-database max concurrent invocations (default 4) +
  bounded dispatch queue; backpressure surfaces to the host as lag metrics
  on the surface schema, never as dropped deliveries (the outbox holds).
- **Resource limits:** Deno child spawned with a V8 heap cap
  (`--v8-flags=--max-old-space-size`, default 512MB, per-database config);
  breach kills the process (supervised restart), outcome `Failed`.
- **No network by default — DECIDED, flag carried:** `egress: []` is the
  default; any network use is a per-handler, per-host declarative allowlist,
  executed host-side (concern 3). This is stricter than Supabase edge
  functions (which get open egress) and is the intended difference — flagged
  prominently for the operator (Friction #6) because it constrains the
  familiar workflow.
- **No secrets in handler space, ever:** `SecretRef` resolution is host-side
  egress injection only; the invariant is structural (nothing to leak from a
  process that never held it), aligned with secrets.md's use-without-seeing
  sinks.
- **Rate honesty:** no global cascade rate-limiter in v1 (loop detection is
  the guardrail that carries intent; a rate limiter would mask loops instead
  of surfacing them). Revisit only with evidence.

### 8. Cloud-target portability — the conformance contract, honestly scoped

Locally the engine IS the implementation. On VDB's cloud targets (Supabase /
AWS RDS+Lambda) the stack pattern materializes as generated artifacts (pg
triggers writing the same `ee_changes` shape; edge functions / Lambdas
wrapping the handler modules with a generated ctx shim), authored by VDB's
adapters using the engine's **portable kernel**: the `ee_*` schemas, the
trigger registry serialization, the `TraceCtx` wire shape, and the
**conformance suite** (the same fixture cascades must produce the same
provenance chains and the same loop-parks on every target). What does NOT
port intact: the host-side egress proxy and the no-net sandbox (a Supabase
edge function has open egress by platform design) — on cloud targets the
egress allowlist degrades from *enforced* to *declared-and-audited* (egress
still logged to `ee_touches` by the generated shim, not physically blocked).
This weakening is stated here, owned jointly with the vdb designer, and
flagged to the operator (Friction #3) rather than discovered in production.

## Relationships / edges

Per the wave2-plan §3 footer, execution-engine's host relationships are
**internal library dependencies, deliberately NOT contract edges**. Its one
assigned contract pair is `ccd-escalation` (shared with queues/ccd). It is
additionally a *consumer* (through its hosts) of two cross-cutting APIs.

- **mesh(DLQ)/execution-engine → ccd** via **`ccd-escalation`** — the shared
  investigation surface; queues (batch 2) proposed the union shape and
  assigned this design the `LoopDepthExceeded` arm — authored below.
  *(scaffold/contracts/ccd-escalation.md — MISSING; my arm proposed below.)*
- **`vdb`** — HOST (internal-lib seam, not a contract): VDB embeds the engine
  with the `Tables` adapter, implements `HostSeam` (SQL via `vdb-db`, mesh via
  its mesh-client, secrets via `vdb-secrets`, artifacts via `stack-vfs`/
  `vdb-vfs`), hosts the `ee_*` schema, owns provenance-level config,
  retention, and cloud-target materialization. Co-batch ⇄: the seam trait +
  `ee_*` schemas must be reconciled mid-batch with the vdb designer.
- **`kg`** — HOST (internal-lib seam): KG embeds a second engine instance
  with the `Graph` adapter; KG-core translates graph mutations into
  node/edge `ChangeSet`s at graph semantics (schema validation happens
  BEFORE the engine sees a change). KG's `HostSeam` effects flow through
  VDB (`kg-vdb`), carrying `TraceCtx` in meta so the chain is continuous
  across the two engines (concern 4). Co-batch ⇄.
- **`types`** — library dependency: consumes `types::trigger` **UNCHANGED**
  (the batch-2 lock honored: adapter needs live in `EngineTrigger`'s
  extension, never on the core struct); consumes `Provenance`
  (`causation_id`/`correlation_id`), `Event`, `Envelope`. NEW types proposed
  for the harmonizer: `TraceCtx`/`ChainLink`/`SubjectKey` pass the inclusion
  test (public signatures of execution-engine, vdb, kg, and the
  `ccd-escalation` schema) → a `types` provenance-adjacent module (extend
  `provenance.rs` or a sibling `trace.rs` — harmonizer's call; own-module
  guardrail says sibling).
- **`queues`** (consumer, via host mesh-client, `queues-api`) — DLQ
  enqueueing (`ee.dlq.<database>`), `ctx.enqueue` event publication. The
  binding to queues is otherwise the **shared trigger data model**, not a
  wire edge.
- **`locks`** (consumer, via host mesh-client, `locks-api`) — Ephemeral
  event-ID semaphores `ee.<database>.<change_id>.<trigger_id>` for
  replicated-database trigger dedup; catches `PartitionMergeExceeded` into
  the per-trigger seam. Exactly the consumer locks.md concern 7 lists.
- **`pubsub-relay`** (consumer, via host) — `ctx.emit` publishes Envelopes
  with `causal_parent` stitching (pubsub-relay concern 1); engine
  observability topics `ee.<database>.*` (invocation lifecycle, loop-park
  notices) — additive row for the topic-prefix table, flagged to the
  harmonizer.
- **`secrets`** (indirect — host's edge) — `SecretRef` resolution for
  host-side egress injection; the engine itself never holds plaintext.
- **`db`** — NO direct relationship (INTENT #29: the engine never links or
  calls `db`; VDB does, over `vdb-db`). Named only because the engine's
  `HandlerDef` deliberately evolves `lib/db`'s handler `Contract` shape and
  its outbox discipline generalizes `lib/db`'s `outbox.rs` — continuity of
  prior art, reconciliation direction flagged (Friction #4).
- **`supervision`** (indirect — host) — Deno children are host-supervised
  internal processes; the host's restart-ladder participation drains the
  engine (in-flight invocations are the `CriticalSection`).
- **`rollup`** (indirect — via queues' assembly) — `AssemblyTemplate`'s
  `Rollup(RollupRef)` nodes resolve through the host's mesh-client at
  assembly time, inheriting queues' `llm_safe` constraint on that edge
  unchanged.

## Nesting

Parent: none (top-level shared lib, `lib/execution-engine`) | Children:
none. Compiled into exactly two hosts: `vdb` and `kg`. Never a service,
never registered, never addressed — "internal library dependency, NOT a
contract edge" (locked rounds 4–5).

## Thoroughness level

**implementation-ready** — the adapter seam (two traits: `ChangeAdapter` +
`HostSeam`), the transactional-outbox capture, the handler model with the
per-database Deno process + stdio JSON-RPC + capability-ctx, the sandbox
posture (no net / no env / no creds; host-side traced egress with
use-without-seeing secret injection), the `TraceCtx`/`CausalChain` structure
with the `(subject, handler)` loop metric and park+escalate semantics, the
deterministic-delivery idempotency contract with engine-local + `locks`
dedup tiers, the in-band `ee_*` provenance schema, and the retry→DLQ→ccd
flow are all decided and specified. **approach-sketched** in three spots:
(a) the `HostSeam` trait's exact method set (co-batch reconciliation with
vdb/kg — mid-batch draft-sharing per the ⇄ marking); (b) the cloud-target
generated-artifact shims (jointly owned with vdb, conformance-suite-bound);
(c) constants (loop threshold default, K, timeouts, heap cap) — fill-time
tuning, defaults proposed. Two decisions are explicitly held for the
operator, not silently made: no-net-by-default strictness, and
park-vs-park-and-disable (see Friction).

## Assigned design-depth

**Fable** (wave2-plan batch-4 Fable seat), single Component-Designer pass
(this file), grounded on: queues.md (the shared trigger model — consumed
unchanged), locks.md (Ephemeral class, `ee.*` slugs, partition-merge
honesty), types.md (Provenance/Event/Envelope, wire-versioning guardrail),
pubsub-relay.md (`causal_parent`), supervision.md (CriticalSection,
databases-as-supervised-services), secrets.md (`llm_safe`,
use-without-seeing sinks), stack.md/kg.md/db.md (host requirements), the
REAL `lib/db` code (`handler.rs` Contract, `outbox.rs` at-least-once
discipline), and INTENT #29, #61/#62, #65, #70/#71, #84, #85/#92, #86,
#95/#101/#103.

## Suggested fill-model

**implementation-ready + high complexity → split by risk.** (a) The
trigger/handler registries, static validation, `ee_*` DDL, and the stdio
JSON-RPC plumbing + ctx shim are careful transcription → **mid model**. (b)
The dispatcher state machine (outbox drain, retry ladder, delivery dedup,
DLQ hand-off) is queues-shaped and conformance-testable → **mid model with
the fixtures written first**. (c) The `TraceCtx` propagation + loop
detection + park/escalate path, and the Deno process supervisor
(timeout-cancel-kill-restart without losing deliveries), are the two subtle
surfaces → **strong model**; write the cascade/loop property tests before
the dispatcher, not after. Fill AFTER `types::trigger` lands and against
vdb's `HostSeam` impl (kg's follows). The conformance suite (concern 8) is
a first-class fill artifact, not an afterthought — it is what the cloud
adapters will be held to.

---

## Proposed contracts (wave 2)

wave2-plan assigns execution-engine exactly one contract pair — the shared
**`ccd-escalation`** (mesh(DLQ)/execution-engine → ccd), of which queues
(batch 2) proposed the union envelope and explicitly assigned this design
the `LoopDepthExceeded` arm. Everything else execution-engine touches is an
internal-lib seam (not a contract edge) or a consumer role in an existing
cross-cutting API; consumer-side notes follow the arm so the per-pair round
has both halves of every conversation. I do NOT edit `scaffold/contracts/*`.

### `ccd-escalation` — the `LoopDepthExceeded` arm (mine; queues owns `DeadLetter`; ccd owns the receiver)

**Purpose.** One investigation surface for the two guardrails-of-last-resort
(INTENT #70/#89): queues' dead-letter exhaustion and this engine's
loop-depth-exceeded. Accepting queues' proposed union (`EscalationRequest` /
`EscalationKind` / `EscalationAck`) unchanged in envelope — but **enriching
the `LoopDepthExceeded` arm**, which queues sketched thin
(`{ engine, depth, threshold }`) as a placeholder for this pass:

```rust
// replaces the placeholder arm inside queues' EscalationKind union:
LoopDepthExceeded {
    host: Slug,                        // "vdb" | "kg" — the embedding app
    adapter: AdapterKind,              // Tables | Graph
    database: DbRef,                   // the provenance home to investigate
    subject: SubjectKey,               // the row/node the loop orbits
    trigger_id: TriggerId,
    handler: String, handler_version: String,
    loop_count: u32, threshold: u32,   // counts[(subject,handler)] at park time
    chain_recent: Vec<ChainLink>,      // last K links verbatim
    parked_invocation: Uuid,           // the invocation that was parked, not run
    provenance_root: Uuid,             // correlation_id — walk ee_* from here
}
```

Delivery mechanics: the engine `SendEvent`s the `EscalationRequest` (as an
`Event`, `event_type: "ee.loop.exceeded"`) onto the mesh queue fabric
through its host; a standing declarative trigger delivers it to ccd —
at-least-once, durable, deduped by `escalation_id`, which the engine mints
deterministically as `uuid_v5(correlation_id, loop_key)` so a hot loop
yields ONE investigation. `context` (the union's open `Value`) carries the
failure history and a pre-rendered lineage summary so the agent starts warm;
the authoritative evidence is the `ee_*` tables, reachable via
`provenance_root` + `database` (the agent queries through vdb/db surfaces —
read-only investigation, no engine back-door).

**Error cases.** Inherits queues' arm-level cases (`CcdUnreachable` → the
escalation event itself dead-letters onto its queue's retention, alarmed,
never silently lost; duplicate arrival → `escalation_id` dedup). One
engine-specific case: `DatabaseUnavailable` at investigation time (the
database was promoted/moved between park and investigation) — the request's
`chain_recent` + `context` snapshot must therefore be self-sufficient for a
first-pass diagnosis; stated as a conformance requirement on the arm, not
left to luck.

**Version-sensitivity.** MEDIUM, matching queues' assessment of the union:
`EscalationKind` is additive-only with `#[serde(other)]` reserved;
within the arm, every field beyond the queues-sketched three is
`#[serde(default)]`-tolerant so a ccd built against the thin sketch still
deserializes an enriched request. `SubjectKey`/`ChainLink` become `types`
structs (inclusion test passed) under guardrail-4 wire discipline.
Ownership line for the per-pair round, restated from queues and accepted:
**queues authors `DeadLetter`, execution-engine authors `LoopDepthExceeded`,
ccd owns the receiver + `EscalationAck`.**

### Consumer-side notes (no contract authored; flags for the named owners)

- **`queues-api` (owner: queues).** The engine consumes `EnsureQueue` +
  `SendEvent` (DLQ + escalation events) through its host's mesh-client.
  One flag: the DLQ payload embeds a full delivery snapshot (change images
  at the configured provenance level + `TraceCtx`) — `queues-api` should
  state a max event-payload size so a `Full`-level image can't jam the
  fabric; the engine will truncate images to refs (`database` +
  `change_id`) past that bound.
- **`locks-api` (owner: locks).** Slugs `ee.<database>.<change_id>.<trigger_id>`,
  class `Ephemeral`, threshold 1, lease = invocation timeout; already named
  by locks.md concern 7 — confirmed verbatim, no deltas requested.
- **`types::trigger` (owner: queues-authored, types-housed).** Consumed
  UNCHANGED, as required. Two harmonizer flags: (1) `Trigger.queue`'s
  `QueueName` doc-comment should bless the engine's canonical pseudo-queue
  names (`ee.<database>.<table>`); (2) `HandlerRef::ExecFn { engine, function }`
  matches the engine's registered-handler addressing — `function` =
  `name@version`; confirmed workable.
- **`HostSeam` + `ChangeAdapter` + `ee_*` schema (owners: this design +
  vdb/kg co-batch).** Internal-lib seams, explicitly NOT contracts; recorded
  in this file's concerns 1/2/6 and reconciled mid-batch. The single hardest
  cross-module dependency in batch 4 (the analogue of batch 2's `put_flush`
  seam): if VDB's write path cannot host the same-transaction `ee_changes`
  write, the at-least-once + atomic-provenance guarantees both fall —
  reconcile FIRST.

## Non-obvious tests (conformance + correctness)

- **Loop trips at exactly threshold, once:** handler A on row X enqueues a
  change that re-fires A on X; with threshold 3, invocations 1–3 run,
  invocation 4 is `LoopParked`, exactly ONE `EscalationRequest` is emitted
  (dedup by `uuid_v5(correlation_id, loop_key)`), and the chain in
  provenance shows `counts[(X,A)] == 3`.
- **Legit recurrence does NOT trip:** the same handler touching 100
  *different* rows of one table in one cascade (batch normalizer) never
  parks — the metric is `(subject, handler)`, not `(table, handler)` and
  not cascade depth.
- **Cross-engine loop detection:** a KG node handler writes (via `kg-vdb`)
  a VDB row whose table trigger writes back a KG node property → the chain
  crosses two engine instances with one `correlation_id`; the repeat count
  on the KG node's `(subject, handler)` accumulates across the boundary and
  parks at threshold.
- **Crash-replay idempotency:** kill the host between handler completion
  and outbox ack; on restart the delivery re-fires with the SAME
  `delivery_id`; `DedupByEventId` skips it (single net effect);
  with `HandlerIdempotent` the handler runs twice and the effect is
  provably single (fixture handler is upsert-shaped).
- **Provenance is atomic with effects:** kill the host mid-`ctx.sql`; the
  data write and its `ee_touches` row either both committed or neither —
  no touch without a record, no record without a touch.
- **Full-lineage walk:** a three-stage cascade (insert → transform →
  aggregate) yields a complete walk from the final row back to the root
  change via `ee_touches` → `ee_invocations` → `ee_changes`; golden-file
  the reconstructed lineage.
- **No secret crosses the stdio boundary:** a handler declares a
  `SecretRef`-authenticated egress call; assert the recorded JSON-RPC
  transcript between host and Deno child contains no plaintext (the host
  injected it egress-side); the `ee_touches` `External` row records host +
  status, never the credential.
- **Sandbox holds:** a handler attempting raw `fetch()` (no `--allow-net`),
  `Deno.env`, or a read outside the vendored dir fails as a *handler
  failure* (permission error → retry ladder per `FailPolicy`), never as an
  engine crash; the Deno process survives.
- **Timeout → kill → no loss:** a handler that ignores cancellation gets
  its per-database Deno process killed; sibling databases' processes are
  untouched; the delivery re-fires after supervised restart with the same
  `delivery_id`.
- **Replicated dedup + partition honesty:** two nodes' engines observe the
  same replicated change; with `SemaphoreChoice::EventId` exactly one
  invokes; forced partition twins both invoke, and on merge the engine
  surfaces `outcome: PartitionConflict` into provenance and the
  per-trigger seam — never a silent double.
- **Registration-time rejection:** a trigger with a malformed `FilterExpr`,
  a binding to a nonexistent table, or a handler module with an unvendored
  remote import is rejected at registration with a typed error — never at
  dispatch.
- **Promotion portability (conformance suite):** run the fixture cascade on
  local SQLite, then on a stubbed Supabase materialization: identical
  trigger firings, identical `ee_invocations`/`ee_touches` chains,
  identical loop-park — the portable-kernel guarantee, mechanized.
- **Shared-model guard (batch-2 handshake):** the same `FilterExpr` +
  `AssemblyTemplate` fixture evaluates identically over a queues event
  subject and an engine row-change subject (queues.md's closing test, run
  from this side).

## Friction points (for the operator round)

1. **Hot-path latency through the seam (INTENT #29 consequence).** Every
   handler `ctx.sql` crosses host → vdb → `db` (cross-app WS per the lock).
   Correct by architecture, but per-statement round trips inside cascades
   will hurt; the batch-4 `vdb-db` contract should offer a
   session/batch-statement surface. Raised to the vdb/db designers, not
   resolved here.
2. **`Trigger.queue` semantics for engine bindings.** The shared struct's
   `queue: QueueName` is queues-native; the engine fills it with a canonical
   pseudo-queue name and keeps the real binding in the extension struct.
   Workable but slightly awkward — types/queues harmonizer should bless or
   improve it.
3. **Cloud targets weaken two guardrails.** On Supabase/RDS, the no-net
   sandbox and host-side egress proxy degrade to declared-and-audited (open
   platform egress); traced/idempotent/loop-bounded port intact via the
   generated shims + conformance suite, but the sandbox does not. Stated
   honestly; operator should bless the asymmetry.
4. **Two handler vocabularies exist during transition.** `lib/db`'s working
   `Contract`/outbox/edge machinery keeps serving db's own surface (incl.
   `ValidatorSync`, which the engine defers in v1); `HandlerDef` is its
   designed successor shape. Reconciliation direction: db's edge/outbox
   path becomes VDB's Supabase materialization substrate — needs the
   db-designer's confirmation this batch.
5. **No-net-by-default is stricter than the operator's Supabase habit.**
   Third-party calls require a per-handler host allowlist and flow through
   the host proxy (which is also how secrets stay unseen and egress gets
   provenance). Deliberate, defensible, and a real workflow change — needs
   an explicit operator yes.
6. **Loop-park disposition.** Default `Park` (skip the Nth+1 invocation,
   escalate); `ParkAndDisable` (circuit-break the trigger) is opt-in.
   Threshold default 3. All three choices are taste — flagged.
7. **KG hosts its own engine instance** (Graph adapter in the kg app, chain
   propagated through `kg-vdb` meta) rather than riding VDB's table-level
   instance. Cleanest semantics; kg designer must confirm the double-level
   firing story (KG-level + underlying table-level triggers are distinct,
   both traced, one chain).
8. **Crate name.** `execution-engine`/`ee` proposed (reserved word honored);
   operator has final say per the "name TBD" note.
