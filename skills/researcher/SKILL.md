---
name: researcher
description: >
  Use when conducting factual research, capturing notes, adding sources, managing the project-local
  research library in SQLite, tagging materials, avoiding hallucination/fabrication, adhering to copyright
  bounds, or resolving AwaitResearch execution blocks.
---

# Researcher Skill

This skill guides agents in conducting factual research, capturing findings in the SQLite project-local library, establishing note/source provenance, and using research-aware drafting effectively while avoiding fabrication and respecting copyright.

## Overview of Research Library Tools

Spindle provides a suite of tools for managing project-local research databases:
1. `research_query`: Execute active searches against external search engines or Gemini models to find facts.
2. `research_add_source`: Register a bibliographic or structural source (books, articles, websites, archives) in the library.
3. `research_add_note`: Capture granular findings, direct quotes, and specific passages linked to a source.
4. `research_add_claim`: Extract explicit claims (atomic statements of fact, topic, confidence, time period, location) linked to a note and/or source.
5. `research_search`: Search the captured research library using keywords, tags, or semantic queries.
6. `research_pack_for_scene`: Automatically gather context-specific research material (sources, notes, claims) matching a scene summary, location, characters, and required tags.
7. `research_usage_for_scene`: View the durable project-scoped history of what research was actively used during the drafting of a scene. Always pass both `project_id` and `scene_id`.

---

## The Insertion Workflow & Provenance

When capturing new research in the project library, always preserve the chain of provenance. Follow this strict order of insertions:

1. **Add the Source first** (`research_add_source`):
   - Provide clean title, type (e.g. `website`, `book`, `paper`), author, URL or file path, accessed timestamp, and reliability rating (e.g. `high`, `medium`, `low`).
   - Add descriptive tags to classify the source (e.g., `nineteenth-century`, `steam-locomotive`, `london`).

2. **Add Notes linked to the Source** (`research_add_note`):
   - Link notes to the parent `source_id`.
   - Write clear, concise note content summarizing the relevant information.
   - Capture exact quotes in `quote` when wording is critical (e.g., technical definitions, historical quotes) and record the page/locator.
   - Carry over or refine tags (e.g., `steam-engine`, `coal-consumption`).

3. **Add Claims linked to the Note and/or Source** (`research_add_claim`):
   - Link claims to the parent `note_id` and/or `source_id` to establish solid provenance.
   - Write the claim as an atomic statement of fact.
   - Categorize by `topic`, `confidence`, `time_period`, and `location` if applicable.
   - Make claims traceable back to source facts.

> [!WARNING]
> Do NOT create orphaned claims. A claim without note or source provenance (i.e. missing both `note_id` and `source_id`, or referencing non-existent records) will trigger a **warning** during checkpoint/consistency checks (`research_accuracy` validator).

---

## Tagging Strategy & Avoiding Blocks

Spindle's automated drafting harness enforces safety and continuity constraints. If a scene plan specifies required research metadata, the harness may halt execution and return `NextAction::AwaitResearch` under the following conditions:
- `research_required` is true but the retrieved research pack is empty.
- `research_tags` are specified but none of the required tags match any tags in the retrieved research pack.

To resolve or prevent these execution blocks:
1. **Analyze scene plan required tags**: Inspect the scene seeds or snapshots for `research_tags` requirements.
2. **Apply consistent tags**: When calling `research_add_source`, `research_add_note`, or `research_add_claim`, ensure that you apply matching tags (case-insensitive) to the relevant research artifacts.
3. **Exploratory query alignment**: If a scene specifies an `explicit_query`, ensure your research items are discoverable under that query string by including relevant keywords in the titles, summaries, notes, or claims.

---

## Avoiding Fabrication

Fiction writing in Spindle requires anchoring world-building and real-world historical/scientific details in verified facts to prevent continuity erosion:
- **Stick to provenance**: When drafting a scene that incorporates research facts, use the research section formatted inside the LLM prompt. Do not invent details that contradict or go beyond the documented research.
- **Reference IDs**: Note which research IDs (e.g., `source:abc`, `note:xyz`, `claim:123`) influenced specific paragraphs or fact claims.
- **Run consistency checks**: Frequently run `check_consistency` with the `"research_accuracy"` check to find and resolve missing research references before finalizing chapters.

---

## Copyright & Fair Use Bounds

When collecting research, respect intellectual property boundaries:
- **Do not store entire copyrighted texts**: Do not copy full chapters, long articles, or entire copyrighted books into the `note` or `quote` fields.
- **Summarize and paraphrase**: Write your own summaries in the `summary` and `note` fields.
- **Keep quotes short**: Use the `quote` field only for brief, atomic excerpts (under 100-200 words) necessary for verifying technical or historical accuracy.
- **Record citations**: Keep the author, publisher, publication date, and source URL updated to maintain a clean bibliography.
