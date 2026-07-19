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
  deployments. **OPEN:** whether that lives inside `environments` or
  interacts with mesh routing — "there may be some interaction between
  network and environments, or it may be inside environments, not sure."
- **Nested projects — naive rule for now:** "assume there is no relationship
  between the environments of different projects; if there IS a relationship
  between a project and its sub-projects, they interface over the same
  environment by default" — i.e. a sub-project shares its parent's
  environment by default.
- **Consumers already known:** `cicd` consumes environments; CCD threads are
  optionally linkable to environments within projects (see
  `components/ccd.md`); branches/worktrees attach to environments in the open
  `repo`-vs-VFS-git fork (see `overview.md`).

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

## Nesting

Parent: none (working assumption) | Children: none (this pass). Hierarchy
vs. `cicd` OPEN.

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
Open: bandit placement (environments vs. mesh routing), hierarchy vs. cicd.
