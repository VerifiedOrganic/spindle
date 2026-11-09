use super::*;
use crate::models::StoryPlacement;

fn placement(revision: &str, text: &str) -> ClaimBindingV1 {
    ClaimBindingV1 {
        schema_version: CLAIM_SCHEMA_VERSION,
        project_id: "project:p".into(),
        branch_id: "branch:main".into(),
        scene_id: Some("scene:1".into()),
        revision_id: revision.into(),
        accepted_text_sha256: exact_text_sha256(text),
        position: StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(1),
            note: None,
        },
        extraction_run_id: "run:1".into(),
        model: "test-model".into(),
        agent_id: None,
        model_call_id: None,
        prompt_version: "canon-extract-v1".into(),
    }
}

fn resolved(id: &str) -> ClaimEntity {
    ClaimEntity::Resolved { id: id.into() }
}

fn claim(
    text: &str,
    subject: &str,
    assertion: ClaimAssertion,
    modality: ClaimModality,
    confidence: f64,
) -> ClaimV1 {
    ClaimV1 {
        binding: placement("revision:1", text),
        source: ClaimSource::Independent,
        proposed: ProposedClaimV1 {
            claim_id: "claim:1".into(),
            subject: resolved(subject),
            assertion,
            within_scene_order: None,
            in_world_time: None,
            modality,
            temporal_scope: TemporalScope::Current,
            speaker: None,
            experiencer: None,
            confidence: Some(confidence),
            evidence: vec![ClaimEvidence {
                start_byte: 0,
                end_byte: text.len(),
                quote: text.into(),
            }],
        },
    }
}

fn calibration(class: CanonClaimClassV1) -> CanonCalibrationProfileV1 {
    CanonCalibrationProfileV1 {
        class,
        model: "test-model".into(),
        prompt_version: "canon-extract-v1".into(),
        accepted_claims: 400,
        correct_claims: 400,
    }
}

fn context(text: &str, profiles: Vec<CanonCalibrationProfileV1>) -> CanonPolicyContextV1 {
    let mut accepted_revisions = BTreeMap::new();
    accepted_revisions.insert(
        "revision:1".into(),
        AcceptedClaimRevisionV1 {
            text: text.into(),
            accepted_text_sha256: exact_text_sha256(text),
        },
    );
    CanonPolicyContextV1 {
        accepted_revisions,
        ratified_state: None,
        package_agreed_fingerprints: BTreeSet::new(),
        secret_linked_propositions: BTreeSet::new(),
        existing_promise_thread_ids: BTreeSet::new(),
        calibrations: profiles,
        config: CanonPolicyConfigV1::default(),
    }
}

fn agreed_context(claim: &ClaimV1, class: CanonClaimClassV1) -> CanonPolicyContextV1 {
    let text = claim.proposed.evidence[0].quote.clone();
    let mut context = context(&text, vec![calibration(class)]);
    context
        .package_agreed_fingerprints
        .insert(semantic_claim_fingerprint(claim));
    context
}

fn event(subject: &str, assertion: ClaimAssertion, decision_seq: i64) -> CanonEventV1 {
    let position = CanonPositionV1::new(1, 1, 1).unwrap();
    let payload = CanonEventPayloadV1::Assertion {
        assertion,
        modality: ClaimModality::Narration,
        temporal_scope: TemporalScope::Current,
        speaker: None,
        experiencer: None,
    };
    CanonEventV1 {
        event_id: format!("canon_event:{decision_seq}"),
        schema_version: CANON_EVENT_SCHEMA_VERSION,
        project_id: "project:p".into(),
        branch_id: "branch:main".into(),
        decision_id: format!("canon_decision:{decision_seq}"),
        decision_seq,
        append_seq: decision_seq,
        tier: CanonTier::Ratified,
        policy_action: CanonPolicyAction::Ratify,
        subject: resolved(subject),
        predicate_family: payload.family(),
        predicate: payload.predicate_and_operation().0.into(),
        operation: payload.predicate_and_operation().1.into(),
        payload,
        effective_position: Some(position),
        within_scene_order: None,
        in_world_time: None,
        scene_id: Some("scene:1".into()),
        source_revision_id: "revision:1".into(),
        source_claim_ids: vec!["claim:prior".into()],
        semantic_fingerprint: format!("fingerprint:{decision_seq}"),
        idempotency_key: format!("idempotency:{decision_seq}"),
        evidence_sha256: exact_text_sha256("evidence"),
        provenance: CanonProvenanceKind::Extracted,
        supersedes_event_id: None,
        withdraws_event_id: None,
        legacy_record_scope: None,
        merge_provenance: None,
        created_at_micros: 0,
    }
}

