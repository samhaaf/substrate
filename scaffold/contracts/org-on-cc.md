# Contract: org-on-cc

> *Renamed from `org-on-ccd` at friction-round 3 (INTENT #131, ccd → cc).*

> **⚠ PARTY DISSOLVED (friction-round 3, 2026-07-20, INTENT #132).** The
> `org` crate is dissolved — "org is emergent, it just is a bunch of
> coordinators" (see `components/org.md`, now a tombstone-with-content).
> This stub survives ONLY as the record that cc's surfaces were shaped for
> a broad downstream consumer from day one; the consumer of that shaping is
> now the **coordinator/owner concept** (INTENT #130/#133 — first-order,
> discussion required, not designed), not an org daemon. No schema is ever
> authored under this name; whatever construct the coordinator rounds
> produce will consume cc through `agent-management` (+ priorities) exactly
> as recorded below.

## Parties
- ~~`org` (L6 stub, maximalist consumer)~~ → the future coordinator/owner
  construct (undesigned) `->` `cc`, bundling broad shaped reads of
  `inference` / system-state / `service-registry` / `db`.

*(Stub-track: content DEFERRED, and now further gated on the coordinators
discussion round (INTENT #123/#130/#133). Recorded so the neighbor contracts
stay shaped for a broad consumer; no schema authored.)*

## Purpose
The maximalist downstream consumer of the platform — historically "org," now
the emergent coordinator layer. This edge is **inbound-to-cc only** (no
cc→consumer reverse dependency) and bundles two things: (a) the consumer's
dependency on cc for agent process supervision / handle operations —
delegated to `agent-management`; and (b) the broad "maximalist-consumer read
bundle" needed across the other planes. It is recorded so
`agent-management`, `cc-events`, `v1-completion-api`, `service-lookup`, and
`db-control-plane` are all shaped for a broad consumer from day one.

## Rough shape
A broad forward-consumer envelope composed of already-authored edges — the
consumer introduces **no new cc surface**:
- **Agent lifecycle / handles:** reuse `agent-management`
  (spawn/signal/list/stream/reap), addressing agents by stable handle. The
  caller passes a `priority` the cc strategy engine honors
  (`AgentCmd::Spawn.priority`) — the shaped-for hook so caller priorities
  survive into admission.
- **Broad shaped reads:** the whole agent roster + budget state via
  `agent-management` + `surface-schema`; completions via `v1-completion-api`
  (one more `/v1/` client, no special variant); system-state and the
  service map via `service-lookup` (resolve designed cheap + broadly
  queryable); flat relational/metadata state via `db-control-plane`
  (graph-shaped state lives on `kg-api` per INTENT #121).
- Coordinator-side semantics — the inter-agent **negotiation protocol**
  (propose → deliberate → counter-propose → tweak → agree), hierarchy, the
  coordinator/owner mini-harness (INTENT #130) — sit ABOVE this edge and
  are explicitly NOT carried by it (not a mesh contract).

## Open questions
- The coordinators discussion round (INTENT #123/#130/#133) decides what
  construct actually consumes this bundle and whether it stays one bundle.
- Whether coordinator-spawned agents re-seat onto `agents` (`agents-cc`)
  once `agents` exists — noting INTENT #137: custom agents do NOT route
  through cc; cc is for the full Claude Code agent shape only.
- The cost-reading path (`spend` pull-shaped via `projects`/`spend-cc`) is
  named but unpinned.
