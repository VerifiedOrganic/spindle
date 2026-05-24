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
//! | `save_summary`, `register_canonical_fact` | Write |
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

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use rmcp::model::{CallToolResult, Content, Tool};
use rmcp::schemars;
use rmcp::schemars::generate::SchemaSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Number, Value};
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use spindle_core::models::*;
use spindle_core::subject_snapshot::SubjectSnapshot as EntitySubjectSnapshot;
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::json_utils::flatten_record_ids;

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

    pub fn list_tools(&self) -> Vec<Tool> {
        let all = self.all_tools();
        let Some(profile) = self.tool_profile.as_deref() else {
            return all;
        };
        let allowed: &[&str] = match profile {
            "import" => &[
                "create_project",
                "list_projects",
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
            ],
            "write" => &[
                "create_project",
                "list_projects",
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
                "batch_create_narrative_promises",
                "create_character_arc",
                "create_system_overlay",
                "preflight_book_export",
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
                "backfill_scene_source_offsets",
                "register_canonical_fact",
                "extract_canonical_facts_from_scene",
                "migrate_canonical_fact",
                "research_query",
                "pull_chapter_from_file",
                "push_chapter_to_file",
            ],
            "minimal" => &[
                "create_project",
                "list_projects",
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
            _ => return all,
        };
        all.into_iter()
            .filter(|t| allowed.contains(&t.name.as_ref()))
            .collect()
    }

    fn all_tools(&self) -> Vec<Tool> {
        vec![
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
                "Create the next chapter within a book using book_number or book_id; optional chapter_number is validated against the next sequential slot",
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
                "Update a supported entity by record id",
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
                "Advance a narrative promise lifecycle",
            ),
            tool::<CreateCharacterArcInput, CreateCharacterArcOutput>(
                "create_character_arc",
                "Create a character arc and pacing tracker",
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
                "Annotate structural beats for a scene",
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
                "Register a canonical story fact (pull result, stat change, item, ability) for contradiction detection",
            ),
            tool::<ExtractCanonicalFactsFromSceneInput, ExtractCanonicalFactsFromSceneOutput>(
                "extract_canonical_facts_from_scene",
                "Extract proposed typed canonical facts from committed scene prose without registering them",
            ),
            tool::<MigrateCanonicalFactInput, MigrateCanonicalFactOutput>(
                "migrate_canonical_fact",
                "Promote a legacy_untyped canonical fact into a typed canonical fact and supersede the legacy row",
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
            tool::<ExportEpubInput, ExportEpubOutput>(
                "export_epub",
                "Export a project, single book, or inclusive chapter range within a book as an EPUB file",
            ),
            tool::<PreflightBookExportInput, PreflightBookExportOutput>(
                "preflight_book_export",
                "Validate a project, single book, or inclusive chapter range within a book for EPUB export and return blocking issues or warnings before writing a file",
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
                "Delete an active-branch scene only when its dependency audit is clear; leaves a gap in scene_order",
            ),
            tool::<OperatorDeleteSceneInput, OperatorDeleteSceneOutput>(
                "operator_delete_scene",
                "Delete an active-branch scene after removing scene_source_link records and invalidating stale chapter_plan/chapter_summary artifacts, but only when no other blockers or semantic risks remain",
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
                "Append a character state snapshot from a saved scene",
            ),
            tool::<UpdateRelationshipInput, UpdateRelationshipOutput>(
                "update_relationship",
                "Update trust and tension for one directed relationship",
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
                "Record canonical knowledge for a character",
            ),
            tool::<RecordNoteInput, RecordNoteOutput>(
                "record_note",
                "Append a freeform note to branch session activity",
            ),
            tool::<UpdateWriterPositionInput, WriterPosition>(
                "update_writer_position",
                "Persist a branch writer cursor position without saving a draft",
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
        let _serialization_guard = match self
            .tool_serialization_scope(name, &resolved_arguments)
            .await
        {
            Ok(Some(scope)) => Some(self.lock_tool_scope(scope).await),
            Ok(None) => None,
            Err(error) => return Ok(structured_error_result(&error)),
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
            "configure_agents" => match parse_arguments::<ConfigureAgentsInput>(arguments) {
                Ok(input) => match self.service.configure_agents(input) {
                    Ok(output) => structured_result(&output),
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            "list_agents" => structured_result(&self.service.list_agents()),
            "init_grok_skills" => match parse_arguments::<InitGrokSkillsInput>(arguments) {
                Ok(input) => {
                    let output = self.handle_init_grok_skills(input)?;
                    structured_result(&output)
                }
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
            "export_epub" => {
                self.invoke(arguments, |input| self.service.export_epub(input))
                    .await
            }
            "preflight_book_export" => {
                self.invoke(arguments, |input| self.service.preflight_book_export(input))
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
            | "list_agents"
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
            | "set_active_project"
            | "import_manuscript"
            | "list_agents"
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
    let object = value
        .as_object()
        .cloned()
        .unwrap_or_else(|| panic!("expected object schema for tool input"));

    Tool::new(name, description, object).with_output_schema::<O>()
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

    #[test]
    fn session_serialization_is_enabled_for_mutating_tools() {
        assert!(tool_requires_session_serialization("save_scene_draft"));
        assert!(tool_requires_session_serialization("commit_scene_changes"));
        assert!(!tool_requires_session_serialization("get_writer_state"));
        assert!(!tool_requires_session_serialization("get_scene_context"));
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
}
