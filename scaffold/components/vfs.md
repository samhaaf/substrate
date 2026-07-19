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
- **Layering (round-8 lock, 2026-07-19): VFS sits BELOW VDB.** In the locked
  VDB/db/KG decomposition (see `components/stack.md`), the SQLite database
  files VDB manages **live in the VFS** ("the SQLite files must live
  somewhere"). Locked layer order: **VFS < VDB < KG**. KG graphs route to a
  location in VDB and/or VFS (see `components/kg.md`).
- **Provenance: a LIGHTER design requirement here (round-7 scoping,
  2026-07-19).** The FIRST-ORDER provenance principle (`overview.md`) is
  configured per-project/per-database with **VDB as its primary home**;
  VFS-level provenance is explicitly de-emphasized — operator: "Also a VFS
  thing, but I typically don't care about provenance in the file system —
  usually only in the database and the stack pattern." VFS still carries
  provenance as a design requirement, just a lighter one than
  stack/VDB/db's.
- **RAID-inspired features** — per-file replication factor; a multi-drive node
  (e.g. a Raspberry Pi with two external drives) configurable as a RAID-like
  warm/cold-storage node.
- ~~**S3 overflow tier**~~ — **SUPERSEDED TWICE — current owner (round-9,
  2026-07-19): the `aws` crate.** History: the round-3 capture placed the S3
  adapter here (overflow tier when the personal mesh runs out of space, S3
  cold-storage classes, client-side encryption before upload); the round-3
  second batch moved it INSIDE `mesh` (mesh owns all eventual-consistency/
  replication). **Round-9 corrects the owner again: the S3/AWS adapter
  surface lives in the new `aws` crate** — the AWS virtualization layer —
  and mesh/vfs/kg/secrets CONSUME it. The requirements themselves stand
  unchanged through both moves — only the owner moved. Mesh still
  orchestrates replication/overflow placement; the actual S3 surface is
  `aws`'s. See `components/aws.md` / `components/mesh.md` and contract
  `aws-vfs`.

## Relationships / edges (stubs only)

- **gc** via `vfs-gc` — per-node storage enforcement: VFS calls the node's GC
  tool to enforce directory policies on that device
  (scaffold/contracts/vfs-gc.md).
- **mesh** via `vfs-mesh` — registration and topology awareness: VFS registers
  in the service registry and learns which nodes/drives exist from mesh
  (round-9: the S3-overflow leg no longer rides this edge — see `aws-vfs`)
  (scaffold/contracts/vfs-mesh.md).
- **aws** via `aws-vfs` — **NEW round-9 (requirements-only).** The S3
  overflow tier: VFS reaches S3 (cold-storage classes, client-side
  encryption with keys held by `secrets`) through the `aws` crate's adapter
  surface (scaffold/contracts/aws-vfs.md).
- **kg** via `kg-vfs` — KG nodes point at files in this flat store; VFS is the
  target of KG's pointed-at-file existence validation
  (scaffold/contracts/kg-vfs.md).
- **projects** via `projects-vfs` — `projects` is the graphical/knowledge layer
  built on top of this flat store (scaffold/contracts/projects-vfs.md).
- **stack/VDB** via `stack-vfs` — the SQLite file each stack/VDB daemon wraps
  lives in the VFS; round-8: this is the locked VFS-below-VDB layering
  (scaffold/contracts/stack-vfs.md).

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
