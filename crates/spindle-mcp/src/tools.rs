//! MCP tool router for Spindle.
//!
//! ## Resource vs Tool Rule
//!
//! Spindle uses MCP resources and tools with a clear separation:
//!
//! **Resources (`bible://...`)** are for **stable, infrequently-changing reads** that benefit
//! from caching. They represent the canonical state of the Story Bible at a point in time.
//!
//! **Tools** are for **everything else**: state-changing operations, computations,
//! parameterized queries, and reads that require dynamic context or parameters.
//!
//! **Decision rule:** if an operation changes state, computes a result dynamically, or
//! requires parameters beyond simple project/entity IDs, use a tool. If it reads stable,
//! cacheable state, use a resource.
//!
//! ## Categorization
//!
//! ### Resource only
//!
//! These are available exclusively as `bible://` resources — no tool provides the same
//! read:
//!
//! | Resource URI | Description |
//! |---|---|
//! | `bible://skills/{name}` | Embedded skill markdown |
//! | `bible://references/{name}` | Craft reference markdown |
//! | `bible://system/model-routes` | Model route config |
//! | `bible://config/agents` | Sanitized agent config |
//! | `bible://config/routing` | Route assignment config |
//! | `bible://projects/{id}/chapters` | Chapters with scene spines |
//! | `bible://projects/{id}/characters` | Character list |
//! | `bible://projects/{id}/locations` | Location list |
//! | `bible://projects/{id}/world-rules` | World rule list |
//! | `bible://projects/{id}/factions` | Faction list |
//! | `bible://projects/{id}/plot-lines` | Plot line list |
//! | `bible://projects/{id}/conflicts` | Conflict list |
//! | `bible://projects/{id}/themes` | Theme list |
//! | `bible://projects/{id}/motifs` | Motif list |
//! | `bible://projects/{id}/narrative-promises` | Narrative promise list |
//! | `bible://projects/{id}/pacing/overview` | Pacing overview |
//! | `bible://projects/{id}/chapter-summaries` | Saved chapter summaries |
//! | `bible://projects/{id}/research-log` | Research log (paginated) |
//! | `bible://projects/{id}/reader-contract` | Reader contract |
//! | `bible://projects/{id}/branches` | Branch list (read-only; create_branch / switch_branch are write tools) |
//! | `bible://projects/{id}/continuity/health` | Active-branch continuity health summary |
//! | `bible://projects/{id}/future-knowledge` | Future knowledge list |
//! | `bible://projects/{id}/timeline-events` | Timeline event list |
//! | `bible://projects/{id}/timeline-graph/mermaid` | Branch/timeline Mermaid graph |
//! | `bible://projects/{id}/temporal-interventions` | Temporal intervention list |
//! | `bible://projects/{id}/system-overlays` | System overlay list |
//! | `bible://projects/{id}/dual-persona-reviews` | Dual persona review list |
//! | `bible://projects/{id}/relationships` | Relationship list |
//! | `bible://projects/{id}/character-arcs` | Character arc list |
//! | `bible://projects/{id}/religions` | Religion list |
//! | `bible://projects/{id}/economies` | Economy list |
//! | `bible://projects/{id}/terms` | Term list |
//! | `bible://projects/{id}/imports` | Import session list |
//! | `bible://projects/{id}/imports/{sid}/{path}` | Import session detail |
//! | `bible://{table}:{id}` | Direct entity lookup by record id |
//! | `bible://projects/{id}/scene-delete-impact/{b}/{c}/{s}` | Scene delete impact audit |
//! | `bible://projects/{id}/scene-move-impact/{from_book}/{from_chapter}/{from_scene}/{to_book}/{to_chapter}/{to_scene}` | Scene move impact audit |
//! | `bible://projects/{id}/research-log/{offset}/{limit}` | Research log page |
//!
//! ### Tool only
//!
//! Representative tool-only operations mutate state, compute dynamic results,
//! or require parameters that go beyond simple IDs:
//!
//! | Tool | Category |
//! |---|---|
//! | `create_project`, `create_book`, `create_chapter` | Write |
//! | `create_character`, `create_location`, `create_faction` | Write |
//! | `create_religion`, `create_economy`, `create_term` | Write |
//! | `create_relationship`, `create_world_rule` | Write |
//! | `create_plot_line`, `create_conflict`, `create_theme` | Write |
//! | `create_motif`, `create_narrative_promise` | Write |
//! | `create_character_arc`, `create_future_knowledge` | Write |
//! | `create_timeline_event`, `create_temporal_intervention` | Write |
//! | `create_system_overlay`, `create_pacing_config` | Write |
//! | `create_pacing_curve`, `set_arc_pacing_constraints` | Write |
//! | `update_entity`, `update_relationship`, `update_promise_status` | Write |
//! | `archive_entity` | Write |
//! | `save_scene_draft`, `commit_scene_changes` | Write |
//! | `commit_character_state`, `record_knowledge` | Write |
//! | `save_summary`, `register_canonical_fact`, `bind_canonical_fact_to_scene` | Write |
//! | `plan_chapter`, `annotate_scene_beats` | Write |
//! | `move_scene`, `delete_scene`, `operator_delete_scene` | Write |
//! | `create_branch`, `switch_branch` | Write |
//! | `create_save_point`, `restore_save_point` | Write |
//! | `diff_branches`, `merge_branch` | Write/Compute |
//! | `revise_scene`, `generate_alternatives`, `compare_alternatives`, `select_alternative` | Write/Compute |
//! | `list_revision_markers`, `resolve_revision_marker` | Read+Write |
//! | `import_manuscript`, `import_extract_entities` | Write |
//! | `import_consolidate_entities`, `import_analyze_character` | Write |
//! | `import_extract_world`, `import_analyze_narrative` | Write |
//! | `import_compute_final_state`, `import_hydrate_bible` | Write |
//! | `import_apply_review_decisions`, `import_status` | Read+Write |
//! | `run_dual_persona_review`, `check_consistency` | Compute |
//! | `search_bible`, `find_scenes_referencing` | Search (dynamic) |
//! | `rebuild_search_index`, `backfill_scene_source_offsets` | Compute |
//! | `configure_agents`, `test_agent`, `continue_generation`, `revise_generation` | Compute |
//! | `research_query` | Compute |
//! | `export_epub`, `preflight_book_export`, `export_bible` | Compute/Export |
//! | `list_scene_versions`, `restore_scene_version` | Read+Write |
//!
//! ### Both (resource + tool with different shapes)
//!
//! These exist as both a resource and a tool. The resource provides a cached, stable
//! read; the tool provides the same data (or a shaped subset) with explicit parameters.
//!
//! | Resource | Tool | Difference |
//! |---|---|---|
//! | `bible://projects` | `list_projects` | Same data; resource is cached, tool is dynamic |
//! | `bible://projects/{id}/books` | `list_book_chapters` | Resource lists all books; tool returns chapters for one book |
//! | `bible://projects/{id}/chapters/{b}/{c}/scenes` | `list_chapter_scenes` | Resource is cached, tool requires explicit project+chapter params |
//! | `bible://config/agents` | `list_agents` | Same data; resource is cached |
//! | `bible://skills/{name}` | `get_skill` | Same content; tool works for clients without resource support |
//! | `bible://references/{name}` | `get_reference` | Same content; tool works for clients without resource support |

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use rmcp::model::{CallToolResult, Content, Tool};
use rmcp::schemars;
use rmcp::schemars::generate::SchemaSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Number, Value};
use spindle_adapters::DraftRoutePreflightProblem;
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use spindle_core::models::*;
use spindle_core::style::*;
use spindle_core::subject_snapshot::SubjectSnapshot as EntitySubjectSnapshot;
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::json_utils::flatten_record_ids;
use crate::run_journal::{self, RunJournal};
use spindle_harness::artifacts::{
    ArtifactStore, ChapterSummaryArtifact, CheckpointReportArtifact,
    GeneratedChapterSummaryPackage, GeneratedScenePackage, SceneGenerationArtifact,
};
use spindle_harness::execution::{ExecutionResult, execute_one};
use spindle_harness::mcp::{McpHarnessClient, TransportConfig};
use spindle_harness::plan::{
    ChapterPlanSnapshot, ChapterSnapshot, NextAction, PersistedScene, PlannedSceneSnapshot,
    ProjectSnapshot, reconcile_state,
};
use spindle_harness::state::{CheckpointRecord, CheckpointStatus, HarnessState, ScenePhase};

#[derive(Debug, Clone, Default)]
struct SessionContext {
    active_project_id: Option<String>,
    active_branch_id: Option<String>,
}

#[derive(Clone)]
pub struct ToolRouter {
    service: SpindleService,
    session_context: Arc<RwLock<SessionContext>>,
    serialization_state: Arc<ToolSerializationState>,
    tool_profile: Option<String>,
}

#[derive(Clone, Default)]
pub struct ToolSerializationState {
    serialization_gate: Arc<RwLock<()>>,
    project_locks: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
}

enum ToolSerializationScope {
    Global,
    Project(String),
}

enum ToolSerializationGuard {
    Global {
        _gate: OwnedRwLockWriteGuard<()>,
    },
    Project {
        _project: OwnedMutexGuard<()>,
        _gate: OwnedRwLockReadGuard<()>,
    },
}

impl ToolRouter {
    pub fn with_tool_profile_and_serialization(
        service: SpindleService,
        tool_profile: Option<String>,
        serialization_state: Arc<ToolSerializationState>,
    ) -> Self {
        Self {
            service,
            session_context: Arc::new(RwLock::new(SessionContext::default())),
            serialization_state,
            tool_profile,
        }
    }

    /// Borrow the underlying service (test-only accessor for integration tests
    /// that need to read persisted state directly through the repository).
    #[cfg(test)]
    pub fn service(&self) -> &SpindleService {
        &self.service
    }

    pub fn list_tools(&self) -> Vec<Tool> {
        let all = self.all_tools();
        let Some(profile) = self.tool_profile.as_deref() else {
            return all;
        };
        let allowed: &[&str] = match profile {
            "import" => &[
                "create_project",
                "list_projects",
                "list_skills",
                "get_skill",
                "get_reference",
                "import_manuscript",
                "import_status",
                "import_extract_entities",
                "import_consolidate_entities",
                "import_analyze_character",
                "import_extract_world",
                "import_analyze_narrative",
                "import_compute_final_state",
                "import_hydrate_bible",
                "import_apply_review_decisions",
                "record_knowledge",
                "record_note",
                "update_writer_position",
                "search_bible",
                "find_scenes_referencing",
                "get_chapter_briefing",
                "create_style_profile_from_markdown",
                "list_style_profiles",
                "get_style_profile",
                "apply_style_profile",
                "preview_apply_style_profile",
                "list_style_profile_applications",
                "rollback_style_profile_application",
                "check_style_profile_sources",
                "preview_refresh_style_profile",
                "refresh_style_profile",
                "check_style_against_profile",
                "plan_style_revision",
                "compare_style_profiles",
                "archive_style_profile",
                "preview_style_revision_patch",
                "evaluate_style_revision_patch",
                "list_style_revision_patch_audits",
                "rollback_style_revision_patch",
            ],
            "write" => &[
                "get_editorial_queue",
                "decide_editorial_item",
                "read_episode",
                "get_model_usage",
                "get_series_status",
                "prepare_episode_release",
                "release_episode",
                "get_episode_release",
                "create_project",
                "list_projects",
                "list_skills",
                "get_skill",
                "get_reference",
                "create_book",
                "create_chapter",
                "create_character",
                "create_location",
                "create_faction",
                "create_religion",
                "create_economy",
                "create_term",
                "batch_create_terms",
                "create_relationship",
                "create_world_rule",
                "update_world_rule",
                "set_character_voice_profile",
                "batch_set_character_voice_profiles",
                "create_save_point",
                "restore_save_point",
                "create_plot_line",
                "create_conflict",
                "create_theme",
                "create_motif",
                "batch_create_motifs",
                "create_narrative_promise",
                "update_promise_status",
                "batch_create_narrative_promises",
                "create_character_arc",
                "update_arc_milestone",
                "create_system_overlay",
                "preflight_book_export",
                "compile_manuscript",
                "export_recap",
                "export_series_bible",
                "mine_scene_canon",
                "list_canon_deltas",
                "decide_canon_deltas",
                "replan_chapter",
                "list_plan_amendments",
                "decide_plan_amendments",
                "get_writer_state",
                "get_scene_context",
                "get_entity",
                "find_entity",
                "get_character_snapshot",
                "get_chapter_briefing",
                "list_chapter_scenes",
                "list_book_chapters",
                "save_scene_draft",
                "move_scene",
                "delete_scene",
                "operator_delete_scene",
                "commit_scene_changes",
                "commit_character_state",
                "update_relationship",
                "record_note",
                "update_writer_position",
                "save_summary",
                "plan_chapter",
                "annotate_scene_beats",
                "set_book_outline",
                "set_chapter_outline",
                "update_entity",
                "search_bible",
                "find_scenes_referencing",
                "check_consistency",
                "run_dual_persona_review",
                "backfill_scene_source_offsets",
                "register_canonical_fact",
                "extract_canonical_facts_from_scene",
                "migrate_canonical_fact",
                "bind_canonical_fact_to_scene",
                "research_query",
                "research_add_source",
                "research_add_note",
                "research_add_claim",
                "research_search",
                "research_pack_for_scene",
                "research_usage_for_scene",
                "research_ingest_report",
                "research_plan_for_scene",
                "pull_chapter_from_file",
                "push_chapter_to_file",
                "authoring_prepare_run",
                "authoring_start_run",
                "authoring_status",
                "authoring_execute_next",
                "authoring_save_scene_draft",
                "authoring_record_checkpoint_audit",
                "authoring_review_checkpoint",
                "authoring_resolve_block",
                "authoring_cancel_run",
                "create_style_profile_from_markdown",
                "list_style_profiles",
                "get_style_profile",
                "apply_style_profile",
                "preview_apply_style_profile",
                "list_style_profile_applications",
                "rollback_style_profile_application",
                "check_style_profile_sources",
                "preview_refresh_style_profile",
                "refresh_style_profile",
                "check_style_against_profile",
                "plan_style_revision",
                "compare_style_profiles",
                "archive_style_profile",
                "preview_style_revision_patch",
                "evaluate_style_revision_patch",
                "apply_style_revision_patch",
                "list_style_revision_patch_audits",
                "rollback_style_revision_patch",
            ],
            "minimal" => &[
                "create_project",
                "list_projects",
                "list_skills",
                "get_skill",
                "get_reference",
                "search_bible",
                "find_scenes_referencing",
                "get_writer_state",
                "get_scene_context",
                "get_entity",
                "find_entity",
                "get_character_snapshot",
                "list_chapter_scenes",
                "list_book_chapters",
                "save_scene_draft",
                "update_writer_position",
                "import_manuscript",
                "import_status",
                "import_hydrate_bible",
            ],
            "authoring" => &[
                "record_note",
                "update_arc_milestone",
                "create_character_arc",
                "create_motif",
                "create_theme",
                "create_conflict",
                "create_plot_line",
                "annotate_scene_beats",
                "save_scene_draft",
                "set_active_project",
                "get_editorial_queue",
                "decide_editorial_item",
                "read_episode",
                "get_series_status",
                "prepare_episode_release",
                "release_episode",
                "get_episode_release",
                "get_model_usage",
                "create_project",
                "list_projects",
                "list_skills",
                "get_skill",
                "get_reference",
                "create_book",
                "create_chapter",
                "create_character",
                "create_location",
                "create_world_rule",
                "create_narrative_promise",
                "update_promise_status",
                "get_writer_state",
                "get_entity",
                "find_entity",
                "search_bible",
                "get_chapter_briefing",
                "get_scene_context",
                "list_chapter_scenes",
                "list_book_chapters",
                "plan_chapter",
                "set_book_outline",
                "set_chapter_outline",
                "update_entity",
                "authoring_prepare_run",
                "authoring_start_run",
                "authoring_status",
                "authoring_execute_next",
                "authoring_save_scene_draft",
                "authoring_record_checkpoint_audit",
                "authoring_review_checkpoint",
                "authoring_resolve_block",
                "authoring_cancel_run",
                "commit_scene_changes",
                "save_summary",
                "check_consistency",
                "run_dual_persona_review",
                "mine_scene_canon",
                "list_canon_deltas",
                "decide_canon_deltas",
                "replan_chapter",
                "list_plan_amendments",
                "decide_plan_amendments",
                "preflight_book_export",
                "compile_manuscript",
                "export_recap",
                "export_series_bible",
                "create_save_point",
                "restore_save_point",
            ],
            _ => return all,
        };
        all.into_iter()
            .filter(|t| allowed.contains(&t.name.as_ref()))
            .collect()
    }

    fn all_tools(&self) -> Vec<Tool> {
        vec![
            tool::<GetEditorialQueueInput, EditorialQueueOutput>(
                "get_editorial_queue",
                "Review reader concerns with chapter evidence, source freshness and author decisions. Default shows unresolved work.",
            ),
            tool::<DecideEditorialItemInput, EditorialItem>(
                "decide_editorial_item",
                "Accept, defer, resolve, dismiss or reopen an editorial concern against the reviewed source and decision revision. Accepted work enters drafting guidance; never changes prose or canon.",
            ),
            tool::<ReadEpisodeInput, ReadEpisodeOutput>(
                "read_episode",
                "Read an episode as the promised reader, continuing source-validated memory across runs and books. Reuses unchanged reads and reports gaps and stale memory.",
            ),
            tool::<GetSeriesStatusInput, SeriesStatusOutput>(
                "get_series_status",
                "Inspect the published cursor, draft and ready backlog, episode revisions, and changes since release.",
            ),
            tool::<PrepareEpisodeReleaseInput, EpisodeReleasePreview>(
                "prepare_episode_release",
                "Preview one chapter as an episode. Reports missing/stub scenes and returns a source hash for a local immutable release.",
            ),
            tool::<ReleaseEpisodeInput, EpisodeRelease>(
                "release_episode",
                "Record an immutable local episode snapshot from an unchanged preview. Corrections append revisions. Does not post to any platform.",
            ),
            tool::<GetEpisodeReleaseInput, EpisodeRelease>(
                "get_episode_release",
                "Read the exact prose and source metadata of an immutable episode release, including the previous revision id.",
            ),
            tool::<GetModelUsageInput, GetModelUsageOutput>(
                "get_model_usage",
                "Inspect persisted model calls, reported tokens, elapsed time and unknown usage. No prose or credentials are logged.",
            ),
            tool::<CreateProjectInput, CreateProjectOutput>(
                "create_project",
                "Create a project, book 1, and chapter 1",
            ),
            tool::<EmptyInput, ListProjectsOutput>(
                "list_projects",
                "List all projects with their record ids",
            ),
            tool::<SetActiveProjectInput, SetActiveProjectOutput>(
                "set_active_project",
                "Persist the default project and branch for this MCP session so follow-up tool calls can omit project_id and use the current branch by default",
            ),
            tool::<CreateBookInput, CreateBookOutput>(
                "create_book",
                "Create the next book within a project",
            ),
            tool::<CreateChapterInput, CreateChapterOutput>(
                "create_chapter",
                "Create the next chapter within a book using book_number or book_id; optional chapter_number may be any unused non-negative number (gaps are allowed, e.g. for restructures) and defaults to one past the current maximum",
            ),
            tool::<CreateBranchInput, CreateBranchOutput>(
                "create_branch",
                "Create a new project branch from the active or specified parent branch",
            ),
            tool::<SwitchBranchInput, SwitchBranchOutput>(
                "switch_branch",
                "Switch a project's active branch",
            ),
            tool::<SetNarratorVoiceInput, SetNarratorVoiceOutput>(
                "set_narrator_voice",
                "Set or clear the project's narrator-voice directive — the prose-level narration style (comedy density, pacing feel, interiority ratio, emotional register, chapter-ending style) that governs the whole reading experience and is distinct from per-character dialogue voice profiles. Enforced across scene context, the save-draft gate, the style_compliance validator, and the review's Target Reader persona",
            ),
            tool::<SetProjectCalendarInput, SetProjectCalendarOutput>(
                "set_project_calendar",
                "Define the project's in-world calendar (days per week, hours per day, months, days per year, epoch). Enables the chronology check and the in-world-time hard constraint; validated for internal consistency.",
            ),
            tool::<SetSceneClockInput, SetSceneClockOutput>(
                "set_scene_clock",
                "Place a scene on the in-world story clock (day_index, time_of_day, duration_days, precision), optionally marking temporal_mode (linear|flashback|flashforward|concurrent) and a parallel thread_key. Drives the between-scene chronology check, the intra-scene temporal_coherence check (duration_days sets the expected span; temporal_mode/precision suppress it), and the in-world-time context constraint that feeds the prior scene's end clock forward.",
            ),
            tool::<SetTimelineEventClockInput, SetTimelineEventClockOutput>(
                "set_timeline_event_clock",
                "Set the in-world time a timeline event occurs (distinct from its manuscript placement) so flashbacks and time-skips are oriented by story time.",
            ),
            tool::<SetCharacterBirthInput, SetCharacterBirthOutput>(
                "set_character_birth",
                "Anchor a character's birth on the story clock so their age can be derived at any story moment. day_index is a signed offset from the story's opening day, so births before the epoch are negative (e.g. -8913 for a character born ~24 years before day 1).",
            ),
            tool::<SetProjectQuantitySchemeInput, SetProjectQuantitySchemeOutput>(
                "set_project_quantity_scheme",
                "Declare a per-project quantity scheme for a measure (e.g. wealth, mana, a cultivation ladder): ordered currency denominations and ordered bands/tiers, plus an optional max_band_jump. Validated for internal consistency; fully opt-in. Money and progression systems reuse the same primitive.",
            ),
            tool::<CommitQuantityStateInput, CommitQuantityStateOutput>(
                "commit_quantity_state",
                "Record a stamped quantity reading for a subject's measure at a story position (band is the primary signal; amount/unit are optional). A change_reason marks a deliberate large jump as legitimate. Validated against the declared scheme when one exists. Manual path; authoring runs with a mining_policy stage quantity_change deltas that apply through this tool after ratification via decide_canon_deltas.",
            ),
            tool::<DeriveQuantitySchemeFromOverlayInput, DeriveQuantitySchemeFromOverlayOutput>(
                "derive_quantity_scheme_from_system_overlay",
                "Derive a quantity scheme from a system overlay so LitRPG / cultivation progression reuses band-monotonicity: the overlay's advancement_tiers become the scheme's ordered bands (measure = its progression_currency, else its name).",
            ),
            tool::<ScanScenePricesInput, ScanScenePricesOutput>(
                "scan_scene_prices",
                "Scan a scene's prose for '<number> <unit>' price mentions using the project's declared denominations + economy currencies (review-gated; nothing is auto-registered). Use the results to seed price-canon facts.",
            ),
            tool::<CreateSavePointInput, CreateSavePointOutput>(
                "create_save_point",
                "Create a save point on the active branch",
            ),
            tool::<RestoreSavePointInput, RestoreSavePointOutput>(
                "restore_save_point",
                "Restore the active branch from a save point snapshot",
            ),
            tool::<DiffBranchesInput, DiffBranchesOutput>(
                "diff_branches",
                "Compare two project branches across scenes, states, relationships, and pacing",
            ),
            tool::<CreateStyleProfileFromMarkdownInput, CreateStyleProfileFromMarkdownOutput>(
                "create_style_profile_from_markdown",
                "Derive a reusable project style profile from user-provided local Markdown files or folders. Persists metadata, hashes, metrics, and generated guidance, but no source text by default. The guidance is synthesized by the routed style_analyze model against an explicit schema; if the model returns no usable guidance the call fails loudly and creates no profile (use metrics_only=true to create a metrics-only profile deliberately).",
            ),
            tool::<CheckStyleProfileSourcesInput, CheckStyleProfileSourcesOutput>(
                "check_style_profile_sources",
                "Check the staleness of source files for a derived style profile compared to disk. Exposes metadata only.",
            ),
            tool::<PreviewRefreshStyleProfileInput, PreviewRefreshStyleProfileOutput>(
                "preview_refresh_style_profile",
                "Preview rebuilding/refreshing a style profile from source files without persisting. Exposes metadata only.",
            ),
            tool::<RefreshStyleProfileInput, RefreshStyleProfileOutput>(
                "refresh_style_profile",
                "Refresh/rebuild a style profile from current local sources, saving a new version linked to the parent.",
            ),
            tool::<ListStyleProfilesInput, ListStyleProfilesOutput>(
                "list_style_profiles",
                "List derived style profiles saved for a project.",
            ),
            tool::<GetStyleProfileInput, GetStyleProfileOutput>(
                "get_style_profile",
                "Retrieve a specific derived style profile by its profile_id.",
            ),
            tool::<ApplyStyleProfileInput, ApplyStyleProfileOutput>(
                "apply_style_profile",
                "Apply a derived style profile's guidance to a project's existing style contract surfaces (NarratorVoice, ReaderContract.style_notes, and style world rules), invalidating style-sensitive validator cache rows. Requires the profile to be ready and to carry application guidance; pass force=true to deliberately activate a NeedsReview or metrics-only profile — with no prose guidance, only the profile is activated and the project's narrator voice/style notes are left untouched.",
            ),
            tool::<PreviewApplyStyleProfileInput, PreviewApplyStyleProfileOutput>(
                "preview_apply_style_profile",
                "Preview applying a derived style profile's changes without mutating state.",
            ),
            tool::<ListStyleProfileApplicationsInput, ListStyleProfileApplicationsOutput>(
                "list_style_profile_applications",
                "List the history of applied style profiles and their rollback status.",
            ),
            tool::<RollbackStyleProfileApplicationInput, RollbackStyleProfileApplicationOutput>(
                "rollback_style_profile_application",
                "Rollback a previously applied style profile, reverting narrator voice, style notes, and conservatively reverting style world rules.",
            ),
            tool::<CheckStyleAgainstProfileInput, CheckStyleAgainstProfileOutput>(
                "check_style_against_profile",
                "Check a scene or raw text for style drift against a derived style profile.",
            ),
            tool::<PlanStyleRevisionInput, PlanStyleRevisionOutput>(
                "plan_style_revision",
                "Generate a non-mutating style revision plan for a target (scene, chapter, or raw text) against a derived style profile.",
            ),
            tool::<CompareStyleProfilesInput, CompareStyleProfilesOutput>(
                "compare_style_profiles",
                "Compare two derived style profiles to report metric deltas, guidance differences, and whether applying the second profile would likely change the project style materially.",
            ),
            tool::<ArchiveStyleProfileInput, ArchiveStyleProfileOutput>(
                "archive_style_profile",
                "Safer profile deletion/archiving. archived profiles cannot be active or default, but preserve audit history.",
            ),
            tool::<PreviewStyleRevisionPatchInput, PreviewStyleRevisionPatchOutput>(
                "preview_style_revision_patch",
                "Preview a style revision patch for a scene or chapter against a style profile, returning proposed revised text and unified diffs without persisting changes.",
            ),
            tool::<EvaluateStyleRevisionPatchInput, EvaluateStyleRevisionPatchOutput>(
                "evaluate_style_revision_patch",
                "Evaluate a proposed style revision patch for alignment improvement and risks without persisting changes.",
            ),
            tool::<ApplyStyleRevisionPatchInput, ApplyStyleRevisionPatchOutput>(
                "apply_style_revision_patch",
                "Apply proposed style revision patches to target scenes, saving drafts through the standard writing pipeline, invalidating caches, and writing an audit trail.",
            ),
            tool::<ListStyleRevisionPatchAuditsInput, ListStyleRevisionPatchAuditsOutput>(
                "list_style_revision_patch_audits",
                "List style revision patch audits for a project.",
            ),
            tool::<RollbackStyleRevisionPatchInput, RollbackStyleRevisionPatchOutput>(
                "rollback_style_revision_patch",
                "Roll back an applied style revision patch, restoring prior prose versions using scene version history.",
            ),
            tool::<MergeBranchInput, MergeBranchOutput>(
                "merge_branch",
                "Merge a source branch into a target branch for branch-aware story records",
            ),
            tool::<ReviseSceneInput, ReviseSceneOutput>(
                "revise_scene",
                "Revise a scene on a non-main branch and flag downstream invalidation impact",
            ),
            tool::<GenerateAlternativesInput, GenerateAlternativesOutput>(
                "generate_alternatives",
                "Generate branch-backed alternative scene drafts from shared context",
            ),
            tool::<CompareAlternativesInput, CompareAlternativesOutput>(
                "compare_alternatives",
                "Compare generated alternative branches and rank them heuristically",
            ),
            tool::<SelectAlternativeInput, SelectAlternativeOutput>(
                "select_alternative",
                "Select an alternative branch and merge it into main",
            ),
            tool::<ListRevisionMarkersInput, ListRevisionMarkersOutput>(
                "list_revision_markers",
                "List persisted revision markers for a scene on the active branch",
            ),
            tool::<ResolveRevisionMarkerInput, ResolveRevisionMarkerOutput>(
                "resolve_revision_marker",
                "Mark a persisted revision marker as resolved",
            ),
            tool::<CreateCharacterInput, CreateCharacterOutput>(
                "create_character",
                "Create a character with profiles and baseline state",
            ),
            tool::<CreateLocationInput, CreateLocationOutput>(
                "create_location",
                "Create a location with optional initial world state; accepts type as an alias for kind and infers a kind when omitted",
            ),
            tool::<CreateFactionInput, CreateFactionOutput>(
                "create_faction",
                "Create a faction entity",
            ),
            tool::<CreateReligionInput, CreateReligionOutput>(
                "create_religion",
                "Create a religion entity",
            ),
            tool::<CreateEconomyInput, CreateEconomyOutput>(
                "create_economy",
                "Create an economy entity",
            ),
            tool::<CreateTermInput, CreateTermOutput>("create_term", "Create a glossary term"),
            tool::<BatchCreateTermsInput, BatchCreateTermsOutput>(
                "batch_create_terms",
                "Create multiple glossary terms in one call",
            ),
            tool::<CreateRelationshipInput, CreateRelationshipOutput>(
                "create_relationship",
                "Create a directed relationship between two characters",
            ),
            tool::<CreateWorldRuleInput, CreateWorldRuleOutput>(
                "create_world_rule",
                "Create a project world rule",
            ),
            tool::<UpdateWorldRuleInput, UpdateWorldRuleOutput>(
                "update_world_rule",
                "Update a world rule by record id",
            ),
            tool::<BatchSetCharacterVoiceProfilesInput, BatchSetCharacterVoiceProfilesOutput>(
                "batch_set_character_voice_profiles",
                "Replace multiple character voice profiles in one call",
            ),
            tool::<UpdateEntityInput, UpdateEntityOutput>(
                "update_entity",
                "Update a supported entity by record id. Character aliases are settable via changes {\"aliases\": [...]}. Renaming a character (changes {\"name\": ...}) is a controlled operation: pass allow_rename: true to move the name's uniqueness key, keep the old name as an alias, refresh the search index, and get a rename_report listing scenes/facts/knowledge/arcs still referencing the old name. Without allow_rename the rename is rejected. For entity_type=project, the reader contract is updatable via changes {\"reader_contract\": {...}} or the sub-fields {\"promise\", \"style_notes\", \"boundaries\"} (partial merge; unset fields are preserved); if the edit leaves the contract's word-count target contradicting the narrator voice or a style world rule, the response warnings flag it",
            ),
            tool::<ArchiveEntityInput, ArchiveEntityOutput>(
                "archive_entity",
                "Archive a supported entity by record id",
            ),
            tool::<CreatePlotLineInput, CreatePlotLineOutput>(
                "create_plot_line",
                "Create a plot line",
            ),
            tool::<CreateConflictInput, CreateConflictOutput>(
                "create_conflict",
                "Create a conflict record",
            ),
            tool::<CreateThemeInput, CreateThemeOutput>("create_theme", "Create a theme record"),
            tool::<CreateMotifInput, CreateMotifOutput>("create_motif", "Create a motif record"),
            tool::<BatchCreateMotifsInput, BatchCreateMotifsOutput>(
                "batch_create_motifs",
                "Create multiple motif records in one call",
            ),
            tool::<CreateNarrativePromiseInput, CreateNarrativePromiseOutput>(
                "create_narrative_promise",
                "Create a narrative promise",
            ),
            tool::<BatchCreateNarrativePromisesInput, BatchCreateNarrativePromisesOutput>(
                "batch_create_narrative_promises",
                "Create multiple narrative promises in one call",
            ),
            tool::<UpdatePromiseStatusInput, UpdatePromiseStatusOutput>(
                "update_promise_status",
                "Advance a narrative promise lifecycle. Manual path; authoring runs with a mining_policy stage promise_payoff_candidate/promise_reinforced deltas that apply through this tool after ratification via decide_canon_deltas.",
            ),
            tool::<CreateCharacterArcInput, CreateCharacterArcOutput>(
                "create_character_arc",
                "Create a character arc and pacing tracker",
            ),
            tool::<UpdateArcMilestoneInput, UpdateArcMilestoneOutput>(
                "update_arc_milestone",
                "Update a single character-arc milestone by label: move its placement and/or stamp reached_at (e.g. after a chapter renumber or when the milestone lands). Only the supplied fields change; description and unlocks are preserved. Read milestones via get_entity(table=\"character_arc\") or get_character_snapshot",
            ),
            tool::<CreateFutureKnowledgeInput, CreateFutureKnowledgeOutput>(
                "create_future_knowledge",
                "Record future knowledge held by a character",
            ),
            tool::<CreateTimelineEventInput, CreateTimelineEventOutput>(
                "create_timeline_event",
                "Record a timeline event for time-aware stories",
            ),
            tool::<CreateTemporalInterventionInput, CreateTemporalInterventionOutput>(
                "create_temporal_intervention",
                "Track a time-travel intervention between timeline events",
            ),
            tool::<CreateSystemOverlayInput, CreateSystemOverlayOutput>(
                "create_system_overlay",
                "Create a LitRPG or cultivation system overlay",
            ),
            tool::<RunDualPersonaReviewInput, RunDualPersonaReviewOutput>(
                "run_dual_persona_review",
                "Run a literary and craft review loop for a branch scene",
            ),
            tool::<CreatePacingConfigInput, CreatePacingConfigOutput>(
                "create_pacing_config",
                "Create pacing configuration for a project",
            ),
            tool::<CreatePacingCurveInput, CreatePacingCurveOutput>(
                "create_pacing_curve",
                "Create pacing curve for a book",
            ),
            tool::<SetArcPacingConstraintsInput, SetArcPacingConstraintsOutput>(
                "set_arc_pacing_constraints",
                "Set pacing constraints for a character arc",
            ),
            tool::<PlanChapterInput, PlanChapterOutput>("plan_chapter", "Create a chapter plan"),
            tool::<AnnotateSceneBeatsInput, AnnotateSceneBeatsOutput>(
                "annotate_scene_beats",
                "Annotate structural beats for a scene. Manual path; authoring runs with a mining_policy stage beat_annotation deltas that apply through this tool after ratification via decide_canon_deltas.",
            ),
            tool::<SaveSummaryInput, SaveSummaryOutput>(
                "save_summary",
                "Save a chapter summary using chapter entity_id/chapter_id or explicit book_number and chapter_number",
            ),
            tool::<SetBookOutlineInput, SetBookOutlineOutput>(
                "set_book_outline",
                "Set or replace a book outline on the active branch using book_id or book_number",
            ),
            tool::<SetChapterOutlineInput, SetChapterOutlineOutput>(
                "set_chapter_outline",
                "Set or replace a chapter outline on the active branch using chapter_id/entity_id or explicit book_number and chapter_number",
            ),
            tool::<CheckConsistencyInput, CheckConsistencyOutput>(
                "check_consistency",
                "Run a structured consistency audit (includes scene_divergence and canonical_fact_consistency checks)",
            ),
            tool::<RegisterCanonicalFactInput, RegisterCanonicalFactOutput>(
                "register_canonical_fact",
                "Register a canonical story fact (pull result, stat change, item, ability) for contradiction detection. scene_id is optional: omit it to register a fact decided during planning but not yet dramatised (placed by book_number/chapter_number only), then attach it later with bind_canonical_fact_to_scene",
            ),
            tool::<ExtractCanonicalFactsFromSceneInput, ExtractCanonicalFactsFromSceneOutput>(
                "extract_canonical_facts_from_scene",
                "Extract proposed typed canonical facts from committed scene prose without registering them",
            ),
            tool::<MigrateCanonicalFactInput, MigrateCanonicalFactOutput>(
                "migrate_canonical_fact",
                "Promote a legacy_untyped canonical fact into a typed canonical fact and supersede the legacy row",
            ),
            tool::<BindCanonicalFactToSceneInput, BindCanonicalFactToSceneOutput>(
                "bind_canonical_fact_to_scene",
                "Attach a planned-and-pending canonical fact (registered without a scene_id) to the scene that dramatises it. The fact and scene must share a project and branch; placement is left untouched",
            ),
            tool::<SearchBibleInput, SearchBibleOutput>(
                "search_bible",
                "Search project records by meaning, exact text, or fuzzy match",
            ),
            tool::<FindScenesReferencingInput, FindScenesReferencingOutput>(
                "find_scenes_referencing",
                "Find up to 100 active-branch scenes that reference a subject record id or literal phrase",
            ),
            tool::<RebuildSearchIndexInput, RebuildSearchIndexOutput>(
                "rebuild_search_index",
                "Rebuild semantic search embeddings for a project",
            ),
            tool::<BackfillSceneSourceOffsetsInput, BackfillSceneSourceOffsetsOutput>(
                "backfill_scene_source_offsets",
                "Recompute scene_source_link offsets with the import slicer for one project branch",
            ),
            tool::<PullChapterFromFileInput, PullReport>(
                "pull_chapter_from_file",
                "Import chapter scene text from a source file into active-branch scenes",
            ),
            tool::<PushChapterToFileInput, PushReport>(
                "push_chapter_to_file",
                "Export active-branch chapter scene text to a source file and store source offsets",
            ),
            tool::<ConfigureAgentsInput, ConfigureAgentsOutput>(
                "configure_agents",
                "Reload model agent and route configuration from spindle.toml",
            ),
            tool::<EmptyInput, ListAgentsOutput>(
                "list_agents",
                "List configured model agents and their route assignments",
            ),
            tool::<InitGrokSkillsInput, InitGrokSkillsOutput>(
                "init_grok_skills",
                "Initialize Grok-compatible Spindle skill adapters. By default installs into ~/.grok/skills/ (global). Pass global=false + target_dir if you want repo-scoped adapters instead. This makes all bible://skills/* (scene-writer, character-creator, etc.) work as first-class skills in Grok.",
            ),
            tool::<EmptyInput, ListSkillsOutput>(
                "list_skills",
                "List the embedded Spindle workflow skills and craft references. Tool equivalent of browsing bible://skills/* and bible://references/*; use get_skill / get_reference to read one.",
            ),
            tool::<GetSkillInput, GetSkillOutput>(
                "get_skill",
                "Return the full content of an embedded Spindle skill. Tool equivalent of reading bible://skills/{name}; call this to load a workflow such as scene-writer or character-creator.",
            ),
            tool::<GetReferenceInput, GetReferenceOutput>(
                "get_reference",
                "Return the full content of an embedded craft reference. Tool equivalent of reading bible://references/{name}.",
            ),
            tool::<TestAgentInput, TestAgentOutput>(
                "test_agent",
                "Send a test prompt through a configured model agent",
            ),
            tool::<ContinueGenerationInput, ContinueGenerationOutput>(
                "continue_generation",
                "Continue a model generation and return a server-side generation receipt",
            ),
            tool::<ReviseGenerationInput, ReviseGenerationOutput>(
                "revise_generation",
                "Revise a server-side generation through the same explicit-capable route and return a new receipt",
            ),
            tool::<ResearchQueryInput, ResearchQueryOutput>(
                "research_query",
                "Research a factual question using Gemini, grounded in project context from the Bible",
            ),
            tool::<ResearchAddSourceInput, ResearchAddSourceOutput>(
                "research_add_source",
                "Create new research source metadata and optional summary in the project-local library",
            ),
            tool::<ResearchAddNoteInput, ResearchAddNoteOutput>(
                "research_add_note",
                "Attach a freeform note and optional quote/locator to a research source",
            ),
            tool::<ResearchAddClaimInput, ResearchAddClaimOutput>(
                "research_add_claim",
                "Store a distilled factual claim with confidence, topic, location, time period, and tags",
            ),
            tool::<ResearchSearchInput, ResearchSearchOutput>(
                "research_search",
                "Search the project-local research library for matching sources, notes, and claims",
            ),
            tool::<ResearchPackForSceneInput, ResearchPackForSceneOutput>(
                "research_pack_for_scene",
                "Retrieve a compact, budget-aware packet of relevant research for a scene or chapter",
            ),
            tool::<ResearchUsageForSceneInput, ResearchUsageForSceneOutput>(
                "research_usage_for_scene",
                "Retrieve the recorded research usage history for a drafted scene",
            ),
            tool::<ResearchIngestReportInput, ResearchIngestReportOutput>(
                "research_ingest_report",
                "Ingest and parse a text-based research report, creating sources, notes, and claims in the project-local database",
            ),
            tool::<ResearchPlanForSceneInput, ResearchPlanForSceneOutput>(
                "research_plan_for_scene",
                "Determine missing research, suggest queries, and check if a scene is blocked on research",
            ),
            tool::<ExportEpubInput, ExportEpubOutput>(
                "export_epub",
                "Export a project, single book, or inclusive chapter range within a book as an EPUB file",
            ),
            tool::<PreflightBookExportInput, PreflightBookExportOutput>(
                "preflight_book_export",
                "Validate a project, single book, or inclusive chapter range within a book for EPUB export and return blocking issues or warnings before writing a file",
            ),
            tool::<CompileManuscriptInput, CompileManuscriptOutput>(
                "compile_manuscript",
                "Assemble the committed prose of a book (or an inclusive chapter range within it) on the active branch into one Markdown read-so-far document, with per-chapter and per-scene headings; planned-but-undrafted scenes render an explicit placeholder and are reported in missing_scenes. Optionally writes the Markdown to the project's workspace artifacts directory.",
            ),
            tool::<ExportRecapInput, ExportRecapOutput>(
                "export_recap",
                "Assemble a spoiler-bounded, reader-facing \"previously on\" recap of a book up to (and including) through_chapter on the active branch: a story-so-far section from chapter summaries at/under the cursor, a \"paid off\" section of resolved promises, and a \"questions still hanging\" section of open promises. Pure read model — no model calls. A secret canonical fact (and any line naming it) is withheld unless a reader-visible reveal has been placed at or before the cursor. Optionally writes the Markdown to the project's workspace artifacts directory.",
            ),
            tool::<ExportSeriesBibleInput, ExportSeriesBibleOutput>(
                "export_series_bible",
                "Assemble a spoiler-bounded, reader-facing series bible on the active branch as of an optional cursor (through, absent = whole project): character pages with state as-of the cursor and relationship bands, locations, a sorted glossary of terms, and factions/religions when present. Pure read model — no model calls. A secret canonical fact (and any line naming it) is withheld unless a reader-visible reveal has been placed at or before the cursor. Optionally writes the Markdown to the project's workspace artifacts directory.",
            ),
            tool::<MineSceneCanonInput, MineSceneCanonOutput>(
                "mine_scene_canon",
                "Mine one committed scene's prose into proposed canon deltas (staged for operator ratification, never auto-applied). One rating-gated model call; malformed model output is rejected and evidence must appear verbatim in the prose. Re-mining a scene supersedes its prior staged deltas. Returns the staged deltas, discard/supersede counts, and a status of staged, skipped (no cleared route or empty scene), or model_output_rejected.",
            ),
            tool::<ListCanonDeltasInput, ListCanonDeltasOutput>(
                "list_canon_deltas",
                "List the canon deltas mined for a project's active branch (the ratify queue). Filter by status (staged | applied | rejected | superseded) and by provenance — either one scene_id OR a chapter_range { book_number, start, end } (mutually exclusive). Results carry each delta's class, payload, verbatim evidence quote, confidence, and status, in deterministic order. Read this before deciding — never bulk-apply without reading the evidence.",
            ),
            tool::<DecideCanonDeltasInput, DecideCanonDeltasOutput>(
                "decide_canon_deltas",
                "Ratify a batch of staged canon deltas: each decision is apply or reject, with an optional edit (a corrected payload applied AND recorded) and an optional operator note (echoed in the output; not persisted). ALL decisions are pre-flighted before ANY write — a bad payload, dangling target, or a decision on an already-decided row (decisions are final) aborts the whole call with zero writes. Applies dispatch to the class's existing write tool (register_canonical_fact, update_promise_status, update_relationship, commit_character_state, record_knowledge, annotate_scene_beats, commit_quantity_state, update_entity for conflict/arc columns, or create_character/location/term). Returns a per-decision outcome (applied | rejected | failed | not_reached) with any created record id.",
            ),
            tool::<ReplanChapterInput, ReplanChapterOutput>(
                "replan_chapter",
                "Audit a book's not-yet-drafted chapter plans against the realized reality one just-summarized chapter established, and stage plan-amendment proposals for operator ratification (never auto-applied — ADR 0003 D5). Non-prose-bearing: the differ reads summaries + metadata only (no scene prose), so no rating clearance applies; on a missing route it falls to review then skips honestly, and it never blocks the run. Re-running for the same source chapter supersedes its prior staged amendments. Returns the staged amendments, discard/supersede counts, and a status of staged, no_summary, no_targets, skipped, or model_output_rejected.",
            ),
            tool::<ListPlanAmendmentsInput, ListPlanAmendmentsOutput>(
                "list_plan_amendments",
                "List the plan amendments staged for a project's active branch (the living-outline ratify queue). Filter by status (staged | applied | rejected | superseded) and by provenance (book_number + source_chapter together — the summarized chapter that triggered a replan pass). Each amendment carries its class, the future target_chapter (null for promise_followup), the typed payload, the replanner's rationale (ids/summaries, no prose), confidence, status, and any prior_state snapshot, in deterministic order. Read this — and the rationale — before deciding; never bulk-apply blind.",
            ),
            tool::<DecidePlanAmendmentsInput, DecidePlanAmendmentsOutput>(
                "decide_plan_amendments",
                "Ratify a batch of staged plan amendments (the living outline). Each decision is apply or reject, with an optional edit (a corrected payload applied AND recorded) and an optional operator note (echoed, not persisted). ALL decisions are pre-flighted before ANY write — a bad payload, a target chapter that already has drafted scenes (the immutability guard: drafted reality is never rewritten), a dangling scene_order/incoherent reorder, or a decision on an already-decided row (decisions are final) aborts the whole call with zero writes. Applies replay through the existing plan write path (plan_chapter for the chapter classes, create_narrative_promise for promise_followup), snapshot the prior plan into prior_state, and bump plan_revision. Returns a per-decision outcome (applied | rejected | failed | not_reached) with any created record id.",
            ),
            tool::<ExportBibleInput, ExportBibleOutput>(
                "export_bible",
                "Export a full project backup as JSON, including branch data",
            ),
            tool::<GetWriterStateInput, WriterStateEnvelope>(
                "get_writer_state",
                "Return a branch-aware re-anchor packet with current cursor state, constraints, subjects, recent scenes, overlays, divergence warnings, and recent activity",
            ),
            tool::<GetSceneContextInput, SceneContextEnvelope>(
                "get_scene_context",
                "Assemble standards, novel, and scene context using chapter_id or explicit book_number and chapter_number",
            ),
            tool::<GetEntityInput, EntitySubjectSnapshot>(
                "get_entity",
                "Resolve one entity by table and record id as a polymorphic subject snapshot",
            ),
            tool::<FindEntityInput, FindEntityOutput>(
                "find_entity",
                "Resolve entities by name or alias with ExactName/SemanticMatch confidence",
            ),
            tool::<GetCharacterSnapshotInput, CharacterSnapshotOutput>(
                "get_character_snapshot",
                "Resolve one character snapshot and promote voice profile, current state, and recent appearances",
            ),
            tool::<SetCharacterVoiceProfileInput, SetCharacterVoiceProfileOutput>(
                "set_character_voice_profile",
                "Set a character voice profile and append a session activity entry summarizing the change",
            ),
            tool::<GetChapterBriefingInput, GetChapterBriefingOutput>(
                "get_chapter_briefing",
                "Assemble a compact pre-write briefing with continuity sheets, recent summaries, chapter plans, and lean scene context",
            ),
            tool::<ListChapterScenesInput, ListChapterScenesOutput>(
                "list_chapter_scenes",
                "List the active-branch scenes in one chapter with canonical order, summary-first-line, word count, and canonical-fact flags",
            ),
            tool::<ListBookChaptersInput, ListBookChaptersOutput>(
                "list_book_chapters",
                "List the active-branch chapters in one book with nested ordered scene spines",
            ),
            tool::<SaveSceneDraftInput, SaveSceneDraftOutput>(
                "save_scene_draft",
                "Create or update a scene draft using chapter_id or explicit book_number and chapter_number; accepts content as an alias for full_text",
            ),
            tool::<MoveSceneInput, MoveSceneOutput>(
                "move_scene",
                "Move an active-branch scene only when its move audit is clear; leaves a gap at the source position",
            ),
            tool::<DeleteSceneInput, DeleteSceneOutput>(
                "delete_scene",
                "Delete an active-branch scene only when its dependency audit is clear; leaves a gap in scene_order. Address by scene_id (preferred — unambiguous even when duplicates or orphans share a position) or by book_number/chapter_number/scene_order",
            ),
            tool::<OperatorDeleteSceneInput, OperatorDeleteSceneOutput>(
                "operator_delete_scene",
                "Delete an active-branch scene after removing scene_source_link records and invalidating stale chapter_plan/chapter_summary artifacts, but only when no other blockers or semantic risks remain. Accepts scene_id for unambiguous addressing, like delete_scene",
            ),
            tool::<ListSceneVersionsInput, ListSceneVersionsOutput>(
                "list_scene_versions",
                "List saved historical versions for a scene",
            ),
            tool::<RestoreSceneVersionInput, RestoreSceneVersionOutput>(
                "restore_scene_version",
                "Restore a scene from one of its saved historical versions",
            ),
            tool::<CommitSceneChangesInput, CommitSceneChangesOutput>(
                "commit_scene_changes",
                "Best-effort batch commit of scene character states, canonical facts, and relationship updates; accepts shorthand summary entries for state notes, canonical fact summaries, and relationship summaries",
            ),
            tool::<CommitCharacterStateInput, CommitCharacterStateOutput>(
                "commit_character_state",
                "Append a character state snapshot from a saved scene. Manual path; authoring runs with a mining_policy stage character_state deltas that apply through this tool after ratification via decide_canon_deltas.",
            ),
            tool::<UpdateRelationshipInput, UpdateRelationshipOutput>(
                "update_relationship",
                "Update trust and tension for one directed relationship. Manual path; authoring runs with a mining_policy stage relationship_shift deltas that apply through this tool after ratification via decide_canon_deltas.",
            ),
            tool::<ImportManuscriptInput, ImportManuscriptOutput>(
                "import_manuscript",
                "Create an import session and persist normalized manuscript structure",
            ),
            tool::<ImportStatusInput, ImportStatusOutput>(
                "import_status",
                "Read the current state of an import session",
            ),
            tool::<ImportExtractEntitiesInput, ImportExtractEntitiesOutput>(
                "import_extract_entities",
                "Extract candidate entities from imported manuscript scenes",
            ),
            tool::<ImportConsolidateEntitiesInput, ImportConsolidateEntitiesOutput>(
                "import_consolidate_entities",
                "Consolidate imported entity mentions into canonical clusters",
            ),
            tool::<ImportAnalyzeCharacterInput, ImportAnalyzeCharacterOutput>(
                "import_analyze_character",
                "Build imported character dossiers from consolidated clusters",
            ),
            tool::<ImportExtractWorldInput, ImportExtractWorldOutput>(
                "import_extract_world",
                "Build imported world dossier candidates",
            ),
            tool::<ImportAnalyzeNarrativeInput, ImportAnalyzeNarrativeOutput>(
                "import_analyze_narrative",
                "Build imported narrative dossier candidates",
            ),
            tool::<ImportComputeFinalStateInput, ImportComputeFinalStateOutput>(
                "import_compute_final_state",
                "Compute the imported manuscript continuation point and final state",
            ),
            tool::<ImportHydrateBibleInput, ImportHydrateBibleOutput>(
                "import_hydrate_bible",
                "Hydrate an import session into canonical story records",
            ),
            tool::<ImportApplyReviewDecisionsInput, ImportApplyReviewDecisionsOutput>(
                "import_apply_review_decisions",
                "Resolve persisted import review items and update session readiness",
            ),
            tool::<RecordKnowledgeInput, RecordKnowledgeOutput>(
                "record_knowledge",
                "Record canonical knowledge for a character. Remains the direct reveal path for circle-of-trust expansion (with secret_of_fact_id + learned_at) alongside the save-draft continuity package and the mined knowledge_learned delta — it is not superseded; those flows apply through this same tool.",
            ),
            tool::<RecordNoteInput, RecordNoteOutput>(
                "record_note",
                "Append a freeform note to branch session activity",
            ),
            tool::<UpdateWriterPositionInput, WriterPosition>(
                "update_writer_position",
                "Persist a branch writer cursor position without saving a draft",
            ),
            tool::<AuthoringPrepareRunInput, AuthoringPrepareRunOutput>(
                "authoring_prepare_run",
                "Inspect the requested chapter range for the project and produce a readiness report before starting",
            ),
            tool::<AuthoringStartRunInput, AuthoringStartRunOutput>(
                "authoring_start_run",
                "Start a new authoring run for the project and book chapter range",
            ),
            tool::<AuthoringStatusInput, AuthoringStatusOutput>(
                "authoring_status",
                "Retrieve progress, next action, and status of the active authoring run",
            ),
            tool::<AuthoringExecuteNextInput, AuthoringExecuteNextOutput>(
                "authoring_execute_next",
                "Execute exactly one safe authoring action for the active run",
            ),
            tool::<AuthoringSaveSceneDraftInput, AuthoringSaveSceneDraftOutput>(
                "authoring_save_scene_draft",
                "Save a host-drafted authoring scene with its required structured continuity package",
            ),
            tool::<AuthoringRecordCheckpointAuditInput, AuthoringRecordCheckpointAuditOutput>(
                "authoring_record_checkpoint_audit",
                "Attach a completed deep consistency audit to a pending authoring checkpoint",
            ),
            tool::<AuthoringReviewCheckpointInput, AuthoringReviewCheckpointOutput>(
                "authoring_review_checkpoint",
                "Mark a checkpoint reviewed after deep consistency and sampled reviews are complete; rejects unresolved/fix-later directives",
            ),
            tool::<AuthoringResolveBlockInput, AuthoringResolveBlockOutput>(
                "authoring_resolve_block",
                "Clear a blocked scene/run and advance it manually",
            ),
            tool::<AuthoringCancelRunInput, AuthoringCancelRunOutput>(
                "authoring_cancel_run",
                "Pause/cancel the active authoring run without deleting progress",
            ),
        ]
    }

