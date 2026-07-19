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
    pub queue: QueueName,             // source queue (queues) — or a table/graph binding for exec-engine
    pub handler: HandlerRef,          // the 1:1 dispatch target
    pub filter: FilterExpr,           // declarative — filters the subject document
    pub assembly: AssemblyTemplate,   // declarative — builds the handler payload
    pub semaphore: SemaphoreChoice,   // per-trigger event-ID-semaphore choice (INTENT #95/#101)
    pub idempotency: IdempotencyMode, // handler-side contract hint
    pub redrive: Option<Redrive>,     // per-trigger override of the queue default
    pub registered_by: Slug,
    pub version: LwwVersion,          // (wall_clock, node_id) — LWW like registry entries
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
}
pub enum SemaphoreChoice { None, EventId, Custom(SemaphoreKeyTemplate) }
pub enum IdempotencyMode { HandlerIdempotent, DedupByEventId, AtMostOnceBestEffort }
```

A `FilterExpr` evaluates over — and an `AssemblyTemplate` reads from — a generic
JSON **subject document** plus well-known meta fields. Each engine binds its
subject in: `queues` binds an `Event` → `{ "type": <event_type>, "payload": <P>,
"meta": { event_id, occurred_at, provenance } }`; `execution-engine` (batch 4)
binds a row/node change → `{ "table"|"node_type", "op", "old", "new", "meta" }`.
The AST + template are identical — **one trigger data model, two subject bindings.**

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
    RegisterTrigger   { trigger: Trigger },                 // static-validated on receipt
    UpdateTrigger     { trigger: Trigger },                 // LWW by trigger.version
    DeregisterTrigger { trigger_id: TriggerId },
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
- **`ccd-escalation` and `rollup-mesh` are NOT authored here** — queues proposed
  only its half of each (the DLQ escalation arm and the assembly-time rollup
  consumer flag). Those are separate contract pairs (`ccd-escalation`,
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

## Example data

A nightly rollup fires on the shared example world. `cron` on node **pi** emits
`stack.analytics.nightly-rollup.tick` into queue `demo.analytics.jobs`; a trigger
filters it and assembles a handler payload for the `vdb` service.

The registered trigger:
```jsonc
{ "trigger_id": "trg-77a1",
  "queue": "demo.analytics.jobs",
  "handler": { "type": "Service", "slug": "vdb", "route": "/run-rollup" },
  "filter": { "type": "EventType", "match": { "Exact": "stack.analytics.nightly-rollup.tick" } },
  "assembly": { "root": { "type": "Object", "fields": {
      "db":   { "type": "SubjectPath", "path": "payload.db" },
      "task": { "type": "Literal", "value": "nightly_rollup" },
      "when": { "type": "Meta", "field": "occurred_at" } } } },
  "semaphore": "EventId",
  "idempotency": "DedupByEventId",
  "redrive": { "dlq": "demo.analytics.jobs.dlq", "max_receive_count": 5 },
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
