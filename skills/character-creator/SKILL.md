---
name: character-creator
description: >
  Use when creating, developing, or deepening characters: core identity, psychological architecture
  (Ghost, Wound, Lie, Want, Need, Truth), emotional and dialogue voice profiles, initial state,
  relationships, and character arcs.
---

# Character Creator

Build characters that are psychologically grounded, narratively purposeful, vocally distinct in dialogue, and complete in the Story Bible.

## Tool Workflow

1. Call `set_active_project` once per session to bind the active project and branch.
2. Call `find_entity` and `get_entity` before linking existing themes, locations, or characters to avoid duplicate or mismatched IDs.
3. Call `create_character` with core identity (`name`, `role`, `summary`, optional `realm`, `aliases`), `voice_profile`, `emotional_profile`, and `initial_state`.
4. Call `set_character_voice_profile` (or `batch_set_character_voice_profiles` for multiple cast members) whenever dialogue voice traits change after creation.
5. Call `commit_character_state` when a scene shifts a character's `emotional_state`, `goals`, `status`, or `notes`.
6. Call `create_relationship` and `update_relationship` to model directed trust/tension edges, and `create_character_arc` (linked to `create_theme` IDs) to track transformation milestones. Revise the character with `update_character` and the arc with `update_character_arc`; reserve `update_entity` for fields without a typed update.
7. Call `register_canonical_fact` for durable decision/physical/boundary facts, `get_character_snapshot` to verify the assembled profile, and `check_consistency` (`subjects: [character_id]`) to catch `voice_drift` or `canonical_fact_prose_drift`.

## Step 1: Inner Psychological Architecture

Before persisting fields, define the character's six-part inner engine:
1. **Ghost**: Formative backstory wound event (catastrophic, quiet, or poisoned success) stored in `summary` or via `register_canonical_fact`.
2. **Wound**: Present psychological scar tissue feeding `emotional_profile` (`suppressed`, `triggers`, `defense_mechanisms`).
3. **Lie**: Protective false belief the story will test (`starting_state` on the arc).
4. **Want**: Concrete external goal driving plot (`goals` in `initial_state`).
5. **Need & Truth**: Internal realization and accurate worldview opposing the Lie (`ending_state` and `thematic_purpose` on the arc across Positive Change, Negative Disillusionment/Fall/Corruption, or Flat/Steadfast arcs).

## Step 2: Voice, Emotional, and Initial State Profiles

### 1. Dialogue Voice Profile (`voice_profile` / `set_character_voice_profile`)
Prevents homogeneous dialogue across the cast:
- `tone`: Vocal posture (e.g. terse command cadence vs. wry self-deprecation).
- `vocabulary` & `sentence_structure`: Register, slang, clause complexity, contraction habits.
- `tics`: Sparingly used speech habits.
- `forbidden_words`: Words this character never says — enforced directly by the `voice_drift` validator in `check_consistency`.
- `example_lines`: 5–8 representative lines across emotional states so `scene-writer` can match cadence without copying.
- `established_in_scene_id`: Source scene when a voice shift occurs mid-book.

### 2. Emotional Profile (`emotional_profile`)
- `base_emotions`: Specific baseline emotion weights (`0.0..=1.0`).
- `suppressed`: Emotions the character refuses to feel until pressured.
- `triggers`: Specific situations that bypass their defenses.
- `defense_mechanisms`: How they self-protect (`humor`, `control`, `withdrawal`, `aggression`, `intellectualization`).
- `flex_range`: Optional out-of-range emotional mode under extreme pressure.

### 3. Decision, Physical, and Boundary Facts (`register_canonical_fact`)
Capture moral framework, risk tolerance, stress response (`fight/flight/freeze/fawn`), specific physical capabilities/injuries, and per-character content boundaries in `summary` or as typed facts via `register_canonical_fact`.

### 4. Initial Character State (`initial_state` / `commit_character_state`)
Seed baseline `emotional_state`, immediate `goals`, `status` tags, `notes`, and `source_summary` so `get_scene_context` has a valid pre-scene state from Scene 1 onward.

## Step 3: Relationships and Character Arcs

- **Relationships (`create_relationship`, `update_relationship`)**: Set `character_a_id`, `character_b_id`, `relationship_type`, `initial_trust`, `initial_tension`, and `dynamics`. Give every key relationship a source of conflict tied to at least one character's Lie and unequal or shifting leverage.
- **Character Arc (`create_character_arc`)**: Specify `arc_type` (`growth`, `fall`, `flat`, `transformation`, `corruption`, `healing`), `starting_state`, `ending_state`, 3–7 `milestones` that each challenge the Lie at a real cost, `thematic_purpose`, and `connected_theme_ids`.

## Step 4: Completeness & Pitfall Check

Call `get_character_snapshot` and verify: `example_lines` and `forbidden_words` are populated, `defense_mechanisms` and `triggers` exist, `initial_state` is seeded, and every visible quirk connects to the Ghost -> Wound -> Lie engine (avoiding flawless protagonists, disconnected quirks, unexamined tragic backstories, author mouthpieces, and voice twins). Run `check_consistency` (`subjects: [character_id]`) to confirm zero `voice_drift` or `canonical_fact_prose_drift` issues.

<!-- tool: batch_set_character_voice_profiles -->
<!-- tool: check_consistency -->
<!-- tool: commit_character_state -->
<!-- tool: create_character -->
<!-- tool: create_character_arc -->
<!-- tool: create_relationship -->
<!-- tool: create_theme -->
<!-- tool: find_entity -->
<!-- tool: get_character_snapshot -->
<!-- tool: get_entity -->
<!-- tool: get_scene_context -->
<!-- tool: register_canonical_fact -->
<!-- tool: set_active_project -->
<!-- tool: set_character_voice_profile -->
<!-- tool: update_character -->
<!-- tool: update_character_arc -->
<!-- tool: update_entity -->
<!-- tool: update_relationship -->
