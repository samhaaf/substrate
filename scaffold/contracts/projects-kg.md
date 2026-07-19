# Contract: projects-kg

## Parties
- `projects` (L6 stub) `->` `kg` (L4).

*(Stub-track. kg.md already records that projects' half is `kg-api` verbatim + a
projects-authored template, so this pair starts from a concrete substrate half.
Content deferred until `projects` leaves the stub track.)*

## Purpose
The project-structure graph: nested project hierarchy, cross-links, and
node→VFS pointers, encoded as a first-class KG graph (INTENT #51, #96). The
graph IS the "graphical file system over the flat VFS" — nodes point at VFS files
(`kg-vfs` existence validation), the tree encodes containment/dependency.

## Rough shape
- **kg side:** `kg-api` **verbatim** — `projects` creates/reads/mutates a
  registered graph; schema-locking pushes validation failures back to `projects`
  (rejected, never coerced).
- **projects-authored template `project-tree@1`:**
  - node types: `project`, `sub_project`, `workspace`, `artifact`, `task`,
    `objective`, `phase`, `handoff`, `human_action`.
  - edge types: `contains`, `depends_on`, `consumes`, `supersedes`, `blocks`.
- **Environment routing inherited per-graph from KG** (local→SQLite /
  cloud→promoted) — `projects` does not re-implement placement.
- Node→VFS pointers resolved via `projects-vfs` for bodies; existence validated
  by KG (`kg-vfs`).

## Open questions
- Whether the project graph is one global graph or one graph per top-level
  project (leaning per-top-level-project, registered in KG's global registry for
  cross-project reuse).
- The `.mind` workspace-schema migration mapping (coordinator artifacts/tasks →
  `project-tree@1` node types) — the confirmed but deferred migration (INTENT #51).
- Which node types are `rollup`-resolved (`projects-rollup`: "tasks are rolled
  up") vs plain KG nodes.
