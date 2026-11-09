//! Editorial controller service methods: hybrid host submission/judging and
//! agent-mode parity over the same packet/revision artifacts.

use super::*;
use crate::drafting::{
    DraftTokenizer, PacketBudget, artifact,
    chapter::{ChapterCriticBudget, save_chapter_critique},
    editorial::{
        apply_host_editorial_submission, editorial_sha256_hex, load_editorial_controller,
        normalized_output_sha256, prepare_host_editorial_task, record_model_attribution,
        update_editorial_controller,
    },
    line_edit::LineEditBudget,
    revision::{
        JudgeBudget, RevisionBudget, RevisionLoopOptions, compare_candidate_checks,
        evaluate_deterministic_checks, resolve_double_order_judgment, select_initial_judge_order,
    },
};
use spindle_core::drafting::{
    ChapterCoverageV1, ChapterCriticOutcomeV1, ChapterCritiqueV1, ChapterEditApplicationV1,
    ChapterEditResultV1, ChapterEditSelectionV1, CriticFindingInput, DeblindedArm,
    EditorialActionKind, EditorialBudgetUsage, EditorialControllerStateV1, EditorialPolicy,
    EditorialStage, HostEditorialSubmissionV1, HostEditorialTaskV1, HostSubmissionOutcomeV1,
    LineEditOutcomeV1, ManualJudgmentReason, RevisionLoopArtifactV1, RevisionRetentionReason,
    TEMPLATE_CHAPTER_CRITIC, TEMPLATE_JUDGE, TEMPLATE_LINE_EDIT, TEMPLATE_PACKET_DRAFT,
    TEMPLATE_REVISION, TEMPLATE_VERSION_V1,
    extraction::{DraftReceiptBinding, PlainTextDraftArtifactV1},
};

