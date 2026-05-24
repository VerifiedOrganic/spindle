use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use spindle_core::models::{
    AnnotateSceneBeatsInput, CheckConsistencyInput, CommitSceneChangesInput, ConsistencyScopeInput,
    ContextFormat, ContinueGenerationInput, CreateSavePointInput, GetChapterBriefingInput,
    GetSceneContextInput, RunDualPersonaReviewInput, SaveSceneDraftInput, SaveSummaryInput,
    TestAgentInput,
};

use crate::artifacts::{
    ArtifactStore, ChapterSummaryArtifact, CheckpointReportArtifact,
    GeneratedChapterSummaryPackage, GeneratedScenePackage, SceneGenerationArtifact,
};
use crate::mcp::{DraftRouteBinding, McpHarnessClient};
use crate::plan::NextAction;
use crate::state::{
    CheckpointRecord, CheckpointStatus, HarnessState, SceneDraftDiagnostics, ScenePhase,
};

const MAX_GENERATION_ROUNDS: usize = 8;
const CHAPTER_BRIEFING_RECENT_LIMIT: usize = 3;
const CHAPTER_BRIEFING_TOKEN_BUDGET: usize = 3500;
const SCENE_CONTEXT_TOKEN_BUDGET: usize = 6000;

pub struct ExecutionResult {
    pub state: HarnessState,
    pub message: String,
}

pub async fn execute_one(
    state_path: &Path,
    mut state: HarnessState,
    client: &McpHarnessClient,
    next_action: NextAction,
) -> Result<ExecutionResult> {
    let artifact_store = ArtifactStore::new(resolve_artifacts_root(state_path, &state));

    let message = match next_action {
        NextAction::Blocked => anyhow::bail!("execution blocked"),
        NextAction::AwaitCheckpointReview {
            start_chapter,
            end_chapter,
            save_point_id,
        } => anyhow::bail!(
            "checkpoint {}-{} is awaiting human review (save point {})",
            start_chapter,
            end_chapter,
            save_point_id
        ),
        NextAction::RunCheckpoint {
            start_chapter,
            end_chapter,
        } => {
            run_checkpoint(
                state_path,
                &mut state,
                client,
                &artifact_store,
                start_chapter,
                end_chapter,
            )
            .await?
        }
        NextAction::DraftScene {
            chapter_number,
            scene_order,
        } => {
            draft_scene(
                state_path,
                &mut state,
                client,
                &artifact_store,
                chapter_number,
                scene_order,
            )
            .await?
        }
        NextAction::CommitSceneChanges {
            chapter_number,
            scene_order,
            ..
        } => {
            commit_scene_changes(
                state_path,
                &mut state,
                client,
                &artifact_store,
                chapter_number,
                scene_order,
            )
            .await?
        }
        NextAction::AnnotateSceneBeats {
            chapter_number,
            scene_order,
            ..
        } => {
            annotate_scene_beats(
                state_path,
                &mut state,
                client,
                &artifact_store,
                chapter_number,
                scene_order,
            )
            .await?
        }
        NextAction::SaveChapterSummary { chapter_number } => {
            save_chapter_summary(
                state_path,
                &mut state,
                client,
                &artifact_store,
                chapter_number,
            )
            .await?
        }
        NextAction::Complete => "Harness range is complete.".to_string(),
    };

    Ok(ExecutionResult { state, message })
}

