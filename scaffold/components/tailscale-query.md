# tailscale-query

**Status:** NEW. **Nesting:** child of mesh.

A standardized, extensible surface for querying the Tailscale network — the
operator explicitly wanted "a crate just to query Tailscale... a separate surface
worth capturing and standardizing," extensible so "we can add new tools to it as
we go." Wraps `tailscale status --json` (and future subcommands) behind a typed
API returning parsed self+peer status (hostname, DNSName, IPs, ACL tags, online).
Earned its own (nested) component because it is independently reusable and
consumed by two siblings — `network-topology` and `completion-router`'s node
discovery — via the `tailscale-status` edge. Kept synchronous (registry does the
async wrapping) per the mesh-design decision.
