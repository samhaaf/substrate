# cache

## Charter
`cache` (`lib/cache`, `CacheManager`) is the per-node, disk-backed KV/prefix
cache manager for `inference`. It leverages llama-server's slot save/restore:
after a slot prefills a prompt, that slot's KV state is written to a blob file
and indexed by the resident model's identity + the tokenized-prefix hash; on a
later completion whose prefix matches, the blob is restored so prefill is
skipped. It enforces a byte budget over one gc-managed directory (`kvcache/`),
indexes entries as metadata rows through `store`, and — the wave-2 headline —
guarantees a **restore can never be applied to the wrong weights** by binding
every entry to a *validity token* (model-content fingerprint + backend/context
configuration). Its boundary: it manages KV state ON DISK and the metadata index
+ invalidation protocol; it does NOT run the backend or issue the actual
save/restore HTTP calls (that is `engine`, which calls cache to decide *whether*
and *where*), does NOT own model weights or model eviction (`models`), does NOT
own the SQLite schema (`store`), and does NOT enforce the disk budget itself (it
drives `gc`, which does). It publishes **nothing** to the mesh — KV cache is a
node-local latency optimization with no fleet-level meaning (see "Controversial
decisions").

## Primary design concerns

1. **Validity-token keying is the whole correctness game (wave-2 core change).**
   A stale or mismatched restore silently corrupts a completion's context — the
   worst failure this subsystem can produce, and undetectable downstream. Today
   the entry is keyed on `(model_id, prompt_hash)` only. That is not sufficient:
   `model_id` is a *registry slug*, not the bytes on disk. Three distinct events
   leave `(model_id, prompt_hash)` unchanged while making a blob incompatible:
   - the registry re-points `model_id` at a different GGUF revision/quant and
     `models` re-downloads it (same slug, different weights);
   - the llama.cpp build is upgraded (`engine`'s `BackendProvisioner` installs a
     new version) — **llama-server's slot-state serialization is build-specific**,
     so an old-format blob restored into a new build fails or corrupts;
   - the context/slot configuration changes (`n_gpu_layers`, `context_size`) —
     the KV layout differs.
   Wave-2 folds all three into a **`validity_token`** — a short hash of
   `(model_fingerprint, backend_build_id, context_config)` — that the caller
   (`engine`, which alone knows what is *actually resident*) supplies at both
   save and lookup. Lookup becomes `(model_id, validity_token, prompt_hash)`. A
   changed model/build/config yields a different token, so an incompatible blob
   simply **misses** (→ normal prefill) instead of being restored. This makes
   correctness *independent of eviction ordering* — the "atomic invalidation
   race" (concern 2) degrades from a corruption bug to a wasted-disk (orphan)
   concern, which is exactly gc's department. See "Invalidation protocol."

2. **The models ↔ cache ↔ gc atomic-invalidation race (wave-1 non-obvious test,
   now designed).** `models` registers model weights with `gc` and `lock`s them
   for 24h; once the lock lapses, gc's autonomous hourly LRU sweep may evict a
   weight file at any time, with no notification to cache. Two failure shapes:
   (a) **corruption** — weight for `model_id` evicted, a *different* revision
   re-downloaded at the same slug, an old KV blob restored into it; (b)
   **orphans** — weight gone, its KV blobs linger in `kvcache/` consuming budget
   and cluttering the index. The wave-2 protocol closes both with **defense in
   depth**, no cross-crate lock or gc dependency-group needed (keeps gc boring):
   - **structural (correctness):** the validity token from concern 1 — a stale
     blob can never restore, regardless of who evicted what when;
   - **eager (prompt reclamation):** `models`' eviction/re-download path signals
     invalidation for that `model_id`, and the inference composition root calls
     `cache.purge_model(model_id)` — reclaims the orphaned blobs' bytes promptly;
   - **lazy (backstop):** on any `find_cached_prefix`, a token mismatch is
     treated as a miss *and* the stale row+blob are deleted; and gc's own
     TTL/budget sweep reclaims anything the eager path missed (a cache blob is
     always safe for gc to delete — losing one costs only a re-prefill).
   None of these three requires the others to be correct; together they make the
   race benign. `purge_model` exists in code today but **is currently wired to
   nothing** — closing that gap is the concrete wave-2 deliverable.

3. **What makes a prefix reusable, and eviction priority vs. model weights.**
   A saved blob is reusable for a new completion iff *all* hold: (i) same
   `validity_token` (⇒ same weights, same build, same context config);
   (ii) the new prompt's tokenized prefix hashes to the stored `prompt_hash`
   (v1: **exact full-prefix match**, not longest-common-prefix — see open
   questions); (iii) the blob still exists on disk; (iv) the engine has a free
   slot on the resident backend to restore into. Retention priority is
   deliberately asymmetric: **model weights outrank KV cache blobs.** A weight
   file is expensive to reconstruct (multi-GB re-download); a KV blob is cheap
   (one re-prefill). So the two live in *separate* gc-managed dirs with
   *separate* budgets (`kvcache/` vs `models/`) — cache pressure never evicts a
   weight, and under global device pressure the operator's per-dir policy makes
   `kvcache/` the first to give. Within `kvcache/`, eviction is plain LRU + TTL
   over gc (matching `models`' shape) — deliberately *not* hit-count-weighted in
   v1 (flagged, not built). This is the "budget logic mirrors models" symmetry
   the later shared-library dedup pass should collapse; keep them structurally
   identical until then.

4. **Cache blobs are ephemeral by contract — every failure falls back to
   prefill, never to error.** Restore is best-effort: if a blob was swept between
   `find_cached_prefix` returning its path and `engine` restoring it, the restore
   simply fails and `engine` prefills normally. This is why cache loss is never
   fatal and why cache needs no strong durability guarantees, no cross-node
   replication, and no place in the mesh's consistency substrate. It also frees
   the design from locking blobs during use — a brief lock is a nicety, not a
   correctness requirement.

## Wave-2 refit summary (what changes vs. wave-1)

- **Keep:** the crate's shape (`CacheManager` over `store` + gc), the
  `make_room → write → register → index` flow, per-node locality, the
  implementation-ready thoroughness.
- **Supersede:** the `(model_id, prompt_hash)`-only key → add `validity_token`;
  the never-called `purge_model` → wired into the invalidation protocol; the
  weak `entry_id` (`DefaultHasher`, 64-bit, **not stable across Rust
  versions/platforms** — a smell for a persisted primary key) → a stable
  content hash (sha256/blake3), and the id now covers the token too.
- **Add (mesh era):** `gc` is reached through the wave-3 **`GcHandle`**
  (`Embedded` standalone / `Remote` to the node's gc daemon) instead of a raw
  `Arc<GcService>` — cache's call sites are unchanged beyond the field type
  (gc.md concern 2); adopt gc's atomic **`register_and_lock`** so a fresh blob
  isn't swept in the register→sweep window (gc.md concern 6); no new mesh
  contract (see below).

## Relationships / edges
- engine ↔ cache via `kv-cache` (see scaffold/contracts/kv-cache.md) — **cache
  co-owns; authored here.** Carries the token, save/restore decisions, prefix
  lookup.
- {models, cache, engine} → gc via `gc-managed-dirs` (see
  scaffold/contracts/gc-managed-dirs.md) — cache is a consumer; the vocabulary
  is authored by `gc.md`. Cache uses `register_dir(kvcache/)`, `make_room`,
  `register_and_lock`, `touch`, `evict`, `unlock`, and subscribes to
  `EntryEvicted` as the lazy-invalidation backstop.
- cache ↔ store via `store-access` (see scaffold/contracts/store-access.md) —
  cache is a consumer; schema/observer contract authored by `store.md`. Cache
  proposes the append-only `validity_token` column delta (flagged to store).
- **No cache ↔ mesh edge.** Intentional — see "Controversial decisions."

## Nesting (if applicable)
Parent: inference | Children: (none)

## Thoroughness level
implementation-ready — keying, reuse rules, the invalidation protocol, eviction
priority, the gc/store deltas, and the GcHandle/register_and_lock adoption are
all grounded in the real `lib/cache`, `lib/store/src/kv_cache.rs`,
`lib/models`, `lib/engine/src/client.rs::slot_action`, and the frozen `gc.md`
command surface. The one genuinely deferred item is longest-common-prefix
(radix) matching (open question); v1 exact-prefix reuse is fully specified.

## Assigned design-depth
Opus 4.8 — single Component-Designer pass. Grounded on the full `lib/cache`
source and every call site (`lib/inference/src/lib.rs` wiring;
`lib/scheduler/src/selection.rs` integration note; `lib/engine/src/client.rs`
slot_action; `lib/models/src/lib.rs` eviction/re-download path;
`lib/store/src/kv_cache.rs`), plus the batch-3 `gc.md` (GcHandle, shared command
vocabulary, register_and_lock, gc-events), the sibling `store.md`/`engine.md`/
`models.md`/`inference.md` designs, and INTENT #28/#44/#53/#57/#85.

## Suggested fill-model
implementation-ready + low complexity → **cheap-to-mid model OK**, with ONE
sequencing constraint and ONE careful spot: (a) fill cache *after* store freezes
the `validity_token` column and gc freezes `register_and_lock`/`EntryEvicted`
(cache serializes against both); (b) the validity-token derivation + the
three-layer invalidation wiring is the one place to fill carefully — the rest
(budget, LRU, index CRUD) is mechanical against existing code.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `kv-cache` — (engine ↔ cache) — KV/prefix slot save/restore: lookup keyed
  `(model_id, validity_token, prompt_hash)`, two-phase save
  (`ReserveSave` → engine saves → `ConfirmSave` via gc's atomic
  `register_and_lock`; `Skip` on `DiskBudgetExceeded`, non-fatal). Upgraded
  to an authored contract at harmonization from this file's design.
  → `scaffold/contracts/kv-cache.md`
- `gc-managed-dirs` — (cache → gc) — consumer; vocabulary authored by gc.md. → `scaffold/contracts/gc-managed-dirs.md`
- `store-access` — (cache ↔ store) — consumer; schema authored by store.md. → `scaffold/contracts/store-access.md`
  - Additive ask not yet in the contract file: the `validity_token TEXT` column
    + composite lookup key on `kv_cache_entries` (flagged to store).

## Invalidation protocol (explicit — the wave-1 race, resolved)

The three layers, and the exact triggers:

1. **Structural (validity token).** Every entry carries the token of the weights
   it was saved against. Lookup compares tokens; an incompatible blob **misses**.
   *This alone prevents corruption* for all three invalidating events (model
   re-download, build upgrade, config change). No timing assumption.

2. **Eager (purge_model).** `models` is the sole owner of model-weight lifecycle.
   On the two events that invalidate blobs — (a) gc-eviction of a weight followed
   by `clear_model_file`, and (b) a *completed re-download* that replaces a
   weight file — `models` emits `ModelInvalidated{model_id}` on the inference
   event bus (composition-root wiring, per inference.md's "composition root is
   private wiring"). The root calls `cache.purge_model(model_id)`, reclaiming
   orphaned bytes promptly. Cache does **not** take a dependency on `models`;
   the root mediates. *(A pure model unload/swap — VRAM only, weights unchanged —
   does NOT invalidate: blobs stay valid for reload. Only weight-file
   deletion/replacement invalidates.)*

3. **Lazy (backstop).** (i) On `Lookup`, a token-mismatched row for the same
   `(model_id, prompt_hash)` is deleted and reported `Miss`. (ii) Cache
   subscribes to gc `EntryEvicted`; an evicted `models/` weight triggers
   `purge_model` even if the eager signal was missed (e.g. eviction happened
   while inference was down and is replayed on boot reconciliation). (iii) gc's
   own TTL/budget sweep of `kvcache/` reclaims anything else — always safe,
   since a lost blob only costs a re-prefill.

**Why no cross-crate lock or gc "reclaim group."** Because layer 1 makes
correctness ordering-independent, the remaining concern is only *disk waste*,
which is precisely what gc already handles. Introducing a gc dependency-group
(evict-weight-cascades-to-blobs atomically) would add non-boring machinery to gc
for a problem the token already solves — explicitly rejected to keep gc boring
(consistent with gc.md's "transport change, not semantics change" stance).

## Controversial decisions

- **Cache publishes NOTHING to the mesh — no `cache ↔ mesh` contract.** Argued
  both ways, landing on *no*: **For** publishing — a future affinity balancer
  could route a completion to the node that already holds its prefix. **Against**
  (decisive): (i) KV cache is a pure node-local latency optimization, invisible
  to correctness; the mesh already gets what it needs — *model inventory* and
  *node load* via `node-state-poll`, *throughput* via telemetry; a prefix index
  is none of those. (ii) Cross-node cache-affinity routing is premature
  optimization that would add a brand-new eventual-consistency surface (which
  node has which prefix, invalidated on every eviction) to the mesh — the
  opposite of boring. (iii) Prompt-derived hashes crossing nodes is needless
  spread of prompt-shaped data. The one place a human sees cache is already
  covered *without* a cache→mesh edge: `kvcache/` is a gc-managed dir, so its
  `used_bytes` flows to the dashboard through **gc-events**, and hit-rate (from
  `hit_count` in store) can surface through **api**'s node surface schema if ever
  wanted. Decision: cache owns zero mesh contracts; observability rides existing
  gc/store/api surfaces. If cross-node affinity is ever justified, it belongs in
  `completion-router` as a routing heuristic fed by `node-state-poll`, not as a
  cache-published topic.
- **Validity token defined by engine, opaque to cache.** Cache only compares
  tokens for equality; it does not know how to fingerprint weights or a backend
  build. This keeps cache free of any dependency on `models`/`engine` internals
  and puts the token's correctness where the resident-state knowledge lives
  (engine). Risk: a lazy engine that returns a constant token would reintroduce
  the corruption bug — mitigated by making "no token" a never-matching sentinel
  (caching off) rather than a wildcard.
- **Exact-prefix reuse in v1, not longest-common-prefix.** Keying on a full
  tokenized-prefix hash means only an *identical* prefix reuses a blob; a prompt
  that shares the first N tokens but diverges gets no reuse. LCP/radix matching
  would materially raise hit rates for chat/agent workloads but needs a prefix
  index and llama-server partial-restore semantics — deferred (open question),
  and the exact-prefix path is fully correct in the meantime.

## Non-obvious tests

- **Token guards a re-download (the corruption case, layer 1).** Save a blob for
  `model_id=X` under token T1; simulate a registry re-point + re-download that
  yields token T2 for the same slug; `Lookup(X, T2, h)` MUST return `Miss` (and
  delete the T1 row), never `Hit`. Then restore-path is prefill. This is the
  test that would catch a silent context corruption.
- **Build-upgrade invalidation.** Same `model_id` + same `prompt_hash`, but
  `backend_build_id` inside the token changes (llama.cpp upgrade): old blob MUST
  miss. Guards against restoring an old-format slot blob into a new llama-server.
- **Eager purge reclaims bytes on model eviction.** Register N blobs for
  `model_id=X` (bytes B), fire `ModelInvalidated{X}` → `purge_model(X)` returns
  `B`, store rows gone, gc `evict` called for each path.
- **Lazy backstop when eager is missed.** `purge_model` NOT called; instead feed
  a gc `EntryEvicted{hint:"weights:X"}` event → cache purges X's blobs anyway.
- **register→sweep window closed.** `ConfirmSave` uses `register_and_lock`; a gc
  sweep fired immediately after MUST NOT evict the just-written blob (it's
  locked); a bare `register_entry` (old behaviour) could.
- **Budget isolation.** Fill `kvcache/` to its budget; assert eviction never
  touches a `models/` weight file (separate dir, separate budget) — model
  weights outrank cache blobs (concern 3).
- **Restore failure falls back, never errors.** `Lookup` returns `Hit`; delete
  the blob out from under the caller before restore; the completion MUST prefill
  and succeed (concern 4), and the dangling row MUST be lazily cleaned.
- **`entry_id` stability across process restarts (regression on the
  DefaultHasher smell).** The id for a fixed `(model_id, token, prompt_hash)`
  MUST be identical after a restart / on another platform — asserts the move off
  `DefaultHasher` to a stable hash.
- **Standalone (Embedded gc) parity.** With `GcHandle::Embedded` (no mesh), all
  of the above behave identically to `Remote` — cache never depends on a live
  gc daemon for correctness (gc.md concern 2).

## Open questions

- **Longest-common-prefix / radix reuse.** Biggest reuse-rate lever, deferred.
  Needs a prefix-tree index over tokenized prefixes and confirmation of
  llama-server partial-restore semantics. Flag for a follow-up once v1 exact
  reuse is measured.
- **Recovery-hint typing.** Cache pins the string convention
  `"kvcache:<model_id>"` / `"weights:<model_id>"` to attribute gc evictions; a
  shared typed `RecoveryHint` enum in `types` would be cleaner. For the
  gc/models/cache pair round.
- **Hit-count-weighted eviction.** v1 is pure LRU+TTL (mirrors `models`).
  Whether frequently-reused prefixes should resist eviction is a future tuning
  question; keep symmetric with `models` until the shared budget/eviction helper
  is extracted (the operator's mandated closing shared-library pass).
- **Where the validity token's `context_config` boundary sits.** If two resident
  slots on the same backend ever run different context sizes, the token must
  distinguish them; current single-config-per-process assumption makes this moot,
  but confirm with engine when multi-config backends land.
