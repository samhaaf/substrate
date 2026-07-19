# Contract: mesh-registry-read — COLLAPSED (tombstone)

> **This edge no longer exists** (2026-07-18, round-3: gateway merged into
> mesh). It was `gateway -> mesh`: gateway resolving node endpoints and fleet
> state (`/api/nodes`, `/api/mesh/stats`) over HTTP. With gateway's
> observability plane absorbed into mesh, this read is mesh consulting its own
> registry in-process — internal to mesh, not a contract. The `/api/nodes` /
> `/api/mesh/stats` surfaces live on as mesh-served endpoints consumed by the
> dashboard via `dashboard-feed`.
