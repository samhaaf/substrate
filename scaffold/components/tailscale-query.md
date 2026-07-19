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
back facts and a rich, catchable error taxonomy; its two consumers
(`network-topology` and `completion-router`) decide what those facts *mean*. It is
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
   interval, `completion-router`'s NodeRegistry refreshes it on a background loop.
   Both callers wrap it in `tokio::task::spawn_blocking`. The crate pulls in no
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
   Without this, no `network-topology`/`completion-router` test and no CI box
   without a tailnet could run at all.

## Relationships / edges

- {network-topology, completion-router} via `tailscale-status` — this crate is the
  producer of parsed self+peer status snapshots to both siblings
  (scaffold/contracts/tailscale-status.md). **Note:** this is an *in-process
  compiled-in library API*, not a mesh WebSocket edge (port `:3649` is not
  involved) — see Proposed contracts for why the stub still earns a documented
  shape and the wave2-plan inconsistency this surfaces.

No other edges. This crate consumes nothing in-workspace (see Nesting).

## Nesting

Parent (organizational): mesh | Children: none.

`substrate-tailscale` lives at `lib/tailscale` and is compiled into `mesh` where
`network-topology` and `completion-router` use it. It is a **crate sibling** of
mesh, deliberately **not** a `lib/mesh` module and deliberately **not** dependent
on `mesh` or on anything else in the workspace:

- Own crate because the operator asked for it verbatim ("a crate just to query
  Tailscale"), it has two in-repo consumers, it is the cleanest "extract to a
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
against `network-topology`'s and `completion-router`'s concrete reads (both are
enumerated in Proposed contracts).

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

## Proposed contracts (wave 2)

### `tailscale-status` — `substrate-tailscale` → {`network-topology`,
`completion-router`}

**Nature (flag for the Contract Harmonizer):** this is a **compiled-in Rust API
contract**, not a mesh WS/`:3649` data contract. wave2-plan §3a lists it as a live
mesh-internal contract stub, while wave2-plan §3's closing note says "all shared-lib
consumption (`types`, `tailscale-query`, `mesh-client`, `execution-engine`) are
internal library dependencies, deliberately NOT contract edges." Those two lines
conflict. **Proposed resolution:** keep the `tailscale-status` stub, but scope it to
document the crate's **public trait + struct surface** (the shape both siblings
compile against), not a wire message — exactly as a shared lib's public API *is* its
contract. No bytes cross the mesh port for this edge. Flagged for the per-pair round.

**Purpose:** give `network-topology` and `completion-router` one stable, typed view
of Tailscale self+peer status and a catchable failure taxonomy, decoupled from
Tailscale's raw JSON.

**Public surface (Rust-flavored pseudocode):**

```rust
// ── The trait (the extension point) ─────────────────────────────────────
pub trait TailscaleQuery: Send + Sync {
    /// `tailscale status --json`, parsed. The only method today.
    fn status(&self) -> Result<StatusSnapshot, TailscaleError>;

    // Future subcommands arrive as NEW methods with default `Unsupported`
    // bodies so adding them never breaks an existing impl or `status()`:
    // fn netcheck(&self) -> Result<NetcheckReport, TailscaleError> { Err(TailscaleError::Unsupported("netcheck")) }
    // fn ping(&self, target: &str) -> Result<PingReport, TailscaleError> { Err(TailscaleError::Unsupported("ping")) }
    // fn whois(&self, addr: IpAddr) -> Result<WhoisReport, TailscaleError> { Err(TailscaleError::Unsupported("whois")) }
}

// ── Public snapshot types (stable; decoupled from private serde structs) ─
pub struct StatusSnapshot {
    pub captured_at: DateTime<Utc>,   // set by this crate at parse time; anchors topology diffing
    pub cli_version: String,          // Tailscale `Version`, for diagnostics/schema-drift triage
    pub backend_state: BackendState,  // typed, not a raw string
    pub self_node: PeerStatus,        // Tailscale `Self`
    pub peers: Vec<PeerStatus>,       // Tailscale `Peer` map, flattened & sorted by id
    pub magic_dns_suffix: String,     // e.g. "tailXXXX.ts.net"
    pub tailnet_name: Option<String>, // `CurrentTailnet.Name`
    pub health: Vec<String>,          // tailscaled health warnings, verbatim
}

pub struct PeerStatus {
    pub id: String,                   // StableNodeID (Tailscale `ID`)
    pub host_name: String,            // `HostName`
    pub dns_name: String,             // `DNSName` (FQDN incl. MagicDNS suffix)
    pub os: String,                   // `OS` ("linux","macOS",…)
    pub tailscale_ips: Vec<IpAddr>,   // `TailscaleIPs` (100.x + fd7a:… )
    pub tags: Vec<String>,            // ACL tags, e.g. ["tag:substrate"]; empty if untagged
    pub user_id: u64,                 // `UserID` (owner)
    pub online: bool,                 // `Online`
    pub active: bool,                 // `Active` (has recent traffic)
    pub exit_node: bool,              // `ExitNode` (currently serving as exit)
    pub exit_node_option: bool,       // `ExitNodeOption` (advertises exit capability)
    pub last_seen: Option<DateTime<Utc>>, // `LastSeen` (None for self / always-online)
    pub key_expiry: Option<DateTime<Utc>>,// `KeyExpiry`
}

pub enum BackendState {              // from Tailscale `BackendState`
    Running, Starting, Stopped, NeedsLogin, NoState, NeedsMachineAuth,
    Unknown(String),                 // forward-compatible catch-all
}

// ── Error taxonomy (catchable; the load-bearing part for network-topology) ─
pub enum TailscaleError {
    BinaryNotFound { searched: Vec<String> }, // no tailscale executable anywhere we looked
    DaemonNotRunning { detail: String },      // tailscaled down / socket unavailable
    NotLoggedIn { backend_state: BackendState }, // reached tailscaled, but off the tailnet
    Timeout { after: Duration },              // subprocess exceeded the wall-clock budget; child killed
    Subprocess { code: Option<i32>, stderr: String }, // other non-zero exit
    Parse { detail: String },                 // JSON deserialize failed → schema drift, treat as bug
    Unsupported(&'static str),                // a not-yet-implemented subcommand method
}

// ── Impls ────────────────────────────────────────────────────────────────
pub struct RealTailscale { binary: PathBuf, timeout: Duration }
impl RealTailscale {
    pub fn discover() -> Result<Self, TailscaleError>; // PATH → TAILSCALE_BIN → macOS bundle → linux path
    pub fn with_binary(path: impl Into<PathBuf>) -> Self;
    pub fn timeout(self, d: Duration) -> Self;
}
impl TailscaleQuery for RealTailscale { /* spawn, wait-with-timeout, classify exit, parse private structs */ }

pub struct FakeTailscale { /* Mutex<VecDeque<Result<StatusSnapshot, TailscaleError>>> */ }
impl FakeTailscale {
    pub fn fixed(snapshot: StatusSnapshot) -> Self;              // same snapshot every call
    pub fn script(seq: Vec<Result<StatusSnapshot, TailscaleError>>) -> Self; // pop in order; last repeats
    pub fn from_json_fixture(path: &Path) -> Result<Self, TailscaleError>;    // parse a captured --json file
    pub fn failing(err: TailscaleError) -> Self;                // always Err — simulate daemon-down
}
```

