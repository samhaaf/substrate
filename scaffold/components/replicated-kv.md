# replicated-kv

**Status:** NEW (wave 2, batch 2 — the Fable seat carrying the wave's
consistency risk). **Nesting:** internal lib of mesh (module
`lib/mesh::kv`), Ring 2 in mesh-core's internal-layering architecture.
**Consumes:** mesh-core Ring-0 seams only (`LocalStore`, `PeerTransport`) +
`types`. **Consumed by:** Ring 3 (`service-registry`, `locks`, `queues`,
`supervision`) and Ring 4 (`cron`), via the `KvHandle`/`KeyspaceHandle` seam
specified here. Designed WITHIN mesh-core.md's ring model; no deviations from
it are proposed.

## Charter

`replicated-kv` is THE one boring replicated primitive of Mind OS (INTENT
#54): an eventually-consistent, per-key **LWW-register** key-value store,
replicated across every mesh daemon in the tailnet, on which
`service-registry` entries, `locks` semaphore state, `queues` metadata,
`supervision` version/boot-order records, `cron` schedules, distributed
config, and the anticipated builds/tools registry (INTENT #30/#33) all ride.
It owns: the version model (timestamp-wins, operator-blessed naive
resolution, INTENT #32), first-class **leases/TTL**, **tombstones** and their
GC, the **anti-entropy sync protocol** between daemons over the
tailscale-discovered peer set, **partition/merge semantics** with an explicit
guarantees-and-non-guarantees contract, a **watch/subscribe** change feed
(reliable in-process; lossy observability mirror into `pubsub-relay`), and
**persistence** to mesh's own local SQLite file via Ring-0's `LocalStore`.
Its boundary — what it does NOT own: it is **never a standalone service** and
has **no external wire surface of its own** — every out-of-process consumer
reaches KV state through an owning Ring-3 lib's contract (`service-lookup`,
`locks-api`, `queues-api`, `cron-api`) or the observability plane; it holds
**no application data** (that is `db`/`vdb`), **no queue items** (durable
at-least-once delivery is not LWW-shaped — that is `queues`' own store), **no
lock policy** (twin detection, thresholds, the partition-merge error type are
`locks`', built ON the pattern this file prescribes), **no files** (VFS is
L3, above it — see concern 9), and no CRDTs beyond the LWW register — no
vector clocks, no Merkle trees, no quorums, no Raft, no multi-key
transactions, ever (escalation, not scope).

## Primary design concerns

This is the hardest correctness surface in the OS precisely because everyone
above assumes it is boring. The design's job is to make the boringness true:
one small state machine, one total order, one sync loop — and an honest,
explicit statement of what that buys and what it cannot.

### 1. The version model — timestamp-wins, made safe with an HLC ratchet

Every entry carries a `Version` and the ENTIRE consistency model is one rule:
**the larger `Version` wins, everywhere, deterministically.**

```rust
// proposed to types (module kv.rs — wire-crossing, guardrail-4 discipline)
pub struct Version {
    pub ts_ms: i64,    // hybrid-logical-clock milliseconds (see below)
    pub ctr: u16,      // per-node intra-millisecond counter
    pub node: NodeId,  // writing node — the deterministic tiebreak
}
// total order: (ts_ms, ctr, node) lexicographic. Two Versions are never
// equal across distinct writes: a node never reuses (ts_ms, ctr).
```

The timestamp source is a **hybrid logical clock ratchet**, not the raw wall
clock: `next_ts = max(wall_clock_ms, last_issued_ts, max_ts_ever_observed_in_
a_merged_entry)`, with `ctr` incrementing within one millisecond and the
ratchet state persisted (crash-safe: on boot, resume from
`max(wall, persisted_max + 1)`). This is still exactly "timestamp-based
last-write-wins" as blessed (INTENT #32) — but it closes the two ugliest
naive-clock artifacts for ~30 lines of code:

- a node with a **slow clock** can no longer silently lose every write it
  makes *after having seen* newer data (once it merges an entry with a higher
  `ts_ms`, its own next write ratchets above it — causality for anything that
  transited the mesh is preserved);
- a node with a **backwards clock jump** (NTP correction) can never issue a
  version below one it already issued.

What the ratchet does NOT fix (stated honestly): two **concurrent** writes on
partitioned nodes with skewed clocks resolve by skewed timestamps — "last"
means largest-timestamp, not causally-last. That is the operator-accepted
naive core ("pretty much everything's going to be driven from my laptop") and
it is why `locks` must never encode contended state as one shared key
(concern 6).

### 2. The entry model — value, lease, tombstone in ONE record shape

One record shape serves all consumers; there are no special cases in the
merge path:

```rust
pub struct Entry {
    pub key: Key,                    // { keyspace: String, path: String } — path is "/"-segmented
    pub value: Option<Bytes>,        // None == tombstone. Opaque to KV; consumers serde their own types
    pub content_type: Option<String>,// advisory tag ("application/json" default)
    pub version: Version,
    pub lease: Option<Lease>,        // first-class TTL (concern 3)
    pub wall_ms: i64,                // raw wall clock at write — display/debug only, NEVER ordering
}
pub struct Lease { pub holder: String, pub expires_at_ms: i64 } // holder is an opaque owner token
```

- **Values are opaque bytes.** KV never deserializes a value — the same
  payload-agnostic decoupling `pubsub-relay` uses (its concern 2), and for
  the same reason: mixed-version fleets. Registry entries, lock claims, and
  supervision records evolve without this lib knowing.
- **A tombstone is just an entry with `value: None`** and a newer `Version`.
  Delete is a write; the merge rule needs no delete-specific branch.
- **Merge rule (the whole algorithm):** on receiving a remote `Entry`, if
  `incoming.version > local.version` (or no local entry) → apply atomically,
  emit a watch event with `cause: Merge`; else discard. Idempotent,
  commutative, associative — apply order never matters, re-delivery is
  harmless.

### 3. Leases/TTL — first-class, read-filtered, locally sweeped

The charter assigns leases here rather than to each consumer because registry
and locks would otherwise each build the same expiry machinery (INTENT #54:
"if it makes sense to do them the same, do them the same"). Semantics:

- `put(..., ttl: Some(d))` stamps `lease.expires_at_ms = now + d`;
  `renew(key, holder, extend)` is a fresh write (new `Version`, same value)
  by the holder — renewal therefore replicates and LWW-beats any concurrent
  stale sweep, by construction.
- **Expiry is a READ-SIDE FILTER, never a replicated event.** `get`/`scan`
  return `None` for an entry whose lease is expired against the local clock
  (an `include_expired` flag exists for diagnostics). Expiry is thus
  evaluated per-node — under clock skew two nodes may disagree for ~skew
  duration; consumers must tolerate that window (registry's resolve already
  does: worst case a just-dead endpoint is briefly returned and the caller's
  request fails).
- **Expiry events are local-per-node notifications, not a cluster event.**
  A keyspace opting into `expiry_events` gets a timer-wheel-driven
  `WatchEvent { cause: LeaseExpired }` on EACH node when an entry crosses
  expiry there (fired on boot-scan too, for leases that expired while the
  daemon was down). Consumers act on their locally-owned slice (the registry
  hides the entry everywhere; zombie-killing is executed only by the node
  that owns the process). No election, no dedup — deliberately.
- **Sweep (cosmetic, not correctness):** an entry expired longer than the
  keyspace's `sweep_grace` (default 24h) is tombstoned so scans stay clean.
  The entry's `version.node` sweeps its own; any node sweeps (with jitter)
  when the origin node has been offline past grace. Racing sweeps are
  harmless — same tombstone effect under LWW.

### 4. Persistence — mesh's own SQLite file, below the storage plane

Storage is the local SQLite handle Ring-0 opens and hands down
(`LocalStore`, consumed by replicated-kv ONLY — mesh-core seam list). WAL
mode; schema:

```sql
CREATE TABLE kv_entries (
  keyspace TEXT NOT NULL, path TEXT NOT NULL,
  value BLOB,                 -- NULL = tombstone
  content_type TEXT,
  ver_ts INTEGER NOT NULL, ver_ctr INTEGER NOT NULL, ver_node TEXT NOT NULL,
  lease_holder TEXT, lease_expires_ms INTEGER,
  wall_ms INTEGER NOT NULL,
  applied_seq INTEGER NOT NULL,        -- local monotonic apply-sequence (concern 5)
  PRIMARY KEY (keyspace, path)
);
CREATE INDEX kv_by_seq ON kv_entries(applied_seq);
CREATE TABLE kv_meta (k TEXT PRIMARY KEY, v BLOB);
  -- rows: hlc_ratchet_state, self_node_id, last_online_ms,
  --       peer_cursor:<node_id> (per-peer sync watermark, concern 5)
```

Local write path: one SQLite transaction (entry + `applied_seq` + ratchet
state) → fsync → notify local watchers → replicate. A local write is durable
before it is acked; a crash mid-anything re-applies idempotently.

**Location and layering (the honest flag).** The file lives in mesh-core's
data dir (`mesh.toml` `data_dir`, platform default) on the **plain OS
filesystem** — explicitly NOT in the VFS, NOT a VDB database, NOT a
gc-managed directory. See concern 9.

### 5. Anti-entropy sync — eager push + cursor delta + full-digest safety net

Replication rides Ring-0 `PeerTransport` frames (`kind = "kv"`) over the
daemon↔daemon `:3649` peer links — single-port locality holds for the
replication plane too. The peer set is `PeerTransport.peers()`, fed by
`network-topology`/`tailscale-query`. Three mechanisms, layered from fast to
paranoid:

1. **Eager push (the fast path).** Every locally-originated write is pushed
   immediately to all currently-connected peers (fire-and-forget for
   `Durability::Local`; acked for `Durability::AllReachable`, concern 6).
   Steady-state propagation latency ≈ one hop.
2. **Cursor delta sync (the reconnect path).** Every applied entry (local OR
   merged-remote) gets this node's next `applied_seq`. Each peer pair keeps a
   cursor: "I have applied everything you had applied up to your seq S." On
   (re)connect and every `sync_interval` (default 30s), A asks B for
   everything since `cursor(B)`, pages flow, cursor advances. Because the
   delta includes entries B merged FROM OTHERS, propagation is transitive —
   A↔B↔C converges even when A–C never connect (non-transitive tailnet
   reachability, the walk-along-Pi case). **Reconnect sync is bidirectional
   by construction:** both sides pull the other's delta (INTENT #71's
   "offline nodes sync bidirectionally on reconnect" holds for the whole
   substrate, not just locks).
3. **Full-state digest compare (the safety net, every `full_sync_interval`,
   default 10min, and on cursor loss).** Exchange per-keyspace
   `(entry_count, xor-of-version-hashes)`; on mismatch, exchange full
   `key → Version` maps and request exactly the losing entries. This is the
   "periodic full-state anti-entropy" of the original registry design, kept
   as the guarantee-carrying layer — cursors are an optimization that may
   lose bookkeeping; the digest never lies. At personal-mesh scale (well
   under 10^5 keys) a full map exchange is a few hundred KB — boring on
   purpose, no Merkle trees.

Wire messages are the `kv-replication` contract (scaffold/contracts/kv-replication.md).

### 6. Partition behavior + merge semantics — the explicit contract

**During a partition:** every node keeps serving reads and accepting writes
against its local store (AP, always). Leases held by nodes on the far side
expire locally and their entries go read-invisible here — correct, since the
far side is unreachable anyway. Nothing blocks, nothing errors, except
`Durability::AllReachable` writes which honestly report what "reachable"
meant (below).

**At merge (reconnect):** cursor delta sync runs both ways; per key, LWW
picks one winner globally and deterministically; losing writes are **silently
overwritten** — that is the blessed naive contract. Watchers on every node
see the merge burst as `cause: Merge { from_node }` events carrying old and
new versions, so layers that CAN'T accept silent loss detect it:

- **The prescribed pattern for conflict-intolerant consumers (`locks`):
  never encode contended state as one shared key.** Each acquisition attempt
  writes its own key (`locks/<slug>/<uuid>`), so LWW never merges two
  different holders into one silently-resolved register; the `locks` lib
  detects partition twins by prefix-scanning `locks/<slug>/` after merge and
  raises its REQUIRED catchable threshold-exceeded error (INTENT #71/#84).
  replicated-kv's obligations to that pattern: cheap prefix `scan`, prefix
  `watch`, and the sync write mode — nothing more. The full twin/merge
  policy is `locks`' design, not this file's.

**`Durability::AllReachable` — the fenced write `locks` builds acquisition
on** ("knowledge distributes to ALL reachable nodes BEFORE the client is
confirmed holding," INTENT #71). Semantics: snapshot the live-connected
`PeerTransport` peer set at call time; apply locally; eager-push to each;
ack the caller only when every snapshot peer confirmed applying. Failures
return `KvError::PartialReplication { confirmed, missing }` — the write
STANDS locally and on confirmed peers (it cannot be unwritten; LWW has no
rollback), and the caller (locks) decides policy. CAP honesty, stated
plainly: "all reachable" promises nothing about unreachable nodes, and a
node may become unreachable between snapshot and ack — physics, not a bug
(INTENT #84).

**The guarantees table (the contract every Ring-3 lib designs against):**

| Guaranteed | Not guaranteed |
|---|---|
| Read-your-own-writes on the local node | Any cross-node read freshness bound (staleness = partition length + sync interval) |
| Per-key monotonicity per node (a node's visible version never regresses) | Cross-key ordering (write k1 then k2 may be seen k2-first remotely) |
| Deterministic global convergence: same entries ⇒ same winner on every node | Causal consistency across nodes beyond the HLC ratchet |
| Durability of acked writes across daemon crash (SQLite WAL) | Lost-update protection on a shared key (concurrent writes: one silently wins) |
| Single-key atomicity (an entry is applied whole or not at all) | Multi-key atomicity / transactions |
| `AllReachable`: applied on every snapshot-reachable peer before ack | Anything about nodes unreachable at snapshot time |
| Watch: at-least-latest per key (never miss the final state) | Watch delivery of every intermediate state (coalescing) |
| Tombstones shadow older writes for `tombstone_ttl` | Deletion durability against a node offline > `tombstone_ttl` (concern 7) |

### 7. Tombstone GC and the fresh-join rule

Tombstones are retained `tombstone_ttl` (per-keyspace, default **30 days**)
then purged locally. The classic hazard: a node offline longer than
`tombstone_ttl` rejoins still holding a live entry whose tombstone everyone
else purged — merge would resurrect the deleted key. The boring, honest fix:

- each daemon persists `last_online_ms` (updated on any successful peer
  sync);
- on boot/reconnect, if `now − last_online_ms > min(tombstone_ttl over
  keyspaces)`, the node performs a **fresh join**: it renames its local
  store aside (kept for operator forensics), pulls a full snapshot from
  peers, and only then resumes writing. Its own writes from the stale epoch
  are deliberately dropped — after ≥30 days offline, they are presumed
  garbage; the operator can recover them from the set-aside file by hand.
- a fresh join emits a `mesh.kv.fresh_join` event (observability +
  dashboard).

At personal-mesh scale tombstone storage is trivial (thousands of rows), so
the window can be generous; 30 days is a config default, flagged for the
operator, not a constant.

### 8. Watch/subscribe — reliable in-process, lossy mirror to pub/sub

Two tiers, deliberately different guarantees:

- **In-process watch (the correctness tier).** `KeyspaceHandle::watch(prefix)
  -> WatchStream` for Ring-3/4 siblings. Guarantee: **at-least-latest with
  coalescing** — implemented as a per-watcher dirty-key set + notify, so a
  slow consumer may skip intermediate values but always observes the final
  state of every changed key, and the stream never drops a key on the floor.
  This is exactly sufficient for the state-shaped consumers (registry cache
  invalidation, locks twin scan, supervision reacting to version records) —
  none of them need every intermediate. Event shape:

```rust
pub struct WatchEvent {
    pub key: Key,
    pub old: Option<(Version, Option<Bytes>)>,
    pub new: (Version, Option<Bytes>),       // value None == tombstone
    pub cause: WatchCause,
}
pub enum WatchCause { LocalPut, LocalDelete, Merge { from_node: NodeId },
                      LeaseExpired, Sweep }
```

- **Pub/sub mirror (the observability tier).** A keyspace may opt in
  (`pubsub_mirror: true`) to have watch events republished onto
  `pubsub-relay` topics `mesh.kv.<keyspace>.<path>` — lossy, best-effort,
  for the dashboard/agents, riding `pubsub-protocol` unchanged (this claims
  a `mesh.kv.*` leaf under pubsub-relay's reserved `<slug>.*` taxonomy;
  flagged for the harmonizer). Per-keyspace `mirror_values: bool` controls
  whether values or only key+version metadata are mirrored — any
  secrets-adjacent keyspace mirrors **metadata only**, since the dashboard
  firehose (`TopicFilter::All`) sees every topic. Consistent with
  pubsub-relay's own decision that its interest tables are volatile and NOT
  in KV — no circularity between the two libs.

### 9. The VFS layering question — flagged honestly

VFS is L3 and is a mesh *client*: it registers via service-lookup, learns
topology from mesh, and (per the locked order VFS < VDB < KG) is the storage
plane everything ABOVE the kernel uses. replicated-kv is L2, *inside* the
kernel VFS depends on. Therefore **the kernel's own state cannot live in the
storage plane it enables**: KV's SQLite file in VFS would be a boot cycle
(mesh needs KV to come up before any service, VFS included, exists) and a
failure cycle (a VFS eviction/migration policy touching kernel state could
take down the mesh that runs VFS). The rule, stated as a principle the
skeleton should carry: **mesh's data dir sits below the storage plane, on the
plain OS filesystem, excluded from VFS, VDB, and gc by construction** — the
same way a real OS's filesystem-driver metadata is not a userland file. What
VFS *does* get is observability: KV row counts, store size, sync lag, peer
cursors — published via mesh's surface schema, not via file access. This is
a deliberate, load-bearing exception to "everything lives in VFS" instincts
and is surfaced as a friction point for the operator to bless.

### 10. Keyspaces — the registration surface and the known tenants

A Ring-3/4 lib opens its keyspace at boot with a declared config
(idempotent; config conflicts are a boot error — kernel bugs, not runtime
conditions):

```rust
pub struct KeyspaceConfig {
    pub replication: Replication,          // All (v1). Factor(n) reserved — see friction
    pub expiry_events: bool,
    pub sweep_grace: Duration,             // default 24h
    pub tombstone_ttl: Duration,           // default 30d
    pub pubsub_mirror: bool,               // default false
    pub mirror_values: bool,               // default false (metadata-only mirror)
}
pub enum Replication { All /* every node */, Factor(u8) /* RESERVED, undesigned */ }
```

Known tenants at design time: `registry/` (service-registry; leases +
expiry_events + mirror metadata), `locks/` (per-acquisition keys, concern 6;
leases + expiry_events), `queues/` (queue/trigger DEFINITIONS and metadata
only — items excluded by charter), `supervision/` (service versions,
pairwise requirements, boot order), `cron/` (schedule entries), `config/`
(distributed config; the INTENT #33 git-per-commit distribution idea lands
its per-node rollout state here when designed), `builds/` (the anticipated
builds/tools registry cache, INTENT #30/#33 — anticipated, not designed).

### 11. The API surface siblings consume (the exact seam)

This expands mesh-core's `trait KvHandle` sketch ("get/put LWW,
subscribe-to-keyspace") into the full seam; it remains an internal library
trait boundary, NOT a contract edge (INTENT #29 lib exception):

```rust
pub trait KvHandle: Send + Sync {
    fn open_keyspace(&self, name: &str, cfg: KeyspaceConfig) -> Result<KeyspaceHandle, KvError>;
    fn stats(&self) -> KvStats;   // surface-schema feed: counts, size, peers, cursors, lag
}

impl KeyspaceHandle {
    // reads — always local, never block on the network
    fn get(&self, path: &str) -> Result<Option<VersionedValue>, KvError>;
    fn scan(&self, prefix: &str) -> Result<Vec<(Key, VersionedValue)>, KvError>;
    fn get_raw(&self, path: &str, include_expired: bool) -> Result<Option<Entry>, KvError>;

    // writes — local apply + replication per opts
    fn put(&self, path: &str, value: Bytes, opts: PutOptions) -> Result<Version, KvError>;
    fn delete(&self, path: &str, opts: PutOptions) -> Result<Version, KvError>;
    fn renew(&self, path: &str, holder: &str, extend: Duration) -> Result<Version, KvError>;

    // watch — at-least-latest, coalescing (concern 8)
    fn watch(&self, prefix: &str) -> WatchStream;
}

pub struct PutOptions {
    pub ttl: Option<Duration>,             // lease; holder token required if set
    pub holder: Option<String>,
    pub durability: Durability,            // default Local
    pub if_version: Option<IfVersion>,     // LOCAL CAS only — see caveat
    pub content_type: Option<String>,
}
pub enum Durability { Local, AllReachable }
pub enum IfVersion { Absent, Equals(Version) }

pub enum KvError {
    PartialReplication { confirmed: Vec<NodeId>, missing: Vec<NodeId> }, // write STANDS locally
    VersionMismatch { current: Option<Version> },   // if_version failed
    NotHolder,                                      // renew by wrong holder token
    NotFound,                                       // renew of absent/expired-swept key
    KeyspaceConfigConflict, InvalidKey, Store(String),
}
```

**The `if_version` caveat, stated in the seam docs verbatim-grade:** CAS is
evaluated against the LOCAL store only. It serializes writers *through one
node*; it is NOT a distributed CAS — a concurrent remote write can still
LWW-win after the CAS succeeds. Correct uses: same-node read-modify-write
(supervision updating its own node's records), guarding renewal. Incorrect
use: distributed mutual exclusion — that is `locks`, which is why `locks`
exists. Offering local CAS with this warning was judged better than
omitting it and watching three siblings hand-roll racier versions.

## Relationships / edges

- mesh peers ↔ mesh peers via **`kv-replication`** — the anti-entropy /
  eager-push / digest sync protocol between daemons; **generalizes and
  supersedes `registry-replication`** (the wave2-plan §5.5 flag, decided
  here: the registry no longer replicates itself; it rides this).
  *(scaffold/contracts/registry-replication.md becomes a tombstone pointing
  at kv-replication — harmonizer's edit, not mine.)*
- aws ↔ mesh via **`aws-mesh`** — replicated-kv authors the replication
  plane's S3 leg (snapshot export / bootstrap import); mesh-core provides
  registration+relay only (per mesh-core.md's explicit flag). Proposed below.
- every service ↔ mesh via **`pubsub-protocol`** — the `mesh.kv.*`
  observability mirror rides it unchanged (topic-prefix claim only; no new
  wire shape) *(authored: scaffold/contracts/pubsub-protocol.md)*.
- `service-registry`, `locks`, `queues`, `supervision`, `cron` — **internal
  lib seams, not contract edges**: they consume `KvHandle` (concern 11).
  Their external contracts (`service-lookup`, `locks-api`, `queues-api`,
  `cron-api`) are theirs; this lib is invisible in them by design.
- mesh-core Ring 0 — consumes `LocalStore` (SQLite handle) and
  `PeerTransport` (peer links); provides `KvHandle` up. Compiled-in seams
  per mesh-core.md.

## Nesting

Parent: mesh | Children: none. Module `lib/mesh::kv`. Never a standalone
crate/service (INTENT #54); may be a workspace lib crate for build
convenience but only ever consumed through mesh.

## Thoroughness level

**implementation-ready** — version model (HLC ratchet + total order), entry
shape, merge rule, lease/expiry/sweep semantics, SQLite schema, the
three-layer sync protocol, partition/merge behavior with the guarantees
table, the fresh-join rule, both watch tiers, keyspace registration, and the
full sibling API are decided and buildable as written. **approach-sketched**
for: the `aws-mesh` S3 snapshot leg (shape proposed, key-bootstrap
circularity unresolved — friction), and `Replication::Factor(n)` (reserved
field only; secrets' batch-3 co-design). Exact tuning constants
(sync intervals, page sizes, tombstone_ttl default) are config with proposed
defaults, not design forks.

## Assigned design-depth

**Fable** single Component-Designer pass (this file), per the wave2-plan
model-tier assignment, grounded in mesh-core.md's ring seams, mesh.md
concerns 1/10, pubsub-relay.md concerns 2/7, types.md guardrail 4, and
INTENT #32/#54/#71/#84.

## Suggested fill-model

implementation-ready + **high complexity** → **strong-mid model with the
test suite written FIRST**. The individual pieces are each boring (an HLC is
30 lines; the merge rule is one comparison; the sync loop is
request/page/ack), but the correctness surface is the composition — fill
should start from the non-obvious test list (clock-skew ratchet, tombstone
shadowing, fresh-join, PartialReplication mid-death, watch coalescing,
transitive delta sync, crash-mid-apply idempotency) as executable
conformance tests against a multi-daemon in-process harness (spawn N brokers
over an in-memory `PeerTransport` fake), and only then implement. Do NOT
send to a cheap model: the failure mode here is code that passes happy-path
tests and silently corrupts the wiring seam of the whole OS. The SQLite
layer and timer wheel are transcription-grade; the sync protocol + merge +
lease interplay is where the risk is.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `kv-replication` — (mesh daemon ↔ mesh daemon, every peer pair; authored
  from this file, authoritative) — the ONE replication protocol for all kernel
  keyspaces: eager push + cursor delta + full-digest safety net, riding
  mesh-core `PeerTransport` frames. **Supersedes `registry-replication`**,
  which is now a tombstone pointing at it (`service-registry` participates
  only as a keyspace tenant). → `scaffold/contracts/kv-replication.md`
- `aws-mesh` — replicated-kv's half (the replication plane's S3 leg). → `scaffold/contracts/aws-mesh.md`
- `pubsub-protocol` — topic-prefix claim only (no new wire shape). → `scaffold/contracts/pubsub-protocol.md`

Also a party to (authored elsewhere / cross-cutting): `restart-protocol` — see `scaffold/contracts/`.

