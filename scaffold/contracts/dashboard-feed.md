# Contract: dashboard-feed

## Parties
- **Server:** `mesh` (`dashboard-serving`, L2) — serves the browser origin,
  registered under the `dashboard` slug (one HTTP origin per node).
- **Client:** `dashboard` (L5, the Svelte/Vite frontend served BY mesh from
  `ui/dashboard/dist/`).

*(was `gateway -> dashboard`; gateway merged into mesh, 2026-07-18)*

## Purpose
The complete browser-facing surface the Svelte dashboard builds against: static
asset hosting, a read-only pub/sub event feed, the schema-driven render manifest,
presentation rollups, and a same-origin node-scoped proxy — all on ONE browser
HTTP origin per node. This is THE batch-6 seam both sides authored against
(`dashboard-serving.md` proposes the serving half; `dashboard.md` proposes the
consumer half; they agree — this file is their reconciliation). The dashboard is
schema-driven: it discovers services via mesh and renders each service's
component from that service's published `SurfaceSchema` (see `surface-schema.md`),
never hand-built per-service.

## Schema

### HTTP surface (the browser origin, `dashboard` slug)

```
GET  /                       -> static ui/dashboard/dist/ (SPA fallback to index.html)
GET  /health                 -> { status: String, version: String, node_id: NodeId }
GET  /events                 -> WebSocket: read-only pubsub-relay profile (below)
GET  /api/surface            -> DashboardManifest        (schema-driven render + nav source)
GET  /api/nodes              -> Vec<NodeInfo>            (fleet roster; drives per-node grid)
GET  /api/mesh/stats         -> MeshStats               (fleet header aggregate)
GET  /api/nodes/:id/stats    -> NodeStats               (per-node disk/gpu/gc)
ANY  /api/nodes/:id/:slug/*  -> same-origin proxy -> resolve(slug,id) -> mesh relay
```

### WS frames (`GET /events`) — reused verbatim from `pubsub-relay`, Publish gated

```rust
// browser -> daemon : exactly types::pubsub::PubSubClientMsg, but Publish is REJECTED
//   Subscribe   { filters: Vec<TopicFilter> }   // Prefix("inference"|"gc"|"ccd"|"network"|"dashboard")
//                                                //   + Completion(id) for the per-completion view (INTENT #5)
//   Unsubscribe { subscriber_id: Uuid }
//   Publish(..)                                 // -> Error { code: NotRegistered } (read-only observer)
// daemon -> browser : exactly types::pubsub::PubSubServerMsg
//   Delivery(Envelope)                          // node_id := Envelope.provenance.origin_node
//   Ack { envelope_id: Uuid }
//   Error { code: PubSubError, detail: String } // NotRegistered on any accidental Publish
//   // Lagged surfaces via the lossy contract as a "N dropped" ticker, never a reconnect
```

### Manifest + rollup structs (land in `types`, `surface.rs` / `node.rs`)

```rust
pub struct DashboardManifest {
    pub v: u16,
    pub generated_at: DateTime<Utc>,
    pub nodes: Vec<NodeInfo>,           // roster (also at /api/nodes)
    pub surfaces: Vec<SurfaceSchema>,   // one per live slug (incl. project dashboards)
    pub navigation: Vec<NavEntry>,      // browsable tree: core / service / project
}
pub struct NavEntry { pub id: String, pub title: String, pub kind: NavKind,
                      pub surface_slug: Option<Slug>, pub children: Vec<NavEntry> }
pub enum   NavKind  { MeshCore, Service, Project }
pub struct MeshStats { pub nodes_total: u32, pub nodes_up: u32, pub nodes_down: u32,
                       pub services_live: u32 }
pub struct NodeStats { pub disk: DiskStats, pub gpu: GpuStats }  // from replicated NodeInfo.last_state
pub struct DiskStats { pub total_bytes: u64, pub used_bytes: u64, pub free_bytes: u64,
                       pub gc_managed_bytes: u64 }
pub struct GpuStats  { pub utilization_fraction: f32, pub is_estimate: bool } // is_estimate -> tilde+tooltip
```

### Frontend consumption guarantees (consumer-side conformance, from `dashboard.md`)
- Reads per-node identity ONLY from `Envelope.provenance.origin_node`; never
  invents or re-stamps a `node_id`.
- Distinguishes `pending` / `live` / `offline` / `unknown(self_offline peer)`
  per node from the roster + `network.*` stream; never renders "no data yet" and
  "fell off the mesh" identically.
- On `Lagged`, stays connected and keeps rendering; never clears state or forces
  a reconnect.
