# dashboard

## Charter

The Svelte/Vite operator-facing dashboard (`ui/dashboard`): a pure rendering +
light local-state layer over whatever mesh's observability plane aggregates. It
renders live disk/GPU/RAM, model/backend lifecycle, the completion queue, the
GC tree, benchmark/kernel curves, and (new in v2) a fleet-wide node roster —
now across potentially many machines instead of one. It owns exactly one
external edge (`dashboard-feed`, mesh -> dashboard) and does **not** own:
aggregation, node discovery, event fan-out, or REST proxying (all mesh's
job); WS reconnect/backoff and event-log persistence already exist
(`stores/connection.js`) and are kept, only reshaped for the node dimension.
It does not talk to inference, gc, or ccd directly, in any v2 shape — every
byte it renders arrives through mesh, and it discovers everything via mesh.

> **SUPERSEDED (2026-07-18, round-3): "served by gateway."** Everywhere this
> file said `gateway`, read `mesh`: gateway merged into mesh, which now serves
> the dashboard's static assets and its `GET /events` fan-out + REST surface.
> The aggregation shapes gateway had locked (`/api/nodes`, `/api/mesh/stats`,
> node-scoped proxy routes) carry over unchanged as mesh surfaces.

> **NEW (round-3): rendering becomes schema-driven.** Per the boring-surface-
> schema pattern (`scaffold/contracts/surface-schema.md`), every service
> publishes a schema of its observable surface — how to render its dashboard
> component and what calls to make against the service — in a shared schema
> language defined in `types`. The dashboard renders every service's component
> **from its published schema** rather than from hand-built per-service panels;
> visual coherence by construction. Project-published dashboards (`projects`)
> surface through the same mechanism and must be navigable from this main mesh
> dashboard. The per-panel concerns below (store reshaping, id templating,
> per-node grids) remain valid as the rendering substrate the schema-driven
> layer targets, but the panel *inventory* becomes schema-supplied, not
> hardcoded. `requirements-only` for this layer — the schema language is not
> yet designed.

## Primary design concerns

The hard part isn't any single panel — it's that **every existing store and
panel assumed exactly one node**, and that assumption is baked in five
different ways that each need their own answer:

1. **State reshaping.** `stores/state.js` today exports five flat singleton
   stores (`systemState`, `modelState`, `backendState`, `queueState`,
   `gcState`) with no `node_id` anywhere — `handleEvent()` switches only on
   `service`/`event.type`. These become **maps keyed by `node_id`**
   (`Map<node_id, SystemState>` etc.), each read via a `perNode(store, id)`
   derived-store helper, not five separate variables per panel instantiation.
   `handleEvent()` gains one line at the top: pull `node_id` off the envelope
   (falling back to a `"self"` sentinel — see point 4) before dispatching on
   `service`/`event.type`, otherwise its switch logic is unchanged. This is
   confirmed low-risk because the current dashboard-stub note ("achieved by
   filtering existing tagged envelopes with no new WS plumbing") already
   commits to reshaping-in-place, not a new subscription mechanism.

2. **Per-node vs. fleet-wide panel classification.** Not every panel
   replicates per node:
   - **Replicate per node:** System, Model, Backend, Queue, GC, Benchmark —
     each is a property of one machine's inference/gc process.
   - **Stay fleet-wide (one instance, aggregate over the same envelope
     stream):** EventLog (a chronological log is naturally cross-node; add a
     `node_id` column + a node filter, don't fork it N ways) and a new
     **node-roster / topology strip** (replaces `NetworkPanel`'s current
     single-node stub, see point 6).
   - Rationale for NOT replicating EventLog: a per-node event log fragments
     the one thing an operator wants during an incident — "what just happened,
     across the whole fleet, in order." Filtering is a strictly better UX than
     forking.

3. **Extending the stable-element-id convention into a second dimension
   (must-preserve, per commit `53eb5f3`).** Today's panel ids are process-wide
   singletons: `system-panel`, `system-cpu-bar`, `queue-panel`,
   `queue-pause-button`, `gc-panel`, `benchmark-run-button`, etc. Once a panel
   type exists once per node, these ids become **duplicate ids in the same
   document** unless templated — which is both invalid HTML and breaks any
   agent doing `document.getElementById(...)` / `#system-panel` on the
   assumption of global uniqueness. The fix is to extend the SAME templating
   pattern already used for dynamic list rows (`queue-item-${item.id}`,
   `gc-dir-toggle-${slug(dir.root)}`, `network-node-${node.id}`) up one level,
   to the panel/section root and everything under it:
   `system-panel-${node_id}`, `system-cpu-bar-${node_id}`,
   `queue-pause-button-${node_id}`, `benchmark-run-button-${node_id}`, etc.
   Static ids stay static ONLY for genuinely-singleton elements
   (`dashboard-app`, `dashboard-header`, `ws-status`, `event-log-panel` and
   its children, the new roster panel and its children). **A Filler must
   audit every existing `id=` in every panel component being made per-node and
   re-template it — this is not optional cleanup, it's required to avoid
   silently regressing the agent-legibility property the prior pass added.**
   Node ids must themselves be slug-safe for id interpolation (reuse
   `GcPanel.svelte`'s existing `slug()` helper rather than inventing a second
   one — first sign of the "watch for duplicated logic" instruction the
   operator flagged for the later shared-lib pass).

4. **Single-box dev / no-mesh fallback.** The overview's wiring-seam section
   commits to "a static-config fallback... retained for single-box dev so the
   seam degrades gracefully." The dashboard needs the same story: when mesh
   reports exactly one node (or, degenerately, zero — mesh not wired
   up yet), the per-node grid renders exactly one node-group using a `"self"`
   sentinel id, so today's single-machine dev loop keeps working with zero
   extra clicks. This mirrors, not duplicates, the seam's own fallback
   decision.

5. **"No data yet" vs. "went offline" are different states on the same
   surface.** A freshly-discovered node (mesh just found it, but no
   `system` event yet) and a previously-live node that
   `network-events` reports as dropped must NOT render identically — today's
   stores have no way to distinguish "never populated" from "was populated,
   now stale/offline" (`writable({...zeroes...})` looks the same either way).
   Each per-node store slot needs an explicit `status: 'pending' | 'live' |
   'offline'` field, set to `'pending'` on node-roster discovery, `'live'` on
   first real event, and `'offline'` on a topology-drop signal — panels dim
   instead of disappearing when `'offline'`, so an operator sees "this node
   fell off the mesh" rather than a panel silently vanishing.

6. **The node-roster / topology strip supersedes `NetworkPanel`'s current
   hardcoded stub.** Today `networkState` is a hand-written single-entry list
   (`[{ id: 'local', ... }]`) with a comment "future: Tailscale nodes will
   appear here." In v2 this becomes real: sourced from mesh's
   `/api/nodes` (NodeInfo + roles + last SystemState; the old
   `mesh-registry-read` hop collapsed into mesh with the gateway merge)
   polled/refreshed the same way `QueuePanel`/`GcPanel` already poll today, plus
   the same envelope stream driving per-node `status` (point 5). This is still
   ONE fleet-wide panel (not per-node) — it's the thing that tells you which
   nodes exist and lets the per-node grid render dynamically instead of a
   fixed compile-time `<SystemPanel/><ModelPanel/>...` block per node type (as
   `App.svelte` does today for the single implicit node).

7. **REST fetch surface generalizes from implicit-single-node to
   node-scoped.** Every panel's poll (`SystemPanel` -> `/api/inference/metrics`
   + `/api/nodes/local/stats`, `QueuePanel` -> `/api/inference/completions`,
   `GcPanel` -> `/api/gc/dirs`/`/api/gc/entries`, `BenchmarkPanel` ->
   `/api/inference/benchmark/*`) currently hits a flat, un-scoped proxy path
   because the old gateway proxied to exactly one static `inference_url`/`gc_url`.
   Note the existing precedent was inconsistent: `/api/nodes/local/stats`
   already had the node-scoped shape (`/api/nodes/<id>/...`), just hardcoded
   to the literal string `local`, while `/api/inference/*` and `/api/gc/*` did
   not. **This is now confirmed, not just proposed** — the pre-merge gateway
   design locked the exact shape, which carries over unchanged as mesh
   surfaces (`mesh.md` concern 5): `GET /api/nodes`, `GET
   /api/mesh/stats`, `GET /api/nodes/:id/stats`, and node-scoped proxy routes
   `/api/nodes/:id/inference/*` + `/api/nodes/:id/gc/*` ("plus default-node
   aliases" for the single-box case, matching this file's `"self"`-sentinel
   fallback in concern 4). Every dashboard panel's `fetch()` call should be
   rewritten against these exact paths — no further reconciliation needed on
   the REST shape.

## Relationships / edges

- **mesh**, via `dashboard-feed` (see `scaffold/contracts/dashboard-feed.md`)
  — the dashboard's ONLY edge *(was gateway; merged into mesh, 2026-07-18)*.
  Carries the aggregated `GET /events` WS
  fan-out (envelopes tagged with the real per-node `node_id`), the node-scoped
  REST polls confirmed
  in concern 7 above (`GET /api/nodes`, `GET /api/mesh/stats`, `GET
  /api/nodes/:id/stats`, `/api/nodes/:id/inference/*`, `/api/nodes/:id/gc/*`),
  static hosting, and (round-3) every service's published **surface schema**
  from which the dashboard renders each component
  (`scaffold/contracts/surface-schema.md`). No direct edges to inference, gc,
  or ccd in this design — deliberately, to keep mesh as the single always-on
  aggregation point (the always-on Pi runs mesh so the dashboard stays live
  even when every GPU box is asleep). `mesh.network-topology`'s
  `network-events` reaches the dashboard only by way of mesh's fan-out hub
  folding it in-process into `dashboard-feed`, never as a second direct
  subscription — the dashboard stays a one-edge component the way its own
  pre-existing stub already states ("Consumes `dashboard-feed` only").

## Nesting

Parent: none (top-level). Children: none.

## Thoroughness level

`approach-sketched` — the store-reshaping approach, the id-templating rule,
the per-node/fleet-wide panel split, and the offline-vs-pending state field are
sketched concretely enough to fill against, but the exact envelope/REST
shapes are intentionally left to the Contract Harmonizer (step 3), since
`dashboard-feed.md` and `surface-schema.md` are still schema-deferred
stubs and this component must not invent a contract unilaterally. The round-3
schema-driven-rendering layer is `requirements-only` (see Charter note).

## Assigned design-depth

Sonnet.

## Suggested fill-model

`approach-sketched` + moderate-but-not-novel complexity (this is mechanical
reshaping of already well-structured Svelte code — five stores go from flat to
keyed, panels get wrapped in a node loop, ids get templated — no new
algorithms, no new protocol design) -> a mid-tier model (Sonnet-class) is
sufficient for Fill. Only escalate if the Contract Harmonizer's final
`dashboard-feed`/`surface-schema` shapes diverge significantly from the
proposal in concern 7 above (e.g. if node-scoped REST turns out to be
WS-only with no REST fallback, the polling-panel pattern would need a
heavier rework than sketched here).
