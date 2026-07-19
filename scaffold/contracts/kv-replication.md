# Contract: kv-replication

## Parties
mesh daemon (`replicated-kv` on node **macbook**) ↔ mesh daemon
(`replicated-kv` on node **pi**) — every pair of mesh daemons in the tailnet,
peer-to-peer. Rides mesh-core Ring-0 `PeerTransport` frames (`kind = "kv"`)
inside the daemon↔daemon `:3649` link (single-port locality holds for the
replication plane too). Authored from `replicated-kv.md` (authoritative — the
Fable L2 design that owns the store); `service-registry` participates only as
a keyspace tenant, contributing value shapes, not wire.

**Supersedes `registry-replication`.** This is the exact generalization
wave2-plan §5.5 / flag 5 anticipated once `replicated-kv` was extracted:
registry entries are no longer replicated by a bespoke registry protocol —
they are rows in the `registry/` keyspace and ride this one protocol like
every other tenant. `scaffold/contracts/registry-replication.md` is a
superseded-tombstone pointing here.

## Purpose
The ONE replication protocol for all kernel state — service-registry
records, lock claims, queue/trigger definitions, supervision version/boot
records, cron schedules, distributed config. It carries three mechanisms
layered fast→paranoid (`replicated-kv.md` concern 5):

1. **Eager push** — every locally-originated write is fanned immediately to
   currently-connected peers; steady-state propagation ≈ one hop.
2. **Cursor delta sync** — on (re)connect and every `sync_interval` (30s
   default) a peer pulls everything the other applied since a per-pair
   `applied_seq` cursor; bidirectional by construction, transitive
   (A↔B↔C converges without A–C ever connecting — the walk-along-pi case).
3. **Full-state digest compare** — every `full_sync_interval` (10min
   default) and on cursor loss, peers exchange per-keyspace
   `(count, xor-of-version-hashes)`; on mismatch they exchange full
   `key → Version` maps and fetch exactly the losing entries. The
   guarantee-carrying layer — cursors optimize, the digest never lies.

## Schema
The replicated record and its total order (structs land in `types::kv`;
values are opaque bytes end-to-end — KV never deserializes a tenant payload):

```rust
pub struct Version {          // the entire consistency model is one rule:
    pub ts_ms: i64,           //   larger Version wins, everywhere, deterministically
    pub ctr:   u16,           // per-node intra-ms counter
    pub node:  NodeId,        // writing node — deterministic tiebreak
}
// total order: (ts_ms, ctr, node) lexicographic; FROZEN FOREVER (see Version sensitivity).
// ts_ms comes from a hybrid-logical-clock ratchet, not the raw wall clock.

pub struct Key   { pub keyspace: String, pub path: String } // path is "/"-segmented
pub struct Lease { pub holder: String, pub expires_at_ms: i64 }

pub struct Entry {
    pub key:          Key,
    pub value:        Option<Bytes>,   // None == tombstone; opaque to KV
    pub content_type: Option<String>,  // advisory ("application/json" default)
    pub version:      Version,
    pub lease:        Option<Lease>,   // first-class TTL; expiry is a read-side filter
    pub wall_ms:      i64,             // raw wall clock at write — display only, NEVER ordering
}
```

The wire frames (every exchange opens with `kv_proto: u16`):

