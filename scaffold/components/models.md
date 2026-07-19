# models

**Status:** WAVE-2 REFIT of the wave-1 `approach-sketched` design. The wave-1
core is **real, working code** (`lib/models`: `lib.rs`/`ModelManager`,
`download.rs` resumable HF/https/file pipeline with `.partial`+atomic-rename+
`Range` resume, `registry.rs` read-only view) and is **preserved unchanged where
it holds**. Wave-2 supersedes three things the new substrate invalidates or
completes: (1) the bespoke disk-budget arithmetic becomes a **gc per-directory
policy** (the gc.md/vfs.md migration) reached through gc.md's `GcHandle`
seam instead of a raw `Arc<GcService>`; (2) the **`download_model` REST stub**
(returns 202, does nothing — wave-1's flagged gap) is replaced by a real
**download-job orchestrator** (spawn, singleflight, progress, status, events);
(3) the **model inventory** (`is_downloaded`) is published the mesh way, feeding
completion-router's Tier-2 affinity. **Nesting:** internal lib of `inference`
(`lib/models`), L5 — never standalone (INTENT #22/#54). Grounded in the live
`lib/models` + `lib/api/src/rest.rs` download stub + `lib/inference/src/lib.rs`
wiring, the batch-1/2/3 designs (types.md `node`/`event`/`pubsub`, gc.md
`GcHandle`/`GcApi`/DirPolicy, vfs.md content-plane/access-migrate,
completion-router.md concerns 1–3, service-registry NodeScoped), and INTENT
#4/#34/#48/#59/#66/#85.

## Charter

`models` is **model-weight lifecycle on one inference node**: it makes a model's
GGUF present on local disk and tracks its download state, and it publishes that
state so the fleet can route to it. It owns exactly four things: **registry
sync** (upsert the config `[[models]]` list into the store — unchanged);
**source resolution + resumable transfer** (`hf:`/`https:`/`file:` →
`.partial`→atomic-rename, integrity-verified — the `download.rs` byte-mover, kept
+ hardened); **download orchestration** (the NEW job layer: singleflight, async
kickoff, progress/status surfacing, terminal events — the honest replacement for
the 202 stub); and **the local weights directory's disk discipline**, delegated
DOWN to `gc` as a policy-bearing managed directory (budget + LRU eviction).

**Boundary — what `models` does NOT own.** It does **not load a model into a
backend** (that is `engine`) or **decide when to swap/preempt** (that is
`scheduler`, which asks via `model-ensure`). It does **not own `is_loaded`
/ resident state** — that is live `SystemState.resident_model`, written by
`engine` on load/unload and read from telemetry (types.md keeps resident OUT of
static capability to avoid two sources of truth). It owns **`is_downloaded`**
only. It holds **no completion/result data** (`store`'s other tables) and **no
cross-node placement authority** — it is a per-node library, one instance per
inference node, reached only through its parent `inference`. It does **not run
GC mechanics** (TTL/sweep/eviction are `gc`'s; models supplies the policy and the
`make_room`/`lock` calls). It does **not decide fleet-wide which node holds which
model** — that is a *derivation* the completion-router computes from the
inventory models publishes.

## Primary design concerns

### 1. Download orchestration: per-node origin download — NOT proactive mesh replication (the decision the brief asks for)

The brief poses the fork directly: *one node downloads and mesh replicates, or
every node downloads its own?* **Decision: every inference node owns its own
weights; a weight lands on a node only because that node elected to serve the
model — there is NO proactive push-replication of model weights across the
mesh.** This is a deliberate **deviation from vfs.md**, which names "an immutable
40 GB model weight replicated 3×" as the *archetype* Immutable content-addressed
blob and would place weights under a replication-factor policy. I override that
**for the `vfs://…/models` prefix specifically**, and record why (the
bandwidth/disk tradeoff the brief asks me to flag):

- **Proactive R=3 replication of tens-of-GB weights is an anti-win on a personal,
  WAN-linked mesh.** A weight is only useful on a node that will *run* it (has the
  accelerator, will `mmap` it into llama-server). Replicating a 40 GB Metal GGUF
  onto a storage-only Pi cold node (INTENT #48) — or onto a laptop over cellular
  Tailscale — burns bandwidth and disk for a copy that will never be loaded. The
  completion-router's Tier-2 affinity is explicitly "which *inference* nodes have
  this ready to load," a scheduling fact, not a durability fact.
- **Durability of weights is cheap to re-derive, unlike data.** A lost weight is
  re-downloadable from its origin (`hf:`/`https:`) or copyable from a peer — it is
  not irreplaceable state. So the "replicate for safety" rationale that justifies
  VFS's factor policy for *data* blobs does not transfer to *weights*. This is the
  same reasoning store.md uses ("results durably outlive on-disk weights, which is
  what makes per-machine GC safe") — weights are the disposable tier.
- **Consequence for the `vfs://…/models` directory policy:** if weights are ever
  registered as VFS content (concern 6, deferred), their `DirPolicy` sets
  `default_replication: 1` and eviction `LeastRecentlyAccessed` — i.e. VFS's
  own **access-can-migrate / `ReplicaKind::Cache`** semantics (vfs.md concern 7),
  not `Durable` replicas that count toward a factor. A weight is a cache copy,
  never a durable one.

**The bandwidth win is kept without the waste — via a source ladder, not
replication.** When a node needs a model it lacks, `ensure_available` resolves a
**source ladder** (concern 6): local disk → **a mesh peer that already holds the
identical weight** (pull at LAN speed) → origin (`hf:`/`https:`). The peer-pull
leg IS the bandwidth optimization the brief is fishing for — but it is
**pull-on-demand by the electing node**, i.e. exactly vfs.md's access-migrate,
NOT push-on-download by the producer. The wave-2 *default path* is origin
download (implementation-ready, works today); the peer-pull leg is designed here
but **deferred** (concern 6, `models-vfs` edge) since it rides vfs (batch-3).

### 2. The honest download path — a job orchestrator, replacing the 202-that-does-nothing

`lib/api/src/rest.rs::download_model` returns `202 Accepted` and *does nothing*
(the manager isn't even in `ApiState`; the comment says "This is a stub"). The
real `ensure_available` **does** exist and works — but it **blocks on the full
multi-GB download** (`.await`s `download.rs`), which cannot back a 202-return REST
handler. The wave-2 fix is the missing **orchestration layer** between the
byte-mover and the callers:

```
lib/models/
  registry.rs   (unchanged: read-only ModelRegistry over store)
  download.rs   (kept + hardened: the byte-mover — hf/https/file, .partial, resume, +sha256)
  jobs.rs       (NEW: DownloadRegistry + DownloadJob state machine — singleflight, progress, events)
  lib.rs        (ModelManager: source ladder, gc discipline, GcHandle field)
```

`jobs.rs` owns an in-memory `DownloadRegistry: Mutex<HashMap<ModelId, JobHandle>>`:

- **`request_download(id) -> DownloadHandle` (async kickoff).** If a job for `id`
  is already in-flight, **return the existing handle** (singleflight — two
  concurrent requests must never both download the same 40 GB weight; this is the
  correctness heart of the stub fix). Otherwise `tokio::spawn` the download task
  and register its handle. Returns *immediately* — this is what makes the 202
  honest: the endpoint now actually kicks off (or attaches to) real work and hands
  back a `download_id` + live status.
- **`download_status(id) -> DownloadStatus`** — `{ state: Absent|Downloading{
  bytes_done, total: Option<u64>, pct }|Ready|Failed{ message } }`, derived from
  the live job + the store row. Powers `GET /v1/models` / `GET /v1/models/:id`
  status and the `model-ensure` status surface (unblocks router Tier-3 readiness).
- **`ensure_available(id) -> path` (blocking, unchanged signature)** now delegates
  to the registry: it `request_download`s (or joins the in-flight job) and awaits
  that ONE shared task to completion, then returns the path. So the blocking swap
  path (scheduler → models) and the async REST path share **one** underlying
  transfer — no double-download, ever.

**Progress is in-memory, not persisted.** A percentage that changes every chunk
must not hammer SQLite (store.md single-writer mutex). Only **terminal
transitions** (downloaded / failed / evicted) hit the store (`set_model_downloaded`
/ `clear_model_file`). `ModelStatus::Downloading` (already an enum variant in
`types::model`) is **derived** when an in-flight job exists — no new column.
Progress is also emitted as throttled pub/sub deltas (concern 5).

### 3. Storage as a gc-managed directory with a DirPolicy — the per-directory-policy migration + `GcHandle`

Today `ModelManager` holds `gc: Arc<GcService>` and enforces the budget with
**bespoke arithmetic**: `make_room(models_dir, expected_bytes)` before each
download, `model_weights_budget_bytes` in config, and a store-`last_used_at` LRU
notion. Wave-2 collapses this onto gc's **per-directory policy** (INTENT #48; the
gc.md/vfs.md migration):

- **The weights directory is registered once with gc under a `DirPolicy`**
  (`register_dir` with `{ max_bytes: budget, eviction: LeastRecentlyAccessed,
  default_ttl, unit: Children }`). gc's **continuous hourly sweep** then owns
  ongoing budget enforcement — LRU-accessed eviction of *unlocked, unloaded*
  weights — replacing models' hand-rolled "evict LRU unloaded models" step. models
  still calls `make_room(dir, need)` **before** a large download (reserve headroom,
  the gc.md concern-2 `make_room → write → register → lock` pattern), but the
  standing budget discipline is now the DirPolicy'd sweep, not models' own logic.
  The store's `last_used_at` becomes advisory/telemetry (gc's touch-based LRU is
  the authority); models `touch`es gc on each model use to keep the two aligned.
- **`Arc<GcService>` → `GcHandle` (gc.md concern 2).** The field type changes to
  gc.md's two-variant handle — `Embedded(Arc<GcService>)` on a standalone/dev/test
  box with no mesh, `Remote(GcClient)` under mesh (WS to the single per-node gc
  daemon). **No call site in `models` changes beyond the field type** — gc.md
  guarantees the `GcApi` method surface (`register_dir`/`register_entry`/
  `register_and_lock`/`touch`/`lock`/`make_room`) is identical across both. The
  mode is selected once at `InferenceService::start` (inference.md), not here.
- The eviction strategy models requests is gc's **`LruAccessed`** (a weight is hot
  when recently loaded/used) — one of the wave-2 `EvictionPolicy` variants gc.md
  added for exactly this (vs the old `Lru`/`Fifo`). This is a concrete consumer of
  gc's additive policy vocabulary.

### 4. gc-race correctness — `.partial` protection, `register_and_lock`, and lock-while-resident

The wave-1 "delicate correctness surface" (a half-download must never be swept; an
eviction must not race an in-flight download) is closed with three specific moves:

- **The `.partial` is registered + locked for the download's duration.** gc only
  evicts *registered* entries, so an unregistered `.partial` is safe from sweep —
  but then its bytes don't count toward the budget and the accounting drifts during
  a 40 GB pull. Fix: on job start, `register_entry(partial)` + `lock(partial, ttl)`
  (ttl renewed each progress tick), so the in-flight bytes **count toward budget
  AND are sweep-protected**. On completion: atomic rename → `register_and_lock`
  (gc.md's NEW atomic one-round-trip op) the *final* path → drop the partial's
  lock. This eliminates the register→sweep window the wave-1 two-step left open.
- **Lock-while-resident, not lock-for-24h.** Today models `lock`s a freshly
  downloaded weight for a flat `86400`s — a crude proxy. A model that llama-server
  has `mmap`'d must **never** be evicted from disk, and 24h is both too long
  (pins cold weights) and too short (a model resident for >24h could be swept from
  under the backend). Wave-2 refinement: **lock on load, renew while resident,
  unlock on unload** — driven by the engine/scheduler resident-transition signal.
  This makes gc's eviction correct-by-construction: the never-evict invariant is
  "gc never evicts a locked entry," and *resident ⇒ locked*. The signal source is
  `engine`/`scheduler` (the `is_loaded` writer, store-access + the lifecycle bus);
  **flagged as a batch-5/6 co-design dependency** — until it lands, the flat-ttl
  lock is retained as the safe fallback (a longer-than-necessary pin never
  corrupts, only wastes headroom).
- **DiskBudgetExceeded is catchable and surfaced.** If `make_room` cannot free
  enough (every candidate is locked/resident), gc returns `DiskBudgetExceeded`
  (gc.md shared taxonomy). models maps it to `model-ensure`'s
  `InsufficientDiskBudget` — the scheduler must handle it (defer the swap / pick a
  smaller model), never a silent stall.

### 5. Inventory publication — `is_downloaded` for the router's Tier-2, the mesh way

completion-router.md concern 1/3 sources per-node model inventory from three
places; models is the **producer** of the download-inventory half. Aligning with
types.md + completion-router.md, `is_downloaded` is published **two ways, and
`is_loaded` is NOT models' to publish**:

- **`NodeCapabilities.models_available: Vec<ModelId>` (snapshot / reconcile).**
  types.md defines this as the DOWNLOAD inventory. models supplies it via the
  existing `registry().downloaded()`; **`inference`/`api` assembles it into the
  node's `NodeCapabilities`** at registration + serves it on `GET /v1/models`
  (which the router's `node-state-poll` reconcile reads). models owns the *data*,
  api owns the *endpoint/registration* — the L5 boundary.
- **`inference.model.*` pub/sub deltas (live-primary).** On each terminal
  transition models emits a `LifecycleEvent` on `inference`'s broadcast bus, which
  `api` bridges to the `inference-events` pub/sub topic the router folds in live
  (completion-router.md concern 3). The kinds models owns:
  `inference.model.download_started`, `inference.model.download_progress`
  (throttled — e.g. every 5% or 2s, never per-chunk), `inference.model.downloaded`
  (→ router adds to `models_available`), `inference.model.download_failed`,
  `inference.model.evicted` (→ router removes). Namespaced `domain.noun.verb` per
  types.md `EventType`. Note the clean split: **`inference.model.loaded`/`evicted`
  for the RESIDENT set is `engine`'s event** (is_loaded); **`inference.model.
  downloaded`/`evicted` for the INVENTORY is models' event** (is_downloaded). Both
  feed the router's inventory projection; they are distinct facts with distinct
  owners, deliberately not conflated (types.md Controversial-decision-3).
- **New `LifecycleEvent` variants.** models adds `ModelDownloadStarted`,
  `ModelDownloadProgress { bytes_done, total }`, `ModelDownloaded`,
  `ModelDownloadFailed`, `ModelEvicted` to inference's `LifecycleEvent` bus enum —
  the mechanism by which the deltas above reach api → pub/sub. Additive.

### 6. Integrity + content-addressing (harden `download.rs`; enable the deferred peer-pull)

`ModelConfig` already carries `sha256: Option<String>` and `expected_size_bytes` —
**unused by `download.rs` today** (a real gap: a truncated/corrupt GGUF is renamed
to final and marked downloaded). Wave-2 hardening (cheap, honest):

- **Verify on completion.** After the atomic rename, if `sha256` is set, hash the
  file and reject on mismatch (delete, `DownloadFailed`, retain nothing corrupt);
  if `expected_size_bytes` is set, check size. Only then `set_model_downloaded` +
  emit `downloaded`. This makes `is_downloaded` mean *verified present*, which the
  router's Tier-2 routing implicitly trusts.
- **The sha256 IS the content address** that a future peer-pull uses. This is the
  hook for concern 1's deferred `models-vfs` leg: `ensure_available`'s source
  ladder, before falling to origin, asks "does any peer hold the blob with this
  content hash?" (discoverable from the router's inventory / vfs metadata) and, if
  so, issues a vfs `Read { path: vfs://models/<sha256>, cache_local: true }` to
  pull it at LAN speed (chunked, integrity-checked, resumable — vfs.md concern 3).
  **Deferred** (rides vfs, batch-3, and needs the operator's blessing on weights
  touching the VFS namespace at all) — `approach-sketched`, `models-vfs` proposed
  below. The default remains origin download.

### 7. Registry sync + store-access + standalone degradation (carried forward, boring)

- **`sync_registry`** (upsert config `[[models]]`, preserve runtime state on
  conflict) is **unchanged** — boring by intent. The per-node registry stays
  per-node: each node's `substrate.toml` lists the models *it* knows how to obtain;
  the fleet-wide picture is the router's aggregation of per-node inventories, not a
  replicated registry. No mesh coupling added here (INTENT #34 no-fault-line: a
  node's registry works with zero peers).
- **`store-access` (models party).** models reads/writes `ModelRow` via the shared
  store (get/list/upsert/`set_model_downloaded`/`clear_model_file`/`touch_model`/
  `total_weights_bytes`). No schema change required for wave-2 (progress is
  in-memory; `Downloading` is derived). One *optional* additive column flagged for
  store's pass: persisting the resolved `content_hash` (= verified sha256) on the
  row, useful for the deferred peer-pull and for cross-restart integrity — additive,
  `store.md` owns the migration.
- **Standalone degradation (INTENT #34, inference.md).** With no mesh: `GcHandle::
  Embedded`, no pub/sub emission (bus has no bridge — events are simply not
  relayed, the local bus still fires), origin-only source ladder. models works
  single-box exactly as today — the mesh integrations are all *additive
  publications*, never a *dependency* for the core download to function.

## Relationships / edges

Contract edges (models is a party):

- **scheduler → models** via `model-ensure` (see
  `scaffold/contracts/model-ensure.md`) — **I OWN/PROPOSE this** (Proposed
  contracts). Carries: blocking `ensure_available(id) -> path` (pre-swap), async
  `request_download(id) -> handle` + `download_status(id)` (the honest 202 path /
  router Tier-3 readiness), and `evict(id)`. The wave-1 no-op stub is now real.
- **{models, cache, engine} → gc** via `gc-managed-dirs` (see
  `scaffold/contracts/gc-managed-dirs.md`) — models is a party; **gc.md authored
  the shared `GcApi`/`GcCommand`/`GcEvent` vocabulary** — I do not re-author it, I
  note models' usage: `register_dir(weights, DirPolicy{LruAccessed})`, the
  `make_room → write → register_and_lock` flow, `touch` on use, `lock` while
  resident. Expressed through `GcHandle` (Embedded/Remote), not `Arc<GcService>`.
- **store ↔ {…models…}** via `store-access` (see
  `scaffold/contracts/store-access.md`) — models is a party; **store.md owns it** —
  I note the `ModelRow` read/write surface + the optional additive `content_hash`
  column.
- **models → vfs** via **`models-vfs`** — **NEW, flagged, DEFERRED** (concern
  1/6): the peer-pull content leg — models as a plain vfs client issuing
  `Read{ path: vfs://models/<hash>, cache_local: true }` to pull a weight from a
  warm peer instead of re-downloading from origin. Not in the wave-2 inventory;
  surfaced here as the concrete answer to the bandwidth question. Rides vfs
  (batch-3) + operator blessing on weights entering the VFS namespace.

Publication paths (models produces; another module owns the wire):

- **is_downloaded inventory** → `inference`/`api` assembles into
  `NodeCapabilities.models_available` (registration + `GET /v1/models`, read by
  the router's `node-state-poll`) and bridges models' `LifecycleEvent`s to the
  `inference-events` pub/sub the router consumes live (concern 5). models does not
  own those endpoints/topics — it owns the data and the delta events.

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45): imports
`substrate-types` (`ModelId`/`ModelConfig`/`ModelRow`/`ModelStatus`/`event`
vocabulary + the `DownloadFailed`/`InferenceError` taxonomy), `substrate-store`
(the `Store` handle), and `substrate-gc` (`GcHandle`/`GcApi`/`EntryKind`/
`DirPolicy`). Consumed only by its parent `inference` (via `ModelManager`), never
cross-app.

## Nesting

Parent: `inference` | Children: none. `lib/models` is an internal library of the
`inference` app (INTENT #22 — a crate is an app; the pieces under inference are
libs), composed by `InferenceService::start`. Modules: `registry` (read view),
`download` (byte-mover), **`jobs` (NEW — orchestrator)**, and the `ModelManager`
root in `lib.rs`. Never a standalone crate/daemon.

## Thoroughness level

**implementation-ready** for: the download-orchestration decision (concern 1 —
per-node origin download, no push-replication, tradeoff flagged); the download-job
layer that fixes the 202 stub (concern 2 — `jobs.rs`, singleflight, async kickoff,
progress/status, terminal events); the gc DirPolicy migration + `GcHandle` field
swap (concern 3); the gc-race closure via `.partial` lock + `register_and_lock`
(concern 4, with lock-while-resident flagged as a batch-5/6 dependency and a safe
flat-ttl fallback); the inventory-publication split (concern 5 — `models_available`
snapshot + `inference.model.*` deltas, is_loaded explicitly out of scope); and the
sha256 integrity hardening (concern 6). **approach-sketched** for: the peer-pull
`models-vfs` leg (concern 1/6 — the bandwidth optimization, deferred behind vfs +
operator blessing) and the exact resident-transition signal wiring (concern 4 —
reconciled with engine/scheduler in batch 5/6). No piece is left as a bare stub:
the deferred items have a named edge and a designed shape.

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), grounded in the full real
`lib/models` source, the `lib/api/src/rest.rs` download stub + `lib/inference/src/
lib.rs::ensure_model` wiring, the `types::model` shapes, the batch-1/2/3 designs
(types.md `node`/`event`/`pubsub`/`error`, gc.md `GcHandle`/`GcApi`/DirPolicy/
`register_and_lock`, vfs.md content-plane/access-migrate/`ReplicaKind::Cache`,
completion-router.md concerns 1–3 inventory feeds, service-registry NodeScoped,
store.md `store-access`), and INTENT #4/#34/#48/#59/#66/#85.

## Suggested fill-model

**implementation-ready + moderate complexity → mid model OK**, with two carve-outs
for a careful hand: (1) the **`jobs.rs` singleflight + shared-task join** (concern
2 — the one correctness spot: two concurrent `ensure`/`download` requests for the
same weight must converge on ONE `tokio::spawn`ed transfer, with progress readable
and the blocking + async callers both awaiting it; a naive impl double-downloads
40 GB or deadlocks the store mutex); (2) the **`.partial`-lock / `register_and_lock`
gc choreography** (concern 4 — the sweep-vs-download race, filled *after* the
Contract Harmonizer freezes gc's `GcCommand`/`register_and_lock` shape in `types`).
The `registry.rs`/`download.rs` byte-mover carry forward near-verbatim; the sha256
verification and the `GcHandle` field swap are mechanical. Sequence **after** gc,
store, and types are filled (models rides all three); the deferred `models-vfs`
leg fills only after vfs lands and the operator blesses it.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them.

- `model-ensure` (scheduler → models; models owns) — blocking
  `ensure_available` pre-swap, async `request_download`/`download_status` (the
  honest 202 path), and `evict`, all resolving through one singleflight job.
  → `scaffold/contracts/model-ensure.md`
  - Contract resolution: models' authored `ModelEnsure` trait won over
    scheduler's simpler `ensure_downloaded`/`EnsureState` sketch (owner wins);
    the `Peer` download source stays deferred pending `models-vfs`.
- `gc-managed-dirs` ({models, cache, engine} → gc; gc authors the vocabulary) —
  disk-budget enforcement for the weights dir via `GcHandle`
  (`register_dir(LruAccessed)`, `make_room` → write → `register_and_lock`,
  `touch` on use, `lock` while resident). → `scaffold/contracts/gc-managed-dirs.md`
- `store-access` (store ↔ {engine, scheduler, models, cache, telemetry,
  benchmark, api}; store authors) — models' slice of the SQLite
  system-of-record (`ModelRow` registry methods; download progress is NOT
  persisted, only terminal transitions). → `scaffold/contracts/store-access.md`
  - Component-side flag still open for store: optional additive
    `content_hash: Option<String>` column on `ModelRow` (verified sha256 —
    enables deferred peer-pull addressing + cross-restart integrity); not yet
    in the authored `store-access` surface.
- `models-vfs` (models → vfs) — NEW edge surfaced by concerns 1/6, **DEFERRED,
  not authored** as a contract file this round: peer-pull of a weight by
  content hash from a warm mesh peer as a plain vfs client
  (`Read { cache_local: true }`), falling through to origin download. Rides
  vfs's `vfs-content` wire and needs operator blessing on weights entering the
  `vfs://models/` namespace; `model-ensure.md` records the reserved `Peer`
  source for it.
