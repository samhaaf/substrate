# Contract: agent-management

## Parties
- `cc` (L5, owner/author) `<->` cloud-code (Claude Code) agent processes.
- Shaped-for consumers (read/drive by handle): `org` (via `org-on-cc`),
  `agents` (via `agents-cc`), the `dashboard`, and `aui`.

*(Stub-track file per this cluster's charter — cc.md authored a complete schema,
reproduced here as a firm Schema sketch; example world deferred until the
per-pair harmonization pins the CLI/WS split.)*

## Purpose
Spawn / track / signal / stream / reap Claude Code processes by a **stable
handle** — the multi-agent process-supervision substrate cc owns ("run all
cloud code through the Marshall"). It is the foundation `org`'s inter-agent
communication and the future `agents` umbrella layer on top; both address agents
by handle through this same surface. Every spawn is **admission-gated**: it
carries a `BudgetGrant` from cc's declarative strategy engine that the
supervisor enforces (INTENT #68).

## Schema sketch

Control API over mesh WS (`:3649`, mediated) + a thin CLI. `types::cc`
vocabulary.

```rust
// caller -> cc
pub enum AgentCmd {
    Spawn  { task: String, workdir: PathBuf, plugin: Option<PluginManifest>, slots: SlotMap,
             caller: CallerContext, priority: u8, thread: Option<ThreadId>,
             project: Option<ProjectId>, environment: Option<EnvironmentId> },
             // -> Result<AgentHandle, CcError>; a strategy-deferred spawn -> Deferred, NOT a handle
    Send   { handle: AgentHandle, input: String },      // deliver input to a running agent
    Signal { handle: AgentHandle, sig: AgentSignal },   // Interrupt | Pause | Resume | Terminate
    List   { filter: AgentFilter },                     // -> Vec<AgentStatus>
    Logs   { handle: AgentHandle, from: Option<u64> },  // -> stream of AgentEvent
}
pub enum AgentSignal { Interrupt, Pause, Resume, Terminate }
pub struct AgentStatus { handle: AgentHandle, state: AgentState, thread: Option<ThreadId>,
                         usage: TurnUsageSummary, grant: BudgetGrant }
pub enum AgentState { Spawning, Running, Idle, Exited { code: Option<i32> }, Failed { reason: String } }

// cc -> caller (also the CcEventPayload::Agent(..) arm on cc-events)
pub enum AgentEvent {
    Started       { handle: AgentHandle },
    TurnCompleted { handle: AgentHandle, usage: TurnUsage },
    ReportEmitted { handle: AgentHandle, path: PathBuf },
    StateChanged  { handle: AgentHandle, state: AgentState },
    Exited        { handle: AgentHandle, code: Option<i32> },
}
```

When `plugin` is present, cc first calls `rollup-cc` to materialize it (no
symlinks/copies), then spawns Claude Code pointed at the materialized dir; the
`RollupProvenance` is recorded on the `agent_runs` row (the run's bill-of-
materials). `AgentHandle` is a stable opaque string that survives a cc daemon
restart if the child process survives (re-adopted by `(pid, spawn_cookie)`).

**Error cases:** `SpawnFailed`, `UnknownHandle`, `AgentExited`,
`BudgetExhausted` / admission `Deferred { retry_after }` (a deferred spawn is NOT
a live handle). `CcError` lives in `types::error::cc`.

**Version sensitivity:** MEDIUM. `AgentCmd`/`AgentEvent`/`AgentState` are
wire-crossing (org, dashboard) → additive-only, `#[serde(other)]` on every enum,
`#[serde(default)]` on new fields. Handle format is an open string.

## Open questions
- CLI-vs-WS split of the control surface (which verbs are CLI-only) — pinned at
  the per-pair harmonization; both ride the same `AgentCmd` vocabulary.
- Exact `TurnUsage` / `TurnUsageSummary` shape shared with `spend-cc`'s read
  model (metering authority is the CC subprocess output stream, NOT inference —
  INTENT #40).
- Whether `Logs` streaming reuses `cc-events`' pubsub topics or a dedicated
  per-handle WS (leaning: `cc/<node>/agent/<handle>` prefix on the same relay).
- Orphan re-adoption identity (`(pid, spawn_cookie)`) is authored; its durability
  across a node reboot (not just a daemon restart) is open.
