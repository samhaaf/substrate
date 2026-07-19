# scheduler

**Status:** WAVE-2 REFIT of the wave-1 `scheduler.md` (approach-sketched). The
carried-forward core — the tick loop (`Scheduler::run`/`tick`), the
`AdmissionController` two-lever design, `execute_model_swap` with drain/requeue,
the `requeue_with_circuit_breaker`, crash-recovery requeue, and the three
pluggable trait seams (`SwapEvaluator`, `SelectionPolicy`) — is **already built
and correct** (real code in `lib/scheduler/src/{lib,admission,swap,selection,
queue}.rs`) and is preserved. Wave-2 makes concrete the four things the wave-1
file left sketched or deferred, all grounded in the batch-5/6 neighbor designs
that now exist: (1) **kernel-driven admission** — the `AdmissionController` target
comes from telemetry's fitted `effective_max_concurrent` at the current operating
point, with today's scalar memory-pressure heuristic demoted to a cold-start
fallback (consumes `kernel-confidence` + the extended `system-state`); (2) the
**interruptibility transitions** the scheduler is the source-of-truth for, feeding
inference's `restart-protocol` participation (`Idle` / `Interruptible` /
`CriticalSection`); (3) **benchmark priority-0 exclusivity** made a concrete,
*generalized declarative preemption-threshold* admission gate (not a
benchmark-special-case); and (4) the **`DebouncedSwapEvaluator`** anti-thrash stub
DESIGNED to implementation depth and the **`LocalitySelection`** stub's seam
designed with its deep coupling explicitly re-deferred. Grounded in the REAL
source and neighbors `telemetry.md`/`benchmark.md` (the adaptive kernel triangle
co-designed in this batch), `engine.md` (the `engine-exec` provider side +
`is_swapping()`), `store.md` (`store-access`, priority-ordered pending views),
`inference.md` (the interruptibility feed + 4-level ladder mapping this scheduler
sources), `api.md` (`api-dispatch` + `subscribe_tokens`), `cache.md` (the prefix
index `LocalitySelection` would query), and `supervision.md` (the ladder daemon
side). INTENT #12/#22/#29/#38/#77/#85.

## Charter

