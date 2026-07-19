# Contract: secrets-mesh

> **RENAMED (round-8, 2026-07-19): was `vault-mesh`.** The `vault` component
> is now `secrets` — "vault" collides with Supabase Vault, "SM" with AWS
> Secrets Manager; see the naming-history note in `components/secrets.md`.

## Parties
secrets  <->  mesh

## What the edge carries
Secrets' mesh participation through the **local mesh daemon on `:3649`**
(single-port locality): service registration/resolution (an instance of
`service-lookup`). The shape of secret brokerage through mesh — how a
consumer *uses* a secret without an agent/LLM ever *seeing* it — is TBD and
belongs to secrets' design pass. Round-8 adds the distribution requirement
(secrets replicated across nodes / factor ~3 — the replication plane rides
mesh like everything else; shape TBD). Schema/example deferred.
**requirements-only** (round-6 lock; renamed + extended round-8,
2026-07-19).
