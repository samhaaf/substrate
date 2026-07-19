# Contract: stack-mesh

## Parties
stack  <->  mesh

## What the edge carries
Stack's mesh participation, all through the **local mesh daemon on `:3649`**
(single-port locality, rounds 4–5): service registration/resolution (an
instance of `service-lookup`) and distributed-handler coordination via mesh's
internal `locks` lib (so a trigger fires once across the mesh). The shared
handler/execution engine itself is an internal library, NOT part of this
contract. Schema/example deferred. **requirements-only** (rounds 4–5 lock,
2026-07-18).
