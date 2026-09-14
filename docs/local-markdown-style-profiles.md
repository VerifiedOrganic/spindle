# Local Markdown Style Profiles

Status: implemented

## Summary

Spindle should support deriving a reusable style profile from one or more
user-provided local Markdown files. The user is responsible for how the files
arrived on disk and whether they have rights to use them. Spindle's job is to
read the local corpus, measure and summarize prose style, produce a structured
style card, and optionally apply that card to the project's existing style
contract surfaces.

This should be implemented as a local corpus feature, not as a web crawler.
Web import can be layered on later by any external tool that produces Markdown.

The output is not "write like this author." The output is an abstract,
project-local style profile: POV, tense, rhythm, paragraph density, dialogue
habits, pacing, exposition style, diction, scene shape, humor mechanics, and
concrete do/avoid guidance. Drafting tools then use this profile as style
context without copying source text.

## Why Local Markdown First

Local Markdown is the right first version because it avoids crawler-specific
complexity and makes the trust boundary clear.

- No site-specific extraction logic.
- No auth, paywall, robots, or rate-limit behavior inside Spindle.
- No need to infer provenance from URLs.
- Easy deterministic tests with fixture files.
- Works for web serials, personal drafts, exports, transcripts, and any other
  source once the user has converted them to Markdown.
- Aligns with existing external Markdown manuscript support in the import
  slicer.

## Goals

1. Accept one or more local Markdown files, or a directory of Markdown files,
   as a style corpus.
2. Normalize and chunk the corpus without persisting long source excerpts by
   default.
3. Compute deterministic style statistics before asking a model for synthesis.
4. Produce a structured style profile card that can be inspected, saved, and
   applied to a project.
5. Integrate with Spindle's existing project style contract:
   `ReaderContract.style_notes`, `style` world rules, and `NarratorVoice`.
6. Invalidate style-sensitive validator caches when an applied profile changes
   project style state.
7. Keep MCP thin: DTOs in `spindle-core`, service orchestration in
   `spindle-adapters`, tool wiring in `spindle-mcp`.

## Non-Goals

- Crawling web pages or downloading URLs.
- Verifying ownership, copyright status, license status, or provenance of the
  local files.
- Generating prose that imitates a named living author or reuses distinctive
  source phrasing.
- Storing the entire source corpus in SQLite as the default behavior.
- Building a general plagiarism detector.
- Replacing character-specific voice profiles. This feature describes the
  narrator/prose style for the whole project or branch.
- Supporting binary formats directly. EPUB, DOCX, PDF, and HTML can be handled
  by existing or future import/export tools that produce Markdown first.

## User Experience

### Create A Profile

The user points Spindle at files:

```json
{
  "project_id": "project:abc",
  "profile_name": "Fast close-POV serial",
  "source_paths": [
    "corpus/series/chapter-001.md",
    "corpus/series/chapter-002.md"
  ],
  "recursive": false,
  "apply": false
}
```

Or a directory:

```json
{
  "project_id": "project:abc",
  "profile_name": "Season 1 reference style",
  "source_paths": ["corpus/season-1"],
  "recursive": true,
  "include_globs": ["**/*.md", "**/*.markdown"],
  "apply": true
}
```

Spindle returns:

- profile id
- corpus summary
- deterministic stats
- generated style card
- warnings and skipped files
- whether the profile was applied
- which project style fields were changed

### Inspect A Profile

The user can inspect saved profiles through either a dynamic tool or a stable
resource:

- `list_style_profiles`
- `get_style_profile`
- `bible://projects/{project_id}/style-profiles`
- `bible://projects/{project_id}/style-profiles/{profile_id}`

### Apply A Profile

Applying a profile should update the project style contract, not silently change
drafting behavior through a hidden side channel.

MVP application should:

1. set `NarratorVoice` from the profile's narrator-facing fields
2. append or replace selected `ReaderContract.style_notes`
3. optionally create or update a `style` world rule named after the profile
4. invalidate style-related validator cache rows

The create tool can support `apply: true`, but application should also exist as
an explicit follow-up operation:

```json
{
  "project_id": "project:abc",
  "profile_id": "style_profile:def",
  "mode": "merge"
}
```

### Apply Modes (Merge vs ReplaceGeneratedStyleNotes)

Applying a profile supports two modes:

1. `merge`: Appends the new profile-generated notes to the project's `ReaderContract.style_notes`. It uses a case-insensitive check to avoid duplicate notes.
2. `replace_generated_style_notes`: Filters out any previously generated style notes (which are tagged with the stable marker `(Style Profile: profile_id/profile_name)`) and replaces them with the new ones. Crucially, all user-authored style notes (which do not have the marker) are fully preserved.

### Preview, Auditing, and Rollback

- **Preview**: Users can run `preview_apply_style_profile` to view proposed changes (including narrator voice changes, added/removed style notes, world rule creation/updates, and validation cache invalidations) without mutating any project state.
- **Auditing & Active State**: Every successful application is logged in the `style_profile_application` table. In addition, the project's `active_style_profile_id` is set to the applied profile.
- **Rollback**: Users can revert any application via the `rollback_style_profile_application` tool. The rollback restores the original narrator voice and style notes, conservatively reverts the style world rules, and appropriately clears or restores the project's `active_style_profile_id` to its previous state.

