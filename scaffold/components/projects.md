# projects

**Status:** NEW (round-3 feedback lock, 2026-07-18). **Nesting:** top-level.
**Stub-plus — requirements captured from the operator; NOT a full design.**

## Charter (requirements)

A **graphical file system / knowledge graph built ON TOP of the flat `vfs`**,
pointing into sections of it. **Round-3 second batch (2026-07-18): projects
straddles `kg` + `vfs`** — the graph that encodes a project's structure lives
in the new Knowledge Graph service (`components/kg.md`), and the graph's nodes
point at project files in the VFS (`kg-vfs` existence-validated pointers).
Projects is a consumer/composer of both, not its own graph engine.
Requirements:

- The place where **applications get built** — hierarchically nested projects.
- A **centralized registry you can push to**: dashboards and source code,
  accessible from anywhere on the mesh.
- **Project-published dashboards must be navigable from the main mesh
  dashboard** — via projects' own surface schema (the boring-surface-schema
  pattern; see `scaffold/contracts/surface-schema.md`): a project publishes its
  dashboard's render/interaction schema and the mesh dashboard mounts it.
- **Per-project metadata and finances** eventually attach here (see the `spend`
  placeholder — renamed from finance, rounds 4–5 — in `overview.md`'s future section).
- **.mind workspace-schema migration — CONFIRMED (round-6, enthusiastically).**
  The operator's long-developed `.mind` workspace schema migrates **directly
  into projects**: **coordinator communication protocols**, **artifacts**,
  and **tasks** all come along — and the **rollup engine integrates with it**
  ("tasks are rolled up"; see `components/rollup.md`).

**Future note — `artifacts` (layer-six placeholder, no component file).**
Operator, verbatim-grade: "once we migrate to projects, no more files —
artifacts": typed, schema'd, **interactable** files — e.g. an HFT-strategy
artifact evaluated against asset artifacts via a versioned controller against a
simulator. Implies a **script execution engine**; to be built out
incrementally; "might have to be its own standalone tool, really boring and
deterministic." **Projects depends on it** once it exists. Named in
`overview.md`'s future/placeholder section; nothing designed this pass.

## Relationships / edges (stubs only)

- **kg** — the project-structure graph itself lives in the KG service
  (templates/schemas per `components/kg.md`); projects consumes KG's surfaces.
  Edge naming deferred to a later pass (no separate contract stub yet; round-3
  second batch).
- **vfs** via `projects-vfs` — the knowledge graph points into sections of the
  flat file system; project artifacts (source, dashboards) live in vfs storage
  (scaffold/contracts/projects-vfs.md).
- **mesh** via `projects-mesh` — registry push + surfacing: projects registers
  itself, pushes project/dashboard registrations, and its published dashboards
  become navigable from the mesh dashboard
  (scaffold/contracts/projects-mesh.md).
- **rollup** (round-6) — the migrated `.mind` workspace schema integrates the
  rollup engine ("tasks are rolled up"); edge naming deferred (see
  `components/rollup.md`).

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
