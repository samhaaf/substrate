# db

**Status:** existing (`lib/db` + `bin/db`), kept as-is. **In scope: CONFIRMED**
(previously flagged as an open/orthogonal inclusion call in the decompose pass —
the operator has since confirmed `db` is in scope for this pass). **Nesting:**
top-level.

The database control plane — a single operational point of contact for everything
database: local-stack lifecycle, migration authoring/apply/rollback, seed +
fake-data, edge-function deploy/activate with pointer-flip rollback, sync
validators, schema introspection, ad-hoc query, the async outbox + validator
audit, and forward-only promotion to prod. Wraps the Supabase CLI + Management API
+ Docker + a native Postgres client behind a `clap` noun-verb surface, over three
drivers (supabase-cloud/supabase-local/sqlite). Distinct from `store` (per-node
inference SQLite). This is the "DB thing in a local stack" for the Org/game demo;
exposed via `db-control-plane`, a shaped-for edge of the maximalist Org consumer.

**Confirmed consumers (operator decisions, this round):**
- **`org` depends on `db`** for its own self-restructuring knowledge graph — the
  metacognitive processes that add/restructure nodes in Org's org-graph are
  themselves database operations, routed through `db`'s control plane rather than
  Org growing a bespoke persistence layer. (See `org.md` and its import list.)
- **`inference` depends on `db`**, not just `org`: a new `inference` node standing
  up on a fresh mesh node should use `db` to initialize its own database rather
  than hand-rolling that bootstrap itself. New contract edge added this round:
  `db-inference-init` (see `contracts/db-inference-init.md`).

**Aspirational / future direction (explicitly NOT scoped this pass, note only —
no implementation, no schema, not a design commitment):** the operator's
longer-term ambition for `db` is a database-agnostic schema/data-model layer —
define the schema once, deploy it to SQLite, a self-managed Postgres, or
Supabase interchangeably — plus trigger-based backend logic that fires on field
changes, and possibly an ORM / query-virtualization layer on top of that
eventually. Recorded here as direction-of-travel context for whoever designs
`db` in step 2; none of it is authorized or scheduled for this pass.
