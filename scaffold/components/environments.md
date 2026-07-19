# environments

**Status:** NEW (rounds 4–5 lock, 2026-07-18). **Nesting:** top-level (but see
the open hierarchy question vs. `cicd`).
**Requirements-only stub — NOT a full design.**

## Charter (requirements, operator's words where quoted)

An **environment is a subset of a project** — richer than a
replicated-store row. Requirements:

- **A CICD pipeline can be attached** to an environment.
- **Deploying/merging a branch or worktree INTO an environment ACTIVATES its
  pipeline, with real-world consequences.** Deployment-into-environment is
  the trigger; the pipeline is what runs.
- **Chained deployments are wanted:** stage → checks → green prod, with the
  old prod flipping to blue (blue/green promotion driven by the chain).
- **Feature load-balancing via multi-armed bandits** over quick production
  deployments. **RESOLVED round-6 (supersedes the OPEN placement question):**
  bandits are **NOT a mesh concern** — the earlier "interaction between
  network and environments" musing referred to complex **in-app behavior**
  inside a public-facing website deployed via projects/environments. The
  bandit logic is an application-level concern, not mesh routing.
- **Standing note (round-6):** whatever we do with environments "should be
  sufficiently boring, predictable and stable, but the user tends to do
  really creative patterns, so it shouldn't be overly rigid."
- **The environments-branches relationship is flagged "strange"** for
  public-website deployments (round-6) — recorded as an unresolved nuance,
  not smoothed over; see the `repo-environments` contract stub.
- **Nested projects — naive rule for now:** "assume there is no relationship
  between the environments of different projects; if there IS a relationship
  between a project and its sub-projects, they interface over the same
  environment by default" — i.e. a sub-project shares its parent's
  environment by default.
- **Consumers already known:** `cicd` consumes environments; CCD threads are
  optionally linkable to environments within projects (see
  `components/ccd.md`); branches/worktrees attach to environments via the
  **`repo` crate — fork RESOLVED round-6** (repo is its own crate on top of
  the VFS; supersedes "the open `repo`-vs-VFS-git fork") — see
  `components/repo.md` and the `repo-environments` contract stub.

## Relationships / edges

- **cicd** — consumes environments; **hierarchy OPEN** (sibling vs. child;
  "environments might be a sibling of CICD, might be a child; CICD probably
  has to use environments"). **Working assumption:
  sibling-with-dependency** (cicd depends on environments). No contract stub
  until the hierarchy is decided.
- **projects** — an environment is a subset of a project; the
  nested-projects naive rule above. Edge naming deferred.
- **ccd** — threads optionally link to environments (recorded on CCD's usage
  database; see `components/ccd.md`). No contract stub yet.
- **repo** via `repo-environments` (NEW round-6) — branches/worktrees
  attachable to environments; deploy-into-environment activates the pipeline
  (scaffold/contracts/repo-environments.md).

## Nesting

Parent: none (working assumption) | Children: none (this pass). Hierarchy
vs. `cicd` OPEN.

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
Open: hierarchy vs. cicd; the "strange" environments-branches relationship
for public-website deployments. (Bandit placement RESOLVED round-6: not
mesh — an in-app concern.)
