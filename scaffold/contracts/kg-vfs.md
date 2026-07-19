# Contract: kg-vfs

## Parties
`kg` → `vfs`. Over the local mesh daemon `:3649`, never linked. Authored from
`kg.md` concern 5 / its `kg-vfs` proposal (consuming vfs.md's offered
`Exists` surface). Replaces the wave-1 requirements-only stub.

**Scope boundary (load-bearing):** this edge is **node→file pointers only**.
The graph *database files* themselves are NOT on this edge — they reach VFS
**through VDB** (`vdb-vfs`, as NodeAnchored SQLite files). `kg-vfs` carries
only the validation of node props that point AT VFS files.

## Purpose
**FileRef existence validation + body reads.** A KG template declares which
node props are `FileRefField`s; a FileRef value is a `vfs://` path plus an
optional pinned `content_hash`. `kg` validates the pointed-at file's existence
at three moments (write time, sweep time, and — advisory — durability
coupling), and reads body blobs for Obsidian-style note bodies. The invariant:
a broken pointer **flags the node, never deletes it** (push-back / surface,
don't destroy).

## Schema
Consumes vfs.md's kg-facing surface (`types::vfs`); `Hash` is vfs's frozen
SHA-256 absolute-name addressing.

```rust
struct Exists      { path: String }                    // -> ExistsReply
struct ExistsReply { present: bool, content_hash: Option<Hash>, size: Option<u64> }
struct ExistsBatch { paths: Vec<String> }              // -> Vec<ExistsReply> (sweep convenience, note below)
struct ReadBody    { path: String, cache_local: bool } // vfs-content Read for body fetches -> Stream<Bytes>
```

**kg-side semantics pinned here (the enforcement moments):**
1. **Write time** — creating/updating a FileRef calls `Exists`. Absent file →
   `KgError::SchemaViolation` push-back to the calling service — **unless** the
   graph's template sets `allow_dangling_file_refs: true` (the Obsidian
   forward-link idiom), in which case the pointer is written with
   `pointer_state: pending` in props and swept later.
2. **Sweep time** — a `cron-api` job per graph (`kg.sweep.<graph_id>`, cadence
   `GraphPolicy.pointer_sweep`, default 24h, on the home node) re-validates
   every FileRef. Absent → `pointer_state: broken` + `kg.pointer.broken` event
   + a `kg_conflicts` row; the node is **retained**. A pinned `content_hash`
   that no longer matches → `pointer_state: stale` (distinct signal, same
   non-destructive handling). A re-validating pointer clears its flag.
3. **Durability coupling (advisory, v1)** — a `kg` pointer does **NOT** pin a
   file against VFS eviction in v1 (vfs's refcount is manifest-derived; kg
   pointers are not manifests). The sweep catches the consequence. Flagged to
   the harmonizer as a deliberate v1 simplification.

## Error cases
- **None beyond a clean `present: false`** (vfs.md's own stance — non-existence
  is a normal reply, not an error).
- vfs unreachable during a sweep → the **sweep defers** (`kg.sweep.deferred`
  event); pointers keep their last state — a sweep *failure* never flags
  pointers (only a confirmed absence does).

## Version sensitivity
LOW — small additive request/reply shapes; the `Hash` algorithm is frozen by
vfs's contract (SHA-256, absolute names). `ExistsBatch` is additive sugar over
N `Exists` calls that the per-pair harmonizer may drop without breaking
callers.

## Reconciliation notes
- **Only `kg` proposed content; `vfs` offered the `Exists` surface, no
  disagreement.** kg.md consumes vfs.md's concern-11 kg-facing surface verbatim
  (`Exists`/`ReadBody`); this file adds only the `ExistsBatch` sweep convenience
  (flagged as droppable).
- **Deviation from the stub:** the wave-1 stub was requirements-only
  ("KG nodes can point to files… validates existence… on creation and sweeps").
  Fully honored; concretized with the three enforcement moments and the
  non-destructive-flagging rule. The stub did not distinguish the *database
  files* edge (`vdb-vfs`) from the *pointer* edge (this one) — pinned here.
- **Advisory-only durability coupling** is a v1 simplification recorded as a
  friction point (a kg pointer does not refcount-pin a VFS file); the sweep is
  the safety net.

## Example data
World: nodes **macbook** and **pi**; project **demo**; graph **demo-notes**
(`g-0001`, template `obsidian-md@1`, `allow_dangling_file_refs: true`). Node
`n-42` (`note` "Mind OS") points at a markdown body in VFS.

**1. Write-time validation (the body exists):**

```jsonc
// Exists   kg -> vfs   (before writing node n-42's `file` FileRef)
{ "path": "vfs://demo/notes/mindos.md" }
// ExistsReply   vfs -> kg
{ "present": true, "content_hash": "sha256:be7a…", "size": 20480 }
// -> pointer written live; no SchemaViolation.
```

**2. A forward-link to a not-yet-created note (allowed → pending):**

```jsonc
// Exists   kg -> vfs
{ "path": "vfs://demo/notes/roadmap.md" }
// ExistsReply
{ "present": false, "content_hash": null, "size": null }
// allow_dangling_file_refs=true -> node written with props.pointer_state="pending"; swept later.
```

**3. Sweep finds an evicted file (flag, not delete):**

```jsonc
// ExistsBatch   kg -> vfs   (kg.sweep.g-0001 on macbook, 24h cadence)
{ "paths": ["vfs://demo/notes/mindos.md", "vfs://demo/notes/roadmap.md"] }
// -> [ { present:false, … },            // mindos.md was evicted from macbook + not re-fetchable
//      { present:true, content_hash:"sha256:11cc…", size:800 } ]  // roadmap.md now exists
// n-42 -> pointer_state="broken" + kg.pointer.broken event + kg_conflicts row (RETAINED);
// the roadmap node's "pending" clears to live.
```
