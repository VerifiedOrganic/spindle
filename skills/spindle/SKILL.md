---
name: spindle
description: Entry workflow for Spindle fiction projects. Route between existing-book and new-book work, select the active project, and load the right task skill on demand.
---

# Spindle Entry Guide

The connected host assistant writes manuscript prose by default (`authorship="assistant"`). Spindle provides the persistent Story Bible, scene context packets, continuity validation, and versioned saves. Host drafting requires no external Spindle API key; server-side model routes are configured separately (`bible://system/model-routes`).

## Choose a Task Skill

List available skills and references with `list_skills`, then load only the unit your current task needs via `get_skill` or `get_reference` (or the equivalent `bible://skills/{name}` and `bible://references/{name}` resources):

- **Draft or continue prose**: `scene-writer`
- **Multi-chapter supervised runs**: `authoring-supervisor` (plus `scene-writer` when a run requests a host draft)
- **Series, book, or chapter planning**: `plot-architect`
- **World and setting canon**: `worldbuilder`
- **Characters, arcs, and voice profiles**: `character-creator`
- **Branches, alternatives, and targeted revisions**: `revision-manager`
- **Developmental review, style profiles, and export**: `editor` (`editor/style` for style profiles)
- **Consistency, chronology, and quantity audits**: `continuity-editor`
- **Bible search, snapshots, and recaps**: `bible-librarian`
- **Manuscript import**: `manuscript-importer`
- **Grounded factual research**: `researcher`
- **Mined canon deltas and plan amendments**: `canon-steward`

## Branch 1: Existing Book

1. Call `list_projects` when the project ID is unknown, then call `set_active_project` so follow-up calls inherit the project and branch.
2. Call `get_writer_state` to confirm the active branch, continuation cursor, open promises, active overlays, and any unsynced local file warnings.
3. Load the relevant workflow with `get_skill` (`scene-writer`, `revision-manager`, `plot-architect`, etc.).
4. When multiple projects could match, ask which one to select. Never create a replacement project because an existing book was not yet selected.

## Branch 2: New Book

1. Establish title, premise, genre, reader promise, style notes, and boundaries from the author's supplied direction; ask only for missing creative choices.
2. Load `get_reference(name="spindle/setup")` (or `bible://references/spindle/setup`) to create and select the project, confirm or create the opening book and chapter, load planning/world/character setup as needed, and prepare the first scene.
3. Setup tools belong to `spindle/setup`, enabled in `workflow-write` (and `write` / `setup`). Under `minimal`, reconnect with `SPINDLE_TOOL_PROFILE=workflow-write` rather than attempting unavailable setup calls.

## Tool Profiles and Response Contracts

Select startup profiles via `SPINDLE_TOOL_PROFILE` (inspect with `spindle-mcp profiles`). `get_skill` and `get_reference` report `covered`, `required_profile`, `available_units`, and `missing_capabilities`; reading a skill never mutates the profile or grants tool access. If a unit needs capabilities outside the active profile, report `required_profile` and reconnect.

Generated profiles omit large context/analysis output schemas from `tools/list` without changing runtime JSON results. Load `get_reference(name="contracts/tools/v1")` (`bible://contracts/tools/v1`) for the index or `contracts/tools/v1/<tool>` for a full schema.

<!-- tool: get_reference -->
<!-- tool: get_skill -->
<!-- tool: get_writer_state -->
<!-- tool: list_projects -->
<!-- tool: list_skills -->
<!-- tool: set_active_project -->
