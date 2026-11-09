//! Editorial controller service methods: hybrid host submission/judging and
//! agent-mode parity over the same packet/revision artifacts.

use super::*;
use crate::drafting::{
    DraftTokenizer, PacketBudget, artifact,
    editorial::{
        apply_host_editorial_submission, editorial_sha256_hex, load_editorial_controller,
        normalized_output_sha256, prepare_host_editorial_task, record_model_attribution,
        update_editorial_controller,
    },
    revision::{
        JudgeBudget, RevisionBudget, RevisionLoopOptions, resolve_double_order_judgment,
        select_initial_judge_order,
    },
};
use spindle_core::drafting::{
    DeblindedArm, EditorialActionKind, EditorialBudgetUsage, EditorialControllerStateV1,
    EditorialPolicy, EditorialStage, HostEditorialSubmissionV1, HostEditorialTaskV1,
    HostSubmissionOutcomeV1, ManualJudgmentReason, TEMPLATE_JUDGE, TEMPLATE_PACKET_DRAFT,
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
        if !availability.verified_tokenizer {
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

    async fn promote_host_candidate(
        &self,
        draft_artifact_path: &std::path::Path,
        task_id: &str,
        candidate_text: &str,
        candidate_hash: &str,
    ) -> Result<()> {
        let mut draft = artifact::load_artifact(draft_artifact_path)?;
        let branch = self.repository.get_active_branch(&draft.project_id).await?;
        anyhow::ensure!(
            branch.id == draft.branch_id,
            "stale_cas_conflict: draft branch is no longer active"
        );
        anyhow::ensure!(
            editorial_sha256_hex(candidate_text.as_bytes()) == candidate_hash,
            "candidate hash mismatch before promotion"
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
            let claimed = state.scenes[scene_index]
                .ledger
                .claim_dispatch(&key_string, now_millis);
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
        let claimed = state.scenes[scene_index]
            .ledger
            .claim_dispatch(&reservation.idempotency_key, now_millis);
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
}

impl<'a> ControllerRevisionBudgetGate<'a> {
    pub(crate) fn new(
        controller_path: &'a std::path::Path,
        scene_index: usize,
        now_millis: u64,
    ) -> Self {
        Self {
            controller_path,
            scene_index,
            now_millis,
        }
    }
}

impl crate::drafting::revision::RevisionBudgetGate for ControllerRevisionBudgetGate<'_> {
    fn admit(
        &self,
        action: EditorialActionKind,
        attempt: u32,
    ) -> Result<crate::drafting::revision::RevisionBudgetAdmission, String> {
        update_editorial_controller(self.controller_path, |state| {
            let reservation = crate::drafting::editorial::admit_editorial_action_in_controller(
                state,
                self.scene_index,
                action,
                attempt,
                self.now_millis,
                true,
            )
            .map_err(|denial| anyhow::anyhow!("{denial}"))?;
            let claimed = state.scenes[self.scene_index]
                .ledger
                .claim_dispatch(&reservation.idempotency_key, self.now_millis);
            anyhow::ensure!(
                claimed == Some(true),
                "editorial_dispatch_in_progress: revision pass {} attempt {} was already dispatched",
                action.as_str(),
                attempt
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
