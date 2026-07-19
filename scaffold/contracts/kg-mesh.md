# Contract: kg-mesh

## Parties
`kg` ↔ mesh. Over the local mesh daemon `:3649` (single-port locality) via
`mesh-client`. Authored from `kg.md` concern 2/4/9 and its `kg-mesh` proposal
(kg is authoritative; mesh contributes the generic protocols it already owns).
Replaces the wave-1 requirements-only stub.

## Purpose
Three sub-surfaces, all riding **existing** mesh protocols:
1. **Registration** — an ordinary `service-lookup` instance (`kg`, NodeScoped,
   `meta.requires: [vdb, vfs]` — supervision boot-order: kg boots after
   mesh/vfs/vdb). Every node's kg leg registers; **graph-home authority is
   kg-level data** (the `GraphDescriptor`), not registry data.
2. **The `kg/` keyspace claim** on `replicated-kv` (the global graph registry +
   template catalog): every node answers "what graphs exist, where does each
   live, what schema does it speak" from a **local KV read** (INTENT #97's
   "access ALL the knowledge graphs" with zero cross-node calls).
3. **Merge-sync frames** — the branch/cloud element-delta stream (kg.md concern
   4 Rule 4), over `mesh-transport` `Request`/`Response` with a streamed body,
   addressed `Node{home}` (the same bulk pattern as `vfs-content`, NOT pubsub,
   NOT KV values).

## Schema
Registry rows and template versions land in the `kg/` keyspace (values are
`types::kg` structs; KV treats them as opaque bytes). Merge frames are
`types::kg`. The HLC `Version` is `replicated-kv`'s frozen `(ts_ms, ctr, node)`.

```rust
// ── (2) the kg/ keyspace (replicated-kv tenant; pubsub_mirror:true, mirror_values:false) ──
//   kg/graph/<graph_id>          -> GraphDescriptor    (LWW; single-writer in practice: the home leg)
//   kg/template/<tid>/<version>  -> TemplateVersion    (IMMUTABLE once published — LWW-safe by construction)
//   kg/template/<tid>            -> TemplateHead        { latest: u32, yanked: Vec<u32> }

// ── (3) merge-sync frames (mesh-transport, Node{home}, streamed body) ──
struct MergeOffer  { graph_id: GraphId, branch_id: BranchId, since: Option<VersionWatermark>,
                     element_count: u64, kg_proto: u16 }
struct MergeDelta  { graph_id: GraphId, elements: Vec<ElementRecord>,  // node/edge rows w/ Version + write_id
                     provenance: Vec<TraceRecord>, done: bool }        // paged; provenance UNIONS (append-only)
struct MergeResult { graph_id: GraphId, applied: u64, conflicts_minted: u32, new_watermark: VersionWatermark }
struct BranchRetire{ graph_id: GraphId, branch_id: BranchId }

struct ElementRecord { subject: Subject /*Node|Edge*/, id: Uuid, element_type: String,
                       props: Value, version: Version, write_id: Uuid, deleted: bool }
```

**Merge rule (kg-semantic, applied by the home leg on each received
`ElementRecord`):** larger `Version` wins (element-wise LWW); tombstones are
writes; inter-element invariant breaks (dangling edge / cardinality / body
divergence) are **quarantined + conflict-minted + published**, never silently
resolved (kg.md concern 4 Rules 1–2). Provenance rows union (uuid-keyed,
conflict-free). Writes whose `write_id` already has an invocation record in the
unioned provenance are NOT re-fired (at-most-once per write across branch/merge).

## Error cases
- `HomeMoved { graph_id, home_node }` — the descriptor changed mid-sync; retry
  against the new home.
- `ProtoTooNew { kg_proto }` — an older home cannot merge a newer branch's
  delta; the merge is **deferred + alarmed, never half-applied** (a wrong merge
  is worse than a late one — deliberately stricter than KV's digest fallback).
- `MergeBusy` — another merge holds `kg.merge.<graph_id>` (`locks-api`); retry.
- `PageTooLarge` — re-page the `MergeDelta`.
- Transport failures (`PeerUnreachable`) are mesh-transport's, not this
  contract's. Keyspace errors wrap `KvError` (incl. `PartialReplication` on a
  descriptor write). `KeyspaceAccessDenied` — only the registered `kg` slug may
  write `kg/`.

## Version sensitivity
HIGH — deltas cross nodes on mixed versions and element records persist.
`ElementRecord` is additive-only with the **`Version` total order
`(ts_ms, ctr, node)` frozen forever** (identical rule and reasoning as
`kv-replication`: reordering is data-corrupting — the one thing the contract
may never do). `kg_proto` rides the first frame; an incompatible major
**defers** the merge rather than degrading. `TemplateVersion` is
content-hashed, so fleet/cloud copies are identity-checkable. Registration is a
plain `service-lookup` instance.

