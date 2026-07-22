# schema

**Status:** wave-3 reshape (batch 4, data/execution plane) — **folds to
implementation-ready.** Supersedes the requirements-only stub that deferred all
design to "the dedicated KG-templating discussion round." That gate is now
LIFTED: the re-spoken rounds settled schema's core (nested + multiple
inheritance, INTENT #131/#150 beat 14; conflict resolution LOCKED #166 Q16;
default values #149 beat 4; the generated-library concept #150 beat 16; the
DC-emergent resolution #166 Q11; the two-engines guardrail #166 F-2). This pass
DESIGNS those. Two things stay OPEN by directive — the **types↔schema
authority-of-record** (OQ-20) and **schema-as-universal-messaging** (OQ-21, INTENT
#139): the codegen bridge is designed so *both* answers remain reachable without a
rewrite; the seams are stated, the answers are not taken.

> **RENAMED `onion` → `schema` (INTENT #131).** The operator: "instead of onion,
> something more understandable — we'll just call it schema. Our crate for
> managing schemas with versions and layering. One boring tool for schema
> management, used within all the other services." **Prior art:** the operator's
> EHR data-lake system — per-practice masks over an EHR template onto one central
> schema.

**Nesting:** top-level app-crate (`bin/schema` + `lib/schema`), L4 data plane.
**Grounding:** INTENT #131 (rename + nested-arbitrary-depth inheritance), #150
beat 14 (multiple/graphical inheritance), #166 Q16 (explicit-manual conflict
resolution, LOCKED), #149 beat 4 (default values in schemas), #145 + #150 beat 16
(schema builds Rust types → versioned struct **libraries**, imported in code +
queryable over WS), #151 + #166 Q11 (DC emergent from chassis + schema +
generated libraries — **no DC service**), #165 (artifact TYPES are schemas), #149
beat 11 / #150 beat 15 (keeper-type schemas define bundle + workspace structure),
#166 F-2 (two-engines rule; gravity-well blessed), #139 (OQ-21 messaging-on-schema
— recorded, NOT applied), #169 (self-seeded, not code-baked); ledger §A rows
59–62, §C OQ-20/OQ-21 design-around. **Reads:** the batch-1 `types.md` + `chassis.md`
(the generated libraries are what chassis's `Contract` traits are synthesized
over), `contracts/mesh-transport.md` (OQ-21 seam mirrored), and `kg.md` concern 3
(the template model schema extracts and generalizes).

## Charter

