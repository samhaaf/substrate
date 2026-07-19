# Contract: rollup-vfs

> Authored from rollup.md concern 6 / its `rollup-vfs` proposal (consumer side)
> and vfs.md's storage-side sketch for `rollup-vfs` (vfs is the target party).

## Parties

rollup (L4 app) → vfs (L3 app, `AddressingClass::NodeScoped`)

rollup is a plain consumer of vfs's file surface (the `FragmentStore` production
impl, `VfsFragmentStore`). vfs owns the bytes; rollup imposes only a layout
convention.

## Purpose

Fragment/plugin **residence**: rollup reads versioned fragment files and lists
fragment versions from VFS, and writes materialized `VfsDir` plugins. Fragments
are `Immutable` content-addressed VFS files (vfs.md concern 2) under the layout
convention `<scope.prefix>/prompts/<id>/<version>.md` (`<version>` an integer;
`latest` = max). rollup depends on **VFS + mesh + secrets only** in v1 — NOT VDB
(the fragments-in-DB cascade is the flagged v2 path, rollup.md concern 8).

## Schema

rollup consumes vfs's authored file surface (the `vfs-content` `Read`/`Write`
verbs) plus a prefix-`List` and an `Exists`. Reconciled naming — vfs's verbs, not
rollup's `Vfs*` aliases:

```rust
// rollup -> vfs  (vfs owns these; rollup is the client)
struct Read  { path: VfsPath, cache_local: bool }   // -> ReadReply { bytes: Bytes, content_hash: Hash }
struct List  { prefix: VfsPath }                     // -> Vec<VfsEntry>   (version discovery)
struct Exists{ path: VfsPath }                       // -> ExistsReply { present: bool, content_hash: Option<Hash>, size: Option<u64> }
struct Write { path: VfsPath, bytes: Bytes,
               class: FileClass,                      // rollup writes Immutable
               replication: Option<u8>,
               provenance: Option<Provenance> }       // -> FileManifest
enum   FileClass { Immutable, NodeAnchored }          // rollup uses Immutable only
struct VfsEntry { path: VfsPath, content_hash: Hash, size: u64 }
```

The `content_hash` vfs returns on `Read` is exactly what rollup records per
`FragmentUse.content_hash` for reproducibility (rollup.md concern 9). Reads may
trigger vfs's access-can-migrate caching (`cache_local`, vfs.md concern 7) —
transparent to rollup.

## Error cases

- `Vfs(String)` — path not found, node unreachable, replication gap; stringified
  at the boundary (vfs's error type never becomes a rollup type, types.md
  guardrail). A missing fragment **version** surfaces on rollup's side as the
  ported `PromptNotFound` / `PinnedVersionNotFound`, not a raw vfs error.
- `OutOfCapacity { node }` on a materialize-to-`VfsDir` write → placement re-picks
  a node (vfs's concern; rollup just retries the write).
- `AnchorLocked` is **not applicable** — rollup only writes `Immutable` files (no
  `NodeAnchored` anchor lock).

## Version sensitivity

- **Additive-safe / LOW:** vfs's read/list/write surface is boring and stable;
  new fields `#[serde(default)]`, `FileClass` reserves `#[serde(other)]`.
- **Frozen upstream:** the content-hash algorithm (SHA-256) is vfs's frozen
  content-name (vfs.md `vfs-content` version-sensitivity) — a content_hash is an
  absolute name. rollup treats `content_hash` as opaque.
- The **fragment layout convention** (`<scope.prefix>/prompts/<id>/<version>.md`)
  is rollup's, versions *inside* the path, and never bumps the vfs wire.

## Reconciliation notes

- **Naming reconciled to vfs's verbs.** rollup.md sketched `VfsRead`/`VfsList`/
  `VfsWrite`/`VfsExists`; vfs.md's storage side named `Read`/`Write` (from
  `vfs-content`) and `Exists` (shared with `kg-vfs`). Adopted vfs's names — vfs is
  the surface owner. rollup's `Vfs*` prefixes were just consumer-side aliases.
- **One real gap closed: `List`/prefix-scan.** rollup requires listing a
  fragment directory's versions (`FragmentStore::list_versions` → pick max
  integer). vfs.md concern 11 states "listing a directory is a prefix scan over
  `vfs/file/*`" but the `vfs-content` sketch exposed only `Read`/`Write` — no
  explicit `List` verb. Reconciliation: **vfs's file surface gains a `List {
  prefix } -> Vec<VfsEntry>`** (a cheap, local, fully-replicated-metadata scan —
  no bytes move), satisfying rollup's version-discovery need. Flagged to vfs's
  fill so the storage-side surface includes it (kg/projects will want it too).
- **No behavioral disagreement** — both sides agree rollup is "a plain client, no
  new bulk wire" (vfs.md). The only additions are the `List` verb and the
  content-hash-for-provenance guarantee, both boring.
- **`Read` return shape:** vfs's `vfs-content::Read` sketch didn't pin a reply
  struct; pinned here as `ReadReply { bytes, content_hash }` so rollup gets the
  hash without a second `Exists` round-trip. Reconciled additively.

## Example data

rollup, on **macbook**, resolves the `code-review` fragment for the `demo`
project (from `rollup-ccd`'s assembly). It lists versions, reads the latest, and
records the content hash:

```jsonc
// rollup -> vfs: discover versions of the fragment
{ "List": { "prefix": "vfs://prompts/demo/prompts/code-review/" } }
// vfs -> rollup
[ { "path": "vfs://prompts/demo/prompts/code-review/3.md", "content_hash": "sha256:71bd…", "size": 2140 },
  { "path": "vfs://prompts/demo/prompts/code-review/4.md", "content_hash": "sha256:9a1f…", "size": 2210 } ]

// rollup -> vfs: read latest (version 4), pull it local for reuse
{ "Read": { "path": "vfs://prompts/demo/prompts/code-review/4.md", "cache_local": true } }
// vfs -> rollup
{ "bytes": "<fragment markdown bytes>", "content_hash": "sha256:9a1f…" }

// later — materialize a durable shared plugin into VFS (OutputSink::VfsDir):
// rollup -> vfs
{ "Write": { "path": "vfs://plugins/demo-agent/skills/code-review/SKILL.md",
             "bytes": "<resolved skill bytes>", "class": "Immutable",
             "replication": 2,
             "provenance": { "correlation_id": "c0110000-0000-4000-8000-000000000001",
                             "service": "rollup", "node_id": "macbook" } } }
// vfs -> rollup: FileManifest (content-addressed, placed on macbook + pi)
{ "path": "vfs://plugins/demo-agent/skills/code-review/SKILL.md",
  "class": "Immutable", "content_hash": "sha256:9a1f…", "size": 2210, "replication": 2 }
```

The `content_hash sha256:9a1f…` matches the one recorded in `rollup-ccd`'s
provenance — the same fragment, exactly reproducible.
