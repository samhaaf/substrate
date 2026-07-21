# Substrate — Scaffold Overview (wave-2 close-out)

> **NAMING REOPENED — "SUBSTRATE" IS THE WORKING NAME; THE FINAL NAME IS
> OPEN (friction-round 3, 2026-07-20, INTENT #129/#141).** This supersedes
> the round-2 "resolved Substrate, full circle" note (INTENT #120). Round 3:
> "Mind temple — that's what this is. It's a mind temple. Maybe we call it
> mind temple, or just temple. Mind Temple OS. Substrate OS. Those are the
> candidates now." Round 4 (same day) crystallized the criteria while
> keeping it open: Mind Temple is "a tiny bit cliche... I don't want to feel
> embarrassed — I want to confuse people by using a word they've never heard
> before"; Substrate remains "a really good starting point because the
> philosophy is true — build up from reusable boring primitives. But a
> temple you build a strong foundation in is a better abstraction."
> **Candidates now: Substrate (OS) vs (Mind) Temple / the temple idea "in a
> different language."** Criteria: simple, transcription-friendly,
> non-cliche, resonant, possibly unfamiliar. Until decided, every
> "Substrate" in this tree reads as the WORKING name.
>
> **Naming history, for the record:** the system was first locked as
> **Mind OS** ("an operating system of my mind"), but that name is taken by
> an existing company the operator finds cheesy; **Broomstick OS** was
> pinned as a candidate but is "corny — it's the name of my business";
> **Mesh OS** "doesn't say what it is." **Substrate** — the name the repo
> carried all along — was restored as THE name at friction-round 2 (INTENT
> #120), then reopened at round 3 as above. (Mycelium and ripple-derived
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
  2 — `onion.md` added as a requirements-only candidate stub, INTENT #122;
  renamed `schema.md` at friction-round 3, INTENT #131) —
  **43 live modules** (44 until friction-round 3 dissolved the `org` crate,
  INTENT #132) across
  the 7-layer OS stack (L0–L6) plus **4 history/tombstone files** (`mesh.md`, the
  round-9 pre-split mesh design record; `gateway.md`, tombstone — gateway
  merged into mesh 2026-07-18; `stack.md`, superseded requirements history —
  the module is `vdb`, "stack" survives as the PATTERN name; `org.md`,
  tombstone-with-content — the org crate dissolved into emergent
  coordinators at friction-round 3, INTENT #132). The former `ccd.md` is
  `cc.md` (rename, INTENT #131).
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

**Governing precedence:** INTENT items (#1–#141, harness workspace) win over
scaffold text; contract files win over component-file proposals;
`wave2-plan.md` remains the module-inventory record of the decompose step
(it retains the pre-rename `ccd`/`onion`/`org` vocabulary as history).

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
  **SUPERSEDED at friction-round 3 (INTENT #128): UNFROZEN and ACCEPTED** —
  see the friction-round 3 section below.
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

- **Naming — RESOLVED: Substrate** (INTENT #120): full
  circle; the Mesh OS / Broomstick OS pin and the "Mind OS at v1" lock are
  both superseded. **REOPENED at friction-round 3 (INTENT #129/#141)** —
  Substrate is now the WORKING name; see the header note.
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
  `components/kg.md` charter, `components/org.md` (since friction-round 3 a
  tombstone — INTENT #121 carries forward in its "What org IS" section).
- **NEW candidate crate: `onion` — schema-delta layering** (INTENT #122): a
  delta language over schema templates, flatten-on-template-update,
  residual deltas; applicable beyond KG (YAML, JSON schema). Captured as a
  requirements-only stub; **a dedicated KG-templating discussion round is
  REQUIRED before any design.** **RENAMED `schema` + upgraded to nested
  inheritance at friction-round 3 (INTENT #131)** — see below. →
  `components/schema.md`, `components/kg.md` concern 3 note, and the tree
  below (candidate, placement undecided).
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

## Friction-round 3 — operator answers folded in (2026-07-20, INTENT #128–#141)

The third clarification round. Dispositions (detail and verbatim rationale
live in the named files):

- **`db serve` — UNFROZEN, ACCEPTED** (INTENT #128, resolving #115): "it
  actually does make sense to have a session concept with a headless
  stateful daemon running as part of db." Division clarified: **VDB = the
  virtualization layer** (same access to databases regardless of
  environment/underlying technology), **delegating to db** so the same
  tools aren't defined twice; **VDB owns statement tracking, cleanup, and
  session management.** All round-1 FREEZE annotations removed. →
  `components/db.md` concern 1, `components/vdb.md` concern 7,
  `contracts/vdb-db.md`, `contracts/db-control-plane.md`,
  `contracts/db-inference-init.md`.
- **RENAME `ccd` → `cc`** (INTENT #131): "They're all daemons — we don't
  need this one to be special. It's just cc, not the ccd." File renames:
  `components/ccd.md` → `cc.md`; contracts `rollup-ccd` → `rollup-cc`,
  `ccd-events` → `cc-events`, `ccd-escalation` → `cc-escalation`,
  `ccd-projects` → `cc-projects`, `org-on-ccd` → `org-on-cc`, `spend-ccd`
  → `spend-cc`, `agents-ccd` → `agents-cc`. Naming-history notes kept in
  each file; `wave2-plan.md` keeps the old names as history.
- **RENAME `onion` → `schema`, upgraded** (INTENT #131): "we'll just call
  it schema. Our crate for managing schemas with versions and layering.
  One boring tool for schema management, used within all the other
  services." Model upgraded from single-layer template+delta to **NESTED
  inheritance to arbitrary depth** ("this schema inherits from that schema
  and applies these migrations, to arbitrary depth"). Accepted as a
  concept; placement still undecided; the KG-templating discussion round
  is still REQUIRED before design. → `components/schema.md`,
  `components/kg.md` concern 3 note.
- **`org` crate DISSOLVED — orgs are emergent** (INTENT #132): "We don't
  need an org crate. Org is emergent — it just is a bunch of coordinators.
  Maybe in the future an org node with a specific owner attached, like the
  chief in the agentic-business idea." Org ≈ a knowledge graph linking
  coordinators with typed edges (reports-to, delegates-to,
  may-create-sub-coordinators), living ON kg (INTENT #121 unchanged).
  `org.md` is now a tombstone-with-content; the L6 tree drops `org`;
  `org-on-cc` survives as a shaped-for record only. → `components/org.md`,
  `contracts/org-on-cc.md`.
- **Coordinator = owner; the concept becomes a MINI-HARNESS** (INTENT
  #130/#133, carrying #127's owners endgame): "They are the same thing."
  Multi-threaded (one human thread + persistent per-message threads), an
  HTTP-like message protocol to a workspace coordinator, broadcast to
  sibling threads in the same space, messaging + workspace construct +
  compaction built in; workspaces stay AS-IS; per-node-TYPE directory
  structures tracked within the `schema` service; the archetype wants a
  grounded name (deva/angel-class, "grounded, not mystical"). Recorded
  verbatim-grade as a **DISCUSSION-REQUIRED first-order concept, not
  designed**. → `components/org.md` (the concept's home), the placeholder
  section below.
- **Layer-6 purpose statement** (INTENT #134): "We need a bunch of boring
  services, but one of those boring services enables the future of mind
  work. That's what we're aiming towards at the layer-6 level." → the L6
  section below.
- **KG distribution: distributed-everywhere may be a HARD constraint**
  (INTENT #135): "This is a distributed graph — it needs to be accessible
  from every mesh node... keeping it distributed might have to be a hard
  constraint. But maybe I'm misunderstanding the question — come back to
  this." The wave-2 home-node + offline refuse/branch model **NEEDS
  REVISITING** against it — flagged as a REQUIRED KG discussion item
  (alongside the templating round), deliberately NOT redesigned now. →
  `components/kg.md` concern 4 flag.
- **Artifacts reframed: objects with attributes AND METHODS** (INTENT
  #136): schematized graph nodes whose distinguishing value is **agent
  interaction** — tools attached to items, visible only when the artifact
  comes into scope; used by the `agents` service. The standing question
  recorded: "is there anything useful in artifacts we can't get from the
  knowledge graph directly?" → `components/artifacts.md` reframe section.
- **Custom agents do NOT route through cc** (INTENT #137): "Custom agents
  might call OpenRouter, might call the inference tool — they're not
  necessarily going to call cc." cc is for the **full Claude Code agent
  shape**; the brokered-through-cc lean is superseded. →
  `components/agents.md`, `contracts/agents-cc.md`, `contracts/llm-calls.md`,
  `components/cc.md` routing addendum.
- **Error-taxonomy migration: NOW** (INTENT #138): "We're going to do
  everything before we even test it. There's no production use of it yet...
  we're going to build it correctly." Existing flat leaves migrate in ONE
  sweep at skeleton time; no bridge period. → `components/types.md`.
- **BIG unification idea: mesh messaging on schema inheritance** (INTENT
  #139): "the ENTIRE messaging protocol in the mesh could be done using
  schemas and schema inheritance." Recorded as a discussion/design item for
  the next pass — NOT applied to the authored contracts. →
  `components/schema.md`.
- **Rollup moves toward KG** (INTENT #140): plugin-rollup "more like a
  graph... built on top of schemas and KG"; prompt fragments possibly stay
  file-based; "skip directly to rollup being built on top of KG" recorded
  as direction, design deferred. → `components/rollup.md` concern 8 note.
- **Naming reopened** (INTENT #129/#141): Substrate (OS) vs (Mind) Temple /
  temple-in-another-language; criteria: simple, transcription-friendly,
  non-cliche, resonant, possibly unfamiliar. **Substrate = working name;
  final name open.** → the header note.
- Newly open / still open after this round: the coordinators discussion
  round (INTENT #123, now enriched by #130/#132/#133 — the mini-harness,
  the archetype name, per-node-type directory structures); the
  KG-templating / `schema` design round (INTENT #122/#131) plus the KG
  distribution-constraint revisit (INTENT #135); the INTENT #139
  schema-unification round; the rollup-on-KG design pass (INTENT #140);
  and the final name (INTENT #141).

## The OS layering — final component tree

43 live modules, 7 layers (44 until friction-round 3 dissolved `org`,
INTENT #132). A module may only depend on its own layer's peers
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
                   repo (app)       cc (app)      dashboard (ui/dashboard)
L6 org plane       projects  artifacts  environments  cicd
   (STUB track)    spend  agents  aui  openrouter-mgmt
                   (design notes + anticipated contracts; "not implementing now")
                   [org — DISSOLVED, friction-round 3, INTENT #132: emergent,
                    "just a bunch of coordinators"; org.md is a tombstone-with-
                    content holding the coordinator/owner concept]

candidate          schema (schema management: versions + layering, nested
   (placement      inheritance to arbitrary depth — INTENT #122/#131; renamed
    undecided)     from `onion` at friction-round 3; accepted as a concept,
                   placement undecided; requirements-only stub; the dedicated
                   KG-templating discussion round is still required first)
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
pub/sub lifecycle stream) · `llm-calls` (cc → inference, metering-shaped).

**Observability plane:**
`dashboard-feed` (browser fan-out + REST + static hosting) · `gc-events` ·
`cc-events` · `system-state` (SystemState incl. the wave-2
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
protocol — db's second public surface; **ACCEPTED — friction-round 3, INTENT
#128 resolved the round-1 freeze**) · `vdb-vfs` (SQLite-file-in-VFS;
renamed from `stack-vfs`) · `vdb-mesh` (registration/catalog/locks; renamed
from `stack-mesh`) · `aws-vdb` (RDS+Lambda target, design-only v1) ·
`kg-vdb` (KG built ON VDB) · `kg-vfs` (node→file pointers) · `kg-mesh` ·
`kg-api` (the KG consumer surface — added by the contract round) ·
`rollup-mesh` (trigger payload-assembly) · `rollup-vfs` · `rollup-cc` ·
`cc-escalation` (DLQ + loop-depth-exceeded investigations, one shape).

**Inference-internal (the node's crate contracts):**
`store-access` · `engine-exec` · `model-ensure` · `kv-cache` ·
`benchmark-collections` · `api-dispatch`.

**Service/application plane:**
`service-registration` (cc as registrant/resolver) · `agent-management`
(spawn/track/signal/stream/reap) · `org-on-cc` (the maximalist-consumer
read bundle — party dissolved at friction-round 3, INTENT #132; survives
as the shaped-for record for the coordinator/owner concept) · `repo-vfs` ·
`repo-environments` (v1-facing shape pinned
despite environments being stub-track) · `cicd-repo`.

**L6 stub-track (anticipated contracts, content deferred):**
`projects-vfs` · `projects-mesh` · `projects-kg` · `projects-rollup` ·
`projects-vdb` · `projects-artifacts` · `cc-projects` · `spend-cc` ·
`agents-cc` (Claude-Code shape only — INTENT #137) · `aui-mesh` ·
`openrouter-spend` · `environments-vdb`. (All seven former `*-ccd` names
were renamed `*-cc` at friction-round 3, INTENT #131.)

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

- **Addressing classes:** `Singleton` (db, cc, kg, secrets…) · `NodeScoped`
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
   (#1–#141), INTENT governs.
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
    renamed; `ccd` is now `cc` and `onion` is now `schema` (INTENT #131);
    the system's WORKING name is **Substrate** — the final name is OPEN
    (INTENT #129/#141, superseding #120's "resolved").
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

**The L6 purpose statement (friction-round 3, INTENT #134):** "We need a
bunch of boring services, but one of those boring services enables the
future of mind work. That's what we're aiming towards at the layer-6 level."

The eight L6 modules (nine until `org` dissolved — friction-round 3, INTENT
#132) are **design stubs with anticipated contracts, explicitly
"not implementing now"** (INTENT #49/#51):
`projects` (graphical FS over VFS via a KG graph; the confirmed .mind
workspace-schema migration — heavy design notes retained), `artifacts` ("no
more files — artifacts"; reframed at friction-round 3 as objects with
attributes AND METHODS that agents interact with, tools visible in scope —
INTENT #136), `environments` (stub-track yet load-bearing: it
pins the v1-facing `repo-environments` shape), `cicd` (own-crate vs
emergent-from-repo OPEN), `spend` (pull-shaped cost queries), `agents` (the
generalization ON TOP of cc for the Claude-Code shape — never absorbs it;
custom agents call OpenRouter/inference DIRECTLY, INTENT #137), `aui`
(voice interface over the mesh), `openrouter-mgmt` (per-key budgets).
**`org` is NOT a module (INTENT #132):** "org is emergent — it just is a
bunch of coordinators," probably a KG linking coordinators with typed edges
(reports-to, delegates-to, may-create-sub-coordinators); possibly a future
org NODE with a chief owner attached; `org.md` is its tombstone-with-content
and the home of the coordinator/owner concept. **`ripples` — RESERVED
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

**Coordinators/OWNERS — ONE first-order concept, dedicated discussion
required (INTENT #123, sharpened by #127/#130/#132/#133; recorded not
designed):** friction-round 3 resolved that **coordinator and owner ARE THE
SAME THING** (INTENT #130) — an owner is a coordinator with no human in the
loop — and the org crate dissolved into this concept (INTENT #132). The
round-2 sketch stands: coordinators attach to workspaces; workspaces often
attach to worktrees; **one coordinator per workspace**. "I really only like
to talk to coordinators — I don't like talking to individual agents at
all." AUI threads = a coordinator with a workspace attached. Coordinators
live inside a project on a branch; the workspace merges into other branches
with it. **Coordinator compaction:** "I should be able to compact the
thread and basically lose nothing." The round-3 sharpening (INTENT
#130/#133): the concept becomes a **MINI-HARNESS** — multi-threaded (one
human thread + persistent per-message threads), an **HTTP-like message
protocol** for sending one message to a workspace coordinator, handled in a
dedicated thread then **broadcast to sibling threads** in the same space;
messaging capability, a dedicated workspace construct, and compaction built
in. Each coordinator has its own KG and workspace; the workspace concept
stays AS IT IS (the existing .mind construct); per-node-TYPE directory
structures are tracked within the `schema` service; the archetype wants a
grounded name (deva, angel — "a spiritual construct, an archetype — but
grounded, not mystical"). The endgame flow (INTENT #127 — owners
hierarchical over the topological map, propose → approve → dispatch →
report with contract + version info) now lives under this concept. The
git+projects+environments+coordinators interplay is still "a bunch of
disparate things" that must be made elegant together. Full verbatim-grade
record: → `org.md` (the concept's home) and `projects.md`.

## History

The round-by-round lock history (rounds 3–9: gateway merge, mesh internals,
VDB/db/KG layering, SQLite-locally, secrets rename, declarative triggers,
the aws crate, wave-2 authorization) that previously filled this file lives
in git history (`git log -- scaffold/overview.md`, through commit `0c5d6fb`)
and in the INTENT ledger; the surviving decisions are all encoded in the
component/contract files above. `wave2-plan.md` §5 records the flags carried
into the batches and their dispositions.