`schema` is the **one boring tool for schema management with versions and
layering, used within all the other services** (INTENT #131). It owns four things
and — by the two-engines rule — deliberately owns nothing else:

1. **The inheritance engine** — a schema inherits from one *or more* parent
   schemas, applying migrations, **to arbitrary depth** (a DAG, not a chain):
   INTENT #131/#150 beat 14.
2. **Conflict resolution at initialization** — when multiple parents disagree,
   the initializer states the resolution explicitly, or initialization fails. No
   quiet overwrites (LOCKED, INTENT #166 Q16).
3. **Codegen to versioned struct libraries** — schema *builds Rust types* (INTENT
   #145/#150 beat 16): each published schema-set emits a **versioned library
   crate**, importable in code, and the *same* schema data is **queryable over
   WS**. This is exactly what makes the **data-contract (DC) surface emergent**
   from chassis + schema + generated libraries with **no separate DC service**
   (INTENT #151/#166 Q11).
4. **Structure migration** — moving data/graphs/records from one schema version to
   the next (the "migrates STRUCTURE" engine of the two-engines rule).

**Applicable beyond graphs.** Not KG-specific: the same mechanism applies to
graph node/edge types, JSON-schema records, YAML config shapes — "the same
philosophy inside the crate, used to do templated behaviors and push changes to
multiple services at once" (INTENT #131). KG's template system (kg.md concern 3)
is schema's **first extraction consumer**, not its owner (concern 9).

**What schema does NOT own (the two-engines guardrail — INTENT #166 F-2).**
schema **migrates STRUCTURE; rollup composes TEXT** — "simple boring thing, many
surfaces," but neither engine grows the other's job. schema never composes a
prompt, never resolves a rollup fragment, never holds free-text content. Its ONE
sanctioned brush with content is a **default value** (concern 3) — a literal typed
constant that is part of a type definition, not composed text. schema also does
**not** own where its consumers *store* data (KG owns graph storage, VDB owns
databases, VFS owns files); it owns the *shape* and the *migration between shapes*,
never the bytes.

## Primary design concerns

### 1. The inheritance model — a DAG to arbitrary depth, multiple parents

The stub's open "chain vs DAG" is decided: **a DAG.** A `SchemaDefinition`
declares zero-or-more `parents`, each parent pinned to a version, and carries its
own **layer** — a set of additive migrations over the merged parents. "This
schema inherits from those schemas at those versions and applies these
migrations, to arbitrary depth" (INTENT #131, superseding the single
template+delta wording).

```rust
pub struct SchemaId(pub String);        // slug, dot-segmented: "keeper.core", "artifact.skill", "research.hypothesis"

pub struct SchemaDefinition {
    pub id: SchemaId,
    pub version: u32,                    // monotonic per id; IMMUTABLE once published (concern 6)
    pub content_hash: Hash,             // canonical-JSON hash — cross-fleet identity (mirrors kg TemplateVersion)
    pub parents: Vec<SchemaRef>,        // the inheritance DAG edges; [] = a root schema
    pub layer: SchemaLayer,             // THIS schema's own additions/overrides over the merged parents
    pub resolutions: Vec<ConflictResolution>, // REQUIRED where parents disagree (concern 2)
    pub migrations: Vec<Migration>,     // declarative structure migrations from version N-1 (concern 7)
}
pub struct SchemaRef { pub id: SchemaId, pub version: u32 } // parents are ALWAYS version-pinned — no floating inheritance

pub struct SchemaLayer {
    pub fields: Vec<FieldSchema>,        // the boring validation subset + defaults (concern 3)
    pub add_types: Vec<TypeDecl>,        // new node/edge/record types this layer introduces
    // the operator's floor from the onion round: add attributes; add node types; add edge types.
    // NOT a full schema language — escalation, not scope (mirrors kg PropSchema's "boring subset").
}
```

**Resolution (linearization).** Publishing a schema *flattens* its parent DAG into
one **effective schema** by a deterministic linearization (topological order over
`parents`, C3-style — the classic multiple-inheritance MRO), then applying this
schema's `layer` on top. The flattened result is what codegen and validation see;
the DAG is retained for provenance and re-flattening when a parent republishes.

**Flatten-on-parent-update + the residual rule (INTENT #131, preserved
verbatim-grade).** When a **parent** schema publishes a new version that now
*includes* what a child layer added, the child's contribution **FLATTENS** into
the parent on the child's next inherit-bump: the child re-inherits at the parent's
new version and drops the now-redundant layer entry. Residual misaligned fields
that the parent did *not* absorb **stay as a layer on top** — never forced into
the parent, never lost. The payoff, operator's words: "if I make a breakthrough in
the structure of my knowledge graph, I can update the template and all the rest of
the nodes inherit it without conflicts, because every deviation has been a delta on
top." This is a *tooling* operation, not automatic: `schema flatten <id>` computes
which layer entries a new parent version subsumes and proposes the reduced layer;
the operator/agent accepts. (No silent structural rewrite — same push-back
philosophy kg uses.)

### 2. Conflict resolution — explicit and manual at initialization (LOCKED #166 Q16)

**LOCKED (INTENT #166 Q16):** when two parents in the DAG define the *same* field
or type incompatibly (the "dual inheritance — merge two branches into a new child"
case, INTENT #150 beat 14), resolution is **explicit and manual at schema
initialization — no quiet overwrites.** Neither parent silently wins. The
linearization pass DETECTS every collision; for each, the child MUST supply a
`ConflictResolution` or **publication fails** with `SchemaError::UnresolvedConflict`.

```rust
pub struct ConflictResolution {
    pub at: FieldPath,                   // "props.priority" | type "task" — the colliding locus
    pub among: Vec<SchemaRef>,           // which parents disagree here (diagnostics + audit)
    pub choice: Resolution,
}
pub enum Resolution {
    TakeFrom(SchemaRef),                 // adopt this parent's definition verbatim
    Override(FieldSchema),               // the child defines its own, superseding all parents
    Rename { keep: Vec<(SchemaRef, String)> }, // keep both, under distinct names (no data loss)
}
```

The resolution set is **recorded in the published `SchemaDefinition`** (auditable,
immutable) — so "why does the merged `task` type look like this" is answerable
forever. A schema that changes a parent later and introduces a *new* collision
fails publication until the new collision is resolved too: conflicts can never
accrue silently across versions. This is schema's analogue of kg's
"surface-don't-destroy" and locks' "resolution is an ordinary explicit act."

### 3. Default values in schemas (INTENT #149 beat 4 — "might be pretty common")

Beat 4 CONFIRMED "schema owns schemas + layered inheritance, not content — with
one edge case: **default values in schemas**." A `FieldSchema` MAY carry a literal
default. This is the *only* content schema holds, and it does not breach the
two-engines rule: a default is a typed constant that is part of the type
definition (exactly Rust's `Default` / serde `default`), **not** rollup-composed
text — schema never *composes* it, it just carries it.

```rust
pub struct FieldSchema {
    pub name: String,
    pub ty: FieldType,                   // String|Int|Float|Bool|Timestamp|Enum(..)|Array(..)|Object(..)|Ref(SchemaRef)
    pub required: bool,
    pub default: Option<DefaultValue>,   // INTENT #149 beat 4 — the one edge case
    pub constraints: Vec<Constraint>,    // pattern|min|max|enum-values — the boring validatable subset
}
pub enum DefaultValue {
    Literal(serde_json::Value),          // a scalar/array/object constant
    // NB: NO "computed"/"expression" default in v1 — that is where a default would start
    // becoming composed content (rollup's job). Escalation, not scope. Two-engines held.
}
```

**Codegen honors defaults** (concern 4): a defaulted field emits
`#[serde(default = "…")]` + a `Default` impl arm on the generated struct, so an
omitted field decodes to its schema default rather than a hard error — and the
**same** default is served over the WS query surface, so a non-Rust consumer (or a
runtime validator) applies the identical constant. One default, two honest
surfaces (compiled + runtime) — the seam OQ-20 must keep consistent (concern 8).

### 4. The library concept + the emergent DC surface (INTENT #150 beat 16 / #166 Q11)

Beat 16, verbatim: the schema tool "produces Rust structs and needs a **library
concept** — versioned struct libraries, imported in code, also queryable over
WebSocket — one place where we define all our types and schemas with inheritance
and multiple inheritance." And #166 Q11: **DC (data contracts) is EMERGENT from
chassis + schema + generated Rust libraries — no DC service; "DC" kept only as the
name of the published-type surface.** These are one design:

**(a) The generated library — a versioned crate.** For a named *library set*
(a curated group of published schemas at pinned versions), `schema codegen` emits a
**standalone Rust crate** — `gen/<lib>@<version>` — of `#[derive(Serialize,
Deserialize)]` structs/enums, one per effective (flattened) schema type, defaults
wired in (concern 3), plus an embedded manifest constant:

```rust
// emitted into every generated library crate — the identity a consumer can check
pub const LIBRARY_ID: &str = "landscape-core";
pub const LIBRARY_VERSION: u32 = 7;
pub const MEMBERS: &[(&str /*SchemaId*/, u32 /*version*/, &str /*content_hash*/)] = &[ /* … */ ];
```

- **Layer-cycle avoidance (ledger §C, load-bearing).** Generated library crates
  depend ONLY on `serde`/`uuid`/`chrono` — **never on `substrate-types` and never
  on `schema` itself.** They are near-leaf crates, like `types`. This is what lets
  **chassis (L1) `Cargo`-depend on a generated library** (its `Contract` traits are
  synthesized over these structs — chassis.md concern 2) even though the `schema`
  *tool* is L4: the dependency is on the generated *artifact*, a codegen/shared-lib
  dependency, **not a contract edge** (chassis.md, ledger §A row 62). "surface-schema
  and the generated-type crate stay separate from `types` — no layer cycle"
  (ledger §C) is realized here.
- **Importable in code** = a plain path/registry Cargo dependency on
  `gen/<lib>@<version>`. Two services agreeing on a contract compile the *same*
  generated crate — the compile-time half of a data contract, with zero hand-wire
  work.

**(b) The WS query surface — the runtime half.** The `schema` service answers,
over WS (the DC surface): *list libraries + versions*; *fetch a `SchemaDefinition`
(raw DAG) or its flattened effective schema*; *fetch a library manifest*; *resolve
a `SchemaId@version` → its JSON-schema-shaped runtime descriptor* (for non-Rust
consumers, dynamic validators, and the dashboard's "every published data type"
view, INTENT #150 beat 16). This is the "**also queryable over WebSocket**" half —
the *same* schema data the crate was generated from, served live.

**(c) DC is therefore emergent, not a service (INTENT #166 Q11).** There is **no
`dc` crate.** "DC" is the *name of this published-type surface* (compiled libraries
+ WS query) that falls out of schema + chassis. The wave-2 "DC service" musing
(INTENT #151) is explicitly NOT built. Dashboard components ride this surface
(OQ-25 leaning, ledger §A row 62) as a downstream consumer — seam only, not
designed here.

### 5. The two-engines rule — the standing guardrail (INTENT #166 F-2)

Kept as a first-class invariant, not prose: **rollup composes TEXT; schema
migrates STRUCTURE; neither grows the other's job, and there is never a third
engine.** Concretely enforced in this design: schema holds no free-text content
(only the concern-3 literal defaults); schema's `codegen`/`migrate`/`validate`
surfaces never call rollup; and rollup's movement toward schemas+KG (INTENT #140,
rollup.md) consumes schema's *type shapes* but schema never consumes rollup. The
gravity-well worry is BLESSED AWAY (#166 F-2): schema accumulating surfaces
(inheritance, codegen, WS query, migration, defaults) is fine *because* every
surface is a facet of the one job — migrating structure — and the two-engines line
is the guardrail that keeps "many surfaces" from becoming "many jobs."

### 6. Storage, publication, immutability, versions — the `schema/` keyspace

Schema definitions and library manifests are small, LWW-friendly control-plane
records; they ride a **`schema/` keyspace on `replicated-kv`** (via chassis's
KvHandle — the exact precedent `kg` set with `kg/` and `vfs` with `vfs/`), so every
node answers "what schemas/libraries exist, at what versions" from a purely local
read.

```
schema/def/<schema_id>/<version>   -> SchemaDefinition   (IMMUTABLE once published — LWW-safe by construction)
schema/def/<schema_id>             -> SchemaHead          { latest: u32, yanked: Vec<u32> }
schema/lib/<lib_id>/<version>      -> LibraryManifest     (IMMUTABLE; the MEMBERS set + gen-crate coordinates)
schema/lib/<lib_id>               -> LibraryHead          { latest: u32, yanked: Vec<u32> }
```

- **Immutable-once-published** (mirrors kg's `TemplateVersion`): a published
  `(id, version)` never changes — codegen determinism and cross-fleet
  `content_hash` identity depend on it. Evolution = publish `version+1`.
- **`PublishSchema`** static-validates the definition (types well-formed,
  constraints coherent, every parent conflict carries a resolution — concern 2)
  BEFORE the KV write; a malformed schema is **rejected at publish, never at use
  time** (queues/kg declarative-data philosophy).
- **Self-seeded, not code-baked (INTENT #169).** The built-in schemas — the core
  keeper schema, the built-in keeper/artifact type schemas (concern 8), and the
  landscape-core library — ship as **DATA seeded deterministically into the
  `schema/` keyspace at install**, versioned like any other schema (exactly kg's
  three-built-in-templates pattern), NOT written into Rust and hoped-not-to-drift.
  Installers extend by publishing more schemas.

### 7. The migration engine — moves STRUCTURE across versions

The "migrates structure" half. A `Migration` is **declarative data, never code**
(mirrors kg's `migration_hints`): it describes how a record/graph conforming to
`version N-1` becomes one conforming to `version N`.

```rust
pub struct Migration { pub from: u32, pub to: u32, pub steps: Vec<MigrationStep> }
pub enum MigrationStep {
    AddField   { path: FieldPath, default: DefaultValue },   // additive → trivial, uses concern-3 defaults
    RenameField{ from: FieldPath, to: FieldPath },
    AddType    { decl: TypeDecl },
    NarrowType { path: FieldPath, to: FieldType },           // may fail validation → rejected-whole (below)
    // deliberately NO free-form transform expression — that is code, not declarative structure.
}
```

`schema migrate <target> --from <vN-1> --to <vN>` validates the ENTIRE target
population against `vN` after applying steps; **any nonconforming element →
rejected whole** with a full violation report (the target stays on `vN-1`,
untouched) — kg's exact rejected-whole migration posture, generalized. Additive
evolution migrates trivially; narrowing requires the population to already
conform or be fixed first. schema owns the *migration semantics*; **where the data
lives is the consumer's** — kg runs a graph migration through `kg-api`, VDB a
record migration through its actions, a config store a file rewrite. schema
computes the diff + validates; it never opens a database or a file itself
(two-engines + INTENT #29).

### 8. First-class consumers as SEAMS — keeper/artifact type schemas (INTENT #149 beat 11, #150, #165)

The re-spoken rounds make schema's *first-class customer* the landscape's type
system. schema designs the **type-schema mechanism**; the **keeper** (batch 6,
keeper.md) and **artifacts** (batch 6, artifacts.md) crates are the CONSUMERS —
designed there, referenced here as seams only, not designed in this file.

- **Keeper-type schemas (INTENT #149 beat 11 / #150 beat 15).** "Every seat/keeper
  TYPE has a schema defining its shared workspace [bundle] + its connection to the
  ephemeral workspace: a **CORE keeper schema** with per-type **INHERITED** schemas
  (new keeper types inherit their own bundle/workspace structure)." This is
  schema's inheritance engine (concerns 1–2) applied verbatim:
  `keeper.core@N` is a root schema; `keeper.project`, `keeper.app`,
  `keeper.marketing`, … inherit from it; the **dual-inheritance** case (INTENT #150
  beat 14 — "merge two branches of child keepers into a new child keeper") is
  concern-2 multiple inheritance with explicit resolution. Each keeper-type schema
  defines the **bundle** structure (persistent shared knowledge) and the
  **workspace** structure (per-thread ephemeral), both *versioned* so a bundle/
  workspace "plays perfectly with rollup" (INTENT #149 beat 11 — rollup reads the
  schema-defined shape and composes the text into it: two engines, one artifact).
  **Seam:** keeper.md owns *what those structures contain* and the runtime that
  instantiates them; schema owns only that they *are* schemas in this tool, with
  inheritance and versions. The CareerCrafter organization types (OQ-9) become
  additional inherited keeper-type schemas when that round lands — no schema change
  needed, just more published definitions.
- **Artifact-type schemas (INTENT #165).** "Artifact TYPES are schemas,
  inspectable and editable directly — like looking at the class of a Python object
  — so agents can improve skills/methods on the type over time; content is
  type-specific." Same mechanism: an artifact type (`artifact.skill`,
  `artifact.report`, …) is a `SchemaDefinition` describing the artifact's structure
  (its VFS/KG pointer fields, its methods/tools binding, its rollup config).
  Because schemas are inspectable/editable through schema's own surfaces (WS query
  + publish-new-version), "improving the type over time" is just publishing
  `version+1` — with the migration engine (concern 7) carrying existing artifacts
  forward. **Seam:** artifacts.md owns the artifact runtime, the bundle-appears-in-
  sight behavior, and the targeted-rollup wiring; schema owns the type-as-schema
  substrate.

Both consumers reach schema over the WS query surface (concern 4b) and compile
against the generated landscape-core library — the DC surface serving its most
important customer, the landscape itself.

### 9. Relationship to kg — the extraction seam (INTENT #131, recorded)

kg.md concern 3's template machinery (`TemplateVersion`, publication,
graph↔version binding, declarative migration, the boring PropSchema subset) is
schema's **extraction source and first consumer**: schema *generalizes* it (kg
templates are graph-shaped schemas; schema adds multiple inheritance, codegen, and
the WS/library surface that kg's local template store lacks). The boring v1 seam,
so the two batch-4 units don't collide: **schema and kg's `schema` child lib share
the same validation/inheritance model** (kg's internal `schema` lib is the same
engine, pre-extraction), and **kg keeps owning graph storage + merge + the `kg/`
registry** (kg.md concerns 1/2/4 unchanged by this file). Whether kg's
`TemplateVersion` becomes literally a `schema` `SchemaDefinition` (kg consuming the
standalone crate over WS) or stays kg-internal for v1 is a **sequencing call the
kg-side owns** — recorded as a seam here, NOT decided in schema.md, so neither
unit rewrites the other. kg.md carries the reciprocal extraction note.

### 10. OQ-20 seam — types↔schema authority-of-record (OPEN; both answers kept reachable)

**OPEN by directive (ledger §B.2, §C).** schema emits two representations of the
same shape: (a) **compiled** Rust structs in a generated library crate, and (b)
**runtime** schema data over the WS query surface. When they *disagree* — a service
compiled against `landscape-core@7` while the live registry serves `@8` — **which
is authority?** This file does NOT decide (it would invert the current
authored-contracts-are-authority posture, ledger §C). The bridge is designed so
either policy is a one-line change later:

- **Every generated library embeds `(LIBRARY_ID, LIBRARY_VERSION, MEMBERS[content_hash])`**
  (concern 4a), and the **runtime registry serves the identical triple** (concern
  6). So a consumer can always *detect* skew cheaply: compare compiled
  `LIBRARY_VERSION`/`content_hash` against the registry's `latest`.
- **Detection is separated from decision.** schema ships the comparison
  (`schema check --compiled <manifest>` → `Match | CompiledAhead | RuntimeAhead`)
  but takes **no** action on a mismatch. Whether a service **refuses** to run
  below the registry's version (runtime-as-authority), **warns and proceeds**
  (compiled-as-authority — today's posture), or **triggers a Compatibility restart**
  to pull the new library, is a **policy hook left OPEN** — a single enum a future
  round sets. Nothing in the codegen or the registry presumes the answer.
- **Kept flexible (ledger §C):** the generated library crate stays a separate,
  near-leaf crate from `types` (no layer cycle); authored `types` contracts stay
  hand-authored and authoritative for now; the runtime WS surface is purely
  advisory until the policy is chosen. Marked **OPEN** in the contract graph.

### 11. OQ-21 seam — schema as the universal messaging protocol (OPEN; #139)

**OPEN by directive (ledger §B.2, §C4).** INTENT #139: "the ENTIRE messaging
protocol in the mesh could be done using schemas and schema inheritance" — every
envelope, event, restart frame, and per-contract payload could be a
`schema`-managed schema, with contract evolution expressed as schema inheritance +
migrations. This is a big-unification idea the operator reserved for **its own
round**; it is **NOT applied to the authored wave-2 contracts** here. The codegen
bridge is designed so it *could* slot in with **no wire change**, exactly mirroring
`contracts/mesh-transport.md`'s seam paragraph:

- Today a `Frame`'s `Request.payload` / `ResponseOutcome::Success.payload` is an
  **opaque `RawValue`** whose concrete shape is fixed by the authored per-edge
  contract's hand-written `types` structs; #154's "a schema which specific
  inter-application messages inherit from, going inward" is satisfied by those
  authored structs.
- If #139 lands, that inward schema becomes a **schema-tool-managed inheritance
  chain** (versioned, multiply-inheriting, codegen'd to Rust *and* queryable over
  WS — concerns 1/4), and `method`/`payload` carries a **`SchemaId@version`**
  rather than an authored-struct name. schema's codegen output is *already* the
  drop-in equivalent of the hand-authored payload structs — that is the entire
  technical enablement.
- **The transport stays payload-opaque precisely so it carries either form**
  (mesh-transport.md §"Schema-inheritance future"): the `Frame` header, addressing,
  `ResponseOutcome` switch, and version stamping are unaffected by whether the
  inward payload is a hand-authored struct or a schema-inherited one. **That is the
  whole seam; the substitution itself is OQ-21's round to design.** schema.md
  applies #139 to **nothing** in this pass.

## Relationships / edges

`schema` is a top-level app-crate; its consumers reach it over the local `:3649`
daemon (INTENT #29 — never imported as a running library by an app), with **one
deliberate exception in the codegen direction**: the *generated library crates* it
emits ARE Cargo-imported (they are separate near-leaf artifacts, not `schema`
itself). Edges:

- **generated DC libraries → chassis / every service** — a **codegen / shared-lib
  dependency, NOT a contract edge** (ledger §A row 62; chassis.md concern 2 /
  "Relationships"). chassis synthesizes its `Contract` traits over these crates;
  the exhaustive-case enforcement (chassis concern 3) rests on their generated
  enums. → `scaffold/components/chassis.md`.
- **schema-query (WS) — any service ↔ schema (NEW pair; proposed below).** The DC
  runtime surface: list/fetch schemas + libraries + versions, resolve
  `SchemaId@version` → runtime descriptor, `check` a compiled manifest (OQ-20).
  Surface-schema-style: one document, every service a potential party.
- **schema ↔ replicated-kv (`schema/` keyspace)** — via chassis's KvHandle
  (concern 6); the registry-of-schemas, readable locally on every node. Rides
  `kv-replication`; not a new pair (the keyspace-claim precedent kg/vfs set).
- **kg (extraction seam, concern 9)** — kg's template/PropSchema machinery is
  schema's generalization target; **no forced edge in v1** (kg keeps its internal
  engine; sequencing is kg-side). Recorded, not authored.
- **keeper / artifacts (batch 6 consumers, concern 8)** — keeper-type and
  artifact-type schemas ARE schemas in this tool; those crates own the runtime,
  schema owns the type substrate. Seam only.
- **rollup (two-engines boundary, concern 5)** — rollup consumes schema's type
  shapes (INTENT #140, rollup.md); schema never consumes rollup. Boundary, not an
  edge schema owns.

Cross-cutting protocols consumed (not authored): `service-lookup` (register
`schema`), `restart-protocol`, `pubsub-protocol` (claims the `schema.*` topic
prefix: `schema.published`, `schema.library.published`, `schema.migrated`),
`surface-schema` (schema's dashboard component: published schemas/libraries,
version heads, pending conflicts). Imports `substrate-types` for shared ID/
provenance vocabulary — a shared-lib dependency, NOT a contract edge. (Note: the
*generated* libraries do NOT import `types` — concern 4a, the anti-cycle rule.)

## Nesting

Parent: none (top-level app-crate `bin/schema` + `lib/schema`). Children (nested
internal libs, compiled into the schema daemon/CLI, never standalone — INTENT #22):
**`model`** (the `SchemaDefinition` / `FieldSchema` / `Migration` data model +
canonical hashing), **`resolve`** (DAG linearization, C3 ordering, conflict
detection — concerns 1–2), **`codegen`** (Rust struct/enum emission → versioned
library crates, defaults wired — concern 4a), **`registry`** (the `schema/`
keyspace: publish, immutability, heads — concern 6), **`migrate`** (declarative
structure migration + rejected-whole validation — concern 7), **`query`** (the WS
DC surface + `check` — concerns 4b/10). Flat `components/` naming per convention;
the tree lives here and in overview.md.

## Thoroughness level

**implementation-ready** for: the DAG inheritance model + flatten/residual
semantics (concern 1); the explicit-manual conflict resolution mechanism, LOCKED
(concern 2); default values + their codegen/runtime honoring (concern 3); the
generated-library concept, the anti-layer-cycle rule, and the emergent WS DC
surface (concern 4); the two-engines guardrail as an enforced invariant (concern
5); the `schema/` keyspace registry with immutability + self-seeding (concern 6);
the declarative migration engine + rejected-whole posture (concern 7); the
keeper/artifact type-schema seams (concern 8); the kg extraction seam (concern 9);
and the OQ-20/OQ-21 seams designed for both-answers-reachable (concerns 10–11).
**Deliberately OPEN (by directive, not gap):** the types↔schema authority-of-record
policy (OQ-20) and applying #139 messaging-on-schema to authored contracts (OQ-21)
— both left as one-line-settable hooks, marked OPEN in the contract graph.

## Assigned design-depth

Opus, single Component-Designer pass (this file), grounded in `types.md` +
`chassis.md` (the generated-library / DC-surface consumer), `kg.md` concern 3 (the
template model schema generalizes), `contracts/mesh-transport.md` (the OQ-21
payload-opaque seam mirrored), and INTENT #131/#139/#145/#149/#150/#151/#165/#166/
#169, ledger §A rows 59–62, §B.2 (OQ-20/OQ-21), §C.

## Suggested fill-model

**implementation-ready + medium-high complexity → strong-mid model, with `resolve`
and `codegen` written first.** Clean risk split: (a) `model`/`registry`/`migrate`
are declarative data + KV records + validation sweeps — near-spec, heavily
testable → **mid model** with conformance fixtures (publish-immutability,
rejected-whole migration, self-seed determinism). (b) **`resolve` is a
do-not-cheap-out surface**: the C3 linearization + collision detection + the
LOCKED explicit-resolution gate must be *provably* total — a two-parent-conflict
that produces a silent overwrite is a direct INTENT #166 Q16 violation; write the
"every collision either resolved or publication-fails" property test first. (c)
**`codegen` is the other do-not-cheap-out surface**: the anti-layer-cycle rule
(generated crates depend on serde/uuid/chrono ONLY, never `types`/`schema`) and
default-wiring (`#[serde(default)]` + `Default` impls matching the runtime
descriptor byte-for-byte — the OQ-20 consistency property) must be enforced by the
generator and checked, not assumed. (d) `query` is transcription-grade over
`registry`. Fill after `types` (ID vocabulary) and after chassis's `Contract`-trait
codegen surface is defined (they meet at the generated libraries).

---

## Proposed contracts (wave 3)

`schema` owns the **shape** of one genuinely new capability whose contract *file*
does not yet exist; it is proposed here for the harmonizer / batch-7 to author (I
do not create files outside my ownership). It is a proposal — the shape schema puts
forward — not a finalized reconciliation.

### P1. `schema-query` (NEW — proposed; the emergent DC runtime surface, INTENT #166 Q11 / #150 beat 16)

Surface-schema-style: one document, any service a party; schema is the server half.
The **runtime** half of the DC surface (the compiled half is the generated library
crates — a codegen dependency, not a contract). Proposed request set:

```rust
enum SchemaQuery {
    ListSchemas,                                    // -> Vec<(SchemaId, SchemaHead)>
    GetDefinition   { id: SchemaId, version: u32 }, // -> SchemaDefinition (raw DAG)
    GetEffective    { id: SchemaId, version: u32 }, // -> EffectiveSchema (flattened; codegen input)
    ListLibraries,                                  // -> Vec<(LibraryId, LibraryHead)>
    GetLibrary      { id: LibraryId, version: u32 },// -> LibraryManifest (MEMBERS + gen coordinates)
    CheckCompiled   { manifest: LibraryManifest },  // OQ-20: -> Match | CompiledAhead | RuntimeAhead (report ONLY, no action)
    Publish         { def: SchemaDefinition },      // static-validated; rejects UnresolvedConflict (concern 2)
    PublishLibrary  { spec: LibrarySpec },          // pins a member set -> emits gen crate + manifest
    Migrate         { target: MigrationTarget, from: u32, to: u32 }, // rejected-whole (concern 7)
}
```

- **DC-emergent flag (INTENT #166 Q11).** This is NOT a `dc` service. "DC" names
  the pair *(generated libraries + this WS query surface)*; no separate crate is
  introduced. → target `scaffold/contracts/schema-query.md` *(to be authored by the
  harmonizer / batch-7)*.
- **OQ-20 seam.** `CheckCompiled` returns a report and takes no action; the
  authority-of-record policy (refuse / warn / compat-restart on skew) is an OPEN
  hook, not part of this contract. Marked OPEN.
- **OQ-21 seam.** This contract carries schema *definitions*; it deliberately does
  NOT route inter-service *messages* through schema. If #139 lands, message payloads
  would reference `SchemaId@version` resolvable here — but that substitution is
  OQ-21's round, and `mesh-transport`'s payload-opaque `Frame` already carries either
  form with no wire change (concern 11). Nothing here presumes it.

### P2. Codegen dependency (recorded, NOT a contract) — generated libraries → chassis

The generated library crates are consumed by chassis (its `Contract` traits) and
any service, as a **Cargo / codegen dependency, not a contract edge** (ledger §A
row 62; chassis.md). Recorded here so the graph is honest: no `schema`↔`chassis`
contract file exists or should — the meeting point is the generated artifact, which
depends on serde/uuid/chrono only (the anti-layer-cycle rule, concern 4a).
