# Substrate — Scaffold Overview (wave-3 close-out)

> **OS NAME STILL OPEN; "SUBSTRATE" IS THE WORKING NAME (INTENT
> #129/#141/#171/#172).** A focused naming round is queued (ledger OQ-7). Until
> it lands, every "Substrate" in this tree reads as the WORKING name. Naming
> history, for the record only: **Mind OS** was once locked ("an operating
> system of my mind") but the name is taken; **Broomstick OS** ("corny"),
> **Mesh OS** ("doesn't say what it is"), **Mind Temple / Temple OS** ("a tiny
> bit cliche"), and **Landscape OS** (the word `landscape` is now spent on the
> graph) were all floated and set aside. Criteria the operator set: simple,
> STT-friendly, non-cliche, resonant, unfamiliar-OK. "Mycelium" and
> ripple-derived names were rejected; **`ripples` is a RESERVED term** and
> names nothing in v1.

## Canonical vocabulary (LOCKED — INTENT #172, wave-3 intent ledger §A1)

The one place the tree's load-bearing words are defined. Where any component or
contract file says otherwise, THIS block supersedes it (with the INTENT ledger
above it). Historical terms may appear ONLY in naming-history notes.

- **landscape** — the ONE open-world topological graph in which all our projects
  and things live; it holds **keepers AND artifacts**. **LOCKED as the GRAPH
  name — explicitly NOT the OS name** (INTENT #158/#162). Built on `kg`.
- **keeper** — a node on the landscape that owns its surrounding region: a
  **topological node = an org-chart seat** ("you split an org by topology, not
  by which humans you can get"). **project is the PRIMARY keeper type.** Keepers
  own their region and delegate to sub-keepers; threads spin up against a keeper.
  **LOCKED** (INTENT #162/#172) — formerly riffed as topological node /
  coordinator / owner / deva / bishop / Jester, **all superseded**. Replaces the
  dissolved `org` concept ("coordinator = owner = keeper," INTENT #130/#132).
- **bundle** — a keeper's **persistent shared knowledge** (rolled-up init: role
  prompt + plugins), generalizing to the **targeted-rollup product of ANY
  landscape node** (keepers and artifact nodes both). For an artifact node it is
  the type's skills/tools, appearing when the artifact is in sight. **LOCKED**
  (INTENT #165/#166/#172). **Supersedes "seed"** for a node's persistent
  memory — that word is retired; use it for nothing but the KG self-seed sense.
- **workspace** — the per-thread **ephemeral memory**, one-to-one with a thread.
  Nothing is shared between threads except the keeper's bundle (like today: one
  repo, many workspaces each with a thread; nothing shared except `.mind`).
  **LOCKED** (INTENT #149/#161).
- **chassis** — **LOCKED** (INTENT #156/#166): the daemon-wrapper core lib every
  service inherits (formerly "proto-daemon / DW / **mesh-client**"). Brings the
  daemon online, establishes data contracts, enforces Rust-exhaustive per-app
  case handling. **`mesh-client` retired into it** (wave-3, ledger D1).
- **threads** — how agents run against a keeper: **interactive** (human-attached)
  or **headless** (started from a message) (INTENT #149/#150). A thread is itself
  a node, linked to what it generated.

## Wave 3 — COMPLETE (status block)

**Date:** 2026-07-22. **Commit lineage (wave-3):**
`0c503a5` (batch 1 — envelope switch, chassis, mesh-transport, mesh-client
tombstone) → `297fb6c` (batch 2 — mesh mediation, delivery modes, topology+time,
dashboard fold) → `bffbfd3` (batch 3 — queues+cron fold, registry versioning,
router+supervision refresh) → `477859b` (batch 4 — schema/rollup/kg reshape, vdb
sessions, engine+kv refresh) → `90f0107` (batch 5 — inference modalities,
vfs+secrets+spend consolidation) → `e20306a` (batch 6 — keeper centerpiece + L6
conceptual designs) → **this commit** (batch 7 — vocabulary sweep, contract
refresh, harmonization close-out). Wave 3 sits on top of the wave-2 line
(`0c5d6fb` contract round → `586561c` harmonization) and the three friction
rounds + re-spoken locks (`e948cb5` → `d605db5` → `ab68f57` → `c4a44f9`).

**What wave 3 did** (bottom-up per #44/#102; guardrail = the wave-3 intent
ledger, `substrate-v2/reports/wave3-intent-ledger.md`, and INTENT.md #1–#173):

- Folded the **comms stack** (#152–#157) that wave-2 left unfolded: the
  success/error/promise **envelope switch** (`mesh-transport`, new), universal
  **mesh mediation + promises + streaming rules** (into `mesh-core`), configurable
  **pub/sub lossiness + intermediate cache** (into `pubsub-relay`).
- Created **`chassis`** (the daemon-wrapper) and retired **`mesh-client`** into it.
- Absorbed **`cron` → `queues`** (a `Schedule` trigger source), **`tailscale-query`
  → `network-topology`**, **`dashboard-serving` → `mesh-core`**, completing the
  earlier **`gc` → `vfs`** and **`openrouter-mgmt` → `spend`** folds.
- Reshaped the data/execution plane: **`schema`** folded to implementation-ready
  (nested + multiple inheritance, codegen bridge, DC-emergent surface),
  **`rollup` toward KG/schema**, **`kg` distributed-everywhere** flagged, **vdb**
  session home confirmed, **error-taxonomy** migration staged in `types`.
- Put **S2T/T2S** as `inference` modalities with a warm-model policy.
- Designed the **L6 conceptual layer**: **`keeper`** (NEW — the wave's central
  design, replaces `org`), plus `projects` (now a landscape view, not a crate),
  `artifacts`, `agents` (stays), `environments`/`cicd` (stubs), and the new
  `ui-server`/`aui-client` placeholders (#168).
- **This batch (7):** vocabulary sweep to the LOCKED words across every component
  and contract; contract-graph refresh (adding `mesh-transport` + `schema-query`,
  tombstone-noting the folded contracts); loose-annotation cleanup; and this
  close-out.

What exists at close-out: **42 live component design files** in
`scaffold/components/` plus **10 tombstone/history records** (`cron`,
`dashboard-serving`, `gateway`, `gc`, `mesh-client`, `openrouter-mgmt`,
`tailscale-query`, `org` — tombstones-with-content; `mesh` — the pre-split mesh
charter record; `stack` — superseded, the module is `vdb`, "stack" survives as
the PATTERN name). **~71 live contract files** in `scaffold/contracts/` plus
tombstones (`cron-api`, `mesh-registry-read`, `registry-replication`,
`stack-vfs`, `stack-mesh`) and two folded-to-internal surfaces (`tailscale-status`
→ network-topology-internal; `vfs-gc` → vfs-internal).

**Governing precedence:** INTENT.md #1–#173 (verbatim operator truth) wins over
all scaffold text; the wave-3 ledger's SETTLED index is the derived guardrail;
contract files win over component-file proposals; the canonical vocabulary block
wins over any older wording; `wave2-plan.md` remains the wave-2 module-inventory
record (retains pre-rename `ccd`/`onion`/`org` vocabulary as history).

## The OS layering — final component tree

A module may only depend on its own layer's peers and lower layers; every
cross-app call goes through the local mesh daemon (single-port locality,
`:3649`); in-process linking across app boundaries is forbidden (INTENT #29),
shared libs excepted (compiled in — `types`, `chassis`, `execution-engine`).
Kinds: app-crate · internal-lib(parent) · shared-lib · app-frontend · stub.

```
L0 foundation      types (shared-lib)
                   [tailscale-query — ABSORBED into network-topology (L1),
                    wave-3 F9a; tombstone-with-content]

L1 mesh kernel     mesh-core (bin/mesh, :3649 — absorbs dashboard-serving:
                   HTTP + kv-read dashboard hosting; universal mediation,
                   promises, streaming rules, mesh time-authority)
                   pubsub-relay (lib — configurable lossiness + intermediate
                   cache)      network-topology (lib — absorbs tailscale-query
                   as its `query` submodule + the time-measurement half)
                   chassis (shared-lib — the daemon-wrapper; ABSORBS mesh-client)

L2 state + OS svcs replicated-kv    service-registry (THE wiring seam)
                   locks            queues (absorbs cron as a Schedule trigger)
                   supervision      completion-router      aws (app-crate)
                   [cron — ABSORBED into queues; dashboard-serving — FOLDED into
                    mesh-core; both tombstones-with-content]

L3 storage plane   vfs (app; gc ABSORBED as an internal removal-spectrum module —
                   mark-for-removal / move-between-devices / cold-storage)
                   secrets (app — use-without-seeing, keeps ALL versions)
                   [gc — ABSORBED into vfs; tombstone-with-content]

L4 data+execution  db (app — pure CLI tool, NO daemon; sessions live in vdb)
                   vdb (app; "stack" = the pattern; owns db sessions + provenance)
                   schema (app — versions + nested/multiple inheritance, codegen
                   → Rust libs, DC-emergent surface)     kg (app — landscape
                   substrate; distributed-everywhere flagged)
                   rollup (app — toward KG/schema)   execution-engine (shared-lib)

L5 services/apps   inference (app) ── store engine scheduler models cache
                                      telemetry benchmark api (8 internal libs);
                                      S2T/T2S are inference MODALITIES (#166 Q12)
                   repo (app)       cc (app)      dashboard (ui/dashboard)

L6 landscape /     keeper (NEW — the topological node; REPLACES org; the wave's
   org plane         central L6 conceptual design; NO crate this pass)
   (conceptual /    projects (a landscape VIEW / primary keeper TYPE — NOT a
    STUB track)      crate; the .mind workspace-schema migration home)
                   artifacts (the OTHER HALF of the landscape — types are schemas,
                     inspectable/editable; bundle-appears-in-sight)
                   spend (openrouter-mgmt ABSORBED; anchored to landscape keepers)
                   agents (custom-agent-harness placeholder ONLY — #167; never cc)
                   environments (RESERVED for its own deep-dive — #164)   cicd (stub)
                   aui   ui-server   aui-client (AUI split placeholders — #168)
                   [org — DISSOLVED (#132): emergent, "a bunch of keepers";
                    org.md is a tombstone-with-content, design-lineage → keeper.md]
                   [openrouter-mgmt — ABSORBED into spend; tombstone-with-content]
```

Notable placements: `aws` sits at L2 because the replication plane, VFS overflow,
VDB cloud target, and secrets push all consume it; `schema` sits at L4 (the
gate that once deferred it — "the dedicated KG-templating round" — is LIFTED,
batch 4); `keeper` is L6 and conceptual only (no crate, #148 beat 1 / directive
#173c). Inference's per-node loopback convention is `:8420`; `:3649` is the only
mesh port.

## Final contract graph

Grouped by cluster; every name is a file in `scaffold/contracts/`. Direction and
payload detail live in the files (their Reconciliation notes are authoritative).

**Cross-cutting protocols (one shared document, every service a party):**
`service-lookup` (register/resolve — THE wiring seam) · `mesh-transport`
(**NEW, wave-3** — the success/error/promise envelope switch + framing layer,
#154; the byte-level layer ~12 contracts bind to) · `pubsub-protocol` (the
standard WS pub/sub envelope + configurable lossiness) · `restart-protocol`
(two-way 4-level graceful-restart ladder + interruptibility + port-handoff) ·
`queues-api` (typed events → declarative triggers → handlers; **absorbs the
tombstoned `cron-api`** as a `Schedule` trigger source) · `locks-api` (distributed
semaphores + the partition-merge error) · `surface-schema` (the boring
render+interaction schema every service publishes) · `schema-query` (**NEW,
wave-3** — the runtime schema / generated-type query surface, DC-emergent, #166
Q11 / #150 beat 16).

**Mesh kernel + replication plane:**
`kv-replication` (the ONE anti-entropy protocol for all kernel state; supersedes
`registry-replication`) · `network-events` (peer on/off + self-connectivity
feed) · `aws-mesh` (the replication plane's S3/AWS leg) · `kernel-confidence`
(scheduler → telemetry kernel reads). *(`tailscale-status` is now a
network-topology-INTERNAL surface — tailscale-query absorbed, wave-3 F9a — the
file records it as the internal record.)*

**Completion data plane:**
`v1-completion-api` (the `/v1` REST+WS surface, forwarded transparently) ·
`node-state-poll` (router's reconcile reads) · `inference-events` (per-node
pub/sub lifecycle stream, incl. modality/warm-model events) · `llm-calls`
(cc → inference, metering-shaped).

**Observability plane:**
`dashboard-feed` (browser fan-out + REST + static hosting — served by mesh-core
now) · `gc-events` (emitter is vfs's internal gc module) · `cc-events` ·
`system-state`.

**Storage plane:**
`vfs-mesh` · `vfs-content` (bulk content bytes, pull-driven) · `vfs-gc` (**now
vfs-INTERNAL** — gc absorbed into vfs; the file carries the module's internal
surface) · `aws-vfs` (S3 overflow tier) · `gc-managed-dirs` (embedded-gc
in-process surface — `lib/gc` stays a reusable piece) · `secrets-mesh` ·
`db-secrets` · `repo-secrets` (secret↔workflow injection-on-push — THE v1
capability) · `rollup-secrets` · `vdb-secrets` · `aws-secrets` · `openrouter-secrets`
(stub-track) · `secrets-environments` (stub-track).

**Data & execution plane:**
`db-control-plane` · `db-inference-init` · `vdb-db` (**sessions live in vdb, db
is a pure CLI with no daemon — the file records vdb's internal session surface**,
#167) · `vdb-vfs` (SQLite-file-in-VFS; renamed from `stack-vfs`) · `vdb-mesh`
(renamed from `stack-mesh`) · `aws-vdb` (RDS+Lambda target, design-only v1) ·
`kg-vdb` (KG built ON VDB) · `kg-vfs` (node→file pointers) · `kg-mesh` · `kg-api`
(the KG consumer surface) · `rollup-mesh` (trigger payload-assembly) · `rollup-vfs`
· `rollup-cc` · `engine-exec` · `cc-escalation` (DLQ + loop-depth investigations).

**Inference-internal (the node's crate contracts):**
`store-access` · `engine-exec` · `model-ensure` · `kv-cache` ·
`benchmark-collections` · `api-dispatch`.

**Service/application plane:**
`service-registration` (cc as registrant/resolver) · `agent-management`
(spawn/track/signal/stream/reap) · `org-on-cc` (the maximalist-consumer read
bundle — party dissolved at #132; survives as the shaped-for record for the
keeper concept) · `repo-vfs` · `repo-environments` (v1-facing shape pinned) ·
`cicd-repo`.

**L6 stub-track (anticipated contracts, content deferred):**
`projects-vfs` · `projects-mesh` · `projects-kg` · `projects-rollup` ·
`projects-vdb` · `projects-artifacts` · `cc-projects` · `spend-cc` · `agents-cc`
(Claude-Code shape only — #137) · `aui-mesh` · `openrouter-spend`
(`openrouter-mgmt` absorbed into `spend` — the OpenRouter key-usage pull is now
spend's own source adapter) · `environments-vdb`.

**Tombstones:** `cron-api` (folded into `queues-api`), `mesh-registry-read`
(gateway merge), `registry-replication` (superseded by `kv-replication`),
`stack-vfs` / `stack-mesh` (renamed `vdb-vfs`/`vdb-mesh`) — each file explains
its supersession.

**Deliberately NOT contract edges:** all shared-lib consumption (`types`,
`chassis`, `execution-engine`) is internal library dependency, invisible in the
contract graph by design.

## The wiring seam — service-registry + supervision

**The runtime composition point is the mesh `service-registry` (`service-lookup`),
executed by `supervision`.** Every service, at boot via `chassis`, registers its
`slug -> {scheme, host, port, health_path}` against its LOCAL mesh daemon on
`:3649` and resolves every dependency by slug — never a static URL. Records ride
`replicated-kv` (leases + heartbeats + LWW tombstones), so "anywhere you access
mesh is exactly the same."

- **Addressing classes:** `Singleton` (cc, kg, secrets…; `db` no longer
  registers — pure CLI, no daemon) · `NodeScoped` (vfs legs incl. its internal
  gc module; per-node `inference` instances) · `FleetAlias` (a resolve-time
  policy on the `inference` slug → completion-router).
- **Supervision executes the seam:** dependency-derived boot order, the 4-level
  restart ladder (`restart-protocol`), port-handoff updates, minimal-restart
  rolling updates. Mixed-version updates are CONFIRMED (#113): the restart-priority
  ladder IS the answer, plus LOCKED sender-version stamping.
- **Universal mediation (wave-3, #152):** EVERYTHING follows the loopback pattern
  through the local mesh daemon; never service-to-service directly. If a target
  is down, mesh restarts it and patches the message through. If a service can't
  answer instantly, mesh returns a **promise**; the caller moves on; the value is
  pushed back over WS. Every inter-service contract handles the promise case
  (`mesh-transport` envelope: success / error / promise).

## Standing cross-cutting principles

1. **INTENT wins.** Where scaffold text conflicts with INTENT.md (#1–#173),
   INTENT governs; the canonical vocabulary block wins over older wording.
2. **Provenance is first-order** — healthcare-grade traces on every handler
   touch; scoped per-project/per-database; VDB primary, VFS lighter.
3. **Pairwise data contracts, mediated by mesh** — any contract change triggers a
   design session analyzing ripple effects; DC is EMERGENT from chassis + schema
   + generated Rust libs (no `dc` crate, #166 Q11).
4. **Single-port locality + universal mediation** — every service talks only to
   its local mesh daemon on `:3649`; mesh does all relaying, promises, and
   patch-through-on-restart. In-process linking across app boundaries forbidden (#29).
5. **Boring layers on boring layers** — mesh utilities are internal libs, never
   standalone services; replicated state is naive LWW; the HLC ratchet stands
   under the mesh time-authority requirement (mesh owns time, #116).
6. **Triggers are declarative** — filter expression + payload-assembly template,
   registered as DATA; code lives only in handlers. One trigger data model shared
   by queues (incl. the absorbed `Schedule`/cron source) and the execution-engine.
7. **No secret ever reaches an LLM** — use-without-seeing, `llm_safe`
   fail-or-degrade; rollup never resolves a raw secret reference in LLM-bound content.
8. **NO DOCKER locally, ever; SQLite-locally LOCKED** — Postgres exists only as a
   cloud VDB target via `aws`; containers, if ever, live only in AWS.
9. **Wire-struct version discipline** — additive-only, `#[serde(default)]`,
   `#[serde(other)]` tolerance, explicit `v` fields on persisted/replicated
   shapes; every mesh-crossing message carries sender service name + version;
   receiver version floors are catchable; one version of back-compat + a
   please-update warning (#113).
10. **The two-engines rule** — **rollup composes text; schema migrates
    structure.** Never a third engine (#166 F-2). schema's gravity-well is
    BLESSED ("simple boring thing, many surfaces").
11. **Naming guardrails:** the LOCKED vocabulary is **landscape / keeper / bundle
    / workspace / chassis / threads** (see the vocabulary block); `inference`
    will not be renamed; the module is `vdb` ("stack" = the pattern); `ccd`→`cc`,
    `onion`→`schema` (#131); **`ripples` is RESERVED** and names nothing in v1;
    the OS working name is **Substrate**, final name OPEN (OQ-7).
12. **Process law — boring decisions only (#124/#173d):** agents capture all open
    questions and make the single most BORING decision; anything non-boring is
    escalated to a clarification-round beat, never assumed. When in doubt, choose
    the placeholder that can be un-chosen with a one-line edit.
13. **AUI never touches `~/code/substrate` directly (#1/#10);** reports are the
    communication fabric (#2); no technical debt by design (#38); a crate is an
    app, everything else a library (#22).

## Open questions — the operator's ledger (32; align to §B of the intent ledger)

Full verbatim-grade records live in `substrate-v2/reports/wave3-intent-ledger.md`
§B and in INTENT.md. Three tiers. **PARKED items MUST stay open — design AROUND
them, do NOT decide them in scaffold (#173d).** The design-around placeholders
live in ledger §C.

**PARKED (6) — reserved for dedicated discussion; do NOT decide:**
- **OQ-1. Authority node + the no-authority merge-delegation alternative.**
  `[PARKED]` The "authority node" framing is WITHDRAWN; the boring provisional is
  a merge-reconciler surface on `locks` that emits a conflict event per affected
  application (#163). No `authority` crate, no blessing-family contract.
- **OQ-2. Environments — full deep-dive.** `[PARKED]` `environments` stays a
  requirements-only stub with thin edges to repo/vdb/secrets (#164).
- **OQ-3. Queues-as-pubsub-with-persistence consolidation.** `[PARKED — "didn't
  seem very boring"]` `pubsub-relay` stays purely lossy; the durable path is a
  separable delegation to queues, marked NEEDS-EXPLANATION, not blessed (#167 SB7a).
- **OQ-4. Bottom-up topography — actual structure + initial schemas per node
  type.** `[PARKED]` Landscape = one open-world KG, project the primary keeper
  type; `has-repo` is a candidate edge, not a resolution (#144/#167 F-3).
- **OQ-5. Keeper↔bundle curation mechanism.** `[PARKED]` Curation = a KG write on
  the bundle node at thread end; golden-rules-vs-metacognitive-pass undecided (#167 F-4).
- **OQ-6. Keeper-to-keeper message + approval protocol.** `[PARKED]` Approvals
  ride the durable queues (exactly-once); the propose→deliberate→approve state
  machine itself is undesigned (#88/#127/#149 beat 10).

**RAISED-AND-UNANSWERED (21) — his questions, no answer:**
- **OQ-7. OS name — final call.** `[OPEN]` Focused naming round queued (#171/#172).
- **OQ-8. herald / marshal / console sub-role words.** `[OPEN]` keeper+bundle
  locked; sub-role assignments unconfirmed (#162/#172).
- **OQ-9. Org-chart seat types beyond technical.** `[OPEN]` Reference the
  CareerCrafter agentic-org proposal; hierarchy of node types w/ multiple
  inheritance (#150 beat 14).
- **OQ-10. CC-free-on-Max spend accounting.** `[OPEN]` Max-plan runs are ~free;
  spend must represent that honestly (#170).
- **OQ-11. KG merge machinery under distributed-everywhere.** `[OPEN]` home-node +
  offline model needs revisiting; kg-api offline behavior is a per-graph knob (#135).
- **OQ-12. Multiple human threads per node — semantics.** `[OPEN]` collides with
  one-per-workspace; conflict-event notification for the losing init delta (#146/#142).
- **OQ-13. Mesh time-authority design.** `[OPEN]` HLC ratchet + mesh-owns-time
  sketched; full design is fill-time (#116).
- **OQ-14. The cloud node.** `[OPEN]` Lambda-behind-API-Gateway + S3 as "one
  node"; not a physical device (#110/#157).
- **OQ-15. .mind migration path.** `[OPEN]` Substrate for coordinator/artifact/task
  migration undecided (queues+pubsub vs KG vs hybrid; synthesis leans hybrid, #87/FP9).
- **OQ-16. git / branch / worktree / merge / environment charter.** `[OPEN — the
  biggest structural gap]` What happens to a keeper's KG-homed workspace/bundle on
  branch merge; projects↔repo binding (#123).
- **OQ-17. Keeper as a crate this pass?** `[OPEN]` Directive #173b: conceptual
  only, no crate this wave — leaning capture-now (answered by keeper.md).
- **OQ-18. Compaction — which bundle version to re-seed from, what must be saved.**
  `[OPEN]` "compact and lose basically nothing" (#123).
- **OQ-19. Ownable node types beyond app/project/component.** `[OPEN]` boring core
  adds sub_project/feature/service; past that is his call (#127).
- **OQ-20. Types↔schema authority-of-record.** `[OPEN]` compiled vs runtime
  types — which wins; codegen bridge keeps both reachable (#145).
- **OQ-21. Schema as the universal messaging protocol.** `[OPEN]` #139 — its own
  round; not applied to authored contracts.
- **OQ-22. Name the "beautiful property."** `[OPEN]` each thread modifying future
  inits; may be retired per #149 beat 5 — verify (#146).
- **OQ-23. Tools registry + optional MCP surface.** `[OPEN]` in tension with the
  harness no-MCP stance — surface it, don't silently import (#30/#126).
- **OQ-24. Postgres-vs-no-Docker reconciliation.** `[OPEN]` prove SQLite-only is
  sufficient first (#82/#91).
- **OQ-25. Services contribute dashboard COMPONENTS — version architecture.**
  `[OPEN]` leaning: components ride the schema/DC surface (#37/#158).
- **OQ-26. App-development framework for the OS.** `[OPEN — future scope]` (#118).
- **OQ-27. Restart-signal philosophy.** `[OPEN]` owned by the (now-existing)
  substrate plugin via a critic pattern (#119).

**PENDING HIS BLESSING (5) — designer positions that can ride the closing round:**
- **OQ-28. Consolidation sign-offs** (seed-bishop Q5–Q12): projects→keeper type
  (reverses #47), environments/artifacts keep-vs-fold, spend+openrouter, DC-emergent,
  S2T/T2S — most applied provisionally, awaiting beat-by-beat blessing.
- **OQ-29. Authority beats** (Q1–Q4): superseded by OQ-1's parking; the
  sub-questions remain his once OQ-1 is discussed.
- **OQ-30. Transport authorizations** (E2a/E2b, FP13): authorize the streaming
  peer-link channel + the `mesh-transport` envelope spec; whether pub/sub ever
  grows a retained store.
- **OQ-31. Standing-law confirmations**: v-field discipline, benchmark-Idle,
  mixed-version newest-wins, presigned-direct-to-S3, no-net-handlers, HLC ratchet
  — mostly answered, awaiting explicit confirmation.
- **OQ-32. cicd** — own crate vs emergent-from-repo; sibling vs child of
  environments — stub-only this wave (FP18/#104).

*Resolved-by-delegation (record, don't re-raise):* db-vs-vdb session home → VDB
(#167); DC → emergent (#166 Q11); S2T/T2S → inference (#166 Q12); gc → vfs (#166
Q9); spend+openrouter merged (#166 Q10); multiple-inheritance conflicts → explicit
manual resolution (#166 Q16); org → emergent keepers (#132); gateway → mesh (#45);
cron → queues (#56/#91); tailscale-query → network-topology (F9a);
dashboard-serving → mesh-core (F9b); mesh-client → chassis (#156).

## History

The round-by-round lock history (wave-2 rounds 3–9; friction rounds 1–3; the
re-spoken vocabulary round) lives in git history (`git log -- scaffold/overview.md`,
through commit `c4a44f9`) and in the wave-3 intent ledger. `wave2-plan.md` records
the wave-2 module inventory and the flags carried into the batches. The surviving
decisions are all encoded in the component/contract files above.
