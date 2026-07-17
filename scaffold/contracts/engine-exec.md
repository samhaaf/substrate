# Contract: engine-exec

## Parties
scheduler  ->  engine

## What the edge carries
Submit/drain completions, slot lifecycle (RAII accounting), and model swap through
the `InferenceBackend` trait. Schema deferred.
