# Contract: registry-replication

## Parties
mesh.service-registry  <->  mesh.service-registry (peer instances on other devices)

## What the edge carries
The eventual-consistency replication/gossip edge between per-device registry
instances (propagation of slug->endpoint entries, conflict handling, tombstones).
The hardest new sub-problem; schema + consistency model deferred to step 2/3.