async fn draft_scene(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    chapter_number: i32,
    scene_order: i32,
) -> Result<String> {
    let (chapter_index, scene_index) =
        scene_indices(state, chapter_number, scene_order).context("scene not found in state")?;
    ensure_scene_artifact_path(state, state_path, chapter_index, scene_index)?;

    let chapter = state.chapters[chapter_index].clone();
    let scene = chapter.scenes[scene_index].clone();
    let artifact_path = scene
        .scene_artifact_path
        .clone()
        .context("scene artifact path missing after initialization")?;
    let draft_route = client.resolve_draft_route().await?;
    let mut artifact = load_or_create_scene_artifact(
        client,
        artifact_store,
        &draft_route,
        state,
        &chapter,
        &scene,
        &artifact_path,
    )
    .await?;

    if artifact.save_draft_output.is_none() {
        ensure_scene_package_ready(
            client,
            artifact_store,
            &scene,
            &mut artifact,
            &artifact_path,
        )
        .await?;
        let package = artifact
            .package
            .as_ref()
            .context("scene artifact missing generated package")?;
        let save_output = client
            .save_scene_draft(&SaveSceneDraftInput {
                project_id: state.project_id.clone(),
                book_number: state.book_number,
                chapter_number,
                chapter_id: None,
                scene_order,
                full_text: package.full_text.clone(),
                summary: package.summary.clone(),
                content_rating: scene.content_rating,
                tone: package.tone.clone().or(scene.tone.clone()),
                source_path: scene.source_path.clone(),
                generation_id: None,
            })
            .await
            .with_context(|| {
                format!("failed to save draft for chapter {chapter_number} scene {scene_order}")
            })?;
        artifact.save_draft_output = Some(save_output.clone());
        artifact_store.save_json(&artifact_path, &artifact)?;
    }

    let save_output = artifact
        .save_draft_output
        .as_ref()
        .context("scene artifact missing save_scene_draft output")?;
    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
    live_scene.scene_id = Some(save_output.scene_id.clone());
    live_scene.phase = ScenePhase::DraftSaved;
    live_scene.blocked_reason = None;
    live_scene.draft_diagnostics = Some(SceneDraftDiagnostics {
        pacing_warnings: save_output.pacing_warnings.clone(),
        agency_warning: save_output.agency_warning.clone(),
        tone_deviation: save_output.tone_deviation,
        content_rating_valid: save_output.content_rating_valid,
        content_rating_warnings: save_output.content_rating_warnings.clone(),
    });
    state.save(state_path)?;

    Ok(format!(
        "Saved draft for chapter {chapter_number} scene {scene_order} as {}",
        save_output.scene_id
    ))
}

async fn commit_scene_changes(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    chapter_number: i32,
    scene_order: i32,
) -> Result<String> {
    let (chapter_index, scene_index) =
        scene_indices(state, chapter_number, scene_order).context("scene not found in state")?;
    let scene = state.chapters[chapter_index].scenes[scene_index].clone();
    let artifact_path = scene
        .scene_artifact_path
        .clone()
        .context("scene artifact path missing")?;
    let mut artifact: SceneGenerationArtifact = artifact_store.load_json(&artifact_path)?;

    if artifact.commit_output.is_none() {
        let scene_id = scene
            .scene_id
            .clone()
            .context("scene_id missing for commit_scene_changes")?;
        let package = artifact
            .package
            .as_ref()
            .context("scene artifact missing generated package")?;
        let commit_output = client
            .commit_scene_changes(&CommitSceneChangesInput {
                project_id: state.project_id.clone(),
                scene_id,
                character_states: package.character_states.clone(),
                canonical_facts: package.canonical_facts.clone(),
                relationship_updates: package.relationship_updates.clone(),
                accept_world_rule_risks: true,
            })
            .await
            .with_context(|| {
                format!("failed to commit scene changes for chapter {chapter_number} scene {scene_order}")
            })?;
        artifact.commit_output = Some(commit_output.clone());
        artifact_store.save_json(&artifact_path, &artifact)?;
    }

    let commit_output = artifact
        .commit_output
        .as_ref()
        .context("scene artifact missing commit output")?;
    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
    if commit_output_has_errors(commit_output) {
        live_scene.blocked_reason = Some(format!(
            "commit_scene_changes applied partial results; inspect artifact {} before continuing",
            artifact_store.root().join(&artifact_path).display()
        ));
        state.save(state_path)?;
        anyhow::bail!(
            "commit_scene_changes reported per-item errors for chapter {} scene {}",
            chapter_number,
            scene_order
        );
    }

    live_scene.phase = ScenePhase::ChangesCommitted;
    live_scene.blocked_reason = None;
    state.save(state_path)?;
    Ok(format!(
        "Committed scene changes for chapter {chapter_number} scene {scene_order}"
    ))
}

