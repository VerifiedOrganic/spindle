use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn binding(text: &str) -> ClaimBindingV1 {
    ClaimBindingV1 {
        schema_version: 1,
        project_id: "project:p".into(),
        branch_id: "branch:b".into(),
        scene_id: None,
        revision_id: "accepted:r1".into(),
        accepted_text_sha256: exact_text_sha256(text),
        position: crate::models::StoryPlacement {
            book_number: 1,
            chapter_number: 2,
            scene_order: Some(1),
            note: None,
        },
        extraction_run_id: "run:r".into(),
        model: "fixture".into(),
        agent_id: None,
        model_call_id: None,
        prompt_version: EXTRACT_PROMPT_VERSION.into(),
    }
}
fn known() -> BTreeSet<String> {
    ["character:veyra".into()].into()
}
fn proposed(text: &str) -> ProposedClaimV1 {
    ProposedClaimV1 {
        claim_id: "claim:1".into(),
        subject: ClaimEntity::Resolved {
            id: "character:veyra".into(),
        },
        assertion: ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        },
        within_scene_order: None,
        in_world_time: None,
        modality: ClaimModality::Narration,
        temporal_scope: TemporalScope::Current,
        speaker: None,
        experiencer: None,
        confidence: Some(0.99),
        evidence: vec![ClaimEvidence {
            start_byte: 0,
            end_byte: text.len(),
            quote: text.into(),
        }],
    }
}
fn payload(text: &str) -> ExtractionPayloadV1 {
    let mut coverage: BTreeMap<_, _> = CLAIM_FAMILIES
        .into_iter()
        .map(|f| (f, FamilyCoverage::InspectedNone))
        .collect();
    coverage.insert(ClaimFamily::LifeAction, FamilyCoverage::ClaimsPresent);
    ExtractionPayloadV1 {
        schema_version: 1,
        accepted_text_sha256: exact_text_sha256(text),
        summary: "Veyra dies.".into(),
        tone: None,
        coverage,
        claims: vec![proposed(text)],
    }
}
fn report(text: &str) -> ExtractionReportV1 {
    bind_extraction(payload(text), binding(text), text, &known()).unwrap()
}

