# Contract: gc-events

## Parties
mesh  <-  gc (per-node store, :8430)
*(was `gateway <- gc`; gateway merged into mesh, 2026-07-18)*

## What the edge carries
GC's WebSocket event stream + REST proxy (managed-dir state, sweeps, evictions)
that mesh's observability plane aggregates into its fan-out hub. Round-3 note:
this API/WS surface is also how the node's ONE centralized GC store is updated
remotely (see `components/gc.md`). Schema/example deferred.
