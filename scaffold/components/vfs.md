# vfs

**Status:** NEW (round-3 feedback lock, 2026-07-18). **Nesting:** top-level.
**Stub-plus — requirements captured from the operator; NOT a full design.**

## Charter (requirements, operator's words where quoted)

"A boring distributed flat file system." The mesh-wide storage substrate other
components (notably `projects`) build on. Requirements:

- **Per-directory configurable policies** — max size; eviction strategy: FIFO /
  least-recently-updated / least-recently-accessed, etc.
- **GC as the per-node enforcement tool** — GC becomes a per-node tool called by
  the VFS to handle that device's storage. "Might need to get rolled up into the
  VFS" — **OPEN question: rolled-in vs. called-as-tool.** See `gc.md`.
- **Self-tracked performance** — VFS tracks its own performance like inference
  does: uptime, per-node read/write latency.
- **RAID-inspired features** — per-file replication factor; a multi-drive node
  (e.g. a Raspberry Pi with two external drives) configurable as a RAID-like
  warm/cold-storage node.
- **S3 overflow tier** — an S3 adapter as an overflow tier when the personal
  mesh runs out of space, using S3 cold-storage classes, with **client-side
  encryption before upload** (not merely encrypted-at-rest).

## Relationships / edges (stubs only)

- **gc** via `vfs-gc` — per-node storage enforcement: VFS calls the node's GC
  tool to enforce directory policies on that device
  (scaffold/contracts/vfs-gc.md).
- **mesh** via `vfs-mesh` — registration and topology awareness: VFS registers
  in the service registry and learns which nodes/drives exist from mesh
  (scaffold/contracts/vfs-mesh.md).
- **projects** via `projects-vfs` — `projects` is the graphical/knowledge layer
  built on top of this flat store (scaffold/contracts/projects-vfs.md).

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
