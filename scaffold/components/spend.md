# spend

**Status:** WAVE-3 consolidation pass (unit `spend-consolidated`, 2026-07-22).
Net-new crate; no repo code. **Track: STUB — design notes + anticipated data
contracts only**, but this pass takes the design further than a bare stub:
attribution model, dual-accounting shape, and adapter boundaries are now
implementation-ready even though schemas/wire types stay sketches until
`spend` leaves the stub track. **Nesting:** top-level app-crate (`bin/spend` +
`lib/spend`) — a daemon + noun-verb CLI when eventually built (crate=app,
INTENT #22), never a library others link (INTENT #29).

> ## NOT IMPLEMENTING NOW (INTENT #41/#68/#107, layer-6 stub track)
> This file captures operator intent and shapes the data contracts `spend`
> will need. It is deliberately **requirements + design, no code**: the
> attribution model and dual-accounting rules below are decided; the wire
> schemas are sketches (`## Proposed contracts` below) that land for real when
> `spend` leaves the stub track.

## Consolidation summary (this pass — INTENT #166 Q10, #170, #68, #41)

`spend` is Mind OS's one finance/reporting surface, now fully folding in what
used to be scattered across three round's-worth of notes:

1. **Anchored to landscape keepers, per-region rollup** (#166 Q10). Cost
   attaches to the **keeper** (the topological node) responsible for it, and
   rolls up over the region of the landscape a keeper owns — the same
   targeted-rollup discipline everything on the landscape gets (#165). Project
   is the primary keeper type, so "per-project spend" from earlier rounds is
   the special case of "per-keeper spend," not a different thing.
2. **`openrouter-mgmt` is absorbed** — key lifecycle (create/rotate/revoke)
   and per-key budget administration are now `spend`'s own OpenRouter source
   adapter, not a separate crate. Enforcement still happens at OpenRouter
   (provider-side hard cap); `spend` sets the number and reads it back.
3. **cc usage stays the flagship source**, and this pass designs the honest
   answer to the #170 loose end: Claude Code reports per-run token costs, but
   on the Max plan those runs are **effectively free**. `spend` represents
   that with **nominal-vs-billed dual accounting, plan-aware** (below) — the
   exact allocation policy is marked OPEN for the operator's closing round,
   not decided here.
4. **AWS billing via `aws`** joins the source list, **design-only** — `aws`
   has no billing/cost adapter yet and none is built in v1 (INTENT #105/#106);
   this file sketches the shape so a future pass has something to build
   against.
5. **Pull-shaped aggregation stays; enforcement stays OUT.** Nothing below
   changes the load-bearing rule: `spend` reads, attributes, reports — it
   never blocks a call, never holds a budget, never enforces a limit. `cc`'s
   strategies enforce Claude Code usage; OpenRouter enforces its own key
   budgets; `spend` only reports on both.

## Charter (requirements)

`spend` is Mind OS's **finance component** — cost tracking across every
money-touching service — named directly by the operator (INTENT #68,
verbatim):

> "let's call it spend — spend can just query the cloud code daemon."

And scoped as **pull-shaped** from the start (INTENT #41):

> "That means we probably need a finance component of substrate, tracking
> costs and things like that, and having projects and orgs."

The load-bearing design fact, stated once so nothing downstream forgets it:
**`spend` is an aggregator + reporter, NOT a controller.** It **queries** its
sources, **attributes** cost to keepers/environments/callers, and
**reports** — it never blocks a call, never holds a budget, never enforces a
limit. Its sources **never push to it**; it pulls (INTENT #41 verbatim:
*"spend can just query the cloud code daemon"*). This is the whole shape: a
boring read-model over other services' authoritative ledgers.

Sources, in build order:

1. **cc's usage database (first, primary).** cc owns its own usage ledger
   (INTENT #68); `spend` reads it pull-shaped (`spend-cc`), now plan-aware
   (below).
2. **OpenRouter (second — absorbed adapter, not a separate crate).** Key
   lifecycle + per-key budgets + usage, administered directly by `spend`'s
   own OpenRouter source lib.
3. **AWS costs (design-only, later).** Cloud billing pulled through the `aws`
   crate — the aggregation point for all AWS interfaces (INTENT #106;
   `spend-aws`, anticipated, no adapter exists yet in `aws`).

Attribution: **per-keeper, per-region rollup.** `spend` groups cost by
keeper / environment / caller; a keeper's spend rolls up over the subtree of
the landscape it owns (its own activity plus its sub-keepers') — the standard
targeted-rollup rule (#165), not a bespoke finance hierarchy.

## The enforcement/aggregation split (unchanged law, restated once)

Budget **ENFORCEMENT** stays where the money is actually spent, at the moment
of the call, under a single-writer authority. `spend` deliberately does
**not** own any of it:

- **Claude-Code budgets are cc's job.** cc runs the usage-limits game with
  declarative budget/priority strategies — admission verdicts (admit/
  throttle/defer/deny), token-ceiling guards, the 75th-percentile burn-down,
  caller-priority honoring (INTENT #49, cc.md's `Strategy` engine). cc.md
  draws this exact line: *"It does not compute spend: it stores usage and
  answers queries; spend computes cost pull-shaped over `spend-cc`."*
  Enforcement reads the ledger *synchronously in the admission path*; `spend`
  reads the same ledger *asynchronously, after the fact*, for reporting. Same
  data, opposite latency contract.
- **OpenRouter per-key budgets are `spend`'s own adapter's job, enforced by
  OpenRouter.** `spend` mints/rotates/revokes runtime keys and sets their
  budget `limit` through OpenRouter's provisioning API; **OpenRouter itself
  hard-caps spend against that limit at call time** — `spend` sets the number
  and reads the remainder, it does not re-police it in-process.
- **AWS costs are enforced (if at all) by AWS-side controls** (account
  budgets/alarms), surfaced through the `aws` crate. `spend` reports them.

Why this split and not "spend enforces everything": enforcement must be
**local, synchronous, and single-writer** at each source; aggregation is
**global, asynchronous, and read-only** — the opposite properties. Collapsing
them into one service would either make `spend` a hot-path dependency of
every money-touching call (a fault line, technical debt by INTENT #38) or
scatter reporting logic across cc/OpenRouter/aws. **`spend` answers "what did
we spend?"; the sources answer "may this be spent?"**

## Attribution model — keeper / keeper+environment scope

A single scope type covers every source (INTENT #166 Q10 verbatim: "OpenRouter
key usage attached to nodes or node+environment combos"; generalizes cleanly
to cc callers and AWS cost tags):

```rust
// conceptual shape — belongs in `types` when spend leaves the stub track,
// NOT authored here (flagged, see Proposed contracts)
struct SpendScope {
    keeper_id: KeeperId,               // the landscape topological node the cost is attributed to
    environment_id: Option<EnvironmentId>, // opaque until `environments` is real (OQ-2, parked)
}
```

- Every OpenRouter runtime key, every cc `agent_runs` row, and (eventually)
  every AWS cost-allocation tag carries a `SpendScope`. Attribution is
  therefore **structural** (the id is already on the source row/key), not a
  separate join spend invents.
- **Per-region rollup** = summing `SpendScope`s over the subtree a keeper
  owns in the landscape KG, the same walk `rollup`/`kg` already do for
  targeted rollup (kg.md's bounded-traversal `kg-api`) — `spend` does not
  grow its own hierarchy walker.
- **Resolving `keeper_id` → name / region / parent-child edges rides
  `kg-api`** (kg.md's universal "any service ↔ kg" surface: get-by-id,
  list/scan by type, bounded neighbor traversal). This **retires the earlier
  `spend-projects` proposal** — there is no bespoke resolution contract;
  `spend` is just another `kg-api` consumer, like every other service. (See
  Proposed contracts.)
- A keeper's own bundle/region is the unit of the per-region rollup; nothing
  about `spend`'s scope type depends on whether the keeper is a project,
  sub-project, or any future ownable node type (OQ-19) — it only needs a
  `KeeperId`.

## Sources

### 1. cc usage — plan-aware, dual accounting (INTENT #68, #170)

cc's ledger (`usage_records ⨝ agent_runs`) is the authoritative metering
signal: token counts, model, and `SpendScope` fields per turn. `spend` pulls
it (`spend-cc`, unchanged pull discipline) and computes cost from a token→
price table `spend` itself owns (cc returns tokens + provenance, never a cost
figure — cc.md's own line).

**The #170 loose end, designed honestly:** Claude Code meters every turn, but
under the operator's **Max plan** a run's marginal cost is $0 — it is covered
by a flat recurring subscription fee, not billed per-token. Reporting the
list-price figure as "money spent" would be dishonest; reporting nothing at
all would throw away the only signal `spend` has for utilization,
prioritization, and "what would this have cost on pay-as-you-go." So every
cc-sourced row carries **two cost figures, not one**:

```rust
struct CcCostRow {
    scope: SpendScope,
    plan: BillingPlan,          // Max | ApiPayAsYouGo | Unknown — see dependency note below
    nominal_cost: Money,        // tokens x the model's list price, ALWAYS computed
    billed_cost: Money,         // what was actually charged for this run
}
```

- **`nominal_cost`** is always the counterfactual list-price figure — the
  same token→price table used for pay-as-you-go rows. It never depends on
  plan; it is "what this would have cost."
- **`billed_cost`** is plan-aware: under `ApiPayAsYouGo`, `billed_cost ==
  nominal_cost` (modulo the cache-read/write discount classes the price table
  already carries); under `Max`, `billed_cost == 0` for every individual run
  — the marginal cost genuinely is zero.
- The Max plan's **flat subscription fee** is its own line item — a
  fixed-cost record scoped to the *billing period*, not to any run or keeper
  — recorded once, never split across runs by default (the boring choice:
  don't invent an allocation formula nobody asked for).
- **OPEN POLICY QUESTION, marked explicitly for the operator's closing
  round, not decided here:** should the Max subscription's flat fee ever be
  *allocated* down to keepers (e.g. proportional to each keeper's share of
  `nominal_cost`), so a per-keeper "billed" total reflects a fair-use share of
  the subscription — or does `spend` stay at the boring default (per-run
  `billed_cost = 0` under Max, the subscription fee reported only as one
  unattributed period-level line)? Both are honest; only the operator's
  answer picks one, and it is not the most boring option in either direction,
  so INTENT #124/#173d routes it to the closing round rather than a designer
  guess.
- **Dependency flagged, not decided here:** this requires cc's ledger to
  carry a `plan`/`billing_mode` signal (today `cc.md`'s schema has none — no
  `sessions`/`agent_runs` column tracks which billing plan was active). See
  `## Proposed contracts` for the sketch; cc's own designer/the batch-7
  harmonizer applies or adjusts it. Until that lands, `spend` may default
  every row to `plan: Unknown, billed_cost = nominal_cost` (fail toward
  *overstating* cost, never toward silently hiding it).

### 2. OpenRouter — absorbed adapter (INTENT #41, #166 Q10)

`openrouter-mgmt` is **not a separate crate**; its whole design note below is
`spend`'s own internal OpenRouter source lib.

- **Two-tier key model.** One **provisioning (management) key** — the
  account root — mints, lists, updates, and deletes **runtime keys**, each
  carrying its own spend `limit` (the budget) and its own `SpendScope`
  (attached to a keeper, or a keeper+environment combo, at mint time).
  `spend` holds exactly one provisioning key and manages N budgeted runtime
  keys under it. Both tiers live in `secrets`; `spend`'s adapter ever handles
  only `SecretRef`s.
- **Key lifecycle = create / rotate / revoke**, all via OpenRouter's
  provisioning REST API. *Create:* mint a runtime key with a budget +
  `SpendScope`; OpenRouter returns the raw key exactly once → it is written
  straight into `secrets` and only a `SecretRef` is returned to the caller
  (never surfaced to an agent/LLM path). *Rotate:* mint a replacement +
  revoke the prior key (two provisioning calls), updating the stored secret.
  *Revoke:* delete the runtime key at OpenRouter and retire its secret.
  Deterministic, no agent in the loop.
- **Per-key budgets are set here, enforced there.** The budget is the runtime
  key's `limit`; `spend` sets/updates it through the provisioning API and
  OpenRouter hard-caps spend against it. `spend`'s responsibility is limited
  to *setting* the number and *reading back* limit/usage/remaining — no
  shadow-metering.
- **Usage pulls are on the same pull discipline as cc.** `spend` polls
  OpenRouter for per-key usage/limit/remaining on the same async,
  after-the-fact cadence it uses for `spend-cc`. `billed_cost` for OpenRouter
  rows is always OpenRouter's own reported actual (there is no Max-style flat
  fee here — every OpenRouter row has `nominal_cost == billed_cost`).
- **Secrets integration is use-without-seeing (INTENT #94)**, unchanged from
  the pre-absorption design: `spend`'s OpenRouter adapter reads a `SecretRef`,
  never the raw key, and uses it via a proxied resolve-into-sink
  (`secrets.use(ref, sink)`) — the raw provisioning/runtime key never enters
  any completion context, prompt, log, or rollup fragment.
- **Dashboard-managed via the standard surface schema** (INTENT #41, #46):
  `spend` publishes a boring surface schema; the dashboard renders
  create/rotate/revoke/set-budget from that schema, with a stable semantic
  `id` on every interactive element (INTENT #16).

### 3. AWS billing — design-only (INTENT #105/#106)

`aws` is the single aggregation point for every AWS-facing surface (aws.md),
but it currently exposes **no billing/cost adapter** — its adapters are
S3/RDS+Lambda/Secrets-Manager only, and AWS work is broadly design-only in v1
("we're not doing pretty much anything in AWS right now," INTENT #105).
`spend`'s AWS source is therefore named and shaped, not built:

- **Shape:** `spend` would query `aws`'s WS interface for Cost-Explorer-style
  billing figures, grouped by AWS cost-allocation tag. For those figures to
  resolve to a `SpendScope`, every AWS resource `aws` provisions would need a
  `keeper_id` (and optionally `environment_id`) cost-allocation tag at
  creation time — a dependency on `aws`'s own provisioning adapters, not
  decided here.
- **`nominal_cost == billed_cost`** for AWS rows, same as OpenRouter — AWS
  billing has no flat-fee-vs-marginal split like the Max plan does; the
  dual-accounting columns exist for schema uniformity across sources, not
  because AWS needs both.
- **Not built until `aws` grows a billing leg.** This is the anticipated
  future capability; see `spend-aws` below.

## Surface schema — a reporting dashboard

`spend` publishes a **boring surface schema** (INTENT #46, surface-schema.md):
cost-by-keeper / by-region / by-service / by-time-window meters, a
nominal-vs-billed comparison panel (making the Max-plan gap visible rather
than hidden), budget-vs-actual panels for OpenRouter keys (reading budgets to
*display*, never to set), and the per-keeper region rollup. Rendered
schema-driven by the mesh dashboard, never hand-built. This is `spend`'s
primary human surface.

## Import surface

All sources are reached **over the wire (mesh-mediated WS/CLI), never
Cargo-linked** (INTENT #29), except the OpenRouter provisioning/usage API,
which is ordinary outbound HTTPS to a third party (parallel to `secrets`
treating the OS keychain / AWS API as platform capabilities, not mesh
contracts). Only `types` + `chassis` are compiled-in shared libs.

| Consumes | Via | Why |
|----------|-----|-----|
| `cc` | `spend-cc` | pull the usage ledger, plan-aware (PRIMARY, build first) |
| `kg` | `kg-api` (generic surface, not a bespoke pair) | resolve `keeper_id`s to names/regions and walk the subtree for per-region rollup |
| `secrets` | `openrouter-secrets` (party already re-homed to `spend`) | store/rotate/resolve-into-sink of the provisioning + runtime OpenRouter keys |
| OpenRouter cloud API | — (external HTTPS, not a mesh contract) | key lifecycle + usage pulls |
| `aws` | `spend-aws` (anticipated, design-only) | billing/cost pulls through the AWS crate (build later, after `aws` grows a billing adapter) |

Party to (consumed cross-cutting, not authored): `surface-schema`,
`service-lookup`/`service-registration`, `restart-protocol`, `queues`
(scheduled recompute of the derived cache), `pubsub-protocol`.

## Proposed contracts (wave 3)

*Sketches only — I own `components/spend.md`, not `scaffold/contracts/*`; the
shapes below are proposals for whichever side/harmonizer next touches each
contract file. Schemas stay deferred until `spend` leaves the stub track.*

- **`spend-cc` — proposed extension: plan-aware rows.** Add a `plan:
  BillingPlan` field (`Max | ApiPayAsYouGo | Unknown`) to the `UsageRow`/
  `UsageKey` cc returns, sourced from a **new column on cc's own ledger**
  (e.g. `sessions.billing_plan`, populated from whatever signal cc has for
  which account/plan is active — operator-set config is the boring default
  until cc auto-detects it). `spend` then computes `nominal_cost` from tokens
  + its own price table (unchanged) and derives `billed_cost` from `plan`
  per the dual-accounting rule above. This is an **additive** field — no
  existing `spend-cc` shape changes, cc's designer/batch-7 confirms the
  column name and default.
- **`openrouter-spend` — retire as a cross-crate contract.** Since
  `openrouter-mgmt` is absorbed into `spend`, this pair no longer crosses a
  process boundary — it is `spend`'s own internal module interface (source
  lib → aggregation lib), not a mesh contract. Propose tombstoning
  `contracts/openrouter-spend.md` at the next harmonization pass (batch 7);
  its content already lives in `## Sources > 2. OpenRouter` above.
- **`openrouter-secrets` — no shape change, party already correct.** The
  contract file already reflects `spend` as the consumer (re-spoken round);
  no further edit needed there.
- **`spend-aws` (spend → aws) — NEW, design-only.** *Purpose:* pull AWS cloud
  costs, tagged by `keeper_id`(+`environment_id`), so total spend is whole.
  *Rough shape:* `spend` queries `aws`'s WS interface for cost/billing
  figures per cost-allocation tag; `aws` would need a Cost-Explorer-shaped
  adapter it does not have yet. Not buildable until `aws` grows that leg
  (aws.md flags no billing surface exists) — named here so the edge is
  shaped when that day comes.
- **`SpendScope` — proposed shared type.** `{ keeper_id: KeeperId,
  environment_id: Option<EnvironmentId> }` belongs in `types` (not authored
  here) so cc, `spend`'s OpenRouter adapter, and (eventually) `aws`'s
  cost-allocation tags all reference the identical scope shape instead of
  three near-duplicate ones.

## Nesting

Parent: none (top-level app-crate). Children: none built this pass. A real
design pass would likely decompose into libs: a `cc-source` lib (the
`spend-cc` pull, price table, dual-accounting math), an `openrouter-source`
lib (key lifecycle + usage, replacing the old `openrouter-mgmt` crate
entirely), an `aws-source` lib (stub until `aws` grows billing), a `rollup`
lib (per-region grouping over `kg-api`'s traversal — name collides with the
`rollup` crate; pick a different lib name), a `cache` lib (the derived
SQLite read-model, SQLite-only locally per the standing rule), and a
`surface` lib (the dashboard schema). Flagged, not built.

## Open questions

1. **Max-subscription allocation policy (#170).** Per-run `billed_cost = 0`
   with the flat fee left unattributed (boring default, assumed above), or
   allocate the subscription fee down to keepers proportional to
   `nominal_cost` share? Marked explicitly for the operator's closing round —
   not the most boring single option, so not decided here.
2. **Plan-detection mechanism.** Does cc's `billing_plan` column come from
   operator-set config, or can cc auto-detect Max-vs-API from its own CLI
   signals? Needs cc's designer; `spend` defaults to `Unknown` until this
   lands.
3. **Currency/unit normalization.** cc meters *tokens*; OpenRouter and AWS
   bill *dollars*. Does `spend` keep tokens and dollars as distinct measures
   side by side (current lean), or normalize everything to money (requiring
   a token→price table per model — `spend` already owns this for
   `nominal_cost`, but confirm it's the single source of pricing truth)?
   Parked for the real design pass.
4. **OpenRouter key scope cardinality.** Can one runtime key's `SpendScope`
   name more than one keeper (a shared team key), or is it always exactly one
   keeper (+ optional environment)? Leaning single-scope-per-key (simplest,
   matches "attachable to nodes or node+environment combos" read literally);
   confirm.
5. **`spend-aws` timing.** AWS is design-only in v1 and `aws` has no billing
   surface yet. Does a future `aws` pass build the billing adapter
   proactively, or only when AWS spend becomes real? Needs the operator.
6. **Historical retention.** Source ledgers may prune (session/weekly windows
   roll); does `spend`'s derived cache become the long-term historical record
   of finance, and if so what are its own retention/provenance guarantees?
   Deferred.

## Thoroughness level

**Requirements + design, no code.** Verbatim intent capture, the
enforcement/aggregation argument, the keeper-anchored attribution model, the
nominal-vs-billed dual-accounting design (with the #170 policy question
explicitly marked open), and proposed contract sketches for the next pass.
No schemas, no implementation. Layer-6 stub track (INTENT #41/#68/#107):
"not implementing now."
