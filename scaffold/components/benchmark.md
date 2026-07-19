# benchmark

## Charter
`benchmark` is the throughput-characterization orchestrator (`lib/benchmark`,
modules `lib`, `suite`): it detects idle periods and submits priority-0,
`request_full_system` sweep collections through the scheduler to measure
throughput, guaranteeing measurement isolation (benchmarks run only when
otherwise idle and are preempted instantly by any priority≥1 work). Its
boundary: it decides WHICH measurements to take and submits them; it does not
schedule/execute them (`scheduler`/`engine`) or own the fitted model
(`telemetry`'s kernel) — it FEEDS that kernel and queries it to decide what to
measure next. It is the data source the kernel is trained on.

## Primary design concerns
- **Fixed grid → adaptive, confidence-driven selection (redesign, core impact).**
  Today `BenchmarkSuite` enumerates a fixed Cartesian grid
  (`OUTPUT_LENGTHS × PARALLELISM_LEVELS × INPUT_LENGTHS`) and tops up any cell
  below `target_samples_per_cell` (=3). The reshape replaces the fixed grid with
  **adaptive selection driven by the kernel's confidence**: instead of a static
  grid, benchmark queries telemetry's kernel for the region of
  (output-length × parallel-completion-count × memory/CPU/GPU pressure) space
  where confidence is currently LOWEST and queues the next test there. The stop
  condition becomes "confidence everywhere above threshold," not "every cell has
  N samples." This is the substance of the new `kernel-confidence` edge and the
  reason this crate is redesigned rather than kept as-is.
- **Pressure axes are observed covariates, not dials.** You cannot directly set
  memory/CPU/GPU pressure the way you set output-length or parallelism — so the
  adaptive loop must record the pressure *observed at sample time* (from
  telemetry's `SystemState`, persisted onto the benchmark-run row — see the
  `store` pressure-covariate columns) and opportunistically sample when the
  system happens to be in an under-explored pressure regime. This is the
  non-obvious hard part: a partially-controllable design space.
- **Isolation vs. pressure exploration tension.** Benchmarks run only when idle
  (low pressure) for isolation, yet the kernel needs samples across the pressure
  range — which mostly occurs under real load. Recommended resolution (flagged):
  benchmark contributes controlled low-pressure points; the kernel also ingests
  passively-observed real-completion points at higher pressure. This makes the
  kernel's training set a union of active (benchmark) and passive (organic)
  observations — a design choice worth the operator's eye.

## Relationships / edges
- benchmark -> scheduler via `benchmark-collections` (see scaffold/contracts/benchmark-collections.md)
- benchmark -> telemetry (kernel) via `kernel-confidence` (query lowest-confidence region; see scaffold/contracts/kernel-confidence.md)
- benchmark <-> store via `store-access` (benchmark runs + pressure covariates; see scaffold/contracts/store-access.md)
- api -> benchmark via `api-dispatch` (`/v1/benchmark/run`, `/v1/benchmark/kernel`; see scaffold/contracts/api-dispatch.md)

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
approach-sketched — the idle-gating/preemption mechanics are grounded and
implementation-ready, but the adaptive confidence-driven selection loop (and the
active/passive-observation union) is a genuine redesign, sketched with a
direction and the key tension surfaced, not fully specified.

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading `lib/benchmark/src/suite.rs`
and `lib/benchmark/src/lib.rs` in full.

## Suggested fill-model
approach-sketched + high complexity -> **strong model or a Design Mesh pass**,
jointly with `telemetry` (owns the confidence surface) and `scheduler` (consumes
`effective_max_concurrent`). Fill these three together; the adaptive-selection
policy is where the design effort is not yet fully bought down.
