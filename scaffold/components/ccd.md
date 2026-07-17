# ccd

**Status:** NEW (reconstructed from operator voice-thread intent capture — the
`ideas` branch it was to live on does not exist; see overview caveat). **Nesting:**
top-level. Aliases: Marshall / CCM. **Build order: FIRST of the new concepts.**

The Cloud Code Daemon: a daemon for managing agents — "we want to run all cloud
code through the Marshall." It spawns, tracks, and routes cloud-code (Claude Code)
agent processes (`agent-management`) and makes their LLM calls via the local
inference node (`llm-calls`). It registers itself and discovers peers/services
through the mesh service-registry (`service-registration`). It is the concrete,
bounded foundation on which Org is built (`org-on-ccd`) — NOT the maximalist
consumer. Open operator question: does CCD live inside the Substrate workspace or
stand alone? (Unresolved.)
