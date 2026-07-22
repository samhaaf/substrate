# Contract: schema-query

## Parties

- **any service** (via `chassis`, the daemon-wrapper client half) — the DC
  runtime *client*: lists/fetches published schemas + libraries, resolves a
  `SchemaId@version` to a runtime descriptor, checks a compiled library manifest
  for skew, and (as an author) publishes schemas/libraries and requests a
  structure migration.
- **schema** (`bin/schema` + `lib/schema`, L4 data plane) — the *server* half:
  the `query` child lib answers reads and the `check` call; `registry` +
  `codegen` + `migrate` answer the write/publish/migrate calls.

Cross-cutting, surface-schema-style: **ONE shared document, every service a
potential party** (the same shape `queues-api` / `mesh-transport` use). schema is
always the server; any service is a client.

## Purpose

This is the **runtime half of the emergent data-contract (DC) surface** (INTENT
#166 Q11 / #150 beat 16). The DC surface is a *pair* — (1) the **compiled** half:
the versioned generated library crates `schema` emits (`gen/<lib>@<version>`),
consumed as a Cargo/codegen dependency, **NOT a contract edge** (schema.md concern
4a / P2); and (2) this **runtime** half: the WS query surface a service speaks to
its LOCAL mesh daemon on `:3649` to interrogate and publish the *same* schema data
the crates were generated from, served live. There is deliberately **no `dc`
crate** — "DC" is only the *name of this published-type surface*; it falls out of
`schema` + `chassis` + the generated libraries (INTENT #151/#166 Q11, superseding
the wave-2 "DC service" musing, which is explicitly NOT built).

The surface serves its most important customer, **the landscape itself**: the
keeper-type and artifact-type schemas (schema.md concern 8; `keeper.md` /
`artifacts.md`, batch 6) are ordinary schemas reachable here, and the built-in
`landscape-core` library is published/fetched over this contract.

## Schema

**Struct home — deliberately NOT `types`.** Unlike most contracts (written in
terms of `substrate-types`), the schema-query vocabulary is homed in `schema`'s
own child libs — `lib/schema::model` (`SchemaDefinition`, `FieldSchema`,
`Migration`, `SchemaId`, …) and `lib/schema::query` (the request/response enums +
`check`) — because the payload types ARE schema's domain model and only `schema`
is ever the server. This preserves the **anti-layer-cycle rule** (schema.md
concern 4a, ledger §C): the *generated* library crates depend on
`serde`/`uuid`/`chrono` ONLY, never on `substrate-types` and never on `schema`
itself, so `chassis` (L1) can Cargo-depend on a generated library even though the
`schema` *tool* is L4. `types` stays out of this edge on purpose; a client
consumes the `schema` crate's public model types (or the JSON-schema runtime
descriptor for non-Rust consumers).

### Client → schema (`SchemaQuery`)

```rust
enum SchemaQuery {
    // ── reads (the runtime DC surface) ──────────────────────────────────
    ListSchemas,                                     // -> Vec<(SchemaId, SchemaHead)>
    GetDefinition   { id: SchemaId, version: u32 },  // -> SchemaDefinition (raw DAG, provenance-complete)
    GetEffective    { id: SchemaId, version: u32 },  // -> EffectiveSchema  (flattened; the codegen/validate input)
    ListLibraries,                                   // -> Vec<(LibraryId, LibraryHead)>
    GetLibrary      { id: LibraryId, version: u32 }, // -> LibraryManifest  (MEMBERS + gen-crate coordinates)
    ResolveDescriptor { id: SchemaId, version: u32 },// -> RuntimeDescriptor (JSON-schema-shaped; non-Rust consumers)

    // ── OQ-20 skew check — REPORT ONLY, no action (seam kept open) ───────
    CheckCompiled   { manifest: LibraryManifest },   // -> CheckResult { Match | CompiledAhead | RuntimeAhead }

    // ── writes (registry / codegen / migrate) ───────────────────────────
    Publish         { def: SchemaDefinition },       // static-validated; rejects UnresolvedConflict (concern 2)
    PublishLibrary  { spec: LibrarySpec },           // pins a member set -> emits gen crate + LibraryManifest
    Migrate         { target: MigrationTarget, from: u32, to: u32 }, // rejected-whole (concern 7)
}
```

### schema → client (the reply types, homed in `lib/schema::model`)

