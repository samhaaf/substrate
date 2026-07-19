# engine

**Status:** WAVE-2 REFIT of the wave-1 implementation-ready design. The
carried-forward core — `ExecutionEngine`, the `InferenceBackend` trait +
`LlamaBackend`, `LlamaProcess`, `LlamaClient`, `SlotTracker`/`SlotGuard`, and the
`BackendProvisioner` — is **already built and correct** (real code in
`lib/engine/src/{lib,backend,process,client,slot,provision}.rs`); wave-2 does
**not** rewrite it. It *adds the mesh-era integration the wave-1 file deferred*:
(1) provisioning of llama.cpp builds in the VFS+gc world (gc reached through the
`GcHandle` seam; a fleet-wide build cache in VFS, INTENT #33); (2) the
long-standing "broadcast channel for token events" gap — a per-token streaming
seam designed against the `pubsub-protocol` envelope so the dashboard and the
completion-router can finally see tokens; (3) the multi-modality seam moves the
`InferenceBackend` trait needs **now** so image/video backends slot in later
without a signature refactor (INTENT #18/#38); and (4) provisioner graceful
degradation when the node is offline from the tailnet. **Nesting:** internal lib
of `inference` (`lib/engine`), never a standalone crate (INTENT #22/#54).

## Charter

`engine` is the **execution engine**: it owns the `llama-server` child process,
the `InferenceBackend` trait seam (with the `Vllm`/`Mlx`/`RemoteApi` stubs and,
now, the image/video seam), RAII slot accounting for concurrency, SSE completion
submission to the backend, drain-before-swap, and the `BackendProvisioner` that
auto-provisions the exact llama.cpp build a model needs (**no pre-installed
llama.cpp** — INTENT #4). It executes ONE generation on ONE resident model and
manages that backend process. **Boundary — what it does NOT own:** it does not
decide WHAT to run or WHEN (that is `scheduler`, over `engine-exec`); it does not
own KV state on disk (that is `cache`, which engine calls over `kv-cache`); it
does not own model-weight download or eviction (that is `models`, over
`model-ensure`); it does not own the SQLite system-of-record (that is `store`,
over `store-access` — engine only marks completion terminal state + inserts
result blobs); and it does not speak mesh directly — the `/v1/` surface,
pub/sub relay, node registration, and restart-protocol participation are
`inference`'s (via `api` + `mesh-client`). Engine emits **structured domain
events onto an in-process broadcast bus**; translating those to
`pubsub-protocol` topics and to the `inference-events` contract is `api`'s job.
The engine stays modality-agnostic on purpose (INTENT #18): the same crate hosts
text today and image/video generation backends later.

## Primary design concerns

### 1. The carried-forward core is correct — wave-2 is strictly additive

`ExecutionEngine`'s process lifecycle + concurrency accounting is the hard,
isolable reason engine earns its own component, and it is **already
implementation-ready and tested** (slot tests in `slot.rs`, matcher/platform
tests in `provision.rs`). Slots are RAII-accounted (`SlotGuard` decrements on
drop with `saturating_sub`, so a panicking completion cannot leak a slot); the
backend process survives completion churn but is drained and killed cleanly on
model swap (`swap_model` → `load_model` kills the old `LlamaProcess` before
spawning, `kill_on_drop(true)` + a `Drop` SIGTERM backstop guarantee no orphan);
`max_concurrent_completions` is fixed at process launch (resizing slots would
drain the KV cache). **Nothing in wave-2 changes these mechanics.** The refit
touches four surfaces only: the provisioner's storage tiers (concern 2/3), the
event bus (concern 4), the trait signature's modality-neutrality (concern 5),
and a new restart-triggered KV-save hook (concern 6). Everything else is
frozen — this is the elegance the operator wants (INTENT #38: get the structure
right once, improve services in place, never refactor the structure).

### 2. Provisioning in the VFS + gc world — `GcHandle` + a fleet build cache

Two integrations replace the wave-1 "`Arc<GcService>` + GitHub only" model:

**(a) gc is reached through `GcHandle`, not a raw `Arc<GcService>` (gc.md
concern 2).** Today `BackendProvisioner::new(gc: Arc<GcService>)` calls
`gc.register_dir`/`register_entry`/`lock` in-process. Wave-2 replaces that field
with gc.md's `GcHandle { Embedded(Arc<GcService>) | Remote(GcClient) }`, selected
**once** at `InferenceService::start`: if the node's `gc` daemon is resolvable
via mesh, `Remote` (every command lands on the single per-node `~/.substrate/gc.db`
owner, INTENT #28); else `Embedded` (standalone, byte-for-byte today's path).
The provisioner's call sites become async but otherwise unchanged, and the
newly-provisioned binary uses gc's wave-2 **atomic `register_and_lock`** (one
round-trip; replaces today's `register_entry` + `lock(30d)` two-step) so the
running binary is protected from the very next sweep with no race window. The
backend-cache directory `{llama_data_dir}/backends/llama-{version}/` is a
gc-managed dir with a *pin-forever-is-forbidden* policy: running builds are
lock-with-expiry protected (renewed while resident), stale builds age out under
budget — the `gc-managed-dirs` edge, now expressed through `GcHandle`.

**(b) A fleet-wide build cache in VFS (INTENT #33 — "cache built binaries
mesh-wide… a key-value store with our builds").** A provisioned llama.cpp build
for a `(version, platform)` is an **immutable, content-addressed VFS artifact**
(`vfs://backends/llama-<version>-<platform>/`, `FileClass::Immutable`). The
provisioner gains a VFS tier *between* the local-disk check and the GitHub
download: before hitting `api.github.com`, it asks VFS whether a compatible build
already exists in the mesh, and if so **pulls it** over `vfs-content`
(access-can-migrate caches it local for reuse — vfs.md concern 7) instead of
re-downloading and re-extracting from GitHub. After a fresh GitHub download +
extract, the provisioner **publishes** the extracted build back into VFS so the
next node (a cold Pi, a peer that just joined) provisions from the fleet, not the
internet. This is a NEW edge, `engine-vfs`, not named in the wave-2 inventory —
surfaced explicitly (see Contracts section) and flagged; it is optional (a
`VfsHandle` capability handed down by inference's composition root, absent when
no mesh), so it never becomes a hard dependency. Model *weights* becoming VFS
artifacts is `models`' parallel story (`model-ensure`/`models-vfs`); backend
*builds* are engine's — both are the same "inference artifacts in VFS" pattern,
reconciled with `models` in this batch (friction note).

### 3. Offline-from-the-tailnet graceful degradation — a layered fallback ladder

The provisioner must never block the node and must degrade honestly when mesh
peers are unreachable. Crucially it distinguishes **tailnet-offline** (can't
reach mesh/VFS/gc-daemon peers) from **internet-offline** (can't reach GitHub) —
they are independent, and a walk-along Pi can be either or both. `ensure_binary`
is a strict fallback ladder, each rung self-sufficient:

1. **Already provisioned on local disk** (`find_binary` hit) → return it. **No
   network of any kind.** This is today's fast path and is *always* available
   offline; it is the whole reason builds are cached under a stable
   version-keyed dir. A node that has ever run a model keeps running it offline.
2. **Mesh/VFS reachable** → pull a compatible cached build from the fleet
   (concern 2b). Works with **no public internet** as long as one peer holds the
   build (INTENT #33's payoff). Uses `Embedded` or `Remote` gc as available.
3. **Public internet reachable** (tailnet may still be down) → the existing
   GitHub download + extract path (`resolve_latest_tag`/`fetch_release`/
   `download`), then register with gc and — if mesh is present — publish into
   VFS for the fleet. GitHub failures already surface as catchable
   `SubstrateError::Transport`/`Config`.
4. **Neither a cached build nor any reachable source** → a **catchable
   `EngineError::BackendUnavailableOffline { version, platform }`** (new sub-enum
   variant, INTENT #84 CAP-honesty style). The engine does **not** panic and the
   `inference` daemon does **not** exit: it degrades to serving whatever model is
   *already resident* (standalone), reports the missing-backend condition up
   through the event bus (concern 4) for the dashboard, and retries provisioning
   opportunistically when connectivity returns. gc runs `Embedded` throughout —
   standalone operation is the embedded path, unchanged (gc.md concern 2).

The single load-bearing rule: **provisioning is best-effort and layered; the
node's already-working state is never sacrificed to a failed fetch.**

### 4. The per-token streaming seam — the broadcast bus, against `pubsub-protocol`

The wave-1 gap: token events flow **only** through the per-completion
`mpsc::Sender<StreamEvent>` handed to `submit()`; the engine's *broadcast*
channel carries model **lifecycle** events (`ModelLoading`/`ModelLoaded`/…) and a
`CompletionMetricsRecorded` sample, but **no tokens** and **no clean
started/finished deltas**. So nothing fleet-observable can stream tokens, and the
completion-router can't see per-completion load transitions. Wave-2 closes this
with **one generalized in-process bus and a tee**:

- **Generalize `broadcast::Sender<LifecycleEvent>` → `broadcast::Sender<InferenceEvent>`**
  (capacity ~256, **lossy** on lag — matches inference's existing bus and
  pubsub-relay's lossy contract, concern 6 there). `InferenceEvent` is a superset:
  the existing model/backend lifecycle + metrics variants, PLUS the new
  `CompletionStarted { id, model_id }`, `CompletionFinished { id, model_id, success }`,
  and `Token { id, model_id, index, text }`.
- **The tee lives in `ExecutionEngine::submit`, not in the backend.** `submit`
  hands the backend an engine-internal `mpsc`; a forwarder task copies each
  `StreamEvent` to (1) the caller's **reliable, backpressured** `token_tx` (the
  direct `/v1` stream client's path — unchanged semantics) and (2) the **lossy**
  `InferenceEvent` broadcast bus, enriched with `completion_id` + `model_id`.
  Keeping the tee in the engine means the `InferenceBackend` trait never learns
  about the observability bus (it still speaks only `StreamEvent` into an mpsc) —
  preserving the modality seam (concern 5).
- **api maps bus → topics; the engine owns no mesh vocabulary.** `api` bridges
  `InferenceEvent` onto `pubsub-protocol` (types.md `Envelope`/`Event`):
  `Token` → topic `inference.completion.<id>` (per-completion-ID subscription,
  INTENT #5 — a dashboard viewing one completion subscribes exactly that leaf);
  `CompletionStarted`/`CompletionFinished`/`model.loaded`/`model.evicted` →
  `inference.<node>.*` (the five load-affecting kinds the completion-router folds
  into its `NodeRegistry` projection — completion-router.md concern 3). The
  **reliable** direct stream stays on `v1-completion-api`'s WS relay; the **lossy**
  tee feeds observers. This split is deliberate: a slow dashboard must never
  backpressure a completion, and the router explicitly needs only the coarse
  started/finished deltas, **not** the token firehose. This is the exact "broadcast
  channel for token events" the design has owed since wave-1, now grounded on the
  frozen envelope.

### 5. Multi-modality seams the `InferenceBackend` trait needs NOW (INTENT #18)

The operator kept the crate named `inference` precisely so text-to-image /
text-to-video / image-to-image extend *this* crate and its `InferenceBackend`
seam rather than spinning out sibling apps. Today's trait is text-shaped in three
places that would each force a **signature refactor** later (the expensive kind
INTENT #38 forbids). The cheap fix is to make three additive moves **now**, while
only `LlamaBackend` (+ text stubs) exist, and implement nothing image/video:

- **(a) Get the llama-specific payload OUT of the trait signature.**
  `run_completion(&self, id, payload: &CompletionPayload, token_tx)` couples the
  trait to llama-server's wire shape (`CompletionPayload{prompt, n_predict, …}`).
  Change the trait to take a **modality-neutral request** (the `store::CompletionRow`
  it already has, or a small `types` `GenerationRequest`), and let each backend
  build its **own** internal wire payload — `CompletionPayload` becomes strictly
  `LlamaBackend`-private (it already lives in `backend.rs`). Renaming the method
  `run_generation` is optional polish; moving the payload off the signature is
  the load-bearing, refactor-avoiding change.
- **(b) Generalize the streamed item beyond `Token`.** Image/video emit
  *progress* (denoise step k/N, frame f) and *artifacts*, not text tokens. Make
  the streamed event type an additive enum — text backends emit `Token{text}`;
  future backends emit `Progress{done, total}` and `Artifact{ref}` — with a
  `#[serde(other)]` catch-all so an older consumer tolerates a new variant
  (types guardrail 4). `StreamEvent` lives in `types`; this is a flagged,
  additive `types` change, reconciled in that pass.
- **(c) Result carries outputs, not just text.** `CompletionResult.text:
  Option<String>` generalizes to `outputs: Vec<Output>` where `Output =
  Text(String) | Artifact(VfsBlobRef)` (additive; text = one `Text`). Image/video
  outputs are **written to VFS as immutable blobs** and referenced by content
  hash — the same VFS-artifact story as concern 2b, closing the loop: generation
  *inputs* (weights, builds) and *outputs* (artifacts) both live in VFS.
- **(d) A capability descriptor on the trait.** Add `fn modality(&self) ->
  Modality` (Text | Image | Video | Audio) and keep `max_concurrent_completions`
  (already modality-agnostic — `SlotTracker` is a pure admission counter, needs
  no change; VRAM-bound image gen accounts slots identically). This lets
  `scheduler`/`api` route by modality later without touching engine internals.

`Vllm`/`Mlx` stubs stay **Text** modality; `RemoteApiBackend` remains the
proxy-to-external seam (a legitimate future *fallback* backend when local hardware
is unavailable — distinct from CCD, which is external Claude-Code and explicitly
NOT routed to inference, INTENT #40). **Implement none of image/video now** —
this concern only pins the seam so later is additive.

### 6. KV-cache save/restore finally has a trigger — the restart save-window

The charter's "KV-cache save/restore hooks" (`LlamaClient::slot_action(save|
restore)`) are wired but uncalled. Wave-2 gives them a concrete driver: the
`restart-protocol` **L3 SaveWindow** (~10s, INTENT #77). When `inference` (via
`mesh-client`) receives an L3 restart signal, it drives the engine to **save the
resident slots' KV state** through the `kv-cache` seam before yielding, so an
in-flight prefix survives the process replacement and is restored on the new
build (concern 5 of `cache`). `drain(timeout)` and `cancel_all_running` are the
L2 (finish-and-relinquish) and L4 (kill) behaviors respectively — the engine
already exposes exactly the primitives the ladder needs (`drain`,
`cancel_all_running`, `take_running_ids` for crash-recovery requeue,
`running_count` for the idle signal). No new engine mechanics; wave-2 only names
which restart level calls which existing primitive.

### 7. Interruptibility inputs engine feeds inference's restart-protocol

Engine is a lib, so it does not *participate* in `restart-protocol` — but it is
the source of inference's `Interruptibility` (types.md / supervision.md): a model
swap in progress is a `CriticalSection` (interrupting mid-`load_model` orphans a
half-spawned server); `running_count() > 0` is `Interruptible`; both zero is
`Idle`. Engine exposes these as cheap sync reads (`resident_model`,
`running_count`, plus a new `is_swapping()` flag guarding the `swap_model`
window); inference maps them to the interruptibility feed. This keeps the restart
policy in supervision/mesh while sourcing its truth from where the work actually
lives.

## Relationships / edges

Inference-internal contract edges (in-process trait/handle seams — NOT wire
contracts; the eight libs compile into one `inference` daemon, INTENT #22):

- **scheduler → engine** via `engine-exec` — submit/drain, slot state, model
  swap, cancel/abort, crash-recovery id drain. Engine is the provider; I author
  the trait shape below. *(scaffold/contracts/engine-exec.md)*
- **engine ↔ cache** via `kv-cache` — KV/prefix save/restore, now triggered by
  the L3 save-window (concern 6). Co-owned with `cache`; engine's side below.
  *(scaffold/contracts/kv-cache.md)*
- **{models, cache, engine} → gc (embedded)** via `gc-managed-dirs` — engine
  manages the backend-build dir; the shared `GcApi`/`GcCommand`/`GcEvent`
  vocabulary is **authored by `gc`** (gc.md), expressed through `GcHandle`
  (`Embedded` standalone / `Remote` under mesh). Engine is a **consumer** — I note
  the dirs it manages + the `register_and_lock` preference, not re-author.
  *(scaffold/contracts/gc-managed-dirs.md)*
- **engine → store** via `store-access` — `insert_result`, `mark_completed`/
  `mark_failed` on completion terminal transitions (the tee-forwarder task). Store
  owns this contract; the observer-under-mutex discipline (store.md concern 1)
  applies — engine's writes are the terminal transitions that fire the
  `PromiseRegistry` observer. Consumer only. *(scaffold/contracts/store-access.md)*

NEW / newly-surfaced edge (not in the wave-2 inventory — flagged):

- **engine → vfs** via **`engine-vfs`** — the fleet-wide llama.cpp build cache
  (concern 2b, INTENT #33): pull a compatible cached build before GitHub, publish
  a fresh build after. Optional `VfsHandle` capability from inference's
  composition root; absent → the GitHub/local-disk ladder (concern 3). Reconciled
  with `models`' parallel weights-in-VFS story this batch.
  *(scaffold/contracts/engine-vfs.md — MISSING)*

Feeds an inference-owned cross-cutting contract (engine is the event *source*, not
the contract party):

- the `InferenceEvent` broadcast vocabulary (concern 4) is what `api` bridges onto
  `pubsub-protocol` topics for **`inference-events`** (router load feed) and the
  per-completion token stream. `api`/`inference` own those contracts; engine only
  emits the domain events. Flagged for the batch-5 `api` pass so the five
  load-affecting kinds the router depends on are guaranteed emitted.

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45): imports
`substrate-types` (`CompletionId`, `ModelId`, `StreamEvent`, `CompletionResult`,
the generalized `InferenceEvent`, the additive `Output`/`Modality`, `error`
vocabulary), holds a `GcHandle` (gc), a `Store` handle, and an optional
`VfsHandle` — all handed down by inference's composition root, never linked
cross-app in-process.

## Nesting

Parent: `inference` | Children: none. Modules `backend`, `client`, `process`,
`provision`, `slot` under `lib/engine` — libraries under the inference app
(INTENT #22), never a top-level crate. Matches `overview.md`'s tree
(engine under inference, no children).

## Thoroughness level

**implementation-ready.** The carried-forward core (process/slot/client/backend
mechanics) is already built + tested and is treated as frozen. The wave-2 deltas
are designed to implementation depth: the `GcHandle` swap and `register_and_lock`
use (concern 2a); the VFS build-cache tier + `engine-vfs` shape (2b); the
offline fallback ladder + the `BackendUnavailableOffline` catchable error (3);
the `InferenceEvent` bus generalization + the `submit`-interposed tee + the
lossy/reliable split + topic mapping (4); the three additive trait moves + the
capability descriptor (5); and the restart-save-window trigger for `kv-cache`
(6). **Genuinely downstream / flagged:** (a) `engine-vfs` is a newly-surfaced
pair reconciled with `vfs`/`models` in the per-pair round; (b) the `types`
additions for `StreamEvent` generalization + `Output`/`Modality` are proposed
here, authored in the concurrent `types` pass; (c) the exact `InferenceEvent`
payload schemas are reconciled with `api` (batch-5 sibling) so `inference-events`
guarantees the router's five kinds.

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), grounded in the full real
source (`lib/engine/src/{lib,backend,process,client,slot,provision}.rs`) and the
batch-1/2/3 designs it must not contradict: `types.md` (Envelope/Event/
provenance/node/restart + guardrail 4), `mesh-client.md` (restart-protocol client
half, pubsub client half), `pubsub-relay.md` (envelope + lossy contract + the
`inference.*` topic taxonomy + the per-completion leaf), `completion-router.md`
(the five load-affecting event kinds it consumes; byte-transparent forward vs the
lossy event tee), `gc.md` (`GcHandle` Embedded/Remote, `register_and_lock`,
per-node single store), `vfs.md` (Immutable content-addressed artifacts,
`vfs-content` pull, access-can-migrate), and `supervision.md` (the 4-level ladder,
interruptibility), plus the batch-5 siblings `cache.md`/`store.md`/`api.md`/
`inference.md`. INTENT #4/#18/#28/#33/#38/#40/#77/#84.

## Suggested fill-model

**Carried-forward core: no Filler dispatch** — the process/slot/client/backend
code is pre-built and tested; the Skeleton Builder treats it as done. **Wave-2
deltas: implementation-ready + moderate complexity → mid model OK**, with two
carve-outs and one sequencing constraint. Carve-outs for a careful hand: (1) the
`submit`-interposed **tee** (concern 4) — the one spot where a lossy broadcast
send must never backpressure or perturb the reliable `token_tx` path, and where a
dropped-on-lag token is correct-by-contract, not a bug; (2) the **offline
fallback ladder** (concern 3) — the tailnet-vs-internet distinction and the
never-sacrifice-resident-state rule are the correctness edge. Sequencing: fill
engine's `GcHandle`/`GcClient` path and the `engine-vfs` tier **after** the
Contract Harmonizer freezes the `pubsub-protocol` envelope, the shared gc
command/event structs, and the `vfs-content` shape (engine serializes all three).
The trait-signature moves (concern 5) are mechanical once `types`' `StreamEvent`/
`Output`/`Modality` additions land.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `engine-exec` — (scheduler → engine) — the execution surface. → `scaffold/contracts/engine-exec.md`
- `kv-cache` — (engine ↔ cache) — save/restore, now save-window-triggered. → `scaffold/contracts/kv-cache.md`
- `gc-managed-dirs` — (engine → gc) — participation (authored by gc). → `scaffold/contracts/gc-managed-dirs.md`
- `store-access` — (engine → store) — participation (authored by store). → `scaffold/contracts/store-access.md`
- `inference-events` — engine is the event SOURCE (api bridges the broadcast bus onto pub/sub). → `scaffold/contracts/inference-events.md`

Also a party to (authored elsewhere / cross-cutting): `node-state-poll`, `pubsub-protocol`, `vfs-content` — see `scaffold/contracts/`.

Component-side notes:
- `engine-vfs` (engine → vfs) — the fleet llama.cpp build cache (INTENT #33):
  proposed edge, but NO contract file was authored (coverage gap flagged for
  the owners); not citable as an authored edge.

