# spend

**Status:** WAVE-2 stub (batch 8, L6 organization plane / ops-facing),
2026-07-19. Net-new crate; no repo code. **Track: STUB — design notes +
anticipated data contracts only.** **Nesting:** top-level app-crate
(`bin/spend` + `lib/spend`) — a daemon + noun-verb CLI when eventually built
(crate=app, INTENT #22), never a library others link (INTENT #29).

> ## NOT IMPLEMENTING NOW (INTENT #41/#68, layer-6 stub track)
> This file captures operator intent and names the data contracts `spend` will
> need. It is deliberately **requirements-only**: no schemas, no wire formats,
> no implementation. Per the wave-2 batch plan L6 items get "design stubs with
> anticipated data contracts and an explicit 'not implementing now' note"
> (INTENT #107). Everything below is a placeholder with a real design note;
> content lands when `spend` leaves the stub track.

## Re-spoken round update (2026-07-21/22, INTENT #166 Q10 + #170) — CONFIRMED, anchored to the LANDSCAPE

`spend` is **confirmed** — and its attribution anchor is upgraded from
"per-project rollups" to **landscape topological nodes** (see overview.md's
vocabulary block; projects are the primary topological node, so this
generalizes rather than replaces the framing below):

- **Anchored to landscape topological nodes: per-REGION spend rollups.**
  Cost attaches to the topological node responsible for it; a node's spend
  rolls up over the region of the landscape it owns (the same
  targeted-rollup discipline everything in the landscape gets — INTENT
  #165). Where the text below says "per-project," read "per topological
  node, projects primary."
- **Sources, restated:** (1) **OpenRouter key usage** — keys are
  **attachable to topological nodes or node+environment combos**, so
  per-key usage IS per-region usage by construction; (2) **Claude Code
  usage via cc** (the `spend-cc` pull, unchanged).
- **`openrouter-mgmt` is ABSORBED INTO spend** (the L6 consolidation): key
  lifecycle/budget administration and the usage pull become spend's own
  OpenRouter source adapter rather than a separate crate. The
  enforcement/aggregation split below SURVIVES the merge as an internal
  line: OpenRouter still enforces key budgets provider-side at call time;
  spend's OpenRouter adapter sets budgets and reads back usage — spend
  still never gates a call. See `openrouter-mgmt.md` (absorbed note); the
  `openrouter-secrets` edge re-parties onto spend.
- **LOOSE END, recorded honestly (INTENT #170):** Claude Code reports
  per-run costs, **but on the Max plan runs are effectively free** — spend
  must represent that honestly somewhere (e.g. nominal/metered cost vs
  actual marginal cost as distinct measures), not report Max-plan token
  "costs" as money spent. Unresolved; joins open question 3
  (unit normalization) for the real design pass.

## Charter (requirements)

`spend` is Mind OS's **finance component** — cost tracking across every
money-touching service — named directly by the operator (INTENT #68, verbatim):

> "let's call it spend — spend can just query the cloud code daemon."

And scoped as **pull-shaped** from the start (INTENT #41):

> "That means we probably need a finance component of substrate, tracking costs
> and things like that, and having projects and orgs."

The load-bearing design fact, stated once so nothing downstream forgets it:
**`spend` is an aggregator + reporter, NOT a controller.** It **queries** its
sources, **attributes** cost to projects/environments/callers, and **reports** —
it never blocks a call, never holds a budget, never enforces a limit. Its
sources **never push to it**; it pulls (INTENT #41 verbatim: *"spend can just
query the cloud code daemon"*). This is the whole shape: a boring read-model
over other services' authoritative ledgers.

Sources, in build order:

1. **cc's usage database (first, primary).** cc owns its own usage ledger
   (INTENT #68); `spend` reads it pull-shaped (`spend-cc`).
2. **OpenRouter (later).** Per-key budgets and actual spend surfaced by
   `openrouter-mgmt` (INTENT #41; `openrouter-spend`).
3. **AWS costs (later).** Cloud billing pulled through the `aws` crate — the
   aggregation point for all AWS interfaces (INTENT #106; `spend-aws`,
   anticipated).

Attribution: **per-project rollups.** `spend` groups cost by project /
environment / caller; eventually it "attaches to projects' per-project
metadata/finances" (INTENT #43/#47) — but `projects` is the *grouping
authority*, not a cost store (see below and projects.md §4).

## The enforcement/aggregation split (argued explicitly, per brief)

Budget **ENFORCEMENT** stays where the money is actually spent, at the moment
of the call, under a single-writer authority. `spend` deliberately does **not**
own any of it. Concretely:

- **Claude-Code budgets are cc's job.** cc runs the usage-limits game with
  rich declarative budget/priority strategies — admission verdicts
  (admit/throttle/defer/deny), the token-ceiling guards, the 75th-percentile
  burn-down, caller-priority honoring (INTENT #49, cc.md's `Strategy` engine).
  cc.md already draws this exact line: *"It does not compute spend: it stores
  usage and answers queries; spend computes cost pull-shaped over `spend-cc`."*
  The reverse also holds — `spend` does not compute admission. Enforcement reads
  the ledger *synchronously in the admission path*; `spend` reads the same
  ledger *asynchronously, after the fact*, for reporting. Same data, opposite
  latency contract.
- **OpenRouter per-key budgets are `openrouter-mgmt`'s job.** Per-key creation
  with per-key budgets, dashboard-managed (INTENT #41, openrouter-mgmt's
  charter). The provider (OpenRouter) enforces the key budget at call time;
  `openrouter-mgmt` sets it; `spend` only *reads back* budget-vs-actual to
  report.
- **AWS costs are enforced (if at all) by AWS-side controls** (account
  budgets/alarms), surfaced through the `aws` crate. `spend` reports them.

Why this split and not "spend enforces everything": enforcement must be
**local, synchronous, and single-writer** at each source (you cannot admit a
Claude Code turn against a budget by round-tripping to a separate finance
daemon — that violates single-port locality's spirit and adds a fault line in
the hot path). Aggregation is **global, asynchronous, and read-only** — the
opposite properties. Collapsing them into one service would either make `spend`
a hot-path dependency of every money-touching call (a fault line, and technical
debt by INTENT #38) or scatter reporting logic across cc/openrouter/aws.
Keeping `spend` a pure read-model over authoritative ledgers is the boring,
no-refactor-later shape (INTENT #38/#25). **`spend` answers "what did we
spend?"; the sources answer "may this be spent?".**

## Design notes (operator intent, faithful)

### 1. Pull-shaped, provenance-inherited

`spend` never receives a push of cost data; it queries on demand and on a
schedule (a `cron-api` tenant for periodic recompute/caching). Because every
source ledger is itself provenance-first (INTENT #85/#92 — cc's
`usage_records`/`agent_runs` carry provenance on every row), `spend` **inherits
provenance** rather than inventing its own: a reported cost traces back through
`spend-cc` to the exact metered turn. `spend` may keep a **derived cache** (a
local SQLite read-model, SQLite-only locally per the standing rule) of rolled-up
figures for fast dashboard reads, but the cache is disposable — the source
ledgers are authoritative and re-pullable.

### 2. Per-project attribution rides fields that already exist

The project/environment attribution keys are **already in cc's ledger**:
`agent_runs.project_id?` / `environment_id?` (cc.md concern 2). So per-project
rollup is `spend` reading `usage_records ⨝ agent_runs` **grouped by
`project_id`/`environment_id`/`caller_slug`** — no new push, no duplicated cost
store. `projects` is the authority those ids resolve against (projects.md §4
frames this as *spend's* edge, not projects': *"projects does not push or
duplicate cost data; it is the grouping dimension"*). Parent-project spend is
the rollup of sub-projects' spend, computed over the project hierarchy graph,
not stored redundantly (projects.md §2).

### 3. Cost = money, not local compute

`spend` tracks **billed** money: Claude-Code token spend (cc), OpenRouter
actuals, AWS charges. **Local `inference` completions are not a `spend` source**
— local generation costs electricity, not a metered bill, and `llm-calls` is
metering-shaped for usage, not cost. If a future `agents` type routes paid
provider calls through inference, that provider's billing (e.g. via
`openrouter-mgmt`) is the cost source, not the inference runtime. Noted so a
later pass doesn't wire `spend` into the inference hot path.

### 4. Surface schema — a reporting dashboard

`spend` publishes a **boring surface schema** (INTENT #46, surface-schema.md):
cost-by-project / by-service / by-time-window meters, budget-vs-actual panels
(reading budgets from cc/openrouter to *display*, never to set), and a
per-project finance rollup — rendered schema-driven by the mesh dashboard,
never hand-built. This is `spend`'s primary human surface.

## Import surface

All sources are top-level apps reached **over the wire (mesh-mediated WS/CLI),
never Cargo-linked** (INTENT #29). Only `types` + `mesh-client` are compiled-in
shared libs.

| Consumes | Via | Why |
|----------|-----|-----|
| `cc` | `spend-cc` | pull the usage ledger (PRIMARY, build first) |
| `projects` | `spend-projects` (anticipated) | resolve/label `project_id`s for per-project rollups |
| `openrouter-mgmt` | `openrouter-spend` | per-key budgets + actual spend (build later) |
| `aws` | `spend-aws` (anticipated) | billing/cost pulls through the AWS crate (build later) |

Party to (consumed cross-cutting, not authored): `surface-schema`,
`service-lookup`/`service-registration`, `restart-protocol`, `cron-api`
(scheduled recompute), `pubsub-protocol`.

## Anticipated contracts (wave 2, stub track)

*Names + purpose + rough shape only — schemas deferred until `spend` leaves the
stub track. Recorded now so neighbor contracts are shaped for a pull-shaped
read-model from the start. I do NOT edit `scaffold/contracts/*`.*

- **`spend-cc`** (spend → cc; existing stub-track pair — **spend's flagship
  edge**, wave2-plan §3c). *Purpose:* pull-shaped queries of cc's usage
  database (INTENT #68). *Rough shape:* a **read-only** query surface over
  `usage_records ⨝ agent_runs`, grouped by `project_id`/`environment_id`/
  `caller_slug`/`model`/time-window; returns rolled-up token+cost figures with
  the underlying provenance re-derivable. cc authors the ledger side and has
  already reserved this pair (cc.md: *"pull-shaped spend queries of the usage
  ledger… spend is L6-stub, content deferred"*). **No push, no reverse
  cc→spend edge, no hot-path coupling** — this is the async reporting read that
  mirrors cc's own synchronous admission read.

- **`spend-projects`** (spend → projects; **NEW anticipated — not in the
  wave2-plan §3c inventory; flagged**). *Purpose:* resolve/label the
  `project_id`/`environment_id` keys that arrive on cc's ledger rows into
  project names + the containment hierarchy `spend` rolls up along. *Rough
  shape:* a thin read/resolve edge — `spend` groups by ids from `spend-cc`,
  then asks `projects` (the grouping authority) to resolve ids → names and to
  supply the parent/child edges for hierarchical rollup. Both ends are L6 stubs.
  **Flag:** projects.md §4 already frames per-project finance as *spend's* edge
  ("spend queries cc grouped by project/environment; projects is the authority
  those ids resolve against"), yet §3c lists no `spend-projects` pair — this
  stub proposes it so the per-pair round can confirm or fold the resolution into
  `spend-cc` + a projects read. Deferred either way.

- **`openrouter-spend`** (openrouter-mgmt ↔ spend; existing stub-track pair,
  wave2-plan §3c — the inventory name; the brief's "spend-openrouter" is the
  same edge). *Purpose:* per-key budgets and **actual** OpenRouter spend
  (INTENT #41). *Rough shape:* `spend` pulls budget-vs-actual per key from
  `openrouter-mgmt` (which owns per-key budget *enforcement*); `spend`
  aggregates and reports, never sets a budget. Co-batched sibling
  (`openrouter-mgmt`, batch 8) authors the source half; content deferred both
  ends.

- **`spend-aws`** (spend → aws; **NEW anticipated — not in the wave2-plan §3c
  inventory; flagged**). *Purpose:* pull AWS cloud costs so total spend is
  whole. *Rough shape:* `spend` queries the `aws` crate's WS interface for
  billing/cost figures (per-service, per-tag → mappable to project/environment).
  **Flag:** aws.md currently exposes **no billing/cost surface** (its adapters
  are S3/RDS+Lambda/Secrets-Manager; grep confirms no Cost-Explorer/billing
  leg), and AWS work is broadly design-only in v1 (INTENT #105/#106). So this
  edge requires `aws` to grow a cost/billing adapter first — named here as an
  anticipated future capability, not a v1 pull. Deferred.

## Nesting

Parent: none (top-level app-crate). Children: none designed this pass. A real
design pass would likely decompose into libs: an `ingest` lib (the per-source
pull adapters behind `spend-cc`/`openrouter-spend`/`spend-aws`), a `rollup`
lib (grouping/attribution over project hierarchy — note the name collides with
the `rollup` crate; pick a different lib name), a `cache` lib (the derived
SQLite read-model), and a `surface` lib (the dashboard schema). Flagged, not
built.

## Open questions

1. **`spend-projects` — real pair or folded?** Is per-project resolution its own
   edge, or does `spend` get project labels via a plain `projects` read while
   attribution rides `spend-cc`'s already-present ids? projects.md calls it
   spend's edge; §3c omits it. For the per-pair round / operator.
2. **`spend-aws` timing.** AWS is design-only in v1 and `aws` has no billing
   surface yet. Does `spend`'s v1 stub anticipate a cost adapter in `aws`, or is
   AWS cost tracking explicitly out until AWS itself is real? Needs the operator.
3. **Currency/unit normalization.** cc meters *tokens*; OpenRouter and AWS bill
   *dollars*. Does `spend` normalize everything to a money figure (requiring a
   token→price table per model — where does that live, and who owns pricing
   updates?), or keep tokens and dollars as distinct measures side by side?
   Parked for the real design pass.
4. **Budget *display* vs *enforcement* boundary.** `spend` reads cc/openrouter
   budgets to *show* budget-vs-actual. Confirm the read-only display of a budget
   never tempts a future pass to let `spend` set or gate one — the enforcement
   split (above) must stay hard.
5. **Historical retention.** Source ledgers may prune (session/weekly windows
   roll); does `spend`'s derived cache become the long-term historical record of
   record for finance, and if so what are its own retention/provenance
   guarantees? Deferred.

## Thoroughness level

**requirements-only** — verbatim intent capture + the enforcement/aggregation
argument + anticipated contract sketches; no design pass, no schemas, no
implementation. Layer-6 stub track (INTENT #41/#68/#107): "not implementing
now."
