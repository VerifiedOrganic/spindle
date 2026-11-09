//! SQLite-backed implementation of the spindle repository surface.
//!
//! This is the Phase 4 home of every async query function. It is intentionally
//! built in parallel with the existing [`crate::repository::SpindleRepository`]
//! so that the SurrealDB-backed code keeps working during the migration. The
//! method signatures mirror the SurrealDB repository (per the plan's "public
//! API stability is a hard constraint") *except* for the small set of cases
//! flagged below where the schema change makes a 1:1 translation impossible.
//!
//! ## Known API breaks vs the SurrealDB repository
//!
//! * `get_main_branch(&self)` — the SurrealDB schema had a global singleton
//!   `bible_branch:main`. SQLite makes branches per-project (FK to `project`),
//!   so there is no global "main" branch. Callers should use
//!   `get_active_branch(project_id)` or look up by `(project_id, name = "main")`.
//!
//! ## Translation pattern (canonical)
//!
//! ```ignore
//! pub async fn get_project(&self, id: &str) -> anyhow::Result<Project> {
//!     let id = id.to_string();
//!     self.pool
//!         .read(move |conn| {
//!             let mut stmt = conn.prepare_cached(&format!(
//!                 "SELECT {PROJECT_COLUMNS} FROM project WHERE id = ?1"
//!             ))?;
//!             stmt.query_row([&id], |r| Project::try_from(r))
//!         })
//!         .await?
//!         .ok_or_else(|| anyhow!("project {id} not found"))
//! }
//! ```
//!
//! Every SELECT uses the matching `*_COLUMNS` constant from `sqlite::records`,
//! so column order is grep-able and drift is caught at the call site.

use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use ulid::Ulid;

use crate::ai::{ModelRouter, SearchDocument};
use serde_json::Value;
use spindle_core::models::{
    AnnotatedBeat, ChapterOutlineBeat, CharacterStatePatch, CreateCharacterArcInput,
    CreateCharacterInput, CreateConflictInput, CreateEconomyInput, CreateFactionInput,
    CreateFutureKnowledgeInput, CreateLocationInput, CreateMotifInput, CreateNarrativePromiseInput,
    CreatePacingConfigInput, CreatePacingCurveInput, CreatePlotLineInput, CreateProjectInput,
    CreateRelationshipInput, CreateReligionInput, CreateSystemOverlayInput,
    CreateTemporalInterventionInput, CreateTermInput, CreateThemeInput, CreateTimelineEventInput,
    CreateWorldRuleInput, DualPersonaReviewRound, PlanChapterInput, PlanChapterSceneInput,
    PlannedScene, SaveSceneDraftInput, SaveSummaryInput, StoryPlacement, UpdateRelationshipInput,
    normalize_name,
};
use spindle_core::provenance::{Provenance, RecordId as SnapshotRecordId};
use spindle_core::subject::{Subject, SubjectTable};
use spindle_core::subject_snapshot::{
    CanonicalFactSummary, CharacterArcDetails, CharacterArcSummary, CharacterDetails,
    CharacterStateSummary, ConflictDetails, EconomyDetails, FactionDetails, KnowledgeFactSummary,
    LocationDetails, MotifDetails, NarrativePromiseDetails, NarrativePromiseSummary,
    PlotLineDetails, RelationshipDetails, RelationshipSummary, ReligionDetails,
    SceneAppearanceSummary, SubjectKindSpecific, SubjectLinkSummary, SubjectSnapshot,
    SystemOverlayDetails, TermDetails, ThemeDetails, TimelineEventDetails, VoiceProfileSummary,
    WorldRuleDetails,
};

use super::records::{
    BIBLE_BRANCH_COLUMNS, BOOK_COLUMNS, BibleBranch, Book, CHAPTER_COLUMNS, CHARACTER_ARC_COLUMNS,
    CHARACTER_COLUMNS, CHARACTER_EMOTIONAL_PROFILE_COLUMNS, CHARACTER_VOICE_PROFILE_COLUMNS,
    CONFLICT_COLUMNS, Chapter, Character, CharacterArc, CharacterEmotionalProfile,
    CharacterVoiceProfile, Conflict, ECONOMY_COLUMNS, Economy, FACTION_COLUMNS, Faction,
    LOCATION_COLUMNS, Location, MOTIF_COLUMNS, Motif, NARRATIVE_PROMISE_COLUMNS, NarrativePromise,
    PLOT_LINE_COLUMNS, PROJECT_COLUMNS, PlotLine, Project, RELIGION_COLUMNS, Religion,
    SCENE_COLUMNS, SYSTEM_OVERLAY_COLUMNS, Scene, StoredCharacterArcMilestone, StoredEstablishedIn,
    StoredFlexRange, StoredIntensityPoint, StoredStatedConsequence, StoredStoryPlacement,
    StoredTryFailCycleStep, SystemOverlay, TEMPORAL_INTERVENTION_COLUMNS, TERM_COLUMNS,
    THEME_COLUMNS, TIMELINE_EVENT_COLUMNS, TemporalIntervention, Term, Theme, TimelineEvent,
    WORLD_RULE_COLUMNS, WORLD_STATE_COLUMNS, WorldRule, WorldState,
};
use super::records::{
    BOOK_OUTLINE_COLUMNS, BookOutline, CANONICAL_FACT_COLUMNS, CHAPTER_OUTLINE_COLUMNS,
    CHAPTER_PLAN_COLUMNS, CHAPTER_SUMMARY_COLUMNS, CHARACTER_STATE_COLUMNS, CanonicalFact,
    ChapterOutline, ChapterPlan, ChapterSummary, CharacterState, DUAL_PERSONA_REVIEW_COLUMNS,
    DualPersonaReview, FUTURE_KNOWLEDGE_COLUMNS, FutureKnowledge, IMPORT_CHARACTER_DOSSIER_COLUMNS,
    IMPORT_ENTITY_CLUSTER_COLUMNS, IMPORT_ENTITY_MENTION_COLUMNS, IMPORT_NARRATIVE_DOSSIER_COLUMNS,
    IMPORT_RESUME_SNAPSHOT_COLUMNS, IMPORT_REVIEW_ITEM_COLUMNS, IMPORT_SEGMENT_COLUMNS,
    IMPORT_SESSION_COLUMNS, IMPORT_SOURCE_DOCUMENT_COLUMNS, IMPORT_WORLD_DOSSIER_COLUMNS,
    ImportCharacterDossier, ImportEntityCluster, ImportEntityMention, ImportNarrativeDossier,
    ImportResumeSnapshot, ImportReviewItem, ImportSegment, ImportSession, ImportSourceDocument,
    ImportWorldDossier, KNOWLEDGE_FACT_COLUMNS, KNOWS_COLUMNS, KnowledgeFact, Knows,
    PACING_CONFIG_COLUMNS, PACING_CURVE_COLUMNS, PACING_TRACKER_COLUMNS, PROGRESSION_EVENT_COLUMNS,
    PacingConfig, PacingCurve, PacingTracker, ProgressionEvent, RELATES_TO_COLUMNS,
    RESEARCH_CLAIM_COLUMNS, RESEARCH_LOG_COLUMNS, RESEARCH_NOTE_COLUMNS, RESEARCH_SOURCE_COLUMNS,
    RESEARCH_USAGE_COLUMNS, REVISION_MARKER_COLUMNS, RelatesTo, ResearchClaim, ResearchLog,
    ResearchNote, ResearchSource, ResearchUsage, RevisionMarker, SAVE_POINT_COLUMNS,
    SCENE_BEAT_ANNOTATION_COLUMNS, SCENE_SOURCE_LINK_COLUMNS, SCENE_VERSION_COLUMNS,
    SEARCH_EMBEDDING_COLUMNS, SESSION_ACTIVITY_COLUMNS, SavePoint, SceneBeatAnnotation,
    SceneSourceLink, SceneVersion, SearchEmbedding, SessionActivity, StoredAnnotatedBeat,
    StoredChapterOutlineBeat, StoredDualPersonaReviewRound, VALIDATOR_FINDING_COLUMNS,
    ValidatorFinding, WRITER_POSITION_COLUMNS, WriterPosition,
};
use super::row::pack_embedding;
use super::row::timestamp_to_micros;
use super::{SqlitePool, records, row};

/// Snapshot of every branch-scoped row needed to rewind a branch to a
/// previous state. Each entry maps a SQLite table name to a list of JSON
/// row objects (keyed by column name; output of `dump_project_table`
/// filtered to the target branch). `restore_branch_snapshot` consumes one
/// of these: it DELETEs the branch's current content and INSERTs the
/// snapshot rows in a single transaction with deferred FK checks.
///
/// Format diverges trivially from the SurrealDB-era struct of the same
/// name (`repository.rs:148..190` in 705b835^), which carried a per-table
/// `Vec<RestoreSnapshotRow { id, content: DbValue }>`. SQLite stores
/// everything as schema-by-name JSON, so we keep that representation
/// end-to-end instead of round-tripping through `surrealdb::sql::Value`.
#[derive(Debug, Clone, Default)]
pub struct BranchRestoreSnapshot {
    pub rows_by_table: std::collections::BTreeMap<String, Vec<serde_json::Map<String, Value>>>,
}

/// Branch-scoped tables that `restore_branch_snapshot` walks. Order is
/// kept stable so DELETE and INSERT visit the same set, and the test
/// surface can pin row-count assertions. Children-first ordering doesn't
/// matter here because we run inside a transaction with
/// `PRAGMA defer_foreign_keys = 1`.
pub(crate) const BRANCH_RESTORE_TABLES: &[&str] = &[
    "revision_marker",
    "dual_persona_review",
    "scene_beat_annotation",
    "canonical_fact",
    "scene_version",
    "scene_source_link",
    "character_voice_profile",
    "character_emotional_profile",
    "character_state",
    "future_knowledge",
    "timeline_event",
    "temporal_intervention",
    "progression_event",
    "pacing_tracker",
    "chapter_plan",
    "chapter_summary",
    "pacing_curve",
    "pacing_config",
    "narrative_promise",
    "character_arc",
    "plot_line",
    "conflict",
    "theme",
    "motif",
    "faction",
    "religion",
    "economy",
    "term",
    "system_overlay",
    "knowledge_fact",
    "world_state",
    "world_rule",
    "scene",
    "location",
    "character",
    "book_outline",
    "chapter_outline",
    "relates_to",
    "relationship_scene_delta",
    "knows",
];

