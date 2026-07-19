# Contract: repo-secrets

> New pair (wave2-plan §3b). Authored from secrets.md concern 5 / its
> `repo-secrets` proposal (the push-mechanics side) and repo.md concern 4/6 / its
> `repo-secrets` proposal (the linkage + git-auth side). One small extension
> reconciled — see notes.

## Parties

repo (L5 app) ↔ secrets (L3 app)

Two facets, each with a clear owner:
- **git auth** — repo *consumes* a credential from secrets;
- **injection-on-push** — repo owns the **linkage**, secrets owns the **push
  mechanics** and never lets repo see plaintext.

## Purpose

1. **GitHub auth, use-without-seeing (concern 4).** git2's credentials callback
   needs a token; repo resolves the **raw** GitHub credential (PAT or GitHub-App
   installation token) from secrets. repo is a **trusted, non-`llm_safe`
   service** (its mesh identity is `repo`, not an LLM-feeding slug; `llm_safe` is
   set from the unspoofable caller identity), so raw resolution is legitimate —
   the plaintext flows secrets→repo's credential callback and never into any LLM
   path (repo runs no completions).
2. **GH-Actions injection-on-push, THE v1 capability (INTENT #104, concern 6).**
   On `git push`, repo walks the repo's `SecretLink`s and asks secrets' GitHub-
   Actions adapter to push each linked value: fetch the repo's Actions public
   key, libsodium sealed-box the value, `PUT` it to GitHub. **secrets resolves
   the `SecretRef`→plaintext internally, seals, PUTs, and returns only a
   receipt** — the value never returns to repo. repo supplies linkage + identity
   only, satisfying use-without-seeing structurally.

## Schema

Reused from `types::secrets`: `SecretRef`, `SecretScope`, `CallerContext`,
`SecureValue`, `AdapterKind`. From `types::repo`: `GhSecretScope`.

```rust
// (a) git auth: repo -> secrets  (raw allowed — repo is a trusted non-LLM caller)
struct ResolveGitCredential { r#ref: SecretRef, caller: CallerContext }  // caller.llm_safe = false
                           // -> SecureValue   (the PAT/installation token; confidential in-mesh channel)

// (b) injection-on-push: repo -> secrets  (secrets seals + PUTs; value never returns)
struct GhSecretPush   { owner: String, repo: String, gh_name: String,
                        gh_scope: GhSecretScope, r#ref: SecretRef }
struct GhSecretRemove { owner: String, repo: String, gh_name: String, gh_scope: GhSecretScope }
enum   GhSecretScope  { Repo, Environment { gh_env: String } }   // GitHub's scoping reality (concern 5)

// secrets -> repo
struct GhPushReceipt  { gh_name: String, pushed: bool }          // linkage + identity in; only a receipt out
```

`SecureValue` (secrets.md) denotes a value crossing a confidential in-mesh
channel (mesh-relayed WS between two trusted services) — never an LLM path.
`GhSecretScope` carries GitHub's reality that Actions secrets are repo-, GitHub-
Environment-, or org-scoped (never per-workflow); repo's logical
"workflow W needs secret NAME X" linkage is *pushed* at the GH scope GitHub
supports.

## Error cases

- `RemoteAuthFailed { remote }` — the resolved GitHub credential was rejected by
  GitHub (repo-side `RepoError`).
- secrets-side `AdapterFailed { GitHubActions, detail }` (GH API/auth failure —
  the GH PAT is itself a Global secret) / `NotFound { ref }` — surfaced to repo as
  `RepoError::SecretLinkUnresolved { gh_name }` or `RepoError::GhApiFailed`
  (stringified at the boundary; secrets' error never becomes a repo type).
- `NotDecryptable` / `KeychainUnavailable` — on an unenrolled node secrets can't
  decrypt the `SecretRef`; repo cannot push that link (a correct hard refusal, not
  a silent skip).

## Version sensitivity

- **LOW.** The GH Actions secrets API is stable; the linkage shape is repo's to
  evolve. New fields `#[serde(default)]`; `GhSecretScope` reserves
  `#[serde(other)]`.
- **`rollup` is EXCLUDED** (INTENT #104, repo.md concern 5): this edge carries
  ONLY value push (keyed by NAME) + auth resolution — never templated content. A
  structural boundary, part of the contract.
- **FROZEN — the no-plaintext-returns-to-repo arm:** repo receives only
  `GhPushReceipt` from the push path (never the sealed or raw value). Part of the
  contract text, not commentary — the use-without-seeing guarantee for the
  injection path.

## Reconciliation notes

- **`gh_scope` added to `GhSecretPush`/`GhSecretRemove` — repo's richer shape
  WINS, secrets' simpler shape is the loser (but additive-compatible).**
  secrets.md's authored `GhSecretPush { owner, repo, gh_name, ref }` omitted the
  scope; repo.md added `gh_scope: GhSecretScope { Repo | Environment{gh_env} }` to
  reflect GitHub's real constraint that Actions secrets have no per-workflow scope
  (repo.md concern 5). repo's version wins because it captures a real GitHub
  reality secrets **must** honor to push to the right place; secrets' simpler
  shape is recorded as the losing position and is a strict subset (adding
  `gh_scope` is additive). secrets' adapter reads `gh_scope` to choose repo-level
  vs GitHub-deployment-environment-level `PUT`.
- **`ResolveGitCredential` is repo's addition; secrets' general model covers
  it.** secrets.md did not author a git-auth-specific verb — its general
  resolve/`use` model with `llm_safe=false` for a non-LLM caller already permits
  raw resolution. Pinned here as a named verb `ResolveGitCredential` for clarity
  at repo's credential-callback site; it is an *instance* of secrets' raw resolve,
  not a new mechanism. No disagreement — recorded so secrets' fill exposes the
  named alias.
- **GitHub "Environments" name collision carried, not resolved.** GitHub's
  deployment Environments (env-scoped Actions secrets) are a *different concept*
  from substrate `environments` (INTENT #75). When a substrate environment maps
  to a GitHub deployment environment for env-scoped Actions secrets, that mapping
  is **explicit repo/secrets state** (`GhSecretScope::Environment{gh_env}`), NOT
  an implicit identity. Surfaced to the environments stub (environments.md
  friction) — `EnvRef.env` and `gh_env` are deliberately distinct strings.

## Example data

repo, on **macbook**, pushes the `demo` repo's `v1` branch; it first resolves the
GitHub PAT to authenticate, then injects the one linked Actions secret
(`OPENROUTER_KEY` at repo scope) before the git push:

```jsonc
// (a) repo -> secrets: resolve the GitHub credential (raw, non-llm_safe)
{ "ref": { "id": "a1b2c3d4-0000-4000-8000-000000000001", "name": "github-pat",
           "scope": "Global", "version": 3 },
  "caller": { "service_slug": "repo", "llm_safe": false, "purpose": "push demo v1" } }
// secrets -> repo: SecureValue (the PAT; used only in git2's credentials callback, never logged)
{ "SecureValue": "<github-pat plaintext over confidential in-mesh channel>" }

// (b) repo -> secrets: inject the linked Actions secret BEFORE git push
{ "owner": "samhaaf", "repo": "demo", "gh_name": "OPENROUTER_KEY",
  "gh_scope": "Repo",
  "ref": { "id": "a1b2c3d4-0000-4000-8000-000000000002", "name": "openrouter-key",
           "scope": { "Project": { "project": "demo" } }, "version": 1 } }
// secrets resolves -> plaintext -> sealed-box under demo's Actions public key -> PUT to GitHub
// secrets -> repo: only the receipt (value never returns)
{ "gh_name": "OPENROUTER_KEY", "pushed": true }

// repo then runs `git push origin v1`; a workflow triggered by the push finds
// OPENROUTER_KEY already present. correlation_id c0110000-…-0001 threads the
// whole push→inject→workflow chain (repo.md concern 9).
```
