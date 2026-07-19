# Contract: vfs-content

## Parties

vfs peer (`bin/vfs`, `AddressingClass::NodeScoped`)  ↔  vfs peer (`bin/vfs`),
relayed by the local mesh daemon (`:3649`).  Also: any in-mesh **caller**
(`projects`, `vdb`, `repo`, `rollup`, `kg`, an agent) ↔ its **local** vfs leg,
for the caller-facing read/write API. Sole authoritative proposer: `vfs.md`
(concern 3 — "the biggest new surface"). This pair was **not named in the
wave-2 inventory**; it is surfaced and authored here per the reconciler charter.

## Purpose

Move file **content bytes** between the drives of different nodes reliably —
chunked, integrity-checked, resumable, and **pull-driven** — and expose the
caller-facing read/write of files on top of it. This is the storage plane's bulk
data path. It is deliberately **neither pub/sub** (`pubsub-relay` is lossy and
capped — wrong for a 2.4 GB model weight) **nor a `replicated-kv` value** (bulk
binary is not LWW small state). It rides `mesh-transport`
`Request`/`Response` with a streamed body under a dedicated bulk `MsgKind`,
addressed `Node{N}` (pinned): because file **metadata** is fully replicated
(`vfs-mesh` / the `vfs/*` KV keyspace), the requesting leg already knows *which*
node holds a blob and pulls it in one relayed hop.

The replication engine is **convergent, not imperative**: `BlobPlacement.desired`
(fully-replicated metadata) names who *should* hold a blob; each vfs leg watches
`vfs/blob/*`, diffs its local blob set against desired, and **pulls what it
lacks / hands to `gc` what it must not keep**. No push protocol, no failover
election — a node that was offline pulls its share on reconnect, exactly
mirroring KV anti-entropy.

## Schema

All types land in `types::vfs` (new module); `Hash`, `Provenance` reuse
`types` vocabulary. `Hash` is `"sha256:<64-hex>"` — a **content-addressed
absolute name**.

```rust
pub type Hash = String;                       // "sha256:<hex>" — absolute content name
pub const CHUNK_BYTES: u32 = 4 * 1024 * 1024; // ~4 MiB fixed chunk (fill-time constant)

// ---- caller-facing file API (caller -> its LOCAL vfs leg) ----
pub struct Read  { pub path: String, pub cache_local: bool }        // concern 7 access-migrate
pub struct ReadReply { pub manifest: FileManifest, pub body: Stream<ChunkFrame> }
pub struct Write { pub path: String, pub class: FileClass,
                   pub replication: Option<u8>,                     // None => DirPolicy.default_replication
                   pub provenance: Option<Provenance>,
                   pub body: Stream<ChunkFrame> }
pub struct WriteReply { pub content_hash: Hash, pub manifest: FileManifest }
pub enum FileClass { Immutable, NodeAnchored }                      // concern 2; #[serde(other)] reserved

pub struct FileManifest {                                          // vfs/file/<path> value
    pub path: String, pub class: FileClass,
    pub content_hash: Hash,                                        // rollup hash over chunk list
    pub chunks: Vec<Hash>,                                         // ordered per-chunk hashes
    pub size: u64, pub replication: u8,
    #[serde(default)] pub provenance: Option<FileProvenance>,
    pub v: u16,                                                    // manifest schema version
}

// ---- peer-to-peer content-transfer plane (vfs leg -> holder leg, Node{N}) ----
pub struct ContentPull { pub content_hash: Hash, pub chunks: Option<ChunkRange> } // None = whole blob; range = resume
pub struct ChunkRange  { pub from_index: u32, pub to_index: u32 }
pub struct ChunkFrame  { pub content_hash: Hash, pub index: u32, pub hash: Hash, pub bytes: Bytes }
pub struct PullDone    { pub content_hash: Hash, pub total_chunks: u32 }

// ---- placement metadata (vfs/blob/<content_hash> value; watched to drive convergence) ----
pub struct BlobPlacement {
    pub content_hash: Hash, pub size: u64,
    pub desired: Vec<PlacementTarget>,   // policy target: R durable holders (+ tiers)
    pub actual:  Vec<HeldReplica>,       // verified current holders
    pub s3: Option<ObjKey>,              // cold/overflow copy in aws (aws-vfs); ObjKey is aws vocab
}
pub struct PlacementTarget { pub node: NodeId, pub drive: Option<DriveId>, pub tier: StorageTier }
pub struct HeldReplica {
    pub node: NodeId, pub drive: DriveId, pub tier: StorageTier,
    pub kind: ReplicaKind, pub verified_at: DateTime<Utc>,
}
pub enum ReplicaKind { Durable, Cache }   // Cache = access-migrated (concern 7); never counts toward factor
```

