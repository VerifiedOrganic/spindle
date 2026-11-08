use super::*;

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
pub(super) async fn authoring_emit_step_events(
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
                            None,
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
                            None,
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