### Quality Scoring and Safeguards

During profile creation, Spindle computes a `StyleProfileQualityReport` containing:
- **Corpus Size**: Number of words and files.
- **Dialogue Coverage**: Proportion of dialogue words.
- **POV/Tense Confidence**: Confidence scores for POV and tense.
- **Chunk Consistency**: Variance across prose chunks.
- **Classification**: Classified as `Ready`, `Thin`, or `Inconsistent`.
- **Safeguard**: Auto-apply is blocked by default if quality is below a safe threshold (e.g. thin or inconsistent corpus), unless `force_apply = true` is passed.

### Drift Checking

Spindle supports checking drift for a scene or a full chapter against a style profile:
- **Chapter Drift**: Analyzes all scenes in a chapter and returns scene-scoped findings.
- **Metric Deltas**: Computes specific delta metrics (e.g. sentence length or dialogue ratio variance).
- **Summary Score**: Classified as `Aligned`, `Mild Drift`, or `Strong Drift`.
- **Default Active Comparison**: By default, checks against the project's currently active profile.
- **Non-mutating**: Drift checking is purely diagnostic and never rewrites manuscript prose.

### Style Revision Planning

To turn diagnostic style drift findings into actionable, previewable revision guidance, Spindle provides a non-mutating style revision planner:
- **Tool**: `plan_style_revision`
- **Goal**: Analyzes a target (`raw_text`, `scene_id`, or `chapter_id`) against the active profile (or a specified `profile_id`) and returns a structured plan to align the target's prose with the profile.
- **Difference from Drift Checking**: While drift checking diagnostics highlight *where* style diverges (e.g., specific metrics deviations or scanner rules triggered), the revision planner translates these findings into ordered instructions and (optionally) concrete rewrite examples to assist the user in editing the prose.
- **Deterministic and Non-Mutating**:
  - The revision planner strictly defaults to deterministic metrics and scanner heuristic findings.
  - It does not rewrite or persist target prose, and guarantees `mutates_prose: false`.
- **Optional LLM Rewrite Examples**:
  - If `include_rewrite_examples` is enabled, the service uses the `style_revise` model route to synthesize short original-to-revised prose snippets highlighting how to resolve style drift.
  - `max_suggestions` caps findings, ordered steps, and any generated rewrite examples.
  - To preserve privacy, raw source prose, target prose, and generated examples are processed purely in-memory and are never stored or persisted to the database.

#### Input / Output Examples

##### 1. Raw Text Target
**Input**:
```json
{
  "project_id": "project_1",
  "raw_text": "She went to the store. She bought some milk. She was happy.",
  "include_rewrite_examples": true
}
```

**Output**:
```json
{
  "project_id": "project_1",
  "profile_id": "style_profile_1",
  "target_summary": "Raw text target (word count: 12)",
  "drift_summary_score": "mild_drift",
  "findings": [
    {
      "severity": "warning",
      "category": "sentence_length",
      "evidence_summary": "Draft average sentence length is 4.0 words, but the style profile average is 15.0 words.",
      "suggested_correction": "Combine short, choppy sentences to flow more naturally."
    }
  ],
  "steps": [
    {
      "order": 1,
      "finding_category": "sentence_length",
      "instructions": "Combine short, choppy sentences to flow more naturally.",
      "target_scope": "raw_text",
      "confidence": "high"
    }
  ],
  "rewrite_examples": [
    {
      "original_prose": "She went to the store. She bought some milk. She was happy.",
      "revised_prose": "Walking down the dusty aisle, she grabbed the cool glass bottle of milk, a small smile softening her face.",
      "explanation": "Combined short, choppy sentences into a more fluid narrative with sensory details to match the style profile."
    }
  ],
  "mutates_prose": false
}
```

##### 2. Scene Target
**Input**:
```json
{
  "project_id": "project_1",
  "scene_id": "scene_abc123",
  "metrics_only": true
}
```

**Output**:
```json
{
  "project_id": "project_1",
  "profile_id": "style_profile_1",
  "target_summary": "Scene: scene_abc123 (word count: 320)",
  "drift_summary_score": "aligned",
  "findings": [],
  "steps": [],
  "rewrite_examples": null,
  "mutates_prose": false
}
```

##### 3. Chapter Target
**Input**:
```json
{
  "project_id": "project_1",
  "chapter_id": "chapter_xyz789"
}
```

