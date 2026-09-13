# Anti-slop reference (fiction)

This is the authoritative **fiction** human catalog for Spindle anti-slop work.
It names **12 shelves** with stable IDs, default limits, and rewrite directions.
Use it during drafting, revision, dual-persona review, and (later) as the source
of truth for a fiction scanner.

Machine-readable companion: [`anti-slop-shelf-pack.v0.toml`](anti-slop-shelf-pack.v0.toml).
Design, product locks, and later packet fields: [`docs/fiction-anti-slop.md`](../docs/fiction-anti-slop.md).

## This is not an AI detector

Do not treat this catalog, any future shelf scan, or any `hard_count` as proof
of authorship. These are **editorial craft shelves**: recurring generic moves
that flatten voice, over-explain feeling, or close a scene on a slogan.

- A human draft can trip a shelf. An LLM draft can miss every shelf and still
  be dull.
- Do not report detector percentages, "AI likelihood," or authorship scores.
- Do not "humanize" by synonym-swapping the flagged line. That is a
  paraphrase-humanizer. Rewrite from the scene's beats.

## Scope

**Fiction only.** Do not apply this catalog to commit messages, design docs,
tool copy, research notes, or other non-prose surfaces. Tech-writing Voices
gates are **non-ports** (listed below).

Genre and style profiles may disable or soften any shelf. A comedy webnovel, a
hard-boiled mystery, and a literary short will not share the same defaults.

## Soft vs hard

| Class | Default behavior (Phase 0 policy; scanner is Phase 1) |
| --- | --- |
| **Hard** | Counted. Over-limit is a hard finding. `auto_strict` later requires `anti_slop.hard_count == 0`. Fail-closed when verify/revise is on. |
| **Soft** | Advisory. Surface on save; do not fail a save. A style profile may promote a soft shelf, but **defaults stay advisory**. |

`said_bookism` carries a numeric default (≤2 non-`said` tags per chapter) but is
**soft-only by default** (product lock). Do not treat it as hard unless a profile
explicitly promotes it.

## Rewrite rule (beats, not paraphrase)

When a shelf hits, go back to the **beat** (goal, conflict, turn, sequel
reaction) and write the moment again. Budget **at most one or two**
rewrite-from-beats passes per flagged stretch.

Do not:

- Swap "a mix of fear and hope" for "fear and hope warred within her."
- Sprinkle synonyms, contractions, or typographic "grit" to look human.
- Add a body-sensation after cutting a named emotion if the beat still is not
  on the page.

Do:

- Name the choice, cost, or concrete next action the beat already required.
- Keep the project's genre contract (funny stays funny; thriller still propels).
- Leave a line alone when it is voice, not generic filler.

## StoryScope five editorial questions

These are **questions for the editor**, adapted from StoryScope's discourse-level
findings about tidy, over-determined AI fiction. They are not shelves, not
scores, and not fail gates. Genre may answer them "wrong" on purpose (a romance
HEA should resolve; a puzzle mystery may stay single-track).

1. **Does the narrator state the theme outright?** If the last page explains
   the moral, cut the lecture and leave the enacted choice.
2. **Is emotion always rendered through the body?** Named feeling is allowed.
   A draft that only ever clenches jaws and drops stomachs is as generic as a
   draft that only labels moods.
3. **Are there any subplots at all?** One tidy causal chain is a default, not a
   virtue. Ask whether a secondary want, cost, or relationship is missing
   because the beat never needed it — or because the draft refused mess.
4. **Are references named, or only gestured at?** "An old novel she had loved"
   is a shrug. A specific title, street, brand, or prayer is a world.
5. **Is anything left unresolved?** Outlook slogans ("only time would tell")
   are not ambiguity. Real residue is a unpaid cost, a lie still standing, or a
   desire that survived the climax.

If a question fails, it is a revision prompt, not an authorship verdict.

## Non-ports (do not apply)

These tech / Voices / detector habits **do not belong** on fiction shelves.
Do not add them to the pack, the scanner, or the quality gate.

