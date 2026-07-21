# pubsub-relay

**Status:** NEW (wave 2). **Nesting:** internal lib of mesh (module
`lib/mesh::pubsub`). **Prior art:** `bin/gateway/src/{topics,hub,ws,upstream}.rs`
(the V1 gateway pub/sub, folded into mesh at the 2026-07-18 gateway merge). This
file designs the wire shape that was left `requirements-only` in `mesh.md`
concern 7 (LOCKED rounds 4–5) and INTENT #53.

## Charter

`pubsub-relay` is the standard WebSocket publish/subscribe transport buried
inside the mesh daemon: the one boring protocol by which any service publishes a
typed message onto a topic and any subscriber — on the same node or a different
node — receives it, with the mesh doing all the relaying (single-port locality,
INTENT #58). It owns the **envelope header, the topic taxonomy, subscription
matching, the local fan-out broker, and the cross-node relay path**. It is
deliberately **lossy and best-effort**: it favors liveness over completeness and
drops for slow subscribers rather than blocking publishers. It does **NOT** own
durability, ordering guarantees, at-least-once delivery, or exactly-once
de-duplication — that is `queues`' job (INTENT #89/#95); the two are separate
mesh libs and this boundary is load-bearing (see concern 8). It does **NOT**
define the *payload* types (those are `types`' event/domain structs, compiled
into publishers and subscribers) — the relay carries the payload **opaque**
(concern 2). It is **NOT** the byte-transparent completion proxy: forwarding the
raw OpenAI-compatible `/v1/completions/:id/stream` bytes is `completion-router`'s
`forward()` path; per-completion typed *events* (for dashboards/observability)
ride here as a per-entity topic (concern 4). It is an internal library, never a
standalone service (INTENT #54).

## Primary design concerns

### 1. Envelope — a parsed header over an opaque payload

Every message on the bus is an `Envelope`. The relay parses and acts on the
**header** only; the **payload is carried opaque** (see concern 2). The header
carries first-order provenance (INTENT #85) because pub/sub is a place where
data is touched and relayed:

```rust
// proposed to `types` (module `pubsub.rs`); authored in scaffold/contracts/pubsub-protocol.md.
struct Envelope {
    msg_id: Uuid,            // unique per publish; subscriber-side dedup key
    topic: Topic,            // routing key (concern 3)
    origin: Provenance,      // WHO/WHERE/WHEN — daemon-stamped, not client-trusted
    causal_parent: Option<Uuid>, // msg_id or handler-invocation id that caused this
    event_type: EventTypeTag,    // string tag naming the payload struct in `types`
    payload: RawPayload,     // opaque bytes (serialized event); relay never parses
}

struct Provenance {
    service: Slug,           // publishing service slug (verified vs registration)
    node_id: NodeId,         // stamped by the LOCAL daemon — a service can't spoof it
    ts_millis: i64,          // daemon wall-clock at publish
    seq: u64,                // per-(service,node) monotonic counter — gap detection
}
```

**The daemon stamps `origin.node_id`, `ts_millis`, `seq`, and `msg_id`; the
publisher supplies only `topic`, `event_type`, `causal_parent`, and the payload.**
This keeps provenance trustworthy (a service cannot claim to be on a node it is
not) and gives subscribers a monotonic `seq` per publisher to detect gaps left by
the lossy policy (concern 6). `causal_parent` is what lets the execution-engine's
causal-chain / loop-depth tracking (INTENT #70, #101) stitch a publish back to the
handler invocation that emitted it — first-order provenance across the bus, not
just within a database.

### 2. Payload-agnostic relay (the version-decoupling move)

The relay routes by the envelope **header** and never deserializes `payload`.
Consequences, all deliberate:

- A **new event type** deployed on node A relays cleanly through an **older mesh
  daemon** on node B to a new-enough subscriber on node C — the middle daemon
  doesn't need to know the type. This is what makes the pub/sub wire survive
  mixed-version fleets (the OPEN mixed-version-update problem in `supervision`,
  INTENT #66) without a flag day.
- Publishers and subscribers share the concrete typed struct by **compiling in
  `types`** (a shared lib — no runtime version tracking, INTENT #45). The relay
  in the middle links `types` too but only for the *header* structs, not the
  payload variants.
- `event_type` is a plain string tag (e.g. `"inference.completion.token"`) so the
  subscriber can pick the right `serde` target; an unknown tag is the subscriber's
  problem to skip, never the relay's.

This is the single most important structural decision in the module: **the relay
is dumb about payloads on purpose.**

### 3. Topic taxonomy — hierarchical path + node scope

The V1 gateway `Topic` enum (`Lifecycle | Queue | Gc | System | Completion(id) |
All`) was a closed set; V2 needs it open (new services must add topics without
editing this lib) and node-scoped (the two addressing classes, concern 5).
Proposed:

```rust
struct Topic {
    scope: Scope,      // Fleet | Node(NodeId)  -- concern 5
    path: TopicPath,   // dot-segmented, hierarchical: "inference.completion.token"
}
enum Scope { Fleet, Node(NodeId) }
```

`TopicPath` is a validated, dot-segmented string (`service.domain.kind...`),
lower-snake segments, so the taxonomy lives in **data**, not in a Rust enum every
new service would have to extend. Reserved top-level segments seed the migration
of the existing surfaces:

| Path prefix | Replaces (V1 surface) | Owning module |
|-------------|-----------------------|---------------|
| `inference.*` | `inference-events`, per-completion streams | inference/api |
| `gc.*` | `gc-events` | gc |
| `cc.*` | `cc-events` | cc |
| `network.*` | `network-events` | network-topology |
| `queue.*` | queue state changes | queues |
| `dashboard.*` | the browser fan-out feed | dashboard-serving |
| `<slug>.*` | any future service's own events | that service |

Per-entity topics are just a leaf: a per-completion stream is
`inference.completion.<completion_id>` (the round-8 `Completion(id)` case
generalized into the path). The operator's explicit "per-completion-ID
subscription" (INTENT #5) is therefore a plain exact-match subscription, not a
special case in code.

### 4. Subscription semantics — all / filtered / per-entity

A subscription is a **set of `TopicFilter`s**; a message matches if ANY filter
matches. Three filter kinds cover the operator's three named cases:

```rust
enum TopicFilter {
    All,                              // "all": every topic, any scope (dashboard/debug)
    Exact(Topic),                     // "per-entity": e.g. inference.completion.<id>
    Prefix { scope: ScopeFilter, prefix: Vec<Segment> }, // "filtered": inference.*
}
enum ScopeFilter { Any, Fleet, Node(NodeId) }
```

`Prefix` matches on **segment boundaries** (so `inference` matches
`inference.completion.token` but not `inference_x.*`) — the boring, unambiguous
rule. `All` is the dashboard's firehose (INTENT #46 boring-surface rendering
subscribes broadly, then filters client-side). **Snapshot-on-connect is NOT this
module's job** — publishers that need it (network-topology's snapshot-then-delta,
INTENT-driven) implement it by replaying current state as normal publishes to the
new subscriber's topic; the relay provides only the transport. (Rationale: a
retained-message/last-value store is durability-flavored and belongs out of the
lossy transport — keeping it out is what keeps this lib boring.)

### 5. The two addressing classes map onto `Scope`

INTENT #59's two classes are the `Topic.scope` field, end to end:

- **Virtualized ("service X, any node"):** subscribe with `ScopeFilter::Any` (or
  `Fleet`). Interest propagates to **all** peer daemons (concern 7); you receive
  matching publishes wherever they originate. Publishing to the fleet = a publish
  with `Scope::Fleet`.
- **Pinned ("service X on node N"):** subscribe with `ScopeFilter::Node(N)`.
  Interest is registered with **only N's daemon**; only publishes stamped
  `origin.node_id == N` (or explicitly `Scope::Node(N)`) reach you. The scope is
  what bounds the relay — you don't receive-then-filter, you never get the other
  nodes' traffic at all.

This is the clean mapping the operator asked to "design in" (mesh.md concern 9):
the addressing class is not a separate API, it is one field on the topic.

### 6. Backpressure / lagging-subscriber policy — lossy by contract

Reuse and formalize the V1 gateway model (`hub.rs`: bounded `broadcast` channel,
`RecvError::Lagged`):

- **Local fan-out:** each subscriber has a bounded per-connection queue. On
  overflow the subscriber receives a `Lagged { dropped: u64, since_seq: u64 }`
  control frame and stays connected — it missed those events, by design. Fast
  publishers are never blocked by a slow subscriber.
- **Cross-node relay link:** each daemon holds a bounded per-peer send buffer on
  its outbound relay link. If a peer's link can't keep up, the origin daemon drops
  and increments a per-peer drop counter (surfaced as a `network.*` health event),
  rather than growing memory unbounded or stalling local delivery.
- **Gap detection:** the `seq` per publisher (concern 1) lets any subscriber that
  cares notice it missed messages and re-sync out of band (e.g. re-query the
  service's REST surface / surface-schema). The relay itself never retries.

The honesty this buys: **pub/sub can drop and can (rarely) duplicate; it never
blocks and never guarantees.** Anything needing delivery guarantees uses `queues`
(concern 8). This matches inference's existing "lossy broadcast event bus"
(inference.md).

### 7. Cross-node relay — one hop, interest-routed, loop-free

The hard part: a publish on node A must reach a subscriber on node B through their
respective mesh daemons, with single-port locality (services only ever touch their
local `:3649`).

- **Local broker per daemon.** Each mesh daemon runs an in-process broker (the V1
  `Hub`, generalized). A local publish fans out to local matching subscribers
  immediately and unconditionally.
- **Interest table, not flooding.** Each daemon maintains, per peer, the set of
  **topic filters that peer currently has local subscribers for** (aggregated —
  the union, coarsened to prefixes so the table stays small). When a daemon's own
  local subscription set changes, it gossips an `InterestUpdate { node, epoch,
  filters }` to peers over a persistent daemon↔daemon control link. On (re)connect
  a daemon sends a full interest **snapshot** (snapshot-then-delta, mirroring
  network-topology and the registry's anti-entropy discipline). Interest is
  **volatile/live state, deliberately NOT in replicated-kv** — it must track
  connectivity reactively, and KV anti-entropy is periodic/slow; interest follows
  the link, and is rebuilt on reconnect.
- **Forward to interested peers only.** On publish, the origin daemon consults the
  interest table and forwards the envelope over the direct relay link to **exactly
  the peer daemons that have matching interest** (for `Scope::Node(N)` it forwards
  only to N, if N is a peer at all). Every node has a direct Tailscale path to
  every other, so relay is always **exactly one hop**: origin daemon → interested
  peer daemon → that peer's local subscribers.
- **Loop-free by construction.** A relayed envelope is marked `relayed` on the
  link frame; a daemon that receives a relayed frame fans it out to LOCAL
  subscribers only and **never re-relays it**. Single-hop + no-re-relay means no
  forwarding loops are possible without any hop counter or path set. (A
  multi-hop/store-and-forward design was considered and rejected: unnecessary at
  personal-mesh scale, and it would reintroduce loop risk this avoids.)
- **Offline peers:** interest and buffered frames are simply lost while a peer is
  unreachable; on reconnect, interest re-snapshots and delivery resumes — no
  backfill (lossy contract, concern 6). Detecting peer reachability rides
  `network-topology` / the registry peer set, not a bespoke membership protocol.

### 8. The queues boundary (why this is a separate lib)

`pubsub-relay` and `queues` (INTENT #89/#95/#101) both move typed events and both
share the `types` event struct, so it is worth stating why they are distinct libs
and where the line is:

- **pub/sub** = ephemeral, best-effort, fan-out-to-current-subscribers,
  observation-shaped (dashboards, topology, live streams). No storage, no
  semaphore, no DLQ.
- **queues** = durable, at-least-once (→ ~exactly-once via `locks` event-ID
  semaphores), consumer-pull, trigger/handler-shaped, with dead-letter escalation.

A dashboard subscribes over pub/sub. A trigger consumes from a queue. They can
carry the *same* standardized `Event` struct from `types`, but they are not the
same delivery mechanism and neither is built on the other. (Open nudge for the
`queues` design pass: whether a queue *optionally mirrors* its state-change events
onto a `queue.*` pub/sub topic for observability — a one-way tee, not a
dependency. Flagged, not decided here.)

## Relationships / edges

- every service ↔ mesh via `pubsub-protocol` — the standard WS pub/sub envelope +
  subscribe/publish protocol; cross-cutting, surface-schema-style (one shared
  document, every service a party). *(authored: scaffold/contracts/pubsub-protocol.md)*
- `types` — **library dependency, not a contract edge** (INTENT rounds 4–5): the
  `Envelope` / `Topic` / `Provenance` header structs and the standardized `Event`
  payload struct live in `types` (module `pubsub.rs` + `event.rs`), compiled into
  every party. I propose their shape below for the concurrent `types` designer.
- **Convergence (harmonization-time, owned by other modules):** the existing WS
  surfaces `network-events`, `dashboard-feed`, `inference-events`, `gc-events`,
  `cc-events` are all expected to be re-expressed AS `pubsub-protocol` topics
  (`network.*`, `dashboard.*`, `inference.*`, `gc.*`, `cc.*`). I do NOT author
  those contracts (they belong to network-topology, dashboard-serving, inference,
  gc, cc) — I only claim the topic-prefix taxonomy they land on (concern 3), for
  the Contract Harmonizer to reconcile.
- `network-topology` / `service-registry` — consumed in-process for the peer set
  the relay links to (not a contract edge; sibling mesh libs).

## Nesting

Parent: mesh | Children: none. Module `lib/mesh::pubsub` (server/broker side) +
the daemon↔daemon relay link. The client half (a service's
subscribe/publish handle to its local daemon) is part of `mesh-client`'s surface,
not a separate crate — services get pub/sub through the same thin boot lib they
get `register`/`resolve` from (mesh.md concern 0). Confirmed at skeleton time.

## Thoroughness level

**implementation-ready** — envelope header, opaque-payload relay, topic taxonomy,
the three subscription filter kinds, the two-addressing-class → scope mapping, the
lossy backpressure policy, and the one-hop interest-routed loop-free relay are all
decided and grounded in the live V1 gateway code. What remains open is genuinely
downstream: (a) the exact interest-coarsening heuristic (how aggressively to
collapse many exact filters into prefixes) — a fill-time tuning knob, not a design
fork; (b) reconciling the five existing `*-events` contracts onto the topic
prefixes — Contract Harmonizer work; (c) the `types` `pubsub.rs`/`event.rs` field
sets — authored, reconciled in the per-pair round with the concurrent
`types` designer.

## Assigned design-depth

Single strong-model (Opus) Component-Designer pass (this file), grounded on
`bin/gateway/src/{topics,hub,ws,upstream}.rs` and mesh.md concerns 5/7/9.

## Suggested fill-model

implementation-ready + moderate complexity → **mid model**. The local broker is a
near-transcription of the V1 `Hub`/`ws.rs` (broadcast channel + per-client
subscription set + lagged handling). The genuinely new code is the daemon↔daemon
interest-routed relay link (concern 7); the design fixes its shape (one-hop,
interest snapshot-then-delta, `relayed` marker, per-peer bounded buffer), so a mid
model can implement it against this spec without a Design Mesh. Do not send to the
cheapest tier only because of the relay's cross-node concurrency.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them.

- `pubsub-protocol` (every service via `mesh-client` ↔ mesh's pubsub-relay;
  cross-cutting, ONE shared surface-schema-style document) — the WS
  publish/subscribe envelope, scoped topics + filters, `RelayFrame`
  daemon↔daemon facet, `PubSubError` taxonomy, and the best-effort/lossy +
  payload-opaque guarantees. → `scaffold/contracts/pubsub-protocol.md`
  - Contract resolution: **pubsub-relay's relay-authoritative shape won**
    (opaque payload + header `event_type`, scoped `Topic`,
    connection-as-subscriber, this module's error taxonomy); `types`' first-cut
    `pubsub.rs` sketch is superseded. Normalizations against this module's old
    proposal: `msg_id`→`envelope_id` plus an `Envelope.v` anchor;
    `origin`→`provenance` with the merged canonical
    `types::provenance::Provenance` (`ts_millis`→`emitted_at`, plus
    `correlation_id`/`causation_id` shared with queues/cron).
  - mesh-client's colliding multiplexing `Envelope` was renamed to the
    `mesh-transport` `Frame` (Reconciliation note 6); pub/sub messages are one
    frame kind inside it.
  - Still pending at harmonization (contract note 8): `network-events` /
    `dashboard-feed` / `inference-events` / `gc-events` / `cc-events`
    re-express as topic prefixes on this envelope — this module claims only the
    topic-prefix taxonomy (concern 3), not those contracts.
  - The retained-snapshot-per-topic capability `network-events` asks of the
    relay is the one open design ask pushed to this pair.
