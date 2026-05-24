---
name: revision-manager
description: >
  Use when the user wants to explore alternate story paths, revise earlier chapters, compare
  different versions, manage branches, merge changes, or handle any structural revision that
  affects downstream content. Also triggers for "what if Marcus betrayed Elena instead",
  "try an alternate version", "go back and change chapter 8", "compare these two approaches",
  "which version is better", "undo the last few scenes", "create a save point", or any request
  about versioning, branching, revision, rollback, or alternate timelines of the STORY BIBLE
  (not the in-world timelines — those are the worldbuilder's domain).
---

# Revision Manager

Writing is rewriting. This skill manages the structural complexity of revision — exploring
alternate paths without destroying existing work, comparing approaches, and merging the
best version back into the main line.

## Revision activity workflow

Track revision intent and outcomes in session activity so later writers can
re-anchor quickly:

1. Call `set_active_project` once per session so subsequent tool calls inherit
   the project (and active branch).
2. Call `record_note` before risky branch work to capture intent and scope.
3. Run the branch/revision flow (`create_branch`, `switch_branch`,
   `revise_scene`, `diff_branches`, `merge_branch`) as needed.
4. Call `list_revision_markers` to see outstanding revision flags raised by
   `revise_scene` (downstream scenes that may need attention). Resolve each
   with `resolve_revision_marker` once you've addressed it.
5. Call `list_scene_versions` and `restore_scene_version` for per-scene
   version history rollback (independent of branch save points).
6. Optionally call `run_dual_persona_review` as a quality gate before
   merging back to main — it persists a `PersistedDualPersonaReview` record
   you can reference later.
7. Call `record_note` after major decisions (accepted variant, rejected path,
   unresolved risk) to preserve rationale.
8. Use `get_writer_state.recent_session_activity` to confirm notes are visible
   and provide handoff context.
9. Call `update_writer_position` when you need to park the cursor explicitly
   between sessions.

## When to Use Branching

### Scenario 1: Creative Exploration ("What If...")
The user wants to try a different direction. "What if Marcus betrays Elena instead of saving her?"

1. Call `create_save_point` on the current branch (insurance).
2. Call `create_branch` with type "exploration" and a descriptive name.
3. Call `switch_branch` to the new branch.
4. Use **scene-writer** to draft the alternate scene on this branch.
   All state changes (character states, relationships, pacing) go to this branch only.
5. When done, call `diff_branches` to compare the exploration vs main.
6. Present the diff to the user: what changed in characters, relationships, pacing?
7. If the user prefers the exploration: call `merge_branch` into main.
8. If the user prefers the original: call `switch_branch` back to main.
   The exploration branch stays archived for reference.

### Scenario 2: Generate Alternatives (Loom Pattern)
Generate N variations of the same scene and pick the best one.

1. Call `generate_alternatives` with the typed input. Required fields:
   - `project_id`
   - `book_number`, `chapter_number`, `scene_order`
   - `character_ids: Vec<String>`
   - `location_id: String`
   - `variation_strategy` (REQUIRED): "temperature" (same prompt, different
     randomness), "approach" (different scene structures), or "agent"
     (different models)
   - `alternatives` (`Option<usize>`): Number of alternatives, defaults to 3.
2. The system automatically:
   - Creates N temporary branches
   - Assembles context once (shared across all branches)
   - Routes each branch to an agent (same or different per strategy)
   - Generates N scene variations
   - Runs quality gate on each (anti-slop, voice, POV)
3. Call `compare_alternatives` to get a structured comparison:
   - Side-by-side summaries
   - Quality scores per variation
   - Which one best serves the current pacing needs
   - Which one has the strongest character voice
   - Which one creates the best hook ending
4. Pick the winner: call `select_alternative` with both `project_id` and
   the chosen `branch_id`. The selected branch merges into main; others are
   archived.

This is the fastest way to find the best version of a scene. Instead of writing,
evaluating, then deciding whether to revise, you generate multiple options up front
and select from strength.

### Scenario 3: Agent Comparison
Route the same scene to two different agents and compare quality.

1. Call `create_branch` (required fields: `project_id`, `name`; optional
   `branch_type` like "agent_comparison") — twice, one per agent.
2. On each branch, use **scene-writer** to draft with a different agent.
3. Call `diff_branches` between the two.
4. Cherry-pick the better output into main.

### Scenario 4: Revision with Ripple Isolation
Going back to revise chapter 8 might invalidate character states in chapters 9-20.

1. Call `create_branch` type "revision" from the point of chapter 8.
2. Call `switch_branch` to the revision branch.
3. If you only know the chapter and need the existing `scene_id`, read
   `bible://projects/{project_id}/chapters/{book_number}/{chapter_number}/scenes`
   first. This returns the active-branch scene ids and `scene_order` values for
   that chapter. Do not use `export_bible` just to recover scene ids.
4. Use `revise_scene` to rewrite the scene on this branch.
   - This tool automatically: invalidates downstream character states,
     marks affected embeddings as stale, flags later scenes that may have
     continuity issues.
5. Re-draft affected downstream scenes on the revision branch.
6. For branch-heavy or timeline-heavy work, read
   `bible://projects/{project_id}/timeline-graph/mermaid` to verify the active
   branch lineage, save points, timeline event order, and temporal
   interventions before merge decisions.
7. Call `check_consistency` on the revision branch to verify continuity.
8. When clean, call `merge_branch` into main.

### Scenario 5: Quick Save Before Risky Operation
Before doing anything irreversible, save state.

1. Call `create_save_point` with a descriptive name.
2. Proceed with the risky operation on main.
3. If it goes badly on the active branch: call `restore_save_point`.
   If you want to preserve the failed state first, create another save point
   before restoring or do the risky work on a branch instead.

---

## The Revision Workflow

### Step 1: Assess Impact

Before revising anything, understand what downstream state depends on the scene
being revised. Call `get_scene_context` for the scene to see:
- What character_states were committed because of this scene?
- What relationship changes were made?
- What knowledge was gained?
- What pacing progress was recorded?
- What narrative promises were advanced?

All of these must be reconsidered if the scene changes.
If you need to revise multiple existing scenes in a chapter, enumerate them with
`bible://projects/{project_id}/chapters/{book_number}/{chapter_number}/scenes`
before you start calling `revise_scene`.

### Step 2: Branch and Revise

Always branch before revising. Call `create_branch`, locate the target scene id
through the chapter scenes resource when needed, then call `revise_scene`:

If the revision also touches local manuscript files outside the Bible tools,
sequence that work explicitly:
- Read the current file contents first. Do not rely on remembered strings or
  stale replace anchors.
- Edit one file at a time when the change depends on exact surrounding prose.
- Do not batch local file edits and Bible-writing MCP calls in the same
  parallel step. If a prose edit fails, stop and fix that edit before issuing
  `update_entity`, `save_summary`, `commit_scene_changes`, or similar state writes.
- After the prose edit succeeds, then apply the matching Bible updates.

The `revise_scene` tool returns a rich envelope:
- **states_invalidated**: Character/world states based on the original scene
  that now need recomputation. These aren't deleted — they're flagged on the
  branch.
- **downstream_scenes_flagged**: Scenes written after this one that may
  reference invalidated state. Each one becomes a `revision_marker`
  retrievable via `list_revision_markers`.
- **pacing_impact**: How the revision affects arc pacing budgets.
- **diff** (`Vec<TextDiffChunk>`): Structured text diff between the previous
  prose and the revised prose.
- **byte_offsets_changed** (`Vec<TextByteRange>`): Byte ranges that changed,
  useful for surgical re-validation.
- **chars_added** / **chars_deleted**: Coarse diff metrics.
- **world_rule_hits**, **voice_drift**, **retcon_findings**: Validator
  findings on the revised scene, returned inline so you can decide whether
  to commit or revise again before saving state.

### Step 3: Cascade Resolution

For each flagged downstream scene:
1. Call `get_scene_context` to see its state WITH the revised data
2. Determine if the scene still makes sense
3. If yes: no action needed (the scene's character state was inherited from pre-revision)
4. If no: revise it too, then check ITS downstream scenes

This cascade is the most expensive operation in the system. It's why we branch first —
if the cascade is too destructive, we can abandon the branch.

### Step 4: Verify and Merge

Call `check_consistency` on the revision branch to verify:
- No knowledge contradictions introduced
- Character states are coherent
- World rules are still respected
- Pacing budgets are still valid

For large or long-running revisions, also read
`bible://projects/{project_id}/continuity/health` before merging. Treat open
validator findings, orphaned temporal interventions, and duplicate active
canonical-fact keys as merge blockers unless the user explicitly accepts them.

If clean, call `merge_branch` (required: `project_id`, `source_branch_id`,
`target_branch_id`, `merge_type`). The merge does not take a `resolutions`
parameter — when conflicts are detected, they are surfaced in
`MergeBranchOutput.conflicts` and must be resolved out-of-band: edit the
conflicting records on either branch and retry the merge, or use
`run_dual_persona_review` and `restore_scene_version` to choose a winning
version per scene before retrying.

---

## Branch Comparison

Call `diff_branches` to get a structured comparison:

The diff shows:
- **Scene diffs**: Side-by-side summaries of scenes that differ
- **Character state diffs**: How characters ended up differently on each branch
- **Relationship diffs**: Trust/tension differences
- **Pacing diffs**: Arc progress differences
- **Knowledge diffs**: What characters know differently
- **Narrative impact summary**: AI-generated description of how the two paths
  diverge narratively

Present this to the user in a clear format. Help them decide by analyzing:
- Which version creates more tension?
- Which version advances the theme more effectively?
- Which version has better pacing?
- Which version gives the protagonist more agency?

---

## Merge Types

| Type | When to Use |
|------|-------------|
| **fast_forward** | Main hasn't changed since the branch was created. Just adopt the branch's changes. |
| **cherry_pick** | Take specific scenes or commits from the branch, not everything. |
| **full_merge** | Merge all changes, resolving conflicts where both branches modified the same data. |
| **squash** | Collapse all branch changes into a single commit on main. Loses branch history. |

---

## Skill Chains

- **← scene-writer**: When a scene needs revision, the scene-writer handles the actual rewrite.
- **← continuity-editor**: When the consistency check finds errors, the revision-manager
  handles the structural changes needed to fix them.
- **→ scene-writer**: After branching, use scene-writer to draft on the new branch.
- **→ continuity-editor**: After merging, run a consistency check to verify the merge is clean.
- **→ plot-architect**: If revision reveals pacing problems, the plot-architect rebalances.

---

## References

The shipped craft references most relevant when revising:

- `bible://references/swain-scene-sequel` — Useful when revising a scene that
  was structurally weak (missing sequel beat, missing reaction).
- `bible://references/mru-guide` — Useful when revising MRU order issues.
- `bible://references/voice-differentiation` — Useful when a `voice_drift`
  finding is the trigger for revision.
- `bible://references/anti-slop` — Final pass before merging back to main.
