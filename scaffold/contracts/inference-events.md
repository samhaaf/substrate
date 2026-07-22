# Contract: inference-events

## Parties
mesh  <-  inference (api)

- **inference (api)** is the **publisher** — it emits per-node lifecycle +
  throughput events onto mesh pub/sub via `chassis` (formerly `mesh-client`;
  api.md proposal).
- **mesh.completion-router** is a **consumer** — this is its live-primary
  load/inventory feed for the `NodeRegistry` projection (completion-router.md
  concern 3).
- **dashboard-serving** is the other consumer — the observability fan-out.
- *(Was `gateway <- inference`; gateway merged into mesh, 2026-07-18, INTENT #45.)*

## Purpose
The per-node event stream carrying inference's externally-visible lifecycle and
throughput deltas: which model is resident, queue depth, execution paused/resumed,
per-completion throughput samples, and backend observability. It rides the
standard WS pub/sub protocol (INTENT #53) — typed `Event<P>` payloads inside a
`types::pubsub::Envelope`, relayed by mesh "where it needs to go." The router
folds the load-carrying subset into fresh least-loaded/affinity decisions; drift
is reconciled off-band by `node-state-poll` (this feed is lossy by contract).

## Schema

Published as `Envelope<Event<P>>` on topic `inference/<node>/<kind>`, scope
`Fleet` (all subscribing daemons receive it). Vocabulary from
`types::{pubsub, event}`:

```rust
// types::pubsub::Envelope<P>
struct Envelope<P> {
    v: u16, envelope_id: Uuid, topic: Topic,
    published_at: DateTime<Utc>, provenance: Provenance, payload: P,
}
// payload P = types::event::Event<PayloadT>
struct Event<PayloadT> {
    event_id: Uuid, event_type: EventType,          // open "domain.noun.verb" id
    occurred_at: DateTime<Utc>, provenance: Provenance, payload: PayloadT,
}
```

### Event kinds (`event_type` string → payload)

**Load-carrying (the router depends ONLY on these five):**

```rust
// inference.queue.depth_changed  -> the AUTHORITATIVE (pending, running) snapshot
struct QueueDepth       { pending: u32, running: u32 }
// inference.model.loaded         -> resident = model_id ; downloaded ∪= {model_id}
struct ModelLoaded      { model_id: ModelId }
// inference.model.evicted        -> resident/downloaded update
struct ModelEvicted     { model_id: ModelId }
// inference.execution.paused     -> node holds admission (selection weight = 0)
struct ExecutionPaused  {}
// inference.execution.resumed    -> node admits again
struct ExecutionResumed {}
```

**Observability (consumed by the dashboard; router ignores):**

```rust
// inference.throughput.sample
struct ThroughputSample { model_id: ModelId, output_tokens: u32, tokens_per_second: f32 }
// inference.completion.*   (per-completion lifecycle, observability granularity)
struct CompletionLifecycle { id: CompletionId }     // started | finished (success|failure)
// inference.backend.*      (ready | stopped | installing | ...)
struct BackendEvent { detail: BackendDetail }
```

## Error cases

- **Lossy by pub/sub contract.** A `Lagged` notice or a dropped delta on the
  local `chassis` (formerly `mesh-client`) connection triggers **no api action**
  — the router reconciles via `node-state-poll` (completion-router concern 3).
  A `seq` gap in pubsub provenance likewise triggers a reconcile poll of that
  node, never a guess.
- **Local daemon down at publish time.** `chassis` buffers best-effort
  (bounded, drop-oldest). In standalone mode inference falls back to the raw
  `/events` WS (api concern 4); no event is an error.
- **No delivery guarantee, no ack semantics** beyond the pub/sub layer's
  `PubSubServerMsg::Ack` (envelope receipt at the local relay only).

## Version sensitivity

- **Relay: LOW.** The relay is payload-opaque (pubsub-relay concern 2), operating
  on `Envelope<serde_json::Value>`; it must not `deny_unknown_fields`. An unknown
  `EventType` string routes/filters as an opaque event.
- **Consumers: additive-safe.** Consumers match on the open `event_type` string
  and ignore unknown kinds — a **new inference event kind never breaks a
  consumer**. New optional payload fields are additive (`#[serde(default)]`).
- **Breaking:** removing a load-carrying kind, changing a payload field's
  meaning/type, or renaming an `event_type` the router depends on. `Envelope.v`
  is the wire anchor.
- **Conformance:** api MUST emit the five load-affecting kinds
  (`queue.depth_changed`, `model.loaded`, `model.evicted`, `execution.paused`,
  `execution.resumed`); all other kinds are observability-only.

## Reconciliation notes

1. **Running-count delta transport — RESOLVED in favor of api (absolute
   snapshot).** *Disputed:* how the router learns the live `running` count.
   - **api's position (WON):** reuse `inference.queue.depth_changed`, which
     carries the **absolute** `{pending, running}` snapshot (matches the live
     `LifecycleEvent::QueueDepthChanged`, emitted after every submit/completion).
   - **completion-router's position (LOST):** dedicated
     `inference.completion.started` (`running += 1`) / `inference.completion.
     finished` (`running -= 1`) incremental deltas.
   - **Why api wins:** the feed is lossy (pub/sub, INTENT #53). An **absolute**
     `{pending, running}` snapshot **self-heals** on the very next event — a
     dropped frame just delays the update. **Incremental** `+=1/-=1` deltas
     accumulate permanent skew on any dropped `finished`, forcing a reconcile
     poll to repair — precisely the fragility the poll was demoted away from
     carrying. Absolute counts are strictly more robust on a lossy transport,
     and reuse a kind that already exists (INTENT #38, no redundant surface).
   - **Kept from the losing side:** per-completion `inference.completion.
     started/finished` events **remain emitted as observability** (dashboards
     want per-completion granularity), but the **router depends only on
     `queue.depth_changed`** for its running count. Nothing is dropped; only the
     router's dependency is pinned to the loss-tolerant kind.

2. **Topic shape (agreed).** `inference/<node>/<kind>` with scope `Fleet`; the
   router subscribes with a `Prefix("inference/")` filter, the dashboard likewise.
   Per-completion token streams do **not** ride this feed — they are the
   `v1-completion-api` WS relay (per-completion-ID subscription, INTENT #5), a
   distinct path.

3. **Publisher ownership (agreed).** The contract is owned by inference/api
   (api.md); completion-router.md consumes it and only flags the kinds it needs.
   No schema conflict — merged.

## Example data

**World:** nodes `macbook` and `pi`; model `qwen3-4b`; project `demo`; completion
`7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f` (the same one submitted in
`v1-completion-api`, running on `pi`).

`pi` loads `qwen3-4b`, admits the completion (queue depth ticks), samples
throughput on finish. The router on `macbook` receives these over the fleet relay
and updates its `NodeRegistry` projection of `pi`.

```jsonc
// 1) pi loads the model
{
  "v": 1,
  "envelope_id": "e1000000-0000-4000-8000-000000000001",
  "topic": "inference/pi/model.loaded",
  "published_at": "2026-07-19T16:59:58Z",
  "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T16:59:58Z" } ] },
  "payload": {
    "event_id": "a1000000-0000-4000-8000-000000000001",
    "event_type": "inference.model.loaded",
    "occurred_at": "2026-07-19T16:59:58Z",
    "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T16:59:58Z" } ] },
    "payload": { "model_id": "qwen3-4b" }
  }
}
```

```jsonc
// 2) admission — absolute queue snapshot (the router's authoritative load read)
{
  "v": 1, "envelope_id": "e1000000-0000-4000-8000-000000000002",
  "topic": "inference/pi/queue.depth_changed",
  "published_at": "2026-07-19T17:00:00Z",
  "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T17:00:00Z" } ] },
  "payload": {
    "event_id": "a1000000-0000-4000-8000-000000000002",
    "event_type": "inference.queue.depth_changed",
    "occurred_at": "2026-07-19T17:00:00Z",
    "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T17:00:00Z" } ] },
    "payload": { "pending": 0, "running": 1 }
  }
}
```

```jsonc
// 3) throughput sample on finish (observability; dashboard renders it)
{
  "v": 1, "envelope_id": "e1000000-0000-4000-8000-000000000003",
  "topic": "inference/pi/throughput.sample",
  "published_at": "2026-07-19T17:00:01Z",
  "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T17:00:01Z" } ] },
  "payload": {
    "event_id": "a1000000-0000-4000-8000-000000000003",
    "event_type": "inference.throughput.sample",
    "occurred_at": "2026-07-19T17:00:01Z",
    "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T17:00:01Z" } ] },
    "payload": { "model_id": "qwen3-4b", "output_tokens": 22, "tokens_per_second": 18.3 }
  }
}
```

```jsonc
// 4) queue drains after the completion finishes (absolute snapshot self-heals)
{
  "v": 1, "envelope_id": "e1000000-0000-4000-8000-000000000004",
  "topic": "inference/pi/queue.depth_changed",
  "published_at": "2026-07-19T17:00:01Z",
  "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T17:00:01Z" } ] },
  "payload": {
    "event_id": "a1000000-0000-4000-8000-000000000004",
    "event_type": "inference.queue.depth_changed",
    "occurred_at": "2026-07-19T17:00:01Z",
    "provenance": { "origin_node": "pi", "hops": [ { "node": "pi", "at": "2026-07-19T17:00:01Z" } ] },
    "payload": { "pending": 0, "running": 0 }
  }
}
```
