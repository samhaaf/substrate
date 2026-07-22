# Contract: pubsub-protocol

## Parties

- **every service** (via `chassis` — formerly `mesh-client`, absorbed, see
  `components/mesh-client.md`) — publisher and/or subscriber
- **mesh** (`pubsub-relay`, the L1 daemon facet on `:3649`) — the relay

Cross-cutting, surface-schema-style: ONE shared document, every service is a
party (wave2-plan §5.4). The struct vocabulary is homed in `types` (`pubsub.rs`,
`event.rs`, `provenance.rs`); the relay behavior is `pubsub-relay`'s. This file
is the reconciliation of both.

## Purpose

The single WebSocket wire protocol a service speaks to its LOCAL mesh daemon to
**publish** typed messages onto **topics** and **subscribe** to topics, with the
mesh relaying matched messages "where they need to go" — across services and
across nodes (INTENT #53, "everything over WebSockets" #28). **Best-effort /
lossy by contract**: durability is `queues-api`'s job, never this one. The relay
is **payload-opaque** — it routes on the envelope header alone and never
deserializes a payload; a party decodes the concrete `Event` variant only at the
edge.

## Schema

All frames are JSON, tagged enums (`#[serde(tag = "type")]`) per the existing
`stream.rs` convention.

### Envelope (the message header — `types::pubsub`)

```rust
struct Envelope {
    v: u16,                          // types version-tolerance anchor
    envelope_id: Uuid,               // daemon-stamped on publish
    topic: Topic,
    event_type: String,             // "domain.noun.verb" — routable WITHOUT decoding payload
    provenance: Provenance,
    // REQUIRED (INTENT #155): the publisher's save-failed-deliveries choice.
    // Type derives NO `Default` (required to author); serde decode-fallback is the
    // SAFE side (SaveFailed) for pre-#155 senders — never a silent-drop default.
    // The relay BRANCHES on this (see "Delivery persistence" below).
    #[serde(default = "delivery::DeliveryPersistence::save_failed")]
    delivery: delivery::DeliveryPersistence,   // LossyDrop | SaveFailed  (types::delivery)
    causal_parent: Option<Uuid>,     // envelope_id of the causing message
    payload: Box<serde_json::value::RawValue>,  // OPAQUE at the relay boundary
}

struct Provenance {
    service: Slug,                   // publishing service (daemon-verified vs registry lease)
    #[serde(default)] service_version: Option<SemVer>, // LOCKED (INTENT #113): sender version stamp
    node_id: NodeId,
    emitted_at: DateTime<Utc>,       // millis-resolution
    seq: u64,                        // per-(service,node) monotonic; resets on service restart
    correlation_id: Option<Uuid>,    // causal-chain root (shared with queues/cron)
    causation_id: Option<Uuid>,      // immediate cause (shared with queues/cron)
}

struct Topic { scope: Scope, path: String }   // path: lower-snake, dot-segmented, validated
enum   Scope { Fleet, Node(NodeId) }

enum TopicFilter {
    All,
    Exact(Topic),                    // per-entity subscription (incl. per-completion, INTENT #5)
    Prefix { scope: ScopeFilter, prefix: Vec<String> },  // subtree
}
enum ScopeFilter { Any, Fleet, Node(NodeId) }
```

Typed edges may use the generic view `Envelope<P>` where `P = RawValue` on the
wire/relay and `P = Event<Concrete>` after edge-decode; the wire form is always
the opaque one above.

### Client → daemon (`PubSubClientMsg`)

```rust
enum PubSubClientMsg {
    Publish {
        topic: Topic,
        event_type: String,
        delivery: delivery::DeliveryPersistence, // REQUIRED (#155): per-message LossyDrop | SaveFailed
        causal_parent: Option<Uuid>,
        payload: Box<RawValue>,      // daemon stamps envelope_id + provenance.{node_id,emitted_at,seq}
    },
    Subscribe   { filters: Vec<TopicFilter> },   // ADDITIVE to the connection's current set
    Unsubscribe { filters: Vec<TopicFilter> },
}
```

### Daemon → client (`PubSubServerMsg`)

```rust
enum PubSubServerMsg {
    Event(Envelope),                        // a matched, relayed message
    SubAck { active: Vec<TopicFilter> },    // current subscription set after a Sub/Unsub
    Lagged { dropped: u64, since_seq: u64 },// NOTICE, not error — the lossy contract, in band
    Error  { code: PubSubError, detail: String },
}
```

### Daemon ↔ daemon (`RelayFrame` — internal facet, versioned WITH this contract)

```rust
enum RelayFrame {
    Deliver { envelope: Envelope },          // relayed==true implicitly; never re-relayed
    InterestSnapshot { node: NodeId, epoch: u64, filters: Vec<TopicFilter> },
    InterestDelta    { node: NodeId, epoch: u64,
                       add: Vec<TopicFilter>, remove: Vec<TopicFilter> },
}
```

### Delivery persistence — LossyDrop vs SaveFailed (INTENT #155)

The `Envelope.delivery` field makes lossiness a **per-message publisher choice**;
`pubsub-relay` branches on it (mechanics: `components/pubsub-relay.md` concern 9).
The wire guarantee this contract binds:

- **`LossyDrop`** — the best-effort/lossy default of this protocol, verbatim: a
  lagged subscriber gets a `Lagged` notice and misses events; a `Scope::Node(N)`
  publish to an offline N is a silent best-effort drop; no retention. This is the
  ONLY behavior before #155 and stays unchanged.
- **`SaveFailed`** — a delivery that `LossyDrop` would drop (subscriber lagged,
  interest registered but subscriber momentarily disconnected, interested peer
  daemon unreachable) is instead **persisted into a distributed intermediate-
  response cache and re-offered on the intended recipient's reconnect** — "no
  silent drops" (#155). The subscriber still receives its `Lagged` **notice** for
  liveness and dedups the replayed envelopes by `envelope_id` (at-least-once on the
  durable path). Replay happens on `Subscribe`: the relay returns undelivered
  `SaveFailed` envelopes matching the new filters.

The cache is a **distributed KV-backed store** (INTENT #155 verbatim — "a caching
system for intermediate responses… distributed, like a KV store"). **Whether
saved deliveries are instead enqueued into `queues`** (the F5 consolidation) is
**PARKED (OQ-3)** and MARKED **NEEDS-EXPLANATION** — this contract names only the
yes/no flag and the delivered guarantee, never the mechanism; pubsub-relay holds
the seam so the flag and its backing un-merge independently.

**Promise-resolution notices ride this** (#152/#155): a pushed promise resolution
(`PromiseDelivery::Push { topic }`, `mesh-transport`/`types::transport`) is an
`Envelope` on the ticket's topic that inherits the ticket's `DeliveryPersistence`
(`SaveFailed` for a value the caller must not miss) — so it survives a caller
reconnect via the same cache. See `mesh-transport` for the transport-frame
(`PromiseFulfillment`) channel; which channel a promise uses is its ticket's
`PromiseDelivery`.

## Error cases

`PubSubError` (server frame):
- `InvalidTopicPath` — path not lower-snake dot-segmented, or has an empty segment.
- `InvalidFilter` — malformed prefix (e.g. an empty prefix that isn't `All`).
- `NotRegistered` — the publishing service has no live registry lease, so its
  provenance can't be attested; the publish is rejected (ties pub/sub honesty to
  the registry). Also raised on a browser/read-only party attempting `Publish`.
- `PayloadTooLarge` — payload exceeds the per-message cap (bounded-memory guard).

Non-errors by design:
- `Lagged` is a **notice** delivered as its own `ServerMsg`, never a connection
  failure — the lossy contract expressed in band.
- Publishing to a topic with **no subscribers** succeeds silently (normal at
  startup, per V1 `Hub::publish`).
- A `Scope::Node(N)` publish where N is offline is a silent best-effort drop.

Transport/relay failures surface as `mesh-transport`'s `PeerUnreachable`, never
as a `PubSubError` — transport and pub/sub semantics stay layered.

## Version sensitivity

**HIGH** — envelopes cross nodes on possibly-different `types` versions (mixed
fleet, INTENT #66). `Envelope.v` is the anchor.

- **Additive-safe:** new optional / `#[serde(default)]` fields on `Envelope` and
  `Provenance`; new `#[serde(other)]`-tolerant variants on the message enums;
  **any new `event_type` string** — the payload-opaque relay carries a new event
  type through an older daemon untouched. This decoupling is the primary
  version-safety property and what lets pub/sub survive the mixed-version fleet
  without a coordinated upgrade.
- **Breaking:** changing the `Envelope` header field set/meaning, changing topic
  path grammar, or making the relay parse payloads. Requires an operator round
  and drives `RestartReason::Compatibility` restarts (see restart-protocol).
- Relay sites operate on the opaque `Envelope` and MUST NOT `deny_unknown_fields`.
- **LOCKED (friction-round 1, INTENT #113) — sender version stamping.** Every
  mesh-crossing message carries the sending service's **name AND version**:
  `Provenance.service` + the new `service_version` (stamped by the daemon
  alongside `node_id`/`emitted_at`/`seq`, verified against the registry lease).
  A subscriber MAY enforce a **version floor** ("only accepting messages from
  nodes with service version greater than X") — a floored message is rejected
  at the subscriber edge with a **catchable** error
  (`VersionBelowFloor`-shaped), never a silent drop. On a schema change, a
  receiver offers **backwards compatibility for one version** and attaches a
  **please-update warning back to the sender** (delivered on the sender's own
  connection/topic) telling it to update. The relay itself stays
  payload-opaque and never enforces floors — flooring is a receiver-edge
  policy.
- `provenance.seq` is per-(service,node) and **resets on service restart**;
  subscribers must read a backward `seq` jump as "publisher restarted," not
  corruption. The daemon folds the registry lease generation into provenance so
  restart is detectable.
- **`Envelope.delivery` — the sanctioned "required to author, safe to decode"
  exception (INTENT #155).** The field derives **no `Default`** (a publisher must
  choose LossyDrop or SaveFailed), yet its serde attribute
  (`#[serde(default = "…save_failed")]`) decodes a **pre-#155 sender's fieldless
  envelope to `SaveFailed`** — the resilient side, never a silent-drop default.
  This is the one place the additive-optional rule bends, and it bends toward
  resiliency exactly as #155 asks. A `SaveFailed` envelope **relayed through a
  pre-#155 daemon** (which lacks the field and the persist path) degrades to
  best-effort on that hop — a transient rolling-update window, surfaced, **not** a
  designed silent drop.

## Reconciliation notes

Two divergent proposals existed for the same wire. **`pubsub-relay`'s
relay-authoritative shape wins**; `types`' first-cut `pubsub.rs` code sketch is
superseded (and its own *prose* proposal already agreed with pubsub-relay's
opaque-payload / open-identifier philosophy). Point by point:

1. **Opaque vs generic payload.** `types::pubsub` sketched `Envelope<P>` with a
   typed `payload: P`; `pubsub-relay` used an opaque `payload: Box<RawValue>` +
   a top-level `event_type` string. **Opaque + header `event_type` wins:** the
   relay MUST route without deserializing (types' own contract prose says
   "relay sites operate on `Envelope<serde_json::Value>`"). Typed `Envelope<P>`
   survives as an edge-only convenience view. *Losing view retained:* the
   generic form is still the ergonomic decode at the subscriber edge.

2. **Ack semantics.** `types` proposed `PubSubServerMsg::Ack { envelope_id }`
   (per-message ack). **Dropped** — a per-message ack contradicts the
   best-effort/lossy contract and conflates pub/sub with `queues-api`'s
   at-least-once delivery. Replaced by `SubAck` (subscription ack) + `Lagged`
   (lossy-drop notice). *Losing view recorded:* if a caller wants delivery
   guarantees, it uses `queues-api`, not this contract.

3. **Topic shape.** `types` had a flat `Topic` with a distinct
   `TopicFilter::Completion(CompletionId)` variant; `pubsub-relay` used a scoped
   `Topic { scope, path }` with `Fleet`/`Node` scopes. **Scoped topic wins**
   (needed for `Scope::Node` routing). Per-completion subscription (INTENT #5)
   folds into `Exact(Topic { Fleet, "inference.completion.<id>" })` — the exact
   shape pubsub-relay's example already uses. *Losing variant recorded:* the
   dedicated `Completion` filter is subsumed by `Exact`-on-leaf, not lost.

4. **Subscription identity.** `types` modeled a `Subscription { subscriber_id,
   filters }` with `Unsubscribe { subscriber_id }`; `pubsub-relay` made the
   connection itself the subscriber with an additive filter set. **Connection-as-
   subscriber wins** for a single socket. *Losing view:* the `subscriber_id`
   model matters only for fan-in multiplexing of many logical subscribers on one
   socket — `chassis` (formerly `mesh-client`) handles that above this layer
   (see mesh-transport's `Frame` multiplexing), so it isn't needed on the wire.

5. **Error taxonomy.** Merged. `types` had `{UnknownTopic, NotSubscribed,
   PayloadTooLarge, Malformed}`; `pubsub-relay` had `{InvalidTopicPath,
   InvalidFilter, NotRegistered, PayloadTooLarge}`. Kept the latter (better
   argued: `NotRegistered` ties provenance to the registry lease). Dropped
   `UnknownTopic` (no-subscriber publish succeeds silently) and `NotSubscribed`
   (Unsubscribe of an absent filter is a no-op, not an error); `Malformed` maps
   to `InvalidTopicPath`/`InvalidFilter`.

6. **`mesh-client`'s multiplexing `Envelope`.** mesh-client proposed
   `Envelope { protocol_version, frame: Frame }` where pub/sub is *one* `Frame`
   kind alongside generic RPC and the other client protocols. That is a
   **transport-multiplexing** wrapper (mesh-transport), NOT the pub/sub message
   header — a name collision. Resolution: rename mesh-client's wrapper to the
   mesh-transport `Frame`; `PubSubClientMsg`/`PubSubServerMsg` are one frame kind
   carried inside it, and the pub/sub `Envelope` above is the header inside
   `Publish`/`Event`. mesh-client's `Topic { Named | Completion }` and
   `TypedEvent` reconcile to the scoped `Topic` and `types::event::Event` here.

7. **Provenance shape** was merged across three proposers (pubsub-relay's
   `{service, node_id, ts_millis, seq}`, cron/queues' `{origin_node,
   origin_service, emitted_at, correlation_id, causation_id}`). The union above
   is the canonical `types::provenance::Provenance`; field names normalized
   (`ts_millis`→`emitted_at`, `origin_*`→`service`/`node_id`).

8. **Convergence coupling (deferred to harmonization):** `network-events`,
   `dashboard-feed`, `inference-events`, `gc-events`, `cc-events` re-express as
   topic prefixes on this envelope, becoming *examples of payloads*, not
   independent wire formats. Their example data should be authored as `Envelope`s
   on the reserved prefixes in this one example world.

9. **`delivery` field added (wave 3, INTENT #155).** The required
   `delivery: DeliveryPersistence` field lands on `Envelope` + `Publish` (homed in
   `types::delivery`, batch-1 wave 3). `types.md` proposed it; `pubsub-relay` (this
   contract's owner) binds it and owns the per-mode relay behavior. The persistence
   **mechanism** (distributed KV cache vs enqueue-into-`queues`) is **PARKED OQ-3**
   and MARKED NEEDS-EXPLANATION — this file names only the flag and the delivered
   guarantee. The wave-2 open ask "retained-snapshot-per-topic for the relay" is
   **RESOLVED**: it is answered by **producer-side snapshot republish**
   (network-topology's own publish), NOT a relay retained store; #155 `SaveFailed`
   is the distinct, separable per-message durability path (pubsub-relay concern 11).

## Example data

`inference` on node **pi** publishes a token event for completion `c-8f3`
(project `demo`, model `qwen3-4b`). A dashboard on **macbook** subscribed to
`Prefix { Any, ["inference"] }` receives it after one relay hop.

Client → daemon (on pi):
```jsonc
{ "type": "Publish",
  "topic": { "scope": "Fleet", "path": "inference.completion.c-8f3" },
  "event_type": "inference.completion.token",
  "delivery": "LossyDrop",
  "causal_parent": null,
  "payload": { "completion_id": "c-8f3", "model": "qwen3-4b",
               "token": " world", "index": 12 } }
```

Daemon → subscriber (on macbook):
```jsonc
{ "type": "Event",
  "envelope": {
    "v": 1,
    "envelope_id": "b1e0c3a2-0000-0000-0000-000000000001",
    "topic": { "scope": "Fleet", "path": "inference.completion.c-8f3" },
    "event_type": "inference.completion.token",
    "delivery": "LossyDrop",
    "provenance": { "service": "inference", "node_id": "pi",
                    "emitted_at": "2026-07-19T18:20:00.123Z", "seq": 4471,
                    "correlation_id": null, "causation_id": null },
    "causal_parent": null,
    "payload": { "completion_id": "c-8f3", "model": "qwen3-4b",
                 "token": " world", "index": 12 } } }
```

Subscribe / SubAck handshake the macbook dashboard did first:
```jsonc
// -> { "type": "Subscribe", "filters": [ { "Prefix": { "scope": "Any", "prefix": ["inference"] } } ] }
// <- { "type": "SubAck", "active": [ { "Prefix": { "scope": "Any", "prefix": ["inference"] } } ] }
```
