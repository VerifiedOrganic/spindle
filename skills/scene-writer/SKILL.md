---
name: scene-writer
description: >
  Core fiction scene-writing workflow. Use whenever drafting, continuing, or revising a scene or
  chapter. Covers context preparation, authoring provenance, voice/canon constraints, focused
  self-review, validation, state commit, recovery, and honest completion.
---

# Scene Writer

Deliver on **this project's declared genre and reader contract** (`promise`, `style_notes`, `boundaries`, `style` world rules, and `narrator_voice`), surfaced as **Project Style Requirements** in scene context. Genre and voice compliance outrank generic craft defaults: a comedy must land jokes, a thriller must propel, and a chapter promised to end on a hook must not end on a quiet reflective fade.

## Voice and Delivery

Honor the author's creative direction and style contract. Surface concrete canon or boundary conflicts; otherwise execute routine drafting, saving, and local fixes without asking permission.

Before continuing an existing manuscript, locate the preceding scene with `list_chapter_scenes` and read a passage of its actual prose with `get_entity`, alongside supplied `voice_samples` and character voice profiles. Summaries preserve events; prose preserves cadence. Match diction, narrative distance, humor, interiority, and dialogue without copying sample phrases.

Save only manuscript prose in the draft text (no assistant prefaces, beat outlines, critique, or closing notes). Use headings or system blocks only when they belong to the book's format. A writing task is complete only when prose and required state updates are saved and verified by tool results.

## Step 1: Prepare Context

1. Call `set_active_project` once per session to set the active project and branch.
2. Call `get_writer_state` to recover the active branch, cursor (`current` / `next`), `hard_constraints`, `open_promises_due_now`, `active_overlays`, `unsynced_local_files`, `drift_warnings`, and `recent_session_activity`.
3. Call `get_chapter_briefing` and read `continuity_sheets` (or `get_character_snapshot` for present characters when sheets are absent).
4. Call `get_scene_context` (`book_number`, `chapter_number`, `scene_order`, `character_ids`, `location_id`, `budget_tokens`, `sections`). Use `find_scenes_referencing` for entity history, `list_chapter_scenes` for the chapter spine, and `list_book_chapters` for book order.
5. Before offloading to a server-side route, check `bible://config/routing` (`caller_should_send_brief`, `adapter_kind`) and load `get_reference(name="scene-writer/offloaded-generation")`.

## Step 2: Enforce Canon, Voice, and Constraints

- **Project Style Requirements (`novel.style_directive`)**: Obey `reader_contract.promise`, `style_notes`, `boundaries`, `[STYLE DIRECTIVE]` world rules, and `narrator_voice` (`emotional_register`, `pacing_feel`, `comedy_density`, `interiority_ratio`, `chapter_ending_style`).
- **Pacing & Agency (`novel.pacing_directives`, `scene.agency_check`)**: Advance `behind` or `stalled` arcs; hold `in_cooldown` or `ahead` arcs. When `scenes_since_active_choice >= 3`, the protagonist must make an active, costly choice that drives events.
- **World Rules & Promises (`novel.world_rules`, `novel.narrative_promises_due`, `novel.semantic_references`)**: Never violate world rules or invent unseeded capabilities; pay off due promises when earned.
- **Knowledge & `[SECRETS IN PLAY]` (`novel.knowledge_briefing`)**: Characters act only on what they know at this cursor. Under `[SECRETS IN PLAY]`, characters in `Present and NOT in the know` must never reference, imply, or react to the secret; under a POV-only secret, narration may hold private awareness while dialogue and outward behavior stay sealed. For an on-page reveal, set `secret_of_fact_id` on the learning entry (or via `record_knowledge` with `secret_of_fact_id` and `learned_at`) so the circle expands forward without leaking backward.

## Step 3: Plan Beats and Draft Prose

Define the scene's purpose, what changes by the end, and the POV character's concrete goal. Outline beats (`goal`, `conflict`, `disaster`, `reaction`, `dilemma`, `decision`, `revelation`, `reversal`, `escalation`, `calm`, `transition`) with `plan_chapter` or `annotate_scene_beats`.

Write with clear stimulus-response causality, concrete character-specific details, distinct dialogue voices, and strict POV filtering. Ground every in-scene time jump in place, body, and mundane action (`[IN-WORLD TIME]`).

## Step 4: Content Rating and Provenance

Choose `general`, `teen`, `mature`, or `explicit` deliberately and verify boundaries against the reader contract.

