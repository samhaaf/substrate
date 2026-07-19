# Contract: cicd-repo

> **STUB-TRACK / layer-6 — cicd is NOT implemented in v1.** Lighter file: parties,
> purpose, a schema *sketch* (repo already authored its side in batch 6), and open
> questions. No full example world. SUPERSEDES the requirements-only stub, keeping
> its still-relevant collapse note.

## Parties

cicd (L6 stub, NOT built in v1) → repo (L5 app, live-track)

repo is the ONLY component touching the GH-Actions directory/remote surface
(INTENT #104). repo **authored its side now** (repo.md concern 8); cicd's
consumption is deferred until cicd leaves the stub track.

## Purpose

cicd triggers, chains, and observes GitHub-Actions workflows **through repo**.
The **watch/verify loop** (poll-to-completed, SHA-verify what production serves,
App-Runner/CloudFront watching, agent-in-loop retry) is **cicd's own scope**, NOT
repo's — repo exposes only trigger/observe/compose verbs; cicd layers the loop on
top. Designed as an external contract so the OPEN "cicd own-crate vs
emerges-from-repo" question (INTENT #104) stays cheap: if cicd emerges from repo,
these verbs become repo-internal calls and the contract collapses with no rework.

## Schema (sketch — repo authored; cicd consumes unchanged)

Reused from `types::repo` (`RepoError`) and `types::ccd` for correlation.

```rust
// cicd -> repo
struct TriggerWorkflow { repo: String, workflow: String, r#ref: String,
                         inputs: Map<String,String> }               // -> RunHandle
struct RunHandle       { repo: String, run_id: u64, workflow: String }

struct ObserveRuns     { repo: String, workflow: Option<String> }   // -> Vec<RunStatus>
struct RunStatus       { run_id: u64, status: RunState, conclusion: Option<RunConclusion>,
                         head_sha: String, logs_url: String }        // head_sha = what cicd SHA-verifies
enum   RunState        { Queued, InProgress, Completed }
enum   RunConclusion   { Success, Failure, Cancelled, TimedOut, ActionRequired }

struct ComposeChain    { repo: String, chain: ChainSpec }
enum   ChainSpec {
    Declarative  { workflow: String, on_run: String },   // a YAML edit: on: workflow_run / workflow_call
    Orchestrated { steps: Vec<String> },                 // repo dispatches step N+1 when cicd reports N complete
}
```

`correlation_id` from repo's `DeployedEvent` (`repo-environments`) threads into
`TriggerWorkflow` so cicd's watch and repo's deploy share one provenance chain.

**Error cases (rough):** `WorkflowInvalid` (e.g. a dispatch on a workflow lacking
`on: workflow_dispatch` → `{ detail: "not manually dispatchable" }`; repo can
offer to add the trigger — a directory change), `GhApiFailed { status }`,
`RepoNotFound` — all `RepoError`.

## Reconciliation notes

- **No disagreement — repo authored, cicd adopts verbatim.** cicd.md explicitly
  "consumes repo's authored verbs unchanged"; both files carry identical
  `TriggerWorkflow`/`ObserveRuns`/`ComposeChain` shapes. Nothing to reconcile;
  this file records the agreement and freezes nothing (stub-track).
- **Collapse path preserved.** repo.md designed its side so that if cicd emerges
  from repo, `cicd-repo` collapses into repo-internal module boundaries (repo's
  `workflows` lib gains the watch/verify loop) with zero contract rework.

## Open questions (carried from cicd.md / environments.md)

1. **own-crate vs emergent-from-repo** (INTENT #104, STANDING OPEN). Deciding
   input: does the watch/verify loop's dependency footprint stay inside git/GitHub
   (→ emergent) or reach across `aws`/`ccd`/`environments`/`cron` (→ own-crate)?
   The Deployment Chaperone touches GH Actions + App Runner + CloudFront + agents,
   leaning own-crate; a GH-Actions-only first pipeline leans emergent. Deferring
   costs nothing (collapse path designed cheap either way).
2. **Is a pipeline step exactly a `queues` trigger-chain**, or does it need a
   thicker step abstraction? The boring hypothesis (cicd.md) says triggers+cron
   suffice; the residue (pipeline-definition model, verify/assert vocabulary,
   de-agenting ledger) is what would justify cicd-specific structure.
3. **The verify/assert vocabulary** ("deployed and healthy" as declarative data —
   SHA-match, App Runner state, CloudFront invalidation complete, health-endpoint)
   — undesigned; the Deployment Chaperone's checks are the seed.
4. **Full schema, example data, and version-sensitivity are deferred** until cicd
   leaves the stub track — pinning them now would freeze shapes the open questions
   above may still move.
