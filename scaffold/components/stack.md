# stack

**Status:** NEW (rounds 4–5 lock, 2026-07-18). **Nesting:** top-level.
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
  OPEN** — and constrained by the no-Docker rule: where that Postgres comes
  from (a native local Postgres process vs. a cloud target) is undecided.
- **Relationship to `db` (working framing):** `db` = the
  control-plane / virtualization / query interface over databases; `stack` =
  the runtime that HOSTS a database. A stack-hosted SQLite file is one of the
  backends `db`'s now-in-scope query/virtualization layer standardizes over.
  See `components/db.md`.

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
Open: the SQLite→Postgres upgrade mechanism (no-Docker-constrained).
