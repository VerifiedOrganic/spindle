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
        generation_seed: None,
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

pub(crate) fn valid_critique_fixture(
    base_text: &str,
    base_text_hash: &str,
    packet_hash: &str,
    scene_id: Option<String>,
) -> CritiqueV1 {
    let quote_end = base_text.lines().next().unwrap_or(base_text).len();
    let exact_quote = base_text[..quote_end].to_string();
    let evidence_anchor = CritiqueAnchor {
        scene_id: scene_id.clone(),
        base_text_hash: base_text_hash.into(),
        start_byte: 0,
        end_byte: quote_end,
        exact_quote: exact_quote.clone(),
        context_before: None,
        context_after: None,
    };
    let dimensions = CriticDimension::ALL
        .iter()
        .map(|&dim| CriticDimensionScore {
            dimension: dim,
            score_1_to_5: 4,
            evidence_spans: vec![evidence_anchor.clone()],
            rationale: format!("Solid {} execution.", dim.as_str()),
        })
        .collect();
    CritiqueV1 {
        schema_version: CRITIQUE_SCHEMA_VERSION.into(),
        base_text_hash: base_text_hash.into(),
        packet_hash: packet_hash.into(),
        rubric_version: critique::CRITIC_RUBRIC_VERSION.into(),
        dimensions,
        directives: vec![CritiqueDirective {
            id: "dir-1".into(),
            dimension: CriticDimension::TensionTurn,
            severity: DirectiveSeverity::Major,
            priority: 1,
            anchor: evidence_anchor,
            diagnosis: "The opening inventories gear before the flood wave hits.".into(),
            requested_change: "Start directly with the timber groaning under the flood.".into(),
            preserve: vec!["Maë's sensory focus on the cold spray".into()],
            success_test: "Opening sentence places Maë in immediate physical motion.".into(),
            source_finding_ids: vec!["f-1".into()],
            scope: DirectiveScope::Span,
        }],
        strengths_to_preserve: vec!["Sharp river imagery.".into()],
        requires_author_decision: vec![],
        no_edit_reason: None,
        no_change: None,
    }
}

#[test]
fn critique_v1_validates_utf8_byte_spans_quotes_and_zero_width_insertions() {
    use critique::*;
    let base_text = "Maë watched 雪が降る across the swollen river.\nThe bridge groaned.";
    let base_hash = format!("{:x}", Sha256::digest(base_text.as_bytes()));
    let mut pkt = fixture();
    pkt.scene_id = Some("scene-1".into());

    // Valid multi-byte UTF-8 span ("雪が降る")
    let snow_start = base_text.find("雪が降る").unwrap();
    let snow_end = snow_start + "雪が降る".len();
    let valid_anchor = CritiqueAnchor {
        scene_id: Some("scene-1".into()),
        base_text_hash: base_hash.clone(),
        start_byte: snow_start,
        end_byte: snow_end,
        exact_quote: "雪が降る".into(),
        context_before: None,
        context_after: None,
    };
    assert!(validate_anchor(&valid_anchor, base_text, &base_hash, Some("scene-1")).is_ok());

    // Mid-codepoint non-char-boundary offset must fail
    let mut bad_boundary = valid_anchor.clone();
    bad_boundary.start_byte = snow_start + 1; // inside 3-byte UTF-8 '雪'
    assert!(matches!(
        validate_anchor(&bad_boundary, base_text, &base_hash, Some("scene-1")),
        Err(CritiqueError::NonCharBoundaryAnchor { .. })
    ));

    // Quote mismatch must fail
    let mut bad_quote = valid_anchor.clone();
    bad_quote.exact_quote = "wrong quote".into();
    assert!(matches!(
        validate_anchor(&bad_quote, base_text, &base_hash, Some("scene-1")),
        Err(CritiqueError::AnchorQuoteMismatch { .. })
    ));

    // Stale base_text_hash must fail
    let mut stale_hash = valid_anchor.clone();
    stale_hash.base_text_hash = "stale-hash".into();
    assert!(matches!(
        validate_anchor(&stale_hash, base_text, &base_hash, Some("scene-1")),
        Err(CritiqueError::StaleAnchorHash { .. })
    ));

    // Mismatched scene_id must fail
    let mut wrong_scene = valid_anchor.clone();
    wrong_scene.scene_id = Some("scene-other".into());
    assert!(matches!(
        validate_anchor(&wrong_scene, base_text, &base_hash, Some("scene-1")),
        Err(CritiqueError::AnchorSceneMismatch { .. })
    ));

    // Out-of-bounds byte span must fail
    let mut out_of_bounds = valid_anchor.clone();
    out_of_bounds.end_byte = base_text.len() + 4;
    assert!(matches!(
        validate_anchor(&out_of_bounds, base_text, &base_hash, Some("scene-1")),
        Err(CritiqueError::InvalidAnchorSpan { .. })
    ));

    // Zero-width insertion requires matching context_before or context_after
    let mut zero_width = CritiqueAnchor {
        scene_id: Some("scene-1".into()),
        base_text_hash: base_hash.clone(),
        start_byte: snow_start,
        end_byte: snow_start,
        exact_quote: String::new(),
        context_before: None,
        context_after: None,
    };
    assert!(matches!(
        validate_anchor(&zero_width, base_text, &base_hash, Some("scene-1")),
        Err(CritiqueError::UnanchoredInsertion { .. })
    ));

    zero_width.context_before = Some("Maë watched ".into());
    zero_width.context_after = Some("雪が降る".into());
    assert!(validate_anchor(&zero_width, base_text, &base_hash, Some("scene-1")).is_ok());

    zero_width.context_after = Some("mismatched context".into());
    assert!(matches!(
        validate_anchor(&zero_width, base_text, &base_hash, Some("scene-1")),
        Err(CritiqueError::UnanchoredInsertion { .. })
    ));
}