    async fn set_session_context(&self, project_id: String, branch_id: Option<String>) {
        let mut session = self.session_context.write().await;
        session.active_project_id = Some(project_id);
        session.active_branch_id = branch_id;
    }

    async fn default_branch_id_for_project(&self, project_id: &str) -> anyhow::Result<String> {
        // Per-project main branch (Phase 6): every project has its own
        // active branch row. Ask the service for it directly rather than
        // guessing or falling back to a global singleton id.
        self.service.active_branch_id_for_project(project_id).await
    }

    async fn resolve_arguments(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
    ) -> anyhow::Result<rmcp::model::JsonObject> {
        let mut resolved = arguments.cloned().unwrap_or_default();
        let mut session = self.session_context.read().await.clone();

        if tool_supports_session_project_default(name) && !resolved.contains_key("project_id") {
            if let Some(project_id) = session.active_project_id.clone() {
                resolved.insert("project_id".to_string(), Value::String(project_id));
            } else if tool_requires_project_context(name) {
                let projects = self.service.list_projects().await?;
                match projects.projects.as_slice() {
                    [project] => {
                        let branch_id = self
                            .default_branch_id_for_project(&project.project_id)
                            .await?;
                        resolved.insert(
                            "project_id".to_string(),
                            Value::String(project.project_id.clone()),
                        );
                        session.active_project_id = Some(project.project_id.clone());
                        session.active_branch_id = Some(branch_id.clone());
                        self.set_session_context(project.project_id.clone(), Some(branch_id))
                            .await;
                    }
                    [] => anyhow::bail!(
                        "`{name}` requires a project, but none exist. Create a project first or pass `project_id` explicitly."
                    ),
                    _ => anyhow::bail!(
                        "`{name}` requires a project, but this MCP session has no active project. Call `set_active_project` or pass `project_id` explicitly."
                    ),
                }
            }
        }

        let resolved_project_id = resolved
            .get("project_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if tool_supports_session_branch_default(name)
            && !resolved.contains_key("branch_id")
            && resolved_project_id.as_deref() == session.active_project_id.as_deref()
            && let Some(branch_id) = session.active_branch_id
        {
            resolved.insert("branch_id".to_string(), Value::String(branch_id));
        }

        Ok(resolved)
    }

    async fn set_active_project(
        &self,
        input: SetActiveProjectInput,
    ) -> anyhow::Result<SetActiveProjectOutput> {
        let project = self.service.read_entity_by_id(&input.project_id).await?;
        let project_id = project
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(input.project_id.as_str())
            .to_string();
        // Resolve branch_id: explicit input, else look up the project's
        // active branch. The per-project main branch design (Phase 6)
        // means there's no global singleton fallback.
        let branch_id = match input.branch_id.clone() {
            Some(id) => id,
            None => {
                self.service
                    .active_branch_id_for_project(&project_id)
                    .await?
            }
        };
        // Ownership check + branch_name fetch in one go.
        let (branch_id, branch_project_id, branch_name) =
            self.service.get_branch_info(&branch_id).await?;
        if branch_project_id != project_id {
            anyhow::bail!("invalid request: branch does not belong to the requested project");
        }
        self.set_session_context(project_id.clone(), Some(branch_id.clone()))
            .await;

        Ok(SetActiveProjectOutput {
            project_id,
            branch_id,
            branch_name,
            status: "ok".to_string(),
        })
    }

    async fn tool_serialization_scope(
        &self,
        name: &str,
        arguments: &rmcp::model::JsonObject,
    ) -> anyhow::Result<Option<ToolSerializationScope>> {
        if !tool_requires_session_serialization(name) {
            return Ok(None);
        }
        if tool_requires_global_serialization(name) {
            return Ok(Some(ToolSerializationScope::Global));
        }

        let Some(project_id) = arguments.get("project_id").and_then(Value::as_str) else {
            if let Some(target_project_id) =
                arguments.get("target_project_id").and_then(Value::as_str)
            {
                return Ok(Some(ToolSerializationScope::Project(
                    target_project_id.to_string(),
                )));
            }
            if name == "import_manuscript"
                && arguments
                    .get("target_project_id")
                    .and_then(Value::as_str)
                    .is_none()
                && arguments
                    .get("create_project_name")
                    .and_then(Value::as_str)
                    .is_some()
            {
                return Ok(Some(ToolSerializationScope::Global));
            }
            return self
                .resolve_project_id_for_scoped_tool(name, arguments)
                .await
                .map(ToolSerializationScope::Project)
                .map(Some);
        };
        if arguments
            .get("target_project_id")
            .and_then(Value::as_str)
            .is_none()
            && arguments
                .get("create_project_name")
                .and_then(Value::as_str)
                .is_some()
            && matches!(name, "import_hydrate_bible")
        {
            return Ok(Some(ToolSerializationScope::Global));
        }
        let project_id = match arguments.get("target_project_id").and_then(Value::as_str) {
            Some(project_id) => project_id.to_string(),
            None => project_id.to_string(),
        };
        Ok(Some(ToolSerializationScope::Project(project_id)))
    }

    async fn resolve_project_id_for_scoped_tool(
        &self,
        name: &str,
        arguments: &rmcp::model::JsonObject,
    ) -> anyhow::Result<String> {
        match name {
            "update_entity" | "archive_entity" => {
                let entity_id = required_string_argument(arguments, "entity_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(entity_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .unwrap_or(entity_id)
                    .to_string())
            }
            "update_world_rule" => {
                let entity_id = required_string_argument(arguments, "world_rule_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(entity_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("world rule is missing project_id")?
                    .to_string())
            }
            "resolve_revision_marker" => {
                let entity_id = required_string_argument(arguments, "marker_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(entity_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("revision marker is missing project_id")?
                    .to_string())
            }
            "pull_chapter_from_file" | "push_chapter_to_file" => {
                let chapter_id = required_string_argument(arguments, "chapter_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(chapter_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("chapter is missing project_id")?
                    .to_string())
            }
            "bind_canonical_fact_to_scene" => {
                let scene_id = required_string_argument(arguments, "scene_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(scene_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("scene is missing project_id")?
                    .to_string())
            }
            _ => anyhow::bail!("{name} requires project_id for mutation serialization"),
        }
    }

    async fn lock_tool_scope(&self, scope: ToolSerializationScope) -> ToolSerializationGuard {
        match scope {
            ToolSerializationScope::Global => ToolSerializationGuard::Global {
                _gate: self
                    .serialization_state
                    .serialization_gate
                    .clone()
                    .write_owned()
                    .await,
            },
            ToolSerializationScope::Project(project_id) => {
                let gate = self
                    .serialization_state
                    .serialization_gate
                    .clone()
                    .read_owned()
                    .await;
                let project_lock = {
                    let mut locks = self.serialization_state.project_locks.lock().await;
                    locks
                        .entry(project_id)
                        .or_insert_with(|| Arc::new(Mutex::new(())))
                        .clone()
                };
                let project = project_lock.lock_owned().await;
                ToolSerializationGuard::Project {
                    _project: project,
                    _gate: gate,
                }
            }
        }
    }

    /// Is this serialization scope free right now? Used only to let
    /// `authoring_status` choose its degraded read-only path instead of queueing
    /// behind a minutes-long write. Advisory by nature — a scope free here can
    /// be taken a moment later, which is harmless: the caller either takes the
    /// lock normally or serves a non-persisting read.
    async fn scope_is_free(&self, scope: &ToolSerializationScope) -> bool {
        match scope {
            ToolSerializationScope::Global => self
                .serialization_state
                .serialization_gate
                .try_write()
                .is_ok(),
            ToolSerializationScope::Project(project_id) => {
                let project_lock = {
                    let mut locks = self.serialization_state.project_locks.lock().await;
                    locks
                        .entry(project_id.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(())))
                        .clone()
                };
                project_lock.try_lock().is_ok()
            }
        }
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
    ) -> anyhow::Result<CallToolResult> {
        let call_started = Instant::now();
        if !self.list_tools().iter().any(|tool| tool.name == name) {
            return Ok(structured_error_result(&anyhow::anyhow!(
                "unknown tool: {name}"
            )));
        }
        let resolved_arguments = match self.resolve_arguments(name, arguments).await {
            Ok(arguments) => arguments,
            Err(error) => return Ok(structured_error_result(&error)),
        };
        let lock_started = Instant::now();
        let scope = match self
            .tool_serialization_scope(name, &resolved_arguments)
            .await
        {
            Ok(scope) => scope,
            Err(error) => return Ok(structured_error_result(&error)),
        };

        // Observability under a long write. Tool calls for a project are
        // serialized, and a drafting/review step can hold that lock for MINUTES
        // when the route points at a reasoning model. `authoring_status` then
        // queued behind the very run it reports on and timed out — the operator
        // lost the one tool that could tell them whether the slow step was still
        // alive, exactly when they needed it.
        //
        // So status degrades instead of blocking: if the project lock is not
        // free promptly, serve the non-persisting `authoring_status_readonly`
        // reconcile. The answer is identical; it just does not write the
        // reconcile back, which is precisely what makes it safe to run
        // alongside the in-flight writer.
        if name == "authoring_status"
            && let Some(scope) = scope.as_ref()
            && !self.scope_is_free(scope).await
        {
            return match parse_arguments::<AuthoringStatusInput>(Some(&resolved_arguments)) {
                Ok(input) => match self.authoring_status_readonly(input).await {
                    Ok(Some(output)) => structured_result(&output)
                        .or_else(|error| Ok(structured_error_result(&error))),
                    Ok(None) => Ok(structured_error_result(&anyhow::anyhow!(
                        "No active or latest authoring run found for project"
                    ))),
                    Err(error) => Ok(structured_error_result(&error)),
                },
                Err(error) => Ok(structured_error_result(&error)),
            };
        }

        let _serialization_guard = match scope {
            Some(scope) => Some(self.lock_tool_scope(scope).await),
            None => None,
        };
        let lock_wait_ms = lock_started.elapsed().as_millis() as u64;
        let arguments = Some(&resolved_arguments);
        let result = match name {
            "create_project" => match parse_arguments::<CreateProjectInput>(arguments) {
                Ok(input) => match self.service.create_project(input).await {
                    Ok(output) => {
                        // Per-project main branch (Phase 6): use the
                        // service-returned branch_id rather than the legacy
                        // hardcoded singleton.
                        self.set_session_context(
                            output.project_id.clone(),
                            Some(output.branch_id.clone()),
                        )
                        .await;
                        structured_result(&output)
                    }
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            "list_projects" => structured_result(&self.service.list_projects().await?),
            "set_active_project" => match parse_arguments::<SetActiveProjectInput>(arguments) {
                Ok(input) => match self.set_active_project(input).await {
                    Ok(output) => structured_result(&output),
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            "create_book" => {
                self.invoke(arguments, |input| self.service.create_book(input))
                    .await
            }
            "create_chapter" => {
                self.invoke(arguments, |input| self.service.create_chapter(input))
                    .await
            }
            "create_branch" => {
                self.invoke(arguments, |input| self.service.create_branch(input))
                    .await
            }
            "switch_branch" => match parse_arguments::<SwitchBranchInput>(arguments) {
                Ok(input) => {
                    let project_id = input.project_id.clone();
                    match self.service.switch_branch(input).await {
                        Ok(output) => {
                            self.set_session_context(project_id, Some(output.branch_id.clone()))
                                .await;
                            structured_result(&output)
                        }
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            },
            "set_narrator_voice" => {
                self.invoke(arguments, |input| self.service.set_narrator_voice(input))
                    .await
            }
            "set_project_calendar" => {
                self.invoke(arguments, |input| self.service.set_project_calendar(input))
                    .await
            }
            "set_scene_clock" => {
                self.invoke(arguments, |input| self.service.set_scene_clock(input))
                    .await
            }
            "set_timeline_event_clock" => {
                self.invoke(arguments, |input| {
                    self.service.set_timeline_event_clock(input)
                })
                .await
            }
            "set_character_birth" => {
                self.invoke(arguments, |input| self.service.set_character_birth(input))
                    .await
            }
            "set_project_quantity_scheme" => {
                self.invoke(arguments, |input| {
                    self.service.set_project_quantity_scheme(input)
                })
                .await
            }
            "commit_quantity_state" => {
                self.invoke(arguments, |input| self.service.commit_quantity_state(input))
                    .await
            }
            "derive_quantity_scheme_from_system_overlay" => {
                self.invoke(arguments, |input| {
                    self.service
                        .derive_quantity_scheme_from_system_overlay(input)
                })
                .await
            }
            "scan_scene_prices" => {
                self.invoke(arguments, |input| self.service.scan_scene_prices(input))
                    .await
            }
            "create_save_point" => {
                self.invoke(arguments, |input| self.service.create_save_point(input))
                    .await
            }
            "restore_save_point" => {
                self.invoke(arguments, |input| self.service.restore_save_point(input))
                    .await
            }
            "diff_branches" => {
                self.invoke(arguments, |input| self.service.diff_branches(input))
                    .await
            }
            "create_style_profile_from_markdown" => {
                self.invoke(arguments, |input| {
                    self.service.create_style_profile_from_markdown(input)
                })
                .await
            }
            "check_style_profile_sources" => {
                self.invoke(arguments, |input| {
                    self.service.check_style_profile_sources(input)
                })
                .await
            }
            "preview_refresh_style_profile" => {
                self.invoke(arguments, |input| {
                    self.service.preview_refresh_style_profile(input)
                })
                .await
            }
            "refresh_style_profile" => {
                self.invoke(arguments, |input| self.service.refresh_style_profile(input))
                    .await
            }
            "list_style_profiles" => {
                self.invoke(arguments, |input| self.service.list_style_profiles(input))
                    .await
            }
            "get_style_profile" => {
                self.invoke(arguments, |input| self.service.get_style_profile(input))
                    .await
            }
            "apply_style_profile" => {
                self.invoke(arguments, |input| self.service.apply_style_profile(input))
                    .await
            }
            "preview_apply_style_profile" => {
                self.invoke(arguments, |input| {
                    self.service.preview_apply_style_profile(input)
                })
                .await
            }
            "list_style_profile_applications" => {
                self.invoke(arguments, |input| {
                    self.service.list_style_profile_applications(input)
                })
                .await
            }
            "rollback_style_profile_application" => {
                self.invoke(arguments, |input| {
                    self.service.rollback_style_profile_application(input)
                })
                .await
            }
            "check_style_against_profile" => {
                self.invoke(arguments, |input| {
                    self.service.check_style_against_profile(input)
                })
                .await
            }
            "plan_style_revision" => {
                self.invoke(arguments, |input| self.service.plan_style_revision(input))
                    .await
            }
            "compare_style_profiles" => {
                self.invoke(arguments, |input| {
                    self.service.compare_style_profiles(input)
                })
                .await
            }
            "archive_style_profile" => {
                self.invoke(arguments, |input| self.service.archive_style_profile(input))
                    .await
            }
            "preview_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.preview_style_revision_patch(input)
                })
                .await
            }
            "evaluate_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.evaluate_style_revision_patch(input)
                })
                .await
            }
            "apply_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.apply_style_revision_patch(input)
                })
                .await
            }
            "list_style_revision_patch_audits" => {
                self.invoke(arguments, |input| {
                    self.service.list_style_revision_patch_audits(input)
                })
                .await
            }
            "rollback_style_revision_patch" => {
                self.invoke(arguments, |input| {
                    self.service.rollback_style_revision_patch(input)
                })
                .await
            }
            "merge_branch" => {
                self.invoke(arguments, |input| self.service.merge_branch(input))
                    .await
            }
            "revise_scene" => {
                self.invoke(arguments, |input| self.service.revise_scene(input))
                    .await
            }
            "generate_alternatives" => {
                self.invoke(arguments, |input| self.service.generate_alternatives(input))
                    .await
            }
            "compare_alternatives" => {
                self.invoke(arguments, |input| self.service.compare_alternatives(input))
                    .await
            }
            "select_alternative" => {
                self.invoke(arguments, |input| self.service.select_alternative(input))
                    .await
            }
            "list_revision_markers" => {
                self.invoke(arguments, |input| self.service.list_revision_markers(input))
                    .await
            }
            "resolve_revision_marker" => {
                self.invoke(arguments, |input| {
                    self.service.resolve_revision_marker(input)
                })
                .await
            }
            "create_character" => {
                self.invoke(arguments, |input| self.service.create_character(input))
                    .await
            }
            "create_location" => {
                self.invoke(arguments, |input| self.service.create_location(input))
                    .await
            }
            "create_faction" => {
                self.invoke(arguments, |input| self.service.create_faction(input))
                    .await
            }
            "create_religion" => {
                self.invoke(arguments, |input| self.service.create_religion(input))
                    .await
            }
            "create_economy" => {
                self.invoke(arguments, |input| self.service.create_economy(input))
                    .await
            }
            "create_term" => {
                self.invoke(arguments, |input| self.service.create_term(input))
                    .await
            }
            "batch_create_terms" => {
                self.invoke(arguments, |input| self.service.batch_create_terms(input))
                    .await
            }
            "create_relationship" => {
                self.invoke(arguments, |input| self.service.create_relationship(input))
                    .await
            }
            "create_world_rule" => {
                self.invoke(arguments, |input| self.service.create_world_rule(input))
                    .await
            }
            "update_entity" => {
                self.invoke(arguments, |input| self.service.update_entity(input))
                    .await
            }
            "archive_entity" => {
                self.invoke(arguments, |input| self.service.archive_entity(input))
                    .await
            }
            "create_plot_line" => {
                self.invoke(arguments, |input| self.service.create_plot_line(input))
                    .await
            }
            "create_conflict" => {
                self.invoke(arguments, |input| self.service.create_conflict(input))
                    .await
            }
            "create_theme" => {
                self.invoke(arguments, |input| self.service.create_theme(input))
                    .await
            }
            "create_motif" => {
                self.invoke(arguments, |input| self.service.create_motif(input))
                    .await
            }
            "batch_create_motifs" => {
                self.invoke(arguments, |input| self.service.batch_create_motifs(input))
                    .await
            }
            "create_narrative_promise" => {
                self.invoke(arguments, |input| {
                    self.service.create_narrative_promise(input)
                })
                .await
            }
            "batch_create_narrative_promises" => {
                self.invoke(arguments, |input| {
                    self.service.batch_create_narrative_promises(input)
                })
                .await
            }
            "update_promise_status" => {
                self.invoke(arguments, |input| self.service.update_promise_status(input))
                    .await
            }
            "create_character_arc" => {
                self.invoke(arguments, |input| self.service.create_character_arc(input))
                    .await
            }
            "update_arc_milestone" => {
                self.invoke(arguments, |input| self.service.update_arc_milestone(input))
                    .await
            }
            "create_future_knowledge" => {
                self.invoke(arguments, |input| {
                    self.service.create_future_knowledge(input)
                })
                .await
            }
            "create_timeline_event" => {
                self.invoke(arguments, |input| self.service.create_timeline_event(input))
                    .await
            }
            "create_temporal_intervention" => {
                self.invoke(arguments, |input| {
                    self.service.create_temporal_intervention(input)
                })
                .await
            }
            "create_system_overlay" => {
                self.invoke(arguments, |input| self.service.create_system_overlay(input))
                    .await
            }
            "run_dual_persona_review" => {
                self.invoke(arguments, |input| {
                    self.service.run_dual_persona_review(input)
                })
                .await
            }
            "create_pacing_config" => {
                self.invoke(arguments, |input| self.service.create_pacing_config(input))
                    .await
            }
            "create_pacing_curve" => {
                self.invoke(arguments, |input| self.service.create_pacing_curve(input))
                    .await
            }
            "set_arc_pacing_constraints" => {
                self.invoke(arguments, |input| {
                    self.service.set_arc_pacing_constraints(input)
                })
                .await
            }
            "plan_chapter" => {
                self.invoke(arguments, |input| self.service.plan_chapter(input))
                    .await
            }
            "annotate_scene_beats" => {
                self.invoke(arguments, |input| self.service.annotate_scene_beats(input))
                    .await
            }
            "save_summary" => {
                self.invoke(arguments, |input| self.service.save_summary(input))
                    .await
            }
            "set_book_outline" => {
                self.invoke(arguments, |input| self.service.set_book_outline(input))
                    .await
            }
            "set_chapter_outline" => {
                self.invoke(arguments, |input| self.service.set_chapter_outline(input))
                    .await
            }
            "check_consistency" => {
                self.invoke(arguments, |input| self.service.check_consistency(input))
                    .await
            }
            "register_canonical_fact" => {
                self.invoke(arguments, |input| {
                    self.service.register_canonical_fact(input)
                })
                .await
            }
            "extract_canonical_facts_from_scene" => {
                self.invoke(arguments, |input| {
                    self.service.extract_canonical_facts_from_scene(input)
                })
                .await
            }
            "migrate_canonical_fact" => {
                self.invoke(arguments, |input| {
                    self.service.migrate_canonical_fact(input)
                })
                .await
            }
            "bind_canonical_fact_to_scene" => {
                self.invoke(arguments, |input| {
                    self.service.bind_canonical_fact_to_scene(input)
                })
                .await
            }
            "update_world_rule" => {
                self.invoke(arguments, |input| self.service.update_world_rule(input))
                    .await
            }
            "set_character_voice_profile" => {
                self.invoke(arguments, |input| {
                    self.service.set_character_voice_profile(input)
                })
                .await
            }
            "batch_set_character_voice_profiles" => {
                self.invoke(arguments, |input| {
                    self.service.batch_set_character_voice_profiles(input)
                })
                .await
            }
            "search_bible" => {
                self.invoke(arguments, |input| self.service.search_bible(input))
                    .await
            }
            "find_scenes_referencing" => {
                self.invoke(arguments, |input| {
                    self.service.find_scenes_referencing(input)
                })
                .await
            }
            "rebuild_search_index" => {
                self.invoke(arguments, |input| self.service.rebuild_search_index(input))
                    .await
            }
            "backfill_scene_source_offsets" => {
                self.invoke(arguments, |input| {
                    self.service.backfill_scene_source_offsets(input)
                })
                .await
            }
            "pull_chapter_from_file" => {
                self.invoke(arguments, |input| {
                    self.service.pull_chapter_from_file(input)
                })
                .await
            }
            "push_chapter_to_file" => {
                self.invoke(arguments, |input| self.service.push_chapter_to_file(input))
                    .await
            }
            "authoring_prepare_run" => {
                // Box::pin: the authoring handlers' futures are large enough
                // that un-boxed they push call_tool past the 2MB tokio worker
                // stack (same class as the decide_canon_deltas fan-out fix).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_prepare_run(input))
                })
                .await
            }
            "authoring_start_run" => {
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_start_run(input))
                })
                .await
            }
            "authoring_status" => {
                self.invoke(arguments, |input| self.handle_authoring_status(input))
                    .await
            }
            "authoring_execute_next" => {
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_execute_next(input))
                })
                .await
            }
            "authoring_save_scene_draft" => {
                // Box::pin: grown by the journal emitters; keep call_tool below
                // the 2MB tokio worker stack (same precedent as the other
                // authoring handlers).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_save_scene_draft(input))
                })
                .await
            }
            "authoring_record_checkpoint_audit" => {
                self.invoke(arguments, |input| {
                    self.handle_authoring_record_checkpoint_audit(input)
                })
                .await
            }
            "authoring_review_checkpoint" => {
                // Box::pin: grown by the journal emitters (see above).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_review_checkpoint(input))
                })
                .await
            }
            "authoring_resolve_block" => {
                self.invoke(arguments, |input| {
                    self.handle_authoring_resolve_block(input)
                })
                .await
            }
            "authoring_cancel_run" => {
                // Box::pin: grown by the journal emitter (see above).
                self.invoke(arguments, |input| {
                    Box::pin(self.handle_authoring_cancel_run(input))
                })
                .await
            }
            "configure_agents" => match parse_arguments::<ConfigureAgentsInput>(arguments) {
                Ok(input) => match self.service.configure_agents(input) {
                    Ok(output) => structured_result(&output),
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            "get_model_usage" => {
                self.invoke(arguments, |input| self.service.get_model_usage(input))
                    .await
            }
            "get_series_status" => {
                self.invoke(arguments, |input| self.service.get_series_status(input))
                    .await
            }
            "prepare_episode_release" => {
                self.invoke(arguments, |input| {
                    self.service.prepare_episode_release(input)
                })
                .await
            }
            "release_episode" => {
                self.invoke(arguments, |input| self.service.release_episode(input))
                    .await
            }
            "get_episode_release" => {
                self.invoke(arguments, |input| self.service.get_episode_release(input))
                    .await
            }
            "get_editorial_queue" => {
                self.invoke(arguments, |input| self.service.get_editorial_queue(input))
                    .await
            }
            "decide_editorial_item" => {
                self.invoke(arguments, |input| self.service.decide_editorial_item(input))
                    .await
            }
            "read_episode" => {
                self.invoke(arguments, |input| self.service.read_episode(input))
                    .await
            }
            "list_agents" => structured_result(&self.service.list_agents()),
            "init_grok_skills" => match parse_arguments::<InitGrokSkillsInput>(arguments) {
                Ok(input) => {
                    let output = self.handle_init_grok_skills(input)?;
                    structured_result(&output)
                }
                Err(error) => Err(error),
            },
            "list_skills" => structured_result(&ListSkillsOutput {
                skills: spindle_adapters::list_skills()
                    .iter()
                    .map(|skill| SkillSummary {
                        name: skill.name.to_string(),
                        kind: "skill".to_string(),
                    })
                    .collect(),
                references: spindle_adapters::list_references()
                    .iter()
                    .map(|reference| SkillSummary {
                        name: reference.name.to_string(),
                        kind: "reference".to_string(),
                    })
                    .collect(),
            }),
            "get_skill" => match parse_arguments::<GetSkillInput>(arguments) {
                Ok(input) => match spindle_adapters::get_skill(&input.name) {
                    Some(skill) => structured_result(&GetSkillOutput {
                        name: skill.name.to_string(),
                        markdown: skill.markdown.to_string(),
                    }),
                    None => Ok(structured_error_result(&anyhow::anyhow!(
                        "unknown skill: {} (use list_skills to see available skills)",
                        input.name
                    ))),
                },
                Err(error) => Err(error),
            },
            "get_reference" => match parse_arguments::<GetReferenceInput>(arguments) {
                Ok(input) => match spindle_adapters::get_reference(&input.name) {
                    Some(reference) => structured_result(&GetReferenceOutput {
                        name: reference.name.to_string(),
                        markdown: reference.markdown.to_string(),
                    }),
                    None => Ok(structured_error_result(&anyhow::anyhow!(
                        "unknown reference: {} (use list_skills to see available references)",
                        input.name
                    ))),
                },
                Err(error) => Err(error),
            },
            "test_agent" => {
                self.invoke(arguments, |input| self.service.test_agent(input))
                    .await
            }
            "continue_generation" => {
                self.invoke(arguments, |input| self.service.continue_generation(input))
                    .await
            }
            "revise_generation" => {
                self.invoke(arguments, |input| self.service.revise_generation(input))
                    .await
            }
            "research_query" => {
                self.invoke(arguments, |input| self.service.research_query(input))
                    .await
            }
            "research_add_source" => {
                self.invoke(arguments, |input| self.service.research_add_source(input))
                    .await
            }
            "research_add_note" => {
                self.invoke(arguments, |input| self.service.research_add_note(input))
                    .await
            }
            "research_add_claim" => {
                self.invoke(arguments, |input| self.service.research_add_claim(input))
                    .await
            }
            "research_search" => {
                self.invoke(arguments, |input| self.service.research_search(input))
                    .await
            }
            "research_pack_for_scene" => {
                self.invoke(arguments, |input| {
                    self.service.research_pack_for_scene(input)
                })
                .await
            }
            "research_usage_for_scene" => {
                self.invoke(arguments, |input| {
                    self.service.research_usage_for_scene(input)
                })
                .await
            }
            "research_ingest_report" => {
                self.invoke(arguments, |input| {
                    self.service.research_ingest_report(input)
                })
                .await
            }
            "research_plan_for_scene" => {
                self.invoke(arguments, |input| {
                    self.service.research_plan_for_scene(input)
                })
                .await
            }
            "export_epub" => {
                self.invoke(arguments, |input| self.service.export_epub(input))
                    .await
            }
            "preflight_book_export" => {
                self.invoke(arguments, |input| self.service.preflight_book_export(input))
                    .await
            }
            "compile_manuscript" => {
                self.invoke(arguments, |input| self.service.compile_manuscript(input))
                    .await
            }
            "export_recap" => {
                self.invoke(arguments, |input| self.service.export_recap(input))
                    .await
            }
            "export_series_bible" => {
                self.invoke(arguments, |input| self.service.export_series_bible(input))
                    .await
            }
            "mine_scene_canon" => {
                self.invoke(arguments, |input| self.service.mine_scene_canon(input))
                    .await
            }
            "list_canon_deltas" => {
                self.invoke(arguments, |input| self.service.list_canon_deltas(input))
                    .await
            }
            "decide_canon_deltas" => {
                self.invoke(arguments, |input| self.service.decide_canon_deltas(input))
                    .await
            }
            "replan_chapter" => {
                // Box::pin: the differ builds the realized/target digests + a
                // model dispatch; un-boxed it inflates the call_tool future past
                // the worker stack (same class as the mine/decide fan-outs).
                self.invoke(arguments, |input| {
                    Box::pin(self.service.replan_chapter(input))
                })
                .await
            }
            "list_plan_amendments" => {
                self.invoke(arguments, |input| self.service.list_plan_amendments(input))
                    .await
            }
            "decide_plan_amendments" => {
                // Box::pin: the apply dispatcher fans out per-class plan replays
                // + the immutability guard; un-boxed it inflates the call_tool
                // future past the worker stack (same class as decide_canon_deltas).
                self.invoke(arguments, |input| {
                    Box::pin(self.service.decide_plan_amendments(input))
                })
                .await
            }
            "export_bible" => {
                self.invoke(arguments, |input| self.service.export_bible(input))
                    .await
            }
            "get_writer_state" => {
                self.invoke(arguments, |input| {
                    self.service.get_writer_state_envelope(input)
                })
                .await
            }
            "get_scene_context" => {
                self.invoke(arguments, |input| {
                    self.service.get_scene_context_envelope(input)
                })
                .await
            }
            "get_entity" => {
                self.invoke(arguments, |input| self.service.get_entity(input))
                    .await
            }
            "find_entity" => {
                self.invoke(arguments, |input| self.service.find_entity(input))
                    .await
            }
            "get_character_snapshot" => {
                self.invoke(arguments, |input| {
                    self.service.get_character_snapshot(input)
                })
                .await
            }
            "get_chapter_briefing" => {
                self.invoke(arguments, |input| self.service.get_chapter_briefing(input))
                    .await
            }
            "list_chapter_scenes" => {
                self.invoke(arguments, |input| self.service.list_chapter_scenes(input))
                    .await
            }
            "list_book_chapters" => {
                self.invoke(arguments, |input| self.service.list_book_chapters(input))
                    .await
            }
            "save_scene_draft" => {
                self.invoke(arguments, |input| self.service.save_scene_draft(input))
                    .await
            }
            "move_scene" => {
                self.invoke(arguments, |input| self.service.move_scene(input))
                    .await
            }
            "delete_scene" => {
                self.invoke(arguments, |input| self.service.delete_scene(input))
                    .await
            }
            "operator_delete_scene" => {
                self.invoke(arguments, |input| self.service.operator_delete_scene(input))
                    .await
            }
            "list_scene_versions" => {
                self.invoke(arguments, |input| self.service.list_scene_versions(input))
                    .await
            }
            "restore_scene_version" => {
                self.invoke(arguments, |input| self.service.restore_scene_version(input))
                    .await
            }
            "commit_scene_changes" => {
                self.invoke(arguments, |input| self.service.commit_scene_changes(input))
                    .await
            }
            "commit_character_state" => {
                self.invoke(arguments, |input| {
                    self.service.commit_character_state(input)
                })
                .await
            }
            "update_relationship" => {
                self.invoke(arguments, |input| self.service.update_relationship(input))
                    .await
            }
            "import_manuscript" => {
                self.invoke(arguments, |input| self.service.import_manuscript(input))
                    .await
            }
            "import_status" => {
                self.invoke(arguments, |input| self.service.import_status(input))
                    .await
            }
            "import_extract_entities" => {
                self.invoke(arguments, |input| {
                    self.service.import_extract_entities(input)
                })
                .await
            }
            "import_consolidate_entities" => {
                self.invoke(arguments, |input| {
                    self.service.import_consolidate_entities(input)
                })
                .await
            }
            "import_analyze_character" => {
                self.invoke(arguments, |input| {
                    self.service.import_analyze_character(input)
                })
                .await
            }
            "import_extract_world" => {
                self.invoke(arguments, |input| self.service.import_extract_world(input))
                    .await
            }
            "import_analyze_narrative" => {
                self.invoke(arguments, |input| {
                    self.service.import_analyze_narrative(input)
                })
                .await
            }
            "import_compute_final_state" => {
                self.invoke(arguments, |input| {
                    self.service.import_compute_final_state(input)
                })
                .await
            }
            "import_hydrate_bible" => {
                self.invoke(arguments, |input| self.service.import_hydrate_bible(input))
                    .await
            }
            "import_apply_review_decisions" => {
                self.invoke(arguments, |input| {
                    self.service.import_apply_review_decisions(input)
                })
                .await
            }
            "record_knowledge" => {
                self.invoke(arguments, |input| self.service.record_knowledge(input))
                    .await
            }
            "record_note" => {
                self.invoke(arguments, |input| self.service.record_note(input))
                    .await
            }
            "update_writer_position" => {
                self.invoke(arguments, |input| {
                    self.service.update_writer_position(input)
                })
                .await
            }
            _ => Err(anyhow::anyhow!("unknown tool: {name}")),
        };

        let is_error = result.is_err();
        log_tool_call_timing(
            name,
            lock_wait_ms,
            call_started.elapsed().as_millis() as u64,
            is_error,
        );

        Ok(match result {
            Ok(value) => value,
            Err(error) => structured_error_result(&error),
        })
    }

    async fn invoke<I, O, F, Fut>(
        &self,
        arguments: Option<&rmcp::model::JsonObject>,
        f: F,
    ) -> anyhow::Result<CallToolResult>
    where
        I: DeserializeOwned + schemars::JsonSchema,
        O: Serialize,
        F: FnOnce(I) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<O>>,
    {
        let input: I = parse_arguments(arguments)?;
        let output = f(input).await?;
        structured_result(&output)
    }
}

