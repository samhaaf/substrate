# completion-router

**Status:** WAVE-3 REFIT (this pass) of the WAVE-2 REFIT of the wave-1
implementation-ready design (the frozen
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

**Wave-3 addendum.** This pass re-grounds the router onto the wave-3 mediation
architecture (mesh-core, mesh-transport, chassis, pubsub-relay batches 1–2) —
a refit of this file's wave-2 design, not a rewrite; the `NodeRegistry`
projection and `ModelAffinityBalancer` (concerns 1–3) are unchanged. Four
things land: (a) inbound completions arrive as **mediated frames** — the
router is the `AnyNode{inference}`/`Node{N,inference}` handler mesh-core's
Dispatcher hands requests to (mesh-core concern 2/10), and every forwarded
exchange resolves through the #154 success/error/promise switch, never a bare
`Result`; (b) a **queued (admission-deferred) completion is modeled as a
Promise, not an error** (INTENT #152) — concern 9, below, which *simplifies*
the router's old "fleet fully busy" story from a hard, blind 503 into a
bounded, guaranteed-eventual-answer promise ticket, reusing `types::transport`'s
promise vocabulary rather than inventing a parallel one; (c) **streaming
tokens ride the relayed tunnel path** — mesh-core Concern 11's `StreamOpen/
Chunk/Close` frames over the ordinary daemon relay are the AUTHORIZED default
for both the WS token relay and node-to-node forwarding; the **direct**
off-relay `PeerLink` data channel remains **AUTHORIZATION PENDING** per OQ-30
and MUST NOT be built this wave (concern 7, updated from "flagged as an open
question" to "settled, pending only the direct-path blessing"); (d) the
**loopback-relay to peer-local inference stays LOCKED from wave-2** —
`v1-completion-api` Reconciliation note 2's `origin daemon → PeerLink
streaming channel → target daemon → target loopback :8420` shape is
unchanged and re-affirmed, not reopened.

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
  Wave-3 makes this concrete rather than gestural: the relay rides
  `mesh-transport`'s `StreamOpen{stream_id, method: "v1.completions.stream", meta}`
  / `StreamChunk{seq, bytes}` / `StreamClose{outcome}` frames (`mesh-transport.md`
  §Streaming), opened `Address::Node{N}` pinned to the completion's owning node
  (the router already knows `completion_id -> node` from its in-memory index,
  concern boundary above) — token bytes are the `StreamChunk` payload, and the
  terminal `StreamEvent` (Completed/Failed/Cancelled/Preempted) closes the
  tunnel via `StreamClose`.

**The node-to-node transport for the data plane — SETTLED this wave (was
flagged open in the wave-2 draft).** The router runs inside the mesh daemon
(Ring 4). For a **local** pick (chosen node == self) it forwards over a direct
local HTTP/WS to the local inference api. For a **remote** pick it crosses the
tailnet, and mesh-core's wave-3 fold (Concern 11) plus the `mesh-transport`
contract (§Streaming, Reconciliation note 2) now answer the two-option question
this file raised in wave-2:

- **Relayed stream — the AUTHORIZED default, and the router's actual data
  plane today.** `StreamChunk`s ride the ordinary daemon relay (local daemon →
  peer daemon → target loopback `:8420`), so mesh can observe, backpressure
  (bounded per-stream buffers; a full buffer raises `mesh-transport`
  `Backpressure`, INTENT #38), and resume across interruption (#152 "so it can
  handle interruption/resume"). This is what both the WS token relay above and
  the remote REST forward ride; no bytes leave the relay.
- **Direct brokered tunnel — AUTHORIZATION PENDING (OQ-30), MUST NOT be built.**
  The off-relay node→node `PeerLink` data channel (bytes straight from the
  generating node to the consumer, taking raw `/v1` inference forwarding and
  multi-GB model weights off the relay) is the specific seam OQ-30 reserves for
  the operator's blessing (`mesh-transport.md` AUTHORIZATION note; mesh-core
  Concern 11). The frames are the *same* `StreamOpen/Chunk/Close` kinds — only
  the broker's *routing* (through the relay vs. a scoped direct tunnel)
  differs — so no new router-side code is needed to adopt the direct path
  later; it is a broker-side switch mesh-core flips once authorized, not a
  completion-router redesign.
- **This resolves the wave-2 "concrete ask to mesh-core" and the single-port-
  locality tension it raised.** The relayed path *is* single-port locality
  held all the way through (INTENT #58 governs service↔service addressing; the
  daemon-to-daemon relay hop is mesh's own internal plane, never a service
  socket) — no exception was needed once mediation was made universal. The
  direct tunnel remains the one sanctioned *future* exception (INTENT #114's
  bulk-transfer precedent), gated on OQ-30.
- **Loopback-relay to peer-local inference — LOCKED, unchanged from wave-2.**
  `v1-completion-api` Reconciliation note 2's shape stands verbatim: inference
  binds only `127.0.0.1:8420` and never a tailnet-public interface; a remote
  completion terminates at the *target node's own daemon* relaying to that
  node's local loopback, never a cross-node dial of `:8420`. Not reopened here.

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

### 9. Admission-deferred completions — a queued completion is a Promise, not an error (INTENT #152, wave-3 NEW)

**The old "queue-full" story, and why #152 simplifies it.** Wave-1/2 gave the
router exactly one answer for "nobody can take this completion right now":
`NoCandidates` → an immediate, blind `503` (`v1-completion-api`'s error table).
A client got no ETA, no guarantee, and had to invent its own retry/backoff —
the mesh gave up rather than mediated. INTENT #152's universal law — "if a
service can't respond immediately, mesh returns a promise; the caller moves
on; the value is pushed back" — applies here exactly as it does to a single
down target (mesh-core concern 10); this concern is the router's one
fleet-selection-shaped instance of that law. **Scope check (what this is NOT):**
this is not a per-node admission control redesign — `scheduler`'s own
completion queue stays unbounded/always-admitting as today (out of scope,
`scheduler`'s call); this concern covers only the router's *own* "no live
candidate is currently selectable" gap, which sits one layer above any single
node's admission and which nothing else in the design covers (mesh-core's
patch-through, concern 10b, restarts and parks a *resolved, specific* down
target — it has nothing to resolve when the balancer cannot even name a
candidate).

**The flow.**

1. **Happy path — unchanged.** `ModelAffinityBalancer.select()` (concern 2)
   finds a viable candidate (Tier-1 or Tier-2, least-loaded tie-break) and the
   router forwards synchronously exactly as today: `201 { id }`, no promise, no
   new latency, no new code path for the common case.
2. **Admission-deferred path (NEW).** `select()` finds no viable candidate
   *right now* — either genuinely `NoCandidates` (fleet empty/unreachable) or
   the new soft case, **every live candidate reports itself at capacity**
   (`running_count >= effective_max_concurrent` on its `NodeRegistry`
   projection, concern 2/3 — the same `effective_max_concurrent` scalar
   concern 2's spill note already tracked as "missing" in the wave-2 draft and
   now populated per `node-state-poll`/`types.md` wave-3; this concern is the
   productive, *much smaller* use for that field spill/Tier-3 still doesn't
   need — "is everyone full" is a fleet-wide boolean, not a bin-packing
   decision). Instead of an immediate `503`, the router:
   a. Buffers the (already fully-read, small JSON) submission body, mints a
      `PromiseId`, and answers the caller immediately with **`202 Accepted`**
      carrying a `types::transport::PromiseTicket` body (reused verbatim, not
      a parallel type — see Proposed contracts below) — a `Push{topic}` or
      `Fetch` delivery choice per the ticket.
   b. Registers `promise_id -> { buffered request, caller connection/return
      route, deadline }` in a small **router-local** pending-admission table.
      This is deliberately *not* mesh-core's cross-node promise routing table
      (mesh-core concern 10) and never emits a `PromiseFulfillment` frame —
      the promise is resolved entirely inside this module's process
      boundary, because resolving it only ever needs "did a candidate become
      selectable," never a cross-daemon relay. Keeping the two tables
      distinct avoids overloading mesh-core's routing-side table with a
      purely-local concern.
   c. Re-attempts `select()` on every live `NodeRegistry` update (the
      `inference-events` pubsub-primary feed, concern 3 — the same feed that
      already updates `running_count` in near-real-time is what notices a
      slot freeing) and, as a fallback, on the `node-state-poll` reconcile
      tick.
   d. **On success:** forwards the buffered submission to the now-selectable
      candidate, gets back the real `SubmitAccepted{id}`, and delivers it per
      the ticket's `PromiseDelivery` — `Fetch`: the caller (re-)`GET`s the
      router-native lookup endpoint below; `Push`: the router publishes the
      resolution as a `SaveFailed` `Envelope` on the ticket's topic via its
      existing `pubsub-relay` publish handle (Relationships section) — no new
      seam, it reuses the handle this module already holds for routing/
      failure events, and inherits pubsub-relay's already-designed
      no-silent-drop replay-on-reconnect (pubsub-relay concern 9), exactly the
      pattern `pubsub-relay` concern 10 documents for wire promise
      resolutions, borrowed here for a router-local one.
   e. **On deadline exceeded (bounded, INTENT #38 — never an unbounded
      wait or an unbounded backlog):** the promise resolves `Failed` with a
      catchable, `retriable: true` `WireError{ domain: "mesh", code:
      "no_candidates_within_deadline" }` — the honest, guaranteed-eventual
      form of what used to be a blind, immediate `503`. Both the deadline and
      the pending-admission table's max size are bounded per-router
      configuration; a full table sheds new admission-deferrals back to the
      old immediate `503` rather than growing without limit.
3. **Pinned (`Node{N}`) submissions are unaffected.** Concern 4's rule stands:
   a pin bypasses the balancer entirely, so there is no "candidate search" to
   defer — if node N itself answers slow/full, that is node N's own
   scheduler's business, forwarded byte-transparently exactly as any other
   status code (concern 7); this concern adds nothing to the pinned path.
4. **Interplay with concern 6 (mid-stream no-failover): none, by
   construction.** A promise only ever concerns *pre-forward* admission —
   before any byte of a response is produced. Once the promise resolves and
   the buffered request is actually forwarded, it is an ordinary completion
   from that point on; concern 6's terminal-fail rule is untouched.

**A second, narrow, sanctioned exception to byte-transparency.** Concern 7's
byte-transparency (the router copies bytes, transforms nothing) still governs
every *forwarded* exchange. This concern's `202`/`PromiseTicket` body and the
promise-lookup endpoint below are the two places the router **synthesizes** a
response on its own behalf rather than proxying one — necessarily, since there
is no live node yet to proxy from. Both are narrow, named exceptions, not a
crack in the byte-transparency discipline; once the buffered submission is
actually forwarded, byte-transparency resumes unchanged.

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
- **NEW (wave-3, proposed) — `completions-promise-lookup`.** A small, router-owned
  addition to the `v1-completion-api` namespace: `GET
  /v1/completions/promise/:promise_id` → `202 PromiseTicket` (still pending), `200
  CompletionSummary` (resolved — same shape `GET /v1/completions/:id` returns), or
  `410 { PromiseResolution::Failed }` (deadline exceeded, concern 9). Answered by
  the router itself, never forwarded to a node (concern 9's second
  byte-transparency exception) — proposed to `v1-completion-api`'s owner
  (api/inference) as a namespace-coordination note, not a terminus route.

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45):

- **consumes** `service-registry::Resolver` (`resolve_all("inference")` for
  membership+endpoints; `resolve(Node{N,…})` for pins) — the wave-2 membership
  source (concern 1/8).
- **consumes** `network-topology`'s peer feed (in-process) as the reachability
  overlay / membership fallback (concern 1/8).
- **consumes** `pubsub-relay`'s subscribe handle for `inference-events` (concern 3)
  and its publish handle to emit routing/`completion`-failure events, **now also**
  the `Push`-delivered admission-promise resolution (concern 9d) — no new handle,
  the existing publish seam carries one more `SaveFailed` envelope shape.
- **consumes** mesh-core's `PeerLink` relayed stream channel (`StreamOpen/Chunk/
  Close`) for remote forwarding and the WS token relay (concern 7) — **settled**
  this wave (mesh-core Concern 11 + `mesh-transport` §Streaming); the direct
  off-relay channel remains mesh-core's to build, gated on OQ-30.
- **provides UP** the fleet-routing entry point mesh-core's Dispatcher calls for
  `AnyNode{inference}` / `Node{N, inference}` (concern 4), now explicitly
  understood as an instance of mesh-core's universal-mediation Dispatcher path
  (mesh-core concern 2/10) rather than a bespoke handoff.
- **must-not-conflate siblings:** `service-registry` (slug→endpoint) and
  `network-topology` (peer vantage) are distinct tables from this module's
  `NodeRegistry` (live load/inventory) — concern 1. The router's **own**
  admission-deferred promise table (concern 9b) is a fourth, similarly distinct,
  purely-local table — never conflated with mesh-core's cross-node promise
  routing table (mesh-core concern 10) or with `chassis`'s `PromiseRegistry`.

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
5), the mid-stream no-failover rule + bounded pre-forward spill (concern 6), the
`forward()`/WS-relay contract change with the streaming transport now **settled**
(concern 7 — relayed = authorized default, direct = OQ-30 pending), and the
admission-deferred-promise flow (concern 9) are all decided and specified. One
piece remains **approach-sketched / open by dependency**: the `inference`-slug
membership reconciliation with service-registry (concern 8 — a flagged deviation
needing a service-registry re-touch + operator blessing on registering the fleet
per-node). The wave-1 trait signatures, status-code semantics, and OQ-1..OQ-11
from the original synthesis carry forward under these unchanged.

## Assigned design-depth

**Opus** single Component-Designer pass (this file), grounded in the frozen wave-1
completion-router design (the `mesh-design-synthesis.md` report — lives in the
harness workspace `substrate-v2/reports/`, NOT in this repo, so re-grounded
against the live
`lib/mesh/src/{router,balancer,discovery,proxy}.rs` + mesh.md concern 2 which
carries the synthesis summary), the wave-2 batch-1/2 designs (mesh-core Ring 4
seams, service-registry FleetAlias/instance model, network-topology feed,
pubsub-relay envelope, replicated-kv guarantees, types::node/pubsub/event), the
wave-3 batch-1/2 designs (`types::transport`/`delivery`, `mesh-transport`,
`chassis`, mesh-core's mediation/promise-routing/streaming folds,
pubsub-relay's `DeliveryPersistence`/`IntermediateCache` seam), and INTENT
#12/#15/#31/#36/#58/#59/#152/#153/#155.

## Suggested fill-model

**implementation-ready + moderate complexity → mid model OK**, with four carve-outs
for a careful hand: (1) the **mid-stream failure boundary** (concern 6 — the exact
"has any byte been emitted yet?" cutover between retryable-spill and terminal-fail is
the one correctness spot); (2) the **pubsub-primary + poll-reconcile join**
(concern 3 — a lossy live feed reconciled by a periodic snapshot without a
routing-critical counter ever going phantom-negative); (3) the **relayed
streaming forward** (concern 7 — sequenced *after* mesh-core's `StreamOpen/Chunk/
Close` broker lands; the direct path is not fill work this wave, OQ-30 pending);
(4) the **admission-deferred promise table** (concern 9 — the router-local
pending-admission table's bounded-deadline/bounded-size discipline and its clean
separation from mesh-core's cross-node promise table are the correctness edge; the
"all candidates saturated" detection itself is a simple boolean over an
already-live field, not new complexity). The balancer itself (concern 2) is
near-transcription from the wave-1 synthesis. Sequence after `service-registry`,
`network-topology`, `pubsub-relay`, `types` (`transport.rs`/`delivery.rs`),
`mesh-transport`, and `chassis` are filled — it rides all of them.

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

## Proposed contracts (wave 3)

This unit owns no new wire-vocabulary module (it consumes `types::transport`
verbatim) but proposes two amendments to contracts it does not own, plus one
settled reconciliation to record:

- **`v1-completion-api` amendment (proposed to api/inference, the terminus
  owner).** A third arm on `POST /v1/completions`: **`202 Accepted` +
  `types::transport::PromiseTicket`** body (concern 9a) when the router cannot
  select a candidate synchronously, alongside the existing `201`/error
  responses — reusing the ticket type verbatim, no new struct. Plus the
  router-native `GET /v1/completions/promise/:promise_id` lookup (Relationships
  section) in the same route namespace — answered by the router, never
  forwarded, so api's terminus authors nothing for it beyond not colliding on
  the path. Both are additive; no existing status code or route changes
  meaning.
- **`types::transport` (consumed, not owned) — confirmed reusable outside a
  `Frame`.** This unit reuses `PromiseId`/`PromiseTicket`/`PromiseDelivery`/
  `PromiseResolution` as plain JSON at an HTTP boundary (concern 9), not just
  inside a mesh-transport `Frame`. Flagged to `types`/`mesh-transport`'s owners
  as confirmation that these four structs have no `Frame`-specific dependency
  (they do not — `types.md`'s own definitions are plain structs/enums) and are
  safe to reuse this way; no shape change requested.
- **Node-to-node streaming transport — RESOLVED, recorded (not a new
  proposal).** `v1-completion-api` Reconciliation note 2's "transport
  requirement placed on mesh-core" is satisfied by mesh-core's wave-3
  Concern 11 + the `mesh-transport` contract's `StreamOpen/Chunk/Close`
  frames (concern 7, above). No further ask; recorded here so a harmonizer
  does not re-flag it as still open.

