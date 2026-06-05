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
1. `research_query`: Execute active, structured research queries through route = "research" (with optional rating routing). Returns a structured JSON result containing a summary, sources, notes, claims, and warnings. If `store` is true, persists elements automatically.
2. `research_ingest_report`: Ingest, parse, and structure a text-based research report gathered externally into proper sources, notes, and claims in the SQLite project-local library.
3. `research_plan_for_scene`: Evaluate what research is missing for a scene before drafting, retrieve missing tags, suggest search queries, and verify if drafting should block with `await_research`.
4. `research_add_source`: Register a bibliographic or structural source (books, articles, websites, archives) in the library.
5. `research_add_note`: Capture granular findings, direct quotes, and specific passages linked to a source.
6. `research_add_claim`: Extract explicit claims (atomic statements of fact, topic, confidence, time period, location) linked to a note and/or source.
7. `research_search`: Search the captured research library using keywords, tags, or semantic queries.
8. `research_pack_for_scene`: Automatically gather context-specific research material (sources, notes, claims) matching a scene summary, location, characters, and required tags.
9. `research_usage_for_scene`: View the durable project-scoped history of what research was actively used during the drafting of a scene.

---

## Factual Research Execution Workflow

When a scene has research requirements or is blocked on `AwaitResearch`, follow this step-by-step workflow:

### 1. Plan and Inspect
Before executing new queries, inspect existing research first to avoid duplicate queries or parallel source creation:
- Call `research_search` to check if relevant details exist.
- Call `research_plan_for_scene` to verify which tags are missing, check if drafting is blocked (`await_research: true`), and fetch suggested query strings.

### 2. Execute Research Queries
- Invoke `research_query` with your structured factual question, specifying the `project_id`, `branch_id`, and `scene_id` so findings tie back to the scene cursor.
- Use `rating = "explicit"` for queries involving adult cultural or social topics.
- Set `store = true` to automatically parse and save findings, or use `store = false` to preview results before storing.

### 3. Ingest Existing Reports
- If you have pre-existing research or external notes, run `research_ingest_report` with a report title, text content, and classification tags. The service will parse the report and insert the structured elements into SQLite.

---

## Provenance and Tagging Strategy

Spindle enforces strict validation gates. Make sure to structure your data to satisfy these rules:

1. **Chain of Provenance**: All factual claims must be linked to a note, which must be linked to a source. Distinguish sourced claims from unsourced leads:
   - Sourced claims carry verified `note_id` / `source_id` references.
   - Unsourced leads are stored as raw leads with low/uncertain confidence when required sources are missing.
   - If the source policy is set to `require_sources`, do not store claims without provenance.
2. **Hallucination Guard**: Never invent or fabricate URL links, publishers, or publication dates. If a citation detail is unknown, leave it empty rather than making it up.
3. **Broad Category Coverage**: Spindle projects benefit from research covering a wide variety of dimensions:
   - Historical eras/periods, geographies, specific locales.
   - Socioeconomic norms, clothing styles, fashion history.
   - Technical processes, scientific facts, laws, and regulations.
   - Local food, dialect/slang, and contemporary culture.
4. **Tag Alignment**: Apply tags to sources, notes, and claims that match the scene's `research_tags` requirements. Tags are case-insensitive.

---

## Explicit/Adult Research Handling

Spindle handles explicit or adult-themed research strictly as factual context. It must never bleed into narrative scene drafting.

### Strict Safety Boundaries
- **Factual only**: Adult research may cover adult sexual culture, relationship norms, adult entertainment history, sexual health context, consent practices, social history, and setting-specific adult context.
- **No Story Prose**: Never generate narrative drafts, dialogues, or story prose inside research claims or summaries. Keep descriptions academic and factual.
- **Forbidden Content**: Never search for or record details involving coercion, abuse, minors, exploitation, or illegal sexual conduct.

### Routing & Tagging Rules
- **Explicit Route Routing**: Always invoke `research_query` with `rating = "explicit"` for adult/explicit queries. This routes the query to the explicit-authorized model route configured in the project-local `.spindle/config.toml` (or an explicit `SPINDLE_CONFIG` override). If no explicit research route is configured, Spindle fails closed instead of falling back to the general research route.
- **Enforced Boundary Language**: The query string must contain academic boundary terms (e.g. `factual`, `historical`, `sociological`, `consent`, `culture`, `norms`) to anchor the model response.
- **Provenance requirement**: Explicit research results must contain clear source/note provenance. Unverified claims are flagged with warnings.
- **Isolate Tags**: The system tags all explicit-rated research with `"explicit"` and `"adult"` tags.
- **Non-Explicit Scene Isolation**: Spindle isolates explicit research. Explicit-tagged research is omitted from general scene research packs unless the scene's rating or planned tags explicitly request `"explicit"` or `"adult"` materials.

---

## Copyright & Fair Use Bounds

- **Do not store entire copyrighted texts**: Do not copy full chapters or long copyrighted texts.
- **Summarize and paraphrase**: Write your own summaries in the `summary` and `note` fields.
- **Keep quotes short**: Keep quotes in `quote` to under 150 words.
- **Record citations**: Keep the author, publisher, publication date, and source URL updated to maintain a clean bibliography.