async fn annotate_scene_beats(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    chapter_number: i32,
    scene_order: i32,
) -> Result<String> {
    let (chapter_index, scene_index) =
        scene_indices(state, chapter_number, scene_order).context("scene not found in state")?;
    let scene = state.chapters[chapter_index].scenes[scene_index].clone();
    let artifact_path = scene
        .scene_artifact_path
        .clone()
        .context("scene artifact path missing")?;
    let mut artifact: SceneGenerationArtifact = artifact_store.load_json(&artifact_path)?;

    if artifact.beat_annotation_output.is_none() {
        let scene_id = scene
            .scene_id
            .clone()
            .context("scene_id missing for annotate_scene_beats")?;
        let package = artifact
            .package
            .as_ref()
            .context("scene artifact missing generated package")?;
        let annotation_output = client
            .annotate_scene_beats(&AnnotateSceneBeatsInput {
                project_id: state.project_id.clone(),
                scene_id,
                beats: package.beats.clone(),
                motif_ids: Vec::new(),
                theme_ids: Vec::new(),
                conflict_ids: Vec::new(),
            })
            .await
            .with_context(|| {
                format!("failed to annotate beats for chapter {chapter_number} scene {scene_order}")
            })?;
        artifact.beat_annotation_output = Some(annotation_output);
        artifact_store.save_json(&artifact_path, &artifact)?;
    }

    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
    live_scene.phase = ScenePhase::BeatsAnnotated;
    live_scene.blocked_reason = None;
    state.save(state_path)?;
    Ok(format!(
        "Annotated beats for chapter {chapter_number} scene {scene_order}"
    ))
}

