# ccd

**Status:** NEW (Cloud Code Daemon; aliases Marshall / CCM). **Nesting:**
top-level, flat (no scaffold-level children — internal parts are libraries within
the crate, per the repo's crate=app philosophy). **Round-3 (2026-07-18): CCD
STAYS top-level — explicitly NOT merged into any `agents` umbrella.** A
generalized `agents` layer is a named FUTURE placeholder only (see
`overview.md`'s future section); if it ever exists it is layered ON TOP of CCD,
it does not absorb it. **Build order: FIRST of the new
concepts, before Org.** **Siting: RESOLVED — lives inside the Substrate workspace
as a first-class member** (see Charter note; the overview's "CCD siting" open
question is now stale — operator answered it directly in the reconstruction
thread: *"The CCD program should also live in the same substrate repo… They're
all just apps in the repo. I don't know why that's even a question."*).

## Charter

CCD is the daemon that runs and governs **cloud-code (Claude Code) agent
processes** on a machine. Its **distinguishing job (round-3 lock, 2026-07-18)
is playing the Claude-Code-usage-limits game well** — weekly limits, session
limits, per-model limits — with **rich declarative budget/priority strategies**.
Operator's example, verbatim: "if there's an hour left in a session and tokens
left, use all available session tokens as long as it doesn't cross the 75th
percentile of total weekly usage." Callers like `org` may call CCD directly
**with a priority**; CCD spends the constrained Claude Code budget according to
the declared strategy. It owns: (1) the **process lifecycle** of
agent subprocesses — spawn, track, signal, stream output, reap (`agent-management`);
(2) a **stable agent-handle namespace + registry** so any caller can address a
live agent by handle and read its status/output; and (3) the **usage-limits
budget engine** — metering every agent's Claude Code usage against the
weekly/session/per-model limit surfaces and scheduling/throttling agent work by
declared strategy + caller priority; and — **LOCKED rounds 4–5 (2026-07-18)** —
(4) **its own usage database**: CCD tracks sessions, tokens, and limits in a
database it owns ("the cloud code daemon should have its own database where
it's tracking all that stuff"). The `spend` component (the renamed `finance`
placeholder — see `overview.md`) **queries CCD** (and other spend sources) for
this data; CCD does not push to spend. Also locked: **threads must be linkable
to projects, and optionally to environments within projects** (see
`components/environments.md`) — the usage database's thread records carry
those links. **NEW round-6 (2026-07-19): CCD consumes the `rollup` system for
its plugin/prompt assembly** — specialized plugins for its specialized
agents, built from fragments and generated on demand ("CCD ideally would be
built on top of this prompt rollup"); see `components/rollup.md` and the
`rollup-ccd` edge below.

> **SUPERSEDED (2026-07-18): "routes agents' LLM calls to the local inference
> fleet."** The previously-flagged BIG open question (route agents' LLM calls
> to local inference?) is **answered NO for Claude Code specifically** — Claude
> Code doesn't let you choose models, so there is no redirect-to-`/v1/` shim
> for CCD's agents; they speak to Anthropic natively and CCD governs *usage*,
> not *egress*. A future `agents` generalization layered on top of CCD may use
> local inference for OTHER agent types — that is where the old routing-shim
> design content becomes relevant again, not in CCD. The `llm-calls` edge
> narrows accordingly (usage-observation/metering, not routing).

It registers itself and resolves its dependencies through
the mesh service-registry (`service-registration`). CCD is "run all cloud code
through the Marshall": the single, observable choke point every agent process and
every unit of Claude Code budget passes through.

**Boundary — what CCD does NOT own.** CCD does **not** own inter-agent
*conversation*, org structure, manager/sub-agent hierarchies, pipelines, or
metacognition — that entire layer is **Org**, which is *built on top of* CCD and
consumes it (`org-on-ccd`). CCD manages *processes and their model calls*; Org
manages *agents talking to each other*. CCD also does **not** implement
completion scheduling, model residency, or the `/v1/` API — it is a **consumer**
of inference, never a peer runtime (no reverse dependency from inference to CCD).
It does not persist an org-graph or knowledge graph (that is Org-on-`db`). It
owns no dashboard; it *emits* events for mesh's observability plane to
aggregate (and publishes its surface schema), but rendering
is the dashboard's job. Keeping CCD bounded to "process supervisor + LLM egress
for agents" is what lets Org be the maximalist consumer without CCD bloating into
it.

## Primary design concerns

These are the genuinely-hard parts that earn CCD its own crate rather than
clumping into mesh or inference:

1. **Cloud-code process supervision (the core).** Spawning Claude Code as a
   long-lived child process with a working dir, an initial prompt/task, and an
   environment; tracking its lifecycle (spawning → running → idle → exited/failed);
   capturing and **parsing its structured output stream** into agent-run events
   ("agent started", "turn completed", "agent exited", report emitted); delivering
   signals/input to a running agent; and reaping/cleanup. The operator's captured
   intent is explicit that agents dump output/reports that the harness watches and
   picks up — so output ingestion is a first-class concern, not an afterthought.
   This is OS-process-supervision work (zombies, backpressure on output, crash
   recovery, orphan cleanup on daemon restart) and is why CCD is a daemon.

2. **The usage-limits budget engine (the subtle part — RESHAPED by the round-3
   resolution).** The old design here was an LLM-routing shim (redirect agents'
   calls to local inference via a pluggable completion backend). That fork is
   **resolved NO for Claude Code** (see the superseded note in the Charter):
   agents talk to Anthropic natively, and CCD's subtle part becomes the
   **declarative budget/priority strategy engine**: modeling the
   weekly / session / per-model limit surfaces, metering per-agent usage
   against them, and evaluating declarative strategies (e.g. the operator's
   75th-percentile-of-weekly example) to decide when to run, throttle, or defer
   agent work — honoring the priority a caller (e.g. `org`) attaches. The
   strategy language itself is `requirements-only` this round: declarative,
   composable, expressed over budget/limit/percentile/time-remaining terms;
   shape undesigned. The old pluggable-backend routing design is parked for a
   future `agents` generalization (other agent types CAN choose models and may
   route to local inference).

3. **A stable handle namespace addressable by an outside consumer.** Org must
   observe and address agents *by handle* (`agent-management` shaped-for note).
   That means agent handles must be stable, survivable across a CCD restart where
   reasonable, and mapped to live process state through a registry that is the
   single source of truth for "who is running." This registry + its query/stream
   API is the surface Org (and the dashboard, via mesh) builds on.

4. **First-class mesh participation.** Unlike a leaf service, CCD is *both*
   registrant and resolver (`service-registration`): it registers its own
   `slug -> host:port` so Org/others can find it, and it resolves `inference`
   (and future peers) by slug rather than a static URL. It must degrade to
   static config on a single box (matching the seam's static-fallback rule).

## Relationships / edges

- **cloud-code agents** via `agent-management` (see scaffold/contracts/agent-management.md)
  — CCD spawns / tracks / signals / routes agent processes and exposes them by
  handle; the substrate Org is layered on.
- **inference (api)** via `llm-calls` (see scaffold/contracts/llm-calls.md) —
  **NARROWED by the round-3 resolution:** Claude Code agents' calls are NOT
  routed to local inference (answered NO — Claude Code can't choose models);
  this edge is now usage-observation/metering-shaped, and the local-inference
  consumption is reserved for the future `agents` generalization's other agent
  types.
- **mesh.service-registry** via `service-registration`, an instance of
  `service-lookup` (see scaffold/contracts/service-registration.md) — CCD
  registers its own endpoint and resolves dependencies by slug.
- **org** via `org-on-ccd` (see scaffold/contracts/org-on-ccd.md) — Org is the
  downstream consumer; this edge is inbound-to-CCD only (CCD never depends on
  Org). Round-3: callers like org may call CCD directly **with a priority**,
  which the budget engine honors within the declared strategy.
- **mesh** via `ccd-events` (see scaffold/contracts/ccd-events.md) —
  **PROPOSED NEW EDGE, pending Decomposer/operator confirmation.**
  CCD emits an agent-lifecycle/run WS event stream that mesh's observability
  plane (which absorbed gateway, 2026-07-18) aggregates,
  mirroring `inference-events` and `gc-events`. Added as a minimal
  stub only (see report). If rejected, delete the stub and drop this line.
- **surface-schema** (see scaffold/contracts/surface-schema.md) — like every
  service, CCD publishes its observable-surface schema (budget state, agent
  roster, limit meters) for the mesh dashboard to render.
- **rollup** via `rollup-ccd` (see scaffold/contracts/rollup-ccd.md) —
  **NEW round-6.** CCD consumes rollup for its plugin/prompt assembly:
  fragments + slots rolled up into specialized plugins for specialized
  agents, generated on demand with no symlinks or file-copying. CCD is the
  consumer; requirements-only. (Also anticipated round-6: mesh's new
  dead-letter queues escalate into a **ccd agent investigation** — same
  escalation pattern as the execution engine's loop-depth hook; see
  `components/mesh.md` concern 14. No new contract stub; rides the existing
  ccd surfaces until designed.)
- **spend** (future placeholder, renamed from `finance` — rounds 4–5) — spend
  QUERIES CCD's usage database (sessions, tokens, limits) as one of its spend
  sources; pull-shaped, inbound-to-CCD only. No contract stub yet (spend is
  still a placeholder with no component file); recorded here so CCD's usage
  database is designed queryable-from-outside from the start.

## Nesting (if applicable)

Parent: none (top-level). Children: none at the *scaffold* level. Internally the
`ccd` crate decomposes into **libraries within the crate** (not separate crates —
they are not independently apps, per the operator's rule "if something is not an
app, make it a library to use within a crate"):

- `supervisor` — process spawn/track/signal/reap + output-stream capture & parse.
- `registry` — the handle namespace + live-agent state; the addressable surface.
- `budget` — the usage-limits/priority strategy engine (was `router`, the
  LLM-routing shim — reshaped by the round-3 NO on local-inference routing).
  Rounds 4–5: also owns (or sits atop) the **CCD usage database** — sessions,
  tokens, limits; queryable by `spend`; thread records linkable to projects
  and optionally environments.
- `control` — the HTTP/WS control API implementing `agent-management` + the
  `org-on-ccd` consumption surface, and the `ccd-events` emitter.
- `mesh_client` — thin register/resolve wrapper over `service-lookup` (see
  shared-library note below).
- `config` — daemon config + static-fallback endpoints.

**Crate = app (CLI + daemon split).** CCD is ONE app with two faces of the same
binary, mirroring the repo (`bin/gc` = daemon, `bin/db` = noun-verb CLI,
`bin/mesh` = daemon): `bin/ccd` provides the daemon (`ccd serve`) and a thin
**client CLI** (`ccd agent spawn|list|logs|send|kill …`) whose subcommands are
just HTTP calls into the running daemon — noun-verb, like `db`. The real logic is
`lib/ccd`; the binary is the thin entry point. Suggested workspace members:
`bin/ccd` + `lib/ccd`.

**Shared-library eye (for the later dedup pass — flag, do not build now).** Two
pieces of CCD are near-certain duplication candidates and should be *consumed*,
not re-hand-rolled: (a) the **inference `/v1/` completion client** (reqwest
wrapper over `v1-completion-api`) is also needed by mesh's observability plane
and Org — extraction
target for a shared `substrate-api-client` lib; (b) the **mesh register/resolve
client** (`service-lookup`) is needed by inference, Org, vfs, projects, and CCD —
extraction target inside `lib/mesh`'s public surface. CCD should depend on these
shared clients once they exist; until then keep its own copies thin and
obviously-extractable. Do not gold-plate either into CCD.

## Thoroughness level

**approach-sketched.** The crate shape, CLI/daemon split, internal module
decomposition, edges, and mesh participation are implementation-ready. The
previously-flagged consequential fork — LLM-routing semantics — is **RESOLVED**
(round-3: NO local-inference routing for Claude Code; see Charter). What now
holds CCD below implementation-ready is the **budget/priority strategy
language** (`requirements-only`: the declarative limit/percentile/priority
strategy shapes are named with one verbatim example but undesigned).

## Assigned design-depth

Single strong-model agent (Opus-class Component Designer), grounded directly
against the live V1 Substrate repo (crate layout, `bin/*` patterns, existing
contracts) and the operator's own voice-thread transcripts under
`~/code/harness/.mind/threads/` — **not** a Design Mesh run. CCD had no prior
repo code (it is net-new), so grounding is intent-capture-grade for scope and
repo-grade for shape/conventions.

## Suggested fill-model

approach-sketched + moderate-to-high complexity (net-new daemon, OS process
supervision, a declarative budget engine) → **needs a strong fill model**, OR:
design the budget/priority strategy language first (the `budget` sub-part is
the remaining hard spot — a **Design Mesh pass scoped to `budget`**) plus a
mid-tier model for supervisor/registry/control/CLI would be the cost-efficient
split. Do not send the whole crate to a cheap model as-is: the process-supervision
and budget modules will regress silently without design rigor at fill time.
