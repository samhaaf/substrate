# Contract: restart-protocol

## Parties

- **mesh** (`supervision`, the daemon side — authoritative view) — signals restarts
- **every service** (via `chassis`, the client half — formerly `mesh-client`,
  absorbed, see `components/mesh-client.md`) — reports interruptibility, yields

Cross-cutting, surface-schema-style: ONE shared document, every service is a
party (wave2-plan §5.4). Struct vocabulary is homed in `types::restart` +
`types::error::supervision`; the daemon behavior is `supervision`'s, the client
behavior is `chassis`'s.

## Purpose

The two-way graceful-restart / supervision choreography built into EVERY service
from the beginning (INTENT #76 continuous interruptibility, #77 the LOCKED
4-level ladder). The service continuously reports interruptibility; mesh signals
a restart need at a laddered priority and drives port-handoff + kill. The
protocol rides `chassis`'s one persistent bidirectional WS to the local daemon
(mandatory — mesh must *push* an unsolicited restart signal) as ordinary
`Request`/`Response` `Frame`s in `mesh-transport`'s multiplexing sense (wave-3
correction: this is **not** a `pubsub-protocol` control frame — restart signals
are daemon-initiated `Request`s over the same socket pub/sub rides, distinct
from the `Publish`/`EventDelivery` data-path kinds; see `contracts/mesh-transport.md`
§3 `MsgKind`). Restart is **primarily node-local** (a service is supervised by
its OWN local daemon), but a control frame may be relayed cross-node during a
skew window.

**Adjacent seam, not this contract's.** How mesh-core decides *when* to ask
supervision to bring a slug up, and how it learns the result is ready to route
to, is the **`ensure_up` + readiness** primitive between `mesh-core` and
`supervision` — an in-process seam, not a wire contract, authored in
`components/supervision.md` concern 11. This file is strictly the wire protocol
between the daemon and a *service the daemon already knows about*.

## Schema

### The ladder + vocabulary (`types::restart`)

```rust
// the LOCKED 4-level ladder, ordered ascending in severity
enum RestartPriority { WaitForIdle, FinishAndRelinquish, SaveWindow, Kill }
enum RestartReason   { Compatibility, Update, OperatorRequested, HealthRemediation, PortConflict }

struct RestartRequest {
    v: u16,
    request_id: Uuid,
    priority: RestartPriority,
    reason: RestartReason,
    save_deadline: Option<Duration>,     // meaningful at SaveWindow; ~10s typical
    requested_at: DateTime<Utc>,
}

enum RestartResponse {
    Acknowledged { will_yield_by: Option<DateTime<Utc>> },
    Busy { state: Interruptibility, retry_after: Option<Duration> }, // honored ONLY below SaveWindow
    Yielding,   // finishing-then-relinquishing in progress (L2)
    Saved,      // state persisted, ready to be replaced (L3)
}

enum Interruptibility { Idle, Interruptible, CriticalSection { until: Option<DateTime<Utc>> } }

struct PortHandoff { service: Slug, old: SocketHint, new: SocketHint, at: DateTime<Utc> }
struct SocketHint  { host: String, port: u16 }   // pure data, NOT a live socket

struct ServiceManifest { slug: Slug, version: String, requires: Vec<Requirement> } // at registration
```

### Daemon → service (`SupervisionServerMsg`, unsolicited push)

```rust
enum SupervisionServerMsg {
    Restart(RestartRequest),          // signal a restart need at a priority
    PortHandoffNotice(PortHandoff),   // FYI: your slug's front door moved
    PollInterruptibility,             // request an immediate state report (rare; feed is push-default)
}
```

### Service → daemon (`SupervisionClientMsg`, client half via `chassis`)

```rust
enum SupervisionClientMsg {
    InterruptibilityUpdate { state: Interruptibility }, // continuous, on change
    RestartReply(RestartResponse),                      // Acknowledged | Busy | Yielding | Saved
    Manifest(ServiceManifest),                          // version + `requires`, at registration
}
```

