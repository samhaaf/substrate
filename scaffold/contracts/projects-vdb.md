# Contract: projects-vdb

## Parties
- `projects` (L6 stub) `->` `vdb` (L4 daemon).

*(Stub-track — projects is un-implemented ("not implementing now", INTENT
#51). Named in wave2-plan §3c; the contract round produced no file — a
coverage gap closed at harmonization. Content deferred until projects leaves
the stub track; both component halves already agree. See
`components/projects.md` §`projects-vdb` and `components/vdb.md`
§`projects-vdb`.)*

## Purpose
Databases attached to projects/environments (INTENT #43: a project "can also
have file systems, workspaces, databases, and services attached"). Projects
records the linkage and resolves ids; **VDB owns the database lifecycle.**

## Rough shape
- The vdb catalog's `project` segment of `DbId` (`"<project>/<name>"`) is
  already the join key — no new mechanism.
- projects reads the vdb catalog (via mesh) and surfaces per-project databases
  in its structure graph.
- Environment routing (local→SQLite-in-VFS / cloud→promote, INTENT #97/#98)
  is VDB's copy/verify/switch mechanic, inherited; reconciles with
  `environments-vdb` (when environments lands, `EnsureDatabase.target_hint`
  is replaced by environment-derived routing).

## Open questions
- Everything downstream of projects being designed for real: the graph-node
  shape for an attached database, finance/metadata rollup of database costs.
- The `environments-vdb` / `projects-vdb` split under nested projects
  (sub-projects share the parent's environment by default — environments.md's
  naive nesting rule).
