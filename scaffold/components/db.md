# db

**Status:** existing (`lib/db` + `bin/db`), kept as-is. **Nesting:** top-level.

The database control plane — a single operational point of contact for everything
database: local-stack lifecycle, migration authoring/apply/rollback, seed +
fake-data, edge-function deploy/activate with pointer-flip rollback, sync
validators, schema introspection, ad-hoc query, the async outbox + validator
audit, and forward-only promotion to prod. Wraps the Supabase CLI + Management API
+ Docker + a native Postgres client behind a `clap` noun-verb surface, over three
drivers (supabase-cloud/supabase-local/sqlite). Distinct from `store` (per-node
inference SQLite). This is the "DB thing in a local stack" for the Org/game demo;
exposed via `db-control-plane`, a shaped-for edge of the maximalist Org consumer.
