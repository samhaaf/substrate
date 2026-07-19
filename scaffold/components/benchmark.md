# benchmark

**Status:** WAVE-2 REFIT (Fable seat) of the wave-1 approach-sketched design,
REDEFINED by INTENT #12 — the operator's explicit success criteria: "This is a
multidimensional problem. There is no worthwhile simplification of it... I don't
actually want to manually decide what tests to run... use some confidence
interval to inform which tests should we run next. What's the highest
information gain that we can get." The wave-1 idle-gating / priority-0 /
`request_full_system` isolation mechanics are **preserved** (they are real,
working code — `lib/benchmark/src/{lib,suite}.rs`); the fixed Cartesian grid
(`OUTPUT_LENGTHS × PARALLELISM_LEVELS × INPUT_LENGTHS`, top-up to
`target_samples_per_cell = 3`) is **retired** and replaced by the
active-learning loop this file specifies. Wave 2 also resolves the two things
wave-1 flagged and left open: the **livelock risk** on unreachable pressure
regimes (concern 4 — termination is now provable) and the **active/passive
training-set union** (concern 3 — adopted, with producers assigned). Grounded in
the real source (`lib/benchmark/src/lib.rs` `BenchmarkOrchestrator`/`suite.rs`
`BenchmarkSuite`, `lib/telemetry/src/estimator.rs` — the OLS degree-2 fit whose
machinery the confidence surface extends, `lib/scheduler/src/{lib,queue,
admission}.rs` — `preemption_threshold`/`preemption_count`/`requeue_preempted`)
and the neighbor designs: `telemetry.md` + `scheduler.md` (wave-1, co-batch —
the kernel triangle), `store.md` (batch 5 — the pressure-covariate columns +
`sample_source`), `api.md` (batch 5 — `api-dispatch`'s benchmark slice +
`/v1/benchmark/run`), `inference.md` (batch 5 — the interruptibility mapping in
which a benchmark sweep is a `CriticalSection`), `completion-router.md` (batch 3
— benchmark node-pinning is a first-class routed path), `supervision.md` (batch
2 — the ladder benchmark must yield to), and `types.md`. INTENT
#8/#9/#12/#15/#22/#38/#44/#77/#85.

## Charter

