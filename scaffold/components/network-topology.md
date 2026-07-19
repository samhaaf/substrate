# network-topology

**Status:** NEW (wave-2 refinement of the wave-1 approach-sketch).
**Nesting:** child of mesh — internal lib `lib/mesh::topology` (module
`lib/mesh/src/topology/`). No crate boundary crossed: consumers subscribe over
the mesh WS pub/sub, never link this module.

## Charter

`network-topology` is the single mesh-internal lib that turns a stream of
`tailscale-status` snapshots into a **live, typed feed of tailnet transition
events** published on the `net.topology` topic (`network-events` edge): peers
joining / leaving / going online / going offline, and — the reason it earns its
own component — **this device's own loss of, and recovery of, connection to the
tailnet** (`self_offline` / `self_online`) as first-class events. It owns the
poll loop, the snapshot-diff state machine, the debounce/cadence policy, the
self-connectivity detector, and the retained-snapshot-on-connect behavior. It
does **NOT**: query Tailscale itself (consumes `tailscale-query`, which owns the
shell-out and parsing); make routing decisions or maintain a load/health table
(that is `completion-router`'s `NodeRegistry`, a deliberately separate concern
even though both consume `tailscale-status`); own the transport of the feed
(that is `pubsub-relay`, which relays the envelopes across services and nodes).
Its output is **a node-local vantage** — "what THIS mesh daemon can currently
see of the tailnet" — never a claim of global truth; that framing is load-bearing
for the self-offline semantics below.

## Primary design concerns

The module is L–M complexity, but three of its concerns are genuinely subtle
and are why it is not clumped into `mesh-core` or `completion-router`:

### 1. Self-connectivity loss is a distinct first-class event, not a peer diff

INTENT (operator, verbatim-grade): *"if we lose connection to the tailscale
network ourselves, that's important information."* This is the distinguishing
concern. Self-offline is **inferred**, not diffed, from four observation
outcomes of a single poll:

| Observation | Meaning | `SelfOfflineCause` |
|---|---|---|
| shell-out returns, `BackendState == Running`, `self.online == true` | connected | (online) |
| shell-out returns, `BackendState == Running`, `self.online == false` | daemon up, tailnet unreachable (no DERP/control) | `TailnetUnreachable` |
| shell-out returns, `BackendState != Running` (`Stopped`/`NeedsLogin`/`NoState`/`Starting`) | tailscaled up but not serving the tailnet | `BackendNotRunning(state)` |
| shell-out errors: binary missing / daemon socket unreachable | tailscale not usable at all | `TailscaleUnavailable` |
| shell-out exceeds the per-poll deadline | tailscaled hung (often itself a symptom of network loss) | `PollTimeout` |

All non-first rows are self-offline *observations*; the state machine
(concern 3) decides when they become an emitted `self_offline` event. The
`cause` is carried on the event so a consumer can tell "we were logged out"
from "the network is down" from "tailscale isn't installed here."

### 2. Peers are UNKNOWN when self is offline — never reported offline

The crux of the whole component. When this device cannot see the tailnet, it
**cannot observe peer state at all**, so it must NOT emit `peer_offline` for
every peer just because they vanished from a failed/empty status. On the
`online → offline` self transition the machine:

- emits exactly one `self_offline { cause }`,
- **freezes** the last-known-good peer snapshot and marks every peer
  `PeerState::Unknown` internally,
- **stops emitting peer transitions entirely** until self recovers.

Consumers are contractually told: after a `self_offline`, treat all peer state
as `Unknown` (stale-but-not-dead) until a `self_online` delivers a fresh
snapshot. This is the single most important behavioral guarantee of the edge
and the reason "peers-unknown vs peers-offline" is a named requirement.

On the `offline → online` self recovery the machine:

- emits `self_online { snapshot }` carrying a **fresh full `TopologySnapshot`**,
- diffs the fresh snapshot against the pre-blackout last-known snapshot and
  emits only **membership** deltas (`peer_joined` / `peer_left`) for peers that
  actually appeared/disappeared across the gap — deliberately **not**
  `peer_online` / `peer_offline` churn, because online-substate changes that
  happened while we were blind cannot be time-attributed and are meaningless to
  replay,
- resets its diff baseline to the fresh snapshot and resumes normal diffing.

### 3. Poll cadence + asymmetric debounce policy

