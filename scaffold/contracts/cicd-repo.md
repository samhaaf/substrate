# Contract: cicd-repo

## Parties
cicd  ->  repo

## What the edge carries
**Layer-6 anticipated contract (round-9) — cicd is not implemented in v1.**
cicd kicks off GitHub Actions workflows manually and sets up workflow
chains **through repo** — repo owns the full GH-Actions workflow-management
surface in v1 (workflows are directory changes; secrets injection on push;
see `components/repo.md`), so cicd never touches the GH-Actions
directory/remote surface directly. Anticipated verbs: trigger a workflow,
compose/chain workflows, observe run outcomes (the watch/verify loop
itself is cicd's own scope — see `components/cicd.md`). Whether cicd stays
its own crate or emerges from repo is OPEN; if it emerges from repo this
contract collapses into repo-internal structure. Schema/example deferred.
**requirements-only, layer-6** (round-9 lock, 2026-07-19).