fn tool_requires_session_serialization(name: &str) -> bool {
    !matches!(
        name,
        "list_projects"
            | "get_model_usage"
            | "get_series_status"
            | "get_editorial_queue"
            | "prepare_episode_release"
            | "get_episode_release"
            | "list_agents"
            | "list_skills"
            | "get_skill"
            | "get_reference"
            | "agent_routing_config"
            | "test_agent"
            | "continue_generation"
            | "revise_generation"
            | "get_writer_state"
            | "get_chapter_briefing"
            | "get_scene_context"
            | "get_entity"
            | "find_entity"
            | "get_character_snapshot"
            | "list_book_chapters"
            | "list_chapter_scenes"
            | "list_revision_markers"
            | "list_scene_versions"
            | "check_consistency"
            | "search_bible"
            | "find_scenes_referencing"
            | "preflight_book_export"
            | "research_query"
            | "research_search"
            | "research_pack_for_scene"
            | "research_usage_for_scene"
            | "research_plan_for_scene"
    )
}

fn tool_requires_global_serialization(name: &str) -> bool {
    matches!(
        name,
        "create_project"
            | "configure_agents"
            | "test_agent"
            | "continue_generation"
            | "revise_generation"
            | "init_grok_skills"
    )
}

fn required_string_argument<'a>(
    arguments: &'a rmcp::model::JsonObject,
    field: &str,
) -> anyhow::Result<&'a str> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("{field} is required"))
}

