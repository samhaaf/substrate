# Contract: mesh-registry-read

## Parties
gateway  ->  mesh (NodeRegistry / service-registry)

## What the edge carries
Gateway resolves node endpoints and fleet state via mesh instead of static config:
`/api/nodes` (NodeInfo + roles + last SystemState), `/api/mesh/stats` (fleet
aggregate). Part of the registry-as-seam refactor. Schema deferred.
