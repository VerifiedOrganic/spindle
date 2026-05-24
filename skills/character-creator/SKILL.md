---
name: character-creator
description: >
  Use when the user wants to create, develop, flesh out, or deepen a character. This includes
  creating new characters from scratch, building character profiles (emotional, voice, decision,
  physical, content), establishing backstory, defining character arcs with the Ghost/Wound/Lie/Truth
  framework, setting up relationships, and creating voice sheets. Also triggers when the user says
  things like "I need a villain", "flesh out this character", "give me a love interest", "make
  this character more interesting", "what's their backstory", or any request about character
  development. If a character feels flat, underdeveloped, or generic, this skill fixes that.
---

# Character Creator

A flat character is the fastest way to kill a story. Readers don't commit to plots — they commit
to people. This skill guides you through creating characters that are psychologically complex,
narratively purposeful, and technically complete in the Story Bible system.

Every character you build must have these layers:
1. **Psychological depth** — Ghost, Wound, Lie, Want, Need, Truth
2. **Narrative purpose** — what role they serve in the story's thematic argument
3. **Sensory reality** — how they look, move, speak, and occupy physical space
4. **Technical completeness** — all five profiles, initial state, arc, relationships

## Tool workflow

1. Call `set_active_project` once per session so subsequent tool calls inherit
   the project (and active branch) without re-passing `project_id`.
2. Call `create_character` for core identity, summary, voice profile,
   emotional profile, and optional initial state. The voice and emotional
   profiles are seeded inline at create time.
3. Call `set_character_voice_profile` as the canonical voice authority
   whenever voice traits change after creation. Use
   `batch_set_character_voice_profiles` for bulk updates across many
   characters.
4. Call `commit_character_state` whenever the character's emotional state,
   goals, status, or notes change as a result of a scene.
5. Call `get_character_snapshot` to verify profile, state, and recent
   appearance context before handing off to drafting.
6. Call `find_entity` and then `get_entity` when relating this character to
   other entities to avoid alias/name mismatches.

Do not treat ad hoc prose notes as final voice canon. Persist voice changes
through `set_character_voice_profile` so downstream voice checks are grounded.
The live validator ID for this is `voice_drift`, surfaced through
`check_consistency`.

---

## The Psychology of a Compelling Character

Before touching any MCP tool, understand the character's inner architecture.
This framework comes from K.M. Weiland's character arc theory, itself grounded
in decades of craft tradition.

### The Ghost (Backstory Wound Event)

Every character who changes in a story was shaped by something that happened BEFORE
the story begins. This is the Ghost — a formative event that traumatized them,
deceived them, or broke something fundamental about how they see the world.

The Ghost doesn't have to be dramatic. It can be:
- **Catastrophic**: a parent's murder, a war, a betrayal that nearly killed them
- **Quiet**: a childhood of emotional neglect, being told they weren't good enough,
  watching a parent's marriage disintegrate
- **Positive-turned-poisonous**: being praised so much they became perfectionists,
  succeeding early and developing an entitled worldview

The Ghost is NOT backstory exposition to dump in chapter 1. It's the reason the
character is broken in a specific way that the story will test and (possibly) heal.

### The Wound (Psychological Impact)

The Wound is what the Ghost DID to the character's psyche. If the Ghost is the event,
the Wound is the scar tissue. It manifests as:
- Behavioral patterns (avoidance, aggression, people-pleasing, isolation)
- Emotional triggers (specific situations that cause disproportionate reactions)
- Defense mechanisms (intellectualization, humor, withdrawal, control)
- Coping strategies (some healthy, some destructive)

The Wound shapes everything the character does in the present. A character wounded
by abandonment will sabotage relationships to avoid being left. A character wounded
by failure will avoid risk at all costs — or become recklessly driven to prove themselves.

### The Lie the Character Believes

The Lie is the false belief the character adopted to survive their Wound. It feels
protective — it's how they've coped. But it's limiting them, damaging them, or
damaging the people around them.

Common Lies:
- "I'm not worthy of love" (wound: abandonment)
- "Trust always leads to betrayal" (wound: betrayal)
- "Vulnerability is weakness" (wound: exploitation)
- "I can only rely on myself" (wound: being let down by authority)
- "Power is the only safety" (wound: powerlessness)
- "If I'm perfect, I'll be safe" (wound: conditional love)

