# types

## Charter

`substrate-types` (`lib/types`) is the zero-dependency shared-contract-types
foundation at the bottom of the workspace's dependency graph: every other
crate imports it, and it imports no other `substrate-*` crate. It holds only
data — IDs, enums, request/response/event structs, the workspace error
taxonomy (`SubstrateError`/`Result<T>`), and the `Promise`/`PromiseSender`
handoff primitive — organized one module per domain (`completion`,
`collection`, `model`, `estimate`, `stream`, `system`, `error`, `promise`).
It owns **no I/O, no async business logic, no service behavior, and no
opinion about how any other crate uses these types** — it is pure vocabulary.
Concretely it is what every `scaffold/contracts/<edge>.md` schema section will
be expressed IN TERMS OF: it is not itself a party to a request/response edge,
it is the shared language every edge's schema is written in.

> **SUPERSEDED (2026-07-18, round-3): the as-is freeze.** This file previously
> confirmed `types` "as-is this pass — no reshape, no new modules." The
> operator removed that freeze: **`types` is now UNLOCKED — additions and
> updates are welcome** (subject to the discipline guardrails below, which
> stand). First planned addition: a new **surface-schema domain module** (e.g.
> `surface.rs`) defining the "boring surface schema" language — the shared
> types every service uses to publish a schema of its observable surface (how
> to render its dashboard component + what calls to make against it), from
> which the mesh dashboard renders every service's component. See
> `scaffold/contracts/surface-schema.md`.
> **Second planned addition (rounds 4–5 lock, 2026-07-18): a WS envelope /
> pub-sub struct domain module** (e.g. `pubsub.rs`) — the typed structs
> publishers and subscribers share under mesh's now-confirmed standard
> WebSocket pub/sub protocol ("we just have certain structs that the
> publishers and subscribers expect"; mesh relays them where they need to
> go). See `components/mesh.md` concern 7. Requirements-only; shapes
> undesigned (a Contract Harmonizer concern, like surface-schema).
> **Third planned addition (round-8 lock, 2026-07-19): a standardized
> EVENT-TYPE struct module** (e.g. `event.rs`) — operator, verbatim: "I
> want a standardized struct for event types." Under the round-8
> queues/events/triggers/handlers vocabulary (see `components/mesh.md`
> concern 14), mesh's queues hold TYPED events; triggers filter by event
> type and by payload content per event type, then assemble the handler's
> payload. The standardized event struct is the shared vocabulary that
> filtering/assembly is written against. Requirements-only; shape
> undesigned (Contract Harmonizer concern; likely related to, but distinct
> from, the pub/sub envelope module).

It remains on a growth path (mesh's OQ-3 `NodeInfo`/`NodeCapabilities`
enrichment, the surface-schema module, the WS pub/sub envelope module, and
whatever `db`/`ccd`/`vfs`/`projects`/`stack`/`org` end up needing to share), so this
file exists to record the discipline that keeps that growth from turning the
crate into an undifferentiated dumping ground as the crate count roughly
doubles in V2.

## Primary design concerns

Low complexity in isolation — the hard part is not any one type, it's staying
disciplined at the ONE seam every crate touches, where entropy is cheapest to
introduce and most expensive to reverse (any correction here is a breaking
change felt by every downstream crate simultaneously). Three concrete
guardrails, in order of how much teeth they have:

1. **Zero-dependency invariant is a hard line, not a guideline.** The only
   permitted dependencies are pure data/utility crates already in use (`serde`,
   `serde_json`, `uuid`, `chrono`, `thiserror`, and `tokio` — used ONLY for
   `oneshot` inside `promise.rs`, not for any async runtime behavior). It must
   NEVER depend on another `substrate-*` crate (that would create the cycle
   the whole design avoids) or on an I/O-flavored crate (`reqwest`, `axum`,
   `rusqlite`, `sqlx`, etc. — `error.rs` already documents this explicitly:
   downstream crates convert their native errors to a `String` payload rather
   than this crate learning their error types). This is the single most
   important thing a reviewer checks on any PR touching this crate.

2. **Inclusion test, not vibes.** A type earns a place here only if it appears
   in the public signature of **two or more** crates, or is literally part of
   a named contract edge's schema in `scaffold/contracts/`. A type that is
   merely "convenient to share" but used by exactly one crate stays in that
   crate. This is the actual anti-dumping-ground rule — it gives a yes/no
   answer instead of a judgment call every time someone is tempted to add
   "just one more struct" here because it's the path of least resistance.

