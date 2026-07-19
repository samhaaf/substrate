# Contract: gc-managed-dirs

## Parties

`{models, cache, engine}` (internal libs of `inference`)  →  `gc`, held as a
`GcHandle` (`Embedded(Arc<GcService>)` standalone/tests, or `Remote(GcClient)`
under mesh). Sole authoritative proposer: `gc.md` (the `GcHandle`/`GcApi` seam).
This is an **in-process / node-local library edge**, not a cross-node wire.

## Purpose

In-process disk-budget enforcement for `inference`'s local storage: model
weights (`models`), llama-server backend builds (`engine`), and the KV/prefix
cache (`cache`). The consumers hold **one method surface** regardless of how gc is
reached — the `GcHandle` abstraction is the single migration seam that (a)
converges every consumer on the node's **one** centralized gc store under mesh,
and (b) preserves byte-for-byte standalone operation when no mesh is present (a
dev box, a test, a cold `pi` before gc boots). The canonical caller flow is
`make_room(budget)` → write → `register_and_lock(ttl)`.

## Schema

The `GcApi` trait is implemented by both `GcHandle` variants; it exposes the
**same** `GcCommand`/`GcQuery` vocabulary and the **same** `GcEvent` stream that
`vfs-gc` and `gc-events` carry (one shared set in `types`, authored once).

```rust
enum GcHandle {
    Embedded(Arc<GcService>),   // in-process fn calls — standalone/tests, today's behaviour
    Remote(GcClient),           // WS to the local gc daemon via mesh-client — the normal mesh world
}

trait GcApi {  // both variants implement this identical surface
    async fn register_dir(&self, path: &Path, policy: Option<GcDirPolicy>) -> Result<()>;
    async fn register_entry(&self, path: &Path, kind: EntryKind, hint: Option<String>) -> Result<()>;
    async fn register_and_lock(&self, path: &Path, kind: EntryKind, ttl: u64, hint: Option<String>) -> Result<()>;
    async fn touch(&self, path: &str) -> Result<()>;
    async fn lock(&self, path: &str, ttl_secs: u64) -> Result<()>;
    async fn unlock(&self, path: &str) -> Result<()>;
    async fn make_room(&self, dir: &str, bytes_needed: u64) -> Result<u64>;   // -> bytes_freed
    async fn move_path(&self, from: &str, to: &str) -> Result<()>;
    async fn evict(&self, path: &str) -> Result<()>;
    async fn get_entry(&self, path: &str) -> Result<Option<EntryRow>>;
    async fn list_entries(&self, dir: Option<&str>) -> Result<Vec<EntryRow>>;
    async fn list_dirs(&self) -> Result<Vec<DirRow>>;
    async fn dir_used_bytes(&self, dir: &str) -> Result<u64>;
}
// GcDirPolicy / EntryKind / GcEvent are the shared vocabulary (see vfs-gc.md / gc-events.md)
```

**Mode selection** happens **once**, at `InferenceService::start`: if the local
gc daemon is resolvable via mesh, use `Remote`; else `Embedded` (with a
"this node is unmanaged-by-mesh" warning). **No call site in
`models`/`cache`/`engine` changes** beyond the handle's field type. In `Remote`
mode the daemon does the `register_entry` path-walk to compute size (it is on the
same physical machine — single-port locality), so the call ships only
`{path, kind, recovery_hint}`.

## Error cases

Shared gc taxonomy (see `vfs-gc.md`): `DirNotRegistered`, `EntryNotFound`,
`DiskBudgetExceeded` (the catchable one — `models` must handle "can't free
enough" when ensuring a model download fits), `EntryLocked`, `NonUtf8Path`,
`ReclaimFailed{path,err}` (per-entry, non-fatal). A caller that forgets the
trailing `lock`/`register_and_lock` leaves the narrow register→sweep race
(`gc.md` concern 6) — `register_and_lock` is the preferred one-shot close.

## Version sensitivity

- **`Embedded` mode: N/A** — compiled in; the trait is a normal Rust API, versioned
  with the `inference` crate, no wire.
- **`Remote` mode:** inherits the shared `GcCommand`/`GcEvent` version rules
  (additive-only; unknown enum variants rejected, never coerced). The
  `GcHandle::Remote` path is filled **after** the harmonizer freezes the shared
  gc command/event structs in `types` and the `pubsub-protocol` envelope (the
  `GcClient` serializes those) — `gc.md`'s stated sequencing constraint.

## Reconciliation notes

- **Single proposer** (`gc.md`); no competing shape, no losing position.
- **Round-3 delta captured:** consumers no longer construct a private
  `Arc<GcService>` over their own `gc.db`; they hold a `GcHandle` and converge on
  the **one** per-node store. The historic two-`gc.db` split-brain (embedded
  `GcService` over `<inference>/gc.db` *plus* the `bin/gc` daemon over
  `~/.substrate/gc.db`) is resolved by construction: `Remote` lands every command
  on the single daemon; mesh's zombie-killing guarantees one daemon = one writer.
- **Deviation from the stub:** the stub said only "register dirs/entries,
  touch/lock, sweep; schema deferred." This full contract pins the `GcHandle`
  seam, the identical-across-transports command set, and the standalone-preserving
  `Embedded` fallback — the wave-2 headline for this edge.
- **Physical store convergence with vfs's gc dirs** (one literal store on a node
  that runs both inference and vfs) is a batch-5 gc/inference concern, out of
  scope here; this edge only requires that inference's consumers share the *one*
  daemon-owned store under mesh.

## Example data

macbook runs `inference` under mesh, so its consumers hold `GcHandle::Remote`;
they manage the qwen3-4b weight and its KV cache against the node's one store.

```jsonc
// models ensures qwen3-4b.gguf fits, then registers+locks it in one shot:
gc.make_room("/Users/op/.substrate/inference/models", 2576980377) -> Ok(3221225472)
gc.register_and_lock("/Users/op/.substrate/inference/models/qwen3-4b.gguf",
                     EntryKind::File, /*ttl*/ 86400, Some("model:qwen3-4b")) -> Ok(())

// cache registers a KV/prefix cache entry keyed by (model, prompt_hash); LRU-accessed:
gc.register_entry("/Users/op/.substrate/inference/cache/qwen3-4b/9f2c…a1.kv",
                  EntryKind::File, Some("kvcache:qwen3-4b")) -> Ok(())
gc.touch("/Users/op/.substrate/inference/cache/qwen3-4b/9f2c…a1.kv") -> Ok(())

// budget pressure on the cache dir; make_room reclaims LRU KV entries:
gc.make_room("/Users/op/.substrate/inference/cache", 1073741824) -> Ok(1181116006)

// same code path on a mesh-less dev box uses GcHandle::Embedded — identical results,
// no daemon, no wire (byte-for-byte the pre-wave-2 behaviour).
```
