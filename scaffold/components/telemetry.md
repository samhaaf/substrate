# telemetry

**Status:** WAVE-2 REFIT of the wave-1 `telemetry.md` (approach-sketched). The
wave-1 sampler is **preserved and re-affirmed** (real code: `lib/telemetry/src/
sampler.rs`, ioreg GPU sampling with `is_estimate`); wave-2 makes this crate
**THE KERNEL'S HOME** per the batch-5 `inference.md` decision, and delivers the
single hard thing the redesign demands: **one multidimensional, confidence-aware
throughput kernel** that supersedes the two divergent estimators shipping today
(this crate's degree-2 `ThroughputEstimator` over `(input, output, parallelism)`
and `api`'s hand-rolled 1-D quadratic `fit_quadratic_tps` at `concurrency==1`,
`rest.rs:696`). Grounded in the REAL code (`lib/telemetry/src/{lib,sampler,
estimator}.rs`, `lib/api/src/rest.rs` `benchmark_kernel`/`estimate`,
`lib/benchmark/src/suite.rs`), `types::system::SystemState`, and the batch-5/6
neighbor designs `store.md` (the pressure-covariate columns + `sample_source`),
`benchmark.md` (adaptive lowest-confidence selection; pressure as observed
covariate; active∪passive training set), `scheduler.md` (kernel-driven admission /
`effective_max_concurrent`), `api.md` (delete the local fit → dispatch to
telemetry's kernel; the `/v1/estimate` warm-kernel seam), and `types.md`
(`SystemState`, `Honesty`, `Provenance`). INTENT #8/#9/#12/#22/#29/#38/#46/#85.

## Charter

`telemetry` is two boring things compiled into the `inference` node
(`lib/telemetry`, modules `sampler`, `kernel` [superseding `estimator`]): (1) a
**`SystemSampler`** that polls CPU / memory / GPU pressure via `sysinfo` + `ioreg`
and produces the `types::system::SystemState` snapshots `scheduler` and `api`
read; and (2) the node's **throughput kernel** — one fitted model, per `(node,
model)`, over the operating space **output-length × parallel-completion-count ×
input-length × observed {memory, CPU, GPU} pressure → per-completion latency**,
carrying a **per-region confidence field** ("how well do I know throughput at THIS
operating point"). The kernel is the **single source of truth** for four
consumers: `/v1/estimate` (a point prediction), the dashboard curve (a slice of
the surface), the `scheduler`'s effective max-concurrency at admission, and the
`benchmark` sweep's next-test selection ("where is confidence lowest?").

**Boundary — what telemetry does NOT own.** It **measures and models**; it does
not **schedule** (`scheduler` consumes `SystemState` + the kernel and decides what
runs), does not **choose or execute measurements** (`benchmark` picks the next
test, `scheduler`/`engine` run it — telemetry only says where it is least
certain), does not **persist raw samples of record** (its training data lives in
`store.benchmark_runs`; telemetry keeps only an in-memory fitted cache), and does
not **speak the wire**: it never registers with mesh, never publishes pub/sub or
participates in the restart protocol — `api` owns the node's single external
surface and serializes the kernel on telemetry's behalf. It is a **compiled-in
library of `inference`** (INTENT #22/#29), reached in-process by its siblings; it
is never a standalone crate and never linked across an app boundary.

## Primary design concerns

The wave-1 sampler concerns are preserved. The concerns below are the wave-2
kernel redesign — this crate's hard core, and the reason it is a distinct
component rather than a helper inside `scheduler` or `benchmark`.

### 1. ONE kernel replaces two divergent estimators — unify at the model, not the caller

Two fits ship today and they disagree by construction:

- **`ThroughputEstimator`** (`estimator.rs`): a **single global** degree-2 OLS
  polynomial over `(input, output, parallelism) → duration_ms`, 10 coefficients,
  refit on every `estimate()` from an **in-memory, non-persisted** `Vec<DataPoint>`
  (lost on restart), with a **binary** `InEnvelope`/`Extrapolated` confidence
  (convex-hull membership, `estimator.rs:154`). No pressure axes.
- **`api::benchmark_kernel`** (`rest.rs:546`): a **separate** hand-rolled 1-D
  quadratic `tps = a + b·x + c·x²` over `output_tokens` **at `concurrency==1`
  only** (`rest.rs:571`), Cramer's-rule least squares, for the dashboard curve.

Wave-2 collapses both into **one `Kernel` per `(node, model)`**, queried at an
operating point `q`. The unification rule is **at the model, not the caller**:
every consumer reads the *same* fitted surface, and each "view" is a projection —
`/v1/estimate` is a point query at `q`; the dashboard 1-D curve is the surface
sliced at `parallelism = 1`, `input = current`, `pressure = current`; the
multi-model overlay (INTENT #8, "same widget, you just expand it") is one query
per model. `api`'s `fit_quadratic_tps` and its in-handler fit are **deleted**
(api.md concern 7); `api` dispatches to telemetry (`kernel_surface()` /
`estimate()`), holding no math. This is the concrete "one source of truth" the
task names.

### 2. The model family — locally-weighted regression with an explicit confidence field (the genuinely-harder part)

A single global OLS polynomial **cannot** answer "where is confidence lowest" —
it has no local notion of support, and a degree-2 surface over 6 axes both
underfits real throughput cliffs and gives a confident-looking extrapolation
everywhere. The redesign's confidence surface is what makes this a real model, so
the recommended family (committed here, with the bandwidth choice flagged for the
fill) is **kernel-weighted local regression + a support-and-variance confidence
field** — boring, honest, and O(n) per query at the sample volumes involved (a
few thousand rows per model). Rejected alternatives are recorded in Controversial
decisions (global OLS: no local confidence; full Gaussian Process: O(n³),
hyperparameter-fragile, not "boring").

Define, per `(node, model)`, over the **normalized** axis space (each axis mapped
to `[0,1]`: output/input log-scaled over `[100, 100k]`, parallelism over
`[1, P_max]`, the three pressures over `[0,1]`):

- **Mean prediction `μ(q)`** — weight every training sample `i` by a tricube
  kernel `w_i = K(‖q − x_i‖_A / h)` over the per-axis-scaled distance (`A` =
  diagonal axis weights, `h` = bandwidth), then fit a **local low-degree
  polynomial** (linear, quadratic where support allows) on the weighted set.
  Inside the observed hull this interpolates (quadratic — matching the operator's
  "quadratic spline inside the range", INTENT #8); outside, the local fit degrades
  to the nearest-support **linear trend** ("linear outside", INTENT #8). The
  canonical target is **per-completion latency (`duration_ms`)** — it is what
  `record()`/`estimate()` already predict and what admission needs under load;
  **tokens/sec is a derived view** (`output_tokens / (μ/1000)`) for the dashboard.
- **Confidence field `c(q) ∈ [0,1]`** — replaces the binary flag. From two honest
  signals: local **support** `N_eff(q) = Σ w_i` and local **residual variance**
  `σ²(q)` (weighted residuals of the local fit). Recommended form
  `c(q) = (1 − e^{−N_eff/N₀}) · 1/(1 + σ²(q)/σ²_ref)` — low where data is sparse
  OR noisy, saturating toward 1 with dense, consistent support. `N₀`, `σ²_ref`,
  `h`, and `A` are the fitted/tuned knobs (carve-out below).
- **Estimate-honesty taint `is_estimate(q)`** (concern 4) — `true` when the
  support at `q` is dominated by samples whose covariates were themselves
  estimates (GPU `is_gpu_estimate = true`, or unknown-VRAM), so the confidence and
  any dashboard render carry the tilde even when `N_eff` is high. Honesty is a
  property of the *data the prediction rests on*, not just of the current reading.

This is the substance that makes telemetry a modeling component and not an OLS
call: a queryable variance/support surface, not a mean.

### 3. Pressure axes are OBSERVED covariates — the training set is active ∪ passive

You cannot *set* memory/CPU/GPU pressure the way you set output-length or
parallelism, so the kernel treats them as **covariates recorded at sample time**,
never as controllable dials (benchmark.md agrees). The training set is the
**union** of two populations, both landing in `store.benchmark_runs` with the
`sample_source` discriminator store.md added:

- **Active / `'benchmark'`** — controlled, isolated, low-pressure points from the
  priority-0 sweep (`benchmark` sets output/parallelism/input; pressure is
  incidentally low because sweeps run only when idle).
- **Passive / `'observed'`** — real completions at whatever pressure the node
  happened to be under; the *only* way the kernel sees the high-pressure regime.
  telemetry records these — see the **confidence-gated passive recording** below.

Each row carries the pressure covariates store.md defined (`mem_pressure`,
`cpu_utilization`, `gpu_utilization`, `gpu_mem_used_bytes`, `is_gpu_estimate`)
sampled from `SystemState` at the moment the point completed. The kernel fits over
active ∪ passive as one set; `sample_source` is retained so a consumer (or the
confidence field) can down-weight or partition the two populations if the operator
later wants isolation-vs-load separation. This resolves benchmark.md's flagged
isolation-vs-pressure-exploration tension: benchmark supplies the clean
low-pressure spine, organic traffic supplies the load regime, one kernel spans
both.

**Confidence-gated passive recording (volume control, elegant closure).** Writing
a `benchmark_runs` row per completion would grow the table without bound. Instead
telemetry records a passive `'observed'` point **only when the completion's
operating point falls in a region below the confidence threshold** — i.e. only
when it carries information the kernel lacks. This bounds volume, auto-targets the
under-explored pressure regimes benchmark structurally cannot reach, and closes
the same active-learning loop `benchmark` runs on the controllable axes. Retention
of `'observed'` rows (a per-model cap / gc-managed TTL so old load points age out)
is flagged to store/gc as a follow-up (friction).

### 4. VRAM telemetry honesty — never fabricate a zero (INTENT #9, `is_estimate` discipline)

Today the sampler reports `gpu_memory_used_bytes = 0` and `gpu_memory_total_bytes
= 0` **unconditionally** (`sampler.rs:196`) — a hardcoded zero that is
indistinguishable from a real "no VRAM in use" reading, which is exactly the
dishonesty INTENT #9 forbids. GPU *utilization* is honestly handled (`ioreg`,
`is_gpu_estimate` flags the estimate) but VRAM is not. Wave-2 fixes the
representation, three parts:

- **Never emit a fabricated VRAM number.** Unknown VRAM must be representable as
  *unknown*, not `0`. This requires a `types::system::SystemState` change (the
  one cross-crate ripple): either `gpu_memory_{used,total}_bytes: Option<u64>` or
  a companion `gpu_mem_known: bool`. Recommended: **`Option<u64>`** (a `None` is
  unambiguously "not measured"; `#[serde(default)]` keeps it additive for
  mixed-version rollout). Flagged to `types` (friction — types.md keeps
  `SystemState`'s home but did not touch this field).
- **Apple-Silicon unified memory is the honest special case.** On Apple Silicon
  there is no separate VRAM pool — GPU memory *is* system memory. So on that
  platform telemetry does **not** invent a VRAM figure; it leaves
  `gpu_memory_*_bytes = None`, and the **GPU-memory-pressure axis folds onto
  `memory_pressure`** (with `is_gpu_estimate = true` marking the whole GPU story as
  estimated). This is a real, defensible model, not a stub. Discrete GPUs (future
  `nvml` / `rocm-smi`, sampler.rs's own "Future" note) report real VRAM and set
  `is_gpu_estimate = false`.
- **The kernel respects unknown.** When the GPU-memory axis is `None`, the kernel
  **drops that axis for the fit** (rather than fitting against a fake 0) and widens
  confidence accordingly; when `is_gpu_estimate = true`, samples carry the honesty
  taint (concern 2) so predictions leaning on them render the tilde. The dashboard
  `SurfaceField.honesty = Estimate{note}` (api.md concern 6, `types::surface::
  Honesty`) is driven by this flag end-to-end.

The store already persists `is_gpu_estimate` per `benchmark_runs` row (store.md
concern 5) precisely so the kernel never fits dishonest GPU readings as if real —
this concern is the producer side of that honesty flag.

### 5. Kernel persistence — samples in `store`, the fit is a rebuilt in-memory cache (resolves "store columns? own file?")

The task asks where the kernel lives on disk. Decision: **no own file, no
persisted model blob.** The kernel is a **pure function of its training samples**,
and the samples already have a durable home — `store.benchmark_runs` (columns, not
a new file; store.md concern 5 added the pressure covariates + `sample_source`).
So:

- **Training data = `store.benchmark_runs`** (active ∪ passive rows), read via
  `store-access`. There is exactly one training table; telemetry adds no schema.
- **The fitted `Kernel` is an in-memory cache** (per `(node, model)`), **rebuilt
  from `store` on boot** and refreshed incrementally as new rows are recorded.
  Persisting fitted coefficients would create a second source of truth and a
  staleness bug (coefficients drifting from the samples they came from) — rejected
  as debt (INTENT #38).
- **Warm-on-boot fixes a live bug.** `api::estimate` today news up a **fresh empty
  `ThroughputEstimator`** per call (`rest.rs:451`, "results are always cold-start
  estimates"). Wave-2: telemetry loads `benchmark_runs` at `Telemetry::new`/first
  use, so `/v1/estimate` is **warm from the first request after a restart** — a
  correctness gain, not just a refactor. `api` dispatches to the shared warm
  `Arc<Telemetry>` kernel (api.md concern 8's warm-estimator seam), never a local
  instance.

So: **store columns, no own file, no persisted model.** Clean and boring.

### 6. Two kernel consumers, two questions, one surface — the `kernel-confidence` contract

The `kernel-confidence` edge has two asymmetric consumers; telemetry answers both
off the one fitted surface:

- **`benchmark` asks "where is confidence LOWEST?"** — telemetry evaluates `c(q)`
  over the **controllable** axis grid (output × parallelism × input) and returns
  the argmin cell as a `KernelRegion` plus the under-explored **pressure regime**
  tag (benchmark cannot set pressure, so it is told *which* regime is thin so it
  can opportunistically sample there — or defer to passive capture, concern 3).
  The stop condition becomes "confidence everywhere ≥ threshold," replacing
  benchmark's fixed Cartesian grid (`suite.rs` `OUTPUT_LENGTHS × PARALLELISM_LEVELS
  × INPUT_LENGTHS`, `target_samples_per_cell = 3`).
- **`scheduler` asks "effective max concurrency HERE?"** — given the current
  operating point (pending output-mix + current pressure from `SystemState`),
  telemetry scans the parallelism axis and returns the largest `p` where predicted
  aggregate throughput still gains (`ΔT(p) ≥ ε`) **and** `c(q_p) ≥ floor`; below
  the confidence floor it returns `Unconfident` so the scheduler **falls back to
  its existing scalar memory-pressure heuristic** (scheduler.md's soft-band
  admission) — graceful degradation, never a confident guess on no data. The
  target policy (latency ceiling vs throughput-knee) is a scheduler co-design knob
  (open question).

`effective_max_concurrent` **placement** is a genuine reconciliation with
scheduler.md/system-state.md (both imply it as a `SystemState` field). Decision:
it is **primarily a `kernel-confidence` query** (it is a *fitted-model output* at
a chosen policy + operating point, not a raw measurement — putting it on the raw
sample conflates measured with modeled and goes stale when the kernel refits).
telemetry **also stamps a convenience scalar** onto `SystemState` each tick,
computed at the current operating point, so the scheduler's common-case tick reads
it with zero extra calls — but the authoritative, what-if-capable, confidence-
bearing answer is the kernel query. Flagged (controversial) for the per-pair round.

## Relationships / edges

- **telemetry → {scheduler, api}** via `system-state` *(EXISTS — content proposed
  below; telemetry is the producer/owner)* — `SystemState` snapshots + the
  convenience `effective_max_concurrent` scalar (concern 6).
- **telemetry (kernel) ↔ {benchmark, scheduler}** via `kernel-confidence` *(EXISTS
  — content proposed below; telemetry owns the kernel)* — the lowest-confidence-
  region query (benchmark) and the effective-concurrency query (scheduler)
  (concern 6).
- **telemetry ↔ store** via `store-access` *(store owns it — store.md authored;
  telemetry is a CONSUMER)* — reads `benchmark_runs` (active ∪ passive training set
  incl. the pressure covariates + `sample_source`, store.md concern 5); records
  confidence-gated passive `'observed'` rows (concern 3). In-process, compiled-in,
  NOT a wire contract. Participation note below.
- **api → telemetry** via `api-dispatch` *(api owns it — api.md authored;
  telemetry is a dispatch TARGET)* — `current_state()`, `kernel_surface()` (serves
  `/v1/benchmark/kernel`, replacing api's deleted local fit), `estimate()` (serves
  `/v1/estimate` from the warm kernel). Reference only.

**NON-edges (deliberate, flagged so the harmonizer does not invent them):**
- **telemetry ↔ mesh** does **not** exist. telemetry never registers, resolves,
  publishes pub/sub, or participates in restart — `api`/`inference` own the node's
  external surface and mesh participation; telemetry is reached only in-process.
- The **`ioreg` subprocess** the sampler execs is a local platform call, not a
  mesh edge and not a contract.

## Nesting

Parent: **inference** | Children: none. Module `lib/telemetry`
(`substrate-telemetry`), modules `sampler` (system sampling + GPU/VRAM honesty),
`kernel` (the multidimensional model + confidence field — **supersedes**
`estimator`; a thin `estimator` re-export is kept for the `/v1/estimate` call
shape during transition). An internal library of the `inference` app-crate (INTENT
#22: not an app → a lib, nested under the app that uses it); never standalone,
never linked across an app boundary (INTENT #29).

## Thoroughness level

**implementation-ready** for: the sampler (unchanged real code); the
two-estimator-unification decision and the delete-api's-fit direction (concern 1);
the pressure-as-covariate training model and the active∪passive union with
confidence-gated passive recording (concern 3); the VRAM-honesty representation +
Apple-Silicon unified-memory folding + kernel-respects-unknown (concern 4); the
persistence answer (store columns, rebuilt in-memory cache, warm-on-boot bug fix,
concern 5); and the two-consumer `kernel-confidence` query shapes + the
`effective_max_concurrent` placement reconciliation (concern 6). **approach-
sketched** for the one deliberately-deferred piece: the **exact confidence-field
math** (concern 2) — the model *family* (kernel-weighted local regression + the
`c(q)` form) and its axes are committed, but the bandwidth `h`, axis weights `A`,
`N₀`/`σ²_ref` tuning, and the local-degree selection are left for the fill /
Design-Mesh, since they are a fit-quality tuning problem best bought down against
real benchmark data, not paper-specified.

## Assigned design-depth

**Opus 4.8**, single Component-Designer pass (this file), grounded in the real
`lib/telemetry/src/{lib,sampler,estimator}.rs`, `lib/api/src/rest.rs`
(`benchmark_kernel` / `estimate` / `fit_quadratic_tps`), `lib/benchmark/src/
suite.rs`, `types::system::SystemState`, and the batch-5/6 neighbor designs
`store.md`, `benchmark.md`, `scheduler.md`, `api.md`, `inference.md`, `types.md`.

## Suggested fill-model

**implementation-ready for the plumbing → mid model OK** (the sampler-honesty
change, the store-backed warm kernel, the api-fit deletion, the two query
entry-points are near-transcription against frozen seams), with **one carve-out
for a strong model or a Design Mesh pass jointly with `benchmark` + `scheduler`:
the confidence-field math** (concern 2) — the `c(q)` support/variance form, the
bandwidth/axis-weight tuning, the lowest-confidence-region search, and the
effective-concurrency scan are the crate's hard core and the shared output surface
of the whole kernel triangle. Fill telemetry, benchmark, and scheduler together
with one agreed kernel definition; do not let three fillers invent three
confidence notions.

---

## Proposed contracts (wave 2)

Proposals only; the per-pair round reconciles both sides. I do NOT edit
`scaffold/contracts/*`. wave2-plan §3a assigns telemetry two owned pairs —
**`system-state`** (telemetry → {scheduler, api}) and **`kernel-confidence`**
(telemetry ↔ {benchmark, scheduler}). `store-access` (store-owned) and
`api-dispatch` (api-owned) get **participation notes** only. All shared vocabulary
lives in `types` (`SystemState` in `types::system`; `Honesty` in `types::surface`;
`Provenance` in `types::provenance`). These are **in-process Rust API surfaces**
(telemetry is compiled into `inference`), not serialized wire contracts — the
"schema" is the trait/method surface the Skeleton Builder freezes; there is no
cross-node deserialization and thus no wire-version skew (the whole `inference`
crate revs as one binary — INTENT #45).

### `system-state` (telemetry → {scheduler, api}) — EXISTS; content proposed

- **Purpose.** telemetry produces the node's point-in-time
  `types::system::SystemState` (resource pressures + scheduling counts + resident
  model), read by `scheduler` each admission tick and by `api` for
  `GET /v1/system/state` (`node-state-poll`) and the surface `system` section.

- **Surface (in-process; the sampler + the async accessor):**
  ```rust
  // lib/telemetry/src/lib.rs — unchanged accessor
  fn current_state(&self) -> SystemState;                 // cheap RwLock clone
  // WAVE-2 additions to types::system::SystemState (all #[serde(default)], additive):
  //   gpu_memory_used_bytes:  Option<u64>   // was u64=0; None = not measured (concern 4)
  //   gpu_memory_total_bytes: Option<u64>   // was u64=0; None on unified memory
  //   effective_max_concurrent: Option<u32> // convenience scalar, kernel-computed
  //                                          // at the current operating point (concern 6)
  // is_gpu_estimate: bool  (already present) — true = GPU covariates are estimated
  ```

- **Field discipline (concern 4).** The GPU-memory fields become `Option<u64>` so
  an unknown VRAM is `None`, **never a fabricated `0`** (INTENT #9). On Apple
  Silicon (unified memory) they stay `None` and the GPU-memory-pressure story
  folds onto `memory_pressure` with `is_gpu_estimate = true`. `scheduler`/`api`
  treat `None` as "no independent VRAM signal," not as zero pressure.

- **`effective_max_concurrent` (concern 6).** Stamped each tick from the kernel at
  the current operating point (current pressure + pending output-mix). `None` when
  the kernel's confidence there is below floor → the scheduler uses its scalar
  memory-pressure heuristic (graceful degradation). The authoritative, confidence-
  bearing, what-if form is `kernel-confidence` (below); this scalar is the
  zero-call common-case read. **Reconciliation flagged** with scheduler.md/system-
  state.md (which imply it as a plain field): telemetry offers both, and pins the
  scalar as best-effort/optional.

- **Error cases.** None — `current_state()` always returns the last good snapshot;
  a poisoned sampler mutex degrades to a zeroed snapshot (real code
  `sampler.rs:210`), never panics. A snapshot taken mid-swap is internally
  consistent (single RwLock).

- **Version-sensitivity.** In-process, single-build → no wire skew. The
  `SystemState` field additions are additive (`#[serde(default)]`) and matter only
  when the struct crosses the mesh via `api`'s `node-state-poll`/surface (api owns
  that serialization; the honesty flag rides along).

### `kernel-confidence` (telemetry [kernel] ↔ {benchmark, scheduler}) — EXISTS; content proposed

- **Purpose.** The query surface over the one multidimensional throughput kernel
  (concern 6): `benchmark` asks for the lowest-confidence region to test next;
  `scheduler` asks for the effective max concurrency at the current operating
  point. Distinct from `system-state` (raw snapshots, not the fitted surface).

- **Vocabulary (kernel operating point + outputs):**
  ```rust
  // an operating point in the kernel's axis space
  struct OperatingPoint {
      output_tokens: u32,
      parallelism:   u32,
      input_tokens:  u32,
      mem_pressure:  f32,           // observed covariate (0..1)
      cpu_utilization: f32,
      gpu_utilization: Option<f32>, // None where GPU is unmeasured (concern 4)
  }
  // a point prediction with honest confidence (supersedes estimator::EstimatedDuration)
  struct KernelEstimate {
      duration_ms:       u64,       // canonical target (per-completion latency)
      tokens_per_second: f64,       // derived view (output_tokens / duration_s)
      confidence:        f32,       // c(q) in [0,1] (concern 2) — replaces the binary flag
      is_estimate:       bool,      // honesty taint: support leans on estimated covariates
  }
  ```

- **Surface (on the kernel, per `(node, model)`):**
  ```rust
  // point query — serves /v1/estimate (via api-dispatch) and the mean surface
  fn estimate(&self, model: &ModelId, q: OperatingPoint) -> KernelEstimate;

  // BENCHMARK consumer: "where is confidence lowest?" — argmin c(q) over the
  // CONTROLLABLE grid, plus the under-explored pressure regime tag (concern 6)
  fn lowest_confidence_region(&self, model: &ModelId) -> Option<KernelRegion>;
  struct KernelRegion {
      output_tokens: u32, parallelism: u32, input_tokens: u32,   // the cell to test
      confidence: f32,                                           // current c there
      underexplored_pressure: Option<PressureRegime>,           // e.g. High — benchmark can't set it
      expected_information_gain: f32,                            // (1 - confidence)-weighted proxy
  }
  // stop condition: None when confidence >= threshold everywhere (replaces the fixed grid)

  // SCHEDULER consumer: "effective max concurrency here?" (concern 6)
  fn effective_concurrency(&self, model: &ModelId, at: OperatingPoint) -> ConcurrencyAdvice;
  enum ConcurrencyAdvice {
      Confident { max_concurrent: u32, confidence: f32 }, // largest p with ΔT>=ε and c>=floor
      Unconfident,                                        // below floor -> scheduler falls back
  }

  // DASHBOARD view (via api kernel_surface()): the multidim surface + a 1-D slice
  fn kernel_surface(&self, model: &ModelId) -> KernelSurface; // curves + confidence + honesty markers
  ```

- **Training-data source (concern 3/5).** The kernel is fit from
  `store.benchmark_runs` (active `'benchmark'` ∪ passive `'observed'`) read via
  `store-access`; the fitted surface is an in-memory cache rebuilt on boot and
  refreshed on new rows. `lowest_confidence_region` returns controllable cells;
  the pressure regime is *reported*, not commanded (benchmark cannot set it).

- **Error cases / non-errors.** A never-benchmarked model → `estimate` returns a
  conservative cold-start `KernelEstimate { confidence: 0, is_estimate: true }`
  (superseding `cold_start_estimate`, `estimator.rs:203`), `lowest_confidence_
  region` returns the whole space as the first cell, `effective_concurrency`
  returns `Unconfident` (scheduler falls back). `is_estimate = true` on any
  prediction whose support is estimate-dominated (concern 4) is **not** an error —
  it is the honesty signal (INTENT #9) the dashboard renders as a tilde.

- **Version-sensitivity.** In-process, single-build → none. The three consumers
  (`benchmark`, `scheduler`, `api`) recompile with telemetry as one `inference`
  binary; the confidence field's exact math (concern 2) can change without a
  contract break since only the query shape above is frozen.

### `store-access` (telemetry ↔ store) — participation note (telemetry is a CONSUMER; store owns the contract)

- telemetry **reads** `store.benchmark_runs` (`benchmark_runs_for_model`) for the
  active ∪ passive training set, relying on store.md's wave-2 pressure-covariate
  columns (`mem_pressure`, `cpu_utilization`, `gpu_utilization`,
  `gpu_mem_used_bytes`, `is_gpu_estimate`) and `sample_source` (concern 3/5).
- telemetry **writes** confidence-gated passive `'observed'` rows via
  `insert_benchmark_run` (concern 3) — a `BenchmarkRun` with `sample_source =
  'observed'` and the pressure covariates sampled from `SystemState` at completion
  time. **Friction flagged to store/gc:** `'observed'` rows need a retention cap /
  gc-managed TTL so `benchmark_runs` does not grow unbounded (the confidence gate
  bounds the rate; retention bounds the total).
- telemetry does **not** implement `StoreObserver` for the kernel path (it pulls on
  fit/refresh, does not fire in the writer mutex) — it must obey store.md concern
  2's reentrancy rules only if it ever registers an observer; it does not this wave.

### `api-dispatch` (api → telemetry) — participation note (telemetry is a TARGET; api owns the contract)

- telemetry exposes `current_state() -> SystemState`, `estimate(model, q) ->
  KernelEstimate` (serves `POST /v1/estimate` from the **warm** kernel, fixing the
  cold-start bug `rest.rs:451`, concern 5), and `kernel_surface(model) ->
  KernelSurface` (serves `GET /v1/benchmark/kernel`, **replacing** api's deleted
  `fit_quadratic_tps`, api.md concern 7). The route shapes stay; only the source
  moves into telemetry. api adds no math (its charter).
