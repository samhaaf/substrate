# supervision

**Status:** REFRESHED (wave 3; NEW at wave 2). **Nesting:** internal lib of mesh
(module `lib/mesh::supervision`), Ring-3 in mesh-core's internal layering (rides
`replicated-kv` + `service-registry`, consumes mesh-core's `ProcessControl`
seam). **Prior art / grounding:** `mesh.md` concerns 6, 8, 12, 13; mesh-core's
Ring architecture + the five DOWN-seam traits (`ProcessControl`, `Subsystem`,
`Resolver`, `KvHandle`, `Supervisor`); `types::restart` (the LOCKED restart
message vocabulary); `chassis`'s restart-protocol client half (the wave-3
successor to `mesh-client`'s). INTENT #45/#57/#66/#76/#77/#86/#98.

**Wave-3 addendum.** This pass reconciles supervision against three wave-3
siblings and folds one philosophy-fold clarification: (a) **the mesh-core
coordination seam** — `mesh-core.md` concern 10's patch-through-on-restart
names an `ensure_up`-shaped ask and a "readiness signal" it needs from
supervision without specifying either; concern 11 below designs both, picking
the boring primitive (observation over the existing `service-registry` lease
flip, not a new push callback); (b) **chassis as the confirmed service-side
party** — `chassis.md` concern 5's `RestartPolicy`/`Wound` seam is reconciled
against `restart-protocol.md`'s wire contract (drift found and fixed there:
the contract still carried the pre-chassis `RestartParticipant`/`Yielded`
sketch); (c) the **#156 severity-in/service-decides doctrine** is made
explicit here as a first-class framing, not just implied by the ladder table;
(d) concern 9's mixed-version framing is corrected to distinguish what INTENT
#113 actually SETTLED (the ladder + version stamping/floors ARE the
mechanism) from what remains **PENDING BLESSING** (ledger OQ-31: the specific
organic-newest-wins v1 rollout protocol built on top). Grounding added:
`chassis.md` (wave 3), `mesh-core.md` (wave 3) concern 10,
`contracts/mesh-transport.md`, ledger OQ-27/OQ-31/§C.

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
protocol (**`chassis`**, wave 3's successor to `mesh-client`). It is **not a package manager or installer**
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
`chassis` connection as `restart-protocol` frames (concern 4). The vocabulary
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
state machine**; `types::restart` owns the structs; `chassis` owns the client
callback (the `RestartPolicy` seam, wave 3 — see `restart-protocol.md`'s
reconciled client-side contract); mesh-core's `ProcessControl` owns the kill. The ladder, canonical
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

> **The severity-in / service-decides-how doctrine (INTENT #156, made explicit
> here).** "The service has to determine how it shuts itself down. Requests are
> requests." The three things supervision ever puts on the wire are the
> *severity* (`RestartPriority`), the *reason*, and (at L3) a *deadline* — never
> a prescription of *how* the service should wind down. Framed as a single rule:
> **supervision REQUESTS, it never FORCES compliance below L4.** L1–L3 each give
> the service a real say — wait for it to report `Idle` on its own schedule
> (L1), let it finish and relinquish on its own timing (L2), give it a save
> window it controls the use of (L3) — and a service that misses its window is
> not punished mid-negotiation, it is simply escalated one step (above) or, at
> the L4 ceiling, killed outright with no further latitude. L4 is the one and
> only level that is a pure force, by design — the ladder's whole point is that
> everything *before* L4 is a request. `chassis.md` concern 5 and
> `restart-protocol.md`'s reconciled client-side contract are the service-side
> statement of this same doctrine — see there for the concrete `RestartPolicy`
> seam.

### 5. Port-handoff update choreography (INTENT #76)

The update pattern is a fixed sequence supervision drives, reusing the registry's
LWW flip and mesh-core's `ProcessControl`:

1. **Spawn new** — `ProcessControl.spawn(SpawnSpec{ slug, version, port: NEW })`
   in handoff/standby mode. The new instance comes up **not yet fronted** and
   **defers self-registration**: the registry holds **exactly one Live record per
   `(slug, node)`** (service-registry concern 7), so there is never a second Live
   entry — the "dual" here is only a brief *process*-level window (two processes
   alive), not a keyspace one. supervision verifies the new instance by probing
   its port directly (it knows the port from `SpawnSpec`), not via a registry
   entry.
2. **Verify** — supervision waits for the new instance to be *healthy* (live
   lease + a passing health check against `Endpoint.health_path`) up to a
   `handoff_deadline`. If it never becomes healthy → **abort**: keep the old,
   kill the new, raise `SuperError::HandoffStalled { slug }` + alarm (dashboard,
   optionally cc). No flip happens; the old instance was never disturbed.
3. **Flip** — issue `FlipEndpoint(slug, node, new)`: **one LWW `put`** on the
   single `(slug, node)` key that swaps the endpoint old→new and bumps
   `generation` (newest wins; INTENT #32), returning the superseded endpoint
   synchronously so supervision knows which port to down (service-registry
   concern 7). Resolves now route to the new instance — no intermediate state
   where the slug points at nothing or at both. This is the atomic cut-over;
   single-port locality means callers never notice.
4. **Relinquish old** — issue a `restart-protocol` signal to the OLD instance at
   the reason-appropriate level (L2 for routine, L3 for compatibility). It drains
   in-flight work behind the already-flipped front door, saves, and yields.
5. **Down old** — on `Relinquished`/`Saved` (or deadline), `ProcessControl.signal`
   the old port down. There is no separate registry entry to tombstone — the
   single `(slug, node)` record already points only at the new endpoint after the
   flip; the old endpoint is simply unreferenced, and its `generation` bump feeds
   zombie-detection (service-registry concern 7).
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
  `cc-escalation`-shaped investigation to cc (the same escalation surface
  DLQ/loop-depth uses — INTENT #70/#89; flagged as a shared shape below, authored
  by queues/cc, consumed here).
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

### 9. Pairwise-compatibility rolling update + the mixed-version protocol — PARTIALLY CONFIRMED, one layer PENDING BLESSING (friction-round 1, INTENT #113; wave-3 correction, ledger OQ-31)

INTENT #66 wants minimal-restart rolling updates with pairwise deps and left
**the update protocol between nodes running mixed versions OPEN**. Wave-3
correction against the ledger (§B.3 OQ-31): the operator's #113 **"the
conversation is moot"** confirms a *narrow* claim — **the restart-priority
system IS the mixed-version mechanism** — not the full v1 protocol supervision
proposes on top of it. Two layers, kept explicitly distinct from here on:

- **SETTLED (ledger §A row 32; INTENT #113, operator-confirmed).** The 4-level
  ladder (concern 4) IS how mixed versions are handled: **non-critical updates
  wait for idle** (`WaitForIdle`/routine levels — mixed versions in the interim
  are fine, that's the point of the design); **critical incompatible updates go
  out to every instance at HIGH priority** (the L3 `Compatibility` push, INTENT
  #77). Also SETTLED and LOCKED: **sender version stamping on every
  mesh-crossing message** (name + version, receiver version floors, one-version
  back-compat + please-update warning) — homed in `types`/`mesh-transport`
  (`Hello.min_peer_version`, `WireError{version_below_floor}`, `PleaseUpdate`)
  /`queues-api`, consumed here as the data that makes version floors
  enforceable.
- **PENDING BLESSING (ledger §B.3 OQ-31), NOT settled.** *Which* version the
  fleet organically converges on by default — the specific "organic-newest-
  wins, LWW-by-registration, with an optional operator pin" v1 rollout
  protocol below — is a **designer position**, not an operator-confirmed
  decision. #113's "moot" framing was about the ladder-is-the-mechanism
  question; it did not bless this specific default-selection policy. Marked
  PENDING per the ledger's global design-around rule (§C): kept a
  one-line-swappable placeholder, with the `version/pin/<slug>` operator pin
  as the literal escape hatch if newest-wins is un-blessed in favor of
  always-pinned.

The v1 protocol (candidate, PENDING BLESSING per the above):

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
   present, it marks the service `Degraded` and surfaces it (dashboard + cc) —
   **never silently runs an incompatible pairing.**

6. **Dispositions after friction-round 1 (INTENT #113), corrected wave-3 (ledger
   OQ-31):** (a) binary delivery + per-node build cache (INTENT #33) — still
   OPEN, supervision's hard dependency, out of scope; (b) **organic-newest-wins
   as the default rollout *trigger*** (+ optional operator pin) —
   **PENDING BLESSING (OQ-31)**, NOT confirmed — #113's "the conversation is
   moot" confirmed that *the restart-priority ladder is the mechanism*, which
   is a different, narrower claim than *newest-wins is the right default
   selection policy*; the latter is this file's own designer proposal, kept
   one-line-swappable behind the operator pin; (c) **no coordinated
   drain-all-then-flip breaking roll — CONFIRMED, this part follows directly
   from the SETTLED ladder** (INTENT #66's pairwise-minimal-restart requirement
   + #113's ladder-is-the-mechanism): a critical incompatible update is a
   **high-priority (L3) push to every instance**, which is the ladder doing the
   coordination organically — no new coordination primitive is proposed, so
   there is nothing here beyond what #113 already settled.

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

### 11. The mesh-core coordination seam — `ensure_up` + readiness (wave 3, NEW)

`mesh-core.md` concern 10 (patch-through-on-restart, INTENT #152) names a
dependency on supervision it never designed: when the Dispatcher resolves a
target whose service is down, "it asks `supervision` … to **ensure `slug` is up
on `node`**," then parks the frame "awaiting a **readiness signal**." Both the
ask and the signal are designed here, because park-and-patch cannot be built
without them and neither has a home elsewhere.

**The ask — `ensure_up`, fire-and-forget, no new state machine.** `ensure_up`
is a single additional method on the `trait Supervisor` seam supervision
already exposes UP to mesh-core's Bootstrapper (concern 2):

```rust
trait Supervisor {
    fn boot_plan(&self) -> Vec<BootStage>;                       // concern 2, existing
    fn ensure_up(&self, slug: Slug, node: NodeId);                // NEW, concern 11
}
```

`ensure_up` is deliberately **not** `async fn … -> Ready` and returns nothing —
it is a nudge, not a request/response call. Internally it does nothing novel:
it folds `slug` into the should-be-running set (concern 6) if it is not
already there, and lets the **existing** boot-order (concern 2) and
crash-restart (concern 7) machinery do the actual `ProcessControl.spawn`. If
`slug` is already up, already booting, or already crash-looping, `ensure_up` is
a no-op or a re-affirmation of state already in flight — it is idempotent by
construction because it feeds the same should-be-running reconciliation every
other boot/restart path feeds. There is no `ensure_up`-specific state to leak
or double-fire.

**The readiness signal — design choice: OBSERVE the registry, don't invent a
callback.** Two shapes were on the table:

1. **A callback:** supervision calls back into mesh-core's Dispatcher (or
   completes a future it handed out) when the ensured service is up, so the
   Dispatcher knows to unpark the held frame.
2. **An observation:** the Dispatcher watches `service-registry`'s
   `(slug, node)` record (the same `kv.subscribe` primitive
   `service-registry.md` concern 7 already uses for zombie-suspicion) and
   unparks the frame the moment a fresh **`Live`** record with a bumped
   `generation` appears for that key — i.e. the ordinary registration
   `chassis` performs at its own bring-up (concern 1 there), which happens
   regardless of whether the process was just spawned by `ensure_up`,
   adopted, or was already running.

**Picked: (2), the observation — the boring one.** Reasons, stated plainly:
- **No new channel, no new race.** A callback needs its own delivery guarantee
  (what if supervision's callback fires before the Dispatcher finished parking
  the frame? what if mesh restarts mid-flight and the callback registration is
  lost?) — exactly the kind of state a second coordination path always grows.
  The registry subscription is a mechanism that already exists, is already
  eventually-consistent-safe (LWW + `generation`), and is already the thing
  supervision itself watches for zombie-suspicion — reusing it adds zero new
  failure modes.
- **Idempotent and late-join-safe for free.** A Dispatcher that starts
  watching *after* the target already came back up simply reads the current
  `Live` record immediately (registry reads are LWW state, not an event that
  can be missed) — a callback scheme would need an explicit "did I miss it"
  reconciliation path that the observation gets for free.
- **Keeps ownership clean.** supervision owns *whether/when* to bring a slug
  up (policy); it does **not** need to also own *telling the Dispatcher it's
  ready* — that would duplicate what `service-registry` already broadcasts.
  The Dispatcher, which already resolves `Address`es against the registry
  (mesh-core concern 2), is the natural, already-existing subscriber.
- **Matches the ledger's global rule** (§C): the observation is the
  more-boring, un-invent-a-mechanism option; a bespoke callback would be the
  non-boring alternative this design steers away from.

**Consequence for mesh-core.md concern 10 (recorded here, not edited there —
mesh-core.md is a sibling-owned file):** "readiness" is not a value supervision
returns or pushes; it is the Dispatcher's own read of
`service-registry`'s live record for `(slug, node)`. supervision's contribution
is exactly and only `ensure_up`'s should-be-running nudge; everything after
that — spawning, registering, and the Dispatcher noticing — runs through
machinery each already owns (supervision's boot/crash-restart, `chassis`'s
bring-up, `service-registry`'s LWW record + subscribe, mesh-core's Dispatcher).

**Boundary.** supervision does **not** own: the parked-frame buffer or its
deadline (mesh-core's, concern 10 there); the registry subscribe mechanics or
the `Live`/`generation` semantics (`service-registry`'s, concerns 4/7); the
decision of *what counts as* the target being ready beyond "a fresh live
registration exists" (that IS the definition — supervision does not layer a
health-check or readiness-probe concept on top; a registered-but-unhealthy
service is `service-registry`'s/mesh-core's problem via the normal handoff
health-check path, concern 5, not a second readiness notion here).

## Relationships / edges

supervision's cross-process **contract edge** is `restart-protocol`; everything
else it touches is an **in-process sibling-lib seam** (compiled into `lib/mesh`,
not a contract edge — INTENT #29/#45).

- **every service ↔ mesh** via `restart-protocol` — the two-way 4-level
  graceful-restart / interruptibility / port-handoff choreography. supervision is
  the authoritative **daemon-side** party; the struct home is `types::restart`,
  the client half is `chassis` (wave 3; formerly `mesh-client`), the kill/handoff
  execution is mesh-core. Cross-cutting, surface-schema-style (one shared
  document, every service a party). *(authored: scaffold/contracts/restart-protocol.md)*
- **service-registry** (sibling lib) — supervision **reads** version/requirement/
  lease/endpoint records and **writes** the LWW registry flip during a handoff;
  wave 3 adds **observes** — the `ensure_up`/readiness seam (concern 11) watches
  the same `(slug, node)` record via `kv.subscribe` that zombie-suspicion already
  drains. In-process via `trait Resolver` + a KV keyspace, NOT a contract edge.
  Proposes an *additive facet* of the `service-lookup` registration payload
  (version + `requires`) — flagged to service-registry, authored there.
- **replicated-kv** (sibling lib) — the `ServiceManifest`/version keyspace
  persistence + anti-entropy. In-process `trait KvHandle`; replication is
  `registry-replication`/`kv-replication` (not supervision's contract).
- **mesh-core** (parent shell) — consumes `trait ProcessControl`
  (spawn/signal/adopt/discover) and provides `trait Supervisor` (boot plan,
  restart choreography, and — wave 3, concern 11 — `ensure_up(slug, node)`) UP
  to the Bootstrapper/Dispatcher. In-process trait seams; the readiness half of
  the `ensure_up` ask is answered by mesh-core observing `service-registry`
  directly (concern 11), not by a call back into supervision.
- **cc** via `cc-escalation` (consumed, not authored) — crash-loop /
  incompatibility investigations ride the same escalation shape as DLQ /
  loop-depth (INTENT #70/#89). Authored by queues/cc; supervision is a producer.
- Imports `substrate-types` (`restart`, `node`, `id`, `error` vocabulary) — a
  shared-lib dependency, NOT a contract edge.

## Nesting

Parent: mesh | Children: none. Module `lib/mesh::supervision`, Ring-3 in
mesh-core's internal layering (mesh-core §1): rides `replicated-kv` (Ring 2) +
`service-registry` (Ring 3 peer), consumes mesh-core Ring-0 `ProcessControl`,
and is composed by the Bootstrapper via `trait Subsystem`/`trait Supervisor`.
Never a standalone crate (INTENT #54). The client half of its protocol is
`chassis` (wave 3; the successor shared lib to `mesh-client`).

## Thoroughness level

**implementation-ready** — the `ServiceManifest`/requirement model, boot-order
derivation, the interruptibility state model, the 4-level ladder state machine +
level-selection + escalation rules (now with the severity-in/service-decides
doctrine made explicit, concern 4), the six-step port-handoff choreography, the
zombie-kill decision boundary, the crash-restart backoff + crash-loop policy, the
two-tier supervision-tree split, and the **mesh-core coordination seam**
(`ensure_up` + registry-observation readiness, concern 11, NEW wave 3) are all
decided and specified against frozen seams (mesh-core's `ProcessControl`/
`Supervisor`, `types::restart`, `service-registry`'s LWW registry). **Genuinely
downstream / left open:** (a) binary delivery + build cache (INTENT #33) — an
out-of-scope hard dependency, not a gap in this module; (b) **corrected wave 3**
— concern-9.6's mixed-version *default-selection* policy (organic-newest-wins)
is **PENDING BLESSING (ledger OQ-31)**, not confirmed; only the ladder-is-the-
mechanism claim and the never-coordinate consequence are settled (concern 9);
(c) exact backoff constants + crash-loop window (fill-time tuning knobs, not
design forks); (d) the `restart-protocol` wire is implementation-ready and
reconciled against `chassis`'s authoritative client-half shape (wave 3; drift
found and fixed in `contracts/restart-protocol.md`).

## Assigned design-depth

**Opus**, single Component-Designer pass (this file), grounded in `mesh.md`
(concerns 6, 8, 12, 13), `mesh-core.md` (Ring architecture + `ProcessControl`/
`Supervisor`/`Subsystem` seams, and wave-3 concern 10's patch-through/mediation),
`types.md` (`restart.rs`), `chassis.md` (wave-3 restart-protocol client half,
concern 5), `service-registry.md`, and INTENT
#45/#57/#66/#76/#77/#84/#86/#98/#119/#152/#156/#163, ledger OQ-27/OQ-31/§C.

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

## Proposed contracts (wave 3)

supervision owns one in-process seam addition this wave — an extension of the
existing `trait Supervisor` boundary with mesh-core (not a new wire contract;
`mesh-core.md` is the sibling-owned consumer, so this is recorded here as a
proposal for the harmonizer to fold into `mesh-core.md`'s own text, per concern
11):

```rust
trait Supervisor {
    fn boot_plan(&self) -> Vec<BootStage>;         // existing (concern 2)
    fn ensure_up(&self, slug: Slug, node: NodeId); // NEW (concern 11) — fire-and-forget,
                                                     // idempotent should-be-running nudge
}
```

- **What's proposed:** the single `ensure_up` method, and the design ruling
  that the corresponding **readiness signal is NOT a new callback** — mesh-core's
  Dispatcher observes `service-registry`'s existing `(slug, node)` live-record
  subscription (the same primitive `service-registry.md` concern 7 already
  exposes for zombie-suspicion) rather than supervision inventing a push
  channel back to the Dispatcher. See concern 11 for the full reasoning.
- **What this does NOT change:** no new wire struct, no new contract file, no
  new party. `trait Supervisor` is already an in-process seam (Ring-3 lib to
  mesh-core's Bootstrapper); this is an additive method on it, consistent with
  `types` guardrail patterns for additive, non-breaking extension even though
  it is in-process rather than wire-crossing.
- **Reconciliation flag for the harmonizer:** `mesh-core.md` concern 10
  currently narrates the `ensure_up`/readiness relationship in prose without
  naming a method or a mechanism; this proposal is the concrete fill. No
  drift was found in mesh-core.md's *framing* (it already leans toward
  "observe the registry," never proposing a callback) — this file makes that
  framing a named, typed primitive.

