# rollup

**Status:** NEW (round-6 lock, 2026-07-19). **Nesting:** top-level.
**Requirements-only stub — NOT a full design.**
**NAMING OPEN:** this file is `rollup.md` for scaffold purposes, but the crate
name is undecided between **`rollup` / `plugins` / other** — do not treat the
filename as a naming decision.

## Charter (requirements, verbatim-grade)

The **prompt/plugin ROLLUP system** — a big new system, captured
verbatim-grade from the operator:

- **Fragments referencing fragments.** A prompt FRAGMENT can reference other
  fragments **via a syntax**; composition is recursive.
- **Slots.** Some fragments have **SLOTS that take variables at reference
  time** — a fragment is parameterized where it is referenced, not where it
  is defined.
- **Plugin rollup.** Plugins are built from fragments and **generated on
  demand**, fed to agents as needed — "specialized plugins for specialized
  agents with no symlinks or file-copying." Fed into e.g. CCD: **"CCD ideally
  would be built on top of this prompt rollup"** (see `components/ccd.md`).
- **Generalization ladder:** string rollup → file rollup → directory rollup.
  The same referencing/slotting mechanism generalizes from composing strings
  to composing files to composing whole directories (a plugin being the
  directory-rollup case).
- **Prior art (do not design from scratch blindly):** the operator has built
  roughly **two prior versions** of this in other projects — "You should scan
  some of my other projects — it could be in harness already." A prior-art
  scan is underway separately; its findings should seed the design pass.

## Relationships / edges (stubs only)

- **ccd** via `rollup-ccd` — CCD consumes rollup for its plugin/prompt
  assembly: specialized plugins for its specialized agents, generated on
  demand (scaffold/contracts/rollup-ccd.md).
- **projects** — the round-6 .mind workspace-schema migration brings the
  rollup engine into projects ("tasks are rolled up" — see
  `components/projects.md`). Edge naming deferred.

## Nesting

Parent: none | Children: none (this pass).

## Thoroughness level

**requirements-only** — verbatim requirements capture; no design pass yet.
Open: the crate name (rollup/plugins/other), the reference/slot syntax, and
the prior-art reconciliation.
