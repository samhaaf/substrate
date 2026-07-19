# service-registry

**Status:** SUPERSEDES the wave-1 `service-registry.md` stub (approach-sketched,
LWW/lease/anti-entropy design carried in `mesh.md` Concern 1). Wave-2 re-grounds
the module **on the extracted `replicated-kv` substrate** (it no longer owns its
own LWW/anti-entropy engine — that moved to a sibling L2 lib) and pins the wire
formats. **Nesting:** internal lib of mesh (`lib/mesh::registry`), Ring 3 in
mesh-core's layering (rides `replicated-kv`, Ring 2). Never a standalone service
(INTENT #54).

## Charter

`service-registry` is **THE wiring seam** of the operating system: the
distributed, eventually-consistent `slug -> endpoint` directory spanning every
device in the tailnet. It answers exactly two questions for the whole system —
"where do I register myself?" and "where is service X?" — so that every top-level
service registers its own slug at boot and resolves its dependencies *by slug*
here instead of by static URL (INTENT #58/#59, the `service-lookup` seam). It
owns the **record model** (`slug -> { endpoint, addressing class, lease,
version/dependency metadata, state }`), the **lease/heartbeat/tombstone
lifecycle**, the **resolve semantics for mesh-core's three addressing classes**
(`AnyNode` / `Node{N}` / `Local`), the **coexistence of the two discovery
mechanisms** (the tag-discovered inference *fleet* behind the single `inference`
front-door alias vs. self-registered *singleton* slugs), and the **read surface
that feeds supervision** (version + pairwise dependency requirements) and
**mesh-core's zombie-killing / port-handoff choreography** (it *detects and
signals* a superseded endpoint; it does not kill processes).

**Boundary — what it does NOT own.** It does not implement the replicated store:
LWW registers, anti-entropy, tombstone GC, and offline reconnect sync are
`replicated-kv`'s (this module is a *keyspace tenant*, consuming `trait KvHandle`
— mesh-core seam). It does not route completions or discover the inference fleet
by tag — that is `completion-router`'s `NodeRegistry` (routing health), a
*distinct table* it must never be conflated with (mesh.md invariant:
`NodeRegistry` ≠ `service-registry` ≠ `network-topology`). It does not *kill*
processes or *decide* boot order or restart level — it *stores* the metadata and
*emits* the signals; `supervision` (policy) + mesh-core's `ProcessControl`
(execution) act on them. It holds no application data (that is `db`) and no
completion state (nodes' SQLite is the system of record). It is a compiled-in
library, not a cross-app link (INTENT #29/#45).

## Primary design concerns

### 1. Re-grounding on `replicated-kv` — the wave-2 structural move

The wave-1 registry *was* its own LWW-register + anti-entropy engine. Wave-2
extracts that engine into `replicated-kv` (INTENT #54: "one boring system with
the same primitive... buried inside mesh"), so the registry becomes a **thin
schema + policy tenant** of a single keyspace. This is the whole point of the
re-grounding: the registry contributes *value shapes and read/resolve policy*;
the store contributes *replication, LWW convergence, tombstones, and reconnect
sync*. Concretely, all registry state lives under one namespaced keyspace opened
via mesh-core's `KvHandle`:

```
registry/instance/<slug>/<node_id>   -> ServiceRecord     (one per running instance)
```

- **The key is `(slug, node_id)`, not `slug`.** A slug can have >1 live instance
  (a NodeScoped service like `gc` runs one per node; a rolling update briefly has
  two on one node during port-handoff). Keying by instance lets LWW converge each
  instance independently and lets `resolve` apply addressing-class policy over the
  *set* of a slug's instances. A pure `slug -> endpoint` key (the operator's
  original phrasing) cannot express node-pinned addressing (INTENT #59) or the
  mixed-version fleet (INTENT #66).
- **`replicated-kv` owns the LWW clock; the record is the payload.** Each value is
  wrapped by `replicated-kv` in its `(wall_clock, node_id)` LWW register (naive
  timestamp-wins, blessed INTENT #32). A heartbeat is a `kv.put` of the same
  record with an extended lease — the store stamps a fresher clock, so the renewal
  wins convergence for free. The registry never compares clocks itself.
- **Consistency latitude respected:** the registry inherits `replicated-kv`'s
  eventual consistency exactly; two partitions each re-registering the same slug
  converge by naive LWW on reconnect (INTENT #32). No vector clocks, no SWIM — out
  of scope by operator blessing.

### 2. The record model — endpoint + addressing class + lease + service metadata

One value type carries everything a resolver, the CLI, supervision, and
zombie-killing need. It is a **wire-crossing struct** (it anti-entropies to
peers on possibly-different versions), so it follows `types`' guardrail-4
discipline (additive-only, `#[serde(default)]`, `#[serde(other)]`-tolerant
enums, explicit `v`).

```rust
// value stored at registry/instance/<slug>/<node_id>; lives in types::registry
pub struct ServiceRecord {
    pub v: u16,                       // wire schema version (guardrail 4)
    pub slug: Slug,
    pub node: NodeId,                 // the instance's node (== key's node_id)
    pub endpoint: Endpoint,           // {scheme, host, port, health_path?}
    pub addressing: AddressingClass,  // resolve policy for this slug (concern 3)
    pub lease: Lease,                 // lifecycle (concern 4)
    pub meta: ServiceMeta,            // version + dependencies (concern 6)
    pub state: RecordState,           // Live | Tombstone (concern 4)
}

pub struct Endpoint {                 // refines "slug -> host:port": a browsable URL
    pub scheme: Scheme,               // Http | Https | Ws | Wss
    pub host: String,                 // Tailscale device name or IP
    pub port: u16,
    #[serde(default)] pub health_path: Option<String>, // e.g. "/healthz" for mesh's health-check
}

pub enum AddressingClass {
    Singleton,   // exactly one live instance mesh-wide (db, ccd, kg, projects, org, secrets)
    NodeScoped,  // one live instance per node (gc :8430, per-node vfs leg, inference api)
    FleetAlias,  // the synthetic front-door alias (inference -> local mesh :3649) — concern 5
}
```

`Endpoint` is `{scheme, host, port, health_path?}` and **not** bare `host:port`
so `mesh service open <slug>` builds a real browsable URL and dashboard-serving
can health-check (mesh.md; INTENT #24). Bare `host:port` can't be opened in a
browser — this is the operator's headline CLI ask.

### 3. Resolve semantics — one directory, mesh-core's three addressing classes

The registry provides mesh-core's `Resolver` seam. Resolution is a **local**
`KvHandle` read (the keyspace is fully replicated locally — "anywhere you access
mesh is exactly the same," INTENT #35), then addressing-class policy over the
slug's live instance set. It never makes a network call to resolve; relay to a
remote owner is mesh-core's Dispatcher job *after* resolve returns the endpoint.

- **`Local { slug }`** → the `(slug, self)` record if live; else `NoLiveInstance`.
  Used for node-scoped health, `gc :8430`, per-node polls.
- **`Node { node, slug }`** → the `(slug, node)` record if live; else
  `NoLiveInstance`. mesh-core then relays to that peer daemon.
- **`AnyNode { slug }`** → policy by `addressing`:
  - `FleetAlias` (only `inference` today) → the **local** mesh front door
    (`http://127.0.0.1:3649`), always — single-port locality; the caller's local
    completion-router does the fleet routing (concern 5).
  - `Singleton` → the single live instance (the current owner).
  - `NodeScoped` → **locality-preferred**: the local instance if live, else any
    live instance (deterministic node-id tiebreak). This is a real policy call —
    `AnyNode` on a per-node service means "give me a reachable one, prefer close."
- **Live filtering is read-time.** A record is *live* iff `state == Live` AND
  `lease.expires_at > now`. Expired-but-not-tombstoned records are invisible to
  `resolve` without any write — a crashed service's slug self-heals on lease
  expiry (concern 4). This is why lease expiry is *data*, not a mutation.

Two broad-query methods beyond single resolve, because `service-lookup` is
explicitly "cheap and broadly queryable" (org resolves the whole service map,
dashboard-serving discovers every service to fetch surface schemas):

```rust
fn resolve_all(&self, slug: &Slug) -> Result<Vec<ServiceRecord>>; // all live instances of a slug
fn list(&self) -> Result<Vec<ServiceRecord>>;                     // the whole live directory
```

### 4. Lease / heartbeat / tombstone lifecycle — the correctness core

A crashed service must not poison its slug forever, and a stale anti-entropy copy
must not resurrect a dead one.

- **Lease + heartbeat.** Registration carries a TTL; the owner renews via a
  periodic `renew` (heartbeat) that re-`put`s the record with a fresh
  `lease.expires_at`. `mesh-client` drives renewal on the service's behalf.
  Missing renewals → `expires_at` passes → record filtered at read time.
- **`generation`** bumps on every *fresh* registration of `(slug, node)` (a new
  process, not a renewal). It is the restart-detection / zombie-disambiguation key
  (concern 7) and lets a subscriber tell "same process renewed" from "restarted on
  a new port."
- **Tombstone = explicit deregister, delegated to `replicated-kv`.** A graceful
  `deregister` calls `kv.delete(key)`, which `replicated-kv` implements as an
  LWW tombstone with its own GC-after horizon — so a stale Live copy arriving late
  from a reconnecting partition loses to the tombstone's fresher clock and does
  **not** resurrect the entry. **Tombstone storage + GC is `replicated-kv`'s
  concern, not the registry's** (the whole reason it was extracted). A *crashed*
  service never deregisters, so it leaves no tombstone — its record simply
  lease-expires and is read-filtered; an optional housekeeping sweep may
  `kv.delete` long-expired records to keep the keyspace tidy, but correctness does
  not depend on it.

```rust
pub struct Lease {
    pub registered_at: DateTime<Utc>,
    pub expires_at:    DateTime<Utc>,  // naive wall-clock; renew extends it (INTENT #32)
    pub ttl_secs:      u32,            // renewal-interval hint for mesh-client
    pub generation:    u64,            // ++ per fresh (re)registration on this node
}
pub enum RecordState { Live, Tombstone { reason: DeregisterReason } }
pub enum DeregisterReason { Graceful, SupersededByHandoff, OperatorEvicted }
```

**Clock-skew caveat (flagged).** Both LWW ordering and lease `expires_at` are
naive wall-clock across nodes; a node with a fast clock can win writes it
shouldn't or expire a peer's lease early. Operator-blessed naive for v1 (INTENT
#32 — "pretty much everything's going to be driven from my laptop"); surfaced as
a friction point, not silently accepted.

### 5. Two discovery mechanisms coexisting — fleet alias vs. singleton (INTENT #57)

This is the non-obvious call the wave-1 design flagged, now pinned. **Two
mechanisms run side by side and consumers never see the split** — they always
just `resolve(slug)`:

1. **Slug registry (this module).** Singletons and node-scoped services
   *self-register* their own `(slug, node) -> endpoint` records via
   `service-lookup`. Included here is the **`inference` front-door alias**: at
   boot (mesh-core boot step 8), each mesh daemon registers `inference` as a
   `FleetAlias` record pointing at its *own* `:3649`. `resolve("inference")` on
   any node therefore returns the local mesh front door — never "one of N boxes."
2. **Tag-based fleet discovery (NOT here — `completion-router`'s `NodeRegistry`).**
   The actual inference nodes are discovered by Tailscale tag and health-polled by
   the router, entirely inside `completion-router`. The registry holds *only* the
   alias; the fleet's membership/health lives in the router's separate table.

So a `resolve("inference")` hits the alias → local `:3649` → the local router
load-balances the tag-discovered fleet internally. A `resolve("db")` hits a
singleton record → the one owner. Same API, two mechanisms, invisible seam
(mesh.md; INTENT #59). The registry must **refuse per-node slug registration of a
`FleetAlias` slug** by anything but mesh-core itself (a fleet member trying to
register `inference` directly is a category error → `FleetSlugNotRegisterable`).

### 6. Version + dependency metadata — the read surface supervision consumes (INTENT #45/#66)

Mesh "just tracks the requirements and the boot order" (INTENT #45). The registry
is where that metadata *lives* (replicated with the record); `supervision`
*consumes* it to derive boot ordering and to plan pairwise-compatibility rolling
updates. The registry **stores, supervision computes** — it does not itself build
the dependency DAG or choose a restart level.

```rust
pub struct ServiceMeta {
    pub service_version: SemVer,           // the RUNNING build's version (per-instance)
    #[serde(default)] pub requires: Vec<Dependency>,  // pairwise version requirements
    #[serde(default)] pub pid: Option<u32>,           // set when mesh-core spawned it (zombie discovery, concern 7)
    #[serde(default)] pub started_at: Option<DateTime<Utc>>,
}
pub struct Dependency { pub slug: Slug, pub req: VersionReq } // e.g. inference ">=2.1, <3.0"
```

- **Version is per-instance** (`(slug, node)`), so the registry naturally captures
  a *mixed-version fleet* during a rolling update — node A on `v2`, node B on `v1`
  (INTENT #66). Supervision reads `resolve_all` across the mesh to see the mixed
  state and drive minimal-restart updates. (The concrete mixed-version *update
  protocol* is OPEN and owned by `supervision` — INTENT #66; the registry only
  provides the observable state it needs.)
- **`requires` are declarative pairwise version constraints** — the registry
  stores them verbatim; supervision topo-sorts the `requires` graph into a boot
  order and evaluates compatibility (does the version I'm about to boot satisfy
  the running versions of what it depends on?). Boot order is a *derivation* over
  registry data, not a stored field — keeping the registry boring and the ordering
  policy in one place (supervision).

### 7. Zombie-killing support + port-handoff — detect & signal, don't kill (INTENT #57/#76)

The registry is the *discovery half* of two process-hygiene duties; mesh-core's
`ProcessControl` is the *execution half* (it owns the OS-level kill;
`supervision` owns when/whether).

- **Port-handoff during updates (INTENT #76): the registry flip is the atomic
  center.** The choreography is: (1) supervision starts the new version on a new
  port; (2) the new instance re-registers — a single `flip_endpoint` LWW `put`
  swaps `(slug, node)`'s endpoint old→new and bumps `generation`; resolvers
  instantly see the new endpoint (registry flip); (3) supervision brings the old
  port down. Step 2 is one atomic KV write, so there is never a window where the
  slug points at nothing. `flip_endpoint` **returns the superseded endpoint** so
  supervision knows exactly which port to bring down.

  ```rust
  fn flip_endpoint(&self, slug: &Slug, node: &NodeId, new: Endpoint)
      -> Result<Endpoint /* the superseded old endpoint */>;
  ```

- **Zombie-killing (INTENT #57): the registry emits the signal.** "When a service
  restarts and re-registers on a new port, the old still-running copy must be
  discovered and killed." The registry `subscribe`s to its own keyspace (via
  `kv.subscribe`); when a `(slug, node)` record's `endpoint`/`generation` changes,
  it emits a `ZombieSuspected { slug, node, superseded: Endpoint, pid: Option<u32> }`
  on a channel `supervision` drains. mesh-core then `ProcessControl.discover(superseded)`
  (by held PID if it spawned the service, else by probing the stale endpoint's
  identity/pidfile) and kills it. **The registry never kills a process** — it
  provides the "here is an endpoint nothing points at anymore, and here is the PID
  if we know it" fact. This keeps the OS-kill authority solely in mesh-core
  (matching its `ProcessControl` ownership) and the *policy* in supervision.

  Zombie-suspicion is distinct from three neighbors, all deliberately separate:
  *lease expiry* (a stale *entry*, self-healed at read time), *squatter-killing*
  (a foreign *process on mesh's own port*, mesh-core concern 3), and *supervision
  restart* (a *deliberate* replacement). The registry owns only the entry-level
  signal.

## Relationships / edges

Contract edges (cross-process WS/wire, rides mesh-core's `mesh-transport` frame on
`:3649`):

- **any device/service (ccd, org, inference, vfs, kg, projects, secrets, the mesh
  CLI) ↔ service-registry** via `service-lookup` — register / renew / deregister /
  resolve / resolve_all / list; THE wiring seam. Client half is `mesh-client`.
  (scaffold/contracts/service-lookup.md)
- **ccd ↔ service-registry** via `service-registration` — CCD as a first-class
  registrant+resolver; an *instance* of `service-lookup`, called out because CCD
  is both. No distinct schema (see Proposed contracts).
  (scaffold/contracts/service-registration.md)
- **registry peers ↔ peers** via `registry-replication` — anti-entropy of the
  `registry/instance/*` keyspace. **Wave-2 recommendation: collapse into
  `kv-replication`** now that `replicated-kv` is extracted (the generalization the
  wave2-plan flagged §5.5). The registry contributes the keyspace + value schema;
  the replication *wire* is `replicated-kv`'s. (scaffold/contracts/registry-replication.md)
- ~~gateway via `mesh-registry-read`~~ — TOMBSTONE (gateway merged into mesh
  2026-07-18; the fleet read is mesh-internal now). No action; noted for lineage.
  (scaffold/contracts/mesh-registry-read.md)

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45):

- **consumes DOWN** `replicated-kv::KvHandle` (get/put LWW, `delete`/tombstone,
  `subscribe`-to-keyspace) — the substrate the whole module rides. *Co-batch
  dependency: `replicated-kv` (Fable) is designed in parallel; this design
  consumes the `KvHandle` shape mesh-core froze and flags the tombstone/`delete`
  and keyspace-subscribe requirements for that designer — see Proposed contracts.*
- **provides UP** `trait Resolver` (mesh-core's Dispatcher + CLI ride it) and
  `trait Registrar` (register/renew/deregister/flip); plus the `ZombieSuspected`
  channel `supervision` drains and the `resolve_all`/`list` read surface
  `dashboard-serving` (surface-schema discovery) and `supervision`
  (version/boot-order) consume.
- **sibling, must-not-conflate:** `completion-router`'s `NodeRegistry` (fleet
  routing health) and `network-topology` (peer on/off feed) are *distinct tables*;
  the registry only holds the `inference` `FleetAlias` (concern 5).

## Nesting

Parent: mesh (mesh-core) | Children: none. Server side is `lib/mesh::registry`,
Ring 3 in mesh-core's layering (rides `replicated-kv` at Ring 2). The shared
client half (`register`/`resolve`/`renew`) is part of `mesh-client`'s surface —
services get it from the same thin boot lib, not a separate crate (confirmed at
skeleton time).

## Thoroughness level

**implementation-ready.** The record model, the KV keyspace layout and
tenancy-on-`replicated-kv` re-grounding, the three-addressing-class resolve
policy, the lease/heartbeat/read-time-expiry/tombstone lifecycle, the two-
discovery-mechanism coexistence (fleet alias vs. singleton), the version/
dependency metadata read surface, and the detect-and-signal split for
zombie-killing + the atomic `flip_endpoint` port-handoff are all decided and
specified. Genuinely-downstream opens: (a) the exact `KvHandle` tombstone/`delete`
+ keyspace-subscribe API — a co-batch reconciliation with `replicated-kv`, flagged
below; (b) whether `registry-replication` fully collapses into `kv-replication` —
recommended here, a batch-2 call; (c) the housekeeping-sweep cadence for
long-expired records — a fill-time knob, not a design fork.

## Assigned design-depth

Opus, single strong-model Component-Designer pass (this file), grounded in the
wave-1 `service-registry.md` + `mesh.md` Concern 1, the batch-1 designs
(`mesh-core.md` seams, `types.md` node/endpoint vocabulary, `pubsub-relay.md`
provenance discipline), the live `lib/mesh/{discovery,config,lib}.rs`, and INTENT
items 32/45/57/58/59/66/76.

## Suggested fill-model

**implementation-ready + medium complexity → mid model OK**, with two carve-outs
for a careful hand: (1) the `resolve` addressing-class policy + read-time live
filtering is the one correctness spot (the `AnyNode` FleetAlias-vs-Singleton-vs-
locality-preferred branch, and never returning an expired/tombstoned record) — do
not send *that* function to the cheapest tier; (2) the `flip_endpoint` +
`ZombieSuspected` subscribe path must be filled *against* `replicated-kv`'s final
`KvHandle`, so it is sequenced after `replicated-kv` fills. Everything else
(record structs, register/renew/deregister, list/resolve_all) is near-transcription
from this file.

---

## Proposed contracts (wave 2)

Proposals only; the per-pair round reconciles both sides. The structs live in
`types::registry` (they cross the wire between nodes, so they follow `types`'
guardrail-4 wire-crossing discipline). I do NOT edit `scaffold/contracts/*`.

### `service-lookup` (any device/service ↔ mesh.service-registry) — the wiring seam

**Purpose.** The one protocol every service speaks to its LOCAL mesh daemon to
register itself and resolve dependencies by slug, over `:3649` (rides mesh-core's
`mesh-transport` envelope; client half is `mesh-client`). Designed cheap and
broadly queryable (org resolves the whole map; dashboard-serving discovers every
service).

**Message/struct sketch** (Rust-flavored; `types::registry`):

```rust
// client -> registry
pub enum RegistryRequest {
    Register   { reg: Registration },                 // fresh registration; ++generation
    Renew      { slug: Slug, node: NodeId },           // heartbeat; extends lease.expires_at
    Deregister { slug: Slug, node: NodeId, reason: DeregisterReason }, // -> kv tombstone
    FlipEndpoint { slug: Slug, node: NodeId, new: Endpoint }, // port-handoff (supervision-driven)
    Resolve     { addr: Address },                     // single, addressing-class aware
    ResolveAll  { slug: Slug },                        // all live instances of a slug
    List        {},                                    // the whole live directory
}
pub struct Registration {
    pub slug: Slug, pub node: NodeId, pub endpoint: Endpoint,
    pub addressing: AddressingClass, pub ttl_secs: u32, pub meta: ServiceMeta,
}
pub enum Address { AnyNode { slug: Slug }, Node { node: NodeId, slug: Slug }, Local { slug: Slug } }

// registry -> client
pub enum RegistryResponse {
    Registered  { lease: Lease },
    Renewed     { lease: Lease },
    Deregistered,
    Flipped     { superseded: Endpoint },              // the old endpoint to bring down
    Resolved    { record: ServiceRecord },
    ResolvedAll { records: Vec<ServiceRecord> },
    Listing     { records: Vec<ServiceRecord> },
    Error       { code: RegistryError, detail: String },
}
```

**Error cases** (`RegistryError`, surfaced as `SubstrateError::Mesh(MeshError::…)`
per `types` error taxonomy):
- `NoSuchSlug` — no record for the slug at all.
- `NoLiveInstance` — records exist but all are lease-expired or tombstoned
  (distinct from `NoSuchSlug` so callers can retry vs. give up).
- `FleetSlugNotRegisterable` — a fleet member tried to self-register a
  `FleetAlias` slug (`inference`); only mesh-core registers the alias (concern 5).
- `NotOwner` — `renew`/`deregister`/`flip` for a `(slug, node)` this node doesn't
  own (a service can only renew its own instance).
- `InvalidEndpoint` — unparseable scheme/host/port.
- `KvUnavailable` — the `replicated-kv` substrate is not ready (boot race);
  catchable, callers back off.
- Non-errors by design: registering a slug already owned by *another* node is
  allowed and converges by LWW (not a conflict); resolving a slug whose owner is a
  remote node succeeds (returns the endpoint) — reachability is mesh-core's relay
  concern, not resolve's.

**Version-sensitivity.** HIGH — `ServiceRecord`/`ServiceMeta`/`Endpoint` cross
nodes on possibly-different `types` versions (they anti-entropy). Additive-only
fields, all new fields `#[serde(default)]`; `AddressingClass`/`Scheme`/
`DeregisterReason` reserve `#[serde(other)]` catch-alls so a newer node's variant
never hard-fails an older peer's deserialize (guardrail 4). `ServiceRecord.v`
anchors the schema version. This is what lets a mixed-version fleet (INTENT #66)
keep a coherent directory during a rolling update.

### `service-registration` (ccd ↔ mesh.service-registry) — an instance of `service-lookup`

**Purpose.** CCD is a first-class registry participant — both registrant (it
registers the `ccd` singleton slug) and resolver (it resolves `inference`, `db`,
`rollup`, etc.). Called out separately per the inventory, but it carries **no
distinct schema**: it is `service-lookup` applied to the CCD party.

**Struct sketch.** None new. CCD's `Registration` is `{ slug: "ccd",
addressing: Singleton, meta.requires: [rollup, db, inference?] }`; its resolves
are ordinary `Resolve`/`ResolveAll`. Recommendation for the per-pair round:
**fold `service-registration` into `service-lookup`'s example world** (CCD as one
of the parties) rather than authoring a parallel wire — the two are the same
contract, and the surface-schema-style "one document, many parties" shape
(wave2-plan §5.4) applies. Flagged, not unilaterally collapsed.

**Error cases / version-sensitivity.** Identical to `service-lookup`.

### `registry-replication` (registry peers ↔ peers) — RE-GROUND: collapse into `kv-replication`

**Purpose.** Propagate the `registry/instance/*` keyspace between per-device
registry instances (slug→endpoint entries, LWW conflict resolution, tombstones,
offline reconnect sync). Wave-1 authored this as the registry's own anti-entropy;
**wave-2 re-grounds it: the anti-entropy engine is now `replicated-kv`, so this
edge should collapse into `kv-replication`** (the exact generalization the
wave2-plan flagged, §5.5). The registry contributes only the **keyspace name and
value schema**; the replication *wire* (full-state periodic merge, LWW register
envelope, tombstone GC horizon, bidirectional reconnect sync) is
`replicated-kv`'s to author.

**What the registry contributes to the shared example world:**

```
keyspace:  registry/instance/<slug>/<node_id>
value:     ServiceRecord (above), wrapped by replicated-kv's LWW register:
           LwwRegister<ServiceRecord> { value, clock: (wall_clock_ns, node_id) }
tombstone: kv.delete(key) -> LWW tombstone with replicated-kv's GC-after horizon
```

**Requirements I flag for the co-batch `replicated-kv` designer** (consumed via
mesh-core's `KvHandle` seam):
- **Tombstones with a GC horizon** — needed so a reconnecting partition's stale
  `Live` record cannot resurrect a deregistered slug (concern 4). This is a
  general KV need (locks + queues want it too), which is *why* it belongs in
  `replicated-kv`, not here.
- **Keyspace-prefix `subscribe`** — the registry watches `registry/instance/*`
  for endpoint/generation changes to emit `ZombieSuspected` (concern 7). Needs
  change-notification carrying old→new value, not just "something changed."
- **Per-node LWW clock stamping on `put`** — so a heartbeat re-put wins over the
  prior record without the registry doing clock math.

**Error cases.** Replication faults (peer unreachable, partial merge) are
`replicated-kv`'s (`MeshError::{ReplicationLagging, PeerUnreachable}`); the
registry surfaces none of its own on this edge — it is a pure tenant.

**Version-sensitivity.** HIGH and shared with `service-lookup`: the same
`ServiceRecord` crosses the same version boundary, so the wire-crossing discipline
(additive, `#[serde(default)]`, `#[serde(other)]`, `v`) is the single source of
truth for both edges. **Batch-2 call to make explicit:** if `registry-replication`
collapses into `kv-replication`, mark `scaffold/contracts/registry-replication.md`
a pointer-to-`kv-replication` (like the `mesh-registry-read` tombstone) rather
than a parallel wire — recommended, deferred to the per-pair round.
