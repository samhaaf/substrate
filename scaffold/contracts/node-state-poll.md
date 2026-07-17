# Contract: node-state-poll

## Parties
mesh.completion-router (NodeRegistry)  ->  inference (api)

## What the edge carries
Read-only polling: `GET /v1/system/state` (running/pending counts, memory
pressure, resident model) and `GET /v1/models` (per-node downloaded/resident
inventory) feeding the registry's health + affinity decisions. Never on the
request path. Schema deferred; shared `NodeInfo`/`NodeCapabilities` shape is OQ-3.
