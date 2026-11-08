use super::*;

impl ToolRouter {
    pub(super) async fn handle_authoring_status(
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
    pub(super) fn assemble_authoring_status_output(
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
            let (sampled_review_status, sampled_review_count) = cp
                .report_artifact_path
                .as_deref()
                .and_then(|rel| read_checkpoint_sampled_review_summary(&status_artifacts_root, rel))
                .unwrap_or((None, None));
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
                sampled_review_status,
                sampled_review_count,
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
}
