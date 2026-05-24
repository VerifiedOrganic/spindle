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

use super::SqlitePool;
use super::records::{
    BIBLE_BRANCH_COLUMNS, BOOK_COLUMNS, BibleBranch, Book, CHAPTER_COLUMNS, CHARACTER_ARC_COLUMNS,
    CHARACTER_COLUMNS, CHARACTER_EMOTIONAL_PROFILE_COLUMNS, CHARACTER_VOICE_PROFILE_COLUMNS,
    CONFLICT_COLUMNS, Chapter, Character, CharacterArc, CharacterEmotionalProfile,
    CharacterVoiceProfile, Conflict, ECONOMY_COLUMNS, Economy, FACTION_COLUMNS, Faction,
    LOCATION_COLUMNS, Location, MOTIF_COLUMNS, Motif, NARRATIVE_PROMISE_COLUMNS, NarrativePromise,
    PLOT_LINE_COLUMNS, PROJECT_COLUMNS, PlotLine, Project, RELIGION_COLUMNS, Religion,
    SCENE_COLUMNS, SYSTEM_OVERLAY_COLUMNS, Scene, StoredCharacterArcMilestone, StoredEstablishedIn,
    StoredFlexRange, StoredStatedConsequence, StoredStoryPlacement, StoredTryFailCycleStep,
    SystemOverlay, TEMPORAL_INTERVENTION_COLUMNS, TERM_COLUMNS, THEME_COLUMNS,
    TIMELINE_EVENT_COLUMNS, TemporalIntervention, Term, Theme, TimelineEvent, WORLD_RULE_COLUMNS,
    WORLD_STATE_COLUMNS, WorldRule, WorldState,
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
    RESEARCH_LOG_COLUMNS, REVISION_MARKER_COLUMNS, RelatesTo, ResearchLog, RevisionMarker,
    SAVE_POINT_COLUMNS, SCENE_BEAT_ANNOTATION_COLUMNS, SCENE_SOURCE_LINK_COLUMNS,
    SCENE_VERSION_COLUMNS, SEARCH_EMBEDDING_COLUMNS, SESSION_ACTIVITY_COLUMNS, SavePoint,
    SceneBeatAnnotation, SceneSourceLink, SceneVersion, SearchEmbedding, SessionActivity,
    StoredAnnotatedBeat, StoredChapterOutlineBeat, StoredDualPersonaReviewRound,
    VALIDATOR_FINDING_COLUMNS, ValidatorFinding, WRITER_POSITION_COLUMNS, WriterPosition,
};
use super::row::pack_embedding;
use super::row::timestamp_to_micros;

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
    pub scene_id: String,
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

struct Inner {
    pool: SqlitePool,
    data_dir: PathBuf,
    model_router: ModelRouter,
}

impl Repository {
    pub fn new(pool: SqlitePool, data_dir: PathBuf) -> Self {
        Self::with_model_router(pool, data_dir, ModelRouter::default())
    }

