# ccd

**Status:** WAVE-2 REFIT of the wave-1 `ccd.md` (net-new crate; no prior repo
code). **Layer:** L5 app-crate (service/application plane). **Nesting:**
top-level, flat — internal parts are libraries within the crate (crate=app,
INTENT #22). **LOCKED (INTENT #49/#68/#104, round-9):** CCD stays a top-level
service, NOT merged under any `agents` umbrella; a future `agents` layer
generalizes ON TOP of CCD and never absorbs it (`agents-ccd`, stub-track). CCD
lives inside the Substrate workspace as a first-class member (INTENT #23:
"they're all just apps in the repo").

**What wave-2 changes vs wave-1.** The charter's center of gravity moves from
"process supervisor + LLM egress" to **the Claude-Code usage-game broker**: the
declarative budget/priority **strategy language** evaluated against CCD's **own
usage ledger** is now the primary design concern, and its admission-under-budget-
pressure shape is deliberately **converged with `scheduler`'s admission design**
(the operator liked that convergence — same shape, different pressure axis).
Process supervision (wave-1's core) is retained and refit with **orphan
re-adoption across daemon restart**. New wave-2 surfaces: the `ccd-escalation`
receiver (DLQ / loop-depth investigation dispatch, INTENT #70/#89), thread↔
project↔environment linking in the ledger (INTENT #68), and `rollup`
consumption for on-demand plugin assembly.

## Charter

CCD is the daemon that **runs, meters, and governs cloud-code (Claude Code)
agent processes** on a machine and **plays the Claude-Code usage-limits game
well**. Its distinguishing job (INTENT #49) is spending a *constrained,
non-fungible* Claude Code budget — weekly limits, 5-hour session windows,
per-model/per-tier sub-limits — according to **rich declarative budget/priority
strategies** so that autonomous orgs can "access resources ongoing without
draining my usage." The operator's verbatim target strategy: *"if there's an
hour left in a session and tokens left, use all available session tokens as long
as it doesn't cross the 75th percentile of total weekly usage."* CCD owns: (1)
the **strategy language + budget-admission engine** — declarative rules
evaluated against the usage ledger to decide admit/throttle/defer/deny, honoring
a caller's `priority`; (2) its **own usage database** (INTENT #68 — sessions,
tokens, observed limits; thread records linkable to projects and optionally
environments; queried **pull-shaped** by `spend`, never pushed); (3) **process
supervision** of Claude Code subprocesses (spawn/track/signal/stream/reap +
orphan re-adoption across daemon restart); (4) a **stable agent-handle
namespace + registry** any caller addresses agents through; (5) the
**escalation-investigation dispatch** (receives DLQ-exhaustion and
loop-depth-exceeded incidents and spawns an investigating agent); and (6)
**on-demand plugin assembly** via `rollup` (specialized plugins for specialized
agents, no symlinks/copies).

**The routing answer (INTENT #40, LOCKED).** Claude Code calls are **NOT** routed
to local inference — Claude Code doesn't let you choose models, so there is no
`/v1/` redirect shim for CCD's Claude Code agents; they speak to Anthropic
natively and CCD governs *usage*, not *egress*. CCD **meters** Claude Code usage
from the subprocess's own structured output stream (turn-level input/output/cache
tokens Claude Code reports), NOT from inference. The `llm-calls`→inference edge
is **reserved metering-shaped** for the FUTURE `agents` umbrella (other agent
types CAN choose models and MAY use local inference); it is not on the Claude
Code path.

**Boundary — what CCD does NOT own.** CCD does not own inter-agent *conversation*,
org structure, manager/sub-agent hierarchies, pipelines, negotiation, or
metacognition — that entire layer is **Org**, built on top of CCD (`org-on-ccd`),
inbound-only (no reverse dependency). CCD manages *processes and their usage*;
Org manages *agents talking to each other*. CCD does not implement completion
scheduling, model residency, or the `/v1/` API (it is a consumer of inference,
never a peer runtime — no inference→CCD edge). It does not persist an org-graph
or knowledge graph. It owns no dashboard — it *emits* events and *publishes a
surface schema* for mesh's observability plane to render (`ccd-events`,
`surface-schema`). It does not compute *spend*: it stores usage and answers
queries; `spend` computes cost pull-shaped over `spend-ccd`. It does not author
the DLQ/loop-depth *conditions* (queues/execution-engine do) — it owns the
*receiver* and the investigation it spawns.

## Primary design concerns

These are the genuinely-hard parts that earn CCD its own crate.

### 1. The declarative strategy language + budget-admission engine — CENTRAL

This is the reason CCD is more than a process babysitter. A **strategy** is
declarative **DATA** (never code — mirroring INTENT #103's declarative triggers
and #101's trigger-as-data), a small ordered rule set evaluated against the usage
ledger to produce an **admission verdict** for a unit of agent work. The engine
is deliberately shaped like `scheduler`'s admission controller (scheduler.md
concern 1): scheduler lowers a *concurrency target* proportionally through a
soft-pressure band and admits nothing above hard pressure; **CCD lowers a
token/agent-admission target proportionally through a soft *budget*-pressure
band and defers above hard budget pressure.** The percentile guard is CCD's
analogue of scheduler's `effective_max_concurrent`-from-a-fitted-surface: a
*dynamic ceiling*, computed from the ledger, that caps admission. Same admission
shape, different axis (budget vs system pressure). This convergence is a design
lock, not a coincidence — see "The strategy language" section below for the full
grammar, evaluation model, and the verbatim-example walk-through.

The subtlety the operator cares about: **maximize utilization without draining.**
The default strategy must *burn down* aggressively where tokens are use-it-or-
lose-it (a session about to reset with tokens left) yet stay conservative against
the *weekly* limit, guarded by a rolling percentile of the operator's own
historical weekly usage so autonomous work never crosses into the budget the
operator wants reserved for himself.

### 2. CCD's own usage ledger / database (INTENT #68) — pull-shaped, provenance-first

CCD owns a **local SQLite** database (per-node local DB, INTENT #31; SQLite-only
locally, INTENT #98/#72). It is the *system of record* for sessions, per-turn
token usage, observed limit surfaces, and agent-run history — and the substrate
the strategy engine reads every admission tick. It is **queried pull-shaped** by
`spend` (INTENT #68 "spend can just query the cloud code daemon"; #41 pull-shaped)
and never pushes. Provenance is first-order (INTENT #85): every usage row carries
the `Provenance` triple (`types::Provenance`) and the `RollupProvenance` bill-of-
materials of the plugin the agent ran with, so "everything that led to the current
state" is traceable — what plugin, what fragments@version, what thread, what
project/environment, what caller/priority produced each token of spend. Thread
records link to projects and optionally to environments (INTENT #68), the
`ccd-projects` edge.

### 3. Cloud-code process supervision + orphan re-adoption (wave-1 core, refit)

Spawning Claude Code as a long-lived child process (working dir, initial
prompt/task, assembled plugin dir, env); tracking lifecycle (spawning → running →
idle → exited/failed); capturing and **parsing its structured output stream**
into agent-run events and **usage records** (turn-level tokens are the
authoritative metering signal — concern 1 depends on this); delivering
signals/input to a running agent; reaping/cleanup. Wave-2 adds **orphan
re-adoption across daemon restart**: CCD persists each spawned agent's OS pid +
a spawn cookie in the ledger and, on restart, re-adopts still-live children
(re-attaching to their output streams via a durable fifo/log file) rather than
orphaning or double-spawning them — the same discipline `supervision` applies to
service processes (supervision.md concern 6, zombie-killing), applied to *agent*
processes. This is OS-process-supervision work (backpressure on output streams,
crash recovery, orphan cleanup) and is why CCD is a daemon.

### 4. A stable handle namespace addressable from outside

Callers (Org, the dashboard via mesh, `spend`) address agents **by handle**.
Handles are stable, survive a CCD restart where the process survived (concern 3),
and map to live process state through a registry that is the single source of
truth for "who is running." This registry + its query/stream API is the surface
`agent-management` exposes.

### 5. Escalation-investigation dispatch (INTENT #70/#89) — the incident receiver

CCD receives `EscalationRequest`s (the shared `ccd-escalation` shape: DLQ-
exhaustion from `queues`, loop-depth-exceeded from `execution-engine`) and
**spawns an investigating agent** — a Claude Code agent, assembled from an
investigation plugin via rollup, handed the `correlation_id`/provenance so it can
trace the causal chain, and returning an `EscalationAck { Investigating { thread }}`.
This is the guardrail-of-last-resort hook: the union of the two conditions the
operator treats as one pattern. It rides the ordinary admission engine — an
investigation is agent work with a (high) priority like any other.

### 6. First-class mesh participation

Unlike a leaf service, CCD is *both* registrant and resolver
(`service-registration`, an instance of `service-lookup`): it registers its own
`slug -> host:port` so Org/spend/mesh find it, and resolves `inference`/`rollup`/
peers by slug. It participates in the cross-cutting mesh protocols like every
service: `restart-protocol` (its interruptibility is a function of live agent
work — it is `Uninterruptible` while agents burn budget mid-turn), `pubsub-
protocol` (its `ccd-events` stream), `queues-api` (the escalation is delivered as
a handler payload), `locks-api` (single-writer guard on the ledger's limit
surfaces across nodes), and `cron-api` (the periodic percentile/limit recompute
and session-reset roll).

## The strategy language

The heart of the wave-2 refit. Declarative, composable, evaluated against the
ledger; expressed over budget/limit/percentile/time-remaining/priority terms.

### Data model (declarative — registered as data, INTENT #101/#103)

A `Strategy` is an ordered list of `Rule`s plus a fallback. Each `Rule` binds a
boolean `when` predicate to an `AdmissionVerdict`, optionally narrowed by one or
more `guard` clauses that *cap* the verdict. The engine evaluates rules top-down;
the first rule whose `when` holds wins, its verdict is then reduced by the min
across its guards, and the result is the admission decision. Strategies are keyed
per caller/priority-class and stored as data (in the ledger, replicated where
relevant) — never compiled code.

```rust
// types::ccd  (proposed — see contracts section)
pub struct Strategy {
    pub id: StrategyId,
    pub scope: StrategyScope,          // Default | Caller(Slug) | PriorityClass(u8) | Named(String)
    pub rules: Vec<Rule>,
    pub fallback: AdmissionVerdict,    // when no rule matches (default: Defer{ retry_after })
}
pub struct Rule {
    pub when: Predicate,
    pub verdict: AdmissionVerdict,
    #[serde(default)] pub guards: Vec<Guard>,   // each caps the verdict; engine takes the min
    #[serde(default)] pub note: String,         // human/agent-readable rationale (shows in provenance)
}

// A pure expression tree over ledger-derived TERMS — no side effects, no I/O, total.
pub enum Predicate {
    Cmp { lhs: Term, op: CmpOp, rhs: Term },
    And(Vec<Predicate>), Or(Vec<Predicate>), Not(Box<Predicate>),
    Always,
}
pub enum Term {
    // literals
    Tokens(u64), Duration(std::time::Duration), Ratio(f64), Priority(u8),
    // ledger reads (computed pull-shaped from the usage DB at eval time)
    SessionTimeRemaining, SessionTokensRemaining, SessionTokensUsed, SessionLimit,
    WeeklyTokensUsed, WeeklyTokensRemaining, WeeklyLimit,
    WeeklyPercentile(u8),              // p-th percentile of ROLLING historical weekly usage
    ModelTierRemaining(ModelTier),     // per-model / per-tier sub-limit (e.g. Opus weekly)
    RequestPriority, RequestEstimatedTokens, CallerIs(Slug),
    // arithmetic over terms (keeps the verbatim example expressible)
    Add(Box<Term>, Box<Term>), Sub(Box<Term>, Box<Term>), Mul(Box<Term>, f64),
}
pub enum CmpOp { Lt, Le, Gt, Ge, Eq, Ne }

pub enum AdmissionVerdict {
    Admit  { token_ceiling: TokenCeiling, max_parallel: u32 },
    Throttle { of: Box<AdmissionVerdict>, factor: f64 },  // proportional lowering (soft band)
    Defer  { retry_after: std::time::Duration },          // re-evaluate later (hard band)
    Deny   { reason: String },                            // structural refusal
}
pub enum TokenCeiling { UpTo(u64), AllRemainingSession, AllRemainingWeekly, Unbounded }

// A guard caps a verdict's token issuance so a rule can say "…as long as it
// doesn't cross X". The engine reduces the winning verdict's ceiling to the min
// over all satisfied guards' caps; a violated guard forces Defer.
pub struct Guard { pub ceiling: Term /* an upper bound on cumulative tokens */,
                   pub on_violation: GuardAction }
pub enum GuardAction { CapToCeiling, Defer, Deny }
```

### Evaluation model (the convergence with scheduler)

Per admission request (`AdmitRequest { estimated_tokens, priority, caller, thread }`),
the engine, each tick:

1. **Reads the ledger** to bind every `Term` (session/weekly/per-tier state +
   the rolling `WeeklyPercentile`). These reads are the CCD analogue of
   scheduler reading `SystemState`.
2. **Finds the first matching rule**, takes its verdict.
3. **Reduces by guards** — computes `min(verdict_ceiling, guard_ceiling…)`; a
   `CapToCeiling` guard lowers the token ceiling *proportionally* (the soft
   band — same move as scheduler shrinking the concurrency target in the
   0.90–0.95 pressure band); a violated `Defer`/`Deny` guard forces the hard
   outcome (scheduler's "admit nothing above hard pressure").
4. **Emits the verdict** — Admit issues a `BudgetGrant` (a token allowance +
   parallelism cap the supervisor enforces on the spawned agent); Throttle
   lowers it; Defer parks the request on a priority queue re-evaluated on the
   next tick / on a `cron`-driven limit-recompute / on session reset; Deny fails
   the request to the caller.

The **budget-pressure fraction** CCD classifies is `weekly_used /
weekly_percentile_ceiling` (and, independently, `session_used / session_limit`),
directly paralleling scheduler's memory-pressure fraction. A single shared
"admission-under-pressure" mental model spans both crates — flagged for the
harmonizer as an intentional convergence (candidate shared vocabulary
`types::admission` at the closing extract-shared-libraries pass, INTENT #25;
NOT extracted now, only flagged).

### The verbatim example, expressed

*"if there's an hour left in a session and tokens left, use all available session
tokens as long as it doesn't cross the 75th percentile of total weekly usage."*

```jsonc
// Strategy { scope: Default, rules: [ … ] }  — the burn-down rule:
{
  "when": { "And": [
     { "Cmp": { "lhs": "SessionTimeRemaining", "op": "Le", "rhs": { "Duration": "1h" } } },
     { "Cmp": { "lhs": "SessionTokensRemaining", "op": "Gt", "rhs": { "Tokens": 0 } } }
  ]},
  "verdict": { "Admit": { "token_ceiling": "AllRemainingSession", "max_parallel": 4 } },
  "guards": [
     { "ceiling": "WeeklyPercentile(75)", "on_violation": "CapToCeiling" }
  ],
  "note": "use-it-or-lose-it session burn-down, weekly-75th-percentile guarded"
}
```

Read exactly as the operator said it: *when* under an hour of session remains and
session tokens remain, admit up to **all** remaining session tokens, *guarded* so
cumulative weekly usage never crosses the **75th percentile** of the operator's
rolling weekly-usage history — the guard caps the grant the moment `weekly_used +
grant` would exceed that percentile, converting "spend it all" into "spend up to
the reserve line," which is precisely *maximize utilization without draining*.
Higher-priority callers (Org's autonomous work) get a strategy with a more
aggressive percentile (e.g. 90) or an explicit `PriorityClass` scope; the
operator's own interactive work is effectively priority-max and unguarded.

## The usage ledger (schema sketch)

Local SQLite (`ccd.db`), owned by the `ledger` lib. Boring, provenance-first,
pull-queryable.

- **`sessions`** — `(session_id, window_start, window_end, session_limit_tokens,
  observed_at, node_id)`. The 5-hour Claude Code window; `session_limit_tokens`
  is *observed* (learned from CC's limit signals), not assumed.
- **`weekly_windows`** — `(week_start, weekly_limit_tokens, tokens_used_cache,
  observed_at)`. Rolling-window rows feed `WeeklyPercentile(p)`.
- **`model_tiers`** — `(tier, sub_limit_tokens, window, observed_at)` — per-model
  / per-tier limits (e.g. Opus cutting-edge sub-limit).
- **`agent_runs`** — `(handle, pid, spawn_cookie, thread_id, project_id?,
  environment_id?, caller_slug, priority, plugin_provenance /*RollupProvenance*/,
  strategy_id, budget_grant, state, started_at, ended_at, exit)`. The re-adoption
  anchor (pid + spawn_cookie) and the project/environment links live here.
- **`usage_records`** — `(id, handle, turn_seq, input_tokens, output_tokens,
  cache_read_tokens, cache_write_tokens, model, occurred_at, provenance)`. One
  row per Claude Code turn, parsed from the CC output stream — the authoritative
  metering signal and the atoms `spend` aggregates.
- **`strategies`** — the declarative `Strategy` rows (data, not code).
- **`escalations`** — `(escalation_id, kind, correlation_id, spawned_handle?,
  ack, received_at)` — the `ccd-escalation` receiver's dedup + audit table.

`spend` reads `usage_records` ⨝ `agent_runs` (grouped by project/environment/
caller) over `spend-ccd`. Provenance on every row satisfies INTENT #85/#92.

## Relationships / edges

- **cloud-code agents** via `agent-management` (scaffold/contracts/agent-management.md)
  — spawn/track/signal/stream/reap by stable handle; the multi-agent substrate
  Org layers on. **I author this edge (wave 2).**
- **mesh.queues(DLQ) / execution-engine** via `ccd-escalation`
  (scaffold/contracts/ccd-escalation.md — MISSING; queues proposed the DLQ half,
  execution-engine the loop-depth arm) — **I own the receiver + `EscalationAck`
  (wave 2).**
- **rollup** via `rollup-ccd` (scaffold/contracts/rollup-ccd.md) — on-demand
  plugin assembly (`Materialize`/`AssemblePlugin`); rollup authored its side, I
  propose the **consumer view (wave 2).**
- **inference (api)** via `llm-calls` (scaffold/contracts/llm-calls.md) —
  **NOT the Claude Code path** (INTENT #40: no local-inference routing for Claude
  Code). Reserved metering-shaped for the FUTURE `agents` umbrella's non-CC agent
  types. I propose the metering-shaped view; inference/api authors the `/v1`
  surface it reuses (`v1-completion-api`).
- **mesh.service-registry** via `service-registration`, an instance of
  `service-lookup` (scaffold/contracts/service-registration.md) — CCD registers
  its own endpoint and resolves dependencies by slug; static-fallback on a single
  box.
- **mesh** via `ccd-events` (scaffold/contracts/ccd-events.md) — agent-lifecycle/
  run/usage WS event stream over `pubsub-protocol`; mesh's observability plane
  aggregates it (mirrors `inference-events`/`gc-events`). **Round-9: strike the
  wave-1 "proposed, pending confirmation" marker — confirmed in scope** (flag #6
  in wave2-plan §5). I author.
- **surface-schema** (scaffold/contracts/surface-schema.md) — CCD publishes its
  observable-surface schema (budget/limit meters, agent roster, strategy set,
  escalation feed) for the mesh dashboard to render schema-driven.
- **org** via `org-on-ccd` (scaffold/contracts/org-on-ccd.md) — Org is the
  downstream maximalist consumer; inbound-to-CCD only, DEFERRED/open (Org is the
  maximalist-consumer exception). Callers like Org call CCD **with a priority**
  the budget engine honors. I record the shaped-for surface; content deferred.
- **projects** via `ccd-projects` (MISSING; stub-track) — thread↔project (+
  optional environment) linkage stored in CCD's ledger. Anticipated; projects is
  L6-stub, content deferred.
- **spend** via `spend-ccd` (MISSING; stub-track) — pull-shaped spend queries of
  the usage ledger. Anticipated; spend is L6-stub, content deferred.
- **agents** via `agents-ccd` (MISSING; stub-track) — the future generalization
  layered on CCD (never absorbs it). Anticipated; content deferred.
- **Cross-cutting mesh protocols** (surface-schema-style, one stub many parties —
  I am a party, I do not author): `pubsub-protocol` (ccd-events transport),
  `restart-protocol` (interruptibility = f(live agent work)), `queues-api`
  (escalation handler delivery), `locks-api` (single-writer on limit surfaces),
  `cron-api` (periodic limit/percentile recompute + session-reset roll).

## Nesting (if applicable)

Parent: none (top-level L5 app). Children: none at the scaffold level. The `ccd`
crate decomposes into **libraries within the crate** (not separate crates — they
are not independently apps, INTENT #22):

- `supervisor` — process spawn/track/signal/reap, output-stream capture & parse
  (into agent-run events + `usage_records`), orphan re-adoption on restart.
- `strategy` — the declarative strategy language: parse/validate `Strategy` data,
  the pure `Predicate`/`Term` evaluator, and the admission engine (concern 1).
  **The one hard lib** — kept trait-shaped (`AdmissionEngine`) so the pressure-
  band logic stays testable in isolation and converges with scheduler's shape.
- `ledger` — the SQLite usage database + its pull-query surface for `spend`;
  provenance-first schema (concern 2).
- `registry` — the stable handle namespace + live-agent state (concern 4).
- `escalation` — the `ccd-escalation` receiver + investigation-agent dispatch
  (concern 5).
- `control` — the HTTP/WS control API implementing `agent-management`, the
  `org-on-ccd` consumption surface, `ccd-events` emission, and `surface-schema`
  publication.
- `mesh_client` — thin `service-lookup`/restart-protocol/pubsub client (compiles
  in `lib/mesh-client`, INTENT #45 — not re-hand-rolled).
- `config` — daemon config + static-fallback endpoints.

**Crate = app (CLI + daemon split).** ONE app, two faces of one binary, mirroring
`bin/gc`/`bin/db`/`bin/mesh`: `bin/ccd` provides the daemon (`ccd serve`) and a
thin noun-verb **client CLI** (`ccd agent spawn|list|logs|send|kill`,
`ccd budget show`, `ccd strategy set|list`, `ccd usage query`) whose subcommands
are HTTP calls into the running daemon. Real logic in `lib/ccd`; the binary is a
thin entry point. Workspace members: `bin/ccd` + `lib/ccd`.

**Shared-library eye (flag for the dedup pass, INTENT #25 — do not build now).**
(a) the `v1-completion-api` client (reqwest wrapper) is shared with mesh's
observability plane and Org → extraction target `substrate-api-client`; (b)
`service-lookup`/restart/pubsub client is already `lib/mesh-client` (consume it);
(c) the **admission-under-pressure** vocabulary is a genuine convergence with
`scheduler` → candidate `types::admission` at the closing extract pass. Keep
CCD's copies thin and obviously-extractable; do not gold-plate.

## Thoroughness level

**implementation-ready** for the crate shape, CLI/daemon split, internal module
decomposition, the usage-ledger schema, process supervision (incl. orphan
re-adoption), the routing answer, edges, and mesh participation. The
**strategy-language grammar and admission engine are now specified**
(`Strategy`/`Rule`/`Predicate`/`Term`/`AdmissionVerdict`/`Guard`, the evaluation
model, and the verbatim example expressed) — moved from wave-1's
`requirements-only` to **approach-sketched-toward-implementation-ready**: the
grammar is concrete and the scheduler-convergence is pinned; what a Filler still
tunes is the *default strategy set* and the exact percentile-window mechanics
(rolling-window size, warm-start before enough history exists — see open
questions), not the language shape.

## Assigned design-depth

Opus 4.8 — single-agent Component Designer, grounded on the wave-1 `ccd.md`, the
governing INTENT items (#40/#49/#68/#70/#89/#104), and the batch-1→5 neighbor
designs (`types`, `queues` [escalation + trigger data model], `scheduler`
[admission convergence], `rollup` [rollup-ccd/materialize], `supervision`
[restart/interruptibility], `service-registry`, `api`/`inference` [llm-calls]).
CCD is net-new (no prior repo code), so grounding is intent-capture-grade for
scope and repo-grade for shape/conventions. NOT a Design Mesh run.

## Suggested fill-model

**implementation-ready-ish, moderate-to-high complexity → strong-ish model, no
Design Mesh required.** The one part warranting the stronger model at fill is the
`strategy` lib (the declarative evaluator + admission-under-pressure engine); the
supervisor/ledger/registry/escalation/control/CLI libs can be filled with a
mid-tier model against this design. Do NOT hand the whole crate to a cheap model
uncritically — process supervision (orphan re-adoption) and the guard-reduction
math in the admission engine regress silently without rigor. Fill `strategy` with
a shared understanding of `scheduler`'s admission (co-read scheduler.md concern 1)
so the convergence is real in code, not just on paper.

## Proposed contracts (wave 2)

Proposals only — the per-pair round reconciles both sides. I do NOT edit
`scaffold/contracts/*`. Shared vocabulary lands in `types::ccd`
(`CcdError` in `types::error::ccd`). Reused from `types`: `Provenance`,
`Event`, `EventType`, `Interruptibility`, `RestartPriority`; from `types::rollup`:
`PluginManifest`, `RollupProvenance`, `MaterializeResult`, `CallerContext`,
`SlotMap`, `ScopeChain`, `OutputSink`.

Shared vocabulary I introduce:

```rust
// types::ccd
pub struct AgentHandle(pub String);      // stable, survives CCD restart if the process survives
pub struct ThreadId(pub String);
pub struct StrategyId(pub String);
pub enum ModelTier { Cutting(String), Standard(String), Fast(String) }  // open string inside — never a closed model enum
pub struct BudgetGrant { pub token_ceiling: u64, pub max_parallel: u32,
                         pub strategy_id: StrategyId, pub expires_at: DateTime<Utc> }
// Strategy / Rule / Predicate / Term / AdmissionVerdict / Guard — see "The strategy language".
pub struct AdmitRequest { pub estimated_tokens: u64, pub priority: u8,
                          pub caller: CallerContext, pub thread: Option<ThreadId> }
pub enum AdmissionOutcome { Admitted(BudgetGrant), Throttled(BudgetGrant),
                            Deferred { retry_after: Duration }, Denied { reason: String } }
```

```rust
// types::error::ccd
pub enum CcdError {
    UnknownHandle(String),
    SpawnFailed { reason: String },
    AgentExited { handle: String, code: Option<i32> },
    StrategyInvalid { message: String },       // parse/validate of declarative Strategy data
    BudgetExhausted { window: String },        // hard-pressure defer surfaced as error where sync
    LimitUnobserved { surface: String },       // no ledger data yet to bind a Term (warm-start)
    Rollup(String),                            // stringified rollup-side failure — never becomes a rollup type
    Inference(String),                         // stringified — future agents path only
    Registry(String),
}
```

### `agent-management` (ccd ↔ cloud-code agents) — I author

- **Purpose.** Spawn/track/signal/stream/reap Claude Code processes by stable
  handle; the surface Org and the dashboard build on. Every spawn carries a
  `BudgetGrant` (from the strategy engine) the supervisor enforces.
- **Message/struct sketch** (control API over mesh WS + the thin CLI):
  ```rust
  // caller -> ccd
  enum AgentCmd {
      Spawn  { task: String, workdir: PathBuf, plugin: Option<PluginManifest>, slots: SlotMap,
               caller: CallerContext, priority: u8, thread: Option<ThreadId>,
               project: Option<ProjectId>, environment: Option<EnvironmentId> }, // -> Result<AgentHandle, CcdError>
      Send   { handle: AgentHandle, input: String },        // deliver input/signal to a running agent
      Signal { handle: AgentHandle, sig: AgentSignal },      // Interrupt | Pause | Resume | Terminate
      List   { filter: AgentFilter },                        // -> Vec<AgentStatus>
      Logs   { handle: AgentHandle, from: Option<u64> },     // -> stream of AgentEvent
  }
  enum AgentSignal { Interrupt, Pause, Resume, Terminate }
  struct AgentStatus { handle: AgentHandle, state: AgentState, thread: Option<ThreadId>,
                       usage: TurnUsageSummary, grant: BudgetGrant }
  enum AgentState { Spawning, Running, Idle, Exited { code: Option<i32> }, Failed { reason: String } }
  // ccd -> caller (also the ccd-events payloads)
  enum AgentEvent { Started { handle: AgentHandle }, TurnCompleted { handle: AgentHandle, usage: TurnUsage },
                    ReportEmitted { handle: AgentHandle, path: PathBuf }, StateChanged { handle: AgentHandle, state: AgentState } }
  ```
  When `plugin` is present, ccd first calls `rollup-ccd` to materialize it, then
  spawns Claude Code pointed at the materialized dir; the `RollupProvenance` is
  recorded on the `agent_runs` row.
- **Error cases.** `SpawnFailed`, `UnknownHandle`, `AgentExited`,
  `BudgetExhausted`/`Deferred` (spawn is admission-gated: a `Spawn` that the
  strategy defers returns `Deferred{retry_after}`, not a live handle).
- **Version-sensitivity.** MEDIUM. `AgentCmd`/`AgentEvent`/`AgentState` are
  wire-crossing (Org, dashboard) → additive-only, `#[serde(other)]` on every enum,
  `#[serde(default)]` on new fields. Handle format is an open string.

### `ccd-escalation` (mesh.queues(DLQ) / execution-engine → ccd) — I own the receiver

- **Purpose.** The single investigation surface for the two conditions INTENT
  #70/#89 treat as one pattern: queues **dead-letter exhaustion** and
  execution-engine **loop-depth-exceeded**. I own the **receiver + `EscalationAck`**;
  queues authors the `DeadLetter` arm, execution-engine the `LoopDepthExceeded`
  arm (agreed in queues.md's `ccd-escalation` proposal — I adopt its struct
  verbatim so the union is one shape).
- **Struct sketch** (adopted from queues.md; receiver-side additions marked):
  ```rust
  struct EscalationRequest {                    // authored jointly (queues + execution-engine arms)
      escalation_id: Uuid,
      kind: EscalationKind,
      correlation_id: Option<Uuid>,             // provenance root — the causal chain to investigate
      provenance: Provenance,
      context: serde_json::Value,               // failure_history / chain snapshot for the agent
  }
  enum EscalationKind {
      DeadLetter { queue: QueueName, dlq: QueueName, trigger_id: TriggerId,
                   event_id: Uuid, receive_count: u32, last_error: String },   // queues authors
      LoopDepthExceeded { engine: Slug, depth: u32, threshold: u32 },          // execution-engine authors
      // #[serde(other)] reserved so a future third kind never breaks ccd's deserialize
  }
  enum EscalationAck { Investigating { thread: ThreadId }, Declined { reason: String } }  // I OWN this
  ```
- **Receiver behavior (mine).** Dedup on `escalation_id` (the `escalations`
  table — an escalation may arrive twice under at-least-once, INTENT #95). On a
  new escalation: assemble the investigation plugin via `rollup-ccd`, spawn a
  high-priority investigating agent through the ordinary admission engine, thread
  the `correlation_id`/`context`, and return `Investigating { thread }`. If the
  strategy defers under hard budget pressure, return `Declined { reason }` (the
  escalation re-delivers / retains on the DLQ — never lost silently).
- **Error cases.** `CcdUnreachable` (offline → the escalation dead-letters on the
  DLQ's own retention, alarmed). Delivery is `queues-api` at-least-once +
  `escalation_id` idempotency.
- **Version-sensitivity.** MEDIUM — matches queues.md. `EscalationKind` additive
  with `#[serde(other)]`; `context` open `Value`. **Ownership for the per-pair
  round: queues = `DeadLetter`, execution-engine = `LoopDepthExceeded`, ccd =
  receiver + `EscalationAck`.**

### `rollup-ccd` (ccd → rollup) — consumer view; rollup authored the shape

- **Purpose.** On-demand plugin assembly: a `PluginManifest` + runtime slots →
  a materialized plugin dir (Claude-Code layout), no symlinks/copies. I consume
  rollup.md's authored `AssemblePlugin`/`AssembleResult` (the `Materialize` shape).
- **Consumer-side contract.** I send `AssemblePlugin { manifest, runtime_slots,
  scope, output: OutputSink::LocalDir(dir) }` and receive `AssembleResult {
  result: MaterializeResult, plugin_json }`; I record `result.provenance`
  (fragment id:version + content_hash per entry) on the agent's `agent_runs` row
  as its bill-of-materials (exact reproducibility of what an agent ran with).
- **Invariants I rely on.** A missing fragment/slot fails the *whole* assembly
  (a plugin is all-or-nothing) → I surface `Rollup(..)` to the caller/priority
  owner and do NOT spawn. A degraded secret is a warning in provenance, never a
  value in the plugin (INTENT #94 — rollup's `rollup-secrets` guarantees no secret
  reaches the agent).
- **Version-sensitivity.** MEDIUM — additive per rollup.md; the Claude-Code
  output layout (`skills/…`, `agents/…`, `commands/…`, `plugin.json`) is stable.

### `llm-calls` (ccd → inference) — metering-shaped; RESERVED for future agents

- **Purpose.** **NOT the Claude Code path** (INTENT #40 LOCKED). Two clarifications
  this edge must carry so the harmonizer doesn't mis-wire it: (a) Claude Code
  usage is metered from the **CC subprocess output stream** (`agent-management`
  `TurnUsage`), NOT via inference — inference sees no Claude Code traffic; (b)
  the edge is reserved for the FUTURE `agents` umbrella's non-CC agent types,
  which CAN choose models and MAY route completions to local inference — those
  reuse `v1-completion-api` (api.md's fleet-facing surface), and CCD meters that
  usage into the same ledger.
- **Metering-shaped view.** When a future non-CC agent's completion goes through
  `/v1/`, CCD records a `usage_records` row from the response usage block; no new
  inference surface is introduced (reuse `v1-completion-api`). No content this
  wave beyond the reservation + the metering shape.
- **Version-sensitivity.** LOW (reserved). Flag for the per-pair round: keep
  `llm-calls` a thin alias of `v1-completion-api` + a metering note; do not
  author a Claude-Code-specific completion surface (there is none).

### `service-registration` (ccd ↔ mesh.service-registry) — instance of `service-lookup`

- **Purpose.** CCD registers its own `slug=ccd -> host:port` and resolves
  `inference`/`rollup`/peers by slug; first-class registrant AND resolver.
  Instance of the authored `service-lookup`; no new schema — CCD uses
  `lib/mesh-client`'s `register`/`resolve`. Static-fallback on a single box.
- **Version-sensitivity.** LOW — rides `service-lookup`/`mesh-client` verbatim.

### `ccd-events` (mesh ← ccd) — agent-lifecycle/usage WS stream — I author

- **Purpose.** CCD emits an agent-lifecycle/run/usage event stream mesh's
  observability plane aggregates (mirrors `inference-events`/`gc-events`).
  **Round-9: the wave-1 "proposed, pending confirmation" marker is STRUCK —
  confirmed in scope** (wave2-plan §5 flag #6).
- **Message/struct sketch** (typed `Event<P>` payloads over `pubsub-protocol`,
  `types` envelope):
  ```rust
  // event_type: "ccd.agent.started" | ".turn_completed" | ".exited"
  //           | "ccd.budget.throttled" | "ccd.budget.deferred" | "ccd.limit.observed"
  //           | "ccd.escalation.received" | "ccd.escalation.investigating"
  enum CcdEventPayload {
      Agent(AgentEvent),
      Budget { verdict: AdmissionOutcome, window: String, pressure: f64 },  // pressure = used/ceiling
      LimitObserved { surface: String, limit_tokens: u64 },
      Escalation { escalation_id: Uuid, kind_tag: String, thread: Option<ThreadId> },
  }
  ```
- **Error cases.** None on the emit path (lossy broadcast — a dropped event never
  blocks agent work; mesh's fan-out reconciles per-node subscription).
- **Version-sensitivity.** MEDIUM — dashboard/observability consumers → additive-
  only; event `payload` is `Value` at the generic relay/filter sites (types.md).

### `org-on-ccd` (org → ccd) — shaped-for, DEFERRED (Org is the exception)

- **Purpose.** Org is the maximalist downstream consumer; inbound-to-CCD only.
  Content DEFERRED (Org is un-designed this pass). Recorded so `agent-management`,
  `ccd-events`, and the priority-honoring admission engine are shaped for a broad
  consumer from the start: **callers pass a `priority` the strategy engine
  honors** (`AdmitRequest.priority`, `AgentCmd::Spawn.priority`), and Org reads
  the whole agent roster + budget state through `agent-management`/`surface-schema`.
- **Version-sensitivity.** N/A (deferred). No schema authored.

### Stub-track anticipated contracts (content deferred — L6 stubs)

- **`ccd-projects` (ccd ↔ projects).** thread↔project(+optional environment)
  linkage. **What it will carry:** `agent_runs.project_id?`/`environment_id?`
  are set at spawn (from `AgentCmd::Spawn`) and validated against projects when
  projects leaves the stub track; projects reads CCD's per-project agent/usage
  rollup. No new mechanism — fields already in the ledger + `agent-management`.
  Content deferred (INTENT #51/#68).
- **`spend-ccd` (spend → ccd).** pull-shaped spend queries. **What it will
  carry:** a read-only query surface over `usage_records ⨝ agent_runs` grouped by
  project/environment/caller/window (`UsageQuery -> UsageRollup`), returning token
  counts + provenance; spend computes cost. Sources never push (INTENT #41/#68).
  Content deferred.
- **`agents-ccd` (agents → ccd).** the future generalization layered on CCD.
  **What it will carry:** the `agents` umbrella manages non-CC agent types on top
  of CCD's process-supervision + admission engine, reusing `agent-management` and
  adding model-choice/inference-routing (the `llm-calls` path). Never absorbs CCD
  (INTENT #49). Content deferred.

## Non-obvious tests (conformance + correctness)

- **The verbatim strategy, end-to-end (the critical one):** with a ledger where
  `session_time_remaining = 45m`, `session_tokens_remaining = 200k`, and
  `weekly_used` sits `30k` below `WeeklyPercentile(75)`, an `AdmitRequest`
  yields `Admitted` with `token_ceiling == 30k` (the guard caps
  `AllRemainingSession` down to the reserve line) — NOT 200k, and NOT 0. Move
  `weekly_used` above the 75th percentile and the same request `Deferred`s.
  Directly encodes "use all session tokens as long as it doesn't cross the 75th
  percentile."
- **Admission convergence with scheduler:** in the soft budget-pressure band the
  granted `token_ceiling` decreases *monotonically and proportionally* as
  `weekly_used/ceiling` rises (the same proportional-lowering shape scheduler
  applies in 0.90–0.95); above the hard line, admission is exactly `Deferred`
  (scheduler's "admit nothing above hard"). Property test the monotonicity.
- **Strategy is data, not code:** a `Strategy` round-trips through JSON and back
  and evaluates identically; an invalid `Strategy` (unknown `Term`, malformed
  `Predicate`) fails `StrategyInvalid` at *registration* (`ccd strategy set`),
  never at admission time — the evaluator is total over valid strategies.
- **Warm-start / unobserved limits:** with an empty ledger (no weekly history),
  `WeeklyPercentile(75)` binding surfaces `LimitUnobserved` and the engine falls
  back to the `Strategy.fallback` (conservative `Defer`/small fixed ceiling) —
  it never divides by zero or admits unbounded on missing data.
- **Orphan re-adoption across daemon restart:** spawn an agent, hard-kill the CCD
  daemon, restart it → the still-live Claude Code child is re-adopted by
  (pid, spawn_cookie), its handle is stable, its output stream re-attaches from
  the durable log, and NO second copy is spawned (the agent-side analogue of
  supervision's zombie-killing).
- **Metering authority is the CC output stream, not inference:** a Claude Code
  agent completing turns produces `usage_records` rows parsed from its own output
  with zero traffic on `/v1/` — assert inference sees no completion for a
  Claude Code agent (the INTENT #40 routing lock, made observable).
- **Escalation dedup + admission-gated:** the same `EscalationRequest` delivered
  twice spawns exactly one investigating agent (`escalation_id` dedup); under
  hard budget pressure the escalation returns `Declined` and the DLQ retains it
  (never lost) rather than spawning past the limit.
- **Provenance completeness for spend:** every `usage_records` row joins to an
  `agent_runs` row carrying `RollupProvenance` (plugin bill-of-materials) +
  project/environment/caller — a `spend-ccd` rollup can attribute every token to
  a (project, environment, plugin@version, thread) with no orphan usage.
- **Priority is honored:** two simultaneous `Spawn`s under the same tight budget,
  priorities 9 and 1, admit the priority-9 work first and `Defer` the priority-1
  work when only one fits under the guard — the caller-priority path Org depends
  on.
