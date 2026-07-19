# Contract: tailscale-status

## Parties
`tailscale-query` (crate `substrate-tailscale`, L0) → **`mesh.network-topology`**
(sole consumer).
*(completion-router DROPPED as a co-consumer, wave-2 — see Reconciliation
notes; the wave-1 party list `{network-topology, completion-router}` and
wave2-plan §3a are updated by this file.)*

**Nature flag (for the harmonizer).** This is a **compiled-in Rust API
contract**, not a mesh WS / `:3649` data contract — no bytes cross the mesh
port for this edge. wave2-plan §3a lists it as a mesh-internal contract stub,
while §3's closing note says shared-lib consumption is "deliberately NOT
contract edges." Those lines conflict; the adopted resolution (from
`tailscale-query.md`) keeps the stub but scopes it to document the crate's
**public trait + struct surface** — a shared lib's public API *is* its
contract. Authored from `tailscale-query.md` (producer/owner) reconciled with
`network-topology.md`'s consumer reads.

## Purpose
Hand `network-topology` one stable, typed view of Tailscale self+peer status
and a **catchable failure taxonomy**, fully decoupled from Tailscale's raw
churning JSON. The error taxonomy is load-bearing, not decoration:
`network-topology`'s hardest job — distinguishing "this device lost the
tailnet" (`SelfOffline`) from "a peer went down" from "we simply can't tell" —
is decided by *which* error `status()` returns. The trait is synchronous by
design (`status()` blocks on a subprocess; it is never on a request path; the
consumer wraps it in `spawn_blocking` + `timeout`), and extensible by adding
methods, never mutating `status()`.

## Schema
The public trait + stable structs both live in `substrate-tailscale` (decoupled
from private serde structs that absorb Tailscale JSON churn):

```rust
// ── the trait (the extension point) ─────────────────────────────────────
pub trait TailscaleQuery: Send + Sync {
    /// `tailscale status --json`, parsed. The only method today.
    fn status(&self) -> Result<StatusSnapshot, TailscaleError>;
    // Future subcommands arrive as NEW methods with default `Unsupported`
    // bodies so adding one never breaks an existing impl or `status()`:
    //   fn netcheck(&self) -> Result<NetcheckReport, TailscaleError> { Err(TailscaleError::Unsupported("netcheck")) }
    //   fn ping(&self, target: &str) -> Result<PingReport, TailscaleError> { Err(TailscaleError::Unsupported("ping")) }
    //   fn whois(&self, addr: IpAddr) -> Result<WhoisReport, TailscaleError> { Err(TailscaleError::Unsupported("whois")) }
}

// ── public snapshot types (stable; decoupled from private serde) ────────
pub struct StatusSnapshot {
    pub captured_at:      DateTime<Utc>,   // set at parse time; anchors topology diffing
    pub cli_version:      String,          // Tailscale `Version`, for schema-drift triage
    pub backend_state:    BackendState,    // typed, not a raw string
    pub self_node:        PeerStatus,      // Tailscale `Self` (is_self == true)
    pub peers:            Vec<PeerStatus>, // `Peer` map, flattened & sorted by id
    pub magic_dns_suffix: String,          // e.g. "tailXXXX.ts.net"
    pub tailnet_name:     Option<String>,  // `CurrentTailnet.Name`
    pub health:           Vec<String>,     // tailscaled health warnings, verbatim passthrough
}

pub struct PeerStatus {
    pub id:               String,          // StableNodeID — the identity key (survives IP churn)
    pub host_name:        String,          // `HostName`
    pub dns_name:         String,          // `DNSName` (FQDN incl. MagicDNS suffix)
    pub os:               String,          // "linux","macOS",…
    pub tailscale_ips:    Vec<IpAddr>,     // 100.x + fd7a:…
    pub tags:             Vec<String>,     // ACL tags, e.g. ["tag:substrate"]; empty if untagged
    pub user_id:          u64,             // owner
    pub online:           bool,
    pub active:           bool,            // has recent traffic
    pub exit_node:        bool,            // currently serving as exit
    pub exit_node_option: bool,            // advertises exit capability
    pub is_self:          bool,            // true only for self_node
    pub last_seen:        Option<DateTime<Utc>>, // None for self / always-online
    pub key_expiry:       Option<DateTime<Utc>>,
    pub relay:            Option<String>,  // DERP relay region, informational (network-topology's ask)
}

pub enum BackendState {                    // from Tailscale `BackendState`
    Running, Starting, Stopped, NeedsLogin, NoState, NeedsMachineAuth,
    Unknown(String),                       // forward-compatible catch-all
}

// ── the error taxonomy (catchable; the load-bearing part) ───────────────
pub enum TailscaleError {
    BinaryNotFound   { searched: Vec<String> },          // no tailscale executable anywhere we looked
    DaemonNotRunning { detail: String },                 // tailscaled down / socket unavailable
    NotLoggedIn      { backend_state: BackendState },     // reached tailscaled, but off the tailnet
    Timeout          { after: Duration },                // exceeded wall-clock budget; child killed
    Subprocess       { code: Option<i32>, stderr: String }, // other non-zero exit
    Parse            { detail: String },                 // deserialize failed → schema drift, treat as bug
    Unsupported(&'static str),                           // a not-yet-implemented subcommand method
}

// ── impls ───────────────────────────────────────────────────────────────
pub struct RealTailscale { binary: PathBuf, timeout: Duration }
impl RealTailscale {
    pub fn discover() -> Result<Self, TailscaleError>; // override → TAILSCALE_BIN → PATH → macOS bundle → linux path
    pub fn with_binary(path: impl Into<PathBuf>) -> Self;
    pub fn timeout(self, d: Duration) -> Self;
}
pub struct FakeTailscale { /* Mutex<VecDeque<Result<StatusSnapshot, TailscaleError>>> */ }
impl FakeTailscale {
    pub fn fixed(snapshot: StatusSnapshot) -> Self;                             // same snapshot every call
    pub fn script(seq: Vec<Result<StatusSnapshot, TailscaleError>>) -> Self;    // pop in order; last repeats
    pub fn from_json_fixture(path: &Path) -> Result<Self, TailscaleError>;      // parse a captured --json file
    pub fn failing(err: TailscaleError) -> Self;                               // always Err — simulate daemon-down
}
```

