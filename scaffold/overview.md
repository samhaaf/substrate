# Substrate V2 — Scaffold Overview (Decompose pass)

> **Stage 1, Step 1 (DECOMPOSE) only.** This file names the components, their
> nesting, and every contract edge between them, and designs the single wiring
> seam. It deliberately does **not** contain full per-component designs — the
> `components/<name>.md` files are STUBS (title + one paragraph). Full component
> design (step 2) and contract schemas + example data (step 3) are deferred and
> not yet authorized. See `~/code/harness/core-plugins/core/skills/scaffolding-pattern/`.

> **ROUND-3 FEEDBACK LOCK (2026-07-18), applied on top of the component-design
> pass (commit `6879866`):** (1) **gateway merged into mesh** — gateway ceases
> to exist as a component; mesh (now explicitly "the operating system") absorbs
> dashboard hosting + browser event fan-out + rollups/proxy
> (`components/gateway.md` is a tombstone). (2) Mesh's charter adds
> service-version tracking, inter-service version requirements, and boot
> ordering (internal layering: OPEN). (3) New components **`vfs`** (boring
> distributed flat file system) and **`projects`** (knowledge graph over vfs) —
> both requirements-only stubs. (4) New cross-cutting **boring surface schema**
> pattern (`contracts/surface-schema.md`); `types` is UNLOCKED. (5) **CCD stays
> top-level** — its distinguishing job is the Claude-Code-usage-limits game;
> the route-to-local-inference question is answered NO for Claude Code.
> (6) New named future placeholders: `agents`, OpenRouter-management, finance,
> AUI/interfaces.

## Scope of this pass

Combined scope across three sources:
1. **Every existing V1 crate/app** — the 19-member Cargo workspace on branch `V1`
   plus `ui/dashboard`. The operator's ask is to rethink the API/data contracts
   between every existing crate, so crate-to-crate edges are surfaced (as
   node-internal contract edges), not just the external service boundaries.
2. **The substantially-expanded `mesh` component** — beyond the already-finalized
   node-discovery/model-affinity routing design (`reports/mesh-design-synthesis.md`),
   mesh now also owns: a standardized/extensible Tailscale-query surface, a live
   WebSocket network-topology + self-connectivity layer, and a distributed,
   eventually-consistent service registry (slug -> host:port).
3. **The ideas-branch concepts** — a cloud-code/agent-manager daemon
   ("Marshall" / "CCM" / **CCD** = Cloud Code Daemon) and **Org** (a multi-agent
   communication / agentic-orgs framework built on top of CCD). Org is the
   **maximalist-consumer exception**: placeholder only, not decomposed this pass.

> **DATA-SOURCE CAVEAT (flagged, not silently resolved):** the `ideas` branch
> described in the brief **does not exist** — no local ref, and `git ls-remote`
> on both `origin` and `broomstick` shows only `V1`, `main`, `interface-layer`,
> `STT`. The branch was *planned* in the operator's own words ("we should just
> branch off of main and have a branch called ideas") but appears never to have
> been created; the idea capture that would have lived there instead exists as
> AUI voice-thread transcripts under `~/code/harness/.mind/threads/`. The CCD and
> Org content below is reconstructed from those transcripts, not from a branch.
> Treat CCD/Org scope as intent-capture-grade, not repo-grade.

## Component tree (nesting shown)

Top-level components are what the operator reads back one at a time. Nested
children are genuinely-hard sub-parts isolated by COMPLEXITY (scaffolding
principle: decompose by design-difficulty, not line count; thin binary glue
`bin/*` clumps into its parent and is not its own component).

