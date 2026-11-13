---
name: spindle
description: Entry workflow for Spindle fiction projects. Route between existing-book and new-book work, select the active project, and load the right task skill on demand.
---

# Spindle Entry Guide

The host assistant writes manuscript prose by default (`authorship="assistant"`). Spindle provides the Story Bible, scene context, continuity validation, and versioned saves. Host drafting needs no API key; model routes are configured separately (`bible://system/model-routes`).

## Choose a Task Skill

List available skills and references with `list_skills`, then load only what the task needs via `get_skill`, `get_reference`, or the equivalent `bible://skills/{name}` / `bible://references/{name}` resources:

- **Start a new book, or plan a new book in an existing series**: `new-book`
- **Draft or continue prose**: `scene-writer`
- **Multi-chapter supervised runs**: `authoring-supervisor` (plus `scene-writer` for host drafts)
- **Series, book, or chapter planning**: `plot-architect`
- **World and setting canon**: `worldbuilder`
- **Characters, arcs, and voice profiles**: `character-creator`
- **Branches, alternatives, and targeted revisions**: `revision-manager`
- **Developmental review, style profiles, and export**: `editor` (`editor/style` for style)
- **Consistency, chronology, and quantity audits**: `continuity-editor`
- **Bible search, snapshots, and recaps**: `bible-librarian`
- **Manuscript import**: `manuscript-importer`
- **Grounded factual research**: `researcher`
- **Mined canon deltas and plan amendments**: `canon-steward`

## Branch 1: Existing Book

1. Call `list_projects` when the project ID is unknown, then `set_active_project` so follow-up calls inherit the project and branch.
2. Call `get_writer_state` to confirm the active branch, cursor, promises, overlays, and unsynced files.
3. Load the relevant workflow with `get_skill` (`scene-writer`, `revision-manager`, `plot-architect`, etc.).
4. If planning is in progress, load `new-book` to reconnect to durable sessions.
5. If several projects match, ask which one; never create a replacement because an existing book was not selected.

## Branch 2: New Book

1. Load `get_skill(name="new-book")` for the host-neutral premise-to-ready flow: intake, project selection, human-approved planning gates, and a real readiness report. It needs the `new-book` profile; if `required_profile` differs, switch to it with `use_profile`.
2. `new-book` keeps every level human-gated and never drafts. Existing-series books pass the project and branch to reuse accepted canon.
3. For smaller manual setup, load `get_reference(name="spindle/setup")` to create and select the project, confirm the opening book and chapter, and prepare the first scene.
4. Setup tools belong to `spindle/setup`; switch to `workflow-write` or `setup` if needed.

## Tool Profiles and Response Contracts

Select the initial profile via `SPINDLE_TOOL_PROFILE` (inspect with `spindle-mcp profiles`). `get_skill` and `get_reference` report `covered`, `required_profile`, `available_units`, and `missing_capabilities`; reading a skill never mutates the profile or grants tool access.

To switch without reconnecting, call `use_profile` with the `required_profile` name. It returns the new tool list and emits `notifications/tools/list_changed`; the switch is session-scoped and not persisted, so `SPINDLE_TOOL_PROFILE` remains the initial profile. Out-of-profile calls stay denied. Prefer the smallest profile and use `full` only when one session must reach every skill-declared workflow.

Generated profiles omit large output schemas from `tools/list` without changing results. Load `get_reference(name="contracts/tools/v1")` (`bible://contracts/tools/v1`) for the index or `contracts/tools/v1/<tool>` for a full schema.

<!-- tool: get_reference -->
<!-- tool: get_skill -->
<!-- tool: get_writer_state -->
<!-- tool: list_projects -->
<!-- tool: list_skills -->
<!-- tool: set_active_project -->
<!-- tool: use_profile -->
