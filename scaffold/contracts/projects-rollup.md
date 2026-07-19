# Contract: projects-rollup

## Parties
- `projects` (L6 stub) `->` `rollup` (L4).

*(Stub-track. rollup.md's `projects-rollup` note reserves exactly this; no new
mechanism. Content deferred until `projects` leaves the stub track.)*

## Purpose
*"Tasks are rolled up"* (INTENT #51): `projects` derives a `ScopeChain` from the
project hierarchy and resolves task/artifact bodies via `rollup`'s
fragment-reference system — the same rollup that assembles CCD plugins assembles
project task/artifact content.

## Rough shape
The existing `rollup-mesh` `Resolve` / `Materialize` surface with a
**projects-supplied scope** — every rollup invariant unchanged:
- `projects` walks the `project-tree@1` hierarchy (from `projects-kg`) to build a
  `ScopeChain` (parent → child scope precedence).
- It calls `rollup`'s `Resolve`/`Materialize` with that scope + the task/artifact
  fragment reference; rollup resolves fragment-references, slots, and the
  raw-vs-reference insert types (INTENT #94).
- **Secret safety inherited:** rollup never resolves a secrets raw reference into
  LLM-bound content (`rollup-secrets`); `projects` relies on that guarantee.
- No new rollup mechanism; `projects` is a plain `rollup-mesh` consumer with a
  project-derived scope.

## Open questions
- Exact `ScopeChain` derivation from nested projects (does a sub-project's scope
  shadow or merge the parent's) — leaning parent-then-child precedence.
- Whether artifact bodies rollup through here or through `projects-artifacts`
  once `artifacts` exists (leaning: `artifacts` owns typed bodies, rollup
  assembles their textual/prompt parts).
