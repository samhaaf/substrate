# db

## Charter

`db` is substrate's single operational point of contact for everything
database. It owns local-stack lifecycle, migration authoring/apply/rollback,
seed + per-migration fake-data, edge-function deploy/activate with
pointer-flip rollback, sync validators, schema introspection, ad-hoc query,
the async outbox + validator audit, and forward-only promotion to prod. It
wraps the Supabase CLI, the Supabase Management API, Docker, and a native
Postgres client behind one `clap` noun-verb surface, over three drivers
(`supabase-cloud`, `supabase-local`, `sqlite`) selected through a single
`Driver` trait + per-driver `Capabilities` flags, so an incapable driver
(e.g. `sqlite` has no `edge`/`local_stack`/`pull`) degrades early with a typed
`NotImplemented` rather than a confusing deep failure. The control plane is
one `ops` schema (migration ledger + handler registry + outbox + audit); the
ledger is the sole source of truth. **This pass makes no implementation
changes to `db`** — status is "existing, as-is"; the only things new this
round are two confirmed consumer edges (below) and an aspirational note for
the future. `db` does NOT own: `store` (the per-node inference SQLite,
system-of-record for completions — a distinct, separately-owned database);
CCD's or Org's own domain logic (they use `db` as infrastructure, not the
other way around); or mesh's service-registry (a different, in-memory/gossip
store, not a `db`-managed schema).

Concretely, `db` is dual-crate like `gc`: `lib/db` (crate name
`substrate-db`) is the library — every real behavior lives here — and
`bin/db` is a thin `clap` dispatcher over it (one screen of glue, per the
repo's binary convention). This dual-role shape is exactly what makes the two
new consumer edges possible without inventing anything: `org` and `inference`
can depend on `substrate-db` as an ordinary Cargo library dependency (the
same way any component already depends on `substrate-types`), while
operators and CI keep using `bin/db`'s CLI for everything else. Neither new
consumer needs `db` to grow a network/daemon interface — it is not itself an
"app" that other apps must reach over the wire; it is a library an app can
compile in directly, same as `gc`'s embedded-lib half.

## Primary design concerns

- **Driver-capability degradation is the load-bearing abstraction.** Every
  new consumer (and the aspirational schema-agnostic future) hangs off the
  existing `Capabilities` flags + `NotImplemented`-on-unsupported-op pattern
  already in `driver/mod.rs` — this is not new design, just confirmation that
  the existing seam is the right one to build on.
- **Library-vs-CLI consumption is now a real fork, not hypothetical.** Before
  this round every consumer of `db` was a human/CI invoking `bin/db`. Now two
  in-process Rust consumers exist (`org`, `inference`). Nothing about `db`
  needs to change for this to work — `lib/db` already exports the
  functionality `bin/db` calls — but it is a design fact worth recording
  explicitly for the Contract Harmonizer and Skeleton Builder: the
  `db-control-plane` and `db-inference-init` edges are Cargo dependency edges
  (`lib/db` as a path dependency), not HTTP/CLI edges, and should be
  generated into the skeleton as such (a normal crate dependency, not a
  service-registry lookup).
- **`inference`'s bootstrap use case is a narrow slice of `db`'s surface.** A
  fresh mesh node only needs the `sqlite`-driver, ledger-only path (baseline
  `OPS_BASELINE_SQLITE` + `migration::apply`) — no edge functions, no local
  Supabase stack, no promote. This already fits the existing capability-gated
  driver design without modification; it's an integration/wiring question for
  `inference`'s own Filler (when does node bootstrap call into `db`, and does
  it call the library directly or shell out to the `db` binary), not a `db`
  design change.
- **`org`'s use case (self-restructuring knowledge graph) is the harder
  unknown, deliberately left to Org's own future design pass.** `db` supplies
  generic migration/query/handler machinery; it does not know or care that
  Org will model nodes/edges of a metacognitive graph on top of it. That
  graph schema is Org's to define (through `db`'s migration + handler
  tooling), not something `db` needs to anticipate.

## Relationships / edges

- `org` (consumer) via `db-control-plane` (see
  `scaffold/contracts/db-control-plane.md`) — Org's self-restructuring
  knowledge graph (metacognitive add/restructure-node operations) is
  implemented as `db` operations (migrations + queries against `db`-owned
  schema), rather than Org growing its own bespoke persistence layer. Same
  edge also covers the pre-existing "Org/game-demo" generic consumer use
  named in the overview (CLI + ad-hoc query access) — one edge, two use
  shapes.
- `inference` (consumer) via `db-inference-init` (see
  `scaffold/contracts/db-inference-init.md`) — **NEW this round.** A fresh
  `inference` node standing up on a new mesh node initializes its own
  database through `db`'s existing migration/local-stack machinery (sqlite
  driver, ledger-only baseline) instead of hand-rolling that bootstrap
  itself.
- (generic/CLI consumers — operators, CI, the promote pipeline) also via
  `db-control-plane` — unchanged from today.

No other edges touch `db` this round. `db` does not call out to any other
substrate component (mesh, ccd) — it is a leaf/foundation dependency,
consumed but not consuming, other than its own external deps (Supabase CLI,
Docker, Postgres).

## Nesting (if applicable)