**Output**:
```json
{
  "project_id": "project_1",
  "profile_id": "style_profile_1",
  "target_summary": "Chapter: chapter_xyz789 (Chapter 1, 2 scenes, total word count: 850)",
  "drift_summary_score": "strong_drift",
  "findings": [
    {
      "severity": "warning",
      "category": "sentence_length",
      "evidence_summary": "Draft average sentence length is 24.5 words, but the style profile average is 12.0 words.",
      "suggested_correction": "Break up long sentences into shorter, punchier clauses.",
      "scene_id": "scene_1"
    }
  ],
  "steps": [
    {
      "order": 1,
      "finding_category": "sentence_length",
      "instructions": "Break up long sentences into shorter, punchier clauses.",
      "target_scope": "scene",
      "target_id": "scene_1",
      "confidence": "high"
    }
  ],
  "rewrite_examples": null,
  "mutates_prose": false
}
```

### Profile Comparison and Management

- **Comparison**: Use `compare_style_profiles` to compare two profiles in the same project, showing metric deltas and guidance differences.
- **Archiving**: Profiles can be archived using `archive_style_profile` (setting `archived_at` timestamp). Archiving an active profile is blocked unless `force = true` is passed. Archived profiles are omitted from active/default selections, but their historical audit logs remain preserved.

### Source Refresh and Versioning

Spindle provides a workflow to detect changes in a style profile's local markdown source corpus, preview the style profile differences, and apply/promote the updated profile version.

#### 1. Refresh Lifecycle
The refresh workflow consists of three main stages:
- **Staleness Check (`check_style_profile_sources`)**:
  - Compares the current local files (using canonical paths, sizes, modified timestamps, and content hashes) against the saved metadata-only fingerprints of the source files in the style profile.
  - Reports if the profile is stale (`stale: true`), what files were added, removed, or changed, and if a refresh is possible (`can_refresh`).
  - By default, rejects archived profiles unless `include_archived` is explicitly set to true.
- **Refresh Preview (`preview_refresh_style_profile`)**:
  - Re-reads the current files matching the profile's original source policy in memory.
  - Recomputes metrics and regenerates guidance (via the `style_analyze` model route, unless `metrics_only` is true) to construct a candidate profile.
  - Compares the candidate profile against the current profile to report metric deltas, quality changes, and warnings without mutating any project state or persisting anything.
- **Refresh Apply (`refresh_style_profile`)**:
  - Re-reads, regenerates, and writes a new profile version.
  - Automatically handles quality gating, active profile promotion, audit logging, and cache invalidation.

#### 2. Versioning Behavior
Instead of overwriting the existing profile, a refresh operation creates a brand new profile record to keep a clean history.
- The new profile is linked to the previous version via:
  - `parent_profile_id`: References the profile ID from which this version was refreshed.
  - `version_number`: A 1-indexed counter incremented sequentially for each version.
  - `refreshed_from_profile_id` / `refreshed_at`: Timestamps and linkages mapping the lineage of refreshes.
- Parent profiles that are archived remain fully intact as valid historical references for audits and rollbacks.

#### 3. Privacy Guarantees
To maintain security and data ownership boundaries:
- **Metadata-only Fingerprints**: Spindle stores canonical safe paths, file sizes, modified timestamps, content hashes, glob metadata, and captured timestamps in the `style_profile_source` table.
- **No Persisted Prose**: The raw source prose is processed strictly in-memory during checks, previews, and refreshes. At no point is raw prose persisted in database columns, refresh records, audits, or MCP resource outputs.
- **Path Traversal Protection**: Existing safe directory boundary checks are strictly enforced. Paths residing outside of the allowed workspace roots will cause the check or refresh to abort immediately.
- **MCP Resource Constraints**: Resources such as `bible://projects/{project_id}/style-profiles/{profile_id}/sources` and `bible://projects/{project_id}/style-profiles/{profile_id}/refresh-preview` expose only metadata (e.g. file lists, metric deltas) and never any raw text content.

#### 4. When to Refresh vs. Create a New Unrelated Profile
- **Refresh**: Use when editing, adding, or deleting chapters within the same target manuscript/corpus to keep the profile updated with the project's current evolution. Refresh preserves the parent-child lineage.
- **Create New Profile**: Use when targeting a completely different reference manuscript, exploring a distinct genre/tone, or establishing a new style line that should not share version history or automatically replace the currently active profile.

#### 5. Active Profile Promotion
- If `apply_after_refresh` is true and the parent profile was the project's currently active profile, the project's `active_style_profile_id` is automatically promoted/moved to the newly created profile version.
- If `apply_after_refresh` is false, the active profile remains unchanged, but the new version is still stored and can be manually applied later.
- If auto-applying, the profile must pass the standard quality gate (having a status of `Ready` in its quality report) unless bypassed with `force_apply: true`.

### Style Learning from Edits

When you re-draft an agent-written scene by hand, that edit is signal: it shows how you actually want the prose to read. Spindle can capture those before/after pairs and fold them into the **existing** style-refresh flow — no new tools, and nothing enters a profile until you run refresh yourself (the same explicit action you already take).

