---
name: continuity-editor
description: >
  Use when checking, verifying, or enforcing consistency across the story. This includes running
  consistency checks, tracking narrative promise lifecycle, verifying world rule compliance,
  monitoring pacing budgets, checking character consistency, tracking consequence delivery,
  auditing agency patterns, and reviewing tone consistency. Also triggers for "check for plot holes",
  "is this consistent", "review my story for problems", "what promises haven't paid off",
  "are there any continuity errors", "something feels off", or any request about quality
  assurance, consistency, or structural integrity. If anything feels wrong but the user
  can't pinpoint why, this skill diagnoses it.
---

# Continuity Editor

Writing a novel is managing a thousand moving parts. Characters change, worlds evolve,
promises accumulate, and every detail the reader remembers must remain consistent.
This skill is the quality assurance layer that catches problems before readers do.

## Triage workflow

Use this triage loop when diagnosing continuity concerns:

1. Call `set_active_project` once per session so subsequent tool calls inherit
   the project (and active branch).
2. Call `get_writer_state` to confirm active branch context, drift warnings,
   and recent session activity before auditing.
3. Read `bible://projects/{project_id}/continuity/health` when the request is
   project-wide, branch-sensitive, timeline-sensitive, or follows a long
   editing session. Use it to spot open validator findings, resolved/stale
   validator cache rows, branch lineage, orphaned temporal interventions, and
   duplicate active canonical-fact keys before deciding what to inspect next.
4. For time-travel, branching, save-point, or timeline-order concerns, read
   `bible://projects/{project_id}/timeline-graph/mermaid` and use the graph to
   orient branch parentage, save points, event order, and temporal
   interventions before making a prose or canon judgment.
5. Call `find_entity` for ambiguous names, then `get_entity` for canonical
   subject snapshots used in the diagnosis.
6. Call `find_scenes_referencing` to enumerate every scene that mentions a
   given subject or phrase. This is the primary backreference lookup before
   you flag cross-scene drift.
7. Call `check_consistency` with explicit scope and inputs. Useful patterns:
   - `subjects: ["character:abc123"]` to scope an entire validator pass to
     scenes that reference one or more specific entities.
   - `format: "markdown"`, optional `budget_tokens` — get a rendered report
     ready to present to the user. Output also includes `report_sections`
     grouped by validator with per-scene findings, plus a `markdown` field.
   - `commit_scene_changes` returns a `findings_summary` so post-revise
     re-validation can happen inline without a separate consistency call.
8. Hand off fixes to the owning skill (scene-writer, plot-architect, or
   worldbuilder), then re-run `check_consistency` to confirm closure.

## The Consistency Checks

Call `check_consistency` with `scope: ConsistencyScopeInput` (a struct with
`scope_type: "full" | "book" | "chapter_range"` and the matching numeric
range fields) and optionally specify which `checks` to run by name. The five
Phase 4 validators (`canonical_fact_prose_drift`, `world_rule_semantic_drift`,
`voice_drift`, `retcon_reachability`, `style_compliance`) run by default and
are cached per `scene_text_hash` and validator-context hash. Relevant canon,
style, voice, and timeline writes invalidate their validator cache rows; the
context hash also prevents stale hits when metadata changes outside the normal
service path. The story-time checks below (`chronology`, `knowledge_timing`,
`pacing_drift`) are deterministic — not cached Phase 4 validators — and are
requested by name or run as part of a full audit.

Here's what each check does and why it matters:

### 1. Character Consistency
Verifies that character behavior matches their established profiles.
- Does dialogue still match the character's established on-page voice and prior summaries?
- Do emotional responses match the emotional_profile? (suppressed emotions, triggers)
- Has the character undergone arc changes that justify behavioral shifts?

**What to do when flagged**: Read the character profiles and the flagged scene.
If the behavior is intentional arc progression, call `commit_character_state` to
update the profile. If it's accidental inconsistency, flag for revision.

### 2. Timeline Continuity
Verifies that events happen in the correct order and durations make sense.
- Do travel times match established distances? (world_rule: time_constraint)
- Are character ages consistent across the timeline?
- Do seasons, day/night cycles, and in-world dates align?
- For time-travel stories: do temporal_intervention records create paradoxes?

### 3. World Rule Compliance
Checks every scene against established world_rules.
- Has any character used a power that violates a magic_limitation?
- Has any solution appeared that wasn't previously established?
- Are power costs being paid consistently?
- Do technology_constraints hold across all scenes?