/// Parameters for upserting writer state. SQLite-side counterpart to the
/// SurrealDB repo's struct of the same name; only difference is `String` IDs.
#[derive(Debug, Clone)]
pub struct UpsertWriterPositionParams {
    pub project_id: String,
    pub branch_id: String,
    pub book_id: Option<String>,
    pub chapter_id: Option<String>,
    pub scene_id: Option<String>,
    pub intent: String,
    pub next_focus: Option<String>,
    pub updated_by: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Parameters for creating an import_session row. SQLite-side counterpart
/// to the SurrealDB struct of the same name.
#[derive(Debug, Clone)]
pub struct CreateImportSessionParams {
    pub project_id: Option<String>,
    pub target_branch_id: Option<String>,
    pub source_format: Option<String>,
    pub active_pass: String,
    pub progress: Value,
    pub session_status: String,
    pub hydrate_mode: String,
    pub source_count: usize,
}

/// Parameters for upserting an import_source_document row.
#[derive(Debug, Clone)]
pub struct UpsertImportSourceDocumentParams {
    pub session_id: String,
    pub project_id: Option<String>,
    pub display_name: String,
    pub source_path: String,
    pub copied_path: String,
    pub source_format: String,
    pub original_sha256: String,
    pub normalized_sha256: String,
    pub normalized_text_ref: String,
    pub word_count: usize,
    pub chapter_hint: Option<String>,
    pub source_order: usize,
}

/// Parameters for upserting an import_segment row.
#[derive(Debug, Clone)]
pub struct UpsertImportSegmentParams {
    pub session_id: String,
    pub source_document_id: String,
    pub parent_segment_id: Option<String>,
    pub segment_type: String,
    pub source_order: usize,
    pub book_number: Option<i32>,
    pub chapter_number: Option<i32>,
    pub scene_order: Option<i32>,
    pub label: Option<String>,
    pub start_offset: usize,
    pub end_offset: usize,
    pub word_count: usize,
    pub character_count: usize,
    pub pov_guess: Option<Value>,
    pub confidence: f64,
    pub segment_status: String,
}

/// Parameters for creating an import_entity_mention row.
#[derive(Debug, Clone)]
pub struct CreateImportEntityMentionParams {
    pub session_id: String,
    pub segment_id: String,
    pub entity_kind: String,
    pub surface_form: String,
    pub normalized_name: String,
    pub alias_hint: Option<String>,
    pub surrounding_text: Option<String>,
    pub confidence: f64,
    pub extraction_pass: String,
}

/// Parameters for upserting an import_entity_cluster row.
#[derive(Debug, Clone)]
pub struct UpsertImportEntityClusterParams {
    pub session_id: String,
    pub entity_kind: String,
    pub canonical_name: String,
    pub normalized_name: String,
    pub aliases: Vec<String>,
    pub mention_ids: Vec<String>,
    pub first_segment_id: Option<String>,
    pub last_segment_id: Option<String>,
    pub importance_rank: i32,
    pub merge_confidence: f64,
    pub review_required: bool,
    pub notes: Vec<String>,
}

/// Parameters for upserting an import_character_dossier row.
#[derive(Debug, Clone)]
pub struct UpsertImportCharacterDossierParams {
    pub session_id: String,
    pub cluster_id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub importance_rank: i32,
    pub voice_profile: Value,
    pub emotional_profile: Value,
    pub state_trajectory: Value,
    pub relationship_inferences: Value,
    pub decision_patterns: Vec<String>,
    pub dialogue_samples: Vec<String>,
    pub confidence: f64,
    pub review_required: bool,
}

/// Parameters for upserting an import_world_dossier row.
#[derive(Debug, Clone)]
pub struct UpsertImportWorldDossierParams {
    pub session_id: String,
    pub world_rules: Value,
    pub locations: Value,
    pub entities: Value,
    pub system_signals: Value,
}

/// Parameters for upserting an import_narrative_dossier row.
#[derive(Debug, Clone)]
pub struct UpsertImportNarrativeDossierParams {
    pub session_id: String,
    pub plot_lines: Value,
    pub conflicts: Value,
    pub narrative_promises: Value,
    pub arcs: Value,
    pub themes: Value,
    pub motifs: Value,
    pub reader_contract: Value,
    pub pacing_hints: Value,
}

/// Parameters for upserting an import_resume_snapshot row.
#[derive(Debug, Clone)]
pub struct UpsertImportResumeSnapshotParams {
    pub session_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: Option<i32>,
    pub summary: String,
    pub characters: Value,
    pub relationships: Value,
    pub locations: Value,
    pub plot_threads: Value,
}

/// Parameters for creating an import_review_item row.
#[derive(Debug, Clone)]
pub struct CreateImportReviewItemParams {
    pub session_id: String,
    pub pass_name: String,
    pub item_kind: String,
    pub severity: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub related_segment_ids: Vec<String>,
    pub related_entity_ids: Vec<String>,
    pub confidence: Option<f64>,
    pub proposed_correction: Option<Value>,
    pub resolver_notes: Option<String>,
}

/// Parameters for `resolve_import_review_item`. SQLite-side counterpart to
/// the SurrealDB struct of the same name.
#[derive(Debug, Clone)]
pub struct ResolveImportReviewItemParams {
    pub status: String,
    pub proposed_correction: Option<Value>,
    pub resolver_notes: Option<String>,
}

/// Parameters for upserting a dual_persona_review row.
#[derive(Debug, Clone)]
pub struct UpsertDualPersonaReviewParams {
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub rounds_completed: usize,
    pub review_rounds: Vec<DualPersonaReviewRound>,
    pub scene_revision_fingerprint: String,
    pub status: String,
}

/// Parameters for upserting a validator_finding row.
#[derive(Debug, Clone)]
pub struct UpsertValidatorFindingParams {
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub scene_text_hash: String,
    pub context_hash: Option<String>,
    pub validator_id: String,
    pub finding_id: String,
    pub severity: String,
    pub message: String,
    pub byte_range: Option<Value>,
    pub details_json: Option<Value>,
}

/// Parameters for appending a progression_event row.
#[derive(Debug, Clone)]
pub struct AppendProgressionEventParams {
    pub project_id: String,
    pub branch_id: String,
    pub subject_table: String,
    pub subject_id: String,
    pub overlay_id: Option<String>,
    pub kind: String,
    pub delta_json: Value,
    pub source_scene_id: Option<String>,
    pub placement: Option<StoryPlacement>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Parameters for `create_canonical_fact`. Mirrors the SurrealDB struct of
/// the same name with `String` IDs and the `value_*` fields kept loose since
/// they hold a JSON value of varying shape.
#[derive(Debug, Clone)]
pub struct CreateCanonicalFactParams {
    pub project_id: String,
    pub branch_id: String,
    /// None registers a planned-and-pending fact (V0038): decided during
    /// planning, placed by book/chapter only, bound to its scene later via
    /// `bind_canonical_fact_to_scene`.
    pub scene_id: Option<String>,
    pub book_number: i32,
    pub chapter_number: i32,
    pub subject_table: String,
    pub subject_id: Option<String>,
    pub predicate: String,
    pub value_kind: String,
    pub value_text: Option<String>,
    pub value_number: Option<f64>,
    pub unit: Option<String>,
    pub value_json: Option<Value>,
    pub aliases: Vec<String>,
    pub scope: String,
    pub valid_from: Option<StoryPlacement>,
    pub valid_until: Option<StoryPlacement>,
    pub legacy_untyped: bool,
    /// Secret-knowledge gating (V0023): mark the new fact as secret. Defaults
    /// to false for every public fact.
    pub secret: bool,
    /// Optional concealment guidance stored on the secret fact.
    pub concealment_note: Option<String>,
}

/// Parameters for upserting a knowledge_fact.
#[derive(Debug, Clone)]
pub struct UpsertKnowledgeFactParams {
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub fact: String,
    pub normalized_fact: String,
    pub source_summary: String,
    pub learned_at: Option<StoryPlacement>,
    pub confidence: Option<f64>,
    pub tags: Vec<String>,
    pub reader_visible: bool,
    pub source_import_session_id: Option<String>,
    /// Secret-knowledge gating (V0023): link this knowledge row to the secret
    /// canonical fact it grants circle membership in. `None` for ordinary
    /// knowledge rows.
    pub secret_of_fact_id: Option<String>,
}

/// Parameters for linking a single holder into a secret fact's circle of trust
/// (design §2.1, declaration path). See [`Repository::link_secret_holder`].
#[derive(Debug, Clone)]
pub struct LinkSecretHolderParams {
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    /// The rendered fact text (reuses the canonical fact's value display).
    pub fact_text: String,
    /// `normalize_name(fact_text)` — the unique-index key.
    pub normalized_fact: String,
    pub source_summary: String,
    /// The secret `canonical_fact.id` this holder is being linked to.
    pub secret_of_fact_id: String,
}

/// Parameters for upserting a `knows` edge.
#[derive(Debug, Clone)]
pub struct UpsertKnowsParams {
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub knowledge_fact_id: String,
    pub source_summary: Option<String>,
    pub learned_at: Option<StoryPlacement>,
    pub confidence: Option<f64>,
    pub reader_visible: bool,
    pub source_import_session_id: Option<String>,
}

/// Parameters for appending a character_state snapshot. SQLite-side
/// counterpart to the SurrealDB repo's struct of the same name; IDs are
/// `String` rather than `RecordId`.
#[derive(Debug, Clone)]
pub struct AppendCharacterStateParams {
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub scene_id: Option<String>,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub patch: CharacterStatePatch,
}

/// Parameters for appending a stamped quantity-state row.
#[derive(Debug, Clone)]
pub struct AppendQuantityStateParams {
    pub project_id: String,
    pub branch_id: String,
    pub subject_table: String,
    pub subject_id: String,
    pub measure: String,
    pub state: spindle_core::models::QuantityState,
    pub scene_id: Option<String>,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
}

/// Parameters for capturing an operator style-edit candidate (V0031, evolution
/// §3.9). The `agent_draft` is the prose the operator edited over; the
/// `operator_edit` is the resulting positive example fed into style refresh.
#[derive(Debug, Clone)]
pub struct CaptureStyleEditParams {
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub agent_draft: String,
    pub operator_edit: String,
    /// The scene's content rating at capture, lowercased.
    pub content_rating: String,
}

/// Parameters for staging a proposed canon delta (ADR 0001 D2).
#[derive(Debug, Clone)]
pub struct StageCanonDeltaParams {
    pub project_id: String,
    pub branch_id: String,
    /// Provenance: the scene this was mined from.
    pub scene_id: String,
    /// The authoring run that mined it, or `None` when mined outside a run.
    pub authoring_run_id: Option<String>,
    /// One of `spindle_core::models::CANON_DELTA_CLASSES`; unknown is rejected.
    pub delta_class: String,
    /// Existing entity this modifies; `None` proposes a new one.
    pub target_id: Option<String>,
    /// Typed per-class payload.
    pub payload: serde_json::Value,
    /// Sanitized prose excerpt (non-empty, ≤300 chars).
    pub evidence: String,
    /// `high` | `medium` | `low`.
    pub confidence: String,
}

/// The operator's ratification of a staged canon delta (ADR 0001 D3). The
/// repository records the decision only; the apply-dispatch to write tools is
/// the service layer's responsibility (keeps the decide/apply seam clean).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonDeltaDecision {
    Applied,
    Rejected,
}

impl CanonDeltaDecision {
    /// The terminal `status` string this decision records.
    fn status(self) -> &'static str {
        match self {
            CanonDeltaDecision::Applied => "applied",
            CanonDeltaDecision::Rejected => "rejected",
        }
    }
}

/// Parameters for staging a proposed plan amendment (ADR 0003 D2).
#[derive(Debug, Clone)]
pub struct StagePlanAmendmentParams {
    pub project_id: String,
    pub branch_id: String,
    /// Provenance: the summarized chapter that triggered the replan pass.
    pub source_chapter: i32,
    /// The book the source/target chapter numbers belong to.
    pub book_number: i32,
    /// The authoring run that staged it, or `None` when replanned outside a run.
    pub authoring_run_id: Option<String>,
    /// One of `spindle_core::models::PLAN_AMENDMENT_CLASSES`; unknown is rejected.
    pub amendment_class: String,
    /// The future chapter this amends. `None` only for `promise_followup`;
    /// required for every other class (validated at staging).
    pub target_chapter: Option<i32>,
    /// Typed per-class payload.
    pub payload: serde_json::Value,
    /// The replanner's stated reasoning (non-empty, ≤500 chars).
    pub rationale: String,
    /// `high` | `medium` | `low`.
    pub confidence: String,
}

/// The operator's ratification of a staged plan amendment (ADR 0003 D5). The
/// repository records the decision + prior state only; the apply-dispatch to
/// plan write paths is the service layer's responsibility (Part B) — keeps the
/// decide/apply seam clean, mirroring [`CanonDeltaDecision`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanAmendmentDecision {
    Applied,
    Rejected,
}

impl PlanAmendmentDecision {
    /// The terminal `status` string this decision records.
    fn status(self) -> &'static str {
        match self {
            PlanAmendmentDecision::Applied => "applied",
            PlanAmendmentDecision::Rejected => "rejected",
        }
    }
}

/// Parameters for appending a session-activity row.
#[derive(Debug, Clone)]
pub struct AppendSessionActivityParams {
    pub project_id: String,
    pub branch_id: String,
    pub kind: String,
    pub subject_table: Option<String>,
    pub subject_id: Option<String>,
    pub summary: String,
    pub details_json: Option<serde_json::Value>,
}

/// SQLite-backed repository. Cheap to clone (Arc-wrapped inner state).
#[derive(Clone)]
pub struct Repository {
    inner: Arc<Inner>,
}

type PlanningRetryLockMap =
    std::sync::Mutex<std::collections::HashMap<(String, String), Arc<tokio::sync::Mutex<()>>>>;

fn planning_attempt_lock_path(db_path: &Path, claim_token: &str) -> Option<PathBuf> {
    if db_path.as_os_str().is_empty() || db_path == Path::new(":memory:") {
        return None;
    }
    let sanitized: String = claim_token
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut os = db_path.as_os_str().to_os_string();
    os.push(format!(".{sanitized}.lock"));
    Some(PathBuf::from(os))
}

pub(crate) fn acquire_planning_attempt_lock(
    db_path: &Path,
    claim_token: &str,
) -> Result<PlanningActiveAttemptGuard> {
    let Some(lp) = planning_attempt_lock_path(db_path, claim_token) else {
        anyhow::bail!(
            "cannot coordinate planning attempt lock on an empty or in-memory database path"
        );
    };
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lp)
        .with_context(|| {
            format!(
                "failed to open planning attempt lock file '{}'",
                lp.display()
            )
        })?;
    match file.try_lock() {
        Ok(()) => Ok(PlanningActiveAttemptGuard {
            lock_file: Some(file),
            lock_path: Some(lp),
        }),
        Err(std::fs::TryLockError::WouldBlock) => {
            anyhow::bail!("planning attempt lock '{}' is already held", lp.display())
        }
        Err(std::fs::TryLockError::Error(err)) => Err(anyhow::Error::new(err).context(format!(
            "failed to lock planning attempt lock file '{}'",
            lp.display()
        ))),
    }
}

pub(crate) fn is_planning_attempt_lock_held(
    db_path: &Path,
    claim_token: Option<&str>,
) -> Result<bool> {
    let Some(token) = claim_token.filter(|t| !t.trim().is_empty()) else {
        return Ok(false);
    };
    let Some(lp) = planning_attempt_lock_path(db_path, token) else {
        anyhow::bail!(
            "cannot inspect planning attempt lock on an empty or in-memory database path"
        );
    };
    let file = match std::fs::OpenOptions::new().read(true).write(true).open(&lp) {
        Ok(f) => f,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => {
            return Err(anyhow::Error::new(err).context(format!(
                "failed to open planning attempt lock file '{}'",
                lp.display()
            )));
        }
    };
    match file.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Ok(()) => {
            let _ = file.unlock();
            Ok(false)
        }
        Err(std::fs::TryLockError::Error(err)) => Err(anyhow::Error::new(err).context(format!(
            "failed to probe planning attempt lock file '{}'",
            lp.display()
        ))),
    }
}

#[derive(Debug)]
pub(crate) struct PlanningActiveAttemptGuard {
    lock_file: Option<std::fs::File>,
    lock_path: Option<PathBuf>,
}

impl Drop for PlanningActiveAttemptGuard {
    fn drop(&mut self) {
        if let Some(file) = &self.lock_file {
            let _ = file.unlock();
        }
        if let Some(path) = &self.lock_path {
            let _ = std::fs::remove_file(path);
        }
    }
}

struct Inner {
    pool: SqlitePool,
    data_dir: PathBuf,
    model_router: ModelRouter,
    planning_retry_locks: PlanningRetryLockMap,
}

mod authoring;
pub(crate) use authoring::check_active_run_scene_handoff_plan_revision_in_conn;
mod branches;
mod canon;
pub mod canon_backfill;
pub use canon_backfill::*;
mod batch;
mod canon_cutover;
mod canon_events;
mod canon_policy;
mod memory;
pub(crate) use canon_policy::{legacy_claim, legacy_snapshot_claim};
mod characters;
mod entities;
mod export;
mod generation;
mod imports;
mod narrative;
mod planning;
pub(crate) use planning::apply::resolve_operation as resolve_planning_operation;
pub(crate) use planning::output_refs as planning_output_refs;
pub(crate) use planning::{
    PlanningAttemptClaim, PlanningSessionMetadataUpdate, PlanningStagedCommitCandidate,
};
mod plans;
mod projects;
mod quantity;
mod relationships;
mod research;
mod reviews;
mod scene_commit;
mod scenes;
mod search;
mod snapshots;
mod style;
mod timeline;
mod validation;
mod world;
mod writer;

impl Repository {
    pub fn new(pool: SqlitePool, data_dir: PathBuf) -> Self {
        Self::with_model_router(pool, data_dir, ModelRouter::default())
    }

    pub fn with_model_router(
        pool: SqlitePool,
        data_dir: PathBuf,
        model_router: ModelRouter,
    ) -> Self {
        let model_router = model_router.with_usage_pool(pool.clone());
        Self {
            inner: Arc::new(Inner {
                pool,
                data_dir,
                model_router,
                planning_retry_locks: std::sync::Mutex::new(std::collections::HashMap::new()),
            }),
        }
    }

    pub(crate) fn planning_retry_lock(
        &self,
        session_id: &str,
        retry_key: &str,
    ) -> Arc<tokio::sync::Mutex<()>> {
        let mut guard = self
            .inner
            .planning_retry_locks
            .lock()
            .expect("planning_retry_locks poisoned");
        guard
            .entry((session_id.to_owned(), retry_key.to_owned()))
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.inner.pool
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub fn model_router(&self) -> &ModelRouter {
        &self.inner.model_router
    }

    pub fn current_embedding_version(&self) -> Result<String> {
        self.inner.model_router.embedding_version()
    }

    /// Execute a multi-step canon unit-of-work inside a single SQLite writer
    /// transaction. If `f` returns `Err`, the transaction rolls back and zero
    /// writes are persisted.
    pub async fn with_transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut CanonTransaction<'_>) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        self.inner
            .pool
            .write(move |conn| {
                Ok((|| -> Result<R> {
                    let tx = conn.transaction()?;
                    let mut unit = CanonTransaction { tx };
                    let out = f(&mut unit)?;
                    unit.tx.commit()?;
                    Ok(out)
                })())
            })
            .await?
    }
}

/// Materialized JSON value that can be bound to a SQLite statement via
/// `rusqlite::ToSql`. The dump path (`row_to_json_value`) is the inverse —
/// this is what we round-trip back into the database during
/// `restore_branch_snapshot`.
///
/// Mapping back to SQLite types:
///   * `Value::Null`                 -> `ValueRef::Null`
///   * `Value::Bool(b)`              -> `INTEGER` (0/1) — matches the
///     schema's `IN (0,1)` checks.
///   * `Value::Number` (i64)         -> `INTEGER`
///   * `Value::Number` (u64 in i64)  -> `INTEGER` (anything larger errors
///     at the bind site).
///   * `Value::Number` (f64)         -> `REAL`
///   * `Value::String("base64:…")`   -> `BLOB` (decoded)
///   * `Value::String(other)`        -> `TEXT`
///   * `Value::Array` / `Value::Object` -> JSON-serialized `TEXT`
///     (matches `looks_like_json_column` on the read side).
enum JsonParam {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl JsonParam {
    fn from_value(value: Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Integer(if b { 1 } else { 0 }),
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::Integer(i)
                } else if let Some(u) = n.as_u64() {
                    Self::Integer(u as i64)
                } else if let Some(f) = n.as_f64() {
                    Self::Real(f)
                } else {
                    Self::Null
                }
            }
            Value::String(s) => {
                if let Some(b64) = s.strip_prefix("base64:") {
                    use base64::Engine;
                    match base64::engine::general_purpose::STANDARD.decode(b64) {
                        Ok(bytes) => Self::Blob(bytes),
                        Err(_) => Self::Text(s),
                    }
                } else {
                    Self::Text(s)
                }
            }
            Value::Array(_) | Value::Object(_) => {
                // Re-serialize nested JSON back into a TEXT column. This
                // matches the read-side re-parse in
                // `looks_like_json_column`; if downstream consumers want
                // raw scalars they shouldn't have nested values here.
                Self::Text(serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string()))
            }
        }
    }

    /// Like `from_value`, but aware of the destination column name. If the
    /// column is one of the schema-level `INTEGER` unix-microsecond
    /// timestamp columns and the incoming value is an RFC-3339 string, the
    /// string is parsed back into a unix-microsecond `INTEGER` so the
    /// typed-record export shape (produced by `build_project_export_payload`)
    /// round-trips through `restore_branch_snapshot`. Other values fall
    /// through to `from_value`.
    fn from_value_for_column(column: &str, value: Value) -> Self {
        if is_timestamp_column(column)
            && let Value::String(ref s) = value
            && let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(s)
        {
            return Self::Integer(parsed.with_timezone(&chrono::Utc).timestamp_micros());
        }
        Self::from_value(value)
    }
}

