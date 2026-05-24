---
name: manuscript-importer
description: >
  Use when the user wants to import an existing manuscript, novel, webnovel, or book
  into Spindle so the system can continue writing from where it left off. Triggers for
  "import this book", "continue this story", "I have an existing manuscript", "analyze
  this novel", "pick up where this left off", or any request involving ingesting existing
  fiction into the Spindle system. This is a multi-pass pipeline that reverse-engineers
  a complete Story Bible from raw prose.
---

# Manuscript Importer

You have an existing manuscript and want Spindle to continue from the current
ending instead of starting from an empty Bible. Use the shipped import MCP
tools and import-session resources to build, inspect, review, and hydrate the
canon step by step.

## Phase 3 source-bridge sync workflow

When imported chapters also live in local manuscript files, use the source
bridge tools as the canonical sync path:

1. Call `pull_chapter_from_file` before drafting when the file is the latest
   source of truth.
2. Run import passes and review decisions.
3. Call `push_chapter_to_file` after accepted updates to write branch scenes
   back to disk and refresh `scene_source_link` offsets.
4. If offsets drift after external edits, call
   `backfill_scene_source_offsets` for the project branch.

Treat direct file edits outside these tools as exceptional. Always reconcile
with pull/push before final continuity checks.

## What this skill uses

This workflow is built around the public import surface that now exists in MCP.
The main tools are:

- `import_manuscript`
- `import_status`
- `import_extract_entities`
- `import_consolidate_entities`
- `import_analyze_character`
- `import_extract_world`
- `import_analyze_narrative`
- `import_compute_final_state`
- `import_hydrate_bible`
- `import_apply_review_decisions`
- `record_knowledge`

The main resources are:

- `bible://projects/{project_id}/imports`
- `bible://projects/{project_id}/imports/{session_id}`
- `bible://projects/{project_id}/imports/{session_id}/structure`
- `bible://projects/{project_id}/imports/{session_id}/review-items`
- `bible://projects/{project_id}/imports/{session_id}/hydration-report`

## Prerequisites

The user must provide the manuscript as local files. Supported formats are
`txt`, `md`, `html`, `epub`, and `docx`.

The user can also provide optional notes about intended direction, continuity
corrections, missing world rules, or planned future reveals.

## The import loop

The import is a reviewable, resumable pipeline. After each major pass, inspect
`import_status` or the import resources, then resolve review items before you
hydrate the Bible.

### Pass 0: structural analysis
- Accept the manuscript in any format (txt, md, html, epub, docx)
- Split into chapters and scenes (detect "Chapter N", roman numerals, large breaks)
- Within chapters, detect scene breaks (blank lines, "***", "---")
- Identify POV character per scene (whose thoughts are accessible)
- Count words per chapter/scene
- Report to user: "I found X chapters, Y scenes, Z POV characters"
- Ask: "Does this look right? Any chapters missing or mis-split?"

Call `import_manuscript` to start. Required and useful fields on
`ImportManuscriptInput`:

- `source_paths: Vec<String>` — local file paths to ingest.
- Either `target_project_id` (hydrate into an existing project) OR
  `create_project_name` (start a fresh project — the default for new imports).
- `source_format_hint: Option<ImportSourceFormat>` — one of `txt`, `md`,
  `html`, `epub`, `docx`. Spindle will auto-detect when omitted.
- `duplicate_strategy: Option<ImportDuplicateStrategy>` — one of `reject`
  (the default — fail if the same file was already imported) or
  `create_new_session` (always start fresh).

Then `import_status` to check progress. If you need a stable resource path
for the same data, read:

- `bible://projects/{project_id}/imports/{session_id}`
- `bible://projects/{project_id}/imports/{session_id}/structure`

After every pass below, also read
`bible://projects/{project_id}/imports/{session_id}/review-items` and
resolve open items with `import_apply_review_decisions` before moving on —
review items accumulate per-pass, not just at the end.