#[test]
fn critique_resolves_quote_first_offsets_and_rejects_ambiguous_quotes() {
    use critique::*;
    let base_text = "The river rose. The bridge fell.";
    let base_hash = sha256_hex(base_text.as_bytes());
    let mut pkt = fixture();
    pkt.scene_id = Some("scene-quote".into());

    let mut raw = valid_critique_fixture(
        base_text,
        &base_hash,
        "pkt-quote",
        Some("scene-quote".into()),
    );
    raw.directives[0].anchor.start_byte = 0;
    raw.directives[0].anchor.end_byte = 1;
    raw.directives[0].anchor.exact_quote = "river rose".into();
    let validated =
        validate_and_normalize_critique(raw, &pkt, "pkt-quote", base_text, &base_hash).unwrap();
    let expected_start = base_text.find("river rose").unwrap();
    assert_eq!(validated.directives[0].anchor.start_byte, expected_start);
    assert_eq!(
        validated.directives[0].anchor.end_byte,
        expected_start + "river rose".len()
    );

    // A repeated quote with no disambiguating context is an honest typed failure.
    let mut ambiguous = valid_critique_fixture(
        base_text,
        &base_hash,
        "pkt-quote",
        Some("scene-quote".into()),
    );
    ambiguous.directives[0].anchor.start_byte = 0;
    ambiguous.directives[0].anchor.end_byte = 1;
    ambiguous.directives[0].anchor.exact_quote = "The ".into();
    ambiguous.directives[0].anchor.context_before = None;
    ambiguous.directives[0].anchor.context_after = None;
    let error =
        validate_and_normalize_critique(ambiguous, &pkt, "pkt-quote", base_text, &base_hash)
            .unwrap_err();
    assert!(matches!(error, CritiqueError::AmbiguousAnchorQuote { .. }));
}

