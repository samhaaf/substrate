# telemetry

**Status:** existing (`lib/telemetry`), kept as-is. **Nesting:** child of inference.

Two components in one crate: a `SystemSampler` polling CPU/memory/GPU via
`sysinfo` (GPU utilization real on macOS `ioreg`; VRAM telemetry stubbed at 0 —
mesh-design finding #5), and a `ThroughputEstimator` fitting a degree-2
polynomial regression to observed completions to predict duration with a
confidence annotation. Publishes the `SystemState` the scheduler and API read
via the `system-state` edge; `store-access` for observations.
