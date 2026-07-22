# Contract: cron-api — FOLDED INTO queues-api (tombstone)

> **⚠ TOMBSTONE — `cron-api` IS NO LONGER A SEPARATE CONTRACT (wave-3 ledger fold
> D2; INTENT #56/#91/F6b).** Scheduling is a **`Schedule` trigger source inside
> `queues`**, so there is no separate cron WS surface: schedule-trigger
> register / update / enable-disable / delete / run-now / list are all facets of
> **`queues-api`**. See `scaffold/contracts/queues-api.md` § "Proposed contracts
> (wave 3)" for the folded schema. The wave-2 standalone contract lives in git
> history (`git log -- scaffold/contracts/cron-api.md`). This file records the
> vocabulary map so nothing is lost in the fold.

## Parties (superseded)

Was: **any service** (via `mesh-client`) ↔ **mesh** (`mesh.cron`). Now: **any
service** (via `mesh-client`) ↔ **mesh** (`mesh.queues`) over `queues-api` — the
same cross-cutting, surface-schema-style single shared document. A schedule is
registered exactly like any other trigger; only its `source` differs.

## Vocabulary map — cron-api → queues-api

The `cron-api` types fold into `queues-api`'s `types::trigger` module as the
`Schedule` trigger source, one-for-one, with **no semantic loss**:

| cron-api (was)                       | queues-api (now)                                              |
|--------------------------------------|--------------------------------------------------------------|
| `CronJob`                            | a `Trigger` whose `source = TriggerSource::Schedule(..)`      |
| `CronJob.job_id`                     | `Trigger.trigger_id`                                          |
| `CronJob.owner`                      | `Trigger.registered_by`                                      |
| `CronJob.schedule: Schedule`         | `ScheduleSource.schedule: Schedule` (moved into `types::trigger`) |
| `CronJob.target: FireTarget`         | `ScheduleSource.target: FireTarget` (`Anywhere` \| `Node(NodeId)`) |
| `CronJob.misfire: MisfirePolicy`     | `ScheduleSource.misfire: MisfirePolicy` (`Skip` \| `FireOnWake`) |
| `CronJob.emit: EmitSpec`             | `HandlerRef::Emit { queue, event_type }` + the `AssemblyTemplate` (payload build) |
| `CronJob.enabled`                    | `Trigger.enabled`                                            |
| `CronJob.last_fired: FireRecord`     | `queues/schedule-state/<trigger_id>` KV row (fire cursor; see queues.md concern 10) |
| `CronFired` event payload            | the emitted `schedule.fired` event (deterministic `event_id`, `catch_up` flag) |
| `CronRequest::Upsert`                | `QueuesClientMsg::RegisterTrigger` / `UpdateTrigger` (schedule source) |
| `CronRequest::Enable`                | `QueuesClientMsg::SetTriggerEnabled`                         |
| `CronRequest::RunNow`                | `QueuesClientMsg::RunTriggerNow` (fires an off-schedule occurrence; still single-fire) |
| `CronRequest::Get` / `List`          | trigger get/list filtered to `source = Schedule`            |
| `CronError::*`                       | `QueuesError::*` (`InvalidSchedule`, `UnknownNode`, `UnknownTargetQueue`, `NotOwner`, `VersionConflict` folded in) |

**Preserved deviations (pinned, not lost):**
- The **deterministic `event_id` recipe** —
  `uuid_v5(SCHEDULE_NS, trigger_id ++ scheduled_for.rfc3339())` — is the cross-
  node single-fire correctness anchor **and** the consume-side dedup key. It MUST
  be computed identically on every node and across versions; changing it is a
  breaking, fleet-coordinated change (carried verbatim from cron-api's Version
  sensitivity into queues-api).
- **Soft/CAP-honest evaluation of `UnknownTargetQueue` (register a schedule before
  its queue exists) and `UnknownNode` (`Node(N)` names an offline walk-along Pi)**
  — both lean *accept + resolve-when-it-appears* (#84), unchanged.
- **Fail-safe unknown-variant evaluation:** a daemon that deserializes a schedule
  into an `Unknown` `Schedule`/`MisfirePolicy` **must not fire it**. The sharp
  edge cron-api flagged (a `Node(N)`-pinned schedule using a kind N cannot parse
  would silently never fire → gate new schedule kinds on fleet capability)
  carries over — see `queues-api.md` Version sensitivity.

The full "run-anywhere / run-on-node" reconciliation, the deterministic-id
rationale, and the pg_cron-replacement example are now in `queues-api.md` (schema
+ example data) and `queues.md` concern 10.