Every reply rides the `mesh-transport` `ResponseOutcome` switch (success / error /
promise). Reads resolve instantly from the local `schema/` keyspace (KV, concern
6); `Publish`/`PublishLibrary`/`Migrate` are heavier (codegen emission, whole-
population validation) and MAY come back as a `ResponseOutcome::Promise` — the
caller is compiler-forced to handle that arm (INTENT #152 / mesh-transport.md).

```rust
struct SchemaHead   { latest: u32, yanked: Vec<u32> }            // per-id publication head
struct LibraryHead  { latest: u32, yanked: Vec<u32> }            // per-library head

struct LibraryManifest {                                          // the identity a consumer can check
    library_id:      LibraryId,
    library_version: u32,
    members:         Vec<(SchemaId, u32 /*version*/, Hash /*content_hash*/)>,
    gen_crate:       GenCoordinates,                              // path/registry coords of gen/<lib>@<version>
}

struct EffectiveSchema {                                          // flattened DAG — codegen + runtime-validate see THIS
    id:           SchemaId,
    version:      u32,
    content_hash: Hash,                                           // canonical-JSON hash; cross-fleet identity
    fields:       Vec<FieldSchema>,                               // merged + resolved, defaults wired
    types:        Vec<TypeDecl>,
}

enum CheckResult { Match, CompiledAhead, RuntimeAhead }           // OQ-20: report ONLY; no action taken

struct RuntimeDescriptor { json_schema: serde_json::Value, content_hash: Hash } // for non-Rust consumers/validators
```

`SchemaDefinition` (the raw DAG returned by `GetDefinition`), `FieldSchema`,
`DefaultValue`, `ConflictResolution`, `Migration`/`MigrationStep`, `LibrarySpec`,
and `MigrationTarget` are the schema.md concern-1/2/3/4/7 model types — authored
in full there, referenced here as the reply/argument vocabulary (not re-specified).

## Error cases

`SchemaError` (surfaced through the transport switch as
`ResponseOutcome::Error`, matchable, never a panic):

- `UnresolvedConflict { at, among }` — `Publish` of a multi-parent schema whose
  colliding locus carries no `ConflictResolution` (LOCKED #166 Q16 — **no quiet
  overwrite**; publication fails, never a silent merge). The load-bearing error.
- `MalformedSchema { detail }` — types not well-formed, incoherent constraints,
  or a parent `SchemaRef` unknown → rejected **at publish, never at use time**
  (schema.md concern 6; the queues/kg declarative-data posture).
- `ImmutableViolation { id, version }` — attempt to re-`Publish` an already-
  published `(id, version)`; evolution is `version+1`, not mutation.
- `SchemaNotFound { id, version }` / `LibraryNotFound { id, version }` — an
  unknown or yanked coordinate.
- `MigrationRejected { report }` — `Migrate` found ≥1 nonconforming element after
  applying steps → the ENTIRE target stays on `from`, untouched (schema.md concern
  7, rejected-whole; the full violation report rides the error).
- `CyclicInheritance { at }` — a proposed `parents` DAG is not acyclic.

Non-errors by design: `CheckCompiled` NEVER errors on skew — `CompiledAhead` /
`RuntimeAhead` are ordinary results, not failures (the authority-of-record
decision is OQ-20's, deliberately not taken here — see Reconciliation).

## Version sensitivity

**HIGH** — schema definitions and library manifests persist in `replicated-kv`
across a mixed-version fleet and are the substrate every other type rides.

- **`content_hash` is the cross-fleet identity anchor.** A published
  `(id, version)` is IMMUTABLE (schema.md concern 6); its canonical-JSON hash must
  be byte-identical on every node and build — codegen determinism and the OQ-20
  skew check both depend on it. A drift in the hashing recipe is a **breaking,
  fleet-coordinated** change.
- **Additive-safe:** new `SchemaQuery` request variants (`#[serde(other)]`-
  tolerant so an older daemon fails an unknown query loudly-and-locally, never
  crashes); new `#[serde(default)]` fields on reply structs.
- **Breaking:** changing the `EffectiveSchema` flattening/linearization order (it
  fixes the `content_hash`), the `LibraryManifest` `members` triple shape (the
  OQ-20 check compares against it), or the meaning of the three `CheckResult`
  arms.

## Reconciliation notes

- **DC-emergent — NOT a service (INTENT #166 Q11): RESOLVED.** This contract is
  the runtime half of the DC pair; there is no `dc` crate and none is proposed.
  "DC" names *(generated libraries + this WS query surface)*. Recorded so the
  graph is honest: the `schema`↔`chassis` meeting point is the *generated
  artifact* (a codegen dependency), which is why no `schema`↔`chassis` **contract**
  file exists (schema.md P2).
- **OQ-20 seam (types↔schema authority-of-record) — OPEN, kept open.**
  `CheckCompiled` returns a `CheckResult` **report and takes no action**. Whether a
  service **refuses** to run below the registry's version (runtime-as-authority),
  **warns and proceeds** (compiled-as-authority — today's posture), or **triggers a
  Compatibility restart** to pull the new library is a **policy hook left OPEN** —
  a single enum a future round sets. Nothing in this contract presumes the answer;
  the runtime WS surface is purely advisory until the policy is chosen. Marked
  **OPEN** in the contract graph (schema.md concern 10, ledger §B.2/§C).
- **OQ-21 seam (schema-as-universal-messaging, INTENT #139) — OPEN, not applied.**
  This contract carries schema *definitions*; it deliberately does NOT route
  inter-service *messages* through schema. If #139 lands, message payloads would
  reference a `SchemaId@version` resolvable via `GetEffective`/`ResolveDescriptor`
  here — but that substitution is OQ-21's own round, and `mesh-transport`'s
  payload-opaque `Frame` already carries either the hand-authored or the
  schema-inherited payload form with **no wire change** (schema.md concern 11,
  mesh-transport.md § "Schema-inheritance future"). Nothing here presumes it.
- **Struct home is `schema`, not `types` — deliberate.** See Schema §; the
  anti-layer-cycle rule forbids the generated libraries depending on `types`, and
  the query vocabulary is schema's own domain model. This is the one contract in
  the graph whose payload types are homed in an L4 crate rather than `types`, and
  it is intentional, not an oversight.
- **Keeper/artifact type schemas are ordinary parties (schema.md concern 8).** The
  landscape's type system reaches schema over THIS surface and compiles against the
  generated `landscape-core` library; `keeper.md` / `artifacts.md` (batch 6) own
  *what those structures contain*, schema owns only that they are schemas — a seam,
  not a second contract.

## Example data

The example world: nodes **macbook** and **pi**; the built-in `landscape-core`
library at version 7; a keeper-type schema `keeper.project` inheriting
`keeper.core`.

**1. A dashboard resolves a published type for its "every data type" view:**
```jsonc
// service -> local daemon (relayed to the schema owner and back)
{ "type": "GetEffective", "id": "keeper.project", "version": 3 }
// <- ResponseOutcome::Success payload:
{ "id": "keeper.project", "version": 3, "content_hash": "sha256:9f2c…",
  "fields": [ { "name": "bundle_ref", "ty": "Ref(keeper.core@2)", "required": true },
              { "name": "region",     "ty": "String", "required": true } ],
  "types":  [ /* … */ ] }
```

**2. A service checks its compiled `landscape-core` against the live registry (OQ-20):**
```jsonc
{ "type": "CheckCompiled",
  "manifest": { "library_id": "landscape-core", "library_version": 7,
                "members": [ ["keeper.core", 2, "sha256:1a…"], ["keeper.project", 3, "sha256:9f2c…"] ],
                "gen_crate": { "kind": "path", "at": "gen/landscape-core@7" } } }
// <- ResponseOutcome::Success payload:  { "CheckResult": "Match" }
// If the registry had advanced to landscape-core@8 -> { "CheckResult": "RuntimeAhead" }
// (report only; the refuse/warn/compat-restart decision is OQ-20, not taken here)
```

**3. Publishing a multi-parent schema with an unresolved collision fails loudly:**
```jsonc
// -> Publish { def: <keeper.hybrid inheriting keeper.project + keeper.marketing,
//              both defining `region` incompatibly, no ConflictResolution> }
// <- ResponseOutcome::Error:
{ "domain": "schema", "code": "unresolved_conflict",
  "message": "field `region` collides among [keeper.project@3, keeper.marketing@1]; supply a ConflictResolution",
  "details": { "at": "region", "among": ["keeper.project@3", "keeper.marketing@1"] } }
```

**4. A heavy publish that comes back as a promise (INTENT #152):**
```jsonc
// -> PublishLibrary { spec: <pins 40 member schemas -> emits gen/landscape-core@8> }
// <- ResponseOutcome::Promise { promise: "p-3c1", hint: { note: "codegen + manifest emission" } }
// later, pushed back as a PromiseFulfillment frame correlated by "p-3c1":
//   ResponseOutcome::Success { payload: <LibraryManifest for landscape-core@8> }
```

## Provenance

**Authored wave-3 (this harmonizer pass).** This file did not exist through batch
6; `components/schema.md` (batch 4, data/execution-plane reshape) proposed its
shape in "Proposed contracts (wave 3) → P1" and explicitly deferred authoring to
the harmonizer ("I do not create files outside my ownership… to be authored by the
harmonizer / batch-7"). This contract adopts that proposal faithfully. Grounding:
schema.md concerns 4/6/7/8/10/11 + P1; INTENT #150 beat 16 (library concept), #151
+ #166 Q11 (DC emergent, no service), #145 (schema builds Rust types), #169
(self-seeded), #131 (schema rename + inheritance); ledger §A rows 59–62, §B.2
(OQ-20/OQ-21), §C. Struct shapes are schema's to final-tune; this file owns the
edge shape + the DC-emergent / OQ-20 / OQ-21 reconciliations.
