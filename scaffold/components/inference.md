# inference

**Status:** WAVE-2 REFIT of the wave-1 `inference.md` (approach-sketched). The
wave-1 crate boundary, composition root, modality-agnostic name lock, and
external-contract set are **preserved** and re-affirmed; this pass adds the
**mesh-era integration** the new substrate demands and supersedes the two places
wave-1 left open. Grounded in the REAL code (`lib/inference/src/{lib,config}.rs`,
`bin/inference/src/main.rs`, and the eight child libs `lib/{store,engine,
scheduler,models,cache,telemetry,benchmark,api}`) and the batch-1..4 neighbor
designs: `mesh-client.md` (client half of every service protocol),
`service-registry.md` (NodeScoped/FleetAlias record model), `completion-router.md`
(byte-transparent forward + the per-node `inference` registration proposal I
align with below), `supervision.md` (the 4-level ladder + interruptibility
vocabulary), `gc.md` (the `GcHandle` Embedded/Remote seam — inference is THE
embedded consumer), `db.md` (`db-inference-init` via a `bin/db` subprocess,
boot-safe, no mesh dependency), `vfs.md` (gc-stays-called-as-tool + the future
storage-migration path), and `types.md`/`store.md`/`api.md`. INTENT
#4/#18/#22/#23/#29/#32/#36/#44/#45/#46/#53/#57/#58/#59/#66/#76/#77/#85/#92/#98.

## Charter

