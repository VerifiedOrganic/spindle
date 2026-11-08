use super::*;

struct VerifiedFixtureTokenizer;
impl DraftTokenizer for VerifiedFixtureTokenizer {
    fn id(&self) -> &str {
        "fixture-unicode"
    }
    fn version(&self) -> &str {
        "1"
    }
    fn supports_model(&self, model: &str) -> bool {
        model == "fixture"
    }
    fn count_text(&self, text: &str) -> Result<usize, String> {
        Ok(text.chars().count())
    }
    fn count_messages(&self, messages: &DraftMessages) -> Result<usize, String> {
        Ok(messages.system.chars().count() + messages.user.chars().count() + 17)
    }
}
fn fixture() -> DraftingPacketV1 {
    let source = PacketSource {
        branch_id: "parent".into(),
        source_id: "plan".into(),
        revision_hash: "hash1".into(),
    };
    DraftingPacketV1 {
        schema_version: "drafting_packet_v1".into(),
        packet_id: "packet1".into(),
        project_id: "project1".into(),
        branch_id: "child".into(),
        run_id: None,
        scene_id: None,
        scene_revision: None,
        canon_snapshot_hash: "snapshot1".into(),
        plan_revision: "1".into(),
        source_versions: vec![source.clone()],
        scene_card: SceneCardV1 {
            source: source.clone(),
            pov: PacketPov {
                character_id: "pov1".into(),
                display_name: "Maë".into(),
                person: NarrativePerson::Third,
                tense: NarrativeTense::Past,
                source,
            },
            goal: "Cross the river".into(),
            opposition: "Flood".into(),
            turn: "Bridge falls".into(),
            intended_outcome: "Return home".into(),
            value_shift: ValueShift {
                axis: "hope".into(),
                before: "high".into(),
                after: "low".into(),
            },
            emotion: EmotionShift {
                entry: "eager".into(),
                exit: "afraid".into(),
            },
            chapter: PacketChapter {
                number: 1,
                scene_index: 1,
                planned_scene_count: None,
                is_closer: false,
            },
            target_words: TargetWords {
                min: 800,
                preferred: 1000,
                max: 1500,
            },
            location: PacketLocation {
                id: "river".into(),
                name: "River".into(),
            },
            story_clock: "dawn".into(),
            rating: spindle_core::models::ContentRating::General,
            tone: None,
            participants: vec![],
            required_beats: vec![],
            promises_due: vec![],
            prohibited_outcomes: vec![],
        },
        hard_constraints: vec![],
        continuity: PacketContinuity::default(),
        voice: PacketVoice::default(),
        editorial_memory: EditorialMemory::default(),
        craft_rubric: CraftRubric {
            version: "untrusted".into(),
            text: "SKILL.md manual must not be rendered".into(),
        },
        research_evidence: vec![],
        assembly: None,
    }
}
fn budget() -> PacketBudget {
    // This fixture counts Unicode scalars, not production model tokens.
    PacketBudget {
        model_id: "fixture".into(),
        input_limit: 12000,
        output_reserve: 3000,
        framing_reserve: 512,
        context_window: 20000,
        section_limits: [2000, 2500, 2500, 1500, 1500, 2000],
    }
}
fn item(packet: &mut DraftingPacketV1, id: &str, text: &str) -> PacketItem {
    let source = PacketSource {
        branch_id: "parent".into(),
        source_id: id.into(),
        revision_hash: "h".into(),
    };
    packet.source_versions.push(source.clone());
    PacketItem {
        source,
        proposition_id: id.into(),
        text: text.into(),
        relevance: 1,
        recency: 1,
    }
}
fn assemble(packet: DraftingPacketV1) -> PreparedDraftingPacket {
    assemble_packet(packet, &budget(), Some(&VerifiedFixtureTokenizer)).unwrap()
}
#[test]
fn final_count_and_hash_bind_exact_messages() {
    let result = assemble(fixture());
    let report = result.packet.assembly.unwrap();
    assert_eq!(
        report.input_tokens,
        VerifiedFixtureTokenizer
            .count_messages(&result.messages)
            .unwrap()
    );
    assert_eq!(
        result.messages_hash,
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&result.messages).unwrap())
        )
    );
    assert!(!result.messages.user.contains("SKILL.md"));
    assert!(result.messages.user.ends_with(REMINDER));
    assert!(result.messages.user.contains("Maë"));
    assert!(result.packet.voice.exemplars.is_empty());
}
#[test]
fn required_field_failures_are_stable() {
    let mut failures = Vec::new();
    for field in ["goal", "turn", "pov", "length"] {
        let mut p = fixture();
        match field {
            "goal" => p.scene_card.goal.clear(),
            "turn" => p.scene_card.turn.clear(),
            "pov" => p.scene_card.pov.character_id.clear(),
            _ => p.scene_card.target_words.min = 0,
        }
        failures.push(
            assemble_packet(p, &budget(), Some(&VerifiedFixtureTokenizer))
                .unwrap_err()
                .to_string(),
        );
    }
    assert_eq!(
        failures,
        [
            "incomplete_packet: goal",
            "incomplete_packet: turn",
            "incomplete_packet: pov.character_id",
            "incomplete_packet: target_words: require 0 < min <= preferred <= max"
        ]
    );
    let mut value = serde_json::to_value(fixture()).unwrap();
    value["scene_card"]
        .as_object_mut()
        .unwrap()
        .remove("rating");
    assert!(
        serde_json::from_value::<DraftingPacketV1>(value.clone())
            .unwrap_err()
            .to_string()
            .contains("missing field `rating`")
    );
    value["scene_card"]["rating"] = serde_json::json!("invalid");
    assert!(
        serde_json::from_value::<DraftingPacketV1>(value.clone())
            .unwrap_err()
            .to_string()
            .contains("invalid content rating")
    );
    value["scene_card"]["rating"] = serde_json::json!("explicit");
    value["scene_card"].as_object_mut().unwrap().remove("pov");
    assert!(
        serde_json::from_value::<DraftingPacketV1>(value)
            .unwrap_err()
            .to_string()
            .contains("missing field `pov`")
    );
}
#[test]
fn unknown_tokenizer_never_estimates() {
    assert!(matches!(
        assemble_packet(fixture(), &budget(), None),
        Err(PacketError::BudgetUnknown(_))
    ));
    let mut b = budget();
    b.model_id = "other-model".into();
    assert!(matches!(
        assemble_packet(fixture(), &b, Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::BudgetUnknown(_))
    ));
}
#[test]
fn ordering_dedup_and_utf8_whole_records() {
    let mut p = fixture();
    let a = item(&mut p, "a", "雪が降る。🌨️");
    let b = item(&mut p, "b", "Après la pluie.");
    p.continuity.facts = vec![b.clone(), a.clone(), a];
    let first = assemble(p.clone());
    p.continuity.facts.reverse();
    p.source_versions.reverse();
    let second = assemble(p);
    assert_eq!(first.messages, second.messages);
    assert_eq!(first.messages_hash, second.messages_hash);
    assert_eq!(first.messages.user.matches("雪が降る。🌨️").count(), 1);
    assert_eq!(
        first.packet.assembly.unwrap().omissions[0].reason,
        OmissionReason::Duplicate
    );
    let mut p = fixture();
    let huge = item(&mut p, "huge", &"雪🌨️".repeat(5000));
    p.research_evidence.push(huge);
    let prepared = assemble(p);
    assert!(!prepared.messages.user.contains("雪"));
    assert_eq!(
        prepared.packet.assembly.unwrap().omissions[0].reason,
        OmissionReason::SectionBudget
    );
}
#[test]
fn mandatory_and_final_overflow_stop() {
    let mut p = fixture();
    let huge = item(&mut p, "knowledge", &"x".repeat(6000));
    p.continuity.knowledge_boundaries.push(huge);
    assert!(matches!(
        assemble_packet(p, &budget(), Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::MandatoryOverflow(_))
    ));
    let mut b = budget();
    b.context_window = 1;
    assert!(matches!(
        assemble_packet(fixture(), &b, Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::MandatoryOverflow(_))
    ));
    let mut b = budget();
    b.input_limit = 1;
    assert!(matches!(
        assemble_packet(fixture(), &b, Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::MandatoryOverflow(_))
    ));
}
#[test]
fn stale_sources_and_unapproved_examples_fail() {
    let mut p = fixture();
    p.scene_card.source.revision_hash = "stale".into();
    assert!(matches!(
        assemble_packet(p, &budget(), Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::Incomplete(_))
    ));
    let mut p = fixture();
    let excerpt = item(&mut p, "excerpt", "Approved prose.");
    p.voice.exemplars.push(ApprovedExemplar {
        item: excerpt,
        speaker_id: None,
        approved_revision_hash: "different".into(),
    });
    assert!(matches!(
        assemble_packet(p, &budget(), Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::Incomplete(_))
    ));
}
#[test]
fn policy_defaults_to_legacy() {
    assert_eq!(EditorialPolicy::default(), EditorialPolicy::Legacy);
}

