# Contract: aws-secrets

## Parties
`secrets` (L3) `<->` `aws` (L2), over the local mesh daemon `:3649`.

*(Authored at wave-2 harmonization. Named in wave2-plan §3b but the contract
round produced no file — a coverage gap. Both component halves exist and are
compatible; this stub merges them. Full sketches: `components/aws.md`
§`aws-secrets` and `components/secrets.md` §`aws-secrets`.)*

## Purpose
One edge, two directions, four facets:

1. **secrets → aws: the AWS Secrets Manager PUSH adapter — DESIGN-ONLY v1**
   (INTENT #105). One of secrets' three push adapters (alongside Supabase
   Vault and GitHub Actions). `aws` is a **transient conduit** to the
   destination store (SM re-encrypts under KMS); it never persists the value.
2. **aws → secrets: aws sources its own AWS account credentials.** aws's
   CredentialProvider fetches the account credential (role ARN / access-key)
   via secrets' general brokerage shape. **Load-bearing even in the stub** —
   aws needs it the moment ANY adapter goes Live.
3. **aws/vfs → secrets: the S3 client-side-encryption runtime key** for the
   overflow tier (use-without-seeing; `aws` is a trusted non-LLM caller →
   raw allowed; distinct from the vfs *data* key).
4. **Genesis kernel-state snapshot key resolution — NOT a secrets-service
   call.** `HKDF(MRK, "mesh-kv-snapshot")` derived locally from the
   operator's keychain/offline MRK, so a dead mesh restores with no running
   secrets service. Documented here as the resolution of KV's aws-mesh
   bootstrap circularity; no message crosses this edge for it.

## Schema (summary — authoritative sketches in the component files)
Facet 1: `AwsSecretsRequest { PutSecret | RotateSecret | DeleteSecret }`
(secrets.md's `AwsSmPush` is the same verb set, one struct — merge at types
time). Facet 2: `FetchAwsCreds { role_ref: SecretRef } -> AwsCreds`. Facet 3:
`S3CseKeyGet { scope: SecretScope } -> SecureValue`. `SecretRef` /
`SecureValue` / `SecretBytes` are `secrets`' vocabulary, reused not redefined.

## Error cases
Facet 1: `AwsDisabled { adapter:"secretsmgr", op }` (v1 default — designed,
not built); `PushFailed`; `AdapterFailed{AwsSecretsManager,…}`. Facet 2:
`SecretNotFound { role_ref }`; `SecretsUnavailable` (whole service
unreachable → all aws ops degrade to `CredentialsUnavailable`). Facet 3:
`KeychainUnavailable` / `NotDecryptable` on an unenrolled node.

## Version sensitivity
Facet 1 design-only (placeholder wire, pinned when built). Facet 2/3 pinned
early — load-bearing at first Live adapter. **The facet-4 genesis-key rule is
FROZEN**: changing the KDF label or source is a data-corrupting change (old
snapshots become unrecoverable).

## Reconciliation notes
- `aws.md` proposed facets 1–2; `secrets.md` proposed facets 1, 3, 4. No
  disagreement — the union is adopted. The one overlap (SM push) differs only
  in struct spelling (`AwsSecretsRequest` enum vs `AwsSmPush` struct);
  resolved to the enum form (extensible per aws.md's adapter conventions),
  content identical.
- The invariant both halves state: no secret ever reaches an LLM context;
  `aws` and `vfs` are trusted non-LLM callers in secrets' resolve-into-sink
  category.
