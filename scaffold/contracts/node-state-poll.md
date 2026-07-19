# Contract: node-state-poll

## Parties
mesh.completion-router (NodeRegistry)  ->  inference (api)

- **inference (api)** is the **terminus** — it serves the read endpoints from
  telemetry's snapshot + the store's model inventory (api.md, telemetry.md).
- **mesh.completion-router** is the **poller** — it corrects drift in its
  `NodeRegistry` projection off the request path (completion-router.md concern 3).

## Purpose
The **reconcile / bootstrap** health + inventory read: two cheap, read-only GETs
the router polls to correct drift in its live projection of each node
(demoted from wave-1's live-primary role — live load now rides `inference-events`;
this poll is the loss-repair and cold-start path). **Never on the request path.**
`GET /v1/system/state` gives running/pending counts, resource pressure, resident
model, and the effective-concurrency scalar; `GET /v1/models` gives the per-node
downloaded/resident model inventory. These feed the registry's health + affinity
(least-loaded, model-affinity) decisions.

## Schema

Reads derive from the shared `types` vocabulary (`types::system::SystemState`,
`types::node::{NodeInfo, NodeCapabilities, NodeRole, Accelerator}`,
`types::model::ModelId`). The router projects them into its internal poll shapes:

```rust
// GET /v1/system/state  ->  200 types::system::SystemState  (serialized as-is)
struct SystemState {
    sampled_at: DateTime<Utc>,
    total_memory_bytes: u64, used_memory_bytes: u64, memory_pressure: f32,
    cpu_utilization: f32,
    gpu_memory_used_bytes:  Option<u64>,   // WAVE-2: was u64=0; None = not measured
    gpu_memory_total_bytes: Option<u64>,   // WAVE-2: None on unified memory (Apple Silicon)
    gpu_utilization: f32,
    is_gpu_estimate: bool,                 // true = GPU covariates are estimated (INTENT #9)
    resident_model: Option<ModelId>,
    running_count: usize, pending_count: usize,
    effective_max_concurrent: Option<u32>, // WAVE-2 (telemetry concern 6): kernel-computed
                                            // at the current operating point; None below
                                            // confidence floor (scheduler/router degrade)
    weights_on_disk_bytes: u64, kv_cache_bytes: u64,
}

// GET /v1/models  ->  200 [ ModelRow ]  (router projects to inventory)
struct ModelRow { id: ModelId, status: String, is_downloaded: bool, is_loaded: bool /* = resident */ }
// router's projection:
struct ModelInventory { node: NodeId, downloaded: Vec<ModelId>, resident: Option<ModelId> }

// GET /health  ->  200 { status: "ok", version }
```

The router's internal poll projection of the state read:

```rust
struct SystemStatePoll {
    node: NodeId, running: u32, pending: u32,
    memory_pressure: f32, resident_model: Option<ModelId>,
    effective_max_concurrent: Option<u32>,  // Tier-3 / spill headroom input (now populated)
}
```

## Error cases

- `NodeUnreachable` / poll timeout → the router marks that node's projection
  **`stale`**. A **single** miss is tolerated (the event feed is likely still
  fresh). After **N consecutive misses** (default 3) **and** no events, the router
  **evicts** the node from the candidate set (never a phantom least-loaded
  target). `NodeUnreachable` is catchable — never panics.
- A poll **during a model swap** returns a consistent snapshot (telemetry's single
  `RwLock` `current_state`), never a torn read.
- api serves these reads from cached telemetry state; they **never** mutate and
  **never** touch the scheduler submit path (api.md wave-1 concern 2).
- `current_state()` always returns the last good snapshot; a poisoned sampler
  mutex degrades to a zeroed snapshot, never a panic.

## Version sensitivity

