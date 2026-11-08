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