`scheduler` is the **central admission / swap / preemption / recovery loop** of
one `inference` node (`lib/scheduler`, modules `queue`, `admission`, `swap`,
`selection`): each tick it reads telemetry's `SystemState`, classifies resource
pressure, decides whether to swap the resident model (draining through
`engine-exec`), and admits pending work up to a **dynamically-computed concurrency
target** — now the kernel's `effective_max_concurrent` for the current operating
point, not a scalar. It owns the priority queue *policy* (a thin view over
`store`'s pending rows — the store is the single source of truth, so the queue
survives crashes with no separate recovery), the **declarative preemption
threshold** that gives benchmark its priority-0 exclusivity, and it is the
source-of-truth for the node's **interruptibility state**. Its boundary: it
decides **WHAT runs and WHEN on this node**; it does **not** execute generation
(`engine`, over `engine-exec`), does **not** sample the machine or fit the kernel
(`telemetry`, over `system-state`/`kernel-confidence`), does **not** persist
(`store`, over `store-access`), does **not** decide which measurements to take
(`benchmark`, which submits collections *through* the scheduler over
`benchmark-collections`), and does **not** speak mesh — it is a compiled-in
**internal library of `inference`** (INTENT #22/#29), reached in-process by `api`
(`api-dispatch`) and read by the composition root for the interruptibility feed;
`api`/`mesh-client` own the wire, registration, and restart callback. It
orchestrates its four siblings; it holds no HTTP, no SQLite schema, no kernel
math, and no mesh vocabulary.

## Primary design concerns

The wave-1 mechanics (the tick loop, hard/soft pressure gates, drain-before-swap,
the circuit breaker, crash-recovery requeue) are **preserved and re-affirmed** and
not re-argued. The concerns below are the wave-2 deltas; each says where it
supersedes wave-1.

### 1. Kernel-driven admission — `effective_max_concurrent` replaces the scalar target (the crate's hardest logic)

Today `AdmissionController::slots_to_admit` (real code, `admission.rs:90`) computes
its concurrency target from a **scalar** `memory_pressure`: at/above `hard_pct`
admit 0; between `soft_pct` and `hard_pct` reduce the fixed `max_concurrent`
*proportionally*; below `soft_pct` use the full `max_concurrent`. INTENT #12's
multidimensional kernel makes this honest: the number of completions a node can
sustain before per-completion throughput collapses (or before OOM risk) is a
function of the **whole operating point** — output-length mix × parallel-completion
count × observed memory/CPU/GPU pressure — not memory alone. So the target must
come from **telemetry's fitted kernel**, published as `effective_max_concurrent`
on `SystemState`, and the scheduler becomes a **consumer** of the kernel, not a
re-implementer of a pressure heuristic. The concrete design, expressed as a
strict layering that never loses the safe cold-start path:

```rust
// AdmissionController::slots_to_admit(sys, running) — wave-2 shape:
fn slots_to_admit(&self, sys: &SystemState, running: usize) -> u32 {
    // (a) Hard-pressure SAFETY FLOOR is unchanged and always wins — a fitted
    //     kernel never overrides the OOM guard (INTENT #38: no clever debt here).
    if sys.memory_pressure >= self.hard_pct { return 0; }

    // (b) Kernel-primary: telemetry's fitted effective_max_concurrent at the
    //     current operating point, bounded ABOVE by the engine's physical slot
    //     count (max_concurrent is fixed at process launch — engine.md; the
    //     kernel can only LOWER effective concurrency, never invent slots).
    let target = match sys.effective_max_concurrent {
        Some(k) => k.min(self.max_concurrent),
        // (c) Scalar FALLBACK when the kernel has no confident answer at this
        //     point (fresh node, benchmark hasn't fit this region yet →
        //     telemetry returns None). This is EXACTLY today's proportional
        //     memory heuristic, kept verbatim as the honest degradation path,
        //     not deleted — a cold node must still admit safely.
        None => if sys.memory_pressure >= self.soft_pct {
            let range = self.hard_pct - self.soft_pct;
            let excess = sys.memory_pressure - self.soft_pct;
            (self.max_concurrent as f32 * (1.0 - excess / range)).round() as u32
        } else {
            self.max_concurrent
        },
    };
    target.saturating_sub(running as u32)
}
```

Three load-bearing decisions live here, and they are the reason this concern is
the crate's hard core:

- **Kernel LOWERS, config CEILINGS.** `effective_max_concurrent` is clamped by
  `self.max_concurrent` (the engine's fixed slot count) so a beefy-machine kernel
  estimate can never exceed physical slots (resizing slots drains the KV cache —
  engine.md concern 1), and a pressured kernel estimate lowers concurrency below
  the ceiling. This is precisely the charter's "AdmissionController lowering
  effective concurrency under memory pressure," now sourced from the fitted
  surface instead of a linear ramp.
- **Cold-start fallback is not debt.** `effective_max_concurrent: None` means
  "the kernel is not confident at this operating point" (telemetry's confidence
  surface below threshold — the same signal `benchmark` reads over
  `kernel-confidence`). Falling back to the scalar memory heuristic is the honest
  boring answer; it is *removed* the instant the kernel is fit, per-region, with
  no code change (telemetry just starts returning `Some`). This resolves wave-1
  OQ-2 (`effective_max_concurrent` stubbed/absent blocked spill).
- **The operating point is telemetry's to sample, not the scheduler's to pass.**
  To keep the seam boring, telemetry computes `effective_max_concurrent` each
  sample from the *observed* pressure axes + the resident model's kernel surface
  at the currently-running parallelism, and stamps it on `SystemState`. The
  scheduler reads a scalar; it does **not** hand telemetry a hypothetical
  operating point on the hot path. (An **optional, approach-sketched** extension —
  a synchronous `kernel-confidence` query evaluating whether admitting *one more*
  completion crosses the throughput knee — is left for the fill; the default and
  the correctness path is the `SystemState` scalar.)

### 2. Benchmark priority-0 exclusivity — a GENERALIZED declarative preemption threshold, not a benchmark hack

Benchmark measurement isolation is a **scheduling guarantee**, not something
benchmark can enforce from its side (benchmark.md concern: "measurement isolation
is a scheduling guarantee"). The real benchmark code already encodes its intent
declaratively: sweep completions are `priority: 0`, `preemption_threshold:
Some(1)`, inside a `request_full_system: true` collection (`suite.rs:99-101`,
`lib.rs:190`). The wave-1 scheduler **does not read any of these** — this is the
concrete wave-2 gap I close, and I close it *generally* so it is boring and reusable
rather than a benchmark special-case:

**The rule (generalized preemption threshold).** A completion may carry
`preemption_threshold: Option<i32>` (already on `CompletionRow`, already computed
by `req.effective_preemption_threshold()` at submit, `lib.rs:227`). Semantics,
owned by the scheduler:

1. **Admission gate.** A pending completion with `preemption_threshold == Some(t)`
   is admitted **only when no pending or running completion has `priority >= t`.**
   For benchmark (`t = 1`): admit a benchmark completion only when the node holds
   no priority≥1 work anywhere — i.e. only when otherwise idle. Non-benchmark
   work has `preemption_threshold == None` and is never held by this gate.
2. **Instant preemption.** The moment a completion with `priority >= t` becomes
   pending (a `submit` wakes the loop via `notify_new_work`), the scheduler
   **immediately preempts every running completion whose `preemption_threshold ==
   Some(t')` with `priority >= t' present`** — for benchmark, any priority≥1
   arrival preempts all running priority-0 sweep completions the same tick. This
   is the "preempted instantly by any priority≥1 work" guarantee, driven by the
   existing `cancel_all_running`/`abort_slot` + requeue path (`lib.rs:437`).
3. **A preempted benchmark sample is DISCARDED, never recorded.** A benchmark
   completion cut mid-flight ran under contention or was interrupted — recording
   it would poison the kernel (benchmark.md: pressure is an *observed covariate*,
   and a preempted sample's covariates are meaningless). So on preemption the
   scheduler marks the benchmark completion `Cancelled` and the sweep re-queries
   it in a future idle window (benchmark's adaptive loop handles the re-plan).
   Real (priority≥1) preempted work goes through `requeue_with_circuit_breaker`
   as today — it is requeued, not discarded. **The discard-vs-requeue split keys
   on `preemption_threshold.is_some()`** (background work is discarded; foreground
   work is requeued): a clean, single predicate.
4. **`request_full_system` is the collection-level echo, not a second mechanism.**
   The flag on the benchmark collection asserts "these completions need the node
   otherwise idle" — which the per-completion `preemption_threshold: Some(1)` gate
   already enforces. The scheduler keys admission on the per-completion threshold;
   `request_full_system` stays a self-documenting collection attribute (and the
   handle by which the dashboard/operator can see a sweep is exclusive). No
   divergent logic.
5. **Benchmark admits to PHYSICAL slots, bypassing the kernel target (concern 1).**
   Under benchmarking mode, `slots_to_admit` uses the physical `max_concurrent`
   ceiling, **not** `effective_max_concurrent` — because the benchmark is the thing
   *generating* the kernel's training data across parallelism levels; clamping
   benchmark admission by the kernel would be **circular** (the kernel limiting its
   own inputs). The hard-pressure safety floor still applies (a benchmark that
   somehow drove memory to hard-pressure is still gated). Since benchmarking runs
   only when otherwise idle (low pressure by construction), this bypass is safe.
   *This circularity break is a non-obvious correctness point (Controversial
   decisions).*

This makes exclusivity a first-class, declarative property of a completion
(matching INTENT's declarative-triggers spirit), with benchmark as the one v1
user of `preemption_threshold`.

### 3. Interruptibility transitions — the scheduler is the source-of-truth (I own the transitions)

inference.md (batch 5) defined the vocabulary (`types::restart::Interruptibility`
= `Idle` / `Interruptible` / `CriticalSection{until}`) and owns the *feed* to
`mesh-client`; `supervision` owns the daemon-side ladder. **The transition logic
— when the node is in each state — is the scheduler's**, because the scheduler is
the only place that knows the live admission/queue/swap state. I expose one pure
read the composition root forwards on change:

```rust
// Scheduler::interruptibility() -> Interruptibility  (pure fn of live state)
fn interruptibility(&self) -> Interruptibility {
    if self.is_swapping() || self.engine.is_swapping() || self.is_kv_saving() {
        // Model swap or KV save/restore mid-flight: interrupting corrupts
        // engine↔store↔disk state (engine.md concern 7). ETA = best-effort.
        Interruptibility::CriticalSection { until: self.critical_eta() }
    } else if self.foreground_running() > 0 {
        // priority>=1 completions in flight: restartable at a cost — in-flight
        // Running rows requeue via recover_running() on the new process, streams
        // are cut (completion-router no mid-stream failover). A real reason, not free.
        Interruptibility::Interruptible
    } else {
        // Empty OR benchmark-only: Idle. Benchmark (priority-0) work does NOT
        // raise interruptibility — it is the lowest-value, fully-preemptible,
        // re-runnable work; sacrificing it to a routine update is CORRECT.
        Interruptibility::Idle
    }
}
```

The **deliberate deviation from inference.md's concern-4 table** (which listed "a
benchmark priority-0 sweep is running" as `CriticalSection`): I report a benchmark
sweep as **`Idle`, not `CriticalSection`.** Reasoning — treating benchmark as
critical would let the *lowest-value* work block a routine (`WaitForIdle`) update
during the *exact idle window* an update wants, inverting priority. A benchmark
sweep interrupted by a restart loses only its in-flight samples (cheap, re-planned
next idle window — concern 2.3); nothing is corrupted. Only genuine engine-internal
atomic operations (model swap, KV save/restore) are `CriticalSection`, because
interrupting them orphans a half-spawned server or leaves store/disk inconsistent.
This is consistent with measurement isolation (concern 2): benchmark yields both
to scheduled priority≥1 work *and* to restarts; it is the universal yielder.
**Flagged as a friction point** to reconcile with inference.md in the per-pair
round — I believe my transition table is the correct one and inference.md should
adopt it; the change is small (one row) and inference.md explicitly delegated the
transitions to this module.

`foreground_running()` is cheap: because exclusivity (concern 2) makes benchmark
and real work **mutually exclusive on the engine**, the scheduler tracks a single
`mode: Idle | Foreground | Benchmarking` set at admission time, so
`foreground_running() == engine.running_count()` when `mode == Foreground` and `0`
otherwise — no per-row priority scan on the feed path. The composition root reads
`interruptibility()` whenever `mode`, `running_count`, or the swap/save flags
change (event-driven, off the hot path) and forwards it as an
`InterruptibilityUpdate` frame (inference.md concern 4). The scheduler owns the
`swapping`/`kv_saving` flags: `swapping` is set for the duration of
`execute_model_swap`; `kv_saving` is set by the composition root's L3 restart
callback while it drives engine KV-save through the `kv-cache` seam (engine.md
concern 6), so the scheduler reflects it in the very state that L3 is trying to
protect.

### 4. The two policy stubs — `DebouncedSwapEvaluator` DESIGNED, `LocalitySelection` seam-defined + deep-part re-deferred

Both trait seams (`SwapEvaluator`, `SelectionPolicy`) are correct and preserved;
the two *stub implementations* are the wave-2 ask.

**(a) `DebouncedSwapEvaluator` — designed to implementation depth.** Today it
delegates straight to `inner` with a TODO (`swap.rs:166-172`). The anti-thrash
hazard is real: `DefaultSwapEvaluator` recommends a swap whenever the top pending
completion targets a non-resident model (`swap.rs:66-72`), so two models competing
at similar priority oscillate A→B→A→B, each swap costing a full drain + weight
load. The design:

```rust
impl SwapEvaluator for DebouncedSwapEvaluator {
    fn should_swap(&self, resident: Option<&ModelId>, cands: &[CompletionRow]) -> bool {
        if !self.inner.should_swap(resident, cands) { return false; }
        // Never debounce the initial load: no resident model → swap immediately.
        if resident.is_none() { return true; }
        // Priority override: a high-priority arrival must not be blocked by
        // anti-thrash. If the top candidate's priority exceeds the resident
        // queue's head priority by >= self.override_margin, allow the swap
        // despite the interval (starvation avoidance).
        if self.priority_override(resident, cands) { return true; }
        // Otherwise enforce the minimum inter-swap interval (hysteresis).
        match *self.last_swap.lock().unwrap() {
            Some(t) if t.elapsed() < self.min_interval => false, // suppress oscillation
            _ => true,
        }
    }
}
```

`record_swap()` (already present, `swap.rs:158`) is called by the scheduler right
after a successful `execute_model_swap`. Default `min_interval` is a small multiple
of observed swap cost (config, default ~10s per the stub's own worked example);
`override_margin` defaults to a one-priority-band gap so urgent work always
preempts the debounce. The scheduler wraps its evaluator in
`DebouncedSwapEvaluator::new(DefaultSwapEvaluator, min_interval)` **by default in
wave-2** (today it defaults to the raw `DefaultSwapEvaluator`, `lib.rs:146`) — this
is the one behavioral default-flip, justified because un-debounced swapping is a
latent thrash bug, and the override keeps it from ever starving priority work.

**(b) `LocalitySelection` — seam designed, tokenized-prefix coupling RE-DEFERRED
(with reasoning).** Today it is `todo!()` (`selection.rs:72-79`). The intent:
among candidates for the resident model, prefer one whose prompt prefix is already
warm in the KV cache so prefill is skipped (`engine`+`cache` restore the blob —
cache.md concern 2). The seam I *do* design now:

```rust
pub struct LocalitySelection { index: Arc<dyn CacheIndex> } // injected at InferenceService::start
pub trait CacheIndex: Send + Sync {   // read-only view cache exposes; authored with cache.md
    fn has_warm_prefix(&self, model_id: &ModelId, prompt_hash: &str) -> bool;
}
impl SelectionPolicy for LocalitySelection {
    fn select<'a>(&self, cands: &'a [CompletionRow]) -> Option<&'a CompletionRow> {
        // INVARIANT: locality only breaks ties WITHIN the top priority band —
        // it must NEVER admit a lower-priority cache-hit over a higher-priority
        // cache-miss (priority strictly dominates locality). cands are already
        // (priority DESC, created_at ASC); restrict to the leading equal-priority
        // run, then prefer a warm-prefix hit, else FIFO.
        let top_prio = cands.first()?.priority;
        cands.iter().take_while(|c| c.priority == top_prio)
             .find(|c| self.index.has_warm_prefix(&c.model_id, &prefix_hash(c)))
             .or_else(|| cands.first())
    }
}
```

The **priority-dominates-locality invariant** and the **read-only `CacheIndex`
seam** are designed now (they are the parts that constrain the interface). What I
**explicitly re-defer** to the cache-integration fill, with reasoning: computing
`prefix_hash(c)` requires the *tokenized*-prefix hash keyed as `(model_id,
validity_token, prompt_hash)` in cache's index (cache.md line 200), and
tokenization is the **engine's** job — it is not available at selection time
(selection precedes admission/prefill). So a truthful v1 `LocalitySelection` either
(i) queries a cache-side *approximate* index (raw-prompt-prefix, cheap, no
tokenizer) accepting some false-negatives, or (ii) waits for cache to expose a
pre-tokenization coarse index. Both depend on cache's index shape (cache.md flags
longest-common-prefix/radix matching as its own open question). The seam above
lets the fill pick either without restructuring the scheduler — which is exactly
what the stub's own comment asked for ("added without restructuring the
scheduler"). `LocalityAwareSwapEvaluator` (`swap.rs:189`) is deferred on the same
grounds (it needs the cache hit-rate signal the same index would provide);
`FifoSelection`/`DefaultSwapEvaluator` remain the correct wired defaults until
cache's index lands.

## Relationships / edges

Inference-internal contract edges (compiled-in trait/handle seams — NOT wire
contracts; the eight libs compile into one `inference` daemon, INTENT #22/#29):

- **scheduler → engine** via `engine-exec` — submit/drain/swap/cancel/abort + the
  crash-recovery id drain + the interruptibility reads (`is_swapping`,
  `running_count`, `resident_model`). **Provider is `engine`** (engine.md authors
  the trait); I am the consumer and pin my *call obligations* below (drain-or-cancel
  before swap; discard-vs-requeue on preempt). (see scaffold/contracts/engine-exec.md)
- **telemetry → {scheduler, api}** via `system-state` — `SystemState` each tick,
  **extended in wave-2 with `effective_max_concurrent: Option<u32>`** (concern 1).
  Provider is `telemetry`; I am the consumer and propose the field addition below.
  (see scaffold/contracts/system-state.md)
- **scheduler → telemetry (kernel)** via `kernel-confidence` — I read
  `effective_max_concurrent` at the current operating point (concern 1). The
  fitted-surface/confidence query is authored by `telemetry`; I author my
  **consumption side** below. (see scaffold/contracts/kernel-confidence.md)
- **scheduler → models** via `model-ensure` — ensure the swap target is downloaded
  before `execute_model_swap`, and surface download-pipeline status (unblocks
  today's no-op `download_model` stub). **I author this** (scheduler is the
  initiator). (see scaffold/contracts/model-ensure.md)
- **benchmark → scheduler** via `benchmark-collections` — priority-0
  `request_full_system` sweep collections submitted *through* the scheduler; the
  **exclusivity guarantee is the scheduler's side of the contract** (concern 2).
  Benchmark is the initiator; **I author the scheduler-side guarantee** below.
  (see scaffold/contracts/benchmark-collections.md)
- **api → {scheduler, …}** via `api-dispatch` — `submit`/`cancel`/`is_queue_empty`
  + the wave-2 `subscribe_tokens(id)` the WS token stream needs (api.md concern 2).
  Provider surface is `api`-authored; I note the **scheduler-provided methods**
  below. (see scaffold/contracts/api-dispatch.md)
- **scheduler ↔ store** via `store-access` — the pending-queue views
  (`select_pending`/`select_pending_for_model`, priority-ordered), state
  transitions (`mark_running`/`mark_cancelled`/`requeue_preempted`), model lookup,
  crash recovery. **Provider is `store`** (store.md authors it, incl. the
  reentrancy invariant); consumer note below. (see scaffold/contracts/store-access.md)

Feeds an inference-owned cross-cutting protocol (scheduler is the *source*, not a
contract party):

- the **interruptibility transitions** (concern 3) are what the `inference`
  composition root forwards to `mesh-client` as `restart-protocol`
  `InterruptibilityUpdate` frames; the scheduler owns the transition function,
  `inference`/`supervision` own the wire and the ladder. Flagged for reconciliation
  with inference.md's concern-4 table (the benchmark-`Idle` deviation).

Internal-lib seams (compiled-in, NOT contract edges): imports `substrate-types`
(`CompletionId`/`CompletionRequest`/`CompletionRow`/`CompletionState`/`ModelId`/
`SystemState`/`LifecycleEvent`/`StreamEvent`/`Interruptibility`), holds an
`Arc<ExecutionEngine>`, a `Store`, an `Arc<Telemetry>`, and (for
`LocalitySelection`, when wired) an `Arc<dyn CacheIndex>` — all handed down by the
`inference` composition root, never linked cross-app.

## Nesting

Parent: **inference** | Children: none. Modules `queue`, `admission`, `swap`,
`selection` under `lib/scheduler` — libraries under the `inference` app (INTENT
#22), never a top-level crate. Matches `overview.md`'s tree (scheduler under
inference, no children).

## Thoroughness level

**implementation-ready** for: the kernel-driven `slots_to_admit` with the
scalar-fallback layering (concern 1); the generalized declarative
preemption-threshold exclusivity gate + instant preemption + discard-vs-requeue +
the benchmark-bypasses-kernel circularity break (concern 2); the interruptibility
transition function + the mode-tracking that makes it cheap + the benchmark-`Idle`
deviation (concern 3); and the `DebouncedSwapEvaluator` (concern 4a). **approach-
sketched** for: `LocalitySelection` (the read-only `CacheIndex` seam and the
priority-dominates-locality invariant are decided; the tokenized-prefix-hash
coupling is explicitly re-deferred to the cache-integration fill with reasoning —
concern 4b), and the optional synchronous per-decision `kernel-confidence` query
(default path is the `SystemState` scalar — concern 1).

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), per the wave2-plan tier
(`scheduler` = Opus), grounded in the REAL `lib/scheduler/src/{lib,admission,swap,
selection,queue}.rs` + `lib/benchmark/src/{lib,suite}.rs` (the priority-0/
`preemption_threshold`/`request_full_system` encoding) + `lib/types/src/system.rs`
(the `SystemState` shape `effective_max_concurrent` extends), and the batch-5/6
neighbor designs `telemetry.md`, `benchmark.md`, `engine.md`, `store.md`, `api.md`,
`cache.md`, `inference.md`, `supervision.md`. INTENT #12/#22/#29/#38/#77/#85.

## Suggested fill-model

**implementation-ready + high complexity → strong model for the admission core,
mid model OK for the rest.** Two carve-outs a careful hand must not hand-wave: (1)
the **kernel-driven `slots_to_admit`** (concern 1 — the `Some`/`None` layering
where a wrong default either over-admits into OOM or never admits on a cold node;
test the fallback path with `effective_max_concurrent: None` at every pressure
band against today's passing admission tests, which MUST still pass unchanged);
(2) the **exclusivity gate + instant preemption + discard-vs-requeue** (concern 2 —
the one place a wrong predicate either records a poisoned benchmark sample or fails
real work; test that a priority≥1 arrival preempts a running priority-0 sweep the
same tick and that the preempted sweep completion is `Cancelled`-not-requeued while
a preempted priority≥1 completion is requeued). The interruptibility function and
`DebouncedSwapEvaluator` are near-transcription of the designs above.
`LocalitySelection` is fill-blocked on cache's index shape — sequence it **after**
`cache` exposes its read seam; do not implement past the `CacheIndex` trait now.
Sequence the whole module **after** `telemetry` (owns `effective_max_concurrent`),
**alongside** `benchmark` (the exclusivity counterpart), and **after** `engine`'s
`is_swapping()`/`engine-exec` shape is frozen.

---

## Proposed contracts (wave 2)

Proposals only; the per-pair round reconciles both sides. I do NOT edit
`scaffold/contracts/*`. wave2-plan §3a makes `scheduler` a party to seven
inference-internal pairs. Three I **author** (`model-ensure`; the scheduler side
of `benchmark-collections`; my consumption side of `kernel-confidence`); four I
**consume** and note against the authoring neighbor (`engine-exec` — engine;
`system-state` — telemetry, plus the one field I ask it to add; `api-dispatch` —
api; `store-access` — store). All are **in-process Rust API surfaces** (compiled
into `inference`), not serialized wire contracts — the "schema" is the trait/method
surface the Skeleton Builder freezes; version-sensitivity is N/A (the whole
`inference` crate revs as one binary, INTENT #45). Structs reference
`types::{system, restart, completion, model}`.

### `kernel-confidence` (scheduler → telemetry) — the consumption side (co-authored)

- **Purpose.** The scheduler reads the kernel's `effective_max_concurrent` at the
  current operating point to drive admission (concern 1). `benchmark` reads the
  *other* half of this edge ("where is confidence lowest?"); telemetry authors the
  fitted-surface/confidence query. This proposal pins **the scheduler's read**.
- **Surface (the scheduler's default read is the `SystemState` scalar, concern 1):**
  ```rust
  // Default hot path: telemetry stamps this on SystemState each sample; scheduler reads it.
  //   sys.effective_max_concurrent: Option<u32>
  //     Some(k) => kernel is confident here: sustain k parallel completions at this
  //                operating point (observed pressure × resident model × parallelism)
  //     None    => kernel confidence below threshold at this point → scheduler falls
  //                back to the scalar memory heuristic (concern 1c)
  // Optional (approach-sketched, off the hot path): an explicit query telemetry may expose
  //   fn effective_max_concurrent_at(&self, op: OperatingPoint) -> Option<u32>;
  //   struct OperatingPoint { model_id, est_output_len, parallelism, mem/cpu/gpu pressure }
  ```
- **How the scheduler uses it.** ONLY to set the admission target ceiling
  (concern 1b); it never writes the kernel, never triggers a fit, and never blocks
  on it (a stale/absent read degrades to the fallback, never an error). Under
  **benchmarking mode the scheduler ignores this read** and admits to physical
  `max_concurrent` (concern 2.5 — the kernel must not clamp its own training data).
- **Error cases.** None surfaced to the scheduler: `None` is a normal branch (the
  fallback), not an error. A telemetry read that would block is treated as `None`.
- **Conformance requirement.** With `effective_max_concurrent: Some(k)`,
  `slots_to_admit` returns `k.min(max_concurrent).saturating_sub(running)` below
  hard pressure and `0` at/above hard pressure; with `None`, it reproduces today's
  scalar-heuristic outputs **exactly** (the existing `admission.rs` tests pass
  unchanged). Under benchmarking mode, admission uses `max_concurrent` regardless
  of `effective_max_concurrent`.

### `system-state` (telemetry → scheduler) — consumer note + the ONE field I ask telemetry to add

- **Purpose.** The per-tick `SystemState` snapshot the scheduler classifies for
  pressure and admission. Authored by `telemetry`; I consume it. Wave-2 ask:
  **add `effective_max_concurrent: Option<u32>`** to `types::system::SystemState`
  (concern 1) — the field wave-1 flagged as stubbed/absent (OQ-2).
- **Consumer note.** The scheduler reads `memory_pressure` (hard/soft gates,
  unchanged), `resident_model` (swap decision), `running_count`/`pending_count`
  (bookkeeping), and the new `effective_max_concurrent` (admission target). It
  treats the GPU fields as honest-estimate-flagged (`is_gpu_estimate`) but does
  not itself render the tilde — that is the dashboard/surface's job.
- **Version-sensitivity.** `effective_max_concurrent` is additive
  (`#[serde(default)]` → `None` on older producers, which the fallback handles), so
  a scheduler build reading an older telemetry's `SystemState` simply always takes
  the scalar path — no break. (Same-binary in practice; the additive discipline
  matters only for the `/v1/system/state` wire read via `node-state-poll`.)

### `benchmark-collections` (benchmark → scheduler) — the scheduler-side exclusivity guarantee (I author)

- **Purpose.** Benchmark submits priority-0 `request_full_system` sweep collections
  *through* the scheduler; the scheduler **guarantees measurement isolation**
  (concern 2). Benchmark owns *which* completions to submit and re-plans discarded
  ones; the scheduler owns the exclusivity/preemption semantics.
- **Surface (how benchmark reaches the scheduler — grounded in real code).**
  Benchmark inserts a `CollectionRow{ request_full_system: true }` and priority-0
  `CompletionRequest`s (`preemption_threshold: Some(1)`) via `store` +
  `scheduler.notify_new_work()` (today's `lib.rs`/`suite.rs` path is preserved). No
  new submit API is required — the exclusivity lives in the scheduler's admission
  gate, keyed on the completion fields benchmark already sets.
- **The guarantee (scheduler side).**
  ```
  admit a preemption_threshold=Some(t) completion  ⟺  no pending/running work has priority >= t
  a priority>=t arrival                             ⟹  preempt every running preemption_threshold=Some(t) completion THIS tick
  a preempted background completion                 ⟹  Cancelled + DISCARDED (benchmark re-plans); never recorded
  benchmarking mode admission target                =  physical max_concurrent (kernel bypass, concern 2.5)
  ```
- **Error cases.** A benchmark completion that fails on its own (engine error) goes
  through the normal `mark_failed` path — benchmark distinguishes a *discarded*
  (preempted) sample from a *failed* one via the terminal state (`Cancelled` vs
  `Failed`). No scheduler-specific error type.
- **Conformance requirement.** (a) With any priority≥1 work pending/running, zero
  priority-0 completions are admitted; (b) a priority≥1 `submit` during a running
  sweep transitions all running priority-0 completions to `Cancelled` within one
  tick and does not record their runs as benchmark samples; (c) with no priority≥1
  work, priority-0 completions admit up to physical `max_concurrent` (not the
  kernel target).

### `model-ensure` (scheduler → models) — ensure-downloaded before swap (I author)

- **Purpose.** Before `execute_model_swap` loads a target model, the scheduler
  ensures its weights are present, and surfaces download-pipeline status (unblocks
  the no-op `download_model` REST stub — model-ensure.md). Today `execute_model_swap`
  fails with `ModelNotDownloaded` if `file_path` is absent (`lib.rs:428`); wave-2
  routes that miss into `models`' resumable pipeline instead of a hard fail.
- **Surface (scheduler calls `models`; models authors its internals):**
  ```rust
  trait ModelEnsure {                       // implemented by models
      // idempotent: returns the local path if resident, else kicks/observes the pipeline
      async fn ensure_downloaded(&self, model_id: &ModelId) -> Result<EnsureState>;
      fn download_status(&self, model_id: &ModelId) -> Option<DownloadProgress>;
  }
  enum EnsureState { Ready { path: String }, InProgress { pct: f32 }, NotFound }
  ```
- **How the scheduler uses it.** In `execute_model_swap`: call `ensure_downloaded`;
  on `Ready{path}` proceed to `engine.swap_model`; on `InProgress` leave the
  triggering completions `Pending` and `notify_new_work` when the download completes
  (models signals via the store/notify) — the swap simply retries a later tick; on
  `NotFound` fail the completions with a catchable `ModelNotFound`. The scheduler
  never blocks its tick on a download.
- **Error cases.** `EnsureState::NotFound` → the scheduler fails the pending
  completions for that model (they can't ever run) rather than looping. A transient
  download error surfaces as `InProgress` retryable, not a completion failure.
- **Conformance requirement.** A swap target that is not yet downloaded does NOT
  fail the completion outright while a download is in progress — it stays `Pending`
  and the swap succeeds once `ensure_downloaded` returns `Ready`.

### `engine-exec` (scheduler → engine) — consumer note + my call obligations (engine authors)

- **Purpose.** The scheduler's submit/drain/swap/cancel surface over the engine.
  **engine.md authors the `EngineExec` trait**; I consume it and pin the *behavioral
  preconditions the scheduler owns.*
- **Call obligations (the scheduler's side of the behavioral contract).**
  - **Drain-or-cancel before swap.** `swap_model` does NOT drain (engine.md); the
    scheduler MUST `drain(timeout)` (normal) or `cancel_all_running` + requeue
    (hard pressure) before `swap_model` — exactly today's `execute_model_swap`.
  - **`is_swapping()` bracket.** The scheduler sets its own `swapping` flag around
    the whole `execute_model_swap` (drain→swap), so the interruptibility function
    reports `CriticalSection` for the *entire* swap window, not just engine's inner
    `is_swapping()` (concern 3).
  - **Preemption uses `abort_slot`/`cancel_all_running` + the discard/requeue split**
    (concern 2.3): background (benchmark) → `mark_cancelled`; foreground →
    `requeue_with_circuit_breaker`.
  - **Crash-recovery drain.** `take_running_ids()` feeds requeue on boot
    (`recover_running`), the L4 backstop (inference.md concern 4).
- **Error cases (consumed).** `EngineError::BackendUnavailableOffline` from
  `swap_model` is **catchable** — the scheduler holds the completions `Pending`
  (does not fail them) and retries provisioning opportunistically (engine.md
  concern 3). `DrainTimeout` → requeue remaining ids and proceed with the swap
  (today's behavior, `lib.rs:444`).

### `api-dispatch` (api → scheduler) — scheduler-provided methods (api authors)

- **Purpose.** The in-process methods `api` dispatches into (api holds no business
  logic). **api.md authors the dispatch surface**; I note what the scheduler
  provides.
- **Scheduler-provided surface (grounded in real code + the wave-2 addition):**
  ```rust
  // on Arc<Scheduler>:
  fn submit(&self, req: CompletionRequest) -> Result<CompletionId>;   // lib.rs:222
  async fn cancel(&self, id: CompletionId) -> Result<()>;             // lib.rs:273
  fn is_queue_empty(&self) -> bool;                                   // benchmark idle-gate + api
  // WAVE-2 (api.md concern 2 — closes the ws.rs poll-loop gap):
  fn subscribe_tokens(&self, id: CompletionId)
        -> Option<broadcast::Receiver<StreamEvent>>;
  ```
- **`subscribe_tokens` note.** The per-completion token `broadcast::Sender` is
  created at **submit** time (not admit — so a client connecting immediately after
  `POST /v1/completions` cannot miss it; api.md concern 2), lives in a scheduler-owned
  registry keyed by `CompletionId`, is fed by the engine tee via the `token_tx`
  the scheduler already creates in `admit_pending` (`lib.rs:398` — today the
  receiver is dropped; wave-2 keeps it in the registry), and is dropped shortly
  after the terminal event. `None` = no live channel (already terminal or never
  admitted) → api falls back to a store replay (a normal branch, not an error).
  **This is the concrete producer side of the seam api specified as a consumer** —
  the wave-1 "receiver currently dropped here; the WebSocket layer will wire this
  up in a future phase" comment is now resolved on the scheduler side.
- **Error cases.** `submit` validates the model exists (`get_model`, `lib.rs:224`)
  → `ModelNotFound`; `cancel` on a terminal completion → `CompletionTerminal`
  (mapped to 409 by api). No new error types.

### `store-access` (scheduler ↔ store) — consumer note (store authors)

- **Purpose.** The scheduler's per-tick queue reads + state transitions. **store.md
  authors the contract**, including the single-writer `Mutex<Connection>` discipline
  and the observer-reentrancy invariant; I consume it.
- **Consumer note.** The scheduler uses the priority-ordered pending views
  (`select_pending`, `select_pending_for_model` — `(priority DESC, created_at ASC)`,
  which is what makes `FifoSelection`/exclusivity correct without an in-memory
  queue), the transitions (`mark_running`/`mark_cancelled`/`mark_failed`/
  `requeue_preempted`), `get_model`/`get_completion`, and `recover_running` on boot.
  It respects the store contract: **no `.await` while holding a store guard**
  (every scheduler store call is a synchronous take-and-drop), and it never
  registers a `StoreObserver` (that seam is inference's `PromiseRegistry` + the
  future KG observer — store.md concern 2). The queue's crash-survival property
  (queue == a store query, so no separate recovery) is preserved verbatim.
- **Wave-2 touchpoint.** `insert_completion` gains a `&Provenance` arg (store.md
  concern 4) that `api` stamps from the inbound request envelope; the scheduler's
  `submit` **threads that provenance through** rather than defaulting it at the
  store boundary (a defaulted provenance is a silent INTENT #85 violation) — the
  provenance originates at `api`, passes through `submit`, lands on the row.