fn log_tool_call_timing(name: &str, lock_wait_ms: u64, total_ms: u64, is_error: bool) {
    let slow_threshold_ms = std::env::var("SPINDLE_SLOW_TOOL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000);
    if matches!(std::env::var("SPINDLE_PERF_LOG").as_deref(), Ok("1"))
        || lock_wait_ms > 0
        || total_ms >= slow_threshold_ms
    {
        tracing::info!(
            tool = name,
            lock_wait_ms,
            total_ms,
            is_error,
            "mcp_tool_call_timing"
        );
    }
}

fn tool_supports_session_project_default(name: &str) -> bool {
    name != "set_active_project"
}

fn tool_requires_project_context(name: &str) -> bool {
    !matches!(
        name,
        "create_project"
            | "list_projects"
            | "get_model_usage"
            | "get_episode_release"
            | "set_active_project"
            | "import_manuscript"
            | "list_agents"
            | "list_skills"
            | "get_skill"
            | "get_reference"
            | "agent_routing_config"
            | "test_agent"
            | "continue_generation"
            | "revise_generation"
            | "configure_agents"
            | "init_grok_skills"
    )
}

/// The Grok skill adapter content for Spindle.
/// This makes the bible://skills/* resources usable as first-class skills in Grok.
const SPINDLE_GROK_SKILL_MD: &str = r#"---
name: spindle
description: Use for any work involving Spindle book projects (new books, scene writing, character creation, worldbuilding, revision, explicit scenes, etc.). This is the Grok adapter for Spindle's official skills.
---

# Spindle (Grok Adapter)

This skill bridges Spindle's canonical instructions (served as `bible://skills/*` MCP resources) into Grok's skill system.

## How to use

When the user asks to do anything related to writing, worldbuilding, or managing a book inside Spindle, activate this skill.

The authoritative, always-up-to-date instructions live at:
- `bible://skills/scene-writer`
- `bible://skills/character-creator`
- `bible://skills/worldbuilder`
- `bible://skills/revision-manager`
- `bible://skills/continuity-editor`
- `bible://skills/editor`
- `bible://skills/manuscript-importer`
- and others under `bible://skills/*`

**Always prefer reading the live `bible://` version** for the current detailed procedure rather than relying on stale embedded text.

## Grok-Specific Guidance

- Spindle already gives excellent structural guardrails (Bible, continuity sheets, hard constraints, voice profiles, explicit routing).
- Your advantage in this environment is producing **natural, voicey, webnovel-style prose** (especially first-person, wry, comedic, internally-monologuing, or raw/explicit when the book calls for it).
- Do **not** over-polish into high-literary fiction unless the project explicitly wants that tone.
- For explicit scenes: respect the rating-aware routing. Never generate `Explicit` sexual prose client-side; use `continue_generation` with `rating: "explicit"`.

## Recommended First Actions on a Spindle Task

1. Ensure the correct project is active (`set_active_project` if needed).
2. Call `get_writer_state` to re-anchor.
3. Call the appropriate high-level briefing (`get_chapter_briefing`, `get_scene_context`, etc.).
4. Follow the detailed steps from the matching `bible://skills/*` resource.

This adapter ensures Spindle workflows feel as well-guided in Grok as they do in Claude.
"#;

impl ToolRouter {
    fn handle_init_grok_skills(
        &self,
        input: InitGrokSkillsInput,
    ) -> anyhow::Result<InitGrokSkillsOutput> {
        run_init_grok_skills(input.target_dir, input.global)
    }

    async fn authoring_project_snapshot(
        &self,
        state: &HarnessState,
    ) -> anyhow::Result<ProjectSnapshot> {
        let repo = self.service.repository();
        let active_branch = repo.get_active_branch(&state.project_id).await?;
        let scenes = repo
            .list_scenes_by_project_and_branch(&state.project_id, &active_branch.id)
            .await?;
        let plans = repo
            .list_chapter_plans_by_project(&state.project_id)
            .await?;
        let summaries = repo
            .list_chapter_summaries_by_project(&state.project_id)
            .await?;
        let summarized_chapters = summaries
            .into_iter()
            .filter(|summary| {
                summary.branch_id == active_branch.id && summary.book_number == state.book_number
            })
            .map(|summary| summary.chapter_number)
            .collect::<BTreeSet<_>>();

        let mut chapters = BTreeMap::new();
        for chapter in &state.chapters {
            let chapter_row = repo
                .get_chapter_by_number(&state.project_id, state.book_number, chapter.chapter_number)
                .await
                .with_context(|| {
                    format!(
                        "failed to resolve chapter {} for authoring snapshot",
                        chapter.chapter_number
                    )
                })?;

            let persisted_scenes = scenes
                .iter()
                .filter(|scene| {
                    scene.book_number == state.book_number
                        && scene.chapter_number == chapter.chapter_number
                })
                .map(|scene| {
                    (
                        scene.scene_order,
                        PersistedScene {
                            scene_id: scene.id.clone(),
                            scene_order: scene.scene_order,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();

            let mut chapter_plan = None;
            if let Some(plan) = plans.iter().find(|plan| {
                plan.branch_id == active_branch.id
                    && plan.book_number == state.book_number
                    && plan.chapter_number == chapter.chapter_number
            }) {
                let mut planned_scenes = Vec::new();
                for planned in &plan.scenes {
                    let harness_scene = chapter
                        .scenes
                        .iter()
                        .find(|scene| scene.scene_order == planned.scene_order);
                    let scene_location = planned
                        .location_id
                        .clone()
                        .or_else(|| harness_scene.map(|scene| scene.location_id.clone()));
                    let research_tags = planned.research_tags.clone();
                    let explicit_query = planned.explicit_query.clone();

                    let pack = self
                        .service
                        .research_pack_for_scene(ResearchPackForSceneInput {
                            project_id: state.project_id.clone(),
                            branch_id: Some(active_branch.id.clone()),
                            scene_summary: Some(planned.summary.clone()),
                            scene_location,
                            character_ids: planned.character_ids.clone(),
                            tags: research_tags.clone(),
                            explicit_query: explicit_query.clone(),
                            limit: Some(10),
                        })
                        .await
                        .unwrap_or(ResearchPackForSceneOutput {
                            sources: vec![],
                            notes: vec![],
                            claims: vec![],
                        });

                    let research_pack_empty =
                        pack.sources.is_empty() && pack.notes.is_empty() && pack.claims.is_empty();
                    let research_tags_matched = if research_tags.is_empty() {
                        true
                    } else {
                        research_tags.iter().any(|tag| {
                            let tag_lower = tag.to_lowercase();
                            pack.sources.iter().any(|source| {
                                source
                                    .tags
                                    .iter()
                                    .any(|source_tag| source_tag.to_lowercase() == tag_lower)
                            }) || pack.notes.iter().any(|note| {
                                note.tags
                                    .iter()
                                    .any(|note_tag| note_tag.to_lowercase() == tag_lower)
                            }) || pack.claims.iter().any(|claim| {
                                claim
                                    .tags
                                    .iter()
                                    .any(|claim_tag| claim_tag.to_lowercase() == tag_lower)
                            })
                        })
                    };

                    planned_scenes.push(PlannedSceneSnapshot {
                        scene_order: planned.scene_order,
                        character_ids: planned.character_ids.clone(),
                        research_required: planned.research_required,
                        research_tags,
                        explicit_query,
                        research_pack_empty,
                        research_tags_matched,
                    });
                }

                chapter_plan = Some(ChapterPlanSnapshot {
                    synopsis: plan.synopsis.clone(),
                    pov_character_id: plan.pov_character_id.clone(),
                    scenes: planned_scenes,
                });
            }

            chapters.insert(
                chapter.chapter_number,
                ChapterSnapshot {
                    chapter_id: chapter_row.id,
                    scenes: persisted_scenes,
                    chapter_plan,
                },
            );
        }

        Ok(ProjectSnapshot {
            active_branch_id: active_branch.id,
            active_branch_name: active_branch.name,
            chapters,
            summarized_chapters,
        })
    }

    async fn handle_authoring_prepare_run(
        &self,
        input: AuthoringPrepareRunInput,
    ) -> anyhow::Result<AuthoringPrepareRunOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let end_chapter = if let Some(ec) = input.end_chapter {
            ec
        } else if let Some(cc) = input.chapter_count {
            input.start_chapter + cc - 1
        } else {
            let chapters = repo
                .list_chapters_by_book_number(&project_id, input.book_number)
                .await?;
            chapters
                .iter()
                .map(|c| c.chapter_number)
                .max()
                .unwrap_or(input.start_chapter)
        };

        let mut invalid_inputs = Vec::new();
        if input.book_number <= 0 {
            invalid_inputs.push(format!(
                "book_number must be positive, got {}",
                input.book_number
            ));
        }
        if input.start_chapter <= 0 {
            invalid_inputs.push(format!(
                "start_chapter must be positive, got {}",
                input.start_chapter
            ));
        }
        if let Some(chapter_count) = input.chapter_count
            && chapter_count <= 0
        {
            invalid_inputs.push(format!(
                "chapter_count must be positive when provided, got {}",
                chapter_count
            ));
        }
        if end_chapter < input.start_chapter {
            invalid_inputs.push(format!(
                "end_chapter {} is before start_chapter {}",
                end_chapter, input.start_chapter
            ));
        }
        if !invalid_inputs.is_empty() {
            return Ok(AuthoringPrepareRunOutput {
                project_id,
                book_number: input.book_number,
                start_chapter: input.start_chapter,
                end_chapter,
                ready_to_draft: false,
                missing_requirements: invalid_inputs,
                details: Vec::new(),
            });
        }

        let active_branch = repo.get_active_branch(&project_id).await?;

        let db_chapters = repo
            .list_chapters_by_book_number(&project_id, input.book_number)
            .await?;
        let db_plans = repo.list_chapter_plans_by_project(&project_id).await?;
        let known_character_ids = repo
            .list_characters_by_project_and_branch(&project_id, &active_branch.id)
            .await?
            .into_iter()
            .map(|character| character.id)
            .collect::<BTreeSet<_>>();
        let known_location_ids = repo
            .list_locations_by_project_and_branch(&project_id, &active_branch.id)
            .await?
            .into_iter()
            .map(|location| location.id)
            .collect::<BTreeSet<_>>();

        let mut missing_requirements = Vec::new();
        let mut details = Vec::new();
        // Distinct content ratings of planned scenes across the requested
        // chapter range. Keyed by the canonical lowercase rating string so the
        // draft-route preflight below runs once per rating in a deterministic
        // order. Populated as each scene's rating is resolved in the loop.
        let mut planned_ratings: BTreeSet<&'static str> = BTreeSet::new();

        for ch_num in input.start_chapter..=end_chapter {
            let mut chapter_missing_items = Vec::new();

            let ch_exists = db_chapters.iter().any(|c| c.chapter_number == ch_num);
            if !ch_exists {
                chapter_missing_items.push("missing chapter".to_string());
                missing_requirements.push(format!("Chapter {}: missing chapter", ch_num));
            }

            let plan_opt = db_plans.iter().find(|p| {
                p.book_number == input.book_number
                    && p.chapter_number == ch_num
                    && p.branch_id == active_branch.id
            });

            if let Some(plan) = plan_opt {
                if plan.scenes.is_empty() {
                    chapter_missing_items.push("missing scene list".to_string());
                    missing_requirements.push(format!("Chapter {}: missing scene list", ch_num));
                } else {
                    for scene in &plan.scenes {
                        if scene.character_ids.is_empty() {
                            chapter_missing_items.push(format!(
                                "scene {}: missing character IDs",
                                scene.scene_order
                            ));
                            missing_requirements.push(format!(
                                "Chapter {} scene {}: missing character IDs",
                                ch_num, scene.scene_order
                            ));
                        }
                        for character_id in &scene.character_ids {
                            if !known_character_ids.contains(character_id) {
                                chapter_missing_items.push(format!(
                                    "scene {}: unknown character ID {}",
                                    scene.scene_order, character_id
                                ));
                                missing_requirements.push(format!(
                                    "Chapter {} scene {}: unknown character ID {}",
                                    ch_num, scene.scene_order, character_id
                                ));
                            }
                        }

                        let (loc_id, rating) = planned_scene_location_and_rating(
                            scene.location_id.as_deref(),
                            scene.content_rating.as_ref(),
                            &scene.summary,
                            &scene.purpose,
                        );
                        if loc_id.is_none() {
                            chapter_missing_items
                                .push(format!("scene {}: missing location ID", scene.scene_order));
                            missing_requirements.push(format!(
                                "Chapter {} scene {}: missing location ID",
                                ch_num, scene.scene_order
                            ));
                        }
                        if let Some(location_id) = loc_id.as_ref()
                            && !known_location_ids.contains(location_id)
                        {
                            chapter_missing_items.push(format!(
                                "scene {}: unknown location ID {}",
                                scene.scene_order, location_id
                            ));
                            missing_requirements.push(format!(
                                "Chapter {} scene {}: unknown location ID {}",
                                ch_num, scene.scene_order, location_id
                            ));
                        }
                        match rating.as_ref() {
                            Some(rating) => {
                                planned_ratings.insert(rating.as_str());
                            }
                            None => {
                                chapter_missing_items.push(format!(
                                    "scene {}: missing content rating",
                                    scene.scene_order
                                ));
                                missing_requirements.push(format!(
                                    "Chapter {} scene {}: missing content rating",
                                    ch_num, scene.scene_order
                                ));
                            }
                        }
                    }
                }
            } else {
                chapter_missing_items.push("missing chapter plan".to_string());
                missing_requirements.push(format!("Chapter {}: missing chapter plan", ch_num));
            }

            details.push(AuthoringPrepareChapterDetails {
                chapter_number: ch_num,
                ready: chapter_missing_items.is_empty(),
                missing_items: chapter_missing_items,
            });
        }

        // Agent-route preflight: for each distinct rating the planned scenes
        // need, verify the `draft` route resolves to a configured, key-holding
        // agent that covers the rating. Config-level only — no network I/O; a
        // route that falls back to a built-in local route serves every rating
        // and is never flagged. Any unmet requirement blocks the run so a
        // multi-hour authoring run fails fast at `prepare` instead of hours in.
        let model_router = self.service.repository().model_router();
        for rating in &planned_ratings {
            let preflight = model_router.draft_route_preflight(rating);
            if let Some(problem) = preflight.problem {
                let detail = match problem {
                    DraftRoutePreflightProblem::Unresolved => {
                        "no draft route is configured for this rating".to_string()
                    }
                    DraftRoutePreflightProblem::MissingApiKey { env_var } => {
                        format!("agent is missing API key env {env_var}")
                    }
                    DraftRoutePreflightProblem::RatingNotCovered => {
                        "agent does not cover this rating".to_string()
                    }
                };
                let agent = preflight
                    .agent_id
                    .map(|id| format!(" (agent {id})"))
                    .unwrap_or_default();
                missing_requirements.push(format!(
                    "Route draft cannot serve rating {rating}{agent}: {detail}"
                ));
            }
        }

        // Canon-mining preflight (evolution §3.1/§4.4): only when the run opted
        // into `propose_all`. Mining is prose-bearing, so every planned rating
        // must be servable through the mine fallback ladder — covered if EITHER
        // the `mine` route OR the `review` route resolves cleared for the
        // rating. Uncovered ratings push a `missing_requirements` entry so the
        // operator decides before the run starts, not hours in (I8).
        if input.mining_policy.as_deref() == Some("propose_all") {
            for rating in &planned_ratings {
                if let Some(preflight) = model_router.mine_fallback_preflight(rating) {
                    let detail = match preflight.problem {
                        Some(DraftRoutePreflightProblem::Unresolved) => {
                            "no mine or review route is configured for this rating".to_string()
                        }
                        Some(DraftRoutePreflightProblem::MissingApiKey { env_var }) => {
                            format!("agent is missing API key env {env_var}")
                        }
                        Some(DraftRoutePreflightProblem::RatingNotCovered) => {
                            "no mine or review agent covers this rating".to_string()
                        }
                        None => continue,
                    };
                    let agent = preflight
                        .agent_id
                        .map(|id| format!(" (agent {id})"))
                        .unwrap_or_default();
                    missing_requirements.push(format!(
                        "Canon mining cannot serve rating {rating}{agent}: {detail}"
                    ));
                }
            }
        }

        // Auto-checkpoint preflight (evolution §3.3 K2): only when the run opted
        // into an auto policy (`auto_advisory` / `auto_strict`). The automation
        // runs its sampled dual-persona reviews AND its deep dual-persona
        // consistency pass through the `review` route (prose-bearing), so that
        // route must resolve rating-cleared for EVERY distinct rating in the
        // range (precondition (a)). Precondition (b) — a deep-check-capable
        // review route — collapses into (a) today: the same `review` route
        // serves the checkpoint's deep pass, so covering (a) covers (b). The
        // runtime explicit-manual-fallback in K3 is defense-in-depth for a
        // mid-run config change, NOT a licence to start uncovered — an auto run
        // must be fully covered at start (I8: fail at prepare, not at 2am).
        if matches!(
            input.checkpoint_policy.as_deref(),
            Some("auto_advisory") | Some("auto_strict")
        ) {
            let policy = input.checkpoint_policy.as_deref().unwrap_or("");
            for rating in &planned_ratings {
                let preflight = model_router.review_route_preflight(rating);
                if let Some(problem) = preflight.problem {
                    let detail = match problem {
                        DraftRoutePreflightProblem::Unresolved => {
                            "no review route is configured for this rating".to_string()
                        }
                        DraftRoutePreflightProblem::MissingApiKey { env_var } => {
                            format!("agent is missing API key env {env_var}")
                        }
                        DraftRoutePreflightProblem::RatingNotCovered => {
                            "review agent does not cover this rating".to_string()
                        }
                    };
                    let agent = preflight
                        .agent_id
                        .map(|id| format!(" (agent {id})"))
                        .unwrap_or_default();
                    missing_requirements.push(format!(
                        "Checkpoint policy {policy} requires route review to serve rating {rating}{agent}: {detail}"
                    ));
                }
            }
        }

        let ready_to_draft = missing_requirements.is_empty();
        Ok(AuthoringPrepareRunOutput {
            project_id,
            book_number: input.book_number,
            start_chapter: input.start_chapter,
            end_chapter,
            ready_to_draft,
            missing_requirements,
            details,
        })
    }

    async fn handle_authoring_start_run(
        &self,
        input: AuthoringStartRunInput,
    ) -> anyhow::Result<AuthoringStartRunOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        if input.checkpoint_interval == 0 {
            return Ok(AuthoringStartRunOutput {
                run_id: String::new(),
                status: "blocked".to_string(),
                message: "Cannot start authoring run: checkpoint_interval must be at least 1"
                    .to_string(),
            });
        }

        // Validate the opt-in mining policy up front. Canonical `mining_policy`
        // is None (disabled/default) or Some("propose_all"); an unknown value
        // is an input error (evolution §3.1). "disabled" canonicalizes to None
        // so the run persists NULL, matching pre-upgrade runs exactly.
        let mining_policy = match spindle_core::models::validate_mining_policy(
            input.mining_policy.as_deref(),
        ) {
            Ok(policy) => policy,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: mining_policy must be one of {:?}, got {:?}",
                        spindle_core::models::AUTHORING_MINING_POLICIES,
                        rejected
                    ),
                });
            }
        };

        // Validate the opt-in bounded-revise budget (evolution §3.2). None/0 =
        // disabled (canonicalized to None so the run persists NULL, matching a
        // pre-upgrade row); 1..=2 enable the in-run verify/revise loop; anything
        // else is an input error — a bound, not a knob explosion.
        let max_revise_attempts = match spindle_core::models::validate_max_revise_attempts(
            input.max_revise_attempts,
        ) {
            Ok(budget) => budget,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: max_revise_attempts must be between 0 and {}, got {}",
                        spindle_core::models::MAX_REVISE_ATTEMPTS_UPPER_BOUND,
                        rejected
                    ),
                });
            }
        };

        // Validate the opt-in checkpoint policy (evolution §3.3). None/"manual"
        // canonicalize to None so the run persists NULL, matching a pre-upgrade
        // row exactly (I1: manual/NULL is byte-identical to today). An auto
        // policy additionally requires review-route coverage across the run's
        // ratings, enforced by prepare's preflight below (K2).
        let checkpoint_policy = match spindle_core::models::validate_checkpoint_policy(
            input.checkpoint_policy.as_deref(),
        ) {
            Ok(policy) => policy,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: checkpoint_policy must be one of {:?}, got {:?}",
                        spindle_core::models::AUTHORING_CHECKPOINT_POLICIES,
                        rejected
                    ),
                });
            }
        };

        // Validate the opt-in living-outline replan policy (ADR 0003, evolution
        // §3.5). None/"disabled" canonicalize to None so the run persists NULL,
        // matching a pre-upgrade row exactly (disabled = never replans). There is
        // NO route preflight: the replan differ is non-prose-bearing (summaries +
        // metadata only — ADR D5), so no rating clearance applies, and on a
        // NoRoute it falls to review then skips honestly. Anything else is an
        // input error.
        let replan_policy = match spindle_core::models::validate_replan_policy(
            input.replan_policy.as_deref(),
        ) {
            Ok(policy) => policy,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: replan_policy must be one of {:?}, got {:?}",
                        spindle_core::models::AUTHORING_REPLAN_POLICIES,
                        rejected
                    ),
                });
            }
        };

        let prep_input = AuthoringPrepareRunInput {
            project_id: project_id.clone(),
            book_number: input.book_number,
            start_chapter: input.start_chapter,
            end_chapter: input.end_chapter,
            chapter_count: input.chapter_count,
            // Thread the resolved policy so prepare's mine-route preflight runs
            // only when the run actually opted into propose_all.
            mining_policy: mining_policy.clone(),
            // Threaded for symmetry; prepare adds NO preflight for revise (verify
            // is deterministic and revision reuses the draft route — §3.2).
            max_revise_attempts,
            // Thread the resolved policy so prepare's review-route preflight runs
            // only when the run actually opted into an auto checkpoint policy
            // (evolution §3.3 K2 precondition).
            checkpoint_policy: checkpoint_policy.clone(),
            // Threaded for symmetry; prepare adds NO preflight for replan (the
            // differ is non-prose-bearing and skips honestly on NoRoute — §3.5).
            replan_policy: replan_policy.clone(),
        };
        let prep_report = Box::pin(self.handle_authoring_prepare_run(prep_input)).await?;
        if !prep_report.ready_to_draft {
            return Ok(AuthoringStartRunOutput {
                run_id: "".to_string(),
                status: "blocked".to_string(),
                message: format!(
                    "Cannot start authoring run due to missing requirements: {:?}",
                    prep_report.missing_requirements
                ),
            });
        }

        let active_branch = repo.get_active_branch(&project_id).await?;
        let end_chapter = prep_report.end_chapter;

        let db_plans = repo.list_chapter_plans_by_project(&project_id).await?;
        let mut chapter_seeds = Vec::new();

        for ch_num in input.start_chapter..=end_chapter {
            let plan = db_plans
                .iter()
                .find(|p| {
                    p.book_number == input.book_number
                        && p.chapter_number == ch_num
                        && p.branch_id == active_branch.id
                })
                .context("plan not found")?;

            let mut scene_seeds = Vec::new();
            for scene in &plan.scenes {
                let (loc_id, rating) = planned_scene_location_and_rating(
                    scene.location_id.as_deref(),
                    scene.content_rating.as_ref(),
                    &scene.summary,
                    &scene.purpose,
                );
                let loc_id = loc_id.unwrap();
                let rating = rating.unwrap();

                scene_seeds.push(spindle_harness::state::SceneSeed {
                    scene_order: scene.scene_order,
                    character_ids: scene.character_ids.clone(),
                    location_id: loc_id,
                    content_rating: rating,
                    tone: None,
                    source_path: None,
                    research_required: scene.research_required,
                    research_tags: scene.research_tags.clone(),
                    explicit_query: scene.explicit_query.clone(),
                });
            }

            chapter_seeds.push(spindle_harness::state::ChapterSeed {
                chapter_number: ch_num,
                synopsis: plan.synopsis.clone(),
                pov_character_id: plan.pov_character_id.clone(),
                scenes: scene_seeds,
            });
        }

        let seed = spindle_harness::state::HarnessSeed {
            project_id: project_id.clone(),
            book_number: input.book_number,
            range: spindle_harness::state::ChapterRange {
                start_chapter: input.start_chapter,
                end_chapter,
            },
            checkpoint_interval: input.checkpoint_interval,
            editorial_directives: input.editorial_directives.unwrap_or_default(),
            chapters: chapter_seeds,
        };

        let mut harness_state =
            spindle_harness::state::HarnessState::from_seed(seed, active_branch.id.clone());
        harness_state.artifacts_dir = "../artifacts".to_string();
        harness_state.mining_policy = mining_policy;
        harness_state.max_revise_attempts = max_revise_attempts;
        harness_state.checkpoint_policy = checkpoint_policy;
        harness_state.replan_policy = replan_policy;

        let run_id = format!(
            "authoring_run:{}",
            ulid::Ulid::new().to_string().to_lowercase()
        );
        let (run, chapters, scenes, checkpoints) =
            map_harness_to_records(&run_id, &harness_state, "active", None);

        repo.save_authoring_run(run, chapters, scenes, checkpoints)
            .await?;

        // Journal: the run persisted (ADR D2 `run_started`). Emitted AFTER the
        // state change commits; a journal error never fails start_run (D3.3).
        RunJournal::new(repo)
            .emit(
                &run_id,
                "run_started",
                run_journal::run_started_payload(
                    input.book_number,
                    input.start_chapter,
                    end_chapter,
                    None,
                    harness_state.mining_policy.as_deref(),
                    harness_state.max_revise_attempts,
                ),
            )
            .await;

        Ok(AuthoringStartRunOutput {
            run_id: run_id.clone(),
            status: "active".to_string(),
            message: format!(
                "Started authoring run {} for chapters {}-{}",
                run_id, input.start_chapter, end_chapter
            ),
        })
    }

    async fn handle_authoring_status(
        &self,
        input: AuthoringStatusInput,
    ) -> anyhow::Result<AuthoringStatusOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;

        let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        let snapshot = self.authoring_project_snapshot(&harness_state).await?;
        let outcome = reconcile_state(harness_state.clone(), &snapshot);

        let next_action_str = outcome.next_action.to_string();
        let mut blocked_reason = None;
        let mut current_status = run.status.clone();

        if outcome.has_errors() {
            current_status = "blocked".to_string();
            let err_msgs: Vec<String> = outcome
                .findings
                .iter()
                .filter(|f| f.severity == spindle_harness::plan::FindingSeverity::Error)
                .map(|f| f.message.clone())
                .collect();
            blocked_reason = Some(err_msgs.join("; "));
        } else {
            for ch in &harness_state.chapters {
                for sc in &ch.scenes {
                    if let Some(r) = &sc.blocked_reason {
                        current_status = "blocked".to_string();
                        blocked_reason = Some(format!(
                            "Chapter {} scene {} blocked: {}",
                            ch.chapter_number, sc.scene_order, r
                        ));
                    }
                }
            }
        }

        if outcome.next_action == NextAction::Complete {
            current_status = "completed".to_string();
        }
        if matches!(
            outcome.next_action,
            NextAction::AwaitCheckpointReview { .. } | NextAction::AwaitResearch { .. }
        ) {
            current_status = "blocked".to_string();
            if matches!(outcome.next_action, NextAction::AwaitResearch { .. }) {
                blocked_reason = Some("await_research".to_string());
            } else {
                blocked_reason = Some("await_checkpoint_review".to_string());
            }
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
            &run_id,
            &outcome.state,
            &current_status,
            Some(run.created_at),
        );
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        Self::assemble_authoring_status_output(
            repo,
            run_id,
            &run,
            &outcome,
            current_status,
            blocked_reason,
            next_action_str,
        )
    }

    /// Read-only sibling of [`handle_authoring_status`] for viewer surfaces (the
    /// operator console, evolution §3.7): reconcile the run in memory and build
    /// the exact same [`AuthoringStatusOutput`], but **never persist** the
    /// reconcile (no `save_authoring_run`). Run-id discovery matches the tool:
    /// `None`/empty `run_id` resolves the project's latest run; a project with no
    /// run returns `Ok(None)` (an honest empty state, not an error). This keeps
    /// the console strictly read-only (I5) — a status read from a browser must
    /// not mutate the run tables.
    pub(crate) async fn authoring_status_readonly(
        &self,
        input: AuthoringStatusInput,
    ) -> anyhow::Result<Option<AuthoringStatusOutput>> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => match repo.find_latest_authoring_run_id(&project_id).await? {
                Some(rid) => rid,
                None => return Ok(None), // no run yet — honest empty state
            },
        };

        let Some((run, chapters, scenes, checkpoints)) = repo.get_authoring_run(&run_id).await?
        else {
            return Ok(None);
        };

        let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);
        let snapshot = self.authoring_project_snapshot(&harness_state).await?;
        let outcome = reconcile_state(harness_state.clone(), &snapshot);
        let next_action_str = outcome.next_action.to_string();

        let mut blocked_reason = None;
        let mut current_status = run.status.clone();
        if outcome.has_errors() {
            current_status = "blocked".to_string();
            let err_msgs: Vec<String> = outcome
                .findings
                .iter()
                .filter(|f| f.severity == spindle_harness::plan::FindingSeverity::Error)
                .map(|f| f.message.clone())
                .collect();
            blocked_reason = Some(err_msgs.join("; "));
        } else {
            for ch in &harness_state.chapters {
                for sc in &ch.scenes {
                    if let Some(r) = &sc.blocked_reason {
                        current_status = "blocked".to_string();
                        blocked_reason = Some(format!(
                            "Chapter {} scene {} blocked: {}",
                            ch.chapter_number, sc.scene_order, r
                        ));
                    }
                }
            }
        }
        if outcome.next_action == NextAction::Complete {
            current_status = "completed".to_string();
        }
        if matches!(
            outcome.next_action,
            NextAction::AwaitCheckpointReview { .. } | NextAction::AwaitResearch { .. }
        ) {
            current_status = "blocked".to_string();
            if matches!(outcome.next_action, NextAction::AwaitResearch { .. }) {
                blocked_reason = Some("await_research".to_string());
            } else {
                blocked_reason = Some("await_checkpoint_review".to_string());
            }
        }

        Self::assemble_authoring_status_output(
            repo,
            run_id,
            &run,
            &outcome,
            current_status,
            blocked_reason,
            next_action_str,
        )
        .map(Some)
    }

    /// Assemble the [`AuthoringStatusOutput`] DTO from a reconciled outcome. Pure
    /// (no writes); shared by [`handle_authoring_status`] (which persists first)
    /// and [`authoring_status_readonly`] (which never persists) so both surfaces
    /// stay byte-identical in shape.
    fn assemble_authoring_status_output(
        repo: &spindle_adapters::sqlite::Repository,
        run_id: String,
        run: &spindle_adapters::sqlite::records::AuthoringRun,
        outcome: &spindle_harness::plan::ReconcileOutcome,
        current_status: String,
        blocked_reason: Option<String>,
        next_action_str: String,
    ) -> anyhow::Result<AuthoringStatusOutput> {
        let project_id = run.project_id.clone();
        let checkpoint_state = match outcome.next_action {
            NextAction::AwaitCheckpointReview { .. } => Some("await_review".to_string()),
            NextAction::RunCheckpoint { .. } => Some("run_pending".to_string()),
            _ => None,
        };

        let mut status_chapters = Vec::new();
        for ch in &outcome.state.chapters {
            let mut status_scenes = Vec::new();
            for sc in &ch.scenes {
                let phase_str = match sc.phase {
                    spindle_harness::state::ScenePhase::Pending => "pending",
                    spindle_harness::state::ScenePhase::DraftSaved => "draft_saved",
                    spindle_harness::state::ScenePhase::ChangesCommitted => "changes_committed",
                    spindle_harness::state::ScenePhase::BeatsAnnotated => "beats_annotated",
                };
                status_scenes.push(AuthoringStatusScene {
                    scene_order: sc.scene_order,
                    phase: phase_str.to_string(),
                    scene_id: sc.scene_id.clone(),
                    scene_artifact_path: sc.scene_artifact_path.clone(),
                    blocked_reason: sc.blocked_reason.clone(),
                    mine_status: sc.mine_status.clone(),
                    mine_detail: sc.mine_detail.clone(),
                    verify_status: sc.verify_status.clone(),
                    verify_detail: sc.verify_detail.clone(),
                    revise_attempts: sc.revise_attempts,
                });
            }
            let ch_status_str = match ch.status {
                spindle_harness::state::ChapterStatus::Pending => "pending",
                spindle_harness::state::ChapterStatus::InProgress => "in_progress",
                spindle_harness::state::ChapterStatus::Complete => "complete",
            };
            status_chapters.push(AuthoringStatusChapter {
                chapter_number: ch.chapter_number,
                status: ch_status_str.to_string(),
                summary_saved: ch.summary_saved,
                summary_artifact_path: ch.summary_artifact_path.clone(),
                scenes: status_scenes,
                // Additive living-outline replan surfacing (ADR 0003, §3.5): the
                // chapter's post-summary replan outcome + detail. Enums/counts
                // only, never prose (I8).
                replan_status: ch.replan_status.clone(),
                replan_detail: ch.replan_detail.clone(),
            });
        }

        let status_state_path = authoring_state_path(repo.data_dir(), &run_id);
        let status_artifacts_root = authoring_artifacts_root(&status_state_path, &outcome.state);
        let mut cp_reports = Vec::new();
        for cp in &outcome.state.checkpoint_history {
            let cp_status_str = match cp.status {
                spindle_harness::state::CheckpointStatus::PendingReview => "pending_review",
                spindle_harness::state::CheckpointStatus::Reviewed => "reviewed",
            };
            // Additive reader-sim engagement surfacing (evolution §3.6, R3):
            // read the compact per-chapter engagement enums from this
            // checkpoint's report `reader_sim` section, if present. Enums/ids
            // only — the reader's notes and concern text stay in the artifact.
            let reader_sim_engagement = cp
                .report_artifact_path
                .as_deref()
                .map(|rel| read_reader_sim_engagement(&status_artifacts_root, rel))
                .unwrap_or_default();
            cp_reports.push(AuthoringStatusCheckpoint {
                start_chapter: cp.start_chapter,
                end_chapter: cp.end_chapter,
                save_point_id: cp.save_point_id.clone(),
                status: cp_status_str.to_string(),
                report_artifact_path: cp.report_artifact_path.clone(),
                // Additive auto-checkpoint surfacing (evolution §3.3, K4): the
                // run policy in force plus this checkpoint's automation outcome
                // and any scenes awaiting manual dual-persona review. Ids/enums
                // only, never prose (I8).
                checkpoint_policy: outcome.state.checkpoint_policy.clone(),
                auto_outcome: cp.auto_outcome.clone(),
                pending_manual_scene_ids: cp.pending_manual_scene_ids.clone(),
                reader_sim_engagement,
            });
        }

        Ok(AuthoringStatusOutput {
            run_id,
            project_id,
            status: current_status,
            next_action: next_action_str,
            blocked_reason,
            checkpoint_state,
            start_chapter: run.start_chapter,
            end_chapter: run.end_chapter,
            completed_chapter_count: outcome.state.completed_chapter_count(),
            total_chapter_count: outcome.state.chapters.len(),
            chapters: status_chapters,
            checkpoint_reports: cp_reports,
        })
    }

    async fn handle_authoring_execute_next(
        &self,
        input: AuthoringExecuteNextInput,
    ) -> anyhow::Result<AuthoringExecuteNextOutput> {
        let project_id = input.project_id.clone();
        let agent_mode = authoring_execute_uses_agent_drafting(input.mode.as_deref());
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;

        if run.status == "completed" {
            return Ok(AuthoringExecuteNextOutput {
                run_id: run_id.clone(),
                next_action: "complete".to_string(),
                executed_action: "none".to_string(),
                message: "Authoring run is already completed.".to_string(),
                status: run.status,
            });
        }
        if run.status == "paused" {
            return Ok(AuthoringExecuteNextOutput {
                run_id: run_id.clone(),
                next_action: "paused".to_string(),
                executed_action: "none".to_string(),
                message: "Authoring run is paused; start or select another run before executing."
                    .to_string(),
                status: run.status,
            });
        }

        let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        let data_dir = repo.data_dir();
        let snapshot = self.authoring_project_snapshot(&harness_state).await?;
        let outcome = reconcile_state(harness_state.clone(), &snapshot);

        if outcome.has_errors() {
            let (updated_run, updated_ch, updated_sc, updated_cp) =
                map_harness_to_records(&run_id, &outcome.state, "blocked", Some(run.created_at));
            repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                .await?;
            return Ok(AuthoringExecuteNextOutput {
                run_id: run_id.clone(),
                next_action: outcome.next_action.to_string(),
                executed_action: "none".to_string(),
                message: "Execution blocked by errors.".to_string(),
                status: "blocked".to_string(),
            });
        }

        // Auto-checkpoint interception (evolution §3.3, K3). A run whose
        // checkpoint_policy is auto_advisory/auto_strict does NOT surface
        // `AwaitCheckpointReview` — instead the harness performs the checkpoint
        // work in-process (deep consistency + sampled dual-persona reviews) and
        // self-clears on a severity threshold. `manual` (None) is untouched and
        // byte-identical to today (I1). Runs before the block match so an
        // approval lets the loop continue in the same call; a block falls
        // through to the existing AwaitCheckpointReview path.
        if let NextAction::AwaitCheckpointReview {
            start_chapter,
            end_chapter,
            ..
        } = &outcome.next_action
            && let Some(policy) = auto_checkpoint_policy(run.checkpoint_policy.as_deref())
        {
            let start_chapter = *start_chapter;
            let end_chapter = *end_chapter;
            let mut auto_state = outcome.state.clone();
            let auto = Box::pin(self.run_auto_checkpoint(
                &run_id,
                policy,
                &mut auto_state,
                start_chapter,
                end_chapter,
            ))
            .await?;

            if auto.approved {
                // The checkpoint self-cleared. Re-reconcile the post-review state
                // and persist; the run continues (or completes) exactly as a
                // manual review would leave it. Journal the auto-approval.
                let auto_snapshot = self.authoring_project_snapshot(&auto_state).await?;
                let auto_outcome = reconcile_state(auto_state.clone(), &auto_snapshot);
                let auto_status = if auto_outcome.next_action == NextAction::Complete {
                    "completed"
                } else if matches!(
                    auto_outcome.next_action,
                    NextAction::AwaitCheckpointReview { .. } | NextAction::AwaitResearch { .. }
                ) {
                    "blocked"
                } else {
                    "active"
                };
                let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
                    &run_id,
                    &auto_outcome.state,
                    auto_status,
                    Some(run.created_at),
                );
                repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                    .await?;

                let journal = RunJournal::new(repo);
                journal
                    .emit(
                        &run_id,
                        "checkpoint_auto_approved",
                        run_journal::checkpoint_auto_approved_payload(
                            start_chapter,
                            end_chapter,
                            policy,
                            &auto.finding_counts,
                        ),
                    )
                    .await;
                if run.status == "blocked" || run.status == "paused" {
                    journal
                        .emit(
                            &run_id,
                            "run_resumed",
                            run_journal::run_status_payload(None),
                        )
                        .await;
                }
                if auto_status == "completed" {
                    journal
                        .emit(
                            &run_id,
                            "run_completed",
                            run_journal::run_status_payload(None),
                        )
                        .await;
                }

                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: auto_outcome.next_action.to_string(),
                    executed_action: format!(
                        "auto-checkpoint approved chapters {start_chapter}-{end_chapter} under {policy}"
                    ),
                    message: format!(
                        "Checkpoint {start_chapter}-{end_chapter} auto-approved under {policy} (0 blocking findings)."
                    ),
                    status: auto_status.to_string(),
                });
            }

            // Not approved: the checkpoint stays pending_review exactly as
            // manual, with the auto_outcome + pending-manual scenes stamped on
            // it. Persist and journal `checkpoint_blocked`, then return blocked
            // with the prose-free reason. The manual escape hatch still works —
            // an operator can still call authoring_review_checkpoint.
            let reason = auto
                .blocked_reason
                .clone()
                .unwrap_or_else(|| "auto_checkpoint_blocked".to_string());
            let (updated_run, updated_ch, updated_sc, updated_cp) =
                map_harness_to_records(&run_id, &auto_state, "blocked", Some(run.created_at));
            repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                .await?;

            let journal = RunJournal::new(repo);
            journal
                .emit(
                    &run_id,
                    "checkpoint_blocked",
                    run_journal::checkpoint_blocked_payload(start_chapter, end_chapter, &reason),
                )
                .await;
            if run.status != "blocked" {
                journal
                    .emit(
                        &run_id,
                        "run_blocked",
                        run_journal::run_status_payload(Some("blocked")),
                    )
                    .await;
            }

            let manual_note = if auto.pending_manual_scene_ids.is_empty() {
                String::new()
            } else {
                format!(
                    " Scenes awaiting manual dual-persona review: [{}].",
                    auto.pending_manual_scene_ids.join(", ")
                )
            };
            return Ok(AuthoringExecuteNextOutput {
                run_id: run_id.clone(),
                next_action: outcome.next_action.to_string(),
                executed_action: format!(
                    "auto-checkpoint blocked chapters {start_chapter}-{end_chapter} under {policy}"
                ),
                message: format!(
                    "Checkpoint {start_chapter}-{end_chapter} blocked under {policy}: {reason}.{manual_note} \
                     Review it manually with authoring_review_checkpoint."
                ),
                status: "blocked".to_string(),
            });
        }

        match &outcome.next_action {
            NextAction::Blocked
            | NextAction::AwaitCheckpointReview { .. }
            | NextAction::AwaitResearch { .. }
            | NextAction::Complete => {
                let status_str = match &outcome.next_action {
                    NextAction::Complete => "completed",
                    NextAction::Blocked => "blocked",
                    NextAction::AwaitCheckpointReview { .. } => "blocked",
                    NextAction::AwaitResearch { .. } => "blocked",
                    _ => &run.status,
                };
                if status_str != run.status {
                    let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
                        &run_id,
                        &outcome.state,
                        status_str,
                        Some(run.created_at),
                    );
                    repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                        .await?;

                    // Journal (ADR D2): a genuine run-status transition persisted
                    // WITHOUT executing a step. Diff-derived from prev vs new
                    // status; emitted AFTER the save commits (D3.3). An
                    // AwaitCheckpointReview block is a `checkpoint_blocked`;
                    // Complete is `run_completed`; any other block is
                    // `run_blocked`.
                    let journal = RunJournal::new(repo);
                    match &outcome.next_action {
                        NextAction::AwaitCheckpointReview {
                            start_chapter,
                            end_chapter,
                            ..
                        } => {
                            journal
                                .emit(
                                    &run_id,
                                    "checkpoint_blocked",
                                    run_journal::checkpoint_blocked_payload(
                                        *start_chapter,
                                        *end_chapter,
                                        "await_checkpoint_review",
                                    ),
                                )
                                .await;
                        }
                        NextAction::Complete => {
                            journal
                                .emit(
                                    &run_id,
                                    "run_completed",
                                    run_journal::run_status_payload(None),
                                )
                                .await;
                        }
                        _ => {
                            let reason = match &outcome.next_action {
                                NextAction::AwaitResearch { .. } => "await_research",
                                _ => "blocked",
                            };
                            journal
                                .emit(
                                    &run_id,
                                    "run_blocked",
                                    run_journal::run_status_payload(Some(reason)),
                                )
                                .await;
                        }
                    }
                }
                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: outcome.next_action.to_string(),
                    executed_action: "none".to_string(),
                    message: format!(
                        "No action executed (action state is {:?})",
                        outcome.next_action
                    ),
                    status: status_str.to_string(),
                });
            }
            _ => {}
        }

        let action_to_execute = outcome.next_action.clone();
        if let NextAction::DraftScene {
            chapter_number,
            scene_order,
        } = &action_to_execute
            && !agent_mode
            && let Some(scene) = find_harness_scene(&outcome.state, *chapter_number, *scene_order)
            && scene.content_rating != spindle_core::models::ContentRating::Explicit
        {
            return Ok(AuthoringExecuteNextOutput {
                run_id: run_id.clone(),
                next_action: action_to_execute.to_string(),
                executed_action: "none".to_string(),
                message: format!(
                    "Host draft required for non-explicit scene {chapter_number}.{scene_order}. \
                     Draft this scene in the active assistant chat using Spindle context, then call authoring_save_scene_draft \
                     with project_id={}, run_id={}, book_number={}, chapter_number={}, \
                     scene_order={}, content_rating={}, full_text, summary, character_states, canonical_facts, \
                     relationship_updates, beats, and continuity_notes. This structured continuity package is required; \
                     use continuity_notes to explicitly say when the scene introduces no durable canon changes. \
                     Then call authoring_execute_next again. Use mode='agent' only when you explicitly \
                     want Spindle to offload non-explicit drafting to the configured draft route.",
                    project_id,
                    run_id,
                    outcome.state.book_number,
                    chapter_number,
                    scene_order,
                    scene.content_rating.as_str()
                ),
                status: run.status.clone(),
            });
        }

        // Hybrid-mode revision hand-off (evolution §3.2, I2). VerifyScene runs
        // host-independently (it falls through to the executor below), but a
        // ReviseScene in hybrid mode means the HOST is the revision agent: list
        // the scene-scoped findings and instruct a revise + re-save. The re-save
        // through authoring_save_scene_draft resets verify_status and counts the
        // attempt, so the loop re-verifies exactly as the agent path does. In
        // agent mode this arm is skipped and ReviseScene re-drafts automatically.
        if let NextAction::ReviseScene {
            chapter_number,
            scene_order,
            attempt,
        } = &action_to_execute
            && !agent_mode
        {
            let findings = self
                .authoring_scene_verify_findings(
                    data_dir,
                    &project_id,
                    outcome.state.book_number,
                    *chapter_number,
                    *scene_order,
                )
                .await
                .unwrap_or_default();
            let findings_block = if findings.is_empty() {
                "(no findings resolved at read time)".to_string()
            } else {
                findings
                    .iter()
                    .take(12)
                    .map(|(check_type, message)| format!("- [{check_type}] {message}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            return Ok(AuthoringExecuteNextOutput {
                run_id: run_id.clone(),
                next_action: action_to_execute.to_string(),
                executed_action: "none".to_string(),
                message: format!(
                    "Host revision required for scene {chapter_number}.{scene_order} \
                     (revision attempt {attempt}). The saved draft tripped these scene-scoped \
                     checks:\n{findings_block}\n\nRevise the scene to resolve each finding, then \
                     re-save it with authoring_save_scene_draft (project_id={project_id}, \
                     run_id={run_id}, book_number={}, chapter_number={chapter_number}, \
                     scene_order={scene_order}) and call authoring_execute_next again. The re-save \
                     re-verifies the revised draft; if the same findings persist the scene is \
                     parked at the checkpoint rather than revised again.",
                    outcome.state.book_number,
                ),
                status: run.status.clone(),
            });
        }

        let state_path = authoring_state_path(data_dir, &run_id);
        outcome.state.save(&state_path)?;

        let exec_result = match &action_to_execute {
            NextAction::RunCheckpoint {
                start_chapter,
                end_chapter,
            } => {
                self.execute_authoring_run_checkpoint(
                    &state_path,
                    outcome.state,
                    *start_chapter,
                    *end_chapter,
                )
                .await
            }
            NextAction::CommitSceneChanges {
                chapter_number,
                scene_order,
                ..
            } => {
                self.execute_authoring_commit_scene_changes(
                    &state_path,
                    outcome.state,
                    *chapter_number,
                    *scene_order,
                )
                .await
            }
            NextAction::AnnotateSceneBeats {
                chapter_number,
                scene_order,
                ..
            } => {
                self.execute_authoring_annotate_scene_beats(
                    &state_path,
                    outcome.state,
                    *chapter_number,
                    *scene_order,
                )
                .await
            }
            NextAction::SaveChapterSummary { chapter_number } => {
                self.execute_authoring_save_chapter_summary(
                    &state_path,
                    outcome.state,
                    *chapter_number,
                    &run_id,
                )
                .await
            }
            _ => {
                // Lazy primacy claim (bug 4a): if no live primary owns the addr
                // file yet, claim it here (bind the internal listener, write the
                // addr file) and proceed. Idempotent + race-safe.
                let active_addr =
                    crate::internal_listener::ensure_primary_addr(&self.service, data_dir).await?;
                let url = format!("http://{}/mcp", active_addr);
                let client = McpHarnessClient::connect(&TransportConfig::Http { url }).await?;
                execute_one(
                    &state_path,
                    outcome.state,
                    &client,
                    action_to_execute.clone(),
                )
                .await
            }
        };

        let _ = std::fs::remove_file(&state_path);

        let exec_res = exec_result?;

        let updated_snapshot = self.authoring_project_snapshot(&exec_res.state).await?;
        let updated_outcome = reconcile_state(exec_res.state.clone(), &updated_snapshot);

        let mut final_status = "active".to_string();
        if updated_outcome.has_errors() {
            final_status = "blocked".to_string();
        } else {
            for ch in &exec_res.state.chapters {
                for sc in &ch.scenes {
                    if sc.blocked_reason.is_some() {
                        final_status = "blocked".to_string();
                    }
                }
            }
        }
        if updated_outcome.next_action == NextAction::Complete {
            final_status = "completed".to_string();
        }
        if matches!(
            updated_outcome.next_action,
            NextAction::AwaitCheckpointReview { .. } | NextAction::AwaitResearch { .. }
        ) {
            final_status = "blocked".to_string();
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
            &run_id,
            &exec_res.state,
            &final_status,
            Some(run.created_at),
        );
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2): step events derived from the executed action + the
        // committed after-state, plus any run-status transition. Emitted AFTER
        // the save commits (D3.3); a journal error never fails the step.
        authoring_emit_step_events(
            repo,
            &run_id,
            &action_to_execute,
            &exec_res.state,
            &run.status,
            &final_status,
        )
        .await;

        Ok(AuthoringExecuteNextOutput {
            run_id: run_id.clone(),
            next_action: updated_outcome.next_action.to_string(),
            executed_action: action_to_execute.to_string(),
            message: exec_res.message,
            status: final_status,
        })
    }

    async fn handle_authoring_save_scene_draft(
        &self,
        mut input: AuthoringSaveSceneDraftInput,
    ) -> anyhow::Result<AuthoringSaveSceneDraftOutput> {
        let structured_update_count = authoring_structured_update_count(&input);
        if structured_update_count == 0 {
            anyhow::bail!(
                "authoring_save_scene_draft requires a structured continuity package: \
                 provide character_states, canonical_facts, relationship_updates, beats, \
                 or continuity_notes. If the scene introduces no durable canon changes, \
                 add a continuity_notes entry saying that explicitly."
            );
        }

        let project_id = input.project_id.clone();
        let repo = self.service.repository();
        let run_id = match input.run_id.clone() {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;
        if run.status == "completed" || run.status == "paused" {
            anyhow::bail!(
                "authoring run {} is {}; cannot save a scene draft",
                run_id,
                run.status
            );
        }

        let mut harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);
        let (chapter_index, scene_index) =
            authoring_scene_indices(&harness_state, input.chapter_number, input.scene_order)
                .with_context(|| {
                    format!(
                        "scene {}.{} not found in authoring run {}",
                        input.chapter_number, input.scene_order, run_id
                    )
                })?;

        let expected_rating = harness_state.chapters[chapter_index].scenes[scene_index]
            .content_rating
            .clone();
        if expected_rating != input.content_rating {
            anyhow::bail!(
                "scene {}.{} is planned as {}, but draft was saved as {}",
                input.chapter_number,
                input.scene_order,
                expected_rating.as_str(),
                input.content_rating.as_str()
            );
        }

        if input.full_text.trim() == "_keep_existing_" {
            let existing_scene_id = harness_state.chapters[chapter_index].scenes[scene_index]
                .scene_id
                .clone()
                .with_context(|| {
                    format!(
                        "scene {}.{} has no saved scene text to keep; provide full_text",
                        input.chapter_number, input.scene_order
                    )
                })?;
            let existing_scene = repo.get_scene(&existing_scene_id).await?;
            if existing_scene.project_id != project_id {
                anyhow::bail!(
                    "scene {} does not belong to project {}; cannot keep existing text",
                    existing_scene_id,
                    project_id
                );
            }
            input.full_text = existing_scene.full_text;
        }

        let save_output = self
            .service
            .save_scene_draft(authoring_save_scene_input(&input))
            .await?;

        let state_path = authoring_state_path(repo.data_dir(), &run_id);
        let artifact_store =
            ArtifactStore::new(authoring_artifacts_root(&state_path, &harness_state));
        let artifact_rel = harness_state.chapters[chapter_index].scenes[scene_index]
            .scene_artifact_path
            .clone()
            .unwrap_or_else(|| {
                ArtifactStore::scene_relative_path(input.chapter_number, input.scene_order)
            });

        let mut artifact = artifact_store
            .load_json::<SceneGenerationArtifact>(&artifact_rel)
            .unwrap_or_else(|_| {
                SceneGenerationArtifact::new(
                    input.chapter_number,
                    input.scene_order,
                    "host".to_string(),
                    "interactive-host".to_string(),
                    Some(input.content_rating.as_str().to_string()),
                    "Host-drafted scene saved through authoring_save_scene_draft.".to_string(),
                )
            });
        artifact.rating = Some(input.content_rating.as_str().to_string());
        artifact.completion_fragments = vec![input.full_text.clone()];
        artifact.adapter_kind = Some("host".to_string());
        artifact.model_name = Some("interactive-host".to_string());
        artifact.truncated = false;
        artifact.last_parse_error = None;
        artifact.package = Some(GeneratedScenePackage {
            full_text: input.full_text.clone(),
            summary: input.summary.clone(),
            tone: input.tone.clone(),
            character_states: input.character_states.clone(),
            canonical_facts: input.canonical_facts.clone(),
            relationship_updates: input.relationship_updates.clone(),
            beats: input.beats.clone(),
            continuity_notes: input.continuity_notes.clone(),
        });
        artifact.save_draft_output = Some(save_output.clone());
        artifact.commit_output = None;
        artifact.beat_annotation_output = None;
        artifact.research_source_ids = input.research_source_ids.clone();
        artifact.research_note_ids = input.research_note_ids.clone();
        artifact.research_claim_ids = input.research_claim_ids.clone();
        artifact.research_query_pack_input = input.research_query_pack_input.clone();
        artifact.research_context_hash = input.research_context_hash.clone();
        artifact_store.save_json(&artifact_rel, &artifact)?;

        let live_scene = &mut harness_state.chapters[chapter_index].scenes[scene_index];
        // In-run verify/revise (evolution §3.2, hybrid mode): a re-save that
        // follows a `findings` verify IS the host's revision. Reset verify_status
        // so the scheduler re-verifies the revised draft, and count the attempt
        // so the bounded budget is honored. Any other prior verify state (clean,
        // parked, error, or none) leaves the counters untouched — a first save,
        // or a save after the loop already converged, must not spend budget.
        let host_revision_attempt = if live_scene.verify_status.as_deref() == Some("findings") {
            live_scene.revise_attempts += 1;
            live_scene.verify_status = None;
            live_scene.verify_detail = None;
            Some(live_scene.revise_attempts)
        } else {
            None
        };
        live_scene.phase = ScenePhase::DraftSaved;
        live_scene.scene_id = Some(save_output.scene_id.clone());
        live_scene.scene_artifact_path = Some(artifact_rel.clone());
        live_scene.tone = input.tone.clone();
        live_scene.source_path = input.source_path.clone();
        live_scene.blocked_reason = None;

        // Summary invalidation (defect item 3): re-saving a scene in a chapter
        // whose summary was already persisted (the documented revise-during-
        // checkpoint flow) leaves that summary describing prose that no longer
        // exists. Delete the row and the summary artifact and clear the harness
        // flags so the pipeline regenerates both after the scene recommits —
        // otherwise the next execute blocks on the summary residue rule.
        let chapter_state = &mut harness_state.chapters[chapter_index];
        let persisted_summary = repo
            .get_chapter_summary(
                &project_id,
                &harness_state.active_branch_id,
                input.book_number,
                input.chapter_number,
            )
            .await?;
        if chapter_state.summary_saved || persisted_summary.is_some() {
            repo.delete_chapter_summaries_for_chapter(
                &project_id,
                &harness_state.active_branch_id,
                input.book_number,
                input.chapter_number,
            )
            .await?;
            let summary_rel = chapter_state
                .summary_artifact_path
                .clone()
                .unwrap_or_else(|| ArtifactStore::summary_relative_path(input.chapter_number));
            let summary_path = artifact_store.root().join(&summary_rel);
            match std::fs::remove_file(&summary_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            "failed to remove stale summary artifact at {}",
                            summary_path.display()
                        )
                    });
                }
            }
            chapter_state.summary_saved = false;
            chapter_state.summary_artifact_path = None;
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &run.status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2, emitted AFTER the save commits — D3.3). The host path
        // always persists a draft; a re-save after a `findings` verify is ALSO
        // the host's revision, so emit `scene_revised` in that case. Ordering:
        // draft first, then revised (the revision is a property of this save).
        let journal = RunJournal::new(repo);
        journal
            .emit(
                &run_id,
                "scene_drafted",
                run_journal::scene_drafted_payload(
                    input.chapter_number,
                    input.scene_order,
                    &save_output.scene_id,
                    "host",
                ),
            )
            .await;
        if let Some(attempt) = host_revision_attempt {
            journal
                .emit(
                    &run_id,
                    "scene_revised",
                    run_journal::scene_revised_payload(
                        input.chapter_number,
                        input.scene_order,
                        &save_output.scene_id,
                        attempt,
                        // The host authored the revision off the listed findings;
                        // the directive count is not re-plumbed here (0 = "count
                        // not recorded on the host path" — additive, prose-free).
                        0,
                    ),
                )
                .await;
        }

        Ok(AuthoringSaveSceneDraftOutput {
            run_id,
            scene_id: save_output.scene_id.clone(),
            scene_artifact_path: artifact_rel,
            status: save_output.status.clone(),
            structured_update_count,
            save_output,
        })
    }

    /// Run the scene-scoped deterministic check subset for one scene and return
    /// its actionable (severity ≥ warning) findings as `(check_type, message)`
    /// pairs, sorted and deduplicated (evolution §3.2). Used by the hybrid-mode
    /// revision hand-off to list findings for the host. Connects to the running
    /// HTTP MCP server (same transport the executor uses); the check is
    /// deterministic and issues zero model calls (`deep_check = false`).
    async fn authoring_scene_verify_findings(
        &self,
        data_dir: &Path,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> anyhow::Result<Vec<(String, String)>> {
        // Lazy primacy claim (bug 4a) — see the sibling call in
        // authoring_execute_next's dispatch arm.
        let active_addr =
            crate::internal_listener::ensure_primary_addr(&self.service, data_dir).await?;
        let url = format!("http://{}/mcp", active_addr);
        let client = McpHarnessClient::connect(&TransportConfig::Http { url }).await?;
        let scope = spindle_core::models::ConsistencyScopeInput {
            scene_order: Some(scene_order),
            ..spindle_core::models::ConsistencyScopeInput::chapter_range(
                book_number,
                chapter_number,
                book_number,
                chapter_number,
            )
        };
        let output = client
            .check_consistency(&spindle_core::models::CheckConsistencyInput {
                deep_scan_offset: None,
                project_id: project_id.to_string(),
                scope,
                checks: spindle_core::models::SCENE_VERIFY_CHECKS
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                severity_filter: Vec::new(),
                deep_check: Some(false),
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await?;
        let mut findings: Vec<(String, String)> = output
            .issues
            .into_iter()
            .filter(|issue| matches!(issue.severity.as_str(), "warning" | "error"))
            .map(|issue| (issue.check_type, issue.message))
            .collect();
        findings.sort();
        findings.dedup();
        Ok(findings)
    }

    async fn execute_authoring_commit_scene_changes(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        chapter_number: i32,
        scene_order: i32,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let (chapter_index, scene_index) =
            authoring_scene_indices(&state, chapter_number, scene_order).with_context(|| {
                format!("scene {chapter_number}.{scene_order} not found in authoring run state")
            })?;
        let scene = state.chapters[chapter_index].scenes[scene_index].clone();
        let scene_id = scene
            .scene_id
            .clone()
            .with_context(|| format!("scene_id missing for {chapter_number}.{scene_order}"))?;
        let artifact_path = scene.scene_artifact_path.clone();
        let mut artifact = artifact_path
            .as_ref()
            .map(|path| artifact_store.load_json::<SceneGenerationArtifact>(path))
            .transpose()?;
        let mut commit_output = artifact
            .as_ref()
            .and_then(|artifact| artifact.commit_output.clone());

        if commit_output.is_none() {
            let package = match artifact
                .as_ref()
                .and_then(|artifact| artifact.package.as_ref())
            {
                Some(package) => package,
                None => {
                    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
                    live_scene.blocked_reason = Some(
                        "missing structured continuity package; save host-authored scenes with authoring_save_scene_draft before committing".to_string(),
                    );
                    state.save(state_path)?;
                    anyhow::bail!(
                        "authoring scene {}.{} is missing its structured continuity package; \
                         call authoring_save_scene_draft with character_states, canonical_facts, \
                         relationship_updates, beats, and/or continuity_notes before resuming",
                        chapter_number,
                        scene_order
                    );
                }
            };
            let (character_states, canonical_facts, relationship_updates) = (
                package.character_states.clone(),
                package.canonical_facts.clone(),
                package.relationship_updates.clone(),
            );
            let new_commit_output = self
                .service
                .commit_scene_changes(CommitSceneChangesInput {
                    project_id: state.project_id.clone(),
                    scene_id: scene_id.clone(),
                    character_states,
                    canonical_facts,
                    relationship_updates,
                    accept_world_rule_risks: true,
                    // Autonomous supervisor commit: surface continuity findings on
                    // the output without halting the run.
                    accept_continuity_risks: false,
                    continuity_gate: Some(spindle_core::models::CommitContinuityGate::WarnOnly),
                })
                .await
                .with_context(|| {
                    format!(
                        "failed to commit scene changes for chapter {chapter_number} scene {scene_order}"
                    )
                })?;
            if let (Some(path), Some(artifact)) = (artifact_path.as_ref(), artifact.as_mut()) {
                artifact.commit_output = Some(new_commit_output.clone());
                artifact_store.save_json(path, artifact)?;
            }
            commit_output = Some(new_commit_output);
        }

        let commit_output = commit_output
            .as_ref()
            .context("scene artifact missing commit output")?;
        let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
        if authoring_commit_output_has_errors(commit_output) {
            let inspect_target = artifact_path
                .as_ref()
                .map(|path| artifact_store.root().join(path).display().to_string())
                .unwrap_or_else(|| scene_id.clone());
            let error_summary = authoring_commit_error_summary(commit_output);
            live_scene.blocked_reason = Some(format!(
                "commit_scene_changes applied partial results: {error_summary}. inspect {inspect_target} before continuing",
            ));
            state.save(state_path)?;
            anyhow::bail!(
                "commit_scene_changes reported per-item errors for chapter {} scene {}: {}",
                chapter_number,
                scene_order,
                error_summary
            );
        }

        live_scene.phase = ScenePhase::ChangesCommitted;
        live_scene.blocked_reason = None;
        state.save(state_path)?;
        Ok(ExecutionResult {
            state,
            message: format!(
                "Committed scene changes for chapter {chapter_number} scene {scene_order}"
            ),
        })
    }

    async fn execute_authoring_annotate_scene_beats(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        chapter_number: i32,
        scene_order: i32,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let (chapter_index, scene_index) =
            authoring_scene_indices(&state, chapter_number, scene_order).with_context(|| {
                format!("scene {chapter_number}.{scene_order} not found in authoring run state")
            })?;
        let scene = state.chapters[chapter_index].scenes[scene_index].clone();
        let scene_id = scene
            .scene_id
            .clone()
            .with_context(|| format!("scene_id missing for {chapter_number}.{scene_order}"))?;
        let artifact_path = scene.scene_artifact_path.clone();
        let mut artifact = artifact_path
            .as_ref()
            .map(|path| artifact_store.load_json::<SceneGenerationArtifact>(path))
            .transpose()?;
        let mut beat_annotation_output = artifact
            .as_ref()
            .and_then(|artifact| artifact.beat_annotation_output.clone());

        if beat_annotation_output.is_none() {
            let beats = artifact
                .as_ref()
                .and_then(|artifact| artifact.package.as_ref())
                .map(|package| package.beats.clone())
                .unwrap_or_default();
            let annotation_output = self
                .service
                .annotate_scene_beats(AnnotateSceneBeatsInput {
                    project_id: state.project_id.clone(),
                    scene_id,
                    beats,
                    motif_ids: Vec::new(),
                    theme_ids: Vec::new(),
                    conflict_ids: Vec::new(),
                    intensity: None,
                })
                .await
                .with_context(|| {
                    format!(
                        "failed to annotate beats for chapter {chapter_number} scene {scene_order}"
                    )
                })?;
            if let (Some(path), Some(artifact)) = (artifact_path.as_ref(), artifact.as_mut()) {
                artifact.beat_annotation_output = Some(annotation_output.clone());
                artifact_store.save_json(path, artifact)?;
            }
            beat_annotation_output = Some(annotation_output);
        }
        beat_annotation_output.context("missing annotate_scene_beats output")?;

        let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
        live_scene.phase = ScenePhase::BeatsAnnotated;
        live_scene.blocked_reason = None;
        state.save(state_path)?;
        Ok(ExecutionResult {
            state,
            message: format!("Annotated beats for chapter {chapter_number} scene {scene_order}"),
        })
    }

    async fn execute_authoring_save_chapter_summary(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        chapter_number: i32,
        run_id: &str,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let chapter_index = state
            .chapters
            .iter()
            .position(|chapter| chapter.chapter_number == chapter_number)
            .with_context(|| {
                format!("chapter {chapter_number} not found in authoring run state")
            })?;

        if state.chapters[chapter_index]
            .summary_artifact_path
            .is_none()
        {
            state.chapters[chapter_index].summary_artifact_path =
                Some(ArtifactStore::summary_relative_path(chapter_number));
            state.save(state_path)?;
        }

        let artifact_path = state.chapters[chapter_index]
            .summary_artifact_path
            .clone()
            .context("summary artifact path missing after initialization")?;
        let mut artifact = artifact_store
            .load_json::<ChapterSummaryArtifact>(&artifact_path)
            .unwrap_or_else(|_| {
                ChapterSummaryArtifact::new(
                    chapter_number,
                    "in-process".to_string(),
                    "deterministic-summary".to_string(),
                    "Synthesized from saved scene packages by authoring_execute_next.".to_string(),
                )
            });
        if artifact.chapter_number != chapter_number {
            anyhow::bail!(
                "summary artifact is for chapter {}, expected chapter {}",
                artifact.chapter_number,
                chapter_number
            );
        }

        // Stale-artifact guard (defect item 2): an on-disk artifact counts as
        // idempotency proof ONLY if it belongs to this run and the
        // chapter_summary row its save_summary_output references still exists
        // on the run's branch. An artifact from an earlier pass (different or
        // unstamped run_id, row since deleted) would otherwise mark
        // summary_saved without persisting anything AND silently reuse old-pass
        // summary content for new prose. Discard its outputs and regenerate
        // from the current scene packages.
        let foreign_run = artifact.run_id.as_deref().is_some_and(|id| id != run_id);
        let dangling_output = match artifact.save_summary_output.as_ref() {
            Some(output) if !foreign_run => {
                let existing = self
                    .service
                    .repository()
                    .get_chapter_summary(
                        &state.project_id,
                        &state.active_branch_id,
                        state.book_number,
                        chapter_number,
                    )
                    .await?;
                existing.is_none_or(|row| row.id != output.chapter_summary_id)
            }
            _ => false,
        };
        if foreign_run || dangling_output {
            artifact.clear_generation();
            artifact.save_summary_output = None;
            artifact.last_parse_error = None;
        }
        artifact.run_id = Some(run_id.to_string());

        let package = match artifact.package.clone() {
            Some(package) => package,
            None => {
                let package = authoring_build_chapter_summary_package(
                    &artifact_store,
                    &state.chapters[chapter_index],
                )?;
                artifact.package = Some(package.clone());
                artifact.truncated = false;
                artifact.last_parse_error = None;
                artifact_store.save_json(&artifact_path, &artifact)?;
                package
            }
        };

        if artifact.save_summary_output.is_none() {
            let save_output = self
                .service
                .save_summary(SaveSummaryInput {
                    project_id: state.project_id.clone(),
                    book_number: state.book_number,
                    chapter_number,
                    entity_type: None,
                    entity_id: None,
                    summary: package.summary.clone(),
                    key_events: package.key_events.clone(),
                    character_changes: package.character_changes.clone(),
                    relationship_shifts: package.relationship_shifts.clone(),
                    arc_advances: package.arc_advances.clone(),
                    promise_events: package.promise_events.clone(),
                })
                .await
                .with_context(|| format!("failed to save summary for chapter {chapter_number}"))?;
            artifact.save_summary_output = Some(save_output);
            artifact_store.save_json(&artifact_path, &artifact)?;
        }

        state.chapters[chapter_index].summary_saved = true;
        state.save(state_path)?;
        Ok(ExecutionResult {
            state,
            message: format!("Saved chapter summary for chapter {chapter_number}"),
        })
    }

    async fn execute_authoring_run_checkpoint(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        start_chapter: i32,
        end_chapter: i32,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let consistency = self
            .service
            .check_consistency(CheckConsistencyInput {
                deep_scan_offset: None,
                project_id: state.project_id.clone(),
                scope: ConsistencyScopeInput::chapter_range(
                    state.book_number,
                    start_chapter,
                    state.book_number,
                    end_chapter,
                ),
                checks: Vec::new(),
                severity_filter: vec![],
                deep_check: Some(false),
                subjects: vec![],
                format: None,
                budget_tokens: None,
            })
            .await
            .with_context(|| {
                format!(
                    "failed to run consistency check for chapters {start_chapter}-{end_chapter}"
                )
            })?;

        let sampled_scene_ids =
            authoring_sample_checkpoint_scene_ids(&state, start_chapter, end_chapter)?;

        let pacing_overview = self
            .service
            .read_project_resource(&state.project_id, "pacing/overview")
            .await?;
        let chapter_summaries = self
            .service
            .read_project_resource(&state.project_id, "chapter-summaries")
            .await?;
        let narrative_promises = self
            .service
            .read_project_resource(&state.project_id, "narrative-promises")
            .await?;

        let report_path = ArtifactStore::checkpoint_relative_path(start_chapter, end_chapter);
        let save_point = self
            .service
            .create_save_point(CreateSavePointInput {
                project_id: state.project_id.clone(),
                name: format!(
                    "checkpoint-b{}-ch{}-{}",
                    state.book_number, start_chapter, end_chapter
                ),
                description: Some(format!(
                    "Before editorial decision for book {} chapters {}-{}",
                    state.book_number, start_chapter, end_chapter
                )),
            })
            .await
            .with_context(|| {
                format!("failed to create save point for checkpoint {start_chapter}-{end_chapter}")
            })?;

        state.checkpoint_history.push(CheckpointRecord {
            start_chapter,
            end_chapter,
            save_point_id: save_point.save_point_id.clone(),
            status: CheckpointStatus::PendingReview,
            report_artifact_path: Some(report_path.clone()),
            auto_outcome: None,
            pending_manual_scene_ids: Vec::new(),
        });
        state.last_checkpoint_end_chapter = end_chapter;
        state.save(state_path)?;

        let sampled_review_instruction = format!(
            "Run dual-persona review for sampled scenes [{}], inspect this checkpoint report, \
             revise any fixable local craft/continuity/system-UI findings before approval, \
             then call authoring_review_checkpoint with operator directives. Do not ask \
             'revise or approve?' unless the finding requires an operator plot, canon, \
             content-boundary, relationship-direction, or author-intent decision.",
            sampled_scene_ids.join(", ")
        );
        let deep_consistency_instruction = format!(
            "Run check_consistency for project {} over book {} chapters {}-{} with deep_check=true, \
             then call authoring_record_checkpoint_audit with the returned consistency payload.",
            state.project_id, state.book_number, start_chapter, end_chapter
        );

        artifact_store.save_json(
            &report_path,
            &CheckpointReportArtifact {
                version: 1,
                start_chapter,
                end_chapter,
                save_point: save_point.clone(),
                consistency: serde_json::to_value(consistency)?,
                deep_consistency: None,
                deep_consistency_status: "pending_deep_consistency".to_string(),
                deep_consistency_instruction: deep_consistency_instruction.clone(),
                sampled_reviews: Vec::new(),
                sampled_review_status: "pending_dual_persona_review".to_string(),
                sampled_review_instruction: sampled_review_instruction.clone(),
                pacing_overview,
                chapter_summaries,
                narrative_promises,
                sampled_scene_ids,
                // Reader-sim runs later, inside the auto-checkpoint automation
                // (evolution §3.6); the report is created with no section.
                reader_sim: None,
            },
        )?;

        Ok(ExecutionResult {
            state,
            message: format!(
                "Created checkpoint for chapters {start_chapter}-{end_chapter}; awaiting deep consistency and sampled dual-persona review ({}) before operator checkpoint review. {} {}",
                save_point.save_point_id, deep_consistency_instruction, sampled_review_instruction
            ),
        })
    }

    async fn handle_authoring_record_checkpoint_audit(
        &self,
        input: AuthoringRecordCheckpointAuditInput,
    ) -> anyhow::Result<AuthoringRecordCheckpointAuditOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;
        let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);
        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, &run_id);

        let report_path = checkpoint_report_path(
            &harness_state,
            &state_path,
            input.start_chapter,
            input.end_chapter,
        )?;
        let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
            format!(
                "failed to read checkpoint report artifact {}",
                report_path.display()
            )
        })?;
        let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
            serde_json::from_str(&raw_report).with_context(|| {
                format!(
                    "failed to parse checkpoint report artifact {}",
                    report_path.display()
                )
            })?;

        report.deep_consistency = Some(input.deep_consistency);
        report.deep_consistency_status = "complete".to_string();
        report.deep_consistency_instruction =
            "Deep consistency audit recorded from a separate check_consistency(deep_check=true) call.".to_string();

        let report_json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&report_path, report_json).with_context(|| {
            format!(
                "failed to update checkpoint report artifact {}",
                report_path.display()
            )
        })?;

        Ok(AuthoringRecordCheckpointAuditOutput {
            run_id,
            message: format!(
                "Recorded deep consistency audit for checkpoint {}-{}.",
                input.start_chapter, input.end_chapter
            ),
            status: run.status,
        })
    }

    async fn handle_authoring_review_checkpoint(
        &self,
        input: AuthoringReviewCheckpointInput,
    ) -> anyhow::Result<AuthoringReviewCheckpointOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;

        let mut harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, &run_id);
        let report_path = checkpoint_report_path(
            &harness_state,
            &state_path,
            input.start_chapter,
            input.end_chapter,
        )?;
        let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
            format!(
                "failed to read checkpoint report artifact {}",
                report_path.display()
            )
        })?;
        let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
            serde_json::from_str(&raw_report).with_context(|| {
                format!(
                    "failed to parse checkpoint report artifact {}",
                    report_path.display()
                )
            })?;

        if report.deep_consistency_status == "pending_deep_consistency"
            || report.deep_consistency_status == "pending"
        {
            anyhow::bail!(
                "checkpoint {}-{} cannot be marked reviewed until deep consistency is recorded. \
                 Run check_consistency with deep_check=true for this chapter range, then call \
                 authoring_record_checkpoint_audit with the returned payload.",
                input.start_chapter,
                input.end_chapter
            );
        }

        let mut sampled_reviews = Vec::new();
        let mut missing_reviews = Vec::new();
        for scene_id in &report.sampled_scene_ids {
            match repo
                .get_dual_persona_review(&harness_state.active_branch_id, scene_id)
                .await?
            {
                Some(review) if review.status == "current" && review.rounds_completed >= 2 => {
                    sampled_reviews.push(serde_json::to_value(review)?);
                }
                Some(review) => missing_reviews.push(format!(
                    "{} (status={}, rounds={})",
                    scene_id, review.status, review.rounds_completed
                )),
                None => missing_reviews.push(format!("{scene_id} (missing)")),
            }
        }
        if !missing_reviews.is_empty() {
            anyhow::bail!(
                "checkpoint {}-{} cannot be marked reviewed until sampled dual-persona reviews are current: {}. \
                 Run run_dual_persona_review with rounds=2 for each sampled scene in the checkpoint report, \
                 then call authoring_review_checkpoint again.",
                input.start_chapter,
                input.end_chapter,
                missing_reviews.join(", ")
            );
        }
        if !report.sampled_scene_ids.is_empty() {
            report.sampled_reviews = sampled_reviews;
            report.sampled_review_status = "complete".to_string();
            report.sampled_review_instruction =
                "Sampled dual-persona reviews verified from the local Spindle database before operator checkpoint review.".to_string();
            let report_json = serde_json::to_string_pretty(&report)?;
            std::fs::write(&report_path, report_json).with_context(|| {
                format!(
                    "failed to update checkpoint report artifact {}",
                    report_path.display()
                )
            })?;
        }

        if let Some(unresolved) = authoring_unresolved_checkpoint_directive(&input.directives) {
            anyhow::bail!(
                "checkpoint {}-{} cannot be marked reviewed because a directive appears to leave fixable findings unresolved: {:?}. \
                 Apply the prose/canon fixes first, then call authoring_review_checkpoint with resolved directives.",
                input.start_chapter,
                input.end_chapter,
                unresolved
            );
        }

        harness_state.save(&state_path)?;

        let review_result = spindle_harness::operator::review_checkpoint(
            &mut harness_state,
            &state_path,
            input.start_chapter,
            input.end_chapter,
            &input.directives,
        );

        let _ = std::fs::remove_file(&state_path);

        let message = review_result?;
        let final_status = authoring_status_after_checkpoint_review(&harness_state);

        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, final_status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2): the operator review completed and persisted. Emit
        // `checkpoint_reviewed`, then the actual run-status transition it caused
        // (diff prev vs final): review of the final checkpoint drives the run
        // straight to `completed`; a mid-run checkpoint review unblocks it to
        // `active` (`run_resumed`). Emitted AFTER the save commits (D3.3).
        let journal = RunJournal::new(repo);
        journal
            .emit(
                &run_id,
                "checkpoint_reviewed",
                run_journal::checkpoint_reviewed_payload(
                    input.start_chapter,
                    input.end_chapter,
                    input.directives.len(),
                ),
            )
            .await;
        if run.status != final_status {
            match final_status {
                "completed" => {
                    journal
                        .emit(
                            &run_id,
                            "run_completed",
                            run_journal::run_status_payload(None),
                        )
                        .await;
                }
                "active" if run.status == "blocked" || run.status == "paused" => {
                    journal
                        .emit(
                            &run_id,
                            "run_resumed",
                            run_journal::run_status_payload(None),
                        )
                        .await;
                }
                _ => {}
            }
        }

        Ok(AuthoringReviewCheckpointOutput {
            run_id,
            message,
            status: final_status.to_string(),
        })
    }

    /// In-process auto-checkpoint automation (evolution §3.3, K3). Invoked from
    /// `handle_authoring_execute_next` when a run whose `checkpoint_policy` is
    /// `auto_advisory`/`auto_strict` reaches a pending checkpoint, INSTEAD of
    /// surfacing `AwaitCheckpointReview`. This is pure orchestration of the
    /// existing manual-checkpoint service paths — it introduces NO new review
    /// logic:
    ///
    /// 1. Deep consistency: `check_consistency(deep_check=true)` over the
    ///    checkpoint's chapter range (full default check set, as the manual flow
    ///    instructs operators), recorded into the report artifact the SAME way
    ///    `authoring_record_checkpoint_audit` records it.
    /// 2. Sampled dual-persona reviews (rounds=2) via `run_dual_persona_review`,
    ///    the exact service path the manual flow uses. Explicit-manual-fallback
    ///    (I3): a scene whose review dispatch fails `RatingNotCovered` (surfaced
    ///    from the chokepoint, NOT pre-checked) is marked pending-manual and is
    ///    dispatched NOWHERE else; if any scene is pending-manual the checkpoint
    ///    BLOCKS listing exactly those scenes — the rest of the automation still
    ///    completed and is recorded.
    /// 3. Verdict from the deep-consistency severity counts: `auto_advisory`
    ///    approves iff zero findings ≥ `warning` (info allowed); `auto_strict`
    ///    iff zero findings of ANY severity. Approve takes the SAME path
    ///    `authoring_review_checkpoint` takes (`operator::review_checkpoint` with
    ///    an honest auto-approval directive). Not approved → the checkpoint stays
    ///    pending_review exactly as manual, with a prose-free `blocked_reason`.
    /// 4. Journal (ADR D2): `checkpoint_auto_approved` on approval,
    ///    `checkpoint_blocked` on block — via the existing RunJournal.
    ///
    /// Cost note: there is NO per-checkpoint model-cost ceiling in v1 (open
    /// design per owner decision D-1); every sampled scene's review and the deep
    /// pass run unbudgeted. A future ADR may add a ceiling.
    ///
    /// Transport-level errors during automation (deep-check call fails, a review
    /// call fails for a NON-clearance reason) never fake completion (I8) and
    /// never crash the run: the checkpoint stays pending_review with a
    /// `blocked_reason` naming the failed step, returned as `AutoCheckpointOutcome`.
    async fn run_auto_checkpoint(
        &self,
        run_id: &str,
        policy: &str,
        harness_state: &mut HarnessState,
        start_chapter: i32,
        end_chapter: i32,
    ) -> anyhow::Result<AutoCheckpointOutcome> {
        let repo = self.service.repository();
        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, run_id);

        // ── Step 1: deep consistency over the checkpoint range ──
        // Any transport error here blocks the checkpoint honestly (I8) — the
        // automation never fabricates a clean audit.
        let deep = match self
            .service
            .check_consistency(CheckConsistencyInput {
                deep_scan_offset: None,
                project_id: harness_state.project_id.clone(),
                scope: ConsistencyScopeInput::chapter_range(
                    harness_state.book_number,
                    start_chapter,
                    harness_state.book_number,
                    end_chapter,
                ),
                checks: Vec::new(),
                severity_filter: Vec::new(),
                deep_check: Some(true),
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
        {
            Ok(output) => output,
            Err(error) => {
                tracing::warn!(
                    run_id,
                    step = "deep_consistency",
                    error = format!("{error:#}"),
                    "auto-checkpoint deep consistency failed; blocking checkpoint (I8)"
                );
                return Ok(auto_checkpoint_block(
                    harness_state,
                    start_chapter,
                    end_chapter,
                    "deep_consistency_failed",
                    Vec::new(),
                ));
            }
        };

        // Record the deep audit into the report artifact — the same shape
        // `handle_authoring_record_checkpoint_audit` writes.
        let deep_value = serde_json::to_value(&deep)?;
        let finding_counts = auto_checkpoint_severity_counts(&deep);
        if let Err(error) = authoring_record_checkpoint_deep_audit(
            harness_state,
            &state_path,
            start_chapter,
            end_chapter,
            deep_value,
        ) {
            tracing::warn!(
                run_id,
                step = "record_audit",
                error = format!("{error:#}"),
                "auto-checkpoint audit record failed; blocking checkpoint (I8)"
            );
            return Ok(auto_checkpoint_block(
                harness_state,
                start_chapter,
                end_chapter,
                "record_audit_failed",
                Vec::new(),
            ));
        }

        // ── Step 2: sampled dual-persona reviews (rounds=2) ──
        let sampled_scene_ids =
            authoring_sample_checkpoint_scene_ids(harness_state, start_chapter, end_chapter)?;
        let mut pending_manual: Vec<String> = Vec::new();
        for scene_id in &sampled_scene_ids {
            match self
                .service
                .run_dual_persona_review(spindle_core::models::RunDualPersonaReviewInput {
                    project_id: harness_state.project_id.clone(),
                    branch_id: Some(harness_state.active_branch_id.clone()),
                    scene_id: scene_id.clone(),
                    rounds: Some(2),
                })
                .await
            {
                Ok(_) => {}
                Err(error) => {
                    // Explicit-manual-fallback (I3): a RatingNotCovered rejection
                    // from the dispatch chokepoint means the `review` route's
                    // agent is not cleared for THIS scene's rating. That scene's
                    // prose was never dispatched anywhere (the chokepoint rejects
                    // before any model call); mark it pending-manual and DO NOT
                    // route it elsewhere. Any OTHER error is a transport failure:
                    // block the checkpoint naming the failed step (I8), never
                    // faking completion.
                    if auto_checkpoint_error_is_rating_not_covered(&error) {
                        pending_manual.push(scene_id.clone());
                    } else {
                        tracing::warn!(
                            run_id,
                            step = "dual_persona_review",
                            scene_id,
                            error = format!("{error:#}"),
                            "auto-checkpoint review failed at transport; blocking checkpoint (I8)"
                        );
                        return Ok(auto_checkpoint_block(
                            harness_state,
                            start_chapter,
                            end_chapter,
                            "review_dispatch_failed",
                            Vec::new(),
                        ));
                    }
                }
            }
        }

        // ── Step 2.5: cumulative reader-simulation pass (evolution §3.6) ──
        // Enrichment, not a gate: it reads each chapter in the range in order
        // with rolling memory, writes a `reader_sim` section into the report and
        // a per-run rolling notes artifact, and NEVER alters the verdict (its
        // concerns are report-only, matching the sampled-review outcomes, which
        // also never fold into the finding counts). A transport error or an
        // uncleared rating skips that chapter honestly and the pass continues;
        // reader-sim skips never mark scenes pending-manual. It runs BEFORE the
        // pending-manual early-return so its enrichment section is always
        // recorded even when a sampled review fell back to manual (a
        // pending-manual block still short-circuits the verdict below). Box::pin
        // keeps this grown path off the auto-checkpoint stack frame
        // (decide_canon_deltas precedent).
        if let Err(error) = Box::pin(self.run_reader_sim_pass(
            run_id,
            &state_path,
            harness_state,
            start_chapter,
            end_chapter,
        ))
        .await
        {
            // A failure to WRITE the report/notes artifact is not a leak and is
            // not a verdict input; log and continue so reader-sim enrichment can
            // never itself block or fabricate a checkpoint outcome (I8).
            tracing::warn!(
                run_id,
                step = "reader_sim",
                error = format!("{error:#}"),
                "auto-checkpoint reader-sim pass failed to record; continuing (enrichment, not a gate)"
            );
        }

        // Any scene awaiting manual review blocks the checkpoint (partial
        // automation, zero leakage): the completed reviews + deep audit are
        // recorded, but the checkpoint stays pending_review listing exactly the
        // scenes the operator must review by hand.
        if !pending_manual.is_empty() {
            return Ok(auto_checkpoint_block(
                harness_state,
                start_chapter,
                end_chapter,
                "pending_manual_review",
                pending_manual,
            ));
        }

        // ── Step 3: verdict from the deep-consistency severity counts ──
        let errors = *finding_counts.get("error").unwrap_or(&0);
        let warnings = *finding_counts.get("warning").unwrap_or(&0);
        let infos = *finding_counts.get("info").unwrap_or(&0);
        let approve = match policy {
            "auto_advisory" => errors == 0 && warnings == 0,
            "auto_strict" => errors == 0 && warnings == 0 && infos == 0,
            // Defensive: an unknown policy never auto-approves.
            _ => false,
        };

        if approve {
            // Approve via the SAME path authoring_review_checkpoint takes: an
            // honest auto-approval directive, then operator::review_checkpoint.
            let directives = vec![format!("auto-approved under {policy}: 0 blocking findings")];
            harness_state.save(&state_path)?;
            let review_result = spindle_harness::operator::review_checkpoint(
                harness_state,
                &state_path,
                start_chapter,
                end_chapter,
                &directives,
            );
            let _ = std::fs::remove_file(&state_path);
            review_result?;
            if let Some(checkpoint) = harness_state
                .checkpoint_history
                .iter_mut()
                .find(|cp| cp.start_chapter == start_chapter && cp.end_chapter == end_chapter)
            {
                checkpoint.auto_outcome = Some("approved".to_string());
            }
            Ok(AutoCheckpointOutcome {
                approved: true,
                finding_counts,
                blocked_reason: None,
                pending_manual_scene_ids: Vec::new(),
            })
        } else {
            let _ = std::fs::remove_file(&state_path);
            // Blocked by findings: stay pending_review exactly as manual, with a
            // prose-free severity-count summary (ids/counts, no prose).
            let reason = format!(
                "auto_{policy_tail}_findings: errors={errors} warnings={warnings} info={infos}",
                policy_tail = policy.strip_prefix("auto_").unwrap_or(policy),
            );
            Ok(auto_checkpoint_block(
                harness_state,
                start_chapter,
                end_chapter,
                &reason,
                Vec::new(),
            ))
        }
    }

    /// Cumulative reader-simulation pass over a checkpoint range (evolution
    /// §3.6, P3.4). For each chapter start_chapter..=end_chapter IN ORDER, gather
    /// its scenes in spine order, derive the batch rating (the strictest across
    /// the chapter's scenes — `max_scene_rating`), load the reader's prior notes
    /// (rolling memory, updated BETWEEN chapters so memory accumulates chapter to
    /// chapter within this checkpoint), and call the service reader-sim pass
    /// (route `reader_sim` → `review` fallback, rating-gated). The rolling notes
    /// artifact (`reader-sim-notes.json`, one per run) is updated after each
    /// chapter; a `reader_sim` section is written into the checkpoint report.
    ///
    /// Enrichment, not a gate (I8): a skip (uncleared rating / transport error /
    /// no prose) or an unparsed read records an honest entry and the pass
    /// continues; reader-sim never marks scenes pending-manual and never alters
    /// the verdict.
    async fn run_reader_sim_pass(
        &self,
        run_id: &str,
        state_path: &Path,
        harness_state: &HarnessState,
        start_chapter: i32,
        end_chapter: i32,
    ) -> anyhow::Result<()> {
        use spindle_harness::artifacts::{
            CheckpointReaderSimChapter, CheckpointReaderSimSection, READER_SIM_NOTES_PATH,
            ReaderSimConcernEntry, ReaderSimHistoryEntry, ReaderSimNotesArtifact,
        };

        let artifacts_root = authoring_artifacts_root(state_path, harness_state);
        let artifact_store = ArtifactStore::new(artifacts_root);

        // Load (or start) the run's rolling reader-sim notes artifact.
        let mut notes_artifact = artifact_store
            .load_json::<ReaderSimNotesArtifact>(READER_SIM_NOTES_PATH)
            .unwrap_or_default();

        let mut report_chapters: Vec<CheckpointReaderSimChapter> = Vec::new();

        for chapter_number in start_chapter..=end_chapter {
            let Some(_chapter) = harness_state
                .chapters
                .iter()
                .find(|ch| ch.chapter_number == chapter_number)
            else {
                continue;
            };

            let read = self
                .service
                .read_episode(spindle_core::models::ReadEpisodeInput {
                    project_id: harness_state.project_id.clone(),
                    book_number: harness_state.book_number,
                    chapter_number,
                    branch_id: Some(harness_state.active_branch_id.clone()),
                    force: false,
                })
                .await?;
            let outcome = read.outcome;

            let concerns_count = outcome.concerns.len();
            let skipped_reason = if outcome.status == "skipped" {
                outcome.skip_reason.clone()
            } else {
                None
            };

            // Update the rolling notes artifact after THIS chapter so the next
            // chapter's prompt carries the accumulated memory. On a read, adopt
            // the model's replacement notes and advance the watermark; on an
            // unparsed read or a skip, keep the prior notes (the reader loses no
            // memory) and do NOT advance the watermark.
            if outcome.status == "read" {
                notes_artifact.notes = outcome.notes.clone();
                notes_artifact.updated_through_chapter = chapter_number;
            }
            notes_artifact.history.push(ReaderSimHistoryEntry {
                range: format!("{chapter_number}..{chapter_number}"),
                engagement: outcome.engagement.clone(),
                concerns_count,
            });
            // Persist the rolling artifact each chapter (crash-safe memory).
            artifact_store.save_json(READER_SIM_NOTES_PATH, &notes_artifact)?;

            report_chapters.push(CheckpointReaderSimChapter {
                memory: Some(read.memory),
                chapter: chapter_number,
                engagement: outcome.engagement.clone(),
                concerns: outcome
                    .concerns
                    .iter()
                    .map(|c| ReaderSimConcernEntry {
                        severity: c.severity.clone(),
                        description: c.description.clone(),
                    })
                    .collect(),
                skipped_reason,
            });

            tracing::debug!(
                run_id,
                chapter = chapter_number,
                engagement = outcome.engagement.as_str(),
                status = outcome.status.as_str(),
                concerns = concerns_count,
                "reader-sim chapter recorded"
            );
        }

        // Fold the section into the checkpoint report (additive; report-only).
        let report_path =
            checkpoint_report_path(harness_state, state_path, start_chapter, end_chapter)?;
        let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
            format!(
                "failed to read checkpoint report artifact {}",
                report_path.display()
            )
        })?;
        let mut report: CheckpointReportArtifact =
            serde_json::from_str(&raw_report).with_context(|| {
                format!(
                    "failed to parse checkpoint report artifact {}",
                    report_path.display()
                )
            })?;
        report.reader_sim = Some(CheckpointReaderSimSection {
            chapters: report_chapters,
            notes_artifact_path: READER_SIM_NOTES_PATH.to_string(),
        });
        let report_json = serde_json::to_string_pretty(&report)?;
        std::fs::write(&report_path, report_json).with_context(|| {
            format!(
                "failed to update checkpoint report artifact {}",
                report_path.display()
            )
        })?;

        Ok(())
    }

    async fn handle_authoring_resolve_block(
        &self,
        input: AuthoringResolveBlockInput,
    ) -> anyhow::Result<AuthoringResolveBlockOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;

        let mut harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        // `redraft` is not a forward phase: it resets a poisoned/unparseable
        // scene BACK to pending-draft so the next execute re-drafts fresh (BUG
        // 3). The forward phases stay unchanged; anything else errors, and the
        // whitelist message now lists redraft.
        let target = input.target_phase.to_ascii_lowercase();
        let parsed_phase = match target.as_str() {
            "pending" => Some(spindle_harness::state::ScenePhase::Pending),
            "draft_saved" => Some(spindle_harness::state::ScenePhase::DraftSaved),
            "changes_committed" => Some(spindle_harness::state::ScenePhase::ChangesCommitted),
            "beats_annotated" => Some(spindle_harness::state::ScenePhase::BeatsAnnotated),
            "redraft" => None,
            _ => anyhow::bail!(
                "Invalid target phase: {} (expected one of: pending, draft_saved, changes_committed, beats_annotated, redraft)",
                input.target_phase
            ),
        };

        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, &run_id);
        harness_state.save(&state_path)?;

        // Distinguish scene-level from run-level blocks (defect item 1c): the
        // reconcile pass against live Spindle state is what raises run-level
        // errors (residue rules, branch mismatch, …). When it has errors, a
        // forward advance is allowed even on a scene with no blocked_reason.
        let run_level_blocked = {
            let snapshot = self.authoring_project_snapshot(&harness_state).await?;
            reconcile_state(harness_state.clone(), &snapshot).has_errors()
        };

        let resolve_result = match parsed_phase {
            Some(phase) => spindle_harness::operator::resolve_scene_block(
                &mut harness_state,
                &state_path,
                input.chapter_number,
                input.scene_order,
                phase,
                run_level_blocked,
            ),
            None => spindle_harness::operator::redraft_scene_block(
                &mut harness_state,
                &state_path,
                input.chapter_number,
                input.scene_order,
            ),
        };

        let _ = std::fs::remove_file(&state_path);

        let message = resolve_result?;

        let snapshot = self.authoring_project_snapshot(&harness_state).await?;
        let outcome = reconcile_state(harness_state.clone(), &snapshot);

        let mut final_status = "active".to_string();
        if outcome.has_errors() {
            final_status = "blocked".to_string();
        } else {
            for ch in &harness_state.chapters {
                for sc in &ch.scenes {
                    if sc.blocked_reason.is_some() {
                        final_status = "blocked".to_string();
                    }
                }
            }
        }
        if outcome.next_action == NextAction::Complete {
            final_status = "completed".to_string();
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &final_status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal: resolve_block emits NOTHING — neither the pre-existing forward
        // phases nor `redraft` journal a run event. The ADR D2 vocabulary has no
        // kind for an operator manual scene reset, `pass_skipped` is for a run
        // PASS being skipped (not an operator action), and inventing a kind is a
        // one-way door (D3.4). `authoring_status` (DB state) stays the source of
        // truth, so the reset is observable there without a journal entry.

        Ok(AuthoringResolveBlockOutput {
            run_id,
            message,
            status: final_status,
        })
    }

    async fn handle_authoring_cancel_run(
        &self,
        input: AuthoringCancelRunInput,
    ) -> anyhow::Result<AuthoringCancelRunOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;

        let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        let status = "paused".to_string();
        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &status, Some(run.created_at));

        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2): a genuine status transition into `paused`. Only emit
        // when the status actually changed (diff prev vs new — not on a no-op
        // re-pause). Emitted AFTER the save commits (D3.3).
        if run.status != status {
            RunJournal::new(repo)
                .emit(&run_id, "run_paused", run_journal::run_status_payload(None))
                .await;
        }

        let message = format!("Successfully paused authoring run {}", run_id);
        Ok(AuthoringCancelRunOutput {
            run_id,
            message,
            status,
        })
    }
}

