# Contract: llm-calls

## Parties
ccd (agents)  ->  inference (api, via v1-completion-api)

## What the edge carries
CCD/agents make LLM calls through the local inference node's `/v1/` API. Reuses
`v1-completion-api` rather than a second surface. Shaped-for note: this is also
how Org's agents will get model calls, at higher volume — keep completions cheap to
fan out. Schema deferred.
