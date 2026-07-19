# Contract: secrets-environments

> **STUB-TRACK / layer-6 — environments is NOT implemented in v1.** Lighter file:
> parties, purpose, a rough schema sketch, open questions. No full example world.
> Authored from secrets.md concern 6 / its `secrets-environments` note and
> environments.md's `secrets-environments` stub. Both sides agree "no new
> mechanism."

## Parties

secrets (L3 app, live-track) ↔ environments (L6 stub, NOT built in v1)

secrets already has the mechanism (scoped push over its existing adapters);
environments supplies the *routing decision* (which backing store an environment
maps to). Content is deferred until environments leaves the stub track.

## Purpose

Push/scope secrets into a **specific environment** (INTENT #75/#99). No new
mechanism — an environment-addressed push over secrets' existing adapters. It
rides secrets' `SecretScope::Environment { project, env }` (the same
`(project, env)` as `EnvRef`), with resolution **most-specific-wins** along the
fallback chain `Environment → Database → Project → Global` (secrets.md concern 6)
so an environment can override a project default without duplicating unchanged
secrets. environments' role is to say **which backing store / adapter target an
environment maps to** (local mesh DB, Supabase Vault, AWS Secrets Manager, or a
GitHub deployment environment) so secrets pushes to the right place.

## Schema (rough sketch — content deferred)

Reused from `types::secrets`: `SecretScope`, `SecretRef`, `AdapterKind`,
`AdapterTarget`. From `types::repo`: `EnvRef` (identity-shared).

```rust
// SecretScope::Environment is the identity link (already exists in secrets)
//   SecretScope::Environment { project: String, env: String }   ==  EnvRef { project, env }

// rough anticipated shape — NOT frozen:
// environments -> secrets: bind an environment to an adapter target
struct EnvSecretTarget {
    env: EnvRef,
    adapter: AdapterKind,          // Supabase | LocalMeshDb | GitHubActions | AwsSecretsManager
    target: AdapterTarget,         // e.g. Supabase project_ref, or a gh_env, or an SM region/name prefix
}
// secrets -> environments: (pull-style) which secrets are scoped to this environment
struct EnvSecretList { env: EnvRef }   // -> Vec<SecretMeta>   (metadata only, never values)
```

Deploy-time push: when a branch/worktree deploys into `env` (repo's
`DeployedEvent`, `repo-environments`), the environment's `EnvSecretTarget`
tells secrets where to mirror the `Environment`-scoped secrets — reusing the
existing `SecretsAdapter::push`, no new push mechanism.

## Reconciliation notes

- **No disagreement — both sides say "no new mechanism."** secrets.md: "an
  environment-addressed push over secrets' existing adapters"; environments.md:
  "rides secrets' `SecretScope::Environment`… environments' role is which backing
  store an environment maps to." Merged; nothing frozen (stub-track).
- **Identity shared across the cluster:** `SecretScope::Environment {project,env}`
  ≡ `EnvRef {project,env}` ≡ the `env` in `repo-environments` — one `(project,
  env)` identity, deliberately consistent so the examples compose. Distinct from a
  *GitHub* deployment environment (`gh_env`), which is a separate string carried
  on `GhSecretScope::Environment` (`repo-secrets`) — the collision is flagged, not
  identified.
- **llm_safe unchanged:** environment-scoped secrets obey the same invariants —
  an LLM-feeding caller resolving one still degrades to a reference; only trusted
  non-`llm_safe` sinks (a deploy step, an adapter) get raw.

## Open questions (deferred to environments' real design pass)

1. **Full schema + example data + version-sensitivity** — deferred until
   environments leaves the stub track; `EnvSecretTarget`/`EnvSecretList` are rough
   shapes, not frozen.
2. **Adapter-target routing** — how an environment's `EnvSecretTarget` is chosen
   (operator-set? derived from the environment's vdb/storage routing via
   `environments-vdb`?) is undesigned.
3. **The GitHub deployment-environment mapping** — when a substrate environment
   maps to a GitHub deployment environment for env-scoped Actions secrets, the
   `gh_env` mapping must be explicit environments/secrets state, not an implicit
   identity (environments.md friction).
