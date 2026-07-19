# Contract: kg-vdb

## Parties
`kg` → `vdb`. The locked-layering edge **VFS < VDB < KG** (INTENT #96): KG is
built ON VDB. Over the local mesh daemon `:3649`, never linked (INTENT #29 —
`kg` never speaks to `db` directly; every action goes through `vdb`). NEW pair
(wave2-plan §3b). Authored from `kg.md` (its `kg-vdb` requirements half) and
`vdb.md` concern 11 / its `kg-vdb` offered surface — which **disagree on the
trigger-evaluation locus and the addressing/definition shape** (see
Reconciliation notes). Co-designed batch 4.

## Purpose
KG built ON VDB, concretely: `kg` asks `vdb` to **(1)** provision a graph's
backing database from a KG-authored stack definition on a target (local
SQLite-in-VFS / Supabase / RDS+Lambda), **(2)** execute **atomic transactional
mutation batches** and bounded queries against it (VDB uses `db` to run the
actions — `kg` never speaks to `db`), **(3)** mark the database **kg-owned** so
VDB refuses non-kg writers (a direct mutation would bypass schema-locking and
provenance), **(4)** run copy/verify/switch **promotion** under kg's WholeFleet
lock, and **(5)** fold the database into the restart/upgrade ladder (INTENT
#86), notifying `kg` before its graphs' databases restart.

A graph's backing store is an ordinary VDB database; `kg` uses `vdb` as a
**transactional SQL store + lifecycle manager**, and hosts its own
trigger/handler evaluation (see Reconciliation note 1). `vdb` offers `kg` no
bespoke graph verbs — the same generic surface every consumer gets.

## Schema
`GraphId`/`Residence` are `types::kg`; `DatabaseId`/`StackDefRef`/
`StackTargetKind`/`VdbError` are `types::vdb`; `Provenance` is `types::provenance`.

```rust
// ── addressing: a graph maps to a vdb database (see Reconciliation note 2) ──
// kg derives DatabaseId { project: "kg", db: "<graph_id>" } for each graph.
// Wire identity is vdb's DatabaseId; kg keeps the GraphId<->DatabaseId map.

// ── lifecycle ─────────────────────────────────────────────────────────
struct EnsureDatabase {                          // idempotent
    db_id:       DatabaseId,
    definition:  StackDefRef,                    // KG authors the graph-relational core-schema stack def
    owner:       Slug,                           // "kg" — vdb enforces kg-owned writes (note 3)
    target_hint: Option<StackTargetKind>,        // environment routing replaces this later (environments-vdb)
    triggers:    TriggerHosting,                 // Kg == kg hosts its own engine adapter; no vdb changelog eval
}                                                // -> EnsureDatabaseAck { created: bool }
enum TriggerHosting { Kg, Vdb }                  // kg always sends Kg (note 1)
struct DropDatabase { db_id: DatabaseId, tombstone: bool }

// ── data plane: atomic batches + bounded reads (kg's write path) ───────
struct ExecBatch {                               // kg's Mutate lands here — always atomic
    db_id: DatabaseId, statements: Vec<SqlOp>, atomic: bool /* always true from kg */,
    provenance: Provenance,                      // vdb traces the action; kg writes kg_provenance rows in-batch
} // -> ExecReceipt { rows_touched: u32 }
struct QueryPage { db_id: DatabaseId, sql: String, params: Vec<SqlParam>,
                   cursor: Option<Cursor>, limit: u32 } // -> Rows

// ── promotion (kg's environment routing; INTENT #86 mechanics are vdb's) ──
struct PromoteDb { db_id: DatabaseId, to: StackTargetKind,
                   lock: HoldToken /* kg.promote.<graph_id>, WholeFleet */ }
enum PromotePhase { Copying, Verifying, Draining /*handlers finish old*/, Switched,
                    Failed { reason: String }, #[serde(other)] Unknown } // streamed

// ── lifecycle coupling (vdb -> kg, pre-restart) ────────────────────────
struct DbRestartNotice { db_id: DatabaseId, level: RestartLevel } // ladder inheritance (note 4)
```

**No `SubscribeChanges`, no `Invoke` on this edge (Reconciliation note 1).**
`vdb`'s generic surface offers both; `kg` uses **neither** — it hosts the
execution-engine's kg-nodes adapter on its **own** write path.

## Error cases
`VdbError` at the boundary; `kg` translates the graph-relevant ones for callers.

- `VdbError::DbNotFound { db_id }`; `VdbError::NotHomeHere { db_id, home }` —
  `kg` resolves the entity and relays, same as any consumer.
- **`NotOwner { db_id }`** — a non-kg writer touched a kg-owned database; `vdb`
  refuses AND raises it to `kg` as an integrity alarm.
- `VdbError::Busy { db_id, state }` — the database is under a promotion barrier
  / migration `CriticalSection`.
- `VdbError::DefinitionInvalid { detail }` — a failed schema validation at
  `apply_definition`; **this is the substrate for KG's push-back-to-caller
  requirement** (KG surfaces it as `KgError::SchemaViolation` on `kg-api`).
- `TargetUnavailable { target }` — cloud target unreachable (`kg` emits
  `kg.sync.deferred` / fails promotion cleanly).
- `VerifyFailed` during promotion → promotion aborts, the old database stands
  (never a half-switch).
- `SchemaDrift { db_id }` — the core schema on the target ≠ the expected
  `core_schema` version in the stack def (a migration of KG's OWN core schema is
  a vdb-mediated, versioned migration).

## Version sensitivity
MEDIUM-HIGH. The KG core-schema version (carried inside `StackDefRef`) bumps
rarely and, when it does, is a **fleet-wide vdb-mediated migration across all
graph databases** — flagged to supervision. `StackTargetKind`/`PromotePhase`
reserve `#[serde(other)]`. The `SqlOp` batch stream is vdb-version-coupled and
rides the mesh-transport proto floor. All structs additive-only,
`#[serde(default)]`.

## Reconciliation notes
1. **THE disagreement — trigger-evaluation locus. Resolved IN KG'S FAVOR.**
   `vdb.md` concern 11 assumed "KG's trigger/handler support IS the
   execution-engine's kg-nodes adapter evaluating **vdb's changelog**" and
   offered `SubscribeChanges` (a durable changelog stream) + `Invoke` as KG's
   feed. `kg.md` concern 7 instead hosts the **kg-nodes adapter IN the kg
   process, hooking kg's OWN `Mutate` write path** — NOT vdb's changelog. KG's
   design **requires** this: its merge model (concern 4) fires triggers by
   `write_id` at the write-serialization point, must NOT re-fire on
   merge-applied elements, and MUST fire on merge-*created* changes (quarantine
   flips) as new writes — semantics `vdb`'s generic changelog evaluator cannot
   know (it has no notion of kg branches, `write_id` dedup, or graph
   templates). Evaluating in `vdb` would double-fire across merges and corrupt
   the "at-most-once per write" guarantee. **KG owns trigger evaluation; `vdb`
   is a transactional store here.** Consequence pinned on the wire:
   `EnsureDatabase.triggers = Kg`, and `vdb` installs **no** `_vdb_changelog`
   trigger machinery for kg-owned databases (nothing would consume it).
   **Losing position recorded:** `vdb`'s `SubscribeChanges`/`Invoke` remain its
   generic surface for the *stack pattern's* own consumers — they are simply
   **unused by `kg-vdb`**, not removed.
2. **Addressing — `GraphId` (kg) vs `DbId`/`DatabaseId` (vdb). Reconciled to
   vdb's `DatabaseId` on the wire.** `kg.md` addressed by `GraphId` (a Uuid) →
   an opaque `db_ref`; `vdb.md` addresses by `DatabaseId`/`DbId`. Adopted:
   **`kg` derives `DatabaseId { project: "kg", db: "<graph_id>" }`** and keeps
   the `GraphId ↔ DatabaseId` map on its side; the wire identity is vdb's
   `DatabaseId` (the same type reconciled in `vdb-db`). This keeps vdb's catalog
   ("knows databases, not graphs") and kg's registry ("knows graphs") cleanly
   separated — vdb's own note that "the global graph registry is KG's own."
3. **`owner` marking — kg's requirement ACCEPTED as an added field.** `vdb.md`
   did not model an owner on `EnsureDatabase`; `kg.md` needs "vdb refuses
   non-kg writers" (its `NotOwner` integrity guard). Accepted: `owner: Slug` is
   added to `EnsureDatabase`, and `vdb` enforces kg-owned-writes-only + raises
   `NotOwner` on violation.
4. **`DbRestartNotice` — kept on this edge for now, flagged.** `vdb.md` asked
   whether pre-restart notice should ride `restart-protocol` instead of a
   bespoke `kg-vdb` frame. Kept here (a database entity is not a mesh service
   `kg` registered; the notice is vdb→kg about a database `kg` owns) — flagged
   for the harmonizer to fold into `restart-protocol` if the addressing works.
5. **Provenance double-write, reconciled by layer.** Both `kg` and `vdb` call
   themselves provenance's "primary home." Resolved by scope: `kg` writes
   **graph-semantic** provenance (`kg_provenance` rows, as ordinary statements
   inside `ExecBatch` — element→write_id→cause); `vdb` records the
   **action-level** touch (its execution arm ran statement X). For kg-owned
   databases, vdb's `ProvenanceConfig` is `Touch` (the action fact), and the
   forensic graph lineage lives in `kg_provenance`. No duplication of the
   healthcare-grade chain — two layers, two questions.
6. **Verb-granularity flag (both flagged mid-batch):** `ExecBatch` carries
   raw `SqlOp`s against kg's generic `kg_nodes`/`kg_edges` tables (kg.md 3b), not
   typed graph verbs — `vdb` stays a boring SQL executor. The harmonizer should
   confirm `SqlOp` == vdb's `db`-session statement shape (it does: both bottom
   out in `db`'s parameterized SQL).

## Example data
World: nodes **macbook** and **pi**; project **demo**. The graph **demo-notes**
(`graph_id = g-0001`, template `obsidian-md@1`) is home-anchored on **macbook**;
its backing database is `DatabaseId { project: "kg", db: "g-0001" }`, a local
SQLite file in VFS.

**1. Ensure the graph database exists (kg hosts its own triggers):**

```jsonc
// EnsureDatabase   kg -> vdb
{ "db_id": { "project": "kg", "db": "g-0001" },
  "definition": { "manifest_hash": "sha256:core-schema-v1…", "ledger_head": "0001_kg_core" },
  "owner": "kg",
  "target_hint": { "LocalSqlite": { "node": "macbook", "vfs_path": "vfs://kg/g-0001/graph.db" } },
  "triggers": "Kg" }
// EnsureDatabaseAck   vdb -> kg
{ "created": true }
// vdb installs NO _vdb_changelog triggers (owner=kg, triggers=Kg): kg evaluates its own.
```

**2. An atomic mutation batch (a kg-api Mutate for demo-notes lands here):**

```jsonc
// ExecBatch   kg -> vdb   (atomic; kg's write-serialization point on macbook)
{ "db_id": { "project": "kg", "db": "g-0001" },
  "atomic": true,
  "statements": [
    { "sql": "INSERT INTO kg_nodes(node_id,node_type,props,ver_ts,ver_ctr,ver_node,write_id,deleted) VALUES(?1,'note',?2,?3,0,'macbook',?4,0)",
      "params": ["n-42","{\"title\":\"Mind OS\",\"file\":\"vfs://demo/notes/mindos.md\"}",1752969600123,"w-42"] },
    { "sql": "INSERT INTO kg_provenance(trace_id,subject_kind,subject_id,op,write_id,actor_service,actor_node,correlation_id,occurred_at,after) VALUES(?1,'node','n-42','insert','w-42','kg','macbook','corr-9',1752969600123,?2)",
      "params": ["t-1","{\"title\":\"Mind OS\"}"] } ],
  "provenance": { "origin_node": "macbook", "origin_service": "kg",
                  "correlation_id": "corr-9", "causation_id": null,
                  "emitted_at": 1752969600123, "hops": 1 } }
// -> ExecReceipt { rows_touched: 2 }
```

**3. A non-kg writer is refused (the ownership guard):**

```jsonc
// some service tries to write kg/g-0001 directly through vdb
// -> VdbError::NotOwner { db_id: { project:"kg", db:"g-0001" } }
// vdb refuses the write AND alarms kg (integrity).
```

**4. Promote demo-notes to a Supabase environment:**

```jsonc
// PromoteDb   kg -> vdb   (kg holds kg.promote.g-0001, WholeFleet)
{ "db_id": { "project": "kg", "db": "g-0001" },
  "to": { "Supabase": { "project_ref": "abcdefghij" } },
  "lock": "<HoldToken kg.promote.g-0001>" }
// PromotePhase stream vdb -> kg: Copying -> Verifying -> Draining -> Switched
// in-flight handlers finish against the OLD sqlite db before the switch (INTENT #86).
```
