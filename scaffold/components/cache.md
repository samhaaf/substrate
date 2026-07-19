# cache

## Charter
`cache` is the disk-backed KV/prefix cache manager (`lib/cache`). It leverages
llama-server slot save/restore: after prefill, a slot's KV state is written to a
file keyed by `sha256(model_id || ":" || prompt_hash)` and restored on a
matching prefix so prefill is skipped. It enforces a byte budget with TTL+LRU
eviction (via `gc`), indexes entries through `store`, and purges a model's cache
when that model is evicted. Its boundary: it manages KV state ON DISK and the
save/restore protocol with the engine; it does NOT run the backend (`engine`) or
own model weights (`models`).

## Primary design concerns
- **Cache-key correctness is the whole game.** The `sha256(model_id ||
  prompt_hash)` key must be stable and collision-safe across restarts; a stale or
  mismatched restore silently corrupts a completion's context. This is why cache
  is its own component rather than folded into engine.
- **Coupled invalidation with `models`.** When `models` evicts a model, `cache`
  must purge that model's KV entries — a cross-crate invalidation that must not
  leave orphaned files (which would then be gc-swept, or worse, restored). Keep
  the invalidation edge explicit through `store` metadata + `gc-managed-dirs`.
- **Budget logic mirrors `models`** (shared TTL/LRU-over-gc-dirs shape) — a
  candidate for the later shared-library dedup pass; keep symmetric.

## Relationships / edges
- engine <-> cache via `kv-cache` (see scaffold/contracts/kv-cache.md)
- cache -> gc via `gc-managed-dirs` (kv_cache dir; see scaffold/contracts/gc-managed-dirs.md)
- cache <-> store via `store-access` (kv_cache metadata; see scaffold/contracts/store-access.md)

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
implementation-ready — kept as-is; keying, eviction, and store indexing are
grounded in existing code and map directly onto the `kv-cache` contract.

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading the cache-manager wiring step
and the crate's dependency set.

## Suggested fill-model
implementation-ready + moderate complexity -> **cheap-to-mid model OK**. The
model-eviction/cache-purge invalidation is the one spot to fill carefully; the
rest is mechanical against the frozen `kv-cache` contract.
