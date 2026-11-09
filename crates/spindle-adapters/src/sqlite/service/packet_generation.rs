use super::*;
use crate::drafting::{
    DraftTokenizer, PacketBudget, artifact,
    critique::{self, CRITIC_RUBRIC_VERSION, CriticBudget},
    revision::{
        self, BoundaryCrashPoint, JudgeBudget, RevisionBudget, RevisionBudgetAdmission,
        RevisionBudgetGate, RevisionLoopOptions, RevisionPassUsage, compare_candidate_checks,
        compute_edit_span_summary, evaluate_deterministic_checks, resolve_double_order_judgment,
        select_initial_judge_order,
    },
};
use spindle_core::drafting::{
    CasVersionExpectation, CriticFindingInput, DeblindedArm, DraftingPacketV1, EditorialActionKind,
    EditorialPolicy, EditorialStage, HARD_MAX_REVISION_ROUNDS, HARD_MAX_SCENE_CALLS,
    HARD_MAX_SCENE_TOKENS, PendingRevisionPromotion, REVISION_LOOP_ARTIFACT_SCHEMA_VERSION,
    RevisionLoopArtifactV1, RevisionRetentionReason, RevisionRoundRecord,
    SHADOW_CRITIQUE_ARTIFACT_SCHEMA_VERSION, ShadowCritiqueArtifactV1, ShadowCritiqueOutcome,
    StageTransitionKey, StageTransitionRecord,
    extraction::{DraftReceiptBinding, PlainTextDraftArtifactV1},
};

