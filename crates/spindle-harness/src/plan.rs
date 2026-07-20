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
    pub research_required: Option<bool>,
    pub research_tags: Vec<String>,
    pub explicit_query: Option<String>,
    pub research_pack_empty: bool,
    pub research_tags_matched: bool,
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
    AwaitResearch {
        chapter_number: i32,
        scene_order: i32,
        missing_tags: Vec<String>,
        query: Option<String>,
        location: Option<String>,
    },
    VerifyScene {
        chapter_number: i32,
        scene_order: i32,
    },
    ReviseScene {
        chapter_number: i32,
        scene_order: i32,
        attempt: i32,
    },
    CommitSceneChanges {
        chapter_number: i32,
        scene_order: i32,
        scene_id: String,
    },
    MineScene {
        chapter_number: i32,
        scene_order: i32,
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
            Self::AwaitResearch {
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
                let query_str = query.as_deref().unwrap_or("none");
                let location_str = location.as_deref().unwrap_or("none");
                write!(
                    f,
                    "await_research: chapter {chapter_number} scene {scene_order} needs research. \
                     Searched tags: [{tags_str}], query: \"{query_str}\", location: \"{location_str}\". \
                     Suggested action: use research tools (e.g. research_add_source, research_add_note, research_add_claim) to add relevant research, then resume."
                )
            }
            Self::VerifyScene {
                chapter_number,
                scene_order,
            } => write!(
                f,
                "verify scene for chapter {chapter_number} scene {scene_order}"
            ),
            Self::ReviseScene {
                chapter_number,
                scene_order,
                attempt,
            } => write!(
                f,
                "revise scene for chapter {chapter_number} scene {scene_order} (attempt {attempt})"
            ),
            Self::CommitSceneChanges {
                chapter_number,
                scene_order,
                scene_id,
            } => write!(
                f,
                "commit scene changes for chapter {chapter_number} scene {scene_order} ({scene_id})"
            ),
            Self::MineScene {
                chapter_number,
                scene_order,
            } => write!(
                f,
                "mine canon for chapter {chapter_number} scene {scene_order}"
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
        }
    }

    findings
}