`benchmark` is the **adaptive measurement orchestrator** of one inference node
(`lib/benchmark`, internal lib of `inference` — INTENT #22): a background loop
that, whenever the node is idle, asks telemetry's multidimensional kernel
**"where is your confidence currently lowest?"**, generates the single most
informative next test at that operating point, submits it as a **priority-0,
`request_full_system`, `preemption_threshold: Some(1)`** collection through the
scheduler, validates the result against preemption/isolation evidence, and
feeds the accepted sample back into the kernel's training data — repeating
until the kernel's confidence exceeds threshold everywhere it can reach, then
going quiet. **Stress-testing and throughput benchmarking are ONE action**
(INTENT #8): there is no separate stress mode — the extreme corners of the dial
space (parallelism 8 × 100k output) ARE the stress tests, and every test
records both throughput and the system-pressure response observed while it ran.
Its boundary: benchmark decides **WHICH measurements to take and whether each
result is trustworthy** — nothing else. It does not schedule or execute
(`scheduler`/`engine`), does not own the fitted kernel or its confidence
surface (`telemetry` — benchmark is the kernel's *active* data source and its
most demanding query client), does not persist (`store` holds `benchmark_runs`),
does not serve HTTP (`api` dispatches `/v1/benchmark/*` into it), and does not
route across nodes (fleet benchmarking reaches a specific machine via
`completion-router`'s node-pinning; benchmark itself is strictly per-node —
the kernel is a per-machine, per-model object).

## Primary design concerns

### 1. One action, full axis space — the measurement model (INTENT #8/#12)

The operator's axis set, verbatim-grade: performance with respect to **memory
pressure, CPU pressure, GPU pressure** on the system, and **the specifics of
what we're asking it to do** — **parallel completion count** and **output
length (1k / 10k / 100k)**. Input length is retained as a sixth axis (it is
already in the real feature vector and costs nothing). One *test* = one **cell
batch**: `parallelism` concurrent completions at the same
(input_length, output_length) dial setting, submitted as one collection with
`request_full_system: true` (isolation) — exactly today's mechanics. One
*sample* = the batch's measured throughput (tokens/sec, wall time) **plus the
pressure covariates observed during the batch window** (from telemetry's
`SystemState`, persisted onto the `benchmark_runs` row per store.md concern 5:
`mem_pressure`, `cpu_utilization`, `gpu_utilization`, `gpu_mem_used_bytes`,
`is_gpu_estimate`, `sample_source`). Covariates are sampled at batch start and
batch end and recorded as (mean, peak) — a long 100k-output batch ramps
pressure as its KV grows, and the *peak* is the stress-test half of the one
action; `is_gpu_estimate` rides along so the kernel can down-weight
Apple-Silicon GPU estimates honestly (INTENT #9).

Two axis-range changes from the live code, both required by INTENT #12:

- **`OUTPUT_LENGTHS` max rises from 10_000 to 100_000** — but the fixed axis
  arrays (`OUTPUT_LENGTHS`/`PARALLELISM_LEVELS`/`INPUT_LENGTHS`) stop being the
  test set. They become **lattice anchors** on a log-spaced candidate lattice
  over the continuous dial space (concern 2); 1k/10k/100k are named anchors on
  the output axis, not the only tested values.
- **Context-window feasibility clip:** a candidate with
  `input_length + output_length > model n_ctx` is *infeasible* and never
  generated (part of the feasible-region constraint set, concern 3). A 100k
  output anchor simply does not exist for an 8k-context model — the kernel's
  "whole range" is the model's whole *reachable* range.

### 2. The active-learning loop — architecture and selection math

The loop (replacing `run()`'s fixed-grid top-up scan):

```text
loop:
  sleep(idle_threshold_secs)  (or wake on: model registered / kernel invalidated / backoff expiry)
  for each downloaded model (round-robin fairness across models):
    1. GATE     is_queue_empty()? (no non-benchmark pending/running) — else continue
    2. QUERY    telemetry.kernel(model).lowest_confidence(feasible_region, backoff_excluded)
                  -> Option<CandidatePoint { input_len, output_len, parallelism,
                                             predicted_pressure_band, score }>
    3. STOP?    None (min confidence >= θ everywhere feasible, or all in backoff)
                  -> model is CONVERGED or QUIESCENT; continue to next model
    4. GENERATE one cell batch at the candidate's dials (priority 0,
                  request_full_system, preemption_threshold Some(1),
                  MetricsFlags::ALL, one collection, `parallelism` completions,
                  synthetic prompt at input_len)
    5. SUBMIT   scheduler.submit() each; record batch -> ActiveBatch state
    6. AWAIT    batch terminal (store poll / in-process event bus), sampling
                  SystemState at start / end
    7. VALIDATE batch-atomic isolation evidence (concern 5)
                  accepted  -> write benchmark_runs rows (sample_source='benchmark'),
                               telemetry.kernel(model).notify_new_samples()
                  rejected  -> write NOTHING; bump attempt count; maybe backoff (concern 4)
    8. one batch in flight per node at a time, ever (isolation is global)
```

**Selection criterion — uncertainty sampling as the max-info-gain
implementation.** "Maximum information gain" is implemented as **argmax
predictive variance over a candidate lattice**, which for the kernel's
regression family is the standard, closed-form, boring choice: for an OLS fit
(the real `estimator.rs` machinery — design matrix `X`, features `φ(x)`), the
predictive variance at a candidate `x` is `σ̂² · φ(x)ᵀ(XᵀX)⁻¹φ(x)` — the same
`(XᵀX)⁻¹` the SVD solve already produces, evaluated per lattice point. For
D-optimal information gain this argmax-leverage point is exactly the greedy
next-sample choice, so "uncertainty sampling ≈ greedy info-gain" is not a
hand-wave, it is the textbook equivalence for linear-in-features models.
Confidence at `x` is defined as a monotone transform of predictive variance
(e.g. `1/(1+cv)` on the coefficient of variation); the **stop threshold θ and
the lattice are config**, owned by benchmark; the **surface and the argmax are
computed by telemetry** (it owns `X`; benchmark passes constraints and gets a
candidate back — the `kernel-confidence` edge, proposed below). Cold-start
(fewer than MIN_DATA_POINTS samples): telemetry returns a seeded
low-confidence-everywhere surface and the argmax degenerates to a **space-
filling first pass** (maximin over the lattice) — the loop needs no special
cold-start mode, the math does the right thing.

The candidate **lattice** is deliberately coarse and log-spaced (≈ 5 output ×
4 parallelism × 3 input anchors + pressure bands ≈ low hundreds of points):
fine enough to steer, small enough that evaluating variance per point per idle
tick is negligible. The kernel itself stays continuous (quadratic spline inside
the observed range, linear outside — INTENT #8); the lattice only discretizes
*selection*, not *modeling*.

### 3. A partially controllable design space — dials, covariates, and the feasible region

The non-obvious hard part, carried from wave-1 and now resolved: **you can set
output-length / parallelism / input-length; you cannot set memory/CPU/GPU
pressure.** Three mechanisms cover the pressure axes, and their union is the
kernel's training set:

1. **Self-induced pressure (active, the main mechanism).** The dials *are* a
   pressure instrument: parallelism × output-length drives KV-cache growth,
   GPU occupancy, and memory pressure. A batch aimed at a low-confidence
   high-pressure band is generated by choosing the dial setting whose
   *predicted* pressure (from the kernel's own current fit of dials→pressure,
   or a monotone prior before enough data exists) lands in that band. This is
   INTENT #8 made structural: the stress test and the pressure-axis sample are
   the same submission.
2. **Passive/organic observation.** Every *real* completion that runs with
   metrics enabled yields an observation at whatever pressure the system
   actually had — precisely the regimes benchmark's idle-gated loop can never
   reach (high pressure from *other* processes / real load). These are
   persisted as `benchmark_runs` rows with `sample_source='observed'`;
   **telemetry is the producer** of observed rows (it already receives
   completion metrics), benchmark of `'benchmark'` rows — this assigns the
   producer store.md left open. The kernel fits the union and can weight the
   two populations differently (telemetry's call — open question below).
3. **Explicitly rejected: a synthetic pressure generator** (memory ballast,
   CPU spinner). It would be a second mechanism where one suffices, it
   distorts the very measurement it enables (a spinner competing with
   inference is not the pressure regime real work produces), and it violates
   the one-action principle. If a pressure band is reachable neither by dials
   nor observed organically, the honest answer is *lower confidence there,
   linear extrapolation outside the observed range* — which is exactly what
   the kernel's inside-spline/outside-linear shape already encodes.

**The feasible region** passed to the kernel query is therefore: the dial
lattice, clipped by context window (concern 1), crossed with the pressure bands
*predicted reachable from the current baseline* (current `SystemState` pressure
+ the batch's own predicted induction). Bands unreachable right now (e.g. a
high-memory-pressure band while the machine is idle and empty) are **excluded
from active selection** — not retried, not waited on — and left to passive
observation. This exclusion is load-bearing for termination (concern 4).

### 4. Termination and convergence — the livelock fix (provable, not vibes)

Wave-1's flagged risk: a naive "always chase the lowest-confidence point" loop
livelocks when the lowest-confidence region is unreachable (the argmax returns
the same point forever, the test never lands in-band, confidence never rises).
The wave-2 loop terminates by construction, via three mechanisms:

1. **Feasible-region projection (concern 3).** The argmax is taken over
   reachable candidates only. Unreachable pressure bands are excluded *before*
   selection, so the loop never queues a test it cannot land.
2. **Per-region attempt budget + exponential backoff.** Every candidate lattice
   point carries an attempt counter (in-memory, rebuilt on restart from the
   accepted-sample history). A batch that is **rejected** (preempted, failed,
   or landed covariates outside the targeted pressure band — the "aimed for
   high pressure, induction prediction was wrong" case) bumps the counter;
   after `max_attempts` (default 3) the point enters exponential backoff
   (cooldown `2^k × backoff_base`, capped at ~24h) and is excluded from
   selection until expiry. Prediction-miss rejections also *feed the miss
   back*: the observed (dials → pressure) pair improves the induction model,
   so the next selection aims better.
3. **The stop condition is a threshold, not exhaustion.** Per model:
   **CONVERGED** when min confidence over the feasible lattice ≥ θ;
   **QUIESCENT** when every below-threshold point is in backoff. Either state
   ends active testing for that model; the loop drops to a long idle interval
   and re-wakes only on: backoff expiry, kernel invalidation (model/backend
   version change — telemetry/models own the staleness signal), a new model,
   or the confidence surface dropping (passive samples revealing drift).

**Convergence argument (the proof sketch a filler can test against):** each
loop iteration either (a) produces an *accepted* sample — which strictly
reduces predictive variance in a neighborhood (adding a row to `X` cannot
decrease `XᵀX` in the Loewner order, so variance is non-increasing everywhere
and strictly decreasing at the sampled point), and only finitely many such
reductions fit above any ε-floor before the θ threshold is crossed on a finite
lattice; or (b) produces a *rejection* — which strictly advances a finite
attempt counter toward backoff on a finite lattice. Both branches strictly
consume a finite resource, so the loop reaches CONVERGED or QUIESCENT in
finitely many iterations. No livelock, including on adversarial regimes.
(Noise floor honesty: with sample noise, θ must be chosen achievable —
θ is calibrated against the observed residual variance σ̂², not an absolute
constant, so a noisy machine converges to *its own* honest confidence ceiling
rather than chasing an impossible one. σ̂-relative θ is part of the
`kernel-confidence` contract.)

### 5. Preempted-sample attribution — batch-atomic validation (don't poison the kernel)

Benchmark completions carry `preemption_threshold: Some(1)`: any real
(priority ≥ 1) work preempts them instantly — the scheduler's
`requeue_preempted` flips them back to Pending and increments
`preemption_count` (real code, `scheduler/lib.rs:475`). A preempted-then-
resumed completion's wall time spans the preemption gap, and its surviving
batch-mates ran at a *changed* effective parallelism the moment their sibling
was evicted — so **neither the preempted completion nor its batch-mates are
valid samples of the intended cell.** The rule, stated as the contract's
teeth:

- **Validation is batch-atomic.** A cell batch is accepted iff **every**
  completion in its collection reached `Completed` with `preemption_count == 0`
  **and** the isolation window was clean: no non-benchmark completion entered
  Pending/Running between batch start and batch end (store window query —
  catches the edge where real work arrived but the benchmark finished before
  the preemption executed, briefly overlapping). Otherwise the **whole batch
  is rejected: zero `benchmark_runs` rows are written.** Rejected timing data
  is *discarded, not re-attributed* — a preempted completion's metrics are not
  internally consistent (the wall clock spans the gap), and "salvage the
  survivors at their observed concurrency" would silently turn a controlled
  sample into a mislabeled one. The kernel's cleanliness outranks the lost
  idle time.
- **The organic observation is not lost.** The real work that caused the
  preemption generates its *own* `sample_source='observed'` row via telemetry
  (concern 3.2) — the system still learns from the episode, just honestly:
  from the real completion, under its real covariates, not from the corrupted
  benchmark.
- Rejection feeds concern 4's attempt counter, so repeated preemption of the
  same cell (a busy node) backs off rather than hammering — benchmark on a
  busy node naturally goes quiet, which is exactly right.
- The scheduler-side guarantees benchmark relies on (and `benchmark-collections`
  pins): priority-0 admitted only when nothing else is pending/running;
  instant preemption at threshold 1; `preemption_count` faithfully incremented;
  `request_full_system` honored. Measurement isolation is a **scheduling
  guarantee, not a benchmark-side hope** (scheduler.md, re-affirmed).

### 6. Benchmark as CriticalSection — coordination with scheduler and supervision

The two-sided truth: **locally**, benchmark is the lowest-priority work on the
node (anything preempts it); **fleet-wise**, an in-flight batch is a
`CriticalSection` (interrupting the process mid-batch corrupts the sample AND
wastes the idle window — inference.md concern 4 already reports it as such).
Reconciliation, refining inference.md's mapping:

- **CriticalSection is scoped to the in-flight batch, never the sweep.**
  Between batches the orchestrator contributes `Idle`. `until` = the batch's
  kernel-estimated completion time (benchmark asks the kernel for the ETA of
  its own test — the kernel estimating its own next training point is not
  circular, it is the cold-start-safe `estimate()` path that already exists).
- **A restart request always outranks a measurement.** On
  `FinishAndRelinquish` (L2) or above, benchmark **cancels the in-flight
  collection at the next completion boundary, discards the batch (concern-5
  rejection, no attempt-count penalty — the node, not the regime, aborted it),
  and yields immediately**; it does not "finish the sweep." On `WaitForIdle`
  (L1), supervision simply waits out the batch (bounded by `until`). Benchmark
  must never be the reason a node cannot update — a missed sample costs one
  idle window; a blocked compatibility restart costs the fleet.
- **Long-batch bound:** a 100k-output × parallelism-8 batch on a slow node can
  run tens of minutes. If the kernel-predicted batch duration exceeds
  `max_batch_duration` (config, default 15 min), the candidate is **still
  tested but at reduced parallelism or via the nearest cheaper lattice
  neighbor** in that session, with the full-cost point left for an explicitly
  operator-triggered `run_now` (concern 7) — keeping unattended
  CriticalSections bounded. Flagged (open question) for the operator: the
  default cap and whether unattended full-cost corners are ever allowed.

### 7. Fleet benchmarking through the mesh — node-pinned, caller-side (INTENT #15)

"When you're running test suites... obviously we want to target specific
machines." The design keeps benchmark strictly per-node and makes fleet
targeting a *routing* fact, already designed on the router side
(completion-router.md concern 5) — benchmark confirms the terminus side:

- **`POST /v1/benchmark/run` pinned via `?node=N` / `Node{N, inference}`**
  reaches node N's api through the mesh front door (`:3649` → router → pinned
  forward, pin authoritative, no failover); api dispatches into N's local
  orchestrator (`api-dispatch`, proposed below): `run_now { model_id?,
  target? }` — an on-demand active-learning session (or a specific candidate
  region) that runs under the same isolation/validation rules, just without
  waiting for the idle timer. The response is the collection id; progress is
  observable via the ordinary `/v1/collections/:id` and the kernel via
  `/v1/benchmark/kernel`.
- **A fleet sweep is a caller-side loop over node pins** (dashboard action or
  CLI: `resolve_all("inference")` → one pinned `run_now` per node), NOT a
  benchmark-internal fleet mode. Benchmark never learns mesh topology, never
  holds cross-node state, and needs no new cross-node contract — it rides
  `v1-completion-api`'s existing pin spelling. The dashboard's per-node
  `benchmark_run` action (api.md concern 6, surface schema) is exactly this
  button; the kernel curves it renders are per-(node, model) by construction.
- The router does not reason about benchmark exclusivity (its own stated
  boundary); a pinned `run_now` landing on a busy node is admitted priority-0
  and simply waits for idle — the pin targets the *machine*, the scheduler
  still owns *when*.

### 8. The ingestion pipeline — who writes, who reads, who is notified

Boring seams, no new plumbing classes:

- **benchmark → store (write):** accepted batches only; one `benchmark_runs`
  row per cell batch with dials, throughput, covariates (mean + peak),
  `is_gpu_estimate`, `sample_source='benchmark'`, and — additive ask on
  store.md's column list — the batch's **`collection_id`**, so a row is
  auditable back to its collection and the batch-atomicity invariant is
  checkable after the fact.
- **telemetry → store (write):** organic `sample_source='observed'` rows from
  real completion metrics (concern 3.2 — producer assignment, needs
  telemetry-side confirmation, friction below).
- **telemetry ← store (read):** the kernel's training set is the union read
  at (re)fit; old rows with NULL covariates load as legacy low-information
  samples (store.md's backward-compat rule).
- **benchmark → telemetry (notify + query):** `notify_new_samples(model)`
  after an accepted write (cheap refit trigger), and the `kernel-confidence`
  query surface (concern 2). No bulk data ever flows benchmark→telemetry
  directly — samples travel through store, keeping one durable source of
  truth (INTENT #38: no shadow datasets).
- **api → benchmark (dispatch):** `run_now` / `status` / `set_enabled`
  (concern 7; replaces the wave-1 `schedule_if_idle` dispatch shape api.md
  transcribed — reconciliation flagged below).

## Relationships / edges

- **benchmark → scheduler** via `benchmark-collections` — priority-0
  `request_full_system` cell batches + the isolation guarantees benchmark
  relies on (see scaffold/contracts/benchmark-collections.md; content proposed
  below).
- **benchmark ↔ telemetry (kernel)** via `kernel-confidence` — the
  lowest-confidence query (constraints in, candidate out), ETA estimates for
  CriticalSection `until`, and the refit notify; scheduler's
  `effective_max_concurrent` read is the other half of this stub, owned by
  scheduler/telemetry (see scaffold/contracts/kernel-confidence.md; benchmark
  half proposed below).
- **benchmark ↔ store** via `store-access` — `benchmark_runs` writes
  (covariates + `sample_source='benchmark'` + the additive `collection_id`
  ask), existing-run reads, and the isolation-window queries for batch
  validation (see scaffold/contracts/store-access.md; store.md's proposal
  consumed, benchmark slice noted below).
- **api → benchmark** via `api-dispatch` — `/v1/benchmark/run` → `run_now`,
  `/v1/benchmark/status`; `/v1/benchmark/kernel` deliberately does NOT
  dispatch here (it is telemetry's kernel, api.md concern 7) (see
  scaffold/contracts/api-dispatch.md; benchmark slice proposed below).
- **NON-edges (deliberate):** benchmark ↔ mesh does not exist (fleet pinning
  is `completion-router` + `v1-completion-api`, caller-side — concern 7);
  benchmark ↔ engine does not exist (the scheduler mediates all execution);
  benchmark participates in `restart-protocol` only *through* the parent
  `inference` crate's interruptibility feed (concern 6 — a participation
  refinement, not an owned contract).

## Nesting (if applicable)

Parent: **inference** | Children: none. Module `lib/benchmark`
(`substrate-benchmark`), an internal library of the `inference` app-crate
(INTENT #22); never a standalone crate, never linked across an app boundary
(INTENT #29). The parent wires it in `InferenceService::start` (step 9,
benchmark loop) and maps its in-flight batch into the node's interruptibility
feed.

## Thoroughness level

**implementation-ready.** Decided and specified: the loop architecture with
its eight steps (concern 2); the selection math (predictive variance from the
existing OLS machinery, argmax over a config lattice, the D-optimal
equivalence that makes "uncertainty sampling = greedy info gain" honest); the
dial/covariate split with the three-mechanism pressure coverage and the
rejected synthetic-load option (concern 3); the termination design with its
convergence argument (feasible-region projection + attempt/backoff + σ̂-relative
θ — concern 4); batch-atomic validation with the exact acceptance predicate
(concern 5); the CriticalSection scoping + L2-abort rule + long-batch bound
(concern 6); node-pinned fleet benchmarking as a caller-side loop (concern 7);
and the full ingestion pipeline with producers assigned (concern 8). **Open by
dependency (not gaps here):** telemetry's wave-2 refit must land the
confidence-surface implementation this file queries (the `kernel-confidence`
proposal below is the co-design input — same batch, reconciled in the per-pair
round); the exact spline/variance formulation inside telemetry is telemetry's
to own against this query shape.

## Assigned design-depth

**Fable** (the batch-6 Fable seat per wave2-plan — `benchmark` is one of six
Fable-tier modules in the wave), single Component-Designer pass, grounded in
the real `lib/benchmark`, `lib/telemetry/src/estimator.rs`, and
`lib/scheduler` source plus the batch-1..5 neighbor designs listed in Status.

## Suggested fill-model

**implementation-ready + high complexity → strong model for the loop core,
cheap model for the plumbing.** The carve-outs that need the careful hand:
(1) the **validation predicate** (concern 5) — the isolation-window query and
batch-atomic rejection must be exact, or the kernel silently trains on poisoned
samples (the one failure mode of this whole module that is invisible when it
happens); test-first with the preemption and window-overlap tests below.
(2) the **backoff/termination state machine** (concern 4) — must be proven
against the livelock test, including the all-in-backoff QUIESCENT path.
(3) the **feasible-region construction** (context clip + reachable pressure
bands) — an over-broad region resurrects the livelock, an over-narrow one
starves the kernel. The submission plumbing, store writes, and api dispatch are
near-transcription of existing code. **Sequence with telemetry's refit**
(the confidence surface must exist to query — same batch); scheduler's refit
only re-affirms guarantees the real code already provides, so it does not
block.

---

## Proposed contracts (wave 2)

Proposals only; the per-pair round reconciles both sides. I do NOT edit
`scaffold/contracts/*`. wave2-plan §3a makes benchmark a party to
`benchmark-collections` (initiator — authored fully here), `kernel-confidence`
(benchmark half — telemetry owns the kernel; scheduler owns the
`effective_max_concurrent` read half), `store-access` (consumer slice — store.md
authored the surface; I consume and add one additive ask), and `api-dispatch`
(the benchmark dispatch slice). Shared vocabulary lands in `types`
(`CompletionRequest`/`CollectionRow`/`MetricsFlags` already exist;
`KernelPoint`/`KernelConfidence` proposed for `types::kernel` or as
telemetry-internal — telemetry's call, since this seam is in-process).

### `benchmark-collections` (benchmark → scheduler) — EXISTS; content proposed

- **Purpose.** Benchmark submits priority-0, fully-preemptible, full-system-
  exclusive cell batches through the scheduler, and relies on the scheduler's
  isolation guarantees to make its measurements valid. In-process seam
  (both libs compiled into `inference`), not a wire contract.
- **Submission shape** (grounded in the real `suite.rs` request construction):
  ```rust
  // one cell batch = one CollectionRow { request_full_system: true,
  //   save_partial_results: true, cancel_on_failure: false, state: Active }
  // + `parallelism` CompletionRequests, each:
  CompletionRequest {
      priority: 0,
      preemption_threshold: Some(1),        // ANY real work preempts instantly
      collection_id: Some(batch_collection),
      max_tokens: Some(output_len),          // dial
      prompt: synthetic(input_len),          // dial (~4 chars/token filler)
      temperature: Some(0.0),
      metrics: MetricsFlags::ALL,
      metadata: {"benchmark": true, "cell": {output_len, parallelism, input_len},
                 "target_pressure_band": ...},   // wave-2: selection provenance
      ..
  }
  // wave-2 additions vs the live code: cells come from the active-learning
  // selection (concern 2), never a grid; output_len ranges to 100_000 clipped
  // by model n_ctx; exactly ONE batch in flight per node.
  fn submit(&self, req: CompletionRequest) -> Result<CompletionId>;   // existing
  fn cancel(&self, id: CompletionId) -> Result<()>;                   // L2-abort path (concern 6)
  ```
- **Scheduler guarantees benchmark relies on (the contract's substance):**
  (1) priority-0 work is admitted only when no priority≥1 work is pending or
  running; (2) arrival of any priority≥1 work preempts running benchmark
  completions immediately (`preemption_threshold` honored, `requeue_preempted`
  increments `preemption_count` faithfully — the validation predicate's
  evidence); (3) `request_full_system` collections run with no co-scheduled
  non-member work; (4) preempted benchmark completions re-enter Pending and
  are NOT silently re-run into a corrupted sample — benchmark cancels
  rejected batches rather than letting requeued members re-execute
  (benchmark-side obligation, stated so the scheduler needn't special-case).
- **Error cases.** `SubstrateError::ResourceExhausted(id, n)` on
  max_preemption_count (never reached in practice — benchmark cancels rejected
  batches first); submit against an unloaded model triggers the ordinary swap
  path (a benchmark batch may legitimately cause a model swap on an idle,
  multi-model node — the swap cost is excluded from the measurement window,
  which starts at first-member `Running`).
- **Conformance requirement.** A preemption test: submit a cell batch, inject
  a priority-1 completion mid-batch, assert every benchmark member is
  preempted within one tick, `preemption_count > 0` on each, and the
  submitting benchmark cancels the batch and writes zero rows.
- **Version-sensitivity.** N/A — in-process, one binary (INTENT #45).

### `kernel-confidence` (telemetry.kernel ↔ benchmark) — EXISTS; benchmark half proposed

- **Purpose.** The query surface benchmark's active-learning loop drives:
  "where, within what I can reach, are you least certain?" plus the ETA read
  (CriticalSection `until`) and the refit notify. The stub's other consumer —
  scheduler's `effective_max_concurrent` at the current operating point — is
  scheduler/telemetry's half, deliberately not re-authored here; both halves
  read ONE fitted surface (the stub stays one document, two read patterns).
- **Query surface** (in-process, on telemetry's per-model kernel handle):
  ```rust
  struct KernelPoint {           // full axis vector — the kernel's domain
      input_tokens: u32, output_tokens: u32, parallelism: u32,
      mem_pressure: f32, cpu_utilization: f32, gpu_utilization: f32,
  }
  struct FeasibleRegion {        // benchmark-constructed (concern 3)
      lattice: Vec<DialPoint>,               // context-clipped dial anchors
      reachable_pressure: Vec<PressureBand>, // predicted-from-baseline bands
      excluded: Vec<LatticeKey>,             // backoff table (concern 4)
  }
  struct CandidatePoint {
      dials: DialPoint,
      target_pressure_band: PressureBand,
      confidence: f32,           // σ̂-relative, in [0,1] (concern 4's honesty rule)
      expected_gain: f32,        // predictive variance at the point (concern 2)
  }
  // benchmark -> telemetry:
  fn lowest_confidence(&self, region: &FeasibleRegion) -> Option<CandidatePoint>;
      // None => min confidence >= θ over region (CONVERGED input) — θ is
      // calibrated against residual variance σ̂², not absolute (concern 4)
  fn confidence_at(&self, p: &KernelPoint) -> f32;          // dashboard + tests
  fn estimate(&self, p: &KernelPoint) -> EstimatedDuration; // batch ETA (concern 6);
      // cold-start-safe (existing conservative default path)
  fn notify_new_samples(&self, model: &ModelId);            // cheap refit trigger (concern 8)
  ```
- **Error cases.** None — total functions; cold-start returns the seeded
  low-confidence surface (space-filling degeneration, concern 2); a poisoned
  internal mutex degrades to cold-start semantics (matching the live
  estimator's behavior).
- **Conformance requirement.** (a) Selection steering: with samples dense in
  one lattice region only, `lowest_confidence` returns a candidate *outside*
  it; (b) monotonicity: after `notify_new_samples` with an accepted sample at
  point p, `confidence_at(p)` does not decrease; (c) `lowest_confidence`
  respects `excluded` and `reachable_pressure` (never returns an excluded or
  unreachable candidate) — the termination proof's load-bearing property.
- **Version-sensitivity.** N/A in-process; if `KernelPoint`/confidence ever
  serialize outward (`/v1/benchmark/kernel`, dashboard), that wire shape is
  api/telemetry's and follows guardrail-4 additive discipline.

### `store-access` (benchmark ↔ store) — consuming store.md's proposal; one additive ask

- **Consumed as proposed** (store.md concern 5 / its `store-access` proposal):
  `insert_benchmark_run` / `benchmark_runs_for_model` with the pressure-
  covariate columns (`mem_pressure`, `cpu_utilization`, `gpu_utilization`,
  `gpu_mem_used_bytes`, `is_gpu_estimate`, `sample_source`); benchmark writes
  `sample_source='benchmark'` for accepted batches ONLY (concern 5),
  telemetry writes `'observed'` (concern 3.2 — producer assignment to
  confirm with telemetry).
- **Additive asks (small, flagged for the per-pair round):** (1) a
  **`collection_id` column on `benchmark_runs`** — batch identity for
  post-hoc auditability of the batch-atomic invariant (concern 8); (2)
  covariates recorded as **(mean, peak)** pairs for the pressure fields
  (concern 1) — peak is the stress-test half of INTENT #8; if store prefers
  single columns, mean-only is acceptable and peak rides `metadata_json`.
- **Validation reads benchmark performs:** `count_by_state` (idle gate,
  existing), per-collection member states (batch terminal detection), and a
  **window query** — completions (any priority ≥ 1) with activity inside
  [batch_start, batch_end] — for the isolation check (concern 5); expressible
  over existing rows via `created_at`/state timestamps, no new store surface
  required.

### `api-dispatch` (api → benchmark) — benchmark slice proposed (supersedes the wave-1 shape)

- **Purpose.** The node-internal dispatch for `/v1/benchmark/*`. Supersedes
  the `schedule_if_idle(model_id, queue_empty)` shape api.md transcribed from
  the live code (reconciliation flagged — api.md wrote down what exists;
  wave-2 benchmark replaces it):
  ```rust
  // POST /v1/benchmark/run  (node-pinned via ?node= at the router — concern 7)
  fn run_now(&self, model_id: Option<ModelId>, target: Option<DialPoint>)
      -> Result<CollectionId>;
      // on-demand active-learning session (or one specific candidate),
      // same isolation/validation rules, skips the idle timer; 409-shaped
      // error if a batch is already in flight (one per node, ever)
  // GET /v1/benchmark/status
  fn status(&self) -> BenchmarkStatus;  // { enabled, phase: Idle|Converged|Quiescent|Running{collection_id, cell, eta},
                                        //   per_model: [{model_id, min_confidence, lattice_coverage, backoff_count}] }
  // operator toggle (dashboard action)
  fn set_enabled(&self, enabled: bool);
  ```
- `/v1/benchmark/kernel` does NOT dispatch here — it serves telemetry's
  kernel (api.md concern 7; the local `fit_quadratic_tps` recompute is
  deleted). Benchmark exposes selection/coverage *status*; telemetry exposes
  the *surface*.
- **Conformance.** `run_now` while a batch is in flight returns the
  409-shaped error, never queues a second batch (the one-batch invariant is
  api-visible); `status().phase` transitions Idle→Running→(Converged|Idle)
  observably across a session.
- **Version-sensitivity.** N/A in-process; `BenchmarkStatus` serializes to
  the dashboard via api → additive-only, `#[serde(default)]` per guardrail 4.

### `restart-protocol` — participation refinement (not an owned contract)

Inference.md's interruptibility mapping (benchmark sweep ⇒
`CriticalSection{until}`) is consumed and refined per concern 6: scope =
in-flight batch only, `until` = kernel ETA, L2+ ⇒ cancel-batch → discard
(no attempt-count penalty) → yield at next completion boundary. This is an
input to the per-pair `restart-protocol` round via the parent `inference`
crate; benchmark owns no frames.
