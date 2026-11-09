use super::*;

fn scene<'a>(text: &'a str) -> SceneValidationInputV1<'a> {
    SceneValidationInputV1 {
        scene_id: "scene:1",
        book_number: 1,
        chapter_number: 11,
        scene_order: 1,
        text,
    }
}

fn attribute_fact(key: &str, value: &str, subject_names: &[&str]) -> AttributeValidationFactV1 {
    AttributeValidationFactV1 {
        fact_id: "canonical_fact:1".into(),
        subject_id: Some("character:mara".into()),
        subject_names: subject_names
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
        key: key.into(),
        value: value.into(),
    }
}

fn knowledge_fact(fact: &str, names: &[&str], chapter: i32) -> KnowledgeValidationFactV1 {
    KnowledgeValidationFactV1 {
        fact_id: "knowledge_fact:1".into(),
        character_id: "character:caelen".into(),
        character_names: names.iter().map(|name| (*name).to_string()).collect(),
        character_aliases: names.iter().map(|name| (*name).to_string()).collect(),
        fact: fact.into(),
        learned_book_number: 1,
        learned_chapter_number: chapter,
        learned_scene_order: Some(1),
    }
}

#[test]
fn attribute_unkeyed_prosthetic_conflict_is_detected() {
    let text =
        "Mara Vance flexed the five bare, unscarred fingers of her flesh-and-blood left hand.";
    let fact = attribute_fact(
        "Mara left hand prosthetic",
        "articulated iron gauntlet",
        &["Mara Vance"],
    );
    let finding = detect_attribute_prose_contradiction(&scene(text), &fact).expect("conflict");
    assert_eq!(finding.check_type, "canonical_fact_prose_drift");
    assert!(finding.message.contains("chapter 11"));
    assert!(finding.message.contains("prosthetic") || finding.message.contains("gauntlet"));
}

#[test]
fn attribute_unkeyed_cybernetic_conflict_is_detected() {
    let text = "Elena Rostova adjusted the crimson-tinted monocle over her cybernetic right eye.";
    let fact = attribute_fact(
        "Elena natural eyes",
        "unaugmented biological eyes",
        &["Elena Rostova"],
    );
    let finding = detect_attribute_prose_contradiction(&scene(text), &fact).expect("conflict");
    assert!(finding.message.contains("biological"));
    assert!(finding.message.contains("cybernetic"));
}

#[test]
fn attribute_negation_pov_and_dialogue_controls_do_not_flag() {
    let fact = attribute_fact(
        "Mara left hand prosthetic",
        "articulated iron gauntlet",
        &["Mara Vance"],
    );
    for text in [
        "Mara Vance did not have a bare flesh-and-blood left hand.",
        "In the dream, Mara Vance flexed the bare fingers of her flesh-and-blood left hand.",
        "\"Mara Vance has a bare flesh-and-blood left hand,\" the liar said.",
        "Mara Vance admired the five unscarred fingers of her iron gauntlet.",
    ] {
        assert!(
            detect_attribute_prose_contradiction(&scene(text), &fact).is_none(),
            "unexpected contradiction for: {text}"
        );
    }
}

#[test]
fn attribute_without_subject_head_or_antonym_does_not_flag() {
    let fact = attribute_fact(
        "Mara left hand prosthetic",
        "articulated iron gauntlet",
        &["Mara Vance"],
    );
    for text in [
        "Caelen Dravos flexed the five bare, unscarred fingers of his flesh-and-blood left hand.",
        "Mara Vance walked the parapet with her amber eyes fixed on the sleet.",
        "Mara Vance adjusted her cloak against the wind.",
    ] {
        assert!(
            detect_attribute_prose_contradiction(&scene(text), &fact).is_none(),
            "unexpected contradiction for: {text}"
        );
    }
}

#[test]
fn knowledge_paraphrase_both_manuscripts_are_detected() {
    let ashfall = "Before retreating from the arch, Caelen Dravos warned the guards that Varek had already tainted the drinking water beneath the Salt Spire.";
    let finding = detect_knowledge_paraphrase_leak(
        &scene(ashfall),
        &knowledge_fact(
            "Commander Varek poisoned the well at Salt Spire",
            &["Caelen Dravos"],
            16,
        ),
    )
    .expect("paraphrase");
    assert!(finding.message.contains("chapter 16"));
    assert!(finding.message.contains("Caelen Dravos"));

    let veridian = "Kira Sato quietly warned Elena Rostova that Director Cassel had personally ordered the depressurization of the cryo vault.";
    let finding = detect_knowledge_paraphrase_leak(
        &scene(veridian),
        &knowledge_fact(
            "Director Cassel vented Cryo Vault Seven",
            &["Kira Sato"],
            16,
        ),
    )
    .expect("paraphrase");
    assert!(finding.message.contains("chapter 16"));
}

