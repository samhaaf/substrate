# cicd

**Status:** NEW (rounds 4–5 lock, 2026-07-18; **round-9 lock, 2026-07-19 —
repositioned as a LAYER-6 STUB with design notes**). **Nesting:** top-level
as a stub; **cicd-as-its-own-crate vs. cicd-emerging-from-repo is OPEN**.
**Layer-6 design stub — NOT built in v1; NOT a full design.**

> **ROUND-9 REPOSITIONING (2026-07-19):** cicd is **layer-6/future** — it
> gets this design stub with anticipated contracts and an explicit
> "not implementing now" note, per the second-design-wave process spec.
> The v1 GitHub-Actions surface it might have owned belongs to **repo**
> instead: repo owns FULL GH-Actions workflow management in v1 (workflows
> are directory changes; secrets injection on push is the required v1
> capability; rollup excluded) — see `components/repo.md`.

## Charter (design notes, round-9)

cicd's real scope, drawn from the operator's live practice:
**pipelines that execute WITHIN the mesh and watch the world OUTSIDE it.**
A cicd pipeline:

- **runs within the mesh** — it is mesh-resident orchestration (queues/
  triggers/handlers underneath), not a hosted CI product;
- **watches external services** — GitHub Actions runs, AWS App Runner,
  Cloudflare/CloudFront, and whatever else a deployment touches;
- **verifies deployments** — polls until the deployed artifact is live and
  healthy (e.g. SHA-verification of what production actually serves);
- **may invoke agents / CCD as pipeline steps** — an agent can be a step:
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

## Relationships / edges (anticipated — layer-6 stubs)

- **repo** via `cicd-repo` — **NEW round-9 (requirements-only).** cicd
  kicks off workflows manually and sets up workflow chains through repo
  (repo is the only component touching the GH-Actions directory/remote
  surface) (scaffold/contracts/cicd-repo.md).
- **environments** — pipelines attach to and are triggered through
  environments (rounds 4–5). No contract stub until the hierarchy is
  decided.
- **ccd / agents** (anticipated) — agents as pipeline steps (diagnose,
  retry, invalidate), invoked via ccd; no contract stub yet.
- **mesh** (anticipated) — pipelines run within the mesh on the
  queues/triggers/handlers fabric; no contract stub yet.
- **secrets** (anticipated) — deploy steps consume secrets
  use-without-seeing; no contract stub yet.

## Nesting

Parent: none (working assumption) | Children: none (this pass).

## Thoroughness level

**layer-6 design stub — NOT implementing now.** Round-9 design notes
(mesh-resident pipelines watching external services, deployment
verification, agents/CCD as steps, increasingly-deterministic goal,
Deployment Chaperone grounding) + the `cicd-repo` contract stub. Open:
**whether cicd stays its own crate or emerges from repo**; the
rounds-4–5 hierarchy question vs. `environments` (sibling-with-dependency
remains the working assumption).
