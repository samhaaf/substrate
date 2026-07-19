# Contract: openrouter-secrets

## Parties
- `openrouter-mgmt` (L6 stub) `->` `secrets` (L3).

*(Stub-track — both ends aware; secrets.md already stubs its side. Content
deferred until `openrouter-mgmt` leaves the stub track. Rides the existing
secrets key hierarchy + `llm_safe` policy unchanged — no new secrets mechanism.)*

## Purpose
Durable, **use-without-seeing** storage and rotation of OpenRouter credentials:
the account provisioning key and every minted per-key runtime key (INTENT #41,
#94). OpenRouter keys are secrets like any other; `openrouter-mgmt` stores them
via `secrets` and uses them via a proxied resolve-into-sink, never receiving
plaintext into its own LLM-reachable paths.

## Rough shape
Ordinary Global/Project-scoped secrets with rotation, reusing `secrets`
vocabulary:

```rust
// on create: openrouter-mgmt mints a key with OpenRouter, hands the once-returned
// raw key to secrets, receives a reference (never re-reads the raw value):
struct SecretRef { id: Uuid, name: String, scope: SecretScope, version: u32 } // types (secrets.md)

// use: proxied resolve-into-sink — the plaintext flows secrets -> outbound TLS,
// never through openrouter-mgmt's own memory as an LLM-reachable value:
//   secrets.use(ref, Sink::HttpAuthHeader { .. })   // "use it in a context, never see it"
```

- **Create:** the once-returned raw OpenRouter key is written to `secrets`; a
  `SecretRef { id, name, scope, version }` is handed back.
- **Use:** `openrouter-mgmt` makes the authenticated OpenRouter HTTPS call via a
  proxied resolve-into-sink (`use(ref, sink)`), never receiving plaintext — the
  use-without-seeing invariant (INTENT #94; no secret ever reaches an LLM path).
- **Rotate:** mint-new-secret + retire-old, tracking the OpenRouter-side rotate;
  `version` bumps.
- The OpenRouter provisioning API traffic itself is ordinary outbound HTTPS to a
  third party — **explicitly NOT a mesh contract pair** (parallel to secrets.md
  treating the OS keychain / AWS API as platform capabilities).

## Open questions
- Scope granularity: one Global provisioning key + per-key runtime secrets at
  Project scope, vs all Global — leaning provisioning=Global, runtime=Project.
- Whether `secrets`' `Sink` vocabulary already covers an HTTP auth-header sink or
  needs an additive `Sink` variant (preferably reuse existing).
- Rotation-atomicity across a live in-flight OpenRouter request (mint-before-
  retire ordering).
