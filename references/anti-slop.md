# Anti-slop reference (fiction)

This is the authoritative **fiction** human catalog for Spindle anti-slop work.
It names **12 shelves** with stable IDs, default limits, and rewrite directions.
Machine-readable companion: [`anti-slop-shelf-pack.v0.toml`](anti-slop-shelf-pack.v0.toml).

## This is not an AI detector

Do not treat this catalog, any shelf scan, or any `hard_count` as proof of authorship. These are **editorial craft shelves**: recurring generic moves that flatten voice, over-explain feeling, or close a scene on a slogan.

- A human draft can trip a shelf. An LLM draft can miss every shelf and still be dull.
- Do not report detector percentages, "AI likelihood," or authorship scores.
- Do not "humanize" by synonym-swapping a flagged line. Rewrite from the scene's beats.

## Scope & classes

**Fiction only.** Do not apply this catalog to commit messages, design docs, tool copy, or research notes. Genre and style profiles may disable or soften any shelf.

| Class | Default behavior |
| --- | --- |
| **Hard** | Counted. Over-limit is a hard finding (`anti_slop.hard_count == 0` in strict gates). |
| **Soft** | Advisory on save; never fails a save unless a style profile explicitly promotes it. |

`said_bookism` carries a numeric default (≤2 non-`said` tags per chapter) but is **soft-only by default** (product lock).

## Rewrite rule (beats, not paraphrase)

When a shelf hits, return to the **beat** (goal, conflict, turn, sequel reaction) and write the moment again in at most one or two passes:
- Name the choice, cost, or concrete next action the beat requires.
- Do not swap "a mix of fear and hope" for "fear and hope warred within her," or replace a named emotion with a generic organ twitch.

## StoryScope five editorial questions

Editorial revision prompts (not shelves or fail gates):
1. **Does the narrator state the theme outright?** Cut the moral lecture; leave the enacted choice.
2. **Is emotion always rendered through the body?** Named feeling is allowed; jaw-clenches alone are as generic as mood labels.
3. **Are there any subplots at all?** Ask whether a secondary want or cost is missing because the draft refused mess.
4. **Are references named, or only gestured at?** Prefer a specific title, street, prayer, or trade over "an old song."
5. **Is anything left unresolved?** Outlook slogans are not ambiguity; real residue is an unpaid cost or surviving desire.

## Non-ports (do not apply)

Tech-writing and detector habits stay out of the fiction pack:
- **BLUF** (fiction leads with scene pressure, not executive summaries)
- **Tech buzzlist** (covered by fiction `naming_watchlist` instead)
- **Em-dash gates** (punctuation quotas punish voice)
- **Flesch / grade-level scores**
- **Detector %** (no likelihood or watermark claims)
- **FAQ endings** (scene closers use `fishing_ending`)
- **Delve-as-tech-gate** (if a POV character would say "delve," leave it)

## Shelf index

Twelve shelves total (never claim "100+" patterns):

| ID | Default class | Default limit |
| --- | --- | --- |
| `contrast_not_x_but_y` | **hard** | ≤1 per chapter |
| `emotion_cocktail` | **hard** | 0 "mix of X and Y" family |
| `fishing_ending` | **hard** | 0 outlook-family closers |
| `said_bookism` | **soft** (product lock) | ≤2 non-`said` tags per chapter |
| `body_reactions` | soft | advisory cluster |
| `eye_department` | soft | advisory cluster |
| `gesture_rack` | soft | advisory cluster |
| `atmosphere_prefabs` | soft | advisory cluster |
| `naming_watchlist` | soft | advisory cluster |
| `rhythm_cadence` | soft | advisory cluster |
| `triadic_listing` | soft | advisory cluster |
| `solitary_fade` | soft | advisory cluster |

---

## Hard shelves

### `contrast_not_x_but_y` — **hard**, ≤1 / chapter

**What it is.** The profundity hinge: "It wasn't X. It was Y." / "Not X, but Y." One hinge can turn a beat; a stack is a lecture.

- **Flag**: "It wasn't anger. It was disappointment." / "This wasn't a homecoming. It was a reckoning."
- **Rewrite from the beat.** Dramatize the action that proves the state: *"He set the cup down without drinking."*

### `emotion_cocktail` — **hard**, 0

**What it is.** Labeling two feelings instead of playing either: "a mix of X and Y," "equal parts," "a swirl of conflicting emotions."