```rust
// ── fast path: new local writes fanned to connected peers ──────────────
struct KvPush    { kv_proto: u16, entries: Vec<Entry>, ack_requested: bool }
struct KvPushAck { applied: Vec<Key>, sender_seq_seen: u64 }
//   ack_requested = true only for Durability::AllReachable writes (the fenced
//   write locks builds acquisition on); ack ties out every snapshot peer.

// ── cursor delta: reconnect + periodic ─────────────────────────────────
struct KvDeltaReq  { since_seq: u64 }               // "everything you applied after seq S"
struct KvDeltaPage { from_seq: u64, to_seq: u64, entries: Vec<Entry>, done: bool }

// ── digest safety net ──────────────────────────────────────────────────
struct KvDigestReq {}
struct KvDigest    { keyspaces: Vec<(String /*keyspace*/, u64 /*count*/, u64 /*xor-version-hash*/)> }
struct KvKeymapReq { keyspace: String }
struct KvKeymap    { keyspace: String, keys: Vec<(String /*path*/, Version)> }
struct KvFetch     { keys: Vec<Key> }               // request exactly the losing entries

// ── fresh join (node offline > tombstone_ttl) ──────────────────────────
struct KvSnapshotReq {}                             // full-state pull, paged as KvDeltaPage from seq 0
```

**Merge rule (the whole algorithm, applied on every received `Entry`):** if
`incoming.version > local.version` (or no local entry) → apply atomically,
emit an in-process `WatchEvent { cause: Merge }`; else discard. Idempotent,
commutative, associative — apply order never matters, re-delivery is harmless.

## Error cases
- `CursorUnknown { fallback: FullSync }` — the peer lost/reset its `seq`
  bookkeeping (e.g. after its own fresh join). Respond by digest+keymap,
  **never** by guessing a cursor.
- `KeyspaceUnknown` — an older peer lacks a newer keyspace. Tolerated:
  entries for unknown keyspaces are still applied (values are opaque, keyspace
  config is local), so a mixed-version fleet converges.
- `PageTooLarge` — frames are bounded; the sender must re-page a `KvDeltaPage`.
- `PersistentDigestMismatch` — the same keyspace mismatches after two full
  keymap reconciles running → a corruption alarm published as
  `mesh.kv.integrity_alarm`; never silently retried forever.
- Transport-level failures (`PeerUnreachable`) are mesh-core's, not this
  contract's — this protocol assumes an established `PeerTransport` link.

Note the `Durability::AllReachable` semantics that `KvPushAck` carries: the
caller (locks) is acked only when every snapshot-reachable peer confirmed
applying; a peer that fails or vanishes between snapshot and ack yields
`KvError::PartialReplication { confirmed, missing }` to that local caller —
the write STANDS locally and on confirmed peers (LWW has no rollback). CAP
honesty: "all reachable" promises nothing about unreachable nodes.

## Version sensitivity
- **`kv_proto: u16` rides the first frame of every exchange.** Incompatible
  majors → the pair falls back to digest-only sync at the older side's proto.
  A `kv_proto` major bump is a **breaking** change and couples to
  `restart-protocol` as a Compatibility-priority fleet restart.
- **Additive-safe (no proto bump):** new `#[serde(default)]` fields on
  `Entry`; new keyspaces (unknown-keyspace tolerance above); new tenant value
  schemas (values are opaque bytes — consumer schema churn NEVER touches this
  protocol, the payload-agnostic decoupling that lets mixed-version fleets
  replicate). New `mesh.kv.*` observability topics are additive.
- **Breaking (must bump `kv_proto`):** any change to `Version`, the merge
  rule, cursor/`applied_seq` semantics, or the digest hash function. The
  **`Version` total order `(ts_ms, ctr, node)` is frozen forever** —
  reordering it is a data-corrupting change, the one thing this contract may
  never do. The guarantees table (`replicated-kv.md` concern 6) is part of the
  contract text, not commentary.

