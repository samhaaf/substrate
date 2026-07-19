# Contract: cron-api

## Parties

- **any service** (via `mesh-client`) — schedules jobs
- **mesh** (`mesh.cron`, the L2 internal library) — schedule authority; fires into `queues`

Cross-cutting, surface-schema-style: ONE shared document, every service is a
party. `Event` / `Provenance` are homed in `types`; the `CronJob` / `Schedule`
vocabulary is `cron`'s. `types` proposed no independent shape for this pair, so
it is authored from `cron`'s proposal plus `types`' wire discipline.

## Purpose

The WS protocol a service speaks to its LOCAL mesh daemon on `:3649` to create,
update, delete, enable/disable, inspect, and manually run scheduled jobs of two
flavors: **run-anywhere** (single-fire, any node wins) and **run-on-node**
(pinned). Registration is the *only* thing a service does with cron; everything
after a fire is a `queues-api` + handler concern. Job definitions persist in
`replicated-kv` and converge fleet-wide, so a job created from **macbook** fires
on whatever node its `target` names.

## Schema

### Job definition (`CronJob` — persists as a `replicated-kv` row → wire-crossing discipline)

```rust
struct CronJob {
    job_id: JobId,                 // uuid; or caller "<owner>/<name>" for idempotent upsert
    owner: String,                 // registering service slug (provenance + soft authz)
    schedule: Schedule,
    target: FireTarget,            // Anywhere (single-fire) | Node(NodeId) (pinned)
    emit: EmitSpec,                // what event to publish + which queue
    misfire: MisfirePolicy,
    #[serde(default = "default_true")] enabled: bool,
    version: LwwVersion,           // (wall_clock, node_id) — from replicated-kv
    #[serde(default)] last_fired: Option<FireRecord>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

enum Schedule {
    Cron  { expr: String, #[serde(default)] tz: Option<String> },   // 5/6-field
    Every { interval: Duration, #[serde(default)] anchor: Option<DateTime<Utc>> },
    Once  { at: DateTime<Utc> },   // one-shot; auto-disables after fire
    #[serde(other)] Unknown,       // older node tolerates a newer kind (fail-safe: never fires it)
}
enum FireTarget { Anywhere, Node(NodeId), #[serde(other)] Unknown }

struct EmitSpec {
    target_queue: String,          // queue the cron.fired Event lands in
    #[serde(default)] event_type: Option<EventType>,   // default "cron.fired"
    #[serde(default)] payload: serde_json::Value,      // static declarative passthrough
}
enum MisfirePolicy {
    Skip,
    FireOnWake { #[serde(default)] grace: Option<Duration>,
                 #[serde(default = "default_true")] coalesce: bool },
    #[serde(other)] Unknown,
}
struct FireRecord { scheduled_for: DateTime<Utc>, fired_at: DateTime<Utc>, fire_node: NodeId }
```

### The emitted event (published into `emit.target_queue`; `Event`/`Provenance` from `types`)

```rust
// types::event::Event<CronFired> with a DETERMINISTIC event_id:
//   event_id = uuid_v5(CRON_NS, job_id ++ scheduled_for.to_rfc3339())
struct CronFired {
    job_id: JobId,
    scheduled_for: DateTime<Utc>,  // nominal fire time (deterministic single-fire key)
    fired_at: DateTime<Utc>,       // actual wall-clock of firing
    fire_node: NodeId,             // node that won the semaphore
    catch_up: bool,                // true for a FireOnWake catch-up fire
    payload: serde_json::Value,    // EmitSpec.payload, passed through untouched
}
```

### Client → daemon (`CronRequest`)

```rust
enum CronRequest {
    Upsert  { job: CronJobSpec },   // create-or-replace; spec omits server-managed fields
    Patch   { job_id: JobId, patch: CronJobPatch, expect_version: Option<LwwVersion> },
    Delete  { job_id: JobId },
    Enable  { job_id: JobId, enabled: bool },
    Get     { job_id: JobId },
    List    { owner: Option<String>, target: Option<FireTarget> },
    RunNow  { job_id: JobId },      // fire immediately off-schedule; still single-fire
}
```

### Daemon → client (`CronResponse`)

```rust
enum CronResponse { Job(CronJob), Jobs(Vec<CronJob>), Ok, Error(CronError) }
```

## Error cases

`CronError` (`types` error taxonomy, a `cron.rs` sub-enum):
- `InvalidSchedule { detail }` — un-parseable cron expr / zero interval / `Once`
  in the past.
- `UnknownJob { job_id }` — patch/delete/get/run-now on a missing job.
- `UnknownTargetQueue { queue }` — `emit.target_queue` names no known queue.
  **Soft (lean warn, not reject):** a job may be registered before its queue
  exists, so schedule and queue can be provisioned in any order.
