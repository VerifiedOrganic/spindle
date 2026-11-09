---
name: worldbuilder
description: >
  Use when building or expanding any aspect of the story's world — realms, locations, factions,
  religions, economies, magic/tech systems, system overlays (LitRPG/progression), world rules,
  calendars, quantity schemes, and glossary terms.
---

# Worldbuilder

You build settings, institutions, and systems that shape conflict and constrain choices.

## Core Principles (Sanderson's Laws Applied to All Systems)

1. **Transparency determines utility** — Hard systems (explicit rules understood by the reader) can solve plot conflicts; soft systems create wonder and atmosphere but must not solve the climax via deus ex machina.
2. **Limitations > Powers** — What a magic, tech, political, or economic system *cannot* do, what it *costs*, and what *side effects* it causes generate story pressure.
3. **Expand before you add** — Deepen existing rules, factions, and cultural interpretations before inventing disconnected new systems.

---

## Step 1: Audit Existing World Canon

1. Call `set_active_project` once per session so subsequent calls inherit the active project and branch.
2. Read `bible://world/overview`, `bible://locations`, `bible://factions`, `bible://rules`, and `bible://terms`.
3. Call `find_entity` and `get_entity` before creating rules, factions, or locations to avoid duplicate canon.
4. Call `find_scenes_referencing` before and after major world changes to spot existing prose affected by the rule or setting.

---

## Step 2: World Rules, Realms & System Overlays

- **Create rules (`create_world_rule`)**:
  - Persist physical laws, magic limitations, technology constraints, social taboos, and resource scarcities (`rule_type`, `rule_name`, `description`, `established_in`).
  - **Always set `scan_pattern`** on rules whose violations should be caught in prose — the `world_rule_semantic_drift` validator scans scene text using `scan_pattern`.
  - Realms are represented via the optional `realm` field on entities plus realm-level `create_world_rule` entries.
- **Update rules (`update_world_rule`)**:
  - Edit existing world rules with `update_world_rule` (`{ world_rule_id, changes }`), not the generic `update_entity` tool.
- **System overlays (`create_system_overlay`)**:
  - For LitRPG, cultivation, or progression systems, define stats, skills, levels, and `visibility` with `create_system_overlay`.

---

## Step 3: Locations, Factions, Religions & Relationships

- **Locations (`create_location`, `update_location`)**:
  - Build hierarchical geography (`parent_id` linking room → building → district → city → region), sensory signatures, climate/terrain, and travel-time constraints.
- **Factions (`create_faction`)**:
  - Define public goals vs. actual leverage, territory, resources, internal fault lines, and inter-faction stances.
- **Conflict repair (`update_conflict`)**:
  - Revise stakes, stakes escalation stages, try/fail cycles, stated consequences, and demonstrated markers as the conflict evolves.
- **Religions (`create_religion`)**:
  - Define deities/cosmology, everyday practice and rituals, orthodoxy vs. folk belief, and taboos.
- **Relationships (`create_relationship`)**:
  - Connect leaders, factions, and institutions with `create_relationship` (`initial_trust`, `initial_tension`, `dynamics`).

---

## Step 4: Economies, Quantities, Calendars & Clocks

- **Economies (`create_economy`, `scan_scene_prices`)**:
  - Define currencies, denominations, wages, and staple prices with `create_economy`. Audit manuscript scenes for price or affordability drift with `scan_scene_prices`.
- **Quantity schemes (`set_project_quantity_scheme`, `derive_quantity_scheme_from_system_overlay`, `commit_quantity_state`)**:
  - Configure tracked resource/wealth counters and bands with `set_project_quantity_scheme` (or derive them from a LitRPG overlay via `derive_quantity_scheme_from_system_overlay`), and record scene ledger changes with `commit_quantity_state`.
- **In-world time (`set_project_calendar`, `set_character_birth`, `set_timeline_event_clock`, `set_scene_clock`)**:
  - Declare the project calendar once with `set_project_calendar` (`days_per_week`, `hours_per_day`, `days_per_year`, `months`, `epoch_label`) to enable `chronology` and `knowledge_timing` checks.
  - Anchor character ages with `set_character_birth`, orient historical/backstory events with `set_timeline_event_clock`, and stamp scene clocks with `set_scene_clock`.

---

## Step 5: Terms, Motifs, Pacing & Consistency Verification

- **Glossary & motifs (`create_term`, `create_terms`, `create_motifs`)**:
  - Register invented terms, titles, and cultural motifs individually or in batches.
- **Pacing (`create_pacing_config`, `set_arc_pacing_constraints`)**:
  - Configure world/arc pacing budgets so exposition and progression stay balanced.
- **Canon facts & knowledge (`register_canonical_fact`, `record_knowledge`, `check_consistency`)**:
  - Lock typed world facts with `register_canonical_fact`, record character knowledge of world secrets with `record_knowledge`, and run `check_consistency` to verify `world_rule_semantic_drift` and timeline compliance.

<!-- tool: batch_create_motifs -->
<!-- tool: create_motifs -->
<!-- tool: batch_create_terms -->
<!-- tool: create_terms -->
<!-- tool: check_consistency -->
<!-- tool: commit_quantity_state -->
<!-- tool: create_economy -->
<!-- tool: create_faction -->
<!-- tool: create_location -->
<!-- tool: create_pacing_config -->
<!-- tool: create_relationship -->
<!-- tool: create_religion -->
<!-- tool: create_system_overlay -->
<!-- tool: create_term -->
<!-- tool: create_world_rule -->
<!-- tool: derive_quantity_scheme_from_system_overlay -->
<!-- tool: find_entity -->
<!-- tool: find_scenes_referencing -->
<!-- tool: get_entity -->
<!-- tool: record_knowledge -->
<!-- tool: register_canonical_fact -->
<!-- tool: scan_scene_prices -->
<!-- tool: set_active_project -->
<!-- tool: set_arc_pacing_constraints -->
<!-- tool: set_character_birth -->
<!-- tool: set_project_calendar -->
<!-- tool: set_project_quantity_scheme -->
<!-- tool: set_scene_clock -->
<!-- tool: set_timeline_event_clock -->
<!-- tool: update_character -->
<!-- tool: update_conflict -->
<!-- tool: update_entity -->
<!-- tool: update_location -->
<!-- tool: update_world_rule -->
