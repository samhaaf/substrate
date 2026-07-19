# completion-router

**Status:** WAVE-2 REFIT of the wave-1 implementation-ready design (the frozen
Design-Mesh synthesis: Tier-1/Tier-2 model-affinity routing, least-loaded
tie-break, spill + Tier-3 deferred, a streaming-capable `forward()` contract).
The wave-1 *routing algorithm* is unchanged and re-affirmed; wave-2 re-grounds
the module onto the new substrate — its membership/endpoint discovery moves off a
bespoke `tailscale status` loop and onto **service-registry** (+ **network-topology**
as the liveness overlay); its per-node health/load feed becomes **pubsub-primary
(`inference-events`) with `node-state-poll` demoted to reconcile/bootstrap**; the
two addressing classes (INTENT #59) become the router's public API; mid-stream
node-failure behavior is pinned (**v2 = fail, no transparent failover**); and
benchmark node-pinning is designed in. **Nesting:** internal lib of mesh
(`lib/mesh::router` + `balancer` + the `NodeRegistry`), **Ring 4** in mesh-core's
internal layering (rides `service-registry` + `network-topology` for membership,
`PeerLink`/the node-to-node data path for forwarding). Never a standalone service
(INTENT #54).

## Charter

`completion-router` is the **transparent completion data plane**: it makes the
inference *fleet* look like a single node behind the mesh's `:3649` front door.
It resolves the fleet's membership and endpoints from the substrate, tracks each
node's live routing health/load/model-inventory in its own `NodeRegistry`
projection, picks a node per `/v1/` request by model affinity (least-loaded
tie-break), and **forwards the OpenAI-compatible REST+WS surface byte-transparently**
so a caller cannot tell node-direct from mesh-routed. It is the concrete
implementation behind the LOCKED decision that **`resolve("inference")` returns the
mesh front door** (service-registry's `FleetAlias`): a consumer resolves `inference`
(AnyNode) → reaches its local `:3649` → mesh-core's Dispatcher hands the request to
*this* module → the router selects a node and forwards. It owns exactly two things —
**node selection** (the `NodeRegistry` health/load projection + the
`ModelAffinityBalancer`) and **forwarding** (the status/headers/streaming-body
`forward()` path + the WS stream relay + the two addressing classes as its API).

**Boundary — what it does NOT own.** It holds **no completion state**: nodes' SQLite
is the system of record; the router keeps only a rebuildable in-memory
`completion_id -> node` index (a cache, not a source of truth). It does **not own
slug addressing** (that is `service-registry` — the router is a *consumer* of it for
the `inference` fleet's endpoints). It does **not discover the tailnet** (that is
`network-topology`/`tailscale-query`; the router consumes their output, and — the
wave-2 change — no longer runs its own `tailscale status` loop). Its `NodeRegistry`
is a **distinct table** from `service-registry` (slug→endpoint+lease) and from
`network-topology` (peer on/off vantage): it is the only place live per-node
inference **load/health/inventory** lives, and the mesh-invariant that these three
tables are never conflated is load-bearing (see concern 1). It does **not admit,
queue, swap, or benchmark** — those are the node's `scheduler`/`engine`; the router
forwards and forgets. It owns no application data (`db`), no relay semantics
(`pubsub-relay`), no supervision (`supervision`).

## Primary design concerns

The wave-1 synthesis fixed the *routing algorithm* to implementation-ready; the
wave-2 concerns below are the *substrate refit* — where the state comes from, how
the addressing classes surface, and the two behaviors the operator called out
(mid-stream failure, benchmark pinning). Where a concern re-affirms wave-1 unchanged
it says so.

### 1. Three tables, never conflated — and the wave-2 re-sourcing of NodeRegistry

The mesh invariant (mesh.md concern 2; service-registry.md boundary;
network-topology.md charter) is that **`NodeRegistry` ≠ `service-registry` ≠
`network-topology`**. Wave-2 keeps that invariant but changes *which columns of
NodeRegistry the router owns vs. projects from the substrate*:

| Column | Wave-1 source | **Wave-2 source** |
|---|---|---|
| fleet **membership** (which nodes run inference) | router's own `tailscale status` tag scan | **service-registry** (per-node inference-api registrations) + **network-topology** reachability overlay |
| node **endpoint** (host:port to forward to) | tailscale IP + a hardcoded default port | **service-registry** `Endpoint{scheme,host,port,health_path}` (INTENT #36 — never a guessed port) |
| node **health/load** (running, pending, memory pressure) | `node-state-poll` (poll-only) | **`inference-events` pub/sub (primary)** + `node-state-poll` (reconcile) |
| node **model inventory** (downloaded / resident) | `node-state-poll` | `inference-events` (load/evict deltas) + `node-state-poll` (reconcile) + `types::node::NodeCapabilities.models_available` |

So the wave-2 `NodeRegistry` is a **health/load projection over a substrate-sourced
membership set**, not a discovery engine. It still exists as a distinct table
(nothing else holds live inference load), but it no longer *discovers* — it
*subscribes and reconciles*. The bespoke `tailscale status` refresh loop from
wave-1 is **deleted** (its work is already done by network-topology). This is the
single biggest structural change and directly answers the refit brief's "NodeRegistry
state now comes from replicated-kv/service-registry instead of its own discovery
loops." (The `replicated-kv` link is indirect and correct: service-registry rides
`replicated-kv`, so the router reading membership from service-registry *is* reading
replicated state — the router never touches `KvHandle` itself.)

**The membership-source decision requires a per-node inference registration that
service-registry.md as-written does NOT provide** (its concern 5 keeps the fleet
*out* of the registry and in a router-owned tag scan, storing only the `inference`
`FleetAlias`). This is a **deviation flagged as a friction point** with a concrete
reconciliation resolved (concern 8) and recorded in the authored contracts.

### 2. Node selection — the wave-1 balancer, re-affirmed, fed from the refit table

`ModelAffinityBalancer` is **unchanged from the frozen wave-1 synthesis** and
re-stated here only so the fill target is self-contained:

- **Tier-1 (resident):** nodes where the requested `model_id` is the live
  `resident_model` (read from the live health projection — types.md deliberately
  keeps the resident set in live `SystemState`, NOT in static `NodeCapabilities`).
- **Tier-2 (downloaded):** nodes where the model is downloaded but not resident
  (a swap cost; from `NodeCapabilities.models_available` + inventory events).
- **Tie-break within the chosen tier: least-loaded** by `running_count`
  (+ pending), from the live projection.
- **Spill and Tier-3 (download-then-serve) remain DEFERRED with reasons** (missing
  `effective_max_concurrent`; `download_model` is a no-op) — carried verbatim from
  wave-1; do not implement them this pass.

`select` takes enriched `NodeCandidate`s (endpoint + live load + inventory), never
bare endpoints — the enrichment is exactly the `NodeRegistry` join of {registry
endpoint} × {live health/inventory}. If the request is **pinned** (concern 4), the
balancer is bypassed entirely — pinning is resolved *above* `select`.

### 3. Health/load feed — pubsub-primary, poll-reconcile (resolving "polling vs events")

Wave-1 was poll-only (`node-state-poll` on an interval). Wave-2 resolves the
"node state polling vs pubsub events" question the refit brief raises, by making it
**both, with a clear primary**, mirroring the snapshot-then-delta discipline
network-topology and replicated-kv already use:

- **Primary (live, low-latency): `inference-events`.** The router subscribes (via
  its in-process `pubsub-relay` handle) to each fleet node's `inference.*` topic and
  updates the `NodeRegistry` projection on the load-affecting deltas — completion
  started/finished (running_count ±1), model loaded/evicted (resident + inventory),
  pause/resume. This is what lets least-loaded tie-break reflect reality within a
  hop instead of within a poll interval. The bus is **lossy by contract**
  (pubsub-relay concern 6): a dropped delta is self-correcting via —
- **Secondary (truth-restoring): `node-state-poll`.** A slow interval poll
  (default ~5–10s, and immediately **on-subscribe / on-node-join** for the
  snapshot) reconciles the projection against `GET /v1/system/state` +
  `GET /v1/models`, correcting any drift the lossy event stream accumulated. It is
  strictly **off the request path** (never polled to make a routing decision — the
  decision reads the in-memory projection).

Rationale for the split: affinity/least-loaded wants *fresh* load (→ events), but a
lossy bus cannot be the sole source of a routing-critical counter (→ periodic
digest-style reconcile). A node that stops emitting events *and* misses N polls is
evicted from the candidate set (marked unreachable), not left as a phantom
least-loaded target.

### 4. The two addressing classes ARE the router's API (INTENT #59)

mesh-core's Dispatcher resolves an `Address` and, for the `inference` fleet slug,
hands the request to this module. The router is the concrete handler for **both**
classes on the `inference` slug — this *is* the router's public API, not a separate
feature:

- **`AnyNode{inference}` (virtualized — "an inference node, don't care which"):**
  mesh-core sees `FleetAlias` → hands to `completion-router` → `NodeRegistry` +
  `ModelAffinityBalancer.select()` pick a node → forward. This is the default and
  the 99% path.
- **`Node{N, inference}` (pinned — "inference *on node N*"):** the balancer is
  bypassed; the router forwards to node N's registered inference endpoint directly.
  A pin to a node not in the live membership set fails cleanly
  (`NoSuchNode`/`NodeUnreachable`) — it is **never silently rehomed** to another
  node (that would break the pin's contract, and is the sibling of the mid-stream
  no-failover rule, concern 5).

The **external HTTP expression** of pinning is preserved from wave-1 for clients
that reach `:3649` directly with a raw `/v1` request: `?node=<id>` or the
`X-Substrate-Node` header. mesh **strips `?node=` before forwarding** to preserve
byte-transparency (wave-1, unchanged). Internally these lower to
`Address::Node{N, inference}` — the HTTP pin and the addressing-class pin are the
same path, one is just the wire spelling for non-mesh-aware clients.

### 5. Benchmark node-pinning — pinning is a first-class, reliable path (INTENT #12/#15)

Benchmark runs are **per-node, priority-0 exclusive** (benchmark.md; INTENT #12) and
the operator explicitly wants to "target specific machines" for test/benchmark
sweeps (INTENT #15/#31). The router's role: **node-pinned addressing (concern 4) is
the mechanism by which benchmark reaches an exact machine**, so pinning must be a
reliable first-class route, not a debug affordance.

- A benchmark submission uses `Node{N, inference}` (or `?node=N`) → the router
  forwards to N unconditionally, **overriding affinity** (N may not have the model
  resident, may not be least-loaded — irrelevant; the pin is authoritative).
- **The router does NOT reason about benchmark exclusivity.** Whether N is currently
  running a priority-0 benchmark and how a new pinned request is admitted/queued is
  **the node's scheduler's** concern (benchmark priority-0 exclusivity lives in
  `scheduler`). The router delivers the pinned request and forwards whatever status
  the node returns (including a 409/503 if the node's scheduler rejects due to a
  benchmark in progress). Keeping admission out of the router preserves byte-
  transparency and the thin-data-plane charter.
- Consequence for selection: because benchmark pins bypass the balancer, a
  benchmarking node is naturally *not* chosen for `AnyNode` traffic as long as its
  live load/`running_count` reflects the benchmark occupancy in the projection
  (concern 3) — no special-casing in the balancer, the load feed does the work.

### 6. Mid-stream node-failure behavior — v2 = FAIL, no transparent failover (CONFIRMED)

The confirmed v2 stance: **once a completion has started forwarding, a mid-stream
node failure is terminal — the router does NOT transparently fail over to another
node.** This is a deliberate non-goal, not an omission:

- A completion is **stateful on its node** — the `completion_id`, the KV/prefix
  cache, and the partial token stream live in that node's `engine`/SQLite. There is
  no cross-node checkpoint to resume from, so a "transparent" failover would have to
  restart the completion from scratch on another node, producing a duplicated/torn
  stream under the same `completion_id` — worse than an honest failure.
- **Pre-forward failures are retryable (spill, bounded):** if the *selected* node is
  unreachable *before* the request is accepted (connection refused / no response to
  the initial forward, before any byte of the response is produced), the router may
  re-select once against the remaining candidates and re-forward. This is the only
  retry, and it is invisible because nothing has been emitted to the client yet.
- **Post-headers / mid-stream failures are propagated:** once status+headers are
  returned or the streaming body/WS relay has begun, a node death closes the stream
  and the router surfaces a clean terminal error (a `502`/aborted body for REST, a
  close frame + error for the WS `/v1/completions/:id/stream` relay), evicts the
  node from the live set, and emits a `completion` failure event. The client re-tries
  as a new completion if it wants — with full knowledge that the prior one failed.
- **Door left open for v3:** resume-from-checkpoint (if `engine`'s KV-cache
  save/restore ever spans nodes) is the future path; explicitly out of scope now.

This mirrors the pinned-addressing rule (concern 4): the router never silently
substitutes a different node once a specific delivery has been committed.

### 7. `forward()` and the WS relay — the wave-1 contract change, on the substrate path

The wave-1 change to `forward()` stands and is now the central data-plane concern:
today's `fn forward(...) -> Result<Vec<u8>>` (see the live `lib/mesh/src/router.rs`
`SingleNodeRouter` stub, still `todo!()`) **must change to carry status + headers +
a streaming body**, because `Result<Vec<u8>>` cannot express 404/409/502/503 or
stream tokens. Two distinct paths (unchanged from wave-1):

- **`forward()` (REST):** returns `{ status, headers, body-stream }`, byte-transparent
  — the router copies bytes and status/headers through, transforming nothing
  (byte-transparency is what makes it version-agnostic w.r.t. inference's `/v1`
  surface, concern in `v1-completion-api`). Models on `bin/gateway/src/proxy.rs`
  (V1 code folded into mesh at the 2026-07-18 gateway merge).
- **WS stream relay (`/v1/completions/:id/stream`):** a *separate* path from
  `forward()` — a transparent WS upgrade + bidirectional frame relay between the
  client and the owning node, with the mid-stream failure semantics of concern 6.

**The node-to-node transport for the data plane (the substrate question):** the
router runs inside the mesh daemon (Ring 4). For a **local** pick (chosen node ==
self) it forwards over a direct local HTTP/WS to the local inference api. For a
**remote** pick it must cross the tailnet. Two options, and I take a position and
flag it:

- **Position (v2):** the router forwards the completion byte stream **node-to-node
  over a mesh-brokered channel to the target node's inference endpoint**, resolved
  from service-registry. Because this is bulk, streaming, byte-transparent traffic
  (not a control frame), it does **not** ride the `pubsub-protocol` envelope (which
  pubsub-relay explicitly disclaims — its charter says raw `/v1` bytes are the
  router's `forward()` path, not pub/sub) and does not fit mesh-transport's
  one-shot `Request/Response` frame. It needs a **streaming data channel on
  mesh-core's `PeerLink`** (a dedicated `kind` / brokered tunnel), OR a direct
  daemon→remote-inference-endpoint tunnel the local daemon opens as the mesh's own
  privileged data-plane.
- This is a **concrete ask to mesh-core** (a streaming node-to-node channel distinct
  from the control-frame mux) and a mild tension with strict single-port locality
  (INTENT #58) — resolved by noting locality governs *service↔service* addressing
  (a service never opens a socket to another service; it always hits its local
  `:3649`), whereas the router **is the mesh's own data plane** reaching the fleet
  node-to-node. Flagged as an open question / friction point for the per-pair round.

### 8. The `inference` slug reconciliation with service-registry (the load-bearing flag)

For the router to source membership+endpoints from service-registry (concern 1),
each inference node's `api` must have a resolvable **per-node** endpoint in the
registry — which service-registry.md concern 5 currently forbids (it stores only the
`inference` `FleetAlias`; per-node fleet membership is deliberately kept out). My
proposed reconciliation, to be settled in the per-pair round + a service-registry
re-touch:

- Each inference node's `api` **self-registers a per-node instance** under slug
  `inference`, keyed `(inference, node)`, addressing `NodeScoped`, with its real
  `Endpoint`. (service-registry's keyspace is already instance-keyed
  `registry/instance/inference/<node>` and its `AddressingClass` enum already lists
  "inference api" under `NodeScoped` — so the storage shape exists; only the concern-5
  policy prose conflicts.)
- The **`FleetAlias` becomes a resolve-time *policy* on the `inference` slug, not a
  stored synthetic record**, cleanly disambiguating the three reads:
  - `resolve(AnyNode{inference})` → local `:3649` front door (FleetAlias policy) →
    into this router. *(consumer-facing; unchanged externally.)*
  - `resolve_all("inference")` → the per-node `NodeScoped` instances = the router's
    fleet membership + endpoints.
  - `resolve(Node{N, inference})` → node N's real inference endpoint (pinned
    forward + benchmark pinning, concerns 4–5).
- **Fallback if the operator prefers membership stay out of the registry:** the
  router derives membership from **network-topology**'s `net.topology` feed (peers
  with `tag:inference` / `NodeRole::Inference`) and obtains endpoints from a
  lighter per-node inference registration — still substrate-sourced, still no
  bespoke `tailscale status` loop. Either way the wave-1 tag-scan loop dies.

## Relationships / edges

Contract edges (rides mesh-core's transport / the node-to-node data path on the
tailnet):

- **client / mesh ↔ inference (api)** via `v1-completion-api` — the transparent
  forward of the `/v1/` REST+WS completion surface; the router is the forwarding
  party (see scaffold/contracts/v1-completion-api.md).
- **mesh.completion-router → inference (api)** via `node-state-poll` — the
  **reconcile/bootstrap** health+inventory poll (demoted from wave-1's live-primary
  role; see concern 3) (see scaffold/contracts/node-state-poll.md).
- **mesh ← inference (api)** via `inference-events` — **NEW consumer edge (wave-2):**
  the router subscribes to per-node `inference.*` load/inventory deltas as the
  **live-primary** feed. Contract **owned by inference/api**; the router is the
  consumer and flags the load-carrying event shapes it needs (see
  scaffold/contracts/inference-events.md).
- ~~tailscale-query → completion-router via `tailscale-status`~~ — **DROPPED
  (wave-2):** the router no longer consumes tailscale-status directly (concern 1);
  network-topology becomes the sole consumer. Recommend striking completion-router
  as a party on `tailscale-status`. Flagged for the per-pair round
  (see scaffold/contracts/tailscale-status.md).

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45):

- **consumes** `service-registry::Resolver` (`resolve_all("inference")` for
  membership+endpoints; `resolve(Node{N,…})` for pins) — the wave-2 membership
  source (concern 1/8).
- **consumes** `network-topology`'s peer feed (in-process) as the reachability
  overlay / membership fallback (concern 1/8).
- **consumes** `pubsub-relay`'s subscribe handle for `inference-events` (concern 3)
  and its publish handle to emit routing/`completion`-failure events.
- **consumes** mesh-core's `PeerLink`/node-to-node data channel for remote forwarding
  (concern 7) — a concrete ask on mesh-core.
- **provides UP** the fleet-routing entry point mesh-core's Dispatcher calls for
  `AnyNode{inference}` / `Node{N, inference}` (concern 4).
- **must-not-conflate siblings:** `service-registry` (slug→endpoint) and
  `network-topology` (peer vantage) are distinct tables from this module's
  `NodeRegistry` (live load/inventory) — concern 1.

## Nesting

Parent: mesh (mesh-core) | Children: none. Modules `router`, `balancer`, `proxy`,
and the `NodeRegistry` in `lib/mesh` (`lib/mesh/src/{router,balancer,discovery→
node_registry,proxy}.rs`), Ring 4 in mesh-core's internal layering. Never a
standalone crate/service (INTENT #54). Note the wave-2 rename intent: the wave-1
`discovery.rs` (`NodeDiscovery`/`TailscaleDiscovery`) shrinks to a `node_registry`
that projects substrate state rather than discovering — the `TailscaleDiscovery`
stub is removed (its job moved to network-topology).

## Thoroughness level

**implementation-ready** for the refit — the three-table re-sourcing (concern 1),
the re-affirmed balancer (concern 2), the pubsub-primary/poll-reconcile feed
(concern 3), the two-addressing-class API (concern 4), benchmark pinning (concern
5), the mid-stream no-failover rule + bounded pre-forward spill (concern 6), and the
`forward()`/WS-relay contract change (concern 7) are all decided and specified. Two
pieces are **approach-sketched / open by dependency**: (a) the node-to-node
streaming data-plane transport (concern 7 — a concrete ask on mesh-core's PeerLink,
resolved in the per-pair round); (b) the `inference`-slug membership reconciliation
with service-registry (concern 8 — a flagged deviation needing a service-registry
re-touch + operator blessing on registering the fleet per-node). The wave-1 trait
signatures, status-code semantics, and OQ-1..OQ-11 from the original synthesis carry
forward under these unchanged.

## Assigned design-depth

**Opus** single Component-Designer pass (this file), grounded in the frozen wave-1
completion-router design (the `mesh-design-synthesis.md` report — lives in the
harness workspace `substrate-v2/reports/`, NOT in this repo, so re-grounded
against the live
`lib/mesh/src/{router,balancer,discovery,proxy}.rs` + mesh.md concern 2 which
carries the synthesis summary), the batch-1/2 designs (mesh-core Ring 4 seams,
service-registry FleetAlias/instance model, network-topology feed, pubsub-relay
envelope, replicated-kv guarantees, types::node/pubsub/event), and INTENT
#12/#15/#31/#36/#58/#59.

## Suggested fill-model

**implementation-ready + moderate complexity → mid model OK**, with three carve-outs
for a careful hand: (1) the **mid-stream failure boundary** (concern 6 — the exact
"has any byte been emitted yet?" cutover between retryable-spill and terminal-fail is
the one correctness spot); (2) the **pubsub-primary + poll-reconcile join**
(concern 3 — a lossy live feed reconciled by a periodic snapshot without a
routing-critical counter ever going phantom-negative); (3) the **node-to-node
streaming forward** (concern 7 — sequenced *after* mesh-core's PeerLink data-channel
lands). The balancer itself (concern 2) is near-transcription from the wave-1
synthesis. Sequence after `service-registry`, `network-topology`, `pubsub-relay`, and
mesh-core's Ring-0 data-channel are filled — it rides all four.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `v1-completion-api` — (client / mesh.completion-router ↔ inference.api) — transparent forward. → `scaffold/contracts/v1-completion-api.md`
- `node-state-poll` — (mesh.completion-router → inference.api) — reconcile/bootstrap poll. → `scaffold/contracts/node-state-poll.md`
- `inference-events` — (mesh.completion-router ← inference.api) — NEW consumed edge (live-primary feed). → `scaffold/contracts/inference-events.md`
- `tailscale-status` — completion-router DROPPED as a party at the contract round (network-topology is the sole consumer; fleet membership = `resolve_all("inference")`). → `scaffold/contracts/tailscale-status.md`

Also a party to (authored elsewhere / cross-cutting): `pubsub-protocol` — see `scaffold/contracts/`.

