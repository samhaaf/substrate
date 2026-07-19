# mesh

**Status:** existing (`lib/mesh` + `bin/mesh`), RESHAPE + substantially EXPAND —
now explicitly **"the operating system"** of Substrate. Absorbs `gateway`
(2026-07-18 round-3 decision; see the superseded note below).
**Nesting:** top-level parent of {tailscale-query, network-topology,
service-registry, completion-router}.

## Charter

`mesh` is Substrate's **operating system** — the network / coordination plane
every other service uses to find and reach every other service, the transparent
front door to the inference fleet, and (since the gateway merge) the browser-
facing observability origin. It is a real app in two forms sharing one crate
tree: a **daemon** (`bin/mesh serve`, bound at `:8419`) that runs on every device
in the tailnet, and a **CLI** (`mesh service ...`, `mesh net ...`) for operator
ergonomics. The daemon owns these capabilities: (1) transparent completion routing
across the inference fleet with model-affinity balancing (the original
`lib/mesh`), (2) a standardized, extensible Tailscale-query surface, (3) a live
network-topology WebSocket feed (peer on/off + this device's own connectivity
loss), (4) a distributed, eventually-consistent `slug -> endpoint` service
registry that is **the system's single wiring seam** — every service registers
itself at boot and resolves its dependencies by slug here instead of by static
URL — and, new in the round-3 lock:

5. **Service-version tracking** — mesh tracks the installed version of every
   service on every node.
6. **Inter-service version relationships / dependencies** — "when a service is
   updated it looks at what versions of other services it depends on — we just
   track the requirements and the boot order" (operator, verbatim). Mesh holds
   the requirement graph; it does not resolve/install anything itself.
7. **Boot ordering** — the dependency-derived order in which services on a node
   (and across the mesh) come up.
8. **Dashboard serving + browser event fan-out** (absorbed from gateway) — mesh
   serves the dashboard's static assets and fans per-node event streams out to
   browser WebSocket clients; the dashboard renders every service's component
   from that service's published **surface schema** (see the boring-surface-schema
   section below and `scaffold/contracts/surface-schema.md`).

> **SUPERSEDED (2026-07-18): the mesh/gateway sibling split.** This file
> previously drew a hard boundary: "It does not own the aggregation/
> observability plane — that is `gateway`, which is a *consumer* of mesh, not
> part of it." The operator reversed this in round-3 feedback: "All inter-node
> communication needs to go through mesh… if mesh is built where anywhere you
> access it is exactly the same, I'm not seeing a need for gateway." Gateway
> ceases to exist as a component; its dashboard hosting, event fan-out,
> observability rollups, and browser REST proxy are now mesh capabilities.
> `components/gateway.md` is a tombstone.

**Boundary — what mesh does NOT own.** It owns no node-internal logic and depends
on no node-internal crate (`inference`, `engine`, `store`, `scheduler`, …); it
talks to nodes **only over HTTP**. This tier boundary is load-bearing: the mesh
must run on a GPU-less box (the always-on Raspberry Pi coordinator) and on every
GPU node alike. It does not own completion state (nodes' SQLite is the system of
record; the router keeps only a rebuildable in-memory `completion_id -> node`
index). It does not own service business
logic; the registry stores only addressing, versions, requirements, and boot
order — never application data (that is `db`).

**OPEN (recorded, not decided): internal layering.** The round-3 scope expansion
is acknowledged as large — mesh now spans routing, discovery, registry,
topology, version/dependency/boot-order tracking, and the dashboard/observability
origin. The operator has been asked whether mesh should decompose internally
into layered libs (e.g. a coordination core vs. an OS/version layer vs. the
observability/dashboard surface). Recorded as an open question; do not design
the layering yet.

## Primary design concerns

The mesh earned four children because it now spans four genuinely-different hard
problems glued by one daemon. Each child's difficulty is called out below; the
model/crate-boundary calls are the interesting part. (The round-3 absorptions —
observability/dashboard from gateway, and the OS-layer version/boot-order scope
— are covered in concerns 5 and 6 below; whether they become additional
children is exactly the open internal-layering question in the Charter.)

