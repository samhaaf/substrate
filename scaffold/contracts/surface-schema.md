# Contract: surface-schema

## Parties

- **every service** (via `mesh-client`) — publishes a `SurfaceSchema` describing its observable surface
- **mesh** (`dashboard-serving`, the L4 internal library) — aggregates schemas into the render manifest and serves the dashboard

Cross-cutting: ONE shared document, every service is a party (the original
round-3 pattern this whole style is named after). Struct vocabulary is homed in
`types::surface`; the aggregation/serving behavior is `dashboard-serving`'s; the
publication path is `mesh-client`'s (a pass-through).

## Purpose

Every service publishes a **boring schema** of its observable surface — a data
structure describing (a) how to render its dashboard component and (b) what calls
to make against it (INTENT #46, LOCKED). The mesh dashboard renders every
service's component *as a pure function of its published schema* — visual
coherence by construction, never per-service custom UI. Two INTENT requirements
are structural, not afterthoughts: **stable semantic ids on every interactive
element** (INTENT #16 — agents drive the same markup humans see) and
**honest-estimate markers** (INTENT #9 — tilde + tooltip for compromised readings
like Apple-Silicon GPU). Projects use the same mechanism to make project-published
dashboards navigable from the main mesh dashboard.

## Schema

### The published schema (`types::surface`)

```rust
pub struct SurfaceSchema {
    pub v: u16,                        // schema version (component-versioning, INTENT #37)
    pub service: String,               // slug
    pub title: String,
    pub sections: Vec<SurfaceSection>, // render layout
    pub actions: Vec<SurfaceAction>,   // what calls to make against the service
}
pub struct SurfaceSection {
    pub id: String,                    // stable semantic id (agent interface)
    pub title: String,
    pub kind: SectionKind,             // KeyValue | Table | TimeSeries | Custom { renderer }
    pub fields: Vec<SurfaceField>,
    pub data_source: DataSource,       // where the dashboard pulls values
}
pub struct SurfaceField {
    pub id: String,                    // stable semantic id
    pub label: String,
    pub value_type: ValueType,         // Int|Float|Bool|Text|Bytes|Percent|Timestamp|Enum(Vec<String>)
    pub unit: Option<Unit>,
    pub honesty: Option<Honesty>,      // Estimate { note } -> render tilde + tooltip (INTENT #9)
}
pub struct SurfaceAction {
    pub id: String,                    // stable semantic id (e.g. "submit")
    pub label: String,
    pub call: ActionCall,              // { method: Get|Post|Ws, path_or_topic, .. }
    pub inputs: Vec<SurfaceField>,
    pub confirm: bool,
}
pub enum DataSource { Rest { path: String }, PubSub { topic: Topic }, Static }
pub enum Honesty    { Exact, Estimate { note: String } }
pub enum SectionKind { KeyValue, Table, TimeSeries, Custom { renderer: String }, #[serde(other)] Unknown }
```

### Publication (client half — `mesh-client`, pass-through)

```rust
struct PublishSurface { schema: SurfaceSchema }
// The schema MAY be supplied at Register (see service-lookup) and updated live via
// publish_surface(schema); the client re-publishes on every reconnect.
```

### Aggregation + serving (`dashboard-serving`)

- **Publish:** a service calls `mesh-client`'s surface-publication method → the
  local daemon writes `replicated-kv["surface/<slug>"] = SurfaceSchema` (LWW,
  slug-keyed). Republishing on a version change is a plain LWW `put`.
- **Aggregate:** `dashboard-serving` joins `service-registry.list()` × `surface/*`
  × the node roster into a `DashboardManifest` (served at `GET /api/surface`), and
  emits `dashboard.surface.changed` on the pub/sub feed when the set changes so the
  frontend re-fetches.
- **`DataSource` resolution (self-describing for the renderer):** `Rest { path }`
  fields resolve against the same-origin proxy `/api/nodes/:id/:slug/*`;
  `PubSub { topic }` fields resolve against a `/events` subscription.

```rust
struct DashboardManifest {
    v: u16,
    generated_at: DateTime<Utc>,
    nodes: Vec<NodeInfo>,           // roster (also at /api/nodes)
    surfaces: Vec<SurfaceSchema>,   // one per live slug (incl. project dashboards)
    navigation: Vec<NavEntry>,      // browsable tree: core / service / project
}
struct NavEntry { id: String, title: String, kind: NavKind,
                  surface_slug: Option<Slug>, children: Vec<NavEntry> }
enum   NavKind  { MeshCore, Service, Project }
```

## Error cases

- A **malformed** published schema → `dashboard-serving` omits it from the
  manifest and raises `MeshError::MalformedSurfaceSchema { service }` (mesh side;
  there is **no `types` error** for this — it is mesh's to raise).
- A service that **never publishes** is simply absent from the manifest —
  reconciliation-by-omission, never a hard failure; it appears once it publishes.
- Publication is **best-effort**: a rejected/failed publish degrades observability
  only, never the service's function; the client retries on reconnect.

## Version sensitivity

**MEDIUM-HIGH.**
- `SurfaceSchema.v` keys the component-versioning story (INTENT #37). Slug-keyed
  LWW means a mixed-version fleet renders the convergence-winning `v`.
- **Additive-safe:** new `#[serde(default)]` fields; new `#[serde(other)]`-tolerant
  `SectionKind` / `ValueType` variants. The frontend MUST render an older schema
  and ignore unknown `SectionKind` / `ValueType` variants gracefully
  (`#[serde(other)] => Unknown/Custom`), so a newer service's richer schema never
  breaks an older dashboard build.
- The publication client adds **no** version surface of its own beyond the
  enclosing transport `protocol_version` — it is a pure carrier of whatever
  `types::surface::SurfaceSchema` is.
- **Breaking:** removing or repurposing a field id / action id that agents drive
  against (INTENT #16 — the ids are the agent contract), or changing `DataSource`
  resolution semantics.

## Reconciliation notes

- **Three concordant proposers, no disagreement.** `types` owns the struct
  (`SurfaceSchema` … with stable ids and honesty markers); `dashboard-serving`
  owns aggregation + serving; `mesh-client` owns the pass-through publication.
  Each proposed only its own half and they compose cleanly — merged as-is.
- **Replaces the round-3 requirements-only stub.** The prior stub deferred schema
  and example ("Schema/example deferred. requirements-only."); this file supplies
  both. The stub's still-relevant framing (boring surface schema; projects reuse
  the mechanism; visual coherence by construction) is preserved above.
- **Error home settled:** malformed-schema is `MeshError::MalformedSurfaceSchema`
  on the mesh side, NOT a `types` error — all three proposers agreed the client
  side has no runtime error class (a malformed schema is a compile-time concern in
  `types`), so the only runtime error lives where aggregation happens.
- **Transport note:** the publication rides `pubsub-protocol` as an
  `Envelope<SurfaceSchema>` (a schema push) — the same wire wrapper events use,
  per `types`' "one wire wrapper" property. The `GET /events` browser feed
  (`dashboard-feed`, a separate pair) reuses `pubsub-protocol` verbatim with
  `Publish` gated off; that pair is out of this cluster and cross-referenced only.

## Example data

The `inference` service on node **pi** publishes its surface (project `demo`,
model `qwen3-4b`). Note the honest GPU estimate marker (INTENT #9) and the stable
`submit` action id (INTENT #16).

```jsonc
{ "v": 1, "service": "inference", "title": "Inference (qwen3-4b)",
  "sections": [
    { "id": "throughput", "title": "Throughput", "kind": "TimeSeries",
      "data_source": { "PubSub": { "topic":
        { "scope": { "Node": "pi" }, "path": "inference.telemetry.throughput" } } },
      "fields": [
        { "id": "tokens_per_sec", "label": "tok/s",
          "value_type": "Float", "unit": "PerSecond", "honesty": "Exact" },
        { "id": "gpu_util", "label": "GPU util",
          "value_type": "Percent", "unit": "Percent",
          "honesty": { "Estimate": { "note": "Apple-Silicon GPU reading is approximate" } } } ] },
    { "id": "queue", "title": "Queue", "kind": "KeyValue",
      "data_source": { "Rest": { "path": "/status/queue" } },
      "fields": [
        { "id": "depth", "label": "Depth", "value_type": "Int", "unit": null, "honesty": "Exact" } ] } ],
  "actions": [
    { "id": "submit", "label": "Run completion",
      "call": { "method": "Post", "path_or_topic": "/v1/completions" },
      "inputs": [
        { "id": "prompt", "label": "Prompt", "value_type": "Text", "unit": null, "honesty": null } ],
      "confirm": false } ] }
```

`dashboard-serving` on **macbook** folds this into the manifest served at
`GET /api/surface`; the browser renders the `inference` card from it, subscribing
to `inference.telemetry.throughput` (scoped to pi) for the live series and
resolving `/status/queue` through `/api/nodes/pi/inference/status/queue`. The
`gpu_util` field renders as `~48%` with the estimate tooltip.
