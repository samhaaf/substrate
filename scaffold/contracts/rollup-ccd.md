# Contract: rollup-ccd

> SUPERSEDES the requirements-only stub. Authored from rollup.md concern 5 (the
> authoring side — rollup owns the `Materialize` shape) and ccd.md concern 6 /
> its `rollup-ccd` proposal (the consumer view). Both sides agreed; this is the
> merge.

## Parties

ccd (L5 app) → rollup (L4 app)

CCD is the consumer (the PRIMARY consumer of rollup); rollup authors the surface.
Direction inbound-to-rollup.

## Purpose

CCD's **on-demand, specialized-plugin-per-agent assembly**: a `PluginManifest` +
runtime slots → a freshly materialized Claude-Code plugin directory, generated
per spawn, **no symlinks / no file-copying** (the fragments in VFS are the
durable source of truth). CCD points a Claude Code process at the materialized
dir. The result carries a full provenance bill-of-materials so CCD can record
exactly which fragment versions an agent ran with (reproducibility + the causal
chain into CCD's usage DB).

## Schema

Reused from `types::rollup`: `PluginManifest`, `FragmentEntry`, `ScopeChain`,
`SlotMap`, `MaterializeResult`, `RollupProvenance`, `RollupError`.

```rust
// ccd -> rollup
struct AssemblePlugin {
    manifest: PluginManifest,     // { name, description, version, skills[], agents[], commands[] }
    runtime_slots: SlotMap,       // variables bound at reference time (INTENT #79)
    scope: ScopeChain,            // ordered most-specific → most-general; ccd supplies it (v1)
    output: OutputSink,           // where the plugin materializes
}
enum OutputSink {
    LocalDir(PathBuf),            // the common case: a real local dir ccd hands Claude Code
    VfsDir(VfsPath),              // a durable, shareable plugin written into VFS
    Inline,                       // small plugins: the file set returned in-band, no disk
}

// rollup -> ccd
struct AssembleResult {
    result: MaterializeResult,    // { files_written: Vec<VfsOrLocalPath>, provenance: RollupProvenance }
    plugin_json: serde_json::Value, // the generated plugin.json (Claude-Code manifest)
}
```

Materialized layout (ported verbatim from the working harness `plugin.rs`, so
stable): `skills/<slug>/SKILL.md`, `agents/<slug>.md`, `commands/<slug>.md`,
`plugin.json`. `alias_map` (external→internal slot names) then `slot_overrides`
(win over everything) apply per `FragmentEntry`.

## Error cases

- `RollupError` (any arm — `PromptNotFound`, `PinnedVersionNotFound`,
  `MissingSlot`, `CircularReference`, `Vfs(..)`, …): **a plugin is
  all-or-nothing.** A missing fragment or unfilled required slot fails the WHOLE
  assembly; ccd surfaces it to the caller/priority owner and does **not** spawn.
- A **degraded secret** (a `{{secret:}}` in a fragment) is **not** an error — it
  is a warning recorded in `result.provenance.warnings`; the plugin materializes
  with a `SecretRef` handle, never a value (guaranteed by `rollup-secrets`). CCD
  spawns normally.
- `Rollup(String)` on ccd's side is the stringified boundary error (rollup's
  error type never becomes a ccd type — types.md guardrail).

## Version sensitivity

- **Additive-safe:** `PluginManifest`/`FragmentEntry`/`OutputSink` grow additively
  (`#[serde(other)]` on `OutputSink`, `#[serde(default)]` on new fields).
- **Stable (ported prior art):** the Claude-Code output layout
  (`skills/…`, `agents/…`, `commands/…`, `plugin.json`) is the harness's working
  format — treated as frozen; a layout change would be a breaking, Claude-Code-
  compatibility restart.
- **MEDIUM overall** — manifests cross the wire but don't persist in mesh state
  (rollup is stateless request/response, `AddressingClass::AnyNode`).

## Reconciliation notes

- **`AssemblePlugin` == the `Materialize` variant of `rollup-mesh`.** rollup.md
  exposes one materialize surface; `rollup-mesh::Materialize { manifest, slots,
  scope, output }` and this edge's `AssemblePlugin { manifest, runtime_slots,
  scope, output }` are the **same shape**, named as a distinct edge only because
  CCD is a distinct first-class consumer. Field rename reconciled: `slots` (mesh)
  ≡ `runtime_slots` (ccd) — kept `runtime_slots` here (ccd's name; clearer at the
  plugin call site), `slots` on the generic `rollup-mesh` surface. One
  implementation serves both.
- **No disagreement.** ccd.md explicitly "consumes rollup.md's authored
  `AssemblePlugin`/`AssembleResult`" — the two proposals are identical modulo the
  slot-field name. `AssembleResult.result.provenance` is the exact bill-of-
  materials ccd records on its `agent_runs` row (ccd.md concern 2).
- **`OutputSink` owner:** defined in `types::rollup` (rollup authored it); ccd
  imports it. Consistent with the shared-vocab rule.

## Example data

CCD, on **macbook**, assembles a `demo-agent` plugin (one skill `code-review`,
`qwen3-4b` referenced only as a slot) into a local dir before spawning a Claude
Code agent for project `demo`:

```jsonc
// ccd -> rollup
{ "manifest": {
    "name": "demo-agent", "description": "reviewer for the demo repo", "version": "0.1.0",
    "skills":   [ { "slug": "code-review", "slot_overrides": { "model": "qwen3-4b" },
                    "alias_map": {}, "volatility": "Stable" } ],
    "agents":   [], "commands": [] },
  "runtime_slots": { "repo": "demo", "branch": "v1" },
  "scope": { "scopes": [
      { "id": "proj:demo", "name": "demo", "prefix": "vfs://prompts/demo/" },
      { "id": "global",    "name": "global", "prefix": "vfs://prompts/global/" } ] },
  "output": { "LocalDir": "/Users/operator/.cache/ccd/plugins/demo-agent-8f2c" } }

// rollup -> ccd
{ "result": {
    "files_written": [
      { "Local": "/Users/operator/.cache/ccd/plugins/demo-agent-8f2c/skills/code-review/SKILL.md" },
      { "Local": "/Users/operator/.cache/ccd/plugins/demo-agent-8f2c/plugin.json" } ],
    "provenance": {
      "fragments_used": [ { "id": "code-review", "version": 4, "scope": "proj:demo",
                            "vfs_path": "vfs://prompts/demo/prompts/code-review/4.md",
                            "content_hash": "sha256:9a1f…" } ],
      "slots_filled": ["model", "repo", "branch"], "files_included": [], "secret_refs": [],
      "referenced_not_inlined": [], "warnings": [],
      "trace": { "correlation_id": "c0110000-0000-4000-8000-000000000001",
                 "service": "rollup", "node_id": "macbook" } } },
  "plugin_json": { "name": "demo-agent", "version": "0.1.0", "skills": ["code-review"] } }
```

CCD records `provenance` (fragment `code-review@4`, `content_hash 9a1f…`) on the
`agent_runs` row for the spawned agent's thread — exact reproducibility of what
the agent ran with.
