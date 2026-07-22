# Contract: cc-escalation

> *Renamed from `ccd-escalation` at friction-round 3 (INTENT #131, ccd → cc).*

## Parties
- **Producers (two, one union shape):**
  - `mesh.queues` (L2) — authors the `DeadLetter` arm (dead-letter exhaustion).
  - `execution-engine` (L4, embedded in `vdb`/`kg`, reaching mesh through its
    host's `chassis`, formerly `mesh-client`) — authors the `LoopDepthExceeded`
    arm.
- **Consumer / receiver:** `cc` (L5) — owns the receiver, the dedup ledger, and
  `EscalationAck`.

## Purpose
The **single investigation surface** for the two guardrails-of-last-resort that
INTENT #70 and #89 treat as one pattern: queues' **dead-letter exhaustion** and
the execution-engine's **loop-depth-exceeded**. Rather than two escalation
surfaces, both conditions arrive at cc as ONE `EscalationRequest` union so cc
grows exactly one investigation receiver. On receipt cc assembles an
investigation plugin (via `rollup-cc`), spawns a high-priority investigating
Claude-Code agent through its ordinary admission engine, and threads the
`correlation_id`/`context` so the agent starts warm.

Delivery is the ordinary mesh queue fabric (`queues-api`, at-least-once,
durable), so the escalation is itself a `SendEvent` toward a standing declarative
trigger whose `HandlerRef::Service { slug: "cc" }` delivers it — the guardrail
is declarative data, consistent with every other trigger.

## Schema

```rust
// authored jointly; envelope owned by queues, arms independently owned:
pub struct EscalationRequest {
    pub escalation_id: Uuid,            // idempotency key; dedup at cc (INTENT #95)
    pub kind: EscalationKind,
    pub correlation_id: Option<Uuid>,   // Provenance root — the causal chain to investigate
    pub provenance: Provenance,         // types::Provenance
    pub context: serde_json::Value,     // failure_history / pre-rendered lineage; agent starts warm
}

pub enum EscalationKind {
    // ── queues authors this arm ──
    DeadLetter {
        queue: QueueName, dlq: QueueName, trigger_id: TriggerId,
        event_id: Uuid, receive_count: u32, last_error: String,
    },
    // ── execution-engine authors this arm (enriched from queues' thin placeholder) ──
    LoopDepthExceeded {
        host: Slug,                     // "vdb" | "kg" — the embedding app
        adapter: AdapterKind,           // Tables | Graph
        database: DbRef,                // the provenance home to investigate
        subject: SubjectKey,            // the row/node the loop orbits
        trigger_id: TriggerId,
        handler: String, handler_version: String,
        loop_count: u32, threshold: u32,// counts[(subject,handler)] at park time
        chain_recent: Vec<ChainLink>,   // last K causal links, verbatim
        parked_invocation: Uuid,        // the invocation parked, not run
        provenance_root: Uuid,          // correlation_id — walk ee_* tables from here
    },
    // #[serde(other)] reserved so a future third kind never breaks cc's deserialize
}

// ── cc owns this ──
pub enum EscalationAck {
    Investigating { thread: ThreadId }, // an investigating agent was spawned
    Declined { reason: String },        // e.g. hard budget pressure — DLQ retains, re-delivers
}
```

Shared vocabulary promoted to `types`: `SubjectKey`, `ChainLink`, `AdapterKind`,
`DbRef` (execution-engine's inclusion test passed — used by both engine and cc's
receiver). `QueueName`/`TriggerId` are `types::queue` vocabulary; `Slug`,
`Provenance`, `ThreadId` are existing `types`.

**Receiver behavior (cc, owned):**
1. Dedup on `escalation_id` against the `escalations` table — an escalation may
   arrive twice under at-least-once (INTENT #95); the second is a no-op returning
   the same `EscalationAck`.
2. On a new escalation: assemble the investigation plugin via `rollup-cc`, spawn
   a high-priority investigating agent through the ordinary admission engine,
   thread `correlation_id`/`context`, return `Investigating { thread }`, and
   emit `cc.escalation.investigating` on `cc-events`.
3. If the strategy defers under hard budget pressure: return `Declined { reason }`.
   The escalation is **retained** (re-delivers / stays on the DLQ) — never lost.

**Producer delivery contract:** each producer mints `escalation_id`
deterministically so a hot loop / repeatedly-dead-lettered event yields ONE
investigation:
- queues: derived from `(dlq, event_id)`.
- execution-engine: `uuid_v5(correlation_id, loop_key)` where
  `loop_key = (subject_key, handler)`.

## Error cases
- **`CcUnreachable`** (cc not registered / offline): the escalation *event*
  itself dead-letters onto its own queue's retention, alarmed, **never silently
  lost**. It re-delivers when cc returns; `escalation_id` dedups the retry.
- **`Declined { reason }`** under hard budget pressure: not an error — the
  contract's honest back-pressure signal; the DLQ retains the escalation.
- **`DatabaseUnavailable`** (execution-engine arm only): the `database` was
  promoted/moved between park and investigation. Conformance requirement: the
  arm's `chain_recent` + `context` snapshot MUST be self-sufficient for a
  first-pass diagnosis; the authoritative `ee_*` evidence is best-effort.
- Duplicate arrival → `escalation_id` dedup (exactly one investigating agent).

## Version sensitivity
**MEDIUM** (both producers concur).
- **Additive-safe:** new `EscalationKind` arms (`#[serde(other)]` reserved so cc
  tolerates a future third kind); new `#[serde(default)]` fields within an arm
  (a cc built against queues' thin `LoopDepthExceeded` sketch still deserializes
  the enriched arm); the open `context: Value` absorbs richer investigation
  payloads without a wire change.
- **Breaking:** changing `escalation_id`/`correlation_id` semantics, removing an
  arm, or changing an existing field's type.
- `SubjectKey`/`ChainLink` land as `types` structs under guardrail-4 wire
  discipline.

## Reconciliation notes
- **`LoopDepthExceeded` arm: thin sketch vs enriched.** DISAGREEMENT (shape).
  queues.md sketched the arm as a placeholder `{ engine: Slug, depth: u32,
  threshold: u32 }`; execution-engine.md replaced it with a 11-field enriched
  arm (`host`/`adapter`/`database`/`subject`/`handler`/`chain_recent`/…).
  **Winner: execution-engine's enriched arm.** Rationale: queues explicitly
  labelled its version a placeholder and assigned the arm's authorship to
  execution-engine ("execution-engine authors this arm (batch 4) — named here so
  the union is one shape"); the arm's owner is the party that has the failure
  context, and a bare `{depth, threshold}` cannot seed a warm investigation (no
  subject, no provenance root, no recent chain). Field rename `engine → host` is
  adopted from the owner. **Losing position (recorded):** queues' thin
  `{engine, depth, threshold}` — kept only as the `#[serde(default)]`
  back-compat floor so a cc compiled against the sketch still deserializes.
- **`EscalationAck::Investigating` thread type: `String` vs `ThreadId`.**
  DISAGREEMENT (minor). queues.md and execution-engine.md both wrote
  `thread: String`; cc.md wrote `thread: ThreadId`. **Winner: `ThreadId`**
  (cc owns `EscalationAck`, and `ThreadId` is cc's ledger-key vocabulary —
  a bare `String` would drop the type that `cc-projects`/`spend-cc` join on).
  `ThreadId(pub String)` is wire-identical, so this is a type-level tightening,
  not a wire break.
- **Ownership line (agreed by all three, restated):** queues authors
  `DeadLetter`, execution-engine authors `LoopDepthExceeded`, cc owns the
  receiver + `EscalationAck`. This file is the union of those three proposals;
  no arm was dropped.
- **Delivery is `queues-api`, not a bespoke transport.** The execution-engine
  builds no escalation transport of its own — both escape paths (DLQ,
  loop-park) converge on this one `SendEvent`-shaped edge.

## Example data
On **macbook**, a `vdb`-hosted stack handler for project **demo** enters a hot
loop; the execution-engine parks invocation 4 and escalates. It `SendEvent`s onto
the mesh queue fabric an `EscalationRequest` (`event_type: "ee.loop.exceeded"`):

```json
{
  "escalation_id": "f5a2-loopkey-1",
  "kind": { "LoopDepthExceeded": {
      "host": "vdb", "adapter": "Tables", "database": "demo/main",
      "subject": { "Row": { "database": "demo/main", "table": "orders", "pk": "42" } },
      "trigger_id": "trg-orders-reprice", "handler": "reprice", "handler_version": "1.3.0",
      "loop_count": 5, "threshold": 4,
      "chain_recent": [ { "invocation": "inv-3", "handler": "reprice" },
                        { "invocation": "inv-4", "handler": "reprice" } ],
      "parked_invocation": "inv-4",
      "provenance_root": "c0-demo-run-1" } },
  "correlation_id": "c0-demo-run-1",
  "provenance": { "origin_node": "macbook", "origin_service": "vdb",
                  "correlation_id": "c0-demo-run-1" },
  "context": { "summary": "reprice handler re-fired 5x on orders/42 without convergence" }
}
```

The standing `cc` trigger delivers it; cc (running on macbook) dedups on
`escalation_id`, assembles an investigation plugin, spawns a high-priority agent,
and replies:

```json
{ "Investigating": { "thread": "thr-invest-77" } }
```

cc then emits on `cc-events` (topic `cc/macbook/escalation`):
`{ "Escalation": { "escalation_id": "f5a2-loopkey-1",
   "kind_tag": "loop_depth_exceeded", "thread": "thr-invest-77" } }`.
Under hard weekly budget pressure the same escalation would instead return
`{ "Declined": { "reason": "weekly budget above 75th percentile" } }` and stay on
the DLQ for re-delivery. A `DeadLetter` escalation from `queues` on **pi** looks
identical in envelope, differing only in the `kind` arm:
`{ "DeadLetter": { "queue": "demo-jobs", "dlq": "demo-jobs-dlq",
   "trigger_id": "trg-ingest", "event_id": "ev-9001", "receive_count": 6,
   "last_error": "handler timeout" } }`.