**This is the deus ex machina detector.** If a rule violation is flagged, it's
either a genuine error that needs revision or a sign that the canon update path
was skipped. Use the shipped world-rule or scene revision workflows instead of
assuming an unpublished exception field exists.

### 4. Narrative Promise Tracking
Reviews all `narrative_promise` records:
- **Planted but unfired**: Chekhov's guns that haven't gone off yet.
  Some may be fine (planted in chapter 5, planned payoff in chapter 30).
  Others may be overdue (planted 15 chapters ago with no reinforcement).
- **Reinforced but not paid off**: Foreshadowing that keeps being hinted at.
  Getting overdue — the reader is waiting.
- **Forgotten**: Promises with no reinforcement and no planned payoff.
  These erode reader trust. Either pay them off, reinforce them, or
  consciously abandon them.

Call `update_promise_status` to manage the lifecycle.

### 5. Pacing Budget Audit
Reviews all `pacing_tracker` records:
- Is any arc significantly behind schedule? (needs acceleration)
- Is any arc ahead of schedule? (needs cooling down)
- Have sprint allowances been exceeded? (forced cooldown required)
- Is regression budget remaining? (character can still backslide)
- Is the net progress still positive per book? (mandatory rule)

Use `set_arc_pacing_constraints` or update the relevant pacing records if the
budget model needs recalibration after structural changes.

### 6. Agency Tracking
Reviews protagonist agency across recent scenes:
- How many consecutive scenes has the protagonist been reactive?
- In what percentage of scenes does the protagonist make an active choice?
- Are choices driving plot, or is the protagonist being dragged by events?

The scene-writer skill embeds agency tracking in every scene, but this check
reviews patterns across the full arc.

### 7. Tone Consistency
Reviews `scene.tone` records against `project.reader_contract.boundaries`:
- Has the darkness_level exceeded the reader contract's max?
- Has humor appeared in scenes where the contract forbids it?
- Are there sudden tonal shifts without transition (emotional whiplash)?
- Does the overall tone arc match the genre expectations?

### 8. Content Boundary Compliance
Reviews all scenes with content_rating "mature" or "explicit":
- Do the reader-contract boundaries match what the prose is doing?
- Have any character hard_limits been violated?
- Are relationship phase content ceilings respected?

### 9. Knowledge Contradiction Detection
The shipped knowledge surface lives in two places: typed `canonical_fact`
records (created with `register_canonical_fact`, with predicate +
value_kind + value_text/value_number/value_unit/value_json) and
`future_knowledge` records for time-displaced or unstable knowledge.
Reviews scan scene content for contradictions:
- Does a character act on information they shouldn't have?
- Is dramatic irony being maintained correctly?
- For time-travel: has future_knowledge been used after it was invalidated?

When you find prose that should be promoted to canon, use
`extract_canonical_facts_from_scene` to propose typed facts, then
`register_canonical_fact` to persist them. To correct an existing canonical
fact, do not mutate it — call `register_canonical_fact` with
`supersedes_fact_id` set to the outdated fact's ID. Use
`migrate_canonical_fact` to convert legacy untyped facts into the typed
shape.

### 10. Try-Fail Cycle Tracking
Reviews `conflict.try_fail_cycles`:
- Do conflicts have enough attempts before resolution? (minimum 2-3)
- Is each attempt more costly than the last? (escalation check)
- Does the final resolution use a different approach than earlier attempts?
- Does the resolution connect to the character's arc? (internal change enables victory)

### 11. Consequence Delivery Audit
Reviews `conflict.stated_consequences` and world-rule evidence in scoped scenes:
- Are stated threats being backed up with on-page demonstrations?
- Are world rules being shown, not just told?
- Are consequences proportional to the established severity?

### 12. In-world Chronology (`chronology`)
Active only when the project declares a calendar (`set_project_calendar`) and
scenes carry clocks (`set_scene_clock`). Flags any scene set earlier in story
time than its predecessor on the same timeline `thread_key` that is **not**
marked `flashback`/`flashforward`/`concurrent`. This is the multi-book timing
guard — it catches a scene that silently rewinds the clock.

**What to do when flagged**: if the rewind is intentional, set the scene's
`temporal_mode` accordingly (or give it its own `thread_key`); if it's an
error, correct the scene's `day_index`. A no-op for projects that never declare
story-time.

