# Contract: v1-completion-api

## Parties
client / mesh.completion-router  <->  inference (api)

- **inference (api)** is the **terminus** — it owns the `/v1/` surface schema,
  status codes, and `StreamEvent` shapes (api.md proposal).
- **mesh.completion-router** is the **forwarding** party — it selects a node (or
  honors a pin), copies status+headers+body byte-for-byte, and relays the WS
  stream. It transforms nothing (completion-router.md proposal).
- Downstream typed consumers of this same surface: `cc`/future `agents` (see
  `llm-calls`), `org`, and the dashboard.

## Purpose
The single external completion surface of an inference node: submit / status /
cancel / re-prioritize / result / token-stream for completions, plus collections,
models, and estimate. Mesh forwards it **byte-transparently** so a caller cannot
tell a node-direct call from a mesh-routed one — the same `/v1/` request works
whether it lands on the local node or is relayed to a peer. This is the one
completions entry point; `llm-calls` (cc/agents) reuses it rather than adding a
second surface (INTENT #40, kept "cheap to fan out" for org).

## Schema

### REST + WS route surface (owned by api; opaque to the router)

Grounded in the live `rest.rs`/`ws.rs`, re-affirmed for wave-2:

```
POST   /v1/completions               -> 201 { id: CompletionId }
GET    /v1/completions                -> 200 [ CompletionSummary ]      (?state=&limit=)
GET    /v1/completions/:id            -> 200 CompletionSummary | 404
DELETE /v1/completions/:id            -> 204 | 404 | 409 (terminal)
PATCH  /v1/completions/:id/priority   { priority: i32 } -> 204 | 409
GET    /v1/completions/:id/result     -> 200 CompletionResult | 409 (not terminal) | 404
GET    /v1/completions/:id/stream     -> WS StreamEvent*   (Started, Token*, terminal; Heartbeat)
POST   /v1/collections                -> 201 { id: CollectionId }
GET    /v1/collections/:id            -> 200 CollectionSummary | 404
DELETE /v1/collections/:id            -> 204 | 409 | 404
GET    /v1/models                     -> 200 [ ModelRow ]               (see node-state-poll)
POST   /v1/models/:id/download        -> 202 { id: ModelId, status }    (stub seam, api concern 8)
POST   /v1/estimate    { CompletionShape } -> 200 { tokens_per_second, estimated_ms, confidence }
GET    /v1/system/state               -> 200 SystemState                (see node-state-poll)
GET    /health                        -> 200 { status: "ok", version }
```

Request/response bodies reference `types` vocabulary:
`types::completion::{CompletionRequest, CompletionId, CompletionResult, CompletionState,
CompletionMetrics, TerminationReason}`, `types::stream::StreamEvent`,
`types::collection::CollectionId`, `types::model::ModelId`,
`types::system::SystemState`.

```rust
// POST /v1/completions accepts a CompletionRequest with id/created_at left blank
// (assigned server-side) and returns the assigned id.
struct SubmitAccepted { id: CompletionId }

// GET /v1/completions[/:id] projection
struct CompletionSummary {
    id: CompletionId, model_id: ModelId, state: CompletionState,
    priority: i32, created_at: DateTime<Utc>,
    // metrics present once terminal (see types::completion::CompletionMetrics)
}
```

### Token stream (WS `/v1/completions/:id/stream`)

`types::stream::StreamEvent` (serde tag `"type"`, snake_case), sourced in wave-2
from the engine's token **broadcast** (not api's old poll loop — api concern 2):

```rust
enum StreamEvent {
    Started   { id: CompletionId },
    Token     { id: CompletionId, text: String, token_count: u32 },
    Completed { id: CompletionId, termination: TerminationReason, metrics: CompletionMetrics },
    Failed    { id: CompletionId, termination: TerminationReason, metrics: CompletionMetrics },
    Cancelled { id: CompletionId },
    Preempted { id: CompletionId, reason: PreemptionReason, requeued: bool },
    Heartbeat { id: CompletionId },
}
// Ordering guarantee (stream.rs): Started once -> Token* in order -> exactly one
// terminal (Completed|Failed|Cancelled|Preempted). Heartbeat every ~30s.
// On Preempted{requeued:true} the client reconnects to the SAME url; id is stable.
```

### Router forward-plane shape (owned by completion-router; opaque `/v1` body)

