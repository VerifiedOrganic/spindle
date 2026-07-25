use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::state::{CheckpointStatus, HarnessState, ScenePhase};

pub fn render_status(state: &HarnessState, state_path: &Path, verbose: bool) -> String {
    let mut out = String::new();
    let artifacts_root = artifacts_root(state_path, state);

    let _ = writeln!(out, "Project: {}", state.project_id);
    let _ = writeln!(out, "Active branch: {}", state.active_branch_id);
    let _ = writeln!(
        out,
        "Range: book {} chapters {}-{}",
        state.book_number, state.range.start_chapter, state.range.end_chapter
    );
    let _ = writeln!(out, "Checkpoint interval: {}", state.checkpoint_interval);
    let _ = writeln!(
        out,
        "Completed chapters: {}",
        state.completed_chapter_count()
    );
    let _ = writeln!(
        out,
        "Last checkpoint end: {}",
        state.last_checkpoint_end_chapter
    );
    let _ = writeln!(
        out,
        "Editorial directives: {}",
        state.editorial_directives.len()
    );
    let _ = writeln!(
        out,
        "Checkpoint history: {}",
        state.checkpoint_history.len()
    );
    if verbose {
        let _ = writeln!(out, "Artifacts root: {}", artifacts_root.display());
    }

    if verbose && !state.editorial_directives.is_empty() {
        let _ = writeln!(out, "Directives:");
        for directive in &state.editorial_directives {
            let _ = writeln!(out, "  - {}", directive);
        }
    }

    for chapter in &state.chapters {
        let scene_progress = chapter
            .scenes
            .iter()
            .map(|scene| format!("{}:{}", scene.scene_order, phase_label(scene.phase)))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(
            out,
            "Chapter {} [{}] summary_saved={} scenes=[{}]",
            chapter.chapter_number,
            chapter_status_label(chapter.status),
            chapter.summary_saved,
            scene_progress
        );

        if !verbose {
            continue;
        }

        if let Some(summary_artifact_path) = chapter.summary_artifact_path.as_ref() {
            let _ = writeln!(
                out,
                "  summary_artifact: {}",
                artifacts_root.join(summary_artifact_path).display()
            );
        }

        for scene in &chapter.scenes {
            let _ = writeln!(
                out,
                "  Scene {} [{}] scene_id={} artifact={}",
                scene.scene_order,
                phase_label(scene.phase),
                scene.scene_id.as_deref().unwrap_or("-"),
                scene
                    .scene_artifact_path
                    .as_ref()
                    .map(|path| artifacts_root.join(path).display().to_string())
                    .unwrap_or_else(|| "-".to_string())
            );
            if let Some(blocked_reason) = scene.blocked_reason.as_ref() {
                let _ = writeln!(out, "    blocked: {}", blocked_reason);
            }
            if let Some(diagnostics) = scene.draft_diagnostics.as_ref() {
                if !diagnostics.pacing_warnings.is_empty() {
                    let _ = writeln!(
                        out,
                        "    pacing_warnings: {}",
                        diagnostics.pacing_warnings.join(" | ")
                    );
                }
                if let Some(agency_warning) = diagnostics.agency_warning.as_ref() {
                    let _ = writeln!(
                        out,
                        "    agency_warning: {:?}: {}",
                        agency_warning.kind, agency_warning.message
                    );
                }
                if diagnostics.tone_deviation {
                    let _ = writeln!(out, "    tone_deviation: true");
                }
                if !diagnostics.content_rating_valid {
                    let _ = writeln!(out, "    content_rating_valid: false");
                }
                if !diagnostics.content_rating_warnings.is_empty() {
                    let _ = writeln!(
                        out,
                        "    content_rating_warnings: {}",
                        diagnostics.content_rating_warnings.join(" | ")
                    );
                }
            }
        }
    }

    if verbose && !state.checkpoint_history.is_empty() {
        let _ = writeln!(out, "Checkpoints:");
        for checkpoint in &state.checkpoint_history {
            let report = checkpoint
                .report_artifact_path
                .as_ref()
                .map(|path| artifacts_root.join(path).display().to_string())
                .unwrap_or_else(|| "-".to_string());
            let _ = writeln!(
                out,
                "  {}-{} [{}] save_point={} report={}",
                checkpoint.start_chapter,
                checkpoint.end_chapter,
                checkpoint_status_label(checkpoint.status),
                checkpoint.save_point_id,
                report
            );
        }
    }

    out
}

