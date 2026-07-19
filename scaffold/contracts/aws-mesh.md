# Contract: aws-mesh

## Parties

`aws` (`bin/aws`, singleton, `AnyNode{aws}`)  ↔  `mesh` (the local `:3649`
daemon: `service-registry` for registration; `replicated-kv` for the cloud
snapshot leg). Both parties proposed complementary halves: `replicated-kv.md`
authors the snapshot **payload** (the kernel-state it exports); `aws.md` authors
the **S3-mechanics** half (how that payload maps onto S3). Not a disagreement —
two halves of one edge.

## Purpose

Two participations:

1. **Registration / resolution** — `aws` registers slug `aws` via `service-lookup`
   through the local daemon (single-port locality, INTENT #58) and is resolved
   `AnyNode{aws}` (consumers don't care which node runs it; it should run where
   internet egress is reliable — the `macbook`, not the `pi`). No new wire; reuses
   `mesh-transport` + the registry.
2. **The replication plane's S3 leg** — mesh periodically exports **encrypted
   snapshots of selected `replicated-kv` keyspaces** to S3 for disaster recovery,
   and a brand-new mesh (zero reachable peers, empty local store) may
   **bootstrap-import** one. `aws` is a **passive, push-only peer** (INTENT #34):
   never consulted during normal sync, never a tiebreak, never required — AWS
   unreachable blocks nothing. In v1 `aws` is a **contract-complete stub**: every
   real op answers `AwsDisabled`.

Because kernel snapshots are **bounded** (kernel state, not GB-scale cold files),
this leg uses **inline ciphertext over the mesh relay** (`TransferPref::Relay`,
Path A) by default — the `aws-vfs` presigned path is available but unneeded here.

## Schema

The snapshot **payload** types are `replicated-kv`'s (`types::mesh`); the **S3
mapping + receipt** types are `aws`'s (`types::aws`). Merged view:

```rust
// ---- replicated-kv -> aws (cron-scheduled inside mesh: default daily + on-demand CLI) ----
struct KvSnapshotExport {                      // replicated-kv owns this shape
    snapshot_id: Uuid, taken_at_ms: i64, node: NodeId,
    keyspaces: Vec<String>,                    // e.g. ["registry/","locks/","vfs/","supervision/"]
    kv_proto: u16,                             // self-describing wire version
    storage_class: S3Class,                    // Standard | Glacier-tier (shared with aws-vfs)
    ciphertext: Bytes,                         // client-side encrypted BEFORE aws sees it
}
// ---- aws -> replicated-kv (S3-mechanics half; aws owns these) ----
struct KvSnapshotReceipt { snapshot_id: Uuid, s3_key: String, etag: String }
struct KvSnapshotMeta    { snapshot_id: Uuid, s3_key: String, node: NodeId,
                           taken_at_ms: i64, kv_proto: u16, storage_class: S3Class }

// bootstrap / operator-driven restore only:
struct KvSnapshotList  {}                 // -> Vec<KvSnapshotMeta>
struct KvSnapshotFetch { s3_key: String } // -> ciphertext (opaque; replicated-kv decrypts + merges)

// s3 key layout OWNED and FROZEN by aws (a restore must find old keys):
//   mesh-kv/<node_id>/<taken_at_ms>-<snapshot_id>
```

`aws` maps `snapshot_id → s3_key` under the reserved `mesh-kv/` prefix, applies
the `storage_class`, stores the ciphertext **opaquely**, and receipts it.
Registration is an ordinary `service-lookup` `Registration { slug: "aws",
addressing: AnyNode /* singleton */, endpoint, meta.requires: ["secrets"] }`.

## Error cases

- `AwsDisabled { adapter: "s3", op }` — **v1 default** (leg designed, not built).
- `ExportFailed { snapshot_id }` — surfaced as `aws.export_failed` /
  `mesh.kv.export_failed`, **retried next schedule, blocks nothing**.
- `SnapshotNotFound` — fetch/list of a missing key.
- `CredentialsUnavailable` — `secrets` couldn't provide `aws`'s AWS **account**
  creds (`aws-secrets` facet 2).
- **Restore-side errors are `replicated-kv`'s, not aws's** — `aws` returns opaque
  ciphertext; `DecryptFailed` / `ProtoTooNew { kv_proto }` / `StoreNotEmpty` (a
  restore into a non-empty store is **refused** — restores are for
  genesis/disaster, not merging) are validated after `aws` hands back bytes.

Domains: `AwsError` (`types::error::aws`, NEW), `MeshError`.

## Version sensitivity

**LOW (opaque, self-describing).**
- **FROZEN:** the `mesh-kv/<node_id>/<taken_at_ms>-<snapshot_id>` **s3 key
  layout** — a restore must locate old keys; additive prefixes only, never a
  rename.
- **ADDITIVE-SAFE:** snapshot payloads embed `kv_proto` and are **self-describing**
  — `aws` treats the body as opaque, so a `kv_proto` bump **never touches this
  contract**; an old snapshot restores through the same merge path as any sync
  (versions are absolute, so a stale snapshot merges harmlessly under newer live
  data). New `S3Class` variants and `KvSnapshotMeta` fields are additive.
- **BREAKING:** changing the s3 key layout or the receipt contract — an `aws`+mesh
  coordinated bump. v1 stub → Live is a per-adapter `Capabilities` flip, not a
  redesign. **SQS-someday** (mesh `queues` → real SQS) is a **future additive
  facet** of this edge, not designed now.

## Reconciliation notes

- **Complementary halves, not a conflict.** `replicated-kv.md` and `aws.md` split
  the edge cleanly: replicated-kv owns the **payload/policy** (what to snapshot,
  when, encryption, the self-describing `kv_proto`); aws owns the **S3 mechanics**
  (key layout, storage class application, opaque storage, receipt). Both are kept;
  neither is a losing position.
- **`KvSnapshotReceipt` — aws's fuller shape wins (trivially).** replicated-kv
  sketched `{ snapshot_id, s3_key }`; aws added `etag`. Adopt aws's
  `{ snapshot_id, s3_key, etag }` (etag lets replicated-kv verify integrity of a
  fetched snapshot). Additive over replicated-kv's; no view dropped.
- **Transfer model consistent with `aws-vfs`.** KV snapshots use the **inline
  Relay** path (bounded state, `ciphertext: Bytes` rides `KvSnapshotExport`);
  `aws-vfs`'s Presigned default does not apply here. Very large snapshots MAY opt
  into `TransferPref::Presigned` (shared vocabulary) — a per-request choice, not a
  contract fork.
- **UNRESOLVED, surfaced (both files flag it): the genesis-key circularity.** The
  client-side encryption key is meant to live in `secrets` (INTENT #48/#98) — but
  `secrets` **rides mesh replication**, so the key needed to restore a *dead* mesh
  cannot live inside the thing being restored. The genesis key must live **outside
  the system** (OS keychain / operator-held file); whether it is *mirrored* into
  `secrets` for runtime rotation is **batch-3's (`secrets` + `aws`) to settle with
  the operator**. Recorded here, not resolved — it belongs to `aws-secrets` /
  `secrets`, not to this reconciler. `aws` itself is unaffected (it stores whatever
  ciphertext it is handed).
- **Deviation from the stub:** the stub was requirements-only (round-9 S3
  supersession note); this contract pins the snapshot leg's merged schema, the
  frozen key layout, the passive-peer semantics, and records the genesis-key
  friction above.

## Example data

macbook's mesh exports a daily kernel snapshot to Glacier; a re-imaged mesh
bootstrap-imports it.

```jsonc
// 1) mesh (macbook) exports selected keyspaces (client-side encrypted)
KvSnapshotExport {
  snapshot_id: "snap-4c9e", taken_at_ms: 1721390400000, node: "macbook",
  keyspaces: ["registry/", "locks/", "supervision/", "vfs/"],
  kv_proto: 3, storage_class: "GlacierFlexible", ciphertext: <bytes>
}
// aws maps + stores opaquely, receipts:
KvSnapshotReceipt { snapshot_id: "snap-4c9e",
  s3_key: "mesh-kv/macbook/1721390400000-snap-4c9e", etag: "\"c7d8…\"" }

// 2) disaster recovery: a re-imaged macbook, empty store, no reachable peers
KvSnapshotList {}  ->
  [ { snapshot_id: "snap-4c9e", s3_key: "mesh-kv/macbook/1721390400000-snap-4c9e",
      node: "macbook", taken_at_ms: 1721390400000, kv_proto: 3,
      storage_class: "GlacierFlexible" } ]
KvSnapshotFetch { s3_key: "mesh-kv/macbook/1721390400000-snap-4c9e" }  -> <ciphertext>
// replicated-kv decrypts (genesis key from OS keychain), merges into the empty store.
// StoreNotEmpty would be raised if the store were NOT empty — restores are genesis-only.

// v1 reality: export/list/fetch all return AwsDisabled { adapter:"s3", op }
//   -> mesh keeps running purely on local + peer replication (its normal v1 state).
```
