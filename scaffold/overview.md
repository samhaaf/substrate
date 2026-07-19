# Substrate V2 — Scaffold Overview (Decompose pass)

> **THE SYSTEM IS "MIND OS" (locked 2026-07-18).** The system being designed
> here is **Mind OS** — "what this is becoming is an operating system of my
> mind, so we should call it Mind OS — that's the name." **"Substrate" remains
> the repo/folder/GitHub name until version one publishes**, at which point it
> is published as Mind OS. (Mycelium rejected; ripple-derived names rejected —
> see the reserved-term note in the future section.)

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

> **ROUND-3 LOCK, SECOND BATCH (2026-07-18, same day, applied on top of commit
> `356b6af`):** (A) new build-now component **`kg`** (Knowledge Graph service;
> requirements-only stub-plus — distributed graph across the whole mesh,
> schema-locked nodes+edges, versioned graph templates, VFS file pointers,
> AWS/S3 cross-boundary sync; consistency model OPEN); `projects` now straddles
> kg + vfs. (B) new layer-six future placeholder **`artifacts`** ("no more
> files — artifacts": typed, schema'd, interactable; projects depends on it).
> (C) **the S3 adapter moves INSIDE mesh** — supersedes the round-3
> S3-as-VFS-feature framing; mesh owns all eventual-consistency/replication, so
> vfs and kg reach S3 through mesh's adapter.

> **ROUNDS 4–5 LOCK (2026-07-18, applied on top of commit `5dcadc1`):**
> (A) **mesh internals locked** — standard WS pub/sub protocol; internal
> utility LIBS (service registry, replicated KV, `locks`, `cron`, S3 adapter —
> never standalone crates/services); **port LOCKED `3649`** (supersedes
> `:8419`); stickiness via system-level resurrection; rigorous zombie-killing;
> single-port locality; two addressing classes ("any node running X" vs "X on
> node N") — see `components/mesh.md` concerns 7–11 (`locks` partition
> semantics OPEN). (B) new build-now component **`stack`** (the lightweight
> database-with-handlers runtime; **HARD RULE: NO DOCKER locally, ever** —
> containers only in AWS via an AWS adapter; Deno confirmed as the TS-handler
> runtime). (C) `db`'s query/virtualization layer upgraded to **in-scope
> direction**. (D) KG gains the trigger/handler paradigm via **ONE shared
> handler/execution engine** with stack (see the shared-libraries section —
> the word "ripples" is RESERVED and must NOT name this engine). (E) new
> requirements-only components **`environments`** and **`cicd`**. (F) new open
> fork: **`repo` crate vs git-in-the-VFS**. (G) CCD owns its own usage
> database; **`finance` renamed `spend`** (spend queries ccd, pull-shaped).
> (H) **the system is Mind OS** (see the note at the top). (I) versioning:
> pairwise service dependencies + minimal-restart rolling updates preferred;
> commit-as-release-set NOT adopted; mixed-version update protocol OPEN.

> **ROUND-6 LOCK (2026-07-19, applied on top of commit `ea715af`):**
> (A) **mesh init-supervision ANSWERED (3rd ask)** — mesh starts and
> supervises the local services; services expose an observable
> interruptibility state; non-critical updates wait for idle; **port-handoff
> update pattern** (new version on new port → registry flip → old port down);
> plus a **two-way, priority-laddered graceful-restart protocol** built into
> EVERY service from the beginning (exact ladder: latitude granted, discuss)
> and new internal **queues + dead-letter queues** (SQS-modeled; dead-letter
> escalation → ccd investigation) — see `mesh.md` concerns 12–14. `locks`
> gains a required catchable partition-merge error type. (B) **three new
> requirements-only components:** **`vault`** (secrets — agents/LLMs
> use-without-seeing; **RENAMED `secrets` round-8**), **`rollup`** (the prompt/plugin rollup system; crate
> name OPEN: rollup/plugins/other; CCD ideally built on top of it), and
> **`repo`** (**fork RESOLVED** — repo IS its own crate on top of the VFS;
> both stay boring). (C) the **stack-vs-db boundary recorded as THE
> operator's most-nebulous OPEN question** (+ the **VDB** idea; Postgres/
> Docker ambiguity flagged unreconciled) — see `stack.md`/`db.md`.
> (D) **PROVENANCE IS FIRST-ORDER** — new standing cross-cutting principle.
> (E) bandits clarified NOT-mesh (in-app concern — `environments.md`); `org`
> gains per-application owning agents + the inter-agent negotiation protocol;
> `projects` gains the confirmed .mind workspace-schema migration.
> (F) **pairwise data contracts affirmed verbatim as the operating model**
> (see the note at the contract graph).

