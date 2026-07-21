# repo

**Status:** SUPERSEDES the wave-1 `repo.md` requirements-only stub. **FULL
DESIGN — wave 2, batch 6.** **Nesting:** top-level L5 app-crate (`lib/repo`
crate `substrate-repo` + `bin/repo` daemon/CLI), one instance per node that
hosts anchored git repos (`AddressingClass::NodeScoped`). **Layer:** L5
service/application plane, built ON TOP of `vfs` (L3) — its only non-boring
trait (INTENT #80). **Consumes (over the wire, never linked — INTENT #29):**
`vfs` via `repo-vfs` (NodeAnchored worktree/`.git` hosting + snapshot
replication), `secrets` via `repo-secrets` (GitHub auth credential
use-without-seeing + GH-Actions injection-on-push), `environments` via
`repo-environments` (branch/worktree → environment attachment), `mesh` via
`service-lookup`/`pubsub-protocol`/`restart-protocol`/`surface-schema`.
**Consumed by:** `cicd` (L6 stub) via `cicd-repo`. Grounded in INTENT
#67/#80/#100/#104 (repo scope), #85/#92 (provenance), #99/#105 (secrets
adapters), #47/#48 (VFS), the batch-3 `vfs.md` (NodeAnchored file class +
`OpenAnchored`/`Snapshot` surface + the two file classes) and `secrets.md`
(the GitHub-Actions adapter + `repo-secrets` counterpart it already
authored), the batch-7-facing `environments.md`/`cicd.md` stubs, and the
operator's live `.mind` workspace schema (worktree ↔ thread ↔ branch prior
art — `core:workspaces`).

## Charter