fn reconcile_chapter_plan(
    chapter: &mut crate::state::ChapterState,
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
    for scene in &mut chapter.scenes {
        let Some(plan_scene) = plan_by_order.get(&scene.scene_order) else {
            continue;
        };
        scene.research_required = plan_scene.research_required;
        scene.research_tags = plan_scene.research_tags.clone();
        scene.explicit_query = plan_scene.explicit_query.clone();
        scene.research_pack_empty = plan_scene.research_pack_empty;
        scene.research_tags_matched = plan_scene.research_tags_matched;

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
                    let is_required = scene.research_required.unwrap_or(false);
                    let has_required_tags = !scene.research_tags.is_empty();

                    if (is_required && scene.research_pack_empty)
                        || (has_required_tags && !scene.research_tags_matched)
                    {
                        return NextAction::AwaitResearch {
                            chapter_number: chapter.chapter_number,
                            scene_order: scene.scene_order,
                            missing_tags: scene.research_tags.clone(),
                            query: scene.explicit_query.clone(),
                            location: Some(scene.location_id.clone()),
                        };
                    }

                    return NextAction::DraftScene {
                        chapter_number: chapter.chapter_number,
                        scene_order: scene.scene_order,
                    };
                }
                ScenePhase::DraftSaved => {
                    // Opt-in in-run verify/revise sits between draft and commit
                    // (evolution §3.2, I7). When the run set a revise budget the
                    // saved draft is verified first; warning-or-worse findings
                    // drive up to `max` bounded revisions before commit. With the
                    // budget disabled (None/0) scheduling is byte-identical to the
                    // pre-verify loop: straight to CommitSceneChanges.
                    if let Some(max) = revise_budget(state) {
                        match scene.verify_status.as_deref() {
                            // Not yet verified: run the deterministic scene check.
                            None => {
                                return NextAction::VerifyScene {
                                    chapter_number: chapter.chapter_number,
                                    scene_order: scene.scene_order,
                                };
                            }
                            // Findings with budget left: revise, then re-verify.
                            // If the budget is spent but state still reads
                            // "findings" (the executor should have parked), treat
                            // it defensively as parked and commit — never loop.
                            Some("findings") if scene.revise_attempts < max => {
                                return NextAction::ReviseScene {
                                    chapter_number: chapter.chapter_number,
                                    scene_order: scene.scene_order,
                                    attempt: scene.revise_attempts + 1,
                                };
                            }
                            // clean | parked_findings | error | findings-exhausted:
                            // fall through to commit.
                            _ => {}
                        }
                    }
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
                    // Opt-in canon mining sits between commit and beats
                    // (evolution §3.1, I7). Only when the run opted into
                    // `propose_all` AND this scene has not been mined yet does
                    // the scheduler yield MineScene; otherwise scheduling is
                    // byte-identical to the pre-mining loop. A `Some(_)`
                    // mine_status (any outcome) means the pass already ran, so
                    // it never re-fires.
                    if mining_enabled(state) && scene.mine_status.is_none() {
                        return NextAction::MineScene {
                            chapter_number: chapter.chapter_number,
                            scene_order: scene.scene_order,
                        };
                    }
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

/// True when the run opted into canon mining (`mining_policy == "propose_all"`).
/// `None` (pre-upgrade / default) and any other value — including the explicit
/// `"disabled"` — leave the loop exactly as it behaved before mining existed.
/// The policy string is validated at `authoring_start_run`; the scheduler is
/// deliberately lenient so an unknown value never diverts the loop.
fn mining_enabled(state: &HarnessState) -> bool {
    state.mining_policy.as_deref() == Some("propose_all")
}

/// The run's bounded in-run revise budget, or `None` when the loop is disabled
/// (evolution §3.2). `None` (pre-upgrade / default) and `Some(0)` both mean
/// "no verify/revise step" so the loop is byte-identical to before; a positive
/// budget is the max number of `ReviseScene` passes a scene may take. The value
/// is validated `0..=2` at `authoring_start_run`; the scheduler is lenient so an
/// out-of-band value never strands the loop.
fn revise_budget(state: &HarnessState) -> Option<i32> {
    match state.max_revise_attempts {
        Some(n) if n > 0 => Some(n),
        _ => None,
    }
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
                        ..Default::default()
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
                        ..Default::default()
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
                                research_required: None,
                                research_tags: vec![],
                                explicit_query: None,
                                research_pack_empty: false,
                                research_tags_matched: true,
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
                                research_required: None,
                                research_tags: vec![],
                                explicit_query: None,
                                research_pack_empty: false,
                                research_tags_matched: true,
                            }],
                        }),
                    },
                ),
            ]),
            summarized_chapters: BTreeSet::new(),
        }
    }

    #[test]
    fn reconcile_promotes_existing_scene_to_draft_saved_without_artifact() {
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
        assert!(!outcome.has_errors(), "{:?}", outcome.findings);
        let chapter = outcome.state.chapter(1).expect("chapter");
        assert_eq!(chapter.scenes[0].phase, ScenePhase::DraftSaved);
        assert_eq!(chapter.scenes[0].scene_id.as_deref(), Some("scene:1"));
        assert_eq!(
            outcome.next_action,
            NextAction::CommitSceneChanges {
                chapter_number: 1,
                scene_order: 1,
                scene_id: "scene:1".to_string(),
            }
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
    fn draft_saved_scene_without_artifact_can_commit() {
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
        assert!(!outcome.has_errors(), "{:?}", outcome.findings);
        assert_eq!(
            outcome.next_action,
            NextAction::CommitSceneChanges {
                chapter_number: 1,
                scene_order: 1,
                scene_id: "scene:1".to_string(),
            }
        );
    }

    /// Advance a chapter-1 scene to `ChangesCommitted` and return its state so
    /// the mining-scheduler tests can toggle `mining_policy` / `mine_status`
    /// against a single committed scene without re-deriving the whole fixture.
    fn committed_state(mining_policy: Option<&str>, mine_status: Option<&str>) -> HarnessState {
        let mut state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());
        state.mining_policy = mining_policy.map(str::to_string);
        let chapter = state.chapter_mut(1).expect("chapter 1");
        chapter.scenes[0].scene_id = Some("scene:1".to_string());
        chapter.scenes[0].phase = ScenePhase::ChangesCommitted;
        chapter.scenes[0].mine_status = mine_status.map(str::to_string);
        state
    }

    #[test]
    fn propose_all_committed_scene_without_mine_status_schedules_mine_before_beats() {
        let state = committed_state(Some("propose_all"), None);
        assert_eq!(
            determine_next_action(&state),
            NextAction::MineScene {
                chapter_number: 1,
                scene_order: 1,
            }
        );
    }

    #[test]
    fn propose_all_committed_scene_with_mine_status_schedules_beats() {
        let state = committed_state(Some("propose_all"), Some("staged"));
        assert_eq!(
            determine_next_action(&state),
            NextAction::AnnotateSceneBeats {
                chapter_number: 1,
                scene_order: 1,
                scene_id: "scene:1".to_string(),
            }
        );
    }

    #[test]
    fn disabled_policy_committed_scene_schedules_beats_never_mine() {
        // policy None (pre-upgrade / default) and policy "disabled" both keep the
        // existing byte-identical schedule: straight to AnnotateSceneBeats.
        for policy in [None, Some("disabled")] {
            let state = committed_state(policy, None);
            assert_eq!(
                determine_next_action(&state),
                NextAction::AnnotateSceneBeats {
                    chapter_number: 1,
                    scene_order: 1,
                    scene_id: "scene:1".to_string(),
                },
                "policy {policy:?} must not schedule MineScene"
            );
        }
    }

    // ── P2.2 in-run verify/revise scheduling (evolution §3.2) ──

    /// Advance a chapter-1 scene to `DraftSaved` and set the run's revise budget
    /// plus the scene's verify state, so the verify/revise scheduler tests can
    /// exercise the `DraftSaved` fork against one scene.
    fn draft_saved_state(
        max_revise_attempts: Option<i32>,
        verify_status: Option<&str>,
        revise_attempts: i32,
    ) -> HarnessState {
        let mut state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());
        state.max_revise_attempts = max_revise_attempts;
        let chapter = state.chapter_mut(1).expect("chapter 1");
        chapter.scenes[0].scene_id = Some("scene:1".to_string());
        chapter.scenes[0].phase = ScenePhase::DraftSaved;
        chapter.scenes[0].verify_status = verify_status.map(str::to_string);
        chapter.scenes[0].revise_attempts = revise_attempts;
        state
    }

    #[test]
    fn revise_disabled_draft_saved_schedules_commit_byte_identical() {
        // None (pre-upgrade / default) and explicit 0 both keep the existing
        // schedule: DraftSaved -> CommitSceneChanges, no VerifyScene.
        for budget in [None, Some(0)] {
            let state = draft_saved_state(budget, None, 0);
            assert_eq!(
                determine_next_action(&state),
                NextAction::CommitSceneChanges {
                    chapter_number: 1,
                    scene_order: 1,
                    scene_id: "scene:1".to_string(),
                },
                "budget {budget:?} must go straight to commit"
            );
        }
    }

    #[test]
    fn enabled_draft_saved_without_verify_schedules_verify() {
        let state = draft_saved_state(Some(1), None, 0);
        assert_eq!(
            determine_next_action(&state),
            NextAction::VerifyScene {
                chapter_number: 1,
                scene_order: 1,
            }
        );
    }

    #[test]
    fn findings_with_attempts_remaining_schedules_revise_with_next_attempt() {
        let state = draft_saved_state(Some(2), Some("findings"), 0);
        assert_eq!(
            determine_next_action(&state),
            NextAction::ReviseScene {
                chapter_number: 1,
                scene_order: 1,
                attempt: 1,
            }
        );
    }

    #[test]
    fn clean_or_parked_or_error_verify_schedules_commit() {
        for status in ["clean", "parked_findings", "error"] {
            let state = draft_saved_state(Some(2), Some(status), 0);
            assert_eq!(
                determine_next_action(&state),
                NextAction::CommitSceneChanges {
                    chapter_number: 1,
                    scene_order: 1,
                    scene_id: "scene:1".to_string(),
                },
                "verify_status {status} must proceed to commit"
            );
        }
    }

    #[test]
    fn findings_with_attempts_exhausted_defensively_schedules_commit() {
        // Defensive: the executor parks on the last attempt, but if state still
        // reads "findings" with the budget spent, treat it as parked and commit
        // rather than looping forever.
        let state = draft_saved_state(Some(1), Some("findings"), 1);
        assert_eq!(
            determine_next_action(&state),
            NextAction::CommitSceneChanges {
                chapter_number: 1,
                scene_order: 1,
                scene_id: "scene:1".to_string(),
            }
        );
    }

    #[test]
    fn test_await_research_when_empty_or_unmatched_tags() {
        let mut state = HarnessState::from_seed(seed(), "bible_branch:main".to_string());

        // 1. With research required but pack empty
        {
            let chapter = state.chapter_mut(1).expect("chapter 1");
            chapter.scenes[0].research_required = Some(true);
            chapter.scenes[0].research_pack_empty = true;
            chapter.scenes[0].research_tags_matched = true;

            let mut snapshot = snapshot();
            if let Some(plan) = snapshot
                .chapters
                .get_mut(&1)
                .and_then(|ch| ch.chapter_plan.as_mut())
            {
                plan.scenes[0].research_required = Some(true);
                plan.scenes[0].research_pack_empty = true;
                plan.scenes[0].research_tags_matched = true;
            }

            let outcome = reconcile_state(state.clone(), &snapshot);
            assert!(matches!(
                outcome.next_action,
                NextAction::AwaitResearch {
                    chapter_number: 1,
                    scene_order: 1,
                    ..
                }
            ));
        }

        // 2. With required tags not matched
        {
            let chapter = state.chapter_mut(1).expect("chapter 1");
            chapter.scenes[0].research_required = Some(false);
            chapter.scenes[0].research_pack_empty = false;
            chapter.scenes[0].research_tags = vec!["tag1".to_string()];
            chapter.scenes[0].research_tags_matched = false;

            let mut snapshot = snapshot();
            if let Some(plan) = snapshot
                .chapters
                .get_mut(&1)
                .and_then(|ch| ch.chapter_plan.as_mut())
            {
                plan.scenes[0].research_required = Some(false);
                plan.scenes[0].research_pack_empty = false;
                plan.scenes[0].research_tags = vec!["tag1".to_string()];
                plan.scenes[0].research_tags_matched = false;
            }

            let outcome = reconcile_state(state.clone(), &snapshot);
            assert!(matches!(
                outcome.next_action,
                NextAction::AwaitResearch {
                    chapter_number: 1,
                    scene_order: 1,
                    ..
                }
            ));
        }
    }
}
