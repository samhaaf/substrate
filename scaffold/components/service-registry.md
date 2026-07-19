# service-registry

**Status:** NEW. **Nesting:** child of mesh. **Also: the single wiring seam.**
Full integrated design + rationale lives in `components/mesh.md` (Concern 1); this
file is the focused per-child contract.

## Charter

A distributed, eventually-consistent `slug -> endpoint` store spanning every
device in the tailnet. It answers exactly two questions for the whole system:
"where do I register myself?" and "where is service X?". It is **the assembly
seam** — every top-level service registers its own slug at boot and resolves its
dependencies by slug here instead of by static URL. It owns **addressing only**,
never application data (that is `db`) and never routing health of the inference
fleet (that is completion-router's `NodeRegistry`).

## Primary design concerns

- **LWW-register CRDT per slug.** Entry = `slug -> { endpoint, owner_node,
  lease_expires_at, version:(wall_clock, node_id) }`. Total order with
  deterministic node-id tiebreak; converges without coordination. Right-sized for
  1–few devices — vector clocks / SWIM are explicitly out of scope (escalation
  only).
- **Leases + heartbeats + tombstones.** Registration carries a TTL; the owner
  renews. Expiry → tombstone (with its own GC-after TTL to block resurrection).
  This is the correctness core: a crashed service must not poison its slug
  forever. A registration without a live lease is invisible to `resolve`.
- **Anti-entropy replication** (`registry-replication`): periodic full-state LWW
  merge with peers discovered via `tailscale-query`. Pi coordinator is the
  recommended seed/anchor but not required (peer-to-peer convergence must hold
  without it).
- **Fleet vs singleton addressing:** the mesh registers the `inference` slug ->
  its own `:8419` front door (it internally load-balances the tag-discovered
  fleet); singleton services register their own slug -> own endpoint. See
  mesh.md Concern 1 — this is the non-obvious call.
- **Endpoint is `{scheme,host,port,health_path?}`, not bare host:port**, so
  `mesh service open` yields a browsable URL and mesh's observability plane can health-check.

## Relationships / edges

- any device/service (ccd, org, inference, vfs, kg, projects, mesh CLI) via `service-lookup`
  — register/resolve; the seam (scaffold/contracts/service-lookup.md)
- ccd via `service-registration` — first-class registrant+resolver
  (scaffold/contracts/service-registration.md)
- ~~gateway via `mesh-registry-read`~~ — collapsed 2026-07-18 (gateway merged
  into mesh; the fleet read is mesh-internal now; the contract file is a
  tombstone) (scaffold/contracts/mesh-registry-read.md)
- service-registry peers via `registry-replication` — anti-entropy merge
  (scaffold/contracts/registry-replication.md)

## Nesting

Parent: mesh | Children: none. Server side lives in `lib/mesh::registry`; the
shared client half is the proposed `substrate-mesh-client` crate (see mesh.md
Concern 0).

## Thoroughness level

approach-sketched — consistency model, lease/tombstone rules, and addressing
decisions are settled; replication wire format + exact merge/GC parameters are
left for the Contract Harmonizer (`registry-replication`, `service-lookup`) and
fill.

## Assigned design-depth

Single strong-model (Opus) Component-Designer pass (this file + mesh.md),
grounded on the live `lib/mesh` and the synthesis's tier-boundary invariant.

## Suggested fill-model

approach-sketched + **high complexity** → **strong model, or a focused Design
Mesh pass** on the replication protocol + LWW/lease/tombstone edge cases first.
The one child where design has not fully bought down fill risk — do not send to a
cheap model.
