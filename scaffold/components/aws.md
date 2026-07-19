# aws

**Status:** NEW (round-9 lock, 2026-07-19). **Nesting:** top-level.
**Requirements-only stub — NOT a full design.**

> **SUPERSESSION NOTE (round-9):** this crate corrects the earlier "S3
> crate/adapter" phrasing and the round-3-second-batch/rounds-4–5
> "S3 adapter lives inside mesh" placement. Operator, verbatim: "I
> definitely meant AWS crate." **The S3/AWS adapter surface lives HERE**;
> `mesh`, `vfs`, `kg`, and `secrets` CONSUME it (each carries its own
> supersession note). The S3-overflow requirements themselves are unchanged
> — only the owner moved (again).

## Charter (requirements, operator's words where quoted)

The **AWS virtualization-layer crate** — "for common AWS tasks and our AWS
virtualization layer... when each of our Mind OS services wants to do
something in AWS, there's a specific set of AWS interfaces all stored within
our AWS crate."

- **The aggregation point for ALL AWS adapters.** Every AWS-facing surface
  in Mind OS lives here — "a place to aggregate our adapters":
  - **S3 overflow** (moved from mesh, round-9) — the overflow tier when the
    personal mesh runs out of space; S3 cold-storage classes; client-side
    encryption before upload (keys held by `secrets`);
  - **RDS + Lambda** — the AWS VDB target (see `components/stack.md`:
    VDB's three adapter targets are local SQLite / Supabase / AWS
    RDS+Lambda; with the round-9 SQLite-locally lock, AWS is strictly a
    CLOUD promotion target);
  - **AWS Secrets Manager push** — `secrets`' AWS adapter (design-only in
    v1: data contract designed, lives here, NOT built now — see
    `components/secrets.md`);
  - **SQS someday** — mesh's SQS-modeled queues deploy someday straight
    onto real SQS through this crate (see `components/mesh.md` concern 14).
- **A WebSocket interface** for **pushing to, reading from, and pulling
  against AWS** "in exactly the ways we need" — the interfaces are shaped by
  what Mind OS services actually need, not by AWS's API surface.
- **Explicitly NOT a boto3 replacement or a Terraform replacement.** No
  general-purpose AWS SDK ambitions, no infrastructure-as-code ambitions —
  only the specific adapter surfaces Mind OS services consume.
- **Containers-in-AWS context (rounds 4–5, standing):** the hard no-Docker
  rule is local-only — "If we do have containers it's going to be in the
  AWS environment and we'll have an AWS adapter to handle all of that."
  That AWS adapter surface is this crate's territory when it becomes real.

## Relationships / edges (stubs only)

- **mesh** via `aws-mesh` — **NEW round-9 (requirements-only).** aws
  registers/resolves like any service (single-port locality); mesh's
  eventual-consistency/replication plane distributes into S3/AWS through
  aws's adapter surface (the round-9 supersession of mesh's internal S3
  adapter) (scaffold/contracts/aws-mesh.md).
- **vfs** via `aws-vfs` — **NEW round-9 (requirements-only).** The S3
  overflow tier: VFS overflow placement reaches S3 (cold-storage classes,
  client-side encryption) through aws (scaffold/contracts/aws-vfs.md).
- **kg** (anticipated, rides mesh) — KG's cross-boundary S3 sync flows
  mesh→aws; no direct contract stub (see `components/kg.md`).
- **secrets** (anticipated) — the AWS Secrets Manager push adapter's data
  contract is designed against this crate; not built now (see
  `components/secrets.md`). Edge naming deferred.
- **stack/VDB** (anticipated) — the AWS (RDS + Lambda) VDB target reaches
  AWS through this crate when built; edge naming deferred to VDB's design
  pass.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
Locked round-9: aws is the aggregation point for ALL AWS adapters (S3
overflow, RDS+Lambda VDB target, Secrets Manager push, SQS someday), with a
WebSocket push/read/pull interface, explicitly not a boto3/Terraform
replacement; the S3-surface-inside-mesh framing is superseded in its favor.
Open: everything else (internal layering, the WebSocket protocol shape,
auth/credentials handling — presumably via `secrets` — and build order).
