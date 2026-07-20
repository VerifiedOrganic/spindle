---
name: bible-librarian
description: >
  Use when the user wants to browse, search, query, summarize, or export any aspect of the Story
  Bible. This includes looking up character details, searching for related entities, reviewing
  relationship webs, checking the current state of the world, reading scene summaries, viewing
  the pacing dashboard, listing narrative promises, or exporting data. Also triggers for "show me
  Marcus's profile", "what do I know about Port Aldren", "who's allied with the Iron Pact",
  "summarize what's happened so far", "show me the relationship map", "what's the state of
  Elena's arc", "list all my characters", "export the Bible", or any request about browsing,
  querying, or understanding the current state of the story. This is the "look things up" skill.
---

# Bible Librarian

## Lookup workflow

Use this lookup order:

1. Call `set_active_project` once per session so subsequent tool calls inherit
   the project (and active branch).
2. Call `get_writer_state` first when the user asks "where am I?" or "what's
   open?". It returns the current `writer_position`, `open_promises_due_now`,
   `recent_session_activity`, and `active_overlays` in one shot. Inputs
   include `include_subjects`, `include_recent_activity`,
   `recent_activity_limit`, `format`, and `budget_tokens`.
3. Call `find_entity` when you have a name, alias, or fuzzy phrase.
4. Call `get_entity` with the resolved `table` and `id` to read the
   canonical subject snapshot. The `bible://{table}:{id}` direct-resource
   form is an alternative for cached reads when you already know the ID.
5. For character deep reads, call `get_character_snapshot` after
   `get_entity`.
6. Call `find_scenes_referencing` when you need scene-level evidence for a
   subject or phrase.
7. Call `get_chapter_briefing` for a per-chapter recap that's richer than a
   raw scene list (POV, summary, beats, knowledge state).
8. Use `search_bible` for exploratory retrieval; use `bible://...`
   resources for stable list browsing.

Treat `find_entity` + `get_entity` as the primary fact-lookup path. This
reduces ID drift and keeps continuity answers anchored to canonical records.

## Resource vs Tool Rule

Spindle exposes data through two MCP interfaces with a clear separation:

**Resources (`bible://...`)** are for **stable, infrequently-changing reads** that benefit from
caching. Use resources when you want:
- Project entity lists (characters, locations, factions, world rules, etc.)
- Embedded skills or craft references
- System configuration snapshots
- Direct entity lookups when you already know the ID

**Tools** are for **everything else**: state changes, computations, parameterized queries,
and dynamic results. Use tools when you need to:
- Create, update, or archive entities
- Search the Bible with specific parameters (`search_bible`, `find_scenes_referencing`)
- Get scene context with formatting (`get_scene_context`)
- Run consistency checks or dual-persona reviews
- Manage branches, revision markers, or import sessions

**Quick decision guide:**
- Want to **read** stable state? → Resource
- Want to **change** anything or **search** dynamically? → Tool

This rule lets you decide unambiguously: if it changes state or requires parameters, it's a
tool; if it reads cached state, it's a resource.

### Full categorization

Some data is available through both interfaces. The table below shows every overlap.

| Resource | Tool | When both exist, prefer |
|---|---|---|
| `bible://projects` | `list_projects` | Resource for browsing; tool for dynamic operations |
| `bible://projects/{id}/books` | (none — books listing is resource-only) | The resource lists books; `list_book_chapters` is a separate tool that lists chapters within ONE book given its id, not equivalent. |
| `bible://projects/{id}/chapters/{b}/{c}/scenes` | `list_chapter_scenes` | Resource for cached browsing; tool for explicit params |
| `bible://config/agents` | `list_agents` | Resource for cached browsing; both return same data |

All other resources have **no read-equivalent tool** — they are resource-only reads.
(Write tools like `create_character` map to their corresponding resource for reading,
but the resource itself has no dedicated read tool.)
All other tools have **no resource equivalent** — they are tool-only operations
(writes, searches, computes, imports).