/// True for every schema column declared as `INTEGER` unix-micros (the
/// `Timestamp` columns in `records.rs`). Used by `JsonParam::from_value_for_column`
/// to recognize RFC-3339 strings emitted by the typed-record export and
/// fold them back into the database's integer-micros representation.
fn is_timestamp_column(column: &str) -> bool {
    matches!(
        column,
        "created_at"
            | "updated_at"
            | "archived_at"
            | "imported_at"
            | "linked_at"
            | "resolved_at"
            | "snapshot_created_at"
    )
}

impl rusqlite::ToSql for JsonParam {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, ValueRef};
        Ok(match self {
            Self::Null => ToSqlOutput::Borrowed(ValueRef::Null),
            Self::Integer(i) => ToSqlOutput::Borrowed(ValueRef::Integer(*i)),
            Self::Real(f) => ToSqlOutput::Borrowed(ValueRef::Real(*f)),
            Self::Text(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
            Self::Blob(b) => ToSqlOutput::Borrowed(ValueRef::Blob(b)),
        })
    }
}

/// Reject anything that isn't a plain `[A-Za-z0-9_]+` identifier. Used to gate
/// the table/column names interpolated into the export-time `SELECT *`
/// statements (see `dump_project_table`).
fn ensure_safe_export_identifier(ident: &str) -> Result<()> {
    if ident.is_empty() || !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        anyhow::bail!("unsafe identifier for export query: {ident:?}");
    }
    Ok(())
}

/// Translate one `rusqlite::Row` into a `serde_json::Value::Object` keyed by
/// the supplied column names. See the dump-helpers comment block above for
/// the per-type mapping rules.
fn row_to_json_value(row: &rusqlite::Row<'_>, column_names: &[String]) -> rusqlite::Result<Value> {
    use rusqlite::types::ValueRef;
    let mut obj = serde_json::Map::with_capacity(column_names.len());
    for (idx, name) in column_names.iter().enumerate() {
        let v = match row.get_ref(idx)? {
            ValueRef::Null => Value::Null,
            ValueRef::Integer(i) => Value::Number(serde_json::Number::from(i)),
            ValueRef::Real(f) => serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
            ValueRef::Text(bytes) => {
                let s = std::str::from_utf8(bytes)
                    .map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            idx,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?
                    .to_string();
                if looks_like_json_column(name)
                    && let Ok(parsed) = serde_json::from_str::<Value>(&s)
                {
                    parsed
                } else {
                    Value::String(s)
                }
            }
            ValueRef::Blob(bytes) => {
                use base64::Engine;
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                Value::String(format!("base64:{encoded}"))
            }
        };
        obj.insert(name.clone(), v);
    }
    Ok(Value::Object(obj))
}

/// Heuristic: column names that hold JSON payloads in the V0001 schema.
/// Used to decide whether to re-parse `TEXT` values into `Value` rather than
/// emit them as escaped JSON strings. The list is conservative — false
/// negatives mean a JSON string survives as a string in the export, which is
/// the safer failure mode.
fn looks_like_json_column(name: &str) -> bool {
    matches!(
        name,
        "voice_profile"
            | "emotional_profile"
            | "voice_profile_data"
            | "emotional_profile_data"
            | "reader_contract"
            | "style_notes"
            | "boundaries"
            | "promise"
            | "emotional_state"
            | "goals"
            | "status"
            | "notes"
            | "warnings"
            | "milestones"
            | "starting_state"
            | "ending_state"
            | "thematic_purpose"
            | "connected_theme_ids"
            | "target_theme_ids"
            | "target_conflict_ids"
            | "target_plot_line_ids"
            | "motif_ids"
            | "theme_ids"
            | "conflict_ids"
            | "tags"
            | "aliases"
            | "relevance_tags"
            | "scan_pattern"
            | "dynamics"
            | "beats"
            | "planned_scenes"
            | "per_book_budget"
            | "progress"
            | "review_rounds"
            | "value_json"
            | "scope_constraints"
            | "context"
            | "established_in"
            | "story_placement"
            | "valid_from"
            | "valid_until"
            | "learned_at"
            | "flex_range"
            | "suppressed"
            | "triggers"
            | "defense_mechanisms"
            | "tics"
            | "vocabulary"
            | "sentence_structure"
            | "forbidden_words"
            | "example_lines"
            | "stated_consequences"
            | "try_fail_cycle"
    ) || name.ends_with("_json")
}

/// Mint a fresh `table:ulid` ID matching the SurrealDB record-id convention.
fn mint_id(table: &str) -> String {
    format!("{}:{}", table, Ulid::new())
}

