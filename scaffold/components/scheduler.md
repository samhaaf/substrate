# scheduler

## Charter
`scheduler` is the central scheduling loop (`lib/scheduler`, modules `queue`,
`admission`, `selection`, `swap`): queue management, admission control, model-
swap evaluation, preemption, crash recovery, and pluggable selection/swap
policies. Each tick it reads `SystemState`, classifies memory pressure, decides
whether to swap the resident model (draining + swapping through `engine-exec`),
then admits pending work up to a dynamically-lowered concurrency target. Its
boundary: it decides WHAT runs and WHEN on this node; it does NOT execute
(`engine`), does NOT sample the system (`telemetry`), and does NOT persist
(`store`) — it orchestrates those.

## Primary design concerns
- **The admission target varies under pressure (mesh-design finding #3).**
  Today `AdmissionController` reduces the concurrency target proportionally in
  the soft-pressure band (0.90–0.95) and admits nothing above hard. This is the
  crate's hardest logic and the reason it's a distinct component.
- **Kernel-driven admission (redesign impact).** The reshape wants admission to
  consult the multidimensional throughput *kernel* rather than only a scalar
  memory-pressure fraction: the effective max-concurrency for the *current*
  operating point (output-length mix × parallelism × observed CPU/GPU/memory
  pressure) should come from the kernel's fitted surface, and be published as
  `effective_max_concurrent` on `SystemState` (today stubbed/absent — blocks
  spill, OQ-2). This makes the scheduler a **consumer** of the kernel, not its
  owner. New read edge to telemetry's kernel surface (see `kernel-confidence`);
  note the scheduler already depends on `telemetry` via `system-state`, so this
  is an extension of an existing dependency, not a new crate edge.
- **Benchmark work is priority-0 and fully preemptible.** The scheduler must
  admit benchmark sweeps only when otherwise idle and preempt them instantly for
  any priority≥1 work (`benchmark-collections` + `preemption_threshold: Some(1)`)
  — measurement isolation is a scheduling guarantee, not a benchmark-side one.
- **Pluggable selection/swap policies** keep model-choice heuristics out of the
  loop mechanics; keep these trait-shaped so mesh-affinity hints could inform
  selection later without rewriting the loop.

## Relationships / edges
- scheduler -> engine via `engine-exec` (see scaffold/contracts/engine-exec.md)
- telemetry -> scheduler via `system-state` (see scaffold/contracts/system-state.md)
- scheduler -> telemetry (kernel) via `kernel-confidence` (read-side: effective-concurrency query; see scaffold/contracts/kernel-confidence.md)
- scheduler -> models via `model-ensure` (see scaffold/contracts/model-ensure.md)
- benchmark -> scheduler via `benchmark-collections` (see scaffold/contracts/benchmark-collections.md)
- api -> scheduler via `api-dispatch` (see scaffold/contracts/api-dispatch.md)
- scheduler <-> store via `store-access` (see scaffold/contracts/store-access.md)

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
approach-sketched — the existing loop is grounded, but the kernel-driven
admission (`effective_max_concurrent` from the fitted surface, replacing the
scalar pressure fraction) is a real design change that is sketched with a
recommended direction, not fully specified.

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading `lib/scheduler/src/admission.rs`
and the scheduler-wiring step in `InferenceService::start`.

## Suggested fill-model
approach-sketched + high complexity -> **strong model, or a Design Mesh pass on
the kernel-driven admission specifically**. The queue/preemption mechanics can be
filled cheaply; the admission-from-kernel logic is the part that warrants the
stronger model. Design co-dependent with `benchmark` and `telemetry` — fill
those three with a shared understanding of the kernel.