pub fn review_checkpoint(
    state: &mut HarnessState,
    state_path: &Path,
    start_chapter: i32,
    end_chapter: i32,
    directives: &[String],
) -> Result<String> {
    let artifacts_root = artifacts_root(state_path, state);
    let checkpoint = state
        .checkpoint_history
        .iter_mut()
        .find(|checkpoint| {
            checkpoint.start_chapter == start_chapter && checkpoint.end_chapter == end_chapter
        })
        .with_context(|| {
            format!(
                "checkpoint {}-{} not found in state",
                start_chapter, end_chapter
            )
        })?;

    if checkpoint.status != CheckpointStatus::PendingReview {
        anyhow::bail!(
            "checkpoint {}-{} is already marked {}",
            start_chapter,
            end_chapter,
            checkpoint_status_label(checkpoint.status)
        );
    }

    let report_artifact_path = checkpoint.report_artifact_path.clone().with_context(|| {
        format!(
            "checkpoint {}-{} has no report artifact path",
            start_chapter, end_chapter
        )
    })?;
    let report_path = artifacts_root.join(report_artifact_path);
    if !report_path.exists() {
        anyhow::bail!(
            "checkpoint {}-{} report artifact does not exist at {}",
            start_chapter,
            end_chapter,
            report_path.display()
        );
    }

    checkpoint.status = CheckpointStatus::Reviewed;
    let added_directives = append_directives(&mut state.editorial_directives, directives);
    state.save(state_path)?;

    Ok(format!(
        "Marked checkpoint {}-{} as reviewed; added {} new directive(s).",
        start_chapter, end_chapter, added_directives
    ))
}

/// Advance a blocked scene one phase forward after operator review.
///
/// Two block levels license the advance (defect item 1c):
/// - scene-level: `scene.blocked_reason` is set — the classic path; the scene's
///   recorded artifact must exist on disk.
/// - run-level: the scene itself is not blocked but the RUN is
///   (`run_level_blocked`, e.g. reconcile findings such as summary residue).
///   Pre-existing-prose scenes have no artifact, so the artifact requirement
///   applies only when the scene actually recorded one.
///
/// A truly unblocked scene in an unblocked run is refused — there is nothing
/// for an operator to resolve.
pub fn resolve_scene_block(
    state: &mut HarnessState,
    state_path: &Path,
    chapter_number: i32,
    scene_order: i32,
    target_phase: ScenePhase,
    run_level_blocked: bool,
) -> Result<String> {
    let artifacts_root = artifacts_root(state_path, state);
    let chapter = state
        .chapters
        .iter_mut()
        .find(|chapter| chapter.chapter_number == chapter_number)
        .with_context(|| format!("chapter {} not found in state", chapter_number))?;
    let scene = chapter
        .scenes
        .iter_mut()
        .find(|scene| scene.scene_order == scene_order)
        .with_context(|| {
            format!(
                "scene {}.{} not found in state",
                chapter_number, scene_order
            )
        })?;

    let blocked_reason = scene.blocked_reason.clone();
    if blocked_reason.is_none() && !run_level_blocked {
        anyhow::bail!(
            "scene {}.{} has no scene-level block and the run has no run-level block; \
             authoring_resolve_block only advances a scene that is blocked itself or \
             sits in a run blocked by reconcile findings",
            chapter_number,
            scene_order
        );
    }

    let expected_phase = next_scene_phase(scene.phase).with_context(|| {
        format!(
            "scene {}.{} is already at the final phase and cannot be advanced manually",
            chapter_number, scene_order
        )
    })?;
    if target_phase != expected_phase {
        anyhow::bail!(
            "scene {}.{} is {}; the only allowed manual advance is to {}",
            chapter_number,
            scene_order,
            phase_label(scene.phase),
            phase_label(expected_phase)
        );
    }

    // A scene-level block always has a run-recorded artifact and must keep it.
    // Under a run-level block the scene may predate the run (no artifact); when
    // one IS recorded it still has to exist — a dangling path is corruption.
    match scene.scene_artifact_path.clone() {
        Some(artifact_path) => {
            let full_artifact_path = artifacts_root.join(artifact_path);
            if !full_artifact_path.exists() {
                anyhow::bail!(
                    "scene {}.{} artifact does not exist at {}",
                    chapter_number,
                    scene_order,
                    full_artifact_path.display()
                );
            }
        }
        None if blocked_reason.is_some() => {
            anyhow::bail!(
                "scene {}.{} has no artifact path; cannot advance safely",
                chapter_number,
                scene_order
            );
        }
        None => {}
    }

    if scene.scene_id.is_none() {
        anyhow::bail!(
            "scene {}.{} has no scene_id; cannot advance manually",
            chapter_number,
            scene_order
        );
    }

    scene.phase = target_phase;
    scene.blocked_reason = None;
    state.save(state_path)?;

    Ok(format!(
        "Advanced scene {}.{} to {} after operator review. Previous block: {}",
        chapter_number,
        scene_order,
        phase_label(target_phase),
        blocked_reason.unwrap_or_else(|| "run-level block (reconcile findings)".to_string())
    ))
}

