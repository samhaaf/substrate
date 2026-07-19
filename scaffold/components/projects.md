# projects

**Status:** NEW (round-3 feedback lock, 2026-07-18). **Nesting:** top-level.
**Stub-plus — requirements captured from the operator; NOT a full design.**

## Charter (requirements)

A **graphical file system / knowledge graph built ON TOP of the flat `vfs`**,
pointing into sections of it. Requirements:

- The place where **applications get built** — hierarchically nested projects.
- A **centralized registry you can push to**: dashboards and source code,
  accessible from anywhere on the mesh.
- **Project-published dashboards must be navigable from the main mesh
  dashboard** — via projects' own surface schema (the boring-surface-schema
  pattern; see `scaffold/contracts/surface-schema.md`): a project publishes its
  dashboard's render/interaction schema and the mesh dashboard mounts it.
- **Per-project metadata and finances** eventually attach here (see the finance
  placeholder in `overview.md`'s future section).

## Relationships / edges (stubs only)

- **vfs** via `projects-vfs` — the knowledge graph points into sections of the
  flat file system; project artifacts (source, dashboards) live in vfs storage
  (scaffold/contracts/projects-vfs.md).
- **mesh** via `projects-mesh` — registry push + surfacing: projects registers
  itself, pushes project/dashboard registrations, and its published dashboards
  become navigable from the mesh dashboard
  (scaffold/contracts/projects-mesh.md).

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
