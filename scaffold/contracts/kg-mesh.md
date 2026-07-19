# Contract: kg-mesh

## Parties
kg  <->  mesh

## What the edge carries
Registration + replication: KG registers its slug/endpoint in mesh's service
registry (an instance of `service-lookup`), and rides mesh's eventual-
consistency/replication plane to distribute graph state across nodes — and into
AWS/S3 through **mesh's S3 adapter** (client-side encryption, cold-storage
classes; see `components/mesh.md`), giving KGs eventual consistency with the
AWS side (use case: an external agent extracts a user's intent into a KG and it
shows up in the mesh). OPEN: the consistency model — a graph of interconnected
nodes is "the superset" of the registry's naive timestamp-wins KV and needs its
own merge design. Schema/example deferred. **requirements-only** (round-3
second batch, 2026-07-18).
