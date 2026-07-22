# Contract: queues-api

## Parties

- **any service** (via `mesh-client`) — enqueues events, registers triggers, runs handlers
- **mesh** (`mesh.queues`, the L2 internal library) — durable queues + declarative dispatch

Cross-cutting, surface-schema-style: ONE shared document, every service is a
party. Struct vocabulary is homed in `types` (`event.rs`, `provenance.rs`, and a
new `trigger.rs`); the queue/dispatch operations are `queues`'.

## Purpose

The one WS protocol a service speaks to its LOCAL mesh daemon on `:3649` to
(1) manage queues, (2) publish typed events, (3) register/update/deregister
**declarative triggers** (INTENT #103), and (4) receive handler deliveries and
ack/nack them. **Durable, at-least-once by contract** — the guaranteed
counterpart to `pubsub-protocol`'s lossy contract. Vocabulary LOCKED at INTENT
#101: events are typed and go into queues; triggers are 1:1 queue→handler
(many per queue), they FILTER and ASSEMBLE the handler payload ("it's the
trigger that dictates the payload, not the event"); handlers hold the code.

## Schema

### The declarative trigger (`types::trigger` — shape authored by queues)

```rust
pub struct Trigger {
    pub trigger_id: TriggerId,
    pub source: TriggerSource,        // WAVE-3 FOLD: Queue(event-driven) | Schedule(time-driven, absorbed cron)
    pub handler: HandlerRef,          // the 1:1 dispatch target (incl. Emit)
    pub filter: FilterExpr,           // declarative — filters the subject document
    pub assembly: AssemblyTemplate,   // declarative — builds the handler/emitted payload
    pub semaphore: SemaphoreChoice,   // per-trigger event-ID-semaphore choice (INTENT #95/#101)
    pub idempotency: IdempotencyMode, // handler-side contract hint
    pub redrive: Option<Redrive>,     // per-trigger override of the queue default
    #[serde(default = "default_true")]
    pub enabled: bool,                // WAVE-3 FOLD: pause/resume (absorbs CronJob.enabled)
    pub registered_by: Slug,
    pub version: LwwVersion,          // (wall_clock, node_id) — LWW like registry entries
}

// WAVE-3 FOLD — a trigger's source is now an enum (the only model change the cron
// absorption makes). See "Proposed contracts (wave 3)" below for ScheduleSource /
// Schedule / FireTarget / MisfirePolicy and HandlerRef::Emit.
pub enum TriggerSource {
    Queue(QueueName),                 // event-driven: fires per matching event (today's behavior)
    Schedule(ScheduleSource),         // time-driven: fires per scheduled occurrence (absorbed cron)
}

pub enum FilterExpr {
    Always,
    EventType(EventTypeMatch),        // Exact | Prefix over "domain.noun.verb"
    Field { path: JsonPath, op: CmpOp, value: serde_json::Value },
    And(Vec<FilterExpr>), Or(Vec<FilterExpr>), Not(Box<FilterExpr>),
}
pub enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge, In, Contains, Exists, Matches }

pub struct AssemblyTemplate { pub root: TemplateNode }
pub enum TemplateNode {
    Literal(serde_json::Value),
    SubjectPath(JsonPath),            // copy from the event/subject document
    Meta(MetaField),                  // event_id | event_type | occurred_at | provenance | correlation_id
    Rollup(RollupRef),               // declarative rollup reference (INTENT #94: Raw | Reference)
    Object(BTreeMap<String, TemplateNode>),
    Array(Vec<TemplateNode>),
}

pub enum HandlerRef {
    Service { slug: Slug, route: String },     // push-deliver over mesh transport, await ack
    Topic(Topic),                              // deliver as a pubsub Envelope (fire-and-forget)
    ExecFn { engine: Slug, function: String }, // execution-engine Deno/SQL handler
    Emit { queue: QueueName, event_type: EventType }, // WAVE-3 FOLD: emit assembled payload as an
                                               // event into a queue (the cron emission; also a router action)
}
pub enum SemaphoreChoice { None, EventId, Custom(SemaphoreKeyTemplate) }
pub enum IdempotencyMode { HandlerIdempotent, DedupByEventId, AtMostOnceBestEffort }
```

A `FilterExpr` evaluates over — and an `AssemblyTemplate` reads from — a generic
JSON **subject document** plus well-known meta fields. Each source/engine binds its
subject in: a `Queue`-source trigger binds an `Event` → `{ "type": <event_type>,
"payload": <P>, "meta": { event_id, occurred_at, provenance } }`; a `Schedule`-source
trigger (WAVE-3 fold) binds the fired occurrence → `{ "type": "schedule.fired",
"payload": <static payload>, "meta": { occurrence_id, scheduled_for, fired_at,
fire_node, catch_up } }`; `execution-engine` (batch 4) binds a row/node change →
`{ "table"|"node_type", "op", "old", "new", "meta" }`. The AST + template are
identical — **one trigger data model, three subject bindings.**

### Queue management + lifecycle

```rust
struct QueueConfig {
    visibility_timeout: Duration,     // per-delivery in-flight window
    max_receive_count: u32,           // redrive threshold
    dlq: Option<QueueName>,           // "a DLQ is just a queue"
    retention: Duration,              // message TTL before tombstone-GC
    tee_topic: Option<Topic>,         // opt-in pubsub observability tee
}
```

### Client → daemon (`QueuesClientMsg`)

```rust
enum QueuesClientMsg {
    EnsureQueue { name: QueueName, config: QueueConfig },   // idempotent
    DeleteQueue { name: QueueName },
    SendEvent  { queue: QueueName, event: Event },          // returns event_id; idempotent on event_id
    RegisterTrigger   { trigger: Trigger },                 // static-validated on receipt (Queue OR Schedule source)
    UpdateTrigger     { trigger: Trigger },                 // LWW by trigger.version
    DeregisterTrigger { trigger_id: TriggerId },
    SetTriggerEnabled { trigger_id: TriggerId, enabled: bool }, // WAVE-3 FOLD: absorbs CronRequest::Enable
    RunTriggerNow     { trigger_id: TriggerId },            // WAVE-3 FOLD: absorbs CronRequest::RunNow — fires a
                                                            // Schedule trigger off-schedule; still single-fire (Anywhere)
    ListTriggers      { source_kind: Option<SourceKind>, owner: Option<Slug> }, // filter by Queue|Schedule (absorbs CronRequest::List)
    // handler-delivery (SQS-style ack; push-dispatch is primary, pull kept for parity)
    AckDelivery  { delivery_id: DeliveryId },               // success -> delete
    NackDelivery { delivery_id: DeliveryId, retry_after: Option<Duration> },
    ExtendVisibility { delivery_id: DeliveryId, by: Duration },
    ReceiveDeliveries { queue: QueueName, max: u32, wait: Option<Duration> },
}
```

### Daemon → service (`QueuesServerMsg`)

```rust
enum QueuesServerMsg {
    EventAccepted { event_id: Uuid },
    TriggerRegistered { trigger_id: TriggerId },
    Deliver {                                   // push-dispatch of an ASSEMBLED payload
        delivery_id: DeliveryId,
        trigger_id: TriggerId,
        event_id: Uuid,
        correlation_id: Option<Uuid>,           // from Provenance — for handler-side dedup
        payload: serde_json::Value,             // trigger-assembled, NOT the raw event
        visibility_deadline: DateTime<Utc>,
    },
    DeliveryOutcome { delivery_id: DeliveryId, outcome: DeliveryOutcome },
    Error { code: QueuesError, detail: String },
}
enum DeliveryOutcome { Acked, DeadLettered { dlq: QueueName }, PartitionConflict }
```

## Error cases

`QueuesError`:
- `QueueNotFound` / `QueueAlreadyExists` — strict create; `EnsureQueue` is idempotent.
- `TriggerNotFound`.
- `InvalidFilterExpr` / `InvalidAssemblyTemplate` — **static validation at
  registration** (possible because triggers are declarative data, INTENT #103),
  never at dispatch.
- `InvalidEventType` — not a `domain.noun.verb` identifier.
- `AssemblyFailed` — a `Rollup(RollupRef)` reference or `SubjectPath` failed to
  resolve during assembly → the delivery fails (returns/redrives), never panics.
  Also the `llm_safe` degrade path: a `secrets` raw reference must NOT resolve
  into LLM-bound assembled content (INTENT #94) → treated as `AssemblyFailed`,
  never a leak.
- `SemaphoreUnavailable` — transient failure acquiring the event-ID semaphore
  (retry); distinct from `PartitionMerge`.
- `PartitionMerge` — bubbles `locks`' `LockError::PartitionMergeExceeded`
  (INTENT #84) as a first-class catchable outcome, surfaced as
  `DeliveryOutcome::PartitionConflict`, never silently swallowed.
- `MessageNotInFlight` — ack/extend for an expired-or-reassigned delivery: its
  visibility timeout lapsed and another node took it. The ack is rejected and the
  handler's write may have doubled — **this is exactly why idempotency is
  required** (a handler that is not idempotent is a bug against this contract),
  not an error queues can prevent.

Non-errors by design: `SendEvent` to a queue with no matching triggers succeeds
(retained until retention TTL, then GC-tombstoned); re-sending the same
`event_id` is idempotent.

## Version sensitivity

**HIGH** — events and trigger definitions cross nodes and **persist** in
`replicated-kv` across a mixed-version fleet (INTENT #66).
- **LOCKED (friction-round 1, INTENT #113) — sender version stamping.** Every
  event crossing the mesh carries the sending service's **name AND version**
  (rides `Provenance` — `service` + the new `service_version`, see
  `types::provenance` / `pubsub-protocol`). A receiver (handler or daemon) MAY
  enforce a **version floor** ("only accepting messages from nodes with service
  version greater than X"); a floored message is rejected with a **catchable**
  error (`VersionBelowFloor`-shaped, delivered as a delivery outcome / nack,
  never a silent drop). On a schema change, offer **backwards compatibility for
  one version**, and attach a **please-update warning back to the sender**.
- **Additive-safe:** any new `EventType` string (an open identifier, NEVER a
  closed enum — mirrors pubsub-relay's payload-opaque property; a new type flows
  through an older daemon untouched); new `#[serde(default)]` fields; new
  `#[serde(other)]`-tolerant enum variants on `FilterExpr` / `TemplateNode` /
  `CmpOp` / `HandlerRef`.
- An older daemon receiving a `Trigger` with an unknown filter/assembly variant
  must **fail that trigger's registration/dispatch loudly and locally**, never
  crash the queue or drop the sibling trigger set.
- **Stable subset:** the SQS-lifecycle fields (`visibility_timeout`,
  `max_receive_count`, `retention`) are frozen for the someday-`SqsBackend`
  drop-in (INTENT #89/#106).
- `Trigger.version` is LWW `(wall_clock, node_id)` — concurrent edits converge
  like registry entries (INTENT #32).
- **Breaking:** changing the deterministic subject-document binding, or the
  meaning of `SemaphoreChoice::EventId` (the cross-node exactly-once key).
- **WAVE-3 FOLD additions (additive-safe):** `Trigger.source` replaces the wave-2
  `Trigger.queue` field with `TriggerSource::Queue(q)` (identical old behavior) |
  `TriggerSource::Schedule(..)`; `HandlerRef::Emit`; `Trigger.enabled`
  (`#[serde(default)]`). `Schedule`/`FireTarget`/`MisfirePolicy` each reserve
  `#[serde(other)] Unknown` and are **fail-safe** (an unknown variant never fires).
  This is a pre-production reshape (INTENT #138 — no production use yet), so the
  `queue → source` migration is mechanical, not a compatibility break.
- **Breaking (fold):** the schedule `event_id` recipe `uuid_v5(SCHEDULE_NS,
  trigger_id ++ scheduled_for.rfc3339())` — the cross-node single-fire + consume-side
  dedup anchor; must be identical on every node and version (carried from cron-api).

## Reconciliation notes

- **Trigger-struct ownership (the flagged types-vs-queues question): RESOLVED —
  concordant, no dispute.** Both parties independently proposed that the `Trigger`
  struct (filter expression + assembly template) lives in **`types`**, in its own
  `trigger.rs` module (never folded into `event.rs`), with its **shape authored by
  `queues`**. `types` (its `queues-api` PARTIAL proposal) reasoned it satisfies
  the inclusion test — pure declarative data used by ≥2 crates (`queues` +
  `execution-engine`) and part of the `queues-api` schema — but deferred the shape
  to queues' domain. `queues` authored the shape (`types::trigger`, "module lives
  in `types`"). This contract adopts both: **home = `types::trigger`, shape =
  queues' proposal above.** The one-model-two-bindings requirement (queues +
  execution-engine share the identical `FilterExpr`/`AssemblyTemplate`) is what
  forces the shared `types` home — it is exactly the ≥2-crate case.
- **`cc-escalation` and `rollup-mesh` are NOT authored here** — queues proposed
  only its half of each (the DLQ escalation arm and the assembly-time rollup
  consumer flag). Those are separate contract pairs (`cc-escalation`,
  `rollup-mesh`) outside this cluster; recorded as cross-references only.
- **Queue-ownership fork — LOCKED (friction-round 1, 2026-07-19, INTENT #112).**
  The provisional assumption this schema was written against is now confirmed:
  **replicated-everywhere + event-ID semaphore; all nodes process; whoever
  discovers an event may try to claim ownership via the semaphore** (which, on
  acquisition, "creates a UTC timestamp down to the nanosecond — whoever gets
  it"). *Single-owner-node per queue with failover* is rejected — operator,
  verbatim: "I don't think having one processor makes sense. All processors —
  whoever discovers it can try to claim ownership." A few seconds of
  semaphore-acquisition latency is explicitly acceptable; the operator's
  reliability-over-speed rationale, verbatim: "What we're going for is
  reliability, not speed. The way we maximize our leverage isn't by getting from
  three seconds down to a tenth of a second — it's by keeping things running all
  the time and having our leverage create more leverage." His partition insight:
  queue-pull semaphore collisions between disjoint partitions aren't a real
  worry — if the event reached both nodes, those nodes were connected. No schema
  change required; the contract as written IS the locked model.
- **`cron-api` absorption (WAVE-3 FOLD, ledger D2; INTENT #56/#91/F6b): RESOLVED —
  the two `FireTarget` flavors and the single-fire-via-event-ID-semaphore semantics
  carry over intact** as a `TriggerSource::Schedule`. `cron-api.md` and
  `components/cron.md` are tombstoned into this contract / `queues.md` concern 10;
  the vocabulary map is one-for-one (see `cron-api.md`). Ring simplification: the
  Ring-4 cron evaluator collapses into this Ring-3 lib with no new dependency (it
  already rode `locks`/`kv`/`registry`). The full schema is in "Proposed contracts
  (wave 3)" below.

## Example data

A nightly rollup fires on the shared example world — the full pg_cron leg, now
**two triggers in one lib**: (A) a `Schedule`-source trigger emits
`stack.analytics.nightly-rollup.tick` into queue `demo.analytics.jobs`; (B) a
`Queue`-source trigger filters it and assembles a handler payload for `vdb`. The
firing node (here **pi**) wins the occurrence semaphore.

**(A) The schedule trigger** (absorbs the tombstoned `cron-api` job — the emitter):
```jsonc
{ "trigger_id": "vdb/analytics/nightly-rollup",
  "source": { "type": "Schedule", "schedule": { "Cron": { "expr": "0 3 * * *", "tz": "UTC" } },
              "target": "Anywhere",
              "misfire": { "FireOnWake": { "grace": "PT6H", "coalesce": true } } },
  "handler": { "type": "Emit", "queue": "demo.analytics.jobs",
               "event_type": "stack.analytics.nightly-rollup.tick" },
  "filter": "Always",
  "assembly": { "root": { "type": "Object", "fields": {
      "db":   { "type": "Literal", "value": "analytics" },
      "task": { "type": "Literal", "value": "nightly_rollup" } } } },
  "semaphore": "EventId",
  "idempotency": "DedupByEventId",
  "enabled": true,
  "registered_by": "vdb",
  "version": { "wall_clock": 1721448000000, "node_id": "macbook" } }
// On fire, occurrence_id = event_id = uuid_v5(SCHEDULE_NS,
//   "vdb/analytics/nightly-rollup" ++ "2026-07-19T03:00:00Z"). The Anywhere
// occurrence semaphore admits exactly one emit; pi wins and publishes the event.
```

**(B) The consuming trigger** (event-driven — unchanged from wave 2, `queue` field
now expressed as `source: Queue`):
```jsonc
{ "trigger_id": "trg-77a1",
  "source": { "type": "Queue", "queue": "demo.analytics.jobs" },
  "handler": { "type": "Service", "slug": "vdb", "route": "/run-rollup" },
  "filter": { "type": "EventType", "match": { "Exact": "stack.analytics.nightly-rollup.tick" } },
  "assembly": { "root": { "type": "Object", "fields": {
      "db":   { "type": "SubjectPath", "path": "payload.db" },
      "task": { "type": "Literal", "value": "nightly_rollup" },
      "when": { "type": "Meta", "field": "occurred_at" } } } },
  "semaphore": "EventId",
  "idempotency": "DedupByEventId",
  "redrive": { "dlq": "demo.analytics.jobs.dlq", "max_receive_count": 5 },
  "enabled": true,
  "registered_by": "vdb",
  "version": { "wall_clock": 1721448000000, "node_id": "macbook" } }
```

The delivery pushed to `vdb` (on whichever node won the event-ID semaphore, here
**pi**):
```jsonc
{ "type": "Deliver",
  "delivery_id": "dlv-3310",
  "trigger_id": "trg-77a1",
  "event_id": "d5c2f4a1-...(v5)...",
  "correlation_id": null,
  "payload": { "db": "analytics", "task": "nightly_rollup",
               "when": "2026-07-19T03:00:01Z" },
  "visibility_deadline": "2026-07-19T03:00:31Z" }
```

`vdb` acks after running the SQL:
```jsonc
// -> { "type": "AckDelivery", "delivery_id": "dlv-3310" }
// <- { "type": "DeliveryOutcome", "delivery_id": "dlv-3310", "outcome": "Acked" }
```

Partition-conflict path: if **macbook** and **pi** partitioned and each dispatched
this event under a UUID-twin semaphore, on merge `locks` raises
`PartitionMergeExceeded` and the outcome surfaces as
`{ "outcome": "PartitionConflict" }` — not a silent double-dead-letter.

---

## Proposed contracts (wave 3)

This unit owns the **trigger-shape change** the cron fold requires, so it is
proposed here (the shape is homed in `types::trigger`; `types` houses it —
flagged for the harmonizer to land the additions). Three folds land, none
altering the LOCKED declarative-trigger / per-trigger-semaphore vocabulary
(#101/#103).

### 1. `cron-api` ABSORBED — the `Schedule` trigger source (ledger D2; INTENT #56/#91/F6b)

`contracts/cron-api.md` is **tombstoned into this contract**. A trigger's source
becomes an enum; the cron vocabulary moves into `types::trigger` intact:

```rust
pub struct ScheduleSource {            // moved from the tombstoned cron lib into types::trigger
    pub schedule: Schedule,            // Cron{expr,tz} | Every{interval,anchor} | Once{at}
    pub target: FireTarget,            // Anywhere (single-fire, raced) | Node(NodeId) (pinned)
    pub misfire: MisfirePolicy,        // Skip | FireOnWake { grace, coalesce }
}
pub enum FireTarget { Anywhere, Node(NodeId), #[serde(other)] Unknown }
pub enum Schedule {
    Cron  { expr: String, #[serde(default)] tz: Option<String> },   // 5/6-field
    Every { interval: Duration, #[serde(default)] anchor: Option<DateTime<Utc>> },
    Once  { at: DateTime<Utc> },       // one-shot; self-disables after fire
    #[serde(other)] Unknown,           // FAIL-SAFE: an older node never fires an unknown kind
}
pub enum MisfirePolicy {
    Skip,                              // default: abandon occurrences missed while unavailable
    FireOnWake { #[serde(default)] grace: Option<Duration>,
                 #[serde(default = "default_true")] coalesce: bool },
    #[serde(other)] Unknown,
}
pub enum SourceKind { Queue, Schedule }   // ListTriggers filter discriminant
```

**The emitted occurrence event** (a `Schedule` trigger with `HandlerRef::Emit`
publishes it into `emit.queue`; `Event`/`Provenance` from `types`):

```rust
// event_id is DETERMINISTIC — the cross-node single-fire key AND the consume-side
// dedup key:  event_id = occurrence_id = uuid_v5(SCHEDULE_NS, trigger_id ++ scheduled_for.rfc3339())
struct ScheduleFired {
    trigger_id: TriggerId,
    scheduled_for: DateTime<Utc>,      // nominal fire time (deterministic key)
    fired_at: DateTime<Utc>,           // actual wall-clock of firing
    fire_node: NodeId,                 // node that won the occurrence semaphore
    catch_up: bool,                    // true for a FireOnWake catch-up fire
    payload: serde_json::Value,        // the ScheduleSource static payload, passed through
}
```

**Management folds into the existing client messages** (no separate `CronRequest`):
`RegisterTrigger`/`UpdateTrigger` carry a `Schedule`-source `Trigger`;
`SetTriggerEnabled` = cron `Enable`; `RunTriggerNow` = cron `RunNow` (fires an
off-schedule occurrence, still single-fire); `ListTriggers { source_kind:
Some(Schedule) }` = cron `List`. Runtime fire state lives server-side in
`queues/schedule-state/<trigger_id>` (LWW cursor), not on the trigger row (so a
fire never LWW-races a definition edit — a refinement over cron's in-row
writeback). See `cron-api.md` for the full one-for-one vocabulary map.

**Errors fold into `QueuesError`:** `InvalidSchedule { detail }` (unparseable
expr / zero interval / `Once` in the past); `UnknownTargetQueue` and `UnknownNode`
lean **soft/accept** (register a schedule before its queue exists; `Node(N)` may be
a currently-offline walk-along Pi — #84); `NotOwner`; `VersionConflict`.

**Version sensitivity (carried from cron-api):** the deterministic `event_id`
recipe is a **breaking, fleet-coordinated** contract detail — it MUST be computed
identically on every node and version. `Schedule`/`FireTarget`/`MisfirePolicy` each
reserve `#[serde(other)] Unknown` and a daemon **must not fire** an `Unknown`
variant. Sharp edge (flagged, carried over): a `Node(N)`-pinned schedule using a
kind N cannot parse would silently *never* fire — gate new schedule kinds on
fleet-wide capability, or pin such jobs only to capable nodes.

### 2. Delivery-persistence is NATIVE on the queue side (INTENT #155; PARKED OQ-3)

`SendEvent` events carry `types::delivery::DeliveryPersistence`, but a queue is
**already durable/at-least-once by contract**, so `SaveFailed` *is* queues' native
behavior and `LossyDrop` has no queue meaning (queues never silently drops). The
field is therefore near-vacuous on this contract — no queue-side branch. It is
meaningful only for `pubsub-protocol`. Whether pub/sub's `SaveFailed`
`IntermediateCache` is **backed by queues** (the F5/SB7a consolidation) is **PARKED
OQ-3, MARKED NEEDS-EXPLANATION** — see `queues.md` concern 11 for the one-page
honest explanation (the ring-layering inversion pubsub Ring-1 → queues Ring-3, the
API/semantic mismatch, and the boring shared-`replicated-kv`-substrate reframe).
**Not decided here**; the seam stays off the contract graph, un-mergeable with a
one-line change.

### 3. Keeper-inbox durability — contract seam only (INTENT #149/#88/#127; PARKED OQ-6)

The batch-6 keeper runtime is a **consumer** of this contract: keeper-to-keeper
proposals/approvals ride a durable per-keeper inbox queue (boring provisional
`keeper.<keeper_id>.inbox`), at-least-once with `SemaphoreChoice::EventId` for
exactly-once approval semantics, so approvals survive compaction ("approvals must
not vanish"). This contract provides only the **seam** — an ordinary queue + the
exactly-once guarantee; the message payload types and the propose→deliberate→approve
**protocol** are the keeper design's (PARKED OQ-6, "needs its own conversation"),
authored in `keeper.md` / a keeper payload module and carried by queues opaquely.
No new wire contract; recorded as a consumer relationship.
