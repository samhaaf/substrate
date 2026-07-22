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
enriched mesh-node identity/capabilities; and — new in wave 3 — the
**mesh-transport outcome switch** (the #154 outermost `success / error /
promise` switch every inter-service exchange resolves to), the **first-class
wire `Promise`** (#152 — a promise id that later fulfils by push or fetch, whose
enum shape is what makes "handle the promise case" a compiler obligation), and
the **required delivery-persistence policy** (#155 — the save-failed-deliveries
yes/no flag every published event must carry). Organized **one module per
domain**.
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

Wave-3 status: this pass folds the **comms-stack** vocabulary that wave 2 left
unfolded — the #154 envelope switch, the #152 wire promise, the #113 sender
version stamp (extended onto the transport layer), and the #155 delivery-
persistence field. Three additions are governed by the guardrails below and
land as ONE new module (`transport.rs`, the mesh-transport vocabulary) plus one
small cross-cutting module (`delivery.rs`) plus one field on the existing
`pubsub.rs` — no grab-bag. **Boundary held (design-around OQ-1, PARKED):**
`types` adds **no authority-node and no blessing-queue types** this wave. The
blessing-queue message shapes are chassis's to design, against an **abstract
blessing-target** — `types` deliberately holds nothing that names, locates, or
privileges an authority node, so that either the no-central-node
merge-reconciler path (#163) or a future cloud authority can be chosen later
without a change here.

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

   **LOCKED addendum (friction-round 1, INTENT #113) — sender version stamping.**
   Every message/event crossing the mesh carries the **sending service's name
   AND version**. The home is `Provenance` (which already carries `service`): a
   new additive `service_version: SemVer` field, `#[serde(default)]`, stamped by
   the daemon/`chassis` alongside the existing provenance fields, so every
   envelope, event, and queue delivery is version-attributed with no per-service
   effort. Receivers MAY enforce a **version floor** ("I'm only accepting
   messages from nodes with service version greater than X"); a floored message
   is rejected with a **catchable** error (a named, matchable variant — e.g.
   `MeshError::VersionBelowFloor { service, have, floor }` in `error/mesh.rs` —
   never a silent drop). On a schema change, a receiver offers **backwards
   compatibility for one version**, but attaches a **please-update warning back
   to the sender** telling it to update. This is the runtime data that makes
   supervision's version floors and compatibility restarts enforceable at the
   message level (see `supervision.md` concern 9).

   **Wave-3 extension (INTENT #113, onto the transport layer).** The sender
   stamp is not new data — it already rides `Provenance.service_version` — but
   wave 3 makes it load-bearing on the #154 transport switch: because every
   `MeshReply`/`Envelope` embeds `Provenance`, the outer switch is
   version-attributed for free, and the two runtime consequences #113 names get
   FIRST-CLASS wire types in `transport.rs`: a floored message is rejected with
   a matchable `WireError { domain: "mesh", code: "version_below_floor", … }`
   (mirroring `MeshError::VersionBelowFloor` in `error/mesh.rs` — a wire code,
   not a silent drop), and the one-version-back-compat "please update" signal is
   the `PleaseUpdate` struct a receiver attaches back to a sender it is
   tolerating. No new stamping effort per service; the daemon (chassis) stamps
   provenance once, as today.

   **Wave-3 LOCKED-nuance (INTENT #155) — a REQUIRED field is not an
   additive-optional field.** #155 demands the delivery-persistence choice be
   *required* ("a required field indicating whether failed deliveries should be
   saved… we can't just be silently dropping things"), which is in tension with
   guardrail 4's "every new field `#[serde(default)]`." The reconciliation, and
   the ONLY sanctioned exception shape: the field is **required at the type
   boundary** — its type (`delivery::DeliveryPersistence`) derives **no
   `Default`**, so a publisher cannot construct a published event without
   consciously choosing — while its serde attribute supplies a **safe-side
   decode fallback for older senders** (`#[serde(default = "…save_failed")]`),
   so a message from a node predating the field decodes to `SaveFailed` (persist,
   never drop), never to a silent-drop default. "Required to author, safe to
   decode." This is the one place the additive-optional rule bends, and it bends
   toward resiliency, exactly as #155 asks. It is NOT a license to add more
   no-default wire fields elsewhere.

**Sharper instance of concern 3 — the error taxonomy restructuring (now built
for real).** `SubstrateError` is today ONE flat enum growing by domain-tagged,
string-payload leaves per consuming crate (`Store`, `Db`, `Engine`, `Transport`,
`ServerUnhealthy`, …). That scales O(error-cases), not O(components): every new
V2 component (`mesh`, `vfs`, `secrets`, `cc`, `kg`, …) would sprinkle more
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

**Migration timing — RESOLVED: NOW (friction-round 3, 2026-07-20, INTENT
#138).** The operator's call, verbatim: "We're going to do everything before
we even test it. There's no production use of it yet... we're going to build
it correctly." The existing flat inference/store/db leaves migrate into
their sub-enums **in one sweep at skeleton time — no bridge period, no
old-leaves-stay-flat transition**. The sweep touches every
`SubstrateError::Store(...)`-style construction site across
`inference`/`db`/`store`; it is mechanical-but-wide and lands before any
production use exists. The formerly-flagged migrate-now-vs-later question is
closed.

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
    #[serde(default)] pub service_version: Option<SemVer>, // LOCKED (INTENT #113): sender version stamp
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
    // REQUIRED (INTENT #155): the publisher's save-failed-deliveries choice.
    // No `Default` on the type => must be chosen; serde decode-fallback is the
    // safe side (SaveFailed) for older senders. See guardrail 4 wave-3 nuance.
    #[serde(default = "delivery::DeliveryPersistence::save_failed")]
    pub delivery: delivery::DeliveryPersistence,
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

**`system.rs` wave-2 extension (acknowledged at harmonization):** telemetry
extends `types::system::SystemState` with the pressure axes and the
kernel-computed **`effective_max_concurrent: Option<u32>`** convenience scalar
(telemetry.md concern 6; the scheduler's kernel-primary admission target —
scheduler.md concern 1). All additive, `#[serde(default)]`, per guardrail 4;
the authored shape lives in `scaffold/contracts/system-state.md`.

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

## New wave-3 modules (all `implementation-ready`)

Same rules as the wave-2 modules: Rust-flavored sketches, exact field names are
the harmonizer's to final-tune, shapes buildable as written. Both new modules
are wire-crossing, so guardrail 4 applies in full (the `delivery.rs` required
field is the sanctioned "required to author, safe to decode" exception above).

### `transport.rs` — the mesh-transport switch, wire promise, and version-floor wire types (INTENT #154, #152, #113)

This is the home of the **#154 envelope**: the operator's "outermost layer is a
switch — success / error / promise — then a schema which specific
inter-application messages inherit from going inward." It is deliberately a
**distinct type from `pubsub::Envelope`** — the two are different layers over the
same WS transport: `pubsub::Envelope` is the fire-and-forget fan-out wrapper;
`transport::MeshReply` is the outcome of a request/response exchange (the
universal loopback/RPC reply, INTENT #152). Keeping them separate is required by
the ledger (the wave-2 "envelope" grep is the pub/sub one, not this switch) and
by concern 3 — two domains, two modules.

```rust
/// INTENT #154 — the outermost switch every inter-service EXCHANGE resolves to.
/// The `T` is the "schema which specific inter-application messages inherit from"
/// going inward: a request/response edge monomorphizes `MeshReply<MyResponse>`;
/// mesh's generic relay carries `MeshReply<serde_json::Value>`.
///
/// THIS ENUM IS THE ENFORCEMENT (INTENT #152 "every inter-service request must
/// have a handler for the promise case"): any caller that `match`es a reply is
/// compiler-forced, Rust-exhaustive-switch style, to write the `Promise` arm —
/// promise-handling is non-optional BY CONSTRUCTION, not by convention. This is
/// the "type vocabulary for that enforcement" the daemon (chassis) leans on.
pub enum MeshReply<T = serde_json::Value> {
    Success(T),
    Error(WireError),
    Promise(PromiseTicket),
}

/// Wire-safe error: preserves the "errors stringify at the boundary" invariant
/// (guardrail 1 — `types` never learns another crate's error type) while giving
/// the caller a MATCHABLE `code`. `domain` mirrors the error-taxonomy submodule
/// names ("mesh" | "inference" | "store" | …); `code` is the machine key
/// (e.g. "version_below_floor", INTENT #113).
pub struct WireError {
    pub domain: String,
    pub code: String,
    pub message: String,
    #[serde(default)] pub retriable: bool,
    #[serde(default)] pub details: serde_json::Value,
}
```

**The first-class wire promise (INTENT #152).** This is NOT the in-process
`Promise`/`PromiseSender` in `promise.rs` (that is a `tokio::oneshot` handoff,
wire-exempt). The wire promise is a durable *id* that crosses the mesh; its
value arrives later by push or fetch. Two distinct domains, two distinct
homes — the module descriptions say so explicitly to keep the vocabulary honest.

```rust
pub struct PromiseId(pub Uuid);

/// Returned inside `MeshReply::Promise` when a service can't answer instantly
/// (INTENT #152 "mesh returns a PROMISE; the caller moves on").
pub struct PromiseTicket {
    pub promise_id: PromiseId,
    pub issued_at: DateTime<Utc>,
    pub delivery: PromiseDelivery,                 // how the value comes back
    // INTENT #155 "Promise resolution notices ride this too": the resolution
    // notice inherits the same save-failed-deliveries durability choice.
    #[serde(default = "delivery::DeliveryPersistence::save_failed")]
    pub persistence: delivery::DeliveryPersistence,
}

pub enum PromiseDelivery {
    Push { topic: Topic },   // value pushed back over WS on this topic (INTENT #152)
    Fetch,                   // caller fetches by `promise_id` later
}

/// The later fulfillment — pushed on the `Push` topic, or returned from a fetch.
/// A promise never resolves to another promise (no infinite regress), so this is
/// a two-plus-pending switch, NOT a re-use of `MeshReply`.
pub enum PromiseResolution<T = serde_json::Value> {
    Fulfilled(T),
    Failed(WireError),
    Pending,                 // a fetch issued before the value is ready
}
```

**NAME RECONCILIATION — `PromiseFulfillment` wins (wave-3 amendment, batch-2
flag).** Three names for "a wire promise's later value" drifted across the batches:
this file's provisional `PromiseResolution` (here), `mesh-transport`'s authored
`MsgKind::PromiseFulfillment { promise, outcome }` frame
(`contracts/mesh-transport.md` §3–4), and chassis's internal `PromiseResolved {
promise_id, outcome }` (`components/chassis.md`). **The authored winner is
`mesh-transport`'s `PromiseFulfillment`** — it is the name on the reconciled,
implementation-ready transport contract that ~23 edges ride. At fill, the wire
type this enum describes is **renamed `PromiseFulfillment`**; `PromiseResolution`
and chassis's `PromiseResolved` are naming-history aliases, superseded. The
*shape* is unchanged (the `Fulfilled`/`Failed` arms map onto mesh-transport's
`ResponseOutcome::{Success, Error}` carried in the `PromiseFulfillment` frame — a
promise never re-resolves to a promise; the `Pending` arm is the fetch-before-ready
read, a query-side state, not a third frame arm). This is a rename, not a redesign
— see § "Wave-3 amendment queue" for provenance.

**Version-floor wire types (INTENT #113).** The sender name+version already rides
`Provenance` (unchanged); wave 3 adds the two runtime consequences #113 names as
matchable wire vocabulary here rather than as stringly leaves:

```rust
/// Attached back to a sender whose message a receiver is tolerating under the
/// one-version-back-compat rule (INTENT #113 "attach a warning back to the
/// sender telling it to update"). Rides back on the reply's provenance/warnings
/// channel; carries no behavior.
pub struct PleaseUpdate {
    pub service: String,
    pub have: SemVer,
    pub floor: SemVer,                       // the version the receiver wants
    pub reason: VersionFloorReason,          // Compatibility | Security | Policy
    #[serde(default)] pub deadline_hint: Option<DateTime<Utc>>,
}
pub enum VersionFloorReason { Compatibility, Security, Policy }
// A message BELOW the floor is not warned but REJECTED, as the matchable
// `WireError { domain: "mesh", code: "version_below_floor", .. }` above.
```

**Boundary (design-around OQ-1, PARKED).** `transport.rs` names no
authority/blessing types. A consistency-requiring exchange that needs blessing is
still, on the wire, just a `MeshReply` (its `Promise` arm covers "answer later");
WHO blesses and WHERE lives entirely in chassis against an abstract
blessing-target. `types` stays authority-agnostic so #163's no-central-node
merge-reconciler path and a future cloud authority are both reachable without a
change here.

### `delivery.rs` — delivery-persistence policy (INTENT #155)

A cross-cutting concern (referenced by `pubsub::Envelope` AND
`transport::PromiseTicket`), so per concern 3 it earns its OWN small module
rather than being bolted onto either.

```rust
/// INTENT #155 — the save-failed-deliveries choice. REQUIRED on every published
/// event and stamped on every promise ticket. Deliberately derives NO `Default`
/// (a publisher must choose; see guardrail 4 wave-3 nuance), but exposes a
/// safe-side constructor used as the serde decode-fallback so older senders
/// never decode to a silent drop.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeliveryPersistence {
    /// Failed deliveries are persisted for later retry/inspection — no silent
    /// drop. The safe side; the decode-fallback for pre-#155 senders.
    SaveFailed,
    /// Best-effort fan-out; a failed delivery is dropped — an EXPLICIT choice,
    /// never a default.
    LossyDrop,
}
impl DeliveryPersistence {
    /// serde decode-fallback + the "resiliency-first" default the safe side.
    pub fn save_failed() -> Self { DeliveryPersistence::SaveFailed }
}
```

`delivery.rs` names **only the yes/no choice**, not the mechanism. WHERE a saved
delivery goes — the distributed KV-backed cache for intermediate/promise
responses (INTENT #155), and specifically whether saved deliveries are enqueued
into `queues` — is **behavior owned by `mesh` / `pubsub-relay`, and is NOT
decided here** (that consolidation is PARKED, OQ-3 — "didn't seem very boring";
the mechanism must stay un-mergeable with a one-line change). `types` supplies
the flag and stops. This keeps the type flippable regardless of how the parked
question resolves.

## Wave-3 amendment queue (collected across batches)

Vocabulary additions that later wave-3 batches *proposed to `types`* but did not
own (each batch authored its own file and flagged the `types` landing for the
harmonizer). Collected here so the `types` owner lands them in one pass, each with
provenance and its governing guardrail. None is a new grab-bag; each is a named
module or a single additive field, per concern 3.

### AQ-1. `types::trigger` extension — the cron fold (batch 3)

`types::trigger` (home already acknowledged wave-2: "shape authored by queues,
module lives in `types`") gains the **cron-absorption** additions the batch-3 fold
requires. **Shape authoritative in the contract** (`contracts/queues-api.md`
§ Proposed contracts (wave 3) / `components/queues.md` concern 10); homed here.
Additions:

- `Trigger.source: TriggerSource` — replaces the wave-2 `Trigger.queue:
  QueueName` field. `enum TriggerSource { Queue(QueueName) | Schedule(ScheduleSource) }`.
- `ScheduleSource { schedule: Schedule, target: FireTarget, misfire: MisfirePolicy }`
  and its `Schedule` (`Cron|Every|Once` + `#[serde(other)] Unknown`), `FireTarget`
  (`Anywhere|Node(NodeId)` + `Unknown`), `MisfirePolicy` (`Skip|FireOnWake{grace,
  coalesce}` + `Unknown`) — vocabulary moved intact from the tombstoned `cron` lib.
- `HandlerRef::Emit { queue: QueueName, event_type: EventType }` — the cron
  emission variant (also a generic event-router action).
- `Trigger.enabled: bool` (`#[serde(default = "default_true")]`), and the
  `SourceKind { Queue, Schedule }` `ListTriggers` discriminant.

**Guardrail fit:** wire-crossing (persists in `replicated-kv` across a mixed-version
fleet) → guardrail 4 in full: every new enum reserves `#[serde(other)] Unknown` and
is **fail-safe** (an unknown `Schedule`/`FireTarget`/`MisfirePolicy` never fires).
The `queue → source` change is a **pre-production reshape** (INTENT #138 — no
production use yet), mechanical, not a compatibility break. **Inclusion test:** pure
declarative data used by ≥2 crates (`queues` + `execution-engine`, which consumes
`types::trigger` unchanged) and part of the `queues-api` schema — passes trivially.
**Provenance:** batch 3; `queues.md` concern 10, `queues-api.md`; INTENT
#56/#91/F6b (cron→Schedule), #101/#103 (LOCKED declarative-trigger vocab, untouched).

### AQ-2. Inference modality vocabulary + capability (batch 5)

Two landings, from the S2T/T2S-are-inference-modalities settlement (INTENT #157/#166
Q12, ledger A5.92):

- **NEW module `types::modality` (`modality.rs`)** — its own module per concern 3
  (a distinct domain from telemetry or events). Shape authoritative in `engine.md`
  P1:
  ```rust
  pub enum MediaType { Text, Audio, Image, Video, #[serde(other)] Unknown } // guardrail 4
  pub struct Modality { pub input: SmallVec<[MediaType; 2]>, pub output: MediaType } // S2T = {[Audio] -> Text}
  // canonical constructors: Modality::{T2T, S2T, T2S, TI2T}
  ```
  Supersedes any prior single-tag `Modality = Text|Image|Video|Audio` sketch (the
  ML input→output pairing wins). **Inclusion test:** used by `engine`, `inference`,
  `models`, `scheduler`, `completion-router`, and the `service-lookup`
  `NodeCapabilities` — well past ≥2. `SmallVec` is a permitted leaf dep in the same
  class as the existing `serde`/`uuid` (or lower to `Vec<MediaType>` if the
  zero-dep line is read strictly — harmonizer's call, a one-word change).
  **Provenance:** batch 5; `engine.md` P1; INTENT #18/#166 Q12.
- **Additive field on `NodeCapabilities` (`node.rs`):**
  `#[serde(default)] pub modalities_available: Vec<Modality>` — mirrors the existing
  `models_available` DOWNLOAD-inventory affinity input; a T2T-only node advertises
  `[Modality::T2T]`, a Pi with no S2T hardware simply omits it. The
  completion-router treats it as **one more affinity axis** — no new routing plane,
  **no authority dependency (OQ-1 stays PARKED)**. **Provenance:** batch 5;
  `inference.md` P1; INTENT #166 Q12.
- **`RuntimeSpec` — home is `engine-exec`, NOT `types` (recorded, not landed).**
  `RuntimeSpec { repo, asset_matcher, version_source }` is the per-runtime
  provisioning key on `engine.md`'s `InferenceBackend` trait (`fn runtime(&self) ->
  RuntimeSpec`, `engine.md` P2); it rides the modality vocabulary but is the
  **execution-surface trait's** type, authored on `contracts/engine-exec.md`, not
  shared `types` vocabulary. It does not pass the `types` inclusion test as a
  cross-cutting struct (single owning trait), so it stays on the engine-exec
  surface. Recorded here only because the amendment queue named it; **no `types`
  landing** unless a second consumer later forces it (the guardrail-2 rule).

### AQ-3. `PromiseFulfillment` name reconciliation (batch 2)

The wire-promise resolution type is authoritatively named **`PromiseFulfillment`**
per the authored `contracts/mesh-transport.md` (`MsgKind::PromiseFulfillment`),
which wins over this file's provisional `transport::PromiseResolution` and chassis's
internal `PromiseResolved`. Landed as a **prose note in the `transport.rs` module
above** (§ "NAME RECONCILIATION") — a rename at fill, shape unchanged. **Provenance:**
batch 2; `mesh-transport.md` §3–4, `chassis.md`; INTENT #152/#154/#156.

## Relationships / edges

`types` is **not a two-party contract edge** — it has no runtime
request/response of its own; it is the shared vocabulary every OTHER edge in the
contract graph is defined in terms of, and shared-lib consumption is explicitly
NOT a contract edge (wave2-plan §3 note; INTENT #45). It is compiled into all 44
modules. What it owns relative to the contract graph: the **struct halves of the
cross-cutting protocol contracts** — `mesh-transport` (NEW, wave 3),
`pubsub-protocol`, `surface-schema`, `restart-protocol`, and the
event/provenance vocabulary inside `queues-api` and
`node-state-poll`/`service-lookup` are all *written in* these `types` structs.
Those proposals are in the wave-2 and wave-3 contract sections below; the
per-pair contract round reconciles the mesh/service sides against them.

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
the rollout is RESOLVED migrate-now-in-one-sweep, INTENT #138), all five new
wave-2 modules (`provenance`, `event`, `pubsub`, `surface`, `restart`) plus the
`node.rs` split, and the two new wave-3 modules (`transport`, `delivery`) plus
the one added `Envelope.delivery` field. The struct shapes are buildable as
written; the per-pair contract round tunes exact field names and the mesh-side
behavior, not the vocabulary.

## Assigned design-depth

Opus (single pass, this file), grounded in the real `lib/types` source and the
mesh/completion-router component designs.

## Suggested fill-model

**implementation-ready + low-to-medium complexity → cheap/fast model OK.** The
new modules are pure data-struct declarations with serde derives — a fast model
can transcribe them directly from this file. Two carve-outs warranting a
slightly stronger hand or a careful reviewer: (1) the `error/` restructuring —
the operator HAS elected the full one-sweep migration of existing flat leaves
(INTENT #138) — a mechanical-but-wide change across `inference`/`db`/`store`
construction sites, and a wide change at the ONE seam wants care; (2) applying
guardrail 4's serde
attributes (`#[serde(default)]`, no `deny_unknown_fields`, `#[serde(other)]`
arms) uniformly across the wire-crossing modules — easy to get individually,
easy to forget one, so it wants a checklist pass. The wave-3 `transport`/
`delivery` modules are the same pure-data shape; the one subtlety a fast model
must not smooth away is `delivery::DeliveryPersistence` deriving **no `Default`**
while still carrying the `#[serde(default = "…save_failed")]` decode-fallback
(the "required to author, safe to decode" exception) — deriving `Default` here
would silently defeat INTENT #155's "no silent drops." Neither carve-out needs a
design-depth pass; all are conformance-checkable.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `pubsub-protocol` — (every service ↔ mesh) — grounded by `pubsub.rs` + `event.rs` + `provenance.rs`. → `scaffold/contracts/pubsub-protocol.md`
  - Contract resolution: pubsub-relay's relay-authoritative shape won; this
    file's first-cut `pubsub.rs` code sketch (typed `Envelope<P>`, per-message
    `Ack`, flat `Topic`, `subscriber_id`) is superseded — typed `Envelope<P>`
    survives only as the edge-decode convenience view. The merged canonical
    `Provenance` (with `correlation_id`/`causation_id`) homes in
    `provenance.rs`.
- `surface-schema` — (every service → mesh dashboard) — grounded by `surface.rs`. → `scaffold/contracts/surface-schema.md`
- `restart-protocol` — (mesh ↔ every service) — grounded by `restart.rs`; the contract adopts this crate's `RestartPriority` name and request-field `save_deadline` shape. → `scaffold/contracts/restart-protocol.md`
- `queues-api` — (any service ↔ mesh.queues) — PARTIAL, event/provenance half only; the flagged trigger-home question closed concordant (home = `types::trigger`, shape authored by queues). → `scaffold/contracts/queues-api.md`
- `node-state-poll` / `service-lookup` (mesh ↔ inference / any service) — node-identity half only (`node.rs`). → `scaffold/contracts/node-state-poll.md`, `scaffold/contracts/service-lookup.md`

Also a party to (as struct home, authored elsewhere): `system-state` —
`SystemState` lives in `types::system`; telemetry's `effective_max_concurrent:
Option<u32>` extension is adopted there in dual form (best-effort convenience
scalar on the snapshot + the authoritative `kernel-confidence` query),
acknowledged in this file's body. → `scaffold/contracts/system-state.md`

---

## Proposed contracts (wave 3)

`types` owns the **struct half** of the wave-3 comms-stack vocabulary; the
contract *files* are authored/reconciled by their owning units (mesh-transport,
pubsub-relay). These are proposals — the shape `types` puts forward — not
finalized reconciliations. Where a proposal touches a contract this unit does not
own, it is a proposal to that contract's owner, flagged here.

- **`mesh-transport` (NEW — proposed to the mesh-transport / chassis units).**
  The #154 outcome switch and #152 wire promise. Struct half grounded by
  `transport.rs`: `MeshReply<T>` (`Success | Error | Promise`), `WireError`,
  `PromiseTicket` / `PromiseId` / `PromiseDelivery` / `PromiseResolution`,
  `PleaseUpdate` / `VersionFloorReason`. Proposed edge shape: every
  request/response exchange over the mesh replies with a `MeshReply<Response>`;
  the caller side is compiler-forced to handle the `Promise` arm (the #152
  contract-level "must handle the promise case" is realized as enum
  exhaustiveness, not a lint). → target `scaffold/contracts/mesh-transport.md`
  (owned by the mesh-transport unit; this is the type-vocabulary proposal it
  binds to).
  - **Chassis seam (design-around OQ-1, PARKED).** The blessing-queue message
    types are **NOT** proposed here — they belong to chassis against an abstract
    blessing-target. A blessed exchange, on the wire, is just a `MeshReply` whose
    `Promise` arm defers the answer; `types` proposes nothing that names an
    authority node. Kept flippable for both the #163 no-central-node path and a
    future cloud authority.

- **`pubsub-protocol` (amendment — proposed to the pubsub-relay unit).** One
  REQUIRED field added to the published `Envelope`: `delivery:
  DeliveryPersistence` (grounded by `delivery.rs`), the #155 save-failed-
  deliveries yes/no choice. Proposed reconciliation notes: (a) the field is
  required-to-author / safe-to-decode (no `Default`, `SaveFailed` serde
  fallback) — never a silent-drop default; (b) `types` names only the yes/no
  flag — the persistence MECHANISM (distributed KV cache; whether saved
  deliveries enqueue into `queues`) stays with mesh/pubsub-relay and is MARKED
  needs-explanation (OQ-3, PARKED), so the flag and the mechanism can be
  un-merged independently. → target `scaffold/contracts/pubsub-protocol.md`
  (owned by the pubsub-relay unit).

- **Promise-resolution notice (rides `pubsub-protocol`).** A fulfilled/failed
  wire promise is delivered as an `Envelope<PromiseResolution<..>>` on the
  ticket's `Push` topic (or fetched by `promise_id`), inheriting the ticket's
  `DeliveryPersistence` (INTENT #155 "promise resolution notices ride this
  too"). No new contract file — a documented use of `pubsub-protocol` +
  `mesh-transport`.

