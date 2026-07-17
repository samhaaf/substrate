# Contract: gc-managed-dirs

## Parties
{models, cache, engine}  ->  gc (embedded library)

## What the edge carries
In-process disk-budget enforcement: register managed dirs/entries, touch/lock (LRU),
and sweep (TTL + size budget). Distinct from the GC daemon's HTTP `gc-events` edge.
Schema deferred.
