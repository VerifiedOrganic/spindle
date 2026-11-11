---
name: authoring-supervisor
description: >
  Use when coordinating, executing, or resuming a multi-chapter drafting run. Covers run
  preparation, bounded step execution, canon mining, plan amendments, checkpoint review and audit,
  editorial triage, and episode release.
---

# Authoring Supervisor

You coordinate structured multi-chapter authoring runs safely, keeping SQLite run state synchronized and resolving review findings autonomously unless a change alters creative intent, canon, plot outcome, or content boundaries.

## Core Principles

1. **Human gates stay honest** — Respect the run's `mode`, `checkpoint_policy`, `mining_policy`, and `replan_policy`. Never bypass a required checkpoint or auto-apply mined canon deltas or in-run plan amendments without review.
2. **State before action** — Inspect `authoring_status` and `get_editorial_queue` before advancing a run or resolving a block.
3. **Autonomous local quality** — Apply line-level prose, sensory grounding, pacing, and scene-local continuity fixes yourself; ask the operator only for plot, canon, or content-boundary decisions.
4. **Write discipline for subagents** — Subagents may read context (`get_scene_context`, `search_bible`, `get_chapter_briefing`) and report findings, but all state-mutating calls stay in the main supervisor context.

---

## Step 1: Prepare and Start an Authoring Run

1. **Inspect active state**: Call `authoring_status`, `get_series_status`, and `get_editorial_queue` (plus `search_bible` when context is needed).
2. **Verify run readiness (`authoring_prepare_run`)**:
   - Call `authoring_prepare_run` with `project_id`, `book_number`, `start_chapter`, `end_chapter`, and the intended `mining_policy` / `checkpoint_policy`.
   - If `ready_to_draft` is `false`, report `missing_requirements`. Fix missing scene `location_id` or `content_rating` fields in the chapter plan via `plan_chapter` (`get_chapter_briefing` to inspect).
   - If the book tracks in-world time, ensure the calendar is configured via `set_project_calendar`.
3. **Start the run (`authoring_start_run`)**:
   - Launch with `authoring_start_run` using supported execution mode and policy values:
     - `mode`: `"hybrid"` (default; host assistant drafts non-explicit scenes) or `"agent"` (route-offloaded drafting).
     - `checkpoint_policy`: `"manual"` (default 4-step operator flow), `"auto_advisory"` (self-clears when deep check has nothing at or above `warning`), or `"auto_strict"` (self-clears only on zero findings and `anti_slop.hard_count == 0`).
     - `mining_policy`: `"disabled"` (default) or `"propose_all"` (stages canon deltas after each committed scene).
     - `replan_policy`: `"disabled"` (default) or `"propose_all"` (runs `replan_chapter` after each chapter summary to stage future-chapter plan amendments).
     - Optional `max_revise_attempts`: `0` (default off), `1`, or `2` for bounded in-run verify/revise passes.

---

## Step 2: Drive the Bounded Execution Loop

Call `authoring_execute_next` (`project_id`, `run_id`, optional `mode: "hybrid" | "agent"`) to advance one bounded step at a time:

- **Default `mode: "hybrid"` (host drafting)**:
  - For non-explicit scenes, `authoring_execute_next` stops at `draft book scene X.Y` with a host-draft instruction.
  - Read `get_scene_context` and `get_chapter_briefing`, draft the scene in the host, and save with `authoring_save_scene_draft` (`authorship: "assistant"`, full prose, and required continuity package: `character_states`, `canonical_facts`, `relationship_updates`, `beats`, `continuity_notes`). Never use generic `save_scene_draft` inside an active authoring run (`save_scene_draft` is for standalone scenes outside a run).
  - Stamp `set_scene_clock` when a project calendar is active.
- **Explicit offload (`mode: "agent"`)**:
  - Use `mode: "agent"` only when the user explicitly requests offloaded route generation or for explicit-rated scenes routed to a cleared backend.
- **Post-scene state & telemetry**:
  - Advance through `commit scene changes` (`commit_scene_changes`), `record_knowledge`, `create_narrative_promise`, and `update_entity` as needed, then `annotate beats` and `save summary`.
  - Inspect token/latency/template attribution with `get_model_usage`.

---

## Step 3: Canon Mining & Plan Amendments

1. **Canon deltas (`mine_scene_canon`, `list_canon_deltas`, `decide_canon_deltas`)**:
   - When `mining_policy: "propose_all"` is active (or via manual `mine_scene_canon`), inspect staged proposals with `list_canon_deltas` (`status: "staged"`).
   - Adjudicate decided proposals with `decide_canon_deltas` using `action: "apply"` or `"reject"` (optional `edit` payload and `note`). To defer a proposal, leave it `staged` without submitting a decision. Never auto-apply `secret_of_fact_id` circle expansions.
