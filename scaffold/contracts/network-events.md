# Contract: network-events

## Parties
mesh.network-topology  ->  any subscriber (ccd, org)
*(gateway removed as a subscriber, 2026-07-18 — gateway merged into mesh; the
observability fan-out hub now consumes topology in-process, not over this edge)*

## What the edge carries
Live WS stream of topology changes: peer devices going on/off the Tailscale
network, and this device's OWN loss of Tailscale connectivity. A general
topology-change feed, distinct from the router's internal health table.
Schema/example deferred.