#[test]
fn critique_requires_typed_no_change_and_rejects_contradictions() {
    use critique::*;
    let base_text = "The river rose.";
    let base_hash = sha256_hex(base_text.as_bytes());
    let mut pkt = fixture();
    pkt.scene_id = Some("scene-no-change".into());
    let raw = valid_critique_fixture(
        base_text,
        &base_hash,
        "pkt-no-change",
        Some("scene-no-change".into()),
    );
    let directive = raw.directives[0].clone();
    let mut no_change = raw.clone();
    no_change.directives.clear();
    no_change.no_change = Some(spindle_core::drafting::NoChangeV1 {
        reason_code: spindle_core::drafting::NoChangeReason::NoMaterialIssue,
        explanation: "All dimensions are publication-ready.".into(),
    });
    for dimension in &mut no_change.dimensions {
        dimension.score_1_to_5 = 5;
    }
    let validated = validate_and_normalize_critique(
        no_change.clone(),
        &pkt,
        "pkt-no-change",
        base_text,
        &base_hash,
    )
    .unwrap();
    assert!(validated.directives.is_empty());
    assert_eq!(
        validated.no_edit_reason.as_deref(),
        Some("All dimensions are publication-ready.")
    );

    let mut action_pkt = pkt.clone();
    action_pkt.scene_card.value_shift.axis = "action".into();
    let mut action_no_change = no_change.clone();
    action_no_change.no_change = Some(spindle_core::drafting::NoChangeV1 {
        reason_code: spindle_core::drafting::NoChangeReason::AlreadySatisfiesBrief,
        explanation: "The action scene is already strong.".into(),
    });
    assert!(matches!(
        validate_and_normalize_critique(
            action_no_change,
            &action_pkt,
            "pkt-no-change",
            base_text,
            &base_hash
        ),
        Err(CritiqueError::InvalidDirective(_))
    ));

    let mut missing = no_change.clone();
    missing.no_change = None;
    assert!(matches!(
        validate_and_normalize_critique(missing, &pkt, "pkt-no-change", base_text, &base_hash),
        Err(CritiqueError::InvalidDirective(_))
    ));

    let mut low_score = no_change.clone();
    low_score.dimensions[0].score_1_to_5 = 4;
    assert!(matches!(
        validate_and_normalize_critique(low_score, &pkt, "pkt-no-change", base_text, &base_hash),
        Err(CritiqueError::InvalidDirective(_))
    ));

    let mut contradictory = no_change;
    contradictory.directives = vec![directive];
    assert!(matches!(
        validate_and_normalize_critique(
            contradictory,
            &pkt,
            "pkt-no-change",
            base_text,
            &base_hash
        ),
        Err(CritiqueError::InvalidDirective(_))
    ));
}

#[test]
fn critic_prompt_uses_scene_specific_focus_and_typed_no_change_contract() {
    use critique::*;
    let base_text = "The river rose.";
    let base_hash = sha256_hex(base_text.as_bytes());
    let mut pkt = fixture();
    pkt.scene_id = Some("scene-focus".into());
    let budget = CriticBudget::scene_default("fixture".into(), 20_000);

    pkt.scene_card.value_shift.axis = "quiet-interior".into();
    let (quiet, _, _) = assemble_critic_messages(
        &pkt,
        "pkt-focus",
        base_text,
        &base_hash,
        &[],
        &budget,
        Some(&VerifiedFixtureTokenizer),
        "",
    )
    .unwrap();
    assert!(quiet.user.contains("QUIET-INTERIOR"));
    assert!(
        quiet
            .user
            .contains("DIRECTIVE QUALITY & NO-CHANGE CONTRACT")
    );
    assert!(quiet.user.contains("no_change"));

    pkt.scene_card.value_shift.axis = "action".into();
    let (action, _, _) = assemble_critic_messages(
        &pkt,
        "pkt-focus",
        base_text,
        &base_hash,
        &[],
        &budget,
        Some(&VerifiedFixtureTokenizer),
        "",
    )
    .unwrap();
    assert!(action.user.contains("ACTION:"));
}

#[test]
fn feedback_ingestion_deduplicates_deterministic_and_reader_findings_and_merges_sources() {
    use critique::*;
    let base_text = "Maë stepped onto the collapsing span and called to the sentry.";
    let base_hash = format!("{:x}", Sha256::digest(base_text.as_bytes()));
    let anchor = CritiqueAnchor {
        scene_id: Some("scene-1".into()),
        base_text_hash: base_hash.clone(),
        start_byte: 0,
        end_byte: 36,
        exact_quote: base_text[..36].into(),
        context_before: None,
        context_after: None,
    };

    let f_det = CriticFindingInput {
        finding_id: "det-1".into(),
        source: FeedbackSourceKind::Continuity,
        check_type: "timeline_coherence".into(),
        dimension: CriticDimension::Continuity,
        severity: DirectiveSeverity::Major,
        message: "Bridge span already collapsed before Maë stepped onto it.".into(),
        anchor: Some(anchor.clone()),
        suggested_fix: None,
    };
    let f_reader = CriticFindingInput {
        finding_id: "reader-1".into(),
        source: FeedbackSourceKind::ReaderPersona,
        check_type: "reader_confusion".into(),
        dimension: CriticDimension::Continuity,
        severity: DirectiveSeverity::Blocker,
        message: "Bridge span already collapsed before Maë stepped onto it!".into(),
        anchor: Some(anchor),
        suggested_fix: Some("Show Maë leaping from the remaining pier.".into()),
    };

    let (deduped, omitted_ids) = ingest_and_dedup_findings(
        vec![f_det, f_reader],
        base_text,
        &base_hash,
        Some("scene-1"),
    )
    .unwrap();
    assert_eq!(deduped.len(), 1);
    assert_eq!(omitted_ids, vec!["reader-1"]);
    assert_eq!(deduped[0].merged_finding_ids, vec!["det-1", "reader-1"]);
    assert_eq!(deduped[0].severity, DirectiveSeverity::Blocker);
    assert_eq!(
        deduped[0].sources,
        vec![
            FeedbackSourceKind::Continuity,
            FeedbackSourceKind::ReaderPersona
        ]
    );
    assert_eq!(
        deduped[0].suggested_fix.as_deref(),
        Some("Show Maë leaping from the remaining pier.")
    );
}