- **Opt-in per project (`style_learning`).** Disabled by default (the flag is `NULL`). Enable it by setting `style_learning` to `1` through `update_entity` on the project (e.g. `update_entity { entity_id: "project:…", changes: { "style_learning": 1 } }`). Set it back to `0` to stop capturing.
- **Capture rule.** When style learning is enabled and you re-save a scene the agent drafted with **different** prose (byte compare after trim), Spindle stores the agent draft and your edit as a *pending style-edit candidate*. Only an operator edit over an agent draft is captured — agent-over-agent re-saves (the revise loop) and operator-over-operator edits are not. A second operator edit of the same scene **replaces** the pending candidate (one per scene; the latest edit is the signal). The captured signal is the scene's `draft_origin`: agent authorship is recognized from a `draft_origin` starting with `agent:` (recorded on the agent draft path). A scene larger than 60,000 characters is skipped (a degenerate case that would only bloat the corpus).
- **The preview→refresh review gate (I4-consistent).** `preview_refresh_style_profile` gains a `style_edit_candidates` section listing the pending candidates by scene reference with a **bounded, prose-free diff summary** (character counts and delta — never the full prose). `refresh_style_profile` then feeds each included candidate's edited prose into the refresh corpus as a positive example alongside the profile's file sources, and marks the candidates `consumed`. Because refresh is an explicit operator action, nothing reaches a profile without your review — the preview→refresh pair *is* the review gate.
- **Dismissal.** Pass `dismiss_candidate_ids: ["style_edit_candidate:…"]` to `refresh_style_profile` to drop candidates instead of consuming them; they are flipped to `dismissed` and never fed.
- **Explicit-content withholding (source-side rating discipline).** An explicit-rated candidate is **withheld** from the refresh sources unless the `style_analyze` route's resolved agent declares the `explicit` rating. Withheld candidates stay pending, and the preview notes them (e.g. *"1 explicit candidate withheld: style route not explicit-cleared"*). Style routes are non-prose-bearing, so this guard protects at the **source** — an explicit example never enters an analyzer that never declared explicit coverage — mirroring the prose-bearing dispatch gate. Point `style_analyze` at an explicit-cleared agent to include them.

### Privacy and Data Minimization

- Creating a style profile sends capped chunks of source prose to the configured `style_analyze` model route.
- `source_sample_word_budget` controls the maximum source-sample words sent to that route.
- A `metrics_only` option is supported: when enabled, the analysis prompt excludes all raw prose chunks and relies entirely on deterministic statistics and local observations.
- **Privacy Implications**:
  - Routing `style_analyze` to an external provider transmits source chunks. If privacy is paramount, configure a local agent (e.g. local llama/ollama) or use `metrics_only` mode.
  - Spindle does not persist raw source prose in style profiles, application audit logs, or drift findings.

## Product Language

Use neutral corpus/profile language in public fields and prompts.

Use:

- "style profile"
- "style card"
- "derived from user-provided local Markdown"
- "abstract prose guidance"
- "do/avoid rules"

Avoid:

- "clone"
- "copy"
- "write exactly like"
- "author imitation"

## Data Model

### `StyleProfileCard`

Add a public DTO in `spindle-core/src/models.rs` or a new
`spindle-core/src/style/profile.rs` module re-exported from core.

Proposed shape:

```rust
pub struct StyleProfileCard {
    pub profile_id: String,
    pub project_id: String,
    pub name: String,
    pub status: StyleProfileStatus,
    pub created_at: String,
    pub updated_at: String,
    pub corpus: StyleCorpusSummary,
    pub metrics: StyleCorpusMetrics,
    pub guidance: StyleProfileGuidance,
    pub source_policy: StyleProfileSourcePolicy,
    pub model_receipt: Option<StyleProfileModelReceipt>,
}
```

### `StyleCorpusSummary`

```rust
pub struct StyleCorpusSummary {
    pub source_count: usize,
    pub analyzed_source_count: usize,
    pub skipped_source_count: usize,
    pub total_words: usize,
    pub total_characters: usize,
    pub chunk_count: usize,
    pub source_refs: Vec<StyleSourceRef>,
    pub warnings: Vec<String>,
}

pub struct StyleSourceRef {
    pub display_name: String,
    pub canonical_path: String,
    pub sha256: String,
    pub word_count: usize,
    pub included: bool,
    pub skip_reason: Option<String>,
}
```

`canonical_path` should be stored for reproducibility, but source text should
not be stored by default.

### `StyleCorpusMetrics`

The deterministic metrics should be computed in Rust and included in the model
prompt. They should also be persisted so the profile remains useful even when a
model call fails.

```rust
pub struct StyleCorpusMetrics {
    pub average_sentence_words: f64,
    pub median_sentence_words: f64,
    pub p90_sentence_words: f64,
    pub average_paragraph_words: f64,
    pub median_paragraph_words: f64,
    pub dialogue_line_ratio: f64,
    pub dialogue_word_ratio: f64,
    pub question_mark_rate_per_1k_words: f64,
    pub exclamation_rate_per_1k_words: f64,
    pub semicolon_rate_per_1k_words: f64,
    pub em_dash_rate_per_1k_words: f64,
    pub ellipsis_rate_per_1k_words: f64,
    pub first_person_pronoun_rate_per_1k_words: f64,
    pub third_person_pronoun_rate_per_1k_words: f64,
    pub top_functional_markers: Vec<String>,
}
```