> **ROUND-7 LOCK (2026-07-19, applied on top of commit `e70eded`):**
> (A) **VDB ELEVATED** — VDB is now the working name for the
> **deploy-anywhere implementation of the stack pattern**: one abstract
> runtime with three adapter targets — **local (the SQLite stack daemon),
> Supabase, and AWS (RDS + Lambda)**; `db`'s Capabilities-gated driver
> architecture is the natural seed of the adapter matrix. Two attached OPEN
> questions: how the stack pattern gets "baked into" VDB, and **whether KG
> should be BUILT ON VDB** — see `stack.md`/`db.md`/`kg.md`. (B) the
> local-Postgres decision is now **GATED on a REQUIRED SQLite-sufficiency
> analysis** (daemon-level gap analysis: does SQLite + stack daemon + mesh
> cron/pub-sub cover everything Postgres provides — pg_cron → mesh cron,
> LISTEN/NOTIFY → mesh pub/sub, procedural triggers → Deno/SQL handlers;
> definitive version belongs in stack/VDB's full design) — see `stack.md`.
> (C) **rollup insert types LOCKED** — `raw` (inline) vs `reference`
> (handle/pointer by ID or name, lazily loadable), extending the harness
> prior art's prompt/slot/file/prompt-file markers — see `rollup.md`.
> (D) **vault LLM-safety mechanism LOCKED** — raw + ID addressing;
> LLM-feeding services default `llm_safe=true`; raw requests fail or degrade
> to ID-plus-warning; **no secret ever reaches an LLM**; rollup must never
> resolve a vault raw reference in LLM-bound content — see
> `secrets.md` (was `vault.md`; renamed round-8)/`rollup.md`. (E) **queue-pull semantics** — pulls acquire a
> semaphore keyed by the event ID (`locks` + queues compose; ~exactly-once
> over at-least-once; duplicate-handling a per-case seam); a dead-letter
> queue is just a queue; "the entire contract of the system is basically
> built off the queueing mechanism" — see `mesh.md` concern 14.
> (F) **provenance SCOPED** — configured per-project/per-database; **VDB is
> its primary home**; VFS carries only a lighter requirement (see the
> standing principles below and `stack.md`/`vfs.md`).

> **ROUND-8 LOCK (2026-07-19, applied on top of commit `35039b7`):**
> (A) **the VDB/db/KG LAYERING IS LOCKED — the stack-vs-db "most-nebulous"
> open question is RESOLVED.** Operator, verbatim: "VDB becomes the daemon
> which tracks the execution, and it takes advantage of the db crate to
> actually run the actions against specific databases — we extend db as
> necessary to support VDB. And KG we build on top of VDB." **VDB = the
> daemon tracking/executing the stack pattern; `db` = the crate VDB uses
> to run actions (extended as needed); KG builds ON TOP of VDB; VFS sits
> BELOW VDB** (the SQLite files live in the VFS). Layer order:
> **VFS < VDB < KG** — see `stack.md`/`db.md`/`kg.md`/`vfs.md` and the
> resolved entry in the open questions below. (B) **KG routing +
> registry** — each graph links to a project+environment; the environment
> routes storage (local → SQLite, cloud → promoted to Supabase/AWS);
> don't over-constrain (a local-purpose KG may support a cloud
> environment); KG stays a **GLOBAL registry of ALL graphs**
> (cross-project reuse, schema referencing); each graph routes to a
> location in VDB and/or VFS — see `kg.md`. (C) **`vault` RENAMED →
> `secrets`** ("vault" collides with Supabase Vault, "SM" with AWS Secrets
> Manager; "secrets is the name because that's what it is") —
> `components/secrets.md`, contract `vault-mesh` → `secrets-mesh`; new
> requirements: **distributed across nodes (or replication factor ~3)**;
> **adapters pushing secrets into Supabase Vault, AWS Secrets Manager,
> and GitHub Actions secrets**; interfaces with VDB, db, projects,
> environments, repo. `lib/db`'s real, existing Supabase-Vault module +
> keychain convention audited and recorded in `db.md`; `secrets` is the
> owner going forward, db reconciles to consume it. (D) **repo gains
> GitHub Actions management** — managing workflows on repos + linking
> secrets into GH Actions workflows — see `repo.md`. (E) **VOCABULARY
> LOCKED: queues / events / triggers / handlers** — queues hold **typed
> EVENTS** (standardized event-type struct in `types`); **TRIGGERS** are
> one-to-one queue→handler (many triggers per queue), FILTER by event
> type + payload content, and ASSEMBLE the handler's payload (the
> trigger, not the event, dictates handler input; assembly may call
> rollup); **HANDLERS** receive the assembled payload; the event-ID
> semaphore becomes a **per-trigger choice** (supersedes round-7's
> blanket every-pull rule) — see `mesh.md` concern 14, `types.md`, and
> the shared-libraries section.

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
                                + the WS pub/sub envelope module (rounds 4–5)
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
                                version/requirements/boot-order tracking and the S3 adapter (cold-
                                storage overflow, client-side encryption — round-3 second batch, moved
                                from vfs). Rounds 4–5: port LOCKED :3649; internal layering ANSWERED —
                                internal utility LIBS (replicated KV, locks, cron, S3 adapter, pub/sub
                                relay), never standalone; sticky (system-level resurrection); zombie-
                                killing; single-port locality; two addressing classes. Round-6: starts/
                                supervises local services (port-handoff updates); two-way graceful-
                                restart protocol; queues + dead-letter queues (SQS-modeled). Round-7:
                                queue pulls acquire a semaphore keyed by event ID (locks + queues
                                compose; ~exactly-once over at-least-once). Round-8: vocabulary locked
                                — typed EVENTS in queues; TRIGGERS (1:1 queue→handler, many per queue)
                                filter + assemble handler payloads; HANDLERS receive assembled
                                payloads; the event-ID semaphore is a per-trigger choice
  ├─ tailscale-query          [NEW] standardized, extensible Tailscale-status query surface
  ├─ network-topology         [NEW] live WS: device on/off + self-connectivity-loss events
  ├─ service-registry         [NEW] distributed, eventually-consistent slug -> host:port registry
  └─ completion-router        [RESHAPE] NodeRegistry + model-affinity balancer + forward()
