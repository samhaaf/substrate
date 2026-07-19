# Contract: locks-api

## Parties

- **any service** (via `mesh-client`) — acquires/releases/renews distributed semaphores
- **mesh** (`mesh.locks`, the L2 internal library) — the semaphore authority

Cross-cutting, surface-schema-style: ONE shared document, every service is a
party. Struct vocabulary is homed in `types` (a `locks` domain module +
`error/locks.rs`); behavior is `locks`'. Notifications ride `pubsub-protocol` on
the `locks.*` prefix. `types` did not propose an independent shape for this pair,
so it is authored from `locks`' proposal plus `types`' error-taxonomy discipline.

## Purpose

The one wire surface for distributed semaphores (INTENT #71 — semaphores as a
first-order mesh library). A service asks its LOCAL mesh daemon (`:3649`, kind
strings `locks.acquire` / `locks.release` / `locks.renew` / `locks.query`) to
acquire leased permits; the daemon distributes acquisition knowledge to all
reachable nodes **before** confirming, then returns a `HoldToken` — or a member
of the typed `LockError` taxonomy, of which **`PartitionMergeExceeded` is the
operator-required first-class catchable variant** (INTENT #84). Identity is
**slug + per-instance UUID**: a slug names the logical semaphore, and each mint
is a UUID-tagged instance — partition twins are two UUID instances of one slug.

## Schema

### Requests (service → local daemon)

```rust
struct AcquireReq {
    slug: SemSlug,          // dot-segmented, validated like a TopicPath
    permits: u32,           // default 1
    threshold: u32,         // default 1 (mutex); first-mint fixes it per instance
    wait: WaitMode,         // NoWait | Block { timeout: Duration }
    lease: Duration,        // clamped to [min, max]
    strictness: Strictness, // Reachable (default) | WholeFleet
    class: SemClass,        // Durable (default) | Ephemeral
    holder: String,         // service slug; daemon-verified vs registration (anti-spoof, as pubsub)
}
struct ReleaseReq { token: HoldToken }
struct RenewReq   { token: HoldToken, extend: Duration }
struct QueryReq   { slug: SemSlug }
```

### Responses

```rust
struct HoldToken {                 // opaque to bearers; an introspectable honesty receipt
    slug: SemSlug, instance: InstanceId, hold_id: HoldId,
    fence: u64,                    // per-instance monotonic fencing token
    lease_expires_at: DateTime<Utc>,
    acked_nodes: Vec<NodeId>,      // who knew at confirmation (the guarantee scope)
    minted_fresh: bool,            // true => this acquire created the instance
}
struct RenewAck  { lease_expires_at: DateTime<Utc> }
struct SemStatus {                 // Query response; also the surface-schema feed
    slug: SemSlug, threshold: u32,
    instances: Vec<InstanceSummary>,  // >1 only mid-merge
    holds: Vec<HoldSummary>, waiters_local: u32,
    conflict: Option<PartitionMerge>,
}
```

### The error taxonomy (`types::error::locks::LockError`)

```rust
enum LockError {
    Contended        { slug: SemSlug, holders: Vec<HoldSummary> }, // NoWait, full
    AcquireTimeout   { slug: SemSlug, waited: Duration },          // Block deadline
    LeaseExpired     { hold_id: HoldId },                          // late holder's signal
    UnknownHold      { hold_id: HoldId },                          // stale token, dead/retired instance
    InvalidRequest   { detail: String },                          // bad slug/permits>threshold/lease bounds
    FleetNotFullyReachable { missing: Vec<NodeId> },              // WholeFleet only
    PartitionMergeExceeded(PartitionMerge),                        // REQUIRED (INTENT #84)
    // wire-crossing enum: reserves #[serde(other)] Unknown
}
struct PartitionMerge {            // matchable, self-describing conflict evidence
    slug: SemSlug, threshold: u32,     // governing (minimum-declared) threshold
    thresholds_seen: Vec<u32>,         // >1 distinct => twins disagreed
    total_confirmed: u32,              // > threshold, by definition of this error
    instances: Vec<InstanceSummary>,   // each twin: instance, owner_node, holds, created_at
    detected_at: DateTime<Utc>,
    your_hold: Option<HoldId>,         // set when delivered to a current holder (via Renew)
}
```

### Pub/sub notification topics (payloads are the structs above, on the standard `Envelope`)

- `locks.merge.<slug>` → `PartitionMerge`
- `locks.consolidated.<slug>` → benign-merge notice
- `locks.expired.<slug>` → `HoldSummary`

Per-entity subscription = `Exact`-match on the leaf (pubsub-protocol convention).

## Error cases

- `Contended` and `AcquireTimeout` are **normal outcomes**, not faults.
- `PartitionMergeExceeded` is returned by **`Acquire` and `Renew`** while a slug
  is in conflict; **`Release` ALWAYS succeeds during conflict** (releasing is the
  resolution path and must never be blocked).
- `LeaseExpired` on `Release` is informational-idempotent: the hold is already
  gone; the caller's cleanup proceeds.
- Daemon-unreachable / relay failures are `mesh-transport`'s `PeerUnreachable`,
  not `LockError` — transport and lock semantics stay layered.

## Version sensitivity

**HIGH for persisted records, MEDIUM for the wire enum.**
- `HoldToken` and `PartitionMerge` **cross the wire and outlive processes** —
  additive-only, every new field `#[serde(default)]`, no `deny_unknown_fields`.
- `LockError` reserves a catch-all so an older consumer degrades to "some lock
  error" rather than a deserialize failure — **but `PartitionMergeExceeded`'s
  presence and shape is the one variant applications match on, so it is FROZEN at
  harmonization**: renaming or renarrowing it is a **breaking** change requiring
  an operator round.
- KV records (`InstanceRecord` / `HoldRecord`) cross nodes via replication on
  mixed versions — same additive discipline; the reconciler must tolerate unknown
  extra fields (INTENT #66).
- `strictness` / `class` may gain variants (`#[serde(other)]` → treated
  conservatively as `Reachable` / `Durable` by old nodes, chosen so an unknown
  *stricter* mode never silently weakens the guarantee).

## Reconciliation notes

- **Single-proposer pair.** Only `locks` proposed a shape; `types` contributed
  only the wire-discipline guardrails and the error-taxonomy home. No cross-party
  disagreement to resolve. Authored from `locks`' proposal + `types`' guardrail-4
  discipline applied to `HoldToken`/`PartitionMerge`/`LockError`.
- **`PartitionMergeExceeded` frozen at harmonization** is the deviation from a
  purely-additive default: because it is the operator-mandated (INTENT #84)
  application match point, its shape is pinned harder than the rest of the enum.
- **Seam note carried forward (NOT a contract edge):** `locks-api`'s guarantee is
  implementable only on an eager propagate-and-ack write
  (`replicated-kv::put_flush → FlushReceipt`) over mesh-core's `PeerTransport`.
  This is an internal-lib seam (no contract file) but the single hardest
  cross-module dependency in batch 2 — recorded so the harmonizer doesn't lose it.
- **Slug convention shared with queues:** the event-ID semaphore queues uses for
  cross-node exactly-once (`SemaphoreChoice::EventId`) keys a `locks` slug by
  event ID — the `queues-api` ↔ `locks-api` reconciliation point; both contracts
  agree the slug is dot-segmented and validated identically to a topic path.

## Example data

Drawn from the shared example world — the walk-along Pi (INTENT #84 color).
**macbook** and **pi** partition. Both sides' `vdb` acquire
`vdb.promote.customers-db` (threshold 1).

macbook confirms against instance `a3f2…`:
```jsonc
{ "slug": "vdb.promote.customers-db", "instance": "a3f2...",
  "hold_id": "h-01", "fence": 1,
  "lease_expires_at": "2026-07-19T18:30:00Z",
  "acked_nodes": ["macbook"], "minted_fresh": false }
```

pi cannot reach `a3f2…`'s owner, so it mints twin `9c81…`:
```jsonc
{ "slug": "vdb.promote.customers-db", "instance": "9c81...",
  "hold_id": "h-02", "fence": 1,
  "lease_expires_at": "2026-07-19T18:30:05Z",
  "acked_nodes": ["pi"], "minted_fresh": true }
```

pi rejoins; KV resyncs bidirectionally; every daemon's reconciler counts 2
confirmed holds > threshold 1 → conflict flushed. Both holders' next `Renew`
returns:
```jsonc
{ "LockError": { "PartitionMergeExceeded": {
    "slug": "vdb.promote.customers-db",
    "threshold": 1, "thresholds_seen": [1],
    "total_confirmed": 2,
    "instances": [
      { "instance": "a3f2...", "owner_node": "macbook", "holds": ["h-01"] },
      { "instance": "9c81...", "owner_node": "pi",      "holds": ["h-02"] } ],
    "detected_at": "2026-07-19T18:25:00Z",
    "your_hold": "h-02" } } }
```

Both holders also see `locks.merge.vdb.promote.customers-db` on pub/sub. `vdb`'s
per-application handling releases the pi-side hold and re-verifies its copy step;
the conflict clears on release; the reconciler consolidates `9c81…` →
`Retired { into: a3f2… }`.
