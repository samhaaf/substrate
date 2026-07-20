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

> **Per-app semantics deferred to a PHILOSOPHY (friction-round 2, INTENT
> #119).** *What* each service reports as `Idle` vs `CriticalSection` — the
> per-app idle/critical semantics question, including inference's
> benchmark-as-Idle item — is NOT ruled on in the scaffold. "The four restart
> levels should be done on an app-by-app basis. We should focus on a
> philosophy about restart interrupt signals" — that philosophy will be
> developed in a **dedicated harness plugin** (for working on substrate) via
> the **critic pattern: Opus proposes, Fable critiques, Opus synthesizes.**
> Until it lands, per-app mappings in the component files (notably
> inference.md concern 4's benchmark-as-Idle) stand **applied provisionally,
> pending the restart philosophy.** The four-level ladder itself remains
> LOCKED; only the per-app signal semantics are philosophy-governed.

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

### 9. Pairwise-compatibility rolling update + the mixed-version protocol — CONFIRMED (friction-round 1, INTENT #113)

INTENT #66 wants minimal-restart rolling updates with pairwise deps and left
**the update protocol between nodes running mixed versions OPEN**. The v1
answer below is now **CONFIRMED by the operator** — his framing: "the
conversation is moot" — **the restart-priority system IS the answer**, tied
explicitly to the LOCKED 4-level ladder (concern 4): **non-critical updates
wait for idle** (`WaitForIdle`/routine levels — mixed versions in the interim
are fine, that's the point of the design); **critical incompatible updates go
out to every instance at HIGH priority** (the L3 `Compatibility` push, INTENT
#77). Organic-newest-wins rollout stands. On top of it the operator added a
NEW LOCKED cross-cutting requirement — **sender version stamping on every
mesh-crossing message** (name + version, receiver version floors, one-version
back-compat + please-update warning) — homed in `types`/`pubsub-protocol`/
`queues-api`, consumed here as the data that makes version floors enforceable.
The protocol:

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

6. **Dispositions after friction-round 1 (INTENT #113):** (a) binary delivery +
   per-node build cache (INTENT #33) — still OPEN, supervision's hard
   dependency, out of scope; (b) organic-newest-wins as the default rollout
   trigger (+ optional operator pin) — **CONFIRMED** ("the conversation is
   moot"; the restart-priority ladder is the mechanism); (c) coordinated
   (drain-all-then-flip) breaking rolls — **CONFIRMED never-coordinate**: a
   critical incompatible update is instead a **high-priority (L3) push to
   every instance**, which is the ladder doing the coordination organically.

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
  party). *(authored: scaffold/contracts/restart-protocol.md)*
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
two-tier supervision-tree split, and the **operator-confirmed** v1 mixed-version
protocol (friction-round 1, INTENT #113) are all
decided and specified against frozen seams (mesh-core's `ProcessControl`/
`Supervisor`, `types::restart`, `service-registry`'s LWW registry). **Genuinely
downstream / left open:** (a) binary delivery + build cache (INTENT #33) — an
out-of-scope hard dependency, not a gap in this module; (b) — resolved: the
concern-9.6 policy choices are confirmed per INTENT #113; (c) exact backoff
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

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `restart-protocol` (mesh ↔ every service) — the 4-level ladder, interruptibility feed, boot ordering, port-handoff choreography (supervision is the daemon side). → `scaffold/contracts/restart-protocol.md`

Also a party to (authored elsewhere / cross-cutting): `pubsub-protocol` — see `scaffold/contracts/`.

