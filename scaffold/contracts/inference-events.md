# Contract: inference-events

## Parties
gateway  <-  inference

## What the edge carries
Per-node WebSocket event stream + REST proxy the gateway subscribes to (one
subscription per node in v2), envelopes tagged with the node's real `node_id`.
Schema/example deferred.
