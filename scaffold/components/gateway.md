# gateway

**Status:** existing (`bin/gateway`), RESHAPE (narrowed role + multi-node + mesh-backed
discovery). **Nesting:** top-level. **Kind:** real app — an always-on daemon (port 8400),
the human/observability front door. Not a library.

## Charter

Gateway is the **observability & presentation plane** of Substrate — the single browser-
facing origin that hosts the dashboard and gives a human one live pane of glass over the
whole fleet. Concretely it owns four things and only these four:

1. **Dashboard hosting** — serves the compiled `ui/dashboard/dist/` Svelte app (static
   files) from one origin.
2. **Event fan-out hub** — subscribes to each node's inference event stream and each
   node's GC event stream, and multiplexes them into ONE topic-filtered WebSocket
   (`GET /events`) that browser clients subscribe to. Envelopes are tagged with the real
   per-node `node_id` so the dashboard can build its per-node grid + fleet header.
3. **Observability aggregation** — presentation-shaped fleet rollups for the dashboard:
   `GET /api/nodes`, `GET /api/mesh/stats`, `GET /api/nodes/:id/stats` (disk, GPU, RAM,
   GC-managed bytes, aggregated `SystemState`).
4. **Browser REST proxy** — a same-origin convenience proxy (`/api/nodes/:id/inference/*`,
   `/api/nodes/:id/gc/*`, plus default-node aliases) so the dashboard talks to one host
   instead of fighting CORS across N nodes + GC daemons.

**What gateway explicitly does NOT own (the resolved identity question):** Gateway is
**NOT "akin to mesh"** and is **not a resource distributor**. It owns no discovery, no
service registry, and **routes no completions**. Those belong to `mesh`
(tailscale-query, service-registry, completion-router). The completion data-path is
`mesh`'s `:8419` transparent forwarder; gateway never sits on it. Gateway is a **read-
mostly CLIENT of mesh**: it learns *where the nodes are* by reading mesh's `/api/nodes`
over HTTP (`mesh-registry-read`), then connects out to those nodes for events and rollups.
The only thing gateway "centralizes" is the **observability view and the dashboard
origin** — a presentation concern, categorically different from mesh's coordination/
routing concern. Mesh is the machine-facing coordination plane; gateway is the human-
facing observability plane. They are siblings that never overlap: mesh moves requests,
gateway aggregates events and hosts the UI.

## Primary design concerns

**1. The identity resolution (the operator's flagged unknown), stated as the boundary.**
Before mesh grew a registry + router, one might have imagined gateway as a "centralized
service distributing resources across the mesh." That role now lives entirely in
`mesh.completion-router` (routing) and `mesh.service-registry` (the wiring seam). Gateway
is what's *left over once mesh takes coordination*: dashboard hosting + event fan-out +
observability rollups + a browser convenience proxy. This earned its own component not for
complexity but because it is a genuinely distinct **concern and lifecycle** — it is the
one process that must stay alive on the always-on Pi so the dashboard renders even when
every GPU box is asleep (offline nodes draw as offline, the page never goes dark).

**2. The static-config → mesh-backed-resolution refactor (confirmed in scope).** Today
gateway carries static `inference_url` / `gc_url` / `node_id` (`config.rs`) and opens
exactly two upstream WS connections (`upstream.rs`). v2 replaces this with dynamic
resolution in two hops:
- **Bootstrap via `service-lookup`:** resolve the `mesh` slug from the service-registry to
  find mesh's base URL, and register gateway's own `gateway` slug -> `host:port` so the
  operator/other services can discover the dashboard.
- **Fleet via `mesh-registry-read`:** read mesh's `GET /api/nodes` (NodeInfo + roles + per-
  node inference/GC endpoints + last `SystemState`) as the authoritative, fleet-aware node
  list. This is richer than the flat slug->endpoint registry — it carries roles and live
  state — so it, not `service-lookup`, is gateway's per-node endpoint source.
- **Dev fallback (graceful degradation):** if no registry/mesh is reachable, fall back to a
  static `nodes` list (or the legacy single-URL default) so a single-box dev setup with no
  mesh still works. Registry present ⇒ dynamic wins; absent ⇒ static fallback.

**3. Deliberate reversal of the mesh-synthesis "embed NodeRegistry" recommendation.**
`reports/mesh-design-synthesis.md` §4 said gateway should *reuse mesh's `NodeRegistry` +
`TailscaleDiscovery` code* and run its own discovery/poll loops (accepting the duplicated
`tailscale status` shell-outs it flagged as finding #12 / OQ-6). Now that mesh exposes
`/api/nodes` as a first-class HTTP surface, the cleaner design is: **gateway embeds NO
registry and runs NO discovery — it reads the fleet from mesh over HTTP.** This (a) kills
the duplicated-discovery concern for gateway outright, (b) makes "gateway is a client of
mesh, not a peer" true by construction, and (c) keeps the tier boundary clean (gateway
depends on `substrate-types` for the `/api/nodes` response shape, not on `lib/mesh`
internals). Surfaced as a controversial decision because it reverses a finalized report.

**4. Dynamic per-node subscription reconciliation (the one real new correctness surface).**
Today gateway opens a fixed two WS connections. In v2 the node set is *dynamic* (mesh's
`/api/nodes` changes as boxes wake/sleep), so gateway must reconcile a live set of per-node
subscriptions: spin up one inference-WS + one GC-WS per newly-seen node, tear down
subscriptions for vanished nodes, without leaking tasks or double-subscribing. The existing
`reconnect_loop` (exponential backoff, envelope tagging) is reused per node; the new piece
is the supervisor that diffs the node set on each `/api/nodes` refresh. Gateway subscribes
**directly to each node** (not through mesh) so mesh stays a lean completion data-plane
that never has to hold browser-shaped event state.