    pub fn with_model_router(
        pool: SqlitePool,
        data_dir: PathBuf,
        model_router: ModelRouter,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                data_dir,
                model_router,
            }),
        }
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

    pub fn current_embedding_version(&self) -> String {
        self.inner.model_router.embedding_version()
    }

    // =========================================================================
    // Project
    // =========================================================================

    pub async fn list_projects(&self) -> Result<Vec<Project>> {
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PROJECT_COLUMNS} FROM project ORDER BY created_at DESC");
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([], |r| Project::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_project(&self, id: &str) -> Result<Project> {
        let id = id.to_string();
        let project = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PROJECT_COLUMNS} FROM project WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Project::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("project not found"))?;
        Ok(project)
    }

    /// Create a project plus the initial main branch, book 1, and chapter 1.
    /// Returns the (project, book, chapter) triple matching the SurrealDB
    /// repository's contract.
    pub async fn create_project(
        &self,
        input: &CreateProjectInput,
    ) -> Result<(Project, BibleBranch, Book, Chapter)> {
        let project_id = mint_id("project");
        let branch_id = mint_id("bible_branch");
        let book_id = mint_id("book");
        let chapter_id = mint_id("chapter");
        let now = timestamp_to_micros(chrono::Utc::now());
        let stored_reader_contract: super::records::StoredReaderContract =
            input.reader_contract.clone().into();
        let reader_contract = serde_json::to_string(&stored_reader_contract)
            .context("serializing reader_contract")?;
        let name = input.name.clone();
        let project_type = input.project_type.clone();
        let genre = input.genre.clone();

        let (project_id_returned, branch_id_returned, book_id_returned, chapter_id_returned) = self
            .inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO project (id, name, project_type, genre, reader_contract, \
                     active_branch_id, notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?7)",
                    rusqlite::params![
                        &project_id,
                        &name,
                        &project_type,
                        &genre,
                        &reader_contract,
                        &branch_id,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO bible_branch (id, project_id, parent_branch_id, name, status, \
                     branch_type, description, created_from_save_point_id, created_at, updated_at) \
                     VALUES (?1, ?2, NULL, 'main', 'active', NULL, NULL, NULL, ?3, ?3)",
                    rusqlite::params![&branch_id, &project_id, now],
                )?;
                tx.execute(
                    "INSERT INTO book (id, project_id, book_number, title, created_at, updated_at) \
                     VALUES (?1, ?2, 1, NULL, ?3, ?3)",
                    rusqlite::params![&book_id, &project_id, now],
                )?;
                tx.execute(
                    "INSERT INTO chapter (id, project_id, book_id, book_number, chapter_number, \
                     title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, 1, 1, NULL, ?4, ?4)",
                    rusqlite::params![&chapter_id, &project_id, &book_id, now],
                )?;
                tx.commit()?;
                Ok((project_id, branch_id, book_id, chapter_id))
            })
            .await?;

        let project = self.get_project(&project_id_returned).await?;
        let branch = self.get_branch(&branch_id_returned).await?;
        let book = self.get_book(&book_id_returned).await?;
        let chapter = self.get_chapter(&chapter_id_returned).await?;
        Ok((project, branch, book, chapter))
    }

    // =========================================================================
    // Bible branch
    // =========================================================================

    pub async fn get_branch(&self, id: &str) -> Result<BibleBranch> {
        let id = id.to_string();
        let branch = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {BIBLE_BRANCH_COLUMNS} FROM bible_branch WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| BibleBranch::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("bible_branch not found"))?;
        Ok(branch)
    }

    /// Look up the active branch on a project, falling back to its 'main'
    /// branch if `active_branch_id` is NULL.
    pub async fn get_active_branch(&self, project_id: &str) -> Result<BibleBranch> {
        let project = self.get_project(project_id).await?;
        if let Some(branch_id) = project.active_branch_id.clone() {
            return self.get_branch(&branch_id).await;
        }
        let project_id = project_id.to_string();
        let branch = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BIBLE_BRANCH_COLUMNS} FROM bible_branch \
                     WHERE project_id = ?1 AND name = 'main'"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&project_id], |r| BibleBranch::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("no main branch for project"))?;
        Ok(branch)
    }

    pub async fn list_branches_by_project(&self, project_id: &str) -> Result<Vec<BibleBranch>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BIBLE_BRANCH_COLUMNS} FROM bible_branch \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| BibleBranch::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn create_branch(
        &self,
        project_id: &str,
        parent_branch_id: &str,
        name: &str,
        branch_type: &str,
        description: Option<String>,
    ) -> Result<BibleBranch> {
        let id = mint_id("bible_branch");
        let project_id = project_id.to_string();
        let parent_branch_id = parent_branch_id.to_string();
        let name = name.to_string();
        let branch_type = branch_type.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let id_for_lookup = id.clone();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO bible_branch (id, project_id, parent_branch_id, name, status, \
                     branch_type, description, created_from_save_point_id, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6, NULL, ?7, ?7)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &parent_branch_id,
                        &name,
                        &branch_type,
                        &description,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_branch(&id_for_lookup).await
    }

    pub async fn switch_active_branch(&self, project_id: &str, branch_id: &str) -> Result<Project> {
        let project_id_owned = project_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE project SET active_branch_id = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&branch_id_owned, now, &project_id_owned],
                )?;
                Ok(())
            })
            .await?;
        self.get_project(project_id).await
    }

    /// Persist (or clear) the project's narrator-voice directive. Passing an
    /// empty voice clears the column back to NULL.
    pub async fn set_narrator_voice(
        &self,
        project_id: &str,
        narrator_voice: spindle_core::style::NarratorVoice,
    ) -> Result<Project> {
        let project_id_owned = project_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let stored_json: Option<String> = if narrator_voice.is_empty() {
            None
        } else {
            let stored: super::records::StoredNarratorVoice = narrator_voice.into();
            Some(serde_json::to_string(&stored).context("serializing narrator_voice")?)
        };
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE project SET narrator_voice = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&stored_json, now, &project_id_owned],
                )?;
                Ok(())
            })
            .await?;
        self.get_project(project_id).await
    }

    // =========================================================================
    // Book
    // =========================================================================

    pub async fn get_book(&self, id: &str) -> Result<Book> {
        let id = id.to_string();
        let book = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {BOOK_COLUMNS} FROM book WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Book::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("book not found"))?;
        Ok(book)
    }

    // =========================================================================
    // Chapter
    // =========================================================================

    pub async fn get_chapter(&self, id: &str) -> Result<Chapter> {
        let id = id.to_string();
        let chapter = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CHAPTER_COLUMNS} FROM chapter WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Chapter::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("chapter not found"))?;
        Ok(chapter)
    }

    pub async fn list_books_by_project(&self, project_id: &str) -> Result<Vec<Book>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_COLUMNS} FROM book \
                     WHERE project_id = ?1 ORDER BY book_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Book::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Create a new book in a project, auto-incrementing `book_number` past
    /// the highest existing one.
    pub async fn create_book(&self, project_id: &str, title: Option<String>) -> Result<Book> {
        let id = mint_id("book");
        let project_id_owned = project_id.to_string();
        let id_for_return = id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let next_book_number: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(book_number), 0) + 1 FROM book WHERE project_id = ?1",
                    [&project_id_owned],
                    |r| r.get(0),
                )?;
                tx.execute(
                    "INSERT INTO book (id, project_id, book_number, title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    rusqlite::params![&id, &project_id_owned, next_book_number, &title, now],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.get_book(&id_for_return).await
    }

    /// Return the (book, chapter) pair for (book_number, chapter_number),
    /// creating either if missing. Used by save_scene_draft and plan_chapter
    /// when callers provide numeric placement that may not exist yet.
    pub async fn ensure_chapter(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Chapter> {
        if let Some(chapter) = self
            .find_chapter_by_number(project_id, book_number, chapter_number)
            .await?
        {
            return Ok(chapter);
        }
        let book = self.ensure_book(project_id, book_number).await?;
        let chapter_id = mint_id("chapter");
        let chapter_id_lookup = chapter_id.clone();
        let project_id_owned = project_id.to_string();
        let book_id_owned = book.id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO chapter (id, project_id, book_id, book_number, chapter_number, \
                     title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?6)",
                    rusqlite::params![
                        &chapter_id,
                        &project_id_owned,
                        &book_id_owned,
                        book_number,
                        chapter_number,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_chapter(&chapter_id_lookup).await
    }

    pub async fn ensure_book(&self, project_id: &str, book_number: i32) -> Result<Book> {
        if let Some(book) = self.find_book_by_number(project_id, book_number).await? {
            return Ok(book);
        }
        let book_id = mint_id("book");
        let book_id_lookup = book_id.clone();
        let project_id_owned = project_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO book (id, project_id, book_number, title, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, NULL, ?4, ?4)",
                    rusqlite::params![&book_id, &project_id_owned, book_number, now],
                )?;
                Ok(())
            })
            .await?;
        self.get_book(&book_id_lookup).await
    }

    /// Option-returning lookup of a book by (project_id, book_number).
    pub async fn find_book_by_number(
        &self,
        project_id: &str,
        book_number: i32,
    ) -> Result<Option<Book>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_COLUMNS} FROM book \
                     WHERE project_id = ?1 AND book_number = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&project_id, book_number], |r| {
                    Book::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    /// Option-returning lookup of a chapter by (project, book#, chapter#).
    pub async fn find_chapter_by_number(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Option<Chapter>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 AND book_number = ?2 AND chapter_number = ?3 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&project_id, book_number, chapter_number],
                    |r| Chapter::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    pub async fn list_chapters_by_book(&self, book_id: &str) -> Result<Vec<Chapter>> {
        let book_id = book_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE book_id = ?1 ORDER BY chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&book_id], |r| Chapter::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Variant of `list_chapters_by_book` that takes a `(project_id,
    /// book_number)` natural key instead of a `book_id`. Used by the
    /// import-hydration pass which walks manuscript book numbers directly.
    pub async fn list_chapters_by_book_number(
        &self,
        project_id: &str,
        book_number: i32,
    ) -> Result<Vec<Chapter>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 AND book_number = ?2 ORDER BY chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, book_number], |r| {
                        Chapter::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Scene
    // =========================================================================

    pub async fn get_scene(&self, id: &str) -> Result<Scene> {
        let id = id.to_string();
        let scene = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {SCENE_COLUMNS} FROM scene WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Scene::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("scene not found"))?;
        Ok(scene)
    }

    pub async fn list_scenes_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        Scene::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Scenes strictly *before* the supplied position (book, chapter, scene_order)
    /// on the given branch, ordered most-recent first by the SQL and then
    /// reversed to chronological so the caller can `take(limit)` from the end
    /// of the recent history. Mirrors the SurrealDB-era helper used by
    /// `get_writer_state` to build the recent-scenes window.
    pub async fn list_recent_scenes_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
        limit: usize,
    ) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND ( \
                            book_number < ?3 \
                            OR (book_number = ?3 AND chapter_number < ?4) \
                            OR (book_number = ?3 AND chapter_number = ?4 AND scene_order < ?5) \
                       ) \
                     ORDER BY book_number DESC, chapter_number DESC, scene_order DESC \
                     LIMIT ?6"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut rows = stmt
                    .query_map(
                        rusqlite::params![
                            &project_id,
                            &branch_id,
                            book_number,
                            chapter_number,
                            scene_order,
                            limit as i64,
                        ],
                        |r| Scene::try_from(r),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.reverse();
                Ok(rows)
            })
            .await
    }

    pub async fn list_scenes_by_chapter(&self, chapter_id: &str) -> Result<Vec<Scene>> {
        let chapter_id = chapter_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE chapter_id = ?1 ORDER BY scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&chapter_id], |r| Scene::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_scenes_by_book(&self, book_id: &str) -> Result<Vec<Scene>> {
        let book_id = book_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE book_id = ?1 ORDER BY chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&book_id], |r| Scene::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// All scenes located *after* the supplied (book, chapter, scene_order)
    /// triple in narrative order. Used by writer-state and continuity sweeps.
    pub async fn list_scenes_after_position(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND ( \
                            book_number > ?3 \
                         OR (book_number = ?3 AND chapter_number > ?4) \
                         OR (book_number = ?3 AND chapter_number = ?4 AND scene_order > ?5) \
                       ) \
                     ORDER BY book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![
                            &project_id,
                            &branch_id,
                            book_number,
                            chapter_number,
                            scene_order
                        ],
                        |r| Scene::try_from(r),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Character states persisted strictly after the supplied position on
    /// the given branch. Used by `revise_scene` to flag state markers
    /// that may be invalidated when a scene's prose is rewritten — every
    /// later state was computed against the original scene context.
    pub async fn list_character_states_after_position(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Vec<CharacterState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND ( \
                            book_number > ?3 \
                         OR (book_number = ?3 AND chapter_number > ?4) \
                         OR (book_number = ?3 AND chapter_number = ?4 AND scene_order > ?5) \
                       ) \
                     ORDER BY book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![
                            &project_id,
                            &branch_id,
                            book_number,
                            chapter_number,
                            scene_order
                        ],
                        |r| CharacterState::try_from(r),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Scene write paths (save_scene_draft + helpers)
    // =========================================================================

    pub async fn get_book_by_number(&self, project_id: &str, book_number: i32) -> Result<Book> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_COLUMNS} FROM book \
                     WHERE project_id = ?1 AND book_number = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&project_id, book_number], |r| {
                    Book::try_from(r)
                })
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("book {book_number} not found for project"))
    }

    pub async fn get_chapter_by_number(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Chapter> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 AND book_number = ?2 AND chapter_number = ?3 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&project_id, book_number, chapter_number],
                    |r| Chapter::try_from(r),
                )
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("chapter {book_number}.{chapter_number} not found for project"))
    }

    pub async fn find_scene_by_natural_key(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<Scene>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4 AND scene_order = ?5 \
                     LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![
                        &project_id,
                        &branch_id,
                        book_number,
                        chapter_number,
                        scene_order
                    ],
                    |r| Scene::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    /// Highest existing version_number for a scene plus 1. Used when creating
    /// a new scene_version snapshot from an in-place scene edit.
    pub async fn next_scene_version_number(&self, scene_id: &str) -> Result<i32> {
        let scene_id = scene_id.to_string();
        let n: i64 = self
            .inner
            .pool
            .read(move |conn| {
                conn.query_row(
                    "SELECT COALESCE(MAX(version_number), 0) FROM scene_version WHERE scene_id = ?1",
                    [&scene_id],
                    |r| r.get(0),
                )
            })
            .await?;
        Ok((n + 1) as i32)
    }

    /// Production scene-creation path. Mirrors the SurrealDB save_scene_draft
    /// flow: find or create the scene by natural key, snapshot the previous
    /// version into scene_version when prose changes, and mark dependent
    /// dual_persona_review rows stale + open validator_finding rows resolved.
    ///
    /// Returns `(Scene, created)` where `created` is true when this call
    /// inserted a new scene rather than updating an existing one.
    pub async fn save_scene_draft(
        &self,
        project_id: &str,
        branch_id: &str,
        input: &SaveSceneDraftInput,
    ) -> Result<(Scene, bool)> {
        self.persist_scene(project_id, branch_id, input, true).await
    }

    pub async fn update_scene_draft_origin(
        &self,
        scene_id: &str,
        draft_origin: &str,
    ) -> Result<()> {
        let scene_id = scene_id.to_string();
        let draft_origin = draft_origin.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE scene SET draft_origin = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![
                        &draft_origin,
                        timestamp_to_micros(chrono::Utc::now()),
                        &scene_id,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    async fn persist_scene(
        &self,
        project_id: &str,
        branch_id: &str,
        input: &SaveSceneDraftInput,
        mark_reviews_stale_on_update: bool,
    ) -> Result<(Scene, bool)> {
        let book = self
            .get_book_by_number(project_id, input.book_number)
            .await?;
        let chapter = self
            .get_chapter_by_number(project_id, input.book_number, input.chapter_number)
            .await?;
        let content_rating = input.content_rating.as_db_str();

        let existing = self
            .find_scene_by_natural_key(
                project_id,
                branch_id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?;

        let project_id_owned = project_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let book_id = book.id.clone();
        let chapter_id = chapter.id.clone();
        let full_text = input.full_text.clone();
        let summary = input.summary.clone();
        let tone = input.tone.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let book_number = input.book_number;
        let chapter_number = input.chapter_number;
        let scene_order = input.scene_order;

        if let Some(existing) = existing {
            // UPDATE path: snapshot the previous prose into scene_version when
            // it changed, then UPDATE the scene, then cascade status changes.
            let scene_changed = existing.full_text != full_text
                || existing.summary != summary
                || existing.content_rating != content_rating
                || existing.tone != tone;
            let next_version_number = if scene_changed {
                Some(self.next_scene_version_number(&existing.id).await?)
            } else {
                None
            };

            let existing_id = existing.id.clone();
            let existing_full_text = existing.full_text.clone();
            let existing_summary = existing.summary.clone();
            let existing_content_rating = existing.content_rating.clone();
            let existing_tone = existing.tone.clone();
            let new_full_text = full_text.clone();
            let new_summary = summary.clone();
            let new_content_rating = content_rating.to_string();
            let new_tone = tone.clone();

            self.inner
                .pool
                .write(move |conn| {
                    let tx = conn.transaction()?;
                    if let Some(v) = next_version_number {
                        let version_id = mint_id_local("scene_version");
                        tx.execute(
                            "INSERT INTO scene_version (id, project_id, branch_id, scene_id, \
                             version_number, book_number, chapter_number, scene_order, \
                             full_text, summary, content_rating, tone, created_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                            rusqlite::params![
                                &version_id,
                                &project_id_owned,
                                &branch_id_owned,
                                &existing_id,
                                v,
                                book_number,
                                chapter_number,
                                scene_order,
                                &existing_full_text,
                                &existing_summary,
                                &existing_content_rating,
                                &existing_tone,
                                now,
                            ],
                        )?;
                    }
                    tx.execute(
                        "UPDATE scene SET full_text = ?1, summary = ?2, content_rating = ?3, \
                         tone = ?4, updated_at = ?5 WHERE id = ?6",
                        rusqlite::params![
                            &new_full_text,
                            &new_summary,
                            &new_content_rating,
                            &new_tone,
                            now,
                            &existing_id,
                        ],
                    )?;
                    if scene_changed {
                        // Resolve any open validator findings on the prior version.
                        tx.execute(
                            "UPDATE validator_finding SET resolved_at = ?1 \
                             WHERE scene_id = ?2 AND branch_id = ?3 AND resolved_at IS NULL",
                            rusqlite::params![now, &existing_id, &branch_id_owned],
                        )?;
                    }
                    if mark_reviews_stale_on_update && scene_changed {
                        tx.execute(
                            "UPDATE dual_persona_review SET status = 'stale', updated_at = ?1 \
                             WHERE branch_id = ?2 AND scene_id = ?3",
                            rusqlite::params![now, &branch_id_owned, &existing_id],
                        )?;
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            let updated = self.get_scene(&existing.id).await?;
            Ok((updated, false))
        } else {
            // INSERT path: brand-new scene at this natural key.
            let scene_id = mint_id("scene");
            let scene_id_lookup = scene_id.clone();
            let rating_owned = content_rating.to_string();
            self.inner
                .pool
                .write(move |conn| {
                    conn.execute(
                        "INSERT INTO scene (id, project_id, branch_id, book_id, chapter_id, \
                         book_number, chapter_number, scene_order, full_text, summary, \
                         content_rating, tone, draft_origin, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?13)",
                        rusqlite::params![
                            &scene_id,
                            &project_id_owned,
                            &branch_id_owned,
                            &book_id,
                            &chapter_id,
                            book_number,
                            chapter_number,
                            scene_order,
                            &full_text,
                            &summary,
                            &rating_owned,
                            &tone,
                            now,
                        ],
                    )?;
                    Ok(())
                })
                .await?;
            let created = self.get_scene(&scene_id_lookup).await?;
            Ok((created, true))
        }
    }

    // =========================================================================
    // Character + voice/emotional profile reads
    // =========================================================================

    pub async fn get_character(&self, id: &str) -> Result<Character> {
        let id = id.to_string();
        let character = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CHARACTER_COLUMNS} FROM character WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Character::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character not found"))?;
        Ok(character)
    }

    pub async fn list_characters_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Character>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_COLUMNS} FROM character \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY normalized_name"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        Character::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_character_voice_profile(
        &self,
        character_id: &str,
    ) -> Result<CharacterVoiceProfile> {
        let character_id = character_id.to_string();
        let profile = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE character_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&character_id], |r| CharacterVoiceProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("voice profile not found"))?;
        Ok(profile)
    }

    pub async fn get_character_emotional_profile(
        &self,
        character_id: &str,
    ) -> Result<CharacterEmotionalProfile> {
        let character_id = character_id.to_string();
        let profile = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_EMOTIONAL_PROFILE_COLUMNS} FROM character_emotional_profile \
                     WHERE character_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&character_id], |r| CharacterEmotionalProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("emotional profile not found"))?;
        Ok(profile)
    }

    // =========================================================================
    // Location + world state reads
    // =========================================================================

    pub async fn get_location(&self, id: &str) -> Result<Location> {
        let id = id.to_string();
        let loc = self
            .inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {LOCATION_COLUMNS} FROM location WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Location::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("location not found"))?;
        Ok(loc)
    }

    pub async fn list_locations_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Location>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {LOCATION_COLUMNS} FROM location \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY normalized_name"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        Location::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Restore an earlier scene_version's prose into the current scene row,
    /// snapshotting the current prose into a new scene_version and marking
    /// dual_persona_review rows on the scene stale. Mirrors the SurrealDB
    /// repo's `restore_scene_version_and_mark_reviews_stale`.
    pub async fn restore_scene_version_and_mark_reviews_stale(
        &self,
        scene_id: &str,
        scene_version: &SceneVersion,
    ) -> Result<Scene> {
        // The existing save_scene_draft path takes the UPDATE branch when a
        // scene at the natural key already exists, snapshots the prior prose,
        // and marks reviews stale. We synthesize the matching input and call
        // it via the project + branch on the existing scene record.
        use spindle_core::models::{ContentRating, SaveSceneDraftInput};
        let existing = self.get_scene(scene_id).await?;
        let content_rating = match scene_version.content_rating.as_str() {
            "General" => ContentRating::General,
            "Teen" => ContentRating::Teen,
            "Mature" => ContentRating::Mature,
            "Explicit" => ContentRating::Explicit,
            other => anyhow::bail!("unknown content_rating in scene_version: {other}"),
        };
        let input = SaveSceneDraftInput {
            project_id: existing.project_id.clone(),
            book_number: existing.book_number,
            chapter_number: existing.chapter_number,
            chapter_id: None,
            scene_order: existing.scene_order,
            full_text: scene_version.full_text.clone(),
            summary: scene_version.summary.clone(),
            content_rating,
            tone: scene_version.tone.clone(),
            generation_id: None,
            source_path: None,
        };
        let (scene, _created) = self
            .save_scene_draft(&existing.project_id, &existing.branch_id, &input)
            .await?;
        Ok(scene)
    }

    /// Delete a scene. Cascades sweep its dependents (scene_version,
    /// scene_beat_annotation, revision_marker, dual_persona_review,
    /// validator_finding, canonical_fact, scene_source_link) per the FK audit.
    /// Returns the deleted scene's prior state for callers that want to log
    /// the removal.
    pub async fn delete_scene(&self, scene_id: &str) -> Result<Scene> {
        let prior = self.get_scene(scene_id).await?;
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute("DELETE FROM scene WHERE id = ?1", [&scene_id])?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(prior)
    }

    /// Move a scene to a new (book, chapter, scene_order) slot. Rejects the
    /// move if the destination is already occupied by another scene.
    #[allow(clippy::too_many_arguments)]
    pub async fn move_scene(
        &self,
        scene_id: &str,
        project_id: &str,
        branch_id: &str,
        destination_book_id: &str,
        destination_chapter_id: &str,
        destination_book_number: i32,
        destination_chapter_number: i32,
        destination_scene_order: i32,
    ) -> Result<Scene> {
        let scene_id_owned = scene_id.to_string();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let book_id = destination_book_id.to_string();
        let chapter_id = destination_chapter_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                let occupied: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM scene \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4 AND scene_order = ?5 \
                       AND id != ?6",
                    rusqlite::params![
                        &project_id,
                        &branch_id,
                        destination_book_number,
                        destination_chapter_number,
                        destination_scene_order,
                        &scene_id_owned,
                    ],
                    |r| r.get(0),
                )?;
                if occupied > 0 {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                        Some("destination scene already exists".into()),
                    ));
                }
                tx.execute(
                    "UPDATE scene SET book_id = ?1, chapter_id = ?2, book_number = ?3, \
                     chapter_number = ?4, scene_order = ?5, updated_at = ?6 WHERE id = ?7",
                    rusqlite::params![
                        &book_id,
                        &chapter_id,
                        destination_book_number,
                        destination_chapter_number,
                        destination_scene_order,
                        now,
                        &scene_id_owned,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.get_scene(scene_id).await
    }

    /// Resolve a project's active branch id without round-tripping through
    /// the full Project record. Useful when create_* helpers only need the
    /// branch_id string.
    async fn active_branch_id(&self, project_id: &str) -> Result<String> {
        Ok(self.get_active_branch(project_id).await?.id)
    }

    /// Public version of [`Self::active_branch_id`] for service-layer callers.
    pub async fn active_branch_id_public(&self, project_id: &str) -> Result<String> {
        self.active_branch_id(project_id).await
    }

    /// Create a location and its paired world_state row in one transaction.
    /// Returns both. Mirrors the SurrealDB repo's `(Location, WorldState)`
    /// return shape.
    pub async fn create_location(
        &self,
        input: &CreateLocationInput,
    ) -> Result<(Location, WorldState)> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let location_id = mint_id("location");
        let world_state_id = mint_id("world_state");
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let kind = input.kind.clone();
        let realm = input.realm.clone();
        let summary = input.summary.clone();
        let state = input.initial_state.clone();
        let sensory_json =
            serde_json::to_string(&state.sensory_details).context("serializing sensory_details")?;
        let now = timestamp_to_micros(chrono::Utc::now());
        let location_id_out = location_id.clone();
        let world_state_id_out = world_state_id.clone();

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO location (id, project_id, branch_id, name, normalized_name, \
                     kind, realm, summary, notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?9)",
                    rusqlite::params![
                        &location_id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &kind,
                        &realm,
                        &summary,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO world_state (id, project_id, branch_id, location_id, \
                     controlling_faction, status, prosperity, stability, threat_level, \
                     sensory_details, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        &world_state_id,
                        &project_id,
                        &branch_id,
                        &location_id,
                        &state.controlling_faction,
                        &state.status,
                        &state.prosperity,
                        &state.stability,
                        &state.threat_level,
                        &sensory_json,
                        now,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;

        let location = self.get_location(&location_id_out).await?;
        let world_state = self
            .get_world_state_for_location(&location.branch_id, &location_id_out)
            .await?
            .ok_or_else(|| anyhow!("world state vanished after insert: {world_state_id_out}"))?;
        Ok((location, world_state))
    }

    pub async fn create_faction(&self, input: &CreateFactionInput) -> Result<Faction> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("faction");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let faction_type = input.faction_type.clone();
        let realm = input.realm.clone();
        let summary = input.summary.clone();
        let tags_json = serde_json::to_string(&input.tags).context("serializing tags")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO faction (id, project_id, branch_id, name, normalized_name, \
                     faction_type, realm, summary, tags, notes, archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &name, &normalized,
                        &faction_type, &realm, &summary, &tags_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_faction(&id_out).await
    }

    pub async fn get_faction(&self, id: &str) -> Result<Faction> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {FACTION_COLUMNS} FROM faction WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Faction::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("faction not found"))
    }

    pub async fn create_religion(&self, input: &CreateReligionInput) -> Result<Religion> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("religion");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let deity_or_principle = input.deity_or_principle.clone();
        let summary = input.summary.clone();
        let tags_json = serde_json::to_string(&input.tags).context("serializing tags")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO religion (id, project_id, branch_id, name, normalized_name, \
                     deity_or_principle, summary, tags, notes, archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &name, &normalized,
                        &deity_or_principle, &summary, &tags_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_religion(&id_out).await
    }

    pub async fn get_religion(&self, id: &str) -> Result<Religion> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {RELIGION_COLUMNS} FROM religion WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Religion::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("religion not found"))
    }

    pub async fn create_economy(&self, input: &CreateEconomyInput) -> Result<Economy> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("economy");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let realm = input.realm.clone();
        let summary = input.summary.clone();
        let scarce = serde_json::to_string(&input.scarce_resources)
            .context("serializing scarce_resources")?;
        let trade = serde_json::to_string(&input.trade_goods).context("serializing trade_goods")?;
        let currency = input.currency.clone();
        // The spindle-core CreateEconomyInput.notes is Vec<String> for the API,
        // but the SQLite economy.notes column is `TEXT NULL` for a free-form
        // human note. We serialize the vec to JSON and store it there to
        // preserve the existing semantics until the upstream contract is
        // reconciled.
        let notes_json = if input.notes.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&input.notes).context("serializing notes")?)
        };
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO economy (id, project_id, branch_id, name, normalized_name, \
                     realm, summary, scarce_resources, trade_goods, currency, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, ?12, ?12)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &realm,
                        &summary,
                        &scarce,
                        &trade,
                        &currency,
                        &notes_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_economy(&id_out).await
    }

    pub async fn get_economy(&self, id: &str) -> Result<Economy> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {ECONOMY_COLUMNS} FROM economy WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Economy::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("economy not found"))
    }

    pub async fn create_term(&self, input: &CreateTermInput) -> Result<Term> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("term");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let term_text = input.term_text.clone();
        let normalized = normalize_name(&input.term_text);
        let pronunciation = input.pronunciation.clone();
        let definition = input.definition.clone();
        let usage_context = input.usage_context.clone();
        let origin = input.origin.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO term (id, project_id, branch_id, term_text, normalized_term, \
                     pronunciation, definition, usage_context, origin, notes, archived_at, \
                     created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &term_text,
                        &normalized,
                        &pronunciation,
                        &definition,
                        &usage_context,
                        &origin,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_term(&id_out).await
    }

    pub async fn get_term(&self, id: &str) -> Result<Term> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {TERM_COLUMNS} FROM term WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Term::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("term not found"))
    }

    pub async fn create_plot_line(&self, input: &CreatePlotLineInput) -> Result<PlotLine> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("plot_line");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let plot_type = input.plot_type.clone();
        let summary = input.summary.clone();
        let status = input
            .status
            .clone()
            .unwrap_or_else(|| "planted".to_string());
        let convergence_points: Vec<StoredStoryPlacement> = input
            .convergence_points
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let convergence_json =
            serde_json::to_string(&convergence_points).context("serializing convergence_points")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO plot_line (id, project_id, branch_id, name, normalized_name, \
                     plot_type, summary, status, convergence_points, notes, archived_at, \
                     created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &plot_type,
                        &summary,
                        &status,
                        &convergence_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_plot_line(&id_out).await
    }

    pub async fn get_plot_line(&self, id: &str) -> Result<PlotLine> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PLOT_LINE_COLUMNS} FROM plot_line WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PlotLine::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("plot_line not found"))
    }

    pub async fn create_conflict(&self, input: &CreateConflictInput) -> Result<Conflict> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("conflict");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let conflict_type = input.conflict_type.clone();
        let stakes = input.stakes.clone();
        let escalation_stages_json = serde_json::to_string(&input.escalation_stages)
            .context("serializing escalation_stages")?;
        let try_fail_cycles: Vec<StoredTryFailCycleStep> = input
            .try_fail_cycles
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let try_fail_json =
            serde_json::to_string(&try_fail_cycles).context("serializing try_fail_cycles")?;
        let stated_consequences: Vec<StoredStatedConsequence> = input
            .stated_consequences
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let consequences_json = serde_json::to_string(&stated_consequences)
            .context("serializing stated_consequences")?;
        let expected_total_cycles = input.expected_total_cycles;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO conflict (id, project_id, branch_id, name, normalized_name, \
                     conflict_type, stakes, escalation_stages, expected_total_cycles, \
                     try_fail_cycles, stated_consequences, resolution_summary, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL, NULL, NULL, ?12, ?12)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &name, &normalized,
                        &conflict_type, &stakes, &escalation_stages_json,
                        expected_total_cycles, &try_fail_json, &consequences_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_conflict(&id_out).await
    }

    pub async fn get_conflict(&self, id: &str) -> Result<Conflict> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CONFLICT_COLUMNS} FROM conflict WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Conflict::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("conflict not found"))
    }

    pub async fn create_theme(&self, input: &CreateThemeInput) -> Result<Theme> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("theme");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let theme_statement = input.theme_statement.clone();
        let thesis_antithesis = input.thesis_antithesis.clone();
        let introduction_point: Option<StoredStoryPlacement> =
            input.introduction_point.clone().map(Into::into);
        let resolution_point: Option<StoredStoryPlacement> =
            input.resolution_point.clone().map(Into::into);
        let introduction_json = match introduction_point.as_ref() {
            Some(p) => Some(serde_json::to_string(p).context("serializing introduction_point")?),
            None => None,
        };
        let resolution_json = match resolution_point.as_ref() {
            Some(p) => Some(serde_json::to_string(p).context("serializing resolution_point")?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO theme (id, project_id, branch_id, theme_statement, \
                     thesis_antithesis, introduction_point, resolution_point, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?8)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &theme_statement,
                        &thesis_antithesis,
                        &introduction_json,
                        &resolution_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_theme(&id_out).await
    }

    pub async fn get_theme(&self, id: &str) -> Result<Theme> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {THEME_COLUMNS} FROM theme WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Theme::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("theme not found"))
    }

    pub async fn create_motif(&self, input: &CreateMotifInput) -> Result<Motif> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("motif");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let description = input.description.clone();
        let max_uses_per_chapter = input.max_uses_per_chapter;
        let connected_theme_json = serde_json::to_string(&input.connected_theme_ids)
            .context("serializing connected_theme_ids")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO motif (id, project_id, branch_id, name, normalized_name, \
                     description, max_uses_per_chapter, connected_theme_ids, notes, \
                     archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &description,
                        max_uses_per_chapter,
                        &connected_theme_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_motif(&id_out).await
    }

    pub async fn get_motif(&self, id: &str) -> Result<Motif> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {MOTIF_COLUMNS} FROM motif WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| Motif::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("motif not found"))
    }

    pub async fn create_narrative_promise(
        &self,
        input: &CreateNarrativePromiseInput,
    ) -> Result<NarrativePromise> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("narrative_promise");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let promise_type = input.promise_type.clone();
        let description = input.description.clone();
        let planted_at: StoredStoryPlacement = input.planted_at.clone().into();
        let planted_json = serde_json::to_string(&planted_at).context("serializing planted_at")?;
        let planned_payoff: Option<StoredStoryPlacement> =
            input.planned_payoff.clone().map(Into::into);
        let payoff_json = match planned_payoff.as_ref() {
            Some(p) => Some(serde_json::to_string(p).context("serializing planned_payoff")?),
            None => None,
        };
        let notes_json = serde_json::to_string(&input.notes).context("serializing notes")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO narrative_promise (id, project_id, branch_id, promise_type, \
                     description, status, planted_at, planned_payoff, notes, archived_at, \
                     created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'planted', ?6, ?7, ?8, NULL, ?9, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &promise_type,
                        &description,
                        &planted_json,
                        &payoff_json,
                        &notes_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_narrative_promise(&id_out).await
    }

    pub async fn get_narrative_promise(&self, id: &str) -> Result<NarrativePromise> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| NarrativePromise::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("narrative_promise not found"))
    }

    // -------------------------------------------------------------------------
    // List methods for project+branch-scoped narrative entities.
    // All follow the same pattern: WHERE project_id = ?1 AND branch_id = ?2
    // ORDER BY normalized_name (or the entity's natural sort key).
    // -------------------------------------------------------------------------

    pub async fn list_factions_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Faction>> {
        list_branch_scoped(
            &self.inner.pool,
            FACTION_COLUMNS,
            "faction",
            "normalized_name",
            project_id,
            branch_id,
            |r| Faction::try_from(r),
        )
        .await
    }

    pub async fn list_religions_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Religion>> {
        list_branch_scoped(
            &self.inner.pool,
            RELIGION_COLUMNS,
            "religion",
            "normalized_name",
            project_id,
            branch_id,
            |r| Religion::try_from(r),
        )
        .await
    }

    pub async fn list_economies_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Economy>> {
        list_branch_scoped(
            &self.inner.pool,
            ECONOMY_COLUMNS,
            "economy",
            "normalized_name",
            project_id,
            branch_id,
            |r| Economy::try_from(r),
        )
        .await
    }

    pub async fn list_terms_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Term>> {
        list_branch_scoped(
            &self.inner.pool,
            TERM_COLUMNS,
            "term",
            "normalized_term",
            project_id,
            branch_id,
            |r| Term::try_from(r),
        )
        .await
    }

    pub async fn list_plot_lines_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<PlotLine>> {
        list_branch_scoped(
            &self.inner.pool,
            PLOT_LINE_COLUMNS,
            "plot_line",
            "normalized_name",
            project_id,
            branch_id,
            |r| PlotLine::try_from(r),
        )
        .await
    }

    pub async fn list_conflicts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Conflict>> {
        list_branch_scoped(
            &self.inner.pool,
            CONFLICT_COLUMNS,
            "conflict",
            "normalized_name",
            project_id,
            branch_id,
            |r| Conflict::try_from(r),
        )
        .await
    }

    pub async fn list_themes_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Theme>> {
        list_branch_scoped(
            &self.inner.pool,
            THEME_COLUMNS,
            "theme",
            "created_at",
            project_id,
            branch_id,
            |r| Theme::try_from(r),
        )
        .await
    }

    pub async fn list_motifs_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Motif>> {
        list_branch_scoped(
            &self.inner.pool,
            MOTIF_COLUMNS,
            "motif",
            "normalized_name",
            project_id,
            branch_id,
            |r| Motif::try_from(r),
        )
        .await
    }

    // =========================================================================
    // World rule
    // =========================================================================

    pub async fn create_world_rule(&self, input: &CreateWorldRuleInput) -> Result<WorldRule> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("world_rule");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let rule_name = input.rule_name.clone();
        let rule_type = input.rule_type.clone();
        let description = input.description.clone();
        let scan_pattern = input.scan_pattern.clone();
        let established: Option<StoredEstablishedIn> = input.established_in.clone().map(Into::into);
        let established_json = match established.as_ref() {
            Some(e) => Some(serde_json::to_string(e).context("serializing established_in")?),
            None => None,
        };
        let relevance_tags_json = if input.relevance_tags.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&input.relevance_tags)
                    .context("serializing relevance_tags")?,
            )
        };
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO world_rule (id, project_id, branch_id, rule_name, rule_type, \
                     description, established_in, relevance_tags, scan_pattern, notes, \
                     created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &rule_name,
                        &rule_type,
                        &description,
                        &established_json,
                        &relevance_tags_json,
                        &scan_pattern,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_world_rule(&id_out).await
    }

    pub async fn get_world_rule(&self, id: &str) -> Result<WorldRule> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {WORLD_RULE_COLUMNS} FROM world_rule WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| WorldRule::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("world_rule not found"))
    }

    pub async fn list_world_rules_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<WorldRule>> {
        list_branch_scoped(
            &self.inner.pool,
            WORLD_RULE_COLUMNS,
            "world_rule",
            "rule_type, rule_name",
            project_id,
            branch_id,
            |r| WorldRule::try_from(r),
        )
        .await
    }

    // =========================================================================
    // Character arc
    // =========================================================================

    pub async fn create_character_arc(
        &self,
        input: &CreateCharacterArcInput,
    ) -> Result<CharacterArc> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("character_arc");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let character_id = input.character_id.clone();
        let arc_type = input.arc_type.clone();
        let starting_state = input.starting_state.clone();
        let ending_state = input.ending_state.clone();
        let thematic_purpose = input.thematic_purpose.clone();
        let milestones: Vec<StoredCharacterArcMilestone> =
            input.milestones.iter().cloned().map(Into::into).collect();
        let milestones_json =
            serde_json::to_string(&milestones).context("serializing milestones")?;
        let connected_theme_json = serde_json::to_string(&input.connected_theme_ids)
            .context("serializing connected_theme_ids")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO character_arc (id, project_id, branch_id, character_id, \
                     arc_type, starting_state, ending_state, milestones, thematic_purpose, \
                     connected_theme_ids, status, progress, notes, archived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'planned', 0.0, NULL, NULL, ?11, ?11)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &character_id,
                        &arc_type,
                        &starting_state,
                        &ending_state,
                        &milestones_json,
                        &thematic_purpose,
                        &connected_theme_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_character_arc(&id_out).await
    }

    pub async fn get_character_arc(&self, id: &str) -> Result<CharacterArc> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CHARACTER_ARC_COLUMNS} FROM character_arc WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterArc::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character_arc not found"))
    }

    // =========================================================================
    // Character creation (4-table transaction)
    // =========================================================================

    /// Create a character along with its voice profile, emotional profile, and
    /// the baseline `character_state` snapshot in a single transaction.
    ///
    /// Atomic semantics: any failure rolls all four inserts back, so a
    /// half-formed character can never appear in the database.
    pub async fn create_character(
        &self,
        input: &CreateCharacterInput,
    ) -> Result<(
        Character,
        CharacterVoiceProfile,
        CharacterEmotionalProfile,
        CharacterState,
    )> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let character_id = mint_id("character");
        let voice_id = mint_id("character_voice_profile");
        let emotional_id = mint_id("character_emotional_profile");
        let state_id = mint_id("character_state");
        let now = timestamp_to_micros(chrono::Utc::now());

        let project_id = input.project_id.clone();
        let name = input.name.clone();
        let normalized = normalize_name(&input.name);
        let summary = input.summary.clone();
        let role = input.role.clone();
        let realm = input.realm.clone();

        // Voice profile (with tone + established_in_scene_id from input).
        let voice = input.voice_profile.clone();
        let vocab_json = serde_json::to_string(&voice.vocabulary)?;
        let sentence_json = serde_json::to_string(&voice.sentence_structure)?;
        let tics_json = serde_json::to_string(&voice.tics)?;
        let forbidden_json = serde_json::to_string(&voice.forbidden_words)?;
        let example_json = serde_json::to_string(&voice.example_lines)?;
        let voice_tone = voice.tone;
        let voice_established = voice.established_in_scene_id;

        // Emotional profile.
        let emo = input.emotional_profile.clone();
        let base_emotions_json = serde_json::to_string(&emo.base_emotions)?;
        let suppressed_json = serde_json::to_string(&emo.suppressed)?;
        let triggers_json = serde_json::to_string(&emo.triggers)?;
        let defense_json = serde_json::to_string(&emo.defense_mechanisms)?;
        let flex_stored: Option<StoredFlexRange> = emo.flex_range.map(Into::into);
        let flex_json = match flex_stored.as_ref() {
            Some(f) => Some(serde_json::to_string(f)?),
            None => None,
        };

        // Initial character_state (defaults match the SurrealDB repo).
        let patch = input.initial_state.clone().unwrap_or(CharacterStatePatch {
            emotional_state: std::collections::BTreeMap::new(),
            goals: Some(Vec::new()),
            status: Some(Vec::new()),
            notes: Some(Vec::new()),
            source_summary: Some("baseline".to_string()),
        });
        let emotional_state_json = serde_json::to_string(&patch.emotional_state)?;
        let goals_json = serde_json::to_string(&patch.goals.unwrap_or_default())?;
        let status_json = serde_json::to_string(&patch.status.unwrap_or_default())?;
        let notes_json = serde_json::to_string(&patch.notes.unwrap_or_default())?;
        let source_summary = patch.source_summary;

        let character_id_out = character_id.clone();
        let voice_id_out = voice_id.clone();
        let emotional_id_out = emotional_id.clone();
        let state_id_out = state_id.clone();

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO character (id, project_id, branch_id, name, normalized_name, \
                     summary, role, realm, appearance, notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?9)",
                    rusqlite::params![
                        &character_id,
                        &project_id,
                        &branch_id,
                        &name,
                        &normalized,
                        &summary,
                        &role,
                        &realm,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO character_voice_profile (id, character_id, vocabulary, \
                     sentence_structure, tics, forbidden_words, example_lines, tone, \
                     established_in_scene_id, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![
                        &voice_id,
                        &character_id,
                        &vocab_json,
                        &sentence_json,
                        &tics_json,
                        &forbidden_json,
                        &example_json,
                        &voice_tone,
                        &voice_established,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO character_emotional_profile (id, character_id, base_emotions, \
                     suppressed, triggers, defense_mechanisms, flex_range, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                    rusqlite::params![
                        &emotional_id,
                        &character_id,
                        &base_emotions_json,
                        &suppressed_json,
                        &triggers_json,
                        &defense_json,
                        &flex_json,
                        now,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO character_state (id, project_id, branch_id, character_id, \
                     scene_id, book_number, chapter_number, scene_order, emotional_state, \
                     goals, status, notes, source_summary, created_at) \
                     VALUES (?1, ?2, ?3, ?4, NULL, 1, 1, 0, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        &state_id,
                        &project_id,
                        &branch_id,
                        &character_id,
                        &emotional_state_json,
                        &goals_json,
                        &status_json,
                        &notes_json,
                        &source_summary,
                        now,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;

        let character = self.get_character(&character_id_out).await?;
        let voice = self
            .get_character_voice_profile_by_id(&voice_id_out)
            .await?;
        let emotional = self
            .get_character_emotional_profile_by_id(&emotional_id_out)
            .await?;
        let state = self.get_character_state_by_id(&state_id_out).await?;
        Ok((character, voice, emotional, state))
    }

    /// Direct-by-id getter for the freshly inserted voice profile. Public so
    /// callers can re-fetch by id; the existing get_character_voice_profile
    /// looks up by character_id which is the more common access pattern.
    /// UPSERT the voice profile for a character. The profile is per-character
    /// (not per-branch), so the unique key is character_id alone.
    pub async fn set_character_voice_profile(
        &self,
        character_id: &str,
        profile: &spindle_core::models::CharacterVoiceProfileData,
    ) -> Result<CharacterVoiceProfile> {
        let new_id = mint_id("character_voice_profile");
        let character_id_owned = character_id.to_string();
        let tone = profile.tone.clone();
        let vocabulary = serde_json::to_string(&profile.vocabulary)?;
        let sentence_structure = serde_json::to_string(&profile.sentence_structure)?;
        let tics = serde_json::to_string(&profile.tics)?;
        let forbidden_words = serde_json::to_string(&profile.forbidden_words)?;
        let example_lines = serde_json::to_string(&profile.example_lines)?;
        let established = profile.established_in_scene_id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let character_id = character_id_owned.clone();
                move |conn| {
                    conn.execute(
                        "INSERT INTO character_voice_profile (id, character_id, vocabulary, \
                         sentence_structure, tics, forbidden_words, example_lines, tone, \
                         established_in_scene_id, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10) \
                         ON CONFLICT (character_id) DO UPDATE SET \
                            vocabulary = excluded.vocabulary, \
                            sentence_structure = excluded.sentence_structure, \
                            tics = excluded.tics, \
                            forbidden_words = excluded.forbidden_words, \
                            example_lines = excluded.example_lines, \
                            tone = excluded.tone, \
                            established_in_scene_id = excluded.established_in_scene_id, \
                            updated_at = excluded.updated_at",
                        rusqlite::params![
                            &new_id,
                            &character_id,
                            &vocabulary,
                            &sentence_structure,
                            &tics,
                            &forbidden_words,
                            &example_lines,
                            &tone,
                            &established,
                            now,
                        ],
                    )?;
                    Ok(())
                }
            })
            .await?;
        self.get_character_voice_profile(&character_id_owned).await
    }

    pub async fn get_character_voice_profile_by_id(
        &self,
        id: &str,
    ) -> Result<CharacterVoiceProfile> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterVoiceProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("voice profile not found"))
    }

    pub async fn get_character_emotional_profile_by_id(
        &self,
        id: &str,
    ) -> Result<CharacterEmotionalProfile> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_EMOTIONAL_PROFILE_COLUMNS} FROM character_emotional_profile \
                     WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterEmotionalProfile::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("emotional profile not found"))
    }

    pub async fn get_character_state_by_id(&self, id: &str) -> Result<CharacterState> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CHARACTER_STATE_COLUMNS} FROM character_state WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CharacterState::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character_state not found"))
    }

    // =========================================================================
    // Timeline, temporal interventions, system overlays
    // =========================================================================

    pub async fn create_timeline_event(
        &self,
        input: &CreateTimelineEventInput,
    ) -> Result<TimelineEvent> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("timeline_event");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let title = input.title.clone();
        let event_type = input.event_type.clone();
        let placement: StoredStoryPlacement = input.placement.clone().into();
        let placement_json = serde_json::to_string(&placement)?;
        let summary = input.summary.clone();
        let related_json = serde_json::to_string(&input.related_entity_ids)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO timeline_event (id, project_id, branch_id, title, event_type, \
                     placement, summary, related_entity_ids, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &title,
                        &event_type,
                        &placement_json,
                        &summary,
                        &related_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_timeline_event(&id_out).await
    }

    pub async fn get_timeline_event(&self, id: &str) -> Result<TimelineEvent> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {TIMELINE_EVENT_COLUMNS} FROM timeline_event WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| TimelineEvent::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("timeline_event not found"))
    }

    pub async fn list_timeline_events_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<TimelineEvent>> {
        list_branch_scoped(
            &self.inner.pool,
            TIMELINE_EVENT_COLUMNS,
            "timeline_event",
            "title",
            project_id,
            branch_id,
            |r| TimelineEvent::try_from(r),
        )
        .await
    }

    pub async fn create_temporal_intervention(
        &self,
        source_event_id: Option<String>,
        target_event_id: Option<String>,
        input: &CreateTemporalInterventionInput,
    ) -> Result<TemporalIntervention> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("temporal_intervention");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let title = input.title.clone();
        let intervention_type = input.intervention_type.clone();
        let summary = input.summary.clone();
        let consequences_json = serde_json::to_string(&input.consequences)?;
        let status = input
            .status
            .clone()
            .unwrap_or_else(|| "planned".to_string());
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO temporal_intervention (id, project_id, branch_id, title, \
                     intervention_type, source_event_id, target_event_id, summary, consequences, \
                     status, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &title,
                        &intervention_type,
                        &source_event_id,
                        &target_event_id,
                        &summary,
                        &consequences_json,
                        &status,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_temporal_intervention(&id_out).await
    }

    pub async fn get_temporal_intervention(&self, id: &str) -> Result<TemporalIntervention> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TEMPORAL_INTERVENTION_COLUMNS} FROM temporal_intervention WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| TemporalIntervention::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("temporal_intervention not found"))
    }

    pub async fn create_system_overlay(
        &self,
        input: &CreateSystemOverlayInput,
    ) -> Result<SystemOverlay> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("system_overlay");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let system_name = input.system_name.clone();
        let normalized = normalize_name(&input.system_name);
        let system_type = input.system_type.clone();
        let rules = input.rules.clone();
        let visibility = input.visibility.clone();
        let progression_currency = input.progression_currency.clone();
        let stats_json = serde_json::to_string(&input.stats)?;
        let advancement_json = serde_json::to_string(&input.advancement_tiers)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO system_overlay (id, project_id, branch_id, system_name, \
                     normalized_name, system_type, rules, visibility, progression_currency, \
                     stats, advancement_tiers, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &system_name,
                        &normalized,
                        &system_type,
                        &rules,
                        &visibility,
                        &progression_currency,
                        &stats_json,
                        &advancement_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_system_overlay(&id_out).await
    }

    pub async fn get_system_overlay(&self, id: &str) -> Result<SystemOverlay> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {SYSTEM_OVERLAY_COLUMNS} FROM system_overlay WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| SystemOverlay::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("system_overlay not found"))
    }

    pub async fn list_system_overlays_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SystemOverlay>> {
        list_branch_scoped(
            &self.inner.pool,
            SYSTEM_OVERLAY_COLUMNS,
            "system_overlay",
            "normalized_name",
            project_id,
            branch_id,
            |r| SystemOverlay::try_from(r),
        )
        .await
    }

    // =========================================================================
    // Canonical fact (continuity tracking)
    // =========================================================================

    pub async fn create_canonical_fact(
        &self,
        params: CreateCanonicalFactParams,
    ) -> Result<CanonicalFact> {
        let id = mint_id("canonical_fact");
        let id_out = id.clone();
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
        let CreateCanonicalFactParams {
            project_id,
            branch_id,
            scene_id,
            book_number,
            chapter_number,
            subject_table,
            subject_id,
            predicate,
            value_kind,
            value_text,
            value_number,
            unit,
            scope,
            legacy_untyped,
            ..
        } = params;
        let _ = legacy_untyped; // column dropped after v029; kept in params for caller parity.

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO canonical_fact (id, project_id, branch_id, scene_id, \
                     source_scene_id, book_number, chapter_number, subject_table, subject_id, \
                     predicate, value_kind, value_number, value_text, value_json, unit, aliases, \
                     scope, valid_from, valid_until, superseded_by, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                             ?15, ?16, ?17, ?18, NULL, ?19, ?19)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &scene_id,
                        book_number,
                        chapter_number,
                        &subject_table,
                        &subject_id,
                        &predicate,
                        &value_kind,
                        value_number,
                        &value_text,
                        &value_json_str,
                        &unit,
                        &aliases_json,
                        &scope,
                        &valid_from_json,
                        &valid_until_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_canonical_fact(&id_out).await
    }

    pub async fn get_canonical_fact(&self, id: &str) -> Result<CanonicalFact> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| CanonicalFact::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("canonical_fact not found"))
    }

    /// Canonical facts about a specific subject (table+id), ordered by
    /// position in the manuscript. Used by continuity-check flows that
    /// need every claim about an entity in narrative order.
    pub async fn list_canonical_facts_by_subject(
        &self,
        project_id: &str,
        branch_id: &str,
        subject_table: &str,
        subject_id: Option<&str>,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let subject_table = subject_table.to_string();
        let subject_id = subject_id.map(|s| s.to_string());
        self.inner
            .pool
            .read(move |conn| {
                let (sql, has_subject) = if subject_id.is_some() {
                    (
                        format!(
                            "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                             WHERE project_id = ?1 AND branch_id = ?2 \
                               AND subject_table = ?3 AND subject_id = ?4 \
                             ORDER BY book_number, chapter_number"
                        ),
                        true,
                    )
                } else {
                    (
                        format!(
                            "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                             WHERE project_id = ?1 AND branch_id = ?2 \
                               AND subject_table = ?3 \
                             ORDER BY book_number, chapter_number"
                        ),
                        false,
                    )
                };
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = if has_subject {
                    stmt.query_map(
                        rusqlite::params![
                            &project_id,
                            &branch_id,
                            &subject_table,
                            &subject_id.unwrap()
                        ],
                        |r| CanonicalFact::try_from(r),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(
                        rusqlite::params![&project_id, &branch_id, &subject_table],
                        |r| CanonicalFact::try_from(r),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await
    }

    /// All canonical facts for a project's active branch, ordered by
    /// position in the manuscript.
    /// Active canonical facts on the project's active branch — convenience
    /// wrapper around the branch-scoped variant.
    pub async fn list_active_canonical_facts_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_active_canonical_facts_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Project-wide canonical facts (`subject_table = 'project'`) active on
    /// a specific branch, filtered to those whose position is at-or-before
    /// the supplied placement. Mirrors the SurrealDB reference of the same
    /// name in 705b835^ (repository.rs:4813..4833). Used by
    /// `get_scene_context` and `get_chapter_briefing` to surface global
    /// constraints alongside subject-scoped ones.
    pub async fn list_canonical_facts_for_project_wide(
        &self,
        project_id: &str,
        branch_id: &str,
        placement: &spindle_core::models::StoryPlacement,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let book_number = placement.book_number;
        let chapter_number = placement.chapter_number;
        let scene_order = placement.scene_order.unwrap_or(0);
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND superseded_by IS NULL \
                       AND subject_table = 'project' \
                       AND book_number <= ?3 \
                       AND (book_number < ?3 OR chapter_number <= ?4) \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                        |r| CanonicalFact::try_from(r),
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let placement = spindle_core::models::StoryPlacement {
                    book_number,
                    chapter_number,
                    scene_order: Some(scene_order),
                    note: None,
                };
                Ok(rows
                    .into_iter()
                    .filter(|fact| fact_at_or_before(fact, &placement))
                    .collect())
            })
            .await
    }

    /// Canonical facts scoped to a list of subjects (per-table+id), active
    /// on a branch and at-or-before the supplied placement. Mirrors the
    /// SurrealDB reference (repository.rs:4748..4812 in 705b835^).
    pub async fn list_canonical_facts_for_subjects(
        &self,
        project_id: &str,
        branch_id: &str,
        subjects: &[spindle_core::subject::Subject],
        placement: &spindle_core::models::StoryPlacement,
    ) -> Result<Vec<CanonicalFact>> {
        // Deduplicate (table, id) pairs and drop project-wide subjects, which
        // are handled by the project-wide variant above.
        let mut seen = std::collections::BTreeSet::new();
        let mut pairs: Vec<(String, String)> = Vec::new();
        for subject in subjects {
            let Some(subject_id) = subject.id() else {
                continue;
            };
            let table = subject.table().as_str().to_string();
            let key = format!("{table}:{subject_id}");
            if seen.insert(key) {
                pairs.push((table, subject_id.to_string()));
            }
        }
        if pairs.is_empty() {
            return Ok(Vec::new());
        }

        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let book_number = placement.book_number;
        let chapter_number = placement.chapter_number;
        let scene_order = placement.scene_order.unwrap_or(0);
        self.inner
            .pool
            .read(move |conn| {
                let mut subject_clauses = Vec::with_capacity(pairs.len());
                for idx in 0..pairs.len() {
                    let table_ph = 5 + idx * 2;
                    let id_ph = table_ph + 1;
                    subject_clauses.push(format!(
                        "(subject_table = ?{table_ph} AND subject_id = ?{id_ph})"
                    ));
                }
                let where_subjects = subject_clauses.join(" OR ");
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND superseded_by IS NULL \
                       AND book_number <= ?3 \
                       AND (book_number < ?3 OR chapter_number <= ?4) \
                       AND ({where_subjects}) \
                     ORDER BY book_number, chapter_number"
                );

                let mut params: Vec<Box<dyn rusqlite::ToSql>> =
                    Vec::with_capacity(4 + pairs.len() * 2);
                params.push(Box::new(project_id.clone()));
                params.push(Box::new(branch_id.clone()));
                params.push(Box::new(book_number));
                params.push(Box::new(chapter_number));
                for (table, subject_id) in &pairs {
                    params.push(Box::new(table.clone()));
                    params.push(Box::new(subject_id.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|b| b.as_ref()).collect();

                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(&param_refs[..], |r| CanonicalFact::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let placement = spindle_core::models::StoryPlacement {
                    book_number,
                    chapter_number,
                    scene_order: Some(scene_order),
                    note: None,
                };
                Ok(rows
                    .into_iter()
                    .filter(|fact| fact.subject_table != "project")
                    .filter(|fact| fact_at_or_before(fact, &placement))
                    .collect())
            })
            .await
    }

    pub async fn list_canonical_facts_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_canonical_facts_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Active (non-superseded) canonical facts on a branch, ordered by
    /// position. Filters out rows whose `superseded_by` points at a still-
    /// valid newer fact.
    pub async fn list_active_canonical_facts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 AND superseded_by IS NULL \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        CanonicalFact::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn set_canonical_fact_legacy_untyped(
        &self,
        fact_id: &str,
        _legacy_untyped: bool,
    ) -> Result<()> {
        // legacy_untyped column was dropped in v029. The SurrealDB API
        // accepted a bool but it's a no-op against the post-v029 schema.
        // Kept for caller-parity through the migration; remove after
        // services migrate to a no-flag API.
        let _ = fact_id;
        Ok(())
    }

    /// Mark every open validator_finding for a specific validator on a branch
    /// as resolved (sets resolved_at). Returns the number of rows updated.
    pub async fn resolve_validator_findings_for_validator(
        &self,
        project_id: &str,
        branch_id: &str,
        validator_id: &str,
    ) -> Result<usize> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let validator_id = validator_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE validator_finding SET resolved_at = ?1 \
                     WHERE project_id = ?2 AND branch_id = ?3 \
                       AND validator_id = ?4 AND resolved_at IS NULL",
                    rusqlite::params![now, &project_id, &branch_id, &validator_id],
                )
            })
            .await
    }

    /// Mark every open validator_finding tied to the given scenes as resolved.
    pub async fn resolve_validator_findings_for_scenes(
        &self,
        branch_id: &str,
        scene_ids: &[String],
    ) -> Result<usize> {
        if scene_ids.is_empty() {
            return Ok(0);
        }
        let branch_id = branch_id.to_string();
        let scene_ids: Vec<String> = scene_ids.to_vec();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
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
                for id in &scene_ids {
                    params.push(id);
                }
                conn.execute(&sql, &params[..])
            })
            .await
    }

    pub async fn supersede_canonical_fact(
        &self,
        old_fact_id: &str,
        new_fact_id: &str,
    ) -> Result<()> {
        let old_fact_id = old_fact_id.to_string();
        let new_fact_id = new_fact_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE canonical_fact SET superseded_by = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&new_fact_id, now, &old_fact_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_canonical_facts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        CanonicalFact::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Knowledge facts + knows edges
    // =========================================================================

    pub async fn upsert_knowledge_fact(
        &self,
        params: UpsertKnowledgeFactParams,
    ) -> Result<KnowledgeFact> {
        let id = mint_id("knowledge_fact");
        let id_out = id.clone();
        let learned_at_stored: Option<StoredStoryPlacement> = params.learned_at.map(Into::into);
        let learned_at_json = match learned_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let tags_json = serde_json::to_string(&params.tags)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        let reader_visible = if params.reader_visible { 1 } else { 0 };
        let UpsertKnowledgeFactParams {
            project_id,
            branch_id,
            character_id,
            fact,
            normalized_fact,
            source_summary,
            confidence,
            source_import_session_id,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                let character_id = character_id.clone();
                let normalized_fact = normalized_fact.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM knowledge_fact \
                         WHERE project_id = ?1 AND branch_id = ?2 AND character_id = ?3 \
                           AND normalized_fact = ?4",
                        rusqlite::params![&project_id, &branch_id, &character_id, &normalized_fact],
                    )?;
                    tx.execute(
                        "INSERT INTO knowledge_fact (id, project_id, branch_id, character_id, \
                         fact, normalized_fact, source_summary, learned_at, confidence, tags, \
                         reader_visible, source_import_session_id, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
                        rusqlite::params![
                            &id,
                            &project_id,
                            &branch_id,
                            &character_id,
                            &fact,
                            &normalized_fact,
                            &source_summary,
                            &learned_at_json,
                            confidence,
                            &tags_json,
                            reader_visible,
                            &source_import_session_id,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_knowledge_fact(&id_out).await
    }

    pub async fn get_knowledge_fact(&self, id: &str) -> Result<KnowledgeFact> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| KnowledgeFact::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("knowledge_fact not found"))
    }

    pub async fn upsert_knows(&self, params: UpsertKnowsParams) -> Result<Knows> {
        let learned_at_stored: Option<StoredStoryPlacement> = params.learned_at.map(Into::into);
        let learned_at_json = match learned_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let reader_visible = if params.reader_visible { 1 } else { 0 };
        let UpsertKnowsParams {
            project_id,
            branch_id,
            character_id,
            knowledge_fact_id,
            source_summary,
            confidence,
            source_import_session_id,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                let character_id = character_id.clone();
                let knowledge_fact_id = knowledge_fact_id.clone();
                move |conn| {
                    conn.execute(
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
                            &character_id,
                            &knowledge_fact_id,
                            &project_id,
                            &branch_id,
                            &source_summary,
                            &learned_at_json,
                            confidence,
                            reader_visible,
                            &source_import_session_id,
                            now,
                        ],
                    )?;
                    Ok(())
                }
            })
            .await?;
        self.get_knows(&branch_id, &character_id, &knowledge_fact_id)
            .await
    }

    pub async fn get_knows(
        &self,
        branch_id: &str,
        character_id: &str,
        knowledge_fact_id: &str,
    ) -> Result<Knows> {
        let branch_id = branch_id.to_string();
        let character_id = character_id.to_string();
        let knowledge_fact_id = knowledge_fact_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {KNOWS_COLUMNS} FROM knows \
                     WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&branch_id, &character_id, &knowledge_fact_id],
                    |r| Knows::try_from(r),
                )
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("knows edge not found"))
    }

    // =========================================================================
    // Import pipeline
    // =========================================================================

    pub async fn create_import_session(
        &self,
        params: CreateImportSessionParams,
    ) -> Result<ImportSession> {
        let id = mint_id("import_session");
        let id_out = id.clone();
        let progress_json = serde_json::to_string(&params.progress)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        let CreateImportSessionParams {
            project_id,
            target_branch_id,
            source_format,
            active_pass,
            session_status,
            hydrate_mode,
            source_count,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO import_session (id, project_id, target_branch_id, source_format, \
                     active_pass, progress, session_status, hydrate_mode, source_count, \
                     hydration_report, imported_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, ?10, ?10)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &target_branch_id,
                        &source_format,
                        &active_pass,
                        &progress_json,
                        &session_status,
                        &hydrate_mode,
                        source_count as i64,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_session(&id_out).await
    }

    pub async fn get_import_session(&self, id: &str) -> Result<ImportSession> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {IMPORT_SESSION_COLUMNS} FROM import_session WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportSession::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_session not found"))
    }

    pub async fn list_import_sessions_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ImportSession>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SESSION_COLUMNS} FROM import_session \
                     WHERE project_id = ?1 ORDER BY imported_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ImportSession::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn update_import_session_state(
        &self,
        session_id: &str,
        active_pass: &str,
        progress: Value,
        session_status: &str,
    ) -> Result<ImportSession> {
        let session_id_owned = session_id.to_string();
        let active_pass = active_pass.to_string();
        let progress_json = serde_json::to_string(&progress)?;
        let session_status = session_status.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE import_session SET active_pass = ?1, progress = ?2, \
                     session_status = ?3, updated_at = ?4 WHERE id = ?5",
                    rusqlite::params![
                        &active_pass,
                        &progress_json,
                        &session_status,
                        now,
                        &session_id_owned,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_session(session_id).await
    }

    pub async fn update_import_session_hydration_report(
        &self,
        session_id: &str,
        report: Value,
    ) -> Result<ImportSession> {
        let session_id_owned = session_id.to_string();
        let report_json = serde_json::to_string(&report)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE import_session SET hydration_report = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&report_json, now, &session_id_owned],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_session(session_id).await
    }

    pub async fn upsert_import_source_document(
        &self,
        params: UpsertImportSourceDocumentParams,
    ) -> Result<ImportSourceDocument> {
        let id = mint_id("import_source_document");
        let id_out = id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportSourceDocumentParams {
            session_id,
            project_id,
            display_name,
            source_path,
            copied_path,
            source_format,
            original_sha256,
            normalized_sha256,
            normalized_text_ref,
            word_count,
            chapter_hint,
            source_order,
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_source_document \
                         WHERE session_id = ?1 AND source_order = ?2",
                        rusqlite::params![&session_id, source_order as i64],
                    )?;
                    tx.execute(
                        "INSERT INTO import_source_document (id, session_id, project_id, \
                         display_name, source_path, copied_path, source_format, original_sha256, \
                         normalized_sha256, normalized_text_ref, word_count, chapter_hint, \
                         source_order, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &project_id,
                            &display_name,
                            &source_path,
                            &copied_path,
                            &source_format,
                            &original_sha256,
                            &normalized_sha256,
                            &normalized_text_ref,
                            word_count as i64,
                            &chapter_hint,
                            source_order as i64,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_source_document(&id_out).await
    }

    pub async fn get_import_source_document(&self, id: &str) -> Result<ImportSourceDocument> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SOURCE_DOCUMENT_COLUMNS} FROM import_source_document WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportSourceDocument::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_source_document not found"))
    }

    pub async fn list_import_source_documents(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportSourceDocument>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SOURCE_DOCUMENT_COLUMNS} FROM import_source_document \
                     WHERE session_id = ?1 ORDER BY source_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportSourceDocument::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn upsert_import_segment(
        &self,
        params: UpsertImportSegmentParams,
    ) -> Result<ImportSegment> {
        let id = mint_id("import_segment");
        let id_out = id.clone();
        let pov_guess_json = match params.pov_guess.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportSegmentParams {
            session_id,
            source_document_id,
            parent_segment_id,
            segment_type,
            source_order,
            book_number,
            chapter_number,
            scene_order,
            label,
            start_offset,
            end_offset,
            word_count,
            character_count,
            confidence,
            segment_status,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                let source_document_id = source_document_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_segment \
                         WHERE session_id = ?1 AND source_document_id = ?2 AND source_order = ?3",
                        rusqlite::params![&session_id, &source_document_id, source_order as i64],
                    )?;
                    tx.execute(
                        "INSERT INTO import_segment (id, session_id, source_document_id, \
                         parent_segment_id, segment_type, source_order, book_number, chapter_number, \
                         scene_order, label, start_offset, end_offset, word_count, character_count, \
                         pov_guess, confidence, segment_status, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, \
                                 ?16, ?17, ?18, ?18)",
                        rusqlite::params![
                            &id, &session_id, &source_document_id, &parent_segment_id,
                            &segment_type, source_order as i64,
                            book_number.map(i64::from), chapter_number.map(i64::from),
                            scene_order.map(i64::from), &label, start_offset as i64,
                            end_offset as i64, word_count as i64, character_count as i64,
                            &pov_guess_json, confidence, &segment_status, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_segment(&id_out).await
    }

    pub async fn get_import_segment(&self, id: &str) -> Result<ImportSegment> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {IMPORT_SEGMENT_COLUMNS} FROM import_segment WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportSegment::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_segment not found"))
    }

    pub async fn list_import_segments(&self, session_id: &str) -> Result<Vec<ImportSegment>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_SEGMENT_COLUMNS} FROM import_segment \
                     WHERE session_id = ?1 ORDER BY source_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportSegment::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn upsert_import_entity_cluster(
        &self,
        params: UpsertImportEntityClusterParams,
    ) -> Result<ImportEntityCluster> {
        let id = mint_id("import_entity_cluster");
        let id_out = id.clone();
        let aliases_json = serde_json::to_string(&params.aliases)?;
        let mention_ids_json = serde_json::to_string(&params.mention_ids)?;
        let notes_json = serde_json::to_string(&params.notes)?;
        let review_required = if params.review_required { 1 } else { 0 };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportEntityClusterParams {
            session_id,
            entity_kind,
            canonical_name,
            normalized_name,
            first_segment_id,
            last_segment_id,
            importance_rank,
            merge_confidence,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                let entity_kind = entity_kind.clone();
                let normalized_name = normalized_name.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_entity_cluster \
                         WHERE session_id = ?1 AND entity_kind = ?2 AND normalized_name = ?3",
                        rusqlite::params![&session_id, &entity_kind, &normalized_name],
                    )?;
                    tx.execute(
                        "INSERT INTO import_entity_cluster (id, session_id, entity_kind, \
                         canonical_name, normalized_name, aliases, mention_ids, first_segment_id, \
                         last_segment_id, importance_rank, merge_confidence, review_required, \
                         notes, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &entity_kind,
                            &canonical_name,
                            &normalized_name,
                            &aliases_json,
                            &mention_ids_json,
                            &first_segment_id,
                            &last_segment_id,
                            importance_rank,
                            merge_confidence,
                            review_required,
                            &notes_json,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_entity_cluster(&id_out).await
    }

    pub async fn get_import_entity_cluster(&self, id: &str) -> Result<ImportEntityCluster> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_CLUSTER_COLUMNS} FROM import_entity_cluster WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportEntityCluster::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_entity_cluster not found"))
    }

    pub async fn upsert_import_character_dossier(
        &self,
        params: UpsertImportCharacterDossierParams,
    ) -> Result<ImportCharacterDossier> {
        let id = mint_id("import_character_dossier");
        let id_out = id.clone();
        let aliases_json = serde_json::to_string(&params.aliases)?;
        let voice_json = serde_json::to_string(&params.voice_profile)?;
        let emo_json = serde_json::to_string(&params.emotional_profile)?;
        let state_json = serde_json::to_string(&params.state_trajectory)?;
        let relationship_json = serde_json::to_string(&params.relationship_inferences)?;
        let decision_json = serde_json::to_string(&params.decision_patterns)?;
        let dialogue_json = serde_json::to_string(&params.dialogue_samples)?;
        let review_required = if params.review_required { 1 } else { 0 };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertImportCharacterDossierParams {
            session_id,
            cluster_id,
            canonical_name,
            importance_rank,
            confidence,
            ..
        } = params;

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                let cluster_id = cluster_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_character_dossier WHERE session_id = ?1 AND cluster_id = ?2",
                        rusqlite::params![&session_id, &cluster_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_character_dossier (id, session_id, cluster_id, \
                         canonical_name, aliases, importance_rank, voice_profile, emotional_profile, \
                         state_trajectory, relationship_inferences, decision_patterns, dialogue_samples, \
                         confidence, review_required, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?15)",
                        rusqlite::params![
                            &id, &session_id, &cluster_id, &canonical_name, &aliases_json,
                            importance_rank, &voice_json, &emo_json, &state_json, &relationship_json,
                            &decision_json, &dialogue_json, confidence, review_required, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_character_dossier(&id_out).await
    }

    pub async fn get_import_character_dossier(&self, id: &str) -> Result<ImportCharacterDossier> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_CHARACTER_DOSSIER_COLUMNS} FROM import_character_dossier WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportCharacterDossier::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_character_dossier not found"))
    }

    pub async fn upsert_import_world_dossier(
        &self,
        params: UpsertImportWorldDossierParams,
    ) -> Result<ImportWorldDossier> {
        let id = mint_id("import_world_dossier");
        let id_out = id.clone();
        let rules_json = serde_json::to_string(&params.world_rules)?;
        let locations_json = serde_json::to_string(&params.locations)?;
        let entities_json = serde_json::to_string(&params.entities)?;
        let signals_json = serde_json::to_string(&params.system_signals)?;
        let session_id = params.session_id;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_world_dossier WHERE session_id = ?1",
                        [&session_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_world_dossier (id, session_id, world_rules, locations, \
                         entities, system_signals, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &rules_json,
                            &locations_json,
                            &entities_json,
                            &signals_json,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_world_dossier(&id_out).await
    }

    pub async fn get_import_world_dossier(&self, id: &str) -> Result<ImportWorldDossier> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_WORLD_DOSSIER_COLUMNS} FROM import_world_dossier WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportWorldDossier::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_world_dossier not found"))
    }

    pub async fn upsert_import_narrative_dossier(
        &self,
        params: UpsertImportNarrativeDossierParams,
    ) -> Result<ImportNarrativeDossier> {
        let id = mint_id("import_narrative_dossier");
        let id_out = id.clone();
        let plot_lines = serde_json::to_string(&params.plot_lines)?;
        let conflicts = serde_json::to_string(&params.conflicts)?;
        let promises = serde_json::to_string(&params.narrative_promises)?;
        let arcs = serde_json::to_string(&params.arcs)?;
        let themes = serde_json::to_string(&params.themes)?;
        let motifs = serde_json::to_string(&params.motifs)?;
        let reader_contract = serde_json::to_string(&params.reader_contract)?;
        let pacing_hints = serde_json::to_string(&params.pacing_hints)?;
        let session_id = params.session_id;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_narrative_dossier WHERE session_id = ?1",
                        [&session_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_narrative_dossier (id, session_id, plot_lines, \
                         conflicts, narrative_promises, arcs, themes, motifs, reader_contract, \
                         pacing_hints, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            &plot_lines,
                            &conflicts,
                            &promises,
                            &arcs,
                            &themes,
                            &motifs,
                            &reader_contract,
                            &pacing_hints,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_narrative_dossier(&id_out).await
    }

    pub async fn get_import_narrative_dossier(&self, id: &str) -> Result<ImportNarrativeDossier> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_NARRATIVE_DOSSIER_COLUMNS} FROM import_narrative_dossier WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportNarrativeDossier::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_narrative_dossier not found"))
    }

    pub async fn upsert_import_resume_snapshot(
        &self,
        params: UpsertImportResumeSnapshotParams,
    ) -> Result<ImportResumeSnapshot> {
        let id = mint_id("import_resume_snapshot");
        let id_out = id.clone();
        let characters = serde_json::to_string(&params.characters)?;
        let relationships = serde_json::to_string(&params.relationships)?;
        let locations = serde_json::to_string(&params.locations)?;
        let plot_threads = serde_json::to_string(&params.plot_threads)?;
        let session_id = params.session_id;
        let book_number = params.book_number;
        let chapter_number = params.chapter_number;
        let scene_order = params.scene_order;
        let summary = params.summary;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let session_id = session_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM import_resume_snapshot WHERE session_id = ?1",
                        [&session_id],
                    )?;
                    tx.execute(
                        "INSERT INTO import_resume_snapshot (id, session_id, book_number, \
                         chapter_number, scene_order, summary, characters, relationships, \
                         locations, plot_threads, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                        rusqlite::params![
                            &id,
                            &session_id,
                            book_number,
                            chapter_number,
                            scene_order.map(i64::from),
                            &summary,
                            &characters,
                            &relationships,
                            &locations,
                            &plot_threads,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_import_resume_snapshot(&id_out).await
    }

    pub async fn get_import_resume_snapshot(&self, id: &str) -> Result<ImportResumeSnapshot> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_RESUME_SNAPSHOT_COLUMNS} FROM import_resume_snapshot WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportResumeSnapshot::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_resume_snapshot not found"))
    }

    pub async fn create_import_review_item(
        &self,
        params: CreateImportReviewItemParams,
    ) -> Result<ImportReviewItem> {
        let id = mint_id("import_review_item");
        let id_out = id.clone();
        let related_segments = serde_json::to_string(&params.related_segment_ids)?;
        let related_entities = serde_json::to_string(&params.related_entity_ids)?;
        let proposed_correction = match params.proposed_correction.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let CreateImportReviewItemParams {
            session_id,
            pass_name,
            item_kind,
            severity,
            status,
            title,
            description,
            confidence,
            resolver_notes,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO import_review_item (id, session_id, pass_name, item_kind, \
                     severity, status, title, description, related_segment_ids, related_entity_ids, \
                     confidence, proposed_correction, resolver_notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?14)",
                    rusqlite::params![
                        &id, &session_id, &pass_name, &item_kind, &severity, &status, &title,
                        &description, &related_segments, &related_entities, confidence,
                        &proposed_correction, &resolver_notes, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_review_item(&id_out).await
    }

    pub async fn get_import_review_item(&self, id: &str) -> Result<ImportReviewItem> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportReviewItem::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_review_item not found"))
    }

    pub async fn list_import_review_items_by_status(
        &self,
        session_id: &str,
        status: &str,
    ) -> Result<Vec<ImportReviewItem>> {
        let session_id = session_id.to_string();
        let status = status.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item \
                     WHERE session_id = ?1 AND status = ?2 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&session_id, &status], |r| {
                        ImportReviewItem::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every review item for a session, ordered by `created_at`. The
    /// `_by_status` variant filters; this returns the full set the import
    /// service replays into its rendered summaries.
    pub async fn list_import_review_items(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportReviewItem>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportReviewItem::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Mark a review item resolved by status, optionally updating the
    /// proposed correction and resolver notes. Mirrors the SurrealDB-era
    /// `resolve_import_review_item` helper.
    pub async fn resolve_import_review_item(
        &self,
        id: &str,
        params: ResolveImportReviewItemParams,
    ) -> Result<ImportReviewItem> {
        let id_owned = id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let proposed_correction = match params.proposed_correction.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let ResolveImportReviewItemParams {
            status,
            resolver_notes,
            ..
        } = params;
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE import_review_item \
                     SET status = ?1, proposed_correction = ?2, resolver_notes = ?3, \
                         updated_at = ?4 \
                     WHERE id = ?5",
                    rusqlite::params![
                        &status,
                        &proposed_correction,
                        &resolver_notes,
                        now,
                        &id_owned,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_review_item(id).await
    }

    /// List every entity mention attached to a session. Used by the
    /// extract/consolidate passes to thread mentions through the cluster
    /// builder.
    pub async fn list_import_entity_mentions(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportEntityMention>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_MENTION_COLUMNS} FROM import_entity_mention \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportEntityMention::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every entity cluster persisted for a session.
    pub async fn list_import_entity_clusters(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportEntityCluster>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_CLUSTER_COLUMNS} FROM import_entity_cluster \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportEntityCluster::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every persisted character dossier for a session.
    pub async fn list_import_character_dossiers(
        &self,
        session_id: &str,
    ) -> Result<Vec<ImportCharacterDossier>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_CHARACTER_DOSSIER_COLUMNS} FROM import_character_dossier \
                     WHERE session_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&session_id], |r| ImportCharacterDossier::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Return the single world dossier for a session, if any.
    pub async fn find_import_world_dossier(
        &self,
        session_id: &str,
    ) -> Result<Option<ImportWorldDossier>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_WORLD_DOSSIER_COLUMNS} FROM import_world_dossier \
                     WHERE session_id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&session_id], |r| ImportWorldDossier::try_from(r))
                    .optional_inner()
            })
            .await
    }

    /// Return the single narrative dossier for a session, if any.
    pub async fn find_import_narrative_dossier(
        &self,
        session_id: &str,
    ) -> Result<Option<ImportNarrativeDossier>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_NARRATIVE_DOSSIER_COLUMNS} FROM import_narrative_dossier \
                     WHERE session_id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&session_id], |r| ImportNarrativeDossier::try_from(r))
                    .optional_inner()
            })
            .await
    }

    /// Return the single resume snapshot for a session, if any.
    pub async fn find_import_resume_snapshot(
        &self,
        session_id: &str,
    ) -> Result<Option<ImportResumeSnapshot>> {
        let session_id = session_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_RESUME_SNAPSHOT_COLUMNS} FROM import_resume_snapshot \
                     WHERE session_id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&session_id], |r| ImportResumeSnapshot::try_from(r))
                    .optional_inner()
            })
            .await
    }

    /// Return true when any existing import session has already ingested a
    /// source document with the given `original_sha256`. Used to honor the
    /// `Reject` duplicate-strategy on `import_manuscript`.
    pub async fn import_session_exists_for_source_hash(&self, hash: &str) -> Result<bool> {
        let hash = hash.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare_cached(
                    "SELECT 1 FROM import_source_document WHERE original_sha256 = ?1 LIMIT 1",
                )?;
                stmt.query_row([&hash], |_| Ok(())).optional_inner()
            })
            .await
            .map(|opt| opt.is_some())
    }

    /// Cheap idempotency check: was hydration for this session already
    /// completed against the requested project/branch? Returns `true` once
    /// `update_import_session_hydration_report` has stored a report that
    /// matches both ids.
    pub async fn hydration_target_exists(
        &self,
        session_id: &str,
        project_id: &str,
        branch_id: &str,
    ) -> Result<bool> {
        let session = self.get_import_session(session_id).await?;
        let Some(report) = session.hydration_report.as_ref() else {
            return Ok(false);
        };
        let report_project = report
            .get("project_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let report_branch = report
            .get("branch_id")
            .and_then(|v| v.as_str())
            .or_else(|| report.get("target_branch_id").and_then(|v| v.as_str()))
            .unwrap_or_default();
        Ok(report_project == project_id && report_branch == branch_id)
    }

    pub async fn create_import_entity_mention(
        &self,
        params: CreateImportEntityMentionParams,
    ) -> Result<ImportEntityMention> {
        let id = mint_id("import_entity_mention");
        let id_out = id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let CreateImportEntityMentionParams {
            session_id,
            segment_id,
            entity_kind,
            surface_form,
            normalized_name,
            alias_hint,
            surrounding_text,
            confidence,
            extraction_pass,
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO import_entity_mention (id, session_id, segment_id, entity_kind, \
                     surface_form, normalized_name, alias_hint, surrounding_text, confidence, \
                     extraction_pass, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        &id,
                        &session_id,
                        &segment_id,
                        &entity_kind,
                        &surface_form,
                        &normalized_name,
                        &alias_hint,
                        &surrounding_text,
                        confidence,
                        &extraction_pass,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_import_entity_mention(&id_out).await
    }

    pub async fn get_import_entity_mention(&self, id: &str) -> Result<ImportEntityMention> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_MENTION_COLUMNS} FROM import_entity_mention WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ImportEntityMention::try_from(r)).optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("import_entity_mention not found"))
    }

    // =========================================================================
    // Scene versions + scene source links + research log
    // =========================================================================

    pub async fn list_scene_versions(&self, scene_id: &str) -> Result<Vec<SceneVersion>> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_VERSION_COLUMNS} FROM scene_version \
                     WHERE scene_id = ?1 ORDER BY version_number DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&scene_id], |r| SceneVersion::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_scene_version(&self, scene_version_id: &str) -> Result<SceneVersion> {
        let id = scene_version_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {SCENE_VERSION_COLUMNS} FROM scene_version WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| SceneVersion::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("scene_version not found"))
    }

    /// Upsert a scene_source_link row for a scene. There's at most one link
    /// per scene; later calls update the existing row.
    pub async fn upsert_scene_source_link(
        &self,
        project_id: &str,
        scene_id: &str,
        source_path: &str,
        content_sha256: &str,
        source_start_offset: Option<i64>,
        source_end_offset: Option<i64>,
    ) -> Result<SceneSourceLink> {
        let id = mint_id("scene_source_link");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let scene_id_owned = scene_id.to_string();
        let source_path = source_path.to_string();
        let content_sha256 = content_sha256.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let scene_id = scene_id_owned.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM scene_source_link WHERE scene_id = ?1",
                        [&scene_id],
                    )?;
                    tx.execute(
                        "INSERT INTO scene_source_link (id, project_id, scene_id, source_path, \
                         content_sha256, source_start_offset, source_end_offset, linked_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                        rusqlite::params![
                            &id, &project_id, &scene_id, &source_path, &content_sha256,
                            source_start_offset, source_end_offset, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        let _ = id_out;
        self.get_scene_source_link_for_scene(scene_id)
            .await?
            .ok_or_else(|| anyhow!("scene_source_link vanished after upsert"))
    }

    pub async fn get_scene_source_link_for_scene(
        &self,
        scene_id: &str,
    ) -> Result<Option<SceneSourceLink>> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_SOURCE_LINK_COLUMNS} FROM scene_source_link \
                     WHERE scene_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&scene_id], |r| SceneSourceLink::try_from(r))
                    .optional_inner()
            })
            .await
    }

    pub async fn delete_scene_source_links_for_scene(&self, scene_id: &str) -> Result<()> {
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM scene_source_link WHERE scene_id = ?1",
                    [&scene_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn create_research_log(
        &self,
        project_id: &str,
        query: &str,
        context_hint: Option<&str>,
        model: &str,
        response: &str,
        context_summary: &str,
    ) -> Result<ResearchLog> {
        let id = mint_id("research_log");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let query_owned = query.to_string();
        let context_hint = context_hint.map(|s| s.to_string());
        let model_owned = model.to_string();
        let response_owned = response.to_string();
        let summary = context_summary.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO research_log (id, project_id, query, context_hint, model, \
                     response, context_summary, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &query_owned,
                        &context_hint,
                        &model_owned,
                        &response_owned,
                        &summary,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        let id_lookup = id_out;
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {RESEARCH_LOG_COLUMNS} FROM research_log WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| ResearchLog::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("research_log vanished after insert"))
    }

    pub async fn list_research_logs_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ResearchLog>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RESEARCH_LOG_COLUMNS} FROM research_log \
                     WHERE project_id = ?1 ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ResearchLog::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn update_promise_status(&self, promise_id: &str, status: &str) -> Result<()> {
        let promise_id = promise_id.to_string();
        let status = status.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute(
                    "UPDATE narrative_promise SET status = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![&status, now, &promise_id],
                )?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    // =========================================================================
    // Reviews + validator findings + progression events + scene beat annotations
    // =========================================================================

    pub async fn upsert_dual_persona_review(
        &self,
        params: UpsertDualPersonaReviewParams,
    ) -> Result<DualPersonaReview> {
        let stored_rounds: Vec<StoredDualPersonaReviewRound> = params
            .review_rounds
            .iter()
            .cloned()
            .map(Into::into)
            .collect();
        let rounds_json = serde_json::to_string(&serde_json::json!({ "rounds": stored_rounds }))?;
        let id = mint_id("dual_persona_review");
        let now = timestamp_to_micros(chrono::Utc::now());
        let project_id = params.project_id.clone();
        let branch_id = params.branch_id.clone();
        let scene_id = params.scene_id.clone();
        let fingerprint = params.scene_revision_fingerprint;
        let rounds_completed = params.rounds_completed as i64;
        let status = params.status;

        self.inner
            .pool
            .write({
                let branch_id = branch_id.clone();
                let scene_id = scene_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM dual_persona_review WHERE branch_id = ?1 AND scene_id = ?2",
                        rusqlite::params![&branch_id, &scene_id],
                    )?;
                    tx.execute(
                        "INSERT INTO dual_persona_review (id, project_id, branch_id, scene_id, \
                         scene_revision_fingerprint, rounds_completed, status, review_rounds, \
                         created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                        rusqlite::params![
                            &id,
                            &project_id,
                            &branch_id,
                            &scene_id,
                            &fingerprint,
                            rounds_completed,
                            &status,
                            &rounds_json,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_dual_persona_review(&branch_id, &scene_id)
            .await?
            .ok_or_else(|| anyhow!("dual_persona_review vanished after upsert"))
    }

    pub async fn get_dual_persona_review(
        &self,
        branch_id: &str,
        scene_id: &str,
    ) -> Result<Option<DualPersonaReview>> {
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {DUAL_PERSONA_REVIEW_COLUMNS} FROM dual_persona_review \
                     WHERE branch_id = ?1 AND scene_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&branch_id, &scene_id], |r| {
                    DualPersonaReview::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn upsert_validator_finding(
        &self,
        params: UpsertValidatorFindingParams,
    ) -> Result<ValidatorFinding> {
        let id = mint_id("validator_finding");
        let id_out = id.clone();
        let byte_range_json = match params.byte_range.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let details_json = match params.details_json.as_ref() {
            Some(v) => Some(serde_json::to_string(v)?),
            None => None,
        };
        let now = timestamp_to_micros(chrono::Utc::now());
        let UpsertValidatorFindingParams {
            project_id,
            branch_id,
            scene_id,
            scene_text_hash,
            context_hash,
            validator_id,
            finding_id,
            severity,
            message,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                // The SurrealDB version uses a non-unique index but treats
                // (branch, scene, validator, finding, hash) as the natural key.
                // We DELETE prior matching rows then INSERT for the same shape.
                let tx = conn.transaction()?;
                tx.execute(
                    "UPDATE validator_finding SET resolved_at = ?1 \
                     WHERE branch_id = ?2 AND scene_id = ?3 AND validator_id = ?4 \
                       AND finding_id = ?5 AND scene_text_hash = ?6 \
                       AND resolved_at IS NULL \
                       AND context_hash IS NOT ?7",
                    rusqlite::params![
                        now,
                        &branch_id,
                        &scene_id,
                        &validator_id,
                        &finding_id,
                        &scene_text_hash,
                        &context_hash,
                    ],
                )?;
                tx.execute(
                    "DELETE FROM validator_finding \
                     WHERE branch_id = ?1 AND scene_id = ?2 AND validator_id = ?3 \
                       AND finding_id = ?4 AND scene_text_hash = ?5 \
                       AND context_hash IS ?6",
                    rusqlite::params![
                        &branch_id,
                        &scene_id,
                        &validator_id,
                        &finding_id,
                        &scene_text_hash,
                        &context_hash,
                    ],
                )?;
                tx.execute(
                    "INSERT INTO validator_finding (id, project_id, branch_id, scene_id, \
                     scene_text_hash, validator_id, finding_id, severity, message, byte_range, \
                     details_json, created_at, resolved_at, context_hash) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &scene_id,
                        &scene_text_hash,
                        &validator_id,
                        &finding_id,
                        &severity,
                        &message,
                        &byte_range_json,
                        &details_json,
                        now,
                        &context_hash,
                    ],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.get_validator_finding(&id_out).await
    }

    pub async fn get_validator_finding(&self, id: &str) -> Result<ValidatorFinding> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ValidatorFinding::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("validator_finding not found"))
    }

    pub async fn list_open_validator_findings(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<ValidatorFinding>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding \
                     WHERE project_id = ?1 AND branch_id = ?2 AND resolved_at IS NULL \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        ValidatorFinding::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_validator_findings_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<ValidatorFinding>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        ValidatorFinding::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Cache lookup for the Phase-4 fan-out: returns every active
    /// validator_finding row for the given branch+scene+text-hash whose
    /// `validator_id` is in `validator_ids`. The service layer treats a
    /// hit on every requested validator id as a cache hit and skips
    /// re-running the registry for that scene.
    pub async fn list_active_validator_findings_by_scene_hash(
        &self,
        branch_id: &str,
        scene_id: &str,
        scene_text_hash: &str,
        validator_ids: &[String],
    ) -> Result<Vec<ValidatorFinding>> {
        if validator_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        let scene_text_hash = scene_text_hash.to_string();
        let validator_ids: Vec<String> = validator_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..validator_ids.len())
                    .map(|i| format!("?{}", i + 4))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {VALIDATOR_FINDING_COLUMNS} FROM validator_finding \
                     WHERE branch_id = ?1 AND scene_id = ?2 AND scene_text_hash = ?3 \
                       AND resolved_at IS NULL AND validator_id IN ({placeholders}) \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let mut params: Vec<&dyn rusqlite::ToSql> =
                    Vec::with_capacity(3 + validator_ids.len());
                params.push(&branch_id);
                params.push(&scene_id);
                params.push(&scene_text_hash);
                for v in &validator_ids {
                    params.push(v);
                }
                let rows = stmt
                    .query_map(params.as_slice(), |r| ValidatorFinding::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn append_progression_event(
        &self,
        params: AppendProgressionEventParams,
    ) -> Result<ProgressionEvent> {
        let id = mint_id("progression_event");
        let id_out = id.clone();
        let delta_json = serde_json::to_string(&params.delta_json)?;
        let placement_stored: Option<StoredStoryPlacement> = params.placement.map(Into::into);
        let placement_json = match placement_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let created_at = timestamp_to_micros(params.created_at);
        let AppendProgressionEventParams {
            project_id,
            branch_id,
            subject_table,
            subject_id,
            overlay_id,
            kind,
            source_scene_id,
            ..
        } = params;

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO progression_event (id, project_id, branch_id, subject_table, \
                     subject_id, overlay_id, kind, delta_json, source_scene_id, placement, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &subject_table, &subject_id, &overlay_id,
                        &kind, &delta_json, &source_scene_id, &placement_json, created_at,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_progression_event(&id_out).await
    }

    pub async fn get_progression_event(&self, id: &str) -> Result<ProgressionEvent> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PROGRESSION_EVENT_COLUMNS} FROM progression_event WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ProgressionEvent::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("progression_event not found"))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn annotate_scene_beats(
        &self,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        motif_ids: Vec<String>,
        theme_ids: Vec<String>,
        conflict_ids: Vec<String>,
        beats: Vec<AnnotatedBeat>,
    ) -> Result<SceneBeatAnnotation> {
        let stored_beats: Vec<StoredAnnotatedBeat> = beats.into_iter().map(Into::into).collect();
        let beats_json = serde_json::to_string(&stored_beats)?;
        let motif_json = serde_json::to_string(&motif_ids)?;
        let theme_json = serde_json::to_string(&theme_ids)?;
        let conflict_json = serde_json::to_string(&conflict_ids)?;
        let id = mint_id("scene_beat_annotation");
        let now = timestamp_to_micros(chrono::Utc::now());
        let project_id_owned = project_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let scene_id_owned = scene_id.to_string();

        self.inner
            .pool
            .write({
                let branch_id = branch_id_owned.clone();
                let scene_id = scene_id_owned.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM scene_beat_annotation \
                         WHERE branch_id = ?1 AND scene_id = ?2",
                        rusqlite::params![&branch_id, &scene_id],
                    )?;
                    tx.execute(
                        "INSERT INTO scene_beat_annotation (id, project_id, branch_id, scene_id, \
                         beats, motif_ids, theme_ids, conflict_ids, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
                        rusqlite::params![
                            &id,
                            &project_id_owned,
                            &branch_id,
                            &scene_id,
                            &beats_json,
                            &motif_json,
                            &theme_json,
                            &conflict_json,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_scene_beat_annotation(branch_id, scene_id)
            .await?
            .ok_or_else(|| anyhow!("scene_beat_annotation vanished after upsert"))
    }

    pub async fn get_scene_beat_annotation(
        &self,
        branch_id: &str,
        scene_id: &str,
    ) -> Result<Option<SceneBeatAnnotation>> {
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_BEAT_ANNOTATION_COLUMNS} FROM scene_beat_annotation \
                     WHERE branch_id = ?1 AND scene_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&branch_id, &scene_id], |r| {
                    SceneBeatAnnotation::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    // =========================================================================
    // Pacing: config, curve, tracker
    // =========================================================================

    pub async fn create_pacing_config(
        &self,
        input: &CreatePacingConfigInput,
    ) -> Result<PacingConfig> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("pacing_config");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let total_planned_books = input.total_planned_books;
        let avg_chapters = input.avg_chapters_per_book;
        let avg_scenes = input.avg_scenes_per_chapter;
        let tension_model = input.tension_model.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO pacing_config (id, project_id, branch_id, total_planned_books, \
                     avg_chapters_per_book, avg_scenes_per_chapter, tension_model, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8) \
                     ON CONFLICT (project_id, branch_id) DO UPDATE SET \
                        total_planned_books = excluded.total_planned_books, \
                        avg_chapters_per_book = excluded.avg_chapters_per_book, \
                        avg_scenes_per_chapter = excluded.avg_scenes_per_chapter, \
                        tension_model = excluded.tension_model, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id, &project_id, &branch_id, total_planned_books, avg_chapters,
                        avg_scenes, &tension_model, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_pacing_config(&id_out).await
    }

    pub async fn get_pacing_config(&self, id: &str) -> Result<PacingConfig> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {PACING_CONFIG_COLUMNS} FROM pacing_config WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PacingConfig::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_config not found"))
    }

    pub async fn create_pacing_curve(&self, input: &CreatePacingCurveInput) -> Result<PacingCurve> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("pacing_curve");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let book_number = input.book_number;
        let act_breakpoints_json = serde_json::to_string(&input.act_breakpoints)?;
        let scene_density_json = serde_json::to_string(&input.scene_type_density)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO pacing_curve (id, project_id, branch_id, book_number, \
                     act_breakpoints, scene_type_density, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7) \
                     ON CONFLICT (project_id, branch_id, book_number) DO UPDATE SET \
                        act_breakpoints = excluded.act_breakpoints, \
                        scene_type_density = excluded.scene_type_density, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        book_number,
                        &act_breakpoints_json,
                        &scene_density_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_pacing_curve(&id_out).await
    }

    pub async fn get_pacing_curve(&self, id: &str) -> Result<PacingCurve> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {PACING_CURVE_COLUMNS} FROM pacing_curve WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PacingCurve::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_curve not found"))
    }

    pub async fn create_pacing_tracker(
        &self,
        project_id: &str,
        character_arc_id: &str,
    ) -> Result<PacingTracker> {
        let branch_id = self.active_branch_id(project_id).await?;
        let id = mint_id("pacing_tracker");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let character_arc_id = character_arc_id.to_string();
        let empty_budget_json = "{}".to_string();
        let empty_warnings_json = "[]".to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO pacing_tracker (id, project_id, branch_id, character_arc_id, \
                     per_book_budget, max_progress_per_chapter, milestone_spacing, sprint_allowance, \
                     regression_budget, current_progress, budget_remaining, velocity, status, \
                     next_milestone, warnings, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, 0.0, 1.0, 'normal', \
                             'on_track', NULL, ?6, ?7)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &character_arc_id, &empty_budget_json,
                        &empty_warnings_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_pacing_tracker(&id_out).await
    }

    /// Look up a pacing tracker by its character_arc_id (1:1 relationship).
    pub async fn get_pacing_tracker_by_arc(&self, character_arc_id: &str) -> Result<PacingTracker> {
        let character_arc_id = character_arc_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_TRACKER_COLUMNS} FROM pacing_tracker \
                     WHERE character_arc_id = ?1 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&character_arc_id], |r| PacingTracker::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_tracker not found"))
    }

    pub async fn get_pacing_tracker(&self, id: &str) -> Result<PacingTracker> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {PACING_TRACKER_COLUMNS} FROM pacing_tracker WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| PacingTracker::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("pacing_tracker not found"))
    }

    /// Update the pacing-constraint fields on a pacing_tracker. Used by
    /// the set_arc_pacing_constraints service tool.
    pub async fn set_arc_pacing_constraints(
        &self,
        pacing_tracker_id: &str,
        per_book_budget: std::collections::BTreeMap<String, f64>,
        max_progress_per_chapter: Option<f64>,
        milestone_spacing: Option<i32>,
        sprint_allowance: Option<i32>,
        regression_budget: Option<f64>,
    ) -> Result<PacingTracker> {
        let id = pacing_tracker_id.to_string();
        let id_out = id.clone();
        let per_book_budget_json = serde_json::to_string(&per_book_budget)?;
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "UPDATE pacing_tracker SET per_book_budget = ?1, \
                     max_progress_per_chapter = ?2, milestone_spacing = ?3, \
                     sprint_allowance = ?4, regression_budget = ?5, updated_at = ?6 \
                     WHERE id = ?7",
                    rusqlite::params![
                        &per_book_budget_json,
                        max_progress_per_chapter,
                        milestone_spacing,
                        sprint_allowance,
                        regression_budget,
                        now,
                        &id,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_pacing_tracker(&id_out).await
    }

    pub async fn list_pacing_trackers_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<PacingTracker>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_TRACKER_COLUMNS} FROM pacing_tracker \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        PacingTracker::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every pacing_config row for a project, across branches. The
    /// import-hydration pass uses this to decide whether to create a fresh
    /// config when seeding pacing.
    pub async fn list_pacing_configs_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<PacingConfig>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_CONFIG_COLUMNS} FROM pacing_config \
                     WHERE project_id = ?1 ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PacingConfig::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every pacing_curve row for a project, across branches.
    pub async fn list_pacing_curves_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<PacingCurve>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_CURVE_COLUMNS} FROM pacing_curve \
                     WHERE project_id = ?1 ORDER BY book_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PacingCurve::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every knowledge_fact row attached to a project, across
    /// branches. Mirrors the existing `_by_project_and_branch` variant; the
    /// import-hydration pass uses the broader view when seeding canon.
    pub async fn list_knowledge_facts_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<KnowledgeFact>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| KnowledgeFact::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Future knowledge
    // =========================================================================

    pub async fn create_future_knowledge(
        &self,
        input: &CreateFutureKnowledgeInput,
    ) -> Result<FutureKnowledge> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let id = mint_id("future_knowledge");
        let id_out = id.clone();
        let project_id = input.project_id.clone();
        let character_id = input.character_id.clone();
        let summary = input.knowledge_summary.clone();
        let source = input.source.clone();
        let learned_at: StoredStoryPlacement = input.learned_at.clone().into();
        let learned_at_json = serde_json::to_string(&learned_at)?;
        let expires_at_stored: Option<StoredStoryPlacement> =
            input.expires_at.clone().map(Into::into);
        let expires_json = match expires_at_stored.as_ref() {
            Some(p) => Some(serde_json::to_string(p)?),
            None => None,
        };
        let notes_json = serde_json::to_string(&input.notes)?;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO future_knowledge (id, project_id, branch_id, character_id, \
                     knowledge_summary, source, learned_at, expires_at, notes, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![
                        &id, &project_id, &branch_id, &character_id, &summary, &source,
                        &learned_at_json, &expires_json, &notes_json, now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_future_knowledge(&id_out).await
    }

    pub async fn get_future_knowledge(&self, id: &str) -> Result<FutureKnowledge> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {FUTURE_KNOWLEDGE_COLUMNS} FROM future_knowledge WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| FutureKnowledge::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("future_knowledge not found"))
    }

    // =========================================================================
    // Chapter summary (save_summary)
    // =========================================================================

    pub async fn save_summary(&self, input: &SaveSummaryInput) -> Result<ChapterSummary> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let project_id = input.project_id.clone();
        let book_number = input.book_number;
        let chapter_number = input.chapter_number;
        let summary = input.summary.clone();
        let key_events_json = serde_json::to_string(&input.key_events)?;
        let character_changes_json = serde_json::to_string(&input.character_changes)?;
        let relationship_shifts_json = serde_json::to_string(&input.relationship_shifts)?;
        let arc_advances_json = serde_json::to_string(&input.arc_advances)?;
        let promise_events_json = serde_json::to_string(&input.promise_events)?;
        let id = mint_id("chapter_summary");
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    tx.execute(
                        "DELETE FROM chapter_summary \
                         WHERE project_id = ?1 AND branch_id = ?2 \
                           AND book_number = ?3 AND chapter_number = ?4",
                        rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                    )?;
                    tx.execute(
                        "INSERT INTO chapter_summary (id, project_id, branch_id, book_number, \
                         chapter_number, summary, key_events, character_changes, relationship_shifts, \
                         arc_advances, promise_events, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                        rusqlite::params![
                            &id, &project_id, &branch_id, book_number, chapter_number, &summary,
                            &key_events_json, &character_changes_json, &relationship_shifts_json,
                            &arc_advances_json, &promise_events_json, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_chapter_summary(&project_id, &branch_id, book_number, chapter_number)
            .await?
            .ok_or_else(|| anyhow!("chapter_summary vanished after save"))
    }

    pub async fn get_chapter_summary(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<Option<ChapterSummary>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_SUMMARY_COLUMNS} FROM chapter_summary \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                    |r| ChapterSummary::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    // =========================================================================
    // Book / chapter outlines (one row per (target, branch) pair)
    // =========================================================================

    // =========================================================================
    // Chapter planning
    // =========================================================================

    /// Plan a chapter: ensure book + chapter exist by number, then upsert
    /// the chapter_plan row and the chapter_outline that mirrors it. The
    /// SurrealDB version did all of this in one multi-statement BEGIN/COMMIT;
    /// the SQLite version uses a single writer transaction for the same
    /// atomicity. Returns the persisted ChapterPlan.
    pub async fn plan_chapter(&self, input: &PlanChapterInput) -> Result<ChapterPlan> {
        let branch_id = self.active_branch_id(&input.project_id).await?;
        let chapter = self
            .ensure_chapter(&input.project_id, input.book_number, input.chapter_number)
            .await?;

        let project_id = input.project_id.clone();
        let book_number = input.book_number;
        let chapter_number = input.chapter_number;
        let pov_character_id = input.pov_character_id.clone();
        let synopsis = input.synopsis.clone();
        let target_theme_ids_json = serde_json::to_string(&input.target_theme_ids)?;
        let target_conflict_ids_json = serde_json::to_string(&input.target_conflict_ids)?;
        let target_plot_line_ids_json = serde_json::to_string(&input.target_plot_line_ids)?;

        // chapter_plan.scenes is a JSON array of PlannedScene shapes.
        let planned_scenes: Vec<PlannedScene> = input
            .scenes
            .iter()
            .map(|s: &PlanChapterSceneInput| PlannedScene {
                scene_order: s.scene_order,
                summary: s.summary.clone(),
                beat_structure: s.beat_structure.clone(),
                character_ids: s.character_ids.clone(),
                purpose: s.purpose.clone(),
            })
            .collect();
        let scenes_json = serde_json::to_string(&planned_scenes)?;

        // chapter_outline mirrors the plan as ChapterOutlineBeat[] for the
        // outline view. Same shape every time so we synthesize from scenes.
        let outline_beats: Vec<ChapterOutlineBeat> = input
            .scenes
            .iter()
            .map(|s| ChapterOutlineBeat {
                order: s.scene_order,
                summary: s.summary.clone(),
                scene_id: None,
                status: "planned".to_string(),
            })
            .collect();
        let outline_content = serde_json::to_string(&outline_beats)?;

        let plan_id = mint_id("chapter_plan");
        let outline_id = mint_id("chapter_outline");
        let plan_id_lookup = plan_id.clone();
        let chapter_id = chapter.id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        // The outline goes through set_chapter_outline (which already
        // implements the proper INSERT ON CONFLICT), then we do the plan
        // INSERT in a separate transaction. Two writes is fine — both target
        // the writer connection and SQLite gives us per-statement durability.
        self.inner
            .pool
            .write({
                let project_id = project_id.clone();
                let branch_id = branch_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    // Upsert the chapter_plan row keyed by (project, branch,
                    // book#, chapter#) — the schema's UNIQUE index.
                    tx.execute(
                        "DELETE FROM chapter_plan \
                         WHERE project_id = ?1 AND branch_id = ?2 \
                           AND book_number = ?3 AND chapter_number = ?4",
                        rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                    )?;
                    tx.execute(
                        "INSERT INTO chapter_plan (id, project_id, branch_id, book_number, \
                         chapter_number, pov_character_id, synopsis, target_theme_ids, \
                         target_conflict_ids, target_plot_line_ids, scenes, created_at, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                        rusqlite::params![
                            &plan_id, &project_id, &branch_id, book_number, chapter_number,
                            &pov_character_id, &synopsis, &target_theme_ids_json,
                            &target_conflict_ids_json, &target_plot_line_ids_json, &scenes_json,
                            now,
                        ],
                    )?;
                    // Upsert the chapter_outline for the same (chapter, branch).
                    tx.execute(
                        "INSERT INTO chapter_outline (id, chapter_id, branch_id, format, content, beats, updated_at) \
                         VALUES (?1, ?2, ?3, 'json', ?4, ?4, ?5) \
                         ON CONFLICT (chapter_id, branch_id) DO UPDATE SET \
                            format = excluded.format, \
                            content = excluded.content, \
                            beats = excluded.beats, \
                            updated_at = excluded.updated_at",
                        rusqlite::params![
                            &outline_id, &chapter_id, &branch_id, &outline_content, now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_chapter_plan(&plan_id_lookup).await
    }

    /// Active-branch list of every chapter_plan in a project, ordered by
    /// (book#, chapter#).
    pub async fn list_chapter_plans_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ChapterPlan>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_PLAN_COLUMNS} FROM chapter_plan \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        ChapterPlan::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn delete_chapter_plans_for_chapter(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM chapter_plan \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4",
                    rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_chapter_summaries_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<ChapterSummary>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_SUMMARY_COLUMNS} FROM chapter_summary \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        ChapterSummary::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn delete_chapter_summaries_for_chapter(
        &self,
        project_id: &str,
        branch_id: &str,
        book_number: i32,
        chapter_number: i32,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM chapter_summary \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                       AND book_number = ?3 AND chapter_number = ?4",
                    rusqlite::params![&project_id, &branch_id, book_number, chapter_number],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_dual_persona_reviews_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<DualPersonaReview>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {DUAL_PERSONA_REVIEW_COLUMNS} FROM dual_persona_review \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        DualPersonaReview::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_future_knowledge_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<FutureKnowledge>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_future_knowledge_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Explicit-branch variant for the Phase-4 retcon scanner, which needs
    /// to merge active-branch + main-branch rows when the project is on a
    /// non-main working branch.
    pub async fn list_future_knowledge_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<FutureKnowledge>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {FUTURE_KNOWLEDGE_COLUMNS} FROM future_knowledge \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY character_id, created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        FutureKnowledge::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_temporal_interventions_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<TemporalIntervention>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_temporal_interventions_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// Explicit-branch variant for the Phase-4 retcon scanner, which needs
    /// to merge active-branch + main-branch rows when the project is on a
    /// non-main working branch.
    pub async fn list_temporal_interventions_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<TemporalIntervention>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TEMPORAL_INTERVENTION_COLUMNS} FROM temporal_intervention \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY title"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        TemporalIntervention::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_scene_beat_annotations_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<SceneBeatAnnotation>> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_BEAT_ANNOTATION_COLUMNS} FROM scene_beat_annotation \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY scene_id"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        SceneBeatAnnotation::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_scene_source_links_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<SceneSourceLink>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_SOURCE_LINK_COLUMNS} FROM scene_source_link \
                     WHERE project_id = ?1 ORDER BY linked_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SceneSourceLink::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// All relates_to rows involving any of the given characters (either
    /// in_id or out_id), within a single branch.
    pub async fn list_relationships_for_characters(
        &self,
        branch_id: &str,
        character_ids: &[String],
    ) -> Result<Vec<RelatesTo>> {
        if character_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_id = branch_id.to_string();
        let character_ids: Vec<String> = character_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..character_ids.len())
                    .map(|i| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id = ?1 \
                       AND (in_id IN ({placeholders}) OR out_id IN ({placeholders})) \
                     ORDER BY relationship_type"
                );
                let mut params: Vec<&dyn rusqlite::ToSql> =
                    Vec::with_capacity(character_ids.len() + 1);
                params.push(&branch_id);
                for id in &character_ids {
                    params.push(id);
                }
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(&params[..], |r| RelatesTo::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Resolve the most recent character_state for (character, scene) at or
    /// before that scene's narrative position on a specific branch. Returns
    /// None if no state has been committed.
    pub async fn resolve_character_state_for_branch(
        &self,
        branch_id: &str,
        character_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<CharacterState>> {
        let branch_id = branch_id.to_string();
        let character_id = character_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE branch_id = ?1 AND character_id = ?2 \
                       AND ( \
                            book_number < ?3 \
                         OR (book_number = ?3 AND chapter_number < ?4) \
                         OR (book_number = ?3 AND chapter_number = ?4 AND scene_order <= ?5) \
                       ) \
                     ORDER BY book_number DESC, chapter_number DESC, scene_order DESC \
                     LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![
                        &branch_id,
                        &character_id,
                        book_number,
                        chapter_number,
                        scene_order
                    ],
                    |r| CharacterState::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    /// World state lookup that resolves through the project's active branch.
    /// Mirrors the SurrealDB repo's get_world_state_by_location convenience.
    pub async fn get_world_state_by_location(
        &self,
        location_id: &str,
    ) -> Result<Option<WorldState>> {
        let location = self.get_location(location_id).await?;
        let branch_id = self.active_branch_id(&location.project_id).await?;
        self.get_world_state_for_location(&branch_id, location_id)
            .await
    }

    /// Trigger an embedding refresh for one entity. The repository's role is
    /// scoped to the storage layer — the actual SearchDocument construction
    /// (which fields to index, how to summarize, etc.) lives at the service
    /// layer. This method is the entry point services call after mutating
    /// an entity; they pass a freshly-built SearchDocument.
    pub async fn refresh_search_embedding_for_entity(
        &self,
        project_id: &str,
        branch_id: &str,
        entity_id: &str,
        document: &SearchDocument,
    ) -> Result<()> {
        self.upsert_search_embedding_document(project_id, branch_id, entity_id, document)
            .await
    }

    pub async fn get_chapter_plan(&self, id: &str) -> Result<ChapterPlan> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {CHAPTER_PLAN_COLUMNS} FROM chapter_plan WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| ChapterPlan::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("chapter_plan not found"))
    }

    pub async fn get_book_outline(
        &self,
        book_id: &str,
        branch_id: &str,
    ) -> Result<Option<BookOutline>> {
        let book_id = book_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {BOOK_OUTLINE_COLUMNS} FROM book_outline \
                     WHERE book_id = ?1 AND branch_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&book_id, &branch_id], |r| {
                    BookOutline::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn set_book_outline(
        &self,
        book_id: &str,
        branch_id: &str,
        format: &str,
        content: &str,
    ) -> Result<BookOutline> {
        let book_id_owned = book_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let format = format.to_string();
        let content = content.to_string();
        let new_id = mint_id("book_outline");
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO book_outline (id, book_id, branch_id, format, content, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                     ON CONFLICT (book_id, branch_id) DO UPDATE SET \
                        format = excluded.format, \
                        content = excluded.content, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![&new_id, &book_id_owned, &branch_id_owned, &format, &content, now],
                )?;
                Ok(())
            })
            .await?;
        self.get_book_outline(book_id, branch_id)
            .await?
            .ok_or_else(|| anyhow!("book_outline vanished after upsert"))
    }

    pub async fn get_chapter_outline(
        &self,
        chapter_id: &str,
        branch_id: &str,
    ) -> Result<Option<ChapterOutline>> {
        let chapter_id = chapter_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_OUTLINE_COLUMNS} FROM chapter_outline \
                     WHERE chapter_id = ?1 AND branch_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&chapter_id, &branch_id], |r| {
                    ChapterOutline::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn set_chapter_outline(
        &self,
        chapter_id: &str,
        branch_id: &str,
        format: &str,
        content: &str,
        beats: Vec<ChapterOutlineBeat>,
    ) -> Result<ChapterOutline> {
        let chapter_id_owned = chapter_id.to_string();
        let branch_id_owned = branch_id.to_string();
        let format = format.to_string();
        let content = content.to_string();
        let stored_beats: Vec<StoredChapterOutlineBeat> =
            beats.into_iter().map(Into::into).collect();
        let beats_json = serde_json::to_string(&stored_beats)?;
        let new_id = mint_id("chapter_outline");
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO chapter_outline (id, chapter_id, branch_id, format, content, beats, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT (chapter_id, branch_id) DO UPDATE SET \
                        format = excluded.format, \
                        content = excluded.content, \
                        beats = excluded.beats, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &new_id,
                        &chapter_id_owned,
                        &branch_id_owned,
                        &format,
                        &content,
                        &beats_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_chapter_outline(chapter_id, branch_id)
            .await?
            .ok_or_else(|| anyhow!("chapter_outline vanished after upsert"))
    }

    // =========================================================================
    // Search embeddings
    // =========================================================================
    //
    // The full search flow (kNN + FTS5) lands in Phase 5 once the vec0
    // virtual table is wired. Phase 4 here covers the write path that Phase 5
    // will consume: upsert_search_embedding_document does the embed-and-store,
    // delete_search_embedding_row removes a row, list returns rows for a
    // (project, branch) pair so the index can be rebuilt.

    pub async fn upsert_search_embedding_document(
        &self,
        project_id: &str,
        branch_id: &str,
        entity_id: &str,
        document: &SearchDocument,
    ) -> Result<()> {
        let embedding_session = self.inner.model_router.embedding_session();
        let embedding = embedding_session.embed_text(&document.content).await?;
        let embedding_blob = pack_embedding(&embedding);
        let version = self.inner.model_router.embedding_version();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let entity_id_owned = entity_id.to_string();
        let entity_table = document.entity_table.clone();
        let title = document.title.clone();
        let excerpt = document.excerpt.clone();
        let content = document.content.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let new_id = mint_id("search_embedding");

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO search_embedding (id, project_id, branch_id, entity_table, \
                     entity_id, title, excerpt, content, embedding_version, embedding, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                     ON CONFLICT (project_id, branch_id, entity_table, entity_id) DO UPDATE SET \
                        title = excluded.title, \
                        excerpt = excluded.excerpt, \
                        content = excluded.content, \
                        embedding_version = excluded.embedding_version, \
                        embedding = excluded.embedding, \
                        updated_at = excluded.updated_at",
                    rusqlite::params![
                        &new_id,
                        &project_id,
                        &branch_id,
                        &entity_table,
                        &entity_id_owned,
                        &title,
                        &excerpt,
                        &content,
                        &version,
                        &embedding_blob,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn list_search_embeddings_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SearchEmbedding>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SEARCH_EMBEDDING_COLUMNS} FROM search_embedding \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY entity_table, entity_id"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        SearchEmbedding::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// Phase 5 lexical search across scene prose using FTS5.
    /// Returns `(scene_id, rank, snippet)` tuples ordered by relevance.
    /// `rank` is FTS5's bm25-style score (lower = better match); `snippet` is
    /// a 32-token preview of the matching prose with `<mark>` tags around the
    /// matched terms.
    #[allow(clippy::type_complexity)]
    pub async fn fts_search_scenes(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.map(|s| s.to_string());
        let query = query.to_string();
        let limit = limit as i64;
        self.inner
            .pool
            .read(move |conn| {
                let (sql, has_branch) = if branch_id.is_some() {
                    (
                        "SELECT scene_id, rank, snippet(fts_scene, 4, '<mark>', '</mark>', '…', 32) \
                         FROM fts_scene \
                         WHERE fts_scene MATCH ?1 AND project_id = ?2 AND branch_id = ?3 \
                         ORDER BY rank LIMIT ?4",
                        true,
                    )
                } else {
                    (
                        "SELECT scene_id, rank, snippet(fts_scene, 4, '<mark>', '</mark>', '…', 32) \
                         FROM fts_scene \
                         WHERE fts_scene MATCH ?1 AND project_id = ?2 \
                         ORDER BY rank LIMIT ?3",
                        false,
                    )
                };
                let mut stmt = conn.prepare_cached(sql)?;
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

    /// Lexical search across characters. Returns `(character_id, rank, snippet)`.
    pub async fn fts_search_characters(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        fts_search_named(
            &self.inner.pool,
            "fts_character",
            "character_id",
            -1, // any matched column
            project_id,
            branch_id,
            query,
            limit,
        )
        .await
    }

    /// Lexical search across locations.
    pub async fn fts_search_locations(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        fts_search_named(
            &self.inner.pool,
            "fts_location",
            "location_id",
            -1,
            project_id,
            branch_id,
            query,
            limit,
        )
        .await
    }

    /// Lexical search across world rules.
    pub async fn fts_search_world_rules(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(String, f64, String)>> {
        fts_search_named(
            &self.inner.pool,
            "fts_world_rule",
            "world_rule_id",
            -1,
            project_id,
            branch_id,
            query,
            limit,
        )
        .await
    }

    /// k-nearest-neighbor search over the vec0 mirror of search_embedding.
    /// Returns `(SearchEmbedding, distance)` pairs ordered by distance.
    pub async fn knn_search_embeddings(
        &self,
        project_id: &str,
        branch_id: Option<&str>,
        query_embedding: &[f64],
        k: usize,
    ) -> Result<Vec<(SearchEmbedding, f64)>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.map(|s| s.to_string());
        let query_blob = pack_embedding(query_embedding);
        let k = k as i64;
        self.inner
            .pool
            .read(move |conn| {
                // se.* explicitly to avoid `embedding` name collision with the
                // vec0 table's own embedding column.
                let qualified_cols = SEARCH_EMBEDDING_COLUMNS
                    .split(',')
                    .map(|c| format!("se.{}", c.trim()))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut sql = format!(
                    "SELECT {qualified_cols}, vec.distance \
                     FROM vec_search_embedding AS vec \
                     JOIN search_embedding AS se ON se.id = vec.se_id \
                     WHERE vec.embedding MATCH ?1 AND vec.k = ?2 \
                       AND se.project_id = ?3"
                );
                if branch_id.is_some() {
                    sql.push_str(" AND se.branch_id = ?4");
                }
                sql.push_str(" ORDER BY vec.distance");
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = if let Some(b) = &branch_id {
                    stmt.query_map(rusqlite::params![&query_blob, k, &project_id, b], |r| {
                        let se = SearchEmbedding::try_from(r)?;
                        let dist: f64 = r.get(11)?;
                        Ok((se, dist))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                } else {
                    stmt.query_map(rusqlite::params![&query_blob, k, &project_id], |r| {
                        let se = SearchEmbedding::try_from(r)?;
                        let dist: f64 = r.get(11)?;
                        Ok((se, dist))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
                };
                Ok(rows)
            })
            .await
    }

    pub async fn delete_search_embedding_for_entity(
        &self,
        project_id: &str,
        entity_id: &str,
    ) -> Result<bool> {
        let project_id = project_id.to_string();
        let entity_id = entity_id.to_string();
        let n: usize = self
            .inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM search_embedding WHERE project_id = ?1 AND entity_id = ?2",
                    rusqlite::params![&project_id, &entity_id],
                )
            })
            .await?;
        Ok(n > 0)
    }

    pub async fn delete_search_embeddings_for_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM search_embedding WHERE project_id = ?1 AND branch_id = ?2",
                    rusqlite::params![&project_id, &branch_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // =========================================================================
    // Relationships (relates_to edge table)
    // =========================================================================
    //
    // Public API break vs SurrealDB repository: the schema-side relates_to
    // table has no surrogate id — it's keyed by composite PK
    // (branch_id, in_id, out_id). So update_relationship takes the character
    // pair + branch instead of a relationship_id RecordId.

    /// Create a relationship (or no-op-update if it already exists in either
    /// direction). Matches the SurrealDB create_relationship semantics:
    /// look up by (a→b) OR (b→a); if found, update its type/trust/tension;
    /// else insert a→b.
    pub async fn create_relationship(
        &self,
        branch_id: &str,
        input: &CreateRelationshipInput,
    ) -> Result<RelatesTo> {
        let branch_id_owned = branch_id.to_string();
        let a = input.character_a_id.clone();
        let b = input.character_b_id.clone();
        let rel_type = input.relationship_type.clone();
        let trust = input.initial_trust;
        let tension = input.initial_tension;
        let dynamics_json =
            serde_json::to_string(&input.dynamics).context("serializing dynamics")?;
        let now = timestamp_to_micros(chrono::Utc::now());

        let (in_id, out_id) = self
            .inner
            .pool
            .write({
                let branch_id_owned = branch_id_owned.clone();
                let a = a.clone();
                let b = b.clone();
                let rel_type = rel_type.clone();
                let dynamics_json = dynamics_json.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    // Look for an existing row in either direction.
                    let mut find = tx.prepare_cached(
                        "SELECT in_id, out_id FROM relates_to \
                         WHERE branch_id = ?1 \
                           AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                         LIMIT 1",
                    )?;
                    let mut rows = find.query(rusqlite::params![&branch_id_owned, &a, &b])?;
                    let existing: Option<(String, String)> = if let Some(row) = rows.next()? {
                        Some((row.get(0)?, row.get(1)?))
                    } else {
                        None
                    };
                    drop(rows);
                    drop(find);

                    let (in_id, out_id) = if let Some((existing_in, existing_out)) = existing {
                        tx.execute(
                            "UPDATE relates_to SET relationship_type = ?1, trust = ?2, \
                             tension = ?3, dynamics = ?4, updated_at = ?5 \
                             WHERE branch_id = ?6 AND in_id = ?7 AND out_id = ?8",
                            rusqlite::params![
                                &rel_type,
                                trust,
                                tension,
                                &dynamics_json,
                                now,
                                &branch_id_owned,
                                &existing_in,
                                &existing_out,
                            ],
                        )?;
                        (existing_in, existing_out)
                    } else {
                        tx.execute(
                            "INSERT INTO relates_to (in_id, out_id, branch_id, \
                             relationship_type, trust, tension, dynamics, reason, last_scene_id, \
                             updated_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8)",
                            rusqlite::params![
                                &a,
                                &b,
                                &branch_id_owned,
                                &rel_type,
                                trust,
                                tension,
                                &dynamics_json,
                                now,
                            ],
                        )?;
                        (a.clone(), b.clone())
                    };
                    tx.commit()?;
                    Ok((in_id, out_id))
                }
            })
            .await?;

        self.get_relationship(branch_id, &in_id, &out_id).await
    }

    /// Apply delta updates to an existing relationship's trust + tension,
    /// set the reason and last_scene_id, and bump updated_at.
    pub async fn update_relationship(
        &self,
        branch_id: &str,
        input: &UpdateRelationshipInput,
    ) -> Result<RelatesTo> {
        let branch_id_owned = branch_id.to_string();
        let a = input.character_a_id.clone();
        let b = input.character_b_id.clone();
        let trust_delta = input.trust_delta;
        let tension_delta = input.tension_delta;
        let reason = input.reason.clone();
        let scene_id = input.scene_id.clone();
        let now = timestamp_to_micros(chrono::Utc::now());

        let (in_id, out_id) = self
            .inner
            .pool
            .write({
                let branch_id_owned = branch_id_owned.clone();
                let a = a.clone();
                let b = b.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    let (in_id, out_id): (String, String) = tx.query_row(
                        "SELECT in_id, out_id FROM relates_to \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                             LIMIT 1",
                        rusqlite::params![&branch_id_owned, &a, &b],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    tx.execute(
                        "UPDATE relates_to SET \
                            trust = trust + ?1, \
                            tension = tension + ?2, \
                            reason = ?3, \
                            last_scene_id = ?4, \
                            updated_at = ?5 \
                         WHERE branch_id = ?6 AND in_id = ?7 AND out_id = ?8",
                        rusqlite::params![
                            trust_delta,
                            tension_delta,
                            &reason,
                            &scene_id,
                            now,
                            &branch_id_owned,
                            &in_id,
                            &out_id,
                        ],
                    )?;
                    tx.commit()?;
                    Ok((in_id, out_id))
                }
            })
            .await?;
        self.get_relationship(branch_id, &in_id, &out_id).await
    }

    /// Write absolute `trust` / `tension` values onto an existing
    /// relationship row (matching either orientation of the character
    /// pair). Distinct from `update_relationship`, which applies deltas.
    /// Used by `import_hydrate_bible` because import snapshots carry
    /// canonical absolutes from the source manuscript, not deltas.
    #[allow(clippy::too_many_arguments)]
    pub async fn set_relationship_absolute(
        &self,
        branch_id: &str,
        character_a_id: &str,
        character_b_id: &str,
        trust: i32,
        tension: i32,
        reason: Option<String>,
        last_scene_id: Option<String>,
    ) -> Result<RelatesTo> {
        let branch_id_owned = branch_id.to_string();
        let a = character_a_id.to_string();
        let b = character_b_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        let (in_id, out_id) = self
            .inner
            .pool
            .write({
                let branch_id_owned = branch_id_owned.clone();
                let a = a.clone();
                let b = b.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    let (in_id, out_id): (String, String) = tx.query_row(
                        "SELECT in_id, out_id FROM relates_to \
                             WHERE branch_id = ?1 \
                               AND ((in_id = ?2 AND out_id = ?3) OR (in_id = ?3 AND out_id = ?2)) \
                             LIMIT 1",
                        rusqlite::params![&branch_id_owned, &a, &b],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    tx.execute(
                        "UPDATE relates_to SET \
                            trust = ?1, \
                            tension = ?2, \
                            reason = COALESCE(?3, reason), \
                            last_scene_id = COALESCE(?4, last_scene_id), \
                            updated_at = ?5 \
                         WHERE branch_id = ?6 AND in_id = ?7 AND out_id = ?8",
                        rusqlite::params![
                            trust,
                            tension,
                            &reason,
                            &last_scene_id,
                            now,
                            &branch_id_owned,
                            &in_id,
                            &out_id,
                        ],
                    )?;
                    tx.commit()?;
                    Ok((in_id, out_id))
                }
            })
            .await?;
        self.get_relationship(branch_id, &in_id, &out_id).await
    }

    /// Fetch a single relationship by composite PK.
    pub async fn get_relationship(
        &self,
        branch_id: &str,
        in_id: &str,
        out_id: &str,
    ) -> Result<RelatesTo> {
        let branch_id = branch_id.to_string();
        let in_id = in_id.to_string();
        let out_id = out_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&branch_id, &in_id, &out_id], |r| {
                    RelatesTo::try_from(r)
                })
                .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("relationship not found"))
    }

    pub async fn list_relationships_by_branch(&self, branch_id: &str) -> Result<Vec<RelatesTo>> {
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id = ?1 ORDER BY relationship_type"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&branch_id], |r| RelatesTo::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Revision marker
    // =========================================================================

    /// UPSERT a revision marker keyed by
    /// (branch_id, scene_id, marker_type, target_record_id). Matches the
    /// SurrealDB unique index — repeated upserts overwrite the existing row's
    /// status / position / note / created_at.
    pub async fn upsert_revision_marker(&self, marker: &RevisionMarker) -> Result<RevisionMarker> {
        let project_id = marker.project_id.clone();
        let branch_id = marker.branch_id.clone();
        let scene_id = marker.scene_id.clone();
        let marker_type = marker.marker_type.clone();
        let target_record_id = marker.target_record_id.clone();
        let position = marker.position.clone();
        let note = marker.note.clone();
        let status = marker.status.clone();
        let now = timestamp_to_micros(chrono::Utc::now());
        let new_id = mint_id("revision_marker");
        let new_id_owned = new_id.clone();

        self.inner
            .pool
            .write({
                let branch_id = branch_id.clone();
                let scene_id = scene_id.clone();
                let marker_type = marker_type.clone();
                let target_record_id = target_record_id.clone();
                move |conn| {
                    let tx = conn.transaction()?;
                    // Delete an existing row matching the unique key (target may be NULL).
                    if let Some(target) = &target_record_id {
                        tx.execute(
                            "DELETE FROM revision_marker \
                             WHERE branch_id = ?1 AND scene_id = ?2 \
                               AND marker_type = ?3 AND target_record_id = ?4",
                            rusqlite::params![&branch_id, &scene_id, &marker_type, target],
                        )?;
                    } else {
                        tx.execute(
                            "DELETE FROM revision_marker \
                             WHERE branch_id = ?1 AND scene_id = ?2 \
                               AND marker_type = ?3 AND target_record_id IS NULL",
                            rusqlite::params![&branch_id, &scene_id, &marker_type],
                        )?;
                    }
                    tx.execute(
                        "INSERT INTO revision_marker (id, project_id, branch_id, scene_id, \
                         marker_type, target_record_id, position, note, status, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![
                            &new_id,
                            &project_id,
                            &branch_id,
                            &scene_id,
                            &marker_type,
                            &target_record_id,
                            &position,
                            &note,
                            &status,
                            now,
                        ],
                    )?;
                    tx.commit()?;
                    Ok(())
                }
            })
            .await?;
        self.get_revision_marker(&new_id_owned).await
    }

    pub async fn resolve_revision_marker(&self, marker_id: &str) -> Result<RevisionMarker> {
        let marker_id_owned = marker_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute(
                    "UPDATE revision_marker SET status = 'resolved' WHERE id = ?1",
                    [&marker_id_owned],
                )?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        self.get_revision_marker(marker_id).await
    }

    pub async fn get_revision_marker(&self, id: &str) -> Result<RevisionMarker> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {REVISION_MARKER_COLUMNS} FROM revision_marker WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| RevisionMarker::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("revision_marker not found"))
    }

    pub async fn list_revision_markers_for_scene(
        &self,
        branch_id: &str,
        scene_id: &str,
    ) -> Result<Vec<RevisionMarker>> {
        let branch_id = branch_id.to_string();
        let scene_id = scene_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {REVISION_MARKER_COLUMNS} FROM revision_marker \
                     WHERE branch_id = ?1 AND scene_id = ?2 \
                     ORDER BY marker_type, position"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&branch_id, &scene_id], |r| {
                        RevisionMarker::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Generic helpers: archive_entity, update_entity, get_entity
    // =========================================================================

    /// Set `archived_at` on a single row identified by `table:id`. The set of
    /// tables that have an `archived_at` column is fixed by the schema; this
    /// helper validates the table is in that allowlist before issuing the
    /// UPDATE so we get a clear error instead of a confusing "no such column"
    /// from SQLite.
    pub async fn archive_entity(&self, table: &str, entity_id: &str) -> Result<()> {
        if !table_has_archived_at(table) {
            anyhow::bail!("entity table '{table}' has no archived_at column");
        }
        let entity_id = entity_id.to_string();
        let table = table.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let sql = format!(
                    "UPDATE \"{table}\" SET archived_at = ?1, updated_at = ?1 WHERE id = ?2"
                );
                let n = conn.execute(&sql, rusqlite::params![now, &entity_id])?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Apply a map of field updates to a row identified by `table:id`. Each
    /// (column, value) pair is gated by the same `column_is_updatable`
    /// allowlist as `update_entity_field`; unknown columns return an error
    /// rather than silently being ignored. `updated_at` is set automatically
    /// and must not appear in the changes map (mirrors the SurrealDB version's
    /// `strip_updated_at` behavior).
    ///
    /// This is the SQLite counterpart to SurrealDB's MERGE-style `update_entity`.
    pub async fn update_entity_fields(
        &self,
        table: &str,
        entity_id: &str,
        changes: std::collections::BTreeMap<String, Value>,
    ) -> Result<()> {
        if changes.is_empty() {
            // No-op but still bump updated_at to match the SurrealDB semantics.
            return self.touch_entity_updated_at(table, entity_id).await;
        }
        for col in changes.keys() {
            if col == "updated_at" {
                anyhow::bail!("update_entity_fields refuses to set updated_at directly");
            }
            if !column_is_updatable(table, col) {
                anyhow::bail!("column '{col}' on '{table}' is not in the update allowlist");
            }
        }
        // Issue one statement per column. The allowlist lookup is small and
        // each `update_entity_field` already uses prepare_cached, so the cost
        // is dominated by the round-trips, which are negligible for the small
        // change maps callers actually pass.
        for (col, value) in changes {
            self.update_entity_field(table, entity_id, &col, value)
                .await?;
        }
        Ok(())
    }

    /// Touch a row's `updated_at` column to the current time, no other
    /// changes. Used as the empty-changes path of `update_entity_fields`.
    pub async fn touch_entity_updated_at(&self, table: &str, entity_id: &str) -> Result<()> {
        // Defensive: only run this against tables that have an updated_at
        // column (every project+branch-scoped table does in our schema).
        let table = table.to_string();
        let entity_id = entity_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let sql = format!("UPDATE \"{table}\" SET updated_at = ?1 WHERE id = ?2");
                conn.execute(&sql, rusqlite::params![now, &entity_id])?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    /// Clear the search-embedding index for a project's active branch.
    /// The caller is expected to iterate searchable entities and re-upsert
    /// embeddings via `upsert_search_embedding_document` afterwards. Returns
    /// the number of rows removed.
    pub async fn rebuild_search_index_clear(&self, project_id: &str) -> Result<usize> {
        let branch_id = self.active_branch_id(project_id).await?;
        let project_id = project_id.to_string();
        let n: usize = self
            .inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM search_embedding WHERE project_id = ?1 AND branch_id = ?2",
                    rusqlite::params![&project_id, &branch_id],
                )
            })
            .await?;
        Ok(n)
    }

    /// Set a single column on a row identified by `table:id` to a JSON-encoded
    /// value. The schema column names allowed for update are checked against a
    /// per-table allowlist; this gives the same safety guarantee the
    /// SurrealDB version had via SCHEMAFULL while keeping the signature simple.
    pub async fn update_entity_field(
        &self,
        table: &str,
        entity_id: &str,
        field: &str,
        value: Value,
    ) -> Result<()> {
        if !column_is_updatable(table, field) {
            anyhow::bail!("column '{field}' on '{table}' is not in the update allowlist");
        }
        let table = table.to_string();
        let entity_id = entity_id.to_string();
        let field = field.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        let value_repr = value;
        self.inner
            .pool
            .write(move |conn| {
                let sql = format!(
                    "UPDATE \"{table}\" SET \"{field}\" = ?1, updated_at = ?2 WHERE id = ?3"
                );
                let n = match &value_repr {
                    Value::String(s) => {
                        conn.execute(&sql, rusqlite::params![s, now, &entity_id])?
                    }
                    Value::Null => conn.execute(
                        &sql,
                        rusqlite::params![rusqlite::types::Null, now, &entity_id],
                    )?,
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            conn.execute(&sql, rusqlite::params![i, now, &entity_id])?
                        } else if let Some(f) = n.as_f64() {
                            conn.execute(&sql, rusqlite::params![f, now, &entity_id])?
                        } else {
                            return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "unsupported numeric width for field update",
                                ),
                            )));
                        }
                    }
                    Value::Bool(b) => conn.execute(
                        &sql,
                        rusqlite::params![if *b { 1 } else { 0 }, now, &entity_id],
                    )?,
                    other => {
                        // Arrays/objects: persist as JSON TEXT.
                        let s = serde_json::to_string(other)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        conn.execute(&sql, rusqlite::params![s, now, &entity_id])?
                    }
                };
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    // =========================================================================
    // Save point
    // =========================================================================

    pub async fn create_save_point(
        &self,
        project_id: &str,
        branch_id: &str,
        name: &str,
        description: Option<String>,
    ) -> Result<SavePoint> {
        let id = mint_id("save_point");
        let id_out = id.clone();
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        let name = name.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO save_point (id, project_id, branch_id, name, description, \
                     snapshot_file_path, snapshot_format, snapshot_record_count, \
                     snapshot_created_at, snapshot_sha256, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL, NULL, NULL, ?6)",
                    rusqlite::params![&id, &project_id, &branch_id, &name, &description, now],
                )?;
                Ok(())
            })
            .await?;
        self.get_save_point(&id_out).await
    }

    pub async fn get_save_point(&self, id: &str) -> Result<SavePoint> {
        let id = id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!("SELECT {SAVE_POINT_COLUMNS} FROM save_point WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id], |r| SavePoint::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("save_point not found"))
    }

    pub async fn list_save_points_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SavePoint>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SAVE_POINT_COLUMNS} FROM save_point \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        SavePoint::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn delete_save_point(&self, save_point_id: &str) -> Result<()> {
        let id = save_point_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute("DELETE FROM save_point WHERE id = ?1", [&id])?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        Ok(())
    }

    pub async fn update_save_point_snapshot(
        &self,
        save_point_id: &str,
        file_path: &str,
        format: &str,
        record_count: i64,
        sha256: &str,
    ) -> Result<SavePoint> {
        let save_point_id_owned = save_point_id.to_string();
        let file_path = file_path.to_string();
        let format = format.to_string();
        let sha256 = sha256.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());
        self.inner
            .pool
            .write(move |conn| {
                let n = conn.execute(
                    "UPDATE save_point SET snapshot_file_path = ?1, snapshot_format = ?2, \
                     snapshot_record_count = ?3, snapshot_created_at = ?4, snapshot_sha256 = ?5 \
                     WHERE id = ?6",
                    rusqlite::params![
                        &file_path,
                        &format,
                        record_count,
                        now,
                        &sha256,
                        &save_point_id_owned,
                    ],
                )?;
                if n == 0 {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                Ok(())
            })
            .await?;
        self.get_save_point(save_point_id).await
    }

    // =========================================================================
    // Character state (append-only snapshots per scene)
    // =========================================================================

    pub async fn append_character_state(
        &self,
        params: AppendCharacterStateParams,
    ) -> Result<CharacterState> {
        let id = mint_id("character_state");
        let id_lookup = id.clone();
        let project_id = params.project_id;
        let branch_id = params.branch_id;
        let character_id = params.character_id;
        let scene_id = params.scene_id;
        let book_number = params.book_number;
        let chapter_number = params.chapter_number;
        let scene_order = params.scene_order;
        let emotional_json = serde_json::to_string(&params.patch.emotional_state)
            .context("serializing emotional_state")?;
        let goals_json = serde_json::to_string(&params.patch.goals.unwrap_or_default())
            .context("serializing goals")?;
        let status_json = serde_json::to_string(&params.patch.status.unwrap_or_default())
            .context("serializing status")?;
        let notes_json = serde_json::to_string(&params.patch.notes.unwrap_or_default())
            .context("serializing notes")?;
        let source_summary = params.patch.source_summary;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO character_state (id, project_id, branch_id, character_id, \
                     scene_id, book_number, chapter_number, scene_order, emotional_state, \
                     goals, status, notes, source_summary, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &character_id,
                        &scene_id,
                        book_number,
                        chapter_number,
                        scene_order,
                        &emotional_json,
                        &goals_json,
                        &status_json,
                        &notes_json,
                        &source_summary,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.inner
            .pool
            .read(move |conn| {
                let sql =
                    format!("SELECT {CHARACTER_STATE_COLUMNS} FROM character_state WHERE id = ?1");
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| CharacterState::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("character_state vanished after insert"))
    }

    /// Most recent character_state row for a character at or before
    /// (book, chapter, scene_order). Used by snapshot/briefing flows.
    pub async fn resolve_character_state(
        &self,
        character_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> Result<Option<CharacterState>> {
        let character_id = character_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE character_id = ?1 \
                       AND ( \
                            book_number < ?2 \
                         OR (book_number = ?2 AND chapter_number < ?3) \
                         OR (book_number = ?2 AND chapter_number = ?3 AND scene_order <= ?4) \
                       ) \
                     ORDER BY book_number DESC, chapter_number DESC, scene_order DESC \
                     LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(
                    rusqlite::params![&character_id, book_number, chapter_number, scene_order],
                    |r| CharacterState::try_from(r),
                )
                .optional_inner()
            })
            .await
    }

    pub async fn list_character_states_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CharacterState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY character_id, book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        CharacterState::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // Writer position
    // =========================================================================

    pub async fn get_writer_position(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Option<WriterPosition>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WRITER_POSITION_COLUMNS} FROM writer_position \
                     WHERE project_id = ?1 AND branch_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&project_id, &branch_id], |r| {
                    WriterPosition::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    pub async fn upsert_writer_position(
        &self,
        params: UpsertWriterPositionParams,
    ) -> Result<WriterPosition> {
        // The schema has `UNIQUE (project_id, branch_id)`. Use INSERT ON CONFLICT
        // to update the existing row in-place; mint a fresh id only on insert.
        let id = mint_id("writer_position");
        let project_id = params.project_id.clone();
        let branch_id = params.branch_id.clone();
        let book_id = params.book_id;
        let chapter_id = params.chapter_id;
        let scene_id = params.scene_id;
        let intent = params.intent;
        let next_focus = params.next_focus;
        let updated_by = params.updated_by;
        let updated_at = timestamp_to_micros(params.updated_at);

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO writer_position (id, project_id, branch_id, book_id, chapter_id, \
                     scene_id, intent, next_focus, updated_by, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                     ON CONFLICT (project_id, branch_id) DO UPDATE SET \
                         book_id = excluded.book_id, \
                         chapter_id = excluded.chapter_id, \
                         scene_id = excluded.scene_id, \
                         intent = excluded.intent, \
                         next_focus = excluded.next_focus, \
                         updated_by = excluded.updated_by, \
                         updated_at = excluded.updated_at",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &book_id,
                        &chapter_id,
                        &scene_id,
                        &intent,
                        &next_focus,
                        &updated_by,
                        updated_at,
                    ],
                )?;
                Ok(())
            })
            .await?;
        self.get_writer_position(&params.project_id, &params.branch_id)
            .await?
            .ok_or_else(|| anyhow!("writer_position vanished after upsert"))
    }

    pub async fn delete_writer_position(&self, project_id: &str, branch_id: &str) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM writer_position WHERE project_id = ?1 AND branch_id = ?2",
                    rusqlite::params![&project_id, &branch_id],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }

    // =========================================================================
    // Session activity (append-only log)
    // =========================================================================

    pub async fn append_session_activity(
        &self,
        params: AppendSessionActivityParams,
    ) -> Result<SessionActivity> {
        let id = mint_id("session_activity");
        let id_for_lookup = id.clone();
        let details_json = match params.details_json {
            Some(v) => Some(serde_json::to_string(&v).context("serializing details_json")?),
            None => None,
        };
        let project_id = params.project_id;
        let branch_id = params.branch_id;
        let kind = params.kind;
        let subject_table = params.subject_table;
        let subject_id = params.subject_id;
        let summary = params.summary;
        let now = timestamp_to_micros(chrono::Utc::now());

        self.inner
            .pool
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO session_activity (id, project_id, branch_id, kind, subject_table, \
                     subject_id, summary, details_json, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![
                        &id,
                        &project_id,
                        &branch_id,
                        &kind,
                        &subject_table,
                        &subject_id,
                        &summary,
                        &details_json,
                        now,
                    ],
                )?;
                Ok(())
            })
            .await?;
        let id_lookup = id_for_lookup.clone();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SESSION_ACTIVITY_COLUMNS} FROM session_activity WHERE id = ?1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row([&id_lookup], |r| SessionActivity::try_from(r))
                    .optional_inner()
            })
            .await?
            .ok_or_else(|| anyhow!("session_activity vanished after insert"))
    }

    pub async fn list_recent_session_activity(
        &self,
        project_id: &str,
        branch_id: &str,
        limit: i64,
    ) -> Result<Vec<SessionActivity>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SESSION_ACTIVITY_COLUMNS} FROM session_activity \
                     WHERE project_id = ?1 AND branch_id = ?2 \
                     ORDER BY created_at DESC LIMIT ?3"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id, limit], |r| {
                        SessionActivity::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_character_arcs_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CharacterArc>> {
        list_branch_scoped(
            &self.inner.pool,
            CHARACTER_ARC_COLUMNS,
            "character_arc",
            "character_id, created_at",
            project_id,
            branch_id,
            |r| CharacterArc::try_from(r),
        )
        .await
    }

    // -------------------------------------------------------------------------
    // `list_X_by_project` convenience wrappers that resolve to the project's
    // active branch and delegate to the branch-scoped lister. Mirrors the
    // SurrealDB repo's pattern where service-layer callers don't always have
    // a branch_id at hand.
    // -------------------------------------------------------------------------

    pub async fn list_characters_by_project(&self, project_id: &str) -> Result<Vec<Character>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_characters_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_locations_by_project(&self, project_id: &str) -> Result<Vec<Location>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_locations_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_factions_by_project(&self, project_id: &str) -> Result<Vec<Faction>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_factions_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_religions_by_project(&self, project_id: &str) -> Result<Vec<Religion>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_religions_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_economies_by_project(&self, project_id: &str) -> Result<Vec<Economy>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_economies_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_terms_by_project(&self, project_id: &str) -> Result<Vec<Term>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_terms_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_plot_lines_by_project(&self, project_id: &str) -> Result<Vec<PlotLine>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_plot_lines_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_conflicts_by_project(&self, project_id: &str) -> Result<Vec<Conflict>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_conflicts_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_themes_by_project(&self, project_id: &str) -> Result<Vec<Theme>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_themes_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_motifs_by_project(&self, project_id: &str) -> Result<Vec<Motif>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_motifs_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_narrative_promises_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<NarrativePromise>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_narrative_promises_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_world_rules_by_project(&self, project_id: &str) -> Result<Vec<WorldRule>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_world_rules_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_timeline_events_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<TimelineEvent>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_timeline_events_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_system_overlays_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<SystemOverlay>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_system_overlays_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_character_arcs_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<CharacterArc>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_character_arcs_by_project_and_branch(project_id, &branch_id)
            .await
    }

    /// List every pacing_tracker row on the project's active branch. Mirrors
    /// the `list_pacing_trackers_by_project_and_branch` shape resolved
    /// against `active_branch_id`; used by the `pacing/overview` project
    /// resource reader and other branch-implicit callers.
    pub async fn list_pacing_trackers_by_project(
        &self,
        project_id: &str,
    ) -> Result<Vec<PacingTracker>> {
        let branch_id = self.active_branch_id(project_id).await?;
        self.list_pacing_trackers_by_project_and_branch(project_id, &branch_id)
            .await
    }

    pub async fn list_narrative_promises_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<NarrativePromise>> {
        list_branch_scoped(
            &self.inner.pool,
            NARRATIVE_PROMISE_COLUMNS,
            "narrative_promise",
            "created_at",
            project_id,
            branch_id,
            |r| NarrativePromise::try_from(r),
        )
        .await
    }

    pub async fn get_world_state_for_location(
        &self,
        branch_id: &str,
        location_id: &str,
    ) -> Result<Option<WorldState>> {
        let branch_id = branch_id.to_string();
        let location_id = location_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WORLD_STATE_COLUMNS} FROM world_state \
                     WHERE branch_id = ?1 AND location_id = ?2 LIMIT 1"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                stmt.query_row(rusqlite::params![&branch_id, &location_id], |r| {
                    WorldState::try_from(r)
                })
                .optional_inner()
            })
            .await
    }

    // =========================================================================
    // Additional list_* helpers backing assemble_subject_snapshot
    // =========================================================================

    /// List every chapter row in a project, ordered by book + chapter number.
    /// Used by the snapshot batch loader; mirrors the SurrealDB scoped
    /// `SELECT * FROM chapter WHERE project_id = $project_id ORDER BY ...`.
    pub async fn list_chapters_by_project(&self, project_id: &str) -> Result<Vec<Chapter>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_COLUMNS} FROM chapter \
                     WHERE project_id = ?1 ORDER BY book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Chapter::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every knowledge_fact row on a branch, ordered by created_at.
    pub async fn list_knowledge_facts_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<KnowledgeFact>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {KNOWLEDGE_FACT_COLUMNS} FROM knowledge_fact \
                     WHERE project_id = ?1 AND branch_id = ?2 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        KnowledgeFact::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every character_voice_profile row tied to characters in the given
    /// branch. Joins through `character.branch_id` since voice profiles are
    /// keyed only by `character_id` themselves.
    pub async fn list_voice_profiles_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<CharacterVoiceProfile>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE character_id IN ( \
                       SELECT id FROM character WHERE project_id = ?1 AND branch_id = ?2 \
                     )"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        CharacterVoiceProfile::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    /// List every world_state row on a branch.
    pub async fn list_world_states_by_project_and_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<WorldState>> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WORLD_STATE_COLUMNS} FROM world_state \
                     WHERE project_id = ?1 AND branch_id = ?2"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params![&project_id, &branch_id], |r| {
                        WorldState::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    // =========================================================================
    // assemble_subject_snapshot
    //
    // Port of the SurrealDB `assemble_subject_snapshot[_s]` /
    // `load_snapshot_batch_context` / `assemble_subject_snapshot_from_batch`
    // surface. The big-batch loader fans out one read per table to populate
    // `SnapshotBatchContext`; pure helpers then project the in-memory batch
    // into a `SubjectSnapshot` per subject. ID semantics differ: SQLite uses
    // plain `String` IDs of the form `"<table>:<ulid>"` rather than the
    // SurrealDB `RecordId { table, key }` pair, so id-to-table dispatch is a
    // string split on the leading `':'`.
    // =========================================================================

    pub async fn assemble_subject_snapshot(
        &self,
        project_id: &str,
        branch_id: &str,
        subject: &Subject,
        placement: &StoryPlacement,
    ) -> Result<SubjectSnapshot> {
        let mut snapshots = self
            .assemble_subject_snapshots(
                project_id,
                branch_id,
                std::slice::from_ref(subject),
                placement,
            )
            .await?;
        snapshots
            .pop()
            .context("subject snapshot assembly returned no snapshot")
    }

    pub async fn assemble_subject_snapshots(
        &self,
        project_id: &str,
        branch_id: &str,
        subjects: &[Subject],
        placement: &StoryPlacement,
    ) -> Result<Vec<SubjectSnapshot>> {
        let batch = self
            .load_snapshot_batch_context(project_id, branch_id)
            .await?;
        subjects
            .iter()
            .map(|subject| assemble_subject_snapshot_from_batch(&batch, subject, placement))
            .collect()
    }

    /// Load every table the snapshot helpers depend on in one shot. The
    /// SurrealDB version fans these out via `tokio::try_join!`; the SQLite
    /// pool is single-writer (and `read` calls already share a pool of
    /// connections), so we issue the reads sequentially rather than fighting
    /// the borrow checker over the `&self` capture across futures. The cost
    /// is the same: each table is one prepared statement against an indexed
    /// `(project_id, branch_id)` key.
    async fn load_snapshot_batch_context(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<SnapshotBatchContext> {
        let project = self.get_project(project_id).await?;
        let books = self.list_books_by_project(project_id).await?;
        let chapters = self.list_chapters_by_project(project_id).await?;
        let world_rules = self
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let characters = self
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let locations = self
            .list_locations_by_project_and_branch(project_id, branch_id)
            .await?;
        let factions = self
            .list_factions_by_project_and_branch(project_id, branch_id)
            .await?;
        let religions = self
            .list_religions_by_project_and_branch(project_id, branch_id)
            .await?;
        let economies = self
            .list_economies_by_project_and_branch(project_id, branch_id)
            .await?;
        let plot_lines = self
            .list_plot_lines_by_project_and_branch(project_id, branch_id)
            .await?;
        let conflicts = self
            .list_conflicts_by_project_and_branch(project_id, branch_id)
            .await?;
        let themes = self
            .list_themes_by_project_and_branch(project_id, branch_id)
            .await?;
        let motifs = self
            .list_motifs_by_project_and_branch(project_id, branch_id)
            .await?;
        let overlays = self
            .list_system_overlays_by_project_and_branch(project_id, branch_id)
            .await?;
        let promises = self
            .list_narrative_promises_by_project_and_branch(project_id, branch_id)
            .await?;
        let arcs = self
            .list_character_arcs_by_project_and_branch(project_id, branch_id)
            .await?;
        let terms = self
            .list_terms_by_project_and_branch(project_id, branch_id)
            .await?;
        let relationships = self.list_relationships_by_branch(branch_id).await?;
        let events = self
            .list_timeline_events_by_project_and_branch(project_id, branch_id)
            .await?;
        let scenes = self
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        let canonical_facts = self
            .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let knowledge_facts = self
            .list_knowledge_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let character_states = self
            .list_character_states_by_project_and_branch(project_id, branch_id)
            .await?;
        let voice_profiles = self
            .list_voice_profiles_by_project_and_branch(project_id, branch_id)
            .await?;
        let world_states = self
            .list_world_states_by_project_and_branch(project_id, branch_id)
            .await?;

        Ok(SnapshotBatchContext {
            project,
            books,
            chapters,
            world_rules,
            characters,
            locations,
            factions,
            religions,
            economies,
            plot_lines,
            conflicts,
            themes,
            motifs,
            overlays,
            promises,
            arcs,
            terms,
            relationships,
            events,
            scenes,
            canonical_facts,
            knowledge_facts,
            character_states,
            voice_profiles,
            world_states,
        })
    }

    // =========================================================================
    // Merge branch snapshot
    //
    // Ports the SurrealDB-era `merge_branch_snapshot` to SQLite. Writes the
    // mergeable slice of a source branch (scenes, character_state,
    // relates_to, pacing_tracker) into `target_branch_id` inside a single
    // transaction. Conflict detection lives in the service layer; this fn
    // assumes the caller has already filtered out conflicting positions.
    //
    // Semantic note: scene/character_state/pacing_tracker rows are
    // re-stamped with the target branch's id and minted fresh primary keys.
    // Relationship edges (relates_to) reuse the source row's character ids
    // (in_id/out_id) — they're FK-constrained to `character(id)`, which is
    // a per-branch row in the SQLite schema. The caller is responsible for
    // ensuring those characters exist on the target branch (typical case:
    // both branches descend from the same parent main branch and share
    // characters).
    // =========================================================================

    /// Apply a pre-filtered set of source-branch rows onto `target_branch_id`
    /// in a single transaction. Mirrors `merge_branch_snapshot` from the
    /// SurrealDB-era repository (repository.rs:5414..5535 in 705b835^).
    pub async fn merge_branch_snapshot(
        &self,
        project_id: &str,
        target_branch_id: &str,
        source_scenes: &[Scene],
        source_states: &[CharacterState],
        source_relationships: &[RelatesTo],
        source_pacing: &[PacingTracker],
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let target_branch_id = target_branch_id.to_string();
        let now = timestamp_to_micros(chrono::Utc::now());

        // Materialize owned inputs so the move-closure can take them. Each
        // entry carries everything the INSERT/UPDATE statements need, plus
        // pre-serialized JSON blobs for the columns that store JSON.
        struct SceneRow {
            book_id: String,
            chapter_id: String,
            book_number: i32,
            chapter_number: i32,
            scene_order: i32,
            full_text: String,
            summary: String,
            content_rating: String,
            tone: Option<String>,
        }
        let scenes_owned: Vec<SceneRow> = source_scenes
            .iter()
            .map(|s| SceneRow {
                book_id: s.book_id.clone(),
                chapter_id: s.chapter_id.clone(),
                book_number: s.book_number,
                chapter_number: s.chapter_number,
                scene_order: s.scene_order,
                full_text: s.full_text.clone(),
                summary: s.summary.clone(),
                content_rating: s.content_rating.clone(),
                tone: s.tone.clone(),
            })
            .collect();

        struct StateRow {
            character_id: String,
            scene_id: Option<String>,
            book_number: i32,
            chapter_number: i32,
            scene_order: i32,
            emotional_json: String,
            goals_json: String,
            status_json: String,
            notes_json: String,
            source_summary: Option<String>,
        }
        let states_owned: Vec<StateRow> = source_states
            .iter()
            .map(|s| {
                Ok(StateRow {
                    character_id: s.character_id.clone(),
                    scene_id: s.scene_id.clone(),
                    book_number: s.book_number,
                    chapter_number: s.chapter_number,
                    scene_order: s.scene_order,
                    emotional_json: serde_json::to_string(&s.emotional_state)
                        .context("serializing character_state.emotional_state")?,
                    goals_json: serde_json::to_string(&s.goals)
                        .context("serializing character_state.goals")?,
                    status_json: serde_json::to_string(&s.status)
                        .context("serializing character_state.status")?,
                    notes_json: serde_json::to_string(&s.notes)
                        .context("serializing character_state.notes")?,
                    source_summary: s.source_summary.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        struct RelRow {
            in_id: String,
            out_id: String,
            relationship_type: String,
            trust: i32,
            tension: i32,
            dynamics_json: String,
            reason: Option<String>,
            last_scene_id: Option<String>,
        }
        let rels_owned: Vec<RelRow> = source_relationships
            .iter()
            .map(|r| {
                Ok(RelRow {
                    in_id: r.in_id.clone(),
                    out_id: r.out_id.clone(),
                    relationship_type: r.relationship_type.clone(),
                    trust: r.trust,
                    tension: r.tension,
                    dynamics_json: serde_json::to_string(&r.dynamics)
                        .context("serializing relates_to.dynamics")?,
                    reason: r.reason.clone(),
                    last_scene_id: r.last_scene_id.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        struct TrackerRow {
            character_arc_id: String,
            per_book_budget_json: String,
            max_progress_per_chapter: Option<f64>,
            milestone_spacing: Option<i32>,
            sprint_allowance: Option<i32>,
            regression_budget: Option<f64>,
            current_progress: f64,
            budget_remaining: f64,
            velocity: String,
            status: String,
            next_milestone: Option<String>,
            warnings_json: String,
        }
        let trackers_owned: Vec<TrackerRow> = source_pacing
            .iter()
            .map(|t| {
                Ok(TrackerRow {
                    character_arc_id: t.character_arc_id.clone(),
                    per_book_budget_json: serde_json::to_string(&t.per_book_budget)
                        .context("serializing pacing_tracker.per_book_budget")?,
                    max_progress_per_chapter: t.max_progress_per_chapter,
                    milestone_spacing: t.milestone_spacing,
                    sprint_allowance: t.sprint_allowance,
                    regression_budget: t.regression_budget,
                    current_progress: t.current_progress,
                    budget_remaining: t.budget_remaining,
                    velocity: t.velocity.clone(),
                    status: t.status.clone(),
                    next_milestone: t.next_milestone.clone(),
                    warnings_json: serde_json::to_string(&t.warnings)
                        .context("serializing pacing_tracker.warnings")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        self.inner
            .pool
            .write(move |conn| {
                let tx = conn.transaction()?;

                // Scenes: UPDATE-or-INSERT keyed by
                // (project_id, branch_id, book#, chapter#, scene_order).
                for scene in &scenes_owned {
                    let affected = tx.execute(
                        "UPDATE scene SET full_text = ?1, summary = ?2, \
                         content_rating = ?3, tone = ?4, updated_at = ?5 \
                         WHERE project_id = ?6 AND branch_id = ?7 \
                           AND book_number = ?8 AND chapter_number = ?9 \
                           AND scene_order = ?10",
                        rusqlite::params![
                            &scene.full_text,
                            &scene.summary,
                            &scene.content_rating,
                            &scene.tone,
                            now,
                            &project_id,
                            &target_branch_id,
                            scene.book_number,
                            scene.chapter_number,
                            scene.scene_order,
                        ],
                    )?;
                    if affected == 0 {
                        let new_id = mint_id_local("scene");
                        tx.execute(
                            "INSERT INTO scene (id, project_id, branch_id, book_id, \
                             chapter_id, book_number, chapter_number, scene_order, \
                             full_text, summary, content_rating, tone, draft_origin, \
                             created_at, updated_at) \
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                                     NULL, ?13, ?13)",
                            rusqlite::params![
                                &new_id,
                                &project_id,
                                &target_branch_id,
                                &scene.book_id,
                                &scene.chapter_id,
                                scene.book_number,
                                scene.chapter_number,
                                scene.scene_order,
                                &scene.full_text,
                                &scene.summary,
                                &scene.content_rating,
                                &scene.tone,
                                now,
                            ],
                        )?;
                    }
                }

                // character_state: DELETE-then-INSERT keyed by
                // (branch_id, character_id, book#, chapter#, scene_order).
                for state in &states_owned {
                    tx.execute(
                        "DELETE FROM character_state \
                         WHERE branch_id = ?1 AND character_id = ?2 \
                           AND book_number = ?3 AND chapter_number = ?4 \
                           AND scene_order = ?5",
                        rusqlite::params![
                            &target_branch_id,
                            &state.character_id,
                            state.book_number,
                            state.chapter_number,
                            state.scene_order,
                        ],
                    )?;
                    let new_id = mint_id_local("character_state");
                    tx.execute(
                        "INSERT INTO character_state (id, project_id, branch_id, \
                         character_id, scene_id, book_number, chapter_number, \
                         scene_order, emotional_state, goals, status, notes, \
                         source_summary, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                                 ?13, ?14)",
                        rusqlite::params![
                            &new_id,
                            &project_id,
                            &target_branch_id,
                            &state.character_id,
                            &state.scene_id,
                            state.book_number,
                            state.chapter_number,
                            state.scene_order,
                            &state.emotional_json,
                            &state.goals_json,
                            &state.status_json,
                            &state.notes_json,
                            &state.source_summary,
                            now,
                        ],
                    )?;
                }

                // relates_to: DELETE-then-INSERT keyed by composite PK
                // (branch_id, in_id, out_id). Re-stamps branch_id to target.
                for rel in &rels_owned {
                    tx.execute(
                        "DELETE FROM relates_to \
                         WHERE branch_id = ?1 AND in_id = ?2 AND out_id = ?3",
                        rusqlite::params![&target_branch_id, &rel.in_id, &rel.out_id,],
                    )?;
                    tx.execute(
                        "INSERT INTO relates_to (in_id, out_id, branch_id, \
                         relationship_type, trust, tension, dynamics, reason, \
                         last_scene_id, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![
                            &rel.in_id,
                            &rel.out_id,
                            &target_branch_id,
                            &rel.relationship_type,
                            rel.trust,
                            rel.tension,
                            &rel.dynamics_json,
                            &rel.reason,
                            &rel.last_scene_id,
                            now,
                        ],
                    )?;
                }

                // pacing_tracker: DELETE-then-INSERT keyed by
                // (branch_id, character_arc_id). Re-stamps branch_id to target.
                for tracker in &trackers_owned {
                    tx.execute(
                        "DELETE FROM pacing_tracker \
                         WHERE branch_id = ?1 AND character_arc_id = ?2",
                        rusqlite::params![&target_branch_id, &tracker.character_arc_id],
                    )?;
                    let new_id = mint_id_local("pacing_tracker");
                    tx.execute(
                        "INSERT INTO pacing_tracker (id, project_id, branch_id, \
                         character_arc_id, per_book_budget, max_progress_per_chapter, \
                         milestone_spacing, sprint_allowance, regression_budget, \
                         current_progress, budget_remaining, velocity, status, \
                         next_milestone, warnings, updated_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                                 ?13, ?14, ?15, ?16)",
                        rusqlite::params![
                            &new_id,
                            &project_id,
                            &target_branch_id,
                            &tracker.character_arc_id,
                            &tracker.per_book_budget_json,
                            tracker.max_progress_per_chapter,
                            tracker.milestone_spacing,
                            tracker.sprint_allowance,
                            tracker.regression_budget,
                            tracker.current_progress,
                            tracker.budget_remaining,
                            &tracker.velocity,
                            &tracker.status,
                            &tracker.next_milestone,
                            &tracker.warnings_json,
                            now,
                        ],
                    )?;
                }

                tx.commit()?;
                Ok(())
            })
            .await
    }

    // =========================================================================
    // Restore branch snapshot (inverse of merge_branch_snapshot)
    //
    // Replaces every branch-scoped row in (project_id, branch_id) with the
    // rows carried in the snapshot. Runs in a single writer transaction with
    // `PRAGMA defer_foreign_keys = 1` so the order of DELETE/INSERT statements
    // doesn't have to obey the FK graph — referential integrity is checked
    // once at COMMIT. Mirrors the intent of `restore_branch_snapshot`
    // (`repository.rs:3202..3390` in 705b835^), which used SurrealDB's
    // BEGIN/COMMIT + heterogeneous CONTENT UPSERT to achieve the same thing.
    //
    // `save_point` and `bible_branch` rows are intentionally NOT touched:
    //   * Save points outlive the branch state they describe (you need to be
    //     able to restore to a different save_point after a restore).
    //   * The bible_branch row itself defines the branch we're restoring
    //     into — wiping it would orphan every FK we're about to write.
    //
    // Tables walked: `BRANCH_RESTORE_TABLES` (defined alongside
    // `BranchRestoreSnapshot` at the top of this file).
    // =========================================================================

    /// Rewind `branch_id` (under `project_id`) to the state encoded by
    /// `snapshot`. Deletes every branch-scoped row in
    /// `BRANCH_RESTORE_TABLES` whose (project_id, branch_id) match, then
    /// re-inserts the snapshot rows. `save_point` and `bible_branch` are
    /// preserved.
    pub async fn restore_branch_snapshot(
        &self,
        project_id: &str,
        branch_id: &str,
        snapshot: &BranchRestoreSnapshot,
    ) -> Result<()> {
        let project_id = project_id.to_string();
        let branch_id = branch_id.to_string();

        // Materialize snapshot for the move-closure. We clone per-table so
        // the closure owns everything it needs without re-borrowing.
        let rows_by_table: std::collections::BTreeMap<String, Vec<serde_json::Map<String, Value>>> =
            snapshot.rows_by_table.clone();

        self.inner
            .pool
            .write(move |conn| {
                conn.execute_batch("PRAGMA defer_foreign_keys = 1;")?;
                let tx = conn.transaction()?;

                // Phase 1: delete the current branch content. character_*
                // child tables cascade with character; scene_* with scene;
                // book_outline/chapter_outline/relates_to/knows have their
                // own branch_id columns and need explicit deletes.
                for table in BRANCH_RESTORE_TABLES {
                    let delete_sql = match *table {
                        // (project_id, branch_id) tables — the common case.
                        "revision_marker"
                        | "dual_persona_review"
                        | "scene_beat_annotation"
                        | "canonical_fact"
                        | "scene_version"
                        | "character_state"
                        | "future_knowledge"
                        | "timeline_event"
                        | "temporal_intervention"
                        | "progression_event"
                        | "pacing_tracker"
                        | "chapter_plan"
                        | "chapter_summary"
                        | "pacing_curve"
                        | "pacing_config"
                        | "narrative_promise"
                        | "character_arc"
                        | "plot_line"
                        | "conflict"
                        | "theme"
                        | "motif"
                        | "faction"
                        | "religion"
                        | "economy"
                        | "term"
                        | "system_overlay"
                        | "knowledge_fact"
                        | "world_state"
                        | "world_rule"
                        | "scene"
                        | "location"
                        | "character"
                        | "knows" => {
                            format!("DELETE FROM {table} WHERE project_id = ?1 AND branch_id = ?2")
                        }
                        // Branch-only tables (no project_id column).
                        "book_outline" | "chapter_outline" | "relates_to" => {
                            format!("DELETE FROM {table} WHERE branch_id = ?1")
                        }
                        // Child tables keyed by a parent_id column. Their
                        // parents (character, scene) cascade, so explicit
                        // deletes are belt-and-suspenders for the case where
                        // a snapshot row points at a parent that's still
                        // present (we want a clean slate before INSERT).
                        "scene_source_link" => format!(
                            "DELETE FROM {table} WHERE scene_id IN \
                             (SELECT id FROM scene WHERE project_id = ?1 AND branch_id = ?2)"
                        ),
                        "character_voice_profile" | "character_emotional_profile" => format!(
                            "DELETE FROM {table} WHERE character_id IN \
                             (SELECT id FROM character WHERE project_id = ?1 AND branch_id = ?2)"
                        ),
                        _ => continue,
                    };
                    if delete_sql.contains("?2") {
                        tx.execute(&delete_sql, rusqlite::params![&project_id, &branch_id])?;
                    } else {
                        tx.execute(&delete_sql, rusqlite::params![&branch_id])?;
                    }
                }

                // Phase 2: insert the snapshot rows. For each table we build
                // an INSERT statement keyed by the union of column names
                // present on the first row (the snapshot is guaranteed to
                // be self-consistent because it was produced by
                // `dump_project_table`).
                for table in BRANCH_RESTORE_TABLES {
                    let Some(rows) = rows_by_table.get(*table) else {
                        continue;
                    };
                    if rows.is_empty() {
                        continue;
                    }
                    // Use the first row's keys as the canonical column
                    // ordering. All rows for a given table come from
                    // `SELECT *` so they share the same key set.
                    let columns: Vec<&str> = rows[0].keys().map(String::as_str).collect();
                    let placeholders: Vec<String> =
                        (1..=columns.len()).map(|i| format!("?{i}")).collect();
                    let col_list = columns.join(", ");
                    let ph_list = placeholders.join(", ");
                    let insert_sql = format!("INSERT INTO {table} ({col_list}) VALUES ({ph_list})");
                    let mut stmt = tx.prepare(&insert_sql)?;
                    for row in rows {
                        // Build the value vector in column order. Anything
                        // missing from the row map becomes NULL.
                        let mut values: Vec<JsonParam> = Vec::with_capacity(columns.len());
                        for col in &columns {
                            let v = row.get(*col).cloned().unwrap_or(Value::Null);
                            values.push(JsonParam::from_value_for_column(col, v));
                        }
                        let params: Vec<&dyn rusqlite::ToSql> =
                            values.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
                        stmt.execute(rusqlite::params_from_iter(params.iter()))?;
                    }
                }

                tx.commit()?;
                Ok(())
            })
            .await
    }

    // =========================================================================
    // Generic JSON-row dumps (used by export_bible / save_point snapshots)
    //
    // Walks an arbitrary table via `SELECT *` and converts each row to a JSON
    // object keyed by column name. Values follow rusqlite's `ValueRef`
    // taxonomy:
    //
    //   NULL    -> Value::Null
    //   INTEGER -> Value::Number (i64)
    //   REAL    -> Value::Number (f64; non-finite becomes Value::Null)
    //   TEXT    -> Value::String, with a heuristic to re-parse JSON-shaped
    //              text (objects "{...}" / arrays "[...]") when the column
    //              name suggests a JSON payload (see `looks_like_json_column`).
    //   BLOB    -> base64-encoded Value::String prefixed with "base64:".
    //
    // This deliberately does not piggy-back on the typed record TryFrom impls
    // because (a) ~59 record types would need `#[derive(Serialize)]` to mirror
    // the SurrealDB JSON shape, and (b) the SurrealDB reference reaches for
    // raw `db().select() / db().query()` which is itself shapeless — so the
    // export format is documented as "schema-by-name JSON" either way.
    // =========================================================================

    /// Dump every row of `table` where `project_id = ?` as a list of JSON
    /// objects. Columns are emitted under their schema names. Used by
    /// `export_bible` and save-point snapshots; the table name is validated
    /// against a fixed allowlist by the caller (`build_project_export_payload`),
    /// which is the only safe entry point for this primitive.
    pub async fn dump_project_table(&self, table: &str, project_id: &str) -> Result<Vec<Value>> {
        ensure_safe_export_identifier(table)?;
        let project_id = project_id.to_string();
        let sql = format!("SELECT * FROM {table} WHERE project_id = ?1");
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let column_names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let rows = stmt
                    .query_map([&project_id], move |row| {
                        Ok(row_to_json_value(row, &column_names))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter().collect::<rusqlite::Result<Vec<Value>>>()
            })
            .await
    }

    /// Dump every row of `table` filtered by an arbitrary single-column WHERE
    /// `{column} = ?` clause. Used for join-table exports (e.g. `relates_to`
    /// keyed by branch_id) where the row itself has no `project_id` column.
    pub async fn dump_table_by_column(
        &self,
        table: &str,
        column: &str,
        value: &str,
    ) -> Result<Vec<Value>> {
        ensure_safe_export_identifier(table)?;
        ensure_safe_export_identifier(column)?;
        let value = value.to_string();
        let sql = format!("SELECT * FROM {table} WHERE {column} = ?1");
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let column_names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let rows = stmt
                    .query_map([&value], move |row| {
                        Ok(row_to_json_value(row, &column_names))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter().collect::<rusqlite::Result<Vec<Value>>>()
            })
            .await
    }

    /// Dump every row of `table` where `{column} IN (...)`. Used to follow
    /// FK chains during export (e.g. `import_source_document` keyed by a set
    /// of `session_id` values gathered from `import_session`).
    pub async fn dump_table_by_column_in(
        &self,
        table: &str,
        column: &str,
        values: &[String],
    ) -> Result<Vec<Value>> {
        ensure_safe_export_identifier(table)?;
        ensure_safe_export_identifier(column)?;
        if values.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (0..values.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT * FROM {table} WHERE {column} IN ({placeholders})");
        let owned: Vec<String> = values.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let mut stmt = conn.prepare(&sql)?;
                let column_names: Vec<String> =
                    stmt.column_names().iter().map(|s| s.to_string()).collect();
                let params: Vec<&dyn rusqlite::ToSql> =
                    owned.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), move |row| {
                        Ok(row_to_json_value(row, &column_names))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows.into_iter().collect::<rusqlite::Result<Vec<Value>>>()
            })
            .await
    }

    // =========================================================================
    // Typed project-wide list_all_* helpers
    //
    // These mirror the existing `_by_project` / `_by_project_and_branch`
    // accessors but always span every branch on the project — the shape
    // `build_project_export_payload` needs to round-trip a full project
    // snapshot through the typed records (so timestamps serialize as
    // RFC-3339 strings rather than unix-micro integers).
    //
    // Each `SELECT` order is stable so the exported `tables.*` arrays are
    // deterministic across runs.
    // =========================================================================

    pub async fn list_all_save_points(&self, project_id: &str) -> Result<Vec<SavePoint>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SAVE_POINT_COLUMNS} FROM save_point \
                     WHERE project_id = ?1 ORDER BY created_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SavePoint::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_characters(&self, project_id: &str) -> Result<Vec<Character>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_COLUMNS} FROM character \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Character::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_voice_profiles(
        &self,
        project_id: &str,
    ) -> Result<Vec<CharacterVoiceProfile>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_VOICE_PROFILE_COLUMNS} FROM character_voice_profile \
                     WHERE character_id IN ( \
                       SELECT id FROM character WHERE project_id = ?1 \
                     ) ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterVoiceProfile::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_emotional_profiles(
        &self,
        project_id: &str,
    ) -> Result<Vec<CharacterEmotionalProfile>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_EMOTIONAL_PROFILE_COLUMNS} FROM character_emotional_profile \
                     WHERE character_id IN ( \
                       SELECT id FROM character WHERE project_id = ?1 \
                     ) ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterEmotionalProfile::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_states(&self, project_id: &str) -> Result<Vec<CharacterState>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_STATE_COLUMNS} FROM character_state \
                     WHERE project_id = ?1 \
                     ORDER BY character_id, book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterState::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_locations(&self, project_id: &str) -> Result<Vec<Location>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {LOCATION_COLUMNS} FROM location \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Location::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_world_states(&self, project_id: &str) -> Result<Vec<WorldState>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WORLD_STATE_COLUMNS} FROM world_state \
                     WHERE project_id = ?1 ORDER BY updated_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| WorldState::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_world_rules(&self, project_id: &str) -> Result<Vec<WorldRule>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {WORLD_RULE_COLUMNS} FROM world_rule \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| WorldRule::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_revision_markers(&self, project_id: &str) -> Result<Vec<RevisionMarker>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {REVISION_MARKER_COLUMNS} FROM revision_marker \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| RevisionMarker::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_scenes(&self, project_id: &str) -> Result<Vec<Scene>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_COLUMNS} FROM scene \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number, scene_order"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Scene::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_scene_versions(&self, project_id: &str) -> Result<Vec<SceneVersion>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_VERSION_COLUMNS} FROM scene_version \
                     WHERE project_id = ?1 ORDER BY scene_id, version_number DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SceneVersion::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_relationships_by_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<RelatesTo>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {RELATES_TO_COLUMNS} FROM relates_to \
                     WHERE branch_id IN ({placeholders}) \
                     ORDER BY branch_id, relationship_type"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        RelatesTo::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_factions(&self, project_id: &str) -> Result<Vec<Faction>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {FACTION_COLUMNS} FROM faction \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Faction::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_religions(&self, project_id: &str) -> Result<Vec<Religion>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {RELIGION_COLUMNS} FROM religion \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Religion::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_economies(&self, project_id: &str) -> Result<Vec<Economy>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {ECONOMY_COLUMNS} FROM economy \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Economy::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_terms(&self, project_id: &str) -> Result<Vec<Term>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TERM_COLUMNS} FROM term \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Term::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_plot_lines(&self, project_id: &str) -> Result<Vec<PlotLine>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PLOT_LINE_COLUMNS} FROM plot_line \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PlotLine::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_conflicts(&self, project_id: &str) -> Result<Vec<Conflict>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CONFLICT_COLUMNS} FROM conflict \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Conflict::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_themes(&self, project_id: &str) -> Result<Vec<Theme>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {THEME_COLUMNS} FROM theme \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Theme::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_motifs(&self, project_id: &str) -> Result<Vec<Motif>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {MOTIF_COLUMNS} FROM motif \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Motif::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_narrative_promises(
        &self,
        project_id: &str,
    ) -> Result<Vec<NarrativePromise>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {NARRATIVE_PROMISE_COLUMNS} FROM narrative_promise \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| NarrativePromise::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_character_arcs(&self, project_id: &str) -> Result<Vec<CharacterArc>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHARACTER_ARC_COLUMNS} FROM character_arc \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CharacterArc::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_pacing_trackers(&self, project_id: &str) -> Result<Vec<PacingTracker>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PACING_TRACKER_COLUMNS} FROM pacing_tracker \
                     WHERE project_id = ?1 ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| PacingTracker::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_book_outlines_by_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<BookOutline>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {BOOK_OUTLINE_COLUMNS} FROM book_outline \
                     WHERE branch_id IN ({placeholders}) \
                     ORDER BY branch_id, book_id"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        BookOutline::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_chapter_outlines_by_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<ChapterOutline>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {CHAPTER_OUTLINE_COLUMNS} FROM chapter_outline \
                     WHERE branch_id IN ({placeholders}) \
                     ORDER BY branch_id, chapter_id"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ChapterOutline::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_chapter_plans(&self, project_id: &str) -> Result<Vec<ChapterPlan>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_PLAN_COLUMNS} FROM chapter_plan \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ChapterPlan::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_scene_beat_annotations(
        &self,
        project_id: &str,
    ) -> Result<Vec<SceneBeatAnnotation>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SCENE_BEAT_ANNOTATION_COLUMNS} FROM scene_beat_annotation \
                     WHERE project_id = ?1 ORDER BY scene_id"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SceneBeatAnnotation::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_chapter_summaries(
        &self,
        project_id: &str,
    ) -> Result<Vec<ChapterSummary>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CHAPTER_SUMMARY_COLUMNS} FROM chapter_summary \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ChapterSummary::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_future_knowledge(
        &self,
        project_id: &str,
    ) -> Result<Vec<FutureKnowledge>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {FUTURE_KNOWLEDGE_COLUMNS} FROM future_knowledge \
                     WHERE project_id = ?1 ORDER BY character_id, created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| FutureKnowledge::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_timeline_events(&self, project_id: &str) -> Result<Vec<TimelineEvent>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TIMELINE_EVENT_COLUMNS} FROM timeline_event \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| TimelineEvent::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_temporal_interventions(
        &self,
        project_id: &str,
    ) -> Result<Vec<TemporalIntervention>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {TEMPORAL_INTERVENTION_COLUMNS} FROM temporal_intervention \
                     WHERE project_id = ?1 ORDER BY title"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| TemporalIntervention::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_system_overlays(&self, project_id: &str) -> Result<Vec<SystemOverlay>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {SYSTEM_OVERLAY_COLUMNS} FROM system_overlay \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| SystemOverlay::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_progression_events(
        &self,
        project_id: &str,
    ) -> Result<Vec<ProgressionEvent>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {PROGRESSION_EVENT_COLUMNS} FROM progression_event \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| ProgressionEvent::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_dual_persona_reviews(
        &self,
        project_id: &str,
    ) -> Result<Vec<DualPersonaReview>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {DUAL_PERSONA_REVIEW_COLUMNS} FROM dual_persona_review \
                     WHERE project_id = ?1 ORDER BY updated_at DESC"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| DualPersonaReview::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_canonical_facts(&self, project_id: &str) -> Result<Vec<CanonicalFact>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {CANONICAL_FACT_COLUMNS} FROM canonical_fact \
                     WHERE project_id = ?1 \
                     ORDER BY branch_id, book_number, chapter_number, created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| CanonicalFact::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_all_knows(&self, project_id: &str) -> Result<Vec<Knows>> {
        let project_id = project_id.to_string();
        self.inner
            .pool
            .read(move |conn| {
                let sql = format!(
                    "SELECT {KNOWS_COLUMNS} FROM knows \
                     WHERE project_id = ?1 ORDER BY created_at"
                );
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map([&project_id], |r| Knows::try_from(r))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_sessions_by_target_branch_ids(
        &self,
        branch_ids: &[String],
    ) -> Result<Vec<ImportSession>> {
        if branch_ids.is_empty() {
            return Ok(Vec::new());
        }
        let branch_ids: Vec<String> = branch_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..branch_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_SESSION_COLUMNS} FROM import_session \
                     WHERE target_branch_id IN ({placeholders}) \
                     ORDER BY imported_at DESC"
                );
                let params: Vec<&dyn rusqlite::ToSql> = branch_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportSession::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_source_documents_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportSourceDocument>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_SOURCE_DOCUMENT_COLUMNS} FROM import_source_document \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, source_order"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportSourceDocument::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_segments_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportSegment>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_SEGMENT_COLUMNS} FROM import_segment \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, source_order"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportSegment::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_entity_mentions_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportEntityMention>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_MENTION_COLUMNS} FROM import_entity_mention \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportEntityMention::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_entity_clusters_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportEntityCluster>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_ENTITY_CLUSTER_COLUMNS} FROM import_entity_cluster \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportEntityCluster::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_character_dossiers_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportCharacterDossier>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_CHARACTER_DOSSIER_COLUMNS} FROM import_character_dossier \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportCharacterDossier::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_world_dossiers_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportWorldDossier>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_WORLD_DOSSIER_COLUMNS} FROM import_world_dossier \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportWorldDossier::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_narrative_dossiers_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportNarrativeDossier>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_NARRATIVE_DOSSIER_COLUMNS} FROM import_narrative_dossier \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportNarrativeDossier::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_resume_snapshots_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportResumeSnapshot>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_RESUME_SNAPSHOT_COLUMNS} FROM import_resume_snapshot \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportResumeSnapshot::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn list_import_review_items_for_sessions(
        &self,
        session_ids: &[String],
    ) -> Result<Vec<ImportReviewItem>> {
        if session_ids.is_empty() {
            return Ok(Vec::new());
        }
        let session_ids: Vec<String> = session_ids.to_vec();
        self.inner
            .pool
            .read(move |conn| {
                let placeholders = (0..session_ids.len())
                    .map(|i| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT {IMPORT_REVIEW_ITEM_COLUMNS} FROM import_review_item \
                     WHERE session_id IN ({placeholders}) \
                     ORDER BY session_id, created_at"
                );
                let params: Vec<&dyn rusqlite::ToSql> = session_ids
                    .iter()
                    .map(|s| s as &dyn rusqlite::ToSql)
                    .collect();
                let mut stmt = conn.prepare_cached(&sql)?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(params.iter()), |r| {
                        ImportReviewItem::try_from(r)
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .await
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

/// Tables that carry an `archived_at` column. Driven by the V0001 schema —
/// see `crates/spindle-adapters/migrations/V0001__initial_schema.sql`.
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
    )
}

/// Allowlist of (table, column) pairs that update_entity_field is permitted
/// to mutate. Limits surface area to text/json fields where partial overwrites
/// are safe; identity fields, FKs, and computed fields stay outside this list.
fn column_is_updatable(table: &str, column: &str) -> bool {
    matches!(
        (table, column),
        // Common notes column.
        (_, "notes")
            // Free-form summaries / titles.
            | ("project", "name")
            | ("project", "genre")
            | ("project", "project_type")
            | ("book", "title")
            | ("chapter", "title")
            | ("character", "summary")
            | ("character", "role")
            | ("character", "realm")
            | ("character", "appearance")
            | ("location", "summary")
            | ("location", "realm")
            | ("location", "kind")
            | ("faction", "summary")
            | ("religion", "summary")
            | ("religion", "deity_or_principle")
            | ("economy", "summary")
            | ("economy", "currency")
            | ("term", "definition")
            | ("term", "pronunciation")
            | ("plot_line", "summary")
            | ("plot_line", "status")
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
            | ("theme", "theme_statement")
            | ("theme", "thesis_antithesis")
            | ("motif", "description")
            | ("narrative_promise", "description")
            | ("narrative_promise", "status")
            | ("world_rule", "description")
    )
}

/// Local clone of `mint_id` usable inside `pool.write` closures, where the
/// outer `mint_id` symbol isn't in the closure's import path. Identical
/// behavior — separate name avoids the captured-symbol headache.
fn mint_id_local(table: &str) -> String {
    mint_id(table)
}

/// Shared body for `list_X_by_project_and_branch` methods. Captures the
/// boilerplate of building the SELECT, binding (project_id, branch_id), and
/// mapping rows through `TryFrom<&Row>`.
async fn list_branch_scoped<T, F>(
    pool: &SqlitePool,
    columns: &'static str,
    table: &'static str,
    order_by: &'static str,
    project_id: &str,
    branch_id: &str,
    map: F,
) -> Result<Vec<T>>
where
    T: Send + 'static,
    F: Fn(&rusqlite::Row<'_>) -> rusqlite::Result<T> + Send + Sync + 'static,
{
    let project_id = project_id.to_string();
    let branch_id = branch_id.to_string();
    pool.read(move |conn| {
        let sql = format!(
            "SELECT {columns} FROM {table} \
             WHERE project_id = ?1 AND branch_id = ?2 ORDER BY {order_by}"
        );
        let mut stmt = conn.prepare_cached(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![&project_id, &branch_id], |r| map(r))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
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
            provenance: scene_provenance(&fact.scene_id),
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
mod tests {
    use super::*;
    use spindle_core::models::ReaderContract;
    use tempfile::TempDir;

    async fn fresh_repo() -> (TempDir, Repository) {
        let tmp = TempDir::new().unwrap();
        let pool = SqlitePool::open(&tmp.path().join("repo.db")).await.unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let repo = Repository::new(pool, data_dir);
        (tmp, repo)
    }

    #[tokio::test]
    async fn create_project_round_trip() {
        let (_tmp, repo) = fresh_repo().await;
        let input = CreateProjectInput {
            name: "Spindle".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "epic".into(),
                style_notes: vec!["sparse".into()],
                boundaries: vec!["no second-person".into()],
            },
        };
        let (project, branch, book, chapter) = repo.create_project(&input).await.unwrap();
        assert!(project.id.starts_with("project:"));
        assert_eq!(project.name, "Spindle");
        assert_eq!(
            project.active_branch_id.as_deref(),
            Some(branch.id.as_str())
        );
        assert_eq!(branch.name, "main");
        assert_eq!(branch.status, "active");
        assert_eq!(book.book_number, 1);
        assert_eq!(chapter.chapter_number, 1);

        // get_project round-trips
        let again = repo.get_project(&project.id).await.unwrap();
        assert_eq!(again.id, project.id);
        assert_eq!(again.reader_contract.promise, "epic");

        // get_active_branch resolves through active_branch_id
        let active = repo.get_active_branch(&project.id).await.unwrap();
        assert_eq!(active.id, branch.id);

        // list_branches_by_project returns the main branch only
        let branches = repo.list_branches_by_project(&project.id).await.unwrap();
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].id, branch.id);
    }

    #[tokio::test]
    async fn create_and_switch_branch() {
        let (_tmp, repo) = fresh_repo().await;
        let input = CreateProjectInput {
            name: "Spindle".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        };
        let (project, main, _book, _chapter) = repo.create_project(&input).await.unwrap();

        let feature = repo
            .create_branch(
                &project.id,
                &main.id,
                "feature-arc-2",
                "feature",
                Some("trying new ending".into()),
            )
            .await
            .unwrap();
        assert_eq!(feature.name, "feature-arc-2");
        assert_eq!(feature.parent_branch_id.as_deref(), Some(main.id.as_str()));

        let updated = repo
            .switch_active_branch(&project.id, &feature.id)
            .await
            .unwrap();
        assert_eq!(
            updated.active_branch_id.as_deref(),
            Some(feature.id.as_str())
        );

        let branches = repo.list_branches_by_project(&project.id).await.unwrap();
        assert_eq!(branches.len(), 2);

        let active = repo.get_active_branch(&project.id).await.unwrap();
        assert_eq!(active.id, feature.id);
    }

    #[tokio::test]
    async fn missing_project_returns_error() {
        let (_tmp, repo) = fresh_repo().await;
        let err = repo
            .get_project("project:does-not-exist")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("project not found"), "{err}");
    }

    #[tokio::test]
    async fn archive_entity_sets_archived_at_and_rejects_non_archivable() {
        use spindle_core::models::CreateFactionInput;
        let (_tmp, repo) = fresh_repo().await;
        let (project, _branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let faction = repo
            .create_faction(&CreateFactionInput {
                project_id: project.id.clone(),
                name: "Wardens".into(),
                faction_type: "military".into(),
                realm: None,
                summary: "Oathbound.".into(),
                tags: Vec::new(),
            })
            .await
            .unwrap();
        assert!(faction.archived_at.is_none());

        repo.archive_entity("faction", &faction.id).await.unwrap();
        let after = repo.get_faction(&faction.id).await.unwrap();
        assert!(after.archived_at.is_some(), "archived_at must be set");

        // Project doesn't have an archived_at column — must reject.
        let err = repo
            .archive_entity("project", &project.id)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no archived_at column"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn fts_search_characters_finds_by_name_and_appearance() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput,
        };
        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let _mara = repo
            .create_character(&CreateCharacterInput {
                project_id: project.id.clone(),
                name: "Mara Oathkeeper".into(),
                summary: "Warden of the Ash Gate.".into(),
                role: "protagonist".into(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: None,
                    vocabulary: Vec::new(),
                    sentence_structure: Vec::new(),
                    tics: Vec::new(),
                    forbidden_words: Vec::new(),
                    example_lines: Vec::new(),
                    established_in_scene_id: None,
                    updated_at: None,
                },
                emotional_profile: CharacterEmotionalProfileData {
                    base_emotions: std::collections::BTreeMap::new(),
                    suppressed: Vec::new(),
                    triggers: Vec::new(),
                    defense_mechanisms: Vec::new(),
                    flex_range: None,
                },
                initial_state: Some(CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: None,
                    status: None,
                    notes: None,
                    source_summary: None,
                }),
            })
            .await
            .unwrap();

        let hits = repo
            .fts_search_characters(&project.id, Some(&branch.id), "warden", 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "Mara matches 'warden' via summary");
        assert!(
            hits[0].2.to_lowercase().contains("warden"),
            "snippet contains the term: {}",
            hits[0].2
        );
    }

    #[tokio::test]
    async fn fts_search_scenes_finds_prose_match() {
        use spindle_core::models::{ContentRating, SaveSceneDraftInput};
        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        // Two scenes with distinct content so FTS can tell them apart.
        repo.save_scene_draft(
            &project.id,
            &branch.id,
            &SaveSceneDraftInput {
                project_id: project.id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "The Ash Gate burned through the night.".into(),
                summary: "Gate burns".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            },
        )
        .await
        .unwrap();
        repo.save_scene_draft(
            &project.id,
            &branch.id,
            &SaveSceneDraftInput {
                project_id: project.id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 2,
                full_text: "Aldric copied maps in the candlelight.".into(),
                summary: "Maps".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            },
        )
        .await
        .unwrap();

        let hits = repo
            .fts_search_scenes(&project.id, Some(&branch.id), "burned", 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "only the Ash Gate scene contains 'burned'");
        assert!(
            hits[0].2.contains("<mark>burned</mark>"),
            "snippet must mark the term: {}",
            hits[0].2
        );

        let hits = repo
            .fts_search_scenes(&project.id, Some(&branch.id), "maps", 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn knn_search_returns_inserted_embeddings_ordered_by_distance() {
        use crate::ai::SearchDocument;
        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        // Two characters with deterministically-distinct content for the
        // TokenHash backend; their embeddings should be similar to themselves
        // and farther from each other.
        let mara_id = "character:01HM";
        let aldric_id = "character:01HA";
        repo.upsert_search_embedding_document(
            &project.id,
            &branch.id,
            mara_id,
            &SearchDocument {
                entity_table: "character".into(),
                title: "Mara".into(),
                excerpt: "oath warden".into(),
                content: "Mara stands at the Ash Gate. Oathbound. Sword across her knee.".into(),
            },
        )
        .await
        .unwrap();
        repo.upsert_search_embedding_document(
            &project.id,
            &branch.id,
            aldric_id,
            &SearchDocument {
                entity_table: "character".into(),
                title: "Aldric".into(),
                excerpt: "scribe".into(),
                content: "Aldric copies maps in candlelight. Ink stains his cuffs.".into(),
            },
        )
        .await
        .unwrap();

        // Fetch Mara's own embedding to use as a query.
        let mara_row = repo
            .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
            .await
            .unwrap()
            .into_iter()
            .find(|r| r.entity_id == mara_id)
            .expect("mara row");
        let mara_query = mara_row.embedding.clone();

        // kNN: nearest should be Mara herself (distance ~= 0).
        let results = repo
            .knn_search_embeddings(&project.id, Some(&branch.id), &mara_query, 2)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0].0.entity_id, mara_id,
            "self-similarity must be the closest match"
        );
        assert!(
            results[0].1 < results[1].1,
            "results must be sorted by ascending distance"
        );
    }

    #[tokio::test]
    async fn search_embedding_upsert_round_trips_blob() {
        use crate::ai::SearchDocument;
        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let doc = SearchDocument {
            entity_table: "character".into(),
            title: "Mara".into(),
            excerpt: "Oathbound warden of the Ash Gate.".into(),
            content: "Mara, an oathbound warden, holds the gate against the dark.".into(),
        };
        let entity_id = "character:01HFAKE";

        repo.upsert_search_embedding_document(&project.id, &branch.id, entity_id, &doc)
            .await
            .unwrap();

        let rows = repo
            .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let stored = &rows[0];
        assert_eq!(stored.entity_table, "character");
        assert_eq!(stored.entity_id, entity_id);
        assert_eq!(stored.embedding.len(), 64, "TokenHash backend = 64 dims");
        // The TokenHash backend is deterministic on content; the embedding
        // contains the L2-normalized hash distribution.
        assert!(stored.embedding.iter().any(|&v| v != 0.0));

        // Second upsert with new content must replace in place (no new row).
        let updated_doc = SearchDocument {
            content: "Mara: oathbound. She doubts.".into(),
            ..doc
        };
        repo.upsert_search_embedding_document(&project.id, &branch.id, entity_id, &updated_doc)
            .await
            .unwrap();
        let rows2 = repo
            .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
            .await
            .unwrap();
        assert_eq!(rows2.len(), 1, "ON CONFLICT must update in place");
        // Content changes alter the embedding deterministically.
        assert_ne!(rows2[0].content, stored.content);

        // Delete by entity.
        let removed = repo
            .delete_search_embedding_for_entity(&project.id, entity_id)
            .await
            .unwrap();
        assert!(removed);
        let rows3 = repo
            .list_search_embeddings_by_project_and_branch(&project.id, &branch.id)
            .await
            .unwrap();
        assert!(rows3.is_empty());
    }

    #[tokio::test]
    async fn save_scene_draft_creates_then_versions_then_cascades() {
        use spindle_core::models::{ContentRating, SaveSceneDraftInput};
        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        // First save: INSERT path. Created flag must be true.
        let (scene, created) = repo
            .save_scene_draft(
                &project.id,
                &branch.id,
                &SaveSceneDraftInput {
                    project_id: project.id.clone(),
                    book_number: 1,
                    chapter_number: 1,
                    chapter_id: None,
                    scene_order: 1,
                    full_text: "First draft of the gate.".into(),
                    summary: "Mara stands watch.".into(),
                    content_rating: ContentRating::General,
                    tone: Some("grim".into()),
                    generation_id: None,
                    source_path: None,
                },
            )
            .await
            .unwrap();
        assert!(created);
        assert!(scene.id.starts_with("scene:"));
        assert_eq!(scene.full_text, "First draft of the gate.");
        let scene_id = scene.id.clone();

        // Second save with changed prose: UPDATE path, scene_changed = true.
        // Expects a scene_version snapshot of the previous prose.
        let (updated, created2) = repo
            .save_scene_draft(
                &project.id,
                &branch.id,
                &SaveSceneDraftInput {
                    project_id: project.id.clone(),
                    book_number: 1,
                    chapter_number: 1,
                    chapter_id: None,
                    scene_order: 1,
                    full_text: "Revised draft. The gate burns.".into(),
                    summary: "Mara holds the gate.".into(),
                    content_rating: ContentRating::General,
                    tone: Some("grim".into()),
                    generation_id: None,
                    source_path: None,
                },
            )
            .await
            .unwrap();
        assert!(!created2, "second save must update in place");
        assert_eq!(updated.id, scene_id);
        assert_eq!(updated.full_text, "Revised draft. The gate burns.");

        // scene_version row should exist holding the prior prose.
        let versions: i64 = repo
            .inner
            .pool
            .read({
                let id = scene_id.clone();
                move |conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM scene_version WHERE scene_id = ?1",
                        [&id],
                        |r| r.get(0),
                    )
                }
            })
            .await
            .unwrap();
        assert_eq!(versions, 1, "one snapshot of the prior prose");

        // Third save with same prose: UPDATE path, scene_changed = false.
        // No new scene_version, no cascade.
        let (_, created3) = repo
            .save_scene_draft(
                &project.id,
                &branch.id,
                &SaveSceneDraftInput {
                    project_id: project.id.clone(),
                    book_number: 1,
                    chapter_number: 1,
                    chapter_id: None,
                    scene_order: 1,
                    full_text: "Revised draft. The gate burns.".into(),
                    summary: "Mara holds the gate.".into(),
                    content_rating: ContentRating::General,
                    tone: Some("grim".into()),
                    generation_id: None,
                    source_path: None,
                },
            )
            .await
            .unwrap();
        assert!(!created3);
        let versions_again: i64 = repo
            .inner
            .pool
            .read({
                let id = scene_id.clone();
                move |conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM scene_version WHERE scene_id = ?1",
                        [&id],
                        |r| r.get(0),
                    )
                }
            })
            .await
            .unwrap();
        assert_eq!(versions_again, 1, "unchanged prose must not snapshot");
    }

    #[tokio::test]
    async fn create_character_inserts_all_four_tables_atomically() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput, FlexRange,
        };
        let (_tmp, repo) = fresh_repo().await;
        let (project, _branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let (character, voice, emotional, state) = repo
            .create_character(&CreateCharacterInput {
                project_id: project.id.clone(),
                name: "Mara".into(),
                summary: "An oathbound warden.".into(),
                role: "protagonist".into(),
                realm: Some("Marches".into()),
                voice_profile: CharacterVoiceProfileData {
                    tone: Some("grim".into()),
                    vocabulary: vec!["oath".into(), "ash".into()],
                    sentence_structure: vec!["clipped".into()],
                    tics: vec!["touches scar".into()],
                    forbidden_words: Vec::new(),
                    example_lines: vec!["I know the cost.".into()],
                    established_in_scene_id: None,
                    updated_at: None,
                },
                emotional_profile: CharacterEmotionalProfileData {
                    base_emotions: std::collections::BTreeMap::new(),
                    suppressed: vec!["fear".into()],
                    triggers: Vec::new(),
                    defense_mechanisms: vec!["silence".into()],
                    flex_range: Some(FlexRange {
                        low: Some("withdrawn".into()),
                        high: Some("explosive".into()),
                    }),
                },
                initial_state: Some(CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: Some(vec!["keep the gate".into()]),
                    status: Some(vec!["wary".into()]),
                    notes: None,
                    source_summary: Some("introduction".into()),
                }),
            })
            .await
            .unwrap();

        assert!(character.id.starts_with("character:"));
        assert_eq!(character.normalized_name, "mara");
        assert_eq!(voice.character_id, character.id);
        assert_eq!(voice.tone.as_deref(), Some("grim"));
        assert_eq!(emotional.character_id, character.id);
        assert!(emotional.flex_range.is_some());
        assert_eq!(state.character_id, character.id);
        assert_eq!(state.goals, vec!["keep the gate".to_string()]);
        assert_eq!(state.source_summary.as_deref(), Some("introduction"));

        // Verify the indirect access paths see the same rows.
        let voice_again = repo
            .get_character_voice_profile(&character.id)
            .await
            .unwrap();
        assert_eq!(voice_again.id, voice.id);
        let emo_again = repo
            .get_character_emotional_profile(&character.id)
            .await
            .unwrap();
        assert_eq!(emo_again.id, emotional.id);
    }

    #[tokio::test]
    async fn writer_position_upsert_replaces_in_place() {
        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, book, chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let p1 = repo
            .upsert_writer_position(UpsertWriterPositionParams {
                project_id: project.id.clone(),
                branch_id: branch.id.clone(),
                book_id: Some(book.id.clone()),
                chapter_id: Some(chapter.id.clone()),
                scene_id: None,
                intent: "draft".into(),
                next_focus: Some("intro".into()),
                updated_by: "test".into(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        assert_eq!(p1.intent, "draft");

        // Second upsert with different intent must update in place (no new row).
        let p2 = repo
            .upsert_writer_position(UpsertWriterPositionParams {
                project_id: project.id.clone(),
                branch_id: branch.id.clone(),
                book_id: Some(book.id.clone()),
                chapter_id: Some(chapter.id.clone()),
                scene_id: None,
                intent: "revise".into(),
                next_focus: None,
                updated_by: "test".into(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        assert_eq!(p2.id, p1.id, "upsert must reuse the row id");
        assert_eq!(p2.intent, "revise");
        assert!(p2.next_focus.is_none());

        repo.delete_writer_position(&project.id, &branch.id)
            .await
            .unwrap();
        assert!(
            repo.get_writer_position(&project.id, &branch.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Exercises every plain narrative-entity create (+ matching get) against
    /// a fresh project. Catches column-order drift between INSERT param order
    /// and the `*_COLUMNS` constant used by the read path.
    #[tokio::test]
    async fn narrative_entity_creates_round_trip() {
        use spindle_core::models::{
            CreateConflictInput, CreateEconomyInput, CreateFactionInput, CreateLocationInput,
            CreateMotifInput, CreateNarrativePromiseInput, CreatePlotLineInput,
            CreateReligionInput, CreateTermInput, CreateThemeInput, StoryPlacement,
            WorldStateInput,
        };
        let (_tmp, repo) = fresh_repo().await;
        let input = CreateProjectInput {
            name: "P".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        };
        let (project, _branch, _book, _chapter) = repo.create_project(&input).await.unwrap();

        let (loc, ws) = repo
            .create_location(&CreateLocationInput {
                project_id: project.id.clone(),
                name: "Ash Gate".into(),
                kind: "city".into(),
                realm: Some("Northern Marches".into()),
                summary: "A blackened wall.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: Some("Wardens".into()),
                    status: Some("watchful".into()),
                    prosperity: Some("poor".into()),
                    stability: Some("tense".into()),
                    threat_level: Some("high".into()),
                    sensory_details: vec!["smell of ash".into()],
                },
            })
            .await
            .unwrap();
        assert!(loc.id.starts_with("location:"));
        assert_eq!(loc.normalized_name, "ash gate");
        assert_eq!(ws.location_id, loc.id);

        let faction = repo
            .create_faction(&CreateFactionInput {
                project_id: project.id.clone(),
                name: "Wardens".into(),
                faction_type: "military".into(),
                realm: None,
                summary: "Oathbound.".into(),
                tags: vec!["oath".into(), "fire".into()],
            })
            .await
            .unwrap();
        assert_eq!(faction.tags, vec!["oath".to_string(), "fire".to_string()]);

        let religion = repo
            .create_religion(&CreateReligionInput {
                project_id: project.id.clone(),
                name: "Ash Pact".into(),
                deity_or_principle: "Cinders".into(),
                summary: "An oath rite.".into(),
                tags: vec!["oath".into()],
            })
            .await
            .unwrap();
        assert_eq!(religion.deity_or_principle, "Cinders");

        let economy = repo
            .create_economy(&CreateEconomyInput {
                project_id: project.id.clone(),
                name: "Marches".into(),
                realm: None,
                summary: "Trade dies at the gate.".into(),
                scarce_resources: vec!["salt".into()],
                trade_goods: vec!["pelts".into()],
                currency: Some("iron coin".into()),
                notes: vec!["barter common".into()],
            })
            .await
            .unwrap();
        assert_eq!(economy.scarce_resources, vec!["salt".to_string()]);

        let term = repo
            .create_term(&CreateTermInput {
                project_id: project.id.clone(),
                term_text: "Ashbound".into(),
                pronunciation: Some("ASH-bownd".into()),
                definition: "Oathsworn to the Ash Gate.".into(),
                usage_context: None,
                origin: None,
            })
            .await
            .unwrap();
        assert_eq!(term.normalized_term, "ashbound");

        let plot = repo
            .create_plot_line(&CreatePlotLineInput {
                project_id: project.id.clone(),
                name: "Gate Breach".into(),
                plot_type: "main".into(),
                summary: "The wardens fail.".into(),
                status: Some("rising".into()),
                convergence_points: vec![StoryPlacement {
                    book_number: 1,
                    chapter_number: 3,
                    scene_order: Some(2),
                    note: None,
                }],
            })
            .await
            .unwrap();
        assert_eq!(plot.status, "rising");
        assert_eq!(plot.convergence_points.len(), 1);

        let conflict = repo
            .create_conflict(&CreateConflictInput {
                project_id: project.id.clone(),
                name: "Oath vs Mercy".into(),
                conflict_type: "internal".into(),
                stakes: "the city".into(),
                escalation_stages: vec!["doubt".into(), "fracture".into()],
                expected_total_cycles: Some(3),
                try_fail_cycles: Vec::new(),
                stated_consequences: Vec::new(),
            })
            .await
            .unwrap();
        assert_eq!(conflict.escalation_stages.len(), 2);

        let theme = repo
            .create_theme(&CreateThemeInput {
                project_id: project.id.clone(),
                theme_statement: "Mercy costs.".into(),
                thesis_antithesis: "Oath / Mercy".into(),
                introduction_point: None,
                resolution_point: None,
            })
            .await
            .unwrap();
        assert_eq!(theme.theme_statement, "Mercy costs.");

        let motif = repo
            .create_motif(&CreateMotifInput {
                project_id: project.id.clone(),
                name: "Ash on lips".into(),
                description: "Recurs at oath moments.".into(),
                max_uses_per_chapter: Some(2),
                connected_theme_ids: vec![theme.id.clone()],
            })
            .await
            .unwrap();
        assert_eq!(motif.connected_theme_ids, vec![theme.id]);

        let promise = repo
            .create_narrative_promise(&CreateNarrativePromiseInput {
                project_id: project.id.clone(),
                promise_type: "payoff".into(),
                description: "The gate will fall.".into(),
                planted_at: StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(0),
                    note: None,
                },
                planned_payoff: Some(StoryPlacement {
                    book_number: 1,
                    chapter_number: 12,
                    scene_order: None,
                    note: Some("climax".into()),
                }),
                notes: vec!["watch for ash imagery".into()],
            })
            .await
            .unwrap();
        assert_eq!(promise.status, "planted");
        assert_eq!(promise.planned_payoff.as_ref().unwrap().chapter_number, 12);
    }

    #[tokio::test]
    async fn assemble_subject_snapshot_character_round_trip() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput, FlexRange, StoryPlacement,
        };
        use spindle_core::subject::{Subject, SubjectTable};
        use spindle_core::subject_snapshot::SubjectKindSpecific;

        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "P".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let (character, voice, _emotional, _state) = repo
            .create_character(&CreateCharacterInput {
                project_id: project.id.clone(),
                name: "Mara".into(),
                summary: "An oathbound warden.".into(),
                role: "protagonist".into(),
                realm: Some("Marches".into()),
                voice_profile: CharacterVoiceProfileData {
                    tone: Some("grim".into()),
                    vocabulary: vec!["oath".into()],
                    sentence_structure: vec!["clipped".into()],
                    tics: Vec::new(),
                    forbidden_words: Vec::new(),
                    example_lines: vec!["I know the cost.".into()],
                    established_in_scene_id: None,
                    updated_at: None,
                },
                emotional_profile: CharacterEmotionalProfileData {
                    base_emotions: std::collections::BTreeMap::new(),
                    suppressed: Vec::new(),
                    triggers: Vec::new(),
                    defense_mechanisms: Vec::new(),
                    flex_range: Some(FlexRange {
                        low: None,
                        high: None,
                    }),
                },
                initial_state: Some(CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: Some(vec!["keep the gate".into()]),
                    status: Some(vec!["wary".into()]),
                    notes: None,
                    source_summary: Some("introduction".into()),
                }),
            })
            .await
            .unwrap();

        let placement = StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(0),
            note: None,
        };
        let subject = Subject::new(SubjectTable::Character, character.id.clone()).unwrap();

        let snapshot = repo
            .assemble_subject_snapshot(&project.id, &branch.id, &subject, &placement)
            .await
            .unwrap();

        assert_eq!(snapshot.display_name(), "Mara");
        assert_eq!(snapshot.at_placement().book_number, 1);
        match snapshot.kind_specific() {
            SubjectKindSpecific::Character(details) => {
                assert_eq!(details.role, "protagonist");
                assert_eq!(details.summary, "An oathbound warden.");
                assert_eq!(details.realm.as_deref(), Some("Marches"));
            }
            other => panic!("expected Character kind, got {other:?}"),
        }

        let voice_summary = snapshot
            .voice_profile()
            .expect("voice profile should be present");
        assert_eq!(voice_summary.tone.as_deref(), Some("grim"));
        assert_eq!(voice_summary.vocabulary, vec!["oath".to_string()]);
        assert_eq!(voice.character_id, character.id);

        let state_summary = snapshot
            .current_state()
            .expect("character state should be present");
        assert_eq!(state_summary.goals, vec!["keep the gate".to_string()]);
        assert_eq!(state_summary.status, vec!["wary".to_string()]);

        // Batch path round-trips identically.
        let batched = repo
            .assemble_subject_snapshots(
                &project.id,
                &branch.id,
                std::slice::from_ref(&subject),
                &placement,
            )
            .await
            .unwrap();
        assert_eq!(batched.len(), 1);
        assert_eq!(batched[0].display_name(), "Mara");
    }

    /// `merge_branch_snapshot` applies the source-branch slice onto a
    /// feature branch and re-stamps every row's `branch_id`.
    ///
    /// Exercises all four merge paths in a single tx:
    ///   * scene INSERT (new position) AND UPDATE (existing position)
    ///   * character_state DELETE-then-INSERT at a fresh position
    ///   * relates_to DELETE-then-INSERT for an existing edge on the target
    ///   * pacing_tracker DELETE-then-INSERT for the target branch's arc
    ///
    /// Verifies via list-by-branch reads that the target branch holds the
    /// expected rows after merge and the source branch is untouched.
    #[tokio::test]
    async fn merge_branch_snapshot_applies_source_slice_to_target_branch() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
            CreateCharacterArcInput, CreateCharacterInput, CreateRelationshipInput,
            SaveSceneDraftInput, StoryPlacement,
        };

        let (_tmp, repo) = fresh_repo().await;
        let (project, main_branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "Merge Source Project".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "ride".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        // Two characters live on the main branch (per-branch character model).
        let empty_voice = CharacterVoiceProfileData {
            vocabulary: Vec::new(),
            sentence_structure: Vec::new(),
            tics: Vec::new(),
            forbidden_words: Vec::new(),
            example_lines: Vec::new(),
            tone: None,
            established_in_scene_id: None,
            updated_at: None,
        };
        let empty_emotional = CharacterEmotionalProfileData {
            base_emotions: Default::default(),
            suppressed: Vec::new(),
            triggers: Vec::new(),
            defense_mechanisms: Vec::new(),
            flex_range: None,
        };
        let (mara, _, _, _) = repo
            .create_character(&CreateCharacterInput {
                project_id: project.id.clone(),
                name: "Mara".into(),
                role: "protagonist".into(),
                realm: None,
                summary: "warden".into(),
                voice_profile: empty_voice.clone(),
                emotional_profile: empty_emotional.clone(),
                initial_state: None,
            })
            .await
            .unwrap();
        let (aldric, _, _, _) = repo
            .create_character(&CreateCharacterInput {
                project_id: project.id.clone(),
                name: "Aldric".into(),
                role: "scribe".into(),
                realm: None,
                summary: "ink-stained".into(),
                voice_profile: empty_voice.clone(),
                emotional_profile: empty_emotional.clone(),
                initial_state: None,
            })
            .await
            .unwrap();

        // A target-branch baseline scene at (1,1,1) — will be UPDATED by merge.
        repo.save_scene_draft(
            &project.id,
            &main_branch.id,
            &SaveSceneDraftInput {
                project_id: project.id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "main: gate stands.".into(),
                summary: "main 1.1.1".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            },
        )
        .await
        .unwrap();

        // A baseline pacing_tracker on the main branch — will be replaced.
        let arc = repo
            .create_character_arc(&CreateCharacterArcInput {
                project_id: project.id.clone(),
                character_id: mara.id.clone(),
                arc_type: "growth".into(),
                starting_state: "wary".into(),
                ending_state: "open".into(),
                milestones: Vec::new(),
                thematic_purpose: "trust".into(),
                connected_theme_ids: Vec::new(),
            })
            .await
            .unwrap();
        let original_tracker = repo
            .create_pacing_tracker(&project.id, &arc.id)
            .await
            .unwrap();
        assert_eq!(original_tracker.status, "on_track");
        assert_eq!(original_tracker.branch_id, main_branch.id);

        // A baseline relationship on the main branch — will be replaced.
        repo.create_relationship(
            &main_branch.id,
            &CreateRelationshipInput {
                character_a_id: mara.id.clone(),
                character_b_id: aldric.id.clone(),
                relationship_type: "ally".into(),
                initial_trust: 5,
                initial_tension: 0,
                dynamics: vec!["trusting".into()],
            },
        )
        .await
        .unwrap();

        // Now build a synthetic source-branch slice. We don't actually need
        // a second branch to exist for `merge_branch_snapshot` — it operates
        // on owned Vec<Scene>/etc passed in by the caller (the service
        // layer fetches them from a real source branch). But to keep the
        // FK invariants sane (scene references book/chapter, etc.), reuse
        // the main-branch row metadata for book_id/chapter_id and just
        // re-stamp branch_id on the way in.
        let main_scenes = repo
            .list_scenes_by_project_and_branch(&project.id, &main_branch.id)
            .await
            .unwrap();
        let baseline_scene = main_scenes
            .iter()
            .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
            .unwrap()
            .clone();

        // Mutate the baseline scene to simulate a source-branch edit, and
        // synthesize a brand-new scene at (1,1,2) that will be INSERTed.
        let updated_scene = Scene {
            full_text: "source: gate held under siege.".into(),
            summary: "source 1.1.1".into(),
            ..baseline_scene.clone()
        };
        let new_scene = Scene {
            id: "scene:source-only-1-1-2".into(),
            scene_order: 2,
            full_text: "source: candle gutters.".into(),
            summary: "source 1.1.2".into(),
            ..baseline_scene.clone()
        };
        let source_scenes = vec![updated_scene, new_scene];

        // Source character_state at a fresh position (no existing row on
        // target at that position — so DELETE is a no-op and INSERT lands).
        let placement = StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(3),
            note: None,
        };
        let synthetic_state = CharacterState {
            id: "character_state:source-1".into(),
            project_id: project.id.clone(),
            branch_id: "bible_branch:other".into(), // will be re-stamped
            character_id: mara.id.clone(),
            scene_id: None,
            book_number: placement.book_number,
            chapter_number: placement.chapter_number,
            scene_order: placement.scene_order.unwrap(),
            emotional_state: Default::default(),
            goals: vec!["hold the gate".into()],
            status: vec!["resolute".into()],
            notes: Vec::new(),
            source_summary: Some("source-snapshot".into()),
            created_at: chrono::Utc::now(),
        };

        // Source relationship: same edge, but with different trust/tension.
        let source_rel = crate::sqlite::records::RelatesTo {
            in_id: mara.id.clone(),
            out_id: aldric.id.clone(),
            branch_id: "bible_branch:other".into(), // will be re-stamped
            relationship_type: "rival".into(),
            trust: 1,
            tension: 7,
            dynamics: vec!["fraught".into()],
            reason: Some("the gate quarrel".into()),
            last_scene_id: None,
            updated_at: chrono::Utc::now(),
        };

        // Source pacing tracker: same arc, but with different progress.
        let source_tracker = crate::sqlite::records::PacingTracker {
            id: "pacing_tracker:source-1".into(),
            project_id: project.id.clone(),
            branch_id: "bible_branch:other".into(), // will be re-stamped
            character_arc_id: arc.id.clone(),
            per_book_budget: Default::default(),
            max_progress_per_chapter: Some(0.5),
            milestone_spacing: Some(2),
            sprint_allowance: Some(1),
            regression_budget: Some(0.1),
            current_progress: 0.75,
            budget_remaining: 0.25,
            velocity: "fast".into(),
            status: "ahead".into(),
            next_milestone: Some("crisis".into()),
            warnings: vec!["watch the trust slide".into()],
            updated_at: chrono::Utc::now(),
        };

        // Execute the merge: target is the main branch.
        repo.merge_branch_snapshot(
            &project.id,
            &main_branch.id,
            &source_scenes,
            &[synthetic_state],
            &[source_rel],
            &[source_tracker],
        )
        .await
        .unwrap();

        // Verify: target main now has 2 scenes, the existing one was UPDATED,
        // the new one was INSERTed under main_branch.id.
        let after_scenes = repo
            .list_scenes_by_project_and_branch(&project.id, &main_branch.id)
            .await
            .unwrap();
        assert_eq!(after_scenes.len(), 2, "INSERT path lands new scene");
        let one_one_one = after_scenes
            .iter()
            .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
            .unwrap();
        assert_eq!(
            one_one_one.summary, "source 1.1.1",
            "UPDATE path overwrites"
        );
        assert_eq!(one_one_one.full_text, "source: gate held under siege.");
        let one_one_two = after_scenes
            .iter()
            .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 2))
            .unwrap();
        assert_eq!(one_one_two.summary, "source 1.1.2");
        assert_eq!(
            one_one_two.branch_id, main_branch.id,
            "INSERTed scene is stamped with target branch"
        );

        // character_state landed on target branch.
        let after_states = repo
            .list_character_states_by_project_and_branch(&project.id, &main_branch.id)
            .await
            .unwrap();
        let merged_state = after_states
            .iter()
            .find(|s| {
                s.character_id == mara.id
                    && (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 3)
            })
            .expect("merged character_state is present at (1,1,3)");
        assert_eq!(merged_state.branch_id, main_branch.id);
        assert_eq!(merged_state.goals, vec!["hold the gate".to_string()]);
        assert_eq!(merged_state.status, vec!["resolute".to_string()]);

        // relationship was overwritten with source's type/trust/tension.
        let merged_rel = repo
            .get_relationship(&main_branch.id, &mara.id, &aldric.id)
            .await
            .unwrap();
        assert_eq!(merged_rel.relationship_type, "rival");
        assert_eq!(merged_rel.trust, 1);
        assert_eq!(merged_rel.tension, 7);
        assert_eq!(merged_rel.reason.as_deref(), Some("the gate quarrel"));

        // pacing_tracker for the arc was replaced with source values.
        let merged_tracker = repo.get_pacing_tracker_by_arc(&arc.id).await.unwrap();
        assert_eq!(merged_tracker.branch_id, main_branch.id);
        assert_eq!(merged_tracker.status, "ahead");
        assert_eq!(merged_tracker.velocity, "fast");
        assert!((merged_tracker.current_progress - 0.75).abs() < f64::EPSILON);
        assert!((merged_tracker.budget_remaining - 0.25).abs() < f64::EPSILON);
        assert_eq!(
            merged_tracker.warnings,
            vec!["watch the trust slide".to_string()]
        );
        assert_ne!(
            merged_tracker.id, original_tracker.id,
            "DELETE-then-INSERT mints a fresh tracker id"
        );
    }

    /// Regression: `dump_project_table` must return one JSON object per row
    /// keyed by column name and reject anything that isn't a plain
    /// identifier. Also exercises the JSON-column re-parse path
    /// (`looks_like_json_column`) via the `dynamics` column on `relates_to`.
    #[tokio::test]
    async fn dump_project_table_emits_json_objects_keyed_by_column_name() {
        let (_tmp, repo) = fresh_repo().await;
        let input = CreateProjectInput {
            name: "DumpTarget".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        };
        let (project, _branch, _book, _chapter) = repo.create_project(&input).await.unwrap();

        // Project table: one row, with the expected scalar fields.
        let rows = repo
            .dump_table_by_column("project", "id", &project.id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        let row = rows[0].as_object().unwrap();
        assert_eq!(
            row.get("id").and_then(|v| v.as_str()),
            Some(project.id.as_str())
        );
        assert_eq!(row.get("name").and_then(|v| v.as_str()), Some("DumpTarget"));

        // bible_branch via dump_project_table: at least the main branch.
        let branches = repo
            .dump_project_table("bible_branch", &project.id)
            .await
            .unwrap();
        assert!(!branches.is_empty(), "main branch should be exported");
        assert!(
            branches
                .iter()
                .any(|b| b.get("name").and_then(|v| v.as_str()) == Some("main")),
        );

        // Unsafe identifier must be rejected.
        let err = repo
            .dump_project_table("project; DROP TABLE foo", &project.id)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("unsafe identifier"),
            "got error: {err}"
        );

        // dump_table_by_column_in returns empty for an empty key set without
        // running the query at all.
        let none = repo
            .dump_table_by_column_in("import_session", "session_id", &[])
            .await
            .unwrap();
        assert!(none.is_empty());
    }

    /// `restore_branch_snapshot` deletes the current branch content and
    /// re-inserts the snapshot rows in a single transaction.
    #[tokio::test]
    async fn restore_branch_snapshot_replaces_branch_content() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterInput,
        };

        let (_tmp, repo) = fresh_repo().await;
        let (project, main_branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "RestoreTarget".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let empty_voice = CharacterVoiceProfileData {
            tone: None,
            vocabulary: Vec::new(),
            sentence_structure: Vec::new(),
            tics: Vec::new(),
            forbidden_words: Vec::new(),
            example_lines: Vec::new(),
            established_in_scene_id: None,
            updated_at: None,
        };
        let empty_emotional = CharacterEmotionalProfileData {
            base_emotions: std::collections::BTreeMap::new(),
            suppressed: Vec::new(),
            triggers: Vec::new(),
            defense_mechanisms: Vec::new(),
            flex_range: None,
        };

        // Live state we'll wipe: one character on the active branch.
        repo.create_character(&CreateCharacterInput {
            project_id: project.id.clone(),
            name: "DoomedDeniz".into(),
            summary: "Will be wiped.".into(),
            role: "antagonist".into(),
            realm: None,
            voice_profile: empty_voice.clone(),
            emotional_profile: empty_emotional.clone(),
            initial_state: None,
        })
        .await
        .unwrap();
        let pre = repo
            .list_characters_by_project_and_branch(&project.id, &main_branch.id)
            .await
            .unwrap();
        assert_eq!(pre.len(), 1, "live character must exist before restore");

        // Snapshot rows: a hand-rolled BranchRestoreSnapshot carrying a
        // single replacement character. Mirrors what
        // `build_branch_restore_snapshot` would produce.
        let mut rows_by_table: std::collections::BTreeMap<
            String,
            Vec<serde_json::Map<String, Value>>,
        > = std::collections::BTreeMap::new();
        let now_us = chrono::Utc::now().timestamp_micros();
        let mut char_row = serde_json::Map::new();
        char_row.insert(
            "id".to_string(),
            Value::String("character:01HZRESTORE0000000000000001".to_string()),
        );
        char_row.insert("project_id".to_string(), Value::String(project.id.clone()));
        char_row.insert(
            "branch_id".to_string(),
            Value::String(main_branch.id.clone()),
        );
        char_row.insert("name".to_string(), Value::String("RestoredRei".to_string()));
        char_row.insert(
            "normalized_name".to_string(),
            Value::String("restoredrei".to_string()),
        );
        char_row.insert(
            "summary".to_string(),
            Value::String("From snapshot.".to_string()),
        );
        char_row.insert("role".to_string(), Value::String("protagonist".to_string()));
        char_row.insert("realm".to_string(), Value::Null);
        char_row.insert("notes".to_string(), Value::Null);
        char_row.insert("appearance".to_string(), Value::Null);
        char_row.insert(
            "created_at".to_string(),
            Value::Number(serde_json::Number::from(now_us)),
        );
        char_row.insert(
            "updated_at".to_string(),
            Value::Number(serde_json::Number::from(now_us)),
        );
        rows_by_table.insert("character".to_string(), vec![char_row]);

        let snapshot = BranchRestoreSnapshot { rows_by_table };
        repo.restore_branch_snapshot(&project.id, &main_branch.id, &snapshot)
            .await
            .unwrap();

        // Post-condition: live state has the snapshot character only.
        let post = repo
            .list_characters_by_project_and_branch(&project.id, &main_branch.id)
            .await
            .unwrap();
        assert_eq!(post.len(), 1);
        assert_eq!(post[0].name, "RestoredRei");
        assert_eq!(post[0].id, "character:01HZRESTORE0000000000000001");
        assert!(
            !post.iter().any(|c| c.name == "DoomedDeniz"),
            "pre-restore character must be deleted"
        );
    }

    #[tokio::test]
    async fn set_relationship_absolute_overwrites_trust_and_tension() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput, CreateRelationshipInput,
        };

        let (_tmp, repo) = fresh_repo().await;
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "RelAbs".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        async fn make_char(repo: &Repository, project_id: &str, name: &str) -> Character {
            let (character, _, _, _) = repo
                .create_character(&CreateCharacterInput {
                    project_id: project_id.to_string(),
                    name: name.to_string(),
                    summary: "x".into(),
                    role: "x".into(),
                    realm: None,
                    voice_profile: CharacterVoiceProfileData {
                        tone: None,
                        vocabulary: Vec::new(),
                        sentence_structure: Vec::new(),
                        tics: Vec::new(),
                        forbidden_words: Vec::new(),
                        example_lines: Vec::new(),
                        established_in_scene_id: None,
                        updated_at: None,
                    },
                    emotional_profile: CharacterEmotionalProfileData {
                        base_emotions: std::collections::BTreeMap::new(),
                        suppressed: Vec::new(),
                        triggers: Vec::new(),
                        defense_mechanisms: Vec::new(),
                        flex_range: None,
                    },
                    initial_state: Some(CharacterStatePatch {
                        emotional_state: std::collections::BTreeMap::new(),
                        goals: None,
                        status: None,
                        notes: None,
                        source_summary: None,
                    }),
                })
                .await
                .unwrap();
            character
        }
        let a = make_char(&repo, &project.id, "Aira").await;
        let b = make_char(&repo, &project.id, "Bren").await;

        // Seed with arbitrary starting numbers via create_relationship.
        repo.create_relationship(
            &branch.id,
            &CreateRelationshipInput {
                character_a_id: a.id.clone(),
                character_b_id: b.id.clone(),
                relationship_type: "ally".into(),
                initial_trust: 10,
                initial_tension: 90,
                dynamics: vec!["initial".into()],
            },
        )
        .await
        .unwrap();

        // Overwrite with absolutes (NOT deltas).
        let updated = repo
            .set_relationship_absolute(
                &branch.id,
                &a.id,
                &b.id,
                75,
                15,
                Some("import-canonical".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            updated.trust, 75,
            "trust must be the absolute new value, not 10+75"
        );
        assert_eq!(updated.tension, 15);
        assert_eq!(updated.reason.as_deref(), Some("import-canonical"));

        // Reversed orientation must still match the same row.
        let updated_reversed = repo
            .set_relationship_absolute(&branch.id, &b.id, &a.id, 50, 50, None, None)
            .await
            .unwrap();
        assert_eq!(updated_reversed.trust, 50);
        assert_eq!(updated_reversed.tension, 50);
    }
}
