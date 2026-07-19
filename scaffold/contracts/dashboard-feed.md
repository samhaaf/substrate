# Contract: dashboard-feed

## Parties
mesh  ->  dashboard
*(was `gateway -> dashboard`; gateway merged into mesh, 2026-07-18)*

## What the edge carries
The aggregated `GET /events` WS fan-out (per-client topic filtering) + REST + static
hosting the dashboard renders, envelopes carrying `node_id` so the UI can build its
per-node grid + fleet header. Stable kebab-case element ids for agent-driving.
Round-3: the dashboard is now schema-driven — it discovers services via mesh
and renders each service's component from that service's published surface
schema (see `surface-schema.md`). Schema/example deferred.