- Renders `GpuStats.is_estimate == true` (and `Honesty::Estimate`) as
  **tilde + hover tooltip** (INTENT #9); never presents an estimate as exact.
- Discovers the dashboard origin via mesh; hardcodes no port (dev proxy targets
  the discovered origin, default `:3648`).
- Generates stable agent-drivable element ids from the schema (INTENT #16):
  `${service}-${section.id}[-${field.id}|-${action.id}]-${node_id}`.

## Error cases
- **WS:** `PubSubError::{ NotRegistered (browser Publish), InvalidFilter,
  InvalidTopicPath }`. `Lagged` is a notice, not an error (lossy contract).
- **`GET /api/nodes/:id/stats`:** `404` for an unknown node (absent from the
  roster) vs. a `pending`/`offline`-flagged entry for known-but-stale — so the
  frontend distinguishes "never seen" from "fell off the mesh".
- **Proxy `/api/nodes/:id/:slug/*`:** `404` `NoSuchSlug`/`NoLiveInstance`
  (registry); `502`/`503` on relay failure or peer-unreachable (surfaces
  mesh-core's `PeerUnreachable` as a clean HTTP status, never a hang); `504` on
  relay timeout. The frontend renders these as an inline panel error.
- **Manifest:** a service live in the registry but with no published surface yet
  is simply **omitted** from `surfaces` (reconciliation-by-omission, never a hard
  error); it appears once it publishes. A malformed published schema is omitted
  on the serving side (mesh raises `MeshError::MalformedSurfaceSchema { service }`)
  and, if one nonetheless reaches the frontend, renders as a raw key/value
  fallback + log, never a crash.

## Version sensitivity
**MEDIUM.**
- The WS wire is `pubsub-protocol`'s (HIGH there; payload-opaque decoupling
  carries the version safety — new event types cross old daemons untouched).
- **Additive-safe:** `DashboardManifest.v` / `SurfaceSchema.v` bumps with
  `#[serde(default)]` new fields; new REST rollup fields (frontend ignores
  unknown JSON fields); new `NavKind`/`SectionKind`/`ValueType` variants
  (`#[serde(other)]`, rendered via the graceful-degradation fallback).
- **Breaking:** removing a REST route, changing an existing field's type, or a
  non-additive `PubSubServerMsg` change.
- The browser is a single-build client of its local mesh origin, so it straddles
  two `types` versions only through relayed `Envelope`s (governed by
  pubsub-protocol). During a mixed-version rolling update the slug-keyed LWW
  `surface/*` store may serve whichever `SurfaceSchema.v` won convergence
  (`dashboard-serving.md` concern 4, tied to `supervision`'s OPEN mixed-version
  protocol, INTENT #66); the frontend renders whatever `v` it receives via the
  graceful-degradation rule and does NOT reconcile two `v`s.

## Reconciliation notes
- **Both batch-6 designs agree on the seam** — this file merges compatible
  proposals, no dispute to arbitrate. `dashboard-serving.md` authored the HTTP
  surface / WS frames / manifest structs (the serving half); `dashboard.md`
  authored the consumption guarantees (the consumer half). The two halves are
  field-for-field consistent (identical HTTP route list, identical WS profile,
  identical error semantics); reconciliation was verification, not arbitration.
- **Struct home:** `DashboardManifest`, `NavEntry`, `MeshStats`, `NodeStats`,
  `DiskStats`, `GpuStats` land in `types` (`surface.rs`/`node.rs`), proposed by
  both dashboard designers and adopted here. `SurfaceSchema`, `NodeInfo`,
  `PubSubClientMsg`/`PubSubServerMsg`, `TopicFilter` are existing `types`
  vocabulary reused unchanged.
- **`/events` is a read-only profile of `pubsub-relay`, not a new wire.** The
  browser sends exactly `PubSubClientMsg` but `Publish` is gated to
  `Error{NotRegistered}` — deviation from a generic relay client is
  behavioral (Publish rejection), not structural.
- **REST proxy is aggregation, never routing** (`dashboard-serving.md`): the
  `ANY /api/nodes/:id/:slug/*` proxy resolves + relays through mesh; it does not
  itself become a routing layer. This subsumes the wave-1 per-service "REST
  proxy" affordances (`inference-events`/`gc-events`/`ccd-events`) into one
  same-origin path.

## Example data
Fleet = **macbook** + **pi**. Browser loads `http://macbook:3648/`, then:

`GET /api/mesh/stats`:
```json
{ "nodes_total": 2, "nodes_up": 2, "nodes_down": 0, "services_live": 7 }
```

`GET /api/nodes/pi/stats` (pi is a cold-storage node with a GPU estimate):
```json
{ "disk": { "total_bytes": 2000000000000, "used_bytes": 640000000000,
            "free_bytes": 1360000000000, "gc_managed_bytes": 512000000000 },
  "gpu":  { "utilization_fraction": 0.12, "is_estimate": true } }
```
The frontend renders pi's GPU as `~12%` with a tooltip (INTENT #9).

`GET /api/surface` returns a `DashboardManifest` whose `navigation` includes a
`NavKind::Service` entry for `ccd` (surface_slug `ccd`) and a `NavKind::Project`
entry for **demo**; `surfaces` carries the `SurfaceSchema` each live service
published (inference on macbook advertises model `qwen3-4b` in its schema fields).

WS session on `/events`:
```
browser -> { "Subscribe": { "filters": [ { "Prefix": "ccd" }, { "Prefix": "network" } ] } }
daemon  -> { "Ack": { "envelope_id": "…" } }
daemon  -> { "Delivery": { "topic": "ccd/macbook/agent",
                           "provenance": { "origin_node": "macbook" },
                           "payload": { "event_type": "ccd.agent.started", … } } }
```
The frontend keys that delivery to the macbook cell of the CCD panel. If the
browser accidentally sends `Publish`, the daemon answers
`{ "Error": { "code": "NotRegistered", "detail": "read-only observer" } }`.