These are signals, not absolute judgements. The generated guidance should treat
them as evidence.

### `StyleProfileGuidance`

```rust
pub struct StyleProfileGuidance {
    pub summary: String,
    pub pov: Option<String>,
    pub tense: Option<String>,
    pub narrator_distance: Option<String>,
    pub narrator_voice: NarratorVoice,
    pub pacing: Vec<String>,
    pub paragraphing: Vec<String>,
    pub sentence_rhythm: Vec<String>,
    pub diction: Vec<String>,
    pub dialogue: Vec<String>,
    pub exposition: Vec<String>,
    pub interiority: Vec<String>,
    pub humor_or_tension: Vec<String>,
    pub scene_structure: Vec<String>,
    pub do_rules: Vec<String>,
    pub avoid_rules: Vec<String>,
    pub prompt_snippet: String,
}
```

`prompt_snippet` is the compact form inserted into drafting/review context.
It must be abstract guidance only. It must not contain source passages longer
than the configured excerpt limit.

### `StyleProfileSourcePolicy`

```rust
pub struct StyleProfileSourcePolicy {
    pub local_user_provided: bool,
    pub source_text_persisted: bool,
    pub max_excerpt_words: usize,
    pub allowed_roots: Vec<String>,
}
```

MVP default:

- `local_user_provided = true`
- `source_text_persisted = false`
- `max_excerpt_words = 0`

### SQLite Tables

Add a migration in `crates/spindle-adapters/migrations`.

Recommended minimal tables:

```sql
CREATE TABLE style_profile (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    card_json TEXT NOT NULL,
    metrics_json TEXT NOT NULL,
    guidance_json TEXT NOT NULL,
    source_policy_json TEXT NOT NULL,
    model_receipt_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_style_profile_project
    ON style_profile(project_id, created_at);

CREATE TABLE style_profile_source (
    id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    display_name TEXT NOT NULL,
    canonical_path TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    word_count INTEGER NOT NULL,
    included INTEGER NOT NULL,
    skip_reason TEXT,
    source_order INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_style_profile_source_profile
    ON style_profile_source(profile_id, source_order);
```

Do not add a source-text table in the MVP. If later needed for audit/debug,
make it opt-in and store excerpts only.

## Tool Surface

### `create_style_profile_from_markdown`

Category: compute/write

Input:

```rust
pub struct CreateStyleProfileFromMarkdownInput {
    pub project_id: String,
    pub profile_name: String,
    pub source_paths: Vec<String>,
    pub recursive: Option<bool>,
    pub include_globs: Option<Vec<String>>,
    pub exclude_globs: Option<Vec<String>>,
    pub max_files: Option<usize>,
    pub max_bytes_per_file: Option<usize>,
    pub max_total_words: Option<usize>,
    pub apply: Option<bool>,
    pub application_mode: Option<StyleProfileApplyMode>,
}
```

Output:

```rust
pub struct CreateStyleProfileFromMarkdownOutput {
    pub profile: StyleProfileCard,
    pub applied: bool,
    pub application: Option<ApplyStyleProfileOutput>,
}
```

Behavior:

1. Validate project exists.
2. Resolve and validate local paths.
3. Collect Markdown files deterministically by canonical path.
4. Normalize and chunk the text.
5. Compute deterministic metrics.
6. Run model synthesis through `style_analyze`.
7. Validate model JSON and repair once if needed.
8. Persist the profile and source metadata.
9. If requested, apply the profile.

### `list_style_profiles`

Input:

```rust
pub struct ListStyleProfilesInput {
    pub project_id: String,
}
```

Output:

```rust
pub struct ListStyleProfilesOutput {
    pub profiles: Vec<StyleProfileCard>,
}
```

### `get_style_profile`

Input:

```rust
pub struct GetStyleProfileInput {
    pub project_id: String,
    pub profile_id: String,
}
```

Output:

```rust
pub struct GetStyleProfileOutput {
    pub profile: StyleProfileCard,
}
```

### `apply_style_profile`

Input:

```rust
pub enum StyleProfileApplyMode {
    Merge,
    ReplaceGeneratedStyleNotes,
}

pub struct ApplyStyleProfileInput {
    pub project_id: String,
    pub profile_id: String,
    pub mode: StyleProfileApplyMode,
}
```

Output:

```rust
pub struct ApplyStyleProfileOutput {
    pub project_id: String,
    pub profile_id: String,
    pub narrator_voice: NarratorVoice,
    pub reader_contract_style_notes: Vec<String>,
    pub style_rule_id: Option<String>,
    pub invalidated_validator_findings: usize,
}
```

MVP should support `Merge`. `ReplaceGeneratedStyleNotes` can be implemented
once generated notes are tagged well enough to avoid overwriting user-authored
style notes.

## Resource Surface

Add resources:

- `bible://projects/{id}/style-profiles`
- `bible://projects/{id}/style-profiles/{profile_id}`

