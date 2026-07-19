# kg

**Status:** NEW (round-3 lock, second batch, 2026-07-18; **round-8 lock,
2026-07-19: KG IS BUILT ON VDB + routing/registry semantics**).
**Nesting:** top-level.
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
- **Trigger/handler paradigm via a SHARED execution engine (rounds 4–5 lock,
  2026-07-18).** KG must support the same database-centric trigger/handler
  paradigm as `stack`: "I do want the knowledge graph to be able to process
  triggers and handlers. If we're going to have knowledge graph as a key
  function of our mesh, we need to support that paradigm, because that's how
  I program. We need an execution engine on top of knowledge graph — and it's
  very similar to the same execution engine that we need in our database."
  This is **ONE shared handler/execution-engine library** (name TBD) with
  adapters for kg-nodes and stack-tables — an internal library dependency,
  NOT a contract edge (see `overview.md`'s shared-libraries section for the
  engine's guardrails: causal chain tracking, loop detection with a
  loop-depth threshold, escalation hook). **Distributed trigger execution
  coordinates via mesh's `locks` lib** (so a trigger fires once across the
  mesh, not once per node) — see `components/mesh.md` concern 10.
- **Provenance is FIRST-ORDER (round-6, cross-cutting standing principle).**
  Traces on every execution and on every handler touch of graph data, from
  the very beginning — healthcare-data-engineer-grade provenance ("I want to
  see everything that led to the current state"); see `overview.md`'s
  standing principle. KG's trigger/handler execution inherits this via the
  shared engine's causal-chain tracking.
- **Cross-boundary sync with AWS/S3.** KGs need eventual consistency with the
  AWS/S3 side too — use case: an external agent extracts a user's intent into a
  KG and it shows up in the mesh. ~~Per the same-day S3-adapter decision, this
  flows through mesh's S3 adapter~~ — **SUPERSEDED round-9 (2026-07-19): the
  S3/AWS adapter surface lives in the new `aws` crate** (see
  `components/aws.md`). Mesh still owns the eventual-consistency/replication
  plane (KG talks to mesh; mesh orchestrates the distribution), but the
  actual S3/AWS surface it distributes through is `aws`'s (contract
  `aws-mesh`) — see `components/mesh.md` capability 9's supersession note.

- ~~**OPEN question (round-7, 2026-07-19): should KG be BUILT ON VDB?**~~ —
  **RESOLVED (round-8, 2026-07-19): YES — KG IS BUILT ON TOP OF VDB.**
  Operator, verbatim: "And KG we build on top of VDB." Part of the locked
  VDB/db/KG decomposition that resolves the stack-vs-db boundary: **VDB is
  the daemon that tracks/executes the stack pattern, using the `db` crate
  to run actions against specific databases (db extended as needed); VFS
  sits below VDB** (the SQLite files live in the VFS). Locked layer order:
  **VFS < VDB < KG.** The round-7 exploration prompt ("is the knowledge
  graph just a special version of VDB…?") is preserved in
  `components/stack.md`'s history. See `components/stack.md` /
  `components/db.md`.

## Routing + registry semantics (round-8 lock, 2026-07-19)

- **Each graph links to a project + environment**, and **the environment
  routes storage**: a **local** environment → **SQLite**; a **cloud**
  environment → **promoted to Supabase/AWS** (the operator's own routing
  rule, consistent with round-7's yes-block: "local environment → SQLite,
  cloud → promote").
- **Do-not-over-constrain nuance (recorded, deliberately open):** a
  **local-purpose KG can support a cloud environment** — the
  environment-routes-storage rule is the default, not a straitjacket; the
  linkage must not be over-constrained. Flagged as an open nuance for the
  design pass.
- **KG remains a GLOBAL registry of ALL graphs** — "a place where we can
  access ALL the knowledge graphs": one graph may serve multiple projects
  (cross-project reuse), or be referenced purely for its **schema**. The
  project+environment link does not scope a graph's visibility to that
  project.
- **Each graph routes to a location in VDB and/or VFS** — the graph's
  storage resolves to a concrete home in the layers beneath it (a
  VDB-managed database and/or VFS-resident files), per the VFS < VDB < KG
  layering.

**Consumer note:** `projects` straddles KG + VFS — the graph encodes project
structure, and graph nodes point at project files in the VFS (see
`components/projects.md`).

## Relationships / edges (stubs only)

- **mesh** via `kg-mesh` — registration (an instance of `service-lookup`) +
  replication: mesh owns the eventual-consistency/replication plane that
  distributes graph state across nodes — and into S3 through the `aws`
  crate's adapter surface (round-9 supersession; was "mesh's S3 adapter");
  the graph-merge consistency model is OPEN
  (scaffold/contracts/kg-mesh.md).
- **vfs** via `kg-vfs` — node-to-file pointers + existence validation of the
  pointed-at files (scaffold/contracts/kg-vfs.md).
- **projects** (consumer) — projects builds its project structure as a KG graph
  and points nodes at VFS files; edge naming deferred to a later pass (today it
  is captured in `components/projects.md`'s requirements, not a separate
  contract stub).
- **shared handler/execution engine** (rounds 4–5) — an internal library
  dependency (kg-nodes adapter), deliberately NOT a contract stub; its
  distributed coordination rides mesh's `locks` (see `overview.md`
  shared-libraries section).
- **VDB** (round-8, LOCKED layering; not a contract stub yet) — KG is
  BUILT ON TOP of VDB; each graph routes to a location in VDB and/or VFS
  (VFS < VDB < KG). Edge/dependency naming deferred to VDB's/KG's design
  passes. See `components/stack.md`.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet. The
distributed-consistency model for interconnected graph state is the flagged
open question. The round-7 KG-on-VDB question is **RESOLVED round-8 (YES —
KG builds on VDB; VFS < VDB < KG)**; the round-8 routing/registry semantics
are locked (environment routes storage; global registry of all graphs),
with the local-KG-supporting-cloud-environment nuance deliberately open.
