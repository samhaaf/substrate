# models

**Status:** existing (`lib/models`), kept as-is. **Nesting:** child of inference.

Model lifecycle management: registry sync from config, a resumable download
pipeline (`hf:`, `https://`, `file://` sources, `.partial` + rename), and
disk-budget LRU eviction. Note the mesh design flags today's `download_model`
REST path as a no-op stub, which is why Tier-3 (registered-but-not-downloaded)
mesh routing is deferred — a real download path here is the unblock. Edges:
`model-ensure` (scheduler), `gc-managed-dirs` (gc), `store-access`.
