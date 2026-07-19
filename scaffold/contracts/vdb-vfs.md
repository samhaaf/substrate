# Contract: vdb-vfs

## Parties
`vdb` (L4 daemon, per-node leg) `<->` `vfs` (L3 storage plane, local leg),
over the local mesh daemon `:3649`.

**Renamed from `stack-vfs` at wave-2 harmonization** (wave2-plan §5.1 flag;
vdb.md's naming resolution: `vdb` is the crate/module, "stack" survives as
the PATTERN name). `scaffold/contracts/stack-vfs.md` is a rename-tombstone
pointing here. The rounds-4–5 requirements-only stub was never upgraded by
the contract round (a coverage gap); this file is the authored minimal
contract, from `components/vdb.md` §`vdb-vfs` (which ACCEPTS vfs.md's
proposed anchor/snapshot surface, one refinement).

## Purpose
The layering edge (**VFS < VDB**, INTENT #96): each local database's single
SQLite file is a **NodeAnchored vfs file**. vfs is the file host +
snapshot-replicator; **vdb is the exclusive owner-writer** (the single-writer
rule for mesh-managed SQLite). The database inherits VFS per-directory
policies/replication through this edge.

## Schema (accepted surface — authoritative sketch in vdb.md/vfs.md)
```rust
// vdb -> vfs (local leg)
struct OpenAnchored { path: String }                    // -> LocalPath { os_path, anchor_lock: HoldToken }
struct Snapshot     { path: String, consistent: bool }  // -> { content_hash: Hash }
    // REFINEMENT (vdb->vfs): `consistent: true` asserts the caller holds a
    // quiesced WAL checkpoint; vfs records snapshot_of + provenance-lite.
struct Retire       { path: String, final_snapshot: bool } // release anchor; normal lifecycle
```
vdb calls `Snapshot` only inside a `wal_checkpoint(TRUNCATE)` window, so every
replicated snapshot is transaction-consistent — asserted on the wire, not
implicit in discipline. Anchor relocation (moving a database's home node)
composes existing verbs: quiesce → `Snapshot` → release anchor →
`OpenAnchored` on the new node → catalog + registry update; no new wire.
S3 replication is inherited via `vdb-vfs → aws-vfs` (the SQLite file is an
ordinary vfs blob to the overflow tier).

## Error cases
`AnchorLocked` (another holder — a second vdb leg must NOT host this db; the
split-brain guard), `NotAnchoredHere` (anchor lives elsewhere — resolve the
entity and relay, or relocate), `SnapshotFailed`. Partition twins on the
anchor lock surface locks' merge error; vdb's per-application handling: the
LWW-newest anchor holder wins, the loser demotes to read-only and alarms —
split-side writes are surfaced for operator reconciliation, never silently
merged (flagged; friction report).

## Version sensitivity
LOW–MEDIUM — node-local calls (vdb and its vfs leg co-reside); the snapshot
`content_hash` rides vfs' frozen SHA-256 addressing. Additive fields only.

## Reconciliation notes
- vfs.md proposed the anchor/snapshot surface; vdb.md accepted it with the
  `consistent: true` refinement — no dispute.
- Whether v1 ships anchor relocation is an open question recorded in vdb.md.
- The `stack-vfs`→`vdb-vfs` rename is performed here per vdb.md's
  recommendation (the rest of the contract corpus already references
  `vdb-vfs`); operator sign-off pending in the friction report.
