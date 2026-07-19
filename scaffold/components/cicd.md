# cicd

**Status:** NEW (rounds 4–5 lock, 2026-07-18). **Nesting:** top-level under
the working assumption (sibling of `environments`, with a dependency on it);
hierarchy OPEN.
**Requirements-only stub — NOT a full design.**

## Charter (requirements)

CICD pipelines for the mesh: a pipeline attaches to an **environment**
(`components/environments.md`), and is **activated when a branch or worktree
is deployed/merged into that environment** — with real-world consequences.
Operator: "We need crates for these things. I don't know exactly what the
hierarchy is — environments might be a sibling of CICD, might be a child;
CICD probably has to use environments."

Requirements:

- **Consumes environments** — pipelines are attached to and triggered
  through them; cicd does not own the environment concept.
- **Chained deployments** (requirement stated on the environments side):
  stage → checks → green prod, old prod flips to blue — the pipeline chain
  is what drives the promotion.
- **Hierarchy OPEN:** sibling vs. child of `environments`;
  **sibling-with-dependency is the working assumption** recorded here.

## Relationships / edges

- **environments** — the one known dependency (see hierarchy note above). No
  contract stub until the hierarchy is decided.

## Nesting

Parent: none (working assumption) | Children: none (this pass).

## Thoroughness level

**requirements-only** — no design pass yet. Open: hierarchy vs. environments.
