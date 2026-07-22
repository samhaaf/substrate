# projects

**Status:** L6 CONCEPTUAL DESIGN NOTE (wave 3, batch 6, unit
`projects-artifacts-landscape`), 2026-07-22. **Supersedes the wave-2 stub's
top-level-app-crate framing.** **Track: L6 conceptual only (INTENT #173c) —
"enough to know what its data contract is supposed to be with the lower
layers." No implementation, no schemas frozen.**

> ## PROJECTS IS NOT A CRATE (INTENT #166 Q5, answered; #150 beat 15)
> The wave-2 lock (INTENT #47: "projects is its own app — a graphical file
> system over the flat VFS") is **REVERSED, operator-confirmed**. The
> seed-bishop round asked it plainly: *"The projects crate was going to be a
> knowledge graph built on top of the file system. But that's now literally
> what the landscape is. So projects stops being its own app and becomes a
> [keeper] type — a project is just a node."* The operator's answer: **"projects
> not-a-crate reconfirmed"** (INTENT #166 Q5) — with one correction, don't call
> the nodes seeds (the vocabulary settled since as **keeper**). There is no
> `bin/projects`, no `lib/projects`, no `projects` daemon, no `projects/`
> registry keyspace. **`project` is the PRIMARY keeper TYPE on the landscape**
> (INTENT #150 beat 15 — "repos separate from projects... all of our projects
> living in one open-world knowledge graph — our topological graph"). This file
> is retained (not tombstoned) because it is where the load-bearing content that
> survives the crate's dissolution lives: the `.mind` workspace-schema migration
> mapping, the pushed-dashboard-registry idea (now a landscape/keeper concern),
> and the thread↔project↔environment linkage. Everything else in the wave-2
> file (the app charter, the standalone registry, the KG+VFS "straddling app"
> framing) is DROPPED as an artifact of the dissolved crate premise, not
> preserved below.

## Charter (post-dissolution)

A **project** is a **keeper type** (`keeper.project`, a `schema`-managed
type inheriting from the core keeper schema — schema.md concern 8) whose
instances are ordinary nodes on the **landscape** — the one open-world KG
(overview.md's canonical vocabulary block; kg.md). Like every keeper:

- it owns a region of the topology and may delegate to sub-keepers
  (INTENT #162; a sub-project is just another `keeper.project` node linked by
  a `contains`/`delegates-to` edge — no separate nesting mechanism);
- it has a **bundle** (persistent shared knowledge — role prompt + plugins,
  the targeted-rollup product of the node, rollup.md concern 12) and
  **threads**, each thread with a **workspace** (INTENT #161/#166 A5 row 79);
- its bundle appears via the SAME mechanism every landscape node uses —
  no bespoke "project app" presentation layer, no project-specific registry
  daemon, no project-specific rollup engine.

**What this dissolves.** The wave-2 app charter — "a centralized registry you
push to," "projects owns the project abstraction, registry, metadata,
finances," "projects is a plain VFS client for the file half," nested-projects-
as-graph-edges-owned-by-an-app — is **gone as an app concern**. The graph
region a project owns is landscape/kg's structure (kg.md, not a projects-owned
graph engine); the files a project's work points at are ordinary VFS content
(vfs.md); the rollup that produces its bundle is rollup's targeted-rollup
mechanism (rollup.md concern 12) with `keeper.project` supplying the type's
fragment/slot shape. **`project` contributes a TYPE (a schema + a rollup
fragment set), not a SERVICE.**

**What survives — three load-bearing pieces, unchanged in substance, re-homed
in vocabulary.** The rest of this file is those three pieces:

1. §1 — the `.mind` workspace-schema migration mapping (INTENT #87/#150) —
   the detailed content mapping, re-read as "onto a keeper, not onto an app."
2. §2 — the registry-of-pushed-dashboards idea — now a **keeper/landscape
   concern**, not a projects-specific registry.
3. §3 — `worktree ↔ thread ↔ project ↔ environment` linking, unchanged, via
   the `cc-projects` seam (renamed in substance to a keeper-type-scoped edge,
   contract file name kept for continuity — see Anticipated contracts).

## §1. The `.mind` workspace-schema migration (INTENT #87) — re-homed on `keeper.project`

CONFIRMED enthusiastically at the original round and never reversed: migrate
the operator's long-developed `.mind` workspace schema *"directly into
projects"* — coordinator communication protocols, artifacts, tasks, and the
rollup engine ("tasks are rolled up"). With projects dissolved into a keeper
type, "into projects" now reads **"onto the `keeper.project` type, as
landscape structure"** — the mapping content is unchanged, only its host
changes from an app to a schema+graph shape:

- **A `.mind` workspace becomes a `keeper.project` node's region of the
  landscape**, not a "project" app record. `manifest.yaml`'s **objectives**
  and **phases** (with `status`/`depends_on` DAGs) become graph nodes+edges
  under a `keeper.project`-scoped template — schema-locked by kg, so the
  objective/phase DAG invariants `mind-index validate` enforces today become
  **kg schema validation with push-back** (INTENT #50) — no change from the
  wave-2 mapping's substance. The generated `MANIFEST.md`/`OPERATOR.md`/
  `INDEX.md` projections become the node's **bundle surface** (rollup's
  targeted-rollup output, rollup.md concern 12) rather than a bespoke
  "projects surface schema."
- **`.mind` artifacts become landscape `artifact` nodes with VFS bodies** —
  frontmatter → node props (schema-locked, per an `artifact.*` schema —
  artifacts.md), body → a `vfs://` FileRef. `consumes`/`supersedes` become
  ordinary KG edges; the DAG staleness `mind-index` computes today becomes a
  kg traversal. NOTE the name collision, still true: a `.mind` "artifact" (a
  work-tracking unit) and the L6 **`artifacts`** crate's typed, methoded
  artifact are related but distinct — migrated work-units become typed
  `artifacts` once that layer is real (§ artifacts.md, this same unit).
- **Tasks are rolled up** — unchanged: a task/artifact's body is a
  `RollupTarget` resolved via **targeted rollup** (rollup.md concern 12): the
  `ScopeChain` is now derived by walking the node's `rolls-up-from` edges
  root-ward (rollup.md concern 12 point 1) instead of a projects-owned
  hierarchy walk — the exact same mechanic rollup already generalized for
  every landscape node, `keeper.project` gets it for free, no bespoke code.
- **Coordinator communication protocols migrate onto mesh** — the `.mind`
  coordination inbox (`claim`/`send`/`inbox`), handoffs, and human-actions
  become mesh-mediated: a natural **`queues`** application (inbox delivery)
  and/or `pubsub` topic, with handoffs/human-actions as graph nodes carrying
  the same `blocking`/status semantics. **This remains the heaviest genuinely-
  migrated-infrastructure surface** — it is also exactly where **OQ-6
  (keeper-to-keeper protocol, PARKED)** and **OQ-5 (curation mechanism,
  PARKED)** live; this file does not decide either, per the design-around
  rules (ledger §C). The `mind-index` tool's verbs (`validate`/`migrate`/
  `cross`/`claim`/`send`/`inbox`/`regenerate`) still need a substrate home —
  unresolved, now the **keeper runtime's** open question (keeper.md), not
  projects'.
- **`worktree ↔ thread ↔ project ↔ environment` linking** — its own section,
  §3 below, unchanged in substance from the wave-2 mapping.

**Vocabulary correction from the wave-2 text, applied throughout this file:**
where the wave-2 mapping said "project (graph region)" it now reads
"`keeper.project` node"; where it said "projects' surface" it reads "the
node's bundle"; "coordinator" reads "keeper" per the locked vocabulary.

## §2. The pushed-dashboard registry — now a keeper/landscape concern

Wave-2's projects app was going to own *"a centralized registry you push to —
dashboards AND source code"* with project-published dashboards navigable from
the main mesh dashboard (INTENT #47). **That registry does not become a
`projects` service — there is no `projects` service left to own it.** The idea
survives, re-homed:

- **Publication is a landscape-wide keeper/artifact behavior, not a
  project-specific one.** Any keeper (not only `keeper.project`) or artifact
  node MAY publish a dashboard component via the **boring surface-schema
  pattern** (INTENT #46) — the same mechanism `dashboard-serving`/mesh-core
  already provide every service. A `keeper.project` node publishing its
  dashboard is one instance of a general capability, not a projects-owned
  registry daemon.
- **Discoverability rides the landscape itself, not a bespoke `projects/`
  keyspace.** "A centralized registry you push to" is satisfied by: (a) the
  node existing on the landscape (kg's global graph registry already answers
  "what nodes exist, where," kg.md concern 2 — no second registry needed),
  and (b) the node's published surface-schema being discoverable the way every
  service's is (`service-lookup` + `surface-schema`, not a project-specific
  push). The wave-2 file's `projects/` replicated-kv keyspace idea is
  **DROPPED** — it was solving a problem the landscape's own registry already
  solves once `projects` is a node type, not an app.
- **Source code** — still ordinary VFS content pointed at by the node
  (`vfs://` FileRef props, existence-validated by kg — kg.md concern 5); no
  change from the wave-2 mapping's substance, just no projects-owned VFS
  client layer sitting in front of it.
- **The topological-map UI** (INTENT #43) is now literally **the landscape
  dashboard** (overview.md, dogfooding — INTENT #150 beat 16: "one dashboard
  showing the topological map and every published data type"), not a
  projects-specific UI. Recorded here so the idea isn't lost, designed there.

## §3. `worktree ↔ thread ↔ project ↔ environment` linking — the `cc-projects` seam

Unchanged from the wave-2 mapping in substance; only the noun on the
project-side of the edge changes (a `keeper.project` node, not a `projects`
app record):

- The `.mind` workspace schema binds a git **worktree** and a CC **thread**
  together (INTENT #67). The **thread** half is the `cc-projects` edge: the
  `coordinator_thread_id` takeover mechanism and CC-thread↔project links
  become cc's `agent_runs` rows linking threads to `keeper.project` nodes (+
  optional environment), with the **keeper.project node** (not a "projects"
  service) the resolution authority for takeover-vs-continue (match/differ/
  unavailable — preserved as a keeper-level protocol, not redesigned here).
- The **worktree/branch** half stays **repo's** (repo.md concern 2 already
  reuses the identical `.mind` worktree↔thread↔branch schema and owns
  `repo-vfs` + `repo-environments`). A `keeper.project` node references its
  repo workspace transitively through the shared worktree↔thread↔branch
  identity, or via `repo-environments` — **not decided here** (this was an
  open question in the wave-2 file and remains one; see OQ-16/OQ-4 below).
- **Per-project metadata + finances** stay pull-shaped, unchanged: `spend`
  queries cc's usage ledger grouped by `project_id`/`environment_id`
  (`spend-cc`), with the `keeper.project` node the grouping authority those
  ids resolve against (INTENT #41/#68/#166 Q10 — spend anchored to landscape
  nodes). No project-owned finance store.
- **Environments.** A project (`keeper.project` region) may have 1+ repos,
  each with branches deploying into environments — this is **OQ-2, PARKED**
  (the operator's own "we're not there yet — full dedicated deep-dive
  required" — INTENT #164). This file does not decide environment↔repo
  cardinality or branch-deploy semantics; `environments.md` stays a
  requirements-only stub per the design-around rules.

## Open questions carried forward (not decided here, per L6/#173d)

1. **OQ-16 / OQ-4 — git + projects + environments + keeper elegance,
   bottom-up topography.** PARKED/OPEN. Who asserts a `contains`/`delegates-to`
   edge and when; what a keeper-merge does to a workspace/bundle on branch
   merge; whether a `keeper.project` differs per branch/environment. Design-
   around: `has-repo` stays a **candidate** many-to-many edge, not resolved
   (ledger §C).
2. **The `.mind`-migration coordinator-protocol surface (heaviest, PARKED via
   OQ-5/OQ-6).** Whether the inbox/handoff/human-action machinery rides
   `queues`+`pubsub`, becomes graph nodes with a thin protocol, or both — the
   keeper runtime's open question now, recorded here for lineage since it was
   raised in this file first (§1).
3. **`.mind` "artifact" vs the `artifacts` crate.** When `artifacts` (typed,
   methoded landscape citizens — this same unit's other file) is live, do
   migrated `.mind` work-units *become* artifacts, or sit beside them as plain
   KG nodes? Leaning "become," per artifacts.md's charter — not locked here.
4. **Registry storage detail** (§2) — whether a project's published-dashboard
   discovery needs any project-scoped index at all beyond kg's global registry
   + `service-lookup`/`surface-schema`; leaning "no new index," not locked.
5. **`environments`/`cicd`** stay RESERVED/stub per OQ-2/OQ-32 — no
   `keeper.project`-specific environment design happens in this file.

## Anticipated contracts (wave 3, L6)

Per INTENT #173c: enough to know the data contract with the lower layers, not
the internals. `keeper.project` is a **type**, not a service — every contract
below is "the type's schema/rollup fragment set consumes X," not "an app
speaks to X over the mesh daemon."

- **`projects/landscape ↔ keeper`** (conceptual seam, not a wire contract —
  keeper.md, batch 6, authors the runtime this pins against). **Purpose:**
  `project` is the PRIMARY keeper type (INTENT #150 beat 15); this file
  defines what a `keeper.project` node needs from the keeper runtime.
  **Rough shape:** keeper.md's core keeper schema (bundle structure +
  workspace-attachment structure, schema.md concern 8) is the PARENT
  `keeper.project` inherits from (single inheritance in the common case; the
  operator's dual-inheritance case — merging two keeper-type branches — is
  schema's multiple-inheritance + explicit-conflict-resolution mechanism,
  unchanged, schema.md concern 2); `keeper.project` supplies only its
  additive layer (a `contains`/`sub_project` edge affinity, the
  `.mind`-derived objective/phase/artifact node-type set from §1). The keeper
  runtime instantiates threads FROM a `keeper.project` node exactly as from
  any keeper — no project-specific spawn path.

- **`projects-kg`** (a `keeper.project` node's structure; kg.md is the
  substrate). **Purpose:** the project-structure graph — nested hierarchy
  (as ordinary `contains` edges between `keeper.project` nodes, not an
  app-owned tree), the objective/phase/artifact node types from §1, cross-
  links to repos/environments. **Rough shape:** `kg-api` verbatim, no
  projects-authored template SERVICE — the `keeper.project` **schema**
  (published through `schema`, not authored as a bespoke kg template) carries
  the node/edge type set (`sub_project`, `workspace`, `artifact`, `task`,
  `objective`, `phase`, `handoff`, `human_action`; edges `contains`,
  `depends_on`, `consumes`, `supersedes`, `blocks`) — same content as the
  wave-2 `project-tree@1` template sketch, now expressed as a `schema`
  definition instead of a kg-local template, per schema.md concern 9's
  extraction seam.

- **`projects-rollup`** (a `keeper.project` node's bundle; rollup.md concern
  12 is the substrate). **Purpose:** *"tasks are rolled up"* + the node's
  bundle. **Rough shape:** targeted rollup pointed at the node — `Rollup
  (keeper.project_node) → bundle`; the `ScopeChain` is derived by walking
  `rolls-up-from` edges (rollup.md concern 12 point 1), replacing the wave-2
  "projects derives the chain from the hierarchy" with the landscape-wide
  mechanic every node already gets. No project-specific rollup surface.

- **`projects-vfs`** (a `keeper.project` node's file bodies; vfs.md /
  `vfs-content` is the substrate). **Purpose:** source code, dashboard
  bundles, and artifact bodies pointed at by the node's `vfs://` FileRef
  props. **Rough shape:** plain kg-mediated FileRef existence-validation
  (`kg-vfs`) — `keeper.project` never opens a VFS connection itself; it is a
  schema (field types), not a client.

- **`projects-vdb`** (databases attached to a project region; vdb.md is the
  substrate). **Purpose:** databases/stacks attached to a project or its
  sub-regions (INTENT #43: a project *"can also have file systems, workspaces,
  databases, and services attached"*). **Rough shape:** unchanged from the
  wave-2 sketch — VDB-managed databases keyed by `keeper.project` node id (+
  environment); environment routing is VDB's copy/verify/switch mechanic,
  inherited. `keeper.project` records the linkage in its schema's fields; VDB
  owns lifecycle.

- **`cc-projects`** (thread↔`keeper.project`(+environment) linkage; cc.md is
  the authoring side). **Purpose:** unchanged from §3 — `agent_runs.
  project_id?`/`environment_id?` validated against `keeper.project` nodes once
  the keeper runtime is live; `keeper.project` reads cc's per-node usage
  rollup back (feeding `spend-cc`). **Rough shape:** no new mechanism — cc's
  existing ledger fields, now resolved against landscape node ids instead of
  a `projects` app's ids. Contract file name kept (`cc-projects.md`) for
  continuity; its party on this side is the keeper concept, not an app.

- **`projects-artifacts`** (a `keeper.project` node's typed work-units; see
  `artifacts.md`, this same unit). **Purpose:** migrated `.mind` work-units
  (§1) becoming typed, methoded artifacts. **Rough shape:** deferred to
  `artifacts.md`'s own anticipated-contracts section — named here so the pair
  exists in the graph.

## Relationships / edges (summary)

Type-level consumption (not service edges): **schema** (`keeper.project`
inherits the core keeper schema; schema.md concern 8), **kg** (structure,
`projects-kg`), **rollup** (bundle + task rollup, `projects-rollup`), **vfs**
(file bodies, `projects-vfs`, via kg-mediated pointers), **vdb** (attached
databases, `projects-vdb`), **artifacts** (typed work-units,
`projects-artifacts`). Party to: **cc** (`cc-projects`, thread linkage),
**spend** (via `spend-cc`, pull-shaped). Cross-cutting (consumed via the
keeper runtime, not authored here): `surface-schema`, `service-lookup`,
`pubsub-protocol`, `queues-api` (for the migrated inbox, §1).

## Nesting

None. `project` is a **type**, not a crate — no `bin/`, no `lib/`, nothing to
nest. It contributes: a `schema` definition (`keeper.project`), a rollup
fragment set (its bundle's content shape), and the migrated `.mind` node/edge
type vocabulary (§1). All three are DATA published into their respective
tools (schema/rollup/kg), not code.

## Thoroughness level

**L6 conceptual (INTENT #173c)** — enough to know the data contract with kg,
schema, rollup, vfs, vdb, cc, and artifacts; no schemas frozen, no
implementation, no service. The three load-bearing wave-2 ideas (`.mind`
migration mapping, pushed-dashboard registry, thread/project/environment
linking) are preserved in substance and re-homed onto the keeper-type
framing.
