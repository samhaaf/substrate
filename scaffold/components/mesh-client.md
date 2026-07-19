# mesh-client

## Charter

`substrate-mesh-client` (`lib/mesh-client`) is the tiny, boring shared library
that **every service compiles in to reach its local mesh daemon on `:3649`** —
and nothing else. It owns exactly one thing: the **single persistent connection
to the local mesh process** and the thin, typed client half of the universal
service protocols that ride it — `register`/`deregister`/`resolve` (the
`service-lookup` seam), heartbeat/lease renewal, the standard WS pub/sub
publish/subscribe wrappers, restart-protocol participation (the LOCKED 4-level
ladder + interruptibility reporting, INTENT #76–77), and surface-schema
publication. It exists so a service that only needs to *find its neighbours and
be found* does not have to link the whole `lib/mesh` tree (tailscale + router +
axum + the registry/queues/locks/cron internals). It is the cheap handle to the
wiring seam, not the seam itself.

**What it does NOT own.** It owns no registry state (that is
`service-registry`, server-side), no relay/routing (that is mesh-core /
`pubsub-relay` / `completion-router`), no queue/lock/cron *semantics* (those
live in the L2 mesh libs; the client only carries their request frames — see
"carried, not owned" below), no application data, and no addressing decisions
(which slug maps where is the registry's call). It holds **one socket and a
handful of typed wrappers over it** — scope creep here is scope creep in every
app crate, so the discipline is: if a capability's *meaning* lives in another
module, mesh-client only ferries its typed envelope, it does not learn its
domain.

## Primary design concerns

This module earned its own component not because any one method is hard, but
because it is **the one dependency every app crate takes** and it is the sole
place four cross-cutting client behaviours must be implemented *once, correctly*
(otherwise every service reinvents reconnect/re-announce and they drift).

1. **Single-port locality shapes everything (INTENT #58).** A service talks
   ONLY to its local daemon; it is "not even aware that there are other
   services running on other ports." So mesh-client holds **one** connection —
   `ws://127.0.0.1:3649` — and *multiplexes every interaction over it*:
   request/response RPC, pub/sub, registration, heartbeat, restart signals,
   surface-schema publication. Everything-over-WebSockets (INTENT #28/#53) plus
   the two-way restart protocol (mesh must be able to *push* a restart signal to
   the service) together force a **persistent bidirectional WebSocket**, not a
   per-call HTTP client — the round-9 `mesh.md` "Concern 0" HTTP sketch is
   superseded here for exactly this reason (an HTTP client cannot receive an
   unsolicited restart push).

2. **Slug-addressed relay, not endpoint-dialing — the non-obvious call.**
   `resolve(slug) -> Endpoint` returns a *browsable* `{scheme,host,port,
   health_path}` (so `mesh service open`, health checks, and the dashboard
   work), but under single-port locality **a service must NOT dial that
   endpoint directly** — that would make it aware of other ports, violating
   #58. The standing inter-service call path is therefore
   **`request(Address, method, payload)`**: the client frames a request
   *addressed by slug* and hands it to the local daemon, which relays it (to
   this node or another). `resolve()`'s Endpoint is for humans, health, and
   the daemon's own routing — not the RPC hot path. This split is the single
   most important thing a reviewer checks, and it is flagged to the operator
   below because `service-lookup`'s stub still reads "`resolve -> endpoint`" as
   if the caller dials it.

3. **Two addressing classes are a client-surface concern (INTENT #59).**
   `Address::Anywhere(slug)` (virtualized — the local daemon picks/relays to any
   live instance) vs `Address::OnNode(slug, node_id)` (pinned). Both are
   first-class on every `resolve`/`request`. The client does not *choose* — it
   passes the class through to the daemon.

4. **Connection-loss behaviour: local daemon down = fully isolated, but
   transient.** The local daemon is a service's only door, so losing it means
   losing the whole mesh. Because mesh is *sticky* (launchd/systemd resurrects
   it — `mesh.md` concern 8), the outage is short-lived. The client's contract:
   - A background **reconnect loop with bounded exponential backoff**
     (e.g. 100 ms → 5 s cap; NO unbounded growth).
   - An **observable `ConnectionState`** (a `tokio::sync::watch`) the embedding
     service reads to gate its own behaviour (e.g. degrade, pause accepting
     work) — the mirror of how the service reports interruptibility upward.
   - **Fail-fast RPC** while disconnected: `request()` awaits reconnect only up
     to a short grace deadline, then returns
     `MeshError::LocalDaemonUnreachable` — callers are never blocked forever.
   - A **bounded** outbound buffer for fire-and-forget publishes only
     (drop-oldest with a dropped-count metric; explicitly no unbounded queue —
     no technical debt, INTENT #38).
   - **Re-announce on reconnect is the load-bearing behaviour.** The client
     holds the service's full *desired mesh state* — its registration, its
     surface schema, its active subscriptions, its current interruptibility —
     and **replays all of it on every reconnect**. Registration is idempotent
     (LWW-register keyed by slug; a fresh `version` wins), subscriptions are
     re-sent, the surface schema re-published. A service author writes their
     announce once; the client keeps it true across every daemon bounce.

5. **Lease lifecycle the service never has to think about.** On connect the
   client registers, receives a lease TTL, and runs a **heartbeat task** that
   renews before expiry. On graceful shutdown it deregisters (server tombstones
   the slug). On crash the lease simply expires → tombstone → self-heal. If the
   client is disconnected past its TTL the entry tombstones; the reconnect
   re-register revives it under a new version. The service sees none of this.

6. **Restart-protocol participation is a callback contract (INTENT #76–77,
   ladder LOCKED at 4 levels).** The embedding service supplies, at
   construction, (a) a way to keep reporting **interruptibility** (`Idle` /
   `Busy`) and (b) an `on_restart(level)` async handler. The client maps the
   ladder: L1 *wait-for-idle* is satisfied purely by the interruptibility feed
   (the client reports `Idle` when work drains, mesh acts); L2
   *finish-and-relinquish* invokes the handler and awaits its `relinquish()`;
   L3 *~10 s save window* invokes the handler under a hard deadline then yields
   regardless; L4 *kill* has no client participation (the process is killed —
   at most a best-effort SIGTERM hook). Port-handoff choreography (new port →
   registry flip → old down) is mesh-core's; the client's only part is
   register-on-boot (which *is* the LWW registry flip) and yield-on-signal.

7. **Wire version-skew is real and LOCAL (INTENT #66).** Although mesh-client is
   compiled in and takes *no runtime version tracking* as a library (INTENT
   #45), it talks over the wire to the **mesh daemon, a separate process that
   revs independently** under minimal-restart rolling updates. So the daemon a
   service connects to may be older or newer than the client compiled into that
   service. The multiplexed envelope must therefore carry a `protocol_version`
   and be **additive-only / tolerant of unknown frame kinds** (ignore-unknown,
   reserved space). This is the one genuinely version-sensitive surface in an
   otherwise compiled-in library, and it is easy to miss precisely because
   "it's a lib, libs don't version."

## Relationships / edges

mesh-client is the **universal client half** of the cross-cutting "every
service ↔ mesh" protocols; the server halves live in mesh-core and its L2 libs.
Per-pair reconciliation happens in the contract round — this file proposes the
client side only.

- mesh.service-registry via `service-lookup` — register / deregister / resolve
  / heartbeat; the wiring seam
  (see scaffold/contracts/service-lookup.md)
- every service ↔ mesh via `restart-protocol` — client half of the 4-level
  graceful-restart ladder + interruptibility reporting **(MISSING stub —
  proposed below)**
- every service ↔ mesh via `pubsub-protocol` — the multiplexed WS envelope +
  publish/subscribe wrappers (client half of the standard pub/sub protocol)
  **(MISSING stub — proposed below)**
- every service → mesh via `surface-schema` — publishes this service's boring
  surface schema and keeps it fresh across reconnects
  (see scaffold/contracts/surface-schema.md)
- **Carried, not owned:** `queues-api`, `locks-api`, `cron-api` ride the generic
  `Request` frame of `pubsub-protocol`; mesh-client serializes/relays their
  typed envelopes (from `types`) but implements none of their semantics. Those
  contracts belong to `queues` / `locks` / `cron`. Deliberately excluded from
  mesh-client's owned surface to keep it tiny.
- Imports `substrate-types` (the envelope, event, surface-schema, error, id
  vocabulary) — a shared-lib dependency, NOT a contract edge (locked
  rounds 4–5).

## Nesting

Parent: (top-level shared-lib) | Children: none.

mesh-client is a **shared library** (`lib/mesh-client`), not a child of mesh —
it is the *client* counterpart that services link instead of `lib/mesh`. It sits
in the mesh kernel layer (L1) conceptually but is a standalone crate any app
crate may compile in. It depends only on `substrate-types` + a WS/runtime stack
(`tokio`, `tokio-tungstenite`, `serde`, `serde_json`, `futures`) — deliberately
NOT on `lib/mesh`, which is the entire reason it exists.

## Thoroughness level

**implementation-ready** — for the client's own mechanics: the connection model
(one persistent local WS), reconnect + re-announce, lease/heartbeat, the two
addressing classes, the restart-callback contract, the error taxonomy, and
connection-loss behaviour are all fully specified above and below. **One input
is not yet frozen:** the exact `pubsub-protocol` **envelope struct**, which is
co-owned with `pubsub-relay` (server side) and `types` (the struct's home) and
is `requirements-only` there. mesh-client's logic is written *parametric* on
that envelope, so it is not a design gap in this module — but the crate cannot
be filled until the envelope is harmonized. Sequence accordingly.

## Assigned design-depth

Opus, single Component-Designer pass (this file), grounded on `mesh.md`
(concerns 0, 1, 7–13), `service-registry.md`, the `service-lookup` /
`surface-schema` contract stubs, INTENT #44–107 (esp. #45, #53, #58, #59, #66,
#76, #77), and the live `lib/mesh` / `lib/inference` code (which today has *no*
self-registration path — confirming mesh-client is genuinely new, not an
extraction of existing duplicated logic).

## Suggested fill-model

**implementation-ready + low complexity → cheap/fast model OK**, with one hard
sequencing constraint: fill mesh-client **after** the Contract Harmonizer has
frozen the `pubsub-protocol` envelope and the `types` pubsub/event/surface
modules, since the whole client serializes those structs. Given that ordering,
the reconnect/re-announce/heartbeat/restart logic is mechanical enough for a
cheap model — the design has fully bought down the fill risk *except* for the
upstream-envelope dependency, which is a scheduling constraint, not a
model-strength one.

---

## Proposed contracts (wave 2)

Client-half proposals only. Server halves (`service-registry`, `pubsub-relay`,
mesh-core supervision) propose their sides; a later per-pair round reconciles.
All structs are expressed in `substrate-types` vocabulary; Rust-flavoured
pseudocode.

### `service-lookup` (client half)

**Purpose.** Let a service register itself under a slug (with a lease it keeps
renewed) and resolve others by slug, in either addressing class. THE wiring
seam.

**Sketch.**
```rust
// Endpoint is browsable, not a dial target under single-port locality.
struct Endpoint { scheme: Scheme, host: String, port: u16, health_path: Option<String> }
enum RegKind { Singleton }        // the `inference` fleet front-door is mesh's own,
                                  // never set by a client (see service-registry.md)
enum Address { Anywhere(Slug), OnNode(Slug, NodeId) }

// client -> local daemon (correlated requests over the one socket)
struct Register   { slug: Slug, endpoint: Endpoint, kind: RegKind,
                    lease_ttl: Duration, surface: Option<SurfaceSchema> }
struct RegisterAck{ lease_id: LeaseId, lease_expires_at: Timestamp, renew_before: Duration }
struct Heartbeat  { lease_id: LeaseId }                 // periodic, < renew_before
struct Deregister { slug: Slug, lease_id: LeaseId }     // graceful shutdown -> tombstone
struct Resolve    { addr: Address }
struct ResolveResp{ endpoint: Option<Endpoint> }        // None = no *live-leased* entry
struct ResolveAll {}                                    // cheap full snapshot (Org resolves the whole map)
struct ResolveAllResp { entries: Vec<(Slug, Endpoint, NodeId)> }
```
**Error cases.** `LocalDaemonUnreachable` (the door is down — reconnecting);
`LeaseExpired` (heartbeat arrived after TTL — client must re-register, handled
automatically on reconnect); `Resolve -> None` (no live owner; NOT an error —
an empty result, so a pinned resolve of a down node reads cleanly);
`SlugRejected` (registry refused the slug — reserved/malformed). No
`SlugConflict`: LWW means the newest `version` simply wins, by design.
**Version-sensitivity.** `Endpoint` and `RegKind` must grow additively (a newer
daemon may return a `RegKind` the client doesn't know — treat unknown as opaque,
never panic). `renew_before` is daemon-advertised so lease timing can change
without a client rebuild.

### `restart-protocol` (client half) — **proposed new stub**

**Purpose.** The client side of the two-way, LOCKED 4-level graceful-restart
ladder (INTENT #77) plus continuous interruptibility reporting (INTENT #76).
Built into every service from the beginning via mesh-client so no service hand-
rolls it.

**Sketch.**
```rust
// client -> daemon: continuous, whenever it changes
enum Interruptibility { Idle, Busy { note: Option<String> } }
struct InterruptibilityUpdate { state: Interruptibility }

// daemon -> client: unsolicited push (the reason a persistent WS is required)
enum RestartLevel {
    WaitForIdle,                       // L1: mesh just waits; satisfied by the Idle feed
    FinishAndRelinquish,               // L2: finish current work, then relinquish()
    SaveWindow { deadline: Duration }, // L3: ~10s to save, then yield regardless
    Kill,                              // L4: no participation (process killed)
}
enum RestartReason { Compatibility, Update, OperatorRequested, PortHandoff, Other(String) }
struct RestartSignal { level: RestartLevel, reason: RestartReason }

// client -> daemon: yield confirmation (L2/L3)
struct Relinquished {}

// service-supplied at construction:
trait RestartParticipant {
    async fn on_restart(&self, level: RestartLevel) -> Yielded; // resolves when the service has yielded
}
```
**Client behaviour.** L1 needs no handler call (the `Idle` feed is the signal);
L2 calls `on_restart` and awaits, then sends `Relinquished`; L3 calls
`on_restart` under `deadline`, sends `Relinquished` on completion **or** yields
anyway when the deadline elapses (best-effort save); L4 is not delivered to the
handler. **Compatibility-driven restarts arrive as high-priority levels.**
**Error cases.** Missing an L3 deadline is *not* a client error — it is the
protocol working (mesh escalates to L4). `on_restart` panicking is contained
(the client yields to protect the ladder). **Version-sensitivity.** The ladder
is LOCKED at four levels, but a *newer* daemon could introduce a level the
client doesn't recognise; unknown levels MUST default to the **most
conservative** interpretation (save-and-yield, i.e. treat like L3) — never
ignore a restart signal. `RestartReason` is freely extensible (`Other`).

### `pubsub-protocol` (client half) — **proposed new stub**

**Purpose.** The multiplexed local-WS **envelope** every frame rides, plus the
publish/subscribe wrappers and per-completion-ID subscription (INTENT #5). This
is the substrate under *all* the other client protocols; mesh-client is the
canonical client party. The envelope struct itself is co-owned with
`pubsub-relay` + `types` — this proposes the **client-side framing, correlation,
and reconnect-replay** view of it.

**Sketch.**
```rust
struct Envelope { protocol_version: u16, frame: Frame }

enum Frame {
    // generic RPC — the standing inter-service call path (slug-addressed relay)
    Request  { corr_id: CorrId, target: Address, method: String, payload: Json },
    Response { corr_id: CorrId, result: Result<Json, WireError> },
    // pub/sub — typed events, mesh relays where they need to go
    Publish   { topic: Topic, event: TypedEvent },
    Deliver   { topic: Topic, event: TypedEvent, origin_node: NodeId },
    Subscribe { topic: Topic }, Unsubscribe { topic: Topic },
    // the other client protocols multiplex here as typed frames:
    Register(..), Heartbeat(..), Deregister(..), Resolve(..),
    InterruptibilityUpdate(..), RestartSignal(..), Relinquished(..),
    PublishSurface(..),
    // reserved: unknown frames from a newer daemon are ignored, not fatal
}
enum Topic { Named(String), Completion(CompletionId) } // per-completion-ID subscription
```
**Client behaviour.** One correlation-ID map for in-flight `Request`s; one
subscription table replayed on reconnect; bounded outbound buffer for
`Publish`; delivery fan-out to per-topic subscriber channels. **Error cases.**
`WireError::NoLiveEndpoint` (relay target has no live-leased instance),
`WireError::RelayFailed`, `WireError::UnknownMethod`, `WireError::PayloadDecode`;
transport drop → reconnect (surfaced as `ConnectionState::Reconnecting`, not per
call). **Version-sensitivity.** THE version-critical surface (concern 7):
`protocol_version` is mandatory; frame set is additive-only; **unknown frame
kinds and unknown fields are ignored, never fatal**, so a service and its local
daemon can rev independently. Defer the canonical envelope shape to `types` +
`pubsub-relay`; reconcile the client multiplexing view against them in the pair
round.

### `surface-schema` (client half)

**Purpose.** Publish this service's boring surface schema (how to render its
dashboard component + what calls to make against it) to mesh, and keep it fresh
— mesh serves it to the dashboard (INTENT #46). Client side is a pass-through.

**Sketch.**
```rust
struct PublishSurface { schema: SurfaceSchema } // SurfaceSchema lives in `types` (requirements-only there)
// convenience: schema may be supplied at Register (see service-lookup) and
// updated live via publish_surface(schema); the client re-publishes on every reconnect.
```
**Error cases.** Best-effort — a rejected/failed publish degrades observability
only, never the service's function; the client retries on reconnect. A
malformed schema is a compile-time concern (typed in `types`), not a runtime
error class here. **Version-sensitivity.** Low — the client is a pure carrier of
whatever `types::surface::SurfaceSchema` is; the schema language is versioned in
`types`, and the dashboard renders defensively from it. mesh-client adds no
version surface of its own beyond the enclosing `protocol_version`.
