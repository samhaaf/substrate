# ui-server

**Status:** L6 CONCEPTUAL DESIGN (wave 3, batch 6 — INTENT #168/#173c). **Track:**
CONCEPTUAL — data contracts with the lower layers, internals at design-notes
depth only; still **not implementing now**. **Nesting:** top-level app-crate
(pairing: `aui-client`). **Grounding:** INTENT #168 (the operator's ALREADY-BUILT
harness refactor into ui-server + aui-client + a layered state-machine library;
he still runs the old version day-to-day), #123/#130/#146/#149/#150 (coordinators/
keepers, threads interactive-or-headless, the "mini-harness" framing), #172
(keeper/bundle/landscape/workspace/chassis/threads LOCKED vocabulary); ledger §A
rows 35/79/86/87; `components/mesh-core.md` concern 12 (the observability
serving plane `dashboard-serving` folded into — the pattern ui-server's own
contract mirrors), `components/dashboard.md` (the sibling batch-6 file that
already builds against the identical `dashboard-feed` seam — the closest
existing analog), `contracts/dashboard-feed.md`, `contracts/surface-schema.md`,
`contracts/agent-management.md`, `~/code/harness/apps/ui-server/src/*`
(the operator's real, already-running server — informs "rough shape" below,
ported, not redesigned).

