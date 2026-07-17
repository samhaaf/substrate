# Substrate API Surface

This document describes the **current, real** external API surface of the Substrate
services, as implemented in this branch (`V1`) today. It is written for someone building
another tool or app that wants to talk to a running Substrate stack — in particular, tools
that want continuous/passive access to completions being run on this machine.

It documents what exists in the source right now. A separate redesign effort (a
"scaffolding" pass plus a new mesh/Tailscale crate) is expected to change some of these
contracts later — treat this document as a snapshot, not a stability promise.

## What Substrate is

Substrate is a personal LLM-serving runtime. **Completion is the only unit of work** — you
submit a prompt + generation parameters as a "completion," it runs on whichever model is
currently resident (one model loaded at a time per machine), and you retrieve the result by
polling or by a lifecycle event feed. There are no DAGs, tool-calling loops, or agents baked
into the runtime itself — that belongs in layers above this API.

The system runs as three cooperating HTTP/WebSocket services on one machine:

| Service                 | Port   | Role                                                         |
|--------------------------|--------|--------------------------------------------------------------|
| `substrate-gateway`      | `:8400`| **The front door.** REST proxy + aggregated stats + pub/sub WS hub + dashboard static files. |
| `substrate-inference`    | `:8420`| Completions, collections, models, benchmark, system state. Internal — not meant for direct external use. |
| `substrate-gc`           | `:8430`| Disk-budget garbage collection for model weights / KV cache. Internal — not meant for direct external use. |

**External tools should talk to the gateway on `:8400`, not directly to `:8420` or `:8430`.**
The gateway proxies REST calls through to the two backend services under
`/api/inference/*` and `/api/gc/*`, and provides its own aggregated endpoints and a
WebSocket pub/sub hub. Direct access to `:8420`/`:8430` works today (nothing firewalls it),
but per this project's own convention it is considered internal plumbing, not a supported
external contract — those ports and their internal `/v1/...` routes are not proxied
1:1 and may change shape without notice. Everything documented under "Completions",
"Collections", "System / Stats", "Benchmark", and "GC" below is reached **through the
gateway's proxy prefixes**; the underlying `:8420`/`:8430` paths are included alongside as
context (since the gateway forwards them nearly verbatim), not as a separately-supported
surface.

The gateway sets `Access-Control-Allow-Origin: *` (permissive CORS), so it can be called
directly from browser-based tools as well as backend services.

All request/response examples below use real field names pulled from
`lib/types/src/*.rs`, not placeholders.

### ⚠️ The gateway proxy drops query strings — verified live, affects both proxy prefixes

Before anything else: `bin/gateway/src/proxy.rs`'s `proxy_inference` and `proxy_gc` handlers
build the upstream URL purely from `{base_url}` + the wildcard path segment — they never look
at the incoming request's query string, and never re-attach one to the outbound request. This
was verified by actually running the live stack on this machine and comparing:

```
$ curl "http://127.0.0.1:8430/entries?dir=/tmp/test-data/dir1"        # direct :8430 — correctly scoped
[{"path":"/tmp/test-data/dir1/a.txt", ...}]                            # 1 entry

$ curl "http://127.0.0.1:8400/api/gc/entries?dir=/tmp/test-data/dir1"  # via gateway :8400 — filter silently dropped
[... 6 entries from every managed directory on the machine ...]

$ curl "http://127.0.0.1:8400/api/gc/entries/get?path=/tmp/test-data/dir1/a.txt"  # required param, not optional
Failed to deserialize query string: missing field `path`               # HTTP 400 — hard failure, not silent
```

**Practical impact:** any endpoint that relies on a query parameter — `GET
/api/inference/v1/completions?state=...&limit=...`, `GET /api/gc/entries?dir=...`, `GET
/api/gc/entries/get?path=...` — either silently ignores your filter (falls back to
defaults/no-filter) or, when the parameter is required rather than optional, fails outright
with a 400 from the upstream service's own query-deserialization step. This is **not**
documented anywhere in the code (no comment in `proxy.rs` acknowledges it) and is easy to miss
in testing if you only ever call unfiltered endpoints. Each affected endpoint below repeats
this caveat inline. Until it's fixed, if you need query-parameter filtering, call `:8420`/
`:8430` directly for that specific request.

---

## 1. Completions

