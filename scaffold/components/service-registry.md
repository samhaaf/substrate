# service-registry

**Status:** NEW. **Nesting:** child of mesh. **Also: the single wiring seam.**

A distributed, eventually-consistent configuration store spanning all devices —
the operator: "store a distributed config across all the devices with eventual
consistency, allowing us to register services on different devices at different
ports and reference the service by a slug, so we can query the mesh to find where
a service is being rendered and access it." Exposes `service-lookup`
(register(slug, host:port) / resolve(slug) -> endpoint) to any device and
replicates between peer instances (`registry-replication`). The hardest new
sub-problem (eventual consistency + conflict handling), which is why it is its own
child. It IS the system's assembly seam: services register + resolve here instead
of using static URLs (see overview "single wiring seam").
