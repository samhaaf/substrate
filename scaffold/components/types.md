# types

## Charter

`substrate-types` (`lib/types`) is the zero-dependency shared-contract-types
foundation at the bottom of the workspace's dependency graph: every other crate
compiles it in, and it imports no other `substrate-*` crate. It holds only
**data** — IDs, enums, request/response/event structs, the workspace error
taxonomy (`SubstrateError`/`Result<T>`), the `Promise`/`PromiseSender` handoff
primitive, and — new in wave 2 — the shared vocabulary of the OS-wide protocols
that every service speaks: the WS pub/sub envelope, the standardized typed
event, the boring surface schema, the graceful-restart messages, and the
enriched mesh-node identity/capabilities. Organized **one module per domain**.
It owns **no I/O, no async business logic, no service behavior, and no opinion
about how any other crate uses these types** — it is pure vocabulary. Every
`scaffold/contracts/<edge>.md` schema section is expressed IN TERMS OF this
crate; `types` is not itself a party to any request/response edge — it is the
shared language every edge's schema is written in. Its boundary (what it does
NOT own): it does not define the *behavior* of any protocol (mesh relays
envelopes, queues route events, supervision drives the restart ladder — those
live in `mesh` and its internal libs); it does not hold any service's private
event *catalog* (event kinds are open namespaced identifiers, not a closed enum
that `types` would have to enumerate); and it never learns another crate's error
types (they stringify at the boundary).

