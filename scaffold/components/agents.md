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
library others link (INTENT #29). **Track:** STUB. **Relationship to cc:**
layered strictly ON TOP; **never absorbs it** (INTENT #49, LOCKED).

> **Re-spoken round (2026-07-21/22, INTENT #167): `agents` STAYS as a
> crate — a conceptual placeholder for CUSTOM agent harnesses** (OpenRouter
> + inference data contracts; the custom-agent row of the table below).
> And a hard scope line, so no future pass conflates them: **`agents` is
> explicitly NOT the thread-from-seed mechanism.** Agents-as-in-this-crate
> ≠ threads spinning up from a topological node's seed — that is the
> **topological-node (bishop/keeper) concept** (see overview.md's
> vocabulary block and org.md). This crate is only the harness/data-contract
> layer for custom (non-Claude-Code) agent runtimes; open question 1 below
> (crate vs facet-of-cc) is resolved to the extent that the crate stays,
> as a conceptual placeholder.

## Charter

`agents` is the **future generalization umbrella over agent TYPES** — the
interface that would let Mind OS drive Claude Code and other, custom agents
through one addressable surface. The operator's framing, verbatim (INTENT #40):

> "there may be a greater umbrella we create at some point called cc — agents
> might be an interface we use to generalize across Claude Code and some other
> custom agents that may or may not use our inference service. Doing LLM
> completions is going to be useful sometimes."

And the hard boundary that pins this crate's relationship to cc (INTENT #49,
verbatim):

> "cc needs to be its own thing that can be called by agents… At some point we
> could generalize agents on top of it."

So: **cc stays a top-level service forever**; `agents` is the layer that CALLS
it, generalizing across agent types. `agents` is the *type-abstraction* layer,
not a second process supervisor. Everything cc already does well — process
spawn/track/signal/stream/reap, the stable agent-handle namespace, the
usage-limits game, escalation-investigation dispatch — `agents` **reuses**
rather than reimplements.

**Boundary — what `agents` does NOT own.** It does not supervise OS processes,
does not play the Claude-Code usage-limits game, and does not own an agent
ledger — all of that is cc, reached via `agents-cc`. It does not run
completions itself (inference). It does not own inter-agent conversation, org
structure, negotiation, or metacognition — that is `org`, which sits *above*
`agents` the same way `org` today sits above cc. `agents` is the thin
type-generalizing seam between org's org-model and cc's process-model.

## The agent-type abstraction (operator intent, faithfully)

The umbrella exists to make **agent type** a first-class axis. Three types are
named or implied by the operator's words; only the first exists today:

| Agent type | Completion egress | Managed via | Status |
|------------|-------------------|-------------|--------|
| **claude-code** | Anthropic native (Claude Code picks no models — INTENT #40 LOCKED) | cc's `agent-management`; usage metered from the CC subprocess stream | exists today (cc) |
| **custom local agents** | **DIRECT — OpenRouter or local inference (`v1-completion-api`), never through cc** (INTENT #137, resolving the earlier cc-brokered lean) | NOT cc — cc is for the full Claude Code agent shape only | future |
| **future third-party** | provider-native or inference, per type — direct, per INTENT #137 | same rule | speculative |

The distinguishing rule the operator drew (INTENT #40): Claude Code **cannot**
choose models, so it never routes to local inference; **custom agents CAN**
choose models and **MAY** use the inference service. `agents` is the layer where
that per-type routing decision lives — for Claude Code it's a no-op passthrough
to cc; for a custom agent the completion egress is its own.

**Friction-round 3 sharpening (2026-07-20, INTENT #137): custom agents do
NOT route through cc.** Verbatim: "Custom agents might call OpenRouter,
might call the inference tool — they're not necessarily going to call cc.
Cloud Code is a full agent expecting a certain structure (plugins etc.);
our custom agents might evolve to make more use of the concepts within our
[system]." cc is the runtime for the **full Claude Code agent shape**
specifically — plugins, its process/output structure, the usage-limits
game. Custom agent types call **OpenRouter or inference directly**; the
earlier lean toward cc-brokered completions (and toward reusing cc's
process supervisor as the default for non-CC types) is superseded. What, if
anything, supervises/meters custom agents is an open design question for
when `agents` leaves the stub track — it is NOT answered "cc" by default.

## What a generalized agent interface OWNS vs DELEGATES

Held to the operator's words — no invented scope. Best current read:

- **OWNS (the thin part):** the *agent-type registry* (what types exist, how
  each is spawned and where its completions go), a **type-agnostic handle** a
  caller uses without knowing the underlying runtime, and the per-type routing
  decision (native egress vs. local-inference egress).
- **DELEGATES (everything heavy):**
  - **Claude-Code-type agents** — spawn / track / signal / stream / reap →
    cc's `agent-management` (the stable handle namespace; `agents` handles
    map onto cc handles), usage metering + the limits/budget game → cc.
    This delegation is for the **Claude Code agent shape only** (INTENT
    #137).
  - **completions for custom agent types** → **OpenRouter or inference
    (`v1-completion-api`) DIRECTLY — not through cc** (INTENT #137,
    superseding the earlier reading of cc.md's `llm-calls` reservation as a
    broker path; `llm-calls` remains at most an optional metering surface).
  - **converse / negotiate / org-structure** → the emergent coordinator
    layer (formerly `org` — dissolved, INTENT #132; see org.md), NOT here.

Open on purpose (see below): whether **converse** — driving a turn, streaming
output — is an `agents`-owned verb or stays a straight passthrough to cc's
stream. The operator hasn't said; this stub does not decide it.

## Import surface (anticipated — all mesh-mediated, never Cargo-linked)

Every dependency is a top-level app reached over the wire (WS/CLI) through mesh
(INTENT #29); only `types` + `mesh-client` are compiled-in shared libs.

| Consumes | Via | Why |
|----------|-----|-----|
| `cc` | `agents-cc` | the Claude-Code agent type ONLY: process supervision, handle namespace, metering, limits (INTENT #137) |
| `inference` | `agents-inference` (rides `v1-completion-api`; direct, not cc-brokered — INTENT #137) | completions for custom agent types that choose models |
| OpenRouter | via `openrouter-mgmt` / provider-native | direct completion egress for custom agent types (INTENT #137) |

And `agents` is itself CONSUMED from above — formerly by `org`
(`org-agents`, below); with the org crate dissolved (INTENT #132), the
consumer is the emergent coordinator/owner layer (see org.md).

## Anticipated contracts (wave 2, stub track)

*Names + purpose + rough shape only — schemas deferred until `agents` leaves the
stub track. Recorded now so cc, inference, and org are shaped for a future
type-generalizing consumer from the start.*

- **`agents-cc`** (agents → cc; authored — `agents`' assigned pair, and
  already sketched from cc's side in cc.md). *Purpose:* driving the
  **Claude-Code agent type** through cc — its process-supervision +
  admission + metering engine. *Rough shape:* inbound-to-cc only (no
  cc→agents reverse edge; cc stays callable and independent per INTENT
  #49). Reuses cc's `agent-management` (spawn/signal/list/stream/reap)
  verbatim; `agents` adds only the **agent-type + model-choice** framing on
  top. No new supervisor, no absorbed cc. **Scope narrowed (friction-round
  3, INTENT #137): this edge carries the full-Claude-Code agent shape ONLY
  — custom agent types do not ride it.**

- **`agents-inference`** (agents → inference; NEW anticipated, stub-track).
  *Purpose:* the completion egress for **custom agent types that CAN choose
  models and MAY use local inference** (INTENT #40). *Rough shape:* NOT a new
  inference surface — reuses the standard `v1-completion-api` (`/v1/` REST+WS,
  forwarded through mesh). **RESOLVED (friction-round 3, INTENT #137): the
  call goes agents→inference (or agents→OpenRouter) DIRECTLY, never brokered
  through cc.** cc.md's `llm-calls` reservation survives only as an optional
  metering surface, not a broker. Claude Code agents never touch this edge.

- **`org-agents`** (coordinators → agents; NEW anticipated, stub-track both
  ends; the `org` crate is DISSOLVED per INTENT #132 — the upstream party is
  now the emergent coordinator/owner layer). *Purpose:* per-thing
  owners/coordinators (INTENT #88a lineage, now the coordinator concept —
  see org.md) manage their agent instances through the `agents` umbrella
  rather than reaching cc directly. *Rough shape:* resolve/spawn/address
  agents by type-agnostic handle through `agents`; `agents` fans Claude-Code
  types onto cc (`agents-cc`) and custom types onto direct inference/
  OpenRouter egress (`agents-inference`, INTENT #137). Whether part of the
  `org-on-cc` bundle re-seats here once `agents` exists stays flagged, not
  decided.

## Open questions

1. **Does `agents` even become a crate, or stay a facet of cc?** The operator
   said the umbrella "**might**" be created and floated calling it cc itself
   ("a greater umbrella… called cc"). INTENT #49 then separated them firmly
   (cc is its own thing agents calls). The stub honors the separation, but
   whether `agents` is a distinct daemon or a thin library facade over cc is
   genuinely undecided.
2. **Direct vs. cc-brokered inference for custom agents — RESOLVED
   (friction-round 3, INTENT #137): DIRECT.** "Custom agents might call
   OpenRouter, might call the inference tool — they're not necessarily
   going to call cc." cc is for the full Claude Code agent shape; the
   `llm-calls` cc-brokered lean is superseded. (What meters/supervises
   custom agents is folded into open question 5.)
3. **Is `converse`/turn-driving an `agents` verb?** Owns-vs-delegates for the
   streaming turn interaction is unspecified by the operator; could be a pure
   passthrough to cc's stream or a first-class `agents` surface.
4. **org-on-cc vs. org-agents once `agents` exists.** org.md wires org→cc
   directly today. When `agents` lands, how much of org's cc interaction
   re-seats onto `org-agents`? Both edges are anticipated; the migration is a
   future design pass's call.
5. **What is a "custom agent," concretely?** The operator named the category
   ("some other custom agents") without defining a runtime, plugin model, or
   handle lifecycle for non-CC types — and noted they "might evolve to make
   more use of the concepts within our [system]" rather than the Claude Code
   plugin structure. With the cc-brokered lean gone (INTENT #137), the
   supervision/metering story for custom agents is fully open — it is NOT
   "reuse cc's supervisor" by default. Intentionally left open.
