# Mind OS — Wave-2 Fine-Grained Decompose (module inventory · contract pairs · batch plan)

**Status:** AUTHORITATIVE module inventory for the second design wave
(authorized round-9, INTENT #107: "super fine-grained... including all the
layers within the mesh, the services within inference, and the data contracts
between all of them... down to the lib level"). This file SUPERSEDES
`overview.md`'s component tree as the module inventory of record;
`overview.md` remains the record of the contract graph's history, the wiring
seam, locks, and open questions. Per the operator's explicit instruction,
**this pass identifies contract pairs but authors NO contract content** —
per-module design passes propose contracts, then a per-pair contract round,
then harmonization.

**Refinement decompose, not from-scratch:** built on the round-9 scaffold
(commit `b2151e3`), the 25+ `components/*.md` design files, and INTENT items
44–107 (the governing decisions; where scaffold text conflicts with INTENT,
INTENT wins).

---

## 1. The OS layering

Derived from INTENT #44 ("built up correctly from the bottom up, layer by
layer, like an operating system"), #45 (mesh IS the operating system), #54–58
(mesh utilities are internal libs, boring layers on boring layers), #96
(VFS < VDB < KG), and #49/#51 (layer six = org/interfaces territory). A module
may only depend on its own layer's peers and lower layers; every cross-app
call goes through the local mesh daemon (single-port locality, `:3649`) —
in-process linking across app boundaries is forbidden (INTENT #29), shared
libs excepted (compiled in, no runtime version tracking — INTENT #45).

| Layer | Name | What lives here |
|-------|------|-----------------|
| **L0** | Foundation | Pure vocabulary + transport visibility: `types`, `tailscale-query`. Zero runtime, zero mesh dependency. (Tailscale itself — the tailnet — is the external transport under everything.) |
| **L1** | Mesh kernel | The daemon shell every service talks to: `mesh-core` (bin/mesh, port 3649, single-port locality, stickiness, zombie-killing, addressing classes, CLI), `pubsub-relay`, `network-topology`, and the thin `mesh-client` boot lib. |
| **L2** | Replicated state + OS services | Mesh's internal utility libs, layered boringly: `replicated-kv`, `service-registry` (THE wiring seam), `locks`, `queues` (events/triggers/handlers), `cron`, `supervision` (versions/boot-order/restart ladder), `completion-router`, `dashboard-serving` — plus the `aws` crate (the cloud adapter leg the replication plane and storage plane consume). |
| **L3** | Storage plane | `vfs` (boring distributed flat file system), `gc` (per-node enforcement tool), `secrets` (distributed, replication ~3). |
| **L4** | Data & execution plane | `db` (action-runner crate), `vdb` (the stack-pattern daemon), the shared `execution-engine` lib, `kg` (on VDB), `rollup`. Locked order within the plane: VFS < VDB < KG. |
| **L5** | Service / application plane | `inference` + its eight internal libs, `repo` (on VFS), `ccd`, `dashboard` (the Svelte app). |
| **L6** | Organization plane (STUB TRACK) | Design stubs + anticipated contracts only, explicit "not implementing now": `org`, `projects`, `artifacts`, `environments`, `cicd`, `spend`, `agents`, `aui`, `openrouter-mgmt`. |

Notes on two deliberate placements:

- **`aws` at L2, not L6.** Round-9 made `aws` the aggregation point for ALL
  AWS adapters; mesh's replication plane (S3 distribution), VFS overflow,
  VDB's cloud target, and secrets' push adapter all consume it. Since L2/L3
  modules consume it, it must sit at the adapter plane, not above it. Its
  registration with mesh rides the universal seam like everyone else's (the
  seam is not a layering violation — every module registers).
- **`gc` at L3 beside `vfs`.** GC is dual-role (embedded lib inside inference
  + per-node daemon called by VFS); the open rolled-into-vfs-vs-called-as-tool
  question (round-3, still open) is decided in their co-batched design pass.

---

## 2. Module inventory

**Kind vocabulary:** `app-crate` = a real app (CLI tool or daemon; a
top-level workspace crate, never imported cross-app) · `internal-lib(X)` = a
library nested under app X (may be a workspace lib crate for convenience but
only ever consumed through X) · `shared-lib` = a Cargo library any crate may
link (compiled in) · `app-frontend` = the Svelte UI app · `stub` = layer-6
design-stub track.

**Track:** FULL = full component design this wave · STUB = design notes +
anticipated contracts only, "not implementing now."

**Model tier:** design-model suggestion per the round-9 policy (Opus default;
Fable ONLY for the genuinely hardest; if a Design Mesh is used, Fable for one
step only).

### L0 — Foundation

| # | Module | Kind | Cx | Model | Track | Charter |
|---|--------|------|----|-------|-------|---------|
| 1 | `types` | shared-lib (`lib/types`) | M | Opus | FULL | The zero-dependency shared-contract-types foundation every crate imports: IDs, enums, request/response/event structs, error taxonomy, Promise primitive — pure vocabulary, no I/O, no behavior. UNLOCKED (round-3). Wave-2 additions already locked: the boring-surface-schema domain module, the WS pub/sub envelope module, and the standardized typed-EVENT struct (round-8 vocabulary). Every contract schema is expressed in terms of this crate; the harmonizer reconciles authored contracts back into it. |
| 2 | `tailscale-query` | shared-lib (`lib/tailscale`, promoted crate) | L | Opus | FULL | The standardized, extensible Tailscale-status query surface: a synchronous `TailscaleQuery` trait, one typed method per subcommand (`status()` now; netcheck/ping/whois as tools-added-as-we-go), Real + Fake impls, public typed structs decoupled from private serde structs so Tailscale JSON churn never leaks. Consumed by `network-topology` and `completion-router`. Approach-sketched already; lowest-risk module in the wave. |

### L1 — Mesh kernel

| # | Module | Kind | Cx | Model | Track | Charter |
|---|--------|------|----|-------|-------|---------|
| 3 | `mesh-core` | app-crate (`bin/mesh` + `lib/mesh` root) | H | Opus | FULL | The daemon shell of the operating system: binds LOCKED port `:3649`, enforces single-port locality (every service talks ONLY to its local mesh daemon; mesh does all relaying — inter-service AND inter-node), implements the two addressing classes ("service X anywhere" vs "service X on node N"), stickiness (system-level resurrection via launchd/systemd), port-squatter killing on restart, and rigorous zombie-killing. Owns the noun-verb CLI (`mesh service open <slug>`, `mesh net status`, ...). Composes all L1/L2 internal libs into the one process; consumes `aws` (via `aws-mesh`) for the cloud legs of the replication plane. Not itself the hard algorithmic problem — the discipline problem: boring layers composed correctly. |
| 4 | `pubsub-relay` | internal-lib(mesh) | M | Opus | FULL | The standard WebSocket pub/sub protocol (LOCKED rounds 4–5): typed envelope structs (defined in `types`), topic-based publish/subscribe, per-completion-ID subscription, and mesh-relayed delivery "where it needs to go" — across services and across nodes. All existing WS surfaces (`network-events`, `dashboard-feed`, per-node event streams) converge onto this protocol at harmonization. The wire shape is undesigned; this module designs it. |
| 5 | `network-topology` | internal-lib(mesh) | L–M | Opus | FULL | Diffs consecutive tailscale-status snapshots into transition events (peer joined/left/offline) published over `network-events`; the distinguishing hard part is detecting THIS device's own connectivity loss (`self_offline`/`self_online`, peers-unknown-not-offline semantics); snapshot-on-connect then deltas. Approach-sketched already. |
| 6 | `mesh-client` | shared-lib (`lib/mesh-client`) | L | Opus | FULL | The ~one-file boot library every service compiles in: `register(slug, endpoint)` / `resolve(slug)` against the LOCAL mesh daemon on `:3649`, plus the client half of the universal service protocols (restart-protocol participation, surface-schema publication, heartbeat/lease renewal). Exists so services don't link the whole mesh tree; the cheap handle to the wiring seam. Final crate-vs-types-module boundary confirmed at skeleton time. |

### L2 — Replicated state + OS services

| # | Module | Kind | Cx | Model | Track | Charter |
|---|--------|------|----|-------|-------|---------|
| 7 | `replicated-kv` | internal-lib(mesh) | H | **Fable** | FULL | The replicated-state primitive everything else rides: an eventually-consistent, LWW-register (timestamp-wins, INTENT #32) key-value store replicated across all mesh daemons via periodic full-state anti-entropy, offline nodes syncing bidirectionally on reconnect. Operator latitude: one primitive vs several ("if it makes sense to do them the same, do them the same") — hard constraints are boringness and buried-inside-mesh. Serves: service-registry entries, lock state, queue metadata, version/boot-order records; its cloud replication leg reaches S3/AWS through `aws`. The consistency substrate of the whole OS — genuinely hardest tier. |
| 8 | `service-registry` | internal-lib(mesh) | M–H | Opus | FULL | THE wiring seam: distributed slug -> endpoint registry (`{scheme, host, port, health_path}`), leases + heartbeat renewal + tombstones, riding `replicated-kv`; fleet-vs-singleton addressing (mesh registers the `inference` slug at its own front door; singletons self-register); resolve-from-any-device. Approach-sketched (LWW/lease/anti-entropy design exists); wave-2 job is re-grounding it ON the extracted `replicated-kv` substrate and pinning wire formats. |
| 9 | `locks` | internal-lib(mesh) | H | **Fable** | FULL | Distributed semaphores as a first-order mesh lib riding the KV store: acquisition knowledge distributes to ALL reachable nodes BEFORE the client is confirmed holding; slug+UUID identity for partition twins; bidirectional reconnect sync; the REQUIRED catchable error type for "lock threshold exceeded because two partitions merged" (CAP honesty blessed, handled per-application). Full partition/merge semantics are OPEN and must "generalize to be reliable in an infinite set of circumstances" — the wave's single most safety-critical open design. Composes with queues (event-ID semaphores) and the execution-engine's distributed trigger coordination. |
| 10 | `queues` | internal-lib(mesh) | H | Opus | FULL | SQS-modeled queues + dead-letter queues (a DLQ is just a queue), carrying the LOCKED vocabulary: typed EVENTS (struct in `types`) → TRIGGERS (declarative DATA — filter expression + payload-assembly template, registered as data, NEVER code; 1:1 queue→handler, many per queue; assembly may call rollup; per-trigger event-ID-semaphore choice) → HANDLERS (receive the assembled payload; the only place code lives). ~Exactly-once over at-least-once via `locks`; DLQ escalation hooks into ccd investigation; someday deployable onto real SQS through `aws`. "The entire contract of the system is basically built off the queueing mechanism." Semantics heavily pre-locked (rounds 7–9), so Opus; escalate the trigger-language design to a Fable step if it resists. |
| 11 | `cron` | internal-lib(mesh) | L | Opus | FULL | Scheduled tasks in two flavors matching the addressing classes: "run on node N" and "run anywhere." Also the pg_cron-equivalent in the SQLite-sufficiency story (a cron handler operating on stack databases). Boring by intent. |
| 12 | `supervision` | internal-lib(mesh) | H | Opus | FULL | The OS lifecycle layer: per-node service-version tracking, pairwise inter-service version requirements ("we just track the requirements and the boot order"), dependency-derived boot ordering that mesh EXECUTES (mesh starts and supervises local services — answered 3rd ask), observable interruptibility states, port-handoff updates (new port → registry flip → old down), the two-way graceful-restart protocol at the LOCKED 4-level ladder (wait-for-idle / finish-and-relinquish / ~10s-save / kill), and minimal-restart rolling updates. OPEN inside it: the concrete update protocol between nodes running mixed versions. Not a package manager/installer. |
| 13 | `completion-router` | internal-lib(mesh) | M | Opus | FULL | The transparent completion data plane across the inference fleet: NodeRegistry (tag-based fleet discovery + state/inventory polling), ModelAffinityBalancer, a `forward()` carrying status+headers+streaming body, WS relay for streams, `?node=`/header pinning. Implementation-ready design frozen in the `mesh-design-synthesis.md` report (harness workspace `substrate-v2/reports/`, not this repo); wave-2 job is integrating it with the L1/L2 module boundaries (its NodeRegistry ≠ service-registry ≠ network-topology). |
| 14 | `dashboard-serving` | internal-lib(mesh) | M | Opus | FULL | The absorbed observability plane: serves `ui/dashboard/dist/` from one origin, per-node subscription reconciliation (one inference-WS + one gc-WS + one ccd-WS per node, no leaks/doubles), the topic-filtered `GET /events` browser fan-out, presentation rollups (`/api/nodes`, `/api/mesh/stats`), same-origin REST proxy, and the surface-schema pipeline (discover services via registry → fetch each service's published surface schema → feed the dashboard). Aggregation must never become routing. |
| 15 | `aws` | app-crate | M | Opus | FULL | The AWS virtualization layer — the aggregation point for ALL AWS adapters: S3 overflow (cold-storage classes, client-side encryption, keys in `secrets`), RDS+Lambda (VDB's cloud target), AWS Secrets Manager push (DESIGN-ONLY in v1), SQS someday. A WebSocket interface shaped by what Mind OS services need, explicitly NOT a boto3/Terraform replacement. Containers, if ever, live only here (AWS side) — never locally. |

### L3 — Storage plane

| # | Module | Kind | Cx | Model | Track | Charter |
|---|--------|------|----|-------|-------|---------|
| 16 | `vfs` | app-crate | H | Opus | FULL | The boring distributed flat file system — the mesh-wide storage substrate. Per-directory configurable policies (max size; FIFO / LRU-updated / LRU-accessed eviction), per-file replication factor, RAID-ish multi-drive warm/cold nodes (the Pi with external drives), inference-style self-tracked performance (uptime, per-node read/write latency), access-can-migrate semantics (reading remote data can pull it local for reuse), S3 overflow through `aws`. Calls the node-local `gc` tool for per-device enforcement. Sits BELOW VDB (VDB's SQLite files live here). Lighter provenance requirement than the database plane. High complexity but boring-by-design; distribution/consistency rides mesh, which is why this stays Opus. |
| 17 | `gc` | app-crate (`lib/gc` + `bin/gc`; dual-role) | L–M | Opus | FULL | The filesystem garbage collector: managed dirs + entries, TTL/size budgets, lock-with-expiry (never pin forever), pluggable Reclaimer (Delete real; Migrate an intentional stub), `.gc` config files in managed directories, ONE centralized per-node store updatable over API/WS. Two forms of one crate: embedded lib inside inference (models/cache/engine) + the per-node `:8430` daemon VFS calls. Carried-forward core is implementation-ready; the wave designs only the round-3 deltas AND resolves rolled-into-vfs vs called-as-tool (co-batched with `vfs`). |
| 18 | `secrets` | app-crate | M–H | Opus | FULL | The secrets service (renamed from `vault`): agents/LLMs can NEVER read a secret but CAN use one in a context — use-without-seeing enforced structurally at the access-pattern level; raw + ID addressing with `llm_safe=true` defaulting for LLM-feeding services (raw requests fail or degrade to ID-plus-warning; NO secret ever reaches an LLM — invariant). Distributed across nodes (replication ~3) via mesh. Adapters: db's existing Supabase-Vault module IS the Supabase adapter (leverage, don't rebuild); the local-mesh-database adapter is the one to BUILD; the AWS Secrets Manager adapter is DESIGN-ONLY (lives in `aws`). Interfaces: VDB, db, projects, environments, repo (GH-Actions secret injection on push = repo's required v1 capability). |

### L4 — Data & execution plane

| # | Module | Kind | Cx | Model | Track | Charter |
|---|--------|------|----|-------|-------|---------|
| 19 | `db` | app-crate (existing `lib/db` + `bin/db`) | M | Opus | FULL | The boring database action-runner: migrations, edge functions, query, outbox, noun-verb CLI, Capabilities-gated drivers (supabase-cloud / supabase-local / sqlite — the seed of VDB's adapter matrix). Round-8 position LOCKED: `db` is the crate VDB uses to run actions against specific databases, EXTENDED as necessary to support VDB — it stays boring under the VDB daemon. Query/virtualization layer is in-scope direction. Reconciles its secret handling toward consuming `secrets` (its Supabase-Vault module becomes secrets' Supabase adapter). Never imported as a lib by other apps (INTENT #29) — called over WS/CLI. |
| 20 | `vdb` | app-crate (component file currently `stack.md`) | H | **Fable** | FULL | The daemon that TRACKS and EXECUTES the stack pattern (the operator's database-centric paradigm: tables as intermediate structures, SQL + Deno/TS handlers on row/table changes) — deploy-anywhere with three adapter targets: local (the SQLite-file-in-VFS daemon; SQLite-locally LOCKED, no local Postgres ever, no Docker ever), Supabase, AWS RDS+Lambda (via `aws`). Uses `db` to run actions; hosts the execution-engine's stack-tables adapter; treats databases like services under mesh's restart/upgrade protocol; copy/verify/switch-under-a-lock promotion mechanics (local→cloud only); PRIMARY home of first-order provenance (per-project/per-database, healthcare-grade: every handler touch traced). Required design input: the SQLite-sufficiency analysis (pg_cron → mesh cron; LISTEN/NOTIFY → mesh pub/sub; procedural triggers → handlers). Naming note: module named `vdb` per the round-8 lock; the `stack` name survives as the PATTERN; crate naming (`bin/vdb` vs `bin/stack`) + contract-stub renames (`stack-mesh`/`stack-vfs`) are the design pass's to settle. |
| 21 | `execution-engine` | shared-lib (name TBD — NEVER "ripples") | H | **Fable** | FULL | ONE shared library implementing the trigger/handler paradigm with adapters for stack-tables and kg-nodes — an internal library dependency, NOT a contract edge. Designed-in guardrails from day one: causal-chain tracking per handler invocation (the concrete instance of first-order provenance), LOOP detection with a loop-depth threshold (not naive cascade depth), and the escalation hook (loop-depth exceeded → ccd agent investigation). Binds the LOCKED declarative-trigger rule on both adapters; Deno is the confirmed TS-handler runtime; distributed trigger execution coordinates via mesh's `locks`. Shares the trigger/event vocabulary with `queues` — the two must be designed against one trigger data model. |
| 22 | `kg` | app-crate | H | **Fable** | FULL | The Knowledge Graph service: a distributed graph structure across the entire mesh, BUILT ON VDB (VFS < VDB < KG, locked). Graph lifecycle with schema-LOCKED nodes+edges (failed validation pushes back to the caller); versioned graph TEMPLATES (research / Obsidian-markdown / .mind-axiomatic from the start); nodes point at VFS files with existence validation; a GLOBAL registry of ALL graphs (cross-project reuse, schema referencing); each graph links to a project+environment which routes storage (local→SQLite, cloud→promoted — don't over-constrain); cross-boundary eventual consistency to AWS (via mesh→aws); trigger/handler support via the execution-engine's kg-nodes adapter. THE open hard problem: the distributed graph-merge consistency model — "the superset" of LWW KV. |
| 23 | `rollup` | app-crate | M | Opus | FULL | The prompt/plugin rollup system: fragments referencing fragments via a syntax, slots taking variables at reference time; plugins generated on demand ("specialized plugins for specialized agents, no symlinks or file-copying"); the generalization ladder string→file→directory rollup; LOCKED insert types raw vs reference (extending the harness prior art's `{{prompt:}}`/`{{slot:}}`/`{{file:}}` markers); NEVER resolves a secrets raw reference in LLM-bound content. Consumed by CCD (plugin assembly), by trigger payload-assembly (declarative references), and eventually by projects ("tasks are rolled up"). Fragment-reference syntax OPEN — prior-art scan seeds the design. |

### L5 — Service / application plane

| # | Module | Kind | Cx | Model | Track | Charter |
|---|--------|------|----|-------|-------|---------|
| 24 | `inference` | app-crate (`bin/inference` + `lib/inference`) | M | Opus | FULL | The per-machine, modality-agnostic generation runtime (name LOCKED): composes its eight internal libs via the private `InferenceService::start` wiring; exposes exactly ONE external contract (the `/v1/` REST+WS surface); owns crash recovery, pause/resume, the lossy broadcast event bus, mesh registration, and db-backed database init on fresh nodes (with graceful standalone degradation). Wave-2 job: re-ground the crate contract against the new mesh protocols (restart ladder, surface schema, pub/sub envelopes). |
| 25 | `store` | internal-lib(inference) | M | Opus | FULL | The per-node SQLite system of record (completions, collections, models, results, benchmarks, kv_cache) + the StoreObserver seam; self-migrating schema (tension with db-inference-init flagged, not silently resolved). Distinct from `db` (control plane) and `vdb` (stack daemon) — deliberately so. |
| 26 | `engine` | internal-lib(inference) | M–H | Opus | FULL | The llama-server process manager + `InferenceBackend` seam: auto-provisions the exact backend build a model needs (no pre-installed llama.cpp — INTENT #4), slot lifecycle, submit/drain, model swap, KV-cache save/restore hooks. |
| 27 | `scheduler` | internal-lib(inference) | H | Opus | FULL | The admission / swap / preemption / recovery loop: priority queueing, model-swap decisions against telemetry's SystemState + the kernel's effective-concurrency reads, benchmark priority-0 exclusivity, crash recovery. Interacts with the multidimensional kernel (see `benchmark`) — co-design in the same batch; escalate to a Fable step if the kernel coupling proves hard. |
| 28 | `models` | internal-lib(inference) | M | Opus | FULL | Model registry sync + resumable download pipeline + eviction, using the gc make_room → write → register → lock pattern. |
| 29 | `cache` | internal-lib(inference) | L | Opus | FULL | The disk-backed KV/prefix cache keyed by (model_id, prompt_hash), gc-managed. |
| 30 | `telemetry` | internal-lib(inference) | M–H | Opus | FULL | System sampling (memory/CPU/GPU pressure; Apple-Silicon GPU estimates visually honest — tilde + tooltip) + throughput estimation + the per-machine per-model performance KERNEL (quadratic spline inside range, linear outside) — now multidimensional per INTENT #12. Serves SystemState to scheduler/api and kernel curves to the dashboard. |
| 31 | `benchmark` | internal-lib(inference) | H | **Fable** | FULL | The idle-gated priority-0 sweep orchestrator, REDEFINED by INTENT #12: stress-testing and throughput benchmarking are ONE action; the kernel must approximate performance across the full multi-axis space (memory/CPU/GPU pressure × parallel completion count × output length 1k/10k/100k); test selection is ADAPTIVE — max-information-gain / lowest-kernel-confidence next-test selection, never a manual grid. Active-learning design — genuinely hard; the kernel-confidence edge to scheduler/telemetry is its output surface. |
| 32 | `api` | internal-lib(inference) | M | Opus | FULL | The Axum `/v1` REST+WS surface — the node's single external contract — dispatching into scheduler/store/telemetry/benchmark; bridges the event bus to the WS stream mesh subscribes to; publishes the node's surface schema. |
| 33 | `repo` | app-crate | M–H | Opus | FULL | Git/GitHub on top of the VFS (its only non-boring trait): push/sync between VFS trees and remotes, branches, worktrees ("I like worktrees. I like workspaces." — reuse the .mind worktree+thread schema), branches attachable to environments, FULL GitHub-Actions workflow management in v1 (workflows are directory changes; rollup excluded), and THE required v1 capability: secrets injection on push (repo names the secret↔workflow linkage; secrets' GH adapter pushes values). |
| 34 | `ccd` | app-crate | H | Opus | FULL | The Cloud Code Daemon: spawns/tracks/routes cloud-code agent processes (stable handle namespace), and plays the Claude-Code-usage-limits game with rich DECLARATIVE budget/priority strategies (weekly/session/per-model limits; percentile-guarded burn-down; caller priorities honored) — top-level forever, the future `agents` layer generalizes on top, never absorbs it. Owns its own usage database (sessions/tokens/limits; threads linkable to projects + optionally environments; queried by spend, pull-shaped). Consumes rollup for on-demand plugin assembly. Claude Code calls are NOT routed to local inference (can't choose models); `llm-calls` is metering-shaped. Receives DLQ/loop-depth escalation investigations. |
| 35 | `dashboard` | app-frontend (`ui/dashboard`) | M–H | Opus | FULL | The Svelte/Vite (no SvelteKit) frontend served BY mesh: schema-driven rendering — every service's panel rendered from its published boring surface schema, never hand-built per-service; per-node dimension throughout; kernel-curve widget (current model default, expandable multi-model overlay, live results dropped on the curve); honest-estimate markers; and the first-order agent interface: every interactive element carries a stable semantic id so agents drive the SAME markup humans see. Discovers itself via mesh — no hardcoded ports. Access control explicitly deprioritized. |

### L6 — Organization plane (STUB TRACK: design notes + anticipated contracts, explicitly "not implementing now")

| # | Module | Kind | Cx | Model | Track | Charter |
|---|--------|------|----|-------|-------|---------|
| 36 | `org` | stub | (H later) | Opus | STUB | The autonomous-organizations foundation: agents, AI pipelines (and, someday, the RESERVED "ripples") using every other service to run organizations autonomously. Locked stub content: minimum imports inference + ccd + db; per-application OWNING agents (with sub/peer agents + their own plugins); the inter-agent NEGOTIATION protocol (propose → deliberate → counter-propose → tweak → agree). North star: generalizes/replaces the hand-built Career Crafter/Lazarus org. |
| 37 | `projects` | stub | (H later) | Opus | STUB | The graphical file system over the flat VFS: a KG graph encodes project structure, nodes point at VFS files; hierarchically nested projects; centralized push registry (dashboards + source); project dashboards navigable from the mesh dashboard via surface schema; per-project metadata + finances; the CONFIRMED .mind workspace-schema migration (coordinator protocols, artifacts, tasks — rolled up). Layer-6 placeholder per INTENT #51 despite its rich confirmed scope — the stub carries heavy design notes. Depends on `artifacts` once it exists. |
| 38 | `artifacts` | stub (needs `components/artifacts.md` created) | (H later) | Opus | STUB | "No more files — artifacts": typed, schema'd, INTERACTABLE files (e.g. an HFT-strategy artifact run via a versioned controller against a simulator) — implies a script-execution engine, built out incrementally; "really boring and deterministic," possibly its own standalone tool. |
| 39 | `environments` | stub | M | Opus | STUB | An environment = a subset of a project; CICD pipeline attachable; deploy-into-environment ACTIVATES the pipeline (real-world consequences); chained blue/green promotion; bandits are in-app, NOT mesh; boring-but-not-rigid; naive nested-project rule (sub-projects share the parent's environment by default). NOTE: repo's v1 capability (branches attach to environments) presupposes a minimal environments data model — this stub must still pin the v1-facing shape of `repo-environments` even though environments itself is not implemented now. |
| 40 | `cicd` | stub (keeps its `components/cicd.md`) | M | Opus | STUB | Pipelines executing WITHIN the mesh that watch the world outside it (GH Actions runs, App Runner, Cloudflare), verify deployments (SHA-verify what production serves), may invoke agents/CCD as steps, and trend deterministic ("as few agents in the loop as possible" — grounded in the Deployment Chaperone). Kicks off workflows through repo (`cicd-repo`). OPEN ×2: sibling-vs-child of environments; own-crate vs emergent-from-repo. |
| 41 | `spend` | stub | L–M | Opus | STUB | Cost tracking, per-project, PULL-shaped: spend QUERIES its sources (CCD's usage database first; OpenRouter budgets later) — sources never push. Eventually attaches to projects' per-project metadata/finances. |
| 42 | `agents` | stub | (H later) | Opus | STUB | The future generalization layered ON TOP of CCD for non-Claude-Code agent types (which CAN choose models and MAY use local inference). Never absorbs CCD. |
| 43 | `aui` | stub | (H later) | Opus | STUB | The voice/audio interface over the whole mesh — brought into scope in the same capacity as org: a design node + anticipated data contracts only. |
| 44 | `openrouter-mgmt` | stub | L–M | Opus | STUB | OpenRouter key management: per-key creation with per-key budgets, dashboard-managed; implies edges to secrets (key storage) and spend (budget/cost tracking). |

**Totals:** 44 modules — 35 full-design (2 L0 + 4 L1 + 9 L2 + 3 L3 + 5 L4 +
12 L5) + 9 stub-track. Fable tier: 6 of 44 (`replicated-kv`, `locks`, `vdb`,
`execution-engine`, `kg`, `benchmark`); everything else Opus. (The operator
expected "roughly 30"; 35 full-design modules is the honest lib-level count —
the delta is inference's eight libs being enumerated individually as
instructed.)

---

## 3. Contract-pair inventory

Identification ONLY — no contract content authored this pass (operator's
explicit instruction). "Existing" names a stub already in `scaffold/contracts/`;
"MISSING" proposes the stub name for the per-module design passes to create
and propose content for. One tombstone stands: `mesh-registry-read` (gateway
merge). Cross-cutting one-to-many protocols follow the `surface-schema`
precedent (ONE stub naming every service as a party) rather than N per-pair
stubs.

### 3a. Existing stubs (41 live)

**Cross-service:**

| Pair | Carries | Stub |
|------|---------|------|
| client/mesh ↔ inference.api | the `/v1/` REST+WS completion surface, forwarded transparently | `v1-completion-api` |
| mesh.completion-router → inference | NodeRegistry health/load/model-inventory polls | `node-state-poll` |
| mesh ← inference | per-node WS lifecycle event stream | `inference-events` |
| mesh ← gc | per-node `:8430` WS events + REST; remote update path to the per-node GC store | `gc-events` |
| mesh ← ccd | agent-lifecycle/run WS event stream (still marked "proposed, pending confirmation") | `ccd-events` |
| mesh → dashboard | `GET /events` fan-out + REST + static hosting | `dashboard-feed` |
| every service → mesh/dashboard | the boring surface schema (render + interaction description) | `surface-schema` |
| ccd → inference | usage-observation/metering (NOT completion routing for Claude Code) | `llm-calls` |
| ccd ↔ mesh.service-registry | CCD as first-class registrant/resolver | `service-registration` |
| ccd ↔ cloud-code agents | spawn/track/signal/stream/reap | `agent-management` |
| org → ccd | org-on-ccd + the maximalist-consumer read bundle | `org-on-ccd` |
| db ↔ consumers | noun-verb control plane (migrations, edge functions, query, outbox) | `db-control-plane` |
| inference → db | fresh-node database init | `db-inference-init` |
| vfs ↔ gc | per-device policy enforcement calls | `vfs-gc` |
| vfs ↔ mesh | registration + node/drive topology + perf reporting | `vfs-mesh` |
| projects → vfs | graph-pointed project storage | `projects-vfs` |
| projects ↔ mesh | registry push + dashboard surfacing | `projects-mesh` |
| kg ↔ mesh | registration + graph replication (S3 leg rides aws) | `kg-mesh` |
| kg → vfs | node→file pointers + existence validation | `kg-vfs` |
| vdb/stack → vfs | the SQLite-file-in-VFS layering edge | `stack-vfs` *(rename to `vdb-vfs` at design time — flagged)* |
| vdb/stack ↔ mesh | registration + distributed-handler coordination via locks | `stack-mesh` *(rename to `vdb-mesh` at design time — flagged)* |
| secrets ↔ mesh | registration + replication ~3 + brokerage shape TBD | `secrets-mesh` |
| ccd → rollup | on-demand plugin/prompt assembly | `rollup-ccd` |
| repo → vfs | repo state materialized through VFS; push/sync to remotes | `repo-vfs` |
| repo ↔ environments | branches/worktrees attach; deploy-into-environment activates pipeline | `repo-environments` |
| cicd → repo | kick off workflows / set up chains (collapses if cicd emerges from repo) | `cicd-repo` |
| aws ↔ mesh | registration + the replication plane's S3/AWS distribution leg | `aws-mesh` |
| aws ↔ vfs | the S3 overflow tier data path | `aws-vfs` |

**Mesh-internal:**

| Pair | Carries | Stub |
|------|---------|------|
| tailscale-query → network-topology *(completion-router dropped as co-consumer at the contract round)* | parsed self+peer status snapshots | `tailscale-status` |
| network-topology → any consumer | peer on/off + self-connectivity WS feed | `network-events` |
| service-registry ↔ any device/service | register/resolve — THE wiring seam | `service-lookup` |
| registry peers ↔ peers | anti-entropy replication | `registry-replication` *(generalization candidate: `kv-replication` once `replicated-kv` is extracted — flagged)* |

**Inference-internal:**

| Pair | Carries | Stub |
|------|---------|------|
| store ↔ all seven siblings | SQLite system-of-record CRUD + observer seam | `store-access` |
| scheduler → engine | submit/drain, slots, model swap | `engine-exec` |
| telemetry → {scheduler, api} | SystemState + throughput estimates | `system-state` |
| scheduler → models | ensure-downloaded + pipeline status | `model-ensure` |
| engine ↔ cache | KV/prefix save/restore | `kv-cache` |
| {models, cache, engine} → gc (embedded) | register/touch/lock/make_room in-process | `gc-managed-dirs` |
| benchmark → scheduler | priority-0 exclusive sweep collections | `benchmark-collections` |
| api → {scheduler, store, telemetry, benchmark} | node-internal dispatch of the /v1 surface | `api-dispatch` |
| scheduler → telemetry(kernel) | effective-concurrency / confidence reads | `kernel-confidence` |

### 3b. MISSING pairs — full-design track (16 proposed stubs)

| Pair | Carries | Proposed stub |
|------|---------|---------------|
| kg ↔ vdb | KG built ON VDB: graph storage/routing through the VDB daemon (locked layering; edge naming was explicitly deferred) | `kg-vdb` |
| vdb → db | the VDB daemon uses `db` to run actions against specific databases (cross-app: WS/CLI, never linked) | `vdb-db` |
| vdb ↔ secrets | connection strings / credentials for cloud targets, use-without-seeing | `vdb-secrets` |
| aws ↔ vdb | the RDS+Lambda cloud target surface | `aws-vdb` |
| db ↔ secrets | db reconciles to consume secrets; its Supabase-Vault module serves as secrets' Supabase adapter | `db-secrets` |
| repo ↔ secrets | secret↔workflow linkage + injection-on-push (THE v1 capability) | `repo-secrets` |
| rollup ↔ secrets | raw/ID addressing from rollup content; llm_safe fail-or-degrade governs | `rollup-secrets` |
| aws ↔ secrets | the AWS Secrets Manager push adapter (DESIGN-ONLY in v1 — contract designed, not built) | `aws-secrets` |
| mesh(queues/triggers) → rollup | trigger payload-assembly calling rollup (declarative references); + rollup's own registration | `rollup-mesh` |
| mesh(DLQ)/execution-engine → ccd | the escalation surface: dead-letter investigation + loop-depth-exceeded investigation (one shared shape) | `ccd-escalation` |
| every service ↔ mesh | the standard WS pub/sub envelope protocol (typed structs, topics, relay semantics) — cross-cutting, surface-schema-style | `pubsub-protocol` |
| mesh ↔ every service | the two-way 4-level graceful-restart / supervision protocol (interruptibility state, port-handoff choreography) | `restart-protocol` |
| any service ↔ mesh.queues | publish typed events, register declarative triggers, handler delivery | `queues-api` |
| any service ↔ mesh.locks | acquire/release/renew distributed semaphores + the partition-merge error type | `locks-api` |
| any service ↔ mesh.cron | schedule "on node N" / "anywhere" tasks | `cron-api` |
| rollup → vfs | fragment/plugin storage residence (probable — confirm in rollup's design pass) | `rollup-vfs` |

### 3c. MISSING pairs — stub-track anticipated contracts (12 proposed stubs)

Layer-6 stubs list anticipated contracts by name only; content comes when the
module leaves the stub track.

| Pair | Carries | Proposed stub |
|------|---------|---------------|
| projects → kg | project structure as a KG graph (deferred edge named in kg.md/projects.md) | `projects-kg` |
| projects → rollup | "tasks are rolled up" — the .mind-migration rollup integration | `projects-rollup` |
| projects → vdb | databases attached to projects/environments | `projects-vdb` |
| projects → artifacts | projects depends on artifacts once it exists | `projects-artifacts` |
| ccd ↔ projects | thread↔project (+optional environment) linkage in CCD's usage DB | `ccd-projects` |
| spend → ccd | pull-shaped spend queries of CCD's usage database | `spend-ccd` |
| secrets ↔ environments | pushing secrets into specific environments | `secrets-environments` |
| environments ↔ vdb | environment routes storage (local→SQLite, cloud→promote) | `environments-vdb` |
| agents → ccd | the future generalization layered on CCD | `agents-ccd` |
| aui ↔ mesh | voice interface over the whole mesh (entry surface) | `aui-mesh` |
| openrouter-mgmt ↔ secrets | API-key storage/rotation | `openrouter-secrets` |
| openrouter-mgmt ↔ spend | per-key budgets / cost tracking | `openrouter-spend` |

**Contract-pair count: 41 existing live stubs + 28 missing (16 full-track +
12 stub-track anticipated) = 69 identified pairs** (+1 tombstone). Note:
execution-engine↔{vdb, kg} and all shared-lib consumption (`types`,
`tailscale-query`, `mesh-client`, `execution-engine`) are internal library
dependencies, deliberately NOT contract edges (locked rounds 4–5).

---

## 4. Batch plan

Bottom-up by layer, 5–8 modules per batch (INTENT #107: "layer by layer, 5–8
agents at a time — I don't want it to fail mid-run on 10 of them"). Each
batch's designers get the finished designs of all lower batches. Co-batched
modules with edges between them (marked ⇄) should share their proposed
contract drafts mid-batch or be sequenced within the batch.

| Batch | Layer(s) | Modules (model tier) | Notes |
|-------|----------|----------------------|-------|
| **1** | L0 + L1 | `types` (Opus), `tailscale-query` (Opus), `mesh-core` (Opus), `pubsub-relay` (Opus), `network-topology` (Opus), `mesh-client` (Opus) | The kernel batch. `types` first among equals — its surface-schema/pubsub/event modules ground everyone. |
| **2** | L2 state | `replicated-kv` (**Fable**), `service-registry` (Opus), `locks` (**Fable**), `queues` (Opus), `cron` (Opus), `supervision` (Opus) | The replicated-state batch. registry ⇄ replicated-kv; locks ⇄ replicated-kv; queues ⇄ locks. The two Fable seats carry the wave's consistency risk. |
| **3** | L2 services + L3 | `completion-router` (Opus), `dashboard-serving` (Opus), `aws` (Opus), `vfs` (Opus), `gc` (Opus), `secrets` (Opus) | vfs ⇄ gc co-design resolves rolled-in-vs-called-as-tool; vfs/secrets ⇄ aws for the overflow/push legs. |
| **4** | L4 | `db` (Opus), `vdb` (**Fable**), `execution-engine` (**Fable**), `kg` (**Fable**), `rollup` (Opus) | The data/execution batch — heaviest Fable concentration. vdb ⇄ db ⇄ execution-engine ⇄ kg; execution-engine's trigger data model must match batch-2's `queues` design. |
| **5** | L5 inference core | `inference` (Opus), `store` (Opus), `engine` (Opus), `models` (Opus), `cache` (Opus), `api` (Opus) | The inference-runtime batch, grounded in real V1 code. |
| **6** | L5 kernel triangle + apps | `scheduler` (Opus), `telemetry` (Opus), `benchmark` (**Fable**), `repo` (Opus), `ccd` (Opus), `dashboard` (Opus) | scheduler ⇄ telemetry ⇄ benchmark co-design the multidimensional adaptive kernel (INTENT #12). repo needs batch-3's vfs/secrets; ccd needs batch-4's rollup; dashboard needs batch-3's dashboard-serving + batch-1's surface schema. |
| **7** | L6 stubs (project-facing) | `projects`, `environments`, `cicd`, `org`, `artifacts` (all Opus, STUB track) | Design notes + anticipated contracts + "not implementing now" notes only. `environments` must still pin the v1-facing shape of `repo-environments` for batch-6's repo. Creates `components/artifacts.md`. |
| **8** | L6 stubs (ops-facing) | `spend`, `agents`, `aui`, `openrouter-mgmt` (all Opus, STUB track) | Lightest batch; closes the inventory. Follow with the wave-end friction-point surfacing + clarification round per INTENT #107. |

After batch 8: the per-pair contract round (Contract Harmonizer discipline —
one internally-consistent example world), then harmonization, then surface ALL
friction points for the operator's clarification round.

---

## 5. Ambiguities & flags carried to the design batches

1. **`vdb` vs `stack` naming** — module named `vdb` per the round-8 lock ("VDB
   becomes the daemon"); "stack" survives as the pattern name and as
   `components/stack.md`. Crate naming + renaming `stack-mesh`/`stack-vfs` →
   `vdb-mesh`/`vdb-vfs` is batch-4's to settle with the operator.
2. **`environments` is stub-track yet load-bearing for repo's v1** — the stub
   must pin a minimal environments data model for `repo-environments`.
3. **`projects` stub-track vs its enthusiastically-confirmed scope** (the
   .mind schema migration) — the stub carries full design notes; nothing is
   lost, only implementation deferred (INTENT #51 governs).
4. **Cross-cutting protocol contracts** (`pubsub-protocol`, `restart-protocol`,
   `queues-api`, `locks-api`, `cron-api`) are modeled surface-schema-style
   (one stub, every service a party) rather than N per-pair files — confirm
   this shape suits the operator's pairwise-contracts philosophy (the pairs
   still exist; the document is shared).
5. **`registry-replication` → `kv-replication` generalization** once
   `replicated-kv` is extracted (batch 2's call).
6. **`ccd-events` still carries its "proposed, pending confirmation" marker**
   from the original decompose — confirm or strike in batch 6.
7. **35 full-design modules vs the operator's "roughly 30"** — the delta is
   the instructed lib-level enumeration (inference ×8, mesh internals ×8);
   no scope was added.
8. **`gc` rolled-into-vfs vs called-as-tool** — still open, resolved in
   batch 3 by co-design.
9. **Mixed-version update protocol** (nodes running different service
   versions) — still OPEN inside `supervision` (batch 2); may need its own
   operator round.
10. **KG graph-merge consistency model** — the wave's hardest single open
    design (batch 4, Fable).
