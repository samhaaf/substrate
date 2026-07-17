# network-topology

**Status:** NEW. **Nesting:** child of mesh.

The live network-topology layer: a WebSocket surface that emits events when the
Tailscale network changes — peer devices going on/off the network, and critically
this device's OWN loss of connection to the Tailscale network. The operator: "a
WebSocket indicating when certain devices are off the network, but also if we lose
connection to the Tailscale network ourselves on this device." Consumes
`tailscale-status` (polling/diffing snapshots) and publishes `network-events` to
any subscriber (gateway, CCD, Org). Distinct from `completion-router`'s internal
health polling: this is a general topology-change feed, not a routing table.