### Client-side handler contract (`chassis`, in-process) — wave-3 reconciled

**Superseded (recorded, not dropped).** This section originally specified the
bare `mesh-client` seam `trait RestartParticipant { async fn on_restart(&self,
priority: RestartPriority) -> Yielded; }`. `chassis` (batch 1/wave 3) is now the
authoritative client-half author and ships the richer seam below; `chassis.md`
concern 5 already flags the supersession, this file adopts it as the wire
contract's own record so the two files never drift again.

```rust
trait RestartPolicy {
    async fn on_restart(&self, priority: RestartPriority, req: &RestartRequest) -> Wound;
    fn interruptibility(&self) -> watch::Receiver<Interruptibility>; // the continuous feed
}
enum Wound { Yielding, Saved }   // maps 1:1 onto RestartResponse::{Yielding, Saved}
```

Two changes from the superseded shape, both adopted: (1) `on_restart` receives
the full `&RestartRequest` (not just the bare `priority`) so a policy can read
`save_deadline`/`reason` — a `SaveWindow` policy otherwise has no way to know
its own deadline; (2) the continuous `Interruptibility` feed (previously modeled
only as the wire push `SupervisionClientMsg::InterruptibilityUpdate`) is now
named as part of the *same* trait a service implements, via a `watch::Receiver`
— chassis reads this feed and is what actually emits `InterruptibilityUpdate` on
change; the service never touches the wire message directly. `Yielded` (an
undefined placeholder type in the superseded shape) is replaced by the concrete
two-arm `Wound` enum.