### 13. Knowledge Timing (`knowledge_timing`)
Active when `knowledge_fact` records carry a `learned_at` position. Flags a
scene whose prose names a present character alongside a fact they do not learn
until a later book/chapter — a character acting on information they shouldn't
have yet. High-precision/low-recall, so it is an advisory warning.

**What to do when flagged**: move the reveal earlier, record the character
learning the fact sooner, or revise the prose to remove the leak. This is the
ordinary-knowledge complement to the time-travel `future_knowledge` checks in
section 9.

### 14. Pacing Drift (`pacing_drift`)
Active for books with a planned pacing curve. Reads per-beat `intensity`
annotations and flags when realized per-chapter intensity falls across three or
more consecutive chapters — a sustained sag (a sagging middle). A single dip is
intentional; a sustained slide is drift.

**What to do when flagged**: raise the stakes or add a turn in the affected
chapters — or, if the de-escalation is deliberate (a lull before a finale),
accept it. Quality depends on honest beat-intensity annotation from
scene-writer; flat annotations blind this check.

### 15. Quantity Drift (`quantity_drift`)
Active when the project declares a quantity scheme (`set_project_quantity_scheme`,
or `derive_quantity_scheme_from_system_overlay`). Flags when a subject's tracked
band jumps more than the scheme's `max_band_jump` ordered tiers between
consecutive stamps without a `change_reason` — the wealth/progression analogue of
chronology drift.

**What to do when flagged**: if the jump is legitimate (an inheritance, a
cultivation breakthrough), re-commit the reading via `commit_quantity_state` with
a `change_reason`; otherwise stage it across intermediate bands or fix the stamp.

### 16. Currency Consistency (`currency_consistency`)
Active when a scheme declares denominations. Converts numeric price facts to base
units and flags two prices for the same good (same `subject`/`predicate`) that
disagree once converted — e.g. "5 silver" and "100 copper" under a 10:1 scheme.

**What to do when flagged**: reconcile the prices (supersede the wrong one via
`register_canonical_fact` + `supersedes_fact_id`) or correct the denomination.

### 17. Affordability (`affordability`)
Advisory (INFO). When a scene names a price (the same `<number> <unit>` pattern
`scan_scene_prices` detects) above a present character's tracked wealth — both
reduced to base units — it raises a tripwire. High-precision/low-recall: it only
fires when both a price mention and a tracked amount exist.

**What to do when flagged**: confirm the character can afford it (a loan, savings
off-page), or update their tracked amount via `commit_quantity_state`.

### 18. In-Scene Temporal Coherence (`temporal_coherence`)
The within-scene, forward-looking complement to §12 chronology (which is the
*between-scene* guard). Scans each scene's prose for time-of-day markers and
flags three things: **teleporting time** — a large forward time-of-day skip
(e.g. morning → night) with no transition beat or scene break; **drifting time**
— prose that contradicts its own established time of day (it is night, then the
same scene refers to morning); and an **unrendered declared span** — a scene
whose clock declares `duration_days` >= 1 but whose prose renders the span as
one unbroken block with no transition. The time vocabulary includes parts of
day, meal names, canonical hours (matins, vespers…), and explicit meridian clock
times (`8 a.m.`, `11 p.m.`). Prose-only and **calendar-free**: it runs even when
the project declares no calendar. High-precision/low-recall (a conservative
band-jump threshold, word-boundary matching, greeting and recollection guards),
so every finding is an advisory **warning**. Suppressed by a transition marker or
scene break between the times, by an in-scene recollection ("she remembered"), by
`temporal_mode` flashback/flashforward/concurrent, and at week-or-coarser
`precision`.

The same scan surfaces at **three enforcement points**, all advisory: on
`save_scene_draft` and `revise_scene` (the `temporal_findings` field — immediate
feedback while drafting), inside `commit_scene_changes` (a `temporal_findings`
field that never blocks the commit under any `continuity_gate`), and across the
whole branch via this `check_consistency` arm.

That deterministic scan is **Tier 1** (high precision, fixed vocabulary). Pass
`deep_check: true` to `check_consistency` to add **Tier 2**: a model-backed
semantic pass that reads each scene's prose and catches the jumps a fixed
lexicon cannot — idiomatic elapsed time ("three cigarettes later"), implied
light/meal/errand cues, and drift phrased in prose. It reuses the `review` model
route, is opt-in (one model call per scene), and degrades to nothing when no
review model is configured, so Tier 1 always stands on its own. Its findings
carry the same `temporal_coherence` check_type and `warning` severity.