/// Reset a scene to pending-draft so the next `authoring_execute_next`
/// re-dispatches a fresh draft (BUG 3 operator path). Use when a draft is
/// unparseable / poisoned and the automatic clear-on-failure was not enough
/// (e.g. the operator wants a clean re-draft after editing config).
///
/// Semantics:
/// - phase → `Pending`, `blocked_reason`, `scene_id`, and `draft_diagnostics`
///   cleared;
/// - the on-disk scene artifact is deleted so `load_or_create_scene_artifact`
///   rebuilds it fresh (a stale artifact would otherwise resurrect the poisoned
///   generation or a stale save-draft output on reuse);
/// - verify state (`verify_status`, `verify_detail`, `last_finding_fingerprint`,
///   `revise_attempts`, `revision_directives`) is cleared per the established
///   column semantics — the prior draft's verify outcome no longer applies.
///
/// Unlike [`resolve_scene_block`], this does NOT require the scene to be
/// currently blocked (a parse-failed scene stays `Pending` with no block set)
/// and it moves the scene BACKWARD rather than one phase forward.
pub fn redraft_scene_block(
    state: &mut HarnessState,
    state_path: &Path,
    chapter_number: i32,
    scene_order: i32,
) -> Result<String> {
    let artifacts_root = artifacts_root(state_path, state);
    let chapter = state
        .chapters
        .iter_mut()
        .find(|chapter| chapter.chapter_number == chapter_number)
        .with_context(|| format!("chapter {} not found in state", chapter_number))?;
    let scene = chapter
        .scenes
        .iter_mut()
        .find(|scene| scene.scene_order == scene_order)
        .with_context(|| {
            format!(
                "scene {}.{} not found in state",
                chapter_number, scene_order
            )
        })?;

    // Delete the on-disk artifact so the reuse path rebuilds it fresh. Absence
    // is fine (nothing to resurrect); other IO errors are real failures. A
    // parse-failed draft writes the artifact to its deterministic path but never
    // persists that path to the run tables (the error aborts before the run-state
    // save), so fall back to the deterministic path when the state has no
    // recorded artifact path — otherwise a poisoned artifact would survive.
    let artifact_rel = scene.scene_artifact_path.clone().unwrap_or_else(|| {
        crate::artifacts::ArtifactStore::scene_relative_path(chapter_number, scene_order)
    });
    let full_artifact_path = artifacts_root.join(&artifact_rel);
    match std::fs::remove_file(&full_artifact_path) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_context(|| {
                format!(
                    "failed to remove scene {}.{} artifact at {}",
                    chapter_number,
                    scene_order,
                    full_artifact_path.display()
                )
            });
        }
    }

    let previous_phase = phase_label(scene.phase);
    scene.phase = ScenePhase::Pending;
    scene.blocked_reason = None;
    scene.scene_id = None;
    scene.draft_diagnostics = None;
    // Verify state no longer applies to a scene being re-drafted from scratch.
    scene.verify_status = None;
    scene.verify_detail = None;
    scene.last_finding_fingerprint = None;
    scene.revise_attempts = 0;
    scene.revision_directives = None;

    state.save(state_path)?;

    Ok(format!(
        "Reset scene {}.{} (was {}) to pending-draft; the next execute will re-draft it.",
        chapter_number, scene_order, previous_phase
    ))
}

