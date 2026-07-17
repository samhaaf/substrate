# gateway

**Status:** existing (`bin/gateway`), RESHAPE (multi-node awareness). **Nesting:**
top-level.

The aggregation / observability plane (port 8400): proxies REST to inference and
gc, aggregates their WS event streams into one fan-out hub (`GET /events`) with
per-client topic filtering, and optionally serves the compiled dashboard. In v2 it
depends on mesh: it resolves node endpoints via `mesh-registry-read` instead of
static `inference_url`/`gc_url`, subscribes once per node (`inference-events`,
`gc-events`), and exposes fleet aggregates (`/api/nodes`, `/api/mesh/stats`). The
always-on Pi runs gateway+mesh so the dashboard stays live even when every GPU box
is asleep. Feeds the dashboard via `dashboard-feed`.