### 0. Crate topology (honors the repo's "crate = app, else nested library" rule)

- `bin/mesh` — the **app**: the `serve` daemon subcommand + the `service`/`net`
  CLI subcommands (noun-verb clap tree, mirroring `bin/db`).
- `lib/mesh` — the daemon's library: modules `router` (completion-router),
  `topology` (network-topology), `registry` (service-registry server side),
  `proxy`, `config`, `lib`. These are **not independently apps**, so they are
  libraries nested under the mesh app, not their own crates.
- `lib/tailscale` (**new crate**, `substrate-tailscale`) — the one child promoted
  to its own crate. The operator asked for it verbatim ("a crate just to query
  Tailscale … a separate surface worth capturing and standardizing," extensible
  "as we go"), it already has **two in-repo consumers** (`topology` and `router`),
  and it is the cleanest general-purpose library in the tree (usable outside
  Substrate). This is the confirm-and-strengthen of the decompose pass's call to
  nest it: it stays a child of mesh in the tree, but as a sibling *crate*, not a
  module.
- `lib/mesh-client` (**new thin crate**, `substrate-mesh-client`) — a shared
  library candidate I am flagging now rather than letting logic duplicate: the
  `register(slug, endpoint)` / `resolve(slug) -> endpoint` HTTP client that hits
  the **local** mesh daemon's registry. Every service (`ccd`,
  `inference`, `vfs`, `projects`, `org`) needs this at boot; if it lived inside `lib/mesh` they would
  all have to depend on the whole mesh (tailscale + router + axum). A ~one-file
  client crate keeps the seam cheap and is exactly the "extract shared logic"
  target a later pass would create anyway — cheaper to seed it here. (Final
  crate-vs-`types`-module boundary is the Skeleton Builder's to confirm; the
  *contract* is `service-lookup`.)

### 1. service-registry — the wiring seam and the hardest new sub-problem

- **Consistency model.** A per-slug **last-writer-wins map** (LWW-register CRDT):
  each entry is `slug -> { endpoint, owner_node, lease_expires_at, version }`
  where `version` is a `(wall_clock, node_id)` pair for a total order with
  deterministic tiebreak. LWW converges without coordination and is right-sized
  for 1–few devices; do **not** build vector clocks / SWIM at this scale (flagged
  as escalation, not built).
- **Leases, not just deregistration.** A registration carries a TTL; the owning
  service **heartbeats to renew**. This is what makes "a service died without
  deregistering" self-heal — expired entries become tombstones (with their own
  GC-after TTL to prevent resurrection). Without leases, a crashed service leaves
  a poisoned slug forever. This is the load-bearing correctness concern.
- **Replication (`registry-replication`).** Periodic **full-state anti-entropy**:
  each daemon merges its LWW-map with peers (peers are discovered *for free* via
  `tailscale-query` — the registry rides the same peer set the router uses). No
  bespoke gossip membership protocol; full-state sync is fine at this node count.
  The always-on Pi coordinator is the **recommended anti-entropy anchor/seed** (a
  new device pulls initial state from it), but must **not be required** — the
  model stays peer-to-peer eventually-consistent so any two online devices
  converge. (Recommended-not-required is a genuine call; see open questions.)
- **Fleet vs singleton addressing (non-obvious, important).** The inference
  *fleet* is NOT slug-registered per node — a `resolve("inference")` can't return
  "one of N interchangeable boxes." Instead the **mesh registers the `inference`
  slug pointing at its own `:8419`**, because the mesh *is* the fleet's
  transparent front door (it load-balances internally via the router's
  `NodeRegistry`, which discovers nodes by Tailscale tag). Singleton services
  (`db`, `ccd`, `vfs`, `projects`, `org`) each register their own slug ->
  their own endpoint (the `dashboard` slug is mesh's own surface now — mesh
  serves it directly, no separate registrant). So two discovery mechanisms coexist deliberately: **tag-based
  fleet discovery** (router) and **slug registry** (singletons + the `inference`
  front-door alias). Consumers never see the split — they just `resolve(slug)`.
