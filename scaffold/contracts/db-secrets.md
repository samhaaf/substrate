# Contract: db-secrets

## Parties
`secrets` ↔ `db`. Two trust-flow directions over one edge, both **over
`db-control-plane` WS or the `db vault …` CLI, NEVER by linking `substrate-db`**
(INTENT #29). Authored from `secrets.md` (authoritative for the secrets
brokerage) and `db.md` concern 7 (authoritative for `vault.rs`), which agree.

## Purpose
- **(a) secrets → db — the Supabase Vault push adapter (REUSE, do NOT rebuild;
  INTENT #105).** `secrets` drives `db`'s existing, real `vault.rs` module as
  its **Supabase adapter**: push a secret's value into a Supabase project's
  `vault.secrets` (bound-param SQL against the `supabase_vault` extension) and
  render `reference_sql` so migrations/handlers reference the secret **by name
  at runtime, never storing plaintext**. `db`'s `vault.rs` becomes secrets'
  Supabase adapter — leverage, not a rebuild.
- **(b) db → secrets — credential reconciliation (direction; concern 7).**
  `db` reconciles its own credential/keychain reads (the `supabase-cloud`
  Management-API PAT, cloud connection strings) to be **`secrets`-mediated**.
  Because `db` is a **non-LLM caller**, raw resolution is permitted for it —
  giving one audited source of truth. This is the operator-authorized "one of
  the times we actually update the db crate" (INTENT #99); scoped as direction,
  it lands when both daemons exist.

## Schema
`SecretRef`/`SecureValue`/`CallerContext` are owned by `secrets`
(`types::secrets`); the Vault verbs map onto `db vault set/rm/exists`
(`types::db`). `SecureValue` = the plaintext crossing a **non-LLM, in-mesh
confidential** WS channel (mesh-relayed between two trusted services); it never
transits an LLM path.

```rust
// ── (a) secrets -> db : the Supabase push adapter (maps onto db vault set/rm/exists) ──
struct DbVaultPush   { env: String, name: String, value: SecureValue, description: Option<String> }
                     // -> DbVaultReceipt
struct DbVaultRemove { env: String, name: String }
struct DbVaultExists { env: String, name: String } // -> bool
struct DbVaultReceipt { name: String, outcome: SetOutcome }   // from vault.rs
enum   SetOutcome     { Created, Updated }
// reference_sql is PURE CODEGEN, no wire value:
//   reference_sql(name) -> "(SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name = '…')"

// ── (b) db -> secrets : db resolves its own credential into its connect path ──
//     (non-llm_safe caller: raw permitted; secrets.md concern 4 vocabulary)
struct DbCredentialResolve { r#ref: SecretRef, caller: CallerContext /* {slug:"db", llm_safe:false} */ }
                          // -> SecureValue  (the PAT / connection string)
```

## Error cases
- `SecretsError::AdapterFailed { adapter: Supabase, detail }` — **wraps** `db`'s
  typed `DbError::NotImplemented { command: "vault set", driver: "sqlite" }`
  (the `sqlite` driver has no Vault; the SQLite equivalent is secrets' **own**
  local-mesh-db adapter over `vdb-secrets`, NOT this edge) and any SQL/connection
  error stringified at the boundary (`db`'s error never becomes a secrets type
  — types.md guardrail).
- `SecretsError::NotFound { name }` on (b) resolution.
- `SecretsError::RawRefusedLlmSafe` is **unreachable** on (b): `db`'s
  `CallerContext.llm_safe` is `false` by construction.

## Version sensitivity
LOW. `vault.rs`'s surface is stable bound-param SQL; the adapter is a thin
WS/CLI shim. The (b) keychain→secrets cutover is an **additive** `db` change
(operator-authorized, INTENT #99). Both directions grow via `#[serde(default)]`
fields; `SetOutcome` reserves `#[serde(other)]`.

## Reconciliation notes
- **Both parties agree; the only difference is a field name.** `secrets.md`
  named the push value `value_ciphertext_channel: SecureValue`; `db.md` named
  it `value: SecureValue`. Adopted **`value: SecureValue`** (shorter; the
  `SecureValue` type already denotes the confidential-channel semantics — the
  longer name duplicated that in the identifier). No semantic divergence.
- **Direction (b) is `db.md`'s addition, accepted.** `secrets.md` proposed only
  the push adapter (a); `db.md` concern 7 adds the reverse credential
  reconciliation (b). Both are recorded here as the "same edge, both directions
  of trust flow" (db.md's phrasing). (b) is scoped as **direction**, not v1
  built behavior — flagged.
- **Boundary vs `vdb-secrets`:** this edge is the **Supabase** adapter only.
  The **local-mesh-database** secret facility (the SQLite equivalent of
  Supabase Vault) is secrets' own v1-BUILD adapter and lives on `vdb-secrets`,
  not here. The `sqlite`-driver `NotImplemented` above is the seam between them.

## Example data
World: nodes **macbook** and **pi**; project **demo**. The `demo` app has a
Supabase cloud environment; its service-role key must reach the Supabase
project's Vault so edge functions can reference it.

**1. secrets pushes a secret into Supabase Vault via db's adapter:**

```jsonc
// DbVaultPush   secrets -> db   (db-control-plane WS; db drives vault.rs)
{ "env": "demo@supabase",
  "name": "SERVICE_ROLE_KEY",
  "value": "<SecureValue: plaintext over the confidential in-mesh channel>",
  "description": "demo service role, rotated 2026-07-19" }
// DbVaultReceipt   db -> secrets
{ "name": "SERVICE_ROLE_KEY", "outcome": "Created" }
// a demo migration now references it WITHOUT plaintext:
//   reference_sql("SERVICE_ROLE_KEY")
//   = "(SELECT decrypted_secret FROM vault.decrypted_secrets WHERE name = 'SERVICE_ROLE_KEY')"
```

**2. The sqlite driver has no Vault — clean degrade (seam to vdb-secrets):**

```jsonc
// DbVaultPush { env: "demo/main" (a LOCAL sqlite stack db), name: "SERVICE_ROLE_KEY", … }
// -> DbError::NotImplemented { command: "vault set", driver: "sqlite", reason: "no Vault extension" }
// secrets wraps -> SecretsError::AdapterFailed { adapter: Supabase, detail: "sqlite has no Vault" }
// secrets instead routes this push to its local-mesh-db adapter over vdb-secrets.
```

**3. db reconciles its own PAT read through secrets (direction b):**

```jsonc
// DbCredentialResolve   db -> secrets
{ "ref": { "id": "…", "name": "supabase_mgmt_pat", "scope": { "Global": {} }, "version": 3 },
  "caller": { "service_slug": "db", "llm_safe": false, "purpose": "edge deploy" } }
// -> SecureValue  (the PAT; db uses it to call the Supabase Management API, never logs it)
```
