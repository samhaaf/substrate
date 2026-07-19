# Contract: projects-artifacts

## Parties
- `projects` (L6 stub) `->` `artifacts` (L6 stub).

*(Stub-track — both ends un-implemented. Named so the pair exists; content
deferred entirely until `artifacts` is designed, INTENT §6 / projects.md §6.)*

## Purpose
`projects` depends on `artifacts` for typed, schema'd, **interactable** work-units
("no more files — artifacts") once that crate exists: e.g. an HFT-strategy
artifact run via a versioned controller against a simulator. A project's `artifact`
graph nodes (in `project-tree@1`) resolve to `artifacts`-managed typed objects
rather than raw VFS blobs.

## Rough shape
Deferred — `artifacts` is un-designed (co-batched batch-7 stub, needs
`components/artifacts.md`). Anticipated:
- a project's `artifact` node points at an `artifacts`-owned typed object (id +
  type + schema version);
- `projects` reads/lists artifacts for its topological map; `artifacts` owns the
  type, schema, versioned controller, and script-execution engine;
- raw bytes underneath still live in VFS (`projects-vfs`), typed/interactable
  behavior is `artifacts`'.

## Open questions
- Everything downstream of `artifacts` existing: its identity type, schema
  vocabulary, and whether it is its own standalone tool.
- Boundary between an `artifact` KG node (`projects-kg`), its typed object
  (`artifacts`), and its raw body (`projects-vfs`).
- (Cluster note) `artifacts-vfs` and `artifacts-kg` are NOT in the wave2-plan
  contract-pair inventory and are therefore not authored — flagged in this
  reconciler's missed-pairs list.