Steady state polls `tailscale-query::status()` on a fixed cadence. `tailscale
status --json` reads the **local** tailscaled over its unix socket (no network
round-trip), so it is cheap; the granularity that matters is Tailscale's own
peer online/offline detection (~tens of seconds via keepalive), so polling
faster than that buys nothing.

- **`poll_interval`** — default **5s** (config `topology.poll_interval_secs`).
  Kept constant even while self-offline (recovery must be detected promptly;
  the call stays cheap).
- **Per-poll deadline** — default **3s** (`topology.poll_timeout_secs`). Bounds
  a hung tailscaled; a timed-out poll is an offline *observation* (cause
  `PollTimeout`), never a stall of the loop. Since `tailscale-query` is a
  synchronous trait (its charter), the poll runs under `spawn_blocking` wrapped
  in `tokio::time::timeout`.
- **Asymmetric debounce** — *up is good news, emit fast; down might be flap,
  confirm it.*
  - `peer_online` / `peer_joined` / `self_online`: emitted on the **first**
    confirming poll (no debounce).
  - `peer_offline` / `peer_left` / `self_offline`: require **`down_confirm`
    consecutive** confirming polls (default **2**, ≈10s) before emission. A
    down-then-up inside the window emits nothing — flap is absorbed.
  This is a single knob (`topology.down_confirm_polls`) applied uniformly to
  peer-down and self-down, so the policy is one boring rule, not a matrix.

### 4. Node-local vantage + provenance-first

Every mesh daemon runs its own `topology` tracker; each feed is **that node's
observation**, and two nodes may legitimately disagree (a peer offline from my
DERP region may be online from yours). Therefore every emitted event carries
provenance as a first-order field, not an afterthought: `observer: NodeId`
(who saw it), `observed_at: DateTime<Utc>` (when), and a monotonic `seq: u64`
(per-observer ordering / gap detection). Consumers that aggregate multiple
nodes' feeds (e.g. mesh's own observability hub, or `org`) use `observer` to
keep vantages distinct rather than collapsing them into a false global truth.
This directly honors the standing "provenance first-order where data is
touched" principle at the exact point data is first observed.

### 5. Snapshot-on-connect then deltas (retained-message semantics)

