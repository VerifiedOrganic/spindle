use super::*;

pub(super) fn extract_scene_location_and_rating(
    summary: &str,
    purpose: &str,
) -> (Option<String>, Option<spindle_core::models::ContentRating>) {
    let combined = format!("{} {}", summary, purpose).to_ascii_lowercase();
    let combined_orig = format!("{} {}", summary, purpose);

    // 1. Extract location
    let location_id = if let Some(idx) = combined.find("location:") {
        let start = idx + "location:".len();
        let remaining = &combined[start..];
        let end = remaining
            .find(|c: char| !c.is_alphanumeric() && c != '_' && c != '-' && c != ':')
            .unwrap_or(remaining.len());
        let id_str = &combined_orig[idx..idx + "location:".len() + end];
        Some(id_str.trim().to_string())
    } else {
        None
    };

    // 2. Extract content rating
    let rating = if combined.contains("rating:explicit") || combined.contains("explicit rating") {
        Some(spindle_core::models::ContentRating::Explicit)
    } else if combined.contains("rating:mature") || combined.contains("mature rating") {
        Some(spindle_core::models::ContentRating::Mature)
    } else if combined.contains("rating:teen") || combined.contains("teen rating") {
        Some(spindle_core::models::ContentRating::Teen)
    } else if combined.contains("rating:general") || combined.contains("general rating") {
        Some(spindle_core::models::ContentRating::General)
    } else {
        if combined.contains("rating: explicit") {
            Some(spindle_core::models::ContentRating::Explicit)
        } else if combined.contains("rating: mature") {
            Some(spindle_core::models::ContentRating::Mature)
        } else if combined.contains("rating: teen") {
            Some(spindle_core::models::ContentRating::Teen)
        } else if combined.contains("rating: general") {
            Some(spindle_core::models::ContentRating::General)
        } else {
            None
        }
    };

    (location_id, rating)
}

pub(super) fn planned_scene_location_and_rating(
    location_id: Option<&str>,
    content_rating: Option<&spindle_core::models::ContentRating>,
    summary: &str,
    purpose: &str,
) -> (Option<String>, Option<spindle_core::models::ContentRating>) {
    let (legacy_location_id, legacy_rating) = extract_scene_location_and_rating(summary, purpose);
    let location_id = location_id
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .or(legacy_location_id);
    let rating = content_rating.cloned().or(legacy_rating);
    (location_id, rating)
}

/// Resolve a plan scene's seed location + rating for `authoring_start_run`.
///
/// `prepare` normally blocks incomplete plans first; this is the
/// defense-in-depth guard so a plan that drifts between prepare and start
/// fails with a chapter/scene-addressed error instead of panicking the
/// handler. Never defaults: silent defaults corrupt fiction canon.
pub(super) fn require_planned_scene_location_and_rating(
    chapter_number: i32,
    scene_order: i32,
    location_id: Option<&str>,
    content_rating: Option<&ContentRating>,
    summary: &str,
    purpose: &str,
) -> anyhow::Result<(String, ContentRating)> {
    let (loc_id, rating) =
        planned_scene_location_and_rating(location_id, content_rating, summary, purpose);
    let loc_id = loc_id.with_context(|| {
        format!(
            "Chapter {chapter_number} scene {scene_order}: missing location ID; \
             set location_id on the plan scene before starting the run"
        )
    })?;
    let rating = rating.with_context(|| {
        format!(
            "Chapter {chapter_number} scene {scene_order}: missing content rating; \
             set content_rating on the plan scene before starting the run"
        )
    })?;
    Ok((loc_id, rating))
}

pub(super) fn authoring_execute_uses_agent_drafting(mode: Option<AuthoringMode>) -> bool {
    mode.is_some_and(|mode| mode.uses_agent_drafting())
}

pub(super) fn find_harness_scene(
    state: &spindle_harness::state::HarnessState,
    chapter_number: i32,
    scene_order: i32,
) -> Option<&spindle_harness::state::SceneState> {
    state
        .chapters
        .iter()
        .find(|chapter| chapter.chapter_number == chapter_number)?
        .scenes
        .iter()
        .find(|scene| scene.scene_order == scene_order)
}

