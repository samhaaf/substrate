# dashboard-serving

**Status:** NEW (wave 2). **Nesting:** internal lib of mesh (module
`lib/mesh::dashboard`), Ring 4 in mesh-core's layering (rides `service-registry`
+ `pubsub-relay` + `replicated-kv`; see `mesh-core.md` concern 1). **Prior art:**
the killed V1 `bin/gateway/src/{main,hub,ws,upstream,proxy,stats,topics,state,config}.rs`,
folded into mesh at the 2026-07-18 gateway merge (`gateway.md` tombstone;
`mesh.md` concern 5, "absorbed observability plane"). This file designs the
serving/aggregation side of the dashboard seam; the Svelte FRONTEND is batch 6
(`dashboard.md`) and builds against the seam contract stated here.

## Charter

`dashboard-serving` is the **observability serving plane** buried inside the mesh
daemon: the one place the browser-facing operator dashboard is served from and
fed by. It owns four jobs and nothing else:

1. **Static asset origin** — serving the compiled `ui/dashboard/dist/` from one
   browser-reachable HTTP origin per node, and registering the `dashboard` slug
   so `mesh service open dashboard` resolves to it (INTENT #24/#36; mesh-core
   boot step 8).
2. **The browser event feed (`GET /events`)** — a topic-filtered, lossy WebSocket
   fan-out to browser clients that is **the read-only profile of `pubsub-relay`'s
   own subscribe/publish protocol** — the SAME `types::pubsub` structs, filters,
   and lossy semantics, not a second protocol (concern 2). dashboard-serving is
   a plain in-process subscriber of `pubsub-relay` (`trait PubSub`); it does not
   scrape any service's `/events` socket the way V1 gateway did (concern 3).