Base path through the gateway: **`/api/inference/v1/completions...`**
(the gateway's proxy prepends `/v1/` automatically to anything under `/api/inference/`
that isn't already `health`, `metrics`, or `v1/...` — so `/api/inference/completions` and
`/api/inference/v1/completions` both work).

Underlying inference-service routes (defined in `lib/api/src/rest.rs`):

| Method | Path                                   | Purpose                          |
|--------|-----------------------------------------|-----------------------------------|
| GET    | `/v1/completions`                       | List completions                 |
| POST   | `/v1/completions`                       | Submit a new completion           |
| GET    | `/v1/completions/:id`                   | Get completion status             |
| DELETE | `/v1/completions/:id`                   | Cancel a completion               |
| PATCH  | `/v1/completions/:id/priority`          | Update scheduling priority        |
| GET    | `/v1/completions/:id/result`            | Fetch the terminal result         |

### Submit a completion — asynchronous, returns an id you then poll

```
POST /api/inference/v1/completions
Content-Type: application/json
```

Request body is a `CompletionRequest` (`lib/types/src/completion.rs`). You may omit `id`
and `created_at` — the server assigns both if left as the zero UUID / default timestamp:

```json
{
  "id": "00000000-0000-0000-0000-000000000000",
  "model_id": "qwen3.6-30b-a3b-q4",
  "prompt": "Explain garbage collection in one paragraph.",
  "max_tokens": 256,
  "temperature": 0.7,
  "top_p": null,
  "top_k": null,
  "repeat_penalty": null,
  "stop": null,
  "json_schema": null,
  "priority": 10,
  "preemption_threshold": null,
  "collection_id": null,
  "metrics": 3,
  "metadata": null,
  "created_at": "1970-01-01T00:00:00Z"
}
```

Notes on fields:
- `priority`: higher runs first; the scheduler orders by `(priority DESC, created_at ASC)`.
  Priority `0` is reserved for benchmark/background sweeps. Normal requests should use `10`.
- `preemption_threshold`: if omitted, defaults to `priority + 2` — a higher-priority request
  on a *different* model above this threshold will preempt this one back to pending.