fn append_directives(existing: &mut Vec<String>, directives: &[String]) -> usize {
    let mut added = 0;
    for directive in directives {
        let trimmed = directive.trim();
        if trimmed.is_empty() {
            continue;
        }
        if existing.iter().any(|existing| existing == trimmed) {
            continue;
        }
        existing.push(trimmed.to_string());
        added += 1;
    }
    added
}

fn next_scene_phase(phase: ScenePhase) -> Option<ScenePhase> {
    match phase {
        ScenePhase::Pending => Some(ScenePhase::DraftSaved),
        ScenePhase::DraftSaved => Some(ScenePhase::ChangesCommitted),
        ScenePhase::ChangesCommitted => Some(ScenePhase::BeatsAnnotated),
        ScenePhase::BeatsAnnotated => None,
    }
}

fn artifacts_root(state_path: &Path, state: &HarnessState) -> PathBuf {
    state_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&state.artifacts_dir)
}

fn chapter_status_label(status: crate::state::ChapterStatus) -> &'static str {
    match status {
        crate::state::ChapterStatus::Pending => "pending",
        crate::state::ChapterStatus::InProgress => "in_progress",
        crate::state::ChapterStatus::Complete => "complete",
    }
}

fn checkpoint_status_label(status: CheckpointStatus) -> &'static str {
    match status {
        CheckpointStatus::PendingReview => "pending_review",
        CheckpointStatus::Reviewed => "reviewed",
    }
}

