# projects

**Status:** WAVE-2 REFIT of the round-3 stub (batch 7, L6 organization plane),
2026-07-19. **Track: STUB — design notes + anticipated data contracts only.**
**Nesting:** top-level app-crate (`bin/projects` + `lib/projects`), the graphical
file system / knowledge-graph layer over the flat VFS.

> ## NOT IMPLEMENTING NOW (INTENT #51/#43, layer-6 stub track)
> This file captures operator intent and names the data contracts projects will
> need. It is deliberately **requirements-only**: no schemas, no wire formats, no
> implementation. Per INTENT #43 the operator scoped this down explicitly —
> *"even if we don't solve them right now, maybe we can get toward what our ideal
> full directory structure looks like, and just have some design stubs and data
> contracts — only the first two layers of services... That's more of just a
> discussion point."* Everything below is a placeholder with a real design note;
> content lands when projects leaves the stub track (after `artifacts` exists).

## Charter (requirements)

`projects` is **the graphical file system over the flat `vfs`** — INTENT #47,
LOCKED and verbatim:

> "VFS should be our flat file system, and then projects can be a graphical file
> system which points to sections within our flat file system... projects is
> basically a knowledge graph built on top of the VFS that allows us to build
> applications... hierarchically... and access them in a really friendly way."

It **straddles KG + VFS** (INTENT #51): the **graph encodes a project's
structure** and lives in the Knowledge Graph service (`kg.md`); the **graph's
nodes point at project files in the VFS** (`kg-vfs` existence-validated
pointers). Projects is a *consumer/composer* of both — it owns no graph engine
and no byte storage; it owns the **project abstraction, its registry, its
metadata/finances, and the migrated `.mind` workspace schema**. Requirements:

- **The place where applications get built** — **hierarchically nested
  projects** (INTENT #43: *"projects within projects, automatically distributed
  across the VFS, tracking finances per project"*; the disk-location binding is
  dropped — a project is a graph region, not a `~/code/<name>` folder).
- **A centralized registry you push to** — **dashboards AND source code** (INTENT
  #47), accessible from anywhere on the mesh.
- **Project-published dashboards navigable from the main mesh dashboard** — via
  projects' own **surface schema** (the boring-surface-schema pattern, INTENT
  #46; `surface-schema.md`): a project publishes its dashboard's
  render/interaction schema and the mesh dashboard mounts it.
- **Per-project metadata + finances** attach here — the **`spend` hookup**
  (INTENT #41/#68): spend queries CCD's usage ledger **pull-shaped, grouped by
  project/environment**, and projects is the grouping authority those
  `project_id`s resolve against. A "topological map" UI is the operator's stated
  interface intent (INTENT #43).
- **.mind workspace-schema migration — CONFIRMED enthusiastically (INTENT #87).**
  The operator's long-developed `.mind` workspace schema migrates **directly into
  projects** (see the dedicated design note below).

## Design notes (operator intent, faithful)

### 1. Projects straddles KG + VFS — the load-bearing shape

A project is **a KG graph** (its own graph in the global registry, bound to a
projects-authored template — sketch name `project-tree@1`) whose **nodes point at
VFS files**. The division mirrors the layers exactly:

- **KG owns structure.** The project hierarchy (project → sub-project → workspace
  → artifact/task), the cross-links between regions, and the schema-locking that
  keeps a project graph well-formed are all `kg` concerns — projects defines a
  template and calls `kg-api`, it does not re-implement graph semantics. Nested
  projects are edges in the graph (a `contains`/`sub_project` edge type on the
  template); KG's bounded traversal (`Traverse`, depth+limit mandatory) does the
  hierarchy walks projects' topological-map UI renders.
- **VFS owns bytes.** Source code, dashboard bundles, and file bodies are
  ordinary VFS files (`Immutable` blobs, or `NodeAnchored` for live working
  trees — vfs.md concern 2); a project-graph node carries a `vfs://` FileRef that
  KG existence-validates (`kg-vfs`, sweep-checked). Projects is a **plain VFS
  client** for the file half (`projects-vfs`): no new storage wire.
- **Projects owns neither** — it owns the *project abstraction* over the two:
  registry, metadata/finances, the `.mind` schema, and the surface that makes it
  navigable. This keeps projects a genuine L6 organization-plane composer, not a
  fourth storage engine.

The flat VFS "just looks like an S3 bucket" (INTENT #43/vfs.md concern 11); the
friendly, navigable, project-oriented structure is *this* strictly-higher layer.

### 2. Nested projects

Hierarchically nested, encoded as graph edges (no filesystem nesting — a project
is not bound to a disk location). The **naive environment rule** carries over
(INTENT #63/#75, environments.md): sub-projects have **no relationship between
their environments by default**; where a relationship exists, a sub-project
**shares its parent's environment by default**. Finance/metadata rollup follows
the same containment edges (a parent project's spend is the rollup of its
sub-projects' — computed over the graph, not stored redundantly).

### 3. The centralized registry + mesh-dashboard navigation

A single mesh-wide registry you **push to** (INTENT #47) — both **dashboards and
source**. Registration and surfacing ride `projects-mesh` (an instance of
`service-lookup` + the registry push) and `surface-schema`: each project
publishes its dashboard's boring render/interaction schema, and the main mesh
dashboard **mounts project dashboards schema-driven** (never hand-built per
project — INTENT #46). The registry itself is small control-plane state and is a
natural `replicated-kv` keyspace tenant (a `projects/` keyspace, the exact
precedent vfs's `vfs/` and kg's `kg/` keyspaces set) so every node can enumerate
projects from a local read — but that is an implementation choice flagged, not
locked, for the real design pass.

### 4. Per-project metadata + finances (the spend hookup)

Per-project **metadata** (name, description, owning agent once `org` exists,
lifecycle) is project-graph data. **Finances** are NOT stored in projects — they
are **derived pull-shaped**: `spend` queries CCD's `usage_records ⨝ agent_runs`
(ccd.md's ledger) grouped by `project_id`/`environment_id` (`spend-ccd`, spend's
edge), and projects is the authority those ids resolve against. So projects
**does not push or duplicate cost data**; it is the grouping dimension. This
honors INTENT #41 (spend is pull-shaped, sources never push) and #68 (CCD owns
the usage DB; spend queries it). CCD threads already carry
`agent_runs.project_id?`/`environment_id?` (ccd.md concern 2) — the
`ccd-projects` edge is where projects validates and reads them back.

### 5. The `.mind` workspace-schema migration (INTENT #87) — the heavy note

CONFIRMED enthusiastically: migrate the operator's long-developed `.mind`
workspace schema **directly into projects** — *"coordinator communication
protocols, artifacts, tasks — and bring the rollup engine into it ('tasks are
rolled up')."* The real schema to migrate is the harness `workspaces` skill
(`~/code/harness/core-plugins/core/skills/workspaces/SKILL.md`, read in full);
the mapping onto substrate's layers, faithfully:

- **A workspace becomes a project (graph region).** The `.mind` workspace
  (`manifest.yaml` + `MANIFEST.md` projection + `INTENT.md`) maps to a project
  node in the KG graph. `manifest.yaml`'s **objectives** and **phases** (with
  their `status`/`depends_on` DAGs) become graph nodes+edges under the
  `project-tree` template — schema-locked by KG, so the objective/phase DAG
  invariants `mind-index validate` enforces today become **KG schema
  validation with push-back** (a malformed objective graph is rejected at write
  time, INTENT #50). The generated `MANIFEST.md`/`OPERATOR.md`/`INDEX.md`
  projections become projects' **surface schema** output — the same
  "frontmatter is the orchestrator's API, the manifest is a generated
  projection" discipline, now graph-derived.
- **`.mind` artifacts become graph nodes with VFS bodies.** Each
  `artifacts/*.md` (the two-tier INTERMEDIATE/NAMED naming, the frontmatter
  schema — `id`/`type`/`status`/`consumes`/`supersedes`/`open_questions`/`say`)
  becomes a project-graph node: **frontmatter → node props (schema-locked),
  body → a `vfs://` FileRef.** The `consumes`/`supersedes` edges are graph
  edges — the DAG staleness (`stale-input` when a consumed input's rev moves)
  that `mind-index` computes today becomes a KG traversal. NOTE the name
  collision, resolved: a **`.mind` "artifact"** (a work-tracking unit) is
  distinct from the L6 **`artifacts` crate** (typed *interactable* files); the
  migrated work-units are graph nodes now and become `artifacts`-crate typed
  artifacts once that tool exists (see §6).
- **Tasks are rolled up (the rollup integration).** *"Tasks are rolled up"*
  (INTENT #87) — a task/artifact's body is a `RollupTarget` projects hands to
  `rollup` (`projects-rollup`): projects derives the **`ScopeChain`** from the
  project hierarchy (replacing rollup's v1 caller-passed chain — rollup.md
  concern 2/§`projects-rollup` already reserves this) and asks rollup to resolve
  it. No new rollup mechanism — the existing `Resolve`/`Materialize` surface
  with a projects-supplied scope; all of rollup's invariants (secret-safety,
  provenance, determinism) hold unchanged.
- **Coordinator communication protocols migrate onto mesh.** The `.mind`
  **coordination inbox** (`claim`/`send`/`inbox`, the passive
  one-coordinator-per-workspace convention), **handoffs**, and **human-actions**
  become mesh-mediated: the inbox is a natural **`queues`** application
  (message delivery into a recipient project's inbox) and/or `pubsub` topic;
  handoffs and human-actions become graph nodes with the same `blocking`/status
  semantics. This is the one part that is genuinely *migrated infrastructure*,
  not just re-homed data — flagged as the heaviest design surface for the real
  pass (the harness `mind-index` tool's behaviors become projects' service
  verbs).
- **`worktree ↔ thread ↔ project ↔ environment` linking (ccd.md + repo.md,
  batch 6).** The `.mind` workspace schema binds a git **worktree** and a CC
  **thread** together (INTENT #67, the "prior art to reuse"); the migration
  splits ownership by layer. The **thread** half lands as the `ccd-projects`
  edge: the `coordinator_thread_id` takeover mechanism and the CC-thread↔project
  links become CCD's `agent_runs` rows linking threads to projects+environments,
  with projects the resolution authority (takeover-vs-continue —
  match/differ/unavailable — preserved as a project-level protocol). The
  **worktree/branch** half is **repo's** — repo.md concern 2 already reuses the
  same `.mind` worktree↔thread↔branch schema verbatim and owns `repo-vfs`
  (`NodeAnchored` worktrees) + `repo-environments` (branch→environment
  attachment). Projects does not re-own worktrees; it references a project's
  repo workspace the way it references the thread. NOTE: wave2-plan §3 lists **no
  `projects-repo` pair** — see open question #7.

### 5b. Coordinators — a FIRST-ORDER concept (friction-round 2, INTENT #123; recorded, NOT designed)

Friction-round 2 elevated **coordinators** from an implementation detail of
the `.mind` migration (§5) to a first-order concept **requiring a dedicated
discussion round** before any design. The operator's sketch, verbatim-grade:

- **Coordinators attach to workspaces; one coordinator per workspace.**
  Workspaces often attach to worktrees (branches with their own directory).
- **Workspaces could be pulled out** as their own thing, **built on top of
  VDB**.
- "I really only like to talk to coordinators — I don't like talking to
  individual agents at all." AUI threads = a coordinator with a workspace
  attached.
- Coordinators live inside a project on a branch; **the workspace merges
  into other branches with it**.
- **Coordinator compaction:** "I should be able to compact the thread and
  basically lose nothing" — compact anytime, lose nothing.
- **Coordinator-as-a-SERVICE** is worth considering.
- The **git + projects + environments + coordinators** interplay is
  currently "a bunch of disparate things" that must be made elegant
  together — this elegance problem is itself part of the discussion's
  charter.

Nothing here is designed; this note exists so the coordinators discussion
starts from the operator's own framing. (INTENT #127's **owners** are
defined in terms of coordinators — see org.md and overview.md.)

### 6. `artifacts` dependency (INTENT #51)

Operator, verbatim-grade: *"once we migrate to projects, no more files —
artifacts."* An artifact = a typed, schema'd, **interactable** file (e.g. an
HFT-strategy artifact run via a versioned controller against a simulator) —
implying a **script-execution engine, built incrementally**, *"might have to be
its own standalone tool, really boring and deterministic."* **Projects depends on
`artifacts`** once it exists (`projects-artifacts`): the migrated work-unit
graph nodes (§5) become typed `artifacts`-crate artifacts. `artifacts` is a
co-batched L6 stub (batch 7 creates `components/artifacts.md`); nothing designed
here.

## Anticipated contracts (wave 2, stub track)

Pairs named only — purpose + rough shape, no schemas. Content lands when projects
leaves the stub track. I do NOT edit `scaffold/contracts/*`. Existing stubs
(`projects-vfs`, `projects-mesh`) are refit in place by note; the rest are
anticipated per wave2-plan §3c.

- **`projects-vfs`** (projects → vfs; existing stub). **Purpose:** projects
  reads/writes ordinary VFS files for project source, dashboard bundles, and
  artifact bodies, and reads placement/policy for its topological-map UI. **Rough
  shape:** projects is a *plain VFS client* — vfs.md's `Read`/`Write` on the
  `vfs-content` surface plus prefix scans; **no new wire** (vfs.md's
  storage-side note already pins this). The graph over the files is KG's, not
  this edge's.

- **`projects-mesh`** (projects ↔ mesh; existing stub). **Purpose:** registration
  (a `service-lookup` instance), the **centralized registry push**
  (project/dashboard registrations), and **surface-schema publication** so
  project dashboards are navigable from the mesh dashboard. **Rough shape:**
  ordinary `service-lookup` registration + a `projects/` registry keyspace
  (replicated-kv, flagged) + `surface-schema` output; project dashboards mount
  schema-driven. No bespoke routing (aggregation never becomes routing —
  dashboard-serving.md).

- **`projects-kg`** (projects → kg; stub-track). **Purpose:** the
  project-structure graph — nested hierarchy, cross-links, node→VFS pointers.
  **Rough shape:** kg's side is `kg-api` **verbatim** plus a projects-authored
  **`project-tree@1` template** (node types: `project`, `sub_project`,
  `workspace`, `artifact`, `task`, `objective`, `phase`, `handoff`,
  `human_action`; edge types: `contains`, `depends_on`, `consumes`,
  `supersedes`, `blocks`) — kg.md already records projects' half is `kg-api` +
  this template, so projects starts from a concrete substrate half. Environment
  routing (local→SQLite / cloud→promoted) is inherited per-graph from KG.

- **`projects-rollup`** (projects → rollup; stub-track). **Purpose:** *"tasks are
  rolled up"* — projects derives the `ScopeChain` from the project hierarchy and
  resolves task/artifact bodies via rollup. **Rough shape:** the existing
  `rollup-mesh` `Resolve`/`Materialize` surface with a **projects-supplied
  scope** (rollup.md's `projects-rollup` note reserves exactly this); no new
  mechanism, every rollup invariant unchanged.

- **`projects-vdb`** (projects → vdb; stub-track). **Purpose:** databases
  attached to projects/environments (INTENT #43: a project *"can also have file
  systems, workspaces, databases, and services attached"*). **Rough shape:** a
  project's attached stack databases are VDB-managed databases keyed by
  project/environment; environment routing (local→SQLite-in-VFS / cloud→promote)
  is VDB's copy/verify/switch mechanic, inherited — projects records the linkage
  and resolves ids, VDB owns the database lifecycle. Reconciles with
  `environments-vdb` (environment routes storage) at the per-pair round.

- **`projects-artifacts`** (projects → artifacts; stub-track). **Purpose:**
  projects depends on `artifacts` for typed, interactable work-units once that
  crate exists (§6). **Rough shape:** deferred entirely — `artifacts` is
  un-designed (co-batched batch-7 stub). Named so the pair exists.

- **`ccd-projects`** (ccd ↔ projects; stub-track — CCD authored the ledger side).
  **Purpose:** thread↔project(+optional environment) linkage (INTENT #68,
  ccd.md concern 2). **Rough shape:** CCD's `agent_runs.project_id?`/
  `environment_id?` are set at spawn and **validated against projects** once
  projects leaves the stub track; projects reads CCD's per-project agent/usage
  rollup back (feeding the finance/metadata view, §4). No new mechanism — the
  fields already live in CCD's ledger and `agent-management`. Content deferred.

## Relationships / edges (summary)

Consumes: **kg** (`projects-kg`, structure graph), **vfs** (`projects-vfs`, file
bodies), **rollup** (`projects-rollup`, task rollup), **vdb** (`projects-vdb`,
attached databases), **mesh** (`projects-mesh`, registry + surface), **artifacts**
(`projects-artifacts`, future typed work-units). Party to: **ccd**
(`ccd-projects`, thread linkage), **spend** (via `spend-ccd`, projects is the
grouping authority — spend's edge, not projects'). Cross-cutting (consumed, not
authored): `surface-schema`, `service-lookup`, `pubsub-protocol`,
`restart-protocol`, and — for the migrated coordinator inbox — `queues-api`.

## Nesting

Parent: none (top-level app-crate). Children: none designed this pass (a real
design pass would decompose into libs: a `registry` lib over the `projects/`
keyspace, a `graph` lib driving `kg-api` + the `project-tree` template, a
`workspace` lib for the migrated `.mind` schema, a `surface` lib). Flagged, not
built.

## Open questions

1. **Coordinator-protocol migration is the heaviest surface** — does the `.mind`
   inbox/handoff/human-action machinery ride `queues` (message delivery) +
   `pubsub`, or become project-graph nodes with a thin protocol, or both? The
   harness `mind-index` tool's verbs (`validate`/`migrate`/`cross`/`claim`/
   `send`/`inbox`/`regenerate`) all need a substrate home. Deferred to the real
   pass.
2. **`.mind` "artifact" vs the `artifacts` crate** — the migrated work-units are
   graph nodes now; when `artifacts` (typed interactable files) lands, do they
   *become* artifacts, or does `artifacts` sit beside them? Depends on
   `artifacts`' un-written design (co-batched batch 7).
3. **`projects-secrets`?** — secrets.md/INTENT #99 names `projects` among
   secrets' interfaces, but wave2-plan assigns projects no `projects-secrets`
   pair. Likely projects attaches secrets to a project/environment via
   `secrets-environments` rather than a direct edge — flagged for the operator /
   per-pair round, not assumed.
4. **Registry storage** — a `projects/` `replicated-kv` keyspace (per the vfs/kg
   precedent) vs. project registry living entirely in KG. Leaning KV for the
   cheap-local-enumeration property, but not locked.
5. **The topological-map UI** (INTENT #43) — a real design problem (schema-driven
   graph rendering feeding the mesh dashboard) parked with the rest of the stub.
6. **Coordinators (INTENT #123) — dedicated discussion required.** A
   first-order concept, not a migration detail: coordinator-per-workspace,
   workspaces possibly pulled out on top of VDB, coordinator-as-a-service,
   coordinator compaction (compact anytime, lose nothing), and the
   git+projects+environments+coordinators elegance problem (see §5b). This
   supersedes the assumption that §5's coordinator-protocol migration fully
   covers the concept.
7. **Missing `projects-repo` pair** — the `.mind` workspace schema projects
   migrates binds a git worktree (INTENT #67); repo owns the worktree half
   (repo.md concern 2 reuses the same schema). But wave2-plan §3 names no
   `projects-repo` contract, only `ccd-projects` (thread) and
   `repo-environments` (branch→env). Does a project reference its repo workspace
   through an unlisted `projects-repo` edge, purely transitively via the shared
   worktree↔thread↔branch identity, or via `repo-environments`? Flagged for the
   operator / per-pair round — not assumed, and not invented here.

## Thoroughness level

**requirements-only** — verbatim intent capture + anticipated contract sketches;
no design pass, no schemas, no implementation. Layer-6 stub track (INTENT #51):
"not implementing now."