fn extract_scene_location_and_rating(
    summary: &str,
    purpose: &str,
) -> (Option<String>, Option<spindle_core::models::ContentRating>) {
    let combined = format!("{} {}", summary, purpose).to_ascii_lowercase();
    let combined_orig = format!("{} {}", summary, purpose);

    // 1. Extract location
    let location_id = if let Some(idx) = combined.find("location:") {
        let start = idx + "location:".len();
        let remaining = &combined[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != ':')
            .unwrap_or(remaining.len());
        let id_str = &combined_orig[idx..idx + "location:".len() + end];
        Some(id_str.trim().to_string())
    } else {
        None
    };

    // 2. Extract content rating
    let rating = if combined.contains("rating:explicit") || combined.contains("explicit rating") {
        Some(spindle_core::models::ContentRating::Explicit)
    } else if combined.contains("rating:mature") || combined.contains("mature rating") {
        Some(spindle_core::models::ContentRating::Mature)
    } else if combined.contains("rating:teen") || combined.contains("teen rating") {
        Some(spindle_core::models::ContentRating::Teen)
    } else if combined.contains("rating:general") || combined.contains("general rating") {
        Some(spindle_core::models::ContentRating::General)
    } else {
        if combined.contains("rating: explicit") {
            Some(spindle_core::models::ContentRating::Explicit)
        } else if combined.contains("rating: mature") {
            Some(spindle_core::models::ContentRating::Mature)
        } else if combined.contains("rating: teen") {
            Some(spindle_core::models::ContentRating::Teen)
        } else if combined.contains("rating: general") {
            Some(spindle_core::models::ContentRating::General)
        } else {
            None
        }
    };

    (location_id, rating)
}

fn planned_scene_location_and_rating(
    location_id: Option<&str>,
    content_rating: Option<&spindle_core::models::ContentRating>,
    summary: &str,
    purpose: &str,
) -> (Option<String>, Option<spindle_core::models::ContentRating>) {
    let (legacy_location_id, legacy_rating) = extract_scene_location_and_rating(summary, purpose);
    let location_id = location_id
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .or(legacy_location_id);
    let rating = content_rating.cloned().or(legacy_rating);
    (location_id, rating)
}

fn authoring_execute_uses_agent_drafting(mode: Option<&str>) -> bool {
    let Some(mode) = mode else {
        return false;
    };
    matches!(
        mode.trim().to_ascii_lowercase().as_str(),
        "agent" | "agents" | "auto" | "automated" | "offload" | "full_agent" | "fully_automated"
    )
}

fn find_harness_scene(
    state: &spindle_harness::state::HarnessState,
    chapter_number: i32,
    scene_order: i32,
) -> Option<&spindle_harness::state::SceneState> {
    state
        .chapters
        .iter()
        .find(|chapter| chapter.chapter_number == chapter_number)?
        .scenes
        .iter()
        .find(|scene| scene.scene_order == scene_order)
}