fn phase_label(phase: ScenePhase) -> &'static str {
    match phase {
        ScenePhase::Pending => "pending",
        ScenePhase::DraftSaved => "draft_saved",
        ScenePhase::ChangesCommitted => "changes_committed",
        ScenePhase::BeatsAnnotated => "beats_annotated",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use spindle_core::models::ContentRating;

    use super::*;
    use crate::state::{
        ChapterRange, ChapterSeed, CheckpointRecord, HarnessSeed, SceneSeed, SceneState,
    };

    fn temp_state_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("spindle-harness-{name}-{unique}"));
        fs::create_dir_all(&root).expect("create temp root");
        root.join("state.json")
    }

    fn seed() -> HarnessSeed {
        HarnessSeed {
            project_id: "project:test".to_string(),
            book_number: 1,
            range: ChapterRange {
                start_chapter: 1,
                end_chapter: 1,
            },
            checkpoint_interval: 1,
            editorial_directives: vec!["hold continuity".to_string()],
            chapters: vec![ChapterSeed {
                chapter_number: 1,
                synopsis: "Test chapter".to_string(),
                pov_character_id: Some("character:pov".to_string()),
                scenes: vec![SceneSeed {
                    scene_order: 1,
                    character_ids: vec!["character:pov".to_string()],
                    location_id: "location:test".to_string(),
                    content_rating: ContentRating::Teen,
                    tone: Some("tense".to_string()),
                    source_path: None,
                    ..Default::default()
                }],
            }],
        }
    }

    #[test]
    fn review_checkpoint_marks_reviewed_and_appends_directives() {
        let state_path = temp_state_path("checkpoint-review");
        let mut state = HarnessState::from_seed(seed(), "branch:main".to_string());
        let report_rel = "checkpoints/chapter-0001-0001.json".to_string();
        let report_path = artifacts_root(&state_path, &state).join(&report_rel);
        fs::create_dir_all(report_path.parent().expect("report parent")).expect("mkdirs");
        fs::write(&report_path, "{}").expect("write report");
        state.checkpoint_history.push(CheckpointRecord {
            start_chapter: 1,
            end_chapter: 1,
            save_point_id: "save_point:1".to_string(),
            status: CheckpointStatus::PendingReview,
            report_artifact_path: Some(report_rel),
            auto_outcome: None,
            pending_manual_scene_ids: Vec::new(),
        });

        let message = review_checkpoint(
            &mut state,
            &state_path,
            1,
            1,
            &[
                "tighten scene transitions".to_string(),
                "hold continuity".to_string(),
            ],
        )
        .expect("review checkpoint");

        assert!(message.contains("added 1 new directive"));
        assert_eq!(
            state.checkpoint_history[0].status,
            CheckpointStatus::Reviewed
        );
        assert_eq!(state.editorial_directives.len(), 2);
    }

    #[test]
    fn resolve_scene_block_advances_one_phase_and_clears_block() {
        let state_path = temp_state_path("resolve-scene-block");
        let mut state = HarnessState::from_seed(seed(), "branch:main".to_string());
        let artifact_rel = "scenes/chapter-0001/scene-001.json".to_string();
        let artifact_path = artifacts_root(&state_path, &state).join(&artifact_rel);
        fs::create_dir_all(artifact_path.parent().expect("artifact parent")).expect("mkdirs");
        fs::write(&artifact_path, "{}").expect("write artifact");

        let scene = &mut state.chapters[0].scenes[0];
        *scene = SceneState {
            scene_order: 1,
            character_ids: vec!["character:pov".to_string()],
            location_id: "location:test".to_string(),
            content_rating: ContentRating::Teen,
            tone: Some("tense".to_string()),
            source_path: None,
            phase: ScenePhase::DraftSaved,
            scene_id: Some("scene:1".to_string()),
            scene_artifact_path: Some(artifact_rel),
            draft_diagnostics: None,
            blocked_reason: Some("partial commit applied".to_string()),
            ..Default::default()
        };

        let message = resolve_scene_block(
            &mut state,
            &state_path,
            1,
            1,
            ScenePhase::ChangesCommitted,
            false,
        )
        .expect("resolve scene block");

        assert!(message.contains("Advanced scene 1.1"));
        assert_eq!(
            state.chapters[0].scenes[0].phase,
            ScenePhase::ChangesCommitted
        );
        assert!(state.chapters[0].scenes[0].blocked_reason.is_none());
    }

    #[test]
    fn redraft_scene_block_resets_to_pending_and_deletes_artifact() {
        let state_path = temp_state_path("redraft-scene-block");
        let mut state = HarnessState::from_seed(seed(), "branch:main".to_string());
        let artifact_rel = "scenes/chapter-0001/scene-001.json".to_string();
        let artifact_path = artifacts_root(&state_path, &state).join(&artifact_rel);
        fs::create_dir_all(artifact_path.parent().expect("artifact parent")).expect("mkdirs");
        fs::write(&artifact_path, "{\"poisoned\": true}").expect("write artifact");

        let scene = &mut state.chapters[0].scenes[0];
        *scene = SceneState {
            scene_order: 1,
            character_ids: vec!["character:pov".to_string()],
            location_id: "location:test".to_string(),
            content_rating: ContentRating::Teen,
            tone: Some("tense".to_string()),
            source_path: None,
            phase: ScenePhase::Pending,
            scene_id: Some("scene:1".to_string()),
            scene_artifact_path: Some(artifact_rel),
            draft_diagnostics: None,
            blocked_reason: None,
            verify_status: Some("findings".to_string()),
            verify_detail: Some("2 finding(s)".to_string()),
            last_finding_fingerprint: Some("fp".to_string()),
            revise_attempts: 2,
            revision_directives: Some("rework".to_string()),
            ..Default::default()
        };

        let message = redraft_scene_block(&mut state, &state_path, 1, 1).expect("redraft");
        assert!(message.contains("Reset scene 1.1"));

        let scene = &state.chapters[0].scenes[0];
        assert_eq!(scene.phase, ScenePhase::Pending);
        assert!(scene.blocked_reason.is_none());
        assert!(scene.scene_id.is_none());
        assert!(scene.verify_status.is_none());
        assert!(scene.verify_detail.is_none());
        assert!(scene.last_finding_fingerprint.is_none());
        assert_eq!(scene.revise_attempts, 0);
        assert!(scene.revision_directives.is_none());
        // The poisoned artifact is gone so the reuse path rebuilds fresh.
        assert!(!artifact_path.exists(), "artifact must be deleted");
    }

    // ── Run-level-block override (defect item 1c) ──

    #[test]
    fn resolve_scene_block_advances_unblocked_scene_when_run_is_blocked() {
        // The block can live at RUN level (reconcile errors) with no
        // scene.blocked_reason set. The operator must still be able to advance
        // a scene forward one phase — including a pre-existing-prose scene
        // that has NO artifact (the prose came from outside any run).
        let state_path = temp_state_path("resolve-run-level-block");
        let mut state = HarnessState::from_seed(seed(), "branch:main".to_string());
        let scene = &mut state.chapters[0].scenes[0];
        scene.phase = ScenePhase::DraftSaved;
        scene.scene_id = Some("scene:1".to_string());
        scene.scene_artifact_path = None;
        scene.blocked_reason = None;

        let message = resolve_scene_block(
            &mut state,
            &state_path,
            1,
            1,
            ScenePhase::ChangesCommitted,
            true,
        )
        .expect("run-level block must allow a forward advance");

        assert!(message.contains("Advanced scene 1.1"), "{message}");
        assert!(message.contains("run-level block"), "{message}");
        assert_eq!(
            state.chapters[0].scenes[0].phase,
            ScenePhase::ChangesCommitted
        );
    }

    #[test]
    fn resolve_scene_block_refusal_names_both_block_levels() {
        // A truly unblocked run + unblocked scene still refuses, and the error
        // text must distinguish scene-level from run-level blocks so the
        // operator knows why the tool declined.
        let state_path = temp_state_path("resolve-no-block");
        let mut state = HarnessState::from_seed(seed(), "branch:main".to_string());
        let scene = &mut state.chapters[0].scenes[0];
        scene.phase = ScenePhase::DraftSaved;
        scene.scene_id = Some("scene:1".to_string());

        let err = resolve_scene_block(
            &mut state,
            &state_path,
            1,
            1,
            ScenePhase::ChangesCommitted,
            false,
        )
        .expect_err("an unblocked scene in an unblocked run must refuse");
        let text = format!("{err:#}");
        assert!(
            text.contains("scene-level") && text.contains("run-level"),
            "refusal must explain both block levels: {text}"
        );
    }

    #[test]
    fn redraft_scene_block_is_idempotent_when_artifact_already_absent() {
        let state_path = temp_state_path("redraft-scene-block-noartifact");
        let mut state = HarnessState::from_seed(seed(), "branch:main".to_string());
        let scene = &mut state.chapters[0].scenes[0];
        scene.scene_artifact_path = Some("scenes/chapter-0001/scene-001.json".to_string());
        scene.phase = ScenePhase::Pending;

        // No artifact file on disk — must not error.
        let message = redraft_scene_block(&mut state, &state_path, 1, 1)
            .expect("redraft with absent artifact");
        assert!(message.contains("Reset scene 1.1"));
        assert_eq!(state.chapters[0].scenes[0].phase, ScenePhase::Pending);
    }
}