async fn save_chapter_summary(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    chapter_number: i32,
) -> Result<String> {
    let chapter_index =
        chapter_index(state, chapter_number).context("chapter not found in state")?;
    ensure_summary_artifact_path(state, state_path, chapter_index)?;

    let chapter = state.chapters[chapter_index].clone();
    let artifact_path = chapter
        .summary_artifact_path
        .clone()
        .context("summary artifact path missing after initialization")?;
    let draft_route = client.resolve_draft_route().await?;
    let mut artifact = load_or_create_summary_artifact(
        client,
        artifact_store,
        &draft_route,
        state,
        &chapter,
        &artifact_path,
    )
    .await?;

    if artifact.save_summary_output.is_none() {
        ensure_summary_package_ready(client, artifact_store, &mut artifact, &artifact_path).await?;
        let package = artifact
            .package
            .as_ref()
            .context("summary artifact missing generated package")?;
        let save_output = client
            .save_summary(&SaveSummaryInput {
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
    Ok(format!(
        "Saved chapter summary for chapter {chapter_number}"
    ))
}

async fn run_checkpoint(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    start_chapter: i32,
    end_chapter: i32,
) -> Result<String> {
    let consistency = client
        .check_consistency(&CheckConsistencyInput {
            project_id: state.project_id.clone(),
            scope: ConsistencyScopeInput::chapter_range(
                state.book_number,
                start_chapter,
                state.book_number,
                end_chapter,
            ),
            checks: Vec::new(),
            severity_filter: vec![],
            deep_check: Some(true),
            subjects: vec![],
            format: None,
            budget_tokens: None,
        })
        .await
        .with_context(|| {
            format!("failed to run consistency check for chapters {start_chapter}-{end_chapter}")
        })?;

    let sampled_scene_ids = sample_checkpoint_scene_ids(state, start_chapter, end_chapter)?;
    let mut sampled_reviews = Vec::new();
    for scene_id in &sampled_scene_ids {
        let review = client
            .run_dual_persona_review(&RunDualPersonaReviewInput {
                project_id: state.project_id.clone(),
                branch_id: Some(state.active_branch_id.clone()),
                scene_id: scene_id.clone(),
                rounds: Some(2),
            })
            .await
            .with_context(|| format!("failed to review scene {scene_id} during checkpoint"))?;
        sampled_reviews.push(serde_json::to_value(review)?);
    }

    let pacing_overview = client
        .read_json_resource::<serde_json::Value>(format!(
            "bible://projects/{}/pacing/overview",
            state.project_id
        ))
        .await?;
    let chapter_summaries = client
        .read_json_resource::<serde_json::Value>(format!(
            "bible://projects/{}/chapter-summaries",
            state.project_id
        ))
        .await?;
    let narrative_promises = client
        .read_json_resource::<serde_json::Value>(format!(
            "bible://projects/{}/narrative-promises",
            state.project_id
        ))
        .await?;

    let report_path = ArtifactStore::checkpoint_relative_path(start_chapter, end_chapter);
    let save_point = client
        .create_save_point(&CreateSavePointInput {
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
    });
    state.last_checkpoint_end_chapter = end_chapter;
    state.save(state_path)?;

    artifact_store.save_json(
        &report_path,
        &CheckpointReportArtifact {
            version: 1,
            start_chapter,
            end_chapter,
            save_point: save_point.clone(),
            consistency: serde_json::to_value(consistency)?,
            sampled_reviews,
            pacing_overview,
            chapter_summaries,
            narrative_promises,
            sampled_scene_ids,
        },
    )?;

    Ok(format!(
        "Created checkpoint for chapters {start_chapter}-{end_chapter}; awaiting human review ({})",
        save_point.save_point_id
    ))
}

async fn load_or_create_scene_artifact(
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    draft_route: &DraftRouteBinding,
    state: &HarnessState,
    chapter: &crate::state::ChapterState,
    scene: &crate::state::SceneState,
    artifact_path: &str,
) -> Result<SceneGenerationArtifact> {
    let full_path = artifact_store.root().join(artifact_path);
    if full_path.exists() {
        let artifact: SceneGenerationArtifact = artifact_store.load_json(artifact_path)?;
        validate_scene_artifact_identity(&artifact, chapter.chapter_number, scene.scene_order)?;
        return Ok(artifact);
    }

    let prompt = build_scene_prompt(client, state, chapter, scene).await?;
    let artifact = SceneGenerationArtifact::new(
        chapter.chapter_number,
        scene.scene_order,
        draft_route.route_name.clone(),
        draft_route.agent_id.clone(),
        prompt,
    );
    artifact_store.save_json(artifact_path, &artifact)?;
    Ok(artifact)
}

async fn ensure_scene_package_ready(
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    scene: &crate::state::SceneState,
    artifact: &mut SceneGenerationArtifact,
    artifact_path: &str,
) -> Result<()> {
    if artifact.is_ready() {
        return Ok(());
    }

    for _ in 0..MAX_GENERATION_ROUNDS {
        if artifact.completion_fragments.is_empty() {
            let response = client
                .test_agent(&TestAgentInput {
                    agent_id: artifact.agent_id.clone(),
                    test_prompt: Some(artifact.prompt.clone()),
                })
                .await
                .context("draft generation failed on initial call")?;
            if response.route_name != artifact.route_name {
                anyhow::bail!(
                    "draft generation used route {} instead of expected {}",
                    response.route_name,
                    artifact.route_name
                );
            }
            artifact.adapter_kind = Some(response.adapter_kind);
            artifact.model_name = Some(response.model_name);
            artifact.completion_fragments.push(response.output);
            artifact.truncated = response.truncated;
            artifact_store.save_json(artifact_path, artifact)?;
        } else if artifact.truncated {
            let response = client
                .continue_generation(&ContinueGenerationInput {
                    route: artifact.route_name.clone(),
                    original_prompt: artifact.prompt.clone(),
                    prior_output: artifact.combined_output(),
                    rating: None,
                    project_id: None,
                    book_id: None,
                    chapter_id: None,
                    scene_id: None,
                })
                .await
                .context("draft generation continuation failed")?;
            artifact.completion_fragments.push(response.output);
            artifact.truncated = response.truncated;
            artifact_store.save_json(artifact_path, artifact)?;
        }

        if !artifact.truncated {
            let output = artifact.combined_output();
            match parse_model_json::<GeneratedScenePackage>(&output)
                .and_then(|package| validate_scene_package(&package, scene))
            {
                Ok(package) => {
                    artifact.package = Some(package);
                    artifact.last_parse_error = None;
                    artifact_store.save_json(artifact_path, artifact)?;
                    return Ok(());
                }
                Err(error) => {
                    artifact.last_parse_error = Some(error.to_string());
                    artifact_store.save_json(artifact_path, artifact)?;
                    return Err(error).with_context(|| {
                        format!(
                            "draft output for chapter {} scene {} was not valid scene JSON",
                            artifact.chapter_number, artifact.scene_order
                        )
                    });
                }
            }
        }
    }

    artifact_store.save_json(artifact_path, artifact)?;
    anyhow::bail!(
        "draft output for chapter {} scene {} is still truncated after {} rounds",
        artifact.chapter_number,
        artifact.scene_order,
        MAX_GENERATION_ROUNDS
    );
}

async fn load_or_create_summary_artifact(
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    draft_route: &DraftRouteBinding,
    state: &HarnessState,
    chapter: &crate::state::ChapterState,
    artifact_path: &str,
) -> Result<ChapterSummaryArtifact> {
    let full_path = artifact_store.root().join(artifact_path);
    if full_path.exists() {
        let artifact: ChapterSummaryArtifact = artifact_store.load_json(artifact_path)?;
        validate_summary_artifact_identity(&artifact, chapter.chapter_number)?;
        return Ok(artifact);
    }

    let prompt = build_summary_prompt(client, artifact_store, state, chapter).await?;
    let artifact = ChapterSummaryArtifact::new(
        chapter.chapter_number,
        draft_route.route_name.clone(),
        draft_route.agent_id.clone(),
        prompt,
    );
    artifact_store.save_json(artifact_path, &artifact)?;
    Ok(artifact)
}

async fn ensure_summary_package_ready(
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    artifact: &mut ChapterSummaryArtifact,
    artifact_path: &str,
) -> Result<()> {
    if artifact.is_ready() {
        return Ok(());
    }

    for _ in 0..MAX_GENERATION_ROUNDS {
        if artifact.completion_fragments.is_empty() {
            let response = client
                .test_agent(&TestAgentInput {
                    agent_id: artifact.agent_id.clone(),
                    test_prompt: Some(artifact.prompt.clone()),
                })
                .await
                .context("summary generation failed on initial call")?;
            if response.route_name != artifact.route_name {
                anyhow::bail!(
                    "summary generation used route {} instead of expected {}",
                    response.route_name,
                    artifact.route_name
                );
            }
            artifact.adapter_kind = Some(response.adapter_kind);
            artifact.model_name = Some(response.model_name);
            artifact.completion_fragments.push(response.output);
            artifact.truncated = response.truncated;
            artifact_store.save_json(artifact_path, artifact)?;
        } else if artifact.truncated {
            let response = client
                .continue_generation(&ContinueGenerationInput {
                    route: artifact.route_name.clone(),
                    original_prompt: artifact.prompt.clone(),
                    prior_output: artifact.combined_output(),
                    rating: None,
                    project_id: None,
                    book_id: None,
                    chapter_id: None,
                    scene_id: None,
                })
                .await
                .context("summary generation continuation failed")?;
            artifact.completion_fragments.push(response.output);
            artifact.truncated = response.truncated;
            artifact_store.save_json(artifact_path, artifact)?;
        }

        if !artifact.truncated {
            let output = artifact.combined_output();
            match parse_model_json::<GeneratedChapterSummaryPackage>(&output)
                .and_then(validate_summary_package)
            {
                Ok(package) => {
                    artifact.package = Some(package);
                    artifact.last_parse_error = None;
                    artifact_store.save_json(artifact_path, artifact)?;
                    return Ok(());
                }
                Err(error) => {
                    artifact.last_parse_error = Some(error.to_string());
                    artifact_store.save_json(artifact_path, artifact)?;
                    return Err(error).with_context(|| {
                        format!(
                            "chapter {} summary output was not valid JSON",
                            artifact.chapter_number
                        )
                    });
                }
            }
        }
    }

    artifact_store.save_json(artifact_path, artifact)?;
    anyhow::bail!(
        "chapter {} summary output is still truncated after {} rounds",
        artifact.chapter_number,
        MAX_GENERATION_ROUNDS
    );
}

async fn build_scene_prompt(
    client: &McpHarnessClient,
    state: &HarnessState,
    chapter: &crate::state::ChapterState,
    scene: &crate::state::SceneState,
) -> Result<String> {
    let briefing = client
        .get_chapter_briefing(&GetChapterBriefingInput {
            project_id: state.project_id.clone(),
            book_number: state.book_number,
            chapter_number: chapter.chapter_number,
            scene_order: Some(scene.scene_order),
            character_ids: scene.character_ids.clone(),
            location_id: Some(scene.location_id.clone()),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(CHAPTER_BRIEFING_TOKEN_BUDGET),
            recent_chapter_limit: Some(CHAPTER_BRIEFING_RECENT_LIMIT),
            token_budget: Some(CHAPTER_BRIEFING_TOKEN_BUDGET),
        })
        .await?;
    let scene_context = client
        .get_scene_context(&GetSceneContextInput {
            project_id: state.project_id.clone(),
            book_number: state.book_number,
            chapter_number: chapter.chapter_number,
            chapter_id: None,
            scene_order: scene.scene_order,
            character_ids: scene.character_ids.clone(),
            max_character_count: None,
            location_id: scene.location_id.clone(),
            format: Some(ContextFormat::Json),
            budget_tokens: Some(SCENE_CONTEXT_TOKEN_BUDGET),
            token_budget: Some(SCENE_CONTEXT_TOKEN_BUDGET),
            sections: None,
        })
        .await?;
    let scene_writer_skill = client
        .read_text_resource("bible://skills/scene-writer".to_string())
        .await
        .context("failed to load scene-writer skill resource")?;

    let directives = render_directives(&state.editorial_directives);
    let manifest_json = serde_json::to_string_pretty(&serde_json::json!({
        "book_number": state.book_number,
        "chapter_number": chapter.chapter_number,
        "chapter_synopsis": chapter.synopsis,
        "pov_character_id": chapter.pov_character_id,
        "scene_order": scene.scene_order,
        "character_ids": scene.character_ids,
        "location_id": scene.location_id,
        "content_rating": scene.content_rating,
        "target_tone": scene.tone,
        "source_path": scene.source_path,
    }))?;
    let scene_context_json = serde_json::to_string_pretty(&scene_context)?;

    Ok(format!(
        concat!(
            "Write exactly one scene for Spindle and return JSON only.\n\n",
            "Output schema:\n",
            "{{\n",
            "  \"full_text\": \"string\",\n",
            "  \"summary\": \"string\",\n",
            "  \"tone\": \"optional string\",\n",
            "  \"character_states\": [{{\"character_id\": \"...\", \"summary\": \"...\"}}],\n",
            "  \"canonical_facts\": [{{\"fact_type\": \"...\", \"key\": \"...\", \"value\": \"...\", \"context\": \"optional\"}}],\n",
            "  \"relationship_updates\": [{{\"character_a_id\": \"...\", \"character_b_id\": \"...\", \"trust_delta\": 0, \"tension_delta\": 0, \"reason\": \"...\"}}],\n",
            "  \"beats\": [{{\"beat_type\": \"...\", \"summary\": \"...\"}}],\n",
            "  \"continuity_notes\": [\"optional notes\"]\n",
            "}}\n\n",
            "Rules:\n",
            "- Return valid JSON only. No markdown fences. No prose outside the JSON object.\n",
            "- Use only the provided character ids and location id.\n",
            "- Preserve continuity from the chapter briefing and scene context.\n",
            "- Treat chapter briefing Continuity sheets as authoritative for character details, habits, voice, state, relationships, recent appearances, and location continuity.\n",
            "- Keep the scene aligned to the requested content rating and tone target.\n",
            "- Use empty arrays instead of null when you have no structured updates.\n\n",
            "Editorial directives:\n{directives}\n\n",
            "Scene manifest:\n{manifest_json}\n\n",
            "Chapter briefing markdown:\n{briefing_markdown}\n\n",
            "Scene context envelope:\n{scene_context_json}\n\n",
            "Scene-writer skill guidance:\n{scene_writer_skill}\n"
        ),
        directives = directives,
        manifest_json = manifest_json,
        briefing_markdown = briefing.briefing_markdown,
        scene_context_json = scene_context_json,
        scene_writer_skill = scene_writer_skill,
    ))
}

async fn build_summary_prompt(
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    state: &HarnessState,
    chapter: &crate::state::ChapterState,
) -> Result<String> {
    let first_scene = chapter
        .scenes
        .first()
        .context("chapter must contain at least one scene to summarize")?;
    let briefing = client
        .get_chapter_briefing(&GetChapterBriefingInput {
            project_id: state.project_id.clone(),
            book_number: state.book_number,
            chapter_number: chapter.chapter_number,
            scene_order: Some(first_scene.scene_order),
            character_ids: first_scene.character_ids.clone(),
            location_id: Some(first_scene.location_id.clone()),
            format: Some(ContextFormat::Markdown),
            budget_tokens: Some(CHAPTER_BRIEFING_TOKEN_BUDGET),
            recent_chapter_limit: Some(CHAPTER_BRIEFING_RECENT_LIMIT),
            token_budget: Some(CHAPTER_BRIEFING_TOKEN_BUDGET),
        })
        .await?;

    let scene_packages = chapter
        .scenes
        .iter()
        .map(|scene| -> Result<serde_json::Value> {
            let artifact_path = scene
                .scene_artifact_path
                .as_ref()
                .context("scene artifact path missing while building summary prompt")?;
            let artifact: SceneGenerationArtifact = artifact_store.load_json(artifact_path)?;
            let package = artifact.package.context(
                "scene artifact missing generated package while building summary prompt",
            )?;
            Ok(serde_json::json!({
                "scene_order": scene.scene_order,
                "summary": package.summary,
                "beats": package.beats,
                "continuity_notes": package.continuity_notes,
                "full_text": package.full_text,
            }))
        })
        .collect::<Result<Vec<_>>>()?;

    let directives = render_directives(&state.editorial_directives);
    let scene_packages_json = serde_json::to_string_pretty(&scene_packages)?;

    Ok(format!(
        concat!(
            "Summarize one completed chapter for Spindle and return JSON only.\n\n",
            "Output schema:\n",
            "{{\n",
            "  \"summary\": \"string\",\n",
            "  \"key_events\": [\"...\"],\n",
            "  \"character_changes\": [\"...\"],\n",
            "  \"relationship_shifts\": [\"...\"],\n",
            "  \"arc_advances\": [\"...\"],\n",
            "  \"promise_events\": [\"...\"]\n",
            "}}\n\n",
            "Rules:\n",
            "- Return valid JSON only. No markdown fences.\n",
            "- Cover only this chapter.\n",
            "- Prefer concrete continuity details over generic phrasing.\n",
            "- Use empty arrays instead of null.\n\n",
            "Editorial directives:\n{directives}\n\n",
            "Chapter synopsis:\n{synopsis}\n\n",
            "Chapter briefing markdown:\n{briefing_markdown}\n\n",
            "Scene packages:\n{scene_packages_json}\n"
        ),
        directives = directives,
        synopsis = chapter.synopsis,
        briefing_markdown = briefing.briefing_markdown,
        scene_packages_json = scene_packages_json,
    ))
}

fn ensure_scene_artifact_path(
    state: &mut HarnessState,
    state_path: &Path,
    chapter_index: usize,
    scene_index: usize,
) -> Result<()> {
    if state.chapters[chapter_index].scenes[scene_index]
        .scene_artifact_path
        .is_none()
    {
        let chapter_number = state.chapters[chapter_index].chapter_number;
        let scene_order = state.chapters[chapter_index].scenes[scene_index].scene_order;
        state.chapters[chapter_index].scenes[scene_index].scene_artifact_path = Some(
            ArtifactStore::scene_relative_path(chapter_number, scene_order),
        );
        state.save(state_path)?;
    }
    Ok(())
}

fn ensure_summary_artifact_path(
    state: &mut HarnessState,
    state_path: &Path,
    chapter_index: usize,
) -> Result<()> {
    if state.chapters[chapter_index]
        .summary_artifact_path
        .is_none()
    {
        let chapter_number = state.chapters[chapter_index].chapter_number;
        state.chapters[chapter_index].summary_artifact_path =
            Some(ArtifactStore::summary_relative_path(chapter_number));
        state.save(state_path)?;
    }
    Ok(())
}

fn scene_indices(
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

fn chapter_index(state: &HarnessState, chapter_number: i32) -> Option<usize> {
    state
        .chapters
        .iter()
        .position(|chapter| chapter.chapter_number == chapter_number)
}

fn resolve_artifacts_root(state_path: &Path, state: &HarnessState) -> PathBuf {
    let parent = state_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(&state.artifacts_dir)
}

fn render_directives(directives: &[String]) -> String {
    if directives.is_empty() {
        "- none".to_string()
    } else {
        directives
            .iter()
            .map(|directive| format!("- {directive}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn parse_model_json<T>(raw: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let trimmed = raw.trim();
    let candidate = if let Some(inner) = trimmed.strip_prefix("```json") {
        inner
            .trim()
            .strip_suffix("```")
            .map(str::trim)
            .unwrap_or(inner.trim())
    } else if let Some(inner) = trimmed.strip_prefix("```") {
        inner
            .trim()
            .strip_suffix("```")
            .map(str::trim)
            .unwrap_or(inner.trim())
    } else {
        trimmed
    };
    serde_json::from_str(candidate).context("model output was not valid JSON")
}

fn validate_scene_package(
    package: &GeneratedScenePackage,
    scene: &crate::state::SceneState,
) -> Result<GeneratedScenePackage> {
    if package.full_text.trim().is_empty() {
        anyhow::bail!(
            "generated package for scene {} has empty full_text",
            scene.scene_order
        );
    }
    if package.summary.trim().is_empty() {
        anyhow::bail!(
            "generated package for scene {} has empty summary",
            scene.scene_order
        );
    }
    let allowed_characters = scene
        .character_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    for entry in &package.character_states {
        if !allowed_characters.contains(entry.character_id.as_str()) {
            anyhow::bail!(
                "generated package references unknown character_id {} in character_states",
                entry.character_id
            );
        }
    }
    for entry in &package.relationship_updates {
        if !allowed_characters.contains(entry.character_a_id.as_str())
            || !allowed_characters.contains(entry.character_b_id.as_str())
        {
            anyhow::bail!(
                "generated package references unknown character ids in relationship_updates"
            );
        }
    }
    Ok(package.clone())
}

fn validate_summary_package(
    package: GeneratedChapterSummaryPackage,
) -> Result<GeneratedChapterSummaryPackage> {
    if package.summary.trim().is_empty() {
        anyhow::bail!("generated chapter summary has empty summary");
    }
    Ok(package)
}

fn validate_scene_artifact_identity(
    artifact: &SceneGenerationArtifact,
    chapter_number: i32,
    scene_order: i32,
) -> Result<()> {
    if artifact.chapter_number != chapter_number || artifact.scene_order != scene_order {
        anyhow::bail!(
            "scene artifact is for chapter {} scene {}, expected chapter {} scene {}",
            artifact.chapter_number,
            artifact.scene_order,
            chapter_number,
            scene_order
        );
    }
    Ok(())
}

fn validate_summary_artifact_identity(
    artifact: &ChapterSummaryArtifact,
    chapter_number: i32,
) -> Result<()> {
    if artifact.chapter_number != chapter_number {
        anyhow::bail!(
            "summary artifact is for chapter {}, expected chapter {}",
            artifact.chapter_number,
            chapter_number
        );
    }
    Ok(())
}

fn commit_output_has_errors(output: &spindle_core::models::CommitSceneChangesOutput) -> bool {
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

fn sample_checkpoint_scene_ids(
    state: &HarnessState,
    start_chapter: i32,
    end_chapter: i32,
) -> Result<Vec<String>> {
    let mut candidates = Vec::new();
    let selected_chapters = [
        start_chapter,
        start_chapter + ((end_chapter - start_chapter) / 2),
        end_chapter,
    ];
    let mut seen = BTreeSet::new();
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
