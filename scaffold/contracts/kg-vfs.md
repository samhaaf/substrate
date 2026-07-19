# Contract: kg-vfs

## Parties
kg  ->  vfs

## What the edge carries
Node-to-file pointers: KG nodes can point to files in the flat VFS, and KG
validates the **existence of the pointed-at file** through this edge (on
pointer creation and on validation sweeps). Schema/example deferred.
**requirements-only** (round-3 second batch, 2026-07-18).