**Transport binding.** `ContentPull` is a `mesh-transport` `Envelope { to:
Address::Node{node, slug:"vfs"}, kind: MsgKind::Request /* bulk */ }`; the reply
streams `ChunkFrame`s as `MsgKind::Response` frames, terminated by `PullDone`.
The bulk `MsgKind` is reliable/ordered/backpressured (the `completion-router`
`forward()` model), distinct from lossy `Publish`.

**Convergence loop (per leg, no wire of its own — reads replicated metadata):**
watch `vfs/blob/*` → for each blob: `desired ∋ self && !held` → **pull** from an
`actual` holder; `held && (refcount==0 || evicted)` → hand to `gc` (`vfs-gc`).
Refcount = `count(FileManifest where content_hash == h)`, computed locally from
fully-replicated metadata — **no distributed counter**. Refcount→0 sets
`desired={}`; every holder's reconcile then evicts.

## Error cases

- `BlobNotHeld { content_hash }` — pulled a node that isn't (yet) a holder; the
  requester re-reads `BlobPlacement.actual` and retries another holder.
- `ChunkHashMismatch { content_hash, index }` — integrity fail; re-request that
  chunk, **never accept**. A blob is written into `actual` only once **all**
  chunks verify (a half-pull never advertises as a replica).
- `TransferAborted { content_hash, at_index }` — link dropped; resume via
  `ContentPull.chunks = ChunkRange { from_index: at_index, .. }`.
- `OutOfCapacity { node }` — target can't hold the pull; the placement engine
  re-picks a target (concern 4).
- `NotFound { path }` — caller read/write of an unknown path.
- `AnchorLocked { path }` — a `NodeAnchored` write without the
  `vfs.anchor.<path>` `locks-api` mutex (concern 12).
