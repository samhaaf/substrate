# supervision

**Status:** NEW (wave 2). **Nesting:** internal lib of mesh (module
`lib/mesh::supervision`), Ring-3 in mesh-core's internal layering (rides
`replicated-kv` + `service-registry`, consumes mesh-core's `ProcessControl`
seam). **Prior art / grounding:** `mesh.md` concerns 6, 8, 12, 13; mesh-core's
Ring architecture + the five DOWN-seam traits (`ProcessControl`, `Subsystem`,
`Resolver`, `KvHandle`, `Supervisor`); `types::restart` (the LOCKED restart
message vocabulary); `mesh-client`'s restart-protocol client half. INTENT
#45/#57/#66/#76/#77/#86/#98.

## Charter

`supervision` is the **init system of the operating system** — the POLICY layer
that decides *what runs, in what order, and when it yields*, buried inside the
mesh daemon. It owns: (a) **per-node service-version tracking** and the
**pairwise inter-service version requirements** that drive everything else
("we just track the requirements and the boot order" — INTENT #45); (b)
**dependency-derived boot ordering** — a topologically-sorted plan that
mesh-core's Bootstrapper *executes* (concern 2); (c) the model of the
**observable interruptibility state** each service continuously exposes; (d) the
**LOCKED 4-level restart ladder** (wait-for-idle / finish-and-relinquish /
~10s-save-window / kill) run as the daemon side of the two-way `restart-protocol`
state machine; (e) **port-handoff update choreography** (start-new → registry
flip → down-old); (f) the **zombie-killing decision** (registry flags a stale
copy → supervision commands the kill); (g) **crash-restart policy with bounded
backoff and crash-loop detection**; (h) the **two-tier supervision-tree model**
(the system supervisor resurrects mesh; mesh supervises services); and (i) the
**pairwise-compatibility rolling-update flow**, including a concrete v1 answer to
the STANDING-OPEN mixed-version update problem (concern 9).

**Boundary — what supervision does NOT own.** It owns *policy and choreography*,
never OS primitives: the actual `spawn`/`signal`/`adopt`/`discover`/PID-kill,
the `:3649` bind, the port-squatter kill, and the pidfile are **mesh-core's
`ProcessControl` + Bootstrapper** (concern 8 — supervision decides *when/whether*,
mesh-core does the killing). The launchd/systemd unit authoring and
`mesh install`/`uninstall` that make mesh itself sticky are **mesh-core's
mechanism** — supervision owns only the *model* of that tree and the service-side
crash policy (concern 8). It does not store the version/requirement/lease records
— those live in **`replicated-kv`** under **`service-registry`**'s keyspaces
(supervision reads them in-process). It does not replicate those records —
that is **`registry-replication`** (a.k.a. `kv-replication`). It does not own the
restart message *structs* (**`types::restart`**) nor the *client half* of the
protocol (**`mesh-client`**). It is **not a package manager or installer**
(INTENT #45): it does not fetch, build, or place binaries — it consumes "a
compatible binary for slug X is present on this node" as an input; how binaries
arrive and get cached per-node is the git-based-distribution idea (INTENT #33),
explicitly out of scope and flagged OPEN there. It owns no application data
(`db`), no completion routing (`completion-router`), no relay (`pubsub-relay`).

## Primary design concerns

### 1. Service-version + pairwise-requirement model — declarative, KV-backed

Everything supervision does keys off two pieces of **declarative data** (INTENT
#45 "we just track the requirements and the boot order"; INTENT #103 "triggers
are declarative, never code" — the same principle: policy is data, not code).
Per node, per service:

```rust
// Held in replicated-kv under a supervision keyspace, written at registration
// (an additive facet of the `service-lookup` registration payload — flagged to
// service-registry below). Reads are eventually consistent (LWW, INTENT #32).
struct ServiceManifest {
    slug: Slug,
    version: SemVer,                    // this instance's own version
    requires: Vec<VersionRequirement>,  // pairwise deps: "I need Y within range R"
    boot_after: Vec<Slug>,              // explicit ordering hints (usually derived from `requires`)
    binary_id: Option<BuildId>,         // which cached build this is (INTENT #33 territory; opaque here)
    critical: bool,                     // does the node's usefulness depend on it (affects crash policy)
}
struct VersionRequirement { slug: Slug, range: SemVerRange } // e.g. inference "^2", db ">=1.4,<3"
```

The **requirement is pairwise** (INTENT #66: "that's why I like the pairwise
dependencies"), never a global release-set — commit-as-release-set was proposed
and NOT adopted. Requirements are the single source from which boot order,
compatibility restarts, and the mixed-version flow are all derived. This earns
its own concern because it is the data model the whole init system is a pure
function of — get it declarative and boring and the rest is mechanical.

### 2. Boot-order derivation — topological sort mesh-core executes

supervision derives the node's boot order by topologically sorting the local
services' `requires`/`boot_after` edges (only services *this node hosts*; a
requirement on a service that resolves to a *remote* node is a runtime routing
concern, not a local boot edge — see concern 9). It hands the ordered plan UP to
mesh-core's Bootstrapper via the `trait Supervisor` seam (mesh-core boot step 10):
**supervision plans, mesh-core spawns.** Concretely `Supervisor::boot_plan() ->
Vec<BootStage>` where each stage is a set of slugs safe to start in parallel
(same topological rank). Cycle detection is a hard error surfaced at boot
(`SuperError::DependencyCycle { slugs }`) — a cyclic requirement graph is a
design bug the operator must see, not something to silently break. On mesh
restart, the plan is reconciled against *already-running* services (mesh-core's
child re-adoption, concern 4 there): a live-and-correct service is **adopted, not
restarted** — supervision marks it satisfied and skips its boot stage.

### 3. Observable interruptibility state — the input to every restart decision

Every service continuously reports its interruptibility (INTENT #76 "each service
exposes an observable state so mesh can tell when it's doing something that
shouldn't be interrupted"). supervision consumes this as **live, volatile
per-service state** — deliberately NOT in replicated-kv (like pubsub interest, it
tracks the live connection and is rebuilt on reconnect; a stale replicated "busy"
flag would be a correctness hazard). It is delivered over the persistent
`mesh-client` connection as `restart-protocol` frames (concern 4). The vocabulary
is `types::restart::Interruptibility`:

```rust
enum Interruptibility {
    Idle,                                           // nothing in flight — free to restart
    Interruptible,                                  // working, but safely restartable
    CriticalSection { until: Option<DateTime<Utc>> },// must NOT interrupt for non-critical updates
}
```

supervision keeps a `HashMap<Slug, (Interruptibility, LastSeen)>`; a service that
stops reporting past a staleness window is treated as `Idle` for wait-for-idle
purposes but *unreachable* for graceful yield (escalates the ladder — concern 4).
A generic "service" here is anything that speaks the protocol — this is exactly
what lets **VDB later present a database as a supervised service** (INTENT #86,
concern 10): a database in a critical write is a `CriticalSection`.

### 4. The LOCKED 4-level restart ladder — the daemon-side state machine

The two-way `restart-protocol` (INTENT #77, ladder LOCKED round-9 at exactly four
levels — concern 13 of mesh.md). supervision owns the **daemon-side policy and
state machine**; `types::restart` owns the structs; `mesh-client` owns the client
callback; mesh-core's `ProcessControl` owns the kill. The ladder, canonical
naming from `types::restart::RestartPriority`:

| # | Level | Service participates? | supervision behaviour |
|---|-------|-----------------------|-----------------------|
| 1 | `WaitForIdle` | passively (Idle feed) | Wait until interruptibility == `Idle`; then proceed. No push needed. |
| 2 | `FinishAndRelinquish` | actively | Push `RestartRequest`; service finishes current work, sends `Yielding`→`Relinquished`; supervision proceeds on `Relinquished`. |
| 3 | `SaveWindow` | best-effort | Push with `save_deadline` (~10s); service saves; supervision proceeds on `Saved` OR when the deadline elapses, whichever first. |
| 4 | `Kill` | none | mesh-core `ProcessControl.signal(SIGKILL)`; unconditional; cannot fail. |

**Level selection** is a pure function of `(RestartReason, urgency, current
Interruptibility)`, decided here:
- `OperatorRequested` / routine `Update` → start at L1 (or L2 if the service is
  `Interruptible` but never idles).
- `Compatibility` → **always HIGH priority** (INTENT #77) → start at L3
  (save-window), because an incompatible pairing must not keep serving.
- `HealthRemediation` (crash-loop / wedged) → L3, escalate to L4 fast.
- `PortConflict` → L2/L3 per handoff (concern 5).

**Escalation is automatic and monotonic:** a level that misses its deadline (L2
`Relinquished` never arrives within a bound; L3 `save_deadline` elapses;
`ServiceUnreachable`) escalates exactly one step, to a hard ceiling of L4. This
is the single subtle correctness spot — supervision must *never* wait forever for
a service to yield (that would let one wedged service block a compatibility roll
across the fleet), and must *never* skip straight to kill when a graceful path
was requested and is progressing. The state machine is per-restart, keyed by
`request_id`.

### 5. Port-handoff update choreography (INTENT #76)

The update pattern is a fixed sequence supervision drives, reusing the registry's
LWW flip and mesh-core's `ProcessControl`:

1. **Spawn new** — `ProcessControl.spawn(SpawnSpec{ slug, version, port: NEW })`.
   The new instance registers itself (a *second*, distinct registry entry — same
   slug, new endpoint) and begins heartbeating; it comes up **not yet fronted**.
2. **Verify** — supervision waits for the new instance to be *healthy* (live
   lease + a passing health check against `Endpoint.health_path`) up to a
   `handoff_deadline`. If it never becomes healthy → **abort**: keep the old,
   kill the new, raise `SuperError::HandoffStalled { slug }` + alarm (dashboard,
   optionally ccd). No flip happens; the old instance was never disturbed.
3. **Flip** — write the registry LWW entry `slug -> new endpoint` (newest
   `version` wins; INTENT #32). Resolves now route to the new instance. This is
   the atomic cut-over; single-port locality means callers never notice.
4. **Relinquish old** — issue a `restart-protocol` signal to the OLD instance at
   the reason-appropriate level (L2 for routine, L3 for compatibility). It drains
   in-flight work behind the already-flipped front door, saves, and yields.
5. **Down old** — on `Relinquished`/`Saved` (or deadline), `ProcessControl.signal`
   the old port down; tombstone its (now superseded) registry entry.
6. **Backstop** — if the old copy lingers (didn't die), **zombie-killing**
   (concern 6) discovers and kills it. The handoff *deliberately* brings the old
   down; zombie-killing is the safety net (mesh.md concern 12).

This choreography is the generic mechanism VDB specializes into
copy/verify/switch-under-a-lock for databases (INTENT #86/#96, concern 10) — the
"verify then flip" shape is identical; VDB adds a `locks`-held write barrier
around steps 2–4.

### 6. Zombie-killing — the decision, not the kill

"I don't want multiple copies of the same app running" (INTENT #57). supervision
maintains the **should-be-running set** (the current boot plan ∩ live registry
entries this node owns) and reconciles it against **what is actually running**
(mesh-core's `ProcessControl.discover`). A process that is running but *not* in
the should-be-running set — a stale copy after a handoff, a re-registration on a
new endpoint (mesh.md concern 8), an orphan after a crash-restart — is a zombie:
supervision commands `ProcessControl.signal(pid, SIGTERM)`→grace→`SIGKILL`.
Three distinct hygiene mechanisms, kept separate on purpose:
- **squatter-kill** (mesh-core, concern 3 there): a *non-mesh* process on
  `:3649`. Not supervision's.
- **lease expiry** (service-registry): removes a stale *registry entry*. Not a
  process kill.
- **zombie-kill** (this concern): removes a stale *process*. supervision decides,
  mesh-core executes.

Discovery is by held PID when mesh spawned the service; else by probing the stale
endpoint's identity/pidfile after the lease flips (mesh-core concern 4). Killing
by endpoint alone is forbidden — supervision confirms process identity before
`SIGKILL` (never kill a freshly-rebound port's *new* owner).

### 7. Crash-restart policy — bounded backoff + crash-loop escalation

mesh-core signals a child's unexpected exit (`ProcessControl` exit notification).
supervision distinguishes **intentional stop** (part of a handoff / operator
deregister — do nothing) from **crash** (exit while in the should-be-running set)
and applies:
- **Bounded exponential backoff** restart: 200ms → cap (e.g. 30s), reset after a
  clean-uptime window. Never unbounded, never a tight respawn loop (INTENT #38 no
  technical debt).
- **Crash-loop detection:** N crashes within a rolling window → stop restarting,
  mark the service `Degraded`, raise `SuperError::CrashLooping { slug, count }`,
  and escalate — surface on the dashboard and hand a
  `ccd-escalation`-shaped investigation to CCD (the same escalation surface
  DLQ/loop-depth uses — INTENT #70/#89; flagged as a shared shape below, authored
  by queues/ccd, consumed here).
- **`critical` services** (concern 1) get a shorter backoff and louder alarm; a
  non-critical service's crash loop degrades quietly.

### 8. mesh's own stickiness — the two-tier supervision tree

*A process cannot resurrect itself.* The supervision model is two-tier and this
module owns the **model**, mesh-core owns the **mechanism**:
- **Tier 1 (system → mesh):** the OS supervisor (launchd `KeepAlive` on macOS,
  systemd `Restart=always` on Linux/Pi) resurrects the mesh daemon. supervision
  does NOT author the unit or run `mesh install` — that is mesh-core's concern 5.
  supervision's only stake: mesh's exit codes must distinguish
  *unrecoverable-fault* (exit non-zero → resurrect) from *duplicate-launch-race*
  (exit 0 → do not thrash) — a contract it shares with mesh-core's port
  acquisition (concern 3 there).
- **Tier 2 (mesh → services):** everything in concerns 2–7 — mesh is the
  supervisor of its local services.

Port-3649 squatter-killing on start is **mesh-core's** (concern 3 there);
supervision references it only as the boot precondition that makes Tier-2
supervision possible. **This is a deliberate deviation from the task brief's
framing** (which placed stickiness + squatter-kill inside supervision) — see
Friction points; the split follows mesh-core's already-designed exclusive
ownership of OS primitives, and keeps supervision purely policy.

### 9. Pairwise-compatibility rolling update + the mixed-version protocol (v1 answer to the STANDING-OPEN question)

INTENT #66 wants minimal-restart rolling updates with pairwise deps and leaves
**the update protocol between nodes running mixed versions OPEN**. Here is a
concrete, buildable v1 answer, flagged as such:

**The core move: there is no flag-day and no global barrier. Mixed-version
safety comes from additive-only contracts, and each node's supervision
independently reconciles a globally-eventually-consistent desired state.**

1. **Desired versions are replicated-kv state.** Each node writes its own
   installed `ServiceManifest` (concern 1); the fleet-desired version of a slug
   is, by default, the **newest** (LWW) — with an optional explicit **operator
   pin** (`version/pin/<slug> -> SemVer`) that overrides organic rollout. Reads
   are eventually consistent — a node may briefly not know a peer updated.

2. **Rolling, per-node, minimal restart.** When a node observes that a newer
   compatible binary for a slug it hosts is present (binary delivery itself is
   INTENT #33, OUT OF SCOPE — supervision consumes "present" as input), it runs
   a port-handoff update (concern 5) of *that service on that node only*. Other
   nodes update when they independently get the binary. A service is restarted
   **only if** (a) its own version changed, or (b) a pairwise requirement it
   declares is now violated by a local dependency's version — the "each service
   only restarts if it needs to" rule (INTENT #66). No node waits on another.

3. **Skew is safe by construction (the primary property).** Adjacent versions
   interoperate because every wire-crossing struct is additive-only, `serde
   (default)`, never `deny_unknown_fields`, with `#[serde(other)]` enum arms
   (`types` guardrail 4), and the pub/sub relay carries payloads **opaque**
   (pubsub-relay concern 2 — a new event type crosses an older daemon untouched).
   **Most updates therefore need zero cross-node coordination.**

4. **Explicit incompatibility gate (the rare breaking change).** For a genuinely
   breaking bump, the `VersionRequirement.range` excludes the old major. Two
   handling paths, keyed to the addressing class (INTENT #59):
   - **`AnyNode` (virtualized):** the local daemon's dispatcher, consulting
     supervision's version view of registry entries, **prefers routing to a
     fleet instance whose version satisfies the caller's requirement**. Skew is
     absorbed by routing — a caller needing `inference ^2` is sent to a node
     already on v2. This is the v1 default and the reason organic rollout works.
   - **`Node{N}` (pinned):** no routing latitude → if N's instance is
     incompatible, fail cleanly with a **catchable `SuperError::
     IncompatibleVersion { slug, have, need }`** (CAP-honesty sibling to `locks`'
     partition error, INTENT #84) — never silently mis-serve.

5. **The compatibility restart.** When a node's update leaves a *local*
   dependant's pairwise requirement unmet AND a compatible binary is present,
   supervision issues a **HIGH-priority (L3) compatibility restart** (INTENT #77)
   to roll the dependant to a compatible version. If no compatible binary is
   present, it marks the service `Degraded` and surfaces it (dashboard + ccd) —
   **never silently runs an incompatible pairing.**

6. **Still OPEN / flagged (not solved here):** (a) binary delivery + per-node
   build cache (INTENT #33) — supervision's hard dependency, out of scope; (b)
   organic-newest-wins vs an operator fleet-pin as the *default* rollout trigger
   — I propose organic default + optional pin, flagged for the operator; (c)
   whether a breaking roll should ever be *coordinated* (drain-all-then-flip)
   rather than routing-absorbed — I propose never-coordinate for v1
   (personal-mesh scale), flagged. See Open questions.

### 10. Databases-as-services genericity (INTENT #86) — design note, not a VDB design

The entire protocol is written against a **generic supervised service** =
`{ endpoint, version, Interruptibility, restart-protocol participation }`. Nothing
in concerns 3–9 is inference-specific. This is deliberate so VDB (batch 4) can
present **a database as a supervised service** under this exact protocol
(INTENT #86/#96): a database's write transaction is a `CriticalSection`; VDB's
copy/verify/switch-under-a-lock (SQLite→Postgres promotion, INTENT #86) is the
concern-5 port-handoff specialized with a `locks`-held write barrier around
verify→flip. supervision designs the generic shape; it does **not** design VDB —
it only guarantees the protocol is general enough to inherit.

## Relationships / edges

supervision's cross-process **contract edge** is `restart-protocol`; everything
else it touches is an **in-process sibling-lib seam** (compiled into `lib/mesh`,
not a contract edge — INTENT #29/#45).

- **every service ↔ mesh** via `restart-protocol` — the two-way 4-level
  graceful-restart / interruptibility / port-handoff choreography. supervision is
  the authoritative **daemon-side** party; the struct home is `types::restart`,
  the client half is `mesh-client`, the kill/handoff execution is mesh-core.
  Cross-cutting, surface-schema-style (one shared document, every service a
  party). *(scaffold/contracts/restart-protocol.md — MISSING, to be created;
  proposed below.)*
- **service-registry** (sibling lib) — supervision **reads** version/requirement/
  lease/endpoint records and **writes** the LWW registry flip during a handoff.
  In-process via `trait Resolver` + a KV keyspace, NOT a contract edge. Proposes
  an *additive facet* of the `service-lookup` registration payload (version +
  `requires`) — flagged to service-registry, authored there.
- **replicated-kv** (sibling lib) — the `ServiceManifest`/version keyspace
  persistence + anti-entropy. In-process `trait KvHandle`; replication is
  `registry-replication`/`kv-replication` (not supervision's contract).
- **mesh-core** (parent shell) — consumes `trait ProcessControl`
  (spawn/signal/adopt/discover) and provides `trait Supervisor` (boot plan,
  restart choreography) UP to the Bootstrapper. In-process trait seams.
- **ccd** via `ccd-escalation` (consumed, not authored) — crash-loop /
  incompatibility investigations ride the same escalation shape as DLQ /
  loop-depth (INTENT #70/#89). Authored by queues/ccd; supervision is a producer.
- Imports `substrate-types` (`restart`, `node`, `id`, `error` vocabulary) — a
  shared-lib dependency, NOT a contract edge.

## Nesting

Parent: mesh | Children: none. Module `lib/mesh::supervision`, Ring-3 in
mesh-core's internal layering (mesh-core §1): rides `replicated-kv` (Ring 2) +
`service-registry` (Ring 3 peer), consumes mesh-core Ring-0 `ProcessControl`,
and is composed by the Bootstrapper via `trait Subsystem`/`trait Supervisor`.
Never a standalone crate (INTENT #54). The client half of its protocol is
`mesh-client`, a separate shared lib.

## Thoroughness level

**implementation-ready** — the `ServiceManifest`/requirement model, boot-order
derivation, the interruptibility state model, the 4-level ladder state machine +
level-selection + escalation rules, the six-step port-handoff choreography, the
zombie-kill decision boundary, the crash-restart backoff + crash-loop policy, the
two-tier supervision-tree split, and a concrete v1 mixed-version protocol are all
decided and specified against frozen seams (mesh-core's `ProcessControl`/
`Supervisor`, `types::restart`, `service-registry`'s LWW registry). **Genuinely
downstream / left open:** (a) binary delivery + build cache (INTENT #33) — an
out-of-scope hard dependency, not a gap in this module; (b) the three flagged
mixed-version policy choices in concern 9.6 (operator's call); (c) exact backoff
constants + crash-loop window (fill-time tuning knobs, not design forks); (d) the
`restart-protocol` wire is `approach-sketched` here and reconciled in the per-pair
round with `types`/`mesh-client`/mesh-core.

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), grounded in `mesh.md`
(concerns 6, 8, 12, 13), `mesh-core.md` (Ring architecture + `ProcessControl`/
`Supervisor`/`Subsystem` seams), `types.md` (`restart.rs`), `mesh-client.md`
(restart-protocol client half), `service-registry.md`, and INTENT
#45/#57/#66/#76/#77/#84/#86/#98.

## Suggested fill-model

**implementation-ready + high complexity → mid-to-strong model.** The version/
requirement model, boot-order topo-sort, and interruptibility bookkeeping are
mechanical (mid model OK). Two spots reward a careful hand: (1) the **ladder
state machine's escalation** (concern 4 — never-wait-forever vs never-skip-a-
graceful-path is the subtle correctness edge) and (2) the **port-handoff
abort/rollback path** (concern 5 step 2 — verify-then-flip with a clean abort
that never disturbs the old instance). Do not send the mixed-version routing gate
(concern 9.4) to a cheap model — the AnyNode-prefer-compatible-instance path is
the one place a wrong default silently mis-serves. Sequence **after** `types`,
`service-registry`, and `replicated-kv` are filled (it rides all three) and
**alongside** mesh-core's `ProcessControl` (it commands it).

---

## Proposed contracts (wave 2)

wave2-plan §3b assigns `supervision` exactly one contract pair: **`restart-protocol`**
— "mesh ↔ every service — the two-way 4-level graceful-restart / supervision
protocol (interruptibility state, port-handoff choreography)." Modeled as ONE
shared document naming every service as a party (surface-schema precedent,
wave2-plan §5.4), not N per-pair files. Proposal only; the per-pair round
reconciles it with the concurrent `types` (struct home), `mesh-client` (client
half), and mesh-core (execution) proposals — I consume their shapes and note
deltas rather than inventing parallel structs.

### Contract: `restart-protocol`

**Purpose.** The two-way graceful-restart / supervision choreography built into
EVERY service from the beginning (INTENT #76/#77, ladder LOCKED at 4 levels).
The service continuously reports interruptibility; mesh (supervision, the daemon
side) signals a restart need at a laddered level and drives port-handoff + kill.
This document is the **daemon-side (authoritative) view**; it rides the
`mesh-client` persistent WS connection (a persistent bidirectional socket is
mandatory because mesh must *push* an unsolicited restart signal — mesh-client
concern 1) and multiplexes over the `pubsub-protocol` envelope as typed control
frames.

**Structs — consumed from `types::restart` (I do not redefine; canonical naming).**
I adopt `types::restart` verbatim as the vocabulary and flag two naming/shape
reconciliations to the harmonizer:

```rust
// from types::restart — the LOCKED 4-level ladder (ordered)
enum RestartPriority { WaitForIdle, FinishAndRelinquish, SaveWindow, Kill }
enum RestartReason   { Compatibility, Update, OperatorRequested, HealthRemediation, PortConflict }
struct RestartRequest {
    v: u16, request_id: Uuid, priority: RestartPriority,
    reason: RestartReason, save_deadline: Option<Duration>, requested_at: DateTime<Utc>,
}
enum RestartResponse {
    Acknowledged { will_yield_by: Option<DateTime<Utc>> },
    Busy { state: Interruptibility, retry_after: Option<Duration> }, // honored ONLY below SaveWindow
    Yielding,                    // finishing-then-relinquishing in progress (L2)
    Saved,                       // state persisted, ready to be replaced (L3)
}
enum Interruptibility { Idle, Interruptible, CriticalSection { until: Option<DateTime<Utc>> } }
struct PortHandoff { service: Slug, old: SocketHint, new: SocketHint, at: DateTime<Utc> }
struct SocketHint  { host: String, port: u16 } // pure data, NOT a live socket
```

**Reconciliation flags (for the per-pair round):**
- `types` calls level `RestartPriority`; mesh-core/mesh-client sketches called it
  `RestartLevel`. Adopt **`RestartPriority`** (types is the struct home). One
  name, fleet-wide.
- mesh-client's client half models `SaveWindow { deadline: Duration }` as an
  in-variant field; `types` carries the deadline as `RestartRequest.save_deadline`.
  Adopt the **`types` shape** (deadline on the request), so the ladder enum stays
  a clean 4-arm LOCKED enum. Flagged.
- mesh-client adds a `RestartReason::Other(String)` / `PortHandoff` reason arm;
  `types` uses a closed set. Keep the closed set + a reserved `#[serde(other)]`
  catch-all for forward-tolerance (guardrail 4) rather than an open `Other(String)`
  — matchability matters for level selection. Flagged.

**Daemon → service (supervision-driven, unsolicited push):**
```rust
enum SupervisionServerMsg {
    Restart(RestartRequest),                 // signal a restart need at a level
    PortHandoffNotice(PortHandoff),          // FYI: your slug's front door moved (old copy)
    PollInterruptibility,                    // request an immediate state report (rare; feed is push-default)
}
```

**Service → daemon (client half, `mesh-client`):**
```rust
enum SupervisionClientMsg {
    InterruptibilityUpdate { state: Interruptibility }, // continuous, on change (concern 3)
    RestartReply(RestartResponse),                      // Acknowledged/Busy/Yielding/Saved
    Manifest(ServiceManifest),                          // version + `requires`, at registration (concern 1)
}
```

**Error cases (`SuperError` — supervision's domain sub-enum in `types::error::supervision`,
matchable, never a panic — CAP honesty INTENT #84):**
- `IncompatibleVersion { slug, have, need }` — pinned (`Node{N}`) resolve of a
  version outside a caller's pairwise requirement (concern 9.4); catchable,
  handled per-application. AnyNode prefers a compatible instance instead of
  raising this.
- `HandoffStalled { slug }` — new instance never became healthy within
  `handoff_deadline`; old kept, no flip (concern 5 step 2).
- `RestartTimeout { slug, request_id }` — a graceful level missed its deadline;
  drives one-step escalation (concern 4), surfaced for observability.
- `ServiceUnreachable { slug }` — never acked / interruptibility feed stale past
  window → escalate toward L4 + zombie-sweep.
- `DependencyCycle { slugs }` — cyclic `requires` graph at boot-plan derivation
  (concern 2); hard, operator-visible.
- `CrashLooping { slug, count }` — crash-loop threshold hit (concern 7); stops
  restarts, marks `Degraded`, escalates to ccd.
- **Non-errors by design:** a `Busy` reply below `SaveWindow` is a normal outcome
  (supervision waits/escalates), not an error; at `SaveWindow`/`Kill` a `Busy`
  reply is *ignored*; a service already `Idle` on an L1 request needs no push.

**Version-sensitivity.**
- The **4-level ladder is LOCKED** — the `RestartPriority` enum is stable; adding
  or removing a level is a breaking change requiring an operator round. Newer
  daemons MUST NOT introduce a 5th level a client can't parse; an unknown level,
  if one ever crossed the wire, MUST default to the **most conservative
  interpretation** (`SaveWindow`, save-and-yield) — never ignore a restart signal
  (mesh-client concern 7). `RestartReason` grows additively behind `#[serde(other)]`.
- `RestartRequest.v` / `PortHandoff` carry the wire-crossing `types` guardrail-4
  discipline (`serde(default)`, no `deny_unknown_fields`) because a daemon on
  node A may relay a supervision control frame toward a service reached through
  node B during the skew window — though restart is *primarily node-local*
  (a service is supervised by its OWN local daemon).
- **This contract is coupled to the mixed-version protocol (concern 9):** a
  breaking `pubsub-protocol`/`mesh-transport` `proto` bump is precisely what
  drives `RestartReason::Compatibility` restarts across nodes. That coupling —
  the mechanism by which the whole fleet rolls to a new transport floor — is the
  concern-9 v1 answer; the STANDING-OPEN part (binary delivery, INTENT #33) is
  flagged, not resolved here.

**Example (one compatibility roll, cross-node, shared example world).**
```jsonc
// operator's laptop "mba-01" hosts inference@2.1.0 and db@1.4.0; a new
// inference@3.0.0 binary lands (proto major bump). supervision on mba-01:
//   1. port-handoff: spawn inference@3.0.0 on a new port, verify healthy,
//      flip registry "inference" -> new endpoint.
//   2. db@1.4.0 declares requires inference "^2" — now violated by 3.0.0.
//      A compatible db@2.0.0 binary IS present -> HIGH-priority compat restart:
{
  "v": 1, "request_id": "r-9c2",
  "priority": "SaveWindow",
  "reason": "Compatibility",
  "save_deadline": { "secs": 10, "nanos": 0 },
  "requested_at": "2026-07-19T18:20:00Z"
}
// db replies, having flushed its outbox:
{ "type": "Saved" }
// meanwhile a caller resolving "inference" AnyNode on peer "pi-01" (still on
// inference@2.1.0, requires-compatible) is simply routed to pi-01's instance —
// no error, skew absorbed by routing (concern 9.4). A *pinned* Node{mba-01}
// resolve from a "^2"-requiring caller during the window would instead get:
// SuperError::IncompatibleVersion { slug:"inference", have:"3.0.0", need:"^2" }
```
