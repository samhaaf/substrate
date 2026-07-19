# Contract: system-state

## Parties
telemetry  ->  {scheduler, api}   (producer/owner: **telemetry**; consumers:
**scheduler** each admission tick, **api** for `GET /v1/system/state`)

`telemetry` is compiled into `inference` (INTENT #22/#29). The `SystemState` struct
itself lives in `types::system` and is produced in-process — but it ALSO crosses the
mesh when `api` serves it on `node-state-poll` / the surface schema, so its field
discipline follows `types.md` guardrail 4 (additive-only) even though the
producer→consumer path here is in-process.

## Purpose
The node's point-in-time resource + scheduling snapshot: memory/CPU/GPU pressure,
running/pending counts, resident model, and — the wave-2 addition — a
kernel-computed `effective_max_concurrent` convenience scalar. The scheduler reads it
each tick to classify pressure and set its admission target; `api` serializes it
for the router's reconcile poll and the dashboard `system` section.

## Schema

The `types::system::SystemState` struct (wave-2 shape). Fields marked NEW are the
wave-2 additions; all are `#[serde(default)]` for additive mixed-version rollout.

```rust
struct SystemState {
    // --- scheduling counts + resident (unchanged) ---
    resident_model:   Option<ModelId>,   // written by engine on load/unload
    running_count:    u32,
    pending_count:    u32,

    // --- pressure axes ---
    memory_pressure:  f32,               // 0.0 ..= 1.0
    cpu_utilization:  f32,               // 0.0 ..= 1.0
    gpu_utilization:  Option<f32>,       // None where GPU is unmeasured (concern 4)
    is_gpu_estimate:  bool,              // true = GPU covariates are estimated (INTENT #9)

    // --- NEW (telemetry concern 4): VRAM honesty — never a fabricated 0 ---
    gpu_memory_used_bytes:  Option<u64>, // was u64=0; None = not measured / unified memory
    gpu_memory_total_bytes: Option<u64>, // None on Apple-Silicon unified memory

    // --- NEW (telemetry concern 6 / scheduler concern 1): the admission-target scalar ---
    effective_max_concurrent: Option<u32>,
    //   Some(k) => kernel is confident at the current operating point: sustain k
    //              parallel completions before per-completion throughput collapses.
    //   None    => kernel confidence below floor here → the scheduler falls back to
    //              its scalar memory-pressure heuristic (graceful degradation).

    sampled_at: Timestamp,
}
```

### Accessor (in-process)

```rust
// lib/telemetry/src/lib.rs — cheap RwLock clone, always returns the last good snapshot
fn current_state(&self) -> SystemState;
```

### How the scheduler consumes it (scheduler concern 1)

`slots_to_admit(sys, running)`:
```rust
if sys.memory_pressure >= self.hard_pct { return 0; }        // (a) hard SAFETY FLOOR always wins
let target = match sys.effective_max_concurrent {
    Some(k) => k.min(self.max_concurrent),                   // (b) kernel LOWERS, config CEILINGS
    None    => /* (c) scalar memory-pressure fallback — today's proportional ramp, verbatim */,
};
target.saturating_sub(running)
```
The scheduler treats `gpu_*` fields as honest-estimate-flagged (reads
`is_gpu_estimate`) but does NOT render the tilde — that is the dashboard/surface's
job. `None` GPU-memory fields mean "no independent VRAM signal," NOT zero pressure.

## Error cases

None surfaced. `current_state()` always returns the last good snapshot; a poisoned
sampler mutex degrades to a zeroed snapshot (real code `sampler.rs:210`), never
panics. `effective_max_concurrent: None` is a **normal branch** (the scheduler
fallback), not an error. A snapshot taken mid-swap is internally consistent (single
`RwLock`).

## Version sensitivity

- **In-process producer→consumer path: no wire skew** (single `inference` binary).
- **Where it crosses the mesh** (via `api`'s `node-state-poll` and surface schema):
  all wave-2 field additions (`gpu_memory_{used,total}_bytes`,
  `effective_max_concurrent`) are **additive-safe** — `#[serde(default)]` yields
  `None` when an older telemetry produces the struct or an older consumer reads a
  newer one. A scheduler reading an older telemetry's `SystemState` (no
  `effective_max_concurrent`) simply always takes the scalar fallback path — no
  break. **Breaking** would be: changing `gpu_memory_used_bytes` back to a bare
  `u64` (reintroduces the fabricated-zero dishonesty, INTENT #9), or making
  `effective_max_concurrent` non-optional (removes the graceful-degradation branch).

## Reconciliation notes

- **`effective_max_concurrent` placement — field vs query (the flagged
  disagreement).** scheduler.md/system-state stub imply it as a plain `SystemState`
  field; telemetry.md (concern 6) argues it is "primarily a `kernel-confidence`
  query" (a *fitted-model output* at a chosen policy + operating point, not a raw
  measurement — putting it on the raw sample "conflates measured with modeled and
  goes stale when the kernel refits"). **Resolution: BOTH, telemetry's dual form
  wins and there is no real conflict.** telemetry stamps a *convenience scalar* onto
  `SystemState` each tick (the zero-extra-call common-case read the scheduler's hot
  path uses), AND exposes the authoritative, confidence-bearing, what-if-capable
  form on the `kernel-confidence` query (`effective_concurrency(...)`). The scalar is
  explicitly **best-effort / optional** (`Option<u32>`, `None` on low confidence).
  This satisfies the scheduler's desire for a cheap scalar field and telemetry's
  correctness concern about staleness. **Losing position recorded:** a *sole* plain
  scalar field with no query counterpart (the stub's literal reading) — rejected
  because it loses the what-if capability benchmark/scheduler co-design may later
  need and hides that the number is a modeled projection, not a measurement.
- **VRAM `Option<u64>` (telemetry concern 4, a `types` ask).** Not disputed by any
  party; scheduler.md's consumer note explicitly accepts treating `None` as "no
  independent VRAM signal." Adopted. Flagged to the `types` pass as the one
  cross-crate ripple (types.md owns `SystemState`'s home but did not touch this
  field) — recorded here as the reconciled shape.

## Example data

Example world: node **macbook**, model **qwen3-4b** resident, one completion
running, three pending. Apple-Silicon unified memory (so VRAM fields are `None`,
`is_gpu_estimate: true`), kernel confident here → an `effective_max_concurrent` of 4.

```json
{
  "resident_model": "qwen3-4b",
  "running_count": 1,
  "pending_count": 3,
  "memory_pressure": 0.42,
  "cpu_utilization": 0.31,
  "gpu_utilization": 0.55,
  "is_gpu_estimate": true,
  "gpu_memory_used_bytes": null,
  "gpu_memory_total_bytes": null,
  "effective_max_concurrent": 4,
  "sampled_at": "2026-07-19T10:15:02Z"
}
```

Contrast — node **pi** (a low-power node, kernel not yet fit for `qwen3-4b`, so the
scheduler falls back to its memory heuristic):

```json
{
  "resident_model": "qwen3-4b",
  "running_count": 0,
  "pending_count": 1,
  "memory_pressure": 0.78,
  "cpu_utilization": 0.60,
  "gpu_utilization": null,
  "is_gpu_estimate": true,
  "gpu_memory_used_bytes": null,
  "gpu_memory_total_bytes": null,
  "effective_max_concurrent": null,
  "sampled_at": "2026-07-19T10:15:02Z"
}
```

For macbook's snapshot the scheduler admits `min(4, max_concurrent) - 1` more
completions; for pi's, with `effective_max_concurrent: null` and
`memory_pressure: 0.78` between soft (say 0.70) and hard (0.90), it uses the
proportional scalar fallback.

## Conformance requirement

A filler's `telemetry` + `scheduler` pass iff, against the example world: (a) an
unknown/unmeasured VRAM is serialized as `null`, **never** `0` (INTENT #9); (b) with
`effective_max_concurrent: Some(4)` and `memory_pressure < hard`, the scheduler's
admission target is `4.min(max_concurrent)`; (c) with `effective_max_concurrent:
null`, the scheduler reproduces today's scalar-heuristic admission outputs exactly
(the existing `admission.rs` tests pass unchanged); (d) an older-schema
`SystemState` (no wave-2 fields) deserializes with all NEW fields `None`, and the
scheduler takes the fallback path without error.