fn fts_safe_query(query: &str) -> Option<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for c in query.chars() {
        if c.is_alphanumeric() {
            current.extend(c.to_lowercase());
        } else if c == '\'' || c == '’' {
            continue;
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    if tokens.is_empty() {
        None
    } else {
        Some(
            tokens
                .into_iter()
                .map(|token| format!("\"{token}\""))
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

/// Shared body of the `fts_search_*` methods. The snippet column index is
/// per-table: it's the position of the first indexed column in the table's
/// column list (UNINDEXED scene_id/project_id/branch_id come first, then the
/// indexed text columns).
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
async fn fts_search_named(
    pool: &SqlitePool,
    fts_table: &'static str,
    id_column: &'static str,
    snippet_col: i32,
    project_id: &str,
    branch_id: Option<&str>,
    query: &str,
    limit: usize,
) -> Result<Vec<(String, f64, String)>> {
    let project_id = project_id.to_string();
    let branch_id = branch_id.map(|s| s.to_string());
    let query = query.to_string();
    let limit = limit as i64;
    pool.read(move |conn| {
        let (sql, has_branch) = if branch_id.is_some() {
            (
                format!(
                    "SELECT {id_column}, rank, snippet({fts_table}, {snippet_col}, '<mark>', '</mark>', '…', 32) \
                     FROM {fts_table} \
                     WHERE {fts_table} MATCH ?1 AND project_id = ?2 AND branch_id = ?3 \
                     ORDER BY rank LIMIT ?4"
                ),
                true,
            )
        } else {
            (
                format!(
                    "SELECT {id_column}, rank, snippet({fts_table}, {snippet_col}, '<mark>', '</mark>', '…', 32) \
                     FROM {fts_table} \
                     WHERE {fts_table} MATCH ?1 AND project_id = ?2 \
                     ORDER BY rank LIMIT ?3"
                ),
                false,
            )
        };
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = if has_branch {
            stmt.query_map(
                rusqlite::params![&query, &project_id, &branch_id.unwrap(), limit],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(rusqlite::params![&query, &project_id, limit], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, r.get::<_, String>(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
    })
    .await
}

/// Tables that carry an `archived_at` column. Driven by the V0001 schema
/// (`V0001__initial_schema.sql`) plus V0033, which added `archived_at` to the
/// three research library tables so junk research entities are archivable
/// (live-run bug 5).
fn table_has_archived_at(table: &str) -> bool {
    matches!(
        table,
        "faction"
            | "religion"
            | "economy"
            | "term"
            | "plot_line"
            | "conflict"
            | "theme"
            | "motif"
            | "narrative_promise"
            | "character_arc"
            | "research_source"
            | "research_note"
            | "research_claim"
    )
}

/// Allowlist of (table, column) pairs that update_entity_field is permitted
/// to mutate. Limits surface area to text/json fields where partial overwrites
/// are safe; identity fields, FKs, and computed fields stay outside this list.
///
/// Audit policy (live-run bug round): authoring scaffolds a record first and
/// deepens it as decisions land, so every PLANNING field — the prose and
/// structure an author revises during planning — must be mutable after
/// creation. What stays locked:
///   * name-family identity columns (character/location/faction/… `name`,
///     `normalized_name`, `term_text`): they back project-wide uniqueness
///     indexes and the search index, so a rename is a controlled operation
///     (see the service's allow_rename path for characters), not a bare
///     column write;
///   * FKs and cross-record links (character_id, source/target_event_id,
///     subject refs);
///   * computed fields (character_arc.progress);
///   * columns with a dedicated typed write path (character voice profiles
///     via set_character_voice_profile, scene text via save/commit,
///     canonical facts via register/supersede).
fn column_is_updatable(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        // Common notes column.
        (_, "notes")
            // Free-form summaries / titles.
            | ("project", "name")
            | ("project", "genre")
            | ("project", "project_type")
            // Style-learning opt-in (V0031): enable capturing operator edits as
            // style-refresh candidates. 0/1 flag via update_entity_field's
            // numeric path; NULL/absent = disabled (evolution §3.9).
            | ("project", "style_learning")
            // Stub-scene word floor (V0036): the minimum scene length the
            // preflight/compile/consistency stub gates enforce. NULL/absent =
            // the built-in default floor.
            | ("project", "min_scene_word_count")
            | ("book", "title")
            | ("chapter", "title")
            | ("character", "summary")
            | ("character", "role")
            | ("character", "realm")
            | ("character", "appearance")
            // Alternate names (V0037): nicknames, titles, an in-world name
            // decided after the record exists. JSON array via the
            // update_entity_field path (same encoding as conflict's array
            // fields). The PRIMARY name is deliberately NOT here: renames go
            // through the service's allow_rename path, which moves the
            // normalized_name uniqueness key, preserves the old name as an
            // alias, refreshes the search index, and reports stale
            // references — none of which a bare column write can do.
            | ("character", "aliases")
            // Emotional profile (V0037 audit): create_character seeds it, but
            // most supporting cast are created before their psychology is
            // worked out — without these the profile is permanently frozen at
            // its create-time (usually empty) values. JSON columns via the
            // update_entity_field path; the service validates the shape of
            // each key against the stored profile types before writing so a
            // malformed value cannot poison the row readers.
            | ("character_emotional_profile", "base_emotions")
            | ("character_emotional_profile", "suppressed")
            | ("character_emotional_profile", "triggers")
            | ("character_emotional_profile", "defense_mechanisms")
            | ("character_emotional_profile", "flex_range")
            | ("location", "summary")
            | ("location", "realm")
            | ("location", "kind")
            | ("faction", "summary")
            // World-entity planning fields (audit): classification, realm,
            // and the JSON tag list are all revised during worldbuilding.
            // The `name` column stays locked (uniqueness index + search
            // index; renames need controlled semantics like characters).
            | ("faction", "faction_type")
            | ("faction", "realm")
            | ("faction", "tags")
            | ("religion", "summary")
            | ("religion", "deity_or_principle")
            | ("religion", "tags")
            | ("economy", "summary")
            | ("economy", "currency")
            | ("economy", "realm")
            | ("economy", "scarce_resources")
            | ("economy", "trade_goods")
            | ("term", "definition")
            | ("term", "pronunciation")
            | ("term", "usage_context")
            | ("term", "origin")
            | ("plot_line", "summary")
            | ("plot_line", "status")
            | ("plot_line", "plot_type")
            // Convergence audit inputs (V0022): the plot line's connected
            // conflicts/themes can be declared at create time or retrofitted
            // onto an existing row so plot_line_convergence_audit has something
            // to check against. JSON arrays via the update_entity_field path.
            | ("plot_line", "connected_conflict_ids")
            | ("plot_line", "connected_theme_ids")
            | ("plot_line", "convergence_points")
            | ("conflict", "stakes")
            | ("conflict", "resolution_summary")
            // Rich array fields. Settable at create_conflict time; without
            // these in the allowlist, callers cannot retroactively populate
            // try-fail cycles or escalation stages after the row exists —
            // which is exactly the workflow continuity-editor prompts for
            // ("add more failed attempts before resolution"). The JSON
            // encoding path in update_entity_field handles array/object
            // values via serde_json::to_string, and the conflict CHECK
            // constraints already require json_valid on these columns.
            | ("conflict", "try_fail_cycles")
            | ("conflict", "escalation_stages")
            | ("conflict", "stated_consequences")
            | ("conflict", "expected_total_cycles")
            | ("conflict", "conflict_type")
            // Per-stage demonstration markers (V0022), index-aligned with
            // escalation_stages. Settable after the row exists so the author
            // can mark a stage demonstrated as they write; drives
            // conflict_escalation_audit. JSON array via update_entity_field.
            | ("conflict", "escalation_demonstrated")
            // Milestone `reached_at` lives inside this JSON blob; the arc
            // update path resends the whole milestones array with the marker
            // set (no per-milestone merge). Drives arc_milestone_audit.
            | ("character_arc", "milestones")
            // Arc planning fields (audit): an arc's destination is among the
            // likeliest things to change during planning — recording it early
            // so it can be revised is the reason to write an arc down before
            // drafting. `status` gates which arcs project into snapshots
            // ("active"); the rest are plain planning prose/JSON. `progress`
            // stays locked (computed), as do the id links.
            | ("character_arc", "arc_type")
            | ("character_arc", "starting_state")
            | ("character_arc", "ending_state")
            | ("character_arc", "thematic_purpose")
            | ("character_arc", "connected_theme_ids")
            | ("character_arc", "status")
            | ("theme", "theme_statement")
            | ("theme", "thesis_antithesis")
            // Placement JSON objects (same encoding as
            // narrative_promise.planned_payoff): where the theme lands.
            | ("theme", "introduction_point")
            | ("theme", "resolution_point")
            | ("motif", "description")
            | ("motif", "max_uses_per_chapter")
            | ("motif", "connected_theme_ids")
            | ("narrative_promise", "description")
            | ("narrative_promise", "status")
            // Payoff retargeting after a chapter renumber/restructure: the
            // placement is a JSON object via update_entity_field's JSON path
            // (same encoding the row reader uses via row::opt_json), so a
            // promise can be re-aimed at its new payoff chapter. Without this,
            // narrative_promise_tracking's "past planned payoff" warnings are
            // unactionable — update_promise_status takes only status/note.
            | ("narrative_promise", "planned_payoff")
            // Planting placement retargeting — symmetric with planned_payoff:
            // a renumber can move where the promise was PLANTED just as easily
            // as where it pays off, and the tracking advice reads both.
            | ("narrative_promise", "planted_at")
            // Scene summaries are drafted at save_scene_draft time but go
            // stale after renumbers/re-cuts (they carry cross-references like
            // "the knife from Chapter 3"). They feed get_chapter_briefing, so
            // the drift propagates into drafting context unless the author
            // can correct them in place.
            | ("scene", "summary")
            | ("world_rule", "description")
            // World-rule planning fields (audit): update_world_rule routes
            // through this same allowlist and its cache invalidation already
            // anticipates rule_type edits, but only `description` was
            // actually writable. scan_pattern feeds the drift scanner (the
            // cache target resolves WorldRuleSemanticDrift on any update);
            // established_in/relevance_tags are JSON via the same path.
            // rule_name stays locked (uniqueness index + search index).
            | ("world_rule", "rule_type")
            | ("world_rule", "scan_pattern")
            | ("world_rule", "established_in")
            | ("world_rule", "relevance_tags")
            // Timeline events (audit): entirely locked before — a renumber
            // stranded their placement and there was no path to fix title or
            // summary drift. related_entity_ids is the JSON id list;
            // event_type classifies. RetconReachability cache invalidation
            // already covers timeline_event/temporal_intervention updates.
            | ("timeline_event", "title")
            | ("timeline_event", "event_type")
            | ("timeline_event", "placement")
            | ("timeline_event", "summary")
            | ("timeline_event", "related_entity_ids")
            | ("temporal_intervention", "intervention_type")
            | ("temporal_intervention", "summary")
            | ("temporal_intervention", "consequences")
            | ("temporal_intervention", "status")
            // System overlays (audit): every field was locked after create,
            // so progression-system tuning (tiers, stats, visibility) had no
            // write path. system_name stays locked (uniqueness index + search
            // index); the FK-free planning columns are all fair game.
            | ("system_overlay", "system_type")
            | ("system_overlay", "rules")
            | ("system_overlay", "visibility")
            | ("system_overlay", "progression_currency")
            | ("system_overlay", "stats")
            | ("system_overlay", "advancement_tiers")
            // Future knowledge (audit): time-displaced facts get revised as
            // the timeline logic firms up; learned_at/expires_at are the JSON
            // placements that drive the invalidation checks. character_id
            // (the holder FK) stays locked.
            | ("future_knowledge", "knowledge_summary")
            | ("future_knowledge", "source")
            | ("future_knowledge", "learned_at")
            | ("future_knowledge", "expires_at")
            // Secret-knowledge gating (V0023): retrofit an existing fact into a
            // secret (or clear it) without a data migration. `secret` is a
            // 0/1 flag via update_entity_field's numeric/bool path;
            // `concealment_note` is free-form guidance. Pairs with
            // `record_knowledge`'s secret_of_fact_id link to build the circle
            // (design §2.1: "existing secrets ... retrofit ... without migration").
            | ("canonical_fact", "secret")
            | ("canonical_fact", "concealment_note")
    )
}

/// Local clone of `mint_id` usable inside `pool.write` closures, where the
/// outer `mint_id` symbol isn't in the closure's import path. Identical
/// behavior — separate name avoids the captured-symbol headache.
fn mint_id_local(table: &str) -> String {
    mint_id(table)
}

/// Rename a character INSIDE the caller's transaction so the name move, the
/// normalized-name uniqueness key, and the old-name-as-alias preservation all
/// commit together (or not at all). Returns the updated row plus whether the
/// old name was added as an alias. Collision handling mirrors the project-wide
/// unique index `idx_character_project_name(project_id, normalized_name)`.
fn rename_character_in_tx(
    tx: &rusqlite::Transaction<'_>,
    character_id: &str,
    new_name: &str,
) -> anyhow::Result<(Character, bool)> {
    let sql = format!("SELECT {CHARACTER_COLUMNS} FROM character WHERE id = ?1");
    let current = {
        let mut stmt = tx.prepare_cached(&sql)?;
        stmt.query_row([character_id], |r| Character::try_from(r))
            .optional_inner()
    }?
    .ok_or_else(|| anyhow::anyhow!("character not found"))?;

    let normalized = normalize_name(new_name);
    if normalized != current.normalized_name {
        let clash: Option<String> = {
            let mut stmt = tx.prepare_cached(
                "SELECT id FROM character \
                 WHERE project_id = ?1 AND normalized_name = ?2 AND id != ?3",
            )?;
            stmt.query_row(
                rusqlite::params![&current.project_id, &normalized, character_id],
                |r| r.get(0),
            )
            .optional_inner()
        }?;
        if let Some(other_id) = clash {
            anyhow::bail!(
                "a character named '{new_name}' already exists in this project ({other_id}); \
                 pick a distinct name or rename/archive the other character first"
            );
        }
    }

    // Preserve the old name as an alias so existing prose and knowledge stay
    // resolvable — but only when the rename is a real name change and the old
    // name isn't aliased already.
    let old_name_kept_as_alias = normalize_name(&current.name) != normalized
        && !current
            .aliases
            .iter()
            .any(|alias| normalize_name(alias) == normalize_name(&current.name));
    let mut aliases = current.aliases.clone();
    if old_name_kept_as_alias {
        aliases.push(current.name.clone());
    }
    let aliases_json = serde_json::to_string(&aliases)?;
    let now = timestamp_to_micros(chrono::Utc::now());
    tx.execute(
        "UPDATE character SET name = ?1, normalized_name = ?2, aliases = ?3, \
         updated_at = ?4 WHERE id = ?5",
        rusqlite::params![new_name, &normalized, &aliases_json, now, character_id],
    )?;
    let updated = {
        let mut stmt = tx.prepare_cached(&sql)?;
        stmt.query_row([character_id], |r| Character::try_from(r))?
    };
    Ok((updated, old_name_kept_as_alias))
}

/// Bind a generation receipt to the one scene it authorizes (migration
/// V0034), INSIDE the caller's transaction so the claim commits if and only
/// if the scene write commits. A save that fails after this point rolls the
/// claim back and the receipt stays unbound and reusable — the live-run bug
/// was a standalone claim that survived a failed save and burned the receipt
/// on the bogus placement.
///
/// An idempotent re-save of the same scene matches the
/// `claimed_scene_key = ?2` arm and passes. A receipt already spent on a
/// DIFFERENT scene matches zero rows; the conflicting key is read back and
/// the save is rejected with the operator-facing message. Two concurrent
/// saves racing for one receipt cannot both win: SQLite serializes the write
/// transactions, so the loser sees the winner's claim and bails.
fn claim_generation_receipt_in_tx(
    tx: &rusqlite::Transaction<'_>,
    receipt_id: &str,
    scene_key: &str,
) -> anyhow::Result<()> {
    let claimed = tx.execute(
        "UPDATE generation_receipt SET claimed_scene_key = ?2 \
         WHERE id = ?1 \
           AND (claimed_scene_key IS NULL OR claimed_scene_key = ?2)",
        rusqlite::params![receipt_id, scene_key],
    )?;
    if claimed > 0 {
        return Ok(());
    }
    // Zero rows updated: either the receipt is claimed elsewhere, or it no
    // longer exists (expired and swept between the verify and this claim).
    let mut stmt =
        tx.prepare_cached("SELECT claimed_scene_key FROM generation_receipt WHERE id = ?1")?;
    let mut rows = stmt.query(rusqlite::params![receipt_id])?;
    match rows.next()? {
        Some(row) => {
            let claimed_key = row.get::<_, Option<String>>(0)?;
            match claimed_key {
                Some(key) if key != scene_key => anyhow::bail!(
                    "generation_id {:?} already authorized a different scene ({}); \
                     each explicit save needs its own receipt — call continue_generation \
                     with route \"draft\" and rating \"explicit\" for this scene",
                    receipt_id,
                    describe_receipt_scene_key(&key)
                ),
                // Claimed by this very scene: idempotent re-save (matched
                // above normally; reached only on a racing writer).
                Some(_) => Ok(()),
                // Unclaimed but the UPDATE matched nothing: the receipt row
                // vanished between verification and claim (expiry sweep).
                None => anyhow::bail!(
                    "generation_id {:?} was not found or has expired",
                    receipt_id
                ),
            }
        }
        None => anyhow::bail!(
            "generation_id {:?} was not found or has expired",
            receipt_id
        ),
    }
}

/// Render a stored claim key back into something an operator can act on.
/// Falls back to the raw key if it is not the expected 4-part shape, so a
/// future key format change degrades to "unhelpful but honest" rather than a
/// panic.
pub(crate) fn describe_receipt_scene_key(key: &str) -> String {
    let parts: Vec<&str> = key.split('|').collect();
    match parts.as_slice() {
        [_project, book, chapter, scene] => {
            format!("book {book}, chapter {chapter}, scene {scene}")
        }
        _ => key.to_string(),
    }
}

/// Walk `bible_branch.parent_branch_id` starting at `branch_id` up to `main`,
/// returning `[branch_id, parent_id, ..., main_id]`. Child branches appear
/// before their ancestors so callers can apply child-overrides-parent merge
/// semantics. If `branch_id` is not found in `bible_branch`, still returns
/// `[branch_id]` (plus the project's `main` branch if `project_id` is known).
pub(crate) fn resolve_branch_lineage_in_conn(
    conn: &rusqlite::Connection,
    project_id: Option<&str>,
    branch_id: &str,
) -> rusqlite::Result<Vec<String>> {
    let mut chain = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut current = Some(branch_id.to_string());
    let mut resolved_project_id = project_id.map(str::to_owned);
    let mut stmt =
        conn.prepare_cached("SELECT parent_branch_id, project_id FROM bible_branch WHERE id = ?1")?;
    while let Some(bid) = current {
        if !visited.insert(bid.clone()) {
            break;
        }
        chain.push(bid.clone());
        let row: Option<(Option<String>, String)> = stmt
            .query_row([&bid], |r| {
                Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
            })
            .optional_inner()?;
        match row {
            Some((parent, pid)) => {
                if resolved_project_id.is_none() {
                    resolved_project_id = Some(pid);
                }
                current = parent;
            }
            None => break,
        }
    }
    if let Some(pid) = resolved_project_id
        && let Some(main_id) = conn
            .query_row(
                "SELECT id FROM bible_branch WHERE project_id = ?1 AND name = 'main' LIMIT 1",
                [&pid],
                |r| r.get::<_, String>(0),
            )
            .optional_inner()?
        && !visited.contains(&main_id)
    {
        chain.push(main_id);
    }
    Ok(chain)
}

/// Resolve the target branch for canon checks and writes anchored to a scene.
/// If the project's active branch is `scene_branch_id` or a descendant branch
/// that inherits from `scene_branch_id`, returns the active branch; otherwise
/// returns `scene_branch_id`.
pub(crate) fn resolve_effective_scene_branch_in_conn(
    conn: &rusqlite::Connection,
    project_id: &str,
    scene_branch_id: &str,
) -> rusqlite::Result<String> {
    let active_branch_id: Option<String> = conn
        .query_row(
            "SELECT active_branch_id FROM project WHERE id = ?1",
            rusqlite::params![project_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional_inner()?
        .flatten();
    if let Some(active_id) = active_branch_id {
        if active_id == scene_branch_id {
            return Ok(active_id);
        }
        let chain = resolve_branch_lineage_in_conn(conn, Some(project_id), &active_id)?;
        if chain.iter().any(|b| b == scene_branch_id) {
            return Ok(active_id);
        }
    }
    Ok(scene_branch_id.to_string())
}

/// Canonical unordered pair key for a relationship between two characters.
pub(crate) fn relationship_pair_key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

/// Resolve the most recent `character_state` at or before `(book, chapter, scene_order)`
/// across `branch_id` and its ancestor chain, with nearer branches breaking ties
/// at the same narrative position.
pub(crate) fn resolve_character_state_for_branch_in_conn(
    conn: &rusqlite::Connection,
    branch_id: &str,
    character_id: &str,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> rusqlite::Result<Option<CharacterState>> {
    let chain = resolve_branch_lineage_in_conn(conn, None, branch_id)?;
    let sql = format!(
        "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
         WHERE branch_id = ?1 AND character_id = ?2 \
           AND ( \
                book_number < ?3 \
             OR (book_number = ?3 AND chapter_number < ?4) \
             OR (book_number = ?3 AND chapter_number = ?4 AND scene_order <= ?5) \
           ) \
         ORDER BY book_number DESC, chapter_number DESC, scene_order DESC, created_at DESC, rowid DESC \
         LIMIT 1"
    );
    let mut stmt = conn.prepare_cached(&sql)?;
    let mut best: Option<((i32, i32, i32), usize, CharacterState)> = None;
    for (depth, candidate_branch_id) in chain.iter().enumerate() {
        if let Some(state) = stmt
            .query_row(
                rusqlite::params![
                    candidate_branch_id,
                    character_id,
                    book_number,
                    chapter_number,
                    scene_order
                ],
                |r| CharacterState::try_from(r),
            )
            .optional_inner()?
        {
            let pos = (state.book_number, state.chapter_number, state.scene_order);
            let replace = match &best {
                None => true,
                Some((best_pos, best_depth, _)) => {
                    pos > *best_pos || (pos == *best_pos && depth < *best_depth)
                }
            };
            if replace {
                best = Some((pos, depth, state));
            }
        }
    }
    Ok(best.map(|(_, _, state)| state))
}

/// Repository-level unit-of-work wrapping a single SQLite writer transaction.
/// Used by multi-step canon write operations (`commit_scene_changes`,
/// `commit_character_state`, `register_canonical_fact`, `record_knowledge`) so
/// any mid-way failure rolls back the entire batch with zero partial writes.
pub struct CanonTransaction<'a> {
    tx: rusqlite::Transaction<'a>,
}

impl CanonTransaction<'_> {
    /// Borrow the underlying write transaction so sibling repositories can add
    /// their own in-transaction statements without opening a second connection.
    pub(crate) fn raw_tx(&self) -> &rusqlite::Transaction<'_> {
        &self.tx
    }

    pub fn get_character(&self, id: &str) -> Result<Character> {
        let sql = format!("SELECT {CHARACTER_COLUMNS} FROM character WHERE id = ?1");
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row([id], |r| Character::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("character not found"))
    }

    pub fn get_scene(&self, id: &str) -> Result<Scene> {
        let sql = format!("SELECT {SCENE_COLUMNS} FROM scene WHERE id = ?1");
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row([id], |r| Scene::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("scene not found"))
    }

    pub fn get_canonical_fact(&self, id: &str) -> Result<CanonicalFact> {
        let sql = format!("SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact WHERE id = ?1");
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row([id], |r| CanonicalFact::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("canonical_fact not found"))
    }

    pub fn resolve_character_state_for_branch(
        &self,
        branch_id: &str,
        character_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<CharacterState>> {
        Ok(resolve_character_state_for_branch_in_conn(
            &self.tx,
            branch_id,
            character_id,
            book_number,
            chapter_number,
            scene_order,
        )?)
    }

    pub fn append_character_state(
        &self,
        params: AppendCharacterStateParams,
    ) -> Result<CharacterState> {
        let id = mint_id("character_state");
        let emotional_json = serde_json::to_string(&params.patch.emotional_state)
            .context("serializing emotional_state")?;
        let goals_json = serde_json::to_string(&params.patch.goals.unwrap_or_default())
            .context("serializing goals")?;
        let status_json = serde_json::to_string(&params.patch.status.unwrap_or_default())
            .context("serializing status")?;
        let notes_json = serde_json::to_string(&params.patch.notes.unwrap_or_default())
            .context("serializing notes")?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.tx.execute(
            "INSERT INTO character_state (id, project_id, branch_id, character_id, \
             scene_id, book_number, chapter_number, scene_order, emotional_state, \
             goals, status, notes, source_summary, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            rusqlite::params![
                &id,
                &params.project_id,
                &params.branch_id,
                &params.character_id,
                &params.scene_id,
                params.book_number,
                params.chapter_number,
                params.scene_order,
                &emotional_json,
                &goals_json,
                &status_json,
                &notes_json,
                &params.patch.source_summary,
                now,
            ],
        )?;
        let sql = format!("SELECT {CHARACTER_STATE_COLUMNS} FROM character_state WHERE id = ?1");
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row([&id], |r| CharacterState::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("character_state vanished after insert"))
    }

    pub fn append_progression_event(
        &self,
        params: AppendProgressionEventParams,
    ) -> Result<ProgressionEvent> {
        let id = mint_id("progression_event");
        let delta_json = serde_json::to_string(&params.delta_json)?;
        let placement_stored: Option<StoredStoryPlacement> = params.placement.map(Into::into);
        let placement_json = match placement_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let created_at = timestamp_to_micros(params.created_at);
        self.tx.execute(
            "INSERT INTO progression_event (id, project_id, branch_id, subject_table, \
             subject_id, overlay_id, kind, delta_json, source_scene_id, placement, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                &id,
                &params.project_id,
                &params.branch_id,
                &params.subject_table,
                &params.subject_id,
                &params.overlay_id,
                &params.kind,
                &delta_json,
                &params.source_scene_id,
                &placement_json,
                created_at,
            ],
        )?;
        let sql =
            format!("SELECT {PROGRESSION_EVENT_COLUMNS} FROM progression_event WHERE id = ?1");
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row([&id], |r| ProgressionEvent::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("progression_event not found"))
    }

    pub fn create_canonical_fact(
        &self,
        params: CreateCanonicalFactParams,
    ) -> Result<CanonicalFact> {
        let id = mint_id("canonical_fact");
        let aliases_json = serde_json::to_string(&params.aliases)?;
        let value_json_str = match params.value_json.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let valid_from_stored: Option<StoredStoryPlacement> = params.valid_from.map(Into::into);
        let valid_from_json = match valid_from_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let valid_until_stored: Option<StoredStoryPlacement> = params.valid_until.map(Into::into);
        let valid_until_json = match valid_until_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let secret_flag = if params.secret { 1 } else { 0 };
        self.tx.execute(
            "INSERT INTO canonical_fact (id, project_id, branch_id, scene_id, \
             source_scene_id, book_number, chapter_number, subject_table, subject_id, \
             predicate, value_kind, value_number, value_text, value_json, unit, aliases, \
             scope, valid_from, valid_until, superseded_by, created_at, updated_at, \
             secret, concealment_note) \
             VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                     ?15, ?16, ?17, ?18, NULL, ?19, ?19, ?20, ?21)",
            rusqlite::params![
                &id,
                &params.project_id,
                &params.branch_id,
                &params.scene_id,
                params.book_number,
                params.chapter_number,
                &params.subject_table,
                &params.subject_id,
                &params.predicate,
                &params.value_kind,
                params.value_number,
                &params.value_text,
                &value_json_str,
                &params.unit,
                &aliases_json,
                &params.scope,
                &valid_from_json,
                &valid_until_json,
                now,
                secret_flag,
                &params.concealment_note,
            ],
        )?;
        self.get_canonical_fact(&id)
    }

    pub fn supersede_canonical_fact(
        &self,
        branch_id: &str,
        old_fact_id: &str,
        new_fact_id: &str,
    ) -> Result<()> {
        let old_branch: Option<String> = self
            .tx
            .query_row(
                "SELECT branch_id FROM canonical_fact WHERE id = ?1",
                [old_fact_id],
                |r| r.get::<_, String>(0),
            )
            .optional_inner()?;
        let Some(old_branch_id) = old_branch else {
            anyhow::bail!("superseded canonical_fact '{old_fact_id}' not found");
        };
        if old_branch_id == branch_id {
            let now = timestamp_to_micros(chrono::Utc::now());
            self.tx.execute(
                "UPDATE canonical_fact SET superseded_by = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![new_fact_id, now, old_fact_id],
            )?;
        } else {
            let lineage = resolve_branch_lineage_in_conn(&self.tx, None, branch_id)?;
            anyhow::ensure!(
                lineage.iter().any(|b| b == &old_branch_id),
                "superseded canonical_fact '{old_fact_id}' does not belong to branch '{branch_id}' or its ancestors"
            );
        }
        self.tx.execute(
            "UPDATE canonical_fact SET supersedes_fact_id = ?1 WHERE id = ?2 AND branch_id = ?3",
            rusqlite::params![old_fact_id, new_fact_id, branch_id],
        )?;
        Ok(())
    }

    pub fn link_secret_holder(&self, params: LinkSecretHolderParams) -> Result<KnowledgeFact> {
        let now = timestamp_to_micros(chrono::Utc::now());
        let tags_json = serde_json::to_string::<Vec<String>>(&Vec::new())?;
        let existing_id: Option<String> = self
            .tx
            .query_row(
                "SELECT id FROM knowledge_fact \
                 WHERE project_id = ?1 AND branch_id = ?2 AND character_id = ?3 \
                   AND normalized_fact = ?4",
                rusqlite::params![
                    &params.project_id,
                    &params.branch_id,
                    &params.character_id,
                    &params.normalized_fact
                ],
                |r| r.get::<_, String>(0),
            )
            .optional_inner()?;

        let linked_id = match existing_id {
            Some(id) => {
                self.tx.execute(
                    "UPDATE knowledge_fact \
                     SET secret_of_fact_id = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&params.secret_of_fact_id, now, &id],
                )?;
                id
            }
            None => {
                let id = mint_id_local("knowledge_fact");
                self.tx.execute(
                    "INSERT INTO knowledge_fact (id, project_id, branch_id, character_id, \
                     fact, normalized_fact, source_summary, learned_at, confidence, tags, \
                     reader_visible, source_import_session_id, created_at, updated_at, \
                     secret_of_fact_id) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, 1, NULL, ?9, ?9, ?10)",
                    rusqlite::params![
                        &id,
                        &params.project_id,
                        &params.branch_id,
                        &params.character_id,
                        &params.fact_text,
                        &params.normalized_fact,
                        &params.source_summary,
                        &tags_json,
                        now,
                        &params.secret_of_fact_id,
                    ],
                )?;
                id
            }
        };
        let sql = format!("SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact WHERE id = ?1");
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row([&linked_id], |r| KnowledgeFact::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("knowledge_fact not found"))
    }

    pub fn upsert_knowledge_fact(
        &self,
        params: UpsertKnowledgeFactParams,
    ) -> Result<KnowledgeFact> {
        let id = mint_id("knowledge_fact");
        let learned_at_stored: Option<StoredStoryPlacement> = params.learned_at.map(Into::into);
        let learned_at_json = match learned_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let tags_json = serde_json::to_string(&params.tags)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        let reader_visible = if params.reader_visible { 1 } else { 0 };
        self.tx.execute(
            "DELETE FROM knowledge_fact \
             WHERE project_id = ?1 AND branch_id = ?2 AND character_id = ?3 \
               AND normalized_fact = ?4",
            rusqlite::params![
                &params.project_id,
                &params.branch_id,
                &params.character_id,
                &params.normalized_fact
            ],
        )?;
        self.tx.execute(
            "INSERT INTO knowledge_fact (id, project_id, branch_id, character_id, \
             fact, normalized_fact, source_summary, learned_at, confidence, tags, \
             reader_visible, source_import_session_id, created_at, updated_at, \
             secret_of_fact_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?14)",
            rusqlite::params![
                &id,
                &params.project_id,
                &params.branch_id,
                &params.character_id,
                &params.fact,
                &params.normalized_fact,
                &params.source_summary,
                &learned_at_json,
                params.confidence,
                &tags_json,
                reader_visible,
                &params.source_import_session_id,
                now,
                &params.secret_of_fact_id,
            ],
        )?;
        let sql = format!("SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact WHERE id = ?1");
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row([&id], |r| KnowledgeFact::try_from(r))
            .optional_inner()?
            .ok_or_else(|| anyhow!("knowledge_fact not found"))
    }

    pub fn upsert_knows(&self, params: UpsertKnowsParams) -> Result<Knows> {
        let learned_at_stored: Option<StoredStoryPlacement> = params.learned_at.map(Into::into);
        let learned_at_json = match learned_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let reader_visible = if params.reader_visible { 1 } else { 0 };
        self.tx.execute(
            "INSERT INTO knows (in_id, out_id, project_id, branch_id, source_summary, \
             learned_at, confidence, reader_visible, source_import_session_id, \
             created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
             ON CONFLICT (branch_id, in_id, out_id) DO UPDATE SET \
                 source_summary = excluded.source_summary, \
                 learned_at = excluded.learned_at, \
                 confidence = excluded.confidence, \
                 reader_visible = excluded.reader_visible, \
                 source_import_session_id = excluded.source_import_session_id, \
                 updated_at = excluded.updated_at",
            rusqlite::params![
                &params.character_id,
                &params.knowledge_fact_id,
                &params.project_id,
                &params.branch_id,
                &params.source_summary,
                &learned_at_json,
                params.confidence,
                reader_visible,
                &params.source_import_session_id,
                now,
            ],
        )?;
        let sql = format!(
            "SELECT {KNOWS_COLUMNS} FROM knows \
             WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3"
        );
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row(
            rusqlite::params![
                &params.branch_id,
                &params.character_id,
                &params.knowledge_fact_id
            ],
            |r| Knows::try_from(r),
        )
        .optional_inner()?
        .ok_or_else(|| anyhow!("knows edge not found"))
    }

    pub fn update_relationship(
        &self,
        branch_id: &str,
        input: &UpdateRelationshipInput,
        source: &str,
    ) -> Result<RelatesTo> {
        let a = &input.character_a_id;
        let b = &input.character_b_id;
        let trust_delta = input.trust_delta;
        let tension_delta = input.tension_delta;
        let reason = &input.reason;
        let scene_id = &input.scene_id;
        let now = timestamp_to_micros(chrono::Utc::now());

        let scene_pos: Option<(String, i32, i32, i32)> = self
            .tx
            .query_row(
                "SELECT project_id, book_number, chapter_number, scene_order \
                 FROM scene WHERE id = ?1",
                [scene_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i32>(1)?,
                        r.get::<_, i32>(2)?,
                        r.get::<_, i32>(3)?,
                    ))
                },
            )
            .optional_inner()?;
        let project_id_hint = scene_pos.as_ref().map(|(pid, _, _, _)| pid.as_str());
        let chain = resolve_branch_lineage_in_conn(&self.tx, project_id_hint, branch_id)?;

        // Check for an existing row on `branch_id` first.
        let existing_local: Option<(String, String)> = self
            .tx
            .query_row(
                "SELECT in_id, out_id FROM relates_to \
                 WHERE branch_id = ?1 \
                   AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                 LIMIT 1",
                rusqlite::params![branch_id, a, b],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional_inner()?;

        // If `branch_id` does not yet have a row, check ancestor branches so a
        // child branch inherits the parent baseline before applying deltas.
        // When materializing a child row from an ancestor, copy that ancestor's
        // `relationship_scene_delta` rows onto `branch_id` so the child's local
        // ledger captures the exact set of inherited scene deltas included in
        // its materialized baseline (isolating it from later ancestor updates).
        let mut inherited_ancestor: Option<(String, String, String, String, i32, i32, String)> =
            None;
        if existing_local.is_none() {
            for ancestor_branch_id in chain.iter().skip(1) {
                if let Some((anc_in, anc_out, rel_type, base_trust, base_tension, dynamics)) = self
                    .tx
                    .query_row(
                        "SELECT in_id, out_id, relationship_type, trust, tension, dynamics \
                         FROM relates_to \
                         WHERE branch_id = ?1 \
                           AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                         LIMIT 1",
                        rusqlite::params![ancestor_branch_id, a, b],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, i32>(3)?,
                                r.get::<_, i32>(4)?,
                                r.get::<_, String>(5)?,
                            ))
                        },
                    )
                    .optional_inner()?
                {
                    inherited_ancestor = Some((
                        ancestor_branch_id.clone(),
                        anc_in,
                        anc_out,
                        rel_type,
                        base_trust,
                        base_tension,
                        dynamics,
                    ));
                    break;
                }
            }

            if let Some((ref anc_branch_id, ref anc_in, ref anc_out, ..)) = inherited_ancestor {
                self.tx.execute(
                    "DELETE FROM relationship_scene_delta \
                     WHERE branch_id = ?1 \
                       AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2))",
                    rusqlite::params![branch_id, anc_in, anc_out],
                )?;
                self.tx.execute(
                    "INSERT OR REPLACE INTO relationship_scene_delta ( \
                        branch_id, in_id, out_id, scene_id, source, \
                        book_number, chapter_number, scene_order, \
                        trust_delta, tension_delta, reason, updated_at \
                     ) \
                     SELECT ?1, in_id, out_id, scene_id, source, \
                            book_number, chapter_number, scene_order, \
                            trust_delta, tension_delta, reason, updated_at \
                     FROM relationship_scene_delta \
                     WHERE branch_id = ?2 \
                       AND ((in_id = ?3 AND out_id = ?4) OR (in_id = ?4 AND out_id = ?3))",
                    rusqlite::params![branch_id, anc_branch_id, anc_in, anc_out],
                )?;
            } else {
                self.tx.execute(
                    "DELETE FROM relationship_scene_delta \
                     WHERE branch_id = ?1 \
                       AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2))",
                    rusqlite::params![branch_id, a, b],
                )?;
            }
        }

        // Look up any prior delta recorded for this pair + scene (by scene_id or
        // by narrative position when a revised scene replaced the scene row)
        // in `branch_id`'s own ledger, which reflects only the deltas included
        // in `branch_id`'s effective baseline.
        let prior_delta: Option<(i32, i32)> = if let Some((_, book_num, chap_num, sc_order)) =
            &scene_pos
        {
            self.tx
                    .query_row(
                        "SELECT trust_delta, tension_delta FROM relationship_scene_delta \
                         WHERE branch_id = ?1 \
                           AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                           AND (scene_id = ?4 \
                                OR (book_number = ?5 AND chapter_number = ?6 AND scene_order = ?7)) \
                         ORDER BY updated_at DESC LIMIT 1",
                        rusqlite::params![
                            branch_id, a, b, scene_id, book_num, chap_num, sc_order
                        ],
                        |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i32>(1)?)),
                    )
                    .optional_inner()?
        } else {
            self.tx
                .query_row(
                    "SELECT trust_delta, tension_delta FROM relationship_scene_delta \
                         WHERE branch_id = ?1 \
                           AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                           AND scene_id = ?4 \
                         ORDER BY updated_at DESC LIMIT 1",
                    rusqlite::params![branch_id, a, b, scene_id],
                    |r| Ok((r.get::<_, i32>(0)?, r.get::<_, i32>(1)?)),
                )
                .optional_inner()?
        };

        let (prior_trust, prior_tension) = prior_delta.unwrap_or((0, 0));
        let net_trust = trust_delta - prior_trust;
        let net_tension = tension_delta - prior_tension;

        let (in_id, out_id) = if let Some((in_id, out_id)) = existing_local {
            self.tx.execute(
                "UPDATE relates_to SET \
                    trust = trust + ?1, \
                    tension = tension + ?2, \
                    reason = ?3, \
                    last_scene_id = ?4, \
                    updated_at = ?5 \
                 WHERE branch_id = ?6 AND in_id = ?7 AND out_id = ?8",
                rusqlite::params![
                    net_trust,
                    net_tension,
                    reason,
                    scene_id,
                    now,
                    branch_id,
                    &in_id,
                    &out_id,
                ],
            )?;
            (in_id, out_id)
        } else if let Some((_, anc_in, anc_out, rel_type, base_trust, base_tension, dynamics)) =
            inherited_ancestor
        {
            self.tx.execute(
                "INSERT INTO relates_to (in_id, out_id, branch_id, \
                 relationship_type, trust, tension, dynamics, reason, last_scene_id, \
                 updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    &anc_in,
                    &anc_out,
                    branch_id,
                    &rel_type,
                    base_trust + net_trust,
                    base_tension + net_tension,
                    &dynamics,
                    reason,
                    scene_id,
                    now,
                ],
            )?;
            (anc_in, anc_out)
        } else {
            self.tx.execute(
                "INSERT INTO relates_to (in_id, out_id, branch_id, \
                 relationship_type, trust, tension, dynamics, reason, last_scene_id, \
                 updated_at) \
                 VALUES (?1, ?2, ?3, 'scene_dynamic', ?4, ?5, '[]', ?6, ?7, ?8)",
                rusqlite::params![
                    a,
                    b,
                    branch_id,
                    net_trust,
                    net_tension,
                    reason,
                    scene_id,
                    now,
                ],
            )?;
            (a.clone(), b.clone())
        };

        // Replace any prior delta row on `branch_id` for this pair + scene.
        if let Some((_, book_num, chap_num, sc_order)) = &scene_pos {
            self.tx.execute(
                "DELETE FROM relationship_scene_delta \
                 WHERE branch_id = ?1 \
                   AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                   AND (scene_id = ?4 \
                        OR (book_number = ?5 AND chapter_number = ?6 AND scene_order = ?7))",
                rusqlite::params![
                    branch_id, &in_id, &out_id, scene_id, book_num, chap_num, sc_order
                ],
            )?;
            self.tx.execute(
                "INSERT INTO relationship_scene_delta \
                 (branch_id, in_id, out_id, scene_id, source, book_number, chapter_number, \
                  scene_order, trust_delta, tension_delta, reason, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                rusqlite::params![
                    branch_id,
                    &in_id,
                    &out_id,
                    scene_id,
                    source,
                    book_num,
                    chap_num,
                    sc_order,
                    trust_delta,
                    tension_delta,
                    reason,
                    now,
                ],
            )?;
        } else {
            self.tx.execute(
                "DELETE FROM relationship_scene_delta \
                 WHERE branch_id = ?1 \
                   AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                   AND scene_id = ?4",
                rusqlite::params![branch_id, &in_id, &out_id, scene_id],
            )?;
            self.tx.execute(
                "INSERT INTO relationship_scene_delta \
                 (branch_id, in_id, out_id, scene_id, source, book_number, chapter_number, \
                  scene_order, trust_delta, tension_delta, reason, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0, 0, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    branch_id,
                    &in_id,
                    &out_id,
                    scene_id,
                    source,
                    trust_delta,
                    tension_delta,
                    reason,
                    now,
                ],
            )?;
        }

        let sql = format!(
            "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
             WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3"
        );
        let mut stmt = self.tx.prepare_cached(&sql)?;
        stmt.query_row(rusqlite::params![branch_id, &in_id, &out_id], |r| {
            RelatesTo::try_from(r)
        })
        .optional_inner()?
        .ok_or_else(|| anyhow!("relationship not found"))
    }

    /// Resolve the target branch for a canon write anchored to a scene.
    /// If the project's active branch is `scene_branch_id` or a descendant
    /// branch that inherits from `scene_branch_id`, write to the active branch;
    /// otherwise write to `scene_branch_id`.
    pub fn resolve_effective_scene_branch(
        &self,
        project_id: &str,
        scene_branch_id: &str,
    ) -> Result<String> {
        Ok(resolve_effective_scene_branch_in_conn(
            &self.tx,
            project_id,
            scene_branch_id,
        )?)
    }

    /// Reconcile prior `continuity_package` relationship contributions for a
    /// scene (or its narrative position when a revised scene replaced the scene
    /// row) against the incoming `keep_pairs` from a newly committed continuity
    /// package. Any pair that previously received a `continuity_package` delta
    /// at this scene/position in `branch_id`'s effective baseline and is omitted
    /// from `keep_pairs` has its contribution reversed and its ledger row
    /// removed, while preserving deltas whose active source is
    /// `mined_relationship_shift` (or `scene`).
    pub fn reconcile_continuity_package_relationships(
        &self,
        branch_id: &str,
        scene_id: &str,
        keep_pairs: &[(String, String)],
    ) -> Result<()> {
        let now = timestamp_to_micros(chrono::Utc::now());
        let canonical_pair = |a: &str, b: &str| -> (String, String) {
            if a <= b {
                (a.to_string(), b.to_string())
            } else {
                (b.to_string(), a.to_string())
            }
        };
        let keep_set: std::collections::BTreeSet<(String, String)> = keep_pairs
            .iter()
            .map(|(a, b)| canonical_pair(a, b))
            .collect();

        let scene_pos: Option<(String, i32, i32, i32)> = self
            .tx
            .query_row(
                "SELECT project_id, book_number, chapter_number, scene_order \
                 FROM scene WHERE id = ?1",
                [scene_id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i32>(1)?,
                        r.get::<_, i32>(2)?,
                        r.get::<_, i32>(3)?,
                    ))
                },
            )
            .optional_inner()?;
        let project_id_hint = scene_pos.as_ref().map(|(pid, _, _, _)| pid.as_str());
        let chain = resolve_branch_lineage_in_conn(&self.tx, project_id_hint, branch_id)?;

        // Collect all candidate pairs that have a `continuity_package` delta at
        // this scene/position on `branch_id` or any ancestor in `chain`.
        let mut candidate_pairs: Vec<(String, String)> = Vec::new();
        let mut seen_keys: std::collections::BTreeSet<(String, String)> =
            std::collections::BTreeSet::new();
        for candidate_branch_id in &chain {
            let pairs_on_branch: Vec<(String, String)> = if let Some((
                _,
                book_num,
                chap_num,
                sc_order,
            )) = &scene_pos
            {
                let mut stmt = self.tx.prepare_cached(
                        "SELECT DISTINCT in_id, out_id FROM relationship_scene_delta \
                         WHERE branch_id = ?1 AND source = 'continuity_package' \
                           AND (scene_id = ?2 \
                                OR (book_number = ?3 AND chapter_number = ?4 AND scene_order = ?5))",
                    )?;
                let rows = stmt.query_map(
                    rusqlite::params![candidate_branch_id, scene_id, book_num, chap_num, sc_order],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                let mut stmt = self.tx.prepare_cached(
                    "SELECT DISTINCT in_id, out_id FROM relationship_scene_delta \
                         WHERE branch_id = ?1 AND source = 'continuity_package' \
                           AND scene_id = ?2",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![candidate_branch_id, scene_id], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for (a, b) in pairs_on_branch {
                let key = canonical_pair(&a, &b);
                if !keep_set.contains(&key) && seen_keys.insert(key) {
                    candidate_pairs.push((a, b));
                }
            }
        }

        for (a, b) in candidate_pairs {
            let existing_local: Option<(String, String)> = self
                .tx
                .query_row(
                    "SELECT in_id, out_id FROM relates_to \
                     WHERE branch_id = ?1 \
                       AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                     LIMIT 1",
                    rusqlite::params![branch_id, &a, &b],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional_inner()?;

            let (in_id, out_id) = if let Some(local_pair) = existing_local {
                local_pair
            } else {
                struct InheritedRel {
                    anc_branch_id: String,
                    anc_in: String,
                    anc_out: String,
                    rel_type: String,
                    base_trust: i32,
                    base_tension: i32,
                    dynamics: String,
                    reason: Option<String>,
                    last_scene_id: Option<String>,
                }

                // If `branch_id` has not materialized this relationship yet, find
                // the nearest ancestor with a `relates_to` row and materialize it
                // (along with its `relationship_scene_delta` snapshot) so we can
                // reverse the inherited continuity-package contribution cleanly.
                let mut inherited: Option<InheritedRel> = None;
                for ancestor_branch_id in chain.iter().skip(1) {
                    if let Some(row) = self
                        .tx
                        .query_row(
                            "SELECT in_id, out_id, relationship_type, trust, tension, dynamics, \
                                    reason, last_scene_id \
                             FROM relates_to \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                             LIMIT 1",
                            rusqlite::params![ancestor_branch_id, &a, &b],
                            |r| {
                                Ok(InheritedRel {
                                    anc_branch_id: ancestor_branch_id.clone(),
                                    anc_in: r.get(0)?,
                                    anc_out: r.get(1)?,
                                    rel_type: r.get(2)?,
                                    base_trust: r.get(3)?,
                                    base_tension: r.get(4)?,
                                    dynamics: r.get(5)?,
                                    reason: r.get(6)?,
                                    last_scene_id: r.get(7)?,
                                })
                            },
                        )
                        .optional_inner()?
                    {
                        inherited = Some(row);
                        break;
                    }
                }

                let Some(InheritedRel {
                    anc_branch_id,
                    anc_in,
                    anc_out,
                    rel_type,
                    base_trust,
                    base_tension,
                    dynamics,
                    reason,
                    last_scene_id,
                }) = inherited
                else {
                    continue;
                };

                self.tx.execute(
                    "INSERT INTO relates_to (in_id, out_id, branch_id, \
                     relationship_type, trust, tension, dynamics, reason, last_scene_id, \
                     updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        &anc_in,
                        &anc_out,
                        branch_id,
                        &rel_type,
                        base_trust,
                        base_tension,
                        &dynamics,
                        &reason,
                        &last_scene_id,
                        now,
                    ],
                )?;
                self.tx.execute(
                    "DELETE FROM relationship_scene_delta \
                     WHERE branch_id = ?1 \
                       AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2))",
                    rusqlite::params![branch_id, &anc_in, &anc_out],
                )?;
                self.tx.execute(
                    "INSERT OR REPLACE INTO relationship_scene_delta ( \
                        branch_id, in_id, out_id, scene_id, source, \
                        book_number, chapter_number, scene_order, \
                        trust_delta, tension_delta, reason, updated_at \
                     ) \
                     SELECT ?1, in_id, out_id, scene_id, source, \
                            book_number, chapter_number, scene_order, \
                            trust_delta, tension_delta, reason, updated_at \
                     FROM relationship_scene_delta \
                     WHERE branch_id = ?2 \
                       AND ((in_id = ?3 AND out_id = ?4) OR (in_id = ?4 AND out_id = ?3))",
                    rusqlite::params![branch_id, &anc_branch_id, &anc_in, &anc_out],
                )?;
                (anc_in, anc_out)
            };

            let active_delta: Option<(String, i32, i32)> = if let Some((
                _,
                book_num,
                chap_num,
                sc_order,
            )) = &scene_pos
            {
                self.tx
                        .query_row(
                            "SELECT source, trust_delta, tension_delta \
                             FROM relationship_scene_delta \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                               AND (scene_id = ?4 \
                                    OR (book_number = ?5 AND chapter_number = ?6 AND scene_order = ?7)) \
                             ORDER BY updated_at DESC LIMIT 1",
                            rusqlite::params![
                                branch_id, &in_id, &out_id, scene_id, book_num, chap_num, sc_order
                            ],
                            |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, i32>(1)?,
                                    r.get::<_, i32>(2)?,
                                ))
                            },
                        )
                        .optional_inner()?
            } else {
                self.tx
                    .query_row(
                        "SELECT source, trust_delta, tension_delta \
                             FROM relationship_scene_delta \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                               AND scene_id = ?4 \
                             ORDER BY updated_at DESC LIMIT 1",
                        rusqlite::params![branch_id, &in_id, &out_id, scene_id],
                        |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, i32>(1)?,
                                r.get::<_, i32>(2)?,
                            ))
                        },
                    )
                    .optional_inner()?
            };

            if let Some((active_source, prior_trust, prior_tension)) = active_delta
                && active_source == "continuity_package"
            {
                self.tx.execute(
                    "UPDATE relates_to SET \
                        trust = trust - ?1, \
                        tension = tension - ?2, \
                        updated_at = ?3 \
                     WHERE branch_id = ?4 AND in_id = ?5 AND out_id = ?6",
                    rusqlite::params![prior_trust, prior_tension, now, branch_id, &in_id, &out_id],
                )?;
                if let Some((_, book_num, chap_num, sc_order)) = &scene_pos {
                    self.tx.execute(
                        "DELETE FROM relationship_scene_delta \
                         WHERE branch_id = ?1 AND source = 'continuity_package' \
                           AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                           AND (scene_id = ?4 \
                                OR (book_number = ?5 AND chapter_number = ?6 AND scene_order = ?7))",
                        rusqlite::params![
                            branch_id, &in_id, &out_id, scene_id, book_num, chap_num, sc_order
                        ],
                    )?;
                } else {
                    self.tx.execute(
                        "DELETE FROM relationship_scene_delta \
                         WHERE branch_id = ?1 AND source = 'continuity_package' \
                           AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                           AND scene_id = ?4",
                        rusqlite::params![branch_id, &in_id, &out_id, scene_id],
                    )?;
                }
            }
        }

        Ok(())
    }

    pub fn resolve_validator_findings_for_validator(
        &self,
        project_id: &str,
        branch_id: &str,
        validator_id: &str,
    ) -> Result<usize> {
        let now = timestamp_to_micros(chrono::Utc::now());
        Ok(self.tx.execute(
            "UPDATE validator_finding SET resolved_at = ?1 \
             WHERE project_id = ?2 AND branch_id = ?3 \
               AND validator_id = ?4 AND resolved_at IS NULL",
            rusqlite::params![now, project_id, branch_id, validator_id],
        )?)
    }

    pub fn resolve_validator_findings_for_scenes(
        &self,
        branch_id: &str,
        scene_ids: &[String],
    ) -> Result<usize> {
        if scene_ids.is_empty() {
            return Ok(0);
        }
        let now = timestamp_to_micros(chrono::Utc::now());
        let placeholders = (0..scene_ids.len())
            .map(|i| format!("?{}", i + 3))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE validator_finding SET resolved_at = ?1 \
             WHERE branch_id = ?2 AND scene_id IN ({placeholders}) \
               AND resolved_at IS NULL"
        );
        let mut params: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(scene_ids.len() + 2);
        params.push(&now);
        params.push(&branch_id);
        for id in scene_ids {
            params.push(id);
        }
        Ok(self.tx.execute(&sql, &params[..])?)
    }
}

