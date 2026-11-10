//! SQLite-backed Spindle service layer.
//!
//! Phase 6 entry point that mirrors [`crate::services::SpindleService`] but
//! against the new [`super::Repository`]. Each method is the SQLite
//! counterpart to a service method in `services/mod.rs`, taking the same
//! `*Input` / `*Output` shapes from `spindle_core::models` so MCP callers
//! can swap by changing only one type binding.
//!
//! This file grows incrementally during Phase 6 — one service method at a
//! time, with each translation independently testable. When the surface is
//! complete the old `crate::services::SpindleService` is deleted and this
//! becomes the canonical service layer.
//!
//! ## Coverage map (initial)
//!
//! Methods land here in MCP-priority order: integration_tests.rs exercises a
//! fairly small subset of tools, and getting those green is the Phase 6 exit
//! criterion. Each method gets a corresponding `#[cfg(test)] mod tests` entry
//! that calls it through a real MCP-shaped input.

use std::time::Duration;

use anyhow::{Context, Result};
use chrono::TimeZone;
use spindle_core::models::{
    AnnotateSceneBeatsInput, AnnotateSceneBeatsOutput, AnnotatedBeat, ArchiveEntityInput,
    ArchiveEntityOutput, BatchCreateMotifsInput, BatchCreateMotifsOutput,
    BatchCreateNarrativePromisesInput, BatchCreateNarrativePromisesOutput, BatchCreateTermsInput,
    BatchCreateTermsOutput, BatchSetCharacterVoiceProfilesInput,
    BatchSetCharacterVoiceProfilesOutput, BindCanonicalFactToSceneInput,
    BindCanonicalFactToSceneOutput, BookOutline, CanonicalFactScope, ChapterOutline,
    ChapterOutlineBeat, CharacterStatePatch, CharacterVoiceProfileData, CommitCharacterStateInput,
    CommitCharacterStateOutput, ConfigureAgentsInput, ConfigureAgentsOutput, CreateBookInput,
    CreateBookOutput, CreateBranchInput, CreateBranchOutput, CreateChapterInput,
    CreateChapterOutput, CreateCharacterArcInput, CreateCharacterArcOutput, CreateCharacterInput,
    CreateCharacterOutput, CreateConflictInput, CreateConflictOutput, CreateEconomyInput,
    CreateEconomyOutput, CreateFactionInput, CreateFactionOutput, CreateFutureKnowledgeInput,
    CreateFutureKnowledgeOutput, CreateLocationInput, CreateLocationOutput, CreateMotifInput,
    CreateMotifOutput, CreateMotifsInput, CreateMotifsOutput, CreateNarrativePromiseInput,
    CreateNarrativePromiseOutput, CreateNarrativePromisesInput, CreateNarrativePromisesOutput,
    CreatePacingConfigInput, CreatePacingConfigOutput, CreatePacingCurveInput,
    CreatePacingCurveOutput, CreatePlotLineInput, CreatePlotLineOutput, CreateProjectInput,
    CreateProjectOutput, CreateRelationshipInput, CreateRelationshipOutput, CreateReligionInput,
    CreateReligionOutput, CreateSavePointInput, CreateSavePointOutput, CreateSystemOverlayInput,
    CreateSystemOverlayOutput, CreateTemporalInterventionInput, CreateTemporalInterventionOutput,
    CreateTermInput, CreateTermOutput, CreateTermsInput, CreateTermsOutput, CreateThemeInput,
    CreateThemeOutput, CreateTimelineEventInput, CreateTimelineEventOutput, CreateWorldRuleInput,
    CreateWorldRuleOutput, DeleteSceneInput, DeleteSceneOutput, EntityResolutionConfidence,
    FindEntityInput, FindEntityMatch, FindEntityOutput, FindScenesReferencingInput,
    FindScenesReferencingOutput, GetSceneDeleteImpactInput, GetSceneDeleteImpactOutput,
    GetSceneMoveImpactInput, GetSceneMoveImpactOutput, ListAgentsOutput, ListBookChaptersInput,
    ListBookChaptersOutput, ListChapterScenesInput, ListChapterScenesOutput, ListProjectsOutput,
    ListRevisionMarkersInput, ListRevisionMarkersOutput, ListSceneVersionsInput,
    ListSceneVersionsOutput, MoveSceneInput, MoveSceneOutput, OperatorDeleteSceneInput,
    OperatorDeleteSceneOutput, PlanChapterInput, PlanChapterOutput, ProjectSummary,
    RebuildSearchIndexInput, RebuildSearchIndexOutput, RecordKnowledgeInput, RecordKnowledgeOutput,
    RecordNoteInput, RecordNoteOutput, RegisterCanonicalFactInput, RegisterCanonicalFactOutput,
    RenameReport, ResolveRevisionMarkerInput, ResolveRevisionMarkerOutput,
    RestoreSceneVersionInput, RestoreSceneVersionOutput, SaveSceneDraftInput, SaveSceneDraftOutput,
    SaveSummaryInput, SaveSummaryOutput, SceneDeleteImpactGroup, SceneDeleteImpactTarget,
    SceneDeleteReadiness, SceneMoveImpactDestination, SceneMoveImpactGroup, SceneMoveReadiness,
    SceneVersionSummary, SearchBibleInput, SearchBibleMode, SearchBibleOutput,
    SearchBibleResultItem, SetArcPacingConstraintsInput, SetArcPacingConstraintsOutput,
    SetBookOutlineInput, SetBookOutlineOutput, SetChapterOutlineInput, SetChapterOutlineOutput,
    SetCharacterVoiceProfileInput, SetCharacterVoiceProfileOutput, SetCharacterVoiceProfilesInput,
    SetCharacterVoiceProfilesOutput, StoryPlacement, SwitchBranchInput, SwitchBranchOutput,
    TestAgentInput, TestAgentOutput, UpdateArcMilestoneInput, UpdateArcMilestoneOutput,
    UpdateEntityInput, UpdateEntityOutput, UpdatePromiseStatusInput, UpdatePromiseStatusOutput,
    UpdateRelationshipInput, UpdateRelationshipOutput, UpdateWorldRuleInput, UpdateWorldRuleOutput,
    UpdateWriterPositionInput,
};

use super::repository::{
    AppendCharacterStateParams, AppendProgressionEventParams, AppendSessionActivityParams,
    UpsertKnowledgeFactParams, UpsertKnowsParams, UpsertWriterPositionParams,
};

use super::project_resources::{
    PaginatedProjectResourceKind, future_knowledge_to_json, paginated_project_resource_response,
    parse_project_resource_page_request, persisted_dual_persona_review, relates_to_json,
    temporal_intervention_to_json, timeline_event_to_json,
};

use crate::ai::SearchDocument;

use super::Repository;

/// Cap on the in-memory generation-receipt cache. Once exceeded, oldest entries
/// are evicted FIFO. Mirrors the SurrealDB-era `MAX_GENERATION_RECEIPTS = 256`.
/// The cache is a read-through front for the persisted `generation_receipt`
/// table (migration V0032); the DB is the source of truth across restarts.
const MAX_GENERATION_RECEIPTS: usize = 256;

/// Time-to-live for a persisted generation receipt (live-run bug 4c). The
/// in-memory version carried NO wall-clock expiry — a receipt lived until the
/// process died. Persisted receipts outlive the process, so an explicit bound
/// replaces the process lifetime as the natural GC. 24h is long enough for any
/// realistic draft→revise→save loop across a primary restart, short enough that
/// abandoned receipts do not accumulate. Constant by decision (no per-call
/// override); expiry is enforced on read and expired rows are swept lazily.
const GENERATION_RECEIPT_TTL: chrono::Duration = chrono::Duration::hours(24);

/// Degenerate-scene guard for style-learning capture (evolution §3.9). A scene
/// whose agent draft or operator edit exceeds this many characters skips
/// capture with a `tracing::warn` — a scene this large is not a clean style
/// signal and would bloat the refresh corpus.
const STYLE_EDIT_MAX_CHARS: usize = 60_000;

/// Built-in minimum scene word count for the stub-scene gates (migration
/// V0036). A live run shipped scenes whose `full_text` was literally
/// "placeholder" (1 word) and a 5-word fragment to EPUB: every gate only
/// checked `full_text.trim().is_empty()`. Scenes below this floor — or whose
/// body matches `PLACEHOLDER_BODY_MARKERS` — are now flagged by
/// `preflight_book_export` (Blocking `scene_stub_text`), `compile_manuscript`
/// (`stub_scenes`), and `check_consistency` (`scene_stub_text` warnings).
/// Per-project override: `project.min_scene_word_count` via `update_entity`.
/// 20 matches the existing "draft is too thin for a confident prose review"
/// heuristic; a legitimately shorter scene (flash interstitials) lowers the
/// floor for its project rather than weakening every project.
pub(crate) const DEFAULT_MIN_SCENE_WORD_COUNT: usize = 20;

/// Whole-body placeholder markers. Compared against the FULLY NORMALIZED
/// body (trimmed, outer punctuation/brackets stripped, lowercased, inner
/// whitespace collapsed), so only a scene whose ENTIRE body is one of these
/// — e.g. literally "placeholder" or "[TODO]" — matches; real prose that
/// merely contains the word never does.
const PLACEHOLDER_BODY_MARKERS: &[&str] = &[
    "placeholder",
    "todo",
    "tbd",
    "fixme",
    "tk",
    "lorem ipsum",
    "coming soon",
    "not yet drafted",
    "not yet written",
    "scene goes here",
    "insert scene here",
    "draft to come",
];

fn normalize_body_for_placeholder_match(full_text: &str) -> String {
    let stripped = full_text
        .trim()
        .trim_matches(|c: char| {
            matches!(
                c,
                '[' | ']'
                    | '('
                    | ')'
                    | '{'
                    | '}'
                    | '<'
                    | '>'
                    | '"'
                    | '\''
                    | '.'
                    | '!'
                    | '?'
                    | ','
                    | ';'
                    | ':'
                    | '*'
                    | '_'
                    | '-'
                    | '~'
            )
        })
        .trim();
    stripped
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Why a scene body is a stub, if it is one: a placeholder body regardless
/// of length, or prose below `min_words`. Whitespace-only bodies count as
/// having no prose (the empty-text gate reports them separately).
pub(crate) fn scene_stub_reason(full_text: &str, min_words: usize) -> Option<String> {
    let words = full_text.split_whitespace().count();
    if words == 0 {
        return Some("scene has no prose".to_string());
    }
    let normalized = normalize_body_for_placeholder_match(full_text);
    if PLACEHOLDER_BODY_MARKERS.contains(&normalized.as_str()) {
        return Some(format!("body is placeholder text ({normalized:?})"));
    }
    if words < min_words {
        return Some(format!(
            "only {words} {words_word} (project minimum is {min_words})",
            words_word = if words == 1 { "word" } else { "words" }
        ));
    }
    None
}

/// Map a poisoned service lock to the pre-decided `anyhow` error naming the
/// lock. Every hot-path lock acquisition funnels through this so a poisoned
/// lock surfaces as `Err` instead of panicking the server. Lock ordering,
/// scope, and granularity are unchanged — only the poisoned-lock outcome
/// changes (panic → `Err`).
fn poisoned_lock(lock: &'static str) -> anyhow::Error {
    anyhow::anyhow!("{lock} poisoned")
}

/// In-memory record of one `continue_generation` / `revise_generation` call.
/// Kept on the service instance so a subsequent `save_scene_draft` /
/// `revise_generation` can resolve a `generation_id` back to the actual
/// model output without re-running the model. The receipt lookup with
/// integrity checks lives on `verified_revisable_draft_receipt`.
#[derive(Debug, Clone)]
struct GenerationReceiptRecord {
    id: String,
    route: String,
    rating: Option<String>,
    agent_id: String,
    output_sha256: String,
    output_text: String,
    explicit_capable_agent: bool,
    model_call_ids: Vec<String>,
}

/// Running-or-terminal state of a detached dual-persona review job. The result
/// itself is NOT carried here — it is persisted to `dual_persona_review` round
/// by round, so a waiter re-reads the row rather than cloning a large payload
/// through the channel.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ReviewJobState {
    Running,
    Done,
    Failed(String),
}

/// A detached review job plus the total round count it is driving toward.
/// `target_rounds` is what makes a post-timeout retry a COLLECT rather than a
/// top-up: a caller asking for that same total joins this job instead of
/// stacking another `rounds` on top of the rounds it is already producing.
#[derive(Clone)]
struct ReviewJob {
    target_rounds: usize,
    state: tokio::sync::watch::Receiver<ReviewJobState>,
}

/// How long `run_dual_persona_review` waits on a detached job before returning
/// `status: "in_progress"`. Deliberately well under a typical 60s MCP request
/// timeout: the point is to answer the CALLER promptly while the job keeps
/// running, not to race the timeout. Small under `cfg!(test)` so the
/// timeout-then-resume path is exercised without slow tests.
const REVIEW_JOB_JOIN_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(150)
} else {
    Duration::from_secs(25)
};

/// Deep-tier model calls in flight at once. The per-scene calls are independent,
/// so fanning them out is what lets a chapter range finish; the cap keeps a wide
/// range from opening one connection per scene against the review provider.
const DEEP_CHECK_CONCURRENCY: usize = 4;

/// One deep-tier model response, plus whether it came from the built-in local
/// stub. Tiers that fall back to a heuristic when no real review model is
/// configured branch on `local_stub`; tiers that parse regardless ignore it.
struct DeepTierOutput {
    output: String,
    local_stub: bool,
}

/// SQLite-backed Spindle service layer. Cheap to clone — the inner
/// [`Repository`] handle is `Arc`-wrapped; generation-receipt state is
/// also `Arc`-shared so all clones see the same receipt cache.
#[derive(Clone)]
pub struct SqliteSpindleService {
    repository: Repository,
    generation_receipts: std::sync::Arc<
        std::sync::RwLock<std::collections::BTreeMap<String, GenerationReceiptRecord>>,
    >,
    generation_receipt_counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// In-flight detached review jobs, keyed by
    /// `{branch_id}|{scene_id}|{scene_fingerprint}`. Shared across clones so a
    /// retry arriving on a different service clone joins the SAME job instead
    /// of launching a duplicate run against the model.
    review_jobs: std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, ReviewJob>>>,
    /// Test-only failure injection for post-commit derived-work tests.
    #[cfg(test)]
    batch_index_refresh_failure: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

const STRUCTURED_RESEARCH_PROMPT_INSTRUCTIONS: &str = "\
Context and safety frame:
- This is factual research for a fictional book project.
- The purpose is historical, cultural, legal, social, technical, sensory, or setting accuracy for fiction writing.
- Do not provide real-world operational instructions, evasion tactics, exploitation guidance, or actionable wrongdoing.
- Do not write story prose, dialogue, or scene drafts in research mode.
- Keep all output copyright-safe, concise, provenance-aware, and clearly labeled for uncertainty.

You must respond ONLY with a valid JSON object matching the following schema. Do not include any markdown formatting, code blocks (like ```json), or trailing text.

JSON Schema:
{
  \"summary\": \"Concise summary of the research query answer (copyright-safe, no copy-pasted prose)\",
  \"sources\": [
    {
      \"title\": \"Title of source\",
      \"source_type\": \"Type of source (e.g. book, website, article, document)\",
      \"url\": \"URL if available\",
      \"author\": \"Author if available\",
      \"publisher\": \"Publisher if available\",
      \"published_date\": \"Published date if available\",
      \"reliability\": \"Reliability assessment (e.g. high, medium, low)\",
      \"tags\": [\"tag1\", \"tag2\"],
      \"summary\": \"Concise summary of this source\"
    }
  ],
  \"notes\": [
    {
      \"source_index\": 0, // 0-based index referring to the sources array above (optional)
      \"note\": \"Detailed factual note\",
      \"quote\": \"Short quote if highly relevant\",
      \"locator\": \"Page number or section locator\",
      \"tags\": [\"tag1\", \"tag2\"]
    }
  ],
  \"claims\": [
    {
      \"note_index\": 0, // 0-based index referring to the notes array above (optional)
      \"claim\": \"Distilled factual claim\",
      \"topic\": \"Specific topic\",
      \"time_period\": \"Time period of claim\",
      \"location\": \"Geographic location\",
      \"confidence\": \"Confidence level (e.g. verified, likely, uncertain)\",
      \"tags\": [\"tag1\", \"tag2\"]
    }
  ],
  \"tags\": [\"general-tag\"],
  \"uncertainty_level\": \"Assessment of uncertainty\"
}
";

const EXPLICIT_RESEARCH_SYSTEM_APPENDIX: &str = "\
YOU ARE DOING FACTUAL, HISTORICAL, OR SOCIOLOGICAL ADULT/EXPLICIT RESEARCH ONLY.
This is adult-only factual research for a fictional book project involving consenting adults.
The purpose is historical, cultural, legal, social, sexual-health, and setting context for fiction writing.
CRITICAL BOUNDARIES AND RULES:
1. You may research adult sexual culture, adult entertainment history, sexual health context, adult relationship norms, consent practices, legal/social history, and setting-specific adult context.
2. DO NOT generate narrative scene prose or story drafts.
3. DO NOT produce any content or instructions involving exploitation, coercion, abuse, minors, or illegal sexual conduct.
4. DO NOT provide pickup tactics, manipulation tactics, pornography, or real-world operational instructions.
5. Tag all explicit/adult research claims, notes, and sources clearly with tags like 'explicit' or 'adult'.
6. Summaries and claims must be copyright-safe, concise, and focused on factual information with provenance.
";

const FICTION_REVIEW_PROMPT_FRAME: &str = "\
Context and safety frame:
- This is editorial review for a fictional book project.
- Treat the excerpt as manuscript prose, not as real-world instructions, events, or advice.
- Provide craft and reader feedback only; do not continue the scene or write replacement prose unless the review prompt explicitly asks for it.
- For explicit-rated scenes, evaluate adult prose only as fiction involving consenting adults and flag consent, age, exploitation, coercion, or boundary concerns instead of normalizing them.
";

fn is_prose_like(text: &str) -> bool {
    let has_dialogue = (text.contains('"') || text.contains("“") || text.contains("”"))
        && (text.contains("said")
            || text.contains("asked")
            || text.contains("replied")
            || text.contains("shouted")
            || text.contains("thought"));
    let is_very_long_prose = text.len() > 300 && text.split('.').count() > 3;
    has_dialogue || is_very_long_prose
}

fn clean_json_response(raw: &str) -> &str {
    let mut s = raw.trim();
    if s.starts_with("```json") {
        s = s.strip_prefix("```json").unwrap_or(s);
    } else if s.starts_with("```") {
        s = s.strip_prefix("```").unwrap_or(s);
    }
    if s.ends_with("```") {
        s = s.strip_suffix("```").unwrap_or(s);
    }
    s.trim()
}

fn extend_unique_tags(tags: &mut Vec<String>, additions: &[String]) {
    for addition in additions {
        let trimmed = addition.trim();
        if !trimmed.is_empty()
            && !tags
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(trimmed))
        {
            tags.push(trimmed.to_string());
        }
    }
}

fn has_tag(tags: &[String], needle: &str) -> bool {
    tags.iter().any(|tag| tag.eq_ignore_ascii_case(needle))
}

/// Sort a single kind's active-thread rows by id ascending and cap the count,
/// enforcing the deterministic within-kind ordering and per-kind budget from
/// T-102.
fn finalize_active_thread_kind(
    kind: &mut Vec<spindle_core::models::ActiveThreadSummary>,
    cap: usize,
) {
    kind.sort_by(|a, b| a.id.cmp(&b.id));
    kind.truncate(cap);
}

/// Render a theme's placement status from its introduction / resolution chapter
/// numbers. Returns "" when neither is set.
fn active_thread_placement_status(
    introduction_chapter: Option<i32>,
    resolution_chapter: Option<i32>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(chapter) = introduction_chapter {
        parts.push(format!("introduction: ch {chapter}"));
    }
    if let Some(chapter) = resolution_chapter {
        parts.push(format!("resolution: ch {chapter}"));
    }
    parts.join(" / ")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct ResearchQueryOutputJson {
    pub summary: String,
    #[serde(default)]
    pub sources: Vec<spindle_core::models::ResearchSourceOutputItem>,
    #[serde(default)]
    pub notes: Vec<spindle_core::models::ResearchNoteOutputItem>,
    #[serde(default)]
    pub claims: Vec<spindle_core::models::ResearchClaimOutputItem>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub uncertainty_level: Option<String>,
}

mod agents;
mod batch;
mod branch_snapshots;
mod branches;
mod canon_events;
mod canon_mining;
mod canon_policy;
mod canon_ratification;
mod consistency;
mod context;
mod drafting;
mod editorial;
mod embedding_generations;
mod entities;
mod entity_updates;
mod export;
mod generation;
mod history;
mod import_hydration;
mod imports;
mod memory;
mod narrative;
mod packet_generation;
mod plan_amendments;
mod planning;
mod projects;
mod reader_sim;
mod replan;
mod research;
mod resources;
mod reviews;
mod scene_changes;
mod scene_commit;
mod scenes;
mod source_bridge;
mod structure;
mod validators;
mod writer;

#[cfg(test)]
mod tests;

impl SqliteSpindleService {
    pub fn new(repository: Repository) -> Self {
        Self {
            repository,
            generation_receipts: std::sync::Arc::new(std::sync::RwLock::new(
                std::collections::BTreeMap::new(),
            )),
            generation_receipt_counter: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            review_jobs: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::BTreeMap::new(),
            )),
            #[cfg(test)]
            batch_index_refresh_failure: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
                false,
            )),
        }
    }

    /// Test-only hook: make the next post-commit batch index refresh fail so
    /// the regression can prove committed rows still return a success receipt.
    #[cfg(test)]
    pub fn fail_next_batch_index_refresh(&self) {
        self.batch_index_refresh_failure
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    pub fn get_all_generation_receipts(&self) -> Result<Vec<(Option<String>, bool)>> {
        let receipts = self
            .generation_receipts
            .read()
            .map_err(|_| poisoned_lock("generation receipts read lock"))?;
        Ok(receipts
            .values()
            .map(|r| (r.rating.clone(), r.explicit_capable_agent))
            .collect())
    }
}