/// Emit the ADR 0002 D2 journal events for one executed `authoring_execute_next`
/// step, derived from the executed action + the committed after-state, plus any
/// run-status transition (prev → new). Called AFTER the run state is persisted
/// (ADR D3.3); every append goes through [`RunJournal`], so a journaling error
/// never fails the step.
///
/// The step event is keyed off the typed `action` (never a rendered string) and
/// reads the scene's post-execute status fields (`scene_id`, `verify_status`,
/// `mine_status`, …) from `after_state`. The run-status event is diff-derived:
/// it fires only when `prev_status != new_status`, so a run that stays `active`
/// across a step emits no spurious transition.
async fn authoring_emit_step_events(
    repo: &spindle_adapters::sqlite::Repository,
    run_id: &str,
    action: &NextAction,
    after_state: &spindle_harness::state::HarnessState,
    prev_status: &str,
    new_status: &str,
) {
    let journal = RunJournal::new(repo);

    match action {
        NextAction::DraftScene {
            chapter_number,
            scene_order,
        } => {
            // Reaching the executor for a DraftScene means the agent path drafted
            // it (the host path returns early before executing). Emit the agent
            // origin; the host origin is emitted from save_scene_draft.
            if let Some(scene) = find_harness_scene(after_state, *chapter_number, *scene_order)
                && let Some(scene_id) = scene.scene_id.as_deref()
            {
                journal
                    .emit(
                        run_id,
                        "scene_drafted",
                        run_journal::scene_drafted_payload(
                            *chapter_number,
                            *scene_order,
                            scene_id,
                            "agent",
                        ),
                    )
                    .await;
            }
        }
        NextAction::VerifyScene {
            chapter_number,
            scene_order,
        } => {
            if let Some(scene) = find_harness_scene(after_state, *chapter_number, *scene_order)
                && let Some(scene_id) = scene.scene_id.as_deref()
                && let Some(status) = scene.verify_status.as_deref()
            {
                journal
                    .emit(
                        run_id,
                        "scene_verify_completed",
                        run_journal::verify_completed_payload(
                            *chapter_number,
                            *scene_order,
                            scene_id,
                            status,
                            scene.verify_detail.as_deref(),
                        ),
                    )
                    .await;
                // A parked/error verify is a skipped revision pass (ADR D2
                // `pass_skipped`): the detail carries the prose-free reason.
                if matches!(status, "parked_findings" | "error") {
                    journal
                        .emit(
                            run_id,
                            "pass_skipped",
                            run_journal::pass_skipped_payload(
                                "verify",
                                Some(*chapter_number),
                                Some(*scene_order),
                                scene.verify_detail.as_deref().unwrap_or(status),
                            ),
                        )
                        .await;
                }
            }
        }
        NextAction::ReviseScene {
            chapter_number,
            scene_order,
            attempt,
        } => {
            if let Some(scene) = find_harness_scene(after_state, *chapter_number, *scene_order)
                && let Some(scene_id) = scene.scene_id.as_deref()
            {
                journal
                    .emit(
                        run_id,
                        "scene_revised",
                        run_journal::scene_revised_payload(
                            *chapter_number,
                            *scene_order,
                            scene_id,
                            *attempt,
                            // Post-revise the directives are cleared; the attempt
                            // count is the durable signal. Directive count is not
                            // recovered here (0 = not recorded on this path).
                            0,
                        ),
                    )
                    .await;
            }
        }
        NextAction::CommitSceneChanges {
            chapter_number,
            scene_order,
            scene_id,
        } => {
            journal
                .emit(
                    run_id,
                    "scene_committed",
                    run_journal::scene_ref_payload(*chapter_number, *scene_order, scene_id),
                )
                .await;
        }
        NextAction::MineScene {
            chapter_number,
            scene_order,
        } => {
            if let Some(scene) = find_harness_scene(after_state, *chapter_number, *scene_order)
                && let Some(scene_id) = scene.scene_id.as_deref()
                && let Some(status) = scene.mine_status.as_deref()
            {
                journal
                    .emit(
                        run_id,
                        "scene_mined",
                        run_journal::scene_mined_payload(
                            *chapter_number,
                            *scene_order,
                            scene_id,
                            status,
                            scene.mine_detail.as_deref(),
                        ),
                    )
                    .await;
                // A skipped/error mine is a skipped pass (ADR D2 `pass_skipped`).
                if matches!(status, "skipped" | "model_output_rejected" | "error") {
                    journal
                        .emit(
                            run_id,
                            "pass_skipped",
                            run_journal::pass_skipped_payload(
                                "mine",
                                Some(*chapter_number),
                                Some(*scene_order),
                                scene.mine_detail.as_deref().unwrap_or(status),
                            ),
                        )
                        .await;
                }
            }
        }
        NextAction::AnnotateSceneBeats {
            chapter_number,
            scene_order,
            scene_id,
        } => {
            journal
                .emit(
                    run_id,
                    "beats_annotated",
                    run_journal::scene_ref_payload(*chapter_number, *scene_order, scene_id),
                )
                .await;
        }
        NextAction::SaveChapterSummary { chapter_number } => {
            let artifact_path = after_state
                .chapters
                .iter()
                .find(|chapter| chapter.chapter_number == *chapter_number)
                .and_then(|chapter| chapter.summary_artifact_path.as_deref());
            journal
                .emit(
                    run_id,
                    "chapter_summarized",
                    run_journal::chapter_summarized_payload(*chapter_number, artifact_path),
                )
                .await;
        }
        NextAction::ReplanChapter { chapter_number } => {
            // The chapter's post-summary replan outcome (ADR 0003 §3.5). The
            // reserved `replan_proposed` kind (ADR 0002 D2) activates ONLY when
            // the pass staged amendments (status `staged`, count > 0). A
            // skip/error/no-target outcome is a skipped pass (`pass_skipped`).
            // The count is recovered from the prose-free detail string ("staged
            // N amendment(s)"), mirroring the scene_mined staged-count recovery.
            if let Some(chapter) = after_state
                .chapters
                .iter()
                .find(|chapter| chapter.chapter_number == *chapter_number)
                && let Some(status) = chapter.replan_status.as_deref()
            {
                let count = chapter
                    .replan_detail
                    .as_deref()
                    .and_then(run_journal::leading_count_pub)
                    .unwrap_or(0);
                if status == "staged" && count > 0 {
                    journal
                        .emit(
                            run_id,
                            "replan_proposed",
                            run_journal::replan_proposed_payload(*chapter_number, count as usize),
                        )
                        .await;
                } else {
                    journal
                        .emit(
                            run_id,
                            "pass_skipped",
                            run_journal::pass_skipped_payload(
                                "replan",
                                Some(*chapter_number),
                                None,
                                chapter.replan_detail.as_deref().unwrap_or(status),
                            ),
                        )
                        .await;
                }
            }
        }
        NextAction::RunCheckpoint {
            start_chapter,
            end_chapter,
        } => {
            if let Some(checkpoint) = after_state
                .checkpoint_history
                .iter()
                .rev()
                .find(|cp| cp.start_chapter == *start_chapter && cp.end_chapter == *end_chapter)
            {
                let sampled = authoring_checkpoint_sampled_scene_ids(after_state, checkpoint);
                journal
                    .emit(
                        run_id,
                        "checkpoint_created",
                        run_journal::checkpoint_created_payload(
                            *start_chapter,
                            *end_chapter,
                            &checkpoint.save_point_id,
                            &sampled,
                        ),
                    )
                    .await;
            }
        }
        // Non-step actions (Blocked / Await* / Complete) never reach the
        // executor path that calls this helper — they early-return above.
        _ => {}
    }

    // Run-status transition (ADR D2). Diff-derived: only on an actual change.
    if prev_status != new_status {
        match new_status {
            "completed" => {
                journal
                    .emit(
                        run_id,
                        "run_completed",
                        run_journal::run_status_payload(None),
                    )
                    .await;
            }
            "blocked" => {
                journal
                    .emit(
                        run_id,
                        "run_blocked",
                        run_journal::run_status_payload(Some("blocked")),
                    )
                    .await;
            }
            "active" if prev_status == "blocked" || prev_status == "paused" => {
                journal
                    .emit(run_id, "run_resumed", run_journal::run_status_payload(None))
                    .await;
            }
            _ => {}
        }
    }
}

/// The sampled scene ids recorded on a checkpoint, for the `checkpoint_created`
/// payload. Recomputed from the after-state the same way the checkpoint report
/// samples them (ids only — no prose).
fn authoring_checkpoint_sampled_scene_ids(
    state: &spindle_harness::state::HarnessState,
    checkpoint: &spindle_harness::state::CheckpointRecord,
) -> Vec<String> {
    authoring_sample_checkpoint_scene_ids(state, checkpoint.start_chapter, checkpoint.end_chapter)
        .unwrap_or_default()
}

fn authoring_scene_indices(
    state: &HarnessState,
    chapter_number: i32,
    scene_order: i32,
) -> Option<(usize, usize)> {
    let chapter_index = state
        .chapters
        .iter()
        .position(|chapter| chapter.chapter_number == chapter_number)?;
    let scene_index = state.chapters[chapter_index]
        .scenes
        .iter()
        .position(|scene| scene.scene_order == scene_order)?;
    Some((chapter_index, scene_index))
}

fn authoring_artifacts_root(state_path: &Path, state: &HarnessState) -> PathBuf {
    let parent = state_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(&state.artifacts_dir)
}

fn authoring_commit_output_has_errors(output: &CommitSceneChangesOutput) -> bool {
    output
        .character_states
        .iter()
        .any(|item| item.error.is_some())
        || output
            .canonical_facts
            .iter()
            .any(|item| item.error.is_some())
        || output
            .relationship_updates
            .iter()
            .any(|item| item.error.is_some())
}

fn authoring_commit_error_summary(output: &CommitSceneChangesOutput) -> String {
    let mut errors = Vec::new();
    for item in &output.character_states {
        if let Some(error) = item.error.as_deref() {
            errors.push(format!("character_state {}: {}", item.character_id, error));
        }
    }
    for item in &output.canonical_facts {
        if let Some(error) = item.error.as_deref() {
            errors.push(format!(
                "canonical_fact {}:{}: {}",
                item.fact_type, item.key, error
            ));
        }
    }
    for item in &output.relationship_updates {
        if let Some(error) = item.error.as_deref() {
            errors.push(format!(
                "relationship {} -> {}: {}",
                item.character_a_id, item.character_b_id, error
            ));
        }
    }
    if errors.is_empty() {
        "no item-level errors were reported".to_string()
    } else {
        errors.join("; ")
    }
}

fn authoring_save_scene_input(input: &AuthoringSaveSceneDraftInput) -> SaveSceneDraftInput {
    SaveSceneDraftInput {
        project_id: input.project_id.clone(),
        book_number: input.book_number,
        chapter_number: input.chapter_number,
        chapter_id: input.chapter_id.clone(),
        scene_order: input.scene_order,
        full_text: input.full_text.clone(),
        authorship: input.authorship,
        summary: input.summary.clone(),
        content_rating: input.content_rating.clone(),
        tone: input.tone.clone(),
        generation_id: input.generation_id.clone(),
        source_path: input.source_path.clone(),
        location_id: input.location_id.clone(),
        research_source_ids: input.research_source_ids.clone(),
        research_note_ids: input.research_note_ids.clone(),
        research_claim_ids: input.research_claim_ids.clone(),
        research_query_pack_input: input.research_query_pack_input.clone(),
        research_context_hash: input.research_context_hash.clone(),
        knowledge_learned: input.knowledge_learned.clone(),
    }
}

fn authoring_structured_update_count(input: &AuthoringSaveSceneDraftInput) -> usize {
    input.character_states.len()
        + input.canonical_facts.len()
        + input.relationship_updates.len()
        + input.beats.len()
        + input.continuity_notes.len()
        + input.knowledge_learned.len()
}

fn authoring_status_after_checkpoint_review(state: &HarnessState) -> &'static str {
    if state.chapters.iter().any(|chapter| {
        chapter
            .scenes
            .iter()
            .any(|scene| scene.blocked_reason.is_some())
    }) {
        return "blocked";
    }
    if state
        .checkpoint_history
        .iter()
        .any(|checkpoint| checkpoint.status == CheckpointStatus::PendingReview)
    {
        return "blocked";
    }
    if state.last_checkpoint_end_chapter >= state.range.end_chapter
        && state.chapters.iter().all(|chapter| {
            matches!(
                chapter.status,
                spindle_harness::state::ChapterStatus::Complete
            )
        })
    {
        return "completed";
    }
    "active"
}

fn authoring_unresolved_checkpoint_directive(directives: &[String]) -> Option<String> {
    const UNRESOLVED_MARKERS: &[&str] = &[
        "acknowledged",
        "acknowledge",
        "fix in polish",
        "polish pass",
        "not actually fixed",
        "not fixed",
        "unfixed",
        "still unfixed",
        "defer",
        "deferred",
        "fix later",
        "later pass",
        "carry forward",
        "known issue",
        "known issues",
        "remaining issue",
        "remaining issues",
        "needs fix",
        "needs fixing",
        "todo",
    ];

    directives.iter().find_map(|directive| {
        let normalized = directive.to_ascii_lowercase();
        if UNRESOLVED_MARKERS
            .iter()
            .any(|marker| normalized.contains(marker))
        {
            Some(directive.clone())
        } else {
            None
        }
    })
}

fn authoring_build_chapter_summary_package(
    artifact_store: &ArtifactStore,
    chapter: &spindle_harness::state::ChapterState,
) -> anyhow::Result<GeneratedChapterSummaryPackage> {
    let mut scene_summaries = Vec::new();
    let mut key_events = Vec::new();
    let mut character_changes = Vec::new();
    let mut relationship_shifts = Vec::new();
    let mut arc_advances = Vec::new();
    let mut promise_events = Vec::new();

    for scene in &chapter.scenes {
        let artifact_path = scene.scene_artifact_path.as_ref().with_context(|| {
            format!(
                "chapter {} scene {} has no scene artifact path for summary",
                chapter.chapter_number, scene.scene_order
            )
        })?;
        let artifact: SceneGenerationArtifact = artifact_store
            .load_json(artifact_path)
            .with_context(|| format!("failed to load scene artifact {artifact_path}"))?;
        let package = artifact.package.as_ref().with_context(|| {
            format!("scene artifact {artifact_path} has no structured package for summary")
        })?;
        let scene_prefix = format!("Scene {}", scene.scene_order);

        if !package.summary.trim().is_empty() {
            scene_summaries.push(format!("{scene_prefix}: {}", package.summary.trim()));
        }

        for beat in &package.beats {
            if !beat.summary.trim().is_empty() {
                let beat_label = beat.beat_type.trim();
                let entry = if beat_label.is_empty() {
                    format!("{scene_prefix}: {}", beat.summary.trim())
                } else {
                    format!("{scene_prefix} [{beat_label}]: {}", beat.summary.trim())
                };
                if beat_label.eq_ignore_ascii_case("promise")
                    || beat_label.eq_ignore_ascii_case("narrative_promise")
                {
                    promise_events.push(entry.clone());
                }
                key_events.push(entry);
            }
        }

        for fact in &package.canonical_facts {
            let value = fact
                .value
                .as_deref()
                .or(fact.value_text.as_deref())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    fact.value_json
                        .as_ref()
                        .map(ToString::to_string)
                        .filter(|value| !value.is_empty())
                });
            let label = fact
                .key
                .as_deref()
                .or(fact.predicate.as_deref())
                .unwrap_or("canonical fact")
                .trim();
            if let Some(value) = value {
                key_events.push(format!("{scene_prefix}: {label} = {value}"));
            }
        }

        for state_change in &package.character_states {
            if let Some(summary) = state_change.changes.source_summary.as_deref()
                && !summary.trim().is_empty()
            {
                character_changes.push(format!(
                    "{scene_prefix}: {} - {}",
                    state_change.character_id,
                    summary.trim()
                ));
            }
            if let Some(notes) = state_change.changes.notes.as_ref() {
                for note in notes {
                    if !note.trim().is_empty() {
                        character_changes.push(format!(
                            "{scene_prefix}: {} - {}",
                            state_change.character_id,
                            note.trim()
                        ));
                    }
                }
            }
            if let Some(statuses) = state_change.changes.status.as_ref() {
                for status in statuses {
                    if !status.trim().is_empty() {
                        character_changes.push(format!(
                            "{scene_prefix}: {} status - {}",
                            state_change.character_id,
                            status.trim()
                        ));
                    }
                }
            }
            if let Some(goals) = state_change.changes.goals.as_ref() {
                for goal in goals {
                    if !goal.trim().is_empty() {
                        arc_advances.push(format!(
                            "{scene_prefix}: {} goal - {}",
                            state_change.character_id,
                            goal.trim()
                        ));
                    }
                }
            }
        }

        for relationship in &package.relationship_updates {
            relationship_shifts.push(format!(
                "{scene_prefix}: {} <-> {} trust {:+}, tension {:+}: {}",
                relationship.character_a_id,
                relationship.character_b_id,
                relationship.trust_delta,
                relationship.tension_delta,
                relationship.reason.trim()
            ));
        }

        for note in &package.continuity_notes {
            if !note.trim().is_empty() {
                arc_advances.push(format!("{scene_prefix}: {}", note.trim()));
            }
        }
    }

    if key_events.is_empty() {
        key_events = scene_summaries.clone();
    }

    let summary = if scene_summaries.is_empty() {
        chapter.synopsis.clone()
    } else {
        format!("{} {}", chapter.synopsis.trim(), scene_summaries.join(" "))
            .trim()
            .to_string()
    };

    Ok(GeneratedChapterSummaryPackage {
        summary,
        key_events: authoring_dedup_preserve_order(key_events),
        character_changes: authoring_dedup_preserve_order(character_changes),
        relationship_shifts: authoring_dedup_preserve_order(relationship_shifts),
        arc_advances: authoring_dedup_preserve_order(arc_advances),
        promise_events: authoring_dedup_preserve_order(promise_events),
    })
}

fn authoring_dedup_preserve_order(items: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut deduped = Vec::new();
    for item in items {
        let normalized = item.trim().to_string();
        if normalized.is_empty() || !seen.insert(normalized.clone()) {
            continue;
        }
        deduped.push(normalized);
    }
    deduped
}

fn authoring_sample_checkpoint_scene_ids(
    state: &HarnessState,
    start_chapter: i32,
    end_chapter: i32,
) -> anyhow::Result<Vec<String>> {
    let selected_chapters = [
        start_chapter,
        start_chapter + ((end_chapter - start_chapter) / 2),
        end_chapter,
    ];
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    for chapter_number in selected_chapters {
        if !seen.insert(chapter_number) {
            continue;
        }
        let chapter = state
            .chapters
            .iter()
            .find(|chapter| chapter.chapter_number == chapter_number)
            .with_context(|| format!("checkpoint chapter {} missing from state", chapter_number))?;
        let scene = if chapter_number == end_chapter {
            chapter.scenes.last()
        } else {
            chapter.scenes.first()
        }
        .with_context(|| format!("checkpoint chapter {} has no scenes", chapter_number))?;
        let scene_id = scene.scene_id.clone().with_context(|| {
            format!(
                "checkpoint chapter {} scene {} has no scene_id",
                chapter_number, scene.scene_order
            )
        })?;
        candidates.push(scene_id);
    }
    Ok(candidates)
}

/// Map a run's persisted `checkpoint_policy` to the auto policy in force, or
/// `None` for manual (the default). `None`/`Some("manual")` (never persisted —
/// manual canonicalizes to NULL at start) run the classic flow; only
/// `auto_advisory`/`auto_strict` trigger the in-process automation. The
/// scheduler is deliberately lenient so an out-of-band value never diverts the
/// checkpoint loop into the automation.
fn auto_checkpoint_policy(policy: Option<&str>) -> Option<&'static str> {
    match policy {
        Some("auto_advisory") => Some("auto_advisory"),
        Some("auto_strict") => Some("auto_strict"),
        _ => None,
    }
}

/// Mark the pending checkpoint as an auto-block (evolution §3.3): stamp its
/// `auto_outcome` (`blocked` or `manual`) and pending-manual scene ids on the
/// in-memory state (the caller persists), and return the outcome carrying the
/// prose-free `blocked_reason`. Never mutates checkpoint status — a blocked
/// auto-checkpoint stays `pending_review` exactly like a manual one, so the
/// operator's manual escape hatch still works.
fn auto_checkpoint_block(
    harness_state: &mut HarnessState,
    start_chapter: i32,
    end_chapter: i32,
    reason: &str,
    pending_manual_scene_ids: Vec<String>,
) -> AutoCheckpointOutcome {
    let outcome = if pending_manual_scene_ids.is_empty() {
        "blocked"
    } else {
        "manual"
    };
    if let Some(checkpoint) = harness_state
        .checkpoint_history
        .iter_mut()
        .find(|cp| cp.start_chapter == start_chapter && cp.end_chapter == end_chapter)
    {
        checkpoint.auto_outcome = Some(outcome.to_string());
        checkpoint.pending_manual_scene_ids = pending_manual_scene_ids.clone();
    }
    AutoCheckpointOutcome {
        approved: false,
        finding_counts: std::collections::BTreeMap::new(),
        blocked_reason: Some(reason.to_string()),
        pending_manual_scene_ids,
    }
}

/// The result of the in-process auto-checkpoint automation (evolution §3.3).
/// Ids/counts/enums only, never prose (I8).
struct AutoCheckpointOutcome {
    /// True iff the automation self-cleared the checkpoint under its policy.
    approved: bool,
    /// Deep-consistency severity counts (`{severity: n}`) at approval; empty on
    /// a block (the block reason carries the prose-free summary).
    finding_counts: std::collections::BTreeMap<String, i64>,
    /// Prose-free block summary when not approved; `None` on approval.
    blocked_reason: Option<String>,
    /// Scene ids awaiting manual dual-persona review (rating not covered — I3).
    pending_manual_scene_ids: Vec<String>,
}

/// Severity counts of a deep `check_consistency` output, keyed by the
/// lowercase severity word (`error` / `warning` / `info`). Uses the returned
/// `summary` (already computed by the service) so the verdict never re-walks
/// prose. Counts only — no prose.
fn auto_checkpoint_severity_counts(
    output: &spindle_core::models::CheckConsistencyOutput,
) -> std::collections::BTreeMap<String, i64> {
    let mut counts = std::collections::BTreeMap::new();
    counts.insert("error".to_string(), output.summary.error_count as i64);
    counts.insert("warning".to_string(), output.summary.warning_count as i64);
    counts.insert("info".to_string(), output.summary.info_count as i64);
    counts
}

