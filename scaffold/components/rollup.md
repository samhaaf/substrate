# rollup

**Status:** NEW (round-6 lock; **name LOCKED round-9**); **FULL DESIGN — wave 2,
batch 4** (this pass supersedes the prior requirements-only stub). **Nesting:**
top-level L4 data/execution-plane app-crate (`lib/rollup` crate
`substrate-rollup` + `bin/rollup` daemon/CLI). **PORT, not a reinvention:** the
harness reference engine at `~/code/harness/libs/core/src/rollup/` (~2,459 LOC,
67 tests, working — `parser.rs`/`resolver.rs`/`plugin.rs`/`types.rs`) is the v1
core; this design PORTS it onto substrate's layers and ADDS the wave-2
requirements. **Consumes (over the wire / compiled shared libs only — never
cross-app links, INTENT #29):** `vfs` via `rollup-vfs` (fragment/plugin
residence), `secrets` via `rollup-secrets` (secret references, never raw in
output), mesh via `rollup-mesh` (registration + resolve surface), `types` +
`mesh-client` (shared libs, compiled in). **Consumed by:** `ccd` via
`rollup-ccd` (on-demand plugin assembly — PRIMARY consumer), mesh
`queues`/triggers via `rollup-mesh` (declarative payload-assembly references),
`projects` via `projects-rollup` (future — "tasks are rolled up"). Grounded in
the harness engine, secrets.md (concern 4/8, `llm_safe`), queues.md (concern 2,
`types::trigger`'s `Rollup(RollupRef)` + `InsertForm`), vfs.md (two file
classes, `vfs://` namespace), stack.md/db.md (VDB/execution-engine, the
fragments-in-DB v2 path), types.md (`Provenance`, `SurfaceSchema`), and INTENT
#79/#94/#85/#92/#101/#103/#29.

## Charter

