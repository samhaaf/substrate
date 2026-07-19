# vfs

**Status:** SUPERSEDES the wave-1 `vfs.md` requirements-only stub. **Nesting:**
top-level app-crate (`bin/vfs` daemon + `lib/vfs`), one instance per storage
node (`AddressingClass::NodeScoped`). **Layer:** L3 storage plane, above the
mesh kernel (L1/L2), **below VDB** (VFS < VDB < KG, LOCKED — INTENT #96).
**Consumes:** mesh (registration/relay/replicated-kv-metadata/locks/pubsub via
`mesh-client`), `gc` (per-device enforcement), `aws` (S3 overflow), `secrets`
(client-side content-encryption keys). **Consumed by:** VDB, repo, kg, projects,
rollup. Grounded in INTENT #27/#47/#48/#82/#92/#98 + the batch-1/2 designs
(replicated-kv guarantees table, mesh-core addressing + PeerTransport,
service-registry NodeScoped, locks-api, pubsub-relay taxonomy, types node/
provenance vocabulary) and the live `lib/gc` + `bin/gc`.

## Charter

`vfs` is **the boring distributed flat file system** — the mesh-wide storage
substrate on which every higher layer that needs bytes-on-disk is built. It owns
exactly one thing and owns it completely: **the placement, replication, tiering,
and lifecycle of file content across the drives of every node in the tailnet,
under a flat path namespace with per-directory policies.** Concretely it owns:
the **flat path namespace** (`vfs://<path>`, "/"-segmented but semantically flat
— a "directory" is a policy-bearing prefix, not a container object); the
**two file classes** (immutable content-addressed blobs; node-anchored mutable
files — concern 2); the **content-transfer plane** that moves bytes between
nodes reliably (concern 3 — the module's hardest new surface); **per-file
replication factor** and desired/actual **placement** over nodes and drives
(concern 4); **per-directory policies** — max size, eviction strategy (FIFO /
least-recently-updated / least-recently-accessed) — enforced per-device by
delegating DOWN to `gc` (concern 5); **RAID-inspired multi-drive warm/cold node
topology** (the walk-along Pi with external drives — concern 6); **access-can-
migrate** semantics (reading a remote blob may cache it local for reuse —
concern 7); **S3 overflow** to cold storage through the `aws` crate, client-side
encrypted (concern 8); **inference-style self-tracked performance** (uptime,
per-node read/write latency, per-drive fill, replication lag — concern 9); and
**lighter-weight per-file provenance** (INTENT #92 — concern 10).

**Boundary — what `vfs` does NOT own.** It does not own the *graph* over the
flat store — the knowledge/hierarchy layer is `projects` (INTENT #47) and `kg`,
which point AT vfs files. It does not own *git* — `repo` (INTENT #80) builds
branches/worktrees on vfs, vfs just serves it mutable files. It does not own
*database semantics* — `vdb` opens SQLite files vfs hosts; vfs never parses a
row. It does not own the *replicated-state engine* — file METADATA rides
`replicated-kv` (a mesh internal lib), which owns LWW/anti-entropy/tombstones;
vfs is a KV keyspace tenant, and **file CONTENT is deliberately NOT a KV value**
(concern 1). It does not own the *S3 API* — that is `aws` (INTENT #106); vfs
decides *what overflows and when*, aws does the bytes-to-cloud. It does not own
*single-device GC mechanics* — TTL/size-budget/LRU/lock-with-expiry/reclaimers
are `gc`'s (already built, `lib/gc`); vfs decides *distributed* policy and
delegates *per-device enforcement* to gc (concern 5, and the gc-relationship
recommendation). It holds **no application data** and imposes **no schema** on
file bytes. It is one boring layer: distribution rides mesh, enforcement rides
gc, cloud rides aws — vfs is the placement brain in the middle.

## Primary design concerns

### 1. Metadata rides replicated-kv; content does NOT — the load-bearing split

The operator's brief names this the biggest open hole, and the answer is a hard
line drawn once: **file metadata is small LWW state and lives in
`replicated-kv`; file content is bulk bytes and rides a dedicated
content-transfer plane (concern 3) — never the KV.** Rationale, each
load-bearing:

- replicated-kv is an eventually-consistent per-key **LWW register over opaque
  small values**, fully replicated to every daemon (`Replication::All`), synced
  by periodic anti-entropy + full-digest compare (its concerns 1/5). That is
  exactly right for "which paths exist, what content-hash each maps to, where
  the replicas should live, what the directory policy is" — a few hundred KB at
  personal-mesh scale, boring. It is exactly WRONG for a 40 GB model blob: KV
  digests, LWW overwrites, and full-state maps are not a bulk-binary transport.
- Because the metadata is **fully replicated**, *every* node can answer "where
  are the replicas of `vfs://models/llama-70b.gguf`?" from a purely local KV
  read — the single-port-locality property ("anywhere you access mesh is the
  same," INTENT #35) holds for the storage directory for free, with zero vfs
  cross-node calls to *find* content. Only *moving the bytes* touches the
  network.
- **The VFS "per-file replication factor" (INTENT #48) is NOT
  replicated-kv's reserved `Replication::Factor(n)`.** VFS metadata uses
  `Replication::All` (boring, fully replicated). The replication factor is a
  *content* placement policy — "keep this blob on R nodes" — recorded as
  ordinary data inside the fully-replicated metadata (concern 4). This cleanly
  sidesteps replicated-kv's undesigned `Factor(n)` (its concern 10 friction);
  vfs never needs it.

VFS's KV keyspaces (opened via `mesh-client`'s KvHandle, keyspace `vfs/`):

```
vfs/dir/<path>            -> DirPolicy      (max size, eviction, default repl factor, tier pref)
vfs/file/<path>           -> FileManifest   (class, content hash, chunk list, size, repl factor, provenance-lite)
vfs/blob/<content_hash>   -> BlobPlacement  (desired holders + actual holders {node,drive}, S3 ref, size)
vfs/node/<node_id>        -> NodeStorageTopology (its drives, capacities — concern 6; refreshed by perf)
```

`replicated-kv`'s own file lives on the plain OS filesystem BELOW the storage
plane (its concern 9) — no boot cycle: mesh+KV come up (L1/L2), *then* vfs (L3)
opens its keyspace. VFS content blobs live in gc-managed drive directories that
vfs owns — those are what vfs manages; the kernel's metadata store is not one of
them. This mirrors and honors replicated-kv's concern-9 layering rule exactly.

### 2. Two file classes — immutable content-addressed blobs vs node-anchored mutable files

A single file model cannot serve both "an immutable 40 GB model weight
replicated 3×" and "a live SQLite database VDB mutates on every transaction"
(INTENT #96: SQLite files live in VFS) and "a git worktree repo edits in place"
(INTENT #80). So VFS has **two file classes**, chosen per file at create time —
a deliberate, controversial fork (flagged):

- **`Immutable` (default; the S3-like flat store).** Content-addressed: the file
  is chunked (fixed ~4 MB chunks — fill-time constant), each chunk hashed
  (SHA-256), the FileManifest is the ordered chunk-hash list + a rollup content
  hash. Writing new bytes to a path = creating new content = a new manifest
  (LWW-wins on the path). Enables **dedup** (two paths → same content hash share
  blobs), **partial/resumable transfer** (chunk-granular), and **factor-based
  replication** (concern 4). The overwhelming majority of files.

- **`NodeAnchored` (mutable; DB backing files, repo worktrees, active working
  files).** A file that is mutated in place cannot be re-hashed-and-re-replicated
  on every `fwrite`. Instead it is **anchored to exactly one owner node**, where
  VFS provides a **real local OS path** (in a vfs-managed directory on a chosen
  drive) that VDB/repo open and mutate directly — vfs does NOT intercept every
  write. Durability/replication is **snapshot-driven**: the owner (VDB at a
  transaction-consistent checkpoint; repo at a commit) calls
  `snapshot(path) -> content_hash`, and VFS content-addresses + replicates *that
  snapshot* as an immutable blob. Exclusive-writer safety is a `locks-api` mutex
  keyed `vfs.anchor.<path>` (concern 12). This is the boring answer to "SQLite
  files live in VFS": VFS is the *file host and snapshot-replicator*, not a block
  device intercepting mutations.

The class is a field on the manifest, not two subsystems — placement, tiering,
perf, and the surface are shared; only the write/replicate path forks. Stated as
a friction point because it is the one place VFS is not a single uniform model.

### 3. The content-transfer plane — reliable, chunked, mesh-relayed (the biggest new surface)

Content bytes move between nodes over a plane that is **neither pub/sub nor
KV**, and it rides mesh single-port locality (INTENT #58 — a service never opens
a socket to a remote node; it hands mesh an addressed envelope). Precedent
exists: `completion-router` already streams byte-transparent bodies through the
mesh relay (`forward()`); the content plane is the same shape for blobs.

- **Transport = mesh-transport `Request`/`Response` with a streamed body,
  addressed `Node{N}` (pinned).** Because metadata is fully replicated (concern
  1), the requesting vfs already knows *which node* holds a blob; it issues a
  pinned request, mesh relays it one hop to that node's daemon → local vfs →
  streamed chunk frames back through the relay. This is a distinct mesh-transport
  `MsgKind` (bulk/stream), **NOT** the lossy `pubsub-relay` (file content needs
  reliable, ordered, integrity-checked delivery — pubsub-relay concern 6 is
  explicitly lossy) and **NOT** a KV value.
- **Pull-driven convergence (the replication engine).** VFS never "pushes a
  replica" imperatively. `BlobPlacement.desired` (in fully-replicated metadata)
  names the set of nodes that SHOULD hold a blob; each node's vfs **watches
  `vfs/blob/*`** (KV watch — replicated-kv concern 8) and reconciles its local
  blob set against desired: a blob it should hold but lacks → **pull** it from a
  current `actual` holder; a blob it holds but shouldn't (0-refcount, or evicted
  by policy) → hand to `gc` for reclaim. This makes replication **self-healing
  and offline-tolerant by construction** — a node that was offline pulls its
  missing blobs on reconnect, exactly mirroring KV anti-entropy, for the same
  reason. No imperative push protocol, no failover election: desired-placement is
  LWW metadata, actual-placement converges.
- **Integrity + resume.** Every chunk is verified against its hash on receipt; a
  dropped transfer resumes at the last verified chunk (chunk-granular). A blob is
  "actually held" only once all its chunks verify — a node writes itself into
  `BlobPlacement.actual` only on complete, verified receipt, so a half-pulled
  blob is never advertised as a replica.
- **Refcount + blob GC are DERIVED, not a protocol.** A blob's refcount = the
  number of `FileManifest`s pointing at its content hash — computable locally
  from fully-replicated metadata by any node. Refcount → 0 sets
  `desired = {}`; every holder's reconcile then evicts. No distributed refcount
  counter, no decrement race — the metadata replication already carries it. (One
  boring hazard, handled: a manifest deleted while a peer is offline; the peer's
  stale manifest re-appears at merge and could resurrect a 0-refcount blob's
  desired set — this is exactly replicated-kv's tombstone/fresh-join concern 7
  operating on vfs's keyspace, inherited, not re-solved here.)

This plane is a NEW contract, `vfs-content` (scaffold/contracts/vfs-content.md) — the one edge
the inventory did not name, surfaced explicitly.

### 4. Placement + per-file replication factor — desired vs actual, drive-aware

`BlobPlacement` separates **desired** (policy target) from **actual** (verified
current holders), and placement is over **(node, drive)** pairs, not just nodes,
so a multi-drive node (concern 6) can hold a warm and a cold copy independently.

```rust
pub struct BlobPlacement {
    pub content_hash: Hash,
    pub size: u64,
    pub desired: Vec<PlacementTarget>,   // policy target: R durable holders (+ tiers)
    pub actual:  Vec<HeldReplica>,       // verified current holders (node,drive) or S3
    pub s3: Option<S3Ref>,               // cold/overflow copy in aws (concern 8)
}
pub struct PlacementTarget { pub node: NodeId, pub drive: Option<DriveId>, pub tier: StorageTier }
pub struct HeldReplica {
    pub node: NodeId, pub drive: DriveId, pub tier: StorageTier,
    pub kind: ReplicaKind,               // Durable (policy-mandated) | Cache (access-migrated, concern 7)
    pub verified_at: DateTime<Utc>,
}
pub enum ReplicaKind { Durable, Cache }
```

- **Replication factor** = `count(desired durable targets)`, defaulting from the
  directory policy, overridable per file. The placement engine picks targets
  preferring: nodes with the required capacity, warm tier for hot files, drive
  fill-balance, and low measured latency (concern 9 feeds this). Placement is a
  *derivation* the writing node computes then records; every node reconciles
  toward it — no central placer.
- **Cache replicas never count toward the factor** (concern 7). gc is instructed
  (concern 5) to evict `Cache` replicas first and to **never** evict a `Durable`
  replica that would drop `actual durable` below the factor — the one hard
  invariant vfs asserts against gc's eviction.

### 5. Per-directory policies + the gc relationship (the STANDING-OPEN question, with a recommendation)

Per-directory policy is fully-replicated data:

```rust
pub struct DirPolicy {
    pub max_bytes: Option<u64>,
    pub eviction: Eviction,              // Fifo | LeastRecentlyUpdated | LeastRecentlyAccessed
    pub default_replication: u8,         // factor for files created under this prefix
    pub tier_pref: TierPref,             // WarmFirst | ColdFirst | S3Overflow (concern 8)
    pub provenance: ProvenanceLevel,     // Off | Light (default) — concern 10
}
pub enum Eviction { Fifo, LeastRecentlyUpdated, LeastRecentlyAccessed }
```

**Enforcement is `gc`'s, on each device; the DECISION is vfs's.** VFS translates
a `DirPolicy` for the local node's slice of a directory into a gc managed-dir
config (budget = the node's share of `max_bytes`; strategy = the `Eviction`
mapped onto gc's LRU/FIFO reclaimer; `lock(ttl)` on freshly-placed durable
replicas so they survive the next sweep — the `make_room → write → register →
lock` pattern gc.md concern 2 already prescribes). gc runs the sweep and the
eviction mechanics; vfs supplies the `.gc` config and the "which replica is safe
to evict" answer (concern 4's invariant).

**The rolled-in-vs-called-as-tool recommendation (round-3 STANDING OPEN, INTENT
#27/#48; my concrete call, flagged for operator confirmation):**

> **Recommend: `gc` stays a distinct crate — NOT absorbed into vfs — and vfs
> drives the node-local gc as a *called tool over its WebSocket/REST surface*
> (the `vfs-gc` edge), honoring INTENT #28 verbatim ("so the virtual file
> system can update it over WebSocket"). VFS becomes the per-node HOST of the
> single centralized gc store (INTENT #28) for vfs-managed directories: on a
> node running vfs, `bin/gc`'s `:8430` surface is served by / co-located with
> vfs, and vfs registers/updates its managed dirs and `.gc` configs over that
> surface; on a node without vfs (a pure inference node), `bin/gc` runs
> standalone exactly as today.**

Rationale: (a) **does not orphan inference's embedded gc** — `lib/gc` embedded
by `models`/`cache`/`engine` (the `gc-managed-dirs` edge) is untouched; folding
gc *into* vfs would force inference to depend on vfs, a layering inversion. (b)
**Boring layers, INTENT #38/#55** — gc owns single-device mechanics (already
built + tested, 5 passing tests), vfs owns distributed policy/placement; the
split is the same "policy above / mechanism below" as mesh-core↔supervision. (c)
**The WS hop is free where it matters** — gc operations (register a dir, update a
budget, run make_room) are control-plane, not per-byte; the hot path (reading/
writing bytes) is the content-transfer plane (concern 3), which never touches
gc. (d) **Honors INTENT #28's "one centralized per-node store, updatable over
WebSocket"** literally — one gc store per node, vfs updates it over WS.
Flagged residual: unifying inference's embedded gc dirs with vfs's gc store into
literally one physical store per node touches inference's wiring (it currently
opens its own `gc.db`) — that convergence is gc/inference's batch-5 concern, out
of scope here; this design only requires that vfs drive gc for *vfs-managed*
directories.

*(Skeleton-time optimization latitude: if the WS hop ever measures, vfs's
per-node leg MAY embed `lib/gc` in-process for vfs-managed dirs and re-expose the
same gc surface — same code, no wire. The design does not depend on which; the
`vfs-gc` contract shape is identical either way. Called-as-tool is the default
because it matches the operator's stated mental model.)*

### 6. RAID-inspired multi-drive nodes — the walk-along Pi (INTENT #48)

A node advertises its **storage topology** (its drives) so vfs can place replicas
on specific drives and treat one node as a warm/cold tier. This is the
`DriveInfo`/`NodeStorageTopology` type that types.md (its concern node.rs)
deferred to *this* design — proposed now (recorded in the authored contracts, per-pair
round):

```rust
pub type DriveId = String;                 // stable per-drive id (serial/uuid)
pub enum StorageTier { Warm, Cold }        // RAID-ish warm/cold (INTENT #48)
pub enum DriveMedia { InternalSsd, InternalHdd, ExternalSsd, ExternalHdd, Network, Other }
pub struct DriveInfo {
    pub drive_id: DriveId, pub node: NodeId,
    pub label: String,                     // "pi-ext-usb-0"
    pub tier: StorageTier,                 // operator-assignable per drive
    pub media: DriveMedia,
    pub mount_path: String,                // where vfs stores blobs on this drive
    pub capacity_bytes: u64,
    pub used_bytes: u64,                   // refreshed by perf reporting (concern 9)
    pub writable: bool,
}
pub struct NodeStorageTopology { pub node: NodeId, pub drives: Vec<DriveInfo> }
```

- A **multi-drive node** (the Pi with two external drives) exposes >1 `DriveInfo`;
  the operator tags drives `Warm`/`Cold`. Placement targets a `(node, drive)`
  pair, so a cold-tier directory routes replicas to that node's cold drive
  specifically — the "RAID-like warm/cold storage node" (INTENT #48) is just
  drive-scoped placement, no RAID controller, no striping-by-default.
- This lands in `types::node` per types.md's stated plan (`NodeCapabilities`
  gains `#[serde(default)] pub storage: Option<NodeStorageTopology>`) — satisfying
  types' inclusion test now that a real consumer (vfs) and the `vfs-mesh` contract
  name it. Wire-crossing → guardrail-4 discipline (additive, `serde(default)`,
  `#[serde(other)]` on the enums).

### 7. Access-can-migrate — reading remote data may cache it local (INTENT #27/#48)

"Accessing data that lived on a different machine can simultaneously move it to
this machine for reuse if requested." A read (`vfs-content` `Read`) carries a
`cache_local: bool` hint (default per directory policy). When set and the blob is
not local, vfs pulls it (concern 3) to serve the read AND records itself as a
`ReplicaKind::Cache` holder in `BlobPlacement.actual`. Cache replicas:

- **do not count toward the replication factor** (concern 4) — they are
  opportunistic, not durable;
- are **the first thing gc evicts** under budget pressure (concern 5), and gc may
  evict them freely (unlike durable replicas);
- turn "read the 70B weight that lives on the desktop" into "the laptop now has a
  warm local copy for the next run" — exactly the operator's reuse case — without
  ever weakening the durable guarantee.

`cache_local` is the operator's explicit "optionally pull it local for reuse if
requested" made a first-class per-read flag.

### 8. S3 overflow — cold tier through `aws`, client-side encrypted (INTENT #48/#98/#106)

When the personal mesh runs out of capacity, or a directory's `tier_pref` is
`S3Overflow`/`ColdFirst`, a blob overflows to S3 cold-storage classes through the
`aws` crate (the `aws-vfs` edge) — **client-side encrypted before aws sees a
byte** (INTENT #48 "not just encrypted at rest"; keys in `secrets`, INTENT #98).

- **VFS decides what overflows and when** (placement policy: least-recently-
  accessed durable blobs when mesh capacity < headroom, or policy-forced cold);
  **aws does the bytes-to-cloud**. S3 is a **cold replica location** in
  `BlobPlacement.s3`, and — echoing replicated-kv's `aws-mesh` stance — a
  **passive tier**: never a tiebreak, never required for the mesh to function
  (INTENT #34 no-fault-line spirit). A blob may be S3-only (mesh capacity
  reclaimed) — reading it **rehydrates** (pull from S3 via aws), optionally
  caching local (concern 7).
- **Encryption is VFS's, not aws's** — client-side means vfs encrypts the chunk
  stream with a key it resolves from `secrets` before handing ciphertext to aws.
  This implies an edge **vfs ↔ secrets** the wave-2 inventory did not name —
  flagged as a MISSING pair (`vfs-secrets`), since "keys held by secrets" +
  "client-side" can only mean vfs holds the key at encrypt time (vfs is not an
  LLM — plaintext key access is legitimate; `llm_safe` does not apply). aws never
  sees plaintext or the key.
- Overflow-vs-evict is a policy choice per directory: `ColdFirst`/`S3Overflow`
  dirs overflow to S3 (durable, encrypted); non-overflow dirs simply evict
  (delete) under budget. The eviction strategies (concern 5) and the overflow
  tier compose: evict a *cache* replica first, then overflow a *durable* blob to
  S3, and only delete when policy says the blob is disposable (refcount 0).

### 9. Inference-style self-tracked performance (INTENT #48)

"VFS tracks its own performance the same way inference does: uptime, per-node
read/write latency, etc." Same telemetry philosophy as inference's `telemetry`
lib, VFS-specific metrics — an internal `perf` module sampling and publishing:

- **per-node:** uptime, read latency / write latency (p50/p95), read+write
  throughput, content-plane transfer rate, pull-queue depth, replication lag
  (count of blobs where `actual durable < desired`);
- **per-drive:** capacity/used/free (feeds `DriveInfo.used_bytes` and placement),
  per-drive IO latency (warm-vs-cold honesty);
- **cluster-derived (any node computes from replicated metadata):** total logical
  bytes, physical bytes, dedup ratio, under-replicated blob count, S3-tier bytes.

These publish two ways: the **boring surface schema** (`surface-schema` — the
dashboard renders vfs's panel from it; honest-estimate markers where a drive
reading is sampled, INTENT #9 style), and a **`vfs.*` pub/sub topic prefix**
(`pubsub-protocol`) for live events — `vfs.file.placed`, `vfs.file.evicted`,
`vfs.blob.pulled`, `vfs.blob.overflowed_s3`, `vfs.replication.degraded`,
`vfs.drive.full`. Latency numbers also feed the placement engine (concern 4:
prefer low-latency targets), closing the loop the way inference's telemetry feeds
its scheduler.

### 10. Provenance — lighter than the database plane (INTENT #92)

VFS carries provenance as a **design requirement but a LIGHT one** — operator:
"I typically don't care about provenance in the file system — usually only in the
database and the stack pattern." VDB is the first-order/healthcare-grade home
(per-project/per-database, every handler touch); VFS's is per-file and coarse,
reusing `types::Provenance` (already defined, batch-1) rather than a bespoke
triple:

```rust
// on FileManifest, gated by DirPolicy.provenance (Off | Light)
pub struct FileProvenance {
    pub created: Provenance,             // origin node/service, emitted_at, correlation_id
    pub last_modified: Provenance,
    #[serde(default)] pub snapshot_of: Option<Hash>, // for NodeAnchored snapshots (concern 2)
}
```

Light provenance answers "who/what/when created or last replaced this file, and
what causal chain it belongs to" — enough to trace a file back into a VDB handler
or a repo push (`correlation_id` stitches across planes), without recording every
byte-level access. `ProvenanceLevel::Off` is allowed for high-churn scratch
directories. This is the deliberate under-design INTENT #92 calls for, stated so
it is not mistaken for a gap.

### 11. Path model — flat namespace; the graph is projects' (INTENT #47)

The path space is **flat**: `vfs://<path>` where `<path>` is a "/"-segmented
string, but there are **no directory objects** — a "directory" is any path
prefix that *happens to* carry a `DirPolicy` (`vfs/dir/<prefix>`). Files are the
only first-class objects. Listing a "directory" is a prefix scan over
`vfs/file/*`. There is no hierarchy semantics, no rename-a-tree, no inode graph —
"boring distributed flat file system." The **graph over it is `projects`** (a KG
encoding project structure whose nodes point at vfs files — INTENT #47) and `kg`
(nodes point at vfs files with existence validation — concern, `kg-vfs`). VFS
stays an S3-bucket-flat keyspace; the friendly navigable structure is a strictly
higher layer. This is the operator's exact framing ("Does the VFS just look like
an S3 bucket, and then we build something more project-oriented on top of it").

### 12. Concurrency + the two addressing classes

- **Immutable writes** to the same path from two nodes → LWW on the manifest
  (naive timestamp-wins, blessed INTENT #32, inherited from replicated-kv); both
  content blobs exist (content-addressed, no corruption), only the *path binding*
  is arbitrated — one wins, deterministically, fleet-wide. No lock needed.
- **NodeAnchored (mutable) files** need exclusive-writer safety: a `locks-api`
  mutex `vfs.anchor.<path>` (threshold 1) held by the owner (VDB/repo) around the
  mutation window; a partition twin surfaces `locks`' `PartitionMergeExceeded`
  (INTENT #84), handled per-application by the owner. VFS is a plain `locks-api`
  consumer here.
- **Addressing:** vfs registers `AddressingClass::NodeScoped` (one instance per
  node — service-registry concern 2 literally names "per-node vfs leg"). Callers
  use `AnyNode{vfs}` (locality-preferred: your local vfs) for ordinary reads/
  writes, and `Node{N, vfs}` (pinned) to force a specific node's storage (e.g.
  place a durable replica on the cold Pi). The content-transfer plane always uses
  `Node{N}` (you pull from a *specific* holder). This is a clean instance of the
  two addressing classes (INTENT #59), not a bespoke scheme.

## Relationships / edges

Contract edges (cross-process WS/wire over mesh-transport `:3649`; client halves
via `mesh-client`):

- **gc** via `vfs-gc` — per-device policy enforcement: vfs registers/updates
  managed dirs + `.gc` configs and the safe-to-evict answer over gc's WS/REST
  surface; gc runs the sweep (concern 5). *(scaffold/contracts/vfs-gc.md)*
- **mesh** via `vfs-mesh` — registration (`service-lookup`, NodeScoped) +
  node/drive **storage-topology** publication (`NodeStorageTopology`/`DriveInfo`)
  + perf reporting; the S3-overflow leg does NOT ride this edge (it rides
  `aws-vfs`). *(scaffold/contracts/vfs-mesh.md)*
- **vfs peer ↔ vfs peer** via **`vfs-content`** — **NEW (authored — see Contracts section):** the
  reliable chunked content-transfer plane (concern 3), relayed by mesh
  (`Node{N}` pinned), pull-driven. The edge the inventory did not name; the
  biggest new surface. *(authored: scaffold/contracts/vfs-content.md)*
- **aws** via `aws-vfs` — S3 overflow cold tier: ciphertext chunks to/from S3
  cold-storage classes through aws (concern 8). *(scaffold/contracts/aws-vfs.md)*
- **secrets** via **`vfs-secrets`** — **NEW (flagged MISSING):** the client-side
  content-encryption key for S3 overflow (concern 8); not in the wave-2 inventory,
  surfaced here. *(scaffold/contracts/vfs-secrets.md — MISSING)*
- **kg** via `kg-vfs` — kg nodes point at vfs files; vfs is the target of kg's
  existence-validation (`exists(path) -> bool + manifest summary`).
  *(scaffold/contracts/kg-vfs.md)*
- **projects** via `projects-vfs` — projects (L6 stub) is the graph layer over the
  flat store (INTENT #47). *(scaffold/contracts/projects-vfs.md)*
- **vdb** via `vdb-vfs` (renamed from `stack-vfs` — flagged) — VDB's SQLite files
  are `NodeAnchored` vfs files; snapshot-replicated (concern 2). *(scaffold/
  contracts/stack-vfs.md → rename)*
- **repo** via `repo-vfs` — repo materializes git trees/worktrees as vfs files
  (mostly `NodeAnchored` mutable worktrees + immutable object snapshots).
  *(scaffold/contracts/repo-vfs.md)*
- **rollup** via `rollup-vfs` — fragment/plugin storage residence (rollup's edge,
  probable — vfs is the store party). *(scaffold/contracts/rollup-vfs.md)*

Cross-cutting protocols (surface-schema-style, vfs a party, authored by their
owners — I consume the shapes, do not re-author):

- `pubsub-protocol` — the `vfs.*` topic prefix (concern 9). Claims a `vfs.*` leaf
  under pubsub-relay's `<slug>.*` taxonomy (additive — flagged for the harmonizer).
- `surface-schema` — vfs publishes its boring surface (perf + dirs + placement)
  for the dashboard (concern 9).
- `restart-protocol` — vfs is a supervised service; a mid-transfer is a
  `CriticalSection` / `FinishAndRelinquish`-shaped interruptibility (a durable
  replication in flight should finish before yield). Consumed, not authored.
- `service-lookup` — vfs registers (NodeScoped) and resolves aws/secrets/gc/peers.
- `locks-api` — vfs consumes for `NodeAnchored` write exclusivity + placement
  transactions (concern 12).

Internal-lib seams (compiled-in, NOT contract edges): `mesh-client` (register/
resolve/pubsub/kv/locks handles), `substrate-types` (`node`/`provenance`/
`pubsub`/`error` vocabulary + the new storage types), and optionally `lib/gc`
embedded (skeleton-time latitude, concern 5).

## Nesting

Parent: none (top-level app-crate `bin/vfs` + `lib/vfs`). Children (nested
internal libs, compiled into the vfs daemon, never standalone): **`content`**
(blob store + chunking/hashing + the content-transfer plane), **`placement`**
(the desired/actual replication + tiering engine + reconcile loop), **`policy`**
(DirPolicy → gc config translation, the gc client), **`perf`** (telemetry
sampling + surface/pubsub publication). These are libraries under the vfs app per
INTENT #22 (a crate is an app; its internal pieces are libs), not top-level
crates. The parent/child structure lives here + overview.md, not the directory
layout (flat `components/`).

## Thoroughness level

**implementation-ready** for: the metadata-in-KV / content-out-of-KV split
(concern 1) and the vfs keyspaces; the two file-class model (concern 2); the
content-transfer plane's pull-driven-convergence design + integrity/resume
(concern 3, with the `vfs-content` wire proposed); the desired/actual
drive-aware placement model + the never-evict-below-factor invariant (concern 4);
the DirPolicy → gc delegation + the gc-relationship recommendation (concern 5);
the `DriveInfo`/`NodeStorageTopology` types (concern 6); access-migration cache
replicas (concern 7); the S3-overflow-as-cold-passive-tier policy (concern 8);
the perf/telemetry surface (concern 9); light provenance (concern 10); the flat
path model (concern 11); and concurrency/addressing (concern 12).
**approach-sketched** for: the exact S3 client-side encryption key exchange
(the `vfs-secrets` edge is newly surfaced, batch-3 `secrets` co-design — friction);
the NodeAnchored snapshot cadence/coordination with VDB (the vdb-vfs shape is
proposed from vfs's side, but VDB is batch-4 — its checkpoint semantics reconcile
there); and the chunk-size / transfer-window / reconcile-interval constants
(fill-time tuning with proposed defaults, not design forks).

## Assigned design-depth

**Opus**, single strong-model Component-Designer pass (this file), per the
wave2-plan tier assignment ("distribution/consistency rides mesh, which is why
this stays Opus"). Grounded in INTENT #27/#47/#48/#82/#92/#98/#106, the batch-1/2
designs (replicated-kv guarantees/anti-entropy, mesh-core addressing +
PeerTransport + mesh-transport, service-registry NodeScoped/lease, locks-api,
pubsub-relay taxonomy/lossiness, supervision restart ladder, types node/
provenance/surface vocabulary), the co-batched `gc.md`/`aws.md`, and the live
`lib/gc` + `bin/gc`.

## Suggested fill-model

**implementation-ready + high complexity → strong-mid model, tests-first for the
content plane.** Most of vfs is boring transcription against frozen seams — the
KV keyspaces, DirPolicy → gc translation, the perf surface, the placement
records, the path/scan model — a mid model transcribes these from this file.
**Two surfaces want the test suite written FIRST and must not go to a cheap
tier:** (1) the **content-transfer + pull-convergence loop** (concern 3) — its
failure mode is a blob that reports replicated but isn't, or a transfer that
corrupts silently; write conformance tests over a multi-node in-process harness
(spawn N vfs legs over a fake PeerTransport, assert: a factor-3 file converges to
3 verified holders; an offline node pulls its share on reconnect; a 0-refcount
blob is evicted everywhere; a half-pull never advertises; a chunk hash mismatch
retries not corrupts) *before* implementing; (2) the **never-evict-below-factor
invariant** at the vfs↔gc seam (concern 4/5) — a property test that gc, given
vfs's safe-to-evict answer, never drops durable `actual` below `desired`.
Everything else (drive topology, S3 overflow policy, provenance, surface schema)
is transcription-grade. Fill **after** replicated-kv, service-registry, locks,
and gc are filled (it rides all four) and **alongside** aws's batch-3 S3 surface.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `vfs-content` (vfs peer ↔ vfs peer, relayed by mesh) — the bulk content-transfer plane (added by the contract round). → `scaffold/contracts/vfs-content.md`
- `vfs-mesh` — (vfs ↔ mesh) — registration + storage topology + perf. → `scaffold/contracts/vfs-mesh.md`
- `vfs-gc` — (vfs ↔ gc) — per-device enforcement (gc.md is the counterpart). → `scaffold/contracts/vfs-gc.md`
- `aws-vfs` — (aws ↔ vfs) — S3 overflow cold tier (aws.md is the counterpart). → `scaffold/contracts/aws-vfs.md`

Also a party to (authored elsewhere / cross-cutting): `aws-mesh`, `gc-events`, `kg-vfs`, `projects-vfs`, `repo-vfs`, `rollup-vfs`, `service-lookup`, `surface-schema`, `vdb-vfs` — see `scaffold/contracts/`.

Component-side notes retained by title (full text in git history, pre-harmonization): `vfs-secrets` — client-side content-encryption key; Storage-side sketches for consumer-owned edges (vfs is the target).

