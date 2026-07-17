# Contract: org-on-ccd

## Parties
org (maximalist consumer)  ->  ccd  (+ broad reads of inference / system-state /
service-registry / db)

## What the edge carries
Org is built on top of CCD for inter-agent communication; this edge bundles that
dependency AND Org's shaped-for maximalist reads across the other services. NOT
designed this pass (Org is the exception) — recorded so the neighbor contracts
(`agent-management`, `v1-completion-api`, `service-lookup`, `db-control-plane`) are
shaped for a broad consumer from the start. Schema deferred / intentionally open.
