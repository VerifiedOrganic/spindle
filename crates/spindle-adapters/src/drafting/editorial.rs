//! Durable editorial-controller store and hybrid host-task lifecycle.
//!
//! Persistence follows the existing P1 local-artifact pattern: one JSON file,
//! atomic temp-file rename, and a per-path lock so concurrent owners cannot
//! double-admit a reservation. Model dispatch stays in the service layer.

use super::artifact;
use crate::drafting::revision::{
    compare_candidate_checks, compute_edit_span_summary, evaluate_deterministic_checks,
};
use sha2::{Digest, Sha256};
use spindle_core::drafting::*;
use std::path::Path;

pub fn editorial_sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Existing receipt semantics: CRLF normalized and outer whitespace trimmed.
pub fn normalized_output_sha256(text: &str) -> String {
    editorial_sha256_hex(text.replace("\r\n", "\n").trim().as_bytes())
}

pub fn save_editorial_controller(
    path: &Path,
    state: &EditorialControllerStateV1,
) -> anyhow::Result<()> {
    state.validate().map_err(anyhow::Error::msg)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), state)?;
    std::io::Write::flush(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

pub fn load_editorial_controller(path: &Path) -> anyhow::Result<EditorialControllerStateV1> {
    let state: EditorialControllerStateV1 = serde_json::from_reader(std::fs::File::open(path)?)?;
    state.validate().map_err(anyhow::Error::msg)?;
    Ok(state)
}

/// Locked read-modify-write. A second owner gets a busy error rather than
/// admitting against a stale counter snapshot.
pub fn update_editorial_controller<T>(
    path: &Path,
    update: impl FnOnce(&mut EditorialControllerStateV1) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let _lock = artifact::lock_artifact(path)?;
    let mut state = load_editorial_controller(path)?;
    let output = update(&mut state)?;
    save_editorial_controller(path, &state)?;
    Ok(output)
}

pub fn new_editorial_scene_state(
    chapter_number: u32,
    scene_order: u32,
    scene_id: Option<String>,
    packet_hash: String,
    source_packet_sha256: String,
    availability: EditorialRouteAvailability,
    settings: &ResolvedEditorialSettingsV1,
) -> EditorialSceneStateV1 {
    EditorialSceneStateV1 {
        chapter_number,
        scene_order,
        scene_id,
        packet_hash,
        source_packet_sha256,
        draft_artifact_path: None,
        candidate_artifact_path: None,
        loop_artifact_path: None,
        stage: EditorialStage::PacketReady,
        attempt: 0,
        incumbent: None,
        candidate: None,
        critic: None,
        judge: None,
        selected_directives: Vec::new(),
        strengths_to_preserve: Vec::new(),
        last_edit_span_summary: None,
        parked_issues: Vec::new(),
        pending_host_task: None,
        pending_manual_judgment: None,
        availability,
        ledger: EditorialBudgetLedgerV1::new(settings.scene_budget, settings.run_budget),
        attribution_event_ids: Vec::new(),
        line_edit_attempted: false,
        extracted: false,
    }
}

/// Admit one exact proposed pass against scene and run ceilings.
pub fn admit_editorial_action(
    scene: &mut EditorialSceneStateV1,
    settings: &ResolvedEditorialSettingsV1,
    action: EditorialActionKind,
    attempt: u32,
    now_millis: u64,
    paid_dispatch: bool,
) -> Result<EditorialBudgetReservation, BudgetDenial> {
    let key = EditorialReservationKey {
        run_id: String::new(),
        branch_id: String::new(),
        scene_id: scene.scene_id.clone(),
        packet_hash: scene.packet_hash.clone(),
        stage: scene.stage,
        attempt,
        action,
    };
    // The controller fills run/branch identity on the persisted reservation;
    // callers route through `admit_editorial_action_for_run` when they have it.
    admit_with_key(
        scene,
        settings,
        key,
        action,
        now_millis,
        paid_dispatch,
        EditorialBudgetUsage::default(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn admit_editorial_action_for_run(
    scene: &mut EditorialSceneStateV1,
    settings: &ResolvedEditorialSettingsV1,
    run_id: &str,
    branch_id: &str,
    action: EditorialActionKind,
    attempt: u32,
    now_millis: u64,
    paid_dispatch: bool,
) -> Result<EditorialBudgetReservation, BudgetDenial> {
    let key = EditorialReservationKey {
        run_id: run_id.to_string(),
        branch_id: branch_id.to_string(),
        scene_id: scene.scene_id.clone(),
        packet_hash: scene.packet_hash.clone(),
        stage: scene.stage,
        attempt,
        action,
    };
    admit_with_key(
        scene,
        settings,
        key,
        action,
        now_millis,
        paid_dispatch,
        EditorialBudgetUsage::default(),
    )
}

/// Controller-scoped admission: sums the run usage of every other scene before
/// checking the shared per-run ceilings.
#[allow(clippy::too_many_arguments)]
pub fn admit_editorial_action_in_controller(
    state: &mut EditorialControllerStateV1,
    scene_index: usize,
    action: EditorialActionKind,
    attempt: u32,
    now_millis: u64,
    paid_dispatch: bool,
) -> Result<EditorialBudgetReservation, BudgetDenial> {
    let prior_run_usage = state.run_usage_excluding(scene_index);
    let run_id = state.run_id.clone();
    let branch_id = state.branch_id.clone();
    let settings = state.settings.clone();
    let scene = state
        .scenes
        .get_mut(scene_index)
        .ok_or(BudgetDenial::UnknownScene)?;
    let key = EditorialReservationKey {
        run_id,
        branch_id,
        scene_id: scene.scene_id.clone(),
        packet_hash: scene.packet_hash.clone(),
        stage: scene.stage,
        attempt,
        action,
    };
    admit_with_key(
        scene,
        &settings,
        key,
        action,
        now_millis,
        paid_dispatch,
        prior_run_usage,
    )
}

fn admit_with_key(
    scene: &mut EditorialSceneStateV1,
    settings: &ResolvedEditorialSettingsV1,
    key: EditorialReservationKey,
    action: EditorialActionKind,
    now_millis: u64,
    paid_dispatch: bool,
    prior_run_usage: EditorialBudgetUsage,
) -> Result<EditorialBudgetReservation, BudgetDenial> {
    let idempotency_key = key.idempotency_key();
    let reserved = reservation_usage_for(action, settings);
    let reservation = EditorialBudgetReservation {
        reservation_id: format!("editorial_reservation:{idempotency_key}"),
        key,
        idempotency_key,
        reserved,
        paid_dispatch: paid_dispatch && action.is_model_pass(),
        pricing_known: settings.pricing_known(),
        usage_known: false,
        actual: None,
        model_call_ids: Vec::new(),
        dispatched_at_millis: None,
        created_at_millis: now_millis,
        reconciled_at_millis: None,
    };
    scene
        .ledger
        .admit_with_prior_run_usage(reservation, prior_run_usage)
}

/// Build the bounded host task for the scene's next hybrid pass. The task
/// snapshots the exact packet, so later retrieval cannot observe a changed
/// branch or snapshot.
#[allow(clippy::too_many_arguments)]
pub fn prepare_host_editorial_task(
    state: &mut EditorialControllerStateV1,
    scene_index: usize,
    task_id: &str,
    packet_hash: &str,
    packet: &DraftingPacketV1,
    incumbent_text: Option<&str>,
    now_millis: u64,
) -> anyhow::Result<HostEditorialTaskV1> {
    anyhow::ensure!(
        !state.settings.is_legacy(),
        "host editorial tasks require critique_v1 policy"
    );
    anyhow::ensure!(!task_id.trim().is_empty(), "host task id is required");
    anyhow::ensure!(
        packet.project_id == state.project_id && packet.branch_id == state.branch_id,
        "packet does not belong to the controller project/branch"
    );
    let run_id = state.run_id.clone();
    let branch_id = state.branch_id.clone();
    let prior_run_usage = state.run_usage_excluding(scene_index);
    let settings = state.settings.clone();
    let scene = state
        .scenes
        .get_mut(scene_index)
        .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
    anyhow::ensure!(
        scene.packet_hash == packet_hash,
        "packet hash does not match the persisted scene"
    );
    let kind = match scene.stage {
        EditorialStage::PacketReady => HostEditorialTaskKind::Draft,
        EditorialStage::Critiqued => HostEditorialTaskKind::Revision,
        other => anyhow::bail!("host task cannot start at stage {}", other.as_str()),
    };
    let action = match kind {
        HostEditorialTaskKind::Draft => EditorialActionKind::Draft,
        HostEditorialTaskKind::Revision => EditorialActionKind::Revise,
    };
    let incumbent = match kind {
        HostEditorialTaskKind::Draft => None,
        HostEditorialTaskKind::Revision => {
            let text = incumbent_text
                .ok_or_else(|| anyhow::anyhow!("revision host task requires the incumbent text"))?;
            let expected = scene
                .incumbent
                .as_ref()
                .map(|reference| reference.text_sha256.as_str())
                .ok_or_else(|| anyhow::anyhow!("revision scene has no incumbent reference"))?;
            anyhow::ensure!(
                editorial_sha256_hex(text.as_bytes()) == expected,
                "incumbent text hash does not match the persisted scene"
            );
            Some(text)
        }
    };
    let key = EditorialReservationKey {
        run_id: run_id.clone(),
        branch_id: branch_id.clone(),
        scene_id: scene.scene_id.clone(),
        packet_hash: scene.packet_hash.clone(),
        stage: scene.stage,
        attempt: scene.attempt,
        action,
    };
    let reservation = admit_with_key(
        scene,
        &settings,
        key,
        action,
        now_millis,
        settings.execution_mode == EditorialExecutionMode::Agent,
        prior_run_usage,
    )
    .map_err(|denial| anyhow::anyhow!("{denial}"))?;
    let task = HostEditorialTaskV1 {
        schema_version: HOST_EDITORIAL_TASK_SCHEMA_VERSION.into(),
        task_id: task_id.to_string(),
        kind,
        run_id,
        project_id: packet.project_id.clone(),
        branch_id: packet.branch_id.clone(),
        scene_id: scene.scene_id.clone(),
        packet_hash: packet_hash.to_string(),
        source_packet_sha256: scene.source_packet_sha256.clone(),
        reservation_id: reservation.reservation_id,
        rating: packet.scene_card.rating.clone(),
        target_words: packet.scene_card.target_words.clone(),
        packet: packet.clone(),
        incumbent_text: incumbent.map(str::to_string),
        incumbent_text_sha256: incumbent.map(|text| editorial_sha256_hex(text.as_bytes())),
        directives: scene.selected_directives.clone(),
        strengths_to_preserve: scene.strengths_to_preserve.clone(),
        max_output_tokens: action.proposed_output_tokens(),
        created_at_millis: now_millis,
        submitted_at_millis: None,
    };
    scene.pending_host_task = Some(task.clone());
    Ok(task)
}

/// Validate and apply a host submission. Deterministic checks run before any
/// judge dispatch; a disqualified revision retains the incumbent.
pub fn apply_host_editorial_submission(
    state: &mut EditorialControllerStateV1,
    submission: &HostEditorialSubmissionV1,
    now_millis: u64,
) -> anyhow::Result<(HostSubmissionOutcomeV1, Option<CandidateCheckComparison>)> {
    let Some((scene_index, task)) = state.scenes.iter().enumerate().find_map(|(index, scene)| {
        scene
            .pending_host_task
            .as_ref()
            .filter(|task| task.task_id == submission.task_id)
            .map(|task| (index, task.clone()))
    }) else {
        return Ok((
            HostSubmissionOutcomeV1::Rejected {
                rejection: HostSubmissionRejection::TaskNotFound,
                message: "host task not found or already retired".into(),
            },
            None,
        ));
    };
    if !task.is_pending() {
        return Ok((
            HostSubmissionOutcomeV1::Rejected {
                rejection: HostSubmissionRejection::TaskAlreadySubmitted,
                message: "host task was already submitted".into(),
            },
            None,
        ));
    }
    if task.packet_hash != submission.packet_hash {
        return Ok((
            HostSubmissionOutcomeV1::Rejected {
                rejection: HostSubmissionRejection::PacketHashMismatch,
                message: "submission packet hash does not match the task".into(),
            },
            None,
        ));
    }
    if submission.text.trim().is_empty() {
        return Ok((
            HostSubmissionOutcomeV1::Rejected {
                rejection: HostSubmissionRejection::EmptyText,
                message: "host submission text is empty".into(),
            },
            None,
        ));
    }
    if submission.truncated {
        return Ok((
            HostSubmissionOutcomeV1::Rejected {
                rejection: HostSubmissionRejection::Truncated,
                message: "truncated host prose cannot be accepted".into(),
            },
            None,
        ));
    }
    let reservation_key = task_reservation_key(&task, &state.scenes[scene_index]);
    if state.scenes[scene_index]
        .ledger
        .find(&reservation_key.idempotency_key())
        .is_none()
    {
        return Ok((
            HostSubmissionOutcomeV1::Rejected {
                rejection: HostSubmissionRejection::ReservationMissing,
                message: "host task has no admitted reservation".into(),
            },
            None,
        ));
    }

    let text_sha256 = editorial_sha256_hex(submission.text.as_bytes());
    let normalized_sha256 = normalized_output_sha256(&submission.text);
    let receipt = HostCompletionReceiptV1 {
        task_id: task.task_id.clone(),
        agent_id: "host".into(),
        text_sha256: text_sha256.clone(),
        normalized_output_sha256: normalized_sha256,
        usage_known: submission.usage.is_some(),
    };

    let scene = &mut state.scenes[scene_index];
    scene.ledger.reconcile(
        &reservation_key.idempotency_key(),
        submission.usage,
        Vec::new(),
        now_millis,
    );
    if let Some(pending) = scene.pending_host_task.as_mut() {
        pending.submitted_at_millis = Some(now_millis);
    }

    match task.kind {
        HostEditorialTaskKind::Draft => {
            scene.incumbent = Some(EditorialTextRefV1 {
                text_sha256: text_sha256.clone(),
                generation_id: Some(format!("host:{}", task.task_id)),
                agent_id: Some("host".into()),
            });
            scene.stage = EditorialStage::Drafted;
            Ok((
                HostSubmissionOutcomeV1::DraftAccepted {
                    receipt,
                    text_sha256,
                },
                None,
            ))
        }
        HostEditorialTaskKind::Revision => {
            let incumbent_text = task.incumbent_text.as_deref().ok_or_else(|| {
                anyhow::anyhow!("revision host task is missing its incumbent text")
            })?;
            let incumbent_hash = scene
                .incumbent
                .as_ref()
                .map(|reference| reference.text_sha256.clone())
                .ok_or_else(|| anyhow::anyhow!("revision scene has no incumbent reference"))?;
            anyhow::ensure!(
                editorial_sha256_hex(incumbent_text.as_bytes()) == incumbent_hash,
                "incumbent hash drifted since the host task was created"
            );
            let incumbent_report =
                evaluate_deterministic_checks(&task.packet, incumbent_text, false, Vec::new())
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
            let candidate_report =
                evaluate_deterministic_checks(&task.packet, &submission.text, false, Vec::new())
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
            let comparison = compare_candidate_checks(
                &task.packet,
                incumbent_text,
                &incumbent_report,
                &submission.text,
                candidate_report,
            );
            scene.last_edit_span_summary = Some(compute_edit_span_summary(
                incumbent_text,
                &submission.text,
                &task.directives,
            ));
            scene.candidate = Some(EditorialTextRefV1 {
                text_sha256: text_sha256.clone(),
                generation_id: Some(format!("host:{}", task.task_id)),
                agent_id: Some("host".into()),
            });
            if comparison.disqualified {
                let reason = comparison
                    .disqualification_reason
                    .clone()
                    .unwrap_or_else(|| "deterministic_candidate_check".into());
                scene.stage = EditorialStage::AcceptedOriginal;
                scene.judge = Some(EditorialJudgeSummaryV1 {
                    verdict: "baseline".into(),
                    accepted_revision: false,
                    order_agreement: false,
                    retention_reason: Some(
                        RevisionRetentionReason::CandidateDisqualifiedByDeterministicCheck
                            .as_str()
                            .into(),
                    ),
                });
                scene.parked_issues.push(reason.clone());
                Ok((
                    HostSubmissionOutcomeV1::RevisionRetained {
                        reason,
                        incumbent_text_sha256: incumbent_hash,
                        candidate_text_sha256: text_sha256,
                    },
                    Some(comparison),
                ))
            } else {
                scene.stage = EditorialStage::CandidateReady;
                Ok((
                    HostSubmissionOutcomeV1::CandidateReady {
                        receipt,
                        text_sha256,
                    },
                    Some(comparison),
                ))
            }
        }
    }
}

fn task_reservation_key(
    task: &HostEditorialTaskV1,
    scene: &EditorialSceneStateV1,
) -> EditorialReservationKey {
    EditorialReservationKey {
        run_id: task.run_id.clone(),
        branch_id: task.branch_id.clone(),
        scene_id: task.scene_id.clone(),
        packet_hash: scene.packet_hash.clone(),
        stage: scene.stage,
        attempt: scene.attempt,
        action: match task.kind {
            HostEditorialTaskKind::Draft => EditorialActionKind::Draft,
            HostEditorialTaskKind::Revision => EditorialActionKind::Revise,
        },
    }
}

/// Persist the double-order judge result and advance the terminal stage.
pub fn record_editorial_judgment(
    state: &mut EditorialControllerStateV1,
    scene_index: usize,
    evaluation: &DoubleOrderJudgeEvaluation,
    retention: Option<RevisionRetentionReason>,
    now_millis: u64,
) -> anyhow::Result<()> {
    let scene = state
        .scenes
        .get_mut(scene_index)
        .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
    let candidate = scene
        .candidate
        .clone()
        .ok_or_else(|| anyhow::anyhow!("judge result has no candidate text reference"))?;
    if evaluation.accepted {
        scene.incumbent = Some(candidate.clone());
        scene.stage = EditorialStage::AcceptedRevision;
        scene.judge = Some(EditorialJudgeSummaryV1 {
            verdict: evaluation.verdict.as_str().into(),
            accepted_revision: true,
            order_agreement: evaluation.position_bias.is_none(),
            retention_reason: None,
        });
    } else {
        scene.stage = EditorialStage::AcceptedOriginal;
        scene.judge = Some(EditorialJudgeSummaryV1 {
            verdict: evaluation.verdict.as_str().into(),
            accepted_revision: false,
            order_agreement: evaluation.position_bias.is_none(),
            retention_reason: retention
                .map(|reason| reason.as_str().to_string())
                .or_else(|| Some("judge_retained_incumbent".into())),
        });
    }
    scene.pending_host_task = None;
    scene.pending_manual_judgment = None;
    scene.attempt = scene.attempt.saturating_add(1);
    let _ = now_millis;
    Ok(())
}

pub fn mark_pending_manual_judgment(
    state: &mut EditorialControllerStateV1,
    scene_index: usize,
    reason: ManualJudgmentReason,
    now_millis: u64,
) -> anyhow::Result<()> {
    let scene = state
        .scenes
        .get_mut(scene_index)
        .ok_or_else(|| anyhow::anyhow!("editorial scene index out of range"))?;
    let incumbent = scene
        .incumbent
        .as_ref()
        .map(|reference| reference.text_sha256.clone())
        .ok_or_else(|| anyhow::anyhow!("manual judgment requires an incumbent draft"))?;
    let candidate = scene
        .candidate
        .as_ref()
        .map(|reference| reference.text_sha256.clone())
        .ok_or_else(|| anyhow::anyhow!("manual judgment requires a candidate draft"))?;
    scene.stage = EditorialStage::AcceptedOriginal;
    scene.pending_manual_judgment = Some(PendingManualJudgmentV1 {
        task_id: scene
            .pending_host_task
            .as_ref()
            .map(|task| task.task_id.clone())
            .unwrap_or_default(),
        reason,
        incumbent_text_sha256: incumbent,
        candidate_text_sha256: candidate,
        created_at_millis: now_millis,
    });
    scene.judge = Some(EditorialJudgeSummaryV1 {
        verdict: "pending_manual_judgment".into(),
        accepted_revision: false,
        order_agreement: false,
        retention_reason: Some(reason.as_str().into()),
    });
    scene.pending_host_task = None;
    scene.parked_issues.push(reason.as_str().into());
    Ok(())
}

/// Prose-free attribution. The event carries hashes, ids, enums and counts;
/// the prose stays in the local draft artifact only.
#[allow(clippy::too_many_arguments)]
pub fn record_model_attribution(
    state: &mut EditorialControllerStateV1,
    scene_index: usize,
    action: EditorialActionKind,
    prompt_template_id: String,
    prompt_version: String,
    prompt_hash: String,
    context_packet_hash: String,
    tokenizer: Option<(String, String)>,
    parent_text_sha256: Option<String>,
    candidate_text_sha256: Option<String>,
    judge_order: Option<String>,
    outcome: String,
    duration_millis: u64,
    actual: Option<EditorialBudgetUsage>,
    reserved: EditorialBudgetUsage,
    model_call_ids: Vec<String>,
    now_millis: u64,
) -> anyhow::Result<EditorialAttributionV1> {
    let event_id = format!(
        "editorial_attribution:{}:{}",
        state.run_id,
        state.attribution.len() + 1
    );
    let (tokenizer_id, tokenizer_version) = tokenizer.unzip();
    let event = EditorialAttributionV1 {
        schema_version: EDITORIAL_ATTRIBUTION_SCHEMA_VERSION.into(),
        event_id: event_id.clone(),
        run_id: state.run_id.clone(),
        project_id: state.project_id.clone(),
        branch_id: state.branch_id.clone(),
        scene_id: state
            .scenes
            .get(scene_index)
            .and_then(|scene| scene.scene_id.clone()),
        stage: state
            .scenes
            .get(scene_index)
            .map(|scene| scene.stage)
            .unwrap_or(EditorialStage::PacketReady),
        attempt: state
            .scenes
            .get(scene_index)
            .map(|scene| scene.attempt)
            .unwrap_or(0),
        action,
        prompt_template_id,
        prompt_version,
        prompt_hash,
        context_packet_hash,
        packet_content_hash: state
            .scenes
            .get(scene_index)
            .map(|scene| scene.source_packet_sha256.clone())
            .unwrap_or_default(),
        tokenizer_id,
        tokenizer_version,
        source_snapshot_hash: state
            .scenes
            .get(scene_index)
            .map(|scene| scene.source_packet_sha256.clone())
            .unwrap_or_default(),
        parent_text_sha256,
        candidate_text_sha256,
        judge_order,
        outcome,
        duration_millis,
        usage_known: actual.is_some(),
        actual_tokens: actual.map(|usage| usage.tokens),
        reserved_tokens: reserved.tokens,
        actual_usd_micros: actual.map(|usage| usage.usd_micros),
        reserved_usd_micros: reserved.usd_micros,
        model_call_ids,
        generation_seed: state.settings.generation_seed,
        recorded_at_millis: now_millis,
    };
    state.record_attribution(event.clone());
    if let Some(scene) = state.scenes.get_mut(scene_index) {
        scene.attribution_event_ids.push(event_id);
    }
    Ok(event)
}

/// Retrieve the immutable packet for a pending host task. Refuses submitted
/// tasks, stale hashes, wrong branches, and missing reservations.
pub fn retrieve_editorial_packet(
    state: &EditorialControllerStateV1,
    task_id: &str,
    expected_packet_hash: &str,
) -> Result<DraftingPacketV1, PacketRetrievalError> {
    let scene = state
        .scenes
        .iter()
        .find(|scene| {
            scene
                .pending_host_task
                .as_ref()
                .is_some_and(|task| task.task_id == task_id)
        })
        .ok_or_else(|| PacketRetrievalError::TaskNotFound(task_id.into()))?;
    retrieve_host_task_packet(scene, task_id, expected_packet_hash, &state.branch_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drafting::DraftTokenizer;
    use crate::drafting::tests::{VerifiedFixtureTokenizer, fixture};

    fn settings() -> ResolvedEditorialSettingsV1 {
        let mut settings = ResolvedEditorialSettingsV1::critique_v1_default();
        settings.pricing = Some(EditorialPricingV1 {
            input_usd_micros_per_million: 0,
            output_usd_micros_per_million: 0,
        });
        settings
    }

    fn controller(path: &Path) -> EditorialControllerStateV1 {
        let packet = fixture();
        let mut state = EditorialControllerStateV1::new(
            "run:1".into(),
            packet.project_id.clone(),
            packet.branch_id.clone(),
            settings(),
        );
        let scene = new_editorial_scene_state(
            1,
            1,
            Some("scene:1".into()),
            "packet-hash".into(),
            "source-hash".into(),
            EditorialRouteAvailability::all_available(),
            &settings(),
        );
        state.scenes.push(scene);
        save_editorial_controller(path, &state).expect("save controller");
        state
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "spindle-editorial-{}-{}-{}.json",
            std::process::id(),
            nanos,
            name
        ))
    }

    #[test]
    fn host_task_snapshots_packet_and_admits_reservation() {
        let path = temp_path("host-task");
        let packet = fixture();
        let mut state = controller(&path);
        let task =
            prepare_host_editorial_task(&mut state, 0, "task:1", "packet-hash", &packet, None, 10)
                .expect("prepare task");
        assert_eq!(task.kind, HostEditorialTaskKind::Draft);
        assert!(task.is_pending());
        assert_eq!(task.packet, packet);
        assert_eq!(
            state.scenes[0]
                .find_pending_task()
                .map(|task| task.task_id.as_str()),
            Some("task:1")
        );
        assert_eq!(state.scenes[0].ledger.scene_used().calls, 1);
        let retrieved = retrieve_editorial_packet(&state, "task:1", "packet-hash")
            .expect("pending task packet retrieves");
        assert_eq!(retrieved, packet);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_host_usage_keeps_full_reservation_charged() {
        let path = temp_path("unknown-usage");
        let packet = fixture();
        let mut state = controller(&path);
        prepare_host_editorial_task(&mut state, 0, "task:1", "packet-hash", &packet, None, 10)
            .expect("prepare task");
        let outcome = apply_host_editorial_submission(
            &mut state,
            &HostEditorialSubmissionV1 {
                task_id: "task:1".into(),
                packet_hash: "packet-hash".into(),
                text: "A clean plain scene draft.".into(),
                usage: None,
                truncated: false,
            },
            20,
        )
        .expect("submit host draft");
        assert!(matches!(
            outcome.0,
            HostSubmissionOutcomeV1::DraftAccepted { .. }
        ));
        let reservation = &state.scenes[0].ledger.scene_reservations[0];
        assert!(!reservation.usage_known);
        assert!(reservation.actual.is_none());
        assert_eq!(state.scenes[0].ledger.scene_used().tokens, 15_512);
        assert_eq!(state.scenes[0].stage, EditorialStage::Drafted);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn submitted_task_packet_cannot_be_retrieved_again() {
        let path = temp_path("retrieval");
        let packet = fixture();
        let mut state = controller(&path);
        prepare_host_editorial_task(&mut state, 0, "task:1", "packet-hash", &packet, None, 10)
            .expect("prepare task");
        apply_host_editorial_submission(
            &mut state,
            &HostEditorialSubmissionV1 {
                task_id: "task:1".into(),
                packet_hash: "packet-hash".into(),
                text: "A clean plain scene draft.".into(),
                usage: Some(EditorialBudgetUsage {
                    calls: 1,
                    tokens: 100,
                    millis: 5,
                    usd_micros: 0,
                }),
                truncated: false,
            },
            20,
        )
        .expect("submit");
        assert!(matches!(
            retrieve_editorial_packet(&state, "task:1", "packet-hash"),
            Err(PacketRetrievalError::TaskAlreadySubmitted(_))
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn revision_submission_runs_deterministic_checks_before_judge() {
        let path = temp_path("revision");
        let packet = fixture();
        let mut state = controller(&path);
        // Draft first so the scene reaches DeterministicChecked/Critiqued.
        state.scenes[0].stage = EditorialStage::Critiqued;
        state.scenes[0].incumbent = Some(EditorialTextRefV1 {
            text_sha256: editorial_sha256_hex(b"Original prose."),
            generation_id: Some("generation:1".into()),
            agent_id: Some("host".into()),
        });
        prepare_host_editorial_task(
            &mut state,
            0,
            "task:2",
            "packet-hash",
            &packet,
            Some("Original prose."),
            10,
        )
        .expect("prepare revision task");
        let (outcome, comparison) = apply_host_editorial_submission(
            &mut state,
            &HostEditorialSubmissionV1 {
                task_id: "task:2".into(),
                packet_hash: "packet-hash".into(),
                text: "Revised plain prose.".into(),
                usage: None,
                truncated: false,
            },
            20,
        )
        .expect("submit revision");
        assert!(matches!(
            outcome,
            HostSubmissionOutcomeV1::CandidateReady { .. }
        ));
        assert!(comparison.is_some());
        assert_eq!(state.scenes[0].stage, EditorialStage::CandidateReady);
        assert_eq!(
            state.scenes[0]
                .candidate
                .as_ref()
                .map(|reference| reference.text_sha256.as_str()),
            Some(editorial_sha256_hex(b"Revised plain prose.").as_str())
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn restart_preserves_ledger_caps_and_restart_cannot_reset_counters() {
        let path = temp_path("restart");
        let packet = fixture();
        {
            let mut state = controller(&path);
            state.settings.scene_budget.max_calls = 1;
            state.scenes[0].ledger.scene_caps.max_calls = 1;
            prepare_host_editorial_task(&mut state, 0, "task:1", "packet-hash", &packet, None, 10)
                .expect("prepare task");
            save_editorial_controller(&path, &state).expect("save");
        }
        let mut reloaded = load_editorial_controller(&path).expect("reload");
        assert_eq!(reloaded.scenes[0].ledger.scene_used().calls, 1);
        let denial = admit_editorial_action_for_run(
            &mut reloaded.scenes[0],
            &reloaded.settings,
            &reloaded.run_id.clone(),
            &reloaded.branch_id.clone(),
            EditorialActionKind::Critique,
            1,
            30,
            false,
        )
        .expect_err("the persisted call cap must survive restart");
        assert!(
            matches!(
                denial,
                BudgetDenial::SceneCalls {
                    used: 1,
                    cap: 1,
                    ..
                }
            ),
            "unexpected denial after restart: {denial:?}"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn concurrent_controller_updates_fail_busy_instead_of_double_admitting() {
        use std::sync::{Arc, Barrier};
        let path = temp_path("concurrency");
        let _ = controller(&path);
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for index in 0..2 {
            let path = path.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                update_editorial_controller(&path, |state| {
                    let packet = fixture();
                    prepare_host_editorial_task(
                        state,
                        0,
                        &format!("task:{index}"),
                        "packet-hash",
                        &packet,
                        None,
                        10,
                    )?;
                    Ok(())
                })
                .is_ok()
            }));
        }
        let results: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            results.iter().filter(|ok| **ok).count(),
            1,
            "exactly one concurrent owner should win: {results:?}"
        );
        let reloaded = load_editorial_controller(&path).expect("reload");
        assert_eq!(reloaded.scenes[0].ledger.scene_reservations.len(), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn attribution_is_prose_free_and_links_model_call_ids() {
        let path = temp_path("attribution");
        let mut state = controller(&path);
        let reserved = state.scenes[0].ledger.scene_used();
        let event = record_model_attribution(
            &mut state,
            0,
            EditorialActionKind::Draft,
            TEMPLATE_PACKET_DRAFT.into(),
            TEMPLATE_VERSION_V1.into(),
            "prompt-hash".into(),
            "context-hash".into(),
            Some(("fixture-tokenizer".into(), "1".into())),
            None,
            Some("candidate-hash".into()),
            None,
            "drafted".into(),
            120,
            Some(EditorialBudgetUsage {
                calls: 1,
                tokens: 1_000,
                millis: 120,
                usd_micros: 0,
            }),
            reserved,
            vec!["model_call:1".into()],
            30,
        )
        .expect("record attribution");
        assert_eq!(event.model_call_ids, vec!["model_call:1"]);
        assert_eq!(event.actual_tokens, Some(1_000));
        assert_eq!(event.reserved_tokens, 0);
        assert_eq!(state.attribution.len(), 1);
        assert!(
            !serde_json::to_string(&event)
                .expect("serialize")
                .contains("scene draft")
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn verified_fixture_tokenizer_identity_is_real_for_tests() {
        let tokenizer = VerifiedFixtureTokenizer;
        assert_eq!(tokenizer.id(), "fixture-unicode");
        assert!(tokenizer.supports_model("fixture"));
    }
}
