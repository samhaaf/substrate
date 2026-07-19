# gc

## Charter

`gc` is the **per-node filesystem garbage collector**: it tracks *managed
directories* and the *entries* (files/dirs) inside them, enforces a
per-directory TTL and size budget, lock-with-expiry (never pin forever), and
evicts LRU/FIFO items through a pluggable `Reclaimer` when a budget is
exceeded. It is the **per-node storage-enforcement tool called by `vfs`** (and
by `inference`'s `models`/`cache`/`engine`) — the boring device-local executor
that never coordinates across nodes: any cross-node placement decision is
VFS's/mesh's, gc just enforces on its own machine once told.

**Wave-2 refit (this pass) — the three round-3 deltas, now designed:**

1. **ONE centralized per-node store (INTENT #28).** The historic two-`gc.db`
   split-brain (an embedded `GcService` inside inference over
   `<inference>/gc.db` *plus* a separate `bin/gc` daemon over `~/.substrate/gc.db`
   — two databases, two managed-dir sets on one box) is **resolved**: the
   `bin/gc` daemon is the *sole owner* of the one per-node store
   (`~/.substrate/gc.db`); every other consumer becomes a **client of that
   daemon**, never a second constructor of `GcService`. mesh's zombie-killing
   (INTENT #57) guarantees exactly one gc daemon (= one writer) per node.
2. **Updatable over API/WS so VFS can drive it (INTENT #28, #53).** VFS mutates
   the one store remotely — set a directory's policy, register/deregister dirs,
   register/touch/lock entries, request `make_room`, move paths — all over the
   local-mesh-relayed WebSocket command surface.
3. **`.gc` config files maintained by the service (INTENT #28).** `.gc/config.toml`
   (per-dir policy) and `.gc/{item}.toml` (per-item overrides) become the
   **durable, co-located source of truth for policy**: when a directory's policy
   changes (VFS drives it) or a directory moves, the gc daemon *rewrites its own
   `.gc` files* in that location, so the store is rebuildable from disk after a
   `gc.db` loss.

**Boundary — what `gc` does NOT own:**
- **Cross-node coordination of any kind.** gc executes strictly per-machine.
  (Amendment to the old "never driven remotely": the store *is* now driven
  remotely by VFS — but gc still never *decides* cross-node anything.)
- **Deciding *what* is garbage or *when* to check.** Callers (`vfs`, and
  `models`/`cache`/`engine`) decide what to register and when to call
  `make_room`/`lock`; gc enforces the policy once told.
- **Actually moving data to a slower tier or another node.** `OnFullAction::Migrate`
  / `MigrateReclaimer` are an intentional stub (see concern 5). Cross-device
  relocation is **VFS's** job; the future `Migrate` reclaimer hands an evicted
  entry *back to VFS for relocation* rather than gc reaching across the network.
- **Provenance.** gc sits *below* the DB plane; it carries **no first-order
  provenance obligation** (INTENT #85/#92 scope provenance to VDB/db, lighter in
  VFS, nil here). Its `GcEvent` stream + `recovery_hint` logging is the closest
  thing to an audit trail, and that is all it is.

## Primary design concerns

gc earned its own top-level component not because any single piece of its logic
is algorithmically hard, but because it is a **genuinely separable dual-surface
tool that two unrelated consumer classes depend on** (VFS, and inference's
model/cache/backend management) and because the wave-2 centralization changes
*how it is reached* without changing what it does.

1. **The centralization is a transport change, not a semantics change — and
   that is the whole elegance.** The carried-forward core (`lib/gc`:
   `GcService` + `GcStore` SQLite + `.gc` config + `Reclaimer` + `GcEvent`
   broadcast, five passing tests) is *already correct and complete*. Wave-2 does
   **not** rewrite it. It (a) makes the daemon the single owner of one store,
   and (b) introduces a thin **`GcHandle` abstraction** so consumers hold *the
   same method surface* whether they reach gc in-process or over the daemon's
   WS. The command vocabulary (`register_dir`, `register_entry`, `touch`,
   `lock`, `unlock`, `make_room`, `move_path`, `set_policy`, `sweep`, `evict`,
   `deregister`, `query`, `list_*`) is **identical** across the embedded and
   remote paths — this is why `gc-managed-dirs` (in-process) and `vfs-gc`
   (remote) carry the *same* request vocabulary and `gc-managed-dirs`/`gc-events`
   carry the *same* `GcEvent` vocabulary. The Contract Harmonizer points these
   at one shared set of `types` structs, never authoring the shapes twice.

2. **`GcHandle`: the single migration seam that preserves standalone
   operation (INTENT #44 layering vs. "don't break single-box").** Consumers
   today hold `Arc<GcService>` (in-process function calls). Wave-2 replaces that
   field type with a two-variant handle:
   ```rust
   enum GcHandle {
       Embedded(Arc<GcService>),   // in-process: standalone / tests — today's behaviour, unchanged
       Remote(GcClient),           // WS to the local gc daemon via mesh-client — the normal mesh world
   }
   ```
   Both expose the same async command methods (the embedded variant wraps the
   synchronous `GcService` calls; they are fast and bounded). Consequences that
   make this the load-bearing design decision:
   - **`Remote` guarantees the single store** — every command lands on the one
     daemon owning `~/.substrate/gc.db`. VFS, models, cache, and engine all
     converge there. Split-brain gone by construction.
   - **`Embedded` guarantees standalone still works** — a `bin/inference`
     started with *no mesh present* (dev box, test, cold Pi before gc boots)
     constructs `GcHandle::Embedded` and behaves exactly as today. This is the
     concrete answer to "without breaking single-box standalone operation": the
     standalone path IS the embedded path, byte-for-byte the current code.
   - **The mode is selected once**, at `InferenceService::start`: if the local
     gc daemon is resolvable via mesh, use `Remote`; else fall back to
     `Embedded` (with a warning that this node is unmanaged-by-mesh). No call
     site in `models`/`cache`/`engine` changes beyond the field type.

3. **Who computes size, and why locality makes remote registration natural.**
   `register_entry` walks the path to compute on-disk size. In `Remote` mode the
   **daemon** does that walk — correct, because gc is per-node and the daemon is
   on the *same physical machine* as the caller (single-port locality: a caller
   reaches only its LOCAL mesh, which relays only to the LOCAL gc). The WS
   `register_entry` therefore ships only `{path, kind, recovery_hint}` and the
   daemon fills in `size_bytes` — exactly what the existing HTTP
   `RegisterEntryRequest` already does. There is no cross-node filesystem access
   anywhere in gc; a remote caller managing a path it cannot see is a
   *programming error*, not a supported mode.

4. **`.gc` files as the durable policy source of truth; the store as runtime
   state (new).** To satisfy "gc maintains its own `.gc` config files" *and*
   make the one store crash-recoverable, split authority cleanly:
   - **`.gc/config.toml` + `.gc/{item}.toml` = policy, durable, co-located,
     human-editable.** A remote `set_policy(dir, policy)` **writes the `.gc`
     file first, then updates the store row** (write-through). A `move_path`
     carries the `.gc` folder with the directory (it lives inside it) and, for a
     per-item override, renames `.gc/{old}.toml` → `.gc/{new}.toml`.
   - **`gc.db` = runtime state (entries, touch counts, locks, sizes, sweep
     timestamps), rebuildable.** On daemon boot, gc loads its dir list from the
     store, **re-reads each `.gc/config.toml`** (disk wins for policy), and a
     `reconcile(dir)` op can re-scan a directory to rebuild entry rows from disk
     if the store was lost. This is the crash-safety story the single-store
     requirement needs and is a genuine wave-2 addition (today `register_dir`
     reads `.gc` once and never writes policy back on remote change).

5. **`Reclaimer` is a clean strategy seam, but only `Delete` is real — and
   that is the correct boundary with VFS tiering (INTENT #27/#48).**
   `DeleteReclaimer` is wired everywhere; `MigrateReclaimer` is an intentional
   stub that logs and falls back to delete. **Not redesigning it this pass** —
   but this is where the "GC grows move-capability vs. a VFS layer uses GC"
   open question (INTENT #27) resolves: gc does **not** grow cross-node move
   capability. When cold-storage tiering (Pi + external drives) arrives, **VFS**
   — which alone knows mesh topology — orchestrates relocation and the `Migrate`
   reclaimer becomes "surrender this entry to VFS for placement elsewhere," a
   self-contained follow-up (`impl Reclaimer for VfsHandoffReclaimer`, no change
   to `GcService`/`eviction.rs`/the event schema).

6. **The register→lock convention is still an unenforced two-step, now over a
   network hop (carried forward, slightly widened).** The safe caller sequence
   is `make_room(budget)` *before* a write (reserve headroom) then
   `register_entry` + `lock(ttl)` *after* the write (protect the fresh file from
   the next sweep). In `Remote` mode these are two WS round-trips, so the
   register→lock window is marginally wider than the in-process case — but sweep
   respects locks and runs hourly, so the race is the same *class* as today.
   Wave-2 closes it cleanly with an **additive atomic `register_and_lock`
   command** (one round-trip, daemon-side atomic under the store mutex) that new
   callers (VFS, and a future `db`/`ccd` cache dir) should prefer. No debt: the
   two-step still works; the combined op is a strict improvement.

7. **gc is a per-node singleton, not a fleet — an addressing subtlety for the
   registry pair round.** A caller *always* wants *its own node's* gc (you never
   garbage-collect another machine's disk). So gc registers under slug `gc`
   per-node and is addressed as the **local instance** — `Address::OnNode(gc,
   self)`, which under single-port locality is just "resolve `gc` on my local
   daemon." It is emphatically **not** an `Anywhere(gc)` fleet like `inference`.
   Flagged for `service-lookup`'s pair round so gc is not modelled as a
   load-balanced fleet.

## Relationships / edges

- **`vfs` via `vfs-gc`** (see `scaffold/contracts/vfs-gc.md`) — **the wave-2
  headline edge.** VFS is a `GcHandle::Remote` client of the node's gc daemon:
  it registers VFS-managed directories, sets their per-directory policies (max
  size; FIFO / LRU-updated / LRU-accessed eviction), and drives
  `make_room`/`move_path`/`deregister` as VFS placement changes. **My
  recommendation on the open rolled-in-vs-called-as-tool question: gc stays
  called-as-tool (a distinct per-node crate), NOT rolled into VFS** — see
  "Controversial decisions" below and the friction note; the pair round with
  the (concurrently-designed) vfs designer reconciles.
- **`{models, cache, engine}` (children of `inference`) via `gc-managed-dirs`**
  (see `scaffold/contracts/gc-managed-dirs.md`) — the in-process edge, now
  expressed through `GcHandle` (`Embedded` standalone, `Remote` under mesh)
  rather than a raw `Arc<GcService>`. Same commands (`make_room` → write →
  `register_entry` → `lock`), same `GcEvent` payloads. Round-3: converges on the
  single per-node store instead of a private `gc.db`.
- **`mesh` via `gc-events`** (see `scaffold/contracts/gc-events.md`) — the
  per-node observability edge: the `GcEvent` WS stream + REST read surface
  (`/dirs`, `/entries`, `/entries/get`, dir `used_bytes`) that mesh's
  dashboard-serving plane aggregates for the dashboard's GC view and stat
  rollups. gc-events is **read/observe**; the mutating remote-update path rides
  `vfs-gc`'s command vocabulary (same daemon, distinct contract intent).
- **`mesh` via the cross-cutting service protocols (consumed via
  `mesh-client`, not authored here):**
  - `service-lookup` — gc registers slug `gc` → `{scheme, 127.0.0.1, port
    (8430 preferred, dynamic per INTENT #36), health_path:/health}`, per-node.
  - `restart-protocol` — gc participates in the LOCKED 4-level ladder; reports
    `Busy` while a sweep/reclaim is mid-flight (interrupting a delete can leave
    store↔disk inconsistent), `Idle` otherwise; on the L3 save window it
    finishes the in-flight reclaim and checkpoints the WAL before yielding.
  - `pubsub-protocol` — `GcEvent`s are published as typed events over the
    standard envelope; mesh relays them to the dashboard fan-out.
  - `surface-schema` — gc publishes its boring surface schema (dirs/entries
    tables, sweep/make-room actions) for schema-driven dashboard rendering.
- Imports `substrate-types` for the shared command/event/id vocabulary — a
  shared-lib dependency, **not** a contract edge (locked rounds 4–5).

## Nesting

Parent: none | Children: none. gc is a flat top-level component with a dual
*deployment form* (embeddable `lib/gc` + per-node `bin/gc` daemon) — not a
parent/child relationship. Matches `overview.md`'s tree (gc, no children).

## Thoroughness level

**implementation-ready.** The carried-forward core is already implemented and
tested (`lib/gc/src/{lib,config,reclaim,events,eviction,store}.rs`,
`bin/gc/src/{main,api,config}.rs`; five passing unit tests). The wave-2 deltas
are designed to implementation depth here: the single-store ownership model, the
`GcHandle` `Embedded`/`Remote` seam and its migration path, `.gc`-as-durable-policy
with write-through `set_policy`, boot reconciliation, per-node registration +
restart participation, and the atomic `register_and_lock` improvement. The one
genuinely deferred piece is `MigrateReclaimer` (intentional stub, resolves with
VFS tiering later — not this pass).

## Assigned design-depth

Opus, single Component-Designer pass. Grounded on the full `lib/gc` + `bin/gc`
source and every real call site (`lib/{inference,models,cache,engine}/src/*.rs`),
the batch-1/2 designs (`mesh-client.md` for the client-half registration/restart/
pubsub protocols, `service-registry.md`, `supervision.md`, `pubsub-relay.md`,
`types.md`), the concurrently-designed `vfs.md` (still requirements-only —
hence the explicit own-recommendation on the rolled-in question), and INTENT
#27/#28/#44/#48/#53/#57/#66/#76/#77/#92.

## Suggested fill-model

**Carried-forward core: no Filler dispatch** — `lib/gc` + `bin/gc` are pre-filled
and tested; the Skeleton Builder treats them as done. **Wave-2 deltas:
implementation-ready + low–moderate complexity → cheap/fast model OK**, with one
sequencing constraint: fill gc's `Remote`/`GcClient` path **after** the Contract
Harmonizer freezes the `pubsub-protocol` envelope and the shared gc command/event
structs in `types` (the `GcClient` serializes those). The `GcHandle` refactor,
`.gc` write-through, and boot reconciliation are mechanical against this design.

---

## Proposed contracts (wave 2)

gc owns three contract pairs (`vfs-gc`, `gc-managed-dirs`, `gc-events`). The
cross-cutting protocols gc merely *participates in* (`service-lookup`,
`restart-protocol`, `pubsub-protocol`, `surface-schema`) are authored by
`mesh-client`/`service-registry`/`supervision`/`pubsub-relay`; gc's participation
is noted above, not re-authored here. All structs live in `substrate-types`
vocabulary; Rust-flavoured pseudocode. **The command vocabulary below is ONE set
shared by `gc-managed-dirs` and `vfs-gc`** (differing only by transport/party);
**the `GcEvent` vocabulary is ONE set shared by `gc-managed-dirs` and
`gc-events`** — the Harmonizer authors each shape once.

### Shared command vocabulary (the `GcApi` surface)

**Purpose.** The single set of enforcement commands gc accepts, whether
in-process (`Embedded`) or over WS (`Remote`). Mirrors the existing `GcService`
method surface + the wave-2 additions (`set_policy`, `register_and_lock`,
`reconcile`).

**Sketch.**
```rust
// --- policy & config (all persisted write-through to `.gc/*.toml`) ---
struct DirPolicy { max_size_bytes: u64, default_ttl_secs: u64,
                   eviction: EvictionPolicy /* Lru|Fifo|LruUpdated|LruAccessed */,
                   unit: UnitMode /* Self|Children */, recursive: bool,
                   on_full: OnFullAction /* Evict|Migrate(stub) */ }
struct ItemConfig { ttl_secs: Option<u64>, recovery_hint: Option<String> }

// --- commands (request half) ---
enum GcCommand {
    RegisterDir      { path: PathStr, policy: Option<DirPolicy> }, // None = load/keep .gc/config.toml
    SetPolicy        { path: PathStr, policy: DirPolicy },         // NEW: write-through .gc/config.toml + store
    DeregisterDir    { path: PathStr },
    RegisterEntry    { path: PathStr, kind: EntryKind, recovery_hint: Option<String> }, // daemon computes size
    RegisterAndLock  { path: PathStr, kind: EntryKind, ttl_secs: u64, recovery_hint: Option<String> }, // NEW: atomic
    Touch            { path: PathStr },
    Lock             { path: PathStr, ttl_secs: u64 },
    Unlock           { path: PathStr },
    MakeRoom         { dir: PathStr, bytes_needed: u64 },
    MovePath         { from: PathStr, to: PathStr },   // renames data + .gc + store rows
    Evict            { path: PathStr },
    Sweep,                                              // manual sweep trigger
    Reconcile        { dir: PathStr },                  // NEW: rebuild entry rows from disk after store loss
}
enum GcQuery { GetEntry{path:PathStr}, ListEntries{dir:Option<PathStr>}, ListDirs, DirUsedBytes{dir:PathStr} }

struct MakeRoomResult { bytes_freed: u64 }
struct SweepResult { expired_evicted: u64, budget_evicted: u64, bytes_freed: u64, errors: Vec<String> }
```
**Error cases (shared taxonomy — the transport maps these to WS `WireError` or
HTTP status; existing daemon mapping preserved).**
- `DirNotRegistered` (→ 404) — command names a dir gc doesn't manage.
- `EntryNotFound` (→ 404).
- `DiskBudgetExceeded` (→ 409) — `make_room` exhausted all *unlocked* candidates
  before freeing enough; **the catchable error a caller must handle** (VFS must
  choose to overflow to `aws`/another node, or surface pressure).
- `EntryLocked` (→ 409) — `evict` on a currently-locked entry.
- `NonUtf8Path`, `PathHasNoParent` (→ 422) — malformed registration.
- `ReclaimFailed{path,err}` — surfaced per-entry in `SweepResult.errors`; a
  sweep never aborts on one failure.
**Version-sensitivity.** `GcCommand`/`GcQuery`/`DirPolicy` are **additive-only**;
a newer VFS may send an `EvictionPolicy` variant (`LruUpdated`/`LruAccessed` are
the VFS-requested additions to today's `Lru`/`Fifo`) or an `on_full` an older
daemon doesn't know — unknown enum variants must be **rejected explicitly with
`UnsupportedPolicy`, never silently coerced** (a policy misunderstanding could
delete data). Envelope carries `protocol_version` (rides `pubsub-protocol`).

### `gc-managed-dirs` (embedded, `{models,cache,engine}` → gc)

**Purpose.** In-process disk-budget enforcement for inference's local model
weights, llama backends, and KV/prefix cache. Expressed via `GcHandle` so the
same call sites work `Embedded` (standalone) or `Remote` (under mesh).

**Sketch.**
```rust
trait GcApi {  // implemented by both GcHandle::Embedded and GcHandle::Remote
    async fn register_dir(&self, path:&Path, policy:Option<DirPolicy>) -> Result<()>;
    async fn register_entry(&self, path:&Path, kind:EntryKind, hint:Option<String>) -> Result<()>;
    async fn register_and_lock(&self, path:&Path, kind:EntryKind, ttl:u64, hint:Option<String>) -> Result<()>;
    async fn touch(&self, path:&str) -> Result<()>;
    async fn lock(&self, path:&str, ttl_secs:u64) -> Result<()>;
    async fn make_room(&self, dir:&str, bytes_needed:u64) -> Result<u64>;
    // ... unlock / move_path / evict / query / list_* mirror GcCommand/GcQuery
}
```
**Error cases.** The shared taxonomy above; the canonical caller flow
(`lib/models`) is `make_room` → write → `register_and_lock` (preferred) — a
caller that forgets the trailing lock leaves the narrow register→sweep race
(concern 6). **Version-sensitivity.** In `Embedded` mode: none (compiled in). In
`Remote` mode: inherits the shared command version rules.

### `vfs-gc` (remote WS, `vfs` → gc daemon, via local mesh)

**Purpose.** VFS drives per-node storage enforcement over WS: the SAME
`GcCommand`/`GcQuery` vocabulary, plus the **policy-management path** that is
VFS's headline use (`SetPolicy` — VFS owns the per-directory eviction/size
policy; gc persists it write-through to `.gc/config.toml`). VFS is always a
`GcHandle::Remote` client (it runs only under mesh).

**Sketch.**
```rust
// request frame (rides pubsub-protocol's generic Request to Address::OnNode(gc, self))
struct GcRequest  { corr_id: CorrId, op: GcCommandOrQuery }
struct GcResponse { corr_id: CorrId, result: Result<GcReply, WireError> }
enum   GcReply    { Ok, MakeRoom(MakeRoomResult), Sweep(SweepResult),
                    Entry(Option<EntryRow>), Entries(Vec<EntryRow>), Dirs(Vec<DirRow>), UsedBytes(u64) }
```
**Error cases.** Shared taxonomy; plus transport-layer `LocalDaemonUnreachable`
(gc daemon down / restarting — VFS must degrade, not block: enforcement pauses,
placement proceeds and re-drives on reconnect) and `NoLiveEndpoint` (no gc
registered on this node — a misconfigured node; VFS surfaces it, does not
silently skip enforcement). **Version-sensitivity.** The most version-exposed gc
surface — VFS and gc rev independently under rolling updates (INTENT #66).
`DiskBudgetExceeded` and `UnsupportedPolicy` are the two errors VFS *must*
handle; policy enums are additive-only and unknown variants are rejected, never
coerced.

### `gc-events` (mesh ← gc daemon)

**Purpose.** The per-node observability surface mesh's dashboard-serving plane
aggregates: the `GcEvent` typed stream + a REST/WS read surface. Read-only;
mutation is `vfs-gc`.

**Sketch.**
```rust
enum EvictionReason { TtlExpired, BudgetPressure, Forced }
enum GcEvent {  // published over pubsub-protocol; ONE definition, also used by gc-managed-dirs
    DirRegistered   { root: String, max_size_bytes: u64, default_ttl_secs: u64 },
    PolicyUpdated   { root: String, policy: DirPolicy },        // NEW: emitted on SetPolicy
    EntryRegistered { path: String, kind: String, size_bytes: u64, recovery_hint: Option<String> },
    EntryTouched    { path: String, touch_count: u64 },
    EntryLocked     { path: String, lock_expires_at: i64 },
    EntryUnlocked   { path: String },
    EntryEvicting   { path: String, reason: EvictionReason },
    EntryEvicted    { path: String, bytes_freed: u64, recovery_hint: Option<String> },
    EntryEvictionFailed { path: String, error: String },
    BudgetExceeded  { dir: String, current_bytes: u64, max_bytes: u64 },
    MakeRoomCompleted { dir: String, bytes_freed: u64 },
    SweepCompleted  { expired_evicted: u64, budget_evicted: u64, bytes_freed: u64 },
    PathMoved       { from: String, to: String },
}
// read surface: GcQuery (GetEntry/ListEntries/ListDirs/DirUsedBytes) + GET /events (WS stream) + /health
```
**Error cases.** Read-only; a lagged WS subscriber drops events (bounded
broadcast, existing behaviour) — the dashboard reconciles from a fresh
`ListDirs`/`ListEntries` snapshot on reconnect (never assumes a continuous
stream). **Version-sensitivity.** `GcEvent` is **additive-only** and the
dashboard renders defensively from the surface schema — a new variant
(`PolicyUpdated` is the wave-2 addition) is ignored by an older dashboard, never
fatal. This is exactly why `gc-events` and `gc-managed-dirs` point at one shared
`GcEvent` in `types`.