3. **One module per domain, no `common.rs`/`misc.rs` catch-all — ever.** The
   existing 8 modules each own one domain. As V2 adds real cross-cutting
   concerns (mesh's node-capability/topology types, whatever `db` or `ccd`/
   `org` end up needing to share), each NEW domain gets its OWN new module
   (e.g. a future `mesh.rs`, not fields bolted onto `system.rs`'s existing
   `NodeInfo` for an unrelated concern). A PR that adds a foreign-domain type
   into an existing module, or adds a grab-bag module, is the one shape of
   change to reject on sight. (Existing exception already in the crate:
   `NodeId`/`NodeInfo` intentionally live in `system.rs` because they were
   judged the same domain as system/telemetry snapshots at the time they were
   added — OQ-3, carried forward from the Decomposer pass, is exactly the
   question of whether that's still true once mesh's capability data grows;
   see Relationships below.)

**A sharper, non-obvious instance of concern 3 worth calling out on its own:**
`SubstrateError` is currently ONE flat enum, and it is already growing by
appending domain-tagged, string-payload variants per consuming crate (`Store`,
`Db`, `Engine`, `Transport`, `ServerUnhealthy`, `ServerCrashed`, ...). That
pattern scales O(error-cases), not O(components) — every new top-level V2
component that wants its own failure modes (mesh, ccd, org all plausibly will)
adds more leaves to one already-long enum, and unlike the module-per-domain
data types, an error enum can't simply be handed its own file without still
being reachable from the one flat `SubstrateError` every crate matches on.
The discipline this crate should hold going forward: when a new component
needs new error cases, add **one** new top-level `SubstrateError` variant that
wraps a domain-scoped sub-enum defined in its own submodule (e.g.
`SubstrateError::Mesh(error::mesh::MeshError)`), not N new flat leaf variants
sprinkled into the top-level enum. That keeps the top-level match arms
growing at O(components) while still letting each domain's error detail live
in its own place — the same module-per-domain rule, just applied one level
down inside `error.rs` instead of skipped there. This is **not implemented
today** (today's variants are already flat) and I have deliberately NOT
migrated the existing variants as part of this design pass — see Controversial
decisions below.

## Relationships / edges

`types` is not a two-party contract edge itself — it has no runtime
request/response of its own — it is the shared vocabulary every OTHER edge in
`overview.md`'s contract graph is defined in terms of (every `schema` section
in `scaffold/contracts/*.md` will cite types from this crate once the Contract
Harmonizer writes them). It is imported by all 19 top-level/nested components
in the tree. No new contract stub is needed for it; instead, two carried-
forward decision points at the type level, both flagged, neither resolved
here (Contract Harmonizer / operator calls, not mine to make):

- **OQ-3** (from `mesh-design-synthesis.md`, carried into `overview.md`):
  final shape of an enriched `NodeInfo` (or a new, separate
  `NodeCapabilities`) to carry `roles`, `accelerator_present`,
  `models_available`, `models_resident` for the mesh's model-affinity
  balancer — and whether that still belongs bundled into `system.rs` or
  earns its own `mesh.rs`-adjacent module per concern 3 above. Recommend the
  latter (a new module) if the field count grows past a handful, on the same
  reasoning as concern 3, but this is the Contract Harmonizer's call once it
  has both `NodeInfo`'s existing consumers and mesh's new consumers in hand.
- **`db-inference-init`** (new edge this round, `inference -> db`): whether
  the payload needs any new shared type here (e.g. a `DbInitSpec`) or is
  fully expressible in terms of existing types + `db`-crate-local types.
  Given concern 2's inclusion test (used by ≥2 crates), this is likely
  crate-local to `db` unless a third consumer appears — flagged, not decided.

## Nesting

Top-level, no parent, no children.

## Thoroughness level

`implementation-ready` for the existing crate + disciplinary charter; the
round-3 **surface-schema module is `requirements-only`** (named and scoped —
render + interaction description language — but its type shapes are undesigned;
a step-3 / Contract Harmonizer concern together with
`scaffold/contracts/surface-schema.md`). The rounds-4–5 **WS pub/sub envelope
module** and the round-8 **standardized event-type struct module** are
likewise **`requirements-only`** (see the planned-additions notes in the
Charter).

## Assigned design-depth

Sonnet (single pass, this file).

## Suggested fill-model

No Filler pass is warranted for `types` itself in this round — there is no
`NotImplemented` stub to fill; the code already exists and passes as its own
conformance baseline. When the Contract Harmonizer resolves OQ-3 and
`db-inference-init` in step 3, the resulting additive field/module changes
here are small and mechanical enough for a cheap/fast model to apply directly
against the harmonized contract's schema — no separate design-depth pass
needed for that follow-on edit, provided it stays additive (new
fields/modules, not restructuring existing ones).
