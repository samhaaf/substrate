# telemetry

## Charter
`telemetry` is two things in one crate (`lib/telemetry`, modules `sampler`,
`estimator`): a `SystemSampler` that polls CPU/memory/GPU via `sysinfo` and
publishes the `SystemState` snapshots the scheduler and api consume, and a
`ThroughputEstimator` — the node's **throughput kernel** — that fits a model over
observed completions to predict duration with a confidence annotation. Its
boundary: it MEASURES the machine and MODELS its throughput; it does not schedule
(`scheduler` consumes its output) or run work. This crate is the natural home of
the multidimensional kernel the redesign calls for.

## Primary design concerns
- **The kernel goes multidimensional (redesign, core impact).** Today the
  estimator fits a degree-2 polynomial over `(input_tokens, output_tokens,
  parallelism) -> duration_ms` with a *binary* in-envelope/extrapolated
  confidence, and the api recomputes a separate 1-D `output_tokens` curve for
  `/v1/benchmark/kernel` (parallelism=1 only). The reshape unifies these into ONE
  kernel whose axes are **output-length, parallel-completion-count, and observed
  memory/CPU/GPU pressure** as full axes (input-length retained), producing not
  just a point estimate but a **per-region confidence interval** — i.e. "how well
  do I know throughput at THIS operating point." This kernel is the single source
  of truth for `/v1/estimate`, the dashboard curve, the scheduler's
  `effective_max_concurrent`, and the benchmark's adaptive test selection.
- **Confidence surface is a first-class output.** The binary
  `InEnvelope`/`Extrapolated` flag is replaced by a queryable confidence surface
  (lowest-confidence region locator) so `benchmark` can ask "where are you least
  certain?" — the substance of the new `kernel-confidence` edge. This is the part
  that makes the estimator genuinely harder than the current OLS fit (variance
  estimation / CI over a fitted surface, not just a mean).
- **Kernel siting is a real choice (FLAG).** The task named benchmark+scheduler
  as the crates the redesign touches; this design places the kernel MODEL in
  `telemetry` (its current home, least-surprising) and makes benchmark/scheduler
  its consumers. An alternative is promoting the kernel to a small shared
  library. Flagged as an open question rather than silently relocated.
- **VRAM telemetry is stubbed at 0 (mesh-design #5).** GPU utilization is real on
  macOS via `ioreg`, but VRAM is stubbed — a real pressure axis for the kernel
  depends on filling this in, so the two are co-dependent.

## Relationships / edges
- telemetry -> {scheduler, api} via `system-state` (see scaffold/contracts/system-state.md)
- telemetry (kernel) <-> {benchmark, scheduler} via `kernel-confidence` (see scaffold/contracts/kernel-confidence.md)
- telemetry <-> store via `store-access` (observations / benchmark-run training data; see scaffold/contracts/store-access.md)

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
approach-sketched — the sampler is implementation-ready; the multidimensional
confidence-aware kernel is a genuine redesign, sketched with a recommended home
and axis set but not fully specified (the CI/variance method is deliberately
left for the Filler/Design-Mesh).

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading `lib/telemetry/src/estimator.rs`
in full and the `/v1/benchmark/kernel` handler.

## Suggested fill-model
approach-sketched + high complexity (the kernel) -> **strong model or a Design
Mesh pass on the confidence-surface math**, jointly with `benchmark` and
`scheduler`. The sampler half is cheap; the kernel half is the crate's hard core.
