# repo

**Status:** NEW (round-6 lock, 2026-07-19 — **RESOLVES the rounds-4–5 open
fork** "repo crate vs git-in-the-VFS"; **round-8 scope addition: GitHub
Actions management**). **Nesting:** top-level.
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
- **GitHub Actions management (round-8 scope addition, 2026-07-19).** repo
  owns "a clean way to manage GitHub Actions on repos": **managing
  workflows on repos**, and **linking secrets into GH Actions workflows** —
  repo owns the linkage of a secret to a workflow, while the `secrets`
  service's GitHub-Actions adapter is what pushes the secret values into GH
  Actions secrets (see `components/secrets.md`).

## Relationships / edges (stubs only)

- **vfs** via `repo-vfs` — repo sits on top of the VFS: repo state lives in /
  is materialized through VFS storage; push/sync bridges VFS trees to
  git/GitHub (scaffold/contracts/repo-vfs.md).
- **environments** via `repo-environments` — branches/worktrees attach to
  environments; deploy-into-environment is the pipeline trigger
  (scaffold/contracts/repo-environments.md).
- **mesh** — registration/resolution like every service; no separate contract
  stub yet (rides `service-lookup`).
- **secrets** (round-8) — linking secrets into GH Actions workflows: repo
  names the linkage; `secrets`' GH-Actions adapter pushes the values. Edge
  naming deferred until either side gets a design pass
  (see `components/secrets.md`).

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — the fork resolution plus verbatim requirements
(round-8 adds the GitHub Actions management scope); no design pass yet.
