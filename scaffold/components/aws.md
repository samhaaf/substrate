# aws

**Status:** NEW (round-9 lock, 2026-07-19); **wave-2 full design pass
(2026-07-19).** **Nesting:** top-level app-crate (L2). This file promotes the
prior round-9 requirements-only stub to a designed component: the internal
adapter architecture, the credential/encryption boundaries, the bulk-transfer
problem under single-port locality, the not-built-now posture, and the four
contract pairs wave2-plan assigns aws (`aws-mesh`, `aws-vfs`, `aws-vdb`,
`aws-secrets`).

> **SUPERSESSION NOTE (round-9, preserved):** this crate corrects the earlier
> "S3 crate/adapter" phrasing and the rounds-4–5 "S3 adapter lives inside
> mesh" placement. Operator, verbatim: "I definitely meant AWS crate." **The
> S3/AWS adapter surface lives HERE**; `mesh` (via `replicated-kv`), `vfs`,
> `vdb`, `secrets` (and `kg`, transitively via mesh) CONSUME it. The
> S3-overflow *requirements* are unchanged — only the owner moved.

## Charter

The **AWS virtualization-layer crate** — the single aggregation point for
**every** AWS-facing surface Mind OS uses. When a Mind OS service needs to do
something in AWS, it does it through `aws`, over the mesh, never against boto3
or Terraform directly. Operator, verbatim (INTENT #106): "a place to aggregate
our adapters, with a WebSocket interface for pushing to, reading from, and
pulling against AWS in exactly the ways we need."

`aws` **IS** a mesh service like any other: it is an app-crate (`bin/aws`
daemon + `lib/aws` internal adapter libs) that embeds `mesh-client`, registers
the slug `aws`, speaks `mesh-transport`/`pubsub-protocol` over the local
`:3649` daemon (single-port locality, INTENT #58), publishes a boring surface
schema, and participates in the restart protocol. It is addressed
**virtualized** (`AnyNode { slug: "aws" }`) — consumers don't care which node
runs it; mesh routes to wherever the singleton instance lives (the node with
good internet egress). It holds AWS account credentials **only** by fetching
them from `secrets` at use time — it is **never its own credential store**.

What `aws` owns:
- **S3** — the VFS overflow tier (cold-storage classes, opaque encrypted
  objects) and `replicated-kv`'s cloud snapshot leg (disaster recovery).
- **RDS + Lambda** — VDB's cloud deploy target (the third VDB adapter after
  local-SQLite and Supabase; INTENT #93/#96). SQLite-locally is LOCKED, so AWS
  is strictly a *cloud promotion* target, never a local runtime.
- **AWS Secrets Manager** — `secrets`' AWS push adapter (**DESIGN-ONLY in
  v1**), plus the reverse leg: sourcing `aws`'s own AWS account credentials
  from `secrets`.
- **SQS someday** — mesh's SQS-modeled `queues` deploying onto real SQS
  (INTENT #89) route through here; anticipated, no contract this wave.
- **Containers, if ever** (INTENT #72) — the hard no-Docker rule is
  *local-only*; any container ever runs *in AWS* (App Runner / ECS / Lambda),
  which is `aws`'s territory. No contract this wave.

**Boundary — what `aws` does NOT own / is NOT:**
- **NOT a boto3 or Terraform replacement.** No general-purpose AWS SDK
  ambitions, no infrastructure-as-code, no service catalog. `aws` exposes
  **only** the specific operations a Mind OS service actually consumes, and
  new operations are added **as they are needed**, never speculatively
  (INTENT #38 no-tech-debt: a narrow surface stays boring).
- **NOT a credential store.** It reads AWS account creds from `secrets` and
  holds them only in memory for the life of a request/session; it persists no
  credentials, ever.
- **NOT the holder of data-encryption keys.** For S3 object bodies, `aws`
  stores **opaque ciphertext** — the plaintext and the per-object data key
  never reach it (concern 4). "Client-side encryption via secrets" is a
  *consumer-side* operation; `aws` provides the storage class and the S3
  mechanics, not the crypto.
- **NOT built now.** Per INTENT #105 ("we're not doing pretty much anything in
  AWS right now — say where the adapter is going to live"), v1 ships a
  **contract-complete stub** (concern 6): it registers, publishes its surface
  schema, and answers every real operation with `AwsDisabled`, so consumers
  can develop and conformance-test against the frozen contracts before the SDK
  code lands.
- **NOT the replication *policy* engine.** It is a **passive, push-only peer**
  (INTENT #34 spirit): mesh decides *when/what* to snapshot or overflow; `aws`
  only executes the push/read/pull. AWS being unreachable must never block
  anything in the mesh.

## Primary design concerns

### 1. The adapter-aggregation architecture (boring layers, added as needed)

`aws` is an app-crate that decomposes internally with the same boring-layers
discipline as mesh (INTENT #55). Three rings, strictly bottom-up:

```
Ring 0  aws SHELL (bin/aws + lib/aws root)
        · mesh-client boot: register slug "aws", open the :3649 session
        · RequestRouter: dispatch an inbound mesh-transport Request by
          (adapter, op) to the right adapter module; stream large bodies
        · CredentialProvider: fetch AWS account creds from `secrets` on
          demand, cache in-memory with TTL, never persist (concern 2)
        · SurfaceSchema + aws.* event publisher (concern 9)
        · Capabilities gate: feature-flags each adapter enabled/stub
          (concern 6)

Ring 1  aws-session  (shared)
        · one thin typed wrapper over the official AWS SDK for Rust
          (aws-sdk-s3, aws-sdk-rds, aws-sdk-lambda, aws-sdk-secretsmanager,
          aws-sdk-sqs) — region/endpoint/retry/backoff/idempotency, ONE place
        · NOT a general SDK re-export: exposes only what Ring-2 adapters call

Ring 2  per-service ADAPTERS (one internal lib module each)
        · s3        — overflow objects + kv snapshots (concerns 3,4,5)
        · rds       — VDB cloud target provisioning/describe (design-only)
        · lambda    — VDB handler deploy/invoke (design-only)
        · secretsmgr— secrets' AWS push adapter (design-only)
        · sqs       — someday; not present in v1
```

Each Ring-2 adapter is its own module under `lib/aws` and maps a **narrow,
need-shaped** request enum onto Ring-1's SDK wrapper. This is what makes
"added as they're needed" structural rather than aspirational: a new AWS need
= one new op variant + its handler, no cross-cutting change. The rings are
compiled into one `bin/aws` process (never separate crates — the same reason
mesh's utility libs nest).

### 2. Credential handling — via `secrets`, never a local store

`aws` needs two *categories* of secret, kept strictly separate:

- **AWS account credentials** (access-key/secret, or preferably a role ARN to
  assume via STS) — what lets `aws` talk to S3/RDS/Lambda/SM at all. The
  `CredentialProvider` (Ring 0) fetches these from `secrets` via the
  `aws-secrets` credential-fetch facet (concern-2 of that contract below),
  caches them in memory with a short TTL, refreshes on expiry, and **persists
  nothing**. On a cred-fetch failure every real operation degrades to
  `CredentialsUnavailable` — it never falls back to `~/.aws/credentials` or
  environment ambient creds (that would be an unaudited backdoor store; INTENT
  #38/#85). `secrets` is the single source of truth.
- **Per-object data-encryption keys** — `aws` **never** holds these
  (concern 4). They belong to the consuming service.

This is the clean split that reconciles the charter's "client-side encryption
via secrets" phrasing with the two consuming contracts' "keys held by
`secrets`, never by aws or vfs": `aws` holds the *account* credential to reach
AWS; the *data* key stays consumer-side.

### 3. The bulk-transfer problem under single-port locality (the hard part)

This is why `aws` earned a designed component rather than a one-liner. Two
consumers move genuinely large byte volumes through it — VFS overflow (cold
files, potentially many GB) and, smaller, KV snapshots. But single-port
locality (INTENT #58) says all traffic funnels through the local `:3649`
daemon, and `pubsub-relay` is explicitly *lossy and capped* (`PayloadTooLarge`)
— wrong for reliable bulk transfer. So bulk object bodies need a deliberate
transfer design. Two paths:

- **Path A — relay bytes through mesh.** Consumer streams ciphertext to `aws`
  over a *reliable* `mesh-transport` request/response streaming channel
  (chunked frames + backpressure, the completion-router `forward()` model, NOT
  pub/sub); `aws` streams it into S3. Pure single-port locality; but every
  cold byte crosses the personal mesh relay twice (consumer→aws-node→S3),
  loading mesh and the aws node.
- **Path B — presigned handle.** `aws` (control plane) mints a short-lived
  **presigned S3 PUT/GET URL** (with the storage class and object lock/policy
  already applied) and returns it; the consumer streams ciphertext **directly
  to S3** over its own egress. `aws` never touches the bulk bytes; scales to
  any object size; keeps `aws` a pure control plane. Cost: the bulk data path
  leaves the mesh (a deliberate exception to "everything over WebSockets",
  INTENT #28), and the consumer node needs direct S3 egress.

**CONFIRMED (friction-round 1, INTENT #114) — direct-upload with mesh-issued
presigned permission.** Transfer mode is a **negotiated field per request**,
defaulting to **Path B (presigned) for the VFS bulk data path** (`aws-vfs`)
and **Path A (inline/relay) for KV snapshots** (`aws-mesh`, bounded kernel
state). The contract carries a `Transfer` enum so a consumer that must stay
strictly on-mesh can request `Relay` and a consumer optimizing for scale gets
`Presigned`. The operator confirmed the flow after the presigned-URL mechanism
was explained: the deliberate exception to "everything over WebSockets"
(INTENT #28) for bulk bytes is accepted; single-port locality remains the
fallback and the default for small payloads. **Routing note (operator's
framing):** the mesh, as router, **locates the node holding the data and tells
it to send to the presigned URL** — and this instruction may ride the queue OR
a direct service-router path: "queues are for generic application logic and
operating-system logic," so this specific flow doesn't have to use one.

### 4. Client-side encryption boundary — `aws` stores opaque ciphertext

For every S3 object body, encryption happens **before `aws` (and before S3)
ever sees the bytes**, using a data key the consumer fetches from `secrets`
(INTENT #48/#98). Consequences, all deliberate:
- `aws` sees only ciphertext (Path A) or not even that (Path B, presigned) —
  it never sees plaintext and never sees the data key.
- This is **client-side encryption** in the S3 sense (the client, i.e. the
  Mind OS consumer, encrypts; not S3 server-side/SSE), giving encryption in
  transit *and* at rest under a key Amazon never holds.
- `aws` still sets S3's own server-side options (bucket policy, storage class,
  object lock) as defense-in-depth, but security does not depend on them.
- **The genesis-key circularity** (surfaced by `replicated-kv`): the KV
  snapshot key can't live *inside* the mesh it exists to restore. That key
  lives outside the system (OS keychain / operator-held). `aws` is unaffected
  — it stores whatever ciphertext it's handed — but the friction is real and
  belongs to `secrets`+`replicated-kv`, re-flagged below.

One exception to "aws never sees a secret value": the **Secrets Manager push
adapter** (`aws-secrets`, design-only) is by definition a conduit that carries
a secret value *to* AWS SM (SM re-encrypts under KMS at the destination). Here
`aws` is a transient conduit to the destination store, never a persister —
acceptable because `aws` is a service, not an agent (the use-without-seeing
invariant governs agents/LLMs, INTENT #78/#94, not service-to-service
transport). Flagged in the contract.

### 5. Storage classes, lifecycle, and restore

`aws` exposes S3's cold-storage tiers (INTENT #48) as an explicit per-object
`S3Class` the **consumer** chooses from its own policy (VFS eviction tier / KV
snapshot cadence): `Standard | IntelligentTiering | StandardIa |
GlacierInstant | GlacierFlexible | DeepArchive`. `aws` applies the class on
PUT and reflects it on HEAD. Glacier-tier objects are not instantly readable:
a GET on an archived object returns `InRestore`, and a `Restore { tier }`
op initiates the thaw (`aws` surfaces the async restore state via a `HEAD`
`restore` field and an `aws.s3.restore_ready` event). Lifecycle policies
(auto-tiering rules) are set once per bucket-namespace at provisioning; `aws`
does not run its own tiering loop (that's mesh/VFS policy — `aws` executes,
mesh decides). This keeps `aws` boring: it is a class-aware object conduit,
not a storage optimizer.

### 6. The not-built-now posture — a contract-complete stub

v1 does **not** implement real AWS calls (INTENT #105). But "not built" must
not mean "consumers can't develop against it." The stub posture:
- **Registers and publishes surface schema** exactly as the real service will,
  so it appears in the dashboard and resolves via `service-lookup`.
- **Every real operation returns `AwsDisabled { adapter, op }`** — a single,
  catchable, well-known error every consumer already handles as "cloud
  overflow/promotion unavailable, stay local" (which is the *normal* v1 state:
  local SQLite, local VFS, no promotion). Nothing in the mesh depends on AWS
  succeeding (concern: passive push-only peer).
- **A `Capabilities` gate** (mirroring `db`'s Capabilities-gated drivers)
  flips each adapter from `Stub` to `Live` independently as it's built, so S3
  can go live long before RDS/Lambda without touching consumers.
- **Contract conformance is met by the stub**: the stub answers the full
  message schema (including well-formed `AwsDisabled` for real ops and real
  answers for cheap metadata like "am I enabled"), so the contract-conformance
  gate (Scaffolding step 5) passes against fixtures with zero AWS account.

This is the design-buys-down-fill payoff: the contracts are
implementation-ready, the *implementation* is a deliberate stub, and the
transition to `Live` is a per-adapter flag flip, not a redesign.

### 7. Failure isolation — AWS down is never a mesh fault line

`aws` is a **passive, push-only peer** (INTENT #34). Every operation is
best-effort and independently retryable; `aws` unreachable or AWS itself down
surfaces as a catchable error to the one consumer that asked, plus an `aws.*`
health event, and **blocks nothing else**. There is no fault line where the
mesh depends on AWS: overflow that can't push stays local and re-tries later;
a snapshot that can't export retries next schedule; a VDB promotion that can't
provision fails the promotion and leaves the local database untouched.
Idempotency keys on writes (concern 1, Ring 1) make retries safe.

### 8. Placement, region, and account model

`aws` is a **singleton** addressed virtualized. Its node placement (which
device runs the instance) is a `supervision` boot-order concern — it should
run where internet egress is reliable (the laptop, or a cloud-adjacent node),
not necessarily on every node. Region/account config lives in `aws.toml`
(region, bucket-namespace roots, role-ARN references — the ARNs are *pointers*
into `secrets`, not values). Multi-account is future; v1 assumes one account.

### 9. Observability + provenance

`aws` publishes its **boring surface schema** (INTENT #46) so the dashboard
renders it with no bespoke UI, and emits typed events on the `aws.*` topic
(`aws.s3.put`, `aws.s3.restore_ready`, `aws.export_failed`, `aws.rds.*`, …)
over `pubsub-protocol`. Every AWS operation is traced with first-order
provenance (INTENT #85): who asked, what object/target, when, and the causal
parent — so a data engineer can see the full chain from a handler touch to an
S3 object landing. Provenance is *lighter* here than in VDB (this is transport,
not the database plane) but present from the start.

## Relationships / edges

- **mesh** via `aws-mesh` — `aws` registers/resolves like any service
  (`service-lookup`) through the local `:3649` daemon, and RECEIVES
  `replicated-kv`'s cloud snapshot leg (export/list/fetch of encrypted kernel
  snapshots to/from S3). SQS-someday (mesh `queues` → real SQS) rides this
  edge too, anticipated. `replicated-kv` authors the snapshot *payload* half;
  `aws` authors the S3-mechanics half (authored — see Contracts section).
  (scaffold/contracts/aws-mesh.md).
- **vfs** via `aws-vfs` — the S3 overflow tier: VFS pushes/reads/pulls/lists
  object bodies (opaque ciphertext) against S3 through `aws`, choosing storage
  class and transfer mode. `aws` mints presigned handles or relays bytes; it
  never holds the data-encryption key. (scaffold/contracts/aws-vfs.md).
- **vdb** via `aws-vdb` — **NEW (proposed).** VDB's RDS+Lambda cloud target:
  provision/describe an RDS instance, deploy/invoke Lambda handlers, run the
  copy/verify migration under a lock (local→cloud promotion). Connection
  *secrets* for the target come via `vdb`↔`secrets` (`vdb-secrets`), NOT from
  `aws`. Design-only in v1. *(authored: scaffold/contracts/aws-vdb.md)*
- **secrets** via `aws-secrets` — **NEW (proposed).** Two facets: (1)
  secrets→aws, the AWS Secrets Manager PUSH adapter (design-only, INTENT
  #105); (2) aws→secrets, `aws` sourcing its own AWS account credentials
  (concern 2). *(authored: scaffold/contracts/aws-secrets.md)*
- **kg** (anticipated, no direct edge) — KG's cross-boundary S3 sync (INTENT
  #50) flows `kg → mesh → aws`; it rides `aws-mesh`, not a direct stub (see
  `components/kg.md`).
- **queues** (anticipated, no stub this wave) — mesh's SQS-modeled queues
  deploying onto real SQS route through `aws` when built; rides `aws-mesh`.

Internal-lib dependencies (compiled in, NOT contract edges, INTENT #29/#45):
`mesh-client` (boot/register/resolve + the transport client half), `types`
(shared vocabulary — `Slug`, `NodeId`, `Endpoint`, `Provenance`, and the aws
request/response structs I propose below land in a `types::aws` module), and
the official AWS SDK for Rust crates (a third-party lib dependency).

## Nesting

Parent: none (top-level app-crate) | Children (nested internal adapter libs in
`lib/aws`, compiled in, never standalone): `aws-session`, `s3`, `rds`,
`lambda`, `secretsmgr` (and `sqs` when it exists). The parent/child structure
lives here and in overview.md, not in the flat `components/` directory layout.

## Thoroughness level

**implementation-ready for the contracts and the architecture** (the ring
decomposition, the credential/encryption boundaries, the transfer-mode model,
the not-built-now stub posture, and the four contract shapes are all decided),
but **the SDK implementation itself is a deliberate STUB in v1** — so the
component as a *shipped runtime* is intentionally requirements/approach-level
while its *contracts* are implementation-ready. The transfer-mode default is
**CONFIRMED** (presigned direct-upload for bulk, friction-round 1 — INTENT
#114, concern 3); genuinely open: everything downstream
of a live account (RDS/Lambda provisioning shapes firm up when VDB's cloud
target is actually built).

## Assigned design-depth

**Opus** single strong-model Component-Designer pass (this file), grounded in
the round-9 `aws.md` requirements stub, the consuming designs (`vfs.md`,
`secrets.md`, `stack.md`, `replicated-kv.md`'s `aws-mesh` leg, `mesh-core.md`'s
transport/addressing seams, `pubsub-relay.md`'s envelope), and INTENT items
34/48/58/72/85/93/98/105/106.

## Suggested fill-model

**implementation-ready contracts + stub implementation → cheap model OK for
the stub.** The v1 deliverable is: register via `mesh-client`, publish surface
schema, answer the contract with `AwsDisabled`/metadata — near-transcription of
the frozen contract, well within a cheap model. When an adapter goes `Live`
later, that adapter's fill wants a **mid model** (the S3 presigned/relay
transfer path and the retry/idempotency layer are the only subtle parts; the
per-service adapters are thin SDK maps). Do **not** spend a strong model on the
whole crate now — the design has bought the fill down.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `aws-vfs` (aws ↔ vfs) — the S3 overflow tier data path (cold classes, client-side encryption). → `scaffold/contracts/aws-vfs.md`
- `aws-mesh` (aws ↔ mesh) — registration + the replication plane's S3/AWS distribution leg. → `scaffold/contracts/aws-mesh.md`
- `aws-vdb` (vdb → aws) — the RDS+Lambda cloud target; DESIGN-ONLY v1 (`AwsDisabled`). → `scaffold/contracts/aws-vdb.md`
- `aws-secrets` (secrets ↔ aws) — SM push (design-only) + AWS creds fetch + S3 CSE keys + the genesis-key rule. → `scaffold/contracts/aws-secrets.md`

Also a party to (authored elsewhere / cross-cutting): `restart-protocol`, `service-lookup` — see `scaffold/contracts/`. (`vdb-secrets` is related but aws is deliberately NOT a party: the RDS connection secret flows vdb ↔ secrets; aws returns only the endpoint.)

