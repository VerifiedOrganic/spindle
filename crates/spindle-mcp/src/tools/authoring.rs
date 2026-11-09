use super::*;

#[cfg(test)]
pub(super) fn chapter_range_end(start_chapter: i32, chapter_count: i32) -> Option<i32> {
    chapter_count
        .checked_sub(1)
        .and_then(|c| start_chapter.checked_add(c))
}

pub(super) fn authoring_scene_indices(
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

pub(super) fn authoring_artifacts_root(state_path: &Path, state: &HarnessState) -> PathBuf {
    let parent = state_path.parent().unwrap_or_else(|| Path::new("."));
    parent.join(&state.artifacts_dir)
}

pub(super) fn authoring_state_path(data_dir: &Path, run_id: &str) -> PathBuf {
    spindle_adapters::workspace::runtime_dir(data_dir).join(format!(
        "authoring_run_{}_temp.json",
        run_id.replace(":", "_")
    ))
}

pub(super) fn map_harness_to_records(
    run_id: &str,
    state: &spindle_harness::state::HarnessState,
    status: &str,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
) -> (
    spindle_adapters::sqlite::records::AuthoringRun,
    Vec<spindle_adapters::sqlite::records::AuthoringRunChapter>,
    Vec<spindle_adapters::sqlite::records::AuthoringRunScene>,
    Vec<spindle_adapters::sqlite::records::AuthoringCheckpoint>,
) {
    let now = chrono::Utc::now();
    let run = spindle_adapters::sqlite::records::AuthoringRun {
        id: run_id.to_string(),
        project_id: state.project_id.clone(),
        active_branch_id: state.active_branch_id.clone(),
        book_number: state.book_number,
        start_chapter: state.range.start_chapter,
        end_chapter: state.range.end_chapter,
        checkpoint_interval: state.checkpoint_interval as i64,
        last_checkpoint_end_chapter: state.last_checkpoint_end_chapter,
        artifacts_dir: state.artifacts_dir.clone(),
        editorial_directives: state.editorial_directives.clone(),
        status: status.to_string(),
        created_at: created_at.unwrap_or(now),
        updated_at: now,
        mining_policy: state.mining_policy.clone(),
        max_revise_attempts: state.max_revise_attempts,
        checkpoint_policy: state.checkpoint_policy.clone(),
        replan_policy: state.replan_policy.clone(),
    };

    let mut chapters = Vec::new();
    let mut scenes = Vec::new();
    for ch in &state.chapters {
        let ch_status = match ch.status {
            spindle_harness::state::ChapterStatus::Pending => "pending",
            spindle_harness::state::ChapterStatus::InProgress => "in_progress",
            spindle_harness::state::ChapterStatus::Complete => "complete",
        };
        chapters.push(spindle_adapters::sqlite::records::AuthoringRunChapter {
            authoring_run_id: run_id.to_string(),
            chapter_number: ch.chapter_number,
            planned: ch.planned,
            synopsis: ch.synopsis.clone(),
            pov_character_id: ch.pov_character_id.clone(),
            status: ch_status.to_string(),
            summary_saved: ch.summary_saved,
            summary_artifact_path: ch.summary_artifact_path.clone(),
            replan_status: ch.replan_status.clone(),
            replan_detail: ch.replan_detail.clone(),
        });

        for sc in &ch.scenes {
            let sc_phase = match sc.phase {
                spindle_harness::state::ScenePhase::Pending => "pending",
                spindle_harness::state::ScenePhase::DraftSaved => "draft_saved",
                spindle_harness::state::ScenePhase::ChangesCommitted => "changes_committed",
                spindle_harness::state::ScenePhase::BeatsAnnotated => "beats_annotated",
            };
            scenes.push(spindle_adapters::sqlite::records::AuthoringRunScene {
                authoring_run_id: run_id.to_string(),
                chapter_number: ch.chapter_number,
                scene_order: sc.scene_order,
                character_ids: sc.character_ids.clone(),
                location_id: sc.location_id.clone(),
                content_rating: sc.content_rating.as_str().to_string(),
                tone: sc.tone.clone(),
                source_path: sc.source_path.clone(),
                phase: sc_phase.to_string(),
                scene_id: sc.scene_id.clone(),
                scene_artifact_path: sc.scene_artifact_path.clone(),
                draft_diagnostics: sc
                    .draft_diagnostics
                    .as_ref()
                    .map(|d| serde_json::to_value(d).unwrap()),
                blocked_reason: sc.blocked_reason.clone(),
                research_required: sc.research_required,
                research_tags: sc.research_tags.clone(),
                explicit_query: sc.explicit_query.clone(),
                mine_status: sc.mine_status.clone(),
                mine_detail: sc.mine_detail.clone(),
                verify_status: sc.verify_status.clone(),
                verify_detail: sc.verify_detail.clone(),
                revise_attempts: sc.revise_attempts,
                // The revision-directives block is transient harness state only;
                // it is never persisted to the run tables (evolution §3.2).
                last_finding_fingerprint: sc.last_finding_fingerprint.clone(),
            });
        }
    }

    let mut checkpoints = Vec::new();
    for cp in &state.checkpoint_history {
        let cp_status = match cp.status {
            spindle_harness::state::CheckpointStatus::PendingReview => "pending_review",
            spindle_harness::state::CheckpointStatus::Reviewed => "reviewed",
        };
        checkpoints.push(spindle_adapters::sqlite::records::AuthoringCheckpoint {
            authoring_run_id: run_id.to_string(),
            start_chapter: cp.start_chapter,
            end_chapter: cp.end_chapter,
            save_point_id: cp.save_point_id.clone(),
            status: cp_status.to_string(),
            report_artifact_path: cp.report_artifact_path.clone(),
            auto_outcome: cp.auto_outcome.clone(),
            pending_manual_scene_ids: cp.pending_manual_scene_ids.clone(),
        });
    }

    (run, chapters, scenes, checkpoints)
}

pub(super) fn map_records_to_harness(
    run: &spindle_adapters::sqlite::records::AuthoringRun,
    chapters: &[spindle_adapters::sqlite::records::AuthoringRunChapter],
    scenes: &[spindle_adapters::sqlite::records::AuthoringRunScene],
    checkpoints: &[spindle_adapters::sqlite::records::AuthoringCheckpoint],
) -> spindle_harness::state::HarnessState {
    let mut ch_states = Vec::new();
    for ch in chapters {
        let mut ch_scenes = Vec::new();
        for sc in scenes {
            if sc.chapter_number == ch.chapter_number {
                let phase = match sc.phase.as_str() {
                    "pending" => spindle_harness::state::ScenePhase::Pending,
                    "draft_saved" => spindle_harness::state::ScenePhase::DraftSaved,
                    "changes_committed" => spindle_harness::state::ScenePhase::ChangesCommitted,
                    "beats_annotated" => spindle_harness::state::ScenePhase::BeatsAnnotated,
                    _ => spindle_harness::state::ScenePhase::Pending,
                };
                let content_rating = match sc.content_rating.to_ascii_lowercase().as_str() {
                    "general" => spindle_core::models::ContentRating::General,
                    "teen" => spindle_core::models::ContentRating::Teen,
                    "mature" => spindle_core::models::ContentRating::Mature,
                    "explicit" => spindle_core::models::ContentRating::Explicit,
                    _ => spindle_core::models::ContentRating::General,
                };
                let draft_diagnostics = sc
                    .draft_diagnostics
                    .as_ref()
                    .and_then(|v| serde_json::from_value(v.clone()).ok());
                ch_scenes.push(spindle_harness::state::SceneState {
                    scene_order: sc.scene_order,
                    character_ids: sc.character_ids.clone(),
                    location_id: sc.location_id.clone(),
                    content_rating,
                    tone: sc.tone.clone(),
                    source_path: sc.source_path.clone(),
                    phase,
                    scene_id: sc.scene_id.clone(),
                    scene_artifact_path: sc.scene_artifact_path.clone(),
                    draft_diagnostics,
                    blocked_reason: sc.blocked_reason.clone(),
                    research_required: sc.research_required,
                    research_tags: sc.research_tags.clone(),
                    explicit_query: sc.explicit_query.clone(),
                    mine_status: sc.mine_status.clone(),
                    mine_detail: sc.mine_detail.clone(),
                    verify_status: sc.verify_status.clone(),
                    verify_detail: sc.verify_detail.clone(),
                    revise_attempts: sc.revise_attempts,
                    last_finding_fingerprint: sc.last_finding_fingerprint.clone(),
                    ..Default::default()
                });
            }
        }
        let status = match ch.status.as_str() {
            "pending" => spindle_harness::state::ChapterStatus::Pending,
            "in_progress" => spindle_harness::state::ChapterStatus::InProgress,
            "complete" => spindle_harness::state::ChapterStatus::Complete,
            _ => spindle_harness::state::ChapterStatus::Pending,
        };
        ch_states.push(spindle_harness::state::ChapterState {
            chapter_number: ch.chapter_number,
            planned: ch.planned,
            synopsis: ch.synopsis.clone(),
            pov_character_id: ch.pov_character_id.clone(),
            status,
            scenes: ch_scenes,
            summary_saved: ch.summary_saved,
            summary_artifact_path: ch.summary_artifact_path.clone(),
            replan_status: ch.replan_status.clone(),
            replan_detail: ch.replan_detail.clone(),
        });
    }

    let mut cp_history = Vec::new();
    for cp in checkpoints {
        let status = match cp.status.as_str() {
            "pending_review" => spindle_harness::state::CheckpointStatus::PendingReview,
            "reviewed" => spindle_harness::state::CheckpointStatus::Reviewed,
            _ => spindle_harness::state::CheckpointStatus::PendingReview,
        };
        cp_history.push(spindle_harness::state::CheckpointRecord {
            start_chapter: cp.start_chapter,
            end_chapter: cp.end_chapter,
            save_point_id: cp.save_point_id.clone(),
            status,
            report_artifact_path: cp.report_artifact_path.clone(),
            auto_outcome: cp.auto_outcome.clone(),
            pending_manual_scene_ids: cp.pending_manual_scene_ids.clone(),
        });
    }

    let mut state = spindle_harness::state::HarnessState {
        project_id: run.project_id.clone(),
        active_branch_id: run.active_branch_id.clone(),
        book_number: run.book_number,
        range: spindle_harness::state::ChapterRange {
            start_chapter: run.start_chapter,
            end_chapter: run.end_chapter,
        },
        checkpoint_interval: run.checkpoint_interval as usize,
        last_checkpoint_end_chapter: run.last_checkpoint_end_chapter,
        artifacts_dir: run.artifacts_dir.clone(),
        editorial_directives: run.editorial_directives.clone(),
        mining_policy: run.mining_policy.clone(),
        max_revise_attempts: run.max_revise_attempts,
        checkpoint_policy: run.checkpoint_policy.clone(),
        replan_policy: run.replan_policy.clone(),
        resolved_editorial: None,
        editorial_controller_path: None,
        chapters: ch_states,
        checkpoint_history: cp_history,
    };

    if state.artifacts_dir == "artifacts" {
        state.artifacts_dir = "../artifacts".to_string();
    }
    state
}

impl ToolRouter {
    pub(super) async fn authoring_project_snapshot(
        &self,
        state: &HarnessState,
    ) -> anyhow::Result<ProjectSnapshot> {
        let repo = self.service.repository();
        let active_branch = repo.get_active_branch(&state.project_id).await?;
        let scenes = repo
            .list_scenes_by_project_and_branch(&state.project_id, &active_branch.id)
            .await?;
        let plans = repo
            .list_chapter_plans_by_project(&state.project_id)
            .await?;
        let summaries = repo
            .list_chapter_summaries_by_project(&state.project_id)
            .await?;
        let summarized_chapters = summaries
            .into_iter()
            .filter(|summary| {
                summary.branch_id == active_branch.id && summary.book_number == state.book_number
            })
            .map(|summary| summary.chapter_number)
            .collect::<BTreeSet<_>>();

        let mut chapters = BTreeMap::new();
        for chapter in &state.chapters {
            let chapter_row = repo
                .get_chapter_by_number(&state.project_id, state.book_number, chapter.chapter_number)
                .await
                .with_context(|| {
                    format!(
                        "failed to resolve chapter {} for authoring snapshot",
                        chapter.chapter_number
                    )
                })?;

            let persisted_scenes = scenes
                .iter()
                .filter(|scene| {
                    scene.book_number == state.book_number
                        && scene.chapter_number == chapter.chapter_number
                })
                .map(|scene| {
                    (
                        scene.scene_order,
                        PersistedScene {
                            scene_id: scene.id.clone(),
                            scene_order: scene.scene_order,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();

            let mut chapter_plan = None;
            if let Some(plan) = plans.iter().find(|plan| {
                plan.branch_id == active_branch.id
                    && plan.book_number == state.book_number
                    && plan.chapter_number == chapter.chapter_number
            }) {
                let mut planned_scenes = Vec::new();
                for planned in &plan.scenes {
                    let harness_scene = chapter
                        .scenes
                        .iter()
                        .find(|scene| scene.scene_order == planned.scene_order);
                    let scene_location = planned
                        .location_id
                        .clone()
                        .or_else(|| harness_scene.map(|scene| scene.location_id.clone()));
                    let research_tags = planned.research_tags.clone();
                    let explicit_query = planned.explicit_query.clone();

                    let pack = self
                        .service
                        .research_pack_for_scene(ResearchPackForSceneInput {
                            project_id: state.project_id.clone(),
                            branch_id: Some(active_branch.id.clone()),
                            scene_summary: Some(planned.summary.clone()),
                            scene_location,
                            character_ids: planned.character_ids.clone(),
                            tags: research_tags.clone(),
                            explicit_query: explicit_query.clone(),
                            limit: Some(10),
                        })
                        .await
                        .unwrap_or(ResearchPackForSceneOutput {
                            sources: vec![],
                            notes: vec![],
                            claims: vec![],
                        });

                    let research_pack_empty =
                        pack.sources.is_empty() && pack.notes.is_empty() && pack.claims.is_empty();
                    let research_tags_matched = if research_tags.is_empty() {
                        true
                    } else {
                        research_tags.iter().any(|tag| {
                            let tag_lower = tag.to_lowercase();
                            pack.sources.iter().any(|source| {
                                source
                                    .tags
                                    .iter()
                                    .any(|source_tag| source_tag.to_lowercase() == tag_lower)
                            }) || pack.notes.iter().any(|note| {
                                note.tags
                                    .iter()
                                    .any(|note_tag| note_tag.to_lowercase() == tag_lower)
                            }) || pack.claims.iter().any(|claim| {
                                claim
                                    .tags
                                    .iter()
                                    .any(|claim_tag| claim_tag.to_lowercase() == tag_lower)
                            })
                        })
                    };

                    planned_scenes.push(PlannedSceneSnapshot {
                        scene_order: planned.scene_order,
                        character_ids: planned.character_ids.clone(),
                        research_required: planned.research_required,
                        research_tags,
                        explicit_query,
                        research_pack_empty,
                        research_tags_matched,
                    });
                }

                chapter_plan = Some(ChapterPlanSnapshot {
                    synopsis: plan.synopsis.clone(),
                    pov_character_id: plan.pov_character_id.clone(),
                    scenes: planned_scenes,
                });
            }

            chapters.insert(
                chapter.chapter_number,
                ChapterSnapshot {
                    chapter_id: chapter_row.id,
                    scenes: persisted_scenes,
                    chapter_plan,
                },
            );
        }

        Ok(ProjectSnapshot {
            active_branch_id: active_branch.id,
            active_branch_name: active_branch.name,
            chapters,
            summarized_chapters,
        })
    }

    pub(super) async fn handle_authoring_prepare_run(
        &self,
        input: AuthoringPrepareRunInput,
    ) -> anyhow::Result<AuthoringPrepareRunOutput> {
        self.service.prepare_authoring_run(input).await
    }

    pub(super) async fn handle_authoring_start_run(
        &self,
        input: AuthoringStartRunInput,
    ) -> anyhow::Result<AuthoringStartRunOutput> {
        let project_id = input.project_id.clone();
        let repo = self.service.repository();

        if input.checkpoint_interval == 0 {
            return Ok(AuthoringStartRunOutput {
                run_id: String::new(),
                status: "blocked".to_string(),
                message: "Cannot start authoring run: checkpoint_interval must be at least 1"
                    .to_string(),
            });
        }

        // Canonical `mining_policy` is None (disabled/default) or Some("propose_all").
        // `Disabled` canonicalizes to None so the run persists NULL, matching
        // pre-upgrade runs exactly.
        let mining_policy = input
            .mining_policy
            .and_then(AuthoringMiningPolicy::into_persisted_option);

        // Validate the opt-in bounded-revise budget (evolution §3.2). None/0 =
        // disabled (canonicalized to None so the run persists NULL, matching a
        // pre-upgrade row); 1..=2 enable the in-run verify/revise loop; anything
        // else is an input error — a bound, not a knob explosion.
        let max_revise_attempts = match spindle_core::models::validate_max_revise_attempts(
            input.max_revise_attempts,
        ) {
            Ok(budget) => budget,
            Err(rejected) => {
                return Ok(AuthoringStartRunOutput {
                    run_id: String::new(),
                    status: "blocked".to_string(),
                    message: format!(
                        "Cannot start authoring run: max_revise_attempts must be between 0 and {}, got {}",
                        spindle_core::models::MAX_REVISE_ATTEMPTS_UPPER_BOUND,
                        rejected
                    ),
                });
            }
        };

        // Canonical `checkpoint_policy` is None (manual/default) or
        // Some("auto_advisory" | "auto_strict"). `Manual` canonicalizes to None
        // so the run persists NULL, matching a pre-upgrade row exactly (I1).
        let checkpoint_policy = input
            .checkpoint_policy
            .and_then(AuthoringCheckpointPolicy::into_persisted_option);

        // Canonical `replan_policy` is None (disabled/default) or Some("propose_all").
        // `Disabled` canonicalizes to None so the run persists NULL.
        let replan_policy = input
            .replan_policy
            .and_then(AuthoringReplanPolicy::into_persisted_option);

        let prep_input = AuthoringPrepareRunInput {
            project_id: project_id.clone(),
            book_number: input.book_number,
            start_chapter: input.start_chapter,
            end_chapter: input.end_chapter,
            chapter_count: input.chapter_count,
            // Thread the resolved policy so prepare's mine-route preflight runs
            // only when the run actually opted into propose_all.
            mining_policy: input.mining_policy,
            // Threaded for symmetry; prepare adds NO preflight for revise (verify
            // is deterministic and revision reuses the draft route — §3.2).
            max_revise_attempts,
            // Thread the resolved policy so prepare's review-route preflight runs
            // only when the run actually opted into an auto checkpoint policy
            // (evolution §3.3 K2 precondition).
            checkpoint_policy: input.checkpoint_policy,
            // Threaded for symmetry; prepare adds NO preflight for replan (the
            // differ is non-prose-bearing and skips honestly on NoRoute — §3.5).
            replan_policy: input.replan_policy,
        };
        let prep_report = Box::pin(self.handle_authoring_prepare_run(prep_input)).await?;
        if !prep_report.ready_to_draft {
            return Ok(AuthoringStartRunOutput {
                run_id: "".to_string(),
                status: "blocked".to_string(),
                message: format!(
                    "Cannot start authoring run due to missing requirements: {:?}",
                    prep_report.missing_requirements
                ),
            });
        }

        let active_branch = repo.get_active_branch(&project_id).await?;
        let end_chapter = prep_report.end_chapter;

        let db_plans = repo.list_chapter_plans_by_project(&project_id).await?;
        let mut chapter_seeds = Vec::new();

        for ch_num in input.start_chapter..=end_chapter {
            let plan = db_plans
                .iter()
                .find(|p| {
                    p.book_number == input.book_number
                        && p.chapter_number == ch_num
                        && p.branch_id == active_branch.id
                })
                .context("plan not found")?;

            let mut scene_seeds = Vec::new();
            for scene in &plan.scenes {
                let (loc_id, rating) = require_planned_scene_location_and_rating(
                    ch_num,
                    scene.scene_order,
                    scene.location_id.as_deref(),
                    scene.content_rating.as_ref(),
                    &scene.summary,
                    &scene.purpose,
                )?;

                scene_seeds.push(spindle_harness::state::SceneSeed {
                    scene_order: scene.scene_order,
                    character_ids: scene.character_ids.clone(),
                    location_id: loc_id,
                    content_rating: rating,
                    tone: None,
                    source_path: None,
                    research_required: scene.research_required,
                    research_tags: scene.research_tags.clone(),
                    explicit_query: scene.explicit_query.clone(),
                });
            }

            chapter_seeds.push(spindle_harness::state::ChapterSeed {
                chapter_number: ch_num,
                synopsis: plan.synopsis.clone(),
                pov_character_id: plan.pov_character_id.clone(),
                scenes: scene_seeds,
            });
        }

        let seed = spindle_harness::state::HarnessSeed {
            project_id: project_id.clone(),
            book_number: input.book_number,
            range: spindle_harness::state::ChapterRange {
                start_chapter: input.start_chapter,
                end_chapter,
            },
            checkpoint_interval: input.checkpoint_interval,
            editorial_directives: input.editorial_directives.unwrap_or_default(),
            mining_policy,
            max_revise_attempts,
            checkpoint_policy,
            replan_policy,
            editorial: None,
            editorial_controller_path: None,
            chapters: chapter_seeds,
        };

        let mut harness_state =
            spindle_harness::state::HarnessState::from_seed(seed, active_branch.id.clone());
        harness_state.artifacts_dir = "../artifacts".to_string();

        let run_id = format!(
            "authoring_run:{}",
            ulid::Ulid::new().to_string().to_lowercase()
        );
        let (run, chapters, scenes, checkpoints) =
            map_harness_to_records(&run_id, &harness_state, "active", None);

        repo.save_authoring_run(run, chapters, scenes, checkpoints)
            .await?;

        // Journal: the run persisted (ADR D2 `run_started`). Emitted AFTER the
        // state change commits; a journal error never fails start_run (D3.3).
        RunJournal::new(repo)
            .emit(
                &run_id,
                "run_started",
                run_journal::run_started_payload(
                    input.book_number,
                    input.start_chapter,
                    end_chapter,
                    None,
                    harness_state.mining_policy.as_deref(),
                    harness_state.max_revise_attempts,
                ),
            )
            .await;

        Ok(AuthoringStartRunOutput {
            run_id: run_id.clone(),
            status: "active".to_string(),
            message: format!(
                "Started authoring run {} for chapters {}-{}",
                run_id, input.start_chapter, end_chapter
            ),
        })
    }

    pub(super) async fn handle_authoring_resolve_block(
        &self,
        input: AuthoringResolveBlockInput,
    ) -> anyhow::Result<AuthoringResolveBlockOutput> {
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

        let mut harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

        // `redraft` is not a forward phase: it resets a poisoned/unparseable
        // scene BACK to pending-draft so the next execute re-drafts fresh (BUG
        // 3). The forward phases stay unchanged; anything else errors, and the
        // whitelist message now lists redraft.
        let target = input.target_phase.to_ascii_lowercase();
        let parsed_phase = match target.as_str() {
            "pending" => Some(spindle_harness::state::ScenePhase::Pending),
            "draft_saved" => Some(spindle_harness::state::ScenePhase::DraftSaved),
            "changes_committed" => Some(spindle_harness::state::ScenePhase::ChangesCommitted),
            "beats_annotated" => Some(spindle_harness::state::ScenePhase::BeatsAnnotated),
            "redraft" => None,
            _ => anyhow::bail!(
                "Invalid target phase: {} (expected one of: pending, draft_saved, changes_committed, beats_annotated, redraft)",
                input.target_phase
            ),
        };

        let data_dir = repo.data_dir();
        let state_path = authoring_state_path(data_dir, &run_id);
        harness_state.save(&state_path)?;

        // Distinguish scene-level from run-level blocks (defect item 1c): the
        // reconcile pass against live Spindle state is what raises run-level
        // errors (residue rules, branch mismatch, …). When it has errors, a
        // forward advance is allowed even on a scene with no blocked_reason.
        let run_level_blocked = {
            let snapshot = self.authoring_project_snapshot(&harness_state).await?;
            reconcile_state(harness_state.clone(), &snapshot).has_errors()
        };

        let resolve_result = match parsed_phase {
            Some(phase) => spindle_harness::operator::resolve_scene_block(
                &mut harness_state,
                &state_path,
                input.chapter_number,
                input.scene_order,
                phase,
                run_level_blocked,
            ),
            None => spindle_harness::operator::redraft_scene_block(
                &mut harness_state,
                &state_path,
                input.chapter_number,
                input.scene_order,
            ),
        };

        let _ = std::fs::remove_file(&state_path);

        let message = resolve_result?;

        let snapshot = self.authoring_project_snapshot(&harness_state).await?;
        let outcome = reconcile_state(harness_state.clone(), &snapshot);

        let mut final_status = "active".to_string();
        if outcome.has_errors() {
            final_status = "blocked".to_string();
        } else {
            for ch in &harness_state.chapters {
                for sc in &ch.scenes {
                    if sc.blocked_reason.is_some() {
                        final_status = "blocked".to_string();
                    }
                }
            }
        }
        if outcome.next_action == NextAction::Complete {
            final_status = "completed".to_string();
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &final_status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal: resolve_block emits NOTHING — neither the pre-existing forward
        // phases nor `redraft` journal a run event. The ADR D2 vocabulary has no
        // kind for an operator manual scene reset, `pass_skipped` is for a run
        // PASS being skipped (not an operator action), and inventing a kind is a
        // one-way door (D3.4). `authoring_status` (DB state) stays the source of
        // truth, so the reset is observable there without a journal entry.

        Ok(AuthoringResolveBlockOutput {
            run_id,
            message,
            status: final_status,
        })
    }

    pub(super) async fn handle_authoring_cancel_run(
        &self,
        input: AuthoringCancelRunInput,
    ) -> anyhow::Result<AuthoringCancelRunOutput> {
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

        let status = "paused".to_string();
        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &status, Some(run.created_at));

        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2): a genuine status transition into `paused`. Only emit
        // when the status actually changed (diff prev vs new — not on a no-op
        // re-pause). Emitted AFTER the save commits (D3.3).
        if run.status != status {
            RunJournal::new(repo)
                .emit(&run_id, "run_paused", run_journal::run_status_payload(None))
                .await;
        }

        let message = format!("Successfully paused authoring run {}", run_id);
        Ok(AuthoringCancelRunOutput {
            run_id,
            message,
            status,
        })
    }
}
