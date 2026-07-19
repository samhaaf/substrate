# Contract: gc-events

## Parties

`gc` daemon (`bin/gc`, per node, `:8430`)  →  `mesh` (the
`dashboard-serving` observability plane). *(Was `gateway ← gc`; gateway merged
into mesh, INTENT #45.)* Sole authoritative proposer: `gc.md`.

## Purpose

The per-node **observability** surface mesh aggregates for the dashboard's GC
view and stat rollups: gc's typed `GcEvent` stream (over `pubsub-protocol`) plus
a REST/WS **read** surface (`ListDirs`, `ListEntries`, `GetEntry`,
`DirUsedBytes`, `/health`). This edge is **read/observe only** — the *mutating*
remote-update path (register dirs, set policy, make_room) rides `vfs-gc`'s command
vocabulary against the **same daemon**; distinct contract *intent*, one process.

## Schema

`GcEvent` is **one definition in `types`**, also emitted through
`gc-managed-dirs`'s `GcApi`; authored once, referenced by both.

```rust
enum EvictionReason { TtlExpired, BudgetPressure, Forced }

enum GcEvent {                                   // published over pubsub-protocol (topic gc/<node>/events)
    DirRegistered   { root: String, max_size_bytes: u64, default_ttl_secs: u64 },
    PolicyUpdated   { root: String, policy: GcDirPolicy },      // NEW (wave-2): emitted on SetPolicy
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

// read surface (REST/WS, existing daemon routes):
//   GET  /dirs                 -> Vec<DirRow>       (== GcQuery::ListDirs)
//   GET  /entries?dir=<path>   -> Vec<EntryRow>     (== GcQuery::ListEntries)
//   GET  /entries/get?path=<p> -> Option<EntryRow>  (== GcQuery::GetEntry)
//   GET  /dirs/used?dir=<path> -> u64               (== GcQuery::DirUsedBytes)
//   GET  /events               -> WS stream of Envelope<GcEvent>
//   GET  /health               -> Ok
struct DirRow   { root: String, max_size_bytes: u64, used_bytes: u64, entry_count: u64, eviction: EvictionPolicy }
struct EntryRow { path: String, kind: String, size_bytes: u64, touch_count: u64,
                  locked: bool, lock_expires_at: Option<i64>, protected: bool }
```

`dashboard-serving` maintains **one gc-WS subscription per node** (no leaks/
doubles) and renders gc's panel from its `surface-schema`, not bespoke UI.

## Error cases

Read-only, so the failure modes are subscription-shaped, not command-shaped:

- A **lagged WS subscriber drops events** (bounded broadcast, existing behaviour).
  The dashboard **never assumes a continuous stream** — on (re)connect it
  reconciles from a fresh `ListDirs` + `ListEntries` snapshot, then follows the
  live stream. This makes a dropped event non-fatal by design.
- `/entries?dir=<unknown>` → empty list (not an error) — an unmanaged dir simply
  has no rows.
- Daemon down → the WS/REST endpoint is unreachable; `dashboard-serving` marks the
  node's GC panel stale and retries — no mesh-wide impact (gc is per-node).

## Version sensitivity

**LOW (observability, defensive rendering).**
- **ADDITIVE-SAFE:** `GcEvent` is **additive-only**; a new variant (`PolicyUpdated`
  is the wave-2 addition) is **ignored by an older dashboard, never fatal** — the
  dashboard renders from the `surface-schema`, so unknown variants degrade to
  "not shown," not a crash. New `DirRow`/`EntryRow` fields are `serde(default)`.
- **BREAKING:** removing/renaming a `GcEvent` variant or a read-surface route —
  gated on a dashboard+gc coordinated bump (rare; observability changes are almost
  always additive).
- This additive-only discipline is exactly why `gc-events` and `gc-managed-dirs`
  point at **one** shared `GcEvent` in `types` — a new variant surfaces in both
  the in-process stream and the dashboard feed with zero divergence.

## Reconciliation notes

- **Single proposer** (`gc.md`); no competing shape, no losing position.
- **Read/observe vs mutate split (carried from the stub, now precise):** the stub
  noted "this API/WS surface is also how the one centralized store is updated
  remotely." That mutation path is **re-homed to `vfs-gc`** (command intent);
  `gc-events` is the pure read/observe half. Same daemon, same `:8430`, two
  contract intents — deliberately, so the dashboard never carries write authority.
- **`GcEvent` shared with `gc-managed-dirs`:** one `types` definition; this file
  and `gc-managed-dirs.md` reference it rather than re-declaring — the harmonizer
  authors the shape once.
- **Deviation from the stub:** schema was deferred; it is now pinned (the full
  `GcEvent` enum + read rows + the reconcile-on-reconnect discipline), and
  `PolicyUpdated` is added for the wave-2 `SetPolicy` write-through path.

## Example data

macbook's gc daemon streams events as VFS drives it (see `vfs-gc` example);
`dashboard-serving` renders the GC panel from them.

```jsonc
// live stream on topic gc/macbook/events (Envelope<GcEvent> over pubsub-protocol)
{ kind: "PolicyUpdated", root: "/Users/op/.substrate/vfs/models",
  policy: { max_size_bytes: 536870912000, default_ttl_secs: 0,
            eviction: "LruAccessed", unit: "Children", recursive: true, on_full: "Evict" } }
{ kind: "EntryRegistered", path: "/Users/op/.substrate/vfs/models/qwen3-4b.gguf",
  kind_str: "File", size_bytes: 2576980377, recovery_hint: "vfs://models/qwen3-4b.gguf" }
{ kind: "EntryLocked", path: "/Users/op/.substrate/vfs/models/qwen3-4b.gguf",
  lock_expires_at: 1721394000 }
{ kind: "BudgetExceeded", dir: "/Users/op/.substrate/vfs/models",
  current_bytes: 539000000000, max_bytes: 536870912000 }
{ kind: "EntryEvicted", path: "/Users/op/.substrate/vfs/models/old-cache.kv",
  bytes_freed: 4194304, recovery_hint: null }

// dashboard reconnect after a drop: snapshot before following the stream
GET /dirs  -> [ { root: "/Users/op/.substrate/vfs/models", max_size_bytes: 536870912000,
                  used_bytes: 534000000000, entry_count: 3, eviction: "LruAccessed" } ]
```