#[test]
fn source_can_support_multiple_propositions_but_conflicts_fail() {
    let mut p = fixture();
    let a = item(&mut p, "source", "First fact.");
    let mut b = a.clone();
    b.proposition_id = "another".into();
    b.text = "Second fact.".into();
    p.continuity.facts = vec![a.clone(), b];
    let messages = assemble(p.clone()).messages.user;
    assert!(messages.contains("First fact."));
    assert!(messages.contains("Second fact."));
    let mut conflict = a;
    conflict.text = "Contradictory fact.".into();
    p.continuity.facts.push(conflict);
    assert!(matches!(
        assemble_packet(p, &budget(), Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::Incomplete(_))
    ));
}

#[test]
fn optional_records_respect_final_message_overhead() {
    let base = assemble(fixture());
    let mut b = budget();
    b.input_limit = base.packet.assembly.unwrap().input_tokens + 5;
    let mut p = fixture();
    let optional = item(&mut p, "extra", "This optional fact exceeds the final cap.");
    p.continuity.facts.push(optional);
    let result = assemble_packet(p, &b, Some(&VerifiedFixtureTokenizer)).unwrap();
    assert_eq!(result.messages, base.messages);
    assert_eq!(result.packet.assembly.unwrap().omissions.len(), 1);
}

#[test]
fn contradictory_author_lock_ids_fail() {
    let mut p = fixture();
    let lock = PacketConstraint {
        id: "ending".into(),
        kind: ConstraintKind::AuthorLock,
        statement: "Stay home".into(),
        source: p.scene_card.source.clone(),
        scope: "scene".into(),
    };
    let mut conflict = lock.clone();
    conflict.statement = "Leave home".into();
    p.hard_constraints = vec![lock, conflict];
    assert!(matches!(
        assemble_packet(p, &budget(), Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::Incomplete(_))
    ));
}

