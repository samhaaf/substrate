# Contract: db-inference-init

## Parties
inference  ->  db

## What the edge carries
When an `inference` node stands up on a new mesh node, it initializes its own
database through `db`'s control plane rather than handling that bootstrap
itself (schema/migration apply for the node's local store setup, via `db`'s
existing local-stack/migration machinery). New edge, added this round per
operator decision — not previously in the contract graph. Schema deferred.
