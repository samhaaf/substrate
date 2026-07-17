# Contract: network-events

## Parties
mesh.network-topology  ->  any subscriber (gateway, ccd, org)

## What the edge carries
Live WS stream of topology changes: peer devices going on/off the Tailscale
network, and this device's OWN loss of Tailscale connectivity. A general
topology-change feed, distinct from the router's internal health table.
Schema/example deferred.