dashboard                     [existing, RESHAPE] Svelte/Vite frontend (per-node dimension; served BY
                                mesh; rendering becomes surface-schema-driven)
vfs                           [NEW round-3, requirements-only] boring distributed flat file system
                                (per-dir policies, replication factor, warm/cold RAID-ish nodes;
                                calls gc per node; S3 overflow now via mesh's adapter); round-8: sits
                                BELOW VDB in the locked layering — VDB's SQLite files live here
                                (VFS < VDB < KG)
kg                            [NEW round-3 second batch, requirements-only] distributed knowledge-graph
                                service (schema-locked nodes+edges, versioned templates, VFS file
                                pointers, S3 cross-boundary sync via mesh; consistency model OPEN);
                                round-8: BUILT ON TOP of VDB (VFS < VDB < KG); each graph links to a
                                project+environment (environment routes storage: local→SQLite,
                                cloud→Supabase/AWS); GLOBAL registry of all graphs; graphs route to a
                                location in VDB and/or VFS
projects                      [NEW round-3, requirements-only] graphical FS / knowledge graph straddling
                                kg + vfs (graph encodes project structure; nodes point at vfs files);
                                centralized push registry for dashboards + source; app-building layer;
                                round-6: the .mind workspace schema migrates in (coordinator protocols,
                                artifacts, tasks; rollup-engine integration — "tasks are rolled up")
repo                          [NEW round-6, requirements-only; fork RESOLVED] its own crate built ON TOP
                                of the VFS (git-in-the-VFS rejected): push/sync between VFS and
                                git/GitHub repos; branches, worktrees, branches attachable to
                                environments; "the only non-boring thing about it is that it sits on
                                the VFS"; round-8: GitHub Actions management — managing workflows on
                                repos + linking secrets into GH Actions workflows
db                            [existing, as-is; SCOPE CONFIRMED] Postgres/Supabase control-plane crate + `db` CLI;
                                rounds 4–5: query/virtualization layer now in-scope DIRECTION
                                (standardized query interface over SQLite-in-VFS / Supabase / RDS);
                                round-7: its Capabilities-gated drivers are the seed of VDB's
                                adapter matrix; round-8: the crate VDB USES to run actions against
                                specific databases — extended as necessary to support VDB; its
                                existing secret handling (real Supabase-Vault module + keychain
                                convention) reconciles toward consuming `secrets`
stack                         [NEW rounds 4–5, requirements-only] lightweight database-with-handlers
                                runtime: daemon around a single SQLite-file-in-the-VFS, SQL + Deno/TS
                                handlers on row/table changes; NO DOCKER locally (hard rule);
                                round-7: VDB elevated to the deploy-anywhere stack-pattern
                                implementation (local SQLite daemon / Supabase / AWS RDS+Lambda);
                                local-Postgres decision GATED on a required SQLite-sufficiency
                                analysis; round-8: LAYERING LOCKED — VDB is the daemon that
                                tracks/executes the stack pattern, USING the db crate to run actions
                                (db extended as needed); KG on top of VDB; VFS below (VFS < VDB < KG);
                                the stack-vs-db open question is RESOLVED
environments                  [NEW rounds 4–5, requirements-only] environment = subset of a project;
                                CICD pipeline attachable; deploy-into-environment activates it;
                                chained blue/green deployments; bandit feature balancing (placement OPEN)
cicd                          [NEW rounds 4–5, requirements-only] pipelines consuming environments;
                                hierarchy vs environments OPEN (sibling-with-dependency assumed)
secrets                       [NEW round-6, requirements-only; RENAMED from `vault` round-8 —
                                Supabase Vault / AWS Secrets Manager collisions] secrets crate/service;
                                agents/LLMs can NEVER directly read a secret but CAN use one in context
                                (use-without-seeing, enforced at the access-pattern level); round-7:
                                llm_safe mechanism locked (raw/ID addressing; raw requests fail or
                                degrade to ID-plus-warning; no secret ever reaches an LLM); round-8:
                                distributed across nodes (replication ~3); adapters push secrets into
                                Supabase Vault, AWS Secrets Manager, GitHub Actions secrets;
                                interfaces with VDB, db, projects, environments, repo
rollup                        [NEW round-6, requirements-only; crate name OPEN: rollup/plugins/other]
                                prompt/plugin rollup: fragments referencing fragments via a syntax,
                                slots taking variables at reference time; specialized plugins generated
                                on demand (CCD ideally built on top of it); generalization ladder
                                string-rollup → file-rollup → directory-rollup; round-7: insert
                                types locked (raw vs reference)
ccd                           [NEW; TOP-LEVEL CONFIRMED round-3] Cloud Code Daemon (Marshall/CCM):
                                cloud-code manager + Claude-Code-usage-limits budget engine; rounds 4–5:
                                owns its own usage database (sessions/tokens/limits; queried by spend);
                                threads linkable to projects + optionally environments; round-6:
                                consumes rollup for its plugin/prompt assembly
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
  `db.md`). `db`'s longer-term direction (schema-once/deploy-anywhere
  data-model layer, field-change triggers, ORM/query virtualization) was
  noted in `db.md` as future-only — **partially SUPERSEDED rounds 4–5: the
  query/virtualization layer is now an in-scope DIRECTION** (standardized
  query interface over SQLite-in-VFS / Supabase / RDS backends), though its
  design pass is still not authorized. Working framing vs. the new `stack`
  — **superseded round-8 by the locked layering:** VDB = the daemon
  tracking/executing the stack pattern; `db` = the crate VDB uses to run
  actions against specific databases (extended as needed); VFS < VDB < KG.
  See `db.md` / `stack.md`.
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
remains un-decomposed. ("Ripples" in that quote is the operator's RESERVED
future concept — see the reserved-term note in the future section — not the
v1 handler/execution engine.)

