# Spindle architecture quick start

This guide gives you the fastest accurate path into the current Spindle code
architecture. Read this first if you are new to the repository, then jump to
deep plan docs only when you need historical context.

## What Spindle is

Spindle is a local-first MCP server for story planning, drafting, revision,
and continuity validation. The architecture centers on one rule: writing
tools must return actionable validation output without leaking storage or
transport concerns across crate boundaries.

## Core framing

Spindle treats continuity checks as a truth-oracle workflow:

1. Capture typed world and story facts in persistent records.
2. Build scene-scoped context from branch-aware data.
3. Run validator passes against scene text and metadata.
4. Surface findings directly in write-tool responses and consistency scans.
5. Expand deterministic validator coverage as those interfaces land.

This framing replaces older "best effort linting" assumptions.

## Six architecture pillars

The current architecture is easiest to reason about as six pillars.

1. Typed data over ad hoc blobs.
2. Branch-aware reads and writes as default behavior.
3. Validator framework rollout across shared context and finding contracts.
4. Thin transport layer over service-level business logic.
5. Explicit invalidation for continuity and consistency-related caches.
6. Documentation and skill prompts that match shipped interfaces.

## Workspace map

The workspace has five active crates with clear ownership boundaries.

- `crates/spindle-core`: shared models and validator contracts.
- `crates/spindle-adapters`: repository + services + model routing + guidance.
- `crates/spindle-skills`: embedded `skills/` asset packaging.
- `crates/spindle-mcp`: MCP tool/resource wiring and process entrypoint.
- `crates/spindle-harness`: operator automation over MCP for batch drafting,
  checkpoints, and resume workflows.

## Hexagonal Shape

Spindle uses a pragmatic hexagonal split. The dependency direction should be:

```text
spindle-mcp / spindle-harness -> spindle-adapters -> spindle-core
spindle-adapters -> spindle-skills
```

`spindle-core` owns public DTOs, domain value types, context-bundle contracts,
subject snapshots, and validator contracts. It must not depend on MCP,
SQLite, model-agent runtimes, filesystem layout, or embedded asset plumbing.

`spindle-adapters` owns outbound and application-service concerns: SQLite
persistence, import/export persistence, model routing, embeddings, guidance
lookup, and orchestration that combines repositories with pure formatting or
validation helpers.

`spindle-mcp` is an inbound adapter. It should parse MCP arguments, apply
session/defaulting and mutation-serialization policy, invoke service methods,
convert errors into MCP results, and expose resource/tool schemas. It should
not own story business rules, persistence query details, or public story DTOs.

`spindle-harness` is another inbound adapter, but as an operator/client over
the public MCP surface. It should not reach into repositories or SQLite.

`spindle-skills` is static embedded asset packaging only.

## Boundary Audit

Current audit findings:

- No reverse dependency from `spindle-core` to adapters, MCP, SQLite, or
  runtime model code was found.
- `spindle-mcp` previously owned several public tool/envelope DTOs
  (`set_active_project`, `init_grok_skills`, `get_writer_state`, and
  `get_scene_context`). These contracts now live in `spindle-core`.
- `spindle-mcp` previously assembled writer-state and scene-context envelopes,
  including markdown rendering and standards insertion. That presentation
  assembly now lives behind `SqliteSpindleService`; MCP only invokes the
  service and serializes the result.
- MCP still owns session defaulting, tool schema sanitization, lock scoping,
  and Grok skill-file installation. Those are transport/client-adapter
  concerns and should stay out of core unless they become reusable public
  contracts.

The service layer currently lives under:

- `crates/spindle-adapters/src/sqlite/service.rs`
- `crates/spindle-adapters/src/sqlite/project_resources.rs`
- `crates/spindle-adapters/src/sqlite/source_bridge.rs`
- `crates/spindle-adapters/src/format.rs`

`service.rs` remains the main application-service entry point. Supporting
modules keep cohesive adapter concerns out of that file: `project_resources.rs`
owns `read_project_resource` pagination envelopes and record-to-JSON resource
projection helpers; `source_bridge.rs` owns external source lookups; and
`format.rs` owns pure presentation formatting shared by service methods.