- `metrics`: a `MetricsFlags` bitmask (`lib/types/src/completion.rs`), serialized as a plain
  integer (it's a newtype tuple struct, so serde emits just the `u32`, not an object) — `1` =
  token counts, `2` = timing, both ORed = `3`, `0` = none. Only fields covered by the flags
  you set will be populated in the eventual result.
- Substrate does **not** normalize sampling parameters — they pass through to the underlying
  llama.cpp server close to verbatim.

Response — `201 Created`:

```json
{ "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6" }
```

Errors: `422` (`InvalidRequest`) for malformed bodies; `500` for anything else.

### List completions

```
GET /api/inference/v1/completions?state=pending,running&limit=50
```

Query params: `state` (comma-separated: `pending`, `running`, `completed`, `failed`,
`cancelled` — omit for all), `limit` (default 50, effectively uncapped by the server beyond
that default — pass an explicit value for large listings).

**⚠️ Verified live bug: query strings do not survive the gateway proxy.** See "The gateway
proxy drops query strings" below — `?state=...&limit=...` is silently ignored when called
through `:8400`; you always get the unfiltered default (`limit=50`, all states). If you need
actual filtering, call `:8420` directly for this one call.

Response — `200 OK`, an array of completion summaries (not the full `CompletionRow`; a
hand-built JSON projection in `rest.rs`):

```json
[
  {
    "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
    "model_id": "qwen3.6-30b-a3b-q4",
    "state": "running",
    "priority": 10,
    "preemption_threshold": 12,
    "preemption_count": 0,
    "error_retry_count": 0,
    "collection_id": null,
    "created_at": "2026-07-17T18:02:00Z",
    "started_at": "2026-07-17T18:02:01Z",
    "completed_at": null
  }
]
```

### Get completion status

```
GET /api/inference/v1/completions/:id
```

Same JSON shape as one element of the list above. `404` (`CompletionNotFound`) if the id
doesn't exist.

### Get the result — only after the completion is terminal

```
GET /api/inference/v1/completions/:id/result
```

- `409 Conflict` if the completion has not yet reached a terminal state (`completed`,
  `failed`, `cancelled`) — **this is a synchronous check, not a blocking wait.** Callers must
  poll `GET /completions/:id` (or watch the WS lifecycle feed — see §6) until the state is
  terminal, then fetch the result.
- `404` if terminal but no result blob was stored (e.g. cancelled before producing output).
- `200 OK` with a `CompletionResult` (`lib/types/src/completion.rs`) on success:

```json
{
  "id": "3fa85f64-5717-4562-b3fc-2c963f66afa6",
  "state": "completed",
  "text": "Garbage collection is the process by which...",
  "termination": { "completed": "eos" },
  "metrics": {
    "queue_latency_ms": 12,
    "generation_ms": 4310,
    "prompt_tokens": 18,
    "completion_tokens": 256,
    "tokens_per_second": 59.4,
    "preemption_count": 0,
    "error_retry_count": 0
  },
  "completed_at": "2026-07-17T18:02:05Z"
}
```

`termination` is an externally-tagged `TerminationReason` enum (no `#[serde(tag=...)]`
attribute on this type, unlike `StreamEvent` below — so it serializes as
`{ "<variant>": <payload> }` for newtype variants, or a bare string for the unit variant):
`{"completed": "eos"|"stop"|"length"}` (a `StopReason`), `{"preempted": <PreemptionReason>}`,
`{"failed": <ErrorKind>}`, or the bare string `"cancelled"`.

### Cancel a completion

```
DELETE /api/inference/v1/completions/:id
```

`204 No Content` on success. `409 Conflict` if already terminal. `404` if not found.

### Update priority in place

```
PATCH /api/inference/v1/completions/:id/priority
Content-Type: application/json

{ "priority": 20 }
```

`204 No Content` on success. `409` if the completion is already terminal.

### Streaming a completion — documented feature that does NOT currently work as an external tool would expect

The inference service exposes `GET /v1/completions/:id/stream` as a WebSocket
(`lib/api/src/ws.rs`) that upgrades and then:

1. Polls the store every 100ms for state changes (not a real push/broadcast).
2. Sends a `Heartbeat` event every 5s.
3. Sends a `Started` event when the completion transitions to `running`.
4. Sends exactly one terminal event (`Completed` / `Failed` / `Cancelled`) and closes.

**Two important gaps, verified directly in source, not assumed from prior docs:**

- **No per-token streaming exists yet.** The doc comment at the top of `lib/api/src/ws.rs`
  states plainly: "The engine does not yet expose a broadcast channel for token events... Until
  the broadcast seam is in place, clients receive lifecycle events... but not individual Token
  events." The `StreamEvent::Token { id, text, token_count }` variant exists in
  `lib/types/src/stream.rs` and is defined in the wire protocol, but nothing in the engine ever
  constructs or sends one today. Token text is not persisted to the store at all outside of the
  final result's `text` field.
- **This endpoint is not reachable through the gateway.** The gateway's `/api/inference/*`
  proxy (`bin/gateway/src/proxy.rs`) is a plain buffered HTTP request/response forwarder built
  on `reqwest` — it reads the full request body, sends a normal HTTP request upstream, and
  returns the full response body. It has no WebSocket-upgrade handling. A WS handshake sent to
  `/api/inference/v1/completions/:id/stream` will not be relayed as a working WebSocket
  connection. **If you need this endpoint at all today, you must bypass the gateway and hit
  `:8420` directly** — which the project's own convention treats as internal, unsupported
  surface for external tools. In practice: **there is currently no supported way for an
  external tool to get a live per-completion event/token stream through the gateway.** Poll
  `GET /api/inference/v1/completions/:id` (§ above) for state changes instead.

**Recommended pattern for "continuous/passive access to completions" today:** poll
`GET /api/inference/v1/completions?state=running,pending` and/or
`GET /api/inference/v1/completions/:id` on an interval, and separately subscribe to the
gateway's own `/events` WebSocket hub (§6) for lifecycle-level signals (model swaps, queue
depth, aggregate throughput samples) — not per-completion token content.

---

## 2. Collections

A collection is a named batch of completions sharing a lifecycle (used by the benchmark
orchestrator, and available for any caller who wants "submit N completions, wait for all").
Base path: **`/api/inference/v1/collections...`**.

| Method | Path                        | Purpose                              |
|--------|------------------------------|---------------------------------------|
| POST   | `/v1/collections`            | Create a collection                    |
| GET    | `/v1/collections/:id`        | Get collection state + member counts   |
| DELETE | `/v1/collections/:id`        | Cancel a collection and its members    |

### Create a collection

```
POST /api/inference/v1/collections
Content-Type: application/json
```

Body is `CollectionOptions` (`lib/types/src/collection.rs`):