#[test]
fn critique_quarantines_author_lock_prohibited_outcome_and_canon_conflicts_and_caps_directives_at_five()
 {
    use critique::*;
    assert!(CRITIC_RUBRIC.contains("0.36"));
    assert!(CRITIC_RUBRIC.contains("0.42"));
    assert!(CRITIC_RUBRIC.contains("0.56"));

    let base_text = "Paragraph one opens on the river.\nParagraph two tests the bridge.\nParagraph three turns on the flood.\nParagraph four holds the bank.\nParagraph five watches the lantern.\nParagraph six closes in silence.";
    let base_hash = format!("{:x}", Sha256::digest(base_text.as_bytes()));
    let mut pkt = fixture();
    pkt.scene_id = Some("scene-1".into());
    pkt.scene_card.prohibited_outcomes = vec!["Maë crosses safely to the far tower".into()];
    pkt.hard_constraints.push(PacketConstraint {
        id: "lock-bridge".into(),
        kind: ConstraintKind::AuthorLock,
        statement: "The timber bridge collapses before anyone reaches the far bank".into(),
        source: pkt.scene_card.source.clone(),
        scope: "scene".into(),
    });

    let mut raw =
        valid_critique_fixture(base_text, &base_hash, "pkt-hash-1", Some("scene-1".into()));
    raw.directives.clear();

    let lines: Vec<&str> = base_text.lines().collect();
    let mut byte_cursor = 0usize;
    let dims = [
        (CriticDimension::Freshness, DirectiveSeverity::Polish, 4),
        (CriticDimension::Voice, DirectiveSeverity::Minor, 3),
        (CriticDimension::Dialogue, DirectiveSeverity::Minor, 2),
        (CriticDimension::Length, DirectiveSeverity::Polish, 5),
        (CriticDimension::ScenePurpose, DirectiveSeverity::Major, 2),
        (CriticDimension::Continuity, DirectiveSeverity::Blocker, 1),
    ];
    for (idx, (line, (dim, sev, prio))) in lines.iter().zip(dims.iter()).enumerate() {
        let start = byte_cursor;
        let end = start + line.len();
        byte_cursor = end + 1;
        raw.directives.push(CritiqueDirective {
            id: format!("dir-{}", idx + 1),
            dimension: *dim,
            severity: *sev,
            priority: *prio,
            anchor: CritiqueAnchor {
                scene_id: Some("scene-1".into()),
                base_text_hash: base_hash.clone(),
                start_byte: start,
                end_byte: end,
                exact_quote: (*line).into(),
                context_before: None,
                context_after: None,
            },
            diagnosis: format!("Distinct diagnosis number {} on {}", idx + 1, dim.as_str()),
            requested_change: format!("Targeted revision number {} on {}", idx + 1, line),
            preserve: vec![],
            success_test: format!("Verify fix {}", idx + 1),
            source_finding_ids: vec![],
            scope: DirectiveScope::Span,
        });
    }

    // Add a directive that conflicts with AuthorLock ("lock-bridge")
    let first_line = lines[0];
    raw.directives.push(CritiqueDirective {
        id: "dir-lock-conflict".into(),
        dimension: CriticDimension::TensionTurn,
        severity: DirectiveSeverity::Major,
        priority: 1,
        anchor: CritiqueAnchor {
            scene_id: Some("scene-1".into()),
            base_text_hash: base_hash.clone(),
            start_byte: 0,
            end_byte: first_line.len(),
            exact_quote: first_line.into(),
            context_before: None,
            context_after: None,
        },
        diagnosis: "The timber bridge collapses too soon (lock:lock-bridge).".into(),
        requested_change: "Instead of collapsing the timber bridge, keep it standing.".into(),
        preserve: vec![],
        success_test: "Bridge stays up.".into(),
        source_finding_ids: vec!["f-lock".into()],
        scope: DirectiveScope::Scene,
    });

    // Add a directive that requests a prohibited outcome
    raw.directives.push(CritiqueDirective {
        id: "dir-prohibited".into(),
        dimension: CriticDimension::ScenePurpose,
        severity: DirectiveSeverity::Major,
        priority: 1,
        anchor: CritiqueAnchor {
            scene_id: Some("scene-1".into()),
            base_text_hash: base_hash.clone(),
            start_byte: 0,
            end_byte: first_line.len(),
            exact_quote: first_line.into(),
            context_before: None,
            context_after: None,
        },
        diagnosis: "Ending feels bleak.".into(),
        requested_change: "Have Maë crosses safely to the far tower at the end.".into(),
        preserve: vec![],
        success_test: "She reaches the tower.".into(),
        source_finding_ids: vec![],
        scope: DirectiveScope::Scene,
    });

    // Add a chapter-scoped directive whose text contains none of the keyword conflict markers
    raw.directives.push(CritiqueDirective {
        id: "dir-chapter-scope".into(),
        dimension: CriticDimension::ChapterRole,
        severity: DirectiveSeverity::Minor,
        priority: 2,
        anchor: CritiqueAnchor {
            scene_id: Some("scene-1".into()),
            base_text_hash: base_hash.clone(),
            start_byte: 0,
            end_byte: first_line.len(),
            exact_quote: first_line.into(),
            context_before: None,
            context_after: None,
        },
        diagnosis: "Pacing slows after the crossing.".into(),
        requested_change: "Tighten pacing in the surrounding chapter.".into(),
        preserve: vec![],
        success_test: "Transition into next scene is brisk.".into(),
        source_finding_ids: vec![],
        scope: DirectiveScope::Chapter,
    });

    let validated =
        validate_and_normalize_critique(raw, &pkt, "pkt-hash-1", base_text, &base_hash).unwrap();

    // All three conflicting/chapter-scoped directives were quarantined into requires_author_decision
    assert_eq!(validated.requires_author_decision.len(), 3);
    assert_eq!(
        validated.requires_author_decision[0]
            .blocking_constraint_id
            .as_deref(),
        Some("lock-bridge")
    );
    assert_eq!(
        validated.requires_author_decision[1]
            .blocking_constraint_id
            .as_deref(),
        Some("scene_card.prohibited_outcomes")
    );
    assert_eq!(
        validated.requires_author_decision[2]
            .blocking_constraint_id
            .as_deref(),
        Some("directive.scope:chapter")
    );

    // Actionable directives capped at MAX_ACTIONABLE_DIRECTIVES (5) and ordered with Continuity first, ScenePurpose second
    assert_eq!(validated.directives.len(), MAX_ACTIONABLE_DIRECTIVES);
    assert_eq!(
        validated.directives[0].dimension,
        CriticDimension::Continuity
    );
    assert_eq!(
        validated.directives[1].dimension,
        CriticDimension::ScenePurpose
    );
}

