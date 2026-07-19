# Contract: ccd-events

## Parties
gateway  <-  ccd (daemon)

## What the edge carries
CCD's agent-lifecycle / agent-run WS event stream + REST proxy (agent
spawned/running/idle/exited, turn/report events, per-agent LLM-usage) that the
gateway aggregates into its fan-out hub, mirroring `inference-events` and
`gc-events`. **PROPOSED by the ccd Component Designer — pending Decomposer /
gateway-designer / operator confirmation; not in the original overview contract
graph.** Schema/example deferred.
