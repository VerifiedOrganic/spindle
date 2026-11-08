use super::*;

pub(crate) struct VerifiedFixtureTokenizer;
impl DraftTokenizer for VerifiedFixtureTokenizer {
    fn id(&self) -> &str {
        "fixture-unicode"
    }
    fn version(&self) -> &str {
        "1"
    }
    fn supports_model(&self, model: &str) -> bool {
        matches!(model, "fixture" | "venice-model")
    }
    fn count_text(&self, text: &str) -> Result<usize, String> {
        Ok(text.chars().count())
    }
    fn count_messages(&self, messages: &DraftMessages) -> Result<usize, String> {
        Ok(messages.system.chars().count() + messages.user.chars().count() + 17)
    }
}
pub(crate) fn fixture() -> DraftingPacketV1 {
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
                book_number: 1,
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
pub(crate) fn budget() -> PacketBudget {
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

fn receipt_for(text: &str) -> spindle_core::drafting::extraction::DraftReceiptBinding {
    spindle_core::drafting::extraction::DraftReceiptBinding {
        generation_id: "receipt1".into(),
        agent_id: "agent1".into(),
        normalized_output_sha256: format!(
            "{:x}",
            Sha256::digest(text.replace("\r\n", "\n").trim().as_bytes())
        ),
    }
}
#[test]
fn plain_text_retains_exact_utf8_whitespace_and_receipt_binding() {
    let text = " \r\n雪。\r\n ";
    let mut artifact = artifact::capture_plain_text(
        &fixture(),
        "packet-hash",
        text.into(),
        false,
        receipt_for(text),
    )
    .unwrap();
    assert_eq!(artifact.text.as_bytes(), text.as_bytes());
    assert_ne!(
        artifact.text_sha256,
        artifact.receipt.normalized_output_sha256
    );
    let hash = artifact.text_sha256.clone();
    artifact::accept_text(&mut artifact, &hash).unwrap();
    assert_eq!(
        artifact.accepted_text_sha256.as_deref(),
        Some(hash.as_str())
    );
    assert!(
        artifact::capture_plain_text(
            &fixture(),
            "hash",
            "different".into(),
            false,
            receipt_for(text)
        )
        .is_err()
    );
}
#[test]
fn truncated_or_changed_text_cannot_be_accepted() {
    let mut artifact = artifact::capture_plain_text(
        &fixture(),
        "hash",
        "partial".into(),
        true,
        receipt_for("partial"),
    )
    .unwrap();
    let hash = artifact.text_sha256.clone();
    assert!(artifact::accept_text(&mut artifact, &hash).is_err());
    artifact.truncated = false;
    artifact.text = "edited".into();
    assert!(artifact::accept_text(&mut artifact, &hash).is_err());
    assert!(artifact.accepted_text_sha256.is_none());
}

#[tokio::test]
async fn accepted_only_extraction_preserves_prose_across_failure_restart_and_retry() {
    use spindle_core::drafting::extraction::*;
    struct Extractor {
        fail: bool,
    }
    impl AcceptedTextExtractor for Extractor {
        fn extract<'a>(
            &'a self,
            request: &'a AcceptedTextExtractionRequestV1,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<CompatibilityExtractionV1, ExtractionFailure>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async move {
                let accepted = serde_json::to_value(&request.accepted).unwrap();
                assert_eq!(accepted["scene_id"], serde_json::Value::Null);
                assert_eq!(accepted["placement"]["book_number"], 1);
                assert_eq!(accepted["placement"]["chapter_number"], 1);
                assert_eq!(accepted["placement"]["scene_order"], 1);
                if self.fail {
                    return Err(ExtractionFailure {
                        code: "route_unavailable".into(),
                        retryable: true,
                    });
                }
                Ok(CompatibilityExtractionV1 {
                    canon_audit: None,
                    accepted_text_sha256: request.accepted.text_sha256.clone(),
                    summary: "A crossing fails.".into(),
                    tone: None,
                    character_states: vec![],
                    canonical_facts: vec![],
                    relationship_updates: vec![],
                    beats: vec![],
                    continuity_notes: vec![],
                })
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("draft.json");
    let text = " \r\nThe bridge fell.\r\n";
    let mut draft =
        artifact::capture_plain_text(&fixture(), "packet", text.into(), false, receipt_for(text))
            .unwrap();
    artifact::save_artifact(&path, &draft).unwrap();
    assert!(
        artifact::extract_accepted(&path, &Extractor { fail: false }, vec![], vec![])
            .await
            .is_err()
    );
    let hash = draft.text_sha256.clone();
    artifact::accept_text(&mut draft, &hash).unwrap();
    artifact::save_artifact(&path, &draft).unwrap();
    let failed = artifact::extract_accepted(&path, &Extractor { fail: true }, vec![], vec![])
        .await
        .unwrap();
    assert_eq!(failed.text.as_bytes(), text.as_bytes());
    assert!(failed.extraction.is_none());
    assert_eq!(failed.extraction_failure.unwrap().code, "route_unavailable");
    assert!(
        artifact::extract_accepted(
            &path,
            &Extractor { fail: false },
            vec![],
            vec![ExtractionEntityRef {
                id: "new-id".into(),
                kind: "character".into(),
                display_name: "New".into()
            }]
        )
        .await
        .is_err()
    );
    let restarted = artifact::load_artifact(&path).unwrap();
    let mut changed_placement = restarted.clone();
    changed_placement.placement.chapter_number = 2;
    artifact::save_artifact(&path, &changed_placement).unwrap();
    assert!(
        artifact::extract_accepted(&path, &Extractor { fail: false }, vec![], vec![])
            .await
            .is_err()
    );
    artifact::save_artifact(&path, &restarted).unwrap();
    assert_eq!(
        restarted.accepted_text_sha256.as_deref(),
        Some(hash.as_str())
    );
    let extracted = artifact::extract_accepted(&path, &Extractor { fail: false }, vec![], vec![])
        .await
        .unwrap();
    assert_eq!(extracted.text.as_bytes(), text.as_bytes());
    assert_eq!(extracted.receipt.generation_id, "receipt1");
    assert_eq!(extracted.extraction_attempts, 2);
    assert_eq!(extracted.extraction.unwrap().summary, "A crossing fails.");
    let cached = artifact::extract_accepted(&path, &Extractor { fail: true }, vec![], vec![])
        .await
        .unwrap();
    assert_eq!(cached.extraction_attempts, 2);
    assert!(cached.extraction_failure.is_none());
}

#[tokio::test]
async fn mismatched_extraction_hash_stays_parked_without_repeated_paid_attempts() {
    use spindle_core::drafting::extraction::*;
    struct Mismatch;
    impl AcceptedTextExtractor for Mismatch {
        fn extract<'a>(
            &'a self,
            _: &'a AcceptedTextExtractionRequestV1,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<CompatibilityExtractionV1, ExtractionFailure>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async {
                Ok(CompatibilityExtractionV1 {
                    canon_audit: None,
                    accepted_text_sha256: "losing-candidate".into(),
                    summary: "Wrong source".into(),
                    tone: None,
                    character_states: vec![],
                    canonical_facts: vec![],
                    relationship_updates: vec![],
                    beats: vec![],
                    continuity_notes: vec![],
                })
            })
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("draft.json");
    let text = "The bridge fell.";
    let draft =
        artifact::capture_plain_text(&fixture(), "packet", text.into(), false, receipt_for(text))
            .unwrap();
    artifact::save_artifact(&path, &draft).unwrap();
    artifact::accept_artifact(&path, &draft.text_sha256).unwrap();
    let first = artifact::extract_accepted(&path, &Mismatch, vec![], vec![])
        .await
        .unwrap();
    assert!(first.extraction.is_none());
    assert!(!first.extraction_failure.unwrap().retryable);
    let second = artifact::extract_accepted(&path, &Mismatch, vec![], vec![])
        .await
        .unwrap();
    assert_eq!(second.extraction_attempts, 1);
    assert_eq!(second.text, text);
    let lock = artifact::lock_artifact(&path).unwrap();
    assert!(
        artifact::accept_artifact(&path, &draft.text_sha256)
            .unwrap_err()
            .to_string()
            .contains("busy")
    );
    drop(lock);
    assert_eq!(
        artifact::load_artifact(&path)
            .unwrap()
            .receipt
            .generation_id,
        "receipt1"
    );
}

#[test]
fn extraction_task_identity_includes_exact_dispatched_packet_hash() {
    let text = "The bridge fell.";
    let a = artifact::capture_plain_text(
        &fixture(),
        "framing-a",
        text.into(),
        false,
        receipt_for(text),
    )
    .unwrap();
    let b = artifact::capture_plain_text(
        &fixture(),
        "framing-b",
        text.into(),
        false,
        receipt_for(text),
    )
    .unwrap();
    assert_eq!(a.text_sha256, b.text_sha256);
    assert_ne!(a.artifact_id, b.artifact_id);
}
