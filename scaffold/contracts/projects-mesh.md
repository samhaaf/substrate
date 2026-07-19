# Contract: projects-mesh

## Parties
projects  <->  mesh

## What the edge carries
Registry + surface: projects registers itself with mesh (an instance of
`service-lookup`), pushes project/dashboard registrations to the centralized
registry, and publishes each project's dashboard surface schema so
project-published dashboards are navigable from the main mesh dashboard (see
`surface-schema.md`). Schema/example deferred. **requirements-only** (round-3,
2026-07-18).
