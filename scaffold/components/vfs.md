# vfs

**Status:** SUPERSEDES the wave-1 `vfs.md` requirements-only stub. **Nesting:**
top-level app-crate (`bin/vfs` daemon + `lib/vfs`), one instance per storage
node (`AddressingClass::NodeScoped`). **Layer:** L3 storage plane, above the
mesh kernel (L1/L2), **below VDB** (VFS < VDB < KG, LOCKED — INTENT #96).
**Consumes:** mesh (registration/relay/replicated-kv-metadata/locks/pubsub) via
**`chassis`** (the daemon-wrapper lib — `mesh-client` retired into it, ledger D1),
its **internal `gc` module** (per-device enforcement — absorbed, INTENT #166 Q9,
concern 5), `aws` (S3 overflow), `secrets` (client-side content-encryption keys).
**Consumed by:** VDB, repo, kg, projects, rollup. Grounded in INTENT
#27/#47/#48/#82/#92/#98 + #166 Q9 (gc→vfs) + the batch-1/2/3/4 designs
(chassis daemon-wrapper + outbox/blessing thin-profile, mesh-transport envelope +
the relayed `StreamOpen/Chunk/Close` tunnels, replicated-kv guarantees table,
mesh-core addressing, service-registry NodeScoped, locks-api, pubsub-relay
taxonomy, queues-api event-id semaphore, types node/provenance vocabulary) and the
absorbed `lib/gc` mechanics (reused, not rewritten).

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
vfs's **internal `gc` enforcement module** (absorbed, INTENT #166 Q9 — the
mark-for-removal → move-between-devices → cold-storage spectrum, concern 5);
**RAID-inspired multi-drive warm/cold node
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
decides *what overflows and when*, aws does the bytes-to-cloud. **Single-device
enforcement mechanics** —
mark-for-removal, size-budget/LRU/FIFO, lock-with-expiry, move-between-devices,
cold-storage hand-off — are NOW vfs's own **internal `gc` module** (absorbed per
INTENT #166 Q9, concern 5): vfs decides *distributed* policy AND runs the
per-device enforcement spectrum in-process (the built `lib/gc` mechanics reused,
not a separate daemon). It holds **no application data** and imposes **no schema**
on file bytes. It is one boring layer: distribution rides mesh (via chassis),
enforcement is vfs's internal gc spectrum, cloud rides aws — vfs is the placement
brain with the enforcement hand built in.

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

VFS's KV keyspaces (opened via `chassis`'s KvHandle, keyspace `vfs/`):

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

