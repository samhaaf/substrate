# models

## Charter
`models` is model lifecycle management (`lib/models`, modules `registry`,
`download`): registry sync from config, a resumable download pipeline (`hf:`,
`https://`, `file://` sources, `.partial` + atomic rename), and disk-budget LRU
eviction (registered against `gc`). Its boundary: it makes model weights present
on disk and tracks their state; it does NOT load a model into a backend (that is
`engine`) or decide when to swap (that is `scheduler`, which asks via
`model-ensure`).

## Primary design concerns
- **The real download path is the unblock for mesh Tier-3 routing.** The
  mesh design flags today's `download_model` REST path (`/v1/models/:id/download`)
  as a no-op stub; a real, resumable download here is what lets the mesh route to
  registered-but-not-yet-downloaded nodes. This is the crate's most load-bearing
  gap and the substance of `model-ensure`'s "ensure available before swap"
  guarantee.
- **Resumability + atomicity under GC.** `.partial` + rename plus registering
  managed dirs with `gc` must interact correctly: a half-downloaded file must
  never be swept as reclaimable, and eviction must not race an in-flight
  download. This is the delicate correctness surface.
- **Budget arithmetic is shared with `cache`.** Both models and cache enforce a
  byte budget via LRU over gc-managed dirs; a later dedup pass may extract the
  shared budget/eviction helper — design the two symmetrically, don't diverge
  their logic gratuitously.

## Relationships / edges
- scheduler -> models via `model-ensure` (see scaffold/contracts/model-ensure.md)
- models -> gc via `gc-managed-dirs` (model weights dir; see scaffold/contracts/gc-managed-dirs.md)
- models <-> store via `store-access` (see scaffold/contracts/store-access.md)

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
approach-sketched — the registry/eviction sides are implementation-ready, but
the download pipeline is the known stub and needs real design (progress/status
surfacing through `model-ensure`, resumability semantics), so the crate as a
whole is one notch below ready.

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading `lib/models` module list, the
model-manager wiring step, and the mesh-design download-stub note.

## Suggested fill-model
approach-sketched + moderate complexity -> **mid-to-strong model**. The download
pipeline (resumability, HF auth, atomic rename, gc-race avoidance) is the part
that justifies the stronger model; registry sync + eviction can be cheap.
