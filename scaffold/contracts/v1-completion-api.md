# Contract: v1-completion-api

## Parties
client / mesh.completion-router  <->  inference-node (api)

## What the edge carries
The `/v1/` REST+WS completion surface: submit/status/cancel/priority/result/stream
for completions, plus collections, models, and estimate. Mesh forwards this
transparently (client cannot tell node-direct from mesh-routed). Shaped-for note:
Org (maximalist) will consume completions/results broadly — keep this surface the
single completions entry point. Full schema + example data deferred to step 3.