**What to do when flagged**: add an explicit transition beat ("Hours later,") or
a scene break (`***`) at the jump; split a multi-block scene into separately
clocked scenes with `set_scene_clock`; reconcile the contradicting time-of-day
references; or, for a deliberate rewind, stamp `temporal_mode: "flashback"`. The
prevention side is the `[IN-WORLD TIME]` hard constraint surfaced in
`get_scene_context` / `get_chapter_briefing`, which feeds the previous scene's
end clock **and location** forward so the writer anchors where and when the new
scene starts. Persist the setting by passing `location_id` to `save_scene_draft`.

### 19. Motif Usage Audit (`motif_usage_audit`)
Metadata-only (beat annotations plus motif fields — never prose). For each
non-archived motif with a `max_uses_per_chapter` limit, it counts beat-annotation
motif links per chapter within scope and raises a **warning** when a chapter goes
**over** the limit (exactly at the limit is clean). A motif with no limit is
skipped for overuse. A motif declared but never linked by any beat annotation in
scope raises an **info**, but only when the scope covers **≥3 chapters**, so small
scopes stay quiet.

**What to do when flagged**: thin the motif's links in the offending chapter (or
raise `max_uses_per_chapter`) for overuse; annotate a scene with the motif, or
archive it, for the unused-info case.

### 20. Theme Placement Audit (`theme_placement_audit`)
Metadata-only. When a theme's `introduction_point` or `resolution_point` resolves
to a chapter **within scope** but no beat annotation in that chapter links the
theme, it raises an **info** naming the theme and the chapter. Placements that are
NULL or outside scope are silent.

**What to do when flagged**: annotate a scene in that chapter with the theme
(`annotate_scene_beats`), or move the theme's placement to a chapter where it is
actually developed.

### 21. Promise-Payoff Detection (`promise_payoff_detection`) — deep only
Model-backed **Tier 2**, opt-in via `deep_check: true` (silent otherwise). After
a long agent run, promises the prose has *already* paid off keep nagging as
overdue in §4 tracking, and genuinely dropped threads hide among those false
alarms. This check reads the scoped prose and **proposes** — it never writes —
that an unresolved promise which appears delivered be confirmed via
`update_promise_status`. It selects non-resolved (not `paid_off`/`abandoned`,
not archived) promises in scope, ranks them by §4 urgency (overdue > due > soon >
watch, tiebreak id), and audits the **10 most urgent**; if more than 10 exist,
one summary **info** finding reports how many were **not scanned** (never a
silent cap). Each positive match is an **info** finding naming the promise id, a
truncated description, the scene reference, and (when supplied) a sanitized
evidence excerpt — the message ends with `confirm with update_promise_status`.
It reuses the `review` model route (one call per promise, the full scoped prose
concatenated) and degrades to no proposals when the local stub is in play. If the
review route is unreachable it emits ONE honest-skip info finding that reads as
**SKIPPED** — a route failure never masquerades as a clean scan.

**What to do when flagged**: read the cited scene; if the payoff truly landed,
call `update_promise_status(promise_id, "paid_off")`; if it did not, reinforce
the thread or move `planned_payoff`. For the SKIPPED finding, configure a review
model route and re-run. For the summary "not scanned" finding, narrow the scope
or resolve higher-urgency promises first, then re-run.

### 22. Plot-Line Convergence Audit (`plot_line_convergence_audit`)
Metadata-only (beat annotations plus plot-line fields — never prose). A plot
line declares which conflicts/themes it expects to braid together at its
convergence via `connected_conflict_ids` / `connected_theme_ids` (both default
`[]`, so a plot line that declares neither is silent). When a plot line's
`convergence_points` resolve to a chapter **within scope** but **no** beat
annotation in that chapter links **any** of the declared connected conflicts (as
`conflict_ids`) OR connected themes (as `theme_ids`), it raises an **info**
naming the plot line and the chapter. Convergences outside scope, or plot lines
with no declared connections, stay quiet.

**What to do when flagged**: annotate a scene in the convergence chapter with a
connected conflict/theme (`annotate_scene_beats`), or revise the plot line's
`convergence_points` / connected-id declarations so the braid lands where it is
actually written.