fn ratified_state(assertion: ClaimAssertion) -> CanonStateViewV1 {
    state_at(
        "branch:main",
        &["branch:main".to_string()],
        &[event("character:veyra", assertion, 1)],
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap()
}

#[test]
fn evidence_verification_requires_exact_bytes_and_hash() {
    let text = "Veyra walks into the market.";
    let claim = claim(
        text,
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: resolved("location:market"),
        },
        ClaimModality::Narration,
        0.99,
    );
    let mut accepted = BTreeMap::new();
    accepted.insert(
        "revision:1".to_string(),
        AcceptedClaimRevisionV1 {
            text: text.into(),
            accepted_text_sha256: exact_text_sha256(text),
        },
    );
    let verified = verify_claim_batch_evidence(std::slice::from_ref(&claim), &accepted);
    assert!(verified[0].verified);

    let mut missing = claim.clone();
    missing.binding.revision_id = "revision:missing".into();
    let failure = verify_claim_batch_evidence(&[missing], &accepted);
    assert!(!failure[0].verified);
    assert!(matches!(
        failure[0].failures[0],
        ClaimEvidenceFailureV1::MissingRevision { .. }
    ));

    let mut wrong_quote = claim.clone();
    wrong_quote.proposed.evidence[0].quote = "not the scene".into();
    let failure = verify_claim_batch_evidence(&[wrong_quote], &accepted);
    assert!(!failure[0].verified);
    assert!(matches!(
        failure[0].failures[0],
        ClaimEvidenceFailureV1::QuoteMismatch { .. }
    ));
}

#[test]
fn calibrated_movement_auto_ratifies_only_when_all_conditions_pass() {
    let text = "Veyra walks into the market.";
    let claim = claim(
        text,
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: resolved("location:market"),
        },
        ClaimModality::Narration,
        0.99,
    );
    let context = agreed_context(&claim, CanonClaimClassV1::ExplicitMovement);
    let evaluation = evaluate_canon_policy(
        &claim,
        CanonPolicyProvenanceV1::IndependentExtracted,
        false,
        &context,
    )
    .unwrap();
    assert_eq!(evaluation.disposition, CanonPolicyDispositionV1::AutoRatify);
    assert_eq!(evaluation.tier, CanonTier::Ratified);
    assert!(evaluation.idempotency_key.len() == 64);
    assert!(evaluation.precondition_summary.contains("evidence=pass"));
}

#[test]
fn uncalibrated_class_stages_even_with_perfect_confidence_and_agreement() {
    let text = "Veyra walks into the market.";
    let claim = claim(
        text,
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: resolved("location:market"),
        },
        ClaimModality::Narration,
        1.0,
    );
    let mut context = context(
        text,
        vec![CanonCalibrationProfileV1 {
            class: CanonClaimClassV1::ExplicitMovement,
            model: "test-model".into(),
            prompt_version: "canon-extract-v1".into(),
            accepted_claims: 99,
            correct_claims: 99,
        }],
    );
    context
        .package_agreed_fingerprints
        .insert(semantic_claim_fingerprint(&claim));
    let evaluation = evaluate_canon_policy(
        &claim,
        CanonPolicyProvenanceV1::IndependentExtracted,
        false,
        &context,
    )
    .unwrap();
    assert_eq!(
        evaluation.disposition,
        CanonPolicyDispositionV1::StageProvisional
    );
    assert!(!evaluation.calibrated);
}

