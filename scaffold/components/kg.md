# kg

**Status:** wave-2 full design pass (batch 4, Fable seat), 2026-07-19 —
SUPERSEDES the round-8 requirements-only stub (its verbatim requirement
capture is honored in full below; nothing is dropped, everything is now
designed). **Nesting:** top-level app-crate (`bin/kg` + `lib/kg`), L4 data &
execution plane. **Locked layering honored throughout: VFS < VDB < KG**
(INTENT #96). **Consumes:** vdb (graph databases), vfs (file-pointer
validation), mesh (registration / `kg/` keyspace / locks / cron / pubsub via
`mesh-client`), and the shared `execution-engine` lib (kg-nodes adapter,
compiled in). **Consumed by:** projects (L6, `projects-kg`), org (RESOLVED
home of its self-restructuring graph — friction-round 2, INTENT #121),
any service that creates or reads graphs (`kg-api`, NEW — see Proposed
contracts) — with the stated posture "assume that every service will be
using the knowledge graph" (INTENT #121). Grounded in INTENT #50/#62/#85/#92/#93/#96/#97, the batch-1/2/3
designs (types provenance/event vocabulary, replicated-kv's Version
discipline + guarantees table, locks' mint-and-merge divergence pattern,
queues' shared trigger data model, vfs's NodeAnchored files + `Exists`
surface + content addressing, mesh-core addressing classes), and the
requirement stubs of the co-batched `stack.md`(vdb)/`db.md`.

## Charter

`kg` is **the Knowledge Graph service of Substrate**: a mesh-wide, globally
registered collection of **graphs** — schema-locked, template-typed,
provenance-traced, trigger-bearing — each stored as **its own VDB-managed
database** and each linked to a project + environment that routes its
storage (local → SQLite-in-VFS; cloud → promoted Supabase/RDS). It owns:
the **global graph registry** ("a place where we can access ALL the
knowledge graphs," INTENT #97 — every graph discoverable from every node);
the **versioned template system** (research / Obsidian-markdown /
`.mind`-axiomatic from day one, INTENT #50) including template publication,
graph↔template-version binding, and explicit template migration; **schema
locking on nodes AND edges** with push-back-on-validation-failure to the
calling service (rejected, never coerced); **node→VFS-file pointers with
existence validation** (creation-time and sweep-time); the **graph merge /
distributed-consistency model** (concern 4 — THE standing open, decided
here as element-wise LWW + structural repair + first-class conflict
surfacing); **trigger/handler support** via the shared execution-engine's
kg-nodes adapter, with healthcare-grade provenance on every handler touch
(INTENT #85/#92 — this plane is provenance's primary home, alongside VDB);
**environment routing and cloud promotion** of individual graphs; and
**cross-boundary eventual consistency with AWS-resident graph replicas**
(the Broomstick intent-extraction use case, INTENT #50).

**Charter addition (friction-round 2, INTENT #121):** kg explicitly exposes,
to any consuming service, the ability to **add new node types, schemas, and
version control** over them — and the design posture is to **"assume that
every service will be using the knowledge graph."** KG is not a specialist
tool a few services opt into; it is a universal plane. The first confirmed
beneficiary is `org`, whose self-restructuring org-graph is now RESOLVED to
live on KG (db keeps only flat relational metadata — see the boundary note
below and org.md).

**Boundary — what `kg` does NOT own.** It does not own *database
mechanics* — a graph's tables live in a VDB-managed database; VDB tracks
the database, `db` runs the actions, VFS hosts the SQLite file (locked
stack, INTENT #96); kg never opens a database file directly and never
imports `vdb`/`db` as libraries (INTENT #29 — every hop is over the local
`:3649` daemon). It does not own *file storage* — node bodies and pointed-at
documents are VFS files; kg stores structure, properties, and pointers. It
does not own *replication transport* — bytes move over mesh
(`mesh-transport`), registry state rides the `kg/` keyspace of
`replicated-kv`, and graph-file durability (snapshots, replication factor,
S3 overflow) is inherited from VFS/aws through VDB; what kg owns is the
**graph-semantic** half of consistency: schema-aware merge, repair, and
conflict surfacing that no generic byte/LWW layer can do (see the honest
scope note in concern 4). It does not own the trigger *vocabulary* — it
consumes `types::trigger` UNCHANGED (queues' locked shared data model) and
the shared execution-engine lib; kg contributes only the kg-nodes subject
binding and the host process. It does not own project structure — `projects`
(L6) builds its graphical file system AS a kg graph; kg is the substrate,
projects is the consumer. The `org` metacognitive graph's home is
**RESOLVED (friction-round 2, INTENT #121): org's self-restructuring graph
lives ON kg** — "Org's self-restructuring knowledge graph definitely
belongs in the KG service"; `db` serves org only for flat relational
metadata. kg is the substrate there too; the org-graph's *semantics* remain
org's.

## Primary design concerns

### 1. One graph = one VDB-managed database — the load-bearing shape

The stub's open question ("graphs-as-VDB-databases: each graph its own
database, or one shared database?") is decided: **each graph is its own
VDB-managed database**, created from the graph-relational core schema
(concern 3) at `CreateGraph` time. Rationale, each point load-bearing:

- **Environment routing is per-graph** (INTENT #97: "each graph links to a
  project + environment which routes storage"). A shared database cannot
  promote one graph to Supabase while its neighbor stays local; per-graph
  databases make promotion *exactly* VDB's copy/verify/switch-under-a-lock
  mechanic (INTENT #86), inherited for free, no kg-specific migration code.
- **Provenance is scoped per-project/per-database** (INTENT #92). One graph
  = one database = one provenance domain — the healthcare-grade trace log
  (concern 6) is naturally partitioned, exportable, and auditable per graph.
- **Databases are services under the restart/upgrade ladder** (INTENT #86;
  supervision's 4-level ladder). A graph inherits lifecycle treatment
  through its database: a template migration or promotion is a
  database-restart choreography VDB already owns.
- **Isolation**: one graph's handler storm, lock contention, or corruption
  never touches another graph. A graph is archived/dropped by
  archiving/dropping a database.
- **The registry problem is solved separately** (concern 2) — the one thing
  a shared database would have provided (a global index) rides the `kg/`
  replicated-kv keyspace instead, which is strictly better: readable on
  every node from a purely local KV read, even when a graph's home database
  is on an unreachable node.

Scale posture, stated honestly: hundreds of graphs = hundreds of small
SQLite files in VFS and hundreds of VDB-tracked databases. At personal-mesh
scale this is boring (SQLite files are cheap; VDB's tracking records are KV
rows); it is flagged for VDB's co-design pass so VDB's database registry is
designed for many-small, not few-large (friction point).

### 2. The global registry + template catalog — the `kg/` keyspace

Registry and templates are small, LWW-friendly control-plane records; they
ride a **`kg/` keyspace on `replicated-kv`** (opened via `mesh-client`'s
KvHandle — the exact precedent `vfs` set with its `vfs/` keyspace). Every
node can answer "what graphs exist, where does each live, what schema does
it speak" from a local read — the INTENT #97 global-registry requirement
("access ALL the knowledge graphs") holds with zero cross-node calls.

```
kg/graph/<graph_id>            -> GraphDescriptor     (LWW; single-writer in practice: the home leg)
kg/template/<tid>/<version>    -> TemplateVersion     (IMMUTABLE once published — LWW-safe by construction)
kg/template/<tid>              -> TemplateHead        { latest: u32, yanked: Vec<u32> }
```

```rust
pub struct GraphDescriptor {
    pub graph_id: GraphId,               // Uuid
    pub slug: String,                    // human name, dot-segmented, unique-by-LWW
    pub template: TemplateBinding,       // { template_id, version } — the pinned schema set
    pub project: Option<ProjectId>,      // linkage, NOT visibility scope (INTENT #97)
    pub environment: Option<EnvironmentId>,
    pub residence: Residence,            // where the graph's database lives (concern 8)
    pub policy: GraphPolicy,             // offline_writes, sweep cadence, sync spec (concerns 4/5/9)
    pub status: GraphStatus,             // Active | Migrating | Promoting | Archived | Tombstoned
    pub created: Provenance,             // types::Provenance — who/when/what minted it
}
pub enum Residence {
    Local { home_node: NodeId, vfs_path: String },        // SQLite file, NodeAnchored in VFS
    Cloud { target: CloudTarget },                        // Supabase | AwsRdsLambda (via VDB)
}
pub struct GraphPolicy {
    pub offline_writes: OfflineWrites,   // Refuse (default) | Allowed — the CAP dial, concern 4
    pub sync: Option<CloudSyncSpec>,     // cross-boundary sync loop, concern 9
    pub pointer_sweep: Duration,         // FileRef re-validation cadence (default 24h), concern 5
}
```

**Cross-project reuse and schema referencing** (INTENT #97): the
project/environment link routes storage and scopes provenance config; it
does NOT scope visibility — any service may resolve any graph by id/slug,
and may fetch any `TemplateVersion` purely to reference its schemas
("referenced for its schema") or to create a new graph "like graph G"
(binding to G's template@version). The do-not-over-constrain nuance is
honored structurally: `residence` and `environment` are independent fields —
a Local-residence graph may serve a cloud environment and vice versa; the
environment only supplies the *default* residence at creation.

### 3. Schema-locked nodes and edges — templates, versions, validation, migration

**The template model (pinned).** A **template** is a named, versioned,
immutable set of node schemas + edge schemas + graph config. A
`TemplateVersion` contains everything a graph needs to validate and render
itself:

```rust
pub struct TemplateVersion {
    pub template_id: TemplateId,         // slug: "research" | "obsidian-md" | "mind-axiomatic" | user-defined
    pub version: u32,                    // monotonic per template; immutable once published
    pub content_hash: Hash,              // canonical-JSON hash — identity check across the fleet/cloud
    pub nodes: Vec<NodeTypeSchema>,
    pub edges: Vec<EdgeTypeSchema>,
    pub graph: GraphConfig,
    pub migration_hints: Vec<MigrationHint>, // declarative, e.g. RenamedProp { node_type, from, to }
}
pub struct NodeTypeSchema {
    pub type_name: String,               // "hypothesis", "note", "axiom", …
    pub props: PropSchema,               // the boring validation subset (below)
    pub file_refs: Vec<FileRefField>,    // which props are VFS pointers + their rules (concern 5)
    pub display: DisplayHints,           // label prop, color, icon — surface-schema rendering feed
}
pub struct EdgeTypeSchema {
    pub type_name: String,               // "supports", "links_to", "decomposes_into", …
    pub endpoints: Vec<EndpointRule>,    // allowed (from_type, to_type) pairs
    pub cardinality: Cardinality,        // ManyToMany (default) | AtMostOneOutgoing | AtMostOneIncoming | OneToOne
    pub props: PropSchema,
}
pub struct GraphConfig {
    pub closed_world: bool,              // true (default): unknown node/edge types REJECTED — schema-locked
    pub allow_dangling_file_refs: bool,  // false default; true for Obsidian forward-links (concern 5)
    pub shipped_triggers: Vec<Trigger>,  // OPTIONAL declarative trigger data (types::trigger, unchanged)
}
```

> **Possible extraction: `onion` (friction-round 2, INTENT #122 — recorded,
> NOT designed).** kg's templating / schema-inheritance machinery (this
> concern) is a candidate for extraction into a standalone crate named
> **`onion`** — a schema-**delta** layering language over templates (add
> attributes to nodes/edges, new node types, new edge types; deltas FLATTEN
> into the template when it is updated to include them; residual misaligned
> fields stay as deltas on top), applicable beyond graphs (YAML, JSON
> schema). See `components/onion.md` for the requirements stub. **A
> dedicated discussion round on KG templating is REQUIRED before any design
> here changes** — the template model below stands as-is until then.

`PropSchema` is a **deliberately boring subset of JSON Schema** — types
(string/number/bool/timestamp/enum/array/object), `required`, `enum` values,
`pattern`, min/max — fully statically validatable, mirroring queues'
declarative-data philosophy (a malformed template is rejected at
`PublishTemplate`, never at write time). NOT full JSON Schema (no `$ref`
recursion, no conditionals) — escalation, not scope.

**The three built-in templates (v1, shipped as data, versioned like any
other):**
- **`research@1`** — nodes: `hypothesis`, `experiment`, `method`,
  `conclusion`, `observation`; edges: `tests` (experiment→hypothesis),
  `uses` (experiment→method), `supports`/`refutes`
  (observation|conclusion→hypothesis), `derives` (conclusion→experiment).
- **`obsidian-md@1`** — nodes: `note` (with a required `file` FileRef to
  the markdown body in VFS, title, tags); edges: `links_to` (note→note,
  wikilink), `embeds`, `tagged` (note→`tag` node);
  `allow_dangling_file_refs: true` (forward links are the Obsidian idiom).
- **`mind-axiomatic@1`** — nodes: `axiom`, `tension`, `map`, `directive`,
  `index`; edges: `derives_from`, `in_tension_with`, `indexes`,
  `supersedes` — mirroring the operator's `.mind` axiomatic-breakdown
  structure (the prior art in `~/code/harness/.mind/axioms`).

**Validation with push-back (INTENT #50, LOCKED behavior).** Every mutation
(`Mutate` batch, concern 7's API) is validated against the graph's bound
`TemplateVersion` BEFORE any write: node/edge type exists (closed world),
props validate, edge endpoint types allowed, cardinality pre-checked against
current state, FileRefs existence-checked (concern 5). Any failure →
`KgError::SchemaViolation { violations: Vec<Violation> }` returned to the
calling service with per-element diagnostics; **the batch is atomic — one
invalid element writes nothing** (a SQLite transaction underneath). Never
coerced, never partially applied. (Merge-time constraint violations are the
one case that cannot be pre-checked — they are *surfaced*, not rejected;
concern 4.)

**Template evolution + graph migration.** Publishing `template@N+1` never
touches existing graphs — each graph stays pinned to its bound version.
Migration is explicit and per-graph: `MigrateGraph { graph_id, to_version }`
runs under a `locks-api` mutex (`kg.migrate.<graph_id>`) as: (1) validate
the ENTIRE existing graph against the new version's schemas (applying
declarative `migration_hints` — renames as data, never code); (2) any
nonconforming element → the migration is **rejected whole** with a full
violation report pushed back to the caller (the graph remains on the old
version, untouched); (3) all-conforming → flip the binding, record the
migration in provenance. Additive evolution (new types, new optional props)
therefore migrates trivially; narrowing evolution requires the sweep to
pass or the caller to fix the data first. Because element storage is
generic (concern 3b) the migration needs **no DDL** — it is a validation
sweep + a binding flip, which is what makes rejection-whole cheap and safe.

### 3b. Graph storage — the generic graph-relational core schema

Inside each graph's database, kg uses **one generic, template-independent
schema** — validation lives in kg's schema layer (that is where the lock
is), not in per-type DDL:

```sql
CREATE TABLE kg_nodes (
  node_id   TEXT PRIMARY KEY,          -- Uuid
  node_type TEXT NOT NULL,
  props     TEXT NOT NULL,             -- canonical JSON, template-validated
  ver_ts INTEGER NOT NULL, ver_ctr INTEGER NOT NULL, ver_node TEXT NOT NULL,  -- HLC Version (concern 4)
  write_id  TEXT NOT NULL,             -- Uuid per write — trigger dedup + provenance join (concern 7)
  deleted   INTEGER NOT NULL DEFAULT 0 -- tombstone flag (delete is a write)
);
CREATE TABLE kg_edges (
  edge_id   TEXT PRIMARY KEY,
  edge_type TEXT NOT NULL,
  from_id   TEXT NOT NULL, to_id TEXT NOT NULL,
  props     TEXT NOT NULL,
  ver_ts INTEGER NOT NULL, ver_ctr INTEGER NOT NULL, ver_node TEXT NOT NULL,
  write_id  TEXT NOT NULL,
  deleted   INTEGER NOT NULL DEFAULT 0,
  state     TEXT NOT NULL DEFAULT 'live'   -- 'live' | 'orphaned' (quarantine, concern 4)
);
CREATE TABLE kg_provenance (             -- append-only; the healthcare-grade trace log (concern 6)
  trace_id TEXT PRIMARY KEY, subject_kind TEXT, subject_id TEXT, op TEXT,
  write_id TEXT, actor_service TEXT, actor_node TEXT,
  causation_id TEXT, correlation_id TEXT, handler_invocation_id TEXT,
  occurred_at INTEGER, before TEXT, after TEXT
);
CREATE TABLE kg_conflicts (              -- the surfaced-conflict set (concern 4)
  conflict_id TEXT PRIMARY KEY, kind TEXT, subjects TEXT /*json ids*/,
  detected_at INTEGER, resolved_at INTEGER, detail TEXT
);
CREATE TABLE kg_meta (k TEXT PRIMARY KEY, v TEXT);
  -- graph_id, template binding cache, branch epoch, sync cursors (concern 9)
```

**Why generic tables, not per-type tables (controversial, flagged):**
per-type DDL would marry every template version to ALTER TABLE migrations
across three heterogeneous VDB targets, and would give the storage layer a
schema opinion that kg's validation layer already owns authoritatively.
Generic tables make: migration a data sweep (concern 3), the merge protocol
uniform (one element shape), the kg-nodes trigger subject uniform, and the
storage schema identical on SQLite/Supabase/RDS. The trade — no per-type
SQL-level constraints, and ad-hoc SQL against a graph reads JSON props — is
accepted and flagged: `db`'s query-virtualization direction can still reach
these tables for ad-hoc analysis, and indexes on
`(node_type)`, `(edge_type, from_id)`, `(edge_type, to_id)` keep traversal
boring-fast at personal scale.

### 4. The distributed-consistency model — THE standing open, decided for v1

The operator's framing: KV can be naive timestamp-wins; a graph of
interconnected nodes is "the superset basically." The v1 model decided here
is **"LWW-plus": element-wise LWW convergence + deterministic structural
repair + first-class conflict surfacing + mint-on-partition write
topology** — a composition of already-designed primitives, not a new CRDT.
Stated as four rules and an honesty table:

**Rule 1 — the unit of convergence is the element.** Every node row and
edge row carries an HLC `Version` — the SAME `(ts_ms, ctr, node)` struct
and frozen total order as `replicated-kv` (one version discipline OS-wide;
kg's writer stamps versions through the same ratchet rules). Merging two
divergent replicas of a graph = per-element compare, larger Version wins,
tombstones are writes. Idempotent, commutative, deterministic — every
replica converges to the same element set. This is the "superset" base: a
graph IS a keyed element set, so per-key LWW applies verbatim.

**Rule 2 — the inter-element invariants LWW can break are enumerated, and
each gets a deterministic repair or a surfaced conflict — never silent
loss:**

- **Dangling edge** (one side deleted a node; the other side concurrently
  added/kept an edge to it): the edge is **quarantined** — flipped to
  `state: 'orphaned'` (invisible to normal reads/traversals, retained in
  full), a `kg_conflicts` row is minted, and `kg.conflict.dangling_edge` is
  published. NOT cascade-deleted: silent cascade would destroy data written
  in good faith; push-back philosophy extends to merge time as
  surface-don't-destroy. Un-deleting the node (a newer write) automatically
  re-lives its orphaned edges.
- **Cardinality violation** (template says at-most-one; each side added
  one): both elements are KEPT, a conflict row is minted
  (`kind: cardinality`), reads see both elements flagged, and
  `kg.conflict.cardinality` publishes. The application/agent resolves by
  deleting one — resolution is just an ordinary write, exactly like locks'
  "resolution is releasing" (no special resolve verb; un-gameable, boring).
- **Concurrent edits to one element**: plain LWW — one wins fleet-wide.
  The loser is NOT lost: every write is in `kg_provenance` (before/after
  images), and provenance logs merge as an **append-only union** (trace
  rows are immutable and uuid-keyed — a natural conflict-free set). The
  healthcare-grade trace IS the lost-update audit trail; a
  `conflict-audit` read reconstructs any overwritten value. This is the
  designed convergence of INTENT #85 and INTENT #32: provenance is what
  makes naive LWW forensically safe.
- **Concurrent body edits** (Obsidian-style FileRef bodies): the two bodies
  are two content-addressed immutable VFS blobs — **both physically
  survive** (VFS dedup/content addressing); the node's pointer prop LWWs to
  one, and a `kind: body_divergence` conflict row records both hashes. No
  bytes are ever lost; the losing body is one `vfs://` read away.

**Rule 3 — write topology: single-home serialization, divergence only by
minting (locks' pattern, reused).** Every Local-residence graph has a
**home node** — the node its SQLite file is anchored to (VFS NodeAnchored;
kg's home leg holds the `vfs.anchor` lock via VDB). ALL writes for a graph
route to the home leg (any kg leg accepts a request and relays
`Node{home}` over mesh — single-port locality; callers never know). One
serializer ⇒ within a connected component, write conflicts **cannot
arise** — Rules 1–2 are the *merge-edge* machinery, idle in steady state.
If the home is unreachable, the graph's policy governs:
- `OfflineWrites::Refuse` (**default**): writes fail fast with the typed,
  catchable `KgError::GraphHomeUnreachable { graph_id, home_node }` — the
  per-graph CP dial, mirroring locks' `WholeFleet` honesty. Reads still
  serve from the local latest snapshot, marked stale.
- `OfflineWrites::Allowed`: the requesting node **mints a branch replica**
  (from its latest VFS snapshot of the graph, new `branch_id`, recorded in
  `kg/graph/<id>` descriptor state) and writes locally — exactly locks'
  "divergence is created only by minting," with the same receipt honesty
  (`minted_branch: true` on every ack). On reconnect, the branch merges
  into home per Rules 1–2 under a `kg.merge.<graph_id>` mutex, then
  retires. Divergence exposure is bounded and *chosen per graph*, never
  ambient.

The default is `Refuse` — graphs are curated knowledge, and the operator's
"everything's driven from my laptop" means home-unreachable writes are
rare; a walk-along-Pi graph that must accept offline writes opts into
`Allowed` knowingly. **Flagged as a real operator call** (the
availability-leaning alternative default is defensible — friction point).

**Rule 4 — merge transport is state-based over snapshots, executed by kg.**
A branch (or cloud replica, concern 9) merges by shipping its element set
(or a cursor-bounded delta: elements with Version > the last-merged
watermark + the `write_id` set) to the home leg over `mesh-transport`
(`Node{home}`, streamed — the `kg-mesh` sync frames — scaffold/contracts/kg-mesh.md).
The home leg applies Rules 1–2 transactionally, appends the provenance
union, mints conflict rows, publishes conflict events, snapshots. Boring,
resumable, no new gossip protocol — graphs are personal-scale (10³–10⁵
elements), and a full element-Version map is KBs–MBs, the same
"no Merkle trees" posture as replicated-kv's digest sync.

**The honesty table (what this model does and does not guarantee):**

| Guaranteed | Not guaranteed |
|---|---|
| Deterministic convergence: same element sets ⇒ identical graph on every replica | Semantic merging of concurrent edits to ONE element (LWW picks; audit recovers) |
| No silent structural destruction: dangling/cardinality/body divergence always surfaced as conflicts | Automatic conflict *resolution* (per-application, by design — INTENT #84 pattern) |
| Serialized writes (no conflicts at all) within reach of the home | Write availability when home is unreachable under `Refuse` (the chosen trade) |
| Full forensic recoverability of every overwritten value via provenance union | Cross-element transactional invariants ACROSS a partition (physics) |
| Bounded, opt-in divergence (`Allowed` mints an explicit, receipted branch) | Real-time cross-replica read freshness (staleness = snapshot/sync cadence) |

**Honest scope note vs the kg-mesh stub (deviation, flagged):** the round-3
stub said "mesh owns the eventual-consistency/replication plane that
distributes graph state." As refined here: mesh distributes the *generic*
layers (the `kg/` registry keyspace via replicated-kv; graph-file snapshot
replication via VFS; transport for sync frames) — but the graph-semantic
merge (Rules 1–2) **cannot** be a generic mesh service, because it is
schema-aware (templates, cardinality, quarantine). kg owns the merge; mesh
owns every byte's journey. Recorded as a friction point for the harmonizer
rather than silently rewritten.

### 5. Node→VFS-file pointers + existence validation (the `kg-vfs` edge)

A template declares which props are `FileRefField`s; a FileRef value is a
`vfs://` path plus an optional pinned `content_hash`. Enforcement at three
moments:

1. **Write time:** creating/updating a FileRef calls vfs
   `Exists { path } -> { present, content_hash, size }` (the exact surface
   vfs.md concern-11/proposed-contracts already offers kg — consumed, not
   re-invented). Absent file → `SchemaViolation` push-back — unless the
   graph's template sets `allow_dangling_file_refs: true` (the Obsidian
   forward-link idiom), in which case the pointer is written with
   `pointer_state: pending` in props and swept later.
2. **Sweep time:** a `cron-api` job per graph (`kg.sweep.<graph_id>`,
   cadence `GraphPolicy.pointer_sweep`, default 24h, run on the home node)
   re-validates every FileRef — files can be evicted, deleted, or replaced
   after pointer creation. A broken pointer **flags the node**
   (`pointer_state: broken` + `kg.pointer.broken` event + a conflict row) —
   the node is never auto-deleted. A pinned `content_hash` that no longer
   matches surfaces `pointer_state: stale` (the file changed under the
   pointer) — distinct signal, same non-destructive handling. A pointer
   that re-validates clears its flag.
3. **Durability coupling (advisory):** kg MAY register pointer interest so
   vfs treats pointed-at files as referenced (refcount-adjacent). v1 keeps
   this advisory-only — vfs's refcount is manifest-derived and kg pointers
   are not vfs manifests; a kg pointer does NOT pin a file against
   eviction. Flagged to the harmonizer as a deliberate v1 simplification
   (the sweep catches the consequence).

### 6. Provenance — first-order, healthcare-grade (INTENT #85/#92)

Every write path lands a `kg_provenance` row: who (`actor_service`,
`actor_node`, verified against registration — the anti-spoof rule), what
(op + before/after images), when, and the causal triple
(`causation_id`/`correlation_id` from `types::Provenance`, carried in on
every `kg-api` request and stamped outward on every event kg publishes).
Handler touches add `handler_invocation_id` — the execution-engine's
causal-chain tracking writes through the same table, so "everything that
led to the current state" is answerable per graph with one query: element →
write_id → trace row → causing invocation → causing trigger → causing
element change, recursively. The provenance log is append-only, merges as
a union (concern 4), is included in snapshots, and is exportable per graph
(the per-project/per-database scoping INTENT #92 asks for). Provenance is
NOT optional for graphs — no `Off` level here (that latitude is VFS's,
explicitly not the database plane's).

### 7. Triggers + handlers — the execution-engine's kg-nodes adapter (INTENT #62)

kg compiles in the shared `execution-engine` lib (internal dependency, NOT
a contract edge — locked) and contributes the **kg-nodes subject binding**
to the ONE shared trigger data model queues authored (`types::trigger`,
consumed UNCHANGED per queues.md concern 2's constraint):

```json
{ "graph": "<graph_id>", "subject": "node|edge", "type": "<node_type|edge_type>",
  "op": "insert|update|delete", "old": { …element… }, "new": { …element… },
  "meta": { "write_id", "occurred_at", "provenance": { … } } }
```

Adapter-specific needs (graph/template binding) ride the adapter extension
struct, never bolted onto core `Trigger` — exactly the boundary queues
drew. Triggers for a graph are registered as data via `kg-api`
(`RegisterTrigger`, static-validated on receipt like queues), stored in the
graph's database, and MAY be shipped declaratively inside a
`TemplateVersion` (`shipped_triggers` — e.g. `mind-axiomatic` ships an
index-regeneration trigger); handlers are Deno/TS or SQL code (Deno is the
confirmed runtime), registered/hosted per the execution-engine's design —
kg is a host process, the engine owns invocation, causal-chain tracking,
loop detection (loop-depth on same-element recurrence, NOT naive cascade
depth), and the ccd escalation hook (`ccd-escalation`'s
`LoopDepthExceeded` arm — execution-engine authors it; kg is the host that
fires it).

**Where triggers fire — the distributed rule (subtle, decided):** triggers
evaluate and handlers fire **only on a graph's current write-serialization
point** — the home leg (or a branch's minting leg, for the branch's own
writes, which IS that branch's serialization point). Since every element
change flows through kg's write path (graph databases are kg-owned;
direct-to-database writes are forbidden — see `kg-vdb`), the adapter hooks
the write path in-process; no database-level trigger machinery is needed on
any VDB target. **Merge does not re-fire:** each write carries a
`write_id`, each invocation is recorded against it in provenance; merged-in
elements whose `write_id` already has an invocation record in the unioned
provenance log are skipped — at-most-once firing per write across
branch/merge, by construction. Merge-*created* changes (quarantine flips,
conflict mints) DO fire as their own new writes (op: update, actor:
`kg.merge`) — handlers can react to conflicts, which is exactly the
per-application resolution seam. Cross-node dedup beyond this (e.g. a
handler that must be fleet-unique) rides `locks-api` event-ID semaphores
per trigger, unchanged from the queues/locks composition.

### 8. Environment routing, residence, and promotion (INTENT #97/#98)

At `CreateGraph`, the environment (if any) supplies the default residence:
local environment → `Residence::Local` (home = the creating node unless
pinned; database = a fresh SQLite file at
`vfs://kg/<graph_id>/graph.db`, NodeAnchored, opened through VDB); cloud
environment → `Residence::Cloud` (VDB provisions on the Supabase or
RDS+Lambda target through its adapter matrix). **Promotion** (local →
cloud only, per the locked direction) is one call —
`PromoteGraph { graph_id, target }` — that kg delegates wholesale to VDB's
copy/verify/switch-under-a-lock mechanics (INTENT #86: running handlers
finish against the old database, the database swaps underneath, results
write back, triggering resumes), under `locks-api`
`kg.promote.<graph_id>` with `Strictness::WholeFleet` (a promotion must
never twin — the one kg call site that opts into CP, exactly what locks
built the knob for). After the switch, kg flips
`GraphDescriptor.residence`, and the old SQLite file is retained as a
snapshot then released to normal VFS lifecycle. Demotion (cloud → local)
is out of v1 scope — flagged, not silently included.

### 9. Cross-boundary AWS sync — the Broomstick case (INTENT #50)

"A Broomstick agent chats with a user, extracts their intent into a KG, and
that graph shows up in my mesh." Decomposed:

- **The cloud replica is not special** — it is either (a) a
  `Residence::Cloud` graph (its home IS the cloud database; mesh-side legs
  are readers/writers over VDB's cloud adapter — the local-purpose-KG-
  supporting-cloud-environment nuance, alive and routed), or (b) a
  **cloud branch** of a Local graph: the same generic tables provisioned on
  a cloud target, written by the external agent's stack, merged by the same
  Rules 1–4. One merge model everywhere — the AWS boundary and a mesh
  partition are the same math, which is the whole payoff of concern 4.
- **The sync loop:** per `GraphPolicy.sync: CloudSyncSpec { target,
  interval, direction: Pull | Push | Bidirectional }`, a `cron-api` job on
  the home node pulls element deltas (Version watermark + write_id set —
  concern 4 Rule 4's cursor form) from the cloud database via VDB's
  query/action surface (`kg-vdb` against the cloud target; the network leg
  under it is VDB↔aws's `aws-vdb`, not a kg-owned AWS surface), merges,
  and (if bidirectional) pushes local deltas up. AWS unreachable → the sync
  job logs, emits `kg.sync.deferred`, and retries next tick — never blocks
  anything (the passive-cloud posture aws.md locks).
- **S3 durability is inherited, not built:** the graph's SQLite file is a
  NodeAnchored VFS file — VFS snapshots it, replicates it per factor, and
  overflows/backs it to S3 (client-side encrypted) through `aws-vfs`; the
  `kg/` registry keyspace rides replicated-kv's existing `aws-mesh`
  snapshot leg. **kg therefore needs NO kg-specific S3 protocol** — a
  refinement of the round-3 kg-mesh stub's S3 note, flagged for the
  harmonizer as a simplification, not a dropped requirement.

### 10. The query surface — deliberately boring in v1

`kg-api` reads: get element by id; list/scan by type with prop filters
(the `PropSchema` types make filters statically checkable); neighbors of a
node (by edge type + direction, orphaned excluded by default); bounded
traversal (`Traverse { roots, edge_types, direction, max_depth, limit }` —
depth/limit mandatory, no unbounded walks); subgraph export; conflict list;
provenance/audit queries (by element, by correlation_id). **NO general
graph query language** (no Cypher/Gremlin/Datalog) in v1 — the named
consumers (projects' hierarchy walks, Obsidian link-following, research
traversals, `.mind` index regeneration) are all bounded traversals, and a
query language is the classic unbounded-scope trap (INTENT #38). Ad-hoc
analytics reach the generic tables through `db`'s query/virtualization
direction instead. Flagged as deliberate under-design with the escalation
path named.

## Relationships / edges

Contract edges (cross-process, over the local `:3649` daemon; client halves
via `mesh-client`):

- **vdb** via **`kg-vdb`** (authored; assigned by wave2-plan §3b) —
  graph-database lifecycle: create-from-core-schema, transactional
  mutation/query actions, kg-owned-database marking (no direct writes),
  promotion delegation, restart-ladder inheritance.
  *(authored: scaffold/contracts/kg-vdb.md)*
- **vfs** via **`kg-vfs`** (authored) —
  FileRef existence validation (`Exists`), body-blob reads, sweep
  re-validation. kg's database *files* reach VFS through VDB (`vdb-vfs`),
  not this edge — this edge is pointers only.
- **mesh** via **`kg-mesh`** (authored) —
  registration (NodeScoped), the `kg/` replicated-kv keyspace claim, the
  `kg.*` pubsub topic prefix, and the branch/cloud **merge-sync frames**
  over mesh-transport. The S3 leg is REMOVED from this edge (inherited via
  vdb-vfs/aws-vfs + aws-mesh — concern 9; flagged supersession).
- **any service ↔ kg** via **`kg-api`** (**NEW pair, not in the wave-2
  inventory — surfaced here**; authored: scaffold/contracts/kg-api.md) — the consumer surface:
  templates, graph lifecycle, mutations with push-back, queries, conflicts,
  trigger registration. Modeled surface-schema-style (one document, every
  consumer a party).
- **projects → kg** via `projects-kg` (stub-track, batch 7 authors) — the
  project-structure graph; kg's side is fully expressed by `kg-api` + a
  `project-tree@1` template projects will define; kg-side sketch noted in
  `kg-api` below so batch 7 has a concrete substrate half.
- **ccd** via `ccd-escalation` — kg is a *host* party: the
  execution-engine's `LoopDepthExceeded` arm fires from inside kg's
  process; execution-engine authors the arm (queues.md already carries the
  union shape), kg adds no schema.

Cross-cutting protocols (consumed, not authored): `service-lookup`
(register `kg`, NodeScoped, `meta.requires: [vdb, vfs]` — supervision
boot-order: kg boots after mesh/vfs/vdb), `restart-protocol` (a mid-merge
or mid-migration kg leg reports `CriticalSection`/`FinishAndRelinquish`;
graph databases beneath inherit the ladder via VDB), `pubsub-protocol`
(claims the `kg.*` topic prefix: `kg.graph.created`, `kg.node.*`,
`kg.edge.*`, `kg.conflict.*`, `kg.pointer.broken`, `kg.merge.*`,
`kg.sync.*` — additive taxonomy row, flagged for the harmonizer),
`surface-schema` (graphs list + per-graph health/conflict counts/sync lag;
`DisplayHints` feed the dashboard's graph rendering), `locks-api`
(`kg.migrate.*`, `kg.merge.*`, `kg.promote.*` [WholeFleet], per-trigger
event semaphores), `cron-api` (pointer sweeps, cloud sync ticks).

Internal-lib seams (compiled in, NOT contract edges): `execution-engine`
(kg-nodes adapter — the locked shared lib), `mesh-client`,
`substrate-types` (Provenance/Event/trigger vocabulary + the new
`types::kg` module authored in the contracts).

## Nesting

Parent: none (top-level app-crate `bin/kg` + `lib/kg`). Children (nested
internal libs, compiled into the kg daemon, never standalone — INTENT #22):
**`catalog`** (registry + template store over the `kg/` keyspace),
**`schema`** (PropSchema validation engine + push-back diagnostics),
**`store`** (the generic graph-relational layer speaking `kg-vdb`),
**`merge`** (Rules 1–4: element LWW, repair, conflict mint, branch/cloud
sync loops), **`exec`** (execution-engine hosting + the kg-nodes subject
binding), **`api`** (the `kg-api` surface + query engine + surface schema).
Flat `components/` naming per the scaffold convention; the tree lives here
and in overview.md.

## Thoroughness level

**implementation-ready** for: the one-graph-one-database decision (concern
1); the `kg/` keyspace registry + descriptor/policy shapes (concern 2); the
template model, the three built-in templates' type sets, validation
push-back semantics, and the migration protocol (concern 3); the generic
core schema (3b); the v1 consistency model — element-LWW + repair +
conflict surfacing + mint-on-partition + the honesty table (concern 4);
FileRef validation moments and non-destructive breakage handling (concern
5); the provenance table + union-merge (concern 6); the kg-nodes subject
binding, fire-at-serialization-point rule, and write_id dedup (concern 7);
residence/promotion flow (concern 8); the cloud-sync decomposition
(concern 9); and the bounded query surface (concern 10).
**approach-sketched** for: the exact `kg-vdb` action verbs (VDB's design
lands in this same batch — mid-batch reconciliation required; kg's
requirements on it are pinned below); the branch-replica snapshot mechanics
(depends on vdb-vfs checkpoint semantics, same reconciliation); the
`CloudSyncSpec` delta-query shape against Supabase/RDS (rides VDB's
adapter matrix); and tuning constants (sweep cadences, sync intervals,
traversal limits — fill-time, with defaults proposed).

## Assigned design-depth

**Fable** (wave2-plan batch-4 Fable seat), single Component-Designer pass
(this file), grounded in the round-8/9 kg/stack locks, INTENT
#50/#62/#84/#85/#92/#93/#96/#97/#98, and the batch-1/2/3 designs read in
full (types, replicated-kv, locks, queues, mesh-core, vfs, aws, secrets,
supervision, db).

## Suggested fill-model

**implementation-ready + high complexity → strong-mid model, with the merge
suite written FIRST.** Clean risk split: (a) `catalog`/`schema`/`api` —
declarative validation, KV records, bounded queries — are near-spec and
heavily testable → **mid model** with conformance fixtures. (b) `store` —
generic tables + transactional batches through `kg-vdb` → **mid model**,
fill AFTER vdb lands its surface. (c) **`merge` is the one
do-not-cheap-out surface**: element LWW + quarantine + conflict mint +
provenance union + write_id trigger dedup + branch lifecycle — write the
non-obvious tests below as an executable suite against a two-leg in-process
harness (two kg stores, scripted divergence schedules, assert bit-identical
convergence + every-conflict-surfaced + no-double-fire) BEFORE
implementing; property-style random partition/merge schedules are the
highest-value fill artifact, same posture as locks. (d) `exec` is mostly
the shared engine's — kg's binding glue is transcription-grade. Fill after:
replicated-kv, locks, vfs, vdb, execution-engine.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `kg-vdb` (kg ↔ vdb) — the locked-layering edge (KG built ON VDB). → `scaffold/contracts/kg-vdb.md`
- `kg-vfs` (kg → vfs) — node→file pointers + existence validation (pointers only). → `scaffold/contracts/kg-vfs.md`
- `kg-mesh` (kg ↔ mesh) — registration + graph replication (S3 leg rides aws). → `scaffold/contracts/kg-mesh.md`
- `kg-api` (any service ↔ kg) — the consumer surface (added by the contract round as a deviation-by-addition). → `scaffold/contracts/kg-api.md`

Also a party to (authored elsewhere / cross-cutting): `ccd-escalation`, `kv-replication`, `projects-kg`, `service-lookup`, `vdb-vfs`, `vfs-content` — see `scaffold/contracts/`.

Component-side notes:
- `ccd-escalation` participation: kg hosts the engine's `LoopDepthExceeded` arm
  (execution-engine authors it); kg contributes `context` enrichment only —
  graph_id, element ids, the causal chain slice from `kg_provenance` (not
  captured in the contract file).
- `projects-kg`: kg's half is `kg-api` verbatim + the projects-authored
  `project-tree@1` template (recorded in the contract file).

## Non-obvious tests (conformance + correctness)

- **Bit-identical convergence:** two replicas of one graph, scripted
  divergent write schedules, merged in both directions → identical element
  sets, identical conflict sets, identical provenance unions (order-free).
- **Dangling-edge quarantine, not cascade:** side A deletes node X while
  side B adds edge Y→X; merge → Y→X is `orphaned` + conflict row + event,
  never deleted; a later un-delete of X re-lives the edge and resolves the
  conflict row.
- **Cardinality twins surfaced:** template `AtMostOneOutgoing conclusion`;
  both branches add one; merge keeps both, mints `cardinality` conflict;
  deleting either (an ordinary write) clears it — no resolve verb.
- **No double-fire across merge:** a branch write fires its trigger on the
  branch; after merge, the home does NOT re-fire it (write_id found in the
  unioned provenance) — but a merge-minted quarantine flip DOES fire as a
  new write with actor `kg.merge`.
- **LWW loser recoverable:** concurrent prop edits, one wins; `Audit` by
  the element id reconstructs the losing value from before/after images —
  the forensic-safety property of concern 4 Rule 2.
- **Push-back atomicity:** a 10-op `Mutate` with one bad element writes
  nothing and returns per-element violations; the graph's Version watermark
  is unchanged.
- **Migration rejected whole:** template v2 narrows a prop; one
  nonconforming node → `MigrationRejected` with a full report; graph still
  binds v1 and serves reads throughout.
- **Broken pointer is a flag, not a delete:** vfs evicts a pointed-at
  file; the sweep flags `broken` + event; the file reappears (re-written,
  same path) → next sweep clears; a pinned-hash mismatch yields `stale`,
  distinctly.
- **Refuse vs Allowed honesty:** home unreachable — `Refuse` graph returns
  typed `GraphHomeUnreachable` (reads still serve, marked stale);
  `Allowed` graph mints a branch and every ack carries
  `minted_branch: true`; reconnect merges and retires the branch.
- **Broomstick round-trip:** elements written into a cloud-target replica
  (via VDB's Supabase/RDS adapter) are pulled by the cron sync, merged,
  and appear mesh-side with cloud-side provenance intact; a concurrent
  mesh-side edit to the same element surfaces exactly one conflict, not
  silent loss; aws unreachable → `kg.sync.deferred`, nothing blocks.
- **Promotion under WholeFleet:** promotion during a partition fails fast
  (`FleetNotFullyReachable`) — never twins; a successful promotion lets
  in-flight handlers finish against the old database before the switch
  (INTENT #86 choreography, asserted at the kg-vdb seam).
- **Mixed-version delta deferral:** an older home receiving a newer
  `kg_proto` MergeOffer defers cleanly (alarmed, retriable), never
  half-applies.
