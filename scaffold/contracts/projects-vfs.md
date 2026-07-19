# Contract: projects-vfs

## Parties
- `projects` (L6 stub) `->` `vfs` (L3).

*(Stub-track / existing stub, refit in place. `projects` is a plain VFS client —
no new wire. Content firm-but-deferred until `projects` leaves the stub track.)*

## Purpose
The graphical-file-system-over-flat-storage edge: `projects` (a knowledge graph
over storage) reads/writes ordinary VFS files for project source, dashboard
bundles, and artifact bodies, and reads placement/policy for its topological-map
UI. The **graph** over the files is KG's (see `projects-kg`), not this edge's;
this edge is just the byte storage.

## Rough shape
`projects` is a **plain VFS client** — vfs.md's storage-side note already pins
this; there is **no new wire**:
- `Read` / `Write` on the `vfs-content` surface (project source code, built
  dashboard bundles, metadata blobs, artifact bodies).
- Prefix scans for the topological-map UI (list a project's file subtree).
- Reads per-directory placement/policy (replication factor, eviction policy) to
  present in the map; does not set VFS policy beyond ordinary directory config.
- KG nodes hold VFS pointers (path/handle); `projects` resolves a graph node's
  file through this edge (KG's existence validation guards dangling pointers —
  `kg-vfs`, not here).

## Open questions
- Whether project artifact bodies live directly in VFS or are mediated by the
  `artifacts` crate once it exists (`projects-artifacts`) — leaning: `artifacts`
  owns typed/interactable bodies, VFS holds raw bytes underneath.
- VFS path/keyspace convention for a project subtree (e.g. `projects/<id>/…`)
  vs a per-project directory policy — pinned when `projects` is designed.
