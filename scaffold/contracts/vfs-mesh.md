# Contract: vfs-mesh

## Parties

`vfs` (`bin/vfs`, one per storage node, `AddressingClass::NodeScoped`)  ↔
`mesh` (the local `:3649` daemon: `service-registry`, `replicated-kv`,
`network-topology`). Proposers: `vfs.md` (authoritative, the registration +
storage-topology + perf half) and the batch-1/2 seams it rides
(`service-lookup`, `node.rs`, `pubsub-protocol`, `surface-schema`).

## Purpose

Make a vfs leg a first-class mesh citizen and publish the **node/drive storage
topology** every other leg needs to place replicas. Three strands:

1. **Registration / resolution** — vfs registers slug `vfs` (NodeScoped: one
   instance per node) via `service-lookup`; callers resolve `AnyNode{vfs}`
   (locality-preferred: your local leg) for ordinary reads/writes and
   `Node{N, vfs}` (pinned) to force a specific node's storage.
2. **Storage-topology publication** — vfs advertises its drives
   (`NodeStorageTopology` / `DriveInfo`) so the placement engine on **any** node
   can target `(node, drive)` pairs (warm/cold RAID-ish tiers — the walk-along Pi
   with external drives, INTENT #48). This rides node capabilities in the
   fully-replicated registry/`replicated-kv`, so any leg reads it locally.
3. **Perf reporting** — vfs publishes inference-style self-tracked performance
   (uptime, read/write latency, per-drive fill, replication lag) over
   `surface-schema` (dashboard) + the `vfs.*` pub/sub topic prefix.

The **S3-overflow leg does NOT ride this edge** — it rides `aws-vfs`. This edge
carries no bulk bytes (that is `vfs-content`); it is control-plane +
metadata-topology only.

## Schema

Storage-topology types land in `types::node` per `types.md`'s stated inclusion
plan (they were deferred there "until a real consumer and the `vfs-mesh` contract
name them" — that condition is now met):

```rust
pub type DriveId = String;                 // stable per-drive id (serial/uuid)
pub enum StorageTier { Warm, Cold }        // #[serde(other)] reserved
pub enum DriveMedia { InternalSsd, InternalHdd, ExternalSsd, ExternalHdd, Network, Other }
pub struct DriveInfo {
    pub drive_id: DriveId, pub node: NodeId,
    pub label: String,                     // "pi-ext-usb-0"
    pub tier: StorageTier,                 // operator-assignable per drive
    pub media: DriveMedia,
    pub mount_path: String,                // where vfs stores blobs on this drive
    pub capacity_bytes: u64,
    pub used_bytes: u64,                   // refreshed by perf sampling
    pub writable: bool,
}
pub struct NodeStorageTopology { pub node: NodeId, pub drives: Vec<DriveInfo> }

// additive extension to the existing NodeCapabilities (types::node):
pub struct NodeCapabilities {
    // …existing roles / accelerator / models_available…
    #[serde(default)] pub storage: Option<NodeStorageTopology>,   // NEW (additive)
}
```

**Registration** is an ordinary `service-lookup` call (no new wire):

```rust
Registration {
    slug: "vfs",
    addressing: AddressingClass::NodeScoped,
    endpoint: Endpoint { scheme: "ws", host: "127.0.0.1", port: 8431, health_path: "/health" },
    meta: RegMeta { requires: ["gc", "aws"?, "secrets"?] },       // soft deps for boot order
}
```

**Perf + topology publication** ride existing cross-cutting protocols (authored
by their owners, consumed here): `surface-schema` (vfs's boring panel:
dirs/placement/perf tables + sweep/rebalance actions) and `pubsub-protocol`
(`vfs.*` topic: `vfs.file.placed`, `vfs.file.evicted`, `vfs.blob.pulled`,
`vfs.blob.overflowed_s3`, `vfs.replication.degraded`, `vfs.drive.full`).

## Error cases

- Registration/resolution errors are `service-lookup`'s
  (`MeshError::NoSuchSlug`, `RegistryError::LeaseExpired`, …) — not re-minted
  here.
- A node advertising a drive that is gone at placement time surfaces on the
  **content plane** as `OutOfCapacity`/`vfs.drive.full` (`vfs-content`), not on
  this edge.
- Stale topology (a peer offline when a drive was added) resolves through the
  same `replicated-kv` anti-entropy/tombstone path as any capability update —
  inherited, not re-solved.

## Version sensitivity

**MEDIUM** — `NodeStorageTopology`/`DriveInfo` cross nodes (anti-entropied with
node capabilities).
- **ADDITIVE-SAFE:** `#[serde(default)] pub storage` on `NodeCapabilities`; new
  `DriveInfo` fields with `serde(default)`; new `StorageTier`/`DriveMedia`
  variants under `#[serde(other)]`. A newer node advertising a tier an older peer
  doesn't understand **degrades conservatively to `Warm`** (never silently
  treated as cold).
- **BREAKING:** removing/renarrowing a `StorageTier` variant, or making `storage`
  non-optional — both gated on a `types` major + a `Compatibility` rolling
  restart.
- Registration wire is `service-lookup`'s floor; this edge adds only data types,
  so its version exposure is the topology types, not a new protocol.

## Reconciliation notes

- **Single strong proposer.** `vfs.md` owns this edge's shape; the mesh side
  contributes only already-frozen batch-1/2 seams (`service-lookup`,
  `types::node`, `pubsub-protocol`, `surface-schema`). No competing proposal, so
  no losing position to record.
- **Deviation from `types.md`:** `types.md` explicitly *deferred* `DriveInfo`/RAID
  topology out of `node.rs` "until vfs is designed and `vfs-mesh` names it." This
  contract is that trigger — the types are defined here and land in `types::node`
  additively (crate-root re-exports keep existing imports compiling). Recorded as
  the intended, not speculative, inclusion.
- **Addressing:** vfs is `NodeScoped`, **not** an `AnyNode` fleet like
  `inference` — a caller's ordinary read/write prefers its *local* leg, and a
  specific node's storage is reached `Node{N}`. This mirrors gc's per-node
  singleton stance (`vfs-gc` / `gc.md` concern 7); confirmed consistent across
  the storage plane.
- **S3 re-routing (carried from the stub):** the earlier note that an S3 leg once
  rode this edge is retired — S3 overflow is `aws-vfs`; this edge is topology +
  registration + perf only.

## Example data

```jsonc
// pi registers its cold-storage topology (two external USB drives)
Registration {
  slug: "vfs", addressing: "NodeScoped",
  endpoint: { scheme: "ws", host: "127.0.0.1", port: 8431, health_path: "/health" }
}
// pi's NodeCapabilities.storage, anti-entropied to every node's replicated-kv:
NodeStorageTopology {
  node: "pi",
  drives: [
    { drive_id: "pi-ext-usb-0", node: "pi", label: "pi-ext-usb-0", tier: "Cold",
      media: "ExternalHdd", mount_path: "/mnt/cold0/vfs", capacity_bytes: 4000787030016,
      used_bytes: 2576980377, writable: true },
    { drive_id: "pi-ext-usb-1", node: "pi", label: "pi-ext-usb-1", tier: "Cold",
      media: "ExternalHdd", mount_path: "/mnt/cold1/vfs", capacity_bytes: 4000787030016,
      used_bytes: 0, writable: true }
  ]
}
// macbook's leg (control+inference+storage) advertises one warm SSD:
NodeStorageTopology {
  node: "macbook",
  drives: [ { drive_id: "mb-internal-ssd", node: "macbook", label: "mb-internal-ssd",
              tier: "Warm", media: "InternalSsd", mount_path: "/Users/op/.substrate/vfs",
              capacity_bytes: 1000204886016, used_bytes: 341123006464, writable: true } ]
}
// the placement engine on ANY node now reads both topologies locally and targets
// a factor-2 durable copy of qwen3-4b.gguf at (macbook, mb-internal-ssd, Warm)
// + (pi, pi-ext-usb-0, Cold) — see vfs-content example.

// live perf event on the vfs.* topic when the pi copy lands:
Envelope { topic: "vfs/pi/events",
           payload: { kind: "vfs.file.placed",
                      path: "vfs://models/qwen3-4b.gguf", node: "pi",
                      drive: "pi-ext-usb-0", tier: "Cold" } }
```