## Reconciliation notes
- **Only `kg` proposed content; mesh contributes generic protocols.** No
  cross-party disagreement on the wire — kg rides `service-lookup`,
  `replicated-kv`, `pubsub-protocol`, and `mesh-transport` as a tenant/party.
- **Deviation from the stub — the S3 leg is REMOVED from this edge
  (supersession, flagged, NOT a dropped requirement).** The wave-1 stub said kg
  "rides mesh's eventual-consistency/replication plane… and into AWS/S3 through
  the `aws` crate." As refined in kg.md concern 9: **graph-file durability to
  S3 is inherited** via `vdb-vfs → aws-vfs` (the SQLite file is a NodeAnchored
  VFS file VFS snapshots/replicates/overflows), and **registry durability** via
  `replicated-kv`'s `aws-mesh` snapshot leg. So `kg` needs **no kg-specific S3
  protocol** — the requirement (KGs eventually consistent with AWS) is fully
  accounted for by inherited legs, not carried on `kg-mesh`. Recorded as a
  simplification for the harmonizer.
- **Deviation — the merge model is kg's, not a generic mesh service (flagged).**
  The stub implied mesh "distributes graph state." kg.md concern 4's honest
  scope note: mesh distributes the *generic* layers (the `kg/` registry
  keyspace; graph-file snapshot replication via VFS; transport for sync frames)
  — but the **graph-semantic merge (Rules 1–2) cannot be a generic mesh
  service** because it is schema-aware (templates, cardinality, quarantine).
  `kg` owns the merge; mesh owns every byte's journey. Recorded, not silently
  rewritten.
- **New pubsub taxonomy row (additive, flagged):** `kg` claims the `kg.*` topic
  prefix (`kg.graph.created`, `kg.node.*`, `kg.edge.*`, `kg.conflict.*`,
  `kg.pointer.broken`, `kg.merge.*`, `kg.sync.*`) — an additive
  `pubsub-protocol` taxonomy row.

## Example data
World: nodes **macbook** and **pi**; project **demo**; graph **demo-notes**
(`g-0001`, template `obsidian-md@1`), home-anchored on **macbook**. `pi` is a
walk-along node whose `GraphPolicy.offline_writes = Allowed`.

**1. The graph descriptor in the `kg/` keyspace (every node reads it locally):**

```jsonc
// kg/graph/g-0001  -> GraphDescriptor   (LWW; home leg is the single writer)
{ "graph_id": "g-0001", "slug": "demo-notes",
  "template": { "template_id": "obsidian-md", "version": 1 },
  "project": "demo", "environment": null,
  "residence": { "Local": { "home_node": "macbook", "vfs_path": "vfs://kg/g-0001/graph.db" } },
  "policy": { "offline_writes": "Allowed", "sync": null, "pointer_sweep": "24h" },
  "status": "Active",
  "created": { "origin_node": "macbook", "origin_service": "kg", "emitted_at": 1752969000000 } }
```

**2. Walk-along branch merge on reconnect.** `pi` went offline, minted a branch
`b-7`, wrote 3 notes; on rejoin it merges into macbook (the home):

```jsonc
// MergeOffer   pi -> macbook   (Node{home=macbook}, mesh-transport)
{ "graph_id": "g-0001", "branch_id": "b-7",
  "since": { "ts_ms": 1752969000000, "ctr": 0, "node": "macbook" },
  "element_count": 3, "kg_proto": 1 }
// MergeDelta   pi -> macbook   (paged; elements carry Version + write_id, provenance unions)
{ "graph_id": "g-0001",
  "elements": [ { "subject": "Node", "id": "n-90", "element_type": "note",
                  "props": { "title": "Offline idea" },
                  "version": { "ts_ms": 1752969500000, "ctr": 0, "node": "pi" },
                  "write_id": "w-90", "deleted": false } /* +2 */ ],
  "provenance": [ { "trace_id": "t-90", "write_id": "w-90", "actor_service": "kg",
                    "actor_node": "pi", "op": "insert", "occurred_at": 1752969500000 } ],
  "done": true }
// MergeResult   macbook -> pi
{ "graph_id": "g-0001", "applied": 3, "conflicts_minted": 0,
  "new_watermark": { "ts_ms": 1752969500000, "ctr": 0, "node": "pi" } }
// BranchRetire   pi -> macbook  { graph_id: "g-0001", branch_id: "b-7" }
```

**3. Stale-home deferral (stricter than KV):** an old macbook build receiving a
newer `kg_proto` MergeOffer replies `ProtoTooNew { kg_proto: 2 }`; `pi` defers
+ alarms, never half-applies.
