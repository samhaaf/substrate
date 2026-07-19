# org

**Status:** NEW, **PLACEHOLDER — NOT DECOMPOSED THIS PASS** (maximalist-consumer
exception). **Nesting:** top-level.

A multi-agent-communication / agentic-orgs framework built on top of CCD: agents
message each other, managers own sub-agents, repeatable decisions crystallize into
pipelines, with a metacognition layer running the whole ladder — demoed as a
game-studio-structured org (storyline director, CTO, script director, ...). Org is
the maximalist consumer that will eventually want broad access to every service
(inference/completions, system state, the mesh service-registry, the db control
plane, CCD agent-management). Per the exception rule it is deliberately left
un-designed here; its anticipated needs instead shape the OTHER components'
contracts. Possible convergence with an existing prototype (`~/code/career_crafter`,
branch `Lazarus`) is an open operator question.

**Design note (still a placeholder, not a full design):** this is where the user
creates the foundation for autonomous organizations, where agents, ripples, and
AI pipelines take advantage of all the tools in the rest of the repo to run
organizations autonomously.

**Explicit imports (confirmed this round, minimum set):** `inference`, `ccd`,
and `db`. (`db` in particular backs Org's own self-restructuring knowledge
graph — the metacognitive processes that add/restructure nodes in Org's
org-graph are database operations against `db`.) Everything else about `org`
remains exactly as previously stubbed — not decomposed further this pass.

**Round-6 additions locked into this design stub (still a placeholder, not a
decomposition):**

- **Per-application OWNING agents:** every application gets an owning
  agent/plugin, which can have **sub-agents or peer agents with their own
  plugins**. (Those plugins are natural consumers of the `rollup` system —
  see `components/rollup.md`.)
- **The inter-agent NEGOTIATION protocol:** deliberately redundant
  back-and-forth **before action** — **propose → deliberate →
  counter-propose → tweak → agree** — "a protocol for making decisions
  between two agents before it happens."
