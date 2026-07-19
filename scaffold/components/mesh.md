# mesh

**Status:** existing (`lib/mesh` + `bin/mesh`), RESHAPE + substantially EXPAND —
now explicitly **"the operating system"** of Substrate. Absorbs `gateway`
(2026-07-18 round-3 decision; see the superseded note below).
**Nesting:** top-level parent of {tailscale-query, network-topology,
service-registry, completion-router}.

> **ROUNDS 4–5 LOCK (2026-07-18, applied on top of commit `5dcadc1`):**
> (1) standard **WebSocket pub/sub protocol** confirmed (concern 7);
> (2) the internal-layering question is **ANSWERED** — mesh decomposes into
> internal LIBS with the same boring-layers discipline; utilities (service
> registry, replicated KV, `locks`, `cron`, the S3 adapter) are never
> standalone crates/services (see Charter);
> (3) **port LOCKED: `3649`** — supersedes `:8419` everywhere (concern 8);
> (4) stickiness (system-level resurrection), rigorous zombie-killing, and
> single-port locality locked (concerns 8–9);
> (5) two addressing classes designed in (concern 9);
> (6) new internal libs **`locks`** (concern 10 — partition semantics OPEN)
> and **`cron`** (concern 11).

> **ROUND-6 LOCK (2026-07-19, applied on top of commit `ea715af`):**
> (1) **init-supervision ANSWERED (3rd ask)** — mesh STARTS and supervises the
> local services (concern 12: observable interruptibility state, wait-for-idle
> non-critical updates, port-handoff update pattern);
> (2) the **two-way graceful-restart protocol** locked in shape — priority-
> laddered, built into EVERY service from the beginning (concern 13; the exact
> ladder is "latitude granted, discuss");
> (3) new internal capability: **queues + dead-letter queues, SQS-modeled**
> (concern 14; dead-letter escalation hooks into ccd investigation);
> (4) `locks` gains a **required catchable error type** for lock-threshold-
> exceeded-on-partition-merge, handled per-application (concern 10).

## Charter

`mesh` is Substrate's **operating system** — the network / coordination plane
every other service uses to find and reach every other service, the transparent
front door to the inference fleet, and (since the gateway merge) the browser-
facing observability origin. It is a real app in two forms sharing one crate
tree: a **daemon** (`bin/mesh serve`, bound at **`:3649`** — LOCKED rounds 4–5,
superseding the `:8419` used earlier in this file; see concern 8) that runs on every device
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
9. **The S3 adapter** (round-3 second batch, 2026-07-18 — moved here from
   `vfs`, superseding the S3-as-VFS-feature framing): since mesh owns ALL
   eventual-consistency/replication in the system, the AWS/S3 adapter lives in
   mesh — the overflow tier when the personal mesh runs out of space, using S3
   cold-storage classes, with **client-side encryption before upload** (not
   merely encrypted-at-rest). `vfs` and `kg` talk to mesh; **mesh distributes
   into S3 through this adapter** — including KG's cross-boundary sync (a KG
   written on the AWS side by an external agent becomes eventually consistent
   with the mesh; see `components/kg.md`). requirements-only.

And, new in the rounds-4–5 lock (2026-07-18):

10. **A standard WebSocket pub/sub protocol** — "We do have a standard
    WebSocket communication protocol, and we have a Pub/Sub system to actually
    communicate via WebSocket, and the mesh network relays it where it needs to
    go. We just have certain structs that the publishers and subscribers
    expect" (operator, verbatim). Typed envelope/message structs live in
    `types` (see `components/types.md`); mesh does the relaying. See concern 7.
11. **Internal utility LIBS** — the service registry's replicated KV store,
    the new `locks` (distributed semaphores, concern 10) and `cron` (scheduled
    tasks, concern 11) libs, and the S3 adapter are **internal libraries of
    mesh**, layered with the same boring-layers discipline — never standalone
    crates/services (see the ANSWERED layering note below).
12. **Fixed port `3649` + stickiness + zombie-killing** — locked; see
    concern 8.
