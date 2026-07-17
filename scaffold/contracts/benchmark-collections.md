# Contract: benchmark-collections

## Parties
benchmark  ->  scheduler

## What the edge carries
Priority-0, `request_full_system` sweep collections submitted through the
scheduler/queue, preemptible by any priority>=1 work, for isolated throughput
characterization across the (output x parallelism x input) grid. Schema deferred.
