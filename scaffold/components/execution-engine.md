# execution-engine

**Status:** NEW (wave 2, batch 4 — Fable seat); **REFRESHED (wave 3, unit
execution-engine-refresh)** — folds queues.md's batch-3 `TriggerSource` split
(`Queue`/`Schedule`) in as a second, independent invocation origin (§3,
below), reconciles idempotency with queues-native ack/nack for that origin,
and reaffirms (unchanged) the no-net-by-default sandbox posture, the
per-database Deno process model, the kg-nodes/edges adapter seam, and
in-transaction provenance emission into VDB's own tables. **Kind:** shared-lib
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
core, **plus** (wave-3 fold, §3) a THIRD, adapter-less invocation origin for
handlers dispatched directly off an ordinary mesh queue or schedule trigger
(`TriggerSource::Queue` / `TriggerSource::Schedule`, batch 3) with no
row/node change behind them at all. It consumes the LOCKED declarative-trigger
data model **unchanged** from `types::trigger` (authored by `queues`, batch 2,
extended batch 3 with `TriggerSource` — one trigger data model, one engine,
three subject bindings: table/node/edge change, mesh event, schedule
occurrence), executes the two handler kinds — **SQL handlers** and **Deno/TS
handlers** (Deno CONFIRMED, INTENT #65) — and carries the OS's three hard
guarantees as designed-in structure, not policy:

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
   a cc agent investigates.

**Boundary — what execution-engine does NOT own.** It is an internal library
dependency of its host apps, **NOT a contract edge** (locked rounds 4–5;
wave2-plan §3 footer) — it never opens a socket, never registers with mesh,
and reaches mesh facilities (queues, locks, pub/sub, secrets) only through its
host's `mesh-client` via the `HostSeam` (below); this includes RECEIVING
queue/schedule-dispatched invocations (§3, wave-3 fold) — the host, already a
`queues-api` party for every other reason a mesh service is, simply hands the
engine a `Deliver` it already received, and the engine hands back an ack/nack
decision for the host to relay. The engine still never speaks `queues-api`
itself. It does not own the trigger
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
// lib/execution-engine — the adapter seam (two CHANGE-BOUND impls: Tables,
// Graph — plus a third, adapter-less origin, Direct, added by the wave-3
// fold, §3: a queue/schedule-dispatched handler with no captured Change at
// all, so it never implements ChangeAdapter — it is described here only so
// AdapterKind/SubjectKey stay ONE enum each across all three origins).
pub enum AdapterKind { Tables, Graph, Direct, #[serde(other)] Unknown }

pub trait ChangeAdapter: Send + Sync {
    /// Bind a captured change into the generic subject document the shared
    /// trigger model evaluates over (queues.md concern 2 — same AST, same
    /// template, different binding):
    ///   Tables: { "table", "op": insert|update|delete, "old", "new",
    ///             "meta": { database, change_id, txn_id, provenance } }
    ///   Graph:  { "node_type"|"edge_type", "op", "old", "new",
    ///             "meta": { graph, change_id, provenance } }
    fn bind(&self, change: &Change) -> SubjectDoc;

    /// The identity loop detection counts on (concern 5):
    ///   Tables: SubjectKey::Row  { database, table, pk }
    ///   Graph:  SubjectKey::Node { graph, node_id } | SubjectKey::Edge { graph, edge_id }
    fn subject_key(&self, change: &Change) -> SubjectKey;
}

// The canonical, now-explicit SubjectKey (previously only informally
// referenced in the doc comments above; written out in full here because §3
// adds its third variant and cc-escalation.md's `LoopDepthExceeded.subject`
// consumes this shape verbatim — see "Proposed contracts (wave 3)"):
pub enum SubjectKey {
    Row    { database: DbRef,    table: String, pk: String },
    Node   { graph: GraphRef,    node_id: String },
    Edge   { graph: GraphRef,    edge_id: String },
    Direct { engine: Slug,       function: String },  // WAVE-3 FOLD, §3 — no
                                                       // row/node exists; the
                                                       // ExecFn target IS the subject
    #[serde(other)]
    Unknown,          // wire-discipline fail-safe (types guardrail 4), matching
                       // Schedule::Unknown/FireTarget::Unknown's pattern in queues.md
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
  idempotency is a contract, not a nicety (concern 6).
- **The outbox row IS the change-provenance record** (concern 7) — captured
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
level (concern 7) — `Full` keeps both images, `Standard` keeps pk + changed
column/property set + content hashes.

### 3. Queue- and Schedule-dispatched handlers — the direct `ExecFn` path (wave-3 fold; resolves queues.md's batch-3 flag)

Batch 3 folded `cron` into `queues` as `TriggerSource::Schedule`, generalizing
every `types::trigger::Trigger`'s origin to `source: TriggerSource { Queue(QueueName)
| Schedule(ScheduleSource) }`. queues.md flagged the open point this design
must close: *"execution-engine still uses only `TriggerSource::Queue` bindings
(its adapters); flagged for batch-4 co-design: it must consume the folded
model unchanged."* **Resolved here:** the engine's adapters (§1) are ONE
invocation origin, not the only one. A SECOND, independent origin exists —
an ordinary `queues`-registered `Trigger` (its `source` may be `Queue`
**or** `Schedule`; this design never needs to distinguish the two past the
`Deliver` boundary) whose `handler: HandlerRef::ExecFn { engine, function }`
names this host's mesh service slug (`"vdb"` or `"kg"` — one registration per
node, per `vdb.md`/`kg.md`) — but `engine: Slug` alone only routes to the
HOST APP, not to *which* managed database or graph the handler runs against.
**Convention this design defines for `function`** (an opaque `String` to
`queues-api` — no wire change, see "Proposed contracts (wave 3)"):
`"<subject_ref>::<handler_name>@<version>"`, e.g. `"demo/main::nightly_rollup@2.0.0"`
(VDB, `subject_ref` = `DbRef`) or `"proj-graph::gc_sweep@1.0.0"` (KG,
`subject_ref` = `GraphRef`, resolved to its underlying `DbRef` via `kg-vdb` —
KG-built-on-VDB, INTENT #96, guarantees this always resolves).

**Why this exists, concretely.** Two real cases the change-bound adapters
(§1) cannot express, both legitimate uses of the database-centric handler
paradigm (INTENT #61/#62) that simply have no captured `Change` to bind:

- **Schedule-only handlers.** A nightly maintenance job, a rollup
  aggregation sweep, a GC pass — driven purely by `TriggerSource::Schedule`,
  with no triggering row/node at all.
- **Cross-database/event-driven handlers.** A handler on database A that
  should react to an arbitrary mesh event published by service B (not a
  change captured in A's own `ee_changes` outbox) — an ordinary
  `TriggerSource::Queue` trigger with `HandlerRef::ExecFn`.

**Dispatch mechanics — deliberately NOT a second execution engine.** The
host (VDB or KG) is an ordinary `queues-api` party (cross-cutting, "every
service is a party" — queues.md's own framing); it receives `Deliver` pushes
addressed to its `engine: Slug`, and on receipt:

1. Parses `function` into `(subject_ref, handler_name, version)`, looks up
   the registered `HandlerDef` in the SAME registry §4 uses — same static
   validation at registration time (§1's registration rules apply verbatim;
   an `ExecFn` naming an unregistered handler or nonexistent `subject_ref` is
   rejected on `RegisterTrigger`, at `queues`, before it ever reaches here).
2. Builds a `TraceCtx` (§5) from the `Deliver` envelope: `causation_id =
   event_id` (queues' own id — for a `Schedule` source this is the
   deterministic `occurrence_id`, queues.md concern 10; the engine never
   needs to know which kind produced it), `correlation_id` from
   `Deliver.correlation_id` (absent ⇒ this invocation roots a NEW chain —
   exactly like any user action; a schedule fire IS a root). `subject_key =
   SubjectKey::Direct { engine, function }`, `adapter = AdapterKind::Direct`
   — so loop detection (§5) applies UNIFORMLY: a direct-dispatched handler
   that recursively re-triggers itself *within one causal chain* still trips
   `(subject_key, handler)`; a schedule tick's own periodic re-firing never
   trips it, because each occurrence roots its own chain (no shared
   `correlation_id` across ticks — queues.md concern 10).
3. Invokes the handler through the IDENTICAL per-database Deno process / SQL
   seam §4 defines — `ctx.sql`/`ctx.emit`/`ctx.fetch` are unchanged. A
   direct-dispatched handler's writes re-enter `ee_changes` exactly like any
   handler's (§2), so a nightly aggregation job that touches rows is traced
   and can itself fire change-bound triggers downstream: ONE chain, crossing
   from a `Direct` origin into `Tables`/`Graph` origins exactly as §5
   already describes chains crossing `Tables` into `Graph`.
4. **Ack/nack rides `queues-api` directly — no engine-local retry/DLQ for
   this path.** The one genuine simplification direct-dispatch buys: `queues`
   already owns the FULL delivery lifecycle (visibility timeout, redrive,
   DLQ, `DeadLetter` escalation) for anything reaching it as a `Deliver`. The
   engine does not reinvent retry/DLQ here — on handler success it
   `AckDelivery`s; on failure (respecting `FailPolicy` — `FailClosed` nacks
   for redrive, `FailOpen` acks anyway and records `Failed` in provenance) it
   `NackDelivery`s or lets the visibility timeout expire. Exhaustion
   dead-letters through `queues`' own DLQ trigger exactly as §6 describes for
   engine-local failures — the two paths converge on the SAME
   `cc-escalation` `DeadLetter` arm (authored by queues), never a second one.
5. **Idempotency is queues' `delivery_id`, not a re-derived one.**
   `IdempotencyMode` on the trigger applies identically: `DedupByEventId`
   looks up `ee_deliveries` keyed by queues' own `delivery_id` (there is no
   `change_id` to derive `uuid_v5` from here — queues' id IS the
   deterministic identity) and skips a delivery already `Completed`;
   `HandlerIdempotent` trusts the handler and always invokes. **The
   per-trigger `SemaphoreChoice` (INTENT #101) is enforced UPSTREAM, by
   `queues`, before the engine ever sees the `Deliver`** — for `EventId`,
   queues already de-duplicates concurrent delivery of the same event/
   occurrence across the fleet (queues.md concerns 4/10), so the engine
   never needs its own `locks` acquisition for this origin (contrast §6's
   replicated-database case, which DOES need one, because there the engine
   itself independently observes the same change on multiple nodes — here
   queues has already collapsed that to one `Deliver`).

**Provenance for a changeless invocation.** `ee_invocations` gains no
matching `ee_changes` row for a `Direct`-origin invocation (there is no
captured change) — its `causation_id` instead points at queues' own
`event_id`/`occurrence_id`, provenance-adjacent metadata that lives in
`queues`' durable store, not this database's tables. The lineage walk (§7)
terminates one hop earlier for this origin: row → `ee_touches` →
`ee_invocations` → (queues' event/occurrence, external to this database) —
still a complete, honest chain, just crossing into a different subsystem's
record instead of another `ee_changes` row. Flagged for the harmonizer:
whether a dashboard lineage-viewer needs `ee_invocations.adapter ==
Direct` as its cue to stop looking for a local `ee_changes` row (it does —
this is exactly what the field is for).

**What does NOT change.** The change-bound path (§1's adapters) is entirely
unaffected — it never touches `queues-api` directly for dispatch and keeps
its own `ee_changes`-outbox-driven flow. An `EngineTrigger`'s `core.source`
set to a `Queue(<canonical pseudo-queue>)` (§1) remains internal bookkeeping
so generic tooling renders one shape, and is orthogonal to this section:
`Direct`-origin triggers are registered directly with `queues`, not with the
engine's own registry — there is no `EngineTrigger` for a `Direct`
invocation at all; `queues` holds the trigger, the engine only supplies the
`ExecFn` target it routes to.

### 4. The handler model — SQL and Deno/TS, with the host holding every capability

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
    pub egress: Vec<EgressRule>,         // per-handler host allowlist — empty = NO network (concern 8)
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

### 5. The causal-trace contract + loop detection (INTENT #70/#85) — the guardrail of record

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
  trigger's delivery routed to the failure path (concern 6's DLQ flow), and
  ONE `EscalationRequest { kind: LoopDepthExceeded }` published toward cc
  (deduped by the loop's identity — `(correlation_id, loop_key)` — so a hot
  loop produces one investigation, not a thousand). Park-don't-run is the
  conservative choice: by the time depth N repeats on one row, running once
  more adds information for nobody; the chain is already in provenance for
  the agent to read.
- **Optional circuit-breaker** (per-trigger `on_loop: Park | ParkAndDisable`):
  `ParkAndDisable` flips the trigger inactive until explicitly re-enabled
  (an operator/cc action), for triggers whose loops are known-destructive.
  Default `Park`. Flagged for operator taste.
- **The chain is bounded in memory, complete on disk.** `recent` truncates at
  K links; `counts` never truncates (it is the guardrail); the FULL chain is
  always reconstructible by walking `ee_invocations.causation_id` links in
  provenance — which is exactly what the investigating cc agent does.
- **The chain crosses app boundaries.** `TraceCtx` serializes into the meta
  of every engine-mediated effect: `ctx.sql` calls carry it down `vdb-db`;
  `ctx.emit` stamps `causal_parent` on the Envelope; KG's writes through VDB
  carry it in `kg-vdb` meta. A KG-level handler whose write fires a VDB
  table-level trigger continues ONE chain across two engine instances — loop
  detection is mesh-wide, not per-engine. (This is why `Provenance`'s
  `causation_id`/`correlation_id` live in `types` — one vocabulary, many
  hops.)

### 6. Idempotency + delivery lifecycle + failure → DLQ (the queues alignment) — the change-bound path

**Scope note (wave-3 fold):** this section is the idempotency/DLQ story for
the **change-bound** path (§1's adapters, outbox-driven). §3's direct-dispatch
origin has its own, simpler idempotency story (queues' own `delivery_id` +
queues-native ack/nack/redrive/DLQ — §3 point 5) because queues already owns
that delivery's full lifecycle; it is not repeated here.

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
  trigger escalates to cc exactly as queues.md concern 8 designed. **The
  engine builds no escalation transport of its own** — both of its escape
  hatches (dead-letter AND loop-park) ride the same queue fabric they guard,
  and both arrive at cc through the ONE shared `cc-escalation` shape.
- `FailPolicy` (from db's contract): `FailClosed` deliveries follow the
  retry→DLQ path; `FailOpen` marks the delivery failed-and-done (logged in
  provenance, no DLQ) for advisory handlers whose failure must never gum the
  works.

### 7. Provenance emission — per-database `ee_*` tables, in-band, inside VDB's in-transaction schema (INTENT #85/#92)

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
recurse to the root user action. This is the query the investigating cc
agent, the dashboard's provenance view, and the healthcare-grade audit all
share.

### 8. Boring guardrails — decided values, flagged where taste matters

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
- **No network by default — OPERATOR-BLESSED (friction-round 2, INTENT
  #125):** `egress: []` is the default; any network use is a per-handler,
  per-host declarative allowlist, executed host-side (concern 4). This is
  stricter than Supabase edge functions (which get open egress) and is the
  intended difference. The operator blessed it explicitly — "Fascinating
  idea. I like it. I'm OK with that" — including the weaker cloud caveat
  (concern 9: on cloud targets the allowlist degrades to
  declared-and-audited, not physically enforced). No longer an open flag.
- **No secrets in handler space, ever:** `SecretRef` resolution is host-side
  egress injection only; the invariant is structural (nothing to leak from a
  process that never held it), aligned with secrets.md's use-without-seeing
  sinks.
- **Rate honesty:** no global cascade rate-limiter in v1 (loop detection is
  the guardrail that carries intent; a rate limiter would mask loops instead
  of surfacing them). Revisit only with evidence.

### 9. Cloud-target portability — the conformance contract, honestly scoped

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
This weakening is stated here, owned jointly with the vdb designer, and was
flagged to the operator (Friction #3) rather than discovered in production —
**blessed at friction-round 2 (INTENT #125), asymmetry included.**

## Relationships / edges

Per the wave2-plan §3 footer, execution-engine's host relationships are
**internal library dependencies, deliberately NOT contract edges**. Its one
assigned contract pair is `cc-escalation` (shared with queues/cc). It is
additionally a *consumer* (through its hosts) of two cross-cutting APIs.

- **mesh(DLQ)/execution-engine → cc** via **`cc-escalation`** — the shared
  investigation surface; queues (batch 2) proposed the union shape and
  assigned this design the `LoopDepthExceeded` arm — authored below.
  *(authored: scaffold/contracts/cc-escalation.md.)*
- **`vdb`** — HOST (internal-lib seam, not a contract): VDB embeds the engine
  with the `Tables` adapter, implements `HostSeam` (SQL via `vdb-db`, mesh via
  its mesh-client, secrets via `vdb-secrets`, artifacts via `vdb-vfs` — the
  `stack-vfs` name is a rename-tombstone), hosts the `ee_*` schema, owns provenance-level config,
  retention, and cloud-target materialization. Co-batch ⇄: the seam trait +
  `ee_*` schemas must be reconciled mid-batch with the vdb designer.
- **`kg`** — HOST (internal-lib seam): KG embeds a second engine instance
  with the `Graph` adapter; KG-core translates graph mutations into
  node/edge `ChangeSet`s at graph semantics (schema validation happens
  BEFORE the engine sees a change). KG's `HostSeam` effects flow through
  VDB (`kg-vdb`), carrying `TraceCtx` in meta so the chain is continuous
  across the two engines (concern 5). Co-batch ⇄.
- **`types`** — library dependency: consumes `types::trigger` **UNCHANGED**
  (the batch-2 lock honored: adapter needs live in `EngineTrigger`'s
  extension, never on the core struct); consumes `Provenance`
  (`causation_id`/`correlation_id`), `Event`, `Envelope`. NEW types proposed
  for the harmonizer: `TraceCtx`/`ChainLink`/`SubjectKey`/`AdapterKind` pass
  the inclusion test (public signatures of execution-engine, vdb, kg, and the
  `cc-escalation` schema) → a `types` provenance-adjacent module (extend
  `provenance.rs` or a sibling `trace.rs` — harmonizer's call; own-module
  guardrail says sibling). **Wave-3 addition:** `AdapterKind` gains a third
  variant `Direct` and `SubjectKey` gains `Direct { engine: Slug, function:
  String }` (§3) — both additive, both given `#[serde(other)] Unknown`
  fail-safe arms so an older `cc` (built against the wave-2 two-variant
  shape) degrades to `Unknown` on a `Direct` escalation instead of failing
  the whole deserialize — see "Proposed contracts (wave 3)".
- **`queues`** (consumer, via host mesh-client, `queues-api`) — DLQ
  enqueueing (`ee.dlq.<database>`), `ctx.enqueue` event publication, AND
  (wave-3 fold, §3) receiving `Deliver` pushes + issuing `AckDelivery`/
  `NackDelivery` for `HandlerRef::ExecFn`-targeted triggers whose `source` is
  `TriggerSource::Queue` or `TriggerSource::Schedule`. This is a wider use of
  `queues-api` than before (push-dispatch + ack/nack, not just enqueue), but
  still not a NEW contract file — it is the same cross-cutting `queues-api`
  every mesh service already speaks, reached through the host's mesh-client,
  same as `vdb`/`kg`'s own generic mesh-service surface. The binding for the
  **change-bound** path remains the **shared trigger data model**, not a wire
  edge.
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
dedup tiers, the in-band `ee_*` provenance schema, and the retry→DLQ→cc
flow are all decided and specified — the no-net-by-default sandbox posture
now operator-blessed (friction-round 2, INTENT #125). **Wave-3 refresh
(§3), also implementation-ready:** the direct-dispatch `ExecFn` origin
(`TriggerSource::Queue`/`Schedule` → `Deliver` → handler → `AckDelivery`/
`NackDelivery`, no engine-local retry/DLQ, `delivery_id`-keyed dedup, unified
loop detection via `AdapterKind::Direct`/`SubjectKey::Direct`) is fully
specified — it resolves queues.md's batch-3 flag rather than opening a new
open question.
**approach-sketched** in four spots:
(a) the `HostSeam` trait's exact method set (co-batch reconciliation with
vdb/kg — mid-batch draft-sharing per the ⇄ marking); (b) the cloud-target
generated-artifact shims (jointly owned with vdb, conformance-suite-bound);
(c) constants (loop threshold default, K, timeouts, heap cap) — fill-time
tuning, defaults proposed; (d) the `HandlerRef::ExecFn.function` string
convention (`"<subject_ref>::<handler_name>@<version>"`, §3) — a component-
level parsing convention this design defines, pending the queues designer's
concurrence that it belongs in a doc-comment rather than a struct change.
One decision remains explicitly held for the operator: park-vs-park-and-
disable (see Friction). The other — no-net-by-default strictness — was
BLESSED at friction-round 2 (INTENT #125), cloud caveat included.

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
#95/#101/#103. **Wave-3 refresh** (unit execution-engine-refresh, Sonnet
Component-Designer pass, this batch), additionally grounded on the harness-
side wave-3 intent ledger (§C design-around rules) and queues.md's wave-3
fold (concerns 10–12, `TriggerSource`/`ScheduleSource`/`HandlerRef::Emit`)
and `contracts/cc-escalation.md` (the arm this design owns).

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
vdb's `HostSeam` impl (kg's follows). The conformance suite (concern 9) is
a first-class fill artifact, not an afterthought — it is what the cloud
adapters will be held to. (d) **The §3 direct-dispatch path is the
LAST-filled surface of the four** — it is genuinely simpler (no
engine-local retry/DLQ, dedup keyed off queues' own `delivery_id`) → **mid
model**, but it must be filled AFTER (b)'s dispatcher and the host's
ordinary `queues-api` client both work, since it only adds a thin
`Deliver`-in / ack-out shim over machinery both already built.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `cc-escalation` — the `LoopDepthExceeded` arm (mine; queues owns `DeadLetter`; cc owns the receiver). → `scaffold/contracts/cc-escalation.md`

Also a party to (authored elsewhere / cross-cutting): `locks-api`, `queues-api` — see `scaffold/contracts/`.

Component-side notes (consumer-side flags for the named owners, not captured in
the contract files):
- `queues-api` (owner: queues) — the engine consumes `EnsureQueue`+`SendEvent`;
  flag: state a max event-payload size so a `Full`-level DLQ image can't jam
  the fabric (the engine truncates images to `database`+`change_id` refs past
  the bound).
- `locks-api` (owner: locks) — slugs `ee.<database>.<change_id>.<trigger_id>`,
  class `Ephemeral`, threshold 1, lease = invocation timeout; confirmed
  verbatim, no deltas.
- `types::trigger` (owner: queues-authored, types-housed) — consumed UNCHANGED;
  flags: bless the `ee.<database>.<table>` pseudo-queue names in `QueueName`'s
  doc-comment; `HandlerRef::ExecFn { engine, function: "name@version" }`
  confirmed workable **at wave 2, for the change-bound path only, where the
  adapter already supplies the database/graph. SUPERSEDED for wave 3's §3
  direct-dispatch use, which has no adapter context** — see "Proposed
  contracts (wave 3)" below for the `"<subject_ref>::<handler_name>@<version>"`
  convention this design now proposes for that case.
- Sequencing (`HostSeam`/`ChangeAdapter`/`ee_*` schema; internal-lib seams, NOT
  contracts) — the hardest cross-module dependency in batch 4: if VDB's write
  path cannot host the same-transaction `ee_changes` write, the at-least-once
  and atomic-provenance guarantees both fall — reconcile FIRST.

## Proposed contracts (wave 3)

Additive-only proposals against contract shapes this design owns or authored
a share of. Neither item changes a wire struct's binary shape; both are
flagged to the respective owner (`cc` for the escalation arm, `queues` for
the doc-comment convention) rather than silently assumed.

- **`cc-escalation` — `LoopDepthExceeded` arm, additive extension (§3).**
  This design owns this arm's shape (wave 2). Wave-3 addition: `AdapterKind`
  gains a third variant `Direct` (alongside `Tables`/`Graph`) and `SubjectKey`
  gains `Direct { engine: Slug, function: String }`, so a direct-dispatched
  handler that loop-parks escalates through the SAME arm, unchanged in every
  other field (`host`/`database`/`trigger_id`/`handler`/`chain_recent`/
  `parked_invocation`/`provenance_root` all still apply — `database` resolves
  via `kg-vdb` for a KG-hosted `Direct` invocation exactly as it already does
  for the `Graph` adapter, since KG is built on VDB, INTENT #96). Both new
  variants carry a `#[serde(other)] Unknown` fail-safe arm, mirroring the
  `Schedule::Unknown`/`FireTarget::Unknown` pattern queues.md's own wave-3
  fold already established — an older `cc` deserializes an unrecognized
  variant as `Unknown` rather than dropping the whole escalation. Per
  cc-escalation.md's own versioning rule ("Additive-safe: new
  `#[serde(default)]` fields... the open `context: Value` absorbs richer
  payloads"), this is squarely additive-safe; no existing field, arm, or
  semantic changes. Flagged to `cc` for concurrence (their receiver need not
  do anything special with a `Direct` subject beyond rendering it — the
  investigation context is already self-sufficient per the arm's existing
  conformance requirement).
- **`queues-api` — `HandlerRef::ExecFn.function` convention, component-level
  (§3).** Not a struct change (`function` stays an opaque `String` in
  `queues-api`'s schema) — a parsing convention this design defines because
  it is the party that must parse it: `"<subject_ref>::<handler_name>@<version>"`,
  `subject_ref` a `DbRef` (VDB) or `GraphRef` (KG, resolved to its
  underlying `DbRef` via `kg-vdb`). Proposed as a doc-comment addition on
  `HandlerRef::ExecFn` in `queues-api.md`/`types::trigger`, flagged to the
  queues designer for concurrence, not adopted unilaterally into their file.

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
- **Schedule-only handler, no row/node (§3, wave-3):** a `TriggerSource::
  Schedule` trigger with `HandlerRef::ExecFn` fires a handler that touches no
  pre-existing row; the resulting `ee_invocations` row has `adapter: Direct`,
  `subject: SubjectKey::Direct{engine,function}`, `causation_id` equal to the
  occurrence's deterministic `event_id`, and **no matching `ee_changes`
  row** — the lineage walk correctly terminates at the `queues`-side
  occurrence rather than erroring.
- **Direct-dispatch dedup uses queues' `delivery_id`, not a derived one (§3):**
  the same `Deliver` is redelivered after a visibility-timeout expiry (no ack
  arrived); with `IdempotencyMode::DedupByEventId` the second attempt is
  skipped in `ee_deliveries` keyed by the SAME `delivery_id` queues assigned
  — no `uuid_v5(change_id, trigger_id)` is computed (there is no `change_id`).
- **Direct-dispatch failure escalates via queues' `DeadLetter`, not
  `LoopDepthExceeded` (§3):** a direct-dispatched handler that always fails
  exhausts `queues`' own `max_receive_count` and dead-letters through
  `queues`' redrive path; the engine issues no DLQ `SendEvent` of its own for
  this origin (contrast the change-bound path's `ee.dlq.<database>`) — assert
  no `ee.dlq.*` queue is touched and the escalation arriving at cc is
  `DeadLetter`, produced by queues, not by the engine.
- **Loop detection crosses the `Direct → Tables` boundary (§3):** a
  schedule-driven aggregation handler (`Direct` origin) writes rows whose
  table trigger (`Tables` origin) writes back and re-emits an event that
  re-invokes the SAME schedule-driven handler within the SAME causal chain
  (a contrived but possible cascade) → the repeat count on
  `(SubjectKey::Direct{engine,function}, handler)` accumulates across the
  boundary and parks at threshold, exactly as the `Tables ↔ Graph` crossing
  test does. Conversely, two SEPARATE schedule ticks of the identical
  trigger (different `correlation_id` each) never accumulate against each
  other — periodic re-firing is not a loop.

## Friction points (for the operator round)

1. **Hot-path latency through the seam (INTENT #29 consequence).** Every
   handler `ctx.sql` crosses host → vdb → `db` (cross-app WS per the lock).
   Correct by architecture, but per-statement round trips inside cascades
   will hurt; the batch-4 `vdb-db` contract should offer a
   session/batch-statement surface. Raised to the vdb/db designers, not
   resolved here.
2. **`Trigger.source` semantics for engine bindings — SUPERSEDED, resolved by
   the wave-3 fold.** Wave 2 flagged the shared struct's queues-native
   `queue: QueueName` field as slightly awkward for engine bindings. Batch 3
   generalized it to `source: TriggerSource` (`Queue`/`Schedule`) for
   unrelated reasons (the cron fold); the engine's `EngineTrigger` now sets
   `core.source = TriggerSource::Queue(<canonical pseudo-queue>)` — the same
   awkwardness, same shape, just riding the enum instead of a bare
   `QueueName`. Still workable, still slightly awkward, still flagged to the
   types/queues harmonizer — carried forward, not re-opened.
3. **Cloud targets weaken two guardrails — BLESSED (friction-round 2,
   INTENT #125).** On Supabase/RDS, the no-net sandbox and host-side egress
   proxy degrade to declared-and-audited (open platform egress);
   traced/idempotent/loop-bounded port intact via the generated shims +
   conformance suite, but the sandbox does not. The operator blessed the
   asymmetry along with no-net-by-default itself (#5 below).
4. **Two handler vocabularies exist during transition.** `lib/db`'s working
   `Contract`/outbox/edge machinery keeps serving db's own surface (incl.
   `ValidatorSync`, which the engine defers in v1); `HandlerDef` is its
   designed successor shape. Reconciliation direction: db's edge/outbox
   path becomes VDB's Supabase materialization substrate — needs the
   db-designer's confirmation this batch.
5. **No-net-by-default is stricter than the operator's Supabase habit —
   BLESSED (friction-round 2, INTENT #125).** Third-party calls require a
   per-handler host allowlist and flow through the host proxy (which is
   also how secrets stay unseen and egress gets provenance). The explicit
   operator yes arrived: "Fascinating idea. I like it. I'm OK with that."
   No longer open.
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
9. **`ExecFn.function` string convention — taste, not correctness (§3, wave-3).**
   `"<subject_ref>::<handler_name>@<version>"` is the boring choice (one
   opaque string, no wire change), but a typed alternative exists — e.g.
   widening `HandlerRef::ExecFn` itself to `{ engine: Slug, database:
   Option<DbRef>, graph: Option<GraphRef>, function: String }` — which is
   more self-describing at the cost of touching `queues-api`'s struct
   (queues owns it; a wire change, not a doc-comment). This design proposes
   the string-convention route as more boring (INTENT #124); flagged to the
   queues designer, not adopted unilaterally.
10. **Direct-dispatch and the `Locality`/replicated-database story (§3).**
    §6's `SemaphoreChoice::EventId` + `locks` composition exists because the
    engine's OWN outbox-drain can independently observe the same replicated
    change on multiple nodes. A `Direct`-origin invocation never has that
    problem — `queues` already collapsed multi-node delivery to one
    `Deliver` before the engine ever runs — so this design asserts §3's
    direct-dispatch path needs NO `locks` involvement at all, for any
    database `Locality`. Flagged in case a future replicated-KG-graph corner
    case proves this assertion wrong; no such case is known today.