- `UnknownNode { node }` — `Node(N)` names a node not (yet) in the registry.
  **Soft/CAP-honest (lean accept + fire-when-N-appears):** N may be a currently-
  offline walk-along Pi (INTENT #84).
- `NotOwner { job_id }` — a non-owner slug tried to modify another owner's job
  (soft authz; ownership is cheap provenance, INTENT #39).
- `VersionConflict { job_id, current: LwwVersion }` — optimistic `expect_version`
  on `Patch` lost the LWW race.

Non-errors by design: firing while the target node is offline is deferred per
misfire policy, not an error; an `Anywhere` fire where every racing node but one
loses the semaphore is normal (losers drop silently).

## Version sensitivity

**HIGH** — `CronJob` rows cross nodes via `replicated-kv` anti-entropy.
- **Additive-safe:** every added field `#[serde(default)]`, no
  `deny_unknown_fields`; `Schedule` / `FireTarget` / `MisfirePolicy` each reserve
  `#[serde(other)] Unknown`; `emit.event_type` is an open namespaced identifier.
- **Fail-safe evaluation of unknown variants:** a daemon that deserializes a job
  into an `Unknown` `Schedule` / `MisfirePolicy` **must not fire it** — it skips
  and lets a newer-version node handle it. For `Anywhere` jobs this is transparent
  (a capable node fires). **Sharp edge (flagged):** a `Node(N)`-pinned job using a
  schedule kind N's build cannot parse would silently *never* fire — mitigation:
  gate new schedule kinds on fleet-wide capability, or pin such jobs only to
  capable nodes. Couples cron's evolution to the OPEN mixed-version update
  protocol (`supervision`, INTENT #66).
- **Breaking:** the deterministic `event_id` recipe (`uuid_v5(CRON_NS, job_id ++
  scheduled_for)`) is a versioned contract detail — it MUST be computed
  identically on every node and across versions, because it is both the cross-node
  single-fire key and the consume-side dedup key. Changing it would let two
  versions double-fire the same occurrence: a breaking, fleet-coordinated change.

## Reconciliation notes

- **Single-proposer pair.** Only `cron` proposed a shape; `types` contributed the
  `Event`/`Provenance` home and wire discipline. No cross-party disagreement.
  Authored from `cron`'s proposal.
- **run-on-node vs run-anywhere (the pair's headline question): RESOLVED — both,
  as `FireTarget { Anywhere | Node(NodeId) }`.** `Anywhere` is single-fire via the
  event-ID / occurrence semaphore (only one racing node fires); `Node(N)` is
  pinned and fires exactly on N (deferred while N is offline, per misfire policy).
  Neither is dropped — they are the two arms of one enum.
- **Deviations pinned rather than deferred:** (1) the deterministic `event_id`
  recipe is written into the contract, not left to fill, because it is the
  cross-node single-fire correctness anchor; (2) `UnknownTargetQueue` and
  `UnknownNode` are resolved to the *soft/accept* interpretation (the queues-side
  and registry-side reconciliation both lean "provision in any order").
- **Downstream is `queues-api`, not a new edge:** everything after a fire is a
  `queues-api` concern — the emitted `Event` lands in a queue and a trigger
  filters/assembles it. This contract stops at the emit.

## Example data

A run-anywhere nightly rollup on the `demo` project's analytics DB — the pg_cron
leg. Registered once from **macbook**; fires on whatever node wins the semaphore.

The job:
```jsonc
{ "job_id": "vdb/analytics/nightly-rollup",
  "owner": "vdb",
  "schedule": { "Cron": { "expr": "0 3 * * *", "tz": "UTC" } },
  "target": "Anywhere",
  "emit": {
    "target_queue": "demo.analytics.jobs",
    "event_type": "stack.analytics.nightly-rollup.tick",
    "payload": { "db": "analytics", "task": "nightly_rollup" } },
  "misfire": { "FireOnWake": { "grace": "PT6H", "coalesce": true } },
  "enabled": true,
  "version": { "wall_clock": 1721448000000, "node_id": "macbook" },
  "last_fired": { "scheduled_for": "2026-07-19T03:00:00Z",
                  "fired_at": "2026-07-19T03:00:01Z", "fire_node": "pi" },
  "created_at": "2026-07-01T12:00:00Z", "updated_at": "2026-07-01T12:00:00Z" }
```

The event **pi** published into queue `demo.analytics.jobs` at that fire.
`event_id` is deterministic: `uuid_v5(CRON_NS, "vdb/analytics/nightly-rollup" +
"2026-07-19T03:00:00Z")`. A trigger on that queue filters this `event_type` and
assembles the handler payload (see `queues-api`); the `vdb` handler runs the
rollup SQL against `qwen3-4b`-free analytics.
```jsonc
{ "event_id": "d5c2f4a1-...(v5)...",
  "event_type": "stack.analytics.nightly-rollup.tick",
  "occurred_at": "2026-07-19T03:00:01Z",
  "provenance": { "service": "cron", "node_id": "pi",
                  "emitted_at": "2026-07-19T03:00:01Z",
                  "seq": 12, "correlation_id": null, "causation_id": null },
  "payload": {
    "job_id": "vdb/analytics/nightly-rollup",
    "scheduled_for": "2026-07-19T03:00:00Z", "fired_at": "2026-07-19T03:00:01Z",
    "fire_node": "pi", "catch_up": false,
    "payload": { "db": "analytics", "task": "nightly_rollup" } } }
```