> **⚠ NOT IMPLEMENTING NOW.** This file goes one notch deeper than the
> re-spoken-round stub (naming the lower-layer contracts concretely, per INTENT
> #173c) — it does **not** design ui-server's internals, its own state machine,
> or the ui-server↔aui-client protocol (that ports from the operator's working
> refactor, untouched by this wave).

## What it is

The mesh-side process that owns AUI's server-half state: which keeper thread
is in scope, thread/turn history, and the fan-out of live output to however
many `aui-client` frontends are attached (a Pi, a phone, a TUI, a desktop —
INTENT #168's "one or more thin clients"). It is the **screen-facing** partner
of `aui-client`: where aui-client is audio/gesture-only and holds no mesh-side
state, ui-server is the party that actually talks to the mesh on the
operator's behalf and mediates a picture/text-capable surface when one is
available. Between them sits the **layered state-machine library** (INTENT
#168) — the shared protocol core both halves consume, out of scope here.

ui-server is, in shape, the same kind of thing `mesh-core`'s folded
`dashboard-serving` module and its sibling `dashboard.md` already are: a
service that (a) discovers everything over the mesh rather than hardcoding
edges, and (b) turns a boring, schema-driven feed into an operator-facing
surface. The difference is *what* it renders — not fleet observability, but
the operator's own conversations with keepers — and *who* it talks to for
content — not `surface-schema`'s generic per-service panels, but a keeper's
interactive-thread stream. Where the two surfaces overlap (discovery,
schema-driven rendering, a read-only pub/sub feed), ui-server is a **plain
consumer** of the same contracts `dashboard.md` consumes, not a reinvention
(the aui.md centerpiece principle, restated for the screen-facing half: "AUI
needs no second integration surface").

## Charter boundary — what ui-server does NOT own

Not a keeper (it drives keeper threads, it is not one); not the state machine
(shared library, ported unchanged, out of scope); not a completion runtime
(inference); not an agent manager (cc) — it is a consumer of `cc`'s
`agent-management` surface, not a reimplementation of it; not the fleet
dashboard (`dashboard.md` — a distinct sibling service; ui-server may reuse
the same `surface-schema`/`dashboard-feed` contracts but serves a different
browser surface for a different purpose); not a new integration edge per
service (per aui.md's centerpiece: any service ui-server needs to expose
becomes so the moment it publishes its `SurfaceSchema`, same as the fleet
dashboard).

## Anticipated contracts (wave 3, L6)

Two lower-layer contracts, named concretely per INTENT #173c. Both are
client-side bindings of contracts already owned/designed by sibling batches;
neither requires new mesh-wide surface area.

### 1. `ui-server ↔ mesh` — surface schemas + dashboard-feed via mesh-core's dashboard module

**Purpose.** Discover the mesh and render/interact with any service the
operator's screen-facing session needs, and receive a live event feed for
whatever is in scope — reusing exactly the machinery `mesh-core`'s folded
`dashboard` module already serves to the fleet dashboard (concern 12), rather
than growing a parallel discovery/feed path.

**Rough shape (reuse, no new wire contract — a second consumer of an
existing seam, same pattern `dashboard.md` already documents).**

```rust
// ui-server links chassis[full] like any ordinary service (it DOES register
// — unlike aui-client, ui-server is mesh-side and may itself be discovered,
// e.g. by aui-client resolving "ui-server" per aui-client.md contract 1)
let chassis = Chassis::builder("ui-server", version!())
    .surface(ui_server_surface_schema())   // ui-server's OWN observable surface,
    .serve::<UiServerContracts>(handlers)  //   published like any service (surface-schema)
    .await?;

// discovery + schema reads — identical shape to dashboard.md's consumption
GET /api/surface   -> DashboardManifest   // (mesh-core concern 12 job 4; which
                                           //  services/keepers publish a surface)
chassis.resolve(Address::AnyNode { slug }) -> Endpoint   // service-lookup, as any client

// the live feed — the SAME pubsub-relay-profile WS dashboard-feed.md already
// specifies (Subscribe/Unsubscribe accepted, Publish rejected for a browser-
// origin-style consumer); ui-server subscribes with its own TopicFilter set
// (keeper/thread-scoped, not the fleet-wide prefixes dashboard.md uses)
GET /events (or the mesh-internal equivalent since ui-server is itself a
            service, not a browser) -> WS Delivery(Envelope<Event<P>>)*
```

- **What's reused, unchanged:** the `SurfaceSchema` publication/aggregation
  path (`surface-schema.md`), the `dashboard-feed` read-only pub/sub profile
  and its lossy/`Lagged` semantics (`dashboard-feed.md`), and mesh-core's
  `dashboard` module as the aggregation point for "what services/keepers
  exist and what can I ask them" (mesh-core.md concern 12, jobs 2 and 4).
  ui-server does not reimplement discovery, aggregation, or the feed
  protocol — it is one more consumer, alongside `dashboard.md`'s Svelte
  frontend and the future landscape view.
- **What's new (ui-server's own surface, not this file's to design):**
  ui-server publishes its **own** `SurfaceSchema` describing its own
  observable state (which threads/keepers are in scope for which operator
  session) so the fleet dashboard, an agent, or a future admin surface can
  see AUI's own state through the same boring mechanism — but the operator-
  facing rendering ui-server itself produces for `aui-client` is NOT this
  schema; it is the layered state-machine library's protocol (out of scope).
- **Landscape awareness (INTENT #150 beat 16, #169).** `dashboard.md`'s
  wave-3 landscape view (rendering the topological map / keepers +
  artifacts) is the natural place the operator would navigate "which keeper
  am I talking to" from a screen — ui-server is a plausible consumer or
  embedder of that same landscape-view data, not a second implementation of
  it. Recorded as an open question (below), not decided.

**Reconciliation flags for the harmonizer:** confirm ui-server's own
registration `AddressingClass` (likely `Singleton` per operator install, or
`NodeScoped` if the operator runs one per device); confirm whether ui-server
reads mesh-core's dashboard feed as an ordinary mesh service (WS via chassis,
its own subscription) or needs a variant of the browser-facing `/events` path
— it is mesh-side, not a browser, so the former is expected.

### 2. `ui-server ↔ keeper` — interactive threads (the operator talks to keepers)

**Purpose.** Identical load-bearing fact to `aui-client.md` contract 3,
restated for the screen-facing half: **the operator's threads ARE keeper
threads** (INTENT #123/#146/#149 beat 11). ui-server owns no conversation
record of its own beyond what it needs to route/render; the thread of record
lives at the keeper (its bundle-initialized, cc-managed interactive thread).
This is the same settlement aui.md's open question 2 pointed at
("AUI-owned vs. cc-registered") — resolved toward keeper/cc-registered for
both AUI halves.

**Rough shape (identical contract shape to `aui-client.md` contract 3 — the
same underlying edge, reached from the screen-facing side; not duplicated
design, restated so this file's own lower-layer contracts are complete).**

```rust
// same agent-management.md surface aui-client's contract 3 uses; ui-server
// is simply a second, screen-capable caller against the same keeper thread
AgentCmd::Spawn { task, thread: Some(thread_id), project, environment, .. } -> AgentHandle
AgentCmd::Send  { handle, input }                    // operator's typed/transcribed input
AgentCmd::List  { filter }                           // "which threads/keepers can I show"
AgentEvent::{ TurnCompleted, StateChanged, Exited } / cc-events  -> rendered to screen
```

- **Multiple frontends, one thread.** Because ui-server (not aui-client)
  holds the mesh-side connection, it is the natural place multiple attached
  `aui-client` frontends fan in/out of the same keeper thread — but the
  underlying "multiple human threads per node" question (INTENT #146 "why
  not") and its interaction with one-keeper-per-workspace (#123) is
  ledger OQ-12, **OPEN**, not decided here; ui-server's contract is written
  to be agnostic to whichever answer lands (it addresses threads by
  `ThreadId`/`AgentHandle`, not by a fixed 1:1 device mapping).
- **Rendering is ui-server's job, not the keeper's.** The keeper/cc side
  emits typed events (`AgentEvent`, `cc-events`); turning that into
  screen-renderable turns/messages is the layered state-machine library's
  concern (out of scope) — this contract only specifies WHERE that data
  comes from.
- **Compaction is invisible here too**, for the same reason as
  `aui-client.md` contract 3: a `thread_id`/`AgentHandle` survives keeper-side
  compaction; ui-server's contract is unaffected.

**Reconciliation flags for the harmonizer:** same as `aui-client.md`
contract 3 — if `keeper.md` (batch-6 sibling) lands interactive-thread
start/resume on a different surface than `agent-management`, both AUI-half
files' contract 2/3 swap parties together (they are the same edge, viewed
from each side).

## Relationships / edges

- **`surface-schema` / `dashboard-feed`** (client, reused verbatim from the
  pattern `dashboard.md` already documents) — contract 1.
- **`agent-management`** (client, same edge `aui-client.md` contract 3 uses)
  — contract 2.
- **`aui-client`** — the paired thin-client half; the state-machine-library
  protocol between them is out of scope this pass (ports from the working
  refactor).
- **`dashboard`** — a sibling consumer of the same `surface-schema`/
  `dashboard-feed` seam, for a different (fleet-observability, not
  operator-conversation) purpose; no direct edge between the two, only a
  shared upstream contract.
- **NOT a party to:** `queues-api`'s keeper-inbox seam (keeper↔keeper only,
  headless — ui-server drives only interactive threads); any authority/
  blessing-target contract (OQ-1 PARKED — ui-server is mesh-side and
  reachable, so it has no offline-outbox need the way `aui-client`'s
  walk-along profile does; it links `chassis[full]`, not the thin profile).

## Nesting

Parent: (top-level app-crate) | Children: none. Pairing: `aui-client`.

## Thoroughness level

**conceptual-contract depth** (INTENT #173c) — the two lower-layer contracts
above are named with purpose + rough shape, sufficient to know what
ui-server needs from mesh/keeper. Internals (the server's own state machine,
session/attach management for multiple `aui-client` frontends, rendering
logic) are explicitly NOT designed here; they port from the operator's
working harness refactor when this file leaves the stub/conceptual track.

## Open

- ui-server's own `AddressingClass` and whether it registers per-operator or
  per-device (contract 1's reconciliation flag).
- Whether ui-server embeds/consumes `dashboard.md`'s landscape view for
  keeper navigation, or the state-machine library owns that entirely
  (contract 1, landscape awareness note).
- Multiple-frontends-per-thread / multiple-threads-per-keeper interaction
  (ledger OQ-12) — explicitly not decided here, by directive.
- The exact keeper-thread-start surface (contract 2) — pinned by `keeper.md`
  (batch-6 sibling), not this file.
- Everything the state-machine library itself carries (INTENT #168) —
  deferred to the port, by directive, not by gap.
