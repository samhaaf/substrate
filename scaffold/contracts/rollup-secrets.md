# Contract: rollup-secrets

> Authored this pass from **both** proposals: secrets.md concern 4/8 (the
> authoring/enforcing side) and rollup.md concern 4/8 (the consumer side). Both
> stated the same invariant independently — this file is the reconciliation.

## Parties

rollup (L4 app) → secrets (L3 app)

rollup is the consumer; secrets is the enforcer. rollup resolves
`RefTarget::Secret` markers embedded in fragments; secrets governs the
`llm_safe` fail-or-degrade so no raw secret ever enters LLM-bound rollup output.

## Purpose

Carry the **no-secret-in-LLM-bound-output invariant** across the two crates so
neither can drift. When a rollup fragment references a secret (`{{secret:name}}`
or `{{ref:secret:name}}`), rollup calls secrets to resolve the reference and
receives back **only a `SecretRef` handle** (never plaintext) for insertion into
assembled text/files/plugins. The invariant holds even if one side is buggy,
because the plaintext refusal is enforced *inside secrets*, where the value
lives.

## Schema

Shared vocabulary reused from `types::secrets` and `types::rollup`:

```rust
// types::secrets
struct SecretRef { id: Uuid, name: String, scope: SecretScope, version: u32 }
enum   SecretRefOrName { Ref(SecretRef), Name { name: String, scope: SecretScope } }
struct CallerContext { service_slug: String, llm_safe: bool, purpose: Option<String> }
// types::rollup — the SINGLE InsertForm (reconciliation: secrets' local copy is dropped)
enum   InsertForm { Raw, Reference }
```

Request / response:

```rust
// rollup -> secrets
struct RollupSecretResolve {
    ref_or_name: SecretRefOrName,
    form: InsertForm,        // rollup ALWAYS sends Reference for output; Raw is treated as a degrade request
    caller: CallerContext,   // caller.service_slug = "rollup", caller.llm_safe = true (from mesh identity)
}

// secrets -> rollup
enum ResolveOutcome {
    Reference(SecretRef),                                   // the handle rollup emits into output
    DegradedToReference { r#ref: SecretRef, warning: String }, // a Raw request under llm_safe → ref + warning
    // Raw(SecureValue) — the third arm of secrets' general ResolveOutcome — is UNREACHABLE on this edge.
}
```

## Error cases

- `Secrets(NotFound { name })` — the referenced secret does not exist;
  rollup surfaces it as `RollupError::Secrets(..)` (stringified — secrets' error
  type never becomes a rollup type, types.md guardrail) and fails **that
  reference**, not the whole assembly (a missing optional fragment behaves as the
  ported harness `PromptNotFound`).
- **`RawRefusedLlmSafe` is NOT an error on this edge.** rollup runs in *degrade*
  mode, so a raw request returns `DegradedToReference` (a warning carried into
  provenance), never a hard failure — the assembly proceeds with the reference.
  (In secrets' general model, `fail` mode would raise `RawRefusedLlmSafe`; rollup
  never selects it.)
- `NotDecryptable` / `KeychainUnavailable` — irrelevant here: rollup never
  requests plaintext, so secrets never needs to decrypt for this caller.

## Version sensitivity

- **Additive-safe:** new `ResolveOutcome` reachable variants (`#[serde(other)]`
  reserved); new `CallerContext`/`SecretRef` fields (`#[serde(default)]`).
- **FROZEN — the invariant, part of the contract text not commentary:** on this
  edge, **a plaintext secret is never returned to rollup.** The `Raw(SecureValue)`
  arm of secrets' general `ResolveOutcome` is *unreachable* here because (a)
  rollup always sends `form: Reference` for output, and (b) secrets' `llm_safe`
  (set from rollup's unspoofable mesh identity) forecloses raw regardless. This
  is the same freezing discipline secrets applied to its `Raw` arm — changing it
  is a disclosure-class breaking change, blessed only by an explicit operator +
  secrets round.

## Reconciliation notes

- **The unconditional-reference deviation — rollup's stronger stance WINS, both
  sides bless it.** secrets' *general* `llm_safe` model permits `Raw` for
  non-LLM callers (a deploy step, a handler). rollup deliberately goes
  **stronger**: it "never resolves raw secrets, full stop," regardless of sink,
  because rollup output is undifferentiated text that may flow anywhere
  downstream — so a raw secret in an assembled document is a disclosure
  *regardless of the eventual sink* (rollup.md concern 4). The losing position —
  "let rollup request raw when its immediate caller is non-LLM" — is recorded and
  rejected: rollup cannot know its output's ultimate destination, so it forecloses
  raw entirely. secrets' enforcement makes this free (the degrade already happens
  inside secrets under `llm_safe`); rollup's foreclosure is defence-in-depth on
  top. A handler that genuinely needs plaintext bypasses rollup and calls secrets'
  `use(ref, sink)` verb directly (secrets.md concern 3).
- **`InsertForm` unified to `types::rollup::InsertForm`.** secrets.md sketched a
  local `enum InsertForm { Reference, Raw }`; rollup.md owns
  `types::rollup::InsertForm { Raw, Reference }`. Since `RollupRef`/`InsertForm`
  are already referenced by `types::trigger` (queues) and now secrets, the single
  `types::rollup` definition is canonical (declarative data used by ≥2 crates →
  `types`). secrets' local copy is dropped; the variant order is cosmetic.
- **`SecretRefOrName` adopted verbatim** from both sketches (they agreed).

## Example data

A `demo`-project agent plugin fragment references the OpenRouter API key. rollup
resolves it reference-only; the assembled prompt carries a handle + a warning,
never the key:

```jsonc
// fragment body being resolved (in vfs://prompts/demo/openrouter-agent/1.md):
//   "Call OpenRouter with key {{secret:openrouter-key}} and summarize."

// rollup -> secrets
{ "ref_or_name": { "Name": { "name": "openrouter-key",
                             "scope": { "Project": { "project": "demo" } } } },
  "form": "Reference",
  "caller": { "service_slug": "rollup", "llm_safe": true, "purpose": "assemble demo/openrouter-agent" } }

// secrets -> rollup  (the marker was a bare {{secret:}} = nominally Raw → degraded)
{ "DegradedToReference": {
    "ref": { "id": "a1b2c3d4-0000-4000-8000-000000000002", "name": "openrouter-key",
             "scope": { "Project": { "project": "demo" } }, "version": 1 },
    "warning": "a raw secret was requested in an llm_safe context; an ID reference was injected instead" } }

// resulting assembled prompt text (NO plaintext anywhere):
//   "Call OpenRouter with key rollup-ref:secret/openrouter-key@1 and summarize."
// rollup provenance records: secret_refs += openrouter-key@1; warnings += <the warning above>
```