13. **Single-port locality + two addressing classes** — every service talks
    ONLY to its local mesh daemon on `:3649`; see concern 9.

And, new in the round-6 lock (2026-07-19):

14. **Service supervision (ANSWERED, 3rd ask)** — mesh starts and supervises
    the local services on its node; each service exposes an observable
    interruptibility state; non-critical updates wait for idle; updates use
    the port-handoff pattern. See concern 12.
15. **The two-way graceful-restart protocol** — priority-laddered restart
    requests built into EVERY service from the beginning. See concern 13.
16. **Queues + dead-letter queues** — an internal, SQS-modeled queue
    capability (same internal-lib discipline as `locks`/`cron`); dead-letter
    escalation hooks into ccd investigation. See concern 14.

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

**ANSWERED (rounds 4–5, 2026-07-18; supersedes the OPEN internal-layering
note that stood here):** mesh DOES decompose internally with the same layering
discipline as the rest of the repo — "Let's do the same discipline inside of
mesh. Keep it layered — boring layers on top of boring layers" (operator). The
hard constraints, verbatim-grade: "any of the utilities offered by mesh are
just inside of mesh. They can be libs, they don't even have to be top-level
crates" — so the service registry, the replicated KV store, `locks`, `cron`,
and the S3 adapter are **internal LIBS of mesh, never standalone
crates/services**. Design latitude is granted on whether the replicated-state
primitives share one implementation or several ("If it makes sense to do them
separately, do them separately; if it makes sense to do them the same, do them
the same. Just make sure it's boring and it's all buried inside of mesh").
The exact layer boundaries remain the designer's to draw within these
constraints.

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
  libraries nested under the mesh app, not their own crates. **Rounds 4–5:**
  the internal utilities join this tree as internal libs — the replicated KV
  store, `locks` (concern 10), `cron` (concern 11), the S3 adapter, and the
  pub/sub relay (concern 7) — layered boringly, never standalone
  crates/services (per the ANSWERED layering note in the Charter; they *may*
  be workspace lib crates if convenient, but are only ever consumed through
  mesh).
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
  `inference`, `vfs`, `kg`, `projects`, `org`) needs this at boot; if it lived inside `lib/mesh` they would
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
  slug pointing at its own `:3649`** (port locked rounds 4–5), because the mesh *is* the fleet's
  transparent front door (it load-balances internally via the router's
  `NodeRegistry`, which discovers nodes by Tailscale tag). Singleton services
  (`db`, `ccd`, `vfs`, `kg`, `projects`, `org`) each register their own slug ->
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
or installer this pass. **Partially SUPERSEDED round-6:** mesh now also STARTS
and supervises the services (concern 12) — the boot order it tracks is one it
executes; the not-a-package-manager/installer boundary still stands.
Requirements-only; no design yet. (The internal-layering
question this used to drive is now ANSWERED — internal libs, boring layers; see
the Charter. The versioning *approach* is also now constrained: pairwise
service dependencies with minimal-restart rolling updates is the operator's
preference — commit-as-release-set was proposed and NOT adopted; the concrete
update protocol between nodes running mixed versions is OPEN. See
`overview.md`.)

### 7. Standard WebSocket pub/sub protocol (LOCKED rounds 4–5, requirements-only)

