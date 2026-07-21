# mesh-core

**Status:** NEW (wave-2 fine-grained split of the round-9 `mesh.md`). The
app-crate `bin/mesh` + the root library `lib/mesh` — **the daemon shell of the
operating system**. `mesh.md` remains the round-1..9 integrated design of the
whole mesh; this file carves out the *kernel shell* proper and leaves the
internal utility libs to their own wave-2 designers (pubsub-relay, replicated-kv,
service-registry, locks, queues, cron, supervision, completion-router,
dashboard-serving, network-topology). **mesh-core owns the process, the port, the
one socket, the relay/addressing plane, the boot sequence, the internal-layering
seams, and the CLI — not the algorithms inside the libs it composes.**

## Charter

`mesh-core` is the single OS-kernel process every Mind OS service talks to. It IS
the `bin/mesh serve` daemon bound at LOCKED port **`:3649`**, running on every
device in the tailnet, plus the `mesh <noun> <verb>` operator CLI. Its charter is
the *discipline problem*, not an algorithmic one (INTENT #44/#55): compose a stack
of boring internal libraries into one correct process and give them a shared spine
to ride on. Concretely mesh-core owns, and owns *exclusively*:

1. **Process lifecycle on a fixed port** (INTENT #57) — acquire `:3649`, kill a
   port-squatter on restart, write/own the pidfile, and provide the process-kill
   and process-spawn primitives the rest of the OS uses (rigorous zombie-killing,
   supervision execution).
2. **Stickiness** (INTENT #57) — the system-process integration (launchd on macOS,
   systemd on Linux/Pi) that resurrects the daemon whenever it dies. mesh-core
   ships the unit and the `mesh install`/`mesh uninstall` commands. *A process
   cannot resurrect itself; the OS supervisor does. mesh supervises services;
   the system supervises mesh.*
3. **Single-port locality** (INTENT #58) — the one socket every local service
   connects to. Services never open sockets to each other or to other nodes; they
   hand mesh an addressed envelope and mesh relays it — inter-service AND
   inter-node. Services are unaware any other port exists.
4. **The two addressing classes** (INTENT #59) — *any-node/virtualized* ("service
   X, don't care which node") vs *specific-node/pinned* ("service X on node N"),
   plus the degenerate *local-only* class. mesh-core owns the `Address` vocabulary
   and the dispatcher that resolves each class into a delivery.
5. **The relay/session spine** — the outer transport frame (connect handshake +
   `Address` + correlation) that every higher protocol rides. The *pub/sub
   semantics* on top of this frame belong to `pubsub-relay`; mesh-core owns the
   frame, the session mux, and the daemon↔daemon peer links over the tailnet.
6. **The composition root + boot sequence** — the dependency-ordered wiring of all
   internal libs into one process (§ Internal layering), and re-adoption of
   orphaned child services after a mesh restart.
7. **The noun-verb CLI** (`mesh service open <slug>`, `mesh net status`, …,
   INTENT #24) — a short-lived client of the local daemon.

**Boundary — what mesh-core does NOT own.** It does not implement any of the
utility algorithms: not the LWW/anti-entropy store (`replicated-kv`), not the
slug↔endpoint registry (`service-registry`), not semaphores (`locks`), not
queues/triggers/handlers (`queues`), not scheduling (`cron`), not
version/boot-order/restart-ladder *policy* (`supervision`), not fleet completion
routing (`completion-router`), not the dashboard/observability plane
(`dashboard-serving`), not topology diffing (`network-topology`), not pub/sub
topic delivery (`pubsub-relay`). mesh-core *composes* these and defines the
**seams** (Rust trait boundaries) they plug into. It owns no application data
(that is `db`), no completion state (nodes' SQLite is the system of record), and
no cloud logic (that is `aws`). It does **not** import any L2+ app-crate as a
library — the internal libs are compiled-in nested libraries, not cross-app links
(INTENT #29/#45).

## Primary design concerns

### 1. The internal-layering architecture — the load-bearing deliverable

This is the design batches 2–3 build against, so it is specified concretely here.
Every internal lib is compiled into the one `bin/mesh` process; **none opens its
own socket** — they receive handles from the shell. The rings are strictly
bottom-up (a ring may consume only lower rings), the same boring-layers discipline
the whole repo uses (INTENT #55):

```
Ring 0  mesh-core SHELL (this module)
        · SessionMux   — the :3649 acceptor + per-connection frame mux
        · Dispatcher   — routes an inbound envelope by Address class + kind
        · PeerLink     — daemon↔daemon links to peers' :3649 over the tailnet
        · ProcessCtl   — spawn / kill / pidfile / adopt primitives
        · KvStore hand — opens the local SQLite file, hands a raw handle up
        · Bootstrapper — composition root + boot order + child re-adoption
        · Cli          — the noun-verb operator CLI

Ring 1  (L1 libs) pubsub-relay · network-topology
        pub/sub topic delivery over Ring-0 frames; tailscale-diff feed.
        network-topology + tailscale-query FEED Ring 0's PeerSet.

Ring 2  (L2 base) replicated-kv
        the eventually-consistent LWW store; persists to the Ring-0 SQLite
        handle, anti-entropies to peers via Ring-0 PeerLink (or Ring-1 relay).

Ring 3  (L2 on KV) service-registry · locks · queues · supervision
        all four ride replicated-kv's keyspaces (addressing / semaphore state /
        queue metadata / version+boot-order records).

Ring 4  (L2 composed) cron · completion-router · dashboard-serving
        cron rides registry+locks; completion-router rides service-registry +
        network-topology (fleet discovery) + PeerLink (forwarding);
        dashboard-serving rides service-registry (discover) + pubsub-relay (feed).
```

**Seams mesh-core DEFINES (the trait boundaries — the actual wave-2 output).**
These are library trait boundaries, deliberately *not* contract edges (INTENT #29
excepts compiled-in libs). Provided DOWN by the shell:

- `trait PeerTransport` — `peers() -> Vec<PeerHandle>`; `send(node_id, Frame)`;
  `on_frame(kind, Handler)`. Consumed by `replicated-kv` (anti-entropy),
  `pubsub-relay` (cross-node relay), `completion-router` (forwarding).
- `trait LocalDelivery` — `deliver(slug_or_session, Frame)` and
  `sessions_for(slug)`; deliver to a locally-connected service by its live
  session. Consumed by the Dispatcher and by any ring answering a local request.
- `trait ProcessControl` — `spawn(SpawnSpec) -> Pid`; `signal(Pid, Level)`;
  `adopt(Pid)`; `discover(endpoint) -> Option<Pid>`. Consumed by `supervision`
  (restart execution, zombie-killing, port-handoff) — mesh-core owns the OS-level
  kill; supervision owns *when/whether*.
- `trait LocalStore` — a raw local SQLite handle + namespaced keyspace opener.
  Consumed by `replicated-kv` only (everything else rides KV, not the file).
- `trait Subsystem` — the uniform lifecycle every ring implements
  (`start(&MeshContext) -> Handle`, `health()`, `shutdown(Deadline)`), so the
  Bootstrapper composes and tears down rings uniformly in dependency order.

Provided UP by the libs, consumed by the shell/CLI/siblings:

- `replicated-kv` → `trait KvHandle` (get/put LWW, subscribe-to-keyspace) —
  the substrate Ring-3 rides.
- `service-registry` → `trait Resolver` (`resolve(Address) -> Endpoint(s)`) — the
  Dispatcher's addressing lookups and `mesh service open/resolve` ride this.
- `pubsub-relay` → `trait PubSub` (publish/subscribe by topic + per-completion-id).
- `supervision` → `trait Supervisor` (boot-order plan, restart choreography).

**Why this is the crux:** get the ring boundaries and the five DOWN-traits right
once and every batch-2/3 lib is a boring fill against a frozen seam; get them
wrong and the whole kernel needs a structural refactor — exactly the technical
debt INTENT #38 forbids.

### 2. Single-port locality + the addressing dispatcher (INTENT #58/#59)

A local service connects once to `ws://127.0.0.1:3649` (via `mesh-client`) and
sends envelopes carrying an `Address`. The Dispatcher resolves each class:

```rust
enum Address {
    AnyNode  { slug: Slug },              // virtualized: don't care which node
    Node     { node: NodeId, slug: Slug },// pinned: this exact node
    Local    { slug: Slug },              // this node's local instance only
}
```

- **`AnyNode` + a fleet slug** (only `inference` today) → hand to
  `completion-router`, which picks a node and forwards. mesh registers the
  `inference` slug at its *own* `:3649` front door (see mesh.md Concern 1) — the
  fleet is never per-node slug-registered.
- **`AnyNode` + a singleton slug** → `Resolver.resolve` gives `owner_node`; if
  local, `LocalDelivery` to that service's session; if remote, `PeerLink.send` to
  that node's daemon, which does the local delivery there. *The local daemon is
  responsible for maintaining consistency; the caller never learns which node
  answered.*
- **`Node{N}`** → if `N == self`, local delivery; else relay to peer `N`.
- **`Local`** → deliver to the local instance only (used for node-scoped health,
  gc `:8430`, per-node polls), fail cleanly if not present locally.

Inter-node relay is daemon↔daemon over the tailnet (`PeerLink` to peers' `:3649`),
never service↔service or service↔remote-node directly. This is the property that
lets "anywhere you access mesh is exactly the same" (INTENT #35/#58) hold.

### 3. Port acquisition + squatter-killing (INTENT #57)

On `mesh serve` boot, bind `:3649`. On `EADDRINUSE`, **probe before killing**:
speak the mesh connect-handshake to `:3649`. Three outcomes:
(a) a *healthy mesh daemon of equal-or-newer version* answers → we are a duplicate
launch (a restart race); log and exit 0, do **not** kill a healthy daemon.
(b) a mesh daemon answers but is *stale/unresponsive/we hold `--force`* → graceful
port-handoff (signal it down via `ProcessControl`, then rebind).
(c) *a non-mesh process* squats the port → resolve its PID (`lsof -i :3649` /
`/proc/net`), `SIGTERM`→(grace)→`SIGKILL`, rebind. This is the literal "on restart,
mesh kills whatever process squats on its port." The squatter-kill (a *process on
my port*) is distinct from zombie-killing (concern 4) and lease expiry (a *stale
registry entry*, owned by service-registry).

### 4. Zombie-killing + child re-adoption (INTENT #57/#76)

mesh is the parent of the local services it starts (INTENT #76), so it holds their
PIDs. Two process-hygiene duties, both executed via `ProcessControl` but *policy-
driven by `supervision`*:

- **Zombie-killing:** when a service re-registers on a new endpoint (a port-handoff
  update, mesh.md Concern 12), the old still-running copy must be discovered and
  killed. Discovery: by held PID if mesh spawned it; else by probing the stale
  endpoint's identity/pidfile after the lease flips. "I don't want multiple copies
  of the same app running." Lease expiry removes the *entry*; this removes the
  *process*.
- **Re-adoption on mesh restart:** when launchd/systemd resurrects mesh, its former
  children were reparented to init. On boot the Bootstrapper enumerates running
  services (pidfiles + last-known registry endpoints), reconciles against the
  boot-order plan, and **adopts** live-and-correct ones rather than restarting the
  world. This keeps mesh restarts cheap and is a real correctness concern (a naive
  daemon would double-spawn every service on every restart).

### 5. Stickiness — system-process integration (INTENT #57)

mesh-core ships the resurrection unit and installs it:
- macOS: a launchd `LaunchDaemon` plist with `KeepAlive=true` (or per-user
  `LaunchAgent` when unprivileged).
- Linux/Pi: a systemd unit with `Restart=always`.
`mesh install` writes + loads the unit; `mesh uninstall` reverses it; `mesh serve`
is what the unit runs. The daemon exits non-zero on unrecoverable faults so the
supervisor restarts it; it exits 0 on the duplicate-launch race (concern 3a) so
the supervisor does *not* thrash. No Docker anywhere (INTENT #72) — this is native
OS supervision only.

### 6. Boot sequence (the daemon's own boot, dependency-ordered)

The Bootstrapper is the composition root; it brings rings up strictly bottom-up
and tears them down top-down:

1. Parse config (`mesh.toml`), init tracing.
2. **Acquire `:3649`** (concern 3): bind / kill-squatter / rebind; write pidfile.
3. Start Ring-0 `SessionMux` + `PeerLink` listeners (accepting, not yet routing to
   libs).
4. Open the **local SQLite** store file (concern 7) → `LocalStore`.
5. Ring 1: `network-topology` (+ `tailscale-query`) → seeds the `PeerSet`;
   `pubsub-relay`.
6. Ring 2: `replicated-kv` — restore persisted keyspaces from SQLite, begin
   anti-entropy with peers.
7. Ring 3: `service-registry`, `locks`, `queues`, `supervision` (all ride KV).
8. Register mesh's own slugs (the `inference` fleet front-door alias; the
   `dashboard` slug served by dashboard-serving).
9. Ring 4: `cron`, `completion-router`, `dashboard-serving`.
10. `supervision` reads boot-order records and starts/adopts local services in
    dependency order (concern 4).
11. Flip the Dispatcher live → accept and route service traffic. Announce ready.

### 7. mesh's own state store: embedded SQLite, NOT the `db` app (INTENT #31/#98)

mesh's kernel state (registry/lock/queue/version records, all inside
`replicated-kv`) persists to a **local SQLite file via embedded `rusqlite`** — not
the `db` crate. Two hard reasons: (a) `db` is an L4 app that must not be imported
as a lib (INTENT #29) and mesh is L1 — importing it would invert the layering and
create a boot cycle (db can't be up before the kernel that supervises it); (b)
SQLite-locally is locked (INTENT #98) and a daemon must persist to a *local* DB
(INTENT #31). This reconciles INTENT #31's "mesh becomes a db consumer" as: mesh
is the eventually-consistent, any-entry-point *access plane* (that IS
replicated-kv + single-port locality), not a literal importer of the db app for
its own kernel state. **Flagged as a friction point** — see below.

### 8. The noun-verb CLI (INTENT #24) — a short-lived local-daemon client

`bin/mesh` is a clap noun-verb tree (mirroring `bin/db`); the bare invocation and
`serve` keep daemon behavior:

```
mesh [serve] [--config mesh.toml]        # run the daemon (default)
mesh install | uninstall                  # (un)install the launchd/systemd unit
mesh service open <slug>                  # resolve slug -> URL, open in browser
mesh service resolve <slug> [--node N]    # print resolved endpoint (scripting)
mesh service list [--json]                # registered services + node + health + lease
mesh service register <slug> <endpoint> [--ttl D]
mesh service deregister <slug>
mesh net status [--json]                  # peers on/off + this device's connectivity
mesh net watch                            # tail the network-events feed
```

`mesh service open <slug>` is the headline ask: resolve via the local daemon's
`Resolver`, build the URL from the `{scheme,host,port,health_path}` endpoint, open
cross-platform (`open`/`xdg-open`). The CLI is just another local client of
`:3649` (reuses the registry query path); it surfaces a clean error, never a silent
hang, if no local daemon (or reachable peer) is up.

### 9. Mesh time authority — NEW design requirement (friction-round 1, INTENT #116), approach-sketched

**Mesh owns time.** The operator's friction-round hardening goes beyond
`replicated-kv`'s HLC ratchet (which stays): time itself is a kernel service,
"a consistent timestamp-based race-condition management system built into the
mesh, **as hardened as is physically possible**." Requirements, verbatim-grade:

- **Enforced UTC sync across devices** — mesh actively verifies/enforces that
  every node's clock is UTC-synced, not merely assumes NTP.
- **Possibly mesh-daemon-issued timestamps:** services/libs acquire timestamps
  FROM the local mesh daemon rather than each reading its own device clock —
  "mesh can handle its own device-specific offset by pinging its neighbors and
  keeping the times in sync" (neighbor-ping offset correction, NTP-style, over
  the existing `PeerLink`s).
- The corrected clock feeds everything timestamp-ordered: `replicated-kv`
  `Version`s (its HLC wall-clock input — see replicated-kv concern 1), `locks`'
  nanosecond-timestamped semaphore acquisitions (the queue-ownership discovery
  claim, INTENT #112), lease expiry, and provenance `emitted_at`.

**Approach sketch (to be pinned at mesh-core's fill):** a Ring-0
`TimeAuthority` beside `LocalStore`/`PeerTransport` — owns the node's offset
estimate (maintained by periodic neighbor pings over `PeerLink`, exchanging
send/receive timestamps and smoothing an offset, with the fleet converging on
a shared UTC view), exposes `now_utc()` (offset-corrected) and a monotonic
component for the HLC, surfaces per-node offset/drift on the surface schema,
and flags a node whose offset exceeds a threshold (dashboard alarm; possibly
refusing timestamp-sensitive operations). Whether services get daemon-issued
timestamps via a `mesh-client` call or only the in-process rings consume the
authority is a fill-time decision. This is a **design item for mesh-core's
fill** — approach-sketched here, not implementation-ready.

## Relationships / edges

mesh-core's *contract* edges (cross-process WS/wire) are the daemon-shell seam;
its *internal-lib* relationships are compiled-in seams (§ Internal layering), not
contract edges. Contract edges, grouped:

- **every service ↔ mesh-core** via `mesh-transport` *(authored — see Contracts section)* — the
  connect handshake + `Address` envelope + single-port-locality relay frame that
  `pubsub-protocol` and all higher protocols ride. Client half is `mesh-client`.
- **mesh-core ↔ every service** via `restart-protocol` *(authored — see Contracts section)* — the
  two-way 4-level graceful-restart / supervision choreography (interruptibility
  state, port-handoff). `supervision` (batch 2) reconciles the daemon side; the
  service side is `mesh-client`.
- **aws ↔ mesh-core** via `aws-mesh` — mesh-core registers `aws` like any service
  and relays the replication plane's S3/AWS leg through it; the replication
  *payload* is `replicated-kv`'s (batch 2). Consumer-side note only
  (see scaffold/contracts/aws-mesh.md).
- **sibling-owned edges mesh-core is the process host for but does NOT author:**
  `service-lookup` / `service-registration` (service-registry), `pubsub-protocol`
  (pubsub-relay), `network-events` (network-topology), `tailscale-status`
  (tailscale-query), `v1-completion-api` / `node-state-poll` (completion-router),
  `dashboard-feed` / `surface-schema` / `inference-events` / `gc-events` /
  `cc-events` (dashboard-serving), `queues-api` / `locks-api` / `cron-api`
  (queues/locks/cron). mesh-core provides the transport and boot they ride; their
  wire shapes are their designers' to propose in batches 2–3.

## Nesting

Parent: (top-level app-crate) | Children (nested internal libs in `lib/mesh`,
compiled in, never standalone): pubsub-relay, network-topology, replicated-kv,
service-registry, locks, queues, cron, supervision, completion-router,
dashboard-serving. `tailscale-query` and `mesh-client` are sibling *shared* crates
(promoted for reuse / to keep the client seam cheap), consumed by mesh-core but not
nested under it. The parent/child structure lives here and in overview.md, not in
the directory layout (flat `components/`).

## Thoroughness level

**implementation-ready** for the shell proper — the ring architecture and the five
DOWN-seam traits, the `Address` model + dispatcher, port acquisition/squatter-kill,
zombie-kill + child re-adoption, stickiness, the boot sequence, the local-SQLite
decision, and the CLI tree are all decided and specified. **approach-sketched** for
the mesh **time authority** (concern 9 — a NEW friction-round-1 design
requirement, INTENT #116, to be pinned at fill) and for
the two proposed contract *wire shapes* (`mesh-transport`, `restart-protocol`) —
their fields are recorded in the Contracts section's notes but the byte-level framing and the per-lib
message-kind registry are deliberately left to the per-pair contract round + the
sibling libs (`pubsub-relay` owns the inner frame; `supervision` owns the restart
policy). `aws-mesh` is **requirements-only** on mesh-core's side (consumer).

## Assigned design-depth

**Opus** single strong-model Component-Designer pass (this file), grounded in the
live `bin/mesh` + `lib/mesh/{lib,config,router,balancer,discovery,proxy}.rs`, the
round-9 `mesh.md`, and INTENT items 55/57/58/59/76/31.

## Suggested fill-model

Split by surface: the **shell/lifecycle** (SessionMux, PeerLink, ProcessCtl,
Bootstrapper, port/squatter/zombie logic, CLI) is implementation-ready + high
complexity → **mid model OK**, but the OS-integration details (launchd/systemd,
platform PID discovery, cross-platform kill) reward a careful pass — **mid-to-strong
model**, and this ring must be filled *before* any batch-2/3 lib since they all
compile against its seams. The **seam traits** themselves are a near-spec →
transcription-grade. Do not send the addressing/dispatcher core to a cheap model:
the `AnyNode`-singleton-remote relay path is the one subtle correctness spot.

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them.

- `restart-protocol` (mesh ↔ every service) — the LOCKED 4-level
  graceful-restart / interruptibility / port-handoff choreography.
  → `scaffold/contracts/restart-protocol.md`
  - Contract resolution supersedes the shape once proposed here: `supervision`'s
    daemon-side view is authoritative and the vocabulary is `types::restart` —
    `RestartPriority` (not `RestartLevel`), `save_deadline` as a
    `RestartRequest` field, three-state `Interruptibility { Idle,
    Interruptible, CriticalSection { until } }`, and `RestartResponse` replies.
  - Component-side split retained: `supervision` owns restart *policy*;
    mesh-core owns *execution* (port-handoff, kill, zombie-sweep, backed by
    `ProcessControl`).
- `aws-mesh` (aws ↔ mesh; mesh-core is the host/relay side) — mesh-core
  registers `aws` like any service and relays the replication plane's S3/AWS
  leg to it; the replication payload/policy is `replicated-kv`'s/`aws`'s,
  mesh-core contributes only registration + relay.
  → `scaffold/contracts/aws-mesh.md`
- `mesh-transport` (every service ↔ mesh-core; mesh-core-owned layer) — the
  connect handshake + `Address` envelope + single-port-locality relay frame all
  higher protocols ride. Not broken out as a standalone contract file at the
  contract round; the authored contracts bind to it by reference
  (`pubsub-protocol.md` Reconciliation note 6 names mesh-client's multiplexing
  wrapper the mesh-transport `Frame`; `service-lookup`, `vfs-content`,
  `kg-mesh`, `aws-vfs`, `locks-api` and others ride its
  `Envelope`/`Request`/`Response` and surface its `PeerUnreachable`). The
  byte-level wire shape remains mesh-core's to carry into fill.
- Hosted-not-authored edges (mesh-core is the process host; sibling libs
  author): `service-lookup` / `service-registration` (service-registry),
  `pubsub-protocol` (pubsub-relay), `network-events` (network-topology),
  `tailscale-status` (tailscale-query), `v1-completion-api` / `node-state-poll`
  (completion-router), `dashboard-feed` / `surface-schema` / `inference-events`
  / `gc-events` / `cc-events` (dashboard-serving), `queues-api` / `locks-api`
  / `cron-api` (queues/locks/cron). → `scaffold/contracts/`
