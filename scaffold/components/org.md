# org

> **⚠ STUB TRACK — NOT IMPLEMENTING NOW.** This file is DESIGN NOTES +
> ANTICIPATED DATA CONTRACTS only, per the wave-2 batch-7 plan and INTENT #20.
> Nothing here is implementation-ready: no schemas, no code, no decomposition
> into libs. `org` stays a placeholder crate whose *anticipated needs shape the
> other components' contracts* — it is the maximalist consumer, designed last,
> built later. Thoroughness: **requirements-only.**

**Status:** WAVE-2 REFIT of the wave-1 placeholder (net-new crate; no repo
code). **Layer:** L6 organization plane. **Nesting:** top-level app-crate
(crate=app, INTENT #22) — a daemon/CLI entry point when it is eventually built,
never a library others link (INTENT #29). **Track:** STUB. **North star, not
this pass's scope** (INTENT #23-context): `org` is meant to eventually
**generalize and replace** the org machinery the operator is hand-building in
the Career Crafter / Lazarus project — *"I'm doing it manually in the career
crafter project, creating this autonomous org thing. Inside substrate we're
going to generalize it, build it on top of the other substrate tools… ideally
in the future I just use org instead of manually building it for different
projects."* Possible convergence with the existing prototype
(`~/code/career_crafter`, branch `Lazarus`) remains an open operator question.

## Charter

`org` is Mind OS's **autonomous-organizations foundation**: the layer where the
user stands up organizations that run themselves — *"this is where the user
creates the foundation for autonomous organizations, where agents […] and AI
pipelines take advantage of all the tools in the rest of the repo to run
organizations autonomously."* (The operator's original vision statement names a
third completion-primitive alongside "agents" and "AI pipelines"; that term is
**RESERVED** and is deliberately not written here.)

Org sits **on top of** CCD and every lower plane. CCD runs, meters, and governs
agent *processes*; `org` owns what CCD explicitly does **not** (per ccd.md's own
boundary): **inter-agent conversation, org structure, manager/sub-agent
hierarchies, pipelines, negotiation, and the metacognition layer** that runs the
whole ladder. Its two locked design pillars this round (INTENT #88) are the
**per-application owning-agent hierarchy** and the **inter-agent negotiation
protocol**. Wave-1 demo shape retained: a game-studio-structured org (storyline
director, CTO, script director, …), managers owning sub-agents, repeatable
decisions crystallizing into pipelines, metacognition running the ladder.

**Boundary — what org does NOT own.** It does not run or supervise agent
processes, does not meter Claude-Code usage, and does not play the usage-limits
game — all of that is CCD, reached inbound-only via `org-on-ccd` with no reverse
CCD→org edge. It does not run completions itself (inference). It does not own
graph storage, schema-locking, or replication (that is `kg`/`vdb`/`db` beneath
it). Org *composes* these services into organizations; it does not reimplement
any of them.

## Design notes (operator intent, faithfully)

- **Per-application OWNING agents (INTENT #88a, locked into the stub).** Every
  application gets an **owning agent/plugin**, which can have **sub-agents or
  peer agents, each with their own plugins.** Those per-agent plugins are the
  natural consumers of the `rollup` system (on-demand plugin assembly, no
  symlinks/copies) — but, per rollup.md/ccd.md, plugin assembly for agents is
  **mediated by CCD** (`rollup-ccd` is rollup's primary consumer); org requests
  a specialized agent+plugin *through* CCD rather than calling rollup directly.

- **The inter-agent NEGOTIATION protocol (INTENT #88b, locked).** Deliberately
  redundant back-and-forth **before action**: **propose → deliberate →
  counter-propose → tweak → agree** — *"a protocol for making decisions between
  two agents before it happens."* This is org's signature primitive and its own
  layer; it is NOT a CCD concern and NOT part of any lower contract.

- **Repeatable decisions crystallize into pipelines**, with a **metacognition
  layer** running the whole ladder (owning agents → sub/peer agents → negotiated
  decisions → crystallized pipelines → metacognitive restructuring).

- **Self-restructuring org-graph (INTENT #19).** Org runs **metacognitive
  processes that add/restructure the nodes of its own org-graph** — *"metacog­
  nitive processes that let it restructure its own knowledge graph, add nodes,
  things like that. That's all database operations."* INTENT #19 framed this as
  a direct `db` dependency; wave-2 has since introduced the **VFS < VDB < KG**
  stack (INTENT #96) and a dedicated `kg` service that IS the mesh-wide,
  schema-locked, provenance-traced knowledge-graph plane (kg.md already lists
  *"consumed by: org (future)"*). Honest reconciliation (flagged as an open
  question below): org's *org-graph* is most naturally a **`kg` graph**, while
  plain relational/metadata needs go to **`db`**. Both edges are anticipated;
  the split is not decided this pass.

- **Provenance is first-order here too.** Because org negotiates and
  restructures autonomously, every decision, pipeline crystallization, and
  org-graph mutation should ride the causal chain (INTENT #85/#92) — org
  inherits provenance from the planes it composes rather than inventing its own.

## Import surface (updated honestly against the L4/L5 designs)

Locked minimum (INTENT #20): **`inference`, `ccd`, `db`.** Honestly, once org is
real it will also lean on **`kg`, `rollup`, and `projects`** (each confirmed by
those components' own wave-2 designs). ALL of these are top-level apps reached
**over the wire (mesh-mediated WS/CLI), never Cargo-linked across app
boundaries** (INTENT #29). Only `types` + `mesh-client` are compiled-in shared
libs.

| Consumes | Via | Why |
|----------|-----|-----|
| `ccd` | `org-on-ccd` | inter-agent process/agent-management substrate (PRIMARY) |
| `inference` | `v1-completion-api` | LLM completions for agents/pipelines |
| `db` | `db-control-plane` | plain relational/metadata store |
| `kg` (realistic) | `kg-api` | the self-restructuring org-graph (see reconciliation note) |
| `rollup` (realistic) | via `ccd` (`rollup-ccd`) | specialized per-agent plugin assembly — mediated by CCD |
| `projects` (realistic) | `org-projects` (anticipated) | "per-application" ≈ per-project scoping |
| `spend` (realistic, later) | via `projects` finances / `spend-ccd` chain | cost-awareness for budget-honoring autonomous work — see note |

**On `spend` (honest status).** `spend` is not designed yet — it is batch 8
(ops-facing), sequenced AFTER org's own batch-7 pass — so org cannot pin a
finished contract against it this round. But an org that runs autonomous work
"without draining my usage" (INTENT #63-context) realistically wants
**cost-awareness reads**: what has this owning-agent / project already spent,
and is it within budget before it negotiates the next action. That read arrives
**pull-shaped and second-hand** — org reads per-project finances through
`projects` (which owns per-project metadata/finances), and `spend` itself
computes cost pull-shaped over `spend-ccd` (per spend's charter + ccd.md's
boundary: CCD stores usage, `spend` computes). Org does NOT get a push feed and
does NOT compute cost itself. Recorded as a realistic-but-unpinned edge so the
batch-8 `spend` designer knows org is a downstream cost-reader.

## Anticipated contracts (wave 2, stub track)

*Names + purpose + rough shape only — schemas deferred until org leaves the stub
track. Recorded now so neighbor contracts are shaped for a broad consumer from
the start.*

- **`org-on-ccd`** (org → ccd; existing stub — org's assigned pair).
  *Purpose:* org's dependency on CCD for inter-agent communication **plus** the
  "maximalist-consumer read bundle" across the other planes. *Rough shape:* an
  inbound-only edge (no CCD→org reverse dep) bundling (a) agent lifecycle/handle
  operations delegated to CCD's `agent-management`, and (b) broad shaped reads
  the org layer needs — completions, system-state, service-registry, db control
  plane. Keep it a broad forward-consumer envelope; the org-side semantics
  (negotiation, hierarchy) sit ABOVE this edge and are not carried by it.

- **`v1-completion-api`** (org → inference; existing cross-service stub).
  *Purpose:* org agents and crystallized AI pipelines run completions on the
  local inference plane. *Rough shape:* org is one more client of the standard
  `/v1/` REST+WS completion surface forwarded through mesh — no org-specific
  variant.

- **`db-control-plane`** (org → db; existing stub — "org is going to have a
  database in it", INTENT #19). *Purpose:* org's relational/metadata state and,
  per INTENT #19's original framing, the database operations behind its
  self-restructuring. *Rough shape:* org as a noun-verb control-plane consumer
  of `db` (migrations / query / edge functions), mesh-mediated. See the kg
  reconciliation — the *graph-shaped* portion likely migrates to `kg-api`.

- **`kg-api`** (org → kg; NEW anticipated, per kg.md "consumed by: org
  (future)"). *Purpose:* org's org-graph as a first-class, schema-locked,
  provenance-traced knowledge graph — the true home of the metacognitive
  add/restructure-nodes behavior. *Rough shape:* org creates/reads/mutates a
  registered graph via kg's graph API; schema-locking pushes validation
  failures back to org (rejected, never coerced). Whether the org-graph is one
  graph or per-organization graphs is open.

- **`org-projects`** (org → projects; NEW anticipated, stub-track both ends).
  *Purpose:* wire "per-application owning agents" to the `projects` plane —
  an "application" the operator organizes ≈ a (possibly nested) project, so
  owning-agent scope, per-project finances, and topological structure line up.
  *Rough shape:* org resolves/attaches owning agents to project nodes; likely a
  thin read/attach edge. Both ends are stubs, so this stays a named intention.

- **`spend` reads — anticipated, unpinned (batch 8, not yet designed).**
  *Purpose:* budget-honoring autonomous work needs to know accrued cost per
  owning-agent/project before negotiating the next action. *Rough shape:*
  pull-shaped, second-hand — org reads per-project finances via `projects`,
  and `spend` computes cost pull-shaped over `spend-ccd`; org never gets a push
  feed and never computes cost itself. No org↔spend stub is minted this pass
  (spend is designed after org); named here so the batch-8 designer knows org
  is a downstream cost-reader.

- **Negotiation protocol — INTERNAL, not a mesh contract.** propose →
  deliberate → counter-propose → tweak → agree runs *between agents inside an
  org*. It is org's own concern and is explicitly NOT a service-to-service
  contract pair in the wave-2 inventory. Noted here so a future design pass does
  not mistakenly try to externalize it.

## Open questions

1. **db vs. kg for the org-graph.** INTENT #19 says the self-restructuring
   graph is `db` operations; wave-2's `kg` (on `vdb` on `db`) is the purpose-
   built graph plane and already anticipates org as a consumer. Which owns the
   org-graph — `kg` for graph structure with `db` for flat metadata, or `db`
   alone as originally stated? Needs the operator, given INTENT #96 postdates
   #19.
2. **rollup: direct vs. CCD-mediated.** Owning-agent plugins are rollup
   consumers, but rollup.md makes `rollup-ccd` the primary path. Does org ever
   call rollup directly, or always request agents+plugins through CCD?
3. **The reserved third primitive.** The operator's vision names a reserved
   completion-primitive term alongside agents and AI pipelines. Its meaning for
   org's execution model is unresolved and intentionally unnamed here.
4. **Negotiation protocol depth.** Is propose→deliberate→counter→tweak→agree a
   fixed 5-phase state machine or a family of negotiation patterns? Affects
   whether it needs a persisted, provenance-traced record (likely, INTENT
   #85/#92) and where that record lives (org-graph vs. db).
5. **Career Crafter / Lazarus convergence.** How much of the hand-built
   `~/code/career_crafter` (Lazarus) org model is prior art `org` should
   generalize from when it is finally designed for real?
