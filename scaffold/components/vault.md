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
- **LLM-safety mechanism LOCKED (round-7, 2026-07-19, from the operator's
  prior professional work):** vault values are addressable in **raw** and
  **ID** forms. Any service known to be feeding an LLM defaults to
  **`llm_safe=true`**; under `llm_safe`, requesting a secret's raw form
  either **fails** or **gracefully degrades to injecting the ID plus a
  warning** ("this was attempted to be injected raw; we've injected an ID
  instead"). **The invariant: no secret ever reaches an LLM** — confirmed.
- **Rollup interaction (round-7):** the rollup engine must NEVER resolve a
  vault raw reference inside LLM-bound content — the `llm_safe`
  fail-or-degrade behavior above governs that path. See
  `components/rollup.md`.
- **Everything else is OPEN** — storage backend, encryption at rest, human
  access paths, rotation, scoping/namespacing, and how non-agent code reads
  secrets are all undesigned. This stub exists to hold the locked
  invariant(s).

## Relationships / edges (stubs only)

- **mesh** via `vault-mesh` — registration/resolution via the local mesh
  daemon (single-port locality); the shape of secret brokerage through mesh
  is TBD (scaffold/contracts/vault-mesh.md).
- **rollup** (round-7 relationship note) — vault values are addressable in
  raw and ID forms from rollup content; rollup must never resolve a vault
  raw reference inside LLM-bound content (`llm_safe` fail-or-degrade applies;
  see `components/rollup.md`). Edge naming deferred until either side gets a
  design pass.
- Consumers (ccd's agents, org's agents, stack/db handlers reaching third
  parties, cicd deploys) are anticipated but not yet edges — deferred until
  vault gets a design pass.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — locked: use-without-seeing for agents/LLMs, and the
round-7 `llm_safe` raw/ID mechanism (no secret ever reaches an LLM);
everything else open.