### Pass 1: entity extraction
For each chapter, extract:
- **Characters**: Every named character, aliases/nicknames/titles, role in chapter
- **Locations**: Every named location, type, descriptions given
- **Key Events**: 3-5 most plot-significant things that happen
- **Information Exchanged**: Who told whom what? What was learned?
- **Relationship Moments**: Interactions that shifted a relationship

Call `import_extract_entities` for the session. Optional inputs:
`segment_ids: Vec<String>` (narrow to specific segments — useful for retries)
and `limit: Option<usize>` (cap the number of segments processed in this
call, useful for very long manuscripts).

Present entity list to user: "I identified these characters..."
Ask: "Am I missing anyone? Are any of these the same person with different names?"

### Pass 2: entity consolidation
- Deduplicate entities across chapters (Marcus = the Knight-Commander = Ash)
- Build canonical entity list with all aliases
- Track first appearance, last appearance, total scene count
- Rank characters by importance (POV chapters + scene count)

Call `import_consolidate_entities`.

### Pass 3: deep character analysis
For each major character (top 10-15 by importance), analyze ALL their scenes:

**Voice extraction**: Collect every dialogue line. Analyze vocabulary level,
sentence structure, verbal tics, profanity level, formality range, humor style.
Generate 5-8 example lines in different emotional states.
This produces imported dossier data that later hydrates into canonical
`character_voice_profile` and `character_emotional_profile` records.

**Emotional profile inference**: From reactions across the manuscript, infer
base emotional state, suppressed emotions, trigger overrides, defense mechanisms.
→ Creates `character_emotional_profile`

**Decision-pattern inference**: From choices made, infer moral alignment,
risk tolerance, decision speed, and stress response, then capture the useful
parts in summaries, arcs, and emotional-state guidance.

**State trajectory**: Track how the character changes across chapters.
What were they like in chapter 1 vs the final chapter?

**Relationship mapping**: For each significant relationship, track initial
dynamic, evolution, and current state (trust/tension estimate at manuscript end).

Call `import_analyze_character` once per BATCH of characters, not once per
character. The input takes `cluster_ids: Vec<String>` — pass the cluster
IDs for all characters you want analyzed in this call. The pipeline will
analyze each cluster's scenes and produce per-character dossiers in one
pass.

Present profiles to user: "Here's how Marcus speaks — does this sound right?"
The user validates, corrects, and enriches each profile.

### Pass 4: world extraction
Across all chapters:

**World rules**: Explicit ("Magic requires physical contact") AND implicit
(if every time magic is used the caster bleeds, that's a rule even if never
stated). Look for costs, limitations, prerequisites, and side effects.

**Locations**: Descriptions, sensory details, controlling factions, current states.

**Factions**: Political groups, organizations — goals, methods, alliances.

**Glossary**: Every invented term with context for how it's used.

Call `import_extract_world`.

Present: "Here are the rules I found for how magic works in this world..."
Ask: "Are there rules I missed? Anything the text implies but never states?"

### Pass 5: narrative architecture
The hardest pass. Analyze the FULL manuscript for:

**Plot lines**: What storylines are running? Which are resolved vs still active?

**Narrative promises**: What was set up but never paid off? Weapons described
in detail but never used. Characters introduced with mysterious motives never
explained. Prophecies mentioned but not fulfilled.

**Character arcs**: Infer Ghost/Wound/Lie/Want/Need/Truth from behavior patterns.
Where is each character on their arc — early, middle, or late?

**Themes**: Recurring moral dilemmas, characters embodying opposing positions.

**Reader contract**: From opening 3-5 chapters, infer tone, genre expectations,
narrative style, pacing rhythm.

Call `import_analyze_narrative`.

Present: "These plot threads seem unresolved..."
Ask: "What was supposed to happen next? Where was the story going?"
This is where the user's knowledge of the PLANNED story matters most.

### Pass 6: final state snapshot
Compute the EXACT state at the manuscript's END:

For each character:
- Physical location, emotional state, active goals
- Physical condition (injuries, resources, equipment)
- What they know and don't know
- Relationship trust/tension levels with every significant character

