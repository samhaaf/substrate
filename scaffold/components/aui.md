# aui

> **⚠ STUB TRACK — NOT IMPLEMENTING NOW.** This file is DESIGN NOTES +
> ANTICIPATED DATA CONTRACTS only, per the wave-2 batch-8 plan and INTENT #42.
> Nothing here is implementation-ready: no schemas, no code, no decomposition
> into libs. `aui` is brought into scope *"in the same capacity we brought in
> org — just a design node and the data contracts."* It is a faithful
> placeholder whose anticipated needs are already **almost entirely satisfied by
> contracts that exist for other reasons** (see the centerpiece below).
> Thoroughness: **requirements-only.**

**Status:** WAVE-2 net-new L6 stub (no substrate repo code). **Layer:** L6
organization plane. **Nesting:** top-level app-crate (crate=app, INTENT #22) — a
daemon/CLI entry point when it is eventually built, never a library others link
(INTENT #29). **Track:** STUB. **Model tier:** Opus.

**Provenance note (not this pass's scope).** The AUI already exists as the
operator's working audio interface in the harness (`~/code/harness/apps/aui`) —
this very interface, which captured INTENT and drives the whole project by voice
(the operator "never touches a keyboard or screen"). Substrate's `aui` is the
*future port/generalization* of that surface into a native mesh client. Whether
substrate's `aui` supersedes, wraps, or shares code with the harness AUI is an
open operator question, deliberately unanswered here.

## Charter (requirements, operator's words where quoted)

`aui` is the **voice/audio interface over the whole mesh** — the surface through
which the operator interfaces with the mesh network and every service on it by
voice. INTENT #42, verbatim:

> "Substrate is going to become — I've got my audio user interface, and we're
> explicitly avoiding that right now, but I want to be able to interface with my
> mesh network and all the services on it over my audio user interface. We might
> just bring it into scope in the same capacity we brought in org — just a
> design node and the data contracts."

So `aui`'s entire scope this pass is: **a design node + its anticipated data
contracts.** It is one more **mesh client** on the single local port (`:3649`,
INTENT #57/#58) — it registers/resolves like everyone else, subscribes to the
pub/sub protocol like everyone else, and is not aware of the ports of the
services it drives (single-port locality, INTENT #58).

**Boundary — what `aui` does NOT own.** It is not a new integration surface for
each service, not a completion runtime (inference), not an agent manager (ccd),
and not a knowledge/graph plane. It **composes existing service surfaces into a
voice-navigable experience**; it reimplements none of them.

## The surface-schema gift — the centerpiece

**AUI needs no second integration surface. It reads the SAME boring surface
schemas the dashboard already renders.** This is the whole reason `aui` can be a
faithful stub rather than a large design: the hard integration work is already
done, for other reasons, by contracts that exist today.

- INTENT #46 (LOCKED) made every service publish a **boring surface schema** — a
  data structure describing *(a) how to render its dashboard component and (b)
  what calls to make against the service* — and the mesh dashboard renders every
  service from that schema, "visual coherence by construction"
  (`surface-schema`). **The same schema that tells the dashboard how to render a
  panel and which calls to expose tells AUI what a service can do and how to
  invoke it by voice.**
- INTENT #16 already required the dashboard's interactive elements to carry
  **stable, semantic ids** so *"an agent can drive the exact same UI a human
  sees, without a separate API surface… Same markup, not a parallel headless
  version."* AUI is exactly that agent-shaped consumer: voice → the same
  semantic surface → the same calls. The operator's own principle is that there
  is **one** interaction surface, not a bespoke voice one per service.
- Consequence for the mesh contracts: **AUI adds no new per-service edges.** A
  new service becomes voice-drivable the moment it publishes its surface schema —
  the same act that makes it dashboard-renderable. This is the property to
  preserve when `aui` is eventually built, and the reason its anticipated
  contract (`aui-mesh`) is a **thin composition of three edges that already
  exist**, not new surface area:
  1. **`service-lookup`** — discover services + resolve endpoints (the wiring
     seam; already designed to be "cheap and broadly queryable" for exactly this
     kind of maximalist reader).
  2. **`surface-schema`** — read each service's render/interaction description
     and turn it into a voice grammar (what can I say, what does it do).
  3. **`pubsub-protocol`** (the standard WS pub/sub envelope, INTENT #53) —
     subscribe to live events so voice output reflects state as it changes
     (per-topic and, where useful, per-completion-ID subscription).

**Design intent to lock into the stub:** keep voice control a *pure consumer* of
the surface-schema/pub-sub planes. If a future voice need can't be met by the
boring surface schema, that is a signal to enrich `surface-schema` (which
benefits the dashboard too) — **not** to grow an AUI-specific integration edge.

## Thread / conversation linkage to projects

The operator drives by voice in ongoing **conversations**; those conversations
are the same kind of object CCD already tracks as **threads**, and CCD already
links **thread↔project (+ optional environment)** in its ledger (INTENT #68;
ccd.md `agent_runs.project_id?`/`environment_id?`, the `ccd-projects` edge).

- **Anticipated shape:** an AUI conversation is (or maps to) a CCD-style thread,
  carrying the same optional `(project, environment)` linkage — so "what I said
  by voice" attaches to the right project the same way an agent run does. This
  reuses `ThreadId` / `ProjectId` / `EnvRef` identities already pinned in
  ccd.md/environments.md; **no new identity type.**
- **Not decided this pass:** whether AUI owns its own conversation records or
  registers them as threads *through* CCD (the natural path, since CCD already
  owns the thread↔project ledger and `agent-management`). Recorded as an open
  question, not resolved — projects and CCD's stub-track `ccd-projects` are the
  neighbors that settle it.

## Relationship to the interface-layer branch — OUT OF SCOPE

A separate interface-layer initiative exists as its own branch/effort; it is a
**distinct initiative, explicitly out of scope here** — noted only so a future
pass doesn't conflate it with this design node. No design, no contract, no
further mention.

## Import surface (anticipated; all mesh-mediated, never Cargo-linked)

Every service below is a top-level app reached **over the wire (mesh-mediated
WS/CLI)** (INTENT #29). Only `types` + `mesh-client` are compiled-in shared
libs. The point of the surface-schema gift is that this list is **generic** —
AUI consumes the mesh's cross-cutting planes, not a fixed roster of services.

| Consumes | Via | Why |
|----------|-----|-----|
| mesh.service-registry | `service-lookup` | discover + resolve every service to drive |
| every service | `surface-schema` | the render/interaction description → voice grammar |
| mesh (pub/sub) | `pubsub-protocol` | live event subscription for spoken state |
| `ccd` (realistic) | `ccd-projects` reuse / `agent-management` | conversation-as-thread + thread↔project linkage |
| `projects` (realistic) | via CCD's thread↔project link | attach voice conversations to projects |

## Anticipated contracts (wave 2, stub track)

*Names + purpose + rough shape only — schemas deferred until `aui` leaves the
stub track. Recorded now so the cross-cutting planes stay shaped for a
voice consumer.*

- **`aui-mesh`** (aui ↔ mesh; MISSING — aui's assigned pair, wave2-plan §3c).
  *Purpose:* the single entry surface by which the audio interface drives the
  whole mesh. *Rough shape (the load-bearing point):* **NOT a new protocol —
  a thin composition of three already-designed cross-cutting edges**:
  `service-lookup` (discover/resolve), `surface-schema` (read each service's
  render+call description as a voice grammar), and `pubsub-protocol` (subscribe
  to live events for spoken output). AUI is a plain mesh client on `:3649`; the
  two addressing classes (INTENT #59) map naturally to voice — *"talk to
  inference, don't care which node"* vs *"talk to inference on node N."* When
  `aui` leaves the stub track, `aui-mesh`'s content should be **authored as a
  binding of those three contracts, not as new surface area.**

- **`ccd-projects` reuse** (aui → ccd/projects; anticipated, not a new stub).
  *Purpose:* an AUI conversation is a CCD-style thread carrying the optional
  `(project, environment)` linkage (INTENT #68). *Rough shape:* reuse ccd.md's
  existing thread↔project ledger + `agent-management`; AUI registers/labels
  conversations as threads rather than inventing a parallel record. Whether AUI
  owns conversation records or delegates to CCD is open (below). No new identity
  type; no AUI-owned contract stub this pass.

## Open questions

1. **Substrate `aui` vs. the harness AUI.** Does substrate's `aui` supersede,
   wrap, or share code with the existing `~/code/harness/apps/aui`? Provenance
   noted; the relationship is unresolved.
2. **Conversation ownership: AUI-owned vs. CCD-registered.** Are AUI
   conversations first-class AUI records, or CCD threads created through
   `agent-management`? The thread↔project ledger already lives in CCD, which
   argues for delegation — but not decided this pass.
3. **Voice grammar from a render-oriented schema.** `surface-schema` was designed
   to describe *rendering + calls* for the dashboard. Is that description rich
   enough to synthesize a voice grammar (disambiguation, confirmation prompts,
   speakable labels), or does it need a modest voice-facing enrichment? If so,
   that enrichment belongs in `surface-schema` (dashboard benefits too), NOT in
   an AUI-specific edge — per the centerpiece principle.
4. **STT/TTS placement.** Speech-to-text and text-to-speech are AUI-internal
   concerns; whether they ever touch the mesh (e.g. voice synthesis as an
   inference modality — inference is deliberately modality-agnostic, INTENT #18)
   is a future question, explicitly not designed now.