impl SqliteSpindleService {
    /// Prepare a bounded hybrid host task from a persisted draft artifact. For a
    /// revision stage, the latest persisted loop round supplies directives.
    pub async fn prepare_editorial_host_task_from_artifact(
        &self,
        controller_path: &std::path::Path,
        scene_index: usize,
        task_id: &str,
        draft_artifact_path: &std::path::Path,
        now_millis: u64,
    ) -> Result<HostEditorialTaskV1> {
        let draft = artifact::load_artifact(draft_artifact_path)?;
        let prepared = draft.prepared_packet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("draft artifact missing prepared_packet; cannot prepare host task")
        })?;
        let packet = prepared.packet.clone();
        let packet_hash = draft.packet_hash.clone();
        let incumbent = draft.text.clone();
        update_editorial_controller(controller_path, |state| {
            anyhow::ensure!(
                state.settings.editorial_policy == EditorialPolicy::CritiqueV1,
                "host editorial tasks require critique_v1 policy"
            );
            let scene = state
                .scenes
                .get_mut(scene_index)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            scene.draft_artifact_path = Some(draft_artifact_path.to_string_lossy().into_owned());
            if scene.stage == EditorialStage::Critiqued
                && scene.selected_directives.is_empty()
                && let Some(loop_path) = scene.loop_artifact_path.as_deref()
            {
                let loop_artifact = crate::drafting::revision::load_revision_loop_artifact(
                    std::path::Path::new(loop_path),
                )?;
                if let Some(round) = loop_artifact.rounds.last() {
                    scene.selected_directives = round.selected_directives.clone();
                    scene.strengths_to_preserve = round.strengths_to_preserve.clone();
                }
            }
            prepare_host_editorial_task(
                state,
                scene_index,
                task_id,
                &packet_hash,
                &packet,
                Some(&incumbent),
                now_millis,
            )
        })
    }

    /// Validate and apply a host submission, persist its exact prose/receipt,
    /// then attempt automated double-order judgment. A missing or failed judge
    /// retains the incumbent and returns an explicit pending manual judgment.
    pub async fn submit_editorial_host_task(
        &self,
        controller_path: &std::path::Path,
        draft_artifact_path: &std::path::Path,
        submission: HostEditorialSubmissionV1,
        tokenizer: Option<&dyn DraftTokenizer>,
        now_millis: u64,
    ) -> Result<HostSubmissionOutcomeV1> {
        let candidate_path = candidate_artifact_path(draft_artifact_path);
        let (outcome, scene_index) = update_editorial_controller(controller_path, |state| {
            let (outcome, comparison) =
                apply_host_editorial_submission(state, &submission, now_millis)?;
            let scene_index = state
                .scenes
                .iter()
                .position(|scene| {
                    scene
                        .pending_host_task
                        .as_ref()
                        .is_some_and(|task| task.task_id == submission.task_id)
                })
                .or_else(|| {
                    state
                        .scenes
                        .iter()
                        .position(|scene| scene.stage == EditorialStage::Drafted)
                })
                .ok_or_else(|| anyhow::anyhow!("editorial scene for host task not found"))?;
            let scene = &mut state.scenes[scene_index];
            match &outcome {
                HostSubmissionOutcomeV1::DraftAccepted { receipt, .. } => {
                    let task = scene
                        .pending_host_task
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("host task missing after acceptance"))?;
                    let mut captured = artifact::capture_plain_text(
                        &task.packet,
                        &task.packet_hash,
                        submission.text.clone(),
                        false,
                        DraftReceiptBinding {
                            generation_id: format!("host:{}", receipt.task_id),
                            agent_id: receipt.agent_id.clone(),
                            normalized_output_sha256: receipt.normalized_output_sha256.clone(),
                        },
                    )?;
                    // Preserve the exact rendered-packet snapshot so a later
                    // revision task can reload it without re-assembly.
                    captured.prepared_packet = artifact::load_artifact(draft_artifact_path)
                        .ok()
                        .and_then(|prior| prior.prepared_packet);
                    artifact::save_artifact(draft_artifact_path, &captured)?;
                    scene.draft_artifact_path =
                        Some(draft_artifact_path.to_string_lossy().into_owned());
                    record_submission_attribution(
                        state,
                        scene_index,
                        EditorialActionKind::Draft,
                        TEMPLATE_PACKET_DRAFT,
                        &captured.text_sha256,
                        None,
                        &submission,
                        now_millis,
                    )?;
                }
                HostSubmissionOutcomeV1::CandidateReady { receipt, .. } => {
                    let task = scene
                        .pending_host_task
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("host task missing after acceptance"))?;
                    let mut captured = artifact::capture_plain_text(
                        &task.packet,
                        &task.packet_hash,
                        submission.text.clone(),
                        false,
                        DraftReceiptBinding {
                            generation_id: format!("host:{}", receipt.task_id),
                            agent_id: receipt.agent_id.clone(),
                            normalized_output_sha256: receipt.normalized_output_sha256.clone(),
                        },
                    )?;
                    captured.prepared_packet = artifact::load_artifact(draft_artifact_path)
                        .ok()
                        .and_then(|prior| prior.prepared_packet);
                    artifact::save_artifact(&candidate_path, &captured)?;
                    scene.candidate_artifact_path =
                        Some(candidate_path.to_string_lossy().into_owned());
                    record_submission_attribution(
                        state,
                        scene_index,
                        EditorialActionKind::Revise,
                        TEMPLATE_REVISION,
                        &captured.text_sha256,
                        comparison.as_ref(),
                        &submission,
                        now_millis,
                    )?;
                }
                _ => {}
            }
            Ok((outcome, scene_index))
        })?;

        if !matches!(outcome, HostSubmissionOutcomeV1::CandidateReady { .. }) {
            return Ok(outcome);
        }

        let controller = load_editorial_controller(controller_path)?;
        let scene = &controller.scenes[scene_index];
        let Some(task) = scene.pending_host_task.clone() else {
            return Ok(outcome);
        };
        let rating = task.rating.as_str().to_string();
        let router = self.repository.model_router();
        let availability = router.editorial_route_availability(&rating);
        if !availability.review {
            return self
                .park_host_judgment(
                    controller_path,
                    scene_index,
                    ManualJudgmentReason::NoAutomatedJudgeRoute,
                    now_millis,
                )
                .await;
        }
        if tokenizer.is_none() && !availability.verified_tokenizer {
            return self
                .park_host_judgment(
                    controller_path,
                    scene_index,
                    ManualJudgmentReason::MissingVerifiedTokenizer,
                    now_millis,
                )
                .await;
        }
        let judge_budget = match router.eval_judge_budget_and_settings(
            "review",
            &rating,
            controller.settings.generation_seed,
        ) {
            Ok((budget, _settings)) => budget,
            Err(_) => {
                return self
                    .park_host_judgment(
                        controller_path,
                        scene_index,
                        ManualJudgmentReason::NoAutomatedJudgeRoute,
                        now_millis,
                    )
                    .await;
            }
        };
        admit_host_judge_action(controller_path, scene_index, now_millis)?;

        let incumbent_text = task.incumbent_text.clone().unwrap_or_default();
        let incumbent_hash = scene
            .incumbent
            .as_ref()
            .map(|reference| reference.text_sha256.clone())
            .unwrap_or_default();
        let candidate_artifact = match scene.candidate_artifact_path.as_deref() {
            Some(path) => artifact::load_artifact(std::path::Path::new(path))?,
            None => {
                return self
                    .park_host_judgment(
                        controller_path,
                        scene_index,
                        ManualJudgmentReason::InvalidJudgeOutput,
                        now_millis,
                    )
                    .await;
            }
        };
        let candidate_text = candidate_artifact.text.clone();
        let candidate_hash = candidate_artifact.text_sha256.clone();
        let (pass_1_arm, pass_2_arm) = select_initial_judge_order(
            &task.packet_hash,
            &incumbent_hash,
            &candidate_hash,
            1,
            controller.settings.generation_seed,
        );
        let (passage_1_arm, passage_1_text, passage_2_arm, passage_2_text) = match pass_1_arm {
            DeblindedArm::Baseline => (
                DeblindedArm::Baseline,
                incumbent_text.as_str(),
                DeblindedArm::Candidate,
                candidate_text.as_str(),
            ),
            DeblindedArm::Candidate => (
                DeblindedArm::Candidate,
                candidate_text.as_str(),
                DeblindedArm::Baseline,
                incumbent_text.as_str(),
            ),
            DeblindedArm::Tie => (
                DeblindedArm::Baseline,
                incumbent_text.as_str(),
                DeblindedArm::Candidate,
                candidate_text.as_str(),
            ),
        };
        let judge_start = std::time::Instant::now();
        let pass_1 = match router
            .judge_candidate_single_pass(
                EditorialPolicy::CritiqueV1,
                &task.packet,
                1,
                passage_1_arm,
                passage_1_text,
                passage_2_arm,
                passage_2_text,
                &judge_budget,
                tokenizer,
                None,
            )
            .await
        {
            Ok(pass) => pass,
            Err(_) => {
                return self
                    .park_host_judgment(
                        controller_path,
                        scene_index,
                        ManualJudgmentReason::InvalidJudgeOutput,
                        now_millis,
                    )
                    .await;
            }
        };
        let pass_2 = match router
            .judge_candidate_single_pass(
                EditorialPolicy::CritiqueV1,
                &task.packet,
                2,
                passage_2_arm,
                passage_2_text,
                passage_1_arm,
                passage_1_text,
                &judge_budget,
                tokenizer,
                None,
            )
            .await
        {
            Ok(pass) => pass,
            Err(_) => {
                return self
                    .park_host_judgment(
                        controller_path,
                        scene_index,
                        ManualJudgmentReason::InvalidJudgeOutput,
                        now_millis,
                    )
                    .await;
            }
        };
        let (evaluation, retention) =
            resolve_double_order_judgment(pass_1.record.clone(), pass_2.record.clone());
        let elapsed_millis = u64::try_from(judge_start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let input_tokens = pass_1.input_tokens.saturating_add(pass_2.input_tokens);
        let output_tokens = pass_1.output_tokens.saturating_add(pass_2.output_tokens);
        // Known usage includes measured wall time and configured-price cost.
        // Missing pricing keeps `actual = None` so the maximum reservation is
        // charged instead of refunding the paid judge calls.
        let usage = controller
            .settings
            .pricing
            .as_ref()
            .map(|pricing| EditorialBudgetUsage {
                calls: 2,
                tokens: input_tokens.saturating_add(output_tokens),
                millis: elapsed_millis,
                usd_micros: pricing.cost_micros(input_tokens, output_tokens),
            });
        let model_call_ids: Vec<String> = pass_1
            .record
            .model_call_id
            .clone()
            .into_iter()
            .chain(pass_2.record.model_call_id.clone())
            .collect();

        if evaluation.accepted {
            // Promote before recording the terminal judgment so the controller
            // never claims an acceptance the draft artifact does not hold.
            self.promote_host_candidate(
                draft_artifact_path,
                &task.task_id,
                &task.packet_hash,
                &incumbent_hash,
                &candidate_text,
                &candidate_hash,
            )
            .await?;
        }

        update_editorial_controller(controller_path, |state| {
            {
                let scene = state
                    .scenes
                    .get_mut(scene_index)
                    .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
                let judge_key = judge_reservation_key(&state.run_id, &state.branch_id, scene);
                scene.ledger.reconcile(
                    &judge_key.idempotency_key(),
                    usage,
                    model_call_ids.clone(),
                    now_millis,
                );
            }
            record_model_attribution(
                state,
                scene_index,
                EditorialActionKind::Judge,
                TEMPLATE_JUDGE.into(),
                TEMPLATE_VERSION_V1.into(),
                pass_1.prompt_hash.clone(),
                pass_1.prompt_hash.clone(),
                None,
                Some(incumbent_hash.clone()),
                Some(candidate_hash.clone()),
                Some(format!("pass_1:{passage_1_arm:?}")),
                evaluation.verdict.as_str().into(),
                elapsed_millis,
                usage,
                usage.unwrap_or_else(|| EditorialBudgetUsage {
                    calls: 2,
                    tokens: input_tokens.saturating_add(output_tokens),
                    millis: elapsed_millis,
                    usd_micros: 0,
                }),
                model_call_ids.clone(),
                now_millis,
            )?;
            crate::drafting::editorial::record_editorial_judgment(
                state,
                scene_index,
                &evaluation,
                retention,
                now_millis,
            )?;
            Ok(())
        })?;

        let _ = pass_2_arm;
        Ok(if evaluation.accepted {
            HostSubmissionOutcomeV1::RevisionAccepted {
                receipt: host_receipt(&task.task_id, &candidate_text),
                text_sha256: candidate_hash,
            }
        } else {
            HostSubmissionOutcomeV1::RevisionRetained {
                reason: retention
                    .map(|reason| reason.as_str().to_string())
                    .unwrap_or_else(|| "judge_retained_incumbent".into()),
                incumbent_text_sha256: incumbent_hash,
                candidate_text_sha256: candidate_hash,
            }
        })
    }

    async fn park_host_judgment(
        &self,
        controller_path: &std::path::Path,
        scene_index: usize,
        reason: ManualJudgmentReason,
        now_millis: u64,
    ) -> Result<HostSubmissionOutcomeV1> {
        let pending = update_editorial_controller(controller_path, |state| {
            crate::drafting::editorial::mark_pending_manual_judgment(
                state,
                scene_index,
                reason,
                now_millis,
            )?;
            let pending = state
                .scenes
                .get(scene_index)
                .and_then(|scene| scene.pending_manual_judgment.clone())
                .ok_or_else(|| anyhow::anyhow!("pending manual judgment was not recorded"))?;
            Ok(pending)
        })?;
        Ok(HostSubmissionOutcomeV1::PendingManualJudgment { pending })
    }

    /// Agent-mode parity for the initial draft: dispatch the same rendered packet
    /// through the existing packet-drafting path and record the controller state.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_editorial_agent_draft(
        &self,
        controller_path: &std::path::Path,
        scene_index: usize,
        packet: spindle_core::drafting::DraftingPacketV1,
        budget: &PacketBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        draft_artifact_path: &std::path::Path,
        now_millis: u64,
    ) -> Result<PlainTextDraftArtifactV1> {
        let packet_content_hash = editorial_sha256_hex(&serde_json::to_vec(&packet)?);
        // Admission is persisted in a locked controller update *before* the paid
        // dispatch, so a crash or a failed dispatch cannot refund it and two
        // concurrent callers cannot admit against the same counters.
        let reservation =
            admit_editorial_agent_draft_action(controller_path, scene_index, now_millis)?;
        let reserved = reservation.reserved;
        let reservation_key = reservation.idempotency_key.clone();
        let started = std::time::Instant::now();
        let captured = match self
            .draft_packet_to_artifact(
                EditorialPolicy::CritiqueV1,
                packet,
                budget,
                tokenizer,
                draft_artifact_path,
            )
            .await
        {
            Ok(captured) => captured,
            Err(error) => {
                let elapsed = elapsed_millis(started);
                let _ = reconcile_editorial_agent_draft_action(
                    controller_path,
                    scene_index,
                    draft_artifact_path,
                    &reservation_key,
                    reserved,
                    None,
                    tokenizer,
                    elapsed,
                    now_millis,
                    packet_content_hash,
                    Vec::new(),
                );
                return Err(error);
            }
        };
        let elapsed = elapsed_millis(started);
        let model_call_ids = self
            .editorial_generation_model_call_ids(&captured.receipt.generation_id)
            .await;
        reconcile_editorial_agent_draft_action(
            controller_path,
            scene_index,
            draft_artifact_path,
            &reservation_key,
            reserved,
            Some(&captured),
            tokenizer,
            elapsed,
            now_millis,
            packet_content_hash,
            model_call_ids,
        )?;
        Ok(captured)
    }

    /// Run one advisory chapter critic pass. The critique is persisted and
    /// returned; no prose or plan is mutated. A missing route/tokenizer/price
    /// returns an honest skip.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_editorial_chapter_critic(
        &self,
        controller_path: &std::path::Path,
        scene_index: usize,
        rating: &str,
        coverage: &ChapterCoverageV1,
        scene_texts: &std::collections::BTreeMap<String, String>,
        feedback: &[CriticFindingInput],
        budget: &ChapterCriticBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        now_millis: u64,
    ) -> Result<ChapterCriticOutcomeV1> {
        let controller = load_editorial_controller(controller_path)?;
        anyhow::ensure!(
            controller.settings.editorial_policy == EditorialPolicy::CritiqueV1,
            "chapter critic requires critique_v1 policy"
        );
        if !controller.settings.chapter_pass_enabled {
            return Ok(ChapterCriticOutcomeV1::Skipped {
                reason: "chapter_pass_disabled".into(),
            });
        }
        let router = self.repository.model_router();
        let availability = router.editorial_route_availability(rating);
        if !availability.review || (tokenizer.is_none() && !availability.verified_tokenizer) {
            return Ok(ChapterCriticOutcomeV1::Skipped {
                reason: "chapter_critic_route_unavailable".into(),
            });
        }
        let windows = coverage.windows.clone();
        if windows.is_empty() {
            return Ok(ChapterCriticOutcomeV1::Skipped {
                reason: "no_chapter_windows".into(),
            });
        }
        let mut aggregate = ChapterCritiqueV1 {
            schema_version: spindle_core::drafting::CHAPTER_CRITIQUE_SCHEMA_VERSION.into(),
            chapter_number: coverage.chapter_number,
            chapter_text_hash: crate::drafting::chapter::chapter_text_hash(coverage, scene_texts),
            concerns: Vec::new(),
            edit_proposals: Vec::new(),
            strengths_to_preserve: Vec::new(),
            requires_author_decision: Vec::new(),
            coverage: Some(coverage.clone()),
            windows_reviewed: 0,
            windows_skipped: 0,
            no_edit_reason: None,
        };
        let mut prompt_hashes = Vec::new();
        let mut model_call_ids = Vec::new();
        let mut seen_concern_ids = std::collections::BTreeSet::new();
        for window in &windows {
            let window_texts: std::collections::BTreeMap<String, String> = window
                .scene_ids
                .iter()
                .filter_map(|scene_id| {
                    scene_texts
                        .get(scene_id)
                        .map(|text| (scene_id.clone(), text.clone()))
                })
                .collect();
            if window_texts.len() != window.scene_ids.len() {
                aggregate.windows_skipped += 1;
                continue;
            }
            let reservation = match update_editorial_controller(controller_path, |state| {
                let reservation = crate::drafting::editorial::admit_editorial_action_in_controller(
                    state,
                    scene_index,
                    EditorialActionKind::ChapterCritique,
                    window.index,
                    now_millis,
                    true,
                )
                .map_err(|denial| anyhow::anyhow!("{denial}"))?;
                let claim = format!("chapter_critic_window_{}", window.index);
                let claimed = state.scenes[scene_index].ledger.claim_dispatch(
                    &reservation.idempotency_key,
                    &claim,
                    now_millis,
                );
                anyhow::ensure!(
                    claimed == Some(true),
                    "editorial_dispatch_in_progress: chapter critic window {} already dispatched",
                    window.index
                );
                Ok(reservation)
            }) {
                Ok(reservation) => reservation,
                Err(_) => {
                    aggregate.windows_skipped += 1;
                    continue;
                }
            };
            let key = reservation.idempotency_key.clone();
            let reserved = reservation.reserved;
            let started = std::time::Instant::now();
            let result = router
                .chapter_critic(
                    EditorialPolicy::CritiqueV1,
                    rating,
                    coverage,
                    Some(window),
                    &window_texts,
                    feedback,
                    budget,
                    tokenizer,
                )
                .await;
            let elapsed_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            match result {
                Ok(result) => {
                    aggregate.windows_reviewed += 1;
                    let window_path = controller_path.with_extension(format!(
                        "chapter-{}.window-{}.critique.json",
                        coverage.chapter_number, window.index
                    ));
                    save_chapter_critique(&window_path, &result.critique)?;
                    for concern in result.critique.concerns {
                        if seen_concern_ids.insert(concern.id.clone()) {
                            aggregate.concerns.push(concern);
                        }
                    }
                    aggregate
                        .edit_proposals
                        .extend(result.critique.edit_proposals);
                    aggregate
                        .strengths_to_preserve
                        .extend(result.critique.strengths_to_preserve);
                    aggregate
                        .requires_author_decision
                        .extend(result.critique.requires_author_decision);
                    prompt_hashes.push(result.prompt_hash.clone());
                    model_call_ids.extend(result.model_call_ids.clone());
                    let actual =
                        controller
                            .settings
                            .pricing
                            .as_ref()
                            .map(|pricing| EditorialBudgetUsage {
                                calls: 1,
                                tokens: result.input_tokens.saturating_add(result.output_tokens),
                                millis: elapsed_millis,
                                usd_micros: pricing
                                    .cost_micros(result.input_tokens, result.output_tokens),
                            });
                    reconcile_chapter_critic(
                        controller_path,
                        scene_index,
                        &key,
                        &result.prompt_hash,
                        reserved,
                        actual,
                        result.model_call_ids,
                        now_millis,
                    )?;
                }
                Err(_) => {
                    aggregate.windows_skipped += 1;
                    let _ = reconcile_chapter_critic(
                        controller_path,
                        scene_index,
                        &key,
                        "",
                        reserved,
                        None,
                        Vec::new(),
                        now_millis,
                    );
                }
            }
        }
        if aggregate.windows_reviewed == 0 {
            return Ok(ChapterCriticOutcomeV1::Skipped {
                reason: "chapter_critic_no_window_reviewed".into(),
            });
        }
        aggregate
            .edit_proposals
            .truncate(spindle_core::drafting::MAX_CHAPTER_EDIT_PROPOSALS);
        aggregate.strengths_to_preserve.sort();
        aggregate.strengths_to_preserve.dedup();
        aggregate
            .requires_author_decision
            .dedup_by(|left, right| left.id == right.id);
        let critique_path = controller_path
            .with_extension(format!("chapter-{}.critique.json", coverage.chapter_number));
        save_chapter_critique(&critique_path, &aggregate)?;
        let prompt_hash = if prompt_hashes.len() == 1 {
            prompt_hashes.remove(0)
        } else {
            crate::drafting::editorial::editorial_sha256_hex(prompt_hashes.join(":").as_bytes())
        };
        let windows_reviewed = aggregate.windows_reviewed;
        let windows_skipped = aggregate.windows_skipped;
        Ok(ChapterCriticOutcomeV1::Critiqued {
            critique: Box::new(aggregate),
            critique_artifact_path: critique_path.to_string_lossy().into_owned(),
            prompt_hash,
            windows_reviewed,
            windows_skipped,
            model_call_ids,
        })
    }

    /// Apply the author-selected subset of chapter edit proposals. Each selected
    /// proposal reuses the scene deterministic-check and doubled-order judge gate
    /// with bounded chapter context; unselected proposals are never applied.
    /// The returned invalidation plan must be honored before the next checkpoint.
    #[allow(clippy::too_many_arguments)]
    pub async fn apply_editorial_chapter_edits(
        &self,
        controller_path: &std::path::Path,
        scene_index: usize,
        draft_artifact_path: &std::path::Path,
        chapter_context: &str,
        critique: &ChapterCritiqueV1,
        selection: &ChapterEditSelectionV1,
        revision_budget: &RevisionBudget,
        judge_budget: &JudgeBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        options: RevisionLoopOptions,
        now_millis: u64,
    ) -> Result<ChapterEditApplicationV1> {
        let controller = load_editorial_controller(controller_path)?;
        anyhow::ensure!(
            controller.settings.chapter_pass_enabled,
            "chapter pass is disabled"
        );
        let mut selected: Vec<&spindle_core::drafting::ChapterEditProposalV1> = selection
            .selected_proposals(critique)
            .into_iter()
            .take(spindle_core::drafting::MAX_CHAPTER_EDIT_PROPOSALS)
            .collect();
        selected.sort_by(|left, right| left.id.cmp(&right.id));
        let mut edits = Vec::new();
        let mut accepted_scene_ids = Vec::new();
        for (proposal_index, proposal) in selected.into_iter().enumerate() {
            let draft = artifact::load_artifact(draft_artifact_path)?;
            if draft.text_sha256 != proposal.base_text_sha256 {
                edits.push(ChapterEditResultV1 {
                    proposal_id: proposal.id.clone(),
                    scene_id: proposal.scene_id.clone(),
                    accepted: false,
                    stage: "skipped".into(),
                    text_sha256: None,
                    reason: Some("stale_proposal_base_hash".into()),
                });
                continue;
            }
            if draft
                .scene_id
                .as_deref()
                .is_some_and(|scene_id| scene_id != proposal.scene_id)
            {
                edits.push(ChapterEditResultV1 {
                    proposal_id: proposal.id.clone(),
                    scene_id: proposal.scene_id.clone(),
                    accepted: false,
                    stage: "skipped".into(),
                    text_sha256: None,
                    reason: Some("proposal_scene_mismatch".into()),
                });
                continue;
            }
            let scene_texts =
                std::collections::BTreeMap::from([(proposal.scene_id.clone(), draft.text.clone())]);
            if spindle_core::drafting::validate_chapter_anchor(&proposal.anchor, &scene_texts)
                .is_err()
            {
                edits.push(ChapterEditResultV1 {
                    proposal_id: proposal.id.clone(),
                    scene_id: proposal.scene_id.clone(),
                    accepted: false,
                    stage: "skipped".into(),
                    text_sha256: None,
                    reason: Some("invalid_proposal_anchor".into()),
                });
                continue;
            }
            let directive = spindle_core::drafting::chapter_proposal_to_directive(proposal);
            let mut edit_options = options.clone();
            edit_options.max_rounds = 1;
            edit_options.pre_selected_directives = Some(vec![directive]);
            edit_options.chapter_context = Some(chapter_context.to_string());
            let safe_id: String = proposal
                .id
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                        ch
                    } else {
                        '_'
                    }
                })
                .collect();
            let loop_path =
                draft_artifact_path.with_extension(format!("chapter-edit-{safe_id}.loop.json"));
            let critic_budget = crate::drafting::critique::CriticBudget::scene_default(
                revision_budget.model_id.clone(),
                revision_budget.context_window,
            );
            let gate = ControllerRevisionBudgetGate::with_namespace(
                controller_path,
                scene_index,
                now_millis,
                u32::try_from(proposal_index + 1).unwrap_or(u32::MAX),
            );
            let result = self
                .revise_artifact_loop_with_gate(
                    EditorialPolicy::CritiqueV1,
                    draft_artifact_path,
                    &loop_path,
                    Vec::new(),
                    &critic_budget,
                    revision_budget,
                    judge_budget,
                    tokenizer,
                    edit_options,
                    Some(&gate),
                )
                .await;
            match result {
                Ok(loop_artifact) => {
                    let accepted = loop_artifact.current_stage == EditorialStage::AcceptedRevision;
                    if accepted {
                        accepted_scene_ids.push(proposal.scene_id.clone());
                    }
                    sync_scene_after_chapter_edit(
                        controller_path,
                        scene_index,
                        &loop_path,
                        &loop_artifact,
                        now_millis,
                    )?;
                    edits.push(ChapterEditResultV1 {
                        proposal_id: proposal.id.clone(),
                        scene_id: proposal.scene_id.clone(),
                        accepted,
                        stage: loop_artifact.current_stage.as_str().into(),
                        text_sha256: Some(loop_artifact.accepted_text_sha256.clone()),
                        reason: loop_artifact
                            .retention_reason
                            .map(|reason| reason.as_str().to_string()),
                    });
                }
                Err(error) => {
                    edits.push(ChapterEditResultV1 {
                        proposal_id: proposal.id.clone(),
                        scene_id: proposal.scene_id.clone(),
                        accepted: false,
                        stage: "error".into(),
                        text_sha256: None,
                        reason: Some(error.to_string()),
                    });
                }
            }
        }
        let invalidation = spindle_core::drafting::plan_chapter_invalidation(
            critique.chapter_number,
            &accepted_scene_ids,
        );
        Ok(ChapterEditApplicationV1 {
            chapter_number: critique.chapter_number,
            edits,
            invalidation,
        })
    }

    /// Execute the optional line-edit pass over a terminal accepted scene. The
    /// candidate passes deterministic checks and the same doubled-order judge;
    /// any non-acceptance leaves the accepted prose untouched and marks the
    /// attempt so it is not repeated.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_editorial_agent_line_edit(
        &self,
        controller_path: &std::path::Path,
        scene_index: usize,
        draft_artifact_path: &std::path::Path,
        line_budget: &LineEditBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        now_millis: u64,
    ) -> Result<LineEditOutcomeV1> {
        let controller = load_editorial_controller(controller_path)?;
        let scene = controller
            .scenes
            .get(scene_index)
            .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
        if !controller.settings.line_edit_enabled
            || scene.line_edit_attempted
            || !scene.stage.is_terminal()
        {
            return Ok(LineEditOutcomeV1::Skipped {
                reason: "line_edit_not_applicable".into(),
            });
        }
        let draft = artifact::load_artifact(draft_artifact_path)?;
        let prepared = draft.prepared_packet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("draft artifact missing prepared_packet; cannot line-edit")
        })?;
        let packet = prepared.packet.clone();
        let rating = packet.scene_card.rating.as_str().to_string();
        let router = self.repository.model_router();
        let availability = router.editorial_route_availability(&rating);
        if !availability.line_edit {
            mark_line_edit_attempted(controller_path, scene_index, now_millis)?;
            return Ok(LineEditOutcomeV1::Skipped {
                reason: "line_edit_route_unavailable".into(),
            });
        }
        let reservation = update_editorial_controller(controller_path, |state| {
            let attempt = state
                .scenes
                .get(scene_index)
                .map(|scene| scene.attempt)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            let reservation = crate::drafting::editorial::admit_editorial_action_in_controller(
                state,
                scene_index,
                EditorialActionKind::LineEdit,
                attempt,
                now_millis,
                true,
            )
            .map_err(|denial| anyhow::anyhow!("{denial}"))?;
            let claimed = state.scenes[scene_index].ledger.claim_dispatch(
                &reservation.idempotency_key,
                "line_edit",
                now_millis,
            );
            anyhow::ensure!(
                claimed == Some(true),
                "editorial_dispatch_in_progress: line-edit pass already dispatched"
            );
            Ok(reservation)
        })?;
        let line_reserved = reservation.reserved;
        let line_key = reservation.idempotency_key.clone();
        let started = std::time::Instant::now();
        let candidate = match router
            .line_edit_candidate(
                EditorialPolicy::CritiqueV1,
                &packet,
                &draft.text,
                &draft.text_sha256,
                line_budget,
                tokenizer,
            )
            .await
        {
            Ok(candidate) => candidate,
            Err(error) => {
                let _ = finalize_line_edit(
                    controller_path,
                    scene_index,
                    &line_key,
                    line_reserved,
                    None,
                    Vec::new(),
                    None,
                    now_millis,
                    LineEditOutcomeV1::Skipped {
                        reason: format!("line_edit_dispatch_failed: {error}"),
                    },
                    draft.text_sha256.clone(),
                    None,
                );
                return Ok(LineEditOutcomeV1::Skipped {
                    reason: format!("line_edit_dispatch_failed: {error}"),
                });
            }
        };
        let elapsed_millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let call_ids: Vec<String> = candidate.response.call_id.clone().into_iter().collect();
        let receipt = self
            .register_generation_receipt_with_calls(
                "line_edit",
                Some(&rating),
                &candidate.response.model_name,
                &candidate.response.output,
                call_ids.clone(),
            )
            .await?;
        let receipt_binding = DraftReceiptBinding {
            generation_id: receipt.id.clone(),
            agent_id: receipt.agent_id.clone(),
            normalized_output_sha256: receipt.output_sha256.clone(),
        };
        let mut candidate_artifact = artifact::capture_plain_text(
            &packet,
            &draft.packet_hash,
            candidate.response.output.clone(),
            candidate.response.truncated,
            receipt_binding.clone(),
        )?;
        candidate_artifact.prepared_packet = draft.prepared_packet.clone();
        let candidate_path = line_edit_candidate_path(draft_artifact_path);
        artifact::save_artifact(&candidate_path, &candidate_artifact)?;
        let candidate_hash = candidate_artifact.text_sha256.clone();
        let pricing = controller.settings.pricing.as_ref();
        let line_actual = pricing.map(|pricing| EditorialBudgetUsage {
            calls: 1,
            tokens: candidate
                .input_tokens
                .saturating_add(candidate.output_tokens),
            millis: elapsed_millis,
            usd_micros: pricing.cost_micros(candidate.input_tokens, candidate.output_tokens),
        });

        let incumbent_report =
            evaluate_deterministic_checks(&packet, &draft.text, false, Vec::new())
                .map_err(|error| anyhow::anyhow!("{error}"))?;
        let candidate_report = evaluate_deterministic_checks(
            &packet,
            &candidate.response.output,
            candidate.response.truncated,
            Vec::new(),
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        let comparison = compare_candidate_checks(
            &packet,
            &draft.text,
            &incumbent_report,
            &candidate.response.output,
            candidate_report,
        );
        let retention_reason = if candidate.response.truncated {
            Some(RevisionRetentionReason::CandidateTruncated)
        } else if comparison.disqualified {
            Some(RevisionRetentionReason::CandidateDisqualifiedByDeterministicCheck)
        } else {
            None
        };
        if let Some(reason) = retention_reason {
            let outcome = LineEditOutcomeV1::Retained {
                reason: reason.as_str().into(),
                incumbent_text_sha256: draft.text_sha256.clone(),
                candidate_text_sha256: candidate_hash.clone(),
            };
            finalize_line_edit(
                controller_path,
                scene_index,
                &line_key,
                line_reserved,
                line_actual,
                call_ids,
                Some((candidate.response.output.clone(), candidate_hash.clone())),
                now_millis,
                outcome.clone(),
                draft.text_sha256.clone(),
                None,
            )?;
            return Ok(outcome);
        }
        if !availability.review || (tokenizer.is_none() && !availability.verified_tokenizer) {
            let outcome = LineEditOutcomeV1::Retained {
                reason: "judge_unavailable".into(),
                incumbent_text_sha256: draft.text_sha256.clone(),
                candidate_text_sha256: candidate_hash.clone(),
            };
            finalize_line_edit(
                controller_path,
                scene_index,
                &line_key,
                line_reserved,
                line_actual,
                call_ids,
                Some((candidate.response.output.clone(), candidate_hash.clone())),
                now_millis,
                outcome.clone(),
                draft.text_sha256.clone(),
                None,
            )?;
            return Ok(outcome);
        }
        let judge_budget = match router.eval_judge_budget_and_settings(
            "review",
            &rating,
            controller.settings.generation_seed,
        ) {
            Ok((budget, _settings)) => budget,
            Err(_) => {
                let outcome = LineEditOutcomeV1::Retained {
                    reason: "judge_unavailable".into(),
                    incumbent_text_sha256: draft.text_sha256.clone(),
                    candidate_text_sha256: candidate_hash.clone(),
                };
                finalize_line_edit(
                    controller_path,
                    scene_index,
                    &line_key,
                    line_reserved,
                    line_actual,
                    call_ids,
                    Some((candidate.response.output.clone(), candidate_hash.clone())),
                    now_millis,
                    outcome.clone(),
                    draft.text_sha256.clone(),
                    None,
                )?;
                return Ok(outcome);
            }
        };
        let judge_reservation = match update_editorial_controller(controller_path, |state| {
            let attempt = state
                .scenes
                .get(scene_index)
                .map(|scene| scene.attempt)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            let reservation = crate::drafting::editorial::admit_editorial_action_in_controller(
                state,
                scene_index,
                EditorialActionKind::Judge,
                attempt,
                now_millis,
                true,
            )
            .map_err(|denial| anyhow::anyhow!("{denial}"))?;
            let claimed = state.scenes[scene_index].ledger.claim_dispatch(
                &reservation.idempotency_key,
                "line_edit_judge",
                now_millis,
            );
            anyhow::ensure!(
                claimed == Some(true),
                "editorial_dispatch_in_progress: line-edit judge already dispatched"
            );
            Ok(reservation)
        }) {
            Ok(reservation) => reservation,
            Err(error) => {
                // The paid line-edit candidate exists, but judging cannot be
                // reserved. Keep the accepted prose, charge the completed line
                // call, mark the attempt, and stop offering line edit.
                let outcome = LineEditOutcomeV1::Retained {
                    reason: format!("judge_budget_denied: {error}"),
                    incumbent_text_sha256: draft.text_sha256.clone(),
                    candidate_text_sha256: candidate_hash.clone(),
                };
                finalize_line_edit(
                    controller_path,
                    scene_index,
                    &line_key,
                    line_reserved,
                    line_actual,
                    call_ids,
                    Some((candidate.response.output.clone(), candidate_hash.clone())),
                    now_millis,
                    outcome.clone(),
                    draft.text_sha256.clone(),
                    None,
                )?;
                return Ok(outcome);
            }
        };
        let judge_key = judge_reservation.idempotency_key.clone();
        let judge_reserved = judge_reservation.reserved;
        let (pass_1_arm, _pass_2_arm) = select_initial_judge_order(
            &draft.packet_hash,
            &draft.text_sha256,
            &candidate_hash,
            1,
            controller.settings.generation_seed,
        );
        let (p1_arm, p1_text, p2_arm, p2_text) = match pass_1_arm {
            DeblindedArm::Baseline => (
                DeblindedArm::Baseline,
                draft.text.as_str(),
                DeblindedArm::Candidate,
                candidate.response.output.as_str(),
            ),
            DeblindedArm::Candidate => (
                DeblindedArm::Candidate,
                candidate.response.output.as_str(),
                DeblindedArm::Baseline,
                draft.text.as_str(),
            ),
            DeblindedArm::Tie => (
                DeblindedArm::Baseline,
                draft.text.as_str(),
                DeblindedArm::Candidate,
                candidate.response.output.as_str(),
            ),
        };
        let judge_start = std::time::Instant::now();
        let pass_1 = router
            .judge_candidate_single_pass(
                EditorialPolicy::CritiqueV1,
                &packet,
                1,
                p1_arm,
                p1_text,
                p2_arm,
                p2_text,
                &judge_budget,
                tokenizer,
                None,
            )
            .await;
        let pass_1 = match pass_1 {
            Ok(pass) => pass,
            Err(error) => {
                let outcome = LineEditOutcomeV1::Retained {
                    reason: "judge_invalid_or_unavailable".into(),
                    incumbent_text_sha256: draft.text_sha256.clone(),
                    candidate_text_sha256: candidate_hash.clone(),
                };
                finalize_line_edit(
                    controller_path,
                    scene_index,
                    &line_key,
                    line_reserved,
                    line_actual,
                    call_ids,
                    Some((candidate.response.output.clone(), candidate_hash.clone())),
                    now_millis,
                    outcome.clone(),
                    draft.text_sha256.clone(),
                    Some((
                        judge_key,
                        judge_reserved,
                        None,
                        Vec::new(),
                        error.to_string(),
                    )),
                )?;
                return Ok(outcome);
            }
        };
        let pass_2 = router
            .judge_candidate_single_pass(
                EditorialPolicy::CritiqueV1,
                &packet,
                2,
                p2_arm,
                p2_text,
                p1_arm,
                p1_text,
                &judge_budget,
                tokenizer,
                None,
            )
            .await;
        let pass_2 = match pass_2 {
            Ok(pass) => pass,
            Err(error) => {
                let outcome = LineEditOutcomeV1::Retained {
                    reason: "judge_invalid_or_unavailable".into(),
                    incumbent_text_sha256: draft.text_sha256.clone(),
                    candidate_text_sha256: candidate_hash.clone(),
                };
                finalize_line_edit(
                    controller_path,
                    scene_index,
                    &line_key,
                    line_reserved,
                    line_actual,
                    call_ids,
                    Some((candidate.response.output.clone(), candidate_hash.clone())),
                    now_millis,
                    outcome.clone(),
                    draft.text_sha256.clone(),
                    Some((
                        judge_key,
                        judge_reserved,
                        None,
                        Vec::new(),
                        error.to_string(),
                    )),
                )?;
                return Ok(outcome);
            }
        };
        let (evaluation, retention) =
            resolve_double_order_judgment(pass_1.record.clone(), pass_2.record.clone());
        let judge_elapsed = u64::try_from(judge_start.elapsed().as_millis()).unwrap_or(u64::MAX);
        let judge_usage = pricing.map(|pricing| {
            let input = pass_1.input_tokens.saturating_add(pass_2.input_tokens);
            let output = pass_1.output_tokens.saturating_add(pass_2.output_tokens);
            EditorialBudgetUsage {
                calls: 2,
                tokens: input.saturating_add(output),
                millis: judge_elapsed,
                usd_micros: pricing.cost_micros(input, output),
            }
        });
        let judge_calls: Vec<String> = pass_1
            .record
            .model_call_id
            .clone()
            .into_iter()
            .chain(pass_2.record.model_call_id.clone())
            .collect();
        let accepted = evaluation.accepted;
        let outcome = if accepted {
            self.promote_line_edit_candidate(
                draft_artifact_path,
                &draft.packet_hash,
                &draft.text_sha256,
                &candidate.response.output,
                &candidate_hash,
                &receipt_binding,
            )
            .await?;
            LineEditOutcomeV1::Accepted {
                text_sha256: candidate_hash.clone(),
                generation_id: receipt_binding.generation_id.clone(),
            }
        } else {
            LineEditOutcomeV1::Retained {
                reason: retention
                    .map(|reason| reason.as_str().to_string())
                    .unwrap_or_else(|| "judge_retained_incumbent".into()),
                incumbent_text_sha256: draft.text_sha256.clone(),
                candidate_text_sha256: candidate_hash.clone(),
            }
        };
        finalize_line_edit(
            controller_path,
            scene_index,
            &line_key,
            line_reserved,
            line_actual,
            call_ids,
            Some((candidate.response.output.clone(), candidate_hash.clone())),
            now_millis,
            outcome.clone(),
            draft.text_sha256.clone(),
            Some((
                judge_key,
                judge_reserved,
                judge_usage,
                judge_calls,
                evaluation.verdict.as_str().into(),
            )),
        )?;
        Ok(outcome)
    }

    /// Agent-mode parity for the bounded revision loop. The controller imports
    /// the loop artifact's terminal state and records prose-free attribution for
    /// every persisted stage transition.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_editorial_agent_revision_loop(
        &self,
        controller_path: &std::path::Path,
        scene_index: usize,
        draft_artifact_path: &std::path::Path,
        loop_artifact_path: &std::path::Path,
        external_findings: Vec<spindle_core::drafting::CriticFindingInput>,
        critic_budget: &crate::drafting::critique::CriticBudget,
        revision_budget: &RevisionBudget,
        judge_budget: &JudgeBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        options: RevisionLoopOptions,
        now_millis: u64,
    ) -> Result<spindle_core::drafting::RevisionLoopArtifactV1> {
        let controller = load_editorial_controller(controller_path)?;
        anyhow::ensure!(
            controller.settings.editorial_policy == EditorialPolicy::CritiqueV1,
            "agent revision loop requires critique_v1 policy"
        );
        anyhow::ensure!(
            controller.scenes.get(scene_index).is_some(),
            "editorial scene index out of range"
        );
        let gate = ControllerRevisionBudgetGate::new(controller_path, scene_index, now_millis);
        let artifact = self
            .revise_artifact_loop_with_gate(
                EditorialPolicy::CritiqueV1,
                draft_artifact_path,
                loop_artifact_path,
                external_findings,
                critic_budget,
                revision_budget,
                judge_budget,
                tokenizer,
                options,
                Some(&gate),
            )
            .await?;
        update_editorial_controller(controller_path, |state| {
            let scene = state
                .scenes
                .get_mut(scene_index)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            scene.loop_artifact_path = Some(loop_artifact_path.to_string_lossy().into_owned());
            scene.stage = artifact.current_stage;
            scene.incumbent = Some(spindle_core::drafting::EditorialTextRefV1 {
                text_sha256: artifact.accepted_text_sha256.clone(),
                generation_id: Some(artifact.accepted_receipt.generation_id.clone()),
                agent_id: Some(artifact.accepted_receipt.agent_id.clone()),
            });
            if let Some(round) = artifact.rounds.last() {
                if let Some(outcome) = &round.critique_outcome {
                    let directive_count = round.selected_directives.len();
                    scene.critic = Some(spindle_core::drafting::EditorialCriticSummaryV1 {
                        status: match outcome {
                            spindle_core::drafting::ShadowCritiqueOutcome::Critiqued { .. } => {
                                "critiqued".into()
                            }
                            spindle_core::drafting::ShadowCritiqueOutcome::CriticInvalid {
                                ..
                            } => "critic_invalid".into(),
                        },
                        rubric_version: "scene-critic-v1".into(),
                        directive_count,
                        requires_author_decision_count: round.requires_author_decision.len(),
                    });
                }
                if let Some(judgment) = &round.double_order_judgment {
                    scene.judge = Some(spindle_core::drafting::EditorialJudgeSummaryV1 {
                        verdict: judgment.verdict.as_str().into(),
                        accepted_revision: judgment.accepted,
                        order_agreement: judgment.position_bias.is_none(),
                        retention_reason: round
                            .round_stop_reason
                            .map(|reason| reason.as_str().to_string()),
                    });
                }
            }
            for transition in &artifact.transitions {
                let action = action_for_stage(transition.key.stage);
                let template = action.prompt_template_id().unwrap_or(TEMPLATE_PACKET_DRAFT);
                record_model_attribution(
                    state,
                    scene_index,
                    action,
                    template.into(),
                    TEMPLATE_VERSION_V1.into(),
                    transition.output_hash.clone(),
                    transition.output_hash.clone(),
                    None,
                    Some(artifact.initial_text_sha256.clone()),
                    Some(artifact.accepted_text_sha256.clone()),
                    None,
                    transition.key.stage.as_str().into(),
                    0,
                    Some(EditorialBudgetUsage {
                        calls: u32::try_from(transition.model_call_ids.len()).unwrap_or(0),
                        tokens: transition.tokens_used,
                        millis: 0,
                        usd_micros: 0,
                    }),
                    EditorialBudgetUsage::default(),
                    transition.model_call_ids.clone(),
                    now_millis,
                )?;
            }
            Ok(())
        })?;
        Ok(artifact)
    }

    #[allow(clippy::too_many_arguments)]
    async fn promote_host_candidate(
        &self,
        draft_artifact_path: &std::path::Path,
        task_id: &str,
        expected_packet_hash: &str,
        expected_incumbent_hash: &str,
        candidate_text: &str,
        candidate_hash: &str,
    ) -> Result<()> {
        let draft = artifact::load_artifact(draft_artifact_path)?;
        let branch = self.repository.get_active_branch(&draft.project_id).await?;
        anyhow::ensure!(
            branch.id == draft.branch_id,
            "stale_cas_conflict: draft branch is no longer active"
        );
        anyhow::ensure!(
            editorial_sha256_hex(candidate_text.as_bytes()) == candidate_hash,
            "candidate hash mismatch before promotion"
        );
        let _lock = artifact::lock_artifact(draft_artifact_path)?;
        let mut draft = artifact::load_artifact(draft_artifact_path)?;
        anyhow::ensure!(
            draft.packet_hash == expected_packet_hash,
            "stale_cas_conflict: draft packet hash changed before promotion"
        );
        if draft.text_sha256 == candidate_hash
            && draft.receipt.generation_id == format!("host:{task_id}")
        {
            // A crashed attempt already promoted this candidate; recover
            // idempotently instead of reporting a stale incumbent.
            return Ok(());
        }
        anyhow::ensure!(
            draft.text_sha256 == expected_incumbent_hash,
            "stale_cas_conflict: incumbent text changed before promotion"
        );
        draft.text = candidate_text.to_string();
        draft.text_sha256 = candidate_hash.to_string();
        draft.truncated = false;
        draft.receipt = DraftReceiptBinding {
            generation_id: format!("host:{task_id}"),
            agent_id: "host".into(),
            normalized_output_sha256: normalized_output_sha256(candidate_text),
        };
        draft.accepted_text_sha256 = Some(candidate_hash.to_string());
        draft.extraction = None;
        draft.extraction_request = None;
        draft.extraction_attempts = 0;
        draft.extraction_failure = None;
        artifact::save_artifact(draft_artifact_path, &draft)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn promote_line_edit_candidate(
        &self,
        draft_artifact_path: &std::path::Path,
        expected_packet_hash: &str,
        expected_incumbent_hash: &str,
        candidate_text: &str,
        candidate_hash: &str,
        receipt: &DraftReceiptBinding,
    ) -> Result<()> {
        let draft = artifact::load_artifact(draft_artifact_path)?;
        let branch = self.repository.get_active_branch(&draft.project_id).await?;
        anyhow::ensure!(
            branch.id == draft.branch_id,
            "stale_cas_conflict: draft branch is no longer active"
        );
        anyhow::ensure!(
            editorial_sha256_hex(candidate_text.as_bytes()) == candidate_hash,
            "candidate hash mismatch before line-edit promotion"
        );
        let _lock = artifact::lock_artifact(draft_artifact_path)?;
        let mut draft = artifact::load_artifact(draft_artifact_path)?;
        anyhow::ensure!(
            draft.packet_hash == expected_packet_hash,
            "stale_cas_conflict: draft packet hash changed before line-edit promotion"
        );
        if draft.text_sha256 == candidate_hash
            && draft.receipt.generation_id == receipt.generation_id
        {
            return Ok(());
        }
        anyhow::ensure!(
            draft.text_sha256 == expected_incumbent_hash,
            "stale_cas_conflict: accepted text changed before line-edit promotion"
        );
        draft.text = candidate_text.to_string();
        draft.text_sha256 = candidate_hash.to_string();
        draft.truncated = false;
        draft.receipt = receipt.clone();
        draft.accepted_text_sha256 = Some(candidate_hash.to_string());
        draft.extraction = None;
        draft.extraction_request = None;
        draft.extraction_attempts = 0;
        draft.extraction_failure = None;
        artifact::save_artifact(draft_artifact_path, &draft)?;
        Ok(())
    }

    /// Fetch the model-call ids linked to a generation receipt. Used to bind
    /// agent-mode attribution to V0047 rows without duplicating the ledger.
    pub async fn editorial_generation_model_call_ids(&self, generation_id: &str) -> Vec<String> {
        let stored_id = generation_id.to_string();
        self.repository
            .pool()
            .read(move |conn| {
                let raw: Option<String> = conn
                    .query_row(
                        "SELECT model_call_ids FROM generation_receipt WHERE id = ?1",
                        rusqlite::params![stored_id],
                        |row| row.get(0),
                    )
                    .ok();
                Ok(raw
                    .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                    .unwrap_or_default())
            })
            .await
            .unwrap_or_default()
    }
}

