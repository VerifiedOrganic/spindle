---
name: plot-architect
description: >
  Use when planning, structuring, or modifying plot architecture: central conflicts, try-fail
  cycles, plot lines, themes, motifs, narrative promises, pacing curves, book/chapter outlines,
  and chapter scene plans.
---

# Plot Architect

Design narrative structure that builds tension, maintains momentum, and delivers earned payoff across single books and multi-book webserials.

## Tool Workflow

1. Call `set_active_project` once per session to bind the active project and branch.
2. Call `get_writer_state` to inspect the current cursor, overdue promises (`open_promises_due_now`), recent activity, and active overlays before restructuring.
3. Call `list_book_chapters` and `list_chapter_scenes` to map the existing spine.
4. For long-running webserials, inspect `get_series_status` and `get_editorial_queue` to separate published episodes from drafted backlog before extending the outline.
5. Use `create_motifs`, `create_narrative_promises`, and `create_terms` for atomic bulk seeding; the `batch_create_*` names remain one-release aliases. Revise conflicts and arcs with `update_conflict` and `update_character_arc`; reserve `update_entity` for unsupported fields.
6. Persist prose outlines with `set_book_outline` and `set_chapter_outline`; build structured scene plans with `plan_chapter` (first-class `location_id` and `content_rating` on every planned scene so `authoring_prepare_run` can validate).
7. Call `get_chapter_briefing` before handing a chapter off to drafting, `annotate_scene_beats` after scenes are written, `move_scene` for reordering, and `check_consistency` after structural changes.

## Three Planning Horizons (Webserials and Series)

- **Series horizon**: Reader contract (`promise`, `style_notes`, `boundaries` updated via `update_entity` on the project), enduring world rules, major character destinations, and long-range promises.
- **Current arc**: Flexible book outline (`set_book_outline`), active plot lines, conflicts, and arc milestones. Name the central question, irreversible character changes, promises to advance, and exit condition.
- **Next episode**: Concrete `plan_chapter` scene list with POV, location, content rating, purpose, one local satisfaction, and one reason to return (curiosity, emotional anticipation, humor, or consequence — never force a cliffhanger where the contract does not want one).

After each episode, compare intent with accepted prose and call `update_promise_status` (`source_scene_id` or `at` placement; use `replaces_event_id` to correct an erroneous event) only for threads that genuinely moved.

## Building the Plot Architecture

### 1. Central Conflict and Try-Fail Cycles (`create_conflict`)
Define `conflict_type` (`person_vs_person`, `person_vs_self`, `person_vs_society`, `person_vs_nature`, `person_vs_fate`, `person_vs_technology`, `person_vs_system`), concrete devastating `stakes`, `escalation_stages`, `expected_total_cycles` (typically 3–5 try-fail cycles escalating from naive failure -> costly `yes-but` -> internal Truth resolution), and `stated_consequences` (threats that must be demonstrated on-page).

### 2. Plot Lines and Convergence (`create_plot_line`)
Create `main`, `parallel`, `mirror`, `conflict`, and `romantic` plot lines with explicit `convergence_points` where subplots collide at major turning points.

### 3. Themes and Motifs (`create_theme`, `create_motif`)
Define `theme_statement`, `thesis_antithesis` (exploring both sides without preaching), `introduction_point`, and `resolution_point`. Pair themes with recurring symbols via `create_motif` (or `batch_create_motifs`) and set `max_uses_per_chapter`.

### 4. Narrative Promises (`create_narrative_promise`, `update_promise_status`)
Plant `chekhov_gun`, `foreshadowing`, `setup`, and `question_raised` promises (or seed in bulk via `batch_create_narrative_promises`) and advance their lifecycle (`planted` -> `reinforced` -> `paid_off` / `subverted` / `abandoned`) with `update_promise_status`.

### 5. Pacing Architecture (`create_pacing_config`, `create_pacing_curve`, `set_arc_pacing_constraints`)
- `create_pacing_config`: Set `total_planned_books`, `avg_chapters_per_book`, `avg_scenes_per_chapter`, and `tension_model` (`escalating`, `wave`, `slow_burn`, `explosive`).
- `create_pacing_curve`: Define act breakpoints and scene-type density per book.
- `set_arc_pacing_constraints`: Allocate `per_book_budget`, `max_progress_per_chapter`, `milestone_spacing`, `sprint_allowance`, and `regression_budget` per character arc.
- To start a new book in an existing series, call `create_book` and `create_chapter` after reading `get_writer_state`.

### 6. Chapter Scene Plans (`plan_chapter`)
Every chapter should change the reader's understanding of the story. Specify chapter `pov`, `synopsis`, active themes/conflicts/plot lines, and each scene's `summary`, beat structure, characters, `location_id`, `content_rating`, and `purpose`.

## Initial Planning Rollout Boundary

Staged planning artifacts and bounded `plan`/`outline` route proposals (ADR 0004, Phases A–C) represent intent, not applied canon or a draftable-plan verdict; new-book planning entry tools are not shipped yet. Route generation stages `PlanningArtifactV1` proposals with versioned prompt/schema hashes, pre-dispatch token/cost ceilings, at most one `planning_repair_v1` attempt, and host-fallback requests when routes are missing or rejected. All initial gates default to human; only an explicit author delegation permits automated chapter/scene acceptance after approved roots, and in-run plan amendments always require human ratification.

<!-- tool: annotate_scene_beats -->
<!-- tool: authoring_prepare_run -->
<!-- tool: batch_create_motifs -->
<!-- tool: create_motifs -->
<!-- tool: batch_create_narrative_promises -->
<!-- tool: create_narrative_promises -->
<!-- tool: batch_create_terms -->
<!-- tool: create_terms -->
<!-- tool: check_consistency -->
<!-- tool: create_book -->
<!-- tool: create_chapter -->
<!-- tool: create_conflict -->
<!-- tool: create_motif -->
<!-- tool: create_narrative_promise -->
<!-- tool: create_pacing_config -->
<!-- tool: create_pacing_curve -->
<!-- tool: create_plot_line -->
<!-- tool: create_theme -->
<!-- tool: get_chapter_briefing -->
<!-- tool: get_editorial_queue -->
<!-- tool: get_series_status -->
<!-- tool: get_writer_state -->
<!-- tool: list_book_chapters -->
<!-- tool: list_chapter_scenes -->
<!-- tool: move_scene -->
<!-- tool: plan_chapter -->
<!-- tool: set_active_project -->
<!-- tool: set_arc_pacing_constraints -->
<!-- tool: set_book_outline -->
<!-- tool: set_chapter_outline -->
<!-- tool: update_character_arc -->
<!-- tool: update_conflict -->
<!-- tool: update_entity -->
<!-- tool: update_promise_status -->