### 23. Arc Milestone Audit (`arc_milestone_audit`)
Metadata-only. A character-arc milestone carries a `placement` (where it is due)
and, once demonstrated, a `reached_at` marker. For each milestone placed at a
chapter **P** in or before the scope end that is **not** marked reached: if the
manuscript already runs past P (max scoped chapter **≥ P+1**) it raises a
**warning** naming the arc and milestone label and the number of chapters
overdue (`max_scoped_chapter − P`); if the scope has only reached P (max **== P**)
it raises an **info** "due now". A milestone with a `reached_at` marker is silent
regardless, and a milestone with no `placement` is silent. Mark a milestone
reached by resending the arc's `milestones` array through `update_entity` with
`reached_at` set.

**What to do when flagged**: demonstrate the milestone in the prose and set its
`reached_at` (via `update_entity` on the `character_arc`), or move the
milestone's `placement` if the arc has legitimately slipped.

### 24. Conflict Escalation Audit (`conflict_escalation_audit`)
Metadata-only. A conflict's `escalation_demonstrated` array is index-aligned
with `escalation_stages`: each entry is the placement where that stage was
demonstrated, or `null`/absent for not-yet-demonstrated (a shorter-than-stages
vector reads as all-`null` beyond its length). When any pair of stages **(i < j)**
has a later stage **j demonstrated** while an earlier stage **i is not**, it
raises a **warning** naming the conflict and the out-of-order stage labels. All
demonstrated stages in order, or a conflict with no demonstration markers at all,
is silent.

**What to do when flagged**: demonstrate the earlier escalation stage before the
later one (or reorder the writing), or correct the `escalation_demonstrated`
markers (via `update_entity` on the `conflict`) if the demonstrations are
mislabeled.

### 25. Secret Leak (`secret_leak`)
The audience-direction complement to `knowledge_timing`. Where `knowledge_timing`
asks "does the *speaker* know this yet?", `secret_leak` asks the audience
question: "did an out-of-circle character act on a secret they were never told?"
A **secret** is a `canonical_fact` marked `secret = true`; its **circle of trust**
is derived — every character with a `knowledge_fact` row linked back to that fact
via `secret_of_fact_id`, placement-stamped by `learned_at`.