// =============================================================================
// Service-private helpers for export tools.
// =============================================================================

/// In-memory artifact returned by `build_project_export_payload`. Carries
/// the rendered JSON payload plus the row-count summary every caller needs
/// (`export_bible` for `ExportBibleOutput`, save-point persistence for
/// `snapshot_record_count`). Mirrors the same-named struct in
/// `services/mod.rs:189` in 705b835^.
struct ProjectExportArtifact {
    project_name: String,
    payload: serde_json::Value,
    exported_tables: usize,
    exported_records: usize,
}

/// Insert one table's worth of rows into the export accumulator, recording
/// both the per-table count and the table's JSON value. Mirrors
/// `insert_export_rows` from the SurrealDB-era helpers.
fn insert_export_rows(
    tables: &mut std::collections::BTreeMap<String, serde_json::Value>,
    counts: &mut std::collections::BTreeMap<String, usize>,
    table_name: &str,
    rows: Vec<serde_json::Value>,
) {
    counts.insert(table_name.to_string(), rows.len());
    tables.insert(table_name.to_string(), serde_json::Value::Array(rows));
}

/// Insert a single-object table (used for `project`). Counts as one record.
fn insert_export_object(
    tables: &mut std::collections::BTreeMap<String, serde_json::Value>,
    counts: &mut std::collections::BTreeMap<String, usize>,
    table_name: &str,
    value: serde_json::Value,
) {
    counts.insert(table_name.to_string(), 1);
    tables.insert(table_name.to_string(), value);
}

/// Cross-check the four invariants of a save-point snapshot JSON payload
/// before applying a restore: format string, project_id, save_point_id,
/// branch_id. Mirrors `validate_save_point_snapshot_payload` from
/// `services/mod.rs:16636..16663` in 705b835^.
fn validate_save_point_snapshot_payload(
    payload: &serde_json::Value,
    project_id: &str,
    save_point_id: &str,
    branch_id: &str,
) -> Result<()> {
    use serde_json::Value;
    if payload.get("format").and_then(Value::as_str) != Some("spindle-save-point-v1") {
        anyhow::bail!("save point snapshot format is invalid");
    }
    if payload.get("project_id").and_then(Value::as_str) != Some(project_id) {
        anyhow::bail!("save point snapshot belongs to a different project");
    }
    if payload.get("save_point_id").and_then(Value::as_str) != Some(save_point_id) {
        anyhow::bail!("save point snapshot does not match the requested save point");
    }
    // `branch_id` here is the *active* branch on the project — we already
    // verified `save_point.branch_id == active_branch_id` upstream, so the
    // snapshot must record the same branch.
    let snapshot_branch = payload
        .get("tables")
        .and_then(Value::as_object)
        .and_then(|t| t.get("save_point"))
        .and_then(Value::as_array)
        .and_then(|rows| {
            rows.iter().find_map(|row| {
                let id = row.get("id").and_then(Value::as_str)?;
                if id == save_point_id {
                    row.get("branch_id").and_then(Value::as_str)
                } else {
                    None
                }
            })
        });
    if snapshot_branch != Some(branch_id) {
        anyhow::bail!("save point snapshot belongs to a different branch");
    }
    Ok(())
}

