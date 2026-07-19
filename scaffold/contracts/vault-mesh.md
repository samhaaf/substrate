# Contract: vault-mesh

## Parties
vault  <->  mesh

## What the edge carries
Vault's mesh participation through the **local mesh daemon on `:3649`**
(single-port locality): service registration/resolution (an instance of
`service-lookup`). The shape of secret brokerage through mesh — how a
consumer *uses* a secret without an agent/LLM ever *seeing* it — is TBD and
belongs to vault's design pass. Schema/example deferred.
**requirements-only** (round-6 lock, 2026-07-19).
