# artifacts

**Status:** NEW layer-six placeholder (wave-2 batch-7 STUB TRACK, 2026-07-19).
**Nesting:** top-level (candidate — "might have to be its own standalone
tool"). Promoted out of the paragraph inside `components/projects.md` (its
future note) + `overview.md`'s future section into its own file, per the
wave2-plan batch-7 instruction and INTENT #51.

> **⚠ NOT IMPLEMENTING NOW.** This is a DESIGN-NOTES + ANTICIPATED-DATA-CONTRACTS
> stub, deliberately NOT an implementation-ready design. It captures operator
> intent faithfully and sketches the contract pairs the wave2-plan assigns
> `artifacts` (`projects-artifacts`, `artifacts-vfs`, `artifacts-kg`) at the
> purpose + rough-shape level ONLY — no full schemas, no fill-model, no
> non-obvious-test suite. Artifacts is "a really big thing... we might have to
> build that one out a little bit at a time" (INTENT #51); nothing here is
> frozen. The contracts firm up when `artifacts` leaves the stub track, AFTER
> its dependencies (`vfs`, `kg`, `vdb`, `execution-engine`, `projects`) are
> real.

## Charter (requirements, operator's words where quoted)

The end-state of the projects migration, in the operator's framing:

> "Once we make the migration to projects, I don't want to have files anymore —
> I want to have artifacts."

An **artifact = a typed, schema'd, INTERACTABLE file.** Where the flat `vfs`
holds opaque bytes and `kg` holds schema-locked graph structure, an artifact is
the layer where a stored thing carries *both* a validated schema *and* a
declared way to be acted upon — you don't just read it, you run it, evaluate it,
simulate against it. The load-bearing example (INTENT #51, verbatim-grade):

> a **high-frequency-trading strategy artifact**, evaluated against **asset
> artifacts** (with market data attached), **run via a versioned controller
> interface against a simulator.**

Read that example as the shape of the whole concept, not a one-off: an artifact
of one type (a strategy) is *executed by a versioned controller* against
artifacts of other types (assets), producing results — which are themselves
artifacts. This is why the operator immediately concludes artifacts **implies a
script execution engine** and why he flags it as its own boring, deterministic
tool:

> "That's a really big thing... we might have to build that one out a little bit
> at a time." / "[artifacts] might have to be its own standalone tool, made
> really boring and deterministic."

**Projects depends on artifacts** once it exists (`projects-artifacts`,
wave2-plan §3c) — artifacts is the "no more files" substance that the projects
graphical-file-system migration is migrating *toward*.

**Boundary — what artifacts does NOT own (even as a stub, these lines matter).**
It does not own **content bytes** — an artifact's body is a `vfs` file
(`vfs://…`), content-addressed and replicated by VFS. It does not own **the
graph** — an artifact's identity, type, and relationships (strategy →evaluated-
against→ asset) are `kg` nodes/edges; artifacts is a *consumer/composer* of KG,
not its own graph engine, exactly as `projects` is. It does not own **the
trigger/handler execution discipline** — running a controller reuses the ONE
shared `execution-engine` (INTENT #61/#62/#65) and its handler model (Deno/TS +
SQL, traced, idempotent, loop-bounded, sandboxed), rather than inventing a
second runtime. It does not own **project structure or environments** — it is a
node *type family* that `projects` arranges and `environments` may route. What
artifacts owns is the thin, boring middle: **the artifact TYPE system (schema +
content pointer + versioned controller/interface binding), the controller
invocation contract, and result-artifact provenance** — the "interactable"
half that neither VFS (bytes) nor KG (structure) nor execution-engine (handler
mechanics) owns on its own.

## Design notes — the artifact model (schema + content + controller versioning)

Sketch only; every struct name below is illustrative, NOT a frozen contract.

**An artifact has four parts** (this is the design intent captured, to be
refined at fill time):

1. **Type** — a named, versioned artifact type (`hft-strategy`, `asset`,
   `simulation-run`, …). The type is the schema-lock: it declares the artifact's
   property schema, which parts of it are VFS-resident content, and which
   controllers/interfaces it supports. Strong parallel to `kg`'s **versioned
   templates** (research / obsidian-md / mind-axiomatic) — an artifact type is
   plausibly *a KG template* (or a thin thing on top of one), so the schema-lock
   + push-back-on-validation-failure machinery is inherited, not rebuilt. OPEN
   whether artifact-type == kg-template or a distinct registry (see Open
   questions).

