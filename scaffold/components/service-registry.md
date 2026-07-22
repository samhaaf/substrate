# service-registry

**Status:** SUPERSEDES the wave-1 `service-registry.md` stub (approach-sketched,
LWW/lease/anti-entropy design carried in `mesh.md` Concern 1). Wave-2 re-grounds
the module **on the extracted `replicated-kv` substrate** (it no longer owns its
own LWW/anti-entropy engine — that moved to a sibling L2 lib) and pins the wire
formats. **Nesting:** internal lib of mesh (`lib/mesh::registry`), Ring 3 in
mesh-core's layering (rides `replicated-kv`, Ring 2). Never a standalone service
(INTENT #54).

**Wave-3 fold (this pass).** Three folds land, no structural change to the
record model or resolve policy: (1) **sender version stamping (INTENT #113)** is
made explicit as a *division of labour* — the registry is the version *inventory*
(`ServiceMeta.service_version` per instance), the transport layer carries the
*per-message* stamp (`Provenance.service_version`, `types::transport`), and
**version-floor enforcement + the `PleaseUpdate` back-signal is a receiver-EDGE
policy owned by chassis / the per-contract edge — NOT the registry** (concern 6,
extended). (2) The registering party is **`chassis`** everywhere (`mesh-client`
retired into it, ledger D1); the client half of `service-lookup` is a chassis
sub-surface. (3) The **update-flow hooks** get registry-side mechanics: the
atomic `flip_endpoint`, the **brief dual-registration window semantics**, and the
zombie-detection feed are specified against supervision's port-handoff
choreography (INTENT #76) and mesh-core's `ProcessControl` (INTENT #57) — concern
7, expanded and reconciled. The per-node `inference` `NodeScoped` registration +
`FleetAlias`-as-resolve-policy is **LOCKED (wave-2 resolution)** and is not
reopened. **PARKED OQ-1 held:** registration / renew / deregister / flip involve
**no** authority or blessing — they are plain LWW writes on `replicated-kv`;
consistency-requiring *application* changes route through chassis's blessing
queue, never through registration (concern 8).

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
    Singleton,   // exactly one live instance mesh-wide (db, cc, kg, projects, secrets)
    NodeScoped,  // one live instance per node (gc :8430, per-node vfs leg, inference api)
    FleetAlias,  // resolve-time POLICY on the inference slug (-> local mesh :3649) — NOT a stored record; concern 5 as relaxed
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
explicitly "cheap and broadly queryable" (the keeper runtime resolves the whole
service map, mesh-core's dashboard surface discovers every service to fetch
surface schemas):

```rust
fn resolve_all(&self, slug: &Slug) -> Result<Vec<ServiceRecord>>; // all live instances of a slug
fn list(&self) -> Result<Vec<ServiceRecord>>;                     // the whole live directory
```

### 4. Lease / heartbeat / tombstone lifecycle — the correctness core

A crashed service must not poison its slug forever, and a stale anti-entropy copy
must not resurrect a dead one.

- **Lease + heartbeat.** Registration carries a TTL; the owner renews via a
  periodic `renew` (heartbeat) that re-`put`s the record with a fresh
  `lease.expires_at`. **`chassis` drives renewal on the service's behalf** (the
  daemon-wrapper's lease task, ledger D1 — `mesh-client` retired into chassis).
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
    pub ttl_secs:      u32,            // renewal-interval hint for chassis
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

### 5. One directory, FleetAlias as resolve-time policy — RELAXED at the contract round (INTENT #57)

> **Superseded prose note (harmonization).** As originally written, this concern
> kept the inference *fleet* OUT of the registry (only a synthetic stored
> `FleetAlias` record; `FleetSlugNotRegisterable` for any fleet member). The
> contract round resolved the three-file dispute AGAINST that stance —
> `completion-router.md` concern 8 and `inference.md` concern 2 independently
> proposed the same replacement, and the registry adopts. Authoritative
> resolution: `scaffold/contracts/service-lookup.md` §Reconciliation notes.
>
> **Wave-3 note: this resolution is LOCKED and is NOT reopened this pass.** The
> per-node `NodeScoped` `inference` registration + `FleetAlias`-as-resolve-time-
> policy split is carried forward verbatim; wave-3's version-stamp and update-flow
> folds touch neither the fleet keyspace nor the resolve policy.

The adopted shape:

1. **Each inference daemon self-registers a per-node instance** under slug
   `inference`, keyed `(inference, node)`, `addressing: NodeScoped`, with its
   real loopback `Endpoint` — the keyspace `registry/instance/inference/<node>`
   as already defined. `resolve_all("inference")` IS the router's fleet
   membership + endpoints; no router-side `tailscale status` scan survives.
2. **`FleetAlias` is a resolve-time policy on the `inference` slug, not a
   stored record.** `resolve(AnyNode{inference})` → the **local** mesh front
   door (`:3649`) → the local completion-router load-balances;
   `resolve(Node{N, inference})` → node N's real endpoint (pinned forward +
   benchmark pinning, INTENT #15/#59). Same API, invisible seam (mesh.md).
3. `FleetSlugNotRegisterable` is **narrowed**: it no longer blocks per-node
   `inference` instances; it fires only if a caller tries to write a *stored*
   `FleetAlias` record.

`completion-router`'s `NodeRegistry` remains a distinct table — but it is now
fed from `resolve_all("inference")` + `network-topology`, holding routing
*health/load/inventory* state, not membership.

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

**Sender version stamping (INTENT #113) — the division of labour (wave-3 fold).**
#113 has three moving parts, and it is a correctness point that the registry owns
exactly ONE of them:

1. **Version INVENTORY — the registry (this module).** `ServiceMeta.service_version`
   is the *declared running build* of each `(slug, node)` instance, replicated
   with the record. It is the authoritative answer to "what version is running
   where," and the read surface (`resolve_all`, `list`) supervision consumes to
   compute floors and plan pairwise-compatible rolling updates. It is registration
   metadata, refreshed at register/flip — **not** a per-message field.
2. **Per-MESSAGE stamp — the transport layer, NOT here.** Every mesh message
   carries the sending service's name + version on `Provenance.service_version`
   (`types::transport` / `types::provenance`), stamped once by **chassis** on the
   outbound frame (INTENT #113 LOCKED; `types.md` wave-3). The registry neither
   stamps nor reads this; it is a property of the envelope, not of a registration.
3. **Version-floor ENFORCEMENT + `PleaseUpdate` back-signal — a RECEIVER-EDGE
   policy, owned by chassis / the per-contract edge, NOT the registry.** A receiver
   may enforce a floor ("I only accept messages from service-version ≥ X"); a
   below-floor message is rejected at the edge with a catchable
   `WireError { domain:"mesh", code:"version_below_floor", … }` (mirrors
   `MeshError::VersionBelowFloor`), and a one-version-back-compat message is
   tolerated *with* a `PleaseUpdate` warning attached back to the sender — never a
   silent drop (`types::transport::{WireError, PleaseUpdate}`, `types.md` wave-3).
   **The registry is referenced by this policy but does not run it:** the floor a
   receiver enforces is *derived* from the registry's inventory (what versions are
   live) plus the receiver's own compatibility rule; the enforcement executes on
   chassis's inbound edge, per contract. The registry stores the truth the floor
   is computed from; it never gatekeeps a message. This keeps the registry a pure
   directory and puts message-time policy where the message is (the edge).

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

- **Brief dual-registration window semantics (wave-3 fold — the reconciliation).**
  The keyspace invariant is **exactly one Live `ServiceRecord` per `(slug, node)`
  key** — this is what MAKES the flip atomic (a resolver never sees two fronted
  endpoints and never sees none). So during a same-node rolling update the "dual"
  is a **process-level** window (two processes briefly alive), *not* a keyspace
  window (there is never a second Live record for one key). The registry represents
  the whole handoff as **one LWW `put`** on the single `(slug, node)` key:

  - **Before the flip:** the record points at the OLD endpoint. `resolve` returns
    old — correct, old is still serving. The new process may already be bound and
    healthy on its new port, but supervision verifies that by **probing the port
    directly** (it knows the port from its `SpawnSpec`), *not* via a registry entry
    — so the new instance need not be registered to be verified.
  - **The flip:** one `flip_endpoint` `put` (or the new instance's own fresh
    `Register` on the same key) swaps endpoint old→new and **bumps `generation`**.
    LWW makes it a single atomic cut-over; `resolve` returns new from the next read.
    There is no intermediate state where the slug points at nothing or at both.
  - **After the flip:** the OLD endpoint is now unreferenced by the record. The
    record's `generation` bump is the marker that a supersession happened; it feeds
    zombie-detection (next bullet). supervision then downs the old process; the
    registry entry already points only at new.

  **Reconciliation with `supervision.md` concern 5 (flagged for the harmonizer).**
  supervision's step-1 prose says the new instance "registers itself (a *second,
  distinct registry entry* — same slug, new endpoint)." Under the `(slug, node)`
  single-key model that is **not** a second *Live record* — it is the same key's
  **next `generation`**, adopted by the flip `put`. The registry deliberately holds
  ≤1 Live record per `(slug, node)`; the atomicity guarantee depends on it. Two
  boring disciplines both land on the same single-key write, and the registry
  supports both:
  - **supervision-driven flip:** new instance boots in a handoff/standby mode
    (carried in `SpawnSpec`) that **defers self-registration**; supervision probes
    its port, then issues `FlipEndpoint` — one `put`, returns the superseded
    endpoint synchronously so supervision knows which port to down.
  - **self-registering flip:** the new instance's chassis registers on the same
    `(slug, node)` key with a fresh `generation` + new endpoint; that `put` *is*
    the flip (higher generation / fresher clock wins), and the registry emits
    `ZombieSuspected` for the superseded endpoint (next bullet) — no separate
    `FlipEndpoint` call, but no synchronous superseded-endpoint return either, so
    supervision learns the old port from the zombie signal instead.

  The registry never health-checks on flip — it is a dumb LWW store and **trusts
  supervision's verify-then-flip ordering** (supervision confirms the new instance
  healthy before flipping; the registry only records the cut). Flagged so the
  harmonizer aligns supervision's "second entry" wording with the single-key
  invariant; no keyspace change is proposed (generation-in-key was considered and
  rejected as less boring — one-line-reversible per ledger §C global rule).

- **Zombie-killing (INTENT #57): the registry emits the signal.** "When a service
  restarts and re-registers on a new port, the old still-running copy must be
  discovered and killed." The registry `subscribe`s to its own keyspace (via
  `kv.subscribe`); when a `(slug, node)` record's `endpoint`/`generation` changes,
  it emits a `ZombieSuspected { slug, node, superseded: Endpoint, pid: Option<u32> }`
  on a channel **`supervision` drains**. supervision *decides* (its
  should-be-running reconciliation, supervision.md concern 6) and commands
  **mesh-core's `ProcessControl.discover(superseded)`** (by held PID if mesh
  spawned the service, else by probing the stale endpoint's identity/pidfile after
  the flip) → `signal(pid, SIGTERM)`→grace→`SIGKILL` (mesh-core concern 4). **The
  registry never kills a process and never decides** — it provides the "here is an
  endpoint nothing points at anymore, and here is the PID if we know it" fact. This
  is the clean three-way split: registry *detects & signals* the entry-level
  supersession, supervision *decides*, mesh-core *executes* the OS-level kill.

  Zombie-suspicion is distinct from three neighbors, all deliberately separate:
  *lease expiry* (a stale *entry*, self-healed at read time), *squatter-killing*
  (a foreign *process on mesh's own port*, mesh-core concern 3), and *supervision
  restart* (a *deliberate* replacement). The registry owns only the entry-level
  signal.

### 8. Registration is authority-free — PARKED OQ-1 held

`register` / `renew` / `deregister` / `flip_endpoint` are **plain LWW writes on
`replicated-kv`**. None of them consults, requires, or emits any *blessing* or
*authority* decision — there is no authority party on any registry edge, and the
registry threads no authority dependency anywhere. This is deliberate and it
respects **PARKED OQ-1** (ledger §B.1/§C — the authority-node-vs-no-central-node
question is the operator's to settle, not a wave-3 call).

The distinction to keep crisp: **registration is directory maintenance, not a
consistency-requiring change.** A service announcing "I am `db` at `:5432`, v1.5"
is eventually-consistent bookkeeping that converges by naive LWW on partition
merge (concern 1) — two partitions each re-registering the same slug is a
**non-conflict** (service-lookup Error cases: "registering a slug another node
already owns is allowed and converges by LWW"). It never needs blessing. The
*application* changes that DO require consistency blessing (a write that two
partitions must not both commit) route through **chassis's blessing-queue seam**
against an **abstract blessing-target** (chassis concern 7) — a path that is
entirely separate from the registry and stays one-line-swappable between #163's
no-central-node `locks` merge-reconciler and a future cloud authority. The
registry deliberately holds **nothing** that names, locates, or privileges an
authority node, so either resolution of OQ-1 lands without a change here.

## Relationships / edges

Contract edges (cross-process WS/wire, rides mesh-core's `mesh-transport` frame on
`:3649`):

- **any device/service (cc, inference, vfs, kg, projects, secrets, the keeper
  runtime, the mesh CLI) ↔ service-registry** via `service-lookup` — register /
  renew / deregister / resolve / resolve_all / list; THE wiring seam. **Client
  half is `chassis`** (the daemon-wrapper every service links; `mesh-client`
  retired into it, ledger D1). (scaffold/contracts/service-lookup.md)
- **cc ↔ service-registry** via `service-registration` — cc as a first-class
  registrant+resolver; an *instance* of `service-lookup`, called out because cc
  is both. No distinct schema (an instance of `service-lookup`).
  (scaffold/contracts/service-registration.md)
- **mesh daemon ↔ mesh daemon** via `kv-replication` — the collapse HAPPENED at
  the contract round (the generalization wave2-plan flagged §5.5): the registry
  contributes only the `registry/` keyspace + value schema; the replication
  wire is `replicated-kv`'s one protocol. `registry-replication` is a
  superseded tombstone. (scaffold/contracts/kv-replication.md)
- ~~gateway via `mesh-registry-read`~~ — TOMBSTONE (gateway merged into mesh
  2026-07-18; the fleet read is mesh-internal now). No action; noted for lineage.
  (scaffold/contracts/mesh-registry-read.md)

Internal-lib seams (compiled-in, NOT contract edges — INTENT #29/#45):

- **consumes DOWN** `replicated-kv::KvHandle` (get/put LWW, `delete`/tombstone,
  `subscribe`-to-keyspace) — the substrate the whole module rides. *Co-batch
  dependency: `replicated-kv` (Fable) is designed in parallel; this design
  consumes the `KvHandle` shape mesh-core froze and flags the tombstone/`delete`
  and keyspace-subscribe requirements for that designer (recorded in the authored contracts).*
- **provides UP** `trait Resolver` (mesh-core's Dispatcher + CLI ride it) and
  `trait Registrar` (register/renew/deregister/flip); plus the `ZombieSuspected`
  channel `supervision` drains and the `resolve_all`/`list` read surface
  **mesh-core's dashboard surface** (surface-schema discovery — `dashboard-serving`
  folded into mesh-core, ledger D5/F9b) and `supervision` (version/boot-order)
  consume.
- **sibling, must-not-conflate:** `completion-router`'s `NodeRegistry` (fleet
  routing health/load/inventory) and `network-topology` (peer on/off feed) are
  *distinct tables*; the registry holds the per-node `NodeScoped` `inference`
  instances (fleet membership = `resolve_all("inference")`), while `FleetAlias`
  is resolve-time policy only (concern 5 as relaxed).

## Nesting

Parent: mesh (mesh-core) | Children: none. Server side is `lib/mesh::registry`,
Ring 3 in mesh-core's layering (rides `replicated-kv` at Ring 2). The shared
client half (`register`/`resolve`/`renew`) is part of **`chassis`**'s surface —
services get it from the same daemon-wrapper lib they link to become a service,
not a separate crate (`mesh-client` retired into `chassis`, ledger D1; confirmed
at skeleton time).

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

Opus, single strong-model Component-Designer pass (wave 2), grounded in the
wave-1 `service-registry.md` + `mesh.md` Concern 1, the batch-1 designs
(`mesh-core.md` seams, `types.md` node/endpoint vocabulary, `pubsub-relay.md`
provenance discipline), the live `lib/mesh/{discovery,config,lib}.rs`, and INTENT
items 32/45/57/58/59/66/76.

**Wave-3 fold pass (this pass, Opus)** — grounded additionally in the batch-1
`types.md` (`transport.rs`: `MeshReply`/`WireError`/`PleaseUpdate`, `provenance.rs`
`service_version`) and `chassis.md` (client-half absorption of `mesh-client`, the
blessing-queue seam), the batch-2 `mesh-core.md` (concern 4 zombie/ProcessControl)
and `supervision.md` (concern 5 port-handoff, concern 6 zombie decision), and the
wave-3 intent ledger §A rows 32/62, §C OQ-1. Folds: #113 version-stamp division of
labour (concern 6), chassis-as-registrant (throughout), the update-flow mechanics
+ dual-registration window semantics + zombie feed (concern 7), OQ-1 hold (concern
8). No structural change to the record model or resolve policy.

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

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `service-lookup` — (any device/service ↔ mesh.service-registry) — the wiring seam: instance-keyed registration + slug resolution. → `scaffold/contracts/service-lookup.md`
  - Contract resolution (concern-5 dispute, RESOLVED against this file's
    original stance): each inference daemon self-registers a per-node
    `NodeScoped` instance under slug `inference` (keyed `(inference, node)`),
    and **`FleetAlias` is a resolve-time policy on the `inference` slug, not a
    stored synthetic record** — `resolve(AnyNode{inference})` → local
    completion-router, `resolve_all("inference")` → the per-node instances
    (the router's fleet membership), `resolve(Node{N,inference})` → node N's
    real endpoint. `FleetSlugNotRegisterable` is narrowed to guard only a
    caller writing a *stored* FleetAlias record. The body of this file was
    updated to the winning shape at harmonization.
- `service-registration` — (cc ↔ mesh.service-registry) — an instance of `service-lookup`. → `scaffold/contracts/service-registration.md`
- `kv-replication` (mesh daemon ↔ mesh daemon) — the registry is a keyspace TENANT of the one replication protocol; `registry-replication` is a superseded tombstone. → `scaffold/contracts/kv-replication.md`

Also a party to (cross-cutting): `restart-protocol` (registry flip in port-handoff), `surface-schema` — see `scaffold/contracts/`.

---

## Proposed contracts (wave 3)

This unit owns `service-lookup`; wave 3 makes three **shape-level clarifications**
to it (no wire-struct rename — the `RegistryRequest`/`RegistryResponse`/
`ServiceRecord` set is unchanged). Proposed to the harmonizer; details authored
into `scaffold/contracts/service-lookup.md` §Reconciliation notes:

1. **`FlipEndpoint` atomicity + the brief dual-registration window (INTENT #76).**
   The keyspace invariant is **≤1 Live `ServiceRecord` per `(slug, node)`**; a
   port-handoff is **one atomic LWW `put`** (bump `generation`, swap `endpoint`),
   never two coexisting Live records. `FlipEndpoint` returns the superseded
   `Endpoint` synchronously (supervision-driven path); a self-registering new
   instance produces the same cut via a fresh `Register` + a `ZombieSuspected`
   signal instead. **Reconciles `supervision.md` concern 5's "second distinct
   registry entry"** to "the same key's next `generation`" — flagged for the
   harmonizer; no keyspace change proposed (generation-in-key rejected as less
   boring, ledger §C).

2. **Sender version stamping (INTENT #113) — division of labour recorded on the
   edge.** `ServiceMeta.service_version` is the registry's version *inventory*
   (register/flip-time metadata, the read surface supervision consumes). The
   *per-message* stamp lives on `Provenance.service_version` (transport, chassis
   stamps it), and **version-floor enforcement + the `PleaseUpdate` back-signal is
   a receiver-EDGE policy owned by chassis / the per-contract edge** (`types::
   transport::{WireError code "version_below_floor", PleaseUpdate}`), NOT a
   `service-lookup` operation. Recorded so no reader mistakes the registry for the
   message-time gate. → cross-refs `contracts/mesh-transport.md`.

3. **Client half is `chassis` (INTENT #156, ledger D1).** Every `service-lookup`
   client operation (`register`/`renew`/`deregister`/`resolve`/`resolve_all`/
   `list`/`FlipEndpoint`) is a `chassis` sub-surface; `mesh-client` is retired.
   The contract's Parties line already reads `chassis`; this pins the component
   side to match.

**Held (not proposed): PARKED OQ-1.** No authority/blessing party is added to any
registry edge (concern 8). Registration stays a plain replicated-kv write.

