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
mesh libs and this boundary is load-bearing (see concern 8). As of wave 3
(INTENT #155) lossiness is **configurable per message**: the broker core stays
lossy, but a `SaveFailed` envelope tees its would-be-dropped deliveries into a
*separable* intermediate-response cache and re-offers them on reconnect
(concern 9) — durability remains a delegation to a swappable seam, never a
property of the broker itself. It does **NOT**
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

### 9. Delivery persistence — the required `delivery` field: LossyDrop vs SaveFailed (INTENT #155)

INTENT #155 makes pub/sub lossiness **configurable per message**: every published
`Envelope` carries a **required** `delivery: DeliveryPersistence` field
(`types::delivery`, batch-1 `types` wave 3) — `LossyDrop` or `SaveFailed`. The
field is *required to author* (derives no `Default`, so a publisher must
consciously choose) yet decodes on the safe side (`SaveFailed`) for pre-#155
senders (the guardrail-4 "required to author, safe to decode" nuance, types.md).
"We can't just be silently dropping things." The relay branches on the field at
every point concern 6 currently drops.

**`LossyDrop` — today's semantics, untouched.** The wave-2 policy verbatim
(concern 6): bounded per-subscriber queues, `Lagged` notice on overflow, bounded
per-peer relay buffer, drop-and-count on peer overflow, no retention, `seq`-gap
detection for out-of-band re-sync. This is the ONLY behavior wave 2 had and it
stays the untouched fast path — the choice for pure fan-out (dashboards,
telemetry, live token streams).

**`SaveFailed` — persist-on-failure + re-offer-on-reconnect, via a separable
cache seam.** For a `SaveFailed` envelope, a delivery that concern 6 would *drop*
is instead **teed into an abstract intermediate-response cache** and **re-offered
when the intended recipient reconnects** — no silent drop. The three drop points
concern 6 names map to three persist points:

| concern-6 drop | SaveFailed behavior |
|----------------|---------------------|
| local subscriber lagged (bounded queue overflow) | the dropped envelopes are written to the cache under that subscriber's recipient key; the subscriber still receives its `Lagged` **notice** (liveness preserved) and drains the saved envelopes on catch-up |
| registered interest exists but the matching subscriber is momentarily disconnected (no live socket to satisfy it) | envelope written to the cache under the intended recipient key; re-offered on that subscriber's re-subscribe |
| cross-node relay: an interested peer daemon is unreachable | envelope written to the cache for that peer's interested recipients; re-offered when the peer reconnects and re-snapshots interest (concern 7) |

On subscriber **(re)connect + subscribe** (concern 8's re-announce), the relay
asks the cache for undelivered `SaveFailed` envelopes matching the new
subscription's filters and replays them (interleaved with, or ahead of, live
traffic; ordering and bound are the cache's retention/TTL policy). The subscriber
dedups by `envelope_id` (concern 1) — at-least-once on the durable path, exactly
as `queues` is at-least-once.

**The broker itself stays lossy.** `SaveFailed` adds only (a) a *tee* of
would-be-dropped envelopes into the cache and (b) a *drain* on reconnect. The
retention lives entirely in the cache seam, never in the in-memory broker — so the
durable path is a **clearly separable delegation** that could be un-merged with a
one-line change (ledger §C, OQ-3). This is also how the wave-2 boundary is
preserved: concern 4 kept a *retained-message/last-value store OUT of the lossy
transport*, and #155 does **not** put it back into the broker — it moves opt-in
retention to a separable, abstract cache. The transport is still lossy; durability
is a delegation.

**The cache seam — interface designed, mechanism OPEN (PARKED OQ-3).** The relay
calls an abstract seam and does **not** decide where the cache lives or how it
stores:

```rust
// internal mesh seam — NOT a cross-service contract edge (kept off the contract
// graph like chassis's BlessingTarget), swappable with a one-line binding change.
trait IntermediateCache {
    /// A SaveFailed delivery that could not reach an intended recipient.
    async fn save_failed(&self, undelivered: Undelivered);
    /// On (re)subscribe: undelivered SaveFailed envelopes for this recipient
    /// matching these filters, for replay. Removal/ack + TTL/eviction/bound are
    /// the cache's policy, not the relay's.
    async fn take_for(&self, recipient: RecipientKey, filters: &[TopicFilter]) -> Vec<Envelope>;
}
struct Undelivered  { envelope: Envelope, recipient: RecipientKey }
// intended recipient ACROSS reconnects = the durable service identity the
// registry already tracks, NOT the volatile socket. Boring provisional:
struct RecipientKey { service: Slug, node: NodeId }   // topic is implicit in the envelope
```

- **Boring provisional (INTENT #155 verbatim):** the cache is *"a caching system
  for intermediate responses that is also distributed, like a KV store"* — a
  **distributed KV-backed cache** (the same `replicated-kv` store promise
  resolutions ride, concern 10). `replicated-kv.md` concern 10 records the
  matching **consumer note** for this same seam story: a `pubsub-cache/` keyspace
  keyed by the same `RecipientKey { service, node }` defined above, values a small
  envelope buffer, `expiry_events` off (retention/TTL stays the cache's own
  policy). That is a *consumer note only* on kv's side — kv does not add the
  keyspace or implement `IntermediateCache` — and it is INTENT's own words, the
  default binding. Both files keep it PARKED (OQ-3); neither adopts it.
- **MUST NOT decide (PARKED OQ-3).** Whether `SaveFailed` deliveries are instead
  **enqueued into `queues`** (the wave-2 F5 consolidation — "saved pub/sub
  deliveries just get enqueued into queues") is the operator's parked "didn't seem
  very boring" question. This file **MARKS that consolidation NEEDS-EXPLANATION**
  and does **not** adopt it; the `IntermediateCache` seam is written so a `queues`
  backing OR a standalone distributed-KV backing is a one-line swap, and the two
  framings stay un-merged (ledger §C OQ-3: "the durable path is a clearly
  separable delegation to queues that could be un-merged").
- **Recipient reconstruction is the honest hard edge.** Keying re-offer by durable
  `(service, node)` is the boring choice — interest is volatile (concern 7), so we
  cannot re-offer by live socket. The *internal* cache semantics (per-recipient
  queue vs per-topic retained log with a subscriber cursor; TTL; eviction; bounded
  size, INTENT #38) are **cache-owned and OQ-3-open**; the relay contracts only the
  two seam calls above. Flagged, not decided.
- **Mixed-version relay hop.** A `SaveFailed` envelope relayed through a pre-#155
  daemon (which lacks the `delivery` header and the persist path) degrades to
  best-effort on that hop — a transient rolling-update window (surfaced), NOT a
  designed silent drop. The decode-fallback (`SaveFailed`) covers the reverse:
  a newer daemon decoding an older sender's fieldless envelope defaults to the
  safe side.

### 10. Promise-resolution notices ride pub/sub (INTENT #152 / #155)

INTENT #155: *"Promise resolution notices ride this too."* When a mesh promise
(INTENT #152 — a service could not answer instantly; `types::transport`) elects
**push** delivery on a pub/sub topic (`PromiseDelivery::Push { topic }`,
`types::transport`), the later resolution is published as an ordinary `Envelope`
on that topic and relayed by this module like any other message — with one
binding: it inherits the promise ticket's `DeliveryPersistence` (types.md: the
ticket's `persistence` field), which for a value the caller must not miss is
`SaveFailed`. So a caller that disconnected before its promise resolved
**collects the resolution on reconnect** from the SaveFailed cache (concern 9) —
never a silent drop, exactly #155's resiliency requirement, reusing concern 9's
machinery with no new mechanism.

- **No new contract file** — this is a documented *use* of `pubsub-protocol` (the
  carrier) + `mesh-transport` (the promise-ticket / resolution vocabulary). The
  pub/sub `Envelope.payload` is a `PromiseResolution<..>` (types.md, `transport.rs`);
  the relay stays payload-opaque (concern 2) — it neither knows nor cares that the
  payload is a resolution.
- **Two delivery channels for a resolution — reconciliation flagged, NOT resolved
  here.** `mesh-transport` (batch 1) also defines a transport-frame
  `PromiseFulfillment { promise, outcome }` correlated by promise id (the RPC-return
  channel on the caller's own connection), while types.md's `PromiseDelivery::Push
  { topic }` routes the resolution over a pub/sub topic (fan-out / observability, or
  a caller that prefers a topic). Which channel a given promise uses is the ticket's
  `PromiseDelivery`. Harmonizing the two representations — and confirming the
  `types::transport` `PromiseTicket`/`PromiseResolution` naming against
  mesh-transport's `ResponseOutcome::Promise`/`PromiseFulfillment` — is the
  **mesh-transport / chassis / types** harmonizer's call. This file guarantees only
  that *when* a resolution rides pub/sub, it rides as a normal, SaveFailed-capable
  `Envelope`.

### 11. Producer-side snapshot republish stays the retention answer for topology (wave 2, reaffirmed)

Reaffirming the wave-2 position (concern 4) against the wave-3 fold: the relay does
**NOT** grow a retained-message / last-value store for late subscribers. A
publisher that needs a newcomer to see current state (network-topology's
snapshot-then-delta, the registry's anti-entropy, dashboard bring-up)
**republishes its current state as normal publishes** to the new subscriber —
producer-side snapshot-on-connect, unchanged. This stays distinct from #155
`SaveFailed`:

- **Snapshot republish** answers *"a subscriber that connects LATE should see
  current state"* — a producer concern; the value is whatever is current,
  reconstructed by the producer.
- **`SaveFailed` (concern 9)** answers *"a specific message must not be silently
  dropped on a failed delivery"* — a per-message durability opt-in; the value is
  the exact undelivered envelope, held in the separable cache.

Both keep retention **out of the lossy broker**: one lives in the producer, the
other in the abstract cache. The relay core stays the boring lossy fan-out it was.
This closes the wave-2 open item ("the retained-snapshot-per-topic capability
`network-events` asks of the relay"): the answer is **producer-side snapshot
republish**, not a relay retained store — network-topology already owns that
publish.

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
- `types::delivery::DeliveryPersistence` — **library dependency**: the required
  `delivery` field on the pub/sub `Envelope` (INTENT #155) is homed in `types`
  (batch-1 wave 3), compiled into every publisher and this relay. The relay OWNS
  the *behavior* per mode (concern 9); `types` owns only the yes/no flag.
- **`IntermediateCache` seam (PARKED OQ-3, kept OFF the contract graph on
  purpose).** The `SaveFailed` retention target (concern 9) is an internal,
  swappable mesh seam — mirroring chassis's `BlessingTarget` treatment — defaulted
  (provisionally) to a distributed KV-backed cache and MARKED NEEDS-EXPLANATION for
  the queues-vs-pubsub consolidation. No `intermediate-cache` contract family, no
  `queues` dependency threaded here, until the operator un-parks OQ-3.

## Nesting

Parent: mesh | Children: none. Module `lib/mesh::pubsub` (server/broker side) +
the daemon↔daemon relay link. The client half (a service's
subscribe/publish handle to its local daemon) is part of `chassis`'s surface,
not a separate crate — services get pub/sub through the same thin boot lib they
get `register`/`resolve` from (mesh.md concern 0). Confirmed at skeleton time.

## Thoroughness level

**implementation-ready** — envelope header, opaque-payload relay, topic taxonomy,
the three subscription filter kinds, the two-addressing-class → scope mapping, the
lossy backpressure policy, the one-hop interest-routed loop-free relay, and — new
in wave 3 — the per-message `delivery` branch (LossyDrop unchanged; SaveFailed
tee-and-re-offer), the promise-resolution-rides-pubsub binding, and the reaffirmed
producer-side snapshot answer are all decided and grounded in the live V1 gateway
code. What remains open is genuinely downstream or deliberately PARKED: (a) the
exact interest-coarsening heuristic (fill-time tuning knob, not a design fork);
(b) reconciling the five existing `*-events` contracts onto the topic prefixes —
Contract Harmonizer work; (c) the `types` `pubsub.rs`/`event.rs` field sets —
authored, reconciled in the per-pair round; (d) **PARKED OQ-3** — the
`IntermediateCache` mechanism (distributed KV vs enqueue-into-`queues`) and its
internal retention semantics (per-recipient queue vs per-topic retained log +
cursor, TTL, eviction): the relay-side **seam** is designed and marked
NEEDS-EXPLANATION; the mechanism is the operator's to settle; (e) the two
promise-resolution delivery channels (pub/sub Push topic vs transport
`PromiseFulfillment`) — mesh-transport/types harmonizer's reconciliation.

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

- `pubsub-protocol` (every service via `chassis` ↔ mesh's pubsub-relay;
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
    relay is the one open design ask pushed to this pair. **RESOLVED (wave 3,
    concern 11):** the relay grows **no** retained store; `network-events` gets
    current-state to late subscribers by **producer-side snapshot republish**
    (network-topology's own publish), and the distinct #155 `SaveFailed` durability
    rides the separable `IntermediateCache` seam — neither is a relay retained store.

---

## Proposed contracts (wave 3)

This unit owns the **relay-side** of the #155 delivery-persistence fold. The
struct half (`DeliveryPersistence`) is `types`' (batch 1); the **contract-file
amendment** and the **relay behavior** are this unit's.

- **`pubsub-protocol` (amendment — this unit authors it).** Add the REQUIRED
  `delivery: DeliveryPersistence` field to the published `Envelope` and to the
  `Publish` client frame (grounded by `types::delivery`, INTENT #155). Guarantees
  bound at this contract: (a) required-to-author / safe-to-decode (no `Default`;
  `SaveFailed` serde fallback) — never a silent-drop default; (b) the relay
  branches per mode — `LossyDrop` = the wave-2 lossy policy verbatim, `SaveFailed`
  = tee-failed-into-cache + re-offer-on-reconnect (concern 9); (c) `types` names
  only the yes/no flag — the persistence **mechanism** (distributed KV cache;
  whether saved deliveries enqueue into `queues`) stays with pubsub-relay/mesh and
  is **MARKED NEEDS-EXPLANATION** (PARKED OQ-3), so flag and mechanism un-merge
  independently. → `scaffold/contracts/pubsub-protocol.md` (updated by this unit).

- **Promise-resolution notice (rides `pubsub-protocol`, no new file).** A pushed
  promise resolution (`PromiseDelivery::Push { topic }`, `types::transport`) is
  delivered as an `Envelope<PromiseResolution<..>>` on the ticket's topic,
  inheriting the ticket's `DeliveryPersistence` (INTENT #152/#155). A documented
  use of `pubsub-protocol` + `mesh-transport`; the relay stays payload-opaque
  (concern 10). The pub/sub-vs-transport channel reconciliation is flagged for the
  mesh-transport/types harmonizer, not decided here.

- **`IntermediateCache` seam — deliberately NOT a contract (PARKED OQ-3).** The
  `SaveFailed` retention target is an internal, one-line-swappable mesh seam
  (concern 9), off the contract graph like chassis's `BlessingTarget`. No
  `intermediate-cache` contract family, no `queues` edge, until OQ-3 is un-parked.
