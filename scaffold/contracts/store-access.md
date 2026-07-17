# Contract: store-access

## Parties
store  <->  {engine, scheduler, models, cache, telemetry, benchmark, api}

## What the edge carries
The SQLite system-of-record CRUD + observer seam: completions, collections, models,
result blobs, benchmark runs, kv_cache metadata, and the queue view. The intra-node
hub edge. Schema deferred.
