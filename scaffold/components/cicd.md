# cicd

**Status:** NEW (rounds 4–5 lock, 2026-07-18; **round-9 lock, 2026-07-19 —
repositioned as a LAYER-6 STUB with design notes**; **wave-2 batch-7 refit,
2026-07-19 — re-grounded on the now-authored `repo`, `environments`, `queues`,
`cron` designs**; **wave-3 refresh, 2026-07-22 (repo-environments-refresh
unit, batch 6) — reference update only: `cron` is no longer a standalone lib,
it is absorbed as `TriggerSource::Schedule` inside `queues` (wave-3 batch 3,
INTENT #56/#91/F6b); every "queues + cron" reference below now reads "queues
(incl. its Schedule trigger source)". This strengthens, not changes, the
pipeline-as-trigger-chain hypothesis below — see the wave-3 note in that
section. Crate-vs-emergent stays OPEN (ledger OQ-32).**). **Nesting:**
top-level as a stub; **cicd-as-its-own-crate vs. cicd-emerging-from-repo is
OPEN.** **Layer-6 design stub — NOT built in v1; NOT a full implementation-ready
design.**

> **NOT IMPLEMENTING NOW.** cicd is **layer-6/future**: this file is design
> notes + anticipated data contracts + open questions only, per the
> second-design-wave stub-track process. No schemas are frozen here; the
> anticipated contracts are named with rough shape only and reconciled when
> cicd leaves the stub track. The v1 GitHub-Actions surface cicd might have
> owned belongs to **`repo`** instead (FULL GH-Actions workflow management in
> v1 — workflows are directory changes; secrets-injection-on-push is the
> required v1 capability; rollup excluded — see `components/repo.md`).

## Charter (design notes, round-9 — preserved)

cicd's real scope, drawn from the operator's live practice:
**pipelines that execute WITHIN the mesh and watch the world OUTSIDE it.**
A cicd pipeline:

- **runs within the mesh** — it is mesh-resident orchestration (queues/
  triggers/handlers, incl. `Schedule`-sourced triggers underneath), not a
  hosted CI product;
- **watches external services** — GitHub Actions runs, AWS App Runner,
  Cloudflare/CloudFront, and whatever else a deployment touches;
- **verifies deployments** — polls until the deployed artifact is live and
  healthy (e.g. SHA-verification of what production actually serves);
- **may invoke agents / cc as pipeline steps** — an agent can be a step:
  diagnosing a failed run, deciding a retry, triggering an invalidation;
- **trends deterministic** — the goal, operator-grade: "eventually make it
  deterministic with as few agents in the loop as possible." Agents are
  scaffolding for the not-yet-deterministic parts; each iteration should
  replace agent judgment with recorded, deterministic steps.

**Grounding — the living example:** the operator's **Deployment Chaperone**
subagent (`~/code/broomstick/.mind/plugins/broomstick-engineer/agents/deploy-chaperone.md`,
read-only prior art). Its loop is exactly the cicd shape: push (the push IS
the trigger) → watch the GH Actions run → on failure, an agent diagnoses
from logs, fixes, re-pushes, loops → poll App Runner via `list-operations`
(not stale timestamps) → wait for the CloudFront invalidation to complete →
**verify the deployed SHA matches** → report; migrations applied separately
and drift-safe, never blind-pushed. That is: an agent-in-the-loop pipeline,
watching external services from outside them, verifying rather than
trusting, with bash tools as deterministic steps — the thing cicd makes
first-class and progressively de-agents.

## History (rounds 4–5 capture — still true, folded into the above)

A pipeline attaches to an **environment** (`components/environments.md`)
and is **activated when a branch or worktree is deployed/merged into that
environment** — with real-world consequences. Operator: "We need crates for
these things. I don't know exactly what the hierarchy is — environments
might be a sibling of CICD, might be a child; CICD probably has to use
environments." **Chained deployments** (stated on the environments side):
stage → checks → green prod, old prod flips to blue — the pipeline chain
drives the promotion. cicd **consumes environments** — it does not own the
environment concept.

## Is cicd mostly configuration over mesh queues (incl. Schedule triggers)? (the boring hypothesis)

The wave-2 batch-2 designs make this worth stating sharply, because if it
holds, cicd is *very* boring indeed — mostly declarative configuration over
primitives that already exist, not a new engine.

> **Wave-3 strengthening (reference update only, no re-decision).** The
> hypothesis gets MORE boring, not less, now that `cron` is folded: it is no
> longer a fourth primitive alongside queues/triggers/handlers — it is
> literally `TriggerSource::Schedule` on the SAME `Trigger` struct
> (`queues.md` concern 10, wave-3 batch 3). So "a pipeline step is a trigger
> chain" and "external-service polling is cron" collapse into **one**
> sentence: every cicd step, whether event-fired or time-fired, is the same
> `Trigger` row with a different `source`. One fewer moving part than the
> wave-2 telling below assumed.

A cicd pipeline decomposes cleanly onto the LOCKED mesh substrate:

- **A pipeline step is arguably just a trigger chain.** `queues` already owns
  the LOCKED four-term vocabulary (INTENT #101/#103): typed **EVENTS** land in
  queues; declarative **TRIGGERS** (pure DATA — filter + payload-assembly
  template, registered as data, NEVER code) bind a queue or a schedule 1:1 to a
  **HANDLER**; handlers are the only place code lives. A pipeline "step" maps
  onto exactly this: an event arrives (a run completed, a deploy fired) OR a
  schedule fires (a poll comes due), a declarative trigger filters/assembles,
  a handler acts and emits the next step's event. The step *graph* is a chain
  of (event-or-schedule → trigger → handler → event) — exactly what `queues`
  calls a handler emitting the next event, now uniformly across both trigger
  sources (`TriggerSource::Queue` | `TriggerSource::Schedule`, wave-3 fold).
- **The deploy kick-off is already a declarative trigger.** `repo` fires
  `repo.deployed { repo, ref, env, head_sha, correlation_id }` on
  deploy-into-environment (`repo-environments`, repo.md concern 7). A cicd
  pipeline is (at minimum) a set of triggers subscribed to that event and to
  `repo.workflow.run.completed` — the pipeline *definition* is trigger data,
  not bespoke orchestration code.
- **The "watch external services" polling is a `Schedule`-sourced trigger +
  a handler.** Poll App Runner `list-operations` / CloudFront invalidation
  status / the deployed SHA on a `TriggerSource::Schedule { target:
  FireTarget::Anywhere }` occurrence (single-fire via the same event-ID
  semaphore every other trigger uses, `queues.md` concern 10); the handler
  checks the external service and emits `deploy.verified` or `deploy.failed`.
  No new scheduler, no separate lib — the schedule evaluator lives inside
  `queues` itself now (the former standalone `cron`/`cron-api` are
  tombstones).
- **Exactly-once and loop-safety come for free.** Per-trigger event-ID
  semaphores (`locks`, INTENT #95) give exactly-once step execution;
  `execution-engine`'s loop-depth detection + the cc escalation hook already
  exist for run-away retries. cicd does not reinvent any of this.
- **Agent steps are just handlers that call cc.** An agent-in-the-loop step
  (diagnose-from-logs, decide-retry, trigger-invalidation) is a handler whose
  code invokes cc (`cc-escalation`-shaped, or a direct `agent-management`
  spawn). The "trend deterministic" goal is then literally *replacing an
  agent-invoking handler with a deterministic bash/SQL handler* over time —
  the substrate makes de-agenting a per-step edit, not a rewrite.

**What that leaves as cicd's own, irreducible scope** (the non-boring residue):

1. **A pipeline/step DEFINITION model** — a named, versioned, environment-attached
   collection of trigger-chains + external-watch specs + verify assertions
   (SHA-match, health) + agent-step declarations. This is *configuration over
   queues* (both trigger sources), but the config schema, its attachment to an
   environment, and its activation semantics are cicd's to own.
2. **The verify/assert vocabulary** — "what does 'deployed' mean" (SHA-verify what
   production serves, App Runner state, CloudFront invalidation complete). The
   Deployment Chaperone's checks, made declarative and recorded.
3. **The de-agenting ledger** — recording which steps are still agent-driven vs.
   made deterministic, so the "as few agents as possible" trend is trackable
   (per-pipeline provenance, INTENT #85-grade).

Working hypothesis (NOT resolved): **cicd ≈ a declarative pipeline-definition
layer + a verify vocabulary + a de-agenting ledger, sitting on queues (incl.
its Schedule trigger source) + locks + repo + cc.** If that holds, cicd is thin
config over frozen primitives —
which is the boring, no-new-engine outcome the standing principles prefer. This
directly feeds the own-crate-vs-emergent question below.

## own-crate vs. emergent-from-repo — sharpening the STANDING OPEN (not resolving)

INTENT #104 leaves this OPEN. The boring-hypothesis above lets us state the
tradeoff precisely rather than hand-wave it:

- **Case for EMERGENT-FROM-REPO.** repo already owns the *only* GH-Actions
  directory/remote surface and already authored `cicd-repo`'s verbs
  (`TriggerWorkflow`/`ComposeChain`/`ObserveRuns`, repo.md concern 8), *and*
  repo already fires the deploy trigger (`repo.deployed`) and already stitches
  `correlation_id` across push→workflow→deploy. repo.md explicitly designed its
  side so that **if cicd emerges from repo, `cicd-repo` collapses into
  repo-internal module boundaries with no contract rework** (its `workflows` lib
  gains the watch/verify loop). If cicd is "thin config over queues+repo"
  (queues now including the Schedule trigger source that used to be `cron`),
  the watch/verify loop is small enough to live as a `repo` internal lib, and
  the whole cicd contract surface disappears. **Cheapest, fewest new crates,
  most boring.**
- **Case for OWN-CRATE.** cicd's substrate is `queues` (event- and
  schedule-sourced triggers alike)/`cc`/`environments`, NOT git — the
  GH-Actions surface (`cicd-repo`) is only *one*
  of the external services it watches (App Runner, Cloudflare have no repo
  seam). Its agent-invoking steps pull in `cc`; its verify loop watches AWS via
  `aws`; its pipeline definitions attach to `environments`. Folding all that
  into `repo` would make `repo` own deployment orchestration, App Runner
  polling, and agent scheduling — a scope creep that violates repo's own stated
  boundary ("repo does not own the watch/verify deployment loop — that is cicd",
  repo.md). An own-crate keeps repo boring and keeps the multi-external-service
  orchestration in one place.
- **The deciding question (for the operator, later):** does the watch/verify
  loop's dependency footprint stay inside repo's world (git + GitHub) or does it
  reach broadly across `aws`/`cc`/`environments`/`queues`-schedule-jobs? The Deployment
  Chaperone touches GH Actions **and** App Runner **and** CloudFront **and**
  agents — which leans **own-crate**. But if v1's first real pipeline is
  GH-Actions-only, **emergent-from-repo** ships sooner with less. **Left OPEN;
  the collapse path is already designed cheap either way (repo.md concern 8), so
  deferring costs nothing.**

## environments consumption (pipeline attachment / activation)

cicd **consumes** `environments`; it does not own the environment concept
(`components/environments.md`). The consumption shape (v1-facing, anticipated):

- **Attachment:** a pipeline is attached to an environment (an environment "can
  have a CICD pipeline attached", INTENT #75). cicd holds the pipeline
  definition keyed by `EnvRef { project, env }` (the shape `repo` and `secrets`
  already share).
- **Activation:** **deploying/merging a branch or worktree INTO an environment
  ACTIVATES its pipeline, with real-world consequences.** The activation trigger
  is `repo`'s `repo.deployed` event (repo fires the declarative trigger; cicd's
  pipeline is the handler-chain that runs — INTENT #103: triggers declarative,
  code in handlers). cicd does not poll repo; it subscribes.
- **Chained promotion:** stage → checks → green prod, old prod flips to blue
  (stated on the environments side). The chain is a pipeline of trigger-steps;
  cicd drives the promotion, `environments` owns the blue/green environment
  model and the bandit logic (which is **in-app, NOT mesh** — environments.md
  round-6 resolution, respected here).
- **Hierarchy carried, not resolved:** environments-as-sibling-with-dependency
  is the working assumption (cicd depends on environments). No cicd↔environments
  contract stub is created until the hierarchy is decided (see open questions).

## Anticipated contracts (wave 2, stub track)

Named + rough shape only — **no schemas frozen**; content comes when cicd leaves
the stub track. Only `cicd-repo` has a live stub file (authored requirements-only
+ repo's batch-6 side already sketched its verbs); the rest are anticipated edges
with no stub until the own-crate-vs-emergent and hierarchy questions resolve.

### `cicd-repo` (cicd → repo) — kick off workflows / compose chains / observe runs

- **Purpose.** cicd triggers, chains, and observes GH-Actions workflows THROUGH
  repo — repo is the ONLY component touching the GH-Actions directory/remote
  surface (INTENT #104; repo.md concern 8). The watch/verify loop
  (poll-to-completed, SHA-verify what production serves, App Runner/CloudFront
  watching, agent-in-loop retry) is **cicd's own scope**, not repo's.
- **Rough shape (repo already authored its side — cicd consumes it unchanged):**
  `TriggerWorkflow { repo, workflow, ref, inputs } -> RunHandle`;
  `ObserveRuns { repo, workflow? } -> Vec<RunStatus>` where `RunStatus` carries
  `{ run_id, status, conclusion, head_sha, logs_url }` — `head_sha` is exactly
  what cicd's SHA-verification needs; `ComposeChain { repo, ChainSpec }` for
  declarative (`on: workflow_run`/`workflow_call` YAML edit) or orchestrated
  (repo dispatches step N+1 when cicd reports step N complete) chains.
  `correlation_id` from `repo`'s `repo.deployed` threads into `TriggerWorkflow`
  so cicd's watch and repo's deploy share one provenance chain.
- **Collapse note.** If cicd emerges from repo, these verbs become repo-internal
  calls and this contract disappears (repo.md designed for that). Stub-blocked:
  cicd's consumption is deferred; repo's side is authored now.
- *(scaffold/contracts/cicd-repo.md — exists, requirements-only, layer-6.)*

### `cicd ↔ mesh` (queues, incl. Schedule triggers / locks) — the pipeline substrate (anticipated)

> **Wave-3 reference update.** `cron`/`cron-api` are tombstones (wave-3 batch
> 3, INTENT #56/#91/F6b); "schedules external-watch `cron` jobs" below now
> reads "registers `TriggerSource::Schedule` triggers" — same mechanism, one
> fewer contract, no behavior change.

- **Purpose.** A pipeline runs ON the mesh fabric: pipeline steps are declarative
  triggers over `queues` — both event-sourced (`TriggerSource::Queue`) and
  schedule-sourced (`TriggerSource::Schedule`, external-service polling as a
  "run-anywhere" occurrence) — with exactly-once step execution riding `locks`
  event-ID semaphores. Per the boring hypothesis above, this is cicd's
  *primary* substrate — most of a pipeline is configuration expressed as
  `queues-api` triggers, of either source.
- **Rough shape.** cicd registers declarative TRIGGERS (`queues-api`) subscribed
  to `repo.deployed` / `repo.workflow.run.completed` / its own step events
  (`source: Queue`), and registers `Schedule`-sourced triggers for
  external-watch polling (`source: Schedule`, same `queues-api`, no separate
  API); acquires per-step semaphores (`locks-api`). Both are cross-cutting
  protocols owned by `queues`/`locks` — cicd is a party/consumer, authors no
  new mesh contract. No stub until cicd leaves the stub track.

### `cicd → cc` (agents as pipeline steps) — anticipated

- **Purpose.** An agent step (diagnose a failed run from logs, decide a retry,
  trigger a CloudFront invalidation) is a handler that invokes a cloud-code
  agent via `cc`. This is the "agents in the loop" the Deployment Chaperone
  embodies — and the thing cicd progressively *replaces* with deterministic
  handlers ("as few agents in the loop as possible").
- **Rough shape.** cicd invokes cc's `agent-management` (spawn/track/stream/reap)
  for an agent step, or reuses the `cc-escalation` shape (DLQ/loop-depth
  investigation) for failure diagnosis. Metering rides cc's usage DB; `spend`
  queries it (pull-shaped). No cicd-owned stub yet.

### `cicd ↔ environments` (pipeline attachment / activation) — anticipated

- **Purpose.** Pipelines attach to environments and activate on
  deploy-into-environment (see the environments section above). cicd consumes
  the environment model; `repo.deployed` is the activation trigger.
- **Rough shape.** Read/attach against `EnvRef { project, env }`; subscribe to
  the deploy event. **No contract stub until the sibling-vs-child hierarchy is
  decided** (INTENT #63/#75; working assumption sibling-with-dependency).

### `cicd ↔ secrets` (deploy-step credentials, use-without-seeing) — anticipated

- **Purpose.** Deploy/verify steps that call external services (AWS App Runner,
  Cloudflare) need credentials use-without-seeing — cicd is a trusted, non-`llm_safe`
  service (like repo/vfs), never an LLM-feeding slug, so raw resolution is
  legitimate; no secret ever reaches an agent step's LLM context.
- **Rough shape.** Resolve a `SecretRef` for an external-service credential via
  `secrets` (mirrors `repo`'s `ResolveGitCredential`). GH-Actions
  secret-injection stays repo's (`repo-secrets`) — cicd does not duplicate it.
  No cicd-owned stub yet.

## Nesting

Parent: none (working assumption) | Children: none (this pass). If cicd emerges
from `repo`, it becomes a `repo` internal lib (the `workflows` lib gains the
watch/verify loop) and this file's contracts collapse — see the own-crate
question.

## Open questions

1. **own-crate vs. emergent-from-repo** (INTENT #104, STANDING OPEN — ledger
   OQ-32; **wave-3: stays OPEN, stub-only this wave, not decided**) —
   sharpened above but deliberately NOT resolved; the collapse path is
   designed cheap either way (repo.md concern 8), so deferring costs nothing.
   Deciding input: does the watch/verify loop's dependency footprint stay
   inside git/GitHub (→ emergent) or reach across aws/cc/environments/queues-
   schedule-jobs (→ own-crate)? The Deployment Chaperone touches all four,
   leaning own-crate; a GH-Actions-only first pipeline leans emergent.
2. **hierarchy vs. `environments`** — sibling vs. child (INTENT #63; "environments
   might be a sibling of CICD, might be a child; CICD probably has to use
   environments"). Working assumption: sibling-with-dependency. No cicd↔environments
   contract stub until decided.
3. **Is a pipeline step exactly a `queues` trigger-chain, or does it need a
   thicker step abstraction?** The boring hypothesis says triggers (of either
   `TriggerSource`) suffice; the residue (pipeline-definition model, verify
   vocabulary, de-agenting ledger) is what would justify any cicd-specific
   structure. To settle when cicd is built, against a real second pipeline
   (beyond the GH-Actions/App-Runner/CloudFront one).
4. **The verify/assert vocabulary** — what "deployed and healthy" means as
   declarative data (SHA-match, App Runner state, CloudFront invalidation
   complete, health-endpoint) — undesigned; the Deployment Chaperone's checks are
   the seed.
5. **The de-agenting ledger shape** — how the "trend deterministic" progress is
   recorded per-pipeline/per-step (provenance-grade, INTENT #85) so agent steps
   can be systematically replaced. Undesigned.

## Thoroughness level

**requirements-only** (layer-6 design stub — **NOT implementing now**). Faithful
capture of operator intent (mesh-resident pipelines watching external services,
deployment verification, agents/cc as steps, the increasingly-deterministic
goal, the Deployment Chaperone grounding) + the boring "config over queues"
hypothesis (strengthened wave-3 now that `cron` is one of queues' own trigger
sources, not a fourth primitive) + a sharpened (unresolved) own-crate-vs-emergent
tradeoff (ledger OQ-32, still open) + anticipated contracts by name and rough
shape only. No schemas frozen; content lands when cicd leaves the stub track.
Assigned model: **Opus**, single stub-track pass (wave2-plan batch 7; wave-3
reference refresh by the repo-environments-refresh unit, batch 6).

## Anticipated contracts (wave 3, L6)

Reconfirming the stub-track edges above (INTENT #173c — contracts-with-lower-
layers concretely, internals at design-notes depth), with the wave-3 reference
fold applied:

- **`cicd-repo`** — purpose: trigger/chain/observe GH-Actions workflows THROUGH
  repo (repo is the sole GH-Actions surface owner). Rough shape:
  `TriggerWorkflow`/`ObserveRuns`/`ComposeChain`, `correlation_id`-threaded.
  Unchanged this wave; live stub at `scaffold/contracts/cicd-repo.md`.
- **`cicd ↔ queues`** (supersedes the wave-2 "`cicd ↔ mesh` (queues/cron/locks)"
  naming) — purpose: pipeline steps are `queues-api` triggers, both
  event-sourced and `Schedule`-sourced (former `cron`); exactly-once via
  `locks-api` event-ID semaphores. Rough shape: registers `Trigger { source:
  Queue(...) | Schedule(...) , handler, filter, assembly }` rows; no new verbs.
  No stub file until cicd leaves the stub track.
- **`cicd → cc`** — purpose: an agent step invokes cc (`agent-management`
  spawn, or the `cc-escalation` shape for failure diagnosis). Rough shape:
  unchanged from wave 2. No stub yet.
- **`cicd ↔ environments`** — purpose: pipeline attachment + activation on
  `repo.deployed`. Rough shape: unchanged; **blocked on the sibling-vs-child
  hierarchy decision** (open question 2) — no stub file until then.
- **`cicd ↔ secrets`** — purpose: use-without-seeing credential resolution for
  external-service deploy/verify steps. Rough shape: unchanged from wave 2. No
  stub yet.

**Crate-vs-emergent (OQ-32) is explicitly NOT decided by this refresh** — every
edge above is authored so it survives either outcome (own-crate keeps them as
external contracts; emergent-from-repo collapses `cicd-repo` into repo-internal
calls, per repo.md concern 8, and the rest ride along as repo's internal
dependencies instead).
