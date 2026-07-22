# Contract: kg-api

## Parties
Any service ↔ `kg`. The **consumer surface** of the Knowledge Graph service,
spoken over the local mesh daemon `:3649` (`AnyNode { slug: "kg" }` — the local
leg relays to the graph's home transparently; callers never know which node).
Modeled **surface-schema-style: one shared document, every consumer a party**
(projects, the emergent keeper layer, cc-driven agents, dashboard). Authored
from `kg.md`
concerns 3/4/6/7/10 and its `kg-api` proposal.

**NEW pair — not named in the wave2-plan §3 inventory.** kg.md surfaces it
because the consumer surface has to live somewhere (precedent: vfs surfacing
its own consumer edges). Recorded as a deviation-by-addition, not a scope
change: every consumer of graphs already needed this surface; it was implicit.

## Purpose
The one surface consumers speak to create and use graphs: **templates**
(publish/get, versioned, immutable), **graph lifecycle**
(create/migrate/promote/list — the global registry read), **schema-locked
mutation with push-back** (atomic batch; a validation failure returns
per-element diagnostics and writes nothing), **bounded queries** (get / scan /
neighbors / traverse — depth+limit mandatory, no unbounded walks),
**conflicts + audit** (the surfaced-conflict set and the healthcare-grade
provenance trace), and **trigger registration** (declarative
`types::trigger` data, unchanged from queues).

## Schema
Wire structs land in `types::kg`; `Trigger` is `types::trigger` (queues'
locked model, consumed UNCHANGED); `Provenance` is `types::provenance`;
`KgError` in `types::error::kg`.

```rust
// ── templates ─────────────────────────────────────────────────────────
PublishTemplate { template: TemplateVersion }                 // static-validated; immutable once accepted
GetTemplate     { template_id: TemplateId, version: Option<u32> } // -> TemplateVersion

// ── graph lifecycle ───────────────────────────────────────────────────
CreateGraph  { slug: String, template: TemplateBinding, project: Option<ProjectId>,
               environment: Option<EnvironmentId>, policy: Option<GraphPolicy>,
               provenance: Provenance } // -> GraphDescriptor
MigrateGraph { graph_id: GraphId, to_version: u32 } // -> MigrationReport (whole-or-nothing)
PromoteGraph { graph_id: GraphId, target: CloudTarget } // -> stream<PromotePhase>
ListGraphs   { filter: GraphFilter } // -> Vec<GraphDescriptor>   (the global-registry read)

// ── mutation: atomic batch, schema-locked, push-back ──────────────────
Mutate { graph_id: GraphId, ops: Vec<GraphOp>, provenance: Provenance }
       // -> MutateAck { applied: Vec<ElementVersion>, minted_branch: bool }  (honesty receipt)
enum GraphOp {
    CreateNode { node_type: String, props: Value },
    UpdateNode { node_id: Uuid, props: Value, if_version: Option<Version> },
    DeleteNode { node_id: Uuid },
    CreateEdge { edge_type: String, from_id: Uuid, to_id: Uuid, props: Value },
    UpdateEdge { edge_id: Uuid, props: Value, if_version: Option<Version> },
    DeleteEdge { edge_id: Uuid },
    #[serde(other)] Unknown,
}

// ── reads (all bounded — concern 10) ──────────────────────────────────
GetNode { graph_id: GraphId, node_id: Uuid } / GetEdge { graph_id, edge_id }
Scan      { graph_id, subject: Subject, type_filter: Option<String>,
            prop_filter: Option<PropFilter>, cursor: Option<Cursor>, limit: u32 }
Neighbors { graph_id, node_id: Uuid, edge_types: Vec<String>, direction: Direction,
            include_orphaned: bool /*default false*/, limit: u32 }
Traverse  { graph_id, roots: Vec<Uuid>, edge_types: Vec<String>, direction: Direction,
            max_depth: u32, limit: u32 }   // depth + limit MANDATORY — no unbounded walks

// ── conflicts + audit ─────────────────────────────────────────────────
ListConflicts { graph_id, unresolved_only: bool } // -> Vec<ConflictRecord>
Audit { graph_id, subject_id: Option<Uuid>, correlation_id: Option<Uuid>,
        cursor: Option<Cursor> } // -> Vec<TraceRecord>

// ── triggers (types::trigger UNCHANGED) ───────────────────────────────
RegisterTrigger   { graph_id, trigger: Trigger } / DeregisterTrigger { graph_id, trigger_id: Uuid }
```

## Error cases
`KgError` (in `types::error::kg`):
- **`SchemaViolation { violations: Vec<Violation> }`** — THE push-back
  (INTENT #50): per-element diagnostics, **batch atomic** (one invalid element
  writes nothing). Never coerced, never partially applied.
- `GraphNotFound` / `TemplateNotFound` / `TemplateVersionYanked`.
- **`GraphHomeUnreachable { graph_id, home_node }`** — the typed CP outcome
  under `OfflineWrites::Refuse` (catchable; the caller's partition seam). Reads
  still serve from the local latest snapshot, marked stale.
- `VersionConflict { current: Version }` — an optimistic `if_version` failed
  (a **local** CAS at the home serialization point — genuinely safe here,
  unlike distributed CAS, because writes serialize at home).
- `MigrationRejected { report }` — whole-graph validation failed; the graph is
  **untouched** and stays on its old version.
- `GraphBusy { op }` — a promotion / migration / merge holds the graph.
- `InvalidTraversal` — missing depth/limit. `TriggerInvalid` — static
  validation failure (queues' property, same reason).
- **Non-errors by design:** creating an edge to a `pointer_state: broken` node
  (broken pointers flag, they don't poison); a `Mutate` on a branch under
  `OfflineWrites::Allowed` **succeeds** with `minted_branch: true`.

## Version sensitivity
HIGH on the persisted vocabulary (`TemplateVersion`, `GraphDescriptor`,
`GraphOp`) — additive-only, `#[serde(default)]`, `#[serde(other)]`;
`TemplateVersion` is additionally content-hashed so fleet/cloud copies are
identity-checkable. MEDIUM on the request verbs (new verbs additive). The
error variants applications match on — `GraphHomeUnreachable`,
`SchemaViolation`, and the conflict-surfacing shape — **freeze at
harmonization** (same rule as locks' required partition error). The three
built-in templates are versioned **DATA, not code**: evolving one is
`PublishTemplate v2` + per-graph opt-in `MigrateGraph`, never a silent
redefinition.

## Reconciliation notes
- **Only `kg` proposed this surface; no counter-party shape to reconcile.**
  Consumers speak it as-is. Recorded as a **new pair added to the inventory**
  (not in wave2-plan §3) — the harmonizer should register `kg-api` alongside
  `kg-vdb`/`kg-vfs`/`kg-mesh`.
- **Consumer-side halves (owned elsewhere, kg is a party):**
  - **`projects-kg`** (batch-7 stub authors): kg's half is `kg-api` **verbatim**
    plus a projects-authored `project-tree@1` template; there is no kg-side
    special surface — recorded so batch 7 starts from this substrate half.
  - **`cc-escalation`**: kg **hosts** the execution-engine's `LoopDepthExceeded`
    arm (execution-engine authors the arm; queues.md reserves the union shape);
    kg contributes only `context` enrichment (graph_id, element ids, the causal
    chain slice from `kg_provenance`).
- **Boundary vs `kg-vdb`:** `kg-api` is the *consumer* surface (graph
  semantics); it is implemented on top of `kg-vdb` (storage) — a `Mutate`
  becomes a validated `ExecBatch` against the graph's VDB database. Consumers
  never see `kg-vdb`.
- **Deliberate under-design (flagged):** **no general graph query language**
  (no Cypher/Gremlin/Datalog) in v1 — the named consumers are all bounded
  traversals, and a query language is the classic unbounded-scope trap
  (INTENT #38). Ad-hoc analytics reach the generic `kg_nodes`/`kg_edges` tables
  through `db`'s query-virtualization direction instead. Escalation path named,
  not built.

## Example data
World: nodes **macbook** and **pi**; project **demo**; graph **demo-notes**
(`g-0001`), template `obsidian-md@1`, home-anchored on **macbook**.

**1. Create the graph (the registry mints a descriptor):**

```jsonc
// CreateGraph   dashboard -> kg
{ "slug": "demo-notes",
  "template": { "template_id": "obsidian-md", "version": 1 },
  "project": "demo", "environment": null,
  "policy": { "offline_writes": "Allowed", "pointer_sweep": "24h" },
  "provenance": { "origin_node": "macbook", "origin_service": "dashboard",
                  "correlation_id": "corr-1", "emitted_at": 1752969000000, "hops": 1 } }
// -> GraphDescriptor { graph_id: "g-0001", slug: "demo-notes", status: "Active", … }
```

**2. Schema-locked mutation with push-back (one bad op → nothing written):**

```jsonc
// Mutate   dashboard -> kg
{ "graph_id": "g-0001",
  "ops": [ { "CreateNode": { "node_type": "note",
             "props": { "title": "Mind OS", "file": "vfs://demo/notes/mindos.md" } } },
           { "CreateEdge": { "edge_type": "links_to", "from_id": "n-42", "to_id": "n-99",
             "props": {} } } ],   // n-99 is a valid forward-link (obsidian-md allows dangling)
  "provenance": { "origin_node": "macbook", "origin_service": "dashboard",
                  "correlation_id": "corr-2", "emitted_at": 1752969600123, "hops": 1 } }
// -> MutateAck { applied: [ {id:"n-42",version:{ts_ms:1752969600123,ctr:0,node:"macbook"}},
//                           {id:"e-7", …} ], minted_branch: false }

// contrast — an unknown node type (closed-world schema lock):
// Mutate { ops: [ CreateNode { node_type: "widget", … } ] }
// -> KgError::SchemaViolation { violations: [ { element: 0,
//      detail: "node_type 'widget' not in template obsidian-md@1 (closed world)" } ] }
//    NOTHING written; the graph's Version watermark is unchanged.
```

**3. Bounded traversal (Obsidian link-following, 2 hops):**

```jsonc
// Traverse   dashboard -> kg
{ "graph_id": "g-0001", "roots": ["n-42"], "edge_types": ["links_to"],
  "direction": "Outgoing", "max_depth": 2, "limit": 100 }
// -> nodes reachable from n-42 by links_to within 2 hops (orphaned edges excluded).
```

**4. Home unreachable under Refuse (the typed CP seam) — here `Allowed`, so a
branch mints instead:** with `offline_writes: Allowed`, a `Mutate` issued from
**pi** while macbook is unreachable returns `MutateAck { …, minted_branch:
true }` (the honesty receipt), and the branch merges on reconnect (`kg-mesh`).
A `Refuse`-policy graph would instead return
`KgError::GraphHomeUnreachable { graph_id: "g-0001", home_node: "macbook" }`.
