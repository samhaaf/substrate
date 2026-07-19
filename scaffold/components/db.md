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

## Aspirational / future direction (NOT scoped this pass — note only)

The operator's longer-term vision for `db` is a database-agnostic
schema/data-model layer: define the schema once and deploy it interchangeably
to SQLite, a self-managed Postgres, or Supabase; add trigger-based backend
logic that fires on field changes (in the spirit of Supabase edge functions,
but backend-agnostic); and possibly an ORM / query-virtualization layer on
top of that eventually. In the operator's own words: "I don't know what the
full future of `db` is, but it's bright — we're building it one layer at a
time as we need it." Nothing here is authorized or scheduled for
implementation now. It is recorded purely as direction-of-travel context: the
existing `Driver` trait + `Capabilities` flags (already handling three
backends today) is the natural seed this future layer would grow from, which
is one more reason to leave `db`'s current architecture alone rather than
rework it preemptively.