#[test]
fn critic_prompt_embeds_complete_json_schema_and_template_that_validates_without_rust_fixture() {
    use critique::*;
    let base_text = "Maë braced against the floodwater as the bridge timber cracked.\nShe reached for the rope.";
    let base_hash = sha256_hex(base_text.as_bytes());
    let mut pkt = fixture();
    pkt.scene_id = Some("scene-contract-1".into());
    let src = pkt.source_versions[0].clone();
    let mk_item = |prop_id: &str, text: &str| PacketItem {
        source: src.clone(),
        proposition_id: prop_id.into(),
        text: text.into(),
        relevance: 90,
        recency: 10,
    };
    pkt.continuity.knowledge_boundaries = vec![mk_item(
        "kb-sentinel",
        "SENTINEL_KB_MAE_DOES_NOT_KNOW_SLUICE_CODE",
    )];
    pkt.continuity.character_states = vec![mk_item(
        "cs-sentinel",
        "SENTINEL_CS_MAE_LEFT_WRIST_SPRAINED",
    )];
    pkt.continuity.facts = vec![mk_item(
        "fact-sentinel",
        "SENTINEL_FACT_RIVER_CREST_AT_DUSK",
    )];
    pkt.continuity.open_threads = vec![mk_item(
        "thread-sentinel",
        "SENTINEL_THREAD_MISSING_LANTERN_SIGNAL",
    )];
    pkt.continuity.prior_scene_tail =
        Some(mk_item("tail-sentinel", "SENTINEL_TAIL_BELL_TOLLED_TWICE"));
    pkt.continuity.story_so_far = vec![mk_item("ssf-sentinel", "SENTINEL_SSF_NORTH_WEIR_BREACHED")];
    pkt.voice.rules = vec![mk_item(
        "voice-sentinel",
        "SENTINEL_VOICE_TACTILE_VERBS_NO_ADVERBS",
    )];
    pkt.voice.exemplars = vec![ApprovedExemplar {
        item: mk_item("ex-sentinel", "SENTINEL_EXEMPLAR_COLD_IRON_RIVETS"),
        speaker_id: Some("pov1".into()),
        approved_revision_hash: src.revision_hash.clone(),
    }];
    pkt.editorial_memory.recent_openings = vec![mk_item(
        "open-sentinel",
        "SENTINEL_MEM_OPENING_RAIN_ON_SLATE",
    )];
    pkt.editorial_memory.recurring_images =
        vec![mk_item("img-sentinel", "SENTINEL_MEM_IMAGE_RUSTED_WINCH")];
    pkt.editorial_memory.pacing_history = vec![mk_item(
        "pace-sentinel",
        "SENTINEL_MEM_PACING_TWO_SLOW_SCENES",
    )];

    let mut critic_budget = CriticBudget::scene_default("fixture".into(), 32_000);
    critic_budget.input_limit = 24_000;

    let (initial_msgs, _, _) = assemble_critic_messages(
        &pkt,
        "pkt-hash-contract",
        base_text,
        &base_hash,
        &[],
        &critic_budget,
        Some(&VerifiedFixtureTokenizer),
        "Route system framing.",
    )
    .unwrap();

    // Verify all packet continuity, voice, and editorial memory sentinels reach the critic prompt
    for sentinel in [
        "KNOWLEDGE BOUNDARIES (MANDATORY)",
        "CONTINUITY CONTEXT",
        "VOICE & STYLE CONTEXT",
        "EDITORIAL MEMORY & STORY CONTEXT",
        "SENTINEL_KB_MAE_DOES_NOT_KNOW_SLUICE_CODE",
        "SENTINEL_CS_MAE_LEFT_WRIST_SPRAINED",
        "SENTINEL_FACT_RIVER_CREST_AT_DUSK",
        "SENTINEL_THREAD_MISSING_LANTERN_SIGNAL",
        "SENTINEL_TAIL_BELL_TOLLED_TWICE",
        "SENTINEL_SSF_NORTH_WEIR_BREACHED",
        "SENTINEL_VOICE_TACTILE_VERBS_NO_ADVERBS",
        "SENTINEL_EXEMPLAR_COLD_IRON_RIVETS",
        "SENTINEL_MEM_OPENING_RAIN_ON_SLATE",
        "SENTINEL_MEM_IMAGE_RUSTED_WINCH",
        "SENTINEL_MEM_PACING_TWO_SLOW_SCENES",
    ] {
        assert!(
            initial_msgs.user.contains(sentinel),
            "initial critic prompt missing packet context sentinel {sentinel:?}"
        );
    }

    // Verify mandatory context overflow fails explicitly with CritiqueError::MandatoryOverflow
    let mut tiny_budget = critic_budget.clone();
    tiny_budget.input_limit = 200;
    let overflow_err = assemble_critic_messages(
        &pkt,
        "pkt-hash-contract",
        base_text,
        &base_hash,
        &[],
        &tiny_budget,
        Some(&VerifiedFixtureTokenizer),
        "Route system framing.",
    )
    .unwrap_err();
    assert!(matches!(overflow_err, CritiqueError::MandatoryOverflow(_)));

    // Verify all required field names and enum values are present in the initial prompt
    for required_token in [
        "CRITIQUE_V1 FIELD & ENUM REQUIREMENTS",
        "CRITIQUE_V1 OUTPUT TEMPLATE",
        "CRITIQUE_V1 JSON SCHEMA",
        "score_1_to_5",
        "evidence_spans",
        "rationale",
        "start_byte",
        "end_byte",
        "exact_quote",
        "context_before",
        "context_after",
        "priority",
        "diagnosis",
        "requested_change",
        "preserve",
        "success_test",
        "source_finding_ids",
        "scope",
        "strengths_to_preserve",
        "requires_author_decision",
        "blocking_constraint_id",
        "proposed_change",
        "no_edit_reason",
        "no_change",
        "reason_code",
        "SCENE-TYPE FOCUS",
        "DIRECTIVE QUALITY & NO-CHANGE CONTRACT",
        "blocker",
        "major",
        "minor",
        "polish",
        "span",
        "scene",
        "chapter",
    ] {
        assert!(
            initial_msgs.user.contains(required_token),
            "initial critic prompt missing contract token {required_token:?}"
        );
    }
    for dim in CriticDimension::ALL {
        assert!(
            initial_msgs.user.contains(dim.as_str()),
            "initial critic prompt missing dimension {:?}",
            dim.as_str()
        );
    }

    // Extract the embedded JSON Schema from the prompt and verify it parses as a valid JSON Schema
    let schema_section = initial_msgs
        .user
        .split("CRITIQUE_V1 JSON SCHEMA\n")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    let parsed_schema: serde_json::Value = serde_json::from_str(schema_section).unwrap();
    assert_eq!(parsed_schema, critique_v1_json_schema());

    // Simulate a model that only sees the prompt: extract the CRITIQUE_V1 OUTPUT TEMPLATE and
    // EXACT BASE DRAFT UTF-8 directly from the prompt text, populate the anchor span from the
    // prompt's draft text, and verify that parse_and_validate_critique_output accepts it.
    let template_line = initial_msgs
        .user
        .split("CRITIQUE_V1 OUTPUT TEMPLATE\n")
        .nth(1)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    let prompt_draft = initial_msgs
        .user
        .split("EXACT BASE DRAFT UTF-8\n")
        .nth(1)
        .unwrap()
        .trim_end_matches('\n');
    assert_eq!(prompt_draft, base_text);

    let mut model_output: serde_json::Value = serde_json::from_str(template_line).unwrap();
    let first_sentence = prompt_draft.lines().next().unwrap();
    model_output["directives"][0]["anchor"]["start_byte"] = serde_json::json!(0);
    model_output["directives"][0]["anchor"]["end_byte"] = serde_json::json!(first_sentence.len());
    model_output["directives"][0]["anchor"]["exact_quote"] = serde_json::json!(first_sentence);
    model_output["requires_author_decision"] = serde_json::json!([]);

    let synthesized_json = serde_json::to_string(&model_output).unwrap();
    let validated = parse_and_validate_critique_output(
        &synthesized_json,
        &pkt,
        "pkt-hash-contract",
        base_text,
        &base_hash,
    )
    .expect("response synthesized purely from prompt template must validate");
    assert_eq!(validated.dimensions.len(), 10);
    assert_eq!(validated.directives.len(), 1);
    assert_eq!(validated.directives[0].anchor.exact_quote, first_sentence);

    // The repair prompt is compact: it keeps the scene context and base draft,
    // not a duplicate of the full schema/template block.
    let (repair_msgs, _, _) = assemble_critic_repair_messages(
        &initial_msgs,
        &pkt,
        "pkt-hash-contract",
        base_text,
        &base_hash,
        "{\"broken\":true}",
        &CritiqueError::MalformedJson("missing fields".into()),
        &critic_budget,
        Some(&VerifiedFixtureTokenizer),
    )
    .unwrap();
    assert!(repair_msgs.user.contains("CRITIQUE_V1 REPAIR"));
    assert!(repair_msgs.user.contains("REPAIR CONTRACT"));
    assert!(repair_msgs.user.contains("SCENE CARD"));
    assert!(repair_msgs.user.contains("EXACT BASE DRAFT UTF-8"));
    assert!(repair_msgs.user.contains("REPAIR RULES"));
    assert!(!repair_msgs.user.contains("CRITIQUE_V1 OUTPUT TEMPLATE"));
}
