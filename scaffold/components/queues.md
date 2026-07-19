# queues

**Status:** NEW (wave 2). **Nesting:** internal lib of mesh (module
`lib/mesh::queues`), Ring 3 in `mesh-core.md`'s internal-layering architecture —
rides `replicated-kv` (Ring 2) for durable metadata, `locks` (Ring 3 sibling)
for the event-ID semaphore, and `pubsub-relay` (Ring 1) for its optional
observability tee. This file designs the round-6..9 `mesh.md` concern-14
requirements-only sketch into an implementation-ready contract fabric, and — per
the wave-2 batch note — **settles the declarative-trigger data model** the
`types` designer flagged as ours (batch 1, `types.md` "queues-api" boundary flag)
and that the batch-4 `execution-engine` designer must consume unchanged.

## Charter

`queues` is the **durable, at-least-once event-delivery fabric buried inside the
mesh daemon** and the home of the LOCKED four-term vocabulary (INTENT
#101/#103): typed **EVENTS** land in named **queues**; declarative **TRIGGERS**
(pure DATA — a filter expression plus a payload-assembly template, registered as
data and NEVER code) bind a queue one-to-one to a **HANDLER**, filtering by event
type and payload content and *assembling* the handler's input (optionally calling
`rollup` during assembly); **HANDLERS** are the only place executable code lives
and receive the trigger-assembled payload. It is modeled on Amazon SQS (INTENT
#89) — visibility timeouts, receive counts, redrive, and *a dead-letter queue is
just a queue* — so Mind OS can someday deploy straight onto real SQS through the
`aws` crate (INTENT #106). It approximates **exactly-once over at-least-once** by
composing with `locks`: pulling through a trigger that requests it acquires a
semaphore keyed by the event ID (INTENT #95), a **per-trigger choice** (INTENT
#101). Dead-letter exhaustion escalates into a **ccd agent investigation** (INTENT
#70/#89). "The entire contract of the system is basically built off the queueing
mechanism" — operator, verbatim.

**Boundary — what queues does NOT own.** It does not define the *event* struct,
the *provenance* triple, or (see the resolution below) the *trigger* struct's
declarative shape — those are `types` vocabulary (queues authors the trigger
shape, `types` houses it). It does not execute handler code: a handler is an
addressable mesh target (a service endpoint, a `pubsub-relay` topic, or an
`execution-engine` Deno/SQL function), and queues only *delivers the assembled
payload and awaits an ack*. It is not `pubsub-relay`: pub/sub is ephemeral,
best-effort, fan-out-to-current-subscribers observation (that boundary is
load-bearing — `pubsub-relay.md` concern 8); queues is durable, pull/dispatch,
guaranteed-at-least-once, DLQ-backed. It is not `execution-engine`: that is the
L4 data-plane trigger/handler engine for VDB rows and KG nodes; queues is the L2
generic mesh queueing fabric. The two are distinct engines that **share one
trigger data model** (below). It owns no application data (that is `db`/`vdb`),
no cloud logic (that is `aws`), and it never opens its own socket — it is a
compiled-in mesh lib (INTENT #54), reached only through the local `:3649` daemon.

## Primary design concerns

### 1. The unit of delivery is `(message × trigger)`, not the message — the crux

SQS delivers a message to one consumer. Mind OS's LOCKED vocabulary is **many
triggers per queue, each 1:1 to a handler** (INTENT #101) — so a single event can
fan out to several handlers. The load-bearing decision: the durable
lifecycle-tracked unit is a **`Delivery` = (message, trigger)** pair, not the
message alone. Consequences, all deliberate:

- Each matching trigger produces an independent `Delivery` with its **own**
  visibility timeout, receive count, redrive/DLQ state, and event-ID-semaphore
  choice. A slow or failing handler on trigger B never blocks trigger A on the
  same event, and each trigger's dead-lettering is independent.
- A message is **retained until every matched trigger's `Delivery` is terminal**
  (acked/deleted or dead-lettered). Message GC keys off "no live deliveries left."
- Filtering happens **before** a `Delivery` exists: a trigger whose filter rejects
  an event produces no delivery and no retained obligation for that trigger.

This is a *deliberate layer above strict SQS* (where fan-out is SNS→N-queues); the
someday-SQS mapping (concern 7) is therefore not perfectly 1:1 — flagged.

### 2. Declarative triggers — the data model (settles the flagged ownership)

INTENT #103 LOCKS triggers as **declarative DATA, never code**: a filter
expression + a payload-assembly template. Because a declarative structure is data,
it is **statically validatable at registration time** (a malformed filter/template
is rejected on `RegisterTrigger`, never at dispatch) — a property arbitrary code
could never give, and the reason "declarative" is worth the constraint.

**Ownership resolution (the batch-1 friction the `types` designer handed us).**
The `types` designer recommended the trigger struct live in `types` (inclusion
test: pure declarative data used by ≥2 crates — `queues` and `execution-engine` —
and part of the `queues-api` schema). **I accept and sharpen that:** the trigger
*data model* lives in a new `types::trigger` module, **its shape authored by this
(queues) design**; `queues` owns the *registry, evaluation, dispatch, and
lifecycle behavior*. Vocabulary in `types`, behavior in `queues`. This is exactly
the `pubsub.rs`-lives-in-`types` / relay-lives-in-`pubsub-relay` split batch 1
already used. Caveat carried to batch 4: `execution-engine` consumes
`types::trigger` **unchanged**; adapter-specific needs (a stack table name, a KG
node schema) go in an adapter extension struct, never bolted onto the core
`Trigger` (keeps the shared struct boring — `types` guardrail 3).

**Subject-generic filter/assembly (what lets one model serve both engines).** A
`FilterExpr` evaluates over, and an `AssemblyTemplate` reads from, a generic JSON
**subject document** plus a small set of well-known meta fields. Each *engine*
binds its subject into that document:

- `queues` binds an `Event` → `{ "type": <event_type>, "payload": <P>,
  "meta": { event_id, occurred_at, provenance } }`.
- `execution-engine` (batch 4) binds a row/node change → `{ "table"|"node_type",
  "op": insert|update|delete, "old", "new", "meta": {…} }`.

The AST and template are identical; only the subject binding differs. This is the
"one trigger data model" the wave-2 plan requires queues and execution-engine to
share.

```rust
// types::trigger  (shape authored here; module lives in `types`)
pub struct Trigger {
    pub trigger_id: TriggerId,
    pub queue: QueueName,             // the source queue (queues); a table/graph binding for exec-engine adapters
    pub handler: HandlerRef,          // the 1:1 dispatch target
    pub filter: FilterExpr,           // declarative — filters the subject document
    pub assembly: AssemblyTemplate,   // declarative — builds the handler payload
    pub semaphore: SemaphoreChoice,   // per-trigger event-ID-semaphore choice (INTENT #95/#101)
    pub idempotency: IdempotencyMode, // handler-side contract hint (concern 5)
    pub redrive: Option<Redrive>,     // per-trigger override of the queue default (concern 6)
    pub registered_by: Slug,
    pub version: LwwVersion,          // (wall_clock, node_id) — LWW like registry entries
}

pub enum FilterExpr {
    Always,
    EventType(EventTypeMatch),                       // Exact | Prefix over "domain.noun.verb"
    Field { path: JsonPath, op: CmpOp, value: serde_json::Value }, // payload/subject predicate
    And(Vec<FilterExpr>), Or(Vec<FilterExpr>), Not(Box<FilterExpr>),
}
pub enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge, In, Contains, Exists, Matches /*regex*/ }

pub struct AssemblyTemplate { pub root: TemplateNode }
pub enum TemplateNode {
    Literal(serde_json::Value),
    SubjectPath(JsonPath),            // copy from the event/payload subject
    Meta(MetaField),                  // event_id | event_type | occurred_at | provenance | correlation_id
    Rollup(RollupRef),               // declarative rollup reference (INTENT #94: Raw | Reference)
    Object(std::collections::BTreeMap<String, TemplateNode>),
    Array(Vec<TemplateNode>),
}

pub enum HandlerRef {
    Service { slug: Slug, route: String },  // push-deliver over mesh transport, await ack
    Topic(Topic),                           // deliver as a pubsub Envelope (fire-and-forget handlers)
    ExecFn { engine: Slug, function: String }, // execution-engine Deno/SQL handler
}
pub enum SemaphoreChoice { None, EventId, Custom(SemaphoreKeyTemplate) }
pub enum IdempotencyMode { HandlerIdempotent, DedupByEventId, AtMostOnceBestEffort }
```

`Rollup(RollupRef)` is the "assembly may call rollup" hook (INTENT #101), reusing
rollup's LOCKED raw-vs-reference insert types (INTENT #94) — the reference stays a
declarative pointer, never inlined code.

### 3. SQS-modeled message lifecycle (INTENT #89) — boring on purpose

Each `Delivery` walks the SQS state machine, tracked in `replicated-kv`:

```
available ──dispatch──▶ in-flight(visibility_deadline) ──ack──▶ deleted
   ▲                          │
   └──visibility expiry / nack─┘  (receive_count += 1)
                              │
             receive_count > max_receive_count
                              ▼
                        dead-letter queue  +  ccd escalation (concern 8)
```

- **Visibility timeout**: on dispatch the `Delivery` becomes in-flight with a
  deadline; a handler must ack (delete) before it. Expiry returns it to available
  and increments `receive_count` — plain SQS.
- **Redrive**: `max_receive_count` exceeded → the *event* is enqueued onto the
  configured DLQ (itself an ordinary queue, INTENT #95) and the `Delivery` is
  marked dead-lettered. A DLQ has triggers like any queue — including, typically,
  a trigger that escalates to ccd (concern 8).
- **Ordering**: unordered/standard-SQS by default (boring). FIFO is *not* offered
  in v1 — no operator ask for it; flagged as an open question, not a silent no.
- **Retention & tombstones**: acked/dead-lettered deliveries and orphaned messages
  are tombstoned with a GC-after TTL, mirroring `service-registry`'s tombstone
  discipline (so a re-anti-entropied stale entry can't resurrect a deleted message).

**QueueBackend seam (what makes someday-SQS structural, not aspirational).** The
lifecycle is expressed against a `trait QueueBackend` with a `LocalReplicatedKv`
impl now and an anticipated `SqsBackend` (in `aws`, INTENT #106) later. The
trigger/dispatch layer sits *above* the backend, so swapping the store to real SQS
is a backend swap, not a rewrite — the SQS-parity conformance test (below) guards
the semantics.

### 4. Distributed dispatch + the event-ID semaphore (compose with `locks`)

Queue metadata rides `replicated-kv`, which is **eventually consistent (naive LWW,
INTENT #32)** and replicated to every mesh daemon. So the *same* available message
is visible on multiple nodes, and any node's dispatcher could try to deliver it.
The event-ID semaphore is what makes that safe:

- A `Delivery` whose trigger declares `SemaphoreChoice::EventId` (or `Custom`)
  requires the dispatcher to **acquire a `locks` semaphore keyed by the event ID**
  (INTENT #95) *before* dispatching, and hold it until ack/delete. Because `locks`
  distributes acquisition knowledge to all reachable nodes **before** confirming
  the hold (INTENT #71), only one node dispatches that `(event, trigger)` at a
  time — the ~exactly-once mechanism.
- A trigger with `SemaphoreChoice::None` accepts naive at-least-once (cheaper, no
  lock round-trip) — the per-trigger choice (INTENT #101). Handlers behind such
  triggers MUST be idempotent (concern 5).
- **CAP honesty (INTENT #84) is first-order here.** If two partitions each
  dispatched the same event (each acquiring a partition-twin semaphore, INTENT
  #71's slug+UUID twins), then on merge `locks` raises the **catchable
  partition-merge error** (`LockError::PartitionMergeThresholdExceeded`, defined
  in `types` per `types.md`). Queues must **surface that error, not silently
  double-dead-letter**: it is delivered as a first-class delivery outcome
  (`DeliveryOutcome::PartitionConflict`) so the per-application seam (or a ccd
  escalation) handles it. This is the concrete place the operator's "handled per
  application" seam lives.

**Queue ownership model — a genuine fork (flagged).** The above is the
*replicated-everywhere + semaphore* model. An alternative is *single-owner-node
per queue with failover* (one daemon leases the queue via `locks`, is the only
dispatcher, hands off on death) — simpler, no cross-node double-dispatch window,
but loses "drain anywhere" resilience if the owner is unreachable. I lean
replicated-everywhere-plus-semaphore (matches INTENT #32's blessed naive LWW and
`locks`' whole reason to exist), but this is a real design fork for the operator —
see Friction points.

### 5. Idempotency is a load-bearing handler contract, not a nicety

Because delivery is at-least-once and exactly-once is only *approximated* (the
semaphore closes the common window; a `locks` partition still admits a duplicate —
concern 4), **handlers must be idempotent** — the operator's per-case
duplicate-handling seam (INTENT #95). Queues makes this a *declared contract*, not
folklore: every `Trigger` carries an `IdempotencyMode`, and every handler-delivery
payload carries the `event_id` + `correlation_id` (from `Provenance`) so a handler
can dedup. `DedupByEventId` lets queues offer a best-effort dedup cache keyed by
event ID as a convenience, but the invariant is stated plainly in the `queues-api`
conformance requirement: *a handler that is not idempotent is a bug against this
contract.* This is the honest counterpart to naive-LWW-on-KV.

### 6. Redrive / dead-letter — "a DLQ is just a queue" (INTENT #95)

There is no special dead-letter mechanism. A queue (or per-trigger override)
declares `Redrive { dlq: QueueName, max_receive_count }`. On exhaustion the *event*
is `SendEvent`-ed onto `dlq` (an ordinary queue with its own triggers), and the
original `Delivery` is marked dead-lettered with its `failure_history`. Because the
DLQ is an ordinary queue, escalation-to-ccd is *itself just a trigger* on the DLQ
(concern 8) — no bespoke code path. This is the cleanest possible expression of the
operator's lock: the guardrail reuses the fabric it guards.

### 7. Someday-SQS deployability (INTENT #89/#106)

The `QueueBackend` seam + the SQS-parity conformance test keep the message
lifecycle a strict subset of SQS's, so an `aws::SqsBackend` is a drop-in for the
store. The **fan-out layer** (concern 1's `message × trigger`) does *not* map 1:1
onto SQS — the someday mapping is: each `Trigger` becomes an SQS event-source
mapping (Lambda-style) *or* an SNS-fan-out to a per-trigger SQS queue, and the
event-ID semaphore maps onto SQS FIFO message-group dedup or a DynamoDB
conditional-put. This is a faithful-but-not-mechanical mapping; the `aws` designer
(batch 3, someday-only) authors the `aws`-side adapter and I flag the non-1:1 fan-out
for that pass.

### 8. Dead-letter → ccd escalation, shared with execution-engine's loop-depth hook

INTENT #70/#89: dead-letter exhaustion and `execution-engine`'s loop-depth-exceeded
condition are **the same escalation pattern** — spawn a ccd agent to investigate.
Queues proposes ONE shared shape (`ccd-escalation`, below) carrying a union of the
two triggering conditions, so ccd (batch 6) grows one investigation surface, not
two. On the queues side the escalation is fired by an ordinary DLQ trigger whose
`HandlerRef::Service { slug: "ccd", … }` delivers the `EscalationRequest` — the
guardrail is itself declarative data, consistent with everything else here.

### 9. Optional observability tee onto `pubsub-relay` (accepting the flagged nudge)

`pubsub-relay.md` concern 8 left open whether a queue mirrors its state-change
events onto a `queue.*` pub/sub topic. **Accepted: an opt-in, one-way tee** (per
`QueueConfig.tee_topic`), a publish-and-forget mirror for dashboards/observability,
never a dependency and never a delivery guarantee (pub/sub is lossy by contract).
Off by default; enabling it makes queue depth / in-flight / dead-letter counts
visible on the boring surface schema without coupling the two libs.

## Relationships / edges

Contract edges (cross-process WS through the local `:3649` daemon). Internal-lib
relationships (`replicated-kv`, `locks`, `pubsub-relay`) are compiled-in Ring
seams per `mesh-core.md`, **not** contract edges (INTENT #45).

- **any service ↔ mesh.queues** via `queues-api` — publish typed events, register/
  update/deregister declarative triggers, and the handler-delivery (ack/nack) side.
  Cross-cutting, surface-schema-style (one shared document, every service a party).
  *(authored: scaffold/contracts/queues-api.md)*
- **mesh.queues(DLQ) / execution-engine → ccd** via `ccd-escalation` — the shared
  dead-letter + loop-depth investigation surface (one union shape).
  *(authored: scaffold/contracts/ccd-escalation.md)*
- **mesh.queues(triggers) → rollup** via `rollup-mesh` — a trigger's
  `AssemblyTemplate` resolves declarative `Rollup(RollupRef)` references at assembly
  time; queues calls rollup over mesh. queues is the *consumer*; `rollup` (batch 4)
  authors `rollup-mesh` (+ its own registration). Consumer-side note below.
  *(authored: scaffold/contracts/rollup-mesh.md)*
- **`types`** — library dependency, NOT a contract edge: `Event`, `EventType`,
  `Provenance` (existing) + the new `types::trigger` module (`Trigger`, `FilterExpr`,
  `AssemblyTemplate`, `SemaphoreChoice`, `HandlerRef`, …) whose shape this design
  authors. Proposed to the batch-1 `types` designer via the `queues-api` boundary
  flag they raised.
- **`locks`** (Ring-3 sibling) — the event-ID semaphore composition (concern 4) and
  the queue-owner lease (if the single-owner fork is chosen). Consumed in-process
  via `locks`' `trait` seam; `locks` owns `locks-api` and the partition-merge error
  type. Co-batched (batch 2, `queues ⇄ locks`) — I depend on that error type and
  the acquire/release surface; flagged for mid-batch draft-sharing.
- **`replicated-kv`** (Ring-2) — durable queue/message/delivery/trigger metadata via
  namespaced LWW keyspaces. Consumed via the `KvHandle` seam; not a contract edge.
- **`pubsub-relay`** (Ring-1) — the opt-in `queue.*` observability tee (concern 9).
  In-process; not a contract edge.
- **`execution-engine`** (batch 4, L4) — NOT a queues contract edge; the binding is
  the **shared `types::trigger` data model** (concern 2). Flagged for batch-4
  co-design: execution-engine must consume it unchanged.

## Nesting

Parent: mesh | Children: none. Server side lives in `lib/mesh::queues`; the client
half (publish/register-trigger/handler-delivery handle to the local daemon) is part
of `mesh-client`'s surface — services get queues through the same thin boot lib
they get `register`/`resolve`/`pubsub` from (per `mesh-core.md` and
`pubsub-relay.md` nesting). Confirmed at skeleton time.

## Thoroughness level

**implementation-ready** — the `(message × trigger)` delivery unit, the declarative
trigger data model + subject-generic filter/assembly, the SQS-modeled lifecycle +
`QueueBackend` seam, the `locks` event-ID-semaphore composition with the
partition-merge CAP-honesty path, the per-trigger semaphore/idempotency/redrive
choices, DLQ-is-just-a-queue with ccd escalation as a declarative DLQ trigger, and
the pub/sub tee are all decided and specified. The three proposed contract *wire
shapes* (`queues-api`, `ccd-escalation` DLQ half, `rollup-mesh` consumer view) are
**approach-sketched** — fields authored, reconciled in the per-pair round
with `locks` (error type), `rollup` (assembly-resolve API), `execution-engine`
(shared trigger model), and `ccd` (escalation receiver). Two genuine forks are left
for the operator, not silently chosen (queue-ownership model; FIFO) — see Friction.

## Assigned design-depth

Opus (single strong-model Component-Designer pass, this file), grounded in
`mesh.md` concern 14 (round-6..9), `types.md`/`pubsub-relay.md`/`mesh-core.md`
(batch 1), the co-batched `locks`/`replicated-kv`/`service-registry` charters
(wave2-plan §2), and INTENT items 32/70/71/84/89/94/95/101/103/106. The
declarative-trigger data model was the candidate for a Fable escalation
(wave2-plan module #10 note); it resolved cleanly as boring subject-generic data
(concern 2), so no Fable step was needed.

## Suggested fill-model

**implementation-ready + high complexity → mid-to-strong model.** Three surfaces
split cleanly by risk: (a) the `types::trigger` structs + `FilterExpr` evaluator +
`AssemblyTemplate` renderer are near-spec, boring, and heavily testable →
**mid model** with the conformance fixtures. (b) the SQS lifecycle state machine +
`LocalReplicatedKv` backend is a careful-but-boring transcription of SQS semantics
against the conformance test → **mid model**. (c) the distributed-dispatch +
event-ID-semaphore + partition-merge path (concern 4) is the one subtle correctness
spot — cross-node double-dispatch and the `locks` partition-merge error → **strong
model or a focused review pass**; do NOT send this surface to a cheap tier. queues
must be filled *after* `locks` and `replicated-kv` (it compiles against their
seams).

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them.

- `queues-api` (any service via `mesh-client` ↔ mesh.queues; cross-cutting, one
  shared document; queues owns) — queue management, `SendEvent`, declarative
  trigger register/update/deregister, and the push-dispatch delivery + ack/nack
  side; durable/at-least-once, the guaranteed counterpart to
  `pubsub-protocol`'s lossy contract. → `scaffold/contracts/queues-api.md`
  - Contract resolution: the flagged trigger-struct home question closed
    concordant — home = `types::trigger`, shape authored by queues (shared
    unchanged with execution-engine).
  - Still open in the contract, operator call: the queue-ownership fork
    (replicated-everywhere + event-ID semaphore, which the schema assumes, vs
    single-owner-node with failover).
- `ccd-escalation` (producers mesh.queues + execution-engine → consumer ccd;
  one union `EscalationRequest` shape) — queues authors the `DeadLetter` arm,
  fired by an ordinary DLQ trigger. → `scaffold/contracts/ccd-escalation.md`
  - Contract resolution: execution-engine's enriched `LoopDepthExceeded` arm
    won over the thin placeholder once sketched here (kept only as the
    back-compat deserialize floor); `EscalationAck::Investigating` carries
    ccd's `ThreadId`, not a bare `String`.
- `rollup-mesh` (rollup ↔ mesh; queues is the chief resolve-surface caller;
  rollup authors) — a trigger `AssemblyTemplate`'s `Rollup(RollupRef)` nodes
  resolve at assembly time via rollup's resolve surface; `llm_safe`
  degradation passes through, and a degraded/failed resolve is
  `AssemblyFailed`, never a leak. → `scaffold/contracts/rollup-mesh.md`
  - Contract resolution: the operation is canonically **`ResolveRefs`**
    (rollup's name; queues' `ResolveReferences` recorded as the alias) —
    queues' fill imports `ResolveRefs`.

Also a party to the cross-cutting `pubsub-protocol` (the opt-in `queue.*`
observability tee) — see `scaffold/contracts/pubsub-protocol.md`.

## Non-obvious tests (conformance + correctness)

- **Cross-node exactly-once via semaphore:** two mesh daemons both hold the same
  available message (replicated-kv); a trigger with `SemaphoreChoice::EventId` →
  the handler is invoked **exactly once** (the `locks` semaphore de-dupes the
  dispatch across nodes). With `SemaphoreChoice::None` on the same setup, the
  handler may be invoked twice → an idempotent handler yields a single effect.
- **Visibility-timeout redelivery:** dispatch, let the visibility deadline lapse
  without an ack → `receive_count` increments and the delivery returns to available;
  a late `AckDelivery` returns `MessageNotInFlight` (the double-write window that
  makes idempotency load-bearing — concern 5).
- **Partition-merge honesty (INTENT #84):** two partition twins each dispatched the
  same event (each acquired a UUID-twin semaphore); on merge `locks` raises
  `PartitionMergeThresholdExceeded` → the delivery surfaces
  `DeliveryOutcome::PartitionConflict`, and the event is NOT silently
  double-dead-lettered.
- **Independent fan-out lifecycles:** one event, two triggers on one queue; trigger
  A's handler acks, trigger B's handler fails past `max_receive_count` → B's event
  dead-letters and escalates while A completes cleanly; the message is retained
  until both deliveries are terminal, then GC-tombstoned.
- **DLQ is just a queue:** the dead-lettered event on the DLQ fires the DLQ's own
  ccd-escalation trigger exactly once (deduped by `escalation_id`); the DLQ itself
  has visibility/retention/triggers like any queue.
- **Declarative static validation:** `RegisterTrigger` with a malformed `FilterExpr`
  or an `AssemblyTemplate` referencing a non-existent `SubjectPath` shape is rejected
  at registration (`InvalidFilterExpr`/`InvalidAssemblyTemplate`), never at dispatch.
- **Assembly + rollup:** a template with a `Rollup(Reference)` node resolves to a
  handle; a template feeding an LLM-classified handler with a `secrets` raw
  reference degrades/fails per `llm_safe` (no secret in the assembled payload —
  INTENT #94); a failed rollup resolve → `AssemblyFailed`, the delivery redrives.
- **SQS-parity backend swap:** the `QueueBackend` conformance suite (visibility
  timeout, receive-count increment, redrive threshold, retention) passes identically
  against `LocalReplicatedKv` and a stubbed `SqsBackend`, proving the someday-SQS
  drop-in (INTENT #89/#106).
- **Mixed-version tolerance:** an older daemon receiving a `Trigger` carrying an
  unknown `FilterExpr`/`TemplateNode` variant (`#[serde(other)]`) fails *that
  trigger* loudly and locally without crashing the queue or dropping sibling
  triggers; an unknown `EventType` string relays/persists untouched.
- **Shared trigger model (batch-4 guard):** `execution-engine` binds a row-change
  subject into the same `FilterExpr`/`AssemblyTemplate` and evaluates identically to
  a queues event subject (one data model, two subject bindings — concern 2).