The list resource should omit bulky model receipts and source refs beyond a
small summary. The detail resource can return the full card.

## Path And File Safety

This feature reads local files, so path handling must be explicit.

Rules:

1. Resolve every input path to a canonical path before reading.
2. Follow symlinks only after canonicalizing and checking the final target.
3. By default, allow files under the project workspace root and the repository
   data directory.
4. Optionally allow additional roots from config later:
   `style_corpus_roots = ["/path/to/corpus"]`.
5. Reject device files, sockets, FIFOs, and files larger than
   `max_bytes_per_file`.
6. Reject non-UTF-8 files in MVP.
7. Ignore hidden directories by default unless explicitly passed as a file.
8. Sort files by canonical path for deterministic output.

The implementation should not try to determine where the text originally came
from. The explicit contract is "user-provided local Markdown."

## Markdown Normalization

Create a small normalizer in `spindle-adapters`, preferably near the existing
import slicer code.

MVP normalization:

1. Strip UTF-8 BOM.
2. Strip YAML frontmatter delimited by `---` at the start of the file.
3. Strip HTML comments.
4. Strip fenced code blocks.
5. Convert ATX headings to blank-line separators while keeping heading text as
   optional chunk labels.
6. Treat `***`, `---`, and `_ _ _` separator lines as scene breaks.
7. Collapse runs of more than three blank lines to two.
8. Keep quoted dialogue and prose punctuation intact.

Do not over-normalize. The point is to measure prose rhythm, so punctuation and
paragraph boundaries matter.

## Chunking

Chunk after normalization. Reuse existing manuscript slicer behavior where it
fits, but style analysis should not require valid chapter/scene structure.

MVP chunking rules:

- Prefer heading and scene-break boundaries.
- Target 1,200 to 2,500 words per chunk.
- Keep paragraphs intact.
- Drop chunks below 150 words unless the entire corpus is small.
- Cap total prompt payload with `max_total_words`.
- Preserve source ref, byte offsets, word count, and label in memory.

Only metrics and source metadata are persisted by default.

## Deterministic Metrics

Metrics should run before model synthesis and should be covered by unit tests.

Required MVP metrics:

- file count, included count, skipped count
- word count
- paragraph count
- sentence count
- average, median, and p90 sentence length
- average and median paragraph length
- dialogue line ratio
- dialogue word ratio
- punctuation rates per 1,000 words:
  - question marks
  - exclamation marks
  - semicolons
  - em dashes
  - ellipses
- first-person and third-person pronoun rates
- top repeated non-content markers, excluding ordinary stopwords

Approximate is acceptable if documented. These numbers are there to ground the
LLM, not to be a perfect stylometry engine.

## Model Route

Add a dedicated model route:

```toml
[[routing]]
route = "style_analyze"
agent = "local-http"
```

Default local route behavior should exist so tests and offline usage do not
fail. The route should use a low temperature, structured-output prompt.

Files to update:

- `crates/spindle-adapters/src/ai.rs`
- `crates/spindle-adapters/src/agent_config.rs`
- `docs/spindle-agent-config.md`
- config resource tests that assert route names

If this is too much for the first implementation PR, temporarily route through
`import_synthesize`, but keep the service code behind a constant named
`STYLE_ANALYZE_ROUTE` so the dedicated route can be added without touching
business logic.

## Model Prompt Contract

The style synthesis prompt must require JSON and must forbid source copying.

Prompt requirements:

- State that the corpus is user-provided local Markdown.
- Ask for abstract prose guidance, not imitation.
- Include deterministic metrics.
- Include short anonymized chunk summaries if available.
- Forbid quoting source passages except very short examples when
  `max_excerpt_words > 0`.
- Require every guidance claim to be supported by either a metric or a chunk
  observation.
- Require uncertainty fields when the corpus is thin or inconsistent.
- Require output that maps cleanly to `StyleProfileGuidance`.

The service should parse JSON strictly. If parsing fails, run one repair prompt
using the same route. If repair fails, persist a profile with metrics and
`status = "needs_review"` rather than dropping all work.

## Application Semantics

Applying a profile is a normal project mutation.

MVP mapping:

- `guidance.narrator_voice` -> `SetNarratorVoiceInput`
- compact summary and strongest do/avoid rules -> `ReaderContract.style_notes`
- `guidance.prompt_snippet` -> optional `style` world rule

Application must:

1. preserve user-authored style notes in merge mode
2. avoid adding duplicate generated notes
3. record which profile was applied
4. invalidate validator caches whose context hash included style state
5. return a clear diff of changed style fields

The profile itself should remain saved even if application fails.

## Drafting And Review Integration

The existing `StyleDirective::assemble` path is the right integration point.
Do not create a second style context path in drafting.

Once a profile is applied:

1. scene context rendering should show the applied style guidance through the
   existing style directive section
2. `save_scene_draft` and `revise_scene` should continue to use the existing
   style compliance scanner and review surfaces
3. dual-persona review should see the profile through the target reader style
   contract

The style profile source corpus should not be injected into drafting prompts.
Only the abstract profile guidance should be injected.