#[test]
fn evidence_hash_ids_and_family_coverage_are_checked() {
    let text = "Veyra is deceased. é";
    assert_eq!(report(text).claims.len(), 1);
    let mut broken = payload(text);
    broken.claims[0].evidence[0].end_byte -= 1; // mid-codepoint UTF-8 boundary
    assert_eq!(
        bind_extraction(broken, binding(text), text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Evidence
    );
    let mut broken = payload(text);
    broken.claims[0].evidence[0].quote = "Veyra is alive.".into();
    assert_eq!(
        bind_extraction(broken, binding(text), text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Evidence
    );
    let mut broken = payload(text);
    broken.claims[0].subject = ClaimEntity::Resolved {
        id: "character:invented".into(),
    };
    assert_eq!(
        bind_extraction(broken, binding(text), text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Entity
    );
    let mut unresolved = payload(text);
    unresolved.claims[0].subject = ClaimEntity::Unresolved {
        label: "the unknown soldier".into(),
    };
    assert!(bind_extraction(unresolved, binding(text), text, &known()).is_ok());
    let mut broken = payload(text);
    broken.coverage.remove(&ClaimFamily::Injury);
    assert_eq!(
        bind_extraction(broken, binding(text), text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Coverage
    );
    let mut broken = payload(text);
    broken
        .coverage
        .insert(ClaimFamily::LifeAction, FamilyCoverage::InspectedNone);
    assert_eq!(
        bind_extraction(broken, binding(text), text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Coverage
    );
    let mut wrong_hash = binding(text);
    wrong_hash.accepted_text_sha256 = exact_text_sha256(text.trim_end_matches('é'));
    assert_eq!(
        bind_extraction(payload(text), wrong_hash, text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Hash
    );
}
#[test]
fn malformed_predicate_and_untyped_payloads_are_rejected() {
    let mut raw = serde_json::to_value(payload("Veyra died.")).unwrap();
    raw["claims"][0]["assertion"] = serde_json::json!({"predicate":"telepathy_blob","anything":{}});
    assert!(serde_json::from_value::<ExtractionPayloadV1>(raw).is_err());
    let mut raw = serde_json::to_value(payload("Veyra died.")).unwrap();
    raw["claims"][0]["assertion"]["anything"] = serde_json::json!({"loose":"data"});
    assert!(serde_json::from_value::<ExtractionPayloadV1>(raw).is_err());
}
#[test]
fn every_family_requires_explicit_coverage_even_when_no_claims() {
    let text = "The wind blew.";
    let mut empty = payload(text);
    empty.claims.clear();
    empty
        .coverage
        .insert(ClaimFamily::LifeAction, FamilyCoverage::InspectedNone);
    assert_eq!(
        bind_extraction(empty.clone(), binding(text), text, &known())
            .unwrap()
            .coverage
            .len(),
        10
    );
    empty.coverage.insert(
        ClaimFamily::Injury,
        FamilyCoverage::Unknown {
            reason: "ambiguous referent".into(),
        },
    );
    assert!(bind_extraction(empty.clone(), binding(text), text, &known()).is_ok());
    empty.coverage.insert(
        ClaimFamily::Injury,
        FamilyCoverage::Unknown { reason: "".into() },
    );
    assert!(bind_extraction(empty, binding(text), text, &known()).is_err());
}
#[test]
fn notes_only_package_is_not_claim_coverage_and_conflicts_keep_both_sources() {
    let text = "Veyra died.";
    let report = report(text);
    let package: DrafterPackageV1 = serde_json::from_value(
        serde_json::json!({"continuity_notes": ["All continuity checked."]}),
    )
    .unwrap();
    assert!(
        serde_json::from_value::<DrafterPackageV1>(serde_json::json!({"unknown_claims": []}))
            .is_err()
    );
    let normalized = normalize_drafter_package(&package, &report.binding, &known());
    assert!(normalized.claims.is_empty());
    assert_eq!(normalized.unsupported_fields, ["continuity_notes"]);
    let diff = compare_claim_sources(&report.claims, &normalized.claims);
    assert_eq!(diff[0].kind, ClaimComparisonKind::ExtractorOnly);
    assert_eq!(diff[0].extractor[0].proposed.evidence[0].quote, text);
    let package = DrafterPackageV1 {
        character_states: vec![crate::models::CharacterStatePatchEntry {
            character_id: "character:veyra".into(),
            changes: crate::models::CharacterStatePatch {
                emotional_state: Default::default(),
                goals: None,
                status: Some(vec!["alive".into()]),
                notes: None,
                source_summary: None,
            },
        }],
        ..Default::default()
    };
    let normalized = normalize_drafter_package(&package, &report.binding, &known());
    let diff = compare_claim_sources(&report.claims, &normalized.claims);
    assert_eq!(diff[0].kind, ClaimComparisonKind::Incompatible);
    assert_eq!(diff[0].extractor[0].source, ClaimSource::Independent);
    assert_eq!(diff[0].drafter[0].source, ClaimSource::Drafter);
    assert!(diff[0].drafter[0].proposed.evidence.is_empty());
    assert!(diff[0].drafter[0].proposed.confidence.is_none());
}
#[test]
fn modality_scope_order_and_revision_are_not_collapsed() {
    let report = report("Veyra said she was dead.");
    for variant in 0..4 {
        let mut competing = report.claims.clone();
        match variant {
            0 => competing[0].proposed.modality = ClaimModality::Belief,
            1 => competing[0].proposed.temporal_scope = TemporalScope::Flashback,
            2 => competing[0].proposed.within_scene_order = Some(2),
            _ => competing[0].binding.revision_id = "other revision".into(),
        }
        let diff = compare_claim_sources(&report.claims, &competing);
        assert_eq!(diff.len(), 2);
        assert!(diff.iter().all(|d| d.kind != ClaimComparisonKind::Agreed));
    }
}
#[test]
fn duplicate_ids_nonfinite_confidence_and_invalid_quantities_fail() {
    let text = "Veyra spent money.";
    let mut invalid = payload(text);
    invalid.claims.push(invalid.claims[0].clone());
    assert_eq!(
        bind_extraction(invalid, binding(text), text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::DuplicateClaim
    );
    let mut invalid = payload(text);
    invalid.claims[0].confidence = Some(f64::NAN);
    assert_eq!(
        bind_extraction(invalid, binding(text), text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Confidence
    );
    for amount in ["NaN", "1e9", "1.2.3", "", "-"] {
        let mut invalid = payload(text);
        invalid.claims[0].assertion = ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "gold".into(),
            amount: amount.into(),
            unit: "coin".into(),
            transaction_id: None,
        };
        assert_eq!(
            bind_extraction(invalid, binding(text), text, &known())
                .unwrap_err()
                .code,
            ExtractionValidationCode::Value
        );
    }
}

#[test]
fn every_family_has_a_typed_round_trip_with_evidence() {
    let unresolved = || ClaimEntity::Unresolved {
        label: "unresolved referent".into(),
    };
    let assertions = vec![
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Unknown,
        },
        ClaimAssertion::Action {
            verb: "walked".into(),
            mode: ActionMode::Physical,
        },
        ClaimAssertion::LocatedAt {
            location: unresolved(),
        },
        ClaimAssertion::Move {
            from: unresolved(),
            to: unresolved(),
        },
        ClaimAssertion::SceneClock {
            clock: crate::models::StoryClock::default(),
        },
        ClaimAssertion::Possession {
            operation: PossessionOperation::Transfer,
            item: unresolved(),
            counterparty: Some(unresolved()),
            custody: CustodyKind::Custodian,
        },
        ClaimAssertion::Injury {
            operation: InjuryOperation::Add,
            injury_id: "wound:1".into(),
            body_site: "left hand".into(),
            severity: InjurySeverity::Minor,
        },
        ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-1.250".into(),
            unit: "gold".into(),
            transaction_id: Some("trade:1".into()),
        },
        ClaimAssertion::Knowledge {
            operation: KnowledgeOperation::Disclose,
            proposition: unresolved(),
            audience: vec![unresolved()],
            source: Some(unresolved()),
        },
        ClaimAssertion::Attribute {
            operation: AttributeOperation::Set,
            key: "eye_color".into(),
            value: ClaimScalar::Text {
                value: "blue".into(),
            },
            nature: AttributeNature::Invariant,
        },
        ClaimAssertion::Relationship {
            operation: RelationshipOperation::Observation,
            other: unresolved(),
            trust: 30,
            tension: 20,
        },
        ClaimAssertion::Promise {
            operation: PromiseOperation::Payoff,
            thread: unresolved(),
        },
        ClaimAssertion::Rule {
            operation: RuleOperation::Use,
            rule: unresolved(),
            anchor: None,
            preconditions: vec!["requires blood".into()],
            effects: vec!["opens gate".into()],
        },
    ];
    let mut families = BTreeSet::new();
    for assertion in assertions {
        let text = "Veyra acted.";
        let mut p = payload(text);
        p.coverage = CLAIM_FAMILIES
            .into_iter()
            .map(|f| (f, FamilyCoverage::InspectedNone))
            .collect();
        p.coverage
            .insert(assertion.family(), FamilyCoverage::ClaimsPresent);
        families.insert(assertion.family());
        p.claims[0].assertion = assertion;
        let p: ExtractionPayloadV1 =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert!(bind_extraction(p, binding(text), text, &known()).is_ok());
    }
    assert_eq!(families.len(), CLAIM_FAMILIES.len());
}

#[test]
fn normalization_preserves_sensitive_text_and_unit_case() {
    let text = "Veyra has the code.";
    let mut left = report(text).claims;
    left[0].proposed.assertion = ClaimAssertion::Attribute {
        operation: AttributeOperation::Set,
        key: "access code".into(),
        value: ClaimScalar::Text {
            value: "ABC".into(),
        },
        nature: AttributeNature::Unknown,
    };
    let mut right = left.clone();
    if let ClaimAssertion::Attribute {
        value: ClaimScalar::Text { value },
        ..
    } = &mut right[0].proposed.assertion
    {
        *value = "abc".into();
    }
    assert_eq!(
        compare_claim_sources(&left, &right)[0].kind,
        ClaimComparisonKind::Incompatible
    );
    left[0].proposed.assertion = ClaimAssertion::Quantity {
        operation: QuantityOperation::Set,
        measure: "mass".into(),
        amount: "001.2500".into(),
        unit: "g".into(),
        transaction_id: None,
    };
    right[0].proposed.assertion = ClaimAssertion::Quantity {
        operation: QuantityOperation::Set,
        measure: "mass".into(),
        amount: "1.25".into(),
        unit: "g".into(),
        transaction_id: None,
    };
    assert_eq!(
        compare_claim_sources(&left, &right)[0].kind,
        ClaimComparisonKind::Agreed
    );
    if let ClaimAssertion::Quantity { unit, .. } = &mut right[0].proposed.assertion {
        *unit = "G".into();
    }
    assert!(
        compare_claim_sources(&left, &right)
            .iter()
            .all(|d| d.kind != ClaimComparisonKind::Agreed)
    );
}

#[test]
fn malformed_resolved_id_is_rejected_even_if_a_caller_lists_it() {
    let text = "Veyra died.";
    let mut invalid = payload(text);
    invalid.claims[0].subject = ClaimEntity::Resolved {
        id: "not an entity id".into(),
    };
    let known = ["not an entity id".into()].into();
    assert_eq!(
        bind_extraction(invalid, binding(text), text, &known)
            .unwrap_err()
            .code,
        ExtractionValidationCode::Entity
    );
}

#[test]
fn host_payload_budget_matches_provider_budget() {
    let text = "x".repeat(MAX_EXTRACTION_PAYLOAD_BYTES);
    let oversized = payload(&text);
    assert_eq!(
        bind_extraction(oversized, binding(&text), &text, &known())
            .unwrap_err()
            .code,
        ExtractionValidationCode::Limit
    );
}

#[test]
fn additive_claim_multiplicity_is_not_hidden_by_value_deduplication() {
    let mut extracted = report("Veyra trusted Bram twice.").claims;
    extracted[0].proposed.assertion = ClaimAssertion::Relationship {
        operation: RelationshipOperation::Delta,
        other: ClaimEntity::Unresolved {
            label: "Bram".into(),
        },
        trust: 5,
        tension: 0,
    };
    let mut drafter = extracted.clone();
    drafter[0].source = ClaimSource::Drafter;
    let mut second = extracted[0].clone();
    second.proposed.claim_id = "claim:2".into();
    extracted.push(second);
    let diff = compare_claim_sources(&extracted, &drafter);
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0].kind, ClaimComparisonKind::Incompatible);
    assert_eq!(diff[0].extractor.len(), 2);
    assert_eq!(diff[0].drafter.len(), 1);
    for claim in extracted.iter_mut().chain(drafter.iter_mut()) {
        claim.proposed.assertion = ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        };
    }
    assert_eq!(
        compare_claim_sources(&extracted, &drafter)[0].kind,
        ClaimComparisonKind::Agreed
    );
}

#[test]
fn ambiguous_legacy_values_are_gaps_and_typed_boolean_is_preserved() {
    let text = "Veyra knew.";
    let package: DrafterPackageV1 = serde_json::from_value(serde_json::json!({
        "canonical_facts": [
            {"subject_id":"character:veyra", "predicate":"alive", "value_kind":"boolean", "value_text":"false"},
            {"subject_id":"character:veyra", "predicate":"code", "value_text":"ABC", "value":"abc"},
            {"subject_id":"character:veyra", "predicate":"date", "value_kind":"date", "value_text":"chapter 2"}
        ]
    })).unwrap();
    let normalized = normalize_drafter_package(&package, &binding(text), &known());
    assert_eq!(normalized.claims.len(), 1);
    assert!(matches!(
        normalized.claims[0].proposed.assertion,
        ClaimAssertion::Attribute {
            value: ClaimScalar::Boolean { value: false },
            ..
        }
    ));
    assert_eq!(normalized.unsupported_fields.len(), 2);
}

#[allow(clippy::too_many_arguments)]
fn make_test_event(
    event_id: &str,
    branch_id: &str,
    decision_seq: i64,
    append_seq: i64,
    subject_id: &str,
    payload: CanonEventPayloadV1,
    position: Option<CanonPositionV1>,
    within_scene_order: Option<u32>,
    scene_id: Option<&str>,
    source_revision_id: &str,
) -> CanonEventV1 {
    let subject = ClaimEntity::Resolved {
        id: subject_id.into(),
    };
    let (predicate, operation) = payload.predicate_and_operation();
    let predicate_family = payload.family();
    let sem_fp =
        semantic_event_fingerprint(&subject, &payload, position, within_scene_order, &None);
    let idem = compute_idempotency_key(
        branch_id,
        source_revision_id,
        &sem_fp,
        CanonPolicyAction::Ratify,
    );
    CanonEventV1 {
        event_id: event_id.into(),
        schema_version: CANON_EVENT_SCHEMA_VERSION,
        project_id: "project:p".into(),
        branch_id: branch_id.into(),
        decision_id: format!("canon_decision:{decision_seq}"),
        decision_seq,
        append_seq,
        tier: CanonTier::Ratified,
        policy_action: CanonPolicyAction::Ratify,
        subject,
        predicate_family,
        predicate: predicate.into(),
        operation: operation.into(),
        payload,
        effective_position: position,
        within_scene_order,
        in_world_time: None,
        scene_id: scene_id.map(str::to_owned),
        source_revision_id: source_revision_id.into(),
        source_claim_ids: vec![format!("claim:{event_id}")],
        semantic_fingerprint: sem_fp,
        idempotency_key: idem,
        evidence_sha256: exact_text_sha256(event_id),
        provenance: CanonProvenanceKind::Extracted,
        supersedes_event_id: None,
        withdraws_event_id: None,
        legacy_record_scope: None,
        merge_provenance: None,
        created_at_micros: decision_seq * 1_000,
    }
}

fn narration_assertion(assertion: ClaimAssertion) -> CanonEventPayloadV1 {
    CanonEventPayloadV1::Assertion {
        assertion,
        modality: ClaimModality::Narration,
        temporal_scope: TemporalScope::Current,
        speaker: None,
        experiencer: None,
    }
}

#[test]
fn missing_state_stays_unknown_and_injury_persists_through_unrelated_changes() {
    let lineage = vec!["branch:main".to_string()];
    let empty_view = state_at(
        "branch:main",
        &lineage,
        &[],
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    // Missing state is unknown, never alive, uninjured, or zero.
    assert_eq!(
        empty_view.life_status("character:veyra"),
        LifeStatus::Unknown
    );
    assert_eq!(empty_view.location("character:veyra"), None);
    assert_eq!(
        empty_view.quantity_balance("character:veyra", "wealth"),
        None
    );
    assert!(
        empty_view
            .relationship("character:veyra", "character:bram")
            .is_none()
    );
    assert_eq!(
        empty_view.knowledge_status("character:veyra", "fact:secret"),
        KnowledgeStatus::Unknown
    );

    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let p2 = CanonPositionV1::new(1, 1, 2).unwrap();
    let p3 = CanonPositionV1::new(1, 2, 1).unwrap();
    let p4 = CanonPositionV1::new(1, 2, 2).unwrap();

    let events = vec![
        // Scene 1: Veyra suffers a broken rib and acquires a dagger.
        make_test_event(
            "canon_event:1",
            "branch:main",
            1,
            1,
            "character:veyra",
            narration_assertion(ClaimAssertion::Injury {
                operation: InjuryOperation::Add,
                injury_id: "broken_rib".into(),
                body_site: "left ribs".into(),
                severity: InjurySeverity::Moderate,
            }),
            Some(p1),
            Some(1),
            Some("scene:1"),
            "rev:1",
        ),
        make_test_event(
            "canon_event:2",
            "branch:main",
            1,
            2,
            "character:veyra",
            narration_assertion(ClaimAssertion::Possession {
                operation: PossessionOperation::Acquire,
                item: ClaimEntity::Resolved {
                    id: "item:dagger".into(),
                },
                counterparty: None,
                custody: CustodyKind::Owner,
            }),
            Some(p1),
            Some(2),
            Some("scene:1"),
            "rev:1",
        ),
        // Scene 2: Unrelated changes (location move, status attribute, mood, second injury added and healed).
        make_test_event(
            "canon_event:3",
            "branch:main",
            2,
            3,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:harbor".into(),
                },
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:2",
        ),
        make_test_event(
            "canon_event:4",
            "branch:main",
            2,
            4,
            "character:veyra",
            narration_assertion(ClaimAssertion::Attribute {
                operation: AttributeOperation::Set,
                key: "status".into(),
                value: ClaimScalar::Text {
                    value: "exhausted".into(),
                },
                nature: AttributeNature::Evolving,
            }),
            Some(p2),
            Some(2),
            Some("scene:2"),
            "rev:2",
        ),
        make_test_event(
            "canon_event:5",
            "branch:main",
            2,
            5,
            "character:veyra",
            narration_assertion(ClaimAssertion::Injury {
                operation: InjuryOperation::Add,
                injury_id: "cut_palm".into(),
                body_site: "right palm".into(),
                severity: InjurySeverity::Minor,
            }),
            Some(p2),
            Some(3),
            Some("scene:2"),
            "rev:2",
        ),
        // Scene 3: Heal cut_palm, move location, update coat attribute; broken_rib must persist!
        make_test_event(
            "canon_event:6",
            "branch:main",
            3,
            6,
            "character:veyra",
            narration_assertion(ClaimAssertion::Injury {
                operation: InjuryOperation::Heal,
                injury_id: "cut_palm".into(),
                body_site: "right palm".into(),
                severity: InjurySeverity::Minor,
            }),
            Some(p3),
            Some(1),
            Some("scene:3"),
            "rev:3",
        ),
        make_test_event(
            "canon_event:7",
            "branch:main",
            3,
            7,
            "character:veyra",
            narration_assertion(ClaimAssertion::Move {
                from: ClaimEntity::Resolved {
                    id: "location:harbor".into(),
                },
                to: ClaimEntity::Resolved {
                    id: "location:citadel".into(),
                },
            }),
            Some(p3),
            Some(2),
            Some("scene:3"),
            "rev:3",
        ),
        // Scene 4: Explicitly heal broken_rib.
        make_test_event(
            "canon_event:8",
            "branch:main",
            4,
            8,
            "character:veyra",
            narration_assertion(ClaimAssertion::Injury {
                operation: InjuryOperation::Heal,
                injury_id: "broken_rib".into(),
                body_site: "left ribs".into(),
                severity: InjurySeverity::Moderate,
            }),
            Some(p4),
            Some(1),
            Some("scene:4"),
            "rev:4",
        ),
    ];

    let view_at_p3 = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::at_scene(p3),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    let active_p3 = view_at_p3.active_injuries("character:veyra");
    assert_eq!(active_p3.len(), 1);
    assert!(active_p3.contains_key("broken_rib"));
    assert_eq!(active_p3["broken_rib"].body_site, "left ribs");
    assert_eq!(active_p3["broken_rib"].severity, InjurySeverity::Moderate);
    assert_eq!(
        active_p3["broken_rib"].source_event_ids,
        vec!["canon_event:1".to_string()]
    );
    assert_eq!(
        view_at_p3.location("character:veyra"),
        Some("location:citadel")
    );
    assert_eq!(
        view_at_p3.holds_item("character:veyra", "item:dagger", CustodyKind::Owner),
        Some(true)
    );

    let view_at_p4 = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::at_scene(p4),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert!(view_at_p4.active_injuries("character:veyra").is_empty());
}

#[test]
fn transfers_conserve_quantities_and_deltas_require_opening_balances() {
    let lineage = vec!["branch:main".to_string()];
    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let p2 = CanonPositionV1::new(1, 1, 2).unwrap();

    let empty_view = state_at(
        "branch:main",
        &lineage,
        &[],
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();

    // 1. Delta without opening balance is rejected by batch precondition check.
    let delta_without_opening = make_test_event(
        "canon_event:no_open",
        "branch:main",
        1,
        1,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-10".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p1),
        Some(1),
        Some("scene:1"),
        "rev:1",
    );
    assert_eq!(
        validate_event_batch_preconditions(
            "author:op",
            CanonPolicyAction::Ratify,
            std::slice::from_ref(&delta_without_opening),
            &empty_view,
        )
        .unwrap_err()
        .code,
        CanonAppendErrorCode::Precondition
    );
    // And if fed directly to the reducer, it never invents a zero balance.
    let reduced_no_open = state_at(
        "branch:main",
        &lineage,
        &[delta_without_opening],
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(
        reduced_no_open.quantity_balance("character:veyra", "wealth"),
        None
    );
    assert_eq!(reduced_no_open.precondition_anomalies.len(), 1);

    // 2. Establish opening balances for Veyra (100.50 gold) and Bram (20.25 gold).
    let open_events = vec![
        make_test_event(
            "canon_event:open_v",
            "branch:main",
            1,
            1,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Set,
                measure: "wealth".into(),
                amount: "100.50".into(),
                unit: "gold".into(),
                transaction_id: None,
            }),
            Some(p1),
            Some(1),
            Some("scene:1"),
            "rev:1",
        ),
        make_test_event(
            "canon_event:open_b",
            "branch:main",
            1,
            2,
            "character:bram",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Set,
                measure: "wealth".into(),
                amount: "20.25".into(),
                unit: "gold".into(),
                transaction_id: None,
            }),
            Some(p1),
            Some(2),
            Some("scene:1"),
            "rev:1",
        ),
    ];
    validate_event_batch_preconditions(
        "author:op",
        CanonPolicyAction::Ratify,
        &open_events,
        &empty_view,
    )
    .unwrap();
    let after_open = state_at(
        "branch:main",
        &lineage,
        &open_events,
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();

    // 3. Unbalanced transfer (-35.50 and +30.00) fails conservation check.
    let bad_transfer = vec![
        make_test_event(
            "canon_event:bad_1",
            "branch:main",
            2,
            3,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "-35.50".into(),
                unit: "gold".into(),
                transaction_id: Some("tx:1".into()),
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:2",
        ),
        make_test_event(
            "canon_event:bad_2",
            "branch:main",
            2,
            4,
            "character:bram",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "30.00".into(),
                unit: "gold".into(),
                transaction_id: Some("tx:1".into()),
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:2",
        ),
    ];
    assert_eq!(
        validate_event_batch_preconditions(
            "author:op",
            CanonPolicyAction::Ratify,
            &bad_transfer,
            &after_open,
        )
        .unwrap_err()
        .code,
        CanonAppendErrorCode::Conservation
    );

    // 4. Balanced paired transfer (-35.50 and +35.50) passes and conserves total quantity.
    let good_transfer = vec![
        make_test_event(
            "canon_event:tx_1",
            "branch:main",
            2,
            3,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "-35.50".into(),
                unit: "gold".into(),
                transaction_id: Some("tx:1".into()),
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:2",
        ),
        make_test_event(
            "canon_event:tx_2",
            "branch:main",
            2,
            4,
            "character:bram",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "35.50".into(),
                unit: "gold".into(),
                transaction_id: Some("tx:1".into()),
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:2",
        ),
    ];
    validate_event_batch_preconditions(
        "author:op",
        CanonPolicyAction::Ratify,
        &good_transfer,
        &after_open,
    )
    .unwrap();

    let mut all_events = open_events;
    all_events.extend(good_transfer);
    let after_tx = state_at(
        "branch:main",
        &lineage,
        &all_events,
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(
        after_tx.quantity_balance("character:veyra", "wealth"),
        Some("65")
    );
    assert_eq!(
        after_tx.quantity_balance("character:bram", "wealth"),
        Some("55.75")
    );
    let total_before = ExactDecimal::parse("100.50")
        .unwrap()
        .checked_add(ExactDecimal::parse("20.25").unwrap())
        .unwrap();
    let total_after = ExactDecimal::parse(
        after_tx
            .quantity_balance("character:veyra", "wealth")
            .unwrap(),
    )
    .unwrap()
    .checked_add(
        ExactDecimal::parse(
            after_tx
                .quantity_balance("character:bram", "wealth")
                .unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(total_before, total_after);
}

#[test]
fn deterministic_replay_across_cursors_watermarks_within_scene_order_and_rollback() {
    let lineage = vec!["branch:main".to_string()];
    let ch1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let ch2 = CanonPositionV1::new(1, 2, 1).unwrap();
    let ch3 = CanonPositionV1::new(1, 3, 1).unwrap();

    let mut events = vec![
        // Decision 1 (Ch 1): Learn proposition at step 1, use at step 2.
        make_test_event(
            "canon_event:k_learn",
            "branch:main",
            1,
            1,
            "character:veyra",
            narration_assertion(ClaimAssertion::Knowledge {
                operation: KnowledgeOperation::Learn,
                proposition: ClaimEntity::Resolved {
                    id: "fact:cipher".into(),
                },
                audience: Vec::new(),
                source: None,
            }),
            Some(ch1),
            Some(1),
            Some("scene:1"),
            "rev:1",
        ),
        make_test_event(
            "canon_event:k_use",
            "branch:main",
            1,
            2,
            "character:veyra",
            narration_assertion(ClaimAssertion::Knowledge {
                operation: KnowledgeOperation::Use,
                proposition: ClaimEntity::Resolved {
                    id: "fact:cipher".into(),
                },
                audience: Vec::new(),
                source: None,
            }),
            Some(ch1),
            Some(2),
            Some("scene:1"),
            "rev:1",
        ),
        // Decision 2 (Ch 2): Eye color initially recorded as blue.
        make_test_event(
            "canon_event:eye_blue",
            "branch:main",
            2,
            3,
            "character:veyra",
            narration_assertion(ClaimAssertion::Attribute {
                operation: AttributeOperation::Set,
                key: "eye_color".into(),
                value: ClaimScalar::Text {
                    value: "blue".into(),
                },
                nature: AttributeNature::Invariant,
            }),
            Some(ch2),
            Some(1),
            Some("scene:2"),
            "rev:2",
        ),
        // Decision 3 (Ch 3): Veyra moves to tower.
        make_test_event(
            "canon_event:loc_tower",
            "branch:main",
            3,
            4,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:tower".into(),
                },
            }),
            Some(ch3),
            Some(1),
            Some("scene:3"),
            "rev:3",
        ),
    ];

    // Correction Decision 4: effective in Ch 2, supersedes canon_event:eye_blue with green.
    let mut correction = make_test_event(
        "canon_event:eye_green",
        "branch:main",
        4,
        5,
        "character:veyra",
        narration_assertion(ClaimAssertion::Attribute {
            operation: AttributeOperation::Set,
            key: "eye_color".into(),
            value: ClaimScalar::Text {
                value: "green".into(),
            },
            nature: AttributeNature::Invariant,
        }),
        Some(ch2),
        Some(1),
        Some("scene:2"),
        "rev:2_corrected",
    );
    correction.supersedes_event_id = Some("canon_event:eye_blue".into());
    events.push(correction);

    // Decision 5: Rollback / restore watermark to decision 3 (before the Ch 2 correction).
    let restore_ev = make_test_event(
        "canon_event:restore_to_3",
        "branch:main",
        5,
        6,
        "branch:main",
        CanonEventPayloadV1::WatermarkRestore {
            restore: WatermarkRestoreV1 {
                save_point_id: Some("save_point:sp3".into()),
                target_decision_seq: 3,
                target_event_seq: 4,
            },
        },
        None,
        None,
        None,
        "restore:sp3",
    );
    events.push(restore_ev);

    // 1. Within-scene ordering in Ch 1:
    let view_ch1_step1 = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::at_within_scene(ch1, 1),
        CanonTier::Ratified,
        Some(1),
    )
    .unwrap();
    let edge_step1 = &view_ch1_step1.knowledge["fact:cipher"].knowers["character:veyra"];
    assert_eq!(edge_step1.status, KnowledgeStatus::Known);
    assert!(edge_step1.uses.is_empty());

    let view_ch1_step2 = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::at_within_scene(ch1, 2),
        CanonTier::Ratified,
        Some(1),
    )
    .unwrap();
    let edge_step2 = &view_ch1_step2.knowledge["fact:cipher"].knowers["character:veyra"];
    assert_eq!(edge_step2.uses.len(), 1);
    assert_eq!(edge_step2.uses[0].status_at_use, KnowledgeStatus::Known);
    assert!(!edge_step2.uses[0].ordering_uncertain);

    // 2. Historical watermark 3 vs corrected watermark 4 at Ch 2:
    let ch2_at_wm3 = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::at_scene(ch2),
        CanonTier::Ratified,
        Some(3),
    )
    .unwrap();
    assert_eq!(
        ch2_at_wm3.entities["character:veyra"].attributes["eye_color"].scalar_value,
        Some(ClaimScalar::Text {
            value: "blue".into()
        })
    );

    let ch2_at_wm4 = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::at_scene(ch2),
        CanonTier::Ratified,
        Some(4),
    )
    .unwrap();
    assert_eq!(
        ch2_at_wm4.entities["character:veyra"].attributes["eye_color"].scalar_value,
        Some(ClaimScalar::Text {
            value: "green".into()
        })
    );

    // 3. Rollback at watermark 5 restores the exact watermark-3 view:
    let view_at_wm3 = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        Some(3),
    )
    .unwrap();
    let view_after_rollback = state_at(
        "branch:main",
        &lineage,
        &events,
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        Some(5),
    )
    .unwrap();
    assert_eq!(view_after_rollback.entities, view_at_wm3.entities);
    assert_eq!(
        view_after_rollback.visible_event_ids,
        view_at_wm3.visible_event_ids
    );

    // 4. Shuffled input event slice replays identically at every watermark.
    let mut reversed = events.clone();
    reversed.reverse();
    for wm in 1..=5 {
        let a = state_at(
            "branch:main",
            &lineage,
            &events,
            &CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            Some(wm),
        )
        .unwrap();
        let b = state_at(
            "branch:main",
            &lineage,
            &reversed,
            &CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            Some(wm),
        )
        .unwrap();
        assert_eq!(a, b, "replay must be deterministic at watermark {wm}");
    }
}

