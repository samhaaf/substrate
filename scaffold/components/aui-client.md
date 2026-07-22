# aui-client

**Status:** L6 PLACEHOLDER stub (re-spoken round, 2026-07-21/22, INTENT
#168). **Track:** STUB — data-contract placeholder ONLY; deliberately
minimal. **Nesting:** top-level (pairing: `ui-server`).

> **⚠ NOT IMPLEMENTING NOW — AND NOT DESIGNING NOW.** Placeholder for the
> client half of the operator's **already-done** AUI refactor (ui-server +
> aui-client with a layered state-machine library between them). Real
> contracts port from the working refactor.

## What it is

The thin frontend half of the refactored audio user interface — audio
capture/playback and interaction, holding no mesh-side state of its own.
**The walk-along Pi runs aui-client** (INTENT #157's small client that
queues messages and sends when connected). It speaks only to `ui-server`
through the shared **layered state-machine library**; that library's
layering is the data contract.

## Anticipated contracts (names only)

- **`ui-server ↔ aui-client`** — the state-machine-library protocol (see
  `ui-server.md`; one contract, named once there).

## Open

Offline queueing semantics on the Pi, S2T placement (S2T/T2S are inference
MODALITIES with a warm-model option — INTENT #166 Q12; hardware placement
decisions were part of the withdrawn authority-node discussion, INTENT
#163), and everything else. Deferred to the port.
