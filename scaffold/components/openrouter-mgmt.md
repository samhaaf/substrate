# openrouter-mgmt

> **⚠ STUB TRACK — NOT IMPLEMENTING NOW.** This file is DESIGN NOTES +
> ANTICIPATED DATA CONTRACTS only, per the wave-2 batch-8 plan and INTENT #41.
> Nothing here is implementation-ready: no schemas, no code, no decomposition
> into libs. `openrouter-mgmt` stays a placeholder crate whose *anticipated
> needs shape secrets' and spend's contracts* — the two pairs it touches
> (`openrouter-secrets`, `openrouter-spend`) are named-only until this module
> leaves the stub track. Thoroughness: **requirements-only.**

**Status:** WAVE-2 net-new stub (no repo code; scope introduced at INTENT #41).
**Layer:** L6 organization/ops plane. **Nesting:** top-level app-crate
(crate=app, INTENT #22) — a daemon/CLI entry point when eventually built, never
a library others link (INTENT #29). **Track:** STUB. **Complexity:** L–M (the
lightest kind of L6 service — a thin management skin over an external provider's
own API; "thin and boring" was the explicit ask).

## Charter

`openrouter-mgmt` is the **management service for the operator's OpenRouter
account** — verbatim intent (INTENT #41):

> "I call OpenRouter all the time, so I'd like a service to manage OpenRouter —
> create new OpenRouter API keys, each with their own budgets, manage it within
> the dashboard. That means we probably need a finance component of substrate,
> tracking costs and things like that, and having projects and orgs — not to be
> confused with the org repo, but maybe just nested projects."

Concretely, one bounded job: **provision and govern OpenRouter runtime API keys,
each with its own budget, drivable from the dashboard, with usage/spend readable
downstream.** It is NOT part of `inference` — that plane runs *local* models;
OpenRouter is an external cloud provider, and this service only administers the
operator's account there. It owns key *lifecycle* and *budget-setting*; it does
NOT own cost aggregation/reporting (that is `spend`) or key *storage* (that is
`secrets`). It composes those two services; it reimplements neither.

**Boundary — what openrouter-mgmt does NOT own.** No secret at-rest storage,
encryption, or rotation mechanism (delegated to `secrets` via
`openrouter-secrets`). No cross-source cost rollup or per-project finance
attachment (delegated to `spend`, which PULLS — see below). No enforcement of
budgets in-substrate: OpenRouter's own API caps each key server-side; we set the
limit and read the remainder, we do not re-police it. No calling of OpenRouter
for *inference* — this is account administration only.

## Design notes (operator intent, faithfully)

- **Two-tier key model (the boring shape of OpenRouter's own API).** One
  **provisioning (management) key** — the account root — mints, lists, updates,
  and deletes **runtime keys**, each carrying its own spend `limit` (the
  budget). openrouter-mgmt holds exactly one provisioning key and manages N
  budgeted runtime keys under it. Both tiers live in `secrets`; the service ever
  handles only `SecretRef`s.

- **Key lifecycle = create / rotate / revoke, all via OpenRouter's provisioning
  REST API.** *Create:* mint a runtime key with a budget; OpenRouter returns the
  raw key exactly once → it is written straight into `secrets` and only a
  `SecretRef` is returned to the caller (never surfaced to an agent/LLM path).
  *Rotate:* mint a replacement + revoke the prior key (two provisioning calls),
  updating the stored secret. *Revoke:* delete the runtime key at OpenRouter and
  retire its secret. Deterministic, no agent in the loop.

- **Per-key budgets are set here, enforced there.** The budget is the runtime
  key's `limit`; we set/update it through the provisioning API and OpenRouter
  hard-caps spend against it. Local responsibility is limited to *setting* the
  number and *reading back* limit/usage/remaining — honest and thin, no
  shadow-metering.

- **Usage pulls are PULL-shaped, and this service is a SOURCE, never a pusher.**
  openrouter-mgmt polls OpenRouter for per-key usage/limit/remaining and exposes
  a read surface. Downstream `spend` QUERIES it (plan §L6: *"spend QUERIES its
  sources … OpenRouter budgets later — sources never push"*). openrouter-mgmt
  itself does no cost aggregation and pushes to nobody.

- **Secrets integration is use-without-seeing (INTENT #94), inherited whole from
  `secrets`.** Per secrets.md's own note on `openrouter-secrets`: *"openrouter-
  mgmt reads a `SecretRef`, never the raw key, and uses it via a proxied call."*
  The privileged action (the authenticated HTTPS call to OpenRouter's API) is a
  resolve-into-sink performed by non-LLM code; the raw provisioning/runtime key
  never enters any completion context, prompt, log, or rollup fragment.

- **Dashboard-managed via the standard surface schema (not a bespoke UI).** Per
  INTENT #41 ("manage it within the dashboard") and the schema-driven dashboard
  pattern: openrouter-mgmt publishes a boring surface schema; the Svelte
  dashboard renders its panel from that schema and drives create/rotate/revoke/
  set-budget through the same surface, with a stable semantic `id` on every
  interactive element (INTENT #16) so agents drive the same markup humans do.
  No parallel headless API.

- **Boring surface — CLI + daemon + surface schema**, mirroring `secrets`. The
  daemon exposes the read/manage surface over mesh; the CLI (`openrouter key
  create --budget … | rotate | revoke | ls | usage`) is the local operator path.

## Import surface (anticipated; all mesh-mediated, INTENT #29)

Only `types` + `mesh-client` are compiled-in shared libs. Every service edge is
over the wire.

| Consumes / Exposes | Via | Why |
|--------------------|-----|-----|
| `secrets` (consumes) | `openrouter-secrets` | store/rotate/resolve-into-sink of provisioning + runtime keys; use-without-seeing |
| `spend` (exposes to) | `openrouter-spend` | spend PULLS per-key budget/usage; openrouter-mgmt is the read source |
| `dashboard` (exposes to) | `surface-schema` | schema-driven management panel (standard pattern, not a bespoke edge) |
| OpenRouter cloud API | — (external HTTPS) | provisioning + usage; **NOT a substrate contract edge** (external platform, like the OS keychain / AWS API in secrets.md) |

## Anticipated contracts (wave 2, stub track)

*Names + purpose + rough shape only — schemas deferred until openrouter-mgmt
leaves the stub track. Recorded now so `secrets` and `spend` are shaped for this
consumer/source from the start. Both pairs are already named in wave2-plan §3c.*

- **`openrouter-secrets`** (openrouter-mgmt → secrets; anticipated, both ends
  aware — secrets.md already stubs its side). *Purpose:* durable, use-without-
  seeing storage and rotation of the account provisioning key and every minted
  runtime key. *Rough shape:* ordinary Global/Project-scoped secrets with
  rotation; on create, the once-returned raw OpenRouter key is written and a
  `SecretRef { id, name, scope, version }` handed back; openrouter-mgmt makes
  the authenticated OpenRouter call via a **proxied resolve-into-sink**
  (`use(ref, sink)`), never receiving plaintext into its own LLM-reachable
  paths. Rotation = mint-new-secret + retire-old, tracking the OpenRouter-side
  rotate. No new secrets mechanism — this rides the existing key hierarchy and
  `llm_safe` policy unchanged.

- **`openrouter-spend`** (spend → openrouter-mgmt; anticipated, PULL-shaped,
  openrouter-mgmt is the SOURCE). *Purpose:* let `spend` fold OpenRouter
  per-key budget/usage into its per-project cost tracking. *Rough shape:* a
  read-only query surface exposing, per runtime key, its `{ budget/limit, usage,
  remaining, label/scope }`; `spend` polls it (never a push from here), on the
  same PULL discipline spend already uses for cc's usage DB. The mapping of
  keys → projects/finances is deferred (see open questions).

- **Provisioning-API interaction — EXTERNAL, not a mesh contract.** All
  create/rotate/revoke/usage traffic to OpenRouter is ordinary outbound HTTPS to
  a third party. It is explicitly NOT a service-to-service contract pair in the
  wave-2 inventory (parallel to secrets.md treating the OS keychain and AWS API
  as platform capabilities). Noted here so a future pass does not try to
  externalize it as a substrate edge.

## Open questions

1. **Provisioning-key bootstrap.** Where does the root OpenRouter management key
   first come from? Likely a one-time manual `secrets` enrollment (reveal-once
   into the store), never minted by substrate. Needs confirming as the intended
   trust root.
2. **Key ↔ project mapping.** INTENT #41 ties this to "having projects … maybe
   just nested projects," and per-key budgets read naturally as per-project
   budgets. When `projects` is real, does each runtime key attach to a project
   node (scope + finance), and does openrouter-spend carry that linkage? Deferred
   — both `projects` and this module are stubs.
3. **Is `spend` the whole "finance component"?** INTENT #41 says the OpenRouter
   service *implies* a finance component. wave-2 answers that with the `spend`
   stub (pull-shaped cost tracking). Confirm openrouter-mgmt needs no finance
   logic of its own beyond exposing budget/usage as a source.
4. **Budget-alerting authority.** OpenRouter hard-caps server-side. Is any local
   soft-threshold alerting (e.g. "80% of budget") owned here, or entirely a
   `spend`/dashboard concern? Leaning: read-only here, alerting downstream.
5. **Usage-poll cadence & freshness.** How stale may pulled usage be, and does
   openrouter-mgmt cache/poll on a timer or fetch on demand when `spend` or the
   dashboard asks? Affects whether it needs its own small store or stays
   stateless over OpenRouter's API.
6. **Naming.** `openrouter-mgmt` (inventory name) vs. the operator's phrasing
   "a service to manage OpenRouter" — provisional; revisit at build time
   alongside the `spend`/finance naming.