## Reconciliation notes
- **Only one party proposed this edge, and there is no disagreement.**
  `replicated-kv.md` (authoritative) authored `kv-replication` in full;
  `service-registry.md` independently *recommended collapsing*
  `registry-replication` into it ("the registry contributes only the keyspace
  name and value schema; the replication wire is `replicated-kv`'s"). Both
  sides converge, so this file adopts `replicated-kv`'s wire verbatim and
  records the registry as a pure keyspace tenant.
- **`registry-replication` is superseded, not merged-with-loss.** Its wave-1
  content (slug→endpoint anti-entropy, LWW conflict handling, tombstones,
  reconnect sync) is fully subsumed: registry rows live at
  `registry/instance/<slug>/<node>`, wrapped in the `Entry` shape above; the
  registry's three flagged requirements — tombstones with a GC horizon,
  keyspace-prefix `watch` carrying old→new value, per-node LWW clock stamping
  on `put` — are all satisfied by `replicated-kv` (concerns 3/7/8) and need no
  registry-specific protocol. `scaffold/contracts/registry-replication.md`
  becomes a superseded-tombstone pointer (harmonizer edit; flagged here).
- **Deviation from the old stub.** The `registry-replication` stub scoped the
  edge to "slug→endpoint entries." This contract generalizes it to ALL kernel
  keyspaces — the intended §5.5 generalization, not a scope change snuck in.
- **Out of this cluster / delegated:** the S3 cold-durability leg
  (`KvSnapshotExport`/`KvSnapshotReceipt`, encrypted snapshots to S3 through
  `aws`) that `replicated-kv.md` also proposed is the **`aws-mesh`** pair, not
  `kv-replication` — authored by the aws/mesh cluster. Its unresolved
  friction (genesis encryption key can't live in `secrets` because `secrets`
  rides this replication) is carried there, not here.

## Example data
The example world: nodes **macbook** and **pi**; `pi` runs the always-on
inference leg for model **qwen3-4b** in project **demo**.

**1. Eager push.** `pi` registers its inference instance (a `service-lookup`
`Register` → a `kv.put` on `registry/instance/inference/pi`). `replicated-kv`
on `pi` immediately pushes to the connected peer `macbook`:

```jsonc
// KvPush  pi -> macbook   (kind="kv" PeerTransport frame)
{
  "kv_proto": 1,
  "ack_requested": false,          // Durability::Local (registration is not fenced)
  "entries": [{
    "key": { "keyspace": "registry", "path": "instance/inference/pi" },
    "value": "<opaque bytes: bincode(ServiceRecord{ slug:'inference', node:'pi',
              endpoint:{http,127.0.0.1,8081,'/health'}, addressing:NodeScoped, ... })>",
    "content_type": "application/x-substrate-registry",
    "version": { "ts_ms": 1752969600123, "ctr": 0, "node": "pi" },
    "lease":   { "holder": "inference@pi", "expires_at_ms": 1752969630123 },
    "wall_ms": 1752969600123
  }]
}
// KvPushAck  macbook -> pi
{ "applied": [{ "keyspace": "registry", "path": "instance/inference/pi" }],
  "sender_seq_seen": 4210 }
```

**2. Delta sync after reconnect.** `pi` (the walk-along node) drops off the
tailnet, then rejoins. `macbook` requests everything since its stored cursor
for `pi`:

```jsonc
// KvDeltaReq  macbook -> pi        (macbook last applied pi's seq 4188)
{ "since_seq": 4188 }
// KvDeltaPage pi -> macbook
{ "from_seq": 4189, "to_seq": 4211, "done": true,
  "entries": [ /* the qwen3-4b heartbeat renewals + a demo cron-schedule write
                  pi accepted while split, each a full Entry with its Version */ ] }
```

The renewed registry lease (a fresh `Version` with a larger `ts_ms`)
LWW-beats the stale copy `macbook` held; `macbook`'s watchers fire
`cause: Merge { from_node: "pi" }`, so the registry cache re-resolves
`inference` on `pi` with the new lease. If `pi` had been offline longer than
`tombstone_ttl` (30d), it would instead have issued `KvSnapshotReq` and done a
fresh join — dropping its stale-epoch writes rather than resurrecting them.

**3. Digest safety net.** Every 10min the pair exchanges
`KvDigest{ keyspaces: [("registry", 7, 0x9f3c…), ("cron", 2, 0x0a11…), …] }`;
matching hashes end the exchange in one round trip — boring on purpose.
