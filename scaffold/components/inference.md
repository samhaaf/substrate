# inference

**Status:** WAVE-2 REFIT (mesh-era integration) **+ WAVE-3 modality/chassis
delta**. The wave-1 crate boundary, composition root, modality-agnostic name
lock, and external-contract set are **preserved** and re-affirmed; wave-2 added
the mesh integration. **Wave-3 adds two things** and touches nothing else: (1) the
modality axis becomes REAL — S2T/T2S as inference modalities with a warm-model
policy and placement-through-the-mesh (INTENT #157/#166 Q12, concern 7, engine.md
concern 5); and (2) **chassis adoption** — inference now registers via `chassis`
(the wave-3 daemon-wrapper that absorbed `mesh-client`), so the bring-up
references below name it. Everything else — the kernel, restart states,
loopback-only, `db-inference-init`, `GcHandle` — stays. Grounded in the REAL code
(`lib/inference/src/{lib,config}.rs`, `bin/inference/src/main.rs`, and the eight
child libs `lib/{store,engine,scheduler,models,cache,telemetry,benchmark,api}`)
and the neighbor designs: `chassis.md` (the daemon-wrapper / client half of every
service protocol — bring-up, registration, restart policy, promises, outbox),
`service-registry.md` (NodeScoped/FleetAlias record model), `completion-router.md`
(byte-transparent forward + the per-node `inference` registration proposal I
align with below + the modality-affinity routing this pass adds),
`scheduler.md`/`models.md` (the warm-model residency co-design),
`supervision.md` (the 4-level ladder + interruptibility
vocabulary), `gc.md` (the `GcHandle` Embedded/Remote seam — inference is THE
embedded consumer), `db.md` (`db-inference-init` via a `bin/db` subprocess,
boot-safe, no mesh dependency), `vfs.md` (gc-stays-called-as-tool + the future
storage-migration path), and `types.md`/`store.md`/`api.md`. INTENT
#4/#18/#22/#23/#29/#32/#36/#44/#45/#46/#53/#57/#58/#59/#66/#76/#77/#85/#92/#98/#156/#157/#166 Q12.

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
per-node `inference` instance, embedding `chassis`, participating in the
supervision restart ladder, publishing its surface schema, and bootstrapping its
control-plane database via `db` on a fresh node — all **degrading gracefully to
standalone single-box operation** when no mesh and no `db` are present. Wave-3:
that mesh participation is now **built on `chassis`** (the daemon-wrapper that
absorbed `mesh-client`) — inference supplies its surface schema, its restart
policy, and an exhaustive set of typed contract handlers, and chassis owns the
socket, registration, reconnect, restart-ladder mechanics, promises, and outbox.

