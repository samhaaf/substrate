# Contract: mesh-transport

## Parties

- **every service** (via `chassis`, the daemon-wrapper client half) — one
  persistent bidirectional WebSocket to its LOCAL mesh daemon on `:3649`
  (INTENT #58 single-port locality; #28 everything-over-WS).
- **mesh-core** (the daemon shell on `:3649`) — terminates that socket, and
  relays frames daemon↔daemon across the tailnet.

Cross-cutting, surface-schema-style: ONE shared document, every service is a
party (wave2-plan §5.4). This is the **base transport layer** every other
`contracts/*.md` edge rides — `pubsub-protocol`, `service-lookup`,
`restart-protocol`, `queues-api`, `locks-api`, `vfs-content`, `kg-mesh`,
`aws-vfs`, `aws-mesh`, `secrets-mesh`, `v1-completion-api` and ~a dozen more
reference this file's `Frame`/`Address`/`ResponseOutcome`/`PeerUnreachable`
vocabulary by name. Authored batch-1 (L0/L1) so every higher layer's contracts
bind to the envelope beneath them. The client half is `chassis` (which absorbs
the former `mesh-client` client seam — ledger D1); the daemon half is
`mesh-core` (its `SessionMux` / `PeerLink` rings). Struct vocabulary is homed in
`types` (see **Proposed contracts (wave 3)** below).

> **AUTHORIZATION status (OQ-30 / INTENT #173d).** OQ-30 reserves *"the
> mesh-transport envelope spec and the streaming peer-link channel"* for the
> operator's explicit blessing as mesh-core design items. The **framing model**
> below (connect handshake, `Frame` mux, `MsgKind`, the #154 `ResponseOutcome`
> switch, sender stamping) is the boring reconciliation of what ~23 contracts
> already reference and is authored implementation-ready. The **direct brokered
> streaming tunnel** (§ Streaming) — bytes leaving the relay for a scoped
> node→node pipe — is the specific seam OQ-30 flags: the frames are specified,
> but the *direct peer-link data channel* is marked **AUTHORIZATION PENDING** and
> must not be built until blessed. The relayed streaming path (chunks through the
> daemon) is the boring default and is not blocked.

## Purpose

Define the **one wire** a service speaks to its local mesh daemon, and the frame
model mesh-core uses to relay between daemons — so that every inter-service call
is universally mediated (INTENT #152: never service-to-service directly; if the
target is down mesh restarts it and patches the message through), fully async
(#28), version-attributed (#113), and forced to handle the promise case (#152,
#156). One connection per service, multiplexing request/response RPC, pub/sub,
restart control, queues/locks/cron request frames, promise fulfilment, and
scoped one-directional streams — all over a single `ws://127.0.0.1:3649`.

The transport is **payload-opaque**: it routes on frame headers (`to`, `kind`,
`correlate`) and never deserializes a `method` payload. Concrete request/response
schemas are the authored per-edge contracts' business; this file owns only the
carrier.

## Schema

All frames are JSON, tagged enums (`#[serde(tag = "type")]`), following the
existing `stream.rs` convention. Every wire-crossing struct here carries
`types` guardrail-4 discipline (additive-only, `#[serde(default)]`, no
`deny_unknown_fields`, `#[serde(other)]` catch-alls).

### 1. Connect handshake (first frames on a fresh `:3649` connection)

A service's `chassis` opens exactly one socket and sends `Hello`; the daemon
replies `Welcome`. This establishes the connection's **authoritative
service + version** (INTENT #113) once, so per-frame stamping is cheap and
daemon-verified against the registry lease.

```rust
struct Hello {
    proto:           u16,        // transport proto version the client speaks
    service:         Slug,       // who I am  (INTENT #113 sender NAME)
    service_version: SemVer,     // my running build (INTENT #113 sender VERSION)
    node:            NodeId,     // Tailscale device name
    role:            ConnRole,   // Service | CliOneShot | Peer (daemon↔daemon)
    #[serde(default)] min_peer_version: Option<SemVer>, // optional inbound version FLOOR (#113)
}
enum ConnRole { Service, CliOneShot, Peer }

struct Welcome {
    proto:          u16,         // daemon proto; the negotiated floor = min(client, daemon)
    daemon_version: SemVer,
    conn_id:        Uuid,        // this connection's id (relay bookkeeping)
    #[serde(default)] please_update: Option<VersionAdvisory>, // #113 back-compat warning
}
struct VersionAdvisory { have: SemVer, floor: SemVer, note: String } // "update, I accept you for one more version"
```

A **browser** cannot speak this handshake (no `Hello`/`Welcome`) — the dashboard
origin is plain HTTP on a separate port (see `dashboard-serving`); mesh-transport
is service-plane only.

### 2. The `Frame` — the multiplexing envelope on every mesh-transport socket

Every message after the handshake — in either direction, service↔daemon and
daemon↔daemon — is one `Frame`. This is the wrapper `pubsub-protocol`
Reconciliation note 6 names "the mesh-transport `Frame`," and the same thing
`vfs-content` / `aws-vfs` loosely call the "`mesh-transport Envelope { to, kind }`"
(see Reconciliation notes — the word *Envelope* is reserved for pub/sub's message
header, so the transport wrapper is the `Frame`).

```rust
struct Frame {
    proto:     u16,               // transport proto (dedupe against the negotiated floor)
    frame_id:  Uuid,              // unique per frame (dedup, correlation target)
    from:      PeerStamp,         // sender NAME + VERSION + node  (INTENT #113, EVERY frame)
    to:        Address,           // routing target; `Local{self}` for daemon-terminated frames
    #[serde(default)] correlate: Option<Uuid>, // the request/stream/promise id this frame answers
    kind:      MsgKind,
}

// sender stamp — the #113 "name AND version on every message crossing the mesh".
// Daemon-verified against the registry lease established at Hello; a spoofed
// service/version is rejected before relay.
struct PeerStamp { service: Slug, service_version: SemVer, node: NodeId }

// Addressing classes (INTENT #59) — homed in `types::registry::Address`, reused verbatim:
//   enum Address { AnyNode{slug} | Node{node, slug} | Local{slug} }
```

`Address` is **not redefined here** — it is the same `Address` `service-lookup`
resolves against; mesh-transport is its consumer. `AnyNode` is virtualized (the
local daemon picks/relays to any live instance); `Node` is pinned; `Local` is
this node's instance.

### 3. `MsgKind` — the frame kinds

```rust
enum MsgKind {
    // ── one-shot RPC ────────────────────────────────────────────────────
    Request  { method: String, payload: Box<RawValue> },   // opaque; `method` routes, daemon never decodes
    Response { outcome: ResponseOutcome },                 // answers `correlate`; the #154 switch

    // ── pub/sub data path (headers = types::pubsub::Envelope) ───────────
    Publish       { envelope: pubsub::Envelope },          // service -> daemon, onto a topic
    EventDelivery { envelope: pubsub::Envelope },          // daemon -> subscriber (a matched relay)

    // ── promise fulfilment (INTENT #152) ────────────────────────────────
    PromiseFulfillment { promise: Uuid, outcome: ResponseOutcome }, // pushed-back value for an earlier Promise

    // ── scoped, ONE-DIRECTIONAL stream (INTENT #153) — see § Streaming ──
    StreamOpen  { stream_id: Uuid, method: String, meta: Box<RawValue> },
    StreamChunk { stream_id: Uuid, seq: u64, bytes: Bytes },   // seq-ordered, backpressured
    StreamClose { stream_id: Uuid, outcome: StreamOutcome },
}
```

pub/sub *control* frames (`Subscribe` / `Unsubscribe` / `SubAck` / `Lagged`,
`pubsub-protocol`) ride the generic `Request`/`Response` kinds; only the
high-volume `Publish` / `EventDelivery` data path gets dedicated kinds so the
relay can fast-path it without an RPC round-trip. `queues-api`, `locks-api`,
`cron-api` are ordinary `Request`/`Response` payloads — `chassis` ferries their
typed envelopes; their semantics live in their own contracts.

### 4. `ResponseOutcome` — THE #154 envelope switch (success / error / promise)

INTENT #154, verbatim: *"the outermost layer is a switch — success / error /
promise — then a schema which specific inter-application messages inherit from
going inward."* This is that switch, at the **response level**. Every `Response`
and every `PromiseFulfillment` is exactly one of three arms; the inner `payload`
is the authored per-edge schema (opaque here).

```rust
enum ResponseOutcome {
    Success { payload: Box<RawValue> },        // the inherited per-contract schema, inward
    Error   { error: SubstrateError },         // the domain-nested taxonomy (types::error)
    Promise { promise: Uuid,                   // caller moves on; value arrives later as PromiseFulfillment
              #[serde(default)] hint: Option<PromiseHint> },
}
struct PromiseHint { eta: Option<DateTime<Utc>>, note: Option<String> } // best-effort, non-binding
```

**Contract-level enforcement (INTENT #152 / #156).** Because any `Request` may be
answered with `Promise`, **every inter-service caller MUST handle all three
arms** — Rust-exhaustive, no wildcard that silently swallows `Promise`. This is
the transport-level expression of "every inter-service request must have a
handler for the promise case." When mesh cannot answer instantly (target busy,
or down-and-being-restarted-and-patched-through, #152), it returns
`Promise{promise}` immediately (`all communication attempts are instant`); the
value is later pushed as a `PromiseFulfillment` frame correlated by `promise`, or
fetched. Promise-resolution notices and intermediate values are cached in the
distributed KV (INTENT #155 — `pubsub-relay` / `kv-cache`), never silently
dropped.

### 5. Streaming — scoped, ONE-DIRECTIONAL tunnels (INTENT #153)

**AUTHORIZATION PENDING (OQ-30).** The frames are specified; the *direct*
brokered channel awaits operator blessing.

Two transfer modes share the `StreamOpen/Chunk/Close` frames:

- **Relayed stream (boring default, authorized).** `StreamChunk`s ride the
  ordinary daemon relay (local daemon → peer daemon → consumer), so mesh can
  observe, backpressure, and resume across an interruption (INTENT #152 "so it
  can handle interruption/resume"). This is what `vfs-content` (bulk blob pull),
  `kg-mesh` (merge-sync body), and `aws-vfs` `Transfer::Relay` (ciphertext
  chunks) already ride. A stream is opened with `Address::Node{N}` (pinned; the
  puller already knows which node holds the blob from replicated metadata).

- **Direct brokered tunnel (AUTHORIZATION PENDING — OQ-30 / INTENT #153).** For
  specific large, byte-transparent transfers (raw `/v1` inference forwarding —
  `completion-router` / `v1-completion-api`; multi-GB model weights), mesh
  *brokers* a connection **scoped to one contracted event** that streams from the
  generating node straight to the consumer, taking the bytes **off the relay**.
  This is the "streaming data-channel `kind` on `PeerLink`" both
  `completion-router` and `v1-completion-api` flagged to this proposal. Hard
  rules the spec locks even while blessing is pending:
  - **ONE-DIRECTIONAL only** — a brokered tunnel carries bytes one way for one
    contracted event; it is **never a permanent live connection** and **never
    bidirectional**. Anything two-directional goes back through the mesh (#153).
  - **Scoped and ephemeral** — brokered per `StreamOpen`, torn down at
    `StreamClose`; no service holds a standing peer socket.
  - Integrity/resume are the payload contract's (e.g. `vfs-content`'s
    content-addressed chunk verification), not the tunnel's.

```rust
enum StreamOutcome { Complete { chunks: u64 }, Error { error: SubstrateError }, Aborted { at_seq: u64 } }
```

Small payloads always route through mesh as ordinary `Request`/`Response`
(#152); streaming is only for bulk, and only one-directional.

## Error cases

`TransportError` (`types::error::transport`; surfaced as
`SubstrateError::Transport(..)`, matchable, never a panic — CAP-honest INTENT
#84). These are the transport failures that ~a dozen edges disclaim as *"not
mine — mesh-transport's"* (`locks-api`, `vfs-content`, `secrets-mesh`,
`kg-mesh`, `repo-vfs`, …):

- `PeerUnreachable { node }` — a `Node{N}` target's daemon is unreachable and
  mesh could not patch through within the relay deadline. The single most-named
  transport error; every higher edge layers its own semantics on top of this.
- `NoLocalDaemon` — the local `:3649` daemon is down (the caller is fully
  isolated; `chassis` reconnects, mesh is sticky). Fail-fast, never block forever.
- `Backpressure { to }` — a stream/relay buffer is full; the sender must slow
  (bounded memory, no unbounded queue — INTENT #38).
- `ProtoFloorViolation { have, floor }` — the frame's `proto` is below the
  daemon's accepted floor after a Compatibility roll; drives a
  `RestartReason::Compatibility` restart.
- `VersionBelowFloor { service, have, floor }` — a receiver enforcing an inbound
  version floor (#113 `min_peer_version`) rejected the sender; **catchable, never
  a silent drop**; the daemon may attach a `please_update` advisory back to the
  sender.
- `NotRegistered { service }` — a frame whose `from` stamp has no live registry
  lease (provenance can't be attested); rejected before relay.
- `Unroutable { addr }` — `to` names a slug/node with no live instance
  (distinct from `PeerUnreachable`: nothing to reach vs. can't reach).
- `FrameTooLarge` — a single non-stream frame exceeds the cap (bulk must use a
  stream, not a giant `Request`).

**Non-errors by design.** A `Promise` outcome is a normal successful response,
not an error. A target that is *down* is not necessarily an error — mesh
restarts it and patches the message through (#152); `PeerUnreachable` is raised
only when patch-through fails. A `Publish` to a topic with no subscribers
succeeds silently (`pubsub-protocol`).

## Version sensitivity

**HIGH — this is the fleet's mixed-version floor.** Frames cross nodes on
possibly-different `types`/proto versions during minimal-restart rolling updates
(INTENT #66/#76).

- **The `proto` field is the anchor.** Two adjacent proto versions MUST
  interoperate; the negotiated floor at `Hello` is `min(client, daemon)`.
- **Additive-safe:** new `#[serde(default)]` fields on `Frame`/`Hello`/`Welcome`;
  new `#[serde(other)]`-tolerant `MsgKind` / `ConnRole` variants; **any new
  `method` string** — the payload-opaque relay carries an unknown method through
  an older daemon untouched (the primary version-safety property, mirroring
  `pubsub-protocol`'s open `event_type`).
- **Breaking:** changing the `Frame` header field set/meaning, the `Address`
  grammar, the `ResponseOutcome` three-arm switch, or making the relay parse
  payloads. A proto **major** bump drives fleet-wide
  `RestartReason::Compatibility` restarts (`restart-protocol`) — the exact
  mechanism by which the whole fleet rolls to a new transport floor.
- **Sender stamping is LOCKED (INTENT #113).** Every `Frame.from` carries
  service **name AND version**; receivers MAY enforce a **version floor**
  (`Hello.min_peer_version`); a floored frame is rejected with the catchable
  `VersionBelowFloor`, and the receiver offers **one version of back-compat**
  with a `please_update` advisory attached back to the sender. The relay itself
  stays payload-opaque and does not enforce receiver floors — flooring is a
  receiver-edge policy (identical discipline to `pubsub-protocol`).
- Relay sites operate on the opaque `Frame` and MUST NOT `deny_unknown_fields`.

## Reconciliation notes

This file is **created new** (wave-3 batch 1); no prior stub. It reconciles the
several loose references scattered across the wave-2 contracts into one carrier.

1. **`Frame` vs `Envelope { to, kind }` — naming collision resolved.**
   `pubsub-protocol` note 6 named the multiplexing wrapper the mesh-transport
   `Frame`; `vfs-content` / `aws-vfs` prose loosely wrote `mesh-transport
   Envelope { to, kind }`. **`Frame` wins** as the transport wrapper's name,
   because *Envelope* is already the pub/sub **message header**
   (`types::pubsub::Envelope`) and reusing it for the transport wrapper is the
   exact collision note 6 flagged. The `{ to, kind }` fields those files named
   ARE `Frame.to` / `Frame.kind` — same object, corrected name. (A one-line
   annotation in `vfs-content` / `aws-vfs` at the batch-7 sweep suffices; the
   fields are unchanged.)

2. **Relayed vs direct-brokered streaming.** `completion-router` and
   `v1-completion-api` asked for a streaming data-channel `kind` on mesh-core's
   `PeerLink`, "distinct from the one-shot control-frame mux and not riding
   `types::pubsub::Envelope`." That is the `StreamOpen/Chunk/Close` kinds here.
   The **relayed** form is authorized and already used by `vfs-content` /
   `kg-mesh` / `aws-vfs`; the **direct** off-relay tunnel is the OQ-30
   authorization-pending seam. Recorded so mesh-core's fill knows exactly which
   half is blessed.

3. **`PeerLink` / `PeerTransport` are mesh-core internals, not the wire.** The
   daemon↔daemon link ring (`PeerLink`, the `trait PeerTransport` seam) is
   mesh-core's implementation of relaying `Frame`s across the tailnet; this
   contract owns the *frame*, not the ring. `Frame` with `ConnRole::Peer` is the
   daemon↔daemon relay form.

4. **`Address` is homed in `types::registry`, not duplicated.** mesh-transport
   consumes the same `Address` `service-lookup` resolves. No second definition.

5. **Losing position recorded — a per-message ack.** Like `pubsub-protocol`,
   this transport has **no per-frame ack**; delivery guarantees are `queues-api`'s
   (at-least-once) job, promise/response correlation covers RPC, and the lossy
   pub/sub path uses `Lagged` notices. A caller wanting durability uses queues,
   not a transport-level ack.

## Proposed contracts (wave 3)

This unit owns a genuinely new **framing vocabulary** that does not yet exist in
`types`. The following additions are proposed for the `types` owner (batch 1) to
land; they are the struct home for everything above (mesh-transport is written
*in terms of* them, exactly as every edge is written in terms of `types`).

- **NEW module `types::transport`** (`transport.rs`) — homes `Frame`,
  `PeerStamp`, `MsgKind`, `ResponseOutcome`, `PromiseHint`, `Hello`, `Welcome`,
  `VersionAdvisory`, `ConnRole`, `StreamOutcome`. All wire-crossing → guardrail-4
  discipline; `proto: u16` is the version anchor (the transport analogue of
  `Envelope.v`). Passes the inclusion test trivially: every service + mesh-core
  are parties.
- **`Address` stays in `types::registry`** (already there); `types::transport`
  re-exports it for ergonomic `use`, but does **not** redefine it — one home,
  avoiding a second source of truth.
- **NEW error domain `types::error::transport`** (`error/transport.rs`) —
  `TransportError` with the variants in § Error cases, wrapped as
  `SubstrateError::Transport(TransportError)` in the domain-nested taxonomy
  (INTENT #138). NB: this replaces the old **flat** `SubstrateError::Transport(String)`
  leaf noted in `types.md` — the error-taxonomy sweep folds it into this
  sub-enum in the one migration pass.
- **`ResponseOutcome` is the #154 envelope switch** and belongs here, NOT bolted
  onto `pubsub.rs`: pub/sub is best-effort/lossy and has no success/error/promise
  response; the switch is an **RPC-response** concept. (The wave-2 `types.md`
  charter mentioned "the WS pub/sub envelope" generically; the #154 switch is a
  distinct, newly-homed type — flagged so the `types` owner does not conflate
  them.)
- **Relationship to `types::promise`.** `types::promise` (`Promise`/
  `PromiseSender`, a `tokio::oneshot`) is the **in-process** handoff primitive
  and stays wire-exempt. The **wire** promise is a `Uuid` ticket
  (`ResponseOutcome::Promise{promise}` → later `PromiseFulfillment{promise}`);
  the two are deliberately different objects and must not be merged (the oneshot
  never crosses the socket).

### Schema-inheritance future (INTENT #139 / OQ-21) — OPEN, not designed here

INTENT #139 (OQ-21): *"the ENTIRE messaging protocol in the mesh could be done
using schemas and schema inheritance"* — this is a big-unification idea the
operator reserved for its own round; **it is NOT decided or applied here.** One
paragraph on how it would slot in *without* designing it: today a `Frame`'s
`Request.payload` and `ResponseOutcome::Success.payload` are opaque `RawValue`
whose concrete shape is fixed by the authored per-edge contract's hand-written
`types` structs, and #154's "a schema which specific inter-application messages
inherit from, going inward" is satisfied by those authored structs. If #139
lands, that inward schema would instead be a `schema`-tool-managed inheritance
chain (versioned, multiply-inheriting, codegen'd to Rust and queryable over WS —
#131/#145), and `method`/`payload` would carry a schema id + version rather than
an authored-struct name. **The framing model above is deliberately
payload-opaque precisely so it can carry either form with no wire change** — the
`Frame` header, addressing, `ResponseOutcome` switch, and stamping are unaffected
by whether the inward payload is a hand-authored struct or a schema-inherited
one. That is the entire seam; the substitution itself is OQ-21's round to design.

## Example data

The example world: nodes **macbook** and **pi**, project **demo**, model
**qwen3-4b** resident on `pi`.

**1. Handshake — `inference` on pi connects to its local daemon:**
```jsonc
// -> Hello
{ "type": "Hello", "proto": 1, "service": "inference", "service_version": "2.1.0",
  "node": "pi", "role": "Service" }
// <- Welcome
{ "type": "Welcome", "proto": 1, "daemon_version": "1.4.0",
  "conn_id": "conn-77a1", "please_update": null }
```

**2. RPC that resolves instantly — `cc` asks `secrets` for a value, `AnyNode`:**
```jsonc
// cc -> local daemon
{ "proto": 1, "frame_id": "f-01",
  "from": { "service": "cc", "service_version": "3.2.0", "node": "macbook" },
  "to":   { "AnyNode": { "slug": "secrets" } },
  "kind": { "type": "Request", "method": "secrets.get",
            "payload": { "ref": "openrouter/api_key" } } }
// daemon (relayed to the secrets owner and back) -> cc
{ "proto": 1, "frame_id": "f-02",
  "from": { "service": "secrets", "service_version": "1.1.0", "node": "macbook" },
  "to":   { "Local": { "slug": "cc" } },
  "correlate": "f-01",
  "kind": { "type": "Response",
            "outcome": { "type": "Success", "payload": { "handle": "sec:opaque:9f2" } } } }
```

**3. The promise path (#152) — target busy; mesh returns a Promise, pushes later:**
```jsonc
// immediate response: a promise ticket (caller moves on)
{ "proto": 1, "frame_id": "f-04",
  "from": { "service": "kg", "service_version": "0.9.0", "node": "macbook" },
  "to":   { "Local": { "slug": "cc" } },
  "correlate": "f-03",
  "kind": { "type": "Response",
            "outcome": { "type": "Promise", "promise": "p-8c1",
                         "hint": { "eta": "2026-07-19T18:20:05Z", "note": "kg reindexing" } } } }
// later: the value is pushed back, correlated by the promise id
{ "proto": 1, "frame_id": "f-09",
  "from": { "service": "kg", "service_version": "0.9.0", "node": "macbook" },
  "to":   { "Local": { "slug": "cc" } },
  "kind": { "type": "PromiseFulfillment", "promise": "p-8c1",
            "outcome": { "type": "Success", "payload": { "node_id": "kg:node:41c" } } } }
```

**4. A version-floored sender (#113) — a receiver rejects an old build:**
```jsonc
{ "proto": 1, "frame_id": "f-11",
  "from": { "service": "db", "service_version": "1.2.0", "node": "pi" },
  "to":   { "Local": { "slug": "vdb" } },
  "correlate": "f-10",
  "kind": { "type": "Response",
            "outcome": { "type": "Error",
              "error": { "Transport": { "VersionBelowFloor":
                { "service": "db", "have": "1.2.0", "floor": "1.4.0" } } } } } }
```

**5. Relayed one-directional stream (authorized) — pi pulls a blob from macbook
(the `vfs-content` bulk path riding stream frames):**
```jsonc
// pi -> daemon: open a pinned stream to macbook's vfs leg
{ "proto": 1, "frame_id": "f-20",
  "from": { "service": "vfs", "service_version": "1.0.0", "node": "pi" },
  "to":   { "Node": { "node": "macbook", "slug": "vfs" } },
  "kind": { "type": "StreamOpen", "stream_id": "s-3b1f", "method": "vfs.content.pull",
            "meta": { "content_hash": "sha256:3b1f9e0a…c7" } } }
// macbook -> pi: chunks, seq-ordered, backpressured, then close
{ "kind": { "type": "StreamChunk", "stream_id": "s-3b1f", "seq": 0,   "bytes": "…4MiB…" } }
// … seq 1..599 …
{ "kind": { "type": "StreamClose", "stream_id": "s-3b1f",
            "outcome": { "type": "Complete", "chunks": 600 } } }
```
The **direct brokered** form of this same stream (bytes off the relay,
node→node) is AUTHORIZATION PENDING (OQ-30) and not exercised until blessed.
