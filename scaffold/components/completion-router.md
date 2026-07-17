# completion-router

**Status:** RESHAPE of the original `lib/mesh` core (router/balancer/registry/
proxy). **Nesting:** child of mesh.

The transparent completion-routing data plane: a `NodeRegistry` (discovery-refresh
+ state/model-inventory poll loops over `node-state-poll`), a `ModelAffinityBalancer`
(prefer resident tier, least-loaded tie-break; spill + Tier-3 deferred), and a
`forward()` that must carry status + headers + streaming body (the corrected
contract from the mesh-design synthesis), plus a WS relay for `/v1/completions/:id/
stream` and header/query node-pinning for benchmarks. Consumes `tailscale-status`
for discovery; forwards `v1-completion-api` to the owning node. Full design already
finalized in `reports/mesh-design-synthesis.md`.
