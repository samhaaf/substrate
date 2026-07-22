# Contract: service-lookup

## Parties
Any device / service (cc, org, **inference**, vfs, kg, projects, secrets, the
`mesh` CLI) ↔ its **LOCAL** `mesh.service-registry` daemon, over `:3649`
(rides mesh-core's `mesh-transport` envelope). The client half is compiled
into every service as part of `chassis` (formerly `mesh-client`, absorbed —
see `components/mesh-client.md`); the server half is
`lib/mesh::registry`, a keyspace tenant of `replicated-kv`.
*(gateway removed from the party list 2026-07-18 — merged into mesh; the
dashboard origin is mesh's own surface now.)*

Authored from `service-registry.md` (server/owner) reconciled with
`inference.md` and `completion-router.md` (the two parties that forced the
concern-5 change — see Reconciliation notes).

## Purpose
THE wiring seam of the operating system. Every service answers exactly two
questions here — "where do I register myself?" and "where is service X?" — so
services resolve dependencies *by slug* against their local daemon instead of
by static URL (INTENT #58/#59). Registration is instance-keyed
(`registry/instance/<slug>/<node>`), leased with heartbeat renewal, and
tombstoned on graceful deregister; resolution is a purely LOCAL
`replicated-kv` read (the keyspace is fully replicated — "anywhere you access
mesh is exactly the same," INTENT #35), then addressing-class policy over the
slug's live instance set. Designed cheap and broadly queryable (org resolves
the whole map; dashboard-serving discovers every service to fetch surface
schemas).

## Schema
Records live in `types::registry` and cross the wire between nodes (they
anti-entropy via `kv-replication`), so they follow `types` guardrail-4
wire-crossing discipline (additive-only, `#[serde(default)]`,
`#[serde(other)]`-tolerant enums, explicit `v`).

```rust
// ── the replicated value stored at registry/instance/<slug>/<node> ──────
pub struct ServiceRecord {
    pub v:          u16,              // wire schema version (guardrail 4)
    pub slug:       Slug,
    pub node:       NodeId,           // == the key's node segment
    pub endpoint:   Endpoint,         // a browsable URL, not bare host:port
    pub addressing: AddressingClass,  // resolve policy for this slug
    pub lease:      Lease,
    pub meta:       ServiceMeta,      // version + pairwise deps (supervision reads this)
    pub state:      RecordState,      // Live | Tombstone
}
pub struct Endpoint {
    pub scheme: Scheme,               // Http | Https | Ws | Wss
    pub host:   String,               // Tailscale device name or IP
    pub port:   u16,
    #[serde(default)] pub health_path: Option<String>, // e.g. "/health"
}
pub enum AddressingClass {
    Singleton,   // exactly one live instance mesh-wide (db, cc, kg, projects, org, secrets)
    NodeScoped,  // one live instance per node (gc :8430, per-node vfs leg, inference api)
    FleetAlias,  // RESOLVE-TIME policy on a slug (inference) — NOT a stored record; see notes
    #[serde(other)] UnknownAddressing,
}
pub struct Lease {
    pub registered_at: DateTime<Utc>,
    pub expires_at:    DateTime<Utc>,  // naive wall-clock; renew extends it (INTENT #32)
    pub ttl_secs:      u32,            // renewal-interval hint for mesh-client
    pub generation:    u64,            // ++ per FRESH (re)registration on this node (restart/zombie key)
}
pub enum RecordState { Live, Tombstone { reason: DeregisterReason } }
pub enum DeregisterReason { Graceful, SupersededByHandoff, OperatorEvicted, #[serde(other)] Unknown }
pub struct ServiceMeta {
    pub service_version: SemVer,                       // the RUNNING build (per-instance)
    #[serde(default)] pub requires:   Vec<Dependency>, // pairwise version requirements
    #[serde(default)] pub pid:        Option<u32>,     // set when mesh-core spawned it (zombie discovery)
    #[serde(default)] pub started_at: Option<DateTime<Utc>>,
}
pub struct Dependency { pub slug: Slug, pub req: VersionReq } // e.g. inference ">=2.1, <3.0"

// ── the wire protocol (client -> local registry) ───────────────────────
pub enum RegistryRequest {
    Register     { reg: Registration },                                  // fresh; ++generation
    Renew        { slug: Slug, node: NodeId },                            // heartbeat; extends lease
    Deregister   { slug: Slug, node: NodeId, reason: DeregisterReason },  // -> kv tombstone
    FlipEndpoint { slug: Slug, node: NodeId, new: Endpoint },             // port-handoff (supervision-driven)
    Resolve      { addr: Address },                                       // single, addressing-class aware
    ResolveAll   { slug: Slug },                                          // all live instances of a slug
    List         {},                                                      // the whole live directory
}
pub struct Registration {
    pub slug: Slug, pub node: NodeId, pub endpoint: Endpoint,
    pub addressing: AddressingClass, pub ttl_secs: u32, pub meta: ServiceMeta,
}
pub enum Address {
    AnyNode { slug: Slug },              // "service X, don't care which node"  (INTENT #59)
    Node    { node: NodeId, slug: Slug },// "service X on node N"               (INTENT #59)
    Local   { slug: Slug },              // "service X on THIS node"
}

// ── registry -> client ─────────────────────────────────────────────────
pub enum RegistryResponse {
    Registered  { lease: Lease },
    Renewed     { lease: Lease },
    Deregistered,
    Flipped     { superseded: Endpoint },   // the old endpoint supervision must bring down
    Resolved    { record: ServiceRecord },
    ResolvedAll { records: Vec<ServiceRecord> },
    Listing     { records: Vec<ServiceRecord> },
    Error       { code: RegistryError, detail: String },
}
```

**Resolve semantics** (a live record is `state == Live` AND
`lease.expires_at > now` — expiry is read-time *data*, so a crashed service
self-heals without a write):
- `Local{slug}` → the `(slug, self)` record if live, else `NoLiveInstance`.
- `Node{node, slug}` → the `(slug, node)` record if live; mesh-core then
  relays to that peer daemon.
- `AnyNode{slug}` → policy by `addressing`:
  - `FleetAlias` (only `inference`) → the LOCAL mesh front door
    (`http://127.0.0.1:3649`), always; the local `completion-router`
    load-balances the fleet internally.
  - `Singleton` → the single live owner.
  - `NodeScoped` → **locality-preferred**: the local instance if live, else
    any live instance (deterministic node-id tiebreak).

## Error cases
`RegistryError`, surfaced as `SubstrateError::Mesh(MeshError::…)`:
- `NoSuchSlug` — no record for the slug at all.
- `NoLiveInstance` — records exist but all are lease-expired/tombstoned
  (distinct from `NoSuchSlug` so a caller can retry-vs-give-up).
- `FleetSlugNotRegisterable` — **narrowed by this contract** (see notes): now
  fires ONLY if a caller tries to write a stored `AddressingClass::FleetAlias`
  *record* for a slug; a fleet member registering its per-node
  `NodeScoped` `inference` instance is **permitted**.
- `NotOwner` — `renew`/`deregister`/`flip` for a `(slug, node)` this node
  doesn't own (a service may only renew its own instance).
- `InvalidEndpoint` — unparseable scheme/host/port.
- `KvUnavailable` — the `replicated-kv` substrate isn't ready yet (boot race);
  catchable, callers back off.
- Non-errors by design: registering a slug another node already owns is
  allowed and converges by LWW (not a conflict); resolving a slug whose owner
  is remote succeeds (reachability is mesh-core's relay concern, not
  resolve's).

## Version sensitivity
HIGH — `ServiceRecord`/`ServiceMeta`/`Endpoint` cross nodes on
possibly-different `types` versions (they anti-entropy through
`kv-replication`).
- **Additive-safe:** new `#[serde(default)]` fields on any struct; new enum
  variants IF the enums keep their `#[serde(other)]` catch-alls
  (`AddressingClass::UnknownAddressing`, `DeregisterReason::Unknown`,
  `Scheme`), so a newer node's variant never hard-fails an older peer's
  deserialize. `ServiceRecord.v` anchors the schema version. This is what
  lets a mixed-version fleet (INTENT #66) keep a coherent directory during a
  rolling update.
- **Breaking:** renaming/retyping any existing field, removing a variant, or
  changing the `(slug, node)` key convention — a `ServiceRecord.v` bump and a
  Compatibility-priority supervision restart.
- `service-lookup` shares its wire-crossing struct set with `kv-replication`
  (the same `ServiceRecord` is both the request payload and the replicated
  value) — the guardrail-4 discipline is the single source of truth for both
  edges.

## Reconciliation notes
**THE concern-5 dispute (flagged in all three files) — RESOLVED in favor of
per-node `NodeScoped` `inference` registration + `FleetAlias`-as-resolve-time-
policy. The registry side adopts.**

- **What was disputed.** `service-registry.md` concern 5 (as written) kept the
  inference *fleet* OUT of the registry: it stored only a synthetic
  `inference` `FleetAlias` record, and would raise `FleetSlugNotRegisterable`
  for any fleet member trying to register the `inference` slug. Per-node fleet
  membership was to live solely in `completion-router`'s tag-scanned
  `NodeRegistry`.
- **Who won and why.** `completion-router.md` concern 8 (the consumer) and
  `inference.md` concern 2 (the registrant) INDEPENDENTLY proposed the same
  replacement and each explicitly aligned with the other. With both the
  registrant and the consumer in agreement — and the router's bespoke
  `tailscale status` scan being exactly the discovery loop wave-2 set out to
  delete — the registry must adopt. The winning shape:
  1. Each inference daemon self-registers a per-node instance under slug
     `inference`, keyed `(inference, node)`, `addressing: NodeScoped`, with its
     real loopback `Endpoint`. The keyspace already exists
     (`registry/instance/inference/<node>`) — only the concern-5 *prose*
     conflicted, not the storage shape.
  2. **`FleetAlias` becomes a resolve-time policy on the `inference` slug, not
     a stored synthetic record.** This cleanly disambiguates three reads:
     `resolve(AnyNode{inference})` → local `:3649` (FleetAlias policy) →
     completion-router; `resolve_all("inference")` → the per-node
     `NodeScoped` instances = the router's fleet membership + endpoints;
     `resolve(Node{N,inference})` → node N's real endpoint (pinned forward +
     benchmark pinning, INTENT #15/#59).
  3. `FleetSlugNotRegisterable` is narrowed accordingly (Error cases above):
     it no longer blocks per-node `inference` instances; it only guards
     against a caller writing a *stored* FleetAlias record.
- **The losing position, recorded (not dropped).** service-registry.md's
  original FleetAlias-only stance — fleet membership out of the registry,
  router owns tag discovery — is preserved here as the rejected design. It was
  not wrong in isolation (it kept the registry minimal), but it forced the
  router to keep a `tailscale status` loop the whole wave-2 refit exists to
  remove, and it split one obvious question ("where is inference on node N?")
  across two tables. Both parties preferred the single-keyspace answer.
- **Operator fallback, noted.** If the operator later prefers membership stay
  OUT of the registry, both parties documented the same fallback: inference
  registers under a distinct `inference-node` slug and the router derives the
  `inference` fleet from `network-topology` tags (`NodeRole::Inference`) —
  still substrate-sourced, still no router-side tailscale loop. Recorded as
  the escape hatch; the adopted primary above is the recommendation.

**Other reconciliations:**
- **`service-registration` (cc) is a separate pair, not folded here.**
  service-registry.md recommends cc ride `service-lookup` with no distinct
  schema (cc's `Registration` is just `{slug:"cc", addressing:Singleton,
  requires:[rollup, db, inference?]}`). That folding is the `service-registration`
  pair's call, outside this cluster; noted so the harmonizer keeps cc as a
  party of THIS document rather than authoring a parallel wire.
- **Clock-skew caveat** (both LWW ordering and lease `expires_at` are naive
  wall-clock across nodes) is operator-blessed naive for v1 (INTENT #32),
  surfaced as a friction point, not silently accepted.
- **Deviation from the old stub.** The wave-1 stub said "`register(slug,
  host:port)` / `resolve(slug) -> endpoint`" with a bare `host:port`. This
  contract uses `Endpoint{scheme,host,port,health_path?}` (a browsable URL) so
  `mesh service open <slug>` works and dashboard-serving can health-check —
  the operator's headline CLI ask (INTENT #24).

## Example data
The example world: nodes **macbook** and **pi**, project **demo**, model
**qwen3-4b** resident on `pi`.

**1. Both inference daemons register (per-node NodeScoped — the adopted
resolution):**
```jsonc
// inference@pi -> local :3649
{ "Register": { "reg": {
  "slug": "inference", "node": "pi",
  "endpoint": { "scheme": "Http", "host": "127.0.0.1", "port": 8081, "health_path": "/health" },
  "addressing": "NodeScoped", "ttl_secs": 30,
  "meta": { "service_version": "2.1.0", "requires": [ { "slug": "db", "req": ">=1.4,<2.0" } ] }
}}}
// registry -> inference@pi
{ "Registered": { "lease": { "registered_at": "2026-07-19T00:00:00Z",
  "expires_at": "2026-07-19T00:00:30Z", "ttl_secs": 30, "generation": 1 } } }
```
`macbook` registers identically with `node:"macbook"`, `port:8080`.

**2. cc resolves inference two ways:**
```jsonc
// "give me an inference node, don't care which"  (the 99% path)
{ "Resolve": { "addr": { "AnyNode": { "slug": "inference" } } } }
//   -> Resolved { record.endpoint = http://127.0.0.1:3649 }   (FleetAlias policy)
//      then the local completion-router picks pi (qwen3-4b resident there).

// supervision / the router wants the whole fleet:
{ "ResolveAll": { "slug": "inference" } }
//   -> ResolvedAll { records: [ (inference,macbook)@:8080, (inference,pi)@:8081 ] }

// benchmark pins the sweep to pi (INTENT #15):
{ "Resolve": { "addr": { "Node": { "node": "pi", "slug": "inference" } } } }
//   -> Resolved { record.endpoint = http://127.0.0.1:8081 }   (real per-node endpoint)
```

**3. Port-handoff during a rolling update of `db` (Singleton on macbook):**
```jsonc
// supervision starts db v1.5 on a new port, the new instance flips the record:
{ "FlipEndpoint": { "slug": "db", "node": "macbook",
    "new": { "scheme": "Http", "host": "127.0.0.1", "port": 5433, "health_path": "/healthz" } } }
// registry -> supervision
{ "Flipped": { "superseded": { "scheme": "Http", "host": "127.0.0.1", "port": 5432,
    "health_path": "/healthz" } } }   // one atomic KV write; supervision now downs :5432
```
