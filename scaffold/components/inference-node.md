# inference-node

**Status:** existing (`lib/inference` + `bin/inference`), RESHAPE; rename
candidate -> `llm` (operator idea, not locked). **Nesting:** top-level parent of
{store, engine, scheduler, models, cache, telemetry, benchmark, api}.

The per-machine LLM serving runtime — the v2 successor to the v1 `Substrate`
struct. It wires every inference subsystem into one single-machine service and
exposes exactly one external contract: the `/v1/` REST+WS API (`v1-completion-api`).
Its internal composition root (`InferenceService::start`) is a private wiring
point, NOT the assembly seam. This pass isolates its genuinely-hard subsystems as
nested children so their crate-to-crate contracts can be reconsidered; the node's
own charter is the orchestration/lifecycle of those children behind the one API.