**How the (sole) consumer reads it** — drives the field trim:
`network-topology` diffs `self_node.online` + `backend_state` for
`SelfOffline`/`SelfOnline`, diffs the `peers` set (`id` as identity;
`online`, `host_name`, `tags` for events) for peer transitions, uses
`captured_at` to order snapshots, and — critically — depends on the
`Err(..)` variant (not a snapshot) to trigger `SelfOffline` with peers marked
**Unknown** rather than **Offline**.

## Error cases
Contract-level, mapped by `network-topology` (concern 1) to self-offline
observations:
- `status()` → `Err(BinaryNotFound | DaemonNotRunning | NotLoggedIn | Timeout |
  Subprocess)` → consumer MUST treat self as offline/degraded and peer state
  as **Unknown** (never synthesize "peer offline" from a failed self-query).
  Cause mapping: `BinaryNotFound`/`DaemonNotRunning`/`Subprocess` →
  `TailscaleUnavailable`; `NotLoggedIn` → `BackendNotRunning(state)` or
  `TailnetUnreachable`; `Timeout` → `PollTimeout`.
- `status()` → `Ok` with `backend_state != Running` OR `self_node.online ==
  false` → reached tailscaled but not on the tailnet → `SelfOffline`, peers
  reported-but-stale.
- `Err(Parse)` → schema-drift **bug**, surfaced/logged distinctly; NOT a
  network state, never an event.
- `Err(Unsupported)` → only reachable via a future method a Fake didn't
  override.

## Version sensitivity
- **Tailscale CLI JSON churn is absorbed inside the crate** (private serde
  structs use `#[serde(default)]` + no `deny_unknown_fields`; `cli_version` is
  surfaced for triage). Additive Tailscale fields never break the consumer; a
  structurally-missing field (`Self`/`BackendState`) becomes `Parse`.
