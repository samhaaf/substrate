# Contract: ccd-events

## Parties
mesh  <-  ccd (daemon)
*(was `gateway <- ccd`; gateway merged into mesh, 2026-07-18)*

## What the edge carries
CCD's agent-lifecycle / agent-run WS event stream + REST proxy (agent
spawned/running/idle/exited, turn/report events, per-agent LLM-usage) that
mesh's observability plane aggregates into its fan-out hub, mirroring
`inference-events` and `gc-events`. **PROPOSED by the ccd Component Designer —
pending Decomposer / operator confirmation; not in the original overview
contract graph.** Schema/example deferred.
