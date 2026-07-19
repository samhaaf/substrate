# ccd

**Status:** NEW (Cloud Code Daemon; aliases Marshall / CCM). **Nesting:**
top-level, flat (no scaffold-level children — internal parts are libraries within
the crate, per the repo's crate=app philosophy). **Build order: FIRST of the new
concepts, before Org.** **Siting: RESOLVED — lives inside the Substrate workspace
as a first-class member** (see Charter note; the overview's "CCD siting" open
question is now stale — operator answered it directly in the reconstruction
thread: *"The CCD program should also live in the same substrate repo… They're
all just apps in the repo. I don't know why that's even a question."*).

## Charter

CCD is the daemon that runs and governs **cloud-code (Claude Code) agent
processes** on a machine, and routes those agents' **LLM calls** to the local
inference fleet. It owns exactly three things: (1) the **process lifecycle** of
agent subprocesses — spawn, track, signal, stream output, reap (`agent-management`);
(2) a **stable agent-handle namespace + registry** so any caller can address a
live agent by handle and read its status/output; and (3) a **completion-routing
shim** that gives each agent an LLM endpoint pointed back at Substrate's own
`/v1/` inference surface instead of an external provider (`llm-calls`, reusing
`v1-completion-api`). It registers itself and resolves its dependencies through
the mesh service-registry (`service-registration`). CCD is "run all cloud code
through the Marshall": the single, observable choke point every agent process and
every agent LLM call passes through.

**Boundary — what CCD does NOT own.** CCD does **not** own inter-agent
*conversation*, org structure, manager/sub-agent hierarchies, pipelines, or
metacognition — that entire layer is **Org**, which is *built on top of* CCD and
consumes it (`org-on-ccd`). CCD manages *processes and their model calls*; Org
manages *agents talking to each other*. CCD also does **not** implement
completion scheduling, model residency, or the `/v1/` API — it is a **consumer**
of inference, never a peer runtime (no reverse dependency from inference to CCD).
It does not persist an org-graph or knowledge graph (that is Org-on-`db`). It
owns no dashboard; it *emits* events for the gateway to aggregate, but rendering
is the dashboard's job. Keeping CCD bounded to "process supervisor + LLM egress
for agents" is what lets Org be the maximalist consumer without CCD bloating into
it.

## Primary design concerns

These are the genuinely-hard parts that earn CCD its own crate rather than
clumping into gateway or inference:

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

2. **The LLM-routing shim (the subtle part).** Cloud-code agents speak a model
   provider's protocol (Anthropic Messages API shape, redirected via a base-URL
   env var). CCD stands up an endpoint each agent points at, and forwards each
   call to inference's `/v1/` completion surface (resolved by slug through the
   mesh, or via the mesh completion-router). This is a **protocol-adaptation +
   egress-metering** seam: it translates between what the agent CLI emits and what
   `v1-completion-api` accepts, and it is the point where per-agent LLM usage is
   observed. **This is also the largest scope-uncertainty in CCD** — see Open
   questions in the report: whether the target is genuinely *local-model serving*
   of agent completions vs. a *metered passthrough* to a cloud provider changes
   this module materially. Designed here as a **pluggable completion backend**
   (local inference via `v1-completion-api` = primary target; provider-passthrough
   = config option) so the choice is a config decision, not a rewrite.

3. **A stable handle namespace addressable by an outside consumer.** Org must
   observe and address agents *by handle* (`agent-management` shaped-for note).
   That means agent handles must be stable, survivable across a CCD restart where
   reasonable, and mapped to live process state through a registry that is the
   single source of truth for "who is running." This registry + its query/stream
   API is the surface Org (and the dashboard, via the gateway) builds on.

4. **First-class mesh participation.** Unlike a leaf service, CCD is *both*
   registrant and resolver (`service-registration`): it registers its own
   `slug -> host:port` so Org/others can find it, and it resolves `inference`
   (and future peers) by slug rather than a static URL. It must degrade to
   static config on a single box (matching the seam's static-fallback rule).

## Relationships / edges

- **cloud-code agents** via `agent-management` (see scaffold/contracts/agent-management.md)
  — CCD spawns / tracks / signals / routes agent processes and exposes them by
  handle; the substrate Org is layered on.
- **inference (api)** via `llm-calls`, which reuses `v1-completion-api` (see
  scaffold/contracts/llm-calls.md) — CCD forwards agents' LLM calls to the local
  `/v1/` surface; CCD is a pure consumer of inference.
- **mesh.service-registry** via `service-registration`, an instance of
  `service-lookup` (see scaffold/contracts/service-registration.md) — CCD
  registers its own endpoint and resolves dependencies by slug.
- **org** via `org-on-ccd` (see scaffold/contracts/org-on-ccd.md) — Org is the
  downstream consumer; this edge is inbound-to-CCD only (CCD never depends on Org).
- **gateway** via `ccd-events` (see scaffold/contracts/ccd-events.md) —
  **PROPOSED NEW EDGE, pending Decomposer/gateway-designer/operator confirmation.**
  CCD emits an agent-lifecycle/run WS event stream that the gateway aggregates for
  observability, mirroring `inference-events` and `gc-events`. Added as a minimal
  stub only (see report). If rejected, delete the stub and drop this line.

## Nesting (if applicable)

Parent: none (top-level). Children: none at the *scaffold* level. Internally the
`ccd` crate decomposes into **libraries within the crate** (not separate crates —
they are not independently apps, per the operator's rule "if something is not an
app, make it a library to use within a crate"):

- `supervisor` — process spawn/track/signal/reap + output-stream capture & parse.
- `registry` — the handle namespace + live-agent state; the addressable surface.
- `router` — the LLM-routing shim (pluggable completion backend → inference `/v1/`).
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
wrapper over `v1-completion-api`) is also needed by gateway and Org — extraction
target for a shared `substrate-api-client` lib; (b) the **mesh register/resolve
client** (`service-lookup`) is needed by gateway, inference, Org, and CCD —
extraction target inside `lib/mesh`'s public surface. CCD should depend on these
shared clients once they exist; until then keep its own copies thin and
obviously-extractable. Do not gold-plate either into CCD.

## Thoroughness level

**approach-sketched.** The crate shape, CLI/daemon split, internal module
decomposition, edges, and mesh participation are implementation-ready. It is held
below implementation-ready by **one consequential unresolved fork the operator
must settle**: the LLM-routing semantics (local-inference serving vs. metered
provider passthrough — see report Open questions), which materially shapes the
`router` module. Resolve that one decision and CCD is implementation-ready.

## Assigned design-depth

Single strong-model agent (Opus-class Component Designer), grounded directly
against the live V1 Substrate repo (crate layout, `bin/*` patterns, existing
contracts) and the operator's own voice-thread transcripts under
`~/code/harness/.mind/threads/` — **not** a Design Mesh run. CCD had no prior
repo code (it is net-new), so grounding is intent-capture-grade for scope and
repo-grade for shape/conventions.

## Suggested fill-model

approach-sketched + moderate-to-high complexity (net-new daemon, OS process
supervision, a protocol-adaptation shim) → **needs a strong fill model**, OR:
resolve the one routing-semantics open question first, then the `router` sub-part
is the only remaining hard spot — a **Design Mesh pass scoped to `router`** plus a
mid-tier model for supervisor/registry/control/CLI would be the cost-efficient
split. Do not send the whole crate to a cheap model as-is: the process-supervision
and shim modules will regress silently without design rigor at fill time.