### Write-to-read mapping

Every `create_*` / `update_*` / `archive_*` tool mutates data that is then readable
through its corresponding resource:

| Write tool | Read resource |
|---|---|
| `create_character` | `bible://projects/{id}/characters` |
| `create_location` | `bible://projects/{id}/locations` |
| `create_faction` | `bible://projects/{id}/factions` |
| `create_religion` | `bible://projects/{id}/religions` |
| `create_economy` | `bible://projects/{id}/economies` |
| `create_term` | `bible://projects/{id}/terms` |
| `create_relationship` | `bible://projects/{id}/relationships` |
| `create_world_rule` | `bible://projects/{id}/world-rules` |
| `create_plot_line` | `bible://projects/{id}/plot-lines` |
| `create_conflict` | `bible://projects/{id}/conflicts` |
| `create_theme` | `bible://projects/{id}/themes` |
| `create_motif` | `bible://projects/{id}/motifs` |
| `create_narrative_promise` / `update_promise_status` | `bible://projects/{id}/narrative-promises` |
| `create_character_arc` | `bible://projects/{id}/character-arcs` |
| `create_future_knowledge` | `bible://projects/{id}/future-knowledge` |
| `create_timeline_event` | `bible://projects/{id}/timeline-events` |
| `create_temporal_intervention` | `bible://projects/{id}/temporal-interventions` |
| `create_system_overlay` | `bible://projects/{id}/system-overlays` |
| `create_pacing_config` / `set_arc_pacing_constraints` | `bible://projects/{id}/pacing/overview` |
| `save_summary` | `bible://projects/{id}/chapter-summaries` |
| `create_branch` / `switch_branch` | `bible://projects/{id}/branches` (write-only tools; resource provides the read) |
| branch/timeline continuity audit | `bible://projects/{id}/continuity/health` and `bible://projects/{id}/timeline-graph/mermaid` |
| `import_manuscript` | `bible://projects/{id}/imports` |

The Story Bible is a living document with 60+ record types, hundreds of entities, and thousands
of relationships. This skill is how you navigate it — finding information, understanding the
current state, and presenting it clearly.

## Browsing Entities

Use MCP resources to browse:

```
bible://projects/{project_id}/books                   → All books with chapter/scene counts
bible://projects/{project_id}/chapters                → All chapters with scene counts
bible://projects/{project_id}/characters              → All characters
bible://projects/{project_id}/locations               → All locations
bible://projects/{project_id}/factions                → All factions
bible://projects/{project_id}/plot-lines              → All plot lines
bible://projects/{project_id}/conflicts               → All conflicts
bible://projects/{project_id}/themes                  → All themes
bible://projects/{project_id}/motifs                  → All motifs
bible://projects/{project_id}/world-rules             → All world rules
bible://projects/{project_id}/narrative-promises      → All promises
bible://projects/{project_id}/pacing/overview         → Pacing overview
bible://projects/{project_id}/branches                → All branches
bible://projects/{project_id}/reader-contract         → The reader contract
bible://projects/{project_id}/chapter-summaries       → Saved chapter summaries
bible://projects/{project_id}/future-knowledge        → Future knowledge records
bible://projects/{project_id}/timeline-events         → Timeline events
bible://projects/{project_id}/timeline-graph/mermaid  → Branch/timeline graph
bible://projects/{project_id}/temporal-interventions  → Temporal interventions
bible://projects/{project_id}/system-overlays         → System overlays
bible://projects/{project_id}/continuity/health       → Continuity health summary
bible://projects/{project_id}/research                 → Concise list of all research sources, notes, and claims
bible://projects/{project_id}/research/tags            → All unique tags used in the research library
bible://references/anti-slop                          → Craft reference
bible://system/model-routes                           → Model routing metadata
```

For concrete scene navigation, use the chapter scenes resource template:

```
bible://projects/{project_id}/chapters/{book_number}/{chapter_number}/scenes
```

