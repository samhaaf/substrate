# Contract: stack-vfs — RENAMED (tombstone)

**Status: RENAMED to `vdb-vfs` (2026-07-19, wave-2 harmonization).**
See `scaffold/contracts/vdb-vfs.md`.

Per the round-8 lock and vdb.md's naming resolution, **`vdb` is the crate and
the module; "stack" survives as the PATTERN name only** (the operator's
database-centric tables+handlers paradigm). The edge itself is unchanged: the
single SQLite file each vdb-managed database wraps is stored in the VFS as a
NodeAnchored file, vdb the exclusive owner-writer. The rounds-4–5
requirements captured here are carried into `vdb-vfs.md` and
`components/vdb.md`. Operator sign-off on the rename is pending (friction
report); if declined, the rename reverts mechanically — content is identical.
