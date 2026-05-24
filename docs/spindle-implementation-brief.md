# Spindle implementation brief

Spindle is a local-first MCP server for fiction planning, drafting, branching,
revision, and story-bible search. This brief describes the current
implementation shape, the active architectural boundaries, and the remaining
cleanup direction.

## Architecture status

The implementation has a service layer rooted in
`crates/spindle-adapters/src/sqlite/service.rs`, with source-file bridge logic
in `crates/spindle-adapters/src/sqlite/source_bridge.rs`. Validator work is
partially present in code paths such as `check_consistency`. The full
validator surface is still in progress and is not yet fully reflected in
write-tool DTOs like `SaveSceneDraftOutput` and `ReviseSceneOutput`.

For new contributors, start with `docs/spindle-architecture.md` for the current
repo shape.

## Project snapshot

The server name is `spindle`. The implementation target is now a Rust workspace
with `spindle-core`, `spindle-adapters`, `spindle-skills`, `spindle-mcp`, and
`spindle-harness`. The server entry point lives in `spindle-mcp`, and the
default client transport is MCP stdio.

Repository inventory note: this section is intentionally non-exhaustive and
focuses on stable top-level orientation, not a full file listing.

Major roots you should expect:

- workspace crates under `crates/` (`spindle-core`, `spindle-adapters`,
  `spindle-skills`, `spindle-mcp`, `spindle-harness`)
- SQLite migrations under `crates/spindle-adapters/migrations/`
- operator and contributor docs under `docs/`
- skill definitions under root `skills/`
- reference material under `references/`

This brief records the actual implemented workspace and the intended long-term
boundaries.

## Product state

The original v0.1 loop is implemented and still matters: a client can create a
project, assemble scene context, save scene drafts, and persist downstream
state changes. The codebase now also includes post-v0.1 work in several areas.

The implemented surface now includes:

- the original scene drafting loop
- project structure tools for books and chapters
- expanded world entities such as factions, religions, economies, and terms
- plot, conflict, theme, motif, promise, and character-arc records
- pacing configuration, pacing curves, chapter plans, beat annotations, and
  chapter summaries
- consistency checks and semantic search
- public branch creation, switching, diff, merge, and save points
- revision workflows, alternatives workflows, and revision markers
- future knowledge, timeline events, temporal interventions, and system
  overlays
- manuscript import sessions, review queues, hydration, and canonical knowledge
  recording
- embedded craft references and model-route resources
- an optional streamable HTTP MCP transport plus read-only operational HTTP and
  SSE endpoints

## Architecture decisions

The current architecture keeps transport concerns out of the shared crates while
keeping the adapter-layer split in progress under the SQLite service modules.

- `spindle-core` owns shared tool contracts and context DTOs.
- `spindle-adapters` owns concrete database bootstrap, migrations,
  repositories, service implementations, local model routing, embedding logic,
  external model-agent config loading, and embedded guidance access.
- `spindle-skills` owns build-time embedding of root `skills/` and exports the
  embedded skill registry.
- `spindle-mcp` owns `rmcp`, resource URIs, initialize instructions, and thin
  tool handlers, plus an optional streamable HTTP MCP transport and read-only
  operational HTTP and SSE endpoints.
- `spindle-harness` owns operator automation over MCP for batch drafting,
  checkpointing, and resume workflows.
- Root `skills/` is the single source of truth for skill content.
- `crates/spindle-adapters/migrations/` is the source of truth for runtime
  SQLite migration SQL.
- Project resources use opaque IDs under the `bible://projects/{project_id}/`
  prefix.

The adapter extraction is real and storage-driver concerns are isolated.
`spindle-core` no longer owns the concrete database record structs, and it no
longer depends on persistence drivers. DB-shaped records, adapter-only storage
types, search embedding logic, model-routing details, and validator cache
persistence now live in `spindle-adapters`, while `spindle-core` stays focused
on shared tool contracts and public DTOs.

## Current repository layout

The layout evolves as migration and adapter modules are added. Use this stable
mental model instead of a frozen tree dump:

- `crates/spindle-core/src/`: shared models and non-transport contracts.
- `crates/spindle-adapters/src/`: database, repository, and service logic.
- `crates/spindle-mcp/src/`: MCP server/tool/resource wiring.
- `crates/spindle-skills/`: embedded skill asset packaging.
- `crates/spindle-harness/`: CLI harness for long-running MCP workflows.
- `crates/spindle-adapters/migrations/`: ordered SQLite schema migrations.
- `skills/`: root source-of-truth skill prompts.
- `docs/`: architecture plans, implementation notes, and operator references.

## Persistence model

The original v0.1 schema still anchors the system, but later migrations expand
the model significantly.

- The core project, character, location, world-state, world-rule, scene, and
  directed relationship records still exist.
- Internal branch support is now public and includes branch and save-point
  records.
- Story architecture records now cover factions, religions, economies, terms,
  plot lines, conflicts, themes, motifs, narrative promises, and character
  arcs.
- Pacing and planning records now cover pacing configuration, pacing curves,
  pacing trackers, chapter plans, scene beat annotations, and chapter
  summaries.
- Search records now cover embedded search documents and versioned embedding
  state.
- Revision markers persist downstream invalidation produced by scene revision
  workflows.
- Advanced records now cover future knowledge, timeline events, temporal
  interventions, and system overlays.
- Import records now cover sessions, source documents, segments, extracted
  mentions, clusters, dossiers, review items, resume snapshots, and hydration
  reports.
