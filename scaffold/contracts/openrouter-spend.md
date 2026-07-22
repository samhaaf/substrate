# Contract: openrouter-spend

> **⚠ ABSORBED (re-spoken round, 2026-07-21/22): `openrouter-mgmt` folded
> INTO `spend`** (see spend.md's re-spoken update and openrouter-mgmt.md's
> tombstone). Both parties below are now the same crate — this edge
> collapses to **spend's internal OpenRouter source adapter**. Retained as
> the record of that adapter's shape. Note also (INTENT #166 Q10): keys are
> attachable to landscape topological nodes or node+environment combos, so
> per-key usage maps to per-region spend.

## Parties
- `spend` (L6 stub) `->` `openrouter-mgmt` (L6 stub) — **pull-shaped;
  `openrouter-mgmt` is the SOURCE.**

*(Stub-track — the wave2-plan §3c inventory name; this cluster's brief calls the
same edge "spend-openrouter". Content deferred until both ends leave the stub
track — INTENT #41.)*

## Purpose
Let `spend` fold OpenRouter per-key budget/usage into its per-project cost
tracking. `openrouter-mgmt` owns per-key budget **enforcement**; `spend`
aggregates and reports and **never sets a budget** — same PULL discipline `spend`
uses for cc's usage DB (`spend-cc`). No push from `openrouter-mgmt`.

## Rough shape
A read-only query surface exposing, per runtime key, its budget-vs-actual:

```rust
// spend -> openrouter-mgmt
struct KeyUsageQuery { keys: Option<Vec<KeyId>>, window: TimeWindow } // None = all keys
// openrouter-mgmt -> spend
struct KeyUsageReport { rows: Vec<KeyUsage> }
struct KeyUsage {
    key_id: KeyId,
    label: String, scope: SecretScope,   // the key's label / scope for mapping
    budget: Option<u64>, usage: u64, remaining: Option<i64>,  // currency-minor or provider units
}
```

`spend` polls this (never a push); `openrouter-mgmt` reports actual OpenRouter
spend it has pulled from the provider. The mapping of keys → projects/finances is
`spend`'s to compute and is deferred (see open questions).

## Open questions
- Key → project/environment mapping: does `openrouter-mgmt` carry a
  project label per key, or does `spend` map externally? (Deferred; likely a
  label on the key echoing `cc-projects`' scheme.)
- Units/currency normalization between OpenRouter's figures and `spend`'s
  per-project cost model.
- Poll cadence + caching so `spend` doesn't hammer OpenRouter's provider API
  (the actual-usage pull is itself an external HTTPS call `openrouter-mgmt`
  makes, not a mesh call).
