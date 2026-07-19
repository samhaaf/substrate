# api

## Charter
`api` is the Axum HTTP REST + WebSocket server (`lib/api`, modules `rest`, `ws`,
`wiki`): the inference node's external contract surface. It serves `/v1/`
completions (submit/status/cancel/priority/result/stream), `/v1/collections`,
`/v1/models` (+ `/:id/download`), `/v1/estimate`, `/v1/benchmark/{kernel,run}`,
`/v1/execution/{pause,resume}`, `/v1/system/state`, read-only `/wiki/`, and
`/metrics` + `/health`. It is the node-side terminus of BOTH the client-facing
`v1-completion-api` and the mesh's read-only `node-state-poll`, and it bridges
node lifecycle events to the WS stream mesh's observability plane (formerly gateway) subscribes to
(`inference-events`). Its boundary: it is pure HTTP/WS translation + dispatch; it
holds NO business logic — every route dispatches into scheduler/store/telemetry/
benchmark via `api-dispatch`.

## Primary design concerns
- **One completions surface, shaped for a maximalist consumer.** This is the
  single entry point for clients, mesh-forwarded traffic, `ccd`/agent
  `llm-calls`, and (future) `org`. The mesh forwards it transparently (a client
  cannot tell node-direct from mesh-routed), so request/response shapes must be
  forward-proxy-clean (no node-local assumptions leaking into the wire format).
  Do NOT grow a second completions API for agents.
- **Read vs. request-path separation.** `node-state-poll` (`GET /v1/system/state`,
  `GET /v1/models`) must stay cheap and strictly off the request path — the mesh
  polls it for health/affinity. `ApiState` already separates the poll reads
  (telemetry/store) from submit dispatch (scheduler); keep that split explicit in
  the contract.
- **Event bridge is per-node in v2.** The api bridges the node's
  `broadcast::Sender<LifecycleEvent>` to a WS stream mesh's observability plane subscribes to,
  one subscription per node, envelopes tagged with the real `node_id`
  (`inference-events`). Lossy-on-lag semantics (256-slot broadcast) must be
  visible to mesh so it can resubscribe.
- **`/v1/benchmark/kernel` reshapes with the kernel.** Once telemetry owns the
  multidimensional confidence-aware kernel, this handler stops recomputing its
  own 1-D `output_tokens` curve and instead serves the kernel's real surface —
  an `api-dispatch` change, not new API surface.

## Relationships / edges
- client / mesh.completion-router <-> api via `v1-completion-api` (see scaffold/contracts/v1-completion-api.md)
- mesh.completion-router -> api via `node-state-poll` (see scaffold/contracts/node-state-poll.md)
- mesh <- api via `inference-events` (was gateway; gateway merged into mesh 2026-07-18) (see scaffold/contracts/inference-events.md)
- api -> {scheduler, store, telemetry, benchmark} via `api-dispatch` (see scaffold/contracts/api-dispatch.md)

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
implementation-ready — the route surface is grounded in the real `rest.rs`; the
only change is the `benchmark_kernel` handler serving telemetry's real kernel
instead of a local recompute, which is a dispatch change against an existing
route.

## Assigned design-depth
Opus 4.8 — single-agent pass, grounded by reading the `lib/api` route list and
the `estimate`/`benchmark_kernel` handlers.

## Suggested fill-model
implementation-ready + low-to-moderate complexity -> **cheap model OK**. It is
translation/dispatch against frozen contracts; the only judgment call
(kernel-surface serialization) follows telemetry's kernel design, so fill api
after telemetry.
