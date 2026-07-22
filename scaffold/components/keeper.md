# keeper — the topological node on the landscape (replaces org)

**Status:** NEW — the **central L6 conceptual design of wave 3** (batch 6),
2026-07-22. **This file REPLACES `org`** as the layer-6 organization mechanism
(INTENT #148 beat 1 / #132: "this is the actual mechanism of the org instead of
some arbitrary concept of an org in a crate"). `org.md` is retained as a
tombstone + design-lineage record; the coordinator/owner/deva/bishop material it
holds resolves HERE.

**Track — L6 CONCEPTUAL, NO CRATE THIS PASS (INTENT #148 beat 1 / directive
#173c; OQ-17 answered).** The operator, verbatim (#148 beat 1): *"design the
coordinator runtime this pass, build no crate; it's layer-6 and replaces org in
layer 6."* Directive #173c scopes the depth: design "enough to know what its data
contract is supposed to be with the lower layers." So this file designs the
**contracts with the lower layers concretely** (the deliverable — see
*Anticipated contracts*) and keeps the keeper's own internals at **design-notes
depth**. **OQ-17 (keeper-as-crate-now) is thereby answered: capture-now,
design-runtime-only — no `keeper` crate, no registry slug, no wire of its own is
built this wave.** Whether the keeper runtime later becomes a crate, a library
hosted inside `cc`/`chassis`, or a face over kg+rollup+queues is left OPEN (see
open questions) — nothing here forecloses it.

> **⚠ NOT IMPLEMENTING NOW.** Conceptual design + anticipated lower-layer
> contracts only. No frozen schemas, no fill-model. Every mechanism named here
> is realized by ALREADY-DESIGNED lower layers (`kg`, `rollup`, `schema`,
> `queues`, `cc`, `vdb`) — the keeper is their **composition**, not a new engine.
> The two-engines rule (schema.md F-2) is absolute: **rollup composes TEXT; kg +
> schema own STRUCTURE; the keeper owns neither — it owns the org-seat CONCEPT
> over them.**

---

## Charter — what a keeper IS

A **keeper is a topological node on the landscape** (LOCKED vocabulary, INTENT
#162/#172). It is, simultaneously and by design, three things the operator fused:

- **A node on the landscape** — the one open-world topological KG (`kg`) in which
  all projects, sub-projects, apps, components, and artifacts live (INTENT #150
  beat 15 / #165). A keeper **owns its region of the topology** and delegates to
  sub-keepers in that region ("monuments on the landscape, obelisks… it owns the
  surrounding land and can delegate to the other sub-keepers in the area," #162).
- **An org-chart seat** — "each topological node is an org-chart seat… you split
  an org by topology, not by which humans you can get" (#162). Instead of a
  person with expertise, a keeper is "a knowledge graph with curated information…
  embedded into a thinking machine." **`org` is emergent** from keepers + their
  typed edges (#132) — there is no org crate; the org IS the keeper graph.
- **A persistent, idle-until-woken agent-home** — "an agent but not an agent…
  it has persistent memory, it owns part of the topological map, like a daemon"
  (#149 beat 6). **coordinator = owner = keeper** — the same thing, accessed
  sometimes directly (a human thread) and sometimes via a message (#127/#130).

A keeper **holds a BUNDLE** and **spawns THREADS**, each thread with an
ephemeral **WORKSPACE**. That triad is the whole keeper (INTENT #161/#172):

> "The topological node has a shared-memory component, which can be edited like
> an artifact. And it also has threads, and each thread has a workspace (like
> today: one repo, many workspaces each with a thread; nothing shared except
> .mind)." (#161)

**Project is the PRIMARY keeper type** (#150 beat 15 / #162); app, component,
and business/strategic seats are other types, all inheriting from a core keeper
type (see *Keeper-type schemas*).

### Boundary — what the keeper does NOT own

- **Not graph structure, node identity, or versioning** — that is `kg`
  (structure) + `schema` (types). The keeper is a `kg-api` consumer.
- **Not text composition or bundle mechanics** — that is `rollup`. The keeper's
  bundle IS the targeted-rollup product of its node (rollup.md concern 12); the
  keeper **references** that mechanism and never redesigns it.
- **Not thread execution / process supervision / usage governance** — that is
  `cc`. A keeper's threads are Claude Code processes cc spawns, meters, and
  reaps by handle (`agent-management`); the keeper is a caller with a priority.
- **Not durable message delivery** — that is `queues`. The keeper's inbox is a
  durable queue family (queues.md concern 12); the keeper is the consumer that
  drains it.
- **Not the curation policy, the keeper-to-keeper protocol, authority, topography
  structure, or environments** — all PARKED (see *Parked*). The keeper runtime
  leaves each a named, un-designed seam.

---

## Design notes (conceptual — internals at design-notes depth)

### 1. The triad: bundle + threads + workspace

A keeper node on the landscape carries:

- **A bundle** — its **persistent shared knowledge**, "curated like an artifact"
  (#161). The bundle is the **targeted-rollup product of the keeper node**
  (bundle = LOCKED vocab, #165/#166 Q14): for a keeper it projects to a **role
  prompt + zero-or-more plugins** (#148 beat 3 / #149 beat 11). It is assembled
  by `rollup` plane 2 pointed at the keeper's bundle node (rollup.md concern 12 —
  referenced, not redesigned). The role prompt "is basically the content of the
  seed" (#165); the historical word *seed* is superseded by *bundle* for this
  initialization knowledge (#172).
- **Threads** — how agents run against the keeper. A thread **starts two ways**
  (#149 beat 11 / #150 beat 15): **interactive** (a human talks to it) or
  **headless** (spawned from an incoming message). Each thread is itself a
  landscape node, linked to what it generated (loose-sense artifacts), and is
  **initialized from a specific VERSION of the bundle** (#146).
- **A workspace per thread** — the **per-thread ephemeral memory**, one-to-one
  with a thread (#149 beat 9 / #161). "Half ephemeral workspaces and half
  permanent [bundle]." The workspace **overlaps** the keeper's shared bundle
  (linked into it) and is **schematized** by the keeper type (below), with the
  opportunity to promote parts of a finished thread's workspace INTO the bundle
  (curation — PARKED, OQ-5). From the bundle you can navigate to all the
  keeper's workspaces in one hop (#150 beat 15).

### 2. The mini-harness shape (INTENT #130)

A keeper is "**more like a mini harness that can have multiple threads**" (#130):

- **One human thread** the user talks to directly (interactive), plus
- **specific message threads** that each handle one incoming message in a
  **dedicated, persistent** thread (headless).
- An **HTTP-like message protocol** delivers one message to a keeper; the
  handling thread does the back-and-forth, then **broadcasts to sibling threads
  in the same space** (e.g. the human thread) so they have awareness that "a
  headless coordinator" changed something.

The keeper is **idle unless woken** (#127): a message arrives on its inbox
(queues), a headless thread is spawned to handle it, and siblings are notified.
**The message + approval PROTOCOL itself is PARKED (OQ-6)** — the keeper runtime
leaves a `propose → approve → dispatch` placeholder loop; only the durability of
those messages is contracted (they ride durable queues, so they survive
compaction exactly-once — queues.md concern 12). Multiple-human-thread semantics
are PARKED (OQ-12).

### 3. Initialization = the bundle-rollup flow; "thread-modifies-future-init" = plain graph writes (INTENT #146)

The initialization node (formerly "the coordinator-initialization node") is the
keeper's **bundle node** on the landscape. A new thread is initialized by:

1. `rollup` performs **targeted rollup** on the keeper's bundle node at a pinned
   or latest `rollup_version` (rollup.md concern 12) → a **role prompt +
   plugin set**.
2. That bundle is **injected into a NEW thread** (`cc` spawns a Claude Code
   process with the assembled plugin dir + a fresh workspace + working dir).
3. The thread is told, verbatim intent (#148 beat 3): *"you were initialized by
   this node; if you want to change how future instantiations of yourself get
   initialized, this is the node you modify."*

**The "beautiful property" (#146, OQ-22) — each thread may modify how all future
threads initialize from that spot — is realized as PLAIN GRAPH WRITES.** "Falling
out of the KG is the elegant mechanism — it just modifies nodes" (#148 beat 3).
Concretely: a thread edits the bundle node in the landscape (a `kg-api` mutation)
and/or publishes a new `rollup_version` on it (rollup.md concern 12's version
mechanic); the next thread rolls up the new version. **No special machinery** —
it is a bundle-node write + a rollup version bump.

**WHEN/HOW that write happens (the curation trigger) is PARKED — OQ-5.** The
design-around (§C, OQ-5) is the boring provisional: **curation = a KG write on
the bundle node at thread end**, MANIFEST-linked not per-decision (#149 beat 8:
"you can't force them to write every decision… the real curation targets are its
purpose, its responsibilities"). Whether it is golden-rules or a dedicated
metacognitive pass on finished threads is undecided (#167 F-4) and the keeper
runtime does **not** decide it — it leaves the curation step a named seam.

### 4. Subagent spawning via task fragments (INTENT #150 beat 12)

A keeper thread dispatches work exactly as the operator specified:

> "Take these **task fragments** with this slot value and start a subagent; that
> assembly becomes a **NODE**; the node is passed into the subagent; the subagent
> executes and produces a **REPORT ARTIFACT** — a markdown node with frontmatter
> (summary, at-a-glance understanding) plus full detail." (#150 beat 12)

The pipeline, mapped to lower layers:

- **task fragments + slot values → assembly node.** `rollup` composes the
  subagent's prompt from fragments with slots (rollup plane 1 / plane 2); the
  **assembly is recorded as a landscape node** (`kg` write) so it is inspectable
  and provenance-traced.
- **assembly node → subagent.** `cc` spawns the subagent (a Claude Code process)
  with the assembled plugin + the assembly node as input.
- **subagent → report artifact.** The subagent produces a **report artifact** =
  a markdown node with frontmatter, an `artifacts`-crate typed artifact on the
  landscape (artifacts.md), linked to the spawning thread node. The keeper thread
  ("still called coordinator sometimes") **routes the outputs onward**.

Keepers are **"conceptual engines"** (#150 beat 12): they work **primarily in the
KG**, coordinating subagents and workflow patterns through the landscape graph,
and **link to VFS only when code must be written** (a thread's code work lands as
`vfs://` FileRefs on artifact nodes — kg concern 5). The keeper touches VFS
transitively (through kg pointers and cc working dirs), not as a first-order edge.

### 5. Keeper-type schemas (INTENT #149 beat 11 / #150 beat 14/15)

"Every seat/keeper TYPE has a schema defining its shared workspace [bundle] + its
connection to the ephemeral workspace: a **CORE keeper schema** with per-type
**INHERITED** schemas" (#150 beat 15). This rides `schema`'s inheritance engine
verbatim (schema.md concern 8 already reserves this exact seam):

- `keeper.core@N` is a root schema; `keeper.project`, `keeper.app`,
  `keeper.component`, `keeper.marketing`, … inherit from it. Each keeper-type
  schema defines that type's **bundle structure** (persistent shared knowledge)
  and its **workspace structure** (per-thread ephemeral), both **versioned** so
  they "play perfectly with rollup" (#149 beat 11 — rollup reads the
  schema-defined shape and composes text into it: two engines, one artifact).
- **Multiple / dual inheritance** (#150 beat 14): "merge two branches of child
  keepers into a new child keeper" is schema's multiple inheritance with
  **explicit manual conflict resolution** (LOCKED, #166 Q16). No keeper-side
  mechanism — schema owns it.
- **Per-node-TYPE workspace directory structures** (#133) are these schemas'
  workspace half — "like a workspace but created specifically for that node type,
  tracked within the schema service."

**Seam ownership:** `keeper` owns *what those structures CONTAIN* and the runtime
that instantiates them; `schema` owns *that they ARE schemas* (inheritance,
versions, codegen). The CareerCrafter organization types (OQ-9 — leadership,
logistical, strategic seats beyond technical) become **additional inherited
keeper-type schemas** when that round lands — no keeper change, just more
published definitions.

### 6. The keeper on the landscape — org is emergent (INTENT #132)

Keepers are landscape nodes linked by **typed edges** — `reports-to`,
`delegates-to`, `may-create-sub-keeper` (#132). The org "is probably just a
knowledge graph linking a bunch of keepers." Keepers arrange **hierarchically**
over the topology of projects / sub-projects / components (#127) and the edges
"naturally create the pathways for sending messages between them" (#149 beat 11 —
an edge is the route a keeper-to-keeper message travels). **Who ASSERTS an edge
and when, how the DAG stays "linearly separable," and what it means to MERGE two
keepers / their bundles — all PARKED (OQ-4 topography).** The keeper design
records these as OPEN and models `reports-to`/`delegates-to`/`may-create-sub` as
**candidate** landscape-template edge types, not a resolved topology.

### 7. Dogfooding — the OS's own keeper (INTENT #169)

The OS gets its own keeper on the landscape; the dashboard is a node inside it
that owns its render types (#150 beat 16 / #169). The landscape is **self-seeded
deterministically** at first boot — "don't just write it into the code and hope
it doesn't drift" — the same self-seed pattern rollup uses for `rollup@1` and
projects for `project-tree@1`. The keeper landscape template (below) is published
as DATA, not baked into code, so installers can extend it.

---

## Anticipated contracts (wave 3, L6)

The deliverable. Each lower-layer contract the keeper runtime needs, with its
**purpose** + **rough shape**. The keeper is a **consumer/composer** — it authors
one landscape-template artifact (`landscape@1`, keeper node/edge types) and
otherwise consumes existing lower-layer surfaces **verbatim**. No new wire
protocol is invented; nothing here is a schema. I do NOT edit `scaffold/contracts/*`.

- **`keeper-kg`** (keeper → kg; keeper authors the consumer side + the
  `landscape@1` keeper template). **Purpose:** the keeper IS a landscape node —
  this edge is how keeper nodes, bundle nodes, thread nodes, and the typed
  org-edges get read/written, and how the "thread-modifies-future-init" property
  is realized (plain node writes, §3). **Rough shape:** ordinary `kg-api`
  (kg.md — `Mutate` with push-back, `GetNode`, bounded `Traverse`, conflict
  list) plus a keeper-authored **`landscape@1` template** published via
  `PublishTemplate`, self-seeded deterministically (#169). Node types (candidate,
  OQ-4/OQ-19 OPEN): `keeper` (props: type-ref to a `keeper.<type>` schema,
  region, bundle-node ref), `bundle` (the initialization node — carries
  `rollup_version`, role-prompt + plugin membership as rollup-graph edges),
  `thread` (props: `kind: interactive|headless`, `bundle_version` it init'd from,
  `cc_thread_id`, status), `assembly` (§4, the subagent prompt assembly). Edge
  types (candidate): `holds-bundle` (keeper→bundle), `has-thread`
  (keeper→thread), `reports-to` / `delegates-to` / `may-create-sub-keeper`
  (keeper→keeper, §6), `generated` (thread→artifact/node, #146 loose-sense
  artifacts), `spawned` (thread→assembly→thread for subagents). kg adds **no
  keeper-specific surface** — `landscape@1` is template DATA, exactly as
  `projects` supplies `project-tree@1` and `rollup` supplies `rollup@1`.
  Distributed-everywhere (kg WAVE-3): keeper reads serve locally from every node.
  → to be authored `scaffold/contracts/keeper-kg.md`.

- **`keeper-rollup`** (keeper → rollup; consumer — bundle assembly). **Purpose:**
  assemble a keeper's **bundle** (role prompt + plugins) for injection into a new
  thread; assemble a subagent's prompt from task fragments (§4). **Rough shape:**
  rollup.md concern 12 is the mechanism, **referenced not redesigned** —
  targeted rollup = `rollup` plane 2 pointed at the keeper's bundle node via
  `ManifestSource::Node(GraphNodeRef { graph_id, node_id, at: VersionSpec })`.
  The keeper passes the bundle node ref + runtime slots + a `ScopeChain` derived
  from `rolls-up-from` ancestry; rollup returns the assembled plugin +
  `RollupProvenance` (with `graph_nodes_used` pinning the exact bundle version).
  Bundle **versioning** (the beautiful property, §3) is rollup's existing
  `VersionSpec`-over-`rollup_version` pinning — `Rollup(bundle, at: Pinned(v))`
  reproduces a prior thread's init; `at: Latest` takes the current. **Secret
  invariant holds unchanged** (rollup never resolves a raw secret into a bundle).
  In practice the keeper reaches this surface **through `keeper-cc`** (cc is
  rollup's primary consumer via `rollup-cc`) for thread-spawn plugin assembly;
  a direct keeper→rollup call is the shape for non-spawn bundle inspection. →
  anticipated `scaffold/contracts/keeper-rollup.md` (may fold into `rollup-cc`
  usage — flagged, open question 3).

- **`keeper-queues`** (keeper → queues; consumer — the durable inbox).
  **Purpose:** the keeper's HTTP-like message intake (§2) and the durable
  substrate for keeper-to-keeper proposals/approvals that must survive compaction
  exactly-once (§2, OQ-6). **Rough shape:** queues.md concern 12 IS this seam,
  **already reserved** — a **durable per-keeper inbox queue** (provisional name
  `keeper.<keeper_id>.inbox`), one per keeper, carrying typed keeper-message
  events; delivery at-least-once with `SemaphoreChoice::EventId` so a re-delivered
  approval applies exactly once (the event_id is the idempotency key). The keeper
  runtime is the **consumer**: it registers the triggers/handlers that drain its
  inbox and drive the `propose → approve → dispatch` loop (a handler spawns a
  headless thread via `keeper-cc`, then broadcasts to siblings). This is an
  ordinary `queues-api` use — **no new wire**. **The PROTOCOL is PARKED (OQ-6):**
  the message payload types (`types::keeper` or a keeper-local module) and the
  propose/deliberate/counter/approve state machine are a named-but-unspecified
  seam; queues guarantees only that the messages do not vanish. →
  `scaffold/contracts/queues-api.md` (documented consumer relationship; no new file).

- **`keeper-schema`** (keeper → schema; consumer — keeper-type schemas).
  **Purpose:** publish and resolve the CORE keeper schema + per-type inherited
  schemas that define each keeper type's bundle + workspace structure (§5).
  **Rough shape:** schema.md concern 8 IS this seam, **already reserved** — the
  keeper reaches schema over its **WS query surface** (resolve a
  `keeper.<type>@N` `SchemaDefinition`, publish `version+1` to evolve a type) and
  compiles against the **generated landscape-core library** (the DC surface). The
  keeper owns the CONTENT of `keeper.core` and each `keeper.<type>` (what the
  bundle/workspace structures contain); schema owns inheritance, versions,
  multiple-inheritance conflict resolution (#166 Q16), and codegen. CareerCrafter
  org types (OQ-9) slot in as more published definitions, no contract change. →
  anticipated `scaffold/contracts/keeper-schema.md` (or consumed via schema's
  cross-cutting WS surface — flagged).

- **`keeper-cc`** (keeper → cc; consumer — thread execution + subagent spawn).
  **Purpose:** run a keeper's threads (interactive + headless) and its subagents
  as Claude Code processes; this is how a bundle becomes a running agent.
  **Rough shape:** cc's `agent-management` verbatim (cc.md — spawn/track/signal/
  stream/reap by **stable handle**, every spawn admission-gated with a
  `BudgetGrant`). A keeper spawns a thread by handing cc: the **assembled bundle
  plugin** (via `rollup-cc` — cc is rollup's primary consumer, so the bundle
  assembly of `keeper-rollup` happens here in one step), a **fresh workspace**
  (the ephemeral per-thread memory, schematized by the keeper type), a **working
  dir**, and a **priority** (cc's strategy engine honors it — the `org-on-cc`
  "caller passes a priority" path, now the keeper). The **thread↔keeper(+project/
  environment) linkage** lands in cc's `agent_runs` ledger (the `cc-projects`
  fields; the keeper is the resolution authority for its thread nodes).
  **Subagent spawn (§4)** is the same surface: the assembly node's prompt +
  plugin → a cc agent → a report artifact. cc meters usage; the keeper never
  touches process supervision. **Interactive vs headless** is a keeper-side
  distinction (who/what woke the thread), not a cc distinction — cc spawns a
  Claude Code process either way. → anticipated `scaffold/contracts/keeper-cc.md`
  (extends the `agent-management` + `rollup-cc` consumer surfaces; `org-on-cc`'s
  shaped-for-a-broad-consumer record is the precedent — the consumer is now the
  keeper).

- **`keeper-vdb`** (keeper → vdb; consumer — keeper-local structured state;
  OPTIONAL / candidate). **Purpose:** any **keeper-local structured state** that
  is genuinely relational rather than graph-shaped (operational counters, a
  thread-scheduling worklist, session-style bookkeeping) — beyond what the KG
  bundle/thread nodes hold. **Rough shape:** a `vdb`-managed database keyed to the
  keeper (vdb.md databases-as-services), routed per environment (local→SQLite-in-
  VFS; cloud→promoted) exactly as kg graphs are. **Flagged thin/optional:** the
  keeper is a *conceptual engine* working primarily in the KG (§4), so most
  keeper state is landscape nodes, not a VDB database. This edge exists as a named
  seam for the case that structured operational state is needed; it is NOT
  assumed. → anticipated `scaffold/contracts/keeper-vdb.md` (candidate — may prove
  unnecessary; open question 4).

**Cross-cutting (consumed, not authored):** `service-lookup` (if the keeper
runtime ever registers — OPEN, depends on the crate-vs-library question),
`restart-protocol`, `pubsub-protocol` (sibling-broadcast, §2, could ride pubsub
or the inbox queues — flagged), `surface-schema` (the landscape/keeper map feeds
the dashboard, #150 beat 16), `locks-api` (single-writer on a bundle node during
curation, if needed). **Party to (authored elsewhere):** `spend` anchors
per-region spend to **landscape/keeper nodes** (#166 Q10) — spend's edge, the
keeper is the grouping authority its `node_id`s resolve against, not a
keeper-authored contract.

---

## Relationships / edges (summary)

Consumes: **kg** (`keeper-kg`, the landscape — keeper/bundle/thread nodes + typed
org-edges; keeper authors `landscape@1`), **rollup** (`keeper-rollup`, bundle +
subagent-prompt assembly — referenced, not redesigned), **queues**
(`keeper-queues`, the durable per-keeper inbox), **schema** (`keeper-schema`,
keeper-type schemas), **cc** (`keeper-cc`, thread + subagent execution), **vdb**
(`keeper-vdb`, optional keeper-local structured state). Party to: **spend**
(per-keeper-node spend rollup — spend's edge), **artifacts** (a thread's report
artifacts + generated nodes are `artifacts`-crate typed artifacts on the same
landscape), **projects** (a `project`-type keeper IS the primary node; the
`.mind` workspace migration lands on the same triad — projects.md §5b's
coordinator concept resolves into the keeper). Replaces: **org** (dissolved,
#132 — org is the emergent keeper graph).

## Nesting

Parent: none. **No crate this pass (OQ-17).** If a real design pass later builds
the keeper runtime, the natural decomposition (flagged, not built): a `landscape`
lib (the `keeper-kg` `landscape@1` template + node/edge reads/writes), a `bundle`
lib (targeted-rollup invocation + version pinning over `keeper-rollup`), a
`threads` lib (spawn/track interactive+headless over `keeper-cc`, the
mini-harness sibling-broadcast), an `inbox` lib (the `keeper-queues` drain loop +
the PARKED propose→approve→dispatch placeholder), and a `types` lib
(`keeper.core` + per-type schemas over `keeper-schema`). Whether these are libs
inside `cc`/`chassis`, a standalone crate, or a thin composition face is the
OQ-17 follow-on — OPEN.

## Open questions

1. **Keeper runtime home (OQ-17 follow-on).** Capture-now is settled (#148 beat
   1 — no crate this pass). WHERE it eventually lives — a `keeper` crate, libraries
   hosted in `cc`, a face over kg+rollup+queues — is undecided. The contracts
   above are authored so any of the three works (all are lower-layer consumers).
2. **Topography (PARKED — OQ-4).** Who asserts a keeper edge and when; keeping the
   keeper DAG "linearly separable" (#127); what it means to MERGE two keepers /
   their bundles / their graphs (#127); the actual bottom-up structure + initial
   schemas per node type (#167 F-3). Recorded as OPEN; `landscape@1` node/edge
   types are **candidate**, not resolved. Ownable node types beyond
   app/project/component (OQ-19) are the operator's call.
3. **`keeper-rollup` vs `rollup-cc` folding.** Bundle assembly for a thread spawn
   naturally happens inside `keeper-cc` (cc is rollup's primary consumer). Is a
   direct `keeper-rollup` edge needed at all (for bundle inspection without a
   spawn), or does everything route through `keeper-cc`? Flagged; leaning
   keep-the-named-edge for inspection, fold the spawn path into cc.
4. **`keeper-vdb` necessity.** May prove unnecessary if all keeper state is
   landscape nodes (§4 — conceptual engine in the KG). Named as a candidate seam;
   not assumed.
5. **Sibling-broadcast transport (§2).** The mini-harness "broadcast to sibling
   threads in the same space" (#130) — does it ride `pubsub-protocol` (lossy
   fan-out) or the durable inbox (queues)? Flagged; the human thread's awareness
   of a headless change is arguably durable-worthy, but that touches OQ-6.

## Parked (stated as such — do NOT decide this wave, #173d)

- **OQ-5 — the bundle-curation mechanism.** Golden-rules vs a dedicated
  metacognitive pass on finished threads (#167 F-4). The keeper runtime leaves a
  **named curation seam**; the design-around provisional is "a KG write on the
  bundle node at thread end" (§C), mechanism-undesigned.
- **OQ-6 — the keeper-to-keeper propose/approve protocol.** The #88 negotiation
  state machine (propose → deliberate → counter → tweak → agree), #127's approval
  gate, the human-in-loop vs no-human variants (#149 beat 11), and the HTTP-like
  message protocol (#130/#149 beat 10) — "needs its own conversation." The keeper
  leaves a `propose → approve → dispatch` **placeholder loop**; only durability is
  contracted (durable queues, `keeper-queues`).
- **OQ-12 — multiple-human-thread semantics.** Multi-thread per keeper is granted
  ("why not," #146) but collides with one-coordinator-per-workspace (#123) and
  "don't casually open the workspace" (#133); who tells human #2 when their init
  delta loses to a sibling's (#142 conflict events) is undecided. The triad
  (§1) supports many threads mechanically; the *semantics* are PARKED.
- **OQ-1 — authority.** No authority dependency is threaded into any keeper
  contract. Consistency-requiring cross-keeper merges (OQ-4) would ride the
  merge-reconciler surface on `locks` (§C), not a keeper mechanism.
- **OQ-2 — environments.** A keeper's thread/bundle may attach to an environment
  (the `agent_runs.environment_id`, kg per-graph routing), but environment↔keeper
  cardinality and per-environment bundle differences are PARKED (OQ-16 too).
- **OQ-18 — compaction re-seed.** "Compact the thread and lose basically nothing"
  (#123): which bundle version a compacted thread re-seeds from (pin-original vs
  take-latest) and what a thread must drain to its node so compaction is lossless
  are OPEN. The keeper records that a thread persists its id + `bundle_version`
  on its thread node at spawn (§1) — the minimum that makes re-seed *possible* —
  and leaves the discipline undesigned.

## Thoroughness level

**conceptual design + concrete lower-layer contracts (L6, INTENT #173c).** The
keeper's IDENTITY (triad, mini-harness, org-seat, conceptual engine), the
initialization/bundle-rollup flow, the subagent-spawn pipeline, and the
keeper-type-schema seam are designed to **verbatim-intent depth**. The **six
anticipated lower-layer contracts** (`keeper-kg`, `keeper-rollup`,
`keeper-queues`, `keeper-schema`, `keeper-cc`, `keeper-vdb`) are designed
concretely — purpose + rough shape + which existing lower-layer surface realizes
each — which is the wave-3 deliverable. **Internals stay at design-notes depth**
(no crate, no frozen schemas, no wire): the keeper runtime loop, the curation
trigger, the keeper-to-keeper protocol, and the topography are PARKED/OPEN and
left as named seams. Every mechanism is a composition of already-designed lower
layers; the keeper invents no engine.

## Design record — where the material came from

Grounded in INTENT #127/#130/#132/#133/#146/#148/#149/#150/#161/#162/#165/#166/
#169/#172 (read in full via the wave-3 intent ledger and INTENT.md), and the
batch-1→5 designs read for the contract seams: `kg.md` (kg-api, `landscape@1`
precedent = `project-tree@1`/`rollup@1`, distributed-everywhere), `rollup.md`
(concern 12 targeted rollup / bundle / `rolls-up-from` ancestry / version-pinning
— the bundle mechanism, referenced not redesigned), `queues.md` (concern 12 the
per-keeper inbox seam), `schema.md` (concern 8 keeper-type schemas seam), `cc.md`
(`agent-management` thread execution, `rollup-cc`, `org-on-cc` priority path),
`vdb.md` (databases-as-services), `artifacts.md` (the other half of the
landscape, report artifacts), `projects.md` (the primary keeper type + the
`.mind` coordinator material resolving here), and `org.md` (the tombstone this
file replaces).
