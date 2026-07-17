# Contract: kv-cache

## Parties
engine  <->  cache

## What the edge carries
KV/prefix cache slot save/restore keyed by (model_id, prompt_hash): write a slot's
KV state after prefill, restore on a matching prefix to skip prefill. Schema
deferred.