## Tests

### Unit Tests

Add focused tests for:

- Markdown frontmatter stripping
- code fence stripping
- scene separator recognition
- chunk sizing and paragraph preservation
- sentence and paragraph metrics
- dialogue ratio calculation
- path canonicalization and root rejection
- JSON parsing and repair fallback

### Service Tests

Add tests under `crates/spindle-adapters/tests` or service tests for:

- create profile from two Markdown fixture files
- skipped non-Markdown and oversized files
- persisted source refs contain hashes but not source text
- list/get profile round trip
- apply profile updates narrator voice and style notes
- style cache invalidation is triggered on apply
- thin corpus returns warnings

### MCP Tests

Add tests for:

- tool schema generation
- calling create/list/get/apply through the tool router
- resource read for list and detail
- tool profile exposure if any restricted profiles need the new tools

### Regression Fixtures

Add small files under `testdata/style/`:

- `fast-serial-chapter-1.md`
- `dialogue-heavy-scene.md`
- `frontmatter-and-code.md`
- `thin-corpus.md`

Keep fixtures original and purpose-built for the test suite.

## Rollout Plan

### Phase 1: Profile MVP

- Core DTOs.
- SQLite migration and repository methods.
- Local Markdown collection, normalization, chunking, metrics.
- `create_style_profile_from_markdown`.
- Deterministic local model fallback.
- Persist profile and source metadata.
- Unit and service tests.

### Phase 2: Apply And Context Integration

- `apply_style_profile`.
- `list_style_profiles` and `get_style_profile`.
- Style-profile resources.
- Merge profile guidance into existing `StyleDirective` sources.
- Validator cache invalidation.
- MCP tests.

### Phase 3: Route And Config Polish

- Add `style_analyze` route to config docs and route defaults.
- Add model route resource coverage.
- Tune prompt and repair behavior.
- Add CLI/harness examples if needed.

### Phase 4: Optional Enhancements

- Multiple active profiles with weights.
- Compare two style profiles.
- Profile drift report for a scene or chapter.
- Opt-in excerpt retention for local audit.
- External importer command that converts URLs or documents to Markdown before
  calling this feature.

## Error Handling

Use specific, user-actionable errors:

- `source path is outside allowed roots`
- `source path does not exist`
- `source path is not a regular UTF-8 Markdown file`
- `source file exceeds max_bytes_per_file`
- `no analyzable Markdown files found`
- `corpus is too small for confident style analysis`
- `style model returned invalid JSON`
- `style profile was saved but could not be applied`

Warnings should not fail the whole operation when at least one file is
analyzable.

## Observability

The tool output should include:

- analyzed file count
- skipped file count
- total words
- chunk count
- model adapter and model name
- whether model output was repaired
- whether source text was persisted
- any warnings

## Style Revision Patch Workflow (Preview and Apply)

To make style revision planning operational, Spindle supports a non-mutating patch preview and an explicit apply workflow. This enables review of proposed prose changes before committing them to the manuscript.

### Comparison: Style Workflow Features

| Feature | Mutates Manuscript? | Scope | Output | Persisted? | Purpose |
|---|---|---|---|---|---|
| **Drift Check** | No | Diagnostic (Scene or Chapter) | Metric deviations and scanner heuristics | No | Identifies *where* and *how much* style drifts from the active profile. |
| **Revision Plan** | No | Diagnostic (Scene, Chapter, or Raw Text) | Ordered steps and optional rewrite examples | No | Provides edit instructions to correct style drift. |
| **Patch Preview** | No | Scene or Chapter | Structured hunks, unified diffs, original/revised word counts, and proposed prose | No | Generates and previews the full style-aligned text draft without changing the manuscript. |
| **Patch Evaluation** | No | Proposed Scenes | Aggregate and per-scene metrics comparison, improvement scores, status, and safety risks | No | Assesses proposed revisions for style improvement, warnings, and safety risks before applying. |
| **Patch Apply** | **Yes** | Proposed Scenes | Applied scene IDs, audit ID | Yes (Saves scene drafts, records audit row) | Commits proposed text changes to the manuscript through the standard save pipeline. |
| **Patch Rollback** | **Yes** | Prior Audit ID | Restored scene IDs, rollback timestamp | Yes (Restores prior scene versions, updates audit) | Reverts the changes made by a specific patch application using historical scene versions. |

### Style Revision Patch Evaluation

To build trust and verify the quality of proposed edits before committing them, Spindle provides a non-mutating patch evaluation workflow. It analyzes the proposed revised prose against the original text and style profile to ensure the revision actually improves style alignment without introducing structural or semantic risks.

#### Key Validation Rules

Before evaluation takes place, the following guards are enforced:
1. **Ownership & Active Branch**: Verifies that all target scenes belong to the specified project and exist on the project's active branch.
2. **Profile Status**: Rejects evaluation if the style profile is archived.
3. **Stale Protection**: Validates that each patch's `before_hash` matches the current scene text hash in the database.
4. **Integrity Check**: Verifies that each patch's `after_hash` matches the calculated SHA256 of its proposed `revised_text`.