- **Endpoint shape (refines the operator's "slug -> host:port").** Registry values
  are `{ scheme, host, port, health_path? }` (or a canonical `base_url`), not bare
  `host:port`, so `mesh service open <slug>` can construct a browsable URL and
  mesh's own observability layer can health-check. Bare host:port cannot be
  opened in a browser.

### 2. completion-router — RESHAPE, already fully designed

The transparent completion data plane. **Design is implementation-ready** and
frozen in `reports/mesh-design-synthesis.md` (three-stage propose/critique/
synthesize): `NodeRegistry` (discovery-refresh + state/inventory poll loops),
`ModelAffinityBalancer` (prefer resident tier, least-loaded tie-break; spill and
Tier-3 deferred with reasons), a `forward()` whose contract **must change** to
carry status + headers + a streaming body (today's `Result<Vec<u8>>` cannot
express 404/409/502/503 or streaming), a separate WS relay for
`/v1/completions/:id/stream`, and `X-Substrate-Node` / `?node=` pinning resolved
above the balancer. Three distinct tables must not be conflated: the router's
`NodeRegistry` (routing health of the inference fleet) ≠ `service-registry`
(slug addressing) ≠ `network-topology` (general topology event feed). See the
synthesis for the full trait signatures, status-code semantics, and OQ-1..OQ-11.

### 3. tailscale-query — standardized, extensible query surface

Own crate (`substrate-tailscale`). A `TailscaleQuery` trait with **one typed
method per subcommand** (`status()` now; `netcheck()`, `ping()`, `whois()` as
future "tools we add as we go") — extension is *adding a method*, never breaking
an existing one. Two impls: `RealTailscale` (shells `tailscale status --json` via
`std::process::Command`) and `FakeTailscale` (fixture-driven) so topology/router
tests and CI/dev boxes with no tailnet still run. The public typed structs
(hostname, DNSName, TailscaleIPs, ACL tags, online, self-vs-peer) are **decoupled
from private serde structs** that deserialize only the JSON subset we use, so
Tailscale's JSON churn never leaks past the crate. **Trait stays synchronous**
(per the synthesis decision — sidesteps the `dyn`-compat problem); async wrapping
via `spawn_blocking` is the caller's job.

### 4. network-topology — live topology + self-connectivity feed

A WS surface that **diffs consecutive `tailscale-status` snapshots** into
transition events (peer joined / left / went offline) and publishes them over
`network-events` to any subscriber. The **hard, distinguishing concern is
detecting this device's OWN connectivity loss**: that is NOT a peer diff — it is
inferred when the `tailscale status` shell-out fails or reports
`BackendState != Running` / self `Online == false`. The layer must emit a
distinct `self_offline` / `self_online` event and clearly separate "the tailnet
is unreachable from here" from "one peer went down," because under self-offline we
cannot observe peers at all (their state is *unknown*, not *offline*). New
subscribers get a **snapshot-on-connect then deltas** (so they don't miss current
state). This is a general feed, deliberately distinct from the router's internal
health polling.

### 5. Absorbed observability / dashboard plane (from gateway, round-3)

Mesh now hosts what gateway used to: serving `ui/dashboard/dist/` static assets
from one origin; subscribing to each node's inference / gc / ccd event streams
and multiplexing them into ONE topic-filtered browser WebSocket (`GET /events`,
envelopes tagged with real `node_id`s); presentation-shaped fleet rollups
(`/api/nodes`, `/api/mesh/stats`, `/api/nodes/:id/stats`); and the same-origin
browser REST proxy (`/api/nodes/:id/inference/*`, `/api/nodes/:id/gc/*`).
Design content carried over from the pre-merge `gateway.md` that still applies:

- **Dynamic per-node subscription reconciliation** — the supervisor that diffs
  the live node set and spins up / tears down one inference-WS + one GC-WS per
  node without leaking tasks or double-subscribing. Now it diffs mesh's *own*
  node registry in-process (the old `mesh-registry-read` HTTP hop collapsed).
- **Aggregation must not become routing** — rollups are derived, cache-cheap,
  disposable; the placement/routing decision stays in `completion-router`.
  (The old worry "gateway must not re-collapse into mesh" inverts: inside one
  daemon the discipline is a module boundary, not a process boundary.)
- **Proxy-forward dedup** — the old gateway `proxy.rs::forward` and the router's
  `forward()` now live in the same crate tree; the shared-lib extraction the
  gateway design flagged becomes an internal refactor, not a cross-crate one.
- The **surface-schema-driven dashboard** (round-3): mesh renders every
  service's dashboard component from that service's published surface schema
  (`scaffold/contracts/surface-schema.md`) — mesh discovers services via its
  own registry, fetches each one's schema, and the dashboard renders from it.

### 6. The OS layer: versions, requirements, boot order (round-3, requirements-only)

Mesh tracks (a) the installed version of every service on every node, (b) the
version-relationships/dependencies BETWEEN services — "when a service is updated
it looks at what versions of other services it depends on — we just track the
requirements and the boot order" — and (c) the boot order derived from those
requirements. Scope note: mesh *tracks and answers*; it is not a package manager
or installer this pass. Requirements-only; no design yet. This is the biggest
driver of the open internal-layering question in the Charter.

### CLI surface (new operator requirement)

`bin/mesh` becomes a noun-verb CLI (clap, like `bin/db`); the bare invocation and
`serve` keep today's daemon behavior for back-compat.

```
mesh [serve] [--config mesh.toml]     # run the daemon (default; back-compat)
mesh service open <slug>              # resolve slug -> URL, open in the browser
mesh service resolve <slug>           # print resolved endpoint (scripting)
mesh service list [--json]            # all registered services + device + health + lease
mesh service register <slug> <endpoint> [--ttl <dur>]   # manual registration
mesh service deregister <slug>
mesh net status [--json]              # current tailnet: peers on/off + self connectivity
mesh net watch                        # tail the network-events WS
```

- `mesh service open <slug>` — the headline ask: resolve the slug, construct the
  URL from the registry endpoint, open it cross-platform (macOS `open` / Linux
  `xdg-open` / the `open` crate). The operator "never has to memorize which port a
  service runs behind."
- **Resolution path:** the CLI is a short-lived process that queries the **local**
  mesh daemon's registry over HTTP (`localhost:8419`), which is the
  eventually-consistent merge of the whole tailnet. Requires a local mesh daemon
  (or a reachable peer) — an explicit precondition, surfaced as a clean error, not
  a silent hang. This reuses `service-lookup` (the CLI is just another party); no
  new contract edge.

## Relationships / edges

Edges match the contract graph in `overview.md`. Grouped by which child owns them.

**completion-router:**
- inference (api) via `v1-completion-api` — forwards the `/v1/` REST+WS surface
  transparently (see scaffold/contracts/v1-completion-api.md)
- inference (api) via `node-state-poll` — NodeRegistry polls `/v1/system/state` +
  `/v1/models`, never on the request path (scaffold/contracts/node-state-poll.md)
- tailscale-query via `tailscale-status` — consumes parsed peer status for fleet
  discovery (scaffold/contracts/tailscale-status.md)

**service-registry (the seam):**
- any device/service (ccd, org, inference, vfs, projects, **and the mesh CLI**)
  via `service-lookup` — register/resolve; THE wiring seam
  (scaffold/contracts/service-lookup.md)
- ccd via `service-registration` — CCD as a first-class registrant+resolver
  (scaffold/contracts/service-registration.md)
- ~~gateway via `mesh-registry-read`~~ — **collapsed** (2026-07-18): the
  gateway->mesh fleet read is now mesh reading its own registry in-process; the
  contract file is a tombstone (scaffold/contracts/mesh-registry-read.md)
- service-registry (peer instances) via `registry-replication` — anti-entropy
  merge (scaffold/contracts/registry-replication.md)
- vfs via `vfs-mesh` — VFS registration + topology awareness
  (scaffold/contracts/vfs-mesh.md)
- projects via `projects-mesh` — project registry push + published-dashboard
  surfacing (scaffold/contracts/projects-mesh.md)

**observability / dashboard plane (absorbed from gateway):**
- inference via `inference-events` (mesh <- inference, one subscription per
  node) (scaffold/contracts/inference-events.md)
- gc via `gc-events` (mesh <- gc daemon) (scaffold/contracts/gc-events.md)
- ccd via `ccd-events` (mesh <- ccd) (scaffold/contracts/ccd-events.md)
- dashboard via `dashboard-feed` (mesh -> dashboard) — `GET /events` fan-out +
  REST + static hosting (scaffold/contracts/dashboard-feed.md)
- every service via `surface-schema` — each service publishes its observable-
  surface schema; mesh's dashboard renders from it
  (scaffold/contracts/surface-schema.md)

**network-topology:**
- tailscale-query via `tailscale-status` — polls/diffs snapshots
  (scaffold/contracts/tailscale-status.md)
- any subscriber (ccd, org; mesh's own fan-out hub consumes it in-process) via
  `network-events` — WS topology + self connectivity feed
  (scaffold/contracts/network-events.md)

**tailscale-query:**
- {network-topology, completion-router} via `tailscale-status` (producer side of
  the above) (scaffold/contracts/tailscale-status.md)

## Nesting

Parent: (top-level) | Children: [tailscale-query, network-topology,
service-registry, completion-router]

- `tailscale-query` and `network-topology` are near-enclaves feeding one/few
  siblings, but tailscale-query is elevated to its own crate per the operator's
  explicit ask + reuse (see Concern 0).
- `service-registry` is a child by containment but its client half
  (`service-lookup`) is the whole system's seam — the one edge every top-level
  component touches. The proposed `substrate-mesh-client` crate is the shared
  handle to it.
- `completion-router` = the original `lib/mesh` core, RESHAPEd per the synthesis.

## Thoroughness level

**approach-sketched** (honest overall level, with two sub-parts differing):
`completion-router` is **implementation-ready** (frozen synthesis with trait
signatures). `service-registry`, `network-topology`, `tailscale-query`, and the
CLI are **approach-sketched** — the consistency model, lease/tombstone rules,
extensibility shape, self-offline detection, and subcommand tree are all decided,
but the replication wire format, exact LWW merge/GC parameters, and the
`mesh-client` crate boundary are deliberately left for the Contract Harmonizer
(step 3) and Skeleton Builder, and several open questions remain.

Round-3 additions differ again: the absorbed observability/dashboard plane
(concern 5) inherits gateway's **implementation-ready** design content, but the
OS layer (concern 6: versions, requirements, boot order) and the surface-schema
rendering are **requirements-only**, pending the internal-layering answer.

## Assigned design-depth

**Design Mesh** for the discovery/routing half — run
`reports/mesh-design-synthesis.md` (three-stage propose → critique → synthesize
over the real V1 code) — **plus a single strong-model (Opus) Component-Designer
pass** for the expanded registry + topology + tailscale-query-crate + CLI scope,
grounded by reading the live `lib/mesh/{lib,config,discovery,router,balancer}.rs`
and the `bin/db` CLI as the noun-verb template.

## Suggested fill-model

Per-child, because the design bought down fill effort unevenly:
- `completion-router` — implementation-ready + high complexity → **cheaper model
  OK** (the synthesis is a near-spec; a strong model is not needed to transcribe
  it).
- `tailscale-query` — approach-sketched + low complexity → **mid model** (Sonnet);
  it is bounded shell-out + serde.
- `network-topology` — approach-sketched + moderate complexity → **mid model**;
  the only subtlety is self-offline detection, which is spelled out.
- `service-registry` — approach-sketched + **high complexity** → **strong model,
  or a focused Design Mesh pass** on the replication protocol + LWW/lease/tombstone
  edge cases before fill. This is the one child where design effort has not fully
  bought down fill risk; do not send it to a cheap model.
- CLI (`bin/mesh` noun-verb) — approach-sketched + low complexity → **mid model**
  (clap tree + `service-lookup` client + browser-open).
