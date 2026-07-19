# Contract: ccd-projects

## Parties
- `ccd` (L5, ledger author) `<->` `projects` (L6 stub).

*(Stub-track: both ends aware; CCD already authored the ledger fields. Content
deferred until `projects` leaves the stub track — INTENT #51/#68.)*

## Purpose
Thread `<->` project (+ optional environment) linkage in CCD's usage database
(INTENT #68, ccd.md concern 2). Lets per-project agent activity and usage be
attributed and rolled up, feeding `projects`' per-project finance/metadata view
and `spend`'s pull-shaped queries.

## Rough shape
**No new mechanism — the fields already live in CCD's ledger and
`agent-management`:**
- At spawn, `AgentCmd::Spawn` carries `project: Option<ProjectId>` and
  `environment: Option<EnvironmentId>`; CCD writes them onto the `agent_runs`
  row (and thence onto `usage_records` via the join).
- When `projects` leaves the stub track, those ids are **validated against
  projects** (an unknown `ProjectId`/`EnvironmentId` rejected or flagged at
  spawn); until then they are recorded uninterpreted.
- Reverse read: `projects` reads CCD's per-project agent/usage rollup back — the
  same read surface `spend-ccd` uses (`UsageQuery -> UsageRollup` grouped by
  `project_id`/`environment_id`). `projects` is the authority those ids resolve
  to names + containment hierarchy against.
- Direction is `ccd` writes the linkage / `projects` validates + reads back; no
  hot-path coupling, no CCD dependency on `projects` being live (graceful when
  `projects` is absent).

## Open questions
- Whether validation is synchronous at spawn (rejecting an unknown project) or
  lazy/advisory — leaning advisory while `projects` is stub-track.
- `ProjectId` / `EnvironmentId` identity type ownership: `types` vs `projects`
  crate — provisionally `types` newtypes so CCD can hold them without a `projects`
  dependency.
- Whether `aui` conversations register as CCD threads carrying this same
  `(project, environment)` linkage (aui.md flags reuse, not a new record).
- Overlap with a proposed `spend-projects` resolve edge (id → name) — may fold
  into this reverse-read + a `projects` read rather than a distinct pair.
