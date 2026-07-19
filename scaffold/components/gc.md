# gc

## Charter

`gc` is the filesystem garbage collector: it tracks **managed directories**
and the **entries** (files/dirs) inside them, enforces a per-directory TTL and
size budget, and evicts LRU/FIFO items via a pluggable `Reclaimer` strategy
when a budget is exceeded. It ships in two forms that share the same core
crate (`lib/gc`, `substrate_gc`): an **embedded library** instantiated
in-process by `inference` (and threaded down into `models`/`cache`/`engine`),
and a **standalone daemon** (`bin/gc`, Axum HTTP+WS on `:8430`) proxied by the
`gateway`. Both forms are kept **as-is** this pass — no redesign — this file
exists to confirm the boundary and surface a couple of things worth the
operator's attention, not to propose changes.

**Boundary — what `gc` does NOT own:**
- Cross-node coordination of any kind. GC is **strictly per-machine**; the
  mesh never queries or drives it directly. A node's disk pressure is only
  observable indirectly, through that node's own `/v1/models` inventory —
  never through a GC API call from another node.
- Deciding *what* counts as garbage or *when* to check — callers (models,
  cache, engine) decide what to register and when to call `make_room`/`lock`;
  `gc` only enforces the policy once told.
- Actually moving data to a slower tier. `OnFullAction::Migrate` /
  `MigrateReclaimer` exist in the type system but are an intentional stub
  (see Primary design concerns).

## Primary design concerns

`gc` earned its own top-level component (rather than clumping into
`inference`) because it has a genuinely separable, dual-surface contract that
several unrelated crates all depend on — not because any single piece of its
logic is algorithmically hard. The concerns worth documenting:

1. **Two independent `GcService` instances per node, on purpose, not
   accidentally.** Reading the real wiring (`lib/inference/src/lib.rs`
   step 4) confirms `inference` constructs its OWN `GcService` over
   `<inference data_dir>/gc.db`, registers `models/`, `backends/`, `kv_cache/`
   under it, and hands that one `Arc<GcService>` down to `models`, `cache`,
   and `engine`. Separately, `bin/gc` (`bin/gc/src/config.rs`,
   `default_local()`) opens its OWN `GcService` over `~/.substrate/gc.db` when
   run as the standalone `:8430` daemon. **These are two different SQLite
   databases with two different sets of managed directories on the same
   machine** — the daemon does not currently know about the dirs the embedded
   instance manages, and vice versa. This is fine as a design (each caller
   gets exactly the isolation it needs, and the daemon's own use case — a
   host-level GC surface any process on the box can register against over
   HTTP — is legitimately different from the in-process one) but it means
   the gateway's `gc-events`/`/api/gc/*` proxy today surfaces the
   **standalone daemon's** state only, NOT `inference`'s embedded
   models/kv-cache/backends directories. See the open question below —
   flagged, not resolved.
2. **The register→lock protocol is an unenforced convention, not an API
   guarantee.** The one real caller sequence worth calling out (from
   `lib/models/src/lib.rs`): `make_room(budget)` *before* a download reserves
   headroom, then `register_entry` + `lock(24h)` *after* the write completes
   protects the fresh file from the next sweep. `gc` itself does not
   atomically couple "write completed" to "now safe to sweep" — a caller that
   forgets the trailing `lock()` leaves a real (if narrow, sweep runs
   hourly) eviction race on a just-created file. Worth keeping in mind for any
   *new* v2 caller that starts registering directories with `gc` (e.g. a
   future `db` or `ccd` cache dir) — they should copy this
   make_room → write → register → lock pattern rather than reinvent it.
3. **Reclaimer is a clean strategy seam, but only one strategy is real.**
   `Reclaimer` (trait) / `DeleteReclaimer` / `MigrateReclaimer` in
   `lib/gc/src/reclaim.rs` is the right shape for "pluggable eviction
   action," and both bin and lib wiring currently pass `DeleteReclaimer`
   everywhere. `MigrateReclaimer::reclaim` is a real stub: it logs a warning
   and falls back to `DeleteReclaimer`. Per this task's brief, **not
   redesigning it** — flagging it so it isn't mistaken for a bug when it's
   read later. If a genuine migrate-elsewhere tier is ever wanted (e.g.
   spilling evicted model weights to a slower disk instead of deleting), it
   is a self-contained follow-up: implement `Reclaimer` for a new struct, no
   change to `GcService`/`eviction.rs`/the event schema required.
