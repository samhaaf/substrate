# gc

**Status:** existing (`lib/gc` + `bin/gc`), kept as-is. **Nesting:** top-level
(dual-role: embedded library AND `:8430` daemon).

A filesystem garbage collector: tracks managed dirs/entries, enforces TTL and
per-directory size budgets, and evicts LRU/FIFO items when a budget is exceeded
(`DeleteReclaimer` for v2 — cross-node `MigrateReclaimer` stays out of scope). It
is embedded in-process by models/cache/engine for disk-budget enforcement
(`gc-managed-dirs`) and also runs standalone, proxied by the gateway
(`gc-events`). GC is strictly per-machine — the mesh never coordinates it; a
node's eviction is observed only indirectly via its `/v1/models` inventory.
