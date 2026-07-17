# types

**Status:** existing (`lib/types`), kept as-is. **Nesting:** top-level foundation.

The zero-dependency foundation crate: all shared IDs, enums, errors, the
`Result` alias, the `promise` mechanism, and the data structures every other
crate imports (`CompletionRequest`/`Result`, `CollectionOptions`, `ModelId`/
`ModelConfig`, `Estimate`, `StreamEvent`, `SystemState`/`NodeId`/`NodeInfo`).
It is effectively the pre-existing shared-contract-types root: the Contract
Harmonizer step will reconcile authored contracts against this crate rather than
generating a greenfield one. No I/O, no async, no business logic — it is the
anchor of every contract edge in the graph.