```
types                         [existing, UNLOCKED] shared contract-types foundation (zero-dep);
                                as-is freeze removed round-3; gains the surface-schema domain module
inference                     [existing, RESHAPE; name locked as "inference"]  per-machine LLM runtime
  ├─ store                    [existing] SQLite system-of-record (per node)
  ├─ engine                   [existing] llama-server process + InferenceBackend seam
  ├─ scheduler                [existing] admission / swap / preemption / recovery loop
  ├─ models                   [existing] registry sync + resumable download + eviction
  ├─ cache                    [existing] disk-backed KV/prefix cache
  ├─ telemetry                [existing] system sampling + throughput estimator
  ├─ benchmark                [existing] idle-gated priority-0 sweep orchestrator
  └─ api                      [existing] Axum /v1 REST + WS surface (node's external contract)
gc                            [existing, RESHAPE round-3] per-node storage-enforcement tool called by
                                vfs (embedded lib + :8430 surface; ONE centralized per-node store;
                                open: rolled into vfs vs. called-as-tool)
mesh                          [RESHAPE + EXPAND — "the operating system"] coordination plane; absorbs
                                gateway (dashboard hosting + event fan-out + rollups); adds service-
                                version/requirements/boot-order tracking (internal layering OPEN)
  ├─ tailscale-query          [NEW] standardized, extensible Tailscale-status query surface
  ├─ network-topology         [NEW] live WS: device on/off + self-connectivity-loss events
  ├─ service-registry         [NEW] distributed, eventually-consistent slug -> host:port registry
  └─ completion-router        [RESHAPE] NodeRegistry + model-affinity balancer + forward()
dashboard                     [existing, RESHAPE] Svelte/Vite frontend (per-node dimension; served BY
                                mesh; rendering becomes surface-schema-driven)
vfs                           [NEW round-3, requirements-only] boring distributed flat file system
                                (per-dir policies, replication factor, warm/cold RAID-ish nodes,
                                encrypted S3 overflow tier; calls gc per node)
projects                      [NEW round-3, requirements-only] graphical FS / knowledge graph over vfs;
                                centralized push registry for dashboards + source; app-building layer
db                            [existing, as-is; SCOPE CONFIRMED] Postgres/Supabase control-plane crate + `db` CLI
ccd                           [NEW; TOP-LEVEL CONFIRMED round-3] Cloud Code Daemon (Marshall/CCM):
                                cloud-code manager + Claude-Code-usage-limits budget engine
org                           [NEW, PLACEHOLDER — NOT DECOMPOSED THIS PASS]  agentic-orgs framework
                                imports: inference, ccd, db
```

**Legend:** `[existing, as-is]` = V1 crate kept unchanged; `[existing, RESHAPE]`
= V1 crate whose contract/scope is being reworked; `[NEW]` = did not exist in V1.

> **GATEWAY IS GONE (2026-07-18):** the former top-level `gateway` component
> merged into `mesh` — "all inter-node communication needs to go through mesh…
> I'm not seeing a need for gateway" (operator). `components/gateway.md` is a
> tombstone; its edges were rewired to mesh (see the contract graph below).

### Notes on component-status calls

- **`types` is the pre-existing shared-types root.** In a from-scratch
  scaffolding run the Contract Harmonizer (step 3) generates a shared
  contract-types package; here `substrate-types` already plays that role. The
  Contract Harmonizer will reconcile the authored contracts with this existing
  crate rather than generating a greenfield one. Flagged as a seam decision for
  step 3, not resolved here.
- **`inference` naming is now LOCKED — no further rename.** A second rename
  (to `llm`, or to `gen`) was floated ("I'm tempted to rename the inference to
  just like LLM") after the `node -> inference` rename already done in V1; the
  operator has since decided against it, on two grounds: (1) the crate is meant
  to stay modality-agnostic as new generation modalities (text-to-image,
  text-to-video, etc.) get added — they extend this same crate rather than
  spinning out separate ones, so a text/LLM-flavored name would misdescribe
  its future scope; (2) `gen` in particular was rejected because it's a
  homophone for the name "Jen" once spoken aloud, which is a real problem in a
  voice-operated project (STT). See `components/inference.md` for the full note.
- **`gc` is dual-role, now converging (round-3):** it is both a library embedded
  inside the inference (models/cache/engine register managed dirs for disk-budget
  enforcement) AND a per-node `:8430` HTTP/WS surface aggregated by mesh
  (formerly proxied by gateway). Round-3 resolutions: the two-`gc.db` split
  collapses into **ONE centralized per-node GC store**, updatable over API/WS,
  with GC managing `.gc` config files in managed directories; and GC becomes
  the **per-node tool `vfs` calls** to enforce that device's storage (open:
  eventually rolled up into vfs vs. staying called-as-tool). See `gc.md`.
- **`db` vs `store`:** two distinct database concerns. `store` = per-node inference
  SQLite (system of record for completions). `db` = the operational Postgres/
  Supabase control plane (migrations, edge functions, outbox, noun-verb CLI) —
  the "DB thing in a local stack" the operator wants for the Org/game demo.