## Contract graph (every edge)

> **PAIRWISE DATA CONTRACTS — AFFIRMED VERBATIM (round-6) as the whole point
> and THE OPERATING MODEL for all future updates:** "Clean data contracts
> between all the services... if one service ever needs another in a way the
> contract doesn't support, we go into a design session about the updated
> contract and how it ripples into other services' contracts. Data contracts
> between each pair of services, mediated by mesh, is how we support future
> updates elegantly with low technical debt." Contracts sit between each
> service pair, mediated by mesh; **any contract change triggers a design
> session analyzing its ripple effects** across the other contracts. ("Ripples"
> in that quote is the operator's ordinary-English verb, not the RESERVED
> term — see the future section.)

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
| `kg-mesh` | kg <-> mesh | **NEW round-3 second batch (requirements-only).** KG registration + graph replication across nodes and into S3 through mesh's adapter; graph-merge consistency model OPEN ("the superset" of the registry's LWW KV). |
| `kg-vfs` | kg -> vfs | **NEW round-3 second batch (requirements-only).** KG nodes point at VFS files; existence validation of the pointed-at file. |
| `stack-vfs` | stack -> vfs | **NEW rounds 4–5 (requirements-only).** The single SQLite file each stack daemon wraps is stored in / accessed through the VFS. |
| `stack-mesh` | stack <-> mesh | **NEW rounds 4–5 (requirements-only).** Registration via the local mesh daemon (:3649, single-port locality) + distributed-handler coordination via mesh's `locks` lib. |
| `secrets-mesh` | secrets <-> mesh | **NEW round-6 (requirements-only); RENAMED round-8 (was `vault-mesh`).** Secrets registration/resolution via the local mesh daemon; the use-without-seeing secret-brokerage shape is TBD; round-8 adds distribution across nodes (replication ~3). |
| `rollup-ccd` | ccd -> rollup | **NEW round-6 (requirements-only).** CCD consumes rollup for its plugin/prompt assembly — fragments + slots rolled up into specialized plugins for specialized agents, generated on demand. |
| `repo-vfs` | repo -> vfs | **NEW round-6 (requirements-only).** Repo sits on top of the VFS: repo state materialized through VFS storage; push/sync between VFS trees and git/GitHub repos. |
| `repo-environments` | repo <-> environments | **NEW round-6 (requirements-only).** Branches/worktrees attachable to environments; deploy-into-environment activates the pipeline (the environments-branches relationship is flagged "strange" for public-website deployments). |

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