**How each consumer reads it (drives the Harmonizer's field trim):**

- `completion-router` (NodeRegistry): filters `peers` where `online` and a
  substrate-identifying predicate holds (`tags.contains("tag:substrate")` **or**
  `host_name` matches a `substrate-*` convention), then uses `tailscale_ips[0]` +
  the default inference port to build `NodeEndpoint`s. Needs: `host_name`,
  `dns_name`, `tailscale_ips`, `tags`, `online`. Does not care about `self_node` /
  `backend_state`.
- `network-topology`: diffs `self_node.online` + `backend_state` for
  `self_offline`/`self_online`, and diffs the `peers` set (`id` as identity;
  `online`, `host_name` for events) for `peer_joined/left/online/offline`. Needs
  `captured_at` to order snapshots, and — critically — **needs the `Err(...)`
  variant**, not a snapshot, to trigger `self_offline` with peers marked *unknown*
  rather than *offline*.

**Error cases (contract-level):**
- `status()` returns `Err(BinaryNotFound|DaemonNotRunning|NotLoggedIn|Timeout|
  Subprocess)` → consumers MUST treat self as offline/degraded and peer state as
  **unknown** (never synthesize "peer offline" from a failed self-query).
- `status()` returns `Ok` with `backend_state != Running` or `self_node.online ==
  false` → reached tailscaled but not on the tailnet; `self_offline`, peers as
  reported-but-stale.
- `Err(Parse)` → schema-drift bug, surfaced/logged distinctly; not a network state.
- `Err(Unsupported)` → only reachable via a future method a Fake didn't override.

**Version-sensitivity notes:**
- **Tailscale CLI JSON version:** absorbed inside the crate. Private serde structs
  use `#[serde(default)]` + no `deny_unknown_fields`; `cli_version` is surfaced in
  the snapshot for triage. Additive Tailscale fields never break callers; a
  structurally-missing field (`Self`/`BackendState`) becomes `Parse`.
- **Public-struct evolution:** treated as an ordinary lib API change. New fields are
  additive (default-constructible); the trait's method **set** grows, existing
  method **signatures** are frozen. Because the crate is compiled in (shared lib, no
  runtime version tracking — INTENT #45), consumers recompile against the new
  surface; there is no on-the-wire version negotiation to manage.
- **New `SubstrateError` variant (touches `types`, flag for that owner/Harmonizer):**
  mesh maps `TailscaleError` into the workspace error as
  `SubstrateError::Tailscale(String)` — one new flat, string-payload variant matching
  the existing `Store`/`Db`/`Engine` per-domain pattern in `lib/types/src/error.rs`,
  preserving `types`' zero-dependency invariant. Proposed here, not authored (no edits
  to shared files this pass).