A new subscriber must not start blind. The `net.topology` topic carries two
message kinds: a **retained** `Snapshot` (the relay replays the last retained
message to every new subscriber) and transient `Delta` events. `network-topology`
re-publishes the retained `Snapshot` whenever the full topology materially
changes (and always immediately after `self_online`). This pushes the
"replay current state to late joiners" responsibility onto `pubsub-relay`'s
retained-per-topic capability rather than making `network-topology` special-case
the subscribe path — see friction points (this is a concrete ask of
`pubsub-relay`'s design).

## Relationships / edges

- `tailscale-query` via `tailscale-status` — **consumes** parsed self+peer
  status snapshots (the sole input); shared with `completion-router` as a
  co-consumer (scaffold/contracts/tailscale-status.md).
- any subscriber (ccd, org, mesh's in-process observability hub) via
  `network-events` — **produces** the topology + self-connectivity WS feed on
  topic `net.topology` (scaffold/contracts/network-events.md).
- `pubsub-relay` (in-process sibling lib) — the transport `network-events` rides:
  typed `Envelope<NetworkEvent>` publish + retained-snapshot semantics. NOT a
  contract edge (in-process lib-to-lib inside the mesh crate), but a hard design
  dependency; the wire envelope type is the `pubsub-protocol` / `types::pubsub`
  domain.
- `types` — `NetworkEvent`, `TopologySnapshot`, `PeerRef`, `SelfConnectivity`,
  `SelfOfflineCause`, `BackendState` are ≥2-crate / named-contract types, so
  they belong in `types` (proposed `types::net` module) per that crate's
  inclusion test — flagged for the `types` owner / Contract Harmonizer.

## Nesting

Parent: mesh | Children: none. Lives in `lib/mesh/src/topology/` (poll loop +
state machine + self-detector). mesh-core spawns its poll task at boot and hands
it the `tailscale-query` handle and a publish handle into `pubsub-relay`.
Consumers reach the feed only over the mesh WS pub/sub, so no crate depends on
`lib/mesh` to consume topology.

## Thoroughness level

**implementation-ready** — the poll loop, the four-outcome self-detector, the
peers-unknown-on-self-offline guarantee, the asymmetric-debounce state machine,
the recovery-diff (membership-only) rule, the cadence/deadline/debounce
defaults, and node-local-vantage provenance are all specified. The only deferred
piece is the exact field-level wire schema, which is the Contract Harmonizer's
(`network-events` / `tailscale-status`) — proposed below.

## Assigned design-depth

Single strong-model (Opus) Component-Designer pass, grounded on the wave-1
`network-topology.md`, `mesh.md` concern 4, the `tailscale-query` design, and
the real `lib/mesh/src/discovery.rs::TailscaleDiscovery` stub.

## Suggested fill-model

implementation-ready + low–moderate complexity → **mid model (Sonnet)**. It is a
bounded poll loop + a small explicit state machine with named defaults; the two
subtle invariants (peers-unknown-under-self-offline, asymmetric debounce) are
spelled out here, so no design work remains for the Filler. Depends only on
`tailscale-query`'s fake impl and `pubsub-relay`'s publish handle for its
conformance tests.

---

## Proposed contracts (wave 2)

Two pairs are assigned to this module by wave2-plan §3a: `tailscale-status`
(consumer side) and `network-events` (producer side). Proposals only — the
per-pair reconciliation round harmonizes both sides.

### Contract `tailscale-status` (I am a consumer; co-consumer: completion-router)

**Purpose.** Hand `network-topology` (and `completion-router`) a parsed,
version-decoupled snapshot of self + peer Tailscale status. Synchronous
(per `tailscale-query`'s charter); I wrap it in `spawn_blocking` + `timeout`.

**Struct sketch** (Rust-flavored; final home decoupled from tailscale's private
serde structs per `tailscale-query` concern 3):

```rust
struct TailscaleStatus {
    sampled_at:   DateTime<Utc>,     // when tailscale-query captured it
    backend_state: BackendState,
    tailnet_name: Option<String>,
    self_node:    PeerStatus,
    peers:        Vec<PeerStatus>,
    health:       Vec<String>,       // tailscaled health warnings, passthrough
}

struct PeerStatus {
    stable_id:     String,           // Tailscale StableNodeID — identity key
    host_name:     String,
    dns_name:      String,
    tailscale_ips: Vec<IpAddr>,
    tags:          Vec<String>,      // ACL tags, e.g. "tag:substrate"
    os:            Option<String>,
    online:        bool,
    is_self:       bool,
    last_seen:     Option<DateTime<Utc>>,
    relay:         Option<String>,   // DERP relay region, informational
}

#[non_exhaustive]
enum BackendState { NoState, NeedsLogin, Stopped, Starting, Running, Unknown(String) }
```

**Error cases** (the `Result<TailscaleStatus>` failure taxonomy I must handle
as self-offline observations, cause-mapped per concern 1):

- `NotInstalled` → cause `TailscaleUnavailable`.
- `DaemonUnreachable` (socket refused) → `TailscaleUnavailable`.
- `Timeout` (I impose the deadline) → `PollTimeout`.
- `ParseError(String)` → treated as a failed poll (offline observation); repeated
  parse errors likely signal a tailscale-version/JSON drift — surfaced in logs,
  not a distinct event (kept boring).
- `CommandFailed { code, stderr }` → offline observation, cause
  `TailscaleUnavailable`.

**Version-sensitivity.** `BackendState` carries an `Unknown(String)` fallback so
a new tailscale backend state never breaks parsing. `PeerStatus` field additions
are non-breaking to me (I read a subset: `stable_id`, `online`, `dns_name`/
`host_name`, `tags`). I key peer identity on `stable_id` (survives IP churn),
NOT on `tailscale_ips` or `host_name`. Note a naming edge: `types::NodeId` today
is the *device name* string used as mesh's routing key, whereas topology identity
is the *stable node key*; I expose both (`node_id` = name for cross-referencing
service-registry/completion-router, `stable_id` = identity) — flagged for the
harmonizer.

### Contract `network-events` (I am the producer; subscribers: ccd, org, mesh hub)

**Purpose.** A live, provenance-tagged WS feed of tailnet transitions and
self-connectivity for any subscriber, delivered as `pubsub-relay` envelopes on
topic `net.topology`, with a retained snapshot for snapshot-on-connect.

**Message sketch** (rides `Envelope<NetworkEvent>` from the `pubsub-protocol` /
`types::pubsub` domain; proposed `types::net` module):

```rust
// Provenance-first header, present on every event (node-local vantage):
struct Provenance { observer: NodeId, observed_at: DateTime<Utc>, seq: u64 }

#[non_exhaustive]
enum NetworkEvent {
    // RETAINED on the topic — replayed to every new subscriber:
    Snapshot   { prov: Provenance, topology: TopologySnapshot },
    // transient deltas:
    PeerJoined { prov: Provenance, peer: PeerRef },
    PeerLeft   { prov: Provenance, peer: PeerRef },
    PeerOnline { prov: Provenance, peer: PeerRef },
    PeerOffline{ prov: Provenance, peer: PeerRef },
    SelfOffline{ prov: Provenance, cause: SelfOfflineCause },
    SelfOnline { prov: Provenance, topology: TopologySnapshot }, // fresh full state
}

struct TopologySnapshot {
    self_state: SelfConnectivity,
    peers:      Vec<PeerRef>,      // empty/Unknown-marked while self offline
    captured_at: DateTime<Utc>,
}

struct PeerRef {
    node_id:   NodeId,            // device name (routing/cross-ref key)
    stable_id: String,           // Tailscale StableNodeID (identity)
    host_name: String,
    tailscale_ips: Vec<IpAddr>,
    tags:      Vec<String>,
    state:     PeerState,
}

#[non_exhaustive]
enum PeerState { Online, Offline, Unknown }     // Unknown only while self-offline

#[non_exhaustive]
enum SelfConnectivity { Online, Offline { cause: SelfOfflineCause }, Unknown }

#[non_exhaustive]
enum SelfOfflineCause {
    TailnetUnreachable,               // daemon up, no DERP/control
    BackendNotRunning(BackendState),  // Stopped / NeedsLogin / NoState / Starting
    TailscaleUnavailable,             // binary/daemon missing
    PollTimeout,                      // poll exceeded deadline
}
```

**Behavioral guarantees the contract must state (not just the shapes):**

1. **Peers-unknown-under-self-offline:** between a `SelfOffline` and the next
   `SelfOnline`, NO `Peer*` events are emitted, and consumers MUST treat all
   peer state as `Unknown`. The retained `Snapshot` during this window carries
   `self_state = Offline{..}` and peers marked `Unknown`.
2. **Recovery replays as a fresh snapshot + membership-only deltas:**
   `SelfOnline` carries a full `TopologySnapshot`; any per-peer deltas emitted
   immediately after are `PeerJoined`/`PeerLeft` only (membership), never
   online-substate churn accrued during the blackout.
3. **Asymmetric debounce is observable, not hidden:** down-events are delayed by
   `down_confirm_polls`; up-events are immediate. (A consumer will never see a
   sub-`down_confirm` flap.)
4. **Ordering / gap detection:** `seq` is monotonic per `observer`; a subscriber
   detecting a `seq` gap should treat its view as stale until the next retained
   `Snapshot`.

**Error cases.**

- Remote relay unreachable → best-effort, at-most-once for transient deltas;
  the retained `Snapshot` + `seq`-gap rule is the recovery path (a subscriber
  that missed deltas resyncs from the next retained snapshot). In-process
  delivery (mesh's own hub) does not drop.
- `network-topology` itself cannot fail the edge from the producer side beyond
  ceasing to publish; a stalled poll loop is surfaced as its own health/surface
  schema, not as a `NetworkEvent`.

**Version-sensitivity.** `NetworkEvent`, `PeerState`, `SelfConnectivity`,
`SelfOfflineCause`, `BackendState` are all `#[non_exhaustive]` so new
variants (e.g. a future `PeerDegraded`, or netcheck-derived latency events once
`tailscale-query` grows `netcheck()`) are additive; subscribers match with a
catch-all. The retained-`Snapshot`-per-topic requirement is a capability this
edge needs from `pubsub-relay` — if the relay cannot retain, `network-topology`
falls back to answering a `topology.snapshot` request on subscribe (flagged as
the less-boring alternative).

**Addressing note (INTENT #59, two addressing classes).** Because the feed is a
node-local vantage, it is addressed like any per-node surface: a subscriber to
its *local* mesh gets its local node's `net.topology` by default; "topology as
seen by node N" is reachable via mesh-core's specific-node addressing class.
The `observer` field lets a fan-in consumer keep multiple nodes' vantages
distinct.
