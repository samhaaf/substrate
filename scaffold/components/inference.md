# inference

**Status:** existing (`lib/inference` + `bin/inference`), RESHAPE. **Name
locked:** stays `inference` — the earlier candidate second rename (-> `llm`, or
any other label such as `gen`) is decided against; see naming note below.
**Nesting:** top-level parent of {store, engine, scheduler, models, cache,
telemetry, benchmark, api}.

The per-machine LLM serving runtime — the v2 successor to the v1 `Substrate`
struct. It wires every inference subsystem into one single-machine service and
exposes exactly one external contract: the `/v1/` REST+WS API (`v1-completion-api`).
Its internal composition root (`InferenceService::start`) is a private wiring
point, NOT the assembly seam. This pass isolates its genuinely-hard subsystems as
nested children so their crate-to-crate contracts can be reconsidered; the node's
own charter is the orchestration/lifecycle of those children behind the one API.

**Naming note (decided, operator confirmed):** the crate keeps the name
`inference`, not `llm`/`gen`/anything narrower. Reasoning is twofold: (1)
`inference` is meant to stay modality-agnostic — as new generation modalities
arrive (text-to-image, text-to-video, etc.) they extend this same crate rather
than spinning out separate per-modality crates, so a text/LLM-flavored name
would misdescribe its future scope; (2) `gen` was specifically rejected for a
practical reason in a voice-operated project — spoken aloud it's a homophone
for the name "Jen," which is untenable for an interface driven by voice
transcription/STT.
