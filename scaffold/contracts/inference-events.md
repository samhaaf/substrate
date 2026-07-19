# Contract: inference-events

## Parties
mesh  <-  inference
*(was `gateway <- inference`; gateway merged into mesh, 2026-07-18)*

## What the edge carries
Per-node WebSocket event stream + REST proxy that mesh's observability plane
subscribes to (one subscription per node in v2), envelopes tagged with the
node's real `node_id`. Schema/example deferred.
