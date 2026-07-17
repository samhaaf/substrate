# Contract: dashboard-feed

## Parties
gateway  ->  dashboard

## What the edge carries
The aggregated `GET /events` WS fan-out (per-client topic filtering) + REST + static
hosting the dashboard renders, envelopes carrying `node_id` so the UI can build its
per-node grid + fleet header. Stable kebab-case element ids for agent-driving.
Schema/example deferred.