- Transport/relay failures (`PeerUnreachable { node }`, `Backpressure`) are
  **mesh-transport's**, not vfs's — layered, catchable, CAP-honest (INTENT #84).

Domain: `VfsError` (`types::error::vfs`).

## Version sensitivity

**MEDIUM.**
- **FROZEN:** the chunk framing and the `Hash` algorithm (SHA-256). A content
  hash is an absolute name; changing the algorithm is a data-renaming the
  contract may never do (same discipline as `replicated-kv`'s frozen `Version`
  order). `CHUNK_BYTES` is a fill-time constant, not wire-negotiated per blob —
  chunk boundaries are recorded in the manifest, so re-chunking is a new content
  hash, not a wire break.
- **ADDITIVE-SAFE:** new fields on `Read`/`Write`/`FileManifest`/`BlobPlacement`
  (`#[serde(default)]`); `FileClass`/`ReplicaKind`/`StorageTier` reserve
  `#[serde(other)]`; a `v: u16` rides the first transfer frame.
- **BREAKING:** a bulk-transfer proto **major** bump (changing frame layout) is a
  `RestartReason::Compatibility` (HIGH-priority) restart. Because chunks are
  content-addressed and verified end-to-end, a **mixed-version fleet transfers
  blobs safely** — bytes are opaque; only the small framing header is
  version-coupled.

## Reconciliation notes

- **Sole proposer.** Only `vfs.md` proposed this edge (the inventory never named
  it). Nothing to reconcile between two parties; authored from `vfs.md` concern 3
  + the batch-1 `mesh-transport` frame it rides. No losing position.
- **`ChunkFrame` is shared with `aws-vfs`.** The same `ChunkFrame` type carries
  ciphertext on the aws-vfs `Transfer::Relay` path. Defined here once; `aws-vfs`
  references it. (aws sees only ciphertext chunks; the plaintext key is vfs's,
  from `secrets` — the `vfs-secrets` edge.)
- **`Read`/`Write` are the caller-facing API**, reused verbatim by
  `projects-vfs`, `rollup-vfs`, and the ordinary leg of `repo-vfs`; the
  `NodeAnchored` open/snapshot surface (`OpenAnchored`/`Snapshot`) is authored on
  the `vdb-vfs` / `repo-vfs` edges, not here — this contract owns the
  Immutable-blob transfer plane and the shared file API.
- **Deviation from the stub:** there is no prior `vfs-content.md` stub; this file
  is created new (the inventory's gap, per `wave2-plan` §3b spirit).
- **`BlobPlacement.s3` uses aws's `ObjKey`**, not vfs's earlier `S3Ref` sketch —
  reconciled toward aws (owner of S3 mechanics); see `aws-vfs.md`.

## Example data

Fresh write of the model weight on `macbook`, converging a factor-2 durable copy
onto `pi`'s cold drive, then a cache-migrating read back to `macbook`.

```jsonc
// 1) macbook: caller (models lib) writes the weight into vfs
Write {
  path: "vfs://models/qwen3-4b.gguf", class: "Immutable",
  replication: 2, provenance: { origin_node: "macbook", origin_service: "vfs" },
  body: <stream of 600 ChunkFrames>          // 2.4 GiB / 4 MiB = 600 chunks
}
WriteReply {
  content_hash: "sha256:3b1f9e0a…c7",
  manifest: { path: "vfs://models/qwen3-4b.gguf", class: "Immutable",
              content_hash: "sha256:3b1f9e0a…c7", size: 2576980377, replication: 2, v: 1 }
}

// 2) vfs/blob/<hash> metadata after the write (fully replicated to macbook + pi)
BlobPlacement {
  content_hash: "sha256:3b1f9e0a…c7", size: 2576980377,
  desired: [ { node: "macbook", drive: "mb-internal-ssd", tier: "Warm" },
             { node: "pi",      drive: "pi-ext-usb-0",    tier: "Cold" } ],
  actual:  [ { node: "macbook", drive: "mb-internal-ssd", tier: "Warm",
              kind: "Durable", verified_at: "2026-07-19T10:00:12Z" } ],
  s3: null
}

// 3) pi's reconcile sees desired∋pi && !held -> pulls from macbook (Node{macbook})
ContentPull { content_hash: "sha256:3b1f9e0a…c7", chunks: null }   // whole blob
// macbook streams: ChunkFrame{index:0..599}, then:
PullDone { content_hash: "sha256:3b1f9e0a…c7", total_chunks: 600 }
// pi writes itself into actual only after all 600 chunks verify -> factor 2 met.

// 4) later: macbook evicted its warm copy; a run needs it -> cache-migrating read
Read { path: "vfs://models/qwen3-4b.gguf", cache_local: true }
// macbook has no local copy -> pulls from pi (Node{pi}) to serve the read AND
// records itself as a ReplicaKind::Cache holder (does NOT raise the factor):
HeldReplica { node: "macbook", drive: "mb-internal-ssd", tier: "Warm",
              kind: "Cache", verified_at: "2026-07-19T18:22:03Z" }

// error path: pi asks macbook for a hash macbook already GC'd
ContentPull { content_hash: "sha256:dead…", chunks: null }
// -> BlobNotHeld { content_hash: "sha256:dead…" }  -> pi retries next actual holder
```