| Non-port | Why it stays out |
| --- | --- |
| **BLUF** (bottom line up front) | Fiction leads with scene pressure, not an executive summary. `solitary_fade` is lived-in grounding, not a BLUF/structure gate. |
| **Tech buzzlist** (leverage, synergy, unlock, ship, iterate) | Wrong register. Fiction naming is the `naming_watchlist` shelf, not a product glossary. |
| **Em-dash gates** | Punctuation quotas punish voice. Cadence belongs under `rhythm_cadence` as a soft editorial note, never as a dash ban. |
| **Flesch / grade-level scores** | Readability formulas are not fiction quality. |
| **Detector %** | Not an AI detector. No likelihood scores, watermark claims, or classifier output. |
| **FAQ endings** | Blog "Q&A wrap-up" closers are not a fiction default. Scene endings use `fishing_ending`. |
| **Delve-as-tech-gate** | Banning "delve" because it appears in LLM essays is a tech tell, not a fiction craft rule. |

## Shelf index

Twelve shelves. That is the real count. Skills must not claim "100+" patterns.

| ID | Default class | Default limit |
| --- | --- | --- |
| `contrast_not_x_but_y` | **hard** | ≤1 per chapter |
| `emotion_cocktail` | **hard** | 0 "mix of X and Y" (and close family) |
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

**What it is.** The profundity hinge: "It wasn't X. It was Y." / "Not X, but Y."
One contrast can turn a beat. A stack of them is a lecture.

**Examples (flag)**

- "It wasn't anger. It was disappointment."
- "This wasn't a homecoming. It was a reckoning."
- "Not fear — something older, something that had no name."
- "She didn't run because she was brave. She ran because she was done."

**Rewrite from the beat.** Pick the true state and dramatize the action that
proves it. If the beat is "he will not drink with her," write the cup going
down, not the thesis about what the feeling is not.

- Weak hinge: "It wasn't anger. It was disappointment."
- From the beat (he expected an apology and got a joke): "He set the cup down
  without drinking."

If the chapter already used one hinge and it earned the turn, leave it. A
second hinge in the same chapter is the finding.

### `emotion_cocktail` — **hard**, 0

**What it is.** Labeling two feelings instead of playing either: "a mix of X
and Y," "equal parts," "a swirl of conflicting emotions."

**Examples (flag)**

- "a mix of relief and dread"
- "equal parts hope and terror"
- "She felt a confusing swirl of love and resentment."
- "He was torn between pride and shame."

**Rewrite from the beat.** Choose the dominant pressure. If the second feeling
matters, give it a contradictory *action*, not a second noun.

- Weak: "She felt a mix of relief and dread when the letter came."
- From the beat (letter decides whether the mill stays open): "The seal
  cracked. She read the first line twice, then folded the paper so the children
  would not see the heading."

Do not "fix" a cocktail by writing "fear and hope warred within her." That is
still the shelf.

### `fishing_ending` — **hard**, 0 outlook-family closers

**What it is.** A scene or chapter that refuses a concrete shift and casts a
line toward an unspecified future: tomorrow, only time, the real journey, and
somehow she knew.

**Examples (flag)**

- "She didn't know what tomorrow would bring."
- "Only time would tell."
- "The real journey was just beginning."
- "And somehow, she knew everything had changed."
- "What happened next would change everything."
- "Nothing would ever be the same."

**Rewrite from the beat.** End on a choice, a cost, a new desire, a closed
door, or a fact that was not true on page one of the scene. Outlook is not
ambiguity.

- Weak: "She didn't know what tomorrow would bring, but she would face it."
- From the beat (shop is lost; union vote is tonight): "She locked the shop and
  walked toward the hall."

