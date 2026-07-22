# chassis

**Status:** NEW (wave 3). Folds the `#156` "proto-daemon core lib" into the
scaffold and **retires `mesh-client` into it** (ledger D1; `mesh-client.md`
becomes a tombstone-with-content authored by its own unit). **Nesting:**
top-level **shared-lib** (`lib/chassis`), L1 conceptually, compiled into every
service. **Grounding:** INTENT #156 (the daemon wrapper — the load-bearing
source), #77/#98 (the LOCKED 4-level restart ladder), #152 (universal mediation +
promises), #154 (the success/error/promise envelope switch), #155 (configurable
lossiness), #157 (blessing queue + walk-along Pi), #151/#166 Q11 (DC = generated
Rust libs), #62/#131/#145 (schema builds types); ledger §A rows 8, 24, 36–40, 62;
ledger §C OQ-1 design-around; the existing `mesh-client.md`, `supervision.md`,
and the `restart-protocol` / `pubsub-protocol` / `service-lookup` /
`surface-schema` contracts; seed-bishop synthesis **F-5** (walk-along Pi = a
stripped-down chassis profile: outbox + blessing queue, no service surface).

## Charter

`chassis` is the **daemon-wrapper library every service is built on** (INTENT
#156, name LOCKED #166 Q15). It is the single construct a crate links to *become
a service on the mesh*: it brings the daemon online, holds the one persistent
WebSocket to the local mesh daemon on `:3649`, establishes the service's **data
contracts**, and — the reason it is a wrapper and not merely a client — it
**forces the service to handle every case at compile time** (INTENT #156,
"every application FORCED to handle every case, Rust-exhaustive-switch style").
A service author writes their role logic; chassis owns bring-up, reconnect,
registration, surface publication, the restart-protocol client half, the promise
machinery, the outbox, and the blessing-queue seam — **once, correctly**, so no
service reinvents them and they never drift.

chassis is the union of two things the scaffold previously split: (a) everything
`mesh-client` was — the tiny wiring handle (register/resolve/subscribe/heartbeat
over one socket) — plus (b) the `#156` proto-daemon additions that turn a wiring
handle into a *service skeleton*: contract-establishment, exhaustive-case
enforcement, the promise machinery, the durable outbox, and the blessing queue.
mesh-client is absorbed wholesale (§ "Absorption of mesh-client" below); the name
`mesh-client` retires.

**Mental model.** A service's `main` is essentially:

```rust
let chassis = Chassis::builder("inference", version!())
    .requires([dep("db", ">=1.4,<2"), dep("kg", "^1")])
    .surface(inference_surface_schema())          // published + kept fresh (surface-schema)
    .restart_policy(MyRestartPolicy)              // service SUPPLIES the wind-down policy (#156)
    .serve::<InferenceContracts>(handlers)        // GENERATED trait — every arm required (compile-time)
    .await?;                                       // registered, surface published, subscriptions live
chassis.run().await;                              // owns the socket, reconnect, heartbeat, restart, promises
```

Everything after `serve` is chassis's; the only things the author supplies are
the manifest (slug/version/requires), the surface schema, the restart *policy*
(not mechanism), and an exhaustive set of typed handlers the compiler refuses to
let them leave incomplete.

**What chassis does NOT own.** No registry *state* (that is `service-registry`,
server side), no relay/routing (mesh-core / `pubsub-relay` / `completion-router`),
no queue/lock/cron *semantics* (L2 mesh libs — chassis only ferries their typed
frames, "carried, not owned" below), no restart *policy for the fleet* (that is
`supervision`, the daemon side; chassis is only the client half + callback seams),
no *decision about who blesses* a consistency-requiring change (PARKED OQ-1 — the
blessing target is an abstract seam, §7), and no application data. Scope creep in
chassis is scope creep in every app crate, so the discipline is strict: **if a
capability's meaning lives in another module, chassis only carries its typed
envelope; it does not learn its domain.**

## Primary design concerns

### 1. Daemon bring-up — one socket, registered, surfaced, subscribed

chassis holds **one** connection — `ws://127.0.0.1:3649` — and multiplexes every
interaction over it (single-port locality, INTENT #58; everything-over-WS #28/#53;
the two-way restart protocol forces a *persistent bidirectional* WS, never a
per-call HTTP client, because mesh must be able to *push* an unsolicited restart
signal). `serve()` runs a fixed bring-up sequence:

1. **Connect** to the local daemon (fail-fast if absent, then the reconnect loop).
2. **Register** via `service-lookup` (`Register { reg }`): slug, node, endpoint,
   `addressing`, `ttl_secs`, and `meta` (version + pairwise `requires` — the same
   `ServiceMeta` `supervision` reads for boot order and compatibility). Receives a
   `Lease`.
3. **Publish surface** via `surface-schema` (`PublishSurface { schema }`) — the
   boring schema-driven dashboard surface (INTENT #46).
4. **Subscribe** the service's declared `TopicFilter` set (`pubsub-protocol`).
5. **Start the maintenance tasks:** heartbeat/lease-renewal, the restart-protocol
   reader, the interruptibility feed, the promise reaper, and the outbox drainer.

All five are **idempotent and replayed on every reconnect** (concern 8). This is
exactly `mesh-client`'s bring-up, now the front half of the wrapper.

### 2. Contract establishment — the DC surface chassis binds to

A service's contracts are **not hand-written wire code**; they are the **generated
Rust libraries** `schema` emits (INTENT #145/#166 Q11 — "DC = EMERGENT from chassis
+ schema + generated Rust libraries; no DC service"; ledger §A row 62). schema
codegens, per contract the service is a party to:

- a request/message **enum** (one variant per message the service may *receive*),
- a **reply type** per request variant,
- the **event** payload structs it may publish/subscribe.

chassis consumes these to synthesize, per contract, a **`Contract` trait** whose
method set is *exactly the generated variants*. The service `impl`s it. Because
Rust requires every trait method to be implemented, **a missing message arm is a
compile error** — this is the mechanism behind INTENT #156's exhaustive-switch
requirement (concern 3 makes it exact). "Establishing the data contract" =
`serve::<C>(handlers)` accepting a `handlers: impl C` that only typechecks when
every case is covered. The generated `C` is the "DC" surface (the name kept for
the published-type surface, #158/#166 Q11), and chassis is the party that turns
it from *types* into a *served contract*.

Version stamping (INTENT #113, LOCKED) rides here: every outbound frame carries
the service's name + version (`Provenance.service` + `service_version`), and a
handler may declare a **version floor** on an inbound contract; a below-floor
message is rejected at the edge with a catchable `VersionBelowFloor`-shaped error
+ a please-update warning back to the sender (never a silent drop). chassis owns
the floor check as a generated, per-contract edge policy (pubsub-protocol homes
the wire discipline).

### 3. Exhaustive-case enforcement — the compile-time forcing, exactly

Three axes must each be total, and chassis makes each a compile-time obligation:

**(a) Inbound message variants.** As concern 2: the generated `Contract` trait has
one method per receivable variant; `impl C` is total or it does not compile. No
`_ => unimplemented!()` escape is offered by chassis's generated trait (no default
methods, no catch-all) — the service must name each case. (Forward-tolerance for
*unknown future* variants is a separate, wire-level concern handled by
`#[serde(other)]` on the generated enums, so an older service degrades a
never-seen variant to a typed `Unknown` arm rather than failing to deserialize —
that arm, too, is a method the author must implement, so "what do we do with a
message from the future" is itself a forced decision.)

**(b) The transport outcome switch — success / error / promise (INTENT #154).**
Every `request(...)` chassis issues returns not `Result<T>` but the **outer
envelope switch**:

```rust
enum Outcome<T> {            // the #154 outermost layer, before the inherited schema inward
    Success(T),
    Error(MeshError),
    Promise(PromiseHandle<T>),   // the value will arrive later, pushed back over WS (#152)
}
```

Because `Outcome` is a plain (non-`#[non_exhaustive]`) enum, **a caller that
`match`es it must handle the `Promise` arm or the code will not compile** — this
is the literal enforcement of INTENT #152's "every inter-service request must have
a handler for the promise case." chassis additionally offers an ergonomic form
`request_awaited(...)` that *itself* supplies a default promise continuation
(await-the-handle) — but the primitive `request` returns the bare `Outcome`, so
the forcing is real and opt-out is explicit, never accidental.

**(c) Restart severity (concern 5).** The restart policy the service supplies is a
trait with a method per ladder level; the 4-level ladder being a LOCKED closed
enum means the policy is total by construction.

This concern is why chassis is a *wrapper*, not a client: the client half of every
protocol is expressed as a set of traits the compiler will not let the service
leave partial.

### 4. The promise machinery — issuing, tracking, fulfilling, caller enforcement

Promise-based messaging (INTENT #152): a service that cannot answer instantly
returns a **promise**; the caller moves on; the value is pushed back over WS. Both
halves live in chassis.

**Issuing (callee side).** When a handler cannot produce its reply synchronously,
it returns `Reply::Promise(promise)` instead of `Reply::Now(value)`. chassis
immediately sends a `Promise{ promise_id, .. }` outcome back to the caller's daemon
(so the caller's `request` resolves to `Outcome::Promise` at once — "all
communication attempts are instant," #152), and registers the `promise_id` in a
**PromiseRegistry** keyed to the reply route (origin envelope id + return topic).
The handler keeps the `promise` token; later it calls
`chassis.fulfill(promise, value)` (or `reject(promise, err)`), and chassis pushes
the resolution back over WS to the waiting caller. Promise resolutions ride the
distributed intermediate-response cache (#155), so a caller that disconnected
before fulfilment still collects the value on reconnect (never a silent drop).

**Tracking.** The registry holds pending promises with: `promise_id`, the return
route, a **deadline** (issued promises are not immortal — a reaper task expires
stale ones into `PromiseExpired`, surfaced, never leaked — INTENT #38 no
unbounded state), and the causal ids (`correlation_id`/`causation_id`) so a
promise chain is traceable.

**Fulfilling (caller side).** `PromiseHandle<T>` is a typed future the caller may
`await`, register a callback on, or store. chassis correlates the incoming
resolution frame (`PromiseResolved{ promise_id, outcome }`) to the outstanding
handle and completes it. Resolution is itself a nested `Outcome<T>` (a promise may
resolve to `Success` or `Error`), and the handle's completion type forces the
caller to handle both — the promise arm bottoms out in an exhaustive success/error
match, never an untyped value.

**Streaming interplay (INTENT #153).** A promise whose payload is a *large*
transfer resolves not to inline bytes but to a **broker ticket** for a
mesh-brokered, one-directional stream-tunnel (bytes off the relay, #114/#153);
chassis exposes the ticket, the actual tunnel is mesh-core's. chassis never opens a
permanent service-to-service connection — two-directional always goes back through
the mesh.

### 5. Restart-severity handling — signal-in, service decides how to wind down

INTENT #156 reframed the restart signal: **the signal goes IN (restart severity),
and "the service has to determine how it shuts itself down. Requests are
requests."** chassis provides the **callback seams**; the service supplies the
**policy**. It does NOT invent per-app idle/critical semantics — those are governed
by the dedicated restart-philosophy plugin (INTENT #119, OQ-27; supervision
concern 3's deferral applies verbatim here).

The LOCKED 4-level ladder (`types::restart::RestartPriority`, restart-protocol)
maps one-to-one onto chassis callbacks:

| Level | `types::restart` | chassis seam | Service supplies |
|-------|------------------|--------------|------------------|
| L1 | `WaitForIdle` | *passive* — chassis reports `Interruptibility` from the service's live feed; no callback | the interruptibility feed (what counts as `Idle`/`Interruptible`/`CriticalSection`) |
| L2 | `FinishAndRelinquish` | `on_restart(L2) -> Yielded` | finish in-flight work, then relinquish |
| L3 | `SaveWindow` | `on_restart(L3) -> Saved` under `save_deadline` (~10s) | persist state; chassis yields on `Saved` **or** deadline |
| L4 | `Kill` | none (best-effort SIGTERM hook only) | nothing — process is killed |

The seam is a single trait the service implements:

```rust
trait RestartPolicy {
    async fn on_restart(&self, priority: RestartPriority, req: &RestartRequest) -> Wound;
    fn interruptibility(&self) -> watch::Receiver<Interruptibility>; // the continuous feed
}
enum Wound { Yielding, Saved }   // maps to RestartResponse::{Yielding, Saved}
```

chassis owns the **client-side ladder mechanics**: it answers L1 purely from the
feed; for L2/L3 it invokes `on_restart` under the reason-appropriate deadline,
maps the return onto `RestartResponse`, and — the correctness edge — **never blocks
the daemon forever**: if `on_restart` overruns or panics, chassis yields anyway (a
panicking policy is contained; the daemon's supervision side escalates the ladder).
The service decides *how* it winds down; chassis guarantees it winds down. Level
*selection*, escalation, and port-handoff choreography are `supervision`'s (daemon
side) — chassis is a party to `restart-protocol`, not its author.

### 6. The outbox — queue-while-offline, send-on-reconnect (the thin-profile heart)

The local daemon is a service's only door; losing it isolates the service (but
transiently — mesh is sticky, resurrected by launchd/systemd). chassis's send path
is **lossiness-tiered per message** (INTENT #155 "a required field indicating
whether failed deliveries should be saved — we can't just be silently dropping
things"):

- **Lossy sends** (fire-and-forget pub/sub, telemetry): a **bounded** in-memory
  buffer, drop-oldest with a dropped-count metric — explicitly no unbounded queue
  (INTENT #38). This is `mesh-client`'s original buffer, retained.
- **Durable sends** (marked `save_on_fail`): the **OUTBOX** — a small
  **chassis-local durable store** (SQLite, per INTENT #72 SQLite-only-locally; a
  flat append log on the thinnest profile) that survives both daemon outage *and
  the service process restarting*. On reconnect, chassis **drains the outbox in
  order**, re-sending each entry; entries are removed only on daemon acceptance.

The outbox MUST be chassis-local (not the mesh `queues` service) precisely because
its job is to work **when the daemon is unreachable** — it cannot depend on the
thing it exists to survive. Ordering is FIFO per destination; at-least-once on
redelivery (duplicates are the receiver's idempotency concern, consistent with
`queues-api`'s event-id semaphore model).

**The walk-along-Pi thin profile (INTENT #157; seed-bishop F-5).** The thinnest
chassis profile is **feature-gated to just the outbox + the blessing queue, with
no service surface** — no registration, no surface schema, no restart
participation, no inbound handlers. A walk-along Pi (or any intermittently
connected small client) links `chassis` with `default-features = false, features
= ["outbox", "blessing"]`, queues messages locally while disconnected, and flushes
on reconnect. F-5 named this the concrete customer for half the promise-and-
blessing machinery; making it a *profile of the same lib* (not a separate crate)
is why the outbox and blessing queue live in chassis rather than in a service.

### 7. The blessing queue — mechanics designed, target ABSTRACT (PARKED OQ-1)

INTENT #157: the daemon wrapper gains a **blessing queue** — "apps work locally as
much as possible, queueing consistency-requiring changes for [blessing]; on
reconnect the [target] blesses or pushes back with conflicts." chassis designs the
**queue mechanics**; it **must NOT decide who blesses** (OQ-1 is PARKED — the
authority-node-vs-no-central-node question, ledger §B.1/§C).

**What chassis designs (mechanics):**

- A change that requires consistency blessing is enqueued as a **`BlessingRequest`**
  in a chassis-local durable queue (same store class as the outbox), tagged with
  the change's schema id, a client-chosen `change_id` (idempotency key), and the
  causal ids. The service treats the change as **locally applied but
  unblessed** — it may proceed on local state (offline work must work, #157) while
  the request is pending.
- On (re)connection to the blessing path, chassis submits pending requests **in
  order** to the **abstract `BlessingTarget`** (below) and awaits, per request,
  one of: `Blessed { change_id }` → the change is confirmed, dequeued; or
  `Rejected { change_id, conflicts: Vec<Conflict> }` → chassis hands the conflicts
  to the service's **conflict-resolution seam** (each affected application decides
  "how do you want to handle this?", INTENT #163) and re-enqueues the resolution.
- Exactly-once semantics via the `change_id` (the same at-least-once-atop-semaphore
  discipline as `queues`): a re-submitted request the target already blessed
  returns `Blessed` idempotently.

**The abstract seam (what chassis must NOT decide — OQ-1):**

```rust
trait BlessingTarget {                        // OPTIONAL, stubbed; who implements it is OPEN
    async fn submit(&self, req: BlessingRequest) -> BlessingOutcome;
}
enum BlessingOutcome { Blessed { change_id: Id }, Rejected { change_id: Id, conflicts: Vec<Conflict> } }
```

Per the ledger §C OQ-1 design-around, the **most-boring provisional** binding is a
**merge-reconciler surface on `locks`** (a per-mesh write-blessing semaphore that,
on partition-merge, emits a conflict per affected application — INTENT #163's
no-central-authority path), and chassis ships that as the default `BlessingTarget`
impl behind a stub. But the trait is written so the *same* seam can later point at
a cloud authority node **without rewriting chassis or any other contract**. The
scaffold records the target as **OPEN**: chassis threads **no** authority
dependency into any other contract; there is no `authority` party, no
`authority-blessing` contract family. The blessing queue is present and mechanical;
who answers it is the parked discussion.

> **PARKED — do not decide (OQ-1).** Whether a dedicated authority node exists,
> whether it lives in the cloud, and whether it blesses anything beyond lock
> conditions are the operator's to settle. chassis keeps `BlessingTarget` a
> one-line-swappable seam and defaults it (stubbed) to the `locks` reconciler.

### 8. Reconnect, re-announce, lease — carried from mesh-client, load-bearing

Unchanged from `mesh-client` (concerns retained verbatim in spirit):

- **Reconnect loop** with bounded exponential backoff (e.g. 100 ms → 5 s cap; never
  unbounded).
- **Observable `ConnectionState`** (`tokio::sync::watch`) the service reads to gate
  its own behaviour.
- **Fail-fast RPC** while disconnected: `request()` awaits reconnect only to a short
  grace deadline, then returns `MeshError::LocalDaemonUnreachable`.
- **Re-announce on reconnect is the load-bearing behaviour:** chassis holds the
  service's full *desired mesh state* — registration, surface schema, active
  subscriptions, current interruptibility — and **replays all of it on every
  reconnect** (registration idempotent via LWW-register keyed by slug + fresh
  `version`/`generation`). It **additionally** drains the durable outbox and
  re-submits the blessing queue (concerns 6–7) — the two new replays chassis adds
  over mesh-client.
- **Lease lifecycle** the service never thinks about: register → lease TTL →
  heartbeat renew → deregister on graceful shutdown → expiry-tombstone on crash →
  revive under a new `generation` on reconnect.

### 9. Wire version-skew is real and LOCAL (INTENT #66)

chassis is compiled-in (no runtime version tracking as a library, #45) but talks
over the wire to the **mesh daemon, a separate process that revs independently**
under minimal-restart rolling updates. So the daemon a service connects to may be
older or newer than the chassis compiled into it. The multiplexed **transport
frame** must carry a `protocol_version` and be **additive-only / tolerant of
unknown frame kinds** (ignore-unknown, reserved space) — this is `mesh-transport`'s
`Frame` (the transport-multiplexing wrapper, distinct from the pub/sub `Envelope`
header, per pubsub-protocol reconciliation note 6). This is the one genuinely
version-sensitive surface in an otherwise compiled-in library, easy to miss because
"it's a lib, libs don't version."

## Absorption of mesh-client — what carries over

`mesh-client` retires into chassis (ledger D1; its file becomes a tombstone
authored by its own unit). Everything mesh-client owned is now a chassis
sub-surface, unchanged in behaviour:

- **register / deregister / resolve** wrappers — the `service-lookup` client half
  (both addressing classes: `Address::AnyNode` vs `Address::Node`; `resolve()`
  returns a *browsable* `Endpoint` for humans/health/the CLI, but the RPC hot path
  is `request(Address, …)` relayed by the local daemon — a service must NOT dial a
  resolved endpoint directly under single-port locality, #58).
- **subscribe / unsubscribe / publish** wrappers — the `pubsub-protocol` client
  half over the one socket.
- **heartbeat / lease renewal**, **reconnect + re-announce**, **`ConnectionState`**
  (concern 8).
- **restart-protocol participation** — now the richer `RestartPolicy` seam
  (concern 5) instead of the bare `RestartParticipant`.
- **surface-schema publication** (pass-through, re-published on reconnect).
- **Carried, not owned:** `queues-api`, `locks-api`, `cron-api` ride the generic
  `Request` frame of the transport; chassis serializes/relays their typed envelopes
  (from `types`) but implements none of their semantics.

The three mesh-client reconciliation losses already recorded in `restart-protocol`
(RestartLevel→`RestartPriority`, save-deadline placement, closed-`RestartReason`)
and in `pubsub-protocol` note 6 (its multiplexing `Envelope` → the mesh-transport
`Frame`) carry forward unchanged; chassis adopts the winning shapes.

## Relationships / edges

chassis is the **universal client half** of every cross-cutting "service ↔ mesh"
protocol; the server halves live in mesh-core and its L2 libs. It is a **party** to
these contracts (pointer-style — the contract files are authoritative; chassis does
not rewrite them):

- **`service-lookup`** (party, client half) — register / deregister / resolve /
  heartbeat; the wiring seam. → `scaffold/contracts/service-lookup.md`
- **`restart-protocol`** (party, client half) — the 4-level ladder callback seams +
  interruptibility feed; `supervision` is the daemon-side authority, `types::restart`
  the struct home. → `scaffold/contracts/restart-protocol.md`
- **`pubsub-protocol`** (party, client half) — subscribe/publish over the local
  daemon; the pub/sub `Envelope`. → `scaffold/contracts/pubsub-protocol.md`
- **`surface-schema`** (party, client half, pass-through) — publish + keep fresh
  across reconnects. → `scaffold/contracts/surface-schema.md`
- **`mesh-transport`** (party, client half) — the **success/error/promise outer
  switch** (INTENT #154) + the `protocol_version` transport `Frame` + `PeerUnreachable`.
  **NEW contract, owned by a sibling batch-1 unit** (`types` / `mesh-transport`);
  chassis is the client-side party and proposes the promise + blessing client-half
  shapes below. → `scaffold/contracts/mesh-transport.md` *(to be authored)*
- **generated DC types** — chassis's `Contract` traits are synthesized over the
  Rust libraries `schema` codegens (INTENT #145/#166 Q11); a **shared-lib / codegen
  dependency, NOT a contract edge** (ledger §A row 62). → `scaffold/components/schema.md`
- Imports `substrate-types` (envelope, event, restart, surface, error, id
  vocabulary) — a shared-lib dependency, NOT a contract edge.

**Blessing target (OQ-1, kept off the contract graph on purpose):** chassis's
`BlessingTarget` seam defaults (stubbed) to a `locks` merge-reconciler surface but
introduces **no** authority contract; the target stays an internal, swappable seam
until the parked discussion resolves.

## Nesting

Parent: (top-level shared-lib) | Children: none. `lib/chassis`, L1 conceptually,
a standalone crate any app crate compiles in. Depends only on `substrate-types`,
the `schema`-generated DC crates, a small durable-store dependency for the outbox +
blessing queue (embedded SQLite / append log), and a WS/runtime stack (`tokio`,
`tokio-tungstenite`, `serde`, `serde_json`, `futures`) — deliberately **NOT** on
`lib/mesh` (the whole reason it exists, inherited from mesh-client). Feature-gated
**profiles**: `full` (default — the whole service skeleton) and `thin` (outbox +
blessing queue only — the walk-along-Pi profile, concern 6).

## Thoroughness level

**implementation-ready** — for chassis's own mechanics: the bring-up sequence, the
absorbed mesh-client surfaces (connection/reconnect/re-announce/lease), the
exhaustive-case enforcement mechanism (generated `Contract` traits + the `Outcome`
switch), the restart-severity callback seams, the promise registry (issue / track /
fulfil / caller-enforce / expire), the lossiness-tiered send path + durable outbox,
the thin profile, and the blessing-queue mechanics are all specified above.

**Deliberately left open (by directive, not by gap):**
- **The blessing target (OQ-1, PARKED).** Who/what implements `BlessingTarget` is
  the operator's parked call; chassis ships the boring `locks`-reconciler default
  behind a stub and keeps the seam one-line-swappable. Marked OPEN.
- **Per-app idle/critical restart semantics (OQ-27).** What a service reports as
  `Idle` vs `CriticalSection`, and its `on_restart` wind-down policy, are
  philosophy-governed (INTENT #119) — chassis provides the seam, not the policy.
- **The `mesh-transport` envelope struct** (co-owned by `types` / `mesh-transport`,
  batch 1) — chassis's logic is written *parametric* on the `Outcome`/`Frame`
  shapes; the crate cannot be filled until that envelope is harmonized. A
  sequencing constraint, not a design gap (mirrors mesh-client's original note).

## Assigned design-depth

Opus, single Component-Designer pass (this file), grounded on `mesh-client.md`
(the surface it absorbs), `supervision.md` (the daemon-side restart authority it is
the client half of), `restart-protocol` / `pubsub-protocol` / `service-lookup` /
`surface-schema` contracts, INTENT #77/#98/#113/#151/#152/#153/#154/#155/#156/#157/
#166, ledger §A/§B.1/§C (esp. the OQ-1 design-around), and seed-bishop synthesis
F-5.

## Suggested fill-model

**implementation-ready + high complexity → strong model, with one hard sequencing
constraint.** The absorbed mesh-client mechanics (reconnect/re-announce/heartbeat)
and the outbox drainer are mechanical (mid model OK). Three spots reward a careful
hand and must not go to a cheap model: (1) the **generated-`Contract`-trait
codegen + `Outcome` switch** — the compile-time exhaustiveness is the whole point
of the wrapper and must actually force, not merely encourage; (2) the **promise
registry** (issue/track/fulfil/expire without leaking state or double-resolving);
(3) the **blessing queue's abstract seam** — it must stay swappable and must never
leak an authority dependency into the rest of the system (OQ-1). Fill chassis
**after** the Contract Harmonizer freezes the `mesh-transport` envelope (the
`Outcome`/`Frame` structs) and the `types` pubsub/event/surface/restart modules,
and **after** `schema`'s codegen surface is defined (chassis's `Contract` traits
are synthesized over it). Given that ordering, the design has bought down the fill
risk except for those upstream dependencies, which are scheduling constraints.

---

## Proposed contracts (wave 3)

chassis owns the **client-half shapes** for two capabilities that are new this
wave and whose server/struct halves live in sibling-owned files. These are
**proposals for the contract round** (chassis does not edit the sibling files);
the harmonizer reconciles them into `mesh-transport` and (if the operator
un-parks OQ-1) a future blessing contract.

### P1. Promise client-half → into `mesh-transport` (INTENT #152/#154)

The `#154` outer switch and the `#152` promise machinery are the same envelope;
chassis proposes the **client-facing** shape (the `mesh-transport` unit owns the
wire struct + relay behaviour):

```rust
// the outer switch every request resolves to (caller MUST handle all three arms)
enum Outcome<T> { Success(T), Error(MeshError), Promise(PromiseHandle<T>) }

// callee-side: a handler that can't answer now returns a promise
enum Reply<T> { Now(T), Promise(PromiseToken<T>) }

// the pushed-back resolution frame (rides the #155 distributed intermediate cache)
struct PromiseResolved { promise_id: Id, outcome: OutcomeWire }   // OutcomeWire = Success|Error (no re-promise)
```

- **Enforcement claim:** `Outcome` is a non-`#[non_exhaustive]` enum, so a caller
  `match` that omits `Promise` fails to compile — the compile-time realization of
  "every inter-service request must handle the promise case" (#152).
- **Reconciliation flags for the harmonizer:** confirm the promise resolution
  cache is the `#155` distributed KV intermediate-response cache (owned by
  pubsub-relay / mesh-core); confirm large-payload promises resolve to a
  `#153` broker ticket, not inline bytes; confirm `PromiseExpired` lives in
  `types::error` alongside `MeshError`.

### P2. Blessing client-half → a NEW seam, target-abstract (INTENT #157; OQ-1 PARKED)

chassis proposes the **queue + submission client shape** and the **abstract
target trait**; it proposes **no** server party (that is the parked authority
question). To be recorded as a **candidate seam**, not an authored contract:

```rust
struct BlessingRequest { change_id: Id, schema_id: SchemaId,
                         payload: Box<RawValue>, correlation_id: Option<Id>, causation_id: Option<Id> }
enum  BlessingOutcome  { Blessed { change_id: Id }, Rejected { change_id: Id, conflicts: Vec<Conflict> } }
trait BlessingTarget   { async fn submit(&self, req: BlessingRequest) -> BlessingOutcome; }  // impl OPEN
```

- **MUST NOT decide (OQ-1):** who implements `BlessingTarget`. Default (stubbed)
  binding = a `locks` merge-reconciler surface (the `#163` no-central-authority
  path, ledger §C). No `authority` party, no `authority-blessing` contract family,
  no authority dependency threaded anywhere else.
- **Keep flexible:** the seam must accept a future cloud-authority impl with a
  one-line swap; `locks` keeps a `Strictness` extension point (ledger §C). Mark
  the target **OPEN** in the contract graph.
