# Contract: aws-vfs

## Parties

`vfs` (per-node leg, storage-side consumer)  ↔  `aws` (`bin/aws`, the singleton
AWS virtualization crate, addressed `AnyNode{aws}` — mesh routes to the node with
good egress). Both parties proposed: `vfs.md` concern 8 (the consumer half) and
`aws.md` concern 3/4 (the S3-mechanics owner). **aws owns the S3 surface**
(INTENT #106).

## Purpose

The **S3 overflow cold tier**: VFS moves file object bodies to/from S3
cold-storage classes through `aws` when the personal mesh runs out of capacity or
a directory's `tier_pref` is `ColdFirst`/`S3Overflow`. **VFS decides what
overflows and when** (least-recently-accessed durable blobs under capacity
pressure, or policy-forced cold); **aws does the bytes-to-cloud**. Object bodies
are **client-side encrypted before aws (and S3) ever see them** — `aws` stores
**opaque ciphertext**; the per-object data key is vfs's, resolved from `secrets`
(the `vfs-secrets` edge), and never reaches `aws`. S3 is a **passive cold replica
location** (`BlobPlacement.s3`), never a tiebreak, never required for the mesh to
function (INTENT #34). In v1 `aws` is a **contract-complete stub** — every real op
answers `AwsDisabled`, the normal v1 state (stay local).

## Schema

Canonical shape is **aws's** (`types::aws`); it carries the presigned-vs-relay
transfer negotiation `vfs.md`'s thinner sketch lacked. `ChunkFrame` on the
`Relay` path is **vfs's type** (from `vfs-content`), carrying ciphertext.

```rust
// namespaced object identity; bucket_ns scopes per-node/per-project so encryption
// + lifecycle policy stay stable and isolated. VFS sets key = the blob content_hash.
struct ObjKey { bucket_ns: String, key: String }

enum AwsS3Request {                          // vfs -> aws, over mesh-transport request/response
    PutBegin { key: ObjKey, storage_class: S3Class, size_hint: Option<u64>,
               transfer: TransferPref, idempotency_key: Uuid },      // -> PutBegun
    PutCommit { key: ObjKey, transfer_id: Uuid, sha256: [u8;32] },   // Path A (Relay) finalize
    GetBegin { key: ObjKey, transfer: TransferPref },                // -> GetBegun
    Head     { key: ObjKey },                                        // -> ObjMeta
    List     { bucket_ns: String, prefix: String, cont: Option<String> }, // -> ObjPage
    Delete   { key: ObjKey },
    Restore  { key: ObjKey, tier: RestoreTier },                     // Glacier thaw
}

enum S3Class { Standard, IntelligentTiering, StandardIa,
               GlacierInstant, GlacierFlexible, DeepArchive }        // #[serde(other)] reserved
enum TransferPref { Presigned, Relay }        // concern 3; Presigned default for VFS bulk
enum RestoreTier  { Expedited, Standard, Bulk }

enum Transfer {                               // returned in Put/GetBegun
    Presigned { url: String, headers: Vec<(String,String)>, expires_at_ms: i64 }, // consumer streams direct-to-S3
    Relay     { transfer_id: Uuid },          // then stream ChunkFrame (ciphertext) over mesh-transport
}
struct PutBegun { transfer: Transfer }
struct GetBegun { transfer: Transfer, meta: ObjMeta }
struct ObjMeta  { size: u64, storage_class: S3Class, etag: String,
                  restore: RestoreState, ts_millis: i64 }
enum   RestoreState { Warm, Archived, Restoring { ready_at_ms: Option<i64> } }
struct ObjPage  { items: Vec<ObjMeta>, cont: Option<String> }
```

**Transfer negotiation (concern 3).** `TransferPref::Presigned` (default for the
VFS bulk data path) → `aws` mints a short-lived presigned S3 PUT/GET URL with the
storage class + policy pre-applied; the consumer streams **ciphertext directly to
S3** over its own egress (`aws` never touches the bulk bytes; scales to any object
size). `TransferPref::Relay` → the consumer streams ciphertext `ChunkFrame`s
through the mesh relay to `aws`, which streams them into S3 (strictly on-mesh, at
the cost of crossing the relay twice). VFS records the result in
`BlobPlacement.s3 = ObjKey`.

## Error cases

- `AwsDisabled { adapter: "s3", op }` — **v1 default / adapter off.** The
  well-known, catchable "cloud overflow unavailable, stay local" signal every
  consumer already handles.
- `ObjectNotFound` — Head/Get/Delete of a missing key.
- `InRestore { restore: RestoreState }` — GET on an archived (Glacier) object;
  catchable — the consumer waits for the `aws.s3.restore_ready` event, then
  re-GETs.
- `CredentialsUnavailable` — `secrets` couldn't provide `aws`'s AWS **account**
  creds (concern 2 / `aws-secrets` facet 2); distinct from the vfs **data** key.
- `StorageClassUnsupported`, `ChecksumMismatch` (Relay `PutCommit` sha256 ≠
  stored), `TransferExpired` (presigned URL / relay id expired), `Unreachable`
  (AWS/network down — best-effort, retry next reconcile).
- **None block the mesh:** an overflow that can't push **keeps its mesh replicas**
  and retries; failure surfaces as `vfs.blob.overflow_failed`. Decryption failures
  are **vfs's**, post-fetch — `aws` returns opaque ciphertext.

Domains: `AwsError` (`types::error::aws`, NEW), `VfsError`.

## Version sensitivity

**LOW-MEDIUM.**
- **FROZEN:** `bucket_ns` scoping (baked into the S3 key layout and the encryption
  scope) — renaming a namespace is a **data migration**, not a schema change. The
  content hash used as `ObjKey.key` is a frozen absolute name (`vfs-content`).
- **ADDITIVE-SAFE:** `S3Class`/`RestoreTier` grow as AWS adds tiers (old consumers
  ignore unknown classes on read, `#[serde(other)]`); `TransferPref`/`Transfer`
  are negotiated **per request**, so adding a mode is backward-compatible; new
  `AwsS3Request` variants and `ObjMeta` fields are additive.
- **Opaque bodies** mean encryption/format churn never touches this contract — the
  key rotates in `secrets` without breaking stored-ciphertext addressing.
- **BREAKING:** changing the ciphertext framing on the `Relay` path or the
  presigned-URL contract — gated on an `aws`+`vfs` coordinated bump. Because v1 is
  a stub, the whole surface is "proposed-frozen": it firms fully when the S3
  adapter flips `Stub → Live` (a per-adapter `Capabilities` flag, not a redesign).

## Reconciliation notes

Two proposers; the transfer model is the real disagreement:

1. **Transfer model — aws wins.** `vfs.md` proposed a **pure-relay** shape
   (`S3PutBlob { content_hash, storage_class, ciphertext_chunks: Stream<ChunkFrame> }`,
   `S3GetBlob`, `S3Ref`) — aws always relays the bytes. `aws.md` proposed a
   **negotiated `TransferPref { Presigned, Relay }`**, defaulting Presigned for the
   VFS bulk path (`AwsS3Request` with `PutBegin`/`PutCommit`/`GetBegin`/`Head`/
   `List`/`Delete`/`Restore`). **Resolution: aws's negotiated shape is canonical.**
   Rationale: (a) `aws` **owns** the S3 surface (INTENT #106); (b) pure-relay
   forces every cold byte across the personal mesh relay **twice**
   (consumer→aws-node→S3), loading mesh and the aws node — aws.md's concern 3
   solves exactly this; (c) Presigned keeps `aws` a pure control plane and scales
   to any object size. **Losing position preserved:** vfs's strictly-on-mesh
   relay is retained as `TransferPref::Relay` (a consumer that must not leave the
   mesh requests it) — vfs's view is not dropped, it is the non-default mode. The
   Presigned-default is **CONFIRMED by the operator** (friction-round 1, INTENT
   #114): direct-upload-with-mesh-issued-presigned-permission — the deliberate
   exception to "everything over WebSockets" (INTENT #28) for bulk bytes is
   accepted. Routing note: the mesh, as router, locates the node holding the
   data and tells it to send to the presigned URL; that instruction may ride
   the queue OR a direct service-router path ("queues are for generic
   application logic and operating-system logic" — this flow needn't use one).

2. **`S3Ref` vs `ObjKey`+`ObjMeta` — aws wins.** vfs's `S3Ref { bucket, key,
   storage_class }` is superseded by aws's `ObjKey { bucket_ns, key }` (identity)
   + `ObjMeta` (class/etag/restore/size). `BlobPlacement.s3` holds an `ObjKey`
   (reconciled in `vfs-content.md`). VFS's content-addressing is preserved:
   `ObjKey.key = content_hash`.

3. **`S3Class` — aws's superset wins.** vfs's `{ StandardIa, GlacierInstant,
   GlacierFlexible }` is a subset of aws's full `{ Standard, IntelligentTiering,
   StandardIa, GlacierInstant, GlacierFlexible, DeepArchive }` — adopt the superset
   (additive; VFS chooses a cold class from its eviction policy).

4. **Encryption boundary — both agree, recorded for clarity.** Client-side, vfs
   holds the data key (from `secrets`, `vfs-secrets`), `aws` sees only ciphertext
   (Relay) or nothing (Presigned). The **`vfs-secrets` edge is NEW / not in the
   inventory** — surfaced by `vfs.md` concern 8, flagged here as a dependency of
   this contract (vfs cannot overflow without a provisioned content key;
   `SecretNotFound` → vfs refuses rather than storing plaintext).

5. **Deviation from the stub:** the stub was requirements-only ("client-side
   encryption, keys in secrets"); this contract pins the full negotiated S3
   surface and the passive-tier semantics, and records the transfer-model
   resolution above.

## Example data

macbook overflows the cold pi-resident blob of the model weight to Glacier when
mesh capacity tightens; a later read rehydrates it.

```jsonc
// 1) VFS decides to overflow (durable, LRU-accessed under pressure) — Presigned default
AwsS3Request::PutBegin {
  key: { bucket_ns: "substrate-demo-macbook", key: "sha256:3b1f9e0a…c7" },
  storage_class: "GlacierFlexible", size_hint: 2576980377,
  transfer: "Presigned", idempotency_key: "idem-7f3a" }
PutBegun { transfer: Presigned {
  url: "https://substrate-demo-macbook.s3.amazonaws.com/sha256%3A3b1f9e0a…c7?X-Amz-…",
  headers: [["x-amz-storage-class","GLACIER"]], expires_at_ms: 1721394900000 } }
// VFS streams CIPHERTEXT direct-to-S3; on success records:
//   BlobPlacement.s3 = { bucket_ns: "substrate-demo-macbook", key: "sha256:3b1f9e0a…c7" }
//   and emits vfs.blob.overflowed_s3.

// 2) months later mesh copies were reclaimed; a read needs it -> archived
AwsS3Request::GetBegin {
  key: { bucket_ns: "substrate-demo-macbook", key: "sha256:3b1f9e0a…c7" },
  transfer: "Presigned" }
// -> InRestore { restore: Archived }         // Glacier object, not instantly readable
AwsS3Request::Restore {
  key: { bucket_ns: "substrate-demo-macbook", key: "sha256:3b1f9e0a…c7" }, tier: "Bulk" }
// later: aws.s3.restore_ready event -> re-GetBegin succeeds:
GetBegun { transfer: Presigned { url: "https://…", headers: [], expires_at_ms: 1721481300000 },
           meta: { size: 2576980377, storage_class: "GlacierFlexible", etag: "\"a1b2…\"",
                   restore: Warm, ts_millis: 1721394000000 } }
// VFS pulls ciphertext, decrypts with the secrets-held key, caches local (cache_local).

// v1 reality: every real op above actually returns AwsDisabled { adapter:"s3", op }
//   -> VFS stays fully local (its normal v1 state). Contract passes conformance.
```
