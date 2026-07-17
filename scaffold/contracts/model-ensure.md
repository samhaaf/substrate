# Contract: model-ensure

## Parties
scheduler  ->  models

## What the edge carries
Ensure a model is downloaded/available before a swap, and surface download-pipeline
status. Today's `download_model` REST path is a no-op stub — the unblock for mesh
Tier-3 routing. Schema deferred.
