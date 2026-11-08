---
name: manuscript-importer
description: >
  Use when importing an existing manuscript, novel, or webnovel into Spindle to build a populated
  Story Bible and continue writing from the current ending. Covers the 8-pass reviewable import
  pipeline and local source-bridge synchronization.
---

# Manuscript Importer

Ingest an existing manuscript (`txt`, `md`, `html`, `epub`, or `docx`) through Spindle's reviewable, resumable import pipeline so continuation starts with a complete Story Bible.

## Source-Bridge File Synchronization

When imported chapters also live in local manuscript files:
1. Call `pull_chapter_from_file` when the local file has the latest edits.
2. Run import passes and review decisions.
3. Call `push_chapter_to_file` after accepted updates to write branch scenes back to disk and refresh `scene_source_link` offsets. If offsets drift after external edits, request operator repair rather than silently rewriting offsets.

## Reviewable Import Pipeline

After each pass, inspect progress with `import_status` (or `bible://projects/{project_id}/imports/{session_id}` and `/review-items`) and resolve open review items with `import_apply_review_decisions` (`accepted`, `rejected`, `corrected`, `deferred`) before hydrating.

### Pass 0: Structural Ingestion (`import_manuscript`)
Call `import_manuscript` with `source_paths`, destination (`create_project_name` or `target_project_id`), optional `source_format_hint`, and `duplicate_strategy` (`reject` or `create_new_session`). Splits chapters and scenes, detects POV characters, and reports counts (`bible://projects/{project_id}/imports/{session_id}/structure`). Confirm chapter/scene splits with the author before extraction.

### Pass 1: Entity Extraction (`import_extract_entities`)
Call `import_extract_entities` (optional `segment_ids`, `limit`) to extract per-chapter characters, aliases, locations, key events, information exchanged, and relationship shifts. Present extracted entities to the author to catch missing characters or split aliases.

### Pass 2: Entity Consolidation (`import_consolidate_entities`)
Call `import_consolidate_entities` to deduplicate entities across chapters, merge aliases, track first/last appearances and scene counts, and rank characters by prominence.

### Pass 3: Deep Character Analysis (`import_analyze_character`)
Call `import_analyze_character` for batches of `cluster_ids` (top characters by prominence). Infers dialogue voice profiles (vocabulary, sentence structure, tics, formality, example lines), emotional profiles, decision patterns, state trajectories, and relationship trust/tension estimates. Review profiles with the author.

### Pass 4: World Extraction (`import_extract_world`)
Call `import_extract_world` to extract explicit and implicit world rules (costs, limits, side effects), locations, factions, and glossary terms. Confirm inferred rules with the author.

### Pass 5: Narrative Architecture (`import_analyze_narrative`)
Call `import_analyze_narrative` to identify active vs. resolved plot lines, unfulfilled narrative promises, character arc stages (Ghost/Wound/Lie/Want/Need/Truth), themes, and the opening reader contract. Ask the author where unresolved threads were headed.

### Pass 6: Final State Snapshot (`import_compute_final_state`)
Call `import_compute_final_state` to compute end-of-manuscript character locations, conditions, goals, knowledge boundaries, relationship levels, active thread tension, and world state. Resolve all remaining review items via `import_apply_review_decisions`.

### Pass 7: Bible Hydration (`import_hydrate_bible`)
Call `import_hydrate_bible` (`project_id`, `session_id`, required `include_scenes: true | false`, `hydrate_mode: "new_project" | "existing_project_branch"`, and target IDs). Materializes books, chapters, scenes, entities, voice profiles, arcs, promises, relationships, and final snapshots (`bible://projects/{project_id}/imports/{session_id}/hydration-report`).

## Post-Import Continuation and Targeted Fixes

After hydration, hand off to `scene-writer` to draft the next chapter. For targeted canon additions without re-running the import pipeline, use `record_knowledge` (`character_id`, `fact`, `source_summary`, `reader_visible`, optional `learned_at`) for character knowledge and `register_canonical_fact` for world/character canon facts.

<!-- tool: import_analyze_character -->
<!-- tool: import_analyze_narrative -->
<!-- tool: import_apply_review_decisions -->
<!-- tool: import_compute_final_state -->
<!-- tool: import_consolidate_entities -->
<!-- tool: import_extract_entities -->
<!-- tool: import_extract_world -->
<!-- tool: import_hydrate_bible -->
<!-- tool: import_manuscript -->
<!-- tool: import_status -->
<!-- tool: pull_chapter_from_file -->
<!-- tool: push_chapter_to_file -->
<!-- tool: record_knowledge -->
<!-- tool: register_canonical_fact -->