impl SqliteSpindleService {
    /// Internal opt-in seam; authoring runs retain the legacy package workflow.
    /// Caller supplies a private local artifact path and a verified tokenizer.
    pub async fn draft_packet_to_artifact(
        &self,
        policy: EditorialPolicy,
        packet: DraftingPacketV1,
        budget: &PacketBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        path: &std::path::Path,
    ) -> Result<PlainTextDraftArtifactV1> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "packet drafting requires critique_v1 policy"
        );
        let branch = self
            .repository
            .get_active_branch(&packet.project_id)
            .await?;
        anyhow::ensure!(
            branch.id == packet.branch_id,
            "packet branch is no longer active"
        );
        let mut lock = artifact::lock_artifact(path)?;
        if path.exists() {
            let captured = artifact::load_artifact(path)?;
            let prepared = captured.prepared_packet.as_ref().ok_or_else(|| {
                anyhow::anyhow!("captured packet/report missing; manual recovery required")
            })?;
            let recorded_seed = prepared
                .packet
                .assembly
                .as_ref()
                .and_then(|assembly| assembly.generation_seed);
            anyhow::ensure!(
                recorded_seed == budget.generation_seed,
                "artifact generation seed mismatch"
            );
            let source_hash = generation_sha256_hex(&serde_json::to_vec(&packet)?);
            anyhow::ensure!(
                captured.source_packet_sha256 == source_hash
                    && captured.project_id == packet.project_id
                    && captured.branch_id == packet.branch_id,
                "artifact belongs to a different packet snapshot"
            );
            let receipt = self
                .verified_revisable_draft_receipt(Some(&captured.receipt.generation_id))
                .await?
                .ok_or_else(|| anyhow::anyhow!("draft receipt missing"))?;
            anyhow::ensure!(
                receipt.output_sha256 == captured.receipt.normalized_output_sha256
                    && receipt.agent_id == captured.receipt.agent_id
                    && receipt.rating.as_deref() == Some(captured.rating.as_str()),
                "artifact receipt binding mismatch"
            );
            return Ok(captured);
        }
        let (prepared, response) = self
            .repository
            .model_router()
            .draft_packet_before_dispatch(policy, packet.clone(), budget, tokenizer, |prepared| {
                lock.retain_for_recovery(&prepared.messages_hash)
            })
            .await?;
        let receipt = self
            .register_generation_receipt_with_calls(
                "draft",
                Some(packet.scene_card.rating.as_str()),
                &response.model_name,
                &response.output,
                response.call_id.into_iter().collect(),
            )
            .await?;
        let mut captured = artifact::capture_plain_text(
            &packet,
            &prepared.messages_hash,
            response.output,
            response.truncated,
            DraftReceiptBinding {
                generation_id: receipt.id,
                agent_id: receipt.agent_id,
                normalized_output_sha256: receipt.output_sha256,
            },
        )?;
        captured.prepared_packet = Some(prepared);
        artifact::save_artifact(path, &captured)?;
        lock.captured();
        Ok(captured)
    }

    /// Execute a shadow-only scene critic pass against a persisted
    /// `PlainTextDraftArtifactV1` and save the resulting `ShadowCritiqueArtifactV1`
    /// without mutating the draft artifact, its prose, or any scene/canon tables.
    #[allow(clippy::too_many_arguments)]
    pub async fn critique_artifact_shadow(
        &self,
        policy: EditorialPolicy,
        draft_artifact_path: &std::path::Path,
        critique_artifact_path: &std::path::Path,
        findings: Vec<CriticFindingInput>,
        budget: &CriticBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        max_calls: u32,
    ) -> Result<ShadowCritiqueArtifactV1> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "shadow critique requires critique_v1 policy"
        );
        let draft_bytes_before = std::fs::read(draft_artifact_path)?;
        let draft = artifact::load_artifact(draft_artifact_path)?;
        anyhow::ensure!(
            !draft.truncated,
            "incomplete/truncated draft artifact cannot be critiqued"
        );
        let prepared = draft.prepared_packet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("draft artifact missing prepared_packet; cannot run shadow critique")
        })?;
        let branch = self.repository.get_active_branch(&draft.project_id).await?;
        anyhow::ensure!(
            branch.id == draft.branch_id,
            "draft artifact branch is no longer active"
        );

        let expected_ingested = critique::ingest_and_dedup_findings(
            findings.clone(),
            &draft.text,
            &draft.text_sha256,
            draft.scene_id.as_deref(),
        )?;
        if critique_artifact_path.exists() {
            let existing = critique::load_shadow_critique_artifact(critique_artifact_path)?;
            anyhow::ensure!(
                existing.draft_artifact_id == draft.artifact_id
                    && existing.base_text_hash == draft.text_sha256
                    && existing.packet_hash == draft.packet_hash
                    && existing.project_id == draft.project_id
                    && existing.branch_id == draft.branch_id
                    && existing.ingested_findings == expected_ingested.0,
                "existing shadow critique artifact belongs to different draft or findings snapshot"
            );
            let draft_bytes_after = std::fs::read(draft_artifact_path)?;
            anyhow::ensure!(
                draft_bytes_after == draft_bytes_before,
                "shadow critique violated draft artifact immutability"
            );
            return Ok(existing);
        }

        let run_result = self
            .repository
            .model_router()
            .critique_draft_shadow(
                policy,
                &prepared.packet,
                &draft.packet_hash,
                &draft.text,
                &draft.text_sha256,
                findings,
                budget,
                tokenizer,
                max_calls,
            )
            .await?;

        let shadow_artifact = ShadowCritiqueArtifactV1 {
            schema_version: SHADOW_CRITIQUE_ARTIFACT_SCHEMA_VERSION.into(),
            artifact_id: format!("shadow_critique:{}", draft.artifact_id),
            draft_artifact_id: draft.artifact_id.clone(),
            project_id: draft.project_id.clone(),
            branch_id: draft.branch_id.clone(),
            scene_id: draft.scene_id.clone(),
            base_text_hash: draft.text_sha256.clone(),
            packet_hash: draft.packet_hash.clone(),
            rubric_version: CRITIC_RUBRIC_VERSION.into(),
            rating: draft.rating.clone(),
            ingested_findings: run_result.ingested_findings,
            deduplicated_finding_ids: run_result.deduplicated_finding_ids,
            prompt_hashes: run_result.prompt_hashes,
            model_call_ids: run_result.model_call_ids,
            outcome: run_result.outcome,
        };
        critique::save_shadow_critique_artifact(critique_artifact_path, &shadow_artifact)?;

        // Enforce strict shadow-mode immutability: the draft artifact on disk must be byte-for-byte unchanged.
        let draft_bytes_after = std::fs::read(draft_artifact_path)?;
        anyhow::ensure!(
            draft_bytes_after == draft_bytes_before,
            "shadow critique violated draft artifact immutability"
        );
        Ok(shadow_artifact)
    }

    /// Execute the bounded scene critique, prior-draft revision, deterministic
    /// candidate check, and swapped-order blinded pairwise judge loop over a
    /// persisted `PlainTextDraftArtifactV1`.
    ///
    /// Persists durable stage transitions and loop state to `loop_artifact_path`
    /// at every stage boundary (`Drafted`, `DeterministicChecked`, `Critiqued`,
    /// `CandidateReady`, `CandidateChecked`, `PairwiseJudged`, `AcceptedOriginal | AcceptedRevision`)
    /// so a crash at any boundary preserves the incumbent artifact and resumes
    /// idempotently without replaying completed paid calls. Before promoting any
    /// accepted revision, verifies Compare-And-Swap (CAS) on the active branch,
    /// canon snapshot, plan revision, source versions, and incumbent artifact hash.
    #[allow(clippy::too_many_arguments)]
    pub async fn revise_artifact_loop(
        &self,
        policy: EditorialPolicy,
        draft_artifact_path: &std::path::Path,
        loop_artifact_path: &std::path::Path,
        external_findings: Vec<CriticFindingInput>,
        critic_budget: &CriticBudget,
        revision_budget: &RevisionBudget,
        judge_budget: &JudgeBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        options: RevisionLoopOptions,
    ) -> Result<RevisionLoopArtifactV1> {
        self.revise_artifact_loop_with_gate(
            policy,
            draft_artifact_path,
            loop_artifact_path,
            external_findings,
            critic_budget,
            revision_budget,
            judge_budget,
            tokenizer,
            options,
            None,
        )
        .await
    }

    /// Gate-aware revision loop used by the editorial controller. Each paid
    /// pass is admitted before dispatch and reconciled afterwards; a failed
    /// pass reconciles with `None` so its maximum reservation stays charged.
    #[allow(clippy::too_many_arguments)]
    pub async fn revise_artifact_loop_with_gate(
        &self,
        policy: EditorialPolicy,
        draft_artifact_path: &std::path::Path,
        loop_artifact_path: &std::path::Path,
        external_findings: Vec<CriticFindingInput>,
        critic_budget: &CriticBudget,
        revision_budget: &RevisionBudget,
        judge_budget: &JudgeBudget,
        tokenizer: Option<&dyn DraftTokenizer>,
        options: RevisionLoopOptions,
        gate: Option<&dyn RevisionBudgetGate>,
    ) -> Result<RevisionLoopArtifactV1> {
        anyhow::ensure!(
            policy == EditorialPolicy::CritiqueV1,
            "revision loop requires critique_v1 policy"
        );
        anyhow::ensure!(
            (1..=HARD_MAX_REVISION_ROUNDS).contains(&options.max_rounds),
            "max_rounds must be 1..={HARD_MAX_REVISION_ROUNDS}"
        );
        anyhow::ensure!(
            options.max_scene_calls <= HARD_MAX_SCENE_CALLS
                && options.max_scene_tokens <= HARD_MAX_SCENE_TOKENS,
            "scene call/token limits exceed hard caps"
        );

        let draft = artifact::load_artifact(draft_artifact_path)?;
        anyhow::ensure!(
            !draft.truncated,
            "incomplete/truncated draft artifact cannot enter revision loop"
        );
        let prepared = draft.prepared_packet.as_ref().ok_or_else(|| {
            anyhow::anyhow!("draft artifact missing prepared_packet; cannot run revision loop")
        })?;
        let packet = &prepared.packet;
        let run_id = packet
            .run_id
            .clone()
            .unwrap_or_else(|| format!("run:{}", draft.artifact_id));

        let record_transition = |loop_art: &mut RevisionLoopArtifactV1,
                                 base_hash: &str,
                                 stage: EditorialStage,
                                 attempt: u32,
                                 output_hash: String,
                                 call_ids: Vec<String>,
                                 tokens_used: usize| {
            let key = StageTransitionKey {
                run_id: loop_art.run_id.clone(),
                branch_id: loop_art.branch_id.clone(),
                scene_id: loop_art.scene_id.clone(),
                base_text_hash: base_hash.into(),
                packet_hash: loop_art.packet_hash.clone(),
                stage,
                attempt,
            };
            let idempotency_key = key.idempotency_key();
            if !loop_art
                .transitions
                .iter()
                .any(|t| t.idempotency_key == idempotency_key)
            {
                loop_art.transitions.push(StageTransitionRecord {
                    key,
                    idempotency_key,
                    output_hash,
                    model_call_ids: call_ids,
                    tokens_used,
                });
            }
            loop_art.current_stage = stage;
        };

        let mut loop_artifact = if loop_artifact_path.exists() {
            let existing = revision::load_revision_loop_artifact(loop_artifact_path)?;
            anyhow::ensure!(
                existing.initial_draft_artifact_id == draft.artifact_id
                    && existing.packet_hash == draft.packet_hash
                    && existing.project_id == draft.project_id
                    && existing.branch_id == draft.branch_id,
                "existing revision loop artifact belongs to a different draft or packet"
            );
            if existing.current_stage.is_terminal() {
                return Ok(existing);
            }
            existing
        } else {
            let cas_expectation = self.resolve_live_cas_expectation(&draft).await?;
            anyhow::ensure!(
                cas_expectation.branch_id == draft.branch_id,
                "draft artifact branch is no longer active"
            );
            let mut fresh = RevisionLoopArtifactV1 {
                schema_version: REVISION_LOOP_ARTIFACT_SCHEMA_VERSION.into(),
                artifact_id: format!("revision_loop:{}", draft.artifact_id),
                run_id,
                project_id: draft.project_id.clone(),
                branch_id: draft.branch_id.clone(),
                scene_id: draft.scene_id.clone(),
                packet_hash: draft.packet_hash.clone(),
                source_packet_sha256: draft.source_packet_sha256.clone(),
                rating: draft.rating.clone(),
                max_rounds: options.max_rounds,
                max_scene_calls: options.max_scene_calls,
                max_scene_tokens: options.max_scene_tokens,
                total_calls_used: 0,
                total_tokens_used: 0,
                cas_expectation,
                initial_draft_artifact_id: draft.artifact_id.clone(),
                initial_text: draft.text.clone(),
                initial_text_sha256: draft.text_sha256.clone(),
                initial_receipt: draft.receipt.clone(),
                current_stage: EditorialStage::PacketReady,
                seen_finding_fingerprints: Vec::new(),
                transitions: Vec::new(),
                rounds: Vec::new(),
                accepted_text: draft.text.clone(),
                accepted_text_sha256: draft.text_sha256.clone(),
                accepted_receipt: draft.receipt.clone(),
                revision_promoted: false,
                retention_reason: None,
                parked_for_review: false,
                broad_edit_flagged_for_review: false,
                requires_author_decision: Vec::new(),
                pending_promotion: None,
            };
            record_transition(
                &mut fresh,
                &draft.text_sha256,
                EditorialStage::PacketReady,
                0,
                draft.packet_hash.clone(),
                Vec::new(),
                0,
            );
            record_transition(
                &mut fresh,
                &draft.text_sha256,
                EditorialStage::Drafted,
                0,
                draft.text_sha256.clone(),
                vec![draft.receipt.generation_id.clone()],
                0,
            );
            revision::save_revision_loop_artifact(loop_artifact_path, &fresh)?;
            if options.crash_after == Some(BoundaryCrashPoint::Stage(EditorialStage::Drafted, 1)) {
                anyhow::bail!("simulated_crash_at_boundary:drafted:1");
            }
            fresh
        };

        let mut working_text = loop_artifact.initial_text.clone();
        let mut working_hash = loop_artifact.initial_text_sha256.clone();
        let mut working_receipt = loop_artifact.initial_receipt.clone();
        let mut any_round_accepted = false;

        if let Some(pending) = &loop_artifact.pending_promotion {
            working_text = pending.candidate_text.clone();
            working_hash = pending.candidate_text_sha256.clone();
            working_receipt = pending.candidate_receipt.clone();
            any_round_accepted = true;
        }

        for round_num in 1..=loop_artifact.max_rounds {
            if loop_artifact.pending_promotion.is_some() {
                break;
            }
            // Ensure a RevisionRoundRecord exists for this round.
            if loop_artifact.rounds.len() < round_num as usize {
                loop_artifact.rounds.push(RevisionRoundRecord {
                    round: round_num,
                    incumbent_text_sha256: working_hash.clone(),
                    incumbent_deterministic_report: None,
                    critique_outcome: None,
                    selected_directives: Vec::new(),
                    strengths_to_preserve: Vec::new(),
                    requires_author_decision: Vec::new(),
                    candidate_text: None,
                    candidate_text_sha256: None,
                    candidate_truncated: None,
                    candidate_receipt: None,
                    edit_span_summary: None,
                    candidate_check_comparison: None,
                    judge_pass_1: None,
                    judge_pass_2: None,
                    double_order_judgment: None,
                    round_accepted: false,
                    round_stop_reason: None,
                    prompt_hashes: Vec::new(),
                    model_call_ids: Vec::new(),
                    tokens_used: 0,
                });
            }

            // If this round was already completed before a crash in a later round, replay its state transition in memory.
            {
                let r = &loop_artifact.rounds[(round_num - 1) as usize];
                if r.round_accepted {
                    if let (Some(ct), Some(ch), Some(cr)) = (
                        r.candidate_text.clone(),
                        r.candidate_text_sha256.clone(),
                        r.candidate_receipt.clone(),
                    ) {
                        working_text = ct;
                        working_hash = ch;
                        working_receipt = cr;
                        any_round_accepted = true;
                        if round_num < loop_artifact.max_rounds
                            && let Some(cmp) = &r.candidate_check_comparison
                        {
                            let remaining_issues =
                                !cmp.candidate_report.ingested_findings.is_empty();
                            let new_fp = !loop_artifact
                                .seen_finding_fingerprints
                                .contains(&cmp.candidate_report.normalized_finding_fingerprint);
                            let budget_allows_round_2 = loop_artifact
                                .total_calls_used
                                .saturating_add(4)
                                <= loop_artifact.max_scene_calls
                                && loop_artifact.total_tokens_used < loop_artifact.max_scene_tokens;
                            if !remaining_issues {
                                break;
                            }
                            if !new_fp {
                                loop_artifact.retention_reason =
                                    Some(RevisionRetentionReason::RepeatedFindingFingerprint);
                                break;
                            }
                            if !budget_allows_round_2 {
                                loop_artifact.retention_reason =
                                    Some(RevisionRetentionReason::BudgetExhausted);
                                break;
                            }
                        }
                        continue;
                    }
                } else if let Some(stop_reason) = r.round_stop_reason {
                    loop_artifact.retention_reason = Some(stop_reason);
                    break;
                }
            }

            // Stage 1: DeterministicChecked
            let inc_report = {
                let r = &loop_artifact.rounds[(round_num - 1) as usize];
                if let Some(existing_rep) = &r.incumbent_deterministic_report {
                    existing_rep.clone()
                } else {
                    let projected_findings = project_external_findings_to_text(
                        &working_text,
                        &working_hash,
                        &external_findings,
                    );
                    let rep = evaluate_deterministic_checks(
                        packet,
                        &working_text,
                        false,
                        projected_findings,
                    )?;
                    let fp = rep.normalized_finding_fingerprint.clone();
                    loop_artifact.rounds[(round_num - 1) as usize].incumbent_deterministic_report =
                        Some(rep.clone());
                    record_transition(
                        &mut loop_artifact,
                        &working_hash,
                        EditorialStage::DeterministicChecked,
                        round_num,
                        fp,
                        Vec::new(),
                        0,
                    );
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    rep
                }
            };
            if options.crash_after
                == Some(BoundaryCrashPoint::Stage(
                    EditorialStage::DeterministicChecked,
                    round_num,
                ))
            {
                anyhow::bail!("simulated_crash_at_boundary:deterministic_checked:{round_num}");
            }

            // Check repeated normalized finding fingerprint convergence stop (in Round 2+).
            let inc_fp = inc_report.normalized_finding_fingerprint.clone();
            if round_num > 1 && loop_artifact.seen_finding_fingerprints.contains(&inc_fp) {
                loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                    Some(RevisionRetentionReason::RepeatedFindingFingerprint);
                loop_artifact.retention_reason =
                    Some(RevisionRetentionReason::RepeatedFindingFingerprint);
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                break;
            }
            if !loop_artifact.seen_finding_fingerprints.contains(&inc_fp) {
                loop_artifact.seen_finding_fingerprints.push(inc_fp);
            }

            // Stage 2: Critiqued
            if loop_artifact.rounds[(round_num - 1) as usize]
                .critique_outcome
                .is_none()
            {
                let remaining_calls = loop_artifact
                    .max_scene_calls
                    .saturating_sub(loop_artifact.total_calls_used);
                if remaining_calls == 0
                    || loop_artifact.total_tokens_used >= loop_artifact.max_scene_tokens
                {
                    loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                        Some(RevisionRetentionReason::BudgetExhausted);
                    loop_artifact.retention_reason = Some(RevisionRetentionReason::BudgetExhausted);
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    break;
                }
                let critic_max_calls = options
                    .max_critic_calls_per_round
                    .min(remaining_calls)
                    .max(1);
                let combined_findings: Vec<CriticFindingInput> = inc_report
                    .ingested_findings
                    .iter()
                    .map(|f| CriticFindingInput {
                        finding_id: f.canonical_id.clone(),
                        source: f
                            .sources
                            .first()
                            .copied()
                            .unwrap_or(spindle_core::drafting::FeedbackSourceKind::Continuity),
                        check_type: f.check_type.clone(),
                        dimension: f.dimension,
                        severity: f.severity,
                        message: f.message.clone(),
                        anchor: f.anchor.clone(),
                        suggested_fix: f.suggested_fix.clone(),
                    })
                    .collect();
                let critic_admission = gate
                    .map(|gate| gate.admit(EditorialActionKind::Critique, round_num, "critic"))
                    .transpose()
                    .map_err(|error| anyhow::anyhow!("editorial_budget_gate: {error}"))?;
                let critic_start = std::time::Instant::now();
                let critic_result = self
                    .repository
                    .model_router()
                    .critique_draft_shadow(
                        policy,
                        packet,
                        &draft.packet_hash,
                        &working_text,
                        &working_hash,
                        combined_findings,
                        critic_budget,
                        tokenizer,
                        critic_max_calls,
                    )
                    .await;
                let critic_res = match critic_result {
                    Ok(result) => result,
                    Err(error) => {
                        if let (Some(gate), Some(admission)) = (gate, &critic_admission) {
                            let _ = gate.reconcile(admission, None, Vec::new());
                        }
                        return Err(error);
                    }
                };
                let calls_spent = u32::try_from(critic_res.prompt_hashes.len()).unwrap_or(1);
                let critic_tokens = critic_res.tokens_used;
                if let (Some(gate), Some(admission)) = (gate, &critic_admission) {
                    let usage = RevisionPassUsage {
                        calls: calls_spent,
                        input_tokens: None,
                        output_tokens: None,
                        total_tokens: critic_tokens,
                        millis: elapsed_millis(critic_start),
                    };
                    gate.reconcile(admission, Some(usage), critic_res.model_call_ids.clone())
                        .map_err(|error| anyhow::anyhow!("editorial_budget_gate: {error}"))?;
                }
                loop_artifact.total_calls_used =
                    loop_artifact.total_calls_used.saturating_add(calls_spent);
                loop_artifact.total_tokens_used = loop_artifact
                    .total_tokens_used
                    .saturating_add(critic_tokens);

                let (directives, strengths, author_decisions, out_hash) = match &critic_res.outcome
                {
                    ShadowCritiqueOutcome::Critiqued { critique, .. } => {
                        let bytes = serde_json::to_vec(critique)?;
                        (
                            critique.directives.clone(),
                            critique.strengths_to_preserve.clone(),
                            critique.requires_author_decision.clone(),
                            generation_sha256_hex(&bytes),
                        )
                    }
                    ShadowCritiqueOutcome::CriticInvalid { reason, .. } => (
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        generation_sha256_hex(reason.as_bytes()),
                    ),
                };
                {
                    let r = &mut loop_artifact.rounds[(round_num - 1) as usize];
                    r.critique_outcome = Some(critic_res.outcome);
                    r.selected_directives = directives;
                    r.strengths_to_preserve = strengths;
                    r.requires_author_decision = author_decisions.clone();
                    r.prompt_hashes.extend(critic_res.prompt_hashes);
                    r.model_call_ids.extend(critic_res.model_call_ids.clone());
                    r.tokens_used = r.tokens_used.saturating_add(critic_tokens);
                }
                for item in author_decisions {
                    if !loop_artifact
                        .requires_author_decision
                        .iter()
                        .any(|existing| existing.id == item.id)
                    {
                        loop_artifact.requires_author_decision.push(item);
                    }
                }
                record_transition(
                    &mut loop_artifact,
                    &working_hash,
                    EditorialStage::Critiqued,
                    round_num,
                    out_hash,
                    critic_res.model_call_ids,
                    critic_tokens,
                );
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
            }
            if options.crash_after
                == Some(BoundaryCrashPoint::Stage(
                    EditorialStage::Critiqued,
                    round_num,
                ))
            {
                anyhow::bail!("simulated_crash_at_boundary:critiqued:{round_num}");
            }

            // Evaluate critic outcome & actionable directive stop rules.
            {
                let r = &loop_artifact.rounds[(round_num - 1) as usize];
                if matches!(
                    r.critique_outcome,
                    Some(ShadowCritiqueOutcome::CriticInvalid { .. })
                ) {
                    loop_artifact.parked_for_review = true;
                    loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                        Some(RevisionRetentionReason::CriticInvalid);
                    loop_artifact.retention_reason = Some(RevisionRetentionReason::CriticInvalid);
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    break;
                }
                if r.selected_directives.is_empty() {
                    loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                        Some(RevisionRetentionReason::NoActionableDirectives);
                    loop_artifact.retention_reason =
                        Some(RevisionRetentionReason::NoActionableDirectives);
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    break;
                }
            }

            // Stage 3: CandidateReady
            if loop_artifact.rounds[(round_num - 1) as usize]
                .candidate_text
                .is_none()
            {
                if loop_artifact.total_calls_used >= loop_artifact.max_scene_calls
                    || loop_artifact.total_tokens_used >= loop_artifact.max_scene_tokens
                {
                    loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                        Some(RevisionRetentionReason::BudgetExhausted);
                    loop_artifact.retention_reason = Some(RevisionRetentionReason::BudgetExhausted);
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    break;
                }
                let directives = loop_artifact.rounds[(round_num - 1) as usize]
                    .selected_directives
                    .clone();
                let strengths = loop_artifact.rounds[(round_num - 1) as usize]
                    .strengths_to_preserve
                    .clone();
                let revision_admission = gate
                    .map(|gate| gate.admit(EditorialActionKind::Revise, round_num, "revision"))
                    .transpose()
                    .map_err(|error| anyhow::anyhow!("editorial_budget_gate: {error}"))?;
                let revision_start = std::time::Instant::now();
                let revision_result = self
                    .repository
                    .model_router()
                    .revise_draft_candidate(
                        policy,
                        packet,
                        &working_text,
                        &working_hash,
                        &directives,
                        &strengths,
                        revision_budget,
                        tokenizer,
                    )
                    .await;
                let rev_res = match revision_result {
                    Ok(result) => result,
                    Err(error) => {
                        if let (Some(gate), Some(admission)) = (gate, &revision_admission) {
                            let _ = gate.reconcile(admission, None, Vec::new());
                        }
                        return Err(error);
                    }
                };
                if let (Some(gate), Some(admission)) = (gate, &revision_admission) {
                    let usage = RevisionPassUsage {
                        calls: 1,
                        input_tokens: Some(rev_res.input_tokens),
                        output_tokens: Some(rev_res.output_tokens),
                        total_tokens: rev_res.input_tokens.saturating_add(rev_res.output_tokens),
                        millis: elapsed_millis(revision_start),
                    };
                    gate.reconcile(
                        admission,
                        Some(usage),
                        rev_res.response.call_id.clone().into_iter().collect(),
                    )
                    .map_err(|error| anyhow::anyhow!("editorial_budget_gate: {error}"))?;
                }
                let receipt = self
                    .register_generation_receipt_with_calls(
                        &rev_res.route_used,
                        Some(packet.scene_card.rating.as_str()),
                        &rev_res.response.model_name,
                        &rev_res.response.output,
                        rev_res.response.call_id.clone().into_iter().collect(),
                    )
                    .await?;
                let cand_sha = generation_sha256_hex(rev_res.response.output.as_bytes());
                let rev_tokens = rev_res.input_tokens.saturating_add(rev_res.output_tokens);
                loop_artifact.total_calls_used = loop_artifact.total_calls_used.saturating_add(1);
                loop_artifact.total_tokens_used =
                    loop_artifact.total_tokens_used.saturating_add(rev_tokens);
                let call_ids: Vec<String> = rev_res.response.call_id.into_iter().collect();
                {
                    let r = &mut loop_artifact.rounds[(round_num - 1) as usize];
                    r.candidate_text = Some(rev_res.response.output);
                    r.candidate_text_sha256 = Some(cand_sha.clone());
                    r.candidate_truncated = Some(rev_res.response.truncated);
                    r.candidate_receipt = Some(DraftReceiptBinding {
                        generation_id: receipt.id,
                        agent_id: receipt.agent_id,
                        normalized_output_sha256: receipt.output_sha256,
                    });
                    r.prompt_hashes.push(rev_res.prompt_hash);
                    r.model_call_ids.extend(call_ids.clone());
                    r.tokens_used = r.tokens_used.saturating_add(rev_tokens);
                }
                record_transition(
                    &mut loop_artifact,
                    &working_hash,
                    EditorialStage::CandidateReady,
                    round_num,
                    cand_sha,
                    call_ids,
                    rev_tokens,
                );
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
            }
            if options.crash_after
                == Some(BoundaryCrashPoint::Stage(
                    EditorialStage::CandidateReady,
                    round_num,
                ))
            {
                anyhow::bail!("simulated_crash_at_boundary:candidate_ready:{round_num}");
            }

            let cand_text = loop_artifact.rounds[(round_num - 1) as usize]
                .candidate_text
                .clone()
                .expect("candidate_text set");
            let cand_sha = loop_artifact.rounds[(round_num - 1) as usize]
                .candidate_text_sha256
                .clone()
                .expect("candidate_text_sha256 set");
            let cand_truncated = loop_artifact.rounds[(round_num - 1) as usize]
                .candidate_truncated
                .unwrap_or(false);

            // Check identical candidate hash stop before deterministic check or judging.
            if cand_sha == working_hash {
                loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                    Some(RevisionRetentionReason::IdenticalCandidateHash);
                loop_artifact.retention_reason =
                    Some(RevisionRetentionReason::IdenticalCandidateHash);
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                break;
            }
            if cand_truncated {
                loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                    Some(RevisionRetentionReason::CandidateTruncated);
                loop_artifact.retention_reason = Some(RevisionRetentionReason::CandidateTruncated);
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                break;
            }

            // Stage 4: CandidateChecked
            let comparison = {
                let r = &loop_artifact.rounds[(round_num - 1) as usize];
                if let Some(existing_cmp) = &r.candidate_check_comparison {
                    existing_cmp.clone()
                } else {
                    let directives = r.selected_directives.clone();
                    let edit_summary =
                        compute_edit_span_summary(&working_text, &cand_text, &directives);
                    if edit_summary.broad_change_flagged {
                        loop_artifact.broad_edit_flagged_for_review = true;
                    }
                    let cand_findings = project_external_findings_to_text(
                        &cand_text,
                        &cand_sha,
                        &external_findings,
                    );
                    let cand_rep = evaluate_deterministic_checks(
                        packet,
                        &cand_text,
                        cand_truncated,
                        cand_findings,
                    )?;
                    let cmp = compare_candidate_checks(
                        packet,
                        &working_text,
                        &inc_report,
                        &cand_text,
                        cand_rep,
                    );
                    let cmp_hash = generation_sha256_hex(&serde_json::to_vec(&cmp)?);
                    {
                        let r_mut = &mut loop_artifact.rounds[(round_num - 1) as usize];
                        r_mut.edit_span_summary = Some(edit_summary);
                        r_mut.candidate_check_comparison = Some(cmp.clone());
                    }
                    record_transition(
                        &mut loop_artifact,
                        &working_hash,
                        EditorialStage::CandidateChecked,
                        round_num,
                        cmp_hash,
                        Vec::new(),
                        0,
                    );
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    cmp
                }
            };
            if options.crash_after
                == Some(BoundaryCrashPoint::Stage(
                    EditorialStage::CandidateChecked,
                    round_num,
                ))
            {
                anyhow::bail!("simulated_crash_at_boundary:candidate_checked:{round_num}");
            }

            if comparison.disqualified {
                loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                    Some(RevisionRetentionReason::CandidateDisqualifiedByDeterministicCheck);
                loop_artifact.retention_reason =
                    Some(RevisionRetentionReason::CandidateDisqualifiedByDeterministicCheck);
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                break;
            }

            // If external/deterministic findings existed on the incumbent and the candidate resolved 0 of them,
            // stop when finding fingerprint is unchanged.
            if !inc_report.ingested_findings.is_empty()
                && comparison.resolved_finding_count == 0
                && comparison.candidate_report.normalized_finding_fingerprint
                    == inc_report.normalized_finding_fingerprint
            {
                loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                    Some(RevisionRetentionReason::NoDirectiveProgress);
                loop_artifact.retention_reason = Some(RevisionRetentionReason::NoDirectiveProgress);
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                break;
            }

            // Stage 5: PairwiseJudged (Pass 1 + swapped Pass 2)
            let (p1_arm, p2_arm) = select_initial_judge_order(
                &draft.packet_hash,
                &working_hash,
                &cand_sha,
                round_num,
                judge_budget.generation_seed,
            );

            let mut judge_admission: Option<RevisionBudgetAdmission> = None;
            let mut judge_usage = RevisionPassUsage::default();
            let mut judge_call_ids: Vec<String> = Vec::new();
            let mut judge_preexisting_pass = false;
            let mut judge_started: Option<std::time::Instant> = None;

            // Judge Pass 1
            if loop_artifact.rounds[(round_num - 1) as usize]
                .judge_pass_1
                .is_none()
            {
                if loop_artifact.total_calls_used >= loop_artifact.max_scene_calls
                    || loop_artifact.total_tokens_used >= loop_artifact.max_scene_tokens
                {
                    loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                        Some(RevisionRetentionReason::BudgetExhausted);
                    loop_artifact.retention_reason = Some(RevisionRetentionReason::BudgetExhausted);
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    break;
                }
                if judge_admission.is_none() {
                    judge_admission = gate
                        .map(|gate| {
                            gate.admit(EditorialActionKind::Judge, round_num, "judge_pass_1")
                        })
                        .transpose()
                        .map_err(|error| anyhow::anyhow!("editorial_budget_gate: {error}"))?;
                    judge_started = Some(std::time::Instant::now());
                }
                let (p1_text, p2_text) = if p1_arm == DeblindedArm::Baseline {
                    (working_text.as_str(), cand_text.as_str())
                } else {
                    (cand_text.as_str(), working_text.as_str())
                };
                match self
                    .repository
                    .model_router()
                    .judge_candidate_single_pass(
                        policy,
                        packet,
                        1,
                        p1_arm,
                        p1_text,
                        p2_arm,
                        p2_text,
                        judge_budget,
                        tokenizer,
                    )
                    .await
                {
                    Ok(pass1_res) => {
                        let pass1_tokens = pass1_res
                            .input_tokens
                            .saturating_add(pass1_res.output_tokens);
                        loop_artifact.total_calls_used =
                            loop_artifact.total_calls_used.saturating_add(1);
                        loop_artifact.total_tokens_used =
                            loop_artifact.total_tokens_used.saturating_add(pass1_tokens);
                        judge_usage.calls = judge_usage.calls.saturating_add(1);
                        judge_usage.input_tokens = Some(
                            judge_usage
                                .input_tokens
                                .unwrap_or(0)
                                .saturating_add(pass1_res.input_tokens),
                        );
                        judge_usage.output_tokens = Some(
                            judge_usage
                                .output_tokens
                                .unwrap_or(0)
                                .saturating_add(pass1_res.output_tokens),
                        );
                        judge_usage.total_tokens =
                            judge_usage.total_tokens.saturating_add(pass1_tokens);
                        if let Some(cid) = pass1_res.record.model_call_id.clone() {
                            judge_call_ids.push(cid);
                        }
                        let r = &mut loop_artifact.rounds[(round_num - 1) as usize];
                        if let Some(cid) = pass1_res.record.model_call_id.clone() {
                            r.model_call_ids.push(cid);
                        }
                        r.prompt_hashes.push(pass1_res.prompt_hash);
                        r.tokens_used = r.tokens_used.saturating_add(pass1_tokens);
                        r.judge_pass_1 = Some(pass1_res.record);
                        revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let pre_dispatch = msg.contains("budget_unknown")
                            || msg.contains("mandatory_overflow")
                            || msg.contains("clearance")
                            || msg.contains("requires critique_v1");
                        if !pre_dispatch {
                            loop_artifact.total_calls_used =
                                loop_artifact.total_calls_used.saturating_add(1);
                        }
                        loop_artifact.parked_for_review = true;
                        loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                            Some(RevisionRetentionReason::JudgeInvalidOrUnavailable);
                        loop_artifact.retention_reason =
                            Some(RevisionRetentionReason::JudgeInvalidOrUnavailable);
                        revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                        if let (Some(gate), Some(admission)) = (gate, &judge_admission) {
                            let _ = gate.reconcile(admission, None, Vec::new());
                        }
                        break;
                    }
                }
            }
            if options.crash_after == Some(BoundaryCrashPoint::JudgePass1(round_num)) {
                anyhow::bail!("simulated_crash_at_boundary:judge_pass_1:{round_num}");
            }

            // Judge Pass 2 (Swapped Order)
            if loop_artifact.rounds[(round_num - 1) as usize]
                .judge_pass_2
                .is_none()
            {
                if loop_artifact.total_calls_used >= loop_artifact.max_scene_calls
                    || loop_artifact.total_tokens_used >= loop_artifact.max_scene_tokens
                {
                    loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                        Some(RevisionRetentionReason::BudgetExhausted);
                    loop_artifact.retention_reason = Some(RevisionRetentionReason::BudgetExhausted);
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    break;
                }
                if judge_admission.is_none() {
                    judge_preexisting_pass = true;
                    judge_admission = gate
                        .map(|gate| {
                            gate.admit(EditorialActionKind::Judge, round_num, "judge_pass_2")
                        })
                        .transpose()
                        .map_err(|error| anyhow::anyhow!("editorial_budget_gate: {error}"))?;
                    judge_started = Some(std::time::Instant::now());
                }
                let (swap_p1_arm, swap_p2_arm) = (p2_arm, p1_arm);
                let (p1_text, p2_text) = if swap_p1_arm == DeblindedArm::Baseline {
                    (working_text.as_str(), cand_text.as_str())
                } else {
                    (cand_text.as_str(), working_text.as_str())
                };
                match self
                    .repository
                    .model_router()
                    .judge_candidate_single_pass(
                        policy,
                        packet,
                        2,
                        swap_p1_arm,
                        p1_text,
                        swap_p2_arm,
                        p2_text,
                        judge_budget,
                        tokenizer,
                    )
                    .await
                {
                    Ok(pass2_res) => {
                        let pass2_tokens = pass2_res
                            .input_tokens
                            .saturating_add(pass2_res.output_tokens);
                        loop_artifact.total_calls_used =
                            loop_artifact.total_calls_used.saturating_add(1);
                        loop_artifact.total_tokens_used =
                            loop_artifact.total_tokens_used.saturating_add(pass2_tokens);
                        judge_usage.calls = judge_usage.calls.saturating_add(1);
                        judge_usage.input_tokens = Some(
                            judge_usage
                                .input_tokens
                                .unwrap_or(0)
                                .saturating_add(pass2_res.input_tokens),
                        );
                        judge_usage.output_tokens = Some(
                            judge_usage
                                .output_tokens
                                .unwrap_or(0)
                                .saturating_add(pass2_res.output_tokens),
                        );
                        judge_usage.total_tokens =
                            judge_usage.total_tokens.saturating_add(pass2_tokens);
                        if let Some(cid) = pass2_res.record.model_call_id.clone() {
                            judge_call_ids.push(cid);
                        }
                        let r = &mut loop_artifact.rounds[(round_num - 1) as usize];
                        if let Some(cid) = pass2_res.record.model_call_id.clone() {
                            r.model_call_ids.push(cid);
                        }
                        r.prompt_hashes.push(pass2_res.prompt_hash);
                        r.tokens_used = r.tokens_used.saturating_add(pass2_tokens);
                        r.judge_pass_2 = Some(pass2_res.record);
                        revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let pre_dispatch = msg.contains("budget_unknown")
                            || msg.contains("mandatory_overflow")
                            || msg.contains("clearance")
                            || msg.contains("requires critique_v1");
                        if !pre_dispatch {
                            loop_artifact.total_calls_used =
                                loop_artifact.total_calls_used.saturating_add(1);
                        }
                        loop_artifact.parked_for_review = true;
                        loop_artifact.rounds[(round_num - 1) as usize].round_stop_reason =
                            Some(RevisionRetentionReason::JudgeInvalidOrUnavailable);
                        loop_artifact.retention_reason =
                            Some(RevisionRetentionReason::JudgeInvalidOrUnavailable);
                        revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                        if let (Some(gate), Some(admission)) = (gate, &judge_admission) {
                            let _ = gate.reconcile(admission, None, Vec::new());
                        }
                        break;
                    }
                }
            }

            if let (Some(gate), Some(admission)) = (gate, &judge_admission)
                && judge_usage.calls > 0
            {
                let usage = if judge_preexisting_pass {
                    // A prior run already charged one pass; do not refund it.
                    None
                } else {
                    Some(RevisionPassUsage {
                        calls: judge_usage.calls,
                        input_tokens: judge_usage.input_tokens,
                        output_tokens: judge_usage.output_tokens,
                        total_tokens: judge_usage.total_tokens,
                        millis: judge_started.map(elapsed_millis).unwrap_or(0),
                    })
                };
                gate.reconcile(admission, usage, judge_call_ids.clone())
                    .map_err(|error| anyhow::anyhow!("editorial_budget_gate: {error}"))?;
            }

            // Combine Pass 1 and Pass 2 into DoubleOrderJudgeEvaluation.
            let (double_eval, judge_retention) = {
                let r = &loop_artifact.rounds[(round_num - 1) as usize];
                if let Some(existing_eval) = &r.double_order_judgment {
                    (existing_eval.clone(), r.round_stop_reason)
                } else {
                    let pass_1 = r.judge_pass_1.clone().expect("judge_pass_1 present");
                    let pass_2 = r.judge_pass_2.clone().expect("judge_pass_2 present");
                    let mut judge_calls = Vec::new();
                    if let Some(c1) = pass_1.model_call_id.clone() {
                        judge_calls.push(c1);
                    }
                    if let Some(c2) = pass_2.model_call_id.clone() {
                        judge_calls.push(c2);
                    }
                    let (eval, retention) = resolve_double_order_judgment(pass_1, pass_2);
                    let eval_hash = generation_sha256_hex(&serde_json::to_vec(&eval)?);
                    {
                        let r_mut = &mut loop_artifact.rounds[(round_num - 1) as usize];
                        r_mut.double_order_judgment = Some(eval.clone());
                        r_mut.round_accepted = eval.accepted;
                        r_mut.round_stop_reason = retention;
                    }
                    record_transition(
                        &mut loop_artifact,
                        &working_hash,
                        EditorialStage::PairwiseJudged,
                        round_num,
                        eval_hash,
                        judge_calls,
                        0,
                    );
                    revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                    (eval, retention)
                }
            };
            if options.crash_after
                == Some(BoundaryCrashPoint::Stage(
                    EditorialStage::PairwiseJudged,
                    round_num,
                ))
            {
                anyhow::bail!("simulated_crash_at_boundary:pairwise_judged:{round_num}");
            }

            if !double_eval.accepted {
                loop_artifact.retention_reason =
                    Some(judge_retention.unwrap_or(RevisionRetentionReason::JudgeTie));
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                break;
            }

            // Candidate won both orders! Update working incumbent.
            let cand_receipt = loop_artifact.rounds[(round_num - 1) as usize]
                .candidate_receipt
                .clone()
                .expect("candidate_receipt present");
            working_text = cand_text;
            working_hash = cand_sha;
            working_receipt = cand_receipt;
            any_round_accepted = true;
            loop_artifact.retention_reason = None;

            // Check whether a second round is warranted and permitted by budget.
            if round_num < loop_artifact.max_rounds {
                let remaining_issues = !comparison.candidate_report.ingested_findings.is_empty();
                let new_fp = !loop_artifact
                    .seen_finding_fingerprints
                    .contains(&comparison.candidate_report.normalized_finding_fingerprint);
                let budget_allows_round_2 = loop_artifact.total_calls_used.saturating_add(4)
                    <= loop_artifact.max_scene_calls
                    && loop_artifact.total_tokens_used < loop_artifact.max_scene_tokens;
                if !remaining_issues {
                    break;
                }
                if !new_fp {
                    loop_artifact.retention_reason =
                        Some(RevisionRetentionReason::RepeatedFindingFingerprint);
                    break;
                }
                if !budget_allows_round_2 {
                    loop_artifact.retention_reason = Some(RevisionRetentionReason::BudgetExhausted);
                    break;
                }
            }
        }

        // Terminal Stage: CAS verification and atomic promotion or incumbent retention.
        if any_round_accepted {
            let unresolved_hard_blockers = if let Some(pending) = &loop_artifact.pending_promotion {
                pending.unresolved_hard_blockers
            } else {
                let final_findings = project_external_findings_to_text(
                    &working_text,
                    &working_hash,
                    &external_findings,
                );
                let final_det =
                    evaluate_deterministic_checks(packet, &working_text, false, final_findings)?;
                let blockers = !final_det.hard_continuity_blockers.is_empty()
                    || !final_det.hard_rating_violations.is_empty()
                    || !final_det.hard_antislop_blockers.is_empty();
                // Persist durable promotion intent before touching the draft artifact on disk
                // so a crash between saving the promoted draft and saving the terminal loop
                // artifact can recover idempotently on resume.
                loop_artifact.pending_promotion = Some(PendingRevisionPromotion {
                    candidate_text: working_text.clone(),
                    candidate_text_sha256: working_hash.clone(),
                    candidate_receipt: working_receipt.clone(),
                    unresolved_hard_blockers: blockers,
                });
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                blockers
            };

            // Synchronize live repository CAS verification and draft artifact promotion
            // under both the SQLite single-writer lock and the per-path artifact lock.
            let draft_path_owned = draft_artifact_path.to_path_buf();
            let expected_cas = loop_artifact.cas_expectation.clone();
            let working_text_for_write = working_text.clone();
            let working_hash_for_write = working_hash.clone();
            let working_receipt_for_write = working_receipt.clone();

            let cas_ok = self
                .repository
                .pool()
                .write(move |conn| {
                    Ok((|| -> Result<bool> {
                        let _draft_lock = artifact::lock_artifact(&draft_path_owned)?;
                        let current_disk_draft = artifact::load_artifact(&draft_path_owned)?;
                        // Recognize already-completed draft promotion when recovering from a
                        // crash between draft save and terminal loop save.
                        let already_promoted_by_pending = current_disk_draft.text_sha256
                            == working_hash_for_write
                            && current_disk_draft.receipt == working_receipt_for_write
                            && current_disk_draft.accepted_text_sha256.as_deref()
                                == Some(working_hash_for_write.as_str())
                            && current_disk_draft.extraction.is_none()
                            && current_disk_draft.extraction_request.is_none()
                            && current_disk_draft.extraction_attempts == 0
                            && current_disk_draft.extraction_failure.is_none();
                        if already_promoted_by_pending {
                            return Ok(true);
                        }
                        // Compose with P3 active-run scene handoff plan-revision check so
                        // a superseded or mismatched authoring-run scene handoff cannot be
                        // bypassed by revision promotion.
                        let handoff_ok =
                            crate::sqlite::repository::check_active_run_scene_handoff_plan_revision_in_conn(
                                conn,
                                &current_disk_draft.project_id,
                                &current_disk_draft.branch_id,
                                current_disk_draft.placement.book_number,
                                current_disk_draft.placement.chapter_number,
                                current_disk_draft.placement.scene_order.unwrap_or(1),
                            )
                            .is_ok();
                        if !handoff_ok {
                            return Ok(false);
                        }
                        let live_cas =
                            resolve_live_cas_expectation_in_conn(conn, &current_disk_draft)?;
                        if live_cas != expected_cas {
                            return Ok(false);
                        }
                        let mut promoted_draft = current_disk_draft;
                        promoted_draft.text = working_text_for_write;
                        promoted_draft.text_sha256 = working_hash_for_write.clone();
                        promoted_draft.truncated = false;
                        promoted_draft.receipt = working_receipt_for_write;
                        promoted_draft.accepted_text_sha256 = Some(working_hash_for_write);
                        // Invalidate all text-bound extraction state from the prior draft
                        // so extraction is required for the newly accepted revision hash.
                        promoted_draft.extraction = None;
                        promoted_draft.extraction_request = None;
                        promoted_draft.extraction_attempts = 0;
                        promoted_draft.extraction_failure = None;
                        artifact::save_artifact(&draft_path_owned, &promoted_draft)?;
                        Ok(true)
                    })())
                })
                .await??;

            if !cas_ok {
                // CAS conflict: fail closed, retain incumbent, never overwrite draft_artifact_path.
                loop_artifact.pending_promotion = None;
                loop_artifact.accepted_text = loop_artifact.initial_text.clone();
                loop_artifact.accepted_text_sha256 = loop_artifact.initial_text_sha256.clone();
                loop_artifact.accepted_receipt = loop_artifact.initial_receipt.clone();
                loop_artifact.revision_promoted = false;
                loop_artifact.retention_reason = Some(RevisionRetentionReason::StaleCasConflict);
                let init_hash = loop_artifact.initial_text_sha256.clone();
                let terminal_attempt = loop_artifact.rounds.len() as u32;
                record_transition(
                    &mut loop_artifact,
                    &init_hash,
                    EditorialStage::AcceptedOriginal,
                    terminal_attempt,
                    init_hash.clone(),
                    Vec::new(),
                    0,
                );
                revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
                return Ok(loop_artifact);
            }

            if options.crash_after == Some(BoundaryCrashPoint::AfterDraftPromotionSave) {
                anyhow::bail!("simulated_crash_at_boundary:after_draft_promotion_save");
            }

            if unresolved_hard_blockers {
                loop_artifact.parked_for_review = true;
            }
            loop_artifact.pending_promotion = None;
            loop_artifact.accepted_text = working_text;
            loop_artifact.accepted_text_sha256 = working_hash.clone();
            loop_artifact.accepted_receipt = working_receipt.clone();
            loop_artifact.revision_promoted = true;
            let base_hash = loop_artifact.initial_text_sha256.clone();
            let terminal_attempt = loop_artifact.rounds.len() as u32;
            record_transition(
                &mut loop_artifact,
                &base_hash,
                EditorialStage::AcceptedRevision,
                terminal_attempt,
                working_hash,
                vec![working_receipt.generation_id],
                0,
            );
            revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
            Ok(loop_artifact)
        } else {
            // Retain original incumbent draft; if it has unresolved hard blockers, park for review.
            let initial_det = evaluate_deterministic_checks(
                packet,
                &loop_artifact.initial_text,
                false,
                external_findings,
            )?;
            let unresolved_hard_blockers = !initial_det.hard_continuity_blockers.is_empty()
                || !initial_det.hard_rating_violations.is_empty()
                || !initial_det.hard_antislop_blockers.is_empty();
            if unresolved_hard_blockers {
                loop_artifact.parked_for_review = true;
            }
            let _draft_lock = artifact::lock_artifact(draft_artifact_path)?;
            let mut current_disk_draft = artifact::load_artifact(draft_artifact_path)?;
            if current_disk_draft.text_sha256 == loop_artifact.initial_text_sha256
                && current_disk_draft.accepted_text_sha256.is_none()
            {
                let init_sha = loop_artifact.initial_text_sha256.clone();
                let _ = artifact::accept_text(&mut current_disk_draft, &init_sha);
                artifact::save_artifact(draft_artifact_path, &current_disk_draft)?;
            }
            loop_artifact.accepted_text = loop_artifact.initial_text.clone();
            loop_artifact.accepted_text_sha256 = loop_artifact.initial_text_sha256.clone();
            loop_artifact.accepted_receipt = loop_artifact.initial_receipt.clone();
            loop_artifact.revision_promoted = false;
            let init_hash = loop_artifact.initial_text_sha256.clone();
            let terminal_attempt = loop_artifact.rounds.len() as u32;
            record_transition(
                &mut loop_artifact,
                &init_hash,
                EditorialStage::AcceptedOriginal,
                terminal_attempt,
                init_hash.clone(),
                Vec::new(),
                0,
            );
            revision::save_revision_loop_artifact(loop_artifact_path, &loop_artifact)?;
            Ok(loop_artifact)
        }
    }

    /// Ensure a deterministic `(project_id, branch_id)` pair exists as an active
    /// project/branch in SQLite for evaluation packet runs. Never mutates an
    /// existing project's active branch to a different branch ID.
    pub async fn ensure_eval_project_branch(
        &self,
        project_id: &str,
        branch_id: &str,
    ) -> Result<()> {
        let pid = project_id.to_string();
        let bid = branch_id.to_string();
        let now = chrono::Utc::now().timestamp_micros();
        self.repository
            .pool()
            .write(move |conn| {
                let tx = conn.savepoint()?;
                tx.execute(
                    "INSERT INTO project \
                     (id, name, project_type, genre, reader_contract, active_branch_id, notes, created_at, updated_at) \
                     VALUES (?1, ?1, 'novel', 'eval', '{\"promise\":\"eval\",\"style_notes\":[],\"boundaries\":[]}', ?2, NULL, ?3, ?3) \
                     ON CONFLICT(id) DO NOTHING",
                    rusqlite::params![&pid, &bid, now],
                )?;
                tx.execute(
                    "INSERT INTO bible_branch \
                     (id, project_id, parent_branch_id, name, status, branch_type, description, created_from_save_point_id, created_at, updated_at) \
                     VALUES (?1, ?2, NULL, 'main', 'active', NULL, NULL, NULL, ?3, ?3) \
                     ON CONFLICT(id) DO NOTHING",
                    rusqlite::params![&bid, &pid, now],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        let active = self.repository.get_active_branch(project_id).await?;
        anyhow::ensure!(active.id == branch_id, "packet branch is no longer active");
        Ok(())
    }

    async fn resolve_live_cas_expectation(
        &self,
        draft: &PlainTextDraftArtifactV1,
    ) -> Result<CasVersionExpectation> {
        let draft_owned = draft.clone();
        self.repository
            .pool()
            .read(move |conn| Ok(resolve_live_cas_expectation_in_conn(conn, &draft_owned)))
            .await?
    }
}

fn query_rows_sorted_json(
    conn: &rusqlite::Connection,
    sql: &str,
    params: &[&dyn rusqlite::ToSql],
) -> Result<Vec<serde_json::Map<String, serde_json::Value>>> {
    use rusqlite::types::ValueRef;
    let mut stmt = conn.prepare(sql)?;
    let names = stmt
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut rows = stmt
        .query_map(params, |r| {
            let mut obj = serde_json::Map::new();
            for (index, name) in names.iter().enumerate() {
                let value = match r.get_ref(index)? {
                    ValueRef::Null => serde_json::Value::Null,
                    ValueRef::Integer(n) => serde_json::Value::from(n),
                    ValueRef::Real(n) => serde_json::Value::from(n),
                    ValueRef::Text(v) => {
                        serde_json::Value::String(String::from_utf8_lossy(v).into_owned())
                    }
                    ValueRef::Blob(b) => serde_json::Value::String(generation_sha256_hex(b)),
                };
                obj.insert(name.clone(), value);
            }
            Ok(obj)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.sort_by(|a, b| {
        let sa = serde_json::to_string(a).unwrap_or_default();
        let sb = serde_json::to_string(b).unwrap_or_default();
        sa.cmp(&sb)
    });
    Ok(rows)
}

/// Resolve and recompute the live branch, canon snapshot hash, plan revision,
/// and source versions from the SQLite repository combined with the on-disk
/// draft artifact packet so any repository mutation to canon, plans, branches,
/// or source entities invalidates CAS at promotion time.
fn resolve_live_cas_expectation_in_conn(
    conn: &rusqlite::Connection,
    draft: &PlainTextDraftArtifactV1,
) -> Result<CasVersionExpectation> {
    use rusqlite::OptionalExtension;
    use spindle_core::drafting::PacketSource;
    use std::collections::BTreeMap;

    let packet = &draft
        .prepared_packet
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("draft artifact missing prepared_packet"))?
        .packet;
    let project_id = &draft.project_id;
    let branch_id = &draft.branch_id;

    let active_branch_id: Option<String> = conn
        .query_row(
            "SELECT active_branch_id FROM project WHERE id = ?1",
            rusqlite::params![project_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    let live_branch_id = match active_branch_id {
        Some(id) => id,
        None => conn
            .query_row(
                "SELECT id FROM bible_branch WHERE project_id = ?1 AND name = 'main' LIMIT 1",
                rusqlite::params![project_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| branch_id.clone()),
    };

    let lineage = crate::sqlite::repository::resolve_branch_lineage_in_conn(
        conn,
        Some(project_id),
        branch_id,
    )?;

    let mut canon_state: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut plan_state: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut live_row_digests: BTreeMap<String, (String, String)> = BTreeMap::new();

    let mut record_rows = |target: &mut BTreeMap<String, serde_json::Value>,
                           key: String,
                           default_branch: &str,
                           rows: Vec<serde_json::Map<String, serde_json::Value>>|
     -> Result<()> {
        for row in &rows {
            if let Some(id) = row.get("id").and_then(serde_json::Value::as_str) {
                let row_branch = row
                    .get("branch_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(default_branch)
                    .to_string();
                let digest = generation_sha256_hex(&serde_json::to_vec(row)?);
                live_row_digests
                    .entry(id.to_string())
                    .or_insert((row_branch, digest));
            }
        }
        let vals = rows
            .into_iter()
            .map(serde_json::Value::Object)
            .collect::<Vec<_>>();
        target.insert(key, serde_json::Value::Array(vals));
        Ok(())
    };

    let project_rows = query_rows_sorted_json(
        conn,
        "SELECT id, name, project_type, genre, reader_contract, notes, updated_at FROM project WHERE id = ?1",
        &[&project_id],
    )?;
    record_rows(&mut canon_state, "project".into(), branch_id, project_rows)?;

    const CANON_PROJECT_BRANCH_TABLES: &[&str] = &[
        "canon_claim",
        "canon_decision",
        "canon_event",
        "canon_delta",
        "canonical_fact",
        "knowledge_fact",
        "character",
        "character_state",
        "character_arc",
        "location",
        "world_rule",
        "world_state",
        "faction",
        "religion",
        "economy",
        "plot_line",
        "conflict",
        "theme",
        "motif",
        "system_overlay",
        "narrative_promise",
        "term",
        "knows",
        "timeline_event",
        "temporal_intervention",
        "progression_event",
        "scene",
        "scene_version",
        "scene_beat_annotation",
        "revision_marker",
        "dual_persona_review",
        "future_knowledge",
    ];
    const CANON_BRANCH_ONLY_TABLES: &[&str] = &["relates_to", "relationship_scene_delta"];
    const CANON_CHARACTER_CHILD_TABLES: &[&str] =
        &["character_voice_profile", "character_emotional_profile"];

    for b in &lineage {
        for table in CANON_PROJECT_BRANCH_TABLES {
            let sql = format!("SELECT * FROM {table} WHERE project_id = ?1 AND branch_id = ?2");
            let rows = query_rows_sorted_json(conn, &sql, &[&project_id, &b])?;
            record_rows(&mut canon_state, format!("{b}:{table}"), b, rows)?;
        }
        for table in CANON_BRANCH_ONLY_TABLES {
            let sql = format!("SELECT * FROM {table} WHERE branch_id = ?1");
            let rows = query_rows_sorted_json(conn, &sql, &[&b])?;
            record_rows(&mut canon_state, format!("{b}:{table}"), b, rows)?;
        }
        for table in CANON_CHARACTER_CHILD_TABLES {
            let sql = format!(
                "SELECT * FROM {table} WHERE character_id IN \
                 (SELECT id FROM character WHERE project_id = ?1 AND branch_id = ?2)"
            );
            let rows = query_rows_sorted_json(conn, &sql, &[&project_id, &b])?;
            record_rows(&mut canon_state, format!("{b}:{table}"), b, rows)?;
        }
    }

    const PLAN_PROJECT_TABLES: &[&str] = &["book", "chapter"];
    const PLAN_PROJECT_BRANCH_TABLES: &[&str] = &[
        "chapter_plan",
        "chapter_summary",
        "plan_amendment",
        "pacing_config",
        "pacing_curve",
        "pacing_tracker",
        "planning_session",
    ];
    const PLAN_BRANCH_ONLY_TABLES: &[&str] = &["book_outline", "chapter_outline"];

    for table in PLAN_PROJECT_TABLES {
        let sql = format!("SELECT * FROM {table} WHERE project_id = ?1");
        let rows = query_rows_sorted_json(conn, &sql, &[&project_id])?;
        record_rows(&mut plan_state, format!("project:{table}"), branch_id, rows)?;
    }
    for b in &lineage {
        for table in PLAN_PROJECT_BRANCH_TABLES {
            let sql = format!("SELECT * FROM {table} WHERE project_id = ?1 AND branch_id = ?2");
            let rows = query_rows_sorted_json(conn, &sql, &[&project_id, &b])?;
            record_rows(&mut plan_state, format!("{b}:{table}"), b, rows)?;
        }
        let proposal_rows = query_rows_sorted_json(
            conn,
            "SELECT * FROM planning_proposal WHERE session_id IN \
             (SELECT id FROM planning_session WHERE project_id = ?1 AND branch_id = ?2)",
            &[&project_id, &b],
        )?;
        record_rows(
            &mut plan_state,
            format!("{b}:planning_proposal"),
            b,
            proposal_rows,
        )?;
        for table in PLAN_BRANCH_ONLY_TABLES {
            let sql = format!("SELECT * FROM {table} WHERE branch_id = ?1");
            let rows = query_rows_sorted_json(conn, &sql, &[&b])?;
            record_rows(&mut plan_state, format!("{b}:{table}"), b, rows)?;
        }
    }

    let scene_order = draft.placement.scene_order.unwrap_or(1);
    let handoff_rows = query_rows_sorted_json(
        conn,
        "SELECT r.id AS run_id, r.status AS run_status, s.phase, s.draft_diagnostics
         FROM authoring_run r
         JOIN authoring_run_scene s ON s.authoring_run_id = r.id
         WHERE r.project_id = ?1
           AND r.active_branch_id = ?2
           AND r.book_number = ?3
           AND r.status IN ('active', 'blocked')
           AND s.chapter_number = ?4
           AND s.scene_order = ?5",
        &[
            &project_id,
            &branch_id,
            &draft.placement.book_number,
            &draft.placement.chapter_number,
            &scene_order,
        ],
    )?;
    record_rows(
        &mut plan_state,
        format!("{branch_id}:active_run_scene_handoff"),
        branch_id,
        handoff_rows,
    )?;

    let live_canon_hash = generation_sha256_hex(&serde_json::to_vec(&canon_state)?);
    let live_plan_hash = generation_sha256_hex(&serde_json::to_vec(&plan_state)?);

    let mut source_versions: Vec<PacketSource> = packet
        .source_versions
        .iter()
        .map(|src| {
            if let Some((_, row_digest)) = live_row_digests.get(&src.source_id) {
                PacketSource {
                    branch_id: src.branch_id.clone(),
                    source_id: src.source_id.clone(),
                    revision_hash: format!("{}:repo:{row_digest}", src.revision_hash),
                }
            } else {
                src.clone()
            }
        })
        .collect();
    for (id, (row_branch, row_digest)) in live_row_digests {
        source_versions.push(PacketSource {
            branch_id: row_branch,
            source_id: format!("repo:{id}"),
            revision_hash: row_digest,
        });
    }
    source_versions.sort();
    source_versions.dedup();

    Ok(CasVersionExpectation {
        branch_id: live_branch_id,
        canon_snapshot_hash: format!(
            "{}:repo_canon:{live_canon_hash}",
            packet.canon_snapshot_hash
        ),
        plan_revision: format!("{}:repo_plan:{live_plan_hash}", packet.plan_revision),
        source_versions,
        incumbent_text_sha256: draft.text_sha256.clone(),
    })
}

/// Project external findings onto `target_text` (`target_hash`).
///
/// Unanchored findings and findings already anchored to `target_hash` are kept as-is.
/// For findings anchored to a prior draft hash with a non-empty `exact_quote`:
/// - if `target_text` still contains `exact_quote` untouched, the anchor is rebased onto
///   `target_hash` at the new byte offsets so the unresolved finding remains active;
/// - if `target_text` rewrote or removed `exact_quote`, the anchored span was resolved
///   and the finding is omitted on `target_text`.
fn project_external_findings_to_text(
    target_text: &str,
    target_hash: &str,
    findings: &[CriticFindingInput],
) -> Vec<CriticFindingInput> {
    let mut projected = Vec::with_capacity(findings.len());
    for finding in findings {
        match &finding.anchor {
            None => projected.push(finding.clone()),
            Some(anchor) if anchor.base_text_hash == target_hash => {
                projected.push(finding.clone());
            }
            Some(anchor) => {
                if !anchor.exact_quote.is_empty()
                    && let Some(new_start) = target_text.find(&anchor.exact_quote)
                {
                    let mut rebased = finding.clone();
                    if let Some(ref mut a) = rebased.anchor {
                        a.base_text_hash = target_hash.to_string();
                        a.start_byte = new_start;
                        a.end_byte = new_start + anchor.exact_quote.len();
                    }
                    projected.push(rebased);
                }
            }
        }
    }
    projected
}

fn elapsed_millis(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}
