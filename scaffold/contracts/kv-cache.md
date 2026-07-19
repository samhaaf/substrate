# Contract: kv-cache

## Parties
`engine` `<->` `cache` — both internal libs of `inference`, in-process
(compiled-in Rust API, not a mesh edge; documented per the inference-internal
contract convention).

*(Upgraded at wave-2 harmonization. The rounds-era stub ("Schema deferred")
was never upgraded by the contract round — a coverage gap, found last. Content
below is the already-designed position from `cache.md` (implementation-ready;
authoritative for keying/invalidation) and `engine.md` (save-window
triggering). No dispute existed between the halves.)*

## Purpose
KV/prefix cache slot save/restore so a matching prefix skips prefill: engine
saves a slot's KV state after prefill (and on the L3 restart save-window —
engine.md's wave-2 addition), and restores on a matching prefix.

## Schema (summary — authoritative detail in cache.md concerns 1–3)
- **Lookup key: `(model_id, validity_token, prompt_hash)`.** The
  `validity_token` = short hash of `(model_fingerprint, backend_build_id,
  context_config)`, supplied by engine (which alone knows what is resident) at
  both save and lookup. An incompatible blob MISSES (normal prefill) instead
  of being restored — correctness independent of eviction ordering.
- **`prompt_hash`:** exact full-tokenized-prefix match in v1 (no
  longest-common-prefix; open question recorded in cache.md).
- **Save is two-phase:** `ReserveSave` (cache checks budget via gc; may reply
  `Skip` on `DiskBudgetExceeded` — non-fatal, engine just doesn't save) →
  engine writes the blob → `ConfirmSave` (cache registers via gc's atomic
  `register_and_lock` so a fresh blob can't be swept in the register→sweep
  window).
- **Entry id:** a stable content hash (sha256/blake3) covering the token —
  supersedes the wave-1 `DefaultHasher` 64-bit id.

## Error cases
`Skip` (budget), miss (any key component differs, blob evicted, or no free
slot on the resident backend) — all degrade to normal prefill; no error is
fatal to a completion.

## Version sensitivity
LOW — in-process, node-local; the persisted bits are the blob files
(self-invalidating via `validity_token`) and the `kv_cache_entries` rows
(store-access; the additive `validity_token TEXT` column is cache's flagged
ask to store).

## Reconciliation notes
- Single-proposer content: cache.md authored keying/invalidation; engine.md
  added save-window triggering (L3 ladder). Compatible; merged here.
- Retention asymmetry recorded: model weights outrank KV blobs (separate
  gc-managed dirs, separate budgets; `kvcache/` yields first).
- No cache↔mesh edge — intentional (cache.md Controversial decisions).
