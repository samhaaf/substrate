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

**Wave-3 addendum.** This pass folds four things into mesh-core: (a) **universal
mediation** (INTENT #152) as an explicit kernel law — the *routing half* of the
"every message goes through mesh; a down target is restarted and the message
patched through; a target that can't answer now returns a promise" guarantee
(Concern 10; the *service half* is `chassis`); (b) the **promise routing table**
that carries a `PromiseFulfillment` back to the caller's return route (Concern
10); (c) the **brokered one-directional stream tunnels** for bulk transfers
(INTENT #153, Concern 11 — the direct off-relay channel is **AUTHORIZATION
PENDING**, OQ-30); and (d) the **observability serving plane** — the wave-2
`dashboard-serving` component is **folded in wholesale** as the internal
`dashboard` module (INTENT #46, ledger F9b; `dashboard-serving.md` is now a
tombstone pointing here — Concern 12). All four ride the batch-1 `mesh-transport`
`Frame`/`ResponseOutcome`/`Address`/stream vocabulary; mesh-core is that
contract's daemon-side party and adds no new wire structs of its own.

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
8. **Universal-mediation routing guarantees** (INTENT #152/#153) — the routing
   half of "uptime and guaranteed responses": patch a message through to a
   restarted target, relay a **promise** (and route its later fulfilment back to
   the caller), and **broker** scoped one-directional stream tunnels for bulk
   transfers. mesh-core owns the *routing* mechanics; `chassis` owns the *service*
   mechanics; `supervision` owns *whether/when* to restart. (Concerns 10–11.)
9. **The observability serving plane** (INTENT #46, folded from `dashboard-serving`)
   — the browser-facing dashboard HTTP/WS origin, the read-only event fan-out, the
   surface-schema aggregation pipeline, and the presentation rollups + node-scoped
   proxy, all inside the daemon. (Concern 12.)

**Boundary — what mesh-core does NOT own.** It does not implement any of the
utility algorithms: not the LWW/anti-entropy store (`replicated-kv`), not the
slug↔endpoint registry (`service-registry`), not semaphores (`locks`), not
queues/triggers/handlers (`queues`, which now also owns scheduling after the
wave-3 cron fold), not
version/boot-order/restart-ladder *policy* (`supervision`), not fleet completion
routing (`completion-router`), not topology diffing (`network-topology`), not
pub/sub topic delivery (`pubsub-relay`). mesh-core *composes* these and defines the
**seams** (Rust trait boundaries) they plug into. The **observability serving
plane** (the `dashboard` module — event fan-out, surface aggregation, rollups) is
a compiled-in Ring-4 module whose *aggregation algorithms* mesh-core-shell does
not itself implement, but whose **design now lives in THIS file** (Concern 12)
after the wave-3 F9b fold — it is no longer a separate component. It owns no application data
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
        · HttpSurface  — the browser-facing HTTP/WS listener (dashboard origin),
                         a SEPARATE port from the :3649 transport floor
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

Ring 4  (L2 composed) completion-router · dashboard (module)
        completion-router rides service-registry +
        network-topology (fleet discovery) + PeerLink (forwarding);
        the dashboard module (folded dashboard-serving, concern 12) rides
        service-registry (discover) + pubsub-relay (feed) + the HttpSurface seam.
        (cron is no longer a Ring-4 module — wave-3 folded its evaluator into
        Ring-3 `queues` as `TriggerSource::Schedule`; see queues.md concern 10.)
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
- `trait HttpSurface` — the browser-HTTP router-mount seam: the `dashboard`
  module *supplies* an axum `Router` (static assets + `GET /events` + `/api/*`)
  and mesh-core's shell owns the actual listener + port acquisition, so no
  internal lib binds its own socket. This is a **DEDICATED browser-HTTP/WS
  listener on a separate port** from the `:3649` mesh-transport floor (a browser
  cannot speak the `Hello`/`Welcome` handshake, and `mesh service open dashboard`
  must yield a real `http(s)://` URL). Acquired try-preferred-then-fall-back
  (INTENT #36; suggested default `:3648`, discovered-not-hardcoded), then
  registered as the `dashboard` slug. Consumed by the `dashboard` module
  (Concern 12). *This CONFIRMS the seam the wave-2 `dashboard-serving` design
  flagged as a co-batch friction point — now an in-file decision.*
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

A local service connects once to `ws://127.0.0.1:3649` (via `chassis`) and
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

The Dispatcher is also the enforcement point for **universal mediation** (INTENT
#152) — the "never service-to-service directly; a down target is restarted and
patched through; a busy target returns a promise" law is *routed here*. That
guarantee is the subject of Concern 10, which builds directly on this dispatcher
and on `supervision`.

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
8. **Acquire the browser-HTTP `HttpSurface` listener** (concern 12): a SEPARATE
   port from `:3649` via try-preferred-then-fall-back (default `:3648`,
   discovered-not-hardcoded, INTENT #36). Register mesh's own slugs — the
   `inference` fleet front-door alias, and the `dashboard` slug →
   `Endpoint{ scheme: Http, host, port, health_path: "/health" }`.
9. Ring 4: `completion-router` and the `dashboard` module (mount its
   axum `Router` — static + `/events` + `/api/*` — onto the `HttpSurface`).
   (cron's scheduler is no longer a Ring-4 module — it folded into Ring-3
   `queues`, started at step 7.)
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
timestamps via a `chassis` call or only the in-process rings consume the
authority is a fill-time decision. This is a **design item for mesh-core's
fill** — approach-sketched here, not implementation-ready.

### 10. Universal mediation — patch-through-on-restart + the promise routing half (INTENT #152)

The headline wave-3 fold. INTENT #152 verbatim: *EVERYTHING follows the loopback
pattern; never service-to-service directly; if a message targets a down service,
mesh RESTARTS it and patches the message through — "that's why we go through the
mesh: uptime and guaranteed responses"; if a service can't respond immediately,
mesh returns a PROMISE; the caller moves on; the value is pushed back over WS —
"all communication attempts are instant."* mesh-core owns the **routing half** of
this guarantee (the *service half* — issuing/awaiting the promise, forcing the
`Outcome::Promise` arm at compile time — is `chassis`'s; concern-split below).

**(a) The mediation law is single-port locality, already enforced.** Every
service message enters its **local** daemon on `:3649` (concern 3/2) and mesh does
the rest; no service opens a socket to another service or a remote node. The
Dispatcher (concern 2) resolves the target's `Address` (any-node/virtualized vs
pinned, INTENT #59) and relays. The only sanctioned exceptions are
**negotiated, one-directional bulk tunnels** (concern 11) — e.g. `aws`/`vfs`→S3
direct with a mesh-issued presigned grant (INTENT #114) — never a standing
service-to-service link.

**(b) Patch-through-on-restart — "guaranteed responses."** When the Dispatcher
resolves a target whose service is **down** on the resolved node, it does **not**
immediately error. Instead:
1. It calls `supervision`'s **`ensure_up(slug, node)`** (the boot-order/restart
   authority; the primitive is designed in supervision concern 11) to **ensure
   `slug` is up on `node`** — a fire-and-forget nudge, not a request/response
   call. `supervision` decides *whether/when* and drives the restart via
   mesh-core's `ProcessControl` (concern 4). mesh-core owns *executing* the
   spawn/kill; `supervision` owns the *policy*.
2. It **parks the inbound frame** in a **bounded** pending buffer keyed by
   `(target, frame_id)`, awaiting a readiness signal. Readiness is **not a pushed
   callback**: the Dispatcher OBSERVES `service-registry` and unparks the moment a
   fresh **`Live`** record (bumped `generation`) appears for `(slug, node)` — the
   ordinary lease `chassis` re-registers at its own bring-up. This is exactly the
   mechanism designed in supervision concern 11 (registry-observation, not a new
   channel).
3. On readiness it **patches the held frame through** to the new session via
   `LocalDelivery` (local) or `PeerLink` (remote). The caller experiences one
   response, never learning the target had bounced.
4. **Bounds (INTENT #38, no unbounded state):** a *patch-through deadline* and a
   bounded buffer. On exceed, the caller gets a `mesh-transport`
   `PeerUnreachable`/`Unroutable` error (catchable, never a silent hang). A large
   backlog sheds with `Backpressure`. Patch-through is a *hold*, not a queue —
   durable delivery is `queues`'s job, not the transport's (`mesh-transport`
   reconciliation note 5).

Patch-through and promises compose: a restarted target that still can't answer
synchronously simply resolves to a promise (below).

**(c) The promise routing half.** When a target can answer but not *now*, its
`chassis` returns `Reply::Promise`; the target's daemon emits an immediate
`ResponseOutcome::Promise{ promise, hint }` (`mesh-transport` §4) back through the
relay to the caller — so `request` resolves *instantly* to `Outcome::Promise`.
mesh-core's routing half then owns getting the eventual value home:

- **The promise routing table.** mesh-core records `promise_id -> return route`
  (caller `NodeId` + session/`conn_id` + the originating `correlate`) when it
  relays the `Promise` outcome. When the fulfilling service later emits a
  `PromiseFulfillment{ promise, outcome }` frame (`mesh-transport` `MsgKind`),
  the daemon looks up the route and relays the fulfilment to the right caller —
  including **cross-node** forwarding over `PeerLink`, and re-resolving the
  caller's live session if it reconnected under a new `conn_id`.
- **Division of labour with `chassis`'s PromiseRegistry (coordinate — chassis
  concern 4).** `chassis` holds the *service-side* registry: the callee's pending
  `promise` tokens and their reply routes, and the *caller-side* `PromiseHandle`
  the compiler forces every caller to handle. mesh-core holds only the
  *routing-side* table: which caller a `PromiseFulfillment` frame must be relayed
  to. The two never overlap — chassis owns *what the promise means and when it
  resolves*; mesh-core owns *where the resolution frame goes*. The promise
  **payload is opaque** to mesh-core (`mesh-transport` is payload-opaque); mesh
  routes the frame, never interprets it.
- **Resiliency, no silent drops (INTENT #155).** A promise resolution notice
  (and any intermediate value) rides the **distributed KV-backed
  intermediate-response cache** — owned by `pubsub-relay`/`replicated-kv` (batch
  2), NOT mesh-core — so a caller that disconnected before fulfilment still
  collects the value on reconnect. mesh-core routes fulfilments to live sessions
  and stashes-for-later via that cache; it does not implement the cache.
- **Delivery-persistence flag is read, not designed (PARKED OQ-3).** Whether a
  *failed* delivery is saved is the per-message `types::delivery::DeliveryPersistence`
  choice (INTENT #155; `types` `delivery.rs`, stamped on the promise ticket and
  every published event). mesh-core's routing **reads** the flag to decide
  save-vs-drop-on-failure and stops there. **WHERE a saved delivery goes — the
  enqueue-into-`queues` consolidation — is NEEDS-EXPLANATION and PARKED (OQ-3,
  "didn't seem very boring"); it is NOT decided here.** The mechanism must stay
  un-mergeable with a one-line change; mesh-core references the `delivery.rs`
  flag and no more.

**PARKED — OQ-1 (no authority in this routing design).** Patch-through and
promise routing are **pure mesh mechanics**: mesh restarts a target and relays
frames. Nothing here blesses a write, locks a condition, or depends on an
authority node. Consistency-blessing rides `chassis`'s separate **blessing queue**
against an abstract `BlessingTarget` (chassis concern 7) — mesh-core threads **no**
authority dependency into the routing plane, and the routing table carries no
blessing state.

### 11. Brokered one-directional stream tunnels (INTENT #153) — AUTHORIZATION PENDING (OQ-30)

For **specific large transfers only**, mesh **brokers** a connection *scoped to
one contracted event* (INTENT #152/#153) rather than round-tripping bulk bytes
through ordinary `Request`/`Response`. All stream traffic rides `mesh-transport`'s
`StreamOpen`/`StreamChunk`/`StreamClose` frames (§ Streaming); mesh-core is the
broker. Two modes share those frames:

- **Relayed stream (boring default, AUTHORIZED).** Chunks ride the ordinary
  daemon relay (local daemon → peer daemon → consumer). Because mesh sits in the
  path it can **observe, backpressure, and resume across interruption** (INTENT
  #152 "so it can handle interruption/resume"). The Dispatcher routes stream
  frames exactly like any pinned `Node{N}` frame, with **per-stream bounded
  buffers** (a full buffer raises `mesh-transport` `Backpressure`, INTENT #38 —
  never an unbounded queue). This is the path `vfs-content` (bulk blob pull),
  `kg-mesh` (merge-sync body), and `aws-vfs` (ciphertext chunks) already ride.
- **Direct brokered tunnel (AUTHORIZATION PENDING — OQ-30 / INTENT #153).** For
  raw byte-transparent bulk (raw `/v1` inference forwarding via
  `completion-router` / `v1-completion-api`; multi-GB model weights), mesh brokers
  a **node→node data channel on `PeerLink`** scoped to a single `StreamOpen`,
  taking the bytes **off the relay** (streams from the generating node straight to
  the consumer). mesh-core owns the broker: issuing the scoped channel, enforcing
  its lifetime, tearing it down at `StreamClose`. Hard rules locked *even while
  blessing is pending* (INTENT #153):
  - **ONE-DIRECTIONAL only** — one contracted event, one direction; anything
    two-directional goes **back through the mesh**. Never a permanent live
    connection; never bidirectional.
  - **Scoped + ephemeral** — brokered per `StreamOpen`, torn down at
    `StreamClose`; no service holds a standing peer socket.
  - Integrity/resume are the payload contract's (e.g. `vfs-content`'s
    content-addressed chunk verification), not the tunnel's.

**The streaming-channel operator AUTHORIZATION stays flagged pending (OQ-30).**
Per `mesh-transport`'s AUTHORIZATION note and INTENT #173d, the **direct off-relay
`PeerLink` data channel** is the seam OQ-30 reserves for the operator's explicit
blessing. The frames and broker mechanics are specified implementation-ready; the
direct channel **MUST NOT be built until blessed**. The relayed path is the boring
default and is **not** blocked. `completion-router` and `v1-completion-api` are the
named customers of the pending direct path; see `mesh-transport` § Streaming for
the frame shapes (mesh-core adds no new stream wire structs).

### 12. The observability serving plane — `dashboard-serving` folded in (INTENT #46; ledger F9b)

The wave-2 `dashboard-serving` component is folded here **wholesale** (ledger D5
F9b: "dashboard-serving → mesh-core HTTP + kv reads"). It is the internal module
`lib/mesh::dashboard`, **Ring 4** (rides Ring-3 `service-registry` + Ring-1
`pubsub-relay` + Ring-2 `replicated-kv` + the shell's Ring-0 `HttpSurface`). Prior
art: the killed V1 `bin/gateway` (folded into mesh at the 2026-07-18 gateway
merge). It owns four jobs and nothing else; the Svelte **frontend** is batch 6
(`dashboard.md`) and builds against the `dashboard-feed` seam below.

**Job 1 — Static asset origin.** Serve the compiled `ui/dashboard/dist/` from one
browser-reachable HTTP origin per node (via the shell's `HttpSurface` listener,
seam above), and register the `dashboard` slug so `mesh service open dashboard`
resolves to a real `http(s)://host:port` URL (INTENT #24/#36; boot step 8). The
dashboard is a first-class browsable registry entry; the `:3649` transport floor
stays pure (a browser cannot speak `Hello`/`Welcome`).

**Job 2 — The browser event feed (`GET /events`) IS pubsub-relay's protocol,
read-only.** The browser WS speaks the **same `types::pubsub` frames** as
`pubsub-relay` — same filters (incl. per-completion `Exact`), same lossy `Lagged`
semantics, same daemon-stamped `Envelope.provenance.origin_node` for the per-node
grid — with exactly one restriction: **Subscribe/Unsubscribe accepted; Publish
rejected** (a browser has no `service-registry` lease, so its provenance can't be
attested → `PubSubServerMsg::Error { NotRegistered }`, session stays open). The
`dashboard` module holds one in-process `pubsub-relay` subscription per browser
connection and pipes matched `Envelope`s out the socket — an authorization
profile of the same protocol, not a fork.

**Job 3 — No upstream scraping; pubsub-relay's interest routing absorbs it.** This
is the big simplification over V1 gateway (which opened a WS to each service on
each node). In v2, `inference`/`gc`/`cc`/`network-topology` **publish their own
event catalogs** onto the reserved `inference.*`/`gc.*`/`cc.*`/`network.*` topic
prefixes (their own designs); the `dashboard` module — running inside the *same*
daemon — simply **subscribes locally** with `Fleet`-scope prefix filters.
`pubsub-relay`'s cross-node interest-routed relay guarantees matching events from
*every* node reach this daemon, so the whole fleet's stream arrives from one local
subscription with **zero per-node/per-service socket bookkeeping**. It keeps NO
upstream connection pool, no reconnect loop, no scrapers — and runs in *every*
mesh daemon (Ring 4), so the always-on Pi keeps the dashboard live even when GPU
boxes sleep.

**Job 4 — Surface-schema aggregation (INTENT #46/#37).** Every service publishes a
boring `SurfaceSchema` (`types::surface`, via `chassis`) into a **replicated-kv
keyspace `surface/<slug> -> SurfaceSchema`** (keyed by slug, not node — a
service's render/interaction description is identical across nodes on the same
build). The `dashboard` module joins `service-registry.list()` (which slugs are
live) with the `surface/*` keyspace (their schemas) and the node roster into one
`DashboardManifest` served at `GET /api/surface`, re-publishing a small
`dashboard.surface.changed` notice when the schema set changes. Because the store
is replicated, this is a **local** KV read — no on-demand per-service fetch, no
fetch-failure surface ("anywhere you access mesh is the same"). Project-published
dashboards (INTENT #47) enter as ordinary `surface/<project-slug>` entries, so a
light `Vec<NavEntry>` nav grouping ({id, title, kind: MeshCore|Service|Project,
surface_slug}) makes them navigable with **no projects-specific code here** — the
mount point is designed, `projects` is not.

- **Version handling (INTENT #37, flagged OPEN).** `SurfaceSchema.v` keys a
  rendered component to a `(service, v)` pair. During a mixed-version rolling
  update the slug-keyed LWW store renders whichever `v` won convergence —
  acceptable for observability but tied to `supervision`'s OPEN mixed-version
  protocol (INTENT #66). Kept boring (slug-keyed, latest-wins); a `(slug, v)`
  composite key is the noted alternative. **Not silently resolved.**

**Presentation rollups + node-scoped proxy — aggregation MUST NOT become
routing.** `GET /api/nodes` (fleet roster from registry + network-topology +
replicated telemetry), `GET /api/mesh/stats`, `GET /api/nodes/:id/stats` (read
from the already-replicated `NodeInfo.last_state`, NOT by sysinfo-probing a remote
box — it can't and must not), and `ANY /api/nodes/:id/:slug/*path` — the
same-origin convenience proxy that resolves `(slug, id)` and hands **mesh-core's
Dispatcher** an `Address::Node{ node: id, slug }` `Request` (concern 2/10), doing
only the browser-HTTP ↔ mesh-`Request`/`Response` translation. The node `:id` and
`:slug` are **browser-supplied**, so the proxy makes **no placement decision**:
`completion-router` picks nodes; this proxy is told which one. It never issues a
fresh cross-host `reqwest` (the V1 `proxy.rs` mistake) — everything rides the
Dispatcher, preserving single-port locality.

**Boundary (unchanged from the folded design).** The `dashboard` module does not
define the pub/sub wire, topic taxonomy, or lossy policy (that is `pubsub-relay`);
does not define `SurfaceSchema`/`Envelope` (that is `types`); does not hold the
registry or manage leases (`service-registry` — it only *reads*); does not route
or pick nodes (`completion-router`). Access control is out of scope (INTENT #39).
It holds no application data and is a compiled-in module, never a standalone
service (INTENT #54).

## Relationships / edges

mesh-core's *contract* edges (cross-process WS/wire) are the daemon-shell seam;
its *internal-lib* relationships are compiled-in seams (§ Internal layering), not
contract edges. Contract edges, grouped:

- **every service ↔ mesh-core** via `mesh-transport` *(authored batch-1 — see
  Contracts section)* — the connect handshake + `Frame`/`Address` relay + the #154
  `ResponseOutcome` switch that `pubsub-protocol` and all higher protocols ride.
  Client half is **`chassis`** (which absorbed the former `mesh-client`, ledger
  D1); daemon half is mesh-core (`SessionMux`/`PeerLink`).
- **mesh-core ↔ every service** via `restart-protocol` *(authored — see Contracts section)* — the
  two-way 4-level graceful-restart / supervision choreography (interruptibility
  state, port-handoff). `supervision` (batch 2) reconciles the daemon side; the
  service side is **`chassis`**.
- **aws ↔ mesh-core** via `aws-mesh` — mesh-core registers `aws` like any service
  and relays the replication plane's S3/AWS leg through it; the replication
  *payload* is `replicated-kv`'s (batch 2). Consumer-side note only
  (see scaffold/contracts/aws-mesh.md).
- **mesh-core (the folded `dashboard` module) AUTHORS these edges** (moved here by
  the wave-3 F9b fold — formerly `dashboard-serving`'s): `dashboard-feed`
  (mesh/`dashboard` → the Svelte frontend — static hosting + read-only `GET /events`
  pub/sub WS + `GET /api/surface` manifest + rollups + node-scoped proxy);
  `surface-schema` (every service → the `dashboard` module — services publish their
  boring `SurfaceSchema` via `chassis`; the module aggregates + serves). The
  `inference-events` / `gc-events` / `cc-events` catalogs are re-grounded as
  `pubsub-protocol` topic prefixes (`inference.*`/`gc.*`/`cc.*`) the module
  subscribes to — mesh-core is the mesh-side subscriber, not a parallel wire.
- **sibling-owned edges mesh-core is the process host for but does NOT author:**
  `service-lookup` / `service-registration` (service-registry), `pubsub-protocol`
  (pubsub-relay), `network-events` (network-topology), `tailscale-status`
  (tailscale-query), `v1-completion-api` / `node-state-poll` (completion-router),
  `queues-api` / `locks-api` / `cron-api` (queues/locks/cron). mesh-core provides
  the transport and boot they ride; their wire shapes are their designers' to
  propose in batches 2–3.

## Nesting

Parent: (top-level app-crate) | Children (nested internal libs/modules in
`lib/mesh`, compiled in, never standalone): pubsub-relay, network-topology,
replicated-kv, service-registry, locks, queues, cron, supervision,
completion-router, and the **`dashboard`** module (the folded `dashboard-serving`,
`lib/mesh::dashboard` — its design now lives in Concern 12 of THIS file;
`dashboard-serving.md` is a tombstone). `tailscale-query` is a sibling *shared*
crate consumed by mesh-core but not nested. `mesh-client` **no longer exists** — it
retired into the top-level `chassis` shared-lib (ledger D1); the service-side
client half of `mesh-transport`/`restart-protocol`/`pubsub-protocol`/
`surface-schema` is `chassis`, not a mesh-nested crate. The parent/child structure
lives here and in overview.md, not in the directory layout (flat `components/`).

## Thoroughness level

**implementation-ready** for the shell proper — the ring architecture and the
DOWN-seam traits (now including `HttpSurface`), the `Address` model + dispatcher,
port acquisition/squatter-kill, zombie-kill + child re-adoption, stickiness, the
boot sequence, the local-SQLite decision, and the CLI tree are all decided and
specified. **implementation-ready** for the wave-3 folds: **universal mediation +
patch-through-on-restart + the promise routing half** (concern 10 — the routing
mechanics, the pending-frame hold, the promise routing table, and the
chassis/supervision division are specified; the `mesh-transport` wire it rides is
batch-1 authored), the **relayed** stream path (concern 11), and the **observability
serving plane** (concern 12 — carried over implementation-ready from the folded
`dashboard-serving`). **AUTHORIZATION PENDING** (OQ-30): the **direct off-relay
brokered stream tunnel** (concern 11) — frames + broker mechanics specified, but
the direct `PeerLink` data channel must not be built until blessed.
**approach-sketched** for the mesh **time authority** (concern 9 — INTENT #116, to
be pinned at fill). **NEEDS-EXPLANATION / PARKED** (OQ-3): the delivery-persistence
*mechanism* (enqueue-into-`queues`) — mesh-core reads the `types::delivery` flag
and stops. `aws-mesh` is **requirements-only** on mesh-core's side (consumer). The
`restart-protocol` daemon shape is authored (`supervision`-owned); the
`mesh-transport` byte-level framing is authored batch-1 (`types::transport`).

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

**Wave-3 folds — extra carve-outs.** (1) The **patch-through hold + promise
routing table** (concern 10) are the correctness-critical new mechanics — bounded
buffers, the readiness handshake with `supervision`, cross-node fulfilment
routing, and never leaking a pending entry (INTENT #38) — **strong model, do not
cheap-model.** (2) The **stream broker** (concern 11): the relayed path is
mechanical; the direct off-relay channel is AUTHORIZATION-PENDING and must not be
built. (3) The folded **`dashboard` module** (concern 12) is mostly boring
(tower-http `ServeDir`; a WS loop near-transcribed from the V1 `ws.rs`, now backed
by `trait PubSub`; JSON manifest assembly) — mid model OK — with two hand-careful
spots carried from the folded design: the **node-scoped proxy's HTTP↔mesh-`Request`
translation** must go through the Dispatcher (single-port locality), NEVER a fresh
`reqwest` to a remote host (the V1 `proxy.rs` mistake); and the **per-browser
`pubsub-relay` subscription lifecycle** (create/mutate/tear-down with no task
leak) is where lagged/dropped handling must be exact.

## Contracts (wave 2 authored + wave-3 folds)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them. Wave 3 adds the batch-1 `mesh-transport` file and moves
the `dashboard-feed` / `surface-schema` authorship home here (F9b fold).

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
- `mesh-transport` (every service ↔ mesh-core; mesh-core is the daemon-side
  party) — **now an authored standalone contract file** (wave-3 batch 1): the
  connect handshake, the `Frame` mux, `MsgKind`, the #154 `ResponseOutcome`
  switch, promise fulfilment (`PromiseFulfillment`), and the scoped one-directional
  stream frames. Struct vocabulary is homed in `types::transport` (batch 1);
  ~23 contracts (`pubsub-protocol`, `service-lookup`, `vfs-content`, `kg-mesh`,
  `aws-vfs`, `locks-api`, …) ride its `Frame`/`ResponseOutcome`/`Address` and
  surface its `PeerUnreachable`. mesh-core's concerns 2/10/11 are its daemon-side
  behaviour (routing, patch-through, promise routing, stream broker); the client
  half is `chassis`. The direct-brokered-stream channel is AUTHORIZATION PENDING
  (OQ-30). → `scaffold/contracts/mesh-transport.md`
- **AUTHORED by mesh-core's folded `dashboard` module (wave-3 F9b):**
  `dashboard-feed` (mesh/`dashboard` → dashboard frontend — the batch-6 seam) and
  the serving/aggregation half of `surface-schema` (every service → the
  `dashboard` module). → `scaffold/contracts/dashboard-feed.md`,
  `scaffold/contracts/surface-schema.md`. The `inference-events` / `gc-events` /
  `cc-events` catalogs re-ground onto `pubsub-protocol` topic prefixes
  (`inference.*`/`gc.*`/`cc.*`) the module subscribes to.
  → `scaffold/contracts/{inference,gc,cc}-events.md`
- Hosted-not-authored edges (mesh-core is the process host; sibling libs
  author): `service-lookup` / `service-registration` (service-registry),
  `pubsub-protocol` (pubsub-relay), `network-events` (network-topology),
  `tailscale-status` (tailscale-query), `v1-completion-api` / `node-state-poll`
  (completion-router), `queues-api` / `locks-api` / `cron-api`
  (queues/locks/cron). → `scaffold/contracts/`
