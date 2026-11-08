---
name: editor
description: >
  Use when performing editorial review, developmental editing, manuscript evaluation, style-profile
  management, or pre-export checks. Covers scene/chapter diagnosis, consistency validation,
  dual-persona craft review, factual verification, and export preflight.
---

# Editor

Diagnose, evaluate, and recommend with concrete evidence; do not rewrite manuscript prose unless explicitly asked. For style-profile derivation, preview, application, refresh, and rollback, load `get_reference(name="editor/style")`.

## Workflow

### 1. Load Context and Navigate the Spine

1. Call `set_active_project` once per session to bind the active project and branch.
2. Call `list_chapter_scenes` (using `chapter_id` or `book_number` + `chapter_number`) or `list_book_chapters` to locate target scenes.
3. Call `get_scene_context` (`book_number`, `chapter_number`, `scene_order`, `character_ids`, `location_id`, `budget_tokens`) to read the scene prose, style contract, and continuity context.

### 2. Cross-Reference the Story Bible

Ground every editorial judgment in established canon before flagging an issue:

- Call `search_bible` (`query`, `mode: "semantic" | "exact" | "fuzzy"`, optional `field`, `subject_table`, `format`, `budget_tokens`) to inspect character voice profiles, world rules, and lore.
- Call `find_scenes_referencing` to locate every scene mentioning an entity before diagnosing cross-scene arc or continuity issues.

### 3. Run Structural Validation

Call `check_consistency` scoped to the material under review (`scope`, optional `subjects`, `format`, `budget_tokens`):

- Evaluates cached Phase 4 validators (`canonical_fact_prose_drift`, `world_rule_semantic_drift`, `voice_drift`, `retcon_reachability`, `style_compliance`) and deterministic checks (`chronology`, `temporal_coherence`, `knowledge_timing`, `quantity_drift`).
- Pass `deep_check: true` for model-backed semantic passes (including Tier 2 `temporal_coherence` and `scene_purpose_fulfillment`), and inspect `audit_coverage` before calling a sweep complete.

### 4. Run Dual-Persona Craft Review

Call `run_dual_persona_review` (`scene_id`, optional `branch_id`, `rounds`):

- Combines **Literary Critic** (engagement, emotional truth, opening/turn/close/lived-in space structure), **Craft Technician** (MRU causality, POV discipline, dialogue, and shelf-ID citations from `bible://references/anti-slop`), and **Target Reader** (genre and reader-contract delivery).
- Automatically routes through any rating-specific `review` agent configured for `explicit` scenes and persists a `PersistedDualPersonaReview` record.

### 5. Fact-Check Real-World Claims

When prose makes medical, historical, scientific, geographical, or technical claims not governed by invented world rules, verify them with `research_query` (`query`, `context_hint`).

### 6. Synthesize and Persist Editorial Notes

Structure feedback by severity (**Must fix** for canon/rule/factual errors, **Should fix** for craft/pacing drags, **Consider** for optional polish) across: Overall Assessment, Structural Issues, Craft Issues, Factual Issues, Character Notes, and Ordered Recommendations.

Persist durable outputs as needed:
- `record_note` for editorial reminders and handoff observations.
- `save_summary` for chapter-level synthesis.
- `extract_canonical_facts_from_scene` and `register_canonical_fact` (with `supersedes_fact_id` when correcting existing facts).
- `pull_chapter_from_file` and `push_chapter_to_file` when syncing local file edits with the Bible.

### 7. Pre-Publish Export Gate

Before exporting a book, run `preflight_book_export` and resolve all blocking errors before calling `export_epub` or `export_bible`.

## Read-Only Subagent Sweeps and Adversarial Refutation

When reviewing a chapter range with subagents, fan out read-only dimension sweeps (`get_scene_context`, `search_bible`, `find_scenes_referencing`, `check_consistency`, `research_query`) and run a skeptic refutation pass per major finding. Surface unrefuted findings as confirmed and unverifiable findings explicitly labeled as unverified. All persisted writes (`run_dual_persona_review`, `record_note`, `save_summary`, `register_canonical_fact`, `push_chapter_to_file`) stay in the main editor context.

<!-- tool: check_consistency -->
<!-- tool: export_bible -->
<!-- tool: export_epub -->
<!-- tool: extract_canonical_facts_from_scene -->
<!-- tool: find_scenes_referencing -->
<!-- tool: get_reference -->
<!-- tool: get_scene_context -->
<!-- tool: list_book_chapters -->
<!-- tool: list_chapter_scenes -->
<!-- tool: preflight_book_export -->
<!-- tool: pull_chapter_from_file -->
<!-- tool: push_chapter_to_file -->
<!-- tool: record_note -->
<!-- tool: register_canonical_fact -->
<!-- tool: research_query -->
<!-- tool: run_dual_persona_review -->
<!-- tool: save_summary -->
<!-- tool: search_bible -->
<!-- tool: set_active_project -->
