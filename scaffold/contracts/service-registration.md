# Contract: service-registration

## Parties
ccd  <->  mesh.service-registry (via service-lookup)

## What the edge carries
CCD registers its own `slug -> host:port` and resolves other services by slug.
Instance of `service-lookup` for the CCD party; called out separately because CCD
is a first-class registry participant (both registrant and resolver). Schema
deferred.