That returns the active-branch scene ids, `scene_order`, and summaries for one
chapter.

For direct lookup when you already know a record ID, use the resource template
`bible://{table}:{id}`. Examples:

```
bible://character:marcus
bible://world_rule:iron-burns-fae
bible://scene:xyz789
```

When presenting results to the user, format them as clear summaries — not raw data dumps.
Highlight what's interesting, active, or problematic.

## Searching the Bible

For indexed canon lookups, call `search_bible` (note: field name is
`project_id`, not `project`):

```
search_bible({ project_id, query: "military characters near Port Aldren", limit: 10 })
search_bible({ project_id, query: "Eddie Edgar", mode: "exact", field: "name", limit: 5 })
search_bible({ project_id, query: "Eddie Edagr", mode: "fuzzy", field: "name", limit: 5 })
```

Use the search modes deliberately:

- `mode="semantic"` for concept recall when the user remembers the idea but not
  the exact wording.
- `mode="exact"` for exact names, rule phrases, and other anchor text after a
  context compression.
- `mode="fuzzy"` for typo-tolerant recall when the remembered name is slightly
  wrong.

Add `field="name"` when looking for entity names, `field="content"` when
searching descriptions/body text, and `field="tags"` when targeting rule or
entity tags. Add `subject_table="character"` / `location` / `world_rule` /
etc. when the user already knows the record family.

For LLM-ready output, add `format: "markdown"` and (optionally)
`budget_tokens: <n>`. The output then includes a `markdown` field rendered
ready to present to the user, with results trimmed from the lowest-scoring
end if the budget would be exceeded.

Present results with source type, relevance score, and a brief excerpt. Use it
for indexed canon entities and semantic recall, not for enumerating scene
records. If you need scene ids or chapter membership, use the books/chapters
resources and the chapter scenes resource template instead. If you need the
scenes that mention a subject or exact phrase, call:

```
find_scenes_referencing({ project_id: "...", query: { kind: "subject", subject_id: "character:..." } })
find_scenes_referencing({ project_id: "...", query: { kind: "phrase", phrase: "Lou arrangement" } })
```

## Deep Character Lookup

When the user asks about a specific character, follow the lookup order from
the top of this skill:

1. Call `find_entity` to resolve the name to a record id.
2. Call `get_entity` for the canonical subject snapshot.
3. Call `get_character_snapshot` for voice profile, latest state, recent
   appearances, and active relationships in one shot.
4. Call `find_scenes_referencing` for scene-level evidence of recent actions.
5. Read `bible://projects/{project_id}/pacing/overview` for arc pacing state.
6. Read advanced resources like `future-knowledge` when the story uses them.

Present as a structured character brief:
```
MARCUS (protagonist, Book 2 Chapter 15)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

CURRENT STATE
  Emotional: guarded but cracking, guilt over the harbor incident
  Physical: healing shoulder wound (chapter 12), otherwise fit
  Goals: find Elena before the council does, clear his name
  Status: fugitive, injured, hunted

ARC: Redemption (growth) — 37% complete, ON TRACK
  Last milestone: "accepted help from a stranger" (ch 12)
  Next milestone: "admits responsibility for past failures" (planned ~ch 20)
  Pacing: budget remaining 63%, velocity healthy

KEY RELATIONSHIPS
  → Elena (complicated): trust 0.4 ↑, tension 0.7 ↓
    Arc phase: "reluctant allies" — approaching "trust tested"
  → Commander Voss (antagonist): trust -0.8, tension 0.9
  → Kai (deceased): residual guilt, drives current motivation

KNOWS (that Elena doesn't)
  - The council has a spy in the resistance (learned ch 10)
  - Voss is Elena's father (learned ch 14) ← DRAMATIC IRONY

DOESN'T KNOW
  - Elena is working with the council (reader knows from ch 8)
  - The silver compass is a tracking device
```

## Relationship Web

When the user asks about relationships, present the network:

For a character: list all relationships with trust/tension levels and arc phases.
For a pair: show the full history — trust trajectory, key events, content unlocks.
For a faction: show all `controls`, `allied_with`, `trades_with` edges.

## State-at-Point Queries

When the user asks "what was the state of X at chapter Y":

1. Call `get_writer_state` first if "X" is the project as a whole — it's the
   fastest re-anchor for active branch, cursor, open promises, and recent
   activity.
2. For per-character or per-location state at a point, use `get_scene_context`
   at the target chapter or scene to inspect the effective branch-aware
   story state. If you only know the chapter, use
   `bible://projects/{project_id}/chapters/{book_number}/{chapter_number}/scenes`
   to locate concrete scene ids first.
3. Call `list_revision_markers` if the user wants to know what outstanding
   edits are pending at this point.

## Summary Generation

When the user asks "summarize what's happened so far" or "recap":

For each chapter up to the current point:
1. Read `bible://projects/{project_id}/chapter-summaries`
2. If a chapter summary is missing, read that chapter's scene list via the
   chapter scenes resource
3. Highlight key events, character changes, relationship shifts
4. Note which arcs advanced and which promises were planted/paid off
5. Present as a narrative recap, not a data dump

Call `save_summary` on any chapters that don't have summaries yet. The
`entity_id` field accepts the chapter record id directly; `chapter_id` is
accepted as a deserialization alias for the same field. Pair `entity_id`
with `entity_type: "chapter"` so the save resolves to the right record. If
you don't have an id, pass explicit `book_number` + `chapter_number`.

`get_chapter_briefing` is a higher-level recap tool that complements
`save_summary` — call it when you want a structured per-chapter view (POV,
beats, knowledge state) rather than just the prose summary.

## Reader Artifacts (spoiler-bounded)

Two read-only tools assemble **reader-facing** Markdown that is safe to hand to
someone who has only read up to a point. Both are pure read models — no model
calls — and both take an opt-in `write_to_workspace` that mirrors
`compile_manuscript` (the file lands in the project's workspace `artifacts/`
directory under a deterministic name).

- `export_recap { project_id, book_number, through_chapter, write_to_workspace }`
  — a "previously on" recap of one book up to (and including) `through_chapter`:
  a story-so-far section from the chapter summaries, a "Paid off" section of
  resolved promises, and a "Questions still hanging" section of open promises.
  Use it to catch a reader up before the next book or chapter.
- `export_series_bible { project_id, through?, write_to_workspace }` — a series
  bible as of an optional `through` placement (absent = whole project):
  character pages (role, summary, status/mood as-of the cursor, banded
  relationships), locations, a sorted glossary of terms, and factions/religions
  when present.

**The spoiler rule (both tools).** Everything is bounded at a cursor = the end
of `through_chapter` / `through` (whole book/project when absent). A fact,
thread, summary line, character page, or glossary entry is only included if it
is established at or before that cursor.

Secrets are handled more strictly than in scene context. A secret canonical
fact (registered with a `secrecy` scope) — and any line that names its value —
is **withheld from the reader** until the author has placed a *reader-visible
reveal*: a `record_knowledge` call carrying `secret_of_fact_id` (the linked
fact), `reader_visible: true`, and a `learned_at` placement at or before the
cursor. `reader_visible` is the **authorial dial** and the reveal's placement is
the **reveal date**. A secret merely held by a character (declared via
`register_canonical_fact { secrecy }`) is dramatic irony, not a reader reveal,
so it stays hidden in these artifacts until you record that dated, reader-visible
reveal. This differs from scene-context gating, which protects *characters* from
knowing; reader artifacts protect the *reader* from spoilers.

## Pacing Dashboard

When the user asks about pacing:

Read `bible://projects/{project_id}/pacing/overview` and present:

