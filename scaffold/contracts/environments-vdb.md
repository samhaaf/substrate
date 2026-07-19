# Contract: environments-vdb  [STUB-TRACK]

## Parties
`environments` (L6 stub-track, "not implementing now") ↔ `vdb`. Over the local
mesh daemon `:3649`, never linked. Authored from `environments.md` (its
`environments-vdb` stub) and `vdb.md`'s stub-track note — which **agree**.
Content is deferred until `environments` leaves the stub track; this file pins
the anticipated shape only (no full example world, per the stub-track format).

## Purpose
An **environment routes a database's storage target**: **local environment →
SQLite; cloud environment → promote** (INTENT #97/#98). Today (v1), a stack
database's target is set by `vdb`'s explicit per-database
`EnsureDatabase.target_hint`. When `environments` lands, that hint is
**replaced by environment-derived routing** keyed on the shared `EnvRef`
identity — the environment says which target class a database anchored to it
should use, and a promotion into a cloud environment triggers `vdb`'s
copy/verify/switch mechanics.

## Schema (rough sketch — deferred)
`EnvRef` is the shared identity (must match `environments.md`, `repo.md`, and
`secrets.md`'s `SecretScope::Environment`); `StackTargetKind`/`DatabaseId` are
`types::vdb`.

```rust
pub struct EnvRef { pub project: String, pub env: String }   // the shared (project, env) identity

// anticipated: environments -> vdb, resolve a database's target from its environment
struct ResolveTarget { db_id: DatabaseId, env: EnvRef }
                     // -> StackTargetKind   (Local environment -> LocalSqlite; Cloud -> a cloud target)
// anticipated: environment-driven promotion (deploy-into-cloud-env ACTIVATES promotion)
struct RouteOnDeploy { db_id: DatabaseId, env: EnvRef, role: EnvRole /*Green|Blue|Stage*/ }
                     // vdb runs its existing PromoteDb path when routing crosses local -> cloud
```

**No mechanism beyond `vdb`'s existing verbs** — `environments-vdb` supplies
only the *routing decision*; `EnsureDatabase.target_hint` and `PromoteDb`
(the real machinery, already designed in `vdb.md`) execute it. The v1
per-database `target_hint` is the placeholder this edge later replaces.

## Open questions
- **Local-purpose KG can support a cloud environment (kept OPEN, INTENT #97;
  kg.md).** `residence` and `environment` are independent fields — a
  Local-residence graph/database may serve a cloud environment and vice versa;
  the environment supplies only the *default* target at creation, never an
  invariant. This edge must not over-constrain that.
- **Nested-project environment inheritance (INTENT #63, naive answer):**
  sub-projects share the parent's environment by default; whether that implies
  shared database routing is deferred to environments' own design.
- **Does `RouteOnDeploy` ride `DeployedEvent` (pubsub) instead of a direct
  call?** `environments.md` has repo firing a `DeployedEvent` that
  environments/cicd subscribe to; the vdb-routing leg may hang off that event
  rather than being a request. Deferred to environments' implementation.
- **Blue/green promotion coupling:** how `EnvRole` transitions (green↔blue)
  map onto `vdb` promotion phases when a database (not just a web service) is
  the deploy target — deferred.

## Reconciliation notes
- **Both parties agree; stub-track, no disagreement.** `environments.md` and
  `vdb.md` independently describe the same "v1 `target_hint` placeholder →
  future `EnvRef`-derived routing" shape. Recorded verbatim.
- **Deviation:** none — this pair had no prior stub file (MISSING per wave2-plan
  §3c); authored fresh at stub depth. Full schema + example data land when
  `environments` is implemented.