`inference` is the **per-machine, modality-agnostic generation runtime** and the
crate that IS the node app: one Cargo crate producing the `inference` daemon
(`bin/inference` + `lib/inference`) that binds a **loopback** `/v1/` surface
(default `127.0.0.1:8420`; dynamic if taken — INTENT #36) and composes its eight
internal libraries (store, engine, scheduler, models, cache, telemetry,
benchmark, api) via the private `InferenceService::start` wiring into a
single-machine service. It exposes exactly **ONE external contract** — the `/v1/`
REST+WS completion surface — reached from the fleet **THROUGH the mesh front
door** (`:3649`), never dialed directly (single-port locality, INTENT #58). It
owns the **lifecycle and composition** of its subsystems (the init sequence,
crash recovery, pause/resume, the lossy broadcast event bus, the prompt-hook
seam) and the node's **participation in the mesh**: self-registration as a
per-node `inference` instance, embedding `mesh-client`, participating in the
supervision restart ladder, publishing its surface schema, and bootstrapping its
control-plane database via `db` on a fresh node — all **degrading gracefully to
standalone single-box operation** when no mesh and no `db` are present.

**Boundary — what inference does NOT own.** It does not own **routing/affinity
across nodes** (that is `mesh.completion-router`; inference is a forwarding
*terminus*, not a router), **aggregation/observability across the fleet** (mesh's
dashboard-serving plane — the former gateway, merged 2026-07-18), the
**control-plane database engine** (`db` — inference is a *consumer* over a CLI
subprocess / WS, never a linker, INTENT #29), **agent management** (`ccd`), or
the **internals of its eight children** (each is a separately-designed module in
this batch; this file designs the *seams* between them and the crate's mesh
edges, not their guts). The name stays `inference`, not `llm`/`gen`: the crate is
deliberately **modality-agnostic** — future generation modalities (text-to-image,
text-to-video, image-to-image) extend THIS crate and its subsystems (a new
`InferenceBackend` impl behind the same `/v1/` surface — concern 7) rather than
spinning out sibling apps, so a text-flavored name would misdescribe its scope
(INTENT #18; `gen` separately rejected as a spoken homophone for "Jen").

## Primary design concerns

The wave-1 concerns (one-crate-not-eight; composition-root-is-private-wiring;
one-event-bus-fanned-out; shaped-for-maximalist-consumer) are **preserved and
re-affirmed** and are not re-argued here. The concerns below are the **wave-2
mesh refit** — each says explicitly where it supersedes wave-1.

### 1. Composition root vs. assembly seam — now with `mesh-client` threaded in

`InferenceService::start` (the real 10-step sequence: open store → recover →
telemetry → gc → models+sync → cache → engine → scheduler → benchmark loop →
scheduler loop; then `serve()` binds Axum) is this crate's **private wiring** and
is explicitly NOT the Scaffolding assembly seam (re-affirmed from wave-1). The
assembly seam is the mesh **`service-registry`** reached through the embedded
`mesh-client`. Wave-2 makes the seam concrete by inserting **two boot steps**
around the existing sequence, both **skippable for standalone**:

- **Step 0 (pre-store, boot-safe):** `db-inference-init` — a `bin/db` CLI
  subprocess bootstraps the node's **control-plane `ops` database** (concern 5).
  No mesh dependency, so it is safe before the daemon relays.
- **Step 11 (post-wiring, mesh-gated):** construct the `MeshClient` (one
  persistent WS to local `:3649`), **register** the per-node `inference` instance
  (concern 2), publish the **surface schema** (concern 6), start the
  **interruptibility feed** + **restart-participant** callback (concern 4), and
  hand `api` a **pubsub publish handle** for the event bus (concern 3). If the
  local daemon is unreachable, this step is skipped with a warning and the node
  runs **standalone** — every existing in-process path is unchanged.

The mode (`mesh` vs `standalone`) is decided **once** at start and threaded down;
no child call site branches on it (the `GcHandle` enum, concern 6, is the one
place the branch materializes as a field type). This preserves the wave-1
invariant that the composition root is the only place that knows the whole graph.

### 2. Mesh registration — a per-node `NodeScoped` `inference` instance (I ALIGN with completion-router's proposal)

completion-router.md concern 8 needs each inference node's endpoint to be
resolvable from `service-registry` so the router can source **fleet membership +
per-node endpoints** from the substrate (not a bespoke tailscale scan). service-
registry.md concern 5 as-written stores only the synthetic `inference`
`FleetAlias` and keeps per-node fleet membership *out*. **As the registrant, I
align with completion-router's reconciliation and confirm the inference side of
it:**

- At Step 11 each inference daemon **self-registers** via `mesh-client`'s
  `service-lookup` client half:
  `Register { slug: "inference", node, endpoint: {http, 127.0.0.1, <bound-port>,
  health_path: "/health"}, addressing: NodeScoped, ttl, meta: ServiceMeta{
  service_version, requires: [db? ] } }`. Keyed `(inference, <node>)` — exactly
  service-registry's `registry/instance/inference/<node_id>` keyspace, which
  already exists.
- The **`FleetAlias` becomes resolve-time policy** on the `inference` slug (owned
  by mesh-core), NOT a record inference writes. So: `resolve(AnyNode{inference})`
  → local `:3649` front door → completion-router; `resolve_all("inference")` →
  the per-node instances = the router's membership+endpoints;
  `resolve(Node{N, inference})` → node N's real endpoint (pinned forward +
  benchmark pinning, INTENT #15/#59).
- **Push-back / friction I own as the registrant:** service-registry.md concern 5
  currently *forbids* a fleet member registering `inference` and would raise
  `FleetSlugNotRegisterable`. That rule must be **relaxed to permit NodeScoped
  per-node instances of `inference`** while still reserving the `FleetAlias`
  record for mesh-core. Flagged for the per-pair round (also flagged by
  completion-router concern 8 — I reinforce it from the registrant side, so both
  parties agree). Fallback if the operator prefers membership stay out of the
  registry: inference registers under a distinct `inference-node` slug and the
  router derives the `inference` fleet from network-topology tags — still
  substrate-sourced, no tailscale loop in the router. I recommend the primary
  (register `inference` NodeScoped) as the boring, single-keyspace answer.

`mesh-client` runs the lease/heartbeat/tombstone lifecycle and **re-announces on
every reconnect** (mesh-client concern 4/5) — inference authors its registration
once and never hand-rolls reconnect. A crash simply lets the lease expire →
tombstone → self-heal; a restart re-registers under a fresh `generation`
(supervision/zombie-killing reads it).

### 3. `mesh-client` embedded — one socket; the event bus becomes pub/sub

Inference compiles in `lib/mesh-client` (a shared-lib, the blessed compiled-in
exception — INTENT #45) and holds **one** persistent WS to `127.0.0.1:3649`.
Everything mesh-facing multiplexes over it (register/resolve/heartbeat, pub/sub,
restart frames, surface-schema). The wave-1 "one event bus fanned out per node"
is preserved and **re-plumbed onto `pubsub-protocol`**:

- `InferenceService` continues to own the `broadcast::Sender<LifecycleEvent>`
  (capacity 256, **lossy** for slow consumers — unchanged real code). The parent
  threads **both** a `broadcast::Receiver` (for the local WS
  `/v1/completions/:id/stream` and any in-process subscriber) **and** the
  `mesh-client` **publish handle** into `api`.
- `api` bridges the bus to the `inference-events` contract (api-owned): each
  `LifecycleEvent` is published as a typed event over `pubsub-protocol` on topic
  `inference.<node_id>.*` (completion started/finished, model loaded/evicted,
  execution paused/resumed — the exact load-carrying kinds completion-router.md's
  `inference-events` consumer depends on). Envelopes are tagged with the node's
  **real `node_id`** (wave-1 requirement, preserved).
- **Lossy-on-lag is a contract property, not a bug:** a lagged mesh subscriber
  (completion-router) reconciles via `node-state-poll` (completion-router concern
  3). Inference does not grow an unbounded event queue (INTENT #38).

This supersedes wave-1's "a WS stream mesh's observability plane subscribes to":
the transport is now the standard relayed pub/sub envelope, published *through*
the local daemon, not a bespoke socket the observability plane dials.

### 4. Interruptibility states + the 4-level restart ladder — made CONCRETE

Inference participates in the LOCKED two-way `restart-protocol` (INTENT #76/#77;
supervision.md is the daemon side, mesh-client the client callback). The refit's
real deliverable is defining inference's **interruptibility states** and what each
ladder rung *does* in terms of the real subsystems. The vocabulary is
`types::restart::Interruptibility` (`Idle` / `Interruptible` /
`CriticalSection{until}`); inference maps it as a pure function of live state:

| Reported state | When | Why |
|---|---|---|
| **`Idle`** | `scheduler.running_count() == 0` AND no benchmark sweep in flight AND no model swap / KV-save in flight | Nothing to lose; free to restart at any level. |
| **`Interruptible`** | ordinary completions in flight (no benchmark, no swap) | Restartable **at a cost**: in-flight `Running` completions are requeued on the new process (existing `recover_running()` flips `Running`→`Pending`), and any streaming client is cut (completion-router's no-mid-stream-failover, concern 6). Safe to interrupt for a real reason, not for free. |
| **`CriticalSection{until}`** | a **benchmark priority-0 sweep** is running (INTENT #12 exclusivity — interrupting corrupts the kernel sample), OR a **model swap** or **KV-cache save/restore** is mid-flight (interrupting leaves engine↔store↔disk inconsistent) | Must NOT be interrupted for non-critical updates. `until` = best-effort ETA (the sweep's/ swap's estimated completion). |

The ladder, in inference's concrete terms:

- **L1 `WaitForIdle`** — supervision waits for the `Idle` feed; inference does
  nothing special (the interruptibility feed IS the signal). A routine update just
  waits out the current completions/sweep.
- **L2 `FinishAndRelinquish`** — the callback maps **directly onto the existing
  `pause_execution()`**: stop admitting new work (the `paused: AtomicBool` already
  in the real code), let `running_count` drain to 0, let the benchmark orchestrator
  stop at its next test boundary, then send `Relinquished`. **"Finish-and-relinquish
  mid-completion" = pause admission + drain in-flight to terminal, then yield** —
  no completion is killed mid-generation. This is the graceful path and it reuses
  code that already exists.
- **L3 `SaveWindow` (~10s)** — inference **cannot** finish long completions in 10s,
  so the save window saves the *recoverable* state and accepts the rest as
  requeued: (a) **checkpoint the KV/prefix cache** for the resident model's active
  slots via the engine's KV-save hook (`kv-cache` edge) so warm state survives the
  restart; (b) the **store is already durable** (synchronous SQLite writes) so
  completion metadata needs no flush — the "save" is a WAL checkpoint at most;
  (c) in-flight `Running` completions are **left in the store** and requeued by the
  next process's `recover_running()` on boot. Streaming clients mid-completion get
  a clean terminal error (no cross-node failover). Send `Saved` when the KV
  checkpoint completes OR when the deadline elapses, whichever first.
- **L4 `Kill`** — no participation; the process is killed. On restart,
  `recover_running()` requeues `Running`→`Pending` (existing crash recovery IS the
  L4 backstop). This is why the store durably outlives on-disk weights (store.md).

**Port-handoff (supervision concern 5):** during a version update the new
inference instance registers on a **new** loopback port (a second `(inference,
node)` registry entry), supervision verifies health via `/health`, flips the
registry endpoint, then drives the OLD instance through L2/L3 to drain and yield.
Because the endpoint is loopback and callers reach it only through the local
daemon (concern 10), the cut-over is invisible to clients.

### 5. `db-inference-init` via a `bin/db` subprocess — the boot-safe path (resolves the wave-1 tension)

Wave-1 flagged a tension between store's self-migrating embedded schema and the
new `db-inference-init` edge, and left it for the operator. **db.md (batch 4)
resolves it and I adopt that resolution:**

- On a **fresh mesh node**, inference bootstraps its **control-plane `ops`
  database** by invoking **`bin/db` as a CLI subprocess** (`db --env <node>
  migrate up`, sqlite driver, ledger-only baseline `OPS_BASELINE_SQLITE`). This
  runs at **Step 0**, is **boot-safe** (needs no mesh — a cold node may come up
  before the daemon relays), and is **not** a linked-lib call (INTENT #29 —
  superseding the wave-1 stub's "library dependency" wording).
- This is a **DIFFERENT database** from `store`'s inference **system-of-record**
  (`substrate.db`), which continues to **self-migrate at `Store::open`** with zero
  external dependency (store.md). The two never collide: `db-inference-init`
  bootstraps the control-plane `ops` schema (node registration + control-plane
  baseline); `store` owns completions/collections/models/results/benchmarks/
  kv_cache in `substrate.db`. **The wave-1 "store rewrite" fear is unfounded** —
  `db-inference-init` does not touch store's schema. This is the concrete answer
  to the flagged tension.
- **Graceful standalone degradation (INTENT #23):** if the `db` binary is absent
  or the subprocess fails, inference logs a warning and continues with just
  `store`'s `substrate.db` — the single-box path is dependency-free and unbroken.
  A second invocation on an already-initialized node is an **idempotent no-op**
  (db's ledger dedups).

### 6. Embedded `GcService` → `GcHandle`; the migration toward VFS-mediated storage

The real code today constructs its **own** `Arc<GcService>` over
`<data_dir>/gc.db` and registers `models`/`backends`/`kv_cache` dirs (lib.rs
Step 4). This is exactly the split-brain gc.md resolves, and **inference is THE
embedded consumer**. The refit (per gc.md concern 2):

- Replace the `Arc<GcService>` field wired into `models`/`cache`/`engine` with the
  **`GcHandle { Embedded(Arc<GcService>) | Remote(GcClient) }`** enum. **Mode
  selected once at `InferenceService::start`:** if the node's `gc` daemon is
  resolvable via mesh (`resolve(Local{gc})`), use **`Remote`** (every command
  lands on the ONE per-node `~/.substrate/gc.db` owned by `bin/gc` — split-brain
  gone by construction); else fall back to **`Embedded`** (byte-for-byte today's
  code, standalone/dev/cold-Pi). **No call site in the three children changes
  beyond the field type** — the command vocabulary (`make_room → write →
  register_and_lock`) is identical across both (gc.md's `GcApi` trait).
- **Migration toward VFS (gc.md concern 5 / vfs.md concern 5):** gc **stays
  called-as-tool**, NOT rolled into inference or VFS. When cold-storage tiering
  (the Pi + external drives) arrives, inference's models/cache/engine directories
  can become **vfs-managed directories** (VFS drives the same node-local gc for
  them over `vfs-gc`), and gc's `MigrateReclaimer` becomes "surrender this entry
  to VFS for placement." That convergence — unifying inference's gc-managed dirs
  with VFS's gc store into literally one per-node store — is a **downstream
  follow-up**, out of scope this pass; the `GcHandle::Remote` move is the step
  that gets inference onto the single per-node store today and makes the VFS
  future a no-op at inference's call sites (they already speak `GcApi`).

### 7. Modality-agnostic charter — the `InferenceBackend` seam is the modality axis (INTENT #18)

The name lock is a **structural commitment**: new modalities extend this crate.
The concrete seam that makes that true is `engine`'s **`InferenceBackend` trait**
(engine.md) — today a llama-server process manager for text generation; a future
text-to-image backend implements the *same* trait (auto-provisioned per model,
INTENT #4 — no pre-installed runtime), scheduled by the *same* scheduler, stored
by the *same* store, served by the *same* `/v1/` surface with **additive**
request/response shapes (a modality tag on `CompletionRequest`; image/video result
blobs in the existing `results` store module). The design requirement this pass
records: **do not special-case text** in the composition root, the store schema,
or the `/v1/` envelope in a way that would force a sibling crate for the next
modality. The event bus (`LifecycleEvent`), the kernel (per-model performance),
and the GC-managed weights directory are all modality-neutral already. This is a
"keep the door open" concern, not new build — but it is the reason the crate is
named as it is, so it is stated so a filler does not bake text-only assumptions in.

### 8. Which `/v1/` routes are mesh-exposed vs. internal-only

A specific refit ask. The `/v1/` surface has three audiences; the split is a
design fact the surface-schema (concern 6) and completion-router (byte-transparent
forward) both depend on:

| Class | Routes | Reached how |
|---|---|---|
| **Mesh-exposed** (forwarded byte-transparently through completion-router, `v1-completion-api`) | `/v1/completions/*` (submit/status/cancel/priority/result/stream), `/v1/collections`, `/v1/models` (+ `/:id/download`), `/v1/estimate`, `/v1/benchmark/run` (PINNED via `?node=`/`Node{N}` — INTENT #15) | client → local `:3649` → router → forward to the selected node's `:8420` |
| **Mesh-internal** (mesh's own planes; not client front-door traffic) | `/v1/system/state` + `/v1/models` **as the `node-state-poll` reconcile reads** (completion-router); `/health` (registry/supervision health-check via `Endpoint.health_path`); `/metrics`; `/v1/benchmark/kernel` (dashboard-serving surface); the `inference.<node>.*` **pub/sub** stream (`inference-events`) | mesh planes poll/subscribe through the local daemon |
| **Operator/control** (mesh-internal but operator-reachable via dashboard/CLI, and driven internally by the restart callback) | `/v1/execution/{pause,resume}`, read-only `/wiki/` | supervision drives pause/resume during a restart (concern 4); operator drives via the dashboard |

`/v1/models` intentionally appears in both mesh-exposed and mesh-internal — it is
byte-transparent, so the same route serves client forwards and reconcile polls
with no divergence. The **only** hard rule: the router must NOT parse or rewrite
the `/v1` body (byte-transparency = version-agnosticism; completion-router
`v1-completion-api` conformance).

### 9. Light provenance where completion data is touched (INTENT #85/#92, standing principle)

Provenance is first-order in VDB/db and light-to-nil elsewhere (gc: nil; vfs:
light). Inference is not the healthcare-grade home, but it **touches completion
data**, so it carries **light** provenance: the incoming request envelope's
`correlation_id`/`causation_id` (from `types::Provenance`, already the one
vocabulary — the `llm-calls`/`v1-completion-api` envelope carries it) is **stamped
onto the completion row** so a completion traces back to the requesting agent/
handler (ccd, org, a VDB handler) without a per-touch ledger. This is a
store-schema addition (an additive column on `completions`, threaded api →
scheduler → store) recorded here as a **parent-level requirement** so the seam
carries the field end-to-end; it is deliberately **light** (no per-token trace,
no separate ledger) — matching INTENT #92's "I don't typically care about
provenance outside the database." Flagged as a small store.md addition alongside
its already-flagged benchmark-covariate columns.

### 10. Single-port locality — `:8420` is loopback, reached through `:3649`

Inference binds `127.0.0.1:8420` (loopback; dynamic if taken, INTENT #36) and
registers that as its per-node endpoint. **Only the LOCAL mesh daemon reaches
`:8420`** (single-port locality, INTENT #58): a client never dials it, and a
*remote* completion arrives via the **target node's own daemon** relaying to its
local loopback `:8420` (completion-router's node-to-node data plane terminates at
the peer daemon, which hits loopback). This means inference **never binds a
tailnet-public interface** — a clean honoring of single-port locality. The one
reconciliation with completion-router.md concern 7 (which sketched a
daemon→remote-inference-endpoint tunnel as an *option*): I take the position that
the remote forward terminates at the **peer's local daemon → peer loopback
`:8420`**, not a cross-node dial of `:8420`, so inference stays loopback-only.
Flagged for the per-pair round with completion-router.

## Relationships / edges

External contract edges (all owned by the `api` child terminus or the parent's
boot/registration path; the router rides mesh-core's node-to-node data plane):

- **client / mesh.completion-router ↔ inference (api)** via `v1-completion-api` —
  the byte-transparent `/v1/` REST+WS surface, forwarded through `:3649`. Owned by
  `api` + `completion-router`; the parent wires the surface. (see
  scaffold/contracts/v1-completion-api.md)
- **mesh.completion-router → inference (api)** via `node-state-poll` — the
  reconcile/bootstrap health+inventory read (demoted to reconcile; live load rides
  `inference-events`). Owned by `api`. (see scaffold/contracts/node-state-poll.md)
- **mesh ← inference (api)** via `inference-events` — per-node lifecycle pub/sub on
  `inference.<node>.*`; the parent owns the `broadcast` bus, `api` publishes it
  over `pubsub-protocol`. (see scaffold/contracts/inference-events.md)
- **ccd (agents) → inference (api)** via `llm-calls` — usage/metering-shaped
  completion calls from agents that DO choose models (Claude Code does not route
  here — INTENT #40). Owned by `api`. (see scaffold/contracts/llm-calls.md)
- **inference → db** via `db-inference-init` — **PARENT-owned**: fresh-node
  control-plane `ops` bootstrap as a `bin/db` CLI subprocess (boot-safe, no mesh,
  distinct from store's `substrate.db`; concern 5). (see
  scaffold/contracts/db-inference-init.md — content proposed below)
- **inference (each daemon) ↔ mesh.service-registry** via `service-lookup` —
  **PARENT-owned participation**: registers the per-node `inference` NodeScoped
  instance via `mesh-client` (concern 2). Client half is `mesh-client`; I propose
  the inference-side Registration shape below. (see
  scaffold/contracts/service-lookup.md)
- **{models, cache, engine} → gc** via `gc-managed-dirs` — the children's in-process
  edge, now expressed through the parent-selected `GcHandle` (Embedded/Remote;
  concern 6). Owned by the children + `gc`. (see
  scaffold/contracts/gc-managed-dirs.md)

Cross-cutting mesh protocols (surface-schema-style, authored by
`mesh-client`/`supervision`/`pubsub-relay`; inference is a **consuming party** —
I define its concrete participation, do not re-author the wire):

- **every service ↔ mesh** via `restart-protocol` — inference's concrete
  interruptibility mapping + ladder behavior (concern 4). Client half `mesh-client`,
  daemon side `supervision`. (participation note below)
- **every service ↔ mesh** via `pubsub-protocol` — the envelope the event bus and
  all mesh frames ride (concern 3). Authored by `pubsub-relay`/`types`.
- **every service → mesh** via `surface-schema` — inference publishes its boring
  surface (completions table, models, system state, kernel curve + the interaction
  calls) for schema-driven dashboard rendering (INTENT #46, concern 6). Assembled
  by `api` from its route table, published + reconnect-replayed by `mesh-client`.

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45): the eight
children interconnect via `store-access`, `engine-exec`, `system-state`,
`model-ensure`, `kv-cache`, `benchmark-collections`, `api-dispatch`, and
`kernel-confidence` (each child's file); the parent compiles in `mesh-client`
(the mesh handle) and `substrate-types` (shared vocabulary).

## Nesting

Parent: (none — top-level L5 app-crate). Children (nested internal libs, compiled
into the `inference` daemon, never standalone top-level crates — INTENT #22):
**[store, engine, scheduler, models, cache, telemetry, benchmark, api]**. The
parent/child structure lives here + `overview.md`, not the directory layout.

## Thoroughness level

**implementation-ready** for the refit's mesh integration: the two-boot-step
`mesh-client` insertion (concern 1), the per-node NodeScoped `inference`
registration + the FleetAlias-as-policy alignment (concern 2), the event-bus →
pub/sub re-plumb (concern 3), the concrete interruptibility state table + the
four ladder rungs mapped onto real subsystems (`pause_execution`/drain/KV-save/
`recover_running`) (concern 4), the `db-inference-init` subprocess boot path
distinct from store's schema (concern 5), the `GcService`→`GcHandle` migration
(concern 6), the mesh-exposed-vs-internal route split (concern 8), and
loopback-`:8420`-through-`:3649` (concern 10) are all decided and specified
against real code + frozen neighbor seams. **approach-sketched** for: the
modality-agnostic "keep the door open" requirement (concern 7 — a real seam
`engine::InferenceBackend`, but no second modality is built this wave) and the
light-provenance column threading (concern 9 — a small store addition, direction
recorded).

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), per the wave2-plan tier
(`inference` = Opus), grounded in the REAL `lib/inference`/`bin/inference` +
`lib/config` source and every child lib, and the batch-1..4 neighbor designs
(`mesh-client`, `service-registry`, `completion-router`, `supervision`, `gc`,
`db`, `vfs`, `types`, `store`, `api`).

## Suggested fill-model

**implementation-ready + moderate complexity → mid model OK for the wiring**, with
three carve-outs for a careful hand: (1) the **restart-ladder mapping** (concern 4
— the L2 `pause`+drain and the L3 "what actually gets saved in 10s" boundary is
the one place a wrong default loses or corrupts in-flight work; test-first that a
drain completes before `Relinquished` and that a KV checkpoint + `recover_running`
loses no completion metadata across a kill); (2) the **standalone/mesh mode
selection** at Step 11 + the `GcHandle` mode selection at Step 4 (concern 1/6 —
every mesh call must degrade to the existing in-process path when no daemon is
present, or single-box dev breaks); (3) the **`db-inference-init` subprocess boot
ordering** (concern 5 — it must be idempotent, boot-safe, and never block a
standalone start when `db` is absent). The event-bus re-plumb, registration, and
route-split are near-transcription. Sequence **after** `mesh-client`,
`service-registry`, `gc`, and `db` are filled (it rides all four) and
**alongside** its own children (`api`/`store`/`engine`).

---

## Proposed contracts (wave 2)

Proposals only; the per-pair round reconciles both sides. I do NOT edit
`scaffold/contracts/*`. The external `/v1` edges (`v1-completion-api`,
`node-state-poll`, `inference-events`, `llm-calls`) are **owned by the `api`
child** and its counterparties (a separately-designed module in this batch) — I
reference them above and do not re-author them here. The **parent-owned** pairs
are `db-inference-init` and the inference side of `service-lookup`; the
cross-cutting `restart-protocol` gets a **participation note** (inference's
concrete interruptibility mapping), not a re-authored wire. Shared vocabulary
lives in `types` (`types::registry`, `types::restart`, `types::provenance`,
`DbError` in `types::error::db`).

### `db-inference-init` (inference → db) — fresh-node control-plane bootstrap (EXISTS; content proposed)

- **Purpose.** When an `inference` node stands up on a **fresh mesh node**, it
  initializes its **control-plane `ops` database** through `db` — the sqlite
  driver, ledger-only baseline (`OPS_BASELINE_SQLITE`) + `migration::apply`.
  **Distinct** from `store`'s self-migrating inference system-of-record
  (`substrate.db`, store.md); this edge is the control-plane bootstrap, not a
  store rewrite (concern 5, resolving the wave-1 tension).
- **Access shape — the boot-safe correction.** A **`bin/db` CLI subprocess**
  (`db --env <node> migrate up`), **not** a linked-lib call (INTENT #29,
  superseding the wave-1 stub's "library dependency" framing) and **not**
  necessarily a daemon WS call: on a cold node mesh may not yet relay and the db
  daemon may not be up, so the subprocess (needing no mesh) is the reliable
  bootstrap. Once warm, later control-plane access can move to `db serve` over WS.
- **Message/struct sketch** (the subprocess CLI is authoritative; structured
  output via `--format json`):
  ```rust
  // inference invokes, at Step 0 (pre-store, boot-safe):
  //   db --env <node_id> migrate up --format json   ->  MigrateReport
  struct MigrateReport { applied: Vec<String>, already_current: bool, ledger_head: String }
  ```
- **Error cases.** `DbError::MigrationFailed { id, detail }` on a failed first-boot
  apply → inference **degrades to standalone** (store's `substrate.db` alone,
  concern 5). Missing `db` binary → warn + continue standalone. A second
  invocation on an already-migrated node is an **idempotent no-op** (ledger dedups)
  — the conformance requirement.
- **Version-sensitivity.** LOW — exercises the narrowest, most stable slice of
  `db` (sqlite baseline + `migration::apply`); the CLI contract is stable.

### `service-lookup` (inference → mesh.service-registry) — the per-node registration (inference side)

- **Purpose.** Inference self-registers its per-node instance so the fleet is
  resolvable and completion-router can source membership+endpoints (concern 2).
  Client half is `mesh-client` (I consume its `Register`/`Renew`/`Deregister`
  frames); this proposal pins the **inference-specific Registration payload** and
  the FleetAlias-as-policy alignment.
- **Message/struct sketch** (`types::registry`; consumes mesh-client's
  `service-lookup` client half):
  ```rust
  // inference -> local daemon, at Step 11 (mesh-gated):
  Register {
      slug: "inference",
      node: <node_id>,
      endpoint: Endpoint { scheme: Http, host: "127.0.0.1", port: <bound_port>, health_path: Some("/health") },
      addressing: AddressingClass::NodeScoped,   // one live instance per node
      ttl_secs: <lease_ttl>,
      meta: ServiceMeta {
          service_version: <inference build semver>,
          requires: vec![ /* db pairwise req, if the node bootstraps control-plane */ ],
      },
  }
  // resolve policy inference RELIES ON (owned by mesh-core, not written by inference):
  //   AnyNode{inference}     -> local :3649 (FleetAlias policy) -> completion-router
  //   resolve_all("inference") -> the per-node NodeScoped instances (router membership)
  //   Node{N, inference}     -> node N's real loopback endpoint (pinned forward + benchmark, INTENT #15)
  ```
- **Error cases.** `RegistryError::FleetSlugNotRegisterable` — the **friction I
  push back on** (concern 2): service-registry.md concern 5 must relax this for
  NodeScoped per-node `inference` instances while still reserving the `FleetAlias`
  record for mesh-core. `LocalDaemonUnreachable` → inference runs **standalone**
  (Step 11 skipped). `LeaseExpired` → `mesh-client` re-registers on reconnect
  (automatic). A crash leaves no tombstone → lease-expiry self-heals; a restart
  re-registers under a fresh `generation` (supervision/zombie-killing reads it).
- **Version-sensitivity.** HIGH — `ServiceRecord`/`Endpoint`/`ServiceMeta`
  anti-entropy to peers on possibly-different versions; additive-only,
  `#[serde(default)]`, `#[serde(other)]`-tolerant enums, explicit `v` (types
  guardrail 4) so a mixed-version fleet keeps a coherent directory during a rolling
  update (INTENT #66).

### `restart-protocol` (mesh ↔ inference) — participation note (interruptibility mapping)

- **Purpose.** Not a re-authored wire (supervision owns the daemon side,
  `mesh-client` the client callback, `types::restart` the structs). This note pins
  **inference's concrete participation** so the harmonizer can reconcile it against
  supervision's `restart-protocol` proposal.
- **Interruptibility feed inference emits** (`types::restart::Interruptibility`,
  continuous on change, over `mesh-client`):
  ```rust
  // pure function of live subsystem state (concern 4):
  //   Idle                       when running_count == 0 && !benchmarking && !swapping
  //   Interruptible              when completions in flight (no benchmark, no swap)
  //   CriticalSection { until }  when a priority-0 benchmark sweep OR a model swap
  //                              OR a KV-cache save/restore is in flight
  ```
- **`on_restart(level)` behavior inference supplies** (the `RestartParticipant`
  callback): **L1** none (Idle feed is the signal); **L2** `pause_execution()` +
  drain `running_count`→0 + stop the benchmark at its next test boundary → send
  `Relinquished`; **L3** KV-cache checkpoint under the ~10s `save_deadline` +
  rely on durable store + leave `Running` completions for the next process's
  `recover_running()` → send `Saved` (or yield on deadline); **L4** none (killed;
  `recover_running()` requeues on boot).
- **Error cases / non-errors.** Missing an L3 deadline is the protocol working
  (supervision escalates to L4), not an inference error. A `Busy`
  (`CriticalSection`) reply is honored below `SaveWindow` and ignored at
  `SaveWindow`/`Kill` (supervision.md). Streaming clients cut at L3/L4 get a clean
  terminal error (no cross-node failover — completion-router concern 6).
- **Version-sensitivity.** The 4-level ladder is LOCKED; an unknown level (from a
  newer daemon) MUST default to the most conservative interpretation
  (save-and-yield, i.e. treat as `SaveWindow`) — never ignore a restart signal
  (mesh-client concern 7). `RestartReason` grows additively behind `#[serde(other)]`.
