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

> **Wave-3 refresh (2026-07-22, unit `agents-cc-refresh`).** Vocabulary lock
> lands (ledger #172): the topological node is **keeper**, its persistent
> shared knowledge is the **bundle**, per-thread ephemeral memory is the
> **workspace**, the daemon wrapper is **chassis**. Where this file previously
> said "org," "coordinator/owner," or "seed," read **keeper**/**bundle**. This
> pass adds three things INTENT #167/#137 imply but the wave-2 stub did not
> yet spell out, all still requirements-only (no schemas, `agents` stays
> stub-track):
> 1. **The runtime-choice seam** — a keeper's thread does not have to run on
>    cc. INTENT #146 already frames initialization as bundle-assembled and
>    runtime-agnostic ("possibly through a rollup producing the initial
>    prompt"); this pass names the seam through which a keeper picks the
>    custom-agent runtime instead, and confirms `agents` (not `org`, which is
>    dissolved) is where that pick is exercised. See "Runtime choice" below.
> 2. **The custom-agent egress seams through `secrets` and `spend`** — a
>    custom agent calling OpenRouter directly still needs a credential
>    (`secrets`, use-without-seeing) and still wants its usage attributed to a
>    keeper (`spend`'s `SpendScope`, ledger #166 Q10); wave 2 named the
>    OpenRouter/inference egress but not these two supporting edges.
> 3. **Modality-tagged completions** — `inference` (batch 5, INTENT #166
>    Q12) now tags `/v1/completions` requests with a `Modality` (T2T today,
>    S2T/T2S next); `agents-inference` inherits the tag for free (same
>    surface), noted below so a custom agent choosing a non-text modality is
>    visibly in scope, not an oversight.
>
> None of this promotes `agents` off the stub track or invents a runtime for
> "custom agent" (open question 5, unchanged) — it only shapes the contracts
> a stub-track crate needs so `cc`, `inference`, `secrets`, and `spend` stay
> correctly shaped for a future consumer, per the L6 rule (INTENT #173c).

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
  - **converse / negotiate / org-structure** → the emergent **keeper**
    layer (formerly `org` — dissolved, INTENT #132; ledger #172 locks the
    node's name as **keeper**; see org.md's tombstone and the anticipated
    `components/keeper.md`), NOT here.

Open on purpose (see below): whether **converse** — driving a turn, streaming
output — is an `agents`-owned verb or stays a straight passthrough to cc's
stream. The operator hasn't said; this stub does not decide it.

## Import surface (anticipated — all mesh-mediated, never Cargo-linked)

Every dependency is a top-level app reached over the wire (WS/CLI) through mesh
(INTENT #29); only `types` + `chassis` are compiled-in shared libs (wave-3:
`mesh-client` retires into `chassis`, ledger D1 — `agents` links `chassis`
like every other future crate, not the old `mesh-client`).

| Consumes | Via | Why |
|----------|-----|-----|
| `cc` | `agents-cc` | the Claude-Code agent type ONLY: process supervision, handle namespace, metering, limits (INTENT #137) |
| `inference` | `agents-inference` (rides `v1-completion-api`; direct, not cc-brokered — INTENT #137; wave-3: the request may carry a `Modality` tag, batch-5 inference.md concern 7 — T2T today, S2T/T2S next) | completions for custom agent types that choose models (and, per batch 5, choose a modality) |
| OpenRouter | direct provider HTTPS, via `secrets` for the credential (`agents-secrets`, NEW wave-3) | direct completion egress for custom agent types (INTENT #137); wave-3 note: `openrouter-mgmt` no longer exists as a separate crate — its key-lifecycle/budget administration is absorbed into `spend` (ledger #166 Q10, spend.md consolidation); `agents` only ever *uses* a runtime key, never mints/rotates one |
| `secrets` | `agents-secrets` (NEW wave-3) | resolve-and-use the OpenRouter (or other provider) runtime key as a `SecretRef`, use-without-seeing (INTENT #94 `llm_safe`) — the raw key never enters a prompt, log, or rollup fragment |
| `spend` | `agents-spend` (NEW wave-3, thin) | tag a custom agent's completion with the `SpendScope` (`keeper_id` [+ `environment_id`]) of the keeper it is running for, so `spend`'s pull-shaped OpenRouter/inference accounting attributes correctly (spend.md's attribution model) — `agents` never computes or enforces cost, only carries the scope |

And `agents` is itself CONSUMED from above — formerly by `org`
(`org-agents`, below); with the org crate dissolved (INTENT #132) and the
node concept renamed **keeper** (ledger #172), the consumer is the keeper
runtime (see the anticipated `components/keeper.md`, not yet authored at
this writing — org.md's tombstone is the interim pointer). Wave-3 renames
this stub's anticipated pair `org-agents` → **`keeper-agents`**, content
unchanged in shape (see "Runtime choice" and "Anticipated contracts (wave 3,
L6)" below).

## Runtime choice: a keeper thread MAY run on a custom agent instead of cc (wave 3, INTENT #146)

Held to the operator's own framing, not invented: a keeper thread is
**initialized from a bundle** — "the coordinator-initialization node… every
new [keeper] thread starts from that node, possibly through a rollup
producing the initial prompt" (INTENT #146) — and bundle assembly (rollup
plane 1, prompt/plugin text; rollup plane 2 when the bundle's structure is
graph-sourced, rollup.md concern 11) does not know or care which runtime
consumes the result. That is the seam this pass names:

- **The bundle → initial-prompt production is runtime-agnostic.** rollup
  resolves a keeper's bundle (role prompt + plugins) into a materialized
  artifact (rollup-cc's `AssembleResult` shape today; the same shape for any
  runtime) regardless of whether the consuming runtime is Claude Code (cc) or
  a custom agent (`agents`). "When running on Claude Code, the shape is
  role-prompt-plus-plugins" (INTENT #148 beat 3) reads as *one instance* of a
  runtime-shaped projection, not the only possible one.
- **The choice of runtime is the keeper's, exercised through `agents`.** A
  keeper spawning a thread picks a runtime the same way `agents`' agent-type
  table already frames it (Charter, above): Claude-Code-shaped → `agents-cc`
  → cc; custom-agent-shaped → `agents` drives it directly against
  `inference`/OpenRouter (this file's charter, unchanged). What's new this
  wave is naming that the **keeper**, not just an abstract "caller," is the
  party making that pick, and that it makes it **per thread**, not per
  keeper — the same keeper could run one thread on cc and another on a
  custom agent.
- **`agents` does not gain a bundle-consumption mechanism of its own.**
  Materializing a bundle into whatever shape a custom runtime needs (a
  system prompt string, a tool-call schema, whatever the runtime expects) is
  still rollup's job (plane 1/2, unchanged); `agents` only receives the
  materialized artifact + a runtime handle, exactly as cc receives
  `AssembleResult` today via `rollup-cc`. If/when a custom-agent runtime
  needs a *different* materialized shape than cc's Claude-Code plugin
  directory, that is a **new rollup output kind** (an `OutputSink` variant
  or an `agents`-shaped `MaterializeResult` projection) — named here as an
  anticipated need, not designed (rollup owns `rollup.md`, not this file).

**What this resolves vs. what stays open.** It resolves the earlier fuzziness
about whether "custom agent" and "keeper-initialized thread" were the same
concept (they are not: a keeper thread is *initialized from a bundle*
regardless of runtime; a "custom agent" is a *runtime choice* for that
thread) — and it confirms `agents`, not a new mechanism, is where the
non-cc runtime choice is exercised. It does **not** design: which bundle
elements a custom runtime actually needs (a system-prompt string? a
tool-schema?); how a custom-runtime thread reports back to its keeper for
curation (OQ-5, PARKED); or how the keeper decides cc-vs-custom (a keeper
config value? per-task-type policy? unnamed — the operator has not said).

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
  > **Wave-3 rename (ledger #172): `org-agents` → `keeper-agents`.** Content
  > unchanged; "coordinators/owners" is now the **keeper**. See "Anticipated
  > contracts (wave 3, L6)" below for the runtime-choice framing this pair
  > picked up this wave.

## Anticipated contracts (wave 3, L6)

*Wave-3 additions/updates only — the wave-2 contracts above stand as authored.
Same discipline: names + purpose + rough shape, schemas deferred until `agents`
leaves the stub track (INTENT #173c).*

- **`keeper-agents`** (keeper → agents; renamed from `org-agents`, content
  extended). *Purpose:* the keeper runtime's **runtime-choice seam** — when a
  keeper spawns a thread that should run on a custom-agent runtime instead of
  cc, it reaches `agents` through this edge (see "Runtime choice" above).
  *Rough shape:* unchanged from `org-agents`'s resolve/spawn/address-by-handle
  shape, PLUS: the spawn request carries the keeper's already-materialized
  bundle artifact (the rollup output — same kind of thing cc receives via
  `rollup-cc`'s `AssembleResult`, whatever shape a custom runtime needs) and
  the keeper's id (so `agents-spend`, below, can scope the resulting
  completions). `agents` does not read the bundle's *content* differently
  than any other caller — it is a materialized artifact + a handle, not a
  KG read. **Owned by whichever unit designs `components/keeper.md`** (not
  yet authored at this writing); this file proposes the `agents`-side shape
  only.

- **`agents-secrets`** (agents → secrets; NEW). *Purpose:* resolve a custom
  agent's provider credential (OpenRouter runtime key today; any future
  provider key) as a `SecretRef` and use it **without ever seeing the raw
  value** (INTENT #94 `llm_safe` — the same discipline `spend`'s own
  OpenRouter adapter already follows for minting/rotating the key, spend.md
  §2). *Rough shape:* `agents` never mints or rotates a key (that stays
  `spend`'s absorbed OpenRouter adapter, spend.md ledger #166 Q10); it only
  **resolves a `SecretRef` scoped to the calling keeper** (secrets already
  keeps every version, INTENT #156) and invokes it through the standard
  resolve-into-sink call (`secrets.use(ref, sink)`) so the raw key lands only
  in the outbound HTTPS call, never in a prompt, log, or rollup fragment.
  Which `SecretRef` to resolve for a given keeper is a lookup this edge
  proposes but does not fully design (candidate: `secrets` indexes runtime
  keys by the same `keeper_id`/`environment_id` pair `spend` mints them
  with — see `SpendScope`, spend.md).

- **`agents-spend`** (agents → spend; NEW, thin). *Purpose:* attribute a
  custom agent's completion (OpenRouter or local `inference`) to the keeper
  it ran for, so `spend`'s pull-shaped, per-keeper rollup (spend.md's
  attribution model, ledger #166 Q10) is whole even for non-cc agent
  activity. *Rough shape:* `agents` tags each call with a `SpendScope {
  keeper_id, environment_id: Option<..> }` (the identical shape spend.md
  already defines — not redefined here); for OpenRouter calls this is
  largely already satisfied structurally by `agents-secrets` resolving a
  key that was minted with that scope (spend's OpenRouter key already
  carries `SpendScope` at mint time, so OpenRouter's own usage/limit
  reporting is pre-attributed — `agents` does not need to push a separate
  usage record). For local-`inference` calls (no OpenRouter key involved),
  `agents` is the only party that knows the calling keeper, so it is the
  natural place to stamp `SpendScope` onto the request if/when `inference`
  usage becomes a `spend` source (today `spend.md` lists cc/OpenRouter/aws
  as sources; a local-inference source is not yet named — flagged here as
  a gap this edge would need to fill, not decided). `agents` never computes
  cost, never enforces a budget — pure attribution, mirroring `spend`'s
  aggregator-not-controller law.

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
4. **keeper-on-cc vs. keeper-agents once `agents` exists** (renamed from
   org-on-cc/org-agents, ledger #172). cc.md's `org-on-cc` wires the keeper
   layer→cc directly today. When `agents` lands, how much of that interaction
   re-seats onto `keeper-agents`? Both edges are anticipated; the migration is
   a future design pass's call — owned jointly by whoever designs
   `components/keeper.md` and this file.
5. **What is a "custom agent," concretely?** The operator named the category
   ("some other custom agents") without defining a runtime, plugin model, or
   handle lifecycle for non-CC types — and noted they "might evolve to make
   more use of the concepts within our [system]" rather than the Claude Code
   plugin structure. With the cc-brokered lean gone (INTENT #137), the
   supervision/metering story for custom agents is fully open — it is NOT
   "reuse cc's supervisor" by default. Intentionally left open.
6. **How does a keeper decide cc-vs-custom-runtime for a given thread?**
   (wave-3, NEW.) A per-keeper config default? A per-task-type policy? The
   operator has not said; the "Runtime choice" section above only names that
   `agents` is where the pick is exercised, not how it is made.
7. **Does a local-`inference`-sourced source ever join `spend`'s source
   list?** (wave-3, NEW.) `agents-spend`'s local-inference attribution only
   matters if/when `spend` grows an inference-usage source; spend.md names
   cc/OpenRouter/aws only. Flagged, not decided — self-hosted inference has
   no marginal dollar cost the way OpenRouter/cc do, so whether attribution
   is even useful there (vs. just a utilization metric) is itself unanswered.
