# Contract: repo-vfs

> SUPERSEDES the requirements-only stub. Authored from repo.md concern 1/2 (the
> consumer side) and vfs.md's `repo-vfs` storage-side sketch + concern 2 (the
> `NodeAnchored` file class). Both sides agree — this is the merge.

## Parties

repo (L5 app, `AddressingClass::NodeScoped`) → vfs (L3 app,
`AddressingClass::NodeScoped`)

repo is the consumer; vfs owns the mutable-file host + snapshot-replicator. Both
legs run on the **same anchor node** (all worktrees of a repo co-anchor, concern
1) — repo drives real git against the real OS path vfs hands it.

## Purpose

repo materializes each repository (`.git` object store + one or more worktrees)
as a single **`NodeAnchored` VFS directory tree** under `vfs://repos/<slug>/`,
and drives **real git** (libgit2, no reimplementation) against the real OS path
vfs exposes. Durability is **snapshot-driven at git-natural boundaries** (after a
commit or a completed fetch/pull), NOT on every `fwrite`. The small `.repo/`
manifest (repo's model, concern 2) rides the same anchored tree, so a
node-loss/worktree-migration carries the model *with* the repo — no separate
replication scheme.

## Schema

repo consumes vfs's authored anchor/snapshot surface (identical to `vdb-vfs`):

```rust
// repo -> vfs  (vfs owns these; repo is the client)
struct OpenAnchored { path: VfsPath }
                    // -> LocalPath { os_path: String, anchor_lock: HoldToken }
                    //    real OS path + the exclusive-writer vfs.anchor.<path> lock (a locks-api mutex)
struct Snapshot     { path: VfsPath }
                    // -> Hash   (content-address + replicate the tree as an immutable blob set)
struct Read  { path: VfsPath, cache_local: bool }   // .repo/ manifest reads, immutable object reads
                    // -> ReadReply { bytes: Bytes, content_hash: Hash }
struct Write { path: VfsPath, class: FileClass, replication: Option<u8>, provenance: Option<Provenance> }
                    // -> FileManifest
enum   FileClass { Immutable, NodeAnchored }        // repo working trees are NodeAnchored; git object snapshots Immutable
struct HoldToken(Uuid);                              // the held vfs.anchor lease (types::locks)
```

Lifecycle mapping: **clone** = create a `NodeAnchored` tree + `git2::clone` into
`OpenAnchored.os_path` → `Snapshot`; **commit** = git commit against the anchored
path → `Snapshot`; **fetch/pull** = git fetch → `Snapshot`; **push** = (inject
secrets first, `repo-secrets`) → git push (no snapshot needed; remote is the
durability). repo holds the `anchor_lock` around each git mutation window and
releases it after.

## Error cases

- `AnchorLocked { path }` — another node/op holds the `vfs.anchor.<path>` lock;
  repo surfaces `RepoError::AnchorLocked` (a rare two-nodes-anchored-the-same-repo
  case → repo refuses the second git op).
- `NotAnchoredHere { path, node }` — the tree is anchored on another node; git
  ops MUST run there (concern 1). repo surfaces `RepoError::AnchorElsewhere` and
  routes to the anchoring node (or the caller wants a separate `clone`).
- `OutOfCapacity { node }` — placement can't host the clone; repo picks another
  anchor node.
- `NotFound { path }`.
- `PartitionMergeExceeded` — the anchor lock's partition twin (locks-api, INTENT
  #84) surfaces through vfs's `NodeAnchored` concurrency (vfs.md concern 12);
  repo handles per-operation.
- Transport failures are mesh-transport's, not vfs's — layered.

## Version sensitivity

- **LOW-MEDIUM.** repo consumes vfs's **frozen** anchor/snapshot surface; new
  fields `#[serde(default)]`, `FileClass` reserves `#[serde(other)]`.
- The `.repo/` manifest format versions *inside* the file (a 1-byte format tag),
  opaque to vfs — repo-model churn never bumps the vfs wire.
- The content-hash algorithm (SHA-256) is vfs's frozen content-name — a
  `Snapshot` hash is an absolute name; repo treats it opaquely.

## Reconciliation notes

- **No disagreement — both sides pre-aligned.** vfs.md's storage-side `repo-vfs`
  sketch ("worktrees are `NodeAnchored` mutable trees; git objects are `Immutable`
  blobs; same `OpenAnchored`/`Snapshot` surface as `vdb-vfs` plus ordinary
  `Read`/`Write`") and repo.md concern 1 ("call vfs's `OpenAnchored { path } ->
  LocalPath { os_path, anchor_lock }`, run git against `os_path`, `Snapshot` at
  git boundaries") describe the identical surface. Merged verbatim.
- **`anchor_lock` typed as `HoldToken`** (both sides used the name); pinned to
  `types::locks::HoldToken` so the same held-lease type flows across `repo-vfs`,
  `vdb-vfs`, and the `locks-api` cross-cutting protocol.
- **Snapshot cadence is repo's, not vfs's.** vfs exposes `Snapshot`; *when* to
  call it (git-natural boundaries) is repo's decision (concern 1). vfs.md flagged
  "the NodeAnchored snapshot cadence reconciles in batch 4/6" — resolved here for
  repo: **after commit and after completed fetch/pull.** A crash between snapshots
  loses at most uncommitted working-tree edits — exactly git's own durability
  grain, agreed by both.
- **`ReadReply` shape** pinned (bytes + content_hash) consistently with
  `rollup-vfs` — a shared vfs read reply, reconciled additively across all vfs
  consumers.

## Example data

repo, on **macbook**, clones `github.com/samhaaf/demo` into VFS and adds a `V1`
worktree; both anchor to macbook:

```jsonc
// repo -> vfs: open the anchored tree for the clone (creates it NodeAnchored)
{ "OpenAnchored": { "path": "vfs://repos/demo/" } }
// vfs -> repo
{ "os_path": "/var/vfs/anchored/repos/demo",
  "anchor_lock": "b7e1a0c2-0000-4000-8000-000000000010" }

// repo runs git2::clone into /var/vfs/anchored/repos/demo, then:
// repo -> vfs: snapshot the freshly-cloned tree
{ "Snapshot": { "path": "vfs://repos/demo/" } }
// vfs -> repo  (content-addressed, replicated macbook + pi)
"sha256:4c9e…"

// repo -> vfs: add the V1 worktree on branch v1 (git worktree add under the same .git)
{ "OpenAnchored": { "path": "vfs://repos/demo/worktrees/V1/" } }
// vfs -> repo
{ "os_path": "/var/vfs/anchored/repos/demo-worktrees-V1",
  "anchor_lock": "b7e1a0c2-0000-4000-8000-000000000011" }

// repo writes its model manifest (small, rides the anchored tree, Immutable snapshot at commit)
{ "Write": { "path": "vfs://repos/demo/.repo/repo-meta.json",
             "bytes": "<RepoRecord + WorktreeRecord json>", "class": "NodeAnchored",
             "replication": 2,
             "provenance": { "correlation_id": "c0110000-0000-4000-8000-000000000001",
                             "service": "repo", "node_id": "macbook" } } }
```

The `demo` repo's `WorktreeRecord` for `V1` (branch `v1`, optional thread link)
is the same record referenced by `repo-environments` when `V1` is deployed into
`demo/prod`.