- **Flag**: "a mix of relief and dread" / "equal parts hope and terror" / "torn between pride and shame."
- **Rewrite from the beat.** Choose the dominant pressure and give the second feeling a contradictory action: *"The seal cracked. She read the first line twice, then folded the paper so the children would not see the heading."*

### `fishing_ending` — **hard**, 0 outlook-family closers

**What it is.** Refusing a concrete shift at scene/chapter end and casting toward an unspecified future.

- **Flag**: "Only time would tell." / "The real journey was just beginning." / "And somehow, she knew everything had changed." / "Nothing would ever be the same."
- **Rewrite from the beat.** End on a choice, a cost, a closed door, or a specific new pressure: *"She locked the shop and walked toward the hall."*

### `said_bookism` — **soft-only by default**, ≤2 non-`said` tags / chapter

**What it is.** Ornamental dialogue verbs doing the acting (`hissed`, `growled`, `snapped`, `breathed`, `intoned`, `queried`). Soft-only unless a profile promotes it.

- **Flag**: `"Leave," she hissed.` / `"You're late," she snapped.`
- **Rewrite from the beat.** Put the force in the line or a physical action beat, then use `said` or no tag: *`"You're late." She let the door hit the frame.`*

---

## Soft shelves

Advisory on save when clustered.

### `body_reactions` — soft

Stock somatic shorthand standing in for the beat: *jaw tightened, stomach dropped, heart hammered, blood ran cold, knuckles whitened, breath she didn't know she was holding*.
- **Rewrite from the beat.** Use this specific character's body and task in the room: *"The ring slot was empty. She had to sit on her hands to keep from slamming the lid."*

### `eye_department` — soft

Eyes that hold, pierce, darken, flicker with emotion, or store abstract nouns (*"eyes that held a quiet sadness"*, *"something flickered behind his eyes"*).
- **Rewrite from the beat.** Eyes move, avoid, water, or fail to focus: *"He looked at the clock, not at her, and slid the envelope across."*

### `gesture_rack` — soft

Stage directions on rotation in every exchange: *shrug, nod, sigh, run a hand through hair, cross arms, clench fists*.
- **Rewrite from the beat.** Give the character a task that can go wrong or a habit specific to their trade: *"He turned the cleaver blade-down and wiped it, still not looking up. 'Ask her.'"*

### `atmosphere_prefabs` — soft

Weather and rooms doing the emotion: *the air crackled with tension, silence hung heavy, the room held its breath, something in the air shifted, a dance of shadows*.
- **Rewrite from the beat.** Name one sensory fact the POV notices because it changes a choice: *"Nobody asked him to pass the bread. The radio stayed on the farm report."*

### `naming_watchlist` — soft

Agency-killing hedges and padding when clustered: *suddenly, somehow, very, deeply, couldn't help but, found themselves, began to, in that moment, a testament to, the weight of*.
- **Rewrite from the beat.** Cut the hedge and keep the verb that carries consequence: *"She had the latch open before he finished the name."*

### `rhythm_cadence` — soft

Machine-smooth cadence: three paragraphs opening with the same pronoun/name, *"She was X. She was Y. She was Z."*, or a short fragment after every long sentence like a metronome.
- **Rewrite from the beat.** Vary sentence length for pressure rather than decoration: *"He got up with her still on his left arm and took the alley because the street had lights."*

### `triadic_listing` — soft

Everything arriving in threes (*"the smell of rain, the taste of copper, the weight of regret"*). One triad can be voice; a page of them is a tic.
- **Rewrite from the beat.** Keep the one detail that changes the next action: *"The faucet still dripped. He turned it harder than it needed."*

### `solitary_fade` — soft

**What it is.** Fading to black whenever the POV is alone — hopping conversation to conversation via bare time skips (*"Hours passed."*, *"Days passed."*, *"She spent the afternoon thinking about what he'd said."*, *"Later, at the meeting, she told him everything."*) with no place, body, or mundane task.
Grounded temporal transitions (*"Hours later, she was still paring apples at the kitchen table"*, *"Two days passed in the saddle before they reached the ford"*) are **not** this shelf.
- **Rewrite from the beat.** When the beat includes alone-time, root the POV in place + body + mundane action: *"The kettle clicked. He poured one cup, left the second mug in the sink, and read the offer again while the grounds went cold."*

## Final pass

Before saving a draft:
- Every paragraph moves action, emotion, or tension in the genre's register.
- Scene-summary language has not leaked into the prose.
- The closing line changes expectation, pressure, or desire (`fishing_ending` = 0 outlook slogans).
- You can name the beat you rewrote from.
