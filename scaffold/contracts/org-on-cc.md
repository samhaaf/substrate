# Contract: org-on-cc

> *Renamed from `org-on-ccd` at friction-round 3 (INTENT #131, ccd → cc).*

> **⚠ PARTY DISSOLVED (friction-round 3, 2026-07-20, INTENT #132).** The
> `org` crate is dissolved — "org is emergent, it just is a bunch of
> keepers" (see `components/org.md`, now a tombstone-with-content, and
> `components/keeper.md`, the L6 conceptual design that replaces it). This
> stub survives ONLY as the record that cc's surfaces were shaped for
> a broad downstream consumer from day one; the consumer of that shaping is
> now the **keeper** (LOCKED vocabulary, ledger #172 — "coordinator/owner"
> was the pre-lock working name, INTENT #130/#133), not an org daemon. No
> schema is ever authored under this name; whatever the keeper runtime
> produces will consume cc through `agent-management` (+ priorities) exactly
> as recorded below.

## Parties
- ~~`org` (L6 stub, maximalist consumer)~~ → the **keeper** construct
  (`components/keeper.md`, design-notes depth, no crate this pass) `->` `cc`,
  bundling broad shaped reads of `inference` / system-state /
  `service-registry` / `db`.

*(Stub-track: content DEFERRED, and now further gated on the keeper runtime's
own design depth (INTENT #123/#148 beat 1, ledger #172). Recorded so the
neighbor contracts stay shaped for a broad consumer; no schema authored.)*

## Purpose
The maximalist downstream consumer of the platform — historically "org," now
the emergent **keeper** layer. This edge is **inbound-to-cc only** (no
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
- Keeper-side semantics — the inter-agent **negotiation protocol**
  (propose → deliberate → counter-propose → tweak → agree), hierarchy, the
  keeper mini-harness (coordinator = owner = keeper, INTENT #127/#130) —
  sit ABOVE this edge and are explicitly NOT carried by it (not a mesh
  contract).

## Open questions
- The keeper design round (INTENT #123/#148, ledger OQ-4/OQ-6) decides the
  full propose/deliberate/approve state machine and whether this stays one
  bundle.
- Whether keeper-spawned agents re-seat onto `agents` (`agents-cc`)
  once `agents` exists — noting INTENT #137: custom agents do NOT route
  through cc; cc is for the full Claude Code agent shape only.
- The cost-reading path (`spend` pull-shaped via `projects`/`spend-cc`) is
  named but unpinned.
