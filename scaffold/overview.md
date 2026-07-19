# Mind OS — Scaffold Overview (wave-2 close-out)

> **THE SYSTEM IS "MIND OS" (locked 2026-07-18).** "What this is becoming is
> an operating system of my mind, so we should call it Mind OS — that's the
> name." **"Substrate" remains the repo/folder/GitHub name until version one
> publishes.** (Mycelium rejected; ripple-derived names rejected — "ripples"
> is a RESERVED term, see the placeholder section.)

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

- **47 component files** in `scaffold/components/` — 44 live modules across
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

**Governing precedence:** INTENT items (#1–#107, harness workspace) win over
scaffold text; contract files win over component-file proposals;
`wave2-plan.md` remains the module-inventory record of the decompose step.

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
protocol — db's second public surface) · `vdb-vfs` (SQLite-file-in-VFS;
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
  rolling updates. The mixed-version update protocol remains OPEN (friction
  report).
- The assembler (skeleton phase) touches exactly this seam; each crate's
  private composition root (`InferenceService::start`, mesh's ring layering)
  is NOT the seam.

## Standing cross-cutting principles

1. **INTENT wins.** Where scaffold text conflicts with the INTENT ledger
   (#1–#107), INTENT governs.
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
   never standalone services; replicated state is LWW (naive by blessing,
   HLC-ratchet hardening pending operator confirmation — friction report).
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
   "no runtime version tracking for shared libs" (friction report).
10. **Naming guardrails:** the module is `vdb`, "stack" is the pattern;
    "ripples" is RESERVED and names nothing in v1; `inference` will not be
    renamed.

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

## History

The round-by-round lock history (rounds 3–9: gateway merge, mesh internals,
VDB/db/KG layering, SQLite-locally, secrets rename, declarative triggers,
the aws crate, wave-2 authorization) that previously filled this file lives
in git history (`git log -- scaffold/overview.md`, through commit `0c5d6fb`)
and in the INTENT ledger; the surviving decisions are all encoded in the
component/contract files above. `wave2-plan.md` §5 records the flags carried
into the batches and their dispositions.
