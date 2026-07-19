# Contract: benchmark-collections

## Parties
benchmark  ->  scheduler
(initiator: **benchmark** — submits the sweep collections and re-plans discarded
ones; guarantor: **scheduler** — owns the exclusivity / preemption / discard
semantics, which are the substance of the contract)

Both are compiled-in libraries of one `inference` daemon (INTENT #22/#29). This is an
**in-process seam, NOT a wire contract** (INTENT #45).

## Purpose
Benchmark submits priority-0, fully-preemptible, full-system-exclusive **cell
batches** (one test = `parallelism` concurrent completions at one output/input dial)
through the scheduler, and relies on the scheduler's **measurement-isolation
guarantee** to make the samples valid. Measurement isolation is a *scheduling
guarantee, not a benchmark-side hope*: benchmark cannot enforce it from its side, so
the scheduler's admission gate + instant preemption + preempted-sample discard IS the
contract. Benchmark owns *which* completions to submit (the active-learning
selection) and re-plans any discarded cell; the scheduler owns *when* they run and
that no other work contaminates the window.

## Schema

### Submission shape (benchmark → scheduler, grounded in the real `suite.rs`)

One cell batch = one `CollectionRow` + `parallelism` priority-0 `CompletionRequest`s.
Types reference `types::{completion, collection, model}`.

```rust
// the collection (isolation intent):
CollectionRow {
    id: CollectionId,
    request_full_system: true,       // "these need the node otherwise idle"
    save_partial_results: true,
    cancel_on_failure: false,
    state: Active,
}
// each of `parallelism` members:
CompletionRequest {
    priority: 0,                     // lowest — anything preempts
    preemption_threshold: Some(1),   // ANY priority>=1 work preempts instantly
    collection_id: Some(batch_collection),
    max_tokens: Some(output_len),    // the output dial (ranges to 100_000, clipped by n_ctx)
    prompt: synthetic(input_len),    // the input dial (~4 chars/token filler)
    temperature: Some(0.0),
    metrics: MetricsFlags::ALL,
    metadata: {                      // wave-2: selection provenance
        "benchmark": true,
        "cell": { "output_len": .., "parallelism": .., "input_len": .. },
        "target_pressure_band": ..
    },
}

// submitted via the ordinary scheduler surface (no new submit API — exclusivity
// lives in the admission gate, keyed on the fields above):
fn submit(&self, req: CompletionRequest) -> Result<CompletionId>;   // existing (api-dispatch too)
fn cancel(&self, id: CompletionId)      -> Result<()>;              // L2-abort / rejected-batch path
```

Invariant: **exactly ONE batch in flight per node, ever** (isolation is global).
Wave-2 vs the live code: cells come from the active-learning selection (see
`kernel-confidence`), never a grid; `output_len` ranges to 100_000 clipped by
`n_ctx`.

### The scheduler-side guarantee (the contract's teeth)

Keyed on `preemption_threshold` / `priority`, owned entirely by the scheduler
(scheduler concern 2):

```
admit a preemption_threshold=Some(t) completion  ⟺  no pending/running work has priority >= t
a priority>=t arrival                            ⟹  preempt every running preemption_threshold=Some(t)
                                                     completion THIS tick (same tick as the submit wakes the loop)
a preempted BACKGROUND completion                ⟹  Cancelled + DISCARDED (never recorded as a sample);
                                                     benchmark re-plans it in a future idle window
a preempted FOREGROUND completion                ⟹  requeue_with_circuit_breaker (unchanged)
benchmarking-mode admission target               =  physical max_concurrent (kernel BYPASS — see below)
```

The discard-vs-requeue split keys on a single predicate: `preemption_threshold.
is_some()` → discard (background); `None` → requeue (foreground).

**Kernel bypass (scheduler concern 2.5).** Under benchmarking mode `slots_to_admit`
uses the physical `max_concurrent` ceiling, NOT `effective_max_concurrent` — because
benchmark is the thing *generating* the kernel's training data across parallelism
levels; clamping benchmark admission by the kernel would be circular (the kernel
limiting its own inputs). The hard-pressure safety floor still applies.

### Benchmark's acceptance predicate (batch-atomic validation, benchmark concern 5)

Benchmark accepts a batch (and writes `benchmark_runs` rows via `store-access`) iff
**every** member reached `Completed` with `preemption_count == 0` AND the isolation
window was clean (no non-benchmark completion entered Pending/Running between batch
start and batch end). Otherwise the **whole batch is rejected: zero rows written**,
and benchmark `cancel`s any survivors rather than letting requeued members
re-execute into a mislabeled sample.

## Error cases

- A benchmark member that fails on its own (engine error) → the ordinary
  `mark_failed` path; benchmark distinguishes *discarded* (preempted → `Cancelled`)
  from *failed* (→ `Failed`) via the terminal state. No scheduler-specific error type.
- `SubstrateError::ResourceExhausted(id, n)` on `max_preemption_count` — never reached
  in practice (benchmark cancels rejected batches before requeue re-runs them).
- A benchmark batch submitted against an unloaded model legitimately triggers the
  ordinary swap path (an idle multi-model node); the swap cost is excluded from the
  measurement window, which starts at first-member `Running`.

## Version sensitivity

**N/A** — in-process, one binary (INTENT #45). No struct here crosses a node
boundary through this seam. (`BenchmarkStatus` and the kernel curves DO serialize to
the dashboard, but via `api-dispatch` / the surface schema, additive-only there.)

## Reconciliation notes

- **Preempted-benchmark handling: scheduler-discard vs requeue-then-benchmark-cancel
  (a genuine mechanism divergence, resolved).** scheduler.md concern 2.3 changes the
  live behavior: a preempted *background* completion is **immediately marked
  `Cancelled` (discarded) by the scheduler**, keyed on `preemption_threshold.
  is_some()`. benchmark.md concern 5 leans on the *live code* path where
  `requeue_preempted` flips a preempted benchmark completion back to `Pending` and
  increments `preemption_count`, then benchmark's batch-atomic validation cancels the
  batch. **Resolution: the scheduler's discard-on-preempt wins as the authoritative
  mechanism** — the scheduler owns admission/preemption semantics, and discarding
  background work on preemption (rather than requeuing it) is cleaner: it structurally
  prevents a corrupted sample from ever re-running, and it makes benchmark-vs-real
  work mutually exclusive on the engine (which is what lets `foreground_running()` be
  a cheap single-mode read, scheduler concern 3). **Both sides reach the same
  OUTCOME** — a preempted batch writes zero rows — because benchmark's acceptance
  predicate ("every member `Completed` with `preemption_count == 0`") is failed by a
  `Cancelled` member regardless of which mechanism produced it. **Losing position
  recorded:** the live requeue-then-validate path (benchmark's literal grounding) —
  not adopted as the mechanism because requeuing a preempted background completion
  risks it re-executing into a mislabeled sample and complicates the exclusivity
  bookkeeping; it is preserved only as benchmark's defensive `cancel` of any survivor
  the scheduler didn't discard.
- **`request_full_system` — mechanism or echo? (resolved to echo).** scheduler.md
  concern 2.4: the per-completion `preemption_threshold: Some(1)` gate already
  enforces "node otherwise idle"; `request_full_system` stays a self-documenting
  collection attribute (the dashboard/operator handle for "this sweep is exclusive"),
  NOT a second admission mechanism. benchmark.md agrees (lists it as an isolation
  intent flag). Adopted — no divergent logic; the scheduler keys admission on the
  per-completion threshold only.
- **Interruptibility of a running benchmark batch — `Idle` vs `CriticalSection`
  (flagged; authoritative resolution belongs to `restart-protocol`, NOT this pair).**
  scheduler.md concern 3 reports a running benchmark sweep as **`Idle`** (benchmark
  is the universal yielder; labeling it critical would let the lowest-value work
  block a routine update during the exact idle window an update wants). benchmark.md
  concern 6 reports an in-flight batch as **`CriticalSection { until: kernel-ETA }`**
  (interrupting mid-batch corrupts the sample and wastes the idle window), scoped to
  the batch, never the sweep. **Both agree benchmark yields to any restart** (L2+ ⇒
  cancel-batch → discard, no attempt-count penalty → yield) — the only difference is
  the reported *label*. For the record I favor **scheduler's `Idle`** as
  better-argued: since benchmark yields to restarts anyway, reporting `CriticalSection`
  (which means "do not interrupt me") is misleading and inverts priority; benchmark's
  legitimate intent (don't waste a nearly-done short batch) is served by the L1
  `WaitForIdle` rung waiting out the batch bounded by `until`, not by a
  `CriticalSection` label. **This interruptibility feed is a `restart-protocol`
  concern (a different cluster's contract)** — recorded here because both parties are
  benchmark-collections parties, but the binding resolution is deferred to the
  `restart-protocol` per-pair round.

## Example data

Example world: node **macbook**, model **qwen3-4b**, project **demo**. Benchmark's
active-learning loop (see `kernel-confidence`) selected the low-confidence cell
`{ output_len: 100000, parallelism: 8, input_len: 512 }`. macbook is idle.

```jsonc
// the batch collection:
{ "id": "demo-bench-col-77", "request_full_system": true,
  "save_partial_results": true, "cancel_on_failure": false, "state": "active" }

// 8 members (one shown; ids demo-bench-c-770 .. demo-bench-c-777):
{ "id": "demo-bench-c-770", "model_id": "qwen3-4b", "priority": 0,
  "preemption_threshold": 1, "collection_id": "demo-bench-col-77",
  "max_tokens": 100000, "temperature": 0.0, "metrics": "ALL",
  "metadata": { "benchmark": true,
                "cell": { "output_len": 100000, "parallelism": 8, "input_len": 512 },
                "target_pressure_band": "high" } }
```

Two outcomes:

1. **Clean run (accepted).** All 8 members reach `Completed`, `preemption_count == 0`
   each, no non-benchmark work entered the window. Benchmark writes one
   `benchmark_runs` row (`sample_source: "benchmark"`, `collection_id:
   "demo-bench-col-77"`, covariates sampled at batch start/end) and calls
   `telemetry.notify_new_samples("qwen3-4b")`.

2. **Preempted (rejected).** Midway, a real priority-1 completion `demo-c-1002`
   arrives. Same tick, the scheduler preempts all 8 benchmark members, marking each
   `Cancelled` (discarded). Benchmark's predicate fails (members not `Completed`) →
   **zero rows written**, the cell's attempt counter bumps (toward backoff). The real
   completion `demo-c-1002` runs and later yields its OWN `sample_source: "observed"`
   row via telemetry — the system still learns from the episode, honestly.

## Conformance requirement

A filler's `scheduler` + `benchmark` pass iff, against the example world: (a) with
any priority≥1 work pending/running, **zero** priority-0 members are admitted; (b) a
priority-1 `submit` during the running batch transitions all 8 members to `Cancelled`
within one tick and **no** `benchmark_runs` row is written for that batch; (c) with no
priority≥1 work, the 8 members admit up to the **physical** `max_concurrent` (not the
kernel `effective_max_concurrent`); (d) a *cleanly completed* batch writes exactly
one row with `sample_source: "benchmark"` and the round-trippable `collection_id`; (e)
the preempted real completion `demo-c-1002` is requeued (foreground), never discarded.
