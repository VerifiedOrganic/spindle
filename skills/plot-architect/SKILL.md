---
name: plot-architect
description: >
  Use when planning, structuring, or modifying the story's plot architecture. This includes
  creating plot lines, designing conflicts, establishing themes and motifs, setting up pacing
  configuration, outlining chapters, planning story arcs, defining try-fail cycles, creating
  narrative promises (Chekhov's guns, foreshadowing), and building the overall story structure.
  Also triggers for "outline my book", "plan the story", "what should happen next", "the plot
  feels unfocused", "how do I structure this", "pacing feels off", "I need more conflict",
  or any request about narrative design, story beats, or structural planning. If the story
  feels aimless, saggy in the middle, or structurally weak, this skill fixes that.
---

# Plot Architect

Plot is the engine of story. Without structure, even the richest characters and most detailed
world are just decoration. This skill guides you through building a plot architecture that
creates tension, maintains momentum, and delivers payoff.

## Tool workflow

Use this sequence when planning or restructuring existing material:

1. Call `set_active_project` once per session so subsequent tool calls inherit
   the project (and active branch) without re-passing `project_id`.
2. Call `get_writer_state` to see the current cursor, open promises (the
   `narrative_promises_due` field surfaces what's overdue), recent activity,
   and active overlays. Read these before restructuring so you don't leave
   live promises stranded.
3. Call `list_book_chapters` and `list_chapter_scenes` to map the active-branch
   spine before changing outlines.
4. Use the batch creators for bulk seeding: `batch_create_motifs`,
   `batch_create_narrative_promises`, `batch_create_terms`. One call beats
   twenty.
5. Outlines vs scene-level plans:
   - `set_book_outline` and `set_chapter_outline` persist prose-form outlines
     attached to the book or chapter.
   - `plan_chapter` produces a structured per-scene plan (POV, synopsis,
     target themes/conflicts, target plot lines, scene list).
   Use outlines for narrative shape; use `plan_chapter` for the concrete
   shooting script the scene-writer reads.
6. Call `get_chapter_briefing` before handing a chapter to scene drafting.
7. Call `annotate_scene_beats` after scenes are drafted to tag them with
   beat type, motifs, themes, and conflicts — this feeds the pacing system
   and future context assembly.
8. Call `move_scene` when the issue is ordering rather than scene content.
9. Call `check_consistency` after major re-structure to verify promise,
   pacing, and continuity impact.

## The Three P's: Promise, Progress, Payoff (Sanderson)

Every story — and every arc within a story — follows this rhythm:

**Promise** (Act 1): Tell the reader what kind of story this is. Establish tone, stakes,
characters, and the central question. The reader invests based on these promises.

**Progress** (Act 2): Show the reader the story advancing toward answering its promises.
Characters try, fail, learn, try again. Subplots weave. Stakes escalate. The reader feels
momentum even when characters are losing ground.

**Payoff** (Act 3): Deliver on the promises. The central question is answered. Arcs resolve.
The payoff must be "surprising yet inevitable" — the reader didn't predict the exact outcome,
but looking back, every element was set up.

Call `update_entity` on the project to set the `reader_contract` with tone_promise,
plot_promise, and character_promise. This contract is loaded by the scene-writer skill
for every scene written.

---

## Building the Plot Architecture

### Step 1: Define the Central Conflict

Every story needs ONE central conflict that everything else orbits. Call `create_conflict`:

- **conflict_type**: person_vs_person, person_vs_self, person_vs_society,
  person_vs_nature, person_vs_fate, person_vs_technology, person_vs_system
- **stakes**: What happens if the protagonist FAILS? This must be concrete and devastating.
  Not "bad things happen" — "the kingdom falls to the tyrant and Elena's family is executed."
- **escalation_stages**: How the conflict intensifies over the story. Each stage should be
  worse than the last with fewer options for the protagonist.

Then define the **try-fail cycles** (Sanderson/Swain pattern):
```
expected_total_cycles: 3-5 for a novel's central conflict

Cycle 1 (early book): Protagonist tries the obvious solution.
  → Fails because they don't yet understand the problem.
  → Cost: loses something (ally, resource, time, reputation).
  → Revelation: discovers the real scope of the problem.

Cycle 2 (mid book): Protagonist tries a more informed approach.
  → Yes-but: partially succeeds, but at a devastating cost.
  → Cost: the cost escalates (personal sacrifice, moral compromise).
  → Revelation: realizes they need to change internally to succeed.

Cycle 3 (late book): Protagonist faces the core of the conflict.
  → Must apply the Truth from their character arc.
  → Resolution: surprising but inevitable.
```

Define **stated_consequences** — threats that MUST be delivered:
```
"The council executes anyone who helps the resistance"
  → stated_at_chapter: 3
  → must_demonstrate_by: "end of book 1"
  → If never shown: empty threat that kills tension
```

### Step 2: Design Plot Lines

For each major and minor plot thread, call `create_plot_line`:

- **Main plot** (plot_type: "main"): The central conflict. Gets the most page time.
- **Parallel subplots**: Run alongside, intersecting at key points.
- **Mirror subplots**: Echo the main theme in a different context (a secondary character
  faces a version of the same dilemma).
- **Conflict subplots**: Directly oppose or complicate the main plot.
- **Romantic subplots**: Relationship arcs that intersect with the main conflict.

**Convergence design**: Plot lines should CONVERGE at major turning points.
When two plot lines collide — "the romantic subplot crashes into the political
conspiracy at the masquerade ball" — that's where the best scenes live.

Set `convergence_points` on each plot line to plan these intersections.

### Step 3: Establish Themes and Motifs

Themes are the story's argument about the human condition. Call `create_theme`:

- **theme_statement**: "Trust requires vulnerability" or "Power corrupts the isolated"
- **thesis_antithesis**: The theme is explored through BOTH sides of the argument.
  Characters who trust are sometimes rewarded, sometimes punished. The story doesn't
  preach — it examines.
- **introduction_point**: Where the theme first appears
- **resolution_point**: Where the story takes its stance

Call `create_motif` for recurring symbols, images, or patterns that reinforce themes:
- A motif of "hands" in a story about trust (reaching out, pulling away, holding on)
- A motif of "fire" in a story about destruction and renewal
- Set `max_uses_per_chapter` to prevent overuse

### Step 4: Create Narrative Promises (Chekhov's Arsenal)

For every setup that needs a payoff, call `create_narrative_promise`:

- **chekhov_gun**: A specific detail introduced that must fire later.
  "Marcus's silver compass — mentioned in chapter 2, becomes critical in chapter 30."
- **foreshadowing**: Hints at future events that the reader may not consciously notice.
  "The weather pattern in chapter 5 echoes the battle conditions in chapter 25."
- **setup**: Establishing a capability, relationship, or fact that enables a future scene.
  "Elena's knowledge of poisons — established in chapter 8, used in chapter 22."
- **question_raised**: A mystery posed to the reader that demands an answer.
  "Who sent the anonymous message in chapter 3? Answer: chapter 18."

Each promise has a `status` lifecycle: planted → reinforced → paid_off (or subverted/abandoned).
Use `update_promise_status` to advance through the lifecycle.
The continuity-editor skill monitors for overdue promises.

### Step 5: Design the Pacing Architecture

Call `create_pacing_config` for the series or standalone book:

- **total_planned_books, avg_chapters_per_book, avg_scenes_per_chapter**
- **tension_model**: "escalating" (rises throughout), "wave" (peaks and valleys),
  "slow_burn" (low early, accelerating), "explosive" (high throughout)

Call `create_pacing_curve` for each book:
- Define act structure with percentage breakpoints:
  ```
  Act 1 (0-25%): Setup, inciting incident
  Act 2A (25-50%): Rising action, midpoint reversal
  Act 2B (50-75%): Complications, darkest moment
  Act 3 (75-100%): Climax, resolution
  ```
- Define scene type density per act:
  ```
  Act 1: 40% dialogue, 30% action, 20% worldbuilding, 10% introspection
  Act 3: 20% dialogue, 50% action, 10% worldbuilding, 20% introspection
  ```

Call `set_arc_pacing_constraints` for each character arc:
- **per_book_budget**: How much of the arc happens in each book
- **max_progress_per_chapter**: Prevents rushing (typically 0.05-0.10)
- **milestone_spacing**: Minimum chapters between milestones
- **sprint_allowance**: How many rapid-advancement chapters before cooldown
- **regression_budget**: How much backsliding is allowed

### Step 6: Outline Chapters

Call `plan_chapter` for each chapter in sequence:

For each chapter, define:
- **POV character**: Who tells this chapter?
- **Synopsis**: 2-3 sentence summary of what happens
- **Target themes and conflicts**: Which themes and conflicts are active
- **Target plot lines**: Which plot threads advance
- **Scenes**: Each with a summary, beat structure, characters, and purpose

**Chapter design principles:**
- Every chapter should change something. If the reader's understanding of the story
  is identical at the end as the beginning, the chapter failed.
- End every chapter on a hook — unresolved tension, unanswered question, or
  impending consequence that pulls the reader into the next chapter.
- Alternate POVs to create dramatic irony (reader knows things characters don't)
  and maintain pace across parallel storylines.

---

## Story Structure Frameworks

Use these as starting points, not prisons.

### Three-Act Structure
| Beat | Timing | Purpose |
|------|--------|---------|
| Opening Image | Ch 1 | Establish tone and normal world |
| Inciting Incident | Ch 2-3 | The event that disrupts everything |
| Debate / Refusal | Ch 3-5 | Character resists the call |
| First Plot Point | ~25% | Point of no return |
| Rising Action | 25-50% | Try-fail cycles, allies, enemies |
| Midpoint | ~50% | False victory or major revelation |
| Complications | 50-75% | Everything gets worse |
| Dark Night of the Soul | ~75% | All seems lost |
| Climax | 85-95% | Final confrontation |
| Resolution | 95-100% | New normal |

### The Try-Fail Cycle Pattern
For the central conflict across a full novel:
```
Ch 3-8:   First attempt → Fail → Revelation about true scope
Ch 10-16: Second attempt → Yes-but → Pyrrhic victory, higher cost
Ch 18-24: Third attempt → Fail → Protagonist must change
Ch 26-30: Final attempt → Applies character arc truth → Resolution
```

---

## Series-Level Planning

For multi-book series, additional considerations:

### Cross-Book Arc Management
- Character arcs can span multiple books. `CreateCharacterArcInput` does not
  itself carry per-book budget fields — use `set_arc_pacing_constraints`
  with the `per_book_budget` field to allocate arc progress across books.
- To start a new book in an existing project, call `create_book` and seed the
  first chapter with `create_chapter`. There is no separate "bootstrap from
  previous book" tool — instead, read `get_writer_state` (which surfaces the
  latest cursor, open promises, and recent activity) and then plan the next
  book's outline against that state.
- There is no shipped `continuity_thread` record. Use `create_narrative_promise`
  for setups that span books, and `create_plot_line` with cross-book
  convergence_points for plot threads that braid across books.

### Series-Level Pacing
- Not every arc should peak in the same book.
- The series needs its own rising tension curve on top of per-book curves.
- Relationship arcs often span the full series while plot arcs are per-book.

---

## Skill Chains

- **← character-creator**: Characters should exist before building plot around them.
  The character's Lie and Ghost should drive the conflict design.
- **← worldbuilder**: World rules create natural conflict constraints and costs.
- **→ scene-writer**: The scene-writer reads all plot architecture from context
  assembly and writes scenes that advance the planned structure.
- **→ continuity-editor**: The continuity-editor checks that plot structure is
  being followed — promises are being paid off, arcs are progressing, pacing budgets
  aren't exceeded.
- **→ revision-manager**: If the plot needs restructuring after writing has begun,
  the revision-manager handles branching and cascading state changes.

---

## References

The shipped craft references most relevant to plot architecture are exposed
as `bible://references/<name>` resources:

- `bible://references/swain-scene-sequel` — Scene/sequel structure that maps
  cleanly to try-fail cycle design.
- `bible://references/mru-guide` — Motivation-Reaction Unit construction at
  the beat level.
- `bible://references/anti-slop` — Catch generic structural patterns
  (weightless conflicts, telegraphed payoffs) before they ossify in the plan.
- `bible://references/voice-differentiation` — Useful when planning
  ensemble-cast scenes with multiple POVs.
