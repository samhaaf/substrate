# Contract: system-state

## Parties
telemetry  ->  {scheduler, api}

## What the edge carries
`SystemState` snapshots (running/pending counts, memory pressure, resident model)
plus throughput estimates from the degree-2 estimator. Note: VRAM fields stubbed at
0 today (mesh-design #5); `effective_max_concurrent` not yet published (blocks spill,
OQ-2). Schema deferred.
