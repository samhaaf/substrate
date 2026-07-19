# Contract: aws-vfs

## Parties
aws  <->  vfs

## What the edge carries
The **S3 overflow tier** (round-9 owner: the `aws` crate — previously
mesh-internal, originally a VFS feature; requirements unchanged through
both moves): when the personal mesh runs out of space, VFS overflow
placement pushes/reads/pulls file content against S3 through aws's
WebSocket adapter surface — S3 cold-storage classes, **client-side
encryption before upload** (encryption keys held by the `secrets` service,
never by aws or vfs). Mesh still orchestrates replication/overflow
placement decisions; this edge is the storage data path. Schema/example
deferred. **requirements-only** (round-9 lock, 2026-07-19).
