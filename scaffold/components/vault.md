# vault

**Status:** NEW (round-6 lock, 2026-07-19). **Nesting:** top-level.
**Requirements-only stub — NOT a full design.**

## Charter (requirements, operator's words where quoted)

The **secrets crate**. One defining, locked requirement — verbatim: "By
default, agents can never directly access a secret, but they can use it in a
context."

- **Use-without-seeing, enforced at the access-pattern level for
  agents/LLMs:** an agent/LLM can cause a secret to be *used* in a context —
  injected into a request, an env var, a deploy, a connection string — but
  can **NEVER directly read the secret's value** into its own
  context/transcript. The access patterns themselves must make the
  distinction structural (there is no "read secret" surface exposed to an
  agent, only "use secret here" surfaces).
- **Everything else is OPEN** — storage backend, encryption at rest, human
  access paths, rotation, scoping/namespacing, and how non-agent code reads
  secrets are all undesigned. This stub exists to hold the one locked
  invariant.

## Relationships / edges (stubs only)

- **mesh** via `vault-mesh` — registration/resolution via the local mesh
  daemon (single-port locality); the shape of secret brokerage through mesh
  is TBD (scaffold/contracts/vault-mesh.md).
- Consumers (ccd's agents, org's agents, stack/db handlers reaching third
  parties, cicd deploys) are anticipated but not yet edges — deferred until
  vault gets a design pass.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — one locked invariant (use-without-seeing for
agents/LLMs); everything else open.
