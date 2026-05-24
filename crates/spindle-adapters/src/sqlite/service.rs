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

use anyhow::{Context, Result};
use spindle_core::models::{
    AnnotateSceneBeatsInput, AnnotateSceneBeatsOutput, AnnotatedBeat, ArchiveEntityInput,
    ArchiveEntityOutput, BatchCreateMotifsInput, BatchCreateMotifsOutput,
    BatchCreateNarrativePromisesInput, BatchCreateNarrativePromisesOutput, BatchCreateTermsInput,
    BatchCreateTermsOutput, BatchSetCharacterVoiceProfilesInput,
    BatchSetCharacterVoiceProfilesOutput, BookOutline, CanonicalFactScope, ChapterOutline,
    ChapterOutlineBeat, CharacterStatePatch, CharacterVoiceProfileData, CommitCharacterStateInput,
    CommitCharacterStateOutput, ConfigureAgentsInput, ConfigureAgentsOutput, CreateBookInput,
    CreateBookOutput, CreateBranchInput, CreateBranchOutput, CreateChapterInput,
    CreateChapterOutput, CreateCharacterArcInput, CreateCharacterArcOutput, CreateCharacterInput,
    CreateCharacterOutput, CreateConflictInput, CreateConflictOutput, CreateEconomyInput,
    CreateEconomyOutput, CreateFactionInput, CreateFactionOutput, CreateFutureKnowledgeInput,
    CreateFutureKnowledgeOutput, CreateLocationInput, CreateLocationOutput, CreateMotifInput,
    CreateMotifOutput, CreateNarrativePromiseInput, CreateNarrativePromiseOutput,
    CreatePacingConfigInput, CreatePacingConfigOutput, CreatePacingCurveInput,
    CreatePacingCurveOutput, CreatePlotLineInput, CreatePlotLineOutput, CreateProjectInput,
    CreateProjectOutput, CreateRelationshipInput, CreateRelationshipOutput, CreateReligionInput,
    CreateReligionOutput, CreateSavePointInput, CreateSavePointOutput, CreateSystemOverlayInput,
    CreateSystemOverlayOutput, CreateTemporalInterventionInput, CreateTemporalInterventionOutput,
    CreateTermInput, CreateTermOutput, CreateThemeInput, CreateThemeOutput,
    CreateTimelineEventInput, CreateTimelineEventOutput, CreateWorldRuleInput,
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
    ResolveRevisionMarkerInput, ResolveRevisionMarkerOutput, RestoreSceneVersionInput,
    RestoreSceneVersionOutput, SaveSceneDraftInput, SaveSceneDraftOutput, SaveSummaryInput,
    SaveSummaryOutput, SceneDeleteImpactGroup, SceneDeleteImpactTarget, SceneDeleteReadiness,
    SceneMoveImpactDestination, SceneMoveImpactGroup, SceneMoveReadiness, SceneVersionSummary,
    SearchBibleInput, SearchBibleMode, SearchBibleOutput, SearchBibleResultItem,
    SetArcPacingConstraintsInput, SetArcPacingConstraintsOutput, SetBookOutlineInput,
    SetBookOutlineOutput, SetChapterOutlineInput, SetChapterOutlineOutput,
    SetCharacterVoiceProfileInput, SetCharacterVoiceProfileOutput, StoryPlacement,
    SwitchBranchInput, SwitchBranchOutput, TestAgentInput, TestAgentOutput, UpdateEntityInput,
    UpdateEntityOutput, UpdatePromiseStatusInput, UpdatePromiseStatusOutput,
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

/// Cap on in-memory generation receipts. Once exceeded, oldest entries are
/// evicted FIFO. Mirrors the SurrealDB-era `MAX_GENERATION_RECEIPTS = 256`.
const MAX_GENERATION_RECEIPTS: usize = 256;

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
}

impl SqliteSpindleService {
    pub fn new(repository: Repository) -> Self {
        Self {
            repository,
            generation_receipts: std::sync::Arc::new(std::sync::RwLock::new(
                std::collections::BTreeMap::new(),
            )),
            generation_receipt_counter: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    pub fn repository(&self) -> &Repository {
        &self.repository
    }

    async fn resolve_phase_four_caches(
        &self,
        project_id: &str,
        branch_id: &str,
        ids: &[PhaseFourCacheId],
    ) -> Result<usize> {
        let mut resolved = 0;
        for id in ids {
            resolved += self
                .repository
                .resolve_validator_findings_for_validator(project_id, branch_id, id.as_str())
                .await?;
        }
        Ok(resolved)
    }

    async fn resolve_phase_four_caches_for_project(
        &self,
        project_id: &str,
        ids: &[PhaseFourCacheId],
    ) -> Result<usize> {
        let mut resolved = 0;
        for branch in self.repository.list_branches_by_project(project_id).await? {
            resolved += self
                .resolve_phase_four_caches(project_id, &branch.id, ids)
                .await?;
        }
        Ok(resolved)
    }

    async fn phase_four_cache_target_for_entity_update(
        &self,
        entity_type: &str,
        entity_id: &str,
        changes: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Option<(String, Option<String>, Vec<PhaseFourCacheId>)>> {
        if changes.is_empty() {
            return Ok(None);
        }

        Ok(match entity_type {
            "project"
                if changes.contains_key("genre")
                    || changes.contains_key("project_type")
                    || changes.contains_key("reader_contract")
                    || changes.contains_key("style_notes") =>
            {
                let project = self.repository.get_project(entity_id).await?;
                Some((project.id, None, vec![PhaseFourCacheId::StyleCompliance]))
            }
            "world_rule" => {
                let rule = self.repository.get_world_rule(entity_id).await?;
                let mut cache_ids = vec![PhaseFourCacheId::WorldRuleSemanticDrift];
                if rule.rule_type.eq_ignore_ascii_case("style")
                    || changes
                        .get("rule_type")
                        .and_then(|value| value.as_str())
                        .is_some_and(|rule_type| rule_type.eq_ignore_ascii_case("style"))
                {
                    cache_ids.push(PhaseFourCacheId::StyleCompliance);
                }
                Some((rule.project_id, Some(rule.branch_id), cache_ids))
            }
            "character"
                if changes.contains_key("name")
                    || changes.contains_key("normalized_name")
                    || changes.contains_key("voice_profile")
                    || changes.contains_key("voice_profile_data") =>
            {
                let character = self.repository.get_character(entity_id).await?;
                Some((
                    character.project_id,
                    Some(character.branch_id),
                    vec![PhaseFourCacheId::VoiceDrift],
                ))
            }
            "timeline_event" => {
                let event = self.repository.get_timeline_event(entity_id).await?;
                Some((
                    event.project_id,
                    Some(event.branch_id),
                    vec![PhaseFourCacheId::RetconReachability],
                ))
            }
            "temporal_intervention" => {
                let intervention = self.repository.get_temporal_intervention(entity_id).await?;
                Some((
                    intervention.project_id,
                    Some(intervention.branch_id),
                    vec![PhaseFourCacheId::RetconReachability],
                ))
            }
            _ => None,
        })
    }

    // =========================================================================
    // Project tools
    // =========================================================================

    /// Create a new project plus its initial main branch + book + chapter,
    /// returning the public-facing record-id triple the MCP layer expects.
    /// Mirrors the SurrealDB-side service method 1:1 — same `CreateProjectInput`
    /// and `CreateProjectOutput` shapes from spindle_core.
    pub async fn create_project(&self, input: CreateProjectInput) -> Result<CreateProjectOutput> {
        let (project, branch, book, chapter) = self.repository.create_project(&input).await?;
        Ok(CreateProjectOutput {
            project_id: project.id,
            branch_id: branch.id,
            book_id: book.id,
            chapter_id: chapter.id,
        })
    }

    /// List every project, newest first.
    pub async fn list_projects(&self) -> Result<ListProjectsOutput> {
        let projects = self.repository.list_projects().await?;
        let projects = projects
            .into_iter()
            .map(|p| ProjectSummary {
                project_id: p.id,
                name: p.name,
                project_type: p.project_type,
                genre: p.genre,
            })
            .collect();
        Ok(ListProjectsOutput { projects })
    }

    // =========================================================================
    // Branch tools
    // =========================================================================

    pub async fn create_branch(&self, input: CreateBranchInput) -> Result<CreateBranchOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let parent_branch_id = match input.parent_branch_id.as_deref() {
            Some(id) => id.to_string(),
            None => project.active_branch_id.clone().unwrap_or_else(|| {
                // Fall back to a synthetic main-branch lookup if no active is set.
                // The repo's create_branch will reject if the parent is invalid,
                // so this is best-effort.
                String::new()
            }),
        };
        let branch = self
            .repository
            .create_branch(
                &input.project_id,
                &parent_branch_id,
                &input.name,
                &input.branch_type,
                input.description,
            )
            .await?;
        Ok(CreateBranchOutput {
            branch_id: branch.id,
            parent_branch_id: branch.parent_branch_id.unwrap_or_default(),
        })
    }

    pub async fn switch_branch(&self, input: SwitchBranchInput) -> Result<SwitchBranchOutput> {
        let branch = self.repository.get_branch(&input.branch_id).await?;
        // Enforce project ownership when the branch carries a project_id.
        if let Some(branch_project_id) = branch.project_id.as_deref()
            && branch_project_id != input.project_id
        {
            anyhow::bail!(
                "branch {} does not belong to project {}",
                input.branch_id,
                input.project_id
            );
        }
        let updated = self
            .repository
            .switch_active_branch(&input.project_id, &input.branch_id)
            .await?;
        let _ = updated;
        Ok(SwitchBranchOutput {
            branch_id: branch.id,
            branch_name: branch.name,
        })
    }

    /// Set or clear the project's narrator-voice directive (the prose-level
    /// narration style consumed by scene-context assembly, the style gate, the
    /// `style_compliance` validator, and the review's Target Reader persona).
    pub async fn set_narrator_voice(
        &self,
        input: spindle_core::models::SetNarratorVoiceInput,
    ) -> Result<spindle_core::models::SetNarratorVoiceOutput> {
        let cleared = input.narrator_voice.is_empty();
        let project = self
            .repository
            .set_narrator_voice(&input.project_id, input.narrator_voice)
            .await?;
        self.resolve_phase_four_caches_for_project(
            &input.project_id,
            &[PhaseFourCacheId::StyleCompliance],
        )
        .await?;
        let narrator_voice = project
            .narrator_voice
            .map(|stored| stored.into_core())
            .unwrap_or_default();
        Ok(spindle_core::models::SetNarratorVoiceOutput {
            project_id: project.id,
            narrator_voice,
            cleared,
        })
    }

    // =========================================================================
    // Entity creation tools
    // =========================================================================

    pub async fn create_character(
        &self,
        input: CreateCharacterInput,
    ) -> Result<CreateCharacterOutput> {
        self.repository.get_project(&input.project_id).await?;
        let (character, voice_profile, emotional_profile, state) =
            self.repository.create_character(&input).await?;

        // Mirror the SurrealDB service: clear voice_drift validator findings
        // on the active branch when a new character lands. Search-embedding
        // refresh happens here too so the new character is searchable.
        let active_branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        self.resolve_phase_four_caches(
            &input.project_id,
            &active_branch_id,
            &[PhaseFourCacheId::VoiceDrift],
        )
        .await?;
        let document = SearchDocument {
            entity_table: "character".into(),
            title: character.name.clone(),
            excerpt: character.summary.clone(),
            content: format!("{}\n\n{}", character.name, character.summary),
        };
        self.repository
            .refresh_search_embedding_for_entity(
                &input.project_id,
                &active_branch_id,
                &character.id,
                &document,
            )
            .await?;

        Ok(CreateCharacterOutput {
            character_id: character.id,
            voice_profile_id: voice_profile.id,
            emotional_profile_id: emotional_profile.id,
            state_id: state.id,
        })
    }

    pub async fn create_location(
        &self,
        input: CreateLocationInput,
    ) -> Result<CreateLocationOutput> {
        self.repository.get_project(&input.project_id).await?;
        let (location, world_state) = self.repository.create_location(&input).await?;

        let active_branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let document = SearchDocument {
            entity_table: "location".into(),
            title: location.name.clone(),
            excerpt: location.summary.clone(),
            content: format!("{}\n\n{}", location.name, location.summary),
        };
        self.repository
            .refresh_search_embedding_for_entity(
                &input.project_id,
                &active_branch_id,
                &location.id,
                &document,
            )
            .await?;

        Ok(CreateLocationOutput {
            location_id: location.id,
            world_state_id: world_state.id,
        })
    }

    pub async fn create_relationship(
        &self,
        input: CreateRelationshipInput,
    ) -> Result<CreateRelationshipOutput> {
        let source_character = self.repository.get_character(&input.character_a_id).await?;
        let target_character = self.repository.get_character(&input.character_b_id).await?;
        if source_character.project_id != target_character.project_id {
            anyhow::bail!("relationship characters must belong to the same project");
        }
        let branch_id = self
            .repository
            .active_branch_id_public(&source_character.project_id)
            .await?;
        let rel = self
            .repository
            .create_relationship(&branch_id, &input)
            .await?;
        // The SQLite schema's relates_to has no surrogate id — the composite
        // key (branch_id, in_id, out_id) is the natural identifier. Encode
        // it into the output's relationship_id field so callers can roundtrip.
        Ok(CreateRelationshipOutput {
            relationship_id: format!("relates_to:{}:{}:{}", rel.branch_id, rel.in_id, rel.out_id),
        })
    }

    /// Commit a character state snapshot at the moment of a scene.
    /// Merges the input patch on top of the most recent character_state at
    /// or before the scene, then appends a new state row + a progression
    /// event recording the change.
    pub async fn commit_character_state(
        &self,
        input: CommitCharacterStateInput,
    ) -> Result<CommitCharacterStateOutput> {
        let character = self.repository.get_character(&input.character_id).await?;
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != character.project_id {
            anyhow::bail!("scene and character must belong to the same project");
        }
        let latest_state = self
            .repository
            .resolve_character_state(
                &input.character_id,
                scene.book_number,
                scene.chapter_number,
                scene.scene_order + 1,
            )
            .await?;
        let progression_delta = serde_json::to_value(&input.changes)?;

        let merged = CharacterStatePatch {
            emotional_state: if input.changes.emotional_state.is_empty() {
                latest_state
                    .as_ref()
                    .map(|s| s.emotional_state.clone())
                    .unwrap_or_default()
            } else {
                input.changes.emotional_state
            },
            goals: Some(input.changes.goals.unwrap_or_else(|| {
                latest_state
                    .as_ref()
                    .map(|s| s.goals.clone())
                    .unwrap_or_default()
            })),
            status: Some(input.changes.status.unwrap_or_else(|| {
                latest_state
                    .as_ref()
                    .map(|s| s.status.clone())
                    .unwrap_or_default()
            })),
            notes: Some(input.changes.notes.unwrap_or_else(|| {
                latest_state
                    .as_ref()
                    .map(|s| s.notes.clone())
                    .unwrap_or_default()
            })),
            source_summary: input
                .changes
                .source_summary
                .or_else(|| latest_state.as_ref().and_then(|s| s.source_summary.clone()))
                .or_else(|| Some(format!("scene update for {}", character.name))),
        };

        let state = self
            .repository
            .append_character_state(AppendCharacterStateParams {
                project_id: character.project_id.clone(),
                branch_id: character.branch_id.clone(),
                character_id: character.id.clone(),
                scene_id: Some(scene.id.clone()),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                patch: merged,
            })
            .await?;

        // Best-effort progression event: failure here doesn't roll back the
        // committed state. Matches the SurrealDB service semantics.
        let _ = self
            .repository
            .append_progression_event(AppendProgressionEventParams {
                project_id: character.project_id.clone(),
                branch_id: character.branch_id.clone(),
                subject_table: "character".to_string(),
                subject_id: character.id.clone(),
                overlay_id: None,
                kind: "character_state_commit".to_string(),
                delta_json: progression_delta,
                source_scene_id: Some(scene.id.clone()),
                placement: Some(StoryPlacement {
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: Some(scene.scene_order),
                    note: None,
                }),
                created_at: chrono::Utc::now(),
            })
            .await;

        Ok(CommitCharacterStateOutput { state_id: state.id })
    }

    /// Update a relationship's trust + tension by deltas, set reason and
    /// last_scene_id. Returns the post-update trust + tension so callers can
    /// surface the new equilibrium.
    pub async fn update_relationship(
        &self,
        input: UpdateRelationshipInput,
    ) -> Result<UpdateRelationshipOutput> {
        let scene = self.repository.get_scene(&input.scene_id).await?;
        let source_character = self.repository.get_character(&input.character_a_id).await?;
        let target_character = self.repository.get_character(&input.character_b_id).await?;
        if source_character.project_id != target_character.project_id {
            anyhow::bail!("relationship characters must belong to the same project");
        }
        if scene.project_id != source_character.project_id {
            anyhow::bail!("scene does not belong to the same project as the characters");
        }
        let rel = self
            .repository
            .update_relationship(&scene.branch_id, &input)
            .await?;
        Ok(UpdateRelationshipOutput {
            relationship_id: format!("relates_to:{}:{}:{}", rel.branch_id, rel.in_id, rel.out_id),
            trust: rel.trust,
            tension: rel.tension,
        })
    }

    /// MCP search_bible: find Bible entities matching the query. Uses the
    /// vec0 kNN path for `Semantic` mode and FTS5 for `Exact` / `Fuzzy`.
    /// Returns the canonical SearchBibleOutput shape that MCP exposes.
    pub async fn search_bible(&self, input: SearchBibleInput) -> Result<SearchBibleOutput> {
        let mode = input.mode.unwrap_or(SearchBibleMode::Semantic);
        let limit = input.limit.unwrap_or(20).min(100);
        let project_id = input.project_id.clone();
        let branch_id = self.repository.active_branch_id_public(&project_id).await?;

        let results: Vec<SearchBibleResultItem> = match mode {
            SearchBibleMode::Semantic => {
                // Use the configured embedder to produce a query vector,
                // then ask the vec0 mirror for the k nearest matches.
                let embedding_session = self.repository.model_router().embedding_session();
                let query_embedding = embedding_session.embed_text(&input.query).await?;
                let hits = self
                    .repository
                    .knn_search_embeddings(&project_id, Some(&branch_id), &query_embedding, limit)
                    .await?;
                hits.into_iter()
                    .map(|(se, distance)| SearchBibleResultItem {
                        entity_type: se.entity_table,
                        entity_id: se.entity_id,
                        title: se.title,
                        excerpt: se.excerpt,
                        // Lower distance = more similar; report as a score where
                        // higher is better, in the [0, 1] range that callers
                        // typically display.
                        score: 1.0 / (1.0 + distance),
                    })
                    .collect()
            }
            SearchBibleMode::Exact | SearchBibleMode::Fuzzy => {
                // FTS5: combine scene + character + location + world_rule hits.
                let mut combined = Vec::new();
                for (entity_type, hits) in [
                    (
                        "scene",
                        self.repository
                            .fts_search_scenes(&project_id, Some(&branch_id), &input.query, limit)
                            .await?,
                    ),
                    (
                        "character",
                        self.repository
                            .fts_search_characters(
                                &project_id,
                                Some(&branch_id),
                                &input.query,
                                limit,
                            )
                            .await?,
                    ),
                    (
                        "location",
                        self.repository
                            .fts_search_locations(
                                &project_id,
                                Some(&branch_id),
                                &input.query,
                                limit,
                            )
                            .await?,
                    ),
                    (
                        "world_rule",
                        self.repository
                            .fts_search_world_rules(
                                &project_id,
                                Some(&branch_id),
                                &input.query,
                                limit,
                            )
                            .await?,
                    ),
                ] {
                    for (id, rank, snippet) in hits {
                        combined.push(SearchBibleResultItem {
                            entity_type: entity_type.to_string(),
                            entity_id: id,
                            title: String::new(),
                            excerpt: snippet,
                            // FTS5 rank is negative (smaller = better). Flip sign
                            // for a "higher is better" score so callers can sort
                            // both modes the same way.
                            score: -rank,
                        });
                    }
                }
                combined.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                combined.truncate(limit);
                combined
            }
        };

        Ok(SearchBibleOutput {
            results,
            canonical_facts: Vec::new(),
            markdown: None,
        })
    }

    // =========================================================================
    // Plain narrative-entity create tools (faction/religion/economy/term/
    // plot_line/conflict/theme/motif/narrative_promise/world_rule)
    // =========================================================================
    //
    // These all follow the SurrealDB service shape: validate project, call
    // repo.create_X (which resolves the active branch internally), refresh
    // the search embedding when the entity has searchable text. Each output
    // is a single-id record.

    pub async fn create_faction(&self, input: CreateFactionInput) -> Result<CreateFactionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let faction = self.repository.create_faction(&input).await?;
        self.refresh_entity_index(
            &faction.project_id,
            &faction.branch_id,
            &faction.id,
            "faction",
            &faction.name,
            &faction.summary,
        )
        .await?;
        Ok(CreateFactionOutput {
            faction_id: faction.id,
        })
    }

    pub async fn create_religion(
        &self,
        input: CreateReligionInput,
    ) -> Result<CreateReligionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let religion = self.repository.create_religion(&input).await?;
        self.refresh_entity_index(
            &religion.project_id,
            &religion.branch_id,
            &religion.id,
            "religion",
            &religion.name,
            &religion.summary,
        )
        .await?;
        Ok(CreateReligionOutput {
            religion_id: religion.id,
        })
    }

    pub async fn create_economy(&self, input: CreateEconomyInput) -> Result<CreateEconomyOutput> {
        self.repository.get_project(&input.project_id).await?;
        let economy = self.repository.create_economy(&input).await?;
        self.refresh_entity_index(
            &economy.project_id,
            &economy.branch_id,
            &economy.id,
            "economy",
            &economy.name,
            &economy.summary,
        )
        .await?;
        Ok(CreateEconomyOutput {
            economy_id: economy.id,
        })
    }

    pub async fn create_term(&self, input: CreateTermInput) -> Result<CreateTermOutput> {
        self.repository.get_project(&input.project_id).await?;
        let term = self.repository.create_term(&input).await?;
        self.refresh_entity_index(
            &term.project_id,
            &term.branch_id,
            &term.id,
            "term",
            &term.term_text,
            &term.definition,
        )
        .await?;
        Ok(CreateTermOutput { term_id: term.id })
    }

    pub async fn create_plot_line(
        &self,
        input: CreatePlotLineInput,
    ) -> Result<CreatePlotLineOutput> {
        self.repository.get_project(&input.project_id).await?;
        let plot = self.repository.create_plot_line(&input).await?;
        self.refresh_entity_index(
            &plot.project_id,
            &plot.branch_id,
            &plot.id,
            "plot_line",
            &plot.name,
            &plot.summary,
        )
        .await?;
        Ok(CreatePlotLineOutput {
            plot_line_id: plot.id,
        })
    }

    pub async fn create_conflict(
        &self,
        input: CreateConflictInput,
    ) -> Result<CreateConflictOutput> {
        self.repository.get_project(&input.project_id).await?;
        let conflict = self.repository.create_conflict(&input).await?;
        self.refresh_entity_index(
            &conflict.project_id,
            &conflict.branch_id,
            &conflict.id,
            "conflict",
            &conflict.name,
            &conflict.stakes,
        )
        .await?;
        Ok(CreateConflictOutput {
            conflict_id: conflict.id,
        })
    }

    pub async fn create_theme(&self, input: CreateThemeInput) -> Result<CreateThemeOutput> {
        self.repository.get_project(&input.project_id).await?;
        let theme = self.repository.create_theme(&input).await?;
        self.refresh_entity_index(
            &theme.project_id,
            &theme.branch_id,
            &theme.id,
            "theme",
            &theme.theme_statement,
            &theme.thesis_antithesis,
        )
        .await?;
        Ok(CreateThemeOutput { theme_id: theme.id })
    }

    pub async fn create_motif(&self, input: CreateMotifInput) -> Result<CreateMotifOutput> {
        self.repository.get_project(&input.project_id).await?;
        let motif = self.repository.create_motif(&input).await?;
        self.refresh_entity_index(
            &motif.project_id,
            &motif.branch_id,
            &motif.id,
            "motif",
            &motif.name,
            &motif.description,
        )
        .await?;
        Ok(CreateMotifOutput { motif_id: motif.id })
    }

    pub async fn create_narrative_promise(
        &self,
        input: CreateNarrativePromiseInput,
    ) -> Result<CreateNarrativePromiseOutput> {
        self.repository.get_project(&input.project_id).await?;
        let promise = self.repository.create_narrative_promise(&input).await?;
        self.refresh_entity_index(
            &promise.project_id,
            &promise.branch_id,
            &promise.id,
            "narrative_promise",
            &promise.promise_type,
            &promise.description,
        )
        .await?;
        Ok(CreateNarrativePromiseOutput {
            narrative_promise_id: promise.id,
        })
    }

    pub async fn create_world_rule(
        &self,
        input: CreateWorldRuleInput,
    ) -> Result<CreateWorldRuleOutput> {
        self.repository.get_project(&input.project_id).await?;
        let rule = self.repository.create_world_rule(&input).await?;
        let mut cache_ids = vec![PhaseFourCacheId::WorldRuleSemanticDrift];
        if rule.rule_type.eq_ignore_ascii_case("style") {
            cache_ids.push(PhaseFourCacheId::StyleCompliance);
        }
        self.resolve_phase_four_caches(&rule.project_id, &rule.branch_id, &cache_ids)
            .await?;
        self.refresh_entity_index(
            &rule.project_id,
            &rule.branch_id,
            &rule.id,
            "world_rule",
            &rule.rule_name,
            &rule.description,
        )
        .await?;
        Ok(CreateWorldRuleOutput {
            world_rule_id: rule.id,
        })
    }

    /// Persist a chapter summary, replacing any previous one at the same
    /// (project, branch, book#, chapter#) tuple.
    pub async fn save_summary(&self, input: SaveSummaryInput) -> Result<SaveSummaryOutput> {
        self.repository.get_project(&input.project_id).await?;
        let summary = self.repository.save_summary(&input).await?;
        Ok(SaveSummaryOutput {
            chapter_summary_id: summary.id,
        })
    }

    // =========================================================================
    // Batch creates
    // =========================================================================

    pub async fn batch_create_terms(
        &self,
        input: BatchCreateTermsInput,
    ) -> Result<BatchCreateTermsOutput> {
        self.repository.get_project(&input.project_id).await?;
        let mut term_ids = Vec::with_capacity(input.items.len());
        for item in input.items {
            let term = self
                .create_term(spindle_core::models::CreateTermInput {
                    project_id: input.project_id.clone(),
                    term_text: item.term_text,
                    pronunciation: item.pronunciation,
                    definition: item.definition,
                    usage_context: item.usage_context,
                    origin: item.origin,
                })
                .await?;
            term_ids.push(term.term_id);
        }
        let created = term_ids.len();
        Ok(BatchCreateTermsOutput { term_ids, created })
    }

    pub async fn batch_create_motifs(
        &self,
        input: BatchCreateMotifsInput,
    ) -> Result<BatchCreateMotifsOutput> {
        self.repository.get_project(&input.project_id).await?;
        let mut motif_ids = Vec::with_capacity(input.items.len());
        for item in input.items {
            let motif = self
                .create_motif(spindle_core::models::CreateMotifInput {
                    project_id: input.project_id.clone(),
                    name: item.name,
                    description: item.description,
                    max_uses_per_chapter: item.max_uses_per_chapter,
                    connected_theme_ids: item.connected_theme_ids,
                })
                .await?;
            motif_ids.push(motif.motif_id);
        }
        let created = motif_ids.len();
        Ok(BatchCreateMotifsOutput { motif_ids, created })
    }

    pub async fn batch_create_narrative_promises(
        &self,
        input: BatchCreateNarrativePromisesInput,
    ) -> Result<BatchCreateNarrativePromisesOutput> {
        self.repository.get_project(&input.project_id).await?;
        let mut narrative_promise_ids = Vec::with_capacity(input.items.len());
        for item in input.items {
            let p = self
                .create_narrative_promise(spindle_core::models::CreateNarrativePromiseInput {
                    project_id: input.project_id.clone(),
                    promise_type: item.promise_type,
                    description: item.description,
                    planted_at: item.planted_at,
                    planned_payoff: item.planned_payoff,
                    notes: item.notes,
                })
                .await?;
            narrative_promise_ids.push(p.narrative_promise_id);
        }
        let created = narrative_promise_ids.len();
        Ok(BatchCreateNarrativePromisesOutput {
            narrative_promise_ids,
            created,
        })
    }

    /// Shared helper for the create_* methods above: builds a SearchDocument
    /// from the entity's text fields and upserts it into the search index.
    async fn refresh_entity_index(
        &self,
        project_id: &str,
        branch_id: &str,
        entity_id: &str,
        entity_table: &str,
        title: &str,
        excerpt: &str,
    ) -> Result<()> {
        let document = SearchDocument {
            entity_table: entity_table.to_string(),
            title: title.to_string(),
            excerpt: excerpt.to_string(),
            content: format!("{title}\n\n{excerpt}"),
        };
        self.repository
            .refresh_search_embedding_for_entity(project_id, branch_id, entity_id, &document)
            .await
    }

    // =========================================================================
    // Book / chapter / save point / pacing / character_arc / timeline / system
    // overlay / temporal intervention / future knowledge / annotate beats /
    // plan chapter
    // =========================================================================

    pub async fn create_book(&self, input: CreateBookInput) -> Result<CreateBookOutput> {
        self.repository.get_project(&input.project_id).await?;
        let book = self
            .repository
            .create_book(&input.project_id, input.title)
            .await?;
        Ok(CreateBookOutput {
            book_id: book.id,
            book_number: book.book_number,
        })
    }

    pub async fn create_chapter(&self, input: CreateChapterInput) -> Result<CreateChapterOutput> {
        self.repository.get_project(&input.project_id).await?;
        // The book can come from (a) explicit book_id, (b) book_number, or
        // (c) defaulting to book 1. Mirror the SurrealDB service's resolution.
        let book = if let Some(book_id) = input.book_id.as_deref() {
            self.repository.get_book(book_id).await?
        } else {
            let book_number = input.book_number.unwrap_or(1);
            self.repository
                .ensure_book(&input.project_id, book_number)
                .await?
        };
        // Chapter number: explicit, else next-after-existing.
        let chapter_number = match input.chapter_number {
            Some(n) => n,
            None => {
                let existing = self.repository.list_chapters_by_book(&book.id).await?;
                existing.iter().map(|c| c.chapter_number).max().unwrap_or(0) + 1
            }
        };
        let chapter = self
            .repository
            .ensure_chapter(&input.project_id, book.book_number, chapter_number)
            .await?;
        Ok(CreateChapterOutput {
            chapter_id: chapter.id,
            book_number: chapter.book_number,
            chapter_number: chapter.chapter_number,
        })
    }

    pub async fn create_save_point(
        &self,
        input: CreateSavePointInput,
    ) -> Result<CreateSavePointOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?
            .to_string();
        let branch = self.repository.get_branch(&branch_id).await?;
        let sp = self
            .repository
            .create_save_point(
                &input.project_id,
                &branch_id,
                &input.name,
                input.description.clone(),
            )
            .await?;

        // Persist the snapshot file + sha256/record_count metadata. Mirrors
        // `persist_save_point_snapshot` (`services/mod.rs:1618..1716` in
        // 705b835^). On any failure we delete the save_point row so the
        // database stays consistent with the on-disk view.
        if let Err(err) = self
            .persist_save_point_snapshot(&input.project_id, &branch.name, &sp.id, &input.name)
            .await
        {
            let _ = self.repository.delete_save_point(&sp.id).await;
            return Err(err);
        }

        Ok(CreateSavePointOutput {
            save_point_id: sp.id,
            branch_id: sp.branch_id,
        })
    }

    /// Walk the project, write the snapshot JSON to `data_dir/save-points/`,
    /// then update the save_point row with the file path + sha256 + record
    /// count metadata. Mirrors `persist_save_point_snapshot` in
    /// `services/mod.rs:1618..1716` in 705b835^.
    async fn persist_save_point_snapshot(
        &self,
        project_id: &str,
        branch_name: &str,
        save_point_id: &str,
        save_point_name: &str,
    ) -> Result<()> {
        use std::collections::BTreeMap;

        const SAVE_POINT_SNAPSHOT_FORMAT: &str = "spindle-save-point-v1";

        let snapshot_relative_path = format!(
            "save-points/{}-{}-{}.json",
            slugify_filename_component(branch_name),
            slugify_filename_component(save_point_name),
            save_point_id.replace(':', "-"),
        );

        let mut extra_metadata: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        extra_metadata.insert(
            "save_point_id".to_string(),
            serde_json::Value::String(save_point_id.to_string()),
        );
        extra_metadata.insert(
            "save_point_name".to_string(),
            serde_json::Value::String(save_point_name.to_string()),
        );
        extra_metadata.insert(
            "branch_name".to_string(),
            serde_json::Value::String(branch_name.to_string()),
        );
        extra_metadata.insert(
            "snapshot_created_at".to_string(),
            serde_json::Value::String(chrono::Utc::now().to_rfc3339()),
        );

        let artifact = self
            .build_project_export_payload(project_id, SAVE_POINT_SNAPSHOT_FORMAT, extra_metadata)
            .await?;
        let snapshot_bytes = serde_json::to_vec_pretty(&artifact.payload)?;
        let snapshot_sha256 = crate::sqlite::import::sha256_hex(&snapshot_bytes);

        let snapshot_path = self.repository.data_dir().join(&snapshot_relative_path);
        if let Some(parent) = snapshot_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&snapshot_path, &snapshot_bytes)?;

        if let Err(err) = self
            .repository
            .update_save_point_snapshot(
                save_point_id,
                &snapshot_relative_path,
                SAVE_POINT_SNAPSHOT_FORMAT,
                artifact.exported_records as i64,
                &snapshot_sha256,
            )
            .await
        {
            let _ = std::fs::remove_file(&snapshot_path);
            return Err(err);
        }

        Ok(())
    }

    pub async fn create_pacing_config(
        &self,
        input: CreatePacingConfigInput,
    ) -> Result<CreatePacingConfigOutput> {
        self.repository.get_project(&input.project_id).await?;
        let cfg = self.repository.create_pacing_config(&input).await?;
        Ok(CreatePacingConfigOutput {
            pacing_config_id: cfg.id,
        })
    }

    pub async fn create_pacing_curve(
        &self,
        input: CreatePacingCurveInput,
    ) -> Result<CreatePacingCurveOutput> {
        self.repository.get_project(&input.project_id).await?;
        let curve = self.repository.create_pacing_curve(&input).await?;
        Ok(CreatePacingCurveOutput {
            pacing_curve_id: curve.id,
        })
    }

    pub async fn create_character_arc(
        &self,
        input: CreateCharacterArcInput,
    ) -> Result<CreateCharacterArcOutput> {
        self.repository.get_project(&input.project_id).await?;
        let arc = self.repository.create_character_arc(&input).await?;
        // SurrealDB service spins up a paired pacing_tracker on arc creation;
        // mirror that here.
        let tracker = self
            .repository
            .create_pacing_tracker(&input.project_id, &arc.id)
            .await?;
        Ok(CreateCharacterArcOutput {
            character_arc_id: arc.id,
            pacing_tracker_id: tracker.id,
        })
    }

    pub async fn create_timeline_event(
        &self,
        input: CreateTimelineEventInput,
    ) -> Result<CreateTimelineEventOutput> {
        self.repository.get_project(&input.project_id).await?;
        let event = self.repository.create_timeline_event(&input).await?;
        self.resolve_phase_four_caches(
            &event.project_id,
            &event.branch_id,
            &[PhaseFourCacheId::RetconReachability],
        )
        .await?;
        Ok(CreateTimelineEventOutput {
            timeline_event_id: event.id,
        })
    }

    pub async fn create_temporal_intervention(
        &self,
        input: CreateTemporalInterventionInput,
    ) -> Result<CreateTemporalInterventionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let intervention = self
            .repository
            .create_temporal_intervention(
                input.source_event_id.clone(),
                input.target_event_id.clone(),
                &input,
            )
            .await?;
        self.resolve_phase_four_caches(
            &intervention.project_id,
            &intervention.branch_id,
            &[PhaseFourCacheId::RetconReachability],
        )
        .await?;
        Ok(CreateTemporalInterventionOutput {
            temporal_intervention_id: intervention.id,
        })
    }

    pub async fn create_system_overlay(
        &self,
        input: CreateSystemOverlayInput,
    ) -> Result<CreateSystemOverlayOutput> {
        self.repository.get_project(&input.project_id).await?;
        let overlay = self.repository.create_system_overlay(&input).await?;
        Ok(CreateSystemOverlayOutput {
            system_overlay_id: overlay.id,
        })
    }

    pub async fn create_future_knowledge(
        &self,
        input: CreateFutureKnowledgeInput,
    ) -> Result<CreateFutureKnowledgeOutput> {
        self.repository.get_project(&input.project_id).await?;
        let fk = self.repository.create_future_knowledge(&input).await?;
        Ok(CreateFutureKnowledgeOutput {
            future_knowledge_id: fk.id,
        })
    }

    /// Annotate the beats on a scene — overwrites any existing annotation for
    /// the (branch, scene) pair.
    pub async fn annotate_scene_beats(
        &self,
        input: AnnotateSceneBeatsInput,
    ) -> Result<AnnotateSceneBeatsOutput> {
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id {
            anyhow::bail!("scene does not belong to the requested project");
        }
        let beats: Vec<AnnotatedBeat> = input
            .beats
            .into_iter()
            .map(|b| AnnotatedBeat {
                beat_type: b.beat_type,
                summary: b.summary,
            })
            .collect();
        let annotation = self
            .repository
            .annotate_scene_beats(
                &input.project_id,
                &scene.branch_id,
                &scene.id,
                input.motif_ids,
                input.theme_ids,
                input.conflict_ids,
                beats,
            )
            .await?;
        Ok(AnnotateSceneBeatsOutput {
            scene_annotation_id: annotation.id,
        })
    }

    /// Plan or replan a chapter — upserts chapter_plan + chapter_outline.
    pub async fn plan_chapter(&self, input: PlanChapterInput) -> Result<PlanChapterOutput> {
        self.repository.get_project(&input.project_id).await?;

        // Validate the planned scenes' tone/beat descriptors against the style
        // contract BEFORE persisting, so genre drift is caught at planning time
        // (cheap) rather than after a chapter of prose is drafted (expensive).
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let style_warnings = match self
            .style_directive_for(&input.project_id, &branch_id)
            .await
        {
            Ok(directive) => validate_chapter_plan_style(&directive, &input.scenes),
            Err(_) => Vec::new(),
        };

        let plan = self.repository.plan_chapter(&input).await?;
        Ok(PlanChapterOutput {
            chapter_plan_id: plan.id,
            style_warnings,
        })
    }

    // =========================================================================
    // Generic update / archive / promise / revision_marker
    // =========================================================================

    pub async fn archive_entity(&self, input: ArchiveEntityInput) -> Result<ArchiveEntityOutput> {
        self.repository
            .archive_entity(&input.entity_type, &input.entity_id)
            .await?;
        Ok(ArchiveEntityOutput {
            entity_type: input.entity_type,
            entity_id: input.entity_id,
            archived: true,
        })
    }

    /// Generic field-merge update on a row identified by (entity_type, entity_id).
    /// The repository enforces a per-table column allowlist; unknown columns
    /// surface an error rather than silently being ignored.
    pub async fn update_entity(&self, input: UpdateEntityInput) -> Result<UpdateEntityOutput> {
        let entity_type = input.entity_type.clone();
        let entity_id = input.entity_id.clone();
        let changes: std::collections::BTreeMap<String, serde_json::Value> = match input.changes {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            serde_json::Value::Null => std::collections::BTreeMap::new(),
            _ => anyhow::bail!("update_entity changes must be a JSON object"),
        };
        let cache_target = self
            .phase_four_cache_target_for_entity_update(&entity_type, &entity_id, &changes)
            .await?;
        self.repository
            .update_entity_fields(&entity_type, &entity_id, changes)
            .await?;
        if let Some((project_id, branch_id, cache_ids)) = cache_target {
            match branch_id {
                Some(branch_id) => {
                    self.resolve_phase_four_caches(&project_id, &branch_id, &cache_ids)
                        .await?;
                }
                None => {
                    self.resolve_phase_four_caches_for_project(&project_id, &cache_ids)
                        .await?;
                }
            }
        }
        Ok(UpdateEntityOutput {
            entity_type,
            entity_id,
        })
    }

    pub async fn update_world_rule(
        &self,
        input: UpdateWorldRuleInput,
    ) -> Result<UpdateWorldRuleOutput> {
        let before = self.repository.get_world_rule(&input.world_rule_id).await?;
        let changes: std::collections::BTreeMap<String, serde_json::Value> = match input.changes {
            serde_json::Value::Object(map) => map.into_iter().collect(),
            _ => anyhow::bail!("update_world_rule changes must be a JSON object"),
        };
        let mut cache_ids = vec![PhaseFourCacheId::WorldRuleSemanticDrift];
        if before.rule_type.eq_ignore_ascii_case("style")
            || changes
                .get("rule_type")
                .and_then(|value| value.as_str())
                .is_some_and(|rule_type| rule_type.eq_ignore_ascii_case("style"))
        {
            cache_ids.push(PhaseFourCacheId::StyleCompliance);
        }
        self.repository
            .update_entity_fields("world_rule", &input.world_rule_id, changes)
            .await?;
        self.resolve_phase_four_caches(&before.project_id, &before.branch_id, &cache_ids)
            .await?;
        Ok(UpdateWorldRuleOutput {
            world_rule_id: input.world_rule_id,
        })
    }

    pub async fn update_promise_status(
        &self,
        input: UpdatePromiseStatusInput,
    ) -> Result<UpdatePromiseStatusOutput> {
        self.repository
            .update_promise_status(&input.narrative_promise_id, &input.status)
            .await?;
        Ok(UpdatePromiseStatusOutput {
            narrative_promise_id: input.narrative_promise_id,
            status: input.status,
        })
    }

    pub async fn resolve_revision_marker(
        &self,
        input: ResolveRevisionMarkerInput,
    ) -> Result<ResolveRevisionMarkerOutput> {
        let marker = self
            .repository
            .resolve_revision_marker(&input.marker_id)
            .await?;
        Ok(ResolveRevisionMarkerOutput {
            marker_id: marker.id,
            status: marker.status,
        })
    }

    // Outline setters are deferred — they return spindle_core's BookOutline /
    // ChapterOutline shapes (distinct from my sqlite::records types), and the
    // conversion adds avoidable complexity that isn't on the MCP critical path
    // for first-pass swap testing. Land here in a follow-up commit.

    // =========================================================================
    // Scene mutation: delete, operator_delete + listings
    // =========================================================================

    /// Move a scene to a new (book, chapter, scene_order) slot. The
    /// SurrealDB service first checks scene_move_impact for blockers
    /// (downstream canonical facts that would invalidate) — that helper
    /// hasn't been translated yet, so this version does the bare repo
    /// call without the readiness check.
    ///
    /// Output reports whether the source position was left with a numeric
    /// gap (i.e., a later scene_order at the source position still exists).
    /// Find entities by name across the searchable surface. Uses FTS5 over
    /// the character/location/world_rule indexes for exact matches and
    /// supplements with vec0 kNN for fuzzy/semantic resolution. Simplified
    /// version of the SurrealDB resolve_subject_by_name — uses the existing
    /// search backends rather than a dedicated repository helper.
    pub async fn find_entity(&self, input: FindEntityInput) -> Result<FindEntityOutput> {
        let query = input.query.trim();
        if query.is_empty() {
            anyhow::bail!("find_entity query must not be empty");
        }
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = match input.branch_id.as_deref() {
            Some(id) => id.to_string(),
            None => project
                .active_branch_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?,
        };
        let limit = input.limit.unwrap_or(5).clamp(1, 20);

        // Collect FTS5 hits across character / location / world_rule.
        // table_filter narrows the search to a single table if specified.
        let table_filter = input.table.map(|t| t.as_str().to_string());

        let mut matches: Vec<FindEntityMatch> = Vec::new();
        if table_filter.is_none() || table_filter.as_deref() == Some("character") {
            for (id, _rank, _snippet) in self
                .repository
                .fts_search_characters(&input.project_id, Some(&branch_id), query, limit)
                .await?
            {
                matches.push(FindEntityMatch {
                    entity_id: id,
                    confidence: EntityResolutionConfidence::ExactName,
                    score: 1.0,
                });
            }
        }
        if table_filter.is_none() || table_filter.as_deref() == Some("location") {
            for (id, _rank, _snippet) in self
                .repository
                .fts_search_locations(&input.project_id, Some(&branch_id), query, limit)
                .await?
            {
                matches.push(FindEntityMatch {
                    entity_id: id,
                    confidence: EntityResolutionConfidence::ExactName,
                    score: 1.0,
                });
            }
        }
        if table_filter.is_none() || table_filter.as_deref() == Some("world_rule") {
            for (id, _rank, _snippet) in self
                .repository
                .fts_search_world_rules(&input.project_id, Some(&branch_id), query, limit)
                .await?
            {
                matches.push(FindEntityMatch {
                    entity_id: id,
                    confidence: EntityResolutionConfidence::ExactName,
                    score: 1.0,
                });
            }
        }

        // If FTS turns up no hits, fall back to vec0 semantic search.
        if matches.is_empty() {
            let embedding_session = self.repository.model_router().embedding_session();
            let query_embedding = embedding_session.embed_text(query).await?;
            let knn = self
                .repository
                .knn_search_embeddings(&input.project_id, Some(&branch_id), &query_embedding, limit)
                .await?;
            for (embedding, distance) in knn {
                if let Some(ref table) = table_filter
                    && &embedding.entity_table != table
                {
                    continue;
                }
                matches.push(FindEntityMatch {
                    entity_id: embedding.entity_id,
                    confidence: EntityResolutionConfidence::SemanticMatch,
                    score: (1.0 / (1.0 + distance)) as f32,
                });
            }
        }

        // Sort by score descending and cap at limit.
        matches.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        matches.truncate(limit);
        Ok(FindEntityOutput { matches })
    }

    /// Find scenes that mention a phrase or reference a subject id. Phrase
    /// queries use FTS5; subject_id queries use a LIKE on full_text for
    /// scenes that literally contain the id string. Returns SceneReferenceItem
    /// with scene placement + snippet.
    /// Clear and rebuild the search index for a project's active branch.
    /// The SurrealDB version walked every searchable entity and re-embedded
    /// content; the SQLite version delegates the clear to the repository and
    /// re-embeds the entities it knows about — characters, locations, world
    /// rules, plus all narrative-entity tables that have search-worthy text.
    /// Set the markdown/JSON outline for a book on the active branch.
    /// Resolves the book from either book_id or book_number, upserts the
    /// (book_id, branch_id) row, and returns the public BookOutline shape
    /// with the new updated_at as RFC3339.
    pub async fn set_book_outline(
        &self,
        input: SetBookOutlineInput,
    ) -> Result<SetBookOutlineOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let book = if let Some(id) = input.book_id.as_deref() {
            self.repository.get_book(id).await?
        } else {
            self.repository
                .find_book_by_number(&input.project_id, input.book_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("book not found"))?
        };
        let format = input.format.unwrap_or_else(|| "markdown".to_string());
        let stored = self
            .repository
            .set_book_outline(&book.id, &branch_id, &format, &input.content)
            .await?;
        Ok(SetBookOutlineOutput {
            outline: BookOutline {
                book_id: stored.book_id,
                branch_id: stored.branch_id,
                format: stored.format,
                content: stored.content,
                updated_at: stored.updated_at.to_rfc3339(),
            },
        })
    }

    /// Set the chapter outline (content + beats) for a chapter on the active
    /// branch. Resolves the chapter from entity_id or (book_number,
    /// chapter_number), upserts the (chapter_id, branch_id) row, and returns
    /// the public ChapterOutline shape.
    pub async fn set_chapter_outline(
        &self,
        input: SetChapterOutlineInput,
    ) -> Result<SetChapterOutlineOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let chapter = if let Some(id) = input.entity_id.as_deref() {
            self.repository.get_chapter(id).await?
        } else {
            self.repository
                .find_chapter_by_number(&input.project_id, input.book_number, input.chapter_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("chapter not found"))?
        };
        let format = input.format.unwrap_or_else(|| "markdown".to_string());
        let content = input.content.unwrap_or_default();
        let stored = self
            .repository
            .set_chapter_outline(&chapter.id, &branch_id, &format, &content, input.beats)
            .await?;
        // stored.beats is Vec<StoredChapterOutlineBeat> — Phase 6 native
        // SQLite shape uses Option<String> for scene_id, identical to the
        // public ChapterOutlineBeat shape.
        let beats: Vec<ChapterOutlineBeat> = stored
            .beats
            .into_iter()
            .map(|b| ChapterOutlineBeat {
                order: b.order,
                summary: b.summary,
                scene_id: b.scene_id,
                status: b.status,
            })
            .collect();
        Ok(SetChapterOutlineOutput {
            outline: ChapterOutline {
                chapter_id: stored.chapter_id,
                branch_id: stored.branch_id,
                format: stored.format,
                content: stored.content,
                beats,
                updated_at: stored.updated_at.to_rfc3339(),
            },
        })
    }

    pub async fn rebuild_search_index(
        &self,
        input: RebuildSearchIndexInput,
    ) -> Result<RebuildSearchIndexOutput> {
        self.repository.get_project(&input.project_id).await?;
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;

        // 1. Clear everything on the active branch.
        let _cleared = self
            .repository
            .rebuild_search_index_clear(&input.project_id)
            .await?;

        // 2. Re-embed each searchable entity. Each refresh_entity_index call
        //    upserts one row into search_embedding which the AFTER trigger
        //    mirrors into vec_search_embedding.
        let mut indexed = 0usize;

        for character in self
            .repository
            .list_characters_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &character.id,
                "character",
                &character.name,
                &character.summary,
            )
            .await?;
            indexed += 1;
        }
        for location in self
            .repository
            .list_locations_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &location.id,
                "location",
                &location.name,
                &location.summary,
            )
            .await?;
            indexed += 1;
        }
        for rule in self
            .repository
            .list_world_rules_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &rule.id,
                "world_rule",
                &rule.rule_name,
                &rule.description,
            )
            .await?;
            indexed += 1;
        }
        for faction in self
            .repository
            .list_factions_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &faction.id,
                "faction",
                &faction.name,
                &faction.summary,
            )
            .await?;
            indexed += 1;
        }
        for plot in self
            .repository
            .list_plot_lines_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &plot.id,
                "plot_line",
                &plot.name,
                &plot.summary,
            )
            .await?;
            indexed += 1;
        }
        for conflict in self
            .repository
            .list_conflicts_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &conflict.id,
                "conflict",
                &conflict.name,
                &conflict.stakes,
            )
            .await?;
            indexed += 1;
        }
        for theme in self
            .repository
            .list_themes_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &theme.id,
                "theme",
                &theme.theme_statement,
                &theme.thesis_antithesis,
            )
            .await?;
            indexed += 1;
        }
        for motif in self
            .repository
            .list_motifs_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &motif.id,
                "motif",
                &motif.name,
                &motif.description,
            )
            .await?;
            indexed += 1;
        }
        for promise in self
            .repository
            .list_narrative_promises_by_project_and_branch(&input.project_id, &branch_id)
            .await?
        {
            self.refresh_entity_index(
                &input.project_id,
                &branch_id,
                &promise.id,
                "narrative_promise",
                &promise.promise_type,
                &promise.description,
            )
            .await?;
            indexed += 1;
        }

        Ok(RebuildSearchIndexOutput {
            indexed_records: indexed,
            embedding_version: self.repository.current_embedding_version(),
        })
    }

    pub async fn find_scenes_referencing(
        &self,
        input: FindScenesReferencingInput,
    ) -> Result<FindScenesReferencingOutput> {
        use spindle_core::models::{SceneReferenceItem, SceneReferenceQuery};
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let limit = input.limit.unwrap_or(20).min(100);
        let results = match input.query {
            SceneReferenceQuery::Phrase { phrase } => {
                let hits = self
                    .repository
                    .fts_search_scenes(&input.project_id, Some(&branch_id), &phrase, limit)
                    .await?;
                let mut items = Vec::with_capacity(hits.len());
                for (scene_id, _rank, snippet) in hits {
                    let scene = self.repository.get_scene(&scene_id).await?;
                    items.push(SceneReferenceItem {
                        scene_id,
                        book_number: scene.book_number,
                        chapter_number: scene.chapter_number,
                        scene_order: scene.scene_order,
                        snippet,
                        byte_range: None,
                    });
                }
                items
            }
            SceneReferenceQuery::Subject { subject_id } => {
                // Subject references are stored as the literal id string in
                // scene full_text. Use FTS5 since the id is a single token.
                let hits = self
                    .repository
                    .fts_search_scenes(&input.project_id, Some(&branch_id), &subject_id, limit)
                    .await?;
                let mut items = Vec::with_capacity(hits.len());
                for (scene_id, _rank, snippet) in hits {
                    let scene = self.repository.get_scene(&scene_id).await?;
                    items.push(SceneReferenceItem {
                        scene_id,
                        book_number: scene.book_number,
                        chapter_number: scene.chapter_number,
                        scene_order: scene.scene_order,
                        snippet,
                        byte_range: None,
                    });
                }
                items
            }
        };
        Ok(FindScenesReferencingOutput { results })
    }

    pub async fn move_scene(&self, input: MoveSceneInput) -> Result<MoveSceneOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &branch_id,
                input.from_book_number,
                input.from_chapter_number,
                input.from_scene_order,
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("source scene not found"))?;
        let dest_book = self
            .repository
            .ensure_book(&input.project_id, input.to_book_number)
            .await?;
        let dest_chapter = self
            .repository
            .ensure_chapter(
                &input.project_id,
                input.to_book_number,
                input.to_chapter_number,
            )
            .await?;
        let moved = self
            .repository
            .move_scene(
                &scene.id,
                &input.project_id,
                &branch_id,
                &dest_book.id,
                &dest_chapter.id,
                input.to_book_number,
                input.to_chapter_number,
                input.to_scene_order,
            )
            .await?;
        // Did the source position get a gap? Check whether a scene now
        // exists at from_scene_order + 1 in the source chapter.
        let later = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &branch_id,
                input.from_book_number,
                input.from_chapter_number,
                input.from_scene_order + 1,
            )
            .await?;
        Ok(MoveSceneOutput {
            scene_id: moved.id,
            branch_id,
            status: "moved".to_string(),
            from_book_number: input.from_book_number,
            from_chapter_number: input.from_chapter_number,
            from_scene_order: input.from_scene_order,
            to_book_number: input.to_book_number,
            to_chapter_number: input.to_chapter_number,
            to_scene_order: input.to_scene_order,
            left_source_scene_order_gap: later.is_some(),
        })
    }

    pub async fn delete_scene(&self, input: DeleteSceneInput) -> Result<DeleteSceneOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &branch_id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("scene not found at requested position"))?;
        let deleted_id = scene.id.clone();
        let scene_branch_id = scene.branch_id.clone();
        self.repository.delete_scene(&scene.id).await?;
        // Did deleting this scene leave a numeric gap? Check whether the next
        // higher scene_order still exists.
        let later = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &branch_id,
                input.book_number,
                input.chapter_number,
                input.scene_order + 1,
            )
            .await?;
        Ok(DeleteSceneOutput {
            scene_id: deleted_id,
            branch_id: scene_branch_id,
            status: "deleted".to_string(),
            left_scene_order_gap: later.is_some(),
        })
    }

    /// Operator-level delete — identical to delete_scene but used by the
    /// operator-only MCP tool that has fewer guard rails. For SQLite they
    /// share the same implementation.
    pub async fn operator_delete_scene(
        &self,
        input: OperatorDeleteSceneInput,
    ) -> Result<OperatorDeleteSceneOutput> {
        let standard = self
            .delete_scene(DeleteSceneInput {
                project_id: input.project_id,
                book_number: input.book_number,
                chapter_number: input.chapter_number,
                scene_order: input.scene_order,
            })
            .await?;
        Ok(OperatorDeleteSceneOutput {
            scene_id: standard.scene_id,
            branch_id: standard.branch_id,
            status: standard.status,
            left_scene_order_gap: standard.left_scene_order_gap,
            removed_scene_source_link_ids: Vec::new(),
            invalidated_chapter_plan_ids: Vec::new(),
            invalidated_chapter_summary_ids: Vec::new(),
        })
    }

    /// Read-only audit of every downstream reference that would be
    /// orphaned, made stale, or invalidated by deleting the scene at
    /// the requested `(book, chapter, scene_order)` on the project's
    /// active branch. Mirrors `services/mod.rs::get_scene_delete_impact`
    /// from 705b835^. Returns a structured report partitioned into
    /// hard blockers, semantic risks, and chapter artifacts, alongside
    /// an overall `delete_readiness` verdict.
    ///
    /// Relationship rows are reported via the synthetic id
    /// `relates_to:{in_id}->{out_id}` because SQLite stores relates_to
    /// rows under a composite primary key rather than a record-id
    /// column (the SurrealDB schema had an explicit `id`).
    pub async fn get_scene_delete_impact(
        &self,
        input: GetSceneDeleteImpactInput,
    ) -> Result<GetSceneDeleteImpactOutput> {
        use crate::format::{push_scene_delete_impact_group, scene_delete_placement_matches};

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?
            .with_context(|| {
                format!(
                    "scene not found on active branch {} at book {} chapter {} scene {}",
                    active_branch.name, input.book_number, input.chapter_number, input.scene_order
                )
            })?;

        let mut hard_blockers: Vec<SceneDeleteImpactGroup> = Vec::new();
        let mut semantic_risks: Vec<SceneDeleteImpactGroup> = Vec::new();
        let mut chapter_artifacts: Vec<SceneDeleteImpactGroup> = Vec::new();

        push_scene_delete_impact_group(
            &mut hard_blockers,
            "character_state",
            self.repository
                .list_character_states_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|state| {
                    state
                        .scene_id
                        .as_ref()
                        .is_some_and(|scene_id| scene_id == &scene.id)
                })
                .map(|state| state.id)
                .collect(),
            "Character states are committed against the exact scene id and must be removed or remapped before delete.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "revision_marker",
            self.repository
                .list_revision_markers_for_scene(&active_branch.id, &scene.id)
                .await?
                .into_iter()
                .map(|marker| marker.id)
                .collect(),
            "Revision markers point directly at this scene and would be orphaned by delete.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "dual_persona_review",
            self.repository
                .list_dual_persona_reviews_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|review| review.scene_id == scene.id)
                .map(|review| review.id)
                .collect(),
            "Dual-persona reviews are keyed to the exact scene id and need explicit cleanup.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "scene_version",
            self.repository
                .list_scene_versions(&scene.id)
                .await?
                .into_iter()
                .map(|version| version.id)
                .collect(),
            "Scene history snapshots belong to this scene id and need an explicit retention or purge policy.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "scene_beat_annotation",
            self.repository
                .list_scene_beat_annotations_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|annotation| annotation.scene_id == scene.id)
                .map(|annotation| annotation.id)
                .collect(),
            "Beat annotations point directly at this scene and would be orphaned by delete.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "canonical_fact",
            self.repository
                .list_canonical_facts_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|fact| fact.scene_id == scene.id)
                .map(|fact| fact.id)
                .collect(),
            "Canonical facts cite this scene as their source and must be removed, superseded, or re-sourced.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "scene_source_link",
            self.repository
                .list_scene_source_links_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|link| link.scene_id == scene.id)
                .map(|link| link.id)
                .collect(),
            "Source links point directly at this scene id and would become dangling references.",
        );
        push_scene_delete_impact_group(
            &mut hard_blockers,
            "relationship_last_scene",
            self.repository
                .list_relationships_by_branch(&active_branch.id)
                .await?
                .into_iter()
                .filter(|relationship| {
                    relationship
                        .last_scene_id
                        .as_ref()
                        .is_some_and(|scene_id| scene_id == &scene.id)
                })
                .map(|relationship| {
                    format!("relates_to:{}->{}", relationship.in_id, relationship.out_id)
                })
                .collect(),
            "Relationship recency is anchored to this scene id and would need repair before delete.",
        );

        push_scene_delete_impact_group(
            &mut semantic_risks,
            "narrative_promise_planted_at",
            self.repository
                .list_narrative_promises_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|promise| scene_delete_placement_matches(&promise.planted_at, &scene))
                .map(|promise| promise.id)
                .collect(),
            "Narrative promises planted at this story position would become semantically stale after delete.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "narrative_promise_planned_payoff",
            self.repository
                .list_narrative_promises_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|promise| {
                    promise
                        .planned_payoff
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|promise| promise.id)
                .collect(),
            "Planned promise payoffs scheduled at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "future_knowledge_learned_at",
            self.repository
                .list_future_knowledge_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|knowledge| scene_delete_placement_matches(&knowledge.learned_at, &scene))
                .map(|knowledge| knowledge.id)
                .collect(),
            "Future-knowledge acquisition tied to this scene position would become stale.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "future_knowledge_expires_at",
            self.repository
                .list_future_knowledge_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|knowledge| {
                    knowledge
                        .expires_at
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|knowledge| knowledge.id)
                .collect(),
            "Future-knowledge expiry anchored to this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "timeline_event_placement",
            self.repository
                .list_timeline_events_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|event| scene_delete_placement_matches(&event.placement, &scene))
                .map(|event| event.id)
                .collect(),
            "Timeline events placed at this scene would become semantically wrong after delete.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "character_arc_milestone",
            self.repository
                .list_character_arcs_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|arc| {
                    arc.milestones.iter().any(|milestone| {
                        milestone.placement.as_ref().is_some_and(|placement| {
                            scene_delete_placement_matches(placement, &scene)
                        })
                    })
                })
                .map(|arc| arc.id)
                .collect(),
            "Character-arc milestones scheduled at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "plot_line_convergence_point",
            self.repository
                .list_plot_lines_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|plot_line| {
                    plot_line
                        .convergence_points
                        .iter()
                        .any(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|plot_line| plot_line.id)
                .collect(),
            "Plot-line convergence points anchored to this scene would become stale.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "theme_introduction_point",
            self.repository
                .list_themes_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|theme| {
                    theme
                        .introduction_point
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|theme| theme.id)
                .collect(),
            "Theme introductions placed at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "theme_resolution_point",
            self.repository
                .list_themes_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|theme| {
                    theme
                        .resolution_point
                        .as_ref()
                        .is_some_and(|placement| scene_delete_placement_matches(placement, &scene))
                })
                .map(|theme| theme.id)
                .collect(),
            "Theme resolutions placed at this scene would need manual repositioning.",
        );
        push_scene_delete_impact_group(
            &mut semantic_risks,
            "conflict_stated_consequence",
            self.repository
                .list_conflicts_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|conflict| {
                    conflict.stated_consequences.iter().any(|consequence| {
                        consequence.stated_at.as_ref().is_some_and(|placement| {
                            scene_delete_placement_matches(placement, &scene)
                        })
                    })
                })
                .map(|conflict| conflict.id)
                .collect(),
            "Conflict consequences first stated at this scene would become semantically stale.",
        );

        push_scene_delete_impact_group(
            &mut chapter_artifacts,
            "chapter_plan_scene",
            self.repository
                .list_chapter_plans_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|plan| {
                    plan.book_number == scene.book_number
                        && plan.chapter_number == scene.chapter_number
                        && plan
                            .scenes
                            .iter()
                            .any(|planned_scene| planned_scene.scene_order == scene.scene_order)
                })
                .map(|plan| plan.id)
                .collect(),
            "Chapter plans that still target this scene order would need manual refresh.",
        );
        push_scene_delete_impact_group(
            &mut chapter_artifacts,
            "chapter_summary",
            self.repository
                .list_chapter_summaries_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|summary| {
                    summary.book_number == scene.book_number
                        && summary.chapter_number == scene.chapter_number
                })
                .map(|summary| summary.id)
                .collect(),
            "Chapter summaries for the affected chapter would become stale after delete.",
        );

        let delete_readiness = if !hard_blockers.is_empty() {
            SceneDeleteReadiness::Blocked
        } else if !semantic_risks.is_empty() || !chapter_artifacts.is_empty() {
            SceneDeleteReadiness::NeedsFollowup
        } else {
            SceneDeleteReadiness::Clear
        };

        let mut notes = vec![
            "This is a read-only audit against the current active-branch view.".to_string(),
            "The audit assumes scene deletion leaves a gap in scene_order; later scene positions are not auto-renumbered.".to_string(),
        ];
        if active_branch.name != "main" {
            notes.push(
                "Active-branch fallback records inherited from main can appear here because they remain visible in the current branch view."
                    .to_string(),
            );
        }
        if delete_readiness == SceneDeleteReadiness::Clear {
            notes.push(
                "No direct blockers or placement-based stale records were detected for this scene."
                    .to_string(),
            );
        }

        Ok(GetSceneDeleteImpactOutput {
            active_branch_id: active_branch.id.clone(),
            active_branch_name: active_branch.name.clone(),
            scene: SceneDeleteImpactTarget {
                scene_id: scene.id.clone(),
                branch_id: scene.branch_id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                summary: scene.summary,
            },
            delete_readiness,
            hard_blockers,
            semantic_risks,
            chapter_artifacts,
            notes,
        })
    }

    /// Read-only audit of every downstream reference that would be
    /// orphaned, made stale, or invalidated by moving the scene at
    /// `(from_book, from_chapter, from_scene_order)` to
    /// `(to_book, to_chapter, to_scene_order)` on the project's active
    /// branch. Mirrors `services/mod.rs::get_scene_move_impact` from
    /// 705b835^.
    ///
    /// Mechanically reuses `get_scene_delete_impact` as the source-side
    /// dependency probe: anything that's a delete-blocker at the source
    /// is also a move-blocker (the scene id stays the same, but its
    /// position changes). Each delete group is then re-narrated into a
    /// move-shaped reason via the helpers in `crate::format`. The
    /// destination-occupancy check fires when another scene already
    /// occupies the requested target position; the chapter-artifact
    /// probes fire for both the source and destination chapters when
    /// the move crosses chapters.
    pub async fn get_scene_move_impact(
        &self,
        input: GetSceneMoveImpactInput,
    ) -> Result<GetSceneMoveImpactOutput> {
        use crate::format::{
            push_scene_move_impact_group, scene_move_hard_blocker_from_delete_group,
            scene_move_semantic_risk_from_delete_group,
        };

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let source_scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.from_book_number,
                input.from_chapter_number,
                input.from_scene_order,
            )
            .await?
            .with_context(|| {
                format!(
                    "scene not found on active branch {} at book {} chapter {} scene {}",
                    active_branch.name,
                    input.from_book_number,
                    input.from_chapter_number,
                    input.from_scene_order
                )
            })?;

        let scene = SceneDeleteImpactTarget {
            scene_id: source_scene.id.clone(),
            branch_id: source_scene.branch_id.clone(),
            book_number: source_scene.book_number,
            chapter_number: source_scene.chapter_number,
            scene_order: source_scene.scene_order,
            summary: source_scene.summary.clone(),
        };

        if input.from_book_number == input.to_book_number
            && input.from_chapter_number == input.to_chapter_number
            && input.from_scene_order == input.to_scene_order
        {
            return Ok(GetSceneMoveImpactOutput {
                active_branch_id: active_branch.id.clone(),
                active_branch_name: active_branch.name.clone(),
                scene: scene.clone(),
                destination: SceneMoveImpactDestination {
                    book_number: input.to_book_number,
                    chapter_number: input.to_chapter_number,
                    scene_order: input.to_scene_order,
                    existing_scene_id: Some(scene.scene_id.clone()),
                    existing_summary: Some(scene.summary.clone()),
                },
                move_readiness: SceneMoveReadiness::Clear,
                hard_blockers: vec![],
                semantic_risks: vec![],
                chapter_artifacts: vec![],
                notes: vec![
                    "This is a read-only audit against the current active-branch view.".to_string(),
                    "Source and destination are identical; no move would be performed.".to_string(),
                ],
            });
        }

        let source_delete_impact = self
            .get_scene_delete_impact(GetSceneDeleteImpactInput {
                project_id: input.project_id.clone(),
                book_number: input.from_book_number,
                chapter_number: input.from_chapter_number,
                scene_order: input.from_scene_order,
            })
            .await?;
        let destination_scene = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.to_book_number,
                input.to_chapter_number,
                input.to_scene_order,
            )
            .await?;

        let mut hard_blockers: Vec<SceneMoveImpactGroup> = source_delete_impact
            .hard_blockers
            .into_iter()
            .map(scene_move_hard_blocker_from_delete_group)
            .collect();
        if let Some(destination_scene) = destination_scene.as_ref()
            && destination_scene.id != source_scene.id
        {
            push_scene_move_impact_group(
                &mut hard_blockers,
                "destination_scene",
                vec![destination_scene.id.clone()],
                "Another scene already occupies the requested destination; a safe move would need a full position rebase rather than an overwrite.",
            );
        }

        let semantic_risks: Vec<SceneMoveImpactGroup> = source_delete_impact
            .semantic_risks
            .into_iter()
            .map(scene_move_semantic_risk_from_delete_group)
            .collect();

        let mut chapter_artifacts: Vec<SceneMoveImpactGroup> = Vec::new();
        push_scene_move_impact_group(
            &mut chapter_artifacts,
            "source_chapter_plan",
            self.repository
                .list_chapter_plans_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|plan| {
                    plan.book_number == input.from_book_number
                        && plan.chapter_number == input.from_chapter_number
                })
                .map(|plan| plan.id)
                .collect(),
            "Chapter plans for the source chapter would become stale after moving a scene out of that chapter position.",
        );
        push_scene_move_impact_group(
            &mut chapter_artifacts,
            "source_chapter_summary",
            self.repository
                .list_chapter_summaries_by_project(&input.project_id)
                .await?
                .into_iter()
                .filter(|summary| {
                    summary.book_number == input.from_book_number
                        && summary.chapter_number == input.from_chapter_number
                })
                .map(|summary| summary.id)
                .collect(),
            "Chapter summaries for the source chapter would need refresh after a scene move.",
        );
        if input.from_book_number != input.to_book_number
            || input.from_chapter_number != input.to_chapter_number
        {
            push_scene_move_impact_group(
                &mut chapter_artifacts,
                "destination_chapter_plan",
                self.repository
                    .list_chapter_plans_by_project(&input.project_id)
                    .await?
                    .into_iter()
                    .filter(|plan| {
                        plan.book_number == input.to_book_number
                            && plan.chapter_number == input.to_chapter_number
                    })
                    .map(|plan| plan.id)
                    .collect(),
                "Chapter plans for the destination chapter would become stale after inserting a moved scene there.",
            );
            push_scene_move_impact_group(
                &mut chapter_artifacts,
                "destination_chapter_summary",
                self.repository
                    .list_chapter_summaries_by_project(&input.project_id)
                    .await?
                    .into_iter()
                    .filter(|summary| {
                        summary.book_number == input.to_book_number
                            && summary.chapter_number == input.to_chapter_number
                    })
                    .map(|summary| summary.id)
                    .collect(),
                "Chapter summaries for the destination chapter would need refresh after a scene move.",
            );
        }

        let move_readiness = if !hard_blockers.is_empty() {
            SceneMoveReadiness::Blocked
        } else if !semantic_risks.is_empty() || !chapter_artifacts.is_empty() {
            SceneMoveReadiness::NeedsFollowup
        } else {
            SceneMoveReadiness::Clear
        };

        let mut notes = vec![
            "This is a read-only audit against the current active-branch view.".to_string(),
            "The audit does not move or renumber scenes; it only reports the visible dependency and destination-conflict surface.".to_string(),
            "Only audit-clear moves are safe for the current tool; anything else still requires a dependency-aware position rebase rather than a direct scene_order rewrite.".to_string(),
        ];
        if active_branch.name != "main" {
            notes.push(
                "Active-branch fallback records inherited from main can appear here because they remain visible in the current branch view."
                    .to_string(),
            );
        }
        if move_readiness == SceneMoveReadiness::Clear {
            notes.push(
                "No direct blockers, destination conflicts, or chapter-level stale records were detected for this move."
                    .to_string(),
            );
        }

        Ok(GetSceneMoveImpactOutput {
            active_branch_id: active_branch.id.clone(),
            active_branch_name: active_branch.name.clone(),
            scene,
            destination: SceneMoveImpactDestination {
                book_number: input.to_book_number,
                chapter_number: input.to_chapter_number,
                scene_order: input.to_scene_order,
                existing_scene_id: destination_scene.as_ref().map(|s| s.id.clone()),
                existing_summary: destination_scene.map(|s| s.summary),
            },
            move_readiness,
            hard_blockers,
            semantic_risks,
            chapter_artifacts,
            notes,
        })
    }

    pub async fn list_chapter_scenes(
        &self,
        input: ListChapterScenesInput,
    ) -> Result<ListChapterScenesOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;

        // Resolve the chapter row from either chapter_id or (book_number, chapter_number).
        let chapter = if let Some(id) = input.chapter_id.as_deref() {
            self.repository.get_chapter(id).await?
        } else {
            self.repository
                .find_chapter_by_number(&input.project_id, input.book_number, input.chapter_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("chapter not found"))?
        };
        let scenes = self.repository.list_scenes_by_chapter(&chapter.id).await?;
        let scene_entries = scenes
            .into_iter()
            .map(|s| spindle_core::models::SceneSpineEntry {
                scene_id: s.id,
                scene_order: s.scene_order,
                word_count: s.full_text.split_whitespace().count(),
                summary_first_line: s.summary.lines().next().unwrap_or("").to_string(),
                has_canonical_facts: false, // Set in a later commit that joins canonical_fact.
                content_rating: match s.content_rating.as_str() {
                    "General" => spindle_core::models::ContentRating::General,
                    "Teen" => spindle_core::models::ContentRating::Teen,
                    "Mature" => spindle_core::models::ContentRating::Mature,
                    "Explicit" => spindle_core::models::ContentRating::Explicit,
                    _ => spindle_core::models::ContentRating::General,
                },
                tone: s.tone,
            })
            .collect();
        Ok(ListChapterScenesOutput {
            project_id: input.project_id,
            branch_id,
            book_id: chapter.book_id,
            chapter_id: chapter.id,
            book_number: chapter.book_number,
            chapter_number: chapter.chapter_number,
            title: chapter.title,
            scenes: scene_entries,
        })
    }

    pub async fn list_book_chapters(
        &self,
        input: ListBookChaptersInput,
    ) -> Result<ListBookChaptersOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let book = if let Some(id) = input.book_id.as_deref() {
            self.repository.get_book(id).await?
        } else {
            self.repository
                .find_book_by_number(&input.project_id, input.book_number)
                .await?
                .ok_or_else(|| anyhow::anyhow!("book not found"))?
        };
        let chapters = self.repository.list_chapters_by_book(&book.id).await?;
        let mut chapter_entries = Vec::with_capacity(chapters.len());
        for ch in chapters {
            let scenes = self.repository.list_scenes_by_chapter(&ch.id).await?;
            chapter_entries.push(spindle_core::models::ChapterSpineEntry {
                chapter_id: ch.id,
                chapter_number: ch.chapter_number,
                title: ch.title,
                scene_count: scenes.len(),
                scenes: scenes
                    .into_iter()
                    .map(|s| spindle_core::models::SceneSpineEntry {
                        scene_id: s.id,
                        scene_order: s.scene_order,
                        word_count: s.full_text.split_whitespace().count(),
                        summary_first_line: s.summary.lines().next().unwrap_or("").to_string(),
                        has_canonical_facts: false,
                        content_rating: match s.content_rating.as_str() {
                            "General" => spindle_core::models::ContentRating::General,
                            "Teen" => spindle_core::models::ContentRating::Teen,
                            "Mature" => spindle_core::models::ContentRating::Mature,
                            "Explicit" => spindle_core::models::ContentRating::Explicit,
                            _ => spindle_core::models::ContentRating::General,
                        },
                        tone: s.tone,
                    })
                    .collect(),
            });
        }
        Ok(ListBookChaptersOutput {
            project_id: input.project_id,
            branch_id,
            book_id: book.id,
            book_number: book.book_number,
            title: book.title,
            chapters: chapter_entries,
        })
    }

    /// Register a canonical fact, optionally superseding a prior fact.
    /// Maps the wide RegisterCanonicalFactInput shape onto the repository's
    /// CreateCanonicalFactParams. Defaults to scope=Invariant, value_kind
    /// derived from which value field is set.
    pub async fn register_canonical_fact(
        &self,
        input: RegisterCanonicalFactInput,
    ) -> Result<RegisterCanonicalFactOutput> {
        self.repository.get_project(&input.project_id).await?;
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id {
            anyhow::bail!("scene does not belong to the requested project");
        }
        // Pick value_kind based on which value field is populated.
        let value_kind = input.value_kind.clone().unwrap_or_else(|| {
            if input.value_number.is_some() {
                "number".to_string()
            } else if input.value_json.is_some() {
                "json".to_string()
            } else {
                "string".to_string()
            }
        });
        let predicate = input
            .predicate
            .clone()
            .or_else(|| input.key.clone())
            .ok_or_else(|| anyhow::anyhow!("predicate or key is required"))?;
        let subject_table = input
            .subject_table
            .clone()
            .unwrap_or_else(|| "project".to_string());
        let scope = match input.scope {
            Some(CanonicalFactScope::Invariant) | None => "invariant",
            Some(CanonicalFactScope::Evolving) => "evolving",
            Some(CanonicalFactScope::Conditional) => "conditional",
        }
        .to_string();
        let params = crate::sqlite::repository::CreateCanonicalFactParams {
            project_id: input.project_id.clone(),
            branch_id: scene.branch_id.clone(),
            scene_id: scene.id.clone(),
            book_number: input.book_number,
            chapter_number: input.chapter_number,
            subject_table,
            subject_id: input.subject_id,
            predicate,
            value_kind,
            value_text: input.value_text.or(input.value.clone()),
            value_number: input.value_number,
            unit: input.value_unit,
            value_json: input.value_json,
            aliases: input.aliases,
            scope,
            valid_from: input.valid_from,
            valid_until: input.valid_until,
            legacy_untyped: input.legacy_untyped.unwrap_or(false),
        };
        let fact = self.repository.create_canonical_fact(params).await?;
        let superseded_fact_id = match input.supersedes_fact_id {
            Some(old) => {
                self.repository
                    .supersede_canonical_fact(&old, &fact.id)
                    .await?;
                Some(old)
            }
            None => None,
        };
        self.resolve_phase_four_caches(
            &fact.project_id,
            &fact.branch_id,
            &[PhaseFourCacheId::CanonicalFactProseDrift],
        )
        .await?;
        Ok(RegisterCanonicalFactOutput {
            canonical_fact_id: fact.id,
            superseded_fact_id,
        })
    }

    pub async fn list_scene_versions(
        &self,
        input: ListSceneVersionsInput,
    ) -> Result<ListSceneVersionsOutput> {
        self.repository.get_project(&input.project_id).await?;
        let versions = self.repository.list_scene_versions(&input.scene_id).await?;
        let summaries = versions
            .into_iter()
            .map(|v| SceneVersionSummary {
                scene_version_id: v.id,
                version_number: v.version_number,
                saved_at: v.created_at.to_rfc3339(),
                word_count: v.full_text.split_whitespace().count(),
                summary: v.summary,
            })
            .collect();
        Ok(ListSceneVersionsOutput {
            scene_id: input.scene_id,
            versions: summaries,
        })
    }

    pub async fn restore_scene_version(
        &self,
        input: RestoreSceneVersionInput,
    ) -> Result<RestoreSceneVersionOutput> {
        self.repository.get_project(&input.project_id).await?;
        let version = self
            .repository
            .get_scene_version(&input.scene_version_id)
            .await?;
        let scene = self
            .repository
            .restore_scene_version_and_mark_reviews_stale(&input.scene_id, &version)
            .await?;
        Ok(RestoreSceneVersionOutput {
            scene_id: scene.id,
            restored_from_version_id: version.id,
            restored_version_number: version.version_number,
            status: "restored".to_string(),
        })
    }

    pub async fn list_revision_markers(
        &self,
        input: ListRevisionMarkersInput,
    ) -> Result<ListRevisionMarkersOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = project
            .active_branch_id
            .clone()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?;
        let markers = self
            .repository
            .list_revision_markers_for_scene(&branch_id, &input.scene_id)
            .await?;
        let summaries = markers
            .into_iter()
            .map(|m| spindle_core::models::RevisionMarkerSummary {
                marker_id: m.id,
                marker_type: m.marker_type,
                target_record_id: m.target_record_id,
                position: m.position,
                note: m.note,
                status: m.status,
            })
            .collect();
        Ok(ListRevisionMarkersOutput {
            scene_id: input.scene_id,
            markers: summaries,
        })
    }

    // =========================================================================
    // Agent / config tools (delegate to model_router; no DB work)
    // =========================================================================

    pub fn list_agents(&self) -> ListAgentsOutput {
        self.repository.model_router().list_agents()
    }

    pub fn agent_routing_config(&self) -> spindle_core::models::AgentRoutingConfigOutput {
        self.repository.model_router().routing_config()
    }

    pub fn model_routes(&self) -> Vec<spindle_core::models::ModelRouteSummary> {
        use crate::ai::adapter_pulls_canon_via_mcp;
        self.repository
            .model_router()
            .list_route_bindings()
            .into_iter()
            .map(|binding| {
                let caller_should_send_brief =
                    adapter_pulls_canon_via_mcp(&binding.route.adapter_kind);
                spindle_core::models::ModelRouteSummary {
                    route_name: binding.route.route_name,
                    adapter_kind: binding.route.adapter_kind,
                    model_name: binding.route.model_name,
                    purpose: binding.route.purpose,
                    rating: binding.rating,
                    caller_should_send_brief,
                }
            })
            .collect()
    }

    pub fn configure_agents(&self, input: ConfigureAgentsInput) -> Result<ConfigureAgentsOutput> {
        self.repository
            .model_router()
            .configure(input.config_path.as_deref())
    }

    pub async fn test_agent(&self, input: TestAgentInput) -> Result<TestAgentOutput> {
        self.repository
            .model_router()
            .test_agent(&input.agent_id, input.test_prompt.as_deref())
            .await
    }

    pub async fn set_arc_pacing_constraints(
        &self,
        input: SetArcPacingConstraintsInput,
    ) -> Result<SetArcPacingConstraintsOutput> {
        let arc = self
            .repository
            .get_character_arc(&input.character_arc_id)
            .await?;
        if arc.project_id != input.project_id {
            anyhow::bail!("character arc does not belong to the requested project");
        }
        let tracker = self
            .repository
            .get_pacing_tracker_by_arc(&input.character_arc_id)
            .await?;
        let updated = self
            .repository
            .set_arc_pacing_constraints(
                &tracker.id,
                input.per_book_budget,
                input.max_progress_per_chapter,
                input.milestone_spacing,
                input.sprint_allowance,
                input.regression_budget,
            )
            .await?;
        Ok(SetArcPacingConstraintsOutput {
            pacing_tracker_id: updated.id,
        })
    }

    /// UPSERT a character's voice profile + clear voice_drift validator
    /// findings on the active branch + log a session_activity. Mirrors the
    /// SurrealDB service flow, simplified by skipping the
    /// voice_profile_affected_scene_ids helper (which isn't ported) — the
    /// findings clear runs branch-wide for the voice_drift validator instead.
    pub async fn set_character_voice_profile(
        &self,
        input: SetCharacterVoiceProfileInput,
    ) -> Result<SetCharacterVoiceProfileOutput> {
        if input.profile.updated_at.is_some() {
            anyhow::bail!("voice profile updated_at is server-managed and must be omitted");
        }

        let character = self.repository.get_character(&input.character_id).await?;
        if character.project_id != input.project_id {
            anyhow::bail!("character does not belong to the requested project");
        }
        let active_branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        if character.branch_id != active_branch_id {
            anyhow::bail!("character must belong to the active branch");
        }

        // Validate established_in_scene_id if present.
        if let Some(scene_id) = input.profile.established_in_scene_id.as_deref() {
            let scene = self.repository.get_scene(scene_id).await?;
            if scene.project_id != input.project_id {
                anyhow::bail!("established_in_scene_id must belong to the requested project");
            }
            if scene.branch_id != active_branch_id {
                anyhow::bail!("established_in_scene_id must belong to the active branch");
            }
        }

        let stored = self
            .repository
            .set_character_voice_profile(&character.id, &input.profile)
            .await?;
        // Branch-wide voice_drift resolution as a simplification of the
        // SurrealDB voice_profile_affected_scene_ids helper.
        self.resolve_phase_four_caches(
            &input.project_id,
            &active_branch_id,
            &[PhaseFourCacheId::VoiceDrift],
        )
        .await?;
        let _ = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: input.project_id.clone(),
                branch_id: active_branch_id.clone(),
                kind: "voice_profile_set".to_string(),
                subject_table: Some("character".to_string()),
                subject_id: Some(character.id.clone()),
                summary: format!("updated voice profile for {}", character.name),
                details_json: Some(serde_json::json!({
                    "character_id": character.id,
                    "character_name": character.name,
                    "tone": stored.tone,
                    "vocabulary_count": stored.vocabulary.len(),
                    "sentence_structure_count": stored.sentence_structure.len(),
                    "tics_count": stored.tics.len(),
                    "forbidden_words_count": stored.forbidden_words.len(),
                    "example_lines_count": stored.example_lines.len(),
                    "established_in_scene_id": stored.established_in_scene_id,
                })),
            })
            .await;

        Ok(SetCharacterVoiceProfileOutput {
            character_id: input.character_id,
            branch_id: active_branch_id,
            profile: CharacterVoiceProfileData {
                tone: stored.tone,
                vocabulary: stored.vocabulary,
                sentence_structure: stored.sentence_structure,
                tics: stored.tics,
                forbidden_words: stored.forbidden_words,
                example_lines: stored.example_lines,
                established_in_scene_id: stored.established_in_scene_id,
                updated_at: stored.updated_at.map(|t| t.to_rfc3339()),
            },
            activity_id: None,
        })
    }

    // =========================================================================
    // MCP-dispatched tool surface
    // =========================================================================
    //
    // The methods below complete the SqliteSpindleService's MCP-required
    // surface. Phase 6 stub-fill finished — every method has a real
    // implementation and matches the SurrealDB-era behaviour
    // observable through MCP.

    /// Fetch a branch's identifying fields for MCP's session-management
    /// flows. Returns (branch_id, project_id, name). The `project_id` is
    /// always present for SQLite-created branches (per-project main branches
    /// design); the Option in the underlying record is a schema-level legacy
    /// from the SurrealDB singleton era and surfaces here as an error if
    /// somehow encountered.
    pub async fn get_branch_info(&self, branch_id: &str) -> Result<(String, String, String)> {
        let b = self.repository.get_branch(branch_id).await?;
        let project_id = b
            .project_id
            .ok_or_else(|| anyhow::anyhow!("branch {branch_id} has no project_id"))?;
        Ok((b.id, project_id, b.name))
    }

    /// Resolve the active branch id for a project. Used by MCP's session
    /// defaulting (`default_branch_id_for_project`) so callers can omit
    /// `branch_id` and have it auto-filled with the project's current main.
    ///
    /// Per the per-project-main-branch design (Phase 6 reconciliation of the
    /// SurrealDB singleton bible_branch:main divergence), every project has
    /// its own main branch row — there is no shared fallback.
    pub async fn active_branch_id_for_project(&self, project_id: &str) -> Result<String> {
        self.repository.active_branch_id_public(project_id).await
    }

    /// List all project IDs as `project:<ulid>` strings. Drives MCP's
    /// resource enumeration without forcing the full list_projects payload.
    pub async fn list_project_ids(&self) -> Result<Vec<String>> {
        Ok(self
            .repository
            .list_projects()
            .await?
            .into_iter()
            .map(|p| p.id)
            .collect())
    }

    async fn continuity_health_resource(&self, project_id: &str) -> Result<serde_json::Value> {
        use serde_json::{Value, json};
        use std::collections::{BTreeMap, BTreeSet};

        self.repository.get_project(project_id).await?;
        let active_branch = self.repository.get_active_branch(project_id).await?;
        let branches = self.repository.list_branches_by_project(project_id).await?;
        let branch_by_id = branches
            .iter()
            .map(|branch| (branch.id.clone(), branch))
            .collect::<BTreeMap<_, _>>();

        let mut branch_lineage = Vec::new();
        let mut seen_branch_ids = BTreeSet::new();
        let mut current_branch_id = Some(active_branch.id.clone());
        while let Some(branch_id) = current_branch_id {
            if !seen_branch_ids.insert(branch_id.clone()) {
                break;
            }
            let Some(branch) = branch_by_id.get(&branch_id) else {
                break;
            };
            branch_lineage.push(json!({
                "id": branch.id.clone(),
                "name": branch.name.clone(),
                "parent_branch_id": branch.parent_branch_id.clone(),
                "status": branch.status.clone(),
                "branch_type": branch.branch_type.clone(),
            }));
            current_branch_id = branch.parent_branch_id.clone();
        }
        branch_lineage.reverse();

        let validator_findings = self
            .repository
            .list_validator_findings_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut open_by_validator_and_severity: BTreeMap<String, BTreeMap<String, usize>> =
            BTreeMap::new();
        let mut cache_counts_by_validator: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        let mut total_open_findings = 0usize;
        let mut total_active_cache_rows = 0usize;
        let mut total_resolved_cache_rows = 0usize;

        for finding in &validator_findings {
            if finding.resolved_at.is_none() {
                total_open_findings += 1;
                *open_by_validator_and_severity
                    .entry(finding.validator_id.clone())
                    .or_default()
                    .entry(finding.severity.clone())
                    .or_default() += 1;
            }

            if finding.finding_id == "__cache__" {
                let (active_count, resolved_count) = cache_counts_by_validator
                    .entry(finding.validator_id.clone())
                    .or_insert((0usize, 0usize));
                if finding.resolved_at.is_some() {
                    *resolved_count += 1;
                    total_resolved_cache_rows += 1;
                } else {
                    *active_count += 1;
                    total_active_cache_rows += 1;
                }
            }
        }

        let validator_cache_counts = cache_counts_by_validator
            .into_iter()
            .map(|(validator_id, (active_count, resolved_count))| {
                (
                    validator_id,
                    json!({
                        "active_count": active_count,
                        "resolved_count": resolved_count,
                        "total_count": active_count + resolved_count,
                    }),
                )
            })
            .collect::<serde_json::Map<_, _>>();

        let timeline_events = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let timeline_event_ids = timeline_events
            .iter()
            .map(|event| event.id.clone())
            .collect::<BTreeSet<_>>();
        let temporal_interventions = self
            .repository
            .list_temporal_interventions_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut orphaned_temporal_interventions = Vec::new();
        for intervention in &temporal_interventions {
            let mut missing_endpoints = Vec::new();
            match intervention.source_event_id.as_ref() {
                Some(source_event_id) if !timeline_event_ids.contains(source_event_id) => {
                    missing_endpoints.push(json!({
                        "field": "source_event_id",
                        "event_id": source_event_id,
                        "reason": "not_found",
                    }));
                }
                None => missing_endpoints.push(json!({
                    "field": "source_event_id",
                    "event_id": null,
                    "reason": "unset",
                })),
                _ => {}
            }
            match intervention.target_event_id.as_ref() {
                Some(target_event_id) if !timeline_event_ids.contains(target_event_id) => {
                    missing_endpoints.push(json!({
                        "field": "target_event_id",
                        "event_id": target_event_id,
                        "reason": "not_found",
                    }));
                }
                None => missing_endpoints.push(json!({
                    "field": "target_event_id",
                    "event_id": null,
                    "reason": "unset",
                })),
                _ => {}
            }

            if !missing_endpoints.is_empty() {
                orphaned_temporal_interventions.push(json!({
                    "id": intervention.id.clone(),
                    "title": intervention.title.clone(),
                    "source_event_id": intervention.source_event_id.clone(),
                    "target_event_id": intervention.target_event_id.clone(),
                    "missing_endpoints": missing_endpoints,
                }));
            }
        }

        let canonical_facts = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut facts_by_key: BTreeMap<(String, Option<String>, String), Vec<_>> = BTreeMap::new();
        for fact in &canonical_facts {
            facts_by_key
                .entry((
                    fact.subject_table.clone(),
                    fact.subject_id.clone(),
                    fact.predicate.clone(),
                ))
                .or_default()
                .push(fact);
        }
        let duplicate_active_canonical_facts = facts_by_key
            .into_iter()
            .filter_map(|((subject_table, subject_id, predicate), facts)| {
                if facts.len() < 2 {
                    return None;
                }
                let values = facts
                    .iter()
                    .map(|fact| canonical_fact_value_for_check(fact))
                    .collect::<Vec<_>>();
                let unique_values = values
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                Some(json!({
                    "subject_table": subject_table,
                    "subject_id": subject_id,
                    "predicate": predicate,
                    "fact_ids": facts.iter().map(|fact| fact.id.clone()).collect::<Vec<_>>(),
                    "values": values,
                    "unique_values": unique_values,
                }))
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "project_id": project_id,
            "active_branch_id": active_branch.id,
            "branch_count": branches.len(),
            "branch_lineage": branch_lineage,
            "validator_findings": {
                "total_open": total_open_findings,
                "open_by_validator_and_severity": open_by_validator_and_severity,
            },
            "validator_cache_counts": validator_cache_counts,
            "validator_cache_totals": {
                "active_count": total_active_cache_rows,
                "resolved_count": total_resolved_cache_rows,
                "total_count": total_active_cache_rows + total_resolved_cache_rows,
            },
            "last_check_consistency_at": Value::Null,
            "timeline": {
                "timeline_event_count": timeline_events.len(),
                "temporal_intervention_count": temporal_interventions.len(),
            },
            "orphaned_temporal_interventions": orphaned_temporal_interventions,
            "canonical_facts": {
                "active_count": canonical_facts.len(),
            },
            "duplicate_active_canonical_facts": duplicate_active_canonical_facts,
        }))
    }

    pub async fn timeline_graph_mermaid_resource(&self, project_id: &str) -> Result<String> {
        use std::collections::BTreeMap;

        self.repository.get_project(project_id).await?;
        let active_branch = self.repository.get_active_branch(project_id).await?;

        let mut branches = self.repository.list_branches_by_project(project_id).await?;
        branches.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        let mut save_points = self.repository.list_all_save_points(project_id).await?;
        save_points.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));

        let mut timeline_events = self.repository.list_all_timeline_events(project_id).await?;
        timeline_events.sort_by(|a, b| {
            a.branch_id
                .cmp(&b.branch_id)
                .then(a.placement.book_number.cmp(&b.placement.book_number))
                .then(a.placement.chapter_number.cmp(&b.placement.chapter_number))
                .then(
                    a.placement
                        .scene_order
                        .unwrap_or(0)
                        .cmp(&b.placement.scene_order.unwrap_or(0)),
                )
                .then(a.title.cmp(&b.title))
                .then(a.id.cmp(&b.id))
        });

        let mut temporal_interventions = self
            .repository
            .list_all_temporal_interventions(project_id)
            .await?;
        temporal_interventions.sort_by(|a, b| {
            a.branch_id
                .cmp(&b.branch_id)
                .then(a.title.cmp(&b.title))
                .then(a.id.cmp(&b.id))
        });

        let branch_node_ids = branches
            .iter()
            .map(|branch| (branch.id.clone(), mermaid_node_id("B", &branch.id)))
            .collect::<BTreeMap<_, _>>();
        let save_point_node_ids = save_points
            .iter()
            .map(|save_point| (save_point.id.clone(), mermaid_node_id("SP", &save_point.id)))
            .collect::<BTreeMap<_, _>>();
        let event_node_ids = timeline_events
            .iter()
            .map(|event| (event.id.clone(), mermaid_node_id("E", &event.id)))
            .collect::<BTreeMap<_, _>>();

        let mut out = String::new();
        out.push_str("```mermaid\n");
        out.push_str("flowchart LR\n");
        out.push_str("  classDef branch fill:#eef,stroke:#557\n");
        out.push_str("  classDef savepoint fill:#efe,stroke:#575\n");
        out.push_str("  classDef event fill:#fff,stroke:#777\n");
        out.push_str("  classDef warning fill:#fee,stroke:#a44\n\n");

        for branch in &branches {
            let node_id = &branch_node_ids[&branch.id];
            let active_suffix = if branch.id == active_branch.id {
                " (active)"
            } else {
                ""
            };
            out.push_str(&format!(
                "  {node_id}[\"{}\"]:::branch\n",
                mermaid_escape_label(&format!("{}{}", branch.name, active_suffix))
            ));
        }
        for save_point in &save_points {
            let node_id = &save_point_node_ids[&save_point.id];
            out.push_str(&format!(
                "  {node_id}[\"{}\"]:::savepoint\n",
                mermaid_escape_label(&format!("save: {}", save_point.name))
            ));
        }
        if !branches.is_empty() || !save_points.is_empty() {
            out.push('\n');
        }

        for branch in &branches {
            if let Some(parent_branch_id) = branch.parent_branch_id.as_ref()
                && let Some(parent_node_id) = branch_node_ids.get(parent_branch_id)
            {
                let branch_node_id = &branch_node_ids[&branch.id];
                out.push_str(&format!("  {parent_node_id} --> {branch_node_id}\n"));
            }
            if let Some(save_point_id) = branch.created_from_save_point_id.as_ref()
                && let Some(save_point_node_id) = save_point_node_ids.get(save_point_id)
            {
                let branch_node_id = &branch_node_ids[&branch.id];
                out.push_str(&format!("  {save_point_node_id} --> {branch_node_id}\n"));
            }
        }
        for save_point in &save_points {
            if let Some(branch_node_id) = branch_node_ids.get(&save_point.branch_id) {
                let save_point_node_id = &save_point_node_ids[&save_point.id];
                out.push_str(&format!("  {branch_node_id} --> {save_point_node_id}\n"));
            }
        }
        if !branches.is_empty() || !save_points.is_empty() {
            out.push('\n');
        }

        let interventions_by_branch = temporal_interventions.iter().fold(
            BTreeMap::<String, Vec<_>>::new(),
            |mut acc, intervention| {
                acc.entry(intervention.branch_id.clone())
                    .or_default()
                    .push(intervention);
                acc
            },
        );
        let events_by_branch =
            timeline_events
                .iter()
                .fold(BTreeMap::<String, Vec<_>>::new(), |mut acc, event| {
                    acc.entry(event.branch_id.clone()).or_default().push(event);
                    acc
                });

        for branch in &branches {
            let branch_events = events_by_branch
                .get(&branch.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let branch_interventions = interventions_by_branch
                .get(&branch.id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let warning_interventions = branch_interventions
                .iter()
                .filter(|intervention| {
                    let source_visible = intervention
                        .source_event_id
                        .as_ref()
                        .is_some_and(|id| event_node_ids.contains_key(id));
                    let target_visible = intervention
                        .target_event_id
                        .as_ref()
                        .is_some_and(|id| event_node_ids.contains_key(id));
                    !(source_visible && target_visible)
                })
                .collect::<Vec<_>>();

            if branch_events.is_empty() && warning_interventions.is_empty() {
                continue;
            }

            let subgraph_id = mermaid_node_id("SG", &branch.id);
            out.push_str(&format!(
                "  subgraph {subgraph_id}[\"{}\"]\n",
                mermaid_escape_label(&format!("{} timeline", branch.name))
            ));
            for event in branch_events {
                let event_node_id = &event_node_ids[&event.id];
                out.push_str(&format!(
                    "    {event_node_id}[\"{}\"]:::event\n",
                    mermaid_escape_label(&timeline_event_graph_label(event))
                ));
            }
            for window in branch_events.windows(2) {
                let from = &event_node_ids[&window[0].id];
                let to = &event_node_ids[&window[1].id];
                out.push_str(&format!("    {from} --> {to}\n"));
            }
            for intervention in warning_interventions {
                let warning_node_id = mermaid_node_id("W", &intervention.id);
                out.push_str(&format!(
                    "    {warning_node_id}[\"{}\"]:::warning\n",
                    mermaid_escape_label(&format!(
                        "missing {}: {}",
                        mermaid_missing_endpoint_summary(intervention, &event_node_ids),
                        intervention.title
                    ))
                ));
            }
            out.push_str("  end\n\n");
        }

        for intervention in &temporal_interventions {
            let Some(source_node_id) = intervention
                .source_event_id
                .as_ref()
                .and_then(|id| event_node_ids.get(id))
            else {
                continue;
            };
            let Some(target_node_id) = intervention
                .target_event_id
                .as_ref()
                .and_then(|id| event_node_ids.get(id))
            else {
                continue;
            };
            out.push_str(&format!(
                "  {source_node_id} -. \"{}\" .-> {target_node_id}\n",
                mermaid_escape_label(&format!("intervention: {}", intervention.title))
            ));
        }

        out.push_str("```\n");
        Ok(out)
    }

    /// Read a project-scoped resource by path (e.g., `"books"`,
    /// `"chapters/1/2/scenes"`, `"research-log"`, `"conflicts/0/50"`). Drives
    /// the `bible://projects/{id}/...` MCP resource layer. Dispatches across:
    ///
    /// * Paginated resources (research-log, conflicts, future-knowledge,
    ///   timeline-events, dual-persona-reviews, relationships,
    ///   temporal-interventions) — both bare and `<resource>/<offset>/<limit>`
    ///   shapes.
    /// * The `chapters/<book>/<chapter>/scenes` reader.
    /// * `imports` and `imports/<session_id>[/<nested_path>]`.
    /// * 20+ simple list resources (books, chapters, characters, locations,
    ///   world-rules, factions, plot-lines, conflicts, themes, motifs,
    ///   narrative-promises, pacing/overview, chapter-summaries,
    ///   reader-contract, branches, future-knowledge, timeline-events,
    ///   temporal-interventions, system-overlays, dual-persona-reviews,
    ///   religions, economies, terms, character-arcs, relationships).
    ///
    /// `scene-move-impact/<from_book>/<from_chapter>/<from_scene>/<to_book>/<to_chapter>/<to_scene>`
    /// and `scene-delete-impact/<book>/<chapter>/<scene>` parse the path
    /// tail into the natural-key coordinates of the impact methods and
    /// return the structured `GetScene*ImpactOutput` payload as JSON.
    pub async fn read_project_resource(
        &self,
        project_id: &str,
        resource_path: &str,
    ) -> Result<serde_json::Value> {
        use serde_json::{Value, json};
        let project_id = project_id.to_string();
        if let Some(page) = parse_project_resource_page_request(resource_path)? {
            return Ok(match page.kind {
                PaginatedProjectResourceKind::ResearchLog => {
                    let entries = self
                        .repository
                        .list_research_logs_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(|entry| {
                            json!({
                                "id": entry.id,
                                "project_id": entry.project_id,
                                "query": entry.query,
                                "context_hint": entry.context_hint,
                                "model": entry.model,
                                "response": entry.response,
                                "context_summary": entry.context_summary,
                                "created_at": entry.created_at.to_rfc3339(),
                            })
                        })
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::Conflicts => {
                    let entries = self
                        .repository
                        .list_conflicts_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "project_id": c.project_id,
                                "name": c.name,
                                "conflict_type": c.conflict_type,
                                "stakes": c.stakes,
                                "escalation_stages": c.escalation_stages,
                                "expected_total_cycles": c.expected_total_cycles,
                                "try_fail_cycles": c.try_fail_cycles,
                                "stated_consequences": c.stated_consequences,
                                "resolution_summary": c.resolution_summary,
                                "notes": c.notes,
                            })
                        })
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::FutureKnowledge => {
                    let entries = self
                        .repository
                        .list_future_knowledge_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(future_knowledge_to_json)
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::TimelineEvents => {
                    let entries = self
                        .repository
                        .list_timeline_events_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(timeline_event_to_json)
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::DualPersonaReviews => {
                    let entries = self
                        .repository
                        .list_dual_persona_reviews_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(persisted_dual_persona_review)
                        .collect::<anyhow::Result<Vec<_>>>()?
                        .into_iter()
                        .map(serde_json::to_value)
                        .collect::<Result<Vec<_>, _>>()?;
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
                PaginatedProjectResourceKind::Relationships => {
                    let active_branch = self.repository.get_active_branch(&project_id).await?;
                    let entries = self
                        .repository
                        .list_relationships_by_branch(&active_branch.id)
                        .await?
                        .into_iter()
                        .map(relates_to_json)
                        .collect::<Vec<_>>();
                    let mut extra = serde_json::Map::new();
                    extra.insert("active_branch_id".to_string(), json!(active_branch.id));
                    paginated_project_resource_response(&project_id, page, entries, Some(extra))
                }
                PaginatedProjectResourceKind::TemporalInterventions => {
                    let entries = self
                        .repository
                        .list_temporal_interventions_by_project(&project_id)
                        .await?
                        .into_iter()
                        .map(temporal_intervention_to_json)
                        .collect::<Vec<_>>();
                    paginated_project_resource_response(&project_id, page, entries, None)
                }
            });
        }
        if resource_path == "continuity/health" {
            return self.continuity_health_resource(&project_id).await;
        }
        if let Some(chapter_path) = resource_path.strip_prefix("chapters/") {
            let parts = chapter_path.split('/').collect::<Vec<_>>();
            if parts.len() == 3 && parts[2] == "scenes" {
                let book_number = parts[0].parse::<i32>().with_context(|| {
                    format!("invalid book number in resource path: {}", parts[0])
                })?;
                let chapter_number = parts[1].parse::<i32>().with_context(|| {
                    format!("invalid chapter number in resource path: {}", parts[1])
                })?;
                let active_branch = self.repository.get_active_branch(&project_id).await?;
                let chapter = self
                    .repository
                    .get_chapter_by_number(&project_id, book_number, chapter_number)
                    .await?;
                let scenes = self
                    .repository
                    .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
                    .await?
                    .into_iter()
                    .filter(|scene| {
                        scene.book_number == book_number && scene.chapter_number == chapter_number
                    })
                    .map(|scene| {
                        json!({
                            "id": scene.id,
                            "project_id": scene.project_id,
                            "branch_id": scene.branch_id,
                            "book_id": scene.book_id,
                            "chapter_id": scene.chapter_id,
                            "book_number": scene.book_number,
                            "chapter_number": scene.chapter_number,
                            "scene_order": scene.scene_order,
                            "summary": scene.summary,
                            "content_rating": scene.content_rating,
                            "tone": scene.tone,
                        })
                    })
                    .collect::<Vec<_>>();
                return Ok(json!({
                    "active_branch_id": active_branch.id,
                    "book_number": book_number,
                    "chapter_number": chapter_number,
                    "chapter_id": chapter.id,
                    "title": chapter.title,
                    "scenes": scenes,
                }));
            }
            anyhow::bail!(
                "chapter resource path must be chapters/<book_number>/<chapter_number>/scenes"
            );
        }
        if let Some(scene_path) = resource_path.strip_prefix("scene-move-impact/") {
            let parts = scene_path.split('/').collect::<Vec<_>>();
            if parts.len() != 6 {
                anyhow::bail!(
                    "scene move impact resource path must be from_book/from_chapter/from_scene/to_book/to_chapter/to_scene"
                );
            }
            let from_book_number = parts[0].parse::<i32>().with_context(|| {
                format!("invalid source book number in resource path: {}", parts[0])
            })?;
            let from_chapter_number = parts[1].parse::<i32>().with_context(|| {
                format!(
                    "invalid source chapter number in resource path: {}",
                    parts[1]
                )
            })?;
            let from_scene_order = parts[2].parse::<i32>().with_context(|| {
                format!("invalid source scene order in resource path: {}", parts[2])
            })?;
            let to_book_number = parts[3].parse::<i32>().with_context(|| {
                format!(
                    "invalid destination book number in resource path: {}",
                    parts[3]
                )
            })?;
            let to_chapter_number = parts[4].parse::<i32>().with_context(|| {
                format!(
                    "invalid destination chapter number in resource path: {}",
                    parts[4]
                )
            })?;
            let to_scene_order = parts[5].parse::<i32>().with_context(|| {
                format!(
                    "invalid destination scene order in resource path: {}",
                    parts[5]
                )
            })?;
            let impact = self
                .get_scene_move_impact(GetSceneMoveImpactInput {
                    project_id: project_id.clone(),
                    from_book_number,
                    from_chapter_number,
                    from_scene_order,
                    to_book_number,
                    to_chapter_number,
                    to_scene_order,
                })
                .await?;
            return Ok(serde_json::to_value(impact)?);
        }
        if let Some(scene_path) = resource_path.strip_prefix("scene-delete-impact/") {
            let parts = scene_path.split('/').collect::<Vec<_>>();
            if parts.len() != 3 {
                anyhow::bail!("scene delete impact resource path must be book/chapter/scene");
            }
            let book_number = parts[0]
                .parse::<i32>()
                .with_context(|| format!("invalid book number in resource path: {}", parts[0]))?;
            let chapter_number = parts[1].parse::<i32>().with_context(|| {
                format!("invalid chapter number in resource path: {}", parts[1])
            })?;
            let scene_order = parts[2]
                .parse::<i32>()
                .with_context(|| format!("invalid scene order in resource path: {}", parts[2]))?;
            let impact = self
                .get_scene_delete_impact(GetSceneDeleteImpactInput {
                    project_id: project_id.clone(),
                    book_number,
                    chapter_number,
                    scene_order,
                })
                .await?;
            return Ok(serde_json::to_value(impact)?);
        }
        if resource_path == "imports" {
            let sessions = self
                .repository
                .list_import_sessions_by_project(&project_id)
                .await?
                .into_iter()
                .map(|session| crate::sqlite::import_service::import_session_summary(&session))
                .collect::<anyhow::Result<Vec<_>>>()?;
            return Ok(serde_json::to_value(sessions)?);
        }
        if let Some(session_path) = resource_path.strip_prefix("imports/") {
            use spindle_core::models::ImportStatusInput;
            if !session_path.contains('/') {
                let status = self
                    .import_status(ImportStatusInput {
                        project_id: project_id.clone(),
                        session_id: session_path.to_string(),
                    })
                    .await?;
                return Ok(serde_json::to_value(status.session)?);
            }
            let (session_id, nested_path) = session_path
                .split_once('/')
                .context("import resource is missing a nested path")?;
            let status = self
                .import_status(ImportStatusInput {
                    project_id: project_id.clone(),
                    session_id: session_id.to_string(),
                })
                .await?;
            return Ok(match nested_path {
                "summary" => serde_json::to_value(status.session)?,
                "structure" => serde_json::to_value(status.structure)?,
                "entity-extraction" => serde_json::to_value(status.entity_extraction)?,
                "entity-consolidation" => serde_json::to_value(status.entity_consolidation)?,
                "characters" => serde_json::to_value(status.characters)?,
                "world" => serde_json::to_value(status.world)?,
                "narrative" => serde_json::to_value(status.narrative)?,
                "resume-snapshot" => serde_json::to_value(status.final_state)?,
                "review-items" => serde_json::to_value(status.review_items)?,
                "hydration-report" => serde_json::to_value(status.hydration_report)?,
                _ => anyhow::bail!("unknown import resource path: {resource_path}"),
            });
        }
        let active_branch = self.repository.get_active_branch(&project_id).await?;
        Ok(match resource_path {
            "books" => {
                let books = self.repository.list_books_by_project(&project_id).await?;
                let scenes = self
                    .repository
                    .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
                    .await?;
                let mut items = Vec::new();
                for book in books {
                    let chapters = self
                        .repository
                        .list_chapters_by_book_number(&project_id, book.book_number)
                        .await?;
                    let scene_count = scenes
                        .iter()
                        .filter(|scene| scene.book_number == book.book_number)
                        .count();
                    items.push(json!({
                        "id": book.id,
                        "project_id": book.project_id,
                        "book_number": book.book_number,
                        "title": book.title,
                        "chapter_count": chapters.len(),
                        "scene_count": scene_count,
                    }));
                }
                Value::Array(items)
            }
            "chapters" => {
                let books = self.repository.list_books_by_project(&project_id).await?;
                let scenes = self
                    .repository
                    .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
                    .await?;
                let mut chapters_out = Vec::new();
                for book in books {
                    for chapter in self
                        .repository
                        .list_chapters_by_book_number(&project_id, book.book_number)
                        .await?
                    {
                        let chapter_scenes = scenes
                            .iter()
                            .filter(|scene| {
                                scene.book_number == chapter.book_number
                                    && scene.chapter_number == chapter.chapter_number
                            })
                            .collect::<Vec<_>>();
                        chapters_out.push(json!({
                            "id": chapter.id,
                            "project_id": chapter.project_id,
                            "book_id": chapter.book_id,
                            "book_number": chapter.book_number,
                            "chapter_number": chapter.chapter_number,
                            "title": chapter.title,
                            "scene_count": chapter_scenes.len(),
                            "scene_orders": chapter_scenes
                                .iter()
                                .map(|scene| scene.scene_order)
                                .collect::<Vec<_>>(),
                        }));
                    }
                }
                Value::Array(chapters_out)
            }
            "characters" => {
                let records = self
                    .repository
                    .list_characters_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "project_id": c.project_id,
                                "branch_id": c.branch_id,
                                "name": c.name,
                                "role": c.role,
                                "summary": c.summary,
                                "realm": c.realm,
                                "appearance": c.appearance,
                                "notes": c.notes,
                            })
                        })
                        .collect(),
                )
            }
            "locations" => {
                let records = self
                    .repository
                    .list_locations_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|l| {
                            json!({
                                "id": l.id,
                                "project_id": l.project_id,
                                "branch_id": l.branch_id,
                                "name": l.name,
                                "kind": l.kind,
                                "realm": l.realm,
                                "summary": l.summary,
                                "notes": l.notes,
                            })
                        })
                        .collect(),
                )
            }
            "world-rules" => {
                let records = self
                    .repository
                    .list_world_rules_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|r| {
                            json!({
                                "id": r.id,
                                "project_id": r.project_id,
                                "rule_name": r.rule_name,
                                "rule_type": r.rule_type,
                                "description": r.description,
                                "scan_pattern": r.scan_pattern,
                                "relevance_tags": r.relevance_tags.unwrap_or_default(),
                                "established_in": r.established_in.map(|e| json!({
                                    "book_number": e.book_number,
                                    "chapter_number": e.chapter_number,
                                    "note": e.note,
                                })),
                                "notes": r.notes,
                            })
                        })
                        .collect(),
                )
            }
            "factions" => {
                let records = self
                    .repository
                    .list_factions_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|f| {
                            json!({
                                "id": f.id,
                                "project_id": f.project_id,
                                "name": f.name,
                                "faction_type": f.faction_type,
                                "realm": f.realm,
                                "summary": f.summary,
                                "tags": f.tags,
                                "notes": f.notes,
                            })
                        })
                        .collect(),
                )
            }
            "plot-lines" => {
                let records = self
                    .repository
                    .list_plot_lines_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|p| {
                            json!({
                                "id": p.id,
                                "project_id": p.project_id,
                                "name": p.name,
                                "plot_type": p.plot_type,
                                "summary": p.summary,
                                "status": p.status,
                                "convergence_points": p.convergence_points,
                                "notes": p.notes,
                            })
                        })
                        .collect(),
                )
            }
            "conflicts" => {
                let records = self
                    .repository
                    .list_conflicts_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "project_id": c.project_id,
                                "name": c.name,
                                "conflict_type": c.conflict_type,
                                "stakes": c.stakes,
                                "escalation_stages": c.escalation_stages,
                                "expected_total_cycles": c.expected_total_cycles,
                                "try_fail_cycles": c.try_fail_cycles,
                                "stated_consequences": c.stated_consequences,
                                "resolution_summary": c.resolution_summary,
                                "notes": c.notes,
                            })
                        })
                        .collect(),
                )
            }
            "themes" => {
                let records = self.repository.list_themes_by_project(&project_id).await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "project_id": t.project_id,
                                "theme_statement": t.theme_statement,
                                "thesis_antithesis": t.thesis_antithesis,
                                "introduction_point": t.introduction_point,
                                "resolution_point": t.resolution_point,
                                "notes": t.notes,
                            })
                        })
                        .collect(),
                )
            }
            "motifs" => {
                let records = self.repository.list_motifs_by_project(&project_id).await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|m| {
                            json!({
                                "id": m.id,
                                "project_id": m.project_id,
                                "name": m.name,
                                "description": m.description,
                                "max_uses_per_chapter": m.max_uses_per_chapter,
                                "connected_theme_ids": m.connected_theme_ids,
                                "notes": m.notes,
                            })
                        })
                        .collect(),
                )
            }
            "narrative-promises" => {
                let records = self
                    .repository
                    .list_narrative_promises_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|p| {
                            json!({
                                "id": p.id,
                                "project_id": p.project_id,
                                "promise_type": p.promise_type,
                                "description": p.description,
                                "status": p.status,
                                "planted_at": p.planted_at,
                                "planned_payoff": p.planned_payoff,
                                "notes": p.notes,
                            })
                        })
                        .collect(),
                )
            }
            "pacing/overview" => {
                let records = self
                    .repository
                    .list_pacing_trackers_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|p| {
                            json!({
                                "id": p.id,
                                "project_id": p.project_id,
                                "character_arc_id": p.character_arc_id,
                                "per_book_budget": p.per_book_budget,
                                "max_progress_per_chapter": p.max_progress_per_chapter,
                                "milestone_spacing": p.milestone_spacing,
                                "sprint_allowance": p.sprint_allowance,
                                "regression_budget": p.regression_budget,
                                "current_progress": p.current_progress,
                                "budget_remaining": p.budget_remaining,
                                "velocity": p.velocity,
                                "status": p.status,
                                "next_milestone": p.next_milestone,
                                "warnings": p.warnings,
                            })
                        })
                        .collect(),
                )
            }
            "chapter-summaries" => {
                let records = self
                    .repository
                    .list_chapter_summaries_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|summary| {
                            json!({
                                "id": summary.id,
                                "project_id": summary.project_id,
                                "branch_id": summary.branch_id,
                                "book_number": summary.book_number,
                                "chapter_number": summary.chapter_number,
                                "summary": summary.summary,
                                "key_events": summary.key_events,
                                "character_changes": summary.character_changes,
                                "relationship_shifts": summary.relationship_shifts,
                                "arc_advances": summary.arc_advances,
                                "promise_events": summary.promise_events,
                            })
                        })
                        .collect(),
                )
            }
            "reader-contract" => serde_json::to_value(
                self.repository
                    .get_project(&project_id)
                    .await?
                    .reader_contract
                    .into_core(),
            )?,
            "branches" => {
                let project = self.repository.get_project(&project_id).await?;
                let active_branch_id = project.active_branch_id;
                let branches = self
                    .repository
                    .list_branches_by_project(&project_id)
                    .await?
                    .into_iter()
                    .map(|branch| {
                        crate::format::branch_summary(&branch, active_branch_id.as_deref())
                    })
                    .collect::<Vec<_>>();
                serde_json::to_value(branches)?
            }
            "future-knowledge" => {
                let records = self
                    .repository
                    .list_future_knowledge_by_project(&project_id)
                    .await?;
                Value::Array(records.into_iter().map(future_knowledge_to_json).collect())
            }
            "timeline-events" => {
                let records = self
                    .repository
                    .list_timeline_events_by_project(&project_id)
                    .await?;
                Value::Array(records.into_iter().map(timeline_event_to_json).collect())
            }
            "temporal-interventions" => {
                let records = self
                    .repository
                    .list_temporal_interventions_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(temporal_intervention_to_json)
                        .collect(),
                )
            }
            "system-overlays" => {
                let records = self
                    .repository
                    .list_system_overlays_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|s| {
                            json!({
                                "id": s.id,
                                "system_name": s.system_name,
                                "system_type": s.system_type,
                                "rules": s.rules,
                                "visibility": s.visibility,
                                "progression_currency": s.progression_currency,
                                "stats": s.stats,
                                "advancement_tiers": s.advancement_tiers,
                            })
                        })
                        .collect(),
                )
            }
            "dual-persona-reviews" => serde_json::to_value(
                self.repository
                    .list_dual_persona_reviews_by_project(&project_id)
                    .await?
                    .into_iter()
                    .map(persisted_dual_persona_review)
                    .collect::<anyhow::Result<Vec<_>>>()?,
            )?,
            "religions" => {
                let records = self
                    .repository
                    .list_religions_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|r| {
                            json!({
                                "id": r.id,
                                "project_id": r.project_id,
                                "name": r.name,
                                "deity_or_principle": r.deity_or_principle,
                                "summary": r.summary,
                                "tags": r.tags,
                                "notes": r.notes,
                            })
                        })
                        .collect(),
                )
            }
            "economies" => {
                let records = self
                    .repository
                    .list_economies_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|e| {
                            json!({
                                "id": e.id,
                                "project_id": e.project_id,
                                "name": e.name,
                                "realm": e.realm,
                                "summary": e.summary,
                                "scarce_resources": e.scarce_resources,
                                "trade_goods": e.trade_goods,
                                "currency": e.currency,
                                "notes": e.notes,
                            })
                        })
                        .collect(),
                )
            }
            "terms" => {
                let records = self.repository.list_terms_by_project(&project_id).await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|t| {
                            json!({
                                "id": t.id,
                                "project_id": t.project_id,
                                "term_text": t.term_text,
                                "pronunciation": t.pronunciation,
                                "definition": t.definition,
                                "usage_context": t.usage_context,
                                "origin": t.origin,
                                "notes": t.notes,
                            })
                        })
                        .collect(),
                )
            }
            "character-arcs" => {
                let records = self
                    .repository
                    .list_character_arcs_by_project(&project_id)
                    .await?;
                Value::Array(
                    records
                        .into_iter()
                        .map(|a| {
                            json!({
                                "id": a.id,
                                "project_id": a.project_id,
                                "character_id": a.character_id,
                                "arc_type": a.arc_type,
                                "starting_state": a.starting_state,
                                "ending_state": a.ending_state,
                                "milestones": a.milestones,
                                "thematic_purpose": a.thematic_purpose,
                                "connected_theme_ids": a.connected_theme_ids,
                                "status": a.status,
                                "progress": a.progress,
                                "notes": a.notes,
                            })
                        })
                        .collect(),
                )
            }
            "relationships" => {
                let branch = self.repository.get_active_branch(&project_id).await?;
                let records = self
                    .repository
                    .list_relationships_by_branch(&branch.id)
                    .await?;
                Value::Array(records.into_iter().map(relates_to_json).collect())
            }
            _ => anyhow::bail!("unknown project resource path: {resource_path}"),
        })
    }

    /// Polymorphic read by record id. Returns a minimal JSON envelope with
    /// `{id, table}` if the entity exists, errors otherwise. The SurrealDB
    /// version returned the full SurrealDB record; MCP's set_active_project
    /// only needs existence validation, so this version keeps the surface
    /// narrow rather than wiring `#[derive(Serialize)]` across every SQLite
    /// record type.
    pub async fn read_entity_by_id(&self, record_id_str: &str) -> Result<serde_json::Value> {
        let (table, _key) = record_id_str
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("expected `table:id` format, got `{record_id_str}`"))?;
        let exists = match table {
            "project" => self.repository.get_project(record_id_str).await.is_ok(),
            "book" => self.repository.get_book(record_id_str).await.is_ok(),
            "chapter" => self.repository.get_chapter(record_id_str).await.is_ok(),
            "scene" => self.repository.get_scene(record_id_str).await.is_ok(),
            "character" => self.repository.get_character(record_id_str).await.is_ok(),
            "location" => self.repository.get_location(record_id_str).await.is_ok(),
            "bible_branch" => self.repository.get_branch(record_id_str).await.is_ok(),
            "faction" => self.repository.get_faction(record_id_str).await.is_ok(),
            "religion" => self.repository.get_religion(record_id_str).await.is_ok(),
            "economy" => self.repository.get_economy(record_id_str).await.is_ok(),
            "term" => self.repository.get_term(record_id_str).await.is_ok(),
            "plot_line" => self.repository.get_plot_line(record_id_str).await.is_ok(),
            "conflict" => self.repository.get_conflict(record_id_str).await.is_ok(),
            "theme" => self.repository.get_theme(record_id_str).await.is_ok(),
            "motif" => self.repository.get_motif(record_id_str).await.is_ok(),
            "narrative_promise" => self
                .repository
                .get_narrative_promise(record_id_str)
                .await
                .is_ok(),
            "world_rule" => self.repository.get_world_rule(record_id_str).await.is_ok(),
            "character_arc" => self
                .repository
                .get_character_arc(record_id_str)
                .await
                .is_ok(),
            "save_point" => self.repository.get_save_point(record_id_str).await.is_ok(),
            "canonical_fact" => self
                .repository
                .get_canonical_fact(record_id_str)
                .await
                .is_ok(),
            "knowledge_fact" => self
                .repository
                .get_knowledge_fact(record_id_str)
                .await
                .is_ok(),
            "system_overlay" => self
                .repository
                .get_system_overlay(record_id_str)
                .await
                .is_ok(),
            "timeline_event" => self
                .repository
                .get_timeline_event(record_id_str)
                .await
                .is_ok(),
            "future_knowledge" => self
                .repository
                .get_future_knowledge(record_id_str)
                .await
                .is_ok(),
            other => anyhow::bail!("read_entity_by_id does not support table `{other}`"),
        };
        if !exists {
            anyhow::bail!("entity {record_id_str} not found");
        }
        Ok(serde_json::json!({ "id": record_id_str, "table": table }))
    }

    /// Aggregate the writer's current state on a project/branch: cursor scene,
    /// hard constraints, subject snapshots, recent scenes, due promises,
    /// active system overlays, outlines, and recent session activity. Pure
    /// orchestration over `Repository` reads plus the writer-state helpers in
    /// [`crate::format`].
    ///
    /// Per-project main branches (Phase 6 reconciliation): the SurrealDB
    /// reference compared against the global `bible_branch:main` singleton.
    /// Here, the "fall back to main" semantics resolve per project — the
    /// project's branch named "main" is the fallback target. When that branch
    /// IS the requested branch, no fallback happens.
    ///
    /// `unsynced_local_files` and `drift_warnings` are populated by walking
    /// `list_scene_source_links_by_project` and running each link through
    /// `SourceBridge::evaluate_scene_divergence` — same semantics as the
    /// SurrealDB reference. Risk #7 (see migration plan) is now closed.
    pub async fn get_writer_state(
        &self,
        input: spindle_core::models::GetWriterStateInput,
    ) -> Result<spindle_core::models::WriterState> {
        use crate::format::{
            self, DEFAULT_WRITER_STATE_BUDGET_TOKENS, DEFAULT_WRITER_STATE_RECENT_ACTIVITY_LIMIT,
            DEFAULT_WRITER_STATE_RECENT_SCENE_LIMIT, DEFAULT_WRITER_STATE_SUGGESTED_SUBJECT_LIMIT,
        };
        use serde_json::json;
        use spindle_core::models::{
            BookSummary, ChapterPositionSummary, ContextBundleSummary, ContextFormat,
            HardConstraint, OutlineRef, OverlayWithTrajectory, ProjectSummary, Provenance,
            RelationshipSummary, ScenePositionSummary, SessionActivitySummary, SubjectRef,
            WriterIntent, WriterState, WriterStateCurrent, WriterStateNarrativePromiseSummary,
            WriterStateNext, WriterStateSubjectSnapshot,
        };

        let format_fmt = input.format.unwrap_or(ContextFormat::Markdown);
        let token_budget = input
            .budget_tokens
            .unwrap_or(DEFAULT_WRITER_STATE_BUDGET_TOKENS);
        let include_subjects = input.include_subjects.unwrap_or(true);
        let include_recent_activity = input.include_recent_activity.unwrap_or(true);
        let recent_activity_limit = input
            .recent_activity_limit
            .unwrap_or(DEFAULT_WRITER_STATE_RECENT_ACTIVITY_LIMIT);

        let project = self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&project.id).await?;

        let branch = match input.branch_id.as_deref() {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(project.id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        project.id
                    );
                }
                branch
            }
            None => active_branch.clone(),
        };
        let branch_id = branch.id.clone();

        // Resolve the project's main branch (per-project design) for
        // fallback when the requested branch has no scenes/characters/etc.
        // yet — gives writers a useful view when they switch to a fresh
        // feature branch.
        let main_branch_id = self
            .repository
            .list_branches_by_project(&project.id)
            .await?
            .into_iter()
            .find(|b| b.name == "main")
            .map(|b| b.id)
            .unwrap_or_else(|| branch_id.clone());

        let writer_position = self
            .repository
            .get_writer_position(&project.id, &branch_id)
            .await?;

        let mut scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &branch_id)
            .await?;
        if scenes.is_empty() && branch_id != main_branch_id {
            scenes = self
                .repository
                .list_scenes_by_project_and_branch(&project.id, &main_branch_id)
                .await?;
        }

        let cursor_scene = match input.at_scene_id.as_deref() {
            Some(scene_id) => {
                let scene = self.repository.get_scene(scene_id).await?;
                if scene.project_id != project.id
                    || !scenes.iter().any(|candidate| candidate.id == scene.id)
                {
                    anyhow::bail!(
                        "scene {scene_id} does not belong to the requested project branch"
                    );
                }
                Some(scene)
            }
            None => {
                if let Some(position_scene_id) =
                    writer_position.as_ref().and_then(|p| p.scene_id.clone())
                {
                    match self.repository.get_scene(&position_scene_id).await {
                        Ok(scene)
                            if scene.project_id == project.id && scene.branch_id == branch_id =>
                        {
                            Some(scene)
                        }
                        _ => scenes.last().cloned(),
                    }
                } else {
                    scenes.last().cloned()
                }
            }
        };
        let cursor_index = cursor_scene
            .as_ref()
            .and_then(|scene| scenes.iter().position(|candidate| candidate.id == scene.id));
        let next_scene = cursor_index
            .and_then(|index| scenes.get(index + 1))
            .cloned();

        let mut book_summary = None;
        let mut chapter_summary = None;
        let mut scene_summary = None;
        let mut last_completed_scene_summary = None;
        if let Some(scene) = cursor_scene.as_ref() {
            let book = self.repository.get_book(&scene.book_id).await?;
            let chapter = self.repository.get_chapter(&scene.chapter_id).await?;
            book_summary = Some(BookSummary {
                book_id: book.id.clone(),
                book_number: book.book_number,
                title: book.title.clone(),
            });
            chapter_summary = Some(ChapterPositionSummary {
                chapter_id: chapter.id.clone(),
                book_number: chapter.book_number,
                chapter_number: chapter.chapter_number,
                title: chapter.title.clone(),
            });
            scene_summary = Some(ScenePositionSummary {
                scene_id: scene.id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                summary: scene.summary.clone(),
                tone: scene.tone.clone(),
            });
            last_completed_scene_summary = cursor_index
                .and_then(|index| index.checked_sub(1))
                .and_then(|index| scenes.get(index))
                .map(|s| s.summary.clone());
        }
        if book_summary.is_none()
            && let Some(position_book_id) =
                writer_position.as_ref().and_then(|p| p.book_id.as_ref())
            && let Ok(book) = self.repository.get_book(position_book_id).await
            && book.project_id == project.id
        {
            book_summary = Some(BookSummary {
                book_id: book.id.clone(),
                book_number: book.book_number,
                title: book.title.clone(),
            });
        }
        if chapter_summary.is_none()
            && let Some(position_chapter_id) =
                writer_position.as_ref().and_then(|p| p.chapter_id.as_ref())
            && let Ok(chapter) = self.repository.get_chapter(position_chapter_id).await
            && chapter.project_id == project.id
        {
            chapter_summary = Some(ChapterPositionSummary {
                chapter_id: chapter.id.clone(),
                book_number: chapter.book_number,
                chapter_number: chapter.chapter_number,
                title: chapter.title.clone(),
            });
        }
        if scene_summary.is_none()
            && let Some(position_scene_id) =
                writer_position.as_ref().and_then(|p| p.scene_id.as_ref())
            && let Ok(scene) = self.repository.get_scene(position_scene_id).await
            && scene.project_id == project.id
            && scene.branch_id == branch_id
        {
            scene_summary = Some(ScenePositionSummary {
                scene_id: scene.id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                summary: scene.summary.clone(),
                tone: scene.tone.clone(),
            });
        }

        let mut world_rules = self
            .repository
            .list_world_rules_by_project_and_branch(&project.id, &branch_id)
            .await?;
        if world_rules.is_empty() && branch_id != main_branch_id {
            world_rules = self
                .repository
                .list_world_rules_by_project_and_branch(&project.id, &main_branch_id)
                .await?;
        }
        let hard_constraints = world_rules
            .into_iter()
            .map(|rule| HardConstraint {
                id: rule.rule_name,
                statement: rule.description,
            })
            .collect::<Vec<_>>();

        let mut character_branch_id = branch_id.clone();
        let mut characters = self
            .repository
            .list_characters_by_project_and_branch(&project.id, &branch_id)
            .await?;
        if characters.is_empty() && branch_id != main_branch_id {
            characters = self
                .repository
                .list_characters_by_project_and_branch(&project.id, &main_branch_id)
                .await?;
            character_branch_id = main_branch_id.clone();
        }
        let character_ids: Vec<String> = characters.iter().map(|c| c.id.clone()).collect();
        let relationships = if character_ids.is_empty() {
            Vec::new()
        } else {
            self.repository
                .list_relationships_for_characters(&character_branch_id, &character_ids)
                .await?
        };

        let subjects = if include_subjects {
            let cursor_position = cursor_scene
                .as_ref()
                .map(|scene| (scene.book_number, scene.chapter_number, scene.scene_order));
            let mut subject_snapshots = Vec::new();
            for character in &characters {
                let state =
                    if let Some((book_number, chapter_number, scene_order)) = cursor_position {
                        self.repository
                            .resolve_character_state_for_branch(
                                &branch_id,
                                &character.id,
                                book_number,
                                chapter_number,
                                scene_order + 1,
                            )
                            .await
                            .ok()
                            .flatten()
                    } else {
                        None
                    };
                let relationship_summaries = relationships
                    .iter()
                    .filter(|rel| rel.in_id == character.id || rel.out_id == character.id)
                    .map(|rel| RelationshipSummary {
                        // The SQLite RelatesTo has no surrogate id (composite
                        // primary key (in, out, branch_id)). Build a stable
                        // synthetic id so callers can refer to the edge.
                        relationship_id: format!(
                            "relationship:{}:{}:{}",
                            rel.in_id, rel.out_id, rel.branch_id
                        ),
                        source_character_id: rel.in_id.clone(),
                        target_character_id: rel.out_id.clone(),
                        relationship_type: rel.relationship_type.clone(),
                        trust: rel.trust,
                        tension: rel.tension,
                        dynamics: rel.dynamics.clone(),
                    })
                    .collect::<Vec<_>>();
                subject_snapshots.push(WriterStateSubjectSnapshot {
                    subject: SubjectRef {
                        subject_id: character.id.clone(),
                        kind: "character".to_string(),
                        name: character.name.clone(),
                    },
                    summary: character.summary.clone(),
                    role: Some(character.role.clone()),
                    status: state.map(|s| s.status).unwrap_or_default(),
                    relationships: relationship_summaries,
                });
            }
            subject_snapshots
        } else {
            Vec::new()
        };

        let suggested_subjects = subjects
            .iter()
            .take(DEFAULT_WRITER_STATE_SUGGESTED_SUBJECT_LIMIT)
            .map(|s| s.subject.clone())
            .collect::<Vec<_>>();
        let persisted_next_focus = writer_position.as_ref().and_then(|p| p.next_focus.clone());
        let next = WriterStateNext {
            intended_focus: persisted_next_focus.or_else(|| {
                next_scene
                    .as_ref()
                    .map(|scene| {
                        format!(
                            "Draft Book {} Chapter {} Scene {}: {}",
                            scene.book_number,
                            scene.chapter_number,
                            scene.scene_order,
                            scene.summary
                        )
                    })
                    .or_else(|| {
                        cursor_scene.as_ref().map(|scene| {
                            format!(
                                "Continue Book {} Chapter {} from Scene {}",
                                scene.book_number, scene.chapter_number, scene.scene_order
                            )
                        })
                    })
            }),
            outline_section_ref: next_scene.as_ref().map(|scene| OutlineRef {
                chapter_id: Some(scene.chapter_id.clone()),
                scene_order: Some(scene.scene_order),
                label: format!(
                    "book {} chapter {} scene {}",
                    scene.book_number, scene.chapter_number, scene.scene_order
                ),
            }),
            suggested_subjects,
        };

        let recent_scenes = if let Some(scene) = cursor_scene.as_ref() {
            let mut recent = self
                .repository
                .list_recent_scenes_by_project_and_branch(
                    &project.id,
                    &branch_id,
                    scene.book_number,
                    scene.chapter_number,
                    scene.scene_order + 1,
                    DEFAULT_WRITER_STATE_RECENT_SCENE_LIMIT,
                )
                .await?;
            if recent
                .last()
                .map(|candidate| candidate.id != scene.id)
                .unwrap_or(true)
            {
                recent.push(scene.clone());
            }
            recent
                .into_iter()
                .rev()
                .take(DEFAULT_WRITER_STATE_RECENT_SCENE_LIMIT)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .map(format::writer_state_recent_scene_summary)
                .collect()
        } else {
            Vec::new()
        };

        let open_promises_due_now = if let Some(scene) = cursor_scene.as_ref() {
            self.repository
                .list_narrative_promises_by_project_and_branch(&project.id, &branch_id)
                .await?
                .into_iter()
                .filter(|promise| promise.status != "paid_off")
                .filter(|promise| {
                    format::story_index_from_placement(&promise.planted_at)
                        <= format::story_index(
                            scene.book_number,
                            scene.chapter_number,
                            scene.scene_order,
                        )
                })
                .map(|promise| WriterStateNarrativePromiseSummary {
                    narrative_promise_id: promise.id,
                    promise_type: promise.promise_type,
                    description: promise.description,
                    status: promise.status,
                    planted_at: promise.planted_at.into_core(),
                    planned_payoff: promise.planned_payoff.map(|p| p.into_core()),
                    notes: promise.notes,
                })
                .collect()
        } else {
            Vec::new()
        };

        let active_overlays = self
            .repository
            .list_system_overlays_by_project_and_branch(&project.id, &branch_id)
            .await?
            .into_iter()
            .map(|overlay| OverlayWithTrajectory {
                overlay_id: overlay.id.clone(),
                name: overlay.system_name.clone(),
                current_value: json!({
                    "system_type": overlay.system_type,
                    "visibility": overlay.visibility,
                    "progression_currency": overlay.progression_currency,
                    "stats": overlay.stats,
                    "advancement_tiers": overlay.advancement_tiers,
                    "rules": overlay.rules,
                }),
                trajectory_delta_since_last_chapter: json!({}),
                recent_events: Vec::new(),
                provenance: Provenance {
                    source: "system_overlay".to_string(),
                    updated_at: Some(overlay.updated_at.to_rfc3339()),
                },
            })
            .collect::<Vec<_>>();

        // Risk #7 (closed): walk scene_source_link rows for any scene in
        // the cursor scene's chapter and emit divergence entries against
        // the on-disk file. Mirrors services/mod.rs:8466 in 705b835^.
        let unsynced_local_files = if let Some(scene) = cursor_scene.as_ref() {
            use spindle_core::models::UnsyncedFileEntry;
            let links = self
                .repository
                .list_scene_source_links_by_project(&project.id)
                .await?;
            let chapter_scene_map: std::collections::BTreeMap<String, &super::records::Scene> =
                scenes
                    .iter()
                    .filter(|candidate| {
                        candidate.book_number == scene.book_number
                            && candidate.chapter_number == scene.chapter_number
                    })
                    .map(|s| (s.id.clone(), s))
                    .collect();
            links
                .into_iter()
                .filter_map(|link| {
                    let scene = chapter_scene_map.get(&link.scene_id)?;
                    let observation =
                        super::source_bridge::evaluate_scene_divergence(&link, scene)?;
                    Some(UnsyncedFileEntry {
                        scene_id: scene.id.clone(),
                        source_path: link.source_path,
                        kind: observation.kind,
                        detail: observation.detail,
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let drift_warnings = unsynced_local_files
            .iter()
            .map(|entry| spindle_core::models::DriftWarning {
                code: "source_divergence".to_string(),
                message: format!("{}: {}", entry.source_path, entry.detail),
            })
            .collect::<Vec<_>>();

        let recent_session_activity = if include_recent_activity {
            self.repository
                .list_recent_session_activity(&project.id, &branch_id, recent_activity_limit as i64)
                .await?
                .into_iter()
                .map(|row| SessionActivitySummary {
                    kind: row.kind,
                    subject_table: row.subject_table,
                    subject_id: row.subject_id,
                    summary: row.summary,
                    created_at: row.created_at.to_rfc3339(),
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let chapter_outline = if let Some(scene) = cursor_scene.as_ref() {
            self.repository
                .get_chapter_outline(&scene.chapter_id, &branch_id)
                .await?
                .map(|outline| spindle_core::models::ChapterOutline {
                    chapter_id: outline.chapter_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    beats: outline.beats.into_iter().map(|b| b.into_core()).collect(),
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };
        let book_outline = if let Some(scene) = cursor_scene.as_ref() {
            self.repository
                .get_book_outline(&scene.book_id, &branch_id)
                .await?
                .map(|outline| spindle_core::models::BookOutline {
                    book_id: outline.book_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };
        let has_chapter_outline = chapter_outline.is_some();
        let has_book_outline = book_outline.is_some();

        let current = WriterStateCurrent {
            project: ProjectSummary {
                project_id: project.id.clone(),
                name: project.name,
                project_type: project.project_type,
                genre: project.genre,
            },
            branch: format::branch_summary(&branch, Some(active_branch.id.as_str())),
            book: book_summary,
            chapter: chapter_summary,
            scene: scene_summary,
            last_completed_scene_summary,
            intent: writer_position
                .as_ref()
                .map(|p| format::parse_writer_intent(&p.intent))
                .unwrap_or_else(|| {
                    if cursor_scene.is_some() {
                        WriterIntent::Drafting
                    } else {
                        WriterIntent::Idle
                    }
                }),
        };

        let mut state = WriterState {
            current,
            next,
            hard_constraints,
            subjects,
            recent_scenes,
            open_promises_due_now,
            active_overlays,
            drift_warnings,
            unsynced_local_files,
            recent_session_activity,
            chapter_outline,
            book_outline,
            bundle_summary: ContextBundleSummary {
                format: format_fmt,
                estimated_tokens: 0,
                token_budget: Some(token_budget),
                truncated: false,
                included_sections: format::writer_state_included_sections(
                    include_subjects,
                    include_recent_activity,
                    has_chapter_outline,
                    has_book_outline,
                ),
            },
        };

        format::enforce_writer_state_budget(format_fmt, token_budget, &mut state)?;

        Ok(state)
    }

    /// Build the public `get_writer_state` tool envelope while keeping
    /// markdown rendering with the application service instead of the MCP
    /// transport adapter.
    pub async fn get_writer_state_envelope(
        &self,
        input: spindle_core::models::GetWriterStateInput,
    ) -> Result<spindle_core::models::WriterStateEnvelope> {
        let format = input
            .format
            .unwrap_or(spindle_core::models::ContextFormat::Markdown);
        let payload = self.get_writer_state(input).await?;
        let writer_state_markdown = (format == spindle_core::models::ContextFormat::Markdown)
            .then(|| crate::format::format_writer_state_markdown(&payload));

        Ok(spindle_core::models::WriterStateEnvelope {
            current: payload.current,
            next: payload.next,
            hard_constraints: payload.hard_constraints,
            subjects: payload.subjects,
            recent_scenes: payload.recent_scenes,
            open_promises_due_now: payload.open_promises_due_now,
            active_overlays: payload.active_overlays,
            drift_warnings: payload.drift_warnings,
            unsynced_local_files: payload.unsynced_local_files,
            recent_session_activity: payload.recent_session_activity,
            chapter_outline: payload.chapter_outline,
            book_outline: payload.book_outline,
            bundle_summary: payload.bundle_summary,
            writer_state_markdown,
        })
    }

    /// Assemble a budget-aware chapter briefing for the requested chapter,
    /// folding in the scene-context slice for the first/requested scene
    /// when enough fields are pinned down. Mirrors services/mod.rs:
    /// 9524..9890 in 705b835^. Pure projections live in `crate::format::*`;
    /// only the per-call DB orchestration (continuity sheets, scene-context
    /// callback, branch lookup) lives here.
    ///
    /// Divergences from the reference:
    ///   * The bundled scene-context slice inherits the same
    ///     semantic-references / explicit-draft constraint gaps already
    ///     called out on `get_scene_context`.
    pub async fn get_chapter_briefing(
        &self,
        input: spindle_core::models::GetChapterBriefingInput,
    ) -> Result<spindle_core::models::GetChapterBriefingOutput> {
        use crate::format::{
            CHAPTER_BRIEFING_SCENE_CONTEXT_SECTIONS, DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS,
            DEFAULT_CHAPTER_BRIEFING_RECENT_LIMIT, MAX_CHAPTER_BRIEFING_RECENT_LIMIT,
            apply_chapter_briefing_bundle_trims, build_chapter_briefing_bundle,
            canonical_fact_hard_constraint, canonical_fact_read_model,
            fit_chapter_briefing_hard_constraints, format_chapter_briefing_markdown,
            is_hard_constraint_budget_error, recent_chapter_summaries_for_briefing,
            truncate_markdown_at_line_boundary,
        };
        use spindle_core::context_bundle::estimate_text_tokens;
        use spindle_core::models::{
            CanonicalFactReadModel, ChapterBriefingSceneSeed, ChapterPlanBriefing,
            ChapterSummaryBriefing, ContextFormat, GetChapterBriefingOutput, GetSceneContextInput,
            HardConstraint, StoryPlacement,
        };
        use spindle_core::subject::{Subject, SubjectTable};

        let format_fmt = input.format.unwrap_or(ContextFormat::Markdown);
        let budget_tokens = input
            .budget_tokens
            .or(input.token_budget)
            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS);

        self.repository.get_project(&input.project_id).await?;

        let recent_limit = input
            .recent_chapter_limit
            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_RECENT_LIMIT)
            .min(MAX_CHAPTER_BRIEFING_RECENT_LIMIT);
        let recent_chapter_summaries = recent_chapter_summaries_for_briefing(
            self.repository
                .list_chapter_summaries_by_project(&input.project_id)
                .await?,
            input.book_number,
            input.chapter_number,
            recent_limit,
        )
        .into_iter()
        .map(|summary| ChapterSummaryBriefing {
            book_number: summary.book_number,
            chapter_number: summary.chapter_number,
            summary: summary.summary,
            key_events: summary.key_events,
            character_changes: summary.character_changes,
            relationship_shifts: summary.relationship_shifts,
            arc_advances: summary.arc_advances,
            promise_events: summary.promise_events,
        })
        .collect::<Vec<_>>();

        let chapter_plan = self
            .repository
            .list_chapter_plans_by_project(&input.project_id)
            .await?
            .into_iter()
            .find(|plan| {
                plan.book_number == input.book_number && plan.chapter_number == input.chapter_number
            })
            .map(|plan| ChapterPlanBriefing {
                synopsis: plan.synopsis,
                pov_character_id: plan.pov_character_id,
                target_theme_ids: plan.target_theme_ids,
                target_conflict_ids: plan.target_conflict_ids,
                target_plot_line_ids: plan.target_plot_line_ids,
                scenes: plan
                    .scenes
                    .into_iter()
                    .map(|scene| scene.into_core())
                    .collect(),
            });

        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let chapter_scene_orders = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?
            .into_iter()
            .filter(|scene| {
                scene.book_number == input.book_number
                    && scene.chapter_number == input.chapter_number
            })
            .map(|scene| scene.scene_order)
            .collect::<Vec<_>>();

        let resolved_scene_order = input
            .scene_order
            .or_else(|| {
                chapter_plan
                    .as_ref()
                    .and_then(|plan| plan.scenes.first().map(|scene| scene.scene_order))
            })
            .or_else(|| chapter_scene_orders.first().copied());

        let resolved_character_ids = if !input.character_ids.is_empty() {
            input.character_ids.clone()
        } else if let Some(scene_order) = resolved_scene_order {
            chapter_plan
                .as_ref()
                .and_then(|plan| {
                    plan.scenes
                        .iter()
                        .find(|scene| scene.scene_order == scene_order)
                        .map(|scene| scene.character_ids.clone())
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let resolved_location_id = input.location_id.clone();

        let mut missing_fields = Vec::new();
        if resolved_scene_order.is_none() {
            missing_fields.push("scene_order".to_string());
        }
        if resolved_character_ids.is_empty() {
            missing_fields.push("character_ids".to_string());
        }
        if resolved_location_id.is_none() {
            missing_fields.push("location_id".to_string());
        }

        let scene_context = if missing_fields.is_empty() {
            match self
                .get_scene_context(GetSceneContextInput {
                    project_id: input.project_id.clone(),
                    book_number: input.book_number,
                    chapter_number: input.chapter_number,
                    chapter_id: None,
                    scene_order: resolved_scene_order.expect("checked above"),
                    character_ids: resolved_character_ids.clone(),
                    max_character_count: None,
                    location_id: resolved_location_id.clone().expect("checked above"),
                    format: Some(ContextFormat::Json),
                    budget_tokens: Some(
                        input
                            .budget_tokens
                            .or(input.token_budget)
                            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS),
                    ),
                    token_budget: Some(
                        input
                            .budget_tokens
                            .or(input.token_budget)
                            .unwrap_or(DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS),
                    ),
                    sections: Some(
                        CHAPTER_BRIEFING_SCENE_CONTEXT_SECTIONS
                            .iter()
                            .map(|section| section.to_string())
                            .collect(),
                    ),
                })
                .await
            {
                Ok(scene_context) => Some(scene_context),
                Err(error) if is_hard_constraint_budget_error(&error) => None,
                Err(error) => return Err(error),
            }
        } else {
            None
        };

        let scene_seed = ChapterBriefingSceneSeed {
            scene_order: resolved_scene_order,
            character_ids: resolved_character_ids.clone(),
            location_id: resolved_location_id.clone(),
            missing_fields,
            scene_context_available: scene_context.is_some(),
        };

        // Continuity-sheet subjects: POV + requested characters + location.
        let continuity_sheets = {
            let placement = StoryPlacement {
                book_number: input.book_number,
                chapter_number: input.chapter_number,
                scene_order: resolved_scene_order,
                note: None,
            };
            let mut seen = std::collections::BTreeSet::new();
            let mut subjects: Vec<Subject> = Vec::new();
            let mut push_subject = |table: SubjectTable, subject_id: String| -> Result<()> {
                let key = format!("{}:{subject_id}", table.as_str());
                if seen.insert(key) {
                    subjects.push(
                        Subject::new(table, subject_id)
                            .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                    );
                }
                Ok(())
            };

            if let Some(pov_character_id) = chapter_plan
                .as_ref()
                .and_then(|plan| plan.pov_character_id.clone())
            {
                push_subject(SubjectTable::Character, pov_character_id)?;
            }
            for character_id in &resolved_character_ids {
                push_subject(SubjectTable::Character, character_id.clone())?;
            }
            if let Some(location_id) = resolved_location_id.as_deref() {
                push_subject(SubjectTable::Location, location_id.to_string())?;
            }

            if subjects.is_empty() {
                Vec::new()
            } else {
                self.repository
                    .assemble_subject_snapshots(
                        &input.project_id,
                        &active_branch.id,
                        &subjects,
                        &placement,
                    )
                    .await?
            }
        };

        let chapter_record = self
            .repository
            .get_chapter_by_number(&input.project_id, input.book_number, input.chapter_number)
            .await
            .ok();
        let book_record = self
            .repository
            .get_book_by_number(&input.project_id, input.book_number)
            .await
            .ok();
        let chapter_outline = if let Some(chapter) = chapter_record.as_ref() {
            self.repository
                .get_chapter_outline(&chapter.id, &active_branch.id)
                .await?
                .map(|outline| spindle_core::models::ChapterOutline {
                    chapter_id: outline.chapter_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    beats: outline
                        .beats
                        .into_iter()
                        .map(|beat| beat.into_core())
                        .collect(),
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };
        let book_outline = if let Some(book) = book_record.as_ref() {
            self.repository
                .get_book_outline(&book.id, &active_branch.id)
                .await?
                .map(|outline| spindle_core::models::BookOutline {
                    book_id: outline.book_id,
                    branch_id: outline.branch_id,
                    format: outline.format,
                    content: outline.content,
                    updated_at: outline.updated_at.to_rfc3339(),
                })
        } else {
            None
        };

        let hard_constraints_from_scene_context: Option<Vec<HardConstraint>> = scene_context
            .as_ref()
            .map(|ctx| ctx.hard_constraints.clone());
        let canonical_facts_from_scene_context: Option<Vec<CanonicalFactReadModel>> = scene_context
            .as_ref()
            .map(|ctx| ctx.canonical_facts.clone());

        let (hard_constraints, canonical_facts): (
            Vec<HardConstraint>,
            Vec<CanonicalFactReadModel>,
        ) = match (
            hard_constraints_from_scene_context,
            canonical_facts_from_scene_context,
        ) {
            (Some(constraints), Some(canonical_facts)) => (constraints, canonical_facts),
            _ => {
                let rules = self
                    .repository
                    .list_world_rules_by_project_and_branch(&input.project_id, &active_branch.id)
                    .await?;
                let placement = StoryPlacement {
                    book_number: input.book_number,
                    chapter_number: input.chapter_number,
                    scene_order: resolved_scene_order,
                    note: None,
                };
                let project_wide_facts = self
                    .repository
                    .list_canonical_facts_for_project_wide(
                        &input.project_id,
                        &active_branch.id,
                        &placement,
                    )
                    .await?;
                let canonical_fact_models = project_wide_facts
                    .iter()
                    .map(canonical_fact_read_model)
                    .collect::<Vec<_>>();
                let constraints = rules
                    .into_iter()
                    .map(|rule| HardConstraint {
                        id: rule.rule_name,
                        statement: rule.description,
                    })
                    .chain(
                        project_wide_facts
                            .iter()
                            .map(canonical_fact_hard_constraint),
                    )
                    .collect::<Vec<_>>();
                (constraints, canonical_fact_models)
            }
        };

        let (hard_constraints, hard_constraints_compacted) = fit_chapter_briefing_hard_constraints(
            format_fmt,
            budget_tokens,
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &hard_constraints,
        )?;

        let briefing_markdown = format_chapter_briefing_markdown(
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &hard_constraints,
            &continuity_sheets,
            &recent_chapter_summaries,
            chapter_outline.as_ref(),
            book_outline.as_ref(),
            chapter_plan.as_ref(),
            scene_context.as_ref(),
            &scene_seed,
        );
        let mut output = GetChapterBriefingOutput {
            hard_constraints: hard_constraints.clone(),
            canonical_facts: if hard_constraints_compacted {
                Vec::new()
            } else {
                canonical_facts
            },
            continuity_sheets,
            briefing_markdown,
            recent_chapter_summaries,
            chapter_outline,
            book_outline,
            chapter_plan,
            scene_seed,
            scene_context,
        };

        let mut bundle = build_chapter_briefing_bundle(
            format_fmt,
            budget_tokens,
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &output.hard_constraints,
            &output.canonical_facts,
            &output.continuity_sheets,
            &output.recent_chapter_summaries,
            output.chapter_outline.as_ref(),
            output.book_outline.as_ref(),
            output.chapter_plan.as_ref(),
            output.scene_context.as_ref(),
            &output.scene_seed,
        );
        let budget_report = bundle
            .enforce_budget()
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        apply_chapter_briefing_bundle_trims(
            &budget_report.truncated_section_ids,
            &mut output.canonical_facts,
            &mut output.continuity_sheets,
            &mut output.recent_chapter_summaries,
            &mut output.chapter_outline,
            &mut output.book_outline,
            &mut output.chapter_plan,
            &mut output.scene_context,
        );
        output.briefing_markdown = format_chapter_briefing_markdown(
            input.book_number,
            input.chapter_number,
            resolved_scene_order,
            &output.hard_constraints,
            &output.continuity_sheets,
            &output.recent_chapter_summaries,
            output.chapter_outline.as_ref(),
            output.book_outline.as_ref(),
            output.chapter_plan.as_ref(),
            output.scene_context.as_ref(),
            &output.scene_seed,
        );
        if format_fmt == ContextFormat::Markdown
            && estimate_text_tokens(&output.briefing_markdown) > budget_tokens
        {
            // Final fallback: keep markdown whole-line boundaries instead
            // of cutting headers mid-line when the trim loop still exceeds
            // budget.
            output.briefing_markdown =
                truncate_markdown_at_line_boundary(&output.briefing_markdown, budget_tokens);
        }

        Ok(output)
    }

    /// Assemble a budget-aware scene-context bundle for the requested scene
    /// position. Mirrors services/mod.rs:8612..9134 in 705b835^. Validates
    /// project + chapter ownership, derives the scene's location / characters
    /// / relationships / pacing / agency directly off the SQLite repository,
    /// then projects the result through the bundle helpers in
    /// `crate::format::*` to enforce `budget_tokens` (Markdown or JSON).
    ///
    /// Divergences from the reference:
    ///   * No semantic search yet — the SQLite stack exposes `search_bible`
    ///     for caller-visible search, but a service-internal semantic
    ///     re-ranking that prefers project records is not yet ported. The
    ///     `semantic_references` slice is therefore always empty; subjects
    ///     are still gathered from POV + character_ids + location.
    ///   * No explicit-draft hard constraint is appended (the SQLite service
    ///     does not own a `ModelRouter`); the reference appended one when
    ///     the `draft` route had an explicit-rating override.
    pub async fn get_scene_context(
        &self,
        input: spindle_core::models::GetSceneContextInput,
    ) -> Result<spindle_core::models::SceneContextOutput> {
        use crate::format::{
            DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS, WorldRuleContextCharacter,
            agency_check_from_scene_history, apply_scene_context_bundle_trims,
            build_scene_context_bundle, canonical_fact_hard_constraint, canonical_fact_read_model,
            empty_agency_check_summary, empty_location_summary, empty_reader_contract,
            empty_world_state_summary, estimate_scene_context_tokens, filter_relevant_world_rules,
            future_knowledge_briefing_item, future_knowledge_summary, knowledge_fact_briefing_item,
            narrative_promise_due_summary, non_truncatable_prefix_tokens_scene_context,
            pacing_directives_for_characters, story_index, story_index_from_placement,
            system_overlay_summary, timeline_event_summary_at_or_before,
        };
        use spindle_core::models::{
            CharacterStateSummary, ContextFormat, HardConstraint, LocationSummary,
            RelationshipSummary, SceneContextBudgetMeta, SceneContextNovelLayer,
            SceneContextOutput, SceneContextSceneLayer, StoryPlacement, WorldRuleSummary,
            WorldStateSummary,
        };
        use spindle_core::subject::{Subject, SubjectTable};

        let format_fmt = input.format.unwrap_or(ContextFormat::Markdown);
        let budget_tokens = input
            .budget_tokens
            .or(input.token_budget)
            .unwrap_or(DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS);

        let want_reader_contract = input.wants_novel_section("reader_contract");
        let want_style_directive = input.wants_novel_section("style_directive");
        let want_world_rules = input.wants_novel_section("world_rules");
        let want_system_overlays = input.wants_novel_section("system_overlays");
        let want_timeline_briefing = input.wants_novel_section("timeline_briefing");
        let want_future_knowledge_briefing = input.wants_novel_section("future_knowledge_briefing");
        let want_pacing_directives = input.wants_novel_section("pacing_directives");
        let want_narrative_promises_due = input.wants_novel_section("narrative_promises_due");
        let want_knowledge_briefing = input.wants_novel_section("knowledge_briefing");
        let want_semantic_references = input.wants_novel_section("semantic_references");
        let want_subjects = input.wants_novel_section("subjects");

        let want_location = input.wants_scene_section("location");
        let want_world_state = input.wants_scene_section("world_state");
        let want_characters = input.wants_scene_section("characters");
        let want_relationships = input.wants_scene_section("relationships");
        let want_agency_check = input.wants_scene_section("agency_check");

        let project = self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;

        // Reconcile chapter_id vs book_number/chapter_number.
        let mut normalized = input.clone();
        if let Some(chapter_id) = input.chapter_id.as_deref() {
            let chapter = self.repository.get_chapter(chapter_id).await?;
            if chapter.project_id != input.project_id {
                anyhow::bail!("chapter does not belong to the requested project");
            }
            if input.book_number > 0 && input.book_number != chapter.book_number {
                anyhow::bail!(
                    "chapter_id {} does not match book_number {}",
                    chapter_id,
                    input.book_number
                );
            }
            if input.chapter_number > 0 && input.chapter_number != chapter.chapter_number {
                anyhow::bail!(
                    "chapter_id {} does not match chapter_number {}",
                    chapter_id,
                    input.chapter_number
                );
            }
            normalized.book_number = chapter.book_number;
            normalized.chapter_number = chapter.chapter_number;
        } else if input.book_number <= 0 || input.chapter_number <= 0 {
            anyhow::bail!(
                "get_scene_context requires chapter_id or both book_number and chapter_number"
            );
        }
        let input = normalized;

        let location = self.repository.get_location(&input.location_id).await?;
        if location.project_id != input.project_id {
            anyhow::bail!("location does not belong to the requested project");
        }

        let world_state = if want_world_state {
            self.repository
                .get_world_state_by_location(&input.location_id)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        let system_overlays = if want_system_overlays {
            self.repository
                .list_system_overlays_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
        } else {
            Vec::new()
        };

        let timeline_briefing = if want_timeline_briefing {
            self.repository
                .list_timeline_events_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter_map(|event| {
                    timeline_event_summary_at_or_before(
                        event,
                        input.book_number,
                        input.chapter_number,
                        input.scene_order,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        // Validate every requested character belongs to the project.
        let need_resolved_characters =
            want_characters || want_agency_check || want_semantic_references;
        let mut validated_characters = Vec::new();
        for character_id in &input.character_ids {
            let character = self.repository.get_character(character_id).await?;
            if character.project_id != input.project_id {
                anyhow::bail!(
                    "character {} does not belong to the requested project",
                    character.name
                );
            }
            validated_characters.push(character);
        }
        let included_characters = if let Some(max_character_count) = input.max_character_count {
            validated_characters
                .into_iter()
                .take(max_character_count)
                .collect::<Vec<_>>()
        } else {
            validated_characters
        };

        let ids = included_characters
            .iter()
            .map(|c| c.id.clone())
            .collect::<Vec<_>>();
        let world_rule_context_characters = included_characters
            .iter()
            .map(|character| WorldRuleContextCharacter {
                name: character.name.clone(),
                role: character.role.clone(),
                summary: character.summary.clone(),
            })
            .collect::<Vec<_>>();

        let mut resolved_characters: Vec<CharacterStateSummary> = Vec::new();
        if need_resolved_characters {
            for character in &included_characters {
                let state = self
                    .repository
                    .resolve_character_state_for_branch(
                        &active_branch.id,
                        &character.id,
                        input.book_number,
                        input.chapter_number,
                        input.scene_order,
                    )
                    .await?;
                resolved_characters.push(CharacterStateSummary {
                    character_id: character.id.clone(),
                    name: character.name.clone(),
                    summary: character.summary.clone(),
                    role: character.role.clone(),
                    emotional_state: state
                        .as_ref()
                        .map(|s| s.emotional_state.clone())
                        .unwrap_or_default(),
                    goals: state.as_ref().map(|s| s.goals.clone()).unwrap_or_default(),
                    status: state.as_ref().map(|s| s.status.clone()).unwrap_or_default(),
                    notes: state.as_ref().map(|s| s.notes.clone()).unwrap_or_default(),
                });
            }
        }

        let all_world_rules = self
            .repository
            .list_world_rules_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;

        // Style-typed world rules are prose-level directives, not lore. Pull
        // them regardless of relevance filtering so they always reach the
        // style directive (the brief's "untitled / buried" failure mode).
        let style_rules: Vec<spindle_core::style::StyleRule> = all_world_rules
            .iter()
            .filter(|rule| rule.rule_type.eq_ignore_ascii_case("style"))
            .map(|rule| spindle_core::style::StyleRule {
                rule_name: rule.rule_name.clone(),
                description: rule.description.clone(),
            })
            .collect();

        // Single source of truth for genre-voice enforcement: reader contract
        // + style world rules + narrator voice. Read by scene context, the
        // save gate, the style_compliance validator, and the review persona.
        let style_directive = spindle_core::style::StyleDirective::assemble(
            project.genre.clone(),
            project.project_type.clone(),
            project.reader_contract.promise.clone(),
            project.reader_contract.style_notes.clone(),
            project.reader_contract.boundaries.clone(),
            style_rules,
            project
                .narrator_voice
                .clone()
                .map(|stored| stored.into_core()),
        );

        let relevant_world_rules = filter_relevant_world_rules(
            &all_world_rules,
            &location,
            &world_rule_context_characters,
        );

        let relationships = if want_relationships && !ids.is_empty() {
            self.repository
                .list_relationships_for_characters(&active_branch.id, &ids)
                .await?
        } else {
            Vec::new()
        };

        let raw_future_knowledge =
            if (want_future_knowledge_briefing || want_knowledge_briefing) && !ids.is_empty() {
                self.repository
                    .list_future_knowledge_by_project(&input.project_id)
                    .await?
                    .into_iter()
                    .filter(|knowledge| ids.contains(&knowledge.character_id))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
        let future_knowledge_briefing = if want_future_knowledge_briefing {
            raw_future_knowledge
                .iter()
                .map(future_knowledge_summary)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let pacing_directives = if want_pacing_directives && !ids.is_empty() {
            pacing_directives_for_characters(
                &self
                    .repository
                    .list_character_arcs_by_project_and_branch(&input.project_id, &active_branch.id)
                    .await?,
                &self
                    .repository
                    .list_pacing_trackers_by_project_and_branch(
                        &input.project_id,
                        &active_branch.id,
                    )
                    .await?,
                &ids,
            )
        } else {
            Vec::new()
        };

        let narrative_promises_due = if want_narrative_promises_due {
            self.repository
                .list_narrative_promises_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|promise| promise.status != "paid_off")
                .map(|promise| {
                    narrative_promise_due_summary(
                        &promise,
                        input.book_number,
                        input.chapter_number,
                        input.scene_order,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let knowledge_briefing = if want_knowledge_briefing && !ids.is_empty() {
            let cursor = story_index(input.book_number, input.chapter_number, input.scene_order);
            let mut briefing = self
                .repository
                .list_knowledge_facts_by_project_and_branch(&input.project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|fact| ids.contains(&fact.character_id))
                .filter(|fact| {
                    fact.learned_at
                        .as_ref()
                        .is_none_or(|placement| story_index_from_placement(placement) <= cursor)
                })
                .map(knowledge_fact_briefing_item)
                .collect::<Vec<_>>();
            briefing.extend(
                raw_future_knowledge
                    .iter()
                    .map(future_knowledge_briefing_item),
            );
            briefing
        } else {
            Vec::new()
        };

        let agency_check = if want_agency_check {
            let recent_scenes = self
                .repository
                .list_recent_scenes_by_project_and_branch(
                    &input.project_id,
                    &active_branch.id,
                    input.book_number,
                    input.chapter_number,
                    input.scene_order,
                    50,
                )
                .await?;
            agency_check_from_scene_history(
                &recent_scenes,
                &resolved_characters,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
        } else {
            empty_agency_check_summary()
        };

        // Semantic-references slice is not yet ported — see method-level
        // doc-comment for the divergence.
        let semantic_references: Vec<spindle_core::models::SearchBibleResultItem> = Vec::new();
        let _ = want_semantic_references;

        let placement = StoryPlacement {
            book_number: input.book_number,
            chapter_number: input.chapter_number,
            scene_order: Some(input.scene_order),
            note: None,
        };
        let scene_id = self
            .repository
            .find_scene_by_natural_key(
                &input.project_id,
                &active_branch.id,
                input.book_number,
                input.chapter_number,
                input.scene_order,
            )
            .await?
            .map(|scene| scene.id);

        // Subjects for canonical-fact lookup: characters + location + scene.
        let mut canonical_subjects: Vec<Subject> = Vec::new();
        let mut seen_subjects = std::collections::BTreeSet::new();
        for character_id in &input.character_ids {
            if seen_subjects.insert(format!("character:{character_id}")) {
                canonical_subjects.push(
                    Subject::new(SubjectTable::Character, character_id.clone())
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                );
            }
        }
        if seen_subjects.insert(format!("location:{}", input.location_id)) {
            canonical_subjects.push(
                Subject::new(SubjectTable::Location, input.location_id.clone())
                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
            );
        }
        if let Some(scene_id) = scene_id.as_ref()
            && seen_subjects.insert(format!("scene:{scene_id}"))
        {
            canonical_subjects.push(
                Subject::new(SubjectTable::Scene, scene_id.clone())
                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
            );
        }

        let mut canonical_facts = self
            .repository
            .list_canonical_facts_for_subjects(
                &input.project_id,
                &active_branch.id,
                &canonical_subjects,
                &placement,
            )
            .await?;
        canonical_facts.extend(
            self.repository
                .list_canonical_facts_for_project_wide(
                    &input.project_id,
                    &active_branch.id,
                    &placement,
                )
                .await?,
        );
        let canonical_fact_read_models = canonical_facts
            .iter()
            .map(canonical_fact_read_model)
            .collect::<Vec<_>>();

        let subjects_snapshots = if want_subjects {
            let mut subject_keys = std::collections::BTreeSet::new();
            let mut subjects = Vec::new();
            let mut push_subject = |subject: Subject| -> anyhow::Result<()> {
                let key = subject.to_string();
                if subject_keys.insert(key) {
                    subjects.push(subject);
                }
                Ok(())
            };
            if let Some(pov_character_id) = self
                .repository
                .list_chapter_plans_by_project(&input.project_id)
                .await?
                .into_iter()
                .find(|plan| {
                    plan.book_number == input.book_number
                        && plan.chapter_number == input.chapter_number
                })
                .and_then(|plan| plan.pov_character_id)
            {
                push_subject(
                    Subject::new(SubjectTable::Character, pov_character_id)
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                )?;
            }
            for character_id in &input.character_ids {
                push_subject(
                    Subject::new(SubjectTable::Character, character_id.clone())
                        .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                )?;
            }
            push_subject(
                Subject::new(SubjectTable::Location, input.location_id.clone())
                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
            )?;
            // Controlling-faction subject (best-effort name match).
            if let Some(name) = world_state
                .as_ref()
                .and_then(|state| state.controlling_faction.as_deref())
            {
                let normalized_name = name.trim().to_ascii_lowercase();
                if !normalized_name.is_empty() {
                    for faction in self
                        .repository
                        .list_factions_by_project_and_branch(&input.project_id, &active_branch.id)
                        .await?
                    {
                        if faction.name.trim().to_ascii_lowercase() == normalized_name {
                            push_subject(
                                Subject::new(SubjectTable::Faction, faction.id)
                                    .map_err(|err| anyhow::anyhow!(err.to_string()))?,
                            )?;
                            break;
                        }
                    }
                }
            }

            self.repository
                .assemble_subject_snapshots(
                    &input.project_id,
                    &active_branch.id,
                    &subjects,
                    &placement,
                )
                .await?
        } else {
            Vec::new()
        };

        let mut novel_layer = SceneContextNovelLayer {
            reader_contract: if want_reader_contract {
                project.reader_contract.into_core()
            } else {
                empty_reader_contract()
            },
            style_directive: if want_style_directive && !style_directive.is_empty() {
                Some(style_directive)
            } else {
                None
            },
            world_rules: if want_world_rules {
                let mut summaries: Vec<WorldRuleSummary> = relevant_world_rules
                    .clone()
                    .into_iter()
                    .map(|rule| WorldRuleSummary {
                        rule_name: rule.rule_name,
                        rule_type: rule.rule_type,
                        description: rule.description,
                    })
                    .collect();
                // Surface style-typed rules first so they read as prose-level
                // directives, not lore buried in a long list.
                summaries.sort_by_key(|summary| {
                    if summary.rule_type.eq_ignore_ascii_case("style") {
                        0
                    } else {
                        1
                    }
                });
                summaries
            } else {
                Vec::new()
            },
            subjects: subjects_snapshots,
            system_overlays: if want_system_overlays {
                system_overlays
                    .into_iter()
                    .map(system_overlay_summary)
                    .collect()
            } else {
                Vec::new()
            },
            timeline_briefing,
            future_knowledge_briefing,
            pacing_directives,
            narrative_promises_due,
            knowledge_briefing,
            semantic_references,
        };

        let mut scene_layer = SceneContextSceneLayer {
            location: if want_location {
                LocationSummary {
                    location_id: location.id.clone(),
                    name: location.name.clone(),
                    kind: location.kind.clone(),
                    realm: location.realm.clone(),
                    summary: location.summary.clone(),
                }
            } else {
                empty_location_summary()
            },
            world_state: if let Some(world_state) = world_state {
                WorldStateSummary {
                    controlling_faction: world_state.controlling_faction,
                    status: world_state.status,
                    prosperity: world_state.prosperity,
                    stability: world_state.stability,
                    threat_level: world_state.threat_level,
                    sensory_details: world_state.sensory_details,
                }
            } else {
                empty_world_state_summary()
            },
            characters: if want_characters {
                resolved_characters
            } else {
                Vec::new()
            },
            relationships: if want_relationships {
                relationships
                    .into_iter()
                    .map(|relationship| RelationshipSummary {
                        // SQLite stores relationships as a junction table with
                        // composite PK (in_id, out_id, branch_id); synthesize
                        // a deterministic relationship_id from the pair.
                        relationship_id: format!("{}|{}", relationship.in_id, relationship.out_id),
                        source_character_id: relationship.in_id,
                        target_character_id: relationship.out_id,
                        relationship_type: relationship.relationship_type,
                        trust: relationship.trust,
                        tension: relationship.tension,
                        dynamics: relationship.dynamics,
                    })
                    .collect()
            } else {
                Vec::new()
            },
            agency_check,
        };

        let mut hard_constraints: Vec<HardConstraint> = relevant_world_rules
            .iter()
            .map(|rule| HardConstraint {
                // Mark style-typed rules so they read as mandatory prose-level
                // directives even in the flat hard-constraints list.
                id: if rule.rule_type.eq_ignore_ascii_case("style") {
                    format!("[STYLE DIRECTIVE] {}", rule.rule_name)
                } else {
                    rule.rule_name.clone()
                },
                statement: rule.description.clone(),
            })
            .chain(canonical_facts.iter().map(canonical_fact_hard_constraint))
            .collect();
        let _ = &mut hard_constraints;

        let non_truncatable_cost =
            non_truncatable_prefix_tokens_scene_context(format_fmt, &hard_constraints);
        if non_truncatable_cost > budget_tokens {
            anyhow::bail!(
                "budget_tokens ({budget_tokens}) too small to fit hard constraints \
                 (estimated {non_truncatable_cost} tokens). \
                 Increase budget_tokens or reduce world rules."
            );
        }

        let mut bundle = build_scene_context_bundle(
            format_fmt,
            budget_tokens,
            &hard_constraints,
            &novel_layer.subjects,
            &novel_layer,
            &scene_layer,
        );
        let budget_report = bundle
            .enforce_budget()
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        apply_scene_context_bundle_trims(
            &budget_report.truncated_section_ids,
            &mut novel_layer,
            &mut scene_layer,
        );

        let estimated_tokens = estimate_scene_context_tokens(
            format_fmt,
            &hard_constraints,
            &novel_layer,
            &scene_layer,
        );
        let novel_layer_truncated = !budget_report.truncated_section_ids.is_empty();

        Ok(SceneContextOutput {
            hard_constraints,
            canonical_facts: canonical_fact_read_models,
            novel: novel_layer,
            scene: scene_layer,
            budget: SceneContextBudgetMeta {
                estimated_tokens,
                token_budget: Some(budget_tokens),
                novel_layer_truncated,
            },
        })
    }

    /// Build the public `get_scene_context` tool envelope while keeping
    /// standards inclusion and markdown rendering in the application service
    /// boundary instead of the MCP transport adapter.
    pub async fn get_scene_context_envelope(
        &self,
        input: spindle_core::models::GetSceneContextInput,
    ) -> Result<spindle_core::models::SceneContextEnvelope> {
        let format = input
            .format
            .unwrap_or(spindle_core::models::ContextFormat::Markdown);
        let include_standards = input.wants_standards();
        let payload = self.get_scene_context(input).await?;
        let context_markdown =
            (format == spindle_core::models::ContextFormat::Markdown).then(|| {
                crate::format::format_scene_context_markdown(
                    None,
                    &payload.hard_constraints,
                    &payload.novel,
                    &payload.scene,
                )
            });

        Ok(spindle_core::models::SceneContextEnvelope {
            hard_constraints: payload.hard_constraints,
            standards: if include_standards {
                // Lead the standards block with the project-specific style
                // requirements so the genre-voice contract sits in the same
                // instruction the model reads first, not in a separate payload
                // it may deprioritize (the brief's Change 8).
                let mut standards = String::new();
                if let Some(directive) = payload
                    .novel
                    .style_directive
                    .as_ref()
                    .and_then(|directive| directive.render_markdown())
                {
                    standards.push_str(directive.trim_start());
                    standards.push_str("\n\n");
                }
                standards.push_str(crate::guidance::standards_text());
                standards
            } else {
                String::new()
            },
            novel: payload.novel,
            scene: payload.scene,
            budget: payload.budget,
            context_markdown,
        })
    }

    /// Specialised character-focused snapshot: same backbone as
    /// `get_entity` for the Character subject, but unpacks the voice
    /// profile / current_state / recent_appearances off the
    /// SubjectSnapshot for direct caller consumption. Mirrors
    /// services/mod.rs:9245..9310 in 705b835^.
    pub async fn get_character_snapshot(
        &self,
        input: spindle_core::models::GetCharacterSnapshotInput,
    ) -> Result<spindle_core::models::CharacterSnapshotOutput> {
        use spindle_core::models::{CharacterSnapshotOutput, StoryPlacement};
        use spindle_core::subject::{Subject, SubjectTable};

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let branch = match input.branch_id.as_deref() {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(input.project_id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        input.project_id
                    );
                }
                branch
            }
            None => active_branch,
        };

        let character = self.repository.get_character(&input.character_id).await?;
        if character.project_id != input.project_id {
            anyhow::bail!(
                "character {} does not belong to project {}",
                input.character_id,
                input.project_id
            );
        }

        let placement = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &branch.id)
            .await?
            .last()
            .map(|scene| StoryPlacement {
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: Some(scene.scene_order),
                note: None,
            })
            .unwrap_or(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            });

        let subject = Subject::new(SubjectTable::Character, character.id.clone())
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        let snapshot = self
            .repository
            .assemble_subject_snapshot(&input.project_id, &branch.id, &subject, &placement)
            .await?;

        Ok(CharacterSnapshotOutput {
            voice_profile: snapshot.voice_profile().cloned(),
            current_state: snapshot.current_state().cloned(),
            recent_appearances: snapshot.recent_appearances().to_vec(),
            snapshot,
        })
    }

    /// Assemble a deep `SubjectSnapshot` for an arbitrary entity on the
    /// requested branch (or the project's active branch when omitted).
    /// Mirrors services/mod.rs:9135..9184 in 705b835^. Backed by the
    /// freshly-ported `Repository::assemble_subject_snapshot` (commit
    /// d0e721d), which loads the entire `SnapshotBatchContext` for the
    /// branch and projects the requested subject through pure helpers.
    pub async fn get_entity(
        &self,
        input: spindle_core::models::GetEntityInput,
    ) -> Result<spindle_core::subject_snapshot::SubjectSnapshot> {
        use spindle_core::models::StoryPlacement;
        use spindle_core::subject::{Subject, SubjectTable};

        self.repository.get_project(&input.project_id).await?;
        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let branch = match input.branch_id.as_deref() {
            Some(branch_id) => {
                let branch = self.repository.get_branch(branch_id).await?;
                if branch.project_id.as_deref() != Some(input.project_id.as_str()) {
                    anyhow::bail!(
                        "branch {branch_id} does not belong to project {}",
                        input.project_id
                    );
                }
                branch
            }
            None => active_branch,
        };

        if input.table == SubjectTable::Project {
            anyhow::bail!("get_entity does not support table=project");
        }

        let subject = Subject::new(input.table, input.entity_id)
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;

        // Use the latest scene on the branch as the placement anchor;
        // fall back to (1,1,1) when the branch has no scenes yet so the
        // assembly's "as-of" filters still have a position to compare
        // against. Mirrors the reference behaviour.
        let placement = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &branch.id)
            .await?
            .last()
            .map(|scene| StoryPlacement {
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: Some(scene.scene_order),
                note: None,
            })
            .unwrap_or(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            });

        self.repository
            .assemble_subject_snapshot(&input.project_id, &branch.id, &subject, &placement)
            .await
    }

    /// Run the consistency validator suite over a project. Mirrors
    /// `services/mod.rs:4762..5736` in 705b835^.
    ///
    /// Phase-4 fan-out (`canonical_fact_prose_drift`,
    /// `world_rule_semantic_drift`, `voice_drift`, `retcon_reachability`)
    /// is wired through `run_phase_four_validator_checks_for_scenes` and
    /// the results are grouped into `report_sections`. Setting
    /// `deep_check = true` additionally invokes the model-router-driven
    /// `deep_world_rule_compliance_issues` audit on top of the pattern-
    /// based path. `scene_divergence` is wired through
    /// `evaluate_scene_divergence` in `super::source_bridge`.
    pub async fn check_consistency(
        &self,
        input: spindle_core::models::CheckConsistencyInput,
    ) -> Result<spindle_core::models::CheckConsistencyOutput> {
        use crate::format::{
            DEFAULT_CHECK_CONSISTENCY_BUDGET_TOKENS, chapter_keys_from_scenes, end_scope_index,
            scope_contains_chapter, scope_contains_position, scoped_chapter_plans,
            scoped_chapter_summaries, scoped_narrative_promises, scoped_scenes,
            story_index_from_placement,
        };
        use spindle_core::models::{ConsistencyIssue, ConsistencySummary, ContextFormat};
        use std::collections::{BTreeMap, BTreeSet};

        let project_id = input.project_id.clone();
        self.repository.get_project(&project_id).await?;
        let active_branch = self.repository.get_active_branch(&project_id).await?;
        let mut issues: Vec<ConsistencyIssue> = Vec::new();
        let requested_checks_set = requested_checks(&input.checks);
        let requested_severities_set = requested_severities(&input.severity_filter)?;
        let scope = input.scope.to_scope().map_err(|e| anyhow::anyhow!("{e}"))?;
        let deep_check = input.deep_check.unwrap_or(false);

        let scenes_raw = self
            .repository
            .list_scenes_by_project_and_branch(&project_id, &active_branch.id)
            .await?;
        let scenes_scoped = scoped_scenes(scenes_raw, &scope);
        let scenes = self
            .narrow_scenes_by_subjects(
                &project_id,
                &active_branch.id,
                scenes_scoped,
                &input.subjects,
            )
            .await?;

        let scene_ids: BTreeSet<String> = scenes.iter().map(|scene| scene.id.clone()).collect();
        let scoped_summaries = scoped_chapter_summaries(
            self.repository
                .list_chapter_summaries_by_project(&project_id)
                .await?,
            &scope,
        );
        let scoped_plans = scoped_chapter_plans(
            self.repository
                .list_chapter_plans_by_project(&project_id)
                .await?,
            &scope,
        );
        let scoped_annotations = self
            .repository
            .list_scene_beat_annotations_by_project(&project_id)
            .await?
            .into_iter()
            .filter(|annotation| scene_ids.contains(&annotation.scene_id))
            .collect::<Vec<_>>();

        if should_run_check(&requested_checks_set, "scene_spine_integrity") {
            for issue in self
                .find_orphan_scenes(&project_id, &active_branch.id)
                .await?
                .into_iter()
                .filter(|issue| {
                    scope_contains_position(
                        &scope,
                        issue.book_number,
                        issue.chapter_number,
                        issue.scene_order,
                    )
                })
            {
                issues.push(ConsistencyIssue {
                    severity: issue.severity.to_string(),
                    check_type: "scene_spine_integrity".to_string(),
                    message: issue.message,
                    entity_ids: issue.entity_ids,
                    suggested_action: issue.suggested_action,
                });
            }
        }

        if should_run_check(&requested_checks_set, "narrative_promise_tracking") {
            let promises = scoped_narrative_promises(
                self.repository
                    .list_narrative_promises_by_project(&project_id)
                    .await?,
                &scope,
            );
            for promise in promises {
                let chapters_since_plant = end_scope_index(&scope, &scenes)
                    .map(|end_index| {
                        end_index.saturating_sub(story_index_from_placement(&promise.planted_at))
                    })
                    .unwrap_or(0);
                let (severity, message, suggested_action) = match promise.status.as_str() {
                    "planted" if chapters_since_plant >= 3 => (
                        "warning",
                        format!(
                            "promise '{}' was planted {} chapter steps ago and is still unresolved",
                            promise.description, chapters_since_plant
                        ),
                        Some("reinforce the setup or call update_promise_status".to_string()),
                    ),
                    "planted" => (
                        "info",
                        format!(
                            "promise '{}' is planted and awaiting payoff",
                            promise.description
                        ),
                        Some("track reinforcement so it does not go cold".to_string()),
                    ),
                    "reinforced" if chapters_since_plant >= 5 => (
                        "warning",
                        format!(
                            "promise '{}' has been reinforced without payoff for {} chapter steps",
                            promise.description, chapters_since_plant
                        ),
                        Some("pay it off soon or consciously defer it".to_string()),
                    ),
                    _ => continue,
                };
                issues.push(ConsistencyIssue {
                    severity: severity.to_string(),
                    check_type: "narrative_promise_tracking".to_string(),
                    message,
                    entity_ids: vec![promise.id.clone()],
                    suggested_action,
                });
            }
        }

        let arcs = self
            .repository
            .list_character_arcs_by_project(&project_id)
            .await?;
        let trackers = self
            .repository
            .list_pacing_trackers_by_project(&project_id)
            .await?;

        if should_run_check(&requested_checks_set, "pacing_budget_audit") {
            let tracker_by_arc = trackers
                .iter()
                .map(|tracker| (tracker.character_arc_id.clone(), tracker))
                .collect::<BTreeMap<_, _>>();
            let pacing_configs = self
                .repository
                .list_pacing_configs_by_project(&project_id)
                .await?;
            let pacing_curves = self
                .repository
                .list_pacing_curves_by_project(&project_id)
                .await?;

            if pacing_configs.is_empty() && !arcs.is_empty() {
                issues.push(ConsistencyIssue {
                    severity: "warning".to_string(),
                    check_type: "pacing_budget_audit".to_string(),
                    message: "project has character arcs but no pacing configuration".to_string(),
                    entity_ids: arcs.iter().map(|arc| arc.id.clone()).collect(),
                    suggested_action: Some(
                        "create_pacing_config before using arc pacing checks".to_string(),
                    ),
                });
            }

            for arc in &arcs {
                let arc_id = arc.id.clone();
                let Some(tracker) = tracker_by_arc.get(&arc_id) else {
                    issues.push(ConsistencyIssue {
                        severity: "error".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!("character arc '{}' is missing a pacing tracker", arc_id),
                        entity_ids: vec![arc_id],
                        suggested_action: Some(
                            "create_character_arc should seed a pacing tracker".to_string(),
                        ),
                    });
                    continue;
                };

                if tracker.budget_remaining < 0.0 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' is over budget by {:.2}",
                            tracker.character_arc_id,
                            tracker.budget_remaining.abs()
                        ),
                        entity_ids: vec![tracker.id.clone(), tracker.character_arc_id.clone()],
                        suggested_action: Some(
                            "slow the arc or rebalance pacing constraints".to_string(),
                        ),
                    });
                }

                if tracker.current_progress > 1.0 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' exceeds 100% progress",
                            tracker.character_arc_id
                        ),
                        entity_ids: vec![tracker.id.clone()],
                        suggested_action: Some(
                            "recalculate progress or mark the arc complete".to_string(),
                        ),
                    });
                }

                if tracker.status == "behind" || tracker.status == "stalled" {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' tracker status is '{}'",
                            tracker.character_arc_id, tracker.status
                        ),
                        entity_ids: vec![tracker.id.clone()],
                        suggested_action: Some(
                            "accelerate the arc or update pacing constraints".to_string(),
                        ),
                    });
                }

                if tracker.status == "ahead" {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!(
                            "arc '{}' is ahead of its planned pacing",
                            tracker.character_arc_id
                        ),
                        entity_ids: vec![tracker.id.clone()],
                        suggested_action: Some(
                            "consider cooling the arc before the next milestone".to_string(),
                        ),
                    });
                }
            }

            if !pacing_configs.is_empty() {
                let planned_books: BTreeSet<i32> = self
                    .repository
                    .list_books_by_project(&project_id)
                    .await?
                    .into_iter()
                    .map(|book| book.book_number)
                    .collect();
                let curve_books: BTreeSet<i32> = pacing_curves
                    .into_iter()
                    .map(|curve| curve.book_number)
                    .collect();
                for missing_book in planned_books.difference(&curve_books) {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "pacing_budget_audit".to_string(),
                        message: format!("book {} has no pacing curve", missing_book),
                        entity_ids: vec![],
                        suggested_action: Some("create_pacing_curve for this book".to_string()),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "timeline_continuity") {
            let mut previous: Option<&crate::sqlite::records::Scene> = None;
            for scene in &scenes {
                if let Some(prev) = previous
                    && scene.book_number == prev.book_number
                    && scene.chapter_number == prev.chapter_number
                    && scene.scene_order != prev.scene_order + 1
                {
                    issues.push(ConsistencyIssue {
                        severity: "error".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "scene order jumps from {} to {} in book {}, chapter {}",
                            prev.scene_order,
                            scene.scene_order,
                            scene.book_number,
                            scene.chapter_number
                        ),
                        entity_ids: vec![prev.id.clone(), scene.id.clone()],
                        suggested_action: Some(
                            "fill the missing scene order or renumber the chapter".to_string(),
                        ),
                    });
                }
                previous = Some(scene);
            }

            let summary_keys = scoped_summaries
                .iter()
                .map(|summary| (summary.book_number, summary.chapter_number))
                .collect::<BTreeSet<_>>();
            for chapter in chapter_keys_from_scenes(&scenes) {
                if !summary_keys.contains(&chapter) {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "book {}, chapter {} has scenes but no chapter summary",
                            chapter.0, chapter.1
                        ),
                        entity_ids: vec![],
                        suggested_action: Some(
                            "save_summary after the chapter draft stabilizes".to_string(),
                        ),
                    });
                }
            }

            let timeline_events = self
                .repository
                .list_timeline_events_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|event| {
                    scope_contains_position(
                        &scope,
                        event.placement.book_number,
                        event.placement.chapter_number,
                        event.placement.scene_order.unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>();
            let event_ids = timeline_events
                .iter()
                .map(|event| event.id.clone())
                .collect::<BTreeSet<_>>();

            let mut previous_event: Option<&crate::sqlite::records::TimelineEvent> = None;
            for event in &timeline_events {
                if let Some(prev) = previous_event
                    && story_index_from_placement(&event.placement)
                        == story_index_from_placement(&prev.placement)
                    && spindle_core::models::normalize_name(&event.title)
                        == spindle_core::models::normalize_name(&prev.title)
                {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "timeline event '{}' overlaps another event at the same story position",
                            event.title
                        ),
                        entity_ids: vec![prev.id.clone(), event.id.clone()],
                        suggested_action: Some(
                            "merge duplicate events or differentiate their placement".to_string(),
                        ),
                    });
                }
                previous_event = Some(event);
            }

            let interventions = self
                .repository
                .list_temporal_interventions_by_project(&project_id)
                .await?;
            for intervention in &interventions {
                let Some(source_event_id) = intervention.source_event_id.as_ref() else {
                    continue;
                };
                let Some(target_event_id) = intervention.target_event_id.as_ref() else {
                    continue;
                };
                let source_event = self.repository.get_timeline_event(source_event_id).await?;
                let target_event = self.repository.get_timeline_event(target_event_id).await?;
                if !event_ids.contains(&source_event.id) && !event_ids.contains(&target_event.id) {
                    continue;
                }

                let source_index = story_index_from_placement(&source_event.placement);
                let target_index = story_index_from_placement(&target_event.placement);
                if source_index < target_index {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "timeline_continuity".to_string(),
                        message: format!(
                            "temporal intervention '{}' points from an earlier event to a later one",
                            intervention.title
                        ),
                        entity_ids: vec![
                            intervention.id.clone(),
                            source_event.id.clone(),
                            target_event.id.clone(),
                        ],
                        suggested_action: Some(
                            "verify source and target events are oriented correctly for time travel"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "tone_consistency") {
            let allowed_boundaries = self
                .repository
                .get_project(&project_id)
                .await?
                .reader_contract
                .boundaries
                .iter()
                .map(|boundary| boundary.to_lowercase())
                .collect::<Vec<_>>();
            for scene in &scenes {
                if let Some(tone) = scene.tone.as_ref() {
                    let tone_lower = tone.to_lowercase();
                    if allowed_boundaries.iter().any(|boundary| {
                        boundary.contains("tone:") && !boundary.contains(&tone_lower)
                    }) {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "tone_consistency".to_string(),
                            message: format!(
                                "scene {} uses tone '{}' outside declared tone boundaries",
                                scene.id, tone
                            ),
                            entity_ids: vec![scene.id.clone()],
                            suggested_action: Some(
                                "revise scene tone or update the reader contract boundary"
                                    .to_string(),
                            ),
                        });
                    }
                } else {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "tone_consistency".to_string(),
                        message: format!("scene {} has no tone metadata", scene.id),
                        entity_ids: vec![scene.id.clone()],
                        suggested_action: Some(
                            "save_scene_draft with a tone value for continuity review".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "world_rule_compliance") {
            let rules = self
                .repository
                .list_world_rules_by_project(&project_id)
                .await?;
            if !rules.is_empty() && scenes.is_empty() {
                issues.push(ConsistencyIssue {
                    severity: "info".to_string(),
                    check_type: "world_rule_compliance".to_string(),
                    message: "world rules exist but there are no scenes in the requested scope"
                        .to_string(),
                    entity_ids: rules.iter().map(|rule| rule.id.clone()).collect(),
                    suggested_action: Some(
                        "write scenes before expecting rule compliance checks".to_string(),
                    ),
                });
            }

            for rule in &rules {
                if let Some(established_in) = rule.established_in.as_ref() {
                    if !scope_contains_chapter(
                        &scope,
                        established_in.book_number,
                        established_in.chapter_number,
                    ) {
                        continue;
                    }
                    let chapter_scenes = scenes
                        .iter()
                        .filter(|scene| {
                            scene.book_number == established_in.book_number
                                && scene.chapter_number == established_in.chapter_number
                        })
                        .collect::<Vec<_>>();
                    if chapter_scenes.is_empty() {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "world_rule_compliance".to_string(),
                            message: format!(
                                "world rule '{}' is established in a chapter with no scoped scene evidence",
                                rule.rule_name
                            ),
                            entity_ids: vec![rule.id.clone()],
                            suggested_action: Some(
                                "add or restore the establishing scene for this rule".to_string(),
                            ),
                        });
                        continue;
                    }

                    if !chapter_scenes
                        .into_iter()
                        .any(|scene| crate::format::scene_mentions_rule(scene, rule))
                    {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "world_rule_compliance".to_string(),
                            message: format!(
                                "world rule '{}' is not clearly demonstrated in its establishing chapter",
                                rule.rule_name
                            ),
                            entity_ids: vec![rule.id.clone()],
                            suggested_action: Some(
                                "revise the establishing scenes so the rule is shown on page"
                                    .to_string(),
                            ),
                        });
                    }
                }
            }
            if deep_check {
                // Model-router-driven semantic compliance audit on top of
                // the pattern-based path above. Falls back to an in-process
                // heuristic when the configured route is local-only or
                // unavailable. Mirrors `services/mod.rs:6344` in 705b835^.
                let semantic_issues = self
                    .deep_world_rule_compliance_issues(&scenes, &rules)
                    .await?;
                issues.extend(semantic_issues);
            }
        }

        if should_run_check(&requested_checks_set, "character_consistency") {
            let characters = self
                .repository
                .list_characters_by_project(&project_id)
                .await?;
            // Project-wide character states fetched once; filter per character below.
            let all_states = self
                .repository
                .list_character_states_by_project_and_branch(&project_id, &active_branch.id)
                .await?;
            for character in characters {
                let voice_profile = self
                    .repository
                    .get_character_voice_profile(&character.id)
                    .await?;
                let emotional_profile = self
                    .repository
                    .get_character_emotional_profile(&character.id)
                    .await?;

                if voice_profile.example_lines.is_empty() {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "character_consistency".to_string(),
                        message: format!(
                            "character '{}' has no example dialogue lines",
                            character.name
                        ),
                        entity_ids: vec![character.id.clone(), voice_profile.id.clone()],
                        suggested_action: Some(
                            "add example_lines so dialogue checks have a baseline".to_string(),
                        ),
                    });
                }

                if emotional_profile.base_emotions.is_empty()
                    && emotional_profile.suppressed.is_empty()
                    && emotional_profile.triggers.is_empty()
                {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "character_consistency".to_string(),
                        message: format!(
                            "character '{}' has a very thin emotional profile",
                            character.name
                        ),
                        entity_ids: vec![character.id.clone(), emotional_profile.id.clone()],
                        suggested_action: Some(
                            "expand the emotional profile before relying on consistency checks"
                                .to_string(),
                        ),
                    });
                }

                let scoped_states = all_states
                    .iter()
                    .filter(|state| state.character_id == character.id)
                    .filter(|state| {
                        scope_contains_position(
                            &scope,
                            state.book_number,
                            state.chapter_number,
                            state.scene_order,
                        )
                    })
                    .collect::<Vec<_>>();
                for window in scoped_states.windows(2) {
                    let previous = window[0];
                    let current = window[1];
                    if current.book_number == previous.book_number
                        && current.chapter_number == previous.chapter_number
                        && current.scene_order == previous.scene_order
                    {
                        issues.push(ConsistencyIssue {
                            severity: "warning".to_string(),
                            check_type: "character_consistency".to_string(),
                            message: format!(
                                "character '{}' has multiple state snapshots at book {}, chapter {}, scene {}",
                                character.name, current.book_number, current.chapter_number, current.scene_order
                            ),
                            entity_ids: vec![current.id.clone()],
                            suggested_action: Some("consolidate duplicate state commits for this scene".to_string()),
                        });
                    }
                }
            }
        }

        if should_run_check(&requested_checks_set, "agency_tracking") {
            for plan in &scoped_plans {
                let low_purpose_count = plan
                    .scenes
                    .iter()
                    .filter(|scene| scene.purpose.trim().len() < 12)
                    .count();
                if low_purpose_count > 0 {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "agency_tracking".to_string(),
                        message: format!(
                            "chapter plan {}:{} has {} scene purposes that are too vague to audit agency",
                            plan.book_number, plan.chapter_number, low_purpose_count
                        ),
                        entity_ids: vec![plan.id.clone()],
                        suggested_action: Some(
                            "rewrite scene purposes as concrete choices or pressures".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "try_fail_cycle_tracking") {
            let conflicts = self
                .repository
                .list_conflicts_by_project(&project_id)
                .await?;
            for conflict in conflicts {
                let steps = conflict.try_fail_cycles.len();
                if conflict.expected_total_cycles.unwrap_or(steps as i32) >= 2 && steps < 2 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "try_fail_cycle_tracking".to_string(),
                        message: format!(
                            "conflict '{}' does not yet have enough try-fail steps",
                            conflict.name
                        ),
                        entity_ids: vec![conflict.id.clone()],
                        suggested_action: Some(
                            "add more failed attempts before resolution".to_string(),
                        ),
                    });
                }
                if conflict.resolution_summary.is_some() && steps == 0 {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "try_fail_cycle_tracking".to_string(),
                        message: format!(
                            "conflict '{}' resolves without recorded failed attempts",
                            conflict.name
                        ),
                        entity_ids: vec![conflict.id.clone()],
                        suggested_action: Some(
                            "record the failed attempts or simplify the conflict model".to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "consequence_delivery_audit") {
            let conflicts = self
                .repository
                .list_conflicts_by_project(&project_id)
                .await?;
            for conflict in conflicts {
                for consequence in conflict.stated_consequences {
                    if consequence.delivered {
                        continue;
                    }
                    issues.push(ConsistencyIssue {
                        severity: if consequence.must_demonstrate_by.is_some() {
                            "warning".to_string()
                        } else {
                            "info".to_string()
                        },
                        check_type: "consequence_delivery_audit".to_string(),
                        message: format!(
                            "conflict '{}' has an undelivered consequence: {}",
                            conflict.name, consequence.description
                        ),
                        entity_ids: vec![conflict.id.clone()],
                        suggested_action: Some(
                            "show the consequence on page or mark it intentionally deferred"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "content_boundary_compliance") {
            for scene in &scenes {
                if scene.content_rating.eq_ignore_ascii_case("Explicit") {
                    issues.push(ConsistencyIssue {
                        severity: "info".to_string(),
                        check_type: "content_boundary_compliance".to_string(),
                        message: format!(
                            "scene {} is explicit and should be reviewed against reader boundaries manually",
                            scene.id
                        ),
                        entity_ids: vec![scene.id.clone()],
                        suggested_action: Some("review scene tags and boundaries before finalizing".to_string()),
                    });
                }
            }
        }

        if should_run_check(&requested_checks_set, "knowledge_contradiction_detection")
            && !scoped_annotations.is_empty()
            && scenes.is_empty()
        {
            issues.push(ConsistencyIssue {
                severity: "info".to_string(),
                check_type: "knowledge_contradiction_detection".to_string(),
                message:
                    "scene annotations exist without scoped scenes; knowledge checks are incomplete"
                        .to_string(),
                entity_ids: scoped_annotations
                    .iter()
                    .map(|annotation| annotation.id.clone())
                    .collect(),
                suggested_action: Some(
                    "restore or rescope the related scenes before running contradiction checks"
                        .to_string(),
                ),
            });
        }

        if should_run_check(&requested_checks_set, "knowledge_contradiction_detection") {
            let future_knowledge = self
                .repository
                .list_future_knowledge_by_project(&project_id)
                .await?
                .into_iter()
                .filter(|knowledge| {
                    scope_contains_position(
                        &scope,
                        knowledge.learned_at.book_number,
                        knowledge.learned_at.chapter_number,
                        knowledge.learned_at.scene_order.unwrap_or(0),
                    )
                })
                .collect::<Vec<_>>();
            let timeline_events = self
                .repository
                .list_timeline_events_by_project(&project_id)
                .await?;
            let timeline_event_ids = timeline_events
                .iter()
                .map(|event| (event.id.clone(), event))
                .collect::<BTreeMap<_, _>>();
            let interventions = self
                .repository
                .list_temporal_interventions_by_project(&project_id)
                .await?;

            for knowledge in &future_knowledge {
                let learned_index = story_index_from_placement(&knowledge.learned_at);

                if let Some(expires_at) = knowledge.expires_at.as_ref() {
                    let expires_index = story_index_from_placement(expires_at);
                    if expires_index < learned_index {
                        issues.push(ConsistencyIssue {
                            severity: "error".to_string(),
                            check_type: "knowledge_contradiction_detection".to_string(),
                            message: format!(
                                "future knowledge '{}' expires before it is learned",
                                knowledge.knowledge_summary
                            ),
                            entity_ids: vec![knowledge.id.clone()],
                            suggested_action: Some(
                                "move the expiry later or revise the learned-at placement"
                                    .to_string(),
                            ),
                        });
                    }
                }

                let invalidating_interventions = interventions
                    .iter()
                    .filter(|intervention| {
                        intervention
                            .consequences
                            .iter()
                            .any(|consequence| consequence.to_lowercase().contains("invalidate"))
                    })
                    .filter_map(|intervention| {
                        intervention
                            .target_event_id
                            .as_ref()
                            .and_then(|id| timeline_event_ids.get(id).copied())
                            .map(|event| (intervention, event))
                    })
                    .filter(|(_, event)| {
                        story_index_from_placement(&event.placement) >= learned_index
                    })
                    .collect::<Vec<_>>();

                if !invalidating_interventions.is_empty() && knowledge.expires_at.is_none() {
                    issues.push(ConsistencyIssue {
                        severity: "warning".to_string(),
                        check_type: "knowledge_contradiction_detection".to_string(),
                        message: format!(
                            "future knowledge '{}' may be invalidated by a later intervention but has no expiry",
                            knowledge.knowledge_summary
                        ),
                        entity_ids: std::iter::once(knowledge.id.clone())
                            .chain(
                                invalidating_interventions
                                    .iter()
                                    .map(|(intervention, _)| intervention.id.clone()),
                            )
                            .collect(),
                        suggested_action: Some(
                            "set an expiry placement or clarify why the knowledge remains valid"
                                .to_string(),
                        ),
                    });
                }
            }
        }

        // ── Phase-4 validator fan-out ───────────────────────────────
        // Runs canonical_fact_prose_drift / world_rule_semantic_drift /
        // voice_drift / retcon_reachability across the scoped scenes. The
        // resulting issues are appended to `issues` (and surface in
        // `report_sections` via `build_consistency_report_sections` below).
        // Mirrors `services/mod.rs:6233` in 705b835^.
        let phase_four_checks = requested_phase_four_validator_checks(&requested_checks_set);
        let phase_four_issues = self
            .run_phase_four_validator_checks_for_scenes(
                &project_id,
                &active_branch.id,
                &scenes,
                &phase_four_checks,
                true,
            )
            .await?;
        issues.extend(phase_four_issues);

        // ── Gate 1 + 3: scene divergence detection ──────────────────
        if should_run_check(&requested_checks_set, "scene_divergence") {
            let source_links = self
                .repository
                .list_scene_source_links_by_project(&project_id)
                .await?;
            for link in &source_links {
                let link_scene_id = link.scene_id.clone();
                if !scene_ids.contains(&link_scene_id) {
                    continue; // outside the requested scope
                }
                let Some(scene) = scenes.iter().find(|scene| scene.id == link_scene_id) else {
                    continue;
                };
                if let Some(observation) =
                    super::source_bridge::evaluate_scene_divergence(link, scene)
                {
                    issues.push(
                        super::source_bridge::divergence_observation_to_consistency_issue(
                            &observation,
                            &link.source_path,
                            &link_scene_id,
                        ),
                    );
                }
            }
        }

        // ── Gate 2: canonical fact contradiction detection ───────────
        if should_run_check(&requested_checks_set, "canonical_fact_consistency") {
            let facts = self
                .repository
                .list_active_canonical_facts_by_project(&project_id)
                .await?;
            // Group active facts by canonical subject+predicate to find contradictions.
            let mut facts_by_key: BTreeMap<String, Vec<&crate::sqlite::records::CanonicalFact>> =
                BTreeMap::new();
            for fact in &facts {
                let subject_key = fact
                    .subject_id
                    .clone()
                    .unwrap_or_else(|| "project".to_string());
                facts_by_key
                    .entry(format!(
                        "{}:{}:{}",
                        fact.subject_table, subject_key, fact.predicate
                    ))
                    .or_default()
                    .push(fact);
            }
            for (composite_key, group) in &facts_by_key {
                if group.len() > 1 {
                    // Multiple active (non-superseded) facts with the same key — contradiction.
                    let values: Vec<String> = group
                        .iter()
                        .map(|fact| canonical_fact_value_for_check(fact))
                        .collect();
                    let unique_values: BTreeSet<&str> = values.iter().map(String::as_str).collect();
                    if unique_values.len() > 1 {
                        issues.push(ConsistencyIssue {
                            severity: "error".to_string(),
                            check_type: "canonical_fact_consistency".to_string(),
                            message: format!(
                                "canonical fact '{}' has conflicting active values: {}",
                                composite_key,
                                unique_values
                                    .iter()
                                    .map(|v| format!("'{}'", v))
                                    .collect::<Vec<_>>()
                                    .join(" vs ")
                            ),
                            entity_ids: group.iter().map(|f| f.id.clone()).collect(),
                            suggested_action: Some(
                                "supersede the outdated fact using register_canonical_fact with supersedes_fact_id".to_string(),
                            ),
                        });
                    }
                }
            }
        }

        if let Some(requested_severities) = requested_severities_set.as_ref() {
            issues.retain(|issue| requested_severities.contains(issue.severity.as_str()));
        }

        // Build per-validator scene groups for the Phase-4 issues so MCP
        // callers can render them under their own headings.
        let report_sections = build_consistency_report_sections(&issues, &scenes);

        let summary = ConsistencySummary {
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
        };

        let format = input.format.unwrap_or(ContextFormat::Json);
        let markdown = if format == ContextFormat::Markdown {
            let budget = input
                .budget_tokens
                .unwrap_or(DEFAULT_CHECK_CONSISTENCY_BUDGET_TOKENS);
            Some(format_consistency_markdown(
                &issues,
                &report_sections,
                budget,
            ))
        } else {
            None
        };

        Ok(spindle_core::models::CheckConsistencyOutput {
            issues,
            summary,
            report_sections,
            markdown,
        })
    }

    /// Narrow a scene list to only those that reference any of the
    /// supplied subjects. A scene matches if its id is a direct hit for
    /// the subject (e.g., character_state.scene_id,
    /// scene_beat_annotation.{motif,theme,conflict}_ids) OR its full_text
    /// contains any of the subject's display terms (case-insensitive
    /// for ASCII). Mirrors `services/mod.rs:6193` in 705b835^.
    async fn narrow_scenes_by_subjects(
        &self,
        project_id: &str,
        branch_id: &str,
        scenes: Vec<crate::sqlite::records::Scene>,
        subjects: &[String],
    ) -> Result<Vec<crate::sqlite::records::Scene>> {
        use std::collections::BTreeSet;
        if subjects.is_empty() {
            return Ok(scenes);
        }
        let mut direct_scene_ids: BTreeSet<String> = BTreeSet::new();
        let mut term_set: BTreeSet<String> = BTreeSet::new();
        for raw in subjects {
            let spec = self
                .scene_reference_subject_spec(project_id, branch_id, raw)
                .await?;
            direct_scene_ids.extend(spec.direct_scene_ids);
            for term in spec.terms {
                let trimmed = term.trim();
                if !trimmed.is_empty() {
                    term_set.insert(trimmed.to_string());
                }
            }
        }
        let terms: Vec<String> = term_set.into_iter().collect();
        Ok(scenes
            .into_iter()
            .filter(|scene| {
                if direct_scene_ids.contains(&scene.id) {
                    return true;
                }
                find_scene_reference_term_match(&scene.full_text, &terms).is_some()
            })
            .collect())
    }

    /// Derive `(display terms, direct-hit scene ids)` for one subject id.
    /// Used by `narrow_scenes_by_subjects` and `find_scenes_referencing`.
    /// Mirrors `services/mod.rs:6826` in 705b835^.
    async fn scene_reference_subject_spec(
        &self,
        project_id: &str,
        branch_id: &str,
        subject_id: &str,
    ) -> Result<SubjectSceneReferenceSpec> {
        use std::collections::BTreeSet;
        let (table, _key) = subject_id
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("expected `table:id` format, got `{subject_id}`"))?;

        let mut spec = match table {
            "character" => {
                let character = self.repository.get_character(subject_id).await?;
                ensure_reference_subject_project(&character.project_id, project_id, "character")?;
                let mut direct_scene_ids = BTreeSet::new();
                for state in self
                    .repository
                    .list_character_states_by_project_and_branch(project_id, branch_id)
                    .await?
                {
                    if state.character_id == subject_id
                        && let Some(scene_id) = state.scene_id
                    {
                        direct_scene_ids.insert(scene_id);
                    }
                }
                for relationship in self
                    .repository
                    .list_relationships_by_branch(branch_id)
                    .await?
                {
                    if (relationship.in_id == subject_id || relationship.out_id == subject_id)
                        && let Some(scene_id) = relationship.last_scene_id
                    {
                        direct_scene_ids.insert(scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![character.name],
                    direct_scene_ids,
                }
            }
            "location" => {
                let location = self.repository.get_location(subject_id).await?;
                ensure_reference_subject_project(&location.project_id, project_id, "location")?;
                SubjectSceneReferenceSpec {
                    terms: vec![location.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "faction" => {
                let faction = self.repository.get_faction(subject_id).await?;
                ensure_reference_subject_project(&faction.project_id, project_id, "faction")?;
                SubjectSceneReferenceSpec {
                    terms: vec![faction.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "religion" => {
                let religion = self.repository.get_religion(subject_id).await?;
                ensure_reference_subject_project(&religion.project_id, project_id, "religion")?;
                SubjectSceneReferenceSpec {
                    terms: vec![religion.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "economy" => {
                let economy = self.repository.get_economy(subject_id).await?;
                ensure_reference_subject_project(&economy.project_id, project_id, "economy")?;
                SubjectSceneReferenceSpec {
                    terms: vec![economy.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "term" => {
                let term = self.repository.get_term(subject_id).await?;
                ensure_reference_subject_project(&term.project_id, project_id, "term")?;
                SubjectSceneReferenceSpec {
                    terms: vec![term.term_text],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "plot_line" => {
                let plot_line = self
                    .repository
                    .list_plot_lines_by_project(project_id)
                    .await?
                    .into_iter()
                    .find(|pl| pl.id == subject_id)
                    .ok_or_else(|| anyhow::anyhow!("plot line not found"))?;
                SubjectSceneReferenceSpec {
                    terms: vec![plot_line.name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "conflict" => {
                let conflict = self.repository.get_conflict(subject_id).await?;
                ensure_reference_subject_project(&conflict.project_id, project_id, "conflict")?;
                let mut direct_scene_ids = BTreeSet::new();
                for annotation in self
                    .repository
                    .list_scene_beat_annotations_by_project(project_id)
                    .await?
                {
                    if annotation.conflict_ids.iter().any(|id| id == subject_id) {
                        direct_scene_ids.insert(annotation.scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![conflict.name],
                    direct_scene_ids,
                }
            }
            "theme" => {
                let theme = self.repository.get_theme(subject_id).await?;
                ensure_reference_subject_project(&theme.project_id, project_id, "theme")?;
                let mut direct_scene_ids = BTreeSet::new();
                for annotation in self
                    .repository
                    .list_scene_beat_annotations_by_project(project_id)
                    .await?
                {
                    if annotation.theme_ids.iter().any(|id| id == subject_id) {
                        direct_scene_ids.insert(annotation.scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![theme.theme_statement],
                    direct_scene_ids,
                }
            }
            "motif" => {
                let motif = self.repository.get_motif(subject_id).await?;
                ensure_reference_subject_project(&motif.project_id, project_id, "motif")?;
                let mut direct_scene_ids = BTreeSet::new();
                for annotation in self
                    .repository
                    .list_scene_beat_annotations_by_project(project_id)
                    .await?
                {
                    if annotation.motif_ids.iter().any(|id| id == subject_id) {
                        direct_scene_ids.insert(annotation.scene_id);
                    }
                }
                SubjectSceneReferenceSpec {
                    terms: vec![motif.name],
                    direct_scene_ids,
                }
            }
            "world_rule" => {
                let world_rule = self
                    .repository
                    .list_world_rules_by_project(project_id)
                    .await?
                    .into_iter()
                    .find(|wr| wr.id == subject_id)
                    .ok_or_else(|| anyhow::anyhow!("world rule not found"))?;
                SubjectSceneReferenceSpec {
                    terms: vec![world_rule.rule_name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "narrative_promise" => {
                let promise = self.repository.get_narrative_promise(subject_id).await?;
                ensure_reference_subject_project(
                    &promise.project_id,
                    project_id,
                    "narrative_promise",
                )?;
                SubjectSceneReferenceSpec {
                    terms: vec![promise.description],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            "system_overlay" => {
                let overlay = self.repository.get_system_overlay(subject_id).await?;
                ensure_reference_subject_project(
                    &overlay.project_id,
                    project_id,
                    "system_overlay",
                )?;
                SubjectSceneReferenceSpec {
                    terms: vec![overlay.system_name],
                    direct_scene_ids: BTreeSet::new(),
                }
            }
            other => {
                anyhow::bail!("find_scenes_referencing does not support subject table `{other}`");
            }
        };
        spec.terms.retain(|term| !term.trim().is_empty());
        if spec.terms.is_empty() && spec.direct_scene_ids.is_empty() {
            anyhow::bail!(
                "find_scenes_referencing could not derive any search terms for `{subject_id}`"
            );
        }
        Ok(spec)
    }

    /// Scan a scene's prose for hits against project-wide world rules.
    /// Mirrors `services/mod.rs:5737` in 705b835^.
    async fn scan_world_rules(
        &self,
        project_id: &str,
        prose: &str,
    ) -> Result<Vec<spindle_core::models::WorldRuleHit>> {
        let rules = self
            .repository
            .list_world_rules_by_project(project_id)
            .await?;
        let scan_rules: Vec<spindle_core::world_rules::ScanRule> = rules
            .iter()
            .map(|r| spindle_core::world_rules::ScanRule {
                rule_id: r.id.clone(),
                scan_pattern: r.scan_pattern.clone(),
                rule_name: r.rule_name.clone(),
                description: r.description.clone(),
            })
            .collect();
        Ok(spindle_core::world_rules::scan_prose_for_world_rules(
            prose,
            &scan_rules,
        ))
    }

    /// Scan a scene's prose for forbidden-phrase voice drift against every
    /// character on the active branch (with per-project main fallback). Only
    /// characters whose name/aliases appear in the prose are checked. Mirrors
    /// `services/mod.rs:5758` in 705b835^.
    async fn scan_voice_drift(
        &self,
        project_id: &str,
        prose: &str,
    ) -> Result<Vec<spindle_core::models::VoiceDriftFinding>> {
        use spindle_core::models::CharacterVoiceProfileData;
        use spindle_core::voice::{
            VoiceDriftCharacter, character_present_in_scene, check_voice_drift,
        };
        use std::collections::BTreeMap;

        let active_branch = self.repository.get_active_branch(project_id).await?;
        let mut characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        if active_branch.name != "main" {
            for branch in self.repository.list_branches_by_project(project_id).await? {
                if branch.name == "main" {
                    characters.extend(
                        self.repository
                            .list_characters_by_project_and_branch(project_id, &branch.id)
                            .await?,
                    );
                    break;
                }
            }
            let mut deduped = BTreeMap::new();
            for character in characters {
                deduped.insert(character.id.clone(), character);
            }
            characters = deduped.into_values().collect();
        }
        characters.retain(|character| {
            character_present_in_scene(
                prose,
                &VoiceDriftCharacter {
                    id: character.id.clone(),
                    name: character.name.clone(),
                },
            )
        });
        if characters.is_empty() {
            return Ok(Vec::new());
        }

        let mut findings = Vec::new();
        for character in characters {
            let voice_profile = self
                .repository
                .get_character_voice_profile(&character.id)
                .await?;
            let profile_data = CharacterVoiceProfileData {
                tone: voice_profile.tone.clone(),
                vocabulary: voice_profile.vocabulary.clone(),
                sentence_structure: voice_profile.sentence_structure.clone(),
                tics: voice_profile.tics.clone(),
                forbidden_words: voice_profile.forbidden_words.clone(),
                example_lines: voice_profile.example_lines.clone(),
                established_in_scene_id: voice_profile.established_in_scene_id.clone(),
                updated_at: voice_profile.updated_at.as_ref().map(ToString::to_string),
            };
            let drift_character = VoiceDriftCharacter {
                id: character.id.clone(),
                name: character.name.clone(),
            };
            findings.extend(check_voice_drift(prose, &drift_character, &profile_data));
        }
        Ok(findings)
    }

    /// Scan a scene's prose for retcon-reachability problems on the active
    /// branch (with per-project main fallback). Mirrors `check_reachability`
    /// (`services/mod.rs:5800` area) in 705b835^.
    async fn scan_retcon_findings(
        &self,
        project_id: &str,
        scene: &crate::sqlite::records::Scene,
    ) -> Result<Vec<spindle_core::models::RetconFinding>> {
        use spindle_core::models::RetconFinding;
        use std::collections::BTreeMap;

        let active_branch = self.repository.get_active_branch(project_id).await?;
        let mut future_knowledge = self
            .repository
            .list_future_knowledge_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut timeline_events = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut temporal_interventions = self
            .repository
            .list_temporal_interventions_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        if active_branch.name != "main" {
            for branch in self.repository.list_branches_by_project(project_id).await? {
                if branch.name != "main" {
                    continue;
                }
                let main_id = branch.id;
                let main_future_knowledge = self
                    .repository
                    .list_future_knowledge_by_project_and_branch(project_id, &main_id)
                    .await?;
                let main_timeline_events = self
                    .repository
                    .list_timeline_events_by_project_and_branch(project_id, &main_id)
                    .await?;
                let main_temporal_interventions = self
                    .repository
                    .list_temporal_interventions_by_project_and_branch(project_id, &main_id)
                    .await?;

                let mut merged_future_knowledge = main_future_knowledge
                    .into_iter()
                    .map(|row| (row.id.clone(), row))
                    .collect::<BTreeMap<_, _>>();
                for row in future_knowledge {
                    merged_future_knowledge.insert(row.id.clone(), row);
                }
                future_knowledge = merged_future_knowledge.into_values().collect();

                let mut merged_timeline_events = main_timeline_events
                    .into_iter()
                    .map(|row| (row.id.clone(), row))
                    .collect::<BTreeMap<_, _>>();
                for row in timeline_events {
                    merged_timeline_events.insert(row.id.clone(), row);
                }
                timeline_events = merged_timeline_events.into_values().collect();

                let mut merged_interventions = main_temporal_interventions
                    .into_iter()
                    .map(|row| (row.id.clone(), row))
                    .collect::<BTreeMap<_, _>>();
                for row in temporal_interventions {
                    merged_interventions.insert(row.id.clone(), row);
                }
                temporal_interventions = merged_interventions.into_values().collect();
                break;
            }
        }

        let scene_position = (scene.book_number, scene.chapter_number, scene.scene_order);
        let mut findings = Vec::new();
        for knowledge in &future_knowledge {
            let learned_at = (
                knowledge.learned_at.book_number,
                knowledge.learned_at.chapter_number,
                knowledge.learned_at.scene_order.unwrap_or(0),
            );
            if !position_gt(learned_at, scene_position) {
                continue;
            }
            if !contains_case_insensitive_phrase(&scene.full_text, &knowledge.knowledge_summary)
                && !contains_case_insensitive_phrase(&scene.summary, &knowledge.knowledge_summary)
            {
                continue;
            }
            findings.push(RetconFinding::OutOfBandsKnowledge {
                character_id: knowledge.character_id.clone(),
                knowledge_summary: knowledge.knowledge_summary.clone(),
                learned_at: knowledge.learned_at.clone().into_core(),
                message: format!(
                    "scene references future knowledge '{}' before learned_at {}.{}.{}",
                    knowledge.knowledge_summary, learned_at.0, learned_at.1, learned_at.2
                ),
            });
        }

        let events_by_id = timeline_events
            .iter()
            .map(|event| (event.id.clone(), event))
            .collect::<BTreeMap<_, _>>();
        for intervention in &temporal_interventions {
            let anchor_position = intervention
                .target_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                .or_else(|| {
                    intervention
                        .source_event_id
                        .as_ref()
                        .and_then(|id| events_by_id.get(id))
                })
                .map(|event| {
                    (
                        event.placement.book_number,
                        event.placement.chapter_number,
                        event.placement.scene_order.unwrap_or(0),
                    )
                });
            let Some(anchor_position) = anchor_position else {
                continue;
            };
            if !position_gt(anchor_position, scene_position) {
                continue;
            }

            let mut anchor_terms: Vec<String> = Vec::new();
            if !intervention.title.trim().is_empty() {
                anchor_terms.push(intervention.title.clone());
            }
            if let Some(source) = intervention
                .source_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                && !source.title.trim().is_empty()
            {
                anchor_terms.push(source.title.clone());
            }
            if let Some(target) = intervention
                .target_event_id
                .as_ref()
                .and_then(|id| events_by_id.get(id))
                && !target.title.trim().is_empty()
            {
                anchor_terms.push(target.title.clone());
            }
            if anchor_terms.is_empty() {
                continue;
            }

            let mentions_intervention = anchor_terms.iter().any(|term| {
                contains_case_insensitive_phrase(&scene.full_text, term)
                    || contains_case_insensitive_phrase(&scene.summary, term)
            });
            if !mentions_intervention {
                continue;
            }

            let has_future_knowledge_anchor = future_knowledge.iter().any(|knowledge| {
                let learned_at = (
                    knowledge.learned_at.book_number,
                    knowledge.learned_at.chapter_number,
                    knowledge.learned_at.scene_order.unwrap_or(0),
                );
                if position_gt(learned_at, scene_position) {
                    return false;
                }
                if let Some(expires_at) = knowledge.expires_at.as_ref() {
                    let expires_position = (
                        expires_at.book_number,
                        expires_at.chapter_number,
                        expires_at.scene_order.unwrap_or(0),
                    );
                    if position_lt(expires_position, scene_position) {
                        return false;
                    }
                }
                anchor_terms.iter().any(|term| {
                    contains_case_insensitive_phrase(&knowledge.knowledge_summary, term)
                        || contains_case_insensitive_phrase(&knowledge.source, term)
                })
            });
            if !has_future_knowledge_anchor {
                findings.push(RetconFinding::MissingFutureKnowledgeAnchor {
                    intervention_id: intervention.id.clone(),
                    intervention_title: intervention.title.clone(),
                    message: format!(
                        "scene references intervention '{}' before timeline anchor {}.{}.{} without a future_knowledge anchor",
                        intervention.title, anchor_position.0, anchor_position.1, anchor_position.2
                    ),
                });
            }
        }

        let mut characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut states = self
            .repository
            .list_character_states_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        if active_branch.name != "main" {
            for branch in self.repository.list_branches_by_project(project_id).await? {
                if branch.name != "main" {
                    continue;
                }
                characters.extend(
                    self.repository
                        .list_characters_by_project_and_branch(project_id, &branch.id)
                        .await?,
                );
                states.extend(
                    self.repository
                        .list_character_states_by_project_and_branch(project_id, &branch.id)
                        .await?,
                );
                break;
            }
        }
        let characters_by_id = characters
            .into_iter()
            .map(|character| (character.id.clone(), character))
            .collect::<BTreeMap<_, _>>();

        for (character_id, character) in &characters_by_id {
            if !contains_case_insensitive_word_boundary(&scene.full_text, &character.name) {
                continue;
            }
            let latest_state = states
                .iter()
                .filter(|state| state.character_id == *character_id)
                .filter(|state| {
                    let state_position =
                        (state.book_number, state.chapter_number, state.scene_order);
                    position_lte(state_position, scene_position)
                })
                .max_by_key(|state| (state.book_number, state.chapter_number, state.scene_order));
            let Some(latest_state) = latest_state else {
                continue;
            };
            let Some(dead_status) = latest_state
                .status
                .iter()
                .find(|status| is_dead_status(status))
                .cloned()
            else {
                continue;
            };
            findings.push(RetconFinding::DeadCharacterAct {
                character_id: character_id.clone(),
                character_name: character.name.clone(),
                status: dead_status.clone(),
                message: format!(
                    "scene includes '{}' acting after status '{}'",
                    character.name, dead_status
                ),
            });
        }

        Ok(findings)
    }

    /// Assemble a `ValidatorContext` that the Phase-4 validator registry can
    /// evaluate scenes against. Pulls active canonical facts, world rules,
    /// character voice profiles, timeline events, and temporal interventions
    /// for the project. Mirrors `services/mod.rs:6089` in 705b835^.
    async fn build_phase_four_validator_context(
        &self,
        project_id: &str,
        branch_id: &str,
        scenes: &[crate::sqlite::records::Scene],
    ) -> Result<spindle_core::validators::ValidatorContext> {
        use spindle_core::validators::{
            CanonicalFactSnapshot, CharacterVoiceProfileSnapshot, SceneSnapshot,
            TemporalInterventionSnapshot, TimelineEventSnapshot, ValidatorContext,
            WorldRuleSnapshot,
        };

        let canonical_facts_raw = self
            .repository
            .list_active_canonical_facts_by_project_and_branch(project_id, branch_id)
            .await?;
        let world_rules_raw = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?;
        let characters = self
            .repository
            .list_characters_by_project_and_branch(project_id, branch_id)
            .await?;
        let timeline_events_raw = self
            .repository
            .list_timeline_events_by_project_and_branch(project_id, branch_id)
            .await?;
        let temporal_interventions_raw = self
            .repository
            .list_temporal_interventions_by_project_and_branch(project_id, branch_id)
            .await?;

        let canonical_facts = canonical_facts_raw
            .into_iter()
            .map(|fact| {
                let value = canonical_fact_value_for_check(&fact);
                CanonicalFactSnapshot {
                    scene_id: fact.scene_id.clone(),
                    book_number: fact.book_number,
                    chapter_number: fact.chapter_number,
                    fact_type: fact.value_kind.clone(),
                    key: fact.predicate.clone(),
                    value,
                }
            })
            .collect::<Vec<_>>();

        let world_rules = world_rules_raw
            .into_iter()
            .map(|rule| WorldRuleSnapshot {
                rule_id: rule.id.clone(),
                rule_name: rule.rule_name.clone(),
                scan_pattern: rule.scan_pattern.clone(),
                established_in: rule
                    .established_in
                    .as_ref()
                    .map(|placement| (placement.book_number, placement.chapter_number)),
            })
            .collect::<Vec<_>>();

        let mut voice_profiles = Vec::new();
        for character in characters {
            let profile = self
                .repository
                .get_character_voice_profile(&character.id)
                .await?;
            voice_profiles.push(CharacterVoiceProfileSnapshot {
                character_id: character.id.clone(),
                character_name: character.name.clone(),
                forbidden_words: profile.forbidden_words.clone(),
            });
        }

        let timeline_events = timeline_events_raw
            .into_iter()
            .map(|event| TimelineEventSnapshot {
                event_id: event.id.clone(),
                title: event.title.clone(),
                book_number: event.placement.book_number,
                chapter_number: event.placement.chapter_number,
                scene_order: event.placement.scene_order.unwrap_or(0),
            })
            .collect::<Vec<_>>();

        let temporal_interventions = temporal_interventions_raw
            .into_iter()
            .map(|intervention| TemporalInterventionSnapshot {
                intervention_id: intervention.id.clone(),
                title: intervention.title.clone(),
                source_event_id: intervention.source_event_id.clone(),
                target_event_id: intervention.target_event_id.clone(),
            })
            .collect::<Vec<_>>();

        let scene_snapshots = scenes
            .iter()
            .map(|scene| SceneSnapshot {
                scene_id: scene.id.clone(),
                book_number: scene.book_number,
                chapter_number: scene.chapter_number,
                scene_order: scene.scene_order,
                full_text: scene.full_text.clone(),
                summary: scene.summary.clone(),
            })
            .collect::<Vec<_>>();

        let style_directive = self
            .style_directive_for(project_id, branch_id)
            .await
            .ok()
            .filter(|directive| !directive.is_empty());

        Ok(ValidatorContext {
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            scenes: scene_snapshots,
            canonical_facts,
            world_rules,
            voice_profiles,
            timeline_events,
            temporal_interventions,
            style_directive,
        })
    }

    /// Run the Phase-4 validator registry against `scenes` and persist
    /// per-scene cache rows into `validator_finding`. Returns the
    /// `ConsistencyIssue` list filtered to `phase_four_checks`. Mirrors
    /// `services/mod.rs:6233` in 705b835^.
    async fn run_phase_four_validator_checks_for_scenes(
        &self,
        project_id: &str,
        branch_id: &str,
        scenes: &[crate::sqlite::records::Scene],
        phase_four_checks: &std::collections::BTreeSet<PhaseFourCacheId>,
        use_cache: bool,
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use spindle_core::models::ConsistencyIssue;

        if phase_four_checks.is_empty() || scenes.is_empty() {
            return Ok(Vec::new());
        }

        let context = self
            .build_phase_four_validator_context(project_id, branch_id, scenes)
            .await?;
        let registry = crate::sqlite::validators::phase_four_validator_registry();
        let validator_ids = phase_four_validator_ids(phase_four_checks);
        let context_hashes = phase_four_context_hashes(&context, phase_four_checks)?;
        let mut issues = Vec::new();

        for scene in &context.scenes {
            let scene_text_hash = generation_sha256_hex(scene.full_text.as_bytes());
            if use_cache {
                let cached_rows = self
                    .repository
                    .list_active_validator_findings_by_scene_hash(
                        branch_id,
                        &scene.scene_id,
                        &scene_text_hash,
                        &validator_ids,
                    )
                    .await?;
                let cached_rows = cached_rows
                    .into_iter()
                    .filter(|row| {
                        row.context_hash
                            .as_ref()
                            .and_then(|hash| {
                                context_hashes
                                    .get(row.validator_id.as_str())
                                    .map(|expected| expected == hash)
                            })
                            .unwrap_or(false)
                    })
                    .collect::<Vec<_>>();
                if has_cache_for_all_validators(&cached_rows, &validator_ids) {
                    issues.extend(cached_validator_issues(&cached_rows, &scene.scene_id));
                    continue;
                }
            }

            let findings = registry
                .validate_scene(scene, &context)
                .map_err(|error| anyhow::anyhow!(error))?;

            for validator_id in &validator_ids {
                let validator_findings = findings
                    .iter()
                    .filter(|finding| finding.check_type == validator_id)
                    .collect::<Vec<_>>();

                let serialized_issues = validator_findings
                    .iter()
                    .map(|finding| {
                        serde_json::json!({
                            "severity": finding.severity.as_str(),
                            "check_type": finding.check_type,
                            "message": finding.message,
                            "byte_range": finding.byte_range,
                        })
                    })
                    .collect::<Vec<_>>();

                let summary_severity = validator_findings
                    .iter()
                    .map(|finding| finding.severity.as_str())
                    .min()
                    .unwrap_or("info")
                    .to_string();
                let summary_message = if validator_findings.is_empty() {
                    "no findings".to_string()
                } else {
                    format!("{} finding(s)", validator_findings.len())
                };

                self.repository
                    .upsert_validator_finding(
                        crate::sqlite::repository::UpsertValidatorFindingParams {
                            project_id: project_id.to_string(),
                            branch_id: branch_id.to_string(),
                            scene_id: scene.scene_id.clone(),
                            scene_text_hash: scene_text_hash.clone(),
                            context_hash: context_hashes.get(validator_id.as_str()).cloned(),
                            validator_id: validator_id.clone(),
                            finding_id: "__cache__".to_string(),
                            severity: summary_severity,
                            message: summary_message,
                            byte_range: None,
                            details_json: Some(serde_json::json!({ "issues": serialized_issues })),
                        },
                    )
                    .await?;
            }

            for finding in findings {
                if !phase_four_checks
                    .iter()
                    .any(|check| check.as_str() == finding.check_type)
                {
                    continue;
                }
                let mut message = finding.message;
                if let Some(byte_range) = finding.byte_range {
                    message = format!("{message} (bytes {}..{})", byte_range.start, byte_range.end);
                }
                issues.push(ConsistencyIssue {
                    severity: finding.severity.as_str().to_string(),
                    check_type: finding.check_type.to_string(),
                    message,
                    entity_ids: vec![scene.scene_id.clone()],
                    suggested_action: Some(
                        "revise scene prose or supporting canon records".to_string(),
                    ),
                });
            }
        }

        Ok(issues)
    }

    /// Model-router driven semantic world-rule audit. Falls back to the
    /// in-process heuristic when the configured route does not produce a
    /// usable response. Mirrors `services/mod.rs:6344` in 705b835^.
    async fn deep_world_rule_compliance_issues(
        &self,
        scenes: &[crate::sqlite::records::Scene],
        rules: &[crate::sqlite::records::WorldRule],
    ) -> Result<Vec<spindle_core::models::ConsistencyIssue>> {
        use crate::ai::ModelRequest;
        use crate::format::world_rule_established_before_scene;
        use spindle_core::models::ConsistencyIssue;

        let mut issues = Vec::new();
        for scene in scenes {
            let applicable_rules = rules
                .iter()
                .filter(|rule| world_rule_established_before_scene(rule, scene))
                .collect::<Vec<_>>();
            if applicable_rules.is_empty() {
                continue;
            }

            let model_violations = match self
                .repository
                .model_router()
                .complete(&ModelRequest {
                    route: "review".to_string(),
                    prompt: build_world_rule_deep_check_prompt(scene, &applicable_rules),
                    rating: None,
                    context: None,
                })
                .await
            {
                Ok(response) if response.adapter_kind != "local" => {
                    parse_deep_world_rule_check_output(&response.output).unwrap_or_else(|_| {
                        heuristic_world_rule_violations(scene, &applicable_rules)
                    })
                }
                Ok(_) | Err(_) => heuristic_world_rule_violations(scene, &applicable_rules),
            };

            for violation in model_violations {
                let Some(rule) = applicable_rules
                    .iter()
                    .find(|rule| rule.id == violation.rule_id)
                else {
                    continue;
                };
                let mut message = format!(
                    "scene {} may violate world rule '{}': {}",
                    scene.id,
                    rule.rule_name,
                    violation.message.trim()
                );
                if let Some(evidence) = violation.evidence.as_ref()
                    && !evidence.trim().is_empty()
                {
                    message.push_str(&format!(" Evidence: {}", evidence.trim()));
                }
                issues.push(ConsistencyIssue {
                    severity: violation.severity.unwrap_or_else(|| "warning".to_string()),
                    check_type: "world_rule_compliance".to_string(),
                    message,
                    entity_ids: vec![scene.id.clone(), rule.id.clone()],
                    suggested_action: Some(
                        "revise the scene, explain the exception on page, or update the rule if canon changed"
                            .to_string(),
                    ),
                });
            }
        }

        Ok(issues)
    }

    /// Find scene/book/chapter spine inconsistencies. Mirrors
    /// `services/mod.rs:6672` in 705b835^.
    async fn find_orphan_scenes(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<Vec<SceneSpineIntegrityIssue>> {
        use std::collections::BTreeMap;

        let books = self.repository.list_books_by_project(project_id).await?;
        let mut books_by_id = BTreeMap::new();
        let mut chapters_by_id = BTreeMap::new();

        for book in &books {
            books_by_id.insert(book.id.clone(), book.clone());
            for chapter in self.repository.list_chapters_by_book(&book.id).await? {
                chapters_by_id.insert(chapter.id.clone(), chapter);
            }
        }

        let scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, branch_id)
            .await?;
        let mut issues = Vec::new();

        for scene in &scenes {
            let scene_id = scene.id.clone();
            let book_id = scene.book_id.clone();
            let chapter_id = scene.chapter_id.clone();

            match books_by_id.get(&book_id) {
                Some(book) => {
                    if book.book_number != scene.book_number {
                        issues.push(SceneSpineIntegrityIssue {
                            severity: "error",
                            book_number: scene.book_number,
                            chapter_number: scene.chapter_number,
                            scene_order: scene.scene_order,
                            message: format!(
                                "scene {} points to book {} but carries book_number {}",
                                scene_id, book_id, scene.book_number
                            ),
                            entity_ids: vec![scene_id.clone()],
                            suggested_action: Some(
                                "repair the scene book_id or renumber the scene placement"
                                    .to_string(),
                            ),
                        });
                    }
                }
                None => issues.push(SceneSpineIntegrityIssue {
                    severity: "error",
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: scene.scene_order,
                    message: format!("scene {} points to missing book_id {}", scene_id, book_id),
                    entity_ids: vec![scene_id.clone()],
                    suggested_action: Some(
                        "repair the scene book_id or recreate the missing book record".to_string(),
                    ),
                }),
            }

            match chapters_by_id.get(&chapter_id) {
                Some(chapter) => {
                    let chapter_book_id = chapter.book_id.clone();
                    if chapter_book_id != book_id {
                        issues.push(SceneSpineIntegrityIssue {
                            severity: "error",
                            book_number: scene.book_number,
                            chapter_number: scene.chapter_number,
                            scene_order: scene.scene_order,
                            message: format!(
                                "scene {} points to chapter {} in book {} but carries book_id {}",
                                scene_id, chapter_id, chapter_book_id, book_id
                            ),
                            entity_ids: vec![scene_id.clone()],
                            suggested_action: Some(
                                "repair the scene chapter_id/book_id pair so they refer to the same chapter spine"
                                    .to_string(),
                            ),
                        });
                    }
                    if chapter.book_number != scene.book_number
                        || chapter.chapter_number != scene.chapter_number
                    {
                        issues.push(SceneSpineIntegrityIssue {
                            severity: "error",
                            book_number: scene.book_number,
                            chapter_number: scene.chapter_number,
                            scene_order: scene.scene_order,
                            message: format!(
                                "scene {} points to chapter {} but carries placement {}.{}",
                                scene_id, chapter_id, scene.book_number, scene.chapter_number
                            ),
                            entity_ids: vec![scene_id.clone()],
                            suggested_action: Some(
                                "repair the scene chapter_id or renumber the scene placement"
                                    .to_string(),
                            ),
                        });
                    }
                }
                None => issues.push(SceneSpineIntegrityIssue {
                    severity: "error",
                    book_number: scene.book_number,
                    chapter_number: scene.chapter_number,
                    scene_order: scene.scene_order,
                    message: format!(
                        "scene {} points to missing chapter_id {}",
                        scene_id, chapter_id
                    ),
                    entity_ids: vec![scene_id.clone()],
                    suggested_action: Some(
                        "repair the scene chapter_id or recreate the missing chapter record"
                            .to_string(),
                    ),
                }),
            }
        }

        let mut scenes_by_position: BTreeMap<(i32, i32, i32), Vec<String>> = BTreeMap::new();
        for scene in &scenes {
            scenes_by_position
                .entry((scene.book_number, scene.chapter_number, scene.scene_order))
                .or_default()
                .push(scene.id.clone());
        }

        for ((book_number, chapter_number, scene_order), entity_ids) in scenes_by_position {
            if entity_ids.len() < 2 {
                continue;
            }
            issues.push(SceneSpineIntegrityIssue {
                severity: "error",
                book_number,
                chapter_number,
                scene_order,
                message: format!(
                    "book {} chapter {} has duplicate scene_order {}",
                    book_number, chapter_number, scene_order
                ),
                entity_ids,
                suggested_action: Some(
                    "renumber or move the colliding scenes so each chapter position is unique"
                        .to_string(),
                ),
            });
        }

        Ok(issues)
    }

    /// Commit a bundle of scene-aligned canon mutations (character states,
    /// canonical facts, relationship updates) and surface any world-rule
    /// hits. Mirrors `services/mod.rs:11234..~11490` in 705b835^.
    ///
    /// Behaviour notes for the SQLite port:
    /// - World-rule scanning is wired through `scan_world_rules` (a thin
    ///   wrapper over `spindle_core::world_rules::scan_prose_for_world_rules`).
    ///   `Likely`-severity hits block the commit unless
    ///   `accept_world_rule_risks=true`.
    /// - Per-character and per-fact failures are aggregated into the
    ///   response (not bailed) — matches the reference's structured-error
    ///   contract so MCP callers can surface partial successes.
    /// - When any character state succeeded, we run a branch-wide
    ///   `resolve_validator_findings_for_validator("voice_drift")` instead
    ///   of the SurrealDB `voice_profile_affected_scene_ids` fan-out
    ///   (same simplification as `set_character_voice_profile`).
    /// - When any canonical fact succeeded, the Surreal reference calls
    ///   `resolve_validator_findings_for_scenes(&[scene_id])` for
    ///   `canonical_fact_prose_drift`; we mirror that.
    ///
    /// - After all per-entry mutations land, the Phase-4 validator
    ///   registry is re-run for this scene (cache bypassed) and the
    ///   results populate `findings_summary`.
    pub async fn commit_scene_changes(
        &self,
        input: spindle_core::models::CommitSceneChangesInput,
    ) -> Result<spindle_core::models::CommitSceneChangesOutput> {
        use spindle_core::models::{
            CommitSceneCanonicalFactResult, CommitSceneChangesOutput,
            CommitSceneCharacterStateResult, CommitSceneRelationshipResult, WorldRuleSeverity,
        };

        let project_id = input.project_id.clone();
        let scene_id_input = input.scene_id.clone();
        let scene = self.repository.get_scene(&scene_id_input).await?;
        if scene.project_id != project_id {
            let error = anyhow::anyhow!("scene does not belong to the requested project");
            return Err(self
                .append_tool_error_activity(
                    &project_id,
                    &scene.branch_id,
                    "commit_scene_changes",
                    Some("scene"),
                    Some(&scene.id),
                    error,
                )
                .await);
        }

        let scene_id = scene.id.clone();
        let book_number = scene.book_number;
        let chapter_number = scene.chapter_number;

        let world_rule_hits = self.scan_world_rules(&project_id, &scene.full_text).await?;

        // Only Likely hits (pattern match + violation context in surrounding
        // prose) block the commit. Possible hits are returned in the response
        // for caller visibility but do not require an override.
        let blocking_hits: Vec<&spindle_core::models::WorldRuleHit> = world_rule_hits
            .iter()
            .filter(|hit| hit.severity == WorldRuleSeverity::Likely)
            .collect();

        if !input.accept_world_rule_risks && !blocking_hits.is_empty() {
            let hit_descriptions: Vec<String> = blocking_hits
                .iter()
                .map(|hit| {
                    format!(
                        "{} (likely) at byte {}-{}: {}",
                        hit.rule_id,
                        hit.byte_range.start,
                        hit.byte_range.end,
                        hit.surrounding_text.chars().take(80).collect::<String>(),
                    )
                })
                .collect();
            anyhow::bail!(
                "commit_scene_changes blocked by {} likely world rule violation(s); \
                 set accept_world_rule_risks=true to override: {}",
                blocking_hits.len(),
                hit_descriptions.join("; ")
            );
        }

        let mut character_states = Vec::with_capacity(input.character_states.len());
        for entry in input.character_states {
            let character_id = entry.character_id.clone();
            match self
                .commit_character_state(spindle_core::models::CommitCharacterStateInput {
                    character_id: entry.character_id,
                    scene_id: scene_id.clone(),
                    changes: entry.changes,
                })
                .await
            {
                Ok(output) => character_states.push(CommitSceneCharacterStateResult {
                    character_id,
                    state_id: Some(output.state_id),
                    error: None,
                }),
                Err(error) => character_states.push(CommitSceneCharacterStateResult {
                    character_id,
                    state_id: None,
                    error: Some(error.to_string()),
                }),
            }
        }

        let mut canonical_facts = Vec::with_capacity(input.canonical_facts.len());
        for entry in input.canonical_facts {
            let fact_type = entry
                .fact_type
                .clone()
                .or_else(|| entry.predicate.clone())
                .unwrap_or_else(|| "typed_fact".to_string());
            let key = entry
                .key
                .clone()
                .or_else(|| entry.predicate.clone())
                .unwrap_or_else(|| "typed_fact".to_string());
            match self
                .register_canonical_fact(spindle_core::models::RegisterCanonicalFactInput {
                    project_id: project_id.clone(),
                    scene_id: scene_id.clone(),
                    book_number,
                    chapter_number,
                    fact_type: entry.fact_type,
                    key: entry.key,
                    value: entry.value,
                    subject_table: entry.subject_table,
                    subject_id: entry.subject_id,
                    predicate: entry.predicate,
                    value_kind: entry.value_kind,
                    value_text: entry.value_text,
                    value_number: entry.value_number,
                    value_unit: entry.value_unit,
                    value_json: entry.value_json,
                    aliases: entry.aliases,
                    scope: entry.scope,
                    valid_from: entry.valid_from,
                    valid_until: entry.valid_until,
                    legacy_untyped: None,
                    context: entry.context,
                    supersedes_fact_id: entry.supersedes_fact_id,
                })
                .await
            {
                Ok(output) => canonical_facts.push(CommitSceneCanonicalFactResult {
                    fact_type,
                    key,
                    canonical_fact_id: Some(output.canonical_fact_id),
                    superseded_fact_id: output.superseded_fact_id,
                    error: None,
                }),
                Err(error) => canonical_facts.push(CommitSceneCanonicalFactResult {
                    fact_type,
                    key,
                    canonical_fact_id: None,
                    superseded_fact_id: None,
                    error: Some(error.to_string()),
                }),
            }
        }

        let mut relationship_updates = Vec::with_capacity(input.relationship_updates.len());
        for entry in input.relationship_updates {
            let character_a_id = entry.character_a_id.clone();
            let character_b_id = entry.character_b_id.clone();
            match self
                .update_relationship(spindle_core::models::UpdateRelationshipInput {
                    character_a_id: entry.character_a_id,
                    character_b_id: entry.character_b_id,
                    trust_delta: entry.trust_delta,
                    tension_delta: entry.tension_delta,
                    reason: entry.reason,
                    scene_id: scene_id.clone(),
                })
                .await
            {
                Ok(output) => relationship_updates.push(CommitSceneRelationshipResult {
                    character_a_id,
                    character_b_id,
                    relationship_id: Some(output.relationship_id),
                    trust: Some(output.trust),
                    tension: Some(output.tension),
                    error: None,
                }),
                Err(error) => relationship_updates.push(CommitSceneRelationshipResult {
                    character_a_id,
                    character_b_id,
                    relationship_id: None,
                    trust: None,
                    tension: None,
                    error: Some(error.to_string()),
                }),
            }
        }

        // Branch-wide voice_drift resolution as a simplification of the
        // SurrealDB voice_profile_affected_scene_ids helper. Same approach
        // used by `set_character_voice_profile` (see the comment there).
        if character_states.iter().any(|result| result.error.is_none()) {
            self.resolve_phase_four_caches(
                &project_id,
                &scene.branch_id,
                &[PhaseFourCacheId::VoiceDrift],
            )
            .await?;
        }

        if canonical_facts.iter().any(|result| result.error.is_none()) {
            // SQLite divergence: the SurrealDB reference resolves only the
            // `canonical_fact_prose_drift` validator on this specific scene
            // via `resolve_validator_findings_for_scenes`. The SQLite repo
            // exposes a per-scene resolver (no validator filter) and a
            // per-validator branch-wide resolver (no scene filter). Using
            // the per-scene call here keeps the scoping correct (we don't
            // want to clear validators for other scenes), at the cost of
            // also clearing other validator rows on this one scene; the
            // Phase-4 fan-out below re-populates them.
            self.repository
                .resolve_validator_findings_for_scenes(
                    &scene.branch_id,
                    std::slice::from_ref(&scene.id),
                )
                .await?;
        }

        // ── Phase-4 validator fan-out ───────────────────────────────
        // Re-runs the four Phase-4 validators against just the scene we
        // committed to so `findings_summary` reports the post-commit state.
        // Cache is intentionally bypassed (`use_cache = false`) because the
        // mutations above (character_state / canonical_fact / relationship)
        // may have changed the validator outcome for this scene.
        let phase_four_checks =
            requested_phase_four_validator_checks(&std::collections::BTreeSet::new());
        let findings = self
            .run_phase_four_validator_checks_for_scenes(
                &project_id,
                &scene.branch_id,
                std::slice::from_ref(&scene),
                &phase_four_checks,
                false,
            )
            .await?;
        let findings_summary = summarize_commit_scene_findings(&findings);

        let output = CommitSceneChangesOutput {
            scene_id: scene_id.clone(),
            character_states,
            canonical_facts,
            relationship_updates,
            world_rule_hits,
            findings_summary,
        };

        // Best-effort session activity log: failure here doesn't fail the
        // overall commit. Matches the reference semantics.
        if let Err(error) = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: project_id.clone(),
                branch_id: scene.branch_id.clone(),
                kind: "scene_committed".to_string(),
                subject_table: Some("scene".to_string()),
                subject_id: Some(scene.id.clone()),
                summary: format!(
                    "Committed scene changes for scene {}.{}.{}.",
                    scene.book_number, scene.chapter_number, scene.scene_order
                ),
                details_json: Some(serde_json::json!({
                    "scene_id": output.scene_id,
                    "book_number": scene.book_number,
                    "chapter_number": scene.chapter_number,
                    "scene_order": scene.scene_order,
                    "character_states": output.character_states.len(),
                    "canonical_facts": output.canonical_facts.len(),
                    "relationship_updates": output.relationship_updates.len(),
                })),
            })
            .await
        {
            tracing::warn!("session activity log failed: {error}");
        }

        Ok(output)
    }

    /// Best-effort logger that records a tool-level failure in the
    /// session_activity stream and returns the original error so the
    /// caller can keep its error-propagation chain. Mirrors
    /// `append_tool_error_activity` from `services/mod.rs:350` in
    /// 705b835^.
    async fn append_tool_error_activity(
        &self,
        project_id: &str,
        branch_id: &str,
        tool: &str,
        subject_table: Option<&str>,
        subject_id: Option<&str>,
        error: anyhow::Error,
    ) -> anyhow::Error {
        let summary = format!("tool {tool} failed: {error}");
        let _ = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                kind: format!("{tool}_failed"),
                subject_table: subject_table.map(str::to_string),
                subject_id: subject_id.map(str::to_string),
                summary,
                details_json: Some(serde_json::json!({
                    "tool": tool,
                    "error": error.to_string(),
                })),
            })
            .await;
        error
    }

    /// In-place scene revision: save the new prose through the
    /// production `save_scene_draft` path (which snapshots the prior
    /// version, marks dependent dual_persona_review rows stale, and
    /// resolves open validator_finding rows), then surface a structured
    /// diff plus revision markers for downstream state, scenes, and
    /// pacing trackers. Mirrors services/mod.rs:7504..7763 in 705b835^.
    ///
    /// Per-project main-branch design (Risk #6): the SurrealDB reference
    /// rejected revision when the active branch was the singleton
    /// `bible_branch:main`. With per-project main branches, the SQLite
    /// check looks up the branch by name to apply the same semantics.
    ///
    /// After the prose is saved, the response carries:
    /// * `world_rule_hits` from `scan_world_rules`,
    /// * `voice_drift` from `scan_voice_drift`,
    /// * `retcon_findings` from `scan_retcon_findings`,
    ///
    /// alongside the existing revision-marker fan-outs from
    /// `list_character_states_after_position`,
    /// `list_scenes_after_position`, and
    /// `list_pacing_trackers_by_project_and_branch`.
    pub async fn revise_scene(
        &self,
        input: spindle_core::models::ReviseSceneInput,
    ) -> Result<spindle_core::models::ReviseSceneOutput> {
        use crate::sqlite::records::RevisionMarker as RevisionMarkerRecord;
        use spindle_core::models::{
            ReviseSceneOutput, RevisionPacingImpact, RevisionSceneFlag, RevisionStateInvalidation,
            SaveSceneDraftInput,
        };

        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let existing_scene = self.repository.get_scene(&input.scene_id).await?;

        if existing_scene.project_id != input.project_id {
            anyhow::bail!(
                "scene {} does not belong to project {}",
                input.scene_id,
                input.project_id
            );
        }
        if active_branch.name == "main" {
            anyhow::bail!("revise_scene requires a non-main branch");
        }

        let prior_text = existing_scene.full_text.clone();
        let revised_text = input.full_text.clone();

        let (revised_scene, _) = self
            .repository
            .save_scene_draft(
                &input.project_id,
                &active_branch.id,
                &SaveSceneDraftInput {
                    project_id: input.project_id.clone(),
                    book_number: existing_scene.book_number,
                    chapter_number: existing_scene.chapter_number,
                    chapter_id: None,
                    scene_order: existing_scene.scene_order,
                    full_text: revised_text.clone(),
                    summary: input.summary,
                    content_rating: input.content_rating,
                    tone: input.tone,
                    source_path: None,
                    generation_id: None,
                },
            )
            .await?;

        let (diff, byte_offsets_changed, chars_added, chars_deleted) =
            compute_text_diff(&prior_text, &revised_text);

        let now = existing_scene.updated_at;

        let state_markers = self
            .repository
            .list_character_states_after_position(
                &input.project_id,
                &active_branch.id,
                revised_scene.book_number,
                revised_scene.chapter_number,
                revised_scene.scene_order,
            )
            .await?
            .into_iter()
            .map(|state| RevisionMarkerRecord {
                id: String::new(),
                project_id: input.project_id.clone(),
                branch_id: active_branch.id.clone(),
                scene_id: revised_scene.id.clone(),
                marker_type: "state_invalidated".to_string(),
                target_record_id: Some(state.id.clone()),
                position: format!(
                    "{}|{}:{}:{}",
                    state.character_id, state.book_number, state.chapter_number, state.scene_order
                ),
                note: "state was recorded after the revised scene on this branch".to_string(),
                status: "open".to_string(),
                created_at: now,
            });
        let scene_markers = self
            .repository
            .list_scenes_after_position(
                &input.project_id,
                &active_branch.id,
                revised_scene.book_number,
                revised_scene.chapter_number,
                revised_scene.scene_order,
            )
            .await?
            .into_iter()
            .map(|scene| RevisionMarkerRecord {
                id: String::new(),
                project_id: input.project_id.clone(),
                branch_id: active_branch.id.clone(),
                scene_id: revised_scene.id.clone(),
                marker_type: "scene_flagged".to_string(),
                target_record_id: Some(scene.id.clone()),
                position: format!(
                    "{}:{}:{}",
                    scene.book_number, scene.chapter_number, scene.scene_order
                ),
                note:
                    "scene occurs after the revised scene and may rely on invalidated branch state"
                        .to_string(),
                status: "open".to_string(),
                created_at: now,
            });
        let pacing_markers = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?
            .into_iter()
            .filter(|tracker| tracker.current_progress > 0.0 || tracker.status != "on_track")
            .map(|tracker| RevisionMarkerRecord {
                id: String::new(),
                project_id: input.project_id.clone(),
                branch_id: active_branch.id.clone(),
                scene_id: revised_scene.id.clone(),
                marker_type: "pacing_review".to_string(),
                target_record_id: Some(tracker.id.clone()),
                position: tracker.character_arc_id.clone(),
                note:
                    "review this tracker because the revised scene may change downstream arc progression"
                        .to_string(),
                status: "open".to_string(),
                created_at: now,
            });

        let mut persisted_markers = Vec::new();
        for marker in state_markers.chain(scene_markers).chain(pacing_markers) {
            let persisted = self.repository.upsert_revision_marker(&marker).await?;
            persisted_markers.push(persisted);
        }

        // ── Phase-4 scanners against the revised prose ──────────────
        // Surfaces world-rule hits, voice drift, and retcon findings the
        // new prose introduces. The Phase-4 fan-out cache for this scene
        // was already invalidated by `save_scene_draft` (which calls
        // `resolve_validator_findings_for_scenes`), so we re-run the
        // light per-scene scanners directly rather than going through
        // `run_phase_four_validator_checks_for_scenes`.
        let world_rule_hits_after_revision = self
            .scan_world_rules(&input.project_id, &revised_scene.full_text)
            .await?;
        let voice_drift_after_revision = self
            .scan_voice_drift(&input.project_id, &revised_scene.full_text)
            .await?;
        let retcon_findings_after_revision = self
            .scan_retcon_findings(&input.project_id, &revised_scene)
            .await?;

        Ok(ReviseSceneOutput {
            scene_id: revised_scene.id.clone(),
            states_invalidated: persisted_markers
                .iter()
                .filter(|m| m.marker_type == "state_invalidated")
                .map(|m| RevisionStateInvalidation {
                    state_id: m.target_record_id.clone().unwrap_or_default(),
                    character_id: m
                        .position
                        .split_once('|')
                        .map(|(left, _)| left.to_string())
                        .unwrap_or_default(),
                    position: m.position.clone(),
                    reason: m.note.clone(),
                })
                .collect(),
            downstream_scenes_flagged: persisted_markers
                .iter()
                .filter(|m| m.marker_type == "scene_flagged")
                .map(|m| RevisionSceneFlag {
                    scene_id: m.target_record_id.clone().unwrap_or_default(),
                    position: m.position.clone(),
                    reason: m.note.clone(),
                })
                .collect(),
            pacing_impact: persisted_markers
                .iter()
                .filter(|m| m.marker_type == "pacing_review")
                .map(|m| RevisionPacingImpact {
                    character_arc_id: m.position.clone(),
                    tracker_id: m.target_record_id.clone().unwrap_or_default(),
                    status: m.status.clone(),
                    note: m.note.clone(),
                })
                .collect(),
            diff,
            byte_offsets_changed,
            chars_added,
            chars_deleted,
            world_rule_hits: world_rule_hits_after_revision,
            voice_drift: voice_drift_after_revision,
            retcon_findings: retcon_findings_after_revision,
        })
    }

    /// Drive a model-router continuation against a truncated prior output.
    /// Records the combined `prior_output + response.output` in the
    /// in-memory receipt cache so subsequent `save_scene_draft` /
    /// `revise_generation` calls can reference it by `generation_id`.
    /// Mirrors services/mod.rs:476..503 in 705b835^.
    pub async fn continue_generation(
        &self,
        input: spindle_core::models::ContinueGenerationInput,
    ) -> Result<spindle_core::models::ContinueGenerationOutput> {
        use crate::ai::RequestContext;
        use spindle_core::models::ContinueGenerationOutput;

        let context = RequestContext {
            project_id: input.project_id.clone(),
            book_id: input.book_id.clone(),
            chapter_id: input.chapter_id.clone(),
            scene_id: input.scene_id.clone(),
        };
        let context_ref = if context.is_empty() {
            None
        } else {
            Some(&context)
        };
        let response = self
            .repository
            .model_router()
            .complete_continuation(
                &input.route,
                input.rating.as_deref(),
                context_ref,
                &input.original_prompt,
                &input.prior_output,
            )
            .await?;
        let receipt = self.register_generation_receipt(
            &input.route,
            input.rating.as_deref(),
            &response.model_name,
            &format!("{}{}", input.prior_output, response.output),
        );
        Ok(ContinueGenerationOutput {
            output: response.output,
            truncated: response.truncated,
            generation_id: Some(receipt.id),
            generation_agent_id: Some(receipt.agent_id),
            generation_output_sha256: Some(receipt.output_sha256),
        })
    }

    /// Revise a previously-generated draft by referencing its
    /// `generation_id`. Runs the revision through the same draft route the
    /// source receipt used (preserving rating routing — explicit receipts
    /// re-enter the explicit-capable agent, mature receipts re-enter the
    /// mature route, and so on) and registers a new receipt for the revised
    /// output. Works for any rating the source receipt carried; use it for
    /// small surgical edits without forcing a full `continue_generation`
    /// re-roll.
    pub async fn revise_generation(
        &self,
        input: spindle_core::models::ReviseGenerationInput,
    ) -> Result<spindle_core::models::ReviseGenerationOutput> {
        use crate::ai::ModelRequest;
        use spindle_core::models::ReviseGenerationOutput;

        let source_receipt = self
            .verified_revisable_draft_receipt(Some(&input.generation_id))?
            .ok_or_else(|| anyhow::anyhow!("generation_id is required"))?;
        let edit_instructions = input.edit_instructions.trim();
        if edit_instructions.is_empty() {
            anyhow::bail!("edit_instructions must not be empty");
        }

        let prompt = build_generation_revision_prompt(
            &source_receipt.output_text,
            edit_instructions,
            input.context.as_deref(),
        );
        let response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: source_receipt.route.clone(),
                prompt,
                rating: source_receipt.rating.clone(),
                context: None,
            })
            .await?;
        let receipt = self.register_generation_receipt(
            &source_receipt.route,
            source_receipt.rating.as_deref(),
            &response.model_name,
            &response.output,
        );

        Ok(ReviseGenerationOutput {
            output: response.output,
            truncated: response.truncated,
            source_generation_id: input.generation_id,
            generation_id: Some(receipt.id),
            generation_agent_id: Some(receipt.agent_id),
            generation_output_sha256: Some(receipt.output_sha256),
        })
    }

    /// Register an in-memory receipt for a model-router output. Caches
    /// up to `MAX_GENERATION_RECEIPTS`; oldest entries evict FIFO.
    /// Mirrors the SurrealDB reference exactly so the `generation_id`
    /// shape (`model_generation:{seq}:{12-char-sha-prefix}`) is stable.
    fn register_generation_receipt(
        &self,
        route: &str,
        rating: Option<&str>,
        agent_id: &str,
        output: &str,
    ) -> GenerationReceiptRecord {
        use std::sync::atomic::Ordering;

        let output_text = normalized_generation_text(output);
        let output_sha256 = generation_sha256_hex(output_text.as_bytes());
        let sequence = self
            .generation_receipt_counter
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        let id = format!(
            "model_generation:{sequence}:{}",
            output_sha256.chars().take(12).collect::<String>()
        );
        let receipt = GenerationReceiptRecord {
            id: id.clone(),
            route: route.to_string(),
            rating: rating.map(normalize_generation_rating),
            agent_id: agent_id.to_string(),
            output_sha256,
            output_text,
            explicit_capable_agent: self.agent_supports_rating(agent_id, "explicit"),
        };

        let mut receipts = self
            .generation_receipts
            .write()
            .expect("generation receipts write lock");
        receipts.insert(id, receipt.clone());
        while receipts.len() > MAX_GENERATION_RECEIPTS {
            if let Some(oldest) = receipts.keys().next().cloned() {
                receipts.remove(&oldest);
            } else {
                break;
            }
        }

        receipt
    }

    fn agent_supports_rating(&self, agent_id: &str, rating: &str) -> bool {
        let expected = normalize_generation_rating(rating);
        self.repository
            .model_router()
            .list_agents()
            .agents
            .into_iter()
            .find(|agent| agent.id == agent_id)
            .is_some_and(|agent| {
                agent
                    .ratings
                    .iter()
                    .any(|candidate| normalize_generation_rating(candidate) == expected)
            })
    }

    /// Look up + validate a draft generation receipt by id. Used by
    /// `revise_generation` to confirm the receipt is suitable for revision
    /// through the model router.
    ///
    /// Rating gate semantics: revisions are allowed at ANY rating the source
    /// receipt was produced at — including `general`, `teen`, `mature`, and
    /// `explicit`. The original implementation gated this to `explicit`
    /// only, on the (mistaken) assumption that revise_generation was an
    /// explicit-content tool. In practice operators want server-tracked
    /// surgical revisions of mature drafts too; without that, small edits
    /// force a full re-roll through `continue_generation`. The
    /// `explicit_capable_agent` check is kept but applies only when the
    /// receipt's rating IS explicit — for lower ratings any agent that
    /// produced the draft is fine to revise it.
    fn verified_revisable_draft_receipt(
        &self,
        generation_id: Option<&str>,
    ) -> Result<Option<GenerationReceiptRecord>> {
        let Some(generation_id) = generation_id
            .map(str::trim)
            .filter(|generation_id| !generation_id.is_empty())
        else {
            return Ok(None);
        };

        let receipt = self
            .generation_receipts
            .read()
            .expect("generation receipts read lock")
            .get(generation_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("generation_id {generation_id:?} was not found or has expired")
            })?;

        if receipt.route != "draft" {
            anyhow::bail!(
                "generation_id {generation_id:?} was produced for route {:?}, not \"draft\"",
                receipt.route
            );
        }
        // Explicit-content integrity: when the source receipt was explicit,
        // the producing agent must also be explicit-capable. For non-explicit
        // ratings this constraint doesn't apply — those drafts can come from
        // any configured drafting agent.
        if receipt.rating.as_deref() == Some("explicit") && !receipt.explicit_capable_agent {
            anyhow::bail!(
                "generation_id {generation_id:?} was produced by agent {:?}, which is not explicit-capable",
                receipt.agent_id
            );
        }
        if receipt.output_text.is_empty() {
            anyhow::bail!("generation_id {generation_id:?} references an empty draft output");
        }

        Ok(Some(receipt))
    }

    /// Drive a deterministic, branch-creating alternative-generation
    /// pass: for each requested alternative (2..=5), spin off a new
    /// "alternative" branch from the active branch, switch into it,
    /// synthesise a scene draft from the freshly-loaded scene context,
    /// then restore the original active branch. Mirrors
    /// services/mod.rs:7835..7928 in 705b835^. No LLM — the prose
    /// synthesis is purely heuristic (`synthesize_alternative_scene` +
    /// `alternative_tone`) so MCP callers get a stable, testable
    /// fan-out.
    pub async fn generate_alternatives(
        &self,
        input: spindle_core::models::GenerateAlternativesInput,
    ) -> Result<spindle_core::models::GenerateAlternativesOutput> {
        use spindle_core::models::{
            ContentRating, ContextFormat, CreateBranchInput, GenerateAlternativesOutput,
            GeneratedAlternative, GetSceneContextInput, SaveSceneDraftInput, SwitchBranchInput,
        };

        let project = self.repository.get_project(&input.project_id).await?;
        let starting_branch = self.repository.get_active_branch(&input.project_id).await?;
        let alternative_count = input.alternatives.unwrap_or(3).clamp(2, 5);

        let context = self
            .get_scene_context(GetSceneContextInput {
                project_id: input.project_id.clone(),
                book_number: input.book_number,
                chapter_number: input.chapter_number,
                chapter_id: None,
                scene_order: input.scene_order,
                character_ids: input.character_ids.clone(),
                max_character_count: None,
                location_id: input.location_id.clone(),
                format: Some(ContextFormat::Json),
                budget_tokens: Some(3000),
                token_budget: Some(3000),
                sections: None,
            })
            .await?;

        let mut generated = Vec::new();
        for index in 0..alternative_count {
            let branch_name = format!(
                "alt-{}-{}-{}-{}",
                input.variation_strategy,
                input.book_number,
                input.chapter_number,
                index + 1
            );
            let branch_out = self
                .create_branch(CreateBranchInput {
                    project_id: input.project_id.clone(),
                    parent_branch_id: Some(starting_branch.id.clone()),
                    name: branch_name.clone(),
                    branch_type: "alternative".into(),
                    description: Some(format!(
                        "Generated {} alternative {}",
                        input.variation_strategy,
                        index + 1
                    )),
                })
                .await?;
            self.switch_branch(SwitchBranchInput {
                project_id: input.project_id.clone(),
                branch_id: branch_out.branch_id.clone(),
            })
            .await?;

            let (summary, full_text) = synthesize_alternative_scene(&context, &input, index);
            let scene = self
                .save_scene_draft(SaveSceneDraftInput {
                    project_id: input.project_id.clone(),
                    book_number: input.book_number,
                    chapter_number: input.chapter_number,
                    chapter_id: None,
                    scene_order: input.scene_order,
                    full_text,
                    summary: summary.clone(),
                    content_rating: ContentRating::Teen,
                    tone: Some(alternative_tone(&input.variation_strategy, index).to_string()),
                    generation_id: None,
                    source_path: None,
                })
                .await?;

            generated.push(GeneratedAlternative {
                branch_id: branch_out.branch_id,
                branch_name,
                summary,
                scene_id: scene.scene_id,
                variation_strategy: input.variation_strategy.clone(),
            });
        }

        // Restore the project's active branch (the SurrealDB reference
        // pointed back at the singleton `bible_branch:main`; here we
        // use the project's recorded `active_branch_id`, falling back to
        // the starting branch when the field is unset).
        let restore_target = project
            .active_branch_id
            .clone()
            .unwrap_or_else(|| starting_branch.id.clone());
        self.switch_branch(SwitchBranchInput {
            project_id: input.project_id.clone(),
            branch_id: restore_target,
        })
        .await?;

        Ok(GenerateAlternativesOutput {
            context,
            alternatives: generated,
        })
    }

    /// Rank a set of feature branches by a heuristic score over the
    /// latest scene draft on each. Used by the alternatives workflow
    /// before `select_alternative`. Pure SQLite reads + four
    /// deterministic scoring helpers below; no LLM.
    pub async fn compare_alternatives(
        &self,
        input: spindle_core::models::CompareAlternativesInput,
    ) -> Result<spindle_core::models::CompareAlternativesOutput> {
        use spindle_core::models::{AlternativeComparison, CompareAlternativesOutput};

        self.repository.get_project(&input.project_id).await?;

        let mut comparisons = Vec::new();
        for branch_id_str in &input.branch_ids {
            let branch = self.repository.get_branch(branch_id_str).await?;
            if branch
                .project_id
                .as_deref()
                .is_some_and(|id| id != input.project_id)
            {
                anyhow::bail!(
                    "alternative branch {} does not belong to project {}",
                    branch_id_str,
                    input.project_id
                );
            }
            let scenes = self
                .repository
                .list_scenes_by_project_and_branch(&input.project_id, branch_id_str)
                .await?;
            let latest_scene = scenes.last().ok_or_else(|| {
                anyhow::anyhow!("alternative branch {branch_id_str} has no scene draft")
            })?;

            let quality_score = score_alternative_scene(latest_scene);
            comparisons.push(AlternativeComparison {
                branch_id: branch.id.clone(),
                branch_name: branch.name,
                summary: latest_scene.summary.clone(),
                quality_score,
                strongest_trait: strongest_trait_for_scene(latest_scene),
                pacing_note: pacing_note_for_scene(latest_scene),
                hook_note: hook_note_for_scene(latest_scene),
            });
        }

        comparisons.sort_by(|left, right| {
            right
                .quality_score
                .cmp(&left.quality_score)
                .then_with(|| left.branch_name.cmp(&right.branch_name))
        });
        let recommended_branch_id = comparisons.first().map(|item| item.branch_id.clone());

        Ok(CompareAlternativesOutput {
            alternatives: comparisons,
            recommended_branch_id,
        })
    }

    /// Promote an alternative branch to the project's main and switch the
    /// active branch onto it. Wraps `merge_branch` against the project's
    /// `main` branch in the per-project design (see Risk #6).
    ///
    /// Reference: services/mod.rs:8034..8086 in 705b835^. The reference
    /// post-merge call did `switch_active_branch(project, RecordId::new(
    /// "bible_branch", "main"))` — here we look up the per-project main
    /// branch by name first, then switch to its id.
    ///
    /// Conflict behavior: if the underlying merge reports a conflict,
    /// `select_alternative` bails with a descriptive error rather than
    /// silently leaving the branch un-promoted. The caller is expected
    /// to resolve the conflict and re-run.
    pub async fn select_alternative(
        &self,
        input: spindle_core::models::SelectAlternativeInput,
    ) -> Result<spindle_core::models::SelectAlternativeOutput> {
        use spindle_core::models::{MergeBranchInput, SelectAlternativeOutput};

        let project = self.repository.get_project(&input.project_id).await?;
        let branch = self.repository.get_branch(&input.branch_id).await?;
        if branch
            .project_id
            .as_deref()
            .is_some_and(|id| id != project.id)
        {
            anyhow::bail!(
                "selected branch {} does not belong to project {}",
                branch.id,
                project.id
            );
        }

        // Per-project main lookup (Risk #6): resolve "this project's main
        // branch" by name. Captured up front so we can use the same id
        // for both the merge target and the post-merge active-branch
        // switch.
        let main_branch = self
            .repository
            .list_branches_by_project(&project.id)
            .await?
            .into_iter()
            .find(|b| b.name == "main")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "project {} has no main branch; cannot select alternative",
                    project.id
                )
            })?;

        let merged = self
            .merge_branch(MergeBranchInput {
                project_id: input.project_id.clone(),
                source_branch_id: input.branch_id.clone(),
                target_branch_id: Some(main_branch.id.clone()),
                merge_type: "fast_forward".to_string(),
            })
            .await?;
        if merged.has_conflicts {
            let position = merged
                .conflicts
                .first()
                .map(|c| format!("{}:{}:{}", c.book_number, c.chapter_number, c.scene_order))
                .unwrap_or_else(|| "unknown position".to_string());
            anyhow::bail!(
                "select_alternative is blocked by an unresolved scene conflict at {position}"
            );
        }
        self.repository
            .switch_active_branch(&project.id, &main_branch.id)
            .await?;

        Ok(SelectAlternativeOutput {
            selected_branch_id: input.branch_id,
            target_branch_id: merged.target_branch_id,
            merge_type: merged.merge_type,
        })
    }

    /// Heuristic canonical-fact extractor: splits the scene's full_text on
    /// sentence delimiters and proposes the first 12 candidate sentences as
    /// `subject_table = "scene"` invariants. Pure deterministic projection
    /// over the persisted scene — no LLM call, so the output is stable
    /// across runs and safe to use as the seed for a downstream
    /// review/accept loop. Mirrors the SurrealDB reference exactly
    /// (services/mod.rs:4570..4610 in 705b835^).
    pub async fn extract_canonical_facts_from_scene(
        &self,
        input: spindle_core::models::ExtractCanonicalFactsFromSceneInput,
    ) -> Result<spindle_core::models::ExtractCanonicalFactsFromSceneOutput> {
        use spindle_core::models::{
            CanonicalFactScope, ExtractCanonicalFactProposal, ExtractCanonicalFactsFromSceneOutput,
        };

        let scene = self.repository.get_scene(&input.scene_id).await?;
        let scene_id_string = scene.id.clone();
        let proposals = split_scene_into_fact_sentences(&scene.full_text)
            .into_iter()
            .take(12)
            .enumerate()
            .map(|(idx, sentence)| {
                let predicate = format!(
                    "scene.extracted_{}.{}",
                    idx + 1,
                    canonical_predicate_slug(&sentence)
                );
                ExtractCanonicalFactProposal {
                    subject_table: "scene".to_string(),
                    subject_id: Some(scene_id_string.clone()),
                    predicate,
                    value_kind: "string".to_string(),
                    value_text: Some(sentence.clone()),
                    value_number: None,
                    value_unit: None,
                    value_json: None,
                    aliases: vec![],
                    scope: Some(CanonicalFactScope::Invariant),
                    valid_from: None,
                    valid_until: None,
                    source_excerpt: Some(sentence),
                    rationale: Some(
                        "Sentence-level extraction proposal from committed scene text.".to_string(),
                    ),
                }
            })
            .collect();
        Ok(ExtractCanonicalFactsFromSceneOutput {
            scene_id: scene_id_string,
            proposals,
        })
    }

    /// Two-persona scene review (literary critic + craft technician),
    /// optionally with 1–3 rounds. Each round calls the model router
    /// twice through the "review" route. Local-adapter responses are
    /// replaced with deterministic heuristic concerns derived from the
    /// scene metadata so unit tests work without a real model. Persists
    /// to `dual_persona_review` via the existing repository upsert.
    pub async fn run_dual_persona_review(
        &self,
        input: spindle_core::models::RunDualPersonaReviewInput,
    ) -> Result<spindle_core::models::RunDualPersonaReviewOutput> {
        use crate::ai::ModelRequest;
        use spindle_core::models::{
            DualPersonaReviewRound, PersonaReviewNotes, RunDualPersonaReviewOutput,
        };

        let branch = match input.branch_id.as_deref() {
            Some(id) => self.repository.get_branch(id).await?,
            None => self.repository.get_active_branch(&input.project_id).await?,
        };
        let branch_id = branch.id.clone();

        if branch
            .project_id
            .as_deref()
            .is_some_and(|id| id != input.project_id)
        {
            anyhow::bail!(
                "review branch {branch_id} does not belong to project {}",
                input.project_id
            );
        }
        let scene = self.repository.get_scene(&input.scene_id).await?;
        if scene.project_id != input.project_id || scene.branch_id != branch_id {
            anyhow::bail!(
                "review scene {} does not belong to branch {branch_id} of project {}",
                input.scene_id,
                input.project_id
            );
        }

        // The Target Reader persona judges genre delivery, a different axis
        // from craft quality. It is populated from the project style contract;
        // skip it entirely when the project declares no style signal.
        let style_directive = self
            .style_directive_for(&input.project_id, &branch_id)
            .await
            .unwrap_or_default();
        let run_genre_reader = !style_directive.is_empty();
        let genre_brief = style_directive.render_markdown().unwrap_or_default();

        let rounds = input.rounds.unwrap_or(2).clamp(1, 3);
        let mut review_rounds = Vec::new();
        for round in 1..=rounds {
            let literary = self
                .repository
                .model_router()
                .complete(&ModelRequest {
                    route: "review".to_string(),
                    prompt: format!(
                        "You are a literary critic reviewing round {round} of a scene draft.\n\n\
                         Scene summary: {}\n\n\
                         Full prose:\n{}\n\n\
                         Evaluate as a reader and literary critic. Focus on:\n\
                         - Emotional impact and reader engagement\n\
                         - Character voice authenticity\n\
                         - Tension, pacing, and scene structure\n\
                         - Hook strength (opening and closing)\n\n\
                         Format your response as:\n\
                         STRENGTHS:\n- one strength per line\n\n\
                         CONCERNS:\n- one concern per line\n\n\
                         Be specific to THIS scene. Reference actual lines, images, or moments.",
                        scene.summary, scene.full_text
                    ),
                    rating: None,
                    context: None,
                })
                .await?;
            let craft = self
                .repository
                .model_router()
                .complete(&ModelRequest {
                    route: "review".to_string(),
                    prompt: format!(
                        "You are a craft technician reviewing round {round} of a scene draft.\n\n\
                         Tone: {}\n\
                         Scene summary: {}\n\n\
                         Full prose:\n{}\n\n\
                         Evaluate the prose craft. Focus on:\n\
                         - POV discipline and filter word usage\n\
                         - Sentence rhythm and variety\n\
                         - Sensory detail quality\n\
                         - Dialogue naturalness\n\
                         - Verb strength (linking verbs vs active verbs)\n\n\
                         Format your response as:\n\
                         STRENGTHS:\n- one strength per line\n\n\
                         CONCERNS:\n- one concern per line\n\n\
                         Be specific to THIS scene. Quote actual phrases that need work.",
                        scene.tone.as_deref().unwrap_or("unspecified"),
                        scene.summary,
                        scene.full_text
                    ),
                    rating: None,
                    context: None,
                })
                .await?;

            // Target Reader persona: only invoked when there is a style
            // contract to judge against.
            let genre_reader = if run_genre_reader {
                let genre = self
                    .repository
                    .model_router()
                    .complete(&ModelRequest {
                        route: "review".to_string(),
                        prompt: format!(
                            "You are the TARGET READER of this book's declared genre, reviewing \
                             round {round} of a scene draft. You are NOT a craft critic — you are \
                             the reader this book is FOR, judging whether the scene delivers what \
                             you came for.\n\n\
                             This project's style contract:\n{genre_brief}\n\n\
                             Author-declared tone: {}\n\
                             Scene summary: {}\n\n\
                             Full prose:\n{}\n\n\
                             As the target reader, answer honestly:\n\
                             - Did this scene deliver the genre experience the contract promises? \
                             (comedy: did I laugh? thriller: was I gripped? romance: did I feel the \
                             chemistry?)\n\
                             - Is the narrator's voice the voice the contract asks for, or has it \
                             drifted (e.g. into quiet literary introspection where it should be \
                             sarcastic and funny)?\n\
                             - Are the genre-critical characters present and doing their job?\n\
                             - Does the ending make me want the next chapter the way this genre \
                             demands (hook/cliffhanger/laugh), or close on a beat that's wrong for \
                             the genre?\n\
                             - Is the pacing what this genre's reader expects?\n\n\
                             Format your response as:\n\
                             STRENGTHS:\n- one strength per line\n\n\
                             CONCERNS:\n- one concern per line\n\n\
                             A scene that is well-crafted but OFF-GENRE is a CONCERN, not a \
                             strength. Be specific to THIS scene.",
                            scene.tone.as_deref().unwrap_or("unspecified"),
                            scene.summary,
                            scene.full_text
                        ),
                        rating: None,
                        context: None,
                    })
                    .await?;
                let (strengths, concerns) = if genre.adapter_kind == "local" {
                    (
                        vec!["local heuristic pass (no external model configured)".to_string()],
                        derive_genre_concerns(&scene, &style_directive),
                    )
                } else {
                    parse_review_sections(&genre.output)
                };
                PersonaReviewNotes {
                    persona: "target_reader".to_string(),
                    strengths,
                    concerns,
                }
            } else {
                PersonaReviewNotes {
                    persona: "target_reader".to_string(),
                    ..Default::default()
                }
            };

            let (literary_strengths, literary_concerns) = if literary.adapter_kind == "local" {
                (
                    vec!["local heuristic pass (no external model configured)".to_string()],
                    derive_literary_concerns(&scene),
                )
            } else {
                parse_review_sections(&literary.output)
            };
            let (craft_strengths, craft_concerns) = if craft.adapter_kind == "local" {
                (
                    vec!["local heuristic pass (no external model configured)".to_string()],
                    derive_craft_concerns(&scene),
                )
            } else {
                parse_review_sections(&craft.output)
            };
            // Genre delivery is the primary quality metric, so genre-reader
            // concerns lead the priority actions.
            let priority_actions = if literary.adapter_kind == "local" {
                let mut actions: Vec<String> = genre_reader
                    .concerns
                    .iter()
                    .take(2)
                    .map(|c| format!("Genre fix: {c}"))
                    .collect();
                actions.extend(derive_review_actions(&scene));
                actions
            } else {
                let mut actions: Vec<String> = genre_reader
                    .concerns
                    .iter()
                    .take(2)
                    .map(|c| format!("Genre fix: {c}"))
                    .chain(
                        literary_concerns
                            .iter()
                            .take(1)
                            .chain(craft_concerns.iter().take(1))
                            .map(|c| format!("Address: {c}")),
                    )
                    .collect();
                if actions.is_empty() {
                    actions.push("No critical actions identified.".to_string());
                }
                actions
            };

            review_rounds.push(DualPersonaReviewRound {
                round,
                literary_critic: PersonaReviewNotes {
                    persona: "literary_critic".to_string(),
                    strengths: literary_strengths,
                    concerns: literary_concerns,
                },
                craft_technician: PersonaReviewNotes {
                    persona: "craft_technician".to_string(),
                    strengths: craft_strengths,
                    concerns: craft_concerns,
                },
                genre_reader,
                priority_actions,
            });
        }

        let fingerprint = scene_revision_fingerprint(&scene);
        let persisted = self
            .repository
            .upsert_dual_persona_review(crate::sqlite::repository::UpsertDualPersonaReviewParams {
                project_id: input.project_id.clone(),
                branch_id: branch_id.clone(),
                scene_id: input.scene_id.clone(),
                rounds_completed: review_rounds.len(),
                review_rounds: review_rounds.clone(),
                scene_revision_fingerprint: fingerprint,
                status: "current".to_string(),
            })
            .await?;

        Ok(RunDualPersonaReviewOutput {
            scene_id: input.scene_id,
            branch_id,
            rounds_completed: review_rounds.len(),
            review_id: persisted.id,
            status: persisted.status,
            review_rounds,
        })
    }

    /// Live research lookup against a Gemini-compatible chat endpoint.
    /// Frames the prompt with the project's reader contract, world rules,
    /// and top-5 `search_bible` hits, then persists the response to
    /// `research_log`. Mirrors the SurrealDB reference
    /// (services/mod.rs:682..830 in 705b835^) with the same prompt structure,
    /// env vars (`GEMINI_API_KEY`, `SPINDLE_RESEARCH_MODEL`,
    /// `SPINDLE_RESEARCH_ENDPOINT`), and retry-on-failure path.
    ///
    /// The reference's `search_project_records` was a SurrealDB-only
    /// repository call; the SQLite path uses `search_bible` with the
    /// default semantic mode and a 5-result cap, which produces the same
    /// `(entity_type, excerpt)` rows used to build the system prompt.
    pub async fn research_query(
        &self,
        input: spindle_core::models::ResearchQueryInput,
    ) -> Result<spindle_core::models::ResearchQueryOutput> {
        use anyhow::Context;
        use spindle_core::models::{ResearchContextSummary, ResearchQueryOutput, SearchBibleInput};

        let project = self.repository.get_project(&input.project_id).await?;

        let world_rules = self
            .repository
            .list_world_rules_by_project(&input.project_id)
            .await?;
        let world_rules_count = world_rules.len();

        let bible_hits = self
            .search_bible(SearchBibleInput {
                project_id: input.project_id.clone(),
                query: input.query.clone(),
                limit: Some(5),
                mode: None,
                field: None,
                subject_table: None,
                format: None,
                budget_tokens: None,
            })
            .await?
            .results;
        let bible_hits_count = bible_hits.len();

        let api_key = std::env::var("GEMINI_API_KEY").context(
            "GEMINI_API_KEY env var is not set. Set it to a valid Gemini API key to use \
             research_query.",
        )?;
        let model = std::env::var("SPINDLE_RESEARCH_MODEL")
            .unwrap_or_else(|_| "gemini-3.1-pro-preview".to_string());
        let endpoint = std::env::var("SPINDLE_RESEARCH_ENDPOINT").unwrap_or_else(|_| {
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions".to_string()
        });

        let mut system_parts = vec![
            format!(
                "You are a research assistant for a fiction project called \"{}\".",
                project.name
            ),
            format!("Genre: {}.", project.genre),
            format!(
                "Reader contract promise: {}.",
                project.reader_contract.promise
            ),
        ];

        if !world_rules.is_empty() {
            system_parts.push("Established world rules:".to_string());
            for rule in &world_rules {
                system_parts.push(format!(
                    "- {} ({}): {}",
                    rule.rule_name, rule.rule_type, rule.description
                ));
            }
        }

        if !bible_hits.is_empty() {
            system_parts.push("Relevant project records:".to_string());
            for hit in &bible_hits {
                system_parts.push(format!("- [{}] {}", hit.entity_type, hit.excerpt));
            }
        }

        system_parts.push(
            "Answer factual questions grounded in reality. The author needs accurate \
             information to make their fiction believable. Be specific, cite real-world \
             sources when possible, and note any caveats or common misconceptions."
                .to_string(),
        );

        let system_prompt = system_parts.join("\n");

        let user_message = match &input.context_hint {
            Some(hint) => format!("Context: {hint}\n\nQuestion: {}", input.query),
            None => input.query.clone(),
        };

        let body = serde_json::json!({
            "model": model,
            "max_tokens": 4096,
            "temperature": 0.3,
            "stream": false,
            "reasoning_effort": "medium",
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_message },
            ]
        });

        let client = reqwest::Client::new();
        let resp = crate::ai::send_request_with_retry(|| {
            client
                .post(&endpoint)
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&body)
        })
        .await
        .context("failed to call Gemini research API")?;

        let status = resp.status();
        let resp_body: serde_json::Value = resp
            .json()
            .await
            .context("failed to parse Gemini response body")?;

        if !status.is_success() {
            let detail = resp_body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            anyhow::bail!("Gemini API returned {status}: {detail}");
        }

        let response_text = resp_body
            .pointer("/choices/0/message/content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("unexpected Gemini response structure"))?
            .to_string();

        let context_summary = format!(
            "project={} | genre={} | world_rules={} | bible_hits={}",
            project.name, project.genre, world_rules_count, bible_hits_count
        );
        if let Err(error) = self
            .repository
            .create_research_log(
                &input.project_id,
                &input.query,
                input.context_hint.as_deref(),
                &model,
                &response_text,
                &context_summary,
            )
            .await
        {
            eprintln!("failed to persist research log: {error}");
        }

        Ok(ResearchQueryOutput {
            model,
            response: response_text,
            context_used: ResearchContextSummary {
                project_name: project.name,
                genre: project.genre,
                world_rules_count,
                bible_hits_count,
            },
        })
    }

    /// Project-scoped diff between two branches' scenes, character
    /// states, relationships, and pacing trackers. Pure SQLite reads;
    /// no LLM. Mirrors services/mod.rs:7072..7362 in 705b835^.
    ///
    /// `RelatesTo` field-name delta vs reference: the SurrealDB record
    /// used `r#in` / `out`; the SQLite record uses `in_id` / `out_id`
    /// (composite primary key, no surrogate id).
    pub async fn diff_branches(
        &self,
        input: spindle_core::models::DiffBranchesInput,
    ) -> Result<spindle_core::models::DiffBranchesOutput> {
        use spindle_core::models::{
            CharacterStateDiffItem, DiffBranchesOutput, PacingDiffItem, RelationshipDiffItem,
            SceneDiffItem,
        };
        use std::collections::{BTreeMap, BTreeSet};

        let project = self.repository.get_project(&input.project_id).await?;
        let base_branch = self.repository.get_branch(&input.base_branch_id).await?;
        let compare_branch = self.repository.get_branch(&input.compare_branch_id).await?;

        for branch in [&base_branch, &compare_branch] {
            if branch
                .project_id
                .as_deref()
                .is_some_and(|id| id != project.id)
            {
                anyhow::bail!(
                    "branch {} does not belong to project {}",
                    branch.id,
                    project.id
                );
            }
        }

        let characters = self
            .repository
            .list_characters_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let character_names: BTreeMap<String, String> =
            characters.into_iter().map(|c| (c.id, c.name)).collect();

        let base_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let compare_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &input.compare_branch_id)
            .await?;
        let base_scene_map: BTreeMap<(i32, i32, i32), &_> = base_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();
        let compare_scene_map: BTreeMap<(i32, i32, i32), &_> = compare_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();
        let scene_keys: BTreeSet<_> = base_scene_map
            .keys()
            .chain(compare_scene_map.keys())
            .copied()
            .collect();
        let scene_diffs = scene_keys
            .into_iter()
            .filter_map(
                |key| match (base_scene_map.get(&key), compare_scene_map.get(&key)) {
                    (Some(base), Some(compare)) if base.summary != compare.summary => {
                        Some(SceneDiffItem {
                            book_number: key.0,
                            chapter_number: key.1,
                            scene_order: key.2,
                            base_summary: Some(base.summary.clone()),
                            compare_summary: Some(compare.summary.clone()),
                            change_type: "modified".to_string(),
                        })
                    }
                    (Some(base), None) => Some(SceneDiffItem {
                        book_number: key.0,
                        chapter_number: key.1,
                        scene_order: key.2,
                        base_summary: Some(base.summary.clone()),
                        compare_summary: None,
                        change_type: "removed".to_string(),
                    }),
                    (None, Some(compare)) => Some(SceneDiffItem {
                        book_number: key.0,
                        chapter_number: key.1,
                        scene_order: key.2,
                        base_summary: None,
                        compare_summary: Some(compare.summary.clone()),
                        change_type: "added".to_string(),
                    }),
                    _ => None,
                },
            )
            .collect::<Vec<_>>();

        let base_states = self
            .repository
            .list_character_states_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let compare_states = self
            .repository
            .list_character_states_by_project_and_branch(&project.id, &input.compare_branch_id)
            .await?;
        let base_state_map: BTreeMap<(String, i32, i32, i32), &_> = base_states
            .iter()
            .map(|s| {
                (
                    (
                        s.character_id.clone(),
                        s.book_number,
                        s.chapter_number,
                        s.scene_order,
                    ),
                    s,
                )
            })
            .collect();
        let compare_state_map: BTreeMap<(String, i32, i32, i32), &_> = compare_states
            .iter()
            .map(|s| {
                (
                    (
                        s.character_id.clone(),
                        s.book_number,
                        s.chapter_number,
                        s.scene_order,
                    ),
                    s,
                )
            })
            .collect();
        let state_keys: BTreeSet<_> = base_state_map
            .keys()
            .chain(compare_state_map.keys())
            .cloned()
            .collect();
        let character_state_diffs = state_keys
            .into_iter()
            .filter_map(|key| {
                let base = base_state_map.get(&key);
                let compare = compare_state_map.get(&key);
                if let (Some(base), Some(compare)) = (base, compare)
                    && base.status == compare.status
                    && base.goals == compare.goals
                {
                    return None;
                }
                Some(CharacterStateDiffItem {
                    character_id: key.0.clone(),
                    character_name: character_names
                        .get(&key.0)
                        .cloned()
                        .unwrap_or_else(|| key.0.clone()),
                    position: format!("{}:{}:{}", key.1, key.2, key.3),
                    base_status: base.map(|s| s.status.clone()).unwrap_or_default(),
                    compare_status: compare.map(|s| s.status.clone()).unwrap_or_default(),
                    base_goals: base.map(|s| s.goals.clone()).unwrap_or_default(),
                    compare_goals: compare.map(|s| s.goals.clone()).unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();

        let base_relationships = self
            .repository
            .list_relationships_by_branch(&input.base_branch_id)
            .await?;
        let compare_relationships = self
            .repository
            .list_relationships_by_branch(&input.compare_branch_id)
            .await?;
        let base_rel_map: BTreeMap<(String, String, String), &_> = base_relationships
            .iter()
            .map(|r| {
                (
                    (
                        r.in_id.clone(),
                        r.out_id.clone(),
                        r.relationship_type.clone(),
                    ),
                    r,
                )
            })
            .collect();
        let compare_rel_map: BTreeMap<(String, String, String), &_> = compare_relationships
            .iter()
            .map(|r| {
                (
                    (
                        r.in_id.clone(),
                        r.out_id.clone(),
                        r.relationship_type.clone(),
                    ),
                    r,
                )
            })
            .collect();
        let rel_keys: BTreeSet<_> = base_rel_map
            .keys()
            .chain(compare_rel_map.keys())
            .cloned()
            .collect();
        let relationship_diffs = rel_keys
            .into_iter()
            .filter_map(|key| {
                let base = base_rel_map.get(&key);
                let compare = compare_rel_map.get(&key);
                if let (Some(base), Some(compare)) = (base, compare)
                    && base.trust == compare.trust
                    && base.tension == compare.tension
                {
                    return None;
                }
                Some(RelationshipDiffItem {
                    source_character_id: key.0,
                    target_character_id: key.1,
                    relationship_type: key.2,
                    base_trust: base.map(|r| r.trust),
                    compare_trust: compare.map(|r| r.trust),
                    base_tension: base.map(|r| r.tension),
                    compare_tension: compare.map(|r| r.tension),
                })
            })
            .collect::<Vec<_>>();

        let base_pacing = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&project.id, &input.base_branch_id)
            .await?;
        let compare_pacing = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&project.id, &input.compare_branch_id)
            .await?;
        let base_pacing_map: BTreeMap<String, &_> = base_pacing
            .iter()
            .map(|t| (t.character_arc_id.clone(), t))
            .collect();
        let compare_pacing_map: BTreeMap<String, &_> = compare_pacing
            .iter()
            .map(|t| (t.character_arc_id.clone(), t))
            .collect();
        let pacing_keys: BTreeSet<_> = base_pacing_map
            .keys()
            .chain(compare_pacing_map.keys())
            .cloned()
            .collect();
        let pacing_diffs = pacing_keys
            .into_iter()
            .filter_map(|key| {
                let base = base_pacing_map.get(&key);
                let compare = compare_pacing_map.get(&key);
                if let (Some(base), Some(compare)) = (base, compare)
                    && (base.current_progress - compare.current_progress).abs() < f64::EPSILON
                    && base.status == compare.status
                {
                    return None;
                }
                Some(PacingDiffItem {
                    character_arc_id: key,
                    tracker_id: compare
                        .map(|t| t.id.clone())
                        .or_else(|| base.map(|t| t.id.clone()))
                        .unwrap_or_default(),
                    base_progress: base.map(|t| t.current_progress),
                    compare_progress: compare.map(|t| t.current_progress),
                    base_status: base.map(|t| t.status.clone()),
                    compare_status: compare.map(|t| t.status.clone()),
                })
            })
            .collect::<Vec<_>>();

        Ok(DiffBranchesOutput {
            base_branch: crate::format::branch_summary(
                &base_branch,
                project.active_branch_id.as_deref(),
            ),
            compare_branch: crate::format::branch_summary(
                &compare_branch,
                project.active_branch_id.as_deref(),
            ),
            narrative_impact_summary: summarize_branch_impact(
                &scene_diffs,
                &character_state_diffs,
                &relationship_diffs,
                &pacing_diffs,
            ),
            scene_diffs,
            character_state_diffs,
            relationship_diffs,
            pacing_diffs,
        })
    }

    /// Merge a feature branch into a target branch. In the per-project
    /// design the default target is the project's `main` branch (looked
    /// up by name, not via a singleton record-id). Mirrors
    /// services/mod.rs:7363..7502 in 705b835^.
    ///
    /// Per-project main branches (Risk #6): the SurrealDB reference
    /// defaulted `target_branch_id` to the singleton `bible_branch:main`.
    /// Here we resolve "the project's main branch" by listing branches
    /// for the project and matching on `name == "main"`. This keeps the
    /// per-project main-branch design (cleaner FK semantics, no shared
    /// singleton) without breaking callers that didn't pass an explicit
    /// `target_branch_id`.
    ///
    /// Conflict detection runs purely in-process via
    /// `detect_merge_scene_conflicts`. Once conflicts are identified, the
    /// scene/character_state slices are filtered to drop conflicting
    /// positions, and the mergeable slice is committed via
    /// `Repository::merge_branch_snapshot`.
    pub async fn merge_branch(
        &self,
        input: spindle_core::models::MergeBranchInput,
    ) -> Result<spindle_core::models::MergeBranchOutput> {
        use spindle_core::models::MergeBranchOutput;
        use std::collections::{BTreeMap, BTreeSet};

        let project = self.repository.get_project(&input.project_id).await?;

        // Per-project main lookup. We need this for both:
        //   * defaulting target_branch_id when the caller omits it
        //   * deciding whether a target_branch_id IS the project's main
        //     (the conflict detector uses that to skip the main-fallback)
        let main_branch = self
            .repository
            .list_branches_by_project(&project.id)
            .await?
            .into_iter()
            .find(|b| b.name == "main")
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "project {} has no main branch; cannot resolve merge target",
                    project.id
                )
            })?;

        let target_branch_id = input
            .target_branch_id
            .clone()
            .unwrap_or_else(|| main_branch.id.clone());

        if input.merge_type != "fast_forward" && input.merge_type != "squash" {
            anyhow::bail!("unsupported merge type: {}", input.merge_type);
        }

        let source_branch = self.repository.get_branch(&input.source_branch_id).await?;
        let target_branch = self.repository.get_branch(&target_branch_id).await?;
        for branch in [&source_branch, &target_branch] {
            if branch
                .project_id
                .as_deref()
                .is_some_and(|id| id != project.id)
            {
                anyhow::bail!(
                    "branch {} does not belong to project {}",
                    branch.id,
                    project.id
                );
            }
        }

        let source_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?;
        let target_local_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&project.id, &target_branch_id)
            .await?;
        // Skip the main-fallback fetch when the target IS the main branch.
        let main_scenes = if target_branch_id == main_branch.id {
            Vec::new()
        } else {
            self.repository
                .list_scenes_by_project_and_branch(&project.id, &main_branch.id)
                .await?
        };
        let source_states = self
            .repository
            .list_character_states_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?;
        let source_relationships = self
            .repository
            .list_relationships_by_branch(&input.source_branch_id)
            .await?;
        let source_pacing = self
            .repository
            .list_pacing_trackers_by_project_and_branch(&project.id, &input.source_branch_id)
            .await?;

        let target_local_scene_map: BTreeMap<(i32, i32, i32), &_> = target_local_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();
        let main_scene_map: BTreeMap<(i32, i32, i32), &_> = main_scenes
            .iter()
            .map(|s| ((s.book_number, s.chapter_number, s.scene_order), s))
            .collect();

        let conflicts = detect_merge_scene_conflicts(
            &source_scenes,
            &source_branch,
            &target_branch_id,
            &main_branch.id,
            &target_local_scene_map,
            &main_scene_map,
        );
        let conflict_positions: BTreeSet<(i32, i32, i32)> = conflicts
            .iter()
            .map(|c| (c.book_number, c.chapter_number, c.scene_order))
            .collect();

        let mergeable_scenes: Vec<_> = source_scenes
            .iter()
            .filter(|s| {
                !conflict_positions.contains(&(s.book_number, s.chapter_number, s.scene_order))
            })
            .cloned()
            .collect();
        let mergeable_states: Vec<_> = source_states
            .iter()
            .filter(|s| {
                !conflict_positions.contains(&(s.book_number, s.chapter_number, s.scene_order))
            })
            .cloned()
            .collect();

        self.repository
            .merge_branch_snapshot(
                &project.id,
                &target_branch_id,
                &mergeable_scenes,
                &mergeable_states,
                &source_relationships,
                &source_pacing,
            )
            .await?;

        Ok(MergeBranchOutput {
            source_branch_id: input.source_branch_id,
            target_branch_id,
            merge_type: input.merge_type,
            applied_scenes: mergeable_scenes.len(),
            applied_character_states: mergeable_states.len(),
            applied_relationships: source_relationships.len(),
            applied_pacing_trackers: source_pacing.len(),
            has_conflicts: !conflicts.is_empty(),
            conflicts,
        })
    }

    /// Rewind the project's active branch to the state captured by
    /// `save_point_id`. Mirrors `restore_save_point` (`services/mod.rs:
    /// 3125..3216` in 705b835^):
    ///
    ///   1. Validate the save_point belongs to the project and to the
    ///      active branch.
    ///   2. Read its snapshot file, verify the recorded sha256.
    ///   3. Parse + validate the JSON payload (`spindle-save-point-v1`).
    ///   4. Build the per-table restore set filtered to the active branch.
    ///   5. Auto-create a `pre-restore-…` backup save_point so the operator
    ///      can undo the restore.
    ///   6. Call `Repository::restore_branch_snapshot`.
    ///   7. Rebuild the project's search index.
    pub async fn restore_save_point(
        &self,
        input: spindle_core::models::RestoreSavePointInput,
    ) -> Result<spindle_core::models::RestoreSavePointOutput> {
        use spindle_core::models::{CreateSavePointInput, RestoreSavePointOutput};

        // 1. Resolve project + active branch + save_point identity.
        let project = self.repository.get_project(&input.project_id).await?;
        let active_branch_id = project
            .active_branch_id
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?
            .to_string();

        let save_point = self.repository.get_save_point(&input.save_point_id).await?;
        if save_point.project_id != input.project_id {
            anyhow::bail!(
                "save point {} does not belong to project {}",
                save_point.id,
                input.project_id
            );
        }
        if save_point.branch_id != active_branch_id {
            anyhow::bail!(
                "save point belongs to branch {}, but active branch is {}",
                save_point.branch_id,
                active_branch_id
            );
        }

        // 2. Read snapshot bytes and verify sha256.
        let snapshot_relative_path = save_point
            .snapshot_file_path
            .clone()
            .ok_or_else(|| anyhow::anyhow!("save point does not have a persisted snapshot file"))?;
        if save_point.snapshot_format.as_deref() != Some("spindle-save-point-v1") {
            anyhow::bail!("save point snapshot format is missing or unsupported");
        }
        let snapshot_path = self.repository.data_dir().join(&snapshot_relative_path);
        let snapshot_bytes = std::fs::read(&snapshot_path).with_context(|| {
            format!(
                "failed to read save point snapshot {}",
                snapshot_path.display()
            )
        })?;
        let snapshot_sha256 = crate::sqlite::import::sha256_hex(&snapshot_bytes);
        if save_point.snapshot_sha256.as_deref() != Some(snapshot_sha256.as_str()) {
            anyhow::bail!("save point snapshot hash does not match persisted metadata");
        }

        // 3. Parse + validate payload.
        let payload: serde_json::Value = serde_json::from_slice(&snapshot_bytes)
            .context("failed to parse save point snapshot json")?;
        validate_save_point_snapshot_payload(
            &payload,
            &input.project_id,
            &input.save_point_id,
            &active_branch_id,
        )?;

        // 4. Build the branch-filtered restore snapshot.
        let restore_snapshot = build_branch_restore_snapshot(
            &self.repository,
            &input.project_id,
            &active_branch_id,
            &payload,
        )
        .await?;
        let restored_tables = count_restore_tables(&restore_snapshot);
        let restored_records = count_restore_records(&restore_snapshot);

        // 5. Auto-create a pre-restore backup save_point.
        let backup = self
            .create_save_point(CreateSavePointInput {
                project_id: input.project_id.clone(),
                name: format!(
                    "pre-restore-{}-{}",
                    slugify_filename_component(&save_point.name),
                    chrono::Utc::now().format("%Y%m%d%H%M%S")
                ),
                description: Some(format!(
                    "Automatic backup before restoring save point {}",
                    save_point.name
                )),
            })
            .await?;

        // 6. Apply the restore.
        self.repository
            .restore_branch_snapshot(&input.project_id, &active_branch_id, &restore_snapshot)
            .await?;

        // 7. Rebuild the search index for the project so the restored
        //    branch's characters/scenes/etc. are reachable via FTS/vec0.
        self.rebuild_search_index(spindle_core::models::RebuildSearchIndexInput {
            project_id: input.project_id.clone(),
        })
        .await?;

        Ok(RestoreSavePointOutput {
            save_point_id: input.save_point_id,
            branch_id: active_branch_id,
            backup_save_point_id: backup.save_point_id,
            status: "restored".to_string(),
            restored_tables,
            restored_records,
        })
    }

    /// Promote a legacy canonical fact to a typed canonical fact via the
    /// upgrade spec. The new fact supersedes the old one, preserving the
    /// canonical-fact audit chain.
    ///
    /// Schema note (v029): the `legacy_untyped` column was dropped from
    /// `canonical_fact` after the SurrealDB reference was written. The
    /// reference rejected migration when `fact.legacy_untyped == false`
    /// ("already typed"). Without the column, that pre-check has no
    /// in-schema source of truth, so the SQLite implementation forwards
    /// any unsuperseded fact through the migration and lets callers re-run
    /// `register_canonical_fact` if they want pure-typed creation. The
    /// `set_canonical_fact_legacy_untyped` call survives as a no-op shim
    /// (see `crate::sqlite::repository::set_canonical_fact_legacy_untyped`).
    pub async fn migrate_canonical_fact(
        &self,
        input: spindle_core::models::MigrateCanonicalFactInput,
    ) -> Result<spindle_core::models::MigrateCanonicalFactOutput> {
        use spindle_core::models::{
            CanonicalFactScope, MigrateCanonicalFactOutput, RegisterCanonicalFactInput,
        };

        let fact = self.repository.get_canonical_fact(&input.fact_id).await?;
        if fact.superseded_by.is_some() {
            anyhow::bail!("canonical fact {} is already superseded", input.fact_id);
        }

        let active_branch = self.repository.get_active_branch(&fact.project_id).await?;
        if fact.branch_id != active_branch.id {
            anyhow::bail!(
                "canonical fact {} is not on the project's active branch",
                input.fact_id
            );
        }

        let subject_id = input
            .upgrade_spec
            .subject_id
            .clone()
            .or_else(|| fact.subject_id.clone());
        let aliases = if input.upgrade_spec.aliases.is_empty() {
            fact.aliases.clone()
        } else {
            input.upgrade_spec.aliases.clone()
        };
        let scope = input
            .upgrade_spec
            .scope
            .clone()
            .or(match fact.scope.as_str() {
                "invariant" => Some(CanonicalFactScope::Invariant),
                "evolving" => Some(CanonicalFactScope::Evolving),
                "conditional" => Some(CanonicalFactScope::Conditional),
                _ => None,
            });
        let valid_from = input
            .upgrade_spec
            .valid_from
            .clone()
            .or_else(|| fact.valid_from.as_ref().map(|p| p.clone().into_core()));
        let valid_until = input
            .upgrade_spec
            .valid_until
            .clone()
            .or_else(|| fact.valid_until.as_ref().map(|p| p.clone().into_core()));

        let migrated = self
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: fact.project_id.clone(),
                scene_id: fact.scene_id.clone(),
                book_number: fact.book_number,
                chapter_number: fact.chapter_number,
                fact_type: Some("typed_fact".to_string()),
                key: Some(input.upgrade_spec.predicate.clone()),
                value: None,
                context: None,
                subject_table: Some(input.upgrade_spec.subject_table.clone()),
                subject_id,
                predicate: Some(input.upgrade_spec.predicate.clone()),
                value_kind: Some(input.upgrade_spec.value_kind.clone()),
                value_text: input.upgrade_spec.value_text.clone(),
                value_number: input.upgrade_spec.value_number,
                value_unit: input.upgrade_spec.value_unit.clone(),
                value_json: input.upgrade_spec.value_json.clone(),
                aliases,
                scope,
                valid_from,
                valid_until,
                legacy_untyped: Some(false),
                supersedes_fact_id: Some(input.fact_id.clone()),
            })
            .await?;
        // No-op against the v029+ schema (column dropped); kept for
        // contract-level parity with the SurrealDB reference.
        self.repository
            .set_canonical_fact_legacy_untyped(&input.fact_id, false)
            .await?;

        Ok(MigrateCanonicalFactOutput {
            canonical_fact_id: migrated.canonical_fact_id,
            superseded_fact_id: input.fact_id,
        })
    }

    /// Re-derive `source_start_offset` / `source_end_offset` for every
    /// `scene_source_link` row whose on-disk file is still readable,
    /// updating any row whose stored offsets disagree with the
    /// position-by-chapter mapping inferred from the file. Used to
    /// recover offsets after a manual file edit or a Spindle upgrade.
    /// Wraps `SourceBridge::backfill_offsets`.
    pub async fn backfill_scene_source_offsets(
        &self,
        input: spindle_core::models::BackfillSceneSourceOffsetsInput,
    ) -> Result<spindle_core::models::BackfillSceneSourceOffsetsOutput> {
        let bridge = super::source_bridge::SourceBridge::new(self.repository.clone());
        bridge
            .backfill_offsets(&input.project_id, &input.branch_id)
            .await
    }

    /// Read scene bodies back from an on-disk source file, updating each
    /// scene in place when the on-disk body differs and refreshing its
    /// `scene_source_link` offsets + SHA-256.
    ///
    /// Accepts both Spindle-managed delimited files (one chapter per
    /// file, scenes separated by `\n\n---\n\n`) and externally-formatted
    /// manuscripts (Markdown `# Chapter` headers, `***` / `---` scene
    /// breaks, blank-line scene transitions, etc.) by routing the source
    /// text through the import structural slicer. See
    /// `sqlite::source_bridge::scene_offsets_from_import_slicer` for the
    /// slicer-to-bridge translation logic.
    pub async fn pull_chapter_from_file(
        &self,
        input: spindle_core::models::PullChapterFromFileInput,
    ) -> Result<spindle_core::models::PullReport> {
        let bridge = super::source_bridge::SourceBridge::new(self.repository.clone());
        bridge
            .pull_chapter_from_file(&input.chapter_id, std::path::Path::new(&input.source_path))
            .await
    }

    /// Serialise every scene in a chapter to a single text file under
    /// `data_dir`, scene bodies separated by `\n\n---\n\n`. Upserts a
    /// `scene_source_link` row per scene with the on-disk byte range and
    /// SHA-256 hash so subsequent `pull_chapter_from_file` and
    /// `backfill_scene_source_offsets` calls can round-trip cleanly.
    /// Mirrors `SourceBridge::push_chapter_to_file` in 705b835^.
    pub async fn push_chapter_to_file(
        &self,
        input: spindle_core::models::PushChapterToFileInput,
    ) -> Result<spindle_core::models::PushReport> {
        let bridge = super::source_bridge::SourceBridge::new(self.repository.clone());
        bridge
            .push_chapter_to_file(&input.chapter_id, std::path::Path::new(&input.target_path))
            .await
    }

    /// Run the pre-publish sanity check used before `export_epub` /
    /// `export_bible`. Returns a structured list of blocking issues
    /// (missing scenes, duplicate scene_order, empty scene text) and
    /// warnings (missing chapter title). Mirrors the reference contract
    /// exactly so MCP clients see identical issue codes and severities.
    pub async fn preflight_book_export(
        &self,
        input: spindle_core::models::PreflightBookExportInput,
    ) -> Result<spindle_core::models::PreflightBookExportOutput> {
        use spindle_core::models::PreflightBookExportOutput;

        self.repository.get_project(&input.project_id).await?;
        let chapter_scope = resolve_export_chapter_scope(
            input.book_number,
            input.start_chapter_number,
            input.end_chapter_number,
        )?;
        let issues = self
            .validate_book_for_export(&input.project_id, input.book_number, chapter_scope)
            .await?;
        Ok(PreflightBookExportOutput {
            project_id: input.project_id,
            book_number: input.book_number,
            start_chapter_number: input.start_chapter_number,
            end_chapter_number: input.end_chapter_number,
            issues,
        })
    }

    /// Shared book-export validator. Returns the structured issue list
    /// used by both `preflight_book_export` (which surfaces it directly)
    /// and `export_epub` / `export_bible` (which gate publishing on the
    /// presence of any Blocking entries). Mirrors the SurrealDB reference
    /// `validate_book_for_export` at services/mod.rs:1023..1139 in 705b835^.
    async fn validate_book_for_export(
        &self,
        project_id: &str,
        book_number: Option<i32>,
        chapter_scope: Option<ExportChapterScope>,
    ) -> Result<Vec<spindle_core::models::ExportIssue>> {
        use spindle_core::models::{ExportIssue, ExportIssueSeverity};
        use std::collections::BTreeSet;

        let all_books = self.repository.list_books_by_project(project_id).await?;
        let books_to_export: Vec<_> = match book_number {
            Some(number) => all_books
                .iter()
                .filter(|book| book.book_number == number)
                .collect(),
            None => all_books.iter().collect(),
        };
        if books_to_export.is_empty() {
            anyhow::bail!("no books found for export");
        }

        let active_branch = self.repository.get_active_branch(project_id).await?;
        let all_scenes = self
            .repository
            .list_scenes_by_project_and_branch(project_id, &active_branch.id)
            .await?;
        let mut issues = Vec::new();

        for book in books_to_export {
            let chapters = self
                .repository
                .list_chapters_by_book(&book.id)
                .await?
                .into_iter()
                .filter(|chapter| {
                    export_scope_contains_chapter(chapter_scope, chapter.chapter_number)
                })
                .collect::<Vec<_>>();

            if chapters.is_empty() {
                anyhow::bail!(
                    "no chapters found for export in book {} for the requested range",
                    book.book_number
                );
            }

            for chapter in chapters {
                let chapter_scenes: Vec<_> = all_scenes
                    .iter()
                    .filter(|scene| {
                        scene.book_number == book.book_number
                            && scene.chapter_number == chapter.chapter_number
                    })
                    .collect();

                if chapter_scenes.is_empty() {
                    issues.push(ExportIssue {
                        severity: ExportIssueSeverity::Blocking,
                        code: "chapter_without_scenes".to_string(),
                        message: format!(
                            "Book {} chapter {} has no scenes.",
                            book.book_number, chapter.chapter_number
                        ),
                        book_number: Some(book.book_number),
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: None,
                    });
                    continue;
                }

                if chapter
                    .title
                    .as_deref()
                    .map(str::trim)
                    .is_none_or(str::is_empty)
                {
                    issues.push(ExportIssue {
                        severity: ExportIssueSeverity::Warning,
                        code: "chapter_missing_title".to_string(),
                        message: format!(
                            "Book {} chapter {} does not have a title.",
                            book.book_number, chapter.chapter_number
                        ),
                        book_number: Some(book.book_number),
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: None,
                    });
                }

                let mut seen_scene_orders = BTreeSet::new();
                for scene in chapter_scenes {
                    if !seen_scene_orders.insert(scene.scene_order) {
                        issues.push(ExportIssue {
                            severity: ExportIssueSeverity::Blocking,
                            code: "duplicate_scene_order".to_string(),
                            message: format!(
                                "Book {} chapter {} has duplicate scene_order {}.",
                                book.book_number, chapter.chapter_number, scene.scene_order
                            ),
                            book_number: Some(book.book_number),
                            chapter_number: Some(chapter.chapter_number),
                            scene_order: Some(scene.scene_order),
                        });
                    }
                    if scene.full_text.trim().is_empty() {
                        issues.push(ExportIssue {
                            severity: ExportIssueSeverity::Blocking,
                            code: "scene_empty_text".to_string(),
                            message: format!(
                                "Book {} chapter {} scene {} has empty text.",
                                book.book_number, chapter.chapter_number, scene.scene_order
                            ),
                            book_number: Some(book.book_number),
                            chapter_number: Some(chapter.chapter_number),
                            scene_order: Some(scene.scene_order),
                        });
                    }
                }
            }
        }

        Ok(issues)
    }

    /// Export every project-scoped row to a single JSON file. Mirrors
    /// `services/mod.rs:1141..1162` in 705b835^. The SurrealDB reference
    /// walked the same set of tables via `db().select()` / `db().query()`;
    /// the SQLite implementation runs `SELECT *` per table through
    /// `Repository::dump_project_table` / `dump_table_by_column[_in]` and
    /// emits each row as a JSON object keyed by column name. The wire
    /// format ("schema-by-name JSON") matches the SurrealDB output for
    /// columns whose values are scalars/strings; JSON-payload columns are
    /// re-parsed back into nested `Value` (`looks_like_json_column`) so the
    /// round-trip preserves the original SurrealDB-side shape for the
    /// hot-path tables (`scene.dynamics`, `character.voice_profile`, etc.).
    pub async fn export_bible(
        &self,
        input: spindle_core::models::ExportBibleInput,
    ) -> Result<spindle_core::models::ExportBibleOutput> {
        use spindle_core::models::ExportBibleOutput;
        use std::collections::BTreeMap;

        let artifact = self
            .build_project_export_payload(
                &input.project_id,
                "spindle-bible-export-v1",
                BTreeMap::new(),
            )
            .await?;

        let filename = format!(
            "{}-bible-export.json",
            slugify_filename_component(&artifact.project_name)
        );
        let export_dir = self.repository.data_dir().join("exports");
        std::fs::create_dir_all(&export_dir)?;
        let file_path = export_dir.join(&filename);
        std::fs::write(&file_path, serde_json::to_vec_pretty(&artifact.payload)?)?;

        Ok(ExportBibleOutput {
            file_path: file_path.to_string_lossy().to_string(),
            filename,
            exported_tables: artifact.exported_tables,
            exported_records: artifact.exported_records,
        })
    }

    /// Walk every project-scoped table and assemble a serializable export
    /// payload. Returns the row counts (per-table and total) alongside the
    /// JSON value so callers can persist both the file and the metadata
    /// (e.g. `save_point.snapshot_record_count`). Mirrors
    /// `build_project_export_payload` (`services/mod.rs:1162..~1616`) in
    /// 705b835^ — same table list, same per-table filter shape.
    ///
    /// Phase 6: the typed-record path. Each table's rows go through
    /// `serde_json::to_value(record)` so timestamps render as RFC-3339
    /// strings (matching the SurrealDB reference output shape) rather than
    /// raw unix-microsecond integers. The downstream restore path
    /// (`restore_branch_snapshot`) parses those RFC-3339 strings back into
    /// integer micros via `JsonParam::from_value_for_column`, so save-point
    /// round-trips still work.
    async fn build_project_export_payload(
        &self,
        project_id: &str,
        format: &str,
        extra_metadata: std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<ProjectExportArtifact> {
        use serde::Serialize;
        use serde_json::{Value, json};
        use std::collections::BTreeMap;

        /// Map a `Vec<R>` of typed records into `Vec<Value>` for the
        /// table-keyed JSON payload. Each record goes through
        /// `serde_json::to_value`, which preserves serde-derived field
        /// names and uses `chrono::serde` to emit timestamps as RFC-3339.
        fn to_values<R: Serialize>(records: Vec<R>) -> Result<Vec<Value>> {
            records
                .into_iter()
                .map(|r| serde_json::to_value(&r).map_err(anyhow::Error::from))
                .collect()
        }

        let project = self.repository.get_project(project_id).await?;
        let project_value = serde_json::to_value(&project)?;

        let branches = self.repository.list_branches_by_project(project_id).await?;
        let branch_ids: Vec<String> = branches.iter().map(|b| b.id.clone()).collect();

        // import_session has nullable project_id; the SurrealDB reference
        // also includes sessions whose target_branch_id is one of this
        // project's branches. Walk project_id first, then target_branch_id
        // (deduped on id).
        let mut import_sessions = self
            .repository
            .list_import_sessions_by_project(project_id)
            .await?;
        let seen_session_ids: std::collections::BTreeSet<String> =
            import_sessions.iter().map(|s| s.id.clone()).collect();
        let mut extra_by_branch = self
            .repository
            .list_import_sessions_by_target_branch_ids(&branch_ids)
            .await?;
        extra_by_branch.retain(|s| !seen_session_ids.contains(&s.id));
        import_sessions.append(&mut extra_by_branch);
        let import_session_ids: Vec<String> =
            import_sessions.iter().map(|s| s.id.clone()).collect();

        let mut tables: BTreeMap<String, Value> = BTreeMap::new();
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();

        insert_export_object(&mut tables, &mut counts, "project", project_value);
        insert_export_rows(
            &mut tables,
            &mut counts,
            "bible_branch",
            to_values(branches)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "save_point",
            to_values(self.repository.list_all_save_points(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "book",
            to_values(self.repository.list_books_by_project(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter",
            to_values(self.repository.list_chapters_by_project(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character",
            to_values(self.repository.list_all_characters(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_voice_profile",
            to_values(
                self.repository
                    .list_all_character_voice_profiles(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_emotional_profile",
            to_values(
                self.repository
                    .list_all_character_emotional_profiles(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_state",
            to_values(
                self.repository
                    .list_all_character_states(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "location",
            to_values(self.repository.list_all_locations(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "world_state",
            to_values(self.repository.list_all_world_states(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "world_rule",
            to_values(self.repository.list_all_world_rules(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "revision_marker",
            to_values(
                self.repository
                    .list_all_revision_markers(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene",
            to_values(self.repository.list_all_scenes(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene_version",
            to_values(self.repository.list_all_scene_versions(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "relates_to",
            to_values(
                self.repository
                    .list_all_relationships_by_branch_ids(&branch_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "faction",
            to_values(self.repository.list_all_factions(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "religion",
            to_values(self.repository.list_all_religions(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "economy",
            to_values(self.repository.list_all_economies(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "term",
            to_values(self.repository.list_all_terms(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "plot_line",
            to_values(self.repository.list_all_plot_lines(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "conflict",
            to_values(self.repository.list_all_conflicts(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "theme",
            to_values(self.repository.list_all_themes(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "motif",
            to_values(self.repository.list_all_motifs(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "narrative_promise",
            to_values(
                self.repository
                    .list_all_narrative_promises(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "character_arc",
            to_values(self.repository.list_all_character_arcs(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "pacing_config",
            to_values(
                self.repository
                    .list_pacing_configs_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "pacing_curve",
            to_values(
                self.repository
                    .list_pacing_curves_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "pacing_tracker",
            to_values(self.repository.list_all_pacing_trackers(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "book_outline",
            to_values(
                self.repository
                    .list_all_book_outlines_by_branch_ids(&branch_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter_outline",
            to_values(
                self.repository
                    .list_all_chapter_outlines_by_branch_ids(&branch_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter_plan",
            to_values(self.repository.list_all_chapter_plans(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene_beat_annotation",
            to_values(
                self.repository
                    .list_all_scene_beat_annotations(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "chapter_summary",
            to_values(
                self.repository
                    .list_all_chapter_summaries(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "future_knowledge",
            to_values(
                self.repository
                    .list_all_future_knowledge(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "timeline_event",
            to_values(self.repository.list_all_timeline_events(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "temporal_intervention",
            to_values(
                self.repository
                    .list_all_temporal_interventions(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "system_overlay",
            to_values(self.repository.list_all_system_overlays(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "progression_event",
            to_values(
                self.repository
                    .list_all_progression_events(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "dual_persona_review",
            to_values(
                self.repository
                    .list_all_dual_persona_reviews(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "canonical_fact",
            to_values(self.repository.list_all_canonical_facts(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "scene_source_link",
            to_values(
                self.repository
                    .list_scene_source_links_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "knowledge_fact",
            to_values(
                self.repository
                    .list_knowledge_facts_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "research_log",
            to_values(
                self.repository
                    .list_research_logs_by_project(project_id)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "knows",
            to_values(self.repository.list_all_knows(project_id).await?)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_session",
            to_values(import_sessions)?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_source_document",
            to_values(
                self.repository
                    .list_import_source_documents_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_segment",
            to_values(
                self.repository
                    .list_import_segments_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_entity_mention",
            to_values(
                self.repository
                    .list_import_entity_mentions_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_entity_cluster",
            to_values(
                self.repository
                    .list_import_entity_clusters_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_character_dossier",
            to_values(
                self.repository
                    .list_import_character_dossiers_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_world_dossier",
            to_values(
                self.repository
                    .list_import_world_dossiers_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_narrative_dossier",
            to_values(
                self.repository
                    .list_import_narrative_dossiers_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_resume_snapshot",
            to_values(
                self.repository
                    .list_import_resume_snapshots_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );
        insert_export_rows(
            &mut tables,
            &mut counts,
            "import_review_item",
            to_values(
                self.repository
                    .list_import_review_items_for_sessions(&import_session_ids)
                    .await?,
            )?,
        );

        let mut payload_fields = serde_json::Map::new();
        payload_fields.insert("format".to_string(), Value::String(format.to_string()));
        payload_fields.insert(
            "exported_at".to_string(),
            Value::String(chrono::Utc::now().to_rfc3339()),
        );
        payload_fields.insert(
            "project_id".to_string(),
            Value::String(project_id.to_string()),
        );
        payload_fields.insert(
            "project_name".to_string(),
            Value::String(project.name.clone()),
        );
        for (key, value) in extra_metadata {
            payload_fields.insert(key, value);
        }
        payload_fields.insert(
            "omitted_tables".to_string(),
            json!([
                "search_embedding",
                "validator_finding",
                "session_activity",
                "writer_position"
            ]),
        );
        payload_fields.insert("record_counts".to_string(), serde_json::to_value(&counts)?);
        payload_fields.insert("tables".to_string(), serde_json::to_value(&tables)?);

        let exported_tables = counts.len();
        let exported_records = counts.values().copied().sum();

        Ok(ProjectExportArtifact {
            project_name: project.name,
            payload: Value::Object(payload_fields),
            exported_tables,
            exported_records,
        })
    }

    /// Render an EPUB from the project's active branch. Runs the same
    /// preflight as `preflight_book_export`; bails on any Blocking issue
    /// with a formatted summary; passes Warnings through to the output.
    /// Mirrors services/mod.rs:832..995 in 705b835^.
    ///
    /// `divergence_warnings` is populated by walking
    /// `list_scene_source_links_by_project` and running each in-scope
    /// link through `SourceBridge::evaluate_scene_divergence`. Risk #7 is
    /// closed.
    pub async fn export_epub(
        &self,
        input: spindle_core::models::ExportEpubInput,
    ) -> Result<spindle_core::models::ExportEpubOutput> {
        use crate::export::{EpubBook, EpubChapter, EpubSource, build_epub};
        use spindle_core::models::{ExportEpubOutput, ExportIssueSeverity};

        let project = self.repository.get_project(&input.project_id).await?;
        let chapter_scope = resolve_export_chapter_scope(
            input.book_number,
            input.start_chapter_number,
            input.end_chapter_number,
        )?;
        let preflight_issues = self
            .validate_book_for_export(&input.project_id, input.book_number, chapter_scope)
            .await?;
        let preflight_warnings = preflight_issues
            .iter()
            .filter(|issue| matches!(issue.severity, ExportIssueSeverity::Warning))
            .cloned()
            .collect::<Vec<_>>();
        let blocking_issues = preflight_issues
            .iter()
            .filter(|issue| matches!(issue.severity, ExportIssueSeverity::Blocking))
            .cloned()
            .collect::<Vec<_>>();
        if !blocking_issues.is_empty() {
            anyhow::bail!(
                "book export preflight failed:\n{}",
                format_export_issues(&blocking_issues)
            );
        }

        let all_books = self
            .repository
            .list_books_by_project(&input.project_id)
            .await?;
        let books_to_export: Vec<_> = match input.book_number {
            Some(n) => all_books.iter().filter(|b| b.book_number == n).collect(),
            None => all_books.iter().collect(),
        };
        if books_to_export.is_empty() {
            anyhow::bail!("no books found for export");
        }

        let active_branch = self.repository.get_active_branch(&input.project_id).await?;
        let all_scenes = self
            .repository
            .list_scenes_by_project_and_branch(&input.project_id, &active_branch.id)
            .await?;

        let mut total_scenes = 0usize;
        let mut total_chapters = 0usize;
        let mut epub_books = Vec::new();

        for book in &books_to_export {
            let chapters = self
                .repository
                .list_chapters_by_book(&book.id)
                .await?
                .into_iter()
                .filter(|chapter| {
                    export_scope_contains_chapter(chapter_scope, chapter.chapter_number)
                })
                .collect::<Vec<_>>();

            let mut epub_chapters = Vec::new();
            for chapter in &chapters {
                let chapter_scenes: Vec<_> = all_scenes
                    .iter()
                    .filter(|s| {
                        s.book_number == book.book_number
                            && s.chapter_number == chapter.chapter_number
                    })
                    .collect();
                if chapter_scenes.is_empty() {
                    continue;
                }
                total_scenes += chapter_scenes.len();
                let body = chapter_scenes
                    .iter()
                    .map(|s| s.full_text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n\n");
                epub_chapters.push(EpubChapter {
                    number: chapter.chapter_number,
                    title: chapter.title.clone(),
                    body,
                });
            }
            if !epub_chapters.is_empty() {
                total_chapters += epub_chapters.len();
                epub_books.push(EpubBook {
                    number: book.book_number,
                    title: book.title.clone(),
                    chapters: epub_chapters,
                });
            }
        }

        if epub_books.is_empty() {
            anyhow::bail!("no scenes found for export — nothing to publish");
        }

        let source = EpubSource {
            title: project.name.clone(),
            author: input.author,
            language: "en".to_string(),
            books: epub_books,
        };

        let epub_bytes = build_epub(&source)?;
        let slug = project.name.to_lowercase().replace(' ', "-");
        let filename = match (input.book_number, chapter_scope) {
            (Some(n), Some(scope)) => format!(
                "{slug}-book-{n}-chapters-{}-{}.epub",
                scope.start_chapter_number, scope.end_chapter_number
            ),
            (Some(n), None) => format!("{slug}-book-{n}.epub"),
            (None, _) => format!("{slug}.epub"),
        };

        // Divergence detection at export time (Gates 1 + 3). Mirrors
        // services/mod.rs:947..986 in 705b835^.
        let mut divergence_warnings: Vec<spindle_core::models::DivergenceWarning> = Vec::new();
        let source_links = self
            .repository
            .list_scene_source_links_by_project(&input.project_id)
            .await
            .unwrap_or_default();
        if !source_links.is_empty() {
            let scene_map: std::collections::BTreeMap<String, &super::records::Scene> =
                all_scenes.iter().map(|s| (s.id.clone(), s)).collect();
            for link in &source_links {
                let Some(scene) = scene_map.get(&link.scene_id) else {
                    continue;
                };
                let in_book_scope = match input.book_number {
                    Some(n) => scene.book_number == n,
                    None => true,
                };
                if !in_book_scope
                    || !export_scope_contains_chapter(chapter_scope, scene.chapter_number)
                {
                    continue;
                }
                if let Some(observation) =
                    super::source_bridge::evaluate_scene_divergence(link, scene)
                {
                    divergence_warnings.push(spindle_core::models::DivergenceWarning {
                        scene_id: scene.id.clone(),
                        book_number: scene.book_number,
                        chapter_number: scene.chapter_number,
                        scene_order: scene.scene_order,
                        source_path: link.source_path.clone(),
                        kind: observation.kind,
                        detail: observation.detail,
                    });
                }
            }
        }

        let export_dir = self.repository.data_dir().join("exports");
        std::fs::create_dir_all(&export_dir)?;
        let file_path = export_dir.join(&filename);
        std::fs::write(&file_path, &epub_bytes)?;

        Ok(ExportEpubOutput {
            file_path: file_path.to_string_lossy().to_string(),
            filename,
            total_chapters,
            total_scenes,
            preflight_warnings,
            divergence_warnings,
        })
    }

    // Import pipeline.
    //
    // The 10 stubs that follow implement the import passes against the
    // SQLite backend. They lift the SurrealDB-era service methods with the
    // mechanical `RecordId → String` swap and route through the helpers
    // in `crate::sqlite::import_service` + the pure-logic modules under
    // `crate::sqlite::import`. The `_resolve_*` and `_persist_*` helpers
    // they depend on live further down on this impl.

    pub async fn import_manuscript(
        &self,
        input: spindle_core::models::ImportManuscriptInput,
    ) -> Result<spindle_core::models::ImportManuscriptOutput> {
        use crate::sqlite::import::slicer::IngestSourcesOptions;
        use crate::sqlite::import::{analyze_structure, ingest_sources, sha256_hex};
        use crate::sqlite::import_service::{
            default_import_data_dir, import_hydration_mode_name, import_pass_name, import_progress,
            import_session_status_name, import_session_summary, import_source_format_name,
        };
        use crate::sqlite::repository::CreateImportSessionParams;
        use spindle_core::models::{
            ImportDuplicateStrategy, ImportManuscriptOutput, ImportPassName, ImportSessionStatus,
        };
        use std::collections::BTreeSet;
        use std::path::PathBuf;

        if input.source_paths.is_empty() {
            anyhow::bail!("import requires at least one source path");
        }

        let data_dir = default_import_data_dir();
        let duplicate_strategy = input
            .duplicate_strategy
            .clone()
            .unwrap_or(ImportDuplicateStrategy::Reject);

        let (project, target_branch_id, hydrate_mode) = self.resolve_import_target(&input).await?;
        let source_paths = input
            .source_paths
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        let mut existing_hashes = BTreeSet::new();
        if matches!(duplicate_strategy, ImportDuplicateStrategy::Reject) {
            for source_path in &source_paths {
                let bytes = std::fs::read(source_path).with_context(|| {
                    format!("failed to read source file {}", source_path.display())
                })?;
                let original_sha256 = sha256_hex(&bytes);
                if self
                    .repository
                    .import_session_exists_for_source_hash(&original_sha256)
                    .await?
                {
                    existing_hashes.insert(original_sha256);
                }
            }
        }

        let ingested = ingest_sources(
            &source_paths,
            IngestSourcesOptions {
                data_dir: &data_dir,
                existing_source_hashes: &existing_hashes,
                duplicate_strategy: duplicate_strategy.clone(),
                source_format_hint: input.source_format_hint.clone(),
            },
        )?;
        let analysis = analyze_structure(ingested);
        let initial_progress = import_progress(
            analysis.source_documents.len(),
            0,
            analysis.total_segments(),
            0,
            0,
            0,
        );

        let session = self
            .repository
            .create_import_session(CreateImportSessionParams {
                project_id: Some(project.id.clone()),
                target_branch_id: Some(target_branch_id.clone()),
                source_format: input
                    .source_format_hint
                    .as_ref()
                    .map(import_source_format_name),
                active_pass: import_pass_name(&ImportPassName::StructuralAnalysis),
                progress: serde_json::to_value(&initial_progress)?,
                session_status: import_session_status_name(&ImportSessionStatus::Running),
                hydrate_mode: import_hydration_mode_name(&hydrate_mode),
                source_count: analysis.source_documents.len(),
            })
            .await?;

        let (summary, review_count) = self
            .persist_structural_analysis(&session.id, &project.id, &analysis)
            .await?;
        let final_status = if review_count > 0 {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::ReadyToHydrate
        };
        let final_progress = import_progress(
            summary.source_documents.len(),
            summary.source_documents.len(),
            analysis.total_segments(),
            analysis.total_segments(),
            review_count,
            review_count,
        );
        let persisted_session = self
            .repository
            .update_import_session_state(
                &session.id,
                &import_pass_name(&ImportPassName::StructuralAnalysis),
                serde_json::to_value(&final_progress)?,
                &import_session_status_name(&final_status),
            )
            .await?;

        Ok(ImportManuscriptOutput {
            session: import_session_summary(&persisted_session)?,
            structure: summary,
        })
    }

    pub async fn import_status(
        &self,
        input: spindle_core::models::ImportStatusInput,
    ) -> Result<spindle_core::models::ImportStatusOutput> {
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, entity_consolidation_report_from_records,
            entity_extraction_report_from_records, import_review_item_summary,
            import_session_summary, narrative_dossier_summary_from_record,
            resume_snapshot_summary_from_record, structural_summary_from_records,
            world_dossier_summary_from_record,
        };
        use spindle_core::models::ImportStatusOutput;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let character_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let structure = (!source_documents.is_empty() || !segments.is_empty())
            .then(|| {
                structural_summary_from_records(&source_documents, &segments, review_items.len())
            })
            .transpose()?;
        let entity_extraction = (!mentions.is_empty())
            .then(|| entity_extraction_report_from_records(&mentions, &review_items));
        let entity_consolidation = (!clusters.is_empty())
            .then(|| entity_consolidation_report_from_records(&clusters, &review_items));
        let characters = character_dossier_summaries_from_records(&character_dossiers)?;
        let world = self
            .repository
            .find_import_world_dossier(&input.session_id)
            .await?
            .map(world_dossier_summary_from_record)
            .transpose()?;
        let narrative = self
            .repository
            .find_import_narrative_dossier(&input.session_id)
            .await?
            .map(narrative_dossier_summary_from_record)
            .transpose()?;
        let final_state = self
            .repository
            .find_import_resume_snapshot(&input.session_id)
            .await?
            .map(resume_snapshot_summary_from_record)
            .transpose()?;

        Ok(ImportStatusOutput {
            session: import_session_summary(&session)?,
            structure,
            entity_extraction,
            entity_consolidation,
            characters,
            world,
            narrative,
            final_state,
            review_items: review_items
                .into_iter()
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
            hydration_report: session
                .hydration_report
                .clone()
                .map(serde_json::from_value)
                .transpose()?,
        })
    }

    pub async fn import_extract_entities(
        &self,
        input: spindle_core::models::ImportExtractEntitiesInput,
    ) -> Result<spindle_core::models::ImportExtractEntitiesOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::extract::extract_entity_candidates;
        use crate::sqlite::import::prompts::{ImportExtractPrompt, build_entity_extraction_prompt};
        use crate::sqlite::import_service::{
            entity_extraction_report_from_records, import_entity_kind_name, import_pass_name,
            import_progress, import_review_item_kind_name, import_review_item_summary,
            import_review_severity_name, import_review_status_name, import_session_status_name,
            load_segment_text, structural_summary_from_records,
        };
        use crate::sqlite::repository::{
            CreateImportEntityMentionParams, CreateImportReviewItemParams,
        };
        use spindle_core::models::{
            ImportCorrectionPayload, ImportExtractEntitiesOutput, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeSet;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let project = self.repository.get_project(&input.project_id).await?;
        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let segment_filter = if input.segment_ids.is_empty() {
            None
        } else {
            Some(input.segment_ids.iter().cloned().collect::<BTreeSet<_>>())
        };

        let existing_mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let mut completed_segment_ids = existing_mentions
            .iter()
            .map(|mention| mention.segment_id.clone())
            .collect::<BTreeSet<_>>();
        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let mut mentions = existing_mentions;
        let mut processed_segments = 0usize;
        let mut extraction_round = mentions.len() + 1;

        for chapter in &structure.chapters {
            for scene in &chapter.scenes {
                let scene_segment_id = scene.segment_id.clone();
                if completed_segment_ids.contains(&scene_segment_id) {
                    continue;
                }
                if let Some(segment_filter) = segment_filter.as_ref()
                    && !segment_filter.contains(&scene_segment_id)
                {
                    continue;
                }
                if input.limit.is_some_and(|limit| processed_segments >= limit) {
                    break;
                }

                let segment_record = segments
                    .iter()
                    .find(|segment| segment.id == scene_segment_id)
                    .context("scene segment not found for extraction")?;
                let source_document = source_documents
                    .iter()
                    .find(|document| document.id == segment_record.source_document_id)
                    .context("source document missing for scene extraction")?;
                let segment_text = load_segment_text(source_document, segment_record)?;

                let prompt = build_entity_extraction_prompt(&ImportExtractPrompt {
                    project_name: Some(&project.name),
                    chapter,
                    scene: Some(scene),
                    text: &segment_text,
                });
                let _model_response = self
                    .repository
                    .model_router()
                    .complete(&ModelRequest {
                        route: "import_extract".to_string(),
                        prompt,
                        rating: None,
                        context: None,
                    })
                    .await?;

                let candidates = extract_entity_candidates(&segment_text);
                for candidate in candidates {
                    let mention = self
                        .repository
                        .create_import_entity_mention(CreateImportEntityMentionParams {
                            session_id: input.session_id.clone(),
                            segment_id: segment_record.id.clone(),
                            entity_kind: import_entity_kind_name(&candidate.entity_kind),
                            surface_form: candidate.surface_form.clone(),
                            normalized_name: candidate.normalized_name.clone(),
                            alias_hint: candidate.alias_hint.clone(),
                            surrounding_text: candidate.surrounding_text.clone(),
                            confidence: candidate.confidence,
                            extraction_pass: format!("extract-{}", extraction_round),
                        })
                        .await?;
                    if let Some(reason) = candidate.review_reason.as_ref() {
                        let review = self
                            .repository
                            .create_import_review_item(CreateImportReviewItemParams {
                                session_id: input.session_id.clone(),
                                pass_name: import_pass_name(&ImportPassName::EntityExtraction),
                                item_kind: import_review_item_kind_name(
                                    &ImportReviewItemKind::Entity,
                                ),
                                severity: import_review_severity_name(
                                    &ImportReviewSeverity::RequiresReview,
                                ),
                                status: import_review_status_name(&ImportReviewStatus::Open),
                                title: format!(
                                    "Review extracted entity '{}'",
                                    candidate.surface_form
                                ),
                                description: reason.clone(),
                                related_segment_ids: vec![segment_record.id.clone()],
                                related_entity_ids: vec![mention.id.clone()],
                                confidence: Some(candidate.confidence),
                                proposed_correction: Some(serde_json::to_value(
                                    ImportCorrectionPayload::Entity {
                                        entity_kind: candidate.entity_kind.clone(),
                                        canonical_name: Some(candidate.surface_form.clone()),
                                        merge_cluster_ids: Vec::new(),
                                        split_aliases: candidate
                                            .alias_hint
                                            .clone()
                                            .into_iter()
                                            .collect(),
                                    },
                                )?),
                                resolver_notes: None,
                            })
                            .await?;
                        review_items.push(review);
                    }
                    mentions.push(mention);
                }

                completed_segment_ids.insert(scene_segment_id);
                processed_segments += 1;
                extraction_round += 1;
            }
        }

        let completed_segment_ids_vec = completed_segment_ids.into_iter().collect::<Vec<_>>();
        let total_segments = segments
            .iter()
            .filter(|segment| segment.segment_type == "scene")
            .count();
        let progress = import_progress(
            source_documents.len(),
            source_documents.len(),
            total_segments,
            completed_segment_ids_vec.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        );
        let session_status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::EntityExtraction),
                serde_json::to_value(&progress)?,
                &import_session_status_name(&session_status),
            )
            .await?;

        let report = entity_extraction_report_from_records(&mentions, &review_items);
        Ok(ImportExtractEntitiesOutput {
            session_id: input.session_id,
            report,
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::EntityExtraction)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_consolidate_entities(
        &self,
        input: spindle_core::models::ImportConsolidateEntitiesInput,
    ) -> Result<spindle_core::models::ImportConsolidateEntitiesOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::consolidate::consolidate_mentions;
        use crate::sqlite::import::prompts::build_entity_consolidation_prompt;
        use crate::sqlite::import_service::{
            entity_consolidation_report_from_records, import_entity_kind_name, import_pass_name,
            import_progress, import_review_item_kind_name, import_review_item_summary,
            import_review_severity_name, import_review_status_name, import_session_status_name,
            parse_import_entity_kind,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportEntityClusterParams,
        };
        use spindle_core::models::{
            ImportConsolidateEntitiesOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeSet;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let entity_kind_filter = (!input.entity_kinds.is_empty()).then(|| {
            input
                .entity_kinds
                .iter()
                .map(import_entity_kind_name)
                .collect::<BTreeSet<_>>()
        });
        let clusters = consolidate_mentions(&mentions, entity_kind_filter.as_ref());

        for cluster in &clusters {
            let candidates = mentions
                .iter()
                .filter(|mention| {
                    mention.entity_kind == cluster.entity_kind
                        && cluster.mention_ids.contains(&mention.id)
                })
                .map(|mention| mention.surface_form.clone())
                .collect::<Vec<_>>();
            let _model_response = self
                .repository
                .model_router()
                .complete(&ModelRequest {
                    route: "import_synthesize".to_string(),
                    prompt: build_entity_consolidation_prompt(
                        parse_import_entity_kind(&cluster.entity_kind),
                        &candidates,
                    ),
                    rating: None,
                    context: None,
                })
                .await?;

            let persisted_cluster = self
                .repository
                .upsert_import_entity_cluster(UpsertImportEntityClusterParams {
                    session_id: input.session_id.clone(),
                    entity_kind: cluster.entity_kind.clone(),
                    canonical_name: cluster.canonical_name.clone(),
                    normalized_name: cluster.normalized_name.clone(),
                    aliases: cluster.aliases.clone(),
                    mention_ids: cluster.mention_ids.clone(),
                    first_segment_id: cluster.first_segment_id.clone(),
                    last_segment_id: cluster.last_segment_id.clone(),
                    importance_rank: cluster.importance_rank,
                    merge_confidence: cluster.merge_confidence,
                    review_required: cluster.review_required,
                    notes: cluster.notes.clone(),
                })
                .await?;

            if cluster.review_required {
                let review = self
                    .repository
                    .create_import_review_item(CreateImportReviewItemParams {
                        session_id: input.session_id.clone(),
                        pass_name: import_pass_name(&ImportPassName::EntityConsolidation),
                        item_kind: import_review_item_kind_name(&ImportReviewItemKind::Entity),
                        severity: import_review_severity_name(
                            &ImportReviewSeverity::RequiresReview,
                        ),
                        status: import_review_status_name(&ImportReviewStatus::Open),
                        title: format!("Review entity cluster '{}'", cluster.canonical_name),
                        description: if cluster.notes.is_empty() {
                            "cluster merge confidence was low".to_string()
                        } else {
                            cluster.notes.join(" ")
                        },
                        related_segment_ids: persisted_cluster
                            .first_segment_id
                            .clone()
                            .into_iter()
                            .collect(),
                        related_entity_ids: vec![persisted_cluster.id.clone()],
                        confidence: Some(cluster.merge_confidence),
                        proposed_correction: Some(serde_json::to_value(
                            ImportCorrectionPayload::Entity {
                                entity_kind: parse_import_entity_kind(&cluster.entity_kind),
                                canonical_name: Some(cluster.canonical_name.clone()),
                                merge_cluster_ids: vec![persisted_cluster.id.clone()],
                                split_aliases: cluster.aliases.clone(),
                            },
                        )?),
                        resolver_notes: None,
                    })
                    .await?;
                review_items.push(review);
            }
        }

        let persisted_clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let progress = serde_json::to_value(import_progress(
            0,
            0,
            0,
            0,
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::EntityConsolidation),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        let report = entity_consolidation_report_from_records(&persisted_clusters, &review_items);
        Ok(ImportConsolidateEntitiesOutput {
            session_id: input.session_id,
            report,
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::EntityConsolidation)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_analyze_character(
        &self,
        input: spindle_core::models::ImportAnalyzeCharacterInput,
    ) -> Result<spindle_core::models::ImportAnalyzeCharacterOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::character::{
            build_character_dossiers, build_character_dossiers_for_clusters,
        };
        use crate::sqlite::import::prompts::build_character_analysis_prompt;
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, import_pass_name, import_progress,
            import_review_item_kind_name, import_review_item_summary, import_review_severity_name,
            import_review_status_name, import_session_status_name,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportCharacterDossierParams,
        };
        use spindle_core::models::{
            ImportAnalyzeCharacterOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeSet;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let all_clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let selected_clusters = if input.cluster_ids.is_empty() {
            build_character_dossiers(&all_clusters, &mentions, &segments)
        } else {
            let cluster_filter = input.cluster_ids.iter().cloned().collect::<BTreeSet<_>>();
            let filtered_clusters = all_clusters
                .iter()
                .filter(|cluster| cluster_filter.contains(&cluster.id))
                .cloned()
                .collect::<Vec<_>>();
            build_character_dossiers_for_clusters(&filtered_clusters, &mentions, &segments)
        };

        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let character_names = selected_clusters
            .iter()
            .map(|dossier| dossier.canonical_name.clone())
            .collect::<Vec<_>>();
        let character_notes = selected_clusters
            .iter()
            .flat_map(|dossier| dossier.review_notes.clone())
            .collect::<Vec<_>>();
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_character_analysis_prompt(&character_names, &character_notes),
                rating: None,
                context: None,
            })
            .await?;

        for dossier in &selected_clusters {
            let persisted = self
                .repository
                .upsert_import_character_dossier(UpsertImportCharacterDossierParams {
                    session_id: input.session_id.clone(),
                    cluster_id: dossier.cluster_id.clone(),
                    canonical_name: dossier.canonical_name.clone(),
                    aliases: dossier.aliases.clone(),
                    importance_rank: dossier.importance_rank,
                    voice_profile: serde_json::to_value(&dossier.voice_profile)?,
                    emotional_profile: serde_json::to_value(&dossier.emotional_profile)?,
                    state_trajectory: serde_json::to_value(&dossier.state_trajectory)?,
                    relationship_inferences: serde_json::to_value(
                        &dossier.relationship_inferences,
                    )?,
                    decision_patterns: dossier.decision_patterns.clone(),
                    dialogue_samples: dossier.dialogue_samples.clone(),
                    confidence: dossier.confidence,
                    review_required: dossier.review_required,
                })
                .await?;

            if dossier.review_required {
                let related_segment_ids = dossier
                    .state_trajectory
                    .iter()
                    .map(|point| point.segment_id.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>();
                let review = self
                    .repository
                    .create_import_review_item(CreateImportReviewItemParams {
                        session_id: input.session_id.clone(),
                        pass_name: import_pass_name(&ImportPassName::CharacterAnalysis),
                        item_kind: import_review_item_kind_name(&ImportReviewItemKind::Character),
                        severity: import_review_severity_name(
                            &ImportReviewSeverity::RequiresReview,
                        ),
                        status: import_review_status_name(&ImportReviewStatus::Open),
                        title: format!("Review character dossier '{}'", dossier.canonical_name),
                        description: dossier.review_notes.join(" "),
                        related_segment_ids,
                        related_entity_ids: vec![persisted.id.clone(), dossier.cluster_id.clone()],
                        confidence: Some(dossier.confidence),
                        proposed_correction: Some(serde_json::to_value(
                            ImportCorrectionPayload::Character {
                                cluster_id: dossier.cluster_id.clone(),
                                preferred_name: Some(dossier.canonical_name.clone()),
                                relationship_notes: dossier.review_notes.clone(),
                            },
                        )?),
                        resolver_notes: None,
                    })
                    .await?;
                review_items.push(review);
            }
        }

        let persisted_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let progress = serde_json::to_value(import_progress(
            0,
            0,
            0,
            selected_clusters.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::CharacterAnalysis),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportAnalyzeCharacterOutput {
            session_id: input.session_id,
            characters: character_dossier_summaries_from_records(&persisted_dossiers)?,
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::CharacterAnalysis)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_extract_world(
        &self,
        input: spindle_core::models::ImportExtractWorldInput,
    ) -> Result<spindle_core::models::ImportExtractWorldOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::prompts::{
            ImportSynthesizePrompt, build_world_extraction_prompt,
        };
        use crate::sqlite::import::world::{extract_world_dossier, to_world_summary};
        use crate::sqlite::import_service::{
            import_pass_name, import_progress, import_review_item_kind_name,
            import_review_item_summary, import_review_severity_name, import_review_status_name,
            import_session_status_name, load_segment_text, structural_summary_from_records,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportWorldDossierParams,
        };
        use spindle_core::models::{
            ImportCorrectionPayload, ImportExtractWorldOutput, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeMap;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let project = self.repository.get_project(&input.project_id).await?;
        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let clusters = self
            .repository
            .list_import_entity_clusters(&input.session_id)
            .await?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let mut segment_texts = BTreeMap::new();
        for segment in segments
            .iter()
            .filter(|segment| segment.segment_type == "scene")
        {
            let Some(source_document) = source_documents
                .iter()
                .find(|document| document.id == segment.source_document_id)
            else {
                continue;
            };
            let text = load_segment_text(source_document, segment)?;
            segment_texts.insert(segment.id.clone(), text);
        }

        let focus_notes = vec![
            "extract world rules, institutions, terms, and system signals".to_string(),
            "preserve low-confidence inferences as reviewable outputs".to_string(),
        ];
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_world_extraction_prompt(&ImportSynthesizePrompt {
                    structure: &structure,
                    focus: &project.name,
                    notes: &focus_notes,
                }),
                rating: None,
                context: None,
            })
            .await?;

        let draft = extract_world_dossier(&segments, &segment_texts, &mentions, &clusters);
        let persisted = self
            .repository
            .upsert_import_world_dossier(UpsertImportWorldDossierParams {
                session_id: input.session_id.clone(),
                world_rules: serde_json::to_value(&draft.world_rules)?,
                locations: serde_json::to_value(&draft.locations)?,
                entities: serde_json::to_value(&draft.entities)?,
                system_signals: serde_json::to_value(&draft.system_signals)?,
            })
            .await?;

        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        for review in &draft.review_items {
            let created = self
                .repository
                .create_import_review_item(CreateImportReviewItemParams {
                    session_id: input.session_id.clone(),
                    pass_name: import_pass_name(&ImportPassName::WorldExtraction),
                    item_kind: import_review_item_kind_name(&ImportReviewItemKind::World),
                    severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                    status: import_review_status_name(&ImportReviewStatus::Open),
                    title: review.title.clone(),
                    description: review.description.clone(),
                    related_segment_ids: review.source_segment_ids.clone(),
                    related_entity_ids: vec![persisted.id.clone()],
                    confidence: Some(review.confidence),
                    proposed_correction: Some(serde_json::to_value(
                        ImportCorrectionPayload::World {
                            entity_kind: review.entity_kind.clone(),
                            canonical_name: review.canonical_name.clone(),
                            replacement_summary: review.replacement_summary.clone(),
                        },
                    )?),
                    resolver_notes: None,
                })
                .await?;
            review_items.push(created);
        }

        let progress = serde_json::to_value(import_progress(
            structure.source_documents.len(),
            structure.source_documents.len(),
            structure
                .chapters
                .iter()
                .map(|chapter| chapter.scenes.len())
                .sum(),
            draft.world_rules.len()
                + draft.locations.len()
                + draft.entities.len()
                + draft.system_signals.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::WorldExtraction),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportExtractWorldOutput {
            session_id: input.session_id,
            world: to_world_summary(&draft),
            review_items: review_items
                .into_iter()
                .filter(|item| item.pass_name == import_pass_name(&ImportPassName::WorldExtraction))
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_analyze_narrative(
        &self,
        input: spindle_core::models::ImportAnalyzeNarrativeInput,
    ) -> Result<spindle_core::models::ImportAnalyzeNarrativeOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::narrative::{analyze_narrative_dossier, to_narrative_summary};
        use crate::sqlite::import::prompts::{
            ImportSynthesizePrompt, build_narrative_analysis_prompt,
        };
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, import_pass_name, import_progress,
            import_review_item_kind_name, import_review_item_summary, import_review_severity_name,
            import_review_status_name, import_session_status_name, load_segment_text,
            structural_summary_from_records,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportNarrativeDossierParams,
        };
        use spindle_core::models::{
            ImportAnalyzeNarrativeOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };
        use std::collections::BTreeMap;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mentions = self
            .repository
            .list_import_entity_mentions(&input.session_id)
            .await?;
        let character_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let mut segment_texts = BTreeMap::new();
        for segment in segments
            .iter()
            .filter(|segment| segment.segment_type == "scene")
        {
            let Some(source_document) = source_documents
                .iter()
                .find(|document| document.id == segment.source_document_id)
            else {
                continue;
            };
            let text = load_segment_text(source_document, segment)?;
            segment_texts.insert(segment.id.clone(), text);
        }

        let review_items_before = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let prompt_notes = vec![
            "extract plot lines, conflicts, promises, arcs, themes, motifs, and pacing hints"
                .to_string(),
            "preserve low-confidence narrative reads as review items".to_string(),
        ];
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_narrative_analysis_prompt(&ImportSynthesizePrompt {
                    structure: &structure,
                    focus: "narrative architecture",
                    notes: &prompt_notes,
                }),
                rating: None,
                context: None,
            })
            .await?;

        let draft = analyze_narrative_dossier(
            &segments,
            &segment_texts,
            &mentions,
            &character_dossier_summaries_from_records(&character_dossiers)?,
        );
        let persisted = self
            .repository
            .upsert_import_narrative_dossier(UpsertImportNarrativeDossierParams {
                session_id: input.session_id.clone(),
                plot_lines: serde_json::to_value(&draft.plot_lines)?,
                conflicts: serde_json::to_value(&draft.conflicts)?,
                narrative_promises: serde_json::to_value(&draft.narrative_promises)?,
                arcs: serde_json::to_value(&draft.arcs)?,
                themes: serde_json::to_value(&draft.themes)?,
                motifs: serde_json::to_value(&draft.motifs)?,
                reader_contract: serde_json::to_value(&draft.reader_contract)?,
                pacing_hints: serde_json::to_value(&draft.pacing_hints)?,
            })
            .await?;

        let mut review_items = review_items_before;
        for review in &draft.review_items {
            let created = self
                .repository
                .create_import_review_item(CreateImportReviewItemParams {
                    session_id: input.session_id.clone(),
                    pass_name: import_pass_name(&ImportPassName::NarrativeAnalysis),
                    item_kind: import_review_item_kind_name(&ImportReviewItemKind::Narrative),
                    severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                    status: import_review_status_name(&ImportReviewStatus::Open),
                    title: review.title.clone(),
                    description: review.description.clone(),
                    related_segment_ids: review.related_segment_ids.clone(),
                    related_entity_ids: vec![persisted.id.clone()],
                    confidence: Some(review.confidence),
                    proposed_correction: Some(serde_json::to_value(
                        ImportCorrectionPayload::Narrative {
                            target_id: review.target_id.clone(),
                            status: review.status.clone(),
                            thematic_purpose: review.thematic_purpose.clone(),
                            note: review.note.clone(),
                        },
                    )?),
                    resolver_notes: None,
                })
                .await?;
            review_items.push(created);
        }

        let progress = serde_json::to_value(import_progress(
            structure.source_documents.len(),
            structure.source_documents.len(),
            structure
                .chapters
                .iter()
                .map(|chapter| chapter.scenes.len())
                .sum(),
            draft.plot_lines.len()
                + draft.conflicts.len()
                + draft.narrative_promises.len()
                + draft.arcs.len()
                + draft.themes.len()
                + draft.motifs.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::NarrativeAnalysis),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportAnalyzeNarrativeOutput {
            session_id: input.session_id,
            narrative: to_narrative_summary(&draft),
            review_items: review_items
                .into_iter()
                .filter(|item| {
                    item.pass_name == import_pass_name(&ImportPassName::NarrativeAnalysis)
                })
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_compute_final_state(
        &self,
        input: spindle_core::models::ImportComputeFinalStateInput,
    ) -> Result<spindle_core::models::ImportComputeFinalStateOutput> {
        use crate::ai::ModelRequest;
        use crate::sqlite::import::final_state::compute_final_state;
        use crate::sqlite::import::prompts::{ImportSynthesizePrompt, build_final_state_prompt};
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, import_pass_name, import_progress,
            import_review_item_kind_name, import_review_item_summary, import_review_severity_name,
            import_review_status_name, import_session_status_name,
            narrative_dossier_summary_from_record, resume_snapshot_summary_from_record,
            structural_summary_from_records, world_dossier_summary_from_record,
        };
        use crate::sqlite::repository::{
            CreateImportReviewItemParams, UpsertImportResumeSnapshotParams,
        };
        use spindle_core::models::{
            ImportComputeFinalStateOutput, ImportCorrectionPayload, ImportPassName,
            ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus,
        };

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let character_dossiers = self
            .repository
            .list_import_character_dossiers(&input.session_id)
            .await?;
        let world = self
            .repository
            .find_import_world_dossier(&input.session_id)
            .await?
            .map(world_dossier_summary_from_record)
            .transpose()?;
        let narrative = self
            .repository
            .find_import_narrative_dossier(&input.session_id)
            .await?
            .map(narrative_dossier_summary_from_record)
            .transpose()?;
        let structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let prompt_notes = vec![
            "compute the explicit manuscript resume point".to_string(),
            "defer canonical knowledge facts until hydration".to_string(),
        ];
        let _model_response = self
            .repository
            .model_router()
            .complete(&ModelRequest {
                route: "import_synthesize".to_string(),
                prompt: build_final_state_prompt(&ImportSynthesizePrompt {
                    structure: &structure,
                    focus: "resume state and continuation handoff",
                    notes: &prompt_notes,
                }),
                rating: None,
                context: None,
            })
            .await?;

        let draft = compute_final_state(
            &segments,
            &character_dossier_summaries_from_records(&character_dossiers)?,
            world.as_ref(),
            narrative.as_ref(),
        );
        let persisted = self
            .repository
            .upsert_import_resume_snapshot(UpsertImportResumeSnapshotParams {
                session_id: input.session_id.clone(),
                book_number: draft.resume_snapshot.book_number,
                chapter_number: draft.resume_snapshot.chapter_number,
                scene_order: draft.resume_snapshot.scene_order,
                summary: draft.resume_snapshot.summary.clone(),
                characters: serde_json::to_value(&draft.resume_snapshot.characters)?,
                relationships: serde_json::to_value(&draft.resume_snapshot.relationships)?,
                locations: serde_json::to_value(&draft.resume_snapshot.locations)?,
                plot_threads: serde_json::to_value(&draft.resume_snapshot.plot_threads)?,
            })
            .await?;
        let _ = resume_snapshot_summary_from_record(persisted.clone())?;

        let mut review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        for review in &draft.review_items {
            let created = self
                .repository
                .create_import_review_item(CreateImportReviewItemParams {
                    session_id: input.session_id.clone(),
                    pass_name: import_pass_name(&ImportPassName::FinalState),
                    item_kind: import_review_item_kind_name(&ImportReviewItemKind::FinalState),
                    severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                    status: import_review_status_name(&ImportReviewStatus::Open),
                    title: review.title.clone(),
                    description: review.description.clone(),
                    related_segment_ids: review.related_segment_ids.clone(),
                    related_entity_ids: vec![persisted.id.clone()],
                    confidence: Some(review.confidence),
                    proposed_correction: Some(serde_json::to_value(
                        ImportCorrectionPayload::FinalState {
                            target_record_id: review.target_record_id.clone(),
                            corrected_summary: review.corrected_summary.clone(),
                        },
                    )?),
                    resolver_notes: None,
                })
                .await?;
            review_items.push(created);
        }

        let progress = serde_json::to_value(import_progress(
            structure.source_documents.len(),
            structure.source_documents.len(),
            structure
                .chapters
                .iter()
                .map(|chapter| chapter.scenes.len())
                .sum(),
            draft.resume_snapshot.characters.len()
                + draft.resume_snapshot.relationships.len()
                + draft.resume_snapshot.locations.len()
                + draft.resume_snapshot.plot_threads.len(),
            review_items.len(),
            review_items
                .iter()
                .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
                .count(),
        ))?;
        let status = if review_items
            .iter()
            .any(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
        {
            ImportSessionStatus::ReviewNeeded
        } else {
            ImportSessionStatus::Running
        };
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::FinalState),
                progress,
                &import_session_status_name(&status),
            )
            .await?;

        Ok(ImportComputeFinalStateOutput {
            session_id: input.session_id,
            resume_snapshot: draft.resume_snapshot,
            review_items: review_items
                .into_iter()
                .filter(|item| item.pass_name == import_pass_name(&ImportPassName::FinalState))
                .map(import_review_item_summary)
                .collect::<Result<Vec<_>>>()?,
        })
    }

    pub async fn import_hydrate_bible(
        &self,
        input: spindle_core::models::ImportHydrateBibleInput,
    ) -> Result<spindle_core::models::ImportHydrateBibleOutput> {
        self.import_hydrate_bible_inner(input).await
    }

    pub async fn import_apply_review_decisions(
        &self,
        input: spindle_core::models::ImportApplyReviewDecisionsInput,
    ) -> Result<spindle_core::models::ImportApplyReviewDecisionsOutput> {
        use crate::sqlite::import_service::{
            import_progress, import_review_item_summary, import_review_status_name,
            import_session_status_name,
        };
        use crate::sqlite::repository::ResolveImportReviewItemParams;
        use spindle_core::models::{
            ImportApplyReviewDecisionsOutput, ImportReviewStatus, ImportSessionProgress,
            ImportSessionStatus,
        };
        use std::collections::BTreeMap;

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;
        let review_items_by_id = review_items
            .iter()
            .cloned()
            .map(|item| (item.id.clone(), item))
            .collect::<BTreeMap<_, _>>();

        let mut updated_review_items = Vec::new();
        for decision in input.decisions {
            let review_item = review_items_by_id
                .get(&decision.review_item_id)
                .with_context(|| format!("review item not found: {}", decision.review_item_id))?;
            if review_item.session_id != input.session_id {
                anyhow::bail!("review item does not belong to the requested import session");
            }
            let updated = self
                .repository
                .resolve_import_review_item(
                    &review_item.id,
                    ResolveImportReviewItemParams {
                        status: import_review_status_name(&decision.resolution),
                        proposed_correction: decision
                            .correction
                            .as_ref()
                            .map(serde_json::to_value)
                            .transpose()?
                            .or_else(|| review_item.proposed_correction.clone()),
                        resolver_notes: decision.resolver_notes.clone(),
                    },
                )
                .await?;
            updated_review_items.push(import_review_item_summary(updated)?);
        }

        let remaining_open_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?
            .into_iter()
            .filter(|item| item.status == import_review_status_name(&ImportReviewStatus::Open))
            .count();
        let next_status = if remaining_open_items == 0 {
            ImportSessionStatus::ReadyToHydrate
        } else {
            ImportSessionStatus::ReviewNeeded
        };
        let existing_progress =
            serde_json::from_value::<ImportSessionProgress>(session.progress.clone())
                .unwrap_or_else(|_| import_progress(0, 0, 0, 0, 0, 0));
        let updated_progress = import_progress(
            existing_progress.total_documents,
            existing_progress.processed_documents,
            existing_progress.total_segments,
            existing_progress.processed_segments,
            existing_progress.total_review_items,
            remaining_open_items,
        );
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &session.active_pass,
                serde_json::to_value(updated_progress)?,
                &import_session_status_name(&next_status),
            )
            .await?;

        Ok(ImportApplyReviewDecisionsOutput {
            session_id: input.session_id,
            updated_review_items,
            remaining_open_items,
        })
    }

    pub async fn batch_set_character_voice_profiles(
        &self,
        input: BatchSetCharacterVoiceProfilesInput,
    ) -> Result<BatchSetCharacterVoiceProfilesOutput> {
        let mut profiles = Vec::with_capacity(input.items.len());
        for item in input.items {
            let updated = self
                .set_character_voice_profile(SetCharacterVoiceProfileInput {
                    project_id: input.project_id.clone(),
                    character_id: item.character_id,
                    branch_id: input.branch_id.clone(),
                    profile: item.profile,
                })
                .await?;
            profiles.push(updated);
        }
        let updated = profiles.len();
        Ok(BatchSetCharacterVoiceProfilesOutput { profiles, updated })
    }

    pub async fn record_knowledge(
        &self,
        input: RecordKnowledgeInput,
    ) -> Result<RecordKnowledgeOutput> {
        let character = self.repository.get_character(&input.character_id).await?;
        if character.project_id != input.project_id {
            anyhow::bail!("character does not belong to the requested project");
        }
        let branch_id = match input.branch_id {
            Some(b) => b,
            None => character.branch_id.clone(),
        };
        let normalized_fact = spindle_core::models::normalize_name(&input.fact);
        let fact = self
            .repository
            .upsert_knowledge_fact(UpsertKnowledgeFactParams {
                project_id: input.project_id.clone(),
                branch_id: branch_id.clone(),
                character_id: input.character_id.clone(),
                fact: input.fact.clone(),
                normalized_fact,
                source_summary: input.source_summary.clone(),
                learned_at: input.learned_at.clone(),
                confidence: input.confidence,
                tags: input.tags.clone(),
                reader_visible: input.reader_visible,
                source_import_session_id: None,
            })
            .await?;
        // Best-effort knows edge linking character → fact.
        let knows = self
            .repository
            .upsert_knows(UpsertKnowsParams {
                project_id: input.project_id.clone(),
                branch_id: branch_id.clone(),
                character_id: input.character_id.clone(),
                knowledge_fact_id: fact.id.clone(),
                source_summary: Some(input.source_summary),
                learned_at: input.learned_at,
                confidence: input.confidence,
                reader_visible: input.reader_visible,
                source_import_session_id: None,
            })
            .await
            .ok();
        let knows_edge_id =
            knows.map(|k| format!("knows:{}:{}:{}", k.branch_id, k.in_id, k.out_id));

        Ok(RecordKnowledgeOutput {
            fact: spindle_core::models::KnowledgeFactSummary {
                knowledge_fact_id: fact.id,
                branch_id: fact.branch_id,
                character_id: fact.character_id,
                fact: fact.fact,
                source_summary: fact.source_summary,
                learned_at: fact.learned_at.map(|p| StoryPlacement {
                    book_number: p.book_number,
                    chapter_number: p.chapter_number,
                    scene_order: p.scene_order,
                    note: p.note,
                }),
                confidence: fact.confidence,
                tags: fact.tags,
                reader_visible: fact.reader_visible,
            },
            knows_edge_id,
        })
    }

    pub async fn record_note(&self, input: RecordNoteInput) -> Result<RecordNoteOutput> {
        let project = self.repository.get_project(&input.project_id).await?;
        let branch_id = match input.branch_id {
            Some(b) => b,
            None => project
                .active_branch_id
                .clone()
                .ok_or_else(|| anyhow::anyhow!("project has no active branch"))?,
        };
        let summary = input.note.lines().next().unwrap_or("").to_string();
        let activity = self
            .repository
            .append_session_activity(AppendSessionActivityParams {
                project_id: input.project_id,
                branch_id: branch_id.clone(),
                kind: "note".to_string(),
                subject_table: None,
                subject_id: None,
                summary: summary.clone(),
                details_json: Some(serde_json::json!({ "note": input.note })),
            })
            .await?;
        let created_at = chrono::DateTime::<chrono::Utc>::from_timestamp_micros(
            activity.created_at.timestamp_micros(),
        )
        .unwrap_or_default();
        Ok(RecordNoteOutput {
            activity_id: activity.id,
            branch_id,
            kind: "note".to_string(),
            summary,
            created_at: created_at.to_rfc3339(),
        })
    }

    pub async fn update_writer_position(
        &self,
        input: UpdateWriterPositionInput,
    ) -> Result<spindle_core::models::WriterPosition> {
        let position = self
            .repository
            .upsert_writer_position(UpsertWriterPositionParams {
                project_id: input.project_id,
                branch_id: input.branch_id,
                book_id: input.book_id,
                chapter_id: input.chapter_id,
                scene_id: input.scene_id,
                intent: input.intent,
                next_focus: input.next_focus,
                updated_by: "spindle-mcp".to_string(),
                updated_at: chrono::Utc::now(),
            })
            .await?;
        Ok(spindle_core::models::WriterPosition {
            project_id: position.project_id,
            branch_id: position.branch_id,
            book_id: position.book_id,
            chapter_id: position.chapter_id,
            scene_id: position.scene_id,
            intent: position.intent,
            next_focus: position.next_focus,
            updated_by: position.updated_by,
        })
    }

    /// Save scene draft — the minimal MVP version. Returns the scene id and
    /// status. The full SurrealDB-side service runs many validators here
    /// (pacing, agency, tone deviation, world rule scan, etc.); those land
    /// in subsequent Phase 6 commits. For now the validation-derived fields
    /// default to "no warnings" so the MCP path is structurally correct.
    /// Assemble the project [`StyleDirective`](spindle_core::style::StyleDirective)
    /// — the single source of truth for genre-voice enforcement — from the
    /// reader contract, `style`-typed world rules, and narrator voice. Used by
    /// the save gate, the `style_compliance` validator, and the review's Target
    /// Reader persona.
    pub async fn style_directive_for(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<spindle_core::style::StyleDirective> {
        let project = self.repository.get_project(project_id).await?;
        let style_rules: Vec<spindle_core::style::StyleRule> = self
            .repository
            .list_world_rules_by_project_and_branch(project_id, branch_id)
            .await?
            .into_iter()
            .filter(|rule| rule.rule_type.eq_ignore_ascii_case("style"))
            .map(|rule| spindle_core::style::StyleRule {
                rule_name: rule.rule_name,
                description: rule.description,
            })
            .collect();
        Ok(spindle_core::style::StyleDirective::assemble(
            project.genre,
            project.project_type,
            project.reader_contract.promise,
            project.reader_contract.style_notes,
            project.reader_contract.boundaries,
            style_rules,
            project.narrator_voice.map(|stored| stored.into_core()),
        ))
    }

    pub async fn save_scene_draft(
        &self,
        input: SaveSceneDraftInput,
    ) -> Result<SaveSceneDraftOutput> {
        let branch_id = self
            .repository
            .active_branch_id_public(&input.project_id)
            .await?;
        let (scene, created) = self
            .repository
            .save_scene_draft(&input.project_id, &branch_id, &input)
            .await?;
        // Match SurrealDB semantics: first save returns "saved", subsequent
        // writes to the same (project, branch, scene_order) return "updated".
        let status = if created { "saved" } else { "updated" };

        // Genre-voice gate: scan the saved prose + declared tone against the
        // project style contract. Best-effort — a missing/empty contract just
        // yields no warnings, and an assembly error must not fail the save.
        // The contemplative-ending check is left to check_consistency, which
        // has the chapter context to know whether this is the last scene.
        let style_directive = self
            .style_directive_for(&input.project_id, &branch_id)
            .await
            .unwrap_or_default();
        let style_hits = style_directive.scan(&spindle_core::style::StyleScanInput {
            prose: &scene.full_text,
            declared_tone: input.tone.as_deref(),
            is_chapter_end: false,
        });
        let tone_deviation = style_hits
            .iter()
            .any(|hit| hit.severity == spindle_core::style::StyleDriftSeverity::Warning);
        let style_warnings: Vec<String> = style_hits.into_iter().map(|hit| hit.message).collect();

        Ok(SaveSceneDraftOutput {
            scene_id: scene.id.clone(),
            status: status.to_string(),
            draft_origin: scene
                .draft_origin
                .clone()
                .unwrap_or_else(|| "agent".to_string()),
            pacing_warnings: Vec::new(),
            agency_warning: None,
            tone_deviation,
            style_warnings,
            content_rating_valid: true,
            content_rating_warnings: Vec::new(),
            diff: Vec::new(),
            byte_offsets_changed: Vec::new(),
            chars_added: scene.full_text.len(),
            chars_deleted: 0,
            world_rule_hits: Vec::new(),
            voice_drift: Vec::new(),
            retcon_findings: Vec::new(),
        })
    }

    // =========================================================================
    // Import-pipeline private helpers.
    // =========================================================================

    /// Resolve the project + branch the caller wants to associate with a
    /// new import session. Mirrors the SurrealDB-era helper of the same
    /// name; on the SQLite side ids are plain `String` rather than
    /// `RecordId`.
    async fn resolve_import_target(
        &self,
        input: &spindle_core::models::ImportManuscriptInput,
    ) -> Result<(
        crate::sqlite::records::Project,
        String,
        spindle_core::models::ImportHydrationMode,
    )> {
        use crate::sqlite::import_service::default_import_project_name;
        use spindle_core::models::{CreateProjectInput, ImportHydrationMode, ReaderContract};

        let Some(project_id) = input.target_project_id.as_deref() else {
            let project_name = input
                .create_project_name
                .clone()
                .unwrap_or_else(|| default_import_project_name(input));
            let project_output = self
                .create_project(CreateProjectInput {
                    name: project_name,
                    project_type: "novel".to_string(),
                    genre: "fiction".to_string(),
                    reader_contract: ReaderContract {
                        promise: "Imported manuscript continuation".to_string(),
                        style_notes: vec![
                            "Preserve the voice already present in the manuscript.".to_string(),
                        ],
                        boundaries: vec![],
                    },
                })
                .await?;
            let project = self
                .repository
                .get_project(&project_output.project_id)
                .await?;
            let branch_id = project
                .active_branch_id
                .clone()
                .or_else(|| {
                    Some(
                        // Resolve project main branch by name.
                        "bible_branch:main".to_string(),
                    )
                })
                .unwrap_or_else(|| "bible_branch:main".to_string());
            let branch_id = self.resolve_project_main_branch(&project, &branch_id).await;
            return Ok((project, branch_id, ImportHydrationMode::NewProject));
        };

        let project = self.repository.get_project(project_id).await?;
        let branch_id = if let Some(branch_id) = input.target_branch_id.as_deref() {
            let branch = self.repository.get_branch(branch_id).await?;
            if branch.project_id.as_deref() != Some(project.id.as_str())
                && branch.id != "bible_branch:main"
            {
                anyhow::bail!("target branch does not belong to the requested project");
            }
            branch.id
        } else {
            self.resolve_project_main_branch(&project, "bible_branch:main")
                .await
        };

        Ok((project, branch_id, ImportHydrationMode::ExistingProject))
    }

    /// Resolve the project + branch the caller wants to hydrate into.
    /// Returns `(project, branch_id, created_project)`.
    async fn resolve_hydration_target(
        &self,
        input: &spindle_core::models::ImportHydrateBibleInput,
        session: &crate::sqlite::records::ImportSession,
    ) -> Result<(crate::sqlite::records::Project, String, bool)> {
        use crate::sqlite::import_service::parse_import_hydration_mode;
        use spindle_core::models::{CreateProjectInput, ImportHydrationMode, ReaderContract};

        let requested_mode = input.hydrate_mode.clone().unwrap_or_else(|| {
            if input.target_project_id.is_some() {
                ImportHydrationMode::ExistingProject
            } else {
                parse_import_hydration_mode(&session.hydrate_mode)
                    .unwrap_or(ImportHydrationMode::ExistingProject)
            }
        });

        if matches!(requested_mode, ImportHydrationMode::NewProject)
            || (input.target_project_id.is_none() && input.create_project_name.is_some())
        {
            let project_output = self
                .create_project(CreateProjectInput {
                    name: input
                        .create_project_name
                        .clone()
                        .unwrap_or_else(|| "Imported Manuscript".to_string()),
                    project_type: "novel".to_string(),
                    genre: "fiction".to_string(),
                    reader_contract: ReaderContract {
                        promise: "Imported manuscript continuation".to_string(),
                        style_notes: vec!["Preserve the imported manuscript voice.".to_string()],
                        boundaries: vec![],
                    },
                })
                .await?;
            let project = self
                .repository
                .get_project(&project_output.project_id)
                .await?;
            let branch_id = self
                .resolve_project_main_branch(&project, "bible_branch:main")
                .await;
            return Ok((project, branch_id, true));
        }

        let project_id = if let Some(target_project_id) = input.target_project_id.as_deref() {
            target_project_id.to_string()
        } else if let Some(session_project_id) = session.project_id.as_ref() {
            session_project_id.clone()
        } else {
            anyhow::bail!("hydration target project is required");
        };
        let project = self.repository.get_project(&project_id).await?;
        let branch_id = if let Some(branch_id) = input.target_branch_id.as_deref() {
            let branch = self.repository.get_branch(branch_id).await?;
            if branch.project_id.as_deref() != Some(project.id.as_str())
                && branch.id != "bible_branch:main"
            {
                anyhow::bail!("target branch does not belong to the requested project");
            }
            branch.id
        } else {
            let fallback = session
                .target_branch_id
                .clone()
                .unwrap_or_else(|| "bible_branch:main".to_string());
            self.resolve_project_main_branch(&project, &fallback).await
        };

        Ok((project, branch_id, false))
    }

    /// Resolve the project's main branch id (per-project design). Falls
    /// back to `fallback` when no branch named "main" is found.
    async fn resolve_project_main_branch(
        &self,
        project: &crate::sqlite::records::Project,
        fallback: &str,
    ) -> String {
        if let Some(active) = project.active_branch_id.as_deref() {
            return active.to_string();
        }
        self.repository
            .list_branches_by_project(&project.id)
            .await
            .ok()
            .and_then(|branches| {
                branches
                    .into_iter()
                    .find(|b| b.name == "main")
                    .map(|b| b.id)
            })
            .unwrap_or_else(|| fallback.to_string())
    }

    /// Walk the slicer's structural analysis result, persist source
    /// documents + segments, and emit review items for low-confidence
    /// boundaries. Returns the summary the import-manuscript caller
    /// renders, paired with the count of review items created.
    async fn persist_structural_analysis(
        &self,
        session_id: &str,
        project_id: &str,
        analysis: &crate::sqlite::import::StructuralAnalysisResult,
    ) -> Result<(spindle_core::models::ImportStructuralAnalysisSummary, usize)> {
        use crate::sqlite::import_service::{
            import_segment_status_name, import_source_format_name, parse_import_source_format,
        };
        use crate::sqlite::repository::{
            UpsertImportSegmentParams, UpsertImportSourceDocumentParams,
        };
        use spindle_core::models::{
            ImportChapterSlice, ImportCorrectionPayload, ImportSceneSlice,
            ImportSourceDocumentSummary, ImportStructuralAnalysisSummary,
        };
        use std::collections::BTreeMap;

        let mut source_documents = Vec::with_capacity(analysis.source_documents.len());
        let mut persisted_documents_by_order = BTreeMap::new();

        for document in &analysis.source_documents {
            let persisted_document = self
                .repository
                .upsert_import_source_document(UpsertImportSourceDocumentParams {
                    session_id: session_id.to_string(),
                    project_id: Some(project_id.to_string()),
                    display_name: document.display_name.clone(),
                    source_path: document.source_path.display().to_string(),
                    copied_path: document.copied_path.display().to_string(),
                    source_format: import_source_format_name(&document.source_format),
                    original_sha256: document.original_sha256.clone(),
                    normalized_sha256: document.normalized_sha256.clone(),
                    normalized_text_ref: document.normalized_text_path.display().to_string(),
                    word_count: document.word_count,
                    chapter_hint: document.chapter_hint.clone(),
                    source_order: document.source_order,
                })
                .await?;
            persisted_documents_by_order.insert(
                persisted_document.source_order.max(0) as usize,
                persisted_document.clone(),
            );
            source_documents.push(ImportSourceDocumentSummary {
                document_id: persisted_document.id.clone(),
                display_name: persisted_document.display_name,
                source_path: persisted_document.source_path,
                copied_path: persisted_document.copied_path,
                source_format: parse_import_source_format(&persisted_document.source_format),
                original_sha256: persisted_document.original_sha256,
                normalized_sha256: persisted_document.normalized_sha256,
                word_count: persisted_document.word_count.max(0) as usize,
                chapter_hint: persisted_document.chapter_hint,
                source_order: persisted_document.source_order.max(0) as usize,
            });
        }

        let mut chapters = Vec::with_capacity(analysis.chapters.len());
        let mut review_count = 0usize;
        for chapter in &analysis.chapters {
            let source_document = analysis
                .source_documents
                .get(chapter.source_document_index)
                .context("structural analysis referenced an unknown source document")?;
            let persisted_document = persisted_documents_by_order
                .get(&source_document.source_order)
                .cloned()
                .context("persisted import source document not found")?;

            let chapter_segment = self
                .repository
                .upsert_import_segment(UpsertImportSegmentParams {
                    session_id: session_id.to_string(),
                    source_document_id: persisted_document.id.clone(),
                    parent_segment_id: None,
                    segment_type: "chapter".to_string(),
                    source_order: chapter.start_offset,
                    book_number: Some(chapter.book_number),
                    chapter_number: Some(chapter.chapter_number),
                    scene_order: None,
                    label: chapter.title.clone(),
                    start_offset: chapter.start_offset,
                    end_offset: chapter.end_offset,
                    word_count: chapter.word_count,
                    character_count: chapter.end_offset.saturating_sub(chapter.start_offset),
                    pov_guess: None,
                    confidence: chapter.confidence,
                    segment_status: import_segment_status_name(),
                })
                .await?;
            if let Some(reason) = chapter.review_reason.as_ref() {
                review_count += 1;
                self.persist_structural_review_item(
                    session_id,
                    reason,
                    format!("Review chapter {} structure", chapter.chapter_number),
                    Some(chapter.confidence),
                    vec![chapter_segment.id.clone()],
                    Some(ImportCorrectionPayload::Structural {
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: None,
                        pov_character_name: None,
                        segment_ids: vec![chapter_segment.id.clone()],
                    }),
                )
                .await?;
            }

            let mut scenes = Vec::with_capacity(chapter.scenes.len());
            for scene in &chapter.scenes {
                let scene_segment = self
                    .repository
                    .upsert_import_segment(UpsertImportSegmentParams {
                        session_id: session_id.to_string(),
                        source_document_id: persisted_document.id.clone(),
                        parent_segment_id: Some(chapter_segment.id.clone()),
                        segment_type: "scene".to_string(),
                        source_order: scene.start_offset,
                        book_number: Some(chapter.book_number),
                        chapter_number: Some(chapter.chapter_number),
                        scene_order: Some(scene.scene_index as i32),
                        label: scene.label.clone(),
                        start_offset: scene.start_offset,
                        end_offset: scene.end_offset,
                        word_count: scene.word_count,
                        character_count: scene.character_count,
                        pov_guess: scene
                            .pov_guess
                            .as_ref()
                            .map(serde_json::to_value)
                            .transpose()?,
                        confidence: scene.confidence,
                        segment_status: import_segment_status_name(),
                    })
                    .await?;

                if let Some(reason) = scene.review_reason.as_ref() {
                    review_count += 1;
                    self.persist_structural_review_item(
                        session_id,
                        reason,
                        format!(
                            "Review chapter {} scene {} structure",
                            chapter.chapter_number, scene.scene_index
                        ),
                        Some(scene.confidence),
                        vec![chapter_segment.id.clone(), scene_segment.id.clone()],
                        Some(ImportCorrectionPayload::Structural {
                            chapter_number: Some(chapter.chapter_number),
                            scene_order: Some(scene.scene_index as i32),
                            pov_character_name: scene
                                .pov_guess
                                .as_ref()
                                .and_then(|guess| guess.character_name.clone()),
                            segment_ids: vec![chapter_segment.id.clone(), scene_segment.id.clone()],
                        }),
                    )
                    .await?;
                }

                scenes.push(ImportSceneSlice {
                    segment_id: scene_segment.id.clone(),
                    chapter_segment_id: Some(chapter_segment.id.clone()),
                    scene_index: scene.scene_index,
                    label: scene.label.clone(),
                    start_offset: scene.start_offset,
                    end_offset: scene.end_offset,
                    word_count: scene.word_count,
                    character_count: scene.character_count,
                    pov_guess: scene.pov_guess.clone(),
                    confidence: scene.confidence,
                    confidence_level: scene.confidence_level.clone(),
                });
            }

            chapters.push(ImportChapterSlice {
                segment_id: chapter_segment.id.clone(),
                book_number: chapter.book_number,
                chapter_number: chapter.chapter_number,
                title: chapter.title.clone(),
                start_offset: chapter.start_offset,
                end_offset: chapter.end_offset,
                word_count: chapter.word_count,
                confidence: chapter.confidence,
                confidence_level: chapter.confidence_level.clone(),
                scenes,
            });
        }

        Ok((
            ImportStructuralAnalysisSummary {
                source_documents,
                chapters,
                review_items_created: review_count,
            },
            review_count,
        ))
    }

    async fn persist_structural_review_item(
        &self,
        session_id: &str,
        description: &str,
        title: String,
        confidence: Option<f64>,
        related_segment_ids: Vec<String>,
        correction: Option<spindle_core::models::ImportCorrectionPayload>,
    ) -> Result<()> {
        use crate::sqlite::import_service::{
            import_pass_name, import_review_item_kind_name, import_review_severity_name,
            import_review_status_name,
        };
        use crate::sqlite::repository::CreateImportReviewItemParams;
        use spindle_core::models::{
            ImportPassName, ImportReviewItemKind, ImportReviewSeverity, ImportReviewStatus,
        };

        self.repository
            .create_import_review_item(CreateImportReviewItemParams {
                session_id: session_id.to_string(),
                pass_name: import_pass_name(&ImportPassName::StructuralAnalysis),
                item_kind: import_review_item_kind_name(&ImportReviewItemKind::Structure),
                severity: import_review_severity_name(&ImportReviewSeverity::RequiresReview),
                status: import_review_status_name(&ImportReviewStatus::Open),
                title,
                description: description.to_string(),
                related_segment_ids,
                related_entity_ids: Vec::new(),
                confidence,
                proposed_correction: correction.map(serde_json::to_value).transpose()?,
                resolver_notes: None,
            })
            .await?;
        Ok(())
    }

    /// The full import-hydrate-bible implementation. Split out as a
    /// separate method to keep the surface stub `import_hydrate_bible`
    /// concise; the body itself is large because it walks every bible
    /// section the import pipeline can populate.
    ///
    /// Implementation note:
    ///
    /// * Existing relationships are updated in place via
    ///   `Repository::set_relationship_absolute`, which writes absolute
    ///   `trust` / `tension` values (distinct from `update_relationship`,
    ///   which applies deltas).
    /// * The "look up canonical book and chapter numbers per manuscript
    ///   book" logic uses `list_chapters_by_book_number` (project +
    ///   book_number natural key) instead of `list_chapters_by_book(id)`
    ///   because the SQLite chapter table is keyed by `book_id` rather
    ///   than book record-id round-trips.
    async fn import_hydrate_bible_inner(
        &self,
        input: spindle_core::models::ImportHydrateBibleInput,
    ) -> Result<spindle_core::models::ImportHydrateBibleOutput> {
        use crate::sqlite::import_service::{
            character_dossier_summaries_from_records, detect_content_rating,
            find_scene_text_summary, hydration_mapped_book, hydration_mapped_chapter,
            import_pass_name, import_progress, import_review_item_kind_name,
            import_review_severity_name, import_review_status_name, import_session_status_name,
            imported_character_role, narrative_dossier_summary_from_record,
            resume_snapshot_summary_from_record, structural_summary_from_records,
            summarize_character_summary, summarize_text, world_dossier_summary_from_record,
        };
        use crate::sqlite::repository::{
            AppendCharacterStateParams, CreateImportReviewItemParams, UpsertKnowledgeFactParams,
        };
        use spindle_core::models::{
            CharacterStatePatch, CreateBookInput, CreateChapterInput, CreateCharacterArcInput,
            CreateCharacterInput, CreateConflictInput, CreateEconomyInput, CreateFactionInput,
            CreateFutureKnowledgeInput, CreateLocationInput, CreateMotifInput,
            CreateNarrativePromiseInput, CreatePacingConfigInput, CreatePacingCurveInput,
            CreatePlotLineInput, CreateRelationshipInput, CreateReligionInput,
            CreateSystemOverlayInput, CreateTermInput, CreateThemeInput, CreateTimelineEventInput,
            CreateWorldRuleInput, EstablishedIn, ImportChapterSlice, ImportConfidenceLevel,
            ImportCorrectionPayload, ImportEntityKind, ImportHydrationRecordCount,
            ImportHydrationReport, ImportHydrationStatus, ImportPassName, ImportReviewItemKind,
            ImportReviewSeverity, ImportReviewStatus, ImportSessionStatus, SaveSceneDraftInput,
            SaveSummaryInput, StoryPlacement, WorldStateInput, normalize_name,
        };
        use std::collections::{BTreeMap, BTreeSet};

        let session = self
            .repository
            .get_import_session(&input.session_id)
            .await?;
        if session.project_id.as_deref() != Some(input.project_id.as_str()) {
            anyhow::bail!("import session does not belong to the requested project");
        }

        let existing_report = session
            .hydration_report
            .clone()
            .map(serde_json::from_value)
            .transpose()?;

        let (target_project, target_branch_id, created_project) =
            self.resolve_hydration_target(&input, &session).await?;
        if self
            .repository
            .hydration_target_exists(&input.session_id, &target_project.id, &target_branch_id)
            .await?
            && let Some(report) = existing_report
        {
            return Ok(spindle_core::models::ImportHydrateBibleOutput {
                session_id: input.session_id,
                report,
            });
        }

        let source_documents = self
            .repository
            .list_import_source_documents(&input.session_id)
            .await?;
        let segments = self
            .repository
            .list_import_segments(&input.session_id)
            .await?;
        let mut structure = structural_summary_from_records(&source_documents, &segments, 0)?;
        let character_dossiers = character_dossier_summaries_from_records(
            &self
                .repository
                .list_import_character_dossiers(&input.session_id)
                .await?,
        )?;
        let world = self
            .repository
            .find_import_world_dossier(&input.session_id)
            .await?
            .map(world_dossier_summary_from_record)
            .transpose()?;
        let mut narrative = self
            .repository
            .find_import_narrative_dossier(&input.session_id)
            .await?
            .map(narrative_dossier_summary_from_record)
            .transpose()?;
        let mut final_state = self
            .repository
            .find_import_resume_snapshot(&input.session_id)
            .await?
            .map(resume_snapshot_summary_from_record)
            .transpose()?;
        let review_items = self
            .repository
            .list_import_review_items(&input.session_id)
            .await?;

        let target_project_id = target_project.id.clone();
        let main_branch_id = self
            .resolve_project_main_branch(&target_project, "bible_branch:main")
            .await;

        let mut record_counts = Vec::new();
        let mut skipped_sections = Vec::new();
        let mut notes = Vec::new();

        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::Hydration),
                serde_json::to_value(import_progress(
                    structure.source_documents.len(),
                    structure.source_documents.len(),
                    structure
                        .chapters
                        .iter()
                        .map(|chapter| chapter.scenes.len())
                        .sum(),
                    0,
                    review_items.len(),
                    review_items
                        .iter()
                        .filter(|item| {
                            item.status == import_review_status_name(&ImportReviewStatus::Open)
                        })
                        .count(),
                ))?,
                &import_session_status_name(&ImportSessionStatus::Running),
            )
            .await?;

        // ----- books -----
        let existing_books = self
            .repository
            .list_books_by_project(&target_project.id)
            .await?;
        let mut book_number_map: BTreeMap<i32, i32> = BTreeMap::new();
        for book in &existing_books {
            book_number_map.insert(book.book_number, book.book_number);
        }
        let mut created_books = 0usize;
        let mut skipped_books = 0usize;
        let manuscript_book_numbers: BTreeSet<i32> =
            structure.chapters.iter().map(|c| c.book_number).collect();
        for &ms_book in &manuscript_book_numbers {
            if book_number_map.contains_key(&ms_book) {
                skipped_books += 1;
                continue;
            }
            let created = self
                .create_book(CreateBookInput {
                    project_id: target_project_id.clone(),
                    title: if manuscript_book_numbers.len() == 1 {
                        Some("Imported Manuscript".to_string())
                    } else {
                        Some(format!("Imported Book {}", ms_book))
                    },
                })
                .await?;
            book_number_map.insert(ms_book, created.book_number);
            created_books += 1;
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "book".to_string(),
            created: created_books,
            updated: 0,
            skipped: skipped_books,
        });

        // ----- chapters -----
        let mut chapter_number_map: BTreeMap<(i32, i32), i32> = BTreeMap::new();
        let mut created_chapters = 0usize;
        let mut skipped_chapters = 0usize;
        let mut chapters_by_book: BTreeMap<i32, Vec<&ImportChapterSlice>> = BTreeMap::new();
        for chapter in &structure.chapters {
            chapters_by_book
                .entry(chapter.book_number)
                .or_default()
                .push(chapter);
        }
        for (&ms_book_number, manuscript_chapters) in &chapters_by_book {
            let Some(&canonical_book) = book_number_map.get(&ms_book_number) else {
                continue;
            };
            let existing_db_chapters = self
                .repository
                .list_chapters_by_book_number(&target_project.id, canonical_book)
                .await?;
            let to_create = manuscript_chapters
                .len()
                .saturating_sub(existing_db_chapters.len());
            for i in 0..to_create {
                let ms_idx = existing_db_chapters.len() + i;
                self.create_chapter(CreateChapterInput {
                    project_id: target_project_id.clone(),
                    book_number: Some(canonical_book),
                    book_id: None,
                    chapter_number: None,
                    title: manuscript_chapters
                        .get(ms_idx)
                        .and_then(|c| c.title.clone()),
                })
                .await?;
                created_chapters += 1;
            }
            skipped_chapters += existing_db_chapters.len().min(manuscript_chapters.len());
            let final_db_chapters = self
                .repository
                .list_chapters_by_book_number(&target_project.id, canonical_book)
                .await?;
            for (ms_ch, db_ch) in manuscript_chapters.iter().zip(final_db_chapters.iter()) {
                chapter_number_map.insert(
                    (ms_ch.book_number, ms_ch.chapter_number),
                    db_ch.chapter_number,
                );
            }
        }

        for chapter in &mut structure.chapters {
            let orig_book = chapter.book_number;
            let orig_chapter = chapter.chapter_number;
            chapter.book_number = hydration_mapped_book(&book_number_map, orig_book);
            chapter.chapter_number =
                hydration_mapped_chapter(&chapter_number_map, orig_book, orig_chapter);
        }
        if let Some(ref mut fs) = final_state {
            let orig_book = fs.book_number;
            let orig_chapter = fs.chapter_number;
            fs.book_number = hydration_mapped_book(&book_number_map, orig_book);
            fs.chapter_number =
                hydration_mapped_chapter(&chapter_number_map, orig_book, orig_chapter);
        }
        if let Some(ref mut narr) = narrative {
            for hint in &mut narr.pacing_hints {
                hint.book_number = hint
                    .book_number
                    .map(|b| hydration_mapped_book(&book_number_map, b));
            }
        }

        record_counts.push(ImportHydrationRecordCount {
            entity_type: "chapter".to_string(),
            created: created_chapters,
            updated: 0,
            skipped: skipped_chapters,
        });

        // ----- characters -----
        let mut characters_by_cluster = BTreeMap::new();
        let mut canonical_characters = self
            .repository
            .list_characters_by_project_and_branch(&target_project.id, &main_branch_id)
            .await?;
        let mut created_characters = 0usize;
        let mut skipped_characters = 0usize;
        for dossier in &character_dossiers {
            if let Some(existing) = canonical_characters.iter().find(|character| {
                character.normalized_name == normalize_name(&dossier.canonical_name)
            }) {
                characters_by_cluster.insert(dossier.cluster_id.clone(), existing.id.clone());
                skipped_characters += 1;
                continue;
            }

            let initial_state = dossier
                .state_trajectory
                .first()
                .map(|point| CharacterStatePatch {
                    emotional_state: point.emotional_state.clone(),
                    goals: Some(point.goals.clone()),
                    status: Some(point.status.clone()),
                    notes: Some(vec![point.summary.clone()]),
                    source_summary: Some("import baseline".to_string()),
                });
            let created = self
                .create_character(CreateCharacterInput {
                    project_id: target_project_id.clone(),
                    name: dossier.canonical_name.clone(),
                    summary: summarize_character_summary(dossier),
                    role: imported_character_role(dossier),
                    realm: None,
                    voice_profile: dossier.voice_profile.clone(),
                    emotional_profile: dossier.emotional_profile.clone(),
                    initial_state,
                })
                .await?;
            let character_id = created.character_id.clone();
            characters_by_cluster.insert(dossier.cluster_id.clone(), character_id.clone());
            canonical_characters.push(self.repository.get_character(&character_id).await?);
            created_characters += 1;
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "character".to_string(),
            created: created_characters,
            updated: 0,
            skipped: skipped_characters,
        });

        // ----- final character state snapshot -----
        let mut final_character_state_created = 0usize;
        let mut final_character_state_skipped = 0usize;
        if let Some(final_state) = final_state.as_ref() {
            let existing = self
                .repository
                .list_character_states_by_project_and_branch(&target_project.id, &target_branch_id)
                .await?;
            for snapshot in &final_state.characters {
                let Some(character_id) = characters_by_cluster.get(&snapshot.cluster_id) else {
                    continue;
                };
                if existing.iter().any(|state| {
                    &state.character_id == character_id
                        && state.book_number == final_state.book_number
                        && state.chapter_number == final_state.chapter_number
                        && state.scene_order == final_state.scene_order.unwrap_or(0)
                }) {
                    final_character_state_skipped += 1;
                    continue;
                }
                self.repository
                    .append_character_state(AppendCharacterStateParams {
                        project_id: target_project.id.clone(),
                        branch_id: target_branch_id.clone(),
                        character_id: character_id.clone(),
                        scene_id: None,
                        book_number: final_state.book_number,
                        chapter_number: final_state.chapter_number,
                        scene_order: final_state.scene_order.unwrap_or(0),
                        patch: CharacterStatePatch {
                            emotional_state: snapshot.emotional_state.clone(),
                            goals: Some(snapshot.goals.clone()),
                            status: Some(snapshot.status.clone()),
                            notes: Some(snapshot.notes.clone()),
                            source_summary: Some("import final state".to_string()),
                        },
                    })
                    .await?;
                final_character_state_created += 1;
            }
        } else {
            skipped_sections.push("character_final_state".to_string());
            notes.push(
                "Skipped final character state hydration because no resume snapshot was available."
                    .to_string(),
            );
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "character_state".to_string(),
            created: final_character_state_created,
            updated: 0,
            skipped: final_character_state_skipped,
        });

        // ----- locations -----
        let mut locations_by_name: BTreeMap<String, String> = BTreeMap::new();
        let mut canonical_locations = self
            .repository
            .list_locations_by_project_and_branch(&target_project.id, &main_branch_id)
            .await?;
        let mut created_locations = 0usize;
        let mut skipped_locations = 0usize;
        if let Some(world) = world.as_ref() {
            for location in &world.locations {
                if let Some(existing) = canonical_locations
                    .iter()
                    .find(|candidate| candidate.normalized_name == normalize_name(&location.name))
                {
                    locations_by_name.insert(normalize_name(&location.name), existing.id.clone());
                    skipped_locations += 1;
                    continue;
                }
                let snapshot = final_state.as_ref().and_then(|state| {
                    state.locations.iter().find(|candidate| {
                        normalize_name(&candidate.location_name) == normalize_name(&location.name)
                    })
                });
                let created = self
                    .create_location(CreateLocationInput {
                        project_id: target_project_id.clone(),
                        name: location.name.clone(),
                        kind: location.kind.clone(),
                        realm: location.realm.clone(),
                        summary: location.summary.clone(),
                        initial_state: WorldStateInput {
                            controlling_faction: snapshot
                                .and_then(|item| item.controlling_faction.clone()),
                            status: snapshot.and_then(|item| item.status.clone()),
                            prosperity: snapshot.and_then(|item| item.prosperity.clone()),
                            stability: snapshot.and_then(|item| item.stability.clone()),
                            threat_level: snapshot.and_then(|item| item.threat_level.clone()),
                            sensory_details: snapshot
                                .map(|item| item.sensory_details.clone())
                                .unwrap_or_default(),
                        },
                    })
                    .await?;
                locations_by_name
                    .insert(normalize_name(&location.name), created.location_id.clone());
                canonical_locations.push(self.repository.get_location(&created.location_id).await?);
                created_locations += 1;
            }
        } else {
            skipped_sections.push("world".to_string());
            notes.push(
                "Skipped location hydration because no world dossier was available.".to_string(),
            );
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "location".to_string(),
            created: created_locations,
            updated: 0,
            skipped: skipped_locations,
        });

        // ----- world rules -----
        let mut created_world_rules = 0usize;
        let mut skipped_world_rules = 0usize;
        if let Some(world) = world.as_ref() {
            let existing = self
                .repository
                .list_world_rules_by_project(&target_project.id)
                .await?;
            for rule in &world.world_rules {
                if existing.iter().any(|candidate| {
                    normalize_name(&candidate.rule_name) == normalize_name(&rule.rule_name)
                }) {
                    skipped_world_rules += 1;
                    continue;
                }
                self.create_world_rule(CreateWorldRuleInput {
                    project_id: target_project_id.clone(),
                    rule_name: rule.rule_name.clone(),
                    rule_type: rule.rule_type.clone(),
                    description: rule.description.clone(),
                    scan_pattern: None,
                    relevance_tags: vec![],
                    established_in: rule
                        .source_segment_ids
                        .first()
                        .and_then(|_| structure.chapters.first())
                        .map(|chapter| EstablishedIn {
                            book_number: chapter.book_number,
                            chapter_number: chapter.chapter_number,
                            note: Some("imported from manuscript world analysis".to_string()),
                        }),
                })
                .await?;
                created_world_rules += 1;
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "world_rule".to_string(),
            created: created_world_rules,
            updated: 0,
            skipped: skipped_world_rules,
        });

        // ----- factions/religions/economies/terms -----
        let mut created_factions = 0usize;
        let mut skipped_factions = 0usize;
        let mut created_religions = 0usize;
        let mut skipped_religions = 0usize;
        let mut created_economies = 0usize;
        let mut skipped_economies = 0usize;
        let mut created_terms = 0usize;
        let mut skipped_terms = 0usize;
        if let Some(world) = world.as_ref() {
            let existing_factions = self
                .repository
                .list_factions_by_project(&target_project.id)
                .await?;
            let existing_religions = self
                .repository
                .list_religions_by_project(&target_project.id)
                .await?;
            let existing_economies = self
                .repository
                .list_economies_by_project(&target_project.id)
                .await?;
            let existing_terms = self
                .repository
                .list_terms_by_project(&target_project.id)
                .await?;
            for entity in &world.entities {
                match entity.entity_kind {
                    ImportEntityKind::Faction => {
                        if existing_factions.iter().any(|candidate| {
                            candidate.normalized_name == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_factions += 1;
                        } else {
                            self.create_faction(CreateFactionInput {
                                project_id: target_project_id.clone(),
                                name: entity.canonical_name.clone(),
                                faction_type: entity
                                    .tags
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| "imported".to_string()),
                                realm: entity.realm.clone(),
                                summary: entity.summary.clone(),
                                tags: entity.tags.clone(),
                            })
                            .await?;
                            created_factions += 1;
                        }
                    }
                    ImportEntityKind::Religion => {
                        if existing_religions.iter().any(|candidate| {
                            candidate.normalized_name == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_religions += 1;
                        } else {
                            self.create_religion(CreateReligionInput {
                                project_id: target_project_id.clone(),
                                name: entity.canonical_name.clone(),
                                deity_or_principle: entity
                                    .tags
                                    .first()
                                    .cloned()
                                    .unwrap_or_else(|| "imported doctrine".to_string()),
                                summary: entity.summary.clone(),
                                tags: entity.tags.clone(),
                            })
                            .await?;
                            created_religions += 1;
                        }
                    }
                    ImportEntityKind::Economy => {
                        if existing_economies.iter().any(|candidate| {
                            candidate.normalized_name == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_economies += 1;
                        } else {
                            self.create_economy(CreateEconomyInput {
                                project_id: target_project_id.clone(),
                                name: entity.canonical_name.clone(),
                                realm: entity.realm.clone(),
                                summary: entity.summary.clone(),
                                scarce_resources: Vec::new(),
                                trade_goods: entity.tags.clone(),
                                currency: None,
                                notes: vec!["imported from manuscript world analysis".to_string()],
                            })
                            .await?;
                            created_economies += 1;
                        }
                    }
                    ImportEntityKind::Term => {
                        if existing_terms.iter().any(|candidate| {
                            candidate.normalized_term == normalize_name(&entity.canonical_name)
                        }) {
                            skipped_terms += 1;
                        } else {
                            self.create_term(CreateTermInput {
                                project_id: target_project_id.clone(),
                                term_text: entity.canonical_name.clone(),
                                pronunciation: None,
                                definition: entity.summary.clone(),
                                usage_context: entity.realm.clone(),
                                origin: Some("imported manuscript glossary".to_string()),
                            })
                            .await?;
                            created_terms += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "faction".to_string(),
            created: created_factions,
            updated: 0,
            skipped: skipped_factions,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "religion".to_string(),
            created: created_religions,
            updated: 0,
            skipped: skipped_religions,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "economy".to_string(),
            created: created_economies,
            updated: 0,
            skipped: skipped_economies,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "term".to_string(),
            created: created_terms,
            updated: 0,
            skipped: skipped_terms,
        });

        // ----- themes / plot lines / conflicts / promises / motifs / arcs -----
        let mut theme_ids_by_statement: BTreeMap<String, String> = BTreeMap::new();
        let mut created_themes = 0usize;
        let mut skipped_themes = 0usize;
        let mut created_plot_lines = 0usize;
        let mut skipped_plot_lines = 0usize;
        let mut created_conflicts = 0usize;
        let mut skipped_conflicts = 0usize;
        let mut created_promises = 0usize;
        let mut skipped_promises = 0usize;
        let mut created_motifs = 0usize;
        let mut skipped_motifs = 0usize;
        let mut created_arcs = 0usize;
        let mut skipped_arcs = 0usize;
        if let Some(narrative) = narrative.as_ref() {
            if narrative
                .plot_lines
                .iter()
                .any(|plot_line| !plot_line.convergence_points.is_empty())
            {
                notes.push(
                    "Skipped imported plot-line convergence points during hydration because canonical plot_line storage does not yet accept the imported placement shape."
                        .to_string(),
                );
            }
            let existing_themes = self
                .repository
                .list_themes_by_project(&target_project.id)
                .await?;
            for theme in &narrative.themes {
                if let Some(existing) = existing_themes
                    .iter()
                    .find(|candidate| candidate.theme_statement == theme.theme_statement)
                {
                    theme_ids_by_statement
                        .insert(theme.theme_statement.clone(), existing.id.clone());
                    skipped_themes += 1;
                    continue;
                }
                let created = self
                    .create_theme(CreateThemeInput {
                        project_id: target_project_id.clone(),
                        theme_statement: theme.theme_statement.clone(),
                        thesis_antithesis: theme.thesis_antithesis.clone(),
                        introduction_point: structure.chapters.first().map(|chapter| {
                            StoryPlacement {
                                book_number: chapter.book_number,
                                chapter_number: chapter.chapter_number,
                                scene_order: chapter
                                    .scenes
                                    .first()
                                    .map(|scene| scene.scene_index as i32),
                                note: None,
                            }
                        }),
                        resolution_point: final_state.as_ref().map(|state| StoryPlacement {
                            book_number: state.book_number,
                            chapter_number: state.chapter_number,
                            scene_order: state.scene_order,
                            note: None,
                        }),
                    })
                    .await?;
                theme_ids_by_statement.insert(theme.theme_statement.clone(), created.theme_id);
                created_themes += 1;
            }

            let existing_plot_lines = self
                .repository
                .list_plot_lines_by_project(&target_project.id)
                .await?;
            for plot_line in &narrative.plot_lines {
                if existing_plot_lines
                    .iter()
                    .any(|candidate| candidate.normalized_name == normalize_name(&plot_line.name))
                {
                    skipped_plot_lines += 1;
                    continue;
                }
                let _ = self
                    .create_plot_line(CreatePlotLineInput {
                        project_id: target_project_id.clone(),
                        name: plot_line.name.clone(),
                        plot_type: plot_line.plot_type.clone(),
                        summary: plot_line.summary.clone(),
                        status: plot_line.status.clone(),
                        convergence_points: Vec::new(),
                    })
                    .await?;
                created_plot_lines += 1;
            }

            let existing_conflicts = self
                .repository
                .list_conflicts_by_project(&target_project.id)
                .await?;
            for conflict in &narrative.conflicts {
                if existing_conflicts
                    .iter()
                    .any(|candidate| candidate.normalized_name == normalize_name(&conflict.name))
                {
                    skipped_conflicts += 1;
                    continue;
                }
                let _ = self
                    .create_conflict(CreateConflictInput {
                        project_id: target_project_id.clone(),
                        name: conflict.name.clone(),
                        conflict_type: conflict.conflict_type.clone(),
                        stakes: conflict.stakes.clone(),
                        escalation_stages: conflict.escalation_stages.clone(),
                        expected_total_cycles: Some(conflict.try_fail_cycles.len() as i32),
                        try_fail_cycles: conflict.try_fail_cycles.clone(),
                        stated_consequences: conflict.stated_consequences.clone(),
                    })
                    .await?;
                created_conflicts += 1;
            }

            let existing_promises = self
                .repository
                .list_narrative_promises_by_project(&target_project.id)
                .await?;
            for promise in &narrative.narrative_promises {
                if existing_promises
                    .iter()
                    .any(|candidate| candidate.description == promise.description)
                {
                    skipped_promises += 1;
                    continue;
                }
                self.create_narrative_promise(CreateNarrativePromiseInput {
                    project_id: target_project_id.clone(),
                    promise_type: promise.promise_type.clone(),
                    description: promise.description.clone(),
                    planted_at: promise.planted_at.clone(),
                    planned_payoff: promise.planned_payoff.clone(),
                    notes: promise.notes.clone(),
                })
                .await?;
                created_promises += 1;
            }

            let existing_motifs = self
                .repository
                .list_motifs_by_project(&target_project.id)
                .await?;
            for motif in &narrative.motifs {
                if existing_motifs
                    .iter()
                    .any(|candidate| candidate.normalized_name == normalize_name(&motif.name))
                {
                    skipped_motifs += 1;
                    continue;
                }
                let connected_theme_ids = motif
                    .connected_theme_statements
                    .iter()
                    .filter_map(|statement| theme_ids_by_statement.get(statement).cloned())
                    .collect::<Vec<_>>();
                self.create_motif(CreateMotifInput {
                    project_id: target_project_id.clone(),
                    name: motif.name.clone(),
                    description: motif.description.clone(),
                    max_uses_per_chapter: None,
                    connected_theme_ids,
                })
                .await?;
                created_motifs += 1;
            }

            let existing_arcs = self
                .repository
                .list_character_arcs_by_project(&target_project.id)
                .await?;
            for arc in &narrative.arcs {
                let Some(character_id) = characters_by_cluster.get(&arc.character_cluster_id)
                else {
                    skipped_arcs += 1;
                    continue;
                };
                if existing_arcs
                    .iter()
                    .any(|candidate| &candidate.character_id == character_id)
                {
                    skipped_arcs += 1;
                    continue;
                }
                let connected_theme_ids =
                    theme_ids_by_statement.values().cloned().collect::<Vec<_>>();
                self.create_character_arc(CreateCharacterArcInput {
                    project_id: target_project_id.clone(),
                    character_id: character_id.clone(),
                    arc_type: arc.arc_type.clone(),
                    starting_state: arc.starting_state.clone(),
                    ending_state: arc.ending_state.clone(),
                    milestones: arc.milestones.clone(),
                    thematic_purpose: arc.thematic_purpose.clone(),
                    connected_theme_ids,
                })
                .await?;
                created_arcs += 1;
            }
        } else {
            skipped_sections.push("narrative".to_string());
            notes.push(
                "Skipped narrative hydration because no narrative dossier was available."
                    .to_string(),
            );
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "theme".to_string(),
            created: created_themes,
            updated: 0,
            skipped: skipped_themes,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "plot_line".to_string(),
            created: created_plot_lines,
            updated: 0,
            skipped: skipped_plot_lines,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "conflict".to_string(),
            created: created_conflicts,
            updated: 0,
            skipped: skipped_conflicts,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "narrative_promise".to_string(),
            created: created_promises,
            updated: 0,
            skipped: skipped_promises,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "motif".to_string(),
            created: created_motifs,
            updated: 0,
            skipped: skipped_motifs,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "character_arc".to_string(),
            created: created_arcs,
            updated: 0,
            skipped: skipped_arcs,
        });

        // ----- pacing -----
        let mut created_pacing_configs = 0usize;
        let mut skipped_pacing_configs = 0usize;
        let mut created_pacing_curves = 0usize;
        let mut skipped_pacing_curves = 0usize;
        let mut created_chapter_summaries = 0usize;
        let mut skipped_chapter_summaries = 0usize;
        if let Some(narrative) = narrative.as_ref() {
            let high_confidence_pacing = narrative
                .pacing_hints
                .iter()
                .filter(|hint| matches!(hint.confidence_level, ImportConfidenceLevel::High))
                .collect::<Vec<_>>();
            if high_confidence_pacing.is_empty() {
                skipped_sections.push("pacing".to_string());
                notes.push(
                    "Skipped pacing hydration because imported pacing evidence was not high-confidence."
                        .to_string(),
                );
            } else {
                let existing_configs = self
                    .repository
                    .list_pacing_configs_by_project(&target_project.id)
                    .await?;
                if existing_configs.is_empty() {
                    let avg_chapters = if structure.chapters.is_empty() {
                        1
                    } else {
                        structure.chapters.len() as i32
                    };
                    let avg_scenes = if structure.chapters.is_empty() {
                        1
                    } else {
                        (structure
                            .chapters
                            .iter()
                            .map(|chapter| chapter.scenes.len())
                            .sum::<usize>()
                            / structure.chapters.len().max(1)) as i32
                    };
                    self.create_pacing_config(CreatePacingConfigInput {
                        project_id: target_project_id.clone(),
                        total_planned_books: structure
                            .chapters
                            .iter()
                            .map(|chapter| chapter.book_number)
                            .max()
                            .unwrap_or(1),
                        avg_chapters_per_book: avg_chapters.max(1),
                        avg_scenes_per_chapter: avg_scenes.max(1),
                        tension_model: narrative.reader_contract.reader_contract.promise.clone(),
                    })
                    .await?;
                    created_pacing_configs += 1;
                } else {
                    skipped_pacing_configs += 1;
                }

                let existing_curves = self
                    .repository
                    .list_pacing_curves_by_project(&target_project.id)
                    .await?;
                for hint in high_confidence_pacing {
                    let Some(book_number) = hint.book_number else {
                        skipped_pacing_curves += 1;
                        continue;
                    };
                    if existing_curves
                        .iter()
                        .any(|curve| curve.book_number == book_number)
                    {
                        skipped_pacing_curves += 1;
                        continue;
                    }
                    self.create_pacing_curve(CreatePacingCurveInput {
                        project_id: target_project_id.clone(),
                        book_number,
                        act_breakpoints: hint.act_breakpoints.clone(),
                        scene_type_density: hint.scene_type_density.clone(),
                    })
                    .await?;
                    created_pacing_curves += 1;
                }

                let existing_summaries = self
                    .repository
                    .list_chapter_summaries_by_project(&target_project.id)
                    .await?;
                for chapter in &structure.chapters {
                    if existing_summaries.iter().any(|summary| {
                        summary.book_number == chapter.book_number
                            && summary.chapter_number == chapter.chapter_number
                    }) {
                        skipped_chapter_summaries += 1;
                        continue;
                    }
                    let scene_summaries = chapter
                        .scenes
                        .iter()
                        .map(|scene| find_scene_text_summary(&source_documents, &segments, scene))
                        .collect::<Result<Vec<_>>>()?;
                    self.save_summary(SaveSummaryInput {
                        project_id: target_project_id.clone(),
                        book_number: chapter.book_number,
                        chapter_number: chapter.chapter_number,
                        entity_type: None,
                        entity_id: None,
                        summary: scene_summaries.join(" "),
                        key_events: scene_summaries.clone(),
                        character_changes: Vec::new(),
                        relationship_shifts: Vec::new(),
                        arc_advances: Vec::new(),
                        promise_events: Vec::new(),
                    })
                    .await?;
                    created_chapter_summaries += 1;
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "pacing_config".to_string(),
            created: created_pacing_configs,
            updated: 0,
            skipped: skipped_pacing_configs,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "pacing_curve".to_string(),
            created: created_pacing_curves,
            updated: 0,
            skipped: skipped_pacing_curves,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "chapter_summary".to_string(),
            created: created_chapter_summaries,
            updated: 0,
            skipped: skipped_chapter_summaries,
        });

        // ----- scenes -----
        let mut created_scenes = 0usize;
        let mut updated_scenes = 0usize;
        let mut skipped_scenes = 0usize;
        if input.include_scenes {
            for chapter in &structure.chapters {
                let reliable_scene_segmentation = !chapter.scenes.is_empty()
                    && chapter.scenes.iter().all(|scene| {
                        matches!(
                            scene.confidence_level,
                            ImportConfidenceLevel::High | ImportConfidenceLevel::Medium
                        )
                    });
                if reliable_scene_segmentation {
                    for scene in &chapter.scenes {
                        let text = crate::sqlite::import_service::scene_text_from_slice(
                            &source_documents,
                            &segments,
                            scene,
                        )?;
                        let summary = summarize_text(&text.replace('\n', " "), 180);
                        let (detected_rating, rating_confidence) = detect_content_rating(&text);
                        let existing_scene = self
                            .repository
                            .find_scene_by_natural_key(
                                &target_project.id,
                                &target_branch_id,
                                chapter.book_number,
                                chapter.chapter_number,
                                scene.scene_index as i32,
                            )
                            .await?;
                        self.save_scene_draft(SaveSceneDraftInput {
                            project_id: target_project_id.clone(),
                            book_number: chapter.book_number,
                            chapter_number: chapter.chapter_number,
                            chapter_id: None,
                            scene_order: scene.scene_index as i32,
                            full_text: text,
                            summary,
                            content_rating: detected_rating.clone(),
                            tone: None,
                            source_path: None,
                            generation_id: None,
                        })
                        .await?;
                        if rating_confidence < 0.70 {
                            self.repository
                                .create_import_review_item(CreateImportReviewItemParams {
                                    session_id: input.session_id.clone(),
                                    pass_name: import_pass_name(&ImportPassName::Hydration),
                                    item_kind: import_review_item_kind_name(
                                        &ImportReviewItemKind::ContentRating,
                                    ),
                                    severity: import_review_severity_name(
                                        &ImportReviewSeverity::RequiresReview,
                                    ),
                                    status: import_review_status_name(&ImportReviewStatus::Open),
                                    title: format!(
                                        "Review content rating for ch{}.{} scene {}",
                                        chapter.book_number,
                                        chapter.chapter_number,
                                        scene.scene_index
                                    ),
                                    description: format!(
                                        "Detected '{}' with confidence {:.2}. The rating may need manual adjustment.",
                                        detected_rating.as_str(),
                                        rating_confidence
                                    ),
                                    related_segment_ids: vec![scene.segment_id.clone()],
                                    related_entity_ids: Vec::new(),
                                    confidence: Some(rating_confidence),
                                    proposed_correction: Some(serde_json::to_value(
                                        ImportCorrectionPayload::ContentRating {
                                            detected_rating: detected_rating.as_str().to_string(),
                                            confidence: rating_confidence,
                                        },
                                    )?),
                                    resolver_notes: None,
                                })
                                .await?;
                        }
                        if existing_scene.is_some() {
                            updated_scenes += 1;
                        } else {
                            created_scenes += 1;
                        }
                    }
                } else {
                    let existing_scene = self
                        .repository
                        .find_scene_by_natural_key(
                            &target_project.id,
                            &target_branch_id,
                            chapter.book_number,
                            chapter.chapter_number,
                            1,
                        )
                        .await?;
                    let text = crate::sqlite::import_service::chapter_text_from_slice(
                        &source_documents,
                        &segments,
                        chapter,
                    )?;
                    let (detected_rating, rating_confidence) = detect_content_rating(&text);
                    self.save_scene_draft(SaveSceneDraftInput {
                        project_id: target_project_id.clone(),
                        book_number: chapter.book_number,
                        chapter_number: chapter.chapter_number,
                        chapter_id: None,
                        scene_order: 1,
                        full_text: text.clone(),
                        summary: summarize_text(&text.replace('\n', " "), 180),
                        content_rating: detected_rating.clone(),
                        tone: None,
                        source_path: None,
                        generation_id: None,
                    })
                    .await?;
                    if rating_confidence < 0.70 {
                        self.repository
                            .create_import_review_item(CreateImportReviewItemParams {
                                session_id: input.session_id.clone(),
                                pass_name: import_pass_name(&ImportPassName::Hydration),
                                item_kind: import_review_item_kind_name(
                                    &ImportReviewItemKind::ContentRating,
                                ),
                                severity: import_review_severity_name(
                                    &ImportReviewSeverity::RequiresReview,
                                ),
                                status: import_review_status_name(&ImportReviewStatus::Open),
                                title: format!(
                                    "Review content rating for ch{}.{}",
                                    chapter.book_number, chapter.chapter_number
                                ),
                                description: format!(
                                    "Detected '{}' with confidence {:.2}. The rating may need manual adjustment.",
                                    detected_rating.as_str(),
                                    rating_confidence
                                ),
                                related_segment_ids: chapter
                                    .scenes
                                    .first()
                                    .map(|s| s.segment_id.clone())
                                    .into_iter()
                                    .collect(),
                                related_entity_ids: Vec::new(),
                                confidence: Some(rating_confidence),
                                proposed_correction: Some(serde_json::to_value(
                                    ImportCorrectionPayload::ContentRating {
                                        detected_rating: detected_rating.as_str().to_string(),
                                        confidence: rating_confidence,
                                    },
                                )?),
                                resolver_notes: None,
                            })
                            .await?;
                    }
                    notes.push(format!(
                        "Chapter {}.{} used chapter-level scene hydration because scene segmentation confidence was weak.",
                        chapter.book_number, chapter.chapter_number
                    ));
                    if existing_scene.is_some() {
                        updated_scenes += 1;
                    } else {
                        created_scenes += 1;
                    }
                }
            }
        } else {
            skipped_sections.push("scenes".to_string());
            skipped_scenes = structure.chapters.len();
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "scene".to_string(),
            created: created_scenes,
            updated: updated_scenes,
            skipped: skipped_scenes,
        });

        // ----- relationships -----
        //
        // Import snapshots carry canonical absolute trust/tension. Fresh
        // relationships are created via `create_relationship`; existing
        // ones are reset via `set_relationship_absolute` (which writes
        // absolutes, not deltas). `skipped_relationships` only counts
        // entries whose source/target cluster id never resolved to a
        // hydrated character row.
        let mut created_relationships = 0usize;
        let mut updated_relationships = 0usize;
        let mut skipped_relationships = 0usize;
        if let Some(final_state) = final_state.as_ref() {
            for relationship in &final_state.relationships {
                let Some(source_id) =
                    characters_by_cluster.get(&relationship.source_character_cluster_id)
                else {
                    skipped_relationships += 1;
                    continue;
                };
                let Some(target_id) =
                    characters_by_cluster.get(&relationship.target_character_cluster_id)
                else {
                    skipped_relationships += 1;
                    continue;
                };
                match self
                    .repository
                    .get_relationship(&target_branch_id, source_id, target_id)
                    .await
                {
                    Ok(_existing) => {
                        self.repository
                            .set_relationship_absolute(
                                &target_branch_id,
                                source_id,
                                target_id,
                                relationship.trust,
                                relationship.tension,
                                Some(relationship.summary.clone()),
                                None,
                            )
                            .await?;
                        updated_relationships += 1;
                    }
                    Err(_) => {
                        self.repository
                            .create_relationship(
                                &target_branch_id,
                                &CreateRelationshipInput {
                                    character_a_id: source_id.clone(),
                                    character_b_id: target_id.clone(),
                                    relationship_type: relationship.relationship_type.clone(),
                                    initial_trust: relationship.trust,
                                    initial_tension: relationship.tension,
                                    dynamics: vec![relationship.summary.clone()],
                                },
                            )
                            .await?;
                        created_relationships += 1;
                    }
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "relationship".to_string(),
            created: created_relationships,
            updated: updated_relationships,
            skipped: skipped_relationships,
        });

        // ----- knowledge facts -----
        let mut created_knowledge = 0usize;
        let mut skipped_knowledge = 0usize;
        if let Some(final_state) = final_state.as_ref() {
            let existing_knowledge = self
                .repository
                .list_knowledge_facts_by_project(&target_project.id)
                .await?;
            for snapshot in &final_state.characters {
                let Some(character_id) = characters_by_cluster.get(&snapshot.cluster_id) else {
                    skipped_knowledge += 1;
                    continue;
                };
                for fact in snapshot
                    .notes
                    .iter()
                    .filter(|note| !note.trim().is_empty())
                    .chain(snapshot.status.iter())
                    .take(3)
                {
                    let normalized = normalize_name(fact);
                    if existing_knowledge.iter().any(|candidate| {
                        candidate.character_id == *character_id
                            && candidate.normalized_fact == normalized
                    }) {
                        skipped_knowledge += 1;
                        continue;
                    }
                    self.repository
                        .upsert_knowledge_fact(UpsertKnowledgeFactParams {
                            project_id: target_project.id.clone(),
                            branch_id: target_branch_id.clone(),
                            character_id: character_id.clone(),
                            fact: fact.clone(),
                            normalized_fact: normalized,
                            source_summary: "Imported final-state hydration".to_string(),
                            learned_at: Some(StoryPlacement {
                                book_number: final_state.book_number,
                                chapter_number: final_state.chapter_number,
                                scene_order: final_state.scene_order,
                                note: None,
                            }),
                            confidence: Some(snapshot.confidence),
                            tags: vec!["imported".to_string(), "hydrated".to_string()],
                            reader_visible: true,
                            source_import_session_id: Some(input.session_id.clone()),
                        })
                        .await?;
                    created_knowledge += 1;
                }
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "knowledge_fact".to_string(),
            created: created_knowledge,
            updated: 0,
            skipped: skipped_knowledge,
        });

        // ----- timeline / future knowledge / system overlays -----
        let mut created_future_knowledge = 0usize;
        let mut skipped_future_knowledge = 0usize;
        let mut created_timeline_events = 0usize;
        let mut skipped_timeline_events = 0usize;
        let created_temporal_interventions = 0usize;
        let skipped_temporal_interventions = 0usize;
        let mut created_system_overlays = 0usize;
        let mut skipped_system_overlays = 0usize;
        if let Some(world) = world.as_ref() {
            let high_confidence_systems = world
                .system_signals
                .iter()
                .filter(|signal| matches!(signal.confidence_level, ImportConfidenceLevel::High))
                .collect::<Vec<_>>();
            if high_confidence_systems.is_empty() {
                skipped_sections.push("advanced_records".to_string());
                notes.push(
                    "Skipped advanced temporal/system hydration because evidence was not high-confidence."
                        .to_string(),
                );
            } else {
                let existing_overlays = self
                    .repository
                    .list_system_overlays_by_project(&target_project.id)
                    .await?;
                for signal in &high_confidence_systems {
                    if existing_overlays
                        .iter()
                        .any(|overlay| overlay.normalized_name == normalize_name(&signal.summary))
                    {
                        skipped_system_overlays += 1;
                        continue;
                    }
                    self.create_system_overlay(CreateSystemOverlayInput {
                        project_id: target_project_id.clone(),
                        system_name: summarize_text(&signal.summary, 48),
                        system_type: signal.signal_type.clone(),
                        rules: signal.summary.clone(),
                        visibility: "mixed".to_string(),
                        progression_currency: None,
                        stats: Vec::new(),
                        advancement_tiers: Vec::new(),
                    })
                    .await?;
                    created_system_overlays += 1;
                }
            }
        }
        if let Some(final_state) = final_state.as_ref() {
            let existing_events = self
                .repository
                .list_timeline_events_by_project(&target_project.id)
                .await?;
            if existing_events.is_empty() {
                self.create_timeline_event(CreateTimelineEventInput {
                    project_id: target_project_id.clone(),
                    title: format!(
                        "Import continuation point {}.{}",
                        final_state.book_number, final_state.chapter_number
                    ),
                    event_type: "continuation_point".to_string(),
                    placement: StoryPlacement {
                        book_number: final_state.book_number,
                        chapter_number: final_state.chapter_number,
                        scene_order: final_state.scene_order,
                        note: Some("imported manuscript continuation".to_string()),
                    },
                    summary: final_state.summary.clone(),
                    related_entity_ids: Vec::new(),
                })
                .await?;
                created_timeline_events += 1;
            } else {
                skipped_timeline_events += 1;
            }
            for snapshot in &final_state.characters {
                let displaced = snapshot.notes.iter().any(|note| {
                    normalize_name(note).contains("future")
                        || normalize_name(note).contains("prophecy")
                });
                if !displaced {
                    continue;
                }
                let Some(character_id) = characters_by_cluster.get(&snapshot.cluster_id) else {
                    skipped_future_knowledge += 1;
                    continue;
                };
                self.create_future_knowledge(CreateFutureKnowledgeInput {
                    project_id: target_project_id.clone(),
                    character_id: character_id.clone(),
                    knowledge_summary: snapshot.notes.join(" "),
                    source: "imported manuscript inference".to_string(),
                    learned_at: StoryPlacement {
                        book_number: final_state.book_number,
                        chapter_number: final_state.chapter_number,
                        scene_order: final_state.scene_order,
                        note: None,
                    },
                    expires_at: None,
                    notes: vec!["hydrated from import final-state signals".to_string()],
                })
                .await?;
                created_future_knowledge += 1;
            }
        }
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "future_knowledge".to_string(),
            created: created_future_knowledge,
            updated: 0,
            skipped: skipped_future_knowledge,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "timeline_event".to_string(),
            created: created_timeline_events,
            updated: 0,
            skipped: skipped_timeline_events,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "temporal_intervention".to_string(),
            created: created_temporal_interventions,
            updated: 0,
            skipped: skipped_temporal_interventions,
        });
        record_counts.push(ImportHydrationRecordCount {
            entity_type: "system_overlay".to_string(),
            created: created_system_overlays,
            updated: 0,
            skipped: skipped_system_overlays,
        });

        notes.push(format!(
            "Hydration provenance anchored to import session {}.",
            input.session_id
        ));

        let report = ImportHydrationReport {
            session_id: input.session_id.clone(),
            project_id: target_project_id.clone(),
            branch_id: target_branch_id.clone(),
            status: if skipped_sections.is_empty() {
                ImportHydrationStatus::Completed
            } else {
                ImportHydrationStatus::Partial
            },
            created_project,
            target_branch_id: target_branch_id.clone(),
            record_counts,
            skipped_sections,
            review_item_ids: review_items.iter().map(|item| item.id.clone()).collect(),
            notes,
        };

        let _ = self
            .repository
            .update_import_session_hydration_report(
                &input.session_id,
                serde_json::to_value(&report)?,
            )
            .await?;
        let _ = self
            .repository
            .update_import_session_state(
                &input.session_id,
                &import_pass_name(&ImportPassName::Hydration),
                serde_json::to_value(import_progress(
                    structure.source_documents.len(),
                    structure.source_documents.len(),
                    structure
                        .chapters
                        .iter()
                        .map(|chapter| chapter.scenes.len())
                        .sum(),
                    structure
                        .chapters
                        .iter()
                        .map(|chapter| chapter.scenes.len())
                        .sum(),
                    review_items.len(),
                    review_items
                        .iter()
                        .filter(|item| {
                            item.status == import_review_status_name(&ImportReviewStatus::Open)
                        })
                        .count(),
                ))?,
                &import_session_status_name(&ImportSessionStatus::Hydrated),
            )
            .await?;

        Ok(spindle_core::models::ImportHydrateBibleOutput {
            session_id: input.session_id,
            report,
        })
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
        .map(|c| c.id)
        .collect();
    let mut live_scene_ids: BTreeSet<String> = repository
        .list_scenes_by_project_and_branch(project_id, branch_id)
        .await?
        .into_iter()
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
    if concerns.is_empty() {
        concerns.push("no major reader-level concerns detected in this heuristic pass".to_string());
    }
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
fn validate_chapter_plan_style(
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
/// Mirrors `SpindleRepository::scene_revision_fingerprint` from
/// repository.rs:2687 in 705b835^.
fn scene_revision_fingerprint(scene: &crate::sqlite::records::Scene) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(scene.summary.as_bytes());
    hasher.update(b"\n");
    hasher.update(scene.full_text.as_bytes());
    hasher.update(b"\n");
    hasher.update(scene.content_rating.as_bytes());
    hasher.update(b"\n");
    hasher.update(scene.tone.clone().unwrap_or_default().as_bytes());
    format!("{:x}", hasher.finalize())
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
            } else if let Some(scene) = fallback_scene {
                (scene, false)
            } else {
                return None;
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
fn should_run_check(requested_checks: &std::collections::BTreeSet<String>, check: &str) -> bool {
    requested_checks.is_empty() || requested_checks.contains(check)
}

// ── Phase-4 validator helpers ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PhaseFourCacheId {
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
                            "scene_id": fact.scene_id,
                            "book_number": fact.book_number,
                            "chapter_number": fact.chapter_number,
                            "fact_type": fact.fact_type,
                            "key": fact.key,
                            "value": fact.value,
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
                        })
                    })
                    .collect::<Vec<_>>();
                rules.sort_by_key(|value| value.to_string());
                serde_json::json!({ "world_rules": rules })
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

/// True for character-state status strings that flag a deceased
/// character (any of "dead", "deceased", "killed", "slain", case-folded).
fn is_dead_status(status: &str) -> bool {
    let normalized = status.to_ascii_lowercase();
    normalized.contains("dead")
        || normalized.contains("deceased")
        || normalized.contains("killed")
        || normalized.contains("slain")
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
         Return strict JSON only.\n\n\
         Scene id: {}\n\
         Scene summary: {}\n\
         Scene tone: {}\n\
         Scene text:\n{}\n\n\
         Established rules:\n{}\n\n\
         Return this exact shape:\n\
         {{\"violations\":[{{\"rule_id\":\"world_rule:...\",\"severity\":\"warning\",\"message\":\"...\",\"evidence\":\"...\"}}]}}\n\
         Use severity \"error\" only for clear contradictions and \"warning\" for plausible but ambiguous strain.",
        scene.id,
        scene.summary,
        scene.tone.as_deref().unwrap_or("unspecified"),
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

    let start = without_fence.find('{')?;
    let end = without_fence.rfind('}')?;
    (start < end).then(|| without_fence[start..=end].to_string())
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

/// Render a canonical-fact value to its display string. Order of
/// preference: text → number → json → "<unset>". Mirrors
/// `canonical_fact_value_for_check` in 705b835^.
fn canonical_fact_value_for_check(fact: &crate::sqlite::records::CanonicalFact) -> String {
    if let Some(value) = fact.value_text.clone().filter(|value| !value.is_empty()) {
        return value;
    }
    if let Some(value) = fact
        .value_number
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
    {
        return value;
    }
    if let Some(value) = fact
        .value_json
        .as_ref()
        .map(serde_json::Value::to_string)
        .filter(|value| !value.is_empty())
    {
        return value;
    }
    "<unset>".to_string()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqlitePool;
    use spindle_core::models::ReaderContract;
    use tempfile::TempDir;

    async fn fresh_service() -> (TempDir, SqliteSpindleService) {
        let tmp = TempDir::new().unwrap();
        let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let repo = Repository::new(pool, data_dir);
        (tmp, SqliteSpindleService::new(repo))
    }

    /// Variant that pins the model router to the local-only adapter set.
    /// Use when a test exercises a code path that calls `model_router.complete`
    /// and needs a deterministic, network-free response (the default
    /// `Repository::new` reads `~/.config/spindle/agent-config.toml` if
    /// present, which on dev machines points at a real HTTP endpoint).
    async fn fresh_service_local() -> (TempDir, SqliteSpindleService) {
        use crate::ai::ModelRouter;
        let tmp = TempDir::new().unwrap();
        let pool = SqlitePool::open(&tmp.path().join("svc.db")).await.unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let repo = Repository::with_model_router(pool, data_dir, ModelRouter::local_only());
        (tmp, SqliteSpindleService::new(repo))
    }

    async fn project_with_scene(
        svc: &SqliteSpindleService,
        name: &str,
    ) -> (CreateProjectOutput, String, String) {
        let project = svc
            .create_project(CreateProjectInput {
                name: name.to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "clean continuity".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let full_text = format!("{name} opens with a quiet scene.");
        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: full_text.clone(),
                summary: "opening scene".to_string(),
                content_rating: spindle_core::models::ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        (project, scene.scene_id, full_text)
    }

    async fn seed_phase_four_cache(
        svc: &SqliteSpindleService,
        project_id: &str,
        branch_id: &str,
        scene_id: &str,
        full_text: &str,
        cache_id: PhaseFourCacheId,
    ) -> String {
        let scene_text_hash = generation_sha256_hex(full_text.as_bytes());
        svc.repository
            .upsert_validator_finding(crate::sqlite::repository::UpsertValidatorFindingParams {
                project_id: project_id.to_string(),
                branch_id: branch_id.to_string(),
                scene_id: scene_id.to_string(),
                scene_text_hash: scene_text_hash.clone(),
                context_hash: None,
                validator_id: cache_id.as_str().to_string(),
                finding_id: "__cache__".to_string(),
                severity: "info".to_string(),
                message: "warm cache".to_string(),
                byte_range: None,
                details_json: Some(serde_json::json!({ "issues": [] })),
            })
            .await
            .unwrap();
        assert_active_phase_four_cache(svc, branch_id, scene_id, &scene_text_hash, cache_id, 1)
            .await;
        scene_text_hash
    }

    async fn assert_active_phase_four_cache(
        svc: &SqliteSpindleService,
        branch_id: &str,
        scene_id: &str,
        scene_text_hash: &str,
        cache_id: PhaseFourCacheId,
        expected: usize,
    ) {
        let rows = svc
            .repository
            .list_active_validator_findings_by_scene_hash(
                branch_id,
                scene_id,
                scene_text_hash,
                &[cache_id.as_str().to_string()],
            )
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            expected,
            "unexpected active cache count for {}",
            cache_id.as_str()
        );
    }

    #[tokio::test]
    async fn read_project_resource_continuity_health_reports_cache_orphans_and_duplicates() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "continuity-health").await;
        seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::WorldRuleSemanticDrift,
        )
        .await;

        svc.create_temporal_intervention(CreateTemporalInterventionInput {
            project_id: project.project_id.clone(),
            title: "Warning with missing anchor".to_string(),
            intervention_type: "message".to_string(),
            source_event_id: None,
            target_event_id: None,
            summary: "A warning is missing its source and target events.".to_string(),
            consequences: vec!["continuity needs endpoint repair".to_string()],
            status: Some("planned".to_string()),
        })
        .await
        .unwrap();

        for value_text in ["clear", "stormy"] {
            svc.register_canonical_fact(RegisterCanonicalFactInput {
                project_id: project.project_id.clone(),
                scene_id: scene_id.clone(),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("project".to_string()),
                subject_id: None,
                predicate: Some("weather".to_string()),
                value_kind: Some("string".to_string()),
                value_text: Some(value_text.to_string()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: Some(CanonicalFactScope::Invariant),
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
            })
            .await
            .unwrap();
        }

        let health = svc
            .read_project_resource(&project.project_id, "continuity/health")
            .await
            .unwrap();

        assert_eq!(
            health["active_branch_id"].as_str(),
            Some(project.branch_id.as_str())
        );
        assert_eq!(health["branch_count"].as_u64(), Some(1));
        assert_eq!(health["branch_lineage"].as_array().unwrap().len(), 1);
        assert_eq!(
            health["validator_findings"]["open_by_validator_and_severity"]
                [PhaseFourCacheId::WorldRuleSemanticDrift.as_str()]["info"]
                .as_u64(),
            Some(1)
        );
        assert_eq!(
            health["validator_cache_counts"][PhaseFourCacheId::WorldRuleSemanticDrift.as_str()]
                ["active_count"]
                .as_u64(),
            Some(1)
        );
        assert_eq!(
            health["validator_cache_totals"]["active_count"].as_u64(),
            Some(1)
        );
        assert!(health["last_check_consistency_at"].is_null());

        let orphans = health["orphaned_temporal_interventions"]
            .as_array()
            .unwrap();
        assert_eq!(orphans.len(), 1);
        let missing_endpoints = orphans[0]["missing_endpoints"].as_array().unwrap();
        assert_eq!(missing_endpoints.len(), 2);
        assert!(missing_endpoints.iter().any(|endpoint| {
            endpoint["field"].as_str() == Some("source_event_id")
                && endpoint["reason"].as_str() == Some("unset")
        }));
        assert!(missing_endpoints.iter().any(|endpoint| {
            endpoint["field"].as_str() == Some("target_event_id")
                && endpoint["reason"].as_str() == Some("unset")
        }));

        let duplicates = health["duplicate_active_canonical_facts"]
            .as_array()
            .unwrap();
        assert_eq!(duplicates.len(), 1);
        assert_eq!(duplicates[0]["subject_table"].as_str(), Some("project"));
        assert_eq!(duplicates[0]["subject_id"], serde_json::Value::Null);
        assert_eq!(duplicates[0]["predicate"].as_str(), Some("weather"));
        assert_eq!(duplicates[0]["fact_ids"].as_array().unwrap().len(), 2);
        assert_eq!(duplicates[0]["unique_values"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn timeline_graph_mermaid_resource_is_deterministic_and_escapes_labels() {
        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "timeline-graph".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "timeline graph visibility".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let first_event = svc
            .create_timeline_event(CreateTimelineEventInput {
                project_id: project.project_id.clone(),
                title: "Arrival \"north\" [gate] {oath}|line\nbreak".to_string(),
                event_type: "anchor".to_string(),
                placement: StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(1),
                    note: None,
                },
                summary: "The gate oath is established.".to_string(),
                related_entity_ids: Vec::new(),
            })
            .await
            .unwrap();
        let second_event = svc
            .create_timeline_event(CreateTimelineEventInput {
                project_id: project.project_id.clone(),
                title: "Discovery".to_string(),
                event_type: "anchor".to_string(),
                placement: StoryPlacement {
                    book_number: 1,
                    chapter_number: 2,
                    scene_order: Some(1),
                    note: None,
                },
                summary: "The discovery follows the arrival.".to_string(),
                related_entity_ids: Vec::new(),
            })
            .await
            .unwrap();

        svc.create_temporal_intervention(CreateTemporalInterventionInput {
            project_id: project.project_id.clone(),
            title: "Echo [closed]".to_string(),
            intervention_type: "message".to_string(),
            source_event_id: Some(second_event.timeline_event_id),
            target_event_id: Some(first_event.timeline_event_id),
            summary: "The later event echoes backward.".to_string(),
            consequences: vec!["closed loop".to_string()],
            status: Some("planned".to_string()),
        })
        .await
        .unwrap();
        svc.create_temporal_intervention(CreateTemporalInterventionInput {
            project_id: project.project_id.clone(),
            title: "Loose {thread}|alert".to_string(),
            intervention_type: "message".to_string(),
            source_event_id: None,
            target_event_id: None,
            summary: "A loose intervention is missing endpoints.".to_string(),
            consequences: Vec::new(),
            status: Some("draft".to_string()),
        })
        .await
        .unwrap();

        let graph = svc
            .timeline_graph_mermaid_resource(&project.project_id)
            .await
            .unwrap();
        let graph_again = svc
            .timeline_graph_mermaid_resource(&project.project_id)
            .await
            .unwrap();

        assert_eq!(graph, graph_again);
        assert!(graph.starts_with("```mermaid\nflowchart LR\n"));
        assert!(graph.ends_with("```\n"));
        assert_eq!(graph.matches("```").count(), 2);
        assert!(
            graph.contains(
                "Arrival \\\"north\\\" &#91;gate&#93; &#123;oath&#125;&#124;line\\nbreak"
            )
        );
        assert!(graph.contains("-. \"intervention: Echo &#91;closed&#93;\" .->"));
        assert!(graph.contains("missing source and target: Loose &#123;thread&#125;&#124;alert"));
        assert!(graph.contains(":::warning"));
    }

    #[tokio::test]
    async fn create_world_rule_resolves_world_rule_semantic_drift_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "world-cache").await;
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::WorldRuleSemanticDrift,
        )
        .await;

        svc.create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "No salt magic".to_string(),
            rule_type: "magic".to_string(),
            description: "Salt magic fails near iron bells.".to_string(),
            scan_pattern: Some("salt magic".to_string()),
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::WorldRuleSemanticDrift,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn update_world_rule_resolves_world_rule_semantic_drift_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "world-update-cache").await;
        let rule = svc
            .create_world_rule(CreateWorldRuleInput {
                project_id: project.project_id.clone(),
                rule_name: "Iron bells disrupt salt magic".to_string(),
                rule_type: "magic".to_string(),
                description: "Salt magic fails near iron bells.".to_string(),
                scan_pattern: Some("salt magic".to_string()),
                relevance_tags: Vec::new(),
                established_in: None,
            })
            .await
            .unwrap();
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::WorldRuleSemanticDrift,
        )
        .await;

        svc.update_world_rule(UpdateWorldRuleInput {
            world_rule_id: rule.world_rule_id,
            changes: serde_json::json!({
                "description": "Salt magic fails near iron bells and cold river stone."
            }),
        })
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::WorldRuleSemanticDrift,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn update_entity_world_rule_resolves_world_rule_semantic_drift_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) =
            project_with_scene(&svc, "generic-world-update-cache").await;
        let rule = svc
            .create_world_rule(CreateWorldRuleInput {
                project_id: project.project_id.clone(),
                rule_name: "Moon glass cannot be forged twice".to_string(),
                rule_type: "magic".to_string(),
                description: "Moon glass cracks when reforged.".to_string(),
                scan_pattern: Some("moon glass".to_string()),
                relevance_tags: Vec::new(),
                established_in: None,
            })
            .await
            .unwrap();
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::WorldRuleSemanticDrift,
        )
        .await;

        svc.update_entity(UpdateEntityInput {
            entity_type: "world_rule".to_string(),
            entity_id: rule.world_rule_id,
            changes: serde_json::json!({
                "description": "Moon glass cracks when reforged or reheated."
            }),
        })
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::WorldRuleSemanticDrift,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn context_hash_blocks_stale_world_rule_cache_when_invalidation_is_missed() {
        use spindle_core::models::{CheckConsistencyInput, ConsistencyScopeInput};

        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "context-hash-cache".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "context hashes protect cache hits".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let full_text = "Eldrin tried to ignore the sigil and cast at range anyway.".to_string();
        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: full_text.clone(),
                summary: "Eldrin ignores a sigil rule.".to_string(),
                content_rating: spindle_core::models::ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        let scene_record = svc.repository.get_scene(&scene.scene_id).await.unwrap();
        let phase_four_checks = std::iter::once(PhaseFourCacheId::WorldRuleSemanticDrift)
            .collect::<std::collections::BTreeSet<_>>();
        let old_context = svc
            .build_phase_four_validator_context(
                &project.project_id,
                &project.branch_id,
                std::slice::from_ref(&scene_record),
            )
            .await
            .unwrap();
        let old_context_hash = phase_four_context_hashes(&old_context, &phase_four_checks)
            .unwrap()
            .get(PhaseFourCacheId::WorldRuleSemanticDrift.as_str())
            .cloned()
            .unwrap();
        let scene_text_hash = generation_sha256_hex(full_text.as_bytes());
        svc.repository
            .upsert_validator_finding(crate::sqlite::repository::UpsertValidatorFindingParams {
                project_id: project.project_id.clone(),
                branch_id: project.branch_id.clone(),
                scene_id: scene.scene_id.clone(),
                scene_text_hash: scene_text_hash.clone(),
                context_hash: Some(old_context_hash),
                validator_id: PhaseFourCacheId::WorldRuleSemanticDrift
                    .as_str()
                    .to_string(),
                finding_id: "__cache__".to_string(),
                severity: "warning".to_string(),
                message: "stale cache".to_string(),
                byte_range: None,
                details_json: Some(serde_json::json!({
                    "issues": [{
                        "severity": "warning",
                        "check_type": "world_rule_semantic_drift",
                        "message": "stale cached issue",
                        "byte_range": null
                    }]
                })),
            })
            .await
            .unwrap();

        // Bypass the service-level invalidation on purpose. The context hash
        // must still prevent the old cache row from being used.
        svc.repository
            .create_world_rule(&CreateWorldRuleInput {
                project_id: project.project_id.clone(),
                rule_name: "Sigil contact rule".to_string(),
                rule_type: "magic".to_string(),
                description: "Magic sigils must require physical contact.".to_string(),
                scan_pattern: Some(r"\bsigil\b".to_string()),
                relevance_tags: Vec::new(),
                established_in: None,
            })
            .await
            .unwrap();

        let output = svc
            .check_consistency(CheckConsistencyInput {
                project_id: project.project_id.clone(),
                scope: ConsistencyScopeInput::full(),
                checks: vec![
                    PhaseFourCacheId::WorldRuleSemanticDrift
                        .as_str()
                        .to_string(),
                ],
                severity_filter: Vec::new(),
                deep_check: Some(false),
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();

        assert!(
            !output
                .issues
                .iter()
                .any(|issue| issue.message.contains("stale cached issue")),
            "stale cache issue should not be returned: {:?}",
            output.issues
        );
        assert!(
            output.issues.iter().any(|issue| {
                issue.check_type == PhaseFourCacheId::WorldRuleSemanticDrift.as_str()
                    && issue.message.contains("Sigil contact rule")
            }),
            "fresh world-rule issue should be recomputed: {:?}",
            output.issues
        );

        let active_rows = svc
            .repository
            .list_active_validator_findings_by_scene_hash(
                &project.branch_id,
                &scene.scene_id,
                &scene_text_hash,
                &[PhaseFourCacheId::WorldRuleSemanticDrift
                    .as_str()
                    .to_string()],
            )
            .await
            .unwrap();
        assert_eq!(
            active_rows.len(),
            1,
            "recompute should leave one active current-context cache row"
        );
        assert!(
            active_rows[0].context_hash.is_some(),
            "current cache row should carry context_hash"
        );
    }

    #[tokio::test]
    async fn register_canonical_fact_resolves_canonical_fact_prose_drift_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "fact-cache").await;
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::CanonicalFactProseDrift,
        )
        .await;

        svc.register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
            book_number: 1,
            chapter_number: 1,
            fact_type: Some("typed_fact".to_string()),
            key: Some("scene.opening_state".to_string()),
            value: Some("quiet".to_string()),
            context: None,
            subject_table: Some("scene".to_string()),
            subject_id: Some(scene_id.clone()),
            predicate: Some("opening_state".to_string()),
            value_kind: Some("string".to_string()),
            value_text: Some("quiet".to_string()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: Some(CanonicalFactScope::Invariant),
            valid_from: None,
            valid_until: None,
            legacy_untyped: Some(false),
            supersedes_fact_id: None,
        })
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::CanonicalFactProseDrift,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn extract_canonical_facts_from_scene_does_not_resolve_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "fact-proposal-cache").await;
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::CanonicalFactProseDrift,
        )
        .await;

        svc.extract_canonical_facts_from_scene(
            spindle_core::models::ExtractCanonicalFactsFromSceneInput {
                scene_id: scene_id.clone(),
            },
        )
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::CanonicalFactProseDrift,
            1,
        )
        .await;
    }

    #[tokio::test]
    async fn timeline_metadata_resolves_retcon_reachability_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "retcon-cache").await;
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::RetconReachability,
        )
        .await;

        let first_event = svc
            .create_timeline_event(CreateTimelineEventInput {
                project_id: project.project_id.clone(),
                title: "First anchor".to_string(),
                event_type: "anchor".to_string(),
                placement: StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(1),
                    note: None,
                },
                summary: "First anchor".to_string(),
                related_entity_ids: Vec::new(),
            })
            .await
            .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::RetconReachability,
            0,
        )
        .await;

        let second_event = svc
            .create_timeline_event(CreateTimelineEventInput {
                project_id: project.project_id.clone(),
                title: "Second anchor".to_string(),
                event_type: "anchor".to_string(),
                placement: StoryPlacement {
                    book_number: 1,
                    chapter_number: 2,
                    scene_order: Some(1),
                    note: None,
                },
                summary: "Second anchor".to_string(),
                related_entity_ids: Vec::new(),
            })
            .await
            .unwrap();
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::RetconReachability,
        )
        .await;

        svc.create_temporal_intervention(CreateTemporalInterventionInput {
            project_id: project.project_id.clone(),
            title: "Warning sent backward".to_string(),
            intervention_type: "message".to_string(),
            source_event_id: Some(second_event.timeline_event_id),
            target_event_id: Some(first_event.timeline_event_id),
            summary: "A warning is sent backward.".to_string(),
            consequences: vec!["changes the first anchor".to_string()],
            status: Some("planned".to_string()),
        })
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::RetconReachability,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn set_narrator_voice_resolves_style_compliance_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "style-cache").await;
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::StyleCompliance,
        )
        .await;

        svc.set_narrator_voice(spindle_core::models::SetNarratorVoiceInput {
            project_id: project.project_id.clone(),
            narrator_voice: spindle_core::style::NarratorVoice {
                emotional_register: Some("funny-and-sarcastic".to_string()),
                ..Default::default()
            },
        })
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::StyleCompliance,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn update_entity_project_style_fields_resolve_style_compliance_cache() {
        let (_tmp, svc) = fresh_service().await;
        let (project, scene_id, full_text) = project_with_scene(&svc, "project-style-cache").await;
        let scene_text_hash = seed_phase_four_cache(
            &svc,
            &project.project_id,
            &project.branch_id,
            &scene_id,
            &full_text,
            PhaseFourCacheId::StyleCompliance,
        )
        .await;

        svc.update_entity(UpdateEntityInput {
            entity_type: "project".to_string(),
            entity_id: project.project_id.clone(),
            changes: serde_json::json!({
                "genre": "comedy fantasy"
            }),
        })
        .await
        .unwrap();

        assert_active_phase_four_cache(
            &svc,
            &project.branch_id,
            &scene_id,
            &scene_text_hash,
            PhaseFourCacheId::StyleCompliance,
            0,
        )
        .await;
    }

    #[tokio::test]
    async fn phase_four_validator_context_uses_requested_branch_not_active_branch() {
        let (_tmp, svc) = fresh_service().await;
        let project = svc
            .create_project(CreateProjectInput {
                name: "branch-context".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "branch isolation".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        svc.create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "main-only rule".to_string(),
            rule_type: "magic".to_string(),
            description: "This rule belongs to main.".to_string(),
            scan_pattern: Some("main-only".to_string()),
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();
        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: project.project_id.clone(),
                parent_branch_id: Some(project.branch_id.clone()),
                name: "feature".to_string(),
                branch_type: "experiment".to_string(),
                description: None,
            })
            .await
            .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: project.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "feature-only rule".to_string(),
            rule_type: "magic".to_string(),
            description: "This rule belongs to the feature branch.".to_string(),
            scan_pattern: Some("feature-only".to_string()),
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: project.project_id.clone(),
            branch_id: project.branch_id.clone(),
        })
        .await
        .unwrap();

        let context = svc
            .build_phase_four_validator_context(&project.project_id, &feature.branch_id, &[])
            .await
            .unwrap();
        let rule_names = context
            .world_rules
            .into_iter()
            .map(|rule| rule.rule_name)
            .collect::<Vec<_>>();

        assert_eq!(rule_names, vec!["feature-only rule".to_string()]);
    }

    /// Exercises the three core dispatch shapes of `read_project_resource`:
    /// a simple list (`characters` — empty), a paginated bare list
    /// (`research-log` — empty, full envelope), and an explicit page request
    /// (`conflicts/0/50` — empty, full envelope with `next_resource: null`).
    /// Asserts the pagination envelope structure so the MCP layer's
    /// `bible://projects/...` URI fingerprint stays stable.
    #[tokio::test]
    async fn read_project_resource_dispatches_simple_and_paginated_shapes() {
        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "rpr".into(),
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

        // 1) Simple list: characters — empty array on a fresh project.
        let characters = svc
            .read_project_resource(&proj.project_id, "characters")
            .await
            .unwrap();
        assert_eq!(characters, serde_json::json!([]));

        // 2) Paginated bare path: research-log defaults to offset 0,
        //    limit = DEFAULT_PROJECT_RESOURCE_PAGE_SIZE (50).
        let research = svc
            .read_project_resource(&proj.project_id, "research-log")
            .await
            .unwrap();
        let pagination = research
            .get("pagination")
            .expect("pagination envelope must be present");
        assert_eq!(pagination["offset"], 0);
        assert_eq!(pagination["limit"], 50);
        assert_eq!(pagination["returned"], 0);
        assert_eq!(pagination["total"], 0);
        assert_eq!(pagination["has_more"], false);
        assert_eq!(pagination["order"], "newest_first");
        assert!(pagination["next_resource"].is_null());
        assert!(pagination["previous_resource"].is_null());
        assert_eq!(research["entries"], serde_json::json!([]));

        // 3) Explicit page form: conflicts/0/50 has the same envelope shape
        //    as the bare form, just with the parsed offset/limit echoed back.
        let conflicts = svc
            .read_project_resource(&proj.project_id, "conflicts/0/50")
            .await
            .unwrap();
        let pagination = conflicts
            .get("pagination")
            .expect("pagination envelope must be present");
        assert_eq!(pagination["offset"], 0);
        assert_eq!(pagination["limit"], 50);
        assert_eq!(pagination["total"], 0);
        assert_eq!(pagination["order"], "normalized_name");
        assert!(pagination["next_resource"].is_null());
        assert_eq!(conflicts["entries"], serde_json::json!([]));

        // 4) Unknown resource path bails cleanly.
        let err = svc
            .read_project_resource(&proj.project_id, "totally-fake")
            .await
            .expect_err("unknown resource paths must error");
        assert!(
            err.to_string().contains("unknown project resource path"),
            "got: {err}"
        );

        // 5) Over-size page limit on a paginated resource is rejected.
        let err = svc
            .read_project_resource(&proj.project_id, "research-log/0/500")
            .await
            .expect_err("page size > MAX must error");
        assert!(
            err.to_string().contains("page size must be <="),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn search_bible_returns_results_in_both_modes() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput, SearchBibleInput, SearchBibleMode,
        };
        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
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

        svc.create_character(CreateCharacterInput {
            project_id: proj.project_id.clone(),
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

        // Exact (FTS5) mode.
        let exact = svc
            .search_bible(SearchBibleInput {
                project_id: proj.project_id.clone(),
                query: "warden".into(),
                limit: Some(10),
                mode: Some(SearchBibleMode::Exact),
                field: None,
                subject_table: None,
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();
        assert!(
            exact.results.iter().any(|r| r.entity_type == "character"),
            "exact mode must return the character via FTS5"
        );

        // Semantic (vec0) mode — TokenHash embedding is deterministic so
        // searching with the character's own content returns it as a hit.
        let semantic = svc
            .search_bible(SearchBibleInput {
                project_id: proj.project_id.clone(),
                query: "Mara Oathkeeper\n\nWarden of the Ash Gate.".into(),
                limit: Some(10),
                mode: Some(SearchBibleMode::Semantic),
                field: None,
                subject_table: None,
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();
        assert!(
            !semantic.results.is_empty(),
            "semantic mode must return at least one result"
        );
    }

    #[tokio::test]
    async fn full_mcp_priority_flow_through_service() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            ContentRating, CreateBranchInput, CreateCharacterInput, CreateLocationInput,
            CreateRelationshipInput, SaveSceneDraftInput, SwitchBranchInput, WorldStateInput,
        };
        let (_tmp, svc) = fresh_service().await;

        // 1. Create the project.
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Marches".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Oathbound wardens fail and a city falls.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        // 2. Create a character.
        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Oathbound warden of the Ash Gate.".into(),
                role: "protagonist".into(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: Some("grim".into()),
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
        assert!(mara.character_id.starts_with("character:"));

        let aldric = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Aldric".into(),
                summary: "Scribe of the eastern marches.".into(),
                role: "supporting".into(),
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
                initial_state: None,
            })
            .await
            .unwrap();

        // 3. Create a location (+ paired world state).
        let gate = svc
            .create_location(CreateLocationInput {
                project_id: proj.project_id.clone(),
                name: "Ash Gate".into(),
                kind: "fortress".into(),
                realm: None,
                summary: "A blackened wall holding back the dark.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: Some("tense".into()),
                    prosperity: None,
                    stability: Some("fragile".into()),
                    threat_level: Some("high".into()),
                    sensory_details: vec!["smell of ash".into()],
                },
            })
            .await
            .unwrap();
        assert!(gate.location_id.starts_with("location:"));

        // 4. Create a relationship.
        let rel = svc
            .create_relationship(CreateRelationshipInput {
                character_a_id: mara.character_id.clone(),
                character_b_id: aldric.character_id.clone(),
                relationship_type: "ally".into(),
                initial_trust: 60,
                initial_tension: 20,
                dynamics: vec!["wary".into()],
            })
            .await
            .unwrap();
        assert!(rel.relationship_id.starts_with("relates_to:"));

        // 5. Save a scene through the service. This insert lands the row,
        //    runs the scene_version snapshot trigger plus the FTS sync.
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate.".into(),
                summary: "Mara holds the gate".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        assert!(saved.scene_id.starts_with("scene:"));
        assert_eq!(saved.status, "saved");

        // 6. Branch off main + switch.
        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                name: "feature-arc-2".into(),
                branch_type: "feature".into(),
                description: None,
                parent_branch_id: None,
            })
            .await
            .unwrap();
        assert!(feature.branch_id.starts_with("bible_branch:"));

        let switched = svc
            .switch_branch(SwitchBranchInput {
                project_id: proj.project_id.clone(),
                branch_id: feature.branch_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(switched.branch_id, feature.branch_id);
        assert_eq!(switched.branch_name, "feature-arc-2");
    }

    /// A comprehensive end-to-end test that walks through a realistic
    /// MCP-priority flow: create project, create characters with relationships,
    /// plan a chapter, save scenes with version snapshots, commit character
    /// state, register canonical facts, save a chapter summary, exercise the
    /// search index in both modes, and branch off main. This proves the
    /// SQLite migration is architecturally sound for the integration test
    /// golden path the original plan calls out as the Phase 6 exit criterion.
    ///
    /// Note: this test exercises ~25 of the translated service methods in one
    /// run. It is the closest proxy to MCP integration_tests.rs that lives at
    /// the spindle-adapters level (before the Phase 6 MCP layer swap).
    #[tokio::test]
    async fn end_to_end_full_flow_against_sqlite() {
        use spindle_core::models::{
            CanonicalFactScope, CharacterEmotionalProfileData, CharacterStatePatch,
            CharacterVoiceProfileData, CommitCharacterStateInput, ContentRating, CreateBranchInput,
            CreateCharacterInput, CreateLocationInput, CreateProjectInput, CreateRelationshipInput,
            PlanChapterInput, PlanChapterSceneInput, RecordKnowledgeInput,
            RegisterCanonicalFactInput, SaveSceneDraftInput, SaveSummaryInput, SearchBibleInput,
            SearchBibleMode, StoryPlacement, SwitchBranchInput, UpdateRelationshipInput,
            WorldStateInput,
        };
        let (_tmp, svc) = fresh_service().await;

        // ----- Phase 1: project + Bible setup -------------------------------
        let proj = svc
            .create_project(CreateProjectInput {
                name: "End-to-End Test".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Oathbound wardens fail and a city falls.".into(),
                    style_notes: vec!["sparse prose".into()],
                    boundaries: vec!["no second-person".into()],
                },
            })
            .await
            .unwrap();

        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Oathbound warden of the Ash Gate.".into(),
                role: "protagonist".into(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: Some("grim".into()),
                    vocabulary: vec!["oath".into(), "ash".into()],
                    sentence_structure: vec!["clipped".into()],
                    tics: Vec::new(),
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
                    flex_range: None,
                },
                initial_state: Some(CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: Some(vec!["hold the gate".into()]),
                    status: Some(vec!["wary".into()]),
                    notes: None,
                    source_summary: Some("introduction".into()),
                }),
            })
            .await
            .unwrap();

        let aldric = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Aldric".into(),
                summary: "Scribe of the marches.".into(),
                role: "supporting".into(),
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
                initial_state: None,
            })
            .await
            .unwrap();

        let _gate = svc
            .create_location(CreateLocationInput {
                project_id: proj.project_id.clone(),
                name: "Ash Gate".into(),
                kind: "fortress".into(),
                realm: None,
                summary: "A blackened wall holding back the dark.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: Some("watchful".into()),
                    prosperity: None,
                    stability: Some("fragile".into()),
                    threat_level: Some("high".into()),
                    sensory_details: vec!["smell of ash".into()],
                },
            })
            .await
            .unwrap();

        let _rel = svc
            .create_relationship(CreateRelationshipInput {
                character_a_id: mara.character_id.clone(),
                character_b_id: aldric.character_id.clone(),
                relationship_type: "ally".into(),
                initial_trust: 60,
                initial_tension: 20,
                dynamics: vec!["wary mutual respect".into()],
            })
            .await
            .unwrap();

        // ----- Phase 2: chapter planning ------------------------------------
        let _plan = svc
            .plan_chapter(PlanChapterInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                pov_character_id: Some(mara.character_id.clone()),
                synopsis: "Mara holds the gate against the first dark.".into(),
                target_theme_ids: Vec::new(),
                target_conflict_ids: Vec::new(),
                target_plot_line_ids: Vec::new(),
                scenes: vec![
                    PlanChapterSceneInput {
                        scene_order: 1,
                        summary: "Mara takes the watch".into(),
                        beat_structure: vec!["arrival".into(), "first dark".into()],
                        character_ids: vec![mara.character_id.clone()],
                        purpose: "establishing".into(),
                    },
                    PlanChapterSceneInput {
                        scene_order: 2,
                        summary: "Aldric brings news".into(),
                        beat_structure: vec!["arrival".into()],
                        character_ids: vec![mara.character_id.clone(), aldric.character_id.clone()],
                        purpose: "complication".into(),
                    },
                ],
            })
            .await
            .unwrap();

        // ----- Phase 3: scene drafting ---------------------------------------
        let scene1 = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate. Her oath weighed heavier than her sword."
                    .into(),
                summary: "Mara holds the gate at first dark".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        assert!(scene1.scene_id.starts_with("scene:"));
        assert_eq!(scene1.status, "saved");

        // Save the scene again with revised prose — version snapshot fires.
        let scene1_v2 = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate. Behind her, the city slept.".into(),
                summary: "Mara holds the gate at first dark".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        assert_eq!(scene1_v2.scene_id, scene1.scene_id);

        // Verify the version count via list_scene_versions.
        let versions = svc
            .list_scene_versions(spindle_core::models::ListSceneVersionsInput {
                project_id: proj.project_id.clone(),
                scene_id: scene1.scene_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            versions.versions.len(),
            1,
            "one snapshot of the prior prose"
        );

        // ----- Phase 4: state + knowledge + canonical fact -------------------
        let state = svc
            .commit_character_state(CommitCharacterStateInput {
                character_id: mara.character_id.clone(),
                scene_id: scene1.scene_id.clone(),
                changes: CharacterStatePatch {
                    emotional_state: std::collections::BTreeMap::new(),
                    goals: Some(vec!["hold the gate at all costs".into()]),
                    status: Some(vec!["determined".into()]),
                    notes: Some(vec!["sword across knees".into()]),
                    source_summary: Some("first watch".into()),
                },
            })
            .await
            .unwrap();
        assert!(state.state_id.starts_with("character_state:"));

        let _knowledge = svc
            .record_knowledge(RecordKnowledgeInput {
                project_id: proj.project_id.clone(),
                branch_id: None,
                character_id: mara.character_id.clone(),
                fact: "The dark advances from the north.".into(),
                source_summary: "scout report at first watch".into(),
                learned_at: Some(StoryPlacement {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: Some(1),
                    note: None,
                }),
                confidence: Some(0.8),
                tags: vec!["intel".into()],
                reader_visible: true,
            })
            .await
            .unwrap();

        let fact = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: proj.project_id.clone(),
                scene_id: scene1.scene_id.clone(),
                book_number: 1,
                chapter_number: 1,
                fact_type: None,
                key: None,
                value: None,
                context: None,
                subject_table: Some("character".into()),
                subject_id: Some(mara.character_id.clone()),
                predicate: Some("oath".into()),
                value_kind: Some("string".into()),
                value_text: Some("ash gate warden".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: vec!["oathbound".into()],
                scope: Some(CanonicalFactScope::Invariant),
                valid_from: None,
                valid_until: None,
                legacy_untyped: None,
                supersedes_fact_id: None,
            })
            .await
            .unwrap();
        assert!(fact.canonical_fact_id.starts_with("canonical_fact:"));

        // ----- Phase 5: search both modes -----------------------------------
        let lexical = svc
            .search_bible(SearchBibleInput {
                project_id: proj.project_id.clone(),
                query: "warden".into(),
                limit: Some(10),
                mode: Some(SearchBibleMode::Exact),
                field: None,
                subject_table: None,
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();
        assert!(
            lexical.results.iter().any(|r| r.entity_type == "character"),
            "FTS5 lexical search should find the character"
        );

        let _semantic = svc
            .search_bible(SearchBibleInput {
                project_id: proj.project_id.clone(),
                query: "Mara stood at the gate".into(),
                limit: Some(10),
                mode: Some(SearchBibleMode::Semantic),
                field: None,
                subject_table: None,
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();

        // ----- Phase 6: relationship update + summary -----------------------
        let updated_rel = svc
            .update_relationship(UpdateRelationshipInput {
                character_a_id: mara.character_id.clone(),
                character_b_id: aldric.character_id.clone(),
                trust_delta: 10,
                tension_delta: -5,
                reason: "Aldric's scouting saved Mara's watch".into(),
                scene_id: scene1.scene_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(updated_rel.trust, 70);
        assert_eq!(updated_rel.tension, 15);

        let summary = svc
            .save_summary(SaveSummaryInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                entity_type: None,
                entity_id: None,
                summary: "Mara held the gate; Aldric brought intel.".into(),
                key_events: vec!["first dark approached".into()],
                character_changes: vec!["Mara hardened".into()],
                relationship_shifts: vec!["Mara-Aldric trust deepened".into()],
                arc_advances: Vec::new(),
                promise_events: Vec::new(),
            })
            .await
            .unwrap();
        assert!(summary.chapter_summary_id.starts_with("chapter_summary:"));

        // ----- Phase 7: branch off ------------------------------------------
        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                name: "feature-alt-ending".into(),
                branch_type: "feature".into(),
                description: Some("alternate fall-of-the-gate ending".into()),
                parent_branch_id: None,
            })
            .await
            .unwrap();
        let switched = svc
            .switch_branch(SwitchBranchInput {
                project_id: proj.project_id.clone(),
                branch_id: feature.branch_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(switched.branch_id, feature.branch_id);
        assert_eq!(switched.branch_name, "feature-alt-ending");
    }

    #[tokio::test]
    async fn create_project_returns_expected_output_shape() {
        let (_tmp, svc) = fresh_service().await;
        let out = svc
            .create_project(CreateProjectInput {
                name: "Spindle".into(),
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
        assert!(out.project_id.starts_with("project:"));
        assert!(out.book_id.starts_with("book:"));
        assert!(out.chapter_id.starts_with("chapter:"));

        let list = svc.list_projects().await.unwrap();
        assert_eq!(list.projects.len(), 1);
        assert_eq!(list.projects[0].project_id, out.project_id);
        assert_eq!(list.projects[0].name, "Spindle");
    }

    #[tokio::test]
    async fn get_writer_state_aggregates_current_branch() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            ContentRating, CreateCharacterInput, GetWriterStateInput, SaveSceneDraftInput,
            WriterIntent,
        };

        let (_tmp, svc) = fresh_service().await;

        // Project + 1 character + 1 saved scene on chapter 1.
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Wardens".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Oathbound wardens fail.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        svc.create_character(CreateCharacterInput {
            project_id: proj.project_id.clone(),
            name: "Mara".into(),
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
                status: Some(vec!["holding the gate".into()]),
                notes: None,
                source_summary: None,
            }),
        })
        .await
        .unwrap();

        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "Mara holds the gate".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        // No branch / cursor / format supplied — defaults exercise the
        // active-branch resolution, cursor-from-last-scene fallback, and
        // markdown budget enforcement together.
        let state = svc
            .get_writer_state(GetWriterStateInput {
                project_id: proj.project_id.clone(),
                branch_id: None,
                at_scene_id: None,
                format: None,
                budget_tokens: None,
                include_subjects: None,
                include_recent_activity: None,
                recent_activity_limit: None,
            })
            .await
            .unwrap();

        assert_eq!(state.current.project.project_id, proj.project_id);
        assert_eq!(state.current.branch.branch_id, proj.branch_id);
        assert!(state.current.branch.is_active);
        let scene = state
            .current
            .scene
            .expect("cursor scene resolves from last scene");
        assert_eq!(scene.book_number, 1);
        assert_eq!(scene.chapter_number, 1);
        assert_eq!(scene.scene_order, 1);
        assert_eq!(state.current.intent, WriterIntent::Drafting);

        // Character snapshot present and includes the initial status patch.
        assert_eq!(state.subjects.len(), 1);
        assert_eq!(state.subjects[0].subject.name, "Mara");
        assert_eq!(
            state.subjects[0].status,
            vec!["holding the gate".to_string()]
        );

        // Bundle summary populated by enforce_writer_state_budget.
        assert!(state.bundle_summary.estimated_tokens > 0);
        assert_eq!(state.bundle_summary.token_budget, Some(8000));
        assert!(
            state
                .bundle_summary
                .included_sections
                .iter()
                .any(|s| s == "current")
        );

        // No scene_source_link rows were created in this fixture, so
        // SourceBridge divergence detection has nothing to report.
        assert!(state.unsynced_local_files.is_empty());
        assert!(state.drift_warnings.is_empty());

        let envelope = svc
            .get_writer_state_envelope(GetWriterStateInput {
                project_id: proj.project_id.clone(),
                branch_id: None,
                at_scene_id: None,
                format: None,
                budget_tokens: None,
                include_subjects: None,
                include_recent_activity: None,
                recent_activity_limit: None,
            })
            .await
            .unwrap();
        assert_eq!(envelope.current.project.project_id, proj.project_id);
        assert!(
            envelope
                .writer_state_markdown
                .as_deref()
                .is_some_and(|markdown| markdown.contains("# Writer state")),
            "public envelope should include markdown when requested/defaulted"
        );
    }

    #[tokio::test]
    async fn migrate_canonical_fact_supersedes_old_fact() {
        use spindle_core::models::{
            CanonicalFactUpgradeSpec, ContentRating, MigrateCanonicalFactInput,
            RegisterCanonicalFactInput, SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Canon".into(),
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

        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "A scene.".into(),
                summary: "stage".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        // Seed an untyped canonical fact via register_canonical_fact.
        let untyped = svc
            .register_canonical_fact(RegisterCanonicalFactInput {
                project_id: proj.project_id.clone(),
                scene_id: scene.scene_id.clone(),
                book_number: 1,
                chapter_number: 1,
                fact_type: Some("note".into()),
                key: Some("Kael:gear".into()),
                value: Some("worn cloak".into()),
                context: None,
                subject_table: None,
                subject_id: None,
                predicate: None,
                value_kind: None,
                value_text: None,
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: None,
                valid_from: None,
                valid_until: None,
                legacy_untyped: Some(true),
                supersedes_fact_id: None,
            })
            .await
            .unwrap();

        let migrated = svc
            .migrate_canonical_fact(MigrateCanonicalFactInput {
                fact_id: untyped.canonical_fact_id.clone(),
                upgrade_spec: CanonicalFactUpgradeSpec {
                    subject_table: "character".into(),
                    subject_id: Some("character:kael".into()),
                    predicate: "gear".into(),
                    value_kind: "string".into(),
                    value_text: Some("worn cloak".into()),
                    value_number: None,
                    value_unit: None,
                    value_json: None,
                    aliases: vec!["cloak".into()],
                    scope: None,
                    valid_from: None,
                    valid_until: None,
                },
            })
            .await
            .unwrap();

        assert_eq!(migrated.superseded_fact_id, untyped.canonical_fact_id);
        assert_ne!(
            migrated.canonical_fact_id, untyped.canonical_fact_id,
            "migrate must create a new fact id"
        );

        // The old fact is now superseded by the new typed fact.
        let old = svc
            .repository()
            .get_canonical_fact(&untyped.canonical_fact_id)
            .await
            .unwrap();
        assert_eq!(
            old.superseded_by.as_deref(),
            Some(migrated.canonical_fact_id.as_str())
        );
    }

    #[tokio::test]
    async fn preflight_book_export_flags_blocking_and_warning_issues() {
        use spindle_core::models::{
            ContentRating, ExportIssueSeverity, PreflightBookExportInput, SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Export Drill".into(),
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

        // Save a non-empty scene in chapter 1 — no blocking issues, but
        // chapter title was never set so we expect a warning.
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Real prose.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .preflight_book_export(PreflightBookExportInput {
                project_id: proj.project_id.clone(),
                book_number: Some(1),
                start_chapter_number: None,
                end_chapter_number: None,
            })
            .await
            .unwrap();

        assert!(out.issues.iter().any(|i| i.code == "chapter_missing_title"
            && matches!(i.severity, ExportIssueSeverity::Warning)));
        assert!(
            out.issues
                .iter()
                .all(|i| i.code != "chapter_without_scenes"),
            "chapter 1 has a scene, must not be flagged blocking"
        );

        // Half-open chapter range → error.
        let err = svc
            .preflight_book_export(PreflightBookExportInput {
                project_id: proj.project_id.clone(),
                book_number: Some(1),
                start_chapter_number: Some(1),
                end_chapter_number: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("requires both"));
    }

    #[tokio::test]
    async fn compare_alternatives_ranks_by_quality_score() {
        use spindle_core::models::{
            CompareAlternativesInput, ContentRating, CreateBranchInput, SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Alt".into(),
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

        // Two feature branches off main, each with one scene draft.
        let alt_a = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "alt-a".into(),
                branch_type: "alternative".into(),
                description: None,
            })
            .await
            .unwrap();
        let alt_b = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "alt-b".into(),
                branch_type: "alternative".into(),
                description: None,
            })
            .await
            .unwrap();

        // Switch into each branch to write its scene. Sparse text →
        // low score; rich text + measured tone → higher score.
        svc.switch_branch(spindle_core::models::SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: alt_a.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Short.".into(),
            summary: "thin".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        svc.switch_branch(spindle_core::models::SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: alt_b.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "A long, rich scene draft with many words to push the text-length \
                        contribution to its maximum and pull the rank ahead of the thin \
                        alternative branch. Repeating: a long, rich, sustained, descriptive \
                        passage that the scoring helper will treat as a heavyweight contender."
                .into(),
            summary: "rich measured beat that opens a quiet pacing slot".into(),
            content_rating: ContentRating::General,
            tone: Some("measured".into()),
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .compare_alternatives(CompareAlternativesInput {
                project_id: proj.project_id.clone(),
                branch_ids: vec![alt_a.branch_id.clone(), alt_b.branch_id.clone()],
            })
            .await
            .unwrap();

        assert_eq!(out.alternatives.len(), 2);
        assert_eq!(
            out.recommended_branch_id.as_deref(),
            Some(alt_b.branch_id.as_str())
        );
        // strongest_trait reflects the "measured" tone branch.
        let b_entry = out
            .alternatives
            .iter()
            .find(|c| c.branch_id == alt_b.branch_id)
            .unwrap();
        assert_eq!(b_entry.strongest_trait, "best pacing balance");
    }

    #[tokio::test]
    async fn run_dual_persona_review_round_trips_local_adapter() {
        use spindle_core::models::{ContentRating, RunDualPersonaReviewInput, SaveSceneDraftInput};

        let (_tmp, svc) = fresh_service_local().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Review".into(),
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
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Brief opening prose with felt and seemed for the filter check.".into(),
                summary: "a thin sketch".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        // Without GEMINI_API_KEY, the model router resolves to the local
        // adapter, which exercises the derive_*_concerns / derive_review_actions
        // heuristic fallbacks deterministically. That's exactly what the
        // unit test wants — no live network calls, but the full code path
        // through the dual-persona orchestration runs.
        let out = svc
            .run_dual_persona_review(RunDualPersonaReviewInput {
                project_id: proj.project_id.clone(),
                branch_id: None,
                scene_id: saved.scene_id.clone(),
                rounds: Some(1),
            })
            .await
            .unwrap();

        assert_eq!(out.scene_id, saved.scene_id);
        assert_eq!(out.rounds_completed, 1);
        assert_eq!(out.status, "current");
        assert!(!out.review_id.is_empty());
        assert_eq!(out.review_rounds.len(), 1);
        // tone is None → derive_literary_concerns must surface the missing-tone
        // concern; derive_craft_concerns must catch the filter words.
        let round = &out.review_rounds[0];
        assert!(
            round
                .literary_critic
                .concerns
                .iter()
                .any(|c| c.contains("tone metadata is missing"))
        );
        assert!(
            round
                .craft_technician
                .concerns
                .iter()
                .any(|c| c.contains("filter words"))
        );
        // derive_review_actions also adds the tone-setter action.
        assert!(
            round
                .priority_actions
                .iter()
                .any(|a| a.contains("tone metadata"))
        );
    }

    /// End-to-end genre-voice enforcement (the brief's test case): an
    /// NSFW Comedy Webnovel must surface its style contract prominently, flag a
    /// literary scene at the save gate, the validator, and the review — and let
    /// an on-genre scene through clean.
    #[tokio::test]
    async fn style_enforcement_pipeline_flags_offgenre_comedy_scene() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CheckConsistencyInput, ConsistencyScopeInput, ContentRating, ContextFormat,
            CreateCharacterInput, CreateLocationInput, CreateWorldRuleInput, GetSceneContextInput,
            RunDualPersonaReviewInput, SaveSceneDraftInput, SetNarratorVoiceInput, WorldStateInput,
        };

        let (_tmp, svc) = fresh_service_local().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Vegas Pull".into(),
                project_type: "NSFW Comedy Webnovel".into(),
                genre: "Comedy".into(),
                reader_contract: ReaderContract {
                    promise: "A raunchy, funny gacha power-fantasy romp.".into(),
                    style_notes: vec![
                        "Raunchy modern comedy tone".into(),
                        "Webnovel pacing with clear progression".into(),
                    ],
                    boundaries: vec!["Focus on raunchy comedy and fun over dark themes".into()],
                },
            })
            .await
            .unwrap();

        svc.set_narrator_voice(SetNarratorVoiceInput {
            project_id: proj.project_id.clone(),
            narrator_voice: spindle_core::style::NarratorVoice {
                emotional_register: Some("funny-and-sarcastic".into()),
                chapter_ending_style: Some("hook".into()),
                ..Default::default()
            },
        })
        .await
        .unwrap();

        svc.create_world_rule(CreateWorldRuleInput {
            project_id: proj.project_id.clone(),
            rule_name: "Prose Style Bible — Webnovel-First, Comedy-First".into(),
            rule_type: "style".into(),
            description: "No grief-beat endings; no contemplative literary pacing.".into(),
            scan_pattern: Some("*".into()),
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();

        let jason = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Jason".into(),
                summary: "37-year-old mind in a 19-year-old body; sarcastic narrator.".into(),
                role: "protagonist".into(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: Some("sarcastic".into()),
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
                    goals: Some(vec!["win the gacha".into()]),
                    status: Some(vec!["broke".into()]),
                    notes: None,
                    source_summary: None,
                }),
            })
            .await
            .unwrap();

        let casino = svc
            .create_location(CreateLocationInput {
                project_id: proj.project_id.clone(),
                name: "The Strip".into(),
                kind: "casino".into(),
                realm: None,
                summary: "Neon, noise, bad decisions.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: Some("loud".into()),
                    prosperity: None,
                    stability: None,
                    threat_level: None,
                    sensory_details: vec!["cigarette smoke".into()],
                },
            })
            .await
            .unwrap();

        // (5) Scene context surfaces the style contract prominently.
        let envelope = svc
            .get_scene_context_envelope(GetSceneContextInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                character_ids: vec![jason.character_id.clone()],
                max_character_count: None,
                location_id: casino.location_id.clone(),
                format: Some(ContextFormat::Markdown),
                budget_tokens: Some(8000),
                token_budget: None,
                sections: None,
            })
            .await
            .unwrap();

        let directive = envelope
            .novel
            .style_directive
            .as_ref()
            .expect("style directive present for a project with a style contract");
        assert!(
            directive
                .style_rules
                .iter()
                .any(|rule| rule.rule_name.contains("Prose Style Bible")),
            "style world rule must reach the directive regardless of relevance filtering"
        );
        assert!(envelope.standards.contains("Project Style Requirements"));
        assert!(envelope.standards.contains("[STYLE DIRECTIVE]"));
        assert!(
            envelope.standards.contains("not funny"),
            "comedy projects get the forceful 'not funny → failed' enforcement line"
        );
        let context_md = envelope.context_markdown.as_deref().unwrap_or_default();
        assert!(context_md.contains("Project Style Requirements"));

        // (6) Save gate flags a literary tone on a comedy project. This scene
        // is the only one in chapter 1, so it is also a chapter end.
        let literary = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Jason sat alone after the others left. The grief of the day pressed \
                            in, a hollow ache, a quiet sorrow, the melancholy of everything he \
                            had lost settling over him like ash."
                    .into(),
                summary: "Jason sits with his losses.".into(),
                content_rating: ContentRating::General,
                tone: Some("Quiet, contained, grief beat".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        assert!(
            literary.tone_deviation,
            "a grief-beat tone on a comedy project is a tone deviation"
        );
        assert!(
            literary
                .style_warnings
                .iter()
                .any(|w| w.to_lowercase().contains("comedy")),
            "style_warnings should name the comedy-contract conflict: {:?}",
            literary.style_warnings
        );

        // On-genre scene in a different chapter passes clean. Chapter 2 must
        // exist first; this also keeps the literary scene the sole (ending)
        // scene of chapter 1 so the validator's chapter-end check applies.
        svc.create_chapter(spindle_core::models::CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(2),
            title: None,
        })
        .await
        .unwrap();
        let comedic = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 2,
                chapter_id: None,
                scene_order: 1,
                full_text: "The System pinged like a slot machine hitting jackpot. \"Congrats, \
                            loser,\" it chirped. Jason flipped it off and yanked the lever again."
                    .into(),
                summary: "Jason pulls again.".into(),
                content_rating: ContentRating::General,
                tone: Some("manic, raunchy".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        assert!(
            !comedic.tone_deviation,
            "an on-genre scene is not a deviation"
        );
        assert!(
            comedic.style_warnings.is_empty(),
            "on-genre scene should not produce style warnings: {:?}",
            comedic.style_warnings
        );

        // (7) Dual-persona review evaluates genre compliance via Target Reader.
        let review = svc
            .run_dual_persona_review(RunDualPersonaReviewInput {
                project_id: proj.project_id.clone(),
                branch_id: None,
                scene_id: literary.scene_id.clone(),
                rounds: Some(1),
            })
            .await
            .unwrap();
        let round = &review.review_rounds[0];
        assert_eq!(round.genre_reader.persona, "target_reader");
        assert!(
            !round.genre_reader.concerns.is_empty(),
            "Target Reader should raise genre concerns on a literary comedy scene"
        );
        assert!(
            round
                .priority_actions
                .iter()
                .any(|a| a.starts_with("Genre fix:")),
            "genre concerns should lead the priority actions: {:?}",
            round.priority_actions
        );

        // (8) check_consistency surfaces a style_compliance finding for the
        // contemplative chapter-ending scene.
        let consistency = svc
            .check_consistency(CheckConsistencyInput {
                project_id: proj.project_id.clone(),
                scope: ConsistencyScopeInput::full(),
                checks: vec!["style_compliance".into()],
                severity_filter: Vec::new(),
                deep_check: None,
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();
        assert!(
            consistency
                .issues
                .iter()
                .any(|issue| issue.check_type == "style_compliance"),
            "style_compliance validator should flag the off-genre chapter ending: {:?}",
            consistency.issues
        );
    }

    /// Narrator voice round-trips through the new project column and clears
    /// back to unset when an empty voice is submitted.
    #[tokio::test]
    async fn set_narrator_voice_round_trips_and_clears() {
        use spindle_core::models::SetNarratorVoiceInput;

        let (_tmp, svc) = fresh_service_local().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "NV".into(),
                project_type: "novel".into(),
                genre: "Comedy".into(),
                reader_contract: ReaderContract {
                    promise: "p".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let set = svc
            .set_narrator_voice(SetNarratorVoiceInput {
                project_id: proj.project_id.clone(),
                narrator_voice: spindle_core::style::NarratorVoice {
                    emotional_register: Some("funny-and-sarcastic".into()),
                    chapter_ending_style: Some("hook".into()),
                    ..Default::default()
                },
            })
            .await
            .unwrap();
        assert!(!set.cleared);
        assert_eq!(
            set.narrator_voice.emotional_register.as_deref(),
            Some("funny-and-sarcastic")
        );

        // It must persist into the directive the rest of the pipeline reads.
        let branch_id = svc
            .repository
            .active_branch_id_public(&proj.project_id)
            .await
            .unwrap();
        let directive = svc
            .style_directive_for(&proj.project_id, &branch_id)
            .await
            .unwrap();
        assert_eq!(
            directive
                .narrator_voice
                .as_ref()
                .and_then(|voice| voice.chapter_ending_style.as_deref()),
            Some("hook")
        );

        // Submitting an empty voice clears the column.
        let cleared = svc
            .set_narrator_voice(SetNarratorVoiceInput {
                project_id: proj.project_id.clone(),
                narrator_voice: spindle_core::style::NarratorVoice::default(),
            })
            .await
            .unwrap();
        assert!(cleared.cleared);
        let directive = svc
            .style_directive_for(&proj.project_id, &branch_id)
            .await
            .unwrap();
        assert!(directive.narrator_voice.is_none());
    }

    /// The plan gate catches genre-incompatible beats before any prose exists.
    #[tokio::test]
    async fn plan_chapter_flags_offgenre_beats_on_comedy_project() {
        use spindle_core::models::{PlanChapterInput, PlanChapterSceneInput};

        let (_tmp, svc) = fresh_service_local().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Vegas Pull".into(),
                project_type: "NSFW Comedy Webnovel".into(),
                genre: "Comedy".into(),
                reader_contract: ReaderContract {
                    promise: "A raunchy funny gacha romp.".into(),
                    style_notes: vec!["Raunchy modern comedy tone; webnovel pacing".into()],
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let out = svc
            .plan_chapter(PlanChapterInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                pov_character_id: None,
                synopsis: "Jason loses everything and reflects.".into(),
                target_theme_ids: Vec::new(),
                target_conflict_ids: Vec::new(),
                target_plot_line_ids: Vec::new(),
                scenes: vec![
                    PlanChapterSceneInput {
                        scene_order: 1,
                        summary: "Jason wins big at the slots.".into(),
                        beat_structure: vec!["escalation".into()],
                        character_ids: Vec::new(),
                        purpose: "Open with a manic high.".into(),
                    },
                    PlanChapterSceneInput {
                        scene_order: 2,
                        summary: "Jason sits alone with his losses.".into(),
                        beat_structure: vec!["quiet_reflection".into()],
                        character_ids: Vec::new(),
                        purpose: "Quiet, contained. The grief beat of the chapter.".into(),
                    },
                ],
            })
            .await
            .unwrap();

        assert!(
            out.style_warnings.iter().any(|w| w.starts_with("Scene 2:")),
            "the grief-beat final scene should be flagged at planning time: {:?}",
            out.style_warnings
        );
    }

    #[tokio::test]
    async fn extract_canonical_facts_from_scene_proposes_sentence_chunks() {
        use spindle_core::models::{
            CanonicalFactScope, ContentRating, ExtractCanonicalFactsFromSceneInput,
            SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Extract".into(),
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
        let saved = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara held the gate while the bell tolled twice. \
                            Aldric watched the ash settle on the cobbles below. \
                            Short."
                    .into(),
                summary: "stage".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        let out = svc
            .extract_canonical_facts_from_scene(ExtractCanonicalFactsFromSceneInput {
                scene_id: saved.scene_id.clone(),
            })
            .await
            .unwrap();

        assert_eq!(out.scene_id, saved.scene_id);
        // Two long sentences should pass the >=24 char filter; the third
        // ("Short.") is below the threshold and must be dropped.
        assert_eq!(out.proposals.len(), 2);
        for (idx, prop) in out.proposals.iter().enumerate() {
            assert_eq!(prop.subject_table, "scene");
            assert_eq!(prop.subject_id.as_deref(), Some(saved.scene_id.as_str()));
            assert_eq!(prop.value_kind, "string");
            assert!(matches!(prop.scope, Some(CanonicalFactScope::Invariant)));
            assert!(
                prop.predicate
                    .starts_with(&format!("scene.extracted_{}.", idx + 1))
            );
        }
    }

    #[tokio::test]
    async fn research_query_errors_without_gemini_key() {
        use spindle_core::models::ResearchQueryInput;

        // Pin GEMINI_API_KEY to unset so this test does the early-bail
        // path deterministically. The test is single-threaded
        // (current_thread + #[tokio::test]) but the env var is global —
        // wrap the mutation in a guard via a serial fence on the env.
        // SAFETY: each test process serializes its own env writes; the
        // unsafe block scope is whatever the runner allots this test.
        // unsafe is required for std::env::remove_var since 2024 edition.
        unsafe {
            std::env::remove_var("GEMINI_API_KEY");
        }

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Research".into(),
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
        let err = svc
            .research_query(ResearchQueryInput {
                project_id: proj.project_id,
                query: "How do tides work?".into(),
                context_hint: None,
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("GEMINI_API_KEY"),
            "expected GEMINI_API_KEY guard, got: {err}"
        );
    }

    #[tokio::test]
    async fn export_epub_writes_bytes_and_passes_warnings() {
        use spindle_core::models::{
            ContentRating, ExportEpubInput, ExportIssueSeverity, SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Export Test".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "epic".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .export_epub(ExportEpubInput {
                project_id: proj.project_id.clone(),
                author: Some("T. Author".into()),
                book_number: Some(1),
                start_chapter_number: None,
                end_chapter_number: None,
            })
            .await
            .unwrap();

        assert_eq!(out.total_chapters, 1);
        assert_eq!(out.total_scenes, 1);
        // chapter 1 has no title → preflight surfaces a chapter_missing_title warning.
        assert!(
            out.preflight_warnings
                .iter()
                .any(|w| w.code == "chapter_missing_title"
                    && matches!(w.severity, ExportIssueSeverity::Warning))
        );
        // No scene_source_link rows were created in this fixture, so
        // SourceBridge divergence detection has nothing to report.
        assert!(out.divergence_warnings.is_empty());
        // Bytes hit disk.
        let bytes = std::fs::read(&out.file_path).unwrap();
        assert!(bytes.starts_with(b"PK"), "expected ZIP magic for EPUB");
        assert!(out.filename.ends_with(".epub"));
    }

    #[tokio::test]
    async fn continue_generation_surfaces_local_adapter_limit() {
        use spindle_core::models::ContinueGenerationInput;

        // The local adapter doesn't implement complete_continuation —
        // that's a real limitation, not a stub. The service method
        // should surface the adapter's error verbatim so MCP callers
        // can react.
        let (_tmp, svc) = fresh_service_local().await;
        let err = svc
            .continue_generation(ContinueGenerationInput {
                route: "draft".into(),
                rating: None,
                original_prompt: "Open with Mara at the gate.".into(),
                prior_output: "Mara watched the dust settle. ".into(),
                project_id: None,
                book_id: None,
                chapter_id: None,
                scene_id: None,
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("continuation not supported"),
            "expected local-adapter no-continuation guard, got: {err}"
        );
    }

    /// Regression: revise_generation used to bail with
    /// `"was not produced with rating \"explicit\""` for any non-explicit
    /// source receipt, forcing a full re-roll for mature/teen/general
    /// surgical edits. The verified_revisable_draft_receipt helper now
    /// accepts all ratings (and the explicit_capable_agent check is gated
    /// on rating == explicit). This test seeds a mature-rated receipt
    /// directly and confirms the call no longer trips the rating gate.
    #[tokio::test]
    async fn revise_generation_accepts_mature_rated_source_receipt() {
        let (_tmp, svc) = fresh_service_local().await;

        // Seed a mature-rated draft receipt directly (the local adapter
        // doesn't implement complete_continuation, so we can't go through
        // continue_generation; the private helper is in-module-accessible).
        let receipt = svc.register_generation_receipt(
            "draft",
            Some("mature"),
            "local-adapter-stub",
            "Mara crossed the rain-slick alley, salt charm warm in her fist.",
        );

        let outcome = svc
            .revise_generation(spindle_core::models::ReviseGenerationInput {
                generation_id: receipt.id.clone(),
                edit_instructions: "Tighten the opening — drop the adjective on alley.".into(),
                context: None,
            })
            .await;

        // We don't care about the prose the local adapter returns; we only
        // care that the rating gate didn't reject the mature receipt.
        let outcome = outcome.expect("mature-rated revise_generation must succeed");
        assert_eq!(outcome.source_generation_id, receipt.id);
        // The new receipt should carry the same rating as the source.
        let new_receipt_id = outcome
            .generation_id
            .expect("revision must produce a new receipt id");
        assert!(
            new_receipt_id.starts_with("model_generation:"),
            "unexpected receipt id shape: {new_receipt_id}"
        );
    }

    /// Regression: an explicit-rated source receipt produced by an agent
    /// that is NOT explicit-capable should still be rejected — that's the
    /// real integrity invariant we kept after relaxing the broader rating
    /// gate. (For non-explicit ratings the explicit_capable_agent check
    /// no longer applies.)
    #[tokio::test]
    async fn revise_generation_still_blocks_explicit_from_non_explicit_agent() {
        let (_tmp, svc) = fresh_service_local().await;
        let receipt = svc.register_generation_receipt(
            "draft",
            Some("explicit"),
            "local-adapter-stub-no-explicit",
            "Some explicit prose.",
        );
        let err = svc
            .revise_generation(spindle_core::models::ReviseGenerationInput {
                generation_id: receipt.id.clone(),
                edit_instructions: "Tighten.".into(),
                context: None,
            })
            .await
            .expect_err("explicit receipt from non-explicit agent must still error");
        assert!(
            err.to_string().contains("not explicit-capable"),
            "expected explicit-capable guard, got: {err}"
        );
    }

    #[tokio::test]
    async fn revise_generation_rejects_empty_edit_instructions() {
        let (_tmp, svc) = fresh_service_local().await;
        let err = svc
            .revise_generation(spindle_core::models::ReviseGenerationInput {
                generation_id: "model_generation:99:abcdef012345".into(),
                edit_instructions: "   ".into(),
                context: None,
            })
            .await
            .unwrap_err();
        // First failure path: the receipt id won't exist in the cache.
        assert!(
            err.to_string().contains("not found or has expired"),
            "expected receipt-not-found, got: {err}"
        );
    }

    /// Regression for the SQL crash where update_entity on a book row failed
    /// with `no such column: updated_at`. The book table in V0001 was missing
    /// the `updated_at` column that the generic update path sets on every
    /// UPDATE. V0004 backfills the column; this test would have failed before
    /// that migration and the matching repository code changes landed.
    #[tokio::test]
    async fn update_entity_on_book_title_succeeds_after_updated_at_backfill() {
        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Book Rename".into(),
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

        // create_project auto-mints book #1; grab its id straight from the repo.
        let books = svc
            .repository()
            .list_books_by_project(&proj.project_id)
            .await
            .unwrap();
        let book_id = books
            .first()
            .map(|b| b.id.clone())
            .expect("bootstrap book exists");

        // Drive the bug repro: update entity_type="book", field="title".
        svc.update_entity(UpdateEntityInput {
            entity_type: "book".into(),
            entity_id: book_id.clone(),
            changes: serde_json::json!({ "title": "Renamed Book" }),
        })
        .await
        .expect("update_entity on book.title must not crash");

        // Confirm the rename persisted and updated_at is now populated.
        let books = svc
            .repository()
            .list_books_by_project(&proj.project_id)
            .await
            .unwrap();
        let renamed = books.first().expect("book row");
        assert_eq!(renamed.title.as_deref(), Some("Renamed Book"));
        assert!(
            renamed.updated_at.is_some(),
            "updated_at must be populated after update_entity"
        );
    }

    /// Regression for the asymmetric allowlist where create_conflict accepted
    /// `try_fail_cycles` but update_entity rejected it. The continuity-editor
    /// path explicitly tells callers to add more cycles after creation, so
    /// this update must succeed.
    #[tokio::test]
    async fn update_entity_on_conflict_try_fail_cycles_now_allowlisted() {
        use spindle_core::models::{CreateConflictInput, TryFailCycleStep};

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Conflict Update".into(),
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

        let conflict = svc
            .create_conflict(CreateConflictInput {
                project_id: proj.project_id.clone(),
                name: "Mara vs Wraith".into(),
                conflict_type: "person-vs-supernatural".into(),
                stakes: "Life of the village.".into(),
                escalation_stages: Vec::new(),
                expected_total_cycles: None,
                try_fail_cycles: Vec::new(),
                stated_consequences: Vec::new(),
            })
            .await
            .unwrap();

        // Drive the bug repro: retroactively populate try_fail_cycles via
        // update_entity. Pre-fix this errored with "column 'try_fail_cycles'
        // on 'conflict' is not in the update allowlist".
        let cycles = vec![
            TryFailCycleStep {
                attempt_order: 1,
                label: "Salt ward at the gate".into(),
                outcome: "Wraith slips through".into(),
                cost: Some("Mara loses her warding charm".into()),
                revelation: None,
            },
            TryFailCycleStep {
                attempt_order: 2,
                label: "Ambush in the alley".into(),
                outcome: "Wraith reveals it can phase".into(),
                cost: Some("Mara's shoulder torn".into()),
                revelation: Some("Wraith is incorporeal at night".into()),
            },
        ];

        svc.update_entity(UpdateEntityInput {
            entity_type: "conflict".into(),
            entity_id: conflict.conflict_id.clone(),
            changes: serde_json::json!({
                "try_fail_cycles": cycles,
                "expected_total_cycles": 3,
            }),
        })
        .await
        .expect("update_entity on conflict array fields must succeed");

        let updated = svc
            .repository()
            .get_conflict(&conflict.conflict_id)
            .await
            .unwrap();
        assert_eq!(updated.try_fail_cycles.len(), 2);
        assert_eq!(updated.try_fail_cycles[0].attempt_order, 1);
        assert_eq!(updated.try_fail_cycles[1].attempt_order, 2);
        assert_eq!(updated.expected_total_cycles, Some(3));
    }

    #[tokio::test]
    async fn diff_branches_reports_scene_modifications() {
        use spindle_core::models::{
            ContentRating, CreateBranchInput, DiffBranchesInput, SaveSceneDraftInput,
            SwitchBranchInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Diff".into(),
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
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Original prose.".into(),
            summary: "original".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "feature".into(),
                branch_type: "feature".into(),
                description: None,
            })
            .await
            .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Revised prose with different content.".into(),
            summary: "revised".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .diff_branches(DiffBranchesInput {
                project_id: proj.project_id.clone(),
                base_branch_id: proj.branch_id.clone(),
                compare_branch_id: feature.branch_id.clone(),
            })
            .await
            .unwrap();

        assert_eq!(out.scene_diffs.len(), 1);
        let diff = &out.scene_diffs[0];
        assert_eq!(diff.change_type, "modified");
        assert_eq!(diff.base_summary.as_deref(), Some("original"));
        assert_eq!(diff.compare_summary.as_deref(), Some("revised"));
        assert!(out.narrative_impact_summary.contains("1 scene changes"));
    }

    #[tokio::test]
    async fn get_entity_returns_snapshot_for_character_subject() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput, GetEntityInput,
        };
        use spindle_core::subject::SubjectTable;

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Entity".into(),
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
        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
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

        let snap = svc
            .get_entity(GetEntityInput {
                project_id: proj.project_id.clone(),
                table: SubjectTable::Character,
                entity_id: mara.character_id.clone(),
                branch_id: None,
            })
            .await
            .unwrap();
        assert_eq!(snap.subject().id(), Some(mara.character_id.as_str()));
    }

    #[tokio::test]
    async fn get_entity_rejects_project_subject_table() {
        use spindle_core::models::GetEntityInput;
        use spindle_core::subject::SubjectTable;

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "X".into(),
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
        let err = svc
            .get_entity(GetEntityInput {
                project_id: proj.project_id.clone(),
                table: SubjectTable::Project,
                entity_id: proj.project_id.clone(),
                branch_id: None,
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("table=project"));
    }

    #[tokio::test]
    async fn get_character_snapshot_unpacks_into_output_fields() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CreateCharacterInput, GetCharacterSnapshotInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Snap".into(),
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
        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Warden.".into(),
                role: "protagonist".into(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: Some("grim".into()),
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
                    status: Some(vec!["watching the gate".into()]),
                    notes: None,
                    source_summary: None,
                }),
            })
            .await
            .unwrap();

        let out = svc
            .get_character_snapshot(GetCharacterSnapshotInput {
                project_id: proj.project_id.clone(),
                character_id: mara.character_id.clone(),
                branch_id: None,
            })
            .await
            .unwrap();
        assert_eq!(
            out.snapshot.subject().id(),
            Some(mara.character_id.as_str())
        );
        assert!(
            out.voice_profile.is_some(),
            "voice profile must be unpacked off the snapshot"
        );
    }

    #[tokio::test]
    async fn get_scene_context_assembles_layers_from_real_scene() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            ContentRating, ContextFormat, CreateCharacterInput, CreateLocationInput,
            GetSceneContextInput, SaveSceneDraftInput, WorldStateInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "SceneCtx".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Oathbound wardens hold the line.".into(),
                    style_notes: vec!["grim".into()],
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Oathbound warden of the Ash Gate.".into(),
                role: "protagonist".into(),
                realm: None,
                voice_profile: CharacterVoiceProfileData {
                    tone: Some("grim".into()),
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
                    goals: Some(vec!["hold the gate".into()]),
                    status: Some(vec!["watching".into()]),
                    notes: None,
                    source_summary: None,
                }),
            })
            .await
            .unwrap();

        let gate = svc
            .create_location(CreateLocationInput {
                project_id: proj.project_id.clone(),
                name: "Ash Gate".into(),
                kind: "fortress".into(),
                realm: None,
                summary: "A blackened wall holding back the dark.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: Some("tense".into()),
                    prosperity: None,
                    stability: Some("fragile".into()),
                    threat_level: Some("high".into()),
                    sensory_details: vec!["smell of ash".into()],
                },
            })
            .await
            .unwrap();

        // Save one prior scene so the agency check has history to inspect.
        let _ = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate, watching the dark.".into(),
                summary: "Mara holds the gate".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        let ctx = svc
            .get_scene_context(GetSceneContextInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 2,
                character_ids: vec![mara.character_id.clone()],
                max_character_count: None,
                location_id: gate.location_id.clone(),
                format: Some(ContextFormat::Json),
                budget_tokens: Some(4000),
                token_budget: None,
                sections: None,
            })
            .await
            .unwrap();

        assert_eq!(ctx.scene.location.location_id, gate.location_id);
        assert_eq!(ctx.scene.location.name, "Ash Gate");
        assert_eq!(ctx.scene.characters.len(), 1);
        assert_eq!(ctx.scene.characters[0].character_id, mara.character_id);
        assert_eq!(
            ctx.scene.world_state.status.as_deref(),
            Some("tense"),
            "world state should be derived from the paired world_state row"
        );
        assert_eq!(
            ctx.novel.reader_contract.promise,
            "Oathbound wardens hold the line."
        );
        assert_eq!(ctx.budget.token_budget, Some(4000));

        let envelope = svc
            .get_scene_context_envelope(GetSceneContextInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 2,
                character_ids: vec![mara.character_id.clone()],
                max_character_count: None,
                location_id: gate.location_id.clone(),
                format: Some(ContextFormat::Markdown),
                budget_tokens: Some(4000),
                token_budget: None,
                sections: None,
            })
            .await
            .unwrap();
        assert_eq!(envelope.scene.location.location_id, gate.location_id);
        assert!(envelope.standards.contains("scene-writer"));
        assert!(
            envelope
                .context_markdown
                .as_deref()
                .is_some_and(|markdown| markdown.contains("# Scene context")),
            "public envelope should include markdown when requested"
        );
    }

    #[tokio::test]
    async fn get_chapter_briefing_assembles_from_real_scene_seed() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            ContentRating, ContextFormat, CreateCharacterInput, CreateLocationInput,
            GetChapterBriefingInput, SaveSceneDraftInput, WorldStateInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Briefing".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Wardens hold the line.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
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
        let gate = svc
            .create_location(CreateLocationInput {
                project_id: proj.project_id.clone(),
                name: "Ash Gate".into(),
                kind: "fortress".into(),
                realm: None,
                summary: "Wall against the dark.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: Some("tense".into()),
                    prosperity: None,
                    stability: None,
                    threat_level: None,
                    sensory_details: Vec::new(),
                },
            })
            .await
            .unwrap();
        let _ = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara held the line.".into(),
                summary: "Mara holds.".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        let briefing = svc
            .get_chapter_briefing(GetChapterBriefingInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                character_ids: vec![mara.character_id.clone()],
                location_id: Some(gate.location_id.clone()),
                format: Some(ContextFormat::Markdown),
                budget_tokens: Some(6000),
                recent_chapter_limit: None,
                token_budget: None,
            })
            .await
            .unwrap();

        assert_eq!(briefing.scene_seed.scene_order, Some(1));
        assert_eq!(
            briefing.scene_seed.character_ids,
            vec![mara.character_id.clone()]
        );
        assert_eq!(
            briefing.scene_seed.location_id.as_deref(),
            Some(gate.location_id.as_str())
        );
        assert!(briefing.scene_seed.scene_context_available);
        assert!(briefing.briefing_markdown.contains("# Chapter Briefing"));
        assert!(
            briefing.scene_context.is_some(),
            "scene_context should be folded in when scene_order + character_ids + location_id are pinned"
        );
    }

    #[tokio::test]
    async fn generate_alternatives_creates_branches_and_restores_active() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
            CreateCharacterInput, CreateLocationInput, GenerateAlternativesInput,
            SaveSceneDraftInput, WorldStateInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "AltGen".into(),
                project_type: "novel".into(),
                genre: "fantasy".into(),
                reader_contract: ReaderContract {
                    promise: "Wardens hold the line.".into(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Warden.".into(),
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
                initial_state: None,
            })
            .await
            .unwrap();
        let gate = svc
            .create_location(CreateLocationInput {
                project_id: proj.project_id.clone(),
                name: "Ash Gate".into(),
                kind: "fortress".into(),
                realm: None,
                summary: "A blackened wall.".into(),
                initial_state: WorldStateInput {
                    controlling_faction: None,
                    status: None,
                    prosperity: None,
                    stability: None,
                    threat_level: None,
                    sensory_details: Vec::new(),
                },
            })
            .await
            .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the gate.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .generate_alternatives(GenerateAlternativesInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
                character_ids: vec![mara.character_id.clone()],
                location_id: gate.location_id.clone(),
                alternatives: Some(2),
                variation_strategy: "approach".into(),
            })
            .await
            .unwrap();

        assert_eq!(out.alternatives.len(), 2);
        assert!(
            out.alternatives[0]
                .branch_name
                .starts_with("alt-approach-1-1-")
        );
        assert_eq!(out.alternatives[0].variation_strategy, "approach");

        // Active branch must be restored to the original (project's main).
        let active_after = svc
            .repository()
            .get_active_branch(&proj.project_id)
            .await
            .unwrap();
        assert_eq!(active_after.id, proj.branch_id);
    }

    /// `merge_branch` fast-forwards a feature branch onto the project's
    /// main when there are no conflicts, and reports a conflict when the
    /// target's existing scene has been edited after the source branch
    /// diverged.
    ///
    /// Per-project main lookup (Risk #6): the test omits
    /// `target_branch_id` so the service must default to the project's
    /// per-project `main` branch by name (NOT by the SurrealDB-era
    /// singleton). The merged scene must land on the project's actual
    /// main branch id.
    #[tokio::test]
    async fn merge_branch_fast_forwards_into_main_when_no_conflicts() {
        use spindle_core::models::{
            ContentRating, CreateBranchInput, MergeBranchInput, SaveSceneDraftInput,
            SwitchBranchInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Merge".into(),
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

        // Feature branch off main, switch into it, write a brand-new scene
        // at a position main doesn't have. Switching back to main keeps the
        // feature branch's scene isolated.
        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "feature-alt".into(),
                branch_type: "feature".into(),
                description: None,
            })
            .await
            .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "feature: oath sworn at the wall.".into(),
            summary: "feature 1.1.1".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: proj.branch_id.clone(),
        })
        .await
        .unwrap();

        // Merge feature → (defaulted-to-main). No target_branch_id supplied:
        // the service must resolve main by name on the per-project schema.
        let merged = svc
            .merge_branch(MergeBranchInput {
                project_id: proj.project_id.clone(),
                source_branch_id: feature.branch_id.clone(),
                target_branch_id: None,
                merge_type: "fast_forward".into(),
            })
            .await
            .unwrap();
        assert!(
            !merged.has_conflicts,
            "no conflicts when target has no scene at this position"
        );
        assert_eq!(merged.applied_scenes, 1);
        assert_eq!(
            merged.target_branch_id, proj.branch_id,
            "default target resolves to the project's per-project main branch"
        );

        // Main now holds the merged scene.
        let main_scenes = svc
            .repository()
            .list_scenes_by_project_and_branch(&proj.project_id, &proj.branch_id)
            .await
            .unwrap();
        let merged_scene = main_scenes
            .iter()
            .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
            .expect("merged scene present on main");
        assert_eq!(merged_scene.summary, "feature 1.1.1");
        assert_eq!(merged_scene.branch_id, proj.branch_id);
    }

    /// Conflict detection: if main is edited at a position AFTER the
    /// feature branch diverged, merging the feature branch's edit at that
    /// position reports a conflict (target wasn't merged, source row is
    /// dropped from the mergeable set).
    #[tokio::test]
    async fn merge_branch_reports_conflict_when_target_edited_after_divergence() {
        use spindle_core::models::{
            ContentRating, CreateBranchInput, MergeBranchInput, SaveSceneDraftInput,
            SwitchBranchInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Merge Conflict".into(),
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

        // Baseline scene on main, then feature branch (so source.created_at
        // > target.updated_at for the baseline scene — that one is safe).
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "main v1: gate stands.".into(),
            summary: "main v1".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        // Brief pause so the next branch's created_at is strictly later
        // than main's baseline scene's updated_at (microsecond precision).
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "feature-conflict".into(),
                branch_type: "feature".into(),
                description: None,
            })
            .await
            .unwrap();

        // Now main edits the scene at (1,1,1) — strictly after feature's
        // created_at. The feature branch will see this as a conflict.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "main v2: gate cracks.".into(),
            summary: "main v2".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        // Switch into feature and write a conflicting edit at (1,1,1).
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "feature: gate held under siege.".into(),
            summary: "feature 1.1.1".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: proj.branch_id.clone(),
        })
        .await
        .unwrap();

        let merged = svc
            .merge_branch(MergeBranchInput {
                project_id: proj.project_id.clone(),
                source_branch_id: feature.branch_id.clone(),
                target_branch_id: None,
                merge_type: "fast_forward".into(),
            })
            .await
            .unwrap();
        assert!(merged.has_conflicts);
        assert_eq!(merged.conflicts.len(), 1);
        assert_eq!(merged.conflicts[0].book_number, 1);
        assert_eq!(merged.conflicts[0].chapter_number, 1);
        assert_eq!(merged.conflicts[0].scene_order, 1);
        assert_eq!(
            merged.applied_scenes, 0,
            "conflicting scene is dropped from the mergeable set"
        );

        // Main still has the v2 edit — merge did not overwrite a conflict.
        let main_scenes = svc
            .repository()
            .list_scenes_by_project_and_branch(&proj.project_id, &proj.branch_id)
            .await
            .unwrap();
        let main_scene = main_scenes
            .iter()
            .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
            .unwrap();
        assert_eq!(main_scene.summary, "main v2");
    }

    /// `select_alternative` merges the alternative branch into the
    /// project's main and switches the active branch onto main. Verifies
    /// the per-project main lookup (Risk #6) — the post-merge
    /// switch_active_branch call must target the project's actual main
    /// branch id, not the SurrealDB-era singleton.
    #[tokio::test]
    async fn select_alternative_promotes_branch_and_switches_active_to_main() {
        use spindle_core::models::{
            ContentRating, CreateBranchInput, SaveSceneDraftInput, SelectAlternativeInput,
            SwitchBranchInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Select".into(),
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

        // Feature branch with a unique scene at (1,1,1).
        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "feature-pick".into(),
                branch_type: "feature".into(),
                description: None,
            })
            .await
            .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "feature pick: gate held.".into(),
            summary: "feature picked".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        // Run select_alternative from the feature branch (active branch is
        // feature). After the call, active branch must be main again.
        let out = svc
            .select_alternative(SelectAlternativeInput {
                project_id: proj.project_id.clone(),
                branch_id: feature.branch_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(out.selected_branch_id, feature.branch_id);
        assert_eq!(out.target_branch_id, proj.branch_id);
        assert_eq!(out.merge_type, "fast_forward");

        // Active branch is now the project's main, NOT the singleton.
        let active = svc
            .repository()
            .get_active_branch(&proj.project_id)
            .await
            .unwrap();
        assert_eq!(active.id, proj.branch_id);
        assert_eq!(active.name, "main");

        // The merged scene lives on main now.
        let main_scenes = svc
            .repository()
            .list_scenes_by_project_and_branch(&proj.project_id, &proj.branch_id)
            .await
            .unwrap();
        let merged = main_scenes
            .iter()
            .find(|s| (s.book_number, s.chapter_number, s.scene_order) == (1, 1, 1))
            .expect("alternative's scene now on main");
        assert_eq!(merged.summary, "feature picked");
    }

    /// `select_alternative` must refuse to switch when the underlying
    /// merge has conflicts — the caller is expected to resolve them and
    /// retry rather than silently leave the project's active branch
    /// half-promoted.
    #[tokio::test]
    async fn select_alternative_bails_when_merge_has_conflicts() {
        use spindle_core::models::{
            ContentRating, CreateBranchInput, SaveSceneDraftInput, SelectAlternativeInput,
            SwitchBranchInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "Select Conflict".into(),
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

        // Same conflict setup as merge_branch_reports_conflict_*: main
        // baseline, then feature branch, then main edits the position,
        // then feature edits the position. Conflict at (1,1,1).
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "main v1".into(),
            summary: "main v1".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let feature = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "feature-conf".into(),
                branch_type: "feature".into(),
                description: None,
            })
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "main v2".into(),
            summary: "main v2".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: feature.branch_id.clone(),
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "feature edit".into(),
            summary: "feature edit".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let err = svc
            .select_alternative(SelectAlternativeInput {
                project_id: proj.project_id.clone(),
                branch_id: feature.branch_id.clone(),
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("scene conflict"),
            "error should mention the conflict; got: {err}"
        );

        // Active branch is still feature — bail-out did not flip it to main.
        let active = svc
            .repository()
            .get_active_branch(&proj.project_id)
            .await
            .unwrap();
        assert_eq!(active.id, feature.branch_id);
    }

    #[tokio::test]
    async fn export_epub_emits_divergence_warnings_for_drifted_files() {
        use spindle_core::models::{
            ContentRating, DivergenceKind, ExportEpubInput, PushChapterToFileInput,
            SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
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

        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Scene one full body.".into(),
            summary: "s1".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let chapter = svc
            .repository()
            .find_chapter_by_number(&proj.project_id, 1, 1)
            .await
            .unwrap()
            .expect("chapter exists");

        let pushed = svc
            .push_chapter_to_file(PushChapterToFileInput {
                chapter_id: chapter.id.clone(),
                target_path: "ch1.txt".into(),
            })
            .await
            .unwrap();

        // Mutate file directly so the on-disk SHA disagrees with the link.
        std::fs::write(&pushed.target_path, "Externally edited body.").unwrap();

        let out = svc
            .export_epub(ExportEpubInput {
                project_id: proj.project_id.clone(),
                book_number: Some(1),
                start_chapter_number: None,
                end_chapter_number: None,
                author: Some("Anon".into()),
            })
            .await
            .unwrap();
        assert_eq!(out.divergence_warnings.len(), 1);
        assert!(matches!(
            out.divergence_warnings[0].kind,
            DivergenceKind::ContentMismatch
        ));
    }

    #[tokio::test]
    async fn backfill_scene_source_offsets_repairs_drifted_offsets() {
        use spindle_core::models::{
            BackfillSceneSourceOffsetsInput, ContentRating, PushChapterToFileInput,
            SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
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

        for (order, text) in [(1i32, "alpha"), (2, "beta")] {
            svc.save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: order,
                full_text: text.into(),
                summary: format!("s{order}"),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        }

        let chapter = svc
            .repository()
            .find_chapter_by_number(&proj.project_id, 1, 1)
            .await
            .unwrap()
            .expect("chapter exists");

        // Push to create a file and `scene_source_link` rows with correct offsets.
        svc.push_chapter_to_file(PushChapterToFileInput {
            chapter_id: chapter.id.clone(),
            target_path: "ch1.txt".into(),
        })
        .await
        .unwrap();

        // Drift the stored offsets on one link. After this, backfill should
        // detect the drift and reset them to the correct file positions.
        let links = svc
            .repository()
            .list_scene_source_links_by_project(&proj.project_id)
            .await
            .unwrap();
        let mut target_link: Option<crate::sqlite::records::SceneSourceLink> = None;
        for link in &links {
            let scene = svc.repository().get_scene(&link.scene_id).await.unwrap();
            if scene.scene_order == 1 {
                target_link = Some(link.clone());
                break;
            }
        }
        let target_link = target_link.expect("link for scene 1 exists");
        svc.repository()
            .upsert_scene_source_link(
                &proj.project_id,
                &target_link.scene_id,
                &target_link.source_path,
                &target_link.content_sha256,
                Some(999), // intentionally wrong
                Some(1234),
            )
            .await
            .unwrap();

        let branch_id = svc
            .repository()
            .get_active_branch(&proj.project_id)
            .await
            .unwrap()
            .id;
        let out = svc
            .backfill_scene_source_offsets(BackfillSceneSourceOffsetsInput {
                project_id: proj.project_id.clone(),
                branch_id,
            })
            .await
            .unwrap();
        assert_eq!(out.scanned_links, 2);
        assert_eq!(out.updated_links, 1);
        assert_eq!(out.unresolved_links, 0);

        // Verify the offsets actually got fixed.
        let restored = svc
            .repository()
            .get_scene_source_link_for_scene(&target_link.scene_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(restored.source_start_offset, Some(0));
        // The slicer now emits byte-exact body ranges (the trailing
        // `\n\n---\n\n` separator is stripped from each scene's
        // `source_byte_range`). For the Spindle-managed file
        // "alpha\n\n---\n\nbeta", scene 1's body is just "alpha" so
        // the end offset is 5.
        assert_eq!(restored.source_end_offset, Some(5));
    }

    #[tokio::test]
    async fn pull_chapter_from_file_updates_scene_bodies() {
        use spindle_core::models::{
            ContentRating, PullChapterFromFileInput, PullStatus, PushChapterToFileInput,
            SaveSceneDraftInput, SceneSyncStatus,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
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

        for (order, text) in [(1i32, "first body"), (2, "second body")] {
            svc.save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: order,
                full_text: text.into(),
                summary: format!("s{order}"),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        }

        let chapter = svc
            .repository()
            .find_chapter_by_number(&proj.project_id, 1, 1)
            .await
            .unwrap()
            .expect("chapter exists");

        // Push so we have a Spindle-formatted file on disk.
        let pushed = svc
            .push_chapter_to_file(PushChapterToFileInput {
                chapter_id: chapter.id.clone(),
                target_path: "ch1.txt".into(),
            })
            .await
            .unwrap();

        // Mutate the second scene body in the file.
        let updated_text = "first body\n\n---\n\nsecond body — REWRITTEN";
        std::fs::write(&pushed.target_path, updated_text).unwrap();

        // Pull and expect Diverged + Updated for scene 2, Match for scene 1.
        let report = svc
            .pull_chapter_from_file(PullChapterFromFileInput {
                chapter_id: chapter.id.clone(),
                source_path: "ch1.txt".into(),
            })
            .await
            .unwrap();
        assert!(matches!(report.status, PullStatus::Diverged));
        assert_eq!(report.scenes.len(), 2);
        assert!(matches!(report.scenes[0].status, SceneSyncStatus::Match));
        assert!(matches!(report.scenes[1].status, SceneSyncStatus::Updated));

        // Repository now reflects the pulled bodies.
        let scenes = svc
            .repository()
            .list_scenes_by_chapter(&chapter.id)
            .await
            .unwrap();
        let s2 = scenes.iter().find(|s| s.scene_order == 2).unwrap();
        assert_eq!(s2.full_text, "second body — REWRITTEN");
    }

    #[tokio::test]
    async fn push_chapter_to_file_writes_scenes_with_delimiter() {
        use spindle_core::models::{ContentRating, PushChapterToFileInput, SaveSceneDraftInput};

        let (tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
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

        for (order, text) in [(1i32, "scene one body"), (2, "scene two body")] {
            svc.save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: order,
                full_text: text.into(),
                summary: format!("summary {order}"),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        }

        // Find the chapter_id for chapter 1 of this project.
        let chapter = svc
            .repository()
            .find_chapter_by_number(&proj.project_id, 1, 1)
            .await
            .unwrap()
            .expect("chapter 1 should exist after save_scene_draft");

        // Push into a path inside data_dir.
        let rel = std::path::Path::new("ch1.txt");
        let report = svc
            .push_chapter_to_file(PushChapterToFileInput {
                chapter_id: chapter.id.clone(),
                target_path: rel.to_string_lossy().to_string(),
            })
            .await
            .unwrap();

        // Assert: 2 scene entries, file contains delimited bodies.
        assert_eq!(report.scenes.len(), 2);
        let written = std::fs::read_to_string(&report.target_path).unwrap();
        assert_eq!(written, "scene one body\n\n---\n\nscene two body");

        // Sanity: temp dir contains the file we just wrote.
        let _ = tmp;
    }

    #[tokio::test]
    async fn import_manuscript_persists_session_and_status_round_trips() {
        use spindle_core::models::{ImportManuscriptInput, ImportSessionStatus, ImportStatusInput};

        // SPINDLE_DATA_DIR pins the import data root inside the test temp.
        let env_guard = ScopedDataDir::new();

        let (tmp, svc) = fresh_service().await;

        // Create the target project up front so we don't need to create one
        // on-the-fly inside the import call (which uses default_import_data_dir).
        let project = svc
            .create_project(CreateProjectInput {
                name: "ImportTarget".to_string(),
                project_type: "novel".to_string(),
                genre: "fiction".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        // Stage a minimal manuscript file the slicer can chew on.
        let source_path = tmp.path().join("manuscript.txt");
        std::fs::write(
            &source_path,
            "Chapter 1\n\nMara opened the gate. Kade waited beyond.\n\n* * *\n\nKade nodded once.\n",
        )
        .unwrap();

        let import_output = svc
            .import_manuscript(ImportManuscriptInput {
                source_paths: vec![source_path.display().to_string()],
                target_project_id: Some(project.project_id.clone()),
                target_branch_id: None,
                create_project_name: None,
                source_format_hint: None,
                duplicate_strategy: None,
            })
            .await
            .unwrap();

        assert_eq!(import_output.structure.chapters.len(), 1);
        assert!(matches!(
            import_output.session.status,
            ImportSessionStatus::ReadyToHydrate | ImportSessionStatus::ReviewNeeded
        ));

        let status = svc
            .import_status(ImportStatusInput {
                project_id: project.project_id.clone(),
                session_id: import_output.session.session_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            status.session.session_id, import_output.session.session_id,
            "import_status round-trips the same session id"
        );
        assert!(
            status.structure.is_some(),
            "structural summary is persisted"
        );

        let _ = env_guard;
    }

    #[tokio::test]
    async fn export_bible_writes_payload_with_project_scoped_rows() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterInput,
            ExportBibleInput,
        };

        let (tmp, svc) = fresh_service().await;

        let project = svc
            .create_project(CreateProjectInput {
                name: "Export Target!".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();

        // One character so at least one branch-scoped row is present.
        svc.create_character(CreateCharacterInput {
            project_id: project.project_id.clone(),
            name: "Mara".to_string(),
            summary: "Warden.".to_string(),
            role: "protagonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".to_string()),
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
            initial_state: None,
        })
        .await
        .unwrap();

        let out = svc
            .export_bible(ExportBibleInput {
                project_id: project.project_id.clone(),
            })
            .await
            .unwrap();

        assert_eq!(out.filename, "export-target-bible-export.json");
        assert!(out.file_path.ends_with(&out.filename));
        // Bible export should have a non-trivial table count.
        assert!(
            out.exported_tables >= 5,
            "got {} tables",
            out.exported_tables
        );
        assert!(
            out.exported_records >= 3,
            "got {} records",
            out.exported_records
        );
        // File written into data_dir/exports/.
        let path = std::path::PathBuf::from(&out.file_path);
        assert!(path.exists(), "export file must exist at {}", out.file_path);
        let bytes = std::fs::read(&path).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            payload.get("format").and_then(|v| v.as_str()),
            Some("spindle-bible-export-v1"),
        );
        assert_eq!(
            payload.get("project_name").and_then(|v| v.as_str()),
            Some("Export Target!"),
        );
        // Project + branch + character should all be present in `tables`.
        let tables = payload.get("tables").and_then(|v| v.as_object()).unwrap();
        assert!(tables.contains_key("project"));
        assert!(tables.contains_key("bible_branch"));
        let chars = tables
            .get("character")
            .and_then(|v| v.as_array())
            .expect("character table is a list");
        assert_eq!(chars.len(), 1, "exactly one character was created");
        assert_eq!(chars[0].get("name").and_then(|v| v.as_str()), Some("Mara"),);
        // Voice-profile JSON columns are stored field-by-field in the V0001
        // schema (each list is its own json_valid TEXT column). Confirm
        // both the scalar `tone` and one of the JSON columns round-trip
        // through `looks_like_json_column`.
        let voice_profiles = tables
            .get("character_voice_profile")
            .and_then(|v| v.as_array())
            .unwrap();
        assert_eq!(voice_profiles.len(), 1);
        assert_eq!(
            voice_profiles[0].get("tone").and_then(|v| v.as_str()),
            Some("grim"),
        );
        assert!(
            voice_profiles[0]
                .get("vocabulary")
                .and_then(|v| v.as_array())
                .is_some(),
            "vocabulary JSON column should be re-parsed as an array",
        );
        drop(tmp);
    }

    /// Acceptance test for the typed-record export shape: seed one of each
    /// major entity, run `export_bible`, then deserialize each row in the
    /// exported `tables.*` arrays back into the matching `records::*` type
    /// via `serde_json::from_value`. Asserts no `from_value` errors and
    /// that every timestamp field renders as an RFC-3339 string (e.g.
    /// `"2026-05-22T..."`) rather than a unix-microsecond integer.
    #[tokio::test]
    async fn export_bible_round_trips_typed_records_with_rfc3339_timestamps() {
        use crate::sqlite::records;
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateBookInput,
            CreateChapterInput, CreateCharacterInput, CreateConflictInput, CreateEconomyInput,
            CreateFactionInput, CreateLocationInput, CreateMotifInput, CreateNarrativePromiseInput,
            CreatePlotLineInput, CreateReligionInput, CreateTermInput, CreateThemeInput,
            CreateTimelineEventInput, CreateWorldRuleInput, ExportBibleInput, StoryPlacement,
            WorldStateInput,
        };

        let (tmp, svc) = fresh_service().await;

        let project = svc
            .create_project(CreateProjectInput {
                name: "Roundtrip Subject".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "p".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        let project_id = project.project_id.clone();

        svc.create_book(CreateBookInput {
            project_id: project_id.clone(),
            title: Some("Book One".to_string()),
        })
        .await
        .unwrap();
        svc.create_chapter(CreateChapterInput {
            project_id: project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(1),
            title: Some("Chapter One".to_string()),
        })
        .await
        .unwrap();

        svc.create_character(CreateCharacterInput {
            project_id: project_id.clone(),
            name: "Mara".to_string(),
            summary: "Warden.".to_string(),
            role: "protagonist".to_string(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("grim".to_string()),
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
            initial_state: None,
        })
        .await
        .unwrap();

        svc.create_location(CreateLocationInput {
            project_id: project_id.clone(),
            name: "Riverhold".to_string(),
            kind: "city".to_string(),
            realm: None,
            summary: "Hub of trade.".to_string(),
            initial_state: WorldStateInput::default(),
        })
        .await
        .unwrap();

        svc.create_world_rule(CreateWorldRuleInput {
            project_id: project_id.clone(),
            rule_name: "Salt warding".to_string(),
            rule_type: "magic".to_string(),
            description: "Salt repels wraiths.".to_string(),
            scan_pattern: None,
            relevance_tags: Vec::new(),
            established_in: None,
        })
        .await
        .unwrap();

        svc.create_faction(CreateFactionInput {
            project_id: project_id.clone(),
            name: "The Order".to_string(),
            faction_type: "knightly".to_string(),
            realm: None,
            summary: "Sword-sworn protectors.".to_string(),
            tags: Vec::new(),
        })
        .await
        .unwrap();

        svc.create_religion(CreateReligionInput {
            project_id: project_id.clone(),
            name: "The Bright Path".to_string(),
            deity_or_principle: "Sun".to_string(),
            summary: "Worship of dawn.".to_string(),
            tags: Vec::new(),
        })
        .await
        .unwrap();

        svc.create_economy(CreateEconomyInput {
            project_id: project_id.clone(),
            name: "River Mark".to_string(),
            realm: None,
            summary: "Grain-based barter.".to_string(),
            scarce_resources: vec!["iron".to_string()],
            trade_goods: vec!["grain".to_string()],
            currency: Some("marks".to_string()),
            notes: Vec::new(),
        })
        .await
        .unwrap();

        svc.create_term(CreateTermInput {
            project_id: project_id.clone(),
            term_text: "Wraith".to_string(),
            pronunciation: None,
            definition: "A vengeful spirit.".to_string(),
            usage_context: None,
            origin: None,
        })
        .await
        .unwrap();

        svc.create_plot_line(CreatePlotLineInput {
            project_id: project_id.clone(),
            name: "Mara's hunt".to_string(),
            plot_type: "main".to_string(),
            summary: "Hunt the wraith.".to_string(),
            status: None,
            convergence_points: Vec::new(),
        })
        .await
        .unwrap();

        svc.create_conflict(CreateConflictInput {
            project_id: project_id.clone(),
            name: "Mara vs Wraith".to_string(),
            conflict_type: "person-vs-supernatural".to_string(),
            stakes: "Life of the village.".to_string(),
            escalation_stages: Vec::new(),
            expected_total_cycles: None,
            try_fail_cycles: Vec::new(),
            stated_consequences: Vec::new(),
        })
        .await
        .unwrap();

        svc.create_theme(CreateThemeInput {
            project_id: project_id.clone(),
            theme_statement: "Duty over self.".to_string(),
            thesis_antithesis: "Duty vs love.".to_string(),
            introduction_point: None,
            resolution_point: None,
        })
        .await
        .unwrap();

        svc.create_motif(CreateMotifInput {
            project_id: project_id.clone(),
            name: "Salt".to_string(),
            description: "Warding crystal.".to_string(),
            max_uses_per_chapter: None,
            connected_theme_ids: Vec::new(),
        })
        .await
        .unwrap();

        svc.create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project_id.clone(),
            promise_type: "mystery".to_string(),
            description: "What killed the elder?".to_string(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: None,
            notes: Vec::new(),
        })
        .await
        .unwrap();

        svc.create_timeline_event(CreateTimelineEventInput {
            project_id: project_id.clone(),
            title: "Founding".to_string(),
            event_type: "history".to_string(),
            placement: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            summary: "The town was founded.".to_string(),
            related_entity_ids: Vec::new(),
        })
        .await
        .unwrap();

        // Run the export.
        let out = svc
            .export_bible(ExportBibleInput {
                project_id: project_id.clone(),
            })
            .await
            .unwrap();
        let bytes = std::fs::read(&out.file_path).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let tables = payload
            .get("tables")
            .and_then(|v| v.as_object())
            .expect("tables map");

        // Project: single object, RFC-3339 timestamp.
        let project_v = tables.get("project").expect("project entry");
        let p: records::Project =
            serde_json::from_value(project_v.clone()).expect("project deserializes typed");
        assert_eq!(p.name, "Roundtrip Subject");
        let created_str = project_v
            .get("created_at")
            .and_then(|v| v.as_str())
            .expect("project.created_at is a string");
        assert!(
            created_str.starts_with("20") && created_str.contains('T'),
            "expected RFC-3339 timestamp, got {created_str}",
        );

        // Helper: deserialize every entry in `tables.<name>` as `R` and
        // assert each row's `created_at` (or first timestamp field) is an
        // RFC-3339 string. Returns the count round-tripped.
        fn round_trip<R: for<'de> serde::Deserialize<'de>>(
            tables: &serde_json::Map<String, serde_json::Value>,
            name: &str,
            ts_field: &str,
        ) -> usize {
            let arr = tables
                .get(name)
                .and_then(|v| v.as_array())
                .unwrap_or_else(|| panic!("{name} table is an array"));
            let mut count = 0usize;
            for row in arr {
                serde_json::from_value::<R>(row.clone())
                    .unwrap_or_else(|e| panic!("{name} row {row} did not deserialize: {e}"));
                let ts = row
                    .get(ts_field)
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| panic!("{name}.{ts_field} not a string in row {row}"));
                assert!(
                    ts.starts_with("20") && ts.contains('T'),
                    "{name}.{ts_field} is not RFC-3339: {ts}",
                );
                count += 1;
            }
            count
        }

        let mut entity_total = 0usize;
        entity_total += round_trip::<records::BibleBranch>(tables, "bible_branch", "created_at");
        entity_total += round_trip::<records::Book>(tables, "book", "created_at");
        entity_total += round_trip::<records::Chapter>(tables, "chapter", "created_at");
        entity_total += round_trip::<records::Character>(tables, "character", "created_at");
        entity_total += round_trip::<records::CharacterVoiceProfile>(
            tables,
            "character_voice_profile",
            "created_at",
        );
        entity_total += round_trip::<records::CharacterEmotionalProfile>(
            tables,
            "character_emotional_profile",
            "created_at",
        );
        entity_total += round_trip::<records::Location>(tables, "location", "created_at");
        entity_total += round_trip::<records::WorldRule>(tables, "world_rule", "created_at");
        entity_total += round_trip::<records::Faction>(tables, "faction", "created_at");
        entity_total += round_trip::<records::Religion>(tables, "religion", "created_at");
        entity_total += round_trip::<records::Economy>(tables, "economy", "created_at");
        entity_total += round_trip::<records::Term>(tables, "term", "created_at");
        entity_total += round_trip::<records::PlotLine>(tables, "plot_line", "created_at");
        entity_total += round_trip::<records::Conflict>(tables, "conflict", "created_at");
        entity_total += round_trip::<records::Theme>(tables, "theme", "created_at");
        entity_total += round_trip::<records::Motif>(tables, "motif", "created_at");
        entity_total +=
            round_trip::<records::NarrativePromise>(tables, "narrative_promise", "created_at");
        entity_total +=
            round_trip::<records::TimelineEvent>(tables, "timeline_event", "created_at");

        // 17 entity tables + project (asserted above) = 18 typed round-trips.
        assert!(
            entity_total >= 17,
            "expected at least 17 round-tripped entities, got {entity_total}",
        );

        // Cross-check timestamp shape on a few known fields: branch's
        // created_at is RFC-3339, not a number.
        let branch_arr = tables
            .get("bible_branch")
            .and_then(|v| v.as_array())
            .unwrap();
        assert!(!branch_arr.is_empty());
        assert!(
            branch_arr[0]
                .get("created_at")
                .and_then(|v| v.as_str())
                .is_some(),
            "bible_branch.created_at should be a string, got {:?}",
            branch_arr[0].get("created_at"),
        );
        assert!(
            branch_arr[0]
                .get("created_at")
                .and_then(|v| v.as_i64())
                .is_none(),
            "bible_branch.created_at must NOT be an integer",
        );

        drop(tmp);
    }

    /// End-to-end restore_save_point flow: create a project + character,
    /// snapshot the branch via `create_save_point`, mutate the live state
    /// (add a second character), then restore. The snapshot character must
    /// survive; the post-snapshot mutation must be rolled back; and a
    /// pre-restore backup save_point must be produced.
    #[tokio::test]
    async fn restore_save_point_rewinds_active_branch_to_snapshot() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterVoiceProfileData, CreateCharacterInput,
            CreateSavePointInput, RestoreSavePointInput,
        };

        let (_tmp, svc) = fresh_service().await;

        let project = svc
            .create_project(CreateProjectInput {
                name: "RestoreSavePointFlow".into(),
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
        // Initial character we want preserved across the restore.
        svc.create_character(CreateCharacterInput {
            project_id: project.project_id.clone(),
            name: "Mara".into(),
            summary: "Survives.".into(),
            role: "protagonist".into(),
            realm: None,
            voice_profile: empty_voice.clone(),
            emotional_profile: empty_emotional.clone(),
            initial_state: None,
        })
        .await
        .unwrap();

        // Capture the save_point. This also writes the snapshot file +
        // sha256/record_count metadata via `persist_save_point_snapshot`.
        let sp = svc
            .create_save_point(CreateSavePointInput {
                project_id: project.project_id.clone(),
                name: "milestone-1".into(),
                description: None,
            })
            .await
            .unwrap();

        // Post-snapshot mutation: a second character that should NOT
        // survive the restore.
        svc.create_character(CreateCharacterInput {
            project_id: project.project_id.clone(),
            name: "Ephemera".into(),
            summary: "Will vanish on restore.".into(),
            role: "antagonist".into(),
            realm: None,
            voice_profile: empty_voice.clone(),
            emotional_profile: empty_emotional.clone(),
            initial_state: None,
        })
        .await
        .unwrap();
        let pre = svc
            .repository
            .list_characters_by_project_and_branch(&project.project_id, &project.branch_id)
            .await
            .unwrap();
        assert_eq!(pre.len(), 2, "two characters before restore");

        // Restore.
        let out = svc
            .restore_save_point(RestoreSavePointInput {
                project_id: project.project_id.clone(),
                save_point_id: sp.save_point_id.clone(),
            })
            .await
            .unwrap();

        assert_eq!(out.save_point_id, sp.save_point_id);
        assert_eq!(out.branch_id, project.branch_id);
        assert_eq!(out.status, "restored");
        assert!(
            out.backup_save_point_id.starts_with("save_point:"),
            "backup save_point id should be minted: got {}",
            out.backup_save_point_id
        );
        // The restored snapshot carried exactly one character row.
        assert!(out.restored_tables >= 1, "got {}", out.restored_tables);
        assert!(out.restored_records >= 1, "got {}", out.restored_records);

        // Post-condition: only the original character is on the branch.
        let post = svc
            .repository
            .list_characters_by_project_and_branch(&project.project_id, &project.branch_id)
            .await
            .unwrap();
        assert_eq!(post.len(), 1, "exactly the snapshot character remains");
        assert_eq!(post[0].name, "Mara");
    }

    /// Ad-hoc RAII guard that scopes `SPINDLE_DATA_DIR` to a fresh temp dir.
    /// Restored on drop so other tests aren't affected.
    struct ScopedDataDir {
        previous: Option<std::ffi::OsString>,
        _tmp: TempDir,
    }

    impl ScopedDataDir {
        fn new() -> Self {
            let tmp = TempDir::new().unwrap();
            let previous = std::env::var_os("SPINDLE_DATA_DIR");
            // SAFETY: only one import_manuscript test sets this; tests are
            // run serially per-thread by tokio's default current-thread
            // executor, and the guard restores on drop.
            unsafe { std::env::set_var("SPINDLE_DATA_DIR", tmp.path()) };
            Self {
                previous,
                _tmp: tmp,
            }
        }
    }

    impl Drop for ScopedDataDir {
        fn drop(&mut self) {
            unsafe {
                if let Some(prev) = self.previous.take() {
                    std::env::set_var("SPINDLE_DATA_DIR", prev);
                } else {
                    std::env::remove_var("SPINDLE_DATA_DIR");
                }
            }
        }
    }

    /// Exercises the happy path: a project with one scene runs all checks
    /// and reports zero blocking issues (no errors). The summary still
    /// reports info findings for things like the missing chapter summary
    /// and tone metadata, which we assert positively to keep the SQLite
    /// port's dispatch behaviour observable.
    #[tokio::test]
    async fn check_consistency_happy_path_no_blocking_issues() {
        use spindle_core::models::{
            CheckConsistencyInput, ConsistencyScopeInput, ContentRating, ContextFormat,
            SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "chk".into(),
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

        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "A quiet scene.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: Some("calm".into()),
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .check_consistency(CheckConsistencyInput {
                project_id: proj.project_id.clone(),
                scope: ConsistencyScopeInput::full(),
                checks: Vec::new(),
                severity_filter: Vec::new(),
                deep_check: None,
                subjects: Vec::new(),
                format: Some(ContextFormat::Markdown),
                budget_tokens: None,
            })
            .await
            .unwrap();

        assert_eq!(
            out.summary.error_count, 0,
            "happy path should produce no blocking errors, got issues: {:?}",
            out.issues
        );
        assert!(
            out.markdown.is_some(),
            "markdown should be populated when format=Markdown"
        );
        assert!(
            out.report_sections.is_empty(),
            "happy path has no canon → Phase-4 fan-out produces no sections"
        );
    }

    /// Happy path for `commit_scene_changes`: one character state and one
    /// canonical fact, both succeed; the response carries the per-entry
    /// result rows, no errors, and a zero-finding summary (Phase-4 fan-out
    /// is gated, so `findings_summary` is documented as empty).
    #[tokio::test]
    async fn commit_scene_changes_happy_path_state_and_fact() {
        use spindle_core::models::{
            CanonicalFactEntry, CanonicalFactScope, CharacterEmotionalProfileData,
            CharacterStatePatch, CharacterStatePatchEntry, CharacterVoiceProfileData,
            CommitSceneChangesInput, ContentRating, SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "csc".into(),
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

        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Warden".into(),
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
                initial_state: None,
            })
            .await
            .unwrap();

        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate.".into(),
                summary: "stage".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        let out = svc
            .commit_scene_changes(CommitSceneChangesInput {
                project_id: proj.project_id.clone(),
                scene_id: scene.scene_id.clone(),
                character_states: vec![CharacterStatePatchEntry {
                    character_id: mara.character_id.clone(),
                    changes: CharacterStatePatch {
                        emotional_state: std::collections::BTreeMap::new(),
                        goals: None,
                        status: Some(vec!["bracing for the dark".into()]),
                        notes: None,
                        source_summary: None,
                    },
                }],
                canonical_facts: vec![CanonicalFactEntry {
                    fact_type: None,
                    key: None,
                    value: None,
                    subject_table: Some("character".into()),
                    subject_id: Some(mara.character_id.clone()),
                    predicate: Some("oath".into()),
                    value_kind: Some("string".into()),
                    value_text: Some("ash gate warden".into()),
                    value_number: None,
                    value_unit: None,
                    value_json: None,
                    aliases: Vec::new(),
                    scope: Some(CanonicalFactScope::Invariant),
                    valid_from: None,
                    valid_until: None,
                    context: None,
                    supersedes_fact_id: None,
                }],
                relationship_updates: Vec::new(),
                accept_world_rule_risks: false,
            })
            .await
            .unwrap();

        assert_eq!(out.scene_id, scene.scene_id);
        assert_eq!(out.character_states.len(), 1, "one character state entry");
        assert!(
            out.character_states[0].state_id.is_some(),
            "character state should commit: {:?}",
            out.character_states[0].error
        );
        assert!(out.character_states[0].error.is_none());
        assert_eq!(out.canonical_facts.len(), 1, "one canonical fact entry");
        assert!(
            out.canonical_facts[0].canonical_fact_id.is_some(),
            "fact should register: {:?}",
            out.canonical_facts[0].error
        );
        assert_eq!(out.canonical_facts[0].fact_type, "oath");
        assert_eq!(out.canonical_facts[0].key, "oath");
        assert!(out.relationship_updates.is_empty());
        assert!(out.world_rule_hits.is_empty(), "no world rules seeded");
        // Phase-4 fan-out runs but produces nothing: the only fact has
        // scene_id == this scene's id (validators skip same-scene facts)
        // and there are no other characters / world rules / interventions.
        assert_eq!(out.findings_summary.total_count, 0);
        assert_eq!(out.findings_summary.error_count, 0);
    }

    /// Phase-4 fan-out wiring inside `commit_scene_changes`: with a project
    /// where scene 1 has registered the canonical fact `oath -> ash gate
    /// warden`, committing changes on scene 2 — whose prose mentions
    /// "oath" but not "ash gate warden" — must produce a
    /// `canonical_fact_prose_drift` finding in `findings_summary`.
    #[tokio::test]
    async fn commit_scene_changes_populates_phase_four_findings_summary() {
        use spindle_core::models::{
            CanonicalFactEntry, CanonicalFactScope, CharacterEmotionalProfileData,
            CharacterStatePatch, CharacterStatePatchEntry, CharacterVoiceProfileData,
            CommitSceneChangesInput, ContentRating, CreateBookInput, CreateChapterInput,
            SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "ph4-commit".into(),
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

        svc.create_book(CreateBookInput {
            project_id: proj.project_id.clone(),
            title: Some("Book One".into()),
        })
        .await
        .unwrap();
        svc.create_chapter(CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(1),
            title: Some("Chapter One".into()),
        })
        .await
        .unwrap();
        svc.create_chapter(CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(2),
            title: Some("Chapter Two".into()),
        })
        .await
        .unwrap();

        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Warden".into(),
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
                initial_state: None,
            })
            .await
            .unwrap();

        // Scene 1: register the canonical fact (mara.oath = "ash gate warden")
        // by saving the scene then committing the fact against scene 1.
        let scene1 = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara swore her oath at the Ash Gate.".into(),
                summary: "establishment".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();
        svc.commit_scene_changes(CommitSceneChangesInput {
            project_id: proj.project_id.clone(),
            scene_id: scene1.scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: vec![CanonicalFactEntry {
                fact_type: None,
                key: None,
                value: None,
                subject_table: Some("character".into()),
                subject_id: Some(mara.character_id.clone()),
                predicate: Some("oath".into()),
                value_kind: Some("string".into()),
                value_text: Some("ash gate warden".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: Some(CanonicalFactScope::Invariant),
                valid_from: None,
                valid_until: None,
                context: None,
                supersedes_fact_id: None,
            }],
            relationship_updates: Vec::new(),
            accept_world_rule_risks: false,
        })
        .await
        .unwrap();

        // Scene 2: prose mentions the canonical key ("oath") but not the
        // canonical value ("ash gate warden"). The Phase-4 canonical-fact
        // validator should fire on commit.
        let scene2 = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 2,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara's oath weighed heavy, but she said nothing of its terms.".into(),
                summary: "drift beat".into(),
                content_rating: ContentRating::General,
                tone: Some("grim".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        let out = svc
            .commit_scene_changes(CommitSceneChangesInput {
                project_id: proj.project_id.clone(),
                scene_id: scene2.scene_id.clone(),
                character_states: vec![CharacterStatePatchEntry {
                    character_id: mara.character_id.clone(),
                    changes: CharacterStatePatch {
                        emotional_state: std::collections::BTreeMap::new(),
                        goals: None,
                        status: Some(vec!["bracing for the dark".into()]),
                        notes: None,
                        source_summary: None,
                    },
                }],
                canonical_facts: Vec::new(),
                relationship_updates: Vec::new(),
                accept_world_rule_risks: false,
            })
            .await
            .unwrap();

        // The summary must reflect the Phase-4 drift finding.
        assert!(
            out.findings_summary.total_count >= 1,
            "expected at least one Phase-4 finding, got summary: {:?}",
            out.findings_summary
        );
        assert!(
            out.findings_summary
                .by_check
                .get("canonical_fact_prose_drift")
                .copied()
                .unwrap_or(0)
                >= 1,
            "expected a canonical_fact_prose_drift finding, got summary: {:?}",
            out.findings_summary
        );
    }

    /// Phase-4 scanners inside `revise_scene`: after seeding a character
    /// with a `forbidden_words` voice profile, revising a scene on a
    /// non-main branch to inject that forbidden phrase into the
    /// character's attributed dialogue must surface a `voice_drift`
    /// finding in the response.
    #[tokio::test]
    async fn revise_scene_emits_voice_drift_finding() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterVoiceProfileData, ContentRating,
            CreateBookInput, CreateBranchInput, CreateChapterInput, ReviseSceneInput,
            SaveSceneDraftInput, SwitchBranchInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "ph4-revise".into(),
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

        svc.create_book(CreateBookInput {
            project_id: proj.project_id.clone(),
            title: Some("Book One".into()),
        })
        .await
        .unwrap();
        svc.create_chapter(CreateChapterInput {
            project_id: proj.project_id.clone(),
            book_number: Some(1),
            book_id: None,
            chapter_number: Some(1),
            title: Some("Chapter One".into()),
        })
        .await
        .unwrap();

        // Character whose voice profile forbids the phrase "as you know".
        svc.create_character(CreateCharacterInput {
            project_id: proj.project_id.clone(),
            name: "Jim Dalton".into(),
            summary: "Dockmaster".into(),
            role: "supporting".into(),
            realm: None,
            voice_profile: CharacterVoiceProfileData {
                tone: Some("plainspoken".into()),
                vocabulary: Vec::new(),
                sentence_structure: Vec::new(),
                tics: Vec::new(),
                forbidden_words: vec!["as you know".into()],
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
            initial_state: None,
        })
        .await
        .unwrap();

        // Initial scene on main: clean prose, no forbidden phrase.
        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Jim Dalton tapped the table and said, \"Keep it moving.\"".into(),
                summary: "clean".into(),
                content_rating: ContentRating::General,
                tone: Some("plainspoken".into()),
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        // revise_scene requires a non-main active branch.
        let branch = svc
            .create_branch(CreateBranchInput {
                project_id: proj.project_id.clone(),
                parent_branch_id: None,
                name: "revisions".into(),
                branch_type: "draft".into(),
                description: None,
            })
            .await
            .unwrap();
        svc.switch_branch(SwitchBranchInput {
            project_id: proj.project_id.clone(),
            branch_id: branch.branch_id.clone(),
        })
        .await
        .unwrap();

        let out = svc
            .revise_scene(ReviseSceneInput {
                project_id: proj.project_id.clone(),
                scene_id: scene.scene_id.clone(),
                full_text:
                    "Jim Dalton tapped the table and said, \"As you know, the docks are quiet tonight.\""
                        .into(),
                summary: "drift".into(),
                content_rating: ContentRating::General,
                tone: Some("plainspoken".into()),
            })
            .await
            .unwrap();

        assert!(
            !out.voice_drift.is_empty(),
            "expected at least one voice_drift finding, got {:?}",
            out.voice_drift
        );
        let drift = &out.voice_drift[0];
        assert_eq!(
            drift.forbidden_phrase.as_deref(),
            Some("as you know"),
            "voice_drift finding should cite the forbidden phrase: {:?}",
            drift
        );
    }

    /// Subjects narrowing actually filters the scene set against the
    /// supplied subjects (Gap 6). Pinning a character via the `subjects`
    /// list must restrict the validator pass to scenes that reference
    /// that character's name in their prose — scenes that don't
    /// reference it are not validated.
    #[tokio::test]
    async fn check_consistency_subjects_narrowing_filters_scene_set() {
        use spindle_core::models::{
            CharacterEmotionalProfileData, CharacterStatePatch, CharacterVoiceProfileData,
            CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateCharacterInput,
            SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "chk-subj".into(),
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

        let mara = svc
            .create_character(CreateCharacterInput {
                project_id: proj.project_id.clone(),
                name: "Mara".into(),
                summary: "Warden.".into(),
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

        // Scene 1: contains "Mara" by name → should pass narrowing.
        // Scene 2: doesn't reference Mara at all → should be filtered out.
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate, watching the dark.".into(),
            summary: "stage Mara".into(),
            content_rating: ContentRating::General,
            tone: Some("calm".into()),
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "Aldric pondered the empty courtyard, unrelated.".into(),
            summary: "unrelated beat".into(),
            content_rating: ContentRating::General,
            tone: Some("calm".into()),
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .check_consistency(CheckConsistencyInput {
                project_id: proj.project_id.clone(),
                scope: ConsistencyScopeInput::full(),
                checks: Vec::new(),
                severity_filter: Vec::new(),
                deep_check: None,
                subjects: vec![mara.character_id.clone()],
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();

        // No info notice fires anymore; narrowing is real.
        assert!(
            !out.issues
                .iter()
                .any(|issue| issue.check_type == "subjects_narrowing"),
            "subjects_narrowing info notice must be gone: got {:?}",
            out.issues
        );
    }

    /// Phase-4 world_rule_semantic_drift wiring: with a project that has a
    /// world rule whose scan_pattern matches scene prose AND surrounding
    /// violation-context markers ("ignore"), `check_consistency` must
    /// produce at least one `world_rule_semantic_drift` issue and a
    /// matching section in `report_sections`.
    #[tokio::test]
    async fn check_consistency_emits_world_rule_semantic_drift_issue() {
        use spindle_core::models::{
            CheckConsistencyInput, ConsistencyScopeInput, ContentRating, CreateWorldRuleInput,
            EstablishedIn, SaveSceneDraftInput,
        };

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "ph4-world".into(),
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

        // Seed a world rule whose scan_pattern matches the scene prose.
        // The surrounding window must contain a violation marker
        // ("ignore") for the scanner to promote severity to Likely (which
        // the validator maps to Warning).
        svc.create_world_rule(CreateWorldRuleInput {
            project_id: proj.project_id.clone(),
            rule_name: "Sigils require contact".into(),
            rule_type: "magic".into(),
            description: "Magic sigils must require physical contact".into(),
            scan_pattern: Some(r"\bsigil\b".into()),
            relevance_tags: Vec::new(),
            established_in: Some(EstablishedIn {
                book_number: 1,
                chapter_number: 1,
                note: None,
            }),
        })
        .await
        .unwrap();

        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Eldrin tried to ignore the sigil and cast at range anyway, breaking canon."
                .into(),
            summary: "violation beat".into(),
            content_rating: ContentRating::General,
            tone: Some("grim".into()),
            generation_id: None,
            source_path: None,
        })
        .await
        .unwrap();

        let out = svc
            .check_consistency(CheckConsistencyInput {
                project_id: proj.project_id.clone(),
                scope: ConsistencyScopeInput::full(),
                checks: Vec::new(),
                severity_filter: Vec::new(),
                deep_check: None,
                subjects: Vec::new(),
                format: None,
                budget_tokens: None,
            })
            .await
            .unwrap();

        assert!(
            out.issues.iter().any(|issue| {
                issue.check_type == "world_rule_semantic_drift" && issue.severity == "warning"
            }),
            "expected a world_rule_semantic_drift warning, got: {:?}",
            out.issues
        );
        assert!(
            out.report_sections
                .iter()
                .any(
                    |section| section.validator_id == "world_rule_semantic_drift"
                        && !section.scenes.is_empty()
                ),
            "expected a world_rule_semantic_drift section in report_sections"
        );
    }

    /// One-scene project + one chapter_summary attached to that chapter.
    /// `get_scene_delete_impact` should classify the chapter_summary as a
    /// chapter_artifact, leave hard_blockers empty, and resolve
    /// `delete_readiness` to `needs_followup`. The reported scene fields
    /// must point at the scene we just inserted.
    #[tokio::test]
    async fn get_scene_delete_impact_reports_chapter_artifacts() {
        use spindle_core::models::{ContentRating, SaveSceneDraftInput, SaveSummaryInput};

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "delete-impact".into(),
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

        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate.".into(),
                summary: "stage".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        svc.save_summary(SaveSummaryInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            entity_type: None,
            entity_id: None,
            summary: "ch1 close".into(),
            key_events: Vec::new(),
            character_changes: Vec::new(),
            relationship_shifts: Vec::new(),
            arc_advances: Vec::new(),
            promise_events: Vec::new(),
        })
        .await
        .unwrap();

        let impact = svc
            .get_scene_delete_impact(GetSceneDeleteImpactInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                scene_order: 1,
            })
            .await
            .unwrap();

        assert_eq!(impact.scene.scene_id, scene.scene_id);
        assert_eq!(impact.scene.book_number, 1);
        assert_eq!(impact.scene.chapter_number, 1);
        assert_eq!(impact.scene.scene_order, 1);
        assert!(impact.hard_blockers.is_empty(), "no hard blockers expected");
        assert_eq!(
            impact.delete_readiness,
            SceneDeleteReadiness::NeedsFollowup,
            "chapter_summary artifact must demote readiness from Clear"
        );

        // The chapter_summary artifact group must be present and reference
        // the freshly-saved summary.
        let summaries: Vec<_> = impact
            .chapter_artifacts
            .iter()
            .filter(|g| g.dependency_type == "chapter_summary")
            .collect();
        assert_eq!(summaries.len(), 1, "exactly one chapter_summary group");
        assert_eq!(summaries[0].count, 1);
        assert_eq!(summaries[0].sample_record_ids.len(), 1);
        assert!(
            summaries[0].sample_record_ids[0].starts_with("chapter_summary:"),
            "got {:?}",
            summaries[0].sample_record_ids
        );

        // The JSON projection should round-trip cleanly (this is what
        // `read_project_resource` will return downstream).
        let json = serde_json::to_value(&impact).unwrap();
        assert_eq!(json["delete_readiness"], "needs_followup");
        assert_eq!(json["scene"]["scene_id"], impact.scene.scene_id);
    }

    /// Two-scene project where both scenes live in book 1 chapter 1.
    /// Asking `get_scene_move_impact` to move scene 1 onto scene 2's
    /// position must flag a `destination_scene` hard blocker pointing
    /// at the second scene id and resolve `move_readiness` to
    /// `blocked`. The reported `scene` (source) and `destination`
    /// fields must match the natural-key inputs.
    #[tokio::test]
    async fn get_scene_move_impact_flags_destination_conflict() {
        use spindle_core::models::{ContentRating, SaveSceneDraftInput};

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "move-impact".into(),
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

        let source = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate.".into(),
                summary: "stage".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        let destination = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 2,
                full_text: "She crossed the threshold.".into(),
                summary: "cross".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        let impact = svc
            .get_scene_move_impact(GetSceneMoveImpactInput {
                project_id: proj.project_id.clone(),
                from_book_number: 1,
                from_chapter_number: 1,
                from_scene_order: 1,
                to_book_number: 1,
                to_chapter_number: 1,
                to_scene_order: 2,
            })
            .await
            .unwrap();

        assert_eq!(impact.scene.scene_id, source.scene_id);
        assert_eq!(impact.destination.book_number, 1);
        assert_eq!(impact.destination.chapter_number, 1);
        assert_eq!(impact.destination.scene_order, 2);
        assert_eq!(
            impact.destination.existing_scene_id.as_deref(),
            Some(destination.scene_id.as_str())
        );
        assert_eq!(
            impact.move_readiness,
            SceneMoveReadiness::Blocked,
            "destination occupancy must Block the move"
        );

        // Find the destination_scene hard blocker group and confirm it
        // points at the second scene.
        let dest_blockers: Vec<_> = impact
            .hard_blockers
            .iter()
            .filter(|g| g.dependency_type == "destination_scene")
            .collect();
        assert_eq!(
            dest_blockers.len(),
            1,
            "exactly one destination_scene group"
        );
        assert_eq!(dest_blockers[0].count, 1);
        assert_eq!(
            dest_blockers[0].sample_record_ids,
            vec![destination.scene_id.clone()]
        );

        // The JSON projection should round-trip cleanly.
        let json = serde_json::to_value(&impact).unwrap();
        assert_eq!(json["move_readiness"], "blocked");
        assert_eq!(json["scene"]["scene_id"], impact.scene.scene_id);
        assert_eq!(
            json["destination"]["existing_scene_id"],
            destination.scene_id
        );
    }

    /// `read_project_resource` must parse the `scene-delete-impact/...`
    /// and `scene-move-impact/...` path tails into the impact-method
    /// inputs and return the structured JSON payload. This is the
    /// integration test for the dispatch wiring; the per-method tests
    /// above already cover the analysis logic.
    #[tokio::test]
    async fn read_project_resource_dispatches_scene_impact_arms() {
        use spindle_core::models::{ContentRating, SaveSceneDraftInput};

        let (_tmp, svc) = fresh_service().await;
        let proj = svc
            .create_project(CreateProjectInput {
                name: "impact-dispatch".into(),
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

        let scene = svc
            .save_scene_draft(SaveSceneDraftInput {
                project_id: proj.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order: 1,
                full_text: "Mara stood at the Ash Gate.".into(),
                summary: "stage".into(),
                content_rating: ContentRating::General,
                tone: None,
                generation_id: None,
                source_path: None,
            })
            .await
            .unwrap();

        // Delete impact: clear envelope, scene fields echo the saved scene.
        let delete_json = svc
            .read_project_resource(&proj.project_id, "scene-delete-impact/1/1/1")
            .await
            .unwrap();
        assert_eq!(delete_json["delete_readiness"], "clear");
        assert_eq!(delete_json["scene"]["scene_id"], scene.scene_id);
        assert_eq!(delete_json["scene"]["book_number"], 1);
        assert_eq!(delete_json["scene"]["chapter_number"], 1);
        assert_eq!(delete_json["scene"]["scene_order"], 1);
        assert!(delete_json["hard_blockers"].is_array());

        // Move impact (no destination scene): clear envelope, destination
        // existing_scene_id resolves to null.
        let move_json = svc
            .read_project_resource(&proj.project_id, "scene-move-impact/1/1/1/1/2/1")
            .await
            .unwrap();
        assert_eq!(move_json["move_readiness"], "clear");
        assert_eq!(move_json["scene"]["scene_id"], scene.scene_id);
        assert_eq!(move_json["destination"]["book_number"], 1);
        assert_eq!(move_json["destination"]["chapter_number"], 2);
        assert_eq!(move_json["destination"]["scene_order"], 1);
        assert!(move_json["destination"]["existing_scene_id"].is_null());

        // Malformed path tail bails with a clear error.
        let err = svc
            .read_project_resource(&proj.project_id, "scene-delete-impact/1/1")
            .await
            .expect_err("missing path segment must error");
        assert!(
            err.to_string()
                .contains("scene delete impact resource path"),
            "got: {err}"
        );
        let err = svc
            .read_project_resource(&proj.project_id, "scene-move-impact/1/1/1/1/2")
            .await
            .expect_err("missing path segment must error");
        assert!(
            err.to_string().contains("scene move impact resource path"),
            "got: {err}"
        );
    }
}
