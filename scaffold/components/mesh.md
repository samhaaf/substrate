# mesh

**Status:** existing (`lib/mesh` + `bin/mesh`), RESHAPE + substantially EXPAND.
**Nesting:** top-level parent of {tailscale-query, network-topology,
service-registry, completion-router}.

The network / coordination plane, now the operator's unified "mesh utility." Its
original job (transparent completion routing across node daemons over HTTP only,
model-affinity balancing, Tailscale-native discovery — see
`reports/mesh-design-synthesis.md`) is retained as the `completion-router` child.
Three new surfaces are added as siblings: a standardized Tailscale-query surface,
a live network-topology WS layer (including this device's own connectivity), and
a distributed eventually-consistent service registry. The registry is also the
system's single wiring seam (see overview). Strict tier boundary preserved: mesh
depends on no node-internal crate and talks to nodes only over HTTP.
