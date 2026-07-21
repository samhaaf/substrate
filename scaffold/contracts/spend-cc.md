# Contract: spend-cc

> *Renamed from `spend-ccd` at friction-round 3 (INTENT #131, ccd → cc).*

## Parties
- `spend` (L6 stub) `->` `cc` (L5, ledger author) — **pull-shaped, one-way.**

*(Stub-track — spend's flagship edge (wave2-plan §3c). cc authored the ledger
side and reserved this pair. Content deferred until `spend` leaves the stub
track — INTENT #41/#68.)*

## Purpose
Pull-shaped queries of cc's usage database so `spend` can compute per-project
cost. This is the async reporting read that mirrors cc's own synchronous
admission read of the same ledger. **Sources never push** — `spend` polls; there
is no reverse cc→spend edge and no hot-path coupling.

## Rough shape
A **read-only** query surface over `usage_records ⨝ agent_runs`, grouped and
rolled up:

```rust
// spend -> cc
struct UsageQuery {
    group_by: Vec<UsageDim>,            // ProjectId | EnvironmentId | CallerSlug | Model | Window
    window: TimeWindow,                 // {from, to} or a rolling bucket (session/weekly)
    filter: Option<UsageFilter>,        // optional project/caller/model narrowing
}
// cc -> spend
struct UsageRollup {
    rows: Vec<UsageRow>,                // one per group key
}
struct UsageRow {
    key: UsageKey,                      // the resolved group tuple
    input_tokens: u64, output_tokens: u64,
    provenance_refs: Vec<Uuid>,         // so the underlying usage is re-derivable (INTENT #68 provenance)
}
```

`spend` computes cost from token counts + model pricing; **cc returns tokens +
provenance, never a cost figure** (spend owns pricing). Every `usage_records` row
joins to an `agent_runs` row (provenance completeness is a cc conformance test),
so a rollup is always re-derivable.

## Open questions
- Whether project/environment ids are resolved to names here or via a separate
  `projects` read (see `cc-projects` / the proposed `spend-projects`).
- Pagination / max-window bounds on a large ledger.
- Exact `TimeWindow` bucket vocabulary shared with cc's admission windows
  (session/weekly/per-model) so spend and admission speak the same buckets.
- Whether the query rides `db-control-plane` (query cc's DB directly) or a
  cc-authored read endpoint — leaning a cc-authored endpoint so the ledger
  schema stays private (INTENT #68: cc owns its database).
