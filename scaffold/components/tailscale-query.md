# tailscale-query — ABSORBED INTO network-topology (tombstone-with-content)

> **⚠ TOMBSTONE — `tailscale-query` IS NO LONGER A STANDALONE CRATE (wave-3
> ledger consolidation F9a / D5; seed-bishop critic-loop synthesis ACCEPT[BORING]).**
> There is no `lib/tailscale` crate and no `substrate-tailscale` dependency. The
> Tailscale query surface is now the **internal `query` submodule of
> `network-topology`** (`lib/mesh/src/topology/query/`). network-topology was
> already the **sole** consumer of this surface (completion-router was dropped as
> a co-consumer at the wave-2 contract round), so this is a
> sole-producer-into-sole-consumer merge: it removes a crate and a cross-crate
> `map_err` seam without losing a single type or guarantee. See
> `scaffold/components/network-topology.md` (§ "Absorbed submodule: `query`") for
> the current design; this file is retained as the record of what moved and why,
> and as the home of the still-live surface detail. The wave-2 standalone-crate
> design lives in git history (`git log -- scaffold/components/tailscale-query.md`).

> **Operator-verbatim tension recorded (not silently dropped).** The operator
> asked verbatim for "**a crate just to query Tailscale**" (INTENT #24) and this
> module was framed as the cleanest "extract to a shared library later" candidate
> (INTENT #25). The consolidation **supersedes** the separate-crate framing — but
> the **extraction seam is preserved**: the `query` submodule keeps its
> zero-mesh-dependency internal boundary (it imports nothing else from `mesh`;
> only `serde`/`serde_json`/`chrono`/`thiserror` + the timeout helper), so
> re-promoting it to a standalone crate is a one-move `cargo new` + path change,
> not a redesign. This keeps the fold reversible with a one-line edit if the
> operator objects to losing the crate (design-around rule: prefer the placeholder
> that can be un-chosen).

## What tailscale-query was (for context)

A standalone, standardized, extensible L0 library for querying the Tailscale
network behind a typed Rust API — "a crate just to query Tailscale." It wrapped
`tailscale status --json`, parsed it into stable public structs, and handed back
facts plus a rich catchable error taxonomy. It owned **only** querying, running
the subprocess, and parsing: no polling loop, no snapshot diffing, no async
runtime, no topology/routing semantics, no mesh awareness. Its sole consumer
(`network-topology`) decided what the facts *meant*.

## What moved into network-topology's `query` submodule (content summary)

Everything tailscale-query owned survives, relocated to
`lib/mesh/src/topology/query/`, unchanged in substance:

- **`TailscaleQuery` trait** — one typed method per subcommand; `status()` is the
  only method today. **Extensibility = adding a method, never breaking one:**
  future subcommands (`netcheck`, `ping`, `whois`, …) arrive as NEW trait methods
  with a default body returning `TailscaleError::Unsupported`, so a new method
  never breaks an existing impl and never touches `status()`'s shape (INTENT #24
  made structural).
- **Private serde subset, decoupled from the public types, tolerant of unknown
  fields** — `#[serde(default)]` on every field, NO `deny_unknown_fields`. New
  Tailscale fields are silently ignored; a dropped optional field defaults; only a
  structurally-missing `Self`/`BackendState` surfaces as `TailscaleError::Parse`.
  Tailscale JSON churn never crosses the submodule boundary — the reason this was
  worth its own boundary rather than a raw `serde_json::Value` poke.
- **The load-bearing catchable error taxonomy** — `BinaryNotFound` /
  `DaemonNotRunning` / `NotLoggedIn` / `Timeout` / `Subprocess` / `Parse` /
  `Unsupported`. network-topology's hardest job (distinguishing "this device lost
  the tailnet" from "a peer went down" from "we can't tell") is **decided by which
  error `status()` returns**, so the taxonomy must stay catchable and precise.
- **Synchronous trait, deliberately** — `status()` blocks on a subprocess; kept
  sync to sidestep `dyn`-compatibility of `async fn` in traits, and because it is
  never on a request path (network-topology polls it under `spawn_blocking` +
  `timeout`). The submodule pulls in no async runtime.
- **Subprocess discipline** — `RealTailscale` discovers the binary in order
  (explicit override → `TAILSCALE_BIN` → `tailscale` on `PATH` → known macOS
  bundle path `/Applications/Tailscale.app/Contents/MacOS/Tailscale` → known Linux
  path, fail-fast `BinaryNotFound`) and enforces a hard wall-clock timeout
  (kill-child-on-hang → `Timeout`; `std::process::Command` has no native timeout).
- **`FakeTailscale` — the first-class scriptable fixture** that makes the whole L1
  mesh layer testable without a tailnet: `fixed(snapshot)` / `script(seq)` /
  `from_json_fixture(path)` (exercises the private-serde parse path against real
  captured `--json`) / `failing(err)`. It scripts an ordered sequence of
  `Result<StatusSnapshot, TailscaleError>` so a network-topology test can drive
  snapshot → snapshot' (peer left) → `Err(DaemonNotRunning)` (self offline) →
  snapshot'' (self back online) through a single fake and assert exact transitions.
  This is the linchpin of network-topology's conformance tests.

## What changed (and only this)

- The crate `substrate-tailscale` disappears; the code lives at
  `lib/mesh/src/topology/query/`.
- The public types (`StatusSnapshot`, `PeerStatus`, `BackendState`,
  `TailscaleError`) are now compiled inside `mesh`. They were single-crate-scoped
  already (only network-topology read them), so they do **not** move to `types`.
  The `SubstrateError::Tailscale(String)` mapping variant (flagged for the `types`
  owner) stays — mesh's workspace `Result` still wraps `TailscaleError`.
- The `tailscale-status` "contract" was already a compiled-in Rust API surface
  (no bytes cross `:3649`), so nothing wire-level changes. Its file is retained as
  a **network-topology-internal module-boundary record** (party line updated) —
  see `scaffold/contracts/tailscale-status.md`.

## Relationships (post-fold)

None external. The surface is consumed intra-component by network-topology's poll
loop. The submodule consumes nothing in-workspace (preserving the
crate-extraction seam).