The Lie is the ENGINE of the character arc. The entire story tests this Lie
against the Truth until the character is forced to choose.

### Want vs Need

- **Want**: the external goal the character consciously pursues. "Find the killer."
  "Win the tournament." "Get the promotion." "Escape the kingdom." The Want drives PLOT.
- **Need**: the internal realization the character must reach. "Learn to trust again."
  "Accept that perfection isn't required." "Forgive themselves." The Need drives ARC.

The best stories make Want and Need intersect: the character CANNOT achieve their
external goal without first addressing their internal Need. This creates natural
pressure for change.

### The Truth

The Truth is what the character needs to learn — the accurate worldview that replaces
the Lie. In a Positive Change Arc, the character eventually embraces the Truth. In a
Negative Change Arc, they reject it and are consumed by the Lie. In a Flat Arc, they
already know the Truth and use it to change the world around them.

### Arc Types

| Arc Type | Lie → Truth | Example |
|----------|-------------|---------|
| **Positive Change** | Believes Lie → learns Truth → transforms | Most protagonists |
| **Negative Change (Disillusionment)** | Believes positive Lie → learns dark Truth | Tragic heroes |
| **Negative Change (Fall)** | Sees Truth → rejects it → embraces Lie | Walter White |
| **Negative Change (Corruption)** | Knows Truth → becomes the Lie | Anakin Skywalker |
| **Flat (Steadfast)** | Already knows Truth → tests it → holds firm | James Bond, Sherlock |
| **Flat (Impact)** | Knows Truth → changes the world around them | Atticus Finch |

---

## Step 1: Core Identity

Ask the user (or infer from context):
- **Name**: What are they called? Any nicknames?
- **Role in story**: protagonist, deuteragonist, antagonist, love interest, mentor,
  ally, foil, comic relief, threshold guardian
- **One-sentence summary**: Who are they in 15 words?
- **Realm**: Optional string naming the world or region they belong to.

Call `create_character` with the full typed input:
```
project_id, name, summary, role,
realm: Option<String>,
voice_profile: CharacterVoiceProfileData,
emotional_profile: CharacterEmotionalProfileData,
initial_state: Option<CharacterStatePatch>
```

Voice and emotional profiles are required at create time and may be edited
later with `set_character_voice_profile` (voice) or by appending state patches
via `commit_character_state` (mutable emotional state, goals, status).

---

## Step 2: The Inner Architecture

This is the most important step. Guide the user through building the character's
psychological foundation. Ask these questions in this order:

1. **What is their Ghost?** What happened to them before the story begins that
   shaped who they are now? (This becomes backstory in the character's summary.)

2. **What is their Wound?** How did the Ghost damage them? What patterns do they
   fall into because of it? (This feeds the emotional_profile: suppressed,
   triggers, defense_mechanisms.)

3. **What Lie do they believe?** What false worldview did they adopt to survive?
   (Capture this in the summary and later character arc design.)

4. **What do they Want?** External, concrete, measurable goal. (This becomes
   active_goals in character_state.)

5. **What do they Need?** Internal truth they must learn. (This feeds the
   character_arc: ending_state, thematic_purpose.)

6. **What is the Truth?** The accurate worldview that opposes their Lie.
   (This feeds the character_arc: ending_state and connected_themes.)

---

## Step 3: Build the Five Profiles

For each profile, translate the psychological foundation into concrete fields.

### Emotional Profile

The emotional profile defines how the character FEELS, not just on the surface
but in the hidden layers they try to suppress.

`CharacterEmotionalProfileData` (set at `create_character` time) has these
fields:

- **base_emotions** (`BTreeMap<String, f32>`): Map of emotion name to baseline
  intensity (0.0–1.0). Not "happy" — that's generic. "sardonic_amusement: 0.6,
  low_grade_anxiety: 0.4" tells you something real.
- **suppressed** (`Vec<String>`): What they refuse to feel. Characters who
  suppress grief become brittle. Characters who suppress anger become
  passive-aggressive. The suppressed emotion will ERUPT at a key moment in the
  arc.
- **triggers** (`Vec<String>`): What bypasses their emotional defenses. A
  tough warrior who melts around children. A cynic who can't resist helping
  strays. A cold strategist who loses composure when reminded of their Ghost.
- **defense_mechanisms** (`Vec<String>`): How they protect themselves
  emotionally. Examples: "intellectualization", "humor", "aggression",
  "withdrawal", "control", "projection".
- **flex_range** (`Option<FlexRange>`): A struct, not a scalar. Captures how
  the character's emotional range expands or contracts under specific
  conditions. Leave `None` for steady-state characters; populate when the
  character has a meaningful "out of normal range" mode.

### Voice Profile

The voice profile defines how the character SOUNDS. This is what makes every
character in a scene distinguishable by dialogue alone.

**Critical insight**: AI models default to homogeneous prose voice. Without a
detailed voice profile, every character will sound like the same articulate,
well-spoken narrator. Voice profiles are your primary defense against this.

`CharacterVoiceProfileData` is set inline at `create_character` time and
edited later via `set_character_voice_profile`. The exact fields:

- **tone** (`Option<String>`): Free-form short description of overall vocal
  posture. Examples: "Terse, command cadence. Never elaborates without
  reason." / "Wry, self-deprecating, defaults to humor under pressure."

- **vocabulary** (`Vec<String>`): Concrete vocabulary descriptors and
  signature word choices. Examples:
  `["academic register", "military slang from service years", "uses 'one' instead of 'you'", "never contracts"]`

- **sentence_structure** (`Vec<String>`): How they build sentences.
  `["short declarative", "rare subordinate clauses", "speaks in orders even when asking"]`
  or `["long winding parenthetical", "interrupts self with qualifications"]`

- **tics** (`Vec<String>`): Recurring speech patterns, used sparingly.
  `["starts sentences with 'Look—' when frustrated", "tags statements with 'yeah?'", "whistles through teeth when thinking"]`

- **forbidden_words** (`Vec<String>`): Words this character would NEVER say.
  These are enforced by the `voice_drift` validator in `check_consistency`.
  `["please", "thank you"]` / `["fuck", "shit"]` / `["love"]`

- **example_lines** (`Vec<String>`): 5-8 example lines showing the character's
  voice in different emotional states. This is the most important part — it
  gives the scene-writer skill concrete examples to pattern-match against.

  ```
  ["Weather's turning. We should move before the pass closes.",
   "You don't get to stand there and tell me what I lost. You weren't there.",
   "Hey. Look at me. ...Yeah. That's what I thought.",
   "Left flank's gone. We fall back to the ridge or we die here. Choose.",
   "Last time I trusted a map, I ended up in a swamp. This time, let's just follow the river."]
  ```

- **established_in_scene_id** (`Option<String>`): When updating voice canon
  to reflect a moment in the story, pass the `scene_id` that established the
  shift so future readers can trace the change.

After persisting voice canon with `set_character_voice_profile`, run
`check_consistency` with `checks=["voice_drift"]` on recent scenes and correct
dialogue drift directly in prose. The validator surfaces every forbidden-word
hit in scenes where this character participates.

Example:
- `forbidden_words` includes "buddy"; tone says terse command cadence.
- Drifted line: "Listen, buddy, I think we can negotiate this."
- Revised line: "No negotiation. Move, now."

### Decision, physical, and content notes (no first-class DTO)

These three dimensions are not stored as separate profile records. The shipped
DTO surface for a character is `voice_profile`, `emotional_profile`, and
`initial_state`. Encode the considerations below in the character's `summary`,
in the arc's `thematic_purpose` and `starting_state`/`ending_state`, or as
canonical facts via `register_canonical_fact` (predicate names like
`decision.moral_framework`, `physical.limits`, `content.boundaries`).

**Decision considerations** to capture in the summary or canonical facts:
- Moral framework: consequentialist? rule-follower? pragmatic survivalist?
- Decision speed: instant in combat, slow in relationships?
- Risk tolerance: paralyzed by risk vs reckless.
- Rule-break conditions: will kill to protect innocents but won't torture?
- Stress response: fight / flight / freeze / fawn, conditional on situation.
- Breaking point: the scenario that would shatter composure (a narrative
  weapon — save for a climactic moment).

**Physical considerations** to capture similarly:
- Specific combat skill ("trained fencer, weak on left side from old injury"),
  not generic ("good fighter").
- Current fitness, chronic conditions, old injuries.
- Stamina and pain tolerance ranges.
- Limitations that create interesting constraints in scenes.

**Content considerations** (only relevant if the project handles explicit
content): there is no shipped content-boundary tool. Capture per-character
comfort tags, hard limits, and contextual stretch conditions as canonical
facts the LLM consults during scene drafting. The historical
`compute_content_boundaries` / `create_handoff` workflow no longer ships and
should not be referenced.

---

## Step 4: Initial Character State

The `initial_state` field on `create_character` seeds the character's first
state snapshot before any scene is written. Subsequent state changes are
appended via `commit_character_state` with a `scene_id` (the snapshot binds to
the scene that caused the change).

`CharacterStatePatch` fields:

- **emotional_state** (`Option<Vec<String>>`): Their emotional state. Examples:
  `["resolute", "guarded", "exhausted"]`. This is their baseline before the
  plot disrupts everything.
- **goals** (`Option<Vec<String>>`): What they want RIGHT NOW (not their arc
  destination). Examples: `["survive the night", "find the witness"]`.
- **status** (`Option<Vec<String>>`): Status tags. Examples:
  `["alive", "armed", "injured-left-shoulder"]`.
- **notes** (`Option<Vec<String>>`): Free-form notes the LLM should remember.
- **source_summary** (`Option<String>`): One-line description of why this
  state was committed. Useful for traceability when reading state history.

The initial state seed is what `get_scene_context` will return for scene 1.
Every later commit appends a snapshot tied to its source scene; reads resolve
the latest snapshot strictly before the requested
`(book_number, chapter_number, scene_order)`.

---

## Step 5: Establish Relationships

For every significant relationship, call `create_relationship`:

- **character_a_id** / **character_b_id** (`String`): Record IDs of the two
  characters. The edge is directed from A to B.
- **relationship_type** (`String`): "rivals", "mentor_student", "siblings",
  "lovers", "commander_subordinate", "former_friends", "strangers_forced_together"
- **initial_trust** (`i32`): Integer trust level. Negative = hostility,
  positive = trust. Start lower than you think — trust that's too high has
  nowhere to go.
- **initial_tension** (`i32`): Integer tension level (0 = none, higher = more
  strained). Tension drives scenes. Relationships with zero tension are boring.
- **dynamics** (`Vec<String>`): Free-form descriptors of the relationship's
  texture. Examples: `["asymmetric power", "shared trauma", "rivalry over A's
  approval"]`.

Update relationships as the story evolves with `update_relationship`, which
takes `character_a_id`, `character_b_id`, `trust_delta`, `tension_delta`,
`reason`, and `scene_id`. The edge stores the source `scene_id` so reads can
trace when each shift happened.

**Relationship design principles:**
- Every relationship should have at least one source of CONFLICT.
  Perfect harmony is unreadable.
- The conflict should connect to at least one character's Lie.
  Marcus can't trust Elena because his Lie says trust leads to betrayal.
  This means the relationship IS the testing ground for his arc.
- Power should be unequal or shifting. Perfectly balanced relationships
  feel static. Who has leverage? Who needs whom more? Does it shift?

---

## Step 6: Define the Character Arc

Call `create_character_arc` with:

- **arc_type**: growth, fall, flat, transformation, corruption, healing
- **starting_state**: The character at the beginning — dominated by the Lie.
  "Distrustful loner who pushes everyone away to avoid being hurt again."
- **ending_state**: The character at the end — transformed by the Truth (or consumed by the Lie).
  "Still cautious but capable of trusting selected people and accepting vulnerability."
- **milestones**: 3-7 key turning points across the arc. Each milestone should:
  - Challenge the Lie specifically
  - Cost the character something
  - Move them toward or away from the Truth
  - Unlock state changes (new emotional capacity, new behavior, etc.)

  Example milestones for a trust arc:
  ```
  1. Forced reliance (ch 5): Must depend on Elena for survival. Hates it.
  2. Small trust rewarded (ch 12): Elena keeps a secret for him. Notices.
  3. Moment of vulnerability (ch 20): Opens up about the Ghost. Terrifying.
  4. Trust betrayed (ch 28): Elena does something that LOOKS like betrayal.
     Tests whether he falls back to the Lie.
  5. Choosing trust (ch 35): Chooses to believe Elena despite evidence
     against her. This is the Truth overcoming the Lie.
  6. Trust reciprocated (ch 40): Elena trusts HIM with something critical.
     Full circle.
  ```

- **thematic_purpose**: What does this arc SAY about the human condition?
  "Explores whether vulnerability is weakness or strength."
- **connected_theme_ids** (`Vec<String>`): Record IDs of theme records in the
  Bible (created via `create_theme`). Use `find_entity` to resolve theme IDs
  by name.

After creating the arc, the system creates a `pacing_tracker` for it. The
**scene-writer** skill will read this tracker during context assembly and
adjust its writing to respect the pacing budget.

---

## Step 7: Verify Completeness

Call `get_character_snapshot` to read back voice profile, latest state, recent
appearances, and active relationships in one shot. Scan for missing pieces:

- Voice profile has `example_lines` (critical for scene writing).
- Voice profile has `forbidden_words` for the `voice_drift` validator to bite on.
- Emotional profile has at least `defense_mechanisms` and `triggers`.
- Initial state was seeded.
- At least one arc and one relationship exist.
- The Lie/Truth are captured in the arc's `starting_state` / `ending_state`.
- The Ghost is captured in the character `summary` or as a canonical fact.

Then call `check_consistency` with `subjects: ["<character_id>"]` and
`format: "markdown"` to scope a Phase 4 validator pass to scenes that
reference this character. The shipped validators are:

- `voice_drift` — flags forbidden_words in scenes where this character
  participates.
- `canonical_fact_prose_drift` — flags scenes that mention canonical facts
  about this character without honoring the canonical value.
- `world_rule_semantic_drift` — flags world-rule violations (not character-specific).
- `retcon_reachability` — flags scenes that reference temporal interventions
  before their reachable timeline anchor.

The completeness audit above is a craft checklist; the validators are the
enforcement layer.

---

## Skill Chains

This skill connects to others:

- **→ worldbuilder**: If the character belongs to a faction, religion, or location
  that doesn't exist yet, switch to the worldbuilder skill to create it first.
- **→ plot-architect**: After creating the character, use the plot-architect skill
  to integrate their arc into the story's conflict structure and pacing plan.
- **→ scene-writer**: Once the character is complete, the scene-writer skill can
  write scenes featuring them with full context.
- **→ continuity-editor**: After scenes are written, the continuity-editor can verify
  the character remains consistent with their profiles across all appearances.

---

## Common Character Creation Mistakes

### "The Perfect Protagonist"
A character with no flaws, no Lie, and no inner conflict. They're good at
everything, liked by everyone, and always make the right choice. This character
has nowhere to go. FIX: Give them a Wound so deep it makes them terrible at
the one thing the story requires.

### "The Quirk Machine"
A character defined entirely by surface traits — purple hair, loves cats, afraid
of heights — with no psychological depth beneath. Quirks aren't character.
FIX: Every visible trait should connect to the invisible architecture.
Afraid of heights because they fell as a child AND their parent didn't catch them
(Ghost → Wound → Lie: "no one will catch me if I fall").

### "The Backstory Orphan"
A character whose Ghost is "my parents died" with no exploration of how that
shaped their worldview, Lie, or behavior patterns. The Ghost is just tragic
decoration. FIX: The Ghost must cause a specific Wound that creates a specific
Lie that manifests as specific behavior that the story specifically challenges.

### "The Mouthpiece"
A character who exists to deliver the author's opinions. They don't have their
own perspective — they have the "correct" perspective. They lecture other
characters. FIX: Give them a Lie that makes their "correct" views cost them
something. Or make them wrong about something important.

### "Voice Twins"
Two characters who sound identical in dialogue. Same vocabulary, same sentence
length, same emotional register. FIX: Write 5 example lines for each character
independently, then read them side by side. If you can't tell who's speaking,
the voice profiles need more differentiation.

---

## References

For deeper craft knowledge, read the embedded craft references via
`bible://references/<name>` resources:

- `bible://references/voice-differentiation` — Voice differentiation
  techniques, dialect construction, verbal tic design, and worked examples.
- `bible://references/anti-slop` — Avoiding generic AI prose patterns when
  drafting dialogue and characterization.
- `bible://references/swain-scene-sequel` — Scene/sequel structure and how
  characters reveal under pressure.
- `bible://references/mru-guide` — Motivation-Reaction Unit construction for
  showing character interiority moment-to-moment.
