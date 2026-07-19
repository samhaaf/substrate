# Contract: aws-mesh

## Parties
aws  <->  mesh

## What the edge carries
aws's mesh participation through the **local mesh daemon on `:3649`**
(single-port locality): service registration/resolution (an instance of
`service-lookup`) — plus the round-9 supersession seam: mesh's
eventual-consistency/replication plane **distributes into S3/AWS through
aws's adapter surface** (S3 overflow, cold-storage classes, client-side
encryption with keys held by `secrets`), replacing the S3 adapter that
previously lived inside mesh as an internal lib. KG's cross-boundary S3
sync rides this leg (kg → mesh → aws). Someday: mesh's SQS-modeled queues
deploying onto real SQS flow through here too. Schema/example deferred.
**requirements-only** (round-9 lock, 2026-07-19).
