# Contract: api-dispatch

## Parties
api  ->  {scheduler, store, telemetry, benchmark}

## What the edge carries
The node-internal side of `v1-completion-api`: how the Axum handlers dispatch into
subsystems to fulfill each `/v1/` route (submit -> scheduler/store, state ->
telemetry, benchmark run -> benchmark). Schema deferred.
