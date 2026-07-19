# api

**Status:** WAVE-2 REFIT of the wave-1 implementation-ready design. The wave-1
route surface (grounded in the real `lib/api/src/{lib,rest,ws,wiki}.rs`) is
**preserved and re-affirmed**; wave-2 re-grounds the module onto the new
substrate. Four things change: (1) the `/v1/` surface is now consumed THROUGH
the mesh front door (`:3649`) via `completion-router`'s byte-transparent
`forward()`/WS-relay — api's route set *is* the byte-transparency contract, and
the on-the-wire structs it emits carry `types`' guardrail-4 wire discipline;
(2) the documented **token-streaming gap** in `ws.rs` (the 100 ms poll loop) is
finally closed by subscribing to the engine/scheduler's **new per-completion
token broadcast seam**; (3) a **surface-schema endpoint** (`GET /v1/surface`,
INTENT #46) is added and published through `mesh-client`; (4) the gateway-era
event/aggregation leftovers are reconciled (pub/sub becomes the primary
`inference-events` path; the raw `/events` WS demotes to standalone-only; the
`benchmark_kernel` local recompute is deleted in favor of telemetry's real
kernel). **Nesting:** internal lib of `inference` (`lib/api`, modules `rest`,
`ws`, `wiki`), served on the inference-bound port; never a standalone crate
(INTENT #22).

## Charter

`api` is the Axum HTTP REST + WebSocket server: the inference node's **single
external contract surface**, and nothing else. It serves the `/v1/` completion
plane (submit / status / cancel / priority / result / stream), `/v1/collections`,
`/v1/models` (+ `/:id/download`), `/v1/estimate`, `/v1/benchmark/{kernel,run}`,
`/v1/execution/{pause,resume}`, `/v1/system/state` (+ `/metrics` alias), the NEW
`/v1/surface` self-description, read-only `/wiki/`, and `/health`. It is the
node-side **terminus** of both the client-facing `v1-completion-api` (which the
mesh forwards byte-transparently) and the mesh's read-only `node-state-poll`, it
publishes the node's **surface schema** and its typed **lifecycle/throughput
events** onto pub/sub through `mesh-client`, and it exposes the per-completion
token stream over the `/v1/completions/:id/stream` WebSocket. Its boundary: it is
**pure HTTP/WS translation + dispatch and holds NO business logic** — every route
dispatches into `scheduler`/`store`/`telemetry`/`benchmark` via `api-dispatch`,
and every cross-mesh interaction (registration, pub/sub publish, surface
publication, restart participation) rides the `mesh-client` handle. It does NOT
own: cross-node routing/affinity (`completion-router`), the pub/sub relay or
envelope (`pubsub-relay`/`types`), fleet aggregation/dashboard rendering
(`dashboard-serving`), the token *source* (the engine/scheduler broadcast seam —
api is a *subscriber*), completion state of record (`store`), or the port-bind /
mesh-registration *lifecycle* (that is the `inference` composition root; api only
supplies the `Router` and the `SurfaceSchema` to publish).

## Primary design concerns

The wave-1 route table is grounded and re-affirmed; the concerns below are the
**substrate refit** — how the surface is reached, the streaming gap closure, the
self-description endpoint, and the gateway-era cleanup. Where a concern
re-affirms wave-1 unchanged it says so.

### 1. One completions surface, reached THROUGH the mesh — byte-transparency is the contract

Under single-port locality (INTENT #58) a client never dials the inference node
directly. It resolves `inference` (AnyNode) → its local `:3649` → mesh-core's
Dispatcher → `completion-router` → `forward()` to the selected node's api
endpoint. **api's route set, status codes, headers, and body shapes ARE the
`v1-completion-api` contract**, because the router copies them through verbatim
(completion-router.md concern 7: byte-transparent `forward()`; the router
transforms nothing). Three conformance requirements fall out, and they are the
load-bearing wave-2 discipline for this module:

- **Forward-proxy-clean bodies.** No node-local assumption may leak into the
  wire format — no absolute `http://host:8420/...` URLs, no node hostname, no
  `:8420` port in any response body or `Location` header. Responses use
  **relative paths and node-agnostic bodies** so a body produced on node N reads
  identically whether the caller hit N directly or was routed to N. (The live
  handlers already satisfy this — they return relative ids and plain JSON — so
  this is a *keep-it-true* invariant, checked on every new field, not a rewrite.)
- **The `completion_id`-in-path convention is the router's ONE coupling.** The
  router keys its `completion_id -> owning-node` index and its WS stream relay on
  the `/v1/completions/:id/stream` path shape. api MUST keep that path stable;
  the router depends on nothing else about the `/v1` body (concern in
  `v1-completion-api`, version-sensitivity LOW *by construction*).
- **`?node=` is stripped before api ever sees it.** The external pin spelling
  (`?node=<id>` / `X-Substrate-Node`) is the router's affair; the router strips
  `?node=` before forwarding to preserve byte-transparency
  (completion-router.md concern 4). api therefore MUST NOT define, parse, or
  branch on a `node` query param — doing so would re-introduce the dead
  gateway-era query-string coupling (concern 5). This is the concrete "the
  query-string proxy bug is dead" cleanup on api's side: api stays *unaware* of
  pinning, which is exactly what keeps it byte-transparent.

**Wire-crossing struct discipline (types.md guardrail 4).** The structs api emits
that cross nodes — its pub/sub payloads (`LifecycleEvent`, the throughput sample,
per-completion `StreamEvent`) and its `SurfaceSchema` — travel node-to-node
through mesh between possibly-different `inference` commits (rolling updates,
INTENT #66). They must be **additive-only, every new field `#[serde(default)]`,
never `#[serde(deny_unknown_fields)]`, and the tagged enums reserve a catch-all
arm**. This is a concrete flag: today `StreamEvent` and `LifecycleEvent`
(`lib/types/src/stream.rs`, `#[serde(tag="type")]`) have **no `#[serde(other)]`
catch-all**, so an older subscriber hard-fails on a newer node's new variant.
Adding the catch-all is a `types` change, not an api change — flagged to the
`types` refit (friction below). The raw `/v1` REST bodies themselves are NOT
version-critical for the router (it never parses them), but ARE for typed
consumers (`ccd`/`org`/dashboard), so additive-only still governs them.

### 2. Close the token-streaming gap — subscribe to the engine/scheduler broadcast seam

The live `ws.rs` (documented, lines 1–24) is an explicit placeholder: it **polls
the store every 100 ms** for state changes and **never delivers `Token` events**
(tokens flow through the engine's per-completion `mpsc` channel only and are not
persisted). The wave-2 job is to finally wire the streaming seam the file's own
TODO names ("wire up the engine's `mpsc::channel::<StreamEvent>` … to a
`tokio::sync::broadcast` sender and replace the poll loop with a subscriber").

**The seam (api-side requirement, api is a pure subscriber).** The
engine/scheduler maintains a **per-completion `broadcast::Sender<StreamEvent>`
registry**, created at **submit** time (not admit — so a client connecting
immediately after `POST /v1/completions` cannot miss the channel), populated by
the engine's SSE loop as it produces `Started`/`Token`/terminal events, and
dropped shortly after the terminal event. api reaches it through `api-dispatch`:

```rust
// on Arc<Scheduler> (the token registry's home; scheduler owns admission, and the
// wave-1 mpsc is created in Scheduler::admit_pending — the registry sits alongside it):
fn subscribe_tokens(&self, id: CompletionId) -> Option<broadcast::Receiver<StreamEvent>>;
```

**The rewritten `/v1/completions/:id/stream` handler (poll loop deleted):**
1. `store.get_completion(id)` — miss → send an error frame + close (unchanged;
   404-as-close is preserved because the WS upgrade must proceed).
2. Already terminal → replay the single terminal `StreamEvent` from store state,
   close (unchanged — `send_terminal_event`).
3. Live → `scheduler.subscribe_tokens(id)` → forward every `StreamEvent` (incl.
   `Token`) as a JSON text frame as it arrives; interleave a `Heartbeat` on a
   timer; close on the terminal event. On `RecvError::Lagged(n)` emit a `lagged`
   notice frame (mirrors the existing `/events` lag handling) and continue — the
   token bus is lossy like every broadcast here (256-slot), and an honest lagged
   notice beats a silent gap. On `Closed` (completion dropped its channel without
   a terminal — crash) fall back to a store read for the final state.

This deletes the `POLL_INTERVAL` loop entirely and delivers real tokens. The
`StreamEvent` ordering guarantee (`stream.rs`: `Started` once → `Token*` →
exactly one terminal) is now honored by the source, not synthesized by polling.
**Deviation flag:** `engine.md` is marked "kept as-is" and does not define this
broadcast; `scheduler` is a batch-6 refit. api specifies only the *subscriber*
side and the `subscribe_tokens` shape; the token-broadcast *producer* is a
concrete ask on engine/scheduler (friction below), consistent with api's
no-business-logic charter.

### 3. Two token-delivery mechanisms, one boring rule — WS relay for bytes, pub/sub for observation

There are two distinct consumers of "what is this completion doing," and — per
the `pubsub-relay` boundary (its concern 4/8: pub/sub is NOT the byte-transparent
completion proxy; that is the router's `forward()`/WS-relay) — they are served by
two mechanisms, never conflated:

- **The client token stream** (a caller, or `ccd`/`org` via `llm-calls`, wanting
  the generated tokens): the `/v1/completions/:id/stream` **WebSocket**, relayed
  byte-transparently by the router (completion-router.md concern 7, a separate
  path from `forward()`). This is the primary, and only always-on, token path.
- **Coarse fleet observability** (the dashboard showing live throughput +
  lifecycle across nodes): api **publishes typed events onto pub/sub** through
  `mesh-client` — the `inference.<node>.*` topics. This carries **lifecycle +
  throughput**, deliberately **not every token** (matching the live
  `is_external_event` filter, which already excludes per-completion token/state
  events from the node `/events` feed and forwards model/backend/queue lifecycle
  + `CompletionMetricsRecorded`). A dashboard that wants live *tokens* for one
  completion subscribes to that completion's `/v1/.../stream` WS THROUGH the mesh
  — the same relayed mechanism any client uses — rather than paying a per-token
  fleet-wide pub/sub fan-out.

**Decision (v2):** api does **not** by default tee per-token events onto a
`inference.completion.<id>` pub/sub topic. `pubsub-relay` concern 4 anticipates
such a topic ("`inference.completion.token`"), so this is a flagged deviation to
reconcile in the per-pair round: in v2, tokens ride the WS relay (byte-transparent,
zero fan-out cost when unobserved); the per-completion typed-token topic is a
deferred opt-in, not wired now (avoids publishing every token through the local
daemon when no one is watching, and keeps ONE token-delivery mechanism boring).

### 4. The event bridge in the mesh era — pub/sub-primary, `/events` WS standalone-only

Wave-1 bridged the node's `broadcast::Sender<LifecycleEvent>` to a raw `/events`
WebSocket that the **gateway** subscribed to (one subscription per node). Gateway
merged into mesh (INTENT #45), and `completion-router` now consumes
`inference-events` as **pub/sub** (completion-router.md concern 3: `inference-events`
is the live-primary load feed, riding `pubsub-protocol` on topic
`inference.<node>.*`). So the wave-2 event path inverts:

- **Primary (mesh mode):** api subscribes to its own in-process
  `event_tx: broadcast::Sender<LifecycleEvent>` and **publishes** the external
  subset (the `is_external_event` set — model/backend/queue lifecycle +
  `CompletionMetricsRecorded` throughput) onto pub/sub via `mesh-client`, each as
  a `types::event::Event<P>` on an `Envelope`, topic `inference.<node>.<kind>`.
  The **`LifecycleEvent`-variant → `EventType`-string + payload mapping is api's
  to own** (the inference event catalog is api's, not `types`' — event kinds are
  open namespaced ids, types.md event.rs). Proposed mapping:

  | `LifecycleEvent` variant | `EventType` (`domain.noun.verb`) | router uses (concern 3) |
  |---|---|---|
  | `ModelLoaded` / `ModelUnloaded` / `ModelSwapping` | `inference.model.loaded` / `.evicted` / `.swapping` | resident + inventory |
  | `QueueDepthChanged` | `inference.queue.depth_changed` | running/pending counts |
  | `ExecutionPaused` / `ExecutionResumed` | `inference.execution.paused` / `.resumed` | admit/hold selection weight |
  | `CompletionMetricsRecorded` | `inference.throughput.sample` | live throughput (dashboard) |
  | `BackendReady` / `BackendStopped` / `BackendInstalling` … | `inference.backend.*` | observability |

  The router's five *load-affecting* kinds it depends on (concern 3 of
  completion-router.md: `completion.started/finished`, `model.loaded/evicted`,
  `execution.paused/resumed`) are api's guarantee to emit. **Note:**
  `completion.started`/`.finished` are NOT in the current `is_external_event`
  set (completion variants are excluded from the node feed); the router needs
  running-count deltas, which today are carried by `QueueDepthChanged`
  (`pending`,`running`). **Reconciliation for the per-pair round:** the router
  can fold `running` from `inference.queue.depth_changed` (already emitted), OR
  api additionally publishes lightweight `inference.completion.started/finished`
  count-only events. Recommend the former (reuse the existing queue event, no new
  per-completion fleet traffic) — flagged.

- **Standalone/dev (no mesh):** the raw `/events` WebSocket handler
  (`lifecycle_events`) is **kept, demoted to standalone-only** — a single-box dev
  affordance so a browser can watch a node with no mesh running (the
  graceful-degradation fallback inference.md requires). In mesh mode it is
  superseded by the pub/sub publish above. Recommend keeping it (cheap, and it is
  the honest degradation path); removing it entirely is a defensible
  simplification and is flagged as the operator's call.

### 5. Which routes are internal-only — the `:8420` convention, updated for the mesh era

SUBAGENT-INIT's standing convention was "never touch the `:8420` binding /
internal `/v1` routes." The mesh era **reframes** it: the node's bound port is a
**mesh-internal forward target**, not a public client address. Every external
caller reaches `/v1/` THROUGH `:3649`; the local daemon forwards to the node's
registered inference endpoint (which is a *preferred* `:8420` acquired at boot,
or a fallback port if taken — INTENT #36 — with the real port resolved via the
service registry, never a guessed/baked URL; types.md deprecates
`NodeInfo.api_url` for exactly this). The route classes:

| Class | Routes | Who reaches it | In the mesh era |
|---|---|---|---|
| **Fleet-facing** (forwarded byte-transparently via `:3649`) | all `/v1/completions*`, `/v1/collections*`, `/v1/models*`, `/v1/estimate`, `/v1/benchmark/run`, the `/v1/completions/:id/stream` WS | clients, `ccd`/`org` (`llm-calls`) | reached ONLY through the router's `forward()`/WS-relay |
| **Mesh-internal reads** (off the request path) | `GET /v1/system/state` (+`/metrics`), `GET /v1/models`, `GET /health`, `GET /v1/benchmark/kernel` | `completion-router` (`node-state-poll`), health probes, dashboard | primary consumer is the mesh; cheap, never on the submit path (api.md wave-1 concern 2, re-affirmed) |
| **Surface self-description** | `GET /v1/surface` (NEW, concern 6) | `dashboard-serving`'s surface pipeline (via registry) | mesh-internal pull; also pushed at registration |
| **Standalone/dev-only** | the raw `/events` WS | a no-mesh dev browser | superseded by pub/sub in mesh mode (concern 4) |

So the updated convention: **the `:8420` surface is private to the local mesh
daemon** (it forwards to it and polls it); external addressing is always
`resolve("inference")` → `:3649`. The route *set* is the byte-transparency
contract and stays forward-proxy-clean (concern 1); the *binding* is the
`inference` composition root's job (port acquisition + fallback + registration
via `mesh-client`), not api's — api only builds the `Router` and hands the
`SurfaceSchema` to publish. This is the precise "internal `/v1` routes" the old
convention protected, now expressed as an addressing rule instead of a
don't-touch note.

### 6. The surface-schema endpoint — inference's boring self-description (INTENT #46)

Every service publishes a `types::surface::SurfaceSchema` describing (a) how to
render its dashboard component and (b) what calls to make against it; the mesh
dashboard renders every service uniformly from it (INTENT #46, schema-driven, no
per-service custom UI). api **owns constructing inference's `SurfaceSchema`** (it
is the module that knows the `/v1` surface) and exposes it two ways, both boring:

- **`GET /v1/surface` → `SurfaceSchema`** — the pull endpoint `dashboard-serving`
  discovers via the registry and fetches (dashboard-serving.md's surface pipeline:
  registry → fetch each service's surface → feed the dashboard).
- **Push at registration** — the schema is handed to `mesh-client` at `Register`
  (`Register.surface`, mesh-client.md `service-lookup`) and **re-published on
  every reconnect** (mesh-client re-announce), so mesh always has it fresh.

Inference's schema (stable semantic ids on every section/field/action — INTENT
#16, agents drive the same markup; honest-estimate markers — INTENT #9):

- **`system`** (KeyValue, `data_source: PubSub inference.<node>.*` + `Rest
  /v1/system/state`): `resident_model`, `running`, `pending`, `memory_pressure`,
  `cpu_pressure`, `gpu_pressure` — the GPU field carries
  `honesty: Estimate { note: "Apple-Silicon GPU pressure is estimated" }` →
  the dashboard renders the tilde + tooltip (INTENT #9).
- **`queue`** (Table, `Rest /v1/completions?state=pending,running`): live
  completions.
- **`models`** (Table, `Rest /v1/models`): id, status, `is_resident`.
- **`kernel`** (TimeSeries/Custom, `Rest /v1/benchmark/kernel`): the per-model
  throughput kernel curve (concern 7).
- **`throughput`** (TimeSeries, `PubSub inference.<node>.throughput`): live
  tokens/sec from `inference.throughput.sample`.
- **Actions:** `submit` (`Post /v1/completions`), `pause`/`resume`
  (`Post /v1/execution/*`), `benchmark_run` (`Post /v1/benchmark/run`),
  `download_model` (`Post /v1/models/:id/download`), `cancel`
  (`Delete /v1/completions/:id`) — each with a stable `id` (e.g. `#submit`) so an
  agent drives the identical control a human sees.

`SurfaceSchema.v` keys the service-component-versioning story (INTENT #37): the
dashboard renders `(inference, v)` and tolerates unknown `SectionKind`/`ValueType`
variants (types.md surface.rs, `#[serde(other)]`). This is boring by intent — the
schema is data, the dashboard is a pure function of it.

### 7. `benchmark_kernel` serves telemetry's real kernel — delete the local recompute

The live `benchmark_kernel` handler (`rest.rs` lines 546–755) **recomputes its
own 1-D quadratic `tps = a + b·x + c·x²` over `output_tokens` at
`concurrency == 1`** via a hand-rolled `fit_quadratic_tps` least-squares solver.
INTENT #12 supersedes this: the kernel is **multidimensional and
confidence-aware** (memory/CPU/GPU pressure × parallel-completion-count × output
length), owned by `telemetry`, fed by the adaptive `benchmark` sweep. Wave-2:

- **Delete `fit_quadratic_tps` and the in-handler fit from api.** api holds no
  business logic; a curve fit in a route handler is exactly the debt the charter
  forbids.
- **`GET /v1/benchmark/kernel` dispatches to telemetry's kernel surface** (via
  `api-dispatch` / the `kernel-confidence` read telemetry exposes), serializing
  telemetry's real multi-axis kernel (curves + confidence + honest-estimate
  markers) rather than a local approximation. The route and its shape stay; only
  the *source* changes — a dispatch change, not new API surface (api.md wave-1
  concern 4, re-affirmed and now concrete: the handler becomes a passthrough of
  `telemetry`'s kernel serialization).

Sequencing note: telemetry/benchmark are batch-6 (the adaptive-kernel triangle),
so the exact kernel serialization shape follows their design; api's change is
"stop computing, start dispatching" and is filled after telemetry.

### 8. `estimate` and `download_model` seams — re-affirmed, not redesigned

Two live handlers carry documented seams; wave-2 leaves the *design* unchanged
and only records they ride `api-dispatch`:

- **`POST /v1/estimate`** currently news up a fresh cold `ThroughputEstimator`
  per call (`rest.rs` line 451, "results are always cold-start estimates"). The
  seam to pass a pre-warmed estimator through `ApiState`/`api-dispatch` is
  preserved; wiring the warm estimator is telemetry's concern (batch-6), api just
  dispatches. No api redesign.
- **`POST /v1/models/:id/download`** is a stub returning current model status
  (`rest.rs` line 415, the `ModelManager` is not in `ApiState`). The seam to
  dispatch into `models` (`model-ensure`) is preserved; api stays a passthrough.
  No api redesign.

These are noted so the fill doesn't gold-plate them and so the `api-dispatch`
contract enumerates them as real (if currently-stubbed) dispatch targets.

## Relationships / edges

Contract edges (all `/v1` traffic reaches api THROUGH the mesh front door; the
node-to-node data path is the router's / the mesh's):

- **client / mesh.completion-router ↔ inference (api)** via `v1-completion-api` —
  api is the **terminus** and owns the `/v1/` surface schema; the router forwards
  it byte-transparently (concern 1) (see scaffold/contracts/v1-completion-api.md).
- **mesh.completion-router → inference (api)** via `node-state-poll` — the
  reconcile/bootstrap health+inventory read (`GET /v1/system/state`,
  `GET /v1/models`), strictly off the request path (concern 5)
  (see scaffold/contracts/node-state-poll.md).
- **mesh ← inference (api)** via `inference-events` — api **publishes** the
  external lifecycle+throughput subset onto pub/sub via `mesh-client`
  (`inference.<node>.*`); the router (concern 3) and dashboard are consumers
  (concern 4) (see scaffold/contracts/inference-events.md).
- **inference (api) → mesh dashboard** via `surface-schema` — api constructs and
  publishes inference's `SurfaceSchema` (concern 6); cross-cutting,
  surface-schema-style, client half in `mesh-client`
  (see scaffold/contracts/surface-schema.md).
- **api → {scheduler, store, telemetry, benchmark}** via `api-dispatch` — the
  node-internal dispatch of the `/v1` surface, extended in wave-2 with
  `scheduler.subscribe_tokens` (concern 2) and the telemetry-kernel dispatch
  (concern 7) (see scaffold/contracts/api-dispatch.md).

Cross-cutting protocols api participates in **through `mesh-client`** (client half
owned by mesh-client; api is a producer/participant, not the author):

- **every service ↔ mesh** via `pubsub-protocol` — api publishes its
  `inference.<node>.*` events (concern 4) and (deferred) could publish
  per-completion token topics (concern 3). Carried by mesh-client.
- **mesh ↔ every service** via `restart-protocol` — the `inference` crate
  supplies the `on_restart`/interruptibility callback to `mesh-client`; api's
  only stake is that **`Interruptibility::CriticalSection` covers an in-flight
  completion/benchmark** so the node isn't torn down mid-generation (the
  interruptibility state is fed from scheduler's admission view, not api). Noted,
  authored by supervision/mesh-client.

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45): api
consumes `Arc<Scheduler>`, `Store`, `Arc<Telemetry>`, `BenchmarkOrchestrator`,
the node `broadcast::Sender<LifecycleEvent>`, and a `mesh-client` handle — all via
`ApiState`; imports `substrate-types` (completion/collection/model/stream/system +
the new `pubsub`/`event`/`surface`/`node` vocabulary).

## Nesting

Parent: `inference` | Children: none. Modules `rest`, `ws`, `wiki` in `lib/api`.
api is a library nested under the `inference` app (INTENT #22), never a
standalone crate. The port bind, mesh registration, and restart-callback wiring
live in the `inference` composition root (`InferenceService::start`); api
supplies the `Router` (via `router(ApiState)`) and the `SurfaceSchema`.

## Thoroughness level

**implementation-ready** for the refit. Decided and specified: the
byte-transparency conformance rules + wire-discipline flag (concern 1); the
token-streaming gap closure via `subscribe_tokens` with the exact rewritten
handler (concern 2); the two-mechanism token-delivery rule (concern 3); the
pub/sub-primary event bridge with the `LifecycleEvent → EventType` mapping
(concern 4); the internal-vs-fleet route classification + the updated `:8420`
convention (concern 5); the `/v1/surface` endpoint + inference's concrete schema
(concern 6); and the `benchmark_kernel` recompute deletion → telemetry dispatch
(concern 7). **Open by dependency (not gaps in this module):** (a) the
engine/scheduler **token-broadcast producer** — api specifies the subscriber side;
the producer is a batch-6 scheduler/engine ask (concern 2, friction); (b)
telemetry's exact **kernel serialization** shape (concern 7 — fill after
telemetry); (c) the `types` catch-all-arm addition to `StreamEvent`/`LifecycleEvent`
(concern 1 — a `types` change, flagged). The wave-1 route surface, status-code
map (`err_response`), and handler bodies carry forward unchanged under these.

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), grounded in the real
`lib/api/src/{lib,rest,ws,wiki}.rs` (the route table, `ApiState`, the 281-line
`ws.rs` poll-loop gap, the `benchmark_kernel` local fit) and
`lib/types/src/stream.rs` (`StreamEvent`/`LifecycleEvent`), plus the batch-1
`types`/`mesh-client`/`pubsub-relay`, batch-3 `completion-router`, batch-2/3
`supervision`/`service-registry`, and batch-5 `inference`/`engine` designs, and
INTENT #9/#12/#16/#22/#36/#37/#45/#46/#53/#58/#66.

## Suggested fill-model

**implementation-ready + low-to-moderate complexity → cheap/mid model OK** — it is
translation/dispatch against frozen contracts. Three carve-outs for a careful
hand: (1) the **rewritten `/v1/completions/:id/stream` handler** (concern 2 — the
subscribe/replay/terminal/lagged/closed branches, and the connect-before-admit
race the submit-time channel creation closes; the one correctness spot); (2) the
**`LifecycleEvent → EventType` publish mapping** (concern 4 — keeping the router's
five load-affecting kinds guaranteed and reconciling `running` from the queue
event vs new completion events); (3) the **`SurfaceSchema` construction** (concern
6 — stable ids on every element, honest-estimate marker on GPU). **Sequence after
`telemetry`** (concern 7 kernel serialization) **and after the scheduler/engine
token-broadcast seam lands** (concern 2) — both are batch-6; api's fill is a
subscriber/passthrough once they exist, so its risk is a scheduling constraint,
not a model-strength one.

---

## Proposed contracts (wave 2)

Proposals only; the per-pair round reconciles both sides. wave2-plan §3a/§3b make
api a party to `v1-completion-api`, `node-state-poll`, `inference-events`,
`api-dispatch`, and the cross-cutting `surface-schema`. `completion-router`
authored the router (consumer) side of the first three; api authors the
**terminus** side here, noting reconciliations. I do NOT edit
`scaffold/contracts/*`. Structs reference `types::{stream, system, event, pubsub,
surface, node}`.

### `v1-completion-api` (client / mesh.completion-router ↔ inference.api) — api is the terminus

**Purpose.** api owns the `/v1/` REST+WS completion surface's schema; the router
forwards it byte-transparently. This proposal pins **api's obligations as the
surface owner** (the router's forward-plane shape is completion-router's proposal;
the two meet at "bytes in = bytes out").

**Route surface (grounded in the live `rest.rs`/`ws.rs`, re-affirmed):**

```
POST   /v1/completions            -> 201 { id }
GET    /v1/completions            -> 200 [ {id, model_id, state, priority, ...} ]   (list, ?state=&limit=)
GET    /v1/completions/:id        -> 200 { id, state, ... } | 404
DELETE /v1/completions/:id        -> 204 | 404 | 409(terminal)
PATCH  /v1/completions/:id/priority  { priority:i32 } -> 204 | 409
GET    /v1/completions/:id/result -> 200 <result> | 409(not terminal) | 404
GET    /v1/completions/:id/stream -> WS StreamEvent* (Started, Token*, terminal; Heartbeat)   [token seam, concern 2]
POST   /v1/collections            -> 201 { id }
GET    /v1/collections/:id        -> 200 { ..., member_counts } | 404
DELETE /v1/collections/:id        -> 204 | 409 | 404
GET    /v1/models                 -> 200 [ {id, status, is_downloaded, is_loaded, ...} ]
POST   /v1/models/:id/download    -> 202 { id, status }        [stub seam, concern 8]
POST   /v1/estimate               { CompletionShape } -> 200 { tokens_per_second, estimated_ms, confidence }
```

**Error map (live `err_response`, re-affirmed):** `*NotFound` → 404;
`*Terminal` → 409; `InvalidRequest`/`EmptyCollection` → 422; else → 500. These
status codes are part of the byte-transparent contract (the router copies them
through) — stable.

**Conformance requirement.** Bodies MUST be forward-proxy-clean (concern 1: no
`:8420`/host/absolute-URL leakage); the `/v1/completions/:id/stream` path shape is
stable (router's index/relay key); api MUST NOT parse a `node` query param
(stripped upstream). `StreamEvent` frames follow the `stream.rs` ordering
guarantee, now sourced from the token broadcast, not the poll loop.

**Version-sensitivity.** LOW for the router (byte-transparent, never parses the
body). MEDIUM for typed consumers (`ccd`/`org`/dashboard): `/v1` bodies and
`StreamEvent` evolve **additive-only**; `StreamEvent`/`LifecycleEvent` need a
`#[serde(other)]` catch-all arm in `types::stream` for mixed-version tolerance
(flagged to `types`).

### `node-state-poll` (mesh.completion-router → inference.api) — api is the terminus

**Purpose.** api serves the cheap reconcile/bootstrap reads the router polls off
the request path (concern 5). Reconciles with completion-router's `SystemStatePoll`
/`ModelInventory` shapes.

```rust
// GET /v1/system/state (+/metrics) -> types::system::SystemState (live), serialized as-is
//   carries: resident_model: Option<ModelId>, running/pending counts, memory/cpu/gpu pressure
// GET /v1/models -> [ ModelRow-derived {id, is_downloaded, is_loaded(=resident), status, ...} ]
// GET /health -> { status:"ok", version }
```

**Error cases.** None api-specific beyond the standard map; these reads never
mutate and never touch the scheduler submit path (api.md wave-1 concern 2). A
poll during model swap returns a consistent snapshot (telemetry's `current_state`).

**Conformance.** `GET /v1/system/state` MUST expose `resident_model` +
running/pending + pressure (the router's affinity/least-loaded inputs);
`GET /v1/models` MUST distinguish downloaded vs resident (Tier-1/Tier-2 input).
GPU pressure is an honest estimate on Apple Silicon (INTENT #9) — the poll carries
the raw value; the honesty marker is on the `surface-schema`, not the poll.

**Version-sensitivity.** MEDIUM — derives from `types::system::SystemState` +
`types::node`; all fields `#[serde(default)]` additive; enums reserve
`#[serde(other)]` so a newer node's richer state never breaks an older router
mid-rollout (INTENT #66).

### `inference-events` (mesh ← inference.api) — api is the publisher (pub/sub in v2)

**Purpose.** api publishes the external lifecycle+throughput subset onto pub/sub
via `mesh-client`, riding `pubsub-protocol` on `inference.<node>.*` (concern 4).
Consumed by the router (live-primary load feed, completion-router.md concern 3)
and the dashboard.

**Published events** (as `types::event::Event<P>` on an `Envelope`; `EventType`
open ids per the concern-4 mapping):

```rust
// topic: inference.<node>.<kind> ; scope Fleet
inference.model.loaded {model_id}          inference.model.evicted {model_id}
inference.queue.depth_changed {pending, running}   // carries the router's running delta
inference.execution.paused {}              inference.execution.resumed {}
inference.throughput.sample {model_id, output_tokens, tokens_per_second}
inference.backend.* {..}                   // observability (ready/stopped/installing)
```

**Error cases.** Lossy by pub/sub contract; a `Lagged` notice on the local
mesh-client connection triggers no api action (the router reconciles via
`node-state-poll`; concern 3). Publish while the local daemon is down is buffered
best-effort by mesh-client (bounded, drop-oldest) — standalone mode falls back to
the raw `/events` WS (concern 4).

**Conformance.** api MUST emit the router's five load-affecting kinds
(model.loaded/evicted, execution.paused/resumed, and running-count via
queue.depth_changed) — the router depends only on these; all other kinds are
observability. **Open reconciliation (per-pair round):** whether `running` deltas
ride `queue.depth_changed` (recommended — reuse) or dedicated
`completion.started/finished` count events.

**Version-sensitivity.** LOW at the relay (payload-opaque, pubsub-relay concern
2); consumers match on the open `event_type` string and ignore unknown kinds. New
inference event kinds never break a consumer.

### `surface-schema` (inference.api → mesh dashboard) — api constructs inference's schema

**Purpose.** api builds inference's `types::surface::SurfaceSchema` (concern 6) and
exposes it via `GET /v1/surface` + pushes it through `mesh-client` at register /
reconnect. Cross-cutting, surface-schema-style; the render/serve side is
`dashboard-serving`'s, the client-half carrier is `mesh-client`.

**Struct (from `types::surface`).** `SurfaceSchema { v, service:"inference",
title, sections:[system, queue, models, kernel, throughput], actions:[submit,
pause, resume, benchmark_run, download_model, cancel] }` — stable `id` on every
section/field/action (INTENT #16); GPU pressure field carries
`honesty: Estimate{note}` (INTENT #9); `data_source` mixes `Rest{path}` and
`PubSub{topic: inference.<node>.*}`.

**Error cases.** None owned by api — a fetch failure means inference is simply
absent from the dashboard (dashboard-serving's reconciliation); a malformed schema
is a compile-time concern (typed in `types`). Best-effort publish (mesh-client
retries on reconnect).

**Version-sensitivity.** MEDIUM — `SurfaceSchema.v` keys the component-versioning
story (INTENT #37); the dashboard renders an older schema and ignores unknown
`SectionKind`/`ValueType` variants (`#[serde(other)]`, types.md surface.rs).
Additive fields only.

### `api-dispatch` (api → {scheduler, store, telemetry, benchmark}) — node-internal, extended in wave-2

**Purpose.** The in-process dispatch every `/v1` route makes (api holds no
business logic). Internal-lib seam, NOT a cross-process contract edge (INTENT
#29/#45); documented here because wave-2 extends it.

**Dispatch surface (grounded in the live handlers) + wave-2 additions:**

```rust
// scheduler (Arc<Scheduler>)
submit(CompletionRequest) -> CompletionId ; cancel(id) ; is_queue_empty()
subscribe_tokens(id) -> Option<broadcast::Receiver<StreamEvent>>   // NEW (concern 2) — closes the ws gap
// store (Store)
get/list_completions* ; get/insert/cancel_collection ; collection_member_counts
get/list_models ; get_result ; update_priority ; benchmark_runs_for_model
// telemetry (Arc<Telemetry>)
current_state() -> SystemState
kernel_surface() -> <telemetry's multi-axis kernel>               // NEW (concern 7) — replaces api's local fit
// benchmark (BenchmarkOrchestrator)
schedule_if_idle(model_id, queue_empty) -> Option<CollectionId>
```

**Error cases.** All dispatch returns `Result<_, SubstrateError>`; api maps via
`err_response`. `subscribe_tokens` returning `None` (no live channel — completion
already terminal or never admitted) is a normal branch, not an error (the WS
handler falls back to store replay).

**Conformance.** api adds NO logic on top of dispatch — a route is
extract → dispatch → serialize. The wave-2 removals are conformance items: **no
curve fit** (`fit_quadratic_tps` deleted, concern 7) and **no poll loop** (concern
2) may remain in api.

**Version-sensitivity.** N/A (compiled-in; the whole `inference` crate revs as
one binary — INTENT #45). The seam is a Rust trait/method surface, not a wire
format.
