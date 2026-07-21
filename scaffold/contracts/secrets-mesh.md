# Contract: secrets-mesh

> **RENAMED (round-8, 2026-07-19): was `vault-mesh`.** The `vault` component is
> now `secrets` — "vault" collides with Supabase Vault, "SM" with AWS Secrets
> Manager (see the naming-history note in `components/secrets.md`). This file
> SUPERSEDES the requirements-only stub, authored from secrets.md concern 1/7/9.

## Parties

secrets (L3 app, `AddressingClass::Singleton`) ↔ mesh (L1/L2 daemon on `:3649`)

secrets authored this edge (secrets.md concern 1); `replicated-kv` blessed the
tenancy in advance (KV concern 8 names "any secrets-adjacent keyspace"). No
competing mesh-side proposal existed — the stub's "brokerage shape TBD" is
resolved here.

## Purpose

Two facets, both over the **local** mesh daemon (single-port locality, INTENT
#58 — secrets never opens a socket to a remote node):

1. **Registration / resolution** — an ordinary `service-lookup` instance:
   secrets registers `slug="secrets"` at its endpoint so consumers
   (rollup/repo/vdb/db/aws/vfs/cc/environments) resolve it, and secrets resolves
   `db`/`aws`/peers by slug.
2. **The mesh-brokered `secrets/` KV keyspace** — a keyspace-scoped surface mesh
   exposes ONLY to the registered `secrets` slug, relaying put/get/scan/watch of
   **opaque ciphertext blobs** against the `secrets/` keyspace of
   `replicated-kv`. secrets is an L3 app and CANNOT link the mesh-internal
   `KvHandle` (INTENT #29); this brokered facet is how it reaches replicated
   storage. The blobs are envelopes (ciphertext + wrapped DEK + nonce + bound
   AAD) — meaningless without a keychain-held key, so `Replication::All`
   (satisfying "~3 copies" without KV's reserved `Factor(n)`) is safe; the
   keyspace sets `mirror_values:false` (metadata-only pub/sub mirror) as
   defence-in-depth.

## Schema

Registration reuses `service-lookup` verbatim:

```rust
// mesh-side vocabulary (types::registry / types::node) — reused, not re-authored
struct Registration {
    slug: Slug,                       // "secrets"
    node: NodeId,
    endpoint: Endpoint,               // { scheme: Wss, host, port, health_path }
    addressing: AddressingClass,      // Singleton
    ttl_secs: u32,                    // lease; renewed by heartbeat
    meta: ServiceMeta,                // meta.requires: [db, aws?]
}
```

The brokered keyspace surface (secrets ↔ mesh, over `pubsub-protocol`/mesh WS;
`types::secrets::kv`):

```rust
enum SecretsKvMsg {
    Put   { path: String, blob: Bytes, if_version: Option<u32> }, // blob = opaque envelope; CAS on if_version
    Get   { path: String },                                       // -> Option<SecretsKvEntry>
    Scan  { prefix: String },                                     // -> Vec<SecretsKvEntry>
    Watch { prefix: String },                                     // -> stream<SecretsKvWatchEvent> (key+version ONLY)
    Delete{ path: String, if_version: Option<u32> },              // tombstone (KV owns tombstone semantics)
}

struct SecretsKvEntry {
    path: String,
    blob: Bytes,                      // opaque ciphertext envelope — mesh/KV never parse it
    kv_version: Version,              // types::kv::Version — KV's frozen LWW total order
}

struct SecretsKvWatchEvent {
    path: String,
    kv_version: Version,
    op: WatchOp,                      // NEVER carries the blob (mirror_values:false)
}
enum WatchOp { Put, Delete }
```

Keyspace path convention (owned by secrets, opaque to mesh):
`secrets/<scope-encoded>/<name>` — one entry per secret; the secret's own
`version` (concern 2) lives INSIDE the envelope, not in `kv_version`.

## Error cases

- `KeyspaceAccessDenied` — caller is not the registered `secrets` slug; mesh
  structurally refuses any other slug touching `secrets/*` (not a runtime policy
  check that could be forgotten).
- `Store(KvError)` — wraps replicated-kv's error, incl.
  `PartialReplication { written_local, peers_ack, peers_pending }` so a `Put`
  reports "written locally + on N peers, M pending" (a secret is durable locally
  the instant it is written; mesh-wide convergence is eventual).
- `VersionMismatch { expected, actual }` — CAS failure on `if_version`.
- Registration/resolution errors are `service-lookup`'s (`RegistryError`).
- Transport/relay failures are mesh-transport's (`PeerUnreachable`) — layered
  below this contract.

## Version sensitivity

- **Additive-safe:** new `SecretsKvMsg` variants (`#[serde(other)]` reserved);
  new `SecretsKvEntry`/`Registration` fields (`#[serde(default)]`). A newer
  secrets daemon against an older mesh degrades to the intersection surface.
- **Frozen (breaking if changed):** the **opacity of `blob`** — mesh and KV MUST
  never parse it, so the envelope format (concern 2) versions via a 1-byte format
  tag *inside* the blob and NEVER bumps the mesh/KV wire; secret-format churn is
  invisible here. KV's `Version` total order is frozen upstream.
  `mirror_values:false` on `secrets/` is a **security invariant**, not a tuning
  knob — flipping it leaks ciphertext blobs onto the dashboard firehose; it is
  part of the contract.
- **Breaking:** removing/renaming a `SecretsKvMsg` variant, or changing the
  `secrets/` keyspace access-control to admit a non-`secrets` slug.

## Reconciliation notes

- **One-sided authoring, pre-blessed tenancy.** Only secrets proposed content;
  mesh had no competing proposal (stub said "shape TBD"). Adopted secrets' design
  wholesale because `replicated-kv` had *anticipated* it: KV concern 8 prescribes
  metadata-only mirroring for "any secrets-adjacent keyspace," so `secrets/` is a
  pre-blessed KV tenant. The one genuinely new thing is that its *accessor* is an
  out-of-process L3 app, not an in-process mesh sibling.
- **Deviation flagged (carried, not resolved).** This lightly generalizes KV's
  "internal mesh-lib tenants only" boundary into a **mesh-brokered keyspace
  exposed to one external app**. v1 scopes the brokerage to `secrets` alone; the
  pattern is a candidate future generic `kv-api` (repo flagged the same wish for
  cross-node repo listing — repo.md concern 2). Surfaced for the operator + KV +
  mesh at harmonization.
- **Genesis-key circularity (resolved on the secrets side, noted here).** The
  kernel-state S3 snapshot key is NOT stored in this keyspace — it is
  `HKDF(MRK, "mesh-kv-snapshot")`, derived from the keychain-held MRK, so a dead
  mesh restores without a running secrets service (secrets.md concern 7). This
  keyspace therefore never holds the key needed to restore itself. See
  `aws-secrets` for the full resolution.
- **"~3 copies" resolution.** secrets uses `Replication::All`, not KV's reserved
  `Factor(n)` — every node holds ciphertext, comfortably exceeding "~3," and the
  undesigned factor knob is avoided (KV concern 10 friction sidestepped). "How
  many nodes can *decrypt*" is a separate knob (node enrollment / MRK
  provisioning), orthogonal to replication.

## Example data

`secrets` registers on **macbook** and writes the Global `github-pat` secret
(the credential repo uses to push the `demo` repo), which replicates to **pi**:

```jsonc
// secrets -> mesh: Registration (service-lookup facet)
{ "slug": "secrets", "node": "macbook",
  "endpoint": { "scheme": "Wss", "host": "macbook", "port": 3649, "health_path": "/secrets/health" },
  "addressing": "Singleton", "ttl_secs": 30,
  "meta": { "requires": ["db", "aws"] } }

// secrets -> mesh: Put the github-pat envelope into the secrets/ keyspace
{ "Put": {
    "path": "secrets/global/github-pat",
    "blob": "<opaque XChaCha20-Poly1305 envelope: ciphertext|wrap(MRK,DEK)|nonce|AAD>",
    "if_version": 2 } }

// mesh -> secrets: reply (durable on macbook; pi still catching up)
{ "Store": { "PartialReplication": { "written_local": true, "peers_ack": 0, "peers_pending": 1 } } }

// later — pi's secrets replica converges; a Watch event on macbook (metadata only):
{ "path": "secrets/global/github-pat",
  "kv_version": { "lww_ts": 1721370000123, "writer": "macbook" }, "op": "Put" }
```

The `github-pat` SecretRef this envelope backs —
`{ id: "a1b2c3d4-0000-4000-8000-000000000001", name: "github-pat",
scope: "Global", version: 3 }` — is the exact handle used in `repo-secrets`.