#[test]
fn parent_chain_inheritance_coverage_masks_legacy_whole_record_and_siblings() {
    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let p2 = CanonPositionV1::new(1, 2, 1).unwrap();
    let p3 = CanonPositionV1::new(1, 3, 1).unwrap();

    let events = vec![
        // Main Ch 1: Veyra at harbor, hair black, coat leather, wealth opening 100, +10 at Ch 2.
        make_test_event(
            "canon_event:m_loc1",
            "branch:main",
            1,
            1,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:harbor".into(),
                },
            }),
            Some(p1),
            Some(1),
            Some("scene:1"),
            "rev:m1",
        ),
        make_test_event(
            "canon_event:m_hair",
            "branch:main",
            1,
            2,
            "character:veyra",
            narration_assertion(ClaimAssertion::Attribute {
                operation: AttributeOperation::Set,
                key: "hair".into(),
                value: ClaimScalar::Text {
                    value: "black".into(),
                },
                nature: AttributeNature::Invariant,
            }),
            Some(p1),
            Some(2),
            Some("scene:1"),
            "rev:m1",
        ),
        make_test_event(
            "canon_event:m_coat",
            "branch:main",
            1,
            3,
            "character:veyra",
            narration_assertion(ClaimAssertion::Attribute {
                operation: AttributeOperation::Set,
                key: "coat".into(),
                value: ClaimScalar::Text {
                    value: "leather".into(),
                },
                nature: AttributeNature::Evolving,
            }),
            Some(p1),
            Some(3),
            Some("scene:1"),
            "rev:m1",
        ),
        make_test_event(
            "canon_event:m_q_open",
            "branch:main",
            1,
            4,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Set,
                measure: "wealth".into(),
                amount: "100".into(),
                unit: "gold".into(),
                transaction_id: None,
            }),
            Some(p1),
            Some(4),
            Some("scene:1"),
            "rev:m1",
        ),
        // Main Ch 2: Moves to market, earns +20 gold.
        make_test_event(
            "canon_event:m_loc2",
            "branch:main",
            2,
            5,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:market".into(),
                },
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:m2",
        ),
        make_test_event(
            "canon_event:m_q_delta2",
            "branch:main",
            2,
            6,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "20".into(),
                unit: "gold".into(),
                transaction_id: None,
            }),
            Some(p2),
            Some(2),
            Some("scene:2"),
            "rev:m2",
        ),
        // Main Ch 3 (live ancestor edit after fork!): Moves to citadel.
        make_test_event(
            "canon_event:m_loc3",
            "branch:main",
            3,
            7,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:citadel".into(),
                },
            }),
            Some(p3),
            Some(1),
            Some("scene:3"),
            "rev:m3",
        ),
        // Sibling branch: sets location to dungeon (must never leak to child or grandchild).
        make_test_event(
            "canon_event:sib_loc",
            "branch:sibling",
            4,
            8,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:dungeon".into(),
                },
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:sib",
        ),
        // Child branch: declares bounded location coverage interval [p2, p2] -> catacombs,
        // and masks ancestor quantity delta event `canon_event:m_q_delta2` replacing it with +50.
        make_test_event(
            "canon_event:c_mask_loc",
            "branch:child",
            5,
            9,
            "character:veyra",
            CanonEventPayloadV1::CoverageMask {
                mask: StreamCoverageMaskV1 {
                    target: CoverageMaskTargetV1::Stream {
                        stream: SemanticStreamKeyV1::Location {
                            subject_id: "character:veyra".into(),
                        },
                    },
                    start_position: Some(p2),
                    end_position: Some(p2),
                    tombstone: false,
                },
            },
            Some(p2),
            Some(0),
            Some("scene:2"),
            "rev:c2",
        ),
        make_test_event(
            "canon_event:c_loc2",
            "branch:child",
            5,
            10,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:catacombs".into(),
                },
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:c2",
        ),
        make_test_event(
            "canon_event:c_mask_q",
            "branch:child",
            5,
            11,
            "character:veyra",
            CanonEventPayloadV1::CoverageMask {
                mask: StreamCoverageMaskV1 {
                    target: CoverageMaskTargetV1::AncestorEvent {
                        event_id: "canon_event:m_q_delta2".into(),
                    },
                    start_position: None,
                    end_position: None,
                    tombstone: false,
                },
            },
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:c2",
        ),
        make_test_event(
            "canon_event:c_q_delta2",
            "branch:child",
            5,
            12,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "50".into(),
                unit: "gold".into(),
                transaction_id: None,
            }),
            Some(p2),
            Some(2),
            Some("scene:2"),
            "rev:c2",
        ),
        // Grandchild branch: tombstones `coat` attribute from p2 onward.
        make_test_event(
            "canon_event:gc_tombstone_coat",
            "branch:grandchild",
            6,
            13,
            "character:veyra",
            CanonEventPayloadV1::CoverageMask {
                mask: StreamCoverageMaskV1 {
                    target: CoverageMaskTargetV1::Stream {
                        stream: SemanticStreamKeyV1::AttributeScalar {
                            subject_id: "character:veyra".into(),
                            key: "coat".into(),
                        },
                    },
                    start_position: Some(p2),
                    end_position: None,
                    tombstone: true,
                },
            },
            Some(p2),
            Some(0),
            Some("scene:2"),
            "rev:gc2",
        ),
    ];

    let gc_lineage = vec![
        "branch:grandchild".to_string(),
        "branch:child".to_string(),
        "branch:main".to_string(),
    ];

    // 1. Before coverage interval (at p1): ancestor state is visible.
    let at_p1 = state_at(
        "branch:grandchild",
        &gc_lineage,
        &events,
        &CanonCursorV1::at_scene(p1),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(at_p1.location("character:veyra"), Some("location:harbor"));
    assert!(
        at_p1.entities["character:veyra"]
            .attributes
            .contains_key("coat")
    );

    // 2. During coverage interval (at p2): child location shadows main, coat is tombstoned,
    //    hair remains inherited, and quantity uses child's +50 instead of main's masked +20!
    let at_p2 = state_at(
        "branch:grandchild",
        &gc_lineage,
        &events,
        &CanonCursorV1::at_scene(p2),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(
        at_p2.location("character:veyra"),
        Some("location:catacombs")
    );
    assert!(
        !at_p2.entities["character:veyra"]
            .attributes
            .contains_key("coat")
    );
    assert_eq!(
        at_p2.entities["character:veyra"].attributes["hair"].scalar_value,
        Some(ClaimScalar::Text {
            value: "black".into()
        })
    );
    assert_eq!(
        at_p2.quantity_balance("character:veyra", "wealth"),
        Some("150")
    );

    // 3. After bounded location coverage interval (at p3): inheritance from main resumes!
    let at_p3 = state_at(
        "branch:grandchild",
        &gc_lineage,
        &events,
        &CanonCursorV1::at_scene(p3),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(at_p3.location("character:veyra"), Some("location:citadel"));

    // 4. Parent branch itself is never mutated by child/grandchild masks:
    let main_at_p2 = state_at(
        "branch:main",
        &["branch:main".into()],
        &events,
        &CanonCursorV1::at_scene(p2),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(
        main_at_p2.location("character:veyra"),
        Some("location:market")
    );
    assert_eq!(
        main_at_p2.quantity_balance("character:veyra", "wealth"),
        Some("120")
    );

    // 5. LegacyWholeRecord mask shadows all character_state fields together.
    let mut legacy_events = events.clone();
    legacy_events.push(make_test_event(
        "canon_event:legacy_whole_mask",
        "branch:legacy_child",
        7,
        14,
        "character:veyra",
        CanonEventPayloadV1::CoverageMask {
            mask: StreamCoverageMaskV1 {
                target: CoverageMaskTargetV1::LegacyWholeRecord {
                    record_kind: "character_state".into(),
                    subject_id: "character:veyra".into(),
                    natural_key: "character:veyra".into(),
                },
                start_position: Some(p1),
                end_position: None,
                tombstone: true,
            },
        },
        Some(p1),
        Some(0),
        Some("scene:1"),
        "rev:leg",
    ));
    let leg_view = state_at(
        "branch:legacy_child",
        &["branch:legacy_child".into(), "branch:main".into()],
        &legacy_events,
        &CanonCursorV1::at_scene(p2),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(leg_view.location("character:veyra"), None);
    assert!(
        leg_view
            .entities
            .get("character:veyra")
            .is_none_or(|e| e.attributes.is_empty())
    );
}

#[test]
fn v1_policy_guard_and_three_way_branch_merge_conflicts() {
    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let p2 = CanonPositionV1::new(1, 2, 1).unwrap();
    let empty_view = state_at(
        "branch:main",
        &["branch:main".into()],
        &[],
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();

    // 1. Deaths, secret disclosures, and promise payoffs never auto-ratify in v1.
    for (idx, forbidden) in [
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        },
        ClaimAssertion::Knowledge {
            operation: KnowledgeOperation::Disclose,
            proposition: ClaimEntity::Resolved {
                id: "fact:secret".into(),
            },
            audience: vec![ClaimEntity::Resolved {
                id: "character:bram".into(),
            }],
            source: None,
        },
        ClaimAssertion::Promise {
            operation: PromiseOperation::Payoff,
            thread: ClaimEntity::Resolved {
                id: "promise:oath".into(),
            },
        },
    ]
    .into_iter()
    .enumerate()
    {
        let ev = make_test_event(
            &format!("canon_event:forbid_{idx}"),
            "branch:main",
            1,
            1,
            "character:veyra",
            narration_assertion(forbidden),
            Some(p1),
            Some(1),
            Some("scene:1"),
            "rev:1",
        );
        assert_eq!(
            validate_event_batch_preconditions(
                "policy:canon-v1",
                CanonPolicyAction::Ratify,
                std::slice::from_ref(&ev),
                &empty_view,
            )
            .unwrap_err()
            .code,
            CanonAppendErrorCode::PolicyGuard
        );
        // Human ratification is permitted.
        assert!(
            validate_event_batch_preconditions(
                "author:human",
                CanonPolicyAction::Ratify,
                std::slice::from_ref(&ev),
                &empty_view,
            )
            .is_ok()
        );
    }

    // 2. 3-way merge conflict detection (divergent scalar set, removal conflict, duplicate transfer).
    let merge_events = vec![
        make_test_event(
            "canon_event:anc_inj",
            "branch:main",
            1,
            1,
            "character:veyra",
            narration_assertion(ClaimAssertion::Injury {
                operation: InjuryOperation::Add,
                injury_id: "scar".into(),
                body_site: "cheek".into(),
                severity: InjurySeverity::Minor,
            }),
            Some(p1),
            Some(1),
            Some("scene:1"),
            "rev:m1",
        ),
        // Target sets location to harbor, heals scar, and records tx:dup.
        make_test_event(
            "canon_event:tgt_loc",
            "branch:target",
            2,
            2,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:harbor".into(),
                },
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:t2",
        ),
        make_test_event(
            "canon_event:tgt_heal",
            "branch:target",
            2,
            3,
            "character:veyra",
            narration_assertion(ClaimAssertion::Injury {
                operation: InjuryOperation::Heal,
                injury_id: "scar".into(),
                body_site: "cheek".into(),
                severity: InjurySeverity::Minor,
            }),
            Some(p2),
            Some(2),
            Some("scene:2"),
            "rev:t2",
        ),
        make_test_event(
            "canon_event:tgt_tx",
            "branch:target",
            2,
            4,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "-10".into(),
                unit: "gold".into(),
                transaction_id: Some("tx:dup".into()),
            }),
            Some(p2),
            Some(3),
            Some("scene:2"),
            "rev:t2",
        ),
        // Source sets location to citadel (divergent scalar), escalates scar to Severe (removal/modify conflict),
        // reuses tx:dup (duplicate transfer), and adds a clean non-conflicting attribute `title`.
        make_test_event(
            "canon_event:src_loc",
            "branch:source",
            3,
            5,
            "character:veyra",
            narration_assertion(ClaimAssertion::LocatedAt {
                location: ClaimEntity::Resolved {
                    id: "location:citadel".into(),
                },
            }),
            Some(p2),
            Some(1),
            Some("scene:2"),
            "rev:s2",
        ),
        make_test_event(
            "canon_event:src_inj_mod",
            "branch:source",
            3,
            6,
            "character:veyra",
            narration_assertion(ClaimAssertion::Injury {
                operation: InjuryOperation::Add,
                injury_id: "scar".into(),
                body_site: "cheek".into(),
                severity: InjurySeverity::Severe,
            }),
            Some(p2),
            Some(2),
            Some("scene:2"),
            "rev:s2",
        ),
        make_test_event(
            "canon_event:src_tx",
            "branch:source",
            3,
            7,
            "character:veyra",
            narration_assertion(ClaimAssertion::Quantity {
                operation: QuantityOperation::Delta,
                measure: "wealth".into(),
                amount: "-25".into(),
                unit: "gold".into(),
                transaction_id: Some("tx:dup".into()),
            }),
            Some(p2),
            Some(3),
            Some("scene:2"),
            "rev:s2",
        ),
        make_test_event(
            "canon_event:src_title",
            "branch:source",
            3,
            8,
            "character:veyra",
            narration_assertion(ClaimAssertion::Attribute {
                operation: AttributeOperation::Set,
                key: "title".into(),
                value: ClaimScalar::Text {
                    value: "Captain".into(),
                },
                nature: AttributeNature::Evolving,
            }),
            Some(p2),
            Some(4),
            Some("scene:2"),
            "rev:s2",
        ),
    ];

    let plan = plan_canon_branch_merge(
        "branch:main",
        &["branch:main".into()],
        "branch:target",
        &["branch:target".into(), "branch:main".into()],
        "branch:source",
        &["branch:source".into(), "branch:main".into()],
        &merge_events,
        None,
    )
    .unwrap();

    let kinds: BTreeSet<CanonMergeConflictKind> = plan.conflicts.iter().map(|c| c.kind).collect();
    assert!(kinds.contains(&CanonMergeConflictKind::DivergentScalarSet));
    assert!(kinds.contains(&CanonMergeConflictKind::RemovalConflict));
    assert!(kinds.contains(&CanonMergeConflictKind::DuplicateTransfer));
    assert_eq!(plan.accepted_source_events.len(), 1);
    assert_eq!(
        plan.accepted_source_events[0].event_id,
        "canon_event:src_title"
    );

    // 7. Source-only removals (scene hiding, source invalidation, withdrawal, tombstone)
    //    are carried by `plan_canon_branch_merge` when target is unchanged.
    let anc_loc = make_test_event(
        "canon_event:anc_scene1_loc",
        "branch:main",
        1,
        1,
        "character:veyra",
        narration_assertion(ClaimAssertion::LocatedAt {
            location: ClaimEntity::Resolved {
                id: "location:harbor".into(),
            },
        }),
        Some(p1),
        Some(1),
        Some("scene:1"),
        "rev:m1",
    );
    let src_hide = make_test_event(
        "canon_event:src_hide_scene1",
        "branch:source",
        2,
        2,
        "scene:1",
        CanonEventPayloadV1::SceneVisibility {
            visibility: ScenePlacementVisibilityV1 {
                scene_id: "scene:1".into(),
                visible: false,
                position_override: None,
                invalidated_from: p1,
            },
        },
        Some(p1),
        None,
        Some("scene:1"),
        "rev:s_hide",
    );
    let removal_plan = plan_canon_branch_merge(
        "branch:main",
        &["branch:main".into()],
        "branch:main",
        &["branch:main".into()],
        "branch:source",
        &["branch:source".into(), "branch:main".into()],
        &[anc_loc.clone(), src_hide.clone()],
        None,
    )
    .unwrap();
    assert!(removal_plan.conflicts.is_empty());
    assert_eq!(removal_plan.accepted_source_events.len(), 1);
    assert_eq!(
        removal_plan.accepted_source_events[0].event_id,
        "canon_event:src_hide_scene1"
    );

    // 8. Preconditions are evaluated in reducer order (within-scene step bounds and
    //    intervening history across multi-position batches): a later opening balance
    //    or acquisition cannot satisfy an earlier delta or drop.
    let open_at_step_10 = make_test_event(
        "canon_event:open_step10",
        "branch:main",
        1,
        1,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Set,
            measure: "wealth".into(),
            amount: "100".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p1),
        Some(10),
        Some("scene:1"),
        "rev:m1",
    );
    let delta_at_step_5 = make_test_event(
        "canon_event:delta_step5",
        "branch:main",
        2,
        2,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-10".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p1),
        Some(5),
        Some("scene:1"),
        "rev:m2",
    );
    assert_eq!(
        validate_event_batch_in_history(
            "branch:main",
            &["branch:main".into()],
            std::slice::from_ref(&open_at_step_10),
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&delta_at_step_5),
        )
        .unwrap_err()
        .code,
        CanonAppendErrorCode::Precondition
    );

    let acquire_at_step_10 = make_test_event(
        "canon_event:acq_step10",
        "branch:main",
        1,
        1,
        "character:veyra",
        narration_assertion(ClaimAssertion::Possession {
            operation: PossessionOperation::Acquire,
            item: ClaimEntity::Resolved {
                id: "item:key".into(),
            },
            counterparty: None,
            custody: CustodyKind::Custodian,
        }),
        Some(p1),
        Some(10),
        Some("scene:1"),
        "rev:m1",
    );
    let drop_at_step_5 = make_test_event(
        "canon_event:drop_step5",
        "branch:main",
        2,
        2,
        "character:veyra",
        narration_assertion(ClaimAssertion::Possession {
            operation: PossessionOperation::Lose,
            item: ClaimEntity::Resolved {
                id: "item:key".into(),
            },
            counterparty: None,
            custody: CustodyKind::Custodian,
        }),
        Some(p1),
        Some(5),
        Some("scene:1"),
        "rev:m2",
    );
    assert_eq!(
        validate_event_batch_in_history(
            "branch:main",
            &["branch:main".into()],
            std::slice::from_ref(&acquire_at_step_10),
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&drop_at_step_5),
        )
        .unwrap_err()
        .code,
        CanonAppendErrorCode::Precondition
    );

    // Intervening history across a multi-position batch: Acquire at p1 and Transfer at p3
    // in one batch fails when existing history has a Lose at p2.
    let p3 = CanonPositionV1::new(1, 3, 1).unwrap();
    let lose_at_p2 = make_test_event(
        "canon_event:lose_p2",
        "branch:main",
        1,
        1,
        "character:veyra",
        narration_assertion(ClaimAssertion::Possession {
            operation: PossessionOperation::Lose,
            item: ClaimEntity::Resolved {
                id: "item:key".into(),
            },
            counterparty: None,
            custody: CustodyKind::Custodian,
        }),
        Some(p2),
        Some(1),
        Some("scene:2"),
        "rev:m_p2",
    );
    let multi_pos_batch = vec![
        make_test_event(
            "canon_event:acq_p1",
            "branch:main",
            2,
            2,
            "character:veyra",
            narration_assertion(ClaimAssertion::Possession {
                operation: PossessionOperation::Acquire,
                item: ClaimEntity::Resolved {
                    id: "item:key".into(),
                },
                counterparty: None,
                custody: CustodyKind::Custodian,
            }),
            Some(p1),
            Some(1),
            Some("scene:1"),
            "rev:m_multi",
        ),
        make_test_event(
            "canon_event:xfer_p3",
            "branch:main",
            2,
            3,
            "character:veyra",
            narration_assertion(ClaimAssertion::Possession {
                operation: PossessionOperation::Transfer,
                item: ClaimEntity::Resolved {
                    id: "item:key".into(),
                },
                counterparty: Some(ClaimEntity::Resolved {
                    id: "character:bram".into(),
                }),
                custody: CustodyKind::Custodian,
            }),
            Some(p3),
            Some(1),
            Some("scene:3"),
            "rev:m_multi",
        ),
    ];
    let multi_err = validate_event_batch_in_history(
        "branch:main",
        &["branch:main".into()],
        std::slice::from_ref(&lose_at_p2),
        "author:op",
        CanonPolicyAction::Ratify,
        CanonTier::Ratified,
        &multi_pos_batch,
    )
    .unwrap_err();
    assert_eq!(multi_err.code, CanonAppendErrorCode::Precondition);
    assert_eq!(multi_err.event_index, Some(1));

    // 9. Coverage bounds are evaluated at each candidate's cursor during validation
    //    (matching `state_at` before, within, and after a mask interval).
    let p5 = CanonPositionV1::new(1, 5, 1).unwrap();
    let p6 = CanonPositionV1::new(1, 6, 1).unwrap();
    let p7 = CanonPositionV1::new(1, 7, 1).unwrap();
    let p8 = CanonPositionV1::new(1, 8, 1).unwrap();
    let p10 = CanonPositionV1::new(1, 10, 1).unwrap();
    let p12 = CanonPositionV1::new(1, 12, 1).unwrap();

    let parent_open_p1 = make_test_event(
        "canon_event:parent_open_p1",
        "branch:main",
        1,
        1,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Set,
            measure: "wealth".into(),
            amount: "100".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p1),
        Some(1),
        Some("scene:1"),
        "rev:m_open",
    );
    let child_unbounded_mask_from_p10 = make_test_event(
        "canon_event:child_mask_p10_onward",
        "branch:child",
        2,
        2,
        "character:veyra",
        CanonEventPayloadV1::CoverageMask {
            mask: StreamCoverageMaskV1 {
                target: CoverageMaskTargetV1::Stream {
                    stream: SemanticStreamKeyV1::QuantityMeasure {
                        subject_id: "character:veyra".into(),
                        measure: "wealth".into(),
                        unit: "gold".into(),
                    },
                },
                start_position: Some(p10),
                end_position: None,
                tombstone: false,
            },
        },
        Some(p10),
        None,
        Some("scene:10"),
        "rev:c_mask10",
    );
    let child_delta_p5 = make_test_event(
        "canon_event:child_delta_p5",
        "branch:child",
        3,
        3,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-15".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p5),
        Some(1),
        Some("scene:5"),
        "rev:c_p5",
    );
    let child_delta_p10 = make_test_event(
        "canon_event:child_delta_p10",
        "branch:child",
        3,
        3,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-15".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p10),
        Some(1),
        Some("scene:10"),
        "rev:c_p10",
    );

    // Before the unbounded mask `[p10, None)` starts, child delta at `p5` inherits the
    // parent opening balance at `p1` and validates cleanly.
    assert!(
        validate_event_batch_in_history(
            "branch:child",
            &["branch:child".into(), "branch:main".into()],
            &[
                parent_open_p1.clone(),
                child_unbounded_mask_from_p10.clone()
            ],
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&child_delta_p5),
        )
        .is_ok()
    );
    // Within `[p10, None)`, the parent opening is masked and child delta at `p10` fails
    // without a child opening balance.
    assert_eq!(
        validate_event_batch_in_history(
            "branch:child",
            &["branch:child".into(), "branch:main".into()],
            &[parent_open_p1.clone(), child_unbounded_mask_from_p10],
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&child_delta_p10),
        )
        .unwrap_err()
        .code,
        CanonAppendErrorCode::Precondition
    );

    // Bounded tombstone mask `[p5, p10]`: candidate before (`p3`) passes, candidate
    // within (`p7`) fails, and candidate after (`p12`) passes once inheritance resumes.
    let bounded_tombstone_p5_p10 = make_test_event(
        "canon_event:child_tombstone_p5_p10",
        "branch:child",
        2,
        2,
        "character:veyra",
        CanonEventPayloadV1::CoverageMask {
            mask: StreamCoverageMaskV1 {
                target: CoverageMaskTargetV1::Stream {
                    stream: SemanticStreamKeyV1::QuantityMeasure {
                        subject_id: "character:veyra".into(),
                        measure: "wealth".into(),
                        unit: "gold".into(),
                    },
                },
                start_position: Some(p5),
                end_position: Some(p10),
                tombstone: true,
            },
        },
        Some(p5),
        None,
        Some("scene:5"),
        "rev:c_tomb",
    );
    let child_delta_p3 = make_test_event(
        "canon_event:child_delta_p3",
        "branch:child",
        3,
        3,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-10".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p3),
        Some(1),
        Some("scene:3"),
        "rev:c_p3",
    );
    let child_delta_p7 = make_test_event(
        "canon_event:child_delta_p7",
        "branch:child",
        3,
        3,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-10".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p7),
        Some(1),
        Some("scene:7"),
        "rev:c_p7",
    );
    let child_delta_p12 = make_test_event(
        "canon_event:child_delta_p12",
        "branch:child",
        3,
        3,
        "character:veyra",
        narration_assertion(ClaimAssertion::Quantity {
            operation: QuantityOperation::Delta,
            measure: "wealth".into(),
            amount: "-10".into(),
            unit: "gold".into(),
            transaction_id: None,
        }),
        Some(p12),
        Some(1),
        Some("scene:12"),
        "rev:c_p12",
    );

    assert!(
        validate_event_batch_in_history(
            "branch:child",
            &["branch:child".into(), "branch:main".into()],
            &[parent_open_p1.clone(), bounded_tombstone_p5_p10.clone()],
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&child_delta_p3),
        )
        .is_ok()
    );
    assert_eq!(
        validate_event_batch_in_history(
            "branch:child",
            &["branch:child".into(), "branch:main".into()],
            &[parent_open_p1.clone(), bounded_tombstone_p5_p10.clone()],
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&child_delta_p7),
        )
        .unwrap_err()
        .code,
        CanonAppendErrorCode::Precondition
    );
    assert!(
        validate_event_batch_in_history(
            "branch:child",
            &["branch:child".into(), "branch:main".into()],
            &[parent_open_p1, bounded_tombstone_p5_p10],
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&child_delta_p12),
        )
        .is_ok()
    );

    // Bounded child override `[p5, p10]` without parent possession: child acquisition at `p6`
    // satisfies a drop within the interval (`p8`), but expires after `p10` so a drop at `p12` fails.
    let bounded_item_mask_p5_p10 = make_test_event(
        "canon_event:child_item_mask_p5_p10",
        "branch:child",
        1,
        1,
        "character:veyra",
        CanonEventPayloadV1::CoverageMask {
            mask: StreamCoverageMaskV1 {
                target: CoverageMaskTargetV1::Stream {
                    stream: SemanticStreamKeyV1::PossessionItem {
                        subject_id: "character:veyra".into(),
                        item_id: "item:badge".into(),
                        custody: CustodyKind::Custodian,
                    },
                },
                start_position: Some(p5),
                end_position: Some(p10),
                tombstone: false,
            },
        },
        Some(p5),
        None,
        Some("scene:5"),
        "rev:c_item_mask",
    );
    let child_acq_p6 = make_test_event(
        "canon_event:child_acq_p6",
        "branch:child",
        2,
        2,
        "character:veyra",
        narration_assertion(ClaimAssertion::Possession {
            operation: PossessionOperation::Acquire,
            item: ClaimEntity::Resolved {
                id: "item:badge".into(),
            },
            counterparty: None,
            custody: CustodyKind::Custodian,
        }),
        Some(p6),
        Some(1),
        Some("scene:6"),
        "rev:c_acq6",
    );
    let child_drop_p8 = make_test_event(
        "canon_event:child_drop_p8",
        "branch:child",
        3,
        3,
        "character:veyra",
        narration_assertion(ClaimAssertion::Possession {
            operation: PossessionOperation::Lose,
            item: ClaimEntity::Resolved {
                id: "item:badge".into(),
            },
            counterparty: None,
            custody: CustodyKind::Custodian,
        }),
        Some(p8),
        Some(1),
        Some("scene:8"),
        "rev:c_drop8",
    );
    let child_drop_p12 = make_test_event(
        "canon_event:child_drop_p12",
        "branch:child",
        3,
        3,
        "character:veyra",
        narration_assertion(ClaimAssertion::Possession {
            operation: PossessionOperation::Lose,
            item: ClaimEntity::Resolved {
                id: "item:badge".into(),
            },
            counterparty: None,
            custody: CustodyKind::Custodian,
        }),
        Some(p12),
        Some(1),
        Some("scene:12"),
        "rev:c_drop12",
    );

    assert!(
        validate_event_batch_in_history(
            "branch:child",
            &["branch:child".into(), "branch:main".into()],
            &[bounded_item_mask_p5_p10.clone(), child_acq_p6.clone()],
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&child_drop_p8),
        )
        .is_ok()
    );
    assert_eq!(
        validate_event_batch_in_history(
            "branch:child",
            &["branch:child".into(), "branch:main".into()],
            &[bounded_item_mask_p5_p10, child_acq_p6],
            "author:op",
            CanonPolicyAction::Ratify,
            CanonTier::Ratified,
            std::slice::from_ref(&child_drop_p12),
        )
        .unwrap_err()
        .code,
        CanonAppendErrorCode::Precondition
    );
}

