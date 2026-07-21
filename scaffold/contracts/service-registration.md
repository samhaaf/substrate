# Contract: service-registration

## Parties
`cc` `<->` `mesh.service-registry` — a named INSTANCE of `service-lookup`.

*(Upgraded at wave-2 harmonization from the rounds-era "Schema deferred"
stub. Deliberately thin: this pair defines NO wire of its own — the schema,
lease/heartbeat/tombstone lifecycle, addressing classes, and error cases are
all `scaffold/contracts/service-lookup.md`. This file exists because the
wave-1 inventory called cc out as a first-class registry participant, and it
records cc's particulars only.)*

## What the edge carries
- cc registers the `cc` slug: `addressing: Singleton`, its real loopback
  `Endpoint` (+ `health_path`), `meta.requires` naming its dependencies
  (rollup, db; inference optional) — supervision derives boot order from
  this record like any other.
- cc resolves its dependencies by slug (`resolve("rollup")`, `resolve("db")`,
  `resolve(AnyNode{inference})` for metering reads) — never a static URL.
- Registration/renewal ride `mesh-client` exactly as every service's do.

## Reconciliation notes
- No party proposed cc-specific wire beyond `service-lookup`'s; the contract
  round left this file untouched (flagged at harmonization as a coverage
  gap, resolved as this pointer contract).
- Candidate for the operator: fold this file into `service-lookup.md`'s party
  list entirely and tombstone the name — kept for now because the wave-1
  inventory names it.
