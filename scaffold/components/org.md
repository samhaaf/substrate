# org

> **⚠ TOMBSTONE — the `org` CRATE IS DISSOLVED (friction-round 3,
> 2026-07-20, INTENT #132).** There is no `org` crate, no `org` daemon, no
> `org` registry slug. The operator, verbatim: "We don't need an org crate.
> **Org is emergent — it just is a bunch of coordinators.** Maybe in the
> future an org node with a specific owner attached, like the chief in the
> agentic-business idea. Let's not start with the idea of an autonomous
> organization and build down — keep building up and let autonomous orgs be
> emergent." This file is retained as the record of what org IS (an emergent
> structure) and as the home of the coordinator/owner concept material it
> dissolves into. The wave-2 `org` component design (charter, negotiation
> protocol, import surface, anticipated contracts) lives in git history
> (`git log -- scaffold/components/org.md`); its still-live pieces are
> restated below. The `org-on-cc` contract stub survives as the
> shaped-for-a-broad-consumer record only (see that file's tombstone note).

## What org IS (emergent, not built)

- **An org is a bunch of coordinators.** No crate builds it; it emerges from
  coordinators existing and being related to one another.
- **Org-as-KG (INTENT #132, continuing #121):** "org is probably just a
  knowledge graph linking a bunch of coordinators," with **typed edges** —
  `reports-to`, `delegates-to`, `may-create-sub-coordinators`. The graph
  lives ON `kg` (INTENT #121 stands: "Org's self-restructuring knowledge
  graph definitely belongs in the KG service"; the design posture "assume
  that every service will be using the knowledge graph" came from this
  thread and stays in kg.md's charter).
- **Possibly, later: an org NODE.** "Maybe in the future an org node with a
  specific owner attached, like the chief in the agentic-business idea" — a
  KG node type, not a crate. Build-up, not build-down: autonomous orgs stay
  emergent.

## The coordinator/owner concept — FIRST-ORDER, DISCUSSION REQUIRED, not designed

The owners-endgame material (friction-round 2, INTENT #127) now sits
conceptually under the **coordinator/archetype concept** — because the
operator resolved (friction-round 3, INTENT #130): **coordinator and owner
ARE THE SAME THING.**

**INTENT #130, verbatim-grade — the concept becomes a MINI-HARNESS:** "They
are the same thing. The question is accessing it sometimes directly and
sometimes via a messaging system. Maybe this coordinator concept is more
like a **mini harness that can have multiple threads**: a single thread the
user talks to, and specific message threads that handle each incoming
message in a dedicated, persistent thread. If we establish a **protocol
that's kind of like HTTP but for sending one message to a workspace
coordinator**, we can facilitate the entire back-and-forth in one thread,
handle it, and then **broadcast to other threads running in the same
space** — like the human thread — so it has awareness that something was
changed by a 'headless coordinator.' **Messaging capability built in, a
dedicated workspace construct, tools to interface with the workspace.**"
(Mid-thought — the operator paused here to continue.)

**INTENT #133, the sharpening:** each coordinator has its own KG and
workspace; keep the workspace concept **AS IT IS** (per-feature-ish, the
existing .mind construct — "I don't like opening up the realm of all
possibilities for the workspace"); the .mind structure's
project/feature/app node types align with owners; **per-node-TYPE directory
structures** ("like a workspace but created specifically for that node
type, tracked within the schema service"). The archetype wants a name —
deva, angel, "a spiritual construct, an archetype — but grounded, not
mystical." Move from the old coordinator idea toward "something more
persistent and specific."

**The endgame (friction-round 2, INTENT #127, carried under this concept
now):** "That is it, dude. That is what we're going for."

- **An owner ( = a coordinator with NO human in the loop)** is idle unless
  woken, owning a thing — an app, a project, a component.
- **The flow:** a user's coordinator sends feedback to an owner → the owner
  examines its thing, responds with a **proposal + new data contract** →
  the requester (human-in-the-loop via their own coordinator) **approves** →
  the owner **dispatches subagents as a coordinator would** → reports
  **completion + new contract + version info**. "All of our boring services
  will be very easy to update."
- **The name is `owner`** (preferred over manager/lead): "it owns a thing;
  once we build a thing, we put an owner in charge of it, and that's how we
  improve the thing in the future."
- **Owners/coordinators arrange HIERARCHICALLY over the topological map**
  of projects / sub-projects / components — THE primitive for (emergent)
  autonomous organizations: agents communicating, accessing specialized
  knowledge, pushing back, keeping contract alignment.
- **Standing open questions, the operator's own:** the KG relationship —
  how much graph feeds an owner on wake (leaning: owners wake to a
  perfectly organized workspace)? How to keep the topological map linearly
  separable — add coordinators, establish relationships, sometimes merge
  them? (The former "does this live in org or projects?" question is
  answered by dissolution: it lives in the coordinator concept + KG.)

**Status: DISCUSSION-REQUIRED first-order concept.** Recorded
verbatim-grade so it shapes the dedicated coordinators discussion round
(INTENT #123, still pending), NOT so anyone builds from this text. Nothing
here is designed — no schemas, no protocol frames, no crate shape. The
"HTTP-like message protocol to a workspace coordinator" and the
multi-threaded mini-harness are sketches awaiting that round.

## What dissolves where (disposition of the old org design)

- **Org structure / hierarchy / metacognition** → emergent over
  coordinators + the KG typed-edge graph (above).
- **The org-graph** → `kg` (INTENT #121, unchanged).
- **Per-application owning agents (INTENT #88a)** → the coordinator/owner
  concept (an owner per built thing).
- **The inter-agent negotiation protocol (INTENT #88b: propose → deliberate
  → counter-propose → tweak → agree)** → carried as coordinator-concept
  material for the discussion round; still explicitly NOT a
  service-to-service mesh contract.
- **The Career Crafter / Lazarus generalization north star** → stands as
  prior art for the coordinator rounds (`~/code/career_crafter`, branch
  `Lazarus`).
- **`org-on-cc`** → survives as a stub recording that cc's surfaces
  (`agent-management`, priorities honored by the strategy engine) were
  shaped for a broad downstream consumer; the consumer is now "coordinators"
  rather than an org crate.
- **`org-projects`, org's `kg-api`/`db-control-plane`/`v1-completion-api`
  consumption** → whatever construct the coordinator rounds produce will
  consume these surfaces; the shaped-for notes in those contracts stand.