`repo` makes **git repositories first-class managed entities of Mind OS, built
on top of the VFS** rather than a normal filesystem. It owns exactly one domain
and owns it completely: **the lifecycle of git repositories, branches, and
worktrees whose bytes live in VFS, the push/sync bridge between those
VFS-resident trees and real GitHub remotes, and the FULL GitHub-Actions
workflow-management surface in v1** (INTENT #104). Concretely it owns: the
**repo/branch/worktree model** (clone, create/delete branches, add/remove
worktrees — the operator's "I like worktrees. I like workspaces.", reusing the
`.mind` worktree↔thread schema); the **git-on-VFS execution model** (drive
*real* git against a VFS-anchored working directory — repo reimplements no git
internals, concern 1); **push/sync** between VFS trees and GitHub, authenticated
with a GitHub credential resolved **use-without-seeing** from `secrets`;
**GitHub-Actions workflow management** — creating/updating/removing workflow
files (workflows ARE directory changes, INTENT #104), **manual kickoff**
(`workflow_dispatch`), **workflow chains**, and **run observation**; **the
secret↔workflow *linkage*** and **secrets-injection-on-push** (THE required v1
capability, INTENT #104 — repo names the link, `secrets`' GH-Actions adapter
pushes the values); **branch/worktree → environment attachment** (deploying a
branch/worktree into an environment activates that environment's pipeline); and
**the cicd seam** (`cicd-repo` — cicd triggers/chains/observes workflows THROUGH
repo; repo is the *only* component that touches the GH-Actions directory/remote
surface).

**Boundary — what `repo` does NOT own.** It does not own **byte
storage/replication/placement** — that is `vfs` (INTENT #48); repo is a plain
VFS client that materializes git trees as VFS files and lets vfs place/replicate
them. It does not own **secret storage, encryption, or the push mechanics** —
that is `secrets` (INTENT #99); repo owns only the *linkage* of a secret to a
workflow and *asks* secrets to push. It does not own the **environment concept**
— that is `environments` (INTENT #75); repo attaches branches to environments
but does not define, chain, or execute pipelines. It does not own the
**watch/verify deployment loop** — that is `cicd` (INTENT #104); repo exposes
workflow trigger/observe verbs, cicd runs the polling/SHA-verify/agent-in-loop
logic on top. It does not own **workflow-content composition via `rollup`** —
`rollup` is **explicitly EXCLUDED** from GH-Actions management (INTENT #104);
workflow YAML is edited as plain files, never fragment-composed. It does not
reimplement **git** — it drives libgit2/the git CLI against a real OS path vfs
anchors. It holds no application data and imposes no schema on file bytes beyond
git's own. It is one boring layer: storage rides vfs, secrets ride secrets,
environments ride environments — repo is the git-management brain that
orchestrates real git over a virtual filesystem.

## Primary design concerns

### 1. The git-on-VFS execution model — drive REAL git against a VFS-anchored OS path (the one load-bearing trick)

The operator's single non-boring requirement (INTENT #80: "the only not-boring
thing about repo is that it's built on top of a virtual file system instead of a
normal one") resolves to one hard line: **repo never reimplements git; it runs
real git against a real OS path that VFS hands it.** The mechanism is already
built by `vfs`'s **`NodeAnchored` file class** (vfs.md concern 2): a mutable file
tree anchored to exactly one owner node, where VFS provides a **real local OS
path** the owner opens and mutates directly — VFS does not intercept every write.

- A **repository** = a git object store (`.git`) plus one or more worktrees,
  materialized as a single **`NodeAnchored` VFS directory tree** under
  `vfs://repos/<repo-slug>/` (the `.git` shared object store + `worktrees/<name>/`
  checkouts). repo calls vfs's `OpenAnchored { path } -> LocalPath { os_path,
  anchor_lock }` (the `repo-vfs` edge, mirroring `vdb-vfs`) to get the real OS
  path and the exclusive-writer `vfs.anchor.<path>` lock, then runs git against
  `os_path`.
- **Git backend behind a `GitBackend` trait** (the inference `InferenceBackend`
  pattern reused): default impl is **`git2`/libgit2 in-process** (clone, fetch,
  push, branch, `worktree add/remove`, commit, status — deterministic, no
  git-binary version drift, no shelling); a **git-CLI-shelling** impl is the
  documented fallback for the handful of operations libgit2 covers awkwardly
  (some worktree-prune and push-refspec edge cases). Fill-time latitude on which
  covers a given verb; the `repo-vfs` and `repo-secrets` seams are identical
  either way.
- **Durability is snapshot-driven, at git-natural boundaries.** repo does NOT
  snapshot on every `fwrite`. After a **commit** or a completed **fetch/pull**,
  repo calls vfs `Snapshot { path } -> content_hash` (vfs.md concern 2) at that
  transaction-consistent point; vfs content-addresses and replicates the tree as
  an immutable blob set. A crash between snapshots loses at most uncommitted
  working-tree edits — exactly git's own durability grain, not weaker.
- **All worktrees of one repo co-anchor to one node.** git worktrees require
  shared-filesystem access to the single `.git` (git's own mechanism); so a
  repo's `.git` + every worktree anchor to the **same** owner node, and git
  worktree operations run there. A "worktree on another machine" is not a git
  worktree — it is a **separate clone** (a distinct `NodeAnchored` repo tree
  anchored elsewhere, sharing the same GitHub remote). Stated as an honest
  constraint, not smoothed over — it is why repo is `NodeScoped` (concern 10).

This is the boring answer to "git on a VFS": VFS is the *host and
snapshot-replicator* of the working tree; **git itself is unmodified**, run
in-process against a real path. repo's cleverness is entirely in *management and
orchestration*, never in git plumbing.

### 2. The repo/branch/worktree model — reuse the `.mind` worktree↔thread schema (INTENT #67)

The operator has prior art he wants reused verbatim: his `.mind` workspace schema
already **links a worktree, a branch, and a Claude Code thread** (INTENT #67; the
`core:workspaces` `release↔worktree` 1:1 model, `sub_workspace.locus ∈
{same-branch, new-worktree}`, and the artifact `thread:` / workspace
`coordinator_thread_id` fields). repo's model adopts the same shape as
first-order records:

```rust
// lib/repo::model  (persisted — see below)
pub struct RepoRecord {
    pub slug: String,                 // "substrate", stable id
    pub vfs_root: String,             // vfs://repos/<slug>/
    pub anchor_node: NodeId,          // where .git + worktrees live (concern 1)
    pub remotes: Vec<GitRemote>,      // github origin(s) (concern 4)
    pub default_branch: String,
    pub provenance: Provenance,       // INTENT #85 (concern 9)
}
pub struct GitRemote { pub name: String, pub url: String, pub auth: SecretRef } // token from secrets (concern 4)

pub struct WorktreeRecord {
    pub id: Uuid,
    pub repo: String,                 // RepoRecord.slug
    pub name: String,                 // "V1", "main", "stt"
    pub branch: String,               // the checked-out branch (worktree ↔ branch)
    pub vfs_path: String,             // vfs://repos/<slug>/worktrees/<name>/  (materialized VFS path)
    pub thread: Option<String>,       // OPTIONAL Claude-Code thread id — the .mind link (INTENT #67)
    pub environment: Option<EnvRef>,  // OPTIONAL environment attachment (concern 7)
    pub provenance: Provenance,
}
pub struct BranchAttachment { pub repo: String, pub branch: String, pub environment: EnvRef } // concern 7
```

**Where repo's model lives (the layering-honest answer).** repo is L5 and cannot
link the mesh-internal `KvHandle` (INTENT #29), and its data is naturally
co-located with the anchored repo. So:

- **Durable source of truth = a small VFS-resident manifest** (`repo-meta.json`,
  or a tiny SQLite) **inside the repo's `NodeAnchored` tree**
  (`vfs://repos/<slug>/.repo/`), snapshot-replicated with the git data (concern
  1). A worktree-migration or node-loss carries the model *with* the repo — no
  separate replication scheme invented.
- **Per-node operational index = a local `repo.db` SQLite** on the plain OS
  filesystem (like inference's `store`, secrets' cache, cc's usage DB — INTENT
  #31: "everywhere you have a daemon running on a node, you need a database that
  manages that node's data"), a read-through cache/index of the repos anchored
  here plus in-flight operation state.
- **Mesh-wide discovery** ("show all repos/worktrees across the fleet") is
  **surface-schema + `repo.*` pub/sub aggregation** across the NodeScoped repo
  fleet (the completion-router/dashboard pattern), NOT a global registry repo
  owns. Flagged (friction): if cross-node repo listing ever wants
  strong/queryable semantics, repo is a candidate for the same
  **mesh-brokered keyspace** surface `secrets` negotiated with mesh (secrets.md
  concern 1) — an anticipated future generalization, out of v1 scope.

### 3. Clone / branch / worktree lifecycle (INTENT #67/#80)

The management verbs, each = a git op against the anchored OS path (concern 1) +
a model update (concern 2) + a snapshot (durability):

- **`clone`** — resolve the remote's GitHub credential from secrets (concern 4),
  `git2::clone` into a freshly-created `NodeAnchored` vfs tree on a chosen anchor
  node (caller pins `Node{N,repo}`, or `AnyNode{repo}` + placement picks), write
  the `RepoRecord`, snapshot.
- **`branch create/delete/list`** — plain git ref operations; a branch is
  attachable to an environment (concern 7).
- **`worktree add/remove/list`** — `git worktree add <path> <branch>` under
  `worktrees/<name>/`, materializing the checkout as a VFS path
  (`vfs://repos/<slug>/worktrees/<name>/`); write the `WorktreeRecord` with its
  optional `thread`/`environment` links. **This is the feature the operator most
  wants** — worktrees are first-class, addressable VFS paths, each on its own
  branch, each optionally bound to a thread and an environment. Removing a
  worktree prunes the git worktree and the record.
- All worktrees of one repo co-anchor (concern 1); repo refuses a
  `worktree add` that would require a cross-node `.git` (`AnchorElsewhere` —
  the caller wants a separate `clone` instead).

### 4. Push/sync between VFS and GitHub remotes — auth use-without-seeing (INTENT #80)

repo bridges VFS-resident trees and real GitHub remotes:

- **Operations:** `fetch`, `pull`, `push`, `sync` (fetch + fast-forward/report),
  status/ahead-behind. Each runs via the `GitBackend` against the anchored path;
  a completed fetch/pull triggers a `Snapshot` (durability); a push runs the
  injection-on-push path first (concern 6).
- **GitHub auth = a credential resolved use-without-seeing from `secrets`.**
  git2's credentials callback needs a token; repo resolves the **raw** GitHub
  credential (PAT or GitHub-App installation token) from `secrets` via
  `repo-secrets` (`ResolveGitCredential`). repo is a **trusted, non-`llm_safe`
  service** (its mesh identity is `repo`, not an LLM-feeding slug), so raw
  resolution is legitimate — exactly the same pattern as `vfs` resolving its S3
  CSE key (vfs.md concern 8; secrets.md `llm_safe` is set from caller mesh
  identity, unspoofable). The credential itself is a **Global- or Project-scoped
  secret**; the plaintext flows secrets→repo's credential callback and never into
  any LLM path (repo runs no completions). `GitRemote.auth` is a `SecretRef`, a
  handle, never a raw value at rest in repo's model.
- **Constraint that stands (INTENT #13):** don't push broken versions — a policy
  the caller/cicd enforces, not repo mechanics; repo pushes what it is told.

### 5. GitHub-Actions workflow management v1 — workflows ARE directory changes (INTENT #104)

Settled on the operator's own reduction: "managing a GitHub Actions pipeline is
just making changes in a directory." repo owns the **full** surface in v1:

- **Workflow files** — create/update/remove `.github/workflows/*.yml` in the
  anchored worktree as **plain file edits** (concern 1), committed + pushed
  (concern 4). **`rollup` is EXCLUDED** (INTENT #104): no fragment composition,
  no slots — repo writes the YAML directly (the content originates from the
  caller/cicd). repo validates basic workflow shape (YAML parses; `on:` present)
  but does not template.
- **Manual kickoff (`workflow_dispatch`)** — `POST
  /repos/{owner}/{repo}/actions/workflows/{id}/dispatches` with a ref + optional
  inputs, over the GitHub REST API (auth from concern 4). repo may ensure a
  managed workflow carries `on: workflow_dispatch` so manual kickoff is possible.
- **Chains** — two shapes, both supported in v1:
  - **declarative chains** = `on: workflow_run` / reusable `workflow_call` wired
    directly into the YAML (a directory change — repo's native surface); and
  - **orchestrated chains** = cicd (or a caller) observes a run's completion via
    repo and dispatches the next workflow (concern 8) — the seam repo exposes so
    cicd composes without touching GitHub directly.
- **Run observation** — list runs, get a run's status/conclusion/head-SHA/logs-URL
  (`GET /repos/{o}/{r}/actions/runs...`), the boundary cicd's watch/verify loop
  polls (concern 8). repo is the GitHub-API boundary; the polling/SHA-verify
  *logic* is cicd's, not repo's.

**GitHub-Actions secret scoping reality (flagged).** GitHub has no per-workflow
secrets — Actions secrets are **repo-, GitHub-Environment-, or org-scoped** and
workflows reference them by NAME. So repo's "secret↔workflow linkage" (concern 6)
is modeled logically ("workflow W needs secret NAME X") but *pushed* at the GH
scope GitHub supports (repo-level, or a GitHub deployment-Environment level).
**Naming collision flagged:** GitHub's "Environments" (deployment environments
with env-scoped secrets) are a *different concept* from substrate `environments`
(INTENT #75); when a substrate environment maps to a GitHub deployment
environment for env-scoped Actions secrets, that mapping is explicit repo/secrets
state, not an implicit identity. Surfaced as friction for the environments
batch-7 stub.

### 6. Secrets-injection-on-push — repo owns the LINKAGE, secrets pushes the VALUE (THE v1 capability, INTENT #104)

The required v1 capability (INTENT #104): "inject secrets when pushing to the
remote." The seam is split exactly as INTENT #99/#104 and `secrets.md` concern 5
lock it — **repo owns the linkage; secrets owns the push mechanics and never
lets repo see plaintext**:

```rust
// lib/repo::model — the linkage repo owns
pub struct SecretLink {
    pub repo: String,
    pub gh_name: String,              // the GH Actions secret NAME workflows reference
    pub secret: SecretRef,            // a handle into secrets — NEVER a value (concern 4 discipline)
    pub gh_scope: GhSecretScope,      // Repo | Environment{ gh_env: String }  (concern 5 scoping reality)
    pub workflows: Vec<String>,       // logical: which workflow(s) reference gh_name (informational)
    pub provenance: Provenance,
}
pub enum GhSecretScope { Repo, Environment { gh_env: String } }
```

- **On `push`**, repo walks the repo's `SecretLink`s and calls secrets' GH-Actions
  adapter once per link (`repo-secrets` → `GhSecretPush { owner, repo, gh_name,
  gh_scope, secret_ref }`, aligning with the `secrets.md` counterpart it already
  authored). **secrets** resolves the `SecretRef` → plaintext internally (non-LLM
  path), seals it with libsodium sealed-box under the repo's Actions public key,
  `PUT`s it to GitHub, and returns only a `GhPushReceipt { gh_name, pushed }`.
  **The value never returns to repo** — repo supplies linkage + identity only,
  satisfying use-without-seeing structurally.
- **Injection is part of the push path** (INTENT #104 "when repo pushes to the
  remote, the linked secrets get injected"), so a workflow that runs on the push
  finds its secrets already present. repo orders: push injected secrets →
  `git push` → (optionally) dispatch a workflow. A `SecretLink` removed →
  `GhSecretRemove` on next push (drift-safe one-way mirror, secrets.md concern 5).
- **`rollup` excluded** (concern 5): this edge carries only value push keyed by
  NAME, never templated content.

### 7. Branches/worktrees attachable to environments — the `repo-environments` seam (INTENT #75)

A branch or worktree is **attachable to an environment**, and
**deploying/merging it INTO an environment ACTIVATES that environment's
pipeline** (INTENT #75; `environments.md`):

- repo records the attachment (`BranchAttachment` / `WorktreeRecord.environment`,
  concern 2). The **environment concept, pipeline definition, and chaining are
  `environments`/`cicd`'s** — repo only names the attachment and emits the
  **deploy-into-environment event** that is the pipeline trigger.
- **Deploy trigger** = repo publishes a `repo.deployed { repo, branch/worktree,
  environment, head_sha, correlation_id }` event (`repo-environments` +
  `pubsub-protocol`); `environments`/`cicd` subscribe and activate the pipeline.
  repo does not run the pipeline — it fires the declarative trigger (INTENT #103:
  triggers declarative, code in handlers; the pipeline handler is cicd's).
- **The "strange" nuance is carried, not resolved** (environments.md; the
  environments-branches relationship is flagged strange for public-website
  deployments). repo's side stays minimal and boring: an attachment record + a
  deploy event. Because `environments` is a **batch-7 L6 stub**, repo pins the
  **v1-facing shape** it needs (`EnvRef`, the deploy event, the attachment) and
  states its requirement of the stub explicitly (see friction / the
  `repo-environments` proposal); the stub must honor that shape for repo's v1
  capability to stand (wave2-plan §5 flag 2).

```rust
pub struct EnvRef { pub project: String, pub env: String } // aligns with secrets' SecretScope::Environment
```

### 8. The cicd seam — cicd triggers/chains/observes THROUGH repo (`cicd-repo`, INTENT #104)

`cicd` is a **batch-7 L6 stub** (`cicd.md`), but repo must design its side of the
seam now (the brief's "design your side of the seam"). The locked division
(INTENT #104): **repo is the ONLY component touching the GH-Actions
directory/remote surface; cicd invokes repo's workflow operations and layers the
watch/verify loop on top.** repo exposes to cicd:

- **`trigger_workflow`** — manual dispatch (concern 5);
- **`compose_chain`** — write/adjust `workflow_run`/`workflow_call` wiring
  (declarative chain, a directory change) and/or register an **orchestrated
  chain** (repo dispatches step N+1 when cicd reports step N complete);
- **`observe_runs`** — list/get run status/conclusion/head-SHA/logs-URL (the
  boundary cicd polls; cicd owns the SHA-verify, App-Runner/CloudFront watching,
  and agent-in-loop retry logic — `cicd.md`, the Deployment-Chaperone grounding).

**The collapse case is designed for.** Whether cicd stays its own crate or
**emerges from repo** is OPEN (cicd.md; INTENT #104). repo's side is authored so
that if cicd emerges from repo, `cicd-repo` collapses into repo-internal module
boundaries (the `workflows` lib gains the watch/verify loop) with no contract
rework — the verbs above become internal calls. Designed as an external contract
so the OPEN question stays open cheaply.

### 9. Provenance + correlation across the push→workflow→deploy chain (INTENT #85/#92)

Provenance is first-order where data is touched (INTENT #85). repo touches data
at commits, pushes, secret injections, workflow dispatches, and deploy events —
each writes a `types::Provenance` record, **lighter than the DB plane** (INTENT
#92; like vfs's file provenance, not vdb's per-handler grain), and **stitched by
`correlation_id`** so a single causal chain is traceable end-to-end:

> `repo.push` → `repo-secrets` injection → GitHub-Actions run → `repo.deployed`
> event → cicd verify

all carry one `correlation_id`, so `spend`/`cicd`/the dashboard can trace a
deploy back to the exact push and the secrets it injected. This is the "every
time data gets touched" trace (INTENT #85) at repo's grain — who/what/when
pushed, which links injected, which workflow fired — without recording file-byte
access (that under-design is vfs's, and deliberate).

### 10. Addressing, concurrency, anchoring, restart (INTENT #59/#76/#77)

- **Addressing:** repo registers `AddressingClass::NodeScoped` (one instance per
  node hosting anchored repos — the same class as `vfs`). Callers use
  `Node{N,repo}` to reach the instance co-located with a worktree's anchor
  (INTENT #59 "talk to the service on a specific node"), or `AnyNode{repo}` for
  locality-preferred operations (clone-here, list-local). Git operations
  **must** run on the anchoring node (concern 1), so worktree-scoped verbs route
  by `WorktreeRecord.anchor_node`.
- **Concurrency:** the working tree is a `NodeAnchored` file — repo holds vfs's
  `vfs.anchor.<path>` **exclusive-writer lock** (a `locks-api` mutex, mesh-owned,
  vfs.md concern 12) around each git mutation window; a partition twin surfaces
  `locks`' `PartitionMergeExceeded` (INTENT #84), handled per-operation (a rare
  two-nodes-anchored-the-same-repo case → repo refuses the second, `AnchorLocked`).
  Concurrent git ops on one repo serialize per worktree behind that lock.
- **Restart interruptibility (`restart-protocol`, `types::restart.rs`):** a
  `clone`, `push`, `fetch`, or mid-commit is a
  `Interruptibility::CriticalSection` (a partial push/clone must not be
  interrupted for a non-critical update); repo answers `RestartResponse::Busy`
  below `SaveWindow` and `FinishAndRelinquish` for an in-flight sync, then
  snapshots and yields. Consumed, not authored.

### 11. Boring surface — CLI + daemon + surface schema + pub/sub

`bin/repo` is a noun-verb `clap` CLI over `lib/repo`, and a daemon registering
with the local mesh (single-port locality, `mesh-client`):

```
repo clone <url> [--slug S] [--node N]              # clone a GitHub remote into VFS
repo ls | repo worktree ls <repo>                   # list repos / worktrees (metadata)
repo branch create|delete <repo> <branch>
repo worktree add <repo> <name> --branch B [--thread T] [--env P/E]
repo worktree rm <repo> <name>
repo push|pull|fetch|sync <repo> [--worktree W]     # VFS <-> GitHub bridge
repo workflow add|update|rm <repo> <file>           # GH-Actions files (directory changes)
repo workflow run <repo> <workflow> [--ref R] [--input k=v]   # manual dispatch
repo workflow chain <repo> ...                      # declarative/orchestrated chains
repo workflow runs <repo> [--workflow W]            # observe run outcomes
repo secret link <repo> <gh_name> --ref <SecretRef> [--gh-env E]   # linkage (concern 6)
repo env attach <repo> <branch|worktree> --env P/E  # environment attachment (concern 7)
repo deploy <repo> <branch|worktree> --env P/E      # fires the pipeline trigger (concern 7)
```

The daemon publishes a `SurfaceSchema` (`types::surface`) rendering repos,
worktrees (with their branch/thread/environment links), workflows, runs, and
secret links — never a secret VALUE (only `SecretRef` handles). Every
interactive element carries a stable semantic id (INTENT #16 first-order agent
interface). A `repo.*` pub/sub topic prefix (`pubsub-protocol`, additive leaf
under pubsub-relay's `<slug>.*` taxonomy) emits `repo.cloned`, `repo.pushed`,
`repo.worktree.added`, `repo.workflow.dispatched`, `repo.workflow.run.completed`,
`repo.secret.injected`, `repo.deployed`. `RepoError` lands in `types::error::repo`
(a reserved slot to request from types — see friction).

## Relationships / edges

Contract edges (cross-process WS/wire over mesh-transport `:3649`; client halves
via `mesh-client`):

- **vfs** via `repo-vfs` — repo materializes git trees/worktrees as `NodeAnchored`
  VFS files; `OpenAnchored`/`Snapshot` at commit/fetch boundaries; ordinary
  `Read`/`Write` for the `.repo/` manifest (concern 1/2).
  *(scaffold/contracts/repo-vfs.md — exists, requirements-only; content proposed
  below.)*
- **secrets** via `repo-secrets` — (a) resolve repo's GitHub auth credential
  use-without-seeing for push/clone (concern 4); (b) GH-Actions
  injection-on-push: repo names the linkage, secrets' adapter pushes the value
  (concern 6). Aligns with `secrets.md`'s already-authored `repo-secrets`
  counterpart (`GhSecretPush`/`GhSecretRemove`/`GhPushReceipt`).
  *(authored: scaffold/contracts/repo-secrets.md; secrets.md authored its side.)*
- **environments** via `repo-environments` — branch/worktree → environment
  attachment + the deploy-into-environment event that triggers the pipeline
  (concern 7). *(scaffold/contracts/repo-environments.md — exists,
  content authored at the contract round.)*
- **cicd** via `cicd-repo` — cicd triggers/chains/observes workflows through repo
  (concern 8; repo authors its side; cicd is a batch-7 stub).
  *(scaffold/contracts/cicd-repo.md — exists, requirements-only, layer-6.)*
- **mesh** — registration/resolution (`service-lookup`, NodeScoped); no separate
  contract stub (rides `service-lookup`).

Cross-cutting protocols (surface-schema-style, repo a party, authored by their
owners — repo consumes the shapes, does not re-author): `pubsub-protocol` (the
`repo.*` topic prefix), `surface-schema` (repo's boring panel), `restart-protocol`
(repo is supervised; a push/clone is a `CriticalSection` — concern 10),
`service-lookup` (register NodeScoped; resolve vfs/secrets/environments).

Internal-lib seams (compiled-in, NOT contract edges): `mesh-client`
(register/resolve/pubsub/locks handles), `substrate-types` (`provenance`/`node`/
`surface`/`restart`/`pubsub`/`error` + `SecretRef`/`SecretScope` from
`types::secrets` + the new `types::repo` vocabulary), and `git2`/libgit2 behind
the `GitBackend` trait.

**GitHub itself is not a substrate contract edge** — it is an external system
repo reaches over HTTPS (the GitHub REST API + git transport), authenticated with
a secret. Only substrate-internal pairs are contracts.

## Nesting

Parent: none (top-level app-crate `bin/repo` + `lib/repo` — `substrate-repo`).
Children (nested internal libs, compiled into the repo daemon, never standalone):
**`git`** (the `GitBackend` trait + libgit2/CLI impls + clone/fetch/push/branch/
worktree ops against the anchored path), **`model`** (RepoRecord/WorktreeRecord/
BranchAttachment/SecretLink registry + the per-node `repo.db` index +
vfs-manifest persistence), **`workflows`** (GH-Actions file management, dispatch,
chains, run observation, the GitHub REST client), **`sync`** (the VFS↔GitHub
bridge orchestration + injection-on-push ordering + provenance/correlation),
**`surface`** (surface-schema + `repo.*` pub/sub + the `clap` CLI). These are
libraries under the repo app per INTENT #22, not top-level crates. The
parent/child structure lives here + overview.md, not the directory layout (flat
`components/`).

## Thoroughness level

**implementation-ready** for: the git-on-VFS execution model (drive real git
against a vfs `OpenAnchored` path, snapshot at git boundaries — concern 1); the
repo/branch/worktree model reusing the `.mind` schema + the metadata-location
decision (concern 2); the clone/branch/worktree lifecycle (concern 3);
push/sync with use-without-seeing GitHub auth (concern 4); GH-Actions workflow
management v1 — files/dispatch/chains/observation, rollup-excluded (concern 5);
secrets-injection-on-push and the `SecretLink` linkage model aligned with
secrets' authored counterpart (concern 6); the cicd seam incl. the
emerges-from-repo collapse case (concern 8); provenance/correlation across the
chain (concern 9); addressing/anchoring/concurrency/restart (concern 10); and the
boring CLI/surface/pubsub (concern 11).
**approach-sketched** for: the `repo-environments` attachment + deploy-event
shape (`environments` is a **batch-7 L6 stub** — repo pins the v1-facing shape it
needs and states its requirement of the stub; the "strange" public-website
nuance stays carried, concern 7); the substrate-environment ↔ GitHub-deployment-
environment mapping for env-scoped Actions secrets (concern 5 flag); and the
GitBackend libgit2-vs-CLI split per verb (fill-time latitude).

## Assigned design-depth

**Opus** (single Component-Designer pass, this file), per the wave2-plan batch-6
tier ("repo needs batch-3's vfs/secrets" — both now designed). Grounded in
INTENT #67/#80/#100/#104/#85/#92/#99/#105/#47/#48, the batch-3 `vfs.md`
(NodeAnchored file class + `OpenAnchored`/`Snapshot` surface) and `secrets.md`
(the GH-Actions adapter + the `repo-secrets` counterpart already authored), the
batch-7-facing `environments.md`/`cicd.md` stubs, types.md
(`provenance`/`node`/`restart`/`surface`/`secrets` vocabulary), and the
operator's live `.mind` worktree↔thread schema (`core:workspaces`).

## Suggested fill-model

**implementation-ready + medium-high complexity → strong-mid model, two surfaces
tests-first.** Most of repo is boring transcription against frozen seams — the
model records + `repo.db` index, the clone/branch/worktree verbs over
`GitBackend`, the workflow-file/dispatch/observe GitHub-REST surface, the
CLI/surface/pubsub. **Two surfaces want conformance tests written FIRST and must
not go to a cheap tier:** (1) the **git-on-VFS snapshot/anchor loop** (concern
1/10) — its failure mode is a repo that reports pushed/committed but whose
snapshot didn't replicate, or a git op running without the anchor lock; test
over a multi-node in-process harness (fake `OpenAnchored`/`Snapshot`, assert: a
commit snapshots and replicates; a crash between snapshots loses only uncommitted
edits; a second node cannot mutate while the anchor lock is held; a worktree-add
that would need a cross-node `.git` is refused); (2) the **injection-on-push
ordering + no-plaintext invariant** (concern 6) — a property test that on `push`,
every `SecretLink` triggers a `GhSecretPush` *before* the git push, that repo
never receives a plaintext value back (only `GhPushReceipt`), and that a removed
link emits `GhSecretRemove`. Everything else (workflow files, chains, run
observation, environment attachment, provenance) is transcription-grade. Fill
**after** vfs and secrets are filled (repo rides both) and **alongside** cicd's
batch-7 stub only for the `cicd-repo` seam shape.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `repo-vfs` — (repo → vfs) — NodeAnchored git-tree hosting (vfs.md authored the storage side). → `scaffold/contracts/repo-vfs.md`
- `repo-secrets` — (repo ↔ secrets) — GitHub auth (use-without-seeing) + injection-on-push (secrets.md authored the push side). → `scaffold/contracts/repo-secrets.md`
- `repo-environments` — (repo ↔ environments) — branch/worktree attachment + the deploy trigger (environments is a batch-7 stub). → `scaffold/contracts/repo-environments.md`
- `cicd-repo` — (cicd → repo) — workflow trigger/chain/observe through repo (cicd is a batch-7 stub; repo authors its side). → `scaffold/contracts/cicd-repo.md`

Also a party to (authored elsewhere / cross-cutting): `vfs-content` — see `scaffold/contracts/`.

