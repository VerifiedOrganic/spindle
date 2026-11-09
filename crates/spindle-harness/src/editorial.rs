//! Operator-facing editorial controller status, dry-run actions, and the
//! local hybrid host-draft submission seam. Read-only unless `resume
//! --execute-one` explicitly accepts a host draft.

use anyhow::{Context, Result};
use spindle_core::drafting::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::state::HarnessState;

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

pub fn controller_path(state: &HarnessState) -> Option<PathBuf> {
    state
        .editorial_controller_path
        .as_deref()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

pub fn load_editorial_controller(path: &Path) -> Result<EditorialControllerStateV1> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read editorial controller {}", path.display()))?;
    let state: EditorialControllerStateV1 = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse editorial controller {}", path.display()))?;
    state
        .validate()
        .map_err(anyhow::Error::msg)
        .with_context(|| format!("invalid editorial controller {}", path.display()))?;
    Ok(state)
}

pub fn save_editorial_controller(path: &Path, state: &EditorialControllerStateV1) -> Result<()> {
    state.validate().map_err(anyhow::Error::msg)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create editorial controller directory {}",
                parent.display()
            )
        })?;
    }
    let temporary = path.with_extension("tmp");
    let json = serde_json::to_string_pretty(state)?;
    fs::write(&temporary, json)
        .with_context(|| format!("failed to write {}", temporary.display()))?;
    fs::rename(&temporary, path)
        .with_context(|| format!("failed to replace editorial controller {}", path.display()))?;
    Ok(())
}

/// Empty string when the run is legacy, so older state output is unchanged.
pub fn render_editorial_status(state: &HarnessState, verbose: bool) -> String {
    let Some(settings) = &state.resolved_editorial else {
        return String::new();
    };
    let mut out = String::new();
    out.push_str(&format!(
        "Editorial policy: {} ({})\n",
        policy_label(settings),
        mode_label(settings)
    ));
    out.push_str(&format!(
        "Editorial versions: packet={} rubric={} critic={} rounds={} line_edit={} chapter_pass={}\n",
        settings.packet_version,
        settings.rubric_version,
        settings.critic_rubric_version,
        settings.max_rounds,
        settings.line_edit_enabled,
        settings.chapter_pass_enabled
    ));
    let controller = controller_path(state);
    let Some(path) = controller else {
        out.push_str("Editorial controller: not prepared\n");
        return out;
    };
    out.push_str(&format!("Editorial controller: {}\n", path.display()));
    let Ok(controller) = load_editorial_controller(&path) else {
        out.push_str("Editorial controller: unreadable; run verify for details\n");
        return out;
    };
    for scene in &controller.scenes {
        out.push_str(&format!(
            "Editorial scene {}.{}: stage={} incumbent={} candidate={} critic={} judge={} parked={}\n",
            scene.chapter_number,
            scene.scene_order,
            scene.stage.as_str(),
            short_hash(scene.incumbent.as_ref().map(|reference| &reference.text_sha256)),
            short_hash(scene.candidate.as_ref().map(|reference| &reference.text_sha256)),
            scene
                .critic
                .as_ref()
                .map(|critic| format!("{}/{}d", critic.status, critic.directive_count))
                .unwrap_or_else(|| "none".into()),
            scene
                .judge
                .as_ref()
                .map(|judge| {
                    format!(
                        "{}{}",
                        judge.verdict,
                        if judge.accepted_revision { "/accepted" } else { "/retained" }
                    )
                })
                .unwrap_or_else(|| "none".into()),
            scene.parked_issues.len()
        ));
        if let Some(task) = scene.find_pending_task() {
            out.push_str(&format!(
                "  pending host task: {} ({:?})\n",
                task.task_id, task.kind
            ));
        }
        if let Some(pending) = &scene.pending_manual_judgment {
            out.push_str(&format!(
                "  pending manual judgment: {}\n",
                pending.reason.as_str()
            ));
        }
        let remaining = scene.ledger.scene_remaining();
        let run_remaining = scene.ledger.run_remaining();
        out.push_str(&format!(
            "  remaining scene caps: calls={} tokens={} millis={} usd_micros={}\n",
            remaining.max_calls,
            remaining.max_tokens,
            remaining.max_millis,
            remaining.max_usd_micros
        ));
        out.push_str(&format!(
            "  remaining run caps: calls={} tokens={} millis={} usd_micros={}\n",
            run_remaining.max_calls,
            run_remaining.max_tokens,
            run_remaining.max_millis,
            run_remaining.max_usd_micros
        ));
        if let Some(action) = editorial_next_action(scene, settings) {
            out.push_str(&format!(
                "  next editorial action: {} ({}) reserve calls={} tokens={} millis={} usd_micros={}\n",
                action.action.as_str(),
                action.stage.as_str(),
                action.reservation.calls,
                action.reservation.tokens,
                action.reservation.millis,
                action.reservation.usd_micros
            ));
            if let Some(route) = &action.missing_route {
                out.push_str(&format!("  missing route: {route}\n"));
            }
            if action.missing_tokenizer {
                out.push_str("  missing verified tokenizer: yes\n");
            }
            if let Some(task_id) = &action.pending_host_task_id {
                out.push_str(&format!("  awaiting host submission: {task_id}\n"));
            }
        }
        if verbose && !scene.parked_issues.is_empty() {
            out.push_str("  parked issues:\n");
            for issue in &scene.parked_issues {
                out.push_str(&format!("    - {issue}\n"));
            }
        }
    }
    out.push_str(&format!(
        "Editorial attribution events: {}\n",
        controller.attribution.len()
    ));
    out
}

