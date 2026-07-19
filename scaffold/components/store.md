# store

## Charter
`store` is the SQLite system-of-record for one machine: the durable home of
completions, collections, models, result blobs, benchmark runs, and KV-cache
metadata (`lib/store`, modules `completions`, `collections`, `models`,
`results`, `benchmarks`, `kv_cache`). All access is serialized through a
`Mutex<Connection>`; the schema is embedded at compile time and applied with
append-only migrations on `Store::open`. It is the intra-node hub — nearly every
sibling reads/writes through it — and it carries a `StoreObserver` seam so
terminal state transitions can fire callbacks (the `inference` crate registers a
`PromiseRegistry` observer on it to fulfill in-process promises). Its boundary:
it owns per-node persistence and the observer seam; it does NOT own the
control-plane Postgres (`db`), cross-node state, or any business logic — callers
own that. Results here durably outlive on-disk weights, which is what makes
per-machine GC safe.

## Primary design concerns
- **Single-writer discipline.** The `Mutex<Connection>` is the concurrency
  contract; every mutation is serialized. The observer callbacks run inside that
  synchronous mutex context (hence the `try_lock`/`tokio::spawn` dance in
  `PromiseRegistry`) — a design concern the `store-access` contract must make
  explicit so Fillers of sibling crates don't block the writer from an observer.
- **Schema-ownership tension with `db` (FLAG — do not silently resolve).** The
  new `db-inference-init` edge says a fresh node initializes "its own database"
  through `db`. But `store` already owns and self-applies its embedded SQLite
  schema at `Store::open` with zero external dependency, which is exactly what
  lets a single-box node run standalone. Recommended default (flagged for the
  operator): `store` keeps its embedded SQLite migrations as the standalone path;
  `db-inference-init` covers control-plane REGISTRATION of the node plus any
  Postgres-backed per-node bootstrap, NOT a rewrite of store's local schema. This
  keeps single-box inference dependency-free while giving multi-node mesh a
  db-driven bootstrap. See open questions.
- **Benchmark-run schema gains pressure covariates (redesign impact).** The
  multidimensional kernel needs each benchmark run row to record the
  memory/CPU/GPU pressure observed at sample time (today `benchmark_runs` has
  `max_tokens`, `concurrency`, `prompt_tokens`, `tokens_per_second` only). This
  is an append-only column addition to the `benchmarks` module, surfaced through
  `store-access`.

## Relationships / edges
- store <-> {engine, scheduler, models, cache, telemetry, benchmark, api} via `store-access` (see scaffold/contracts/store-access.md)
- (observer seam) store -> inference (`PromiseRegistry`) — part of `store-access`; the terminal-transition callback path.

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
implementation-ready — the crate exists and is stable; the only new work is the
append-only pressure-covariate columns on `benchmark_runs` and clarifying the
observer-under-mutex contract. Both are mechanical against existing code.

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading `lib/store/src/*`.

## Suggested fill-model
implementation-ready + low complexity -> **cheap model OK**. The one caveat is
the schema-vs-`db` question above, which is a design/operator decision, not a
fill decision — resolve it before the Filler starts, then the fill is trivial.
