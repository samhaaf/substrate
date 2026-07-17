# store

**Status:** existing (`lib/store`), kept as-is. **Nesting:** child of inference.

The SQLite system-of-record for one machine: completions, collections, models,
result blobs, benchmark runs, KV-cache metadata. All access serialized through a
`Mutex<Connection>`; schema embedded at compile time; append-only migrations. It
is the intra-node hub — nearly every sibling reads/writes through it via the
`store-access` edge, and it carries an observer seam for event emission. Results
here durably outlive on-disk weights, which is what makes per-machine GC safe.