Architecture-sensitive boundaries are enforced by
`crates/spindle-core/tests/architecture_boundaries.rs`. The guard test checks
that:

- `spindle-core` does not import adapters, MCP, SQLite, or skill packaging.
- `spindle-skills` stays static asset packaging only.
- `spindle-harness` does not import SQLite service/repository internals.
- `spindle-mcp` does not define public `*Input`, `*Output`, or `*Envelope`
  DTO structs.

Run the focused guard with:

```bash
cargo test -p spindle-core --test architecture_boundaries
```

It is also covered by the normal workspace test command.

## Validator architecture (in progress)

Continuity behavior is centered in service-layer checks and consistency
tooling. Validator-oriented plumbing exists in adapter services, but do not
assume the full target validator DTO and output surface is available on every
write tool response yet.

SQLite Phase-4 validator cache rows live in `validator_finding`. Cache hits are
keyed by branch, scene, `validator_id`, `scene_text_hash`, and a validator
context hash derived from the relevant branch-scoped facts, rules, characters,
timeline events, temporal interventions, and style context. Service write paths
that alter canon, voice, style, or timeline state must resolve the matching
validator cache id, and read paths must use explicit branch-aware repository
queries. The continuity health resource
`bible://projects/{project_id}/continuity/health` exposes open findings,
resolved cache rows, branch lineage, orphaned temporal interventions, and
duplicate active canonical-fact keys for operational checks.

## Scene write path

The normal write path keeps generation, persistence, and validation separate
but connected:

1. Client calls scene-writing tools (`save_scene_draft`, `revise_scene`,
   `commit_scene_changes`).
2. Services persist branch-local scene updates through repository methods.
3. Services derive warnings, diffs, and consistency-related results.
4. Tool responses return scene IDs and currently shipped response fields from
   `crates/spindle-core/src/models.rs`.

`check_consistency` remains the broad audit surface for branch-scoped
consistency review.

## Canonical fact model in practice

Canonical facts are active architecture work. Treat the canonical-fact
validator flow as transitional rather than final.

- Any schema/tool change that affects facts must update docs and tests in the
  same change.

## Voice drift and retcon checks

Voice-drift and retcon/reachability checks are planned continuity domains.
Treat these as target architecture unless your current branch explicitly ships
the corresponding DTOs and tool outputs.

## Where to make changes

Use this map when you need to patch behavior quickly.

- Add or modify tool DTOs: `crates/spindle-core/src/models.rs`.
- Add or modify continuity and validator-facing DTOs:
  `crates/spindle-core/src/models.rs`.
- Add or modify MCP session/defaulting/schema behavior:
  `crates/spindle-mcp/src/tools.rs`.
- Change repository persistence/query behavior:
  `crates/spindle-adapters/src/sqlite/repository.rs`.
- Change business logic or tool orchestration:
  `crates/spindle-adapters/src/sqlite/service.rs`.
- Change MCP tool/resource exposure:
  `crates/spindle-mcp/src/tools.rs` and
  `crates/spindle-mcp/src/resources.rs`.

## Contributor guardrails

When you change architecture-sensitive code, keep these guardrails.

1. Do not move transport logic into `spindle-core`.
2. Do not bypass service-layer invariants from MCP handlers.
3. Put public tool/resource DTOs in `spindle-core`, not in `spindle-mcp`.
4. Keep validator findings structured and cache-aware.
5. Update docs and skill prompts when public behavior changes.
6. Run `cargo test -p spindle-core --test architecture_boundaries` after
   crate-boundary or DTO-placement changes.
7. Prefer branch-aware queries and avoid hidden project-wide scans unless
   explicitly required.

## First-hour onboarding checklist

Run this sequence when you start a new implementation task.

1. Read `README.md` for runtime and test commands.
2. Read this file end to end.
3. Locate affected DTOs in `spindle-core` and service paths in
   `spindle-adapters`.
4. Confirm any tool-surface impact in `spindle-mcp`.
5. Run targeted tests before and after edits.

## Related docs

- `docs/spindle-implementation-brief.md`: broader implementation snapshot.
