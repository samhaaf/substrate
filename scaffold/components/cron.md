# cron — ABSORBED INTO queues (tombstone-with-content-pointer)

> **⚠ TOMBSTONE — `cron` IS NO LONGER A STANDALONE MESH LIB (wave-3 ledger fold
> D2; INTENT #56/#91/F6b — "Cron/scheduling belongs in mesh → a `Schedule(…)`
> trigger inside queues").** There is no `lib/mesh::cron` module and no Ring-4
> cron evaluator. Scheduling is now a **`Schedule` trigger source inside
> `queues`** (`lib/mesh::queues`, Ring 3): a cron firing is *just a scheduled
> event emission*, and every semantic this file designed carries over **intact**.
> See `scaffold/components/queues.md` § "Concern 10 — Scheduled triggers (cron
> absorbed as a `Schedule` trigger source)" for the current design and
> `scaffold/contracts/queues-api.md` § "Proposed contracts (wave 3)" for the wire
> shape. The wave-2 standalone design lives in git history
> (`git log -- scaffold/components/cron.md`). This file is retained as the record
> of what moved and why.

## Why the fold is boring (and a simplification, not just a rename)

Cron was already defined as *"decide **when** something should fire and then
publish a standardized typed EVENT saying it fired — nothing more."* A `queues`
**trigger** was already *"filter + assemble a subject → a handler."* The only gap
between them was cron's **source of firing** (a wall-clock schedule) versus a
trigger's source (an event landing in a queue). The fold closes exactly that gap:
a trigger's source becomes an enum — `TriggerSource::Queue(..)` (event-driven,
today's behavior) **or** `TriggerSource::Schedule(..)` (time-driven, the absorbed
cron). Nothing else about the trigger model changes; the declarative-trigger
vocabulary stays LOCKED (#101/#103).

**Ring simplification.** Cron was Ring 4 (rode `service-registry` + `locks` +
`queues` + `replicated-kv`). Folded into `queues` (Ring 3), the schedule
evaluator needs **no dependency queues did not already hold**: queues already
rides `locks` (Ring-3 sibling — the event-ID semaphore), `replicated-kv` (Ring 2
— schedule store + fire cursor), and `service-registry` (Ring-3 sibling — self
identity + peer set for `Node(N)` targeting), and it *is* the emit target. The
fold therefore **removes a ring level**, not just a file (`mesh-core.md` §
Internal layering: Ring 4 loses its `cron` entry).

## What carried over intact (the semantics pointer)

Every load-bearing decision below is preserved verbatim in `queues.md` concern 10
/ `queues-api.md`; nothing was dropped in the fold:

- **The two flavors = `FireTarget { Anywhere, Node(NodeId) }`** — the same
  virtualized-vs-pinned split as mesh addressing (#56/#59). `Node(N)` is fired
  only by node N's evaluator; `Anywhere` is raced by every reachable daemon and
  made single-fire by the semaphore below. One field, not two API paths.
- **Single-fire via the deterministic event-ID semaphore (#71/#95).** A run-
  anywhere occurrence is content-addressable:
  `occurrence_id = uuid_v5(SCHEDULE_NS, trigger_id ++ scheduled_for.rfc3339())`.
  The firing daemon acquires the `locks` semaphore keyed by `occurrence_id`
  (threshold 1) before emitting; only the winner fires. The **same
  `occurrence_id` becomes the emitted `Event.event_id`**, so a consume-side
  trigger opting into `SemaphoreChoice::EventId` de-dups against the identical
  key — one deterministic id threads both fire-side single-fire and consume-side
  ~exactly-once. This is the identical mechanism `queues` already uses; cron never
  invented its own primitive, and now literally shares it.
- **`Schedule { Cron | Every | Once }`** — 5/6-field cron expressions (proven
  crate, never hand-rolled), fixed intervals, and one-shot-at-a-timestamp (self-
  disabling after fire). Naive UTC wall-clock per node (#32); optional `tz`
  carried but DST correctness out of scope for v1.
- **`MisfirePolicy { Skip | FireOnWake { grace, coalesce } }`** — the fire-on-wake
  vs skip choice for a node/fleet asleep at a scheduled time; catch-up fires ride
  the same occurrence-keyed semaphore, so "fire once on wake" is single-fire
  across the fleet for free. `catch_up: true` on the emitted event distinguishes a
  catch-up from an on-time fire.
- **Schedule store rides `replicated-kv`; no store of its own.** Schedule-source
  triggers are ordinary trigger rows; the per-occurrence **fire cursor**
  (`last_fired`, the misfire cursor) lives in a `queues/schedule-state/<trigger_id>`
  keyspace (a refinement over cron's in-`CronJob`-row writeback — see queues.md
  concern 10, so a fire never LWW-races a definition edit).
- **The pg_cron-replacement decomposition (#91) is unchanged.** A schedule trigger
  **emits** a standardized `schedule.fired` event into a target queue
  (`HandlerRef::Emit { queue, event_type }`); a downstream event-driven trigger
  filters/assembles it; a stack/VDB handler runs the SQL. cron still touches no
  database; the three-lib decomposition (schedule → trigger → handler) is intact,
  now all inside one lib boundary.
- **Provenance first-order (#85):** the emitted event carries `Provenance`
  stamped by the firing daemon (`origin_service = "queues"`,
  `origin_node = fire_node`, `emitted_at = fired_at`).

## Contract fold

`cron-api` is tombstoned into `queues-api` — see
`scaffold/contracts/cron-api.md` (tombstone) and `scaffold/contracts/queues-api.md`
(§ "Proposed contracts (wave 3)", the `TriggerSource::Schedule` shape absorbing
`CronJob` / `Schedule` / `FireTarget` / `MisfirePolicy`). Schedule-trigger
management (register / enable-disable / run-now / list) is now a facet of
`queues-api`, not a separate `cron-api` document.
