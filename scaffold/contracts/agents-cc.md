# Contract: agents-cc

> *Renamed from `agents-ccd` at friction-round 3 (INTENT #131, ccd → cc).*

## Parties
- `agents` (L6 stub) `->` `cc` (L5) — **inbound-to-cc only.**

*(Stub-track — `agents`' assigned pair (wave2-plan §3c); already sketched from
cc's side in cc.md. Content deferred until `agents` leaves the stub track —
INTENT #49.)*

## Purpose
The future generalization layered ON TOP of cc — **for the full Claude Code
agent shape ONLY (friction-round 3, INTENT #137)**. `agents` drives cc's
process-supervision + admission + metering engine for Claude-Code-type
agents. It **never absorbs cc** — cc stays top-level, callable, and
independent forever (INTENT #49).

## Rough shape
- Inbound-to-cc only; no cc→agents reverse edge.
- Reuses cc's `agent-management` (spawn/signal/list/stream/reap) **verbatim** —
  no new supervisor.
- Adds, above that surface, only the **agent-type** framing.
- **Custom (non-Claude-Code) agent types do NOT ride this edge (INTENT
  #137, superseding the earlier brokered-through-cc lean):** "Custom agents
  might call OpenRouter, might call the inference tool — they're not
  necessarily going to call cc. Cloud Code is a full agent expecting a
  certain structure (plugins etc.)." Custom types call OpenRouter or
  inference (`v1-completion-api`) directly; cc's reserved `llm-calls` path
  survives only as an *optional* metering surface, never a mandatory
  broker.
- Claude Code agents never touch the model-choice/inference path (INTENT #40).

## Open questions
- ~~Direct vs. brokered-through-cc completions for custom agents~~ —
  **RESOLVED (INTENT #137): direct**; and their supervision/metering story
  is fully open (NOT "cc" by default). See agents.md open question 5.
- How `agent-type` is represented (open string vs registered type descriptor).
- Whether coordinator-owned agents re-seat onto this edge (`org-agents`,
  now coordinator-shaped — the `org` crate is dissolved, INTENT #132) once
  `agents` exists, moving part of `org-on-cc` here.
