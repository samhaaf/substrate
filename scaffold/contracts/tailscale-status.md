# Contract: tailscale-status

## Parties
mesh.tailscale-query  ->  {mesh.network-topology, mesh.completion-router}

## What the edge carries
Parsed Tailscale self+peer status snapshots (hostname, DNSName, TailscaleIPs, ACL
tags, online). The standardized, extensible query surface both siblings consume;
kept synchronous. Schema deferred.