Ladder behavior, unchanged in shape, restated against `RestartPolicy`: **L1
`WaitForIdle`** — no handler call; the `interruptibility()` feed IS the signal
(chassis relays it as `InterruptibilityUpdate`, supervision reads it — no
`on_restart` invocation happens at L1). **L2 `FinishAndRelinquish`** — chassis
calls `on_restart(L2, req)`, awaits, sends `RestartReply(Yielding)` immediately
and again on completion. **L3 `SaveWindow`** — chassis calls `on_restart(L3,
req)` under `req.save_deadline`; sends `RestartReply(Saved)` on completion **or**
yields anyway when the deadline elapses (best-effort save — chassis, not the
policy, enforces the deadline; see `chassis.md` concern 5's "never blocks the
daemon forever" correctness rule). **L4 `Kill`** — not delivered to the handler;
the process is killed; chassis offers only a best-effort `SIGTERM` hook, never a
`RestartPolicy` callback.

**The severity-in / service-decides-how doctrine (INTENT #156, folded here).**
"The service has to determine how it shuts itself down. Requests are requests."
The signal that crosses this wire is *only* the severity (`RestartPriority` +
`RestartRequest`) — supervision never prescribes *how* a service winds down at
L1–L3, only *that* it must, and by when. `supervision` (and, through it, mesh)
**REQUESTS**; it never **FORCES** compliance below L4 — a policy that overruns
its deadline or panics is not punished mid-ladder, it is simply superseded by the
daemon's own escalation (restart-protocol.md's "Error cases" `RestartTimeout`) or,
at the ceiling, killed outright (L4, the one level requiring no participation and
granting no latitude). *What* a given service reports as `Idle` vs
`CriticalSection`, and the concrete wind-down a policy performs, are governed by
a dedicated restart-interrupt-signal philosophy (INTENT #119, OQ-27) developed
via critic-pattern in a Substrate plugin — **NOT decided here**. Until that
philosophy lands, per-app `RestartPolicy` implementations (notably
`inference.md` concern 4's benchmark-priority-0-sweep-reports-`Idle` stance)
stand **applied provisionally, pending the restart philosophy** — the same
deferral `supervision.md` concern 3 records for the daemon side.

## Error cases

`SuperError` (`types::error::supervision`, matchable, never a panic — CAP honesty
INTENT #84):
- `IncompatibleVersion { slug, have, need }` — a pinned (`Node{N}`) resolve of a
  version outside a caller's pairwise requirement; catchable. `AnyNode` resolution
  prefers a compatible instance instead of raising this.
- `HandoffStalled { slug }` — new instance never became healthy within
  `handoff_deadline`; the old instance is kept, no flip.
- `RestartTimeout { slug, request_id }` — a graceful level missed its deadline;
  drives one-step escalation up the ladder, surfaced for observability.
- `ServiceUnreachable { slug }` — never acked / interruptibility feed stale past
  window → escalate toward L4 + zombie-sweep.
- `DependencyCycle { slugs }` — cyclic `requires` graph at boot-plan derivation.
- `CrashLooping { slug, count }` — crash-loop threshold hit; stops restarts,
  marks `Degraded`, escalates to cc.

Non-errors by design:
- A `Busy` reply **below** `SaveWindow` is a normal outcome (supervision waits /
  escalates), not an error; at `SaveWindow`/`Kill` a `Busy` reply is **ignored**.
- A service already `Idle` on an L1 request needs no push.
- Missing an L3 deadline is the protocol working (mesh escalates to L4), not a
  client error; a panicking `on_restart` is contained (the client yields to
  protect the ladder).

Daemon-unreachable / relay failures are `mesh-transport`'s (`PeerUnreachable`),
not `SuperError`.

## Version sensitivity

**MEDIUM.**
- **The 4-level ladder is LOCKED** — `RestartPriority` is a stable, closed enum.
  Adding or removing a level is **breaking** and requires an operator round.
  Newer daemons MUST NOT introduce a 5th level a client can't parse; if an
  unknown priority ever crossed the wire it MUST default to the **most
  conservative interpretation** (`SaveWindow` — save-and-yield), never ignore a
  restart signal.
- **Additive-safe:** `RestartReason` grows behind `#[serde(other)]`; new optional
  `#[serde(default)]` fields on `RestartRequest`/`PortHandoff`. `RestartRequest.v`
  anchors the wire.
- Restart frames carry the `types` guardrail-4 discipline (`serde(default)`, no
  `deny_unknown_fields`) because a daemon on one node may relay a control frame to
  a service reached through another during the skew window.
- **Coupled to the mixed-version transport floor:** a breaking `mesh-transport`
  `proto` bump (or an inbound `min_peer_version` floor, `mesh-transport` §1
  `Hello.min_peer_version`) is exactly what drives `RestartReason::Compatibility`
  restarts across the fleet — the mechanism by which the whole fleet rolls to a
  new transport floor (concern-9 v1 answer). A floored sender gets the wire-level
  `WireError{ code: "version_below_floor" }` / `MeshError::VersionBelowFloor`
  (`mesh-transport` §Error cases) plus a `PleaseUpdate`/`VersionAdvisory`
  back-compat warning — this is the **SETTLED** (ledger §A row 32, INTENT #113)
  machinery that makes a version floor *enforceable at the message level*, and
  what supervision's compatibility-restart decision (concern 9) *reacts to*, not
  what it designs. Binary delivery (INTENT #33) stays OPEN.
- **PENDING BLESSING, not settled (OQ-31, ledger §B.3):** the settled part is
  narrow — that the 4-level ladder + sender version stamping + version floors
  ARE the mixed-version mechanism (INTENT #113 "the conversation is moot").
  **Not settled:** concern-9's specific v1 protocol built on top of that —
  organic-newest-wins as the fleet's *default* desired version (vs. an
  operator-pinned alternative) and the no-coordinated-drain-all-then-flip
  choice — is a **designer position awaiting the operator's blessing**, not a
  locked decision. It is deliberately a **one-line-swappable placeholder** (the
  ledger §C global rule): the existing `version/pin/<slug>` operator-pin escape
  hatch is exactly that swap, needing no protocol change if newest-wins is
  un-blessed in favor of always-pinned.

## Reconciliation notes

`supervision`'s daemon-side view is authoritative; `mesh-client`'s client-half
sketch loses on three flagged points (all recorded, not dropped):

1. **Enum name.** mesh-client called the ladder `RestartLevel`; `types` /
   supervision call it **`RestartPriority`**. Adopt `RestartPriority` (types is
   the struct home) — one name, fleet-wide. *Losing name recorded.*

2. **SaveWindow deadline placement.** mesh-client modeled it as an in-variant
   field `SaveWindow { deadline: Duration }`; `types`/supervision carry it as
   `RestartRequest.save_deadline`. **Adopt the request-field shape** so the ladder
   enum stays a clean 4-arm LOCKED enum (matchability for level selection matters).
   *Losing shape recorded.*

3. **RestartReason openness.** mesh-client proposed `RestartReason::Other(String)`
   (freely extensible open arm). **Rejected** in favor of a closed set + reserved
   `#[serde(other)]` catch-all — an open `Other(String)` is not matchable for
   priority/handling selection, whereas a reserved catch-all gives forward
   tolerance without losing matchability. *Losing view: mesh-client wanted
   free-text reasons; the closed-set-plus-catch-all is the compromise.*

4. **Interruptibility richness.** mesh-client had `{ Idle, Busy { note } }`;
   supervision/`types` have `{ Idle, Interruptible, CriticalSection { until } }`.
   **Adopt the three-state version** — `CriticalSection { until }` lets
   supervision reason about *when* a service will become interruptible, and
   `Interruptible` (distinct from `Idle`) lets L1 be satisfied without forcing
   idleness. mesh-client's `Busy { note }` maps onto
   `CriticalSection { until: None }` with the note dropped (observability only).

5. **Reply shape.** mesh-client had a bare `Relinquished {}` confirmation;
   supervision has the richer `RestartResponse { Acknowledged | Busy | Yielding |
   Saved }`. **Adopt `RestartResponse`** — it distinguishes L2 `Yielding` from L3
   `Saved` and carries `Busy` back up. mesh-client's `Relinquished` corresponds to
   the terminal `Yielding`/`Saved` completion.

## Example data

One **compatibility roll**, cross-node, in the shared example world. Node
**macbook** hosts `inference@2.1.0` and `db@1.4.0`; a new `inference@3.0.0`
binary lands (proto major bump). Supervision on macbook: (1) port-handoff —
spawn `inference@3.0.0` on a new port, verify healthy, flip the registry
`inference` → new endpoint; (2) `db@1.4.0` declares `requires inference "^2"`,
now violated, and a compatible `db@2.0.0` is present → an L3 compatibility
restart.

Daemon → db (macbook):
```jsonc
{ "type": "Restart",
  "v": 1, "request_id": "r-9c2",
  "priority": "SaveWindow",
  "reason": "Compatibility",
  "save_deadline": { "secs": 10, "nanos": 0 },
  "requested_at": "2026-07-19T18:20:00Z" }
```

db → daemon, having flushed its outbox:
```jsonc
{ "type": "RestartReply", "reply": { "type": "Saved" } }
```

Meanwhile a caller resolving `inference` **AnyNode** on peer **pi** (still on
`inference@2.1.0`, requires-compatible) is routed to pi's instance — no error,
skew absorbed by routing. A *pinned* `Node{macbook}` resolve from a `"^2"`-
requiring caller during the window instead gets:
```jsonc
{ "SuperError": { "IncompatibleVersion":
  { "slug": "inference", "have": "3.0.0", "need": "^2" } } }
```

Port-handoff notice broadcast to the old inference copy:
```jsonc
{ "type": "PortHandoffNotice",
  "service": "inference",
  "old": { "host": "macbook", "port": 8081 },
  "new": { "host": "macbook", "port": 8093 },
  "at": "2026-07-19T18:19:58Z" }
```
