# Contract: registry-replication  — SUPERSEDED (tombstone)

**Status: SUPERSEDED by `kv-replication` (2026-07-19, wave-2 contract round).**
See `scaffold/contracts/kv-replication.md`.

This edge no longer exists as a distinct protocol. Once `replicated-kv` was
extracted as the one boring replicated primitive (INTENT #54; wave2-plan §5.5
flag), the service-registry stopped owning its own anti-entropy engine and
became a plain **keyspace tenant**: registry records live at
`registry/instance/<slug>/<node>`, wrapped in `replicated-kv`'s `Entry` shape,
and replicate over the single `kv-replication` protocol (eager-push + cursor
delta + full-digest safety net) like every other kernel keyspace.

Both parties converged on this collapse — `replicated-kv.md` authored the
generalized wire, and `service-registry.md` independently recommended folding
`registry-replication` into it, contributing only the keyspace name and value
schema. There was no disagreement to resolve.

The registry's three former replication requirements are all satisfied by
`replicated-kv` and need no registry-specific wire:
- tombstones with a GC horizon (so a reconnecting partition can't resurrect a
  deregistered slug) — `replicated-kv` concern 7;
- keyspace-prefix `watch` carrying old→new value (for `ZombieSuspected`
  emission) — `replicated-kv` concern 8;
- per-node LWW clock stamping on `put` (so a heartbeat re-put wins without the
  registry doing clock math) — `replicated-kv` concern 1.

## Parties (historical)
mesh.service-registry ↔ mesh.service-registry (peer instances). Now: mesh
daemon ↔ mesh daemon over `kv-replication`.

No content is authored here. Follow the pointer to `kv-replication.md`.
