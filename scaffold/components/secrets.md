# secrets

**Status:** NEW (round-6 lock, 2026-07-19); **RENAMED `vault` → `secrets`
(round-8 lock, 2026-07-19).** **Nesting:** top-level.
**Requirements-only stub — NOT a full design.**

> **NAMING HISTORY (round-8, LOCKED):** this component was born **`vault`**
> (round-6). "vault" is REJECTED because it collides with **Supabase Vault**
> — one of this very service's push targets; "SM" was rejected for colliding
> with **AWS Secrets Manager**; safe/locker/shelf also rejected. Operator:
> "secrets is the name because that's what it is." The old `vault-mesh`
> contract is renamed `contracts/secrets-mesh.md`. Any pre-round-8 "vault"
> wording in this scaffold's history refers to this component; verbatim
> operator quotes from earlier rounds keep the word "vault" as spoken.

## Charter (requirements, operator's words where quoted)

The **secrets crate/service**. One defining, locked requirement — verbatim:
"By default, agents can never directly access a secret, but they can use it
in a context."

- **Use-without-seeing, enforced at the access-pattern level for
  agents/LLMs:** an agent/LLM can cause a secret to be *used* in a context —
  injected into a request, an env var, a deploy, a connection string — but
  can **NEVER directly read the secret's value** into its own
  context/transcript. The access patterns themselves must make the
  distinction structural (there is no "read secret" surface exposed to an
  agent, only "use secret here" surfaces).
- **LLM-safety mechanism LOCKED (round-7, 2026-07-19, from the operator's
  prior professional work):** secret values are addressable in **raw** and
  **ID** forms. Any service known to be feeding an LLM defaults to
  **`llm_safe=true`**; under `llm_safe`, requesting a secret's raw form
  either **fails** or **gracefully degrades to injecting the ID plus a
  warning** ("this was attempted to be injected raw; we've injected an ID
  instead"). **The invariant: no secret ever reaches an LLM** — confirmed.
- **Rollup interaction (round-7):** the rollup engine must NEVER resolve a
  secrets raw reference inside LLM-bound content — the `llm_safe`
  fail-or-degrade behavior above governs that path. See
  `components/rollup.md`.
- **Distribution (round-8):** managed by mesh, and **distributed across
  nodes — or replication factor ~3** ("vault is just important" — operator,
  pre-rename wording). A single-node secret store is not acceptable.
- **Adapters (round-8):** the service owns **adapters that PUSH secrets into
  specific environments**: **Supabase Vault**, **AWS Secrets Manager**, and
  **GitHub Actions secrets**. (`db`'s existing Supabase-Vault module is the
  natural seed of the Supabase adapter — see the db note below.)
- **Interfaces (round-8):** `secrets` interfaces with **VDB, db, projects,
  environments, and repo** (repo's slice: linking secrets into GitHub
  Actions workflows — see `components/repo.md`). Anticipated interfaces,
  not yet contract edges.
- **S3 encryption keys live here (round-8 yes-block confirmation):** the
  client-side-encryption keys used by mesh's S3 adapter are held by the
  `secrets` service (see the S3-adapter capability in `components/mesh.md`).
- **Must be boring and "not choppy."**
- **Everything else is OPEN** — storage backend, encryption at rest, human
  access paths, rotation, scoping/namespacing, and how non-agent code reads
  secrets are all undesigned. This stub exists to hold the locked
  invariant(s) and the round-8 requirements.

**db reconciliation (round-8):** `lib/db` already contains real secret
handling today — a full Supabase-Vault module (`lib/db/src/vault.rs`, plus
the `db vault` CLI surface) and the PATs-from-the-OS-keychain-at-runtime
convention ("Secrets NEVER live here" — `etc/db.example.toml`). Going
forward, **`secrets` is the OWNER of secret management**; `db`'s own secret
handling gets reconciled to consume `secrets`, and **a real `db`-crate
update is operator-authorized when that design lands** ("this might be one
of the times where we actually have to update the db crate"). Facts +
direction recorded in `components/db.md`.

## Relationships / edges (stubs only)

- **mesh** via `secrets-mesh` (RENAMED from `vault-mesh` round-8) —
  registration/resolution via the local mesh daemon (single-port locality);
  the shape of secret brokerage through mesh is TBD
  (scaffold/contracts/secrets-mesh.md).
- **rollup** (round-7 relationship note) — secret values are addressable in
  raw and ID forms from rollup content; rollup must never resolve a secrets
  raw reference inside LLM-bound content (`llm_safe` fail-or-degrade
  applies; see `components/rollup.md`). Edge naming deferred until either
  side gets a design pass.
- **VDB / db / projects / environments / repo** (round-8 interface
  requirements) — anticipated interfaces per the Charter; repo's slice is
  linking secrets into GH Actions workflows (see `components/repo.md`);
  db's slice is the reconciliation note above. Edge naming deferred until
  `secrets` gets a design pass.
- Consumers (ccd's agents, org's agents, stack/db handlers reaching third
  parties, cicd deploys) are anticipated but not yet edges — deferred until
  `secrets` gets a design pass.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — locked: use-without-seeing for agents/LLMs, the
round-7 `llm_safe` raw/ID mechanism (no secret ever reaches an LLM), and
the round-8 name (`secrets`), distribution (across nodes / replication ~3),
push adapters (Supabase Vault, AWS Secrets Manager, GitHub Actions
secrets), and interface list (VDB, db, projects, environments, repo);
everything else open.