```json
{
  "name": "my-batch-2026-07-17",
  "description": "nightly regression prompts",
  "cancel_on_failure": false,
  "request_full_system": false,
  "start_with_no_model_loaded": false,
  "save_partial_results": true,
  "metadata": null
}
```

Note (v2 semantic change, not a bug): scheduling fields like priority now live on each
individual `CompletionRequest.collection_id` member, not on the collection itself — the
collection record is purely about lifecycle grouping.

Response — `201 Created`: `{ "id": "<collection-uuid>" }`. Then submit completions normally
via `POST /v1/completions` with `"collection_id"` set to that id.

### Get collection status

```
GET /api/inference/v1/collections/:id
```

```json
{
  "id": "6ad56a2e-...",
  "name": "my-batch-2026-07-17",
  "description": "nightly regression prompts",
  "state": "active",
  "cancel_on_failure": false,
  "request_full_system": false,
  "start_with_no_model_loaded": false,
  "save_partial_results": true,
  "created_at": "2026-07-17T18:00:00Z",
  "started_at": "2026-07-17T18:00:01Z",
  "completed_at": null,
  "member_counts": {
    "total": 20,
    "completed": 12,
    "failed": 1,
    "cancelled": 0,
    "active": 7
  }
}
```

`state` is one of `active`, `completed`, `failed`, `cancelled`. There is no dedicated
"result of a collection" endpoint — poll member completions individually via the completions
list filtered by `collection_id` is not currently exposed as a query filter either (the
`/v1/completions` list endpoint only filters by `state`), so in practice you track membership
client-side by the ids you submitted, or poll `GET /v1/collections/:id` for aggregate counts.

### Cancel a collection

```
DELETE /api/inference/v1/collections/:id
```

Cancels the collection and all non-terminal members. `204` on success, `409` if already
terminal, `404` if not found.

---

## 3. Models

Base path: `/api/inference/v1/models...`.

| Method | Path                          | Purpose                     |
|--------|-------------------------------|-------------------------------|
| GET    | `/v1/models`                  | List registered models         |
| POST   | `/v1/models/:id/download`     | Trigger a model download (stub)|

```
GET /api/inference/v1/models
```

```json
[
  {
    "id": "qwen3.6-30b-a3b-q4",
    "source": "hf:owner/repo/model.gguf",
    "context_length": 8192,
    "status": "Loaded",
    "is_downloaded": true,
    "is_loaded": true,
    "file_bytes": 18253611008,
    "file_path": "/Users/you/.substrate/models/qwen3.6-30b-a3b-q4.gguf",
    "n_gpu_layers": -1,
    "max_slots": null,
    "last_used_at": "2026-07-17T18:02:05Z",
    "registered_at": "2026-06-26T00:00:00Z"
  }
]
```

`status` is one of `Absent`, `Downloading`, `Ready`, `Loaded`, `Unloading`
(`ModelStatus` in `lib/types/src/model.rs`, derived from the boolean flags, `Debug`-formatted
— note the capitalized values, unlike the snake_case state strings used elsewhere in this API).