#[test]
fn backfill_position_and_knowledge_helpers_never_invent_scene_zero() {
    assert_eq!(
        coordinates_to_canon_position(Some(1), Some(2), Some(3)),
        CanonPositionV1::new(1, 2, 3).ok()
    );
    assert_eq!(
        coordinates_to_canon_position(Some(0), Some(1), Some(1)),
        None
    );
    assert_eq!(
        coordinates_to_canon_position(Some(1), Some(0), Some(1)),
        None
    );
    assert_eq!(
        coordinates_to_canon_position(Some(1), Some(1), Some(0)),
        None
    );
    assert_eq!(coordinates_to_canon_position(Some(1), Some(1), None), None);

    let p_valid = crate::models::StoryPlacement {
        book_number: 2,
        chapter_number: 4,
        scene_order: Some(1),
        note: None,
    };
    assert_eq!(
        story_placement_to_canon_position(&p_valid),
        CanonPositionV1::new(2, 4, 1).ok()
    );
    let p_chapter_only = crate::models::StoryPlacement {
        book_number: 2,
        chapter_number: 4,
        scene_order: None,
        note: None,
    };
    assert_eq!(story_placement_to_canon_position(&p_chapter_only), None);

    // Deduplicated knowledge proposition ID: collision-resistant for punctuation and non-Latin facts,
    // deterministic across case/whitespace, and preserves secret_of_fact_id grouping.
    let prop_cipher_1 = deduplicated_knowledge_proposition_id("Veyra has the cipher", None);
    let prop_cipher_2 = deduplicated_knowledge_proposition_id("  veyra has the cipher  ", None);
    assert_eq!(prop_cipher_1, prop_cipher_2);
    assert!(prop_cipher_1.starts_with("proposition:fact:veyra_has_the_cipher:"));

    let prop_gt = deduplicated_knowledge_proposition_id("x > y", None);
    let prop_lt = deduplicated_knowledge_proposition_id("x < y", None);
    assert_ne!(
        prop_gt, prop_lt,
        "punctuation-distinguished facts must not collide"
    );

    let prop_zh_1 = deduplicated_knowledge_proposition_id("北门有暗道", None);
    let prop_zh_2 = deduplicated_knowledge_proposition_id("南塔已封锁", None);
    assert_ne!(prop_zh_1, prop_zh_2, "non-Latin facts must not collide");
    assert!(!prop_zh_1.ends_with(":unnamed"));

    assert_eq!(
        deduplicated_knowledge_proposition_id(
            "Veyra has the cipher",
            Some("canonical_fact:secret_1")
        ),
        "proposition:secret:canonical_fact:secret_1"
    );
    assert_eq!(
        deduplicated_knowledge_proposition_id("北门有暗道", Some("canonical_fact:secret_1")),
        "proposition:secret:canonical_fact:secret_1"
    );
}

