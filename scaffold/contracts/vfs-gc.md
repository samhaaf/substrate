# Contract: vfs-gc

## Parties

`vfs` (`bin/vfs`, per node — a `GcHandle::Remote` client)  ↔  `gc` daemon
(`bin/gc`, the node's **one** centralized store `~/.substrate/gc.db`, `:8430`),
over the local mesh relay (`Address::OnNode(gc, self)` — you never GC another
machine's disk). Both parties proposed: `vfs.md` concern 5 (the policy-driver
half) and `gc.md` (the authoritative command-vocabulary owner).

## Purpose

VFS drives **per-device storage enforcement** for vfs-managed directories over
gc's WS/REST surface (INTENT #28 verbatim: "so the virtual file system can update
it over WebSocket"). VFS owns the *distributed* decision (which blob goes where,
what the per-directory eviction/size policy is, which replica is safe to evict);
gc owns the *device-local mechanism* (the sweep, the LRU/FIFO reclaim, the `.gc`
config files, lock-with-expiry). Concretely VFS: registers/deregisters
vfs-managed dirs; sets their per-directory policy (`SetPolicy`, write-through to
`.gc/config.toml`); reserves headroom (`MakeRoom`) before a durable write; marks
fresh durable replicas **protected** (the never-evict-below-factor invariant);
and consumes gc's eviction events (`gc-events`) to keep `BlobPlacement.actual`
truthful.

**The `gc` rolled-in-vs-called-as-tool question is RESOLVED here:** gc stays a
**distinct per-node crate, called-as-tool** (both `vfs.md` concern 5 and `gc.md`
concern 7 independently recommend this) — see Reconciliation notes.

## Schema

The command vocabulary is **gc's** (`gc.md`) — the *same* `GcCommand`/`GcQuery`
set shared with `gc-managed-dirs`, differing only by transport (WS vs in-process)
and party. VFS is one caller of it, plus the `SetPolicy` path that is VFS's
headline use. Frame:

```rust
struct GcRequest  { corr_id: CorrId, op: GcCommandOrQuery }
struct GcResponse { corr_id: CorrId, result: Result<GcReply, WireError> }
enum   GcReply    { Ok, MakeRoom(MakeRoomResult), Sweep(SweepResult),
                    Entry(Option<EntryRow>), Entries(Vec<EntryRow>),
                    Dirs(Vec<DirRow>), UsedBytes(u64) }

enum GcCommand {                                     // gc.md owns this set
    RegisterDir     { path: PathStr, policy: Option<GcDirPolicy> },
    SetPolicy       { path: PathStr, policy: GcDirPolicy },     // VFS headline: write-through .gc/config.toml
    DeregisterDir   { path: PathStr },
    RegisterEntry   { path: PathStr, kind: EntryKind, recovery_hint: Option<String> }, // daemon computes size
    RegisterAndLock { path: PathStr, kind: EntryKind, ttl_secs: u64, recovery_hint: Option<String> },
    SetProtected    { path: PathStr, kind: ReplicaKind, protected: bool }, // MERGED (see notes) — durable-at-factor guard
    Touch           { path: PathStr },
    Lock            { path: PathStr, ttl_secs: u64 },
    Unlock          { path: PathStr },
    MakeRoom        { dir: PathStr, bytes_needed: u64 },
    MovePath        { from: PathStr, to: PathStr },
    Evict           { path: PathStr },
    Sweep,
    Reconcile       { dir: PathStr },
}
enum GcQuery { GetEntry{path:PathStr}, ListEntries{dir:Option<PathStr>}, ListDirs, DirUsedBytes{dir:PathStr} }

// gc's DEVICE-LOCAL enforcement policy (distinct from vfs's distributed VfsDirPolicy — see notes)
struct GcDirPolicy { max_size_bytes: u64, default_ttl_secs: u64,
                     eviction: EvictionPolicy, unit: UnitMode /* Self|Children */,
                     recursive: bool, on_full: OnFullAction /* Evict | Migrate(stub) */ }
enum EvictionPolicy { Lru, Fifo, LruUpdated, LruAccessed }   // gc's names win; VfsDirPolicy.eviction maps in
enum ReplicaKind    { Durable, Cache }                       // vfs vocab (from vfs-content)

struct MakeRoomResult { bytes_freed: u64 }
struct SweepResult    { expired_evicted: u64, budget_evicted: u64, bytes_freed: u64, errors: Vec<String> }
```

