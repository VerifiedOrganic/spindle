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

If the book tracks in-world time — long spans, time-skips, flashbacks, or an
invented calendar — make sure the project calendar is declared with
`set_project_calendar` before the run (see the worldbuilder skill). It is what
powers the `chronology` check and the in-world-time hard constraint the drafting
step relies on. This is optional: projects that never declare a calendar simply
skip the timing checks.

### 2. Initialize and Start the Run

Once preparation returns `ready_to_draft: true`, start the run:

```json
authoring_start_run({
  "project_id": "project:123",
  "book_number": 1,
  "start_chapter": 1,
  "end_chapter": 5,
  "checkpoint_interval": 1,
  "editorial_directives": ["Keep the prose dark."],
  "mining_policy": "disabled",
  "max_revise_attempts": 0,
  "checkpoint_policy": "manual"
})
```

This returns a `run_id` (e.g., `authoring_run:xyz`) which persists the run state in the SQLite database.

`mining_policy` is optional and defaults to `disabled` (omit it for the exact
pre-mining behavior). Set it to `"propose_all"` to have the run stage proposed
canon deltas for every committed scene: a `mine canon` step runs automatically
between `commit scene changes` and `annotate beats`, recording an honest
`mine_status` (`staged` | `skipped` | `model_output_rejected` | `error`) and a
`mine_detail` on each scene in `authoring_status`. Mining never blocks a run — a
skip or error is reported, not fatal. When `propose_all` is set, run
`authoring_prepare_run` with the same `mining_policy` first: it additionally
verifies the mine-or-review route ladder covers every planned rating and returns
`missing_requirements` when it does not. Ratification stays an operator decision
between checkpoints — hand the staged queue to the **canon-steward** skill to
review and apply/reject; never auto-apply mined deltas.

`max_revise_attempts` is optional and defaults to **off** (omit it, or pass `0`,
for the exact pre-revise behavior: a saved draft goes straight to commit). Set it
to `1` or `2` (bounded — higher values are rejected as an input error) to enable
the in-run verify/revise loop: after each draft a deterministic `verify scene`
step runs the scene-scoped check subset, and any finding at or above `warning`
sends the scene back to the same draft route (or, in hybrid mode, back to you)
for a bounded revision before commit. Verify is deterministic (zero model calls)
and revision reuses the already-preflighted draft route, so no extra
`authoring_prepare_run` coverage is required. `authoring_status` surfaces each
scene's `verify_status` (`clean` | `findings` | `parked_findings` | `error`),
`verify_detail`, and `revise_attempts`. The loop converges or stops: the same
findings are never revised twice (a re-verify with an unchanged finding set parks
the scene as `parked_findings` "unchanged after revision"), and parked findings
inherit to the checkpoint exactly as today. Verify never blocks a run.

`checkpoint_policy` is optional and defaults to `manual` (omit it for the exact
classic 4-step operator checkpoint flow). Choose a policy by how much checkpoint
judgment you want to delegate:

- **`manual`** — you drive every checkpoint yourself (deep audit → record →
  sampled dual-persona reviews → `authoring_review_checkpoint`). Use when an
  editor is in the loop for each checkpoint, or the material is high-stakes.
- **`auto_advisory`** — the harness self-clears a checkpoint when the deep
  consistency check finds **nothing at or above `warning`** (`info` advisories
  are allowed through). It blocks with the full report otherwise, exactly like
  `manual`. Use for a mostly-unattended run where you still want to be stopped by
  genuine continuity/craft warnings but not by cosmetic `info` notes. (This is the
  recommended agent-mode policy per the design's D-1 record; `manual` stays the
  default.)
- **`auto_strict`** — self-clears only on **zero findings of any severity**. Any
  `info` note blocks. Use when even advisory findings must be operator-reviewed.

An auto policy has a start-time precondition: the `review` route must resolve
rating-cleared for **every** content rating in the run's range (the automation
runs its sampled dual-persona reviews and deep pass through `review`). Run
`authoring_prepare_run` with the same `checkpoint_policy` first; an uncovered
rating returns a `missing_requirements` entry naming the policy, the `review`
route, and the rating, so an offload gap fails at prepare rather than at 2am.

