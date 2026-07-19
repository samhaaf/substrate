# Contract: repo-environments

## Parties
repo  <->  environments

## What the edge carries
Branches/worktrees (repo-owned) are **attachable to environments**;
deploying/merging a branch or worktree into an environment ACTIVATES that
environment's pipeline (the trigger recorded in
`components/environments.md`). The environments-branches relationship is
flagged **"strange"** for public-website deployments — unresolved nuance,
recorded not smoothed over. Schema/example deferred.
**requirements-only** (round-6 lock, 2026-07-19).
