---
name: scene-writer
description: >
  The core fiction writing skill. Use whenever the user asks to write, draft, continue, or generate
  a scene, chapter, or any narrative prose. Also triggers when the user says things like "write the
  next scene", "continue the story", "draft chapter 15", "what happens next", or any request that
  involves producing fiction prose. This skill teaches you how to use the Story Bible MCP tools to
  assemble context, plan scene structure using Swain's scene/sequel framework, write prose that
  follows Motivation-Reaction Unit discipline, enforce show-don't-tell at the line level, maintain
  distinct character voices, respect POV constraints, and handle all post-scene state updates.
  If the user mentions writing, drafting, or generating any fiction content, use this skill.
---

# Scene Writer

You are writing fiction that must deliver on **this project's declared genre and reader
contract** — not generic "literary" prose. Before you write a word, read the project's style
contract: the reader contract (`promise`, `style_notes`, `boundaries`), any world rules of type
`style`, and the narrator-voice directive. Scene context surfaces all of these together as a
**Project Style Requirements** block at the top of the payload. They define what the prose must
FEEL like to its reader, and they are the PRIMARY quality metric — above every craft technique
in this skill.

Genre compliance is not optional, and it overrides general craft guidance wherever the two
conflict. A comedy webnovel must be funny, fast, and voice-driven; if your scene is not funny,
it has failed no matter how well-crafted it is. A dark romance must be tense and charged. A
thriller must propel. If the style contract says "raunchy modern comedy" and your prose reads
like a contemplative grief memoir, you have broken the contract — beautiful sentences do not
redeem a scene that betrays its genre. If a chapter is supposed to end on a hook and yours ends
on a quiet reflective beat, that is a failure, not a stylistic choice. When in doubt, write the
thing the target reader actually came for.

The craft techniques below (Swain's scene/sequel, MRU discipline, show-don't-tell) are tools,
not goals. Use them in service of the declared genre and voice — never let them pull the prose
toward prestige-literary defaults the contract did not ask for. This skill also teaches you how
to use the Story Bible system to maintain consistency across a potentially massive series.

## Canonical scene loop

Follow this loop for every drafting pass on an active branch:

0. Call `set_active_project` once per session so subsequent tool calls inherit
   the project (and active branch) without re-passing `project_id`. The
   session default is also restored on `set_active_project` after a reconnect.
1. Call `get_writer_state` next. This is the re-anchor contract for fresh
   or compressed sessions and pairs with `update_writer_position` for cursor
   handoff.
2. Call `get_chapter_briefing` for the target chapter/scene. Read the
   `continuity_sheets` and the matching `## Continuity sheets` markdown before
   drafting; these are the handoff contract for character details, habits,
   voice, current state, relationships, recent appearances, and location
   continuity.
3. Call `get_scene_context` for the target scene scope. Use
   `find_scenes_referencing` when you need to locate every scene that mentions
   a character, location, faction, or other entity before drafting or revising.
4. Draft with `save_scene_draft`.
5. Review `save_scene_draft` output and iterate until it is acceptable:
   `pacing_warnings`, optional `agency_warning`, `tone_deviation`,
   `style_warnings` (genre-voice mismatches against the style contract),
   `content_rating_valid` / `content_rating_warnings`, text diff metadata
   (`diff`, `byte_offsets_changed`, `chars_added`, `chars_deleted`), and the
   immediate validator findings already returned on the draft response:
   `world_rule_hits`, `voice_drift`, and `retcon_findings`. Then run the
   Step 5a genre/style self-check. For canonical-fact prose drift and broader
   cross-scene validation, run `check_consistency` before finalizing.
6. Run the post-draft validator loop with `check_consistency` and explicitly
   evaluate these live validator IDs (all run by default):
   - `canonical_fact_prose_drift`: prose conflicts with typed canonical facts.
   - `world_rule_semantic_drift`: prose semantically conflicts with world-rule
     expectations.
   - `voice_drift`: dialogue conflicts with character voice profiles.
   - `retcon_reachability`: knowledge/reachability contradictions.
   - `style_compliance`: prose tone/voice conflicts with the project style
     contract (reader contract style_notes, `style` world rules, narrator
     voice) — e.g. literary-contemplative markers or a no-hook ending on a
     genre that asks for comedy and hooks.

   `check_consistency` accepts `subjects: Vec<String>` (record IDs to narrow
   the scenes scanned — e.g. `["character:abc123"]` to only validate scenes
   where a specific character appears), `format: "markdown"|"json"`, and
   `budget_tokens` for trimming the rendered report. The output also includes
   `report_sections` grouped by validator, and `markdown` when format is
   markdown.
   Example triage:
   - If prose says "Cole is 19" but canon has `cole.age = 20`, fix the prose
     or supersede canon through the canonical-fact workflow.
   - If Jim Dalton uses a forbidden phrase in dialogue, revise line wording to
     match his persisted voice profile.
   - If a scene implies knowledge not yet learned, revise the scene to remove
     the leak or add an earlier discovery scene.
7. Call `commit_scene_changes` to persist structured canon updates from the
   accepted prose.
8. Call `commit_character_state` only for targeted state corrections not
   covered by the batch commit.