```
PACING DASHBOARD — Book 2
━━━━━━━━━━━━━━━━━━━━━━━━━

Marcus: Redemption Arc (growth)
  ████████░░░░░░░░░░ 37% | Budget: 63% remaining | ON TRACK
  Next milestone: ch ~20 | Velocity: normal

Elena: Agency Arc (transformation)  
  ██████░░░░░░░░░░░░ 28% | Budget: 72% remaining | BEHIND ⚠️
  Suggestion: needs advancement in next 2-3 scenes

Marcus↔Elena: Trust Arc
  ████████████░░░░░░ 55% | Phase: reluctant allies | AHEAD ⚡
  In cooldown: 2 chapters remaining before next phase transition allowed

Main Conflict: Council Conspiracy
  Try-fail cycles: 2/4 completed
  Current stage: "escalation" | Next attempt: planned ch 18

PROMISES
  ⏳ Silver compass (planted ch 2, reinforced ch 9) — due by ch 25
  ⏳ Anonymous message sender (planted ch 3) — OVERDUE, no reinforcement
  ✅ Harbor secret (planted ch 5, paid off ch 14)
```

## Research Library

Use the Research Library tools to store and query external research, factual claims, and historical details that ground the narrative:

- Call `research_add_source` to document a new research source (books, articles, interviews, files, etc.).
- Call `research_add_note` to attach specific quotes, page numbers, or general notes to an existing source.
- Call `research_add_claim` to record a distilled factual claim (with topic, location, time period, confidence, and tags) linked to a source or note.
- Call `research_search` to search the research library using full-text search matching claim text, note text, and source titles, optionally filtering by tags, time period, or location.
- Call `research_pack_for_scene` to retrieve a compact, budget-aware packet of relevant research matching a scene's summary, location, character IDs, or tags. The scene-writer or authoring supervisor should call this tool before writing a scene draft to ground the prose without bloating the prompt context.

The research library can be browsed using these resources:
- `bible://projects/{project_id}/research` returns a concise listing of all sources, notes, and claims.
- `bible://projects/{project_id}/research/tags` returns all unique tags used in the research library.

## Export

When the user wants to export:
- Reader recap → `export_recap` (spoiler-bounded "previously on"; see Reader Artifacts)
- Reader-facing series bible → `export_series_bible` (spoiler-bounded; see Reader Artifacts)
- Bible summary → Generate a comprehensive document using all entity summaries
- Manuscript → `compile_manuscript` (assemble scene prose in order)
- Character sheets → Detailed profiles for all characters
- World guide → All world entities, rules, and maps

## Skill Chains

- **→ scene-writer**: After looking something up, the user often wants to write next.
- **→ continuity-editor**: If browsing reveals something that seems wrong, switch to
  the continuity-editor to diagnose.
- **→ character-creator**: If a character feels thin while browsing, switch to
  character-creator to flesh them out.
- **→ revision-manager**: If browsing reveals something that needs changing,
  switch to revision-manager to handle the structural revision.

---

## References

The full embedded craft and skill catalogs live under `bible://references/*`
and `bible://skills/*`. The reference resources currently shipped are:

- `bible://references/anti-slop`
- `bible://references/voice-differentiation`
- `bible://references/swain-scene-sequel`
- `bible://references/mru-guide`

The skill catalog includes every SKILL.md in the repo, readable via
`bible://skills/<name>` (e.g. `bible://skills/scene-writer`).

## Shared convention: subagent orchestration

Several skills (authoring-supervisor, editor, continuity-editor,
revision-manager, scene-writer) carry a `## Subagent orchestration (Claude Code
/ grok)` section teaching the same pattern coding harnesses use: fan out
INDEPENDENT read-only research/verification to parallel subagents when the
harness supports them (Claude Code's Task/Agent tool, grok's subagents), and run
the same steps sequentially inline when it does not. The rule is uniform and
non-negotiable: subagents research and report; every state-mutating spindle tool
call stays in the main context, where the host decides and writes. Consult the
individual skill for its specific fan-out (per-scene review, per-dimension
editorial passes with adversarial verification, per-chapter continuity sweeps,
per-alternative revision assessments, pre-draft canon recon).
