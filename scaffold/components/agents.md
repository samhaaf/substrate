# agents

> **⚠ STUB TRACK — NOT IMPLEMENTING NOW.** This file is DESIGN NOTES +
> ANTICIPATED DATA CONTRACTS only, per the wave-2 batch-8 plan and INTENT
> #40/#49. Nothing here is implementation-ready: no schemas, no code, no
> decomposition into libs. `agents` is the **most speculative stub in the
> wave** — the operator has named the *idea* ("there may be a greater umbrella
> we create at some point") but not its shape. Scope here is deliberately held
> to the operator's own words; nothing is invented beyond them. Thoroughness:
> **requirements-only.**

**Status:** WAVE-2 net-new stub (no repo code; no wave-1 placeholder). **Layer:**
L6 organization plane, ops-facing. **Nesting:** top-level app-crate when it is
eventually built (crate=app, INTENT #22) — a daemon/CLI entry point, never a
library others link (INTENT #29). **Track:** STUB. **Relationship to CCD:**
layered strictly ON TOP; **never absorbs it** (INTENT #49, LOCKED).

## Charter

`agents` is the **future generalization umbrella over agent TYPES** — the
interface that would let Mind OS drive Claude Code and other, custom agents
through one addressable surface. The operator's framing, verbatim (INTENT #40):

> "there may be a greater umbrella we create at some point called CCD — agents
> might be an interface we use to generalize across Claude Code and some other
> custom agents that may or may not use our inference service. Doing LLM
> completions is going to be useful sometimes."

And the hard boundary that pins this crate's relationship to CCD (INTENT #49,
verbatim):

> "CCD needs to be its own thing that can be called by agents… At some point we
> could generalize agents on top of it."

So: **CCD stays a top-level service forever**; `agents` is the layer that CALLS
it, generalizing across agent types. `agents` is the *type-abstraction* layer,
not a second process supervisor. Everything CCD already does well — process
spawn/track/signal/stream/reap, the stable agent-handle namespace, the
usage-limits game, escalation-investigation dispatch — `agents` **reuses**
rather than reimplements.

**Boundary — what `agents` does NOT own.** It does not supervise OS processes,
does not play the Claude-Code usage-limits game, and does not own an agent
ledger — all of that is CCD, reached via `agents-ccd`. It does not run
completions itself (inference). It does not own inter-agent conversation, org
structure, negotiation, or metacognition — that is `org`, which sits *above*
`agents` the same way `org` today sits above CCD. `agents` is the thin
type-generalizing seam between org's org-model and CCD's process-model.

## The agent-type abstraction (operator intent, faithfully)

The umbrella exists to make **agent type** a first-class axis. Three types are
named or implied by the operator's words; only the first exists today:

| Agent type | Completion egress | Managed via | Status |
|------------|-------------------|-------------|--------|
| **claude-code** | Anthropic native (Claude Code picks no models — INTENT #40 LOCKED) | CCD's `agent-management`; usage metered from the CC subprocess stream | exists today (CCD) |
| **custom local agents** | **local inference** (these CAN choose models — INTENT #40) via `v1-completion-api`, metered through CCD's reserved `llm-calls` edge | CCD process-supervision reused; completion routed to inference | future |
| **future third-party** | provider-native or inference, per type | same reuse pattern | speculative |

The distinguishing rule the operator drew (INTENT #40): Claude Code **cannot**
choose models, so it never routes to local inference; **custom agents CAN**
choose models and **MAY** use the inference service. `agents` is the layer where
that per-type routing decision lives — for Claude Code it's a no-op passthrough
to CCD; for a custom agent it's "spawn under CCD's supervisor, but point its
completions at `/v1/`."

## What a generalized agent interface OWNS vs DELEGATES

Held to the operator's words — no invented scope. Best current read:

- **OWNS (the thin part):** the *agent-type registry* (what types exist, how
  each is spawned and where its completions go), a **type-agnostic handle** a
  caller uses without knowing the underlying runtime, and the per-type routing
  decision (native egress vs. local-inference egress).
- **DELEGATES (everything heavy):**
  - **spawn / track / signal / stream / reap** → CCD's `agent-management`
    (already the stable handle namespace; `agents` handles map onto CCD handles).
  - **usage metering + the limits/budget game** → CCD (its ledger, its
    declarative budget/priority strategies).
  - **completions for inference-using types** → inference's `v1-completion-api`,
    with CCD metering via the reserved `llm-calls` path (ccd.md §`llm-calls`
    already reserves this edge "for the FUTURE `agents` umbrella").
  - **converse / negotiate / org-structure** → `org` (above `agents`), NOT here.

Open on purpose (see below): whether **converse** — driving a turn, streaming
output — is an `agents`-owned verb or stays a straight passthrough to CCD's
stream. The operator hasn't said; this stub does not decide it.

## Import surface (anticipated — all mesh-mediated, never Cargo-linked)

Every dependency is a top-level app reached over the wire (WS/CLI) through mesh
(INTENT #29); only `types` + `mesh-client` are compiled-in shared libs.

| Consumes | Via | Why |
|----------|-----|-----|
| `ccd` | `agents-ccd` | process supervision, handle namespace, metering, limits — the substrate `agents` generalizes over (PRIMARY) |
| `inference` | `agents-inference` (rides `v1-completion-api` + CCD's `llm-calls` metering) | completions for custom agent types that choose models |

And `agents` is itself CONSUMED by `org` (`org-agents`, below): org's
per-application owning agents become instances managed through this umbrella.

## Anticipated contracts (wave 2, stub track)

*Names + purpose + rough shape only — schemas deferred until `agents` leaves the
stub track. Recorded now so CCD, inference, and org are shaped for a future
type-generalizing consumer from the start.*

- **`agents-ccd`** (agents → ccd; authored — `agents`' assigned pair, and
  already sketched from CCD's side in ccd.md). *Purpose:* the generalization
  layered ON CCD — `agents` drives CCD's process-supervision + admission +
  metering engine for whatever agent type it is managing. *Rough shape:*
  inbound-to-CCD only (no CCD→agents reverse edge; CCD stays callable and
  independent per INTENT #49). Reuses CCD's `agent-management`
  (spawn/signal/list/stream/reap) verbatim, plus the reserved `llm-calls`
  metering path for non-CC types; `agents` adds only the **agent-type +
  model-choice** framing on top. No new supervisor, no absorbed CCD.

- **`agents-inference`** (agents → inference; NEW anticipated, stub-track).
  *Purpose:* the completion egress for **custom agent types that CAN choose
  models and MAY use local inference** (INTENT #40). *Rough shape:* NOT a new
  inference surface — reuses the standard `v1-completion-api` (`/v1/` REST+WS,
  forwarded through mesh), with usage metered into CCD's ledger via the reserved
  `llm-calls` edge (ccd.md §`llm-calls`). Claude Code agents never touch this
  edge. Open: whether a custom agent's completion goes agents→inference directly
  or is brokered through CCD (which meters it) — see open questions.

- **`org-agents`** (org → agents; NEW anticipated, stub-track both ends).
  *Purpose:* org's **per-application owning agents** (INTENT #88a — every app
  gets an owning agent, with sub/peer agents) become instances managed through
  the `agents` umbrella rather than org reaching CCD directly. *Rough shape:*
  org resolves/spawns/addresses owning + sub/peer agents by type-agnostic handle
  through `agents`; `agents` fans that onto CCD (`agents-ccd`) and inference
  (`agents-inference`). This would re-seat part of org.md's current
  `org-on-ccd` bundle onto `agents` once `agents` exists — flagged, not decided
  (org is designed against CCD today because `agents` isn't built).

## Open questions

1. **Does `agents` even become a crate, or stay a facet of CCD?** The operator
   said the umbrella "**might**" be created and floated calling it CCD itself
   ("a greater umbrella… called CCD"). INTENT #49 then separated them firmly
   (CCD is its own thing agents calls). The stub honors the separation, but
   whether `agents` is a distinct daemon or a thin library facade over CCD is
   genuinely undecided.
2. **Direct vs. CCD-brokered inference for custom agents.** Does a custom
   agent's completion go agents→inference directly (with CCD metering after the
   fact), or does CCD broker every completion so metering is inline? ccd.md's
   `llm-calls` reservation leans CCD-brokered; not settled.
3. **Is `converse`/turn-driving an `agents` verb?** Owns-vs-delegates for the
   streaming turn interaction is unspecified by the operator; could be a pure
   passthrough to CCD's stream or a first-class `agents` surface.
4. **org-on-ccd vs. org-agents once `agents` exists.** org.md wires org→ccd
   directly today. When `agents` lands, how much of org's CCD interaction
   re-seats onto `org-agents`? Both edges are anticipated; the migration is a
   future design pass's call.
5. **What is a "custom agent," concretely?** The operator named the category
   ("some other custom agents") without defining a runtime, plugin model, or
   handle lifecycle for non-CC types. Everything past "reuse CCD's supervisor,
   route completions to inference" is unspecified and intentionally left open.
