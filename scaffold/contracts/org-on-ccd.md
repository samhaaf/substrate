# Contract: org-on-ccd

## Parties
- `org` (L6 stub, maximalist consumer) `->` `ccd`, bundling broad shaped reads of
  `inference` / system-state / `service-registry` / `db`.

*(Stub-track: content DEFERRED — `org` is the un-designed exception this pass.
Recorded so the neighbor contracts are shaped for a broad consumer from the
start; no schema authored.)*

## Purpose
Org is the maximalist downstream consumer of the platform. This edge is
**inbound-to-CCD only** (no CCD→org reverse dependency) and bundles two things:
(a) org's dependency on CCD for inter-agent process supervision / handle
operations — delegated to `agent-management`; and (b) the broad "maximalist-
consumer read bundle" org needs across the other planes. It is recorded now so
`agent-management`, `ccd-events`, `v1-completion-api`, `service-lookup`, and
`db-control-plane` are all shaped for a broad consumer from day one.

## Rough shape
A broad forward-consumer envelope composed of already-authored edges — org
introduces **no new CCD surface**:
- **Agent lifecycle / handles:** reuse `agent-management`
  (spawn/signal/list/stream/reap), addressing agents by stable handle. Org passes
  a `priority` the CCD strategy engine honors (`AgentCmd::Spawn.priority`) — the
  shaped-for hook so caller priorities survive into admission.
- **Broad shaped reads:** the whole agent roster + budget state via
  `agent-management` + `surface-schema`; completions via `v1-completion-api`
  (org is one more `/v1/` client, no org-specific variant); system-state and the
  service map via `service-lookup` (resolve designed cheap + broadly queryable);
  org's own relational/metadata state via `db-control-plane`.
- Org-side semantics — the inter-agent **negotiation protocol** (propose →
  deliberate → counter-propose → tweak → agree), hierarchy, per-app owning agents
  — sit ABOVE this edge and are explicitly NOT carried by it (org-internal, not a
  mesh contract).

## Open questions
- Whether the graph-shaped portion of org's state migrates from
  `db-control-plane` to a `kg-api` edge (org's org-graph as a first-class
  provenance-traced KG) — flagged in org.md, deferred.
- Whether per-app owning agents re-seat onto `agents` (`org-agents` → `agents-ccd`)
  once `agents` exists, dissolving part of this bundle. Org is designed against
  CCD today because `agents` isn't built.
- Org's cost-reading path (`spend` pull-shaped via `projects`/`spend-ccd`) is
  named but unpinned (spend is designed after org, batch 8).
