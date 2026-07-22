# locks

**Status:** wave-2 design (batch 2 — Fable seat), **light-refreshed wave 3**
(intent-ledger unit `replicated-kv-locks-refresh`): UTC-nanosecond
acquisition-timestamp sourcing via the mesh time module (concern 2, INTENT
#112/#116), the queues/cron batch-3 fold reflected in Relationships, and the
PARKED OQ-1 boundary restated explicitly (concern 6). No structural change —
identity model, acquire protocol, strictness knob, and the merge reconciler
are unchanged from wave 2. **Nesting:** internal lib of mesh
(module `lib/mesh::locks`), Ring 3 in mesh-core's internal layering, riding
`replicated-kv`. Designs the module left OPEN in `mesh.md` concern 10 ("the full
partition/merge semantics... it has to generalize to be reliable in an infinite
set of circumstances") against INTENT #71 (slug+UUID identity, LOCKED), #84 (CAP
honesty blessed + the required catchable partition-merge error), #95/#101
(event-ID semaphores, per-trigger), and #32 (naive timestamp/LWW blessed).

## Charter

`locks` is the distributed-semaphore library buried inside every mesh daemon: the
one mechanism by which any service — and mesh's own libs (`queues`, `cron`, the
execution-engine's distributed trigger coordination) — obtains time-bounded,
leased, counted permits against a named semaphore, mesh-wide. A semaphore with
`threshold = 1` is a mutex; higher thresholds are counting semaphores. It owns:
**semaphore identity** (slug + instance-UUID, INTENT #71), the **acquire protocol**
(knowledge of a grant distributes to ALL currently-reachable nodes BEFORE the
client is confirmed holding — INTENT #71), **leases, renewal, holder-death expiry,
and fencing tokens**, the **partition/merge reconciler** with its first-class
catchable `PartitionMergeExceeded` error (INTENT #84, aligned with `types`'
`error/locks.rs`), and the **ephemeral event-ID semaphore class** queues consumes
per-trigger (INTENT #95/#101). It does NOT own: queue/trigger semantics (that is
`queues` — queues is merely a `locks-api` client), KV replication mechanics or
anti-entropy (that is `replicated-kv` — locks rides it), reachability detection
(that is `network-topology`), process supervision or zombie-killing (`supervision`
/ mesh-core), and it is never a standalone service (INTENT #54). It never
*revokes* a hold and never *retracts* work done under one — detection and
per-application resolution, not enforcement, is its contract at the merge edge.

**The honest guarantee (operator-blessed, INTENT #84):** a confirmed hold is
*exclusive among the nodes that could currently hear each other at grant time,
and divergence is detected and surfaced — never silently absorbed — at merge.*
Nothing stronger is claimed anywhere in this design.

## Impossibility boundaries — stated plainly

These are laws of physics, not defects; the design's job is to handle the edges
elegantly (operator: "we cannot violate the laws of physics"):

1. **No mutual exclusion + availability + partition tolerance together.** Under
   partition, each connected component can independently grant the "same" slug.
   We choose availability inside each component (the operator's walk-along Pi
   must keep working offline), and pay for it with after-the-fact merge
   detection. Callers who want the CP trade instead get it per-call via the
   `Strictness::WholeFleet` knob (concern 4) — unavailable under partition, by
   definition, not by bug.
2. **"All reachable nodes know" is a statement about the past.** The reachable
   set is a snapshot at grant time; a partition can begin one instant after
   confirmation. The grant receipt records *which* nodes acked so the honesty is
   auditable, not implied.
3. **Merge detection is detection, not prevention.** Side effects performed under
   two partitioned holds have already happened when the partitions merge. The
   `PartitionMergeExceeded` error is the seam where the *application* decides
   what reconciliation means (operator: "handled per-application").
4. **A crashed node and a partitioned node are indistinguishable** from the
   other side. locks treats them identically (concern 3's uniform
   owner-unreachable path) rather than pretending to tell them apart.
5. **A paused holder can believe it still holds after lease expiry** (laptop lid,
   GC pause, SIGSTOP). Fencing tokens (concern 5) are the mitigation; a resource
   that ignores fence numbers is unprotected past expiry, and that is stated,
   not hidden.
6. **Clocks are naive.** LWW timestamps and tie-breaks use wall clocks with a
   deterministic node-id tiebreak (INTENT #32, operator-blessed). A skewed clock
   can win a consolidation tie-break; acceptable at personal-mesh scale, flagged
   for the record. **Refined, not repealed (concern 2):** the wall clock behind
   `created_at`/`granted_at` is now `MeshClock`'s neighbor-corrected UTC
   (INTENT #112/#116, network-topology.md concern 6), not a raw local clock —
   fewer ties in practice, same naive-timestamp model in principle; the HLC
   ratchet underneath the KV envelope is unaffected either way.

## Primary design concerns

### 1. Identity: slug + instance-UUID, and why re-acquire mints a new UUID (INTENT #71 — LOCKED)

A **slug** is the human name (`SemSlug`, dot-segmented lower-snake, e.g.
`vdb.promote.customers-db`, `queues.dlq-main.<event_id>`). A **semaphore
instance** is `(slug, InstanceId)` where `InstanceId` is a UUID minted by exactly
one node — the **owner node** — at instance creation. Holds are taken against an
*instance*, never against a bare slug. Consequences, each load-bearing:

- **Partition twins don't collide.** Two partitions each creating slug `X` mint
  two different UUIDs; in the KV store they are two different keys
  (`locks/instance/<slug>/<uuid>`), so LWW never silently merges them into one
  record. Divergence is *structurally visible* at merge instead of being
  destroyed by timestamp-wins — this is the whole point of the UUID.
- **Re-acquiring a slug after the instance dies mints a NEW UUID**, making
  non-identity explicit: a `HoldToken` from a previous instance can never be
  confused with (or validated against) the new one — stale tokens fail with
  `UnknownHold`/`LeaseExpired`, never falsely succeed.
- **One node owns each UUID** (INTENT #71's "possibly force one node to own each
  UUID" — adopted, and made load-bearing): the owner node **serializes all
  grants and releases for its instance**. Within a connected component there is
  exactly one live instance per slug and one serialization point, so two clients
  in the *same* partition can never double-grant — no multi-writer LWW race on a
  counter, ever. Distribution (concern 2) spreads *knowledge* of the owner's
  decisions; it never votes on them.

An instance dies by **retirement** (merge consolidation, concern 6) or by
**expiring empty** (`Ephemeral` class, concern 7; or a `Durable` instance whose
owner is gone and whose last hold expired — swept after a tombstone TTL).

### 2. The acquire protocol — reachable-ack before confirmation (INTENT #71 — LOCKED)

The operator's locked requirement: *knowledge of the acquisition distributes to
all reachable nodes BEFORE the client is told it holds the lock.* The protocol,
end to end (all hops ride mesh-transport envelopes; the client only ever talks
to its local `:3649`):

1. **Client → local daemon:** `Acquire { slug, permits, threshold, wait, lease,
   strictness }` via `locks-api`.
2. **Resolve the live instance** for the slug from the local KV view. If one
   exists and its owner is reachable → route the request to the owner daemon
   (PeerLink, one hop). If none exists — or the owner is unreachable (concern 3)
   — the local daemon **mints a new instance** (new UUID, itself as owner).
3. **Owner serializes:** checks `confirmed + provisional permits + requested ≤
   threshold`. If contended: `NoWait` → `Err(Contended)`; `Block{timeout}` →
   enter the owner-local FIFO waiter queue (volatile, deliberately NOT
   replicated — see Controversial decisions).
4. **Provisional grant:** owner writes the `HoldRecord` (state `Provisional`,
   fence = next per-instance fence number) to the `locks/` keyspace.
5. **Eager flush — the locked step:** the owner pushes the write to **every
   currently-reachable peer daemon** and awaits acks:
   `KvHandle::put_flush(key, value) -> FlushReceipt { acked, unreachable }`
   (a seam locks REQUIRES of `replicated-kv` — see Relationships; this is
   an eager, synchronous propagation, not the periodic anti-entropy pass).
   A peer that is "reachable" per `network-topology` but does not ack within
   the flush timeout is *reclassified as unreachable for this grant* and
   recorded as such — the grant proceeds without it (it will learn at
   resync, concern 6). Reachability shrinking during the wait never blocks a
   grant; only the owner's own death does (→ client times out → retries →
   mints a twin: the uniform path, concern 3).
6. **Confirm:** owner flips the record to `Confirmed { acked_by }` (second
   `put_flush`, piggybacked), and replies. The client receives
   `HoldToken { slug, instance, hold_id, fence, lease_expires_at, acked_nodes,
   minted_fresh }`. Only now does the client hold the lock.

The receipt fields `acked_nodes` and `minted_fresh` make the guarantee's *scope*
first-class data: an application that cares can see exactly which nodes knew at
confirmation and whether this grant created a fresh (possibly-twin) instance.

**Timestamp precision — UTC nanosecond via the mesh time module (INTENT
#112/#116).** `InstanceRecord.created_at` (minted at step 2) and
`HoldRecord.granted_at` (stamped at step 6, the owner's local clock) are read
from **`MeshClock::now_utc()`** — network-topology's neighbor-ping-corrected
clock register (`network-topology.md` concern 6, "Mesh time authority"; the
register itself lives on the mesh-core Ring-0 seam that concern splits out) —
**not** raw `SystemTime::now()`, at the type's native nanosecond resolution
(`chrono::DateTime<Utc>` is nanosecond-precision internally; nothing here
truncates it for display). This is the operator's own framing of "the
semaphore," verbatim (#112): *"when you try to acquire it, creates a UTC
timestamp down to the nanosecond — whoever gets it."* Reading the corrected
register rather than an uncorrected per-node clock buys exactly one thing,
stated honestly: it **shrinks, but does not eliminate**, the odds that two
owner nodes minting twin instances at nearly the same instant land on a
genuine `created_at` tie in the merge reconciler (concern 6) — the
deterministic `InstanceId` tiebreak still exists for the residual case, and
boundary 6 ("clocks are naive") stands: this is a better-corrected wall clock,
not a causal or vector clock, and a skewed/lagging neighbor can still win a
close tiebreak (network-topology.md's own honesty note on its filtering).
**This is a different timestamp from replicated-kv's `Version.ts_ms`** (the
HLC ratchet that governs the *KV envelope's* own LWW merge/apply decision for
the `locks/` keyspace entries these records ride in — kept exactly as
designed, unchanged by this note, replicated-kv.md concern 1). `created_at`/
`granted_at` are lock-domain fields *inside* that entry's value, used only for
locks' own tiebreak/ordering logic — reading `MeshClock` here threads no new
dependency: it is the same Ring-0-adjacent register replicated-kv's HLC ratchet
already reads (network-topology.md concern 6's register/control-loop split),
so locks gains a more-honest timestamp without gaining a new party.

### 3. Owner death ≡ partition ≡ flap: ONE divergence mechanism (the "infinite circumstances" answer)

The operator's bar: "it has to generalize to be reliable in an infinite set of
circumstances." The design meets it by having exactly **one** divergence-creating
path and exactly **one** repair path, so every failure permutation — clean
partition, owner crash, asymmetric reachability, flapping link, walk-along Pi,
node decommissioned forever — composes out of the same two moves:

- **Divergence is created only by minting.** An acquirer that cannot reach a
  live instance's owner (crashed, partitioned, flapping — indistinguishable,
  boundary 4) mints a fresh instance and proceeds under the honest guarantee.
  There is no ownership-migration protocol, no failover election, no special
  case for "owner died" vs "owner unreachable" — nothing to get wrong in the
  permutations. The old instance's holds simply live out their leases.
- **Divergence is repaired only by the merge reconciler** (concern 6), which
  runs on KV convergence regardless of *why* two live instances of one slug
  exist. Overlap exposure is bounded above by the maximum remaining lease of
  the old instance's holds — leases are what make minting safe rather than
  reckless.

### 4. The strictness knob — the CAP dial is per-call, not global

`Acquire.strictness` (default `Reachable`) makes the physics trade explicit at
each call site instead of baked into the library:

- **`Reachable`** (default; the operator's semantics): grant when all
  *currently-reachable* nodes ack. Available under partition; twin-able;
  merge-reconciled. This is the honest guarantee verbatim.
- **`WholeFleet`**: grant only if **every registered mesh node** (the
  service-registry's live node set — definition flagged in Open questions) acks
  the flush. Under any partition this fails fast with
  `Err(FleetNotFullyReachable { missing })` — a caller that must never twin
  (e.g. VDB's copy/verify/switch promotion lock, INTENT #86) opts into
  unavailability knowingly. No twin instance is ever minted on this path.

Two modes only, both boring; anything fancier (quorum counts, region scopes) is
deliberately rejected until a real consumer needs it.

### 5. Leases, renewal, holder death, fencing

- **Every hold is leased** (`lease` requested at acquire; default 30s, min 1s,
  max 10min — fill-time constants). The holder renews via
  `Renew { hold_token, extend }` through its local daemon to the owner
  (recommended cadence: lease/3). Renewal extends `lease_expires_at` via a
  normal `put_flush` (best-effort eager; a renewal that can't reach a peer does
  not fail the renewal — expiry convergence handles stragglers).
- **Expiry:** when a lease passes unexpired-renewal, the *owner* flips the hold
  to `Expired` (flushed), returning the permits, and emits
  `locks.expired.<slug>` on pub/sub. If the owner itself is gone, every daemon's
  reconciler treats a hold whose `lease_expires_at + skew_grace` has passed as
  expired for counting purposes — expiry is convergent because it is a pure
  function of the record + local clock (naive clocks accepted, boundary 6).
- **The late holder** (paused, partitioned, slow) discovers death on its next
  `Renew`/`Release`: `Err(LeaseExpired)` — catchable, and the signal that its
  protected work is no longer protected.
- **Fencing tokens:** `fence: u64` is strictly monotonic **per instance**
  (owner-assigned). A resource guarding writes (e.g. VDB's switch-under-a-lock)
  records the highest fence it has seen and rejects lower ones — the standard
  stale-holder defense. Honesty note: fence order is total only within an
  instance; across partition twins there is no true order — cross-instance
  comparison falls back to `(granted_at, instance_id)` naive-timestamp ordering
  (INTENT #32), and the merge reconciler is the real arbiter there.

### 6. Merge reconciliation + `PartitionMergeExceeded` (INTENT #84 — the required error, first-class)

> **Re-spoken-round note (2026-07-21/22, INTENT #163):** the
> "authority node" conclusion from the coordinator round is **WITHDRAWN
> pending a dedicated discussion** (see overview.md's re-spoken section).
> The operator's alternative paradigm lands closest to THIS concern:
> single-node write-blessing **via semaphore within the mesh**, and on
> partition-merge **one node in the newly-joined network steps through the
> merge and asks each affected application how to resolve** — merge
> resolution pushed back to services, "no need for a central authority
> node ever." Nothing below is redesigned now; this concern's
> deterministic reconciler + `PartitionMergeExceeded` surface is where
> that discussion will attach.

> **PARKED — do not decide (OQ-1, ledger §B.1/§C, restated here.)** Whether a
> dedicated authority node exists, whether it lives in the cloud, and whether
> it blesses anything beyond lock conditions are the operator's to settle —
> not this file's. What THIS file's boundary already is, and stays, regardless
> of how OQ-1 resolves: **locks works among the nodes that can currently reach
> each other, full stop** — grants, flushes, and the reconciler below operate
> only over the live-reachable set (concern 2) or fail fast under
> `Strictness::WholeFleet` (concern 4); there is no dependency on any
> authority, cloud or otherwise, anywhere in the acquire/renew/merge path, and
> none is threaded in by this note. The operator's **no-authority
> merge-delegation alternative** (INTENT #163 — a per-mesh write-blessing
> semaphore, with partition-merge resolution pushed back to each affected
> application rather than a central node) is listed here as the **open
> fork**, not adopted: this concern's deterministic reconciler +
> `PartitionMergeExceeded` surface already *is* the no-central-authority shape
> (every daemon reconciles independently from converged KV, per-application
> resolution via the error, concern 6 below) — but whether it ever grows into
> the more general "one node asks each affected application" semaphore
> `BlessingTarget` describes is the parked question, not a foregone
> conclusion. Concretely: `chassis.md` concern 7 already defaults its
> `BlessingTarget` seam (the client-half outbox for consistency-requiring
> changes, INTENT #157) to **this exact reconciler surface**, stubbed and
> one-line-swappable to a future cloud authority without rewriting chassis or
> any other contract — locks is the boring-provisional target, not a
> commitment to be the permanent one. No `authority` crate, no
> `authority-blessing` contract family, exists or is implied by this file.

Every daemon's locks lib subscribes to the `locks/` keyspace
(`KvHandle::subscribe`). When resync after a rejoin (bidirectional, both sides
learning — INTENT #71) converges the KV, each daemon independently runs the same
deterministic reconciler per slug:

1. **Gather** all `Live` instances of the slug and their unexpired
   `Confirmed` holds.
2. **One instance → nothing to do.** (The overwhelmingly common case.)
3. **Multiple instances, total holds ≤ threshold → silent consolidation.**
   Deterministically elect the canonical instance — earliest `created_at`,
   tiebreak lexicographically-lowest `InstanceId` (every node computes the same
   answer from the same converged data; no coordination round needed). The
   canonical instance's owner **adopts** the twins' surviving holds (re-recorded
   under the canonical instance, original fence order preserved by
   `(granted_at, instance_id)`); twins flip to
   `Retired { into: canonical }`. Existing HoldTokens against retired instances
   remain renewable/releasable — the owner maps them via the retirement pointer
   — so a well-behaved holder never notices a benign merge. Emits
   `locks.consolidated.<slug>` (observability, not an error).
4. **Multiple instances, total holds > threshold → the required error.** The
   slug enters `MergeConflict` state (a flushed marker record). While in it:
   - every `Renew` and every `Acquire` on the slug returns
     `Err(LockError::PartitionMergeExceeded(PartitionMerge {...}))` — the
     specific, catchable, matchable error type (a named variant on `types`'
     `LockError`, per types.md's `error/locks.rs` — never a stringly leaf);
   - every current holder is additionally notified on pub/sub
     (`locks.merge.<slug>`) so control loops learn without waiting for their
     renewal tick;
   - **no hold is revoked and no permit is granted.** Existing holds live out
     their leases; the *applications* holding them decide — release, finish,
     compensate — per-application, exactly as blessed.
   - The conflict **clears automatically** when releases/expiries bring the
     total within threshold; the reconciler then consolidates per step 3. There
     is no `resolve` verb — resolution is just releasing, which keeps the API
     surface boring and un-gameable.

**Threshold mismatch between twins** (each partition declared a different
threshold): surfaced inside the same `PartitionMerge` payload
(`thresholds_seen`), and the *minimum* declared threshold governs conflict
arithmetic (the conservative read). Flagged in Open questions for operator
taste.

### 7. The event-ID semaphore class — the queues integration (INTENT #95/#101)

Queues' per-trigger ~exactly-once rides `locks` unchanged — same API, one
dedicated class:

- A trigger that declares `requires_event_semaphore: true` (the PER-TRIGGER
  choice, INTENT #101) causes queues to call
  `Acquire { slug: "queues.<queue>.<event_id>", threshold: 1, wait: NoWait,
  lease: <visibility_timeout>, class: Ephemeral }` before delivering the
  assembled payload to the handler; `Release` on handler ack; lease expiry (a
  dead handler) returns the event to visibility for redelivery — SQS-shaped by
  construction (INTENT #89).
- **`SemClass::Ephemeral`** exists so millions of event-ID semaphores don't
  bloat the KV: an Ephemeral instance auto-retires the moment its last hold
  releases/expires, and its records carry a short tombstone GC-after TTL
  (minutes, fill-time constant). `Durable` (the default class) keeps normal
  tombstone discipline.
- **The elegant edge:** two partitions each processing the same event once is
  precisely a partition-twin on the event-ID semaphore — at merge, threshold 1
  with two consumed holds surfaces `PartitionMergeExceeded` to queues, which
  routes it into **the trigger's per-case duplicate-handling seam** (INTENT
  #95: "a seam for duplicate handling on a per-case basis"). The at-least-once
  honesty and the CAP honesty are the same honesty, expressed once.

The execution-engine's distributed trigger coordination (INTENT #62; wave2-plan
row 21) is a plain `locks-api` consumer with slugs like
`ee.<database>.<table>.<key>` — no special surface, listed only so the
harmonizer knows the party.

### 8. State model in the KV (keyspace `locks/`)

All records are LWW registers in `replicated-kv`, but **each key has a single
writer** (the instance's owner node) during normal operation, so LWW only ever
arbitrates the cases it is safe for: merge-time co-existence of *different*
keys, and retirement/expiry markers.

```
locks/instance/<slug>/<instance_id> -> InstanceRecord
locks/hold/<instance_id>/<hold_id>  -> HoldRecord
locks/conflict/<slug>               -> MergeConflictMarker (present only during conflict)
```

```rust
// Rust-flavored; final field names are the harmonizer's. Lands in types (locks domain).
struct InstanceRecord {
    slug: SemSlug, instance: InstanceId /*Uuid*/, owner_node: NodeId,
    threshold: u32, class: SemClass /* Durable | Ephemeral */,
    created_at: DateTime<Utc>,
    status: InstanceStatus, // Live | Retired { into: Option<InstanceId>, at } | Expired { at }
}
struct HoldRecord {
    hold_id: HoldId /*Uuid*/, instance: InstanceId, slug: SemSlug,
    permits: u32, holder_service: String /*slug*/, holder_node: NodeId,
    fence: u64,                       // per-instance monotonic, owner-assigned
    granted_at: DateTime<Utc>, lease_expires_at: DateTime<Utc>,
    state: HoldState,                 // Provisional | Confirmed { acked_by: Vec<NodeId> }
                                      // | Released { at } | Expired { at }
    strictness: Strictness,           // Reachable | WholeFleet (provenance of the grant)
}
```

Waiter queues are owner-local and volatile — never in the KV (see Controversial
decisions #4). Tombstoned (`Released`/`Expired`/`Retired`) records are swept
after a GC TTL; sweep discipline mirrors service-registry's tombstone rules.

## Relationships / edges

- **any service ↔ mesh.locks** via **`locks-api`** — acquire/release/renew/query
  + the partition-merge error type; cross-cutting, surface-schema-style (ONE
  shared document, every service a party — wave2-plan §3b + flag §5.4). Known
  parties today, **updated for the wave-3 batch-3 queues/cron fold**
  (`queues.md` concern 10 — `cron` is now a tombstone, absorbed into
  `TriggerSource::Schedule`, so it is no longer a separate party here):
  - **`queues`**, riding the *same* event-ID semaphore composition for **two**
    distinct call sites, both consumers of this file's identical
    acquire/release surface, neither a new mechanism: (1) the **queue-pull**
    discovery-claims-ownership model (INTENT #112 — every processor races to
    acquire the per-event semaphore rather than one leased owner, "reliability
    over speed"; the UTC-nanosecond acquisition timestamp above (concern 2) is
    this file's answer to the operator's own framing of that race); and (2)
    the **absorbed schedule evaluator's single-fire** semaphore (`queues.md`
    concern 10 — an `Anywhere`-target occurrence races the identical
    `Acquire{slug: occurrence_id, threshold: 1}` shape cron used to call
    directly; cron itself no longer exists as a party, only `queues` does).
  - `execution-engine` (distributed trigger coordination — reaches locks over
    the wire via its host app's mesh-client, since shared libs are not
    contract parties), `vdb` (promotion locks, INTENT #86), `vfs`/`gc`
    (lock-with-expiry on managed entries — candidate convergence, flagged).
  *(authored: scaffold/contracts/locks-api.md)*
- **`replicated-kv`** — **internal-lib seam, not a contract edge** (compiled-in
  sibling, INTENT #29 exception). locks consumes `KvHandle` (get / put /
  subscribe) **plus one seam it REQUIRES beyond mesh-core's sketch:**
  `put_flush(key, value) -> FlushReceipt { acked: Vec<NodeId>, unreachable:
  Vec<NodeId> }` — eager push to all reachable peers with ack collection,
  synchronous, distinct from the periodic anti-entropy pass. **Co-batch
  reconciliation required** (wave2-plan batch-2 note: locks ⇄ replicated-kv
  share drafts mid-batch); if replicated-kv declines, locks must consume
  mesh-core's `PeerTransport` directly and duplicate propagation machinery —
  the strictly worse outcome. See friction points.
- **`network-topology`** — in-process read of the current reachable peer set
  (routing + flush-set computation + `self_offline` awareness); sibling lib,
  not a contract edge. mesh-core's `MeshContext` must expose this view to
  Ring 3 (flagged — the mesh-core seam list currently gives Ring 3 only
  `KvHandle`). **Wave-3 addition:** also the in-process read of `MeshClock`
  (the corrected-wall-clock register `network-topology.md` concern 6 measures
  and mesh-core's Ring-0 holds) for `created_at`/`granted_at` stamping
  (concern 2) — the identical register replicated-kv's HLC ratchet already
  reads (INTENT #112/#116), so this is not a new dependency, only a second use
  of one already flagged for Ring-3 exposure above.
- **`pubsub-relay`** — locks emits observability/notification events on the
  reserved topic prefix **`locks.*`** (`locks.merge.<slug>`,
  `locks.consolidated.<slug>`, `locks.expired.<slug>`) as a normal internal
  publisher. The pubsub-relay topic-prefix table needs the additive `locks.*`
  row (taxonomy is data, so additive — flagged for the harmonizer).
- **`types`** — library dependency: `LockError` lives in `types`'
  `error/locks.rs` (types.md already reserves it, INTENT #84); the
  `HoldToken`/`PartitionMerge` structs land in a `types` locks domain module
  (they appear in ≥2 crates' public signatures — passes the inclusion test).
  Full shape authored in scaffold/contracts/locks-api.md.
- **`supervision` / mesh-core ProcessControl** — none. Lock-holder death is
  handled by lease expiry alone; locks does not watch processes. (Deliberate:
  keeps the failure model uniform with remote holders, which supervision can
  never see anyway.)

## Nesting

Parent: mesh | Children: none. Module `lib/mesh::locks` (Ring 3), compiled into
`bin/mesh`, never standalone (INTENT #54). The client half of `locks-api` is
part of `mesh-client`'s thin surface, same as registry/pub-sub access.

## Thoroughness level

**implementation-ready** — identity model, the six-step acquire protocol, the
strictness knob, lease/fencing/expiry rules, the deterministic merge reconciler
with consolidation + conflict semantics, the Ephemeral event-ID class, the KV
key/record layout, and the full error taxonomy are decided and specified.
**approach-sketched** in exactly two spots, both externally owned: (a) the
`put_flush` seam's final shape (co-batch with `replicated-kv`); (b) exact
timeout/TTL constants (flush timeout, lease bounds, tombstone GC, skew grace) —
fill-time tuning, not design forks.

## Assigned design-depth

**Fable** (wave2-plan batch-2 Fable seat), single Component-Designer pass (this
file), grounded on mesh.md concern 10, mesh-core's ring seams, types.md's error
restructuring, pubsub-relay's envelope/topic design, service-registry's
LWW/lease/tombstone prior art, and INTENT #32/#71/#84/#95/#101.

## Suggested fill-model

implementation-ready + high complexity → **strong model for the core**
(owner serialization + flush + reconciler): the logic is fully specified but the
concurrency is subtle and this is the wave's most safety-critical lib — do not
send the reconciler or the acquire path to a cheap tier. The API plumbing,
record types, and pub/sub emissions are transcription-grade → cheap model OK.
**Fill AFTER replicated-kv** lands `put_flush` (hard dependency), and write the
partition tests (below) before the reconciler, not after. Property-style tests
(random partition/merge/lease schedules asserting the two invariants: never >
threshold confirmed within one connected component; every over-threshold merge
surfaces the error) are the single highest-value fill artifact.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `locks-api` (any service ↔ mesh.locks) — acquire/release/renew distributed semaphores + the catchable partition-merge error. → `scaffold/contracts/locks-api.md`

Also a party to (authored elsewhere / cross-cutting): `pubsub-protocol` — see `scaffold/contracts/`.

Component-side note: the `replicated-kv::put_flush` seam (NOT a contract —
in-process, same daemon) is recorded in `scaffold/contracts/locks-api.md`.

