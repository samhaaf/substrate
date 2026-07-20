# Substrate — Scaffold Overview (wave-2 close-out)

> **THE SYSTEM IS "SUBSTRATE" — RESOLVED, full circle (friction-round 2,
> 2026-07-20, INTENT #120).** "Substrate is a good name." The repo keeps its
> name at v1; system name and repo name are now the same. This supersedes
> both the "Mind OS at v1" lock (2026-07-18) and the friction-round-1 naming
> pin (INTENT #117, Mesh OS / Broomstick OS candidates).
>
> **Naming history, for the record:** the system was first locked as
> **Mind OS** ("an operating system of my mind"), but that name is taken by
> an existing company the operator finds cheesy; **Broomstick OS** was
> pinned as a candidate but is "corny — it's the name of my business";
> **Mesh OS** "doesn't say what it is." **Substrate** — the name the repo
> carried all along — is restored as THE name. (Mycelium and ripple-derived
> names were rejected earlier; "ripples" is a RESERVED term, see the
> placeholder section.) Stale "Mind OS" mentions surviving in component
> files read as "Substrate."

## Wave 2 — COMPLETE (status block)

**Date:** 2026-07-19. **Commit lineage:** `bca1100` (wave-2 decompose /
`wave2-plan.md` authored) → batches 1–8 (per-module design passes, layer by
layer, 5–8 modules per batch) → `0c5d6fb` (per-pair contract round — all
pairs reconciled) → **this harmonization commit** (consistency sweep +
coverage-gap closure + this close-out). Process per INTENT #107 and the
Scaffolding Pattern: decompose → per-module designs proposing neighbor
contracts → per-pair contract round → harmonization → **friction-point
surfacing for the operator's clarification round** (the friction report lives
in the harness workspace, `substrate-v2/reports/wave2-friction-points.md`).

What exists at close-out:

- **47 component files** in `scaffold/components/` (48 as of friction-round
  2 — `onion.md` added as a requirements-only candidate stub, INTENT #122) —
  44 live modules across
  the 7-layer OS stack (L0–L6) plus 3 history/tombstone files (`mesh.md`, the
  round-9 pre-split mesh design record; `gateway.md`, tombstone — gateway
  merged into mesh 2026-07-18; `stack.md`, superseded requirements history —
  the module is `vdb`, "stack" survives as the PATTERN name).
- **75 contract files** in `scaffold/contracts/` — 71 live authored contracts
  + 4 tombstones (`mesh-registry-read` — gateway merge; `registry-replication`
  — superseded by `kv-replication`; `stack-vfs`/`stack-mesh` — renamed
  `vdb-vfs`/`vdb-mesh` at harmonization, operator sign-off pending).
- **Harmonization coverage result:** wave2-plan §3 identified 69 pairs; the
  contract round authored 64 of them and legitimately ADDED two
  (`kg-api`, `vfs-content` — recorded as deviations-by-addition in the files)
  plus the planned `kv-replication` generalization. Three planned pairs had
  no file (`aws-vdb`, `aws-secrets`, `projects-vdb`) and four rounds-era
  stubs were never upgraded (`stack-vfs`, `stack-mesh` — renamed
  `vdb-vfs`/`vdb-mesh`; `kv-cache`; `service-registration`); all were
  authored/renamed at harmonization from the already-reconciled component
  halves. Gaps deliberately NOT closed (surfaced in the friction report
  instead of invented here): **projects↔repo** (worktree binding — no edge in
  the inventory), **vfs-secrets**, **engine-vfs**, and **mesh-transport**
  (the byte-level framing layer ~12 contracts bind to by reference — carried
  as mesh-core's to pin at fill).
- Each component file now carries a compact **"Contracts (wave 2 —
  authored)"** section pointing at its contract files; the contract files are
  authoritative (including their Reconciliation notes). Detail lives there,
  not here.

**Governing precedence:** INTENT items (#1–#127, harness workspace) win over
scaffold text; contract files win over component-file proposals;
`wave2-plan.md` remains the module-inventory record of the decompose step.

## Friction-round 1 — operator answers folded in (2026-07-19, INTENT #108–#118)

The first clarification round after wave-2 close-out. Dispositions (detail and
verbatim rationale live in the named files):

- **Queue-ownership fork — LOCKED** (INTENT #112): replicated-everywhere +
  event-ID semaphore; **all nodes process; whoever discovers an event may
  claim ownership via the semaphore** (nanosecond UTC timestamp, whoever gets
  it). Single-owner-node rejected. Rationale: reliability over speed
  ("keeping things running all the time and having our leverage create more
  leverage"); partition insight: if an event reached both nodes, those nodes
  were connected. → `components/queues.md` concern 4,
  `contracts/queues-api.md` (its provisional assumption is now the locked
  model).
- **Kernel-KV-outside-VFS — BLESSED** (INTENT #108): mesh's own SQLite/KV
  state sits below the storage plane, on the plain OS filesystem — "Makes
  sense because everything depends on it. It's the one exception." →
  `components/replicated-kv.md` concern 9.
- **Mixed-version updates — CONFIRMED, no longer open** (INTENT #113): "the
  conversation is moot" — the restart-priority ladder IS the answer.
  Non-critical updates wait for idle (mixed versions fine, that's the point);
  critical incompatible updates go out to every instance at HIGH priority.
  Organic-newest-wins stands. **NEW LOCKED cross-cutting requirement on top:
  sender version stamping** — every mesh-crossing message/event carries the
  sending service's name AND version; receivers may enforce a version floor
  (catchable rejection); schema changes offer one version of back-compat plus
  a please-update warning attached back to the sender. →
  `components/supervision.md` concern 9, `components/types.md` guardrail 4,
  `contracts/pubsub-protocol.md`, `contracts/queues-api.md`.
- **S3 bulk flow — CONFIRMED** (INTENT #114): direct-upload with mesh-issued
  presigned permission (the Presigned default); the mesh as router locates
  the data-holding node and tells it to send to the presigned URL — via queue
  OR a direct service-router path. → `components/aws.md` concern 3,
  `contracts/aws-vfs.md`.
- **Clock discipline — HARDENED, new design item** (INTENT #116): the HLC
  ratchet stays; ADDED a **mesh time-authority requirement** — mesh owns
  time: enforced UTC sync, possibly mesh-daemon-issued timestamps with
  neighbor-ping offset correction, race-condition management "as hardened as
  is physically possible." Approach-sketched; a design item for mesh-core's
  fill. → `components/mesh-core.md` concern 9,
  `components/replicated-kv.md` concern 1.
- **`db serve` daemon — FROZEN** (INTENT #115): operator flagged possible
  drift ("db = a standalone crate you call as a tool; VDB = the mesh-accessed
  service"); **do not build; discussion pending**. Design annotated, not
  deleted. Mesh may use db via direct CLI execution for its own database (no
  daemon, no mesh dependency). → `components/db.md` concern 1,
  `contracts/vdb-db.md`, `contracts/db-control-plane.md`.
- **Naming — PINNED, undecided** (INTENT #117): SUPERSEDED at friction-round
  2 (INTENT #120) — the name is **Substrate**; see the header note.
- **App-dev framework — noted, future scope** (INTENT #118): see the
  placeholder section.
- Still genuinely open after this round: FIFO queues (no operator ask),
  binary delivery / per-node build cache (INTENT #33), the cloud node
  (INTENT #110 — topics for discussion), disaster-recovery design (INTENT
  #109 — direction rich, design pending), and the remaining wave-2 friction
  items awaiting later clarification rounds (per INTENT #111, the loop
  continues, curated by the friction report).

## Friction-round 2 — operator answers folded in (2026-07-20, INTENT #119–#127)

The second clarification round. Dispositions (detail and verbatim rationale
live in the named files):

- **Naming — RESOLVED: Substrate** (INTENT #120): see the header note. Full
  circle; the Mesh OS / Broomstick OS pin and the "Mind OS at v1" lock are
  both superseded.
- **Restart-signal semantics — deferred to a PHILOSOPHY** (INTENT #119): the
  per-app idle/critical question (including benchmark-as-Idle) is not ruled
  on here; a restart-interrupt-signal philosophy will be developed via the
  critic pattern (Opus proposes, Fable critiques, Opus synthesizes) in a
  dedicated harness plugin for working on substrate. Benchmark-as-Idle
  stands **applied provisionally, pending that philosophy**. →
  `components/supervision.md` concern 3, `components/inference.md` concern 4.
- **Org's graph home — RESOLVED: KG** (INTENT #121): "Org's
  self-restructuring knowledge graph definitely belongs in the KG service";
  `db` keeps only flat relational metadata. KG's charter explicitly gains:
  services can add new node types, schemas, version control — and "assume
  that every service will be using the knowledge graph." →
  `components/kg.md` charter, `components/org.md` design note + OQ 1.
- **NEW candidate crate: `onion` — schema-delta layering** (INTENT #122): a
  delta language over schema templates, flatten-on-template-update,
  residual deltas; applicable beyond KG (YAML, JSON schema). Captured as a
  requirements-only stub; **a dedicated KG-templating discussion round is
  REQUIRED before any design.** → `components/onion.md`,
  `components/kg.md` concern 3 note, and the tree below (candidate,
  placement undecided).
- **Coordinators — elevated to a first-order concept** (INTENT #123):
  dedicated discussion required; recorded, not designed. →
  `components/projects.md` design note + open questions, and the open
  questions below.
- **Process law for the next scaffolding run** (INTENT #124): boring
  decisions only; see standing principle 11.
- **Handlers no-net-by-default — BLESSED** (INTENT #125): "Fascinating
  idea. I like it. I'm OK with that" — including the weaker
  declared-not-enforced cloud caveat. → `components/execution-engine.md`
  concern 7 + Friction #3/#5.
- **Third-party tools + browser-as-a-tool** (INTENT #126): a future `tools`
  concept; browser explicitly a third-party tool, NOT a first-order crate;
  KG around tool uses and feedback; the don't-reinvent principle joins the
  standing principles (principle 12). → placeholder section.
- **THE ENDGAME: topological owners** (INTENT #127): recorded
  verbatim-grade, deliberately NOT designed. → `components/org.md` and the
  placeholder section.
- Newly open (recorded, awaiting their own rounds): the restart-signal
  philosophy (INTENT #119, critic-pattern plugin); the KG-templating /
  `onion` discussion round (INTENT #122); the coordinators discussion
  (INTENT #123) — coordinator-per-workspace, workspaces possibly pulled out
  on top of VDB, coordinator-as-a-service, coordinator compaction ("compact
  the thread and basically lose nothing"), and the
  git+projects+environments+coordinators elegance problem ("a bunch of
  disparate things" that must be made elegant together); and the owners
  open questions (INTENT #127, listed in the placeholder section).

## The OS layering — final component tree

44 live modules, 7 layers. A module may only depend on its own layer's peers
and lower layers; every cross-app call goes through the local mesh daemon
(single-port locality, `:3649`); in-process linking across app boundaries is
forbidden (INTENT #29), shared libs excepted (compiled in — `types`,
`tailscale-query`, `mesh-client`, `execution-engine`). Kinds: app-crate ·
internal-lib(parent) · shared-lib · app-frontend · stub.

```
L0 foundation      types (shared-lib)             tailscale-query (shared-lib)
L1 mesh kernel     mesh-core (bin/mesh, :3649)    pubsub-relay (lib)
                   network-topology (lib)          mesh-client (shared-lib)
L2 state + OS svcs replicated-kv    service-registry (THE wiring seam)
                   locks            queues          cron
                   supervision      completion-router
                   dashboard-serving                aws (app-crate)
L3 storage plane   vfs (app)        gc (app, dual-role)   secrets (app)
L4 data+execution  db (app)         vdb (app; "stack" = the pattern name)
                   execution-engine (shared-lib)   kg (app)   rollup (app)
L5 services/apps   inference (app) ── store engine scheduler models cache
                                      telemetry benchmark api (8 internal libs)
                   repo (app)       ccd (app)      dashboard (ui/dashboard)
L6 org plane       org  projects  artifacts  environments  cicd
   (STUB track)    spend  agents  aui  openrouter-mgmt
                   (design notes + anticipated contracts; "not implementing now")

candidate          onion (schema-delta layering — INTENT #122; existence and
   (undecided)     placement both undecided; requirements-only stub; a
                   dedicated KG-templating discussion round required first)
```

Notable placements (full reasoning in `wave2-plan.md` §1): `aws` sits at L2
because the replication plane, VFS overflow, VDB cloud target, and secrets
push all consume it; `gc` sits at L3 beside `vfs` (dual-role: embedded lib in
inference + per-node `:8430` daemon VFS calls); inference's per-node loopback
API convention is **`:8420`** (real per-node endpoints resolve via the
registry; `:3649` is the only mesh port).

## Final contract graph — the 71 live contracts

Grouped by cluster; every name is a file in `scaffold/contracts/`. Direction
and payload detail live in the files.

**Cross-cutting protocols (one shared document, every service a party —
surface-schema precedent):**
`service-lookup` (register/resolve — THE wiring seam) · `pubsub-protocol`
(the standard WS pub/sub envelope) · `restart-protocol` (two-way 4-level
graceful-restart ladder + interruptibility + port-handoff) · `queues-api`
(typed events → declarative triggers → handlers) · `locks-api` (distributed
semaphores + the partition-merge error) · `cron-api` (on-node-N / anywhere
schedules) · `surface-schema` (the boring render+interaction schema every
service publishes).

**Mesh kernel + replication plane:**
`kv-replication` (the ONE anti-entropy protocol for all kernel state;
supersedes `registry-replication`) · `tailscale-status` (tailscale-query →
network-topology, SOLE consumer — completion-router dropped at the contract
round) · `network-events` (peer on/off + self-connectivity feed) ·
`aws-mesh` (the replication plane's S3/AWS leg) · `kernel-confidence`
(scheduler → telemetry kernel reads).

**Completion data plane:**
`v1-completion-api` (the `/v1` REST+WS surface, forwarded transparently) ·
`node-state-poll` (router's reconcile reads) · `inference-events` (per-node
pub/sub lifecycle stream) · `llm-calls` (ccd → inference, metering-shaped).

**Observability plane:**
`dashboard-feed` (browser fan-out + REST + static hosting) · `gc-events` ·
`ccd-events` · `system-state` (SystemState incl. the wave-2
`effective_max_concurrent`).

**Storage plane:**
`vfs-mesh` · `vfs-content` (bulk content bytes, pull-driven — added by the
contract round) · `vfs-gc` · `aws-vfs` (S3 overflow tier) ·
`gc-managed-dirs` (embedded-gc in-process surface) · `secrets-mesh` ·
`db-secrets` · `repo-secrets` (secret↔workflow injection-on-push — THE v1
capability) · `rollup-secrets` · `vdb-secrets` · `aws-secrets` (SM push
design-only + creds + S3 CSE keys + genesis rule) · `openrouter-secrets`
(stub-track) · `secrets-environments` (stub-track).

**Data & execution plane:**
`db-control-plane` · `db-inference-init` · `vdb-db` (the `db serve` session
protocol — db's second public surface; **FROZEN pending the INTENT #115
db-serve drift discussion**) · `vdb-vfs` (SQLite-file-in-VFS;
renamed from `stack-vfs`) · `vdb-mesh` (registration/catalog/locks; renamed
from `stack-mesh`) · `aws-vdb` (RDS+Lambda target, design-only v1) ·
`kg-vdb` (KG built ON VDB) · `kg-vfs` (node→file pointers) · `kg-mesh` ·
`kg-api` (the KG consumer surface — added by the contract round) ·
`rollup-mesh` (trigger payload-assembly) · `rollup-vfs` · `rollup-ccd` ·
`ccd-escalation` (DLQ + loop-depth-exceeded investigations, one shape).

**Inference-internal (the node's crate contracts):**
`store-access` · `engine-exec` · `model-ensure` · `kv-cache` ·
`benchmark-collections` · `api-dispatch`.

**Service/application plane:**
`service-registration` (ccd as registrant/resolver) · `agent-management`
(spawn/track/signal/stream/reap) · `org-on-ccd` (+ the maximalist-consumer
read bundle) · `repo-vfs` · `repo-environments` (v1-facing shape pinned
despite environments being stub-track) · `cicd-repo`.

**L6 stub-track (anticipated contracts, content deferred):**
`projects-vfs` · `projects-mesh` · `projects-kg` · `projects-rollup` ·
`projects-vdb` · `projects-artifacts` · `ccd-projects` · `spend-ccd` ·
`agents-ccd` · `aui-mesh` · `openrouter-spend` · `environments-vdb`.

**Tombstones (4):** `mesh-registry-read`, `registry-replication`,
`stack-vfs`, `stack-mesh` — each file explains its supersession.

**Deliberately NOT contract edges** (locked rounds 4–5): all shared-lib
consumption (`types`, `tailscale-query`, `mesh-client`, `execution-engine`) —
internal library dependencies; `tailscale-status` is the one shared-lib API
documented as a contract anyway (its public trait IS its contract — nature
flag recorded in the file).

## The wiring seam — service-registry + supervision

**The runtime composition point is the mesh `service-registry`
(`service-lookup`), executed by `supervision`.** Every service, at boot,
registers its `slug -> {scheme, host, port, health_path}` against its LOCAL
mesh daemon on `:3649` and resolves every dependency by slug — never a static
URL. Records ride `replicated-kv` (leases + heartbeats + LWW tombstones), so
"anywhere you access mesh is exactly the same."

- **Addressing classes:** `Singleton` (db, ccd, kg, secrets…) · `NodeScoped`
  (gc `:8430`, vfs legs, per-node `inference` instances) · `FleetAlias` — a
  **resolve-time policy** on the `inference` slug (→ local `:3649` →
  completion-router), NOT a stored record. Per-node `inference` registration
  is legal and IS the router's fleet membership (`resolve_all("inference")`)
  — the concern-5 dispute resolved at the contract round
  (`service-lookup.md` Reconciliation notes).
- **Supervision executes the seam:** dependency-derived boot order (registry
  stores `requires`; supervision topo-sorts and mesh STARTS local services),
  the 4-level restart ladder (`restart-protocol`), port-handoff updates
  (new port → health check → registry flip → drain old), minimal-restart
  rolling updates. The mixed-version update protocol is **CONFIRMED**
  (friction-round 1, INTENT #113): the restart-priority ladder IS the answer
  — non-critical waits for idle; critical incompatible updates push to every
  instance at high priority — plus the LOCKED sender-version-stamping
  requirement (friction-round section above).
- The assembler (skeleton phase) touches exactly this seam; each crate's
  private composition root (`InferenceService::start`, mesh's ring layering)
  is NOT the seam.

## Standing cross-cutting principles

1. **INTENT wins.** Where scaffold text conflicts with the INTENT ledger
   (#1–#127), INTENT governs.
2. **Provenance is first-order** — healthcare-grade traces on every handler
   touch; scoped per-project/per-database; VDB is its primary home; VFS
   carries a lighter requirement.
3. **Pairwise data contracts are the operating model** — any contract change
   triggers a design session analyzing ripple effects.
4. **Single-port locality:** every service talks only to its local mesh
   daemon on `:3649`; mesh does all relaying. Per-node inference loopback
   convention: `:8420`. In-process linking across app boundaries forbidden
   (INTENT #29).
5. **Boring layers on boring layers** — mesh utilities are internal libs,
   never standalone services; replicated state is LWW (naive by blessing);
   the HLC ratchet stands and is now exceeded by the mesh time-authority
   requirement — mesh owns time (friction-round 1, INTENT #116; approach-
   sketched in `mesh-core.md` concern 9).
6. **Triggers are declarative** — filter expression + payload-assembly
   template, registered as DATA; code lives only in handlers. One trigger
   data model shared by queues and the execution-engine's two adapters.
7. **No secret ever reaches an LLM** — use-without-seeing, `llm_safe`
   fail-or-degrade, structurally enforced; rollup never resolves a raw
   secret reference in LLM-bound content.
8. **NO DOCKER locally, ever; SQLite-locally LOCKED** — Postgres exists only
   as a cloud VDB target; containers, if ever, live only in AWS via `aws`.
9. **Wire-struct version discipline** (types guardrail 4): additive-only,
   `#[serde(default)]`, `#[serde(other)]` enum tolerance, explicit `v`
   fields on persisted/replicated shapes — the qualifier on INTENT #45's
   "no runtime version tracking for shared libs". PLUS the LOCKED
   sender-version-stamping requirement (friction-round 1, INTENT #113):
   every mesh-crossing message carries sender service name + version;
   receiver version floors are catchable; one version of back-compat with a
   please-update warning to the sender.
10. **Naming guardrails:** the module is `vdb`, "stack" is the pattern;
    "ripples" is RESERVED and names nothing in v1; `inference` will not be
    renamed; the system is **Substrate** (INTENT #120).
11. **Process law — boring decisions only (friction-round 2, INTENT #124):**
    in the next scaffolding run, agents capture all open questions and make
    BORING decisions only. "I want them to make boring decisions, and if a
    decision is not boring, then we need to talk about it and figure out
    which path is the most boring" — non-boring decisions escalate to the
    operator, never get assumed.
12. **Don't reinvent — leverage third-party tools (friction-round 2, INTENT
    #126):** "we're building what's strictly necessary for our mind temple,
    plus ways to enable agents to be leveraged further — take advantage of
    third-party tools, we don't want to reinvent everybody else's work."
    Concretely: the browser is a third-party tool under the future `tools`
    concept, NOT a first-order crate (see the placeholder section).

## L6 / placeholder section

The nine L6 modules are **design stubs with anticipated contracts, explicitly
"not implementing now"** (INTENT #49/#51): `org` (autonomous organizations —
owning agents + the negotiation protocol; minimum imports inference+ccd+db),
`projects` (graphical FS over VFS via a KG graph; the confirmed .mind
workspace-schema migration — heavy design notes retained), `artifacts` ("no
more files — artifacts"), `environments` (stub-track yet load-bearing: it
pins the v1-facing `repo-environments` shape), `cicd` (own-crate vs
emergent-from-repo OPEN), `spend` (pull-shaped cost queries), `agents` (the
generalization ON TOP of ccd — never absorbs it), `aui` (voice interface over
the mesh), `openrouter-mgmt` (per-key budgets). **`ripples` — RESERVED
TERM**: a future KG micro-agent system; not designed, not discussed, and the
word must not name the execution-engine or any propagation mechanism.
**App-development framework (INTENT #118, future scope, noted only):** "We
need a whole system built around designing applications built on top of our
mesh OS" — an application-development framework for the OS; not designed, not
scheduled.

**`tools` — third-party agent-tool integration (INTENT #126, future scope,
noted only):** agents need tools, and GitHub hosts whole repos of agent
tools (e.g. agent browsers). The **browser is explicitly a third-party tool
under this general `tools` concept, NOT a first-order crate** — so versions
can be swapped without re-engineering. Wanted alongside it: a KG around tool
uses and feedback on tools. Governed by standing principle 12
(don't-reinvent). Not designed, not scheduled.

**Coordinators — a first-order concept, dedicated discussion required
(INTENT #123, recorded not designed):** the operator's verbatim-grade
sketch: coordinators attach to workspaces; workspaces often attach to
worktrees; workspaces could be pulled out as their own thing built on top of
VDB; **one coordinator per workspace**. "I really only like to talk to
coordinators — I don't like talking to individual agents at all." AUI
threads = a coordinator with a workspace attached. Coordinators live inside
a project on a branch; the workspace merges into other branches with it.
**Coordinator compaction:** "I should be able to compact the thread and
basically lose nothing." Coordinator-as-a-SERVICE is worth considering. The
git+projects+environments+coordinators interplay is currently "a bunch of
disparate things" that must be made elegant together. → `projects.md`.

**THE ENDGAME — topological OWNERS (INTENT #127, recorded verbatim-grade,
deliberately NOT designed):** "That is it, dude. That is what we're going
for." An **owner** is a coordinator-like daemon with NO human in the loop,
idle unless woken, owning a thing (an app, a project, a component). Flow: a
user's coordinator sends feedback to an owner → the owner examines its
thing, responds with a proposal + new data contract → the requester
(human-in-the-loop via their coordinator) approves → the owner dispatches
subagents as a coordinator would → reports completion + new contract +
version info. "All of our boring services will be very easy to update."
Name: **owner** (over manager/lead) — "it owns a thing; once we build a
thing, we put an owner in charge of it, and that's how we improve the thing
in the future." Owners arrange **hierarchically over the topological map**
of projects/sub-projects/components — THE primitive for building autonomous
organizations. The operator's own open questions: org or projects? KG
relationship — how much graph feeds an owner on wake? Is an owner exactly a
coordinator without a human (leaning YES — owners wake to a perfectly
organized workspace)? How to keep the topological map linearly separable —
add coordinators, establish relationships, sometimes merge them? →
`org.md`.

## History

The round-by-round lock history (rounds 3–9: gateway merge, mesh internals,
VDB/db/KG layering, SQLite-locally, secrets rename, declarative triggers,
the aws crate, wave-2 authorization) that previously filled this file lives
in git history (`git log -- scaffold/overview.md`, through commit `0c5d6fb`)
and in the INTENT ledger; the surviving decisions are all encoded in the
component/contract files above. `wave2-plan.md` §5 records the flags carried
into the batches and their dispositions.
