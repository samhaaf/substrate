# stack

> **SUPERSEDED-PENDING-HARMONIZER (wave 2, batch 4, 2026-07-19).** The full
> component design of this module now lives in **`components/vdb.md`** (the
> round-8 lock named the module `vdb`; "stack" survives as the PATTERN name
> only — see vdb.md's naming-resolution note). All requirements below are
> carried into vdb.md's concerns; this file is retained as verbatim
> requirements HISTORY and is not edited further. Contract-stub renames
> flagged there (NOT performed): `stack-vfs` → `vdb-vfs`, `stack-mesh` →
> `vdb-mesh`. Do not extend this file; extend vdb.md.

**Status:** NEW (rounds 4–5 lock, 2026-07-18; round-7 update, 2026-07-19;
**round-8 lock, 2026-07-19 — the stack-vs-db boundary is RESOLVED**;
**round-9 lock, 2026-07-19 — SQLite-locally RESOLVES the Postgres/Docker
ambiguity**). **Nesting:** top-level.
**Stub-plus — requirements captured from the operator; NOT a full design.**

## Charter (requirements, operator's words where quoted)

The **lightweight database-with-handlers runtime**: "a daemon around SQLite
that allows our DB runtime to run on a single file" — a daemon wrapped around
a **single SQLite file, stored in the VFS**, with **SQL handlers and
Deno/TypeScript handlers** firing on row/table changes. This is the
operator's database-centric backend paradigm made first-class, without any
Docker/Supabase-local weight:

> "The way I build now is database-centric. I start with database tables
> representing all of my intermediate data structures, and I use handlers to
> process changes to certain rows and do transformations and drop them into
> other tables or modify columns. Those handlers are either SQL or code — I'm
> leaning towards TypeScript for the code-based handlers. I've almost
> entirely eliminated my need for FastAPI... I want to be able to use that
> same coding paradigm [everywhere] — I have the required engine in each
> location and adapters to deploy this thing in that location."

Requirements:

- **A daemon around a single SQLite file** (the file lives in the VFS) —
  tables as intermediate data structures, handlers doing the
  transformations. The backend web stack without the Supabase Docker weight.
- **Handlers: SQL and Deno/TypeScript.** **Deno is the CONFIRMED TS
  runtime** (rounds 4–5 blanket yes). Handlers fire on row/table changes.
- **Handler execution is the SHARED engine, not stack-private.** The
  handler/execution engine is ONE shared library (name TBD) with adapters
  for stack-tables and kg-nodes, carrying the guardrails (causal chain
  tracking, loop detection with a loop-depth threshold, escalation hook) —
  see `overview.md`'s shared-libraries section. An internal library
  dependency, NOT a contract edge.
- **HARD RULE: NO DOCKER, locally, ever.** Verbatim: "I literally just don't
  want Docker as a part of this at all. I'm not trying to build a Kubernetes
  network that runs containers... Docker crashes all the time... I don't want
  to turn my computer into a runtime for containers. If we do have containers
  it's going to be in the AWS environment and we'll have an AWS adapter to
  handle all of that." Containers exist ONLY in the AWS environment, via an
  AWS adapter.
- **Graceful auto-upgrade SQLite → Postgres.** If a database attached to a
  project/environment needs something SQLite can't do, it "gracefully,
  automatically upgrades from SQLite to a Postgres database." **Mechanism
  PARTIALLY RESOLVED round-6 (supersedes "Mechanism OPEN"):** the migration
  mechanics are confirmed — **copy/verify/switch under a lock** ("exactly
  right"); during a critical upgrade, **let running edge functions finish
  against the old DB, swap the database underneath, write results back,
  resume triggering**. Databases get treated like services under mesh's
  restart/upgrade protocol (via db-as-daemon or the VDB idea — see the
  stack-vs-db section below). **Where that Postgres comes from is RESOLVED
  round-9 (2026-07-19): there is NO local Postgres, native or Docker —
  the upgrade target is always a CLOUD VDB target (Supabase or AWS
  RDS+Lambda)**; upgrading past SQLite means promoting to the cloud (see
  the resolved Postgres-ambiguity section below).
- **Provenance is FIRST-ORDER (round-6 cross-cutting principle; SCOPED
  round-7).** Every handler touch of data is traced from the very
  beginning — healthcare-data-engineer-grade provenance; see `overview.md`'s
  standing principle and the shared execution engine's causal-chain
  tracking. **Round-7 scoping: provenance is configured
  per-project/per-database, and VDB is its primary home** ("probably a VDB
  thing. Also a VFS thing, but I typically don't care about provenance in
  the file system — usually only in the database and the stack pattern");
  VFS carries only a lighter provenance design requirement (see
  `components/vfs.md`).
- **Relationship to `db` — RESOLVED round-8 (supersedes the "most-nebulous
  OPEN question" framing; see the section below):** the locked decomposition
  is **VDB = the daemon that tracks and executes the stack pattern; `db` =
  the crate VDB uses to run actions against specific databases (extended as
  necessary to support VDB); KG built on top of VDB; VFS below VDB**. Layer
  order: **VFS < VDB < KG**. See `components/db.md`.

## The stack-vs-db boundary — RESOLVED round-8 (was THE operator's most-nebulous OPEN question, round-6)

> **RESOLVED (round-8, 2026-07-19) — SUPERSEDES the round-6 "do NOT force
> it" OPEN status.** The resolving decomposition, operator verbatim: **"VDB
> becomes the daemon which tracks the execution, and it takes advantage of
> the db crate to actually run the actions against specific databases — we
> extend db as necessary to support VDB. And KG we build on top of VDB."**
> Layering fact: VDB sits ON VFS (the SQLite files must live somewhere), so
> the locked layer order is **VFS < VDB < KG**. Neither round-6 option won
> outright: stack is NOT folded into `db`, and `db` is not left untouched —
> `db` stays the boring action-runner crate UNDER the VDB daemon, extended
> as needed to support it. Both round-7 attached open questions close with
> this: (a) the stack pattern is "baked into" VDB by VDB **being** the
> daemon that tracks/executes it, and (b) **KG IS built on top of VDB**
> (see `components/kg.md`). The sections below are preserved as history of
> how the question stood before the lock.

**History (round-6 capture — superseded by the resolution above):**
recorded verbatim-grade, at the time deliberately NOT resolved. Operator:
stack "is a
pattern — database-driven triggers and handlers that can do database updates
and call third-party systems. I don't know how many places it's going to show
up or how much can be standardized. A lot of it's already in db (db already
deploys edge functions to Supabase). There's a strong case we put it in db
and use db to run the knowledge graph — but then db also has to do eventual
consistency across multiple databases, and we're really pushing db. Or we
leave db boring as a utility used within the system that does eventual
consistency across databases in the mesh. I don't know. Open question."

Options on the table:

1. **Fold stack into db** — and run the knowledge graph through db too,
   pushing db hard (db stops being boring).
2. **Keep db boring** as a utility used within a bigger
   eventual-consistency system in the mesh.

**VDB (operator-coined round-6 — ELEVATED round-7, 2026-07-19):** originally
a **virtualized-database SERVICE** idea that uses `db` under the hood,
treating **databases like services under the same mesh restart/upgrade
protocol** (`mesh.md` concerns 12–13). The confirmed migration mechanics fold
in here: **copy/verify/switch under a lock**, and during a critical upgrade
**let running edge functions finish against the old DB, swap the database
underneath, write results back, resume triggering**. **Round-7: VDB is now
the working name for the deploy-anywhere IMPLEMENTATION of the stack
pattern** — one abstract runtime with **three adapter targets**: **local**
(the SQLite stack daemon), **Supabase**, and **AWS (RDS + Lambda)**.
Operator, verbatim:

> "Ideally VDB perfectly implements our stack, and maps onto Supabase, maps
> onto AWS via RDS + Lambda, or maps onto local with a stack [daemon]. Worth
> talking about: how do we have our stack baked into VDB? And are there
> reusable components for the knowledge graph — is the knowledge graph just
> a special version of VDB, living as a distributed service via the mesh?
> Should the knowledge graph be built on top of VDB? I actually think that's
> a worthwhile question."

Two OPEN questions were attached round-7, verbatim-grade: (a) **how the
stack pattern gets "baked into" VDB**, and (b) **whether KG should be BUILT
ON VDB** ("is the knowledge graph just a special version of VDB, living as
a distributed service via the mesh? I actually think that's a worthwhile
question") — see `components/kg.md`. `db`'s existing Capabilities-gated
driver architecture (supabase-cloud / supabase-local / sqlite) is the
natural seed of VDB's adapter matrix — see `components/db.md`.
**Round-8: BOTH attached questions are ANSWERED and the boundary is
RESOLVED** (see the supersession note at the top of this section): VDB is
the daemon tracking/executing the stack pattern, `db` is the crate it uses
to run actions (extended as needed), KG builds on top of VDB, VFS sits
below (VFS < VDB < KG).

## Postgres ambiguity (round-6 flagged — RESOLVED round-9, 2026-07-19)

> **RESOLVED (round-9, 2026-07-19): the SQLite-locally rule is LOCKED.
> Local environments run SQLite, PERIOD. Postgres exists ONLY as a cloud
> VDB target (Supabase / AWS RDS+Lambda). There is NO local Postgres —
> not native, not Docker.** This resolves the flagged "one Postgres
> Docker" tension below in favor of the hard no-Docker rule and the
> operator's own round-8 yes-block routing rule ("local environment →
> SQLite, cloud → promote"). The "one Postgres Docker machine" float is
> DEAD — kept below only as history.

History (round-6 capture — superseded by the resolution above): in response
to the native-vs-cloud Postgres question, the operator floated:
"I think that's the line — it standardizes Postgres and we can just have one
Postgres Docker machine, multiple databases in the same Postgres Docker. The
only downside is on a small device like a Raspberry Pi I'd rather be running
SQLite or even directly installing Postgres. So I'm not sure." "One
Postgres Docker" sat in direct tension with the emphatic HARD no-Docker
rule above — flagged unreconciled rounds 6–8, reconciled round-9 as above.
Related, the operator's own counter-question: "it's worth talking about why
we would ever want to [upgrade SQLite → Postgres] if we're able to get our
full stack working on top of SQLite" — answered by the same lock: locally
we never do; going past SQLite means promoting to a cloud target.

## REQUIRED design analysis: SQLite sufficiency (round-7; DE-GATED round-9)

Round-7 recorded this analysis as GATING the local-Postgres decision.
**Round-9's SQLite-locally lock makes the decision without it** (no local
Postgres, ever), so the analysis **no longer gates a decision** — but it is
KEPT as a required design input for the stack/VDB design pass, because with
SQLite locked as the only local engine, the daemon-level equivalents are
now load-bearing: the local stack MUST cover functionally what Postgres
would have provided. Operator, verbatim (round-7): "I still can't answer
until we talk about why we might never need Postgres — specifically if we
can do everything in SQLite. What could we do at the daemon level that
would give us everything we need functionally for our working stack
pattern? Like pg_cron — we can create a cron handler inside of mesh that
operates on the database; triggers on insert, update, delete..."

The analysis: enumerate what Postgres provides, and show which daemon-level
equivalent covers each — e.g. **pg_cron → mesh cron; LISTEN/NOTIFY → mesh
pub/sub; procedural triggers → daemon-level Deno/SQL handlers**; etc. AUI
delivered a first-pass analysis conversationally; the definitive
version belongs in stack/VDB's full component design.

## Relationships / edges (stubs only)

- **vfs** via `stack-vfs` — the single SQLite file each stack daemon wraps is
  stored in (and read/written through) the VFS
  (scaffold/contracts/stack-vfs.md). **Round-8: this is the locked layering
  fact — VFS sits BELOW VDB** (the SQLite files must live somewhere);
  VFS < VDB < KG.
- **mesh** via `stack-mesh` — registration in the service registry via the
  local mesh daemon (single-port locality), plus distributed-handler
  coordination via mesh's `locks` lib (scaffold/contracts/stack-mesh.md).
- **shared handler/execution engine** — internal library dependency
  (stack-tables adapter), deliberately NOT a contract stub (see `overview.md`
  shared-libraries section).
- **db** (not a contract yet; **layering LOCKED round-8**) — the VDB daemon
  USES the `db` crate to run actions against specific databases (`db`
  extended as necessary to support VDB); `db`'s query/virtualization layer
  reaches stack/VDB-hosted SQLite databases. Edge/dependency naming
  deferred until VDB's design pass.
- **kg** (not a contract yet; **layering LOCKED round-8**) — KG is built ON
  TOP of VDB (see `components/kg.md`); layer order VFS < VDB < KG.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
**RESOLVED round-8:** the stack-vs-db boundary (the former most-nebulous
open question) — **VDB is the daemon tracking/executing the stack pattern,
using `db` to run actions against specific databases (db extended as
needed); KG builds on top of VDB; VFS sits below VDB (VFS < VDB < KG)**;
both round-7 attached questions (stack-baked-into-VDB, KG-on-VDB) closed
with it. **RESOLVED round-9:** the Postgres source / Docker tension —
**SQLite-locally LOCKED: local environments run SQLite, period; Postgres
only as a cloud VDB target; no local Postgres, native or Docker.** The
SQLite-sufficiency analysis no longer gates a decision but is kept as a
required design input (section above); the migration *mechanics* remain
confirmed (copy/verify/switch, let-edge-functions-finish — now always a
local→cloud promotion).
