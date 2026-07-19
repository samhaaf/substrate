# kg

**Status:** NEW (round-3 lock, second batch, 2026-07-18). **Nesting:** top-level.
**Stub-plus — requirements captured from the operator; NOT a full design.**
Build-now, not a placeholder — operator's framing, verbatim: build it now
"because it's boring... very important, very useful."

## Charter (requirements, operator's words where quoted)

The **Knowledge Graph service**: a **distributed graph structure across the
entire mesh**. Requirements:

- **Distributed consistency is the hard, OPEN design question.** Unlike the KV
  store (the service registry's naive timestamp-wins / LWW model), a graph with
  many interconnected nodes is "the superset basically" — merging concurrent
  edits to an interconnected structure is strictly harder than per-key LWW.
  Flagged as an open design question, NOT decided here. Replication/
  distribution itself rides mesh (see `kg-mesh` and the S3 note below).
- **Graph lifecycle + schema locking.** KG can initialize new graphs; each
  graph **locks in schemas on nodes AND edges**. A change that fails schema
  validation is **pushed back to the calling service** (rejected, caller must
  conform), not silently coerced.
- **Graph TEMPLATES, with versions.** A template is a reusable set of node/edge
  schemas for a graph type; each template **version pins its schemas**. Known
  template types to support from the start:
  - **research graphs** — hypotheses, experiments, conclusions, methods;
  - **Obsidian-style markdown-linking graphs**;
  - **`.mind`-style axiomatic breakdowns**.
- **Nodes can point to files in the VFS**, with **existence validation** of the
  pointed-at file (`kg-vfs`).
- **Cross-boundary sync with AWS/S3.** KGs need eventual consistency with the
  AWS/S3 side too — use case: an external agent extracts a user's intent into a
  KG and it shows up in the mesh. Per the same-day S3-adapter decision, this
  flows **through mesh's S3 adapter** (mesh owns all eventual-consistency/
  replication; KG talks to mesh, mesh distributes into S3) — see
  `components/mesh.md`.

**Consumer note:** `projects` straddles KG + VFS — the graph encodes project
structure, and graph nodes point at project files in the VFS (see
`components/projects.md`).

## Relationships / edges (stubs only)

- **mesh** via `kg-mesh` — registration (an instance of `service-lookup`) +
  replication: mesh owns the eventual-consistency/replication plane that
  distributes graph state across nodes and into S3 through mesh's S3 adapter;
  the graph-merge consistency model is OPEN
  (scaffold/contracts/kg-mesh.md).
- **vfs** via `kg-vfs` — node-to-file pointers + existence validation of the
  pointed-at files (scaffold/contracts/kg-vfs.md).
- **projects** (consumer) — projects builds its project structure as a KG graph
  and points nodes at VFS files; edge naming deferred to a later pass (today it
  is captured in `components/projects.md`'s requirements, not a separate
  contract stub).

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet. The
distributed-consistency model for interconnected graph state is the flagged
open question.
