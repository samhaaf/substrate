# Contract: db-inference-init

## Parties
inference  ->  db

## What the edge carries
When an `inference` node stands up on a new mesh node, it initializes its own
database through `db`'s control plane rather than handling that bootstrap
itself (schema/migration apply for the node's local store setup, via `db`'s
existing local-stack/migration machinery). New edge, added this round per
operator decision — not previously in the contract graph.

This is a **library dependency edge, not a network/CLI edge**: `inference`
depends on `substrate-db` (the `lib/db` crate) directly and calls into it
(e.g. `Db::open` + `migration::apply`) at node-bootstrap time, rather than
shelling out to the `bin/db` binary or calling a `db` service over HTTP —
`db` has no daemon/network surface to call. The slice of `db` this edge
exercises is narrow: the `sqlite`-driver, ledger-only path (the
`OPS_BASELINE_SQLITE` baseline), since a fresh inference node has no local
Supabase stack, edge functions, or promote flow to worry about — those
`Capabilities` are already `false` for the `sqlite` driver and need no new
gating. Exact call sequence, error handling on a failed first-boot migration,
and whether this reuses or is distinct from `store`'s own schema bootstrap are
left to the Contract Harmonizer / `inference`'s own component design. Schema
and example data deferred to the Contract Harmonizer pass.

## Conformance requirement (preview, not authoritative — Harmonizer owns this)
At minimum: a fresh node with no prior `ops` schema, given a `sqlite`-driver
`db` config, can reach a migrated/ready state through this edge without any
manual intervention, and a second invocation against an already-migrated node
is a safe no-op (idempotent bootstrap).
