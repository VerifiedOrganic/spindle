---
name: bible-librarian
description: >
  Use when browsing, searching, querying, summarizing, or exporting any aspect of the Story Bible:
  character briefs, relationship webs, state-at-point lookups, chapter recaps, pacing dashboards,
  research library queries, and spoiler-bounded reader exports.
---

# Bible Librarian

Navigate the Story Bible, resolve entity IDs without drift, assemble character and chapter briefings, and generate spoiler-bounded reader artifacts.

## Lookup Order

1. Call `set_active_project` once per session to bind the active project and branch (`list_projects` when the project ID is unknown).
2. Call `get_writer_state` when answering "where am I?" or "what's open?" (`writer_position`, `open_promises_due_now`, `recent_session_activity`, `active_overlays`).
3. Call `find_entity` to resolve a name, alias, or fuzzy phrase to its canonical `table` and `id`.
4. Call `get_entity` (`table`, `entity_id`) to read the canonical subject snapshot (or read `bible://{table}:{id}`).
5. For character deep reads, call `get_character_snapshot` after `get_entity` to retrieve voice profile, latest state, recent appearances, and active relationships in one call.
6. Call `find_scenes_referencing` (`query: { kind: "subject", subject_id }` or `{ kind: "phrase", phrase }`) for scene-level evidence, and `list_book_chapters` / `list_chapter_scenes` for spine navigation.
7. Call `get_chapter_briefing` for a structured chapter recap (POV, summary, beats, continuity sheets) or `get_scene_context` for branch-aware state at a specific `(book_number, chapter_number, scene_order)`.
8. Use `search_bible` (`mode: "semantic" | "exact" | "fuzzy"`, `field: "name" | "content" | "tags"`, optional `subject_table`, `format`, `budget_tokens`) for indexed canon retrieval.

## Resource vs. Tool Separation

- **Resources (`bible://...`)** provide cacheable reads for project entity lists (`characters`, `locations`, `factions`, `religions`, `economies`, `terms`, `relationships`, `world-rules`, `plot-lines`, `conflicts`, `themes`, `motifs`, `narrative-promises`, `character-arcs`, `future-knowledge`, `timeline-events`, `timeline-graph/mermaid`, `temporal-interventions`, `system-overlays`, `pacing/overview`, `branches`, `continuity/health`, `reader-contract`, `chapter-summaries`, `imports`, `research`, `research/tags`), direct records (`bible://{table}:{id}`), and config (`bible://config/agents` / `list_agents`, `bible://system/model-routes`).
- **Tools** handle state mutations (`create_character`, `create_location`, `create_faction`, `create_religion`, `create_economy`, `create_term`, `create_relationship`, `create_world_rule`, `create_plot_line`, `create_conflict`, `create_theme`, `create_motif`, `create_narrative_promise`, `update_promise_status`, `create_character_arc`, `create_future_knowledge`, `create_timeline_event`, `create_temporal_intervention`, `create_system_overlay`, `create_pacing_config`, `set_arc_pacing_constraints`, `create_branch`, `switch_branch`, `import_manuscript`, `register_canonical_fact`, `record_knowledge`), parameterized searches, and dynamic computations (`list_revision_markers`).

## Chapter Summaries and Spoiler-Bounded Reader Artifacts

- **Chapter summaries**: Read `bible://projects/{project_id}/chapter-summaries` and call `save_summary` (`entity_type: "chapter"`, `entity_id` or `book_number` + `chapter_number`) for chapters missing a saved summary.
- **Spoiler-bounded reader exports** (pure read models with optional `write_to_workspace`):
  - `export_recap`: "Previously on" recap of one book through `through_chapter` (story-so-far, paid-off promises, open questions).
  - `export_series_bible`: Reader-facing series bible bounded at optional `through` placement (character pages, locations, glossary, factions/religions).
  - **Spoiler & secrecy rule**: Only facts established at or before the cursor are included. Secret canonical facts (`register_canonical_fact` with `secrecy`) remain hidden from reader artifacts until a dated, reader-visible reveal is recorded via `record_knowledge` with `secret_of_fact_id`, `reader_visible: true`, and `learned_at` at or before the cursor.
- **Full manuscript assembly**: Call `compile_manuscript` to assemble ordered scene prose.

## Research Library

Store and query grounding material with `research_add_source`, `research_add_note`, `research_add_claim`, `research_search`, and `research_pack_for_scene` (and browse `bible://projects/{project_id}/research` and `/research/tags`).

## Shared Subagent Convention

Across Spindle skills, subagents may fan out independent read-only lookups and verification, while every state-mutating tool call stays in the main context.

<!-- tool: compile_manuscript -->
<!-- tool: create_branch -->
<!-- tool: create_character -->
<!-- tool: create_character_arc -->
<!-- tool: create_conflict -->
<!-- tool: create_economy -->
<!-- tool: create_faction -->
<!-- tool: create_future_knowledge -->
<!-- tool: create_location -->
<!-- tool: create_motif -->
<!-- tool: create_narrative_promise -->
<!-- tool: create_pacing_config -->
<!-- tool: create_plot_line -->
<!-- tool: create_relationship -->
<!-- tool: create_religion -->
<!-- tool: create_system_overlay -->
<!-- tool: create_temporal_intervention -->
<!-- tool: create_term -->
<!-- tool: create_theme -->
<!-- tool: create_timeline_event -->
<!-- tool: create_world_rule -->
<!-- tool: export_recap -->
<!-- tool: export_series_bible -->
<!-- tool: find_entity -->
<!-- tool: find_scenes_referencing -->
<!-- tool: get_chapter_briefing -->
<!-- tool: get_character_snapshot -->
<!-- tool: get_entity -->
<!-- tool: get_scene_context -->
<!-- tool: get_writer_state -->
<!-- tool: import_manuscript -->
<!-- tool: list_agents -->
<!-- tool: list_book_chapters -->
<!-- tool: list_chapter_scenes -->
<!-- tool: list_projects -->
<!-- tool: list_revision_markers -->
<!-- tool: record_knowledge -->
<!-- tool: register_canonical_fact -->
<!-- tool: research_add_claim -->
<!-- tool: research_add_note -->
<!-- tool: research_add_source -->
<!-- tool: research_pack_for_scene -->
<!-- tool: research_search -->
<!-- tool: save_summary -->
<!-- tool: search_bible -->
<!-- tool: set_active_project -->
<!-- tool: set_arc_pacing_constraints -->
<!-- tool: switch_branch -->
<!-- tool: update_promise_status -->
