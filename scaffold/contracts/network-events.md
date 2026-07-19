# Contract: network-events

## Parties
`mesh.network-topology` (producer — one tracker per mesh daemon) → any
subscriber: **ccd**, **org**, and mesh's own in-process observability hub.
Delivered as `pubsub-relay` envelopes on topic `net.topology`, relayed across
services and nodes by the pub/sub plane (rides `pubsub-protocol`).
**Harmonization flag (unresolved tension):** this file assumed a relay-side
retained-per-topic capability for the `Snapshot`, but `pubsub-relay.md`
concern 4 explicitly scopes retention OUT of the lossy relay (publishers
replay current state as ordinary publishes), and `pubsub-protocol.md` defines
no retention. Until the operator/next pass grants pubsub-relay a
retained-message store, the producer satisfies every "retained `Snapshot`"
reference below by REPUBLISHING a fresh `Snapshot` on subscriber-join and on
`seq`-gap resync — same guarantees, producer-side. (Friction report.)
*(gateway removed as a subscriber 2026-07-18 — merged into mesh; the
observability fan-out hub now consumes topology in-process, not over this
edge.)*

Authored from `network-topology.md` (sole producer/owner; the only party that
proposed this edge).

## Purpose
A live, provenance-tagged feed of tailnet transitions — peers
joining/leaving/going online/offline — and, the reason the module earns its
own component, **this device's own loss and recovery of tailnet connectivity**
(`SelfOffline`/`SelfOnline`) as first-class events (INTENT: "if we lose
connection to the tailscale network ourselves, that's important information").
Every feed is a **node-local vantage** — "what THIS mesh daemon can currently
see" — never a claim of global truth; that framing is load-bearing for the
peers-unknown semantics below.

## Schema
Rides `Envelope<NetworkEvent>` from the `pubsub-protocol` / `types::pubsub`
domain; the event types belong in a proposed `types::net` module (flagged for
the `types` owner).

```rust
// Provenance-first header, present on EVERY event (node-local vantage):
struct Provenance {
    observer:    NodeId,           // who saw it
    observed_at: DateTime<Utc>,    // when
    seq:         u64,              // monotonic per-observer; ordering / gap detection
}

#[non_exhaustive]
enum NetworkEvent {
    // RETAINED on the topic — the relay replays the last one to every new subscriber:
    Snapshot   { prov: Provenance, topology: TopologySnapshot },
    // transient deltas:
    PeerJoined { prov: Provenance, peer: PeerRef },
    PeerLeft   { prov: Provenance, peer: PeerRef },
    PeerOnline { prov: Provenance, peer: PeerRef },
    PeerOffline{ prov: Provenance, peer: PeerRef },
    SelfOffline{ prov: Provenance, cause: SelfOfflineCause },
    SelfOnline { prov: Provenance, topology: TopologySnapshot }, // fresh full state on recovery
}

struct TopologySnapshot {
    self_state:  SelfConnectivity,
    peers:       Vec<PeerRef>,        // Unknown-marked while self offline
    captured_at: DateTime<Utc>,
}

struct PeerRef {
    node_id:       NodeId,            // device name (routing / cross-ref key vs service-registry)
    stable_id:     String,           // Tailscale StableNodeID (identity, survives IP churn)
    host_name:     String,
    tailscale_ips: Vec<IpAddr>,
    tags:          Vec<String>,       // ACL tags, e.g. ["tag:substrate"]
    state:         PeerState,
}

#[non_exhaustive] enum PeerState { Online, Offline, Unknown } // Unknown ONLY while self-offline
#[non_exhaustive] enum SelfConnectivity { Online, Offline { cause: SelfOfflineCause }, Unknown }

#[non_exhaustive]
enum SelfOfflineCause {
    TailnetUnreachable,               // daemon up, no DERP/control
    BackendNotRunning(BackendState),  // Stopped / NeedsLogin / NoState / Starting
    TailscaleUnavailable,             // binary/daemon missing
    PollTimeout,                      // poll exceeded the per-poll deadline
}
```

**Behavioral guarantees the contract states (not just the shapes):**
1. **Peers-unknown-under-self-offline.** Between a `SelfOffline` and the next
   `SelfOnline`, NO `Peer*` events are emitted, and consumers MUST treat all
   peer state as `Unknown` (stale-but-not-dead). The retained `Snapshot`
   during this window carries `self_state = Offline{..}` and peers marked
   `Unknown`. This is the single most important behavioral guarantee — a
   device that can't see the tailnet must never synthesize `PeerOffline` for
   everyone.
2. **Recovery replays as a fresh snapshot + membership-only deltas.**
   `SelfOnline` carries a full `TopologySnapshot`; any per-peer deltas
   immediately after are `PeerJoined`/`PeerLeft` only (membership) — never
   `PeerOnline`/`PeerOffline` churn accrued during the blackout, which can't
   be time-attributed and is meaningless to replay.
3. **Asymmetric debounce is observable, not hidden.** Up-events
   (`PeerOnline`/`PeerJoined`/`SelfOnline`) emit on the first confirming poll;
   down-events (`PeerOffline`/`PeerLeft`/`SelfOffline`) require
   `down_confirm_polls` consecutive confirming polls (default 2, ≈10s). A
   consumer will never see a sub-`down_confirm` flap.
4. **Ordering / gap detection.** `seq` is monotonic per `observer`; a
   subscriber detecting a `seq` gap treats its view as stale until the next
   retained `Snapshot`.

## Error cases
- Remote relay unreachable → best-effort, **at-most-once** for transient
  deltas; recovery is the retained `Snapshot` + `seq`-gap rule (a subscriber
  that missed deltas resyncs from the next retained snapshot). In-process
  delivery (mesh's own hub) does not drop.
- `network-topology` cannot fail the edge from the producer side beyond
  ceasing to publish; a stalled poll loop surfaces via its own health / surface
  schema, NOT as a `NetworkEvent`.
- The retained-`Snapshot`-per-topic requirement is a capability this edge
  needs from `pubsub-relay`. If the relay cannot retain, `network-topology`
  falls back to answering a `topology.snapshot` request on subscribe (the
  less-boring alternative) — a design dependency, flagged for the
  `pubsub-protocol` pair.

## Version sensitivity
- **Additive-safe:** `NetworkEvent`, `PeerState`, `SelfConnectivity`,
  `SelfOfflineCause`, `BackendState` are all `#[non_exhaustive]`, so new
  variants (a future `PeerDegraded`, or netcheck-derived latency events once
  `tailscale-query` grows `netcheck()`) are additive; subscribers match with a
  catch-all arm. New `#[serde(default)]` fields on `PeerRef`/`Provenance`/
  `TopologySnapshot` are additive.
- **Breaking:** removing/retyping an existing field, or changing the
  peers-unknown-under-self-offline guarantee (that is semantic wire, not just
  shape). A retained-`Snapshot` format change is breaking for late joiners.
- Because this rides `pubsub-protocol`'s envelope, the envelope's own version
  discipline governs the transport; `NetworkEvent` payload evolution is
  additive per the above.

## Reconciliation notes
- **Single-party edge — authored from the producer's proposal.**
  `network-topology.md` is the only party; no cross-party disagreement to
  resolve. Subscribers (ccd, org) named the edge in their designs but proposed
  no conflicting shape, so this file adopts the producer's proposal verbatim.
- **Gateway dropped as a subscriber** (2026-07-18 mesh merge): the
  observability fan-out hub now consumes topology in-process, so the external
  subscriber list is ccd + org + (in-process) mesh hub. Updated from the
  wave-1 stub, which still named gateway.
- **NodeId naming edge, flagged for the harmonizer.** `PeerRef` carries BOTH
  `node_id` (device name — the routing/cross-ref key shared with
  service-registry and completion-router) and `stable_id` (Tailscale
  StableNodeID — the identity that survives IP churn). `types::NodeId` today
  is the device-name string; topology identity is the stable key. Both are
  surfaced deliberately; whether `types` grows a distinct `StableNodeId`
  newtype is the harmonizer's call, not this edge's.
- **Distinct from sibling tables.** This feed is NOT `completion-router`'s
  `NodeRegistry` (live inference load/health) and NOT `service-registry`
  (slug→endpoint). The mesh invariant that these three tables are never
  conflated is load-bearing; `network-events` is the peer-vantage feed only.
- **Retained-snapshot dependency** on `pubsub-relay` is the one open design
  ask (pushed to the `pubsub-protocol` pair); the fallback keeps this edge
  authorable regardless.

## Example data
The example world: nodes **macbook** (the operator's laptop, current observer)
and **pi** (always-on inference node, `tag:substrate`, running qwen3-4b for
project **demo**).

**1. Steady state — `pi` comes online, observed from `macbook`:**
```jsonc
// retained Snapshot on net.topology (replayed to any new subscriber):
{ "Snapshot": {
  "prov": { "observer": "macbook", "observed_at": "2026-07-19T00:00:00Z", "seq": 812 },
  "topology": {
    "self_state": "Online",
    "captured_at": "2026-07-19T00:00:00Z",
    "peers": [ { "node_id": "pi", "stable_id": "nABC123CNTRL", "host_name": "pi",
                 "tailscale_ips": ["100.64.0.7"], "tags": ["tag:substrate"], "state": "Online" } ]
  }
}}
// transient delta a moment earlier (pi had just appeared, up = emitted immediately):
{ "PeerJoined": {
  "prov": { "observer": "macbook", "observed_at": "2026-07-18T23:59:58Z", "seq": 811 },
  "peer": { "node_id": "pi", "stable_id": "nABC123CNTRL", "host_name": "pi",
            "tailscale_ips": ["100.64.0.7"], "tags": ["tag:substrate"], "state": "Online" } }}
```

**2. `macbook` loses the tailnet (drives out of range) — the crux behavior:**
```jsonc
// after down_confirm_polls (≈10s) of TailnetUnreachable observations:
{ "SelfOffline": {
  "prov": { "observer": "macbook", "observed_at": "2026-07-19T00:05:12Z", "seq": 815 },
  "cause": "TailnetUnreachable" }}
// NO PeerOffline for pi is emitted. The retained Snapshot now reads:
{ "Snapshot": {
  "prov": { "observer": "macbook", "observed_at": "2026-07-19T00:05:12Z", "seq": 816 },
  "topology": { "self_state": { "Offline": { "cause": "TailnetUnreachable" } },
    "captured_at": "2026-07-19T00:05:12Z",
    "peers": [ { "node_id": "pi", "stable_id": "nABC123CNTRL", "host_name": "pi",
                 "tailscale_ips": ["100.64.0.7"], "tags": ["tag:substrate"], "state": "Unknown" } ] }
}}
```

**3. `macbook` rejoins — fresh snapshot, membership-only diff:**
```jsonc
{ "SelfOnline": {
  "prov": { "observer": "macbook", "observed_at": "2026-07-19T00:12:40Z", "seq": 817 },
  "topology": { "self_state": "Online", "captured_at": "2026-07-19T00:12:40Z",
    "peers": [ { "node_id": "pi", "stable_id": "nABC123CNTRL", "host_name": "pi",
                 "tailscale_ips": ["100.64.0.7"], "tags": ["tag:substrate"], "state": "Online" } ] }
}}
// pi never left across the gap, so no PeerJoined/PeerLeft follows — the fresh
// snapshot IS the truth; no online-substate churn is replayed.
```