**Rounds 4–5 strengthen the seam:** with single-port locality locked, every
service registers/resolves against its **local mesh daemon on `:3649`** and is
unaware of any other service's port — mesh does all relaying (see
`components/mesh.md` concerns 8–9). The seam's mechanics are unchanged; its
exclusivity is now an operator-locked rule rather than a design preference.

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
CCD's endpoints, Org's service map, and the new
`vfs`/`kg`/`projects`/`stack`/`secrets`/`repo` services. The
mesh-design-synthesis
already anticipated the observability-plane change (OQ-style, written pre-merge
against gateway). This is a cross-cutting refactor
to flag for the operator, not to specify in this pass.

## Standing cross-cutting principles (NEW section, round-6)

- **PROVENANCE IS FIRST-ORDER, from the very beginning.** Operator, verbatim:
  "Traces on executions in our runtime stack are first-order. I want
  provenance from the very beginning. I'm a long-term data engineer in
  healthcare — I want to see everything that led to the current state of our
  database, every time data gets touched by a handler."
  Healthcare-data-engineer-grade provenance is a design input to EVERY
  component from day one, never a retrofit: **traces on every execution** in
  the runtime stack, and **a trace every time data gets touched by a
  handler**. Concretely wired in already: the shared handler/execution
  engine's causal-chain tracking (below), `kg.md` (graph-data provenance via
  the shared engine), and `stack.md` (handler-touch traces); every future
  design pass must treat provenance as a first-order requirement.
  **SCOPED round-7 (2026-07-19): provenance is configured
  per-project/per-database, and VDB is its primary home** — operator:
  "probably a VDB thing. Also a VFS thing, but I typically don't care about
  provenance in the file system — usually only in the database and the
  stack pattern." VFS-level provenance is a **lighter** design requirement
  (see `vfs.md`); the full-strength principle lives in the
  database/stack-pattern plane (`stack.md`/`db.md`).