#[test]
fn diff_character_status_snapshots_emits_life_injury_and_set_diffs() {
    let initial = diff_character_status_snapshots(
        "character:veyra",
        &[],
        &[
            "alive".into(),
            "injury:left_arm:severe".into(),
            "exhausted".into(),
        ],
    );
    assert!(initial.contains(&ClaimAssertion::LifeStatus {
        status: LifeStatus::Alive,
    }));
    assert!(initial.contains(&ClaimAssertion::Injury {
        operation: InjuryOperation::Add,
        injury_id: "legacy_injury:character:veyra:left_arm".into(),
        body_site: "left_arm".into(),
        severity: InjurySeverity::Severe,
    }));
    assert!(initial.contains(&ClaimAssertion::Attribute {
        operation: AttributeOperation::Add,
        key: "character_state.status".into(),
        value: ClaimScalar::Text {
            value: "exhausted".into(),
        },
        nature: AttributeNature::Evolving,
    }));

    // Next snapshot heals the injury, removes "exhausted", and marks dead
    let next = diff_character_status_snapshots(
        "character:veyra",
        &[
            "alive".into(),
            "injury:left_arm:severe".into(),
            "exhausted".into(),
        ],
        &["dead".into()],
    );
    assert!(next.contains(&ClaimAssertion::LifeStatus {
        status: LifeStatus::Dead,
    }));
    assert!(next.contains(&ClaimAssertion::Injury {
        operation: InjuryOperation::Heal,
        injury_id: "legacy_injury:character:veyra:left_arm".into(),
        body_site: "left_arm".into(),
        severity: InjurySeverity::Severe,
    }));
    assert!(next.contains(&ClaimAssertion::Attribute {
        operation: AttributeOperation::Remove,
        key: "character_state.status".into(),
        value: ClaimScalar::Text {
            value: "exhausted".into(),
        },
        nature: AttributeNature::Evolving,
    }));
}