- **`db`'s in-scope status is now CONFIRMED** (previously flagged as an
  open/orthogonal inclusion call in the decompose pass). Two confirmed
  consumers this round: `org` will depend on `db` for its own self-restructuring
  knowledge graph (metacognitive node add/restructure operations are `db`
  operations — see `db-control-plane` / `org.md`), and `inference` will depend
  on `db` to initialize its own database when standing up on a new mesh node
  rather than bootstrapping that itself (new edge `db-inference-init`, see
  `db.md`). `db`'s longer-term aspirational direction (schema-once/deploy-
  anywhere data-model layer, field-change triggers, eventual ORM/query
  virtualization) is noted in `db.md` as future-only, not scoped this pass.
- **`mesh` absorbs the expanded scope as four children.** The Tailscale-query
  surface EARNED its own (nested) component: the operator explicitly wants it
  "standardized" and "extensible" ("add new tools as we go"), and it is consumed
  by two independent siblings (network-topology and completion-router's node
  discovery). Folding it into one flat `mesh` would hide a genuinely separable,
  reusable surface. The service-registry likewise earned its own child
  (eventual-consistency + peer replication is the hardest new sub-problem).

## The maximalist-consumer exception: **Org**

Identified as **Org** (confirmed from the operator's captured intent, not just
suspected). Rationale: the operator described one concept as "kind of the
maximalist, like, resource-consumption thing that it might want from all of the
other services, as it is going to take advantage of all the services," and in
the same capture defined **Org** as "a second layer built on top of CCD for
agents to actually communicate," a game-studio-structured multi-agent framework
that will make LLM calls via inference, use the DB in a local stack, resolve
services via the mesh, and drive agents via CCD. That is the broad, everything-
consuming layer. **CCD is NOT the exception** — CCD is a concrete daemon with a
bounded job (manage cloud-code agents + play the Claude-Code-usage-limits
budget game; the route-their-LLM-calls-to-local-inference idea was answered NO
round-3) and IS decomposed
this pass. Build order the operator locked: **CCD first, then Org.**

Per the exception rule, Org gets only a placeholder (`components/org.md`: name +
one line, marked "not decomposed this pass"). Its anticipated needs — broad read
access to completions/inference, system state, the mesh service-registry, the db
control plane, and CCD agent-management — are folded into the *other* components'
edges and design notes so their contracts are shaped for a maximalist consumer
from the start (see `service-lookup`, `v1-completion-api`, `agent-management`,
`db-control-plane`, `org-on-ccd`).

**Confirmed this round (still placeholder-level, not a full design):** Org's
design note is now "this is where the user creates the foundation for
autonomous organizations, where agents, ripples, and AI pipelines take
advantage of all the tools in the rest of the repo to run organizations
autonomously," and its explicit minimum import list is locked as `inference`,
`ccd`, and `db` — see `components/org.md`. Nothing else about Org changes; it
remains un-decomposed.

## Contract graph (every edge)

Edges are named; each has a stub in `contracts/<edge-name>.md`. Direction noted
where a caller/callee asymmetry matters.

### Cross-service / external edges

