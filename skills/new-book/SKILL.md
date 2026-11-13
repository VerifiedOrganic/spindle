---
name: new-book
description: Host-neutral new-book entry flow. Take an author from a premise, through human-approved planning gates, to a plan that is genuinely ready to draft.
---

# New Book: Premise to Ready

Use this unit when the author wants to start a new book or plan a new book in an existing series. This workflow only creates and ratifies unwritten intent. It never writes prose, saves a scene draft, approves an in-run amendment, or starts a drafting run without a separate author instruction.

## Intake

Ask at most five questions and only for what the author has not already supplied:

1. What is the premise in one or two sentences?
2. What content rating ceiling should the plan respect?
3. Is this standalone, a series opener, or a new book in an existing series? How long is the near horizon?
4. Single point of view, or a named POV roster? If a roster, any hard rotation rule?
5. How deep should planning go right now: a three-chapter detailed horizon (recommended) or a different chapter range?

Record unknowns as assumptions. Do not invent canon to fill a missing answer.

## Step 1: Select or Create the Project

1. Call `list_projects` before offering creation. If the author names an existing series, ask which project and reuse its accepted contract and canon; never duplicate the series.
2. For an existing project, call `set_active_project`, then call `get_writer_state` to confirm the branch and current position.
3. For a brand-new book, keep the planning session unattached. The Contract level creates the project only after the author accepts it. Do not create a placeholder project.

## Step 2: Begin the Planning Session

Call `planning_begin` with the premise, the agreed horizon, the content-rating ceiling, and the budget when the author set one. For a new book in an existing series, pass `project_id` (and `branch_id` when the author chose a non-default branch) so the session binds to that canon.

Every level is human-gated by default. Only pass a gate policy when the author explicitly asks to delegate Chapter or Scene detail. Contract, CastWorld, SeriesArc, and Book always stay human-approved, and in-run amendments always stay human-ratified.

The next action in the result names the safe next step. Follow it.

## Step 3: Premise and Cast Gate

1. Call `planning_advance` to stage the Contract level. It returns a staged proposal, findings, and the next action.
2. Call `planning_status` to show the author the compact proposal: reader promise, core cast and world needs, assumptions, author questions, and any blocking findings.
3. Discuss the creative choices. If the author wants changes, reject or supersede the staged proposal with `planning_decide`, adjust the premise or direction, then advance again. Repairs are replacement proposals, never edits under an existing hash.
4. Call `planning_decide` with the proposal's exact payload hash only after the author explicitly approves. Then advance the CastWorld level and repeat the same review and decision.

Never treat model output, passing checks, or silence as approval.

## Step 4: Arc, Book, and Chapter Gate

Continue the same staged, review, decide loop for SeriesArc, Book, and Chapter:

- Show the central question, arc exit condition, promise setup and payoff schedule, POV plan, and chapter turns.
- Keep later books coarse. Detail only the agreed near horizon.
- Surface author questions before asking for a decision.
- Require an explicit author decision for each level. Record the author as the decision actor.

If a level returns a host generation request instead of a proposal, generate the artifact with the host model, then resubmit it through `planning_advance` with `action: "stage_host_artifact"` and the artifact in `host_artifact`. The same validator and human gates apply to host output. Actions are JSON strings (`"generate"`, `"stage_host_artifact"`, `"repair_last_proposal"`, `"recompute_readiness"`, `"set_policy"`); the legacy single-key object form such as `{"stage_host_artifact": null}` is still accepted for older clients.

## Step 5: Scene Packets and Handoff

1. Advance the Scene level so every chapter in the horizon gets concrete scene packets: purpose, cast, location, rating, beats, and promise movements.
2. Call `planning_status` and read the real readiness report. It distinguishes planner checks from the authoring prepare result.
3. Call `planning_decide` with `action: "prepare_authoring_run"` to get the real prepare report for the exact range. Do not claim readiness from planner checks alone. To record a decision, use `action: "decide"` with `decision` and the exact `expected_hashes`; `action: "revise"` supersedes the selected proposals.
4. When readiness passes, summarize accepted IDs, the chapter and scene packets, and any advisory warnings. Then load the drafting workflow: `scene-writer` for one scene, or `authoring-supervisor` for a supervised multi-chapter run. Starting prose remains a separate author instruction.
5. If readiness fails, show the missing requirements and advance or repair the relevant level. `authoring_prepare_run` remains available for an independent low-level recheck.

## Reconnect

Planning is durable. If a session is interrupted, call `planning_status` without a session ID to list active sessions, then call it with the chosen session ID. The status result reports the active gate, staged proposals, findings, readiness, and the next safe action. Continue from there; do not restart an accepted level.

<!-- tool: authoring_prepare_run -->
<!-- tool: create_book -->
<!-- tool: create_chapter -->
<!-- tool: create_project -->
<!-- tool: get_writer_state -->
<!-- tool: list_projects -->
<!-- tool: planning_advance -->
<!-- tool: planning_begin -->
<!-- tool: planning_decide -->
<!-- tool: planning_status -->
<!-- tool: set_active_project -->