```rust
// what the router carries per forwarded request (?node= already stripped upstream)
struct ForwardRequest  { addr: Address, method: Method, path: String,
                         headers: Headers, body: BodyStream }
struct ForwardResponse { status: u16, headers: Headers, body: BodyStream } // streaming
enum   Address         { AnyNode { slug: Slug },            // "inference, any node"
                         Node    { node: NodeId, slug: Slug } } // pinned (INTENT #15/#59)
// WS relay: transparent upgrade + bidirectional frame relay for
// /v1/completions/:id/stream, keyed completion_id -> owning node.
```

## Error cases

**api terminus (live `err_response`, part of the byte-transparent contract):**

| Condition | Status |
|-----------|--------|
| `*NotFound` (unknown completion/collection/model) | 404 |
| `*Terminal` (cancel/priority/result on a terminal completion) | 409 |
| `InvalidRequest` / `EmptyCollection` | 422 |
| anything else | 500 |

**router forward-plane (completion-router.md):**

| Condition | Result |
|-----------|--------|
| `NoCandidates` — fleet empty / all nodes unreachable | 503 |
| `ModelUnavailable` — no node has/loads the model (spill deferred) | node's own 409/404 passed through, or 503 if selection fails |
| `NodeUnreachablePreForward` | **one** bounded re-selection (spill), then 502/503 |
| `NodeFailedMidStream` | **terminal** — aborted body / WS close+error; **NO failover**; router emits a `completion` failure event |
| `NoSuchNode` / `NodeUnreachable` on a **pin** (`Node{N}`) | clean error, never rehomed |

The status codes above are copied through unchanged by the router (it never
synthesizes a body from the `/v1` payload).

## Version sensitivity

- **Router coupling: LOW by construction.** The router is byte-transparent — it
  never parses or rewrites the `/v1` body, so api can add fields/routes freely.
  The router couples only to (a) the streaming shape (status/headers/chunked body
  — stable) and (b) the `/v1/completions/:id/stream` path convention (its relay
  index key). Parsing/rewriting the body is a **conformance violation** (would
  re-introduce version coupling).
