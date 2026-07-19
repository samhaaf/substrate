# Contract: kernel-confidence

## Parties
telemetry (throughput kernel)  <->  {benchmark, scheduler}
(owner/provider: **telemetry** — owns the fitted surface and all math; consumers:
**benchmark** asks "where is confidence lowest?"; **scheduler** asks "effective max
concurrency here?")

All three are compiled-in libraries of one `inference` daemon (INTENT #22/#29). This
is an **in-process Rust query surface, NOT a wire contract.** The exact
confidence-field math (bandwidth, axis weights, `N₀`/`σ²_ref`) can change without a
contract break — only the query signatures below are frozen.

## Purpose
The single query surface over telemetry's one multidimensional, confidence-aware
throughput kernel (per `(node, model)`), fit from `store.benchmark_runs` (active
`'benchmark'` ∪ passive `'observed'` samples). Two asymmetric consumers read the
SAME fitted surface off different projections:

- **benchmark** (active-learning loop): supplies a *feasible region* and gets back
  the single most informative next test (argmax predictive variance), plus a batch
  ETA and a refit notify. Replaces the retired fixed Cartesian grid.
- **scheduler** (admission): given the current operating point, gets the effective
  max concurrency (largest parallelism that still gains throughput at confidence ≥
  floor), or `Unconfident` → fall back to the scalar memory heuristic.

Distinct from `system-state`, which carries raw snapshots, not the fitted surface.
(`system-state` also stamps a convenience `effective_max_concurrent` scalar — that is
the zero-call hot-path read; THIS query is the authoritative, what-if-capable form.)

## Schema

Vocabulary and the query surface, on telemetry's per-model kernel handle. Types
reference `types::{model, system}`; kernel-local structs are telemetry-owned (in
`types::kernel` or telemetry-internal — telemetry's call, since the seam is
in-process).

```rust
// --- shared axis vocabulary (the kernel's domain) ---
struct OperatingPoint {                 // a point in the kernel's 6-axis space
    output_tokens:   u32,
    parallelism:     u32,
    input_tokens:    u32,
    mem_pressure:    f32,               // observed covariate, 0..1 (cannot be set — concern 3)
    cpu_utilization: f32,
    gpu_utilization: Option<f32>,       // None where GPU unmeasured (system-state concern 4)
}

// --- a point prediction with honest confidence (supersedes estimator::EstimatedDuration) ---
struct KernelEstimate {
    duration_ms:       u64,             // canonical target: per-completion latency
    tokens_per_second: f64,             // derived view (output_tokens / duration_s) for the dashboard
    confidence:        f32,             // c(q) in [0,1] — σ̂-relative, replaces the binary flag
    is_estimate:       bool,            // honesty taint: support leans on estimated covariates (INTENT #9)
}

// === BENCHMARK consumer ===
// benchmark constructs the feasible region (context-clip + reachable pressure +
// backoff exclusion — benchmark concern 3/4); telemetry computes argmax over it.
struct FeasibleRegion {
    lattice:            Vec<DialPoint>,     // context-clipped log-spaced dial anchors
    reachable_pressure: Vec<PressureBand>,  // predicted-reachable-from-current-baseline bands
    excluded:           Vec<LatticeKey>,    // the backoff table (termination-critical)
}
struct DialPoint { input_tokens: u32, output_tokens: u32, parallelism: u32 }
struct CandidatePoint {
    dials:                DialPoint,
    target_pressure_band: PressureBand,
    confidence:           f32,          // current c() there (σ̂-relative)
    expected_gain:        f32,          // predictive variance at the point (D-optimal proxy)
}

// telemetry, per-model kernel handle:
fn lowest_confidence(&self, model: &ModelId, region: &FeasibleRegion)
      -> Option<CandidatePoint>;
      // None => min confidence >= θ over the region (CONVERGED/QUIESCENT input).
      //         θ is calibrated against residual variance σ̂², not an absolute constant.
      // MUST respect `excluded` and `reachable_pressure` (never returns an excluded
      //         or unreachable candidate) — the termination proof's load-bearing property.
fn confidence_at(&self, model: &ModelId, p: &OperatingPoint) -> f32;   // dashboard + tests
fn estimate(&self, model: &ModelId, q: OperatingPoint) -> KernelEstimate;
      // point prediction; also the batch-ETA read (benchmark CriticalSection `until`)
      // and the /v1/estimate source. Cold-start-safe (conservative default path).
fn notify_new_samples(&self, model: &ModelId);   // cheap incremental refit trigger

// === SCHEDULER consumer ===
fn effective_concurrency(&self, model: &ModelId, at: OperatingPoint)
      -> ConcurrencyAdvice;
enum ConcurrencyAdvice {
    Confident { max_concurrent: u32, confidence: f32 },  // largest p with ΔT>=ε and c>=floor
    Unconfident,                                         // below floor -> scheduler falls back
}

// === DASHBOARD view (via api-dispatch kernel_surface(), NOT a direct party here) ===
fn kernel_surface(&self, model: &ModelId) -> KernelSurface;  // curves + confidence + honesty markers
```

### Training-data source
The kernel is fit from `store.benchmark_runs` (active ∪ passive) read via
`store-access`; the fitted surface is an **in-memory cache rebuilt on boot** and
refreshed on `notify_new_samples`. No persisted model blob, no own file (telemetry
concern 5). `lowest_confidence` returns *controllable* cells; the pressure regime is
*reported* (`target_pressure_band`), never commanded — benchmark cannot set pressure.

## Error cases

**Total functions — no error type surfaced to either consumer.**
- A never-benchmarked model: `estimate` returns a conservative cold-start
  `KernelEstimate { confidence: 0.0, is_estimate: true }` (supersedes
  `cold_start_estimate`); `lowest_confidence` returns the whole space as the first
  (space-filling) candidate; `effective_concurrency` returns `Unconfident` →
  scheduler falls back.
- `ConcurrencyAdvice::Unconfident` and `lowest_confidence -> None` are **normal
  branches**, not errors.
- `is_estimate = true` on any prediction whose support is estimate-dominated is the
  **honesty signal** (INTENT #9) the dashboard renders as a tilde — not an error.
- A poisoned internal mutex degrades to cold-start semantics (matches the live
  estimator), never panics.

## Version sensitivity

**N/A in-process** — the three consumers recompile with telemetry as one `inference`
binary. The confidence-field math is intentionally NOT part of the frozen contract:
only the query signatures + struct shapes above are frozen, so telemetry can tune
bandwidth/axis-weights/`c(q)` form freely. If `OperatingPoint`/`KernelEstimate` ever
serialize outward (`GET /v1/benchmark/kernel`, dashboard), that wire shape is
api/telemetry's and follows `types.md` guardrail-4 additive discipline
(`#[serde(default)]`, catch-all arms).

## Reconciliation notes

- **Query signature: model-only vs constraint-passing (the real merge).** telemetry
  proposed `lowest_confidence_region(model) -> Option<KernelRegion>` (model only, it
  picks the region). benchmark proposed `lowest_confidence(region: &FeasibleRegion)
  -> Option<CandidatePoint>` (benchmark supplies constraints). **Resolution:
  benchmark's constraint-passing form wins**, merged as `lowest_confidence(model,
  &FeasibleRegion)`. Rationale: the feasible-region projection — context-window
  clip, reachable-pressure prediction, and the **backoff exclusion set** — is
  benchmark's to own (benchmark concern 3/4), and the module's *termination proof*
  depends on telemetry respecting `excluded` + `reachable_pressure`. telemetry's
  model-only signature had no way to receive those constraints, so it could return an
  unreachable or backed-off cell and resurrect the livelock. telemetry keeps
  ownership of the argmax math over `X`/`(XᵀX)⁻¹`; benchmark keeps ownership of *what
  is feasible*. **Losing position recorded:** telemetry's `lowest_confidence_region(
  model)` — rejected because it cannot honor the backoff/feasibility constraints the
  termination argument requires.
- **Return/vocabulary naming unified to telemetry's (owner wins on shape).**
  benchmark's `KernelPoint` == telemetry's `OperatingPoint` (same six fields) →
  keep **`OperatingPoint`**, with `gpu_utilization: Option<f32>` (telemetry's
  honesty form) over benchmark's bare `f32`. benchmark's `estimate(p) ->
  EstimatedDuration` folds into `estimate(model, q) -> KernelEstimate` (the
  `duration_ms` field is the batch ETA). No semantic loss; benchmark's names were a
  consumer sketch of the same objects.
- **`effective_max_concurrent`: query here, scalar on `system-state` (cross-file
  reconciliation).** See `system-state.md` reconciliation notes — resolved as BOTH:
  the scalar is the scheduler's hot-path read; `effective_concurrency(...)` here is
  the authoritative, confidence-bearing form. The scheduler's own proposal marks the
  synchronous per-decision query "optional, approach-sketched" (default path = the
  `SystemState` scalar), which is consistent with this being the what-if surface.
- **Two consumers, one document (confirmed).** Both benchmark and scheduler read one
  fitted surface; per wave2-plan the stub stays ONE file with two read patterns
  rather than splitting into per-consumer files. Adopted.

## Example data

Example world: node **macbook**, model **qwen3-4b**. Two reads against the one
fitted kernel.

**benchmark read** — the active-learning loop asks for the next test. macbook is
idle; benchmark has dense samples at low output/parallelism but is thin at
`output_tokens=100000`. It passes a feasible region (100k output is within
qwen3-4b's context, and the high-parallelism band is reachable by self-induction):

```rust
let region = FeasibleRegion {
    lattice: vec![
        DialPoint { input_tokens: 512, output_tokens: 1000,   parallelism: 1 },
        DialPoint { input_tokens: 512, output_tokens: 10000,  parallelism: 4 },
        DialPoint { input_tokens: 512, output_tokens: 100000, parallelism: 8 },
        // ... ~low hundreds of log-spaced anchors
    ],
    reachable_pressure: vec![PressureBand::Low, PressureBand::Medium, PressureBand::High],
    excluded: vec![],   // nothing in backoff yet
};
let next = telemetry.lowest_confidence(&ModelId("qwen3-4b".into()), &region);
// Some(CandidatePoint {
//   dials: DialPoint { input_tokens: 512, output_tokens: 100000, parallelism: 8 },
//   target_pressure_band: PressureBand::High,
//   confidence: 0.18,          // lowest here — the argmax-variance cell
//   expected_gain: 0.71,
// })

// batch ETA for the CriticalSection `until` (benchmark concern 6):
let eta = telemetry.estimate(&ModelId("qwen3-4b".into()),
    OperatingPoint { output_tokens: 100000, parallelism: 8, input_tokens: 512,
                     mem_pressure: 0.10, cpu_utilization: 0.15, gpu_utilization: Some(0.20) });
// KernelEstimate { duration_ms: 540_000, tokens_per_second: 1481.0, confidence: 0.18, is_estimate: true }

// after the batch is accepted and rows written:
telemetry.notify_new_samples(&ModelId("qwen3-4b".into()));
// confidence_at(that point) is now non-decreasing (monotonicity conformance)
```

**scheduler read** — a tick at the operating point from `system-state.md`'s macbook
snapshot (memory 0.42, 1 running):

```rust
let advice = telemetry.effective_concurrency(&ModelId("qwen3-4b".into()),
    OperatingPoint { output_tokens: 2048, parallelism: 1, input_tokens: 256,
                     mem_pressure: 0.42, cpu_utilization: 0.31, gpu_utilization: Some(0.55) });
// ConcurrencyAdvice::Confident { max_concurrent: 4, confidence: 0.86 }
// -> telemetry ALSO stamped effective_max_concurrent: Some(4) on the SystemState scalar.
```

On **pi** (kernel not yet fit) the same call returns `ConcurrencyAdvice::Unconfident`
and the scalar is `None`, so the scheduler falls back to its memory heuristic.

## Conformance requirement

A filler's `telemetry` (+ its benchmark/scheduler consumers) passes iff, against the
example world: (a) **selection steering** — with samples dense in one lattice region
only, `lowest_confidence` returns a candidate *outside* it; (b) **monotonicity** —
after `notify_new_samples` with an accepted sample at point `p`, `confidence_at(p)`
does not decrease; (c) **feasibility respect** — `lowest_confidence` never returns a
candidate whose key is in `excluded` or whose band is outside `reachable_pressure`
(the termination-proof property); (d) **graceful degradation** — a never-benchmarked
model yields `effective_concurrency -> Unconfident` and `estimate -> confidence: 0.0,
is_estimate: true`, and the scheduler falls back rather than admitting on no data.
