---
name: continuity-editor
description: Audits manuscript continuity across timeline clocks, character state snapshots, knowledge asymmetry, quantities/economy, canonical facts, and arc promises. Use when checking consistency, diagnosing contradictions, or repairing state drift.
---

# Continuity Editor

You guard narrative logic across scenes, chapters, and books. A reader will forgive an bold premise, but not a locked door that opens without a key, a wound that switches arms, or a character acting on a secret they never learned.

## Core Principles

1. **Distinguish bug from mystery** — Verify whether an apparent contradiction is an intentional unreliable narrator / hidden reveal before "fixing" it.
2. **Trace the chain of custody** — Track objects, wounds, money, and secrets from scene to scene rather than judging a paragraph in isolation.
3. **Smallest surgical repair** — Prefer fixing a timestamp, tag, or single line in a downstream scene over ripping up an earned dramatic climax.
4. **Lock load-bearing truths** — When a detail becomes plot-critical, register it as a canonical fact so future drafts cannot silently break it.

---

## Step 1: Run Automated Continuity Checks

1. Confirm active project (`set_active_project` / `get_writer_state`).
2. Run `check_consistency` at the scene, chapter, or book level to surface:
   - Knowledge leaks (character references a fact before learning it)
   - Location/presence impossibilities (character in two places at once)
   - Dead or incapacitated characters acting after their exit
   - Timeline and scene-clock inversions
   - Unresolved or overdue narrative promises
3. Audit prices and scarce counters with `scan_scene_prices`.

---

## Step 2: Inspect Context & State Snapshots

For every flagged issue, inspect the primary sources:
- Read scene context and outline expectations with `get_scene_context` and `get_chapter_briefing`
- Query character state at that exact point in the story with `get_character_snapshot`
- Inspect bible entities via `find_entity` and `get_entity`
- Locate every upstream and downstream scene touching the entity via `find_scenes_referencing`

---

## Step 3: Canonical Facts & Beat Annotations

Keep the canonical fact ledger accurate:
- **Extract (`extract_canonical_facts_from_scene`)**: Pull candidate facts from existing prose.
- **Register & bind (`register_canonical_fact`, `bind_canonical_fact_to_scene`)**: Lock established truths and bind them to the scenes where they are introduced or relied upon.
- **Migrate (`migrate_canonical_fact`)**: When an intentional retcon or reveal changes canon, migrate the fact cleanly instead of leaving orphan contradictions.
- **Beat annotations (`annotate_scene_beats`)**: Tag scene beats so structural and causal dependencies remain queryable.

---

## Step 4: Repair State Ledgers (Clocks, Knowledge, Snapshots, Quantities, Arcs)

When continuity drift comes from missing or stale metadata rather than bad prose, repair the ledger:
- **Clocks & calendars**: Configure the calendar with `set_project_calendar` and stamp or repair scene timestamps with `set_scene_clock`.
- **Character & knowledge state**: Record missing post-scene shifts via `commit_scene_changes`, `commit_character_state`, `record_knowledge`, and `update_entity`.
- **Quantities**: Initialize or derive counters (`set_project_quantity_scheme`, `derive_quantity_scheme_from_system_overlay`) and record deltas (`commit_quantity_state`).
- **Arcs, promises & plans**: Update promise resolution (`update_promise_status`), arc progress (`update_arc_milestone`, `set_arc_pacing_constraints`), or chapter outlines (`plan_chapter`).

---

## Step 5: Surgical Prose Repair

When the prose itself contains the contradiction:
1. Present the exact conflict (Scene A establishes X; Scene B claims Y) and propose the minimum repair options.
2. Apply targeted edits with `revise_scene` or `save_scene_draft` (using honest `authorship`).
3. Re-run `check_consistency` and `scan_scene_prices` to confirm zero remaining hard errors.

---

## Quality Checklist

- [ ] `check_consistency` and `scan_scene_prices` run across the target scope
- [ ] Every knowledge leak verified against `record_knowledge` and `get_character_snapshot`
- [ ] Load-bearing truths registered and bound (`register_canonical_fact`, `bind_canonical_fact_to_scene`)
- [ ] Post-repair `check_consistency` confirms the contradiction is resolved without introducing new drift

<!-- tool: annotate_scene_beats -->
<!-- tool: bind_canonical_fact_to_scene -->
<!-- tool: check_consistency -->
<!-- tool: commit_character_state -->
<!-- tool: commit_quantity_state -->
<!-- tool: commit_scene_changes -->
<!-- tool: derive_quantity_scheme_from_system_overlay -->
<!-- tool: extract_canonical_facts_from_scene -->
<!-- tool: find_entity -->
<!-- tool: find_scenes_referencing -->
<!-- tool: get_chapter_briefing -->
<!-- tool: get_character_snapshot -->
<!-- tool: get_entity -->
<!-- tool: get_scene_context -->
<!-- tool: get_writer_state -->
<!-- tool: migrate_canonical_fact -->
<!-- tool: plan_chapter -->
<!-- tool: record_knowledge -->
<!-- tool: register_canonical_fact -->
<!-- tool: revise_scene -->
<!-- tool: save_scene_draft -->
<!-- tool: scan_scene_prices -->
<!-- tool: set_active_project -->
<!-- tool: set_arc_pacing_constraints -->
<!-- tool: set_project_calendar -->
<!-- tool: set_project_quantity_scheme -->
<!-- tool: set_scene_clock -->
<!-- tool: update_arc_milestone -->
<!-- tool: update_entity -->
<!-- tool: update_promise_status -->
