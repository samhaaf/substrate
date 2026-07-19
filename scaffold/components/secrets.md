# secrets

**Status:** NEW (round-6 lock; **RENAMED `vault` → `secrets` round-8**);
**FULL DESIGN — wave 2, batch 3** (this pass supersedes the prior
requirements-only stub). **Nesting:** top-level L3 app-crate (`lib/secrets`
crate `substrate-secrets` + `bin/secrets` daemon/CLI). **Consumes (over the
wire, never linked — INTENT #29):** mesh via `secrets-mesh` (registration +
the mesh-brokered `secrets/` replicated keyspace), the OS keychain as
root-of-trust, `db` via `db-secrets` (the Supabase push adapter), `aws` via
`aws-secrets` (SM push + S3 CSE key handoff, design-only). **Consumed by:**
`rollup`, `repo`, `vdb`/`db` handlers, `ccd`/`org` agents, `environments`,
`aws`/`vfs` (S3 encryption keys), `openrouter-mgmt`. Grounded in
replicated-kv.md (opaque-bytes replication, `mirror_values` redaction, the
delegated aws-mesh genesis-key circularity), db.md + `lib/db/src/vault.rs`
(the real Supabase-Vault module = the Supabase adapter), aws.md, repo.md,
rollup.md, types.md (guardrail 4, the reserved `error/secrets.rs`).

> **NAMING HISTORY (round-8, LOCKED):** born `vault` (round-6); "vault"
> REJECTED (collides with **Supabase Vault**, a push target), "SM" rejected
> (collides with **AWS Secrets Manager**), safe/locker/shelf rejected.
> Operator: "secrets is the name because that's what it is." The `vault-mesh`
> contract is `contracts/secrets-mesh.md`. Verbatim operator quotes from
> earlier rounds keep the word "vault" as spoken.

## Charter

