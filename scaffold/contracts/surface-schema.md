# Contract: surface-schema

## Parties
every service  ->  mesh dashboard

## What the edge carries
The **boring surface schema** (round-3 cross-cutting pattern, 2026-07-18):
every service exposes an endpoint publishing a schema of its observable surface
— a data structure describing (a) how to render its dashboard component and
(b) what calls to make against the service. A shared set of types in `types`
defines this schema language; the mesh dashboard renders every service's
component from its published schema — visual coherence by construction.
Projects use the same mechanism to make project-published dashboards navigable
from the main mesh dashboard. Schema/example deferred. **requirements-only.**