/// Shared body for `list_X_by_project_and_branch` methods. Resolves along the
/// `bible_branch.parent_branch_id` chain so child branches inherit ancestor
/// canon while overriding ancestor rows that share the same natural key.
#[allow(clippy::too_many_arguments)]
async fn list_branch_scoped<T, K, F, KFn>(
    pool: &SqlitePool,
    columns: &'static str,
    table: &'static str,
    order_by: &'static str,
    project_id: &str,
    branch_id: &str,
    map: F,
    key_fn: KFn,
) -> Result<Vec<T>>
where
    T: Send + 'static,
    K: Eq + std::hash::Hash + Send + 'static,
    F: Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T> + Send + Sync + 'static,
    KFn: Fn(&T) -> K + Send + Sync + 'static,
{
    let project_id = project_id.to_string();
    let branch_id = branch_id.to_string();
    pool.read(move |conn| {
        let chain = resolve_branch_lineage_in_conn(conn, Some(&project_id), &branch_id)?;
        let sql = format!(
            "SELECT {columns} FROM {table} \
             WHERE project_id = ?1 AND branch_id = ?2 ORDER BY {order_by}"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let mut overridden = std::collections::HashSet::new();
        let mut out = Vec::new();
        for candidate_branch_id in &chain {
            let rows = stmt
                .query_map(rusqlite::params![&project_id, candidate_branch_id], |r| {
                    map(r)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut branch_keys = Vec::new();
            for row in rows {
                let key = key_fn(&row);
                if !overridden.contains(&key) {
                    branch_keys.push(key);
                    out.push(row);
                }
            }
            overridden.extend(branch_keys);
        }
        Ok(out)
    })
    .await
}

/// Local extension to map `Err(QueryReturnedNoRows)` into `Ok(None)` cleanly
/// from inside a closure passed to `pool.read` / `pool.write`. Plain
/// `.optional()` from rusqlite works the same but conflicts with our
/// `Result<T, rusqlite::Error>` boundary inside the closure (rusqlite's trait
/// is implemented for `Result`, not `Option`).
trait OptionalInner<T> {
    fn optional_inner(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalInner<T> for rusqlite::Result<T> {
    fn optional_inner(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// =============================================================================
// SnapshotBatchContext + pure projection helpers
//
// Ported verbatim from the SurrealDB repository. Two structural differences
// from the original:
//   * IDs are plain `String` (e.g. "character:01ARZ..."), so id-to-table
//     dispatch reads the prefix before the leading `':'` instead of pulling
//     `RecordId.table`.
//   * `CanonicalFact.value_number` is `Option<f64>` here (the SurrealDB row
//     held `Option<serde_json::Value>`), so the value-text helper formats the
//     number directly.
// All filter / sort semantics are otherwise preserved.
// =============================================================================

struct SnapshotBatchContext {
    project: Project,
    books: Vec<Book>,
    chapters: Vec<Chapter>,
    world_rules: Vec<WorldRule>,
    characters: Vec<Character>,
    locations: Vec<Location>,
    factions: Vec<Faction>,
    religions: Vec<Religion>,
    economies: Vec<Economy>,
    plot_lines: Vec<PlotLine>,
    conflicts: Vec<Conflict>,
    themes: Vec<Theme>,
    motifs: Vec<Motif>,
    overlays: Vec<SystemOverlay>,
    promises: Vec<NarrativePromise>,
    arcs: Vec<CharacterArc>,
    terms: Vec<Term>,
    relationships: Vec<RelatesTo>,
    events: Vec<TimelineEvent>,
    scenes: Vec<Scene>,
    canonical_facts: Vec<CanonicalFact>,
    knowledge_facts: Vec<KnowledgeFact>,
    character_states: Vec<CharacterState>,
    voice_profiles: Vec<CharacterVoiceProfile>,
    world_states: Vec<WorldState>,
}

fn assemble_subject_snapshot_from_batch(
    batch: &SnapshotBatchContext,
    subject: &Subject,
    placement: &StoryPlacement,
) -> Result<SubjectSnapshot> {
    let canonical_facts = batch_canonical_fact_summaries(batch, subject, placement)?;
    let (display_name, kind_specific, provenance) = batch_subject_kind(batch, subject, placement)?;
    let knowledge = batch_knowledge_summaries(batch, subject, placement)?;
    let relationships = batch_relationship_summaries(batch, subject)?;
    let open_promises = batch_open_promise_summaries(batch, subject, placement)?;
    let active_arcs = batch_active_arc_summaries(batch, subject, placement)?;
    let recent_appearances = batch_recent_appearances(batch, &display_name, placement);
    let voice_profile = batch_voice_profile_summary(batch, subject)?;
    let current_state = batch_character_state_summary(batch, subject, placement)?;

    SubjectSnapshot::new(
        subject.clone(),
        display_name,
        kind_specific,
        canonical_facts,
        knowledge,
        relationships,
        open_promises,
        active_arcs,
        recent_appearances,
        voice_profile,
        current_state,
        placement.clone(),
        provenance,
    )
    .map_err(Into::into)
}

fn batch_subject_kind(
    batch: &SnapshotBatchContext,
    subject: &Subject,
    placement: &StoryPlacement,
) -> Result<(String, SubjectKindSpecific, Provenance)> {
    if subject.table() == SubjectTable::Project && subject.is_project_wide() {
        return Ok((
            batch.project.name.clone(),
            SubjectKindSpecific::Generic(serde_json::json!({
                "project_type": batch.project.project_type,
                "genre": batch.project.genre,
                "reader_contract": batch.project.reader_contract.clone().into_core(),
                "notes": batch.project.notes,
            })),
            Provenance::asserted_by_author(batch.project.updated_at),
        ));
    }

    let raw_id = subject.id().context("subject id missing")?.to_string();
    match subject.table() {
        SubjectTable::WorldRule => {
            let world_rule = find_by_id(&batch.world_rules, &raw_id, |item| &item.id)?;
            Ok((
                world_rule.rule_name.clone(),
                SubjectKindSpecific::WorldRule(WorldRuleDetails {
                    rule_type: world_rule.rule_type.clone(),
                    description: world_rule.description.clone(),
                    scan_pattern: world_rule.scan_pattern.clone(),
                    relevance_tags: world_rule.relevance_tags_or_empty().to_vec(),
                    established_in: world_rule
                        .established_in
                        .clone()
                        .map(StoredEstablishedIn::into_core),
                }),
                Provenance::asserted_by_author(world_rule.created_at),
            ))
        }
        SubjectTable::Character => {
            let character = find_by_id(&batch.characters, &raw_id, |item| &item.id)?;
            Ok((
                character.name.clone(),
                SubjectKindSpecific::Character(CharacterDetails {
                    role: character.role.clone(),
                    summary: character.summary.clone(),
                    realm: character.realm.clone(),
                    aliases: character.aliases.clone(),
                }),
                Provenance::asserted_by_author(character.updated_at),
            ))
        }
        SubjectTable::Location => {
            let location = find_by_id(&batch.locations, &raw_id, |item| &item.id)?;
            let world_state = batch
                .world_states
                .iter()
                .find(|state| state.location_id == raw_id);
            Ok((
                location.name.clone(),
                SubjectKindSpecific::Location(LocationDetails {
                    kind: location.kind.clone(),
                    summary: location.summary.clone(),
                    realm: location.realm.clone(),
                    controlling_faction: world_state
                        .and_then(|state| state.controlling_faction.clone()),
                    status: world_state.and_then(|state| state.status.clone()),
                }),
                Provenance::asserted_by_author(location.updated_at),
            ))
        }
        SubjectTable::Faction => {
            let faction = find_by_id(&batch.factions, &raw_id, |item| &item.id)?;
            Ok((
                faction.name.clone(),
                SubjectKindSpecific::Faction(FactionDetails {
                    category: faction.faction_type.clone(),
                    summary: faction.summary.clone(),
                    goals: faction.tags.clone(),
                    sphere_of_influence: faction.realm.clone(),
                }),
                Provenance::asserted_by_author(faction.updated_at),
            ))
        }
        SubjectTable::Religion => {
            let religion = find_by_id(&batch.religions, &raw_id, |item| &item.id)?;
            Ok((
                religion.name.clone(),
                SubjectKindSpecific::Religion(ReligionDetails {
                    domain: religion.deity_or_principle.clone(),
                    summary: religion.summary.clone(),
                    core_beliefs: religion.tags.clone(),
                    practices: Vec::new(),
                }),
                Provenance::asserted_by_author(religion.updated_at),
            ))
        }
        SubjectTable::Economy => {
            let economy = find_by_id(&batch.economies, &raw_id, |item| &item.id)?;
            Ok((
                economy.name.clone(),
                SubjectKindSpecific::Economy(EconomyDetails {
                    system: economy
                        .realm
                        .clone()
                        .unwrap_or_else(|| "regional".to_string()),
                    summary: economy.summary.clone(),
                    currency: economy.currency.clone(),
                    trade_goods: economy.trade_goods.clone(),
                }),
                Provenance::asserted_by_author(economy.updated_at),
            ))
        }
        SubjectTable::PlotLine => {
            let plot_line = find_by_id(&batch.plot_lines, &raw_id, |item| &item.id)?;
            Ok((
                plot_line.name.clone(),
                SubjectKindSpecific::PlotLine(PlotLineDetails {
                    plot_type: plot_line.plot_type.clone(),
                    summary: plot_line.summary.clone(),
                    status: Some(plot_line.status.clone()),
                }),
                Provenance::asserted_by_author(plot_line.updated_at),
            ))
        }
        SubjectTable::Conflict => {
            let conflict = find_by_id(&batch.conflicts, &raw_id, |item| &item.id)?;
            Ok((
                conflict.name.clone(),
                SubjectKindSpecific::Conflict(ConflictDetails {
                    conflict_type: conflict.conflict_type.clone(),
                    stakes: conflict.stakes.clone(),
                    status: conflict.resolution_summary.clone(),
                    escalation_stage: conflict.escalation_stages.last().cloned(),
                }),
                Provenance::asserted_by_author(conflict.updated_at),
            ))
        }
        SubjectTable::Theme => {
            let theme = find_by_id(&batch.themes, &raw_id, |item| &item.id)?;
            Ok((
                theme.theme_statement.clone(),
                SubjectKindSpecific::Theme(ThemeDetails {
                    statement: theme.theme_statement.clone(),
                    thesis_antithesis: theme.thesis_antithesis.clone(),
                    status: theme
                        .resolution_point
                        .clone()
                        .map(StoredStoryPlacement::into_core)
                        .filter(|point| placement_at_or_after(placement, point))
                        .map(|_| "resolved".to_string()),
                }),
                Provenance::asserted_by_author(theme.updated_at),
            ))
        }
        SubjectTable::Motif => {
            let motif = find_by_id(&batch.motifs, &raw_id, |item| &item.id)?;
            let thematic_links = batch
                .themes
                .iter()
                .filter(|theme| motif.connected_theme_ids.iter().any(|id| id == &theme.id))
                .map(|theme| theme.theme_statement.clone())
                .collect();
            Ok((
                motif.name.clone(),
                SubjectKindSpecific::Motif(MotifDetails {
                    name: motif.name.clone(),
                    description: motif.description.clone(),
                    thematic_links,
                }),
                Provenance::asserted_by_author(motif.updated_at),
            ))
        }
        SubjectTable::SystemOverlay => {
            let overlay = find_by_id(&batch.overlays, &raw_id, |item| &item.id)?;
            Ok((
                overlay.system_name.clone(),
                SubjectKindSpecific::SystemOverlay(SystemOverlayDetails {
                    system_name: overlay.system_name.clone(),
                    system_type: overlay.system_type.clone(),
                    visibility: overlay.visibility.clone(),
                    rules: overlay.rules.clone(),
                    stats: overlay.stats.clone(),
                }),
                Provenance::asserted_by_author(overlay.updated_at),
            ))
        }
        SubjectTable::NarrativePromise => {
            let promise = find_by_id(&batch.promises, &raw_id, |item| &item.id)?;
            Ok((
                promise.description.clone(),
                SubjectKindSpecific::NarrativePromise(NarrativePromiseDetails {
                    promise_type: promise.promise_type.clone(),
                    description: promise.description.clone(),
                    status: promise.status.clone(),
                    planned_payoff: promise
                        .planned_payoff
                        .clone()
                        .map(StoredStoryPlacement::into_core),
                }),
                placement_provenance(&promise.planted_at.clone().into_core()),
            ))
        }
        SubjectTable::CharacterArc => {
            let arc = find_by_id(&batch.arcs, &raw_id, |item| &item.id)?;
            Ok((
                arc.arc_type.clone(),
                SubjectKindSpecific::CharacterArc(CharacterArcDetails {
                    arc_type: arc.arc_type.clone(),
                    starting_state: arc.starting_state.clone(),
                    current_state: if arc.progress >= 1.0 {
                        arc.ending_state.clone()
                    } else {
                        arc.starting_state.clone()
                    },
                    target_state: arc.ending_state.clone(),
                    thematic_purpose: arc.thematic_purpose.clone(),
                    milestones: arc
                        .milestones
                        .iter()
                        .cloned()
                        .map(StoredCharacterArcMilestone::into_core)
                        .collect(),
                }),
                Provenance::asserted_by_author(arc.updated_at),
            ))
        }
        SubjectTable::Term => {
            let term = find_by_id(&batch.terms, &raw_id, |item| &item.id)?;
            Ok((
                term.term_text.clone(),
                SubjectKindSpecific::Term(TermDetails {
                    term_text: term.term_text.clone(),
                    pronunciation: term.pronunciation.clone(),
                    definition: term.definition.clone(),
                    usage_context: term.usage_context.clone(),
                    origin: term.origin.clone(),
                }),
                Provenance::asserted_by_author(term.updated_at),
            ))
        }
        SubjectTable::Relationship => {
            // The SQLite `relates_to` table is composite-keyed (in_id, out_id,
            // branch_id) and has no surrogate id column. Subject IDs of the
            // form `"<in_id>|<out_id>"` are how the SQLite layer addresses a
            // single relationship row.
            let (in_id, out_id) = split_relationship_subject_id(&raw_id)?;
            let relationship = batch
                .relationships
                .iter()
                .find(|r| r.in_id == in_id && r.out_id == out_id)
                .with_context(|| format!("relationship not found in batch: {raw_id}"))?;
            let source = batch_subject_link_summary(batch, &relationship.in_id)?;
            let target = batch_subject_link_summary(batch, &relationship.out_id)?;
            Ok((
                format!("{} -> {}", source.display_name, target.display_name),
                SubjectKindSpecific::Relationship(RelationshipDetails {
                    relationship_type: relationship.relationship_type.clone(),
                    source,
                    target,
                    trust: Some(relationship.trust),
                    tension: Some(relationship.tension),
                    summary: relationship
                        .reason
                        .clone()
                        .unwrap_or_else(|| relationship.dynamics.join(", ")),
                }),
                relationship
                    .last_scene_id
                    .as_deref()
                    .map(scene_provenance)
                    .unwrap_or_else(|| Provenance::asserted_by_author(relationship.updated_at)),
            ))
        }
        SubjectTable::TimelineEvent => {
            let event = find_by_id(&batch.events, &raw_id, |item| &item.id)?;
            let related_subjects = event
                .related_entity_ids
                .iter()
                .filter_map(|id| batch_subject_link_summary(batch, id).ok())
                .collect();
            Ok((
                event.title.clone(),
                SubjectKindSpecific::TimelineEvent(TimelineEventDetails {
                    title: event.title.clone(),
                    event_type: event.event_type.clone(),
                    placement: event.placement.clone().into_core(),
                    summary: event.summary.clone(),
                    related_subjects,
                }),
                placement_provenance(&event.placement.clone().into_core()),
            ))
        }
        SubjectTable::Scene => {
            let scene = find_by_id(&batch.scenes, &raw_id, |item| &item.id)?;
            Ok((
                format!(
                    "Scene {}.{}.{}",
                    scene.book_number, scene.chapter_number, scene.scene_order
                ),
                SubjectKindSpecific::Generic(serde_json::json!({
                    "summary": scene.summary,
                    "content_rating": scene.content_rating,
                    "tone": scene.tone,
                })),
                scene_provenance(&scene.id),
            ))
        }
        SubjectTable::Chapter => {
            let chapter = find_by_id(&batch.chapters, &raw_id, |item| &item.id)?;
            Ok((
                chapter
                    .title
                    .clone()
                    .unwrap_or_else(|| format!("Chapter {}", chapter.chapter_number)),
                SubjectKindSpecific::Generic(serde_json::json!({
                    "book_number": chapter.book_number,
                    "chapter_number": chapter.chapter_number,
                    "title": chapter.title,
                })),
                Provenance::chapter(SnapshotRecordId::new(chapter.id.clone())),
            ))
        }
        SubjectTable::Book => {
            let book = find_by_id(&batch.books, &raw_id, |item| &item.id)?;
            Ok((
                book.title
                    .clone()
                    .unwrap_or_else(|| format!("Book {}", book.book_number)),
                SubjectKindSpecific::Generic(serde_json::json!({
                    "book_number": book.book_number,
                    "title": book.title,
                })),
                Provenance::book(SnapshotRecordId::new(book.id.clone())),
            ))
        }
        SubjectTable::Project => anyhow::bail!("project snapshots are handled separately"),
    }
}

fn batch_canonical_fact_summaries(
    batch: &SnapshotBatchContext,
    subject: &Subject,
    placement: &StoryPlacement,
) -> Result<Vec<CanonicalFactSummary>> {
    let display_name = if subject.table() == SubjectTable::Project && subject.is_project_wide() {
        batch.project.name.clone()
    } else {
        let subject_id = subject.id().context("subject id missing")?.to_string();
        batch_subject_display_name(batch, &subject_id)?
    };
    let needle = normalize_name(&display_name);
    Ok(batch
        .canonical_facts
        .iter()
        .filter(|fact| fact_matches_subject(fact, &needle))
        .filter(|fact| fact_at_or_before(fact, placement))
        .map(|fact| CanonicalFactSummary {
            fact: format!(
                "{}: {}",
                fact.predicate,
                canonical_fact_value_text(fact).unwrap_or_else(|| "<unset>".to_string())
            ),
            source_label: None,
            // Planned-and-pending facts (V0038) have no scene yet; cite their
            // book/chapter placement instead so the summary still carries a
            // source.
            provenance: match fact.scene_id.as_deref() {
                Some(scene_id) => scene_provenance(scene_id),
                None => placement_provenance(&StoryPlacement {
                    book_number: fact.book_number,
                    chapter_number: fact.chapter_number,
                    scene_order: None,
                    note: None,
                }),
            },
        })
        .collect())
}

fn batch_knowledge_summaries(
    batch: &SnapshotBatchContext,
    subject: &Subject,
    placement: &StoryPlacement,
) -> Result<Vec<KnowledgeFactSummary>> {
    if subject.table() != SubjectTable::Character {
        return Ok(Vec::new());
    }
    let character_id = subject
        .id()
        .context("character subject id missing")?
        .to_string();
    Ok(batch
        .knowledge_facts
        .iter()
        .filter(|fact| fact.character_id == character_id)
        .filter(|fact| {
            fact.learned_at
                .as_ref()
                .map(|at| placement_at_or_after(placement, &at.clone().into_core()))
                .unwrap_or(true)
        })
        .map(|fact| KnowledgeFactSummary {
            fact: fact.fact.clone(),
            scope: Some(if fact.reader_visible {
                "reader_visible".to_string()
            } else {
                "private".to_string()
            }),
            source: Some(fact.source_summary.clone()),
            learned_at: fact.learned_at.clone().map(StoredStoryPlacement::into_core),
            confidence: fact.confidence,
            provenance: Provenance::asserted_by_author(fact.updated_at),
        })
        .collect())
}

fn batch_relationship_summaries(
    batch: &SnapshotBatchContext,
    subject: &Subject,
) -> Result<Vec<RelationshipSummary>> {
    if subject.table() != SubjectTable::Character {
        return Ok(Vec::new());
    }
    let character_id = subject
        .id()
        .context("character subject id missing")?
        .to_string();
    batch
        .relationships
        .iter()
        .filter(|relationship| {
            relationship.in_id == character_id || relationship.out_id == character_id
        })
        .map(|relationship| {
            let counterpart_id = if relationship.in_id == character_id {
                &relationship.out_id
            } else {
                &relationship.in_id
            };
            Ok(RelationshipSummary {
                relationship_type: relationship.relationship_type.clone(),
                counterpart: Some(batch_subject_display_name(batch, counterpart_id)?),
                summary: relationship
                    .reason
                    .clone()
                    .unwrap_or_else(|| relationship.dynamics.join(", ")),
                trust: Some(relationship.trust),
                tension: Some(relationship.tension),
                provenance: relationship
                    .last_scene_id
                    .as_deref()
                    .map(scene_provenance)
                    .unwrap_or_else(|| Provenance::asserted_by_author(relationship.updated_at)),
            })
        })
        .collect()
}

fn batch_open_promise_summaries(
    batch: &SnapshotBatchContext,
    subject: &Subject,
    placement: &StoryPlacement,
) -> Result<Vec<NarrativePromiseSummary>> {
    let display_name = if subject.table() == SubjectTable::Project && subject.is_project_wide() {
        batch.project.name.clone()
    } else {
        let subject_id = subject.id().context("subject id missing")?.to_string();
        batch_subject_display_name(batch, &subject_id)?
    };
    let needle = normalize_name(&display_name);
    Ok(batch
        .promises
        .iter()
        .filter(|promise| promise.status != "paid_off")
        .filter(|promise| placement_at_or_after(placement, &promise.planted_at.clone().into_core()))
        .filter(|promise| normalize_name(&promise.description).contains(&needle))
        .map(|promise| NarrativePromiseSummary {
            promise_type: promise.promise_type.clone(),
            description: promise.description.clone(),
            status: promise.status.clone(),
            planned_payoff: promise
                .planned_payoff
                .clone()
                .map(StoredStoryPlacement::into_core),
            provenance: placement_provenance(&promise.planted_at.clone().into_core()),
        })
        .collect())
}

fn batch_active_arc_summaries(
    batch: &SnapshotBatchContext,
    subject: &Subject,
    placement: &StoryPlacement,
) -> Result<Vec<CharacterArcSummary>> {
    if subject.table() != SubjectTable::Character {
        return Ok(Vec::new());
    }
    let character_id = subject
        .id()
        .context("character subject id missing")?
        .to_string();
    Ok(batch
        .arcs
        .iter()
        .filter(|arc| arc.character_id == character_id && arc.status == "active")
        .filter(|arc| arc_established_at_or_before(arc, placement))
        .map(|arc| CharacterArcSummary {
            arc_type: arc.arc_type.clone(),
            summary: arc.thematic_purpose.clone(),
            current_phase: arc
                .milestones
                .iter()
                .rfind(|milestone| {
                    milestone
                        .placement
                        .as_ref()
                        .map(|at| placement_at_or_after(placement, &at.clone().into_core()))
                        .unwrap_or(true)
                })
                .map(|milestone| milestone.label.clone()),
            provenance: Provenance::asserted_by_author(arc.updated_at),
            milestones: arc
                .milestones
                .iter()
                .cloned()
                .map(StoredCharacterArcMilestone::into_core)
                .collect(),
        })
        .collect())
}

fn batch_recent_appearances(
    batch: &SnapshotBatchContext,
    display_name: &str,
    placement: &StoryPlacement,
) -> Vec<SceneAppearanceSummary> {
    let needle = normalize_name(display_name);
    let mut scenes = batch
        .scenes
        .iter()
        .filter(|scene| scene_at_or_before(scene, placement))
        .cloned()
        .collect::<Vec<_>>();
    scenes.reverse();
    scenes
        .into_iter()
        .filter(|scene| {
            normalize_name(&format!("{} {}", scene.summary, scene.full_text)).contains(&needle)
        })
        .take(5)
        .map(|scene| SceneAppearanceSummary {
            scene_id: SnapshotRecordId::new(scene.id.clone()),
            placement: Some(StoryPlacement {
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: Some(scene.scene_order),
                note: None,
            }),
            summary: Some(scene.summary),
            provenance: scene_provenance(&scene.id),
        })
        .collect()
}

fn batch_voice_profile_summary(
    batch: &SnapshotBatchContext,
    subject: &Subject,
) -> Result<Option<VoiceProfileSummary>> {
    if subject.table() != SubjectTable::Character {
        return Ok(None);
    }
    let character_id = subject
        .id()
        .context("character subject id missing")?
        .to_string();
    let profile = batch
        .voice_profiles
        .iter()
        .find(|profile| profile.character_id == character_id);
    Ok(profile.map(|profile| {
        let observed_at = profile.updated_at.unwrap_or(profile.created_at);
        VoiceProfileSummary {
            tone: profile.tone.clone(),
            vocabulary: profile.vocabulary.clone(),
            sentence_structure: profile.sentence_structure.clone(),
            tics: profile.tics.clone(),
            forbidden_words: profile.forbidden_words.clone(),
            example_lines: profile.example_lines.clone(),
            established_in_scene_id: profile
                .established_in_scene_id
                .as_deref()
                .map(|id| SnapshotRecordId::new(id.to_string())),
            updated_at: Some(observed_at.to_rfc3339()),
            provenance: Provenance::asserted_by_author(observed_at),
        }
    }))
}

fn batch_character_state_summary(
    batch: &SnapshotBatchContext,
    subject: &Subject,
    placement: &StoryPlacement,
) -> Result<Option<CharacterStateSummary>> {
    if subject.table() != SubjectTable::Character {
        return Ok(None);
    }
    let character_id = subject
        .id()
        .context("character subject id missing")?
        .to_string();
    let state = batch
        .character_states
        .iter()
        .filter(|state| state.character_id == character_id)
        .filter(|state| state_at_or_before(state, placement))
        .max_by_key(|state| (state.book_number, state.chapter_number, state.scene_order));
    Ok(state.map(|state| CharacterStateSummary {
        emotional_state: state.emotional_state.clone(),
        goals: state.goals.clone(),
        status: state.status.clone(),
        notes: state.notes.clone(),
        source_summary: state.source_summary.clone(),
        provenance: state
            .scene_id
            .as_deref()
            .map(scene_provenance)
            .unwrap_or_else(|| Provenance::asserted_by_author(state.created_at)),
    }))
}

fn batch_subject_link_summary(
    batch: &SnapshotBatchContext,
    id: &str,
) -> Result<SubjectLinkSummary> {
    let table = subject_table_from_id(id)?;
    Ok(SubjectLinkSummary {
        subject: Subject::new(table, id.to_string())?,
        display_name: batch_subject_display_name(batch, id)?,
    })
}

fn batch_subject_display_name(batch: &SnapshotBatchContext, id: &str) -> Result<String> {
    let table_prefix = id_table_prefix(id)?;
    match table_prefix {
        "world_rule" => Ok(find_by_id(&batch.world_rules, id, |item| &item.id)?
            .rule_name
            .clone()),
        "character" => Ok(find_by_id(&batch.characters, id, |item| &item.id)?
            .name
            .clone()),
        "location" => Ok(find_by_id(&batch.locations, id, |item| &item.id)?
            .name
            .clone()),
        "faction" => Ok(find_by_id(&batch.factions, id, |item| &item.id)?
            .name
            .clone()),
        "religion" => Ok(find_by_id(&batch.religions, id, |item| &item.id)?
            .name
            .clone()),
        "economy" => Ok(find_by_id(&batch.economies, id, |item| &item.id)?
            .name
            .clone()),
        "plot_line" => Ok(find_by_id(&batch.plot_lines, id, |item| &item.id)?
            .name
            .clone()),
        "conflict" => Ok(find_by_id(&batch.conflicts, id, |item| &item.id)?
            .name
            .clone()),
        "theme" => Ok(find_by_id(&batch.themes, id, |item| &item.id)?
            .theme_statement
            .clone()),
        "motif" => Ok(find_by_id(&batch.motifs, id, |item| &item.id)?.name.clone()),
        "system_overlay" => Ok(find_by_id(&batch.overlays, id, |item| &item.id)?
            .system_name
            .clone()),
        "narrative_promise" => Ok(find_by_id(&batch.promises, id, |item| &item.id)?
            .description
            .clone()),
        "character_arc" => Ok(find_by_id(&batch.arcs, id, |item| &item.id)?
            .arc_type
            .clone()),
        "term" => Ok(find_by_id(&batch.terms, id, |item| &item.id)?
            .term_text
            .clone()),
        "timeline_event" => Ok(find_by_id(&batch.events, id, |item| &item.id)?
            .title
            .clone()),
        "scene" => {
            let scene = find_by_id(&batch.scenes, id, |item| &item.id)?;
            Ok(format!(
                "Scene {}.{}.{}",
                scene.book_number, scene.chapter_number, scene.scene_order
            ))
        }
        "chapter" => {
            let chapter = find_by_id(&batch.chapters, id, |item| &item.id)?;
            Ok(chapter
                .title
                .clone()
                .unwrap_or_else(|| format!("Chapter {}", chapter.chapter_number)))
        }
        "book" => {
            let book = find_by_id(&batch.books, id, |item| &item.id)?;
            Ok(book
                .title
                .clone()
                .unwrap_or_else(|| format!("Book {}", book.book_number)))
        }
        "relationship" | "relates_to" => {
            let (in_id, out_id) = split_relationship_subject_id(id)?;
            let relationship = batch
                .relationships
                .iter()
                .find(|r| r.in_id == in_id && r.out_id == out_id)
                .with_context(|| format!("relationship not found in batch: {id}"))?;
            Ok(format!(
                "{} -> {}",
                batch_subject_display_name(batch, &relationship.in_id)?,
                batch_subject_display_name(batch, &relationship.out_id)?
            ))
        }
        other => anyhow::bail!("unsupported subject table for batch display name: {other}"),
    }
}

fn find_by_id<'a, T>(values: &'a [T], id: &str, id_of: impl Fn(&'a T) -> &String) -> Result<&'a T> {
    values
        .iter()
        .find(|value| id_of(value) == id)
        .with_context(|| format!("record not found in batch: {id}"))
}

/// Extract the `table` prefix from a SQLite-flavoured id of the form
/// `"<table>:<key>"`. Errors if the id is missing the `:` separator.
fn id_table_prefix(id: &str) -> Result<&str> {
    id.split_once(':')
        .map(|(table, _)| table)
        .with_context(|| format!("id missing table prefix: {id}"))
}

fn subject_table_from_id(id: &str) -> Result<SubjectTable> {
    match id_table_prefix(id)? {
        "world_rule" => Ok(SubjectTable::WorldRule),
        "character" => Ok(SubjectTable::Character),
        "location" => Ok(SubjectTable::Location),
        "faction" => Ok(SubjectTable::Faction),
        "religion" => Ok(SubjectTable::Religion),
        "economy" => Ok(SubjectTable::Economy),
        "plot_line" => Ok(SubjectTable::PlotLine),
        "conflict" => Ok(SubjectTable::Conflict),
        "theme" => Ok(SubjectTable::Theme),
        "motif" => Ok(SubjectTable::Motif),
        "system_overlay" => Ok(SubjectTable::SystemOverlay),
        "narrative_promise" => Ok(SubjectTable::NarrativePromise),
        "character_arc" => Ok(SubjectTable::CharacterArc),
        "term" => Ok(SubjectTable::Term),
        "relationship" | "relates_to" => Ok(SubjectTable::Relationship),
        "timeline_event" => Ok(SubjectTable::TimelineEvent),
        "scene" => Ok(SubjectTable::Scene),
        "chapter" => Ok(SubjectTable::Chapter),
        "book" => Ok(SubjectTable::Book),
        other => anyhow::bail!("unsupported subject table: {other}"),
    }
}

/// SQLite Relationship subjects encode their composite key as
/// `"<in_id>|<out_id>"` because the underlying `relates_to` row has no
/// surrogate id.
fn split_relationship_subject_id(id: &str) -> Result<(String, String)> {
    id.split_once('|')
        .map(|(in_id, out_id)| (in_id.to_string(), out_id.to_string()))
        .with_context(|| {
            format!("relationship subject id must be of the form '<in_id>|<out_id>': {id}")
        })
}

fn scene_provenance(id: &str) -> Provenance {
    Provenance::scene(SnapshotRecordId::new(id.to_string()), None)
}

fn placement_provenance(placement: &StoryPlacement) -> Provenance {
    if let Some(scene_order) = placement.scene_order {
        Provenance::scene(
            SnapshotRecordId::new(format!(
                "scene:{}:{}:{}",
                placement.book_number, placement.chapter_number, scene_order
            )),
            None,
        )
    } else {
        Provenance::chapter(SnapshotRecordId::new(format!(
            "chapter:{}:{}",
            placement.book_number, placement.chapter_number
        )))
    }
}

fn fact_matches_subject(fact: &CanonicalFact, subject_name: &str) -> bool {
    let mut fields = vec![normalize_name(&fact.predicate)];
    if let Some(value_text) = canonical_fact_value_text(fact) {
        fields.push(normalize_name(&value_text));
    }
    fields.extend(fact.aliases.iter().map(|value| normalize_name(value)));
    fields.into_iter().any(|field| field.contains(subject_name))
}

fn fact_at_or_before(fact: &CanonicalFact, placement: &StoryPlacement) -> bool {
    let current = placement_key(placement);
    let established_at = (fact.book_number, fact.chapter_number, i32::MIN);
    if established_at > current {
        return false;
    }
    if let Some(valid_from) = fact.valid_from.as_ref().map(|sp| sp.clone().into_core())
        && placement_key(&valid_from) > current
    {
        return false;
    }
    if let Some(valid_until) = fact.valid_until.as_ref().map(|sp| sp.clone().into_core())
        && placement_key(&valid_until) < current
    {
        return false;
    }
    true
}

fn canonical_fact_value_text(fact: &CanonicalFact) -> Option<String> {
    if let Some(value) = fact.value_text.clone().filter(|value| !value.is_empty()) {
        return Some(value);
    }
    if let Some(number) = fact.value_number {
        // Format with the smallest representation that round-trips: integers
        // render as `"42"`, fractions keep their natural decimal form.
        let rendered = if number.fract() == 0.0 {
            format!("{}", number as i64)
        } else {
            format!("{number}")
        };
        if !rendered.is_empty() {
            return Some(rendered);
        }
    }
    fact.value_json
        .as_ref()
        .map(serde_json::Value::to_string)
        .filter(|value| !value.is_empty())
}

fn placement_at_or_after(current: &StoryPlacement, point: &StoryPlacement) -> bool {
    placement_key(current) >= placement_key(point)
}

fn scene_at_or_before(scene: &Scene, placement: &StoryPlacement) -> bool {
    (scene.book_number, scene.chapter_number, scene.scene_order)
        <= (
            placement.book_number,
            placement.chapter_number,
            placement.scene_order.unwrap_or(i32::MAX),
        )
}

fn state_at_or_before(state: &CharacterState, placement: &StoryPlacement) -> bool {
    (state.book_number, state.chapter_number, state.scene_order)
        <= (
            placement.book_number,
            placement.chapter_number,
            placement.scene_order.unwrap_or(i32::MAX),
        )
}

fn arc_established_at_or_before(arc: &CharacterArc, placement: &StoryPlacement) -> bool {
    match arc
        .milestones
        .iter()
        .filter_map(|milestone| {
            milestone
                .placement
                .clone()
                .map(StoredStoryPlacement::into_core)
        })
        .min_by_key(placement_key)
    {
        Some(first_milestone) => placement_at_or_after(placement, &first_milestone),
        None => true,
    }
}

fn placement_key(placement: &StoryPlacement) -> (i32, i32, i32) {
    (
        placement.book_number,
        placement.chapter_number,
        placement.scene_order.unwrap_or(i32::MAX),
    )
}

#[cfg(test)]
mod tests;
