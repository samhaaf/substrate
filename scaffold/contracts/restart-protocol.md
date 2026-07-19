# Contract: restart-protocol

## Parties

- **mesh** (`supervision`, the daemon side — authoritative view) — signals restarts
- **every service** (via `mesh-client`, the client half) — reports interruptibility, yields

Cross-cutting, surface-schema-style: ONE shared document, every service is a
party (wave2-plan §5.4). Struct vocabulary is homed in `types::restart` +
`types::error::supervision`; the daemon behavior is `supervision`'s, the client
behavior is `mesh-client`'s.

## Purpose

The two-way graceful-restart / supervision choreography built into EVERY service
from the beginning (INTENT #76 continuous interruptibility, #77 the LOCKED
4-level ladder). The service continuously reports interruptibility; mesh signals
a restart need at a laddered priority and drives port-handoff + kill. The
protocol rides the `mesh-client` persistent bidirectional WS (mandatory — mesh
must *push* an unsolicited restart signal) and multiplexes over the
`pubsub-protocol` transport as typed control frames. Restart is **primarily
node-local** (a service is supervised by its OWN local daemon), but a control
frame may be relayed cross-node during a skew window.

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

### Service → daemon (`SupervisionClientMsg`, client half via mesh-client)

```rust
enum SupervisionClientMsg {
    InterruptibilityUpdate { state: Interruptibility }, // continuous, on change
    RestartReply(RestartResponse),                      // Acknowledged | Busy | Yielding | Saved
    Manifest(ServiceManifest),                          // version + `requires`, at registration
}
```

### Client-side handler contract (mesh-client, in-process)

```rust
trait RestartParticipant {
    async fn on_restart(&self, priority: RestartPriority) -> Yielded;
}
```
Ladder behavior: **L1 `WaitForIdle`** — no handler call; the `Idle` feed IS the
signal. **L2 `FinishAndRelinquish`** — call `on_restart`, await, reply `Yielding`
then (on completion) proceed to yield. **L3 `SaveWindow`** — call `on_restart`
under `save_deadline`; reply `Saved` on completion **or** yield anyway when the
deadline elapses (best-effort save). **L4 `Kill`** — not delivered to the
handler; the process is killed.

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
  marks `Degraded`, escalates to ccd.

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
- **Coupled to the mixed-version transport floor:** a breaking `pubsub-protocol` /
  `mesh-transport` `proto` bump is exactly what drives `RestartReason::Compatibility`
  restarts across the fleet — the mechanism by which the whole fleet rolls to a new
  transport floor (concern-9 v1 answer). Binary delivery (INTENT #33) stays OPEN.

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
