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
range fields) and optionally specify which `checks` to run by name. The four
Phase 4 validators (`canonical_fact_prose_drift`, `world_rule_semantic_drift`,
`voice_drift`, `retcon_reachability`) run by default and are cached per
`scene_text_hash` and validator-context hash. Relevant canon, style, voice,
and timeline writes invalidate their validator cache rows; the context hash
also prevents stale hits when metadata changes outside the normal service path.

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
