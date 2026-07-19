# Contract: kernel-confidence

## Parties
telemetry (throughput kernel)  <->  {benchmark, scheduler}

## What the edge carries
**NEW this round** (implied by the benchmark/kernel redesign). Query surface over
telemetry's multidimensional throughput kernel — axes: output-length ×
parallel-completion-count × observed memory/CPU/GPU pressure (input-length
retained). Two consumers: `benchmark` asks "where is your confidence currently
LOWEST?" to pick the next adaptive test (replacing the fixed grid); `scheduler`
asks for `effective_max_concurrent` at the current operating point (feeds
admission). Distinct from `system-state`, which carries raw snapshots, not the
fitted kernel/confidence surface. Schema + example data deferred to step 3.
