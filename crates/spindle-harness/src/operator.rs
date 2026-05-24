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

pub fn resolve_scene_block(
    state: &mut HarnessState,
    state_path: &Path,
    chapter_number: i32,
    scene_order: i32,
    target_phase: ScenePhase,
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

    let blocked_reason = scene.blocked_reason.clone().with_context(|| {
        format!(
            "scene {}.{} is not currently blocked for operator review",
            chapter_number, scene_order
        )
    })?;

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

    let artifact_path = scene.scene_artifact_path.clone().with_context(|| {
        format!(
            "scene {}.{} has no artifact path; cannot advance safely",
            chapter_number, scene_order
        )
    })?;
    let full_artifact_path = artifacts_root.join(artifact_path);
    if !full_artifact_path.exists() {
        anyhow::bail!(
            "scene {}.{} artifact does not exist at {}",
            chapter_number,
            scene_order,
            full_artifact_path.display()
        );
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
        blocked_reason
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
        };

        let message =
            resolve_scene_block(&mut state, &state_path, 1, 1, ScenePhase::ChangesCommitted)
                .expect("resolve scene block");

        assert!(message.contains("Advanced scene 1.1"));
        assert_eq!(
            state.chapters[0].scenes[0].phase,
            ScenePhase::ChangesCommitted
        );
        assert!(state.chapters[0].scenes[0].blocked_reason.is_none());
    }
}