pub(super) fn authoring_commit_output_has_errors(output: &CommitSceneChangesOutput) -> bool {
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

pub(super) fn authoring_commit_error_summary(output: &CommitSceneChangesOutput) -> String {
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

pub(super) fn authoring_save_scene_input(
    input: &AuthoringSaveSceneDraftInput,
) -> SaveSceneDraftInput {
    SaveSceneDraftInput {
        project_id: input.project_id.clone(),
        book_number: input.book_number,
        chapter_number: input.chapter_number,
        chapter_id: input.chapter_id.clone(),
        scene_order: input.scene_order,
        full_text: input.full_text.clone(),
        authorship: input.authorship,
        summary: input.summary.clone(),
        content_rating: input.content_rating.clone(),
        tone: input.tone.clone(),
        generation_id: input.generation_id.clone(),
        source_path: input.source_path.clone(),
        location_id: input.location_id.clone(),
        research_source_ids: input.research_source_ids.clone(),
        research_note_ids: input.research_note_ids.clone(),
        research_claim_ids: input.research_claim_ids.clone(),
        research_query_pack_input: input.research_query_pack_input.clone(),
        research_context_hash: input.research_context_hash.clone(),
        knowledge_learned: input.knowledge_learned.clone(),
    }
}

pub(super) fn authoring_structured_update_count(input: &AuthoringSaveSceneDraftInput) -> usize {
    input.character_states.len()
        + input.canonical_facts.len()
        + input.relationship_updates.len()
        + input.beats.len()
        + input.continuity_notes.len()
        + input.knowledge_learned.len()
}

impl ToolRouter {
    pub(super) async fn handle_authoring_execute_next(
        &self,
        input: AuthoringExecuteNextInput,
    ) -> anyhow::Result<AuthoringExecuteNextOutput> {
        self.handle_authoring_execute_next_with_seam(input, || async {})
            .await
    }

    pub(crate) async fn handle_authoring_execute_next_with_seam<H, Fut>(
        &self,
        input: AuthoringExecuteNextInput,
        mut before_handoff_persist: H,
    ) -> anyhow::Result<AuthoringExecuteNextOutput>
    where
        H: FnMut() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let project_id = input.project_id.clone();
        let agent_mode = authoring_execute_uses_agent_drafting(input.mode);
        let repo = self.service.repository();

        let run_id = match input.run_id {
            Some(rid) if !rid.is_empty() => rid,
            _ => repo
                .find_latest_authoring_run_id(&project_id)
                .await?
                .context("No active or latest authoring run found for project")?,
        };

        let data_dir = repo.data_dir();
        let mut handoff_seam_fired = false;
        let mut attempts = 0usize;

        let (run, dispatch_state, action_to_execute) = loop {
            attempts += 1;
            if attempts > 4 {
                anyhow::bail!(
                    "authoring_execute_next failed to persist a consistent run/plan handoff snapshot after {} attempts due to concurrent plan or run updates",
                    attempts - 1
                );
            }

            let (run, chapters, scenes, checkpoints, run_updated_at_micros, plan_revisions) = repo
                .get_authoring_run_with_plan_revisions(&run_id)
                .await?
                .with_context(|| format!("Authoring run {} not found", run_id))?;

            if run.status == "completed" {
                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: "complete".to_string(),
                    executed_action: "none".to_string(),
                    message: "Authoring run is already completed.".to_string(),
                    status: run.status,
                });
            }
            if run.status == "paused" {
                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: "paused".to_string(),
                    executed_action: "none".to_string(),
                    message:
                        "Authoring run is paused; start or select another run before executing."
                            .to_string(),
                    status: run.status,
                });
            }

            let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);

            let snapshot = self.authoring_project_snapshot(&harness_state).await?;
            let outcome = reconcile_state(harness_state.clone(), &snapshot);

            if outcome.has_errors() {
                let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
                    &run_id,
                    &outcome.state,
                    "blocked",
                    Some(run.created_at),
                );
                repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                    .await?;
                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: outcome.next_action.to_string(),
                    executed_action: "none".to_string(),
                    message: "Execution blocked by errors.".to_string(),
                    status: "blocked".to_string(),
                });
            }

            // Auto-checkpoint interception (evolution §3.3, K3). A run whose
            // checkpoint_policy is auto_advisory/auto_strict does NOT surface
            // `AwaitCheckpointReview` — instead the harness performs the checkpoint
            // work in-process (deep consistency + sampled dual-persona reviews) and
            // self-clears on a severity threshold. `manual` (None) is untouched and
            // byte-identical to today (I1). Runs before the block match so an
            // approval lets the loop continue in the same call; a block falls
            // through to the existing AwaitCheckpointReview path.
            if let NextAction::AwaitCheckpointReview {
                start_chapter,
                end_chapter,
                ..
            } = &outcome.next_action
                && let Some(policy) = auto_checkpoint_policy(run.checkpoint_policy.as_deref())
            {
                let start_chapter = *start_chapter;
                let end_chapter = *end_chapter;
                let mut auto_state = outcome.state.clone();
                let auto = Box::pin(self.run_auto_checkpoint(
                    &run_id,
                    policy,
                    &mut auto_state,
                    start_chapter,
                    end_chapter,
                ))
                .await?;

                if auto.approved {
                    // The checkpoint self-cleared. Re-reconcile the post-review state
                    // and persist; the run continues (or completes) exactly as a
                    // manual review would leave it. Journal the auto-approval.
                    let auto_snapshot = self.authoring_project_snapshot(&auto_state).await?;
                    let auto_outcome = reconcile_state(auto_state.clone(), &auto_snapshot);
                    let auto_status = if auto_outcome.next_action == NextAction::Complete {
                        "completed"
                    } else if matches!(
                        auto_outcome.next_action,
                        NextAction::AwaitCheckpointReview { .. } | NextAction::AwaitResearch { .. }
                    ) {
                        "blocked"
                    } else {
                        "active"
                    };
                    let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
                        &run_id,
                        &auto_outcome.state,
                        auto_status,
                        Some(run.created_at),
                    );
                    repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                        .await?;

                    let journal = RunJournal::new(repo);
                    journal
                        .emit(
                            &run_id,
                            "checkpoint_auto_approved",
                            run_journal::checkpoint_auto_approved_payload(
                                start_chapter,
                                end_chapter,
                                policy,
                                &auto.finding_counts,
                            ),
                        )
                        .await;
                    if run.status == "blocked" || run.status == "paused" {
                        journal
                            .emit(
                                &run_id,
                                "run_resumed",
                                run_journal::run_status_payload(None),
                            )
                            .await;
                    }
                    if auto_status == "completed" {
                        journal
                            .emit(
                                &run_id,
                                "run_completed",
                                run_journal::run_status_payload(None),
                            )
                            .await;
                    }

                    return Ok(AuthoringExecuteNextOutput {
                        run_id: run_id.clone(),
                        next_action: auto_outcome.next_action.to_string(),
                        executed_action: format!(
                            "auto-checkpoint approved chapters {start_chapter}-{end_chapter} under {policy}"
                        ),
                        message: format!(
                            "Checkpoint {start_chapter}-{end_chapter} auto-approved under {policy} (0 blocking findings)."
                        ),
                        status: auto_status.to_string(),
                    });
                }

                // Not approved: the checkpoint stays pending_review exactly as
                // manual, with the auto_outcome + pending-manual scenes stamped on
                // it. Persist and journal `checkpoint_blocked`, then return blocked
                // with the prose-free reason. The manual escape hatch still works —
                // an operator can still call authoring_review_checkpoint.
                let reason = auto
                    .blocked_reason
                    .clone()
                    .unwrap_or_else(|| "auto_checkpoint_blocked".to_string());
                let (updated_run, updated_ch, updated_sc, updated_cp) =
                    map_harness_to_records(&run_id, &auto_state, "blocked", Some(run.created_at));
                repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                    .await?;

                let journal = RunJournal::new(repo);
                journal
                    .emit(
                        &run_id,
                        "checkpoint_blocked",
                        run_journal::checkpoint_blocked_payload(
                            start_chapter,
                            end_chapter,
                            &reason,
                        ),
                    )
                    .await;
                if run.status != "blocked" {
                    journal
                        .emit(
                            &run_id,
                            "run_blocked",
                            run_journal::run_status_payload(Some("blocked")),
                        )
                        .await;
                }

                let manual_note = if auto.pending_manual_scene_ids.is_empty() {
                    String::new()
                } else {
                    format!(
                        " Scenes awaiting manual dual-persona review: [{}].",
                        auto.pending_manual_scene_ids.join(", ")
                    )
                };
                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: outcome.next_action.to_string(),
                    executed_action: format!(
                        "auto-checkpoint blocked chapters {start_chapter}-{end_chapter} under {policy}"
                    ),
                    message: format!(
                        "Checkpoint {start_chapter}-{end_chapter} blocked under {policy}: {reason}.{manual_note} \
                         Review it manually with authoring_review_checkpoint."
                    ),
                    status: "blocked".to_string(),
                });
            }

            match &outcome.next_action {
                NextAction::Blocked
                | NextAction::AwaitCheckpointReview { .. }
                | NextAction::AwaitResearch { .. }
                | NextAction::Complete => {
                    let status_str = match &outcome.next_action {
                        NextAction::Complete => "completed",
                        NextAction::Blocked => "blocked",
                        NextAction::AwaitCheckpointReview { .. } => "blocked",
                        NextAction::AwaitResearch { .. } => "blocked",
                        _ => &run.status,
                    };
                    if status_str != run.status {
                        let (updated_run, updated_ch, updated_sc, updated_cp) =
                            map_harness_to_records(
                                &run_id,
                                &outcome.state,
                                status_str,
                                Some(run.created_at),
                            );
                        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
                            .await?;

                        // Journal (ADR D2): a genuine run-status transition persisted
                        // WITHOUT executing a step. Diff-derived from prev vs new
                        // status; emitted AFTER the save commits (D3.3). An
                        // AwaitCheckpointReview block is a `checkpoint_blocked`;
                        // Complete is `run_completed`; any other block is
                        // `run_blocked`.
                        let journal = RunJournal::new(repo);
                        match &outcome.next_action {
                            NextAction::AwaitCheckpointReview {
                                start_chapter,
                                end_chapter,
                                ..
                            } => {
                                journal
                                    .emit(
                                        &run_id,
                                        "checkpoint_blocked",
                                        run_journal::checkpoint_blocked_payload(
                                            *start_chapter,
                                            *end_chapter,
                                            "await_checkpoint_review",
                                        ),
                                    )
                                    .await;
                            }
                            NextAction::Complete => {
                                journal
                                    .emit(
                                        &run_id,
                                        "run_completed",
                                        run_journal::run_status_payload(None),
                                    )
                                    .await;
                            }
                            _ => {
                                let reason = match &outcome.next_action {
                                    NextAction::AwaitResearch { .. } => "await_research",
                                    _ => "blocked",
                                };
                                journal
                                    .emit(
                                        &run_id,
                                        "run_blocked",
                                        run_journal::run_status_payload(Some(reason)),
                                    )
                                    .await;
                            }
                        }
                    }
                    return Ok(AuthoringExecuteNextOutput {
                        run_id: run_id.clone(),
                        next_action: outcome.next_action.to_string(),
                        executed_action: "none".to_string(),
                        message: format!(
                            "No action executed (action state is {:?})",
                            outcome.next_action
                        ),
                        status: status_str.to_string(),
                    });
                }
                _ => {}
            }

            let action_to_execute = outcome.next_action.clone();
            if let NextAction::DraftScene {
                chapter_number,
                scene_order,
            } = &action_to_execute
                && !agent_mode
                && let Some(scene) =
                    find_harness_scene(&outcome.state, *chapter_number, *scene_order)
                && scene.content_rating != spindle_core::models::ContentRating::Explicit
            {
                let scene_rating = scene.content_rating.as_str().to_string();
                let current_plan_revision =
                    plan_revisions.get(chapter_number).copied().unwrap_or(0);
                let mut handoff_state = outcome.state.clone();
                if let Some(ch_state) = handoff_state
                    .chapters
                    .iter_mut()
                    .find(|ch| ch.chapter_number == *chapter_number)
                {
                    for sc_state in &mut ch_state.scenes {
                        if let Some(ref mut diag) = sc_state.draft_diagnostics {
                            diag.superseded_handoff_plan_revision = None;
                        }
                        if sc_state.scene_order == *scene_order {
                            let diag = sc_state
                                .draft_diagnostics
                                .get_or_insert_with(Default::default);
                            diag.handoff_plan_revision = Some(current_plan_revision);
                            diag.superseded_handoff_plan_revision = None;
                        }
                    }
                }
                let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
                    &run_id,
                    &handoff_state,
                    &run.status,
                    Some(run.created_at),
                );
                if !handoff_seam_fired {
                    handoff_seam_fired = true;
                    before_handoff_persist().await;
                }
                if !repo
                    .try_save_authoring_run_handoff(
                        updated_run,
                        updated_ch,
                        updated_sc,
                        updated_cp,
                        run_updated_at_micros,
                        plan_revisions,
                    )
                    .await?
                {
                    continue;
                }
                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: action_to_execute.to_string(),
                    executed_action: "none".to_string(),
                    message: format!(
                        "Host draft required for non-explicit scene {chapter_number}.{scene_order}. \
                         Draft this scene in the active assistant chat using Spindle context, then call authoring_save_scene_draft \
                         with project_id={}, run_id={}, book_number={}, chapter_number={}, \
                         scene_order={}, content_rating={}, full_text, summary, character_states, canonical_facts, \
                         relationship_updates, beats, and continuity_notes. This structured continuity package is required; \
                         use continuity_notes to explicitly say when the scene introduces no durable canon changes. \
                         Then call authoring_execute_next again. Use mode='agent' only when you explicitly \
                         want Spindle to offload non-explicit drafting to the configured draft route.",
                        project_id,
                        run_id,
                        outcome.state.book_number,
                        chapter_number,
                        scene_order,
                        scene_rating
                    ),
                    status: run.status.clone(),
                });
            }

            // Hybrid-mode revision hand-off (evolution §3.2, I2). VerifyScene runs
            // host-independently (it falls through to the executor below), but a
            // ReviseScene in hybrid mode means the HOST is the revision agent: list
            // the scene-scoped findings and instruct a revise + re-save. The re-save
            // through authoring_save_scene_draft resets verify_status and counts the
            // attempt, so the loop re-verifies exactly as the agent path does. In
            // agent mode this arm is skipped and ReviseScene re-drafts automatically.
            if let NextAction::ReviseScene {
                chapter_number,
                scene_order,
                attempt,
            } = &action_to_execute
                && !agent_mode
            {
                let current_plan_revision =
                    plan_revisions.get(chapter_number).copied().unwrap_or(0);
                let mut handoff_state = outcome.state.clone();
                if let Some(ch_state) = handoff_state
                    .chapters
                    .iter_mut()
                    .find(|ch| ch.chapter_number == *chapter_number)
                {
                    for sc_state in &mut ch_state.scenes {
                        if let Some(ref mut diag) = sc_state.draft_diagnostics {
                            diag.superseded_handoff_plan_revision = None;
                        }
                        if sc_state.scene_order == *scene_order {
                            let diag = sc_state
                                .draft_diagnostics
                                .get_or_insert_with(Default::default);
                            diag.handoff_plan_revision = Some(current_plan_revision);
                            diag.superseded_handoff_plan_revision = None;
                        }
                    }
                }
                let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
                    &run_id,
                    &handoff_state,
                    &run.status,
                    Some(run.created_at),
                );
                if !handoff_seam_fired {
                    handoff_seam_fired = true;
                    before_handoff_persist().await;
                }
                if !repo
                    .try_save_authoring_run_handoff(
                        updated_run,
                        updated_ch,
                        updated_sc,
                        updated_cp,
                        run_updated_at_micros,
                        plan_revisions,
                    )
                    .await?
                {
                    continue;
                }
                let findings = self
                    .authoring_scene_verify_findings(
                        data_dir,
                        &project_id,
                        outcome.state.book_number,
                        *chapter_number,
                        *scene_order,
                    )
                    .await
                    .unwrap_or_default();
                let findings_block = if findings.is_empty() {
                    "(no findings resolved at read time)".to_string()
                } else {
                    findings
                        .iter()
                        .take(12)
                        .map(|(check_type, message)| format!("- [{check_type}] {message}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                return Ok(AuthoringExecuteNextOutput {
                    run_id: run_id.clone(),
                    next_action: action_to_execute.to_string(),
                    executed_action: "none".to_string(),
                    message: format!(
                        "Host revision required for scene {chapter_number}.{scene_order} \
                         (revision attempt {attempt}). The saved draft tripped these scene-scoped \
                         checks:\n{findings_block}\n\nRevise the scene to resolve each finding, then \
                         re-save it with authoring_save_scene_draft (project_id={project_id}, \
                         run_id={run_id}, book_number={}, chapter_number={chapter_number}, \
                         scene_order={scene_order}) and call authoring_execute_next again. The re-save \
                         re-verifies the revised draft; if the same findings persist the scene is \
                         parked at the checkpoint rather than revised again.",
                        outcome.state.book_number,
                    ),
                    status: run.status.clone(),
                });
            }

            let mut dispatch_state = outcome.state;
            if let NextAction::DraftScene { chapter_number, .. }
            | NextAction::ReviseScene { chapter_number, .. } = &action_to_execute
            {
                let mut cleared = false;
                if let Some(ch_state) = dispatch_state
                    .chapters
                    .iter_mut()
                    .find(|ch| ch.chapter_number == *chapter_number)
                {
                    for sc_state in &mut ch_state.scenes {
                        if let Some(ref mut diag) = sc_state.draft_diagnostics
                            && (diag.handoff_plan_revision.is_some()
                                || diag.superseded_handoff_plan_revision.is_some())
                        {
                            diag.handoff_plan_revision = None;
                            diag.superseded_handoff_plan_revision = None;
                            cleared = true;
                        }
                    }
                }
                if cleared {
                    let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
                        &run_id,
                        &dispatch_state,
                        &run.status,
                        Some(run.created_at),
                    );
                    if !repo
                        .try_save_authoring_run_handoff(
                            updated_run,
                            updated_ch,
                            updated_sc,
                            updated_cp,
                            run_updated_at_micros,
                            plan_revisions,
                        )
                        .await?
                    {
                        continue;
                    }
                }
            }

            break (run, dispatch_state, action_to_execute);
        };

        let state_path = authoring_state_path(data_dir, &run_id);
        dispatch_state.save(&state_path)?;

        let exec_result = match &action_to_execute {
            NextAction::RunCheckpoint {
                start_chapter,
                end_chapter,
            } => {
                self.execute_authoring_run_checkpoint(
                    &state_path,
                    dispatch_state,
                    *start_chapter,
                    *end_chapter,
                )
                .await
            }
            NextAction::CommitSceneChanges {
                chapter_number,
                scene_order,
                ..
            } => {
                self.execute_authoring_commit_scene_changes(
                    &state_path,
                    dispatch_state,
                    *chapter_number,
                    *scene_order,
                )
                .await
            }
            NextAction::AnnotateSceneBeats {
                chapter_number,
                scene_order,
                ..
            } => {
                self.execute_authoring_annotate_scene_beats(
                    &state_path,
                    dispatch_state,
                    *chapter_number,
                    *scene_order,
                )
                .await
            }
            NextAction::SaveChapterSummary { chapter_number } => {
                self.execute_authoring_save_chapter_summary(
                    &state_path,
                    dispatch_state,
                    *chapter_number,
                    &run_id,
                )
                .await
            }
            _ => {
                // Lazy primacy claim (bug 4a): if no live primary owns the addr
                // file yet, claim it here (bind the internal listener, write the
                // addr file) and proceed. Idempotent + race-safe.
                let active_addr =
                    crate::internal_listener::ensure_primary_addr(&self.service, data_dir).await?;
                let internal_token_guard = self
                    .serialization_state
                    .issue_internal_token(ToolSerializationScope::Project(project_id.clone()));
                let url = format!(
                    "http://{}/mcp?internal_token={}",
                    active_addr,
                    internal_token_guard.token()
                );
                let client = McpHarnessClient::connect(&TransportConfig::Http { url }).await?;
                execute_one(
                    &state_path,
                    dispatch_state,
                    &client,
                    action_to_execute.clone(),
                )
                .await
            }
        };

        let _ = std::fs::remove_file(&state_path);

        let exec_res = exec_result?;

        let updated_snapshot = self.authoring_project_snapshot(&exec_res.state).await?;
        let updated_outcome = reconcile_state(exec_res.state.clone(), &updated_snapshot);

        let mut final_status = "active".to_string();
        if updated_outcome.has_errors() {
            final_status = "blocked".to_string();
        } else {
            for ch in &exec_res.state.chapters {
                for sc in &ch.scenes {
                    if sc.blocked_reason.is_some() {
                        final_status = "blocked".to_string();
                    }
                }
            }
        }
        if updated_outcome.next_action == NextAction::Complete {
            final_status = "completed".to_string();
        }
        if matches!(
            updated_outcome.next_action,
            NextAction::AwaitCheckpointReview { .. } | NextAction::AwaitResearch { .. }
        ) {
            final_status = "blocked".to_string();
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) = map_harness_to_records(
            &run_id,
            &exec_res.state,
            &final_status,
            Some(run.created_at),
        );
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2): step events derived from the executed action + the
        // committed after-state, plus any run-status transition. Emitted AFTER
        // the save commits (D3.3); a journal error never fails the step.
        authoring_emit_step_events(
            repo,
            &run_id,
            &action_to_execute,
            &exec_res.state,
            &run.status,
            &final_status,
        )
        .await;

        Ok(AuthoringExecuteNextOutput {
            run_id: run_id.clone(),
            next_action: updated_outcome.next_action.to_string(),
            executed_action: action_to_execute.to_string(),
            message: exec_res.message,
            status: final_status,
        })
    }

    pub(super) async fn handle_authoring_save_scene_draft(
        &self,
        input: AuthoringSaveSceneDraftInput,
    ) -> anyhow::Result<AuthoringSaveSceneDraftOutput> {
        self.handle_authoring_save_scene_draft_with_seam(input, || async {})
            .await
    }

    pub(crate) async fn handle_authoring_save_scene_draft_with_seam<H, Fut>(
        &self,
        mut input: AuthoringSaveSceneDraftInput,
        before_scene_write: H,
    ) -> anyhow::Result<AuthoringSaveSceneDraftOutput>
    where
        H: FnOnce() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let structured_update_count = authoring_structured_update_count(&input);
        if structured_update_count == 0 {
            anyhow::bail!(
                "authoring_save_scene_draft requires a structured continuity package: \
                 provide character_states, canonical_facts, relationship_updates, beats, \
                 or continuity_notes. If the scene introduces no durable canon changes, \
                 add a continuity_notes entry saying that explicitly."
            );
        }

        let project_id = input.project_id.clone();
        let repo = self.service.repository();
        let run_id = match input.run_id.clone() {
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
        if run.status == "completed" || run.status == "paused" {
            anyhow::bail!(
                "authoring run {} is {}; cannot save a scene draft",
                run_id,
                run.status
            );
        }

        let harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);
        let (chapter_index, scene_index) =
            authoring_scene_indices(&harness_state, input.chapter_number, input.scene_order)
                .with_context(|| {
                    format!(
                        "scene {}.{} not found in authoring run {}",
                        input.chapter_number, input.scene_order, run_id
                    )
                })?;

        let expected_rating = harness_state.chapters[chapter_index].scenes[scene_index]
            .content_rating
            .clone();
        if expected_rating != input.content_rating {
            anyhow::bail!(
                "scene {}.{} is planned as {}, but draft was saved as {}",
                input.chapter_number,
                input.scene_order,
                expected_rating.as_str(),
                input.content_rating.as_str()
            );
        }

        if input.full_text.trim() == "_keep_existing_" {
            let existing_scene_id = harness_state.chapters[chapter_index].scenes[scene_index]
                .scene_id
                .clone()
                .with_context(|| {
                    format!(
                        "scene {}.{} has no saved scene text to keep; provide full_text",
                        input.chapter_number, input.scene_order
                    )
                })?;
            let existing_scene = repo.get_scene(&existing_scene_id).await?;
            if existing_scene.project_id != project_id {
                anyhow::bail!(
                    "scene {} does not belong to project {}; cannot keep existing text",
                    existing_scene_id,
                    project_id
                );
            }
            input.full_text = existing_scene.full_text;
        }

        repo.check_active_run_scene_handoff_plan_revision(
            &project_id,
            &harness_state.active_branch_id,
            input.book_number,
            input.chapter_number,
            input.scene_order,
        )
        .await?;

        let save_output = self
            .service
            .save_scene_draft_with_seam(authoring_save_scene_input(&input), before_scene_write)
            .await?;

        let (run, chapters, scenes, checkpoints) = repo
            .get_authoring_run(&run_id)
            .await?
            .with_context(|| format!("Authoring run {} not found", run_id))?;
        let mut harness_state = map_records_to_harness(&run, &chapters, &scenes, &checkpoints);
        let (chapter_index, scene_index) =
            authoring_scene_indices(&harness_state, input.chapter_number, input.scene_order)
                .with_context(|| {
                    format!(
                        "scene {}.{} not found in authoring run {}",
                        input.chapter_number, input.scene_order, run_id
                    )
                })?;

        let state_path = authoring_state_path(repo.data_dir(), &run_id);
        let artifact_store =
            ArtifactStore::new(authoring_artifacts_root(&state_path, &harness_state));
        let artifact_rel = harness_state.chapters[chapter_index].scenes[scene_index]
            .scene_artifact_path
            .clone()
            .unwrap_or_else(|| {
                ArtifactStore::scene_relative_path(input.chapter_number, input.scene_order)
            });

        let mut artifact = artifact_store
            .load_json::<SceneGenerationArtifact>(&artifact_rel)
            .unwrap_or_else(|_| {
                SceneGenerationArtifact::new(
                    input.chapter_number,
                    input.scene_order,
                    "host".to_string(),
                    "interactive-host".to_string(),
                    Some(input.content_rating.as_str().to_string()),
                    "Host-drafted scene saved through authoring_save_scene_draft.".to_string(),
                )
            });
        artifact.rating = Some(input.content_rating.as_str().to_string());
        artifact.completion_fragments = vec![input.full_text.clone()];
        artifact.adapter_kind = Some("host".to_string());
        artifact.model_name = Some("interactive-host".to_string());
        artifact.truncated = false;
        artifact.last_parse_error = None;
        artifact.package = Some(GeneratedScenePackage {
            full_text: input.full_text.clone(),
            summary: input.summary.clone(),
            tone: input.tone.clone(),
            character_states: input.character_states.clone(),
            canonical_facts: input.canonical_facts.clone(),
            relationship_updates: input.relationship_updates.clone(),
            beats: input.beats.clone(),
            continuity_notes: input.continuity_notes.clone(),
        });
        artifact.save_draft_output = Some(save_output.clone());
        artifact.commit_output = None;
        artifact.beat_annotation_output = None;
        artifact.research_source_ids = input.research_source_ids.clone();
        artifact.research_note_ids = input.research_note_ids.clone();
        artifact.research_claim_ids = input.research_claim_ids.clone();
        artifact.research_query_pack_input = input.research_query_pack_input.clone();
        artifact.research_context_hash = input.research_context_hash.clone();
        artifact_store.save_json(&artifact_rel, &artifact)?;

        let live_scene = &mut harness_state.chapters[chapter_index].scenes[scene_index];
        // In-run verify/revise (evolution §3.2, hybrid mode): a re-save that
        // follows a `findings` verify IS the host's revision. Reset verify_status
        // so the scheduler re-verifies the revised draft, and count the attempt
        // so the bounded budget is honored. Any other prior verify state (clean,
        // parked, error, or none) leaves the counters untouched — a first save,
        // or a save after the loop already converged, must not spend budget.
        let host_revision_attempt = if live_scene.verify_status.as_deref() == Some("findings") {
            live_scene.revise_attempts += 1;
            live_scene.verify_status = None;
            live_scene.verify_detail = None;
            Some(live_scene.revise_attempts)
        } else {
            None
        };
        live_scene.phase = ScenePhase::DraftSaved;
        live_scene.scene_id = Some(save_output.scene_id.clone());
        live_scene.scene_artifact_path = Some(artifact_rel.clone());
        live_scene.tone = input.tone.clone();
        live_scene.source_path = input.source_path.clone();
        live_scene.blocked_reason = None;
        if let Some(ref mut diag) = live_scene.draft_diagnostics {
            diag.handoff_plan_revision = None;
            diag.superseded_handoff_plan_revision = None;
        }

        // Summary invalidation (defect item 3): re-saving a scene in a chapter
        // whose summary was already persisted (the documented revise-during-
        // checkpoint flow) leaves that summary describing prose that no longer
        // exists. Delete the row and the summary artifact and clear the harness
        // flags so the pipeline regenerates both after the scene recommits —
        // otherwise the next execute blocks on the summary residue rule.
        let chapter_state = &mut harness_state.chapters[chapter_index];
        let persisted_summary = repo
            .get_chapter_summary(
                &project_id,
                &harness_state.active_branch_id,
                input.book_number,
                input.chapter_number,
            )
            .await?;
        if chapter_state.summary_saved || persisted_summary.is_some() {
            repo.delete_chapter_summaries_for_chapter(
                &project_id,
                &harness_state.active_branch_id,
                input.book_number,
                input.chapter_number,
            )
            .await?;
            let summary_rel = chapter_state
                .summary_artifact_path
                .clone()
                .unwrap_or_else(|| ArtifactStore::summary_relative_path(input.chapter_number));
            let summary_path = artifact_store.root().join(&summary_rel);
            match std::fs::remove_file(&summary_path) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            "failed to remove stale summary artifact at {}",
                            summary_path.display()
                        )
                    });
                }
            }
            chapter_state.summary_saved = false;
            chapter_state.summary_artifact_path = None;
        }

        let (updated_run, updated_ch, updated_sc, updated_cp) =
            map_harness_to_records(&run_id, &harness_state, &run.status, Some(run.created_at));
        repo.save_authoring_run(updated_run, updated_ch, updated_sc, updated_cp)
            .await?;

        // Journal (ADR D2, emitted AFTER the save commits — D3.3). The host path
        // always persists a draft; a re-save after a `findings` verify is ALSO
        // the host's revision, so emit `scene_revised` in that case. Ordering:
        // draft first, then revised (the revision is a property of this save).
        let journal = RunJournal::new(repo);
        let anti_slop = save_output
            .anti_slop
            .as_ref()
            .map(spindle_core::style::antislop::journal_summary);
        journal
            .emit(
                &run_id,
                "scene_drafted",
                run_journal::scene_drafted_payload(
                    input.chapter_number,
                    input.scene_order,
                    &save_output.scene_id,
                    "host",
                    anti_slop.as_ref(),
                ),
            )
            .await;
        if let Some(attempt) = host_revision_attempt {
            journal
                .emit(
                    &run_id,
                    "scene_revised",
                    run_journal::scene_revised_payload(
                        input.chapter_number,
                        input.scene_order,
                        &save_output.scene_id,
                        attempt,
                        // The host authored the revision off the listed findings;
                        // the directive count is not re-plumbed here (0 = "count
                        // not recorded on the host path" — additive, prose-free).
                        0,
                    ),
                )
                .await;
        }

        Ok(AuthoringSaveSceneDraftOutput {
            run_id,
            scene_id: save_output.scene_id.clone(),
            scene_artifact_path: artifact_rel,
            status: save_output.status.clone(),
            structured_update_count,
            save_output,
        })
    }

    /// Run the scene-scoped deterministic check subset for one scene and return
    /// its actionable (severity ≥ warning) findings as `(check_type, message)`
    /// pairs, sorted and deduplicated (evolution §3.2). Used by the hybrid-mode
    /// revision hand-off to list findings for the host. Connects to the running
    /// HTTP MCP server (same transport the executor uses); the check is
    /// deterministic and issues zero model calls (`deep_check = false`).
    pub(super) async fn authoring_scene_verify_findings(
        &self,
        data_dir: &Path,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
        scene_order: i32,
    ) -> anyhow::Result<Vec<(String, String)>> {
        // Lazy primacy claim (bug 4a) — see the sibling call in
        // authoring_execute_next's dispatch arm.
        let active_addr =
            crate::internal_listener::ensure_primary_addr(&self.service, data_dir).await?;
        let internal_token_guard = self
            .serialization_state
            .issue_internal_token(ToolSerializationScope::Project(project_id.to_string()));
        let url = format!(
            "http://{}/mcp?internal_token={}",
            active_addr,
            internal_token_guard.token()
        );
        let client = McpHarnessClient::connect(&TransportConfig::Http { url }).await?;
        let scope = spindle_core::models::ConsistencyScopeInput {
            scene_order: Some(scene_order),
            ..spindle_core::models::ConsistencyScopeInput::chapter_range(
                book_number,
                chapter_number,
                book_number,
                chapter_number,
            )
        };
        let output = client
            .check_consistency(&spindle_core::models::CheckConsistencyInput {
                deep_scan_offset: None,
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
            .filter(|issue| matches!(issue.severity.as_str(), "warning" | "error"))
            .map(|issue| (issue.check_type, issue.message))
            .collect();
        findings.sort();
        findings.dedup();
        Ok(findings)
    }

    pub(super) async fn execute_authoring_commit_scene_changes(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        chapter_number: i32,
        scene_order: i32,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let (chapter_index, scene_index) =
            authoring_scene_indices(&state, chapter_number, scene_order).with_context(|| {
                format!("scene {chapter_number}.{scene_order} not found in authoring run state")
            })?;
        let scene = state.chapters[chapter_index].scenes[scene_index].clone();
        let scene_id = scene
            .scene_id
            .clone()
            .with_context(|| format!("scene_id missing for {chapter_number}.{scene_order}"))?;
        let artifact_path = scene.scene_artifact_path.clone();
        let mut artifact = artifact_path
            .as_ref()
            .map(|path| artifact_store.load_json::<SceneGenerationArtifact>(path))
            .transpose()?;
        let mut commit_output = artifact
            .as_ref()
            .and_then(|artifact| artifact.commit_output.clone());

        if commit_output.is_none() {
            let package = match artifact
                .as_ref()
                .and_then(|artifact| artifact.package.as_ref())
            {
                Some(package) => package,
                None => {
                    let live_scene = &mut state.chapters[chapter_index].scenes[scene_index];
                    live_scene.blocked_reason = Some(
                        "missing structured continuity package; save host-authored scenes with authoring_save_scene_draft before committing".to_string(),
                    );
                    state.save(state_path)?;
                    anyhow::bail!(
                        "authoring scene {}.{} is missing its structured continuity package; \
                         call authoring_save_scene_draft with character_states, canonical_facts, \
                         relationship_updates, beats, and/or continuity_notes before resuming",
                        chapter_number,
                        scene_order
                    );
                }
            };
            let (character_states, canonical_facts, relationship_updates) = (
                package.character_states.clone(),
                package.canonical_facts.clone(),
                package.relationship_updates.clone(),
            );
            let new_commit_output = self
                .service
                .commit_scene_changes(CommitSceneChangesInput {
                    project_id: state.project_id.clone(),
                    scene_id: scene_id.clone(),
                    character_states,
                    canonical_facts,
                    relationship_updates,
                    accept_world_rule_risks: true,
                    // Autonomous supervisor commit: surface continuity findings on
                    // the output without halting the run.
                    accept_continuity_risks: false,
                    continuity_gate: Some(spindle_core::models::CommitContinuityGate::WarnOnly),
                })
                .await
                .with_context(|| {
                    format!(
                        "failed to commit scene changes for chapter {chapter_number} scene {scene_order}"
                    )
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
        if authoring_commit_output_has_errors(commit_output) {
            let inspect_target = artifact_path
                .as_ref()
                .map(|path| artifact_store.root().join(path).display().to_string())
                .unwrap_or_else(|| scene_id.clone());
            let error_summary = authoring_commit_error_summary(commit_output);
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
        Ok(ExecutionResult {
            state,
            message: format!(
                "Committed scene changes for chapter {chapter_number} scene {scene_order}"
            ),
        })
    }

    pub(super) async fn execute_authoring_annotate_scene_beats(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        chapter_number: i32,
        scene_order: i32,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let (chapter_index, scene_index) =
            authoring_scene_indices(&state, chapter_number, scene_order).with_context(|| {
                format!("scene {chapter_number}.{scene_order} not found in authoring run state")
            })?;
        let scene = state.chapters[chapter_index].scenes[scene_index].clone();
        let scene_id = scene
            .scene_id
            .clone()
            .with_context(|| format!("scene_id missing for {chapter_number}.{scene_order}"))?;
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
            let annotation_output = self
                .service
                .annotate_scene_beats(AnnotateSceneBeatsInput {
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
                    format!(
                        "failed to annotate beats for chapter {chapter_number} scene {scene_order}"
                    )
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
        Ok(ExecutionResult {
            state,
            message: format!("Annotated beats for chapter {chapter_number} scene {scene_order}"),
        })
    }

    pub(super) async fn execute_authoring_save_chapter_summary(
        &self,
        state_path: &Path,
        mut state: HarnessState,
        chapter_number: i32,
        run_id: &str,
    ) -> anyhow::Result<ExecutionResult> {
        let artifact_store = ArtifactStore::new(authoring_artifacts_root(state_path, &state));
        let chapter_index = state
            .chapters
            .iter()
            .position(|chapter| chapter.chapter_number == chapter_number)
            .with_context(|| {
                format!("chapter {chapter_number} not found in authoring run state")
            })?;

        if state.chapters[chapter_index]
            .summary_artifact_path
            .is_none()
        {
            state.chapters[chapter_index].summary_artifact_path =
                Some(ArtifactStore::summary_relative_path(chapter_number));
            state.save(state_path)?;
        }

        let artifact_path = state.chapters[chapter_index]
            .summary_artifact_path
            .clone()
            .context("summary artifact path missing after initialization")?;
        let mut artifact = artifact_store
            .load_json::<ChapterSummaryArtifact>(&artifact_path)
            .unwrap_or_else(|_| {
                ChapterSummaryArtifact::new(
                    chapter_number,
                    "in-process".to_string(),
                    "deterministic-summary".to_string(),
                    "Synthesized from saved scene packages by authoring_execute_next.".to_string(),
                )
            });
        if artifact.chapter_number != chapter_number {
            anyhow::bail!(
                "summary artifact is for chapter {}, expected chapter {}",
                artifact.chapter_number,
                chapter_number
            );
        }

        // Stale-artifact guard (defect item 2): an on-disk artifact counts as
        // idempotency proof ONLY if it belongs to this run and the
        // chapter_summary row its save_summary_output references still exists
        // on the run's branch. An artifact from an earlier pass (different or
        // unstamped run_id, row since deleted) would otherwise mark
        // summary_saved without persisting anything AND silently reuse old-pass
        // summary content for new prose. Discard its outputs and regenerate
        // from the current scene packages.
        let foreign_run = artifact.run_id.as_deref().is_some_and(|id| id != run_id);
        let dangling_output = match artifact.save_summary_output.as_ref() {
            Some(output) if !foreign_run => {
                let existing = self
                    .service
                    .repository()
                    .get_chapter_summary(
                        &state.project_id,
                        &state.active_branch_id,
                        state.book_number,
                        chapter_number,
                    )
                    .await?;
                existing.is_none_or(|row| row.id != output.chapter_summary_id)
            }
            _ => false,
        };
        if foreign_run || dangling_output {
            artifact.clear_generation();
            artifact.save_summary_output = None;
            artifact.last_parse_error = None;
        }
        artifact.run_id = Some(run_id.to_string());

        let package = match artifact.package.clone() {
            Some(package) => package,
            None => {
                let package = authoring_build_chapter_summary_package(
                    &artifact_store,
                    &state.chapters[chapter_index],
                )?;
                artifact.package = Some(package.clone());
                artifact.truncated = false;
                artifact.last_parse_error = None;
                artifact_store.save_json(&artifact_path, &artifact)?;
                package
            }
        };

        if artifact.save_summary_output.is_none() {
            let save_output = self
                .service
                .save_summary(SaveSummaryInput {
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
        Ok(ExecutionResult {
            state,
            message: format!("Saved chapter summary for chapter {chapter_number}"),
        })
    }
}
