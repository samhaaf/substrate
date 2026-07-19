# Contract: engine-exec

## Parties
scheduler  ->  engine   (provider: **engine** — `engine.md` authors the trait shape;
consumer: **scheduler** — pins the behavioral call obligations)

Both are compiled-in internal libraries of the one `inference` daemon (INTENT
#22/#29). This is an **in-process Rust trait seam, NOT a wire contract** — there is
no serialization, no mesh relay, and the whole `inference` crate revs as one binary
(INTENT #45).

## Purpose
The scheduler's submit / drain / swap / cancel / recover surface over the execution
engine. The scheduler decides **what runs and when**; the engine **executes one
generation on one resident model** and manages the `llama-server` child process. The
edge carries: completion submission (with the per-completion token tee), the
drain-before-swap discipline, model swap, preemption/abort, crash-recovery id drain,
and the cheap interruptibility reads (`is_swapping` / `running_count` /
`resident_model`) that source the node's restart-protocol `Interruptibility`.

## Schema

The `EngineExec` trait, implemented by `ExecutionEngine`. Types reference
`types::{completion, model, stream, error}`.

```rust
trait EngineExec {                       // implemented by ExecutionEngine
    // --- execution ---
    // submit takes the modality-NEUTRAL request now (engine.md concern 5a): the
    // store CompletionRow, not the llama-specific CompletionPayload (which is now
    // LlamaBackend-private). token_tx is the caller's reliable, backpressured
    // per-completion stream sink; the engine's submit-interposed forwarder tees
    // each StreamEvent to (1) this token_tx and (2) the lossy InferenceEvent
    // broadcast bus (engine.md concern 4). The engine never learns of the bus at
    // the trait boundary — the tee lives inside ExecutionEngine::submit.
    async fn submit(&self, row: CompletionRow,
                    token_tx: mpsc::Sender<StreamEvent>) -> Result<()>;

    // --- model swap (swap does NOT drain — the scheduler's precondition below) ---
    async fn swap_model(&self, model_id: ModelId, model_path: &Path) -> Result<()>;
    async fn unload(&self) -> Result<()>;

    // --- drain / preempt / recover ---
    async fn drain(&self, timeout_secs: u64) -> Result<()>;   // L2 finish-and-relinquish
    async fn cancel_all_running(&self) -> Result<()>;         // L4 kill path
    async fn abort_slot(&self, id: CompletionId) -> Result<()>;
    fn take_running_ids(&self) -> Vec<CompletionId>;          // crash-recovery requeue

    // --- interruptibility inputs for inference's restart-protocol (engine.md concern 7) ---
    fn resident_model(&self) -> Option<ModelId>;
    fn running_count(&self) -> usize;
    fn is_swapping(&self) -> bool;   // guards the engine's INNER swap_model window
    async fn is_healthy(&self) -> bool;
}
```

### Scheduler-side call obligations (the behavioral half of the contract)

These are preconditions the **scheduler** owns; the engine assumes them.

1. **Drain-or-cancel before swap.** `swap_model` does NOT drain in-flight work
   (engine.md concern 1: replacing the backend process while completions run
   corrupts them). The scheduler MUST, in `execute_model_swap`, call `drain(timeout)`
   (normal path) or `cancel_all_running()` + requeue (hard-pressure path) **before**
   `swap_model`.
2. **The `is_swapping` bracket.** The scheduler sets its OWN `swapping` flag for the
   entire `execute_model_swap` window (drain → `swap_model` → load), so its
   `interruptibility()` reports `CriticalSection` across the whole swap, not merely
   engine's inner `is_swapping()` (which covers only the `swap_model` call). The two
   flags are additive: `scheduler.is_swapping() || engine.is_swapping()` bounds the
   critical window (scheduler.md concern 3).
3. **Preemption uses `abort_slot` / `cancel_all_running` + the discard/requeue
   split.** On preemption the scheduler keys on `preemption_threshold.is_some()`:
   background (benchmark, `Some(t)`) → `abort_slot` then `mark_cancelled` (discard —
   see `benchmark-collections`); foreground (`None`) → `abort_slot` then
   `requeue_with_circuit_breaker`.
4. **Crash-recovery drain.** `take_running_ids()` on boot feeds `recover_running`
   (the L4 backstop, inference.md concern 4): rows left `Running` from a crashed
   process are requeued to `Pending`.

## Error cases

Engine errors are the `EngineError` sub-enum (`types::error`), all **catchable**;
the engine never panics and the `inference` daemon never exits on them.

| Error | Raised by | Scheduler handling |
|---|---|---|
| `SlotsBusy { max }` | `submit` when all `max_concurrent` slots are taken | admission ceiling hit — the scheduler should not have admitted; treat as backpressure, retry next tick |
| `DrainTimeout { remaining }` | `drain` | requeue the `remaining` ids, proceed with the swap anyway (today's `lib.rs:444` behavior) |
| `CompletionNotFound` | `abort_slot` on an unknown/already-terminal id | benign — the slot already freed |
| `BackendUnavailableOffline { version, platform }` | `swap_model` when provisioning fails offline (engine.md concern 3) | **catchable** — hold the triggering completions `Pending` (do NOT fail them), retry provisioning opportunistically when connectivity returns |
| `SlotAction { slot, action, detail }` | KV save/restore (via `kv-cache`, out of this pair) | logged; a failed restore falls back to full prefill |

## Version sensitivity

**N/A on the wire** — in-process, single binary (INTENT #45). No struct here crosses
a node boundary through this seam. (The `StreamEvent`s teed to `token_tx` DO cross
the mesh via `api`'s WS relay / `inference-events`, but that additive discipline is
`v1-completion-api`'s / `inference-events`' concern, not this trait's.) The trait
signature is a source-level frozen surface; adding a method or an `EngineError`
variant is a same-binary recompile — additive-safe. The one **behavioral** invariant
that is breaking-if-violated: `submit` must be preceded by a drain-or-cancel before
any `swap_model` — a scheduler that swaps without draining corrupts in-flight
completions.

## Reconciliation notes

- **Provider vs consumer authorship (no conflict).** wave2-plan §3a and both design
  files agree: `engine.md` **owns the trait shape**; `scheduler.md` **consumes** it
  and pins the call obligations. The modality-neutral `submit(row, token_tx)` (engine
  concern 5a) replaces the wave-1 `run_completion(payload)`; the scheduler's proposal
  already transcribes the neutral form, so there was nothing to reconcile — merged
  directly.
- **The `is_swapping()` split (both sides agree; made explicit here).** engine.md
  adds `is_swapping()` as a NEW flag guarding its inner `swap_model` window;
  scheduler.md keeps its OWN `swapping` flag for the *whole* `execute_model_swap`.
  These are two nested windows, not a conflict, and `interruptibility()` ORs them.
  Recorded so the harmonizer does not collapse them into one flag — collapsing would
  shrink the critical window to just the engine call and expose the drain phase to
  interruption.
- **The token tee — where it lives.** engine.md concern 4 places the tee inside
  `ExecutionEngine::submit` (copy each `StreamEvent` to the reliable `token_tx` and
  the lossy `InferenceEvent` bus). scheduler.md/api.md need a per-completion
  `broadcast::Sender<StreamEvent>` registry (for `api.subscribe_tokens`, see
  `api-dispatch`) created at *submit* time. Reconciliation: the **scheduler** owns
  that per-completion broadcast registry (created in `admit_pending`, keyed by
  `CompletionId`); the `token_tx` it hands to `engine.submit` feeds that registry;
  the **engine** owns the tee from `token_tx` to the lossy observability bus. The two
  designs compose (scheduler owns the registry, engine owns the tee) — documented so
  neither filler re-creates the channel on the other side. No losing position.

## Example data

Example world: node **macbook** (an M-series inference node), project **demo**,
model **qwen3-4b**. The scheduler swaps macbook to `qwen3-4b` to serve a pending
completion `demo-c-1001`.

```rust
// 1. scheduler decides a swap is needed (top pending completion targets qwen3-4b,
//    not resident). It brackets the whole operation with its own swapping flag:
scheduler.swapping.store(true);   // interruptibility() now == CriticalSection

// 2. drain-before-swap obligation (#1):
engine.drain(/* timeout_secs */ 30).await?;   // Ok(()) — no in-flight work on idle macbook

// 3. ensure the weight is present (see model-ensure) then swap:
//    model-ensure returned Ready { path: "/Users/sam/.substrate/models/qwen3-4b.gguf" }
engine.swap_model(ModelId("qwen3-4b".into()),
                  Path::new("/Users/sam/.substrate/models/qwen3-4b.gguf")).await?;
//    during this call engine.is_swapping() == true; after it returns,
//    engine.resident_model() == Some(ModelId("qwen3-4b"))
scheduler.swapping.store(false);

// 4. admit the completion; the scheduler created a broadcast registry entry for
//    CompletionId("demo-c-1001") at submit time; token_tx feeds it:
let (token_tx, _rx) = mpsc::channel::<StreamEvent>(64);   // scheduler-owned; tees into registry
engine.submit(row /* CompletionRow{ id:"demo-c-1001", model_id:"qwen3-4b", .. } */,
              token_tx).await?;
//    engine.running_count() == 1 ; interruptibility() == Interruptible

// 5. StreamEvents observed on token_tx (the reliable client path) — also teed to
//    the lossy InferenceEvent bus:
//    StreamEvent::Started   { id: "demo-c-1001" }
//    StreamEvent::Token     { id: "demo-c-1001", index: 0, text: "Hello" }
//    StreamEvent::Token     { id: "demo-c-1001", index: 1, text: " world" }
//    StreamEvent::Completed { id: "demo-c-1001" }

// crash-recovery path on the next boot of macbook (obligation #4):
let orphaned = engine.take_running_ids();   // vec!["demo-c-1001"] if the process died mid-flight
// -> scheduler.recover_running() requeues them Pending
```

## Conformance requirement

A filler's `engine` + `scheduler` pass this contract iff, against the example world:
(a) a `swap_model` issued without a preceding `drain`/`cancel_all_running` is a test
**failure** (assert `execute_model_swap` drains first); (b) during
`execute_model_swap`, `scheduler.is_swapping()` reads `true` for the ENTIRE window
(drain through load), so `interruptibility()` reports `CriticalSection` even during
drain; (c) `submit` tees every `StreamEvent` to the provided `token_tx` in
`stream.rs` order (`Started` once → `Token*` → exactly one terminal); (d)
`BackendUnavailableOffline` from `swap_model` leaves the triggering completions
`Pending`, never `Failed`; (e) `take_running_ids()` on boot returns the ids of rows
the store still marks `Running`, which the scheduler requeues.