fn policy_label(settings: &ResolvedEditorialSettingsV1) -> &'static str {
    match settings.editorial_policy {
        EditorialPolicy::Legacy => "legacy",
        EditorialPolicy::CritiqueV1 => "critique_v1",
    }
}

fn mode_label(settings: &ResolvedEditorialSettingsV1) -> &'static str {
    match settings.execution_mode {
        EditorialExecutionMode::Hybrid => "hybrid",
        EditorialExecutionMode::Agent => "agent",
    }
}

fn short_hash(hash: Option<&String>) -> String {
    match hash {
        Some(hash) if hash.len() >= 8 => hash[..8].to_string(),
        Some(hash) => hash.clone(),
        None => "-".into(),
    }
}

pub fn parse_host_submission(path: &Path) -> Result<HostEditorialSubmissionV1> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read host submission {}", path.display()))?;
    let submission: HostEditorialSubmissionV1 = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse host submission {}", path.display()))?;
    Ok(submission)
}

/// Accept exactly one hybrid host draft. Revision submissions stay parked for
/// server-side deterministic checks and judging; this seam never skips them.
pub fn accept_host_draft_submission(
    controller: &mut EditorialControllerStateV1,
    submission: &HostEditorialSubmissionV1,
    now: u64,
) -> Result<(HostSubmissionOutcomeV1, Option<EditorialReservationKey>)> {
    let Some((scene_index, task)) =
        controller
            .scenes
            .iter()
            .enumerate()
            .find_map(|(index, scene)| {
                scene
                    .pending_host_task
                    .as_ref()
                    .filter(|task| task.task_id == submission.task_id)
                    .map(|task| (index, task.clone()))
            })
    else {
        return Ok((
            HostSubmissionOutcomeV1::Rejected {
                rejection: HostSubmissionRejection::TaskNotFound,
                message: "host task not found".into(),
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
    if task.kind != HostEditorialTaskKind::Draft {
        anyhow::bail!(
            "revision host submissions require server-side deterministic checks and judging"
        );
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
    let key = EditorialReservationKey {
        run_id: task.run_id.clone(),
        branch_id: task.branch_id.clone(),
        scene_id: task.scene_id.clone(),
        packet_hash: controller.scenes[scene_index].packet_hash.clone(),
        stage: controller.scenes[scene_index].stage,
        attempt: controller.scenes[scene_index].attempt,
        action: EditorialActionKind::Draft,
    };
    if controller.scenes[scene_index]
        .ledger
        .find(&key.idempotency_key())
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
    let text_sha256 = editorial_sha256(submission.text.as_bytes());
    let normalized_output_sha256 =
        editorial_sha256(submission.text.replace("\r\n", "\n").trim().as_bytes());
    let receipt = HostCompletionReceiptV1 {
        task_id: task.task_id.clone(),
        agent_id: "host".into(),
        text_sha256: text_sha256.clone(),
        normalized_output_sha256,
        usage_known: submission.usage.is_some(),
    };
    let scene = &mut controller.scenes[scene_index];
    scene
        .ledger
        .reconcile(&key.idempotency_key(), submission.usage, Vec::new(), now);
    if let Some(pending) = scene.pending_host_task.as_mut() {
        pending.submitted_at_millis = Some(now);
    }
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
        Some(key),
    ))
}

pub fn write_host_submission_artifacts(
    controller_path: &Path,
    submission: &HostEditorialSubmissionV1,
    receipt: &HostCompletionReceiptV1,
) -> Result<PathBuf> {
    let safe_task = submission
        .task_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let text_path = controller_path.with_extension(format!("host-{safe_task}.txt"));
    fs::write(&text_path, &submission.text)
        .with_context(|| format!("failed to write {}", text_path.display()))?;
    let receipt_path = controller_path.with_extension(format!("host-{safe_task}.receipt.json"));
    fs::write(&receipt_path, serde_json::to_string_pretty(receipt)?)
        .with_context(|| format!("failed to write {}", receipt_path.display()))?;
    Ok(text_path)
}

pub fn editorial_sha256(text: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(text);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChapterRange, HarnessSeed, HarnessState};

    fn state_with_editorial(editorial: Option<EditorialRunConfig>) -> HarnessState {
        let seed = HarnessSeed {
            project_id: "project:1".into(),
            book_number: 1,
            range: ChapterRange {
                start_chapter: 1,
                end_chapter: 1,
            },
            checkpoint_interval: 1,
            editorial_directives: Vec::new(),
            mining_policy: None,
            max_revise_attempts: None,
            checkpoint_policy: None,
            replan_policy: None,
            editorial,
            editorial_controller_path: None,
            chapters: Vec::new(),
        };
        HarnessState::from_seed(seed, "branch:1".into())
    }

    #[test]
    fn legacy_state_renders_no_editorial_output() {
        let state = state_with_editorial(None);
        assert!(state.resolved_editorial.is_none());
        assert!(render_editorial_status(&state, true).is_empty());
    }

    #[test]
    fn explicit_legacy_policy_renders_no_editorial_output() {
        let state = state_with_editorial(Some(EditorialRunConfig {
            editorial_policy: Some(EditorialPolicy::Legacy),
            ..Default::default()
        }));
        assert!(state.resolved_editorial.is_none());
        assert!(render_editorial_status(&state, true).is_empty());
    }

    #[test]
    fn critique_policy_resolves_and_renders_settings() {
        let state = state_with_editorial(Some(EditorialRunConfig {
            editorial_policy: Some(EditorialPolicy::CritiqueV1),
            max_rounds: Some(2),
            line_edit_enabled: Some(false),
            ..Default::default()
        }));
        let settings = state.resolved_editorial.as_ref().expect("resolved");
        assert_eq!(settings.max_rounds, 2);
        let rendered = render_editorial_status(&state, false);
        assert!(rendered.contains("Editorial policy: critique_v1 (hybrid)"));
        assert!(rendered.contains("Editorial controller: not prepared"));
    }

    #[test]
    fn host_draft_submission_keeps_unknown_usage_reserved() {
        let mut settings = ResolvedEditorialSettingsV1::critique_v1_default();
        settings.pricing = Some(EditorialPricingV1 {
            input_usd_micros_per_million: 0,
            output_usd_micros_per_million: 0,
        });
        let mut controller = EditorialControllerStateV1::new(
            "run:1".into(),
            "project:1".into(),
            "branch:1".into(),
            settings.clone(),
        );
        let mut scene = new_scene_for_test(&settings);
        let key = EditorialReservationKey {
            run_id: "run:1".into(),
            branch_id: "branch:1".into(),
            scene_id: scene.scene_id.clone(),
            packet_hash: scene.packet_hash.clone(),
            stage: scene.stage,
            attempt: 0,
            action: EditorialActionKind::Draft,
        };
        scene
            .ledger
            .admit(EditorialBudgetReservation {
                reservation_id: "reservation:1".into(),
                idempotency_key: key.idempotency_key(),
                key: key.clone(),
                reserved: EditorialBudgetUsage {
                    calls: 1,
                    tokens: 15_512,
                    millis: 90_000,
                    usd_micros: 0,
                },
                paid_dispatch: false,
                pricing_known: true,
                usage_known: false,
                actual: None,
                model_call_ids: Vec::new(),
                dispatched_at_millis: None,
                dispatched_claims: Vec::new(),
                created_at_millis: 1,
                reconciled_at_millis: None,
            })
            .expect("admit");
        scene.pending_host_task = Some(HostEditorialTaskV1 {
            schema_version: HOST_EDITORIAL_TASK_SCHEMA_VERSION.into(),
            task_id: "task:1".into(),
            kind: HostEditorialTaskKind::Draft,
            run_id: "run:1".into(),
            project_id: "project:1".into(),
            branch_id: "branch:1".into(),
            scene_id: scene.scene_id.clone(),
            packet_hash: scene.packet_hash.clone(),
            source_packet_sha256: "source".into(),
            reservation_id: "reservation:1".into(),
            rating: spindle_core::models::ContentRating::General,
            target_words: TargetWords {
                min: 800,
                preferred: 1000,
                max: 1500,
            },
            packet: empty_packet(),
            incumbent_text: None,
            incumbent_text_sha256: None,
            directives: Vec::new(),
            strengths_to_preserve: Vec::new(),
            max_output_tokens: 3_000,
            created_at_millis: 1,
            submitted_at_millis: None,
        });
        let packet_hash = scene.packet_hash.clone();
        controller.scenes.push(scene);
        let (outcome, _) = accept_host_draft_submission(
            &mut controller,
            &HostEditorialSubmissionV1 {
                task_id: "task:1".into(),
                packet_hash,
                text: "Host prose.".into(),
                usage: None,
                truncated: false,
            },
            5,
        )
        .expect("accept host draft");
        assert!(matches!(
            outcome,
            HostSubmissionOutcomeV1::DraftAccepted { .. }
        ));
        assert_eq!(controller.scenes[0].stage, EditorialStage::Drafted);
        assert!(!controller.scenes[0].ledger.scene_reservations[0].usage_known);
        assert_eq!(
            controller.scenes[0].ledger.scene_used().tokens,
            15_512,
            "unknown usage keeps the full reservation charged"
        );
    }

    fn empty_packet() -> DraftingPacketV1 {
        // The submission path only inspects packet identity for drafts.
        serde_json::from_value(serde_json::json!({
            "schema_version": "drafting_packet_v1",
            "packet_id": "packet:1",
            "project_id": "project:1",
            "branch_id": "branch:1",
            "run_id": null,
            "scene_id": null,
            "scene_revision": null,
            "canon_snapshot_hash": "snapshot",
            "plan_revision": "1",
            "source_versions": [],
            "scene_card": {
                "source": {"branch_id": "branch:1", "source_id": "plan", "revision_hash": "h"},
                "pov": {
                    "character_id": "pov",
                    "display_name": "Pov",
                    "person": "third",
                    "tense": "past",
                    "source": {"branch_id": "branch:1", "source_id": "plan", "revision_hash": "h"}
                },
                "goal": "goal",
                "opposition": "opposition",
                "turn": "turn",
                "intended_outcome": "outcome",
                "value_shift": {"axis": "hope", "before": "low", "after": "high"},
                "emotion": {"entry": "calm", "exit": "tense"},
                "chapter": {
                    "book_number": 1,
                    "number": 1,
                    "scene_index": 1,
                    "planned_scene_count": null,
                    "is_closer": false
                },
                "target_words": {"min": 800, "preferred": 1000, "max": 1500},
                "location": {"id": "loc", "name": "Loc"},
                "story_clock": "dawn",
                "rating": "General",
                "tone": null,
                "participants": [],
                "required_beats": [],
                "promises_due": [],
                "prohibited_outcomes": []
            },
            "hard_constraints": [],
            "continuity": {
                "character_states": [],
                "knowledge_boundaries": [],
                "facts": [],
                "prior_scene_tail": null,
                "story_so_far": [],
                "open_threads": []
            },
            "voice": {"rules": [], "exemplars": []},
            "editorial_memory": {
                "recent_openings": [],
                "recent_endings": [],
                "recurring_images": [],
                "repeated_phrases": [],
                "pacing_history": []
            },
            "craft_rubric": {"version": "scene-craft-v1", "text": "rubric"},
            "research_evidence": [],
            "assembly": null
        }))
        .expect("packet fixture")
    }

    pub(crate) fn new_scene_for_test(
        settings: &ResolvedEditorialSettingsV1,
    ) -> EditorialSceneStateV1 {
        EditorialSceneStateV1 {
            chapter_number: 1,
            scene_order: 1,
            scene_id: Some("scene:1".into()),
            packet_hash: "packet-hash".into(),
            source_packet_sha256: "source-sha".into(),
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
            availability: EditorialRouteAvailability::all_available(),
            ledger: EditorialBudgetLedgerV1::new(settings.scene_budget, settings.run_budget),
            attribution_event_ids: Vec::new(),
            line_edit_attempted: false,
            extracted: false,
        }
    }
}
