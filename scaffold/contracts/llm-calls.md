# Contract: llm-calls

## Parties
ccd / future `agents`  ->  inference (api, via `v1-completion-api`)

- **inference (api)** is the terminus — the same `/v1/` completion surface as
  `v1-completion-api`. **No second surface is introduced.**
- **ccd** is the caller **and** the meter — non-Claude-Code agents make
  completions through `/v1/`, and CCD records the usage into its own ledger.

## Purpose
How non-Claude-Code agents get model calls, and how that usage is metered.
**RESERVED, thin this wave** (INTENT #40 LOCKED). Two clarifications this edge
must carry so the harmonizer never mis-wires it:

1. **This is NOT the Claude Code path.** Claude Code usage is metered from the CC
   subprocess output stream (`agent-management`'s `TurnUsage`), NOT via inference
   — **inference sees no Claude Code traffic** (Claude Code cannot choose its
   model, so it is never routed to local inference).
2. The edge is **reserved for the future `agents` umbrella's non-CC agent types**,
   which CAN choose models and MAY route completions to local inference. Those
   agents reuse `v1-completion-api` (the fleet-facing surface); CCD meters that
   usage into the same `usage_records` ledger `spend` reads (pull-shaped).

Kept cheap to fan out (org's agents will call at higher volume) — same reason
`v1-completion-api` stays the single completions entry point.

## Schema

**Transport:** verbatim `v1-completion-api` (`POST /v1/completions`, the token
stream, `GET /v1/completions/:id/result`). Nothing added on the inference side.

**Metering-shaped view (CCD-internal, from the completion response usage block):**

```rust
// types::ccd — CCD records one row per metered completion (rough sketch)
struct UsageRecord {
    thread: Option<ThreadId>,          // links to a CCD thread (and optionally project/env)
    model_id: ModelId,                 // an inference ModelId (NOT a Claude model)
    caller: CallerContext,             // which agent/priority requested it
    prompt_tokens: u32,
    completion_tokens: u32,
    surface: String,                   // "inference" (vs "claude-code" for the CC path)
    recorded_at: DateTime<Utc>,
}
```

The token counts come from `CompletionMetrics` (`prompt_tokens` /
`completion_tokens`) in the `/v1/completions/:id/result` (or the terminal
`StreamEvent`). No inference-side change; CCD just reads the response it already
gets.

## Error cases
- Reuses `v1-completion-api`'s error surface entirely (404 / 409 / 422 / 500
  terminus, 502/503 router). CCD adds no inference-facing errors.
- `CcdError::Inference(String)` — stringified failure on the future agents path
  only (never a Claude Code failure; never becomes an inference type).
- Warm-start: no ledger data yet to attribute a call → CCD records the row anyway
  (usage is observed, not gated here).

## Version sensitivity
- **LOW (reserved).** The edge is a thin alias; it inherits `v1-completion-api`'s
  version discipline (additive-only bodies, `#[serde(other)]` on `StreamEvent`).
- **Additive-safe:** new `UsageRecord` fields (CCD-internal), new metered surfaces.
- **Breaking:** authoring a Claude-Code-specific completion surface — **forbidden;
  there is none** (conformance).

## Reconciliation notes

1. **Shape vs wave-1 — CONFIRMED thin/reserved (ccd wins; the only proposer).**
   Wave-1's stub framed `llm-calls` as "CCD/agents make LLM calls through the
   local inference node's `/v1/` API." ccd.md refines this: keep it a **thin
   alias of `v1-completion-api` plus a metering note**, and do **not** author a
   separate surface. inference/api proposes no `llm-calls`-specific content (its
   `/v1` surface is the whole contract), so there is no competing position to
   reconcile — this file records ccd's reservation + metering shape and defers
   full content to when `agents` leaves the stub track.

2. **Claude Code exclusion (LOCKED, INTENT #40).** Recorded here so a later
   harmonizer does not accidentally route Claude Code through inference: the CC
   metering signal is `agent-management`'s `TurnUsage`, a separate edge.

## Example data

**World:** nodes `macbook` and `pi`; model `qwen3-4b`; project `demo`. A future
(non-Claude-Code) agent under the `agents` umbrella, running in a CCD thread on
`macbook`, makes a completion via `/v1/` — the router forwards it to `pi` exactly
as in `v1-completion-api` (completion `7f3c1a90-2b4e-4c1a-9e21-0a1b2c3d4e5f`). CCD
meters the result:

```jsonc
// CCD usage_records row derived from the /v1/completions/:id/result usage block
{
  "thread": "thr-demo-0001",
  "model_id": "qwen3-4b",
  "caller": { "agent": "demo-writer", "priority": 10 },
  "prompt_tokens": 14,
  "completion_tokens": 22,
  "surface": "inference",
  "recorded_at": "2026-07-19T17:00:01Z"
}
// `spend` later PULLS this row (spend-ccd) for the demo project's cost rollup.
```
