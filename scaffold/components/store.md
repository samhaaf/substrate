# store

**Status:** EXISTING internal-lib of `inference` (`lib/store` = `substrate-store`),
stable real code. **FULL DESIGN — wave 2, batch 5 (L5 inference core).** This pass
is a **refit, not a rewrite**: the wave-1 file (charter + single-writer discipline
+ the two flagged deltas) holds and is preserved; wave 2 (a) **resolves** the
`db-inference-init` schema-ownership tension against the batch-4 `db.md` design
(now settled, no longer a naked flag), (b) makes the **StoreObserver
under-mutex reentrancy hazard a stated first-class contract** rather than a code
comment, (c) has store **adopt `types::Provenance`** as the completion-origin
vocabulary (INTENT #85, "provenance where data is touched"), and (d) adds the
**pressure-covariate columns** the multidimensional kernel needs (INTENT #12).
Grounded in the real source (`lib/store/src/{lib,completions,collections,models,
results,benchmarks,kv_cache}.rs`, `schema.sql`), the `PromiseRegistry`
`StoreObserver` impl in `lib/inference/src/lib.rs`, and the neighbor designs
`db.md` (batch 4 — the ledger-only `db-inference-init` baseline), `types.md`
(`Provenance`, the `error/` restructuring, `SystemState`), `benchmark.md` +
`telemetry.md` (the pressure-covariate requirement), and `inference.md`
(`InferenceService::start` wiring, the store↔db standalone/mesh boundary).

## Charter

`store` is the **per-machine SQLite system of record** for one `inference` node:
the durable home of completions, collections, models, result blobs, benchmark
runs, and KV-cache metadata (`lib/store`, modules `completions`, `collections`,
`models`, `results`, `benchmarks`, `kv_cache`). Every access is serialized
through a single `Arc<Mutex<Connection>>` (`lib.rs:92`); the schema is embedded at
compile time (`SCHEMA_SQL = include_str!("../schema.sql")`, `lib.rs:58`) and
applied append-only on `Store::open` (`apply_schema`, `lib.rs:150`). It is the
**intra-node hub** — nearly every inference sibling reads/writes through it — and
it carries the `StoreObserver` seam (`lib.rs:71`) so terminal state transitions
fire callbacks synchronously inside the writer mutex (inference registers a
`PromiseRegistry` observer to fulfill in-process `Promise`s; the KG service is the
named future second implementer, `lib.rs:26`).

**Boundary — what store does NOT own.** It is deliberately distinct from two
other SQLite-touching modules and must not absorb either: it is **not `db`** (the
boring control-plane action-runner with its `ops` migration ledger, drivers, and
healthcare-grade provenance ledger — `db.md`), and it is **not `vdb`** (the stack
daemon that tracks trigger/handler execution). Store owns per-node inference
persistence and the observer seam only — no control-plane schema, no cross-node
state, no eventual consistency, no business logic (callers own that), no wire
surface. It is a **compiled-in library of `inference` (INTENT #22/#29)**, reached
in-process by its siblings — it never registers with mesh, never speaks the
pub/sub or restart protocols, and is never linked across an app boundary; the
`api` layer owns the `/v1/` surface and mesh registration on store's behalf. Its
results durably **outlive on-disk weights**, which is exactly what makes per-node
`gc` reclamation safe.

## Primary design concerns

### 1. Single-writer discipline — the concurrency contract that everything else rides

The `Arc<Mutex<Connection>>` (`lib.rs:92`) is store's whole concurrency model:
`conn()` (`lib.rs:142`) hands out a `MutexGuard`, and **every mutation and query
serializes behind it**. There is intentionally no async I/O — SQLite ops are fast
and bounded (`lib.rs:5`), and WAL mode (`schema.sql:5`) gives concurrent readers
against the single writer at the file level. This is a load-bearing invariant,
not an incidental choice, because the observer seam fires *inside* this held lock
(concern 2), and because it is the local sibling of `db`'s own
"mesh-managed-SQLite is single-writer" rule (`db.md` concern 1) — same physics,
enforced here by the `std::sync::Mutex` rather than a daemon. **No caller may hold
the guard across an `.await`** (it is a std, non-async mutex); every store method
takes and drops it within one synchronous body, which the whole codebase already
respects.

### 2. The StoreObserver under-mutex reentrancy hazard — now a STATED CONTRACT (the wave-2 hardening)

Wave-1 treated this as a code comment inside `PromiseRegistry`; wave 2 promotes
it to a **first-class, documented invariant** in `store-access`, because the KG
service is slated to be the second `StoreObserver` implementer (`lib.rs:26`) and
a naive implementation will **deadlock the node's entire persistence layer**.

The hazard, precisely (grounded in `completions.rs:136`–`148`, `mark_running`):
the observer callback fires **while the `conn()` `MutexGuard` is still in scope**
— `let conn = self.conn()?; … conn.execute(…)?; if let Some(obs) = &self.observer
{ obs.on_completion_state_changed(…) }` — the guard is not dropped until the
method returns, so the observer runs **synchronously, on the writer thread, with
the store mutex HELD.** Therefore an observer implementer MUST obey four rules:

1. **Never re-enter `Store`.** Any `Store` method that calls `conn()` will
   deadlock (the `std::sync::Mutex` is non-reentrant). `PromiseRegistry` needs
   `get_result` (which calls `conn()`, `results.rs:51`) — so it **defers that
   read to `tokio::spawn`** (`lib/inference/src/lib.rs:139`), which runs *after*
   the guard is dropped. This deferral is mandatory for any observer that reads
   store.
2. **Never block.** Blocking the callback stalls the single writer and therefore
   *all* node persistence. The observer touches only its own in-memory state.
3. **Lock its own state non-blockingly.** `PromiseRegistry` does
   `self.senders.try_lock()` (`lib.rs:121`) rather than a blocking/`.await` lock,
   because `register()` holds that same async lock across an `.await`
   (`lib.rs:96`); under contention the observer **logs and skips** — delivery is
   **best-effort by design**, not guaranteed. A KG observer must likewise offload,
   never await inside the callback.
4. **Never panic.** A panic unwinds through the writer while the mutex is held,
   poisoning it (`conn()` maps a poisoned lock to `SubstrateError::Store`,
   `lib.rs:145`) and bricking the node's store.

This is the interruptibility story at the store layer: the observer seam is a
*notification* channel, not a *transaction participant*. Stating the try_lock +
spawn + best-effort pattern as a contract is the single most important wave-2
deliverable here, because it is the one place a downstream Filler (KG) can
silently break every node.

### 3. Schema-ownership vs `db-inference-init` — RESOLVED against `db.md` (was a flag, now settled)

Wave-1 flagged the tension: the `db-inference-init` edge says a fresh node
"initializes its own database through `db`," yet store already self-applies its
embedded schema at `Store::open` with **zero external dependency** — the property
that lets a single box run standalone. The batch-4 `db.md` design **resolves this
cleanly, and store confirms the resolution rather than reopening it:**

- They are **two distinct SQLite databases on the node.** `db-inference-init`
  bootstraps the **control-plane `ops` database** (the migration ledger + handler
  registry + outbox + audit + provenance ledger), via the **`sqlite` driver,
  ledger-only baseline `OPS_BASELINE_SQLITE` + `migration::apply`**, reached as a
  **`bin/db` CLI subprocess** — the boot-safe, mesh-free path (`db.md`
  `db-inference-init`). `db.md` states explicitly that `db` "does not own `store`
  … a separate database, self-migrating."
- **`store` keeps its embedded self-migration verbatim** (`apply_schema`,
  `SCHEMA_SQL`) as the **inference system-of-record** path. It is untouched by
  `db-inference-init`; there is no schema handoff, no `db` dependency in
  `Store::open`, and single-box inference stays dependency-free (INTENT #23's
  "degrade gracefully to standalone").

So the settled recommendation stands and is now **cross-consistent with `db.md`**:
store owns its local schema; `db` owns the control-plane `ops` schema; the two do
not overlap. This is recorded here as *resolved*; the only residual is the
harmonizer noting the two databases live in separate files under the node's data
dir (e.g. `store.db` vs the `ops`-baseline db) so nothing co-opens them.

### 4. Provenance adoption — store records completion ORIGIN + causation via `types::Provenance` (INTENT #85, scoped by #92)

INTENT #85 makes provenance first-order "from the very beginning … every time
data gets touched." A completion insert/transition **is** a data touch, so store
must participate — but scoped honestly by INTENT #92: healthcare-grade,
per-touch, atomic-ledger provenance is **VDB's** primary home (the stack pattern
/ database), and store is the inference system-of-record, not a stack database.
Store's right-sized role: **record where each completion came from and what
caused it**, adopting the shared `types::Provenance` vocabulary so the causal
identity is the *same* one that flows through mesh envelopes and `db`/VDB traces
(one vocabulary, many consumers — `types.md` `provenance.rs`).

Design:

- **Adopt `types::Provenance`** (`origin_node`, `origin_service`, `emitted_at`,
  `causation_id`, `correlation_id`; the relay `hops` vec is a mesh concern and is
  NOT persisted at store). Store persists the **scalar identity fields as
  append-only columns on `completions`** and reconstructs a `Provenance` for the
  observer/api on read. `insert_completion` (`completions.rs:93`) gains a
  `&Provenance` argument that the `api` layer stamps from the inbound `/v1`
  request envelope (which already carries `Provenance` per `types.md` `pubsub.rs`).
  A cc `llm-calls` completion thus records `origin_service = "cc"` +
  `correlation_id` = the agent session's chain root; a benchmark completion
  records `origin_service = "benchmark"`.
- **Atomic by construction.** Unlike `db` (which needs a *separate* `ops.provenance`
  ledger written in the same transaction to avoid orphaned traces — `db.md`
  concern 3), store's provenance is **columns on the completions row itself**, so
  it commits in the *same single `INSERT`* — there is no orphan-trace failure mode
  to engineer around. This is the deliberate, boring right-sizing: store gets
  first-order *origin* provenance for free, and leaves the heavy per-handler-touch
  causal chain to VDB where the operator actually wants it (#92).
- **Observer carries provenance (additive trait change).** To let the KG observer
  build causal links, the `StoreObserver` methods gain an additive `&Provenance`
  parameter (`on_completion_inserted(&self, id, prov)` etc.). `PromiseRegistry`
  ignores it (no recompile break beyond the signature). Flagged as the one
  observer-trait shape change of the wave (Controversial decisions).
- **Terminal-transition provenance.** State transitions (`mark_completed`,
  `mark_failed`, …) are *caused by* the engine/scheduler, not an external
  submitter; they inherit the completion's stored `correlation_id` and stamp
  `causation_id` = the triggering action. Store does not invent a per-transition
  ledger — the completion's lifecycle timestamps (`created_at`, `started_at`,
  `completed_at`, `recovered_at`) + `preemption_count`/`error_retry_count`
  already ARE the intra-node lifecycle provenance trail; wave 2 only adds the
  *origin/causation identity* on top.

### 5. Pressure-covariate columns on `benchmark_runs` — the multidimensional kernel's training data (INTENT #12)

Today `benchmark_runs` (`benchmarks.rs:14`, `schema.sql`) records only
`prompt_tokens`, `max_tokens`, `concurrency`, `tokens_per_second`, `wall_time_ms`.
INTENT #12's kernel is multidimensional over **output-length × parallel-completion
-count × observed memory/CPU/GPU pressure**, and `benchmark.md` is explicit that
**pressure is an observed covariate, not a dial** — "record the pressure *observed
at sample time* (from telemetry's `SystemState`), persisted onto the benchmark-run
row — see the `store` pressure-covariate columns." So store adds, **append-only**
(new `ALTER TABLE … IF NOT EXISTS`-style columns at the bottom of `schema.sql`,
per the stated migration convention `schema.sql:3`), the covariates sampled from
`SystemState` (`types::system::SystemState`) at the moment the run point is
recorded:

| New column (nullable) | Source (`SystemState`) | Why |
|---|---|---|
| `mem_pressure` REAL | `memory_pressure` | memory-pressure axis |
| `cpu_utilization` REAL | `cpu_utilization` | CPU-pressure axis |
| `gpu_utilization` REAL | `gpu_utilization` | GPU-pressure axis |
| `gpu_mem_used_bytes` INTEGER | `gpu_memory_used_bytes` | GPU-memory-pressure axis |
| `is_gpu_estimate` INTEGER | `is_gpu_estimate` | **honesty flag** (INTENT #9): marks the GPU covariate as an estimate so the kernel/dashboard render the tilde/tooltip and can down-weight it |
| `sample_source` TEXT | (producer) | `'benchmark'` (controlled, low-pressure, isolated — `benchmark.md`) vs `'observed'` (a passively-observed real-completion point at higher pressure) — resolves benchmark.md's isolation-vs-pressure-exploration tension by letting the kernel distinguish the two populations |

The `BenchmarkRun` struct (`benchmarks.rs:14`) grows the matching fields;
`insert_benchmark_run` (`benchmarks.rs:41`) and `map_benchmark_row`
(`benchmarks.rs:101`) grow to write/read them (old rows read the new columns as
`NULL`, so the change is backward-safe). This is the concrete "benchmark redesign
needs" delta, and it is mechanical against the existing insert/map pair — the only
judgment is column naming, aligned above to `SystemState`'s field names. Carrying
`is_gpu_estimate` onto the row is the non-obvious but important bit: without it the
kernel would fit against dishonest GPU zeros/estimates as if they were real
readings (telemetry.md: "VRAM telemetry is stubbed at 0").

### 6. Error taxonomy alignment — adopt `types::error::store::StoreError`

Store today returns the flat `SubstrateError::Store(String)` leaf (`sqlite_err`,
`lib.rs:48`; `SubstrateError::Store(e.to_string())`). `types.md`'s wave-2 `error/`
restructuring gives each domain its own sub-enum (`error/store.rs → StoreError`)
wrapped by `SubstrateError::Store(#[from] StoreError)`. Store adopts it: the
existing leaves (`sqlite_err`, poisoned-mutex, `CompletionNotFound`
`completions.rs:130`, the `json_err` → `Internal` mapping `lib.rs:53`) become
matchable `StoreError` variants. Per `types.md`'s explicit migrate-now-vs-later
flag, this is a **mechanical but wide** change across store's construction sites
and is gated on the operator's call on the taxonomy rollout — store is ready to
move whenever types does. No behavior change; matchability improves.

## Relationships / edges

- **{engine, scheduler, models, cache, telemetry, benchmark, api}** (siblings)
  via **`store-access`** *(authored: scaffold/contracts/store-access.md)* — the SQLite
  system-of-record CRUD + queue view + the observer seam, plus (wave 2) the
  provenance-on-completions surface and the benchmark pressure covariates. This is
  an **inference-internal, in-process edge** (store is compiled into `inference`),
  NOT a cross-app wire contract — no mesh relay, no serialization.
- **inference (`PromiseRegistry`)** via the **observer seam** — part of
  `store-access`; the terminal-transition callback path, governed by the concern-2
  reentrancy contract. The KG service is the named future second `StoreObserver`.
- **NON-edges (deliberate, flagged so the harmonizer does not invent them):**
  - **store ↔ mesh** does **not exist.** Store never registers, resolves, or
    speaks pub/sub/restart. `api` owns the node's single external surface and
    mesh registration; store is reached only through `api`/siblings in-process.
  - **store ↔ db** does **not exist** as a data edge. `db-inference-init`
    (inference→db, `bin/db` CLI subprocess) bootstraps the *control-plane `ops`*
    database, a **separate file**; store's own schema is self-applied at
    `Store::open` with no `db` dependency (concern 3). The harmonizer must not
    merge these into one database or invent a store→db call.
  - **store ↔ vfs** does **not exist.** Store's SQLite file is a local node file
    (like `db`'s SQLite handling, `db.md` concern 6); VFS residency of inference
    artifacts (weights, kv-cache blobs) is `models`/`cache` + `gc`'s concern, not
    store's (store holds only *metadata* rows for those).

## Nesting

Parent: **inference** | Children: none. Module `lib/store` (`substrate-store`),
an internal library of the `inference` app-crate (INTENT #22: not an app → a lib,
nested under the app that uses it). Never a standalone crate; never linked across
an app boundary (INTENT #29).

## Thoroughness level

**implementation-ready.** The crate exists and is stable; every wave-2 delta is
mechanical against real code and cross-consistent with the neighbor designs: (a)
the observer reentrancy **contract** is a documentation/interface hardening of an
already-implemented pattern (`PromiseRegistry`); (b) the `db-inference-init`
tension is **resolved** by `db.md`, not left open; (c) provenance adoption is
append-only columns + a `&Provenance` arg threaded from `api` + an additive
observer-trait parameter, using an existing `types` struct; (d) the pressure
covariates are append-only columns + a struct/insert/map extension against the
existing `benchmark_runs` pair; (e) the error alignment tracks `types`'s
restructuring. The one genuine *decision* (observer-trait signature change) is
flagged below, small, and single-implementer today.

## Assigned design-depth

**Opus** (single Component-Designer pass, this file), per the wave2-plan model
tier (`store` = Opus), grounded in the real `lib/store/src/*` + `schema.sql`, the
`PromiseRegistry` `StoreObserver` impl in `lib/inference/src/lib.rs`, and the
neighbor designs `db.md`, `types.md`, `benchmark.md`, `telemetry.md`,
`inference.md`.

## Suggested fill-model

**implementation-ready + low complexity → cheap/fast model OK**, with two
carve-outs a careful hand must not hand-wave: (1) the **observer reentrancy
contract** (concern 2) — a Filler wiring the KG observer must reproduce the
try_lock + `tokio::spawn`-the-store-read + never-re-enter pattern exactly, or the
node deadlocks; put this in a conformance test (below). (2) the **provenance
threading** — `insert_completion`'s new `&Provenance` arg must be plumbed from the
`api` request envelope, not defaulted to empty at the store boundary (a defaulted
provenance is a silent #85 violation). The pressure-covariate columns and the
error-enum alignment are pure transcription.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `store-access` — (store ↔ {engine, scheduler, models, cache, telemetry, benchmark, api} + the composition root's `StoreObserver`; store owns/authors) — the in-process `Store` trait surface + the reentrancy contract for the observer seam; every sibling filed a participation note against it. → `scaffold/contracts/store-access.md`

