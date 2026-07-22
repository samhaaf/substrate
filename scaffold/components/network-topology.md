# network-topology

**Status:** UPDATED (wave-3 ledger batch-2). Two folds land here on top of the
wave-2 design: (F9a) **`tailscale-query` is absorbed** as this component's
internal `query` submodule — no longer its own crate; and (INTENT #116) the
**mesh time authority's measurement half** is folded in as an internal
`topology/time` submodule (neighbor-ping offset correction over the daemon
links). The wave-2 tailnet-transition-feed design below is unchanged in
substance; the two new submodules are additive.
**Nesting:** child of mesh — internal lib `lib/mesh::topology` (module
`lib/mesh/src/topology/`), now with `topology/query/` (absorbed tailscale
querying) and `topology/time/` (offset measurement) submodules. No crate
boundary crossed: consumers subscribe over the mesh WS pub/sub, never link this
module; the corrected clock is read through a Ring-0 handle (concern 6).

## Charter

`network-topology` is the single mesh-internal lib that owns **what this mesh
daemon can observe of the tailnet, and how it keeps its clock aligned with the
fleet**. Three concerns now live under one roof, because all three ride the same
peer graph and the same node-local-vantage discipline:

1. **The tailnet transition feed** (the original charter) — turns Tailscale
   status snapshots into a **live, typed feed of tailnet transition events**
   published on the `net.topology` topic (`network-events` edge): peers
   joining / leaving / going online / going offline, and — the reason it earns
   its own component — **this device's own loss of, and recovery of, connection
   to the tailnet** (`self_offline` / `self_online`) as first-class events. It
   owns the poll loop, the snapshot-diff state machine, the debounce/cadence
   policy, the self-connectivity detector, and the retained-snapshot-on-connect
   behavior.
2. **The Tailscale query surface** (absorbed from `tailscale-query`, F9a) — the
   `TailscaleQuery` trait, the shell-out to `tailscale status --json`, the
   private-serde-vs-public-types split, the catchable `TailscaleError` taxonomy,
   `RealTailscale`, and the scriptable `FakeTailscale` fixture. Formerly its own
   L0 crate; now the internal `query` submodule this component consumes directly
   (see "Absorbed submodule" below). network-topology was already its **sole**
   consumer, so the fold is a sole-producer-into-sole-consumer merge.
3. **The mesh time-offset measurement engine** (INTENT #116) — the neighbor-ping
   loop that measures this device's clock offset against its tailnet neighbors
   over the daemon links and writes a fleet-converged correction into mesh's
   Ring-0 clock handle (see concern 6). The offset feeds everything
   timestamp-ordered (`replicated-kv`'s HLC wall-clock input, `locks`' semaphore
   timestamps, lease expiry, provenance `emitted_at`).

It does **NOT**: make routing decisions or maintain a load/health table (that is
`completion-router`'s `NodeRegistry`, a deliberately separate concern — the
router's fleet membership comes from `resolve_all("inference")` + this module's
feed, never its own tailscale scan); own the transport of the feed (that is
`pubsub-relay`, which relays the envelopes across services and nodes); nor
**own the clock register itself** — it is the sole *writer* of the offset into a
Ring-0 clock handle mesh-core places beside `LocalStore` (concern 6), not the
holder of that handle. Its output is **a node-local vantage** — "what THIS mesh
daemon can currently see of the tailnet, and how far its clock sits from its
neighbors'" — never a claim of global truth; that framing is load-bearing for
both the self-offline semantics and the time-offset semantics below.

## Absorbed submodule: `query` (was `tailscale-query`, F9a)

The wave-2 `tailscale-query` component (a standalone L0 crate,
`substrate-tailscale`) is **folded in as this component's internal `query`
submodule** (`lib/mesh/src/topology/query/`), per the wave-3 ledger
consolidation F9a (seed-bishop critic-loop ACCEPT[BORING]). The tombstone with
the full moved-content record is `scaffold/components/tailscale-query.md`.

**Why it folds in (the boring justification).** network-topology was already the
**sole** consumer of the query surface (completion-router was dropped as a
co-consumer at the wave-2 contract round). A one-producer / one-consumer edge
across a crate boundary buys nothing but a crate; collapsing it removes a crate
and a cross-crate `map_err` seam without losing a single type or guarantee. The
`tailscale-status` "contract" was already flagged as a compiled-in Rust API
surface (no bytes cross `:3649`), so it was never a wire edge to begin with.

**What the fold preserves exactly** (all of `tailscale-query`'s design survives,
unchanged in substance, now as the `query` submodule):

- The **`TailscaleQuery` trait** with `status()` as the only method today, and
  the **extensibility rule**: new subcommands (`netcheck`, `ping`, `whois`, …)
  arrive as NEW trait methods with a default `Err(Unsupported)` body, never
  mutating `status()`'s shape. (Extensibility is in the method set, not a
  god-struct — INTENT #24 made structural.)
- The **private-serde subset** decoupled from the public structs, `#[serde(
  default)]` on every field, no `deny_unknown_fields` — Tailscale JSON churn is
  absorbed inside the submodule; only a structurally-missing `Self`/`BackendState`
  surfaces as `Parse`.
- The stable public structs (`StatusSnapshot`, `PeerStatus`, `BackendState`) and
  the load-bearing catchable **`TailscaleError` taxonomy** (`BinaryNotFound` /
  `DaemonNotRunning` / `NotLoggedIn` / `Timeout` / `Subprocess` / `Parse` /
  `Unsupported`) that concern 1's self-offline detector maps onto.
- `RealTailscale` with **binary discovery** (override → `TAILSCALE_BIN` → `PATH`
  → macOS bundle → Linux path, fail-fast `BinaryNotFound`) and a **hard
  wall-clock timeout** (kill-child-on-hang → `Timeout`).
- **`FakeTailscale`** — the scriptable fixture that makes the whole L1 mesh layer
  testable without a tailnet (`fixed` / `script` / `from_json_fixture` /
  `failing`). It stays the linchpin of network-topology's conformance tests.

**What changes (and only this).** The crate `substrate-tailscale` disappears;
the code moves to `lib/mesh/src/topology/query/`. Two consequences:

- The types (`StatusSnapshot`, `PeerStatus`, `BackendState`, `TailscaleError`)
  are now compiled inside `mesh`. They were single-crate-scoped already (only
  network-topology read them), so they do **not** need to move to `types`; the
  `SubstrateError::Tailscale(String)` mapping variant (flagged for the `types`
  owner) stays, since mesh's workspace `Result` still wraps them.
- The operator's verbatim "**a crate just to query Tailscale**" (INTENT #24) and
  the "cleanest shared-library-extraction candidate" framing (INTENT #25) are
  **superseded by consolidation** — BUT the extraction seam is **preserved**: the
  `query` submodule keeps its zero-mesh-dependency internal boundary (it imports
  nothing from the rest of `mesh`; only `serde`/`serde_json`/`chrono`/`thiserror`
  + the timeout helper), so re-promoting it to a standalone crate later is a
  one-move `cargo new` + path change, not a redesign. This honours the
  design-around rule "choose the placeholder that can be un-chosen with a one-line
  edit" and keeps the fold reversible if the operator objects to losing the crate.

## Primary design concerns

The component is L–M complexity, but four of its concerns are genuinely subtle
and are why it is not clumped into `mesh-core` or `completion-router`. Concerns
1–3 are the tailnet-feed concerns (wave-2, unchanged); concern 6 is the folded-in
time-offset engine (INTENT #116). The `query` submodule's own concerns
(extensibility, private-serde split, subprocess discipline, the scriptable fake)
live in the tombstone and are summarised above.

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

### 6. Mesh time authority — the neighbor-ping offset-measurement engine (INTENT #116)

INTENT #116 (verbatim-grade): *"harden time itself inside mesh … enforce UTC
sync across devices; possibly timestamps are acquired FROM the local mesh daemon
… mesh can handle its own device-specific offset by pinging its neighbors and
keeping the times in sync … a consistent timestamp-based race-condition
management system built into the mesh, as hardened as is physically possible."*
This folds in **here** because the measurement rides the exact peer graph and
daemon links this component already tracks, reuses its poll/cadence and
node-local-vantage discipline, and — critically — reuses its self-offline state
machine (concern 1/2) for the "can't-observe" case. `replicated-kv`'s **HLC
ratchet stays exactly as designed** (its concern 1); this concern supplies the
*corrected wall-clock input* the ratchet already asked mesh for.

**The register / control-loop split (the load-bearing structural decision).**
Time authority is deliberately split into two halves so there is **no layering
inversion** (Ring-0 `LocalStore` must never depend on an L2 topology module):

- **The clock register — `MeshClock` (Ring-0, not owned here).** A tiny,
  dependency-free primitive mesh-core places beside `LocalStore`/`PeerTransport`:
  an `AtomicI64` corrected-offset plus `now_utc()` (= `SystemTime::now()` +
  offset), a monotonic component for the HLC, and `offset_ns()`. It has no async
  and no workspace deps, so Ring-0 code (replicated-kv HLC, locks' semaphore
  timestamps, lease expiry, provenance `emitted_at`) can *read* it directly. This
  is mesh-core §9's `TimeAuthority`, narrowed to just the register.
- **The control loop — `topology/time` (owned here, the fold).** The periodic
  neighbor-ping engine that estimates this node's offset and is the **sole
  writer** of `MeshClock`. mesh-core §9 sketched TimeAuthority as one monolith;
  this fold splits off its measurement half into topology, because that half *is*
  a topology concern (it needs the live peer graph + the daemon links). See
  "Proposed contracts (wave 3)" for the seam and the mesh-core reconciliation.

**How the offset is measured (NTP-style, over the daemon links).** On a separate
cadence (`topology.timesync_interval_secs`, default **30s** — offset drifts
slowly), for each **online tag:substrate neighbor** the live topology already
knows about, exchange a timestamped ping/pong over the inter-daemon `PeerLink`
(a dedicated control frame — NOT pub/sub, whose relay latency would poison the
measurement; see proposed `time-sync` frame). Four timestamps per exchange
(`t1` local-send, `t2` remote-recv, `t3` remote-send, `t4` local-recv) yield the
standard estimators:
- `offset ≈ ((t2 − t1) + (t3 − t4)) / 2`, `round_trip_delay = (t4 − t1) − (t3 − t2)`.
- Keep the **lowest-delay** sample per neighbor (asymmetric/queued links are
  rejected as noise), aggregate across neighbors, and write a **slew-bounded**
  update into `MeshClock` (clamp per-update change so one bad sample can't yank
  the clock). Persist last-good offset across restarts.

**"As hardened as physically possible" (the concrete hardening measures).**
- **Lowest-delay filtering + slew clamping + bad-neighbor rejection** (a neighbor
  whose implied offset is a wild outlier vs the rest is dropped for that round).
- **Never regress the ordering clock.** The corrected wall-clock feeds the HLC
  *ratchet*, which already guarantees issued `Version`s never go backwards even
  if a correction nudges wall-time down — so a downward slew is safe for causal
  ordering by construction. `MeshClock::now_utc()` is a *display/correction*
  clock; the HLC ratchet remains the authority for issued versions (respecting
  replicated-kv's design).
- **Self-offline freeze (reuses concern 1/2).** When self is offline (no tailnet),
  no neighbor is reachable → the engine **freezes the offset at last-good** and
  emits nothing new, exactly the "node-local vantage, can't observe → don't
  fabricate" rule the peer-state machine already enforces. On `self_online` it
  resumes measuring and re-converges. This cohesion is a second reason the engine
  belongs in this component.
- **Drift surfacing.** Per-node offset/drift is published on the surface schema
  (mesh-core §9); a node past a soft threshold raises a dashboard drift alarm.
  A hard-threshold *refuse-timestamp-sensitive-ops* policy is **listed as an open
  nuance** (OQ-13), not decided here.

**Timestamps "acquired FROM the local mesh daemon."** In-process Ring-0 consumers
read `MeshClock` directly (compiled-in — that already satisfies "from the daemon"
for the libs that matter to race-conditions). Whether **out-of-process** services
also get daemon-issued timestamps via a `mesh.now_utc()` chassis/mesh-client call,
or just read their own node's `MeshClock`, is a **fill-time OPEN nuance** (OQ-13;
mesh-core §9 defers it identically).

## Relationships / edges

- `topology/query` submodule (was `tailscale-query`) via the now-**internal**
  `tailscale-status` surface — **consumes** parsed self+peer status snapshots (the
  sole input to concern 1). This is now an intra-component module boundary, not a
  crate edge; the surface is documented in `scaffold/contracts/tailscale-status.md`
  (party line updated to network-topology-internal).
- any subscriber (cc, org, mesh's in-process observability hub) via
  `network-events` — **produces** the topology + self-connectivity WS feed on
  topic `net.topology` (scaffold/contracts/network-events.md).
- `pubsub-relay` (in-process sibling lib) — the transport `network-events` rides:
  typed `Envelope<NetworkEvent>` publish + retained-snapshot semantics. NOT a
  contract edge (in-process lib-to-lib inside the mesh crate), but a hard design
  dependency; the wire envelope type is the `pubsub-protocol` / `types::pubsub`
  domain.
- `mesh-core` (Ring-0, in-process) — **writes** the fleet-converged offset into
  the `MeshClock` register mesh-core holds (concern 6); **borrows** a `PeerLink`
  handle from `PeerTransport` to send the `time-sync` control frames to
  neighbors. Both are compiled-in seams, not contract edges — but they are hard
  design dependencies (see Proposed contracts for the seam shapes and the
  mesh-core §9 reconciliation).
- `types` — `NetworkEvent`, `TopologySnapshot`, `PeerRef`, `SelfConnectivity`,
  `SelfOfflineCause`, `BackendState` are ≥2-crate / named-contract types, so
  they belong in `types` (proposed `types::net` module) per that crate's
  inclusion test — flagged for the `types` owner / Contract Harmonizer. The
  `time-sync` frame body (`TimePing`/`TimePong`) is wire-crossing (daemon↔daemon)
  and belongs in the `types` transport/frame domain alongside the mesh-transport
  frames — flagged for `types` + `mesh-transport` owners. The absorbed
  `query` submodule's structs (`StatusSnapshot`, `PeerStatus`, `TailscaleError`)
  stay **local to `mesh`** (single-crate-scoped; only network-topology reads
  them), NOT promoted to `types`.

## Nesting

Parent: mesh | Children: none (submodules `topology/query/` and `topology/time/`
are internal, not components). Lives in `lib/mesh/src/topology/`:
- `topology/` — poll loop + snapshot-diff state machine + self-connectivity
  detector (concerns 1–3);
- `topology/query/` — the absorbed Tailscale query surface (was `substrate-
  tailscale`); imports nothing else from `mesh`, preserving a one-move
  crate-extraction seam (see Absorbed submodule);
- `topology/time/` — the neighbor-ping offset-measurement engine (concern 6).

mesh-core spawns the poll task **and** the time-sync task at boot, handing them
the (internal) query handle, a publish handle into `pubsub-relay`, a `PeerLink`
handle from `PeerTransport`, and the `MeshClock` **writer** half. Consumers reach
the feed only over the mesh WS pub/sub; Ring-0 code reads the corrected clock via
the `MeshClock` **reader** handle. No crate depends on `lib/mesh` to consume
topology.

## Thoroughness level

**implementation-ready** for the tailnet feed (concerns 1–5) and the absorbed
`query` submodule (its full surface survives in the tombstone): the poll loop,
the four-outcome self-detector, the peers-unknown-on-self-offline guarantee, the
asymmetric-debounce state machine, the recovery-diff (membership-only) rule, the
cadence/deadline/debounce defaults, and node-local-vantage provenance are all
specified. **Approach-sketched** for the time-offset engine (concern 6): the
register/control-loop split, the NTP-style estimators, the hardening measures,
the self-offline freeze, and the cadence knob are pinned; the convergence
algorithm's exact filter/smoother and the OQ-13 nuances are deferred (see below).
The only deferred wire schema is the Contract Harmonizer's (`network-events`,
now-internal `tailscale-status`, and the new `time-sync` frame).

## Assigned design-depth

Single strong-model (Opus) Component-Designer pass (wave-2), extended in wave-3
by the ledger folds (F9a query absorption; INTENT #116 time engine), grounded on
the wave-1 `network-topology.md`, `mesh.md` concern 4, the absorbed
`tailscale-query` design, `mesh-core.md` §9, `replicated-kv.md` concern 1, and
the real `lib/mesh/src/discovery.rs::TailscaleDiscovery` stub.

## Suggested fill-model

Feed + query submodule: implementation-ready + low–moderate complexity → **mid
model (Sonnet)** — a bounded poll loop + a small explicit state machine + a
fixture-backed shell-out, with the two subtle invariants
(peers-unknown-under-self-offline, asymmetric debounce) spelled out, so no design
work remains. Time engine (concern 6): once mesh-core pins the `MeshClock`
register and the `time-sync` frame lands, the fill is a bounded NTP-style loop —
**mid model (Sonnet)**, but the OQ-13 nuances must be resolved (operator round)
before the hardening policy is finalised. Conformance tests depend only on the
internal `FakeTailscale`, `pubsub-relay`'s publish handle, and a fake `PeerLink`
that scripts pong timestamps.

---

## Proposed contracts (wave 3)

Three contract-shape changes originate here. All three are flagged for the named
owners / the Contract Harmonizer; nothing below decides a PARKED question.

### 1. `time-sync` peer-link control frame *(new; owner: `mesh-transport` / `mesh-core`)*

The offset engine needs a **precise, low-latency, symmetric** request/response
between adjacent daemons. It must **NOT** ride pub/sub (the relay's queue +
fan-out adds variable latency that poisons offset estimation) — it rides the
inter-daemon `PeerLink` directly as a dedicated control-frame pair:

```rust
// proposed to the mesh-transport / types frame domain (wire-crossing, daemon↔daemon)
struct TimePing { nonce: u64, t1_ns: i128 }              // t1 = sender MeshClock at send
struct TimePong { nonce: u64, t1_ns: i128,               // echoed
                  t2_ns: i128, t3_ns: i128 }             // t2 = responder recv, t3 = responder send
```

The responder stamps `t2`/`t3` from its own `MeshClock`; the initiator computes
offset + delay from `(t1,t2,t3,t4)`. This is the first inter-daemon control frame
network-topology needs that is **not** tailscale-derived, so the frame belongs in
the mesh-transport frame registry, not in this component. **Flag:** mesh-transport
owns whether this is a distinct top-level frame or a variant on an existing
peer-link control channel; network-topology owns only the timestamp semantics.

### 2. `MeshClock` register + writer seam *(reconciles `mesh-core` §9; owner: `mesh-core`)*

Splits mesh-core §9's monolithic `TimeAuthority` into a **register** (mesh-core)
and a **control loop** (this component). Proposed handle shape:

```rust
// lives beside LocalStore/PeerTransport (Ring-0); mesh-core owns placement
struct MeshClock { offset_ns: AtomicI64, /* + persisted last-good */ }
impl MeshClock {
    fn now_utc(&self) -> DateTime<Utc>;   // SystemTime::now() + offset — READ by kv/locks/leases/provenance
    fn now_monotonic(&self) -> u64;       // HLC monotonic component
    fn offset_ns(&self) -> i64;           // surfaced on the surface schema (drift)
}
struct MeshClockWriter(Arc<MeshClock>);   // handed ONLY to topology/time; the sole writer
impl MeshClockWriter { fn slew_to(&self, target_offset_ns: i64 /* slew-clamped inside */); }
```

**Flag for mesh-core owner + harmonizer:** mesh-core §9 currently describes the
whole `TimeAuthority` (offset estimate + neighbor pings + `now_utc()`) as
"a design item for mesh-core's fill." That neighbor-ping estimation half is now
**owned here** (the fold); mesh-core §9 should be updated to keep only the
register + reader placement and delegate estimation to `topology/time`. I do not
edit `mesh-core.md` (out of ownership) — this seam is the reconciliation record.

### 3. `tailscale-status` is now network-topology-internal *(owner: this component; updated)*

The `tailscale-status` surface is no longer a crate edge — producer (`query`
submodule) and consumer (topology state machine) are the same component. The
contract file is updated to a **network-topology-internal module-boundary**
record: it documents the `query` submodule's public trait+struct surface (the
extraction seam), no longer a two-party edge. Party line updated in
`scaffold/contracts/tailscale-status.md`; the full moved content lives in the
`tailscale-query.md` tombstone.

## OQ-13 open nuances — LISTED, not decided (INTENT #116 / ledger B.2 OQ-13)

Per the design-around rules (§C, "as hardened as physically possible" is sketched
as a fill-time direction, the authority-of-time nuances stay open), these remain
the operator's to settle in the closing round; the design above stays flexible so
each is a one-knob change:

1. **The true-UTC anchor.** Does the fleet track *true* UTC (one node — the
   laptop with real NTP — is the anchor and others converge toward it), or only a
   *shared internal* view that may collectively drift? Which node anchors, and how
   it is designated vs observed, is undecided. (No authority node is implied —
   see nuance 5.)
2. **Daemon-issued timestamps for out-of-process services.** In-process Ring-0
   libs read `MeshClock` directly. Whether external services get timestamps via a
   `mesh.now_utc()` chassis/mesh-client RPC, or just read their own node's clock,
   is deferred (mesh-core §9 defers it identically).
3. **The drift refuse-policy.** Past a *hard* offset threshold, does a node
   **refuse** timestamp-sensitive operations, or only raise a dashboard alarm?
   Only the soft-threshold alarm is designed; the hard-refuse policy is open.
4. **Convergence algorithm specifics.** Neighbor weighting, asymmetric-delay
   handling, and Byzantine/bad-clock neighbor rejection are sketched NTP-style
   (lowest-delay + outlier-drop + slew clamp); the exact filter/smoother/weighting
   is fill-time.
5. **Interaction with the PARKED authority-node question (OQ-1).** Time authority
   must **not** become a hidden central authority: every node self-measures
   against neighbors; **no node "blesses" time**. This design threads **no**
   authority dependency (honouring the OQ-1 design-around rule); if a future
   dedicated-authority discussion changes that, it is additive, not a rewrite.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them.

- `tailscale-status` (**now network-topology-internal**, wave-3 F9a) — was the
  `tailscale-query` (crate) → network-topology (sole consumer) edge; both parties
  are now this component (the `query` submodule → the topology state machine), so
  the file is a **module-boundary record**, not a crate edge. The Rust API surface
  it documents (the `TailscaleQuery` trait, `StatusSnapshot`/`PeerStatus`/
  `BackendState`, and the catchable `TailscaleError` taxonomy the self-offline
  detector maps onto) is unchanged — it is now the `query` submodule's public
  surface / extraction seam. → `scaffold/contracts/tailscale-status.md` (party
  line updated); full moved content in the `tailscale-query.md` tombstone.
  - Surface names (unchanged, producer's names): `StatusSnapshot`, `captured_at`,
    `PeerStatus.id`; self folded into `peers`-shaped `PeerStatus` via `self_node`
    + `is_self`; `relay: Option<String>` kept.
  - Participation note: `status()` is synchronous; the poll loop wraps it in
    `spawn_blocking` + its own `timeout` (now an intra-component call).
- `network-events` (network-topology → subscribers cc, org, and mesh's
  in-process observability hub) — the provenance-tagged `net.topology` feed
  (retained `Snapshot` + `Peer*`/`SelfOffline`/`SelfOnline` deltas, the
  peers-unknown-under-self-offline and seq-gap rules).
  → `scaffold/contracts/network-events.md` (authored verbatim from this
  module's proposal; gateway dropped as a subscriber at the mesh merge)
  - Component-side note: the feed is a node-local vantage; "topology as seen
    by node N" is reachable via mesh-core's specific-node addressing class,
    and `prov.observer` keeps fan-in vantages distinct.