`rollup` is the OS's **deterministic composition engine**: it assembles prompts,
files, and whole plugin directories out of small versioned **fragments** that
reference other fragments through a marker syntax, with **slots** taking
variables at reference time, and it does so **on demand — no symlinks, no
file-copying** ("specialized plugins for specialized agents"). It owns the
**generalization ladder string → file → directory rollup** (a plugin is the
directory-rollup case) as one recursive resolver over three target shapes; the
**LOCKED raw-vs-reference insert axis** (INTENT #94 — `Raw` inlines full content,
`Reference` emits a lazily-loadable handle, e.g. a skill pointing at another
skill without inlining it); the **fragment scope/version resolution model**
(walk-up over an ordered scope chain, version pinning, named-ancestor pins,
circular-reference detection — ported verbatim from harness); and — because
provenance is FIRST-ORDER on this plane (INTENT #85/#92) — the **reproducible
build manifest** of every assembled artifact (every fragment id:version, slot,
file, and secret reference that went into the output, traced on the causal
chain). It is **unconditionally secret-safe**: no `secrets` raw value is ever
resolvable into rollup output (concern 4), stronger than a per-sink `llm_safe`
check.

**Boundary — what rollup does NOT own.** It does not own **fragment storage** —
fragments and materialized plugins live in `vfs` (rollup is a VFS tenant, reads
versioned files, optionally writes materialized dirs); it imposes the *layout
convention* but not the bytes. It does not own **secret material or the
use-without-seeing model** — that is `secrets`; rollup only holds and renders
`SecretRef` handles and calls secrets to resolve references (never plaintext into
output). It does not own the **trigger/handler paradigm** — that is `queues`
(L2) and `execution-engine` (L4); rollup is *called by* a trigger's
`AssemblyTemplate` to resolve a `Rollup(RollupRef)` node, it does not own
queues/dispatch. It does not own **plugin execution or agent spawning** — that
is `ccd`; rollup materializes the plugin directory, ccd points a Claude Code
process at it. It does not own the **project/scope hierarchy** — that is
`projects` (L6); v1 rollup takes an explicit ordered scope chain per request
(projects supplies it later). It does not own **the fragments dependency
graph + transactional cascade recompilation** in v1 — that is the flagged v2
evolution (concern 8), which belongs to `vdb`/`execution-engine`. It never
routes anything to an LLM and never *runs* code — rollup output is always
text/files; code lives only in handlers.

## Primary design concerns

Ordered by how much of the design they carry. Concerns 1–3 are the harness PORT
(low risk — working code, 67 tests); 4–8 are the wave-2 ADDITIONS (the design
work).

### 1. The recursive resolver — PORT the harness engine verbatim, behind a store seam

The harness `resolver.rs` is the load-bearing algorithm and ports **unchanged in
logic**: parse a template into `Token`s (`parser.rs`), then recursively expand —
`{{prompt:id}}` loads a versioned fragment and re-parses/expands it;
`{{slot:name}}`/`{{slot:name?"default"}}` substitutes; `{{file:path}}` inlines
verbatim (no expansion); `{{prompt-file:path}}` inlines *with* expansion; slot
maps inherit caller → global → inline-override precedence; cycle detection keys
on the **resolved** `(id, version_number, scope)` triple (so `{{prompt:foo}}` and
`{{prompt:foo:2}}` alias correctly when 2 is latest — the harness's subtle,
already-tested invariant). The **one substrate change**: the harness reads
fragments via `std::fs` against ancestor `.mind/prompts/` dirs; substrate reads
them from **VFS**. Port introduces a `trait FragmentStore` (the queues
`QueueBackend` / secrets `SecretsAdapter` pattern):

```rust
trait FragmentStore {
    fn read(&self, path: &VfsPath) -> Result<Bytes, RollupError>;          // one file's bytes
    fn list_versions(&self, scope: &ScopeId, id: &FragmentId)             // versioned dir listing
        -> Result<Vec<u32>, RollupError>;                                  // integer-named *.md files
    fn exists(&self, path: &VfsPath) -> Result<bool, RollupError>;
}
```

Two impls: `VfsFragmentStore` (production — reads over `rollup-vfs`, concern 6)
and `LocalFragmentStore` (dev/test — the harness `std::fs` path, so the ported
67 tests keep running against tempdirs). The resolver's walk-up, version, slot,
and cycle logic is store-agnostic and copied across near-verbatim. **This is why
rollup is Opus, not Fable: the hard algorithm already exists and is tested.**

### 2. Scope/version resolution — walk-up ported onto an ordered scope chain

Harness walk-up climbs the filesystem ancestor chain (`project → parent → … →
harness`), each level a `.mind/prompts/<id>/<version>.md` dir, most-specific
wins, bare-integer pins strict to the *current project*, named-ancestor pins
(`{{prompt:cfg:harness.3}}`) target a named level. Substrate has no filesystem
ancestry to walk (fragments live in a flat VFS namespace), so the ancestor chain
becomes an **explicit ordered `ScopeChain`** supplied per request:

```rust
struct ScopeChain { scopes: Vec<Scope> }              // ordered most-specific → most-general
struct Scope { id: ScopeId, name: String, prefix: VfsPath }  // e.g. vfs://prompts/<project>/
enum VersionSpec { Latest, Pinned(u32), AncestorLatest(String), AncestorPinned(String, u32) }
```

Resolution maps directly: `Latest` = first scope in the chain that has the
fragment, take its highest version (`FragmentStore::list_versions`); `Pinned(n)`
= strict to `scopes[0]` (the "current project"); `AncestorLatest/Pinned(name,…)`
= the uniquely-named scope (ambiguous-name → the harness `AmbiguousAncestor`
error, ported). A fragment loaded from scope S resolves *its own* inner
`{{file:}}`/`{{prompt:}}` refs relative to S — the harness's
"file-resolves-relative-to-the-containing-prompt's-project" rule (tests 24/25),
ported so an inherited fragment's relative paths never break. **v1 ScopeChain
source:** the caller passes it (ccd knows its plugin's scopes; a trigger's
assembly carries a scope; a CLI user passes `--scope`). **Future:** `projects`
(L6) derives the chain from the project hierarchy (`projects-rollup`) — flagged,
not built now. Fragment layout convention in VFS:
`<scope.prefix>/prompts/<id>/<version>.md` (`<version>` an integer; `latest` =
max).

### 3. Directory rollup = plugin materialization — PORT `plugin.rs`, generate on demand

The generalization ladder (INTENT #79) is **one resolver, three target shapes**:

```rust
enum RollupTarget {
    String(String),                 // string rollup: a template → resolved text
    File(VfsPath),                   // file rollup:   a fragment file → resolved file
    Directory(PluginManifest),       // directory rollup: a manifest → a materialized dir (a PLUGIN)
}
```

`Directory` ports harness `plugin.rs`: a `PluginManifest { name, description,
version, skills, agents, commands }`, each a `FragmentEntry { slug,
slot_overrides, alias_map }`; materialize resolves each entry via
`{{prompt:<slug>}}` (reusing the whole walk-up/version/recursion engine) and
writes `skills/<slug>/SKILL.md`, `agents/<slug>.md`, `commands/<slug>.md`,
`plugin.json` — the exact harness Claude-Code plugin layout. `alias_map`
(external→internal slot names) then `slot_overrides` (win over everything) port
unchanged. **Substrate change:** the output goes to a caller-chosen `OutputSink`
(concern 5) not a fixed dir, and generation is **on demand per request** — no
symlinks, no copying: ccd asks for a plugin, gets a freshly-assembled directory,
uses it, discards it (the fragments are the durable source of truth in VFS).

### 4. Raw-vs-reference insert axis (INTENT #94) + the unconditional secret-safety invariant

The LOCKED addition to the harness markers is an **orthogonal insert form** on
every reference:

```rust
struct RollupRef { target: RefTarget, form: InsertForm, version: VersionSpec, inline_slots: SlotMap }
enum InsertForm { Raw, Reference }                    // Raw = inline content; Reference = emit a handle
enum RefTarget {
    Prompt(FragmentId),      // {{prompt:id}}      — a fragment
    File(VfsPath),           // {{file:path}}       — verbatim file
    PromptFile(VfsPath),     // {{prompt-file:path}}— file, recursively expanded
    Skill(Slug),             // {{skill:slug}}      — a plugin skill (reference-first target)
    Secret(SecretName),      // {{secret:name}}     — resolves through `secrets`, reference-ONLY in output
}
```

- **`Raw` (harness default)** inlines the fully-expanded content — the existing
  `{{prompt:}}`/`{{file:}}`/`{{prompt-file:}}` behavior. Back-compatible: a bare
  marker is `InsertForm::Raw`.
- **`Reference`** emits a stable, lazily-loadable **handle** instead of inlining
  — the operator's "skill file pointing at another skill file so the agent can
  load it if wanted." Syntax: a `ref:` wrapper — `{{ref:prompt:id}}`,
  `{{ref:skill:code-review}}`, `{{ref:file:schemas/tool.json}}`. It renders to a
  canonical locator (`rollup-ref:skill/code-review@latest`) or, for a skill in a
  plugin context, a description-level "See also" pointer the Claude Code Skill
  tool resolves. rollup does NOT load the referenced content; it records it in
  provenance as `referenced_not_inlined`. This is the token-saving,
  prefix-cache-stabilizing path (concern 7).

**The secret-safety invariant (INTENT #94, concern 8, both sides).** A
`RefTarget::Secret` is resolved through `secrets` over `rollup-secrets` — and
**rollup unconditionally requests reference form; it has no code path that inlines
a plaintext secret into output.** A `{{secret:name}}` (nominally `Raw`) is
treated as a raw request in an `llm_safe` context: secrets returns the
`SecretRef` + the operator's warning ("a raw secret was requested; an ID
reference was injected instead"), and rollup emits the reference, never the
value. This is *stronger* than secrets' per-caller `llm_safe` (which permits raw
for non-LLM callers): because rollup output is undifferentiated text that may
flow anywhere downstream, **a raw secret in an assembled document is a disclosure
regardless of sink**, so rollup forecloses it entirely. A handler that genuinely
needs the plaintext does NOT get it via rollup — it holds a `SecretRef` and calls
secrets' `use(ref, sink)` verb directly. Stated as a deliberate deviation in
Friction points so secrets + the operator bless "rollup never resolves raw
secrets, full stop."

### 5. CCD as primary consumer — plugin assembly on demand, into a caller sink

CCD spawns Claude Code processes with a working dir and a plugin directory
("CCD ideally would be built on top of this prompt rollup"). The `rollup-ccd`
surface is **directory rollup with a caller-chosen output**:

```rust
enum OutputSink {
    LocalDir(PathBuf),        // materialize to a real local dir ccd hands to Claude Code (the common case)
    VfsDir(VfsPath),           // materialize into VFS (a durable, shareable plugin)
    Inline,                    // return the file set in the response (small plugins / no disk)
}
```

ccd sends a `PluginManifest` + `runtime_slots` + `ScopeChain` + `OutputSink`;
rollup resolves every fragment (walk-up/version/recursion, VFS-backed) and
materializes. **No symlinks, no file-copying** — each spawn gets a fresh
assembly from the fragment source of truth, so specialized agents get
specialized plugins without a proliferation of copied files. The result carries
the full provenance manifest (concern 9) so ccd can record exactly which
fragment versions an agent ran with (reproducibility + the causal chain into
ccd's usage DB).

### 6. Fragment residence — VFS-backed v1 (`rollup-vfs`); NOT VDB-backed yet

Fragments and materialized plugins live in **`vfs`** (L3), reached over
`rollup-vfs`. Fragments are `Immutable` VFS files (concern 2 of vfs.md) —
content-addressed, dedup'd, replicated — under the layout convention
`<scope>/prompts/<id>/<version>.md`; a materialized `VfsDir` plugin is written as
ordinary VFS files. Version discovery = list the `<id>/` prefix, pick the max
integer (`FragmentStore::list_versions`). **rollup does NOT depend on VDB in v1**
— only VFS + mesh + secrets. This keeps rollup on the boring lower layer and
avoids coupling it to VDB's stack pattern before VDB is solid (VFS < VDB < KG;
rollup needs only the VFS floor).

### 7. Prefix-cache-stability ordering (the polyculture axioms)

Assembled prompts feed inference, whose disk-backed KV/prefix cache is keyed by
`(model_id, prompt_hash)` (`cache.md`) and rewards **long stable prefixes**
shared across invocations. rollup therefore commits to **determinism and
stable-first ordering** as a first-order property, not an accident:

- **Deterministic output.** Fragment resolution follows manifest/marker order,
  never hashmap iteration; any map serialized into output uses `BTreeMap`
  (matching queues' `TemplateNode::Object`); identical inputs → byte-identical
  output. (Slot substitution is already positional in the harness, so `SlotMap`
  being a `HashMap` is safe — order affects only the `slots_filled` provenance
  list, which is `dedup_ordered` by first encounter.)
- **Stable-before-volatile.** A `PluginManifest`/template can mark fragments with
  a `volatility` hint (`Stable | Volatile`); rollup emits stable (invariant,
  cacheable) fragments *before* slot-driven volatile ones where the manifest
  permits, maximizing the shared cache prefix so only the changing tail busts
  the cache. `Reference` inserts (concern 4) reinforce this: a referenced (not
  inlined) skill keeps volatile bulk *out* of the prefix entirely.
- **Cache-key surface.** `Resolved` exposes a `stable_prefix_len` hint so
  inference/ccd can reason about the cacheable boundary. Advisory, not enforced.

### 8. Fragments-in-database evolution — RECOMMEND file-based v1, VDB-backed v2 (FLAGGED)

The Broomstick design's **fragment dependency graph + transactional cascade
recompilation** (edit fragment B → every dependent fragment recompiles in one
transaction, provenance-traced) is the natural end state. The question the brief
poses: file-based-on-VFS v1 or VDB-backed now?

**Recommendation: file-based on VFS for v1 (the harness port); the
dependency-graph + cascade recompilation as v2, VDB-backed.** Rationale:

1. **v1 doesn't need it.** rollup is **pull / resolve-on-demand** (ccd assembles
   a plugin when it needs one; a trigger resolves at assembly time). With lazy
   resolution, "recompilation" is just *re-resolving at read time* — always
   fresh, no cache to invalidate. The dependency graph is a *precompilation
   optimization* (a materialized cache + push invalidation), not a correctness
   requirement.
2. **The cascade IS the stack pattern.** "On fragment change → recompile
   dependents → trace provenance" is precisely `execution-engine`'s
   trigger/handler paradigm on a `fragments` table with a `depends_on` edge set —
   a textbook VDB application. It belongs *on* VDB, so it can only exist cleanly
   once VDB + execution-engine are solid (batch-4 siblings, both Fable). Building
   it now would either duplicate execution-engine or couple rollup to an
   unfinished VDB.
3. **Layering.** rollup at L4 already sits on VFS (L3); a VDB-backed fragment
   store is *additive* — a second `FragmentStore` impl (`VdbFragmentStore`) plus
   an execution-engine trigger, behind the seam concern 1 already defines. The v1
   port loses nothing; v2 slots in without a rewrite.

**v2 sketch (for the flag):** fragments become rows in a VDB stack database
(`fragments(id, version, scope, body, depends_on[])`); an execution-engine
handler on `fragments` change walks the dependency graph and recompiles the
precompiled-output cache transactionally (copy/verify/switch-under-a-lock, the
same VDB promotion mechanics), emitting healthcare-grade provenance per
recompilation. rollup's read path checks the precompiled cache first, falls back
to resolve-on-demand. **Flagged for the operator** — recommendation is
file-based v1, and I do not build the VDB path this pass.

### 9. Provenance FIRST-ORDER — the reproducible build manifest (INTENT #85/#92)

This plane is provenance's home, and rollup's provenance is the **bill of
materials of every assembled artifact**. The harness already returns
`Resolved { fragments_used, slots_filled, files_included }`; substrate ELEVATES
this to a first-order `types::Provenance`-carrying record:

```rust
struct RollupProvenance {
    fragments_used: Vec<FragmentUse>,        // (id, version, scope, vfs_path, content_hash) — dedup, ordered
    slots_filled: Vec<String>,               // ordered by first encounter
    files_included: Vec<String>,
    secret_refs: Vec<SecretRef>,             // references touched — NEVER values
    referenced_not_inlined: Vec<RollupRef>,  // the InsertForm::Reference handles
    warnings: Vec<String>,                   // e.g. degraded-secret, unresolved-optional
    trace: Provenance,                       // types::Provenance — causation/correlation for the causal chain
}
```

When rollup is called from a trigger's assembly (`rollup-mesh`), it inherits the
event's `correlation_id` so the assembled payload's provenance joins the same
causal chain execution-engine's loop-detection and VDB's traces ride (INTENT
#70/#85). Because fragments are content-addressed in VFS, `content_hash` per
fragment makes an assembled artifact **exactly reproducible** — you can
re-materialize the identical plugin an agent ran with. The manifest never
contains a secret value (only `SecretRef`s).

### 10. Service shape — daemon + CLI, dual-role (crate=app, INTENT #29)

rollup is consumed by *apps* (ccd) and by *mesh* (queues/triggers) that cannot
link it (INTENT #29: no cross-app in-process linking). So rollup is a **daemon
registering on the local mesh** (single-port locality, `:3649`, via
`mesh-client`), serving `rollup-mesh`/`rollup-ccd` over WS — and **also a
noun-verb `clap` CLI** for human/local use, exactly like `db`/`secrets`/`gc`/
`ccd`:

```
rollup resolve <template|--file P>  --scope <chain> [--slot k=v]...  # string/file rollup → stdout
rollup materialize <manifest.json>  --scope <chain> --out <dir|vfs:path> [--slot k=v]...  # directory rollup (a plugin)
rollup fragments ls    --scope <chain>            # list fragment ids/versions in scope
rollup fragments show  <id[:version]> --scope <chain>
rollup provenance <artifact>                       # print the bill of materials of a prior rollup
```

The engine is `lib/rollup` (`substrate-rollup`, the harness port + the wave-2
additions); `bin/rollup` is the thin daemon + CLI. Participates in
`restart-protocol` and `pubsub-protocol` like every service, and publishes a
`SurfaceSchema` (fragment counts per scope, recent assemblies, cache-hit stats)
so the boring dashboard renders it — the schema exposes no fragment *bodies* and
no secret surface. Stateless request/response (fragments are in VFS), so rollup
is `AddressingClass::AnyNode` — any node's daemon can serve any resolve, which is
what lets ccd on any machine assemble a plugin from the mesh-replicated
fragments.

## Relationships / edges

Contract edges are cross-process WS through the local `:3649` daemon (INTENT
#29/#58). `types` and `mesh-client` are compiled-in shared libs, NOT contract
edges.

- **ccd** via `rollup-ccd` — CCD's on-demand plugin/prompt assembly (directory
  rollup, `OutputSink`), the PRIMARY consumer; ccd is the consumer, rollup
  authors this side. *(scaffold/contracts/rollup-ccd.md — exists,
  content authored at the contract round.)*
- **mesh (queues/triggers)** via `rollup-mesh` — rollup's registration
  (`service-lookup` instance) + the resolve surface a trigger's `AssemblyTemplate`
  calls to resolve `Rollup(RollupRef)` nodes (queues is the consumer; queues.md
  proposed the consumer view `ResolveReferences`). rollup authors this contract.
  *(authored at the contract round.)*
- **secrets** via `rollup-secrets` — rollup resolves `RefTarget::Secret`
  references through secrets; unconditionally reference-only in output; the
  no-secret-in-output invariant (concern 4/8). rollup is the consumer; secrets.md
  authored its side. *(authored at the contract round.)*
- **vfs** via `rollup-vfs` — fragment/plugin residence: read versioned fragment
  files, list versions, write materialized `VfsDir` plugins. rollup is the
  consumer of vfs's file surface. *(authored at the contract round.)*
- **projects** via `projects-rollup` — the migrated `.mind` workspace schema
  brings rollup into projects ("tasks are rolled up"); projects supplies the
  `ScopeChain` from the project hierarchy. *(stub-track anticipated — named below,
  content deferred; projects is an L6 stub.)*
- **`types`** — shared lib, NOT a contract edge: `types::rollup` (RollupRef,
  InsertForm, RefTarget, VersionSpec, RollupTarget, PluginManifest, FragmentEntry,
  ScopeChain, Resolved, RollupProvenance, MaterializeResult) whose shape this
  design authors, plus existing `Provenance`/`SurfaceSchema`. `RollupRef` +
  `InsertForm` are already referenced by `types::trigger` (queues) and by
  secrets' `rollup-secrets` — confirming the `types` placement (declarative data
  used by ≥2 crates).
- **`execution-engine`** (batch-4 sibling) — NOT a v1 edge and NOT a contract:
  the v2 fragments-in-DB cascade (concern 8) would ride execution-engine's
  trigger/handler paradigm on a VDB `fragments` table. Flagged, not built.
- **`secrets`/`vfs`/`mesh`** are all reached over the wire; rollup links none of
  them (INTENT #29).

## Nesting

Parent: none | Children: none. Dual-role crate like `db`/`secrets`/`gc`/`ccd`:
`lib/rollup` (`substrate-rollup` — the ported engine: `parser`, `resolver`,
`plugin`, plus new `store` (FragmentStore seam), `refs` (raw-vs-reference),
`provenance`) + `bin/rollup` (daemon + `clap` CLI). `FragmentStore` /
`OutputSink` are internal module seams, not sub-crates.

## Thoroughness level

**implementation-ready** for: the ported resolver (parser/walk-up/version/
recursion/cycle-detection — working harness code + 67 tests), the `FragmentStore`
seam (VFS + Local impls), the ScopeChain resolution model, the directory-rollup
/ plugin materialization port, the raw-vs-reference insert axis with the
unconditional secret-safety invariant, the CCD `OutputSink` assembly surface,
the provenance build-manifest, prefix-cache-stability ordering, and the
daemon+CLI service shape. **approach-sketched** for: the `Reference` rendering of
non-secret targets (canonical locator vs description-level "See also" — fill-time
choice, both specified), and the `projects-rollup` scope-hierarchy source (v1
explicit chain, projects-derived later). **recommendation-flagged** (operator's
call): the fragments-in-DB v2 (file-based v1 recommended; VDB-backed v2 sketched,
not built) and the "rollup never resolves raw secrets, full stop" deviation from
secrets' per-sink `llm_safe`.

## Assigned design-depth

Opus (single Component-Designer pass, this file), grounded in the harness
reference engine (`~/code/harness/libs/core/src/rollup/`, read in full),
secrets.md, queues.md, vfs.md, stack.md, db.md, projects.md, types.md, and INTENT
#79/#94/#85/#92/#101/#103/#29. No Fable step needed — the hard algorithm is
ported from tested prior art; the wave-2 additions are boring axes on top.

## Suggested fill-model

**implementation-ready + moderate complexity → mid model, PORT-FIRST.** The
resolver/parser/plugin are a near-mechanical transcription of the harness code
(carry over its 67 tests against `LocalFragmentStore` first — they must pass
unchanged before any substrate wiring). The wave-2 additions split by risk: the
raw-vs-reference axis + provenance manifest are boring additive data → mid model;
the **secret-safety invariant** (concern 4/8) is the one correctness-critical
spot — it must be conformance-tested (a `{{secret:}}` raw request NEVER yields a
value in output, under every target/sink) before ship, same discipline secrets
applied to its crypto core. Do not gold-plate the v2 fragments-in-DB path — it is
explicitly deferred.

---

## Contracts (wave 2 — authored)

The per-pair contract round authored these edges; the contract files are
authoritative (including their Reconciliation notes). The detailed proposals
formerly in this section are superseded by the authored contracts.

- `rollup-mesh` — (rollup ↔ mesh; rollup authors) — registration (`AnyNode` `service-lookup` instance) + the resolve surface callers invoke; the operation is canonically `ResolveRefs` (queues' `ResolveReferences` recorded as the alias). → `scaffold/contracts/rollup-mesh.md`
- `rollup-ccd` — (ccd → rollup; rollup authors, ccd is the primary consumer) — on-demand plugin assembly; `AssemblePlugin` and `rollup-mesh`'s `Materialize` are ONE operation, one implementation. → `scaffold/contracts/rollup-ccd.md`
- `rollup-secrets` — (rollup → secrets) — reference-only secret resolution (consumer side). → `scaffold/contracts/rollup-secrets.md`
- `rollup-vfs` — (rollup → vfs) — fragment/plugin residence (consumer side). → `scaffold/contracts/rollup-vfs.md`
- `projects-rollup` — (projects → rollup) — stub-track (anticipated, content deferred). → `scaffold/contracts/projects-rollup.md`

Also a party to (authored elsewhere / cross-cutting): `pubsub-protocol`, `repo-vfs`, `service-lookup` — see `scaffold/contracts/`.

## Non-obvious tests (conformance + correctness)

- **Harness parity (PORT gate):** the 67 ported harness tests
  (parser/resolver/plugin) pass unchanged against `LocalFragmentStore` — walk-up,
  latest-version-by-integer, pinned-strict-to-current-scope, named-ancestor,
  ambiguous-ancestor, recursive expansion, inline-slot precedence, the
  file-resolves-relative-to-containing-scope rule (harness tests 24/25), and the
  Latest-vs-Pinned cycle-key aliasing (harness test 29) — all green before any
  substrate wiring.
- **Secret-safety invariant (the critical one):** a `{{secret:api-key}}` (raw)
  marker in a `String`, a `File`, and a `Directory`(plugin skill) target ALL
  yield `DegradedToReference` in output — the plaintext appears in NONE of them,
  under every `OutputSink`; provenance carries the warning + the `SecretRef`,
  never a value. A `{{ref:secret:api-key}}` yields the reference directly. No code
  path returns a secret value into `Resolved.text`.
- **Reference vs raw:** `{{prompt:big}}` inlines big's expanded body;
  `{{ref:prompt:big}}` (or `{{ref:skill:big}}`) emits only a handle,
  `referenced_not_inlined` records it, and `Resolved.text` does NOT contain big's
  body (token-saving lazy load).
- **VFS-backed resolution matches Local:** the same fragment tree resolved via
  `VfsFragmentStore` (over `rollup-vfs`) and `LocalFragmentStore` produces
  byte-identical `Resolved.text` and equivalent provenance (modulo `vfs_path`).
- **Determinism / prefix-cache stability:** identical `(manifest, slots, scope)`
  → byte-identical output across runs (BTreeMap serialization, ordered
  resolution); a stable-marked fragment precedes a volatile slot-driven one, and
  `stable_prefix_len` marks the shared boundary; changing only a volatile slot
  leaves the stable prefix byte-identical.
- **On-demand plugin assembly (ccd):** `AssemblePlugin` with a `PluginManifest`
  → the Claude-Code layout (`skills/<slug>/SKILL.md`, `agents/<slug>.md`,
  `commands/<slug>.md`, `plugin.json`) materializes with correct `alias_map` →
  `slot_overrides` precedence (ported harness plugin tests); a second identical
  request re-materializes fresh (no symlink/copy shared state).
- **Provenance reproducibility:** two assemblies of the same manifest at the same
  fragment versions produce identical `content_hash` sets in `fragments_used`;
  re-materializing from the recorded manifest reproduces the identical artifact.
- **Trigger-assembly path (queues co-design):** a queues `Rollup(RollupRef)`
  assembly node resolves over `rollup-mesh::ResolveRefs` binding the event subject
  into fragment slots; a `RefTarget::Secret` in an LLM-classified handler's
  assembly degrades (no secret in the assembled payload — INTENT #94); a missing
  fragment surfaces as `AssemblyFailed` on the queues side, never a panic.
- **Mixed-version tolerance:** a rollup daemon receiving a `RollupRef` with an
  unknown `RefTarget`/`InsertForm` variant (`#[serde(other)]`) fails *that
  reference* loudly and locally without crashing the resolve or dropping sibling
  fragments.
- **Circular-reference across scopes:** a fragment in scope A referencing a
  fragment in scope B that references back into A is detected (resolved
  `(id,version,scope)` cycle key), erroring `CircularReference` — not infinite
  recursion, not a false positive on Latest-vs-Pinned aliasing.