#[test]
fn hard_constraints_never_truncate_and_unknown_usage_fails() {
    let mut p = fixture();
    p.hard_constraints.push(PacketConstraint {
        id: "required".into(),
        kind: ConstraintKind::Canon,
        statement: "é".repeat(6000),
        source: p.scene_card.source.clone(),
        scope: "scene".into(),
    });
    assert!(matches!(
        assemble_packet(p, &budget(), Some(&VerifiedFixtureTokenizer)),
        Err(PacketError::MandatoryOverflow(_))
    ));
    struct Broken;
    impl DraftTokenizer for Broken {
        fn id(&self) -> &str {
            "broken"
        }
        fn version(&self) -> &str {
            "1"
        }
        fn supports_model(&self, _: &str) -> bool {
            true
        }
        fn count_text(&self, _: &str) -> Result<usize, String> {
            Err("unavailable".into())
        }
        fn count_messages(&self, _: &DraftMessages) -> Result<usize, String> {
            Err("unavailable".into())
        }
    }
    assert!(matches!(
        assemble_packet(fixture(), &budget(), Some(&Broken)),
        Err(PacketError::BudgetUnknown(_))
    ));
}

#[test]
fn proposition_ids_are_local_to_the_full_branch_aware_source() {
    for same_text in [false, true] {
        for changed_field in ["source_id", "branch_id", "revision_hash"] {
            let mut p = fixture();
            let a = item(&mut p, "character-a", "First character state.");
            let mut b = a.clone();
            b.text = if same_text {
                a.text.clone()
            } else {
                "Second character state.".into()
            };
            match changed_field {
                "source_id" => b.source.source_id = "character-b".into(),
                "branch_id" => b.source.branch_id = "child".into(),
                _ => b.source.revision_hash = "other-revision".into(),
            }
            p.source_versions.push(b.source.clone());
            p.continuity.character_states = vec![a.clone(), b.clone()];
            let prepared = assemble(p.clone());
            assert!(prepared.packet.assembly.unwrap().omissions.is_empty());
            assert!(prepared.messages.user.contains(&json(&a).unwrap()));
            assert!(prepared.messages.user.contains(&json(&b).unwrap()));
            p.continuity.character_states.reverse();
            assert_eq!(prepared.messages, assemble(p).messages);
        }
    }
}
