# cron

**Status:** NEW (wave 2). **Nesting:** internal lib of mesh (module
`lib/mesh::cron`, Ring 4 per `mesh-core.md` § Internal layering). **Prior
requirements:** `mesh.md` concern 11 (LOCKED rounds 4–5, was requirements-only),
INTENT #56 (two flavors), INTENT #91 (pg_cron replacement in the SQLite-
sufficiency story), INTENT #95/#101 (event-ID semaphore + typed events). This
file promotes cron from requirements-only to implementation-ready.

## Charter

`cron` is the tiny, boring scheduled-task lib buried inside the mesh daemon whose
**entire job is to decide *when* something should fire and then publish a
standardized typed EVENT saying it fired** — nothing more. A cron firing "is just
an event"; the actual work is done downstream by a declarative TRIGGER + HANDLER
on the queue the event lands in (INTENT #101/#103). Cron therefore owns exactly
three things: (1) a replicated store of **job definitions** (schedule + fire
target + what event to emit + misfire policy), riding `replicated-kv`; (2) a
per-daemon **tick evaluator** that computes which jobs are due; and (3) the act of
**emitting a `cron.fired` `Event` into a queue** at fire time, acquiring a `locks`
event-ID semaphore first so a run-anywhere job fires **exactly once** across the
fleet. It supports the two flavors that mirror mesh's addressing classes (INTENT
#56/#59): **run-on-node-N** (pinned) and **run-anywhere** (fleet, single-fire).

**Boundary — what cron does NOT own.** It does not run handler code, contains no
handler runtime, and knows nothing about what a job *does* — that is `queues` +
the handler (Deno/SQL) reacting to cron's event. It does not touch any database
(the pg_cron-replacement work happens in a stack/VDB handler triggered by cron's
event, not in cron). It has **no storage of its own** — job rows live in
`replicated-kv`; it does not open the SQLite file (that is `replicated-kv`, which
owns the sole `LocalStore` handle per `mesh-core.md`). It does not implement
semaphores (`locks`), queue delivery/at-least-once (`queues`), pub/sub relay
(`pubsub-relay`), or the WS transport (`mesh-core`). It is an internal library,
never a standalone crate/service (INTENT #54/#55). Cron stays deliberately small:
**schedule store + ticker + one event emit.** Everything that makes scheduled work
*useful* is composed on top of it by triggers/handlers, not built into it.

## Primary design concerns

### 1. The two flavors map onto the addressing classes (INTENT #56/#59)

A job's `target` is either `Node(NodeId)` (pinned) or `Anywhere` (fleet) — the
same virtualized-vs-pinned split `mesh-core`'s `Address` uses, one field on the
job, not two code paths of API.

- **`Node(N)` (run-on-node-N):** only node N's evaluator ever fires this job.
  Every daemon holds the definition (it is replicated), but a daemon evaluates a
  `Node(N)` job **only when it *is* N**. No semaphore needed — there is exactly
  one candidate firer by construction. If N is offline at a scheduled time, the
  misfire policy (concern 4) governs whether it fires on wake or is skipped. This
  is the "tasks that need to be run on specific nodes" case.

- **`Anywhere` (run-anywhere):** every reachable daemon's evaluator independently
  sees the job as due at the same nominal wall-clock and **races to fire it**;
  correctness comes from concern 3's single-fire semaphore, not from leader
  election. This is the "tasks that just need to be run somewhere" case. Racing +
  semaphore (rather than electing an evaluator leader) is the deliberate choice:
  it is more available (no leader is a single point), partition-tolerant, and
  trivially cheap at personal-mesh scale (1–few devices).

### 2. Schedule persistence rides `replicated-kv` — cron has no store of its own

Job definitions are LWW-register rows in a `replicated-kv` keyspace
(`cron/jobs/<job_id>` → `CronJob`), exactly like `service-registry` rows and lock
state. Consequences, all deliberate and boring:

- **Every daemon has the full schedule set** by anti-entropy, so any node can
  evaluate any `Anywhere` job locally with no lookup — the racing model (concern 1)
  needs this.
- **Edits are timestamp-wins** (LWW, INTENT #32): updating a job's schedule from
  the laptop propagates; concurrent edits resolve by `(wall_clock, node_id)` like
  every other KV row. A `CronJob` therefore carries the KV `version` tuple.
- **`last_fired` is written back into the same row** (LWW) by whichever node fired,
  and doubles as the **misfire cursor** (concern 4) — so cron needs *no* separate
  per-node evaluator-cursor state. It is advisory for single-fire (the semaphore is
  authoritative there) but load-bearing for computing what was missed. Because it
  is only ever advanced and reconciled by LWW, staleness or loss is self-healing:
  a stale cursor just recomputes occurrences and re-attempts semaphores that are
  already taken — a safe no-op.

### 3. Single-fire for run-anywhere jobs — the deterministic event-ID semaphore

The correctness core of the `Anywhere` flavor, riding `locks` (INTENT #71/#95).
The nominal fire time is deterministic, so the firing is **content-addressable**:

```
fire_event_id = uuid_v5(CRON_NS, job_id ++ scheduled_for_rfc3339)
```

Before emitting, the firing daemon acquires the `locks` **event-ID semaphore keyed
by `fire_event_id`** (threshold 1). Only the winner publishes the `cron.fired`
event; every other racing daemon fails to acquire and drops silently. This is the
identical event-ID-semaphore mechanism `queues` uses for ~exactly-once
(INTENT #95) — cron does not invent its own concurrency primitive, it *rides* the
one already designed. Two elegances fall out of the deterministic id:

- The **same `fire_event_id` becomes the emitted `Event.event_id`**, so a
  consume-side trigger that opts into the event-ID semaphore (INTENT #95/#101) is
  de-duplicating against the very same key — one deterministic id threads both the
  fire-side single-fire and the consume-side ~exactly-once.
- **CAP honesty (INTENT #84):** the semaphore guarantees single-fire only within a
  connected component. Two partitions each holding a partition-local semaphore may
  each fire an `Anywhere` job; on merge, `last_fired` reconciles by LWW but the
  double-fire already happened. This is the same partition-merge limitation `locks`
  documents and is operator-blessed naive-consistency territory (INTENT #32/#84).
  Because the two fires carry the **identical `event_id`**, they *can* collapse to
  one delivery if `queues` dedups enqueue by `event_id` on partition merge —
  flagged for the queues designer, not claimed here (see friction points). Handlers
  should be written idempotent regardless.

### 4. Misfire policy — node/fleet asleep at fire time (fire-on-wake vs skip)

Per-job policy for the case the responsible node (or the whole fleet, for
`Anywhere`) was down when a scheduled time passed. On each tick a daemon computes
the set of nominal fire times in `(cursor, now]` where `cursor = last_fired.
scheduled_for` (or `created_at`/`Schedule` anchor for a never-fired job):

- **`Skip` (default):** any fire time that passed while unavailable is abandoned;
  only the next upcoming occurrence fires. A backlog never accumulates — the boring,
  safe default for periodic maintenance.
- **`FireOnWake { grace, coalesce }`:** missed occurrences are caught up on wake.
  `grace` bounds how stale a missed occurrence may be to still fire (a missed daily
  job from three weeks ago is usually not worth running). `coalesce = true`
  (default) fires **one** catch-up for a run of missed occurrences rather than a
  storm of back-to-back fires; `coalesce = false` fires each missed occurrence
  (rare; for jobs that must run N times). The emitted event carries `catch_up:
  true` so handlers can distinguish a catch-up from an on-time fire.

For `Node(N)` jobs misfire is purely local to N (its `last_fired` cursor). For
`Anywhere` jobs, catch-up fires go through the **same event-ID semaphore** keyed by
the missed occurrence's `scheduled_for`, so "fire once on wake" is single-fire
across the fleet for free: whichever node wakes first and wins the semaphore fires
it; if some node already fired it before going down, the semaphore is taken and the
catch-up is a no-op.

### 5. Emitting the event — cron → `queues`, never `pubsub` (INTENT #56/#101)

A firing publishes a standardized `types::event::Event` (event vocabulary from the
`types` designer's `event.rs`) into the job's configured **target queue** — *not*
onto the lossy pub/sub bus. Rationale: scheduled work needs at-least-once
durability and the trigger/handler pipeline; `pubsub-relay` is best-effort and
would silently drop a fire (concern 8 of `pubsub-relay.md` draws exactly this
line). The event's payload is `CronFired { job_id, scheduled_for, fired_at,
fire_node, catch_up, payload }`, where `payload` is the job's static declarative
`EmitSpec.payload` passed straight through. Cron does **no** payload assembly, no
rollup, no filtering — that is the downstream **trigger's** job (INTENT #101: "it's
not the event which dictates the payload... it's the trigger"). Cron emits a plain,
uniform fired-event; the trigger filters on `event_type`/`job_id` and assembles the
handler payload (which *may* call rollup). This is what keeps cron tiny: it is a
scheduled *event source*, structurally identical to any other event producer.

Provenance (INTENT #85) is first-order: the emitted `Event` carries `Provenance`
stamped by the local daemon (`origin_node = fire_node`, `origin_service = "cron"`,
`emitted_at = fired_at`), so a handler's causal chain traces back to the exact
scheduled fire.

### 6. The pg_cron-replacement role (INTENT #91) — cron ⊕ queues ⊕ stack

Cron is the "pg_cron → mesh cron" leg of the SQLite-sufficiency story
(`stack.md` analysis, INTENT #91). The operator's frame: "a cron handler inside of
mesh that operates on the database." That decomposes cleanly across three boring
libs, none of which knows the others' internals:

1. **cron** holds the schedule and, on fire, emits `cron.fired` (event_type e.g.
   `stack.<db>.<name>.tick`) into a queue — *knows nothing about databases*.
2. **queues** carries a **declarative trigger** (registered via `queues-api` by the
   stack/VDB layer) that filters that event_type and assembles the handler payload.
3. **stack/VDB** runs the SQL/Deno **handler** against the specific SQLite
   database — the only place database code lives.

So a scheduled nightly rollup or hourly cleanup on a stack database is: one
`cron-api` job + one `queues-api` trigger + one stack handler. Postgres's pg_cron
(scheduled `SELECT cron.schedule('… SQL …')`) is replaced without cron ever
touching SQL — the exact boring, layered decomposition the operator wants. LISTEN/
NOTIFY → `pubsub-relay` and procedural triggers → Deno/SQL handlers are the sibling
legs (not cron's); cron owns only the *scheduled* leg.

### 7. The tick evaluator — naive wall-clock, boring by intent

Each daemon runs one periodic evaluator (aligned to a coarse tick, e.g. 1s or the
minute boundary). Per tick it scans the replicated job set and, for each job it is
responsible for (all `Anywhere` jobs; `Node(N)` jobs only when self == N; only
`enabled` jobs), computes due occurrences in `(cursor, now]` and fires them per the
flavor + misfire rules. Clock handling is **naive wall-clock per node** (INTENT
#32): no NTP assumption, no distributed clock. Skew between nodes is harmless — for
`Anywhere` jobs the earliest-clock node simply wins the semaphore a little sooner;
skew changes *which* node fires and by how much latency, never *whether* it double-
fires (the semaphore protects). `Schedule` supports standard **5/6-field cron
expressions**, fixed **intervals**, and **one-shot** at a timestamp (which self-
disables after firing). Cron-expression parsing reuses a proven crate (`cron` /
`saffron`), never hand-rolled. Timezone defaults to UTC/naive; an optional `tz` is
carried but DST correctness is explicitly out of scope for v1 (naive is blessed).

## Relationships / edges

- **any service ↔ mesh.cron** via `cron-api` — schedule / update / delete /
  enable / list / run-now for "on node N" and "anywhere" jobs; cross-cutting,
  surface-schema-style (one shared document, every service a party — the
  wave2-plan §5.4 model). Rides `mesh-transport`/`pubsub-protocol` on the wire; the
  client half is `mesh-client`. *(authored: scaffold/contracts/cron-api.md)*

**Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45; the Ring-4-
rides-Ring-3 wiring from `mesh-core.md`):**

- `replicated-kv` → consumes its `KvHandle` for the `cron/jobs/*` keyspace
  (schedule store + `last_fired`). Cron's only persistence.
- `locks` → consumes its semaphore API for the fire-side event-ID semaphore
  (`Anywhere` single-fire, concern 3).
- `queues` → consumes its publish API to emit `cron.fired` into a job's target
  queue (concern 5).
- `service-registry` / `network-topology` → consumes the peer/node set to validate
  a `Node(N)` target and to know self-identity in the evaluator.
- `types` → the `Event`, `EventType`, `Provenance` vocabulary the emitted event is
  written in (library dependency, not a contract edge).

**Composed (not a direct cron edge — mediated by the event/queue):** the
pg_cron-replacement path to `stack`/`vdb` (concern 6) is realized through
`queues-api` + a stack handler, never a cron→stack contract. Cron's dashboard
surface (jobs table: next fire, last fire, node, enabled) is aggregated by
`dashboard-serving` via mesh's `surface-schema`, not a cron-authored contract.

## Nesting

Parent: mesh | Children: none. Module `lib/mesh::cron`, Ring 4 (`mesh-core.md` §
Internal layering — cron rides registry + locks + queues + kv). The client half (a
service's schedule/manage handle to its local `:3649` daemon) is part of
`mesh-client`'s surface, like `register`/`resolve` and pub/sub — not a separate
crate. Confirmed at skeleton time.

## Thoroughness level

**implementation-ready** — the two flavors and their evaluator responsibility
rules, the `replicated-kv` schedule store + `last_fired`-as-cursor decision, the
deterministic-`event_id` single-fire semaphore (with its CAP-honest partition
caveat), the fire-on-wake-vs-skip misfire policy with grace/coalesce, the emit-to-
queues (never pub/sub) decision, the naive-wall-clock racing evaluator, and the
pg_cron-replacement decomposition are all decided and specified. The `cron-api`
wire shape is authored in scaffold/contracts/cron-api.md (approach-sketched → the per-pair round finalized
field names and reconciles against `queues`/`locks`). Two items are deliberately
left downstream: the enqueue-dedup-on-merge property (needs the `queues` designer's
confirmation) and the exact cron-expression crate/grammar (a fill-time pick, not a
design fork).

## Assigned design-depth

Opus (single Component-Designer pass, this file), grounded in `mesh.md` concern 11,
`mesh-core.md`'s ring/seam architecture, `pubsub-relay.md`'s queues-vs-pubsub
boundary, `types.md`'s event/provenance vocabulary, `service-registry.md`'s LWW/
lease substrate, and INTENT items 32/56/59/71/84/91/95/101/103.

## Suggested fill-model

**implementation-ready + low complexity → cheap/fast model OK.** Cron is the
smallest L2 lib by design — a schedule struct, a tick loop, and one event emit,
each riding a sibling lib's already-designed API. Reuse a proven cron-expression
crate rather than parsing by hand. Two spots want a careful eye (not a stronger
design pass, just review): (a) the deterministic-`event_id` single-fire path — the
one subtle correctness point, where the `uuid_v5(job_id, scheduled_for)` key must
be computed identically on every node; (b) the missed-occurrence computation in
`FireOnWake` (off-by-one on the `(cursor, now]` interval and coalescing). Both are
conformance-checkable against the example data below.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `cron-api` (any service ↔ mesh.cron) — schedule "on node N" / "anywhere" tasks. → `scaffold/contracts/cron-api.md`

Also a party to (authored elsewhere / cross-cutting): `locks-api`, `queues-api` — see `scaffold/contracts/`.