`secrets` is the OS's single owner of secret material: it holds every
credential, token, connection string, API key, and encryption key that any
Mind OS service needs, encrypts them at rest under a keychain-rooted key
hierarchy, replicates the *ciphertext* across the mesh (≥3 copies), and — the
one defining, LOCKED requirement — makes secrets **usable without being
seeable**: "By default, agents can never directly access a secret, but they
can use it in a context." An agent/LLM can cause a secret to be *injected*
into a sink (a request header, an env var, a connection string, a deploy, a
GitHub-Actions secret) but can **never read the plaintext into its own
completion context** — enforced structurally, because there is no raw-read
surface reachable from an LLM path, only opaque **references** and
**use-here** verbs. It owns: the encrypted store and its key hierarchy; the
raw-vs-reference addressing model and the `llm_safe` fail-or-degrade policy;
per-project / per-database / per-environment scoping; rotation and node
enrollment; and the **push adapters** that mirror a secret out into an
external secret store (Supabase Vault, local mesh DB, GitHub Actions, AWS
Secrets Manager). Its boundary — what it does NOT own: the **replication
mechanism** (that is `replicated-kv` inside mesh; secrets is a *tenant* of one
mesh-brokered keyspace, not a replicator); **the secret↔workflow *linkage***
(that is `repo`'s — secrets only pushes values when repo names the link);
**which environment/database a deploy targets** (that is `environments`/`vdb`
— secrets pushes where told); **its own daemon crypto being novel** (it uses
boring, audited AEAD envelope encryption — no home-grown ciphers); and the
external stores themselves (Supabase/GitHub/AWS hold their own copies; secrets
is the source of truth and one-way mirrors outward). It must be "boring and
not choppy."

## Primary design concerns

Secrets earned a top-level component (not a mesh utility, not a fold into
`db`) because it composes three genuinely separable hard surfaces — an
at-rest key hierarchy, a structural use-without-seeing access model, and a
fan of heterogeneous push adapters — behind one boring CLI/daemon, and because
it is the one place in the OS where a design mistake is a *disclosure*, not a
crash. The concerns below are ordered by how load-bearing the correctness is.

### 1. The storage model — ciphertext in mesh KV, keys in the keychain (the honest replication answer)

The operator wants secrets "distributed across nodes / replication ~3." The
one boring replicated primitive is `replicated-kv` (INTENT #54), which
replicates **opaque value bytes to every node** (`Replication::All` in v1;
`Factor(n)` is RESERVED/undesigned). replicated-kv's designer deliberately
excluded value-visible replication for sensitive payloads at the
*observability* layer (per-keyspace `mirror_values: false` → the pub/sub
mirror carries key+version metadata only, never the value) but the **core
anti-entropy still ships the value bytes to every daemon's SQLite**. Storing a
plaintext secret there would leak it to every node's disk.

**The resolution is to make the replicated bytes non-sensitive by
construction: KV stores only ciphertext.** Secrets encrypts every value
locally (concern 2) and writes the *envelope* — ciphertext + wrapped
data-key + nonce + bound metadata — as the opaque KV value. Because the bytes
are meaningless without a key that lives only in each node's OS keychain
(never in KV, never on the wire in plaintext), replicating them to every node
is safe and *desirable*: `Replication::All` gives every node a copy, which
comfortably satisfies "~3" without needing KV's reserved `Factor(n)`. This is
the honest reading of KV's model — I do **not** ask KV to hide values; I make
the value itself a ciphertext. The `secrets/` keyspace still sets
`mirror_values: false` (metadata-only pub/sub mirror) as defence-in-depth so
the dashboard firehose never even sees ciphertext blobs.

**Layering (the load-bearing wrinkle).** `secrets` is an **L3 app**;
`replicated-kv` is a mesh-internal L2 lib with "no external wire surface of
its own" whose tenants are mesh-internal libs (registry/locks/queues/
supervision/cron). An L3 app **cannot link `KvHandle`** (INTENT #29). So
secrets reaches replicated storage through a **mesh-brokered, keyspace-scoped
KV surface** exposed over the `secrets-mesh` contract: mesh owns a thin facet
that opens the `secrets/` keyspace on replicated-kv and relays
put/get/scan/watch of opaque blobs to *only* the registered `secrets` service.
KV's designer already anticipated this — concern 8 explicitly names "any
secrets-adjacent keyspace" and prescribes metadata-only mirroring for it — so
`secrets/` is a blessed KV tenant; the only genuinely new thing is that its
*accessor* is an out-of-process L3 app rather than an in-process sibling. That
is the "brokerage shape TBD" the `secrets-mesh` stub flagged, resolved here
(and surfaced as friction: it lightly generalizes KV's "internal tenants only"
boundary into a mesh-brokered keyspace — the pattern could later become a
generic `kv-api`, but v1 scopes it to secrets alone).

Each node also keeps a small **local SQLite** (`secrets.db`, in mesh's data
dir on the plain OS filesystem — NOT in VFS, same reasoning as KV concern 9)
as a read-through cache + the home for adapter push-state and audit; the
source of truth for a value is the `secrets/` KV keyspace.

### 2. The key hierarchy — envelope encryption rooted in the OS keychain

Encrypted at rest with a three-tier envelope, so no single compromise
discloses everything and rotation is cheap:

```
OS keychain (per node, ROOT OF TRUST)
  └─ NK   Node Key            — X25519/AEAD key, generated on first boot,
  │                             stored ONLY in this node's keychain, never leaves it
  └─ wrap(NK, MRK)  ──────────┐
                              │
MRK  Mesh Root Key  ──────────┘  — one mesh-wide symmetric master; on each
  │                               enrolled node it exists only as wrap(NK,MRK)
  │                               in that node's keychain (decrypted into RAM on use)
  └─ wrap(MRK, DEK_i)             — per-secret Data Encryption Key, random per secret
        └─ AEAD(DEK_i, plaintext, AAD = secret metadata)   — the ciphertext in KV
```

- **Crypto is boring and audited.** AEAD = **XChaCha20-Poly1305** (RustCrypto
  `chacha20poly1305`) for value encryption and for every wrap step;
  random 192-bit nonces; the secret's canonical metadata (id, name, scope,
  version) is the **AAD**, so a ciphertext cannot be silently relabelled or
  moved to another secret's slot. `age` (X25519 + ChaCha20-Poly1305) is the
  fallback library if a higher-level envelope format is preferred — decision
  left to the Filler, both are acceptable; **no home-grown cipher, ever.**
- **Root of trust = OS keychain** (macOS Keychain / Linux Secret Service /
  Windows Credential Manager via the `keyring` crate), matching `db`'s
  documented "secrets come from the OS keychain at runtime" convention
  (`etc/db.example.toml`, `driver/supabase_cloud.rs` — today aspirational, not
  yet code; secrets makes it real). The keychain holds NK and wrap(NK, MRK)
  only — never a DEK, never a plaintext value.
- **Headless-node fallback (the Pi).** A headless Linux node may have no
  Secret Service daemon. Fallback: NK in a `0600` key file under the mesh data
  dir, itself wrapped by a passphrase-derived key (Argon2id) supplied at
  enrollment. Flagged as a real degradation of the root-of-trust guarantee for
  headless nodes — operator's call whether such a node is allowed to hold the
  MRK at all (a cold-storage Pi may deliberately be enrolled for *ciphertext
  replication only*, without the MRK — it stores blobs it cannot read).
- **Node enrollment** is the deliberate act of provisioning the MRK to a new
  node: an already-enrolled node encrypts MRK to the joiner's NK public key and
  hands it over (`secrets enroll <node>`), gated by an operator confirmation.
  A never-enrolled node participates in `secrets/` replication (holds
  ciphertext) but cannot decrypt — it is a replica, not a reader. This is the
  clean split that makes "~3 copies" and "few nodes can actually read"
  independent knobs.
- **Rotation** is two independent operations: **rotate a secret** (mint a new
  DEK, re-encrypt the value, bump `version` — old ciphertext tombstoned via
  KV) and **rotate the MRK** (mint MRK′, rewrap every DEK under MRK′ — cheap,
  DEKs and ciphertexts are untouched — then re-enroll nodes with wrap(NK,MRK′)).
  Both emit `secrets.rotated` events (metadata only).

### 3. Use-without-seeing — structural, not policy

The distinction is enforced by the *shape of the surfaces*, not by a runtime
check that could be forgotten. Two address forms and exactly one plaintext
egress path:

- **Reference form — `SecretRef { id, name, scope, version }`.** Freely
  passable, safe to place in any LLM context, a prompt, a rollup fragment, a
  log. This is the ONLY form an agent/LLM path ever receives. It is a handle,
  not a value.
- **Use surfaces (resolve-into-sink).** The plaintext flows from the secrets
  store *directly into a sink* by non-LLM code — never through the caller's
  context:
  - **injected into a spawned process's env** (ccd/org agent processes, deploy
    steps) — done by the supervising harness, not returned to the completion;
  - **composed into a DB connection string / auth header inside a handler**
    (vdb/db handlers reaching third parties) — the handler receives a
    `SecretRef` and calls a `use(ref, sink)` verb that performs the
    authenticated action, or the adapter renders `db`'s `reference_sql`
    expression so the plaintext lives only in the database's own decrypted
    view (see `lib/db/src/vault.rs::reference_sql`);
  - **pushed into an external secret store** (concern 5).