#[test]
fn classify_progression_delta_distinguishes_typed_transitions_from_opaque_blobs() {
    let tier_up = classify_progression_delta_json(
        "character:veyra",
        "tier_up",
        r#"{"from_tier":"Iron","to_tier":"Silver"}"#,
    );
    match tier_up {
        ProgressionDeltaClassification::Attributable { assertions, .. } => {
            assert!(assertions.contains(&ClaimAssertion::Attribute {
                operation: AttributeOperation::Set,
                key: "progression.to_tier".into(),
                value: ClaimScalar::Text {
                    value: "Silver".into(),
                },
                nature: AttributeNature::Evolving,
            }));
        }
        other => panic!("expected Attributable, got {other:?}"),
    }

    let opaque = classify_progression_delta_json(
        "character:veyra",
        "custom_mystery",
        r#"{"unstructured_blob":[1,2,3]}"#,
    );
    assert!(matches!(
        opaque,
        ProgressionDeltaClassification::Quarantined { .. }
    ));
}

#[test]
fn invalidated_coverage_mask_or_scene_visibility_restores_masked_state() {
    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let main_attr = make_test_event(
        "canon_event:main_attr",
        "branch:main",
        1,
        1,
        "location:citadel",
        narration_assertion(ClaimAssertion::Attribute {
            operation: AttributeOperation::Set,
            key: "world_state.status".into(),
            value: ClaimScalar::Text {
                value: "intact".into(),
            },
            nature: AttributeNature::Evolving,
        }),
        Some(p1),
        Some(1),
        Some("scene:1"),
        "rev:m1",
    );
    let tombstone = make_test_event(
        "canon_event:child_tombstone",
        "branch:child",
        1,
        1,
        "location:citadel",
        CanonEventPayloadV1::CoverageMask {
            mask: StreamCoverageMaskV1 {
                target: CoverageMaskTargetV1::Stream {
                    stream: SemanticStreamKeyV1::AttributeScalar {
                        subject_id: "location:citadel".into(),
                        key: "world_state.status".into(),
                    },
                },
                start_position: None,
                end_position: None,
                tombstone: true,
            },
        },
        None,
        None,
        None,
        "rev:c_tombstone",
    );

    let lineage = vec!["branch:child".to_string(), "branch:main".to_string()];
    let view_masked = state_at(
        "branch:child",
        &lineage,
        &[main_attr.clone(), tombstone.clone()],
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert!(
        view_masked
            .entity("location:citadel")
            .and_then(|e| e.attributes.get("world_state.status"))
            .is_none()
    );

    // Now invalidate the tombstone (as happens during a savepoint restore rollback)
    let rollback = make_test_event(
        "canon_event:child_rollback",
        "branch:child",
        2,
        2,
        "branch:child",
        CanonEventPayloadV1::SourceInvalidation {
            invalidation: SourceInvalidationV1 {
                scene_id: None,
                invalidated_revision_ids: vec!["rev:c_tombstone".into()],
                invalidated_event_ids: vec!["canon_event:child_tombstone".into()],
                replacement_revision_id: None,
            },
        },
        None,
        None,
        None,
        "rev:c_rollback",
    );
    let view_restored = state_at(
        "branch:child",
        &lineage,
        &[main_attr, tombstone, rollback],
        &CanonCursorV1::unbounded(),
        CanonTier::Ratified,
        None,
    )
    .unwrap();
    assert_eq!(
        view_restored
            .entity("location:citadel")
            .and_then(|e| e.attributes.get("world_state.status"))
            .and_then(|a| a.scalar_value.as_ref()),
        Some(&ClaimScalar::Text {
            value: "intact".into()
        })
    );
}