pub(crate) fn admit_editorial_agent_draft_action(
    controller_path: &std::path::Path,
    scene_index: usize,
    now_millis: u64,
) -> Result<spindle_core::drafting::EditorialBudgetReservation> {
    update_editorial_controller(controller_path, |state| {
        let (attempt, stage, run_id, branch_id, scene_id, packet_hash) = {
            let scene = state
                .scenes
                .get(scene_index)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            (
                scene.attempt,
                scene.stage,
                state.run_id.clone(),
                state.branch_id.clone(),
                scene.scene_id.clone(),
                scene.packet_hash.clone(),
            )
        };
        let key = spindle_core::drafting::EditorialReservationKey {
            run_id,
            branch_id,
            scene_id,
            packet_hash,
            stage,
            attempt,
            action: EditorialActionKind::Draft,
        };
        let key_string = key.idempotency_key();
        if let Some(existing) = state.scenes[scene_index].ledger.find(&key_string) {
            if existing.is_dispatched() {
                anyhow::bail!(
                    "editorial_dispatch_in_progress: reservation {key_string} was already dispatched;                      a failed attempt must be reconciled before a new charged attempt"
                );
            }
            let claimed =
                state.scenes[scene_index]
                    .ledger
                    .claim_dispatch(&key_string, "draft", now_millis);
            anyhow::ensure!(
                claimed == Some(true),
                "editorial_dispatch_in_progress: reservation {key_string} was already dispatched"
            );
            return Ok(state.scenes[scene_index]
                .ledger
                .find(&key_string)
                .expect("claimed reservation")
                .clone());
        }
        let reservation = crate::drafting::editorial::admit_editorial_action_in_controller(
            state,
            scene_index,
            EditorialActionKind::Draft,
            attempt,
            now_millis,
            true,
        )
        .map_err(|denial| anyhow::anyhow!("{denial}"))?;
        let claimed = state.scenes[scene_index].ledger.claim_dispatch(
            &reservation.idempotency_key,
            "draft",
            now_millis,
        );
        anyhow::ensure!(
            claimed == Some(true),
            "editorial_dispatch_in_progress: reservation was already dispatched"
        );
        Ok(state.scenes[scene_index]
            .ledger
            .find(&reservation.idempotency_key)
            .expect("admitted reservation")
            .clone())
    })
}

