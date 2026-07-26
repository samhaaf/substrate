# Contract: ccw-api

## Parties

- **CCW** (`ccw`, the Claude Code Wrapper daemon) — a pure WebSocket service on a
  fixed local port. It OWNS every `claude` invocation on the machine (INTENT
  #208: "assume I'm never going to directly call Cloud Code again — it's only
  managed by this service").
- **every Leverage service that reaches Claude Code** — keepers spawning threads,
  the loop harness, AUI, `spend`/`agents` (future) — as WS clients. This is THE
  data contract other services use to start threads, stream events, and read
  history/usage. Pre-mesh today (direct `ws://127.0.0.1:<port>/ws`); mesh-ready
  by construction (see Version sensitivity).

This is the **v1 SHIPPED** surface (crate `substrate-ccw`, binary `ccw`). The v0
CLI-wrapper (`ccw run|status|budget …`) is DELETED — there is no CLI.

## Purpose

Expose EVERYTHING about Claude Code activity over one socket (INTENT #208 "full
event exposure"): every in-thread event (assistant text, tool_use/tool_result,
thinking, per-message + per-turn usage, compaction), subagent
spawn/launch/completion with **liveness heartbeats** (a session reports BUSY
while any async subagent runs — the operator's critical requirement), and the
wrapper's own governance meta-events (budget pending, account switch, retry,
limit hit, calibration). Plus the session lifecycle (start/send/resume/list/
history/cancel), the budgets/accounts/ledger surfaces, and a per-session tool
**clean-slate** option (deactivate all built-ins, inject engineered tools only).

## Transport

- **Endpoint:** `GET /ws` → WebSocket upgrade. `GET /health` is a plain-HTTP
  liveness probe (`{status,daemon,version,proto}`).
- **Port:** fixed default **3651**. Rationale: the mesh daemon reserves **3649**
  (INTENT #58 single-port locality); CCW is a distinct local daemon and must not
  collide, so it takes 3651, leaving 3650 as a deliberate gap for a future
  sibling. Overridable via `~/.leverage/ccw/config.toml` `port`. Bind host
  defaults to `127.0.0.1` (local-only; the remote plane is mesh's job).
- **Encoding:** JSON text frames, one message per frame. 30s server pings.
- **Framing philosophy:** promise/event-based, consistent with `mesh-transport`
  (the #154 success/error/promise `Outcome` switch; a version-stamped welcome)
  but kept **pre-mesh boring** — one socket, request/response RPC + a pushed
  event stream, no relay/addressing/peer handshake yet.

## Schema

All shapes are tagged enums (`#[serde(tag="type")]` / `tag="kind"`), additive-
safe (`#[serde(default)]`, no `deny_unknown_fields`, an `unknown` catch-all on
the event taxonomy).

### 1. Connect handshake

On upgrade the daemon immediately sends a **welcome** (sender version stamp,
INTENT #113 — minimal pre-mesh):

```jsonc
{ "type": "welcome", "proto": 1, "daemon": "ccw", "daemon_version": "0.1.0" }
```

### 2. Client → daemon frames (`ClientMsg`)

```jsonc
// RPC request — `id` correlates the response, `method` routes, `params` is the
// per-method schema.
{ "type": "request", "id": "<client-uuid>", "method": "sessions.start", "params": { … } }

// Subscribe to the event stream. `sessions:null` = firehose (every session);
// `sessions:[…]` = only those session ids.
{ "type": "subscribe",   "sessions": null }
{ "type": "unsubscribe", "sessions": ["<sid>"] }   // null clears the whole subscription
```

### 3. Daemon → client frames (`ServerMsg`)

```jsonc
{ "type": "sub_ack",  "firehose": true, "sessions": [] }
{ "type": "response", "correlate": "<id>", "outcome": { … } }         // the #154 switch
{ "type": "event",    "session_id": "<sid>", "seq": 42, "event": { … } }  // a CcwEvent
```

### 4. `Outcome` — the success / error / promise switch (INTENT #154)

Every `response` is exactly one arm; a `promise` is a normal success, not an
error. v1 never emits `promise` for its own methods yet — the arm exists so the
contract is mesh-ready with no wire change.

```jsonc
{ "type": "success", "payload": { … } }   // the per-method result schema
{ "type": "error",   "error": "<message>" }
{ "type": "promise", "promise": "<uuid>" } // (reserved) value later as a correlated push
```

### 5. Methods

| method | params | success payload |
|---|---|---|
| `sessions.start` | `SessionSpec` (+ optional `prompt` to run the first turn) | `{ session_id }` |
| `sessions.send` | `{ session_id, prompt }` | `{ accepted, session_id }` |
| `sessions.resume` | `{ session_id, prompt? }` | `{ resumed, session_id }` |
| `sessions.list` | `{ limit? }` | `{ sessions:[{session_id,account,cwd,model,budget_id,status,last_seq,…}] }` |
| `sessions.history` | `{ session_id, after_seq?, limit? }` | `{ session_id, events:[{seq,event}], next_after }` |
| `sessions.cancel` | `{ session_id }` | `{ cancelled, session_id }` |
| `budgets.add` | `{ id, weekly_pct?, session_backoff_pct?, accounts? }` | `{ id, rules }` |
| `budgets.list` | `{}` | `{ budgets:[{id,weekly_pct,session_backoff_pct,accounts}] }` |
| `accounts.list` | `{}` | `{ accounts:[{account,config_dir}] }` |
| `accounts.status` | `{}` | `{ accounts:[{account,pools:[{pool,pct,reset}]}] }` (live zero-cost `/usage`) |
| `ledger.query` | `{ budget?, limit? }` | `{ invocations:[{account,session_id,total_tokens,cost_usd,limit_hit,…}] }` |

`sessions.history` is served from **CCW's own event log** (SQLite `events`,
keyed `(session_id, seq)`), NOT from claude's on-disk transcripts — the resume/
paging cursor is `seq`. `after_seq` is exclusive; page forward with the returned
`next_after`.

### 6. `SessionSpec` (start params)

```jsonc
{
  "cwd": "/work/project",          // working dir (must exist)
  "model": "fable",                // alias (fable|opus|sonnet) or full id
  "account": "auto",               // "auto" | { "named": "<name>" }
  "budget_id": "std",              // optional; governs pre-spawn admission
  "append_system_prompt": "…",     // role prompt
  "add_dirs": ["/extra"],          // --add-dir scoping
  "permission_mode": "bypassPermissions", // else --dangerously-skip-permissions
  "tools": {                       // the clean-slate / injection policy
    "clean_slate": true,           // --tools "" — deactivate ALL built-ins
    "tools": null,                 // else an explicit set "Bash,Edit,Read"
    "agents_json": "{…}",          // --agents (inject custom agents, no MCP)
    "plugin_dirs": ["/plugins/x"], // --plugin-dir (hooks/skills/commands/agents)
    "allowed_tools": ["Bash(git *)"],
    "disallowed_tools": ["WebFetch"]
  },
  "max_budget_usd": 5.0,           // optional native --max-budget-usd
  "fallback_model": "sonnet",      // optional --fallback-model (overload)
  "rollup": null                   // STUB — a non-null ref is REJECTED (see below)
}
```

The daemon always spawns with the canonical streaming recipe
(`-p --output-format stream-json --verbose --model … --session-id/--resume …`),
prompt on stdin, env stripping `ANTHROPIC_API_KEY`, disabling auto-memory, and
keeping async subagents alive past claude's 10-min ceilings
(`CLAUDE_CODE_PRINT_BG_WAIT_CEILING_MS=0`,
`CLAUDE_ASYNC_AGENT_STALL_TIMEOUT_MS=3600000`).

### 7. `CcwEvent` — the event taxonomy (`kind` discriminator)

Every stream-json line maps to a concrete variant; genuinely-unrecognized lines
surface as `unknown` (nothing is ever dropped — aui's blind spot fixed).

**In-thread (from the claude stream):** `system_init` (full tool/model/mcp
roster), `compacted{trigger,pre_tokens}`, `assistant_text{text,parent_tool_use_id?}`,
`thinking{text}` (kept, not dropped), `tool_use{id,name,input,parent_tool_use_id?}`,
`tool_result{tool_use_id,content,is_error,parent_tool_use_id?}` (full content,
not a 200-char preview), `message_usage{model,input/output/cache tokens}`,
`stream_event{raw}`, `agent_listing_delta{raw}`, `result_turn{subtype,is_error,
total_cost_usd,num_turns,duration_ms,stop_reason,model_usage}`, `unknown{raw}`.

**Subagent liveness (recon Deliverable 2):** `agent_spawn{tool_use_id,
subagent_type,description,run_in_background}`, `agent_launched{tool_use_id,
agent_id,is_async,status,output_file?}`, `agent_completed{agent_id}`,
`liveness{busy,parent_running,running_subagents:[…]}`. **Rule:** `busy` is true
while the parent process runs OR any async subagent's transcript is still
churning (mtime within the idle window); a thread is IDLE only when the parent
`result` arrived AND every async agent has gone quiet. A `liveness` beat with
`busy:true` fires the moment the parent `result` streams if async agents remain
— so a subscriber never mistakes parent-result for thread-idle.

**Session lifecycle:** `session_started{session_id,account,cwd,model}`,
`turn_started{prompt_preview,attempt}`, `session_ended{session_id,exit_code}`,
`error{message}`.

**Wrapper meta (CCW's governance surface):** `budget_pending{reason,eta?}`,
`account_switched{from,to,reason}`, `retry_started{reason,attempt,wait_ms?,eta?}`,
`limit_hit{pool?,reset?,status,text}` (surfaced the moment a 429 streams),
`calibration_updated{account,pool,tokens_per_percent,sample_count}`.

## Behavior — the daemon owns invocation + governance

- **Retries (recon Deliverable 5).** Transient failures retry with meta-events;
  deterministic ones are fatal. 429 → switch to an alternate account with
  headroom (`account_switched`) or wait to the parsed reset time
  (`retry_started`, capped). 5xx/overload/conn-drop → bounded backoff.
  `error_during_execution` → retry once. `error_max_*` ceilings → fatal.
  "No conversation found" (expired resume) → restart fresh, not a same-resume
  retry. Attempt ceiling default 3.
- **Budget admission (pre-spawn, INTENT #202/#207).** With a `budget_id`, the
  turn is admitted against a zero-cost `/usage` snapshot + the ledger's weekly
  consumption + the learned tokens-per-percent calibration. Blocked → a
  `budget_pending{reason,eta}` event (the loop UI's ETA); observe-don't-kill
  stays the mid-run default (overage debits the budget so the NEXT turn blocks).
  No estimates — weekly enforcement only bites once calibrated.
- **Event log.** Every `CcwEvent` is persisted (`events`, `(session_id,seq)`)
  before it is pushed; `sessions.history` replays from here; sessions survive a
  daemon restart (rehydrated from the ledger; `seq` continues).

## Error cases

- Per-request failures come back as the `error` arm of `Outcome` (never a
  dropped frame): `missing required param: <k>`, `no such session: <id>`,
  `unknown method: <m>`, `bad session spec: …`, `no registered accounts`,
  `session <id> already has a turn in flight`.
- **rollup STUB (INTENT #208).** `sessions.start` with a non-null `rollup` ref
  is rejected with `Error("rollup assembly is not implemented in CCW v1 …")`.
  Intent: CCW will assemble prompts/plugins via `rollup` ("inside CCW we're going
  to have it do rollup — that's going to be huge"); v1 ships the named seam only
  (`rollup-cc` remains the consumer edge — see `components/ccw.md`).
- In-turn failures surface as `error`/`limit_hit`/`result_turn{is_error}` events
  on the stream, not as an RPC error (the request that started the turn already
  succeeded with `{accepted}`).
- Invalid client frames get an `error` response correlated to an empty id.

## Version sensitivity

**Pre-mesh, additive-safe.** The `proto` field on `welcome` is the anchor (bumped
only on a breaking envelope change). New `method` strings, new `CcwEvent` `kind`
variants (readers fall through to `unknown`), and new `#[serde(default)]` fields
are all non-breaking. Breaking: changing the `Outcome` three-arm switch, the
`request`/`response`/`event` frame shape, or the `(session_id,seq)` history
cursor. **Mesh-ready seam:** when CCW joins the mesh, each `method`/`params`
becomes a `mesh-transport` `Request` payload and `Outcome` maps onto
`ResponseOutcome` unchanged (the switch is deliberately identical); the pushed
`event` stream becomes `pubsub-protocol` `cc.*` deliveries. No client-visible
schema change is required to make that move — this contract is the payload, the
mesh envelope wraps it.

## Example data

**Start a clean-slate session and stream its first turn:**
```jsonc
// -> subscribe first so no events are missed
{ "type": "subscribe", "sessions": null }
// <- { "type": "sub_ack", "firehose": true, "sessions": [] }

// -> start + run the first turn
{ "type": "request", "id": "r1", "method": "sessions.start",
  "params": { "cwd": "/work", "model": "fable", "account": "auto",
              "tools": { "clean_slate": true }, "prompt": "summarize README" } }
// <- { "type": "response", "correlate": "r1",
//      "outcome": { "type": "success", "payload": { "session_id": "8f2c…" } } }

// <- events (seq-ordered):
{ "type": "event", "session_id": "8f2c…", "seq": 1, "event": { "kind": "session_started", … } }
{ "type": "event", "session_id": "8f2c…", "seq": 3, "event": { "kind": "system_init", "tools": ["Bash","Read"], … } }
{ "type": "event", "session_id": "8f2c…", "seq": 5, "event": { "kind": "assistant_text", "text": "The README…" } }
{ "type": "event", "session_id": "8f2c…", "seq": 8, "event": { "kind": "result_turn", "subtype": "success", "total_cost_usd": 0.01 } }
```

**Subagent still running at parent result (the liveness guard):**
```jsonc
{ "kind": "agent_spawn",     "tool_use_id": "toolu_1", "subagent_type": "general-purpose", "run_in_background": true }
{ "kind": "agent_launched",  "tool_use_id": "toolu_1", "agent_id": "agent-abc", "is_async": true, "status": "async_launched", "output_file": "…/agent-abc.jsonl" }
{ "kind": "result_turn",     "subtype": "success", "stop_reason": "end_turn" }   // parent done…
{ "kind": "liveness",        "busy": true, "parent_running": false, "running_subagents": ["agent-abc"] }  // …but NOT idle
// …later, transcript goes quiet:
{ "kind": "agent_completed", "agent_id": "agent-abc" }
{ "kind": "liveness",        "busy": false, "parent_running": false, "running_subagents": [] }
```

**429 → retry meta then recovery:**
```jsonc
{ "kind": "limit_hit",     "status": 429, "reset": "2:40am (America/Chicago)", "text": "You have hit your session limit …" }
{ "kind": "retry_started", "reason": "rate limit (429)", "attempt": 2, "wait_ms": 50, "eta": "2:40am (America/Chicago)" }
{ "kind": "assistant_text","text": "…" }   // second attempt succeeds
```

**Over-budget admission:**
```jsonc
{ "kind": "budget_pending", "reason": "account default: session 95% ≥ backoff 10%", "eta": "Jul 25 at 11:29pm (America/Chicago)" }
```

## Reconciliation notes

1. **This is `ccw`'s own WS wire, distinct from `cc-events`.** `cc-events`
   (wave-2) is the `cc.*` topic catalog on `pubsub-protocol` for mesh's
   observability plane; `ccw-api` is the direct pre-mesh service contract clients
   speak today. When CCW joins the mesh the two converge (Version sensitivity) —
   the `CcwEvent` taxonomy here is the concrete `AgentEvent`/`cc.*` payload
   `cc-events`/`agent-management` describe abstractly.
2. **`agent-management`'s spawn/track/signal/stream/reap-by-handle is realized
   here** as `sessions.*` + the event stream; the "stable handle" is the
   `session_id`. The full strategy-language admission engine (cc.md concern 1)
   stays FUTURE — v1 ships the boring 20%-weekly/50%-session template
   (`budgets.*`), not the declarative `Strategy`/`Predicate` grammar.
3. **rollup is the one stubbed seam** (INTENT #208), consistent with
   `rollup-cc`: CCW is the consumer; v1 rejects a rollup ref rather than
   assembling. No `rollup-cc` shape changes.
