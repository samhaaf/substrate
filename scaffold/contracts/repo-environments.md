# Contract: repo-environments

> SUPERSEDES the requirements-only stub. Authored from environments.md's PINNED
> v1-facing shape (the authoritative pin — environments.md §"`repo-environments`
> PINNED v1-facing shape") and repo.md concern 7 / its `repo-environments`
> proposal. **environments is a batch-7 L6 stub; repo is live-track** — repo's v1
> capability depends on this shape, so it is pinned precisely even though
> environments is not built now (wave2-plan §5 flag 2).

## Parties

repo (L5 app, live-track) ↔ environments (L6 stub, NOT implemented in v1)

repo *names* an attachment and *fires* the deploy event; environments *owns* the
environment concept + pipeline attachment; cicd *runs* the pipeline. environments
pinned the shape, repo agrees.

## Purpose

A branch or worktree (repo-owned) is **attachable to an environment**, and
**deploying/merging it INTO an environment ACTIVATES that environment's
pipeline** with real-world consequences (INTENT #75). The activation is a
**declarative trigger** (INTENT #103): repo publishes a `DeployedEvent`;
environments/cicd subscribe and activate the pipeline. repo does not run the
pipeline — it fires the trigger.

## Schema

Shared identity (must match repo.md line 312 and secrets' `SecretScope::Environment`):

```rust
// types::repo (shared with environments, secrets)
pub struct EnvRef { pub project: String, pub env: String }
```

Inbound from repo (pinned to repo.md's proposal):

```rust
// repo -> environments: attachment (recorded on repo's side; environments MAY record too)
struct AttachBranch  { repo: String, branch_or_worktree: String, env: EnvRef }

// repo -> pubsub (the declarative deploy trigger environments/cicd subscribe to)
struct DeployedEvent { repo: String, ref_name: String, env: EnvRef,
                       head_sha: String, correlation_id: Uuid }
```

Minimal v1 environments-side data model (what the stub must be able to represent
for repo's capability to stand — shape, not implementation):

```rust
// types (environments) — pinned shape, content of the body deferred
struct Environment {
    id: EnvRef,                    // (project, env) — the shared identity
    pipeline: Option<PipelineRef>, // attached CICD pipeline (cicd owns its body)
    role: EnvRole,                 // blue/green promotion state
    // routing/target for secrets + vdb resolved via secrets-environments / environments-vdb
}
enum   EnvRole { Green, Blue, Stage, Other(String) }
struct PipelineRef { /* opaque handle into cicd; environments does not run it */ }
```

## Error cases

- `EnvNotFound { env }` — environments-side, **once it exists**. Until
  environments is built, repo records attachments **optimistically** and
  reconciles when environments lands (agreed stub-track behavior, both files).
- No repo-side hard error for attaching to a not-yet-existent environment in v1 —
  the attachment is a local record + a fired event; a subscriber that isn't there
  yet simply doesn't activate (the event is lossy pub/sub, but the attachment
  record persists in repo's model for later reconciliation).

## Version sensitivity

- **LOW, but STUB-BLOCKED.** Content is v1-facing-shape-only until environments
  leaves the batch-7 stub track. New fields `#[serde(default)]`; `EnvRole`
  reserves `#[serde(other)]` (`Other(String)` already gives an open escape hatch).
- **Requirement of the stub, stated explicitly (both files):** repo's v1
  secrets-injection / branch-attachment flow depends on `EnvRef` + `DeployedEvent`
  being honored **exactly** as above. When environments is implemented, it MUST
  honor this shape — the pin is load-bearing.

## Reconciliation notes

- **No disagreement — environments PINNED, repo AGREES, shapes are byte-identical.**
  environments.md pinned `EnvRef`/`AttachBranch`/`DeployedEvent` explicitly "to
  match repo.md lines 312/610-615"; repo.md proposed the same structs. Merged
  verbatim; nothing dropped.
- **Direction:** the pair is written `repo ↔ environments` (bidirectional stub
  header) but the v1 *data flow is one-way*: repo → environments (attach + deploy
  event). environments → repo has no v1 traffic (environments is unbuilt). Kept
  the `↔` header for continuity with the stub; the schema is repo-outbound only.
- **Carried nuance (not resolved):** the "strange" public-website
  environments-branches relationship (INTENT #83) is carried into this contract,
  **not smoothed over** — both files flag it. Blue/green chaining as first-class
  state (ordered environments + role flips vs a bespoke primitive) is deferred to
  environments' real design pass.
- **GitHub-Environment vs substrate-environment collision** (from repo.md concern
  5 / repo-secrets): `EnvRef.env` is a substrate environment; a *GitHub*
  deployment environment (for env-scoped Actions secrets) is a distinct `gh_env`
  string carried on `GhSecretScope::Environment` (`repo-secrets`), NOT identified
  with `EnvRef.env`. environments should carry an optional explicit `gh_env`
  mapping when built (environments.md friction).

## Example data

repo, on **macbook**, deploys the `demo` repo's `V1` worktree (branch `v1`) into
the `demo/prod` environment — firing the pipeline:

```jsonc
// repo -> environments: record the attachment (V1 worktree -> demo/prod)
{ "repo": "demo", "branch_or_worktree": "V1",
  "env": { "project": "demo", "env": "prod" } }

// repo -> pubsub: the declarative deploy trigger (topic: repo.deployed)
{ "repo": "demo", "ref_name": "v1", "env": { "project": "demo", "env": "prod" },
  "head_sha": "4c9e2b71a0f3d5e8c1029ab4f6d7e8091a2b3c4d",
  "correlation_id": "c0110000-0000-4000-8000-000000000001" }

// the v1 environments-side model the stub must represent for this to activate:
{ "id": { "project": "demo", "env": "prod" },
  "pipeline": { /* PipelineRef into cicd's demo-prod pipeline */ },
  "role": "Green" }
```

The `correlation_id c0110000-…-0001` is the same chain that threads
`repo-secrets`' injection-on-push and `cicd-repo`'s workflow watch — one causal
trace from push → inject → deploy → verify (repo.md concern 9).
