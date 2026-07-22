# secrets

**Status:** NEW (round-6 lock; **RENAMED `vault` → `secrets` round-8**);
**FULL DESIGN — wave 2, batch 3**; **CONSOLIDATED TO IMPLEMENTATION-READY —
wave 3** (adds the secrets-manifest requirement, chassis adoption, explicit
keep-ALL-versions, and the abstract blessing-queue relationship; this pass
supersedes the prior requirements-only stub and refines, not reopens, the
wave-2 design). **Nesting:** top-level L3 app-crate (`lib/secrets` crate
`substrate-secrets` + `bin/secrets` daemon/CLI, now **built on `chassis`**
— see concern 10). **Consumes (over the wire, never linked — INTENT #29):**
mesh via `secrets-mesh` (registration + the mesh-brokered `secrets/`
replicated keyspace), the OS keychain as root-of-trust, `db` via
`db-secrets` (the Supabase push adapter), `aws` via `aws-secrets` (SM push +
S3 CSE key handoff, design-only). **Consumed by:** `rollup`, `repo`,
`vdb`/`db` handlers, `cc`/`org`/`keeper` agents, `environments`, `aws`/`vfs`
(S3 encryption keys), `openrouter-mgmt`. Grounded in replicated-kv.md
(opaque-bytes replication, `mirror_values` redaction, the delegated
aws-mesh genesis-key circularity), db.md + `lib/db/src/vault.rs` (the real
Supabase-Vault module = the Supabase adapter), aws.md, repo.md, rollup.md,
chassis.md (the daemon-wrapper this crate now compiles against, incl. its
abstract `BlessingTarget` seam), types.md (guardrail 4, the reserved
`error/secrets.rs`), the wave-3 intent ledger (§A row 75, §C OQ-1
design-around, INTENT #143/#156).

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
Secrets Manager). It also owns the **secrets manifest** — the declared,
versioned record of which secrets a service *requires*, checked and
reconciled at deploy time (concern 10, INTENT #143). Its boundary — what it
does NOT own: the
**replication mechanism** (that is `replicated-kv` inside mesh; secrets is a
*tenant* of one mesh-brokered keyspace, not a replicator); **the secret↔workflow
*linkage*** (that is `repo`'s — secrets only pushes values when repo names the
link); **which environment/database a deploy targets** (that is
`environments`/`vdb` — secrets pushes where told); **its own daemon crypto
being novel** (it uses boring, audited AEAD envelope encryption — no
home-grown ciphers); **who blesses a consistency-requiring secret write**
(chassis's `BlessingTarget` seam is abstract by mandate — PARKED OQ-1, concern
11); and the external stores themselves (Supabase/GitHub/AWS hold their own
copies; secrets is the source of truth and one-way mirrors outward). It must
be "boring and not choppy."

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
  DEK, re-encrypt the value, bump `version` — the **prior version's ciphertext
  is kept, not tombstoned**; see concern 6's keep-ALL-versions rule, INTENT
  #156) and **rotate the MRK** (mint MRK′, rewrap every DEK under MRK′ —
  cheap, DEKs and ciphertexts are untouched — then re-enroll nodes with
  wrap(NK,MRK′)). Both emit `secrets.rotated` events (metadata only).

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
  - **injected into a spawned process's env** (cc/org agent processes, deploy
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

Services known to feed an LLM (`rollup`, `cc`, `inference` prompt assembly,
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

**Versioning: keep ALL versions, no cap (INTENT #156, LOCKED).** "Doesn't need
to be three — keep them all, version them." A secret's history is therefore
**append-only**, never pruned by the crate itself (a separate, later,
operator-invoked retention/archival policy could exist, but secrets never
silently drops a version). KV path is version-keyed, not name-keyed, so a
rotation never overwrites an entry: `secrets/<scope-encoded>/<name>/<version>`
holds one envelope per version forever; a small **pointer entry**
`secrets/<scope-encoded>/<name>/HEAD` holds the current version number and is
the only key rotation actually overwrites. Resolving "the current value" reads
`HEAD` then the pointed-at versioned entry; resolving `SecretRef{ .., version:
Some(v) }` reads that exact version directly — the same address form
`use-without-seeing` (concern 3) already exposes, now load-bearing for
history too: an agent may hold and reference an old `SecretRef` version
without ever having seen its plaintext. Because every version is real
ciphertext under its own `AAD`-bound envelope (concern 2), replaying an old
version costs nothing extra to keep and nothing extra to replicate-correctly
— `Replication::All` carries the full version history to every node exactly as
it carries the current one. `secrets ls --versions <name>` and `secrets reveal
<name> --version N` expose the history without adding a new access model.

Resolution across **scope** (not version) is **most-specific-wins** with an
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

`bin/secrets` is a noun-verb `clap` CLI over `lib/secrets`, and a daemon built
on **`chassis`** (concern 10 of `chassis.md`; see concern 10 below for what
that buys):

```
secrets set <name>  [--project P|--db P/D|--env P/E] [--from-stdin]   # create/update (value never in argv)
secrets ref <name>  [--scope …]        # print a SecretRef (safe to paste anywhere)
secrets ls          [--scope …] [--versions]   # metadata only; --versions lists the FULL kept history
secrets reveal <name> [--version N]    # the ONE plaintext path — local, audited, off the agent surface
secrets rm <name>
secrets rotate <name> | secrets rotate --mrk
secrets enroll <node> | secrets nodes  # key-hierarchy / enrollment ops
secrets push <name> --adapter supabase|vdb|github|aws --target <…>
secrets link <name> --workflow <owner/repo/workflow>   # delegates linkage to repo
secrets manifest check <service>       # ensure-exists: verify a service's declared requirements (concern 10)
secrets manifest ensure <service> --env E   # ensure-exists at deploy: create missing, error on unresolvable
```

The daemon publishes a `SurfaceSchema` (`types::surface`) rendering
metadata-only panels (never a value field; the `reveal` action carries
`confirm: true` and is dashboard-hidden), so the boring dashboard renders it
uniformly and agents can drive it — but the schema exposes no value-egress
action. `SecretsError` lands in `types::error::secrets` (the reserved slot).

### 10. Built on `chassis` — and the secrets manifest (INTENT #143)

**Chassis adoption.** `bin/secrets` compiles in `lib/chassis` (INTENT #156)
rather than hand-rolling registration/reconnect/restart-participation; what
was written above (round-8/wave-2) as "a daemon registering with the local
mesh (single-port locality, `mesh-client`)" now reads **"a service built on
`chassis`"** — `mesh-client` retires (ledger D1), and chassis's absorption of
its surface (register/resolve/subscribe/heartbeat/reconnect/re-announce, plus
the restart-protocol client half and the generated `Contract` trait for
`secrets-mesh`/`db-secrets`/`vdb-secrets`/`repo-secrets`/`rollup-secrets`/
`aws-secrets`) is a straight substitution — no behavior in this file changes,
only which crate supplies the plumbing. secrets supplies its own
`RestartPolicy` (its `Interruptibility` is `CriticalSection` mid-rotation or
mid-enrollment, `Idle` otherwise — a rotation or an MRK re-wrap must not be
torn mid-flight) and its own `Contract` impls (concern 9's verbs plus the
adapter push RPCs); chassis owns bring-up, the promise machinery for any push
adapter call that can't answer synchronously (a GitHub API round-trip, an AWS
SM call once built), and the outbox for any durable send. This is a pure
adoption, not a redesign — flagged here so the fill wave doesn't rebuild what
chassis already gives every service.

**The secrets manifest — services declare required secrets (INTENT #143,
NEW this wave).** A parallel branch's earlier db-vault work independently
converged on a **secrets manifest**: a service-side declaration of the
secrets it needs, checked so a calling service can't silently run without
one. secrets' version of that requirement, per the operator's words: *"link
directly to the secrets manager and ensure the secret exists — appropriate
migrate-up and migrate-down behavior, linked to VERSIONS of secrets."* Three
pieces:

- **The declaration.** A service ships a `SecretsManifest` — the same shape
  of thing `chassis`'s `.requires([dep("db", ">=1.4,<2"), …])` already does
  for service-to-service version floors (ledger §A row 62's sibling idea,
  applied to secrets instead of services):

  ```rust
  pub struct SecretsManifest {
      pub service: String,                      // the mesh service slug this manifest belongs to
      pub required: Vec<SecretRequirement>,
  }
  pub struct SecretRequirement {
      pub name: String,
      pub scope: SecretScopeTemplate,            // scope with holes: Project{project} filled at ensure-time
      pub min_version: Option<u32>,               // None = "any version, just must exist"
      pub optional: bool,                         // false by default — missing = hard block
  }
  ```

  The manifest is authored alongside the service (a `secrets.manifest.toml` or
  a `SecretsManifest::declare()` call at chassis bring-up — Filler's choice,
  not frozen here) and is itself content-addressed/versioned like any other
  service artifact, so "the manifest changed" is an observable, diffable
  event, not a side effect.

- **`ensure-exists` at deploy.** secrets subscribes to `repo.deployed`
  (environments.md's declarative deploy trigger, INTENT #103) and, for a
  service landing in a target environment/project, runs `manifest ensure`:
  for each `SecretRequirement`, resolve the templated scope, check existence
  (and `min_version` — concern 6's HEAD pointer makes "does version ≥ N
  exist" a cheap check), and:
  - **exists & satisfies `min_version`** → no-op, deploy proceeds;
  - **missing & not `optional`** → the deploy is **blocked**, a
    `SecretsError::ManifestUnsatisfied { service, name }` is surfaced (never a
    silent partial deploy — INTENT #38's "no technical debt by design" reading
    applied to deploy safety);
  - **missing & `optional`** → warned, deploy proceeds;
  - **exists but below `min_version`** → treated as missing for blocking
    purposes (the service asked for a version floor for a reason — a stale
    secret is not "close enough").
  This is the same shape as chassis's version-floor check on inter-service
  messages (chassis.md concern 2) — a manifest is a version-floor check
  applied to *secrets* instead of *services*, so the two mechanisms rhyme on
  purpose and a future harmonizer pass could plausibly unify their plumbing
  (flagged, not done here — different owners, same shape).

- **migrate-up / migrate-down, linked to secret VERSIONS.** Because secrets
  keeps ALL versions (concern 6), a manifest bump is naturally reversible:
  **migrate-up** = a manifest revision raises a requirement's `min_version` (or
  adds a new required secret) — `ensure` at the next deploy provisions/rotates
  to satisfy it. **migrate-down** = reverting to a prior manifest revision
  drops the requirement back to its old `min_version` (or removes it) —
  because no version was ever deleted, the rollback target's secret is
  **already there**, so migrate-down is a pure manifest-state change, never a
  data migration. This is the direct payoff of "keep all versions, no cap"
  (INTENT #156): rollback of a *secret requirement* costs nothing extra
  precisely because rollback of the *secret itself* was never needed. A
  manifest revision is addressed the same way a stack definition is
  (`vdb.md`'s ledger-head-as-version idiom) — a small, append-only history,
  not a mutable pointer.

**Prior art, credited not rebuilt.** The parallel-branch db-vault work is
cited as the origin of the manifest idea (env-files + gitignore version,
service-declares-requirements) — this design generalizes it to secrets'
scope/version model rather than adopting its literal shape, since that branch
predates the `SecretScope`/keep-all-versions decisions this crate already
made. Contract seam: `secrets-environments` gains the `repo.deployed`
subscription and the `ManifestUnsatisfied` error as an anticipated addition
(named here; content deferred with the rest of that stub-track contract,
consistent with its existing status).

### 11. The blessing queue's relationship to secret writes — ABSTRACT (PARKED OQ-1)

chassis (INTENT #157) gives every service, secrets included, a **blessing
queue**: a consistency-requiring change can be enqueued locally and submitted
to an abstract `BlessingTarget` on reconnect (chassis.md concern 7). Whether
a *secret write* is one of those consistency-requiring changes is left
**deliberately unresolved** here, per the ledger's OQ-1 design-around rule
(§C: "keep flexible... do not thread an authority dependency into any other
contract").

**What this file commits to:** secrets writes (set/rotate/enroll) already
have a correctness story that does not depend on blessing —
`replicated-kv`'s naive last-write-wins + HLC ratchet (ledger §A row 29)
governs ordinary KV convergence, and MRK rotation's re-enrollment step is
already an explicit, operator-confirmed, one-node-at-a-time act (concern 2)
that is not racy the way an offline double-write could be. So secrets does
**not** require the blessing queue to function correctly at v1.

**What is left open, on purpose:** a *future* tightening — e.g. "two disjoint
partitions each rotated the same secret while split, which rotation wins" —
is exactly the kind of consistency-requiring race OQ-1's authority-vs-no-
authority question is about, and secrets **could** later route MRK rotation
and enrollment through chassis's blessing queue as a `BlessingRequest` (its
`schema_id` would be the secrets-rotation schema, its `change_id` the
secret's `id` + target `version`). This file does **not** wire that up now:
no code path in this design submits a `BlessingRequest`, and no contract here
gains a `BlessingTarget` dependency. The seam is named so a future pass has
somewhere obvious to attach it, and left otherwise untouched — abstract, as
directed.

## Relationships / edges

- **mesh** via `secrets-mesh` — registration/resolution (a `service-lookup`
  instance) **plus** the mesh-brokered `secrets/` replicated KV keyspace
  (opaque ciphertext, `Replication::All`, `mirror_values:false`) that is
  secrets' distributed store (concern 1). *(scaffold/contracts/secrets-mesh.md
  — exists, requirements-only; this design proposes its content below.)*
- **db** via `db-secrets` — secrets drives `db`'s existing Supabase-Vault
  module (`vault.rs`) as the Supabase push adapter, over WS/CLI, never linked
  (INTENT #29/#105). *(authored at the contract round.)*
- **vdb**/`stack` via `vdb-secrets` — the local-mesh-database push adapter
  (BUILD) + connection-string/credential use-without-seeing for cloud targets.
  *(authored at the contract round.)*
- **repo** via `repo-secrets` — repo names the secret↔workflow linkage; secrets'
  GitHub-Actions adapter pushes values on `git push` (injection-on-push, the
  v1 capability). *(authored at the contract round.)*
- **rollup** via `rollup-secrets` — raw/ID addressing from rollup content;
  `llm_safe` fail-or-degrade governs; the no-secret-in-LLM-output invariant
  (concern 8). *(authored at the contract round.)*
- **aws** via `aws-secrets` — AWS Secrets Manager push adapter (DESIGN-ONLY,
  lives in `aws`) **and** the S3 CSE runtime-key handoff + the MRK-derived
  genesis snapshot-key resolution (concern 7). *(authored at the contract round.)*
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
- **chassis** — **shared-lib dependency, NOT a contract edge** (like `schema`'s
  codegen dependency on chassis's own edges list). `bin/secrets` compiles in
  `lib/chassis` for bring-up/reconnect/restart/promise/outbox mechanics
  (concern 10); the abstract `BlessingTarget` seam is present but unused by
  this design (concern 11, PARKED OQ-1). → `scaffold/components/chassis.md`

## Nesting

Parent: none | Children: none. Dual-role crate like `db`/`gc`: `lib/secrets`
(`substrate-secrets`, all behavior incl. adapters) + `bin/secrets` (daemon +
`clap` CLI). Adapters are internal modules, not sub-crates.

## Thoroughness level

**implementation-ready** for the core: storage model (ciphertext-in-KV +
keychain, now explicitly **version-keyed with a HEAD pointer, keep-ALL-
versions**), the three-tier envelope key hierarchy with rotation/enrollment,
the use-without-seeing SecretRef/resolve-to-sink model, the `llm_safe`
fail-or-degrade policy, scoping/provenance, the adapter trait + the three
build-now adapters (Supabase-reuse, local-mesh-db, GitHub Actions), the
genesis-key resolution, the CLI/surface, **chassis adoption** (concern 10),
and the **secrets-manifest / ensure-exists / migrate-up-down model** (concern
10). **approach-sketched** for: the AWS Secrets Manager adapter (DESIGN-ONLY
by mandate — data contract only), the `secrets-environments`/
`openrouter-secrets` L6-stub edges (named, content deferred; the manifest's
`repo.deployed` hook is named on `secrets-environments` but not built), the
headless-Pi keychain fallback (approach given, the allow-MRK-on-headless
question is the operator's), and the manifest's own authoring format
(`.toml` vs a `declare()` call — Filler's choice). **Deliberately abstract by
mandate:** the blessing queue's relationship to secret writes (concern 11,
PARKED OQ-1) — the seam is named, nothing wired.

## Assigned design-depth

Opus (single Component-Designer pass, wave 2; consolidated wave 3 by the
`secrets-consolidated` unit, same file), per the wave2-plan/wave3 model tier,
grounded in replicated-kv.md/types.md/db.md/aws.md/repo.md/rollup.md/
chassis.md, the real `lib/db/src/vault.rs`, INTENT #78/#92/#94/#99/#105/#143/
#156, and the wave-3 intent ledger (§A row 75, §C OQ-1).

## Suggested fill-model

**implementation-ready + medium-high complexity → strong-mid model, crypto
tests FIRST.** The envelope/wrap/unwrap and AAD-binding must be conformance-
tested before anything else (round-trip, wrong-key-fails, relabel-attack-fails,
rotate-MRK-preserves-plaintext, enroll-then-decrypt, replica-without-MRK-
cannot-read, **version-history-never-overwritten**), and the `llm_safe`
refusal must be tested against a spoofed caller identity. Adapters are
transcription-grade once the trait is fixed (the Supabase one is a WS/CLI shim
over existing code). The manifest's `ensure-exists` check (concern 10) is
mid-model-safe (a lookup + comparison, no crypto). Do NOT send the crypto core
to a cheap model — the failure mode is silent disclosure, not a crash. Fill
secrets **after** chassis is filled (concern 10's adoption is a real build
dependency, not just a design reference).

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `secrets-mesh` — (secrets ↔ mesh) — registration + the mesh-brokered `secrets/` keyspace. → `scaffold/contracts/secrets-mesh.md`
- `db-secrets` — (secrets → db) — the Supabase Vault push adapter (REUSE, over the wire). → `scaffold/contracts/db-secrets.md`
- `vdb-secrets` — (secrets ↔ vdb) — local-mesh-db push adapter (BUILD) + credential use-without-seeing. → `scaffold/contracts/vdb-secrets.md`
- `repo-secrets` — (secrets ↔ repo) — GitHub-Actions injection-on-push (THE v1 capability). → `scaffold/contracts/repo-secrets.md`
- `rollup-secrets` — (secrets ↔ rollup) — raw/ID addressing under llm_safe. → `scaffold/contracts/rollup-secrets.md`
- `aws-secrets` — (secrets ↔ aws) — SM push (DESIGN-ONLY) + S3 CSE keys + genesis resolution. → `scaffold/contracts/aws-secrets.md`
- `secrets-environments` / `openrouter-secrets` — stub-track (anticipated, content deferred). → `scaffold/contracts/secrets-environments.md`, `scaffold/contracts/openrouter-secrets.md`

Also a party to (authored elsewhere / cross-cutting): `db-control-plane`, `pubsub-protocol`, `service-lookup` — see `scaffold/contracts/`.

## Proposed contracts (wave 3)

secrets does not own any contract file (all are shared with a party crate),
so wave-3 contract-shape changes are recorded here as **proposals for the
harmonizer / the owning contract's next revision**, not applied directly to
files this unit doesn't own:

- **`chassis` shared-lib adoption is NOT a contract edge** (see Relationships)
  — no contract file needs a new party, but every existing `secrets-*`
  contract's "registers with the local mesh" language should be read as
  "registers via chassis" going forward. No wording change forced on the
  contract files themselves (they already say "over the wire, mesh-relayed,"
  which remains true).
- **`secrets-environments` gains an anticipated hook**: a `repo.deployed`
  subscription and a `SecretsError::ManifestUnsatisfied { service, name }`
  error variant, for the manifest's `ensure-exists` check (concern 10). Named
  here; **not authored** — `secrets-environments` stays stub-track content-
  deferred (environments.md is L6, NOT implemented in v1) and this unit does
  not edit that contract file. The harmonizer or a future environments design
  pass should fold this in when that file leaves the stub track.
- **`SecretsManifest`/`SecretRequirement` land in `types::secrets`** alongside
  `SecretRef`/`SecretScope` (types.md is a sibling-owned file; flagged for the
  harmonizer, not edited here) — they are pure data shapes with no wire
  behavior of their own; `manifest check`/`manifest ensure` are `bin/secrets`
  CLI verbs, not new mesh RPCs, so no existing contract's message enum needs a
  new variant to support them.
- **No `BlessingTarget` dependency added anywhere** (concern 11) — recorded
  explicitly so a future contract-graph audit can confirm this file introduced
  none, consistent with chassis.md's own "no authority dependency threaded
  into any other contract" discipline.