**Explicit-manual-fallback (offload-critical):** if a sampled scene's review is
rejected at the offload chokepoint because the `review` agent is not cleared for
that scene's rating (e.g. a mid-run `configure_agents` swap dropped explicit
coverage), that scene is marked **pending-manual** and its prose is dispatched
nowhere else. The checkpoint blocks listing exactly those scenes
(`pending_manual_scene_ids` in `authoring_status`); the cleared scenes' reviews
and the deep audit still completed. A blocked auto-checkpoint stays
`pending_review`, so you can still clear it by hand with the manual 4-step flow.
`authoring_status` reports each checkpoint's `checkpoint_policy`, `auto_outcome`
(`approved` | `blocked` | `manual`), and `pending_manual_scene_ids`.
Canon-steward ratification (`list_canon_deltas` / `decide_canon_deltas`) still
happens between checkpoints regardless of policy.

#### When a pass skips

A `mine_status` of `skipped` (detail: `"…SKIPPED: the mine route is not cleared
for rating \`explicit\`…"`) and a matching `pass_skipped` journal event mean the
explicit-content offload gate refused to send that scene's prose to a model whose
configured agent does not declare the scene's content rating. This is by design:
a skip is honest, not a failure, and the run advances (skips never block). A
skipped pass is **not** a clean pass — do not report it as one.

Two remedies:

- **Clear the agent's ratings coverage** — add the missing rating to the
  agent's `ratings` list in the project agent config (or route the pass to an
  agent that already declares it), then re-run. This is what the rating-coverage
  preflight (`authoring_prepare_run` with the run's `mining_policy`) checks up
  front, so an offload gap surfaces at prepare rather than mid-run.
- **Accept the skip** — leave the scene unmined/unreviewed by an automated model
  and handle it manually (mine/review it yourself in-chat, or ratify canon by
  hand). Nothing is lost; only the automation was declined for that rating.

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
Spindle context/tools, call `authoring_save_scene_draft` with the full prose and
its structured continuity package, then call `authoring_execute_next` again. Do
not opt into agent drafting unless the user explicitly asks for a fully
automated/offloaded run.

`authoring_save_scene_draft` is mandatory for host-drafted authoring scenes.
Include `character_states`, `canonical_facts`, `relationship_updates`, `beats`,
and `continuity_notes`. If the scene introduces no durable canon changes, add a
`continuity_notes` entry saying that explicitly. Do not use generic
`save_scene_draft` inside an active authoring run; it lacks the required
continuity package.

When the project declares a calendar, also stamp each drafted scene on the
in-world clock with `set_scene_clock`, marking `temporal_mode` and/or a
`thread_key` for any scene that is deliberately out of linear order. Give the
`beats` honest per-beat `intensity` values rather than flattening them — the
clock feeds the `chronology` check and the intensities feed `pacing_drift`, both
of which the checkpoint audit runs.

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
- `verify scene X.Y` (only when `max_revise_attempts > 0`; deterministic scene-scoped checks after a draft)
- `revise scene X.Y (attempt N)` (only when verify found `warning`+ findings and attempts remain; agent mode re-drafts, hybrid mode hands the findings back to you to revise and re-save)
- `commit scene changes` (validating and committing draft changes)
- `annotate beats` (extracting structure and tagging beats)
- `save summary` (compiling chapter summaries)
- `run checkpoint` (creating the checkpoint snapshot)

After every execution step, keep the user informed concisely. Do not stop for a
choice unless the run is blocked on a real operator decision.

**Post-scene canon mining (hand off to canon-steward).** Once a scene is
committed, its prose can be mined into proposed canon deltas the operator
ratifies, instead of hand-authoring canon calls. Call `mine_scene_canon` for the
committed scene, then hand off to the **canon-steward** skill to review the
staged queue with `list_canon_deltas` and apply/reject with
`decide_canon_deltas`. The steward reads each delta's verbatim evidence before
applying, never bulk-applies, and gives secret-link (circle-expansion) deltas
extra scrutiny. Ratification stays an operator decision — never auto-apply mined
deltas inside the run.

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
   checkpoint. For a book that tracks in-world time, make sure the deep pass
   includes the story-time checks (`chronology`, `temporal_coherence`,
   `knowledge_timing`, `pacing_drift`) so timing drift is caught at the
   checkpoint rather than fifty scenes later. `deep_check: true` also runs the
   Tier 2 model-backed `temporal_coherence` pass, which catches intra-scene
   time jumps phrased idiomatically (implied light/meal cues, "three cigarettes
   later") that the deterministic scan on each draft/commit cannot.
3. If deep consistency returns fixable findings, fix them before proceeding.
   Treat them the same way as reviewer findings: local fixes are autonomous;
   only ask for plot/canon/content-boundary choices.
   - **`secret_leak` findings need deliberate-irony triage**, not a blind fix.
     The check (deterministic; it runs at every checkpoint audit) flags an
     out-of-circle character who speaks a secret in attributed dialogue. Two
     legitimate outcomes: (a) the reveal is INTENDED — the character was told
     off-page or guessed — so record it with `record_knowledge`
     (`secret_of_fact_id` set, `learned_at` at the scene's placement), which
     expands the circle and clears the finding; or (b) it is a genuine leak, so
     revise the dialogue (autonomous local fix). If unsure whether the reveal is
     canon, that is a plot/canon choice — ask the operator, or dismiss with a
     note explaining the deliberate irony.
4. Call `run_dual_persona_review` with `rounds: 2` for each sampled scene ID.
5. Classify the review feedback:
   - **Autonomous local fix**: line-level prose, sensory grounding, pacing
     trim, repetition, filter words, UI/system gag sharpening, scene-specific
     continuity, missing expected LitRPG/system markup, required reward/result
     UI blocks, or other changes that do not alter canon or author intent.
     Revise the affected scene yourself, save the full revised prose with
     `authoring_save_scene_draft`, recommit scene changes, refresh beat
     annotations and chapter summary when applicable, rerun the sampled
     dual-persona review, and then approve the checkpoint.
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

## Subagent orchestration (Claude Code / grok)

Checkpoint review (step 5) is embarrassingly parallel: each sampled scene's
dual-persona analysis and the deep-consistency triage are independent read-only
research. If your harness supports subagents (Claude Code's Task/Agent tool,
grok's subagents), fan them out; otherwise run the same steps sequentially
inline — the workflow is identical, only the concurrency changes.

**Write discipline (non-negotiable):** subagents research and report only. Every
state-mutating spindle call stays in the main context — the supervisor decides
and writes. Subagents never call `authoring_save_scene_draft`,
`commit_scene_changes`, `authoring_record_checkpoint_audit`,
`run_dual_persona_review` (it persists a record), `authoring_review_checkpoint`,
or any `revise_*`/`commit_*`/`update_*` tool. They read
(`get_scene_context`, `search_bible`, `get_chapter_briefing`) and return
structured findings.

Fan-out for a checkpoint:
- **One subagent per sampled scene** — dispatch each with the scene id and its
  prose scope; the subagent runs the dual-persona *analysis* (Literary Critic +
  Craft Technician) as pure reasoning and returns findings classified as
  autonomous-local-fix / forward-directive / operator-decision (the step-5
  buckets), with scene-anchored evidence. The supervisor still calls
  `run_dual_persona_review` itself for the persisted record.
- **One subagent to triage deep-consistency** — hand it the `check_consistency`
  (`deep_check: true`) output and have it group findings by severity and
  fixability, returning a ranked worklist. The supervisor runs the
  `check_consistency` call and `authoring_record_checkpoint_audit` write itself.

The supervisor then merges every report, makes ratification decisions, applies
autonomous local fixes (saving via `authoring_save_scene_draft`), and calls
`authoring_review_checkpoint` to resume. If no subagent mechanism is available,
walk the sampled scenes and the triage one at a time in the main context.