- **Pairwise data contracts are the operating model** (round-6 affirmation —
  see the note at the top of the contract graph): contract changes trigger
  design sessions analyzing ripple effects; this is how all future updates
  land.

## Shared libraries (NEW section, rounds 4–5)

Cross-component libraries that are consumed as Cargo dependencies, not
contract edges. (The earlier per-file candidates — `substrate-mesh-client`,
`substrate-api-client`, `substrate-tailscale` — stay documented where they
were flagged, in `mesh.md`/`ccd.md`; this section exists for the first
operator-locked shared lib.)

- **The handler/execution engine (name TBD — LOCKED rounds 4–5).** ONE shared
  library implementing the trigger/handler paradigm, with **adapters** for
  `stack`-tables and `kg`-nodes — "I have the required engine in each
  location and adapters to deploy this thing in that location." The
  kg/stack <-> engine relationship is an **internal library dependency, NOT a
  contract edge** (no `contracts/` stub). Distributed trigger execution
  coordinates via mesh's internal `locks` lib. **Guardrails are designed-in
  from day one:**
  - **causal chain tracking** per handler invocation (what change caused
    what) — round-6: this is one concrete instance of the FIRST-ORDER
    provenance principle (see the standing cross-cutting principles above);
    every handler touch of data is traced from the very beginning;
  - **LOOP detection with a loop-depth threshold** — not a naive
    cascade-depth cutoff: "It's hard to set an arbitrary [cascade] depth
    [with tons of services cascading into each other]. It has to track loops
    — we need a loop depth [for updates recurring on the same table]";
  - **an escalation hook** — exceeding the loop-depth threshold triggers an
    **agent investigation via `ccd`** ("at a certain loop depth, that can
    trigger a handler for a Claude Code invocation to look at it and be
    like: what happened?").
  - **Naming guardrail:** this engine and its propagation must NOT be called
    "ripples" anywhere — that word is RESERVED (see the future section).
  - **Trigger vocabulary (LOCKED round-8, 2026-07-19) — shared with mesh's
    queues; supersedes any simpler queue→handler wording:** queues hold
    **typed EVENTS** (a standardized event-type struct in `types` — see
    `components/types.md`); **TRIGGERS** are **one-to-one from a queue to a
    handler**, with **many triggers per queue** — a trigger **FILTERS** (by
    event type, and by payload content per event type) and **ASSEMBLES the
    handler's payload** ("it's not the event which dictates the payload
    going into the handler, it's the trigger"), optionally calling the
    **rollup** system during assembly; **HANDLERS** receive the assembled
    payload. **Each trigger declares whether pulling through it requires
    the event-ID semaphore** (per-trigger choice — supersedes round-7's
    blanket rule; the `locks`+queues composition stands). See
    `components/mesh.md` concern 14.

## Future / placeholder concepts (named only — NO component files)

Operator-named coming-later scope, recorded as one-liners so nothing squats on
the names. These are layer-six territory alongside `org` and CCD's strategic
layer; none is decomposed, designed, or given a `components/` file this pass:

- **`agents`** — a future generalization layered ON TOP of CCD for non-Claude-
  Code agent types (which CAN choose models and may use local inference); CCD
  itself stays top-level and is NOT merged into this umbrella.
- **`artifacts`** (round-3 second batch) — "once we migrate to projects, no
  more files — artifacts": typed, schema'd, interactable files (e.g. an
  HFT-strategy artifact evaluated against asset artifacts via a versioned
  controller against a simulator); implies a script execution engine; built out
  incrementally; "might have to be its own standalone tool, really boring and
  deterministic." `projects` depends on it (see `components/projects.md`).
- **OpenRouter-management** — a service managing OpenRouter API keys: per-key
  creation with per-key budgets, dashboard-managed.
