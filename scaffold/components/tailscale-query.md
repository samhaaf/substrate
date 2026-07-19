# tailscale-query

**Status:** NEW — promoted to its own crate (`lib/tailscale`, `substrate-tailscale`).
**Nesting:** child of mesh (crate sibling, not a module — see mesh.md Concern 0).

## Charter

A standardized, extensible library for querying the Tailscale network behind a
typed API. Wraps `tailscale status --json` today and is shaped so new subcommands
("tools we add as we go") become new typed methods without breaking callers. It
returns parsed self+peer status (hostname, DNSName, TailscaleIPs, ACL tags,
online, self-vs-peer). It owns **only querying/parsing Tailscale** — no polling
loop, no diffing, no async runtime (callers own those). It does NOT decide routing
or topology semantics; it hands back facts.

## Primary design concerns

- **Extensibility = adding a method, never breaking one.** `TailscaleQuery` trait,
  one typed method per subcommand: `status()` now; `netcheck()`, `ping()`,
  `whois()` as future additions. This is the operator's "add new tools as we go"
  made structural.
- **Two impls.** `RealTailscale` (shells out via `std::process::Command`) and
  `FakeTailscale` (fixture-driven) — the fake is what lets topology/router tests
  and no-tailnet CI/dev boxes run at all.
- **Private serde subset, decoupled from public types.** Deserialize only the JSON
  fields we use into private structs; expose stable public types. Tailscale's JSON
  churn never leaks past the crate boundary.
- **Synchronous trait, deliberately.** Sidesteps the `dyn`-compat problem from the
  synthesis (finding #2); callers wrap the blocking shell-out in `spawn_blocking`.
  Never called on a request path.

## Relationships / edges

- {network-topology, completion-router} via `tailscale-status` — producer of
  parsed self+peer snapshots to both siblings
  (scaffold/contracts/tailscale-status.md)

## Nesting

Parent: mesh | Children: none. Own crate because the operator asked for it
verbatim, it has two in-repo consumers, and it is the cleanest reuse/"shared
library later" candidate in the tree (confirming + strengthening the decompose
pass's nest-it call).

## Thoroughness level

approach-sketched — trait shape, impls, and parse-decoupling are decided; the
exact public struct fields are the Contract Harmonizer's (`tailscale-status`).

## Assigned design-depth

Single strong-model (Opus) Component-Designer pass, grounded on the existing
`lib/mesh/src/discovery.rs` `TailscaleDiscovery` stub and the synthesis §1.

## Suggested fill-model

approach-sketched + low complexity → **mid model (Sonnet)**. Bounded shell-out +
serde + a fixture-backed fake.
