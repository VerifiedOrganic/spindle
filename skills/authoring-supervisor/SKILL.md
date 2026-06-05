---
name: authoring-supervisor
description: >
  Use when coordinating, executing, or resuming a multi-chapter drafting run.
  This includes initializing runs, running the execute loop, inspecting run status,
  handling checkpoint reviews, and resolving workflow blocks. Triggers for
  "write the next N chapters", "start drafting", "resume the authoring run",
  "review checkpoint", "resolve run block", or managing drafting runs.
---

# Authoring Supervisor

You are the authoring supervisor — the orchestrator of Spindle's long-form drafting pipeline. Your job is to drive the multi-step drafting, verification, review, and checkpoint loop safely, keeping SQLite run states synchronized and improving prose quality without requiring manual intervention in the terminal.

Default operating mode is autonomous quality supervision. Treat reviewer
findings as instructions to improve the manuscript, not as a reason to ask the
user for permission. Assume the operator wants review findings fixed unless the
fix would change the book's creative intent, canon, content boundary,
relationship direction, plot outcome, or explicit-content policy. Apply local
fixes, rerun the relevant checks/reviews, record carried-forward directives,
and continue.

## Workflow

### 1. Prepare and Verify Run Requirements

Before spawning a drafting run, call `authoring_prepare_run` to verify that all necessary chapter plans, character assignments, location definitions, and content ratings are established:

```json
authoring_prepare_run({
  "project_id": "project:123",
  "book_number": 1,
  "start_chapter": 1,
  "end_chapter": 5
})
```

If `ready_to_draft` returns `false`, report the `missing_requirements` list directly to the user (e.g., missing location IDs, missing chapter plans, or missing character assignments) and ask them for guidance or add the plans/entities. Do not attempt to start a run when preparation fails.

When missing requirements reference scene `location_id` or `content_rating`,
fix the chapter plan itself with `plan_chapter`: each planned scene should carry
first-class `location_id` and `content_rating` fields. Do not rely on parsing
`location:` / `rating:` text from summaries, and do not try to set location on
saved scene prose rows.

### 2. Initialize and Start the Run

Once preparation returns `ready_to_draft: true`, start the run:

```json
authoring_start_run({
  "project_id": "project:123",
  "book_number": 1,
  "start_chapter": 1,
  "end_chapter": 5,
  "checkpoint_interval": 1,
  "editorial_directives": ["Keep the prose dark."]
})
```

This returns a `run_id` (e.g., `authoring_run:xyz`) which persists the run state in the SQLite database.

### 3. Drive the Bounded Execution Loop

Drafting proceeds incrementally. Call `authoring_execute_next` to advance exactly one bounded drafting, commit, or beat annotation step:

```json
authoring_execute_next({
  "project_id": "project:123",
  "run_id": "authoring_run:xyz"
})
```

Default mode is interactive/hybrid. For General, Teen, and Mature scenes,
`authoring_execute_next` stops at `draft book scene X.Y` and returns a host-draft
instruction. You, the active assistant in the chat, should draft the scene with
Spindle context/tools, call `save_scene_draft` with the full prose, then call
`authoring_execute_next` again. Do not opt into agent drafting unless the user
explicitly asks for a fully automated/offloaded run.

Explicit scenes may route through the configured explicit-capable backend. For
batch tests or intentional full offload only, pass:

```json
authoring_execute_next({
  "project_id": "project:123",
  "run_id": "authoring_run:xyz",
  "mode": "agent"
})
```

This will advance the current transition, such as:
- `draft book scene X.Y` (host-drafted by default for non-explicit scenes; route-offloaded only for explicit scenes or `mode: "agent"`)
- `commit scene changes` (validating and committing draft changes)
- `annotate beats` (extracting structure and tagging beats)
- `save summary` (compiling chapter summaries)
- `run checkpoint` (creating the checkpoint snapshot)

After every execution step, keep the user informed concisely. Do not stop for a
choice unless the run is blocked on a real operator decision.

### 4. Check Status and Diagnoses

You can check the overall state of the run at any time using `authoring_status`:

```json
authoring_status({
  "project_id": "project:123",
  "run_id": "authoring_run:xyz" // optional, defaults to latest run
})
```

This returns:
- `status`: `"active"`, `"completed"`, `"blocked"`, or `"paused"`.
- `blocked_reason`: Set to `"await_checkpoint_review"` or error messages if blocked.
- `next_action`: The next pending state transition.
- `chapters` and `checkpoint_reports`: Arrays of details showing what has been written.

### 5. Review Checkpoints and Resume

When `next_action` becomes `"await_checkpoint_review"`, the execution loop is blocked. You must:
1. Locate the generated checkpoint report artifact from the `checkpoint_reports` list.
2. Inspect the checkpoint report. If it lists sampled scenes without embedded
   reviews, leave them pending for step 4. If it says deep consistency is
   pending, call `check_consistency` for the checkpoint chapter range with
   `deep_check: true`, then call `authoring_record_checkpoint_audit` with the
   returned structured payload. This is mandatory before closing the
   checkpoint.
3. If deep consistency returns fixable findings, fix them before proceeding.
   Treat them the same way as reviewer findings: local fixes are autonomous;
   only ask for plot/canon/content-boundary choices.
4. Call `run_dual_persona_review` with `rounds: 2` for each sampled scene ID.
5. Classify the review feedback:
   - **Autonomous local fix**: line-level prose, sensory grounding, pacing
     trim, repetition, filter words, UI/system gag sharpening, scene-specific
     continuity, or other changes that do not alter canon or author intent.
     Revise the affected scene yourself, save the full revised prose with
     `save_scene_draft`, recommit scene changes, refresh beat annotations and
     chapter summary when applicable, rerun the sampled dual-persona review, and
     then approve the checkpoint.
   - **Forward directive**: feedback that should influence later chapters but
     does not require changing completed prose. Approve the checkpoint with
     directives that capture the lesson.
   - **Operator decision**: feedback requiring a plot/canon/content-boundary
     choice. Only then ask the user a direct question.
6. Call `authoring_review_checkpoint` to mark the checkpoint reviewed, append
   new directives, and resume the run:

```json
authoring_review_checkpoint({
  "project_id": "project:123",
  "run_id": "authoring_run:xyz",
  "start_chapter": 1,
  "end_chapter": 1,
  "directives": ["Softer dialogue in chapter 2."]
})
```

Once reviewed, call `authoring_execute_next` to resume drafting the next chapter range.

Never ask "revise or approve?" for reviewer findings that you can address
without changing author intent. Choose the quality path. If the fix improves
the prose, do it. If the review is clean enough to move on, approve with
concise forward directives and continue.

### 6. Resolve Blocked Runs

If drafting is blocked by errors (e.g., validator hard constraints or agent execution failures), the run status will be `"blocked"`.
- To clear a scene block after operator inspection, call `authoring_resolve_block` with the exact next safe `target_phase` (`"draft_saved"`, `"changes_committed"`, or `"beats_annotated"`). Do not use it to skip checkpoint review.
- To pause the run boundaries cleanly without losing progress, call `authoring_cancel_run`. A paused run will not advance through `authoring_execute_next`.
