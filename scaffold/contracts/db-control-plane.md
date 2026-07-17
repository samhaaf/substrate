# Contract: db-control-plane

## Parties
db  <->  consumers (Org / game-demo; operator-driven CLI)

## What the edge carries
The noun-verb DB control plane: migration authoring/apply/rollback, edge-function
deploy/activate, ad-hoc query, seed/fake-data, outbox + audit, over Postgres/SQLite
drivers. The "DB thing in a local stack" for the Org/game demo. Shaped-for note: Org
will want programmatic (not just CLI) access to a stable subset. Schema deferred.