- **Transport = mesh-transport's RELAYED, scoped, one-directional stream tunnel
  (`StreamOpen`/`StreamChunk`/`StreamClose`, INTENT #153), addressed `Node{N}`
  (pinned) — the authorized default.** Because metadata is fully replicated
  (concern 1), the requesting vfs already knows *which node* holds a blob; it
  opens a pinned stream, mesh relays the `StreamChunk` frames one hop
  (local daemon → holder's daemon → holder's vfs → chunk frames back through the
  relay, seq-ordered and backpressured). The bytes stay **on the relay** so mesh
  can observe, backpressure, and resume across an interruption (mesh-transport §5
  "Relayed stream (boring default, authorized)"). This is **NOT** the lossy
  `pubsub-relay` (file content needs reliable, ordered, integrity-checked
  delivery — pubsub-relay is explicitly lossy) and **NOT** a KV value.
  - **The DIRECT brokered peer-link tunnel — taking multi-GB blob bytes OFF the
    relay for a scoped node→node pipe — is AUTHORIZATION PENDING (OQ-30 / INTENT
    #153).** vfs-content is authored implementation-ready on the *relayed* path;
    the direct path is a marked, non-decided optimization seam that MUST NOT be
    built until the operator blesses OQ-30. The frame vocabulary
    (`StreamOpen/Chunk/Close`) is identical for both modes, so flipping vfs's bulk
    pull from relayed to direct is a one-line `Address`/mode change once blessed —
    no contract reshape. (This mirrors INTENT #114's "bulk S3 transfer =
    direct-with-mesh-issued-permission" instinct for the *S3* leg, held under the
    same OQ-30 gate.)
- **Pull-driven convergence (the replication engine).** VFS never "pushes a
  replica" imperatively. `BlobPlacement.desired` (in fully-replicated metadata)
  names the set of nodes that SHOULD hold a blob; each node's vfs **watches
  `vfs/blob/*`** (KV watch — replicated-kv concern 8) and reconciles its local
  blob set against desired: a blob it should hold but lacks → **pull** it from a
  current `actual` holder; a blob it holds but shouldn't (0-refcount, or evicted
  by policy) → hand to the internal `gc` module for reclaim. This makes replication **self-healing
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
- **Cache replicas never count toward the factor** (concern 7). vfs's internal gc
  module (concern 5) evicts `Cache` replicas first and **never** evicts a
  `Durable` replica that would drop `actual durable` below the factor — the one
  hard invariant, now a direct in-process assertion (gc is absorbed, not a
  separate service).

### 5. Per-directory policies + the internal `gc` enforcement module — the spectrum (INTENT #166 Q9, SETTLED)

**SETTLED (INTENT #166 Q9): `gc` is ABSORBED INTO vfs as an internal enforcement
module — not a distinct crate, not a called-as-tool WS edge.** The operator's
verbatim resolution: *"garbage collection is actually a spectrum —
mark-for-removal, move to another device, cold storage; maybe take it apart and
reuse pieces."* This wave consolidates on that: the standing round-3
"rolled-in-vs-called-as-tool" question is **closed in favor of rolled-in**, and
the earlier called-as-tool recommendation is retired. The built `lib/gc` pieces
are **reused, not rewritten** — vfs's internal `gc` module *is* that mechanism,
recompiled in-process; the `vfs-gc` command vocabulary survives as this module's
**internal API surface** (the contract file is retained as its design record, not
a live wire edge).

**The spectrum, owned end-to-end by vfs (the reason the operator folded it in):**
enforcement is one continuum, not a delete switch —

1. **mark-for-removal** — today's evict/delete: a blob whose refcount hit 0
   (concern 3) or a `Cache` replica under budget pressure is marked and reclaimed
   by the LRU/FIFO reclaimer (the built `lib/gc` mechanics).
2. **move-between-devices** — the `Migrate` reclaimer hand-back: rather than
   delete, relocate a durable blob from a full warm drive to a cold drive on the
   same or another node (concern 6's `(node,drive)` placement is the target
   vocabulary; the placement engine, concern 4, picks the destination).
3. **cold-storage** — the S3/cold tier (concern 8): overflow a durable blob to
   `aws`, client-side encrypted, when even cold local drives are full or policy
   forces it. Deletion of the local copy is then just step 1 applied to a blob
   that now has a durable S3 replica.

Placement policy (concern 4) chooses *where on the spectrum* a blob under pressure
goes — evict a cache replica → migrate a durable blob to cold → overflow to S3 →
delete only when refcount 0. One module, one continuum, one owner.

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

**Both the DECISION and the enforcement are vfs's — in one process.** For the
local node's slice of a directory, vfs's `policy` child (below) compiles a
`DirPolicy` into an internal gc managed-dir config (budget = the node's share of
`max_bytes`; strategy = the `Eviction` mapped onto the reclaimer; `lock(ttl)` on
freshly-placed durable replicas so they survive the next sweep — the
`make_room → write → register → lock` pattern the built `lib/gc` already
prescribes). The internal gc module runs the sweep and the spectrum mechanics
in-process; the `placement` child supplies the **"which replica is safe to
evict/migrate/overflow"** answer, enforcing concern 4's **one hard invariant:
never drop `actual durable` below the replication factor** (evict `Cache` first;
migrate or overflow a durable blob rather than delete it if deletion would breach
the factor). Because this is now an in-process call, the invariant is a direct
assertion in vfs's own code path, not a cross-service instruction — strictly
tighter than the old WS-edge framing.

**Residual (batch-5 `gc`/inference convergence, out of scope here).** Inference's
own embedded `lib/gc` consumers (`models`/`cache`/`engine`, the `gc-managed-dirs`
edge) are a *separate* embedding of the same library for model/cache dirs — they
are untouched by this absorption and do **not** depend on vfs (no layering
inversion). vfs owns gc for **vfs-managed** directories only; a pure inference
node with no vfs leg embeds `lib/gc` exactly as today. Unifying the two embeddings
into literally one physical per-node store is a `gc`/inference concern flagged in
`gc-managed-dirs` / `gc-events`, not decided here.

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

### 13. Worked example — the walk-along Pi: offline ingest → VFS sync on reconnect (INTENT #34/#48/#157)

The load-bearing story that ties the storage plane to `chassis`'s durable outbox
(chassis concern 6) and the blessing queue (chassis concern 7): **a walk-along Pi
ingests recordings while disconnected from the tailnet, then syncs into VFS on
reconnect — losing nothing.** The Pi is "just a node" (INTENT #34); the design has
two boring profiles, and the interplay is the same shape in both.

**Profile A — the Pi runs a full vfs leg (concern 6, its external drives).** This
is the default when the Pi has storage to contribute:

1. **Offline capture.** The recorder writes each clip to its *local* vfs leg:
   `vfs-content` `Write { path: "vfs://recordings/pi/2026-07-22T14-03.wav",
   class: Immutable, .. }`. Single-port locality (INTENT #58) means the local vfs
   leg is reachable **even with the tailnet down** — the write is a purely local
   operation: content blob lands on the Pi's local drive, `vfs/file/*` +
   `vfs/blob/*` metadata lands in the Pi's **local** replicated-kv copy with
   `actual = [{ node: "pi", .. Durable }]` and `desired` from the directory's
   replication factor (e.g. factor 2, a second holder that is currently
   unreachable). **Offline work works by construction** (INTENT #157) — nothing
   about a `Write` needs the rest of the mesh.
2. **The chassis outbox carries the durable side-band.** vfs wants to *announce*
   each capture so a downstream trigger (transcription, backup) fires — a
   `vfs.recording.captured` pub/sub event marked `save_on_fail` (a **durable**
   send, INTENT #155 no-silent-drops). While offline, chassis parks these in its
   **local durable outbox** (chassis concern 6 — SQLite/append-log, survives both
   daemon outage and a Pi reboot). The bulk recording **bytes never touch the
   outbox** (they are already safe on the local drive as vfs blobs); only the
   small durable *notifications* queue there. That division is the interplay:
   **bulk content rides vfs's pull-convergence plane; durable messages ride the
   chassis outbox — each offline-tolerant, neither overloaded with the other's
   job.**
3. **Reconnect — three channels drain in concert, all idempotent:**
   - **replicated-kv anti-entropy** propagates the Pi's new `vfs/file/*` /
     `vfs/blob/*` metadata to the rest of the mesh (its normal digest-compare
     sync) — now every node knows the recordings exist and where.
   - **pull-driven convergence** (concern 3): the factor-2 `desired` holder, now
     reachable, sees `desired ∋ self && !held` and **pulls** each recording blob
     from the Pi over the relayed `StreamOpen/Chunk/Close` tunnel, writing itself
     into `actual` only after all chunks verify. The recordings are now durably
     replicated — **exactly the offline-node-pulls-its-share-on-reconnect property
     KV anti-entropy already gives metadata, applied to bytes.**
   - **chassis outbox drainer** flushes the parked `vfs.recording.captured` events
     FIFO to the daemon (at-least-once; the transcription trigger dedups on the
     event id, `queues-api` semaphore discipline). No announcement is lost, so no
     downstream work is silently skipped.
4. **No blessing needed for the recordings themselves.** Immutable writes are
   LWW on the path (concern 12) — two nodes writing the same path just pick a
   winner deterministically, both content blobs survive (content-addressed). So
   recordings do **not** enter the blessing queue; the blessing queue (chassis
   concern 7, PARKED OQ-1) is reserved for *consistency-requiring* changes, which
   plain content capture is not. This keeps the hot ingest path free of the parked
   authority question entirely.

**Profile B — the Pi is a thin ingest client (chassis thin-profile, F-5: outbox +
blessing, no service surface).** When the Pi contributes no storage, it links
`chassis` with `features = ["outbox"]` and has **no vfs leg**. Offline, the
recorder stages each clip in the Pi's local chassis-durable store and enqueues a
**durable write-intent** in the outbox addressed to a vfs leg on another node. On
reconnect, the outbox drainer replays each intent as a `vfs-content` `Write`
streamed (relayed tunnel) to that leg — the recordings land in VFS on a storage
node, then converge to their replication factor from there. Same guarantee (lose
nothing), the outbox just also carries the *bytes* because the Pi has nowhere else
durable to keep them until reconnect.

**Which profile a given Pi uses is a deployment choice**, not a fork in vfs: both
ride the same `vfs-content` `Write` + pull-convergence + chassis-outbox
primitives. Profile A is preferred when the Pi has drives (INTENT #48's multi-drive
walk-along node); Profile B when it is a bare capture device. The design commits
to neither globally — the seam is the chassis feature-set, flippable per node.

## Relationships / edges

Contract edges (cross-process WS/wire over mesh-transport `:3649`; client halves
via **`chassis`**):

- **gc** — **NO LONGER a contract edge; absorbed as an internal module (INTENT
  #166 Q9, concern 5).** vfs's `policy`/`placement` children call the internal gc
  module in-process for the enforcement spectrum (mark-for-removal →
  move-between-devices → cold-storage). `scaffold/contracts/vfs-gc.md` is
  **retained as the design record of that module's internal API vocabulary**, not
  a live wire edge. (Inference's separate `lib/gc` embedding keeps `gc-events` /
  `gc-managed-dirs` — those are inference's, not vfs's.)
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
- `service-lookup` — vfs registers (NodeScoped) and resolves aws/secrets/peers
  (gc is no longer resolved — it is in-process, concern 5).
- `locks-api` — vfs consumes for `NodeAnchored` write exclusivity + placement
  transactions (concern 12).

Internal-lib seams (compiled-in, NOT contract edges): **`chassis`** (the
daemon-wrapper — register/resolve/pubsub/kv/locks handles + the durable outbox +
the reconnect/heartbeat/restart client half; `mesh-client` retired into it, ledger
D1), `substrate-types` (`node`/`provenance`/`pubsub`/`error` vocabulary + the new
storage types), and the absorbed **`gc`** module (the built `lib/gc` mechanics
recompiled in-process — the enforcement spectrum, concern 5, NOT a wire edge).

## Nesting

Parent: none (top-level app-crate `bin/vfs` + `lib/vfs`). Children (nested
internal libs, compiled into the vfs daemon, never standalone): **`content`**
(blob store + chunking/hashing + the content-transfer plane), **`placement`**
(the desired/actual replication + tiering engine + reconcile loop), **`policy`**
(DirPolicy → internal-gc config translation), **`gc`** (the **absorbed
enforcement module**, INTENT #166 Q9 — the built `lib/gc` mechanics recompiled
in-process: mark-for-removal / move-between-devices / cold-storage reclaimers +
the managed-dir sweep + `lock(ttl)`), **`perf`** (telemetry sampling +
surface/pubsub publication). These are libraries under the vfs app per INTENT #22
(a crate is an app; its internal pieces are libs), not top-level crates. The
parent/child structure lives here + overview.md, not the directory layout (flat
`components/`).

## Thoroughness level

**implementation-ready** for: the metadata-in-KV / content-out-of-KV split
(concern 1) and the vfs keyspaces; the two file-class model (concern 2); the
content-transfer plane's pull-driven-convergence design + integrity/resume
(concern 3, with the `vfs-content` wire proposed); the desired/actual
drive-aware placement model + the never-evict-below-factor invariant (concern 4);
the DirPolicy → internal-gc translation + the **gc-absorption spectrum** (concern
5, INTENT #166 Q9, SETTLED — mark-for-removal / move-between-devices /
cold-storage in-process); the `DriveInfo`/`NodeStorageTopology` types (concern 6);
access-migration cache replicas (concern 7); the S3-overflow-as-cold-passive-tier
policy (concern 8); the perf/telemetry surface (concern 9); light provenance
(concern 10); the flat path model (concern 11); concurrency/addressing (concern
12); and the walk-along-Pi offline-ingest → reconnect-sync worked example (concern
13, the chassis-outbox / pull-convergence interplay).
**approach-sketched** for: the exact S3 client-side encryption key exchange
(the `vfs-secrets` edge — the client-side content-encryption key vfs resolves from
`secrets` before handing ciphertext to `aws`, concern 8; batch-5 `secrets`
co-design — friction); the NodeAnchored snapshot cadence/coordination with VDB
(the vdb-vfs shape is proposed from vfs's side, its checkpoint semantics reconcile
in vdb); the **direct brokered content tunnel** (concern 3 — AUTHORIZATION PENDING
OQ-30; the relayed path is implementation-ready, the direct path is a marked
one-line-flip seam, not built); and the chunk-size / transfer-window /
reconcile-interval constants (fill-time tuning with proposed defaults, not design
forks).

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
KV keyspaces, DirPolicy → internal-gc translation, the perf surface, the placement
records, the path/scan model — a mid model transcribes these from this file.
**Two surfaces want the test suite written FIRST and must not go to a cheap
tier:** (1) the **content-transfer + pull-convergence loop** (concern 3) — its
failure mode is a blob that reports replicated but isn't, or a transfer that
corrupts silently; write conformance tests over a multi-node in-process harness
(spawn N vfs legs over a fake PeerTransport, assert: a factor-3 file converges to
3 verified holders; an offline node pulls its share on reconnect; a 0-refcount
blob is evicted everywhere; a half-pull never advertises; a chunk hash mismatch
retries not corrupts) *before* implementing; (2) the **never-evict-below-factor
invariant** at the internal placement↔gc seam (concern 4/5) — a property test that
the in-process gc module, given `placement`'s safe-to-evict/migrate/overflow
answer, never drops durable `actual` below `desired`. Everything else (drive
topology, S3 overflow policy, provenance, surface schema) is transcription-grade.
Fill **after** replicated-kv, service-registry, and locks are filled (it rides all
three; gc is now in-process, no external fill dependency) and **alongside** aws's
S3 surface.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `vfs-content` (vfs peer ↔ vfs peer, relayed by mesh) — the bulk content-transfer plane (added by the contract round). → `scaffold/contracts/vfs-content.md`
- `vfs-mesh` — (vfs ↔ mesh) — registration + storage topology + perf. → `scaffold/contracts/vfs-mesh.md`
- `vfs-gc` — **RETAINED as the internal `gc` module's API record, NOT a live wire edge** (gc absorbed into vfs, INTENT #166 Q9 — concern 5). → `scaffold/contracts/vfs-gc.md`
- `aws-vfs` — (aws ↔ vfs) — S3 overflow cold tier (aws.md is the counterpart). → `scaffold/contracts/aws-vfs.md`

Also a party to (authored elsewhere / cross-cutting): `aws-mesh`, `kg-vfs`, `projects-vfs`, `repo-vfs`, `rollup-vfs`, `service-lookup`, `surface-schema`, `vdb-vfs` — see `scaffold/contracts/`. (`gc-events` / `gc-managed-dirs` are **inference's** separate `lib/gc` embedding, not vfs's — see concern 5 residual.)

## Proposed contracts (wave 3)

Wave-3 consolidation changed the *shape* of three seams vfs owns. None invents a
new wire contract; the net effect is one contract retired to internal, one
transport binding re-pointed, and one worked-example dependency made explicit —
all one-line-flippable where a parked question touches them.

1. **`vfs-gc` demoted from wire edge → internal module API (SETTLED, INTENT #166
   Q9).** gc is absorbed as vfs's internal enforcement module (the spectrum:
   mark-for-removal → move-between-devices → cold-storage). The `vfs-gc.md`
   contract file is **retained verbatim as that module's internal API record** but
   is no longer a cross-process contract, no longer resolved via `service-lookup`,
   and no longer carries a mesh-transport binding. The never-evict-below-factor
   invariant (concern 4/5) becomes a direct in-process assertion. *Harmonizer
   note:* `gc.md` should mark its vfs-facing surface as the in-process module
   consumed by vfs (inference's embedding is separate); `service-registry` /
   `service-lookup` drop any vfs→gc resolution.
2. **`vfs-content` transport binding re-pointed to the RELAYED stream tunnel
   (`StreamOpen/StreamChunk/StreamClose`, mesh-transport §5 — authorized
   default).** vfs.md concern 3 now names the relayed one-directional stream
   tunnel as the transport, superseding the earlier "bulk `MsgKind`
   Request/Response with streamed body" phrasing. The **direct brokered peer-link
   tunnel** (bytes off the relay) is **AUTHORIZATION PENDING (OQ-30)** and MUST
   NOT be built until blessed; the frame vocabulary is identical, so the flip is a
   one-line `Address`/mode change. *Harmonizer note:* `vfs-content.md`'s
   "Transport binding" paragraph should be reconciled to `StreamOpen/Chunk/Close`
   to match mesh-transport §5 (which already lists `vfs-content` as riding the
   relayed stream); the schema and integrity/resume design are unchanged.
3. **`vfs-content` `Write` gains the walk-along-Pi offline-outbox interplay as a
   documented consumer (concern 13).** No schema change — the Pi's durable
   write-intents / capture announcements ride **`chassis`'s** durable outbox
   (thin-profile, F-5) while the bulk bytes ride pull-convergence; both drain on
   reconnect. This pins a **`chassis` dependency** (outbox + optional
   blessing-queue seam) that the earlier `mesh-client` framing did not name.

**Consuming (shapes unchanged, dependencies restated):** `vfs-secrets`
(approach-sketched — the client-side content-encryption key from `secrets`, concern
8; batch-5 co-design); `vfs-mesh`, `aws-vfs`, `kg-vfs`, `projects-vfs`, `repo-vfs`,
`rollup-vfs`, `vdb-vfs`, `surface-schema`, `pubsub-protocol`, `restart-protocol`,
`locks-api` — authored elsewhere, consumed as-is.

