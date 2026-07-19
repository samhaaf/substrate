# Contract: stack-vfs

## Parties
stack  ->  vfs

## What the edge carries
The single SQLite file each `stack` daemon wraps is **stored in the VFS**:
stack opens/reads/writes its database file through this edge (and inherits
VFS per-directory policies/replication for it). Schema/example deferred.
**requirements-only** (rounds 4–5 lock, 2026-07-18).
