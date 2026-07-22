# mesh-client — ABSORBED INTO CHASSIS (tombstone-with-content)

> **⚠ TOMBSTONE — `mesh-client` IS NO LONGER A STANDALONE LIBRARY (wave-3
> ledger batch plan D1/BATCH-1; INTENT #156; seed-bishop critic-loop synthesis
> §C, state-back SB3 — operator-affirmed, no pushback recorded).** The
> operator, verbatim (SB3): "There's now one library every single service is
> built on — the daemon wrapper. It brings the daemon online, forces you to
> handle every message case at compile time, and carries the blessing queue.
> **It swallows the old mesh-client entirely — that name is retiring; it's
> just the transport half of the wrapper now.**" There is no `lib/mesh-client`
> crate, no standalone mesh-client dependency any service compiles in on its
> own. Every service instead compiles in **`chassis`** (the daemon-wrapper
> lib, INTENT #156), and everything this file designed is now chassis's
> **transport module** — one absorbed concern among chassis's others (bring-up,
> Rust-exhaustive case handling, restart-severity handling, the blessing-queue
> outbox, schema-typed errors). See `scaffold/components/chassis.md` for the
> current design; this file is retained as the record of what moved and why.

## What mesh-client was (for context)

`substrate-mesh-client` was designed (wave 2) as the tiny shared library every
service would compile in to reach its local mesh daemon on `:3649` — the single
persistent WS connection plus the thin typed client half of the universal
service protocols riding it. Chassis was independently required by INTENT #156
as the proto-daemon core lib every service inherits; rather than have services
compose two overlapping cross-cutting libraries (each wanting its own
reconnect loop, its own persistent socket), chassis absorbs mesh-client's
surface wholesale. One library, one socket, one dependency per service.

## What moved into chassis (content summary)

Everything mesh-client owned survives, relocated to chassis's transport
module, unchanged in substance:

- The **single persistent bidirectional WebSocket** to the local mesh daemon
  (`ws://127.0.0.1:3649`), multiplexing every interaction over it — RPC,
  pub/sub, registration, restart signals, surface-schema publication.
- **register / deregister / resolve** wrappers (the `service-lookup` client
  half) — slug-addressed `request(Address, method, payload)` as the standing
  inter-service call path, plus the browsable `resolve() -> Endpoint` for
  humans/health/dashboard use only (never dialed directly — single-port
  locality, INTENT #58).
- **subscribe / publish** wrappers (the `pubsub-protocol` client half) over the
  same multiplexed socket.
- **Restart-protocol participation** — the client half of the LOCKED 4-level
  graceful-restart ladder: continuous interruptibility reporting, the
  `on_restart(level)` callback contract, the L3 save-window deadline, and
  register-on-boot as the LWW registry flip. This absorbs directly into
  chassis's restart-severity handling (INTENT #156's benchmark-idle reframe:
  the service decides how it shuts itself down; requests are requests).
- **surface-schema publication** (pass-through) and its republish-on-reconnect
  behavior.
- **Lease/heartbeat lifecycle** (register → TTL → renew; deregister on
  graceful shutdown; expire-to-tombstone on crash).
- **Reconnect + re-announce**: bounded exponential backoff, an observable
  `ConnectionState`, fail-fast RPC past a grace deadline
  (`MeshError::LocalDaemonUnreachable`), a bounded drop-oldest publish buffer,
  and full re-announce of desired mesh state (registration, subscriptions,
  surface schema, interruptibility) on every reconnect.
- The **wire version-skew discipline** (INTENT #66): the client-daemon
  envelope carries a `protocol_version` and stays additive-only /
  ignore-unknown, since the daemon revs independently under minimal-restart
  rolling updates.
- **Carried-not-owned** framing: `queues-api` / `locks-api` / `cron-api`
  request frames still ride the transport's generic envelope without chassis
  learning their semantics — the same discipline mesh-client held, now
  chassis's.

Two design details from mesh-client's wave-2 pass were flagged during
harmonization and did **not** survive as proposed (recorded so chassis's
designer doesn't have to re-derive the conflict): its `RestartLevel` enum name
lost to `RestartPriority`, and its bare `Relinquished {}` reply lost to the
richer `RestartResponse` (see `restart-protocol.md`'s Reconciliation notes,
still authoritative and unchanged by this tombstone).

## What did NOT move (still true, now of chassis)

The boundary mesh-client held stands unchanged under chassis: no registry
state (server-side, `service-registry`), no relay/routing (`mesh-core` /
`pubsub-relay` / `completion-router`), no queue/lock/cron *semantics*, no
application data, no addressing decisions. Chassis's transport module still
ferries typed envelopes; it does not learn the domain meaning of what it
carries.

## Pointer

The current design lives in `scaffold/components/chassis.md`. Contract edges
formerly listed as mesh-client's (`service-lookup`, `restart-protocol`,
`pubsub-protocol`, `surface-schema`) are unchanged in shape — only the
party name changed, from `mesh-client` to `chassis`; see those contract
files' party lines.