3. **The surface-schema aggregation pipeline** (INTENT #46) — discover every live
   service via `service-registry`, read each service's published boring
   `SurfaceSchema` (`types::surface`) from the replicated surface store, and
   assemble the fleet-wide `DashboardManifest` the schema-driven frontend renders
   from (concern 4). Project-published dashboards (INTENT #47) enter this manifest
   as ordinary surfaces, so they become navigable without dashboard-serving
   knowing anything about `projects` (concern 6).
4. **Presentation rollups + same-origin REST convenience proxy** — `/api/nodes`,
   `/api/mesh/stats`, `/api/nodes/:id/stats` (derived, cache-cheap, disposable
   views over already-aggregated state), and a node-scoped browser proxy
   `/api/nodes/:id/:slug/*` that lets the schema-driven renderer issue plain
   same-origin `fetch()`es without the browser ever leaving the mesh front door
   (concern 5).

**Boundary — what it does NOT own.** It does not define the pub/sub wire, topic
taxonomy, filter matching, cross-node relay, or lossy backpressure policy — that
is `pubsub-relay`; dashboard-serving is one of its subscribers and reuses its
protocol verbatim. It does not define the `SurfaceSchema`/`Envelope`/`Event`
structs — those are `types` (`surface.rs`/`pubsub.rs`/`event.rs`), compiled in.
It does not discover services, hold the registry, or manage leases — that is
`service-registry`; it only *reads* the directory. It does not route or forward
completions and it does not pick nodes — that is `completion-router`. **The
load-bearing invariant carried from the gateway design: aggregation must never
become routing** (concern 5) — rollups are derived and disposable, the proxy
uses the browser-supplied `:id`, and no placement decision lives here. It holds
no application data, no completion state, and is a compiled-in internal library,
never a standalone service (INTENT #54). Access control is explicitly out of
scope (INTENT #39).

## Primary design concerns

### 1. The browser origin is HTTP, distinct from the mesh-transport `:3649` floor

The dashboard, `GET /events`, and `/api/*` are **browser HTTP**, not
mesh-transport frames. A browser cannot speak mesh-core's `Hello`/`Welcome`
handshake or `Address`-envelope framing, and `mesh service open dashboard` must
produce a real browsable `http(s)://host:port` URL — which a WS/mesh-transport
port cannot be. So the observability surface needs a **plain HTTP/WS origin
separate from the mesh-transport `:3649` port** (exactly as V1 gateway ran on its
own `:8400`).

Reconciling this with mesh-core's "internal libs never open their own socket;
the shell hands them handles" discipline (`mesh-core.md` concern 1): the
**mesh-core shell owns the browser HTTP listener** and hands dashboard-serving a
router-mount seam; dashboard-serving *supplies the axum `Router`* (static +
`/events` + `/api/*`) and never binds a socket itself. The shell acquires the
port using the standard try-preferred-then-fall-back rule (INTENT #36; suggested
default `:3648`, one below the mesh floor, but discovered-not-hardcoded), then
registers the `dashboard` slug → `Endpoint { scheme: Http, host, port,
health_path: "/health" }` in `service-registry` (boot step 8). This keeps the
mesh-transport floor pure, keeps libs socket-free, and makes the dashboard a
first-class browsable registry entry.

**Flagged as a co-batch friction point with mesh-core (batch 1, already
designed):** `mesh-core.md` registers "the `dashboard` slug served by
dashboard-serving" and ships `mesh service open dashboard`, but it does not
explicitly carve out a **browser HTTP listener + a `trait HttpSurface`
router-mount seam** among its Ring-0 seams (it lists SessionMux/PeerLink/
ProcessCtl/etc.). This design *requires* that seam. The alternative — sniffing
browser HTTP vs. mesh-transport on one `:3649` socket via SessionMux — is
rejected here (it muddies the transport floor and complicates `mesh service
open`), but the choice is mesh-core's to confirm. See friction points.

### 2. The browser feed IS pubsub-relay's protocol, read-only

The operator's ask (INTENT #5): a "topic-based pub/sub WebSocket with
per-completion-ID subscription." `pubsub-relay` already designed exactly this
protocol (`PubSubClientMsg`/`PubSubServerMsg` over `types::pubsub`, three filter
kinds incl. per-completion `Exact`). The instruction is explicit: reuse its
subscription semantics, do not invent a second protocol. So the browser `GET
/events` WS speaks **the same `types::pubsub` frames**, with exactly one
restriction:

- **Subscribe / Unsubscribe accepted; Publish rejected.** A browser is an
  unprivileged read-only observer — it has no `service-registry` lease, so its
  provenance cannot be attested (`pubsub-relay` already rejects unattested
  publishes with `NotRegistered`). dashboard-serving applies that same gate: a
  `PubSubClientMsg::Publish` from a browser session is answered with
  `PubSubServerMsg::Error { code: NotRegistered }` and the session stays open.
  This is genuinely the same protocol with an authorization profile, not a fork.
- **Delivery, lag, and per-completion are inherited unchanged.** Matched messages
  arrive as `PubSubServerMsg::Event(Envelope)`; a slow browser gets
  `PubSubServerMsg::Lagged { dropped, since_seq }` and stays connected (lossy by
  contract — correct for observation). Per-completion view = a plain
  `TopicFilter::Exact(inference.completion.<id>)` (or `Completion(id)`), no
  special case.
- **Per-node tagging is free.** V1 gateway hand-stamped `node_id` onto every
  event at scrape time (`upstream.rs`). In v2 the `Envelope.provenance.origin_node`
  (daemon-stamped, un-spoofable — `pubsub-relay` concern 1) already carries it,
  so the browser reads `envelope.provenance.origin_node` to build its per-node
  grid. dashboard-serving re-stamps nothing.

Mechanically, dashboard-serving holds ONE in-process `pubsub-relay` subscription
per browser connection (mirroring the filter set the browser sent) and pipes
matched `Envelope`s out the browser socket — the V1 `ws.rs` per-client
`broadcast::Receiver` + `should_forward` loop, now backed by `trait PubSub`
instead of a bespoke `Hub`. The ping-keepalive and lagged-tolerance loop carry
over near-verbatim from `ws.rs`.

### 3. No upstream scraping — pubsub-relay's interest routing absorbs it

This is the biggest structural change from the gateway design, and it
**deviates from the literal wave2-plan charter line** ("per-node subscription
reconciliation — one inference-WS + one gc-WS + one ccd-WS per node, no leaks/
doubles"). That line describes the V1 gateway model (`upstream.rs`:
`connect_inference`/`connect_gc` opened a WS to each service on each node and
republished into a `Hub`). In v2 that model is **superseded by `pubsub-relay`**:

- inference / gc / ccd **publish their own event catalogs** onto the reserved
  topic prefixes `inference.*` / `gc.*` / `ccd.*` via `mesh-client` →
  `pubsub-relay` (their responsibility, their design passes — batch 5, batch 3,
  batch 6). network-topology publishes `network.*`.
- dashboard-serving, running inside the *same* daemon, simply subscribes locally
  to `pubsub-relay` with `Fleet`-scope prefix filters (`inference`, `gc`, `ccd`,
  `network`). `pubsub-relay`'s cross-node interest-routed relay (its concern 7)
  guarantees matching events from *every* node reach this daemon — so "anywhere
  you access mesh is the same" (INTENT #35) gives dashboard-serving the whole
  fleet's stream from one local subscription, with **zero per-node/per-service
  socket bookkeeping**. The "no leaks / no doubles" reconciliation the charter
  worried about now lives in `pubsub-relay`'s interest table, not here.

Consequence: dashboard-serving keeps NO upstream connection pool, no reconnect
loop, no `connect_*` scrapers. It runs in *every* mesh daemon (Ring 4), so every
node exposes an equivalent dashboard fed by its local relay — the always-on Pi's
mesh keeps the dashboard live even when every GPU box sleeps (`dashboard.md`
rationale), because the roster/registry/surfaces are replicated and the feed is
relayed. This deviation is called out as a friction point so the wave-end
reconciliation records that the charter's per-node-WS reconciliation job was
consciously moved into `pubsub-relay`, not dropped.

### 4. The surface-schema aggregation pipeline (INTENT #46/#37)

Every service publishes a boring `SurfaceSchema` (`types::surface`) describing
how to render its dashboard component and what calls to make against it; the
dashboard renders from schema, never from hand-built per-service panels. The
pipeline has two halves:

- **Publication (mesh-client push → replicated store).** A service publishes its
  `SurfaceSchema` through `mesh-client` the same way it registers (surface-schema
  publication is named in `mesh-client`'s charter). The schema lands in a
  **replicated-kv keyspace `surface/<slug> -> SurfaceSchema`** — keyed by slug,
  not by node, because a service's *render/interaction description* is identical
  across the nodes running the same build (the per-node *data* differs and rides
  the event feed / REST, not the schema). Replication means any node's
  dashboard-serving reads the whole fleet's schemas from a **local** KV read
  ("anywhere you access mesh is the same"); no on-demand per-service fetch, no
  fetch-failure surface. Schemas are small and change only on version bumps —
  a comfortable fit for `replicated-kv`'s periodic full-state anti-entropy.
- **Aggregation (read → assemble → serve).** dashboard-serving joins
  `service-registry.list()` (which slugs are live) with the `surface/*` keyspace
  (their schemas) and the node roster (concern 5) into one `DashboardManifest`
  served at `GET /api/surface`, and re-publishes a small
  `dashboard.surface.changed` event on the feed when the schema set changes so
  the frontend re-fetches. The manifest is a pure function of registry + surface
  store + roster; dashboard-serving adds no per-service opinion.

**Version handling (INTENT #37, the component-versioning story the operator
flagged unresolved).** `SurfaceSchema.v` keys the rendered component to a
`(service, v)` pair. During a mixed-version rolling update two nodes may publish
different `v` for the same slug; because the store is keyed by slug and LWW, the
dashboard renders whichever `v` won convergence. This is acceptable for
observability but is a real edge tied to `supervision`'s OPEN mixed-version
protocol (INTENT #66) — flagged as an open question, not silently resolved. A
`(slug, v)` composite key that retains both is the alternative; kept boring
(slug-keyed, latest-wins) for v1.

### 5. Presentation rollups + the node-scoped convenience proxy (aggregation ≠ routing)

Carried from V1 gateway `stats.rs`/`proxy.rs`, re-grounded on the aggregated
mesh state so they make no fresh cross-service HTTP calls:

- **`GET /api/nodes`** → the fleet roster: `Vec<NodeInfo>` (identity, roles,
  `last_state: SystemState`) built from `service-registry` + `network-topology`
  status + the replicated telemetry snapshot. Supersedes V1 gateway's hardcoded
  single-entry `networkState` (`dashboard.md` concern 6).
- **`GET /api/mesh/stats`** → fleet-wide aggregate (node count, up/down, totals).
- **`GET /api/nodes/:id/stats`** → the V1 `local_stats` generalized per node
  (disk total/used/free, gc-managed bytes, GPU utilization + `is_estimate`
  honesty flag — INTENT #9). In v2 these values are read from the already-
  replicated `NodeInfo.last_state`/`SystemState` + gc's published surface data,
  NOT by dashboard-serving sysinfo-probing a remote box (it can't, and must not).
- **`ANY /api/nodes/:id/:slug/*path`** → the same-origin convenience proxy. It
  resolves `(slug, id)` via `service-registry` and hands mesh-core a `Request`
  envelope (`Address::Node { node: id, slug }`) — mesh-core's Dispatcher relays
  it (local delivery or `PeerLink` to peer `id`) and returns the `Response`;
  dashboard-serving does only the browser-HTTP ↔ mesh-`Request`/`Response`
  translation. This preserves single-port locality (the browser never leaves the
  mesh origin; no cross-node browser HTTP), generalizes V1's flat
  `/api/inference/*` + `/api/gc/*` to node-scoped + slug-generic, and — critically
  — the node `:id` and `:slug` are **browser-supplied**, so the proxy makes NO
  placement decision. That is the concrete meaning of "aggregation must not
  become routing": `completion-router` picks nodes; this proxy is told which one.
  A `Rest { path }` `DataSource` in any surface schema resolves against these
  paths, so the schema-driven renderer needs no bespoke networking.

### 6. Project-published dashboards, served without designing projects (INTENT #47)

`projects` (a layer-6 stub) will publish, per project, a dashboard
`SurfaceSchema` via `projects-mesh`, exactly like any service publishes its
surface. Because dashboard-serving's aggregation (concern 4) is slug-generic, a
project dashboard is **just another `surface/<project-slug>` entry** that lands
in the `DashboardManifest` — no projects-specific code here. To make them
*navigable* rather than flat, the manifest carries a light **navigation grouping**
(`Vec<NavEntry>` — a tree of `{ id, title, kind: MeshCore | Service | Project,
surface_slug }`) so the frontend can mount core mesh panels, per-service panels,
and project sub-dashboards under a browsable tree. The nav tree is derived (core
panels are built-in; service entries come from the live registry; project
entries come from surfaces whose publisher registered as a project). This is the
whole seam projects needs — it publishes a surface + a registration; navigation
falls out — and it is forward-compatible: with `projects` unbuilt, the
`Project` branch is simply empty. dashboard-serving designs the mount point, not
projects.

## Relationships / edges

Contract edges (the browser edge is HTTP on the dashboard origin; the rest ride
mesh-internal seams):

- **mesh(dashboard-serving) → dashboard (frontend)** via `dashboard-feed` — the
  whole batch-6 seam: static hosting + the read-only `GET /events` pub/sub WS +
  `GET /api/surface` manifest + `/api/nodes`, `/api/mesh/stats`,
  `/api/nodes/:id/stats` rollups + the `/api/nodes/:id/:slug/*` proxy. Proposed
  below. (scaffold/contracts/dashboard-feed.md)
- **every service → mesh dashboard** via `surface-schema` — services publish
  their boring `SurfaceSchema` (via `mesh-client`); dashboard-serving aggregates
  and serves it. Cross-cutting, surface-schema-style (one document, every service
  a party); struct half is `types::surface`. Proposed below.
  (scaffold/contracts/surface-schema.md)
- **mesh ← inference** via `inference-events` — inference's event catalog on the
  `inference.*` topic prefix; dashboard-serving is the mesh-side subscriber.
  Re-grounded below as a pubsub-protocol topic/catalog, not a parallel wire.
  (scaffold/contracts/inference-events.md)
- **mesh ← gc** via `gc-events` — gc's catalog on `gc.*`. Same re-grounding.
  (scaffold/contracts/gc-events.md)
- **mesh ← ccd** via `ccd-events` — ccd's agent-lifecycle catalog on `ccd.*`;
  still carries the wave2-plan "proposed, pending confirmation" marker (flag #6).
  Same re-grounding. (scaffold/contracts/ccd-events.md)

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45):

- **consumes** `pubsub-relay::trait PubSub` (in-process subscribe to
  `inference.*`/`gc.*`/`ccd.*`/`network.*` for the browser feed; publish
  `dashboard.*` change notices) — the feed substrate (concern 2/3).
- **consumes** `service-registry::trait Resolver` (`list`/`resolve_all`/`resolve`
  — discover services for aggregation, resolve `(slug, node)` for the proxy) and
  its `replicated-kv` `surface/*` keyspace read (published schemas, concern 4).
- **consumes** mesh-core's browser-HTTP listener handle + `Dispatcher`/
  `LocalDelivery` (serve the Router; relay the node-scoped proxy `Request`s) —
  see concern 1/5 and the mesh-core friction flag.
- **sibling, must-not-conflate:** `completion-router` owns node selection +
  completion forwarding; dashboard-serving's proxy never selects (concern 5).
  `network-topology` owns `network-events`; dashboard-serving only *subscribes*
  to `network.*` for the roster, it does not author that contract.

## Nesting

Parent: mesh (mesh-core) | Children: none. Module `lib/mesh::dashboard`, Ring 4
(rides Ring-3 `service-registry` + Ring-1 `pubsub-relay` + Ring-2 `replicated-kv`,
and mesh-core's browser-HTTP listener seam). Never standalone. The parent/child
structure lives here and in overview.md, not in the directory layout.

## Thoroughness level

**implementation-ready.** The browser HTTP origin decision (separate listener,
shell-owned, `dashboard` slug), the read-only pubsub-relay feed profile, the
in-process-subscription-instead-of-upstream-scraping simplification, the
surface-schema publish→replicate→aggregate→serve pipeline and its slug-keyed
version handling, the presentation rollups, the single-port-locality node-scoped
proxy, and the project-dashboard mount seam are all decided and grounded in the
live V1 gateway code. Genuinely-downstream opens, all flagged: (a) mesh-core must
confirm the browser-HTTP-listener + router-mount seam (co-batch, batch 1 already
designed — friction point); (b) the mixed-version `SurfaceSchema.v` LWW edge ties
to `supervision`'s OPEN mixed-version protocol (open question); (c) the exact
`TopicFilter` shape the browser uses is whatever `types::pubsub` finalizes
(pubsub-relay and types differ on scoped-vs-flat filters — a harmonizer call I
consume, do not re-litigate); (d) whether `inference-events`/`gc-events`/
`ccd-events` fully collapse into pubsub-protocol topic pointers is a per-pair
round call (recommended below).

## Assigned design-depth

Opus, single strong-model Component-Designer pass (this file), grounded in the
live `bin/gateway/src/{main,hub,ws,upstream,proxy,stats,topics,state,config}.rs`,
the batch-1/2 designs (`mesh-core.md` ring/seam architecture, `pubsub-relay.md`
protocol + lossy semantics, `service-registry.md` resolve/list surface,
`types.md` `surface`/`pubsub`/`event`/`node` modules), `dashboard.md` (the batch-6
consumer), `mesh.md` concern 5, and INTENT items 5/24/35/36/37/39/46/47/54/58.

## Suggested fill-model

**implementation-ready + moderate complexity → mid model OK.** The static
serving (tower-http `ServeDir`), the browser WS loop (near-transcription of V1
`ws.rs`, now backed by `trait PubSub`), the rollups (`stats.rs`), and the
JSON-shaped manifest assembly are boring, well-precedented fills. Two carve-outs
for a careful hand: (1) the **node-scoped proxy's HTTP↔mesh-`Request`/`Response`
translation** must go through mesh-core's Dispatcher (single-port locality), NOT
a fresh `reqwest` to a remote host as V1 `proxy.rs` did — do not let a cheap model
transcribe the old direct-HTTP proxy; (2) the **per-browser pubsub-relay
subscription lifecycle** (create on connect, mutate on subscribe/unsubscribe,
tear down on disconnect with no task leak) is the one spot lagged/dropped
handling and cleanup must be exact. Neither needs a Design Mesh.

---

## Proposed contracts (wave 2)

Proposals only; the per-pair round reconciles both sides. Structs live in `types`
(`surface.rs`, `pubsub.rs`, `event.rs`, `node.rs`) — proposed there by the `types`
designer; I propose the **serving/aggregation behavior** here. I do NOT edit
`scaffold/contracts/*`.

### `dashboard-feed` (mesh/dashboard-serving → dashboard frontend) — THE batch-6 seam

**Purpose.** The complete browser-facing surface the Svelte dashboard builds
against: static assets, the read-only pub/sub event feed, the schema-driven
render manifest, presentation rollups, and the same-origin node-scoped proxy —
all on one browser HTTP origin per node (concern 1). This is the seam contract
batch 6 develops against before any code exists.

**HTTP surface (the browser origin, registered as the `dashboard` slug):**

```
GET  /                       -> static ui/dashboard/dist/ (SPA fallback to index.html)
GET  /health                 -> { status, version, node_id }
GET  /events                 -> WebSocket: read-only pubsub-relay profile (below)
GET  /api/surface            -> DashboardManifest              (schema-driven render source)
GET  /api/nodes              -> Vec<NodeInfo>                  (fleet roster)
GET  /api/mesh/stats         -> MeshStats                     (fleet aggregate)
GET  /api/nodes/:id/stats    -> NodeStats                     (per-node disk/gpu/gc)
ANY  /api/nodes/:id/:slug/*  -> same-origin proxy -> resolve(slug,id) -> mesh relay
```

**WS frames (`GET /events`) — reused verbatim from `pubsub-relay`, Publish gated:**

```rust
// browser -> daemon : exactly types::pubsub::PubSubClientMsg, but Publish is rejected
//   Subscribe   { filters: Vec<TopicFilter> }   // additive; incl. Exact per-completion (INTENT #5)
//   Unsubscribe { filters: Vec<TopicFilter> }
//   Publish(..)                                 // -> Error{NotRegistered} (read-only observer)
// daemon -> browser : exactly types::pubsub::PubSubServerMsg
//   Event(Envelope)                             // node_id read from provenance.origin_node
//   SubAck { active: Vec<TopicFilter> }
//   Lagged { dropped: u64, since_seq: u64 }     // lossy, stays connected
//   Error  { code: PubSubError, detail: String }
```

**Manifest + rollup structs I propose (land in `types`, likely `surface.rs`/`node.rs`):**

```rust
struct DashboardManifest {
    v: u16,
    generated_at: DateTime<Utc>,
    nodes: Vec<NodeInfo>,           // roster (also at /api/nodes)
    surfaces: Vec<SurfaceSchema>,   // one per live slug (incl. project dashboards)
    navigation: Vec<NavEntry>,      // browsable tree: core / service / project (concern 6)
}
struct NavEntry { id: String, title: String, kind: NavKind, surface_slug: Option<Slug>,
                  children: Vec<NavEntry> }
enum   NavKind  { MeshCore, Service, Project }
struct MeshStats { nodes_total: u32, nodes_up: u32, nodes_down: u32, services_live: u32 }
struct NodeStats { disk: DiskStats, gpu: GpuStats }        // from replicated NodeInfo.last_state
struct DiskStats { total_bytes: u64, used_bytes: u64, free_bytes: u64, gc_managed_bytes: u64 }
struct GpuStats  { utilization_fraction: f32, is_estimate: bool }   // is_estimate -> tilde+tooltip (#9)
```

**Error cases.**
- WS: `PubSubError::{NotRegistered (browser Publish), InvalidFilter, InvalidTopicPath}`;
  `Lagged` is a notice, not an error (lossy contract).
- `GET /api/nodes/:id/stats` for an unknown/offline node → `404` (node absent from
  roster) vs. a `pending`/`offline`-flagged entry if known-but-stale (so the
  frontend distinguishes "never seen" from "fell off the mesh" — `dashboard.md`
  concern 5).
- Proxy `/api/nodes/:id/:slug/*`: `404` `NoSuchSlug`/`NoLiveInstance` (registry);
  `502`/`503` on relay failure or peer-unreachable (surfaces mesh-core's
  `PeerUnreachable` as a clean HTTP status, never a hang); `504` on relay timeout.
- Manifest: a service live in the registry but with no published surface yet is
  simply omitted from `surfaces` (not an error) — it appears once it publishes.

**Version-sensitivity.** MEDIUM. The WS wire is `pubsub-protocol`'s (HIGH there,
payload-opaque decoupling carries the version safety). `DashboardManifest.v` and
`SurfaceSchema.v` are additive-only, `#[serde(default)]`; the browser is a
single-build client of its local mesh origin, so it never straddles two
`types` versions on the wire *except* through relayed `Envelope`s (governed by
pubsub-protocol's discipline). REST rollup shapes evolve additively.

### `surface-schema` (every service → mesh dashboard) — the serving/aggregation half

**Purpose.** Every service publishes a boring `SurfaceSchema`; dashboard-serving
aggregates the fleet's schemas into the render manifest (INTENT #46). The struct
half is `types::surface` (the `types` designer's proposal — `SurfaceSchema`,
`SurfaceSection`, `SurfaceField`, `SurfaceAction`, `DataSource`, `Honesty`, …
with stable agent-drivable ids per INTENT #16); I propose the behavior.

**Publication + aggregation sketch.**
- **Publish:** a service calls `mesh-client`'s surface-publication method (part of
  its universal-protocol surface) → the local daemon writes
  `replicated-kv["surface/<slug>"] = SurfaceSchema` (LWW, slug-keyed, concern 4).
  Republishing on version change is a plain LWW `put`.
- **Aggregate:** dashboard-serving joins `service-registry.list()` × `surface/*` ×
  roster → `DashboardManifest`; emits `dashboard.surface.changed` on the feed when
  the set changes so the frontend re-fetches `/api/surface`.
- **`DataSource` resolution:** `Rest { path }` fields resolve against
  `/api/nodes/:id/:slug/*`; `PubSub { topic }` fields resolve against a `/events`
  subscription — so the schema is self-describing for the renderer.

**Error cases.** A malformed published schema → dashboard-serving omits it and
raises `MeshError::MalformedSurfaceSchema { service }` (mesh side; no `types`
error). A service that never publishes is simply absent from the manifest —
reconciliation-by-omission, never a hard failure.

**Version-sensitivity.** MEDIUM-HIGH. `SurfaceSchema.v` keys the component-
versioning story (INTENT #37); slug-keyed LWW means a mixed-version fleet renders
the convergence-winning `v` (concern 4 open question). The frontend must render
an older schema and ignore unknown `SectionKind`/`ValueType` variants gracefully
(`#[serde(other)]`), so a newer service's richer schema never breaks an older
dashboard build.

### `inference-events` / `gc-events` / `ccd-events` (mesh ← inference/gc/ccd) — RE-GROUND onto pubsub-protocol

**Purpose.** The per-service event streams the dashboard feed carries: inference's
completion/model/backend/queue/telemetry events (`inference.*`), gc's
directory/entry/reclaim events (`gc.*`), and ccd's agent-lifecycle/run events
(`ccd.*`). Wave-1 modeled these as three independent gateway-scraped WS surfaces;
**wave-2 re-grounds all three as EventType catalogs published on their reserved
topic prefixes over `pubsub-protocol`** — the same collapse `pubsub-relay` calls
for ("those contracts become examples of payloads on this envelope, not
independent wire formats") and that `service-registry` did for
`registry-replication → kv-replication`.

**Struct sketch.** No new wire. Each carries `types::event::Event<P>` inside a
`types::pubsub::Envelope`, on topic `-<service>.<domain>.<verb>` (per-completion is
the leaf `inference.completion.<id>`). The **EventType catalog** (which
`domain.noun.verb` identifiers exist, and each payload `P`) is the *publisher's*
to enumerate — inference in batch 5, gc in batch 3, ccd in batch 6 — not
dashboard-serving's; dashboard-serving is the mesh-side **subscriber** that folds
them into the browser feed. Recommendation for the per-pair round: mark
`scaffold/contracts/{inference-events,gc-events,ccd-events}.md` as
**topic-prefix + EventType-catalog pointers into `pubsub-protocol`** (like the
`mesh-registry-read` tombstone / `kv-replication` collapse), each owning only its
catalog table, not a parallel wire.

**Error cases.** None owned by dashboard-serving on these edges — it is a pure
subscriber; delivery is lossy by pubsub-relay's contract (a `Lagged` notice, never
a failure). An unknown `EventType` on a subscribed topic is passed through to the
browser opaque (the relay never parses payloads), never dropped as an error.

**Version-sensitivity.** HIGH but delegated: it is `pubsub-protocol`'s payload-
opaque decoupling (new event types cross old daemons untouched). `ccd-events`
additionally still carries the wave2-plan "proposed, pending confirmation" marker
(flag #6) — confirm-or-strike in ccd's batch-6 pass.
