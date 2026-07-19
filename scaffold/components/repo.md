# repo

**Status:** NEW (round-6 lock, 2026-07-19 — **RESOLVES the rounds-4–5 open
fork** "repo crate vs git-in-the-VFS"). **Nesting:** top-level.
**Requirements-only stub — NOT a full design.**

## Charter (requirements, operator's words where quoted)

**Fork RESOLVED: `repo` IS its own crate, built on top of the VFS** —
git-semantics-inside-the-VFS is rejected. Both VFS and repo **stay boring**;
verbatim: "the only not-boring thing about repo is that it's built on top of
a virtual file system instead of a normal one."

Requirements:

- **Push/sync between the VFS and git repos / GitHub** — repo manages the
  bridge between VFS-resident trees and real git remotes.
- **Branches.**
- **Worktrees** — "I like worktrees. I like workspaces."
- **Branches attachable to environments** — deploying/merging a branch or
  worktree into an environment activates that environment's pipeline (see
  `components/environments.md`; the environments-branches relationship is
  flagged "strange" for public-website deployments — recorded there).
- **Prior art:** the operator's `.mind` workspace schema already links
  worktrees + Claude Code threads — reuse it when this is designed (carried
  over from the rounds-4–5 open-fork note).

## Relationships / edges (stubs only)

- **vfs** via `repo-vfs` — repo sits on top of the VFS: repo state lives in /
  is materialized through VFS storage; push/sync bridges VFS trees to
  git/GitHub (scaffold/contracts/repo-vfs.md).
- **environments** via `repo-environments` — branches/worktrees attach to
  environments; deploy-into-environment is the pipeline trigger
  (scaffold/contracts/repo-environments.md).
- **mesh** — registration/resolution like every service; no separate contract
  stub yet (rides `service-lookup`).

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — the fork resolution plus verbatim requirements; no
design pass yet.