Deterministic tier (always runs, no `deep_check` needed): for each scoped scene,
for each secret fact whose circle **at that scene's story cursor** does not
include a **present** character, the check scans that out-of-circle character's
**attributed dialogue** (reusing the `voice_drift` speaker attribution) for the
secret's value lexeme (reusing `canonical_fact_prose_drift`'s whole-word matcher,
uninverted — a *hit* is the violation). A hit raises a **warning** naming the
fact, the character, the scene reference, and the reveal status ("no recorded
reveal to `<character>` before this scene"). An **insider** speaking the secret is
clean, and a reveal recorded in an **earlier** scene puts the speaker inside the
circle at the later cursor (so it is clean); a reveal only in a **later** scene
does not excuse an earlier leak. Cursor-aware, so a ch-12 reveal never leaks
backward into a ch-9 flashback. Behavioral/narration leaks (a character avoiding
a place they have no reason to avoid) are the **deep tier** (`deep_check`), a
later wave — v1 scans dialogue only. Silent for projects with no secret facts.

**What to do when flagged**: this is a deliberate-irony triage point. If the
reveal is intended (she told him off-page, or he guessed), record it —
`record_knowledge` with `secret_of_fact_id` set and `learned_at` at the reveal's
placement — which expands the circle and clears the finding. Otherwise the
dialogue genuinely leaks: route to scene-writer to revise it out. Mark a reviewed
false positive by recording the reveal or dismissing it with a note, mirroring how
other findings are triaged at checkpoints.

---

## Running a Full Audit

For a comprehensive quality check, call:

```
check_consistency({
  project_id: "<project record id>",
  scope: { scope_type: "full" },
  format: "markdown"   // optional: get a rendered report back as `markdown`
})
```

The output has three views:
- `issues: Vec<ConsistencyIssue>` — flat list of every finding (existing
  shape).
- `report_sections: Vec<ConsistencySection>` — Phase 4 validator findings
  grouped by `validator_id` and then by scene with positions in story order.
- `markdown: Option<String>` — populated when `format: "markdown"`. Errors
  are pinned under `## Hard constraints` (never trimmed); warnings and info
  findings sit under `## Validator findings` per validator.

This returns a structured report with severity levels:
- **ERROR**: Something is wrong and must be fixed (rule violation, knowledge contradiction)
- **WARNING**: Something is concerning (overdue promise, pacing behind schedule)
- **INFO**: Something to be aware of (low agency score, approaching pacing limit)

Present the results to the user organized by severity, then by type.
Suggest specific fixes for each issue using the appropriate skill:

If `canonical_fact_consistency` reports conflicting active facts, fix it by
calling `register_canonical_fact` with `supersedes_fact_id` for the outdated
fact. Do not use `update_entity` on `canonical_fact`; that entity type is not a
mutable `update_entity` target.

`canonical_fact_prose_drift` is the prose-vs-canon check to use when scene text
drifts from typed canonical facts.

Use these shipped Phase 4 validator IDs as your live evidence package:
- `canonical_fact_prose_drift`
- `world_rule_semantic_drift`
- `voice_drift`
- `retcon_reachability`
- `style_compliance`

The deterministic story-time and quantity checks (`chronology`,
`knowledge_timing`, `pacing_drift`, `quantity_drift`, `currency_consistency`,
`affordability`), the metadata audits (`motif_usage_audit`,
`theme_placement_audit`, `plot_line_convergence_audit`, `arc_milestone_audit`,
`conflict_escalation_audit`), and the secret-knowledge audit (`secret_leak`) are
requested by name and reported via the `check_type` field on each issue rather
than as Phase 4 validator rows.

Example:
- Canon says `cole.age = 20`.
- Scene prose says "Cole was nineteen."
- `check_consistency(..., checks=[\"canonical_fact_prose_drift\"])` flags the
  scene; treat this as must-fix unless canon is intentionally superseded.

Planned/Future naming in planning docs may use aliases like
`canonical_fact_violations`, `world_rule_hits`, or `retcon_findings`. Do not
use those alias names when calling live checks; use the concrete IDs above.

| Issue Type | Fix Skill |
|-----------|-----------|
| Character inconsistency | → character-creator (update profile) or → scene-writer (revise) |
| World rule violation | → scene-writer (revise scene) or → worldbuilder (revise rule) |
| Overdue promise | → scene-writer (write payoff scene) or → plot-architect (reschedule) |
| Pacing violation | → plot-architect (rebalance) |
| Agency deficit | → scene-writer (write active-choice scene) |
| Tone deviation | → scene-writer (revise scene tone) |
| Knowledge contradiction | → scene-writer (revise to remove forbidden knowledge) |
| Chronology drift (`chronology`) | → scene-writer (set `temporal_mode`/`thread_key` or fix `day_index`) |
| Knowledge timing (`knowledge_timing`) | → scene-writer (move the reveal) or → plot-architect (reschedule) |
| Pacing drift (`pacing_drift`) | → plot-architect (rebalance) or → scene-writer (raise the stakes) |
| Quantity drift (`quantity_drift`) | → scene-writer / worldbuilder (re-commit with a `change_reason`, or stage the change) |
| Currency consistency (`currency_consistency`) | → worldbuilder (reconcile prices / fix the denomination) |
| Affordability (`affordability`) | → scene-writer (confirm the purchase or update tracked wealth) |
| Motif usage (`motif_usage_audit`) | → scene-writer (thin/annotate the motif) or → plot-architect (raise `max_uses_per_chapter` or archive) |
| Theme placement (`theme_placement_audit`) | → scene-writer (annotate the theme in that chapter) or → plot-architect (move the placement) |
| Promise-payoff candidate (`promise_payoff_detection`, deep) | → plot-architect / bible-librarian (confirm with `update_promise_status("paid_off")`, or reinforce the thread if not actually landed) |
| Plot-line convergence (`plot_line_convergence_audit`) | → scene-writer (annotate a connected conflict/theme in the convergence chapter) or → plot-architect (revise `convergence_points` / connected-id declarations) |
| Arc milestone (`arc_milestone_audit`) | → scene-writer (demonstrate the milestone and set `reached_at`) or → plot-architect (move the milestone `placement`) |
| Conflict escalation (`conflict_escalation_audit`) | → scene-writer (demonstrate the earlier stage first) or → plot-architect (correct the `escalation_demonstrated` markers) |
| Secret leak (`secret_leak`) | → scene-writer (revise the leaking dialogue) or, if the reveal is intended, `record_knowledge` with `secret_of_fact_id` to expand the circle |

If a world rule has a legitimate exception, encode it as a separate
`world_rule` (e.g. with `relevance_tags: ["exception"]` and a
`scan_pattern` matching the exception trigger) — there is no shipped
"exception" field on the rule itself.

---

## Proactive Monitoring

Don't wait for the user to ask for a consistency check. After every 5-10 scenes
written, suggest running `check_consistency` on the recent chapter range.
Catching problems early is far cheaper than catching them after 50 scenes.

## Dual-Persona Review Loop (from autonovel)

For deep quality review of a chapter or manuscript section, run a dual-persona
analysis. This catches what automated checks cannot — prose-level repetition,
character thinness, pacing drag, thematic incoherence.

### How to run it:

1. Gather the prose text for the section being reviewed.
2. Analyze it through TWO personas sequentially:

**Persona 1 — Literary Critic**: Read the prose as a demanding reader.
- Is the opening hook effective? Would you keep reading?
- Do the characters feel like real people or cardboard cutouts?
- Is the dialogue natural? Does each character sound distinct?
- Are there passages where attention wanders? Why?
- Does the scene earn its emotional moments or reach for them cheaply?
- Is there anything that feels contrived, convenient, or unearned?

**Persona 2 — Craft Technician**: Read the prose as a writing professor.
- Are MRUs in the correct order (motivation before reaction)?
- Is show-don't-tell consistently applied?
- Are there filter words ("felt", "seemed", "realized")?
- Are there AI slop patterns? (See `bible://references/anti-slop`.)
- Is POV discipline maintained throughout?
- Are dialogue tags minimal and action beats doing the work?
- Is sentence length varied for rhythm?
- Are there crutch words or repeated phrases?

3. Compile specific, actionable items from both personas.
4. Prioritize: fix structural issues first (plot, character), then prose quality.
5. Use **scene-writer** to revise, addressing the top issues.
6. Re-run the review. Loop until the reviewer's items are mostly qualified hedges
   ("this is a minor stylistic preference") rather than real problems.

This loop typically takes 2-3 iterations to produce clean prose.

---

## Subagent orchestration (Claude Code / grok)

A multi-chapter continuity sweep is naturally shardable: evidence-gathering for
each chapter (or each tracked entity) is independent read-only work. If your
harness supports subagents (Claude Code's Task/Agent tool, grok's subagents),
fan the sweep out; otherwise walk the chapters/entities sequentially inline —
the diagnosis is identical, only the concurrency changes.

**Write discipline (non-negotiable):** subagents research and report only. Every
state-mutating call stays in the main context. Subagents call read-only tools —
`check_consistency`, `find_scenes_referencing`, `find_entity`, `get_entity`,
`get_scene_context`, and the `bible://.../continuity/health` resource — and
return structured findings. They never call `commit_character_state`,
`register_canonical_fact`, `update_promise_status`, `set_arc_pacing_constraints`,
`migrate_canonical_fact`, `commit_quantity_state`, or any `update_*`/`commit_*`
write. The continuity-editor (main context) decides and writes.

Fan-out for a sweep: dispatch **one subagent per chapter** (for a chapter-range
audit) or **one per entity** (for a subject-scoped audit, `subjects: [...]`),
each running its scoped `check_consistency` plus backreference lookups and
returning findings keyed by scene id, check_type, and severity. The supervisor
then **merges and dedupes** — the same drift often surfaces from adjacent
chapters or from two entities that share a scene — ranks by severity, and hands
each surviving issue to the owning skill. Without subagents, run the scoped
checks one chapter/entity at a time and dedupe as you go.

---

## Skill Chains

- **← scene-writer**: Every scene saved triggers minor validation.
  The continuity-editor provides deeper, cross-scene analysis.
- **← plot-architect**: Plot architecture establishes the expectations
  that the continuity-editor validates against.
- **→ scene-writer**: Flagged issues often require scene revision.
- **→ revision-manager**: Significant structural issues may require branching
  and cascading state recomputation.
- **→ plot-architect**: Pacing problems may require rebalancing or restructuring.

---

## References

The shipped craft references most relevant to continuity work:

- `bible://references/anti-slop` — Pattern catalog for prose-level drift the
  Phase 4 validators don't catch (cliché abstractions, filter-word creep,
  generic emotional shorthand).
- `bible://references/voice-differentiation` — Reference for diagnosing voice
  drift findings beyond raw forbidden-word matches.
- `bible://references/swain-scene-sequel` and `bible://references/mru-guide`
  — Useful when a continuity issue is rooted in poor scene structure rather
  than a true canon contradiction.