2. **Schema** — the boring, statically-validatable property subset (mirrors
   `kg`'s `PropSchema`: types, required, enum, pattern, min/max). A malformed
   artifact is rejected at write time with per-field push-back, never coerced —
   the same "surface, don't destroy" discipline the KG plane locks.

3. **Content** — the artifact's body/payload lives in `vfs` (`vfs://…`), an
   existence-validated pointer exactly like `kg`'s `FileRefField` (creation-time
   `Exists` check; sweep-time re-validation; broken pointer flags the node, never
   auto-deletes). Big attached data (an asset's market-data blob) is
   content-addressed VFS bytes; the artifact node holds the pointer + hash.

4. **Controller / interface (versioned)** — THE distinguishing part, and the
   reason artifacts is more than "a KG node that points at a file." A **controller**
   is the *versioned* interface through which the artifact is run/evaluated:
   `run`, `evaluate(against: [artifact_ref])`, `simulate(simulator, params)`.
   Controllers are **pinned by version** on the artifact (the operator's "run via
   a versioned controller interface" is explicit and load-bearing): the same
   strategy artifact evaluated by controller `v3` vs `v4` is a reproducible,
   provenance-distinct fact. A controller is realized as **execution-engine
   handler(s)** (Deno/TS or SQL), so "artifacts implies a script execution
   engine" resolves to *reuse the shared engine*, not build a new one — the
   determinism, sandboxing (no-net-by-default, use-without-seeing secrets),
   tracing, and loop-bounding all come for free, which is precisely the "boring
   and deterministic" property the operator asked for.

**Running an artifact** (the intent, mechanism deferred): a controller
invocation takes the subject artifact + referenced artifacts + a target (a
simulator, itself plausibly an artifact/service), runs the pinned controller
version as an execution-engine handler, and **produces result artifacts** whose
provenance records the exact controller version, input artifact versions, and
the full causal chain (INTENT #85 healthcare-grade provenance — the same
`ee_*` / `kg_provenance` lineage, inherited). Reproducibility = pinned type
version + pinned controller version + content-hash-pinned inputs.

## Relationships / edges (design intent — no contract content this pass)

- **kg** — an artifact's identity/type/schema/relationships are KG structure;
  "evaluated against" / "produced from" are KG edges. Artifacts likely rides
  KG's template + schema-lock + FileRef + provenance machinery rather than
  duplicating it. Consumer of `kg-api`. (`artifacts-kg`, sketched below.)
- **vfs** — artifact CONTENT (bodies, attached datasets, controller code
  modules, result payloads) are VFS files, existence-validated pointers.
  Consumer of `vfs-content`/`Exists`. (`artifacts-vfs`, sketched below.)
- **execution-engine** (internal-lib seam if artifacts is a host; OR consumed
  through a host — OPEN) — controllers ARE handlers; running an artifact is a
  traced, sandboxed, loop-bounded engine invocation. Deliberately NOT a new
  runtime. Note: `execution-engine` is an internal library, never a contract
  edge (locked rounds 4–5) — whether artifacts embeds it directly or invokes
  through `kg`/`vdb` is an Open question.
- **projects** — projects DEPENDS ON artifacts (the "no more files" migration
  target); projects arranges artifacts hierarchically in its graphical file
  system. (`projects-artifacts`, sketched below — owned/authored jointly with
  projects, batch 7.)
- **vdb** (indirect) — if controller runs / result sets need database-backed
  intermediate tables (the stack pattern), those are `vdb`-managed databases,
  reached through the standard stack, not a new artifacts-owned store. Noted,
  not designed.

## Anticipated contracts (wave 2, stub track)

Names + purpose + rough shape ONLY. No schemas frozen; the per-pair round
authors real content once artifacts leaves the stub track and its dependencies
exist. Following stub-track discipline, `artifacts` lists these by name and
sketches each — it does NOT edit `scaffold/contracts/*`.

### `projects-artifacts` (projects → artifacts) — the "no more files" edge

**Purpose.** Projects, the graphical file system, migrates from pointing at raw
VFS files to composing **artifacts**: a project's graph nodes reference typed,
interactable artifacts instead of opaque paths. This edge carries how projects
resolves, lists, and arranges artifacts within a project (and across nested
projects), and how a project publishes/pushes artifacts to its centralized
registry.

**Rough shape.** Consumer surface (projects calls artifacts): resolve an
artifact by id/ref within a project scope; list a project's artifacts by type;
attach/detach an artifact to a project node; (later) push an artifact to the
project registry. Almost certainly expressed AS `kg-api` reads/writes over a
shared artifact-type set rather than a bespoke wire surface — parallels
`projects-kg`, where projects' half is largely `kg-api` + a projects-defined
template. Authored jointly with `projects` (batch 7). No fields frozen.

### `artifacts-vfs` (artifacts → vfs) — content residence + existence validation

**Purpose.** Artifact bodies, attached datasets, controller code modules, and
result payloads live in `vfs`; this edge is the existence-validated pointer +
body-read surface. Directly mirrors `kg-vfs` (pointers only — the graph
database FILES reach VFS through VDB, not here; likewise artifact CONTENT vs any
artifacts-owned database files).

**Rough shape.** Consumes vfs's kg-facing surface unchanged: `Exists { path } ->
{ present, content_hash, size }` at artifact-write time (absent → push-back,
unless a dangling-allowed type); a sweep re-validation cadence (broken pointer
FLAGS the artifact, never deletes it — the KG non-destructive rule reused);
`Read`/body-fetch for controller execution; content-address on write for
immutable payloads (datasets, results). LOW version-sensitivity — small additive
request/reply, `Hash` frozen by vfs's contract. No new VFS surface needed
(reuses `vfs-content` + `Exists`); flagged so vfs's designer need not add
anything for artifacts.

### `artifacts-kg` (artifacts → kg) — identity, type/schema, relationships, provenance

**Purpose.** An artifact's identity, its versioned type/schema, its
relationships to other artifacts (`evaluated-against`, `produced-from`,
`controlled-by@version`), and its provenance are KG structure. This edge is how
artifacts creates/reads/mutates the graph elements that ARE artifacts, with
schema-lock push-back and healthcare-grade lineage inherited.

**Rough shape.** Consumer of `kg-api` (`CreateGraph`/`CreateNode`/`CreateEdge`/
`Mutate` with push-back, bounded `Scan`/`Neighbors`/`Traverse`, `Audit`).
Artifact TYPES are strong candidates to be **KG templates** (versioned node/edge
schema sets) — an `artifact-types@1` template family (or per-domain families
like `trading@1`) defined the way `projects` will define `project-tree@1`. If
so, artifacts adds NO new kg-side wire surface (its half is `kg-api` verbatim +
templates it publishes), and the controller-version binding rides node props +
a `controlled-by` edge to a controller/handler artifact. OPEN whether an
artifact-type registry needs anything KG doesn't already give (versioned
templates + FileRefs + provenance cover most of it). Authored against `kg-api`
when artifacts is built.

## Open questions (carried forward — NOT decided this pass)

1. **Standalone tool vs. a face on kg+vdb.** The operator flags artifacts
   "might have to be its own standalone tool." Is `artifacts` a top-level
   app-crate with its own daemon, or is it a thin library/convention layered on
   `kg` (types/schema/pointers) + `execution-engine` (controllers) + `vfs`
   (content), with `projects` as the arranger? Leaning: mostly composition over
   existing planes, with a thin artifacts surface for the controller-invocation
   + result-artifact lifecycle that has no natural home elsewhere. Not decided.

2. **Artifact-type == KG template, or a distinct type registry?** KG's versioned
   templates already give schema-locked, versioned node/edge type sets with
   push-back and FileRefs. Does the artifact type system reuse that wholesale, or
   does the *controller/interface binding* (the one thing KG templates don't
   model) justify a distinct artifact-type registry on top? Sketched above as
   "probably KG templates + a controller-binding extension"; needs a real design
   pass.

3. **Controllers vs. handlers — same engine or a sibling?** The operator's
   "script execution engine" for artifacts and the LOCKED `execution-engine`
   (INTENT #61/#62/#65) for db/kg triggers are almost certainly the same engine
   reused. A controller is a handler invoked *on demand by an artifact
   operation* rather than *reactively by a change-trigger* — the engine supports
   both, but artifacts introduces an **imperative/RPC invocation mode**
   (run-this-now) alongside the engine's current reactive/outbox mode. Confirm
   the engine's invocation surface can host on-demand controller calls, or
   whether artifacts needs a sibling invocation path. (Flag for the
   execution-engine designer.)

4. **The simulator.** In the HFT example the strategy runs "against a
   simulator." Is a simulator itself an artifact (a controller with no subject,
   or an artifact type whose controller consumes strategy+asset artifacts)? Is
   it a service? Deferred — but the answer shapes whether "evaluate against X"
   generalizes cleanly.

5. **Result artifacts + reproducibility.** Running a controller produces
   results; treating results as first-class artifacts (versioned, provenance-
   linked to exact input + controller versions) gives reproducibility for free.
   Confirm the result-artifact lifecycle and how much of it is just KG
   provenance vs. a new lifecycle state machine.

6. **Migration semantics.** "No more files" is aspirational — how do existing
   raw `vfs` files / `projects` file pointers migrate INTO typed artifacts?
   Incremental (artifacts coexist with untyped files during transition) is the
   only realistic path given "built out a little bit at a time"; the migration
   contract is future work.

7. **Determinism guarantees.** The operator wants artifacts "really boring and
   deterministic." Reusing `execution-engine` (sandboxed, no-net-by-default,
   pinned-vendored modules) gets most of it, but market-data-driven evaluation
   (the HFT case) is deterministic only relative to pinned input artifacts —
   confirm content-hash-pinning of all inputs is the determinism boundary.

## Nesting

Parent: none (candidate top-level; OPEN per Open-question 1) | Children: none
(this pass).

## Thoroughness level

**requirements-only** — operator-intent capture + anticipated-contract sketches
(purpose + rough shape) + open questions. Explicitly NOT a design pass:
no frozen schemas, no fill-model, no non-obvious-test suite. Layer-six stub
track per INTENT #51 and wave2-plan batch 7; "not implementing now."
