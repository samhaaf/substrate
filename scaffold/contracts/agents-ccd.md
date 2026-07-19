# Contract: agents-ccd

## Parties
- `agents` (L6 stub) `->` `ccd` (L5) — **inbound-to-CCD only.**

*(Stub-track — `agents`' assigned pair (wave2-plan §3c); already sketched from
CCD's side in ccd.md. Content deferred until `agents` leaves the stub track —
INTENT #49.)*

## Purpose
The future generalization layered ON TOP of CCD for non-Claude-Code agent types
(which CAN choose models and MAY use local inference). `agents` drives CCD's
process-supervision + admission + metering engine for whatever agent type it is
managing. It **never absorbs CCD** — CCD stays top-level, callable, and
independent forever (INTENT #49).

## Rough shape
- Inbound-to-CCD only; no CCD→agents reverse edge.
- Reuses CCD's `agent-management` (spawn/signal/list/stream/reap) **verbatim** —
  no new supervisor.
- Adds, above that surface, only the **agent-type + model-choice** framing:
  for non-CC types, completions route to local inference via the reserved
  `llm-calls` metering path, and CCD meters that usage into the same ledger that
  feeds `spend-ccd`.
- Claude Code agents never touch the model-choice/inference path (INTENT #40).

## Open questions
- Whether a custom agent's completion goes `agents -> inference` directly (via
  `v1-completion-api`) or is brokered through CCD so CCD meters it — the
  `agents-inference` pair (not in this cluster's inventory) carries that; leaning
  brokered-through-CCD so metering stays single-authority.
- How `agent-type` is represented (open string vs registered type descriptor).
- Whether `org`'s per-app owning agents re-seat onto this edge (`org-agents`)
  once `agents` exists, moving part of `org-on-ccd` here.