**Boundary — what inference does NOT own.** It does not own **routing/affinity
across nodes** (that is `mesh.completion-router`; inference is a forwarding
*terminus*, not a router), **aggregation/observability across the fleet** (mesh's
dashboard-serving plane — the former gateway, merged 2026-07-18), the
**control-plane database engine** (`db` — inference is a *consumer* over a CLI
subprocess / WS, never a linker, INTENT #29), **agent management** (`cc`), or
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

### 1. Composition root vs. assembly seam — now with `chassis` threaded in

`InferenceService::start` (the real 10-step sequence: open store → recover →
telemetry → gc → models+sync → cache → engine → scheduler → benchmark loop →
scheduler loop; then `serve()` binds Axum) is this crate's **private wiring** and
is explicitly NOT the Scaffolding assembly seam (re-affirmed from wave-1). The
assembly seam is the mesh **`service-registry`** reached through the embedded
**`chassis`** (wave-3: the daemon-wrapper, absorbing what wave-2 called
`mesh-client`). Wave-2 makes the seam concrete by inserting **two boot steps**
around the existing sequence, both **skippable for standalone**:

- **Step 0 (pre-store, boot-safe):** `db-inference-init` — a `bin/db` CLI
  subprocess bootstraps the node's **control-plane `ops` database** (concern 5).
  No mesh dependency, so it is safe before the daemon relays.
- **Step 11 (post-wiring, mesh-gated):** build the **`chassis`** —
  `Chassis::builder("inference", version!()).requires([dep("db", …)])
  .surface(inference_surface_schema()).restart_policy(InferenceRestartPolicy)
  .serve::<InferenceContracts>(handlers)` — which opens the one persistent WS to
  local `:3649`, **registers** the per-node `inference` instance (concern 2),
  publishes the **surface schema** (concern 6), starts the **interruptibility
  feed** + the **restart-policy callback** (concern 4), and hands `api` a
  **pub/sub publish handle** for the event bus (concern 3). `handlers` is the
  exhaustive typed set chassis's generated `Contract` trait forces inference to
  cover at compile time; `InferenceRestartPolicy` is the wind-down *policy*
  inference supplies (chassis owns the ladder *mechanics*). If the local daemon
  is unreachable, chassis enters its reconnect loop and the node runs
  **standalone** — every existing in-process path is unchanged; the warm-model
  and provisioning paths (concern 7) are unaffected by mesh presence.

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

- At Step 11 each inference daemon **self-registers** via `chassis`'s
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

`chassis` runs the lease/heartbeat/tombstone lifecycle and **re-announces on
every reconnect** (chassis concern 8) — inference authors its registration
once and never hand-rolls reconnect. A crash simply lets the lease expire →
tombstone → self-heal; a restart re-registers under a fresh `generation`
(supervision/zombie-killing reads it).

### 3. `chassis` embedded — one socket; the event bus becomes pub/sub

Inference compiles in `lib/chassis` (a shared-lib, the blessed compiled-in
exception — INTENT #45) and holds **one** persistent WS to `127.0.0.1:3649`.
Everything mesh-facing multiplexes over it (register/resolve/heartbeat, pub/sub,
restart frames, surface-schema). The wave-1 "one event bus fanned out per node"
is preserved and **re-plumbed onto `pubsub-protocol`**:

- `InferenceService` continues to own the `broadcast::Sender<LifecycleEvent>`
  (capacity 256, **lossy** for slow consumers — unchanged real code). The parent
  threads **both** a `broadcast::Receiver` (for the local WS
  `/v1/completions/:id/stream` and any in-process subscriber) **and** the
  `chassis` **publish handle** into `api`.
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
supervision.md is the daemon side, `chassis`'s `RestartPolicy` seam is the client
callback — inference supplies `InferenceRestartPolicy`, chassis owns the ladder
mechanics, chassis concern 5). The refit's
real deliverable is defining inference's **interruptibility states** and what each
ladder rung *does* in terms of the real subsystems. The vocabulary is
`types::restart::Interruptibility` (`Idle` / `Interruptible` /
`CriticalSection{until}`); inference maps it as a pure function of live state:

> **Superseded one row at harmonization (scheduler.md concern 3 wins):** a
> running **benchmark priority-0 sweep reports `Idle`, NOT `CriticalSection`**.
> Treating benchmark as critical would let the lowest-value, fully-preemptible,
> re-runnable work block a routine `WaitForIdle` update during the exact idle
> window an update wants. An interrupted sweep loses only its in-flight samples
> (cheap, re-planned next idle window); nothing is corrupted. Inference
> explicitly delegated the transition function to `scheduler`; the table below
> is updated to the scheduler's version.
>
> **Friction-round 2 disposition (INTENT #119): applied provisionally,
> pending the restart philosophy.** The operator did NOT rule on
> idle-vs-critical here — per-app restart-signal semantics (this
> benchmark-as-Idle row included) are deferred to a restart-interrupt-signal
> PHILOSOPHY, to be developed in a dedicated harness plugin via the critic
> pattern (Opus proposes, Fable critiques, Opus synthesizes). The mapping
> below stands until that philosophy lands, and yields to it. See
> supervision.md concern 3.

| Reported state | When | Why |
|---|---|---|
| **`Idle`** | `scheduler.running_count() == 0` AND no model swap / KV-save in flight (a priority-0 benchmark sweep MAY be running — it does not raise interruptibility) | Nothing to lose worth protecting; free to restart at any level. A sweep interrupted by a restart is simply re-planned. |
| **`Interruptible`** | ordinary (priority ≥ 1) completions in flight (no swap) | Restartable **at a cost**: in-flight `Running` completions are requeued on the new process (existing `recover_running()` flips `Running`→`Pending`), and any streaming client is cut (completion-router's no-mid-stream-failover, concern 6). Safe to interrupt for a real reason, not for free. |
| **`CriticalSection{until}`** | a **model swap** or **KV-cache save/restore** is mid-flight (interrupting leaves engine↔store↔disk inconsistent) | Must NOT be interrupted for non-critical updates. `until` = best-effort ETA (the swap's/save's estimated completion). Only genuine engine-internal atomic operations qualify. |

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

### 7. Modality axis + warm-model policy + placement through the mesh (INTENT #18/#157/#166 Q12)

The name lock is a **structural commitment made concrete this wave.** #166 Q12
settles that **S2T (speech-to-text) and T2S (text-to-speech) are inference
MODALITIES**, not standalone always-on services; the ML-style naming is LOCKED
(S2T/T2S, never STT/TTS), and the modality axis is now real and populated: **T2T
today, S2T/T2S next, TI2T future.** The *seam* mechanics are engine's (the
`Modality` input→output pairing, the per-runtime `BackendProvisioner` for
whisper.cpp / the TTS engine, the media-typed request/result — engine.md concern
5). The four *node-and-fleet* consequences are this file's:

**(a) A modality is a node capability the fleet routes by.** A backend declares
its `Modality`; inference advertises the modalities a node can serve in its
`NodeCapabilities` (`modalities_available: Vec<Modality>`, additive — Proposed
contracts below), alongside `models_available` (concern 2). The completion-router
routes a modality-tagged request to a node that advertises that modality **exactly
as it already routes by model affinity** (its Tier-2 affinity) — modality
capability is one more affinity axis, **not** a new routing plane. This is the
**boring, no-authority placement path**: the authority-node discussion #157 raised
is PARKED (ledger OQ-1); #163's no-central-node alternative is the standing
direction. Inference therefore threads **no** authority dependency — placement is
the router's existing scheduling call over the substrate the mesh already carries.

**(b) Warm-model policy is the "always-on" answer (#166 Q12's warm-model option).**
#157 left open whether S2T/T2S are "always-on mesh services"; Q12 answers with a
**warm-model policy** instead: a modality is kept **resident by config**.
Inference's `substrate.toml` gains a **`warm` set** — models (a whisper S2T model,
a TTS model, or any T2T model) the composition root **pre-loads at boot and pins
resident**: the scheduler never swaps them out (scheduler.md's swap logic yields
to a warm pin) and `models` locks them **never-evict** (models.md concern 4's
lock-while-resident, here made permanent by config rather than by live use). Warm
= ready with **zero load latency** — the property interactive speech needs (you
cannot wait for a multi-GB whisper weight to `mmap` on each utterance). This is
how a modality becomes "always on" **without** a standalone always-on service: an
inference modality with a resident-by-config pin, served through the same `/v1/`
surface, scheduled by the same scheduler. Warm residency is a first-class
scheduler input **flagged for scheduler/models co-design** (a warm model bypasses
`DebouncedSwapEvaluator` eviction and is loaded at boot); until wired, a `warm`
entry is simply the model loaded first and never chosen for eviction.

**(c) Placement is hardware-constrained — the Pi-can't-run-S2T worked example
(#157).** Because a warm model permanently occupies a slot + VRAM, **which** node
keeps a modality warm is a hardware decision, and #157's constraint is exact: a
walk-along **Pi cannot host a warm whisper model** (no accelerator, too little
RAM), so it **never advertises S2T** in its `NodeCapabilities` and never gets a
warm S2T pin. When the Pi's operator speaks, the Pi's AUI issues an S2T request
that — under single-port locality (concern 10) — leaves the Pi through its
**local mesh daemon**, and the completion-router places it on a node that *does*
advertise a warm S2T modality (a desktop/laptop). "Hardware constraints mean
placement decisions go through [the mesh]" (#157) resolves to exactly this: the
router, sourcing modality capability from the substrate, routes to a capable node
— no authority node, no bespoke placement service.

**(d) The AUI use case grounds it — the operator's own harness needs S2T/T2S from
the mesh.** The concrete v1 consumer is the operator's AUI (`aui-client.md`, batch
6): a mostly-interactive client that turns the operator's speech into text (S2T)
and speaks results back (T2S), obtaining **both from the mesh as inference
modalities.** An `aui-client` issues a modality-tagged completion (S2T with an
audio input blob, or T2S with text) over the standard `v1-completion-api` front
door (`:3649`), byte-transparently forwarded by the completion-router to a node
holding the warm modality. **No new contract is required** — S2T/T2S ride the
existing `/v1/completions` surface with a **modality tag on the request** (concern
8's mesh-exposed class); `aui-client.md` (batch 6) owns the client seam, and its
"S2T placement / warm-model" open item resolves to this design. This is the
dogfooding case: the OS serves its own operator's speech through its own inference
plane.

**Seam-only, not built this wave.** Everything above pins seams (the
`modalities_available` capability, the `warm` config set, the router's
modality-affinity flag, the aui-client edge). No S2T/T2S backend is implemented
this pass; engine.md concern 5 keeps the trait ready so building them is additive.
The event bus (`InferenceEvent`), the kernel (per-model performance —
modality-neutral already), the store schema, and the GC-managed weights directory
are all modality-neutral; **do not special-case text** in the composition root,
the store, or the `/v1/` envelope in a way that would force a sibling crate. The
one net-new build is the whisper.cpp / TTS-engine provisioning tuple (engine.md
concern 5d).

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
handler (cc, org, a VDB handler) without a per-touch ledger. This is a
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
- **cc (agents) → inference (api)** via `llm-calls` — usage/metering-shaped
  completion calls from agents that DO choose models (Claude Code does not route
  here — INTENT #40). Owned by `api`. (see scaffold/contracts/llm-calls.md)
- **inference → db** via `db-inference-init` — **PARENT-owned**: fresh-node
  control-plane `ops` bootstrap as a `bin/db` CLI subprocess (boot-safe, no mesh,
  distinct from store's `substrate.db`; concern 5). (see
  authored: scaffold/contracts/db-inference-init.md)
- **inference (each daemon) ↔ mesh.service-registry** via `service-lookup` —
  **PARENT-owned participation**: registers the per-node `inference` NodeScoped
  instance via `chassis` (concern 2), now advertising `modalities_available`
  alongside `models_available` (concern 7a; Proposed contracts below). Client half
  is `chassis`; I propose the inference-side Registration shape below. (see
  scaffold/contracts/service-lookup.md)
- **aui-client → inference (api)** via `v1-completion-api` — the operator's AUI
  obtains S2T/T2S from the mesh as modality-tagged completions, byte-transparently
  forwarded by completion-router to a node holding the warm modality (concern 7d).
  No new contract — a modality tag on the existing `/v1/completions` request;
  `aui-client.md` (batch 6) owns the client seam. (see
  scaffold/contracts/v1-completion-api.md)
- **{models, cache, engine} → gc** via `gc-managed-dirs` — the children's in-process
  edge, now expressed through the parent-selected `GcHandle` (Embedded/Remote;
  concern 6). Owned by the children + `gc`. (see
  scaffold/contracts/gc-managed-dirs.md)

Cross-cutting mesh protocols (surface-schema-style, authored by
`chassis`/`supervision`/`pubsub-relay`; inference is a **consuming party** —
I define its concrete participation, do not re-author the wire):

- **every service ↔ mesh** via `restart-protocol` — inference's concrete
  interruptibility mapping + ladder behavior (concern 4). Client half `chassis`
  (the `RestartPolicy` seam), daemon side `supervision`. (participation note below)
- **every service ↔ mesh** via `pubsub-protocol` — the envelope the event bus and
  all mesh frames ride (concern 3). Authored by `pubsub-relay`/`types`.
- **every service → mesh** via `surface-schema` — inference publishes its boring
  surface (completions table, models, system state, kernel curve + the interaction
  calls) for schema-driven dashboard rendering (INTENT #46, concern 6). Assembled
  by `api` from its route table, published + reconnect-replayed by `chassis`.

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45): the eight
children interconnect via `store-access`, `engine-exec`, `system-state`,
`model-ensure`, `kv-cache`, `benchmark-collections`, `api-dispatch`, and
`kernel-confidence` (each child's file); the parent compiles in `chassis`
(the mesh handle) and `substrate-types` (shared vocabulary).

## Nesting

Parent: (none — top-level L5 app-crate). Children (nested internal libs, compiled
into the `inference` daemon, never standalone top-level crates — INTENT #22):
**[store, engine, scheduler, models, cache, telemetry, benchmark, api]**. The
parent/child structure lives here + `overview.md`, not the directory layout.

## Thoroughness level

**implementation-ready** for the refit's mesh integration: the two-boot-step
`chassis` insertion (concern 1), the per-node NodeScoped `inference`
registration + the FleetAlias-as-policy alignment (concern 2), the event-bus →
pub/sub re-plumb (concern 3), the concrete interruptibility state table + the
four ladder rungs mapped onto real subsystems (`pause_execution`/drain/KV-save/
`recover_running`) (concern 4), the `db-inference-init` subprocess boot path
distinct from store's schema (concern 5), the `GcService`→`GcHandle` migration
(concern 6), the mesh-exposed-vs-internal route split (concern 8), and
loopback-`:8420`-through-`:3649` (concern 10) are all decided and specified
against real code + frozen neighbor seams. **implementation-ready for the wave-3
modality seams** (concern 7): the `modalities_available` capability advertisement,
the warm-model `warm` config set + never-evict pin, and the mesh-affinity
placement path (Pi-can't-run-S2T) are decided against engine.md concern 5 and the
router's existing model-affinity plane. **approach-sketched** for: the S2T/T2S
backends themselves (concern 7 / engine.md concern 5 — the seam is ready, no
`WhisperBackend`/`PiperBackend` is built this wave; the warm-residency wiring is a
flagged scheduler/models co-design) and the light-provenance column threading
(concern 9 — a small store addition, direction recorded).

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), per the wave2-plan tier
(`inference` = Opus), grounded in the REAL `lib/inference`/`bin/inference` +
`lib/config` source and every child lib, the batch-1..4 neighbor designs
(`service-registry`, `completion-router`, `supervision`, `gc`, `db`, `vfs`,
`types`, `store`, `api`), and the wave-3 batch-1/5 designs (`chassis` — absorbing
`mesh-client`; `engine`/`scheduler`/`models` — the modality + warm-model seams).

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
route-split are near-transcription. Sequence **after** `chassis`,
`service-registry`, `gc`, and `db` are filled (it rides all four) and
**alongside** its own children (`api`/`store`/`engine`). The wave-3 modality
concern (7) is seam-only and rides engine.md concern 5 — fill it alongside
`engine`; the warm-model residency wiring waits on the scheduler/models co-design.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `db-inference-init` (inference → db) — fresh-node control-plane bootstrap with graceful standalone degradation. → `scaffold/contracts/db-inference-init.md`
- `service-lookup` (inference → mesh.service-registry) — per-node `NodeScoped` registration under the `inference` slug (the adopted concern-5 resolution). → `scaffold/contracts/service-lookup.md`
- `restart-protocol` (mesh ↔ inference) — participation: the concern-4 interruptibility mapping AS SUPERSEDED at harmonization (benchmark sweeps report `Idle`, not `CriticalSection` — scheduler.md concern 3; only model swap / KV save-restore are critical). Applied provisionally, pending the restart philosophy (friction-round 2, INTENT #119 — see concern 4). → `scaffold/contracts/restart-protocol.md`

Also a party to (authored elsewhere / cross-cutting): `inference-events`, `llm-calls`, `node-state-poll`, `v1-completion-api` — see `scaffold/contracts/`.

---

## Proposed contracts (wave 3)

Two additive, non-decisive proposals the wave-3 modality work surfaces. The
sibling units own the authored surfaces (`service-registry`/`types` and
`completion-router`); inference proposes only the shapes it is the source of.

### P1. `modalities_available` on `NodeCapabilities` → into `types` / `service-lookup` (INTENT #166 Q12)

Inference advertises, per node, which inference modalities it can serve, so the
completion-router can place a modality-tagged request on a capable node
(concern 7a/c). Additive, mirrors the existing `models_available`:

```rust
// on NodeCapabilities (types) — additive
modalities_available: Vec<Modality>,   // Modality = the engine.md concern-5 input→output pairing
```

- **Reconciliation flags:** `Modality` is engine.md's proposed `types` addition
  (P1 there); a node that runs only T2T advertises `[Modality::T2T]`; a Pi with no
  S2T-capable hardware simply omits S2T (the placement constraint, concern 7c). The
  router treats this as one more **affinity axis** on its existing model-affinity
  plane — **no new routing plane, no authority dependency** (OQ-1 stays PARKED).

### P2. Warm-model residency as inference config (not a wire contract) (INTENT #166 Q12)

The `warm` set (concern 7b) is **inference-local config**, not a contract — but it
is the source of two flagged co-design obligations recorded here so the seams
carry: (a) the **scheduler** must treat a `warm` model as never-swapped (pinned
resident, bypassing `DebouncedSwapEvaluator` eviction — scheduler.md); (b)
**models** must lock a `warm` model **never-evict** for the process lifetime
(models.md concern 4's lock-while-resident, made permanent by config). No new
contract file; flagged for the scheduler/models fill.