**Policy translation (VFS-side, load-bearing).** VFS holds its own
`VfsDirPolicy { max_bytes, eviction: Eviction, default_replication, tier_pref,
provenance }` (distributed policy, `vfs-content`/`vfs.md` concern 5). For the
local node's slice of a directory it derives a `GcDirPolicy`:
`max_size_bytes = the node's share of max_bytes`;
`eviction = Eviction::Fifo→Fifo | LeastRecentlyUpdated→LruUpdated |
LeastRecentlyAccessed→LruAccessed`; `on_full = Evict` (VFS handles S3 overflow
*itself* via `aws-vfs` before letting gc delete — `Migrate` reclaimer is the
future hand-back-to-vfs, `gc.md` concern 5). VFS then `SetProtected(protected:
true)` on each fresh **durable** replica so the sweep never drops it below factor.

## Error cases

Shared gc taxonomy (`gc.md`), mapped to WS `WireError` / HTTP status:

- `DirNotRegistered` (404) · `EntryNotFound` (404) · `EntryLocked` (409, `evict`
  on a locked entry) · `NonUtf8Path` / `PathHasNoParent` (422) ·
  `ReclaimFailed{path,err}` (per-entry in `SweepResult.errors`; a sweep never
  aborts on one failure).
- `DiskBudgetExceeded` (409) — **`MakeRoom` exhausted all *unlocked, unprotected*
  candidates before freeing enough.** The catchable error VFS **must** handle:
  it escalates to S3 overflow (`aws-vfs`) or another node, or surfaces pressure —
  never silently drops a durable replica.
- `UnsupportedPolicy` — VFS sent an `EvictionPolicy`/`OnFullAction` variant an
  older gc doesn't know; **rejected explicitly, never coerced** (a policy
  misunderstanding could delete data).
- `WouldViolateFactor` — **VFS-side guard**: VFS refuses to `SetProtected(false)`
  or `Evict` a *last* durable replica of a blob still at factor. gc mechanically
  honors `protected`; the invariant *decision* is VFS's.
- Transport: `LocalDaemonUnreachable` (gc down/restarting — VFS **degrades, does
  not block**: enforcement pauses, placement proceeds and re-drives on reconnect)
  and `NoLiveEndpoint` (no gc registered on this node — surfaced, not silently
  skipped).

Domains: `VfsError`, plus gc's WS/HTTP mapping.

## Version sensitivity