#[test]
fn no_package_agreement_or_non_narration_modality_stages() {
    let text = "Veyra might be at the market.";
    let claim = claim(
        text,
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: resolved("location:market"),
        },
        ClaimModality::Speculation,
        0.99,
    );
    let context = context(text, vec![calibration(CanonClaimClassV1::ExplicitMovement)]);
    let evaluation = evaluate_canon_policy(
        &claim,
        CanonPolicyProvenanceV1::IndependentExtracted,
        false,
        &context,
    )
    .unwrap();
    assert_eq!(
        evaluation.disposition,
        CanonPolicyDispositionV1::StageProvisional
    );
    assert!(
        evaluation
            .preconditions
            .iter()
            .any(|precondition| precondition.name == "modality" && !precondition.passed)
    );
}

#[test]
fn deaths_secret_disclosures_and_promise_payoffs_never_auto_ratify() {
    let text = "Veyra dies and the secret is spoken; the promise is paid.";
    let cases = [
        claim(
            text,
            "character:veyra",
            ClaimAssertion::LifeStatus {
                status: LifeStatus::Dead,
            },
            ClaimModality::Narration,
            1.0,
        ),
        claim(
            text,
            "character:veyra",
            ClaimAssertion::Knowledge {
                operation: KnowledgeOperation::Disclose,
                proposition: resolved("proposition:secret"),
                audience: vec![resolved("character:other")],
                source: None,
            },
            ClaimModality::Narration,
            1.0,
        ),
        claim(
            text,
            "character:veyra",
            ClaimAssertion::Promise {
                operation: PromiseOperation::Payoff,
                thread: resolved("thread:promise"),
            },
            ClaimModality::Narration,
            1.0,
        ),
    ];
    for (index, claim) in cases.iter().enumerate() {
        let class = match index {
            0 => CanonClaimClassV1::DeathOrResurrection,
            1 => CanonClaimClassV1::SecretDisclosure,
            _ => CanonClaimClassV1::PromisePayoffOrAbandonment,
        };
        let context = agreed_context(claim, class);
        let evaluation = evaluate_canon_policy(
            claim,
            CanonPolicyProvenanceV1::IndependentExtracted,
            index == 1,
            &context,
        )
        .unwrap();
        assert_eq!(
            evaluation.disposition,
            CanonPolicyDispositionV1::StageProvisional,
            "case {index}"
        );
        assert_eq!(evaluation.tier, CanonTier::Provisional);
        assert!(evaluation.class.is_v1_high_impact());
    }
}

#[test]
fn unresolved_entity_and_contradiction_block_auto_ratification() {
    let text = "Veyra wears a blue cloak.";
    let unresolved = ClaimV1 {
        binding: placement("revision:1", text),
        source: ClaimSource::Independent,
        proposed: ProposedClaimV1 {
            claim_id: "claim:unresolved".into(),
            subject: ClaimEntity::Unresolved {
                label: "the stranger".into(),
            },
            assertion: ClaimAssertion::LocatedAt {
                location: resolved("location:market"),
            },
            within_scene_order: None,
            in_world_time: None,
            modality: ClaimModality::Narration,
            temporal_scope: TemporalScope::Current,
            speaker: None,
            experiencer: None,
            confidence: Some(1.0),
            evidence: vec![ClaimEvidence {
                start_byte: 0,
                end_byte: text.len(),
                quote: text.into(),
            }],
        },
    };
    let context = context(text, vec![calibration(CanonClaimClassV1::ExplicitMovement)]);
    let evaluation = evaluate_canon_policy(
        &unresolved,
        CanonPolicyProvenanceV1::IndependentExtracted,
        false,
        &context,
    )
    .unwrap();
    assert_eq!(
        evaluation.disposition,
        CanonPolicyDispositionV1::StageProvisional
    );
    assert_eq!(evaluation.class, CanonClaimClassV1::AmbiguousOrUnsupported);

    let contradiction_text = "Veyra's cloak is brown.";
    let candidate = claim(
        contradiction_text,
        "character:veyra",
        ClaimAssertion::Attribute {
            operation: AttributeOperation::Set,
            key: "cloak_color".into(),
            value: ClaimScalar::Text {
                value: "brown".into(),
            },
            nature: AttributeNature::Evolving,
        },
        ClaimModality::Narration,
        1.0,
    );
    let mut context = agreed_context(&candidate, CanonClaimClassV1::EvolvingAttribute);
    context.ratified_state = Some(ratified_state(ClaimAssertion::Attribute {
        operation: AttributeOperation::Set,
        key: "cloak_color".into(),
        value: ClaimScalar::Text {
            value: "blue".into(),
        },
        nature: AttributeNature::Evolving,
    }));
    let evaluation = evaluate_canon_policy(
        &candidate,
        CanonPolicyProvenanceV1::IndependentExtracted,
        false,
        &context,
    )
    .unwrap();
    assert_eq!(
        evaluation.disposition,
        CanonPolicyDispositionV1::StageProvisional
    );
    assert!(
        evaluation
            .contradictions
            .iter()
            .any(|contradiction| { contradiction.severity == CanonContradictionSeverityV1::Error })
    );
}

