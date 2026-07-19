# Contract: vfs-mesh

## Parties
vfs  <->  mesh

## What the edge carries
Registration + topology awareness: VFS registers its slug/endpoint in mesh's
service registry (an instance of `service-lookup`), and reads mesh's node/drive
topology to place replicas (per-file replication factor, warm/cold multi-drive
nodes) and to report its per-node performance (uptime, read/write latency) into
mesh's observability plane. (Round-9: the S3-overflow leg that briefly rode
this edge via mesh's internal S3 adapter now has its own edge — the S3/AWS
surface lives in the `aws` crate; see `aws-vfs` / `components/aws.md`.)
Schema/example deferred. **requirements-only**
(round-3, 2026-07-18; S3 leg re-routed round-9, 2026-07-19).
