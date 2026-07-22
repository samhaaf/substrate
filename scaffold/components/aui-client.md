# aui-client

**Status:** L6 CONCEPTUAL DESIGN (wave 3, batch 6 — INTENT #168/#173c). **Track:**
CONCEPTUAL — data contracts with the lower layers, internals at design-notes
depth only; still **not implementing now**. **Nesting:** top-level app-crate
(pairing: `ui-server`). **Grounding:** INTENT #168 (the operator's ALREADY-BUILT
harness refactor into ui-server + aui-client + a layered state-machine library),
#157 (the walk-along Pi, the chassis blessing queue, queued-messages-on-
reconnect, S2T/T2S placement), #166 Q12 (S2T/T2S = inference modalities), #123/
#146/#149/#150 (coordinators/keepers, threads interactive-or-headless, "I really
only like to talk to coordinators"), #172 (keeper/bundle/landscape/workspace/
chassis/threads LOCKED vocabulary); ledger §A rows 8, 79, 87, 92; ledger §C
OQ-1 design-around (no authority dependency threaded here); `components/
chassis.md` (the thin outbox+blessing profile this file is the concrete
customer of), `components/inference.md` concern 7 (already designs the S2T/T2S
modality-tagged-completion path this file consumes), `components/queues.md`
concern 12 (keeper-inbox, NOT this file's path — see boundary note below),
`contracts/agent-management.md` (the cc surface a keeper's interactive thread
rides), `~/code/harness/apps/aui/src/proto.rs` (the operator's real, already-
running wire shape — informs "rough shape" below, ported, not redesigned).

> **⚠ NOT IMPLEMENTING NOW.** This file goes one notch deeper than the
> re-spoken-round stub (naming the three lower-layer contracts concretely,
> per INTENT #173c: "enough to know what its data contract is supposed to be
> with the lower layers") — it does **not** design aui-client's internals, its
> own state machine, or the ui-server↔aui-client protocol (that ports from the
> operator's working refactor, untouched by this wave). No schema is frozen
> here; every shape below is "rough" and revisits when the port happens.

## What it is

The thin, walk-along-capable client half of the operator's audio user
interface: audio capture/playback, gesture input, and local buffering, holding
**no mesh-side state of its own** — all durable state (thread history, project
scope, keeper identity) lives across the mesh, not on the client. It speaks to
its paired `ui-server` through the shared **layered state-machine library**
(INTENT #168); that library's layering is the aui-client↔ui-server data
contract and is explicitly **out of scope** here (it ports verbatim from the
already-working refactor).

**The walk-along Pi runs aui-client** (INTENT #157). Because the Pi is
frequently offline (out of Tailscale range, asleep, moving between networks),
aui-client is the concrete, named customer of `chassis`'s **thin feature
profile**: `default-features = false, features = ["outbox", "blessing"]` — no
service registration, no surface schema, no restart participation, no inbound
handler surface. It links chassis only for the outbox (queue-while-
disconnected, drain-in-order-on-reconnect) and the blessing-queue mechanics
(chassis.md concern 6/7) — nothing else chassis offers is relevant to a device
that never *serves* anything on the mesh, only *consumes*.

Everything aui-client needs from the mesh proper — discovering `ui-server` (or
a keeper's interactive-thread surface directly, on a screen-optional device),
S2T/T2S, and reaching a keeper — rides ordinary mesh contracts a client-only
device is already a party to; it invents none of its own.

## Charter boundary — what aui-client does NOT own

Not a service (never registers a slug, never publishes a `SurfaceSchema`); not
the state machine (that lives in the shared library between it and
`ui-server`, ported unchanged); not a keeper (it talks *to* keeper threads, it
is not one); not a modality backend (S2T/T2S execute in `inference`, never on
the client, unless a future device is capable-and-warm — see below); not the
authority/blessing target (PARKED OQ-1 — aui-client only *emits* into the
blessing queue chassis gives it, never decides who answers it).

## Anticipated contracts (wave 3, L6)

Three lower-layer contracts, named concretely per INTENT #173c. None require
new mesh-wide surface area — each is a client-side binding of a contract a
sibling batch already owns/designed.

### 1. `aui-client ↔ mesh` — service-lookup, pub/sub, outbox semantics

**Purpose.** The wiring + liveness plane every mesh party uses, bound here to
a **client-only** device: discover `ui-server` (or, on a screen-optional
walk-along device, discover a keeper's interactive-thread surface directly),
subscribe to the events that drive spoken/played output, and survive being
offline without losing anything the operator said.

**Rough shape (chassis thin profile, no new wire contract).**

```rust
// aui-client links chassis[thin] — outbox + blessing only, no serve()/surface
let chassis = Chassis::thin_builder("aui-client", version!())
    .build().await?;               // connects, no Register, no PublishSurface

// service-lookup (client half only — resolve, never register)
chassis.resolve(Address::AnyNode { slug: "ui-server" }) -> Endpoint;
//   (screen-optional device: resolve a keeper's thread surface directly —
//   see contract 3 — instead of routing through ui-server)

// pubsub-protocol (client half only — subscribe, never publish a surface)
chassis.subscribe([TopicFilter::Exact(thread_id)]) -> live ThreadEvent/TextReply
  //   the same live-output stream ui-server would otherwise relay 1:1;
  //   on a screen-optional device this can be the DIRECT feed, bypassing
  //   ui-server, since aui-client is a pure mesh client like any other (aui.md
  //   centerpiece: "AUI adds no new per-service edges").

// outbox semantics (chassis-local, concern 6 of chassis.md — the load-bearing
// piece for #157's "queues messages and sends when connected")
chassis.send_durable(msg, save_on_fail: true);   // AudioBlob / TextInput / Gesture
  //   queued FIFO in the chassis-local durable store while `ui-server`/mesh
  //   is unreachable; drained in order on reconnect; at-least-once, receiver-
  //   side idempotency by message id (consistent with queues-api's event-id
  //   semaphore discipline, reused here as a naming convention, not a shared
  //   queue). This is the concrete mechanism behind #157's "a small client
  //   queues messages and sends when connected."
```

**Reconciliation flags for the harmonizer:** confirm `ui-server` is
`AddressingClass::Singleton` or `NodeScoped` per operator device (open,
depends on ui-server.md's own registration); confirm the screen-optional
direct-to-keeper path (bypassing ui-server) is legitimate under aui.md's
"pure mesh client, no new per-service edges" principle rather than a special
case.

### 2. `aui-client ↔ inference` — S2T/T2S as modality-tagged completions

**Purpose.** Turn the operator's speech into text and speak results back,
**without aui-client ever running a model itself** unless the device is
S2T/T2S-*capable* (has the hardware AND is configured warm for that
modality — the exception, not the default; a walk-along Pi never qualifies,
INTENT #157).

**Rough shape (no new contract — `inference.md` concern 7d already designs
this exactly; aui-client is the named consumer).**

```rust
// aui-client issues a modality-tagged completion over the existing
// v1-completion-api front door — NOT a new endpoint, NOT a new envelope
POST /v1/completions {
    modality: Modality::S2T,           // or Modality::T2S
    input: CompletionInput::Audio(bytes) | CompletionInput::Text(String),
    ..
} -> CompletionId

GET /v1/completions/:id/result -> CompletionResult { text | audio bytes }
GET /v1/completions/:id/stream -> WS StreamEvent*   // for lower-latency T2S playback
```

- The **completion-router** places the request on whichever node advertises
  a warm `S2T`/`T2S` modality (inference.md concern 7a/7b/7c) — aui-client
  makes **no placement decision**; it does not know or care which node
  answers. This is the resolution to #157's "hardware constraints mean
  placement decisions go through [the mesh]": the Pi never advertises S2T
  capability and so never gets asked to run it.
- **Capable-device exception.** A device that IS S2T/T2S-capable (enough
  RAM/accelerator to hold a warm model) MAY run its own modality locally as
  an optimization — but that is a config choice on that device's own
  `inference` node (it becomes an ordinary inference node with a warm pin,
  per inference.md concern 7b), not a special aui-client code path. aui-client
  itself always speaks the same `v1-completion-api` surface whether the
  modality resolves locally or across the mesh — byte-transparent forwarding
  (v1-completion-api.md) makes the two indistinguishable to the caller.
- No S2T/T2S backend, no audio codec choice, and no warm-model policy are
  decided here — those are inference/engine/models/scheduler's (already
  specified). aui-client's contract surface is exactly the existing
  `/v1/completions` request/result/stream trio with a `modality` tag.

**Reconciliation flags for the harmonizer:** confirm `CompletionInput`'s
audio variant (raw bytes vs a VFS/stream-tunnel pointer for large clips,
per `#153` streaming rules) with `api.md`/`engine.md`; confirm whether
aui-client should prefer the WS `stream` route over polling `result` for
T2S playback latency (a client-side choice, not a new contract).

### 3. `aui-client ↔ keeper` — the operator's threads ARE keeper threads

**Purpose.** The single most load-bearing fact for this file: when the
operator talks to AUI, **he is talking to a keeper's interactive thread**,
not to a bespoke "AUI conversation" object (INTENT #123 "I really only like
to talk to coordinators"; #146 "a coordinator owns a topological node... the
workspace = a task-specific extension"; #149 beat 11 "threads start two ways:
interactive... and headless"). aui-client therefore owns **no conversation
record of its own** — this settles aui.md's open question 2
("AUI-owned vs. cc-registered") in the keeper direction: an AUI conversation
*is* a keeper's interactive thread.

**Rough shape (composition of already-designed edges, no new wire contract
— mirrors aui.md's centerpiece pattern exactly).**

```rust
// starting/resuming an interactive thread at a keeper — realized today as
// a cc-managed agent process (agent-management.md), initialized from the
// keeper's BUNDLE (role prompt + plugins, rolled up from its persistent
// memory — INTENT #148 beat 2/3, #165); the keeper concept's own runtime
// design (batch-6 sibling, components/keeper.md) is the authority on HOW
// a thread starts — aui-client only needs the resulting handle.
AgentCmd::Spawn { task: initial_utterance_or_resume, thread: Some(thread_id),
                  project: Option<ProjectId>, environment: Option<EnvironmentId>, .. }
  -> AgentHandle   // (agent-management.md; "aui" already listed as a shaped-for consumer)

// sending a follow-up utterance into a running thread (the everyday path,
// once transcribed to text by contract 2)
AgentCmd::Send { handle, input: transcribed_text }

// live output back to the operator — the same event stream ui-server/aui.md
// already consume, subscribed via contract 1's pubsub binding
AgentEvent::{ TurnCompleted, StateChanged, Exited } / cc-events  -> spoken via
  contract 2's T2S, or displayed via ui-server on a screen device
```

- **Interactive vs headless, aui-client's slice.** aui-client only ever
  drives the **interactive** half (INTENT #149 beat 11) — a human-in-the-loop
  thread the operator is actively speaking into. It never emits or consumes
  keeper-to-keeper **headless** messages; those ride the durable
  `keeper.<keeper_id>.inbox` seam `queues.md` concern 12 designs, which is a
  **different contract with a different party** (keeper ↔ keeper, not
  aui-client ↔ keeper) — noted here only to draw the boundary precisely, not
  because aui-client touches it.
- **Compaction is invisible to aui-client.** INTENT #123's "I should be able
  to compact the thread and lose basically nothing" is a keeper-runtime
  property (bundle-plus-workspace re-seeding, batch-6 sibling's design);
  aui-client's contract is unaffected either way — it addresses the same
  `thread_id`/`AgentHandle` before and after compaction.
- **Which keeper?** Scoping "which topological node the operator is currently
  talking to" is a navigation/state concern of the **shared state-machine
  library** between aui-client and ui-server (out of scope, INTENT #168) —
  by the time a message reaches this contract, the keeper/thread is already
  resolved. aui-client does not itself walk the landscape graph.

**Reconciliation flags for the harmonizer:** the exact mechanism a keeper
uses to spin up/resume an interactive thread (cc `agent-management` today,
per #130's "mini-harness" framing) is `keeper.md`'s (batch-6 sibling) to pin;
if it lands on a different surface than `agent-management`, this contract's
party swaps but its shape (spawn/send/subscribe by handle) is expected to
survive unchanged.

## Relationships / edges

- **`chassis`** (thin profile: outbox + blessing) — shared-lib dependency,
  not a contract edge; the mechanism behind contract 1's durability.
- **`service-lookup` / `pubsub-protocol`** (client half, via chassis-thin) —
  contract 1.
- **`v1-completion-api`** (client, modality-tagged) — contract 2.
- **`agent-management`** (client, "aui" already a listed shaped-for consumer)
  — contract 3.
- **`ui-server`** — the paired screen-facing half; the state-machine-library
  protocol between them is out of scope this pass (ports from the working
  refactor).
- **NOT a party to:** `queues-api`'s keeper-inbox seam (keeper↔keeper only),
  `surface-schema`/`dashboard-feed` (that is ui-server's contract, below —
  aui-client has no screen), any authority/blessing-target contract (OQ-1
  PARKED — aui-client only emits into chassis's abstract seam).

## Nesting

Parent: (top-level app-crate) | Children: none (thin client; links `chassis`
as a library, does not nest it). Pairing: `ui-server`.

## Thoroughness level

**conceptual-contract depth** (INTENT #173c) — the three lower-layer
contracts above are named with purpose + rough shape, sufficient to know
what aui-client needs from mesh/inference/keeper. Internals (the client's own
event loop, audio pipeline, gesture handling, the state-machine library
itself) are explicitly NOT designed here; they port from the operator's
working harness refactor when this file leaves the stub/conceptual track.

## Open

- Whether a screen-optional walk-along device resolves `ui-server` or a
  keeper's thread surface directly (contract 1's reconciliation flag).
- S2T/T2S capable-device exception mechanics (contract 2) — real but
  expected to stay rare; not designed further here.
- The exact keeper-thread-start surface (contract 3) — pinned by `keeper.md`
  (batch-6 sibling), not this file.
- Everything the state-machine library itself carries (INTENT #168) —
  deferred to the port, by directive, not by gap.
