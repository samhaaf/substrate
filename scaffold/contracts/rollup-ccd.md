# Contract: rollup-ccd

## Parties
ccd  ->  rollup

## What the edge carries
CCD consumes the rollup system for its **plugin/prompt assembly**:
specialized plugins for specialized agents, built from fragments
(fragment-references + slots taking variables at reference time) and
generated on demand — no symlinks or file-copying. Operator: "CCD ideally
would be built on top of this prompt rollup." Direction is inbound-to-rollup
(CCD is the consumer). Schema/example deferred; the reference/slot syntax is
OPEN (the crate name is LOCKED `rollup`, round-9).
**requirements-only** (round-6 lock, 2026-07-19; name locked round-9).
