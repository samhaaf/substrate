# stack

**Status:** NEW (rounds 4–5 lock, 2026-07-18; round-7 update, 2026-07-19).
**Nesting:** top-level.
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
  stack-vs-db section below). Still open, and still constrained by the
  no-Docker rule: where that Postgres comes from (see the Postgres-ambiguity
  section below).
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
- **Relationship to `db` (working framing — now explicitly the operator's
  most-nebulous OPEN question; see the section below):** `db` = the
  control-plane / virtualization / query interface over databases; `stack` =
  the runtime that HOSTS a database. A stack-hosted SQLite file is one of the
  backends `db`'s now-in-scope query/virtualization layer standardizes over.
  See `components/db.md`.

## The stack-vs-db boundary — THE operator's most-nebulous OPEN question (round-6; do NOT force it)

Recorded verbatim-grade, deliberately NOT resolved. Operator: stack "is a
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

Two OPEN questions attached, verbatim-grade: (a) **how the stack pattern
gets "baked into" VDB**, and (b) **whether KG should be BUILT ON VDB** ("is
the knowledge graph just a special version of VDB, living as a distributed
service via the mesh? I actually think that's a worthwhile question") — see
`components/kg.md`. `db`'s existing Capabilities-gated driver architecture
(supabase-cloud / supabase-local / sqlite) is the natural seed of VDB's
adapter matrix — see `components/db.md`. The stack-vs-db boundary question
above stays OPEN; VDB's elevation names the implementation vehicle, it does
not resolve the boundary.

## Postgres ambiguity (round-6 — flagged, UNRECONCILED)

In response to the native-vs-cloud Postgres question, the operator floated:
"I think that's the line — it standardizes Postgres and we can just have one
Postgres Docker machine, multiple databases in the same Postgres Docker. The
only downside is on a small device like a Raspberry Pi I'd rather be running
SQLite or even directly installing Postgres. So I'm not sure." **"One
Postgres Docker" sits in direct tension with the emphatic HARD no-Docker
rule above — flagged, not reconciled; needs explicit operator
reconciliation before any design leans on it.** Related, the operator's own
counter-question: "it's worth talking about why we would ever want to
[upgrade SQLite → Postgres] if we're able to get our full stack working on
top of SQLite."

## REQUIRED design analysis: SQLite sufficiency (round-7, 2026-07-19 — GATES the local-Postgres decision)

The operator will NOT decide the local-Postgres question until a
**daemon-level gap analysis** shows whether **SQLite + the stack daemon +
mesh (cron, pub/sub)** functionally covers everything Postgres would
provide. Operator, verbatim: "I still can't answer until we talk about why
we might never need Postgres — specifically if we can do everything in
SQLite. What could we do at the daemon level that would give us everything
we need functionally for our working stack pattern? Like pg_cron — we can
create a cron handler inside of mesh that operates on the database; triggers
on insert, update, delete..."

The analysis: enumerate what Postgres provides, and show which daemon-level
equivalent covers each — e.g. **pg_cron → mesh cron; LISTEN/NOTIFY → mesh
pub/sub; procedural triggers → daemon-level Deno/SQL handlers**; etc. This
is recorded as a **REQUIRED design analysis gating that decision**. AUI is
delivering a first-pass analysis conversationally, but the definitive
version belongs in stack/VDB's full component design.

## Relationships / edges (stubs only)

- **vfs** via `stack-vfs` — the single SQLite file each stack daemon wraps is
  stored in (and read/written through) the VFS
  (scaffold/contracts/stack-vfs.md).
- **mesh** via `stack-mesh` — registration in the service registry via the
  local mesh daemon (single-port locality), plus distributed-handler
  coordination via mesh's `locks` lib (scaffold/contracts/stack-mesh.md).
- **shared handler/execution engine** — internal library dependency
  (stack-tables adapter), deliberately NOT a contract stub (see `overview.md`
  shared-libraries section).
- **db** (framing, not a contract yet) — `db`'s query/virtualization layer
  reaches stack-hosted SQLite databases; edge naming deferred until that
  layer is designed.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
Open (round-7 revision): the **stack-vs-db boundary** (the operator's
most-nebulous open question — **VDB now elevated to the deploy-anywhere
implementation vehicle**, with its two attached open questions:
stack-baked-into-VDB, KG-on-VDB), the **Postgres source / Docker tension**
(unreconciled), and whether SQLite→Postgres upgrade is even needed — now
explicitly **gated on the REQUIRED SQLite-sufficiency analysis** (section
above); the migration *mechanics* themselves are confirmed
(copy/verify/switch, let-edge-functions-finish).