9. Call `update_writer_position` whenever you need cursor state persisted for
   handoff or pause/resume workflows.

## Before You Write Anything

Before drafting a single word of prose, you must complete four steps:
1. **Re-anchor the session** — call `get_writer_state` first to recover the active branch, cursor scene, hard constraints, open promises, overlays, divergence warnings, and recent activity
2. **Read the continuity sheets** — call `get_chapter_briefing` and lock in character details, habits, voice, current state, recent appearances, and location continuity
3. **Plan the scene structure** — decide what type of scene this is and outline its beats
4. **Check constraints** — read the pacing directives, agency check, knowledge briefing, and world rules

Skipping any of these steps produces generic, context-free prose that contradicts the
established story. Never skip them.

---

## Step 1: Re-anchor and assemble context

At session start, call `get_writer_state` before any other writing tool. Treat it as the
single re-anchor packet for a compressed or fresh model session. Read:
- `current` for the active project, branch, chapter, and scene
- `next` for the intended continuation target
- `hard_constraints`, `open_promises_due_now`, and `active_overlays` for non-negotiable continuity state
- `unsynced_local_files` and `drift_warnings` before you trust local manuscript files
- `recent_session_activity` to see what changed most recently

Also check `bible://config/routing` once at session start for whichever
`(route, rating)` pairs you expect to invoke (typically `draft` default and
`draft` + `explicit`). If a rule has `caller_should_send_brief: true`, the
resolved adapter pulls bible canon on demand via the spindle MCP server and
you should **send a short brief** to that route instead of pre-packing
context — see *Step 4 → Prompt strategy: brief vs packed* for the full
contract. If false, gather the canon below as normal.

After reading `get_writer_state`, call `get_chapter_briefing` for the target
chapter/scene. If the briefing has `continuity_sheets`, treat them as required
input for drafting. If the server does not provide continuity sheets, call
`get_character_snapshot` for every character present before drafting.

(Skip the heavy canon gathering below if every route you plan to invoke has
`caller_should_send_brief: true` — the spawned agent will fetch what it
needs itself.)

Then call `get_scene_context` with:
- project, book, chapter, scene_order
- characters present (by record ID)
- location
- budget_tokens (default 8000, increase for complex scenes)
- optional `sections` when you only need part of the payload; for example,
  `characters`, `relationships`, `agency_check`, or `standards`

If you need the chapter spine before drafting or revising, call
`list_chapter_scenes` for the current chapter. If you need the wider book
order, call `list_book_chapters`. Use these instead of project-wide scene
enumeration when you are deciding where the new scene fits.

The two responses together contain everything you need. Read them carefully — every field matters:

### Project Style Requirements (from `novel.style_directive`)
This is the consolidated style contract for the project, assembled from three sources and
surfaced together so you cannot miss it:
- **`reader_contract.promise`** — what the story is fundamentally promising the reader.
- **`reader_contract.style_notes`** — the *positive* prose requirements: tone, comedy density,
  pacing feel, voice. These are not optional flavor; they are what the prose must deliver. If a
  style note says "raunchy modern comedy tone," every scene must be raunchy and funny.
- **`reader_contract.boundaries`** — what to keep out (e.g., "focus on comedy over dark themes").
- **`style`-typed world rules** — rendered with a `[STYLE DIRECTIVE]` marker. Treat these as
  mandatory prose-level instructions, not background lore. They often encode anti-patterns
  ("no grief-beat endings," "no contemplative literary pacing") that you must obey.
- **`narrator_voice`** — for first-person or close-third narration, the narrator's voice IS the
  prose style of the book. Read its `emotional_register`, `pacing_feel`, `comedy_density`,
  `interiority_ratio`, and `chapter_ending_style` and write to them. This is distinct from any
  single character's dialogue voice profile: it governs the whole reading experience.