- Pass `authorship: "assistant"` to `save_scene_draft` for host-composed or host-revised prose (`"human"` is reserved for the author's own writing).
- **Explicit sexual prose origin gate**: `save_scene_draft` rejects client-authored explicit sexual prose without a server-side `generation_id` receipt. When drafting or revising `explicit` sexual prose, load `get_reference(name="scene-writer/offloaded-generation")`.

## Step 5: Save, Self-Review, and Validate

Call `save_scene_draft` (`chapter_id` or `book_number` + `chapter_number`, `scene_order`, `title`, `full_text`, `summary`, `content_rating`, `tone`, `location_id`, `authorship`) and inspect the receipt:

1. **Receipt warnings**: Check `pacing_warnings`, `tone_deviation`, `style_warnings`, `agency_warning`, `content_rating_valid` / `content_rating_warnings`, `world_rule_hits`, `voice_drift`, `retcon_findings`, and advisory `temporal_findings`.
2. **Step 5a — Genre & Style Gate**: Verify tone match, `narrator_voice` delivery, genre payload density, active co-leads, `chapter_ending_style`, and cadence. Rewrite and re-save if any check fails.
3. **Step 5b — Fiction Anti-Slop & Craft Gate**: Check `anti_slop` against `compact_shelf_digest`, `voice_samples`, and `scene_negatives` (full 12-shelf catalog at `bible://references/anti-slop`):
   - **Hard shelves**: `contrast_not_x_but_y` (<=1/chapter), `emotion_cocktail` (0), `fishing_ending` (0).
   - **Soft shelves** (advisory; `said_bookism` is soft-only by default at <=2 non-`said` tags/chapter): `said_bookism`, `body_reactions`, `eye_department`, `gesture_rack`, `atmosphere_prefabs`, `naming_watchlist`, `rhythm_cadence`, `triadic_listing`, `solitary_fade`.
   - When a shelf hits, rewrite from the scene beat in 1–2 passes (never synonym-swap) and re-save.
4. **Consistency & Review**: Run `check_consistency` (`canonical_fact_prose_drift`, `world_rule_semantic_drift`, `voice_drift`, `retcon_reachability`, `style_compliance`, `temporal_coherence`, `chronology`, `knowledge_timing`; with `deep_check: true`, inspect `audit_coverage` and `next_offset`). Use `run_dual_persona_review` for Craft Technician, Literary Critic, and Target Reader evaluation.

## Step 6: Commit Canon, Beats, Summary, and Cursor

1. **Batch commit**: Call `commit_scene_changes` (`continuity_gate: "block_errors"` by default; resolve `blocking_continuity_findings` / `retcon_findings` or record an explicit override rationale with `record_note`).
2. **Targeted updates**: `set_scene_clock` (`day_index`, `time_of_day`, `duration_days`, `precision`, `temporal_mode`, `thread_key`), `commit_character_state`, `update_relationship` (`trust_delta`, `tension_delta`, `reason`, `scene_id`), `record_knowledge` (or `create_future_knowledge`), `update_promise_status`, `update_entity`, `update_world_rule` (`description`, not `summary`), `annotate_scene_beats` (`intensity` in `0.0..=1.0`), and `save_summary` on chapter end.
3. **Incidental facts & cursor**: Propose new minor characters (`create_character`), world rules (`create_world_rule`), entity updates (`update_entity`, `allow_rename: true` for renames), or superseded facts (`register_canonical_fact` with `supersedes_fact_id`), then call `update_writer_position`. After any timeout or reconnect, inspect `get_writer_state` before retrying a write.

## On-Demand References (Load Only When Triggered)

Never read every reference before drafting; call `get_reference` only when needed:

- `scene-writer/offloaded-generation` (`bible://references/scene-writer/offloaded-generation`): Explicit-rating offload, `generation_id` receipts, and `bible://config/routing` (`caller_should_send_brief` / `adapter_kind`).
- `scene-writer/craft-and-voice` (`bible://references/scene-writer/craft-and-voice`): Swain's Scene/Sequel framework, MRU/dialogue/POV craft, and common failure modes.
- `scene-writer/research-and-sources` (`bible://references/scene-writer/research-and-sources`): Scene research packs, read-only canon recon subagents, and local file sync.
- `scene-writer/authoring-runs` (`bible://references/scene-writer/authoring-runs`): Supervised authoring run drafting and post-commit canon mining.
- `scene-writer/drafting-packets` (`bible://references/scene-writer/drafting-packets`): Typed drafting-packet rollout boundary.
- `anti-slop` (`bible://references/anti-slop`): Full 12-shelf fiction anti-slop catalog.

<!-- tool: annotate_scene_beats -->
<!-- tool: check_consistency -->
<!-- tool: commit_character_state -->
<!-- tool: commit_scene_changes -->
<!-- tool: create_character -->
<!-- tool: create_future_knowledge -->
<!-- tool: create_world_rule -->
<!-- tool: find_scenes_referencing -->
<!-- tool: get_chapter_briefing -->
<!-- tool: get_character_snapshot -->
<!-- tool: get_entity -->
<!-- tool: get_reference -->
<!-- tool: get_scene_context -->
<!-- tool: get_writer_state -->
<!-- tool: list_book_chapters -->
<!-- tool: list_chapter_scenes -->
<!-- tool: plan_chapter -->
<!-- tool: record_knowledge -->
<!-- tool: record_note -->
<!-- tool: register_canonical_fact -->
<!-- tool: run_dual_persona_review -->
<!-- tool: save_scene_draft -->
<!-- tool: save_summary -->
<!-- tool: set_active_project -->
<!-- tool: set_scene_clock -->
<!-- tool: update_entity -->
<!-- tool: update_promise_status -->
<!-- tool: update_relationship -->
<!-- tool: update_world_rule -->
<!-- tool: update_writer_position -->