| Edge | Parties | Carries (one line) |
|------|---------|--------------------|
| `v1-completion-api` | client / mesh -> inference (api) | The `/v1/` REST+WS completion surface: submit/status/cancel/priority/result/stream, collections, models, estimate. Mesh forwards it transparently. |
| `node-state-poll` | mesh.completion-router -> inference | `GET /v1/system/state` + `GET /v1/models` reads that feed the NodeRegistry (health, load, per-node model inventory). |
| `inference-events` | mesh <- inference | Per-node WS event stream + REST proxy mesh's observability plane subscribes to (one subscription per node). *(was gateway <- inference)* |
| `gc-events` | mesh <- gc | GC's per-node WS event stream + REST surface (`:8430`); also the remote-update path to the ONE per-node GC store. *(was gateway <- gc)* |
| `ccd-events` | mesh <- ccd | **PROPOSED (pending confirmation).** CCD's agent-lifecycle/run WS event stream, aggregated like `inference-events`/`gc-events`. *(was gateway <- ccd)* |
| `dashboard-feed` | mesh -> dashboard | Aggregated `GET /events` WS fan-out + REST + static hosting the dashboard renders. *(was gateway -> dashboard)* |
| `surface-schema` | every service -> mesh dashboard | **NEW round-3.** Each service publishes a schema of its observable surface (render + interaction description, in `types`' shared schema language); the dashboard renders every component from it. |
| `llm-calls` | ccd -> inference | **NARROWED round-3:** Claude Code agents are NOT routed to local inference (they can't choose models); edge is usage-observation/metering-shaped. Local-inference serving is reserved for the future `agents` layer's other agent types. |
| `service-registration` | ccd <-> mesh.service-registry | CCD registers its own slug->endpoint and resolves other services by slug. |
| `agent-management` | ccd <-> cloud-code agents | Spawn / track / route cloud-code (Claude Code) agent processes; the multi-agent substrate CCD owns. |
| `org-on-ccd` | org -> ccd (+ maximalist reads) | Org built atop CCD for inter-agent comms; also the shaped-for edge bundling Org's broad reads of inference/state/registry/db. Round-3: org calls CCD with a priority the budget engine honors. |
| `db-control-plane` | db <-> consumers (Org/game-demo) | Noun-verb DB control plane: migrations, edge functions, query, outbox, over Postgres/SQLite. |
| `db-inference-init` | inference -> db | A new `inference` node standing up on a fresh mesh node initializes its own database through `db` rather than bootstrapping it itself. |
| `vfs-gc` | vfs <-> gc | **NEW round-3 (requirements-only).** VFS calls the node-local GC tool to enforce that device's directory policies (max size, FIFO/LRU-ish eviction). |
| `vfs-mesh` | vfs <-> mesh | **NEW round-3 (requirements-only).** VFS registration + node/drive topology awareness + per-node perf reporting (uptime, read/write latency). |
| `projects-vfs` | projects -> vfs | **NEW round-3 (requirements-only).** The knowledge graph points into sections of the flat VFS; project artifacts stored through it. |
| `projects-mesh` | projects <-> mesh | **NEW round-3 (requirements-only).** Registry push (dashboards + source) + surfacing project dashboards on the mesh dashboard via surface schemas. |

**Collapsed edge:** `mesh-registry-read` (was gateway -> mesh) no longer exists —
with gateway absorbed, that read is mesh consulting its own registry in-process;
`contracts/mesh-registry-read.md` is a tombstone.

### mesh-internal edges

| Edge | Parties | Carries |
|------|---------|---------|
| `tailscale-status` | mesh.tailscale-query -> {network-topology, completion-router} | Parsed Tailscale self+peer status snapshots (hostname, DNSName, IPs, tags, online). |
| `network-events` | mesh.network-topology -> any consumer | WS stream: peer device on/off transitions + this device's own Tailscale-connectivity-loss events. |
| `service-lookup` | mesh.service-registry <-> any device | `register(slug, host:port)` / `resolve(slug) -> endpoint`; eventually consistent, queryable from any device. |
| `registry-replication` | service-registry <-> service-registry (peers) | The eventual-consistency replication/gossip edge between per-device registry instances. |

### inference-internal edges (rethinking crate contracts)

| Edge | Parties | Carries |
|------|---------|---------|
| `store-access` | store <-> {engine, scheduler, models, cache, telemetry, benchmark, api} | The SQLite system-of-record CRUD + observer seam (completions, collections, models, results, benchmarks, kv_cache). |
| `engine-exec` | scheduler -> engine | Submit/drain completions, slot lifecycle, model swap through the `InferenceBackend`. |
| `system-state` | telemetry -> {scheduler, api} | `SystemState` snapshots (running/pending counts, memory pressure, resident model) + throughput estimates. |
| `model-ensure` | scheduler -> models | Ensure a model is downloaded/available before a swap; download-pipeline status. |
| `kv-cache` | engine <-> cache | KV/prefix cache slot save/restore keyed by (model_id, prompt_hash). |
| `gc-managed-dirs` | {models, cache, engine} -> gc (embedded lib) | Register managed dirs/entries, touch/lock, sweep — disk-budget enforcement in-process (distinct from the GC daemon HTTP edge). |
| `benchmark-collections` | benchmark -> scheduler | Priority-0, full-system-exclusive sweep collections submitted through the scheduler/queue. |
| `api-dispatch` | api -> {scheduler, store, telemetry, benchmark} | The node-internal side of `v1-completion-api`: how the Axum handlers dispatch into subsystems. |
| `kernel-confidence` | scheduler -> telemetry (kernel) | Effective-concurrency / confidence-kernel read edge (added by the component-design pass; previously missing from this table). |

## The single wiring seam

**Seam = the mesh `service-registry`, as the runtime service-composition point.**

Substrate V2 is a multi-process distributed system, so the "single wiring seam"
is not one process's composition root but the ONE mechanism through which every
service discovers and binds to every other at runtime. The operator's expanded
mesh vision names this directly: a distributed slug->host:port registry that
"solve[s] some of the complexity of having a centralized location for tools that
sit on top of the mesh," so "we can query the mesh to find where a service is
being rendered and access it."

Concretely, the Assembler (step 6) touches exactly one thing: the
**registry-backed startup wiring** (`service-lookup`). Each top-level service, at
boot, (a) **registers** its own `slug -> host:port` into the service-registry,
and (b) **resolves** each dependency by slug rather than by static URL. This
replaces today's static config (e.g. the old gateway-code-now-in-mesh
`inference_url` / `gc_url`, the mesh's static node list) with dynamic
resolution through the one seam. A
static-config fallback is retained for single-box dev so the seam degrades
gracefully.

Everything else connects only through the named contracts above; the intra-process
composition roots that already exist (`InferenceService::start`, `MeshProxy::start`,
`GcService`, the observability-plane `serve()` mesh absorbed from gateway)
remain each component's private wiring and are
NOT the assembly seam — the Assembler wires services to each other solely at the
registry.

**Open note (candidate refactor, do NOT design now):** adopting the registry as
the seam implies reworking every currently-static endpoint config to resolve via
`service-lookup`: the absorbed observability plane's `inference_url`/`gc_url`
(now mesh-internal config), the mesh's node list,
CCD's endpoints, Org's service map, and the new `vfs`/`projects` services. The
mesh-design-synthesis
already anticipated the observability-plane change (OQ-style, written pre-merge
against gateway). This is a cross-cutting refactor
to flag for the operator, not to specify in this pass.

## Future / placeholder concepts (named only — NO component files)

Operator-named coming-later scope, recorded as one-liners so nothing squats on
the names. These are layer-six territory alongside `org` and CCD's strategic
layer; none is decomposed, designed, or given a `components/` file this pass:

- **`agents`** — a future generalization layered ON TOP of CCD for non-Claude-
  Code agent types (which CAN choose models and may use local inference); CCD
  itself stays top-level and is NOT merged into this umbrella.
- **OpenRouter-management** — a service managing OpenRouter API keys: per-key
  creation with per-key budgets, dashboard-managed.
- **finance** — cost tracking, per-project (eventually attaching to `projects`'
  per-project metadata).
- **AUI / interfaces** — a voice interface over the whole mesh.

## Carry-forward open questions for later steps

- OQ-3 (mesh-design-synthesis): final shape of the shared `NodeInfo` /
  `NodeCapabilities` in `types` — a step-3 (Contract Harmonizer) decision, since
  it's the shared-types reconciliation with the pre-existing `types` crate.
- ~~CCD siting~~: RESOLVED (component-design pass) — CCD lives inside the
  Substrate workspace as a first-class member; see `ccd.md`.
- **Mesh internal layering (NEW round-3):** mesh's OS-scope expansion is
  acknowledged as large; the operator has been asked whether mesh should
  decompose internally into layered libs. Open — see `mesh.md`.
- **GC rolled into VFS vs. called-as-tool (NEW round-3):** GC is now the
  per-node tool VFS calls; "might need to get rolled up into the VFS." Open —
  see `gc.md` / `vfs.md`.
- **Surface-schema language shape (NEW round-3):** the `types` schema language
  for the boring surface schema is requirements-only; a step-3 / Contract
  Harmonizer design concern. See `contracts/surface-schema.md`.
- ~~`inference -> llm` rename~~: RESOLVED this round — name stays `inference`; see notes above.
- Org convergence: a separate prototype ("autonomous org", `~/code/career_crafter`
  branch `Lazarus`) may become Org instead of a from-scratch build — the operator
  flagged this as a blocking question. Out of scope to resolve here; noted so Org's
  placeholder isn't mistaken for a greenlit greenfield build.
