# network-topology

**Status:** NEW. **Nesting:** child of mesh (module `lib/mesh::topology`).

## Charter

A live WebSocket surface that turns Tailscale state changes into an event feed:
peer devices joining/leaving/going offline, and — critically — **this device's
own loss of connection to the tailnet**. It publishes `network-events` to any
subscriber (ccd, org; mesh's own observability hub consumes it in-process
since the gateway merge, 2026-07-18). It owns topology *change detection and
broadcast* only; it does not query Tailscale itself (consumes `tailscale-query`)
and is deliberately distinct from completion-router's internal routing health
table (that is a routing decision input, this is a general topology feed).

## Primary design concerns

- **Snapshot diffing.** Poll `tailscale-status` on an interval, diff consecutive
  snapshots into transition events (peer_joined / peer_left / peer_offline /
  peer_online). Emit deltas, not full dumps.
- **Self-connectivity loss is the hard case, and it is NOT a peer diff.** Inferred
  when the `tailscale status` call fails or reports `BackendState != Running` /
  self `Online == false`. Must emit a distinct `self_offline` / `self_online`
  event, and while self-offline, peer state is **unknown** (not "offline") — the
  feed must not report peers as down just because we can't see them. This
  distinction is the reason this earned its own component.
- **Snapshot-on-connect then deltas.** A new WS subscriber first receives the
  current topology, then the delta stream, so it never starts from a blank/missed
  state.

## Relationships / edges

- tailscale-query via `tailscale-status` — consumes/diffs snapshots
  (scaffold/contracts/tailscale-status.md)
- any subscriber (ccd, org) via `network-events` — WS topology + self
  connectivity feed (scaffold/contracts/network-events.md)

## Nesting

Parent: mesh | Children: none. Module in `lib/mesh`; consumers subscribe over WS,
so no crate dependency on mesh is created.

## Thoroughness level

approach-sketched — polling/diffing model and self-offline detection are decided;
the exact event schema is the Contract Harmonizer's (`network-events`).

## Assigned design-depth

Single strong-model (Opus) Component-Designer pass (this file + mesh.md).

## Suggested fill-model

approach-sketched + moderate complexity → **mid model**. The only subtlety
(self-offline vs peer-unknown) is spelled out here.
