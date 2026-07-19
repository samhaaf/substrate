# Contract: repo-vfs

## Parties
repo  ->  vfs

## What the edge carries
Repo sits **on top of the VFS** (round-6 fork resolution: repo is its own
crate; git-in-the-VFS rejected): repo trees/branches/worktrees live in and
are materialized through VFS storage, and repo manages push/sync between that
VFS-resident state and real git repos / GitHub. "The only not-boring thing
about repo is that it's built on top of a virtual file system instead of a
normal one" — both sides stay boring. Schema/example deferred.
**requirements-only** (round-6 lock, 2026-07-19).
