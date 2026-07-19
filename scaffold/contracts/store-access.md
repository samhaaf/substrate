# Contract: store-access

> **In-process seam — NOT a wire contract.** `store` is a compiled-in library of the
> `inference` daemon (INTENT #22/#29). Every party reaches it by holding a cloned
> `Store` handle (`#[derive(Clone)]`, cheap `Arc`); there is no network, no relay, no
> serialization. Per the task's lighter treatment for compiled-in internal edges,
> this file documents the **trait surface + the reentrancy contract** rather than a
> full example world.

## Parties
store  <->  {engine, scheduler, models, cache, telemetry, benchmark, api}  +  the
`inference` composition root's `StoreObserver` (the `PromiseRegistry`; the KG service
is the named future second implementer)

Owner/author: **store** (`store.md`). Every sibling is a consumer; each authored a
participation note against this surface.

## Purpose
The single in-process access surface to the per-node SQLite system of record:
completion CRUD + the priority-ordered queue view, collection lifecycle, model
registry, result blobs, benchmark runs (with wave-2 pressure covariates), kv-cache
metadata, and the `StoreObserver` notification seam. It is the intra-node hub nearly
every inference sibling reads/writes through. Deliberately distinct from `db` (the
control-plane action-runner) and `vdb` (the stack daemon) — store owns per-node
inference persistence and the observer seam only.

## Schema (trait surface)

Method families on `Store`, real signatures + wave-2 deltas. Types reference
`types::{completion, collection, model, provenance, error}`.

```rust
// completions (completions.rs) — WAVE-2: insert takes provenance (store concern 4)
fn insert_completion(&self, row: &CompletionRow, prov: &Provenance) -> Result<()>;
fn get_completion(&self, id: CompletionId) -> Result<CompletionRow>;
fn mark_running/completed/failed/cancelled(&self, id: CompletionId, ..) -> Result<()>;
fn requeue_preempted/requeue_error_retry(&self, id: CompletionId) -> Result<()>;
fn select_pending(&self) -> Result<Vec<CompletionRow>>;             // scheduler tick, (priority DESC, created_at ASC)
fn select_pending_for_model(&self, m: &ModelId) -> Result<Vec<CompletionRow>>;
fn count_by_state(&self) -> Result<Vec<(CompletionState,u64)>>;     // system-state counts + benchmark idle gate
fn list_completions_by_state(&self, states: &[CompletionState], limit: u32) -> ..; // caps at 1000
fn recover_running(&self) -> Result<u64>;                          // crash recovery — running -> pending, stamps recovered_at

// results (results.rs)
fn insert_result(&self, r: &CompletionResult) -> Result<()>;       // engine terminal write
fn get_result(&self, id: CompletionId) -> Result<Option<CompletionResult>>; // observer must DEFER this

// models / collections / kv_cache — registry + lifecycle + metadata (unchanged)
fn get_model / list_models / upsert_model / set_model_downloaded / clear_model_file
   / touch_model / total_weights_bytes(..) -> ..;                  // models' slice

// benchmarks (benchmarks.rs) — WAVE-2: BenchmarkRun + insert/map grow pressure covariates
fn insert_benchmark_run(&self, run: &BenchmarkRun) -> Result<i64>;
   // run carries: prompt_tokens, max_tokens, concurrency, tokens_per_second, wall_time_ms,
   //   + mem_pressure, cpu_utilization, gpu_utilization, gpu_mem_used_bytes,
   //   + is_gpu_estimate (honesty flag, INTENT #9), sample_source ('benchmark'|'observed'),
   //   + collection_id (benchmark's additive ask — batch auditability)
fn benchmark_runs_for_model(&self, m: &ModelId) -> Result<Vec<BenchmarkRun>>;

// observer seam (lib.rs) — WAVE-2: methods carry &Provenance (additive)
trait StoreObserver: Send + Sync {
    fn on_completion_inserted(&self, id: CompletionId, prov: &Provenance) {}
    fn on_completion_state_changed(&self, id: CompletionId, old: CompletionState, new: CompletionState) {}
    fn on_model_registered(&self, id: &ModelId) {}
    fn on_benchmark_recorded(&self, model: &ModelId, run_id: i64) {}
}
fn with_observer(self, obs: Arc<dyn StoreObserver>) -> Self;
```