- **MEDIUM.** `SystemState` / `NodeInfo` / `NodeCapabilities` cross the mesh
  serialized. All new fields are `#[serde(default)]` (additive) and enums reserve
  `#[serde(other)]`, so a **newer node's richer state never breaks an older
  router** mid-rollout (INTENT #66). `SystemState.effective_max_concurrent`,
  `gpu_memory_used_bytes`, and `gpu_memory_total_bytes` are the wave-2 additive
  fields.
- **Additive-safe:** new `Option`/`#[serde(default)]` state fields, new
  `NodeRole`/`AcceleratorKind` variants (behind `#[serde(other)]`).
- **Breaking:** removing/retyping a field the router reads (`running_count`,
  `pending_count`, `memory_pressure`, `resident_model`), or changing the
  `is_downloaded` / `is_loaded` semantics of `GET /v1/models`.
- **Conformance:** `GET /v1/system/state` MUST expose `resident_model` +
  running/pending + `memory_pressure` (affinity/least-loaded inputs);
  `GET /v1/models` MUST distinguish `is_downloaded` from `is_loaded`
  (Tier-1/Tier-2 affinity input). GPU pressure is an honest estimate on Apple
  Silicon (INTENT #9) — the poll carries the raw value + `is_gpu_estimate`; the
  tilde/tooltip honesty marker lives on the `surface-schema`, not this poll.

## Reconciliation notes

1. **`effective_max_concurrent` — RESOLVED: INCLUDE it (telemetry wins).**
   *Disputed:* whether `SystemState` carries an effective-concurrency scalar over
   this poll.
   - **completion-router's position (superseded):** its `SystemStatePoll` sketch
     left `effective_max_concurrent` **commented out / "still deferred."**
   - **telemetry's position (WON, and authoritative):** telemetry.md concern 6
     **adds** `effective_max_concurrent: Option<u32>` to `types::system::
     SystemState`, stamped each tick from the multidimensional kernel at the
     current operating point.
   - **Why include it:** telemetry **owns** `SystemState` and was explicitly
     tasked (INTENT #12; wave2-plan telemetry charter) with producing this scalar;
     the field is `Option` and additive, so carrying it costs the router nothing
     when absent. It gives the router a zero-extra-call spill/headroom input
     (Tier-3 selection) it otherwise lacks. Since telemetry populates the field
     regardless, "deferring" it in the poll would only discard information already
     on the wire.
   - **Semantics pinned:** `None` when the kernel's confidence at that operating
     point is below floor → the router (and scheduler) fall back to the scalar
     `memory_pressure` heuristic (graceful degradation). The authoritative,
     confidence-bearing, what-if form is telemetry's in-process
     `kernel-confidence` (`effective_concurrency(model, at)`); the field here is
     the **best-effort convenience read** for the cross-node poll. This matches
     telemetry's own "offers both, pins the scalar as best-effort/optional"
     reconciliation flag against scheduler.md/system-state.md.

2. **Poll demoted to reconcile/bootstrap (agreed).** Both sides agree live load
   now rides `inference-events` (the loss-tolerant `queue.depth_changed`
   snapshot); this poll repairs drift and cold-starts the projection. No conflict.

3. **Shared `NodeInfo`/`NodeCapabilities` shape (OQ-3, RESOLVED upstream in
   types).** `types::node` gives one `NodeInfo`/`NodeCapabilities` both parties
   read; resident models are **not** duplicated into `NodeCapabilities` — they are
   live state read from `SystemState.resident_model` (single source of truth,
   types.md controversial-decision #3). The router's `ModelInventory.resident`
   therefore derives from the state read, not the capabilities read.

## Example data

**World:** nodes `macbook` and `pi`; model `qwen3-4b`; project `demo`. The router
on `macbook` polls `pi` to reconcile its projection after a `Lagged` notice on the
event feed. `pi` has `qwen3-4b` resident and one completion running (the
`v1-completion-api` example).

```jsonc
// GET http://pi/v1/system/state  (relayed through mesh; served from pi loopback)  -> 200
{
  "sampled_at": "2026-07-19T17:00:00Z",
  "total_memory_bytes": 8589934592,
  "used_memory_bytes": 6012954214,
  "memory_pressure": 0.70,
  "cpu_utilization": 0.55,
  "gpu_memory_used_bytes": null,        // Pi / unified memory: not independently measured
  "gpu_memory_total_bytes": null,
  "gpu_utilization": 0.0,
  "is_gpu_estimate": true,              // honest-estimate flag (INTENT #9)
  "resident_model": "qwen3-4b",
  "running_count": 1,
  "pending_count": 0,
  "effective_max_concurrent": 2,        // kernel says pi can run ~2 qwen3-4b at this pressure
  "weights_on_disk_bytes": 2483027968,
  "kv_cache_bytes": 134217728
}
```

```jsonc
// GET http://pi/v1/models  -> 200
[
  { "id": "qwen3-4b", "status": "ready", "is_downloaded": true, "is_loaded": true }
]
// router projects -> ModelInventory { node: "pi", downloaded: ["qwen3-4b"], resident: Some("qwen3-4b") }
```

```jsonc
// GET http://macbook/v1/system/state  (the local node, for contrast)  -> 200
{
  "sampled_at": "2026-07-19T17:00:00Z",
  "total_memory_bytes": 17179869184,
  "used_memory_bytes": 15461882368,
  "memory_pressure": 0.90,              // busy -> router preferred pi for the submit
  "cpu_utilization": 0.80,
  "gpu_memory_used_bytes": null,
  "gpu_memory_total_bytes": null,
  "gpu_utilization": 0.0,
  "is_gpu_estimate": true,
  "resident_model": null,               // macbook has no qwen3-4b resident
  "running_count": 0,
  "pending_count": 0,
  "effective_max_concurrent": null,     // below confidence floor -> router uses memory_pressure
  "weights_on_disk_bytes": 0,
  "kv_cache_bytes": 0
}
```

```jsonc
// Error example: pi unreachable on the poll -> router marks projection stale
// (no body; transport-level). 3rd consecutive miss with no events -> evict pi from candidates.
```
