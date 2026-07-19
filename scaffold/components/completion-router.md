# completion-router

**Status:** RESHAPE of the original `lib/mesh` core (router/balancer/registry/
proxy). **Nesting:** child of mesh (modules in `lib/mesh`).

## Charter

The transparent completion data plane: it makes the inference *fleet* look like a
single node. It discovers inference nodes (by Tailscale tag), tracks their health
and model inventory, picks a node per request by model affinity, and forwards the
`/v1/` REST+WS surface so a client cannot tell node-direct from mesh-routed. It
owns routing + forwarding only — no completion state (nodes' SQLite is the system
of record; it keeps a rebuildable in-memory `completion_id -> node` index) and no
slug addressing (that is service-registry).

## Primary design concerns

**This child's design is implementation-ready and frozen in
`reports/mesh-design-synthesis.md`.** The load-bearing points:

- **`forward()`'s contract must change** from `Result<Vec<u8>>` to carry status +
  headers + a streaming body (modeled on `bin/gateway/src/proxy.rs` — V1 code; that crate's surviving logic now folds into mesh per the 2026-07-18 gateway merge) — required
  for 404/409/502/503 and streaming. The WS relay for
  `/v1/completions/:id/stream` is a *separate* path from `forward()`.
- **`NodeRegistry`** = discovery-refresh loop (Tailscale via `spawn_blocking`) +
  state/inventory poll loop; `snapshot`/`get`/`candidates_for`. Never discovers on
  the request path. Distinct from service-registry and network-topology.
- **`ModelAffinityBalancer`** = prefer resident tier, least-loaded tie-break;
  **spill and Tier-3 deferred** with reasons (missing `effective_max_concurrent`;
  `download_model` is a no-op). `select` takes enriched `NodeCandidate`s, not bare
  `NodeEndpoint`.
- **Pinning** (`X-Substrate-Node` / `?node=`) resolved above the balancer; mesh
  strips `?node=` before forwarding to preserve byte-transparency.
- **The mesh registers the `inference` slug -> its own `:3649`** (port LOCKED rounds 4–5; was `:8419`) in
  service-registry, so consumers resolve `inference` and reach this front door.

## Relationships / edges

- inference (api) via `v1-completion-api` — transparent forward of the `/v1/`
  surface (scaffold/contracts/v1-completion-api.md)
- inference (api) via `node-state-poll` — NodeRegistry health/inventory polling,
  off the request path (scaffold/contracts/node-state-poll.md)
- tailscale-query via `tailscale-status` — fleet discovery input
  (scaffold/contracts/tailscale-status.md)

## Nesting

Parent: mesh | Children: none. Modules `router`, `balancer`, `proxy` (+ the
`NodeRegistry`) in `lib/mesh`.

## Thoroughness level

implementation-ready — trait signatures, status-code semantics, deferrals, and
OQ-1..OQ-11 are all fixed in `reports/mesh-design-synthesis.md`.

## Assigned design-depth

**Design Mesh** — `reports/mesh-design-synthesis.md` (three-stage propose →
critique → synthesize over the real V1 code).

## Suggested fill-model

implementation-ready + high complexity → **cheaper model OK**. The synthesis is a
near-spec; fill is largely transcription against the live code it already cites.