- **`spend`** (RENAMED from `finance`, rounds 4–5) — cost tracking,
  per-project (eventually attaching to `projects`' per-project metadata).
  Pull-shaped: **spend QUERIES its spend sources** — CCD's own usage database
  (sessions/tokens/limits; see `components/ccd.md`) first among them — rather
  than sources pushing to spend.
- **AUI / interfaces** — a voice interface over the whole mesh.
- ~~**`repo`** (OPEN FORK, rounds 4–5)~~ — **RESOLVED + PROMOTED round-6:**
  repo is a real build-now component (its own crate on top of the VFS;
  git-in-the-VFS rejected; both stay boring) — no longer a placeholder; see
  `components/repo.md`. This entry is kept as a tombstone so the name's
  history stays traceable.
- **`ripples` — RESERVED TERM, out of v1 scope, do not design or discuss in
  v1.** The word "ripples" belongs to a future KG micro-agent system: LLM
  micro-agents that respond to other micro-agents, rippling through a
  knowledge graph — a single LLM completion that can pull context from
  surrounding KG nodes and call tools including other ripples; one schema per
  graph type; "the LLM-and-knowledge-graph equivalent of the backend web
  stack." Operator: "I'm not ready to discuss it yet in the context of this
  project. It's not version one." **The word must NOT be used for the shared
  handler/execution engine or any other propagation mechanism** (see the
  shared-libraries section).

## Carry-forward open questions for later steps

- OQ-3 (mesh-design-synthesis): final shape of the shared `NodeInfo` /
  `NodeCapabilities` in `types` — a step-3 (Contract Harmonizer) decision, since
  it's the shared-types reconciliation with the pre-existing `types` crate.
- ~~CCD siting~~: RESOLVED (component-design pass) — CCD lives inside the
  Substrate workspace as a first-class member; see `ccd.md`.
- ~~Mesh internal layering~~: **RESOLVED (rounds 4–5)** — yes, mesh decomposes
  internally into layered libs ("boring layers on top of boring layers");
  utilities (service registry, replicated KV, `locks`, `cron`, S3 adapter)
  are internal libs of mesh, never standalone crates/services. See `mesh.md`.
- **`locks` partition semantics (NEW rounds 4–5, needs real care):** the
  slug+UUID identity sketch handles partition twins, but the full
  partition/merge semantics are OPEN — "it has to generalize to be reliable
  in an infinite set of circumstances." Round-6 locks one requirement within
  them: a specific, **catchable error type for "lock threshold exceeded
  because two network partitions merged," handled per-application** (CAP
  honesty blessed — "we cannot violate the laws of physics"). See `mesh.md`
  concern 10.