/// Read the compact per-chapter reader-sim engagement summary from a checkpoint
/// report's `reader_sim` section (evolution §3.6, R3), for `authoring_status`.
/// Returns an empty vec when the report is missing/unreadable or carries no
/// reader-sim section (manual policy, or an older report) — a best-effort read
/// that never fails status. Enums/ids only, never prose (I8).
fn read_reader_sim_engagement(
    artifacts_root: &Path,
    report_rel: &str,
) -> Vec<spindle_core::models::ReaderSimEngagementSummary> {
    let report_path = artifacts_root.join(report_rel);
    let Ok(raw) = std::fs::read_to_string(&report_path) else {
        return Vec::new();
    };
    let Ok(report) = serde_json::from_str::<CheckpointReportArtifact>(&raw) else {
        return Vec::new();
    };
    report
        .reader_sim
        .map(|section| {
            section
                .chapters
                .into_iter()
                .map(|ch| spindle_core::models::ReaderSimEngagementSummary {
                    chapter: ch.chapter,
                    engagement: ch.engagement,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Is `error` a rating-not-covered rejection from the dispatch chokepoint
/// (evolution §4 rule 2)? The service surfaces the typed
/// `RouteClearanceError::RatingNotCovered` through anyhow, so downcast to detect
/// the explicit-manual-fallback case (I3) precisely — never string-matching an
/// error message. Names ids only; carries no prose.
fn auto_checkpoint_error_is_rating_not_covered(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<spindle_adapters::ai::RouteClearanceError>(),
        Some(spindle_adapters::ai::RouteClearanceError::RatingNotCovered { .. })
    )
}

/// Record a deep-consistency audit into the checkpoint report artifact — the
/// exact write `authoring_record_checkpoint_audit` performs, factored out so the
/// auto-checkpoint automation records the audit through the SAME persistence
/// shape (evolution §3.3 K3.1: "call the service/handler internals, not a new
/// persistence shape"). `deep_consistency` is the serialized deep output.
fn authoring_record_checkpoint_deep_audit(
    harness_state: &spindle_harness::state::HarnessState,
    state_path: &Path,
    start_chapter: i32,
    end_chapter: i32,
    deep_consistency: serde_json::Value,
) -> anyhow::Result<()> {
    let report_path =
        checkpoint_report_path(harness_state, state_path, start_chapter, end_chapter)?;
    let raw_report = std::fs::read_to_string(&report_path).with_context(|| {
        format!(
            "failed to read checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    let mut report: spindle_harness::artifacts::CheckpointReportArtifact =
        serde_json::from_str(&raw_report).with_context(|| {
            format!(
                "failed to parse checkpoint report artifact {}",
                report_path.display()
            )
        })?;
    report.deep_consistency = Some(deep_consistency);
    report.deep_consistency_status = "complete".to_string();
    report.deep_consistency_instruction =
        "Deep consistency audit recorded by the auto-checkpoint automation (evolution §3.3)."
            .to_string();
    let report_json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&report_path, report_json).with_context(|| {
        format!(
            "failed to update checkpoint report artifact {}",
            report_path.display()
        )
    })?;
    Ok(())
}

fn authoring_state_path(data_dir: &Path, run_id: &str) -> PathBuf {
    spindle_adapters::workspace::runtime_dir(data_dir).join(format!(
        "authoring_run_{}_temp.json",
        run_id.replace(":", "_")
    ))
}

fn checkpoint_report_path(
    state: &spindle_harness::state::HarnessState,
    state_path: &Path,
    start_chapter: i32,
    end_chapter: i32,
) -> anyhow::Result<PathBuf> {
    let checkpoint = state
        .checkpoint_history
        .iter()
        .find(|checkpoint| {
            checkpoint.start_chapter == start_chapter && checkpoint.end_chapter == end_chapter
        })
        .with_context(|| {
            format!(
                "checkpoint {}-{} not found in authoring run",
                start_chapter, end_chapter
            )
        })?;
    let report_artifact_path = checkpoint.report_artifact_path.clone().with_context(|| {
        format!(
            "checkpoint {}-{} has no report artifact path",
            start_chapter, end_chapter
        )
    })?;
    let artifacts_root = state_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&state.artifacts_dir);
    Ok(artifacts_root.join(report_artifact_path))
}

fn map_harness_to_records(
    run_id: &str,
    state: &spindle_harness::state::HarnessState,
    status: &str,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
) -> (
    spindle_adapters::sqlite::records::AuthoringRun,
    Vec<spindle_adapters::sqlite::records::AuthoringRunChapter>,
    Vec<spindle_adapters::sqlite::records::AuthoringRunScene>,
    Vec<spindle_adapters::sqlite::records::AuthoringCheckpoint>,
) {
    let now = chrono::Utc::now();
    let run = spindle_adapters::sqlite::records::AuthoringRun {
        id: run_id.to_string(),
        project_id: state.project_id.clone(),
        active_branch_id: state.active_branch_id.clone(),
        book_number: state.book_number,
        start_chapter: state.range.start_chapter,
        end_chapter: state.range.end_chapter,
        checkpoint_interval: state.checkpoint_interval as i64,
        last_checkpoint_end_chapter: state.last_checkpoint_end_chapter,
        artifacts_dir: state.artifacts_dir.clone(),
        editorial_directives: state.editorial_directives.clone(),
        status: status.to_string(),
        created_at: created_at.unwrap_or(now),
        updated_at: now,
        mining_policy: state.mining_policy.clone(),
        max_revise_attempts: state.max_revise_attempts,
        checkpoint_policy: state.checkpoint_policy.clone(),
        replan_policy: state.replan_policy.clone(),
    };

    let mut chapters = Vec::new();
    let mut scenes = Vec::new();
    for ch in &state.chapters {
        let ch_status = match ch.status {
            spindle_harness::state::ChapterStatus::Pending => "pending",
            spindle_harness::state::ChapterStatus::InProgress => "in_progress",
            spindle_harness::state::ChapterStatus::Complete => "complete",
        };
        chapters.push(spindle_adapters::sqlite::records::AuthoringRunChapter {
            authoring_run_id: run_id.to_string(),
            chapter_number: ch.chapter_number,
            planned: ch.planned,
            synopsis: ch.synopsis.clone(),
            pov_character_id: ch.pov_character_id.clone(),
            status: ch_status.to_string(),
            summary_saved: ch.summary_saved,
            summary_artifact_path: ch.summary_artifact_path.clone(),
            replan_status: ch.replan_status.clone(),
            replan_detail: ch.replan_detail.clone(),
        });

        for sc in &ch.scenes {
            let sc_phase = match sc.phase {
                spindle_harness::state::ScenePhase::Pending => "pending",
                spindle_harness::state::ScenePhase::DraftSaved => "draft_saved",
                spindle_harness::state::ScenePhase::ChangesCommitted => "changes_committed",
                spindle_harness::state::ScenePhase::BeatsAnnotated => "beats_annotated",
            };
            scenes.push(spindle_adapters::sqlite::records::AuthoringRunScene {
                authoring_run_id: run_id.to_string(),
                chapter_number: ch.chapter_number,
                scene_order: sc.scene_order,
                character_ids: sc.character_ids.clone(),
                location_id: sc.location_id.clone(),
                content_rating: sc.content_rating.as_str().to_string(),
                tone: sc.tone.clone(),
                source_path: sc.source_path.clone(),
                phase: sc_phase.to_string(),
                scene_id: sc.scene_id.clone(),
                scene_artifact_path: sc.scene_artifact_path.clone(),
                draft_diagnostics: sc
                    .draft_diagnostics
                    .as_ref()
                    .map(|d| serde_json::to_value(d).unwrap()),
                blocked_reason: sc.blocked_reason.clone(),
                research_required: sc.research_required,
                research_tags: sc.research_tags.clone(),
                explicit_query: sc.explicit_query.clone(),
                mine_status: sc.mine_status.clone(),
                mine_detail: sc.mine_detail.clone(),
                verify_status: sc.verify_status.clone(),
                verify_detail: sc.verify_detail.clone(),
                revise_attempts: sc.revise_attempts,
                // The revision-directives block is transient harness state only;
                // it is never persisted to the run tables (evolution §3.2).
                last_finding_fingerprint: sc.last_finding_fingerprint.clone(),
            });
        }
    }

    let mut checkpoints = Vec::new();
    for cp in &state.checkpoint_history {
        let cp_status = match cp.status {
            spindle_harness::state::CheckpointStatus::PendingReview => "pending_review",
            spindle_harness::state::CheckpointStatus::Reviewed => "reviewed",
        };
        checkpoints.push(spindle_adapters::sqlite::records::AuthoringCheckpoint {
            authoring_run_id: run_id.to_string(),
            start_chapter: cp.start_chapter,
            end_chapter: cp.end_chapter,
            save_point_id: cp.save_point_id.clone(),
            status: cp_status.to_string(),
            report_artifact_path: cp.report_artifact_path.clone(),
            auto_outcome: cp.auto_outcome.clone(),
            pending_manual_scene_ids: cp.pending_manual_scene_ids.clone(),
        });
    }

    (run, chapters, scenes, checkpoints)
}

fn map_records_to_harness(
    run: &spindle_adapters::sqlite::records::AuthoringRun,
    chapters: &[spindle_adapters::sqlite::records::AuthoringRunChapter],
    scenes: &[spindle_adapters::sqlite::records::AuthoringRunScene],
    checkpoints: &[spindle_adapters::sqlite::records::AuthoringCheckpoint],
) -> spindle_harness::state::HarnessState {
    let mut ch_states = Vec::new();
    for ch in chapters {
        let mut ch_scenes = Vec::new();
        for sc in scenes {
            if sc.chapter_number == ch.chapter_number {
                let phase = match sc.phase.as_str() {
                    "pending" => spindle_harness::state::ScenePhase::Pending,
                    "draft_saved" => spindle_harness::state::ScenePhase::DraftSaved,
                    "changes_committed" => spindle_harness::state::ScenePhase::ChangesCommitted,
                    "beats_annotated" => spindle_harness::state::ScenePhase::BeatsAnnotated,
                    _ => spindle_harness::state::ScenePhase::Pending,
                };
                let content_rating = match sc.content_rating.to_ascii_lowercase().as_str() {
                    "general" => spindle_core::models::ContentRating::General,
                    "teen" => spindle_core::models::ContentRating::Teen,
                    "mature" => spindle_core::models::ContentRating::Mature,
                    "explicit" => spindle_core::models::ContentRating::Explicit,
                    _ => spindle_core::models::ContentRating::General,
                };
                let draft_diagnostics = sc
                    .draft_diagnostics
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                ch_scenes.push(spindle_harness::state::SceneState {
                    scene_order: sc.scene_order,
                    character_ids: sc.character_ids.clone(),
                    location_id: sc.location_id.clone(),
                    content_rating,
                    tone: sc.tone.clone(),
                    source_path: sc.source_path.clone(),
                    phase,
                    scene_id: sc.scene_id.clone(),
                    scene_artifact_path: sc.scene_artifact_path.clone(),
                    draft_diagnostics,
                    blocked_reason: sc.blocked_reason.clone(),
                    research_required: sc.research_required,
                    research_tags: sc.research_tags.clone(),
                    explicit_query: sc.explicit_query.clone(),
                    mine_status: sc.mine_status.clone(),
                    mine_detail: sc.mine_detail.clone(),
                    verify_status: sc.verify_status.clone(),
                    verify_detail: sc.verify_detail.clone(),
                    revise_attempts: sc.revise_attempts,
                    last_finding_fingerprint: sc.last_finding_fingerprint.clone(),
                    ..Default::default()
                });
            }
        }
        let status = match ch.status.as_str() {
            "pending" => spindle_harness::state::ChapterStatus::Pending,
            "in_progress" => spindle_harness::state::ChapterStatus::InProgress,
            "complete" => spindle_harness::state::ChapterStatus::Complete,
            _ => spindle_harness::state::ChapterStatus::Pending,
        };
        ch_states.push(spindle_harness::state::ChapterState {
            chapter_number: ch.chapter_number,
            planned: ch.planned,
            synopsis: ch.synopsis.clone(),
            pov_character_id: ch.pov_character_id.clone(),
            status,
            scenes: ch_scenes,
            summary_saved: ch.summary_saved,
            summary_artifact_path: ch.summary_artifact_path.clone(),
            replan_status: ch.replan_status.clone(),
            replan_detail: ch.replan_detail.clone(),
        });
    }

    let mut cp_history = Vec::new();
    for cp in checkpoints {
        let status = match cp.status.as_str() {
            "pending_review" => spindle_harness::state::CheckpointStatus::PendingReview,
            "reviewed" => spindle_harness::state::CheckpointStatus::Reviewed,
            _ => spindle_harness::state::CheckpointStatus::PendingReview,
        };
        cp_history.push(spindle_harness::state::CheckpointRecord {
            start_chapter: cp.start_chapter,
            end_chapter: cp.end_chapter,
            save_point_id: cp.save_point_id.clone(),
            status,
            report_artifact_path: cp.report_artifact_path.clone(),
            auto_outcome: cp.auto_outcome.clone(),
            pending_manual_scene_ids: cp.pending_manual_scene_ids.clone(),
        });
    }

    let mut state = spindle_harness::state::HarnessState {
        project_id: run.project_id.clone(),
        active_branch_id: run.active_branch_id.clone(),
        book_number: run.book_number,
        range: spindle_harness::state::ChapterRange {
            start_chapter: run.start_chapter,
            end_chapter: run.end_chapter,
        },
        checkpoint_interval: run.checkpoint_interval as usize,
        last_checkpoint_end_chapter: run.last_checkpoint_end_chapter,
        artifacts_dir: run.artifacts_dir.clone(),
        editorial_directives: run.editorial_directives.clone(),
        mining_policy: run.mining_policy.clone(),
        max_revise_attempts: run.max_revise_attempts,
        checkpoint_policy: run.checkpoint_policy.clone(),
        replan_policy: run.replan_policy.clone(),
        chapters: ch_states,
        checkpoint_history: cp_history,
    };

    if state.artifacts_dir == "artifacts" {
        state.artifacts_dir = "../artifacts".to_string();
    }
    state
}

/// Public entry point for both the MCP tool and the CLI.
/// Writes the Grok skill adapter(s) for Spindle into the target directory
/// (or into ~/.grok/skills/ when `global` is true).
pub fn run_init_grok_skills(
    target_dir: Option<String>,
    global: bool,
) -> anyhow::Result<InitGrokSkillsOutput> {
    use std::fs;
    use std::path::PathBuf;

    let base_skills_dir: PathBuf = if global {
        let home = dirs::home_dir()
            .ok_or_else(|| anyhow::anyhow!("could not determine home directory"))?;
        home.join(".grok").join("skills")
    } else {
        let target = match target_dir {
            Some(p) if !p.trim().is_empty() => PathBuf::from(p),
            _ => std::env::current_dir().map_err(|e| anyhow::anyhow!("could not get cwd: {e}"))?,
        };
        target.join(".grok").join("skills")
    };

    fs::create_dir_all(&base_skills_dir)
        .map_err(|e| anyhow::anyhow!("failed to create skills directory: {e}"))?;

    let mut files_written = Vec::new();

    // 1. Write the main meta skill
    let meta_dir = base_skills_dir.join("spindle");
    fs::create_dir_all(&meta_dir)?;
    let meta_path = meta_dir.join("SKILL.md");
    fs::write(&meta_path, SPINDLE_GROK_SKILL_MD)?;
    files_written.push(meta_path.display().to_string());

    // 2. Write thin adapters for every individual Spindle skill
    let spindle_skills = [
        "scene-writer",
        "character-creator",
        "worldbuilder",
        "revision-manager",
        "continuity-editor",
        "editor",
        "manuscript-importer",
        "bible-librarian",
        "plot-architect",
    ];

    for skill in spindle_skills {
        let adapter_name = format!("spindle-{}", skill);
        let adapter_dir = base_skills_dir.join(&adapter_name);
        fs::create_dir_all(&adapter_dir)?;

        let content = generate_spindle_skill_adapter(skill);
        let path = adapter_dir.join("SKILL.md");
        fs::write(&path, content)?;
        files_written.push(path.display().to_string());
    }

    let location = if global {
        "~/.grok/skills/ (global)"
    } else {
        "repo-scoped .grok/skills/"
    };

    Ok(InitGrokSkillsOutput {
        target_dir: if global {
            "~/.grok/skills".to_string()
        } else {
            // best effort
            std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_default()
        },
        files_written,
        message: format!(
            "Grok Spindle skills initialized ({location}). You now have the 'spindle' meta skill plus individual adapters (spindle-scene-writer, spindle-character-creator, etc.). They will be available globally in Grok."
        ),
    })
}

/// Generates a thin Grok adapter for a specific Spindle bible skill.
fn generate_spindle_skill_adapter(skill_name: &str) -> String {
    let title = skill_name.replace('-', " ");
    let title = title
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        r#"---
name: spindle-{skill_name}
description: Use when doing {title} work inside Spindle book projects. This is the Grok adapter for the official Spindle {skill_name} skill.
---

# Spindle {title} (Grok Adapter)

This is a thin Grok-specific wrapper around Spindle's canonical skill.

**Authoritative instructions (always read these):** `bible://skills/{skill_name}`

## When to activate
- The user asks to write, plan, revise, or manage anything related to **{title}** in a Spindle project.

## Grok-specific notes
- Spindle already provides strong guardrails via the Bible, continuity, and voice profiles.
- Prefer natural, readable, webnovel-style prose.
- For explicit content, always route through the proper `continue_generation` + `rating: "explicit"` path.
- Re-anchor with `get_writer_state` + the relevant briefing tools before major work.

Use the live `bible://skills/{skill_name}` resource for the detailed step-by-step procedure.
"#
    )
}

fn tool_supports_session_branch_default(name: &str) -> bool {
    matches!(
        name,
        "get_writer_state"
            | "get_entity"
            | "find_entity"
            | "get_character_snapshot"
            | "set_character_voice_profile"
            | "batch_set_character_voice_profiles"
            | "run_dual_persona_review"
            | "record_knowledge"
            | "record_note"
            | "update_writer_position"
    )
}

fn relax_session_default_fields(tool_name: &str, value: &mut Value) {
    let Some(schema_obj) = value.as_object_mut() else {
        return;
    };
    let Some(required) = schema_obj.get_mut("required").and_then(Value::as_array_mut) else {
        return;
    };

    required.retain(|field| match field.as_str() {
        Some("project_id") if tool_supports_session_project_default(tool_name) => false,
        Some("branch_id") if tool_supports_session_branch_default(tool_name) => false,
        _ => true,
    });

    if required.is_empty() {
        schema_obj.remove("required");
    }
}

fn tool<I, O>(name: &'static str, description: &'static str) -> Tool
where
    I: schemars::JsonSchema + 'static,
    O: schemars::JsonSchema + 'static,
{
    let settings = SchemaSettings::openapi3().with(|s| {
        s.inline_subschemas = true;
    });
    let generator = settings.into_generator();
    let schema = generator.into_root_schema_for::<I>();
    let mut value = serde_json::to_value(&schema).expect("schema to json");
    relax_session_default_fields(name, &mut value);
    sanitize_for_gemini(&mut value);
    // Scrub schemars' non-standard numeric `format` annotations so strict
    // JSON-Schema clients do not warn per count-typed field. This is the single
    // finalization chokepoint: every tool served over stdio and HTTP flows
    // through here for both its input and output schema.
    strip_nonstandard_formats(&mut value);
    strip_null_enum_values(&mut value);
    let object = value
        .as_object()
        .cloned()
        .unwrap_or_else(|| panic!("expected object schema for tool input"));

    let tool = Tool::new(name, description, object).with_output_schema::<O>();
    scrub_output_schema_formats(tool)
}

/// Apply [`strip_nonstandard_formats`] to a tool's already-generated output
/// schema (rmcp builds it via its own draft-2020-12 generator, so it never
/// passes through the input-schema scrub above). Returns the tool unchanged
/// when it has no output schema.
fn scrub_output_schema_formats(tool: Tool) -> Tool {
    let Some(output) = tool.output_schema.as_ref() else {
        return tool;
    };
    let mut value = Value::Object((**output).clone());
    strip_nonstandard_formats(&mut value);
    strip_null_enum_values(&mut value);
    let object = value
        .as_object()
        .cloned()
        .unwrap_or_else(|| panic!("expected object schema for tool output"));
    tool.with_raw_output_schema(std::sync::Arc::new(object))
}

/// Recursively strip schemars' non-standard numeric `format` annotations from a
/// JSON Schema value so strict JSON-Schema clients (Kimi Code and any
/// spec-compliant validator) do not warn on every count-typed field.
///
/// Denylist rule (value-based, not key-position-based): remove any object's
/// `"format"` key whose value is a schemars numeric format —
/// `float`, `double`, or `^u?int\d*$` (`int`, `uint`, `int8`..`int64`,
/// `uint8`..`uint64`). Standard formats (`date-time`, `uuid`, `uri`, `email`,
/// `regex`, …) and any other future format are preserved. A JSON Schema
/// document cannot contain a data object with a coincidental numeric-format
/// `"format"` value, so matching on the value is safe. Structure is otherwise
/// untouched — types, properties, `$defs`, `required`, descriptions stay
/// byte-identical.
fn strip_nonstandard_formats(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if map
                .get("format")
                .and_then(Value::as_str)
                .is_some_and(is_nonstandard_numeric_format)
            {
                map.remove("format");
            }
            for child in map.values_mut() {
                strip_nonstandard_formats(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_nonstandard_formats(item);
            }
        }
        _ => {}
    }
}

/// Remove `null` entries from `"enum"` arrays anywhere in a schema tree.
///
/// schemars emits `"enum": [..., null]` for `Option<Enum>` fields. MCP
/// consumers vary: Anthropic tolerates the null entry; Moonshot's validator
/// rejects it (observed live: `enum value (<nil>) does not match any type in
/// [string]` at `properties.canonical_facts.items.properties.scope.enum`).
/// Optionality is already expressed by the field's absence from `required`,
/// and serde accepts explicit nulls for `Option` fields regardless of the
/// advertised schema, so dropping the null tightens the advertisement without
/// changing server behavior. An enum emptied by the removal is dropped.
fn strip_null_enum_values(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let emptied = if let Some(Value::Array(items)) = map.get_mut("enum") {
                items.retain(|v| !v.is_null());
                items.is_empty()
            } else {
                false
            };
            if emptied {
                map.remove("enum");
            }
            for child in map.values_mut() {
                strip_null_enum_values(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_null_enum_values(item);
            }
        }
        _ => {}
    }
}

/// Returns true for schemars' non-standard numeric `format` values: `float`,
/// `double`, or `int`/`uint` optionally followed by only digits (`int8`,
/// `uint16`, `int32`, `uint64`, …). Matches the regex `^(u?int\d*|float|double)$`.
fn is_nonstandard_numeric_format(s: &str) -> bool {
    if s == "float" || s == "double" {
        return true;
    }
    let rest = match s.strip_prefix("uint").or_else(|| s.strip_prefix("int")) {
        Some(rest) => rest,
        None => return false,
    };
    rest.chars().all(|c| c.is_ascii_digit())
}

/// Post-process a JSON Schema value to be compatible with Gemini's strict subset.
/// Removes `$defs`, resolves remaining `anyOf`/`oneOf` nullable patterns,
/// strips unsupported keywords, and converts array `type` to single string.
fn sanitize_for_gemini(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Remove unsupported top-level keywords
            map.remove("$schema");
            map.remove("$defs");
            map.remove("definitions");
            map.remove("$ref");
            map.remove("format");
            map.remove("default");
            map.remove("const");
            map.remove("nullable");
            map.remove("examples");
            map.remove("$id");

            // Convert type arrays like ["string", "null"] to just "string"
            if let Some(ty) = map.get_mut("type")
                && let Some(arr) = ty.as_array().cloned()
            {
                let non_null: Vec<_> = arr
                    .into_iter()
                    .filter(|v| v.as_str() != Some("null"))
                    .collect();
                if non_null.len() == 1 {
                    *ty = non_null.into_iter().next().unwrap();
                }
            }

            // Flatten anyOf where one branch is null (nullable pattern)
            if let Some(any_of) = map.remove("anyOf")
                && let Some(branches) = any_of.as_array()
            {
                let non_null: Vec<_> = branches
                    .iter()
                    .filter(|b| {
                        b.get("type").and_then(Value::as_str) != Some("null")
                            && b.get("const") != Some(&Value::Null)
                            && !b.as_object().is_some_and(|o| o.is_empty())
                    })
                    .collect();
                if non_null.len() == 1 {
                    // Merge the single non-null branch into this schema
                    if let Some(obj) = non_null[0].as_object() {
                        for (k, v) in obj {
                            map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                } else if !non_null.is_empty() {
                    // Multiple real branches — keep as anyOf (best effort)
                    map.insert(
                        "anyOf".to_string(),
                        Value::Array(non_null.into_iter().cloned().collect()),
                    );
                }
            }

            // Flatten oneOf similarly
            if let Some(one_of) = map.remove("oneOf")
                && let Some(branches) = one_of.as_array()
            {
                let non_null: Vec<_> = branches
                    .iter()
                    .filter(|b| {
                        b.get("type").and_then(Value::as_str) != Some("null")
                            && b.get("const") != Some(&Value::Null)
                    })
                    .collect();
                if non_null.len() == 1 {
                    if let Some(obj) = non_null[0].as_object() {
                        for (k, v) in obj {
                            map.entry(k.clone()).or_insert_with(|| v.clone());
                        }
                    }
                } else if !non_null.is_empty() {
                    // Convert tagged enum oneOf to a plain object schema
                    // (Gemini can't handle oneOf, so we just accept any object)
                    map.insert("type".to_string(), Value::String("object".to_string()));
                }
            }

            // Ensure object schemas with properties have a type
            if map.contains_key("properties") && !map.contains_key("type") {
                map.insert("type".to_string(), Value::String("object".to_string()));
            }

            // Recurse into all remaining values
            for v in map.values_mut() {
                sanitize_for_gemini(v);
            }
        }
        Value::Array(arr) => {
            for v in arr {
                sanitize_for_gemini(v);
            }
        }
        _ => {}
    }
}

fn parse_arguments<T>(arguments: Option<&rmcp::model::JsonObject>) -> anyhow::Result<T>
where
    T: DeserializeOwned + schemars::JsonSchema,
{
    let mut value = match arguments {
        Some(args) => Value::Object(args.clone()),
        None => Value::Object(Default::default()),
    };
    let schema = SchemaSettings::openapi3()
        .with(|s| {
            s.meta_schema = None;
        })
        .into_generator()
        .into_root_schema_for::<T>();
    coerce_value_for_schema(&mut value, schema.as_value(), schema.as_value());
    Ok(serde_json::from_value(value)?)
}

fn coerce_value_for_schema(value: &mut Value, schema: &Value, root_schema: &Value) {
    let schema = resolve_schema_refs(schema, root_schema);
    let Some(schema_obj) = schema.as_object() else {
        return;
    };

    if let Some(all_of) = schema_obj.get("allOf").and_then(Value::as_array) {
        for subschema in all_of {
            coerce_value_for_schema(value, subschema, root_schema);
        }
    }

    if matches!(value, Value::String(_))
        && let Some(coerced) = coerce_string_value_for_schema(value, schema_obj, root_schema)
    {
        *value = coerced;
    }

    if value.is_object() {
        for keyword in ["anyOf", "oneOf"] {
            if let Some(candidates) = schema_obj.get(keyword).and_then(Value::as_array) {
                for candidate in candidates {
                    coerce_value_for_schema(value, candidate, root_schema);
                }
            }
        }
    }

    match value {
        Value::Object(map) => {
            let properties = schema_obj.get("properties").and_then(Value::as_object);
            let additional = schema_obj.get("additionalProperties");
            for (key, child) in map {
                if let Some(schema) = properties.and_then(|properties| properties.get(key)) {
                    coerce_value_for_schema(child, schema, root_schema);
                } else if let Some(schema) = additional {
                    coerce_value_for_schema(child, schema, root_schema);
                }
            }
        }
        Value::Array(items) => {
            if let Some(item_schema) = schema_obj.get("items") {
                for item in items {
                    coerce_value_for_schema(item, item_schema, root_schema);
                }
            }
        }
        _ => {}
    }
}

fn coerce_string_value_for_schema(
    value: &Value,
    schema_obj: &serde_json::Map<String, Value>,
    root_schema: &Value,
) -> Option<Value> {
    let Value::String(raw) = value else {
        return None;
    };

    if schema_allows_type(schema_obj, "string") {
        return None;
    }

    for keyword in ["anyOf", "oneOf"] {
        if let Some(candidates) = schema_obj.get(keyword).and_then(Value::as_array) {
            if candidates.iter().any(|candidate| {
                resolve_schema_refs(candidate, root_schema)
                    .as_object()
                    .is_some_and(|candidate| schema_allows_type(candidate, "string"))
            }) {
                return None;
            }
            for candidate in candidates {
                let candidate = resolve_schema_refs(candidate, root_schema);
                if let Some(candidate_obj) = candidate.as_object()
                    && let Some(coerced) =
                        coerce_string_value_for_schema(value, candidate_obj, root_schema)
                {
                    return Some(coerced);
                }
            }
        }
    }

    if schema_obj
        .get("nullable")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        && raw.eq_ignore_ascii_case("null")
    {
        return Some(Value::Null);
    }

    if schema_allows_type(schema_obj, "integer") {
        if let Ok(parsed) = raw.parse::<i64>() {
            return Some(Value::Number(Number::from(parsed)));
        }
        if let Ok(parsed) = raw.parse::<u64>() {
            return Some(Value::Number(Number::from(parsed)));
        }
    }

    if schema_allows_type(schema_obj, "number")
        && let Ok(parsed) = raw.parse::<f64>()
        && let Some(number) = Number::from_f64(parsed)
    {
        return Some(Value::Number(number));
    }

    if schema_allows_type(schema_obj, "boolean") {
        match raw.as_str() {
            "true" => return Some(Value::Bool(true)),
            "false" => return Some(Value::Bool(false)),
            _ => {}
        }
    }

    if schema_allows_type(schema_obj, "null") && raw.eq_ignore_ascii_case("null") {
        return Some(Value::Null);
    }

    if (schema_allows_type(schema_obj, "array") || schema_allows_type(schema_obj, "object"))
        && let Ok(parsed) = serde_json::from_str::<Value>(raw)
        && ((parsed.is_array() && schema_allows_type(schema_obj, "array"))
            || (parsed.is_object() && schema_allows_type(schema_obj, "object")))
    {
        return Some(parsed);
    }

    None
}

fn schema_allows_type(schema_obj: &serde_json::Map<String, Value>, target: &str) -> bool {
    schema_obj.get("type").is_some_and(|value| match value {
        Value::String(kind) => kind == target,
        Value::Array(kinds) => kinds.iter().any(|kind| kind.as_str() == Some(target)),
        _ => false,
    })
}

fn resolve_schema_refs<'a>(schema: &'a Value, root_schema: &'a Value) -> &'a Value {
    let mut current = schema;
    for _ in 0..8 {
        let Some(reference) = current
            .as_object()
            .and_then(|schema| schema.get("$ref"))
            .and_then(Value::as_str)
        else {
            break;
        };

        let Some(pointer) = reference.strip_prefix('#') else {
            break;
        };
        let Some(target) = root_schema.pointer(pointer) else {
            break;
        };
        current = target;
    }
    current
}

fn structured_result<T>(value: &T) -> anyhow::Result<CallToolResult>
where
    T: Serialize,
{
    let mut structured = serde_json::to_value(value)?;
    flatten_record_ids(&mut structured);
    Ok(CallToolResult::structured(structured))
}

fn structured_error_result(error: &anyhow::Error) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("Error: {error:#}"))])
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use tokio::time::{Duration, timeout};

    use spindle_adapters::ModelRouter;
    use spindle_adapters::SqlitePool;
    use spindle_adapters::sqlite::Repository as SpindleRepository;

    use super::*;

    fn structured_json(result: CallToolResult) -> Value {
        assert_eq!(result.is_error, Some(false));
        result.structured_content.expect("structured content")
    }

    /// A knowledge_learned reveal on its own SATISFIES the mandatory continuity
    /// package (design §2.3 path 2): a scene whose only durable change is an
    /// on-page reveal must not be rejected for an "empty" package.
    #[test]
    fn knowledge_learned_counts_toward_package_satisfaction() {
        use spindle_core::models::{
            AuthoringSaveSceneDraftInput, ContentRating, KnowledgeLearnedEntry,
        };
        let base = AuthoringSaveSceneDraftInput {
            project_id: "project:x".into(),
            run_id: None,
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "prose".into(),
            authorship: Default::default(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            location_id: None,
            research_source_ids: Vec::new(),
            research_note_ids: Vec::new(),
            research_claim_ids: Vec::new(),
            research_query_pack_input: None,
            research_context_hash: None,
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: Vec::new(),
            beats: Vec::new(),
            continuity_notes: Vec::new(),
            knowledge_learned: Vec::new(),
        };
        assert_eq!(
            super::authoring_structured_update_count(&base),
            0,
            "an empty package is empty"
        );
        let with_reveal = AuthoringSaveSceneDraftInput {
            knowledge_learned: vec![KnowledgeLearnedEntry {
                character_id: "character:bran".into(),
                fact: "Mara is reincarnated.".into(),
                source_summary: None,
                secret_of_fact_id: Some("canonical_fact:reinc".into()),
                reader_visible: Some(true),
            }],
            ..base
        };
        assert_eq!(
            super::authoring_structured_update_count(&with_reveal),
            1,
            "a knowledge_learned reveal satisfies the mandatory package"
        );
    }

    async fn router() -> ToolRouter {
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("router.db"))
            .await
            .expect("db init");
        let data_dir = temp.keep();
        ToolRouter::with_tool_profile_and_serialization(
            SpindleService::new(SpindleRepository::with_model_router(
                db,
                data_dir,
                ModelRouter::local_only(),
            )),
            None,
            Arc::new(ToolSerializationState::default()),
        )
    }

    /// A fresh repository + a persisted `active` authoring run, for driving the
    /// journal emitter directly (deterministic step-event trace tests).
    async fn repo_with_run() -> (tempfile::TempDir, SpindleRepository, String) {
        use spindle_core::models::{CreateProjectInput, ReaderContract};
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("trace.db"))
            .await
            .expect("db init");
        let data_dir = temp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let repo = SpindleRepository::with_model_router(db, data_dir, ModelRouter::local_only());
        let (project, branch, _book, _chapter) = repo
            .create_project(&CreateProjectInput {
                name: "Trace".into(),
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
        let run_id = format!(
            "authoring_run:{}",
            ulid::Ulid::new().to_string().to_lowercase()
        );
        let now = chrono::Utc::now();
        repo.save_authoring_run(
            spindle_adapters::sqlite::records::AuthoringRun {
                id: run_id.clone(),
                project_id: project.id.clone(),
                active_branch_id: branch.id,
                book_number: 1,
                start_chapter: 1,
                end_chapter: 1,
                checkpoint_interval: 1,
                last_checkpoint_end_chapter: 0,
                artifacts_dir: "../artifacts".into(),
                editorial_directives: Vec::new(),
                status: "active".into(),
                created_at: now,
                updated_at: now,
                mining_policy: Some("propose_all".into()),
                max_revise_attempts: Some(1),
                checkpoint_policy: None,
                replan_policy: None,
            },
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .await
        .unwrap();
        (temp, repo, run_id)
    }

    /// Build a single-scene [`HarnessState`] snapshot with the given per-scene
    /// status fields, for driving `authoring_emit_step_events`.
    fn trace_state(
        scene_id: &str,
        phase: ScenePhase,
        verify_status: Option<&str>,
        verify_detail: Option<&str>,
        mine_status: Option<&str>,
        mine_detail: Option<&str>,
        revise_attempts: i32,
    ) -> HarnessState {
        use spindle_harness::state::{ChapterState, ChapterStatus, SceneState};
        let scene = SceneState {
            scene_order: 1,
            location_id: "location:x".into(),
            phase,
            scene_id: Some(scene_id.into()),
            verify_status: verify_status.map(str::to_string),
            verify_detail: verify_detail.map(str::to_string),
            mine_status: mine_status.map(str::to_string),
            mine_detail: mine_detail.map(str::to_string),
            revise_attempts,
            ..SceneState::default()
        };
        let mut state = HarnessState::from_seed(
            spindle_harness::state::HarnessSeed {
                project_id: "project:x".into(),
                book_number: 1,
                range: spindle_harness::state::ChapterRange {
                    start_chapter: 1,
                    end_chapter: 1,
                },
                checkpoint_interval: 1,
                editorial_directives: Vec::new(),
                chapters: Vec::new(),
            },
            "bible_branch:x".into(),
        );
        state.chapters = vec![ChapterState {
            chapter_number: 1,
            planned: true,
            synopsis: "s".into(),
            pov_character_id: None,
            status: ChapterStatus::InProgress,
            scenes: vec![scene],
            summary_saved: false,
            summary_artifact_path: None,
            replan_status: None,
            replan_detail: None,
        }];
        state
    }

    /// Deterministic full per-scene step-event trace (J3 test 3, revise arm
    /// included). Drives `authoring_emit_step_events` for the exact NextAction
    /// sequence a verify+revise+mine scene walks, then asserts the emitted kind
    /// order — including `scene_verify_completed(findings)` → `scene_revised` →
    /// `scene_verify_completed(clean)` — and that no step event double-emits.
    #[tokio::test(flavor = "current_thread")]
    async fn step_event_trace_covers_verify_revise_mine_sequence() {
        let (_tmp, repo, run_id) = repo_with_run().await;
        let scene_id = "scene:trace1";

        // draft (agent)
        authoring_emit_step_events(
            &repo,
            &run_id,
            &NextAction::DraftScene {
                chapter_number: 1,
                scene_order: 1,
            },
            &trace_state(scene_id, ScenePhase::DraftSaved, None, None, None, None, 0),
            "active",
            "active",
        )
        .await;
        // verify → findings
        authoring_emit_step_events(
            &repo,
            &run_id,
            &NextAction::VerifyScene {
                chapter_number: 1,
                scene_order: 1,
            },
            &trace_state(
                scene_id,
                ScenePhase::DraftSaved,
                Some("findings"),
                Some("2 finding(s) at or above warning"),
                None,
                None,
                0,
            ),
            "active",
            "active",
        )
        .await;
        // revise
        authoring_emit_step_events(
            &repo,
            &run_id,
            &NextAction::ReviseScene {
                chapter_number: 1,
                scene_order: 1,
                attempt: 1,
            },
            &trace_state(scene_id, ScenePhase::DraftSaved, None, None, None, None, 1),
            "active",
            "active",
        )
        .await;
        // verify → clean
        authoring_emit_step_events(
            &repo,
            &run_id,
            &NextAction::VerifyScene {
                chapter_number: 1,
                scene_order: 1,
            },
            &trace_state(
                scene_id,
                ScenePhase::DraftSaved,
                Some("clean"),
                Some("0 finding(s) at or above warning"),
                None,
                None,
                1,
            ),
            "active",
            "active",
        )
        .await;
        // commit
        authoring_emit_step_events(
            &repo,
            &run_id,
            &NextAction::CommitSceneChanges {
                chapter_number: 1,
                scene_order: 1,
                scene_id: scene_id.into(),
            },
            &trace_state(
                scene_id,
                ScenePhase::ChangesCommitted,
                Some("clean"),
                None,
                None,
                None,
                1,
            ),
            "active",
            "active",
        )
        .await;
        // mine → staged
        authoring_emit_step_events(
            &repo,
            &run_id,
            &NextAction::MineScene {
                chapter_number: 1,
                scene_order: 1,
            },
            &trace_state(
                scene_id,
                ScenePhase::ChangesCommitted,
                Some("clean"),
                None,
                Some("staged"),
                Some("staged 1 delta(s)"),
                1,
            ),
            "active",
            "active",
        )
        .await;
        // annotate
        authoring_emit_step_events(
            &repo,
            &run_id,
            &NextAction::AnnotateSceneBeats {
                chapter_number: 1,
                scene_order: 1,
                scene_id: scene_id.into(),
            },
            &trace_state(
                scene_id,
                ScenePhase::BeatsAnnotated,
                Some("clean"),
                None,
                Some("staged"),
                None,
                1,
            ),
            "active",
            "active",
        )
        .await;

        let events = repo.list_run_events(&run_id, None, None).await.unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "scene_drafted",
                "scene_verify_completed",
                "scene_revised",
                "scene_verify_completed",
                "scene_committed",
                "scene_mined",
                "beats_annotated",
            ],
            "full per-scene step trace, revise arm included, no duplicates"
        );

        // Verdict discipline: first verify is findings, second is clean.
        let verifies: Vec<&serde_json::Value> = events
            .iter()
            .filter(|e| e.kind == "scene_verify_completed")
            .map(|e| &e.payload)
            .collect();
        assert_eq!(verifies[0]["verdict"], serde_json::json!("findings"));
        assert_eq!(
            verifies[0]["finding_counts"]["actionable"],
            serde_json::json!(2)
        );
        assert_eq!(verifies[1]["verdict"], serde_json::json!("clean"));

        // scene_revised carries the attempt; scene_mined the staged count.
        let revised = events.iter().find(|e| e.kind == "scene_revised").unwrap();
        assert_eq!(revised.payload["attempt"], serde_json::json!(1));
        let mined = events.iter().find(|e| e.kind == "scene_mined").unwrap();
        assert_eq!(mined.payload["mine_status"], serde_json::json!("staged"));
        assert_eq!(mined.payload["staged_count"], serde_json::json!(1));

        // Dense seqs 1..=N (resume-token integrity).
        let seqs: Vec<i64> = events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, (1..=events.len() as i64).collect::<Vec<_>>());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn invalid_tool_input_returns_structured_tool_error() {
        let router = router().await;

        let result = router
            .call_tool("create_project", Some(&serde_json::Map::new()))
            .await
            .expect("tool call should return result");

        assert_eq!(result.is_error, Some(true));
        assert!(
            result.structured_content.is_none(),
            "error should not use structured content"
        );
        let text = result.content.first().expect("error content");
        let text = format!("{text:?}");
        assert!(
            text.contains("missing field"),
            "expected 'missing field' in: {text}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn unknown_tool_returns_structured_tool_error() {
        let router = router().await;

        let result = router
            .call_tool("not_a_real_tool", None)
            .await
            .expect("tool call should return result");

        assert_eq!(result.is_error, Some(true));
        assert!(
            result.structured_content.is_none(),
            "error should not use structured content"
        );
        let text = result.content.first().expect("error content");
        let text = format!("{text:?}");
        assert!(
            text.contains("unknown tool: not_a_real_tool"),
            "expected error text in: {text}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_skills_returns_embedded_skills_and_references() {
        let router = router().await;

        let result = router
            .call_tool("list_skills", None)
            .await
            .expect("tool call should return result");

        assert_eq!(result.is_error, Some(false));
        let content = result
            .structured_content
            .expect("list_skills should use structured content");
        let skill_names: Vec<&str> = content["skills"]
            .as_array()
            .expect("skills array")
            .iter()
            .filter_map(|skill| skill["name"].as_str())
            .collect();
        for expected in [
            "scene-writer",
            "character-creator",
            "worldbuilder",
            "authoring-supervisor",
        ] {
            assert!(
                skill_names.contains(&expected),
                "missing skill {expected} in: {skill_names:?}"
            );
        }
        let reference_names: Vec<&str> = content["references"]
            .as_array()
            .expect("references array")
            .iter()
            .filter_map(|reference| reference["name"].as_str())
            .collect();
        assert!(
            reference_names.contains(&"anti-slop"),
            "missing reference anti-slop in: {reference_names:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_skill_returns_markdown_and_unknown_skill_errors() {
        let router = router().await;
        let mut args = serde_json::Map::new();
        args.insert("name".to_string(), "scene-writer".into());

        let result = router
            .call_tool("get_skill", Some(&args))
            .await
            .expect("tool call should return result");

        assert_eq!(result.is_error, Some(false));
        let content = result
            .structured_content
            .expect("get_skill should use structured content");
        assert_eq!(content["name"].as_str(), Some("scene-writer"));
        assert!(
            content["markdown"]
                .as_str()
                .is_some_and(|markdown| !markdown.is_empty()),
            "markdown should be non-empty"
        );

        args.insert("name".to_string(), "not-a-skill".into());
        let result = router
            .call_tool("get_skill", Some(&args))
            .await
            .expect("tool call should return result");
        assert_eq!(result.is_error, Some(true));
        let text = format!("{:?}", result.content.first().expect("error content"));
        assert!(
            text.contains("unknown skill: not-a-skill"),
            "expected error text in: {text}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_reference_returns_markdown_and_unknown_reference_errors() {
        let router = router().await;
        let mut args = serde_json::Map::new();
        args.insert("name".to_string(), "anti-slop".into());

        let result = router
            .call_tool("get_reference", Some(&args))
            .await
            .expect("tool call should return result");

        assert_eq!(result.is_error, Some(false));
        let content = result
            .structured_content
            .expect("get_reference should use structured content");
        assert_eq!(content["name"].as_str(), Some("anti-slop"));
        assert!(
            content["markdown"]
                .as_str()
                .is_some_and(|markdown| !markdown.is_empty()),
            "markdown should be non-empty"
        );

        args.insert("name".to_string(), "not-a-reference".into());
        let result = router
            .call_tool("get_reference", Some(&args))
            .await
            .expect("tool call should return result");
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn skill_tools_are_listed_in_every_tool_profile() {
        for profile in ["import", "write", "minimal", "authoring"] {
            let temp = tempdir().expect("temp dir");
            let db = SqlitePool::open(&temp.path().join("router.db"))
                .await
                .expect("db init");
            let data_dir = temp.keep();
            let router = ToolRouter::with_tool_profile_and_serialization(
                SpindleService::new(SpindleRepository::with_model_router(
                    db,
                    data_dir,
                    ModelRouter::local_only(),
                )),
                Some(profile.to_string()),
                Arc::new(ToolSerializationState::default()),
            );
            let names: Vec<String> = router
                .list_tools()
                .iter()
                .map(|tool| tool.name.to_string())
                .collect();
            for tool_name in ["list_skills", "get_skill", "get_reference"] {
                assert!(
                    names.iter().any(|name| name == tool_name),
                    "profile {profile} is missing tool {tool_name}"
                );
            }
            if profile == "authoring" {
                for step in [
                    "authoring_prepare_run",
                    "authoring_start_run",
                    "authoring_status",
                    "authoring_execute_next",
                    "authoring_save_scene_draft",
                    "commit_scene_changes",
                    "save_summary",
                    "authoring_record_checkpoint_audit",
                    "authoring_review_checkpoint",
                    "authoring_resolve_block",
                    "authoring_cancel_run",
                    "decide_canon_deltas",
                    "decide_plan_amendments",
                    "compile_manuscript",
                    "set_active_project",
                    "save_scene_draft",
                    "annotate_scene_beats",
                    "get_editorial_queue",
                    "decide_editorial_item",
                    "read_episode",
                    "prepare_episode_release",
                    "release_episode",
                    "get_episode_release",
                    "create_plot_line",
                    "create_conflict",
                    "create_character_arc",
                ] {
                    assert!(
                        names.iter().any(|name| name == step),
                        "authoring profile cannot complete {step}"
                    );
                }
                assert!(names.len() < router.all_tools().len() / 2);
            }
        }
    }

    #[test]
    fn session_serialization_is_enabled_for_mutating_tools() {
        assert!(tool_requires_session_serialization("save_scene_draft"));
        assert!(tool_requires_session_serialization("commit_scene_changes"));
        assert!(!tool_requires_session_serialization("get_writer_state"));
        assert!(!tool_requires_session_serialization("get_scene_context"));
        assert!(!tool_requires_session_serialization("list_skills"));
        assert!(!tool_requires_session_serialization("get_skill"));
        assert!(!tool_requires_session_serialization("get_reference"));
    }

    #[test]
    fn serialization_scope_uses_global_only_for_process_wide_tools() {
        assert!(tool_requires_global_serialization("create_project"));
        assert!(tool_requires_global_serialization("configure_agents"));
        assert!(tool_requires_global_serialization("revise_generation"));
        assert!(tool_requires_global_serialization("init_grok_skills"));
        assert!(!tool_requires_global_serialization("save_scene_draft"));
        assert!(!tool_requires_global_serialization("record_note"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn project_scoped_serialization_allows_different_projects_to_lock_independently() {
        let router = router().await;

        let first = router
            .lock_tool_scope(ToolSerializationScope::Project("project:first".to_string()))
            .await;
        let second = timeout(
            Duration::from_millis(100),
            router.lock_tool_scope(ToolSerializationScope::Project(
                "project:second".to_string(),
            )),
        )
        .await;
        assert!(
            second.is_ok(),
            "different projects should not share a mutation lock"
        );

        drop(second);
        drop(first);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn project_scoped_serialization_queues_same_project_mutations() {
        let router = router().await;

        let first = router
            .lock_tool_scope(ToolSerializationScope::Project("project:same".to_string()))
            .await;
        let second = timeout(
            Duration::from_millis(50),
            router.lock_tool_scope(ToolSerializationScope::Project("project:same".to_string())),
        )
        .await;
        assert!(
            second.is_err(),
            "same-project mutations must still serialize"
        );

        drop(first);
        let second = timeout(
            Duration::from_millis(100),
            router.lock_tool_scope(ToolSerializationScope::Project("project:same".to_string())),
        )
        .await;
        assert!(second.is_ok(), "same-project lock should release");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn global_serialization_blocks_project_mutations() {
        let router = router().await;

        let global = router.lock_tool_scope(ToolSerializationScope::Global).await;
        let project = timeout(
            Duration::from_millis(50),
            router.lock_tool_scope(ToolSerializationScope::Project("project:first".to_string())),
        )
        .await;
        assert!(project.is_err(), "global lock must block project writes");

        drop(global);
        let project = timeout(
            Duration::from_millis(100),
            router.lock_tool_scope(ToolSerializationScope::Project("project:first".to_string())),
        )
        .await;
        assert!(
            project.is_ok(),
            "project lock should proceed after global release"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_save_scene_draft_calls_complete_without_hanging() {
        let router = router().await;
        let project = router
            .service
            .create_project(CreateProjectInput {
                name: "Concurrent Save Scene Draft".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "Concurrent scene saves should queue, not hang.".to_string(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await
            .expect("project");

        for scene_order in 1..=3 {
            router
                .service
                .save_scene_draft(SaveSceneDraftInput {
                    project_id: project.project_id.clone(),
                    book_number: 1,
                    chapter_number: 1,
                    chapter_id: None,
                    scene_order,
                    full_text: format!("Scene {scene_order} baseline."),
                    summary: format!("Baseline summary {scene_order}."),
                    content_rating: ContentRating::Teen,
                    tone: Some("grounded".to_string()),
                    source_path: None,
                    generation_id: None,
                    ..Default::default()
                })
                .await
                .expect("seed scene");
        }

        let temp = tempdir().expect("temp dir");
        let chapter_path = temp.path().join("ch26_signal_and_noise.md");
        std::fs::write(&chapter_path, "placeholder chapter source").expect("write chapter file");
        let chapter_path = chapter_path.display().to_string();

        let save_call = |router: ToolRouter, scene_order: i32| {
            let project_id = project.project_id.clone();
            let chapter_path = chapter_path.clone();
            async move {
                let args = serde_json::to_value(SaveSceneDraftInput {
                    project_id,
                    book_number: 1,
                    chapter_number: 1,
                    chapter_id: None,
                    scene_order,
                    full_text: format!("Scene {scene_order} updated from concurrent tool call."),
                    summary: format!("Concurrent summary {scene_order}."),
                    content_rating: ContentRating::Teen,
                    tone: Some("grounded".to_string()),
                    source_path: Some(chapter_path),
                    generation_id: None,
                    ..Default::default()
                })
                .expect("save args");
                let args = args.as_object().cloned().expect("save args object");
                let result = router
                    .call_tool("save_scene_draft", Some(&args))
                    .await
                    .expect("save scene draft");
                let payload = structured_json(result);
                payload["status"].as_str().expect("save status").to_string()
            }
        };

        let joined = timeout(Duration::from_secs(5), async {
            tokio::join!(
                save_call(router.clone(), 1),
                save_call(router.clone(), 2),
                save_call(router.clone(), 3)
            )
        })
        .await
        .expect("concurrent save_scene_draft calls should not hang");

        assert_eq!(joined.0, "updated");
        assert_eq!(joined.1, "updated");
        assert_eq!(joined.2, "updated");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_schemas_contain_no_gemini_incompatible_keywords() {
        let router = router().await;
        let tools = router.list_tools();
        let forbidden = ["$ref", "$defs", "definitions", "$schema", "$id"];
        for tool in &tools {
            let schema_json = serde_json::to_string(&tool.input_schema).expect("serialize");
            for keyword in &forbidden {
                assert!(
                    !schema_json.contains(keyword),
                    "tool '{}' schema contains forbidden keyword '{}':\n{}",
                    tool.name,
                    keyword,
                    serde_json::to_string_pretty(&tool.input_schema).unwrap()
                );
            }
        }
    }

    /// Recursively collect every `format` string value present anywhere in a
    /// schema JSON tree, tagged with the JSON-pointer-ish path where it lives.
    /// Used by the interop-sweep tests to name offenders precisely.
    fn collect_null_enum_paths(value: &Value, path: &str, out: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::Array(items)) = map.get("enum")
                    && items.iter().any(Value::is_null)
                {
                    out.push(format!("{path}/enum"));
                }
                for (key, child) in map {
                    collect_null_enum_paths(child, &format!("{path}/{key}"), out);
                }
            }
            Value::Array(items) => {
                for (idx, item) in items.iter().enumerate() {
                    collect_null_enum_paths(item, &format!("{path}[{idx}]"), out);
                }
            }
            _ => {}
        }
    }

    /// Regression pin for the Moonshot rejection: `enum value (<nil>) does
    /// not match any type in [string]`. No served schema may carry a null
    /// enum entry, input or output, anywhere in the tree.
    #[tokio::test]
    async fn served_tool_schemas_carry_no_null_enum_values() {
        let router = router().await;
        let tools = router.list_tools();
        assert!(!tools.is_empty(), "expected registered tools");

        let mut offenders: Vec<String> = Vec::new();
        for tool in &tools {
            let input = Value::Object((*tool.input_schema).clone());
            collect_null_enum_paths(&input, &format!("{}#input", tool.name), &mut offenders);
            if let Some(output) = &tool.output_schema {
                let output = Value::Object((**output).clone());
                collect_null_enum_paths(&output, &format!("{}#output", tool.name), &mut offenders);
            }
        }

        assert!(
            offenders.is_empty(),
            "found {} enum arrays containing null in served schemas; sample:\n{}",
            offenders.len(),
            offenders
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn strip_null_enum_values_drops_nulls_and_empty_enums() {
        let mut value = serde_json::json!({
            "properties": {
                "scope": {"type": "string", "enum": ["book", "chapter", null]},
                "only_null": {"enum": [null]},
                "clean": {"type": "string", "enum": ["a", "b"]},
                "nested": {"items": {"anyOf": [{"enum": ["x", null]}]}}
            }
        });
        strip_null_enum_values(&mut value);
        assert_eq!(
            value["properties"]["scope"]["enum"],
            serde_json::json!(["book", "chapter"])
        );
        assert!(
            value["properties"]["only_null"].get("enum").is_none(),
            "an enum emptied by null-removal must be dropped entirely"
        );
        assert_eq!(
            value["properties"]["clean"]["enum"],
            serde_json::json!(["a", "b"])
        );
        assert_eq!(
            value["properties"]["nested"]["items"]["anyOf"][0]["enum"],
            serde_json::json!(["x"])
        );
    }

    fn collect_format_values(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    if k == "format"
                        && let Value::String(s) = v
                    {
                        out.push((format!("{path}/format"), s.clone()));
                    }
                    collect_format_values(v, &format!("{path}/{k}"), out);
                }
            }
            Value::Array(arr) => {
                for (i, v) in arr.iter().enumerate() {
                    collect_format_values(v, &format!("{path}/{i}"), out);
                }
            }
            _ => {}
        }
    }

    /// The denylist of schemars' non-standard numeric `format` values that
    /// strict JSON-Schema clients (Kimi Code et al.) warn about.
    fn is_denylisted_numeric_format(s: &str) -> bool {
        matches!(s, "float" | "double")
            || (s
                .strip_prefix("int")
                .or_else(|| s.strip_prefix("uint"))
                .is_some_and(|rest| {
                    // `int`/`uint` alone, or followed only by digits (int8..64, uint8..64)
                    rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit())
                })
                && (s.starts_with("int") || s.starts_with("uint")))
    }

    /// REGRESSION PIN: every registered tool's input AND output schema, as
    /// served over MCP, must contain zero denylisted numeric `format` values
    /// anywhere in the JSON tree. This catches any future count-heavy tool that
    /// reintroduces schemars' non-standard formats at the serving seam.
    #[tokio::test(flavor = "current_thread")]
    async fn served_tool_schemas_carry_no_nonstandard_numeric_formats() {
        let router = router().await;
        let tools = router.list_tools();
        assert!(!tools.is_empty(), "expected registered tools");

        let mut offenders: Vec<String> = Vec::new();
        for tool in &tools {
            let input = Value::Object((*tool.input_schema).clone());
            let mut found = Vec::new();
            collect_format_values(&input, &format!("{}#input", tool.name), &mut found);
            if let Some(output) = &tool.output_schema {
                let output = Value::Object((**output).clone());
                collect_format_values(&output, &format!("{}#output", tool.name), &mut found);
            }
            for (path, fmt) in found {
                if is_denylisted_numeric_format(&fmt) {
                    offenders.push(format!("{path} = \"{fmt}\""));
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "found {} non-standard numeric `format` annotations in served schemas; sample:\n{}",
            offenders.len(),
            offenders
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// INTEGRATION-SHAPE PIN: `import_hydrate_bible`'s output schema carries
    /// `ImportHydrationRecordCount` (a `$defs` subschema of `usize` counts) —
    /// exactly the shape whose `#/$defs/ImportHydrationRecordCount/properties/created`
    /// path Kimi Code warned about. Assert the previously-observed paths are clean
    /// while the surrounding structure (the $def, its properties) survives intact.
    #[tokio::test(flavor = "current_thread")]
    async fn import_hydrate_bible_output_schema_is_format_clean() {
        let router = router().await;
        let tools = router.list_tools();
        let tool = tools
            .iter()
            .find(|t| t.name == "import_hydrate_bible")
            .expect("import_hydrate_bible tool");
        let output = tool
            .output_schema
            .as_ref()
            .expect("import_hydrate_bible has an output schema");
        let output = Value::Object((**output).clone());

        // Structure preserved: the $def and its count properties still exist.
        let created = output
            .pointer("/$defs/ImportHydrationRecordCount/properties/created")
            .expect("ImportHydrationRecordCount.created property must survive scrubbing");
        assert_eq!(
            created.get("type").and_then(Value::as_str),
            Some("integer"),
            "count property keeps its integer type"
        );
        // The offending format is gone from every observed path.
        assert!(
            created.get("format").is_none(),
            "created must not carry a non-standard `format`: {created}"
        );

        // Whole-tree sweep on this specific schema.
        let mut found = Vec::new();
        collect_format_values(&output, "#", &mut found);
        let offenders: Vec<_> = found
            .into_iter()
            .filter(|(_, f)| is_denylisted_numeric_format(f))
            .collect();
        assert!(
            offenders.is_empty(),
            "import_hydrate_bible output schema still has numeric formats: {offenders:?}"
        );
    }

    #[test]
    fn strip_nonstandard_formats_removes_denylisted_numeric_formats() {
        // Nested $defs, anyOf branches, arrays-of-schemas, items,
        // additionalProperties — the scrubber must reach all of them.
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "created": {"type": "integer", "format": "uint"},
                "ratio": {"type": "number", "format": "double"},
                "count32": {"type": "integer", "format": "uint32"},
                "signed": {"type": "integer", "format": "int64"},
                "tags": {
                    "type": "array",
                    "items": {"type": "integer", "format": "uint8"}
                },
                "maybe": {
                    "anyOf": [
                        {"type": "integer", "format": "int16"},
                        {"type": "null"}
                    ]
                },
                "extra": {
                    "type": "object",
                    "additionalProperties": {"type": "integer", "format": "usize"}
                }
            },
            "$defs": {
                "Row": {
                    "type": "object",
                    "properties": {
                        "n": {"type": "integer", "format": "uint64"},
                        "f": {"type": "number", "format": "float"}
                    }
                }
            }
        });

        strip_nonstandard_formats(&mut schema);

        // No denylisted format survives anywhere.
        let mut found = Vec::new();
        collect_format_values(&schema, "#", &mut found);
        let survivors: Vec<_> = found
            .iter()
            .filter(|(_, f)| is_denylisted_numeric_format(f))
            .collect();
        assert!(
            survivors.is_empty(),
            "denylisted formats survived scrubbing: {survivors:?}"
        );

        // Note: "usize" is not `^u?int\d*$`, so it is intentionally NOT stripped
        // by the denylist. Assert it is untouched (schemars does not emit it, but
        // this documents the exact denylist boundary).
        assert_eq!(
            schema.pointer("/properties/extra/additionalProperties/format"),
            Some(&Value::String("usize".to_string())),
            "usize is outside the numeric denylist and must be preserved"
        );

        // Structure is otherwise byte-identical: types, properties, $defs remain.
        assert_eq!(
            schema.pointer("/properties/created/type"),
            Some(&Value::String("integer".to_string()))
        );
        assert!(schema.pointer("/$defs/Row/properties/n").is_some());
        assert!(schema.pointer("/properties/maybe/anyOf/0/type").is_some());
        assert!(schema.pointer("/properties/tags/items/type").is_some());
    }

    #[test]
    fn strip_nonstandard_formats_preserves_standard_formats() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "when": {"type": "string", "format": "date-time"},
                "id": {"type": "string", "format": "uuid"},
                "link": {"type": "string", "format": "uri"},
                "mail": {"type": "string", "format": "email"},
                "pat": {"type": "string", "format": "regex"}
            }
        });
        let before = schema.clone();
        strip_nonstandard_formats(&mut schema);
        assert_eq!(
            schema, before,
            "standard string formats must survive untouched"
        );
    }

    #[test]
    fn strip_nonstandard_formats_ignores_unrelated_format_valued_data() {
        // A `format` key whose value is NOT a denylisted numeric string must be
        // left alone — e.g. a description literal or an unrelated enum member.
        // (A schema document cannot contain a data object with a coincidental
        // numeric-format-valued "format" key, so the value-based rule is safe.)
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "output": {"type": "string", "format": "markdown"},
                "note": {"type": "string", "description": "format: uint is a warning"}
            }
        });
        let before = schema.clone();
        strip_nonstandard_formats(&mut schema);
        assert_eq!(
            schema, before,
            "non-denylisted format values and description text must be preserved"
        );
    }

    #[test]
    fn structured_result_flattens_nested_record_ids() {
        let payload = serde_json::json!({
            "id": {"tb": "scene", "id": {"String": "abc123"}},
            "nested": {
                "character_id": {"tb": "character", "id": "mara"}
            },
            "items": [
                {"tb": "world_rule", "id": {"String": "law-1"}}
            ]
        });

        let result = structured_result(&payload).expect("structured result");

        assert_eq!(result.is_error, Some(false));
        let structured = result.structured_content.expect("structured content");
        assert_eq!(structured["id"], serde_json::json!("scene:abc123"));
        assert_eq!(
            structured["nested"]["character_id"],
            serde_json::json!("character:mara")
        );
        assert_eq!(
            structured["items"][0],
            serde_json::json!("world_rule:law-1")
        );
    }

    #[test]
    fn parse_arguments_coerces_stringified_arrays_for_schema_arrays() {
        let args = serde_json::json!({
            "project_id": "project:test",
            "book_number": "1",
            "chapter_number": "2",
            "scene_order": "3",
            "character_ids": "[\"character:alpha\",\"character:beta\"]",
            "location_id": "location:arena",
            "sections": "[\"scene\",\"world_rules\"]"
        });
        let args = args
            .as_object()
            .cloned()
            .expect("tool args should be object");

        let parsed: GetSceneContextInput =
            parse_arguments(Some(&args)).expect("arguments should coerce");

        assert_eq!(parsed.book_number, 1);
        assert_eq!(parsed.chapter_number, 2);
        assert_eq!(parsed.scene_order, 3);
        assert_eq!(
            parsed.character_ids,
            vec!["character:alpha".to_string(), "character:beta".to_string()]
        );
        assert_eq!(
            parsed.sections,
            Some(vec!["scene".to_string(), "world_rules".to_string()])
        );
    }

    #[test]
    fn parse_arguments_keeps_mixed_type_update_entity_changes_object() {
        // Regression: a mixed-type payload — one string field plus one array
        // field in the same `changes` object — was rejected upstream with
        // "update_entity changes must be a JSON object" despite changes being
        // a valid object. Schema coercion must not reshape or drop a
        // heterogeneous change map, and a stringified-object form (some
        // clients stringify nested objects) must be parsed back.
        let args = serde_json::json!({
            "entity_type": "character",
            "entity_id": "character:mara",
            "changes": {
                "summary": "A smuggler with a salt blade.",
                "aliases": ["The Salt Blade", "Gull"]
            }
        });
        let args = args
            .as_object()
            .cloned()
            .expect("tool args should be object");
        let parsed: UpdateEntityInput =
            parse_arguments(Some(&args)).expect("arguments should parse");
        assert_eq!(parsed.entity_type, "character");
        let changes = parsed
            .changes
            .as_object()
            .expect("changes must remain a JSON object");
        assert_eq!(
            changes["summary"],
            serde_json::json!("A smuggler with a salt blade.")
        );
        assert_eq!(
            changes["aliases"],
            serde_json::json!(["The Salt Blade", "Gull"])
        );

        // Stringified-object form coerces back to an object.
        let args = serde_json::json!({
            "entity_type": "character",
            "entity_id": "character:mara",
            "changes": "{\"summary\":\"S\",\"aliases\":[\"A\"]}"
        });
        let args = args
            .as_object()
            .cloned()
            .expect("tool args should be object");
        let parsed: UpdateEntityInput =
            parse_arguments(Some(&args)).expect("stringified changes should coerce");
        assert!(
            parsed.changes.is_object(),
            "stringified changes must coerce to an object"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tool_schemas_relax_session_default_project_and_branch_fields() {
        let router = router().await;
        let tools = router.list_tools();
        let required_fields = |tool: &Tool| {
            tool.input_schema
                .get("required")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };

        let create_character = tools
            .iter()
            .find(|tool| tool.name == "create_character")
            .expect("create_character tool");
        let create_character_required = required_fields(create_character);
        assert!(
            !create_character_required
                .iter()
                .any(|entry| entry.as_str() == Some("project_id"))
        );

        let update_writer_position = tools
            .iter()
            .find(|tool| tool.name == "update_writer_position")
            .expect("update_writer_position tool");
        let update_required = required_fields(update_writer_position);
        assert!(
            !update_required
                .iter()
                .any(|entry| entry.as_str() == Some("project_id"))
        );
        assert!(
            !update_required
                .iter()
                .any(|entry| entry.as_str() == Some("branch_id"))
        );

        let batch_create_terms = tools
            .iter()
            .find(|tool| tool.name == "batch_create_terms")
            .expect("batch_create_terms tool");
        let batch_terms_required = required_fields(batch_create_terms);
        assert!(
            !batch_terms_required
                .iter()
                .any(|entry| entry.as_str() == Some("project_id"))
        );

        let batch_set_voice_profiles = tools
            .iter()
            .find(|tool| tool.name == "batch_set_character_voice_profiles")
            .expect("batch_set_character_voice_profiles tool");
        let batch_voice_required = required_fields(batch_set_voice_profiles);
        assert!(
            !batch_voice_required
                .iter()
                .any(|entry| entry.as_str() == Some("project_id"))
        );
        assert!(
            !batch_voice_required
                .iter()
                .any(|entry| entry.as_str() == Some("branch_id"))
        );

        let set_active_project = tools
            .iter()
            .find(|tool| tool.name == "set_active_project")
            .expect("set_active_project tool");
        let set_active_required = required_fields(set_active_project);
        assert!(
            set_active_required
                .iter()
                .any(|entry| entry.as_str() == Some("project_id"))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_project_sets_session_defaults_for_follow_up_tools() {
        let router = router().await;

        let create_project_args = serde_json::to_value(CreateProjectInput {
            name: "Session Defaults".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "Session defaults should remove redundant ids.".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .expect("create project args");
        let create_project_args = create_project_args
            .as_object()
            .cloned()
            .expect("create project object");
        let project: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&create_project_args))
                .await
                .expect("create project"),
        ))
        .expect("decode create project");

        let mut create_character_args = serde_json::to_value(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: "project:placeholder".to_string(),
            name: "Liora".to_string(),
            summary: "A courier with a perfect memory.".to_string(),
            role: "protagonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                vocabulary: vec![],
                sentence_structure: vec![],
                tics: vec![],
                forbidden_words: vec![],
                example_lines: vec![],
                tone: None,
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: Default::default(),
                suppressed: vec![],
                triggers: vec![],
                defense_mechanisms: vec![],
                flex_range: None,
            },
            initial_state: None,
        })
        .expect("create character args");
        let create_character_args = create_character_args
            .as_object_mut()
            .expect("create character object");
        create_character_args.remove("project_id");
        let character: CreateCharacterOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_character", Some(create_character_args))
                .await
                .expect("create character with session default"),
        ))
        .expect("decode create character");

        let mut batch_voice_profile_args =
            serde_json::to_value(BatchSetCharacterVoiceProfilesInput {
                project_id: "project:placeholder".to_string(),
                branch_id: "bible_branch:main".to_string(),
                items: vec![BatchSetCharacterVoiceProfileItem {
                    character_id: character.character_id.clone(),
                    profile: CharacterVoiceProfileData {
                        vocabulary: vec!["ash".to_string()],
                        sentence_structure: vec!["short".to_string()],
                        tics: vec!["counts exits".to_string()],
                        forbidden_words: vec![],
                        example_lines: vec!["We move before the gate fails.".to_string()],
                        tone: Some("clipped".to_string()),
                        established_in_scene_id: None,
                        updated_at: None,
                    },
                }],
            })
            .expect("batch voice args");
        let batch_voice_profile_args = batch_voice_profile_args
            .as_object_mut()
            .expect("batch voice profile object");
        batch_voice_profile_args.remove("project_id");
        batch_voice_profile_args.remove("branch_id");
        let batch_voice_profiles: BatchSetCharacterVoiceProfilesOutput =
            serde_json::from_value(structured_json(
                router
                    .call_tool(
                        "batch_set_character_voice_profiles",
                        Some(batch_voice_profile_args),
                    )
                    .await
                    .expect("batch set voice profiles with session defaults"),
            ))
            .expect("decode batch voice profiles");
        assert_eq!(batch_voice_profiles.updated, 1);
        assert_eq!(
            batch_voice_profiles.profiles[0].character_id,
            character.character_id
        );
        // Per-project main branch (Phase 6): use the project's actual
        // branch_id from create_project rather than the legacy literal.
        assert_eq!(
            batch_voice_profiles.profiles[0].branch_id,
            project.branch_id
        );

        let mut update_writer_position_args = serde_json::to_value(UpdateWriterPositionInput {
            project_id: "project:placeholder".to_string(),
            branch_id: project.branch_id.clone(),
            book_id: None,
            chapter_id: None,
            scene_id: None,
            intent: "planning".to_string(),
            next_focus: Some("Outline the next scene.".to_string()),
        })
        .expect("writer position args");
        let update_writer_position_args = update_writer_position_args
            .as_object_mut()
            .expect("writer position object");
        update_writer_position_args.remove("project_id");
        update_writer_position_args.remove("branch_id");
        let position: WriterPosition = serde_json::from_value(structured_json(
            router
                .call_tool("update_writer_position", Some(update_writer_position_args))
                .await
                .expect("update writer position with session defaults"),
        ))
        .expect("decode writer position");
        assert_eq!(position.project_id, project.project_id);
        assert_eq!(position.branch_id, project.branch_id);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_character_infers_single_project_without_active_session_project() {
        let router = router().await;

        let project = router
            .service
            .create_project(CreateProjectInput {
                name: "Implicit Project".to_string(),
                project_type: "novel".to_string(),
                genre: "sports drama".to_string(),
                reader_contract: ReaderContract {
                    promise: "Single-project sessions should infer project context.".to_string(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await
            .expect("create project directly");

        let mut create_character_args = serde_json::to_value(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: "project:placeholder".to_string(),
            name: "Mike Petrovic".to_string(),
            summary: "Head coach.".to_string(),
            role: "supporting".to_string(),
            realm: Some("Livonia".to_string()),
            voice_profile: CharacterVoiceProfileData {
                vocabulary: vec![],
                sentence_structure: vec![],
                tics: vec![],
                forbidden_words: vec![],
                example_lines: vec![],
                tone: Some("dry".to_string()),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: Default::default(),
                suppressed: vec![],
                triggers: vec![],
                defense_mechanisms: vec![],
                flex_range: None,
            },
            initial_state: None,
        })
        .expect("create character args");
        let create_character_args = create_character_args
            .as_object_mut()
            .expect("create character object");
        create_character_args.remove("project_id");

        let character: CreateCharacterOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_character", Some(create_character_args))
                .await
                .expect("create character with inferred project"),
        ))
        .expect("decode create character");
        assert!(character.character_id.starts_with("character:"));

        let writer_state = structured_json(
            router
                .call_tool("get_writer_state", Some(&serde_json::Map::new()))
                .await
                .expect("writer state after inferred project"),
        );
        assert_eq!(
            writer_state["current"]["project"]["project_id"],
            serde_json::json!(project.project_id)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_character_without_project_id_errors_when_session_is_ambiguous() {
        let router = router().await;

        for name in ["First Project", "Second Project"] {
            router
                .service
                .create_project(CreateProjectInput {
                    name: name.to_string(),
                    project_type: "novel".to_string(),
                    genre: "sports drama".to_string(),
                    reader_contract: ReaderContract {
                        promise: "Ambiguous sessions should fail clearly.".to_string(),
                        style_notes: vec![],
                        boundaries: vec![],
                    },
                })
                .await
                .expect("create project directly");
        }

        let mut create_character_args = serde_json::to_value(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: "project:placeholder".to_string(),
            name: "Danny Voss".to_string(),
            summary: "Undersized center.".to_string(),
            role: "supporting".to_string(),
            realm: Some("Westland".to_string()),
            voice_profile: CharacterVoiceProfileData {
                vocabulary: vec![],
                sentence_structure: vec![],
                tics: vec![],
                forbidden_words: vec![],
                example_lines: vec![],
                tone: Some("quick".to_string()),
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: Default::default(),
                suppressed: vec![],
                triggers: vec![],
                defense_mechanisms: vec![],
                flex_range: None,
            },
            initial_state: None,
        })
        .expect("create character args");
        let create_character_args = create_character_args
            .as_object_mut()
            .expect("create character object");
        create_character_args.remove("project_id");

        let result = router
            .call_tool("create_character", Some(create_character_args))
            .await
            .expect("tool result");
        assert_eq!(result.is_error, Some(true));
        let text = format!("{:?}", result.content.first().expect("error content"));
        assert!(text.contains("set_active_project"));
        assert!(text.contains("project_id"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn update_promise_status_accepts_common_alias_fields() {
        let router = router().await;

        let project = router
            .service
            .create_project(CreateProjectInput {
                name: "Promise Alias Test".to_string(),
                project_type: "novel".to_string(),
                genre: "sports drama".to_string(),
                reader_contract: ReaderContract {
                    promise: "Alias fields should deserialize for promise updates.".to_string(),
                    style_notes: vec![],
                    boundaries: vec![],
                },
            })
            .await
            .expect("create project directly");

        let promise = router
            .service
            .create_narrative_promise(CreateNarrativePromiseInput {
                project_id: project.project_id,
                promise_type: "callback".to_string(),
                description: "Hotel room details recur later.".to_string(),
                planted_at: StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(1),
                    note: None,
                },
                planned_payoff: Some(StoryPlacement {
                    book_number: 1,
                    chapter_number: 25,
                    scene_order: Some(1),
                    note: None,
                }),
                notes: vec![],
            })
            .await
            .expect("create promise directly");

        let args = serde_json::json!({
            "promise_id": promise.narrative_promise_id,
            "new_status": "planted",
            "scene_id": "scene:unused-alias-check",
            "note": "Specific hotel room details planted in scene 1."
        });
        let args = args.as_object().cloned().expect("alias args object");

        let updated: UpdatePromiseStatusOutput = serde_json::from_value(structured_json(
            router
                .call_tool("update_promise_status", Some(&args))
                .await
                .expect("update promise with alias fields"),
        ))
        .expect("decode updated promise");

        assert_eq!(updated.status, "planted");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_active_project_switches_session_defaults() {
        let router = router().await;

        let create_project = |name: &str| CreateProjectInput {
            name: name.to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "Project switching should be session scoped.".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        };

        let first_args =
            serde_json::to_value(create_project("First Project")).expect("first project args");
        let first_args = first_args
            .as_object()
            .cloned()
            .expect("first project object");
        let first: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&first_args))
                .await
                .expect("create first project"),
        ))
        .expect("decode first project");

        let second_args =
            serde_json::to_value(create_project("Second Project")).expect("second project args");
        let second_args = second_args
            .as_object()
            .cloned()
            .expect("second project object");
        let second: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&second_args))
                .await
                .expect("create second project"),
        ))
        .expect("decode second project");

        let set_active_args = serde_json::to_value(SetActiveProjectInput {
            project_id: first.project_id.clone(),
            branch_id: None,
        })
        .expect("set active args");
        let set_active_args = set_active_args
            .as_object()
            .cloned()
            .expect("set active object");
        let active: SetActiveProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("set_active_project", Some(&set_active_args))
                .await
                .expect("set active project"),
        ))
        .expect("decode active project");
        assert_eq!(active.project_id, first.project_id);
        // Per-project main branch (Phase 6): use the project's actual
        // branch_id rather than the legacy singleton literal.
        assert_eq!(active.branch_id, first.branch_id);

        let writer_state_args = serde_json::json!({
            "format": "json",
            "budget_tokens": 2000,
            "include_subjects": false,
            "include_recent_activity": false,
            "recent_activity_limit": 0
        });
        let writer_state_args = writer_state_args
            .as_object()
            .cloned()
            .expect("writer state object");
        let writer_state = structured_json(
            router
                .call_tool("get_writer_state", Some(&writer_state_args))
                .await
                .expect("writer state with session project"),
        );
        assert_eq!(
            writer_state["current"]["project"]["project_id"],
            serde_json::json!(first.project_id)
        );
        assert_ne!(
            writer_state["current"]["project"]["project_id"],
            serde_json::json!(second.project_id)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn set_active_project_does_not_depend_on_writer_state_budget() {
        let router = router().await;

        let create_project_args = serde_json::to_value(CreateProjectInput {
            name: "Budget Independent Session Defaults".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "Session defaults should not require a writer-state bundle.".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .expect("create project args");
        let create_project_args = create_project_args
            .as_object()
            .cloned()
            .expect("create project object");
        let project: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&create_project_args))
                .await
                .expect("create project"),
        ))
        .expect("decode project");

        let long_rule_body = "Magic rewrites cause cascading constraint debt across every \
            revision boundary and must be treated as binding law in all future scenes. "
            .repeat(40);
        for index in 0..12 {
            router
                .service
                .create_world_rule(CreateWorldRuleInput {
                    project_id: project.project_id.clone(),
                    rule_name: format!("Constraint {}", index + 1),
                    rule_type: "law".to_string(),
                    description: format!("{}{}", long_rule_body, index + 1),
                    scan_pattern: None,
                    relevance_tags: vec![],
                    established_in: None,
                })
                .await
                .expect("create world rule");
        }

        let writer_state_args = serde_json::json!({
            "project_id": project.project_id.clone(),
            "format": "json",
            "budget_tokens": 2000,
            "include_subjects": false,
            "include_recent_activity": false,
            "recent_activity_limit": 0
        });
        let writer_state_args = writer_state_args
            .as_object()
            .cloned()
            .expect("writer state object");
        let writer_state_error = router
            .call_tool("get_writer_state", Some(&writer_state_args))
            .await
            .expect("writer state should return a structured error result");
        assert_eq!(writer_state_error.is_error, Some(true));
        let writer_state_error = format!(
            "{:?}",
            writer_state_error.content.first().expect("error content")
        );
        assert!(
            writer_state_error.contains("mandatory writer-state sections"),
            "expected writer-state budget failure, got: {writer_state_error}"
        );

        let set_active_args = serde_json::to_value(SetActiveProjectInput {
            project_id: project.project_id.clone(),
            branch_id: None,
        })
        .expect("set active args");
        let set_active_args = set_active_args
            .as_object()
            .cloned()
            .expect("set active object");
        let active: SetActiveProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("set_active_project", Some(&set_active_args))
                .await
                .expect("set active project"),
        ))
        .expect("decode set active project");
        assert_eq!(active.project_id, project.project_id);
        // Per-project main branches (Phase 6 reconciliation, Risk #6): the
        // SurrealDB-era singleton `bible_branch:main` no longer exists.
        // Every project owns its own main branch with a ULID-flavoured id,
        // surfaced via `CreateProjectOutput.branch_id`. The session default
        // resolved through set_active_project must round-trip to that id.
        assert_eq!(active.branch_id, project.branch_id);
    }

    #[tokio::test]
    async fn test_style_profile_mcp_tools() {
        let router = router().await;
        let tools = router.list_tools();
        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(tool_names.contains(&"create_style_profile_from_markdown"));
        assert!(tool_names.contains(&"list_style_profiles"));
        assert!(tool_names.contains(&"get_style_profile"));
        assert!(tool_names.contains(&"apply_style_profile"));
        assert!(tool_names.contains(&"preview_apply_style_profile"));
        assert!(tool_names.contains(&"list_style_profile_applications"));
        assert!(tool_names.contains(&"rollback_style_profile_application"));
        assert!(tool_names.contains(&"check_style_profile_sources"));
        assert!(tool_names.contains(&"preview_refresh_style_profile"));
        assert!(tool_names.contains(&"refresh_style_profile"));
        assert!(tool_names.contains(&"check_style_against_profile"));
        assert!(tool_names.contains(&"compare_style_profiles"));
        assert!(tool_names.contains(&"archive_style_profile"));
        assert!(tool_names.contains(&"plan_style_revision"));
        assert!(tool_names.contains(&"preview_style_revision_patch"));
        assert!(tool_names.contains(&"evaluate_style_revision_patch"));
        assert!(tool_names.contains(&"apply_style_revision_patch"));
        assert!(tool_names.contains(&"list_style_revision_patch_audits"));
        assert!(tool_names.contains(&"rollback_style_revision_patch"));
    }

    #[tokio::test]
    async fn compile_manuscript_tool_reads_chapters_and_flags_undrafted_scenes() {
        let router = router().await;

        let create_project_args = serde_json::to_value(CreateProjectInput {
            name: "Compile MCP".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "read so far".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .expect("create project args");
        let create_project_args = create_project_args
            .as_object()
            .cloned()
            .expect("create project object");
        let project: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&create_project_args))
                .await
                .expect("create project"),
        ))
        .expect("project output");

        // One committed scene in chapter 1.
        let save_args = serde_json::to_value(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "The tool assembles this prose.".to_string(),
            summary: "s".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            source_path: None,
            generation_id: None,
            ..Default::default()
        })
        .expect("save args");
        let save_args = save_args.as_object().cloned().expect("save args object");
        router
            .call_tool("save_scene_draft", Some(&save_args))
            .await
            .expect("save scene draft");

        // Chapter 2 has a plan with an undrafted second scene.
        let plan_args = serde_json::to_value(PlanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 2,
            pov_character_id: None,
            synopsis: "planned".to_string(),
            target_theme_ids: vec![],
            target_conflict_ids: vec![],
            target_plot_line_ids: vec![],
            scenes: vec![PlanChapterSceneInput {
                scene_order: 1,
                summary: "undrafted".to_string(),
                purpose: "establishing".to_string(),
                ..Default::default()
            }],
        })
        .expect("plan args");
        let plan_args = plan_args.as_object().cloned().expect("plan args object");
        router
            .call_tool("plan_chapter", Some(&plan_args))
            .await
            .expect("plan chapter");

        let compile_args = serde_json::to_value(CompileManuscriptInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            start_chapter: None,
            end_chapter: None,
            write_to_workspace: false,
        })
        .expect("compile args");
        let compile_args = compile_args
            .as_object()
            .cloned()
            .expect("compile args object");
        let out: CompileManuscriptOutput = serde_json::from_value(structured_json(
            router
                .call_tool("compile_manuscript", Some(&compile_args))
                .await
                .expect("compile manuscript"),
        ))
        .expect("compile output");

        assert_eq!(out.scene_count, 1);
        assert!(out.markdown.contains("The tool assembles this prose."));
        assert!(out.markdown.contains("> [scene 2.1 not yet drafted]"));
        assert_eq!(out.missing_scenes, vec!["2.1".to_string()]);
    }

    #[tokio::test]
    async fn export_recap_tool_folds_summary_end_to_end() {
        let router = router().await;

        let create_project_args = serde_json::to_value(CreateProjectInput {
            name: "Recap MCP".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "read so far".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .expect("create project args");
        let create_project_args = create_project_args
            .as_object()
            .cloned()
            .expect("create project object");
        let project: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&create_project_args))
                .await
                .expect("create project"),
        ))
        .expect("project output");

        let save_summary_args = serde_json::to_value(SaveSummaryInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            entity_type: None,
            entity_id: None,
            summary: "The recap tool folds this chapter.".to_string(),
            key_events: vec![],
            character_changes: vec![],
            relationship_shifts: vec![],
            arc_advances: vec![],
            promise_events: vec![],
        })
        .expect("save summary args");
        let save_summary_args = save_summary_args
            .as_object()
            .cloned()
            .expect("save summary object");
        router
            .call_tool("save_summary", Some(&save_summary_args))
            .await
            .expect("save summary");

        let recap_args = serde_json::to_value(ExportRecapInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            through_chapter: 1,
            write_to_workspace: false,
        })
        .expect("recap args");
        let recap_args = recap_args.as_object().cloned().expect("recap args object");
        let out: ExportRecapOutput = serde_json::from_value(structured_json(
            router
                .call_tool("export_recap", Some(&recap_args))
                .await
                .expect("export recap"),
        ))
        .expect("recap output");

        assert_eq!(out.chapter_count, 1);
        assert!(out.markdown.contains("The recap tool folds this chapter."));
        assert!(out.word_count > 0);
    }

    #[tokio::test]
    async fn export_series_bible_tool_lists_character_end_to_end() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput,
        };
        let router = router().await;

        let create_project_args = serde_json::to_value(CreateProjectInput {
            name: "Bible MCP".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "series bible".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .expect("create project args");
        let create_project_args = create_project_args
            .as_object()
            .cloned()
            .expect("create project object");
        let project: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&create_project_args))
                .await
                .expect("create project"),
        ))
        .expect("project output");

        let char_args = serde_json::to_value(CreateCharacterInput {
            aliases: Vec::new(),
            project_id: project.project_id.clone(),
            name: "Bellwether".to_string(),
            summary: "The bell-ringer of the keep.".to_string(),
            role: "protagonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: None,
                vocabulary: vec![],
                sentence_structure: vec![],
                tics: vec![],
                forbidden_words: vec![],
                example_lines: vec![],
                established_in_scene_id: None,
                updated_at: None,
            },
            emotional_profile: CharacterEmotionalProfileData {
                base_emotions: std::collections::BTreeMap::new(),
                suppressed: vec![],
                triggers: vec![],
                defense_mechanisms: vec![],
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
        .expect("char args");
        let char_args = char_args.as_object().cloned().expect("char args object");
        router
            .call_tool("create_character", Some(&char_args))
            .await
            .expect("create character");

        let bible_args = serde_json::to_value(ExportSeriesBibleInput {
            project_id: project.project_id.clone(),
            through: None,
            write_to_workspace: false,
        })
        .expect("bible args");
        let bible_args = bible_args.as_object().cloned().expect("bible args object");
        let out: ExportSeriesBibleOutput = serde_json::from_value(structured_json(
            router
                .call_tool("export_series_bible", Some(&bible_args))
                .await
                .expect("export series bible"),
        ))
        .expect("bible output");

        assert_eq!(out.chapter_count, 1);
        assert!(out.markdown.contains("Bellwether"));
        assert!(out.word_count > 0);
    }

    #[tokio::test]
    async fn mine_scene_canon_tool_stages_deltas_end_to_end() {
        let router = router().await;

        let create_project_args = serde_json::to_value(CreateProjectInput {
            name: "Mine MCP".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "mined canon".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .expect("create project args");
        let create_project_args = create_project_args
            .as_object()
            .cloned()
            .expect("create project object");
        let project: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&create_project_args))
                .await
                .expect("create project"),
        ))
        .expect("project output");

        // One committed scene carrying the mining sentinel.
        let save_args = serde_json::to_value(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "A grey tower loomed. MOCK_CANON_MINE stood at its gate.".to_string(),
            summary: "s".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            source_path: None,
            generation_id: None,
            ..Default::default()
        })
        .expect("save args");
        let save_args = save_args.as_object().cloned().expect("save args object");
        let saved: SaveSceneDraftOutput = serde_json::from_value(structured_json(
            router
                .call_tool("save_scene_draft", Some(&save_args))
                .await
                .expect("save scene draft"),
        ))
        .expect("save output");

        let mine_args = serde_json::to_value(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: saved.scene_id.clone(),
        })
        .expect("mine args");
        let mine_args = mine_args.as_object().cloned().expect("mine args object");
        let out: MineSceneCanonOutput = serde_json::from_value(structured_json(
            router
                .call_tool("mine_scene_canon", Some(&mine_args))
                .await
                .expect("mine scene canon"),
        ))
        .expect("mine output");

        assert_eq!(out.status, "staged");
        assert_eq!(out.staged.len(), 1);
        assert_eq!(out.staged[0].delta_class, "canonical_fact");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn canon_delta_ratify_round_trip_via_mcp() {
        // End-to-end over the MCP surface: stage via mine (sentinel), list the
        // queue, apply the one delta, and confirm the fact is real canon.
        let router = router().await;

        let create_project_args = serde_json::to_value(CreateProjectInput {
            name: "Ratify MCP".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "ratified canon".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .expect("create project args");
        let create_project_args = create_project_args
            .as_object()
            .cloned()
            .expect("create project object");
        let project: CreateProjectOutput = serde_json::from_value(structured_json(
            router
                .call_tool("create_project", Some(&create_project_args))
                .await
                .expect("create project"),
        ))
        .expect("project output");

        let save_args = serde_json::to_value(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "A grey tower loomed. MOCK_CANON_MINE stood at its gate.".to_string(),
            summary: "s".to_string(),
            content_rating: ContentRating::General,
            tone: None,
            source_path: None,
            generation_id: None,
            ..Default::default()
        })
        .expect("save args");
        let save_args = save_args.as_object().cloned().expect("save args object");
        let saved: SaveSceneDraftOutput = serde_json::from_value(structured_json(
            router
                .call_tool("save_scene_draft", Some(&save_args))
                .await
                .expect("save scene draft"),
        ))
        .expect("save output");

        // Stage via mine.
        let mine_args = serde_json::to_value(MineSceneCanonInput {
            project_id: project.project_id.clone(),
            scene_id: saved.scene_id.clone(),
        })
        .expect("mine args");
        let mine_args = mine_args.as_object().cloned().expect("mine args object");
        router
            .call_tool("mine_scene_canon", Some(&mine_args))
            .await
            .expect("mine scene canon");

        // List the ratify queue.
        let list_args = serde_json::to_value(ListCanonDeltasInput {
            project_id: project.project_id.clone(),
            status: Some("staged".to_string()),
            scene_id: Some(saved.scene_id.clone()),
            chapter_range: None,
        })
        .expect("list args");
        let list_args = list_args.as_object().cloned().expect("list args object");
        let listed: ListCanonDeltasOutput = serde_json::from_value(structured_json(
            router
                .call_tool("list_canon_deltas", Some(&list_args))
                .await
                .expect("list canon deltas"),
        ))
        .expect("list output");
        assert_eq!(listed.deltas.len(), 1);
        let delta = &listed.deltas[0];
        assert_eq!(delta.delta_class, "canonical_fact");
        assert_eq!(delta.status, "staged");
        // Evidence is the verbatim sentinel from the prose.
        assert_eq!(delta.evidence, "MOCK_CANON_MINE");

        // Apply it.
        let decide_args = serde_json::to_value(DecideCanonDeltasInput {
            project_id: project.project_id.clone(),
            decisions: vec![CanonDeltaDecisionInput {
                delta_id: delta.id.clone(),
                action: "apply".to_string(),
                edit: None,
                note: Some("looks right".to_string()),
            }],
            decided_by: Some("mcp-operator".to_string()),
        })
        .expect("decide args");
        let decide_args = decide_args
            .as_object()
            .cloned()
            .expect("decide args object");
        let decided: DecideCanonDeltasOutput = serde_json::from_value(structured_json(
            router
                .call_tool("decide_canon_deltas", Some(&decide_args))
                .await
                .expect("decide canon deltas"),
        ))
        .expect("decide output");
        assert_eq!(decided.applied_count, 1);
        assert_eq!(decided.results[0].outcome, "applied");
        assert_eq!(decided.results[0].note.as_deref(), Some("looks right"));
        assert!(
            decided.results[0]
                .applied_record_id
                .as_deref()
                .is_some_and(|id| id.starts_with("canonical_fact:"))
        );

        // The row is now applied.
        let applied_list_args = serde_json::to_value(ListCanonDeltasInput {
            project_id: project.project_id.clone(),
            status: Some("applied".to_string()),
            scene_id: Some(saved.scene_id.clone()),
            chapter_range: None,
        })
        .expect("applied list args");
        let applied_list_args = applied_list_args
            .as_object()
            .cloned()
            .expect("applied list object");
        let applied: ListCanonDeltasOutput = serde_json::from_value(structured_json(
            router
                .call_tool("list_canon_deltas", Some(&applied_list_args))
                .await
                .expect("list applied"),
        ))
        .expect("applied output");
        assert_eq!(applied.deltas.len(), 1);
        assert_eq!(applied.deltas[0].status, "applied");
        assert_eq!(
            applied.deltas[0].decided_by.as_deref(),
            Some("mcp-operator")
        );
    }
}
