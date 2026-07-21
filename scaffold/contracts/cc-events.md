# Contract: cc-events

> *Renamed from `ccd-events` at friction-round 3 (INTENT #131, ccd → cc).*

## Parties
- **Producer:** `cc` (the Cloud Code Daemon, L5) — publisher of the `cc.*`
  EventType catalog.
- **Consumer:** `mesh` (`dashboard-serving`, L2) — the observability plane that
  subscribes per-node and folds the stream into the browser `dashboard-feed`.
  Any other mesh client (e.g. `org`) may also subscribe.

*(was `gateway <- cc`; gateway merged into mesh, 2026-07-18)*

## Purpose
cc emits an agent-lifecycle / run / usage / budget / escalation event stream
that mesh's observability plane aggregates, mirroring `inference-events` and
`gc-events`. Wave-2 re-grounds this edge: it is **not** an independent
gateway-scraped WS surface but an **EventType catalog published on the reserved
`cc.*` topic prefix over `pubsub-protocol`** (the same collapse
`inference-events`/`gc-events` adopt). This file owns only cc's catalog table
and payload shapes; the wire wrapper (`Envelope<Event<P>>`), the relay, and the
lossy delivery semantics belong to `pubsub-protocol`.

**Round-9 resolution of the long-standing marker:** the wave-1
"PROPOSED by the cc Component Designer — pending Decomposer / operator
confirmation" marker is **STRUCK**. The edge is CONFIRMED live and in scope
(wave2-plan §5 flag #6; cc.md batch-6 pass). Batch designs (`dashboard-serving`,
`dashboard`) already assume it; this contract confirms it. See Reconciliation
notes.

## Schema

cc is a plain `pubsub-protocol` publisher. It publishes
`types::pubsub::Envelope<types::event::Event<CcEventPayload>>` frames on topics
under the reserved prefix `cc`. Topic convention is `cc/<node_id>/<domain>`
(hierarchical, "/"-delimited — `types::Topic`); `event_type` follows the
`domain.noun.verb` convention (`types::EventType`, an open namespaced string).

```rust
// types::cc — the CcEventPayload catalog (P in Event<P>)
// event_type identifiers cc guarantees to emit:
//   "cc.agent.started" | "cc.agent.turn_completed" | "cc.agent.report_emitted"
//   | "cc.agent.state_changed" | "cc.agent.exited"
//   | "cc.budget.throttled" | "cc.budget.deferred" | "cc.limit.observed"
//   | "cc.escalation.received" | "cc.escalation.investigating"
pub enum CcEventPayload {
    Agent(AgentEvent),                 // reuses agent-management's AgentEvent verbatim
    Budget {                           // admission-engine verdict, observable for the dashboard
        verdict: AdmissionOutcome,     // Admitted | Throttled | Deferred | Denied
        window: String,                // "session" | "weekly" | "per-model:<model>"
        pressure: f64,                 // used / ceiling, 0.0..=1.0+
    },
    LimitObserved { surface: String, limit_tokens: u64 },  // a limit newly bound from the ledger
    Escalation {                       // mirror of a cc-escalation receipt, for observability
        escalation_id: Uuid,
        kind_tag: String,              // "dead_letter" | "loop_depth_exceeded"
        thread: Option<ThreadId>,      // set once an investigating agent is spawned
    },
}

// AgentEvent — authored in agent-management, reused here as the Agent(..) arm:
pub enum AgentEvent {
    Started       { handle: AgentHandle },
    TurnCompleted { handle: AgentHandle, usage: TurnUsage },
    ReportEmitted { handle: AgentHandle, path: PathBuf },
    StateChanged  { handle: AgentHandle, state: AgentState },
    Exited        { handle: AgentHandle, code: Option<i32> },
}
```

The `Envelope`/`Event` wrapper is `types` vocabulary (unchanged): every frame
carries `provenance.origin_node` (the emitting node's `NodeId`), which the
dashboard reads for its per-node grid — cc never invents or re-stamps a
`node_id`.

**Subscriber contract (mesh side).** `dashboard-serving` opens ONE subscription
per node with `TopicFilter::Prefix("cc")` and folds every `Delivery` into the
`GET /events` fan-out unparsed (the relay never inspects `payload`). A focused
per-agent view uses `TopicFilter::Prefix("cc/<node>/agent")`.

## Error cases
- **Emit path: NONE.** Emission is lossy broadcast (INTENT #53) — a dropped
  event never blocks agent work. If no subscriber is present the publish is a
  no-op; if a subscriber lags, `pubsub-relay` emits `Lagged { dropped }` on ITS
  side, not cc's.
- **Unknown `event_type` on the topic:** passed through opaque to the browser
  (the relay does not parse payloads); never an error, never dropped.
- cc itself owns no request/response error type on this edge (`CcError` is for
  the `agent-management` control surface, not the event stream).

## Version sensitivity
**MEDIUM**, but the wire-crossing risk is delegated to `pubsub-protocol`
(payload-opaque decoupling: a new `cc.*` event type crosses an older daemon
untouched).
- **Additive-safe:** new `event_type` identifiers; new `CcEventPayload` arms
  (`#[serde(other)]` reserved so an older `dashboard` deserialize tolerates a
  future kind); new `#[serde(default)]` fields on existing arms; new topic
  subtrees under `cc/`.
- **Breaking:** removing/renaming a published `event_type`, changing an existing
  arm's field types, or changing the topic-prefix reservation `cc`.
- The `Envelope.v` / `Event` shape is governed by `types`' wire discipline
  (guardrail 4); cc publishes at the current `v` and never straddles two.

## Reconciliation notes
- **The "proposed, pending confirmation" marker is resolved to CONFIRMED.** Both
  the wave-1 stub and wave2-plan §5 flag #6 carried it as an open question.
  Deciding side: **confirm live.** Rationale: (a) `dashboard-serving.md` and
  `dashboard.md` (batch 6) both already build against a `cc.*` stream as a peer
  of `inference.*`/`gc.*` — striking it would orphan two authored designs;
  (b) INTENT #68 makes cc a first-class observable service with its own usage
  ledger, and an observability plane that shows inference/gc but not agents would
  be incoherent; (c) the edge introduces NO new wire — it is one more publisher
  on the already-locked `pubsub-protocol`, so the cost of confirming is zero.
  No party argued to strike it; the marker was inertia from the wave-1
  gateway-scrape framing, which the pubsub re-grounding dissolves. **Losing
  position (for the record):** "leave it pending until an operator explicitly
  asks for agent telemetry on the dashboard" — rejected because the batch-6
  designs already consumed it and INTENT #68/#16 (clean agent interface, cc owns
  its data) implies it.
- **Re-grounded from a WS surface to a pubsub catalog.** Deviation from the
  wave-1 stub ("WS event stream + REST proxy"): per `dashboard-serving.md`'s
  proposal, this is now a topic-prefix + EventType-catalog pointer into
  `pubsub-protocol`, not a parallel wire. The REST-proxy affordance is subsumed
  by `dashboard-feed`'s `ANY /api/nodes/:id/cc/*` same-origin proxy.
- **`AgentEvent` is owned by `agent-management`, not duplicated here.** This
  contract references it as the `CcEventPayload::Agent` arm so the lifecycle
  vocabulary has a single home.

## Example data
A Claude-Code investigation agent runs on **macbook** for project **demo**. cc
publishes on topic `cc/macbook/agent` an `Envelope<Event<CcEventPayload>>`:

```json
{
  "v": 1,
  "envelope_id": "6f1c-a1",
  "topic": "cc/macbook/agent",
  "published_at": "2026-07-19T18:04:11Z",
  "provenance": { "origin_node": "macbook", "origin_service": "cc",
                  "emitted_at": "2026-07-19T18:04:11Z",
                  "correlation_id": "c0-demo-run-1" },
  "payload": {
    "event_id": "e-8801",
    "event_type": "cc.agent.turn_completed",
    "occurred_at": "2026-07-19T18:04:11Z",
    "provenance": { "origin_node": "macbook", "origin_service": "cc" },
    "payload": { "Agent": { "TurnCompleted": {
        "handle": "agent-demo-01",
        "usage": { "input_tokens": 4120, "output_tokens": 880, "model": "claude" }
    } } }
  }
}
```

A budget event on the same node (topic `cc/macbook/budget`,
`event_type: "cc.budget.throttled"`):

```json
{ "payload": { "Budget": {
    "verdict": { "Throttled": { "token_ceiling": 30000, "max_parallel": 2,
                                "strategy_id": "default", "expires_at": "2026-07-19T18:45:00Z" } },
    "window": "session", "pressure": 0.86 } } }
```

`dashboard-serving` on macbook subscribes `Prefix("cc")`, relays both frames
unparsed to the browser, which renders them on the cc panel keyed by
`origin_node = "macbook"`. (For the escalation-received event that composes with
this stream, see `cc-escalation.md` — same `correlation_id` `c0-demo-run-1`.)
