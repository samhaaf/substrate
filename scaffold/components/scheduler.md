# scheduler

**Status:** existing (`lib/scheduler`), kept as-is. **Nesting:** child of inference-node.

The central scheduling loop: queue management, admission control, swap
evaluation, preemption, crash recovery, and pluggable selection/swap policies.
Each tick reads system state, checks memory pressure, evaluates whether to swap
models, drains + swaps, then admits pending work up to a (dynamically lowered)
concurrency target. High design difficulty (the admission target varies under
pressure — see mesh-design finding #3). Edges: `engine-exec`, `system-state`,
`model-ensure`, `benchmark-collections`, `api-dispatch`, `store-access`.
