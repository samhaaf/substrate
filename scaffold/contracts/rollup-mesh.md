# Contract: rollup-mesh

> New pair (wave2-plan §3b). Authored from rollup.md's `rollup-mesh` proposal
> (the authoring side) reconciled with queues.md's consumer-side
> `ResolveReferences` ask (the trigger-assembly caller).

## Parties

rollup (L4 app, `AddressingClass::AnyNode`) ↔ mesh (`:3649`) — and, through the
mesh relay, any service calling rollup's resolve surface (chiefly a `queues`
trigger's `AssemblyTemplate`).

rollup authors this edge. Two facets: (a) rollup's own registration
(`service-lookup` instance); (b) the **resolve surface** callers invoke.

## Purpose

1. **Registration / resolution** — rollup registers `slug="rollup"`
   (`AnyNode` — stateless request/response; fragments live in mesh-replicated
   VFS, so any node's rollup daemon can serve any resolve). Callers resolve it by
   slug.
2. **The resolve surface** — turn `RollupRef`s / a `RollupTarget` into text +
   provenance. The primary caller is a `queues` **trigger** whose declarative
   `AssemblyTemplate` contains a `Rollup(RollupRef)` node (INTENT #101/#103:
   triggers are declarative DATA; assembly may call rollup, INTENT #79 slots at
   reference time). This answers queues.md's consumer-side `ResolveReferences`
   ask.

## Schema

Reused from `types::rollup`: `RollupTarget`, `RollupRef`, `RefTarget`,
`InsertForm`, `VersionSpec`, `SlotMap`, `ScopeChain`, `Resolved`,
`PluginManifest`, `MaterializeResult`, `RollupError`, `CallerContext`;
`OutputSink` (from `rollup-cc`).

```rust
// caller -> rollup (over pubsub-protocol / mesh WS)
enum RollupClientMsg {
    Resolve      { target: RollupTarget, slots: SlotMap, scope: ScopeChain, caller: CallerContext },
                 // -> Resolved { text, stable_prefix_len, provenance }
    ResolveRefs  { refs: Vec<RollupRef>, subject: serde_json::Value,
                   scope: ScopeChain, caller: CallerContext },
                 // -> Vec<ResolvedInsert>   (the trigger-assembly subset queues needs)
    Materialize  { manifest: PluginManifest, slots: SlotMap, scope: ScopeChain, output: OutputSink },
                 // -> MaterializeResult      (same op as rollup-cc::AssemblePlugin)
    ListFragments{ scope: ScopeChain },
                 // -> Vec<FragmentUse>       (metadata only — no bodies)
}

enum ResolvedInsert {
    Inlined(String),                                  // Raw form: the expanded content
    Reference(RollupRef),                             // Reference form: a lazily-loadable handle
    Degraded { r#ref: RollupRef, warning: String },   // a degraded secret (never a failure)
}
```

`subject` on `ResolveRefs` is queues' generic subject document (the event /
assembled payload) so a fragment's `{{slot:}}` markers bind event variables at
reference time (queues.md concern 2).

## Error cases

- All `RollupError` arms, delivered as a **first-class error frame** so a
  trigger's assembly reports `AssemblyFailed` (queues.md) rather than panicking —
  a bad reference fails that assembly, never the whole queue.
- A **degraded secret is NOT an error** — it is `ResolvedInsert::Degraded` (the
  delivery proceeds with the reference + warning; no leak, no failure). This is
  the `rollup-secrets` invariant surfaced on the trigger-assembly path.
- **Mixed-version tolerance:** an unknown `RefTarget`/`InsertForm` variant
  (`#[serde(other)]`) fails *that reference* loudly and locally without crashing
  the resolve or dropping sibling fragments.
- Registration errors are `service-lookup`'s.

## Version sensitivity

- **HIGH — the highest-churn edge in the cluster.** `RollupRef` / `RefTarget` /
  `InsertForm` / `VersionSpec` cross nodes AND persist *inside trigger
  definitions* in `replicated-kv` (a trigger is declarative data). So:
  additive-only, every enum reserves `#[serde(other)]`, every new field
  `#[serde(default)]`; `FragmentId` / scope prefixes are open strings, never
  closed enums.
- **Frozen (LOCKED, INTENT #94):** the **raw-vs-reference distinction** is part
  of the contract, not commentary — `InsertForm` may never collapse its two arms.
- An older rollup daemon receiving an unknown `RefTarget` degrades per the
  mixed-version rule above (fails that reference, keeps the assembly).

## Reconciliation notes

- **`ResolveRefs` vs queues' `ResolveReferences` — rollup's name wins (it's the
  surface owner); queues' name recorded as the alias.** queues.md proposed the
  consumer view under the name `ResolveReferences`; rollup.md authored it as
  `ResolveRefs`. Since rollup owns and serves the surface, `ResolveRefs` is
  canonical. The two describe the identical operation (resolve a `Vec<RollupRef>`
  against an event subject) — no semantic disagreement, only a name. Noted so
  queues' fill imports `ResolveRefs`.
- **`Materialize` shared with `rollup-cc`.** `RollupClientMsg::Materialize` and
  `rollup-cc::AssemblePlugin` are one operation (see `rollup-cc` reconciliation
  notes); the generic `rollup-mesh` surface carries it under `Materialize` with a
  `slots` field, cc's named edge under `AssemblePlugin` with `runtime_slots`.
  One implementation.
- **No competing mesh-side proposal** for registration — plain `service-lookup`
  instance, adopted verbatim. `AnyNode` addressing is rollup's stateless-service
  property (rollup.md concern 10), not contested.

## Example data

A `queues` trigger on **macbook** assembles an incident-investigation payload
(the `cc-escalation` path): its `AssemblyTemplate` has a `Rollup(RollupRef)`
node that inlines an incident-summary fragment and *references* (not inlines) the
`code-review` skill:

```jsonc
// queues trigger -> rollup: ResolveRefs, binding the event subject into slots
{ "ResolveRefs": {
    "refs": [
      { "target": { "Prompt": "incident-summary" }, "form": "Raw",
        "version": "Latest", "inline_slots": { "queue": "deploy-verify" } },
      { "target": { "Skill": "code-review" }, "form": "Reference",
        "version": "Latest", "inline_slots": {} } ],
    "subject": { "event_type": "deploy.failed", "repo": "demo", "env": "demo/prod",
                 "correlation_id": "c0110000-0000-4000-8000-000000000001" },
    "scope": { "scopes": [
        { "id": "proj:demo", "name": "demo", "prefix": "vfs://prompts/demo/" },
        { "id": "global", "name": "global", "prefix": "vfs://prompts/global/" } ] },
    "caller": { "service_slug": "queues", "llm_safe": true, "purpose": "assemble deploy-failed investigation" } } }

// rollup -> queues
[ { "Inlined": "Incident on queue deploy-verify: the demo/prod deploy failed…" },
  { "Reference": { "target": { "Skill": "code-review" }, "form": "Reference",
                   "version": "Latest", "inline_slots": {} } } ]
```

The assembled payload feeds the `cc-escalation` investigating agent; the
`code-review` skill stays a reference (token-saving, prefix-cache-stable),
carrying the same `correlation_id` across the causal chain.
