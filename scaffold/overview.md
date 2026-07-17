# Substrate V2 — Scaffold Overview (Decompose pass)

> **Stage 1, Step 1 (DECOMPOSE) only.** This file names the components, their
> nesting, and every contract edge between them, and designs the single wiring
> seam. It deliberately does **not** contain full per-component designs — the
> `components/<name>.md` files are STUBS (title + one paragraph). Full component
> design (step 2) and contract schemas + example data (step 3) are deferred and
> not yet authorized. See `~/code/harness/core-plugins/core/skills/scaffolding-pattern/`.

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
types                         [existing, as-is] shared contract-types foundation (zero-dep)
inference                     [existing, RESHAPE; name locked as "inference"]  per-machine LLM runtime
  ├─ store                    [existing] SQLite system-of-record (per node)
  ├─ engine                   [existing] llama-server process + InferenceBackend seam
  ├─ scheduler                [existing] admission / swap / preemption / recovery loop
  ├─ models                   [existing] registry sync + resumable download + eviction
  ├─ cache                    [existing] disk-backed KV/prefix cache
  ├─ telemetry                [existing] system sampling + throughput estimator
  ├─ benchmark                [existing] idle-gated priority-0 sweep orchestrator
  └─ api                      [existing] Axum /v1 REST + WS surface (node's external contract)
gc                            [existing, as-is] filesystem garbage collector (embedded lib + :8430 daemon)
mesh                          [RESHAPE + EXPAND] network/coordination plane
  ├─ tailscale-query          [NEW] standardized, extensible Tailscale-status query surface
  ├─ network-topology         [NEW] live WS: device on/off + self-connectivity-loss events
  ├─ service-registry         [NEW] distributed, eventually-consistent slug -> host:port registry
  └─ completion-router        [RESHAPE] NodeRegistry + model-affinity balancer + forward()
gateway                       [existing, RESHAPE] aggregation/observability plane (:8400)
dashboard                     [existing, RESHAPE] Svelte/Vite frontend (gains node dimension)
db                            [existing, as-is; SCOPE CONFIRMED] Postgres/Supabase control-plane crate + `db` CLI
ccd                           [NEW] Cloud Code Daemon (Marshall/CCM): agent/cloud-code manager
org                           [NEW, PLACEHOLDER — NOT DECOMPOSED THIS PASS]  agentic-orgs framework
                                imports: inference, ccd, db
```

**Legend:** `[existing, as-is]` = V1 crate kept unchanged; `[existing, RESHAPE]`
= V1 crate whose contract/scope is being reworked; `[NEW]` = did not exist in V1.

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
- **`gc` is dual-role:** it is both a library embedded inside the inference
  (models/cache/engine register managed dirs for disk-budget enforcement) AND a
  standalone `:8430` daemon proxied by the gateway. Both roles are kept.
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
bounded job (manage cloud-code agents + route their LLM calls) and IS decomposed
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
| `inference-events` | gateway <- inference | Per-node WS event stream + REST proxy the gateway subscribes to (one subscription per node). |
| `gc-events` | gateway <- gc | GC daemon WS event stream + REST proxy (`:8430`). |
| `mesh-registry-read` | gateway -> mesh | Gateway resolves node endpoints + fleet state via mesh's NodeRegistry / service-registry (`/api/nodes`, `/api/mesh/stats`). |
| `dashboard-feed` | gateway -> dashboard | Aggregated `GET /events` WS fan-out + REST + static hosting the dashboard renders. |
| `llm-calls` | ccd -> inference | CCD/agents make model calls through the node's `/v1/` API (LLM calls run on local inference). |
| `service-registration` | ccd <-> mesh.service-registry | CCD registers its own slug->endpoint and resolves other services by slug. |
| `agent-management` | ccd <-> cloud-code agents | Spawn / track / route cloud-code (Claude Code) agent processes; the multi-agent substrate CCD owns. |
| `org-on-ccd` | org -> ccd (+ maximalist reads) | Org built atop CCD for inter-agent comms; also the shaped-for edge bundling Org's broad reads of inference/state/registry/db. |
| `db-control-plane` | db <-> consumers (Org/game-demo) | Noun-verb DB control plane: migrations, edge functions, query, outbox, over Postgres/SQLite. |
| `db-inference-init` | inference -> db | **NEW this round.** A new `inference` node standing up on a fresh mesh node initializes its own database through `db` rather than bootstrapping it itself. |

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
replaces today's static config (e.g. the gateway's `inference_url` / `gc_url`,
the mesh's static node list) with dynamic resolution through the one seam. A
static-config fallback is retained for single-box dev so the seam degrades
gracefully.

Everything else connects only through the named contracts above; the intra-process
composition roots that already exist (`InferenceService::start`, `MeshProxy::start`,
`GcService`, the gateway `serve()`) remain each component's private wiring and are
NOT the assembly seam — the Assembler wires services to each other solely at the
registry.

**Open note (candidate refactor, do NOT design now):** adopting the registry as
the seam implies reworking every currently-static endpoint config to resolve via
`service-lookup`: the gateway's `inference_url`/`gc_url`, the mesh's node list,
CCD's inference endpoint, and Org's service map. The mesh-design-synthesis
already anticipated the gateway change (OQ-style). This is a cross-cutting refactor
to flag for the operator, not to specify in this pass.

## Carry-forward open questions for later steps

- OQ-3 (mesh-design-synthesis): final shape of the shared `NodeInfo` /
  `NodeCapabilities` in `types` — a step-3 (Contract Harmonizer) decision, since
  it's the shared-types reconciliation with the pre-existing `types` crate.
- CCD siting: "does CCD live inside the Substrate workspace or stand alone?" —
  the operator's own open question; affects whether `ccd` is a workspace member
  or a sibling repo. Left open.
- ~~`inference -> llm` rename~~: RESOLVED this round — name stays `inference`; see notes above.
- Org convergence: a separate prototype ("autonomous org", `~/code/career_crafter`
  branch `Lazarus`) may become Org instead of a from-scratch build — the operator
  flagged this as a blocking question. Out of scope to resolve here; noted so Org's
  placeholder isn't mistaken for a greenlit greenfield build.
