# Contract: projects-mesh

## Parties
- `projects` (L6 stub) `<->` `mesh` (L1/L2).

*(Stub-track / existing stub, refit in place. Composed entirely of already-
authored cross-cutting edges — no bespoke routing. Content deferred until
`projects` leaves the stub track.)*

## Purpose
Registration + surface: `projects` registers itself with mesh (an instance of
`service-lookup`), pushes project/dashboard registrations to the centralized
registry, and publishes each project's dashboard surface schema so project-
published dashboards are navigable from the main mesh dashboard (INTENT #51;
`surface-schema`, `dashboard-feed`).

## Rough shape
No new mechanism — three existing edges bound together:
- **`service-lookup` instance:** `register(slug="projects", host:port)` /
  `resolve(..)` via `chassis` (formerly `mesh-client`), like every service.
- **Centralized registry push:** a `projects/` keyspace in `replicated-kv`
  (flagged) holding project + dashboard registrations, so any node can enumerate
  registered projects and their dashboards.
- **`surface-schema` publication:** `projects` publishes a `SurfaceSchema` per
  project dashboard; `dashboard-serving` folds them into the `DashboardManifest`
  as `NavKind::Project` entries, rendered through the **identical**
  `SchemaRenderer` (no projects-specific frontend code — the hook in
  dashboard.md concern 8). **Aggregation never becomes routing**
  (`dashboard-serving.md`): project dashboards are surfaced, not proxied through a
  bespoke path.

## Open questions
- The `projects/` registry keyspace shape (flagged: replicated-kv layout for
  project + dashboard registrations).
- Whether nested projects publish nested `NavEntry` trees or flat entries with a
  parent ref — leaning nested `children` on `NavEntry`.
- Whether project-dashboard `SurfaceSchema`s are versioned per-project or share
  the fleet `SurfaceSchema.v` discipline.
