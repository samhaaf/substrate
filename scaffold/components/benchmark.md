# benchmark

**Status:** existing (`lib/benchmark`), kept as-is. **Nesting:** child of inference-node.

The benchmark orchestrator: idle-period detection and priority-0,
`request_full_system` sweep collections that characterize throughput across an
(output_length x parallelism x input_length) grid. Benchmarks run only when the
system is otherwise idle and are preempted immediately by real (priority >= 1)
work, guaranteeing measurement isolation. Submits its sweeps through the
scheduler (`benchmark-collections`); `store-access` for run records. This is the
data source the telemetry estimator is trained on.