Wave-2 status: **UNLOCKED (INTENT #26)** — the round-3 as-is freeze is gone;
additions and updates are welcome, governed by the anti-dumping-ground
guardrails below (which stand and gain a fourth). Everything in this pass is
designed for real, to `implementation-ready`.

## Primary design concerns

Low complexity per individual type; the hard part is discipline at the ONE seam
every crate touches, where entropy is cheapest to introduce and most expensive
to reverse (a correction here is a breaking change felt by every downstream
crate at once). Four guardrails, in order of teeth:

1. **Zero-dependency invariant — a hard line, not a guideline.** Permitted deps
   only: `serde`, `serde_json`, `uuid`, `chrono`, `thiserror`, and `tokio` (used
   ONLY for `oneshot` inside `promise.rs`, never as an async runtime). It must
   NEVER depend on another `substrate-*` crate (the cycle the whole design
   avoids) or on an I/O-flavored crate (`reqwest`, `axum`, `rusqlite`, `sqlx`,
   `tungstenite`, …). Downstream crates convert native errors to a `String`
   payload rather than this crate learning their error types. The single most
   important thing a reviewer checks on any PR here.

2. **Inclusion test, not vibes.** A type earns a place here only if it appears
   in the public signature of **two or more** crates, or is literally part of a
   named contract edge's schema in `scaffold/contracts/`. A type merely
   "convenient to share" but used by exactly one crate stays in that crate. This
   is the yes/no answer that replaces a judgment call every time someone is
   tempted to add "just one more struct" here.

3. **One module per domain — no `common.rs`/`misc.rs` catch-all, ever.** Each
   module owns exactly one domain; a new cross-cutting concern gets its OWN new
   module, never fields bolted onto an unrelated existing one. A PR that adds a
   foreign-domain type into an existing module, or adds a grab-bag module, is
   the one shape of change to reject on sight. Applied one level down inside
   `error/` too (see the error restructuring below): each error domain is a
   sub-enum in its own file, not more flat leaves on one enum.

4. **NEW (wave 2) — wire-crossing structs are version-tolerant by construction.**
   `types` is a shared lib compiled into every crate, so within a single build
   every crate agrees on its shape and needs no runtime version negotiation
   (INTENT #45). But a subset of these structs **cross the wire between nodes**
   that may be running different commits (envelopes, events, surface schemas,
   restart messages, node capabilities all travel node-to-node through mesh).
   Those structs — and ONLY those — carry an explicit compatibility discipline:
   additive-only evolution, every new field `#[serde(default)]`, **never**
   `#[serde(deny_unknown_fields)]` (an older node must tolerate a newer node's
   extra fields), an explicit `v` schema-version integer on the top-level
   envelope/schema types, and enums that cross the wire reserve an `Unknown`/
   catch-all arm (or use `#[serde(other)]`) so a new variant never hard-fails an
   old deserializer. In-process-only types (`promise`) are exempt. This
   guardrail is what lets the mesh do minimal-restart rolling updates (INTENT
   #66/#76) without a flag-day: two nodes on adjacent versions must interoperate.

**Sharper instance of concern 3 — the error taxonomy restructuring (now built
for real).** `SubstrateError` is today ONE flat enum growing by domain-tagged,
string-payload leaves per consuming crate (`Store`, `Db`, `Engine`, `Transport`,
`ServerUnhealthy`, …). That scales O(error-cases), not O(components): every new
V2 component (`mesh`, `vfs`, `secrets`, `ccd`, `kg`, …) would sprinkle more
leaves onto one already-long enum. Wave-2 restructuring (INTENT #26 unlocked
this): `error.rs` becomes an `error/` module directory —

```
error/
  mod.rs        // pub enum SubstrateError + pub type Result<T>
  inference.rs  // InferenceError: completion/collection/model/engine/resource cases
  store.rs      // StoreError
  db.rs         // DbError
  mesh.rs       // MeshError: registry/lease/routing/relay cases (NEW)
  locks.rs      // LockError: incl. the REQUIRED partition-merge variant (NEW, INTENT #84)
  vfs.rs        // VfsError (NEW, batch-3)
  secrets.rs    // SecretsError (NEW, batch-3)
  // … one submodule per error domain, added as its component is designed
```

The top-level enum holds **one variant per domain wrapping that domain's
sub-enum**, plus the genuinely cross-cutting leaves that belong to no single
domain:

```rust
#[derive(Debug, Error)]
pub enum SubstrateError {
    #[error(transparent)] Inference(#[from] inference::InferenceError),
    #[error(transparent)] Store(#[from] store::StoreError),
    #[error(transparent)] Db(#[from] db::DbError),
    #[error(transparent)] Mesh(#[from] mesh::MeshError),
    #[error(transparent)] Locks(#[from] locks::LockError),
    // cross-cutting, domain-less:
    #[error("invalid request: {0}")] InvalidRequest(String),
    #[error("config error: {0}")]    Config(String),
    #[error("IO error: {0}")]        Io(#[from] std::io::Error),
    #[error("serialization error: {0}")] Serialization(#[from] serde_json::Error),
    #[error("internal error: {0}")]  Internal(String),
}
```

Now the top-level match arms grow at O(components) while each domain's detail
lives in its own file. The partition-merge error (INTENT #84 — "lock threshold
exceeded because two partitions merged," catchable, handled per-application) is
a named variant on `LockError`, not a stringly leaf — it is a first-class,
matchable error because applications branch on it.

**Migration cost is real and is the operator's call** — see Controversial
decisions. The existing flat inference/store/db leaves must move into their
sub-enums, which touches every `SubstrateError::Store(...)` construction site
across `inference`/`db`/`store`. This design specifies the target end-state;
whether wave-2 migrates the existing leaves now or only routes NEW domains
through sub-enums (leaving old leaves flat until each owning component is
re-touched) is flagged, not silently chosen.

## New wave-2 modules (all `implementation-ready`)

Sketches are Rust-flavored; exact field names are the harmonizer's to final-tune,
but shapes below are meant to be buildable as written.

### `provenance.rs` — first-order provenance (INTENT #85)

Provenance is first-order "from the very beginning," so it is its own module,
shared by both the envelope and the event (and later by VDB/KG handler traces).
Defining it once here prevents three modules from each inventing a
causation/correlation triple.

```rust
pub struct Provenance {
    pub origin_node: NodeId,           // which device emitted it
    pub origin_service: String,        // slug of the emitting service
    pub emitted_at: DateTime<Utc>,
    pub causation_id: Option<Uuid>,    // the envelope/event that directly caused this
    pub correlation_id: Option<Uuid>,  // the root of the causal chain (stable across hops)
    #[serde(default)] pub hops: Vec<Hop>, // relay path through the mesh (optional, additive)
}
pub struct Hop { pub node: NodeId, pub at: DateTime<Utc> }
```

`causation_id` + `correlation_id` are exactly the causal-chain substrate the
execution-engine's loop-detection (INTENT #70) and VDB's healthcare-grade
provenance (INTENT #85/#92) build on — one vocabulary, many consumers.

### `event.rs` — the standardized typed EVENT (INTENT #101)

Operator, verbatim: "I want a standardized struct for event types." The STRUCT
is standardized; the *type* field is an **open namespaced identifier**, not a
closed enum — a closed enum would force `types` to enumerate every service's
event kinds (a dumping-ground violation and a layering inversion). Convention:
`domain.noun.verb` (e.g. `inference.completion.started`, `vfs.file.evicted`).

```rust
pub struct EventType(pub String);   // newtype; documented "domain.noun.verb" convention

pub struct Event<P = serde_json::Value> {
    pub event_id: Uuid,
    pub event_type: EventType,
    pub occurred_at: DateTime<Utc>,
    pub provenance: Provenance,
    pub payload: P,                 // typed at the producer, Value at generic relay/filter sites
}
```

Events are transport-agnostic: they go INTO queues (mesh.queues) and can be the
`payload` of a pub/sub `Envelope`. Triggers (mesh.queues, declarative — INTENT
#101/#103) FILTER on `event_type` + `payload` content and ASSEMBLE the handler
payload. The generic `P = serde_json::Value` default lets mesh's relay and a
trigger's declarative filter operate on events without knowing the producer's
payload type, while a producer/consumer pair can monomorphize `Event<MyPayload>`.

### `pubsub.rs` — the WS pub/sub envelope (INTENT #53)

Operator, verbatim: "we just have certain structs that the publishers and
subscribers expect," mesh relays them "where it needs to go." Topic-based
publish/subscribe with per-completion-ID subscription (INTENT #5).

```rust
pub struct Topic(pub String);      // hierarchical, "/"-delimited, e.g. "inference/node-01/events"

pub struct Envelope<P = serde_json::Value> {
    pub v: u16,                    // schema version (wire-crossing discipline, guardrail 4)
    pub envelope_id: Uuid,
    pub topic: Topic,
    pub published_at: DateTime<Utc>,
    pub provenance: Provenance,
    pub payload: P,                // frequently Event<..>, but any typed struct
}

pub enum TopicFilter {
    Exact(Topic),
    Prefix(String),               // subtree subscription
    Completion(CompletionId),     // per-completion-ID subscription (INTENT #5)
}
pub struct Subscription {
    pub subscriber_id: Uuid,
    pub filters: Vec<TopicFilter>,
}
// client -> mesh control frames on the pub/sub socket
pub enum PubSubClientMsg { Subscribe(Subscription), Unsubscribe { subscriber_id: Uuid }, Publish(Envelope) }
// mesh -> client
pub enum PubSubServerMsg { Delivery(Envelope), Ack { envelope_id: Uuid }, Error { code: PubSubError, detail: String } }
pub enum PubSubError { UnknownTopic, NotSubscribed, PayloadTooLarge, Malformed }
```

Envelope carries provenance (first-order) and `payload` is generic so an
`Envelope<Event<..>>` (event over pub/sub) and an `Envelope<SurfaceSchema>`
(schema push) share one wire wrapper. `v` is the version-tolerance anchor.

### `surface.rs` — the boring surface schema (INTENT #46, #16, #9, #37)

Every service publishes a schema describing (a) how to render its dashboard
component and (b) what calls to make against it; the mesh dashboard renders from
it — visual coherence by construction, never per-service custom UI. Two INTENT
requirements are structural here, not afterthoughts: **stable semantic ids on
every interactive element** (INTENT #16 — agents drive the same markup humans
see) and **honest-estimate markers** (INTENT #9 — tilde + tooltip for
compromised readings like Apple-Silicon GPU).

```rust
pub struct SurfaceSchema {
    pub v: u16,                        // schema version (component-versioning story, INTENT #37)
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
    pub data_source: DataSource,       // where the dashboard pulls values (REST path / pub/sub topic)
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
pub enum Honesty { Exact, Estimate { note: String } }
```

`v` supports the service-component-versioning architecture the operator flagged
as unresolved (INTENT #37): the dashboard can key a rendered component to the
`(service, v)` pair. `honesty` makes the GPU-estimate compromise honest at the
schema level, so every dashboard renders the tilde/tooltip uniformly.

### `restart.rs` — the graceful-restart protocol messages (INTENT #77, #76)

The two-way protocol built into EVERY service. The **4-level ladder is LOCKED**
(INTENT #77/#98). The message shapes (undesigned until now) live here so mesh
and every service share one vocabulary.

```rust
pub enum RestartPriority {           // LOCKED 4-level ladder, ordered
    WaitForIdle,                     // 1: mesh just waits for idle
    FinishAndRelinquish,             // 2: finish current work, then yield (service decides when)
    SaveWindow,                      // 3: interrupting regardless; ~10s to save
    Kill,                            // 4: kill outright, no warning
}
pub enum RestartReason { Compatibility, Update, OperatorRequested, HealthRemediation, PortConflict }
// mesh -> service
pub struct RestartRequest {
    pub v: u16,
    pub request_id: Uuid,
    pub priority: RestartPriority,
    pub reason: RestartReason,        // Compatibility => always HIGH priority
    pub save_deadline: Option<Duration>, // populated for SaveWindow (~10s)
    pub requested_at: DateTime<Utc>,
}
// service -> mesh
pub enum RestartResponse {
    Acknowledged { will_yield_by: Option<DateTime<Utc>> },
    Busy { state: Interruptibility, retry_after: Option<Duration> }, // only honored below SaveWindow
    Yielding,                         // finishing-then-relinquishing in progress
    Saved,                            // state persisted, ready to be replaced
}
// the "observable interruptibility state" mesh polls/subscribes (INTENT #76)
pub enum Interruptibility {
    Idle,
    Interruptible,
    CriticalSection { until: Option<DateTime<Utc>> }, // must not interrupt for non-critical updates
}
// port-handoff choreography (INTENT #76): new port -> registry flip -> old down
pub struct PortHandoff { pub service: String, pub old: SocketHint, pub new: SocketHint, pub at: DateTime<Utc> }
pub struct SocketHint { pub host: String, pub port: u16 } // NOT a live socket — pure data
```

### `node.rs` — mesh node identity + capabilities (resolves OQ-3)

**Decision (OQ-3, previously deferred, resolved now per concern 3):** `NodeId`
and `NodeInfo` **move out of `system.rs` into their own `node.rs` module**, and
the mesh-registry enrichment lands as a sibling `NodeCapabilities` here — NOT as
more fields on `system.rs`'s telemetry snapshot. Reasoning: node identity/
capability is a distinct domain from a telemetry sample (`SystemState`), it now
has multiple consumers (mesh's completion-router affinity balancer,
service-registry, network-topology, vfs's node/drive topology), and the field
count has grown past the "same domain, judged at the time" call that put them in
`system.rs`. Crate-root re-exports (`substrate_types::NodeInfo`) keep every
existing import compiling — the move is additive at the public API.

```rust
pub type NodeId = String;            // Tailscale device name by convention

pub struct NodeInfo {
    pub id: NodeId,
    pub is_self: bool,
    #[serde(default)] pub capabilities: NodeCapabilities, // NEW (additive)
    #[serde(default)] pub last_state: Option<SystemState>,// live telemetry snapshot (unchanged home)
    // `api_url` retained but DEPRECATED: endpoints now resolve via service-registry,
    // not a baked URL. Kept for one transition; slated for removal once mesh-core lands.
    #[serde(default)] pub api_url: Option<String>,
}

pub struct NodeCapabilities {
    #[serde(default)] pub roles: Vec<NodeRole>,           // Inference | Storage | ColdStorage | ...
    #[serde(default)] pub accelerator: Option<Accelerator>,
    #[serde(default)] pub models_available: Vec<ModelId>, // DOWNLOAD inventory (affinity input)
    // NB: RESIDENT models are NOT duplicated here — that is live state, read from
    // `last_state`/SystemState.resident_model. Avoids two sources of truth (see friction note).
}
pub enum NodeRole { Inference, Storage, ColdStorage, Control /* runs mesh utilities */ }
pub struct Accelerator { pub kind: AcceleratorKind, pub memory_bytes: u64 }
pub enum AcceleratorKind { Metal, Cuda, Rocm, None }
```

`models_resident` deliberately omitted (see Controversial decisions #3): the
resident set is live telemetry, sourced from `SystemState`, not static
capability. `DriveInfo`/RAID warm-cold topology (INTENT #48) is **anticipated,
not defined now** — it enters `node.rs` when `vfs` is designed (batch 3) and the
`vfs-mesh` contract names it, satisfying the inclusion test then rather than
speculatively today.

## Relationships / edges

`types` is **not a two-party contract edge** — it has no runtime
request/response of its own; it is the shared vocabulary every OTHER edge in the
contract graph is defined in terms of, and shared-lib consumption is explicitly
NOT a contract edge (wave2-plan §3 note; INTENT #45). It is compiled into all 44
modules. What it owns relative to the contract graph: the **struct halves of the
cross-cutting protocol contracts** — `pubsub-protocol`, `surface-schema`,
`restart-protocol`, and the event/provenance vocabulary inside `queues-api` and
`node-state-poll`/`service-lookup` are all *written in* these `types` structs.
Those proposals are in the section below; the per-pair contract round reconciles
the mesh/service sides against them.

Carried-forward decision points, now resolved or explicitly deferred:

- **OQ-3 (NodeInfo/NodeCapabilities)** — RESOLVED above: own `node.rs` module,
  enrichment as `NodeCapabilities`, resident-set not duplicated.
- **`db-inference-init` payload** — still crate-local to `db` unless a third
  consumer appears (inclusion test); no shared `DbInitSpec` in `types` this
  pass. Flagged, unchanged.

## Nesting

Top-level shared lib (`lib/types`). No parent, no children.

## Thoroughness level

`implementation-ready` — for the existing crate, the four disciplinary
guardrails, the error-module restructuring (target end-state fully specified;
only the migrate-now-vs-later rollout is an operator flag), and all five new
wave-2 modules (`provenance`, `event`, `pubsub`, `surface`, `restart`) plus the
`node.rs` split. The struct shapes are buildable as written; the per-pair
contract round tunes exact field names and the mesh-side behavior, not the
vocabulary.

## Assigned design-depth

Opus (single pass, this file), grounded in the real `lib/types` source and the
mesh/completion-router component designs.

## Suggested fill-model

**implementation-ready + low-to-medium complexity → cheap/fast model OK.** The
new modules are pure data-struct declarations with serde derives — a fast model
can transcribe them directly from this file. Two carve-outs warranting a
slightly stronger hand or a careful reviewer: (1) the `error/` restructuring, if
the operator elects the full migration of existing flat leaves — that is a
mechanical-but-wide change across `inference`/`db`/`store` construction sites,
and a wide change at the ONE seam wants care; (2) applying guardrail 4's serde
attributes (`#[serde(default)]`, no `deny_unknown_fields`, `#[serde(other)]`
arms) uniformly across the wire-crossing modules — easy to get individually,
easy to forget one, so it wants a checklist pass. Neither needs a design-depth
pass; both are conformance-checkable.

---

## Proposed contracts (wave 2)

`types` is a shared lib, so it is not a runtime party to any contract edge.
What it contributes to wave 2 is the **shared struct vocabulary** that the
cross-cutting, surface-schema-style protocol contracts are authored in. For each
such contract that my modules ground, I propose the type-shape half below.
Behavior (relay, routing, supervision, dashboard rendering) is the mesh side's
proposal; these are reconciled in the per-pair round. I do NOT edit
`scaffold/contracts/*` — these proposals live here only.

### `pubsub-protocol` (every service ↔ mesh) — grounded by `pubsub.rs` + `event.rs` + `provenance.rs`

- **Purpose:** the standard WS pub/sub wire vocabulary — typed envelopes,
  topics, subscription filters, control frames — that mesh relays "where it
  needs to go" (INTENT #53).
- **Structs (from `types`):** `Envelope<P>`, `Topic`, `TopicFilter`,
  `Subscription`, `PubSubClientMsg`, `PubSubServerMsg`, `Provenance`, and
  `Event<P>` as the common payload. Wire form: JSON, tagged enums
  (`#[serde(tag="type")]`) matching the existing `stream.rs` convention.
- **Error cases:** `PubSubError { UnknownTopic, NotSubscribed, PayloadTooLarge,
  Malformed }` on the server frame; transport/relay failures surface as
  `SubstrateError::Mesh(MeshError::…)` on the mesh side (mesh's proposal).
- **Version-sensitivity:** HIGH — envelopes cross nodes on possibly-different
  `types` versions. `Envelope.v` is the anchor; additive-only payload evolution;
  relay sites operate on `Envelope<serde_json::Value>` and must not
  `deny_unknown_fields`. An unknown `EventType` string must route/filter as a
  pass-through, never a hard error (open identifiers, guardrail 4).

### `surface-schema` (every service → mesh dashboard) — grounded by `surface.rs`

- **Purpose:** each service publishes a boring schema of its observable surface;
  the dashboard renders every service's component from it (INTENT #46), with
  stable agent-drivable ids (#16) and honest estimate markers (#9).
- **Structs (from `types`):** `SurfaceSchema`, `SurfaceSection`, `SurfaceField`,
  `SurfaceAction`, `SectionKind`, `ValueType`, `Unit`, `Honesty`, `DataSource`,
  `ActionCall`. The published endpoint returns a `SurfaceSchema`; the dashboard
  is a pure function of it.
- **Error cases:** a service that fails to publish is simply absent from the
  dashboard (mesh's reconciliation concern); a malformed schema →
  `MeshError::MalformedSurfaceSchema { service }` (mesh side). No error type in
  `types` for this — it is mesh's to raise.
- **Version-sensitivity:** MEDIUM-HIGH — `SurfaceSchema.v` keys the
  service-component-versioning story (#37); the dashboard must render an older
  schema and ignore unknown `SectionKind`/`ValueType` variants gracefully
  (`#[serde(other)] => Custom/Unknown`). Additive fields only.

### `restart-protocol` (mesh ↔ every service) — grounded by `restart.rs`

- **Purpose:** the two-way, priority-laddered graceful-restart choreography
  built into every service (INTENT #77), plus observable interruptibility and
  port-handoff (#76).
- **Structs (from `types`):** `RestartRequest`, `RestartResponse`,
  `RestartPriority` (LOCKED 4-level ladder), `RestartReason`, `Interruptibility`,
  `PortHandoff`, `SocketHint`.
- **Error cases:** protocol-level failures (service unreachable, no response
  before deadline) are mesh's supervision concern →
  `MeshError::{RestartTimeout, ServiceUnreachable}`. A service refusing a
  sub-`SaveWindow` request via `Busy` is a normal outcome, not an error; at
  `SaveWindow`/`Kill` the `Busy` response is ignored by mesh.
- **Version-sensitivity:** MEDIUM — the 4-level ladder is LOCKED so the enum is
  stable; `RestartReason` may gain variants (reserve `#[serde(other)]`).
  `RestartRequest.v` anchors it. Cross-node because mesh on node A may supervise
  via relayed control to a service, but restart is primarily node-local.

### `queues-api` (any service ↔ mesh.queues) — PARTIAL, event/provenance half only

- **Purpose:** publish typed events, register declarative triggers, receive
  assembled handler payloads (INTENT #101/#103). `types` owns the **event and
  provenance** vocabulary this rides; the queue/trigger/handler *operations* are
  mesh.queues' proposal.
- **Structs (from `types`):** `Event<P>`, `EventType`, `Provenance`.
- **Boundary flag (for the queues design pass / harmonizer):** the **declarative
  trigger struct** (filter expression + payload-assembly template, registered as
  DATA — INTENT #103) is cross-cutting shared data authored by many services.
  My recommendation is it lives in `types` (a future `trigger.rs`) since it is
  pure declarative data used by ≥2 crates and is part of the `queues-api`
  schema — satisfying the inclusion test. But its shape is queues' domain to
  design; I flag it rather than pre-empt it. If it lands in `types`, it gets its
  own module, never folded into `event.rs`.
- **Version-sensitivity:** HIGH — events cross nodes and persist in queues;
  `EventType` is an open identifier (never a closed enum), payloads additive.

### `node-state-poll` / `service-lookup` (mesh ↔ inference / any service) — node-identity half only

- **Purpose:** the `NodeInfo`/`NodeCapabilities` shared shape (OQ-3) that the
  completion-router's affinity balancer and the service-registry read.
- **Structs (from `types`):** `NodeInfo`, `NodeCapabilities`, `NodeId`,
  `NodeRole`, `Accelerator`, and (unchanged) `SystemState` for the live snapshot.
- **Error cases:** none owned by `types`; registry/poll failures are mesh's
  (`MeshError::{NodeUnreachable, StaleSnapshot}`).
- **Version-sensitivity:** MEDIUM — `NodeCapabilities` is additive-heavy (roles,
  accelerator kinds, and later `DriveInfo` all grow); every field
  `#[serde(default)]` so a newer node's richer capabilities don't break an older
  peer's deserialize. `NodeRole`/`AcceleratorKind` reserve `#[serde(other)]`.
