# Contract: gc-events

## Parties
gateway  <-  gc (daemon, :8430)

## What the edge carries
The GC daemon's WebSocket event stream + REST proxy (managed-dir state, sweeps,
evictions) the gateway aggregates into its fan-out hub. Schema/example deferred.
