# Scene Structure, Voice Craft, and Failure Modes

Load this reference when structuring complex scene/sequel turns, calibrating line-level MRU and dialogue craft, or diagnosing narrative failure modes.

## 1. Swain's Scene / Sequel Framework

Use Scene and Sequel patterns in service of the project's genre and reader contract. Blend, compress, or adapt them as the book requires; never force a formulaic disaster when the contract calls for a different turn.

- **SCENE (Action: Goal -> Conflict -> Disaster)**
  - **Goal**: Concrete, immediate want established early so the reader can measure progress.
  - **Conflict**: Escalating opposition that tests commitment through try-fail cycles.
  - **Disaster / Turn**: Ends on **No**, **No, and furthermore** (escalation), **Yes, but** (costly win), or **Yes** (clean win, reserved for earned milestones).
- **SEQUEL (Reaction: Reaction -> Dilemma -> Decision)**
  - **Reaction**: Emotional/physical response to the turn, rendered at the book's narrative distance.
  - **Dilemma**: Hard choice among costly options that reveals character through action.
  - **Decision**: Commitment that becomes the next scene's goal.
- **Pacing proportion**: Short/absent sequels speed pacing; extended sequels deepen aftermath. Record beats (`goal`, `conflict`, `disaster`, `reaction`, `dilemma`, `decision`, `revelation`, `reversal`, `escalation`, `calm`, `transition`) with `plan_chapter` or `annotate_scene_beats`.

## 2. Causality, Voice, and POV Discipline

- **MRU causality**: Keep stimulus -> response clear without forcing a bodily reaction, action beat, and speech line into every paragraph. Do not re-explain an emotion after action or dialogue has already shown it.
- **Dramatization vs. summary**: Dramatize pivotal choices, conflict, and intimacy; use concise summary for transitions and orientation. Avoid replacing named emotions with stock organ clichés (clenched jaws, dropped stomachs, held breath).
- **Character voice**: Anchor each speaker in `get_character_snapshot` and prior scenes (`list_chapter_scenes`, `get_entity`). Differentiate vocabulary, sentence length, verbal tics, forbidden words, formality range, and interruption habits. Keep dialogue tags unobtrusive (`said`, `asked`) unless a colored tag earns its place.
- **POV filter**: Restrict internal thoughts and knowledge strictly to the POV character. Filter sensory details and emotional coloring through what *this* POV character notices and values.

## 3. Common Failure Modes and Fixes

- **White Room Syndrome**: Dialogue in a sensory void. *Fix*: Open with a concrete detail tying the POV character's body or task to the physical room.
- **Teleporting / Drifting Time**: Unsignaled morning-to-night jumps or prose contradicting its own clock (`temporal_coherence` in `check_consistency`). *Fix*: Anchor scene openings to `[IN-WORLD TIME]`, render multi-hour or multi-day spans (`duration_days >= 1` on `set_scene_clock`) with grounded place + body + mundane-task transitions (avoiding bare `"Hours passed."` fades that trip `solitary_fade`), and mark deliberate rewinds with `temporal_mode: "flashback"`.
- **Talking Heads**: Pages of ungrounded back-and-forth speech. *Fix*: Give speakers a physical task that can go wrong or reveal subtext.
- **Info Dump**: Narrator lecturing on lore or backstory. *Fix*: Surface facts only through active conflict, discovery, or immediate stakes.
- **Passive Protagonist**: Events happen *to* the lead (`scenes_since_active_choice >= 3`). *Fix*: Force a costly, plot-driving choice in this scene.
- **Emotional Whiplash**: Instant mood flips from grief to banter. *Fix*: Earn transitions through sequel beats.
- **Homogeneous Voices**: Interchangeable dialogue across cast members. *Fix*: Re-read each character's voice profile and prior lines; strip dialogue tags to test whether each speaker remains identifiable.
- **Stakes Deflation**: Stated threats fail to bite. *Fix*: Enforce `world_rules` and deliver real costs when a plan goes wrong.

<!-- tool: annotate_scene_beats -->
<!-- tool: check_consistency -->
<!-- tool: get_character_snapshot -->
<!-- tool: get_entity -->
<!-- tool: list_chapter_scenes -->
<!-- tool: plan_chapter -->
<!-- tool: set_scene_clock -->
