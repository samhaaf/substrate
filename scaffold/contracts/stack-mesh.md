# Contract: stack-mesh — RENAMED (tombstone)

**Status: RENAMED to `vdb-mesh` (2026-07-19, wave-2 harmonization).**
See `scaffold/contracts/vdb-mesh.md`.

Per the round-8 lock and vdb.md's naming resolution, **`vdb` is the crate and
the module; "stack" survives as the PATTERN name only**. The edge itself is
unchanged: vdb's registration/catalog/locks participation through the local
mesh daemon on `:3649` (single-port locality). One scope correction recorded
in the new file: the old "distributed-handler coordination" framing is
REDUCED per vdb.md concern 6. Operator sign-off on the rename is pending
(friction report); if declined, the rename reverts mechanically.
