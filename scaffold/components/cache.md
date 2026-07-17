# cache

**Status:** existing (`lib/cache`), kept as-is. **Nesting:** child of inference.

The disk-backed KV/prefix cache manager. Leverages llama-server slot
save/restore: after prefill, a slot's KV state is written to a file keyed by
`sha256(model_id || ":" || prompt_hash)` and restored on a matching prefix,
skipping prefill. Enforces a byte budget with TTL+LRU eviction, indexes entries
via the store, and purges a model's cache when that model is evicted. Edges:
`kv-cache` (engine), `gc-managed-dirs` (gc), `store-access`.