Enforce these as you draft, not just at the boundaries. The contract is violated **both** when
you add something forbidden (a slapstick scene in a grimdark series; killing the love interest
in a romance without license) **and** when you fail to deliver something promised (a comedy that
isn't funny; a webnovel chapter that ends on a quiet grief beat instead of a hook). If your
scene would do either, STOP and rewrite before saving. "Beautifully written" is not a defense
against "wrong for this book."

### Pacing Directives (from `novel.pacing_directives`)
For each active character arc, you'll see:
- **Current progress**: where the arc is (0.0 = start, 1.0 = complete)
- **Budget remaining**: how much progress is left for this book
- **Velocity**: how fast the arc has been moving recently
- **Status**: "on_track", "behind", "ahead", "in_cooldown", "stalled"
- **Next milestone**: what the next turning point should be
- **Warnings**: specific pacing problems to address

**How to use pacing directives:**
- If an arc is "behind": this scene must advance it. Include a beat that moves it forward.
- If an arc is "ahead": slow down. Don't advance it further. Let tension simmer.
- If an arc is "in_cooldown": the arc just had a sprint. This scene should NOT touch it.
  Let the character process. Focus on other arcs or subplots.
- If an arc is "stalled": this is a problem. The reader is getting bored. Make something happen.

### Agency Check (from `scene.agency_check`)
- **scenes_since_active_choice**: how many scenes since the protagonist last drove action
- **warning**: direct reminder text when the protagonist has been passive too long
- If this number is 3 or higher, THIS SCENE MUST have the protagonist make an active choice
  that drives the plot. The protagonist cannot be passive, reactive, or dragged through events.

### World Rules (from `novel.world_rules`)
These are the established rules of the story's world. NEVER violate them.
- If magic requires physical contact, no character can cast at range
- If FTL travel takes 3 days, no one arrives instantly
- If the council executes traitors, that must be a real threat, not an empty one

If a solution appears in your scene, it MUST use only capabilities established in the Bible.
Introducing a new power, item, or ability to solve a problem without prior setup is a
deus ex machina. The reader will feel cheated.

### Knowledge Constraints (from `novel.knowledge_briefing`)
Each item is a briefing fact available to the current scene assembly. Check:
- What does the POV character know? They can think about these things.
- What do they NOT know? They cannot reference, think about, or act on unknown information.
- What do they know that others don't? This creates dramatic irony — the reader feels tension
  when a character doesn't know what they know.
- If a character has future knowledge (time travel), check confidence levels. Degraded
  foreknowledge creates interesting uncertainty — the character isn't sure if their knowledge
  still applies.

### Narrative Promises Due (from `novel.narrative_promises_due`)
If any Chekhov's guns, foreshadowing, or setups are overdue for payoff, weave the payoff
into this scene if narratively appropriate. Don't force it, but don't ignore it either.
Unfired guns accumulate as narrative debt that erodes reader trust.

### Semantic References (from `novel.semantic_references`)
These are optional recall hits from the Bible search index. Use them as supporting canon,
not as the primary source of truth. If the token budget is tight, this list may be empty.

---

## Step 2: Plan the Scene Structure

Every scene must have a PURPOSE. Before writing, answer:
1. **What is this scene's job?** (advance a conflict, deepen a relationship, reveal information, etc.)
2. **What changes by the end?** If nothing changes, the scene has no purpose. Cut it.
3. **What is the POV character's scene-level goal?** (must be specific and measurable)

### Swain's Scene/Sequel Framework

Every scene falls into one of two types. Alternate between them to create rhythm.

**SCENE (action)** — structured as Goal → Conflict → Disaster
- **Goal**: The POV character wants something specific in THIS scene. Not "save the world" —
  that's a story goal. A scene goal is: "convince Elena to share her intelligence", "find the
  hidden passage before the guards return", "survive the ambush at the bridge."
  The reader must know the goal EARLY so they can measure progress and feel tension.
- **Conflict**: Obstacles prevent the goal. This is NOT just "bad things happen." It's
  opposition that TESTS the character's commitment. Each obstacle should escalate.
  The character tries, adjusts, tries harder. This is where try-fail cycles live.
- **Disaster**: The scene ends with one of four outcomes:
  - **No** — goal is not achieved (most common, drives urgency)
  - **No, and furthermore** — goal fails AND something worse happens (escalation)
  - **Yes, but** — goal achieved BUT at an unexpected cost (pyrrhic victory)
  - **Yes** — goal achieved cleanly (use sparingly, usually only at major turning points)

**SEQUEL (reaction)** — structured as Reaction → Dilemma → Decision
- **Reaction**: The character's emotional and physical response to the previous disaster.
  This is where the reader processes what happened. Don't skip this. Readers need breathing room.
  Show the emotion — don't name it. (See MRU section below.)
- **Dilemma**: The character faces a choice with no good option. Every path has costs.
  This is where character is revealed — not by what they say, but by what they choose
  when the stakes are real and the options are bad.
- **Decision**: The character commits to a course of action. This decision becomes the
  GOAL of the next scene. The cycle continues.

**Pacing control through scene/sequel proportion:**
- Fast pacing: short or absent sequels, rapid scenes
- Slow pacing: long sequels with deep emotional exploration
- The Dark Night of the Soul: the longest sequel in the story

### Scene Beats

After choosing the scene type, outline the beats. Use `plan_chapter` or `annotate_scene_beats`
to record them. Standard beat types:

| Beat | Purpose | Swain Element |
|------|---------|---------------|
| goal | Establish what POV character wants | Scene: Goal |
| conflict | Opposition to the goal | Scene: Conflict |
| disaster | Scene outcome (fail, yes-but, etc.) | Scene: Disaster |
| reaction | Emotional/physical response | Sequel: Reaction |
| dilemma | Character faces hard choice | Sequel: Dilemma |
| decision | Character commits to action | Sequel: Decision |
| revelation | New information changes the situation | Any |
| reversal | Expectation is subverted | Any |
| escalation | Stakes or tension increase | Any |
| calm | Breathing room, worldbuilding, connection | Any |
| transition | Move between locations/times | Any |

---

## Step 3: Write the Prose

Now write. Follow these rules at every level — from paragraph to sentence to word.

### Motivation-Reaction Units (MRUs) — The Line-Level Structure

Every paragraph of your prose should follow the MRU pattern from Dwight Swain.

**Motivation** (external stimulus) → **Reaction** (character's response)

The Motivation always comes FIRST. It's something external to the POV character:
another character's words, a sound, a sight, a physical sensation from the environment.

The Reaction follows, always in this order:
1. **Feeling** (involuntary emotional/physical response — gut clench, heat in face, heart rate)
2. **Action** (physical response — stepping back, clenching fists, reaching for weapon)
3. **Speech** (verbal response — dialogue)

Not every reaction needs all three. Minor stimuli may only trigger speech. Major turning
points should include all three in order.

**WRONG ORDER** (reaction before motivation):
> Elena screamed when the knife embedded in the door frame inches from her face.

**RIGHT ORDER** (motivation first, then reaction in feeling→action→speech):
> The knife thunked into the door frame, splinters spraying her cheek. [MOTIVATION]
> Ice flooded her veins. [FEELING]
> She threw herself sideways, shoulder crashing into the stone wall. [ACTION]
> "You missed," she said, her voice steadier than her hands. [SPEECH]

### Show, Don't Tell — The Fundamental Discipline

This is the single most important prose rule and the one AI models violate most often.

**TELLING** names the emotion: "Marcus felt angry." "Elena was sad." "He was nervous."
**SHOWING** renders the emotion through observable behavior, physical sensation, and action.

| Telling (NEVER) | Showing (ALWAYS) |
|-----------------|-------------------|
| Marcus felt angry | Marcus's jaw clenched. His knuckles whitened on the sword grip. |
| Elena was sad | Elena's gaze dropped to her hands. She picked at a loose thread on her sleeve. |
| He was nervous | A bead of sweat traced his temple. He checked the door for the third time. |
| She felt betrayed | Something cracked behind her ribs — not pain, exactly, but the sound a frozen lake makes before it gives way. |
| They were in love | His thumb traced the scar on her wrist, the one she never talked about. She let him. |

**Rules for show-don't-tell enforcement:**
- NEVER use "felt [emotion]", "was [emotion]", "seemed [emotion]"
- NEVER use "he/she realized" — show the realization through behavior change
- NEVER summarize an emotional exchange — dramatize it with dialogue and action
- Physical sensation > named emotion. "His stomach dropped" beats "he was afraid."
- Subtext > text. Characters rarely say exactly what they mean, especially about feelings.
- If you catch yourself writing "she felt a wave of [X]" — STOP and rewrite.

### Character Voice

Every character must sound like themselves, not like each other and not like the narrator.

For each character in the scene, infer their live voice from the context you do
have:
- the character summary, goals, status, and notes in `scene.characters`
- prior scene text or summaries surfaced through semantic references
- any imported or canonical knowledge that changes how guarded or direct they are

Then preserve continuity in:
- **Vocabulary level**: A street thief doesn't use academic language. A scholar doesn't use slang.
- **Sentence structure**: Short, punchy sentences for terse characters. Complex clauses for thinkers.
- **Verbal tics**: Use sparingly but consistently. If Marcus says "hells" as a mild oath, he always does.
- **Forbidden words**: If a character would NEVER say "please", they don't.
- **Signature phrases**: Weave them in naturally, not as a catchphrase.
- **Profanity level**: Match the character, not the author's preference.
- **Formality range**: Characters speak differently to their superior, their lover, and a stranger.

**Dialogue rules:**
- No dialogue tags except "said" and "asked" (invisible to readers). Never "exclaimed",
  "muttered", "declared", "retorted". SHOW the manner of speech through action beats.
- Avoid adverbs on dialogue tags entirely. Not "she said quietly" — "she dropped her voice."
- Every line of dialogue should do at least one of: advance the plot, reveal character, or increase tension. Preferably two.
- Characters interrupt each other, trail off, avoid questions, change the subject.
  Real people don't take turns delivering perfect paragraphs.

### POV Discipline

The POV character is a camera and a filter. Everything the reader experiences comes through them.

- **Internal access**: Only the POV character's thoughts are accessible. You can NEVER
  write "Elena thought" in a Marcus POV scene. You can write what Marcus OBSERVES about
  Elena — her expression, posture, tone — and what he INTERPRETS from those observations.
  His interpretation can be wrong. That's dramatic irony.
- **Sensory filtering**: The POV character notices what matters to THEM. A soldier notices
  exits, weapons, tactical positioning. A healer notices injuries, pallor, breathing.
  A thief notices valuables, guard rotations, unlocked windows.
- **Knowledge boundary**: The POV character cannot reference information they don't have.
  Check the knowledge_briefing from context assembly. If Marcus doesn't know Elena is
  a spy, he cannot think "he wondered if her loyalty was real." He has no reason to wonder.
- **Emotional coloring**: The world is described through the POV character's emotional state.
  If Marcus is furious, the crowd is "suffocating." If he's relieved, the same crowd is "alive."
  The narrator is not neutral — the narrator IS the POV character.

### Prose Craft Rules

These combat common AI writing weaknesses:

- **Vary sentence length.** Long sentences build tension. Short ones release it. A paragraph
  of all same-length sentences is monotonous. Read your prose aloud — it should have rhythm.
- **Cut filter words.** Not "he saw the ship approach" — "the ship approached." Not "she heard
  footsteps" — "footsteps echoed in the corridor." Remove the character from between the reader
  and the experience.
- **Active voice, not passive.** Not "the door was opened by Marcus" — "Marcus opened the door."
  Passive voice distances the reader from the action.
- **Concrete over abstract.** Not "the room was messy" — "crumpled maps covered the table,
  and a boot lay on its side by the door, its sole half-detached." Specific details create reality.
- **Start scenes in medias res.** Don't open with the character waking up, traveling to the
  location, or thinking about what they need to do. Open in the middle of something happening.
- **End scenes on a hook.** The last line should make the reader NEED to read the next scene.
  A question unanswered, a threat unresolved, a decision that will have consequences.
- **No "suddenly."** If something is surprising, the surprise comes from the event itself, not
  from the word "suddenly." Cut it every time.
- **No "began to" / "started to."** Characters don't begin to do things. They do them.
- **Avoid "could feel" / "could see" / "could hear."** They feel, see, hear. The modal adds nothing.

---

## Step 4: Content Rating Check

Choose the rating before you draft. The system validates the declared rating
during `save_scene_draft`, and explicit sexual prose has an origin gate.

1. Use `general`, `teen`, `mature`, or `explicit` deliberately.
2. If the scene is stronger than `teen`, check the reader contract before you commit to the draft.
3. If the scene includes explicit sexual prose, draft that prose through
   `continue_generation` with `route: "draft"` and `rating: "explicit"`.
   `save_scene_draft` rejects client-authored explicit sexual prose unless the
   call provides that generation's `generation_id`; Spindle then saves the
   server-held generation output.
   For continuity cleanup inside that explicit prose, call `revise_generation`
   with the source `generation_id` and edit instructions, then save the new
   `generation_id` it returns.
4. If the validation response says the draft exceeded its declared rating, revise the prose or raise the rating.

### Rating-aware model offload

Spindle supports per-rating model routing. The operator can configure
`spindle.toml` so that explicit drafts go to a different agent than the
default — useful when the default model declines explicit content but a
secondary "uncensored" model is wired in for that case.

Before drafting an explicit scene, read `bible://config/routing` and check
whether a rule with `rating: "explicit"` exists for the `draft` route. The
shape:

```
{
  route_name: "draft",
  agent_id: "uncensored",        // the explicit-capable agent
  rating: "explicit",
  ...
}
```

When such a rule exists:
- Server-side draft paths (e.g. `continue_generation`) honor the rating.
  Pass `rating: "explicit"` on the input and the router auto-selects the
  override agent. Keep the returned `generation_id` and pass it to
  `save_scene_draft`. For continuations, the receipt tracks the full
  accumulated text (`prior_output + output`).
- Explicit `draft` route requests also receive Spindle's built-in
  explicit-rating drafting directive, which tells the external model not to
  fade out requested adult material and to preserve consent, adult age,
  continuity, character voice, and story tone. Project-specific flavor still
  belongs in the explicit route's configured `system_prompt`.
- To clean up continuity, style, or local details inside explicit prose, use
  `revise_generation`; it routes the edit back through the explicit-capable
  draft agent and returns a fresh receipt.
- For client-side drafting (where you, the LLM, ARE the drafter): write only
  non-explicit framing yourself. Any explicit sexual prose must come from
  `continue_generation`; otherwise `save_scene_draft` will reject it.

When no `explicit` rule exists, all drafts go to the default agent for the
`draft` route. If the user expected an offload and there is no override,
surface that mismatch instead of silently drafting with the default model.

Verify wiring with `list_agents`: each agent declares a `ratings` field. An
agent listed with `ratings: ["safe", "mature", "explicit"]` is what an
operator typically binds to the explicit override.

### Prompt strategy: brief vs packed

`bible://config/routing` rules carry a `caller_should_send_brief: bool` plus
an `adapter_kind` for the resolved adapter. **Always check this flag for the
specific `(route, rating)` you're about to invoke** and pick your prompt
strategy accordingly. Sending the wrong shape either wastes tokens or
starves the model of context.

**`caller_should_send_brief: true`** — the resolved adapter spawns its own
agent with MCP access to spindle (today: `adapter_kind: "grok"`; future
CLI-with-MCP adapters will also surface as `true`). The spawned agent will
pull bible canon on demand. Send a SHORT brief:

- the user's intent for this turn (what should happen in the scene, voice
  notes, beats to hit, anything you'd add as commentary above the prose)
- a short list of pointers to canon the agent should fetch (character voice
  IDs, prior-scene IDs, world rules to consult)
- nothing else — no inlined character snapshots, no chapter briefing dump,
  no prior-scene full text. Spindle separately injects a Spindle Context
  block with project/book/chapter/scene IDs so the agent can bootstrap.

Example brief:

```
Continue the scene from chapter:01KS9... scene:01KS9.... Beat: Mara confronts
the Wraith in the alley after the failed salt ward; she's hurt, low on
options, and the Wraith finally taunts her by name. POV stays first-person
Mara. Pull her voice profile and the prior scene before drafting. Keep the
rule "wards weaken in rain" from world canon in mind.
```

**`caller_should_send_brief: false`** — the resolved adapter is a stateless
HTTP / local endpoint with no MCP access (today: `http`, `local`, raw `cli`).
Pre-pack everything the model needs to see:

- chapter briefing
- relevant character snapshots and voice profiles
- prior scene text or summaries
- applicable world rules
- the user's intent

The model can't pull anything itself, so anything missing from the prompt
won't make it into the draft.

**Why this matters**: skipping the check and always pre-packing can spend
100k+ tokens per call against an adapter that only needed 2k of intent +
its own MCP lookups. The flag exists so you don't have to enumerate which
provider is which — you just read `caller_should_send_brief` and branch.

---

## Step 5: Save and Validate

Call `save_scene_draft` with:
- Either `chapter_id` or explicit `book_number` + `chapter_number`
- The full prose text. For explicit generation receipt saves, Spindle replaces
  this with the server-held generated text.
- `content` is accepted as an alias for `full_text`
- A summary (2-3 sentences describing what happened)
- The content rating
- A short tone string such as `grim`, `tense`, `lyrical`, or `measured`
- For explicit sexual prose, the `generation_id` returned by
  `continue_generation`; Spindle persists the server-held generated text as
  the scene `full_text`.

Read the validation response:
- **pacing_warnings**: If any arc pacing is violated, the system will say why. Revise if needed.
- **tone_deviation**: If true, the declared tone or the prose conflicts with the project's style
  contract (reader contract style_notes / style world rules). Revise — do not save over it.
- **style_warnings**: A list of specific genre-voice mismatches the heuristic gate detected
  (e.g., a grief-beat tone string on a comedy project, or a contemplative no-hook ending where
  the narrator voice asks for a hook). Each is actionable. Treat a non-empty list as a failed
  gate and rewrite before proceeding. These are deliberately coarse — absence of warnings is NOT
  proof the scene is on-genre; you still owe the self-check in Step 5a.
- **agency_warning**: If present, this is a typed `AgencyWarning` struct, not a plain string. Read the subfields: `kind` (the warning category), `message`, `character_id` and `character_name` (which protagonist is passive), `evidence: Vec<AgencyEvidence>` (specific scene snippets backing the warning), and `suggestion`. Revise the prose to add active choice for the named character.
- **content_rating_valid**: If false, your content exceeded the declared rating.
- **content_rating_warnings**: Read the mismatch details before proceeding.
- **scene_id**: Save this. You can re-read the persisted scene as
  `bible://scene:{scene_id}` to inspect the canonical saved `full_text`,
  `summary`, and placement fields.

---

## Step 5a: Genre & Style Compliance Gate (MUST PASS before proceeding)

This gate runs BEFORE the craft quality gate, because a scene can be flawlessly crafted and
still be wrong for the book. Genre compliance is the first thing to verify, not the last.

Re-read the **Project Style Requirements** block from scene context (reader contract
`style_notes` / `boundaries`, `[STYLE DIRECTIVE]` world rules, and the narrator voice). Then
interrogate your own draft against the declared genre — be your own target reader:

- **Tone match**: Does the prose actually feel like the declared genre? If the contract says
  "raunchy modern comedy" and the scene reads quiet, mournful, or prestige-literary, it fails.
- **Voice delivery**: Does the narrator sound like the `narrator_voice` directive (its
  `emotional_register`, `comedy_density`, `pacing_feel`)? A "sarcastic, funny" narrator written
  as "quiet, dry, reflective" is a voice failure even if no dialogue line is technically wrong.
- **Comedy / payload present**: If the genre demands comedy, did you actually land jokes — not
  one wry aside, but the density the contract asks for? If it demands tension or heat, is it on
  the page? Count the beats; do not assume.
- **Genre-critical characters**: Is every character the bible flags as a comedic engine,
  co-lead, or genre-driver actually *present and active*, not sidelined to a one-liner?
- **Chapter-ending style**: If this is a chapter-ending scene, does the ending match the
  `chapter_ending_style` the narrator voice asks for (hook / cliffhanger / laugh) rather than a
  resolution or grief beat the genre does not want?
- **Pacing feel**: Does the scene move at the declared cadence (punchy/serial for webnovels;
  slower-burn for literary) rather than defaulting to flowing literary rhythm?

If ANY of these fail, REWRITE before continuing. Do not rationalize a literary scene as
"emotionally resonant" — for a genre project, emotional resonance that misses the genre is a
miss. Re-call `save_scene_draft` with the corrected version and confirm `style_warnings` is
empty and `tone_deviation` is false. The dual-persona review (`run_dual_persona_review`)
includes a Target Reader persona that will independently judge genre delivery; passing this
self-check first saves a round trip.

---

## Step 5b: Quality Gate (MUST PASS before proceeding)

After the initial save and validation, run a quality gate before committing any
state changes. This prevents bad prose from polluting the Bible with incorrect
state updates.

**Anti-Slop Check**: Scan your prose for AI writing tells. These are words and
patterns that signal machine-generated text. If you find them, rewrite:

| Slop Pattern | Why It's Bad | Fix |
|-------------|-------------|-----|
| "a testament to" | Generic filler | Cut entirely or be specific |
| "the weight of [emotion]" | Cliché abstraction | Show the physical sensation |
| "couldn't help but" | Removes agency | Character chooses to do it |
| "a dance of [abstract]" | Purple prose tell | Describe the actual movement |
| "sent shivers down [body part]" | Dead metaphor | Find a fresh sensation |
| "eyes that held [emotion]" | Eyes don't hold things | Describe what the eyes DO |
| "the air crackled with [tension/energy]" | Atmosphere cliché | Use a specific sensory detail |
| "in that moment" | Temporal padding | Cut. The moment is implicit. |
| "something shifted" | Vague non-event | Name what shifted and how |
| "a mix of [emotion] and [emotion]" | Telling, not showing | Show both emotions via behavior |
| "let out a breath [they] didn't know [they] were holding" | Most overused AI line in existence | Just describe the exhale |
| "[they] found [themselves]" | Passive self-discovery | Character actively does/realizes |
| "the [noun] seemed to [verb]" | Hedging weakens the image | Commit: the noun verbed. |
| "with a sense of [noun]" | Abstract padding | Show the sense through action |
| "it was as if" | Simile crutch when overused | Use sparingly; prefer direct imagery |

**See `bible://references/anti-slop` for the full 100+ pattern list.**

Also check:
- **Voice consistency**: Re-read each character's dialogue. Does it match the
  character's established summary, state, and prior on-page voice? If two
  characters sound identical, rewrite one.
- **POV violations**: Did you accidentally access another character's thoughts?
- **World rule compliance**: Did any action violate an established world_rule?
- **Filter word creep**: Search for "felt", "seemed", "realized", "somehow",
  "began to", "started to", "suddenly". Remove or rewrite each one.

If ANY quality gate check fails, revise the prose BEFORE proceeding to state updates.
Re-call `save_scene_draft` with the revised version.

If you are also editing local manuscript files outside Spindle, the canonical
sync path is:
- `pull_chapter_from_file` — read prose from a local file into the Bible,
  reconciling any divergence.
- `push_chapter_to_file` — write canonical Bible prose out to a local file
  for editing in your normal text editor.

Use these instead of ad-hoc file Reads/Edits when you want Spindle to track
the round-trip and surface drift via `unsynced_local_files` /
`drift_warnings` on `get_writer_state`.

If you do edit local files manually, keep prose edits and Bible writes
separate:
- Re-read the current file before applying replacements; do not assume an
  older anchor string still exists.
- Apply fragile prose edits sequentially, not in a mixed parallel batch.
- Only after the prose edit succeeds should you call Bible-writing tools such
  as `save_scene_draft`, `commit_scene_changes`, `update_entity`, or
  `save_summary`.

---

## Step 6: Post-Scene State Updates

After the quality gate passes, update the Bible to reflect what happened. This is
CRITICAL — skipping this step means future scenes will have stale context.

If you have several post-scene updates to apply at once, prefer
`commit_scene_changes`. It batches character state commits, canonical fact
registration, and relationship updates while returning per-item errors instead
of failing the whole pass on the first bad entry. It also accepts shorthand
authoring input:
- `character_states`: `{ character_id, state }` summary objects are accepted and
  are stored as note-style state updates when you do not have a structured patch
- `canonical_facts`: plain fact strings are accepted
  For new facts, add them here or call `register_canonical_fact` directly.
  To CORRECT an existing canonical fact, do not use `update_entity` on
  `canonical_fact`; call `register_canonical_fact` with `supersedes_fact_id`.
- `relationship_updates`: `{ character_id_1, character_id_2, summary }` is accepted;
  if you omit trust/tension deltas, Spindle records the summary with `0` deltas

For each change, call the appropriate tool:

1. **Character state changes** → `commit_character_state`
   - Emotional shifts (was hopeful → now despairing)
   - Physical changes (was healthy → now injured)
   - Goal changes (was seeking allies → now seeking revenge)
   - Only include fields that ACTUALLY CHANGED
   
2. **Relationship changes** → `update_relationship`
   - Required fields: `character_a_id`, `character_b_id`, `trust_delta` (i32),
     `tension_delta` (i32), `reason`, `scene_id`.
   - For example `-10` trust after a lie is exposed; `+15` tension after a
     confrontation. The scene_id is what makes the change traceable.
   - The `commit_scene_changes` shorthand at Step 6 above accepts
     `character_id_1` / `character_id_2` as aliases for `character_a_id` /
     `character_b_id`.

3. **Knowledge gained** → use the shipped knowledge tools deliberately
   - For normal canon updates outside import workflows, use `record_knowledge`
   - For time-aware stories, use `create_future_knowledge` when the knowledge is
     displaced, unstable, or explicitly future-derived

4. **Try-fail cycle** → update the relevant conflict record or beat annotation
   - If a conflict saw an attempt, capture outcome, cost, and revelation in the Bible

5. **Arc milestone** → update the character arc or pacing records
   - If a character arc hit a milestone (rare — maybe every 5-10 scenes)

6. **Relationship arc phase** → update relationship and pacing state as needed
   - If trust/tension crossed a phase boundary

7. **Narrative promise** → `update_promise_status`
   - If foreshadowing was reinforced or paid off

8. **Consequence delivered** → update the conflict or world-rule records
   - If a stated threat was demonstrated on-page, capture that proof in canon

9. **World state** → `update_entity` (generic) or `update_world_rule` (dedicated)
   - If the scene changed a location, faction, or other world entity, use
     `update_entity { entity_type, entity_id, changes }`.
   - For a `world_rule`, prefer the dedicated `update_world_rule { world_rule_id, changes }`
     tool. Send `description` for the canonical rule text, not `summary`.
   - Unknown fields are packed into `notes`, so field names must match the real
     entity schema.

10. **Authorial scratch / process notes** → `record_note`
    - Use this for editorial reminders, future-you handoff notes, or out-of-band
      observations the LLM should remember but that don't fit any other entity.

---

## Step 7: Beat Annotation

Call `annotate_scene_beats` to decompose the written scene into its structural beats.
This feeds the pacing system and future context assembly.

For each beat, identify:
- Beat type (goal, conflict, disaster, reaction, dilemma, decision, etc.)
- Emotional intensity (0.0 to 1.0)
- Which arcs it advanced
- Which threads it advanced
- Which motifs it used
- Which themes it explored

---

## Step 8: Chapter Summary

If this was the last scene in a chapter, call `save_summary` to generate and save
a chapter summary. This summary will be used in future context assembly to give
writing agents a compressed view of what happened earlier in the book.

`save_summary` accepts either the chapter `entity_id`/`chapter_id` or explicit
`book_number` and `chapter_number`. Prefer the chapter id when you already have
it from the chapter or chapter-scenes resources.

---

## Step 9: Auto-Extract Incidental Facts

The post-scene state updates in Step 6 capture INTENTIONAL changes — things you
know happened because you wrote them to happen. But prose often contains INCIDENTAL
details that are now canon but weren't tracked:

- A character's physical appearance described in passing ("she pushed her dark hair back")
- A new minor character introduced by name ("the barkeep, Oswin, slid a mug across")
- A location detail that enriches the setting ("the east wall had collapsed years ago")
- A world rule demonstrated but not yet in the Bible ("iron disrupts the wards")
- A relationship nuance revealed through dialogue ("you sound like your mother")
- A timeline detail ("three days since the harbor incident")

Scan the scene you just wrote and identify any facts that should be recorded
but weren't captured in the explicit state updates. For each:

- **New minor character** → propose `create_character` with minimal profile
- **Physical description update** → propose `update_entity` on the character
- **Location detail** → propose `update_entity` on the location
- **Demonstrated world rule** → propose `update_entity` or `create_world_rule`
  Use `description` when updating an existing `world_rule`; do not send `summary`
- **Canonical fact correction** → call `register_canonical_fact` with
  `supersedes_fact_id`; canonical facts are superseded, not mutated with `update_entity`
- **Timeline fact** → note for the user, may need a timeline event or entity update

Present these as proposals to the user: "I noticed these details in the scene that
aren't in the Bible yet. Should I add them?" The user confirms, modifies, or skips
each one. This prevents the Bible from slowly drifting out of sync with the prose.

---

## Common Failure Modes (and how to avoid them)

### "White Room Syndrome"
Characters talk in a void with no grounding. Fix: open every scene with a grounding beat —
one sensory detail that places the reader in the physical space.

### "Talking Heads"
Long stretches of dialogue with no action beats, physical business, or environmental
interaction. Fix: characters should DO things while talking. They fidget, cook, walk,
fight, repair equipment. The physical business reveals character and subtext.

### "Info Dump"
The narrator explains world history, magic systems, or character backstory in a long
expository passage. Fix: information comes through conflict, not exposition. A character
ARGUES about the magic system's limitations because their life depends on it. A character
DISCOVERS history through a letter, a mural, a conversation with an old survivor.

### "Passive Protagonist"
Things happen TO the protagonist. They react but never initiate. Fix: check the
agency_check in context. If the protagonist has been passive, this scene MUST have
them make an active, costly choice that drives subsequent events.

### "Emotional Whiplash"
The scene jerks between emotional extremes without transition. A character goes from
devastated to joking in one paragraph. Fix: emotional transitions need sequel beats.
Give the character time to process before the mood shifts.

### "Homogeneous Voices"
Every character sounds the same — same vocabulary, same sentence length, same speech
patterns. Fix: before writing dialogue, re-read each character's current state,
summary, and prior dialogue evidence. Write their lines separately, then interleave.
If you can't tell who's speaking without dialogue tags, the voices aren't distinct
enough.

### "Stakes Deflation"
A threat is established but never delivered. A "deadly" opponent is defeated easily.
A "dangerous" plan succeeds without cost. Fix: check `stated_consequences` and
`world_rules`. If a threat exists, it must be real. Someone must pay the cost.
The plan must go wrong in a way that costs the protagonist something.

---

## References

For deeper craft knowledge, read these reference resources:

- `bible://references/swain-scene-sequel` — Detailed breakdown of Swain's
  scene/sequel structure with extended examples from published fiction.
- `bible://references/mru-guide` — Motivation-Reaction Unit patterns with
  before/after examples.
- `bible://references/voice-differentiation` — How to create and maintain
  distinct character voices.
- `bible://references/anti-slop` — The full anti-slop pattern catalog used by
  Step 5b.
