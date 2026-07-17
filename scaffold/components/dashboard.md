# dashboard

**Status:** existing (`ui/dashboard`), RESHAPE (gains node dimension). **Nesting:**
top-level.

The Svelte/Vite frontend. Renders live disk/GPU/RAM, GC tree, benchmark/kernel
panels, and the completion queue by filtering the gateway's `node_id`-tagged event
envelopes. In v2 it gains a node dimension: a per-node panel grid plus a
fleet-summary header, achieved by filtering existing tagged envelopes with no new
WS plumbing. Carries stable kebab-case element `id`s for agent-driving. Consumes
`dashboard-feed` only.