**5. Shared-library foresight — the #1 dedup candidate in the whole workspace.** Gateway's
`proxy.rs::forward` (preserve method/headers, strip hop-by-hop, return upstream status +
headers + body) is *the exact working template* the mesh synthesis says `mesh`'s
`MultiNodeRouter::forward` must copy (§"Trait/module changes"). That is duplicated reverse-
proxy logic across two crates by construction. Design note for the later dedup pass: this
helper (a reqwest-based, status/header-preserving HTTP forward) is the prime extraction
target — a small shared `lib/http-proxy` (or a helper in `substrate-types`/a new util lib)
that both gateway and mesh import. **Do not extract now** (that's the dedup pass's job and
mesh isn't built yet); just don't reinvent a *second, divergent* copy in gateway. Secondary
candidate: the event-envelope + `Topic` taxonomy (currently gateway-local `GatewayEvent`/
`Topic`) may want to promote into `substrate-types` if nodes emit pre-enveloped events —
flagged to the Contract Harmonizer, not decided here.

**6. Aggregation must not become routing.** `/api/mesh/stats` and `/api/nodes` are read-only
rollups computed by fanning out reads to nodes (or reading mesh's already-aggregated view);
they must never make a placement/routing decision or hold authoritative state — that would
re-collapse gateway back into "akin to mesh." Rollups are derived, cache-cheap, and
disposable. The GC-managed-bytes figure keeps the existing subtlety (sum each dir's real
`used_bytes`, never `policy.max_size_bytes`, so disk-composition never sums past 100%).

## Relationships / edges

- **mesh** via `mesh-registry-read` (gateway -> mesh): reads `GET /api/nodes` (endpoints +
  roles + last `SystemState`) and `GET /api/mesh/stats`; authoritative per-node endpoint
  source. See `scaffold/contracts/mesh-registry-read.md`.
- **mesh.service-registry** via `service-lookup` (gateway <-> mesh.service-registry):
  registers the `gateway` slug; resolves the `mesh` slug at bootstrap. Replaces static
  `inference_url`/`gc_url`. See `scaffold/contracts/service-lookup.md`.
- **mesh.network-topology** via `network-events` (gateway <- mesh.network-topology): gateway
  is a concrete "any subscriber" — folds device on/off + self-connectivity-loss events into
  its hub so the dashboard's fleet header shows live online/offline + connectivity state.
  See `scaffold/contracts/network-events.md`.
- **inference** via `inference-events` (gateway <- inference, one subscription per node):
  per-node WS event stream + REST proxy, envelopes tagged with the node's real `node_id`.
  See `scaffold/contracts/inference-events.md`.
- **gc** via `gc-events` (gateway <- gc daemon `:8430`, one per node): GC WS event stream +
  REST proxy. See `scaffold/contracts/gc-events.md`.
- **dashboard** via `dashboard-feed` (gateway -> dashboard): aggregated `GET /events` WS
  fan-out (per-client topic filtering) + REST + static hosting; stable kebab-case element
  ids for agent-driving. See `scaffold/contracts/dashboard-feed.md`.

All edges gateway touches already have contract stubs; **no new contract stub is needed**
from this component. (`network-events`'s stub already names gateway as a subscriber.)

## Nesting (if applicable)

Top-level, no children. `bin/gateway` stays a single binary crate; its modules (`hub`,
`upstream`, `proxy`, `stats`, `ws`, `topics`, `config`, `state`) are internal, not
components. The proxy-forward helper is a *future* extraction to a shared lib (concern #5),
not a child component.

## Thoroughness level

implementation-ready

Rationale: the crate exists and works; the reshape is enumerated as concrete per-file
changes (`config.rs` multi-node/discovery + static fallback; `upstream.rs` dynamic per-node
subscription supervisor with real `node_id`; `proxy.rs` node-scoped routes + default
aliases, reusing the existing `forward`; `stats.rs` `/api/nodes`, `/api/mesh/stats`,
`/api/nodes/:id/stats`; add a `network-events` subscription into the hub). The one genuinely
new module — the dynamic subscription supervisor that diffs mesh's `/api/nodes` — is
specified in concern #4. The identity/boundary question is resolved, not deferred. Remaining
undecideds are surfaced as open questions, not silent gaps.

## Assigned design-depth

Single strong-model Component Designer (Opus-class), one pass — with direct reads of the
live `bin/gateway/src/*` code, the finalized `reports/mesh-design-synthesis.md`, the
Decomposer's `overview.md`, and the neighboring mesh/service-registry/dashboard component
designs. Not a Design Mesh run (the component is a well-understood reshape, not novel), but
deeper than a stub because it had to resolve the operator's flagged identity question.

## Suggested fill-model

**implementation-ready + low/moderate complexity -> cheap model OK**, with two guardrails
for the Filler: (a) the dynamic per-node subscription supervisor (concern #4) is the only
part with real concurrency subtlety — no task leaks, no double-subscribe, clean teardown on
node vanish; give that module a careful review even on a cheap model. (b) Do NOT write a
second reverse-proxy forward implementation — reuse/keep gateway's existing `proxy.rs::forward`
and remember it's the shared-lib extraction target (concern #5), so keep it self-contained
and copy-clean rather than entangled with gateway-specific state.
