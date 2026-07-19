# inference

## Charter
`inference` is the per-machine generation runtime and the crate that IS the
node app: one Cargo crate producing the `inference` daemon (`bin/inference`,
binds `:8420`) plus its embedded in-process Rust API. It wires eight internal
libraries — store, engine, scheduler, models, cache, telemetry, benchmark, api
— into a single-machine service that exposes exactly ONE external contract: the
`/v1/` REST+WS completion surface (`v1-completion-api`). Its boundary: it owns
the *lifecycle and composition* of its subsystems (the `InferenceService::start`
init sequence, crash recovery, pause/resume, event fan-out, prompt-hook seam)
and the node's participation in the mesh (registering its own slug, initializing
its database on a fresh node via `db`). It does NOT own routing/affinity across
nodes (that is `mesh.completion-router`), aggregation/observability across the
fleet (`gateway`), the control-plane Postgres (`db`), or agent management
(`ccd`). The name stays `inference`, not `llm`/`gen`: the crate is deliberately
**modality-agnostic** — future generation modalities (text-to-image,
text-to-video) extend this same crate and its subsystems rather than spinning
out sibling apps, so a text-flavored name would misdescribe its scope; `gen` was
separately rejected as a spoken homophone for "Jen" in a voice-operated project.

## Primary design concerns
- **It is one crate, not eight.** Per the repo philosophy (a CRATE is an app —
  a CLI tool or a daemon; everything else is a LIBRARY nested under the app that
  uses it), the eight subsystems are `lib/*` libraries nested under `inference`,
  NOT sibling top-level crates. The operator confirmed this explicitly ("any
  pieces nested under the inference tool should be left under the inference
  tool"). The reshape rethinks the crate-to-crate CONTRACTS between these
  libraries, not their nesting.
- **The composition root is private wiring, not the assembly seam.**
  `InferenceService::start` (the 10-step init: open store → recover → telemetry
  → gc → models+sync → cache → engine → scheduler → benchmark loop → scheduler
  loop) is this crate's internal wiring and is explicitly NOT the Scaffolding
  wiring seam. The assembly seam is the mesh `service-registry`: at boot the
  node registers its own `slug -> host:port` and resolves dependencies (`db`,
  peers) by slug rather than static URL. Retain a static-config fallback for
  single-box dev so the node runs standalone with no mesh and no `db`.
- **New boot-time dependency on `db` (`db-inference-init`).** When the node
  stands up on a fresh mesh node it initializes its own database through `db`'s
  control plane rather than hand-rolling that bootstrap. This introduces a
  standalone-vs-mesh split that must degrade gracefully — see the tension noted
  under the `store` child (store already self-migrates its embedded SQLite
  schema at `Store::open`), flagged for the operator, not silently resolved.
- **One event bus, fanned out per node.** `InferenceService` owns a
  `broadcast::Sender<LifecycleEvent>` (capacity 256, lossy for slow consumers)
  that the api layer bridges to the WS surface the gateway subscribes to
  (`inference-events`, one subscription per node in v2, envelopes tagged with
  the node's real `node_id`).
- **Shaped-for-maximalist-consumer.** Both `ccd` (agents' `llm-calls`) and, in
  future, `org` route ALL their model calls through this one `/v1/` surface at
  potentially high fan-out volume. Keep completions cheap to submit and the
  entry point singular — do not grow a second completions API.
- **Dedup foresight (for the later shared-library pass, do NOT extract now):**
  three patterns here are plausible future shared libs — (a) the store's
  `Mutex<Connection>` + `StoreObserver` seam versus `db`'s driver layer; (b) the
  reqwest/SSE HTTP-client pattern in `engine`/`models` versus `mesh`'s
  forwarding client; (c) config-from-TOML parsing. Noted so the design doesn't
  duplicate them gratuitously; not gold-plated into shared crates this pass.

## Relationships / edges
- client / mesh.completion-router <-> inference (api) via `v1-completion-api` (see scaffold/contracts/v1-completion-api.md)
- mesh.completion-router -> inference (api) via `node-state-poll` (see scaffold/contracts/node-state-poll.md)
- gateway <- inference via `inference-events` (see scaffold/contracts/inference-events.md)
- ccd (agents) -> inference (api) via `llm-calls` (see scaffold/contracts/llm-calls.md)
- inference -> db via `db-inference-init` (see scaffold/contracts/db-inference-init.md)
- inference (each service) <-> mesh.service-registry via `service-lookup` (the assembly seam; see scaffold/contracts/service-lookup.md)
- {models, cache, engine} -> gc via `gc-managed-dirs` (see scaffold/contracts/gc-managed-dirs.md)
- (internal, node-local) the eight children interconnect via `store-access`, `engine-exec`, `system-state`, `model-ensure`, `kv-cache`, `benchmark-collections`, `api-dispatch`, and the NEW `kernel-confidence` edge — see each child file.

## Nesting (if applicable)
Parent: (none — top-level crate) | Children: [store, engine, scheduler, models, cache, telemetry, benchmark, api]

## Thoroughness level
approach-sketched — the crate boundary, composition root, event model, and the
external contract set are grounded in the real V1 code; the two genuinely-open
items (the `db-inference-init` standalone/mesh split, and the multidimensional
adaptive kernel that reshapes benchmark/scheduler/telemetry) are sketched with a
recommended direction plus flagged open questions rather than fully specified.

## Assigned design-depth
Opus 4.8 — single-agent Component Designer pass (grounded by reading the real
`lib/inference`, `lib/engine`, `lib/scheduler`, `lib/telemetry`,
`lib/benchmark`, and the api/store surfaces on branch V1).

## Suggested fill-model
Mixed by child (this is the design-buys-down-fill lookup, recorded not vibed):
the crate-level orchestration reshape is implementation-ready-adjacent and can
go to a **cheaper model** for the wiring itself, EXCEPT the new
`db-inference-init` boot path, which needs a **strong model** because it crosses
the standalone/mesh boundary and interacts with store's existing self-migration.
Per-child fill recommendations are in each child file; the two that need a
strong model or a Design Mesh pass are `benchmark` and `scheduler` (the adaptive
kernel), with `telemetry` close behind.
