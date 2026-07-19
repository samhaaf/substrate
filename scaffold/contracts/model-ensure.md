# Contract: model-ensure

## Parties
scheduler  ->  models
(initiator: **scheduler** — needs a weight present before a swap; owner/provider:
**models** — `models.md` authors the `ModelEnsure` trait and its download
orchestration)

Both are compiled-in libraries of one `inference` daemon (INTENT #22/#29). This is an
**in-process trait seam, NOT a wire contract** — though its `request_download` /
`download_status` methods are what `api` lowers `POST /v1/models/:id/download` and
`GET /v1/models/:id` onto (via `api-dispatch`).

## Purpose
Before `execute_model_swap` loads a target model, the scheduler ensures its GGUF
weights are present on local disk; and — the wave-2 delta — this is also the honest
replacement for the `download_model` REST stub (which returned `202` and did nothing).
Both the blocking swap path (scheduler) and the async REST path (`api`) resolve
through **one singleflight download job** (models concern 2) — no double-download of
a 40 GB weight, ever. The edge carries: blocking ensure-available, async
download-kickoff, live status, and eviction.

## Schema

The `ModelEnsure` trait, implemented by `models`' `ModelManager`. Types reference
`types::{model, error}`.

```rust
trait ModelEnsure {
    /// Blocking: await the model present on disk; returns its absolute path.
    /// Joins the in-flight job if one exists (singleflight — no double-download).
    /// This is the pre-swap call the scheduler makes in execute_model_swap.
    async fn ensure_available(&self, id: &ModelId) -> Result<String>;

    /// Async: kick off (or attach to) a download; return immediately.
    /// Backs POST /v1/models/:id/download (now a real 202).
    async fn request_download(&self, id: &ModelId) -> Result<DownloadHandle>;

    /// Poll live status (in-memory job progress + store row). Never blocks.
    fn download_status(&self, id: &ModelId) -> Result<DownloadStatus>;

    /// Drop a downloaded weight (unlock + gc evict); no-op if resident.
    async fn evict(&self, id: &ModelId) -> Result<()>;
}

struct DownloadHandle { id: ModelId, download_id: Uuid }

enum DownloadStatus {
    Absent,
    Downloading { bytes_done: u64, total: Option<u64>, pct: Option<f32>, source: SourceKind },
    Ready       { path: String, bytes: u64 },
    Failed      { message: String },
}
enum SourceKind { Origin /* hf:/https:/file: */, Peer /* vfs pull — deferred (models-vfs) */ }
```

### How the scheduler uses it (scheduler concern; models concern 2)

In `execute_model_swap`, before `engine.swap_model` (see `engine-exec`):
```rust
match models.ensure_available(&target).await {                 // or request_download + poll
    Ok(path)                          => engine.swap_model(target, Path::new(&path)).await?,
    // download still running: leave the triggering completions Pending, notify_new_work
    // when it finishes; the swap simply retries a later tick — the scheduler NEVER
    // blocks its tick on a multi-GB download.
    Err(ModelError::InProgress)       => { /* stay Pending; retry next tick */ }
    Err(ModelError::ModelNotFound)    => { /* fail the pending completions — they can never run */ }
    Err(ModelError::InsufficientDiskBudget) => { /* defer swap / pick smaller model */ }
}
```
Progress is **in-memory, not persisted** (models concern 2) — a per-chunk percentage
must not hammer the store's single-writer mutex. Only terminal transitions
(downloaded/failed/evicted) hit the store (`set_model_downloaded`/`clear_model_file`).
`ModelStatus::Downloading` is *derived* when an in-flight job exists.

## Error cases

All catchable (`ModelError` / the existing `DownloadFailed` taxonomy), none panic:

| Error | Cause | Scheduler handling |
|---|---|---|
| `ModelNotFound` | id not in the node's registry | fail the pending completions for that model (they can never run) → `404` at the REST edge |
| `DownloadFailed { model, message }` | network/HTTP failure; `.partial` retained for resume | surfaced as `Downloading` retryable, not a completion failure |
| `IntegrityMismatch { model }` | sha256/size check failed (models concern 6); corrupt file deleted | fail the completions; do not retain a corrupt weight |
| `InsufficientDiskBudget` | gc `DiskBudgetExceeded` after `make_room` exhausted unlocked candidates | scheduler must defer the swap or downshift to a smaller model — never a silent stall |
| `SourceUnsupported { scheme }` | unknown `hf:`/`https:`/`file:` scheme | fail the completions |

Key invariant: a swap target that is not yet downloaded does **NOT** fail the
completion outright *while a download is in progress* — it stays `Pending` and the
swap succeeds once `ensure_available` returns `Ready`.

## Version sensitivity

**LOW** — in-process, compiled in; no wire skew on this seam. The REST lowering
(`POST /v1/models/:id/download`, `GET /v1/models/:id`) inherits `v1-completion-api`'s
byte-transparent discipline (the router never parses the body). `DownloadStatus` /
`SourceKind` reserve additive / `#[serde(other)]` room for the day the deferred
`Peer` source (peer-pull via `models-vfs`, models concern 6) turns on — additive-safe.

## Reconciliation notes

- **Surface shape: scheduler's sketch vs models' authored trait (owner wins).**
  scheduler.md sketched a consumer-side `trait ModelEnsure { ensure_downloaded(id)
  -> EnsureState; download_status(id) -> DownloadProgress }` with `enum EnsureState {
  Ready{path}, InProgress{pct}, NotFound }`. models.md (the pair OWNER per wave2-plan
  §3a) authored the fuller `trait ModelEnsure { ensure_available; request_download;
  download_status; evict }` with the richer `DownloadStatus` (Absent / Downloading /
  Ready / Failed) and `DownloadHandle`. **Resolution: models' authored surface wins**
  — it is the owning side and strictly more complete (it distinguishes the blocking
  pre-swap path `ensure_available` from the async REST kickoff `request_download`,
  which the scheduler's single `ensure_downloaded` conflated; and it adds `evict` +
  `Failed`/`Absent` states the scheduler omitted). **The scheduler's SEMANTICS are
  fully preserved:** its `EnsureState::Ready{path}` == models' `Ready{path}`; its
  `InProgress` (leave completions Pending) == models' `Downloading` +
  `ModelError::InProgress`; its `NotFound` (fail the completions) == models'
  `ModelNotFound`. **Losing position recorded:** the scheduler's flatter
  `ensure_downloaded -> EnsureState` naming and 3-variant enum — superseded because
  it could not express the async-kickoff-vs-blocking-await split that the singleflight
  design (one job serving both callers) requires.
- **Singleflight is the correctness heart (both sides implicitly rely on it).** The
  scheduler assumes calling `ensure_available` while a REST-triggered download is
  running does not start a second transfer; models concern 2 guarantees this via the
  in-memory `DownloadRegistry`. Recorded so a filler does not implement two
  independent download paths.
- **`InsufficientDiskBudget` is a first-class scheduler branch, not a panic.** models
  maps gc's `DiskBudgetExceeded` to it; the scheduler must defer/downshift. Both files
  agree; adopted.

## Example data

Example world: node **pi** (limited disk), project **demo**, model **qwen3-4b** not
yet downloaded. A `demo` completion targeting `qwen3-4b` arrives; the scheduler needs
to swap.

```rust
// 1. scheduler, in execute_model_swap, calls ensure_available:
models.ensure_available(&ModelId("qwen3-4b".into())).await
// first tick — download just kicked off:
//   Err(ModelError::InProgress)  -> completions stay Pending, scheduler retries next tick

// meanwhile the REST/async view (api -> request_download at submit of a download action):
models.request_download(&ModelId("qwen3-4b".into())).await
//   Ok(DownloadHandle { id: "qwen3-4b", download_id: "b1e7-..." })  // joins the same job

models.download_status(&ModelId("qwen3-4b".into()))
//   Ok(Downloading { bytes_done: 1_610_612_736, total: Some(2_415_919_104),
//                    pct: Some(66.7), source: Origin })

// 2. later tick — download + sha256 verify complete:
models.ensure_available(&ModelId("qwen3-4b".into())).await
//   Ok("/home/pi/.substrate/models/qwen3-4b.gguf")
//   -> scheduler proceeds to engine.swap_model (see engine-exec example)

// eviction when pi needs room for another model and qwen3-4b is not resident:
models.evict(&ModelId("qwen3-4b".into())).await   // Ok(()) — gc unlock + reclaim
```

Failure branch: if pi's weights dir is full of *locked* (resident) weights,
`ensure_available` returns `Err(InsufficientDiskBudget)` and the scheduler defers the
swap rather than stalling.

## Conformance requirement

A filler's `models` + `scheduler` pass iff, against the example world: (a) a swap
target not yet downloaded does **not** fail the completion while a download is in
progress — it stays `Pending` and the swap succeeds once `ensure_available` returns
`Ready`; (b) a concurrent `ensure_available` (scheduler) and `request_download`
(REST) for `qwen3-4b` converge on **one** `tokio::spawn`ed transfer — assert exactly
one download task, both callers awaiting it; (c) `ModelNotFound` fails the pending
completions rather than looping; (d) a completed download whose sha256 mismatches is
deleted and reported `IntegrityMismatch`, and `is_downloaded` stays false.