A hook that names a *specific* next pressure ("the warrant was already on the
counter") is not this shelf. A slogan about the future is.

### `said_bookism` — **soft-only by default**, ≤2 non-`said` tags / chapter

**What it is.** Fancy dialogue tags doing the acting: ejaculated, expostulated,
intoned, queried, interjected, hissed, growled, snapped, breathed, as the
default verb.

**LOCKED decision.** Soft-only unless a profile promotes it. Pulp, romance, and
comedy often want more color than `said`. Do not fail a save on this shelf.

**Examples (flag as soft)**

- `"Leave," she hissed.` / `"Please," he breathed.` / `"Impossible," she gasped.`
- A page where nobody merely says anything.

`asked` for a question is fine. An action beat instead of a tag is fine.
Two non-`said` tags in a chapter can stay; a stack is the advisory.

**Rewrite from the beat.** Put the force in the line or in a physical beat,
then use `said` (or nothing).

- Weak: `"You're late," she snapped.`
- From the beat (she has been holding the door on a swollen hinge): `"You're
  late." She let the door hit the frame.`

---

## Soft shelves

Advisory on save. Clusters, not single-word guilt. One shrug is not a finding;
a gesture in every exchange is.

### `body_reactions` — soft

Stock somatic shorthand that stands in for the beat.

**Examples (flag when clustered)**

- jaw tightened / stomach dropped / heart hammered / blood ran cold
- knuckles whitened / breath she didn't know she was holding
- sent shivers down her spine / a lump in his throat
- let out a breath they didn't know they were holding

**Rewrite from the beat.** If the body matters, use *this* body in *this*
room: a specific tell the character already owns, or a task they fail because
the feeling is in the way. Do not replace a named emotion with a generic organ.

- Weak: "Her heart hammered as she opened the lockbox."
- From the beat (the lockbox should hold her brother's ring; it holds a pawn
  ticket): "The ring slot was empty. She had to sit on her hands to keep from
  slamming the lid."

### `eye_department` — soft

Eyes that hold, pierce, darken, flicker with emotion, or reveal a soul.

**Examples (flag when clustered)**

- "eyes that held a quiet sadness"
- "his gaze pierced her"
- "her eyes darkened with fury"
- "she looked into his eyes and saw the truth"
- "something flickered behind his eyes"

**Rewrite from the beat.** Eyes can move, avoid, water, or fail to focus. They
do not store nouns. Write what the look *does* to the other person or to the
next action.

- Weak: "His eyes held a mixture of pity and resolve."
- From the beat (he is about to fire her and wants it over): "He looked at the
  clock, not at her, and slid the envelope across."

### `gesture_rack` — soft

The same stage directions in every exchange: shrug, nod, sigh, run a hand
through hair, cross arms, clench fists.

**Examples (flag when clustered)**

- nodding through an entire argument
- sighing as punctuation
- hair-hand as the only think-beat
- shrug / crossed arms / clenched fists on rotation

**Rewrite from the beat.** Give the character a task that can go wrong (pouring,
reloading, buttoning a child, counting bills). If they must gesture, use a
gesture that belongs to *their* job, injury, or habit — once.

- Weak: "He shrugged and ran a hand through his hair. 'I don't know.'"
- From the beat (he is a butcher who will not name a price): "He turned the
  cleaver blade-down and wiped it, still not looking up. 'Ask her.'"

### `atmosphere_prefabs` — soft

Weather and rooms doing the emotion: crackling air, heavy silence, a room that
holds its breath, something shifting in the atmosphere.

**Examples (flag when clustered)**

- "the air crackled with tension"
- "silence hung heavy"
- "the room seemed to hold its breath"
- "something in the air shifted"
- "a testament to" used as mood glue
- "a dance of [light / shadows / death]"

**Rewrite from the beat.** One sensory fact that the POV would actually notice
because it changes a choice (the fryer is still on; the hymn has stopped; the
neighbor's TV is loud enough to cover a voice).

- Weak: "The air crackled with unspoken tension."
- From the beat (family dinner after the arrest): "Nobody asked him to pass the
  bread. The radio stayed on the farm report."

### `naming_watchlist` — soft

Padding and agency-killers. A single `very` is not a case. A paragraph of
hedges is.

**Watch terms (illustrative, not a detector list)**

- suddenly, somehow, very, deeply, just, almost, perhaps
- couldn't help but / found themselves / began to / started to
- in that moment / something shifted / it was as if / with a sense of
- the weight of [emotion] / a testament to / seemed to

**Rewrite from the beat.** Cut the adverb and keep the verb that already has
consequence. If the character "couldn't help but look," decide whether they
*choose* to look, and what it costs.

- Weak: "She suddenly found herself reaching for the door, as if something had
  shifted in that moment."
- From the beat (she is leaving before he can apologize): "She had the latch
  open before he finished the name."

`delve` is **not** on this list as a tech gate. If a viewpoint character would
say "delve," leave it.

### `rhythm_cadence` — soft

Machine-smooth cadence: repeated openings, anaphora stacks, every paragraph
ending on the same short punch, identical sentence length for a page.

**Examples (flag when clustered)**

- three paragraphs in a row opening with the character's name or "He"
- "She was X. She was Y. She was Z."
- a short fragment after every long sentence, as if on a metronome

**Rewrite from the beat.** Vary for *pressure*, not for decoration. A chase can
be short. A lie can sprawl. Do not "fix" cadence by inserting em-dashes (em-dash
gates are a non-port) or by making every line a fragment.

- Weak: "He ran. He fell. He rose. He ran again."
- From the beat (he is carrying a child and cannot use his right hand): "He
  got up with her still on his left arm and took the alley because the street
  had lights."

### `triadic_listing` — soft

Everything arrives in threes: three smells, three memories, three abstract
nouns. One triad can be voice. A page of them is a tic.

**Examples (flag when clustered)**

- "the smell of rain, the taste of copper, the weight of regret"
- "She wanted rest, safety, and a door that locked."
- "blood, ash, and memory" as the default cadence

**Rewrite from the beat.** Keep the one detail that changes the next action.
If you need a list, make the items unequal (two concrete, one refused).

- Weak: "He remembered the rain, the copper taste, and the weight of regret."
- From the beat (he is back in the kitchen where the fight started): "The
  faucet still dripped. He turned it harder than it needed."

### `solitary_fade` — soft

**What it is.** The draft fades to black whenever the POV is not talking to
someone. Scenes hop interpersonal beat → interpersonal beat and skip lived-in
solitude: no bus, bench, kettle, queue, or familiar walk. The character exists
only in conflict with others.

Alternate name (not a second ID): ungrounded solitude. Use `solitary_fade`.

This is fiction craft grounding, not a tech BLUF or structure gate. Do not
"fix" it by adding a topic sentence or an outline beat. Do not pad with empty
time skips ("hours passed," "she thought about what he'd said").

**Examples (flag when the alone-time is skipped or emptied)**

- Cut from the argument to the next conversation: "Later, at the meeting, she
  told him everything."
- "She spent the afternoon thinking about what he'd said." (thought as a time
  skip, no place or task)
- "Hours passed." / "The next morning…" with no rooted beat in between
- A sequel that is only interior voice in a white room — no body, no chore,
  no route
- Waiting, transit, or errands mentioned in the plan but missing from the
  prose

A scene that is *supposed* to be one unbroken dialogue (a deposition, a
phone call, a locked-room argument) is not this shelf. A montage the genre
asked for is not this shelf. The finding is the habit of never showing the
character alone in their life.

**Rewrite from the beat.** When the POV is alone, root the reader in **place +
body + mundane action** before or around the turn. Prefer concrete errands,
transit, waiting, chores, idle observation. Show them *in their life*.

- Weak: "She left the kitchen and later told Marcus she would take the shift."
- From the beat (she has already decided; the bus is how she lives with it):
  "The 14 was packed. She stood with the transfer in her teeth and watched
  the river lots go by, then texted Marcus from the shelter: I'll take it."
- Weak: "He thought about the offer until morning."
- From the beat (the offer is on the counter; he still has to make coffee):
  "The kettle clicked. He poured one cup, left the second mug in the sink,
  and read the offer again while the grounds went cold."

If the beat has no alone-time, do not invent a commute. If it does, do not
skip it.

## Fast scan (still fiction, still not a detector)

Use this as a pass *after* the shelves, not instead of them.

- Cut abstract intensifiers (`suddenly`, `somehow`, `very`, `deeply`) unless
  they are voice.
- Cut summary that restates what the reader just watched.
- Cut ornamental synonyms that flatten a character into the narrator.
- Cut placeholder emotion labels when behavior is cheaper and truer.

## Final pass

Before saving a draft:

- Each paragraph moves action, emotion, or tension — in the genre's register.
- Named characters still sound like themselves.
- Scene-summary language has not leaked into the prose.
- The last line changes expectation, pressure, or desire (`fishing_ending` is
  0 outlook slogans).
- You can name the beat you rewrote from. If you cannot, you paraphrased.