One standard WS communication protocol, pub/sub-shaped, relayed by mesh:
publishers and subscribers share typed structs ("we just have certain structs
that the publishers and subscribers expect"), and mesh relays messages where
they need to go — across services and across nodes. The envelope / pub-sub
struct domain is a planned `types` module (see `components/types.md`); the
existing WS surfaces (`network-events`, `dashboard-feed`, per-node event
streams) are expected to converge onto this protocol at harmonization time.
Requirements-only; wire shape undesigned.

### 8. Port 3649, stickiness, and rigorous zombie-killing (LOCKED rounds 4–5)

- **Port LOCKED: `3649`** (supersedes `:8419` throughout this scaffold).
  Verified free on the operator's fleet; IANA's obscure "nmmp" registration
  for 3649 is dead and ignored. (1649 was the other candidate — "1649 is a
  better number" — but was at risk of being already utilized; 3649 won.) On
  restart, **mesh kills whatever process squats on its port**.
- **Stickiness:** a system-level process (launchd/systemd-grade supervision)
  resurrects the mesh daemon whenever it dies — "I want this to be really
  sticky." Mesh must always come back without operator action.
- **Rigorous zombie-killing:** when a service restarts and re-registers on a
  new port, the old still-running copy must be **discovered and killed** —
  "I don't want multiple copies of the same app running. We need a discovery
  mechanism to kill services which are no longer supposed to be running." No
  duplicate app copies, ever. Interacts with the registry's lease model
  (concern 1) but is a distinct requirement: leases expire *entries*; this
  kills *processes*.

### 9. Single-port locality + two addressing classes (LOCKED rounds 4–5)

- **Single-port locality:** every service talks ONLY to its local mesh daemon
  on `:3649` — "Each service connects to each other service through the one
  port. They are not even aware that there are other services running on other
  ports... you just communicate with the mesh process. That's it." All
  inter-service and inter-node communication funnels through the local daemon;
  mesh does all relaying. Services are unaware of other services' ports.
- **Two addressing classes designed in:** (a) *virtualized* — "I want to talk
  to service X and I don't care which node" (talk to your local daemon, which
  maintains consistency across the network); (b) *pinned* — "I want to talk to
  service X on node N." Both are first-class; the operator flags there may be
  more axes. Requirements-only; the address syntax/API is undesigned.

### 10. Internal lib: `locks` (LOCKED rounds 4–5; partition semantics OPEN)

Distributed **semaphores as a first-order mesh library** (an internal lib, not
a crate/service), riding the replicated KV store:

- **Acquisition semantics:** knowledge of a semaphore acquisition must
  distribute to **all reachable nodes BEFORE the client is confirmed as
  holding the lock**.
- **Offline nodes** sync bidirectionally on reconnect.
- **Partition-twin problem:** if two partitioned subsets each create a
  same-named semaphore, identity is **slug + UUID** — the same slug in two
  partitions is two different UUIDs; re-acquiring a slug mints a new UUID (so
  non-identity is explicit); possibly one node owns each UUID.
- **OPEN — needs real care:** the full partition/merge semantics. Operator:
  "There's definitely a design question there — it has to generalize to be
  reliable in an infinite set of circumstances." Do not treat the slug+UUID
  sketch as the finished design.
- **Required catchable error type (NEW round-6; CAP honesty blessed):** the
  operator blessed the CAP-honesty framing ("You have my explicit
  understanding — we cannot violate the laws of physics") and requires a
  **specific, catchable ERROR TYPE for "lock threshold exceeded because two
  network partitions merged," handled per-application**. (Color: a walk-along
  Raspberry Pi that goes offline and rejoins later.) The full partition/merge
  semantics above remain OPEN; this error type is a locked requirement within
  them.

Consumers already known: the shared handler/execution engine's distributed
trigger execution (kg + stack) coordinates via `locks` — see `overview.md`'s
shared-libraries section and `components/kg.md`.

### 11. Internal lib: `cron` (LOCKED rounds 4–5, requirements-only)

Scheduled tasks as an internal mesh lib, in two flavors matching the
addressing classes: **"run on node N"** and **"run anywhere"** ("Tasks that
need to be run on specific nodes; tasks that just need to be run somewhere.
Cron should be something available to mesh"). Requirements-only.

### 12. Service supervision + port-handoff updates (ANSWERED round-6, 3rd ask)

**Mesh starts and supervises the local services** — the question asked three
times across rounds is now resolved yes. Requirements:

- **Observable interruptibility state:** each service exposes an observable
  state so mesh can tell when it's doing something that shouldn't be
  interrupted.
- **Non-critical updates wait for idle.**
- **Port-handoff update pattern:** start the new version of a service on a
  new port, flip the registry entry old→new, bring down the old port.
  (Interacts with concern 8's zombie-killing — here the old copy is brought
  down deliberately as part of the handoff; zombie-killing is the backstop if
  it lingers.)

Requirements-only; no design yet.

### 13. Two-way graceful-restart protocol (round-6; shape LOCKED, ladder = latitude granted)

A **two-way protocol built into EVERY service from the beginning**: mesh
signals a restart need with a priority, and the service participates in
deciding when it yields. The priority ladder, requirements-grade:

- **low** — mesh just waits for idle;
- **higher** — "finish what you're doing, then relinquish" (the service
  decides when its current work completes);
- **critical** — "I'm interrupting regardless — you have ~10 seconds to
  save";
- **beyond that** — kill outright, no warning.

**Compatibility-driven restarts are HIGH priority.** The exact ladder is
delegated — operator, verbatim: "whether we do priority levels on the restart
is up to you — something to discuss there" — **latitude granted, discuss with
the operator before locking the levels**. The two-way, service-participates
shape is LOCKED. Requirements-only.

### 14. Internal capability: queues + dead-letter queues (round-6, requirements-only)

**SQS-modeled queues** as an internal mesh capability (same internal-lib
discipline as `locks` and `cron` — never a standalone crate/service).
Operator, verbatim: "Model it after SQS in Amazon, so someday you can deploy
Mind OS directly into your Amazon account and there's a native system to take
advantage of" — i.e. the queue semantics must be deployable someday straight
onto real SQS via mesh's AWS adapter. **Dead-letter queues included**, with a
**dead-letter escalation hook into a ccd agent investigation** (confirmed as
a good guardrail — the same escalation pattern as the shared execution
engine's loop-depth hook; see `overview.md`'s shared-libraries section).
Requirements-only.

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
  mesh daemon's registry over HTTP (`localhost:3649` — port locked rounds 4–5), which is the
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
- any device/service (ccd, org, inference, vfs, kg, projects, **and the mesh CLI**)
  via `service-lookup` — register/resolve; THE wiring seam
  (scaffold/contracts/service-lookup.md)
- ccd via `service-registration` — CCD as a first-class registrant+resolver
  (scaffold/contracts/service-registration.md)
- ~~gateway via `mesh-registry-read`~~ — **collapsed** (2026-07-18): the
  gateway->mesh fleet read is now mesh reading its own registry in-process; the
  contract file is a tombstone (scaffold/contracts/mesh-registry-read.md)
- service-registry (peer instances) via `registry-replication` — anti-entropy
  merge (scaffold/contracts/registry-replication.md)
- vfs via `vfs-mesh` — VFS registration + topology awareness; S3 overflow now
  flows through mesh's own S3 adapter (scaffold/contracts/vfs-mesh.md)
- kg via `kg-mesh` — KG registration + graph replication across nodes and into
  S3 through mesh's adapter; graph consistency model OPEN
  (scaffold/contracts/kg-mesh.md)
- projects via `projects-mesh` — project registry push + published-dashboard
  surfacing (scaffold/contracts/projects-mesh.md)
- vault via `vault-mesh` — **NEW round-6 (requirements-only).** Vault
  registration/resolution; the use-without-seeing secret-brokerage shape is
  TBD (scaffold/contracts/vault-mesh.md)

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
rendering are **requirements-only**. (The internal-layering question that was
pending here is now ANSWERED — see Charter.)

Rounds-4–5 additions (concerns 7–11: pub/sub protocol, port/stickiness/
zombie-killing, single-port locality + addressing classes, `locks`, `cron`)
are **requirements-only** across the board, with `locks`' partition semantics
explicitly flagged OPEN.

Round-6 additions (concerns 12–14: supervision + port-handoff, the
graceful-restart protocol, queues + dead-letter queues; plus `locks`'
partition-merge error type) are likewise **requirements-only**, with the
restart-priority ladder explicitly "latitude granted, discuss."

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
