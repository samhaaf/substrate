# Contract: aui-mesh

## Parties
- `aui` (L6 stub, the voice/audio interface) `<->` `mesh` (L1, the local daemon
  on `:3649`).

*(Stub-track — aui's assigned pair (wave2-plan §3c). Content deferred until `aui`
leaves the stub track; when it does, author `aui-mesh` as a BINDING of three
already-designed cross-cutting contracts, not as new surface area.)*

## Purpose
The single entry surface by which the audio interface drives the whole mesh. The
load-bearing design point: **AUI introduces no new protocol.** It is a plain mesh
client — a thin composition of three already-designed cross-cutting edges:
- `service-lookup` — discover/resolve services by slug.
- `surface-schema` — read each service's render + call description, consumed as a
  **voice grammar** (the surface-schema "gift": the same boring schema that
  drives the dashboard drives voice).
- `pubsub-protocol` — subscribe to live events for spoken output.

## Rough shape
- AUI registers/resolves on `:3649` like any client; the two addressing classes
  (INTENT #59) map naturally to voice: *"talk to inference, don't care which
  node"* (service-anywhere) vs *"talk to inference on node N"* (service-on-node).
- Each service's `SurfaceSchema.actions[]` becomes an utterable command grammar;
  `sections[]`/`fields[]` become speakable readouts (honoring `Honesty::Estimate`
  → spoken hedge, the audio analogue of the tilde+tooltip).
- Live `pubsub` subscriptions (`inference.*`, `ccd.*`, `network.*`) drive spoken
  notifications; the top-level-agent-only speaking rule is an AUI-side concern,
  not carried on the wire.
- An AUI conversation is a CCD-style thread carrying the optional
  `(project, environment)` linkage — reuse `ccd-projects` / `agent-management`
  rather than inventing a parallel conversation record.

## Open questions
- Whether AUI owns conversation records or delegates them to CCD (leaning
  delegate — no new identity type).
- Whether a "voice grammar" needs any schema hint beyond `surface-schema`
  (e.g. an optional `speakable` field) — preferably none; keep the schema boring.
- Relationship to the separate interface-layer branch — explicitly OUT OF SCOPE
  this pass (aui.md §"Relationship to the interface-layer branch").
