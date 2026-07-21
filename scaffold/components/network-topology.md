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
(that is `completion-router`'s `NodeRegistry`, a deliberately separate concern —
and since the contract round, network-topology is `tailscale-status`'s SOLE
consumer; the router's fleet membership comes from `resolve_all("inference")` +
this module's feed, never its own tailscale scan); own the transport of the feed
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
  status snapshots (the sole input); network-topology is the SOLE consumer of
  this edge since the contract round (completion-router dropped — see
  scaffold/contracts/tailscale-status.md Reconciliation notes).
- any subscriber (cc, org, mesh's in-process observability hub) via
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
(`network-events` / `tailscale-status`) — authored in scaffold/contracts/.

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

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them.

- `tailscale-status` (tailscale-query → network-topology; network-topology is
  the **sole consumer** — completion-router was DROPPED as a party at the
  contract round) — the compiled-in Rust API surface handing this module
  parsed self+peer Tailscale status plus the catchable failure taxonomy it
  maps onto self-offline causes. → `scaffold/contracts/tailscale-status.md`
  - Contract resolution supersedes the consumer-side sketch once proposed
    here: the producer's names win — `StatusSnapshot` (not `TailscaleStatus`),
    `captured_at` (not `sampled_at`), `PeerStatus.id` (not `stable_id`); self
    is folded into `peers`-shaped `PeerStatus` via `self_node` + `is_self`;
    `relay: Option<String>` was kept at this module's request.
  - Participation note: the call is synchronous per tailscale-query's charter;
    this module wraps it in `spawn_blocking` + its own `timeout`.
- `network-events` (network-topology → subscribers cc, org, and mesh's
  in-process observability hub) — the provenance-tagged `net.topology` feed
  (retained `Snapshot` + `Peer*`/`SelfOffline`/`SelfOnline` deltas, the
  peers-unknown-under-self-offline and seq-gap rules).
  → `scaffold/contracts/network-events.md` (authored verbatim from this
  module's proposal; gateway dropped as a subscriber at the mesh merge)
  - Component-side note: the feed is a node-local vantage; "topology as seen
    by node N" is reachable via mesh-core's specific-node addressing class,
    and `prov.observer` keeps fan-in vantages distinct.
