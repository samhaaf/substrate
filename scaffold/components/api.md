# api

**Status:** existing (`lib/api`), kept as-is. **Nesting:** child of inference.

The Axum HTTP REST + WebSocket API server — the inference's external
contract surface (`v1-completion-api`): `/v1/completions` (submit/status/cancel/
priority/result/stream), `/v1/collections`, `/v1/models`, `/v1/estimate`, plus
`/v1/system/state` and read-only `/wiki/` endpoints. It is the node-side terminus
of both the client-facing API and the mesh's `node-state-poll`. Internally it
dispatches into scheduler/store/telemetry/benchmark (`api-dispatch`).
