# Scene Research, Canon Recon, and Local Source Sync

Load this reference when assembling grounded research for a scene, delegating read-only canon recon to a subagent, or synchronizing local manuscript files with the Story Bible.

## 1. Scene Research Packages

When a scene depends on stored factual sources, claims, or historical/technical grounding:

1. Call `research_pack_for_scene` to assemble the scene-scoped research package, or `search_bible` to query indexed canon and notes.
2. Treat research excerpts as background evidence that must be filtered through the POV character's voice and immediate conflict — never paste raw research notes as narrator exposition.

## 2. Read-Only Canon Recon Subagents

Drafting prose is a single focused act of authorship and is never fanned out to subagents. When a host harness supports subagents and pre-draft canon verification is heavy, you may delegate **read-only canon recon**:

- **Allowed recon tools**: `get_character_snapshot`, `find_scenes_referencing`, `search_bible`, `get_scene_context`, and `research_pack_for_scene`.
- **Recon output**: A compact fact sheet covering voice reminders, current state, knowledge timing (what each present character knows or does not know at this cursor), due promises, and distant canon.
- **Write discipline (non-negotiable)**: A recon subagent never drafts prose and never calls `save_scene_draft`, `commit_scene_changes`, `update_entity`, `save_summary`, or any other state-mutating tool. All writes stay in the main writer context.

## 3. Local Manuscript File Sync

When the author also edits local Markdown manuscript files outside Spindle:

1. Check `unsynced_local_files` and `drift_warnings` in `get_writer_state` before trusting local files.
2. Use `pull_chapter_from_file` to import local file edits into the Story Bible and reconcile divergence.
3. Use `push_chapter_to_file` to export canonical Story Bible prose back to disk.
4. Keep manual file edits and Story Bible mutations sequential: re-read the file before editing, apply file edits one at a time, and call `save_scene_draft`, `commit_scene_changes`, `update_entity`, or `save_summary` only after the file operation succeeds.

<!-- tool: commit_scene_changes -->
<!-- tool: find_scenes_referencing -->
<!-- tool: get_character_snapshot -->
<!-- tool: get_scene_context -->
<!-- tool: get_writer_state -->
<!-- tool: pull_chapter_from_file -->
<!-- tool: push_chapter_to_file -->
<!-- tool: research_pack_for_scene -->
<!-- tool: save_scene_draft -->
<!-- tool: save_summary -->
<!-- tool: search_bible -->
<!-- tool: update_entity -->