/// Walk every branch-scoped table in the snapshot payload and keep only the
/// rows that belong to `branch_id` (or to a character/scene on the branch
/// for the FK-joined child tables). Mirrors `build_branch_restore_snapshot`
/// from `services/mod.rs:1718..1831` in 705b835^.
async fn build_branch_restore_snapshot(
    repository: &super::Repository,
    project_id: &str,
    branch_id: &str,
    payload: &serde_json::Value,
) -> Result<crate::sqlite::repository::BranchRestoreSnapshot> {
    use crate::sqlite::repository::{BRANCH_RESTORE_TABLES, BranchRestoreSnapshot};
    use serde_json::Value;
    use std::collections::{BTreeMap, BTreeSet};

    // Live character/scene ids on the branch — used to filter join-table
    // rows whose own row schema doesn't carry branch_id directly
    // (character_voice_profile, character_emotional_profile,
    // scene_source_link). The snapshot itself carries the *target-state*
    // character/scene rows, so we union the live set with the snapshot's
    // set so newly added entities also get their child rows restored.
    let mut live_character_ids: BTreeSet<String> = repository
        .list_characters_by_project_and_branch(project_id, branch_id)
        .await?
        .into_iter()
        .filter(|c| c.branch_id == branch_id)
        .map(|c| c.id)
        .collect();
    let mut live_scene_ids: BTreeSet<String> = repository
        .list_scenes_by_project_and_branch(project_id, branch_id)
        .await?
        .into_iter()
        .filter(|s| s.branch_id == branch_id)
        .map(|s| s.id)
        .collect();

    // Pull characters/scenes from the snapshot and pre-collect their ids so
    // the child-row filters can see them.
    let snapshot_character_ids: BTreeSet<String> = rows_for_branch(payload, "character", branch_id)
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    let snapshot_scene_ids: BTreeSet<String> = rows_for_branch(payload, "scene", branch_id)
        .iter()
        .filter_map(|row| row.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    live_character_ids.extend(snapshot_character_ids.iter().cloned());
    live_scene_ids.extend(snapshot_scene_ids.iter().cloned());

    let mut rows_by_table: BTreeMap<String, Vec<serde_json::Map<String, Value>>> = BTreeMap::new();

    for table in BRANCH_RESTORE_TABLES {
        let rows = match *table {
            // child tables filtered by parent_id
            "character_voice_profile" | "character_emotional_profile" => {
                rows_filtered_by_field(payload, table, "character_id", &live_character_ids)
            }
            "scene_source_link" => {
                rows_filtered_by_field(payload, table, "scene_id", &live_scene_ids)
            }
            // everything else lives under branch_id
            _ => rows_for_branch(payload, table, branch_id),
        };
        if !rows.is_empty() {
            rows_by_table.insert((*table).to_string(), rows);
        }
    }

    Ok(BranchRestoreSnapshot { rows_by_table })
}

/// Pull `tables.{table}` from the payload and return every row whose
/// `branch_id` matches. Helper for `build_branch_restore_snapshot`.
fn rows_for_branch(
    payload: &serde_json::Value,
    table: &str,
    branch_id: &str,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    use serde_json::Value;
    payload
        .get("tables")
        .and_then(Value::as_object)
        .and_then(|tables| tables.get(table))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_object())
                .filter(|row| row.get("branch_id").and_then(Value::as_str) == Some(branch_id))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Pull `tables.{table}` and return rows whose `field` value is in
/// `allowed`. Helper for join-table child rows
/// (e.g. `character_voice_profile` filtered by `character_id`).
fn rows_filtered_by_field(
    payload: &serde_json::Value,
    table: &str,
    field: &str,
    allowed: &std::collections::BTreeSet<String>,
) -> Vec<serde_json::Map<String, serde_json::Value>> {
    use serde_json::Value;
    payload
        .get("tables")
        .and_then(Value::as_object)
        .and_then(|tables| tables.get(table))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.as_object())
                .filter(|row| {
                    row.get(field)
                        .and_then(Value::as_str)
                        .is_some_and(|v| allowed.contains(v))
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Number of tables in the snapshot that carry at least one row. Mirrors
/// `count_restore_tables` from `services/mod.rs:16821..16866` in 705b835^.
fn count_restore_tables(snapshot: &crate::sqlite::repository::BranchRestoreSnapshot) -> usize {
    snapshot
        .rows_by_table
        .values()
        .filter(|v| !v.is_empty())
        .count()
}

/// Total rows in the snapshot, summed across every table. Mirrors
/// `count_restore_records` from `services/mod.rs:16868..16912` in 705b835^.
fn count_restore_records(snapshot: &crate::sqlite::repository::BranchRestoreSnapshot) -> usize {
    snapshot.rows_by_table.values().map(|v| v.len()).sum()
}

/// Convert an arbitrary display string into a safe filesystem component:
/// lowercase ASCII alphanumerics, single dashes for everything else, no
/// leading/trailing dashes. Empty input → `"snapshot"`. Byte-for-byte
/// mirror of `slugify_filename_component` from the SurrealDB-era helpers
/// (`services/mod.rs:16958..16978` in 705b835^).
fn slugify_filename_component(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if !last_was_dash {
            slug.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-');
    if trimmed.is_empty() {
        "snapshot".to_string()
    } else {
        trimmed.to_string()
    }
}

fn export_dir_for_data_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    if data_dir.file_name().and_then(|name| name.to_str()) == Some(".spindle")
        && let Some(workspace_root) = data_dir.parent()
    {
        return workspace_root.join("exports");
    }
    data_dir.join("exports")
}

/// Resolve the workspace `artifacts/` directory for a data dir, using the same
/// workspace-root discovery as [`export_dir_for_data_dir`]: a project-local
/// `.spindle/` data dir places artifacts beside the workspace root, and a
/// global data dir nests them under itself. This is the directory the
/// `compile_manuscript` opt-in write targets; it does not invent a new root.
fn artifacts_dir_for_data_dir(data_dir: &std::path::Path) -> std::path::PathBuf {
    if data_dir.file_name().and_then(|name| name.to_str()) == Some(".spindle")
        && let Some(workspace_root) = data_dir.parent()
    {
        return workspace_root.join("artifacts");
    }
    data_dir.join("artifacts")
}

#[derive(Debug, Clone, Copy)]
struct ExportChapterScope {
    start_chapter_number: i32,
    end_chapter_number: i32,
}

/// Resolve the chapter-range scope from the export inputs. Returns
/// `None` for full-book export, `Some(scope)` for a range, and errors on
/// invalid combinations (one side missing, range without book_number,
/// inverted range).
fn resolve_export_chapter_scope(
    book_number: Option<i32>,
    start_chapter_number: Option<i32>,
    end_chapter_number: Option<i32>,
) -> Result<Option<ExportChapterScope>> {
    match (start_chapter_number, end_chapter_number) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => anyhow::bail!(
            "chapter range export requires both start_chapter_number and end_chapter_number"
        ),
        (Some(start), Some(end)) => {
            if book_number.is_none() {
                anyhow::bail!("chapter range export requires book_number");
            }
            if start > end {
                anyhow::bail!(
                    "start_chapter_number must be less than or equal to end_chapter_number"
                );
            }
            Ok(Some(ExportChapterScope {
                start_chapter_number: start,
                end_chapter_number: end,
            }))
        }
    }
}

fn export_scope_contains_chapter(
    chapter_scope: Option<ExportChapterScope>,
    chapter_number: i32,
) -> bool {
    chapter_scope.is_none_or(|scope| {
        chapter_number >= scope.start_chapter_number && chapter_number <= scope.end_chapter_number
    })
}

/// Render the natural-language summary for `diff_branches`'s
/// `narrative_impact_summary` field. Verbatim from
/// services/mod.rs:16107..16138 in 705b835^.
fn summarize_branch_impact(
    scene_diffs: &[spindle_core::models::SceneDiffItem],
    character_state_diffs: &[spindle_core::models::CharacterStateDiffItem],
    relationship_diffs: &[spindle_core::models::RelationshipDiffItem],
    pacing_diffs: &[spindle_core::models::PacingDiffItem],
) -> String {
    let mut parts = Vec::new();
    if !scene_diffs.is_empty() {
        parts.push(format!("{} scene changes", scene_diffs.len()));
    }
    if !character_state_diffs.is_empty() {
        parts.push(format!(
            "{} character state shifts",
            character_state_diffs.len()
        ));
    }
    if !relationship_diffs.is_empty() {
        parts.push(format!("{} relationship deltas", relationship_diffs.len()));
    }
    if !pacing_diffs.is_empty() {
        parts.push(format!("{} pacing adjustments", pacing_diffs.len()));
    }
    if parts.is_empty() {
        "No material differences were detected between the two branches.".to_string()
    } else {
        format!(
            "The comparison branch diverges through {}.",
            parts.join(", ")
        )
    }
}

// =============================================================================
// Service-private helpers for continue_generation / revise_generation.
// Verbatim from services/mod.rs lines 15817..15846 in 705b835^.
// =============================================================================

fn normalize_generation_rating(rating: &str) -> String {
    rating.trim().to_ascii_lowercase()
}

/// Natural key identifying the one scene an explicit generation receipt is
/// spent on (migration V0034). Built from the same
/// `(book_number, chapter_number, scene_order)` fields
/// `Repository::persist_scene` treats as authoritative placement, so the claim
/// always names the scene the save actually writes.
fn explicit_receipt_scene_key(input: &SaveSceneDraftInput) -> String {
    format!(
        "{}|{}|{}|{}",
        input.project_id, input.book_number, input.chapter_number, input.scene_order
    )
}

fn normalized_generation_text(text: &str) -> String {
    text.replace("\r\n", "\n").trim().to_string()
}

fn build_generation_revision_prompt(
    original_text: &str,
    edit_instructions: &str,
    context: Option<&str>,
) -> String {
    let context_section = context
        .map(str::trim)
        .filter(|context| !context.is_empty())
        .map(|context| format!("\nContinuity/context notes:\n{context}\n"))
        .unwrap_or_default();

    format!(
        concat!(
            "Revise the generated scene prose below according to the instructions.\n",
            "Return only the revised prose text. Do not include markdown fences, commentary, ",
            "summaries, or analysis.\n",
            "{}\n",
            "Edit instructions:\n{}\n\n",
            "Original prose:\n{}"
        ),
        context_section, edit_instructions, original_text
    )
}

fn mermaid_node_id(prefix: &str, persisted_id: &str) -> String {
    let hash = generation_sha256_hex(format!("{prefix}:{persisted_id}").as_bytes());
    format!("{prefix}_{}", &hash[..16])
}

fn mermaid_escape_label(label: &str) -> String {
    let mut escaped = String::with_capacity(label.len());
    for ch in label.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '[' => escaped.push_str("&#91;"),
            ']' => escaped.push_str("&#93;"),
            '{' => escaped.push_str("&#123;"),
            '}' => escaped.push_str("&#125;"),
            '|' => escaped.push_str("&#124;"),
            '\n' | '\r' => escaped.push_str("\\n"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn timeline_event_graph_label(event: &crate::sqlite::records::TimelineEvent) -> String {
    let scene = event
        .placement
        .scene_order
        .map(|scene_order| format!(" S{scene_order}"))
        .unwrap_or_default();
    format!(
        "B{} C{}{}: {}",
        event.placement.book_number, event.placement.chapter_number, scene, event.title
    )
}

fn mermaid_missing_endpoint_summary(
    intervention: &crate::sqlite::records::TemporalIntervention,
    event_node_ids: &std::collections::BTreeMap<String, String>,
) -> String {
    let mut missing = Vec::new();
    match intervention.source_event_id.as_ref() {
        Some(source_event_id) if !event_node_ids.contains_key(source_event_id) => {
            missing.push("source");
        }
        None => missing.push("source"),
        _ => {}
    }
    match intervention.target_event_id.as_ref() {
        Some(target_event_id) if !event_node_ids.contains_key(target_event_id) => {
            missing.push("target");
        }
        None => missing.push("target"),
        _ => {}
    }
    if missing.is_empty() {
        "endpoint".to_string()
    } else {
        missing.join(" and ")
    }
}

/// Local helper named to avoid clashing with `crate::import::sha256_hex`
/// in case both end up imported here later. Mirrors that helper byte-for-byte.
fn generation_sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Render a Blocking-issue list into the user-facing string format used
/// in the `export_epub` / `export_bible` failure path. Mirrors
/// services/mod.rs:18787 in 705b835^.
fn format_export_issues(issues: &[spindle_core::models::ExportIssue]) -> String {
    issues
        .iter()
        .map(|issue| {
            let mut scope = String::new();
            if let Some(book_number) = issue.book_number {
                scope.push_str(&format!("book {book_number}"));
            }
            if let Some(chapter_number) = issue.chapter_number {
                if !scope.is_empty() {
                    scope.push(' ');
                }
                scope.push_str(&format!("chapter {chapter_number}"));
            }
            if let Some(scene_order) = issue.scene_order {
                if !scope.is_empty() {
                    scope.push(' ');
                }
                scope.push_str(&format!("scene {scene_order}"));
            }
            if scope.is_empty() {
                format!("- [{}] {}", issue.code, issue.message)
            } else {
                format!("- [{}] {} ({scope})", issue.code, issue.message)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// Service-private helpers for compare_alternatives.
// Deterministic scoring over Scene records — see the SurrealDB reference
// (services/mod.rs:16270..16307 in 705b835^) for the original. Intentionally
// kept private; these are not general-purpose formatters and should not
// migrate to `crate::format`.
// =============================================================================

fn score_alternative_scene(scene: &crate::sqlite::records::Scene) -> i32 {
    let summary_len = scene.summary.split_whitespace().count() as i32;
    let text_len = scene.full_text.split_whitespace().count() as i32;
    summary_len.min(20) + (text_len / 4).min(30)
}

fn strongest_trait_for_scene(scene: &crate::sqlite::records::Scene) -> String {
    match scene.tone.as_deref() {
        Some("volatile") | Some("reckless") | Some("urgent") => "best hook ending".to_string(),
        Some("measured") | Some("subtle") => "best pacing balance".to_string(),
        Some("lyrical") | Some("intense") => "strongest voice signal".to_string(),
        _ => "most balanced scene shape".to_string(),
    }
}

fn pacing_note_for_scene(scene: &crate::sqlite::records::Scene) -> String {
    match scene.tone.as_deref() {
        Some("measured") | Some("subtle") => {
            "This draft is the easiest to slot into steady pacing.".to_string()
        }
        Some("volatile") | Some("reckless") | Some("urgent") => {
            "This draft spends pacing budget aggressively.".to_string()
        }
        _ => "This draft keeps pacing impact moderate.".to_string(),
    }
}

fn hook_note_for_scene(scene: &crate::sqlite::records::Scene) -> String {
    if scene.summary.to_lowercase().contains("uncertainty")
        || scene.full_text.to_lowercase().contains("semantic recall")
    {
        "Ends on an explicit unresolved hook.".to_string()
    } else {
        "Ends with a cleaner transition than a cliff-hanger.".to_string()
    }
}

// =============================================================================
// Service-private helpers for generate_alternatives. Verbatim from
// services/mod.rs lines 16205..16271 and 17069..17084 in 705b835^. The
// "synthesis" is deliberately a deterministic projection over the scene
// context — no LLM call — so the alternative-generation fan-out is
// testable and reproducible.
// =============================================================================

fn synthesize_alternative_scene(
    context: &spindle_core::models::SceneContextOutput,
    input: &spindle_core::models::GenerateAlternativesInput,
    index: usize,
) -> (String, String) {
    let location = &context.scene.location.name;
    let lead_character = context
        .scene
        .characters
        .first()
        .map(|character| character.name.clone())
        .unwrap_or_else(|| "The viewpoint character".to_string());
    let relationship_note = context
        .scene
        .relationships
        .first()
        .map(|relationship| {
            format!(
                "Trust {} and tension {} shape the exchange.",
                relationship.trust, relationship.tension
            )
        })
        .unwrap_or_else(|| "No major relationship pressure is yet committed.".to_string());

    let summary = match input.variation_strategy.as_str() {
        "temperature" => format!(
            "Alternative {} heightens uncertainty as {} confronts the scene at {}.",
            index + 1,
            lead_character,
            location
        ),
        "approach" => format!(
            "Alternative {} reframes the scene around a different tactical approach at {}.",
            index + 1,
            location
        ),
        "agent" => format!(
            "Alternative {} simulates a different drafting persona for {} at {}.",
            index + 1,
            lead_character,
            location
        ),
        _ => format!(
            "Alternative {} explores a different branch outcome for {}.",
            index + 1,
            lead_character
        ),
    };

    let full_text = format!(
        "{} {} Reader promise: {}. Semantic recall: {}.",
        summary,
        relationship_note,
        context.novel.reader_contract.promise,
        context
            .novel
            .semantic_references
            .first()
            .map(|reference| reference.title.clone())
            .unwrap_or_else(|| "none".to_string())
    );

    (summary, full_text)
}

// =============================================================================
// Service-private helpers for revise_scene. Verbatim from
// services/mod.rs:17282..17383 in 705b835^. Linear-time longest-common-prefix
// + longest-common-suffix diff; sufficient resolution for the per-revision
// MCP response. Returns (chunks, changed_ranges, chars_added, chars_deleted).
// =============================================================================

fn compute_text_diff(
    prior: &str,
    current: &str,
) -> (
    Vec<spindle_core::models::TextDiffChunk>,
    Vec<spindle_core::models::TextByteRange>,
    usize,
    usize,
) {
    use spindle_core::models::{TextByteRange, TextDiffChunk, TextDiffKind};

    if prior == current {
        let diff = if current.is_empty() {
            Vec::new()
        } else {
            vec![TextDiffChunk {
                kind: TextDiffKind::Unchanged,
                text: current.to_string(),
                byte_range: Some(TextByteRange {
                    start: 0,
                    end: current.len(),
                }),
            }]
        };
        return (diff, Vec::new(), 0, 0);
    }

    let prefix_len = shared_prefix_len(prior, current);
    let suffix_len = shared_suffix_len(&prior[prefix_len..], &current[prefix_len..]);
    let prior_mid_end = prior.len().saturating_sub(suffix_len);
    let current_mid_end = current.len().saturating_sub(suffix_len);
    let deleted = &prior[prefix_len..prior_mid_end];
    let inserted = &current[prefix_len..current_mid_end];

    let mut diff = Vec::new();
    if prefix_len > 0 {
        diff.push(TextDiffChunk {
            kind: TextDiffKind::Unchanged,
            text: current[..prefix_len].to_string(),
            byte_range: Some(TextByteRange {
                start: 0,
                end: prefix_len,
            }),
        });
    }
    if !deleted.is_empty() {
        diff.push(TextDiffChunk {
            kind: TextDiffKind::Delete,
            text: deleted.to_string(),
            byte_range: Some(TextByteRange {
                start: prefix_len,
                end: prior_mid_end,
            }),
        });
    }
    if !inserted.is_empty() {
        diff.push(TextDiffChunk {
            kind: TextDiffKind::Insert,
            text: inserted.to_string(),
            byte_range: Some(TextByteRange {
                start: prefix_len,
                end: current_mid_end,
            }),
        });
    }
    if suffix_len > 0 {
        diff.push(TextDiffChunk {
            kind: TextDiffKind::Unchanged,
            text: current[current_mid_end..].to_string(),
            byte_range: Some(TextByteRange {
                start: current_mid_end,
                end: current.len(),
            }),
        });
    }

    let byte_offsets_changed = vec![TextByteRange {
        start: prefix_len,
        end: current_mid_end,
    }];
    let chars_added = inserted.chars().count();
    let chars_deleted = deleted.chars().count();
    (diff, byte_offsets_changed, chars_added, chars_deleted)
}

fn shared_prefix_len(left: &str, right: &str) -> usize {
    let mut len = 0usize;
    for (left_char, right_char) in left.chars().zip(right.chars()) {
        if left_char != right_char {
            break;
        }
        len += left_char.len_utf8();
    }
    len
}

fn shared_suffix_len(left: &str, right: &str) -> usize {
    let mut len = 0usize;
    for (left_char, right_char) in left.chars().rev().zip(right.chars().rev()) {
        if left_char != right_char {
            break;
        }
        len += left_char.len_utf8();
    }
    len.min(left.len()).min(right.len())
}

// =============================================================================
// Helpers for narrow_scenes_by_subjects / scene_reference_subject_spec.
// Verbatim translations of the SurrealDB reference (services/mod.rs:17124
// + 17180 in 705b835^).
// =============================================================================

#[derive(Debug)]
struct SubjectSceneReferenceSpec {
    terms: Vec<String>,
    direct_scene_ids: std::collections::BTreeSet<String>,
}

fn ensure_reference_subject_project(
    entity_project_id: &str,
    project_id: &str,
    entity_label: &str,
) -> Result<()> {
    if entity_project_id != project_id {
        anyhow::bail!("{entity_label} does not belong to the requested project");
    }
    Ok(())
}

fn find_scene_reference_term_match(text: &str, terms: &[String]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for term in terms {
        let Some((start, end)) = find_literal_scene_match(text, term) else {
            continue;
        };
        let replace = match best {
            None => true,
            Some((best_start, best_end)) => {
                start < best_start || (start == best_start && end - start > best_end - best_start)
            }
        };
        if replace {
            best = Some((start, end));
        }
    }
    best
}

fn find_literal_scene_match(text: &str, needle: &str) -> Option<(usize, usize)> {
    let needle = needle.trim();
    if needle.is_empty() {
        return None;
    }
    if let Some(start) = text.find(needle) {
        return Some((start, start + needle.len()));
    }
    if text.is_ascii() && needle.is_ascii() {
        let lowered_text = text.to_ascii_lowercase();
        let lowered_needle = needle.to_ascii_lowercase();
        if let Some(start) = lowered_text.find(&lowered_needle) {
            return Some((start, start + needle.len()));
        }
    }
    None
}

fn alternative_tone(strategy: &str, index: usize) -> &'static str {
    match (strategy, index % 3) {
        ("temperature", 0) => "volatile",
        ("temperature", 1) => "uneasy",
        ("temperature", _) => "reckless",
        ("approach", 0) => "measured",
        ("approach", 1) => "confrontational",
        ("approach", _) => "subtle",
        ("agent", 0) => "lyrical",
        ("agent", 1) => "spare",
        ("agent", _) => "intense",
        (_, 0) => "tense",
        (_, 1) => "grim",
        _ => "urgent",
    }
}

// =============================================================================
// Service-private helpers for run_dual_persona_review. Mirrors the SurrealDB
// reference (services/mod.rs:16309..17080 in 705b835^) — the model-response
// parser plus the deterministic heuristic fallbacks used when the model
// router resolves to the local adapter (so unit tests pass without a real
// LLM endpoint).
// =============================================================================

fn parse_review_sections(output: &str) -> (Vec<String>, Vec<String>) {
    let mut strengths = Vec::new();
    let mut concerns = Vec::new();
    let mut current_section: Option<&str> = None;

    for line in output.lines() {
        let trimmed = line.trim();
        let upper = trimmed.to_uppercase();
        if upper.starts_with("STRENGTHS") {
            current_section = Some("strengths");
            continue;
        }
        if upper.starts_with("CONCERNS") {
            current_section = Some("concerns");
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let content = trimmed
            .trim_start_matches('-')
            .trim_start_matches('*')
            .trim_start_matches("• ")
            .trim();
        if content.is_empty() {
            continue;
        }
        match current_section {
            Some("strengths") => strengths.push(content.to_string()),
            Some("concerns") => concerns.push(content.to_string()),
            _ => {
                if strengths.len() <= concerns.len() {
                    strengths.push(content.to_string());
                } else {
                    concerns.push(content.to_string());
                }
            }
        }
    }

    if strengths.is_empty() {
        strengths.push("No specific strengths identified in this pass.".to_string());
    }
    if concerns.is_empty() {
        concerns.push("No specific concerns identified in this pass.".to_string());
    }

    (strengths, concerns)
}

fn derive_literary_concerns(scene: &crate::sqlite::records::Scene) -> Vec<String> {
    let mut concerns = Vec::new();
    if scene.summary.split_whitespace().count() < 8 {
        concerns.push("summary may undersell the scene hook".to_string());
    }
    if scene.full_text.split_whitespace().count() < 20 {
        concerns.push("draft is too thin for a confident prose review".to_string());
    }
    if scene.tone.is_none() {
        concerns
            .push("tone metadata is missing, which weakens pacing and voice review".to_string());
    }
    concerns.push(
        "structure: judge opening pressure, turn, close, and lived-in space — not BLUF or tech structure"
            .to_string(),
    );
    concerns
}

fn derive_craft_concerns(scene: &crate::sqlite::records::Scene) -> Vec<String> {
    let mut concerns = Vec::new();
    let prose = scene.full_text.to_lowercase();
    if prose.contains("felt ") || prose.contains("seemed ") || prose.contains("realized ") {
        concerns.push("filter words suggest a tighter POV rewrite pass".to_string());
    }
    if prose.matches(" was ").count() >= 3 {
        concerns.push("prose may rely on flat linking verbs more than necessary".to_string());
    }
    if scene.summary.to_lowercase().contains("alternative") {
        concerns.push(
            "alternative placeholder language should be revised into story-specific prose"
                .to_string(),
        );
    }
    if let Some(report) = scan_scene_anti_slop(&scene.full_text) {
        let hard = spindle_core::style::antislop::verify_findings(&report);
        if hard.is_empty() {
            concerns.push(
                "NONE — no hard fiction shelf to cite from the injected anti-slop report"
                    .to_string(),
            );
        } else {
            for finding in hard {
                concerns.push(format!("cite `{}`: {}", finding.shelf_id, finding.message));
            }
        }
    }
    if concerns.is_empty() {
        concerns.push("no major craft-level concerns detected in this heuristic pass".to_string());
    }
    concerns
}

/// Validate a chapter plan's scene tone/beat descriptors against the project
/// style contract. The planned `beat_structure` + `purpose` carry the tone
/// language (e.g. "Quiet, contained. The grief beat of the chapter."), so they
/// are scanned as the declared tone; the final scene also gets the
/// chapter-ending check. Returns per-scene warning strings.
pub(crate) fn validate_chapter_plan_style(
    directive: &spindle_core::style::StyleDirective,
    scenes: &[spindle_core::models::PlanChapterSceneInput],
) -> Vec<String> {
    if directive.is_empty() || scenes.is_empty() {
        return Vec::new();
    }
    let max_order = scenes
        .iter()
        .map(|scene| scene.scene_order)
        .max()
        .unwrap_or(0);
    let mut warnings = Vec::new();
    for scene in scenes {
        // beats + purpose read like a tone label at planning time.
        let declared_tone = {
            let mut parts = scene.beat_structure.clone();
            if !scene.purpose.trim().is_empty() {
                parts.push(scene.purpose.clone());
            }
            parts.join(", ")
        };
        let hits = directive.scan(&spindle_core::style::StyleScanInput {
            prose: &scene.summary,
            declared_tone: if declared_tone.trim().is_empty() {
                None
            } else {
                Some(&declared_tone)
            },
            is_chapter_end: scene.scene_order == max_order,
        });
        for hit in hits {
            warnings.push(format!("Scene {}: {}", scene.scene_order, hit.message));
        }
    }
    warnings
}

/// Heuristic backstop for the Target Reader persona when no external review
/// model is configured. Runs the style-drift scanner over the scene; the
/// review has no sibling-scene context, so the ending-beat check is left off.
fn derive_genre_concerns(
    scene: &crate::sqlite::records::Scene,
    directive: &spindle_core::style::StyleDirective,
) -> Vec<String> {
    let hits = directive.scan(&spindle_core::style::StyleScanInput {
        prose: &scene.full_text,
        declared_tone: scene.tone.as_deref(),
        is_chapter_end: false,
    });
    let mut concerns: Vec<String> = hits.into_iter().map(|hit| hit.message).collect();
    if concerns.is_empty() {
        concerns.push(
            "heuristic pass found no obvious genre-voice drift; configure a review model to judge \
             whether the scene truly delivers the declared genre experience"
                .to_string(),
        );
    }
    concerns
}

fn derive_review_actions(scene: &crate::sqlite::records::Scene) -> Vec<String> {
    let mut actions = Vec::new();
    if scene.tone.is_none() {
        actions.push("set or revise tone metadata before the next review pass".to_string());
    }
    actions.push("address reader-impact issues before line-level polish".to_string());
    if scene.full_text.split_whitespace().count() < 60 {
        actions.push(
            "expand the draft so the next review can evaluate pacing and MRU flow".to_string(),
        );
    } else {
        actions.push(
            "tighten repeated phrases and filter words in the next revision pass".to_string(),
        );
    }
    actions
}

// =============================================================================
// Service-private helpers for extract_canonical_facts_from_scene.
// Verbatim from services/mod.rs:15472..15502 in 705b835^.
// =============================================================================

fn split_scene_into_fact_sentences(text: &str) -> Vec<String> {
    text.split(['.', '!', '?', '\n'])
        .map(str::trim)
        .filter(|sentence| sentence.len() >= 24)
        .map(ToString::to_string)
        .collect()
}

fn canonical_predicate_slug(sentence: &str) -> String {
    let slug = sentence
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let collapsed = slug
        .split('_')
        .filter(|part| !part.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("_");
    if collapsed.is_empty() {
        "fact".to_string()
    } else {
        collapsed
    }
}

/// Stable fingerprint over the scene fields that drive review staleness.
fn scene_revision_fingerprint(scene: &crate::sqlite::records::Scene) -> String {
    crate::sqlite::repository::Repository::scene_revision_fingerprint(scene)
}

/// Pure helper: compare a source branch's scenes against the target
/// branch's existing scenes (with a fallback to the project's main
/// branch when the target is something other than main), and emit a
/// `MergeConflictItem` per conflicting position.
///
/// Mirrors `detect_merge_scene_conflicts` from services/mod.rs:16140..16205
/// in 705b835^. Two structural changes from the reference:
///   * IDs are plain `String`, not SurrealDB `RecordId`.
///   * The reference resolved "the main branch" via the
///     `RecordId::new("bible_branch", "main")` singleton; here the
///     caller passes in the project's per-project main branch id.
fn detect_merge_scene_conflicts(
    source_scenes: &[crate::sqlite::records::Scene],
    source_branch: &crate::sqlite::records::BibleBranch,
    target_branch_id: &str,
    main_branch_id: &str,
    target_local_scene_map: &std::collections::BTreeMap<
        (i32, i32, i32),
        &crate::sqlite::records::Scene,
    >,
    main_scene_map: &std::collections::BTreeMap<(i32, i32, i32), &crate::sqlite::records::Scene>,
) -> Vec<spindle_core::models::MergeConflictItem> {
    use spindle_core::models::MergeConflictItem;

    let source_parent_branch_id = source_branch
        .parent_branch_id
        .clone()
        .unwrap_or_else(|| main_branch_id.to_string());

    source_scenes
        .iter()
        .filter_map(|source_scene| {
            let key = (
                source_scene.book_number,
                source_scene.chapter_number,
                source_scene.scene_order,
            );
            let target_local_scene = target_local_scene_map.get(&key).copied();
            let fallback_scene = if target_branch_id == main_branch_id {
                None
            } else {
                main_scene_map.get(&key).copied()
            };
            let (target_scene, target_scene_is_local) = if let Some(scene) = target_local_scene {
                (scene, true)
            } else {
                let scene = fallback_scene?;
                (scene, false)
            };

            if scene_revision_fingerprint(source_scene) == scene_revision_fingerprint(target_scene)
            {
                return None;
            }

            let target_changed_after_source_diverged =
                target_scene.updated_at > source_branch.created_at;
            let has_conflict = if target_branch_id == source_parent_branch_id {
                target_changed_after_source_diverged
            } else if target_scene_is_local {
                true
            } else {
                target_changed_after_source_diverged
            };

            has_conflict.then(|| MergeConflictItem {
                book_number: source_scene.book_number,
                chapter_number: source_scene.chapter_number,
                scene_order: source_scene.scene_order,
                source_scene_id: source_scene.id.clone(),
                target_scene_id: target_scene.id.clone(),
                target_origin_branch_id: target_scene.branch_id.clone(),
                source_summary: source_scene.summary.clone(),
                target_summary: target_scene.summary.clone(),
            })
        })
        .collect()
}

// =============================================================================
// Consistency-check service-private helpers.
//
// These mirror the pure helpers from `services/mod.rs:18387..18900` in 705b835^.
// They live here (not in `crate::format`) because they are intimately tied to
// `check_consistency` dispatch logic and are not used by other modules.
// =============================================================================

/// Output of [`SqliteSpindleService::find_orphan_scenes`]. Stitched into
/// `ConsistencyIssue` rows by the `scene_spine_integrity` branch of
/// `check_consistency`. Mirrors the same struct in 705b835^.
#[derive(Debug, Clone)]
pub(crate) struct SceneSpineIntegrityIssue {
    pub severity: &'static str,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub message: String,
    pub entity_ids: Vec<String>,
    pub suggested_action: Option<String>,
}

/// Normalize a caller-supplied `checks` list into a stable lowercase set.
fn requested_checks(checks: &[String]) -> std::collections::BTreeSet<String> {
    checks
        .iter()
        .map(|check| check.trim().to_lowercase())
        .filter(|check| !check.is_empty())
        .collect()
}

/// Normalize and validate the optional `severity_filter`. Returns
/// `Ok(None)` when the filter is empty (meaning "do not filter"),
/// `Ok(Some(set))` for an explicit allow-list, and an error for any
/// severity name that is not one of `error | warning | info`.
fn requested_severities(
    severities: &[String],
) -> Result<Option<std::collections::BTreeSet<String>>> {
    let normalized = severities
        .iter()
        .map(|severity| severity.trim().to_lowercase())
        .filter(|severity| !severity.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    if normalized.is_empty() {
        return Ok(None);
    }

    for severity in &normalized {
        if !matches!(severity.as_str(), "error" | "warning" | "info") {
            anyhow::bail!(
                "unknown severity_filter '{}': expected error, warning, or info",
                severity
            );
        }
    }

    Ok(Some(normalized))
}

/// Returns true when the given check should run: either no checks were
/// requested (run all) or the check is explicitly listed.
/// Price facts grouped by `(subject_table, subject_id, predicate)`, each value a
/// list of `(base_value, unit, fact_id)` — drives the currency-consistency check.
type PriceFactGroups =
    std::collections::BTreeMap<(String, String, String), Vec<(f64, String, String)>>;

fn should_run_check(requested_checks: &std::collections::BTreeSet<String>, check: &str) -> bool {
    requested_checks.is_empty() || requested_checks.contains(check)
}

fn load_anti_slop_pack() -> Option<spindle_core::style::antislop::ShelfPack> {
    match spindle_core::style::antislop::ShelfPack::load_default() {
        Ok(pack) => Some(pack),
        Err(error) => {
            tracing::warn!(error = %error, "fiction anti-slop pack failed to load; skipping scan");
            None
        }
    }
}

fn project_anti_slop_overlay() -> Option<spindle_core::style::antislop::GenreOverride> {
    crate::agent_config::load_agent_config(None)
        .ok()
        .and_then(|config| config.anti_slop.to_overlay())
}

fn project_anti_slop_experimental_structural() -> bool {
    crate::agent_config::load_agent_config(None)
        .ok()
        .is_some_and(|config| config.anti_slop.experimental_structural)
}

fn suppression_store_from_rows(
    rows: &[crate::sqlite::records::StoredAntiSlopSuppression],
) -> spindle_core::style::antislop::SuppressionStore {
    spindle_core::style::antislop::SuppressionStore::from_entries(
        rows.iter()
            .map(
                |row| spindle_core::style::antislop::FalsePositiveSuppression {
                    shelf_id: row.shelf_id.clone(),
                    excerpt_normalized: row.excerpt_normalized.clone(),
                },
            )
            .collect(),
    )
}

fn scan_scene_anti_slop_with(
    prose: &str,
    chapter_prior: Option<&spindle_core::style::antislop::ChapterCounters>,
    suppressions: Option<&spindle_core::style::antislop::SuppressionStore>,
) -> Option<spindle_core::style::antislop::AntiSlopReport> {
    let pack = load_anti_slop_pack()?;
    Some(spindle_core::style::antislop::scan_with_options(
        &pack,
        &spindle_core::style::antislop::ScanInput::fiction(prose),
        spindle_core::style::antislop::ScanOptions {
            overlay: project_anti_slop_overlay().as_ref(),
            chapter_prior,
            suppressions,
            experimental_structural: project_anti_slop_experimental_structural(),
        },
    ))
}

/// Advisory scan for `save_scene_draft` / review when chapter context is
/// unavailable. Never fails the save.
fn scan_scene_anti_slop(prose: &str) -> Option<spindle_core::style::antislop::AntiSlopReport> {
    scan_scene_anti_slop_with(prose, None, None)
}

async fn chapter_counters_before(
    repo: &Repository,
    project_id: &str,
    branch_id: &str,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
    exclude_scene_id: Option<&str>,
) -> spindle_core::style::antislop::ChapterCounters {
    let mut counters = spindle_core::style::antislop::ChapterCounters::new();
    let Some(pack) = load_anti_slop_pack() else {
        return counters;
    };
    let Ok(scenes) = repo
        .list_scenes_by_project_and_branch(project_id, branch_id)
        .await
    else {
        return counters;
    };
    for scene in scenes {
        if scene.book_number != book_number || scene.chapter_number != chapter_number {
            continue;
        }
        if scene.scene_order >= scene_order {
            continue;
        }
        if exclude_scene_id.is_some_and(|id| id == scene.id) {
            continue;
        }
        counters.add_scene(&pack, &scene.full_text);
    }
    counters
}

async fn load_suppression_store(
    repo: &Repository,
    project_id: &str,
    branch_id: &str,
) -> spindle_core::style::antislop::SuppressionStore {
    match repo
        .list_anti_slop_suppressions(project_id, branch_id)
        .await
    {
        Ok(rows) => suppression_store_from_rows(&rows),
        Err(error) => {
            tracing::warn!(error = %error, "anti-slop suppressions failed to load");
            spindle_core::style::antislop::SuppressionStore::new()
        }
    }
}

/// Render the pre-draft in-world-time hard constraint: the previous scene's end
/// clock (the expected start for this scene), this scene's own clock and the
/// elapsed gap when it is already stamped, and the grounding instruction to
/// signal any elapsed time. Returns `None` when neither clock carries a day.
///
/// `current` is the latest clock at-or-before the cursor (the prior scene when
/// this one is not yet clocked); `prior` is the latest clock strictly before the
/// cursor (always the previous scene's end). `current` is treated as this
/// scene's own clock only when it is a distinct record from `prior`.
fn temporal_anchor_statement(
    calendar: &spindle_core::models::CalendarDef,
    prior: Option<&crate::sqlite::records::StoredSceneClock>,
    current: Option<&crate::sqlite::records::StoredSceneClock>,
    prior_location: Option<&str>,
) -> Option<String> {
    let prior = prior.filter(|clock| clock.clock.day_index.is_some());
    let this_clock = match (current, prior) {
        (Some(current), Some(prior)) if current.scene_id != prior.scene_id => Some(current),
        (Some(current), None) => Some(current),
        _ => None,
    }
    .filter(|clock| clock.clock.day_index.is_some());

    if prior.is_none() && this_clock.is_none() {
        return None;
    }

    let epoch_suffix = calendar
        .epoch_label
        .as_deref()
        .map(|label| format!(" (epoch: {label})"))
        .unwrap_or_default();

    let location_phrase = prior_location
        .map(|location| format!(" in {location}"))
        .unwrap_or_default();

    let mut parts: Vec<String> = Vec::new();
    if let Some(prior) = prior {
        match prior.clock.duration_days {
            Some(duration) if duration >= 1.0 => parts.push(format!(
                "The previous scene was set on {}{location_phrase}{epoch_suffix} and spanned about {} day(s).",
                render_story_clock(&prior.clock),
                format_day_count(duration),
            )),
            _ => parts.push(format!(
                "The previous scene was set on {}{location_phrase}{epoch_suffix}.",
                render_story_clock(&prior.clock),
            )),
        }
    }
    if let Some(this_clock) = this_clock {
        let elapsed = prior.and_then(|prior| {
            match (
                prior.clock.total_index(calendar),
                this_clock.clock.total_index(calendar),
            ) {
                (Some(prior_index), Some(this_index)) if this_index > prior_index => {
                    Some(format_elapsed_minutes(this_index - prior_index, calendar))
                }
                _ => None,
            }
        });
        match elapsed {
            Some(elapsed) => parts.push(format!(
                "This scene is stamped {} — about {elapsed} after the previous scene.",
                render_story_clock(&this_clock.clock),
            )),
            None => parts.push(format!(
                "This scene is stamped {}{epoch_suffix}.",
                render_story_clock(&this_clock.clock),
            )),
        }
    }

    parts.push(
        "Open this scene by establishing WHEN it takes place relative to the previous one. \
         Signal any elapsed in-world time with an explicit transition beat or a scene break, and \
         never skip the time of day inside the scene without one."
            .to_string(),
    );

    Some(parts.join(" "))
}

/// Render a story clock as `day N` or `day N at HH:MM` (time of day is minutes
/// from midnight, calendar-agnostic).
fn render_story_clock(clock: &spindle_core::models::StoryClock) -> String {
    let day = clock.day_index.unwrap_or_default();
    match clock.time_of_day {
        Some(minutes) if minutes >= 0 => {
            format!("day {day} at {:02}:{:02}", minutes / 60, minutes % 60)
        }
        _ => format!("day {day}"),
    }
}

/// Render a `duration_days` value without a trailing `.0` for whole spans.
fn format_day_count(duration: f64) -> String {
    if (duration.fract()).abs() < f64::EPSILON {
        format!("{}", duration as i64)
    } else {
        format!("{duration:.1}")
    }
}

/// Render an elapsed in-world span (in minutes) as days and/or hours.
fn format_elapsed_minutes(minutes: i64, calendar: &spindle_core::models::CalendarDef) -> String {
    let per_day = calendar.minutes_per_day().max(1);
    let days = minutes / per_day;
    let hours = (minutes % per_day) / 60;
    match (days, hours) {
        (0, 0) => "less than an hour".to_string(),
        (0, hours) => format!("{hours} hour(s)"),
        (days, 0) => format!("{days} day(s)"),
        (days, hours) => format!("{days} day(s) {hours} hour(s)"),
    }
}

/// Map the pure intra-scene temporal-coherence scan into advisory
/// `ConsistencyIssue`s for one scene. Shared by the `temporal_coherence` audit
/// check, the draft/revise validator surface, and the commit gate so all four
/// surfaces agree on representation, message, and remedy. Every issue is
/// advisory `severity: "warning"`; the scene clock, when provided, only ever
/// suppresses a finding (it never creates one), so passing `None` is the
/// correct prose-only path for a not-yet-clocked scene.
fn scan_temporal_findings(
    scene_id: &str,
    prose: &str,
    clock: Option<&crate::sqlite::records::StoredSceneClock>,
) -> Vec<spindle_core::models::ConsistencyIssue> {
    let input = spindle_core::temporal::TemporalSceneInput {
        prose,
        duration_days: clock.and_then(|clock| clock.clock.duration_days),
        temporal_mode: clock.and_then(|clock| clock.temporal_mode.as_deref()),
        precision: clock.and_then(|clock| clock.clock.precision.as_deref()),
    };
    spindle_core::temporal::scan_temporal_coherence(&input)
        .into_iter()
        .map(|hit| spindle_core::models::ConsistencyIssue {
            severity: "warning".to_string(),
            check_type: "temporal_coherence".to_string(),
            message: hit.message,
            entity_ids: vec![scene_id.to_string()],
            suggested_action: Some(
                match hit.kind {
                    "temporal_teleport" => {
                        "add a transition beat (e.g. \"Hours later,\"), a scene break (***), or \
                         split the time blocks into separately clocked scenes"
                    }
                    "temporal_drift" => {
                        "reconcile the contradicting time-of-day references, or mark an intended \
                         rewind with temporal_mode \"flashback\""
                    }
                    _ => {
                        "signal the elapsed span with a transition beat or scene break, or lower \
                         duration_days if the scene is momentary"
                    }
                }
                .to_string(),
            ),
            evidence: None,
        })
        .collect()
}

// ── Phase-4 validator helpers ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PhaseFourCacheId {
    CanonicalFactProseDrift,
    WorldRuleSemanticDrift,
    VoiceDrift,
    RetconReachability,
    StyleCompliance,
}

impl PhaseFourCacheId {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CanonicalFactProseDrift => "canonical_fact_prose_drift",
            Self::WorldRuleSemanticDrift => "world_rule_semantic_drift",
            Self::VoiceDrift => "voice_drift",
            Self::RetconReachability => "retcon_reachability",
            Self::StyleCompliance => "style_compliance",
        }
    }

    const fn all() -> &'static [Self] {
        &[
            Self::CanonicalFactProseDrift,
            Self::WorldRuleSemanticDrift,
            Self::VoiceDrift,
            Self::RetconReachability,
            Self::StyleCompliance,
        ]
    }
}

/// The Phase-4 validator `check_type` strings, in the order they appear in the
/// consistency report.
pub(super) const PHASE_FOUR_CHECK_TYPES: &[&str] = &[
    PhaseFourCacheId::CanonicalFactProseDrift.as_str(),
    PhaseFourCacheId::WorldRuleSemanticDrift.as_str(),
    PhaseFourCacheId::VoiceDrift.as_str(),
    PhaseFourCacheId::RetconReachability.as_str(),
    PhaseFourCacheId::StyleCompliance.as_str(),
];

/// Resolve the caller's `checks` list into the subset of Phase-4 validator
/// `check_type` strings that should run. An empty caller list expands to
/// every Phase-4 validator (default behaviour).
fn requested_phase_four_validator_checks(
    requested_checks: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<PhaseFourCacheId> {
    if requested_checks.is_empty() {
        return PhaseFourCacheId::all().iter().copied().collect();
    }
    PhaseFourCacheId::all()
        .iter()
        .copied()
        .filter(|check| requested_checks.contains(check.as_str()))
        .collect()
}

/// Stable list of validator id strings derived from the requested set,
/// in the canonical order. Used as the cache key when looking up
/// `validator_finding` rows by scene hash.
fn phase_four_validator_ids(checks: &std::collections::BTreeSet<PhaseFourCacheId>) -> Vec<String> {
    PhaseFourCacheId::all()
        .iter()
        .copied()
        .filter(|check| checks.contains(check))
        .map(|check| check.as_str().to_string())
        .collect()
}

fn phase_four_context_hashes(
    context: &spindle_core::validators::ValidatorContext,
    checks: &std::collections::BTreeSet<PhaseFourCacheId>,
) -> Result<std::collections::BTreeMap<&'static str, String>> {
    let mut hashes = std::collections::BTreeMap::new();
    for check in PhaseFourCacheId::all()
        .iter()
        .copied()
        .filter(|check| checks.contains(check))
    {
        let payload = match check {
            PhaseFourCacheId::CanonicalFactProseDrift => {
                let mut facts = context
                    .canonical_facts
                    .iter()
                    .map(|fact| {
                        serde_json::json!({
                            "fact_id": fact.fact_id,
                            "scene_id": fact.scene_id,
                            "book_number": fact.book_number,
                            "chapter_number": fact.chapter_number,
                            "fact_type": fact.fact_type,
                            "key": fact.key,
                            "value": fact.value,
                            "subject_id": fact.subject_id,
                            "subject_names": fact.subject_names,
                        })
                    })
                    .collect::<Vec<_>>();
                facts.sort_by_key(|value| value.to_string());
                serde_json::json!({ "canonical_facts": facts })
            }
            PhaseFourCacheId::WorldRuleSemanticDrift => {
                let mut rules = context
                    .world_rules
                    .iter()
                    .map(|rule| {
                        serde_json::json!({
                            "rule_id": rule.rule_id,
                            "rule_name": rule.rule_name,
                            "scan_pattern": rule.scan_pattern,
                            "established_in": rule.established_in,
                            // Classification inputs for secrecy-class scoping:
                            // editing either must invalidate cached findings.
                            "rule_type": rule.rule_type,
                            "description": rule.description,
                        })
                    })
                    .collect::<Vec<_>>();
                rules.sort_by_key(|value| value.to_string());
                serde_json::json!({
                    "world_rules": rules,
                    // Scanner behavior is part of the cache key: findings
                    // computed by older scanner semantics (e.g. pre-secrecy
                    // v1, which flagged narration/HUD hits) must not be
                    // served after a scanner upgrade.
                    "scanner_version": spindle_core::world_rules::SCANNER_SEMANTICS_VERSION,
                })
            }
            PhaseFourCacheId::VoiceDrift => {
                let mut profiles = context
                    .voice_profiles
                    .iter()
                    .map(|profile| {
                        serde_json::json!({
                            "character_id": profile.character_id,
                            "character_name": profile.character_name,
                            "forbidden_words": profile.forbidden_words,
                        })
                    })
                    .collect::<Vec<_>>();
                profiles.sort_by_key(|value| value.to_string());
                serde_json::json!({ "voice_profiles": profiles })
            }
            PhaseFourCacheId::RetconReachability => {
                let mut timeline_events = context
                    .timeline_events
                    .iter()
                    .map(|event| {
                        serde_json::json!({
                            "event_id": event.event_id,
                            "title": event.title,
                            "book_number": event.book_number,
                            "chapter_number": event.chapter_number,
                            "scene_order": event.scene_order,
                        })
                    })
                    .collect::<Vec<_>>();
                timeline_events.sort_by_key(|value| value.to_string());
                let mut interventions = context
                    .temporal_interventions
                    .iter()
                    .map(|intervention| {
                        serde_json::json!({
                            "intervention_id": intervention.intervention_id,
                            "title": intervention.title,
                            "source_event_id": intervention.source_event_id,
                            "target_event_id": intervention.target_event_id,
                        })
                    })
                    .collect::<Vec<_>>();
                interventions.sort_by_key(|value| value.to_string());
                serde_json::json!({
                    "timeline_events": timeline_events,
                    "temporal_interventions": interventions,
                })
            }
            PhaseFourCacheId::StyleCompliance => {
                serde_json::json!({ "style_directive": context.style_directive })
            }
        };
        let serialized = serde_json::to_vec(&payload)?;
        hashes.insert(check.as_str(), generation_sha256_hex(&serialized));
    }
    Ok(hashes)
}

/// True when every validator id in `validator_ids` appears at least once in
/// the cached rows. Used to short-circuit the registry per-scene.
fn has_cache_for_all_validators(
    rows: &[crate::sqlite::records::ValidatorFinding],
    validator_ids: &[String],
) -> bool {
    use std::collections::BTreeSet;
    let cached = rows
        .iter()
        .map(|row| row.validator_id.as_str())
        .collect::<BTreeSet<_>>();
    validator_ids
        .iter()
        .all(|validator_id| cached.contains(validator_id.as_str()))
}

/// Convert cached `validator_finding` rows into the `ConsistencyIssue`
/// payload shape — pulling apart the serialized `details_json.issues`
/// array we wrote in `run_phase_four_validator_checks_for_scenes`.
fn cached_validator_issues(
    rows: &[crate::sqlite::records::ValidatorFinding],
    scene_id: &str,
) -> Vec<spindle_core::models::ConsistencyIssue> {
    let mut issues = Vec::new();
    for row in rows {
        let Some(details_json) = row.details_json.as_ref() else {
            continue;
        };
        let Some(serialized) = details_json
            .get("issues")
            .and_then(|value| value.as_array())
        else {
            continue;
        };
        for issue in serialized {
            let severity = issue
                .get("severity")
                .and_then(|value| value.as_str())
                .unwrap_or("info")
                .to_string();
            let check_type = issue
                .get("check_type")
                .and_then(|value| value.as_str())
                .unwrap_or(row.validator_id.as_str())
                .to_string();
            let mut message = issue
                .get("message")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .to_string();
            if let Some(byte_range) = issue.get("byte_range").and_then(|value| value.as_object())
                && let (Some(start), Some(end)) = (
                    byte_range.get("start").and_then(|value| value.as_u64()),
                    byte_range.get("end").and_then(|value| value.as_u64()),
                )
            {
                message = format!("{message} (bytes {start}..{end})");
            }
            issues.push(spindle_core::models::ConsistencyIssue {
                severity,
                check_type,
                message,
                entity_ids: vec![scene_id.to_string()],
                suggested_action: Some(
                    "revise scene prose or supporting canon records".to_string(),
                ),
                evidence: None,
            });
        }
    }
    issues
}

/// Compare two `(book, chapter, scene_order)` tuples lexicographically.
fn position_lte(lhs: (i32, i32, i32), rhs: (i32, i32, i32)) -> bool {
    lhs <= rhs
}

fn position_lt(lhs: (i32, i32, i32), rhs: (i32, i32, i32)) -> bool {
    lhs < rhs
}

fn position_gt(lhs: (i32, i32, i32), rhs: (i32, i32, i32)) -> bool {
    lhs > rhs
}

/// Case-insensitive substring match (ASCII-only fold). Empty needles
/// return `false`.
fn contains_case_insensitive_phrase(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim();
    if haystack.is_empty() || needle.is_empty() {
        return false;
    }
    haystack
        .to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Like `contains_case_insensitive_phrase`, but requires non-alphanumeric
/// (or string-edge) characters on both sides of the match so e.g.
/// "siren" doesn't match "sir" embedded inside the word.
fn contains_case_insensitive_word_boundary(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim();
    if haystack.is_empty() || needle.is_empty() {
        return false;
    }
    let needle_len = needle.len();
    for (start, _) in haystack.char_indices() {
        let Some(end) = start.checked_add(needle_len) else {
            break;
        };
        if end > haystack.len() || !haystack.is_char_boundary(end) {
            continue;
        }
        let segment = &haystack[start..end];
        if !segment.eq_ignore_ascii_case(needle) {
            continue;
        }
        let prev_is_alnum = haystack[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let next_is_alnum = haystack[end..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric);
        if !prev_is_alnum && !next_is_alnum {
            return true;
        }
    }
    false
}

/// True when `status` is an explicit typed deceased marker (e.g.
/// `status:deceased`, `vital:deceased`, `vital_status:dead`, `deceased:true`).
/// Only explicit typed markers escalate `DeadCharacterAct` to `"error"`;
/// free-form deceased statuses surface as `"warning"` because prose name
/// mentions include memories, dialogue references, and corpses.
fn is_explicit_typed_deceased_marker(status: &str) -> bool {
    let trimmed = status.trim().to_ascii_lowercase();
    if let Some((prefix, rest)) = trimmed.split_once([':', '=']) {
        let key = prefix.trim();
        let val = rest.trim();
        let value_token = val
            .split(|c: char| !c.is_ascii_alphabetic())
            .find(|t| !t.is_empty())
            .unwrap_or("");
        if matches!(
            key,
            "status" | "vital" | "vital_status" | "state" | "condition" | "life_status"
        ) && matches!(value_token, "deceased" | "dead" | "killed" | "slain")
        {
            return true;
        }
        if matches!(key, "deceased" | "dead") && matches!(value_token, "true" | "yes" | "confirmed")
        {
            return true;
        }
    }
    false
}

/// True for character-state status strings that flag a deceased character
/// using explicit or word-boundary status matching while rejecting idioms
/// ("dead tired", "dead wrong", "dead quiet") and negations ("not dead",
/// "nearly dead", "presumed dead").
fn is_dead_status(status: &str) -> bool {
    if is_explicit_typed_deceased_marker(status) {
        return true;
    }
    let normalized = status.trim().to_ascii_lowercase();
    let cleaned = normalized.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    if cleaned.is_empty() {
        return false;
    }
    if matches!(cleaned, "dead" | "deceased" | "killed" | "slain") {
        return true;
    }

    let tokens: Vec<&str> = cleaned
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    for (idx, &tok) in tokens.iter().enumerate() {
        if !matches!(tok, "dead" | "deceased" | "killed" | "slain") {
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|i| tokens.get(i)).copied();
        let prev2 = idx.checked_sub(2).and_then(|i| tokens.get(i)).copied();
        let next = tokens.get(idx + 1).copied();

        if matches!(
            prev,
            Some(
                "not"
                    | "never"
                    | "nearly"
                    | "almost"
                    | "half"
                    | "presumed"
                    | "thought"
                    | "believed"
                    | "rumored"
                    | "rumoured"
                    | "feigned"
                    | "faked"
                    | "playing"
                    | "play"
                    | "drop"
                    | "brain"
                    | "walking"
            )
        ) || matches!(prev2, Some("not" | "never" | "presumed" | "thought"))
        {
            continue;
        }

        match tok {
            "deceased" => return true,
            "dead" => {
                // Reject idiomatic intensifiers / compound nouns ("dead tired",
                // "dead wrong", "dead quiet", "dead silent", "dead weight",
                // "dead end", "dead center", "dead set", "dead heat", etc.).
                if prev.is_none()
                    && (next.is_none()
                        || matches!(next, Some("in" | "at" | "by" | "from" | "since" | "after")))
                {
                    return true;
                }
                if matches!(prev, Some("confirmed" | "declared" | "pronounced" | "now"))
                    && (next.is_none()
                        || matches!(next, Some("in" | "at" | "by" | "from" | "since" | "after")))
                {
                    return true;
                }
            }
            "killed" | "slain" => {
                // Match passive status clauses ("killed in battle", "slain by
                // the warden") while rejecting active transitives ("killed a
                // guard", "killed the lights", "killed time").
                if prev.is_none()
                    && (next.is_none()
                        || matches!(
                            next,
                            Some("in" | "by" | "at" | "during" | "on" | "near" | "under" | "after")
                        ))
                {
                    return true;
                }
                if matches!(prev, Some("was" | "were" | "is" | "confirmed"))
                    && (next.is_none()
                        || matches!(
                            next,
                            Some("in" | "by" | "at" | "during" | "on" | "near" | "under" | "after")
                        ))
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Group Phase-4 validator issues into per-validator-id sections (each
/// listing the affected scenes with positions, sorted by book/chapter/
/// scene_order). Non-Phase-4 issues are skipped; empty sections are
/// dropped. Mirrors `build_consistency_report_sections` in 705b835^.
fn build_consistency_report_sections(
    issues: &[spindle_core::models::ConsistencyIssue],
    scenes: &[crate::sqlite::records::Scene],
) -> Vec<spindle_core::models::ConsistencySection> {
    use spindle_core::models::{ConsistencyIssue, ConsistencySceneFindings, ConsistencySection};
    use std::collections::BTreeMap;

    let scene_position: BTreeMap<String, (i32, i32, i32)> = scenes
        .iter()
        .map(|scene| {
            (
                scene.id.clone(),
                (scene.book_number, scene.chapter_number, scene.scene_order),
            )
        })
        .collect();

    let mut sections: Vec<ConsistencySection> = Vec::new();
    for &validator_id in PHASE_FOUR_CHECK_TYPES {
        let mut by_scene: BTreeMap<String, Vec<ConsistencyIssue>> = BTreeMap::new();
        for issue in issues {
            if issue.check_type != validator_id {
                continue;
            }
            let Some(scene_id) = issue.entity_ids.first().cloned() else {
                continue;
            };
            by_scene.entry(scene_id).or_default().push(issue.clone());
        }
        if by_scene.is_empty() {
            continue;
        }

        let mut scene_groups: Vec<ConsistencySceneFindings> = by_scene
            .into_iter()
            .map(|(scene_id, findings)| {
                let (book_number, chapter_number, scene_order) =
                    scene_position.get(&scene_id).copied().unwrap_or((0, 0, 0));
                ConsistencySceneFindings {
                    scene_id,
                    book_number,
                    chapter_number,
                    scene_order,
                    findings,
                }
            })
            .collect();
        scene_groups
            .sort_by_key(|group| (group.book_number, group.chapter_number, group.scene_order));

        sections.push(ConsistencySection {
            validator_id: validator_id.to_string(),
            scenes: scene_groups,
        });
    }
    sections
}

/// Output shape returned by the model router for the deep world-rule
/// compliance audit.
#[derive(Debug, Clone, serde::Deserialize)]
struct DeepWorldRuleCheckOutput {
    #[serde(default)]
    violations: Vec<DeepWorldRuleViolation>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DeepWorldRuleViolation {
    rule_id: String,
    #[serde(default)]
    severity: Option<String>,
    message: String,
    #[serde(default)]
    evidence: Option<String>,
}

fn build_world_rule_deep_check_prompt(
    scene: &crate::sqlite::records::Scene,
    rules: &[&crate::sqlite::records::WorldRule],
) -> String {
    let rules_block = rules
        .iter()
        .map(|rule| {
            let established_in = rule
                .established_in
                .as_ref()
                .map(|placement| {
                    format!(
                        "established at {}:{}",
                        placement.book_number, placement.chapter_number
                    )
                })
                .unwrap_or_else(|| "established earlier in canon".to_string());
            format!(
                "- rule_id: {}\n  name: {}\n  type: {}\n  description: {}\n  canon: {}",
                rule.id, rule.rule_name, rule.rule_type, rule.description, established_in
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are performing a semantic world-rule compliance audit for a novel scene.\n\
         Check whether the scene appears to violate any already-established world rules.\n\
         Ignore rules the scene simply does not engage with.\n\
         The scene's declared content rating is authoritative: judge any rating-scoped \
         mandate (e.g. requirements that apply to \"explicit\" scenes) against the scene's \
         declared rating tier, not against the intensity of what happens on the page. A \
         mandate scoped to a rating the scene does not declare is not violated by this scene.\n\
         Return strict JSON only.\n\n\
         Scene id: {}\n\
         Scene summary: {}\n\
         Scene tone: {}\n\
         Scene content rating: {}\n\
         Scene text:\n{}\n\n\
         Established rules:\n{}\n\n\
         Return this exact shape:\n\
         {{\"violations\":[{{\"rule_id\":\"world_rule:...\",\"severity\":\"warning\",\"message\":\"...\",\"evidence\":\"...\"}}]}}\n\
         Use severity \"error\" only for clear contradictions and \"warning\" for plausible but ambiguous strain.",
        scene.id,
        scene.summary,
        scene.tone.as_deref().unwrap_or("unspecified"),
        scene.content_rating.trim().to_ascii_lowercase(),
        scene.full_text,
        rules_block
    )
}

fn parse_deep_world_rule_check_output(output: &str) -> anyhow::Result<Vec<DeepWorldRuleViolation>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some(fenced) = extract_json_object(output) {
        fenced
    } else {
        trimmed.to_string()
    };

    let parsed: DeepWorldRuleCheckOutput = serde_json::from_str(&candidate)?;
    Ok(parsed.violations)
}

// ── Replan differ (replan_chapter — evolution §3.5, ADR 0003) ────────────────

/// The distinctive header line the replan prompt leads with. Doubles as the
/// fake-router marker (`ai.rs::local_completion`) so a local-only deployment
/// returns deterministic staged amendments. Non-prose-bearing (ADR D5): the
/// review-route stub keys on this header exactly like the mine header.
const REPLAN_AUDIT_HEADER: &str = "outline replanning audit";

/// Per-pass cap on staged amendments (ADR D5). Survivors beyond this are dropped
/// and counted toward `discarded_count`.
const REPLAN_STAGE_CAP: usize = 8;

/// Per-section cap keeping the realized/target digests bounded and deterministic
/// (mirrors `CANON_MINE_DIGEST_CAP`).
const REPLAN_DIGEST_CAP: usize = 30;

/// Char cap applied to every free-text field the digest surfaces (summaries,
/// synopses, list items, rationale echoes) — char-safe truncation keeps the
/// prompt lean and deterministic.
const REPLAN_TEXT_CAP: usize = 300;

/// Maximum characters a staged rationale may occupy (ADR D2). Matches the
/// repository staging guard; the differ truncates char-safe before staging so a
/// long model rationale never trips the guard.
const REPLAN_RATIONALE_MAX_CHARS: usize = 500;

/// Truncate every item of a string list char-safe to [`REPLAN_TEXT_CAP`],
/// dropping empty items and capping the count at [`REPLAN_DIGEST_CAP`]. Keeps
/// the realized-digest sections lean and deterministic.
fn cap_string_list(items: &[String]) -> Vec<String> {
    items
        .iter()
        .filter(|s| !s.trim().is_empty())
        .take(REPLAN_DIGEST_CAP)
        .map(|s| crate::format::truncate_at_chars(s, REPLAN_TEXT_CAP))
        .collect()
}

/// One source-chapter scene's realized beat annotation, digested for the replan
/// prompt (motif/theme/conflict ids + intensity — ids and numbers only, no
/// prose).
#[derive(Debug, Clone)]
struct ReplanBeatDigest {
    scene_order: i32,
    motif_ids: Vec<String>,
    theme_ids: Vec<String>,
    conflict_ids: Vec<String>,
    intensity: Option<f64>,
}

/// The realized digest for the source chapter: its saved-summary fields, beat
/// annotations, and the current promise/arc states — the reality the outline is
/// audited against. Summaries + metadata only (ADR D5); never scene prose.
#[derive(Debug, Clone)]
struct ReplanRealizedDigest {
    source_chapter: i32,
    summary: String,
    key_events: Vec<String>,
    character_changes: Vec<String>,
    relationship_shifts: Vec<String>,
    arc_advances: Vec<String>,
    promise_events: Vec<String>,
    beats: Vec<ReplanBeatDigest>,
    /// Current promise states: (id, status, urgency).
    promise_states: Vec<(String, String, String)>,
    /// Arc tracker states: (id, status, progress).
    arc_states: Vec<(String, String, f64)>,
}

/// One target chapter's plan, digested for the replan prompt. Planned scenes are
/// summarized by (order, summary, purpose, cast ids, location, rating); thread
/// targets are the theme/conflict/plot-line ids; `plan_revision` is the ADR D4
/// counter (NULL = 0).
#[derive(Debug, Clone)]
struct ReplanTargetDigest {
    chapter_number: i32,
    synopsis: String,
    scenes: Vec<ReplanTargetSceneDigest>,
    target_theme_ids: Vec<String>,
    target_conflict_ids: Vec<String>,
    target_plot_line_ids: Vec<String>,
    plan_revision: i64,
}

#[derive(Debug, Clone)]
struct ReplanTargetSceneDigest {
    scene_order: i32,
    summary: String,
    purpose: String,
    character_ids: Vec<String>,
    location_id: Option<String>,
    content_rating: Option<String>,
}

impl ReplanTargetDigest {
    /// Build the digest from a stored chapter plan. Free text is char-capped;
    /// scenes stay in the plan's stored spine order.
    fn from_plan(plan: &crate::sqlite::records::ChapterPlan) -> Self {
        let mut scenes: Vec<ReplanTargetSceneDigest> = plan
            .scenes
            .iter()
            .map(|s| ReplanTargetSceneDigest {
                scene_order: s.scene_order,
                summary: crate::format::truncate_at_chars(&s.summary, REPLAN_TEXT_CAP),
                purpose: crate::format::truncate_at_chars(&s.purpose, REPLAN_TEXT_CAP),
                character_ids: s.character_ids.clone(),
                location_id: s.location_id.clone(),
                content_rating: s.content_rating.as_ref().map(|r| r.as_str().to_string()),
            })
            .collect();
        scenes.sort_by_key(|s| s.scene_order);
        Self {
            chapter_number: plan.chapter_number,
            synopsis: crate::format::truncate_at_chars(&plan.synopsis, REPLAN_TEXT_CAP),
            scenes,
            target_theme_ids: plan.target_theme_ids.clone(),
            target_conflict_ids: plan.target_conflict_ids.clone(),
            target_plot_line_ids: plan.target_plot_line_ids.clone(),
            // ADR D4: NULL reads as revision 0.
            plan_revision: plan.plan_revision.unwrap_or(0),
        }
    }
}

/// Render the realized-reality section of the replan prompt deterministically.
/// Sections are emitted in a fixed order; empty ones are omitted so the prompt
/// stays lean. Pure — no I/O, unit-tested directly.
fn build_replan_realized_section(realized: &ReplanRealizedDigest) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "REALIZED REALITY of chapter {} (what the story ACTUALLY did — audit the future plans against THIS):\n",
        realized.source_chapter
    ));
    if !realized.summary.trim().is_empty() {
        out.push_str(&format!("Summary: {}\n", realized.summary));
    }
    let list_section = |out: &mut String, label: &str, items: &[String]| {
        if !items.is_empty() {
            out.push_str(&format!("{label}:\n"));
            for item in items {
                out.push_str(&format!("  - {item}\n"));
            }
        }
    };
    list_section(&mut out, "Key events", &realized.key_events);
    list_section(&mut out, "Character changes", &realized.character_changes);
    list_section(
        &mut out,
        "Relationship shifts",
        &realized.relationship_shifts,
    );
    list_section(&mut out, "Arc advances", &realized.arc_advances);
    list_section(&mut out, "Promise events", &realized.promise_events);

    if !realized.beats.is_empty() {
        out.push_str("Realized beat annotations (ids + intensity):\n");
        for beat in &realized.beats {
            out.push_str(&format!("  - scene {}", beat.scene_order));
            if !beat.motif_ids.is_empty() {
                out.push_str(&format!("; motifs: {}", beat.motif_ids.join(",")));
            }
            if !beat.theme_ids.is_empty() {
                out.push_str(&format!("; themes: {}", beat.theme_ids.join(",")));
            }
            if !beat.conflict_ids.is_empty() {
                out.push_str(&format!("; conflicts: {}", beat.conflict_ids.join(",")));
            }
            if let Some(intensity) = beat.intensity {
                out.push_str(&format!("; intensity: {intensity:.2}"));
            }
            out.push('\n');
        }
    }
    if !realized.promise_states.is_empty() {
        out.push_str("Current promise states (id, status, urgency):\n");
        for (id, status, urgency) in &realized.promise_states {
            out.push_str(&format!("  - {id} ({status}, {urgency})\n"));
        }
    }
    if !realized.arc_states.is_empty() {
        out.push_str("Arc tracker states (id, status, progress):\n");
        for (id, status, progress) in &realized.arc_states {
            out.push_str(&format!("  - {id} ({status}, {progress:.2})\n"));
        }
    }
    out
}

/// Render the target-plans section of the replan prompt deterministically. Each
/// undrafted future chapter's plan is emitted in ascending chapter order. Pure.
fn build_replan_targets_section(targets: &[ReplanTargetDigest]) -> String {
    let mut out = String::new();
    out.push_str(
        "FUTURE CHAPTER PLANS to audit (propose amendments ONLY against these chapter numbers):\n",
    );
    for target in targets {
        out.push_str(&format!(
            "Chapter {} (plan_revision {}):\n",
            target.chapter_number, target.plan_revision
        ));
        out.push_str(&format!("  Synopsis: {}\n", target.synopsis));
        if !target.target_theme_ids.is_empty() {
            out.push_str(&format!(
                "  Target themes: {}\n",
                target.target_theme_ids.join(",")
            ));
        }
        if !target.target_conflict_ids.is_empty() {
            out.push_str(&format!(
                "  Target conflicts: {}\n",
                target.target_conflict_ids.join(",")
            ));
        }
        if !target.target_plot_line_ids.is_empty() {
            out.push_str(&format!(
                "  Target plot lines: {}\n",
                target.target_plot_line_ids.join(",")
            ));
        }
        if target.scenes.is_empty() {
            out.push_str("  Planned scenes: (none)\n");
        } else {
            out.push_str("  Planned scenes:\n");
            for scene in &target.scenes {
                out.push_str(&format!(
                    "    - order {} — {} (purpose: {})",
                    scene.scene_order, scene.summary, scene.purpose
                ));
                if !scene.character_ids.is_empty() {
                    out.push_str(&format!("; cast: {}", scene.character_ids.join(",")));
                }
                if let Some(location) = &scene.location_id {
                    out.push_str(&format!("; location: {location}"));
                }
                if let Some(rating) = &scene.content_rating {
                    out.push_str(&format!("; rating: {rating}"));
                }
                out.push('\n');
            }
        }
    }
    out
}

/// The static output-contract block: the exact JSON envelope plus the per-class
/// payload field lists (ADR 0003 D1). Spelled out so the model never has to
/// guess a payload shape; each shape is the minimal delta against the plan-write
/// input the class applies through (validated by [`validate_plan_amendment`]).
fn replan_output_contract() -> &'static str {
    "OUTPUT CONTRACT — return STRICT JSON only, no prose, this exact envelope:\n\
     {\"amendments\":[{\"amendment_class\":\"...\",\"target_chapter\":N|null,\"confidence\":\"high|medium|low\",\"rationale\":\"<why, referencing realized events by id>\",\"payload\":{...}}]}\n\
     `target_chapter` is a future chapter number for every class EXCEPT promise_followup (which uses null — it targets a placement).\n\
     `rationale` is mandatory and cites realized ids/events only — NO prose quotes.\n\
     Per-class payload fields (use EXACTLY these keys):\n\
     - synopsis_update: synopsis\n\
     - scene_add: scene_order, summary, purpose, character_ids[], location_id?, content_rating?, beat_structure?[], insert_at\n\
     - scene_drop: scene_order\n\
     - scene_replace: scene_order, summary, purpose, character_ids[], location_id?, content_rating?\n\
     - scene_reorder: new_order (array of existing scene_orders, a permutation)\n\
     - thread_promote: kind (theme|conflict|plot_line), id\n\
     - thread_retire: kind (theme|conflict|plot_line), id\n\
     - promise_followup: promise_type, description, planted_at_placement {book_number, chapter_number, scene_order?}, planned_payoff? {book_number, chapter_number, scene_order?}"
}

/// Assemble the full replan prompt from the header/instructions, the realized
/// digest, the target plans, and the output contract. Pure — the service builds
/// `realized`/`targets`.
fn build_replan_prompt(realized: &ReplanRealizedDigest, targets: &[ReplanTargetDigest]) -> String {
    let mut prompt = String::new();
    prompt.push_str(REPLAN_AUDIT_HEADER);
    prompt.push('\n');
    prompt.push_str(
        "You are auditing a book's not-yet-drafted chapter plans against the reality one\n\
         just-completed chapter established. Propose plan amendments ONLY where a future plan\n\
         has drifted from what actually happened. Never rewrite a drafted chapter; never invent\n\
         reality not present below. Prefer no amendment over a speculative one.\n\n",
    );
    prompt.push_str(&build_replan_realized_section(realized));
    prompt.push('\n');
    prompt.push_str(&build_replan_targets_section(targets));
    prompt.push('\n');
    prompt.push_str(replan_output_contract());
    prompt
}

/// One raw amendment as emitted by the model, before validation. Fields mirror
/// the output contract; unknown extras are ignored.
#[derive(Debug, Clone, serde::Deserialize)]
struct RawPlanAmendment {
    amendment_class: String,
    #[serde(default)]
    target_chapter: Option<i32>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RawReplanOutput {
    #[serde(default)]
    amendments: Vec<RawPlanAmendment>,
}

/// Parse the differ's strict-JSON output. Tolerates code fences / surrounding
/// prose via [`extract_json_object`]; a genuinely malformed payload is an error
/// the caller maps to `model_output_rejected` (never a guess). Mirrors
/// [`parse_canon_mine_output`].
fn parse_replan_output(output: &str) -> anyhow::Result<Vec<RawPlanAmendment>> {
    let trimmed = output.trim();
    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some(fenced) = extract_json_object(output) {
        fenced
    } else {
        trimmed.to_string()
    };
    let parsed: RawReplanOutput =
        serde_json::from_str(&candidate).context("parsing replan model output as strict JSON")?;
    Ok(parsed.amendments)
}

/// Whether a per-class payload is structurally sound (ADR 0003 D1). Each arm
/// checks only the minimal keys Part B's apply dispatcher needs to replay the
/// class against the plan-write input — it does NOT resolve ids against the
/// bible (Part B does at apply). Returns `true` when the payload deserializes
/// into the class's minimal delta, `false` (discard) otherwise.
///
/// Payload shapes (Part B's contract — precise):
///   * `synopsis_update` — `{synopsis: String (non-empty)}`
///   * `scene_add` — the full planned-scene shape plus an insert point:
///     `{scene_order: i32, summary: String, purpose: String, character_ids: [String],
///       location_id?: String, content_rating?: String, beat_structure?: [String],
///       insert_at: i32}`
///   * `scene_drop` — `{scene_order: i32}`
///   * `scene_replace` — the mutable planned-scene fields keyed by order:
///     `{scene_order: i32, summary: String, purpose: String, character_ids: [String],
///       location_id?: String, content_rating?: String}`
///   * `scene_reorder` — `{new_order: [i32] (non-empty permutation of scene orders)}`
///   * `thread_promote` / `thread_retire` —
///     `{kind: "theme"|"conflict"|"plot_line", id: String (non-empty)}`
///   * `promise_followup` — the `create_narrative_promise` delta:
///     `{promise_type: String, description: String,
///       planted_at_placement: {book_number: i32, chapter_number: i32, scene_order?: i32},
///       planned_payoff?: {book_number: i32, chapter_number: i32, scene_order?: i32}}`
fn plan_amendment_payload_is_valid(class: &str, payload: &serde_json::Value) -> bool {
    fn nonempty_str(v: &serde_json::Value, key: &str) -> bool {
        v.get(key)
            .and_then(|f| f.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }
    fn is_i64(v: &serde_json::Value, key: &str) -> bool {
        v.get(key).and_then(|f| f.as_i64()).is_some()
    }
    fn placement_ok(v: &serde_json::Value) -> bool {
        // Minimal StoryPlacement: book_number + chapter_number required ints;
        // scene_order optional int (ignored if absent/null).
        is_i64(v, "book_number") && is_i64(v, "chapter_number")
    }
    match class {
        "synopsis_update" => nonempty_str(payload, "synopsis"),
        "scene_add" => {
            is_i64(payload, "scene_order")
                && nonempty_str(payload, "summary")
                && nonempty_str(payload, "purpose")
                && payload
                    .get("character_ids")
                    .map(|c| c.is_array())
                    .unwrap_or(false)
                && is_i64(payload, "insert_at")
        }
        "scene_drop" => is_i64(payload, "scene_order"),
        "scene_replace" => {
            is_i64(payload, "scene_order")
                && nonempty_str(payload, "summary")
                && nonempty_str(payload, "purpose")
                && payload
                    .get("character_ids")
                    .map(|c| c.is_array())
                    .unwrap_or(false)
        }
        "scene_reorder" => payload
            .get("new_order")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty() && a.iter().all(|e| e.as_i64().is_some()))
            .unwrap_or(false),
        "thread_promote" | "thread_retire" => {
            let kind = payload
                .get("kind")
                .and_then(|v| v.as_str())
                .map(|k| k.trim());
            matches!(kind, Some("theme" | "conflict" | "plot_line")) && nonempty_str(payload, "id")
        }
        "promise_followup" => {
            nonempty_str(payload, "promise_type")
                && nonempty_str(payload, "description")
                && payload
                    .get("planted_at_placement")
                    .map(placement_ok)
                    .unwrap_or(false)
                && match payload.get("planned_payoff") {
                    None | Some(serde_json::Value::Null) => true,
                    Some(payoff) => placement_ok(payoff),
                }
        }
        _ => false,
    }
}

/// Validate one raw amendment against the ADR 0003 D5 discard discipline.
/// Returns the staged tuple `(amendment_class, target_chapter, confidence,
/// rationale, payload)` on success, or `None` when the amendment is discarded:
///   * `amendment_class ∉ PLAN_AMENDMENT_CLASSES` (unknown → discard);
///   * rationale empty (whitespace-only) after trim (the differ's evidence
///     discipline — an amendment with no stated reasoning is not stageable). The
///     stored rationale is then truncated char-safe to ≤500;
///   * `target_chapter` missing for a chapter-targeting class, or present for
///     `promise_followup` (forbidden-none);
///   * `target_chapter` present but NOT in `eligible_chapters` (the differ never
///     stages against a chapter outside the audited set);
///   * payload failing [`plan_amendment_payload_is_valid`] for the class.
///
/// `confidence` normalizes to `high|medium|low`, defaulting to `low` when the
/// model omits/garbles it (a low-confidence proposal is still stageable).
fn validate_plan_amendment(
    raw: &RawPlanAmendment,
    eligible_chapters: &std::collections::BTreeSet<i32>,
) -> Option<(String, Option<i32>, String, String, serde_json::Value)> {
    if !spindle_core::models::is_plan_amendment_class(&raw.amendment_class) {
        return None;
    }
    let rationale = raw.rationale.as_deref().unwrap_or("").trim();
    if rationale.is_empty() {
        return None;
    }
    // Target-chapter rules per class (mirror the repository staging guard so an
    // amendment that would be rejected at stage is discarded here first).
    if raw.amendment_class == "promise_followup" {
        if raw.target_chapter.is_some() {
            return None;
        }
    } else {
        match raw.target_chapter {
            Some(chapter) if eligible_chapters.contains(&chapter) => {}
            _ => return None,
        }
    }
    if !plan_amendment_payload_is_valid(&raw.amendment_class, &raw.payload) {
        return None;
    }
    let confidence = match raw
        .confidence
        .as_deref()
        .map(|c| c.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("high") => "high",
        Some("medium") => "medium",
        _ => "low",
    }
    .to_string();
    let stored_rationale = crate::format::truncate_at_chars(rationale, REPLAN_RATIONALE_MAX_CHARS);
    Some((
        raw.amendment_class.clone(),
        raw.target_chapter,
        confidence,
        stored_rationale,
        raw.payload.clone(),
    ))
}

/// Build the honest-skip `ReplanChapterOutput` for a route failure. A
/// RatingNotCovered names route+rating (never applicable here — the route is
/// non-prose-bearing — but handled for symmetry); any other error reads as a
/// dead-route skip naming the attempted route. Never carries prose.
fn replan_skip_output(
    err: &anyhow::Error,
    attempted_route: &str,
) -> spindle_core::models::ReplanChapterOutput {
    let skip_reason = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => {
            format!("replan SKIPPED: the `{route}` route is not cleared for rating `{rating}`")
        }
        _ => format!("replan SKIPPED: no usable `{attempted_route}` route (the model call failed)"),
    };
    spindle_core::models::ReplanChapterOutput {
        staged: Vec::new(),
        discarded_count: 0,
        superseded_count: 0,
        status: "skipped".to_string(),
        skip_reason: Some(skip_reason),
    }
}

// ── Canon miner (mine_scene_canon — evolution §3.1, ADR 0001) ────────────────

/// The distinctive header line the mining prompt leads with. Doubles as the
/// fake-router marker (`ai.rs::local_completion`) so a local-only deployment
/// returns deterministic staged deltas.
const CANON_MINE_AUDIT_HEADER: &str = "canon mining audit";

/// Maximum characters an evidence quote may occupy in a staged row (ADR 0001
/// D2). The verbatim substring check runs on the UNtruncated quote; truncation
/// is applied only for storage.
const CANON_MINE_EVIDENCE_MAX_CHARS: usize = 300;

/// Per-section caps keeping the current-state digest bounded and deterministic.
const CANON_MINE_DIGEST_CAP: usize = 30;
const CANON_MINE_PROMISE_DESC_CAP: usize = 80;

/// One raw delta as emitted by the model, before validation. Fields mirror the
/// output contract spelled into the prompt; unknown extras are ignored.
#[derive(Debug, Clone, serde::Deserialize)]
struct RawCanonDelta {
    delta_class: String,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RawCanonMineOutput {
    #[serde(default)]
    deltas: Vec<RawCanonDelta>,
}

/// Parse the miner's strict-JSON output. Tolerates code fences / surrounding
/// prose via [`extract_json_object`]; a genuinely malformed payload is an error
/// the caller maps to `model_output_rejected` (never a guess).
fn parse_canon_mine_output(output: &str) -> anyhow::Result<Vec<RawCanonDelta>> {
    let trimmed = output.trim();
    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some(fenced) = extract_json_object(output) {
        fenced
    } else {
        trimmed.to_string()
    };
    let parsed: RawCanonMineOutput = serde_json::from_str(&candidate)
        .context("parsing canon-mine model output as strict JSON")?;
    Ok(parsed.deltas)
}

/// Validate one raw delta against the ADR lifecycle (M3). Returns the staged
/// tuple `(delta_class, target_id, confidence, evidence, payload)` on success,
/// or `None` when the delta is discarded:
///   * `delta_class ∉ CANON_DELTA_CLASSES` (unknown → discard);
///   * evidence empty, or NOT a verbatim substring of the scene prose (the
///     model does not get to fabricate evidence). The substring check runs on
///     the UNtruncated quote; the stored evidence is then truncated char-safe
///     to ≤300;
///   * `entity_candidate` payload missing a `kind ∈ {character, location, term}`.
///
/// `confidence` normalizes to `high|medium|low`, defaulting to `low` when the
/// model omits/garbles it (a low-confidence proposal is still stageable).
/// Prefix a bare record key with its class's table (defect item 4): the miner
/// emits `"01KT..."` where the repository stores `"conflict:01KT..."`. Ids that
/// already carry any `table:` prefix pass through untouched, so an operator
/// edit is never re-prefixed.
fn canon_prefix_id(id: &str, table: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() || trimmed.contains(':') {
        trimmed.to_string()
    } else {
        format!("{table}:{trimmed}")
    }
}

/// Prefix a bare id sitting at `key` in a payload object (see [`canon_prefix_id`]).
fn canon_prefix_payload_id(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    table: &str,
) {
    if let Some(serde_json::Value::String(id)) = object.get(key) {
        let prefixed = canon_prefix_id(id, table);
        if &prefixed != id {
            object.insert(key.to_string(), serde_json::Value::String(prefixed));
        }
    }
}

/// Map a plural table/kind name the miner emits (`"characters"`) onto the
/// singular the DTOs and record ids use (`"character"`). Unknown names pass
/// through lowercased so genuinely wrong values still fail loudly downstream.
fn canon_singularize_table(table: &str) -> String {
    let lower = table.trim().to_ascii_lowercase();
    match lower.as_str() {
        "characters" => "character",
        "locations" => "location",
        "scenes" => "scene",
        "factions" => "faction",
        "terms" => "term",
        "conflicts" => "conflict",
        "themes" => "theme",
        "motifs" => "motif",
        "religions" => "religion",
        "economies" => "economy",
        "plot_lines" => "plot_line",
        "world_rules" => "world_rule",
        "relationships" => "relationship",
        "projects" => "project",
        "books" => "book",
        "chapters" => "chapter",
        _ => return lower,
    }
    .to_string()
}

/// Move `from` to `to` when `to` is absent/null — the miner says `description`
/// where a create DTO wants `summary`/`definition` (defect item 4).
fn canon_rename_missing(
    object: &mut serde_json::Map<String, serde_json::Value>,
    from: &str,
    to: &str,
) {
    let to_missing = matches!(object.get(to), None | Some(serde_json::Value::Null));
    if to_missing
        && let Some(value) = object.remove(from)
        && !value.is_null()
    {
        object.insert(to.to_string(), value);
    }
}

/// Remove an explicit-null key so the DTO's serde default applies.
fn canon_drop_null(object: &mut serde_json::Map<String, serde_json::Value>, key: &str) {
    if matches!(object.get(key), Some(serde_json::Value::Null)) {
        object.remove(key);
    }
}

fn validate_canon_delta(
    raw: &RawCanonDelta,
    scene_prose: &str,
) -> Option<(String, Option<String>, String, String, serde_json::Value)> {
    if !spindle_core::models::is_canon_delta_class(&raw.delta_class) {
        return None;
    }
    let evidence = raw.evidence.as_deref().unwrap_or("").trim();
    if evidence.is_empty() || !scene_prose.contains(evidence) {
        return None;
    }
    if raw.delta_class == "entity_candidate" {
        let kind = raw
            .payload
            .get("kind")
            .and_then(|v| v.as_str())
            .map(|k| k.trim().to_ascii_lowercase());
        if !matches!(kind.as_deref(), Some("character" | "location" | "term")) {
            return None;
        }
    }
    let confidence = match raw
        .confidence
        .as_deref()
        .map(|c| c.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("high") => "high",
        Some("medium") => "medium",
        _ => "low",
    }
    .to_string();
    let stored_evidence = crate::format::truncate_at_chars(evidence, CANON_MINE_EVIDENCE_MAX_CHARS);
    Some((
        raw.delta_class.clone(),
        raw.target_id.clone().filter(|s| !s.trim().is_empty()),
        confidence,
        stored_evidence,
        raw.payload.clone(),
    ))
}

/// Build the honest-skip `MineSceneCanonOutput` for a route failure. A
/// RatingNotCovered names route+rating (ids only, never prose); any other error
/// reads as a dead-route skip naming the attempted route.
fn mine_skip_output(
    err: &anyhow::Error,
    attempted_route: &str,
) -> spindle_core::models::MineSceneCanonOutput {
    let skip_reason = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => format!(
            "canon mining SKIPPED: the `{route}` route is not cleared for rating `{rating}`"
        ),
        _ => match err.downcast_ref::<crate::ai::ContentPolicyRefusal>() {
            Some(refusal) => format!(
                "canon mining SKIPPED: the `{}` provider refused this scene on content-policy grounds ({})",
                refusal.endpoint, refusal.detail
            ),
            None => format!(
                "canon mining SKIPPED: no usable `{attempted_route}` route (the model call failed)"
            ),
        },
    };
    spindle_core::models::MineSceneCanonOutput {
        staged: Vec::new(),
        discarded_count: 0,
        superseded_count: 0,
        status: "skipped".to_string(),
        skip_reason: Some(skip_reason),
    }
}

/// Distinctive header for the cumulative reader-simulation prompt (evolution
/// §3.6). Doubles as the marker the local `review` stub keys on to return
/// deterministic reader-sim JSON in tests / local-only runs, exactly like the
/// temporal / promise-payoff / secret-leak / scene-purpose headers.
const READER_SIM_HEADER: &str = "cumulative reader simulation";

/// The first-N-chars of the prior-notes block that a reader-sim prompt exposes
/// to the memory-echo test sentinel. Keeping this a named constant lets the
/// local stub and the test agree on exactly how much the `PRIOR:` marker echoes.
pub const READER_SIM_PRIOR_ECHO_CHARS: usize = 40;

/// Build the cumulative reader-simulation prompt (evolution §3.6). The header is
/// the deterministic marker the local `review` stub keys on. The persona derives
/// from the project's reader contract ("you are the reader this book
/// promises…"); the prior notes carry the reader's rolling memory; the prose
/// block is the chapter's scenes in spine order. Strict JSON out.
fn build_reader_sim_prompt(
    contract: &spindle_core::models::ReaderContract,
    prior_notes: &str,
    prose: &str,
) -> String {
    let style = if contract.style_notes.is_empty() {
        "(none stated)".to_string()
    } else {
        contract.style_notes.join("; ")
    };
    let boundaries = if contract.boundaries.is_empty() {
        "(none stated)".to_string()
    } else {
        contract.boundaries.join("; ")
    };
    let prior = if prior_notes.trim().is_empty() {
        "(no prior notes — this is the first chapter you have read)".to_string()
    } else {
        prior_notes.to_string()
    };
    format!(
        "You are performing a {header}.\n\
         You are the reader this book promises: {promise}\n\
         Style you were promised: {style}\n\
         Boundaries you were promised: {boundaries}\n\
         You read chapters IN ORDER and you REMEMBER. Below are your cumulative \
         notes from everything you have read so far, then the next chapter's prose \
         in spine order. Read the new chapter as this reader, update your memory, \
         and report your engagement and any craft concerns.\n\n\
         YOUR PRIOR NOTES:\n{prior}\n\n\
         NEXT CHAPTER (scenes in spine order):\n{prose}\n\n\
         Return strict JSON only, this exact shape:\n\
         {{\"engagement\":\"high|steady|dipping\",\"notes\":\"<your cumulative, \
         self-contained replacement notes — compress if long>\",\"concerns\":\
         [{{\"severity\":\"info|warning\",\"description\":\"<a craft observation>\"}}]}}\n\
         `notes` REPLACES your prior notes (self-contained, not a diff). Raise a \
         concern only for a real craft signal (engagement dip, retread, unpaid \
         setup); an empty concerns array is fine. Also return `open_questions`, an array of \
         at most 12 unresolved reader questions, preserving older questions until the prose \
         answers them. Keep those separate from your replacement notes. Missing chapters \
         are unknown; never invent events to fill a gap.",
        header = READER_SIM_HEADER,
        promise = contract.promise,
        style = style,
        boundaries = boundaries,
        prior = prior,
        prose = prose,
    )
}

/// Parsed reader-sim model output (evolution §3.6). Only the fields the contract
/// spells out; unknown extras are ignored.
#[derive(Debug, Clone, serde::Deserialize)]
struct RawReaderSimOutput {
    #[serde(default)]
    open_questions: Option<Vec<String>>,
    engagement: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    concerns: Vec<RawReaderSimConcern>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct RawReaderSimConcern {
    severity: String,
    #[serde(default)]
    description: String,
}

/// The normalized reader-sim read (engagement + notes + validated concerns).
struct ParsedReaderSim {
    open_questions: Option<Vec<String>>,
    engagement: String,
    notes: String,
    concerns: Vec<spindle_core::models::ReaderSimConcern>,
}

/// Strict-parse the reader-sim model output (evolution §3.6). Tolerates code
/// fences / surrounding prose via [`extract_json_object`], normalizes the
/// engagement to one of `high`/`steady`/`dipping` and each concern severity to
/// `info`/`warning` (an out-of-vocabulary value is coerced to the safe end:
/// unknown engagement → `dipping`, unknown severity → `info`). Returns `None`
/// on unparseable output so the caller keeps prior notes and never guesses (I8).
fn parse_reader_sim_output(output: &str) -> Option<ParsedReaderSim> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return None;
    }
    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else {
        extract_json_object(output)?
    };
    let raw: RawReaderSimOutput = serde_json::from_str(&candidate).ok()?;
    let engagement = match raw.engagement.trim().to_ascii_lowercase().as_str() {
        "high" => "high",
        "steady" => "steady",
        // Any other value reads as the reader disengaging — coerce to the
        // signal-bearing end so an odd label never silently reads as fine.
        _ => "dipping",
    }
    .to_string();
    let concerns = raw
        .concerns
        .into_iter()
        .map(|c| spindle_core::models::ReaderSimConcern {
            severity: match c.severity.trim().to_ascii_lowercase().as_str() {
                "warning" => "warning".to_string(),
                _ => "info".to_string(),
            },
            description: c.description,
        })
        .collect();
    Some(ParsedReaderSim {
        open_questions: raw.open_questions,
        engagement,
        notes: raw.notes,
        concerns,
    })
}

/// Honest reader-sim skip (evolution §3.6, I8). Names the route+rating that was
/// uncleared, or the transport failure — never any prose. A skip never reads as
/// a clean pass.
fn reader_sim_skip_outcome(
    err: &anyhow::Error,
    attempted_route: &str,
) -> spindle_core::models::ReaderSimChapterOutcome {
    let skip_reason = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => format!(
            "reader simulation SKIPPED: the `{route}` route is not cleared for rating `{rating}`"
        ),
        _ => format!(
            "reader simulation SKIPPED: no usable `{attempted_route}` route (the model call failed)"
        ),
    };
    spindle_core::models::ReaderSimChapterOutcome {
        open_questions: None,
        status: "skipped".to_string(),
        engagement: "skipped".to_string(),
        notes: String::new(),
        concerns: Vec::new(),
        skip_reason: Some(skip_reason),
    }
}

/// The compact current-state digest fed to the miner so it proposes UPDATES
/// against real ids instead of duplicating canon. Populated by the service AFTER
/// secret-gating (so a withheld fact key never reaches [`build_mine_digest`]),
/// then rendered by that pure builder — keeping the render deterministic and
/// unit-testable in isolation.
#[derive(Debug, Clone, Default)]
struct MineDigest {
    /// Scene cast: (character id, name).
    cast: Vec<(String, String)>,
    /// Open promises: (id, description ≤cap, status).
    open_promises: Vec<(String, String, String)>,
    /// Active conflicts: (id, name, undelivered consequence indexes,
    /// undemonstrated escalation-stage indexes).
    active_conflicts: Vec<(String, String, Vec<usize>, Vec<usize>)>,
    /// Arcs for cast: (id, unreached milestone labels).
    arcs: Vec<(String, Vec<String>)>,
    /// Existing canonical-fact keys per cast subject: (subject name, keys). Only
    /// visible (non-withheld) keys ever appear here.
    fact_keys: Vec<(String, Vec<String>)>,
}

/// Render the current-state digest deterministically. Sections are emitted in a
/// fixed order; each is capped by the caller; empty sections are omitted so the
/// prompt stays lean. Pure — no I/O, unit-tested directly.
fn build_mine_digest(digest: &MineDigest) -> String {
    let mut out = String::new();
    out.push_str("CURRENT STATE (propose UPDATES against these real ids; do not duplicate):\n");

    if !digest.cast.is_empty() {
        out.push_str("Cast:\n");
        for (id, name) in &digest.cast {
            out.push_str(&format!("  - {name} [{id}]\n"));
        }
    }
    if !digest.open_promises.is_empty() {
        out.push_str("Open promises (payoff/reinforce must reference a real id):\n");
        for (id, desc, status) in &digest.open_promises {
            out.push_str(&format!("  - {id} ({status}): {desc}\n"));
        }
    }
    if !digest.active_conflicts.is_empty() {
        out.push_str("Active conflicts:\n");
        for (id, name, undelivered, undemonstrated) in &digest.active_conflicts {
            out.push_str(&format!("  - {id} ({name})"));
            if !undelivered.is_empty() {
                out.push_str(&format!(
                    "; undelivered consequence indexes: {}",
                    undelivered
                        .iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            if !undemonstrated.is_empty() {
                out.push_str(&format!(
                    "; undemonstrated escalation stage indexes: {}",
                    undemonstrated
                        .iter()
                        .map(|i| i.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
            out.push('\n');
        }
    }
    if !digest.arcs.is_empty() {
        out.push_str("Arcs (unreached milestones):\n");
        for (id, labels) in &digest.arcs {
            out.push_str(&format!("  - {id}: {}\n", labels.join("; ")));
        }
    }
    if !digest.fact_keys.is_empty() {
        out.push_str("Existing canonical-fact keys (values withheld):\n");
        for (subject, keys) in &digest.fact_keys {
            out.push_str(&format!("  - {subject}: {}\n", keys.join(", ")));
        }
    }
    out
}

/// The static output-contract block: the exact JSON envelope plus the per-class
/// payload field lists (ADR 0001 D1). Spelled out so the model never has to
/// guess a payload shape.
fn canon_mine_output_contract() -> &'static str {
    "OUTPUT CONTRACT — return STRICT JSON only, no prose, this exact envelope:\n\
     {\"deltas\":[{\"delta_class\":\"...\",\"target_id\":null|\"<id>\",\"confidence\":\"high|medium|low\",\"evidence\":\"<verbatim quote>\",\"payload\":{...}}]}\n\
     Every item MUST carry a short VERBATIM evidence quote copied from the prose above.\n\
     Per-class payload fields (use EXACTLY these keys):\n\
     - canonical_fact: subject_table, subject_id?, predicate, value_text|value_number, unit?, scope?\n\
     - promise_planted: promise_type, description, planted_at, planned_payoff?\n\
     - promise_payoff_candidate: narrative_promise_id, proposed_status=\"paid_off\", payoff_scene_ref?\n\
     - promise_reinforced: narrative_promise_id, proposed_status=\"reinforced\"\n\
     - relationship_shift: relationship_id, trust_delta?, tension_delta?, dynamics_note?\n\
     - character_state: character_id, state fields, position stamp\n\
     - knowledge_learned: character_id, fact, learned_at, secret_of_fact_id?\n\
     - beat_annotation: motif_ids?, theme_ids?, conflict_ids?, intensity?\n\
     - try_fail_cycle: conflict_id, cycle description\n\
     - consequence_delivered: conflict_id, consequence_index, delivered=true\n\
     - escalation_demonstrated: conflict_id, stage_index, demonstrated_at\n\
     - arc_milestone_reached: character_arc_id, milestone_label, reached_at\n\
     - quantity_change: character_id, measure, amount|band, change_reason\n\
     - entity_candidate: kind (character|location|term) + the create tool's input"
}

/// Assemble the full mining prompt from the header/instructions, the scene
/// prose, the current-state digest, the optional quantity-scheme guidance, and
/// the output contract. Pure — the service builds `digest`/`scheme_name`.
fn build_mine_prompt(scene_text: &str, digest: &str, quantity_scheme_name: Option<&str>) -> String {
    use crate::sqlite::import::prompts::FICTION_FRAME_PREFIX;

    let mut prompt = String::new();
    // Fiction/safety frame first: mining ships the scene's verbatim prose plus a
    // mature-adjacent output contract (relationship_shift/tension_delta/…), which
    // stricter provider classifiers (e.g. Qwen) can read as real-world content
    // without the frame the review/research routes already carry.
    prompt.push_str(FICTION_FRAME_PREFIX);
    prompt.push_str(CANON_MINE_AUDIT_HEADER);
    prompt.push('\n');
    prompt.push_str(
        "You are mining ONE committed scene into proposed canon deltas for operator review.\n\
         Extract ONLY what the prose on the page ESTABLISHES — never infer, never invent.\n\
         Every item MUST carry a short verbatim evidence quote copied from the prose.\n\
         Prefer UPDATES against the real ids in the current-state digest over new proposals.\n\n",
    );
    prompt.push_str("SCENE PROSE:\n");
    prompt.push_str(scene_text);
    prompt.push_str("\n\n");
    if !digest.trim().is_empty() {
        prompt.push_str(digest);
        prompt.push('\n');
    }
    if let Some(name) = quantity_scheme_name {
        prompt.push_str(&format!(
            "Quantity scheme in play: `{name}` — use quantity_change deltas for measured amounts of this scheme.\n\n"
        ));
    }
    prompt.push_str(canon_mine_output_contract());
    prompt
}

fn extract_json_object(output: &str) -> Option<String> {
    let trimmed = output.trim();
    let without_fence = if trimmed.starts_with("```") {
        trimmed
            .lines()
            .skip_while(|line| line.trim_start().starts_with("```"))
            .take_while(|line| !line.trim_start().starts_with("```"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        trimmed.to_string()
    };

    // grok-4.5 narrates before/between tool calls and places the JSON payload
    // at the tail (BUG 2). Recover the last balanced top-level object, which is
    // robust to braces in the narration and to multiple objects (takes the last
    // complete one). Returns None only when no balanced object exists.
    spindle_core::model_output::extract_trailing_json_object(&without_fence).map(str::to_string)
}

#[cfg(test)]
mod extract_json_object_tests {
    use super::extract_json_object;

    #[test]
    fn recovers_trailing_json_after_grok_narration() {
        // grok narration before the payload — the concatenation the live run
        // fed to serde and got `expected value at line 1 column 1`.
        let output = "I'll pull Spindle canon first, then audit.\n\n{\"deltas\":[]}";
        assert_eq!(
            extract_json_object(output).as_deref(),
            Some("{\"deltas\":[]}")
        );
    }

    #[test]
    fn narration_with_a_brace_does_not_start_the_span_too_early() {
        // The old find('{')..rfind('}') span would start at the narration brace
        // and produce an unparseable slice; tail-first ignores it.
        let output = "note: the set is {incomplete here\nNow the answer:\n{\"findings\":[]}";
        assert_eq!(
            extract_json_object(output).as_deref(),
            Some("{\"findings\":[]}")
        );
    }

    #[test]
    fn takes_the_last_complete_object_when_multiple_present() {
        let output = "{\"a\":1} chatter about tools {\"deltas\":[]}";
        assert_eq!(
            extract_json_object(output).as_deref(),
            Some("{\"deltas\":[]}")
        );
    }

    #[test]
    fn still_extracts_from_code_fence() {
        let output = "```json\n{\"matches\":[]}\n```";
        assert_eq!(
            extract_json_object(output).as_deref(),
            Some("{\"matches\":[]}")
        );
    }

    #[test]
    fn none_when_no_object_present() {
        assert_eq!(extract_json_object("no json here at all"), None);
    }
}

fn heuristic_world_rule_violations(
    scene: &crate::sqlite::records::Scene,
    rules: &[&crate::sqlite::records::WorldRule],
) -> Vec<DeepWorldRuleViolation> {
    let haystack = format!("{} {}", scene.summary, scene.full_text).to_lowercase();
    let uses_magic = [
        "magic", "spell", "cast", "sigil", "ward", "rune", "hex", "oath",
    ]
    .iter()
    .any(|needle| haystack.contains(needle));
    let remote_action = [
        "across the room",
        "from across the room",
        "from a distance",
        "at a distance",
        "without touching",
        "without contact",
    ]
    .iter()
    .any(|needle| haystack.contains(needle));

    rules
        .iter()
        .filter_map(|rule| {
            let rule_text =
                format!("{} {} {}", rule.rule_name, rule.rule_type, rule.description).to_lowercase();
            let requires_contact = [
                "physical contact",
                "requires contact",
                "requires physical contact",
                "requires touch",
                "must touch",
                "touch the target",
            ]
            .iter()
            .any(|needle| rule_text.contains(needle));

            if requires_contact && uses_magic && remote_action {
                Some(DeepWorldRuleViolation {
                    rule_id: rule.id.clone(),
                    severity: Some("warning".to_string()),
                    message: "the scene shows magic or a rule-bound action being performed at range even though the rule requires physical contact".to_string(),
                    evidence: Some(
                        "the scene describes a spell-like action happening across the room or without touch"
                            .to_string(),
                    ),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Output shape returned by the model router for the intra-scene temporal
/// coherence deep check (Tier 2).
#[derive(Debug, Clone, serde::Deserialize)]
struct DeepTemporalCheckOutput {
    #[serde(default)]
    findings: Vec<DeepTemporalFinding>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DeepTemporalFinding {
    message: String,
    #[serde(default)]
    severity: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

/// Build the prompt for the model-backed intra-scene temporal-coherence audit.
/// The "intra-scene temporal-coherence audit" header is also the marker the
/// local stub keys on to return deterministic JSON in tests / local-only runs.
fn build_temporal_deep_check_prompt(scene: &crate::sqlite::records::Scene) -> String {
    format!(
        "You are performing an intra-scene temporal-coherence audit for a single novel scene.\n\
         Read ONLY this scene's prose and find places where in-world time is mishandled WITHIN the scene:\n\
         - an unsignaled jump in the time of day or elapsed time (e.g. it is morning, then with no transition it is night);\n\
         - prose that contradicts its own established time of day;\n\
         - a long span of in-world time that passes with no transition beat or scene break to render it.\n\
         Judge by what the prose IMPLIES — meals, changing light, errands, fatigue, idioms like \"three cigarettes later\" — not only explicit clock or part-of-day words.\n\
         Do NOT flag a jump the prose already signals with a transition beat, a scene break, or a clearly-marked flashback or recollection.\n\
         Return strict JSON only.\n\n\
         Scene id: {}\n\
         Scene tone: {}\n\
         Scene text:\n{}\n\n\
         Return this exact shape:\n\
         {{\"findings\":[{{\"severity\":\"warning\",\"message\":\"...\",\"evidence\":\"...\"}}]}}\n\
         Use severity \"warning\" for every finding. Return an empty findings array when the scene's time handling is coherent.",
        scene.id,
        scene.tone.as_deref().unwrap_or("unspecified"),
        scene.full_text,
    )
}

/// Parse the model router's temporal deep-check output. Tolerates code fences
/// and surrounding prose via [`extract_json_object`]; an unparseable response
/// (e.g. the default local stub) yields no findings.
fn parse_deep_temporal_check_output(output: &str) -> anyhow::Result<Vec<DeepTemporalFinding>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some(fenced) = extract_json_object(output) {
        fenced
    } else {
        trimmed.to_string()
    };
    let parsed: DeepTemporalCheckOutput = serde_json::from_str(&candidate)?;
    Ok(parsed.findings)
}

/// Output shape returned by the model router for the promise-payoff deep check
/// (T-106). A `match` claims that an unresolved narrative promise appears to be
/// paid off within the scoped prose. `paid_off` gates the claim so a model that
/// echoes an empty/negative verdict never produces a false proposal.
#[derive(Debug, Clone, serde::Deserialize)]
struct DeepPromisePayoffOutput {
    #[serde(default)]
    matches: Vec<DeepPromisePayoffMatch>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DeepPromisePayoffMatch {
    #[serde(default)]
    paid_off: bool,
    #[serde(default)]
    scene_ref: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

/// Build the prompt for the model-backed promise-payoff audit. The
/// "narrative promise payoff audit" header line is also the marker the local
/// stub keys on to return deterministic JSON in tests / local-only runs.
///
/// Prose budgeting mirrors [`build_temporal_deep_check_prompt`] exactly: that
/// check sends the whole `scene.full_text` with no character cap, one model
/// call per scene. Here we make one call per promise and concatenate the FULL
/// text of every scoped scene under the same convention — no additional
/// truncation cap is applied, because the temporal check applies none.
fn build_promise_payoff_deep_check_prompt(
    promise: &crate::sqlite::records::NarrativePromise,
    scenes: &[crate::sqlite::records::Scene],
) -> String {
    let planted = format!(
        "{}:{}",
        promise.planted_at.book_number, promise.planted_at.chapter_number
    );
    let planned_payoff = promise
        .planned_payoff
        .as_ref()
        .map(|p| format!("{}:{}", p.book_number, p.chapter_number))
        .unwrap_or_else(|| "unset".to_string());
    let prose_block = scenes
        .iter()
        .map(|scene| {
            format!(
                "[scene {} @ {}:{}]\n{}",
                scene.id, scene.book_number, scene.chapter_number, scene.full_text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "You are performing a narrative promise payoff audit for a single novel promise.\n\
         Read the scoped prose and decide whether the promise below appears to have ALREADY been paid off — its setup delivered on the page.\n\
         Judge by what the prose renders, not by what is merely foreshadowed or restated. Do NOT claim a payoff that has not actually landed.\n\
         Return strict JSON only.\n\n\
         Promise description: {}\n\
         Planted at (book:chapter): {}\n\
         Planned payoff (book:chapter): {}\n\
         Scoped prose:\n{}\n\n\
         Return this exact shape:\n\
         {{\"matches\":[{{\"paid_off\":true,\"scene_ref\":\"scene <id>\",\"evidence\":\"<short quote>\"}}]}}\n\
         Set paid_off true only when the payoff is clearly on the page. Return an empty matches array when it is not.",
        promise.description, planted, planned_payoff, prose_block,
    )
}

/// Build the honest-skip `ConsistencyIssue` for a promise-payoff deep-check
/// route failure. When `err` is a [`RouteClearanceError::RatingNotCovered`] the
/// message names the route and rating (ids only, never prose); any other error
/// (unreachable/missing route) reads as a generic dead-route skip. Either way
/// the finding reads as SKIPPED, never clean.
fn payoff_skip_issue(err: &anyhow::Error) -> spindle_core::models::ConsistencyIssue {
    use spindle_core::models::ConsistencyIssue;
    let message = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => format!(
            "promise payoff detection was SKIPPED: the `{route}` route is not cleared for rating \
             `{rating}` (the scene set's maximum). Promises were NOT scanned for payoff."
        ),
        _ => "promise payoff detection was SKIPPED: no usable review route (the model call \
              failed). Promises were NOT scanned for payoff."
            .to_string(),
    };
    ConsistencyIssue {
        severity: "info".to_string(),
        check_type: "promise_payoff_detection".to_string(),
        message,
        entity_ids: Vec::new(),
        suggested_action: Some(
            "configure a review model route cleared for the batch rating, then re-run this check"
                .to_string(),
        ),
        evidence: None,
    }
}

/// Build the honest-skip `ConsistencyIssue` for a behavioral secret-leak
/// deep-check route failure. Same shape as [`payoff_skip_issue`]: a rating-
/// clearance failure names route+rating (ids only, never prose); any other
/// error reads as a generic dead-route skip.
fn secret_leak_skip_issue(err: &anyhow::Error) -> spindle_core::models::ConsistencyIssue {
    use spindle_core::models::ConsistencyIssue;
    let message = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => format!(
            "behavioral secret-leak detection was SKIPPED: the `{route}` route is not cleared for \
             rating `{rating}`. Scenes were NOT audited for behavioral leaks."
        ),
        _ => "behavioral secret-leak detection was SKIPPED: no usable review route (the model \
              call failed). Scenes were NOT audited for behavioral leaks."
            .to_string(),
    };
    ConsistencyIssue {
        severity: "info".to_string(),
        check_type: "secret_leak".to_string(),
        message,
        entity_ids: Vec::new(),
        suggested_action: Some(
            "configure a review model route cleared for the scene rating, then re-run this check"
                .to_string(),
        ),
        evidence: None,
    }
}

/// Build the honest-skip `ConsistencyIssue` for the intra-scene temporal-
/// coherence deep tier. Two branches, two emission cadences:
///
/// - `RatingNotCovered` (rating-clearance rejection): per-scene granularity —
///   different scenes can carry different ratings, so each uncleared scene emits
///   its own skip naming route + rating (ids only, never prose). "This scene".
/// - any OTHER error (transport failure / dead route): the whole deep TIER is
///   skipped ONCE per run — a dead route won't heal mid-run, so the caller emits
///   this once and stops calling the model for remaining scenes. Frames the
///   deep TIER (not a single scene).
///
/// This asymmetry (per-scene for clearance, once-per-run for transport) is
/// deliberate; see the caller in `deep_temporal_coherence_issues`.
fn temporal_deep_skip_issue(err: &anyhow::Error) -> spindle_core::models::ConsistencyIssue {
    use spindle_core::models::ConsistencyIssue;
    let message = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => format!(
            "temporal coherence deep check was SKIPPED: the `{route}` route is not cleared for \
             rating `{rating}`. This scene was NOT audited for intra-scene time jumps."
        ),
        _ => "temporal coherence deep tier was SKIPPED: no usable review route (the model call \
              failed / transport error). No scenes were audited for intra-scene time jumps by \
              the model-backed tier this run; the deterministic Tier-1 scan is unaffected."
            .to_string(),
    };
    ConsistencyIssue {
        severity: "info".to_string(),
        check_type: "temporal_coherence".to_string(),
        message,
        entity_ids: Vec::new(),
        suggested_action: Some(
            "configure a review model route cleared for the scene rating, then re-run this check"
                .to_string(),
        ),
        evidence: None,
    }
}

/// Build the honest-skip `ConsistencyIssue` for the semantic world-rule deep
/// tier. Two branches, two emission cadences (mirrors
/// [`temporal_deep_skip_issue`]):
///
/// - `RatingNotCovered` (rating-clearance rejection): per-scene granularity —
///   ratings can differ per scene, so each uncleared scene emits its own skip
///   naming route + rating (ids only, never prose). "This scene".
/// - any OTHER error (transport failure / dead route): emitted ONCE per run.
///   The model tier degraded to the deterministic HEURISTIC tier only — the
///   heuristic path is real coverage and KEEPS running; this finding makes the
///   degradation visible instead of silent. Frames the whole deep TIER.
///
/// See the caller in `deep_world_rule_compliance_issues` for the asymmetry.
fn world_rule_deep_skip_issue(err: &anyhow::Error) -> spindle_core::models::ConsistencyIssue {
    use spindle_core::models::ConsistencyIssue;
    let message = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => format!(
            "world rule semantic deep check was SKIPPED: the `{route}` route is not cleared for \
             rating `{rating}`. This scene was NOT audited for semantic world-rule violations."
        ),
        _ => "world rule semantic deep tier DEGRADED to the deterministic heuristic tier only: \
              no usable review route (the model call failed / transport error). Heuristic \
              world-rule coverage still ran for every scene; the model-backed deep tier did not."
            .to_string(),
    };
    ConsistencyIssue {
        severity: "info".to_string(),
        check_type: "world_rule_compliance".to_string(),
        message,
        entity_ids: Vec::new(),
        suggested_action: Some(
            "configure a review model route cleared for the scene rating, then re-run this check"
                .to_string(),
        ),
        evidence: None,
    }
}

/// The maximum content rating across a set of scenes, ordered
/// general < teen < mature < explicit, lowercased. Returns `None` for an empty
/// slice.
///
/// The promise-payoff deep check concatenates several scenes' prose into one
/// prompt, so the outgoing request must carry the STRICTEST rating present — a
/// single explicit scene in the batch makes the whole request explicit, so the
/// rating-gated chokepoint routes it to an explicit-cleared agent (evolution
/// §4). Unknown rating strings are treated as `general` (rank 0) so a typo can
/// never silently downgrade a stricter scene below it.
fn max_scene_rating(scenes: &[crate::sqlite::records::Scene]) -> Option<String> {
    fn rank(rating: &str) -> u8 {
        match rating.trim().to_ascii_lowercase().as_str() {
            "explicit" => 3,
            "mature" => 2,
            "teen" => 1,
            _ => 0,
        }
    }
    scenes
        .iter()
        .max_by_key(|scene| rank(&scene.content_rating))
        .map(|scene| scene.content_rating.trim().to_ascii_lowercase())
}

/// Parse the model router's promise-payoff deep-check output. Tolerates code
/// fences and surrounding prose via [`extract_json_object`]; an unparseable
/// response (e.g. the default local stub) is an error the caller treats as no
/// match rather than a guess.
fn parse_deep_promise_payoff_output(output: &str) -> anyhow::Result<Vec<DeepPromisePayoffMatch>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some(fenced) = extract_json_object(output) {
        fenced
    } else {
        trimmed.to_string()
    };
    let parsed: DeepPromisePayoffOutput = serde_json::from_str(&candidate)?;
    Ok(parsed.matches)
}

/// Distinctive header line for the model-backed behavioral secret-leak audit
/// (design §2.4 deep tier). Doubles as the marker the local `review` stub keys
/// on to return deterministic JSON in tests / local-only runs, exactly like the
/// temporal and promise-payoff headers.
const SECRET_LEAK_AUDIT_HEADER: &str = "secret knowledge leak audit";

/// One secret fact "in play" for the behavioral audit: the fact statement plus
/// the present rosters (who present knows it, who present does not). The
/// unaware-present ids are the only characters the model may name as leakers.
struct SecretInPlay {
    fact_value: String,
    known_present: Vec<String>,
    unaware_present: Vec<String>,
}

/// At most this many (scene, secret-set) model calls per behavioral secret-leak
/// run, ranked by scene story order; the remainder is reported as one summary
/// info finding. Mirrors `PROMISE_PAYOFF_SCAN_CAP`.
const SECRET_LEAK_DEEP_SCAN_CAP: usize = 10;

/// One scene's worth of work for the behavioral secret-leak audit: the scene
/// ref/id/rating, its prose, the secrets in play, and the id→name map of the
/// present out-of-circle characters that a model finding is validated against
/// (any character_id outside this map is discarded).
struct SecretLeakScanUnit {
    scene_id: String,
    scene_ref: String,
    scene_rating: String,
    scene_text: String,
    secrets: Vec<SecretInPlay>,
    unaware_present: std::collections::BTreeMap<String, String>,
}

/// Strict-parse output contract for the behavioral secret-leak audit. A
/// non-JSON local stub (the default `review` route) parses to nothing, so a
/// local-only deployment never fabricates a leak.
#[derive(Debug, Clone, serde::Deserialize)]
struct DeepSecretLeakOutput {
    #[serde(default)]
    findings: Vec<DeepSecretLeakFinding>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct DeepSecretLeakFinding {
    #[serde(default)]
    character_id: String,
    // The model reports `severity` per the contract, but every emitted
    // behavioral leak is a `warning` (same tier as the deterministic finding),
    // so the value is not read — serde ignores the extra field.
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    evidence: Option<String>,
}

/// Build the model-backed behavioral secret-leak prompt for one (scene,
/// secret-set). Carries the scene prose and, per secret in play, the fact and
/// its present rosters. The `[SECRET-LEAK-AUDIT ...]` header is also the marker
/// the local stub keys on.
///
/// Prose budgeting mirrors [`build_temporal_deep_check_prompt`]: the whole
/// `scene_text` ships with no character cap, one model call per scene.
fn build_secret_behavioral_leak_prompt(scene_text: &str, secrets: &[SecretInPlay]) -> String {
    let secret_block = secrets
        .iter()
        .enumerate()
        .map(|(idx, secret)| {
            format!(
                "Secret {}: {}\n  Present and IN the know: {}\n  Present and NOT in the know (only these may be flagged): {}",
                idx + 1,
                secret.fact_value,
                if secret.known_present.is_empty() {
                    "(none present)".to_string()
                } else {
                    secret.known_present.join(", ")
                },
                secret.unaware_present.join(", "),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are performing a {SECRET_LEAK_AUDIT_HEADER} for a single novel scene.\n\
         Some facts are held in confidence by a circle of trust. Read the scene prose and decide whether any character who is NOT in a secret's circle betrays knowledge of it — by referencing it, implying they know it, acting upon it, or conspicuously AVOIDING something only a knower would avoid (e.g. shunning a place they have no stated reason to shun).\n\
         Judge by what the prose renders, not by what the reader knows. Only flag a character listed as \"NOT in the know\" for that secret. Do NOT invent leaks or flag characters who are in the know.\n\
         Return strict JSON only.\n\n\
         Secrets in play:\n{secret_block}\n\n\
         Scene prose:\n{scene_text}\n\n\
         Return this exact shape:\n\
         {{\"findings\":[{{\"character_id\":\"<one of the NOT-in-the-know ids>\",\"severity\":\"warning\",\"description\":\"<how they leaked it>\",\"evidence\":\"<short quote>\"}}]}}\n\
         Return an empty findings array when no out-of-circle character betrays the secret.",
    )
}

/// Construct the outgoing [`ModelRequest`] for the behavioral secret-leak audit.
///
/// Rating discipline (design §2.4, evolution §4): this pass carries scene prose,
/// so it MUST stamp the scene's content rating (lowercased) on the request. The
/// router's `resolve_route` then applies any `(route, rating)` override — an
/// explicit-rated scene resolves through the `("review","explicit")` cleared
/// route rather than the base review agent. Pure so the rating carriage is
/// unit-testable without a live router.
fn build_secret_behavioral_leak_request(
    scene_rating: &str,
    prompt: String,
) -> crate::ai::ModelRequest {
    crate::ai::ModelRequest {
        route: "review".to_string(),
        prompt,
        rating: Some(scene_rating.to_ascii_lowercase()),
        context: None,
    }
}

/// Parse the behavioral secret-leak audit output. Tolerates code fences and
/// surrounding prose via [`extract_json_object`]; an unparseable response is an
/// error the caller treats as no findings rather than a guess.
fn parse_deep_secret_leak_output(output: &str) -> anyhow::Result<Vec<DeepSecretLeakFinding>> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some(fenced) = extract_json_object(output) {
        fenced
    } else {
        trimmed.to_string()
    };
    let parsed: DeepSecretLeakOutput = serde_json::from_str(&candidate)?;
    Ok(parsed.findings)
}

/// Distinctive header line for the model-backed scene-purpose fulfillment audit
/// (evolution design §3.6). Doubles as the marker the local `review` stub keys
/// on to return deterministic JSON in tests / local-only runs, exactly like the
/// temporal, promise-payoff, and secret-leak headers.
const SCENE_PURPOSE_AUDIT_HEADER: &str = "scene purpose fulfillment audit";

/// At most this many scenes per scene-purpose fulfillment run, ranked by scene
/// story order; the remainder is reported as one summary info finding. Mirrors
/// `PROMISE_PAYOFF_SCAN_CAP` / `SECRET_LEAK_DEEP_SCAN_CAP`.
const SCENE_PURPOSE_SCAN_CAP: usize = 10;

/// One scene's worth of work for the scene-purpose fulfillment audit: the scene
/// ref/id/rating, its prose, and the planned purpose it is measured against.
struct ScenePurposeScanUnit {
    scene_id: String,
    scene_ref: String,
    scene_rating: String,
    scene_text: String,
    purpose: String,
}

impl ScenePurposeScanUnit {
    /// The model's one-sentence assessment, sanitized for a finding message:
    /// newlines/runs of whitespace collapsed to single spaces and char-safe
    /// truncated to 200 chars. `None` when the model returned no assessment.
    fn assessment(&self, verdict: &ScenePurposeVerdict) -> Option<String> {
        let raw = verdict.assessment.as_deref()?;
        let cleaned = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        if cleaned.is_empty() {
            return None;
        }
        Some(crate::format::truncate_at_chars(&cleaned, 200))
    }
}

/// Strict-parse output contract for the scene-purpose fulfillment audit. A
/// non-JSON local stub (the default `review` route) parses to an error the
/// caller treats as no finding. `fulfilled` is `Option<bool>` so a payload that
/// omits the field is DISCARDED rather than defaulted (never guess a verdict).
#[derive(Debug, Clone, serde::Deserialize)]
struct ScenePurposeVerdict {
    #[serde(default)]
    fulfilled: Option<bool>,
    #[serde(default)]
    assessment: Option<String>,
    // The model reports `evidence` per the contract, but the finding message
    // carries the assessment, not the raw quote — serde ignores the extra field.
    #[serde(default)]
    #[allow(dead_code)]
    evidence: Option<String>,
}

/// Build the model-backed scene-purpose fulfillment prompt for one scene. Ships
/// the planned purpose and the whole scene prose (one call per scene, no
/// character cap — mirroring the temporal/secret-leak prose budgeting). The
/// [`SCENE_PURPOSE_AUDIT_HEADER`] line is also the marker the local stub keys on.
fn build_scene_purpose_prompt(purpose: &str, scene_text: &str) -> String {
    format!(
        "You are performing a {SCENE_PURPOSE_AUDIT_HEADER} for a single novel scene.\n\
         The chapter plan assigned this scene a purpose. Read the scene prose and decide whether the drafted scene ACTUALLY accomplishes that purpose on the page.\n\
         Judge by what the prose renders, not by what the summary claims or the plan intends. A scene may be well written yet fail to do the job the plan asked of it.\n\
         Return strict JSON only.\n\n\
         Planned purpose: {purpose}\n\n\
         Scene prose:\n{scene_text}\n\n\
         Return this exact shape:\n\
         {{\"fulfilled\":true,\"assessment\":\"<one sentence>\",\"evidence\":\"<short quote or empty>\"}}\n\
         Set fulfilled true only when the scene clearly delivers the planned purpose; set it false when the scene drifts from or fails that purpose.",
    )
}

/// Construct the outgoing [`ModelRequest`] for the scene-purpose fulfillment
/// audit. Rating discipline (evolution §4): this pass carries scene prose, so it
/// MUST stamp the scene's content rating (lowercased) on the request so the
/// router applies any `(review, rating)` override. Pure so the rating carriage
/// is unit-testable without a live router.
fn build_scene_purpose_request(scene_rating: &str, prompt: String) -> crate::ai::ModelRequest {
    crate::ai::ModelRequest {
        route: "review".to_string(),
        prompt,
        rating: Some(scene_rating.to_ascii_lowercase()),
        context: None,
    }
}

/// Parse the scene-purpose fulfillment audit output. Tolerates code fences and
/// surrounding prose via [`extract_json_object`]; an unparseable response is an
/// error the caller treats as no finding rather than a guess. A payload missing
/// the `fulfilled` field parses with `fulfilled: None` and is discarded upstream.
fn parse_scene_purpose_output(output: &str) -> anyhow::Result<ScenePurposeVerdict> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty scene-purpose audit output");
    }
    let candidate = if trimmed.starts_with('{') {
        trimmed.to_string()
    } else if let Some(fenced) = extract_json_object(output) {
        fenced
    } else {
        trimmed.to_string()
    };
    let parsed: ScenePurposeVerdict = serde_json::from_str(&candidate)?;
    Ok(parsed)
}

/// Build the honest-skip `ConsistencyIssue` for a scene-purpose fulfillment
/// deep-check route failure. Same shape as [`secret_leak_skip_issue`]: a
/// rating-clearance failure names route+rating (ids only, never prose); any
/// other error reads as a generic dead-route skip. Either way the finding reads
/// as SKIPPED, never clean.
fn scene_purpose_skip_issue(err: &anyhow::Error) -> spindle_core::models::ConsistencyIssue {
    use spindle_core::models::ConsistencyIssue;
    let message = match err.downcast_ref::<crate::ai::RouteClearanceError>() {
        Some(crate::ai::RouteClearanceError::RatingNotCovered { route, rating, .. }) => format!(
            "scene purpose fulfillment audit was SKIPPED: the `{route}` route is not cleared for \
             rating `{rating}`. Scenes were NOT audited for purpose fulfillment."
        ),
        _ => "scene purpose fulfillment audit was SKIPPED: no usable review route (the model call \
              failed). Scenes were NOT audited for purpose fulfillment."
            .to_string(),
    };
    ConsistencyIssue {
        severity: "info".to_string(),
        check_type: "scene_purpose_fulfillment".to_string(),
        message,
        entity_ids: Vec::new(),
        suggested_action: Some(
            "configure a review model route cleared for the scene rating, then re-run this check"
                .to_string(),
        ),
        evidence: None,
    }
}

/// Render a canonical-fact value to its display string. Order of
/// preference: text → number → json → "<unset>". Mirrors
/// `canonical_fact_value_for_check` in 705b835^.
/// Canonicalize a stored fact's value for consistency comparisons. Thin delegate
/// to [`crate::format::canonical_fact_value`] so there is a single source of truth.
fn canonical_fact_value_for_check(fact: &crate::sqlite::records::CanonicalFact) -> String {
    crate::format::canonical_fact_value(fact)
}

/// Shape-check proposed `character_emotional_profile` column writes against
/// the types the row readers deserialize into. The schema only enforces
/// `json_valid` on these columns, so without this a type-wrong-but-valid-JSON
/// value would persist and then break every subsequent read of the profile —
/// and the emotional profile is embedded in character briefings, so one bad
/// write poisons the character's whole drafting context. Unknown columns are
/// ignored here; the allowlist check rejects them separately.
fn validate_emotional_profile_changes(
    changes: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<()> {
    use crate::sqlite::json_records::StoredFlexRange;
    for (column, value) in changes {
        match column.as_str() {
            "base_emotions" if !value.is_object() => {
                anyhow::bail!(
                    "character_emotional_profile.base_emotions must be a JSON object \
                     mapping emotion names to values"
                );
            }
            "suppressed" | "triggers" | "defense_mechanisms" => {
                serde_json::from_value::<Vec<String>>(value.clone()).map_err(|_| {
                    anyhow::anyhow!(
                        "character_emotional_profile.{column} must be an array of strings"
                    )
                })?;
            }
            "flex_range" if !value.is_null() => {
                serde_json::from_value::<StoredFlexRange>(value.clone()).map_err(|_| {
                    anyhow::anyhow!(
                        "character_emotional_profile.flex_range must be null or an object \
                         with optional `low`/`high` string fields"
                    )
                })?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Shape-check a proposed `character.aliases` write. The column only requires
/// `json_valid`, so `[1, 2]` or `{"nick": "Gull"}` would persist and then
/// break every later `get_character` / snapshot / FTS-adjacent read of the row.
fn validate_character_aliases_value(value: &serde_json::Value) -> Result<()> {
    serde_json::from_value::<Vec<String>>(value.clone())
        .map_err(|_| anyhow::anyhow!("character.aliases must be an array of strings"))?;
    Ok(())
}

/// Validate a reader-contract field that must be a string.
fn parse_contract_string(field: &str, value: &serde_json::Value) -> Result<String> {
    match value {
        serde_json::Value::String(s) => Ok(s.clone()),
        other => anyhow::bail!("reader_contract.{field} must be a string, got {other}"),
    }
}

/// Validate a reader-contract field that must be an array of strings.
fn parse_contract_string_list(field: &str, value: &serde_json::Value) -> Result<Vec<String>> {
    serde_json::from_value::<Vec<String>>(value.clone())
        .map_err(|_| anyhow::anyhow!("reader_contract.{field} must be an array of strings"))
}

/// The free-text fields of a narrator voice that may carry a word-count
/// target, flattened for contradiction scanning.
fn narrator_voice_text_fields(voice: &spindle_core::style::NarratorVoice) -> Vec<String> {
    let mut fields = Vec::new();
    if let Some(v) = &voice.comedy_density {
        fields.push(v.clone());
    }
    if let Some(v) = &voice.pacing_feel {
        fields.push(v.clone());
    }
    if let Some(v) = &voice.interiority_ratio {
        fields.push(v.clone());
    }
    if let Some(v) = &voice.emotional_register {
        fields.push(v.clone());
    }
    if let Some(v) = &voice.chapter_ending_style {
        fields.push(v.clone());
    }
    fields.extend(voice.notes.iter().cloned());
    fields
}

/// Extract word-count target ranges like "1,200–2,000 words", "1200-2000
/// words", "3,500 to 5,000 words", or "1200–2000-word chapters". Only ranges
/// followed (or immediately trailed) by the word "word"/"words" count, so an
/// unrelated pair of numbers is not treated as a length target. Returns
/// inclusive `(lo, hi)` pairs, deduplicated.
fn extract_word_count_ranges(text: &str) -> Vec<(i64, i64)> {
    // Work on the lowercased string so byte indices stay valid for slicing.
    let lower = text.to_lowercase();
    let bytes = lower.as_bytes();

    // Number spans as (start_byte, end_byte, value).
    let mut numbers: Vec<(usize, usize, i64)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut digits = String::new();
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b',') {
                if bytes[i].is_ascii_digit() {
                    digits.push(bytes[i] as char);
                }
                i += 1;
            }
            if let Ok(value) = digits.parse::<i64>() {
                numbers.push((start, i, value));
            }
        } else {
            i += 1;
        }
    }

    let mut ranges: Vec<(i64, i64)> = Vec::new();
    for pair in numbers.windows(2) {
        let (_, a_end, a_val) = pair[0];
        let (b_start, b_end, b_val) = pair[1];
        if a_end > b_start {
            continue;
        }
        let gap = lower[a_end..b_start].trim();
        let is_range_separator =
            gap == "-" || gap == "–" || gap == "—" || gap == "to" || gap == "and";
        if !is_range_separator {
            continue;
        }
        // Require "word"/"words" shortly after the second number.
        let after_end = (b_end + 40).min(lower.len());
        let after = &lower[b_end..after_end];
        if after.contains("word") {
            let lo = a_val.min(b_val);
            let hi = a_val.max(b_val);
            if !ranges.contains(&(lo, hi)) {
                ranges.push((lo, hi));
            }
        }
    }
    ranges
}

/// True when two inclusive ranges overlap.
fn word_ranges_overlap(a: &(i64, i64), b: &(i64, i64)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

/// Two surfaces contradict when no range on one side overlaps any range on the
/// other. Empty sides never contradict.
fn word_ranges_contradict(a: &[(i64, i64)], b: &[(i64, i64)]) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    !a.iter()
        .any(|ra| b.iter().any(|rb| word_ranges_overlap(ra, rb)))
}

/// Render a set of ranges for a warning message.
fn format_word_ranges(ranges: &[(i64, i64)]) -> String {
    ranges
        .iter()
        .map(|(lo, hi)| format!("{lo}-{hi} words"))
        .collect::<Vec<_>>()
        .join(" / ")
}

/// The character's search-embedding document. Aliases ride in the content so
/// find_entity's semantic (kNN) path resolves nicknames and pre-rename names;
/// the exact-name FTS path is covered by fts_character.aliases (V0037). Every
/// write path that indexes a character (create, rename, rebuild) must go
/// through this so the document shape stays consistent.
fn character_search_document(character: &crate::sqlite::records::Character) -> SearchDocument {
    let mut content = format!("{}\n\n{}", character.name, character.summary);
    if !character.aliases.is_empty() {
        content.push_str(&format!(
            "\n\nAlso known as: {}",
            character.aliases.join(", ")
        ));
    }
    SearchDocument {
        entity_table: "character".to_string(),
        title: character.name.clone(),
        excerpt: character.summary.clone(),
        content,
    }
}

/// Secret-knowledge gating (design §2.2): the per-scene decision the context
/// assembly applies to every carrier. Produced once by
/// `resolve_scene_secret_gate` and consumed by `apply_secret_visibility`, so no
/// section decides secrecy on its own.
#[derive(Debug, Default)]
struct SceneSecretGate {
    /// Canonical-fact ids that must be stripped from the loaded fact list (and
    /// therefore from hard-constraint fact lines and read models). Every gated
    /// secret — withheld or enveloped — is here; it only ever surfaces in the
    /// `[SECRETS IN PLAY]` block.
    gated_fact_ids: std::collections::BTreeSet<String>,
    /// Value-display strings of gated secrets, used to strip the fact from
    /// subject-snapshot `canonical_facts` (which carry rendered text, not ids).
    withheld_snapshot_facts: std::collections::BTreeSet<String>,
    /// The `[SECRETS IN PLAY]` hard-constraint block(s) to append, one per
    /// enveloped secret. Empty for pure-withhold scenes (and for the common
    /// no-secrets case), keeping the no-secrets path byte-identical.
    envelope_blocks: Vec<spindle_core::models::HardConstraint>,
}

impl SceneSecretGate {
    fn is_empty(&self) -> bool {
        self.gated_fact_ids.is_empty()
    }
}

/// Scene-scoping inputs for `resolve_scene_secret_gate` (bundled to keep the
/// method's signature narrow): the present cast, the POV, the story cursor, and
/// the character id→name map used for the envelope rosters.
struct SceneSecretGateParams<'a> {
    scene_cast: &'a [String],
    pov: Option<&'a str>,
    cursor: i64,
    name_of: &'a std::collections::BTreeMap<String, String>,
}

/// The value-display string used to recognise a gated secret inside a subject
/// snapshot's `canonical_facts` (which render as `"{predicate}: {value}"` with
/// no id). Matching on the value display is robust to the snapshot's
/// `<unset>`/number formatting because filtering uses `contains`.
fn snapshot_fact_string(fact: &crate::sqlite::records::CanonicalFact) -> String {
    crate::format::canonical_fact_value(fact)
}

/// Apply a resolved [`SceneSecretGate`] to every context carrier in one place
/// (design §2.2: "The filter lives in ONE place ... not scattered per-section").
///
/// Strips gated secrets from: the assembled hard-constraint fact lines and the
/// `canonical_facts` list (via `gated_fact_ids` on the loaded facts BEFORE
/// read models / hard constraints are derived — see the call site), every
/// subject snapshot's `canonical_facts`, and the per-character knowledge
/// briefing entries linked to a gated secret. The envelope block(s) are
/// appended to `hard_constraints` as a non-truncatable tier.
///
/// Note on `semantic_references`: recon confirms semantic recall surfaces
/// ENTITY hits (characters / locations by embedding similarity), never raw
/// canonical-fact text — a secret fact statement cannot appear there, so no
/// filtering is applied to that carrier (documented rather than dead-coded).
fn apply_secret_visibility(
    gate: &SceneSecretGate,
    subjects: &mut [spindle_core::subject_snapshot::SubjectSnapshot],
) {
    if gate.is_empty() {
        return;
    }
    for snapshot in subjects.iter_mut() {
        snapshot.retain_canonical_facts(|summary| {
            !gate
                .withheld_snapshot_facts
                .iter()
                .any(|needle| summary.fact.contains(needle))
        });
    }
}

/// Map a structured [`spindle_core::models::RetconFinding`] to a
/// [`spindle_core::models::ConsistencyIssue`] for the write-time continuity gate.
/// All retcon classes surface as errors.
fn retcon_finding_to_consistency_issue(
    finding: &spindle_core::models::RetconFinding,
) -> spindle_core::models::ConsistencyIssue {
    use spindle_core::models::RetconFinding;
    match finding {
        RetconFinding::OutOfBandsKnowledge {
            character_id,
            message,
            ..
        } => spindle_core::models::ConsistencyIssue {
            severity: "error".to_string(),
            check_type: "retcon_reachability".to_string(),
            message: message.clone(),
            entity_ids: vec![character_id.clone()],
            suggested_action: Some(
                "anchor the knowledge with future_knowledge or move the reveal later".to_string(),
            ),
            evidence: None,
        },
        RetconFinding::MissingFutureKnowledgeAnchor {
            intervention_id,
            message,
            ..
        } => spindle_core::models::ConsistencyIssue {
            severity: "error".to_string(),
            check_type: "retcon_reachability".to_string(),
            message: message.clone(),
            entity_ids: vec![intervention_id.clone()],
            suggested_action: Some("add the missing future_knowledge anchor".to_string()),
            evidence: None,
        },
        RetconFinding::DeadCharacterAct {
            character_id,
            status,
            message,
            ..
        } => spindle_core::models::ConsistencyIssue {
            severity: if is_explicit_typed_deceased_marker(status) {
                "error".to_string()
            } else {
                "warning".to_string()
            },
            check_type: "retcon_reachability".to_string(),
            message: message.clone(),
            entity_ids: vec![character_id.clone()],
            suggested_action: Some(
                "revise so the deceased character does not act, or update their status".to_string(),
            ),
            evidence: None,
        },
        RetconFinding::PrematureKnowledge {
            character_id,
            message,
            ..
        } => spindle_core::models::ConsistencyIssue {
            // Advisory (whole-phrase match is low-recall); does not hard-block.
            severity: "warning".to_string(),
            check_type: "retcon_reachability".to_string(),
            message: message.clone(),
            entity_ids: vec![character_id.clone()],
            suggested_action: Some(
                "move the reveal, or record the character learning this earlier (record_knowledge)"
                    .to_string(),
            ),
            evidence: None,
        },
    }
}

/// Render the `check_consistency` markdown report. Errors land under the
/// `## Hard constraints` heading and are never trimmed; warnings and info
/// findings appear under `## Validator findings` / `## Other findings`
/// and may be dropped if the budget would be exceeded. Mirrors
/// `format_consistency_markdown` in 705b835^.
fn format_consistency_markdown(
    issues: &[spindle_core::models::ConsistencyIssue],
    report_sections: &[spindle_core::models::ConsistencySection],
    budget_tokens: usize,
) -> String {
    use spindle_core::context_bundle::estimate_text_tokens;

    let mut hard = String::new();
    hard.push_str("## Hard constraints\n");
    let errors: Vec<&spindle_core::models::ConsistencyIssue> = issues
        .iter()
        .filter(|issue| issue.severity == "error")
        .collect();
    if errors.is_empty() {
        hard.push_str("- (none)\n");
    } else {
        for issue in &errors {
            hard.push_str(&format!(
                "- **{}** [{}]: {}\n",
                issue.severity, issue.check_type, issue.message
            ));
        }
    }

    let phase_four_check_types: &[&str] = &[
        "canonical_fact_prose_drift",
        "world_rule_semantic_drift",
        "voice_drift",
        "retcon_reachability",
    ];

    let mut supplementary = String::new();
    if !report_sections.is_empty() {
        supplementary.push_str("\n## Validator findings\n");
        for section in report_sections {
            supplementary.push_str(&format!("\n### {}\n", section.validator_id));
            for scene_group in &section.scenes {
                supplementary.push_str(&format!(
                    "- Scene {}.{}.{} ({}):\n",
                    scene_group.book_number,
                    scene_group.chapter_number,
                    scene_group.scene_order,
                    scene_group.scene_id,
                ));
                for finding in &scene_group.findings {
                    if finding.severity == "error" {
                        continue;
                    }
                    supplementary.push_str(&format!(
                        "  - **{}**: {}\n",
                        finding.severity, finding.message
                    ));
                }
            }
        }
    }

    let other_supplementary: Vec<&spindle_core::models::ConsistencyIssue> = issues
        .iter()
        .filter(|issue| {
            issue.severity != "error"
                && !phase_four_check_types.contains(&issue.check_type.as_str())
        })
        .collect();
    if !other_supplementary.is_empty() {
        supplementary.push_str("\n## Other findings\n");
        for issue in &other_supplementary {
            supplementary.push_str(&format!(
                "- **{}** [{}]: {}\n",
                issue.severity, issue.check_type, issue.message
            ));
        }
    }

    let combined = format!("{hard}{supplementary}");
    if estimate_text_tokens(&combined) <= budget_tokens {
        return combined;
    }
    if estimate_text_tokens(&hard) > budget_tokens {
        return hard;
    }
    let mut trimmed = hard.clone();
    trimmed.push_str("\n## Validator findings\n");
    trimmed.push_str("- (truncated to fit budget_tokens)\n");
    trimmed
}

/// Aggregate a list of consistency issues into the
/// `CommitSceneFindingsSummary` shape that `commit_scene_changes`
/// returns to MCP callers. Mirrors `summarize_commit_scene_findings` in
/// 705b835^.
fn summarize_commit_scene_findings(
    issues: &[spindle_core::models::ConsistencyIssue],
) -> spindle_core::models::CommitSceneFindingsSummary {
    let mut by_check = std::collections::BTreeMap::new();
    for issue in issues {
        *by_check.entry(issue.check_type.clone()).or_insert(0usize) += 1;
    }
    spindle_core::models::CommitSceneFindingsSummary {
        total_count: issues.len(),
        error_count: issues
            .iter()
            .filter(|issue| issue.severity == "error")
            .count(),
        warning_count: issues
            .iter()
            .filter(|issue| issue.severity == "warning")
            .count(),
        info_count: issues
            .iter()
            .filter(|issue| issue.severity == "info")
            .count(),
        by_check,
    }
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for t in tags {
        let trimmed = t.trim().to_string();
        if !trimmed.is_empty()
            && !normalized
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&trimmed))
        {
            normalized.push(trimmed);
        }
    }
    normalized
}

fn research_branch_matches(item_branch_id: Option<&str>, target_branch_id: Option<&str>) -> bool {
    match target_branch_id {
        Some(target_branch_id) => {
            item_branch_id.is_none() || item_branch_id == Some(target_branch_id)
        }
        None => true,
    }
}

fn map_research_source(
    s: crate::sqlite::records::ResearchSource,
) -> spindle_core::models::ResearchSource {
    spindle_core::models::ResearchSource {
        id: s.id,
        project_id: s.project_id,
        branch_id: s.branch_id,
        title: s.title,
        source_type: s.source_type,
        url: s.url,
        file_path: s.file_path,
        author: s.author,
        publisher: s.publisher,
        published_date: s.published_date,
        accessed_at: crate::sqlite::row::timestamp_to_micros(s.accessed_at),
        reliability: s.reliability,
        tags: s.tags,
        summary: s.summary,
        created_at: crate::sqlite::row::timestamp_to_micros(s.created_at),
        updated_at: crate::sqlite::row::timestamp_to_micros(s.updated_at),
    }
}

fn map_research_note(
    n: crate::sqlite::records::ResearchNote,
) -> spindle_core::models::ResearchNote {
    spindle_core::models::ResearchNote {
        id: n.id,
        project_id: n.project_id,
        source_id: n.source_id,
        branch_id: n.branch_id,
        note: n.note,
        quote: n.quote,
        locator: n.locator,
        tags: n.tags,
        created_at: crate::sqlite::row::timestamp_to_micros(n.created_at),
        updated_at: crate::sqlite::row::timestamp_to_micros(n.updated_at),
    }
}

fn map_research_claim(
    c: crate::sqlite::records::ResearchClaim,
) -> spindle_core::models::ResearchClaim {
    spindle_core::models::ResearchClaim {
        id: c.id,
        project_id: c.project_id,
        source_id: c.source_id,
        note_id: c.note_id,
        branch_id: c.branch_id,
        claim: c.claim,
        topic: c.topic,
        time_period: c.time_period,
        location: c.location,
        confidence: c.confidence,
        tags: c.tags,
        created_at: crate::sqlite::row::timestamp_to_micros(c.created_at),
        updated_at: crate::sqlite::row::timestamp_to_micros(c.updated_at),
    }
}

fn map_research_usage_record(
    r: crate::sqlite::records::ResearchUsage,
) -> spindle_core::models::ResearchUsage {
    spindle_core::models::ResearchUsage {
        id: r.id,
        project_id: r.project_id,
        branch_id: r.branch_id,
        run_id: r.run_id,
        step_checkpoint_id: r.step_checkpoint_id,
        scene_id: r.scene_id,
        source_ids: r.source_ids,
        note_ids: r.note_ids,
        claim_ids: r.claim_ids,
        query_pack_input: r.query_pack_input,
        context_hash: r.context_hash,
        created_at: crate::sqlite::row::timestamp_to_micros(r.created_at),
    }
}