- **Typed consumers (cc/org/dashboard): MEDIUM.** `/v1` bodies and `StreamEvent`
  evolve **additive-only**. `StreamEvent`/`LifecycleEvent` require a
  `#[serde(other)]` catch-all arm in `types::stream` so a mixed-version fleet
  tolerates unknown variants mid-rollout (flagged to `types`; INTENT #66).
- **Additive-safe:** new routes, new optional body fields, new `StreamEvent`
  variants (behind `#[serde(other)]`), new headers.
- **Breaking:** removing/renaming a route or a body field, changing a status-code
  mapping, changing the stream ordering guarantee or the `:id/stream` path shape.

## Reconciliation notes

1. **Forward-plane vs terminus split (agreed).** completion-router authored the
   forwarding side; api authored the terminus side. They meet at "bytes in =
   bytes out." No conflict — merged as the two halves above.

2. **Node-to-node transport / loopback-relay — RESOLVED (the transport pair).**
   wave2-plan does **not** name a separate `completion-forwarding`/`peer-relay`
   contract file; the node-to-node data path is an internal `mesh-core`
   `PeerTransport` seam (INTENT #29/#45 — a compiled-in lib boundary, not a wire
   contract edge) plus a concrete ask on mesh-core. It is recorded here because
   this is the pair whose remote forward it carries.
   - **inference's position (inference.md concern 10):** inference binds only
     `127.0.0.1:8420` (loopback; dynamic if taken, INTENT #36) and **never binds
     a tailnet-public interface**. A remote completion terminates at the **target
     node's own daemon relaying to that node's local loopback `:8420`** — not a
     cross-node dial of `:8420`. inference stays loopback-only.
   - **completion-router's ask (completion-router.md concern 7):** remote bulk,
     streaming, byte-transparent `/v1` traffic does not fit the pubsub envelope
     (pubsub-relay disclaims it) nor mesh-core's one-shot `Request/Response`
     frame; it needs a **streaming node-to-node channel** on mesh-core's
     `PeerLink` (a dedicated `kind` / brokered tunnel).
   - **Resolution (both satisfied, no loser):** a remote forward is
     `origin daemon → PeerLink streaming channel → target daemon → target loopback :8420`.
     inference remains loopback-only (its position holds verbatim); the router's
     ask holds too, since the origin daemon opens the streaming channel to the
     *target daemon*, never to inference directly. This honors single-port
     locality (INTENT #58) — locality governs service↔service addressing; the
     router **is** the mesh's own privileged data plane reaching the fleet
     node-to-node.
   - **Transport requirement placed on `mesh-core` (for its own design pass):**
     add a **streaming data-channel `kind` on `PeerLink`** (daemon↔daemon over
     the tailnet, `trait PeerTransport`), distinct from the one-shot
     control-frame mux and **not** riding the `types::pubsub::Envelope`. It
     carries `ForwardRequest`/`ForwardResponse` byte streams + the WS-relay
     upgrade. Flagged to mesh-core's `mesh-transport` proposal and sequenced
     after that data-channel lands.

3. **`?node=` handling (agreed).** The pin is expressed as `Address::Node{node}`
   and stripped upstream; api MUST NOT parse a `node` query param (api concern 1)
   — the pin never reaches the terminus body.

4. **Forward-proxy cleanliness (agreed conformance).** api bodies MUST be
   forward-proxy-clean: no `:8420` / host / absolute-URL leakage (api concern 1),
   so a relayed body is indistinguishable from a direct one.

5. **Endpoint discovery (upstream dependency, not resolved here).** For the
   router to reach a specific node's terminus it resolves per-node inference
   endpoints via `service-lookup` (each node self-registers a `NodeScoped`
   `(inference, node)` instance; the `FleetAlias` on the `inference` slug becomes
   a resolve-time policy, not a stored record). Both inference.md (service-lookup)
   and completion-router.md (concern 8) propose this same shape; the actual
   reconciliation belongs to the `service-lookup`/`service-registration` pair
   (another cluster) and is referenced here only as context.

## Example data

**World:** two mesh nodes — `macbook` (the operator's driving laptop; mesh daemon
+ inference) and `pi` (always-on Raspberry Pi inference node). Model `qwen3-4b`.
Project `demo`. Completion id `7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f`.

A client on `macbook` submits to `AnyNode{inference}`. `macbook` is busy and does
not have `qwen3-4b` resident; the router selects `pi`, opens a PeerLink streaming
channel to `pi`'s daemon, which relays to `pi`'s loopback `:8420`. The client sees
a normal local-looking `/v1/` exchange.

```jsonc
// 1) POST /v1/completions  (request body; id + created_at blank, server assigns)
{
  "model_id": "qwen3-4b",
  "prompt": "Summarize the demo project's goals in one sentence.",
  "max_tokens": 128,
  "temperature": 0.7,
  "priority": 10,
  "metrics": 3,                      // MetricsFlags TOKENS|TIMING
  "metadata": { "project": "demo" }
}
// -> 201
{ "id": "7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f" }
```

```jsonc
// 2) GET /v1/completions/7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f  -> 200
{
  "id": "7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f",
  "model_id": "qwen3-4b",
  "state": "running",
  "priority": 10,
  "created_at": "2026-07-19T17:00:00Z"
}
```

```jsonc
// 3) WS GET /v1/completions/7f3c1a90-.../stream  (frames, in order — relayed from `pi`)
{ "type": "started", "id": "7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f" }
{ "type": "token", "id": "7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f", "text": "The", "token_count": 1 }
{ "type": "token", "id": "7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f", "text": " demo", "token_count": 2 }
{ "type": "completed", "id": "7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f",
  "termination": { "completed": "stop" },
  "metrics": { "queue_latency_ms": 40, "generation_ms": 1200,
               "prompt_tokens": 14, "completion_tokens": 22,
               "tokens_per_second": 18.3, "preemption_count": 0, "error_retry_count": 0 } }
```

```jsonc
// 4) GET /v1/completions/7f3c1a90-.../result  -> 200 (CompletionResult)
{
  "id": "7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f",
  "state": "completed",
  "text": "The demo project builds a bottom-up, mesh-distributed OS.",
  "termination": { "completed": "stop" },
  "metrics": { "queue_latency_ms": 40, "generation_ms": 1200,
               "prompt_tokens": 14, "completion_tokens": 22,
               "tokens_per_second": 18.3, "preemption_count": 0, "error_retry_count": 0 },
  "completed_at": "2026-07-19T17:00:01Z"
}
```

```jsonc
// Error example: DELETE /v1/completions/7f3c1a90-... after it is terminal -> 409
{ "error": "completion_terminal", "detail": "completion is completed; cannot cancel" }
// Fleet-empty example: POST /v1/completions when no node is reachable -> 503 (router)
{ "error": "no_candidates", "detail": "no inference node reachable in the fleet" }
```
