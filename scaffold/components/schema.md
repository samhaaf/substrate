# schema

> **RENAMED `onion` → `schema` (friction-round 3, 2026-07-20, INTENT #131).**
> The operator: "instead of onion, something more understandable — we'll just
> call it schema. Our crate for managing schemas with versions and layering.
> One boring tool for schema management, used within all the other services."
> The name is settled; the crate is ACCEPTED as a concept (no longer
> existence-undecided) — but placement/layer is still unassigned and **no
> design work happens before the dedicated KG-templating discussion round
> (INTENT #122, still REQUIRED).** This file remains verbatim intent capture
> only: no schemas, no API, no layer assignment. Thoroughness:
> **requirements-only.**

**Status:** ACCEPTED-as-concept candidate crate (friction-round 3, INTENT
#131; introduced as `onion` at friction-round 2, INTENT #122) — the
schema-management crate. **Track:** candidate (not in the live-module tree;
listed in overview.md's tree as candidate, placement undecided). **Prior
art:** the operator's EHR data-lake system — per-practice masks over an EHR
template onto one central schema.

## Charter (requirements, verbatim-grade)

`schema` is the crate for **schema management with versions and layering**:
"one boring tool for schema management, used within all the other services."
It starts from KG's template model (kg.md concern 3) but is deliberately more
general.

- **NESTED inheritance to arbitrary depth (INTENT #131 — supersedes the
  single-layer template+delta wording).** The layering model is not "one
  template plus one delta mask": **a schema inherits from another schema and
  applies migrations, to arbitrary depth** — "this schema inherits from that
  schema and applies these migrations, to arbitrary depth." A chain (or DAG —
  open) of schemas, each layer a set of migrations over its parent.
- **The layer operations** (the operator's floor from the `onion` round, not
  a spec): add attributes to nodes/edges, add new node types, add new edge
  types.
- **Flatten-on-parent-update semantics.** When a parent schema is updated to
  include what a child layer added, the child's contribution **FLATTENS**
  into it.
- **The residual rule.** Residual misaligned fields stay as a layer on top —
  never forced into the parent, never lost.
- **The payoff, in the operator's words (from the `onion` round):** "If I
  make a breakthrough in the structure of my knowledge graph, I can update
  [the template] and all the rest of the nodes inherit it without conflicts,
  because every deviation has been a delta on top."
- **Applicable beyond graphs.** Not KG-specific: the same mechanism applies
  to YAML, JSON schema, etc. — "the same philosophy inside the crate, used
  to do templated behaviors and push changes to multiple services at once."
- **Versions are first-order.** Schemas carry versions; inheriting a schema
  means inheriting at a version and applying migrations forward.
- **Must be boring.** Stated explicitly by the operator.

## The BIG unification direction (friction-round 3, INTENT #139 — recorded, NOT applied)

The operator's verbatim-grade extension: "Onion [schema] might handle the
restart protocols — but that's one step short: the **ENTIRE messaging
protocol in the mesh could be done using schemas and schema inheritance**."
I.e. every mesh-crossing message shape — envelopes, events, restart frames,
the per-contract payloads — could be a `schema`-managed schema, with contract
evolution expressed as schema inheritance + migrations. This is a
**discussion/design item for the next pass** — it is deliberately NOT applied
to the authored wave-2 contracts, which stand as written until that round.
(Related: rollup's movement toward schemas+KG, INTENT #140 — see rollup.md
concern 8.)

## Relationship to kg (recorded, not designed)

kg.md's templating / schema-inheritance machinery (its concern 3: the
`TemplateVersion` model, publication, graph↔version binding, migration) is
the candidate **extraction source** — kg would then consume `schema` rather
than owning template mechanics itself. kg.md carries the reciprocal note.
Nothing moves until the KG-templating discussion round happens; kg's
current template model stands as-is until then.

## Open questions (existence is settled; everything else is not)

1. ~~Does the crate exist at all?~~ SETTLED (INTENT #131): it is "our crate
   for managing schemas." Whether the delta machinery *also* stays inside
   kg's template system during a transition is a sequencing question for the
   discussion round.
2. Shared-lib or service? Which layer? (Unplaced in the tree on purpose.)
3. The inheritance model's exact scope — chain vs DAG, the migration
   language, what a "layer" may add/change (the three listed operations are
   the operator's floor, not a spec).
4. Flatten/residual semantics precisely — what counts as "the parent now
   includes the layer," and how are residuals surfaced?
5. The beyond-KG story (YAML / JSON-schema targets; "push changes to
   multiple services at once") — same crate or a family?
6. The INTENT #139 unification: does the mesh messaging protocol ride schema
   inheritance, and what does that do to the authored contracts? (Its own
   discussion round; see above.)

**Design deferred whole to the dedicated KG-templating discussion round
(REQUIRED before design — INTENT #122; reaffirmed at friction-round 3).**