- **Public-struct evolution is an ordinary lib API change.** New fields are
  additive (default-constructible); the trait's method *set* grows, existing
  method *signatures* are frozen. `BackendState::Unknown(String)` absorbs new
  backend states. Because the crate is **compiled in** (shared lib, no runtime
  version tracking — INTENT #45), consumers recompile against the new surface;
  there is NO on-the-wire version negotiation — the key contrast with the
  wire-crossing contracts (`service-lookup`, `kv-replication`).
- **New `SubstrateError` variant** (flag for the `types` owner): mesh maps
  `TailscaleError` into the workspace error as `SubstrateError::Tailscale(String)`
  — one flat string-payload variant matching the existing `Store`/`Db`/`Engine`
  pattern, preserving `types`' zero-dependency invariant.

## Reconciliation notes
- **completion-router DROPPED as a party (resolved deviation).** Both sides
  now agree on this subtraction: `completion-router.md` concern 1 deleted its
  bespoke `tailscale status` scan (fleet membership moved to `service-registry`
  + `network-topology`), and its Proposed-contracts section explicitly
  *recommends striking itself* as a co-consumer. `tailscale-query.md` still
  named completion-router as a second consumer; this file resolves the two by
  making **`network-topology` the sole consumer**. Consequence recorded for
  the harmonizer: the party references in `tailscale-query.md` and
  wave2-plan §3a (`tailscale-status → {network-topology, completion-router}`)
  should be updated to drop completion-router. No schema is lost — this is a
  pure subtraction; the fields completion-router once used
  (`tailscale_ips`, `tags`) remain (network-topology reads them too).
- **Struct-name reconciliation (compatible shapes, one winner).**
  `network-topology.md` sketched a near-identical `TailscaleStatus` /
  `PeerStatus` with minor naming/shape differences; `tailscale-query.md` (the
  producer that owns the surface) is authoritative, so its names win:
  - `TailscaleStatus` → **`StatusSnapshot`** (producer's name).
  - `sampled_at` → **`captured_at`** (producer's name; same meaning).
  - `PeerStatus.stable_id` → **`id`** (producer's name; StableNodeID identity).
    Note: `network-topology`'s own OUTPUT type `PeerRef` (in `network-events`)
    keeps a `stable_id` field name — that is a different, downstream struct;
    the INPUT struct here uses `id`. The device-name-vs-stable-key naming edge
    is flagged for the harmonizer (also noted in `network-events`).
  - `self` vs `peers` split: the producer folds self into
    `peers`-shaped `PeerStatus` via `self_node` + `is_self`, rather than
    network-topology's separate handling — adopted, since `is_self` covers the
    consumer's need.
  - `relay: Option<String>` (DERP region) — network-topology asked for it;
    **kept** as an informational field on `PeerStatus` (additive, harmless).
- **Deviation from the old stub.** The wave-1 stub said "producer of parsed
  self+peer status snapshots to both siblings." This contract narrows the
  consumer set to one and documents the full trait+struct surface (the stub
  deferred schema).

## Example data
The example world: observer **macbook** (macOS, the operator's laptop),
peer **pi** (`tag:substrate`, running qwen3-4b for project **demo**). Note the
macOS binary-discovery reality: `macbook` has no `tailscale` on `PATH`, only
`/Applications/Tailscale.app/Contents/MacOS/Tailscale`.

**A successful `status()` on macbook:**
```jsonc
Ok(StatusSnapshot {
  captured_at:      "2026-07-19T00:00:00Z",
  cli_version:      "1.78.1",
  backend_state:    Running,
  magic_dns_suffix: "tailXXXX.ts.net",
  tailnet_name:     Some("substrate.ts.net"),
  health:           [],
  self_node: PeerStatus { id: "nMACSELF01", host_name: "macbook", dns_name: "macbook.tailXXXX.ts.net",
    os: "macOS", tailscale_ips: ["100.64.0.3"], tags: ["tag:substrate"], user_id: 1,
    online: true, active: true, exit_node: false, exit_node_option: false, is_self: true,
    last_seen: None, key_expiry: Some("2026-10-01T00:00:00Z"), relay: Some("sfo") },
  peers: [ PeerStatus { id: "nABC123CNTRL", host_name: "pi", dns_name: "pi.tailXXXX.ts.net",
    os: "linux", tailscale_ips: ["100.64.0.7"], tags: ["tag:substrate"], user_id: 1,
    online: true, active: false, exit_node: false, exit_node_option: false, is_self: false,
    last_seen: Some("2026-07-19T00:00:00Z"), key_expiry: Some("2026-10-01T00:00:00Z"), relay: Some("sfo") } ],
})
```
`network-topology` diffs this against its prior baseline → no change → no
event (the retained `net.topology` Snapshot already reflects it).

**A scripted `FakeTailscale` driving the self-offline path in a test:**
```rust
FakeTailscale::script(vec![
    Ok(snapshot_pi_online),                                   // baseline: pi visible
    Ok(snapshot_pi_gone),                                     // pi dropped from status (peer left)
    Err(TailscaleError::DaemonNotRunning { detail: "socket refused".into() }), // -> SelfOffline, peers Unknown
    Ok(snapshot_pi_online),                                   // -> SelfOnline { fresh snapshot }
]);
```
This is exactly the sequence that lets a `network-topology` conformance test
assert `PeerLeft(pi)` → `SelfOffline(TailscaleUnavailable)` (NOT `PeerOffline`
for pi) → `SelfOnline{topology}` through a single fake, with no tailnet
present.
