# Contract: aws-vdb

## Parties
`vdb` (L4 daemon) `->` `aws` (L2 app-crate), over the local mesh daemon
`:3649` (single-port locality; cross-app, never linked — INTENT #29).

*(Authored at wave-2 harmonization. This pair was named in wave2-plan §3b but
the contract round produced no file — a coverage gap. Content is the
already-reconciled position: `aws.md`'s proposed surface, ACCEPTED verbatim by
`vdb.md` ("aws.md's half ACCEPTED (design-only v1)") with two vdb-side notes.
Minimal-stub discipline: the full sketches live in `components/aws.md`
§`aws-vdb` and `components/vdb.md` §`aws-vdb`.)*

## Purpose
VDB's **cloud deploy target** — the RDS+Lambda third adapter after
local-SQLite and Supabase (INTENT #93/#96). Reached **only** on a local→cloud
*promotion* (INTENT #98 routing: local→SQLite, cloud→promote), never for
local work. **DESIGN-ONLY in v1** (INTENT #105): every call answers
`AwsDisabled` until built; vdb treats that as promotion-unavailable.

## Schema (summary — authoritative sketch in aws.md)
`AwsVdbRequest { ProvisionTarget{RdsSpec} | DescribeTarget | DeployFunction{LambdaSpec}
| InvokeFunction | Migrate{MigrationPlan} | Teardown }`;
`RdsEngine::Postgres` only (no local pg ever — cloud RDS is the only
Postgres); `LambdaRuntime::{Deno, Sql}` (Deno = the confirmed TS runtime,
INTENT #65). Wire structs land in `types::aws` when built.

**Load-bearing boundary:** the target's **connection secret** is held and
brokered by `secrets` via `vdb-secrets`, **not** returned by `aws`. `aws`
returns the *endpoint*; `secrets` holds the *credential* — the
use-without-seeing split.

## Error cases
`AwsDisabled { adapter, op }` (v1 default); `ProvisionFailed`;
`TargetNotFound`; `DeployFailed`; `InvokeFailed`; `MigrationConflict`
(copy/verify mismatch — the promotion aborts, local DB untouched);
`CredentialsUnavailable`.

## Version sensitivity
Entirely design-only in v1 — the whole surface is proposed-not-frozen;
additive growth expected when VDB's cloud path is actually built.

## Reconciliation notes
- `vdb.md` accepted `aws.md`'s proposal with two notes, both adopted:
  (1) vdb's promotion state machine drives `Migrate` as steps 2–4 only — the
  **barrier, delta, and registry flip (steps 5–7) remain vdb's**, executed
  through db sessions against the RDS endpoint; `MigrationPlan.verify` maps
  onto `verify_against`'s report so "MigrationConflict → abort, local
  untouched" is one shared semantics. (2) `DeployFunction.code_ref: ObjKey` —
  vdb supplies the VFS content hash re-materialized into the aws ObjKey
  namespace (handler sources are content-addressed; the mapping is
  mechanical).
- Promotions treat the target DB as a service under mesh's restart/upgrade
  protocol (INTENT #86) — coupled to `restart-protocol`.