**LOW-to-MEDIUM.**
- The vfs↔gc hop is **node-local** (same box) — no cross-node skew on the wire
  itself. BUT VFS and gc **rev independently under rolling updates** (INTENT #66),
  so it is still the most version-exposed gc surface.
- **ADDITIVE-SAFE:** new `GcCommand`/`GcQuery` variants; new fields on
  `GcDirPolicy` (`serde(default)`); new `EvictionPolicy` variants **only if**
  older daemons reject-not-coerce (they do → `UnsupportedPolicy`). `EntryKind`,
  `ReplicaKind`, `OnFullAction` reserve `#[serde(other)]`.
- **BREAKING:** changing `MakeRoom`/`DiskBudgetExceeded` semantics, or the
  protected-entry contract — gated on a coordinated vfs+gc restart.
- `DiskBudgetExceeded` and `UnsupportedPolicy` are the two errors VFS **must**
  handle across versions.

## Reconciliation notes

Two proposers; several genuine deltas resolved:

1. **Command-vocabulary ownership — gc wins.** `vfs.md` sketched a thinner set
   (`RegisterManagedDir { path, budget_bytes, strategy }`, `SetEvictable`,
   `MakeRoom`). `gc.md` owns the full `GcCommand`/`GcQuery`/`GcDirPolicy` set (gc
   is the store owner and `.gc`-file author). **Resolution: gc's set is
   canonical**; vfs's thinner sketch is a strict subset that maps in
   (`RegisterManagedDir` → `RegisterDir` with a full `GcDirPolicy`). Losing
   position (vfs's `budget_bytes`+`strategy` shorthand) preserved here as the
   translation rule, not the wire.

2. **Eviction-enum naming — gc wins.** vfs proposed `GcStrategy { Fifo,
   LruAccessed, LruUpdated }` and, separately, a distributed
   `Eviction { Fifo, LeastRecentlyUpdated, LeastRecentlyAccessed }`. gc proposed
   `EvictionPolicy { Lru, Fifo, LruUpdated, LruAccessed }`. **Resolution: gc's
   `EvictionPolicy` is the wire/`.gc`-file enum** (gc persists it); vfs's
   long-form `Eviction` stays as vfs's *distributed* `VfsDirPolicy` field name and
   translates down. Rationale: the enum lives in gc's durable config file, so gc's
   naming is the source of truth; a second name on the wire would be two sources.

3. **Never-evict-below-factor — MERGED (both had a piece).** vfs proposed an
   explicit `SetEvictable { entry, kind, protected }` + a vfs-side
   `WouldViolateFactor`. gc proposed protecting fresh durables via `Lock(ttl)` +
   its `EntryLocked`/`DiskBudgetExceeded` taxonomy. A **pure TTL lock is
   insufficient** — a durable-at-factor replica needs *indefinite* protection
   (until replication elsewhere restores factor), which a TTL can only express via
   renewal churn, and INTENT #3's "never pin forever" is about *caller* locks, not
   a structural policy flag. **Resolution: adopt vfs's explicit protection as a
   `SetProtected { path, kind, protected }` command** (merged into gc's set,
   persisted to `.gc/{item}.toml`); gc **mechanically** excludes protected entries
   from eviction candidates; VFS **decides** protection and owns
   `WouldViolateFactor`. This splits mechanism (gc) from decision (vfs) cleanly.
   `Lock(ttl)` is retained for the *transient* fresh-write window
   (`make_room → write → register_and_lock`); `SetProtected` is the *durable*
   factor guard. Both mechanisms coexist; neither party's view was dropped.

4. **`MakeRoom` field naming — gc wins** (`{ dir, bytes_needed }` over vfs's
   `{ path, need_bytes }`) — trivial, gc owns the surface.

5. **Two structs named `DirPolicy` — disambiguated.** vfs's `DirPolicy`
   (distributed: replication/tier/provenance) and gc's `DirPolicy` (device-local:
   ttl/unit/on_full) are **different types**. Renamed vfs's → `VfsDirPolicy` and
   gc's → `GcDirPolicy` in this contract to prevent a harmonizer collision; the
   translation (2 above) is the seam between them. Flagged for the final
   `types` harmonization.

6. **Rolled-in-vs-called-as-tool — RESOLVED called-as-tool.** Both files
   recommend it. gc stays a distinct crate; folding it into vfs would force
   `inference` (which embeds `lib/gc` via `gc-managed-dirs`) to depend on vfs — a
   layering inversion. Skeleton-time latitude (both files): vfs's per-node leg
   *may* embed `lib/gc` in-process for vfs-managed dirs and re-expose the same
   surface — the `vfs-gc` shape is identical either way; called-as-tool is the
   default because it matches the operator's stated model (INTENT #28). Physical
   convergence of inference's embedded gc dirs with vfs's gc store into literally
   one store is **out of scope here** (batch-5 gc/inference concern).

7. **Deviation from the stub:** the stub left the rolled-in question OPEN; it is
   now decided (called-as-tool) with the losing branch preserved as
   skeleton-latitude above.

## Example data

macbook: VFS registers its models directory, reserves room, writes the weight,
protects the fresh durable replica; later a budget squeeze evicts a *cache*
replica first.

```jsonc
// 1) VFS sets the models-dir policy (write-through to .gc/config.toml)
GcRequest { corr_id: "c-8801", op: SetPolicy {
  path: "/Users/op/.substrate/vfs/models",
  policy: { max_size_bytes: 536870912000 /* 500 GiB */, default_ttl_secs: 0,
            eviction: "LruAccessed", unit: "Children", recursive: true, on_full: "Evict" } } }
GcResponse { corr_id: "c-8801", result: Ok(Ok) }

// 2) reserve headroom before the 2.4 GiB write
GcRequest { corr_id: "c-8802", op: MakeRoom {
  dir: "/Users/op/.substrate/vfs/models", bytes_needed: 2576980377 } }
GcResponse { corr_id: "c-8802", result: Ok(MakeRoom({ bytes_freed: 3221225472 })) }

// 3) register the fresh durable replica and mark it protected (factor guard)
GcRequest { corr_id: "c-8803", op: RegisterAndLock {
  path: "/Users/op/.substrate/vfs/models/qwen3-4b.gguf", kind: "File",
  ttl_secs: 3600, recovery_hint: "vfs://models/qwen3-4b.gguf" } }
GcRequest { corr_id: "c-8804", op: SetProtected {
  path: "/Users/op/.substrate/vfs/models/qwen3-4b.gguf", kind: "Durable", protected: true } }

// 4) budget squeeze: MakeRoom evicts the access-migrated CACHE copy first,
//    never the protected durable copy:
GcRequest { corr_id: "c-8890", op: MakeRoom {
  dir: "/Users/op/.substrate/vfs/models", bytes_needed: 2000000000 } }
GcResponse { corr_id: "c-8890",
  result: Err(DiskBudgetExceeded) }   // only protected durables remain unlocked
// -> VFS catches DiskBudgetExceeded, overflows a cold blob to S3 via aws-vfs.
```