4. **Events are the one thing every consumer of `gc` actually shares.**
   `GcEvent` (`lib/gc/src/events.rs`) is broadcast identically whether `gc` is
   embedded or standalone; the daemon just puts an Axum WS front end
   (`bin/gc/src/api.rs::events_ws`) on the same `broadcast::Sender` the
   library exposes via `subscribe_events()`. This is why `gc-events` (the
   external, gateway-facing edge) and `gc-managed-dirs` (the internal,
   in-process edge) can stay two separate contract files carrying the *same*
   payload shape (`GcEvent`) without duplicating the schema: the Contract
   Harmonizer should point both contract files at one shared `GcEvent`
   definition in `types` rather than authoring it twice.

## Relationships / edges

- `gateway` via `gc-events` (see `scaffold/contracts/gc-events.md`) — the
  standalone `:8430` daemon's WS event stream + REST proxy (`/dirs`,
  `/entries`, `/ops/*`, `/events`) that the gateway aggregates for the
  dashboard's GC view and folds into its `used_bytes` stats sum
  (`bin/gateway/src/stats.rs`).
- `{models, cache, engine}` (all children of `inference`) via
  `gc-managed-dirs` (see `scaffold/contracts/gc-managed-dirs.md`) — the
  embedded-library edge: `inference` constructs one `Arc<GcService>` at
  startup and threads it down; `models`/`cache`/`engine` call
  `register_dir`/`register_entry`/`touch`/`lock`/`make_room` against it
  directly (in-process function calls, not HTTP).
- No edge to `mesh` — intentional (per Charter's boundary: GC is per-machine,
  never coordinated or queried cross-node). Confirmed against
  `scaffold/overview.md`'s own note: "a node's eviction is observed only
  indirectly via its `/v1/models` inventory."

No new contract edges identified beyond the two stubs already present under
`scaffold/contracts/` (`gc-events.md`, `gc-managed-dirs.md`); both already
correctly note they carry distinct payloads from the same `GcEvent`/API
surface, so no new stub file was added.

## Nesting

Parent: none | Children: none. `gc` is a flat top-level component — it has a
dual *role* (embedded lib + daemon) but that is not a parent/child
relationship in the scaffold sense (no enclave that talks only to `gc`); it
is the same crate's two entry points (`lib/gc` + `bin/gc`), matching
`overview.md`'s component tree where `gc` is listed with no children.

## Thoroughness level

`implementation-ready` — this is a working, tested V1 crate (`lib/gc` has
five passing unit tests covering touch/lock/expiry/LRU-order/move; `bin/gc`
is a complete Axum server with a full route table and error-code mapping)
being carried forward as-is. This file documents the real, already-built
design, grounded directly in the source (`lib/gc/src/{lib,config,reclaim,
events,eviction,store}.rs`, `bin/gc/src/{main,api,config}.rs`) and in every
real call site (`lib/{models,cache,engine,inference}/src/*.rs`), not a
speculative plan.

## Assigned design-depth

Sonnet (single pass) — the component is as-is; the job was source-grounded
confirmation and edge documentation, not novel design, so no Design Mesh
escalation was warranted.

## Suggested fill-model

**No Filler dispatch needed for `gc` itself.** The crate is already fully
implemented and tested; the Skeleton Builder should treat `lib/gc` + `bin/gc`
as pre-filled (skip generating `NotImplemented` stub bodies for it) and only
route it through a Filler if the Assembler's registry-backed wiring change
(see `overview.md`'s "single wiring seam") requires `bin/gc` to additionally
**register** its own `slug -> host:port` into `mesh.service-registry` at
startup — that specific, bounded addition is `requirements-only` today (it
isn't designed anywhere yet) and would warrant a cheap/fast model for a small,
well-scoped patch, not a strong model or Design Mesh.
