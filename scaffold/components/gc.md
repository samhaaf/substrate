# gc

## Charter

`gc` is the filesystem garbage collector: it tracks **managed directories**
and the **entries** (files/dirs) inside them, enforces a per-directory TTL and
size budget, and evicts LRU/FIFO items via a pluggable `Reclaimer` strategy
when a budget is exceeded. It ships in two forms that share the same core
crate (`lib/gc`, `substrate_gc`): an **embedded library** instantiated
in-process by `inference` (and threaded down into `models`/`cache`/`engine`),
and a **per-node HTTP+WS surface** (`bin/gc`, Axum on `:8430`) whose events
mesh's observability plane aggregates.

> **SUPERSEDED (2026-07-18, round-3): the "as-is forever, standalone daemon"
> framing.** This file previously carried gc forward unchanged, framed as a
> per-machine standalone daemon + embedded lib pair with no new relationships.
> The round-3 lock changes gc's position in the system:
> 1. **GC becomes a per-node tool called by the `vfs`** (the new distributed
>    flat file system) to handle that device's storage — see
>    `components/vfs.md` and `contracts/vfs-gc.md`. Open question (operator's,
>    recorded not resolved): whether GC eventually gets **rolled up into the
>    VFS** entirely vs. staying a called-as-tool.
> 2. **The two-gc.db split is RESOLVED: ONE centralized per-node GC store.**
>    The embedded-instance-vs-daemon-instance duality (two SQLite databases,
>    two managed-dir sets on one machine — concern 1 below) collapses into a
>    single per-node store, **updatable over API/WebSocket**, with GC managing
>    **`.gc` config files in the managed directories** themselves.
> 3. The `gateway` proxy references below are stale: gateway merged into mesh
>    (see `components/mesh.md`); `gc-events` is now a `mesh <- gc` edge.

**Boundary — what `gc` does NOT own:**
- Cross-node coordination of any kind. GC **executes strictly per-machine**.
  (Round-3 amendment: its per-node store is now *updatable over API/WebSocket*
  — e.g. by the VFS enforcing directory policies — so "never driven remotely"
  no longer holds; what remains true is that GC never *coordinates* across
  nodes: any cross-node storage decision is VFS's/mesh's, GC just enforces on
  its own device.)
- Deciding *what* counts as garbage or *when* to check — callers (models,
  cache, engine, and now `vfs`) decide what to register and when to call
  `make_room`/`lock`; `gc` only enforces the policy once told.
- Actually moving data to a slower tier. `OnFullAction::Migrate` /
  `MigrateReclaimer` exist in the type system but are an intentional stub
  (see Primary design concerns).

## Primary design concerns

`gc` earned its own top-level component (rather than clumping into
`inference`) because it has a genuinely separable, dual-surface contract that
several unrelated crates all depend on — not because any single piece of its
logic is algorithmically hard. The concerns worth documenting:

1. **~~Two independent `GcService` instances per node, on purpose~~ —
   SUPERSEDED (2026-07-18): the operator resolved this split. There is now
   ONE centralized per-node GC store, updatable over API/WebSocket, with GC
   managing `.gc` config files in the managed directories. The embedded
   callers (models/cache/engine) and any remote caller (VFS, mesh dashboard
   ops) all converge on that one store; the "daemon can't see the embedded
   dirs" problem below disappears by construction.** Original analysis kept
   for context: reading the real wiring (`lib/inference/src/lib.rs`
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
   the old gateway's `gc-events`/`/api/gc/*` proxy surfaced the
   **standalone daemon's** state only, NOT `inference`'s embedded
   models/kv-cache/backends directories — exactly the blind spot the round-3
   single-store resolution above eliminates.
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
   external, mesh-facing edge — formerly gateway-facing) and `gc-managed-dirs` (the internal,
   in-process edge) can stay two separate contract files carrying the *same*
   payload shape (`GcEvent`) without duplicating the schema: the Contract
   Harmonizer should point both contract files at one shared `GcEvent`
   definition in `types` rather than authoring it twice.

## Relationships / edges

- `mesh` via `gc-events` (see `scaffold/contracts/gc-events.md`) — the per-node
  `:8430` WS event stream + REST surface (`/dirs`, `/entries`, `/ops/*`,
  `/events`) that mesh's observability plane (absorbed from gateway,
  2026-07-18) aggregates for the dashboard's GC view and stats rollups. This
  same API/WS surface is how the ONE centralized per-node GC store is updated
  remotely.
- `vfs` via `vfs-gc` (see `scaffold/contracts/vfs-gc.md`) — **NEW, round-3**:
  GC is the per-node tool VFS calls to enforce that device's directory
  policies (max size; FIFO / LRU-updated / LRU-accessed eviction). Open
  question: rolled into VFS vs. called-as-tool.
- `{models, cache, engine}` (all children of `inference`) via
  `gc-managed-dirs` (see `scaffold/contracts/gc-managed-dirs.md`) — the
  embedded-library edge: `inference` constructs one `Arc<GcService>` at
  startup and threads it down; `models`/`cache`/`engine` call
  `register_dir`/`register_entry`/`touch`/`lock`/`make_room` against it
  directly (in-process function calls, not HTTP). Round-3: these now converge
  on the single per-node store rather than a private `gc.db`.
- ~~No edge to `mesh`~~ — superseded (2026-07-18): mesh now aggregates
  `gc-events` directly (as gateway's absorber), and the per-node store is
  remotely updatable; GC still never coordinates cross-node itself.

## Nesting

Parent: none | Children: none. `gc` is a flat top-level component — it has a
dual *role* (embedded lib + daemon) but that is not a parent/child
relationship in the scaffold sense (no enclave that talks only to `gc`); it
is the same crate's two entry points (`lib/gc` + `bin/gc`), matching
`overview.md`'s component tree where `gc` is listed with no children.

## Thoroughness level

`implementation-ready` for the carried-forward core (`lib/gc` has five passing
unit tests covering touch/lock/expiry/LRU-order/move; `bin/gc` is a complete
Axum server with a full route table and error-code mapping), grounded directly
in the source (`lib/gc/src/{lib,config,reclaim,events,eviction,store}.rs`,
`bin/gc/src/{main,api,config}.rs`) and in every real call site
(`lib/{models,cache,engine,inference}/src/*.rs`).
**Round-3 deltas are `requirements-only`:** the single-per-node-store
consolidation, `.gc` config files in managed directories, remote update over
API/WS, and the `vfs-gc` tool relationship are locked as requirements but not
yet designed.

## Assigned design-depth

Sonnet (single pass) — the component is as-is; the job was source-grounded
confirmation and edge documentation, not novel design, so no Design Mesh
escalation was warranted.

## Suggested fill-model

**No Filler dispatch needed for `gc`'s carried-forward core.** The crate is
already implemented and tested; the Skeleton Builder should treat `lib/gc` +
`bin/gc` as pre-filled for that core. The round-3 deltas (single per-node
store, `.gc` config files, remote update over API/WS, the `vfs-gc` surface,
plus registering its own slug into `mesh.service-registry` at startup) are
`requirements-only` and need a design pass before fill — likely alongside the
`vfs` design, since the rolled-in-vs-called-as-tool question decides how much
of gc survives as a separate deployable at all.