#[test]
fn operator_declared_ratifies_without_package_agreement_but_not_death() {
    let text = "Veyra walks into the market.";
    let candidate = claim(
        text,
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: resolved("location:market"),
        },
        ClaimModality::Narration,
        1.0,
    );
    let context = context(text, vec![calibration(CanonClaimClassV1::ExplicitMovement)]);
    let evaluation = evaluate_canon_policy(
        &candidate,
        CanonPolicyProvenanceV1::OperatorDeclared,
        false,
        &context,
    )
    .unwrap();
    assert_eq!(evaluation.disposition, CanonPolicyDispositionV1::Ratify);
    assert_eq!(evaluation.tier, CanonTier::Ratified);

    let death = claim(
        text,
        "character:veyra",
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        },
        ClaimModality::Narration,
        1.0,
    );
    let evaluation = evaluate_canon_policy(
        &death,
        CanonPolicyProvenanceV1::OperatorDeclared,
        false,
        &context,
    )
    .unwrap();
    assert_eq!(
        evaluation.disposition,
        CanonPolicyDispositionV1::StageProvisional
    );
}

#[test]
fn calibration_gate_requires_sample_point_precision_and_lower_bound() {
    let config = CanonPolicyConfigV1::default();
    let insufficient_sample = CanonCalibrationProfileV1 {
        class: CanonClaimClassV1::ExplicitMovement,
        model: "m".into(),
        prompt_version: "p".into(),
        accepted_claims: 99,
        correct_claims: 99,
    };
    assert!(!insufficient_sample.meets_gate(&config));

    let too_imprecise = CanonCalibrationProfileV1 {
        class: CanonClaimClassV1::ExplicitMovement,
        model: "m".into(),
        prompt_version: "p".into(),
        accepted_claims: 400,
        correct_claims: 395,
    };
    assert!(!too_imprecise.meets_gate(&config));

    let calibrated = CanonCalibrationProfileV1 {
        class: CanonClaimClassV1::ExplicitMovement,
        model: "m".into(),
        prompt_version: "p".into(),
        accepted_claims: 400,
        correct_claims: 400,
    };
    assert!(calibrated.meets_gate(&config));
    assert!(calibrated.precision_lower_bound(config.calibration_z) > 0.98);
}

#[test]
fn pending_work_never_reports_continuity_clean() {
    let pending = evaluate_canon_pending_work(&CanonPendingWorkInputV1 {
        project_id: "project:p".into(),
        branch_id: "branch:main".into(),
        pending_scene_count: 1,
        disputed_high_impact_prerequisites: 0,
        failed_extraction_scene_count: 0,
        retries_remaining: 2,
    });
    assert!(!pending.continuity_clean());
    assert!(pending.next_scene_has_pending_work);

    let blocked = evaluate_canon_pending_work(&CanonPendingWorkInputV1 {
        project_id: "project:p".into(),
        branch_id: "branch:main".into(),
        pending_scene_count: 4,
        disputed_high_impact_prerequisites: 1,
        failed_extraction_scene_count: 1,
        retries_remaining: 0,
    });
    assert!(blocked.pause_at_next_checkpoint);
    assert!(!blocked.continuity_clean());

    let clean = evaluate_canon_pending_work(&CanonPendingWorkInputV1 {
        project_id: "project:p".into(),
        branch_id: "branch:main".into(),
        pending_scene_count: 0,
        disputed_high_impact_prerequisites: 0,
        failed_extraction_scene_count: 0,
        retries_remaining: 2,
    });
    assert!(clean.continuity_clean());
}

