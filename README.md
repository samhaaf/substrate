# Substrate

Substrate is a from-scratch rebuild of a personal LLM-serving runtime, built on one
non-negotiable primitive: completion is the only unit of work. No DAGs, no baked-in
agent loops, no tool-calling in the runtime itself — those belong above this layer,
not inside it.

A single scheduler admits completions onto one resident model at a time per machine,
treats resource pressure as backpressure rather than a hard error, and persists every
result in SQLite as the system of record, independent of whatever model weights happen
to be loaded at any given moment.

The design is mesh-network-forward from day one: it runs single-node today, but every
cross-machine concern — routing, load balancing, node discovery over Tailscale — already
exists as a real trait, waiting to be filled in rather than bolted on later.

It's meant to be operated by voice: watched on a live dashboard and driven by spoken
interaction with an AI operator, not by hand-editing code directly.

This repository is under active, early development. Expect the shape of things to keep
changing.
