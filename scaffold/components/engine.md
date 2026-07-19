# engine

## Charter
`engine` is the execution engine (`lib/engine`, modules `process`, `slot`,
`client`, `backend`, `provision`): it owns the `llama-server` child process, the
`InferenceBackend` trait seam (with stub seams for future vLLM/MLX/remote
backends), RAII slot accounting for concurrency, SSE completion submission to
the backend, and drain-before-swap. It handles process spawn/health/kill/
restart-on-model-swap and auto-provisions the correct llama.cpp build for the
platform (`BackendProvisioner`). Its boundary: it executes ONE completion on ONE
resident model and manages that backend process; it does NOT decide WHAT to run
or WHEN (that is `scheduler`), does NOT own the KV state on disk (that is
`cache`, which engine calls), and does NOT own model download (that is `models`).

## Primary design concerns
- **Process lifecycle + concurrency accounting is the hard, isolable core** —
  the reason engine earns its own component. Slots are RAII-accounted so a
  panicking completion cannot leak a concurrency slot; the backend process must
  survive completion churn but be drained and killed cleanly on model swap.
- **The `InferenceBackend` trait is the modality/future-proofing seam.** Keeping
  `llama-server` behind this trait is what lets the modality-agnostic `inference`
  crate later add non-llama backends (vLLM/MLX/remote, and eventually
  image/video generators) without touching the scheduler above it. This trait is
  the substance of the `engine-exec` contract.
- **GPU-layer / context-size configuration flows in from `InferenceConfig`** and
  is fixed at process spawn; a model swap reprovisions the process. Restart-on-
  swap correctness (no orphaned processes, no double-bind of `llama_server_port`)
  is the delicate part.

## Relationships / edges
- scheduler -> engine via `engine-exec` (see scaffold/contracts/engine-exec.md)
- engine <-> cache via `kv-cache` (see scaffold/contracts/kv-cache.md)
- engine -> gc via `gc-managed-dirs` (backend binaries dir; see scaffold/contracts/gc-managed-dirs.md)
- engine -> store via `store-access` (see scaffold/contracts/store-access.md)

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
implementation-ready — kept as-is; the crate is grounded and the contracts map
directly onto existing `ExecutionEngine`/`InferenceBackend` code. No redesign
lands here beyond honoring the frozen `engine-exec`/`kv-cache` shapes.

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading `lib/engine` module list and
the `InferenceService::start` engine-wiring step.

## Suggested fill-model
implementation-ready + moderate complexity -> **cheap-to-mid model OK**. Process
lifecycle is fiddly but already implemented; the fill is conformance against the
existing behavior, not novel design.