#[test]
fn knowledge_negation_time_and_thin_overlap_controls_do_not_flag() {
    let fact = knowledge_fact(
        "Commander Varek poisoned the well at Salt Spire",
        &["Caelen Dravos"],
        16,
    );
    for text in [
        "Caelen Dravos did not know that Varek had poisoned the well at Salt Spire.",
        "Caelen Dravos wondered whether Varek had poisoned the well at Salt Spire.",
        "Caelen Dravos warned the guards about the bridge.",
        "Kira Sato warned that Varek had poisoned the well at Salt Spire.",
        "Three years ago, Caelen Dravos warned that Varek had poisoned the well at Salt Spire.",
        "Caelen Dravos never warned the guards that Varek had tainted the water beneath Salt Spire.",
        "Caelen Dravos did not mention that Varek poisoned the well at Salt Spire.",
        "Caelen Dravos warned no one that Varek poisoned the well at Salt Spire.",
    ] {
        assert!(
            detect_knowledge_paraphrase_leak(&scene(text), &fact).is_none(),
            "unexpected paraphrase for: {text}"
        );
    }
}

#[test]
fn knowledge_paraphrase_handles_curly_punctuation_and_multibyte_names() {
    let fact = knowledge_fact(
        "Commander Varek poisoned the well at Salt Spire",
        &["Caelen Drávos"],
        16,
    );
    for text in [
        "Caelen Drávos warned the guards about Varek’s attack at Salt Spire.",
        "Caelen Drávos warned the guards about Varek’s attack at Salt Spire — “and worse.”",
    ] {
        let _ = detect_knowledge_paraphrase_leak(&scene(text), &fact);
    }
}

#[test]
fn secret_narration_leak_is_detected_but_negation_and_dialogue_are_not() {
    let candidate = SecretValidationCandidateV1 {
        fact_id: "canonical_fact:secret".into(),
        predicate: "vault passphrase".into(),
        value: "obsidiancrown".into(),
        character_id: "character:caelen".into(),
        character_name: "Caelen Dravos".into(),
        character_aliases: vec!["Caelen Dravos".into()],
        other_present_character_names: vec!["Oren Vale".into()],
        other_present_character_aliases: vec!["Oren Vale".into()],
    };
    let text = "While Oren Vale turned away, Caelen Dravos quietly copied the vault passphrase obsidiancrown into his personal field notebook.";
    let finding = detect_secret_narration_leak(&scene(text), &candidate).expect("narration leak");
    assert_eq!(finding.check_type, "secret_leak");
    assert!(finding.message.contains("obsidiancrown"));
    assert!(finding.message.contains("Caelen Dravos"));

    for control in [
        "Caelen Dravos did not write obsidiancrown in his notebook.",
        "\"Caelen Dravos knows obsidiancrown,\" the guard said.",
        "While Caelen Dravos turned away, Oren Vale quietly copied the vault passphrase obsidiancrown.",
    ] {
        assert!(
            detect_secret_narration_leak(&scene(control), &candidate).is_none(),
            "unexpected narration leak for: {control}"
        );
    }
}

#[test]
fn lexical_prefilter_is_retained_but_not_a_contradiction() {
    let text = "In the mirror, Mara eye color gleamed a vivid cerulean blue.";
    assert!(lexical_prefilter_hit(text, "Mara eye color"));
    assert!(!lexical_prefilter_hit(text, "Mara left hand prosthetic"));
}

struct FakeAdjudicator;

impl SemanticClaimAdjudicator for FakeAdjudicator {
    fn adjudicate(
        &self,
        request: &SemanticAdjudicationRequestV1,
    ) -> Result<Vec<ClaimValidationFindingV1>, String> {
        Ok(vec![ClaimValidationFindingV1 {
            check_type: "canonical_fact_prose_drift".into(),
            severity: ClaimValidationSeverity::Warning,
            message: format!("semantic finding for scene {}", request.scene_id),
            byte_range: None,
            entity_ids: Vec::new(),
            lane: ClaimValidationLane::SemanticAdjudication,
            evidence_quote: None,
            dedup_key: "semantic:1".into(),
        }])
    }
}

#[test]
fn semantic_lane_is_explicitly_degraded_without_a_route_and_runs_when_present() {
    let text = "Mara Vance flexed the five bare fingers of her flesh-and-blood left hand.";
    let context = ClaimValidationContextV1 {
        attributes: vec![attribute_fact(
            "Mara left hand prosthetic",
            "articulated iron gauntlet",
            &["Mara Vance"],
        )],
        knowledge: Vec::new(),
        secrets: Vec::new(),
    };
    let degraded = validate_scene_claims(&scene(text), &context, None);
    assert!(matches!(
        degraded.semantic_status,
        ClaimValidationLaneStatus::Degraded { .. }
    ));
    assert!(!degraded.findings.is_empty());

    let complete = validate_scene_claims(&scene(text), &context, Some(&FakeAdjudicator));
    assert_eq!(
        complete.semantic_status,
        ClaimValidationLaneStatus::Complete
    );
    assert!(
        complete
            .findings
            .iter()
            .any(|finding| finding.lane == ClaimValidationLane::SemanticAdjudication)
    );
}

