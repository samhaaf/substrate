# gateway — MERGED INTO MESH (tombstone)

> **This component no longer exists.** Merged into `mesh` — see
> `scaffold/components/mesh.md` — 2026-07-18 (round-3 feedback lock).

## Why

Operator decision, round-3 feedback: "All inter-node communication needs to go
through mesh… if mesh is built where anywhere you access it is exactly the same,
I'm not seeing a need for gateway." With mesh present on every device and
identical wherever you access it, a separate browser-facing aggregation daemon
was a redundant plane. Mesh — now explicitly "the operating system" — absorbs
gateway's remaining real jobs:

- **Dashboard hosting** — mesh serves the compiled `ui/dashboard/dist/` static
  assets.
- **Browser event fan-out** — mesh subscribes to per-node event streams
  (inference, gc, cc) and multiplexes them into the topic-filtered `GET /events`
  WebSocket browser clients consume.
- **Observability rollups + browser REST proxy** — `/api/nodes`,
  `/api/mesh/stats`, node-scoped proxy routes: all mesh surfaces now.

## Edge rewiring (what happened to gateway's contracts)

- `inference-events`, `gc-events`, `cc-events` — party renamed: now
  `mesh <- {inference, gc, cc}`.
- `dashboard-feed` — party renamed: now `mesh -> dashboard`.
- `mesh-registry-read` — **collapsed entirely**: gateway reading mesh's
  `/api/nodes` is now mesh reading its own registry; internal, no contract.
- `service-lookup` — gateway dropped from the party list (mesh doesn't register
  itself with itself for this role; the `dashboard` slug is mesh's own surface).
- `network-events` — gateway-as-subscriber becomes mesh-internal (the fan-out
  hub consumes topology in-process); external subscribers (cc, org) unchanged.

Design content worth keeping (dynamic per-node subscription supervisor, the
`proxy.rs::forward` reuse/dedup note, "aggregation must not become routing")
moved into `mesh.md`'s absorbed-observability section. The pre-merge design of
this file is preserved in git history at commit `6879866`.
