# environments

**Status:** L6 STUB TRACK — design notes + anticipated data contracts only.
**NOT IMPLEMENTING NOW.** This file captures operator intent and pins the
minimal data shapes its neighbors need in v1; it is not an implementation-ready
design. **Nesting:** top-level (hierarchy vs. `cicd` OPEN).
**Thoroughness: requirements-only.**

> **Load-bearing caveat.** `environments` is stub-track, but `repo`'s v1
> secrets-injection / branch-attachment capability presupposes a minimal
> environments data model (wave2-plan §5 flag 2). This stub therefore pins the
> **v1-facing shape of `repo-environments`** (and the `EnvRef` identity every
> neighbor shares) precisely, even though nothing here is built now.

## Charter (requirements, operator's words where quoted)

An **environment is a subset of a project** (INTENT #63/#75) — richer than a
replicated-store row. Requirements:

- **A CICD pipeline can be attached** to an environment.
- **Deploying/merging a branch or worktree INTO an environment ACTIVATES its
  pipeline, with real-world consequences.** Deployment-into-environment is the
  trigger; the pipeline is what runs. environments does not run the pipeline —
  it holds the environment concept + pipeline attachment; `cicd` owns pipeline
  execution; `repo` fires the declarative deploy trigger (INTENT #103).
- **Chained blue/green promotion is wanted:** "Chained deployments (stage →
  checks → green prod, old prod flips to blue) are wanted" (INTENT #75) — stage
  runs checks, promotes to green production, and the old production flips to
  blue.
- **Feature load-balancing via multi-armed bandits is IN-APP, NOT mesh.**
  INTENT #83, verbatim: bandits were "about complex in-app behavior inside a
  public-facing website deployed via projects/environments." The earlier
  "there may be some interaction between network and environments, or it may be
  inside environments, not sure" (INTENT #75) is superseded — bandit logic is
  an application-level concern, not mesh routing and not an environments
  mechanism.
- **Boring but not rigid.** INTENT #83, verbatim: whatever we do with
  environments "should be sufficiently boring, predictable and stable, but the
  user tends to do really creative patterns, so it shouldn't be overly rigid."
- **The environments-branches relationship is flagged "strange"** for
  public-website deployments (INTENT #83) — recorded as an unresolved nuance,
  carried not smoothed over. Surfaces in `repo-environments`.
- **Nested-projects — naive rule for now.** INTENT #63, verbatim: "assume there
  is no relationship between the environments of different projects; if there IS
  a relationship between a project and its sub-projects, they interface over the
  same environment by default." So: **no cross-project environment
  relationships; a sub-project shares its parent's environment by default.**

## Design notes (operator intent, faithful)

- environments is the **owner of the environment concept** — the `EnvRef`
  identity `(project, env)`, the pipeline attachment, and the blue/green
  chaining state. `repo` only *names* an attachment and *fires* the deploy
  event; `cicd` *runs* the pipeline; `secrets`/`vdb` *route* by the environment.
  environments is the small, boring hub the others reference.
- **Deploy-into-environment is a declarative trigger** (INTENT #103): the
  activation is a `repo.deployed` event on the pubsub protocol that
  environments/cicd subscribe to; the pipeline logic lives in cicd's handler,
  never in the trigger.
- **Blue/green as chained environments, not a bespoke primitive.** Sketch to
  revisit at design time: a promotion chain is an ordered set of environments
  (stage → green), each with its own pipeline; a successful chain step flips the
  prior production environment's role to blue. Keep it as boring
  environment-to-environment state transitions; the creative/bandit behavior
  stays in-app.
- **Hierarchy vs. `cicd` is OPEN** (INTENT #63): "environments might be a
  sibling of CICD, might be a child; CICD probably has to use environments."
  Working assumption: **sibling, with cicd depending on environments.** No
  environments↔cicd contract stub until the hierarchy is decided.

## Relationships / edges

- **repo** via `repo-environments` — branches/worktrees attach; the
  deploy-into-environment event activates the pipeline. **Load-bearing; pinned
  below.** (repo designed its side — repo.md concern 7 / its `repo-environments`
  proposal — this stub honors that shape.)
- **secrets** via `secrets-environments` — pushing secrets scoped/targeted to a
  specific environment (secrets.md concern 6, `SecretScope::Environment`).
- **vdb** via `environments-vdb` — an environment routes a database's storage
  target (local→SQLite, cloud→promote).
- **projects** — an environment is a subset of a project; nested-projects naive
  rule above; databases attach to projects/environments (rides projects/vdb —
  `projects-vdb`). Edge naming deferred; no environments-owned stub.
- **cc** — cc threads are optionally linkable to an environment within a
  project (recorded on cc's usage DB; cc.md / `cc-projects`). No
  environments-owned stub — cc holds the optional `EnvRef`.
- **cicd** — consumes environments; hierarchy OPEN (see design notes). No stub
  until decided.

## Anticipated contracts (wave 2, stub track)

Content is deferred until environments leaves the stub track — EXCEPT
`repo-environments`, whose v1-facing shape is pinned precisely below because
repo's v1 capability depends on it.

### `repo-environments` (repo ↔ environments) — PINNED v1-facing shape

- **Purpose.** repo attaches a branch/worktree to an environment and emits the
  deploy-into-environment event that activates the environment's pipeline
  (INTENT #75). repo names the attachment + fires the declarative trigger;
  environments owns the environment concept and pipeline attachment; cicd runs
  the pipeline.
- **Shared identity (pinned, must match repo.md line 312 and secrets.md's
  `SecretScope::Environment`):**
  ```rust
  pub struct EnvRef { pub project: String, pub env: String }
  ```
- **Minimal v1 environments-side data model** (what the stub must be able to
  represent for repo's capability to stand — shape, not implementation):
  ```rust
  pub struct Environment {
      pub id: EnvRef,                       // (project, env) — the shared identity
      pub pipeline: Option<PipelineRef>,    // attached CICD pipeline (cicd owns its body)
      pub role: EnvRole,                    // blue/green promotion state
      // routing/target for secrets + vdb resolved via secrets-environments / environments-vdb
  }
  pub enum EnvRole { Green, Blue, Stage, Other(String) }
  pub struct PipelineRef { /* opaque handle into cicd; environments does not run it */ }
  ```
- **Inbound from repo (pinned to repo.md's proposal, lines 610-615):**
  ```rust
  struct AttachBranch  { repo: String, branch_or_worktree: String, env: EnvRef }
  struct DeployedEvent { repo: String, ref_name: String, env: EnvRef,
                         head_sha: String, correlation_id: Uuid } // the declarative trigger
  ```
  environments/cicd subscribe to `DeployedEvent` (pubsub-protocol) and activate
  the pipeline. environments MAY record the attachment on its side; repo also
  records it.
- **Error cases.** `EnvNotFound` is environments-side (once it exists). Until
  environments is built, repo records attachments optimistically and reconciles
  when environments lands (agreed stub-track behavior — repo.md).
- **Carried nuance.** The "strange" public-website environments-branches
  relationship (INTENT #83) is carried into this contract, **not resolved.**
- **Requirement of the stub, stated explicitly:** repo's v1 secrets-injection /
  branch-attachment flow depends on `EnvRef` + `DeployedEvent` being honored
  exactly as above. When environments is implemented, it MUST honor this shape.

### `secrets-environments` (secrets ↔ environments) — stub-track (deferred content)

- **Purpose.** Push/scope secrets into a specific environment (INTENT #75/#99;
  secrets.md concern 6). No new mechanism — an environment-addressed push over
  secrets' existing adapters.
- **Rough shape.** Rides secrets' `SecretScope::Environment { project, env }`
  (the same `(project, env)` as `EnvRef`); resolution is most-specific-wins with
  fallback `Environment → Database → Project → Global`. environments' role is to
  say **which backing store / adapter target an environment maps to** (local
  mesh DB, Supabase Vault, AWS Secrets Manager, or a GitHub deployment
  environment) so secrets pushes to the right place. Content deferred.

### `environments-vdb` (environments ↔ vdb) — stub-track (deferred content)

- **Purpose.** An environment routes a database's storage target: **local
  environment → SQLite; cloud environment → promote** (INTENT #97/#98;
  vdb.md `environments-vdb`).
- **Rough shape.** v1 placeholder is vdb's explicit per-database
  `EnsureDatabase.target_hint`; when environments lands, that hint is replaced
  by **environment-derived routing** keyed on `EnvRef`. Keep the
  local-purpose-KG-can-support-a-cloud-environment nuance OPEN (INTENT #97;
  kg.md). No mechanism beyond vdb's existing verbs. Content deferred.

## Friction points

- **GitHub "Environments" name collision (from repo.md concern 5).** GitHub's
  deployment Environments (with env-scoped Actions secrets) are a *different
  concept* from substrate `environments`. When a substrate environment maps to a
  GitHub deployment environment for env-scoped Actions secrets, that mapping
  must be **explicit repo/secrets state, not an implicit identity.** environments
  should carry an optional explicit `gh_env` mapping when this is implemented.
- **repo depends on an unbuilt stub.** repo's v1 capability is live-track but its
  environments counterpart is stub-track; the `EnvRef`/`DeployedEvent` shapes are
  pinned here to de-risk that, but the dependency is real and flagged.

## Open questions

- **Hierarchy vs. `cicd`** — sibling or child? Working assumption: sibling, cicd
  depends on environments. Blocks the environments↔cicd contract.
- **The "strange" environments-branches relationship** for public-website
  deployments (INTENT #83) — unresolved; carried into `repo-environments`.
- **Blue/green chaining as first-class state** — is a promotion chain modeled as
  ordered environments + role flips (sketch above), or as a distinct primitive?
  Deferred to environments' real design pass.
- **local-purpose KG supporting a cloud environment** (INTENT #97) — the routing
  edge case kept open, shared with `environments-vdb` / kg.md.
