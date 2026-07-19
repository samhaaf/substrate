# Contract: vdb-secrets

## Parties
`secrets` ↔ `vdb`. Over the local mesh daemon `:3649`, never linked (INTENT
#29). Authored from `secrets.md` (its `vdb-secrets` half) and `vdb.md`
concern 7 / its `vdb-secrets` refinement — which **disagree on one point**
(see Reconciliation notes), resolved in vdb's favor.

## Purpose
- **(a) The local-mesh-database push adapter — the v1-BUILD adapter.** Push a
  secret into a `vdb`/stack-hosted local SQLite database's **secret facility**
  (the SQLite equivalent of Supabase Vault: an ops-schema `_vdb_secrets` table
  with an encrypted column + a decrypt-at-read mechanism whose keys stay rooted
  in `secrets`). `vdb` provisions the facility; `secrets` pushes into it. This
  is the adapter secrets actually builds in v1 (the Supabase one is reused from
  `db`; the AWS SM one is design-only).
- **(b) Cloud-target credentials, use-without-seeing.** Hand `vdb` the
  connection string / password for its Supabase or AWS RDS promotion/serving
  targets. `vdb` is a **trusted, non-LLM service** performing the privileged
  connect itself; **handlers never see it** — a Deno handler's context exposes
  only a `SecretRef` + the `use(ref, sink)` verb, never a raw secret.

## Schema
`SecretRef`/`SecureValue`/`SecretScope` are `secrets`' (`types::secrets`);
`CloudTarget` is shared (reconciled to vdb's `target_id` naming). The
`_vdb_secrets` facility schema is part of the stack ops schema and versions
with the stack-definition ledger.

```rust
// ── (a) secrets -> vdb : the local-mesh-db push adapter (v1 BUILD) ──────
struct VdbSecretPush   { project: String, db: String, name: String, value: SecureValue }
                       // -> VdbSecretPushReceipt
struct VdbSecretPushReceipt { name: String, outcome: SetOutcome }   // Created | Updated
enum   SetOutcome { Created, Updated }

// ── (b) vdb -> secrets : resolve a cloud credential into vdb's OWN connect path ──
//     RESOLVED IN VDB'S FAVOR: returns the value, not a brokered connection (see notes)
struct VdbCredentialResolve { r#ref: SecretRef, target: CloudTarget }
                          // -> SecureValue   (conn string / password; vdb connects, never persists it)
enum CloudTarget {
    Supabase { project_ref: String },
    AwsRds   { target_id: String },     // reconciled from secrets.md's `arn` — see notes
    #[serde(other)] Unknown,
}
```

`vdb` holds the resolved `SecureValue` **in memory only**, never persists it,
never places it in a handler payload or context — the same posture aws.md's
`CredentialProvider` takes.

## Error cases
- `SecretsError::AdapterFailed { adapter: LocalMeshDb, detail }` — (a) push
  failed (facility not provisioned, SQLite error).
- `SecretsError::NotFound { name }`.
- `SecretsError::NotDecryptable` — the target database is hosted on a
  ciphertext-only replica node (unenrolled: holds ciphertext but no master
  key). Surfaced, not worked around: such a node cannot serve
  cloud-credentialed work — `vdb` reports it rather than silently failing open.
- `SecretsError::RawRefusedLlmSafe` — would fire only if a `vdb` caller were
  **mis-flagged** `llm_safe`. `vdb` the service is not llm_safe; its handlers'
  rollup-assembled payloads inherit queues' `llm_safe` pass-through instead
  (so a raw secret can never enter a handler's LLM-bound content).

## Version sensitivity
MEDIUM. `CloudTarget` grows with `vdb`'s adapter matrix (Supabase/RDS now, more
later) — additive, `#[serde(other)]` reserved. The `_vdb_secrets` facility
schema versions with the stack definition. The **use-without-seeing invariant
is frozen**: no verb ever returns a raw secret into an LLM-reachable path.

## Reconciliation notes
- **THE disagreement — brokered `ConnectionHandle` (secrets) vs resolved
  `SecureValue` (vdb). Resolved IN VDB'S FAVOR.** `secrets.md` proposed
  `VdbCredentialUse { ref, target } -> ConnectionHandle` ("secrets connects; vdb
  never sees raw") — i.e. secrets opens the DB connection and brokers a handle
  to it. `vdb.md` refined this to `VdbCredentialResolve { ref, target } ->
  SecureValue`, on two well-argued grounds: **(1)** a live DB connection
  (TCP+TLS session, driver-specific state) **cannot practically be brokered
  across two processes** — a handle to secrets' connection is not usable by
  `vdb`'s driver; **(2)** `vdb` is a **trusted, non-LLM service performing the
  privileged action itself**, which is exactly secrets' own
  *resolve-into-sink* category (secrets.md concern 4) — the same posture aws's
  `CredentialProvider` already takes. The invariant that actually matters is
  preserved either way: **no secret in any LLM context** (handlers get only
  `SecretRef` + `use`). **Losing position recorded (not dropped):** secrets'
  stricter proxied-connect is a legitimate future hardening; if the operator
  prefers it, it is a **secrets-side, additive verb** `vdb` can adopt later
  **without contract breakage** — so nothing is foreclosed.
- **`CloudTarget` field-name reconciliation:** `secrets.md` used
  `AwsRds { arn: String }`; `vdb.md` uses `AwsRds { target_id: String }`
  (consistent with `types::vdb::StackTargetKind::AwsRdsLambda { target_id }`).
  Adopted **`target_id`** for cross-file consistency; the ARN, when needed, is
  what `target_id` resolves to inside the `aws` crate.
- **Boundary vs `db-secrets`:** the **Supabase** secret path is `db`'s Vault
  adapter (`db-secrets`); the **local SQLite** facility is this edge's (a). The
  two adapters are disjoint — the `sqlite`-driver `NotImplemented` on
  `db-secrets` is the seam that routes local pushes here.

## Example data
World: nodes **macbook** and **pi**; project **demo**; database **demo/main**
(local SQLite stack DB, anchored on macbook). Later, `demo/analytics` promotes
to a Supabase cloud target and needs its connection password.

**1. (a) Push a secret into demo/main's local vault facility:**

```jsonc
// VdbSecretPush   secrets -> vdb   (local-mesh-db adapter)
{ "project": "demo", "db": "main", "name": "STRIPE_WEBHOOK_SECRET",
  "value": "<SecureValue: plaintext over the confidential in-mesh channel>" }
// VdbSecretPushReceipt   vdb -> secrets
{ "name": "STRIPE_WEBHOOK_SECRET", "outcome": "Created" }
// vdb stores it in _vdb_secrets encrypted; a demo/main SQL handler reads it via
// the decrypt-at-read view; a Deno handler wields it via secrets.use(ref, sink).
```

**2. (b) vdb resolves a cloud credential to connect during promotion:**

```jsonc
// VdbCredentialResolve   vdb -> secrets
{ "ref": { "id": "…", "name": "supabase_demo_analytics_pw",
           "scope": { "Database": { "project": "demo", "db": "analytics" } }, "version": 1 },
  "target": { "Supabase": { "project_ref": "abcdefghij" } } }
// -> SecureValue  (the connection password)
// vdb opens the RDS/Supabase connection itself, holds the value in memory only,
// never writes it to _vdb_secrets, a handler payload, or a log.
```

**3. Ciphertext-only replica cannot serve cloud-credentialed work:**

```jsonc
// pi holds a ciphertext replica of the secrets keyspace but is unenrolled
// -> SecretsError::NotDecryptable
// vdb surfaces "demo/analytics cannot promote from pi: node not enrolled" — honest, not silent.
```
