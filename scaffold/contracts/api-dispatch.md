# Contract: api-dispatch

> **In-process seam — NOT a wire contract.** `api`, `scheduler`, `store`,
> `telemetry`, `benchmark` are all compiled-in libraries of the one `inference`
> daemon (INTENT #22/#29/#45). `api` holds the `Router` and dispatches into its
> siblings via `ApiState`-held handles; there is no serialization at this seam (the
> *wire* is `v1-completion-api` / `node-state-poll` / `inference-events`, which `api`
> serializes on the far side). Per the task's lighter treatment for compiled-in
> internal edges, this file documents the **dispatch trait surface + the two wave-2
> extensions** rather than a full example world.

## Parties
api  ->  {scheduler, store, telemetry, benchmark}
(consumer/dispatcher: **api**; providers: the four subsystems, each authoring its own
method surface — api adds NO business logic on top of dispatch)

## Purpose
The node-internal dispatch every `/v1` route makes. api is pure HTTP/WS translation:
a route is `extract → dispatch → serialize`. This edge enumerates the in-process
methods the Axum handlers call, including the two wave-2 additions that close
long-standing gaps: `scheduler.subscribe_tokens` (the token-streaming seam) and the
telemetry-kernel dispatch (replacing api's deleted local curve fit).

## Schema (dispatch surface)

Grounded in the live handlers + wave-2 additions. Types reference `types::{completion,
system, stream}`.

```rust
// --- scheduler (Arc<Scheduler>) ---
fn submit(&self, req: CompletionRequest) -> Result<CompletionId>;   // POST /v1/completions
async fn cancel(&self, id: CompletionId) -> Result<()>;             // DELETE /v1/completions/:id
fn is_queue_empty(&self) -> bool;
// WAVE-2 (api concern 2 — closes the ws.rs 100ms poll-loop gap):
fn subscribe_tokens(&self, id: CompletionId) -> Option<broadcast::Receiver<StreamEvent>>;
//   per-completion broadcast::Sender created at SUBMIT time (not admit — so a client
//   connecting immediately after POST cannot miss it), lives in a scheduler-owned
//   registry keyed by CompletionId, fed by the engine tee (see engine-exec), dropped
//   shortly after the terminal event. None = no live channel (already terminal or
//   never admitted) -> the WS handler falls back to a store replay (a normal branch).

// --- store (Store) ---
fn get/list_completions*(..) ; get/insert/cancel_collection(..) ; collection_member_counts(..);
fn get/list_models(..) ; get_result(..) ; update_priority(..) ; benchmark_runs_for_model(..);
//   (all via store-access; api never mutates the queue directly — submit goes through scheduler)

// --- telemetry (Arc<Telemetry>) ---
fn current_state(&self) -> SystemState;                             // GET /v1/system/state, /metrics
// WAVE-2 (api concern 7 — replaces the deleted fit_quadratic_tps in-handler fit):
fn kernel_surface(&self, model: &ModelId) -> KernelSurface;        // GET /v1/benchmark/kernel
fn estimate(&self, model: &ModelId, q: OperatingPoint) -> KernelEstimate; // POST /v1/estimate (WARM kernel)

// --- benchmark (BenchmarkOrchestrator) ---
fn run_now(&self, model_id: Option<ModelId>, target: Option<DialPoint>) -> Result<CollectionId>; // POST /v1/benchmark/run
fn status(&self) -> BenchmarkStatus;                               // GET /v1/benchmark/status
fn set_enabled(&self, enabled: bool);                             // dashboard toggle
```

## Error cases
All dispatch returns `Result<_, SubstrateError>`; api maps to HTTP via `err_response`
(`*NotFound` → 404; `*Terminal` → 409; `InvalidRequest`/`EmptyCollection` → 422; else
→ 500). `subscribe_tokens` returning `None` is a **normal branch**, not an error (the
WS handler replays terminal state from store). `run_now` while a batch is already in
flight returns a 409-shaped error (one batch per node, ever).

## Version sensitivity
**N/A** (compiled-in; the whole `inference` crate revs as one binary — INTENT #45).
The seam is a Rust trait/method surface, not a wire format. The structs that DO cross
the wire are serialized by api on the *far* side (`v1-completion-api`,
`node-state-poll`, `inference-events`, `surface-schema`) and follow guardrail-4
additive discipline there — not here.

## Reconciliation notes / open questions
- **`subscribe_tokens` producer side — resolved across engine/scheduler/api.** api
  authored the *subscriber* side (api concern 2) and flagged the producer as an open
  ask on engine/scheduler. scheduler.md (concern / api-dispatch note) resolved it: the
  scheduler owns the per-completion `broadcast::Sender` registry (created in
  `admit_pending`, where the wave-1 `token_tx` receiver is currently dropped), and the
  engine tee (see `engine-exec`) feeds it. **No disagreement** — the three files
  compose; recorded here so no filler creates a second token channel. The one pinned
  decision: the channel is created at **submit** time, not admit, to close the
  connect-before-admit race.
- **`benchmark` dispatch shape: `schedule_if_idle` (live/api) vs `run_now`+`status`
  (benchmark wave-2) — resolved to benchmark's.** api.md transcribed the live
  `schedule_if_idle(model_id, queue_empty)` shape; benchmark.md (concern 7/8)
  supersedes it with `run_now(model_id?, target?) -> CollectionId` + `status() ->
  BenchmarkStatus` + `set_enabled(bool)`, matching the redesigned active-learning
  loop (the fixed-grid `schedule_if_idle` is retired). **benchmark's shape wins** — it
  is the provider and the live shape belongs to the retired fixed-grid design.
  **Losing position recorded:** `schedule_if_idle` — superseded; api dispatches
  `run_now`/`status`/`set_enabled` instead.
- **`GET /v1/benchmark/kernel` dispatches to TELEMETRY, not benchmark.** api concern 7
  deletes the in-handler `fit_quadratic_tps` recompute; the route now serves
  telemetry's real multi-axis kernel via `kernel_surface`. benchmark.md agrees
  explicitly ("`/v1/benchmark/kernel` does NOT dispatch here — it is telemetry's
  kernel"). No conflict; recorded because the route name misleadingly implies
  benchmark ownership.
- **`estimate` warm-kernel fix.** `POST /v1/estimate` today news up a fresh cold
  `ThroughputEstimator` per call; wave-2 dispatches to telemetry's warm shared kernel
  (`estimate(model, q)`), fixing the cold-start bug. Both api and telemetry agree;
  adopted.
- **Conformance carry-over:** api adds NO logic on top of dispatch — a route is
  extract → dispatch → serialize. The wave-2 removals ARE conformance items: **no
  curve fit** (`fit_quadratic_tps` deleted) and **no poll loop** (the 100ms
  `POLL_INTERVAL` in `ws.rs` deleted) may remain in api.

## Conformance requirement
A filler's `api` passes iff: (a) the `/v1/completions/:id/stream` handler subscribes
via `scheduler.subscribe_tokens` and forwards real `Token` frames — the 100ms poll
loop is gone; `None` falls back to a store replay; (b) `GET /v1/benchmark/kernel`
serializes `telemetry.kernel_surface(model)` with no local curve fit in the handler;
(c) `POST /v1/benchmark/run` dispatches `benchmark.run_now` and returns a 409-shaped
error when a batch is already in flight; (d) every route is extract → dispatch →
serialize with no business logic added at the api layer.