- **The one raw egress: `reveal`.** `secrets reveal <name>` prints plaintext —
  the single human/trusted-tool path, mirrored on `db vault get --reveal`.
  Local-CLI only, never exposed over the agent surface, always audited. There
  is **deliberately no "read raw" verb on the WS/agent surface.**

**The honest limit, stated plainly.** For an agent that must itself wield a
credential (e.g. it shells `curl` with a token), env injection puts the value
in the agent's own process where `printenv` could read it — so
use-without-seeing is *perfect* only for handler/service/deploy sinks, where
non-LLM code performs the privileged action. The invariant we actually
guarantee is INTENT #94's exact one: **no secret ever reaches an LLM
completion's context/transcript**, because every resolve-into-sink is executed
by code paths outside the completion, and the LLM only ever handles
`SecretRef`s. Where an agent genuinely needs to act with a credential, the
preferred pattern is a **proxied action** (secrets/a handler performs the
authenticated call on the agent's behalf); direct injection is the degraded
fallback and is scoped + short-lived + warned.

### 4. `llm_safe` — the raw-vs-ID fail-or-degrade policy (LOCKED round-7)

Every resolve request carries a **caller context**:

```rust
pub struct CallerContext {
    pub service_slug: String,   // resolved from the mesh registration of the caller
    pub llm_safe: bool,         // TRUE by default for any service known to feed an LLM
    pub purpose: Option<String>,// audited
}
```

Services known to feed an LLM (`rollup`, `ccd`, `inference` prompt assembly,
`org` agents) default `llm_safe = true`. Under `llm_safe`, a request for a
secret's **raw** form does one of two configured things (per-deployment
default = degrade):

- **fail** — `SecretsError::RawRefusedLlmSafe { secret }`; or
- **degrade to reference** — return the `SecretRef` plus a
  warning payload: *"a raw secret was requested in an llm_safe context; an ID
  reference was injected instead."* (the operator's exact framing).

**The invariant, confirmed: no secret ever reaches an LLM.** A non-`llm_safe`
caller (a deploy step, a handler, an adapter) may resolve raw normally. The
`llm_safe` flag is set from the *caller's mesh identity*, not self-asserted by
the request, so an LLM-feeding service cannot spoof its way to raw.

### 5. Push adapters — one-way mirrors into external secret stores

An **adapter** takes a secret from the internal store (source of truth) and
pushes its value into an *external* system's own secret store. This is
distinct from concern 3's in-mesh use surfaces. One trait, four
implementations at v1-differentiated maturity:

```rust
#[async_trait]
pub trait SecretsAdapter {
    fn kind(&self) -> AdapterKind;                 // Supabase | LocalMeshDb | GitHubActions | AwsSecretsManager
    async fn push(&self, secret: &ResolvedSecret, target: &AdapterTarget) -> Result<PushReceipt, SecretsError>;
    async fn exists(&self, name: &str, target: &AdapterTarget) -> Result<bool, SecretsError>;
    async fn remove(&self, name: &str, target: &AdapterTarget) -> Result<(), SecretsError>;
}
```

- **Supabase Vault adapter — EXISTS, do NOT rebuild (INTENT #105).** `db`'s
  working `lib/db/src/vault.rs` (`set`/`list`/`get`/`remove`/`exists`/
  `reference_sql`) **IS** the Supabase adapter. secrets drives it **over the
  wire** — `db vault set` CLI / a `db-control-plane` WS call — **not by linking
  `substrate-db`** (INTENT #29: `db` is an app, called over WS/CLI). Contract:
  `db-secrets`. The push writes the value into that Supabase project's
  `vault.secrets`; handlers then reference it via `reference_sql` so plaintext
  never lands in a migration/handler file.
- **Local-mesh-database adapter — the one v1 BUILDS.** Pushes a secret into a
  `vdb`/stack-hosted local SQLite database's own secret table (SQLite has no
  Supabase Vault, so this adapter provides the equivalent: an encrypted
  column + a decrypt-at-read view, keys still rooted in secrets). Also serves
  the reverse need: handing `vdb` connection strings/credentials for cloud
  targets, use-without-seeing. Contract: `vdb-secrets`.
- **GitHub Actions adapter — BUILD (repo's required v1 capability).** Pushes a
  value into a repo's GH Actions secrets: fetch the repo's Actions public key,
  seal the value with libsodium sealed-box, `PUT
  /repos/{owner}/{repo}/actions/secrets/{NAME}`. secrets owns the *push
  mechanics*; `repo` owns the secret↔workflow *linkage* and triggers the push
  on `git push` (injection-on-push, THE v1 capability). Contract:
  `repo-secrets`.
- **AWS Secrets Manager adapter — DESIGN-ONLY, lives in `aws` (INTENT #105).**
  The data contract is designed here; the surface lives in the `aws` crate's
  WS interface; **not built in v1**. Contract: `aws-secrets`.

Adapters are **one-directional** (secrets → external). secrets never treats an
external store as an authority; drift (an external value changed out-of-band)
is detected by `exists`/hash-compare and re-pushed, never pulled back.

### 6. Scoping / namespacing — provenance-first (INTENT #85/#92)

A secret is addressed by **name within a scope**, and scope is first-order
because provenance is:

```rust
pub enum SecretScope {
    Global,                                  // mesh-wide (e.g. the S3 CSE key)
    Project { project: String },
    Database { project: String, db: String },// the VDB/db provenance grain (INTENT #92)
    Environment { project: String, env: String },
}
```

KV path: `secrets/<scope-encoded>/<name>` → one KV entry per secret,
version-carried by the envelope. Resolution is **most-specific-wins** with an
explicit fallback chain (Environment → Database → Project → Global), so an
environment can override a project default without duplicating unchanged
secrets. Every resolve/use/push writes a provenance record (INTENT #85: "every
time data gets touched") — `Provenance` from `types`, causation/correlation
carried — to the local audit and the metadata mirror; the audit itself never
contains plaintext.

### 7. The aws-mesh genesis-key circularity — resolved here (KV delegated it)

replicated-kv's `aws-mesh` design flagged an UNRESOLVED circularity and handed
it to this batch: kernel-state S3 snapshots must be client-side encrypted; the
key is "supposed to live in secrets"; but secrets rides mesh replication, so
the key needed to *restore a dead mesh* cannot live inside the thing being
restored. **Resolution:** the kernel-state snapshot key is **not** a
KV-stored DEK — it is derived directly from the **MRK** (which lives in the OS
keychain and is operator-recoverable *outside* the mesh, exactly because it
must survive total mesh loss). Concretely: `snapshot_key = HKDF(MRK,
"mesh-kv-snapshot")`. Restoring a genesis mesh therefore needs only the
operator's keychain-held (or offline-recovery-exported) MRK — no running
secrets service, no circular dependency. Runtime S3 encryption keys for the
*VFS overflow tier* (not kernel state — INTENT #48/#98) are ordinary
Global-scoped secrets handed to `aws`/`vfs` over `aws-secrets` /`aws-vfs`;
those DO ride the store because losing them only costs cold-tier access, not
mesh genesis. So: **two key classes** — genesis (MRK-derived, offline-rooted)
and runtime-data (normal secrets). Surfaced as a friction point for the
operator + KV + aws to bless.

### 8. The rollup invariant — no secret resolvable in LLM-bound rollup output (both sides)

Cross-crate invariant, stated on both sides so neither can drift:

- **rollup side (`components/rollup.md`):** rollup supports raw-vs-reference
  inserts; when a fragment references a secret, rollup calls secrets with the
  caller's `llm_safe` flag. For LLM-bound content that flag is `true`, so a
  **raw** secret insert is refused/degraded — rollup emits the ID reference +
  the warning, never the plaintext, into the assembled prompt.
- **secrets side (this file, concern 4):** the resolve API physically has no
  raw egress under `llm_safe`; the degrade-to-reference happens inside secrets,
  so even a buggy rollup that *asks* for raw cannot receive it in an LLM
  context.

The invariant holds even if one side is wrong, because the refusal is enforced
where the plaintext lives. Contract: `rollup-secrets`.

### 9. Boring surface — CLI + daemon + surface schema

`bin/secrets` is a noun-verb `clap` CLI over `lib/secrets`, and a daemon
registering with the local mesh (single-port locality, `mesh-client`):

```
secrets set <name>  [--project P|--db P/D|--env P/E] [--from-stdin]   # create/update (value never in argv)
secrets ref <name>  [--scope …]        # print a SecretRef (safe to paste anywhere)
secrets ls          [--scope …]        # metadata only (name, scope, version, updated_at, adapters)
secrets reveal <name>                  # the ONE plaintext path — local, audited, off the agent surface
secrets rm <name>
secrets rotate <name> | secrets rotate --mrk
secrets enroll <node> | secrets nodes  # key-hierarchy / enrollment ops
secrets push <name> --adapter supabase|vdb|github|aws --target <…>
secrets link <name> --workflow <owner/repo/workflow>   # delegates linkage to repo
```

The daemon publishes a `SurfaceSchema` (`types::surface`) rendering
metadata-only panels (never a value field; the `reveal` action carries
`confirm: true` and is dashboard-hidden), so the boring dashboard renders it
uniformly and agents can drive it — but the schema exposes no value-egress
action. Participates in `restart-protocol` and `pubsub-protocol` like every
service; `SecretsError` lands in `types::error::secrets` (the reserved slot).

## Relationships / edges

- **mesh** via `secrets-mesh` — registration/resolution (a `service-lookup`
  instance) **plus** the mesh-brokered `secrets/` replicated KV keyspace
  (opaque ciphertext, `Replication::All`, `mirror_values:false`) that is
  secrets' distributed store (concern 1). *(scaffold/contracts/secrets-mesh.md
  — exists, requirements-only; this design proposes its content below.)*
- **db** via `db-secrets` — secrets drives `db`'s existing Supabase-Vault
  module (`vault.rs`) as the Supabase push adapter, over WS/CLI, never linked
  (INTENT #29/#105). *(MISSING — proposed below.)*
- **vdb**/`stack` via `vdb-secrets` — the local-mesh-database push adapter
  (BUILD) + connection-string/credential use-without-seeing for cloud targets.
  *(MISSING — proposed below.)*
- **repo** via `repo-secrets` — repo names the secret↔workflow linkage; secrets'
  GitHub-Actions adapter pushes values on `git push` (injection-on-push, the
  v1 capability). *(MISSING — proposed below.)*
- **rollup** via `rollup-secrets` — raw/ID addressing from rollup content;
  `llm_safe` fail-or-degrade governs; the no-secret-in-LLM-output invariant
  (concern 8). *(MISSING — proposed below.)*
- **aws** via `aws-secrets` — AWS Secrets Manager push adapter (DESIGN-ONLY,
  lives in `aws`) **and** the S3 CSE runtime-key handoff + the MRK-derived
  genesis snapshot-key resolution (concern 7). *(MISSING — proposed below.)*
- **environments** via `secrets-environments` — pushing secrets into a
  specific environment (anticipated; environments is a L6 stub). *(stub-track
  anticipated — named below, content deferred.)*
- **openrouter-mgmt** via `openrouter-secrets` — API-key storage/rotation for
  per-key OpenRouter keys (anticipated; L6 stub). *(stub-track anticipated —
  named below, content deferred.)*
- **OS keychain** — root of trust for NK/MRK; **not a substrate contract edge**
  (a platform capability, like the local filesystem).
- **replicated-kv** — NOT a direct edge: secrets is L3, cannot link the
  mesh-internal `KvHandle`; it reaches the `secrets/` keyspace only through
  `secrets-mesh`. The tenancy is blessed by KV concern 8 (the anticipated
  "secrets-adjacent keyspace").

## Nesting

Parent: none | Children: none. Dual-role crate like `db`/`gc`: `lib/secrets`
(`substrate-secrets`, all behavior incl. adapters) + `bin/secrets` (daemon +
`clap` CLI). Adapters are internal modules, not sub-crates.

## Thoroughness level

**implementation-ready** for the core: storage model (ciphertext-in-KV +
keychain), the three-tier envelope key hierarchy with rotation/enrollment, the
use-without-seeing SecretRef/resolve-to-sink model, the `llm_safe`
fail-or-degrade policy, scoping/provenance, the adapter trait + the three
build-now adapters (Supabase-reuse, local-mesh-db, GitHub Actions), the
genesis-key resolution, and the CLI/surface. **approach-sketched** for: the
AWS Secrets Manager adapter (DESIGN-ONLY by mandate — data contract only), the
`secrets-environments`/`openrouter-secrets` L6-stub edges (named, content
deferred), and the headless-Pi keychain fallback (approach given, the
allow-MRK-on-headless question is the operator's).

## Assigned design-depth

Opus (single Component-Designer pass, this file), per the wave2-plan model
tier, grounded in replicated-kv.md/types.md/db.md/aws.md/repo.md/rollup.md,
the real `lib/db/src/vault.rs`, and INTENT #78/#92/#94/#99/#105.

## Suggested fill-model

**implementation-ready + medium-high complexity → strong-mid model, crypto
tests FIRST.** The envelope/wrap/unwrap and AAD-binding must be conformance-
tested before anything else (round-trip, wrong-key-fails, relabel-attack-fails,
rotate-MRK-preserves-plaintext, enroll-then-decrypt, replica-without-MRK-
cannot-read), and the `llm_safe` refusal must be tested against a spoofed
caller identity. Adapters are transcription-grade once the trait is fixed (the
Supabase one is a WS/CLI shim over existing code). Do NOT send the crypto core
to a cheap model — the failure mode is silent disclosure, not a crash.

---

## Proposed contracts (wave 2)

Proposals only — the per-pair round reconciles; I do NOT edit
`scaffold/contracts/*`. Structs land in `types` (`types::secrets` for the
shared vocabulary; `SecretsError` in the reserved `types::error::secrets`).
The `SecretsAdapter` trait is an internal lib boundary, NOT a contract.

Shared vocabulary referenced by every edge below:

```rust
// types::secrets
pub struct SecretRef { pub id: Uuid, pub name: String, pub scope: SecretScope, pub version: u32 }
pub enum SecretScope { Global, Project{project:String}, Database{project:String,db:String}, Environment{project:String,env:String} }
pub struct SecretMeta { pub r#ref: SecretRef, pub updated_at: DateTime<Utc>, pub adapters: Vec<AdapterKind>, pub provenance: Provenance }
pub struct CallerContext { pub service_slug: String, pub llm_safe: bool, pub purpose: Option<String> }
pub enum AdapterKind { Supabase, LocalMeshDb, GitHubActions, AwsSecretsManager }
```

```rust
// types::error::secrets
pub enum SecretsError {
    NotFound { name: String },
    RawRefusedLlmSafe { secret: String },     // llm_safe raw request, fail mode
    NotDecryptable,                            // this node holds ciphertext but no MRK (unenrolled replica)
    KeychainUnavailable(String),               // root-of-trust access failed
    DecryptFailed,                             // AEAD tag / AAD mismatch (tamper or wrong key)
    AdapterFailed { adapter: AdapterKind, detail: String },
    ScopeInvalid(String),
    Store(String),                             // mesh-brokered KV surface error
}
```

### `secrets-mesh` (secrets ↔ mesh) — registration + the mesh-brokered `secrets/` keyspace

- **Purpose.** Two facets over the local mesh daemon (`:3649`, single-port
  locality): (a) ordinary registration/resolution (`service-lookup` instance);
  (b) a **keyspace-scoped KV surface** giving the registered secrets service
  put/get/scan/watch over the `secrets/` keyspace of `replicated-kv` — opaque
  ciphertext blobs, `Replication::All` (≥3 satisfied), `expiry_events:false`,
  `mirror_values:false`. Only the registered `secrets` slug may touch it (mesh
  enforces).
- **Message/struct sketch** (over `pubsub-protocol`/mesh WS):
  ```rust
  struct SecretsKvPut  { path: String, blob: Bytes, if_version: Option<u32> } // blob = the envelope, opaque
  struct SecretsKvGet  { path: String } -> Option<SecretsKvEntry>
  struct SecretsKvScan { prefix: String } -> Vec<SecretsKvEntry>
  struct SecretsKvEntry { path: String, blob: Bytes, kv_version: Version }     // Version from types::kv
  struct SecretsKvWatch { prefix: String } -> stream<SecretsKvEvent>           // key+version only, never blob on the mirror
  ```
- **Error cases.** `KeyspaceAccessDenied` (caller is not the `secrets` slug),
  `Store(...)` (wraps `KvError` — incl. `PartialReplication` surfaced so a
  `set` can report "written locally + on N peers, M pending"),
  `VersionMismatch` (CAS on `if_version`).
- **Version-sensitivity.** The blob is opaque end-to-end — the envelope format
  (concern 2) versions *inside* the blob (a 1-byte format tag), so KV and mesh
  never parse it and secret-format churn never bumps `kv_proto`. Rides KV's
  frozen `Version` total order. Registration/resolve is a plain
  `service-lookup` instance.

### `db-secrets` (secrets → db) — the Supabase Vault push adapter (REUSE, over the wire)

- **Purpose.** Push a secret's value into a Supabase project's `vault.secrets`
  by driving `db`'s existing `vault.rs` — over `db-control-plane` WS or the
  `db vault set` CLI, **never by linking `substrate-db`** (INTENT #29). Also
  covers metadata `exists`/`remove` and rendering the `reference_sql`
  expression so handlers reference the secret at runtime without storing
  plaintext.
- **Message/struct sketch.**
  ```rust
  // secrets -> db (maps onto db vault set/rm/exists over db-control-plane)
  struct DbVaultPush   { env: String, name: String, value_ciphertext_channel: SecureValue, description: Option<String> }
  struct DbVaultRemove { env: String, name: String }
  struct DbVaultExists { env: String, name: String } -> bool
  // db -> secrets
  struct DbVaultReceipt { name: String, outcome: SetOutcome }   // Created|Updated, from vault.rs
  ```
  `SecureValue` denotes the value crosses a non-LLM, in-mesh confidential
  channel (mesh-relayed WS between two trusted services) — it is the plaintext
  going secret→db-vault; it never transits an LLM path.
- **Error cases.** `AdapterFailed{Supabase,...}` wrapping db's typed
  `NotImplemented` (the `sqlite` driver has no Vault — degrade cleanly),
  connection/SQL errors stringified at the boundary (db's error never becomes
  a secrets type — types.md guardrail).
- **Version-sensitivity.** Low — `vault.rs`'s surface is stable, bound-param
  SQL; adapter is a thin shim. If `db` gains the operator-authorized
  reconciliation (its keychain reads become secrets-mediated), that is a `db`
  change tracked in `db.md`, additive to this edge.

### `vdb-secrets` (secrets ↔ vdb) — local-mesh-db push adapter (BUILD) + credential use-without-seeing

- **Purpose.** (a) Push a secret into a `vdb`/stack-hosted local SQLite
  database's secret facility (the SQLite equivalent of Supabase Vault: an
  encrypted column + decrypt-at-read view, keys rooted in secrets — the adapter
  v1 actually builds). (b) Hand `vdb` connection strings/credentials for its
  Supabase/AWS cloud targets, use-without-seeing (vdb receives a `SecretRef` +
  a `use(ref, connect)` verb, or a resolved connection assembled by secrets).
- **Message/struct sketch.**
  ```rust
  struct VdbSecretPush   { project:String, db:String, name:String, value:SecureValue }
  struct VdbCredentialUse{ r#ref: SecretRef, target: CloudTarget } -> ConnectionHandle // secrets connects; vdb never sees raw
  enum   CloudTarget { Supabase{project_ref:String}, AwsRds{arn:String} }
  ```
- **Error cases.** `AdapterFailed{LocalMeshDb,...}`, `NotFound`,
  `RawRefusedLlmSafe` (if a vdb *handler* is flagged llm_safe — normally not).
- **Version-sensitivity.** Medium — `CloudTarget` grows with vdb's adapter
  matrix (Supabase/RDS now, more later); additive, `#[serde(other)]` reserved.

### `repo-secrets` (secrets ↔ repo) — GitHub-Actions injection-on-push (THE v1 capability)

- **Purpose.** repo owns the secret↔workflow *linkage* and, on `git push`,
  asks secrets' GitHub-Actions adapter to push linked values into the repo's
  Actions secrets (libsodium sealed-box under the repo's Actions public key).
- **Message/struct sketch.**
  ```rust
  // repo -> secrets (on push, per linked secret)
  struct GhSecretPush   { owner:String, repo:String, gh_name:String, r#ref: SecretRef }
  struct GhSecretRemove { owner:String, repo:String, gh_name:String }
  // secrets -> repo
  struct GhPushReceipt  { gh_name:String, pushed:bool }
  ```
  secrets resolves `ref` → plaintext internally (non-LLM path), seals it,
  PUTs to the GH API; the value never returns to repo. repo supplies only
  linkage + identity.
- **Error cases.** `AdapterFailed{GitHubActions, detail}` (GH API/auth
  failure — the GH PAT is itself a Global secret here), `NotFound{ref}`.
- **Version-sensitivity.** Low — GH Actions secrets API is stable; the linkage
  shape is repo's to evolve. rollup is explicitly EXCLUDED from GH-Actions
  content (repo.md) — this edge carries only value push, never templating.

### `rollup-secrets` (secrets ↔ rollup) — raw/ID addressing under llm_safe

- **Purpose.** rollup resolves secret references embedded in fragments; secrets
  enforces the `llm_safe` fail-or-degrade so no raw secret enters LLM-bound
  rollup output (concern 8, both sides).
- **Message/struct sketch.**
  ```rust
  struct RollupSecretResolve { r#ref_or_name: SecretRefOrName, form: InsertForm, caller: CallerContext }
  enum   InsertForm { Reference, Raw }
  enum   ResolveOutcome { Reference(SecretRef), Raw(SecureValue), DegradedToReference{ r#ref:SecretRef, warning:String } }
  ```
- **Error cases.** `RawRefusedLlmSafe{secret}` (fail mode) — else
  `DegradedToReference` (degrade mode, the default) carrying the operator's
  warning text; `NotFound`.
- **Version-sensitivity.** Low, but the invariant is FROZEN: `Raw` under
  `caller.llm_safe==true` must NEVER be returned — that arm is unreachable by
  contract, part of the contract text, not commentary.

### `aws-secrets` (secrets ↔ aws) — SM push (DESIGN-ONLY) + S3 CSE keys + genesis resolution

- **Purpose.** Three things, all through the `aws` crate's WS surface (INTENT
  #105 — the adapter *lives in* `aws`): (a) **DESIGN-ONLY** AWS Secrets Manager
  push (data contract designed, not built v1); (b) hand `aws`/`vfs` the S3
  client-side-encryption **runtime** key for the overflow tier
  (use-without-seeing, `aws` is a trusted non-LLM caller → raw allowed);
  (c) the **genesis** kernel-state snapshot key resolution (concern 7): NOT a
  stored secret — `HKDF(MRK, "mesh-kv-snapshot")`, so a dead mesh restores from
  the operator's keychain/offline MRK with no running secrets service.
- **Message/struct sketch.**
  ```rust
  // (a) DESIGN-ONLY: secrets -> aws
  struct AwsSmPush { region:String, sm_name:String, value:SecureValue } // NOT built v1; contract only
  // (b) runtime S3 CSE key: aws/vfs -> secrets
  struct S3CseKeyGet { scope: SecretScope } -> SecureValue               // aws is non-llm_safe; raw allowed
  // (c) genesis: NOT a secrets-service call — aws/mesh derive locally from the keychain MRK
  //     documented here as the resolution of KV's aws-mesh circularity, no message
  ```
- **Error cases.** `AdapterFailed{AwsSecretsManager, ...}` (design-only —
  never fires in v1), `KeychainUnavailable`/`NotDecryptable` for the CSE key on
  an unenrolled node.
- **Version-sensitivity.** The SM adapter is design-only so its wire is a
  placeholder to be pinned when built. The **genesis-key rule is frozen** —
  changing the KDF label or source is a data-corrupting change (old snapshots
  become unrecoverable), exactly the class KV froze for `Version`.

### `secrets-environments`, `openrouter-secrets` — stub-track (anticipated, content deferred)

Both partners are L6 stubs ("not implementing now"). Named per wave2-plan §3c
so the pairs exist; content is deferred to when those modules leave the stub
track. **`secrets-environments`:** pushing/scoping secrets into a specific
environment — rides the `Environment{project,env}` scope (concern 6) and the
existing adapters targeted at that environment's backing store; no new
mechanism, just an environment-addressed push. **`openrouter-secrets`:**
storage/rotation of per-key OpenRouter API keys — ordinary Global/Project
secrets with rotation; `openrouter-mgmt` reads a `SecretRef`, never the raw
key, and uses it via a proxied call. Both satisfy the invariants above
unchanged.