2. **Plan amendments (`replan_chapter`, `list_plan_amendments`, `decide_plan_amendments`)**:
   - Inspect proposed outline updates for undrafted future chapters with `list_plan_amendments`.
   - Adjudicate with `decide_plan_amendments` (`action: "apply"` or `"reject"`, leaving deferred proposals `staged`).

---

## Step 4: Review Checkpoints and Resume

When `authoring_status` reports `next_action: "await_checkpoint_review"` (or when an auto-checkpoint blocks with `pending_manual_scene_ids`), execute the mandatory 4-step sequence in order:

1. **Read the checkpoint report**: Locate and inspect the checkpoint report artifact listed in `authoring_status.checkpoint_reports` (`start_chapter`, `end_chapter`, `sampled_scene_ids`, `pending_manual_scene_ids`).
2. **Run and record the scoped deep audit**:
   - If deep consistency is pending, call `check_consistency` for the checkpoint chapter range (`start_chapter`..`end_chapter`) with `deep_check: true`.
   - Pass the returned structured output (`deep_consistency`) to `authoring_record_checkpoint_audit` (`project_id`, `run_id`, `start_chapter`, `end_chapter`, `deep_consistency`).
   - Fix local continuity findings before proceeding. For `secret_leak` findings, either record an intended reveal with `record_knowledge` (`secret_of_fact_id` + `learned_at`) or revise the leaked dialogue.
3. **Run sampled two-round dual-persona reviews**:
   - For each sampled scene requiring review, call `run_dual_persona_review` with `rounds: 2`.
   - If a review surfaces autonomous local fixes, revise the scene and save via `authoring_save_scene_draft`, recommit with `commit_scene_changes`, and rerun `run_dual_persona_review` (`rounds: 2`) so the current scene text has a completed review.
4. **Mark the checkpoint reviewed (`authoring_review_checkpoint`)**:
   - Only after the deep audit is recorded and all sampled scene reviews are complete, call `authoring_review_checkpoint` (`project_id`, `run_id`, `start_chapter`, `end_chapter`, `directives`) to record carried-forward editorial directives and resume the run.

---

## Step 5: Blocked Runs, Editorial Queue, Style Refresh & Episode Release

- **Blocked runs (`authoring_resolve_block`, `authoring_cancel_run`)**:
  - After fixing a blocked scene or run-level reconcile issue, advance or reset the scene with `authoring_resolve_block` (`target_phase`: `"draft_saved"`, `"changes_committed"`, `"beats_annotated"`, or `"redraft"`). Never use `authoring_resolve_block` to skip checkpoint review.
  - Call `authoring_cancel_run` to pause a run cleanly.
- **Reader simulation & editorial queue (`read_episode`, `get_editorial_queue`, `decide_editorial_item`)**:
  - Run `read_episode` at chapter review to stage reader concerns into `get_editorial_queue`, and adjudicate items with `decide_editorial_item` using the current source hash and item revision.
- **Style profile learning (`refresh_style_profile`)**:
  - Inspect captured operator style-edit candidates with `refresh_style_profile` (`dry_run: true`) and fold approved updates into the profile with a live `refresh_style_profile`.
- **Episode release (`prepare_episode_release`, `release_episode`, `get_episode_release`)**:
  - Stage an episode snapshot with `prepare_episode_release`, verify zero blockers, publish locally with `release_episode` (passing source hash and previous release ID), and read snapshots via `get_episode_release`.

<!-- tool: authoring_cancel_run -->
<!-- tool: authoring_execute_next -->
<!-- tool: authoring_prepare_run -->
<!-- tool: authoring_record_checkpoint_audit -->
<!-- tool: authoring_resolve_block -->
<!-- tool: authoring_review_checkpoint -->
<!-- tool: authoring_save_scene_draft -->
<!-- tool: authoring_start_run -->
<!-- tool: authoring_status -->
<!-- tool: check_consistency -->
<!-- tool: commit_scene_changes -->
<!-- tool: create_narrative_promise -->
<!-- tool: decide_canon_deltas -->
<!-- tool: decide_editorial_item -->
<!-- tool: decide_plan_amendments -->
<!-- tool: get_chapter_briefing -->
<!-- tool: get_editorial_queue -->
<!-- tool: get_episode_release -->
<!-- tool: get_model_usage -->
<!-- tool: get_scene_context -->
<!-- tool: get_series_status -->
<!-- tool: list_canon_deltas -->
<!-- tool: list_plan_amendments -->
<!-- tool: mine_scene_canon -->
<!-- tool: plan_chapter -->
<!-- tool: prepare_episode_release -->
<!-- tool: read_episode -->
<!-- tool: record_knowledge -->
<!-- tool: refresh_style_profile -->
<!-- tool: release_episode -->
<!-- tool: replan_chapter -->
<!-- tool: run_dual_persona_review -->
<!-- tool: save_scene_draft -->
<!-- tool: search_bible -->
<!-- tool: set_project_calendar -->
<!-- tool: set_scene_clock -->
<!-- tool: update_entity -->
<!-- tool: prepare_semantic_continuity -->
