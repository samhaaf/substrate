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

**Wave-3 fold (ledger batch 3, D2/OQ-3/OQ-6).** Three changes land here, none of
which alters the LOCKED declarative-trigger vocabulary (#101/#103): (a) **`cron`
is absorbed** as a `Schedule` trigger *source* (concern 10) — `components/cron.md`
and `contracts/cron-api.md` are tombstoned into this file and `queues-api.md`
(INTENT #56/#91/F6b); the Ring-4 cron evaluator collapses into this Ring-3 lib
with no new dependency. (b) queues **consumes `types::delivery` vocabulary**, but
the queue side of `SaveFailed` is **native durability** — concern 11 writes the
honest one-page explanation of the PARKED queues-as-pubsub-with-persistence
consolidation (OQ-3) **without resolving it**. (c) **keeper→keeper messages ride
queues durably** (approvals must not vanish) — concern 12 states the contract seam
to the batch-6 keeper runtime **only**, not the (PARKED, OQ-6) approval protocol.

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
#101). Dead-letter exhaustion escalates into a **cc agent investigation** (INTENT
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
    pub source: TriggerSource,        // WAVE-3 FOLD: Queue(event-driven) | Schedule(time-driven, absorbed cron)
    pub handler: HandlerRef,          // the 1:1 dispatch target (incl. Emit, the cron-emission variant)
    pub filter: FilterExpr,           // declarative — filters the subject document
    pub assembly: AssemblyTemplate,   // declarative — builds the handler/emitted payload
    pub semaphore: SemaphoreChoice,   // per-trigger event-ID-semaphore choice (INTENT #95/#101)
    pub idempotency: IdempotencyMode, // handler-side contract hint (concern 5)
    pub redrive: Option<Redrive>,     // per-trigger override of the queue default (concern 6)
    #[serde(default = "default_true")]
    pub enabled: bool,                // WAVE-3 FOLD: pause/resume a trigger (absorbs CronJob.enabled)
    pub registered_by: Slug,
    pub version: LwwVersion,          // (wall_clock, node_id) — LWW like registry entries
}

// WAVE-3 FOLD (concern 10): a trigger's SOURCE is now an enum — the only change
// the cron absorption makes to the model. Queue = today's behavior verbatim.
pub enum TriggerSource {
    Queue(QueueName),                 // event-driven: fires per matching event landing in this queue
    Schedule(ScheduleSource),         // time-driven: fires per scheduled occurrence (absorbed cron)
}
pub struct ScheduleSource {           // vocabulary moved intact from the tombstoned cron-api
    pub schedule: Schedule,           // Cron{expr,tz} | Every{interval,anchor} | Once{at}
    pub target: FireTarget,           // Anywhere (single-fire, raced) | Node(NodeId) (pinned)
    pub misfire: MisfirePolicy,       // Skip | FireOnWake { grace, coalesce }
}
pub enum FireTarget { Anywhere, Node(NodeId), #[serde(other)] Unknown }
pub enum Schedule {
    Cron  { expr: String, #[serde(default)] tz: Option<String> },
    Every { interval: Duration, #[serde(default)] anchor: Option<DateTime<Utc>> },
    Once  { at: DateTime<Utc> },
    #[serde(other)] Unknown,          // fail-safe: an older node never fires an unknown kind
}
pub enum MisfirePolicy {
    Skip,
    FireOnWake { #[serde(default)] grace: Option<Duration>,
                 #[serde(default = "default_true")] coalesce: bool },
    #[serde(other)] Unknown,
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
    Emit { queue: QueueName, event_type: EventType }, // WAVE-3 FOLD: emit the assembled payload as
                                            // an EVENT into a queue — the cron emission (concern 10),
                                            // also a generic event-router action for Queue-source triggers
}
pub enum SemaphoreChoice { None, EventId, Custom(SemaphoreKeyTemplate) }
pub enum IdempotencyMode { HandlerIdempotent, DedupByEventId, AtMostOnceBestEffort }
```

`Rollup(RollupRef)` is the "assembly may call rollup" hook (INTENT #101), reusing
rollup's LOCKED raw-vs-reference insert types (INTENT #94) — the reference stays a
declarative pointer, never inlined code. **The assembly-calls-rollup path is
UNCHANGED by the wave-3 fold** (ledger item d): a `Schedule`-source trigger
assembles its emitted payload with the identical `AssemblyTemplate`, so a
scheduled fire can call rollup at assembly exactly as an event-driven trigger can.

**Subject binding for a `Schedule` source (concern 10).** An event-driven
(`Queue`) trigger binds an `Event` into the subject document (above). A
`Schedule` trigger binds the **fired occurrence**:
`{ "type": "schedule.fired", "payload": <ScheduleSource static payload>,
"meta": { occurrence_id, scheduled_for, fired_at, fire_node, catch_up } }`. The
`FilterExpr`/`AssemblyTemplate` AST is identical — a third subject binding
alongside `queues`' event and `execution-engine`'s row/node change (concern 2),
so cron folds in *without a second evaluator language*.

### 3. SQS-modeled message lifecycle (INTENT #89) — boring on purpose

Each `Delivery` walks the SQS state machine, tracked in `replicated-kv`:

```
available ──dispatch──▶ in-flight(visibility_deadline) ──ack──▶ deleted
   ▲                          │
   └──visibility expiry / nack─┘  (receive_count += 1)
                              │
             receive_count > max_receive_count
                              ▼
                        dead-letter queue  +  cc escalation (concern 8)
```

- **Visibility timeout**: on dispatch the `Delivery` becomes in-flight with a
  deadline; a handler must ack (delete) before it. Expiry returns it to available
  and increments `receive_count` — plain SQS.
- **Redrive**: `max_receive_count` exceeded → the *event* is enqueued onto the
  configured DLQ (itself an ordinary queue, INTENT #95) and the `Delivery` is
  marked dead-lettered. A DLQ has triggers like any queue — including, typically,
  a trigger that escalates to cc (concern 8).
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
  (`DeliveryOutcome::PartitionConflict`) so the per-application seam (or a cc
  escalation) handles it. This is the concrete place the operator's "handled per
  application" seam lives.

**Queue ownership model — RESOLVED, LOCKED (friction-round 1, INTENT #112).**
The fork is closed: **replicated-everywhere + event-ID semaphore, all nodes
process, discovery-claims-ownership** — exactly the model this file designed.
Operator, verbatim: "I don't think having one processor makes sense. All
processors — whoever discovers it can try to claim ownership. The semaphore,
when you try to acquire it, creates a UTC timestamp down to the nanosecond —
whoever gets it." The single-owner-node-with-failover alternative (one daemon
leases the queue via `locks`, is the only dispatcher, hands off on death) is
REJECTED. A few seconds of semaphore-acquisition latency is explicitly fine —
the operator's reliability-over-speed rationale, verbatim:

> "What we're going for is reliability, not speed. The way we maximize our
> leverage isn't by getting from three seconds down to a tenth of a second —
> it's by keeping things running all the time and having our leverage create
> more leverage."

And his partition insight, resolving the double-dispatch worry directly:
queue-pull semaphore collisions between disjoint partitions aren't a real
worry — **if the event reached both nodes, those nodes were connected.** (The
`locks` partition-merge error path above remains as the honest backstop for
the residual case, but it is a backstop, not a design driver.)

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
DLQ is an ordinary queue, escalation-to-cc is *itself just a trigger* on the DLQ
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

### 8. Dead-letter → cc escalation, shared with execution-engine's loop-depth hook

INTENT #70/#89: dead-letter exhaustion and `execution-engine`'s loop-depth-exceeded
condition are **the same escalation pattern** — spawn a cc agent to investigate.
Queues proposes ONE shared shape (`cc-escalation`, below) carrying a union of the
two triggering conditions, so cc (batch 6) grows one investigation surface, not
two. On the queues side the escalation is fired by an ordinary DLQ trigger whose
`HandlerRef::Service { slug: "cc", … }` delivers the `EscalationRequest` — the
guardrail is itself declarative data, consistent with everything else here.

### 9. Optional observability tee onto `pubsub-relay` (accepting the flagged nudge)

`pubsub-relay.md` concern 8 left open whether a queue mirrors its state-change
events onto a `queue.*` pub/sub topic. **Accepted: an opt-in, one-way tee** (per
`QueueConfig.tee_topic`), a publish-and-forget mirror for dashboards/observability,
never a dependency and never a delivery guarantee (pub/sub is lossy by contract).
Off by default; enabling it makes queue depth / in-flight / dead-letter counts
visible on the boring surface schema without coupling the two libs.

### 10. Scheduled triggers — cron absorbed as a `Schedule` trigger source (WAVE-3 FOLD, INTENT #56/#91/F6b)

The wave-2 `cron` lib (`components/cron.md`, now a tombstone) is folded in here.
The observation that makes it boring: cron was already *"decide when to fire, then
emit a standardized event — nothing more,"* and a trigger was already *"filter +
assemble a subject → dispatch."* The only difference was the **source of firing**.
So a trigger's `queue: QueueName` field generalizes to `source: TriggerSource`
(concern 2), and cron becomes `TriggerSource::Schedule(ScheduleSource)`. Nothing
else in the trigger model changes; the LOCKED declarative-trigger and per-trigger-
semaphore vocabulary (#101/#103) is untouched (ledger item b). What this buys and
how the semantics carry over intact:

- **The two FireTarget flavors carry over verbatim.** `FireTarget::Node(N)` is
  evaluated only by node N's schedule evaluator (exactly one candidate firer, no
  semaphore); `FireTarget::Anywhere` is raced by every reachable daemon and made
  single-fire by the semaphore below. One field on `ScheduleSource`, not two API
  paths — the same virtualized-vs-pinned split as mesh addressing (#59).
- **Single-fire via the deterministic event-ID semaphore — the SAME mechanism
  queues already has (#71/#95).** A run-anywhere occurrence is content-addressable:
  `occurrence_id = uuid_v5(SCHEDULE_NS, trigger_id ++ scheduled_for.rfc3339())`.
  The firing daemon acquires the `locks` semaphore keyed by `occurrence_id`
  (threshold 1) before firing; the loser daemons drop silently. The **same
  `occurrence_id` becomes the emitted event's `event_id`**, so if the trigger emits
  (below) and a downstream trigger opts into `SemaphoreChoice::EventId`, both the
  fire-side single-fire and the consume-side ~exactly-once de-dup against one
  identical key. Cron never invented a concurrency primitive; now it literally
  reuses queues' `locks` composition (concern 4). CAP honesty is identical: two
  partitions may each fire an `Anywhere` occurrence under a partition-twin
  semaphore; because both carry the identical `event_id`, the emit can collapse to
  one delivery on merge if the target queue dedups by `event_id` (concern 4's
  `PartitionConflict` backstop applies to the emitted delivery).
- **The schedule evaluator becomes a Ring-3 concern inside queues, needing no new
  dependency.** Each daemon runs one periodic evaluator that scans `enabled`
  `Schedule`-source triggers, computes due occurrences in `(cursor, now]` per the
  flavor + misfire rules, acquires the semaphore for `Anywhere`, and fires. cron
  was Ring 4 and rode `locks` + `replicated-kv` + `service-registry` + `queues`;
  queues (Ring 3) already rides `locks` (sibling), `replicated-kv` (Ring 2), and
  `service-registry` (sibling), and it *is* the emit target — so the fold
  **removes a ring level** (`mesh-core.md` § Internal layering: Ring 4 loses
  `cron`), a strict simplification. Clock handling stays naive wall-clock per node
  (#32); skew only changes *which* node wins the semaphore, never *whether* an
  occurrence double-fires.
- **Fire cursor lives in a separate keyspace — a deliberate refinement over cron.**
  cron wrote `last_fired` back into the `CronJob` row by LWW. A schedule trigger
  instead advances its cursor in `queues/schedule-state/<trigger_id>` (LWW,
  advisory, self-healing: a stale cursor just recomputes occurrences and re-attempts
  already-taken semaphores — a safe no-op). Rationale: a fire must never LWW-race a
  concurrent *definition* edit to the same trigger row. The `MisfirePolicy`
  (`Skip` default vs `FireOnWake { grace, coalesce }`) and the `catch_up: true`
  flag on catch-up fires are unchanged; catch-up occurrences ride the same
  occurrence-keyed semaphore, so "fire once on wake" is single-fire across the
  fleet for free.
- **The pg_cron-replacement decomposition (#91) is unchanged, now one lib.** A
  schedule trigger's `HandlerRef::Emit { queue, event_type }` publishes the
  assembled `schedule.fired` event into a target queue (never onto lossy pub/sub —
  scheduled work needs durability); a downstream event-driven trigger filters and
  assembles it; a stack/VDB handler runs the SQL. queues touches no database; the
  three-stage decomposition (schedule → trigger → handler) is intact. *(The more
  aggressive collapse — a schedule trigger dispatching `Service`/`ExecFn`
  **directly**, skipping the queue hop — is expressible with the same struct but is
  NOT the folded default: emitting an event preserves cron's fan-out property, keeps
  handlers database-agnostic, and matches the ledger's "a cron firing is just a
  scheduled event emission." Direct dispatch is left available, not adopted.)*

### 11. Delivery-persistence — queues durability is NATIVE; the pubsub `IntermediateCache` relationship (INTENT #155 / PARKED OQ-3)

queues consumes the `types::delivery::DeliveryPersistence` vocabulary (batch-1
`types`), but on the queue side the flag is **near-vacuous by construction**: a
queue message is *already* durably persisted in `replicated-kv` and retried at-
least-once with a DLQ (the whole charter). "Save failed deliveries" is what queues
*is* — `DeliveryPersistence::SaveFailed` describes queues' native behavior, and
`LossyDrop` has no queue meaning (queues never silently drops; the closest analog,
dead-lettering, is itself a durable save). So queues neither reads nor branches on
the field for its own traffic; the field is meaningful only for **pub/sub**, which
*can* be lossy and must opt into durability. This asymmetry is the whole reason the
consolidation the operator flagged is a live question rather than a settled one —
and per the ledger (item c, §C OQ-3) this concern's job is to write **one honest
page** explaining what that consolidation would and would not mean, **without
deciding it**.

**The consolidation, stated precisely (F5 / SB7a).** pub/sub's `SaveFailed`
retention target is the abstract `IntermediateCache` seam (`pubsub-relay.md`
concern 9): a would-be-dropped envelope is teed into the cache and re-offered on
reconnect. The parked proposal is to **back that seam with `queues`** — i.e. a
failed `SaveFailed` pub/sub delivery is `SendEvent`-ed into a durable queue keyed
by the intended recipient, and drained on reconnect. Today the seam's boring
provisional is instead a distributed KV-backed cache (INTENT #155's own words),
kept deliberately un-merged.

**What the consolidation WOULD buy (the genuine pull):**
- *One durability engine.* No second "intermediate-response cache" concept; queues'
  already-designed at-least-once + dedup-by-id + retention/TTL + DLQ machinery backs
  both explicit queue traffic and pub/sub's save-failed path. Less surface area.
- *Exactly-once-ish for saved pub/sub deliveries for free*, via the same event-ID
  semaphore (concern 4), if a recipient wants it.
- *A single mental model:* "anything that must not vanish is a queue message."

**What it WOULD NOT mean / the honest costs (why it "didn't seem very boring"):**
- **It does NOT make pub/sub durable.** The broker stays lossy; `LossyDrop` is
  untouched; only the opt-in `SaveFailed` tee changes its backing store. The two
  wire protocols (`pubsub-protocol` fan-out vs `queues-api` trigger/handler/ack)
  stay **separate contracts** — the consolidation is only about *where saved
  deliveries land*, never about merging the APIs.
- **It inverts the mesh ring layering — the concrete un-boring core.**
  `pubsub-relay` is **Ring 1**; `queues` is **Ring 3** (`mesh-core.md` § Internal
  layering: "a ring may consume only lower rings"). queues→pubsub (the observability
  tee, concern 9) is Ring 3 → Ring 1, *allowed*. But backing pubsub's cache with
  queues is **pubsub (Ring 1) → queues (Ring 3)** — a lower ring consuming a higher
  one, skipping past `replicated-kv` and `locks`. That is the sharpest layering
  inversion in the mesh, and it makes pub/sub's liveness-oriented resiliency depend
  on the *entire* durable-queue subsystem (visibility timeouts, redrive, DLQ,
  cc-escalation) — far more machinery than a save-and-re-offer cache needs. The
  distributed-KV backing keeps the dependency minimal (a KV put/get near Ring 2)
  and the layering closer to honest.
- **Semantic mismatch.** A `SaveFailed` re-offer is *"hold this exact envelope until
  the recipient reconnects, then replay it"* — a per-recipient retained log with a
  cursor. A queue is *pull / dispatch / visibility-timeout / ack / redrive / DLQ*.
  Forcing a save-failed pub/sub envelope through the queue lifecycle raises awkward
  questions with no natural answer: does an un-drained save-failed envelope
  dead-letter? does it cc-escalate? what is its visibility timeout when there is no
  handler, only a future re-subscribe? The shapes do not line up.
- **The genuinely shared substrate is LOWER than either.** Both queues' native
  durability and pub/sub's `SaveFailed` cache want the *same* primitives: a durable
  per-key store, dedup-by-id, at-least-once replay, TTL/eviction — and both already
  sit **above `replicated-kv` (Ring 2)**. So the boring common ground, if any is
  wanted, is "both durability paths share the `replicated-kv` substrate and the
  dedup-by-id + TTL discipline," **not** "pub/sub enqueues through the whole queues
  lib." The un-boring version routes pub/sub through a Ring-3 lib; the boring
  version would share a Ring-2 primitive. That is the precise distinction for the
  operator to react to.

**Stance (NEEDS-EXPLANATION, not decided — §C OQ-3).** queues does **not** adopt
the consolidation and does **not** thread a pub/sub dependency into itself; the
`IntermediateCache` seam stays off the contract graph (`pubsub-relay.md`). If the
operator un-parks OQ-3 in favor of the queues backing, the change is additive and
local: pub/sub's seam impl calls `SendEvent` on a recipient-keyed queue and drains
via `ReceiveDeliveries` on reconnect — a one-binding swap, exactly the reversibility
the ledger requires. Until then both framings stay un-merged.

### 12. Keeper→keeper messages ride queues durably — contract seam only (INTENT #149/#88/#127; PARKED OQ-6)

The batch-6 **keeper** runtime (`components/keeper.md`, the wave's central L6
design, replaces `org`) needs keeper-to-keeper **proposals and approvals** to
survive thread compaction **exactly-once** — *"approvals must not vanish"* (ledger
item e; synthesis F-4/B-r4; §C OQ-5/OQ-6). That durability requirement lands here:
such messages ride **durable queues**, exactly-once via the event-ID semaphore
(concern 4), the same guarantee cron and the pg_cron path use. queues provides the
seam; it does **not** design the protocol.

- **What queues contracts (the seam).** A keeper-message queue family (boring
  provisional name `keeper.<keeper_id>.inbox`, one durable queue per keeper) carrying
  typed keeper-message events; delivery is at-least-once with `SemaphoreChoice::EventId`
  for the exactly-once approval semantics; the message's `event_id` is the idempotency
  key so a re-delivered approval applies once. The keeper runtime (batch 6) is the
  **consumer** — it registers the triggers/handlers that drain its inbox and drive
  its propose→approve→dispatch loop. This is an ordinary use of `queues-api`; no new
  wire contract, only a documented consumer relationship (below).
- **What queues MUST NOT decide (PARKED OQ-6, §C).** The keeper-to-keeper message +
  approval **protocol** — the #88 negotiation state machine (propose → deliberate →
  counter → tweak → agree), #127's approval gate, the human-in-loop vs no-human
  variants (#149 beat 11), and which bundle-curation writes an approval triggers —
  is *"needs its own conversation."* queues leaves the keeper runtime a
  `propose→approve→dispatch` placeholder; the approval *wire* is a named-but-
  unspecified seam. queues guarantees only that whatever messages the protocol emits
  **do not vanish across compaction**, because they are durable queue events. The
  message payload types are the keeper design's to author (`types::keeper` or a
  keeper-local module), consumed by queues opaquely like any event payload.

## Relationships / edges

Contract edges (cross-process WS through the local `:3649` daemon). Internal-lib
relationships (`replicated-kv`, `locks`, `pubsub-relay`) are compiled-in Ring
seams per `mesh-core.md`, **not** contract edges (INTENT #45).

- **any service ↔ mesh.queues** via `queues-api` — publish typed events, register/
  update/deregister declarative triggers, and the handler-delivery (ack/nack) side.
  Cross-cutting, surface-schema-style (one shared document, every service a party).
  *(authored: scaffold/contracts/queues-api.md)*
- **mesh.queues(DLQ) / execution-engine → cc** via `cc-escalation` — the shared
  dead-letter + loop-depth investigation surface (one union shape).
  *(authored: scaffold/contracts/cc-escalation.md)*
- **mesh.queues(triggers) → rollup** via `rollup-mesh` — a trigger's
  `AssemblyTemplate` resolves declarative `Rollup(RollupRef)` references at assembly
  time; queues calls rollup over mesh. queues is the *consumer*; `rollup` (batch 4)
  authors `rollup-mesh` (+ its own registration). Consumer-side note below.
  *(authored: scaffold/contracts/rollup-mesh.md)*
- **keeper (batch 6, L6) → mesh.queues** via `queues-api` — the keeper runtime is a
  **consumer**: keeper-to-keeper proposals/approvals ride a durable per-keeper inbox
  queue, exactly-once via the event-ID semaphore, so approvals survive compaction
  (concern 12). Contract **seam only**; the approval protocol is PARKED (OQ-6).
  keeper.md (batch 6) authors the consumer side; no new wire contract here.
- **`types`** — library dependency, NOT a contract edge: `Event`, `EventType`,
  `Provenance` (existing) + the new `types::trigger` module (`Trigger`, `FilterExpr`,
  `AssemblyTemplate`, `SemaphoreChoice`, `HandlerRef`, …) whose shape this design
  authors, now **extended by the wave-3 fold** with `TriggerSource`, `ScheduleSource`,
  `Schedule`, `FireTarget`, `MisfirePolicy` (absorbed from the tombstoned `cron`
  lib's own vocabulary into `types::trigger`) and the `HandlerRef::Emit` variant.
  Also consumes
  **`types::delivery::DeliveryPersistence`** (concern 11) — but queues' durability is
  native, so the flag is near-vacuous on the queue side. All proposed to the batch-1
  `types` designer; the fold additions are flagged for the harmonizer to land in
  `types::trigger`.
- **`locks`** (Ring-3 sibling) — the event-ID semaphore composition (concern 4;
  the queue-owner-lease alternative died with the fork — INTENT #112) **and** the
  absorbed schedule evaluator's occurrence-keyed single-fire semaphore (concern 10).
  Consumed in-process via `locks`' `trait` seam; `locks` owns `locks-api` and the
  partition-merge error type. Co-batched (batch 2, `queues ⇄ locks`) — I depend on
  that error type and the acquire/release surface; flagged for mid-batch draft-sharing.
- **`replicated-kv`** (Ring-2) — durable queue/message/delivery/trigger metadata via
  namespaced LWW keyspaces, **plus** the absorbed schedule store (trigger rows) and
  the `queues/schedule-state/<trigger_id>` fire cursor (concern 10). Consumed via the
  `KvHandle` seam; not a contract edge.
- **`service-registry`** (Ring-3 sibling) — consumed in-process by the absorbed
  schedule evaluator for **self identity** (`FireTarget::Node(N)` fires only when
  self == N) and the **peer set** (validating a `Node(N)` target). Not a contract
  edge. (New with the wave-3 cron fold; was a cron dependency, now internal to queues.)
- **`pubsub-relay`** (Ring-1) — the opt-in `queue.*` observability tee (concern 9),
  a Ring-3 → Ring-1 consume (allowed). Distinct from the **PARKED reverse** relationship
  (concern 11 / OQ-3): backing pub/sub's `IntermediateCache` with queues would be
  Ring-1 → Ring-3, a layering inversion — **NOT adopted**, seam kept off the contract
  graph. In-process; not a contract edge either way.
- **`execution-engine`** (batch 4, L4) — NOT a queues contract edge; the binding is
  the **shared `types::trigger` data model** (concern 2), whose `TriggerSource` now
  also carries `Schedule`. execution-engine still uses only `TriggerSource::Queue`
  bindings (its adapters); flagged for batch-4 co-design: it must consume the folded
  model unchanged.

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
choices, DLQ-is-just-a-queue with cc escalation as a declarative DLQ trigger, and
the pub/sub tee are all decided and specified. **Wave-3 fold (concerns 10–12) is
likewise implementation-ready:** the `TriggerSource::Schedule` absorption of cron
(FireTarget flavors + occurrence-keyed single-fire + misfire + `Emit` decomposition,
all carried over intact), the native-durability treatment of `types::delivery` with
the honest OQ-3 explanation left un-decided, and the keeper-inbox durability seam
(protocol PARKED) are all specified to fill. The three proposed contract *wire
shapes* (`queues-api`, `cc-escalation` DLQ half, `rollup-mesh` consumer view) are
**approach-sketched** — fields authored, reconciled in the per-pair round
with `locks` (error type), `rollup` (assembly-resolve API), `execution-engine`
(shared trigger model), and `cc` (escalation receiver). The queue-ownership fork
is **LOCKED** (replicated-everywhere + semaphore, INTENT #112 — concern 4); FIFO
remains the one genuine fork left for the operator — see Friction.

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
  - The queue-ownership fork the contract flagged is now **LOCKED** (friction-
    round 1, INTENT #112): replicated-everywhere + event-ID semaphore — the
    model the schema assumed is confirmed; single-owner-node-with-failover is
    rejected. See concern 4 for the operator's verbatim rationale.
  - **WAVE-3 FOLD (batch 3):** `queues-api` now **absorbs `cron-api`** (tombstoned)
    — the `Schedule` trigger source, `FireTarget`/`Schedule`/`MisfirePolicy`
    vocabulary, schedule-trigger management (register / enable-disable / run-now),
    and the deterministic-`event_id` emit. See `queues-api.md` § "Proposed
    contracts (wave 3)". No change to the LOCKED declarative-trigger vocabulary.
- `cc-escalation` (producers mesh.queues + execution-engine → consumer cc;
  one union `EscalationRequest` shape) — queues authors the `DeadLetter` arm,
  fired by an ordinary DLQ trigger. → `scaffold/contracts/cc-escalation.md`
  - Contract resolution: execution-engine's enriched `LoopDepthExceeded` arm
    won over the thin placeholder once sketched here (kept only as the
    back-compat deserialize floor); `EscalationAck::Investigating` carries
    cc's `ThreadId`, not a bare `String`.
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
  cc-escalation trigger exactly once (deduped by `escalation_id`); the DLQ itself
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

**Wave-3 fold conformance (cron absorbed — concern 10):**
- **Run-anywhere single-fire across nodes:** two daemons both evaluate an `Anywhere`
  `Schedule` trigger due at the same nominal time; the `locks` occurrence semaphore
  (keyed by `uuid_v5(SCHEDULE_NS, trigger_id ++ scheduled_for)`) admits **exactly
  one** emit; the loser drops silently. The emitted event's `event_id` equals the
  `occurrence_id`.
- **Run-on-node pinning:** a `Node(N)` `Schedule` trigger is fired only by N's
  evaluator; other daemons hold the definition but never fire it; while N is offline
  the occurrence defers per `MisfirePolicy`, not an error.
- **Misfire fire-on-wake single-fire:** a fleet asleep across k occurrences of an
  `Anywhere` `FireOnWake { coalesce: true }` trigger fires **one** catch-up on wake
  (`catch_up: true`), single-fire via the missed-occurrence semaphore; `Skip` fires
  none of the missed, only the next upcoming.
- **Fire cursor never races a definition edit:** advancing `queues/schedule-state/
  <trigger_id>` on fire and concurrently `UpdateTrigger`-ing the same trigger's
  schedule converge independently (separate keyspaces, both LWW) — no lost edit, no
  lost fire.
- **pg_cron decomposition end-to-end:** a `Schedule` trigger with `HandlerRef::Emit`
  publishes `schedule.fired` into a target queue; a downstream `Queue`-source trigger
  filters that `event_type` and dispatches a `vdb` handler — the tombstoned cron
  example runs unchanged (see `queues-api.md` example data).
- **Deterministic-id stability across versions:** the `occurrence_id` recipe yields
  byte-identical uuids on two different builds for the same `(trigger_id,
  scheduled_for)` — the cross-node single-fire + consume-side dedup anchor (a
  breaking change if it ever differs).
- **Unknown-schedule fail-safe:** a daemon deserializing a `Schedule::Unknown` /
  `MisfirePolicy::Unknown` (newer kind) **never fires it**; an `Anywhere` such job is
  transparently fired by a capable node; a `Node(N)`-pinned such job on an incapable
  N surfaces the flagged "silently never fires" sharp edge (gate new kinds on fleet
  capability).

**Wave-3 delivery-persistence (concern 11):**
- **`SaveFailed` is native on the queue side:** a queue `SendEvent` marked
  `DeliveryPersistence::SaveFailed` behaves identically to today (already durable);
  `LossyDrop` on a queue send is rejected/ignored as meaningless (queues never
  silently drops) — the flag has no queue-side branch, proving the asymmetry that
  keeps OQ-3 a live, un-merged question.