Parent: none (top-level) | Children: none. (`lib/db` + `bin/db` is the
existing internal lib/bin split, not a scaffold nesting — see Charter.)

## Thoroughness level

`approach-sketched`.

The existing surface (everything under "Charter") is already implemented and
unchanged — effectively `implementation-ready` by default, since it's real
running code, not new design. The two new edges are sketched at the
integration/dependency-shape level (library-not-network, which driver
capability each consumer needs) but deliberately NOT specified down to exact
function signatures or schema — that's the Contract Harmonizer's job for
`db-inference-init`'s and `db-control-plane`'s schema sections, and Org's own
future design pass for the knowledge-graph schema itself.

## Assigned design-depth

Sonnet (this file). No Design Mesh warranted — the operator was explicit that
`db` itself needs no design rework this pass; the only work was writing the
charter against the real existing code and lightly specifying two already-
decided new edges.

## Suggested fill-model

No Filler work needed for `db` itself this pass — it is unchanged, already-
implemented code (skip or no-op its Fill step). The actual new integration
code lands in `inference`'s and `org`'s own Fill steps (adding a
`substrate-db` path dependency + calling into `migration`/`Db::open`, per the
edges above) — those Fillers should get whatever model their own component
design assigns; this file's contribution is just making sure they know `db`
is a normal library dependency, not a network call, which should keep their
fill effort low regardless of model tier.

## Query / virtualization layer — now IN-SCOPE DIRECTION (rounds 4–5 lock)

> **SUPERSEDED (2026-07-18, rounds 4–5): the "aspirational only" framing.**
> This section previously recorded the query-virtualization layer as
> future-only direction-of-travel, explicitly not scoped. The operator has
> upgraded it to an **in-scope direction** (upgrading the earlier "might be
> too much" aside):
> "Actually being able to query through the db crate and get a standardized
> interface to the different databases — SQLite files stored in the VFS, or
> Supabase projects, or Dockers or RDS instances. A virtualization layer on
> top of our databases makes sense, so all we have to do is make a decision
> like: oh, this only needs to be run in an SQLite."

In-scope direction (not yet designed): a **standardized query interface over
heterogeneous backends** — SQLite-files-in-the-VFS, Supabase projects, RDS
instances — so choosing a backend becomes a capability decision, not an API
change. The existing `Driver` trait + `Capabilities` flags (already handling
three backends) is the natural seed this layer grows from. Still true: "we're
building it one layer at a time as we need it" — direction is locked, the
design pass is not authorized here.

**Relationship to the new `stack` component (working framing, rounds 4–5 —
now explicitly THE operator's most-nebulous OPEN question, round-6):**
`db` = the control-plane / virtualization / query interface over databases;
`stack` = the runtime that *hosts* a database (the daemon around a
SQLite-file-in-the-VFS with SQL + Deno/TS handlers). See
`components/stack.md`.

**Round-6: the stack-vs-db boundary is recorded as the operator's
most-nebulous open question — do NOT force it.** The live tension, in brief
(full verbatim in `components/stack.md`): stack is "a pattern" (db-driven
triggers/handlers calling third parties) and much of it already lives in db —
which **really does deploy edge functions to Supabase today** — so there's a
strong case to fold stack into db (and run the knowledge graph through db,
pushing db hard); the alternative is keeping db boring as a utility under a
bigger eventual-consistency system in the mesh. The operator also coined
**VDB**: a virtualized-database service using `db` under the hood, databases
treated like services under mesh's restart/upgrade protocol, with the
confirmed copy/verify/switch + let-edge-functions-finish migration mechanics.
Recorded faithfully as OPEN with the VDB idea attached; nothing resolved
here.

**Round-7 (2026-07-19): VDB ELEVATED.** VDB is now the working name for the
**deploy-anywhere implementation of the stack pattern** — one abstract
runtime with **three adapter targets: local (the SQLite stack daemon),
Supabase, and AWS (RDS + Lambda)**. `db`'s existing **Capabilities-gated
driver architecture** (`supabase-cloud` / `supabase-local` / `sqlite`,
selected through the single `Driver` trait + per-driver `Capabilities`
flags) is **the natural seed of VDB's adapter matrix** — the same
incapable-driver-degrades-early seam, grown into deploy-target adapters.
Open questions attached (verbatim in `components/stack.md`): how the stack
pattern gets "baked into" VDB, and whether KG should be BUILT ON VDB (see
`components/kg.md`). The stack-vs-db boundary itself remains OPEN.
**Provenance note (round-6 cross-cutting; SCOPED round-7):** whatever shape
wins, provenance is FIRST-ORDER — every handler touch of data traced from
the very beginning (see `overview.md`'s standing principle) — and it is
**configured per-project/per-database, with VDB as its primary home**
(round-7 scoping; VFS carries only a lighter requirement).

**Flagged implication (not resolved here): the NO-DOCKER rule.** Rounds 4–5
locked a hard rule — no Docker locally, ever; containers only in the AWS
environment via an AWS adapter (see `components/stack.md`). `db`'s Charter
above still says it "wraps ... Docker" via the `supabase-local` driver — that
local-Docker path is now in tension with the rule and presumably deprecates
toward `stack`/native/cloud backends. Flagged for the operator/harmonizer,
not silently rewritten.
