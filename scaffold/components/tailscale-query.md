# tailscale-query

**Status:** NEW — promoted to its own crate (`lib/tailscale`, `substrate-tailscale`).
**Layer:** L0 (Foundation). **Nesting:** crate sibling of mesh, but NOT a mesh
module and NOT dependent on mesh (see Nesting).

## Charter

A standalone, standardized, extensible library for querying the Tailscale network
behind a typed Rust API. It wraps `tailscale status --json` today and is shaped so
that "tools we add as we go" (`netcheck`, `ping`, `whois`, …) become **new typed
methods** without ever changing the signature or return type of an existing one.
It parses the Tailscale CLI's JSON into **public, stable structs** describing self
and peers (id, hostname, DNSName, TailscaleIPs, ACL tags, OS, online, exit-node
role, last-seen) plus the tailnet backend state. It owns **only querying, running
the subprocess, and parsing** — it holds no polling loop, no snapshot diffing, no
async runtime, no topology or routing semantics, and no mesh awareness. It hands
back facts and a rich, catchable error taxonomy; its sole consumer
(`network-topology` — completion-router was DROPPED as a co-consumer at the
contract round; see `scaffold/contracts/tailscale-status.md` Reconciliation
notes) decides what those facts *mean*. It is
the one crate in the tree with zero in-workspace dependencies — "a crate just to
query Tailscale" (operator, verbatim) — which is also what makes it the cleanest
future shared-library extraction candidate (INTENT #25).

## Primary design concerns

The module is deliberately low-complexity (lowest-risk in the wave), so the design
work is about getting five boundaries *exactly* right so nothing downstream ever
has to refactor around them (INTENT #38, no technical debt).

1. **Extensibility = adding a method, never breaking one.** One `TailscaleQuery`
   trait; one typed method per subcommand. `status()` is the only method now.
   Future subcommands are added as **new trait methods with a default body that
   returns `TailscaleError::Unsupported`**, so a new method never breaks either
   existing impl and never touches `status()`'s shape. This is the operator's "add
   new tools as we go" (INTENT #24) made structural — the extensibility is in the
   method set, not in mutating a god-struct.

2. **Private serde subset, fully decoupled from the public types, tolerant of
   unknown fields.** Tailscale's JSON is large and churns across CLI versions
   (grounded against the live schema: top-level `Version, BackendState,
   TailscaleIPs, Self, Peer, MagicDNSSuffix, CurrentTailnet, Health, …`; each
   Self/Peer node carries ~30 fields). We deserialize into **private** structs
   that (a) name only the handful of fields we consume, (b) put `#[serde(default)]`
   on every field, and (c) do **NOT** use `deny_unknown_fields`. New Tailscale
   fields are silently ignored; a dropped *optional* field defaults; only a dropped
   *structurally-required* field (missing `Self`/`BackendState`) surfaces as
   `TailscaleError::Parse`. The public structs are the stable contract — Tailscale
   JSON churn never crosses the crate boundary. This is why the module earned its
   own crate rather than a raw `serde_json::Value` poke inside mesh.

3. **The error taxonomy is load-bearing for `network-topology`, not decoration.**
   `network-topology`'s hardest job — distinguishing *"this device lost the
   tailnet"* (`self_offline`) from *"a peer went down"* and from *"we simply can't
   tell"* — is **decided by which error `status()` returns**, so the taxonomy must
   be catchable and precise (not a flattened string). Concretely:
   `BinaryNotFound` / `DaemonNotRunning` / `NotLoggedIn` / `Timeout` /
   `Subprocess` all mean "peer state is UNKNOWN, and self is likely offline"; a
   *successful* snapshot whose `self.online == false` or whose
   `backend_state != Running` means "we reached tailscaled but we're not on the
   tailnet." Only `Parse` means "schema drift — bug, not a network condition."
   The distinctions exist so the consumer can implement peers-unknown-not-offline
   semantics correctly.

4. **Synchronous trait, deliberately.** `status()` blocks on a subprocess. Kept
   sync to sidestep the `dyn`-compatibility problem with `async fn` in traits
   (mesh-design-synthesis finding on object-safe traits) and because it is
   **never** called on a request path — `network-topology` polls it on an
   interval and wraps it in `tokio::task::spawn_blocking`. The crate pulls in no
   async runtime.

5. **Subprocess discipline: binary discovery + a hard timeout.** `tailscaled`/the
   CLI can hang, and the binary is not always on `PATH` — on macOS it commonly
   lives only at `/Applications/Tailscale.app/Contents/MacOS/Tailscale` (verified:
   this dev box has no `tailscale` on `PATH`, only the app bundle). So
   `RealTailscale` (a) **discovers** the binary in order: explicit override →
   `TAILSCALE_BIN` env → `tailscale` on `PATH` → known macOS bundle path → known
   Linux path, failing fast with `BinaryNotFound`; and (b) enforces a **wall-clock
   timeout** (default ~5s, configurable) via `wait-timeout` (or a reaper thread),
   killing the child and returning `TailscaleError::Timeout` rather than blocking a
   `spawn_blocking` worker forever. `std::process::Command` has no native timeout;
   this is a real gap the design closes on purpose.

6. **`FakeTailscale` is a first-class, *scriptable* fixture — the reason the whole
   L1 mesh layer is testable without a tailnet.** It is not just "return one canned
   snapshot." It scripts an ordered sequence of `Result<StatusSnapshot,
   TailscaleError>` responses, so a `network-topology` test can drive
   snapshot → snapshot' (peer left) → `Err(DaemonNotRunning)` (self offline) →
   snapshot'' (self back online) through a single fake and assert the exact
   transition events. It also loads a real captured `--json` fixture file so the
   private-serde parse path itself is exercised against genuine Tailscale output.
   Without this, no `network-topology` test and no CI box
   without a tailnet could run at all.

## Relationships / edges

- network-topology via `tailscale-status` — this crate is the producer of
  parsed self+peer status snapshots to its SOLE consumer
  (scaffold/contracts/tailscale-status.md; completion-router was dropped as a
  co-consumer at the contract round — its fleet membership now comes from
  `resolve_all("inference")` + network-topology). **Note:** this is an *in-process
  compiled-in library API*, not a mesh WebSocket edge (port `:3649` is not
  involved) — see scaffold/contracts/tailscale-status.md's nature flag for why the stub still earns a documented
  shape and the wave2-plan inconsistency this surfaces.

No other edges. This crate consumes nothing in-workspace (see Nesting).

## Nesting

Parent (organizational): mesh | Children: none.

`substrate-tailscale` lives at `lib/tailscale` and is compiled into `mesh` where
`network-topology` uses it. It is a **crate sibling** of
mesh, deliberately **not** a `lib/mesh` module and deliberately **not** dependent
on `mesh` or on anything else in the workspace:

- Own crate because the operator asked for it verbatim ("a crate just to query
  Tailscale"), it is the cleanest "extract to a
  shared library later" candidate (INTENT #25), and — being pure vocabulary +
  subprocess I/O with zero mesh dependency — it belongs on L0 beside `types`, not
  inside the L1 mesh kernel.
- **Recommended: zero `substrate-*` dependencies**, not even `substrate-types`.
  `TailscaleError` and the public snapshot structs are defined locally; the crate's
  only deps are `serde`/`serde_json`/`chrono`/`thiserror` (+ `wait-timeout`). This
  maximises the standalone-reuse value the operator's framing implies and keeps L0
  acyclic. The cost — mesh code returning the workspace `Result` cannot `?` a
  `TailscaleError` directly — is paid on **mesh's** side with a one-line
  `.map_err(|e| SubstrateError::Tailscale(e.to_string()))`. See Controversial
  decisions in the summary for the rejected alternative (depend on `types` to ship
  a `From` impl).

## Thoroughness level

**implementation-ready.** The trait shape, both impls, the subprocess/timeout
discipline, binary discovery, the private-serde-vs-public-types split with the
concrete field list (grounded on the live JSON), the scriptable-fake mechanics, and
the full error taxonomy are all decided below. The only genuinely open item is the
exact final public-struct field set, which the Contract Harmonizer trims/confirms
against `network-topology`'s concrete reads (enumerated in
`scaffold/contracts/tailscale-status.md`).

## Assigned design-depth

Single strong-model (Opus) Component-Designer pass, grounded on the real
`lib/mesh/src/discovery.rs` `TailscaleDiscovery`/`NodeDiscovery` stub, the live
`tailscale status --json` schema (captured field-name-only), and `lib/types`'s
existing `SubstrateError` taxonomy.

## Suggested fill-model

implementation-ready + low complexity → **cheap/fast model (Sonnet)**. A bounded
subprocess-shell-out + serde + a fixture-backed fake, fully specified here; the
Filler transcribes against the cited live stub and the captured JSON fixture.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including Reconciliation notes). Detailed proposals formerly here
are superseded by them.

- `tailscale-status` (tailscale-query → network-topology, the **sole
  consumer**) — the crate's public compiled-in Rust API surface:
  `TailscaleQuery::status()` → `StatusSnapshot`/`PeerStatus`/`BackendState`
  plus the catchable `TailscaleError` taxonomy, `RealTailscale` +
  `FakeTailscale` impls, and the version-absorption rules.
  → `scaffold/contracts/tailscale-status.md`
  - Contract resolution: **completion-router was DROPPED as a co-consumer**
    (its own concern-1 deletion; fleet membership now comes from
    service-registry + network-topology) — this file's earlier two-consumer
    framing is superseded, network-topology is the only party. A pure
    subtraction; no fields were lost.
  - The nature question this module flagged (compiled-in lib API vs mesh wire
    contract; the wave2-plan §3/§3a conflict) was resolved as proposed: the
    stub is kept, scoped to the public trait + struct surface.
  - The `SubstrateError::Tailscale(String)` mapping variant is recorded in the
    contract as a flag for the `types` owner (the crate itself stays
    zero-`substrate-*`-dependency; mesh pays the `map_err`).