#[test]
fn cutover_is_gated_by_readiness_and_reversible() {
    let readiness = ProjectCutoverReadinessV1 {
        project_id: "project:p".into(),
        migration_version: 1,
        parity_clean: true,
        quarantined_count: 0,
        unmapped_legacy_row_count: 0,
        cutover_allowed: true,
        blocking_reasons: Vec::new(),
        branch_reports: Vec::new(),
    };
    let legacy =
        evaluate_canon_cutover_transition(None, CanonCutoverModeV1::Legacy, &readiness, 0, 1)
            .unwrap();
    assert_eq!(legacy.mode, CanonCutoverModeV1::Legacy);
    let dual = evaluate_canon_cutover_transition(
        Some(&legacy),
        CanonCutoverModeV1::DualWrite,
        &readiness,
        7,
        2,
    )
    .unwrap();
    assert_eq!(dual.mode, CanonCutoverModeV1::DualWrite);
    assert_eq!(dual.activated_decision_watermark, 7);
    let rollback = evaluate_canon_cutover_transition(
        Some(&dual),
        CanonCutoverModeV1::Legacy,
        &readiness,
        8,
        3,
    )
    .unwrap();
    assert_eq!(rollback.mode, CanonCutoverModeV1::Legacy);

    let blocked = ProjectCutoverReadinessV1 {
        cutover_allowed: false,
        blocking_reasons: vec!["shadow read parity differs".into()],
        ..readiness
    };
    let error =
        evaluate_canon_cutover_transition(None, CanonCutoverModeV1::DualWrite, &blocked, 0, 1)
            .unwrap_err();
    assert!(error.detail.contains("parity differs"));
}

#[test]
fn legacy_unverified_findings_are_warning_only_without_corroboration() {
    assert!(!can_trigger_error_finding(
        CanonProvenanceKind::LegacyUnverified,
        false
    ));
    assert!(can_trigger_error_finding(
        CanonProvenanceKind::LegacyUnverified,
        true
    ));
    assert!(can_trigger_error_finding(
        CanonProvenanceKind::Extracted,
        false
    ));
    let downgraded = classify_legacy_finding(LegacyCanonFindingV1 {
        severity: "error".into(),
        check_type: "canon_consistency".into(),
        message: "legacy says blue".into(),
        provenance: CanonProvenanceKind::LegacyUnverified,
        corroborated: false,
    });
    assert_eq!(downgraded.severity, "warning");
    assert!(downgraded.message.contains("requires corroboration"));
}

#[test]
fn revocation_plan_withdraws_targets_and_invalidates_dependents() {
    let target = event(
        "character:veyra",
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        },
        1,
    );
    let mut dependent = event(
        "character:veyra",
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        },
        2,
    );
    dependent.source_claim_ids = vec!["claim:prior".into()];
    let plan = plan_canon_batch_revocation(
        std::slice::from_ref(&target),
        &[target.clone(), dependent.clone()],
        &[],
        "branch:main",
        "revision:1",
    );
    assert_eq!(plan.withdraw_event_drafts.len(), 1);
    assert_eq!(
        plan.withdraw_event_drafts[0].withdraws_event_id.as_deref(),
        Some(target.event_id.as_str())
    );
    assert!(
        plan.invalidated_dependent_event_ids
            .contains(&dependent.event_id)
    );
    assert!(
        plan.invalidated_summary_keys
            .contains(&"scene:scene:1".into())
    );
}
