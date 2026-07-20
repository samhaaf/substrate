# onion

> **⚠ CANDIDATE CRATE — REQUIREMENTS-ONLY STUB (friction-round 2,
> 2026-07-20, INTENT #122).** Existence and placement are both UNDECIDED.
> This file is verbatim intent capture only: no design, no schemas, no API,
> no layer assignment. **A dedicated discussion round on KG templating is
> REQUIRED before any design work here or any change to kg.md's template
> model.** Thoroughness: **requirements-only.**

**Status:** NEW candidate at friction-round 2 — the schema-delta layering
crate. **Track:** candidate (not in the 44-module tree; listed in
overview.md's tree as candidate, placement undecided). **Prior art:** the
operator's EHR data-lake system — per-practice masks over an EHR template
onto one central schema.

## Charter (requirements, verbatim-grade)

`onion` is a candidate crate for **schema-delta layering**: a language for
**DELTAS over schema templates** — starting from KG's template model
(kg.md concern 3), but deliberately more general.

- **The delta language.** Deltas over a template can: add attributes to
  nodes/edges, add new node types, add new edge types.
- **Flatten-on-template-update semantics.** When the underlying template is
  updated to include a delta, the delta **FLATTENS** into it.
- **The residual-delta rule.** Residual misaligned fields stay as deltas on
  top — they are never forced into the template and never lost.
- **The payoff, in the operator's words:** "If I make a breakthrough in the
  structure of my knowledge graph, I can update [the template] and all the
  rest of the nodes inherit it without conflicts, because every deviation
  has been a delta on top."
- **Applicable beyond graphs.** Not KG-specific: the same mechanism applies
  to YAML, JSON schema, etc. — "the same philosophy inside the crate, used
  to do templated behaviors and push changes to multiple services at once."
- **Must be boring.** Like everything else in the scaffold, and stated
  explicitly by the operator for this concept.

## Relationship to kg (recorded, not designed)

kg.md's templating / schema-inheritance machinery (its concern 3: the
`TemplateVersion` model, publication, graph↔version binding, migration) is
the candidate **extraction source** — kg would then consume `onion` rather
than owning template mechanics itself. kg.md carries the reciprocal note.
Nothing moves until the KG-templating discussion round happens; kg's
current template model stands as-is until then.

## Open questions (all of them — nothing is decided)

1. Does `onion` exist at all, or does the delta idea land inside kg's
   template system?
2. If it exists: shared-lib or service? Which layer? (Unplaced in the tree
   on purpose.)
3. The delta language's exact scope (the three listed operations are the
   operator's floor, not a spec).
4. Flatten/residual semantics precisely — what counts as "the template now
   includes the delta," and how are residuals surfaced?
5. The beyond-KG story (YAML / JSON-schema targets; "push changes to
   multiple services at once") — same crate or a family?

**Deferred whole to the dedicated KG-templating discussion round (REQUIRED
before design — INTENT #122).**