- ~~**The stack-vs-db boundary (round-6) — THE operator's MOST-NEBULOUS open
  question; do NOT force it.**~~ — **RESOLVED round-8 (2026-07-19): the
  VDB/db/KG layering is LOCKED.** Operator, verbatim: "VDB becomes the
  daemon which tracks the execution, and it takes advantage of the db crate
  to actually run the actions against specific databases — we extend db as
  necessary to support VDB. And KG we build on top of VDB." History: stack
  is "a pattern" (database-driven triggers/handlers calling third parties);
  round-6 posed fold-stack-into-db vs. keep-db-boring and coined **VDB** (a
  virtualized-database service using `db` under the hood, databases treated
  like services under mesh's restart/upgrade protocol, with the confirmed
  copy/verify/switch + let-edge-functions-finish migration mechanics);
  round-7 ELEVATED VDB to the deploy-anywhere implementation of the stack
  pattern (one abstract runtime; local SQLite-stack-daemon / Supabase / AWS
  RDS+Lambda adapters; `db`'s Capabilities-gated drivers as the seed) with
  two attached open questions. **Round-8 closes all of it with the
  decomposition: VDB is the DAEMON that tracks/executes the stack pattern;
  `db` is the crate VDB USES to run actions against specific databases
  (extended as needed to support VDB); KG is BUILT ON TOP of VDB (the
  KG-on-VDB question answers YES — see `kg.md`); VFS sits BELOW VDB (the
  SQLite files live in the VFS). Locked layer order: VFS < VDB < KG.**
  Neither round-6 option won outright — db is not swallowed by stack, and
  db does not stay frozen: it stays the boring action-runner under the VDB
  daemon. Still open nearby: the Postgres source / Docker tension and the
  SQLite-sufficiency gate (next bullet). Full supersession note in
  `stack.md`; see also `db.md`.
- **`stack` SQLite→Postgres upgrade (rounds 4–5; PARTIALLY RESOLVED
  round-6; GATED round-7):** the migration *mechanics* are confirmed
  (copy/verify/switch under a lock; let running edge functions finish, swap
  underneath, write back, resume). Still OPEN: where Postgres comes from —
  the operator's "one Postgres Docker machine" line sits **unreconciled
  against the hard no-Docker rule** — and his own counter-question "why ever
  upgrade past SQLite if the full stack works on SQLite." **Round-7: the
  operator will not decide the local-Postgres question until a REQUIRED
  daemon-level SQLite-sufficiency gap analysis** shows whether SQLite + the
  stack daemon + mesh (cron, pub/sub) functionally covers everything
  Postgres would provide (pg_cron → mesh cron; LISTEN/NOTIFY → mesh pub/sub;
  procedural triggers → daemon-level Deno/SQL handlers; etc.); AUI is
  delivering a first-pass analysis conversationally, but the definitive
  version belongs in stack/VDB's full component design. See `stack.md`.
- **`environments` vs `cicd` hierarchy (NEW rounds 4–5):** sibling vs. child
  OPEN; sibling-with-dependency is the working assumption. ~~And bandit-based
  feature load-balancing placement~~ — **RESOLVED round-6: bandits are NOT a
  mesh concern; they are complex in-app behavior** inside a public-facing
  website deployed via projects/environments. Standing note recorded:
  environments must be boring/predictable/stable but not overly rigid; the
  environments-branches relationship is flagged "strange" for public-website
  deployments. See `environments.md` / `cicd.md`.
- ~~**`repo` crate vs git-in-the-VFS (OPEN FORK, rounds 4–5)**~~ —
  **RESOLVED round-6: repo IS its own crate, built on top of the VFS**;
  git-in-the-VFS rejected; both VFS and repo stay boring ("the only
  not-boring thing about repo is that it's built on top of a virtual file
  system"). Branches, worktrees, and branches-attachable-to-environments
  now live in `components/repo.md`. **Prior art still applies:** the
  operator's `.mind` workspace schema already links worktrees + Claude Code
  threads — reuse it when repo is designed.
- **Versioning / update protocol (rounds 4–5; NARROWED round-6):** the
  APPROACH is decided — pairwise service dependencies with minimal-restart
  rolling updates ("each individual service only gets restarted if it needs
  to get restarted"); commit-as-release-set was proposed and NOT adopted.
  Round-6 supplies the per-service mechanics: port-handoff updates + the
  two-way graceful-restart protocol (`mesh.md` concerns 12–13). Still OPEN:
  the concrete update protocol between nodes running mixed versions, and the
  exact restart-priority ladder ("latitude granted, discuss"). See `mesh.md`
  concerns 6, 12–13.
- **`rollup` naming + syntax (NEW round-6; insert types LOCKED round-7):**
  the crate name is OPEN (rollup / plugins / other), as is the
  fragment-reference/slot syntax; the operator has built ~two prior versions
  in other projects — a prior-art scan is underway separately and should
  seed the design pass. Round-7 locks the **raw-vs-reference insert-type
  axis** (extending the prior art's prompt/slot/file/prompt-file markers)
  and the **no-secrets-raw-in-LLM-bound-content rule**. See
  `components/rollup.md` / `components/secrets.md` (was `vault.md`;
  renamed round-8).
- **GC rolled into VFS vs. called-as-tool (NEW round-3):** GC is now the
  per-node tool VFS calls; "might need to get rolled up into the VFS." Open —
  see `gc.md` / `vfs.md`.
- **KG distributed-consistency model (NEW round-3 second batch):** a graph of
  interconnected nodes is "the superset" of the registry's naive
  timestamp-wins KV — its merge/consistency design is open. See `kg.md` /
  `contracts/kg-mesh.md`.
- **Surface-schema language shape (NEW round-3):** the `types` schema language
  for the boring surface schema is requirements-only; a step-3 / Contract
  Harmonizer design concern. See `contracts/surface-schema.md`.
- ~~`inference -> llm` rename~~: RESOLVED this round — name stays `inference`; see notes above.
- Org convergence: a separate prototype ("autonomous org", `~/code/career_crafter`
  branch `Lazarus`) may become Org instead of a from-scratch build — the operator
  flagged this as a blocking question. Out of scope to resolve here; noted so Org's
  placeholder isn't mistaken for a greenlit greenfield build.
