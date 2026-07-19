# Contract: service-lookup

## Parties
mesh.service-registry  <->  any device / service (ccd, org, inference, vfs, projects)
*(gateway removed from the party list, 2026-07-18 — gateway merged into mesh;
the dashboard origin is mesh's own surface now)*

## What the edge carries
`register(slug, host:port)` / `resolve(slug) -> endpoint` over a distributed,
eventually-consistent registry. THE single wiring seam: services register + resolve
here instead of using static URLs. Shaped-for note: Org will resolve the whole
service map through this — design resolve() to be cheap and broadly queryable.
Schema deferred.