#### Analysis & Risk Detection Heuristics

The evaluation performs the following checks for each scene:
- **Style Metrics Delta**: Compares sentence length, paragraph length, and dialogue ratio distributions before and after revision.
- **Drift Warnings**: Runs the style drift scanner on both the original and revised text.
- **Improvement Score**: Computes a numeric improvement score based on the reduction of warnings and errors.
- **Status Classification**: Classifies each scene and the overall patch as:
  - `improved`: Warnings/errors decreased and improvement score is positive.
  - `neutral`: Warnings/errors remained the same.
  - `regressed`: Warnings/errors increased, or risks triggered regression.
- **Risk Identification**: Detects and reports critical risks:
  - *Increased Style Drift*: Warnings/errors increased.
  - *Large Word-Count Swings*: A word-count change of 30% or more.
  - *Empty/Near-Empty Prose*: Revised prose that is empty or contains fewer than 5 words or 15 characters.
  - *Content Rating / Safety Violations*: Major tone or rating changes.
  - *Validator Preflight Errors*: Optionally runs style-sensitive validators in-memory (using the phase-four registry with in-memory scene snapshots) to catch semantic/rule violations without persisting any draft.

#### Integration Points

1. **Preview Integration**: The `preview_style_revision_patch` input supports an optional `run_evaluation` flag. If `true`, the preview output embeds a complete `EvaluateStyleRevisionPatchOutput`.
2. **Apply Gating Integration**: The `apply_style_revision_patch` input accepts `require_positive_evaluation: Option<bool>` and `minimum_improvement_score: Option<f64>`. When enabled:
   - It runs the evaluation before applying.
   - It rejects the apply with an error if the overall patch status is `regressed` or if the score falls below the minimum threshold.
   - Default behavior preserves explicit applying of patches without evaluation.
3. **MCP Interface**: Exposed via the `evaluate_style_revision_patch` MCP tool.

### Privacy and Data Security

Prose sent to the model via the `style_revise` route is processed purely in-memory and **never persisted** during preview.
The database audit logs recorded on apply (in `style_revision_patch_audit` table) store only metadata, including:
- Profile ID
- Target Scene/Chapter IDs
- Pre-apply and post-apply text hashes
- Route completion receipt (without prompt or output content)
- Rollback status metadata (`rolled_back_at`, `rollback_status`)

Crucially, **no source prose, target prose, or generated drafts** are stored in the audit trail database rows. This ensures complete privacy for draft content.

### Patch Audit & Rollback Lifecycle

Every patch application writes a metadata-only audit row to the database. Spindle supports rolling back applied patches to restore the manuscript back to its prior state.

- **Listing Audits**: The `list_style_revision_patch_audits` service method and MCP tool, and the `bible://projects/{project_id}/style-revision-patch-audits` resource list the patch audits, which include fields like `rolled_back_at` and `rollback_status`.
- **Rollback Process**: Rolling back a patch (via the `rollback_style_revision_patch` tool/service method) performs the following:
  1. **Validation**: Loads the audit row and ensures it exists and has not been rolled back yet. It preserves project and branch boundaries by validating that all target scenes belong to the project and its active branch.
  2. **Stale Protection**: Rejects rollback if the current scene text hash has changed since the patch was applied (protecting against stale rollbacks).
  3. **Metadata-only Restoration**: Restores each scene to its matching previous prose version. Since audit rows contain only metadata (to preserve privacy), the rollback relies on the scene version history (`scene_version` table). It searches for a version whose text hash matches the audit's `before_hash` and restores it using the standard scene save/update pipeline.
  4. **Audit Update**: Updates the audit row's `rollback_status` to `"rolled_back"` and records `rolled_back_at`.
  5. **Cache Invalidation**: Invalidates the style-sensitive validator caches (`StyleCompliance` and `WorldRuleSemanticDrift`).

## Open Questions

1. Should style profiles be project-wide only in the MVP, or branch-scoped from
   day one?
2. Should applying a profile update the reader contract directly, or should
   reader-contract rendering include applied profile guidance as a separate
   source?
3. Should `ReplaceGeneratedStyleNotes` wait until style notes have stable
   provenance tags?
4. Should additional allowed corpus roots live in `.spindle/config.toml`, a
   per-project DB setting, or both?

Recommended MVP answers:

1. Project-wide only.
2. Update existing style surfaces so all current drafting/review paths work.
3. Wait.
4. Config later; start with workspace/data-dir roots.

## Implementation Checklist

1. Add core DTOs for profile create/list/get/apply.
2. Add style profile storage migration.
3. Add repository methods for profile/source insert and lookup.
4. Add Markdown corpus collector with canonical path checks.
5. Add normalizer, chunker, and metrics modules.
6. Add style synthesis prompt and parser.
7. Add service methods.
8. Wire MCP tools.
9. Add resources.
10. Add route/config docs for `style_analyze`.
11. Add tests and fixtures.
12. Update public docs and examples.