struct FailingAdjudicator;

impl SemanticClaimAdjudicator for FailingAdjudicator {
    fn adjudicate(
        &self,
        _request: &SemanticAdjudicationRequestV1,
    ) -> Result<Vec<ClaimValidationFindingV1>, String> {
        Err("route not configured".into())
    }
}

#[test]
fn semantic_route_failure_is_degraded_not_clean() {
    let context = ClaimValidationContextV1 {
        attributes: Vec::new(),
        knowledge: Vec::new(),
        secrets: Vec::new(),
    };
    let report = validate_scene_claims(
        &scene("A quiet scene."),
        &context,
        Some(&FailingAdjudicator),
    );
    match report.semantic_status {
        ClaimValidationLaneStatus::Degraded { reason } => {
            assert!(reason.contains("route not configured"));
        }
        other => panic!("expected degraded, got {other:?}"),
    }
}

#[test]
fn attribute_eye_color_matches_plural_head_and_rejects_opposite_color() {
    let fact = attribute_fact("eye_color", "blue", &["Sela Voss"]);
    let finding = detect_attribute_prose_contradiction(
        &scene("Sela Voss watched the door, her eyes a warm brown in the lamplight."),
        &fact,
    )
    .expect("eye colour conflict");
    assert!(finding.message.contains("eye_color"));
    assert!(
        detect_attribute_prose_contradiction(
            &scene("Sela Voss watched the door, her eyes a pale blue."),
            &fact,
        )
        .is_none()
    );
}

#[test]
fn attribute_handedness_height_and_scar_families_are_checked() {
    let handed = attribute_fact("handedness", "right-handed", &["Sela Voss", "Sela", "Voss"]);
    assert!(
        detect_attribute_prose_contradiction(
            &scene("Sela signed the ledger with her left hand."),
            &handed,
        )
        .is_some()
    );
    assert!(
        detect_attribute_prose_contradiction(
            &scene("Sela signed the ledger with her right hand."),
            &handed,
        )
        .is_none()
    );

    let height = attribute_fact("height", "tall", &["Sela Voss", "Sela", "Voss"]);
    assert!(
        detect_attribute_prose_contradiction(
            &scene("Sela was short enough to pass under the low beam."),
            &height,
        )
        .is_some()
    );
    assert!(
        detect_attribute_prose_contradiction(
            &scene("Sela was tall enough to pass over the low beam."),
            &height,
        )
        .is_none()
    );

    let scar = attribute_fact(
        "scar",
        "a scar across her jaw",
        &["Sela Voss", "Sela", "Voss"],
    );
    assert!(
        detect_attribute_prose_contradiction(
            &scene("Sela's jaw was smooth and unblemished."),
            &scar,
        )
        .is_some()
    );
    assert!(
        detect_attribute_prose_contradiction(
            &scene("The old scar across Sela's jaw caught the light."),
            &scar,
        )
        .is_none()
    );
}

#[test]
fn knowledge_and_secret_accept_resolver_approved_forenames() {
    let knowledge = KnowledgeValidationFactV1 {
        fact_id: "knowledge_fact:first-name".into(),
        character_id: "character:caelen".into(),
        character_names: vec!["Caelen Dravos".into()],
        character_aliases: vec!["Caelen".into(), "Dravos".into()],
        fact: "the eastern gate is unlocked".into(),
        learned_book_number: 1,
        learned_chapter_number: 12,
        learned_scene_order: Some(1),
    };
    assert!(
        detect_knowledge_paraphrase_leak(
            &scene("Caelen told the guard the eastern gate was unlocked."),
            &knowledge,
        )
        .is_some()
    );

    let secret = SecretValidationCandidateV1 {
        fact_id: "canonical_fact:secret".into(),
        predicate: "vault passphrase".into(),
        value: "obsidiancrown".into(),
        character_id: "character:caelen".into(),
        character_name: "Caelen Dravos".into(),
        character_aliases: vec!["Caelen".into(), "Dravos".into()],
        other_present_character_names: vec!["Oren Vale".into()],
        other_present_character_aliases: vec!["Oren".into()],
    };
    assert!(
        detect_secret_narration_leak(
            &scene("Caelen copied obsidiancrown into the ledger."),
            &secret,
        )
        .is_some()
    );
    assert!(
        detect_secret_narration_leak(
            &scene("Oren copied obsidiancrown into the ledger while Caelen watched."),
            &secret,
        )
        .is_none()
    );
}

#[test]
fn attribute_height_accepts_a_pronoun_after_the_subject_is_established() {
    let fact = attribute_fact("height", "tall", &["Mor Tarn", "Mor"]);
    let finding = detect_attribute_prose_contradiction(
        &scene("Mor Tarn entered the hall. He was short."),
        &fact,
    )
    .expect("pronoun height conflict");
    assert!(finding.message.contains("height"));
}
