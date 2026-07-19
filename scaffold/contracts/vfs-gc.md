# Contract: vfs-gc

## Parties
vfs  <->  gc (per-node tool)

## What the edge carries
Per-node storage enforcement: VFS calls the node-local GC tool to enforce that
device's directory policies (max size; FIFO / least-recently-updated /
least-recently-accessed eviction, etc.). GC's role here is per-node executor of
VFS policy. OPEN: whether GC stays a called-as-tool or gets rolled up into the
VFS (operator flagged both; see `components/gc.md` / `components/vfs.md`).
Schema/example deferred. **requirements-only** (round-3, 2026-07-18).