For each active plot thread:
- Current tension, next expected beat

For the world:
- Political situation, active threats, available resources

Call `import_compute_final_state`.

Present the full state snapshot for validation.

`import_apply_review_decisions` takes `decisions: Vec<ImportReviewDecisionInput>`
where each entry has `review_item_id`, `resolution: ImportReviewStatus`
(values like `accepted`, `rejected`, `corrected`, `deferred`),
`correction: Option<...>` (when overriding the inferred value), and
`resolver_notes: Option<String>`. Resolve any open items from THIS pass
(and any earlier passes you skipped review on) before hydration.

### Pass 7: Bible hydration
Call `import_hydrate_bible`. Required and optional fields on
`ImportHydrateBibleInput`:

- `project_id`, `session_id` — required.
- `include_scenes: bool` — REQUIRED, no default. `true` to materialize scene
  prose into canonical `scene` records (recommended for full continuation);
  `false` to skip scene-level hydration and only land entities.
- `hydrate_mode: Option<ImportHydrationMode>` — controls hydration target.
  Values include `new_project` (default — create a fresh project from the
  import) and `existing_project_branch` (hydrate into an existing project,
  requires `target_project_id` and optionally `target_branch_id`).
- `target_project_id` / `target_branch_id` / `create_project_name` — used
  with `hydrate_mode` to pin the destination.

This pass writes canonical project records from the imported dossiers and final
state, including:

- books and chapters
- characters, locations, world rules, factions, religions, economies, and terms
- plot lines, conflicts, themes, motifs, narrative promises, and character arcs
- scenes, relationships, final character-state snapshots, and deferred knowledge
- hydration report metadata for later inspection

Report: "Bible is populated. You have X characters, Y locations, Z active plot
threads. Ready to continue writing from [last chapter + 1]."

Read `bible://projects/{project_id}/imports/{session_id}/hydration-report` to
inspect exactly what was created, skipped, or downgraded.

---

## After Import: First Continuation

Ask: "What should happen next?"
Switch to the **scene-writer** skill with the fully populated Bible.
Write the first new scene maintaining full consistency with everything imported.

If the user later needs to correct canon without rerunning the whole import,
use `record_knowledge` for canonical character knowledge updates. Required
fields: `character_id`, `fact`, `source_summary`, `reader_visible: bool`.
Optional `learned_at: StoryPlacement` ties the fact to a specific story
position so future scenes can reason about when the character learned it.
For non-character canon, use `register_canonical_fact` and the normal
entity tools.

---

## Critical Design Principle: Human-in-the-Loop

The import pipeline is NOT fully autonomous. After each pass, present results
for validation. The user knows things the text doesn't say:

- Characters who will become important later but seem minor now
- World rules in the author's head but not yet on the page
- Planned plot directions that haven't been hinted at yet
- The REAL relationships between characters (subtext the LLM might miss)

---

## Token Budget

A 300-page manuscript (~100K words) requires:
- Pass 0: ~0 tokens (structural, no LLM)
- Pass 1: ~60K input tokens (30 chapters × 2K each)
- Pass 2: ~10K tokens (one consolidation call)
- Pass 3: ~200K tokens (10 characters × 20K each)
- Pass 4: ~150K tokens (5 calls × 30K each)
- Pass 5: ~300K tokens (needs large context model)
- Pass 6: ~50K tokens
- Pass 7: ~0 tokens (database writes)
Total: ~770K input tokens (~$3-5 at typical API pricing)

---

## Skill Chains

- **→ scene-writer**: After import, continue writing with the populated Bible
- **→ character-creator**: If imported profiles are thin, enrich them
- **→ worldbuilder**: If imported world rules are incomplete, expand them
- **→ plot-architect**: If the user wants to restructure before continuing
- **→ continuity-editor**: Run a full consistency check on the imported Bible

---

## References

- `docs/spindle-implementation-brief.md` — Current 5-crate architecture and
  crate ownership boundaries
- `README.md` — Current public MCP tool and resource surface
