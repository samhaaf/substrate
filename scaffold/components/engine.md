# engine

**Status:** existing (`lib/engine`), kept as-is. **Nesting:** child of inference.

The execution engine: owns the `llama-server` child process, slot lifecycle, and
the `InferenceBackend` trait seam (stub seams for vLLM/MLX/remote backends). It
handles process spawn/health/kill/restart-on-swap, RAII slot accounting, SSE
submission, and drain-before-swap. Earns its own component: process-lifecycle +
concurrency accounting is a genuinely hard, isolable concern. Talks to the
scheduler (`engine-exec`), the cache (`kv-cache`), and gc (`gc-managed-dirs`).
