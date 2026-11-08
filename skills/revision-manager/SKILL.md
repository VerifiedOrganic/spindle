---
name: revision-manager
description: >
  Use when exploring alternate story paths, revising earlier chapters with downstream ripple
  isolation, generating and comparing scene alternatives, managing Story Bible branches and save
  points, or refreshing style profiles from operator edits.
---

# Revision Manager

Manage structural revisions, "what-if" exploration branches, multi-candidate scene generation, save-point recovery, and branch merges without corrupting main-line continuity.

## Core Revision Activity Loop

1. Call `set_active_project` once per session to bind the active project and branch.
2. Call `record_note` before risky branch or revision work to log intent and scope.
3. Use `create_save_point` and `restore_save_point` for branch-level snapshot recovery, or `list_scene_versions` and `restore_scene_version` for single-scene version rollback.
4. Execute the branch, alternative, or ripple-revision workflow below.
5. Inspect downstream flags with `list_revision_markers` and clear each addressed item with `resolve_revision_marker`.
6. Verify branch health with `check_consistency` and `run_dual_persona_review`, record outcome rationale with `record_note`, confirm visibility in `get_writer_state`, and park the cursor with `update_writer_position`.

## Branching and Revision Workflows

### 1. Creative Exploration ("What If...")
1. Call `create_save_point`, then `create_branch` (`branch_type: "exploration"`) and `switch_branch`.
2. Draft or update scenes on the exploration branch (`commit_scene_changes`, `save_summary`, `update_entity` affect only the active branch).
3. Compare against main with `diff_branches` (scene, character-state, relationship, pacing, and narrative-impact diffs).
4. Call `merge_branch` (`fast_forward`, `cherry_pick`, `full_merge`, or `squash`) to adopt the exploration, or `switch_branch` back to main to leave it archived.

### 2. Generate and Select Alternatives (Loom Pattern)
1. Call `generate_alternatives` (`book_number`, `chapter_number`, `scene_order`, `character_ids`, `location_id`, `variation_strategy: "temperature" | "approach" | "agent"`, optional `alternatives`).
2. Call `compare_alternatives` to evaluate side-by-side summaries, quality scores, pacing fit, voice strength, and ending hooks.
3. Call `select_alternative` (`project_id`, `branch_id`) to merge the winning variation into main and archive the rest.

### 3. Earlier-Chapter Revision with Ripple Isolation
1. Call `create_branch` (`branch_type: "revision"`) and `switch_branch`.
2. Inspect `get_scene_context` (and `bible://projects/{project_id}/chapters/{book_number}/{chapter_number}/scenes` when locating `scene_id` — do not call `export_bible` merely to look up scene IDs; reserve `export_bible` for full Bible snapshots).
3. Call `revise_scene` on the revision branch. Inspect `states_invalidated`, `downstream_scenes_flagged`, `pacing_impact`, text diff metrics, and inline validator findings (`world_rule_hits`, `voice_drift`, `retcon_findings`, `temporal_findings`).
4. For each flagged downstream scene in `list_revision_markers`, inspect `get_scene_context`, revise if invalidated, and call `resolve_revision_marker`.
5. If local manuscript files are also edited, keep file edits sequential and separate from Bible writes (`commit_scene_changes`, `save_summary`, `update_entity`).
6. Run `check_consistency` (and inspect `bible://projects/{project_id}/continuity/health` and `bible://projects/{project_id}/timeline-graph/mermaid`). When clean, call `merge_branch`. If `MergeBranchOutput.conflicts` is non-empty, resolve conflicting records on either branch (using `run_dual_persona_review` or `restore_scene_version` as needed) and retry `merge_branch`.

## Style Learning from Operator Edits

When a project enables `style_learning` via `update_entity` (`changes: { "style_learning": 1 }`), operator re-saves over agent-drafted scenes are captured as style-edit candidates:
- Call `preview_refresh_style_profile` to inspect pending candidates (`included_count`, `withheld_count`, and prose-free diff summaries).
- Call `refresh_style_profile` (with optional `dismiss_candidate_ids`) to fold approved edits into a new style profile version.

## Read-Only Subagent Candidate Evaluation

When comparing branches or alternatives with subagents, dispatch one read-only subagent per candidate (`get_scene_context`, `diff_branches`, `search_bible`, `compare_alternatives`, `check_consistency`) to score tension, theme, pacing, agency, and continuity independently. Keep all branch/scene writes (`select_alternative`, `merge_branch`, `switch_branch`, `revise_scene`, `create_branch`, `restore_scene_version`, `resolve_revision_marker`, `run_dual_persona_review`) in the main context.

<!-- tool: check_consistency -->
<!-- tool: commit_scene_changes -->
<!-- tool: compare_alternatives -->
<!-- tool: create_branch -->
<!-- tool: create_save_point -->
<!-- tool: diff_branches -->
<!-- tool: export_bible -->
<!-- tool: generate_alternatives -->
<!-- tool: get_scene_context -->
<!-- tool: get_writer_state -->
<!-- tool: list_revision_markers -->
<!-- tool: list_scene_versions -->
<!-- tool: merge_branch -->
<!-- tool: preview_refresh_style_profile -->
<!-- tool: record_note -->
<!-- tool: refresh_style_profile -->
<!-- tool: resolve_revision_marker -->
<!-- tool: restore_save_point -->
<!-- tool: restore_scene_version -->
<!-- tool: revise_scene -->
<!-- tool: run_dual_persona_review -->
<!-- tool: save_summary -->
<!-- tool: search_bible -->
<!-- tool: select_alternative -->
<!-- tool: set_active_project -->
<!-- tool: switch_branch -->
<!-- tool: update_entity -->
<!-- tool: update_writer_position -->