/// Locked host-judge admission. The run-cap check and the persisted reservation
/// serialize with every other controller mutation.
pub(crate) fn admit_host_judge_action(
    controller_path: &std::path::Path,
    scene_index: usize,
    now_millis: u64,
) -> Result<spindle_core::drafting::EditorialBudgetReservation> {
    update_editorial_controller(controller_path, |state| {
        let attempt = state
            .scenes
            .get(scene_index)
            .map(|scene| scene.attempt)
            .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
        crate::drafting::editorial::admit_editorial_action_in_controller(
            state,
            scene_index,
            EditorialActionKind::Judge,
            attempt,
            now_millis,
            true,
        )
        .map_err(|denial| anyhow::anyhow!("{denial}"))
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn reconcile_editorial_agent_draft_action(
    controller_path: &std::path::Path,
    scene_index: usize,
    draft_artifact_path: &std::path::Path,
    reservation_key: &str,
    reserved: EditorialBudgetUsage,
    captured: Option<&PlainTextDraftArtifactV1>,
    tokenizer: Option<&dyn DraftTokenizer>,
    elapsed_millis: u64,
    now_millis: u64,
    packet_content_hash: String,
    model_call_ids: Vec<String>,
) -> Result<()> {
    update_editorial_controller(controller_path, |state| {
        let actual = captured.and_then(|captured| {
            known_draft_usage(
                captured,
                tokenizer,
                elapsed_millis,
                state.settings.pricing.as_ref(),
            )
        });
        {
            let scene = state
                .scenes
                .get_mut(scene_index)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            scene
                .ledger
                .reconcile(reservation_key, actual, model_call_ids.clone(), now_millis);
            if captured.is_none() {
                // A failed paid attempt is consumed: the next retry must use a
                // new attempt key and pass the cap check again.
                scene.attempt = scene.attempt.saturating_add(1);
            }
            if let Some(captured) = captured {
                scene.draft_artifact_path =
                    Some(draft_artifact_path.to_string_lossy().into_owned());
                scene.incumbent = Some(spindle_core::drafting::EditorialTextRefV1 {
                    text_sha256: captured.text_sha256.clone(),
                    generation_id: Some(captured.receipt.generation_id.clone()),
                    agent_id: Some(captured.receipt.agent_id.clone()),
                });
                scene.stage = EditorialStage::Drafted;
                scene.source_packet_sha256 = packet_content_hash.clone();
            }
        }
        record_model_attribution(
            state,
            scene_index,
            EditorialActionKind::Draft,
            TEMPLATE_PACKET_DRAFT.into(),
            TEMPLATE_VERSION_V1.into(),
            captured
                .map(|captured| captured.packet_hash.clone())
                .unwrap_or_default(),
            captured
                .map(|captured| captured.packet_hash.clone())
                .unwrap_or_default(),
            None,
            None,
            captured.map(|captured| captured.text_sha256.clone()),
            None,
            if captured.is_some() {
                "drafted".into()
            } else {
                "dispatch_failed".into()
            },
            elapsed_millis,
            actual,
            reserved,
            model_call_ids,
            now_millis,
        )?;
        Ok(())
    })
}

fn known_draft_usage(
    captured: &PlainTextDraftArtifactV1,
    tokenizer: Option<&dyn DraftTokenizer>,
    elapsed_millis: u64,
    pricing: Option<&spindle_core::drafting::EditorialPricingV1>,
) -> Option<EditorialBudgetUsage> {
    let input_tokens = captured
        .prepared_packet
        .as_ref()?
        .packet
        .assembly
        .as_ref()?
        .input_tokens;
    let output_tokens = tokenizer?.count_text(&captured.text).ok()?;
    let pricing = pricing?;
    Some(EditorialBudgetUsage {
        calls: 1,
        tokens: input_tokens.saturating_add(output_tokens),
        millis: elapsed_millis,
        usd_micros: pricing.cost_micros(input_tokens, output_tokens),
    })
}

fn elapsed_millis(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Durable revision-loop gate backed by the controller artifact. Each pass is
/// admitted in a locked controller update before dispatch, and reconciled in a
/// second locked update. Failure paths reconcile `None`, keeping the maximum
/// reservation charged.
pub(crate) struct ControllerRevisionBudgetGate<'a> {
    controller_path: &'a std::path::Path,
    scene_index: usize,
    now_millis: u64,
    /// Distinguishes one bounded chapter-edit operation from another on the same
    /// scene. The normal scene loop uses namespace 0.
    namespace: u32,
}

impl<'a> ControllerRevisionBudgetGate<'a> {
    pub(crate) fn new(
        controller_path: &'a std::path::Path,
        scene_index: usize,
        now_millis: u64,
    ) -> Self {
        Self::with_namespace(controller_path, scene_index, now_millis, 0)
    }

    pub(crate) fn with_namespace(
        controller_path: &'a std::path::Path,
        scene_index: usize,
        now_millis: u64,
        namespace: u32,
    ) -> Self {
        Self {
            controller_path,
            scene_index,
            now_millis,
            namespace,
        }
    }

    fn effective_attempt(&self, attempt: u32) -> u32 {
        self.namespace.saturating_mul(1_000).saturating_add(attempt)
    }
}

impl crate::drafting::revision::RevisionBudgetGate for ControllerRevisionBudgetGate<'_> {
    fn admit(
        &self,
        action: EditorialActionKind,
        attempt: u32,
        claim: &str,
    ) -> Result<crate::drafting::revision::RevisionBudgetAdmission, String> {
        update_editorial_controller(self.controller_path, |state| {
            let reservation = crate::drafting::editorial::admit_editorial_action_in_controller(
                state,
                self.scene_index,
                action,
                self.effective_attempt(attempt),
                self.now_millis,
                true,
            )
            .map_err(|denial| anyhow::anyhow!("{denial}"))?;
            let claimed = state.scenes[self.scene_index]
                .ledger
                .claim_dispatch(&reservation.idempotency_key, claim, self.now_millis);
            anyhow::ensure!(
                claimed == Some(true),
                "editorial_dispatch_in_progress: revision pass {} attempt {} claim {} was already dispatched",
                action.as_str(),
                attempt,
                claim
            );
            Ok(crate::drafting::revision::RevisionBudgetAdmission {
                admission_id: reservation.idempotency_key,
                action,
                reserved: reservation.reserved,
            })
        })
        .map_err(|error| format!("{error:#}"))
    }

    fn reconcile(
        &self,
        admission: &crate::drafting::revision::RevisionBudgetAdmission,
        usage: Option<crate::drafting::revision::RevisionPassUsage>,
        model_call_ids: Vec<String>,
    ) -> Result<(), String> {
        update_editorial_controller(self.controller_path, |state| {
            let actual = usage.and_then(|usage| {
                let pricing = state.settings.pricing.as_ref()?;
                let usd_micros = match (usage.input_tokens, usage.output_tokens) {
                    (Some(input), Some(output)) => pricing.cost_micros(input, output),
                    _ => {
                        let max_price = pricing
                            .input_usd_micros_per_million
                            .max(pricing.output_usd_micros_per_million);
                        u64::try_from(
                            (usage.total_tokens as u128).saturating_mul(max_price as u128)
                                / 1_000_000,
                        )
                        .unwrap_or(u64::MAX)
                    }
                };
                Some(EditorialBudgetUsage {
                    calls: usage.calls,
                    tokens: usage.total_tokens,
                    millis: usage.millis,
                    usd_micros,
                })
            });
            for scene in &mut state.scenes {
                scene.ledger.reconcile(
                    &admission.admission_id,
                    actual,
                    model_call_ids.clone(),
                    self.now_millis,
                );
            }
            Ok(())
        })
        .map_err(|error| format!("{error:#}"))
    }
}

#[allow(clippy::too_many_arguments)]
fn reconcile_chapter_critic(
    controller_path: &std::path::Path,
    scene_index: usize,
    key: &str,
    prompt_hash: &str,
    reserved: EditorialBudgetUsage,
    actual: Option<EditorialBudgetUsage>,
    model_call_ids: Vec<String>,
    now_millis: u64,
) -> Result<()> {
    update_editorial_controller(controller_path, |state| {
        {
            let scene = state
                .scenes
                .get_mut(scene_index)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            scene
                .ledger
                .reconcile(key, actual, model_call_ids.clone(), now_millis);
        }
        record_model_attribution(
            state,
            scene_index,
            EditorialActionKind::ChapterCritique,
            TEMPLATE_CHAPTER_CRITIC.into(),
            TEMPLATE_VERSION_V1.into(),
            prompt_hash.to_string(),
            prompt_hash.to_string(),
            None,
            None,
            None,
            None,
            if actual.is_some() {
                "chapter_critiqued".into()
            } else {
                "chapter_critic_failed".into()
            },
            actual.map(|usage| usage.millis).unwrap_or(0),
            actual,
            reserved,
            model_call_ids,
            now_millis,
        )?;
        Ok(())
    })
}

fn sync_scene_after_chapter_edit(
    controller_path: &std::path::Path,
    scene_index: usize,
    loop_artifact_path: &std::path::Path,
    artifact: &RevisionLoopArtifactV1,
    now_millis: u64,
) -> Result<()> {
    update_editorial_controller(controller_path, |state| {
        {
            let scene = state
                .scenes
                .get_mut(scene_index)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            scene.loop_artifact_path = Some(loop_artifact_path.to_string_lossy().into_owned());
            scene.stage = artifact.current_stage;
            scene.incumbent = Some(spindle_core::drafting::EditorialTextRefV1 {
                text_sha256: artifact.accepted_text_sha256.clone(),
                generation_id: Some(artifact.accepted_receipt.generation_id.clone()),
                agent_id: Some(artifact.accepted_receipt.agent_id.clone()),
            });
            if let Some(round) = artifact.rounds.last()
                && let Some(judgment) = &round.double_order_judgment
            {
                scene.judge = Some(spindle_core::drafting::EditorialJudgeSummaryV1 {
                    verdict: judgment.verdict.as_str().into(),
                    accepted_revision: judgment.accepted,
                    order_agreement: judgment.position_bias.is_none(),
                    retention_reason: round
                        .round_stop_reason
                        .map(|reason| reason.as_str().to_string()),
                });
            }
        }
        for transition in &artifact.transitions {
            let action = action_for_stage(transition.key.stage);
            let template = action.prompt_template_id().unwrap_or(TEMPLATE_PACKET_DRAFT);
            record_model_attribution(
                state,
                scene_index,
                action,
                template.into(),
                TEMPLATE_VERSION_V1.into(),
                transition.output_hash.clone(),
                transition.output_hash.clone(),
                None,
                Some(artifact.initial_text_sha256.clone()),
                Some(artifact.accepted_text_sha256.clone()),
                None,
                transition.key.stage.as_str().into(),
                0,
                Some(EditorialBudgetUsage {
                    calls: u32::try_from(transition.model_call_ids.len()).unwrap_or(0),
                    tokens: transition.tokens_used,
                    millis: 0,
                    usd_micros: 0,
                }),
                EditorialBudgetUsage::default(),
                transition.model_call_ids.clone(),
                now_millis,
            )?;
        }
        Ok(())
    })
}

fn line_edit_candidate_path(draft_artifact_path: &std::path::Path) -> std::path::PathBuf {
    let mut path = draft_artifact_path.as_os_str().to_os_string();
    path.push(".line-edit-candidate.json");
    std::path::PathBuf::from(path)
}

fn mark_line_edit_attempted(
    controller_path: &std::path::Path,
    scene_index: usize,
    _now_millis: u64,
) -> Result<()> {
    update_editorial_controller(controller_path, |state| {
        let scene = state
            .scenes
            .get_mut(scene_index)
            .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
        scene.line_edit_attempted = true;
        Ok(())
    })
}

#[allow(clippy::type_complexity)]
type LineEditJudgeSettlement = (
    String,
    EditorialBudgetUsage,
    Option<EditorialBudgetUsage>,
    Vec<String>,
    String,
);

#[allow(clippy::too_many_arguments)]
fn finalize_line_edit(
    controller_path: &std::path::Path,
    scene_index: usize,
    line_key: &str,
    line_reserved: EditorialBudgetUsage,
    line_actual: Option<EditorialBudgetUsage>,
    line_call_ids: Vec<String>,
    _candidate: Option<(String, String)>,
    now_millis: u64,
    outcome: LineEditOutcomeV1,
    incumbent_hash: String,
    judge: Option<LineEditJudgeSettlement>,
) -> Result<()> {
    update_editorial_controller(controller_path, |state| {
        {
            let scene = state
                .scenes
                .get_mut(scene_index)
                .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
            scene
                .ledger
                .reconcile(line_key, line_actual, line_call_ids.clone(), now_millis);
            scene.line_edit_attempted = true;
            if let LineEditOutcomeV1::Accepted {
                text_sha256,
                generation_id,
            } = &outcome
            {
                scene.incumbent = Some(spindle_core::drafting::EditorialTextRefV1 {
                    text_sha256: text_sha256.clone(),
                    generation_id: Some(generation_id.clone()),
                    agent_id: Some("agent".into()),
                });
            }
        }
        record_model_attribution(
            state,
            scene_index,
            EditorialActionKind::LineEdit,
            TEMPLATE_LINE_EDIT.into(),
            TEMPLATE_VERSION_V1.into(),
            String::new(),
            String::new(),
            None,
            Some(incumbent_hash.clone()),
            None,
            None,
            line_edit_outcome_label(&outcome),
            0,
            line_actual,
            line_reserved,
            line_call_ids,
            now_millis,
        )?;
        if let Some((judge_key, judge_reserved, judge_actual, judge_calls, verdict)) = judge {
            {
                let scene = state
                    .scenes
                    .get_mut(scene_index)
                    .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
                scene
                    .ledger
                    .reconcile(&judge_key, judge_actual, judge_calls.clone(), now_millis);
            }
            record_model_attribution(
                state,
                scene_index,
                EditorialActionKind::Judge,
                TEMPLATE_JUDGE.into(),
                TEMPLATE_VERSION_V1.into(),
                String::new(),
                String::new(),
                None,
                Some(incumbent_hash),
                None,
                None,
                verdict,
                0,
                judge_actual,
                judge_reserved,
                judge_calls,
                now_millis,
            )?;
        }
        Ok(())
    })
}

fn line_edit_outcome_label(outcome: &LineEditOutcomeV1) -> String {
    match outcome {
        LineEditOutcomeV1::Accepted { .. } => "accepted".into(),
        LineEditOutcomeV1::Retained { reason, .. } => format!("retained:{reason}"),
        LineEditOutcomeV1::Skipped { reason } => format!("skipped:{reason}"),
    }
}

fn candidate_artifact_path(draft_artifact_path: &std::path::Path) -> std::path::PathBuf {
    let mut path = draft_artifact_path.as_os_str().to_os_string();
    path.push(".candidate.json");
    std::path::PathBuf::from(path)
}

fn host_receipt(task_id: &str, text: &str) -> spindle_core::drafting::HostCompletionReceiptV1 {
    spindle_core::drafting::HostCompletionReceiptV1 {
        task_id: task_id.to_string(),
        agent_id: "host".into(),
        text_sha256: editorial_sha256_hex(text.as_bytes()),
        normalized_output_sha256: normalized_output_sha256(text),
        usage_known: false,
    }
}

fn judge_reservation_key(
    run_id: &str,
    branch_id: &str,
    scene: &spindle_core::drafting::EditorialSceneStateV1,
) -> spindle_core::drafting::EditorialReservationKey {
    spindle_core::drafting::EditorialReservationKey {
        run_id: run_id.to_string(),
        branch_id: branch_id.to_string(),
        scene_id: scene.scene_id.clone(),
        packet_hash: scene.packet_hash.clone(),
        stage: scene.stage,
        attempt: scene.attempt,
        action: EditorialActionKind::Judge,
    }
}

fn action_for_stage(stage: EditorialStage) -> EditorialActionKind {
    match stage {
        EditorialStage::PacketReady | EditorialStage::Drafted => EditorialActionKind::Draft,
        EditorialStage::DeterministicChecked => EditorialActionKind::DeterministicCheck,
        EditorialStage::Critiqued => EditorialActionKind::Critique,
        EditorialStage::CandidateReady | EditorialStage::CandidateChecked => {
            EditorialActionKind::Revise
        }
        EditorialStage::PairwiseJudged => EditorialActionKind::Judge,
        EditorialStage::AcceptedOriginal | EditorialStage::AcceptedRevision => {
            EditorialActionKind::Judge
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn record_submission_attribution(
    state: &mut EditorialControllerStateV1,
    scene_index: usize,
    action: EditorialActionKind,
    template: &str,
    candidate_hash: &str,
    comparison: Option<&spindle_core::drafting::CandidateCheckComparison>,
    submission: &HostEditorialSubmissionV1,
    now_millis: u64,
) -> anyhow::Result<()> {
    let reserved = subscription_reserved(state, scene_index, action);
    let actual = submission.usage;
    let outcome = if comparison.is_some_and(|value| value.disqualified) {
        "candidate_disqualified"
    } else {
        "host_submitted"
    };
    record_model_attribution(
        state,
        scene_index,
        action,
        template.into(),
        TEMPLATE_VERSION_V1.into(),
        candidate_hash.into(),
        candidate_hash.into(),
        None,
        None,
        Some(candidate_hash.into()),
        None,
        outcome.into(),
        0,
        actual,
        reserved,
        Vec::new(),
        now_millis,
    )?;
    Ok(())
}

fn subscription_reserved(
    state: &EditorialControllerStateV1,
    scene_index: usize,
    action: EditorialActionKind,
) -> EditorialBudgetUsage {
    state
        .scenes
        .get(scene_index)
        .and_then(|scene| {
            scene
                .ledger
                .scene_reservations
                .iter()
                .rev()
                .find(|reservation| reservation.key.action == action)
                .map(|reservation| reservation.reserved)
        })
        .unwrap_or_default()
}