- Canonical knowledge now covers `knowledge_fact` records plus `knows`
  relations, with `future_knowledge` reserved for time-displaced knowledge.

Typed contracts remain preferred over loose JSON blobs, even when the model has
expanded beyond the original v0.1 set.

## MCP surface

The current MCP surface is broader than the original 9-tool contract. The tools
currently exposed by the default server profile fall into these groups.

- session and project catalog: `list_projects`, `set_active_project`
- project structure: `create_project`, `create_book`, `create_chapter`
- branches and revision: `create_branch`, `switch_branch`,
  `create_save_point`, `restore_save_point`, `diff_branches`, `merge_branch`,
  `revise_scene`, `generate_alternatives`, `compare_alternatives`,
  `select_alternative`, `list_revision_markers`, `resolve_revision_marker`,
  `list_scene_versions`, `restore_scene_version`
- world and entities: `create_character`, `create_location`,
  `create_faction`, `create_religion`, `create_economy`, `create_term`,
  `batch_create_terms`, `create_relationship`, `create_world_rule`,
  `update_world_rule`, `set_character_voice_profile`,
  `batch_set_character_voice_profiles`, `update_entity`, `archive_entity`
- plot and arc tracking: `create_plot_line`, `create_conflict`,
  `create_theme`, `create_motif`, `batch_create_motifs`,
  `create_narrative_promise`, `batch_create_narrative_promises`,
  `update_promise_status`, `create_character_arc`, `create_future_knowledge`,
  `create_timeline_event`, `create_temporal_intervention`,
  `create_system_overlay`
- pacing and planning: `create_pacing_config`, `create_pacing_curve`,
  `set_arc_pacing_constraints`, `plan_chapter`, `annotate_scene_beats`,
  `save_summary`, `set_book_outline`, `set_chapter_outline`
- analysis and search: `check_consistency`, `search_bible`,
  `find_scenes_referencing`, `rebuild_search_index`,
  `run_dual_persona_review`
- canonical facts: `register_canonical_fact`,
  `extract_canonical_facts_from_scene`, `migrate_canonical_fact`
- model config and generation: `configure_agents`, `list_agents`, `test_agent`,
  `continue_generation`, `research_query`
- export and preflight: `export_epub`, `preflight_book_export`, `export_bible`
- drafting loop: `get_scene_context`, `get_chapter_briefing`,
  `save_scene_draft`, `move_scene`, `delete_scene`, `operator_delete_scene`,
  `commit_scene_changes`, `commit_character_state`, `update_relationship`
- writer state and lookup: `get_writer_state`, `get_entity`, `find_entity`,
  `get_character_snapshot`, `list_chapter_scenes`, `list_book_chapters`,
  `record_note`, `update_writer_position`
- source bridging: `backfill_scene_source_offsets`, `pull_chapter_from_file`,
  `push_chapter_to_file`
- import and knowledge: `import_manuscript`, `import_status`,
  `import_extract_entities`, `import_consolidate_entities`,
  `import_analyze_character`, `import_extract_world`,
  `import_analyze_narrative`, `import_compute_final_state`,
  `import_hydrate_bible`, `import_apply_review_decisions`,
  `record_knowledge`

## Resource URIs and skill loading

Resource naming still follows the original ID-based direction, but the resource
surface is broader now.

- `bible://skills/{skill-name}` is used for embedded root skills.
- `bible://references/{reference-name}` is used for embedded craft references.
- `bible://system/model-routes` exposes the configured model adapter routes.
- `bible://config/agents` exposes sanitized configured agent state.
- `bible://config/routing` exposes configured route assignments.
- `bible://projects/{project_id}/...` is used for project-scoped resources.
- Nested project resource paths are supported, including paths such as
  `pacing/overview` and `imports/{session_id}/structure`.

Initialize instructions still steer the client toward the main setup and
drafting skills, while the broader skill set remains readable as resources.

## Optional HTTP surface

Spindle also keeps an experimental opt-in HTTP mode. This mode exposes the full
MCP surface over streamable HTTP at `/mcp` and read-only operational endpoints
for lightweight monitoring.

- `SPINDLE_HTTP_ADDR` enables the HTTP server instead of the stdio MCP
  transport for the foreground process.
- `/mcp` serves streamable HTTP MCP clients.
- `GET /health` returns server status, the `/mcp` endpoint, and the read-only
  operational endpoint list.
- `GET /model-routes` returns the current model-route snapshot.
- `GET /events` streams SSE snapshot events for model-route data only.

## Current cleanup direction

The remaining architecture work is not to re-prove the product. It is to finish
reconciling the code and docs around the adapter split and any remaining stale
assumptions.

The highest-value next structural tasks are:

1. finish documenting the five-crate workspace and the current public MCP
   surface
2. deepen the optional HTTP and SSE read surface only if real multi-client use
   emerges
3. keep new skill and reference docs aligned with shipped tool behavior

## Review checklist

Use this checklist when reviewing the implementation against the current code
and docs.

1. `spindle-core` contains no `rmcp` transport code.
2. Root `skills/` remains the source of truth for skill assets, and
   `crates/spindle-adapters/migrations/` remains the source of truth for
   runtime SQLite migration SQL.
3. `spindle-mcp` depends on adapter-layer services rather than concrete core
   infrastructure code.
4. Public docs do not claim the repo is docs-only or limited to the original 9
   tools.
5. Branch-aware fallback works across the expanded read surface that now relies
   on inherited branch state.
6. Validation passes with `cargo fmt --all`, `cargo test --workspace`, and
   `cargo clippy --workspace --all-targets -- -D warnings`.
