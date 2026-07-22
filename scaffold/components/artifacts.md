# artifacts — the other half of the landscape

**Status:** L6 CONCEPTUAL DESIGN NOTE (wave 3, batch 6, unit
`projects-artifacts-landscape`), 2026-07-22 — consolidates the re-spoken-round
rewrite (INTENT #165) and the friction-round-3 reframe (INTENT #136) to
**conceptual-design depth (INTENT #173c)**: enough to know the data contracts
with the lower layers, not implementation. **Track:** STUB — "not implementing
now" stands. **Nesting:** none — open whether artifacts is ever a standalone
crate or stays a composition of kg+schema+vfs+rollup+execution-engine (see
Open questions); no code exists either way this wave.

> **NOT IMPLEMENTING NOW.** Requirements-and-contracts only: operator-intent
> capture + anticipated lower-layer shapes. No frozen schemas, no fill-model.
> The dependencies (`kg`, `schema`, `vfs`, `rollup`, `execution-engine`, the
> **keeper** concept) must be real first.

## Charter

**The landscape holds two kinds of things: keepers AND artifacts** (INTENT
#165, overview.md's canonical vocabulary block). Artifacts are the other half
of the landscape — the things keepers' agents work ON, arranged in the same
one open-world graph. An **artifact node** carries:

- **pointers to VFS** (its content/files),
- **pointers to the knowledge graph** (its structure/relations),
- **methods and tools** (what agents can DO with it), and
- **a whole rollup system** (how it presents itself — its bundle).

**Artifact TYPES are schemas — directly inspectable and editable.** The
operator's analogy: "like looking at the class of a Python object." A type is
not a sealed registry entry; it is itself an editable, versioned thing — **so
agents can improve a type's skills/methods over time.** An agent that learns a
better way to handle experiment artifacts edits the experiment TYPE, and every
instance benefits. Content is type-specific.

**Bundle-appears-when-in-sight.** When an artifact comes into an agent's
sight, its **bundle** appears: the skills/tools for that artifact's type enter
the agent's visible surface, and leave when the artifact leaves scope.
(**bundle** is LOCKED vocabulary — the targeted-rollup product of ANY
landscape node; for keepers it is role prompt + plugins, for artifact nodes it
is the type's skills/tools — INTENT #165/#166, rollup.md concern 12.)

**Everything in the landscape gets targeted rollup** — keepers and artifact
nodes both, ONE rollup discipline, no bespoke per-node presentation mechanism
(rollup.md concern 12).

Examples of artifact nodes (operator, across rounds): a program; an execution
with script + logs; an experiment with output data; a high-frequency-trading
strategy evaluated against asset artifacts via a versioned controller.

**Objects with attributes AND methods (INTENT #136, absorbed).** The
distinguishing value over a raw KG node is agent interaction: "tools attached
to those items, the tools only visible when the artifact comes into scope; an
artifact can be an object with attributes and methods that agents can
interact with." This is the value proposition targeted rollup + schema-typed
methods together deliver (below), not a separate mechanism.

**The consumer — corrected across rounds.** #136 first floated "the agents
service" as artifacts' consumer. **Corrected (INTENT #167): NOT the `agents`
crate** — `agents` stays a conceptual placeholder for CUSTOM agent harnesses
(OpenRouter + inference contracts) that go direct to inference, not through
cc. The actual consumer is the **keeper concept**: the agents that interact
with artifacts spin up from keeper nodes; when an artifact enters a thread's
sight, its bundle joins that thread's surface (rollup.md concern 12; keeper
runtime, keeper.md, batch 6).

## What settles vs what stays open

**Settled (INTENT #165, this round):**

1. **Artifacts are first-order landscape citizens**, not a maybe-reducible KG
   convention. The standing question ("is there anything useful in artifacts
   we can't get from the knowledge graph directly?") is answered in
   practice: type-as-editable-schema + methods/tools + in-sight bundling +
   targeted rollup **is** the added value over a bare KG node — a bare node
   has structure; an artifact node additionally has a **runnable, versioned,
   agent-improvable capability surface**. The question is recorded, not
   re-litigated, should a future round want to revisit it (see Open
   questions).
2. **Types are schemas** — the type system rides the `schema` crate
   (versions, inheritance including multiple inheritance with
   explicit-manual-conflict-resolution, LOCKED — schema.md concerns 1/2/8),
   not a bespoke artifact-type registry.
3. **The presentation mechanism is the universal one** (targeted rollup /
   bundles), shared with keepers — rollup.md concern 12, not designed here.

**Still open (carried forward, none decided in this pass):**

- **Standalone crate vs composition.** A thin artifacts surface over
  kg (structure) + schema (types) + vfs (content) + execution-engine
  (methods) + rollup (bundles), or its own daemon? Leaning composition — no
  artifact-owned storage or execution mechanism duplicates a lower layer's
  job (mirrors schema's two-engines discipline: artifacts adds no third
  execution/storage engine, it composes the ones that exist).
- **Methods/tools mechanics — the imperative-invocation question, flagged,
  NOT decided.** Execution-engine's paradigm (kg.md/vdb.md's shared lib) is
  built for the database-centric **trigger/handler** model: async,
  at-least-once, fired off a captured change or a queue/schedule occurrence
  (execution-engine.md §§1–3). An agent invoking an artifact's method reads,
  in the operator's language, as a **synchronous call** ("run this
  controller against this simulator"). Whether that maps onto execution-
  engine's `Direct`/`ExecFn` dispatch path as-is (a queue-triggered handler
  invocation an agent's request enqueues, then awaits via mesh's
  promise-based messaging, INTENT #152) or needs a thin request/response
  wrapper on top is an **open design question for the real pass** — flagged,
  not resolved, so artifacts does not silently invent a second execution
  paradigm nor silently force-fit the async one without checking the fit.
- **Type-editing governance.** Agents improving a type's skills/methods over
  time needs versioning + review discipline; schema's version/migration
  engine gives the mechanism (schema.md concerns 1/7), the *policy*
  (who/when/how a type-improvement publishes) is undesigned — plausibly the
  same PARKED curation question (OQ-5) keeper bundles face, but not asserted
  as identical here.
- **Migration.** How raw VFS files / project file pointers become typed
  artifacts remains incremental coexistence — unchanged from the wave-2
  framing.

## Anticipated contracts (wave 3, L6)

Per INTENT #173c: purpose + rough shape with each lower layer, not schemas.
Every edge below is a **type/schema-level or node-level consumption**, not a
new artifacts-owned service — artifacts contributes data (schema
definitions, rollup fragments, kg node types) into the tools that already
exist, exactly as `keeper.project` does in `projects.md`.

- **`artifacts-kg`** (artifact identity, structure, relations — kg.md is the
  substrate). **Purpose:** an artifact IS a landscape node; its placement,
  its relations to other nodes (a `rolls-up-from` parent, `consumes`/
  `supersedes` edges to other artifacts, links to the keeper region it
  belongs to), and its discoverability are kg concerns. **Rough shape:**
  `kg-api` verbatim — no artifacts-owned graph engine. An artifact TYPE
  (below) declares its node/edge shape as a `schema` definition (not a
  kg-local template, mirroring `keeper.project`'s approach — schema.md
  concern 9's extraction seam); kg validates instances against it with
  push-back (INTENT #50), closed-world, same as every other kg-resident
  type. Node props carry the VFS FileRef(s) (content) and any KG pointer
  props (structure/relations) the type schema declares.

- **`artifacts-schema`** (artifact TYPES are schema definitions — schema.md
  concern 8 is the substrate, already reserving this seam). **Purpose:** the
  operator's "like looking at the class of a Python object" — a type
  (`artifact.skill`, `artifact.report`, `artifact.experiment`, `artifact.hft-
  strategy`, …) is a `SchemaDefinition`: its structure (VFS/KG pointer
  fields), its methods/tools binding (which `HandlerDef`s a method name
  resolves to — see `artifacts-execution-engine`), and its rollup config
  (which fragments compose its bundle). **Rough shape:** "improving a type's
  skills/methods over time" is publishing `schema` `version+1` on that
  `SchemaId` — schema's migration engine (concern 7) carries existing
  artifact instances forward, rejected-whole on nonconformance, exactly kg's
  posture. Multiple inheritance with explicit conflict resolution (schema.md
  concern 2, LOCKED) covers a type built by merging two prior types' method
  sets. Artifacts owns *what a type means*; schema owns *that it is a
  schema, versioned, inheritable* in this tool.

- **`artifacts-execution-engine`** (methods run as handlers — execution-
  engine.md is the substrate; internal-lib seam of its HOST, not a direct
  artifacts↔engine wire edge — execution-engine is never a contract party
  itself, locked rounds 4–5). **Purpose:** a method/tool invocation on an
  artifact is realized as an execution-engine handler run — sandboxed
  (no-net-by-default, per-handler egress allowlist), traced (provenance,
  `ee_touches`/`ee_invocations`), and deterministic (vendored modules,
  content-pinned). Reuses the ONE shared engine — no second runtime.
  **Rough shape (two candidate bindings, neither chosen — the imperative-
  invocation question above):** (a) **change-bound** — an artifact-type
  method fires off a kg node/edge change via the engine's `Graph` adapter
  (execution-engine.md §1), e.g. writing a `run_requested` prop triggers the
  handler; (b) **direct-dispatch** — an agent's request enqueues onto
  `queues` and the engine's `Direct`/`ExecFn` origin (execution-engine.md §3)
  invokes the bound `HandlerDef` (SQL or Deno/TS) with `subject_ref` resolving
  to the artifact's underlying graph/database via `kg-vdb`; the agent awaits
  the result through mesh's promise-based messaging (INTENT #152) rather than
  a synchronous call. Result artifacts carry provenance: exact controller
  version + content-hash-pinned inputs = reproducibility (execution-
  engine.md §7's lineage walk, unchanged). **Not decided:** which binding (or
  a third, request/response wrapper) is right — flagged for the real pass,
  per the open question above.

- **`artifacts-vfs`** (content — vfs.md / `vfs-content` is the substrate).
  **Purpose:** an artifact's body/payload (script, logs, output data,
  simulator inputs) lives in ordinary VFS storage; the artifact node holds
  existence-validated pointers, not bytes. **Rough shape:** identical to
  kg's own FileRef discipline (kg.md concern 5) — write-time `Exists` check,
  sweep-time re-validation, broken-pointer-flags-not-deletes, never silent
  loss. Bulk content transfer (a large simulator log, a dataset) rides
  `vfs-content`'s relayed/brokered stream-tunnel path (vfs.md, INTENT
  #153) rather than small payloads through mesh — the same size-based
  routing split every large-transfer case in the OS uses.

- **`projects/landscape ↔ keeper`** (the consumer relationship — keeper.md,
  batch 6, is the substrate; see also `projects.md`'s identical entry from
  the keeper-type side). **Purpose:** keepers are the "conceptual engines"
  that work on artifacts; artifacts do not initiate anything themselves. An
  artifact entering a thread's sight is what triggers its bundle to appear
  (targeted rollup, rollup.md concern 12) — the keeper runtime is the party
  that notices "in sight" and pulls the bundle in. **Rough shape:** no
  artifacts-owned notification mechanism — "in sight" is a keeper-runtime-side
  concept (which artifacts the current thread/workspace has referenced or
  traversed to), and the bundle fetch is an ordinary `Rollup(artifact_node) →
  bundle` call (rollup.md concern 12), same call shape a keeper uses for its
  own bundle. Not designed here — keeper.md owns the runtime side of "in
  sight."

## Relationships / edges (summary)

Consumes (type/schema/data level, not service edges): **kg** (identity,
structure, relations — `artifacts-kg`), **schema** (the type system —
`artifacts-schema`), **execution-engine** (methods, via its HOST's internal
lib — `artifacts-execution-engine`, not a direct wire edge), **vfs** (content
— `artifacts-vfs`), **rollup** (bundles — the same targeted-rollup mechanism
every landscape node uses, rollup.md concern 12). Consumed by: **keeper**
(the agents that work on artifacts — `projects/landscape ↔ keeper`), **cc**
(only insofar as a keeper's thread is a cc thread — no direct artifacts↔cc
edge). Explicitly NOT a party to: **`agents`** (the custom-harness
placeholder crate, INTENT #167 — corrected framing, see Charter).

## Nesting

None. Open whether artifacts ever becomes its own thin composition crate; no
code exists this wave either way (see Open questions, "standalone crate vs
composition").

## Open questions

1. **Standalone crate vs composition** — thin surface over kg+schema+vfs+
   execution-engine+rollup, or its own daemon. Leaning composition, not
   locked.
2. **The imperative-invocation question (methods/tools mechanics)** — does an
   artifact method map onto execution-engine's async trigger/handler paradigm
   as-is (queue-dispatch + mesh promise-await), or does it need a thin
   synchronous-feeling wrapper on top? Flagged, genuinely undecided.
3. **Type-editing governance** — versioning mechanism exists (schema), the
   policy for WHEN a type-improvement publishes does not; possibly the same
   territory as the PARKED bundle-curation question (OQ-5), not asserted
   identical.
4. **Migration** — incremental coexistence of raw VFS/project files and typed
   artifacts, mechanism unspecified.
5. **Is there anything artifacts give that a bare, well-schematized KG node
   could not?** Answered in practice (see "What settles") by the combination
   of type-as-editable-schema + methods/tools + in-sight bundling + targeted
   rollup — recorded here as the standing question in case a future round
   wants to press on it further, not re-opened by this file.

## Thoroughness level

**L6 conceptual (INTENT #173c)** — enough to know the data contracts with kg,
schema, execution-engine, vfs, rollup, and the keeper concept; no schemas
frozen, no fill-model, no implementation. Consolidates INTENT #165 (re-spoken
round, primary source) and INTENT #136 (friction-round-3, absorbed/corrected)
into one conceptual pass. Full prior design record (the four-part
type/schema/content/controller sketch, its detailed anticipated-contract
drafts) lives in git history (`git log -- scaffold/components/artifacts.md`).
