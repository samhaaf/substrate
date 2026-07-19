# Contract: spend-ccd

## Parties
- `spend` (L6 stub) `->` `ccd` (L5, ledger author) — **pull-shaped, one-way.**

*(Stub-track — spend's flagship edge (wave2-plan §3c). CCD authored the ledger
side and reserved this pair. Content deferred until `spend` leaves the stub
track — INTENT #41/#68.)*

## Purpose
Pull-shaped queries of CCD's usage database so `spend` can compute per-project
cost. This is the async reporting read that mirrors CCD's own synchronous
admission read of the same ledger. **Sources never push** — `spend` polls; there
is no reverse ccd→spend edge and no hot-path coupling.

## Rough shape
A **read-only** query surface over `usage_records ⨝ agent_runs`, grouped and
rolled up:

```rust
// spend -> ccd
struct UsageQuery {
    group_by: Vec<UsageDim>,            // ProjectId | EnvironmentId | CallerSlug | Model | Window
    window: TimeWindow,                 // {from, to} or a rolling bucket (session/weekly)
    filter: Option<UsageFilter>,        // optional project/caller/model narrowing
}
// ccd -> spend
struct UsageRollup {
    rows: Vec<UsageRow>,                // one per group key
}
struct UsageRow {
    key: UsageKey,                      // the resolved group tuple
    input_tokens: u64, output_tokens: u64,
    provenance_refs: Vec<Uuid>,         // so the underlying usage is re-derivable (INTENT #68 provenance)
}
```

`spend` computes cost from token counts + model pricing; **CCD returns tokens +
provenance, never a cost figure** (spend owns pricing). Every `usage_records` row
joins to an `agent_runs` row (provenance completeness is a CCD conformance test),
so a rollup is always re-derivable.

## Open questions
- Whether project/environment ids are resolved to names here or via a separate
  `projects` read (see `ccd-projects` / the proposed `spend-projects`).
- Pagination / max-window bounds on a large ledger.
- Exact `TimeWindow` bucket vocabulary shared with CCD's admission windows
  (session/weekly/per-model) so spend and admission speak the same buckets.
- Whether the query rides `db-control-plane` (query CCD's DB directly) or a
  CCD-authored read endpoint — leaning a CCD-authored endpoint so the ledger
  schema stays private (INTENT #68: CCD owns its database).
