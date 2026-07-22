# artifacts — the other half of the landscape

**Status:** L6 stub, REWRITTEN at the re-spoken round (2026-07-21/22,
INTENT #165 — verbatim-grade; supersedes the emphasis of the friction-round-3
reframe INTENT #136, which itself superseded the wave-2 files-with-runners
sketch; both are retained below as the design record). **Track:** STUB —
"not implementing now" stands. **Nesting:** open (standalone crate vs a
face on kg+schema+vfs — see open questions).

> **⚠ NOT IMPLEMENTING NOW.** Requirements-only: operator-intent capture +
> anticipated shapes. No frozen schemas, no fill-model. The dependencies
> (`kg`, `schema`, `vfs`, `rollup`, the topological-node concept) must be
> real first.

## Charter (re-spoken round, INTENT #165 — verbatim-grade)

**The landscape holds two kinds of things: topological nodes AND
artifacts.** Artifacts are the other half of the landscape — the things the
topological nodes' agents work ON, arranged in the same one open-world
graph (see overview.md's canonical vocabulary block).

An **artifact node** carries:

- **pointers to VFS** (its content/files),
- **pointers to knowledge graphs** (its structure/relations),
- **methods and tools** (what agents can DO with it), and
- **a whole rollup system** (how it presents itself).

**Artifact TYPES are schemas — directly inspectable and editable.** The
operator's analogy: "like looking at the class of a Python object." The
type is not a sealed registry entry; it is an artifact-like, editable thing
itself — **so agents can improve a type's skills/methods over time.** An
agent that learns a better way to handle experiment artifacts edits the
experiment TYPE, and every instance benefits. Content is type-specific.

**Bundle-appears-when-in-sight.** When an artifact comes into sight of an
agent, its **BUNDLE** appears: the skills/tools for that artifact's type
enter the agent's visible surface, and leave when the artifact leaves
scope. (Bundle is LOCKED vocabulary — the targeted-rollup product of ANY
landscape node; for topological nodes it is role prompt + plugins, for
artifact nodes it is the type's skills/tools. INTENT #165/#166.)

**Everything in the landscape gets TARGETED ROLLUP** — topological nodes
and artifact nodes both. There is one rollup discipline for the whole
landscape; artifacts do not have a bespoke presentation mechanism.

Examples of artifact nodes (from the operator across rounds): a program; an
execution with script + logs; an experiment with output data; a
high-frequency-trading strategy evaluated against asset artifacts.

## What this rewrite settles vs leaves open

Settled by INTENT #165 (this round):

1. **Artifacts are first-order landscape citizens**, not a maybe-reducible
   KG convention. The friction-round-3 standing question ("is there
   anything useful in artifacts we can't get from the knowledge graph
   directly?") is answered in practice: the type-as-editable-schema +
   methods/tools + in-sight bundling + targeted rollup IS the added value.
2. **Types are schemas** — so the type system rides the `schema` crate
   (versions, inheritance incl. multiple inheritance with
   explicit-manual-conflict-resolution — schema.md), not a bespoke
   artifact-type registry.
3. **The presentation mechanism is the universal one** (targeted rollup /
   bundles), shared with topological nodes.

Still open (carried forward):

- **Standalone crate vs composition.** A thin artifacts surface over
  kg (structure) + schema (types) + vfs (content) + rollup (bundles), or
  its own daemon? Unchanged from before; leaning composition.
- **Methods/tools mechanics.** How a type's methods bind to executable
  handlers (the execution-engine reuse argument from the wave-2 sketch
  below still looks right: controllers/methods as engine handlers —
  sandboxed, traced, deterministic).
- **Type-editing governance.** Agents improving a type's skills/methods
  over time needs versioning + review discipline (schema versions give the
  mechanism; the policy is undesigned).
- **Migration.** How raw VFS files / project file pointers become typed
  artifacts — incremental coexistence, as before.

## Relationships / edges (updated framing)

- **the landscape / kg** — artifact identity, relations, and placement are
  landscape structure (KG-resident). The `artifacts-kg` sketch below still
  describes the mechanics.
- **schema** — artifact types ARE schemas: versioned, inheritable
  (multiple inheritance with explicit manual conflict resolution — LOCKED,
  INTENT #166 Q16), directly inspectable/editable. This replaces the
  "artifact-type == kg-template?" open question's bespoke-registry branch.
- **vfs** — content pointers, existence-validated (`artifacts-vfs` sketch
  below unchanged in substance).
- **rollup** — the bundle producer; artifacts consume the same targeted-
  rollup machinery as every landscape node.
- **topological nodes (bishop/keeper concept)** — the agents that interact
  with artifacts spin up from topological nodes; when an artifact enters a
  thread's sight, its bundle joins that thread's surface. (NOT the `agents`
  crate — that is the custom-agent-harness placeholder, INTENT #167.)
- **projects** — projects arrange artifacts; "no more files — artifacts"
  remains the migration direction.

---

## Design record — superseded framings (retained for lineage)

### Friction-round 3 reframe (INTENT #136, 2026-07-20) — objects with attributes AND METHODS

Superseded-in-emphasis by INTENT #165 above (which absorbs and extends it);
its content was: artifacts look like "schematized nodes in some graph"; the
distinguishing value over raw KG is AGENT INTERACTION — "tools attached to
those items, the tools only visible when the artifact comes into scope; an
artifact can be an object with attributes and methods that agents can
interact with." It posed the standing is-this-just-KG question (now
answered above) and suggested the `agents` service as the consumer (now
corrected: the topological-node concept is the consumer; `agents` is the
custom-harness placeholder — INTENT #167).

### Wave-2 sketch (batch 7, 2026-07-19) — the four-part artifact model

The original files-with-runners design, still useful as mechanics for the
methods/tools half:

1. **Type** — named, versioned (now: a `schema`-managed schema, per above).
2. **Schema** — statically-validatable property subset; malformed writes
   rejected with per-field push-back, never coerced.
3. **Content** — body/payload in `vfs` (`vfs://…`), existence-validated
   pointers (creation-time `Exists`, sweep re-validation, broken pointer
   flags-not-deletes).
4. **Controller / interface (versioned)** — the run/evaluate/simulate
   surface, pinned by version; realized as **execution-engine handlers**
   (sandboxed, no-net-by-default, traced, loop-bounded) — reuse the ONE
   shared engine, never a second runtime. Result artifacts carry
   provenance: exact controller version + content-hash-pinned inputs =
   reproducibility. Load-bearing example: an HFT strategy artifact
   evaluated against asset artifacts via a versioned controller against a
   simulator.

Anticipated contract sketches (`projects-artifacts`, `artifacts-vfs`,
`artifacts-kg`) from that pass survive in substance — resolve/list/attach
within project scope; existence-validated content pointers; kg-api
consumption with schema-locked push-back — with the type-registry halves
re-read against "types are `schema` schemas." Full text in git history
(`git log -- scaffold/components/artifacts.md`).
