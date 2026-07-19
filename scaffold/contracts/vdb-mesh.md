# Contract: vdb-mesh

## Parties
`vdb` (L4 daemon, `AddressingClass::NodeScoped`, one leg per database-hosting
node) `<->` mesh (local daemon `:3649`).

**Renamed from `stack-mesh` at wave-2 harmonization** (wave2-plan §5.1 flag;
vdb.md's naming resolution: `vdb` is the crate/module, "stack" survives as
the PATTERN name). `scaffold/contracts/stack-mesh.md` is a rename-tombstone
pointing here. The rounds-4–5 requirements-only stub was never upgraded by
the contract round (a coverage gap); this file is the authored minimal
contract, from `components/vdb.md` §`vdb-mesh`.

## Purpose
vdb's mesh participation — three facets over the local daemon:

- **(a) Registration.** The daemon registers `vdb` (NodeScoped) AND one
  virtual supervised entity per hosted database (`vdb/<project>/<name>` →
  daemon endpoint + db route) — ordinary `service-lookup` instances. Entity
  registrations carry a `ServiceManifest` whose `version` is the
  stack-definition version, so **supervision inherits databases with zero new
  protocol** ("treat databases like services under mesh's restart/upgrade
  protocol", INTENT #86).
- **(b) Catalog tenancy.** The `vdb/` replicated-kv keyspace
  (`DatabaseRecord`/def-ref rows), reached through the same mesh-brokered
  keyspace surface `secrets-mesh` established (vdb is an L4 app; it cannot
  link `KvHandle`).
- **(c) Locks usage.** The promotion mutex and event-ID semaphores via
  `locks-api` (consumed, not re-authored). NOTE: the old stub's
  "distributed-handler coordination" framing is REDUCED per vdb.md concern 6
  — an intentional scope correction.

## Schema (summary — authoritative sketch in vdb.md)
No new wire beyond composition: `Registration` (service-lookup) with
`meta.requires: ["db"]` and per-entity `meta: { db_id, target, definition }`;
`VdbKvPut/Get/Scan/Watch` mirroring `secrets-mesh`'s brokered-keyspace verbs
over keyspace `vdb/` (`mirror_values: true` — nothing sensitive);
restart-protocol frames per entity (`types::restart`, unchanged — the entity
slug rides the frame's addressing).

## Error cases
Registry/lease errors are `service-lookup`'s; keyspace errors wrap `KvError`
(incl. `PartialReplication` surfaced on catalog writes);
`KeyspaceAccessDenied` (only the registered `vdb` slug touches `vdb/`);
locks errors are `locks-api`'s (`Contended`, `LeaseExpired`,
`PartitionMergeThresholdExceeded` — promotion aborts on the latter).

## Version sensitivity
HIGH — `DatabaseRecord` persists in replicated-kv across mixed-version
fleets: full types-guardrail-4 discipline; `StackTargetKind`/`DbStatus`/
`ProvLevel` reserve `#[serde(other)]`; a daemon seeing an Unknown target kind
treats the database as *present but not hostable here* (never fires, never
deletes — cron's Unknown-schedule fail-safe rule, reused).

## Reconciliation notes
- The per-entity registration shape is an *additive facet* of
  `service-lookup`'s payload — same route supervision's manifest facet took;
  flagged to service-registry.
- The brokered-keyspace pattern generalizes exactly as secrets' design
  predicted (second tenant).
- The `stack-mesh`→`vdb-mesh` rename is performed here per vdb.md's
  recommendation; operator sign-off pending in the friction report.