## The reentrancy contract (the teeth — store concern 2)

The whole concurrency model is a single `Arc<Mutex<Connection>>` (`lib.rs:92`); every
mutation and query serializes behind it. **Observer callbacks fire synchronously on
the writer thread with the store `Mutex` HELD.** An implementer MUST:

1. **Never re-enter `Store`.** Any method that calls `conn()` deadlocks (the
   `std::sync::Mutex` is non-reentrant). Defer any store read (e.g. `get_result`) to
   `tokio::spawn`, which runs *after* the guard drops.
2. **Never block or `.await`** inside the callback (it stalls the single writer and
   therefore *all* node persistence). Touch only in-memory state.
3. **`try_lock` its own state and skip on contention** — delivery is **best-effort by
   design**, not guaranteed.
4. **Never panic** — a panic unwinds through the held mutex, poisons it, and bricks
   the node's store.

Additionally: **no caller may hold the `MutexGuard` across an `.await`** — every store
method takes-and-drops within one synchronous body.

## Error cases
`StoreError::{Sqlite, Poisoned, CompletionNotFound, Json}` (adopting
`types::error::store`, store concern 6; today surfaced as `SubstrateError::
Store(String)` / `CompletionNotFound(id)` / `Internal(json)`). Queries cap at 1000
rows to protect callers.

## Version sensitivity
**In-process, single-build → no wire skew** (no cross-node `store` deserialization).
The only compatibility axis is **on-disk schema forward-safety**: the append-only
`schema.sql` convention adds new columns at the bottom (`IF NOT EXISTS`/nullable), so
an older binary reading a newer file ignores unknown columns and a newer binary reads
old rows with `NULL` covariates (backward-safe). The `StoreObserver` `&Provenance`
addition is a source-level additive change (one in-tree implementer today).

## Reconciliation notes / open questions
- **No cross-party disagreement.** store.md is the sole author; the six sibling
  design files (scheduler, engine, models, telemetry, benchmark, api) each filed a
  *participation note* that consumes this surface as proposed. The consumer notes are
  consistent: scheduler uses the priority-ordered pending views + transitions + never
  registers an observer; engine writes only terminal transitions (`insert_result` +
  `mark_completed`/`mark_failed`) from the tee-forwarder; telemetry reads
  `benchmark_runs` and writes confidence-gated `'observed'` rows; benchmark writes
  `'benchmark'` rows for accepted batches only. Merged as-is.
- **Provenance threading (store concern 4, cross-file with scheduler/api).**
  `insert_completion`'s new `&Provenance` arg MUST be plumbed from the `api` request
  envelope through the scheduler's `submit`, **not defaulted at the store boundary** (a
  defaulted provenance is a silent INTENT #85 violation). scheduler.md's `store-access`
  note explicitly accepts threading it through; api stamps it from the inbound `/v1`
  envelope. Recorded as the reconciled ownership chain: api (origin) → scheduler
  (thread) → store (persist).
- **Open (flagged to store/gc, not resolved here):** telemetry's confidence-gated
  `'observed'` rows need a **retention cap / gc-managed TTL** so `benchmark_runs`
  does not grow unbounded (the confidence gate bounds the *rate*; retention bounds the
  *total*). Deferred to a store/gc follow-up.
- **Open (flagged to store):** benchmark's `collection_id` column and the (mean, peak)
  covariate-pair ask — store may accept single-column mean-only with peak riding
  `metadata_json`; benchmark accepts either. Store's migration call.

## Conformance requirement
A filler's `store` (and any `StoreObserver` implementer) passes iff: (a) an observer
that reads the store does so via deferred `tokio::spawn`, never re-entrant — proven by
a **deadlock-freedom test** (register an observer whose callback attempts a store read;
assert the writing call returns without hanging); (b) `insert_completion` persists the
passed `Provenance` and `get_completion` round-trips it; (c) a `benchmark_runs` row
with pressure covariates round-trips, and an old-schema row (no covariate columns)
reads back with `NULL`/`None` covariates; (d) `recover_running` requeues
`running`→`pending` and stamps `recovered_at`.
