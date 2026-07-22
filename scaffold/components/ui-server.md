# ui-server

**Status:** L6 PLACEHOLDER stub (re-spoken round, 2026-07-21/22, INTENT
#168). **Track:** STUB — data-contract placeholder ONLY; deliberately
minimal. **Nesting:** top-level (pairing: `aui-client`).

> **⚠ NOT IMPLEMENTING NOW — AND NOT DESIGNING NOW.** The operator has
> **already refactored** the harness AUI into **ui-server + aui-client**
> (with a layered state-machine library between them) and still runs the
> old version day-to-day. This file exists so the L6 tree reflects that
> real split and so neighbor contracts have a name to point at. Nothing
> here is invented beyond the operator's statement; the real data
> contracts port from the working refactor when this leaves the stub
> track.

## What it is

The server half of the refactored audio user interface: the mesh-side
process that owns AUI state, threads/conversations, and service access,
serving one or more thin `aui-client` frontends. Between ui-server and
aui-client sits a **layered state-machine library** — the shared protocol
core both halves consume; its layering IS the data contract between them.

## Anticipated contracts (names only)

- **`ui-server ↔ aui-client`** — the state-machine-library protocol
  (port from the existing harness refactor; not authored here).
- **`aui-mesh` lineage** — ui-server inherits `aui.md`'s stance: a pure
  mesh client composing `service-lookup` + `surface-schema` +
  `pubsub-protocol`; no per-service voice edges.

## Open

Everything else — including how `aui.md`'s single-crate framing splits
across these two files, and whether the state-machine library is a shared
lib in the tree. Deferred to the port.
