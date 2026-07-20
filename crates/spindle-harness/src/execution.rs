use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use spindle_core::models::{
    ActiveThreadSummary, AnnotateSceneBeatsInput, CheckConsistencyInput, CommitSceneChangesInput,
    ConsistencyScopeInput, ContextFormat, ContinueGenerationInput, CreateSavePointInput,
    GetChapterBriefingInput, GetSceneContextInput, NarrativePromiseDueSummary,
    ResearchPackForSceneInput, SaveSceneDraftInput, SaveSummaryInput, TestAgentInput,
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
const CHAPTER_BRIEFING_TOKEN_BUDGET: usize = 12_000;
const SCENE_CONTEXT_TOKEN_BUDGET: usize = 32_000;

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
        NextAction::AwaitResearch {
            chapter_number,
            scene_order,
            missing_tags,
            query,
            location,
        } => {
            let tags_str = if missing_tags.is_empty() {
                "none".to_string()
            } else {
                missing_tags.join(", ")
            };
            anyhow::bail!(
                "await_research: chapter {} scene {} needs research. \
                 Searched tags: [{}], query: \"{}\", location: \"{}\". \
                 Suggested action: use research tools (e.g. research_add_source, research_add_note, research_add_claim) to add relevant research, then resume.",
                chapter_number,
                scene_order,
                tags_str,
                query.as_deref().unwrap_or("none"),
                location.as_deref().unwrap_or("none")
            );
        }
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
        NextAction::VerifyScene {
            chapter_number,
            scene_order,
        } => {
            // Box::pin: this arm's future contributes its full size to the
            // enclosing executor future even when never executed; un-boxed it
            // pushes the C1 integration test past the default macOS test stack
            // (same failure class as the MineScene / ratify dispatch arms).
            Box::pin(verify_scene(
                state_path,
                &mut state,
                client,
                chapter_number,
                scene_order,
            ))
            .await?
        }
        NextAction::ReviseScene {
            chapter_number,
            scene_order,
            attempt,
        } => {
            // Box::pin: see VerifyScene above — the revise arm re-dispatches the
            // full draft path, so its future is large; box it to keep the
            // enclosing executor future off the default test stack.
            Box::pin(revise_scene(
                state_path,
                &mut state,
                client,
                &artifact_store,
                chapter_number,
                scene_order,
                attempt,
            ))
            .await?
        }
        NextAction::MineScene {
            chapter_number,
            scene_order,
        } => {
            // Box::pin: this arm's future contributes its full size to the
            // enclosing executor future even when never executed; un-boxed it
            // pushes the C1 integration test past the default macOS test
            // stack (same failure class as the ratify dispatch arms).
            Box::pin(mine_scene(
                state_path,
                &mut state,
                client,
                chapter_number,
                scene_order,
            ))
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
        NextAction::ReplanChapter { chapter_number } => {
            // Box::pin: this arm's future contributes its full size to the
            // enclosing executor future even when never executed; un-boxed it
            // pushes the C1 integration test past the default macOS test stack
            // (same failure class as the MineScene / verify / revise arms).
            Box::pin(replan_chapter(
                state_path,
                &mut state,
                client,
                chapter_number,
            ))
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
    let draft_route = client
        .resolve_draft_route(Some(scene.content_rating.clone()))
        .await?;
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
                // Persist the planned location so the next scene's pre-draft
                // temporal anchor can name where this scene ended. Empty → None
                // to avoid a dangling location reference.
                location_id: Some(scene.location_id.clone()).filter(|id| !id.is_empty()),
                generation_id: artifact.generation_id.clone(),
                research_source_ids: artifact.research_source_ids.clone(),
                research_note_ids: artifact.research_note_ids.clone(),
                research_claim_ids: artifact.research_claim_ids.clone(),
                research_query_pack_input: artifact.research_query_pack_input.clone(),
                research_context_hash: artifact.research_context_hash.clone(),
                // Automated drafting does not flag on-page reveals; the reveal
                // package is a host/interactive (authoring_save_scene_draft) flow.
                knowledge_learned: Vec::new(),
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
    let scene_id = scene
        .scene_id
        .clone()
        .context("scene_id missing for commit_scene_changes")?;
    let artifact_path = scene.scene_artifact_path.clone();
    let mut artifact = artifact_path
        .as_ref()
        .map(|path| artifact_store.load_json::<SceneGenerationArtifact>(path))
        .transpose()?;
    let mut commit_output = artifact
        .as_ref()
        .and_then(|artifact| artifact.commit_output.clone());

    if commit_output.is_none() {
        let (character_states, canonical_facts, relationship_updates) = artifact
            .as_ref()
            .and_then(|artifact| artifact.package.as_ref())
            .map(|package| {
                (
                    package.character_states.clone(),
                    package.canonical_facts.clone(),
                    package.relationship_updates.clone(),
                )
            })
            .unwrap_or_default();
        let new_commit_output = client
            .commit_scene_changes(&CommitSceneChangesInput {
                project_id: state.project_id.clone(),
                scene_id: scene_id.clone(),
                character_states,
                canonical_facts,
                relationship_updates,
                accept_world_rule_risks: true,
                // Surface continuity findings on the commit output without halting
                // the unattended batch; the supervisor inspects them at checkpoints.
                accept_continuity_risks: false,
                continuity_gate: Some(spindle_core::models::CommitContinuityGate::WarnOnly),
            })
            .await
            .with_context(|| {
                format!("failed to commit scene changes for chapter {chapter_number} scene {scene_order}")
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
    if commit_output_has_errors(commit_output) {
        let inspect_target = artifact_path
            .as_ref()
            .map(|path| artifact_store.root().join(path).display().to_string())
            .unwrap_or_else(|| scene_id.clone());
        let error_summary = commit_error_summary(commit_output);
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
    let scene_id = scene
        .scene_id
        .clone()
        .context("scene_id missing for annotate_scene_beats")?;
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
        let annotation_output = client
            .annotate_scene_beats(&AnnotateSceneBeatsInput {
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
                format!("failed to annotate beats for chapter {chapter_number} scene {scene_order}")
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
    Ok(format!(
        "Annotated beats for chapter {chapter_number} scene {scene_order}"
    ))
}

/// Canon-mining step (evolution §3.1). Invokes `mine_scene_canon` for the
/// committed scene through the same MCP client the commit/annotate steps use,
/// then records the outcome on the scene's `mine_status`/`mine_detail` and
/// advances the run regardless of outcome.
///
/// Mining never blocks a run (evolution I8): a `staged`, `skipped`,
/// `model_output_rejected`, OR a transport-level tool error all set a
/// `Some(_)` mine_status and let the loop proceed to beats. Honesty lives in
/// the recorded status/detail and the run-status render, not in blocking — a
/// blocked mine would strand an otherwise complete scene. The detail carries a
/// staged count or the skip/error reason and never carries prose.
async fn mine_scene(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    chapter_number: i32,
    scene_order: i32,
) -> Result<String> {
    let (chapter_index, scene_index) =
        scene_indices(state, chapter_number, scene_order).context("scene not found in state")?;
    let scene_id = state.chapters[chapter_index].scenes[scene_index]
        .scene_id
        .clone()
        .context("scene_id missing for mine_scene")?;

    let (status, detail) = match client
        .mine_scene_canon(&spindle_core::models::MineSceneCanonInput {
            project_id: state.project_id.clone(),
            scene_id: scene_id.clone(),
        })
        .await
    {
        Ok(output) => {
            let detail = match output.status.as_str() {
                "staged" => format!("staged {} delta(s)", output.staged.len()),
                _ => output
                    .skip_reason
                    .clone()
                    .unwrap_or_else(|| output.status.clone()),
            };
            (output.status, detail)
        }
        Err(error) => {
            // Transport-level tool error: record an honest `error` status and
            // advance anyway (mining never blocks the run — evolution I8).
            tracing::warn!(
                "mine_scene_canon for scene {scene_id} failed at transport; \
                 recording error and advancing: {error:#}"
            );
            (
                "error".to_string(),
                format!("mine transport error: {error}"),
            )
        }
    };

    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
    live_scene.mine_status = Some(status.clone());
    live_scene.mine_detail = Some(detail.clone());
    state.save(state_path)?;
    Ok(format!(
        "Mined canon for chapter {chapter_number} scene {scene_order} ({status}: {detail})"
    ))
}

/// Opt-in living-outline replan step (ADR 0003, evolution §3.5). Runs the
/// `replan_chapter` differ against the just-summarized chapter, comparing its
/// realized reality to the not-yet-drafted future chapters' plans and staging
/// amendment proposals for operator ratification (never auto-applied — ADR D5).
/// Records the outcome on the chapter's `replan_status`/`replan_detail` and
/// advances the run regardless of outcome.
///
/// Replan never blocks a run (evolution I8, mirroring mining): a transport-level
/// error records an honest `error` status and proceeds. Outcomes:
/// - `staged` — amendments persisted; detail carries the count.
/// - `skipped` / `no_targets` / `no_summary` — detail carries the reason.
/// - `error` — a transport failure; detail carries the error (prose-free).
async fn replan_chapter(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    chapter_number: i32,
) -> Result<String> {
    let chapter_index =
        chapter_index(state, chapter_number).context("chapter not found in state")?;

    let (status, detail) = match client
        .replan_chapter(&spindle_core::models::ReplanChapterInput {
            project_id: state.project_id.clone(),
            book_number: state.book_number,
            source_chapter: chapter_number,
        })
        .await
    {
        Ok(output) => {
            let detail = match output.status.as_str() {
                "staged" => format!("staged {} amendment(s)", output.staged.len()),
                _ => output
                    .skip_reason
                    .clone()
                    .unwrap_or_else(|| output.status.clone()),
            };
            (output.status, detail)
        }
        Err(error) => {
            // Transport-level tool error: record an honest `error` status and
            // advance anyway (replan never blocks the run — evolution I8).
            tracing::warn!(
                "replan_chapter for chapter {chapter_number} failed at transport; \
                 recording error and advancing: {error:#}"
            );
            (
                "error".to_string(),
                format!("replan transport error: {error}"),
            )
        }
    };

    let live_chapter = &mut state.chapters[chapter_index];
    live_chapter.replan_status = Some(status.clone());
    live_chapter.replan_detail = Some(detail.clone());
    state.save(state_path)?;
    Ok(format!(
        "Replanned future plans after chapter {chapter_number} ({status}: {detail})"
    ))
}

/// Maximum number of rendered `## Revision directives` lines carried into a
/// revision brief (evolution §3.2). Beyond this the block is truncated with an
/// explicit tail line, so the brief carries findings-not-prose and stays small.
const MAX_REVISION_DIRECTIVE_LINES: usize = 12;

/// The severity floor at or above which a verification finding drives a revision
/// (evolution §3.2: findings ≥ `warning`). `info` findings never trigger a
/// revise pass.
fn finding_is_actionable(severity: &str) -> bool {
    matches!(severity, "warning" | "error")
}

/// Run the scene-scoped deterministic check subset for one scene and return its
/// actionable (severity ≥ warning) findings as `(check_type, message)` pairs,
/// deterministically ordered.
///
/// The scope is pinned to exactly this scene (single-chapter range +
/// `scene_order`), so every finding the check returns is already attributed to
/// the scene — the same narrowing the scene-scoped verifier honors. `checks` is
/// the `SCENE_VERIFY_CHECKS` subset and `deep_check` is false, so this issues
/// **zero** model calls; it is a pure deterministic pass.
async fn scene_verify_findings(
    client: &McpHarnessClient,
    project_id: &str,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> Result<Vec<(String, String)>> {
    let scope = ConsistencyScopeInput {
        scene_order: Some(scene_order),
        ..ConsistencyScopeInput::chapter_range(
            book_number,
            chapter_number,
            book_number,
            chapter_number,
        )
    };
    let output = client
        .check_consistency(&CheckConsistencyInput {
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
        .filter(|issue| finding_is_actionable(&issue.severity))
        .map(|issue| (issue.check_type, issue.message))
        .collect();
    findings.sort();
    findings.dedup();
    Ok(findings)
}

/// Deterministic fingerprint of a warning-or-worse finding set (evolution §3.2
/// convergence guard).
///
/// The key is the sorted, de-duplicated set of `"{check_type}:{message}"` lines
/// joined by newlines and SHA-256'd to hex. Sorting makes the digest independent
/// of finding order; keying on `check_type` **and** the full message means an
/// identical finding (same check flagging the same scene id / same drifted
/// value — the messages embed those stable ids) across two verify rounds yields
/// an identical fingerprint, while any change to the finding set changes it. A
/// re-verify whose fingerprint matches the prior one is what parks a scene
/// instead of re-revising the same findings. An empty set never reaches here
/// (no findings ⇒ `clean`).
fn finding_fingerprint(findings: &[(String, String)]) -> String {
    let mut keys: Vec<String> = findings
        .iter()
        .map(|(check_type, message)| format!("{check_type}:{message}"))
        .collect();
    keys.sort();
    keys.dedup();
    let mut hasher = Sha256::new();
    hasher.update(keys.join("\n").as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Render a bounded `## Revision directives` block from the actionable findings.
/// Ids-not-prose: each line is the finding's `check_type` plus its own message
/// (which itself names scene ids / drifted values, not scene prose), capped at
/// `MAX_REVISION_DIRECTIVE_LINES` with an explicit truncation tail.
fn render_revision_directives(findings: &[(String, String)]) -> String {
    let mut block = String::from(
        "## Revision directives\n\
         The previous draft tripped these scene-scoped checks. Revise the scene to \
         resolve each finding, then return the same strict JSON package:\n",
    );
    for (check_type, message) in findings.iter().take(MAX_REVISION_DIRECTIVE_LINES) {
        block.push_str(&format!("- [{check_type}] {message}\n"));
    }
    if findings.len() > MAX_REVISION_DIRECTIVE_LINES {
        block.push_str(&format!(
            "- (+{} more finding(s) omitted; resolve the categories above)\n",
            findings.len() - MAX_REVISION_DIRECTIVE_LINES
        ));
    }
    block
}

/// In-run verification step (evolution §3.2). Runs the deterministic
/// scene-scoped check subset for a just-saved draft and records the outcome on
/// the scene's `verify_status`/`verify_detail`/`last_finding_fingerprint`,
/// advancing the run regardless of outcome.
///
/// Verify never blocks a run (evolution I8, mirroring mining): a transport-level
/// error records an honest `error` status and proceeds. Outcomes:
/// - no actionable findings → `clean`; the loop proceeds to commit.
/// - findings, budget remaining, fingerprint changed → `findings`; the
///   scheduler will schedule a `ReviseScene`.
/// - findings, budget remaining, fingerprint UNCHANGED from the prior round →
///   `parked_findings` ("unchanged after revision"): the convergence guard, so
///   the same findings are never re-revised.
/// - findings, budget exhausted → `parked_findings` with counts.
async fn verify_scene(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    chapter_number: i32,
    scene_order: i32,
) -> Result<String> {
    let (chapter_index, scene_index) =
        scene_indices(state, chapter_number, scene_order).context("scene not found in state")?;
    let book_number = state.book_number;
    let max = state.max_revise_attempts.unwrap_or(0);
    let revise_attempts = state.chapters[chapter_index].scenes[scene_index].revise_attempts;
    let prior_fingerprint = state.chapters[chapter_index].scenes[scene_index]
        .last_finding_fingerprint
        .clone();

    let (status, detail, fingerprint) = match scene_verify_findings(
        client,
        &state.project_id,
        book_number,
        chapter_number,
        scene_order,
    )
    .await
    {
        Ok(findings) if findings.is_empty() => (
            "clean".to_string(),
            "0 finding(s) at or above warning".to_string(),
            None,
        ),
        Ok(findings) => {
            let fingerprint = finding_fingerprint(&findings);
            let count = findings.len();
            if prior_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                // Convergence guard: the revision did not change the finding set,
                // so parking beats another identical revision pass.
                (
                    "parked_findings".to_string(),
                    "unchanged after revision".to_string(),
                    Some(fingerprint),
                )
            } else if revise_attempts >= max {
                // Budget spent on this finding set: park with counts.
                (
                    "parked_findings".to_string(),
                    format!("{count} finding(s) parked after {revise_attempts} revision(s)"),
                    Some(fingerprint),
                )
            } else {
                (
                    "findings".to_string(),
                    format!("{count} finding(s) at or above warning"),
                    Some(fingerprint),
                )
            }
        }
        Err(error) => {
            // Transport-level tool error: record an honest `error` status and
            // advance anyway (verify never blocks the run — evolution I8).
            tracing::warn!(
                "check_consistency for chapter {chapter_number} scene {scene_order} failed at \
                 transport; recording error and advancing: {error:#}"
            );
            (
                "error".to_string(),
                format!("verify transport error: {error}"),
                None,
            )
        }
    };

    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
    live_scene.verify_status = Some(status.clone());
    live_scene.verify_detail = Some(detail.clone());
    if let Some(fingerprint) = fingerprint {
        live_scene.last_finding_fingerprint = Some(fingerprint);
    }
    state.save(state_path)?;
    Ok(format!(
        "Verified chapter {chapter_number} scene {scene_order} ({status}: {detail})"
    ))
}

/// In-run revision step (evolution §3.2), agent mode. Re-dispatches the scene
/// through the SAME draft route it originally used, appending a bounded
/// `## Revision directives` block rendered from the current actionable findings.
/// On a successful re-save the scene's `revise_attempts` increments and its
/// `verify_status` resets to `None` so the scheduler re-runs `VerifyScene`.
///
/// The chokepoint is inherited: `draft_scene` resolves the draft route at the
/// scene's rating exactly as the first draft did, so the explicit-offload
/// guarantee holds with zero new configuration (evolution I3).
async fn revise_scene(
    state_path: &Path,
    state: &mut HarnessState,
    client: &McpHarnessClient,
    artifact_store: &ArtifactStore,
    chapter_number: i32,
    scene_order: i32,
    attempt: i32,
) -> Result<String> {
    let (chapter_index, scene_index) =
        scene_indices(state, chapter_number, scene_order).context("scene not found in state")?;
    let book_number = state.book_number;

    // Re-run the deterministic scene check to render the current directives.
    // The check is deterministic and prose-free, so this is a stable re-read of
    // exactly the findings the prior verify saw.
    let findings = scene_verify_findings(
        client,
        &state.project_id,
        book_number,
        chapter_number,
        scene_order,
    )
    .await
    .unwrap_or_default();
    let directives = render_revision_directives(&findings);

    // Reset the scene artifact so the draft path rebuilds the prompt with the
    // appended revision directives, then re-run the same draft route. Removing
    // the artifact + resetting the phase makes the re-dispatch idempotent and
    // reuses the existing draft plumbing wholesale (evolution §3.2: "same draft
    // path").
    if let Some(artifact_rel) = state.chapters[chapter_index].scenes[scene_index]
        .scene_artifact_path
        .clone()
    {
        let full = artifact_store.root().join(&artifact_rel);
        let _ = std::fs::remove_file(&full);
    }
    {
        let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
        live_scene.phase = ScenePhase::Pending;
        live_scene.revision_directives = Some(directives);
        state.save(state_path)?;
    }

    // Re-dispatch through the same draft route/rating (chokepoint inherited).
    draft_scene(
        state_path,
        state,
        client,
        artifact_store,
        chapter_number,
        scene_order,
    )
    .await?;

    // Successful re-save: increment the attempt count, clear the directives, and
    // reset verify_status so the scheduler re-verifies the revised draft.
    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
    live_scene.revise_attempts += 1;
    live_scene.verify_status = None;
    live_scene.verify_detail = None;
    live_scene.revision_directives = None;
    state.save(state_path)?;
    Ok(format!(
        "Revised chapter {chapter_number} scene {scene_order} (attempt {attempt})"
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
    let draft_route = client.resolve_draft_route(None).await?;
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
            deep_check: Some(false),
            subjects: vec![],
            format: None,
            budget_tokens: None,
        })
        .await
        .with_context(|| {
            format!("failed to run consistency check for chapters {start_chapter}-{end_chapter}")
        })?;

    let sampled_scene_ids = sample_checkpoint_scene_ids(state, start_chapter, end_chapter)?;

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
            // (evolution §3.6). The report is created with no section; the
            // automation adds it after the sampled reviews and before the
            // verdict. A manual-policy run never populates it.
            reader_sim: None,
        },
    )?;

    Ok(format!(
        "Created checkpoint for chapters {start_chapter}-{end_chapter}; awaiting deep consistency and sampled dual-persona review ({}) before operator checkpoint review. {} {}",
        save_point.save_point_id, deep_consistency_instruction, sampled_review_instruction
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

    let scene_summary = briefing.chapter_plan.as_ref().and_then(|plan| {
        plan.scenes
            .iter()
            .find(|s| s.scene_order == scene.scene_order)
            .map(|s| s.summary.clone())
    });

    let research_input = ResearchPackForSceneInput {
        project_id: state.project_id.clone(),
        branch_id: Some(state.active_branch_id.clone()),
        scene_summary,
        scene_location: Some(scene.location_id.clone()),
        character_ids: scene.character_ids.clone(),
        tags: scene.research_tags.clone(),
        explicit_query: scene.explicit_query.clone(),
        limit: Some(10),
    };

    let pack = client
        .research_pack_for_scene(&research_input)
        .await
        .with_context(|| {
            format!(
                "failed to retrieve research pack for chapter {} scene {}",
                chapter.chapter_number, scene.scene_order
            )
        })?;

    let mut stable_sources = pack.sources.clone();
    stable_sources.sort_by(|a, b| a.id.cmp(&b.id));
    let mut stable_notes = pack.notes.clone();
    stable_notes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut stable_claims = pack.claims.clone();
    stable_claims.sort_by(|a, b| a.id.cmp(&b.id));

    let stable_pack = serde_json::json!({
        "sources": stable_sources,
        "notes": stable_notes,
        "claims": stable_claims,
    });
    let serialized = serde_json::to_string(&stable_pack).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let hash_result = hasher.finalize();
    let context_hash = format!("{:x}", hash_result);

    let source_ids = stable_sources
        .iter()
        .map(|s| s.id.clone())
        .collect::<Vec<_>>();
    let note_ids = stable_notes
        .iter()
        .map(|n| n.id.clone())
        .collect::<Vec<_>>();
    let claim_ids = stable_claims
        .iter()
        .map(|c| c.id.clone())
        .collect::<Vec<_>>();
    let query_pack_input_str = serde_json::to_string(&research_input).ok();

    let mut prompt = build_scene_prompt(
        client,
        state,
        chapter,
        scene,
        &briefing,
        Some(&pack),
        draft_route,
    )
    .await?;
    // In-run revision (evolution §3.2): when this scene is being re-dispatched
    // by the revise step, append the bounded revision-directives block so the
    // same draft route reworks the flagged findings. Ids-not-prose; cleared on
    // a successful re-save by `revise_scene`.
    if let Some(directives) = scene.revision_directives.as_deref() {
        prompt.push_str("\n\n");
        prompt.push_str(directives);
    }
    let mut artifact = SceneGenerationArtifact::new(
        chapter.chapter_number,
        scene.scene_order,
        draft_route.route_name.clone(),
        draft_route.agent_id.clone(),
        draft_route.rating.clone(),
        prompt,
    );
    artifact.research_source_ids = source_ids;
    artifact.research_note_ids = note_ids;
    artifact.research_claim_ids = claim_ids;
    artifact.research_query_pack_input = query_pack_input_str;
    artifact.research_context_hash = Some(context_hash);
    artifact.research_sources = stable_sources;
    artifact.research_notes = stable_notes;
    artifact.research_claims = stable_claims;

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
                    route: Some(artifact.route_name.clone()),
                    rating: artifact.rating.clone(),
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
            artifact.generation_id = response.generation_id;
            artifact.generation_agent_id = response.generation_agent_id;
            artifact.generation_output_sha256 = response.generation_output_sha256;
            artifact.completion_fragments.push(response.output);
            artifact.truncated = response.truncated;
            artifact_store.save_json(artifact_path, artifact)?;
        } else if artifact.truncated {
            let response = client
                .continue_generation(&ContinueGenerationInput {
                    route: artifact.route_name.clone(),
                    original_prompt: artifact.prompt.clone(),
                    prior_output: artifact.combined_output(),
                    rating: artifact.rating.clone(),
                    project_id: None,
                    book_id: None,
                    chapter_id: None,
                    scene_id: None,
                })
                .await
                .context("draft generation continuation failed")?;
            artifact.generation_id = response.generation_id;
            artifact.generation_agent_id = response.generation_agent_id;
            artifact.generation_output_sha256 = response.generation_output_sha256;
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
                    route: Some(artifact.route_name.clone()),
                    rating: None,
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
    briefing: &spindle_core::models::GetChapterBriefingOutput,
    research_pack: Option<&spindle_core::models::ResearchPackForSceneOutput>,
    draft_route: &DraftRouteBinding,
) -> Result<String> {
    if draft_route.caller_should_send_brief {
        // Pull path: the drafting agent fetches its own full canon via MCP, so
        // the host does NOT embed scene context here. It does, however, run a
        // deliberately minimal `get_scene_context` fetch naming only the two
        // threads sections (see `pull_path_threads_sections`) so the shared
        // "## Threads to advance" block carries real data instead of empty
        // slices — resolving the T-110 host/pull asymmetry. The fetch is
        // best-effort: on error the block is simply omitted and the brief still
        // builds (the agent pulls its own context regardless). No retry.
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
                sections: Some(pull_path_threads_sections()),
            })
            .await;
        let (active_threads, promises_due) = pull_path_threads_from_result(scene_context);
        return build_scene_mcp_pull_prompt(
            state,
            chapter,
            scene,
            research_pack,
            &active_threads,
            &promises_due,
        );
    }

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

    let mut research_section = String::new();
    if let Some(pack) = research_pack
        .filter(|p| !p.sources.is_empty() || !p.notes.is_empty() || !p.claims.is_empty())
    {
        research_section.push_str("Research materials (use these facts as reference, avoiding fabrication or unsupported claims):\n");
        if !pack.sources.is_empty() {
            research_section.push_str("### Sources:\n");
            for s in &pack.sources {
                research_section.push_str(&format!(
                    "- ID: {}\n  Title: {}\n  Type: {}\n  Author: {}\n  Reliability: {}\n  Summary: {}\n",
                    s.id,
                    s.title,
                    s.source_type,
                    s.author.as_deref().unwrap_or("Unknown"),
                    s.reliability,
                    s.summary.as_deref().unwrap_or("No summary available")
                ));
            }
        }
        if !pack.notes.is_empty() {
            research_section.push_str("### Notes:\n");
            for n in &pack.notes {
                research_section.push_str(&format!(
                    "- ID: {}\n  Source ID: {}\n  Note: {}\n  Quote: {}\n  Tags: [{}]\n",
                    n.id,
                    n.source_id.as_deref().unwrap_or("None"),
                    n.note,
                    n.quote.as_deref().unwrap_or("None"),
                    n.tags.join(", ")
                ));
            }
        }
        if !pack.claims.is_empty() {
            research_section.push_str("### Claims:\n");
            for c in &pack.claims {
                research_section.push_str(&format!(
                    "- ID: {}\n  Claim: {}\n  Source ID: {}\n  Note ID: {}\n  Topic: {}\n  Confidence: {}\n  Tags: [{}]\n",
                    c.id,
                    c.claim,
                    c.source_id.as_deref().unwrap_or("None"),
                    c.note_id.as_deref().unwrap_or("None"),
                    c.topic.as_deref().unwrap_or("None"),
                    c.confidence,
                    c.tags.join(", ")
                ));
            }
        }
        research_section.push('\n');
    }

    assemble_host_scene_prompt(
        state,
        chapter,
        scene,
        &briefing.briefing_markdown,
        &scene_context,
        &scene_writer_skill,
        research_section,
    )
}

/// Assemble the host-embedded mega-prompt from data already in hand. Pure and
/// synchronous so it is unit-testable without a live MCP client. The
/// threads-to-advance directive block is derived from the scene context's
/// novel layer (`active_threads` + `narrative_promises_due`) via the shared
/// [`format_threads_to_advance`] formatter and inserted immediately after the
/// editorial directives and before the scene manifest JSON.
fn assemble_host_scene_prompt(
    state: &HarnessState,
    chapter: &crate::state::ChapterState,
    scene: &crate::state::SceneState,
    briefing_markdown: &str,
    scene_context: &crate::mcp::SceneContextEnvelope,
    scene_writer_skill: &str,
    research_section: String,
) -> Result<String> {
    let directives = render_directives(&state.editorial_directives);
    let threads_section = format_threads_to_advance(
        &scene_context.novel.active_threads,
        &scene_context.novel.narrative_promises_due,
    )
    .map(|block| format!("{block}\n\n"))
    .unwrap_or_default();
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
            "{threads_section}",
            "Scene manifest:\n{manifest_json}\n\n",
            "{research_section}",
            "Chapter briefing markdown:\n{briefing_markdown}\n\n",
            "Scene context envelope:\n{scene_context_json}\n\n",
            "Scene-writer skill guidance:\n{scene_writer_skill}\n"
        ),
        directives = directives,
        threads_section = threads_section,
        manifest_json = manifest_json,
        research_section = research_section,
        briefing_markdown = briefing_markdown,
        scene_context_json = scene_context_json,
        scene_writer_skill = scene_writer_skill,
    ))
}

fn build_scene_mcp_pull_prompt(
    state: &HarnessState,
    chapter: &crate::state::ChapterState,
    scene: &crate::state::SceneState,
    research_pack: Option<&spindle_core::models::ResearchPackForSceneOutput>,
    active_threads: &[ActiveThreadSummary],
    promises_due: &[NarrativePromiseDueSummary],
) -> Result<String> {
    let directives = render_directives(&state.editorial_directives);
    let threads_section = format_threads_to_advance(active_threads, promises_due)
        .map(|block| format!("{block}\n\n"))
        .unwrap_or_default();
    let research_tags = if scene.research_tags.is_empty() {
        "none".to_string()
    } else {
        scene.research_tags.join(", ")
    };
    let research_counts = research_pack
        .map(|pack| {
            format!(
                "{} sources, {} notes, {} claims",
                pack.sources.len(),
                pack.notes.len(),
                pack.claims.len()
            )
        })
        .unwrap_or_else(|| "not requested".to_string());
    let manifest_json = serde_json::to_string_pretty(&serde_json::json!({
        "project_id": state.project_id.clone(),
        "book_number": state.book_number,
        "chapter_number": chapter.chapter_number,
        "chapter_synopsis": chapter.synopsis.clone(),
        "pov_character_id": chapter.pov_character_id.clone(),
        "scene_order": scene.scene_order,
        "character_ids": scene.character_ids.clone(),
        "location_id": scene.location_id.clone(),
        "content_rating": scene.content_rating.clone(),
        "target_tone": scene.tone.clone(),
        "source_path": scene.source_path.clone(),
        "research_required": scene.research_required,
        "research_tags": scene.research_tags.clone(),
        "explicit_query": scene.explicit_query.clone(),
    }))?;

    Ok(format!(
        concat!(
            "You are drafting fiction for a book project through Spindle. This is not real-life advice, targeting, or activity planning.\n",
            "Write exactly one scene and return JSON only. No markdown fences. No prose outside the JSON object.\n\n",
            "Use the Spindle MCP server/resources to pull the context you need instead of relying on this prompt to embed canon.\n",
            "Required context pulls before drafting:\n",
            "- Call set_active_project for the project_id in the scene manifest if your MCP session is not already scoped.\n",
            "- Call get_chapter_briefing for this project/book/chapter/scene.\n",
            "- Call get_scene_context for this exact scene, including the listed character_ids and location_id.\n",
            "- Read bible://skills/scene-writer if your local skill profile has not already loaded it.\n",
            "- If research_required is true or research_tags are present, call research_pack_for_scene for this exact scene and use only supported research claims.\n\n",
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
            "- Use only the provided character ids and location id unless the pulled canon clearly requires another referenced entity.\n",
            "- Preserve continuity from Spindle canon, prior chapter summaries, scene context, and research claims.\n",
            "- Keep the scene aligned to the requested content rating and target tone.\n",
            "- Use empty arrays instead of null when you have no structured updates.\n\n",
            "Editorial directives:\n{directives}\n\n",
            "{threads_section}",
            "Scene manifest:\n{manifest_json}\n\n",
            "Research cue: required={research_required}; tags=[{research_tags}]; explicit_query={explicit_query}; preflight pack={research_counts}\n"
        ),
        directives = directives,
        threads_section = threads_section,
        manifest_json = manifest_json,
        research_required = scene.research_required.unwrap_or(false),
        research_tags = research_tags,
        explicit_query = scene.explicit_query.as_deref().unwrap_or("none"),
        research_counts = research_counts,
    ))
}

/// Section names the pull path requests from `get_scene_context` so the
/// "## Threads to advance" block can carry real data instead of empty slices
/// (fixes the T-110 host/pull asymmetry). Only `narrative_promises_due` is an
/// individually-gated novel section in the service; `active_threads` is
/// ungated (always hydrated whenever the chapter has a plan), so naming it is
/// harmless intent-documentation that does not enlarge the payload. Naming
/// exactly these two — and no other section — keeps the fetch minimal.
fn pull_path_threads_sections() -> Vec<String> {
    vec![
        "active_threads".to_string(),
        "narrative_promises_due".to_string(),
    ]
}

/// Project a `get_scene_context` result into the `(active_threads,
/// promises_due)` slices the pull prompt's threads block consumes. On success
/// the scene context's novel layer is unpacked; on error the block must never
/// fail the brief (the drafting agent will pull full context itself), so this
/// logs at warn level via the existing tracing pattern and yields empty
/// slices. No retry.
fn pull_path_threads_from_result(
    result: Result<crate::mcp::SceneContextEnvelope>,
) -> (Vec<ActiveThreadSummary>, Vec<NarrativePromiseDueSummary>) {
    match result {
        Ok(context) => (
            context.novel.active_threads,
            context.novel.narrative_promises_due,
        ),
        Err(error) => {
            tracing::warn!(
                "pull-path scene-context fetch for threads block failed; \
                 proceeding without it: {error:#}"
            );
            (Vec::new(), Vec::new())
        }
    }
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

    let mut scene_packages = Vec::new();
    for scene in &chapter.scenes {
        if let Some(artifact_path) = scene.scene_artifact_path.as_ref() {
            let artifact: SceneGenerationArtifact = artifact_store.load_json(artifact_path)?;
            if let Some(package) = artifact.package {
                scene_packages.push(serde_json::json!({
                    "scene_order": scene.scene_order,
                    "summary": package.summary,
                    "beats": package.beats,
                    "continuity_notes": package.continuity_notes,
                    "full_text": package.full_text,
                }));
                continue;
            }
        }

        let scene_id = scene.scene_id.as_ref().with_context(|| {
            format!(
                "chapter {} scene {} has no artifact package and no scene_id",
                chapter.chapter_number, scene.scene_order
            )
        })?;
        let persisted: serde_json::Value = client
            .read_json_resource(format!("bible://{scene_id}"))
            .await
            .with_context(|| format!("failed to read persisted scene resource {scene_id}"))?;
        scene_packages.push(serde_json::json!({
            "scene_order": scene.scene_order,
            "summary": persisted.get("summary").and_then(|value| value.as_str()).unwrap_or(""),
            "beats": [],
            "continuity_notes": [],
            "full_text": persisted.get("full_text").and_then(|value| value.as_str()).unwrap_or(""),
        }));
    }

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

/// Maximum number of item lines the "Threads to advance" directive block may
/// carry before it collapses the remainder into a single overflow note.
const THREADS_TO_ADVANCE_MAX_LINES: usize = 8;
/// Char-boundary-safe truncation width applied to every directive-block line.
const THREADS_TO_ADVANCE_LINE_WIDTH: usize = 160;

/// Build the "## Threads to advance" directive block shared by both agent-brief
/// builders (host-assembled and MCP-pull). Active threads render first in their
/// existing order, then promises whose urgency is `due`, `overdue`, or `soon`
/// (in that order); `watch`/`resolved` promises are excluded. The block caps at
/// [`THREADS_TO_ADVANCE_MAX_LINES`] item lines and, when more candidates exist,
/// appends exactly one `+N more in context` line. Every line is truncated to
/// [`THREADS_TO_ADVANCE_LINE_WIDTH`] chars on a char boundary.
///
/// Returns `None` when there are zero active threads AND zero due/overdue/soon
/// promises, so the caller omits the header entirely rather than emitting an
/// empty block.
fn format_threads_to_advance(
    active_threads: &[ActiveThreadSummary],
    promises_due: &[NarrativePromiseDueSummary],
) -> Option<String> {
    let promise_order = |urgency: &str| match urgency {
        "due" => Some(0u8),
        "overdue" => Some(1u8),
        "soon" => Some(2u8),
        _ => None,
    };

    let mut candidates: Vec<String> = Vec::new();
    for thread in active_threads {
        candidates.push(format!(
            "- [{}] {} — {}",
            thread.kind, thread.name, thread.statement
        ));
    }
    let mut relevant_promises: Vec<&NarrativePromiseDueSummary> = promises_due
        .iter()
        .filter(|promise| promise_order(promise.urgency.as_str()).is_some())
        .collect();
    // Stable sort by urgency rank (due < overdue < soon); ties keep source order.
    relevant_promises
        .sort_by_key(|promise| promise_order(promise.urgency.as_str()).unwrap_or(u8::MAX));
    for promise in relevant_promises {
        candidates.push(format!(
            "- [promise/{}] {} ({})",
            promise.urgency, promise.description, promise.urgency
        ));
    }

    if candidates.is_empty() {
        return None;
    }

    let total = candidates.len();
    let mut lines: Vec<String> = candidates
        .into_iter()
        .take(THREADS_TO_ADVANCE_MAX_LINES)
        .map(|line| truncate_on_char_boundary(&line, THREADS_TO_ADVANCE_LINE_WIDTH))
        .collect();
    if total > THREADS_TO_ADVANCE_MAX_LINES {
        lines.push(format!(
            "+{} more in context",
            total - THREADS_TO_ADVANCE_MAX_LINES
        ));
    }

    let mut block = String::from("## Threads to advance\n");
    block.push_str(&lines.join("\n"));
    Some(block)
}

/// Truncate `input` to at most `max_chars` characters, never splitting a
/// multibyte char. Returns the input unchanged when it already fits.
fn truncate_on_char_boundary(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect()
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

fn commit_error_summary(output: &spindle_core::models::CommitSceneChangesOutput) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::SceneContextEnvelope;
    use crate::state::{ChapterState, ChapterStatus, SceneState};
    use spindle_core::models::{
        ActiveThreadSummary, AgencyCheckSummary, ContentRating, LocationSummary,
        NarrativePromiseDueSummary, ReaderContract, SceneContextBudgetMeta, SceneContextNovelLayer,
        SceneContextSceneLayer, StoryPlacement, WorldStateSummary,
    };

    fn sample_state() -> HarnessState {
        HarnessState {
            project_id: "project:test".to_string(),
            active_branch_id: "branch:main".to_string(),
            book_number: 1,
            range: crate::state::ChapterRange {
                start_chapter: 1,
                end_chapter: 1,
            },
            checkpoint_interval: 1,
            last_checkpoint_end_chapter: 0,
            artifacts_dir: "artifacts".to_string(),
            editorial_directives: vec!["Keep the voice sharp.".to_string()],
            mining_policy: None,
            max_revise_attempts: None,
            checkpoint_policy: None,
            replan_policy: None,
            chapters: Vec::new(),
            checkpoint_history: Vec::new(),
        }
    }

    fn sample_chapter() -> ChapterState {
        ChapterState {
            chapter_number: 1,
            planned: true,
            synopsis: "A first turn in the casino.".to_string(),
            pov_character_id: Some("character:dave".to_string()),
            status: ChapterStatus::Pending,
            scenes: Vec::new(),
            summary_saved: false,
            summary_artifact_path: None,
            replan_status: None,
            replan_detail: None,
        }
    }

    fn sample_scene() -> SceneState {
        SceneState {
            scene_order: 1,
            character_ids: vec!["character:dave".to_string(), "character:ricky".to_string()],
            location_id: "location:vegas-strip".to_string(),
            content_rating: ContentRating::Mature,
            tone: Some("tense comic dread".to_string()),
            source_path: None,
            phase: ScenePhase::Pending,
            scene_id: None,
            scene_artifact_path: None,
            draft_diagnostics: None,
            blocked_reason: None,
            research_required: Some(true),
            research_tags: vec!["1970s-vegas".to_string()],
            explicit_query: None,
            research_pack_empty: false,
            research_tags_matched: true,
            mine_status: None,
            mine_detail: None,
            verify_status: None,
            verify_detail: None,
            revise_attempts: 0,
            last_finding_fingerprint: None,
            revision_directives: None,
        }
    }

    fn thread(kind: &str, name: &str, statement: &str) -> ActiveThreadSummary {
        ActiveThreadSummary {
            id: format!("{kind}:{name}"),
            kind: kind.to_string(),
            name: name.to_string(),
            statement: statement.to_string(),
            status: String::new(),
            next_expectation: None,
        }
    }

    fn promise(description: &str, urgency: &str) -> NarrativePromiseDueSummary {
        NarrativePromiseDueSummary {
            narrative_promise_id: format!("promise:{description}"),
            promise_type: "setup".to_string(),
            description: description.to_string(),
            status: "open".to_string(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: None,
            urgency: urgency.to_string(),
            chapters_since_plant: 2,
            notes: Vec::new(),
        }
    }

    fn sample_envelope(
        active_threads: Vec<ActiveThreadSummary>,
        promises_due: Vec<NarrativePromiseDueSummary>,
    ) -> SceneContextEnvelope {
        SceneContextEnvelope {
            standards: "house standards".to_string(),
            novel: SceneContextNovelLayer {
                reader_contract: ReaderContract {
                    promise: "a tense casino noir".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
                style_directive: None,
                world_rules: Vec::new(),
                subjects: Vec::new(),
                active_style_profile_id: None,
                system_overlays: Vec::new(),
                timeline_briefing: Vec::new(),
                future_knowledge_briefing: Vec::new(),
                pacing_directives: Vec::new(),
                realized_intensity_trend: None,
                narrative_promises_due: promises_due,
                knowledge_briefing: Vec::new(),
                semantic_references: Vec::new(),
                economy_briefing: Vec::new(),
                active_threads,
                previous_scene_tail: None,
            },
            scene: SceneContextSceneLayer {
                location: LocationSummary {
                    location_id: "location:vegas-strip".to_string(),
                    name: "The Strip".to_string(),
                    kind: "district".to_string(),
                    realm: None,
                    summary: "Neon and desperation.".to_string(),
                },
                world_state: WorldStateSummary {
                    controlling_faction: None,
                    status: None,
                    prosperity: None,
                    stability: None,
                    threat_level: None,
                    sensory_details: Vec::new(),
                },
                characters: Vec::new(),
                relationships: Vec::new(),
                agency_check: AgencyCheckSummary {
                    protagonist_character_id: None,
                    scenes_since_active_choice: 0,
                    needs_active_choice: false,
                    warning: None,
                },
            },
            budget: SceneContextBudgetMeta {
                estimated_tokens: 0,
                token_budget: None,
                novel_layer_truncated: false,
            },
        }
    }

    /// Pull the "## Threads to advance" block (header through the last consecutive
    /// bullet line) out of an assembled prompt, for byte-equality assertions.
    fn extract_threads_block(prompt: &str) -> Option<String> {
        let start = prompt.find("## Threads to advance")?;
        let tail = &prompt[start..];
        let mut end = tail.len();
        let mut seen_header = false;
        for (offset, line) in line_spans(tail) {
            if !seen_header {
                seen_header = true;
                continue;
            }
            if line.starts_with("- ") || line.starts_with("+") {
                continue;
            }
            end = offset;
            break;
        }
        Some(tail[..end].trim_end().to_string())
    }

    /// Yield (byte offset of line start, line without trailing newline).
    fn line_spans(text: &str) -> Vec<(usize, &str)> {
        let mut spans = Vec::new();
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            spans.push((offset, line.trim_end_matches('\n')));
            offset += line.len();
        }
        spans
    }

    #[test]
    fn threads_block_absent_when_no_threads_and_no_due_promises() {
        // Watch-urgency promises must not trigger the block.
        let promises = vec![promise("A slow-burn romance", "watch")];
        assert!(format_threads_to_advance(&[], &promises).is_none());
        assert!(format_threads_to_advance(&[], &[]).is_none());
    }

    #[test]
    fn host_prompt_carries_threads_block_after_directives_before_manifest() {
        let state = sample_state();
        let chapter = sample_chapter();
        let scene = sample_scene();
        let envelope = sample_envelope(
            vec![
                thread("theme", "Sunk Cost", "Every chip doubles the lie."),
                thread("conflict", "Dave vs the House", "The pit boss is onto him."),
            ],
            vec![promise("Ricky's debt comes due", "due")],
        );

        let prompt = assemble_host_scene_prompt(
            &state,
            &chapter,
            &scene,
            "chapter briefing markdown",
            &envelope,
            "scene-writer skill text",
            String::new(),
        )
        .expect("host prompt assembles");

        assert!(prompt.contains("## Threads to advance"));
        assert!(prompt.contains("Sunk Cost"));
        assert!(prompt.contains("Dave vs the House"));
        assert!(prompt.contains("Ricky's debt comes due"));

        let block_pos = prompt.find("## Threads to advance").unwrap();
        let directives_pos = prompt.find("Editorial directives:").unwrap();
        let manifest_pos = prompt.find("Scene manifest:").unwrap();
        assert!(
            directives_pos < block_pos,
            "block must follow the editorial directives"
        );
        assert!(
            block_pos < manifest_pos,
            "block must precede the scene manifest JSON"
        );
    }

    #[test]
    fn mcp_pull_prompt_carries_threads_block_after_directives_before_manifest() {
        let state = sample_state();
        let chapter = sample_chapter();
        let scene = sample_scene();
        let active_threads = vec![
            thread("theme", "Sunk Cost", "Every chip doubles the lie."),
            thread("conflict", "Dave vs the House", "The pit boss is onto him."),
        ];
        let promises = vec![promise("Ricky's debt comes due", "due")];

        let prompt =
            build_scene_mcp_pull_prompt(&state, &chapter, &scene, None, &active_threads, &promises)
                .expect("pull prompt assembles");

        assert!(prompt.contains("## Threads to advance"));
        assert!(prompt.contains("Sunk Cost"));
        assert!(prompt.contains("Ricky's debt comes due"));

        let block_pos = prompt.find("## Threads to advance").unwrap();
        let directives_pos = prompt.find("Editorial directives:").unwrap();
        let manifest_pos = prompt.find("Scene manifest:").unwrap();
        assert!(directives_pos < block_pos);
        assert!(block_pos < manifest_pos);
    }

    #[test]
    fn threads_block_absent_from_both_paths_when_empty() {
        let state = sample_state();
        let chapter = sample_chapter();
        let scene = sample_scene();
        let watch_only = vec![promise("A slow-burn romance", "watch")];
        let envelope = sample_envelope(Vec::new(), watch_only.clone());

        let host = assemble_host_scene_prompt(
            &state,
            &chapter,
            &scene,
            "chapter briefing markdown",
            &envelope,
            "scene-writer skill text",
            String::new(),
        )
        .unwrap();
        let pull =
            build_scene_mcp_pull_prompt(&state, &chapter, &scene, None, &[], &watch_only).unwrap();

        assert!(!host.contains("## Threads to advance"));
        assert!(!pull.contains("## Threads to advance"));
    }

    #[test]
    fn threads_block_caps_at_eight_lines_with_overflow_note_and_multibyte_safe_truncation() {
        // 10 threads + 3 due promises = 13 candidates. Cap 8, then "+5 more".
        // The first thread's statement is built so that after the "- [theme]
        // First — " prefix (18 chars) a run of multibyte '★' straddles the
        // 160-char truncation frontier: char 159 (last kept) and char 160
        // (first dropped) are both '★', so a naive byte-index slice at 160
        // would split the multibyte char and panic.
        let mut long_statement = "y".repeat(140);
        long_statement.push_str("★★★★★");
        long_statement.push_str(&"z".repeat(60));
        let mut threads = vec![thread("theme", "First", &long_statement)];
        for i in 2..=10 {
            threads.push(thread("plot_line", &format!("T{i}"), "short"));
        }
        let promises = vec![
            promise("Overdue payoff", "overdue"),
            promise("Due payoff", "due"),
            promise("Soon payoff", "soon"),
        ];

        let block = format_threads_to_advance(&threads, &promises).expect("block present");
        let item_lines: Vec<&str> = block
            .lines()
            .filter(|line| line.starts_with("- "))
            .collect();
        assert_eq!(item_lines.len(), 8, "exactly 8 item lines");
        assert!(
            block.contains("+5 more in context"),
            "overflow note for the 5 dropped candidates"
        );

        // Every line is char-boundary-safe truncated to at most 160 chars.
        for line in &item_lines {
            assert!(
                line.chars().count() <= 160,
                "line exceeded 160 chars: {line}"
            );
        }
        // The long first line is truncated to exactly 160 chars and lands on a
        // whole multibyte '★', proving no byte-boundary split occurred.
        let long_line = item_lines[0];
        assert_eq!(long_line.chars().count(), 160);
        assert_eq!(long_line.chars().last(), Some('★'));
    }

    #[test]
    fn shared_formatter_produces_byte_identical_block_in_both_paths() {
        let state = sample_state();
        let chapter = sample_chapter();
        let scene = sample_scene();
        let active_threads = vec![
            thread("theme", "Sunk Cost", "Every chip doubles the lie."),
            thread("motif", "The Red Seven", "A number that keeps recurring."),
        ];
        let promises = vec![
            promise("Ricky's debt comes due", "due"),
            promise("The marker is called", "overdue"),
        ];
        let envelope = sample_envelope(active_threads.clone(), promises.clone());

        let host = assemble_host_scene_prompt(
            &state,
            &chapter,
            &scene,
            "chapter briefing markdown",
            &envelope,
            "scene-writer skill text",
            String::new(),
        )
        .unwrap();
        let pull =
            build_scene_mcp_pull_prompt(&state, &chapter, &scene, None, &active_threads, &promises)
                .unwrap();

        let host_block = extract_threads_block(&host).expect("host block present");
        let pull_block = extract_threads_block(&pull).expect("pull block present");
        assert_eq!(
            host_block.as_bytes(),
            pull_block.as_bytes(),
            "the shared formatter must yield a byte-identical block in both paths"
        );
    }

    #[test]
    fn mcp_pull_scene_prompt_is_compact_and_context_seeking() {
        let state = HarnessState {
            project_id: "project:test".to_string(),
            active_branch_id: "branch:main".to_string(),
            book_number: 1,
            range: crate::state::ChapterRange {
                start_chapter: 1,
                end_chapter: 1,
            },
            checkpoint_interval: 1,
            last_checkpoint_end_chapter: 0,
            artifacts_dir: "artifacts".to_string(),
            editorial_directives: vec!["Keep the voice sharp.".to_string()],
            mining_policy: None,
            max_revise_attempts: None,
            checkpoint_policy: None,
            replan_policy: None,
            chapters: Vec::new(),
            checkpoint_history: Vec::new(),
        };
        let chapter = ChapterState {
            chapter_number: 1,
            planned: true,
            synopsis: "A first turn in the casino.".to_string(),
            pov_character_id: Some("character:dave".to_string()),
            status: ChapterStatus::Pending,
            scenes: Vec::new(),
            summary_saved: false,
            summary_artifact_path: None,
            replan_status: None,
            replan_detail: None,
        };
        let scene = SceneState {
            scene_order: 1,
            character_ids: vec!["character:dave".to_string(), "character:ricky".to_string()],
            location_id: "location:vegas-strip".to_string(),
            content_rating: ContentRating::Mature,
            tone: Some("tense comic dread".to_string()),
            source_path: None,
            phase: ScenePhase::Pending,
            scene_id: None,
            scene_artifact_path: None,
            draft_diagnostics: None,
            blocked_reason: None,
            research_required: Some(true),
            research_tags: vec!["1970s-vegas".to_string()],
            explicit_query: None,
            research_pack_empty: false,
            research_tags_matched: true,
            mine_status: None,
            mine_detail: None,
            verify_status: None,
            verify_detail: None,
            revise_attempts: 0,
            last_finding_fingerprint: None,
            revision_directives: None,
        };

        let prompt = build_scene_mcp_pull_prompt(&state, &chapter, &scene, None, &[], &[]).unwrap();

        assert!(
            prompt.len() < 6_000,
            "prompt was too large: {}",
            prompt.len()
        );
        assert!(prompt.contains("fiction for a book project"));
        assert!(prompt.contains("\"project_id\": \"project:test\""));
        assert!(prompt.contains("get_chapter_briefing"));
        assert!(prompt.contains("get_scene_context"));
        assert!(prompt.contains("research_pack_for_scene"));
        assert!(!prompt.contains("Scene context envelope:"));
        assert!(!prompt.contains("Scene-writer skill guidance:"));
    }

    // -- Pull-path threads fetch (T-110 asymmetry fix) --------------------
    //
    // The pull path cannot be exercised end-to-end because `McpHarnessClient`
    // is a concrete struct wrapping a live MCP transport with no trait seam to
    // fake `get_scene_context` (see the report). The testable seams are the two
    // pure helpers the pull path is built on: the sections request it issues,
    // and the "response/error -> (threads, promises)" projection that feeds the
    // shared formatter. These tests pin sections-minimality (test 2), the
    // success projection that lets the block render (test 1), and the
    // fetch-failure tolerance that yields empty slices (test 3).

    #[test]
    fn pull_path_requests_only_active_threads_and_promises_due_sections() {
        // Sections-minimality: the fetch names exactly the two sections we
        // need and nothing else, so the payload stays small.
        let sections = pull_path_threads_sections();
        assert_eq!(
            sections,
            vec![
                "active_threads".to_string(),
                "narrative_promises_due".to_string()
            ],
            "pull-path scene-context fetch must request only the two threads sections"
        );
    }

    #[test]
    fn pull_path_threads_projection_surfaces_threads_and_promises_on_success() {
        // Success flow: a scene context carrying 1 active thread + 1 due
        // promise projects to non-empty slices, so the shared formatter (and
        // therefore the pull prompt) renders the "## Threads to advance" block.
        let envelope = sample_envelope(
            vec![thread("theme", "Sunk Cost", "Every chip doubles the lie.")],
            vec![promise("Ricky's debt comes due", "due")],
        );

        let (threads, promises) = pull_path_threads_from_result(Ok(envelope));
        assert_eq!(threads.len(), 1);
        assert_eq!(promises.len(), 1);

        let state = sample_state();
        let chapter = sample_chapter();
        let scene = sample_scene();
        let prompt =
            build_scene_mcp_pull_prompt(&state, &chapter, &scene, None, &threads, &promises)
                .expect("pull prompt assembles");
        assert!(prompt.contains("## Threads to advance"));
        assert!(prompt.contains("Sunk Cost"));
        assert!(prompt.contains("Ricky's debt comes due"));
    }

    #[test]
    fn pull_path_threads_projection_tolerates_fetch_failure_with_empty_slices() {
        // Fetch-failure tolerance: a fetch error yields empty slices (no
        // propagation), so the pull prompt still builds without the block.
        let (threads, promises) =
            pull_path_threads_from_result(Err(anyhow::anyhow!("scene context fetch failed")));
        assert!(threads.is_empty());
        assert!(promises.is_empty());

        let state = sample_state();
        let chapter = sample_chapter();
        let scene = sample_scene();
        let prompt =
            build_scene_mcp_pull_prompt(&state, &chapter, &scene, None, &threads, &promises)
                .expect("pull prompt still assembles after a failed fetch");
        assert!(!prompt.contains("## Threads to advance"));
    }
}
