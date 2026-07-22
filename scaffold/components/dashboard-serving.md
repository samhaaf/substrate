# dashboard-serving — FOLDED INTO mesh-core (tombstone-with-content)

> **⚠ TOMBSTONE — `dashboard-serving` IS NO LONGER A STANDALONE COMPONENT
> (wave-3 ledger batch plan D5/F9b; INTENT #46; seed-bishop critic-loop synthesis
> ACCEPT[BORING] F9b — "dashboard-serving → mesh-core HTTP + kv reads").** The
> observability serving plane was already a compiled-in Ring-4 internal lib of
> mesh (`lib/mesh::dashboard`); wave 3 completes the consolidation by folding its
> **design content** into `mesh-core.md` so there is no separate component file
> to keep in sync. There is no standalone `dashboard-serving` crate, app, or
> service. Its four jobs now live as **`mesh-core.md` Concern 12 ("The
> observability serving plane")**, and the browser-HTTP listener seam it required
> is now a first-class mesh-core Ring-0 seam (`trait HttpSurface`). See
> `scaffold/components/mesh-core.md` (Concerns 1, 12, and the boot sequence) for
> the current, authoritative design; this file is retained as the record of what
> moved and why.

## Why it folded

dashboard-serving never opened its own socket — mesh-core's shell was always going
to own the listener and hand it a router-mount seam (the co-batch friction point
the wave-2 design flagged). With the listener, the port acquisition, the registry
`dashboard`-slug registration, and the boot ordering all owned by the mesh-core
shell, and the feed/aggregation riding `pubsub-relay` + `service-registry` +
`replicated-kv` (all mesh-internal), the serving plane is most honestly documented
*inside* the kernel that hosts it. F9b makes that documentation move official:
mesh-core HTTP surface + KV reads, one component. The old friction flag ("mesh-core
must confirm the browser-HTTP listener + router-mount seam") is thereby **resolved
in-file** — it is now an mesh-core Ring-0 decision, not a cross-component ask.

## What moved into mesh-core (content summary)

Everything dashboard-serving owned survives, relocated to the `dashboard` module
documented in `mesh-core.md` Concern 12, unchanged in substance:

- **Job 1 — Static asset origin.** Serve `ui/dashboard/dist/` from one
  browser-reachable HTTP origin per node via the shell's **`HttpSurface`**
  listener (a SEPARATE port from the `:3649` mesh-transport floor — a browser
  cannot speak the `Hello`/`Welcome` handshake, and `mesh service open dashboard`
  must yield a real `http(s)://` URL; default `:3648`, discovered-not-hardcoded,
  INTENT #24/#36). Register the `dashboard` slug.
- **Job 2 — Browser event feed (`GET /events`) = pubsub-relay's protocol,
  read-only.** The same `types::pubsub` frames/filters/lossy semantics, with one
  restriction: Subscribe/Unsubscribe accepted, Publish rejected (a browser has no
  registry lease → `NotRegistered`, session stays open). Per-node grid reads the
  daemon-stamped `Envelope.provenance.origin_node`; nothing is re-stamped.
- **Job 3 — No upstream scraping.** Services publish their own `inference.*` /
  `gc.*` / `cc.*` / `network.*` catalogs; the module subscribes locally with
  `Fleet`-scope prefix filters and `pubsub-relay`'s cross-node interest routing
  delivers the whole fleet from one local subscription — zero per-node/per-service
  socket bookkeeping. Runs in every mesh daemon, so the always-on Pi keeps the
  dashboard live even when GPU boxes sleep.
- **Job 4 — Surface-schema aggregation (INTENT #46/#37).** Services publish a
  boring `SurfaceSchema` (`types::surface`, via `chassis`) into the replicated-kv
  keyspace `surface/<slug>`; the module joins `service-registry.list()` + the
  `surface/*` keyspace + the node roster into a `DashboardManifest` at
  `GET /api/surface`. Slug-keyed LWW; the mixed-version `SurfaceSchema.v` edge
  ties to `supervision`'s OPEN mixed-version protocol (INTENT #66) — flagged, not
  silently resolved. Project dashboards enter as ordinary `surface/<project-slug>`
  entries with a light `NavEntry` grouping — no projects-specific code.
- **Presentation rollups + node-scoped proxy (aggregation ≠ routing).**
  `/api/nodes`, `/api/mesh/stats`, `/api/nodes/:id/stats` (read from replicated
  telemetry, never sysinfo-probing a remote box), and `ANY /api/nodes/:id/:slug/*`
  — a same-origin proxy that hands mesh-core's Dispatcher an
  `Address::Node{ node: id, slug }` `Request` with **browser-supplied** `:id`/
  `:slug`, so it makes NO placement decision. Never a fresh cross-host `reqwest`.

## Contracts (authored by the folded module, homed in mesh-core)

The contract *files* remain and stay authoritative; their mesh-side author is now
the `dashboard` module inside mesh-core (see `mesh-core.md` Relationships / edges
and Contracts sections):

- `dashboard-feed` — (mesh/`dashboard` → dashboard frontend) — THE batch-6 seam.
  → `scaffold/contracts/dashboard-feed.md`
- `surface-schema` — (every service → the `dashboard` module) — the
  serving/aggregation half. → `scaffold/contracts/surface-schema.md`
- `inference-events` / `gc-events` / `cc-events` — re-grounded onto
  `pubsub-protocol` topic prefixes the module subscribes to.
  → `scaffold/contracts/{inference,gc,cc}-events.md`
- Tombstones (unchanged): `mesh-registry-read` and `registry-replication` were
  already superseded by mesh-internal reads / `kv-replication`.

## Frontend is elsewhere

The Svelte **frontend** the serving plane feeds is `dashboard.md` (batch 6); it
builds against the `dashboard-feed` seam above and is unaffected by this fold.