**`POST /v1/models/:id/download` is a stub**, per its own doc comment in `rest.rs`: it
acknowledges the request and returns the model's current status, but does not actually kick
off a download (the model manager isn't wired into `ApiState` yet). Returns `202 Accepted`
regardless:

```json
{ "id": "qwen3.6-30b-a3b-q4", "status": "Absent", "is_downloaded": false, "message": "download request accepted" }
```

---

## 4. System / Stats / Estimation / Execution control

### System state (inference-level)

```
GET /api/inference/v1/system/state
```
(also aliased at `/api/inference/metrics` for Prometheus-style scraping, and at the
inference service's bare `/health` for liveness — both reachable through the gateway proxy's
special-cased `health`/`metrics` path handling.)

Returns a `SystemState` (`lib/types/src/system.rs`) — synchronous, point-in-time snapshot:

```json
{
  "sampled_at": "2026-07-17T18:05:00Z",
  "total_memory_bytes": 137438953472,
  "used_memory_bytes": 98784247808,
  "memory_pressure": 0.719,
  "cpu_utilization": 0.34,
  "gpu_memory_used_bytes": 0,
  "gpu_memory_total_bytes": 0,
  "gpu_utilization": 0.61,
  "is_gpu_estimate": true,
  "resident_model": "qwen3.6-30b-a3b-q4",
  "running_count": 1,
  "pending_count": 3,
  "weights_on_disk_bytes": 54761832448,
  "kv_cache_bytes": 1073741824
}
```

`gpu_memory_used_bytes`/`gpu_memory_total_bytes` are always `0` — GPU memory sampling isn't
implemented; only `gpu_utilization` (fraction) is real (macOS, via `ioreg`), flagged by
`is_gpu_estimate`.

### Aggregated node stats (gateway-native — not proxied, gateway computes this itself)

```
GET /api/nodes/local/stats
```

This endpoint is implemented directly in the gateway (`bin/gateway/src/stats.rs`), combining
three sources server-side on every call: OS-level disk stats (via `sysinfo`), GC-managed
bytes (fetched live from GC's `GET /dirs` and summed by actual `used_bytes`, deliberately not
the policy's budget ceiling, which can exceed real usage), and GPU info proxied from the
inference service's `/v1/system/state`.

```json
{
  "disk": {
    "total_bytes": 994662584320,
    "used_bytes": 612345678901,
    "free_bytes": 382316905419,
    "gc_managed_bytes": 54761832448
  },
  "gpu": {
    "utilization_fraction": 0.61,
    "is_estimate": true
  }
}
```

Degrades gracefully to zeros on any upstream fetch failure — it never errors out to the
caller.

### Throughput estimation

```
POST /api/inference/v1/estimate
Content-Type: application/json

{ "model_id": "qwen3.6-30b-a3b-q4", "prompt_tokens": 500, "max_tokens": 256, "concurrency": 1 }
```

Body is a `CompletionShape`. Response — synchronous, `200 OK`:

```json
{
  "shape": { "model_id": "qwen3.6-30b-a3b-q4", "prompt_tokens": 500, "max_tokens": 256, "concurrency": 1 },
  "tokens_per_second": 58.2,
  "estimated_ms": 4399,
  "cold_start": false,
  "confidence": "in_envelope"
}
```

`confidence` is `"in_envelope"` (interpolating within observed benchmark data) or
`"extrapolated"` (outside the observed range — treat with less trust). Note: per the
handler's own comment in `rest.rs`, this endpoint currently builds a **fresh** estimator per
call rather than reusing a pre-warmed one — functionally correct, but each call redoes the
regression fit from scratch.

### Execution pause/resume

```
POST /api/inference/v1/execution/pause
POST /api/inference/v1/execution/resume
```

Both return `{ "ok": true }`. Pausing stops new admission; in-flight completions keep
running.

---

## 5. Benchmark

Base path: `/api/inference/v1/benchmark...`.

| Method | Path                      | Purpose                                    |
|--------|----------------------------|----------------------------------------------|
| GET    | `/v1/benchmark/kernel`     | Per-model fitted throughput curve + raw points |
| POST   | `/v1/benchmark/run`        | Trigger (or skip, if already satisfied) a benchmark sweep |

### Kernel data

```
GET /api/inference/v1/benchmark/kernel
```

```json
{
  "models": [
    {
      "model_id": "qwen3.6-30b-a3b-q4",
      "model_name": "qwen3.6-30b-a3b-q4",
      "is_current": true,
      "x_axis": "output_tokens",
      "x_label": "Output tokens",
      "y_label": "tokens/sec",
      "coefficients": [61.2, -0.014, 0.0000009],
      "x_min_measured": 32,
      "x_max_measured": 1024,
      "data_points": [{ "x": 32, "y": 63.1 }, { "x": 1024, "y": 45.7 }]
    }
  ]
}
```

`coefficients` are `[a, b, c]` for the quadratic fit `tps = a + b·x + c·x²` over
`(output_tokens, tokens_per_second)` pairs at `concurrency == 1` only — this is a **1-D**
curve (single independent variable), not a full 2-D sweep display. Per the workspace
STATUS.md this single-axis rendering vs. a genuine 2-D sweep view is an open,
unconfirmed design question — do not assume `coefficients` captures concurrency effects.
Returns `{ "models": [] }` if no benchmark data exists yet for any model.

### Trigger a sweep — asynchronous, returns a run_id (a collection id) you poll

```
POST /api/inference/v1/benchmark/run
Content-Type: application/json

{ "model_id": "qwen3.6-30b-a3b-q4" }
```

`model_id` is optional — if omitted, uses whichever model is currently resident (`422` if
none is loaded and none was given).

Response — `202 Accepted` if a sweep was actually scheduled:

```json
{ "run_id": "b12e4b1a-..." }
```

or `200 OK` if every benchmark cell is already satisfied (no sweep needed):

```json
{ "run_id": null, "message": "all benchmark cells already satisfied; no sweep needed" }
```

`run_id` is a **collection id** — poll `GET /api/inference/v1/collections/:id` (§2) to track
sweep progress, the same as any other collection.

---

## 6. GC (garbage collector)

Base path through the gateway: **`/api/gc/...`** (the gateway forwards the path segment
after `/api/gc/` verbatim to the GC service — there is no `/v1/` prefix rewriting here, unlike
the inference proxy).

| Method | Path                  | Purpose                                        |
|--------|------------------------|--------------------------------------------------|
| POST   | `/dirs/register`       | Register a managed directory + optional policy    |
| POST   | `/dirs/deregister`     | Stop managing a directory                          |
| GET    | `/dirs`                | List managed directories (with live `used_bytes`)  |
| POST   | `/entries/register`    | Register a file/dir entry for tracking             |
| POST   | `/entries/touch`       | Mark an entry as recently used (LRU bump)          |
| POST   | `/entries/lock`        | Lock an entry against eviction for a TTL           |
| POST   | `/entries/unlock`      | Remove a lock                                       |
| GET    | `/entries/get?path=`   | Get one entry's record                              |
| GET    | `/entries?dir=`        | List entries (all entries if `dir` omitted/empty)  |
| POST   | `/ops/make-room`       | Evict enough entries in a dir to free N bytes       |
| POST   | `/ops/move`            | Move a managed path                                 |
| POST   | `/ops/evict`           | Force-evict one entry                               |
| POST   | `/ops/sweep`           | Run a full TTL + budget eviction sweep now          |

This subsystem manages disk budget for model weights and KV cache — most external tools
will only ever need the read-only listing endpoints (`/dirs`, `/entries`) to inspect what's
on disk, plus the aggregated `/api/nodes/local/stats` endpoint (§4) for a summary. The
mutating endpoints exist mainly for the inference service itself to call.

### List directories

```
GET /api/gc/dirs
```

```json
[
  {
    "root": "/Users/you/.substrate/models",
    "policy": {
      "max_size_bytes": 42949672960,
      "default_ttl_secs": 604800,
      "eviction": "Lru",
      "unit": "Children",
      "recursive": false,
      "on_full": "Evict"
    },
    "registered_at": 1751000000,
    "last_swept_at": 1752768000,
    "used_bytes": 54761832448
  }
]
```

`used_bytes` here is a **live sum** of `present` entries' `size_bytes` under this root — the
"actual bytes on disk right now" figure, not the `max_size_bytes` ceiling. (This is the fix
noted in the workspace STATUS.md: `used_bytes` is computed from real entries, not the budget.)
`registered_at`/`last_swept_at` are Unix seconds (`i64`), not RFC3339 strings — inconsistent
with the inference service's timestamp convention, worth knowing if you're parsing both.

### List entries

```
GET /api/gc/entries              # all entries across all managed dirs
GET /api/gc/entries?dir=/Users/you/.substrate/models   # scoped to one dir — BROKEN through the gateway, see below
```

**⚠️ Verified live: the `?dir=` filter is silently dropped through the gateway** — see "The
gateway proxy drops query strings" below. `GET /api/gc/entries` (no filter) works fine either
way, but a scoped query through `:8400` silently returns *all* entries across every managed
directory, not just the one you asked for.

```json
[
  {
    "path": "/Users/you/.substrate/models/qwen3.6-30b-a3b-q4.gguf",
    "dir_root": "/Users/you/.substrate/models",
    "kind": "file",
    "size_bytes": 18253611008,
    "registered_at": 1751000000,
    "last_touched_at": 1752767000,
    "touch_count": 42,
    "lock_expires_at": null,
    "ttl_override_secs": null,
    "recovery_hint": null,
    "state": "present"
  }
]
```

`state` is one of `"present"`, `"evicting"`, `"absent"` (plain strings, not a typed enum on
the wire). Note: `GET /entries` with `dir` omitted or empty now correctly returns **all**
entries (`list_all_entries`) rather than silently returning an empty list — this was a real
bug fixed earlier in this project's history; verified fixed as of this document by reading
`bin/gc/src/api.rs::list_entries` directly.

### Get one entry — hard-broken through the gateway

```
GET /api/gc/entries/get?path=/Users/you/.substrate/models/qwen3.6-30b-a3b-q4.gguf
```

**⚠️ Verified live: this call fails outright through the gateway.** `path` is a *required*
query parameter (`GetEntryQuery` has no `Option`/default), and since the gateway drops query
strings entirely (see below), the request that reaches the GC service has no `path` at all.
Axum's query extractor rejects it before the handler even runs:

```
$ curl "http://127.0.0.1:8400/api/gc/entries/get?path=/tmp/some/file"
Failed to deserialize query string: missing field `path`      (HTTP 400)
```

The same call against `:8430` directly works correctly and returns the entry. **Use
`GET /api/gc/entries` (unfiltered, no query params) and filter client-side, or call `:8430`
directly, until this is fixed.**

### Sweep now

```
POST /api/gc/ops/sweep
```

```json
{ "expired_evicted": 2, "budget_evicted": 0, "bytes_freed": 8192000000, "errors": [] }
```

**Known stub:** if a directory's policy is `"on_full": "Migrate"`, eviction does **not**
actually migrate the file elsewhere — `MigrateReclaimer` in `lib/gc/src/reclaim.rs` logs a
warning ("MigrateReclaimer not implemented, falling back to delete") and deletes it like the
default `Evict` policy. Don't rely on `Migrate` preserving data.

### GC health

```
GET /api/gc/health
```
`{ "status": "ok", "version": "0.1.0" }`

---

## 7. WebSocket events

There are **two independent WebSocket layers** — do not confuse them:

1. **`GET /events` on the gateway (`:8400`)** — the pub/sub hub external tools should use.
2. **`GET /events` on each backend service (`:8420`, `:8430` directly)** — internal upstream
   feeds that the gateway itself connects to and re-broadcasts through (1). Not meant to be
   consumed directly by external tools, though nothing prevents it today.

### Gateway hub: `ws://<host>:8400/events`

On connect, the gateway immediately sends a synthetic event:

```json
{ "topic": "lifecycle", "service": "gateway", "node_id": "local", "ts": 1752768000000, "event": { "type": "gateway.connected", "node_id": "local" } }
```

**Client → gateway** control messages (JSON text frames):

```json
{ "action": "subscribe",   "topics": ["lifecycle", "queue"] }
{ "action": "unsubscribe", "topics": ["queue"] }
```

Valid `Topic` values (`bin/gateway/src/topics.rs`, snake_case on the wire):
`"lifecycle"`, `"queue"`, `"gc"`, `"system"`, `{"completion": "<id>"}` (an object variant,
not a bare string — see caveat below), and `"all"`. **No subscription = no events delivered**
— you must send a `subscribe` message after connecting, even to get the initial lifecycle
feed. Subscribing to `"all"` receives everything regardless of topic.

**Gateway → client** event envelope (`GatewayEvent`):

```json
{
  "topic": "lifecycle",
  "service": "inference",
  "node_id": "local",
  "ts": 1752768000000,
  "event": { "type": "model_loaded", "model_id": "qwen3.6-30b-a3b-q4", "path": "/Users/you/.substrate/models/qwen3.6-30b-a3b-q4.gguf" }
}
```

- `service` is `"inference"`, `"gc"`, or `"gateway"` — which upstream produced the event.
- `node_id` is the gateway's configured logical node name (`"local"` by default,
  `etc/gateway.toml`).
- `ts` is Unix milliseconds.
- `event` is the raw upstream payload passed through as-is (a `LifecycleEvent` from
  `lib/types/src/stream.rs` for inference-sourced events, tagged with `"type"` in
  snake_case; a `GcEvent` from `lib/gc/src/events.rs` for GC-sourced events).

**Which lifecycle event types actually arrive:** the inference service's own `/events`
endpoint (`lib/api/src/ws.rs::lifecycle_events`) forwards model lifecycle
(`model_loading`/`model_loaded`/`model_unloading`/`model_unloaded`/`model_swapping`),
backend lifecycle (`backend_starting`/`backend_ready`/`backend_stopping`/`backend_stopped`/
`backend_installing`/`backend_installed`), queue state
(`queue_depth_changed`/`execution_paused`/`execution_resumed`), and one throughput-sample
exception, `completion_metrics_recorded` (aggregate tokens/sec for a just-finished
completion, not the completion's content). **Per-completion lifecycle events
(`completion_submitted`, `completion_started`, `completion_completed`, `completion_failed`,
`completion_cancelled`, `completion_preempted`) are explicitly filtered out** by
`is_external_event()` in `lib/api/src/ws.rs` and never leave the inference process over this
channel.

**`Topic::Completion(id)` is wired but currently dead — verified by reading the source, not
assumed from prior docs.** The gateway's `infer_topic()` (`bin/gateway/src/upstream.rs`) is
able to route an event to `Topic::Completion(id)` if the JSON payload has a top-level
`"completion_id"` string field and a `"type"` of `"token"`, `"completed"`, or `"failed"`. But
no event the inference service actually emits over `/events` has a `completion_id` field —
`CompletionMetricsRecorded`'s id field is literally named `id`, not `completion_id`, and (as
above) all other completion-scoped events are filtered out before they'd ever reach this
router. **The practical result: subscribing to `{"completion": "<id>"}` on the gateway hub
today will never deliver anything.** This matches (and sharpens) the workspace STATUS.md's
note that this feature is "wired but unused" — it is not just unexercised, the current event
producers don't emit the field the router needs to make it work at all.

**Reconnection:** the gateway maintains its own reconnecting client to each upstream `/events`
(exponential backoff, 1s → 30s cap) — you don't need to worry about upstream drops as an
external client of the gateway, only about your own connection to `:8400`.

### GC's own `/events` (internal, `:8430` directly — not proxied by the gateway either)

Emits `GcEvent` (`lib/gc/src/events.rs`) directly, tagged by `"type"`: `entry_registered`,
`entry_touched`, `entry_locked`, `entry_unlocked`, `entry_evicting` (carries
`"reason": "ttl_expired" | "budget_pressure" | "forced"`), `entry_evicted`,
`entry_eviction_failed`, `sweep_completed`, `dir_registered`, `budget_exceeded`,
`make_room_completed`, `path_moved`. These are exactly what gets re-broadcast under
`Topic::Gc` on the gateway hub — there is no reason to connect to `:8430` directly instead of
subscribing to `"gc"` on the gateway.

---

## Summary of gaps vs. documented behavior (do not assume otherwise)

Verified directly against source on this date, not copied from a prior status document:

- **The gateway's REST proxy (`/api/inference/*`, `/api/gc/*`) silently drops all query
  strings** — confirmed by live testing against a running stack, not just source reading (see
  the callout near the top of this document). Filtered list calls silently return unfiltered
  results; calls with a *required* query param (e.g. `GET /api/gc/entries/get?path=...`) fail
  outright with a 400 through the gateway even though the identical call works against
  `:8420`/`:8430` directly. This is the single biggest practical trap for a tool author who
  assumes "always go through the gateway" applies uniformly.
- **No per-token WebSocket streaming exists anywhere in the stack.** `StreamEvent::Token`
  is defined in the wire types but never constructed by the engine.
- **`GET /v1/completions/:id/stream` (the inference service's own per-completion WS) cannot
  be reached through the gateway at all** — the gateway's REST proxy is a buffered
  request/response forwarder with no WebSocket-upgrade support.
- **`Topic::Completion(id)` subscriptions on the gateway hub are inert** — the routing logic
  exists but no upstream event currently carries the `completion_id` field it looks for.
- **`POST /v1/models/:id/download` is a stub** — acknowledges but does not trigger a real
  download.
- **GC's `"on_full": "Migrate"` policy silently falls back to delete** — not implemented.
- **The benchmark kernel curve is a single-axis (`output_tokens`) fit**, not a full
  multi-dimensional sweep surface, and whether that's the intended final shape is an open
  question per the workspace status doc, not something this document should be read as
  endorsing as final.
- **`/v1/estimate` builds a fresh, unwarmed estimator on every call** rather than reusing a
  pre-warmed one — functionally fine, just recomputes the regression each time.

For continuous/passive access to completions today, the reliable pattern is: submit via
`POST /api/inference/v1/completions` (returns an id), then poll
`GET /api/inference/v1/completions/:id` (a path parameter, so this one is unaffected by the
query-string bug above) until its `state` is terminal, then `GET .../result` — combined with
subscribing to the gateway's `/events` hub (`"lifecycle"`, `"queue"`, `"system"`, `"gc"`
topics, or `"all"`) for coarse-grained system activity. If you need the *filtered* list
endpoint (`?state=...`), call `:8420` directly — through the gateway the filter is silently
ignored and you'll get the unfiltered default page instead. There is currently no supported
way to get live per-completion or per-token push updates through the gateway.
