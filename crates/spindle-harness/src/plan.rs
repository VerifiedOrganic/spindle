use std::collections::{BTreeMap, BTreeSet};

use crate::state::{ChapterRange, CheckpointStatus, HarnessState, ScenePhase};

#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    pub active_branch_id: String,
    pub active_branch_name: String,
    pub chapters: BTreeMap<i32, ChapterSnapshot>,
    pub summarized_chapters: BTreeSet<i32>,
}

#[derive(Debug, Clone)]
pub struct ChapterSnapshot {
    pub chapter_id: String,
    pub scenes: BTreeMap<i32, PersistedScene>,
    pub chapter_plan: Option<ChapterPlanSnapshot>,
}

#[derive(Debug, Clone)]
pub struct PersistedScene {
    pub scene_id: String,
    pub scene_order: i32,
}

#[derive(Debug, Clone)]
pub struct ChapterPlanSnapshot {
    pub synopsis: String,
    pub pov_character_id: Option<String>,
    pub scenes: Vec<PlannedSceneSnapshot>,
}

#[derive(Debug, Clone)]
pub struct PlannedSceneSnapshot {
    pub scene_order: i32,
    pub character_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: FindingSeverity,
    pub code: &'static str,
    pub message: String,
}

impl Finding {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: FindingSeverity::Error,
            code,
            message: message.into(),
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: FindingSeverity::Warning,
            code,
            message: message.into(),
        }
    }

    pub fn info(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: FindingSeverity::Info,
            code,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NextAction {
    Blocked,
    AwaitCheckpointReview {
        start_chapter: i32,
        end_chapter: i32,
        save_point_id: String,
    },
    RunCheckpoint {
        start_chapter: i32,
        end_chapter: i32,
    },
    DraftScene {
        chapter_number: i32,
        scene_order: i32,
    },
    CommitSceneChanges {
        chapter_number: i32,
        scene_order: i32,
        scene_id: String,
    },
    AnnotateSceneBeats {
        chapter_number: i32,
        scene_order: i32,
        scene_id: String,
    },
    SaveChapterSummary {
        chapter_number: i32,
    },
    Complete,
}

impl std::fmt::Display for NextAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Blocked => write!(f, "blocked"),
            Self::AwaitCheckpointReview {
                start_chapter,
                end_chapter,
                save_point_id,
            } => write!(
                f,
                "await checkpoint review for chapters {start_chapter}-{end_chapter} (save point {save_point_id})"
            ),
            Self::RunCheckpoint {
                start_chapter,
                end_chapter,
            } => write!(
                f,
                "run checkpoint for chapters {start_chapter}-{end_chapter}"
            ),
            Self::DraftScene {
                chapter_number,
                scene_order,
            } => write!(f, "draft book scene {chapter_number}.{scene_order}"),
            Self::CommitSceneChanges {
                chapter_number,
                scene_order,
                scene_id,
            } => write!(
                f,
                "commit scene changes for chapter {chapter_number} scene {scene_order} ({scene_id})"
            ),
            Self::AnnotateSceneBeats {
                chapter_number,
                scene_order,
                scene_id,
            } => write!(
                f,
                "annotate beats for chapter {chapter_number} scene {scene_order} ({scene_id})"
            ),
            Self::SaveChapterSummary { chapter_number } => {
                write!(f, "save summary for chapter {chapter_number}")
            }
            Self::Complete => write!(f, "complete"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReconcileOutcome {
    pub state: HarnessState,
    pub findings: Vec<Finding>,
    pub next_action: NextAction,
}

impl ReconcileOutcome {
    pub fn has_errors(&self) -> bool {
        self.findings
            .iter()
            .any(|finding| finding.severity == FindingSeverity::Error)
    }
}

pub fn reconcile_state(mut state: HarnessState, snapshot: &ProjectSnapshot) -> ReconcileOutcome {
    state.normalize();
    let mut findings = validate_state_shape(&state);

    if state.active_branch_id != snapshot.active_branch_id {
        findings.push(Finding::error(
            "branch_mismatch",
            format!(
                "state expects active branch {}, but Spindle reports {} ({})",
                state.active_branch_id, snapshot.active_branch_id, snapshot.active_branch_name
            ),
        ));
    }

    for chapter in &mut state.chapters {
        let Some(chapter_snapshot) = snapshot.chapters.get(&chapter.chapter_number) else {
            findings.push(Finding::error(
                "missing_chapter",
                format!(
                    "chapter {} does not exist or could not be read from Spindle",
                    chapter.chapter_number
                ),
            ));
            continue;
        };

        if chapter_snapshot.chapter_id.is_empty() {
            findings.push(Finding::error(
                "missing_chapter_id",
                format!(
                    "chapter {} resolved without a stable chapter id",
                    chapter.chapter_number
                ),
            ));
        }

        reconcile_chapter_plan(chapter, chapter_snapshot, &mut findings);
        reconcile_persisted_scenes(chapter, chapter_snapshot, &mut findings);
        reconcile_summary_state(
            chapter,
            snapshot
                .summarized_chapters
                .contains(&chapter.chapter_number),
            &mut findings,
        );
        chapter.recompute_status();
    }

    validate_checkpoint_history(&state, &mut findings);
    validate_completion_order(&state, &mut findings);

    let next_action = if findings
        .iter()
        .any(|finding| finding.severity == FindingSeverity::Error)
    {
        NextAction::Blocked
    } else {
        determine_next_action(&state)
    };

    ReconcileOutcome {
        state,
        findings,
        next_action,
    }
}

fn validate_state_shape(state: &HarnessState) -> Vec<Finding> {
    let mut findings = Vec::new();

    if state.project_id.trim().is_empty() {
        findings.push(Finding::error(
            "missing_project_id",
            "state.project_id must not be empty",
        ));
    }
    if state.active_branch_id.trim().is_empty() {
        findings.push(Finding::error(
            "missing_active_branch_id",
            "state.active_branch_id must not be empty",
        ));
    }
    if state.book_number <= 0 {
        findings.push(Finding::error(
            "invalid_book_number",
            format!("book_number must be positive, got {}", state.book_number),
        ));
    }
    if state.checkpoint_interval == 0 {
        findings.push(Finding::error(
            "invalid_checkpoint_interval",
            "checkpoint_interval must be at least 1",
        ));
    }
    if state.range.start_chapter <= 0 || state.range.end_chapter <= 0 {
        findings.push(Finding::error(
            "invalid_range",
            format!(
                "chapter range must be positive, got {}-{}",
                state.range.start_chapter, state.range.end_chapter
            ),
        ));
    }
    if state.range.start_chapter > state.range.end_chapter {
        findings.push(Finding::error(
            "invalid_range",
            format!(
                "chapter range start {} is after end {}",
                state.range.start_chapter, state.range.end_chapter
            ),
        ));
    }

    let expected_chapters = chapter_numbers_in_range(&state.range);
    let actual_chapters = state
        .chapters
        .iter()
        .map(|chapter| chapter.chapter_number)
        .collect::<Vec<_>>();
    if actual_chapters != expected_chapters {
        findings.push(Finding::error(
            "chapter_range_gap",
            format!(
                "state chapters {:?} do not exactly cover range {:?}",
                actual_chapters, expected_chapters
            ),
        ));
    }

    for chapter in &state.chapters {
        if chapter.synopsis.trim().is_empty() {
            findings.push(Finding::error(
                "missing_synopsis",
                format!(
                    "chapter {} synopsis must not be empty",
                    chapter.chapter_number
                ),
            ));
        }
        if chapter.scenes.is_empty() {
            findings.push(Finding::error(
                "missing_scenes",
                format!("chapter {} has no scene manifest", chapter.chapter_number),
            ));
            continue;
        }

        let expected_scene_orders = (1..=chapter.scenes.len() as i32).collect::<Vec<_>>();
        let actual_scene_orders = chapter
            .scenes
            .iter()
            .map(|scene| scene.scene_order)
            .collect::<Vec<_>>();
        if actual_scene_orders != expected_scene_orders {
            findings.push(Finding::error(
                "scene_order_gap",
                format!(
                    "chapter {} scene orders {:?} are not contiguous {:?}",
                    chapter.chapter_number, actual_scene_orders, expected_scene_orders
                ),
            ));
        }

        for scene in &chapter.scenes {
            if scene.character_ids.is_empty() {
                findings.push(Finding::error(
                    "missing_scene_characters",
                    format!(
                        "chapter {} scene {} has no character_ids",
                        chapter.chapter_number, scene.scene_order
                    ),
                ));
            }
            if scene.location_id.trim().is_empty() {
                findings.push(Finding::error(
                    "missing_scene_location",
                    format!(
                        "chapter {} scene {} has no location_id",
                        chapter.chapter_number, scene.scene_order
                    ),
                ));
            }
            if let Some(blocked_reason) = scene.blocked_reason.as_ref()
                && !blocked_reason.trim().is_empty()
            {
                findings.push(Finding::error(
                    "scene_manual_review_required",
                    format!(
                        "chapter {} scene {} requires manual review: {}",
                        chapter.chapter_number, scene.scene_order, blocked_reason
                    ),
                ));
            }
            if scene.phase == ScenePhase::Pending && scene.scene_id.is_some() {
                findings.push(Finding::error(
                    "phase_scene_id_mismatch",
                    format!(
                        "chapter {} scene {} is pending but already has a scene_id",
                        chapter.chapter_number, scene.scene_order
                    ),
                ));
            }
            if scene.phase != ScenePhase::Pending && scene.scene_id.is_none() {
                findings.push(Finding::error(
                    "phase_missing_scene_id",
                    format!(
                        "chapter {} scene {} is {:?} but has no scene_id",
                        chapter.chapter_number, scene.scene_order, scene.phase
                    ),
                ));
            }
            if scene.phase != ScenePhase::Pending && scene.scene_artifact_path.is_none() {
                findings.push(Finding::error(
                    "missing_scene_artifact",
                    format!(
                        "chapter {} scene {} is {:?} but has no scene artifact path for safe resume",
                        chapter.chapter_number, scene.scene_order, scene.phase
                    ),
                ));
            }
        }
    }

    findings
}

fn reconcile_chapter_plan(
    chapter: &crate::state::ChapterState,
    snapshot: &ChapterSnapshot,
    findings: &mut Vec<Finding>,
) {
    let Some(plan) = snapshot.chapter_plan.as_ref() else {
        findings.push(Finding::warning(
            "missing_chapter_plan",
            format!(
                "chapter {} has no persisted chapter plan; harness manifest is the only plan source",
                chapter.chapter_number
            ),
        ));
        return;
    };

    if chapter.synopsis != plan.synopsis {
        findings.push(Finding::error(
            "chapter_plan_synopsis_mismatch",
            format!(
                "chapter {} synopsis differs from persisted chapter plan",
                chapter.chapter_number
            ),
        ));
    }
    if chapter.pov_character_id != plan.pov_character_id {
        findings.push(Finding::error(
            "chapter_plan_pov_mismatch",
            format!(
                "chapter {} POV differs from persisted chapter plan",
                chapter.chapter_number
            ),
        ));
    }

    let manifest_orders = chapter
        .scenes
        .iter()
        .map(|scene| scene.scene_order)
        .collect::<Vec<_>>();
    let plan_orders = plan
        .scenes
        .iter()
        .map(|scene| scene.scene_order)
        .collect::<Vec<_>>();
    if manifest_orders != plan_orders {
        findings.push(Finding::error(
            "chapter_plan_scene_order_mismatch",
            format!(
                "chapter {} scene orders differ between harness manifest {:?} and chapter plan {:?}",
                chapter.chapter_number, manifest_orders, plan_orders
            ),
        ));
        return;
    }

    let plan_by_order = plan
        .scenes
        .iter()
        .map(|scene| (scene.scene_order, scene))
        .collect::<BTreeMap<_, _>>();
    for scene in &chapter.scenes {
        let Some(plan_scene) = plan_by_order.get(&scene.scene_order) else {
            continue;
        };
        if scene.character_ids != plan_scene.character_ids {
            findings.push(Finding::error(
                "chapter_plan_character_mismatch",
                format!(
                    "chapter {} scene {} character_ids differ between harness manifest and persisted chapter plan",
                    chapter.chapter_number, scene.scene_order
                ),
            ));
        }
    }
}

fn reconcile_persisted_scenes(
    chapter: &mut crate::state::ChapterState,
    snapshot: &ChapterSnapshot,
    findings: &mut Vec<Finding>,
) {
    for scene in &mut chapter.scenes {
        match snapshot.scenes.get(&scene.scene_order) {
            Some(persisted) => {
                if let Some(existing_scene_id) = scene.scene_id.as_ref() {
                    if existing_scene_id != &persisted.scene_id {
                        findings.push(Finding::error(
                            "scene_id_mismatch",
                            format!(
                                "chapter {} scene {} state scene_id {} does not match persisted scene_id {}",
                                chapter.chapter_number,
                                scene.scene_order,
                                existing_scene_id,
                                persisted.scene_id
                            ),
                        ));
                    }
                } else {
                    scene.scene_id = Some(persisted.scene_id.clone());
                    findings.push(Finding::info(
                        "scene_id_captured",
                        format!(
                            "captured scene_id {} for chapter {} scene {}",
                            persisted.scene_id, chapter.chapter_number, scene.scene_order
                        ),
                    ));
                }

                if scene.phase == ScenePhase::Pending {
                    scene.phase = ScenePhase::DraftSaved;
                    findings.push(Finding::info(
                        "phase_promoted_to_draft_saved",
                        format!(
                            "chapter {} scene {} exists in Spindle; promoted phase to draft_saved",
                            chapter.chapter_number, scene.scene_order
                        ),
                    ));
                }
                if scene.phase != ScenePhase::Pending && scene.scene_artifact_path.is_none() {
                    findings.push(Finding::error(
                        "missing_scene_artifact",
                        format!(
                            "chapter {} scene {} exists in Spindle but has no scene artifact path for safe resume",
                            chapter.chapter_number, scene.scene_order
                        ),
                    ));
                }
            }
            None => {
                if scene.phase != ScenePhase::Pending || scene.scene_id.is_some() {
                    findings.push(Finding::error(
                        "missing_persisted_scene",
                        format!(
                            "chapter {} scene {} is marked {:?} in state but no persisted scene exists on the active branch",
                            chapter.chapter_number, scene.scene_order, scene.phase
                        ),
                    ));
                }
            }
        }
    }

    for persisted in snapshot.scenes.values() {
        if chapter
            .scenes
            .iter()
            .all(|scene| scene.scene_order != persisted.scene_order)
        {
            findings.push(Finding::error(
                "unexpected_persisted_scene",
                format!(
                    "chapter {} has persisted scene order {} not represented in the harness manifest",
                    chapter.chapter_number, persisted.scene_order
                ),
            ));
        }
    }
}

fn reconcile_summary_state(
    chapter: &mut crate::state::ChapterState,
    summary_exists: bool,
    findings: &mut Vec<Finding>,
) {
    let all_beats_annotated = chapter
        .scenes
        .iter()
        .all(|scene| scene.phase == ScenePhase::BeatsAnnotated);

    if summary_exists {
        if all_beats_annotated {
            if !chapter.summary_saved {
                chapter.summary_saved = true;
                findings.push(Finding::info(
                    "summary_promoted",
                    format!(
                        "chapter {} already has a persisted summary; marked summary_saved",
                        chapter.chapter_number
                    ),
                ));
            }
        } else {
            findings.push(Finding::error(
                "summary_phase_mismatch",
                format!(
                    "chapter {} has a persisted summary but one or more scenes are not marked beats_annotated in harness state",
                    chapter.chapter_number
                ),
            ));
        }
    } else if chapter.summary_saved {
        findings.push(Finding::error(
            "missing_persisted_summary",
            format!(
                "chapter {} is marked summary_saved in state but no persisted summary exists",
                chapter.chapter_number
            ),
        ));
    }
}

fn validate_checkpoint_history(state: &HarnessState, findings: &mut Vec<Finding>) {
    let mut seen = BTreeSet::new();
    for checkpoint in &state.checkpoint_history {
        let key = (checkpoint.start_chapter, checkpoint.end_chapter);
        if !seen.insert(key) {
            findings.push(Finding::error(
                "duplicate_checkpoint_history",
                format!(
                    "duplicate checkpoint history entry for chapters {}-{}",
                    checkpoint.start_chapter, checkpoint.end_chapter
                ),
            ));
        }
        if checkpoint.start_chapter > checkpoint.end_chapter {
            findings.push(Finding::error(
                "invalid_checkpoint_history",
                format!(
                    "checkpoint history range {}-{} is invalid",
                    checkpoint.start_chapter, checkpoint.end_chapter
                ),
            ));
        }
        if checkpoint.save_point_id.trim().is_empty() {
            findings.push(Finding::error(
                "missing_checkpoint_save_point_id",
                format!(
                    "checkpoint {}-{} has an empty save_point_id",
                    checkpoint.start_chapter, checkpoint.end_chapter
                ),
            ));
        }
        if checkpoint.status == CheckpointStatus::PendingReview
            && checkpoint.report_artifact_path.is_none()
        {
            findings.push(Finding::error(
                "missing_checkpoint_report_artifact",
                format!(
                    "checkpoint {}-{} is pending review but has no report artifact path",
                    checkpoint.start_chapter, checkpoint.end_chapter
                ),
            ));
        }
        if !state.range.contains(checkpoint.start_chapter)
            || !state.range.contains(checkpoint.end_chapter)
        {
            findings.push(Finding::error(
                "checkpoint_out_of_range",
                format!(
                    "checkpoint {}-{} falls outside configured range {}-{}",
                    checkpoint.start_chapter,
                    checkpoint.end_chapter,
                    state.range.start_chapter,
                    state.range.end_chapter
                ),
            ));
        }
    }
}

fn validate_completion_order(state: &HarnessState, findings: &mut Vec<Finding>) {
    let mut saw_incomplete = false;
    for chapter in &state.chapters {
        if !chapter.summary_saved {
            saw_incomplete = true;
            continue;
        }
        if saw_incomplete {
            findings.push(Finding::error(
                "completion_gap",
                format!(
                    "chapter {} is marked complete after an incomplete earlier chapter",
                    chapter.chapter_number
                ),
            ));
        }
    }
}

fn determine_next_action(state: &HarnessState) -> NextAction {
    if let Some(checkpoint) = state
        .checkpoint_history
        .iter()
        .find(|checkpoint| checkpoint.status == CheckpointStatus::PendingReview)
    {
        return NextAction::AwaitCheckpointReview {
            start_chapter: checkpoint.start_chapter,
            end_chapter: checkpoint.end_chapter,
            save_point_id: checkpoint.save_point_id.clone(),
        };
    }

    let completed_since_checkpoint = contiguous_completed_after_last_checkpoint(state);
    if completed_since_checkpoint.len() >= state.checkpoint_interval {
        return NextAction::RunCheckpoint {
            start_chapter: completed_since_checkpoint[0],
            end_chapter: completed_since_checkpoint[state.checkpoint_interval - 1],
        };
    }

    for chapter in &state.chapters {
        if chapter.summary_saved {
            continue;
        }

        for scene in &chapter.scenes {
            match scene.phase {
                ScenePhase::Pending => {
                    return NextAction::DraftScene {
                        chapter_number: chapter.chapter_number,
                        scene_order: scene.scene_order,
                    };
                }
                ScenePhase::DraftSaved => {
                    return NextAction::CommitSceneChanges {
                        chapter_number: chapter.chapter_number,
                        scene_order: scene.scene_order,
                        scene_id: scene
                            .scene_id
                            .clone()
                            .unwrap_or_else(|| "<missing-scene-id>".to_string()),
                    };
                }
                ScenePhase::ChangesCommitted => {
                    return NextAction::AnnotateSceneBeats {
                        chapter_number: chapter.chapter_number,
                        scene_order: scene.scene_order,
                        scene_id: scene
                            .scene_id
                            .clone()
                            .unwrap_or_else(|| "<missing-scene-id>".to_string()),
                    };
                }
                ScenePhase::BeatsAnnotated => {}
            }
        }

        return NextAction::SaveChapterSummary {
            chapter_number: chapter.chapter_number,
        };
    }

    if !completed_since_checkpoint.is_empty() {
        return NextAction::RunCheckpoint {
            start_chapter: completed_since_checkpoint[0],
            end_chapter: *completed_since_checkpoint
                .last()
                .expect("non-empty completed_since_checkpoint"),
        };
    }

    NextAction::Complete
}

fn contiguous_completed_after_last_checkpoint(state: &HarnessState) -> Vec<i32> {
    let mut completed = Vec::new();
    let mut expected = state.last_checkpoint_end_chapter + 1;
    for chapter in &state.chapters {
        if chapter.chapter_number < expected {
            continue;
        }
        if chapter.chapter_number != expected || !chapter.summary_saved {
            break;
        }
        completed.push(chapter.chapter_number);
        expected += 1;
    }
    completed
}

fn chapter_numbers_in_range(range: &ChapterRange) -> Vec<i32> {
    (range.start_chapter..=range.end_chapter).collect()
}

#[cfg(test)]
mod tests {
    use spindle_core::models::ContentRating;

    use super::*;
    use crate::state::{ChapterSeed, HarnessSeed, SceneSeed};

    fn seed() -> HarnessSeed {
        HarnessSeed {
            project_id: "project:test".to_string(),
            book_number: 1,
            range: ChapterRange {
                start_chapter: 1,
                end_chapter: 2,
            },
            checkpoint_interval: 1,
            editorial_directives: vec![],
            chapters: vec![
                ChapterSeed {
                    chapter_number: 1,
                    synopsis: "First".to_string(),
                    pov_character_id: Some("character:pov".to_string()),
                    scenes: vec![SceneSeed {
                        scene_order: 1,
                        character_ids: vec!["character:pov".to_string()],
                        location_id: "location:a".to_string(),
                        content_rating: ContentRating::Teen,
                        tone: Some("tense".to_string()),
                        source_path: None,
                    }],
                },
                ChapterSeed {
                    chapter_number: 2,
                    synopsis: "Second".to_string(),
                    pov_character_id: Some("character:pov".to_string()),
                    scenes: vec![SceneSeed {
                        scene_order: 1,
                        character_ids: vec!["character:pov".to_string()],
                        location_id: "location:b".to_string(),
                        content_rating: ContentRating::Teen,
                        tone: Some("grim".to_string()),
                        source_path: None,
                    }],
                },
            ],
        }
    }

    fn snapshot() -> ProjectSnapshot {
        ProjectSnapshot {
            active_branch_id: "bible_branch:main".to_string(),
            active_branch_name: "main".to_string(),
            chapters: BTreeMap::from([
                (
                    1,
                    ChapterSnapshot {
                        chapter_id: "chapter:1".to_string(),
                        scenes: BTreeMap::new(),
                        chapter_plan: Some(ChapterPlanSnapshot {
                            synopsis: "First".to_string(),
                            pov_character_id: Some("character:pov".to_string()),
                            scenes: vec![PlannedSceneSnapshot {
                                scene_order: 1,
                                character_ids: vec!["character:pov".to_string()],
                            }],
                        }),
                    },
                ),
                (
                    2,
                    ChapterSnapshot {
                        chapter_id: "chapter:2".to_string(),
                        scenes: BTreeMap::new(),
                        chapter_plan: Some(ChapterPlanSnapshot {
                            synopsis: "Second".to_string(),
                            pov_character_id: Some("character:pov".to_string()),
                            scenes: vec![PlannedSceneSnapshot {
                                scene_order: 1,
                                character_ids: vec!["character:pov".to_string()],
                            }],
                        }),
                    },
                ),
            ]),
            summarized_chapters: BTreeSet::new(),
        }
    }

    #[test]
    fn reconcile_promotes_existing_scene_to_draft_saved_and_blocks_without_artifact() {
        let state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());
        let mut snapshot = snapshot();
        snapshot
            .chapters
            .get_mut(&1)
            .expect("chapter 1")
            .scenes
            .insert(
                1,
                PersistedScene {
                    scene_id: "scene:1".to_string(),
                    scene_order: 1,
                },
            );

        let outcome = reconcile_state(state, &snapshot);
        assert!(outcome.has_errors());
        let chapter = outcome.state.chapter(1).expect("chapter");
        assert_eq!(chapter.scenes[0].phase, ScenePhase::DraftSaved);
        assert_eq!(chapter.scenes[0].scene_id.as_deref(), Some("scene:1"));
        assert_eq!(outcome.next_action, NextAction::Blocked);
        assert!(
            outcome
                .findings
                .iter()
                .any(|finding| { finding.code == "missing_scene_artifact" })
        );
    }

    #[test]
    fn reconcile_blocks_on_branch_mismatch() {
        let state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());
        let mut snapshot = snapshot();
        snapshot.active_branch_id = "bible_branch:alt".to_string();

        let outcome = reconcile_state(state, &snapshot);
        assert!(outcome.has_errors());
        assert_eq!(outcome.next_action, NextAction::Blocked);
    }

    #[test]
    fn completed_chapters_trigger_checkpoint() {
        let mut state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());
        state.checkpoint_interval = 1;
        let chapter = state.chapter_mut(1).expect("chapter 1");
        chapter.scenes[0].scene_id = Some("scene:1".to_string());
        chapter.scenes[0].phase = ScenePhase::BeatsAnnotated;
        chapter.scenes[0].scene_artifact_path =
            Some("scenes/chapter-0001/scene-001.json".to_string());
        chapter.summary_saved = true;
        chapter.recompute_status();
        let mut snapshot = snapshot();
        snapshot.summarized_chapters.insert(1);
        snapshot
            .chapters
            .get_mut(&1)
            .expect("chapter 1")
            .scenes
            .insert(
                1,
                PersistedScene {
                    scene_id: "scene:1".to_string(),
                    scene_order: 1,
                },
            );

        let outcome = reconcile_state(state, &snapshot);
        assert_eq!(
            outcome.next_action,
            NextAction::RunCheckpoint {
                start_chapter: 1,
                end_chapter: 1,
            }
        );
    }

    #[test]
    fn summary_without_beats_is_blocked() {
        let state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());
        let mut snapshot = snapshot();
        snapshot.summarized_chapters.insert(1);

        let outcome = reconcile_state(state, &snapshot);
        assert!(outcome.has_errors());
        assert_eq!(outcome.next_action, NextAction::Blocked);
    }

    #[test]
    fn draft_saved_scene_without_artifact_blocks() {
        let mut state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());
        let chapter = state.chapter_mut(1).expect("chapter 1");
        chapter.scenes[0].scene_id = Some("scene:1".to_string());
        chapter.scenes[0].phase = ScenePhase::DraftSaved;

        let mut snapshot = snapshot();
        snapshot
            .chapters
            .get_mut(&1)
            .expect("chapter 1")
            .scenes
            .insert(
                1,
                PersistedScene {
                    scene_id: "scene:1".to_string(),
                    scene_order: 1,
                },
            );

        let outcome = reconcile_state(state, &snapshot);
        assert!(outcome.has_errors());
        assert_eq!(outcome.next_action, NextAction::Blocked);
        assert!(
            outcome
                .findings
                .iter()
                .any(|finding| { finding.code == "missing_scene_artifact" })
        );
    }
}
