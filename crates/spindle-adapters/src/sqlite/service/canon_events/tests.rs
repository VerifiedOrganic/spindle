use super::*;
use spindle_core::canon::*;

fn resolved(id: &str) -> ClaimEntity {
    ClaimEntity::Resolved { id: id.into() }
}

fn draft_assertion(
    subject_id: &str,
    assertion: ClaimAssertion,
    pos: CanonPositionV1,
    within_scene_order: Option<u32>,
    scene_id: Option<&str>,
) -> CanonEventDraftV1 {
    CanonEventDraftV1 {
        subject: resolved(subject_id),
        payload: CanonEventPayloadV1::Assertion {
            assertion,
            modality: ClaimModality::Narration,
            temporal_scope: TemporalScope::Current,
            speaker: None,
            experiencer: None,
        },
        effective_position: Some(pos),
        within_scene_order,
        in_world_time: None,
        scene_id: scene_id.map(str::to_owned),
        source_claim_ids: vec!["claim:test".into()],
        evidence_sha256: exact_text_sha256(subject_id),
        provenance: CanonProvenanceKind::Extracted,
        supersedes_event_id: None,
        withdraws_event_id: None,
        legacy_record_scope: None,
        merge_provenance: None,
    }
}

#[tokio::test]
async fn sqlite_replay_is_deterministic_injury_persists_and_projections_rebuild_identically() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Canon Event Replay").await;
    let main_branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();

    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let p2 = CanonPositionV1::new(1, 1, 2).unwrap();
    let p3 = CanonPositionV1::new(1, 2, 1).unwrap();

    // Decision 1 at p1: Injury broken_rib + location + clock + promise plant + rule define
    let d1 = svc
        .append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:s1".into(),
            target_scene_id: Some(scene_id.clone()),
            replace_prior_scene_revisions: false,
            previous_decision_id: None,
            confidence_inputs: vec![0.99],
            reason: "initial scene 1 canon".into(),
            events: vec![
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Injury {
                        operation: InjuryOperation::Add,
                        injury_id: "broken_rib".into(),
                        body_site: "left ribs".into(),
                        severity: InjurySeverity::Moderate,
                    },
                    p1,
                    Some(1),
                    Some(&scene_id),
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::LocatedAt {
                        location: resolved("location:harbor"),
                    },
                    p1,
                    Some(2),
                    Some(&scene_id),
                ),
                draft_assertion(
                    "scene:clock1",
                    ClaimAssertion::SceneClock {
                        clock: spindle_core::models::StoryClock {
                            day_index: Some(1),
                            ..Default::default()
                        },
                    },
                    p1,
                    Some(3),
                    Some(&scene_id),
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Promise {
                        operation: PromiseOperation::Plant,
                        thread: resolved("promise:harbor_oath"),
                    },
                    p1,
                    Some(4),
                    Some(&scene_id),
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Rule {
                        operation: RuleOperation::Define,
                        rule: resolved("world_rule:iron_tithe"),
                        anchor: None,
                        preconditions: vec!["hold iron token".into()],
                        effects: vec!["cross harbor gate".into()],
                    },
                    p1,
                    Some(5),
                    Some(&scene_id),
                ),
            ],
        })
        .await
        .unwrap();
    assert!(!d1.deduplicated);
    assert_eq!(d1.events.len(), 5);

    // Decision 2 at p2: Unrelated changes (attribute, second injury added, knowledge learned & used)
    let d2 = svc
        .append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:s2".into(),
            target_scene_id: None,
            replace_prior_scene_revisions: false,
            previous_decision_id: Some(d1.decision.decision_id.clone()),
            confidence_inputs: vec![0.99],
            reason: "scene 2 canon".into(),
            events: vec![
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Attribute {
                        operation: AttributeOperation::Set,
                        key: "status".into(),
                        value: ClaimScalar::Text {
                            value: "winded".into(),
                        },
                        nature: AttributeNature::Evolving,
                    },
                    p2,
                    Some(1),
                    None,
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Injury {
                        operation: InjuryOperation::Add,
                        injury_id: "scraped_knuckle".into(),
                        body_site: "right hand".into(),
                        severity: InjurySeverity::Minor,
                    },
                    p2,
                    Some(2),
                    None,
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Learn,
                        proposition: resolved("knowledge_fact:gate_password"),
                        audience: Vec::new(),
                        source: None,
                    },
                    p2,
                    Some(3),
                    None,
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Knowledge {
                        operation: KnowledgeOperation::Use,
                        proposition: resolved("knowledge_fact:gate_password"),
                        audience: Vec::new(),
                        source: None,
                    },
                    p2,
                    Some(4),
                    None,
                ),
            ],
        })
        .await
        .unwrap();

    // Decision 3 at p3: Heal scraped_knuckle and move location; broken_rib must persist!
    let _d3 = svc
        .append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:s3".into(),
            target_scene_id: None,
            replace_prior_scene_revisions: false,
            previous_decision_id: Some(d2.decision.decision_id.clone()),
            confidence_inputs: vec![0.99],
            reason: "scene 3 canon".into(),
            events: vec![
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Injury {
                        operation: InjuryOperation::Heal,
                        injury_id: "scraped_knuckle".into(),
                        body_site: "right hand".into(),
                        severity: InjurySeverity::Minor,
                    },
                    p3,
                    Some(1),
                    None,
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Move {
                        from: resolved("location:harbor"),
                        to: resolved("location:keep"),
                    },
                    p3,
                    Some(2),
                    None,
                ),
            ],
        })
        .await
        .unwrap();

    let view_latest = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let active_injuries = view_latest.active_injuries("character:veyra");
    assert_eq!(active_injuries.len(), 1);
    assert!(active_injuries.contains_key("broken_rib"));
    assert_eq!(
        view_latest.location("character:veyra"),
        Some("location:keep")
    );

    let rows_before_rebuild = svc
        .list_canon_projection_rows(&project.project_id, &main_branch.id, CanonTier::Ratified)
        .await
        .unwrap();
    assert!(!rows_before_rebuild.is_empty());
    assert!(
        rows_before_rebuild
            .iter()
            .all(|r| r.reducer_version == CANON_REDUCER_VERSION && !r.source_event_ids.is_empty())
    );

    let rebuilt_view = svc
        .rebuild_canon_projections(&project.project_id, &main_branch.id, CanonTier::Ratified)
        .await
        .unwrap();
    assert_eq!(rebuilt_view, view_latest);

    let rows_after_rebuild = svc
        .list_canon_projection_rows(&project.project_id, &main_branch.id, CanonTier::Ratified)
        .await
        .unwrap();
    assert_eq!(rows_before_rebuild.len(), rows_after_rebuild.len());
    for (left, right) in rows_before_rebuild.iter().zip(rows_after_rebuild.iter()) {
        assert_eq!(left.projection_kind, right.projection_kind);
        assert_eq!(left.stream_key, right.stream_key);
        assert_eq!(left.source_event_ids, right.source_event_ids);
        assert_eq!(left.reducer_version, right.reducer_version);
        assert_eq!(left.data_json, right.data_json);
    }
}

#[tokio::test]
async fn sqlite_transfers_conserve_quantities_and_duplicate_retries_and_recommits_do_not_accumulate()
 {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Canon Transfers Idempotency").await;
    let main_branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();

    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let p2 = CanonPositionV1::new(1, 1, 2).unwrap();

    // 1. Opening balances at p1: Veyra = 100 gold, Bram = 40 gold; relationship trust = 50, tension = 10.
    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: main_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:open".into(),
        target_scene_id: None,
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "opening balances".into(),
        events: vec![
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Set,
                    measure: "wealth".into(),
                    amount: "100".into(),
                    unit: "gold".into(),
                    transaction_id: None,
                },
                p1,
                Some(1),
                None,
            ),
            draft_assertion(
                "character:bram",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Set,
                    measure: "wealth".into(),
                    amount: "40".into(),
                    unit: "gold".into(),
                    transaction_id: None,
                },
                p1,
                Some(2),
                None,
            ),
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Relationship {
                    operation: RelationshipOperation::Observation,
                    other: resolved("character:bram"),
                    trust: 50,
                    tension: 10,
                },
                p1,
                Some(3),
                None,
            ),
        ],
    })
    .await
    .unwrap();

    // 2. Unbalanced transfer must be rejected before append.
    let bad_err = svc
        .append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:bad_tx".into(),
            target_scene_id: Some(scene_id.clone()),
            replace_prior_scene_revisions: false,
            previous_decision_id: None,
            confidence_inputs: Vec::new(),
            reason: "unbalanced transfer".into(),
            events: vec![
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Quantity {
                        operation: QuantityOperation::Delta,
                        measure: "wealth".into(),
                        amount: "-30".into(),
                        unit: "gold".into(),
                        transaction_id: Some("tx:bad".into()),
                    },
                    p2,
                    Some(1),
                    Some(&scene_id),
                ),
                draft_assertion(
                    "character:bram",
                    ClaimAssertion::Quantity {
                        operation: QuantityOperation::Delta,
                        measure: "wealth".into(),
                        amount: "20".into(),
                        unit: "gold".into(),
                        transaction_id: Some("tx:bad".into()),
                    },
                    p2,
                    Some(1),
                    Some(&scene_id),
                ),
            ],
        })
        .await;
    assert!(bad_err.is_err());

    // 3. Valid transfer (-30 / +30) + relationship delta (+5 trust, -2 tension) in rev:1 of scene_id.
    let rev1_input = AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: main_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:scene2_v1".into(),
        target_scene_id: Some(scene_id.clone()),
        replace_prior_scene_revisions: true,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "commit scene 2 v1".into(),
        events: vec![
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Delta,
                    measure: "wealth".into(),
                    amount: "-30".into(),
                    unit: "gold".into(),
                    transaction_id: Some("tx:v1".into()),
                },
                p2,
                Some(1),
                Some(&scene_id),
            ),
            draft_assertion(
                "character:bram",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Delta,
                    measure: "wealth".into(),
                    amount: "30".into(),
                    unit: "gold".into(),
                    transaction_id: Some("tx:v1".into()),
                },
                p2,
                Some(1),
                Some(&scene_id),
            ),
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Relationship {
                    operation: RelationshipOperation::Delta,
                    other: resolved("character:bram"),
                    trust: 5,
                    tension: -2,
                },
                p2,
                Some(2),
                Some(&scene_id),
            ),
        ],
    };

    let first_commit = svc.append_canon_decision(rev1_input.clone()).await.unwrap();
    assert!(!first_commit.deduplicated);
    let wm_v1 = first_commit.decision.decision_seq;

    // 4. Duplicate retry of rev:scene2_v1 must deduplicate and not accumulate deltas!
    let retry_commit = svc.append_canon_decision(rev1_input).await.unwrap();
    assert!(retry_commit.deduplicated);
    assert_eq!(
        retry_commit.decision.decision_id,
        first_commit.decision.decision_id
    );

    let state_after_retry = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        state_after_retry.quantity_balance("character:veyra", "wealth"),
        Some("70")
    );
    assert_eq!(
        state_after_retry.quantity_balance("character:bram", "wealth"),
        Some("70")
    );
    assert_eq!(
        state_after_retry
            .relationship("character:veyra", "character:bram")
            .unwrap()
            .resolved_trust,
        Some(55)
    );

    // 5. Recommit scene_id with revised rev:scene2_v2 (-15 / +15 gold, +8 trust / -1 tension).
    //    Prior rev:scene2_v1 contributions must be invalidated rather than accumulated!
    let rev2_commit = svc
        .append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:scene2_v2".into(),
            target_scene_id: Some(scene_id.clone()),
            replace_prior_scene_revisions: true,
            previous_decision_id: Some(first_commit.decision.decision_id.clone()),
            confidence_inputs: Vec::new(),
            reason: "recommit revised scene 2 v2".into(),
            events: vec![
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Quantity {
                        operation: QuantityOperation::Delta,
                        measure: "wealth".into(),
                        amount: "-15".into(),
                        unit: "gold".into(),
                        transaction_id: Some("tx:v2".into()),
                    },
                    p2,
                    Some(1),
                    Some(&scene_id),
                ),
                draft_assertion(
                    "character:bram",
                    ClaimAssertion::Quantity {
                        operation: QuantityOperation::Delta,
                        measure: "wealth".into(),
                        amount: "15".into(),
                        unit: "gold".into(),
                        transaction_id: Some("tx:v2".into()),
                    },
                    p2,
                    Some(1),
                    Some(&scene_id),
                ),
                draft_assertion(
                    "character:veyra",
                    ClaimAssertion::Relationship {
                        operation: RelationshipOperation::Delta,
                        other: resolved("character:bram"),
                        trust: 8,
                        tension: -1,
                    },
                    p2,
                    Some(2),
                    Some(&scene_id),
                ),
            ],
        })
        .await
        .unwrap();
    assert!(!rev2_commit.deduplicated);

    let state_after_recommit = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        state_after_recommit.quantity_balance("character:veyra", "wealth"),
        Some("85")
    );
    assert_eq!(
        state_after_recommit.quantity_balance("character:bram", "wealth"),
        Some("55")
    );
    assert_eq!(
        state_after_recommit
            .relationship("character:veyra", "character:bram")
            .unwrap()
            .resolved_trust,
        Some(58)
    );

    // And historical watermark `wm_v1` still reproduces the v1 view!
    let historical_v1 = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            Some(wm_v1),
        )
        .await
        .unwrap();
    assert_eq!(
        historical_v1.quantity_balance("character:veyra", "wealth"),
        Some("70")
    );
    assert_eq!(
        historical_v1.quantity_balance("character:bram", "wealth"),
        Some("70")
    );
}

#[tokio::test]
async fn sqlite_checkpoint_rollback_nested_inheritance_and_branch_merge() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Canon Rollback And Merge").await;
    let main_branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();

    let p1 = CanonPositionV1::new(1, 1, 1).unwrap();
    let p2 = CanonPositionV1::new(1, 2, 1).unwrap();

    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: main_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:base".into(),
        target_scene_id: Some(scene_id.clone()),
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "base canon".into(),
        events: vec![draft_assertion(
            "character:veyra",
            ClaimAssertion::Injury {
                operation: InjuryOperation::Add,
                injury_id: "shoulder_wound".into(),
                body_site: "left shoulder".into(),
                severity: InjurySeverity::Moderate,
            },
            p1,
            Some(1),
            Some(&scene_id),
        )],
    })
    .await
    .unwrap();

    // 1. Capture savepoint checkpoint on main_branch.
    let cp = svc
        .create_canon_checkpoint(
            &project.project_id,
            &main_branch.id,
            None,
            CanonTier::Ratified,
        )
        .await
        .unwrap();
    let view_at_cp = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();

    // 2. Append a post-checkpoint change on main_branch (healing shoulder_wound).
    let post_cp = svc
        .append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:post_cp".into(),
            target_scene_id: None,
            replace_prior_scene_revisions: false,
            previous_decision_id: None,
            confidence_inputs: Vec::new(),
            reason: "heal wound".into(),
            events: vec![draft_assertion(
                "character:veyra",
                ClaimAssertion::Injury {
                    operation: InjuryOperation::Heal,
                    injury_id: "shoulder_wound".into(),
                    body_site: "left shoulder".into(),
                    severity: InjurySeverity::Moderate,
                },
                p2,
                Some(1),
                None,
            )],
        })
        .await
        .unwrap();
    let wm_before_restore = post_cp.decision.decision_seq;

    let view_healed = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(view_healed.active_injuries("character:veyra").is_empty());

    // 3. Roll back to checkpoint `cp`: restores the prior view while keeping history at `wm_before_restore`.
    let restored_view = svc
        .restore_canon_checkpoint(
            &project.project_id,
            &main_branch.id,
            &cp.checkpoint_id,
            "author:human",
            "rollback to cp",
        )
        .await
        .unwrap();
    assert_eq!(restored_view.entities, view_at_cp.entities);
    assert_eq!(
        restored_view.visible_event_ids,
        view_at_cp.visible_event_ids
    );

    let historical_healed = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            Some(wm_before_restore),
        )
        .await
        .unwrap();
    assert!(
        historical_healed
            .active_injuries("character:veyra")
            .is_empty()
    );

    // 4. Create child branch, switch active branch to child, and append an event targeting
    //    inherited `main_branch.id` with `resolve_active_descendant_branch: true`.
    let child_branch = svc
        .repository()
        .create_branch(
            &project.project_id,
            &main_branch.id,
            "child-arc",
            "experiment",
            None,
        )
        .await
        .unwrap();
    svc.repository()
        .switch_active_branch(&project.project_id, &child_branch.id)
        .await
        .unwrap();

    let child_commit = svc
        .append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: true,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:child_scene".into(),
            target_scene_id: Some(scene_id.clone()),
            replace_prior_scene_revisions: false,
            previous_decision_id: None,
            confidence_inputs: Vec::new(),
            reason: "inherited scene committed on active child branch".into(),
            events: vec![draft_assertion(
                "character:veyra",
                ClaimAssertion::Attribute {
                    operation: AttributeOperation::Set,
                    key: "rank".into(),
                    value: ClaimScalar::Text {
                        value: "Marshal".into(),
                    },
                    nature: AttributeNature::Evolving,
                },
                p2,
                Some(1),
                Some(&scene_id),
            )],
        })
        .await
        .unwrap();
    assert_eq!(child_commit.decision.branch_id, child_branch.id);

    // Main does not yet have `rank = Marshal`; child has both inherited `shoulder_wound` and `rank = Marshal`.
    let main_before_merge = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(
        !main_before_merge.entities["character:veyra"]
            .attributes
            .contains_key("rank")
    );

    let child_view = svc
        .canon_state_at(
            &project.project_id,
            &child_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(
        child_view
            .active_injuries("character:veyra")
            .contains_key("shoulder_wound")
    );
    assert!(
        child_view.entities["character:veyra"]
            .attributes
            .contains_key("rank")
    );

    // 5. Merge `child_branch` into `main_branch`: preserves source provenance and mints target event IDs.
    let (merge_plan, merge_out) = svc
        .merge_canon_branch_events(
            &project.project_id,
            &main_branch.id,
            &main_branch.id,
            &child_branch.id,
            "author:human",
            "merge child-arc into main",
        )
        .await
        .unwrap();
    assert!(merge_plan.conflicts.is_empty());
    let merge_out = merge_out.unwrap();
    assert_eq!(merge_out.events.len(), 1);
    assert_eq!(merge_out.events[0].provenance, CanonProvenanceKind::Merged);
    assert_eq!(
        merge_out.events[0]
            .merge_provenance
            .as_ref()
            .unwrap()
            .source_branch_id,
        child_branch.id
    );
    assert_ne!(
        merge_out.events[0].event_id,
        child_commit.events[0].event_id
    );

    let main_after_merge = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        main_after_merge.entities["character:veyra"].attributes["rank"].scalar_value,
        Some(ClaimScalar::Text {
            value: "Marshal".into()
        })
    );

    // 6. Idempotent `canon_source_map` round-trip.
    let sm = svc
        .upsert_canon_source_map(CanonSourceMapEntryV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            source_table: "character_state".into(),
            legacy_primary_key: "character_state:legacy1".into(),
            migration_version: 1,
            status: CanonSourceMapStatus::Mapped,
            provenance: CanonProvenanceKind::LegacyUnverified,
            claim_ids: Vec::new(),
            event_ids: vec![merge_out.events[0].event_id.clone()],
            decision_id: Some(merge_out.decision.decision_id.clone()),
            quarantine_reason: None,
            raw_snapshot_json: r#"{"status":["injured"]}"#.into(),
            created_at_micros: 0,
            updated_at_micros: 0,
        })
        .await
        .unwrap();
    let maps = svc
        .list_canon_source_maps(
            &project.project_id,
            Some(&main_branch.id),
            Some("character_state"),
        )
        .await
        .unwrap();
    assert_eq!(maps.len(), 1);
    assert_eq!(maps[0].legacy_primary_key, sm.legacy_primary_key);

    // 7. Incremental second merge recognizes previously merged source events via
    //    `merge_provenance`: relationship and untransacted quantity deltas are
    //    applied exactly once, and source-only removals (hiding an inherited scene)
    //    propagate to target.
    let p3 = CanonPositionV1::new(1, 3, 1).unwrap();
    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: main_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:main_rel_q_open".into(),
        target_scene_id: Some(scene_id.clone()),
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "establish relationship and wealth opening on main".into(),
        events: vec![
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Relationship {
                    operation: RelationshipOperation::Observation,
                    other: resolved("character:bram"),
                    trust: 10,
                    tension: 5,
                },
                p1,
                Some(20),
                None,
            ),
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Set,
                    measure: "coins".into(),
                    amount: "100".into(),
                    unit: "silver".into(),
                    transaction_id: None,
                },
                p1,
                Some(21),
                None,
            ),
            draft_assertion(
                "character:veyra",
                ClaimAssertion::LocatedAt {
                    location: resolved("location:watchtower"),
                },
                p1,
                Some(22),
                Some(&scene_id),
            ),
        ],
    })
    .await
    .unwrap();

    // First child delta batch: relationship +5/-2, coins +15.
    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: child_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:child_delta_1".into(),
        target_scene_id: None,
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "child delta 1".into(),
        events: vec![
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Relationship {
                    operation: RelationshipOperation::Delta,
                    other: resolved("character:bram"),
                    trust: 5,
                    tension: -2,
                },
                p2,
                Some(10),
                None,
            ),
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Delta,
                    measure: "coins".into(),
                    amount: "15".into(),
                    unit: "silver".into(),
                    transaction_id: None,
                },
                p2,
                Some(11),
                None,
            ),
        ],
    })
    .await
    .unwrap();

    let (plan_m1, out_m1) = svc
        .merge_canon_branch_events(
            &project.project_id,
            &main_branch.id,
            &main_branch.id,
            &child_branch.id,
            "author:human",
            "incremental merge 1",
        )
        .await
        .unwrap();
    assert!(plan_m1.conflicts.is_empty());
    assert_eq!(out_m1.unwrap().events.len(), 2);

    // Second child batch: relationship +3/+1, coins -5, plus hiding inherited `scene_id`.
    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: child_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:child_delta_2_and_hide".into(),
        target_scene_id: None,
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "child delta 2 and hide scene 1".into(),
        events: vec![
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Relationship {
                    operation: RelationshipOperation::Delta,
                    other: resolved("character:bram"),
                    trust: 3,
                    tension: 1,
                },
                p3,
                Some(1),
                None,
            ),
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Delta,
                    measure: "coins".into(),
                    amount: "-5".into(),
                    unit: "silver".into(),
                    transaction_id: None,
                },
                p3,
                Some(2),
                None,
            ),
            CanonEventDraftV1 {
                subject: resolved(&format!("scene:{scene_id}")),
                payload: CanonEventPayloadV1::SceneVisibility {
                    visibility: ScenePlacementVisibilityV1 {
                        scene_id: scene_id.clone(),
                        visible: false,
                        position_override: None,
                        invalidated_from: p1,
                    },
                },
                effective_position: Some(p1),
                within_scene_order: None,
                in_world_time: None,
                scene_id: Some(scene_id.clone()),
                source_claim_ids: Vec::new(),
                evidence_sha256: exact_text_sha256("hide_scene_1"),
                provenance: CanonProvenanceKind::ManualAuthor,
                supersedes_event_id: None,
                withdraws_event_id: None,
                legacy_record_scope: None,
                merge_provenance: None,
            },
        ],
    })
    .await
    .unwrap();

    let (plan_m2, out_m2) = svc
        .merge_canon_branch_events(
            &project.project_id,
            &main_branch.id,
            &main_branch.id,
            &child_branch.id,
            "author:human",
            "incremental merge 2",
        )
        .await
        .unwrap();
    assert!(plan_m2.conflicts.is_empty());
    // Only the 3 new events from batch 2 are accepted; batch 1 events are NOT re-applied.
    assert_eq!(out_m2.unwrap().events.len(), 3);

    let main_after_m2 = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    // Hiding `scene_id` removed the scene 1 location on main (`location:watchtower`),
    // and both incremental merges applied batch 1 (+5/-2 trust/tension, +15 coins)
    // and batch 2 (+3/+1 trust/tension, -5 coins) exactly once.
    assert_eq!(main_after_m2.location("character:veyra"), None);
    assert_eq!(
        main_after_m2.quantity_balance("character:veyra", "coins"),
        Some("110")
    );
    let rel_after_m2 = main_after_m2
        .relationship("character:veyra", "character:bram")
        .unwrap();
    assert_eq!(rel_after_m2.resolved_trust, Some(18));
    assert_eq!(rel_after_m2.resolved_tension, Some(4));

    // 8. Precondition validation in SQLite rejects a quantity delta or item drop
    //    ordered at an earlier `within_scene_order` step than its opening/acquisition.
    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: main_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:step10_open_and_acq".into(),
        target_scene_id: None,
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "opening balance and item acquisition at step 10".into(),
        events: vec![
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Set,
                    measure: "mana".into(),
                    amount: "50".into(),
                    unit: "mp".into(),
                    transaction_id: None,
                },
                p2,
                Some(10),
                None,
            ),
            draft_assertion(
                "character:veyra",
                ClaimAssertion::Possession {
                    operation: PossessionOperation::Acquire,
                    item: resolved("item:sunstone"),
                    counterparty: None,
                    custody: CustodyKind::Custodian,
                },
                p2,
                Some(10),
                None,
            ),
        ],
    })
    .await
    .unwrap();

    // Delta at step 5 in `p2` must fail because opening balance is at step 10.
    assert!(
        svc.append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:step5_early_delta".into(),
            target_scene_id: None,
            replace_prior_scene_revisions: false,
            previous_decision_id: None,
            confidence_inputs: Vec::new(),
            reason: "early delta before step 10 opening".into(),
            events: vec![draft_assertion(
                "character:veyra",
                ClaimAssertion::Quantity {
                    operation: QuantityOperation::Delta,
                    measure: "mana".into(),
                    amount: "-10".into(),
                    unit: "mp".into(),
                    transaction_id: None,
                },
                p2,
                Some(5),
                None,
            )],
        })
        .await
        .is_err()
    );

    // Drop at step 5 in `p2` must fail because acquisition is at step 10.
    assert!(
        svc.append_canon_decision(AppendCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            resolve_active_descendant_branch: false,
            action: CanonPolicyAction::Ratify,
            tier: CanonTier::Ratified,
            actor: "author:human".into(),
            policy_version: CANON_POLICY_VERSION_V1.into(),
            source_revision_id: "rev:step5_early_drop".into(),
            target_scene_id: None,
            replace_prior_scene_revisions: false,
            previous_decision_id: None,
            confidence_inputs: Vec::new(),
            reason: "early drop before step 10 acquisition".into(),
            events: vec![draft_assertion(
                "character:veyra",
                ClaimAssertion::Possession {
                    operation: PossessionOperation::Lose,
                    item: resolved("item:sunstone"),
                    counterparty: None,
                    custody: CustodyKind::Custodian,
                },
                p2,
                Some(5),
                None,
            )],
        })
        .await
        .is_err()
    );

    // 9. Incremental merge of a revision-only `SourceInvalidation` (empty `invalidated_event_ids`)
    //    removes the previously merged target copy of the invalidated source revision.
    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: child_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::Ratify,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:child_alias_shadow".into(),
        target_scene_id: None,
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "child sets temporary alias".into(),
        events: vec![draft_assertion(
            "character:veyra",
            ClaimAssertion::Attribute {
                operation: AttributeOperation::Set,
                key: "alias".into(),
                value: ClaimScalar::Text {
                    value: "Shadowblade".into(),
                },
                nature: AttributeNature::Evolving,
            },
            p3,
            Some(10),
            None,
        )],
    })
    .await
    .unwrap();

    let (plan_m3, out_m3) = svc
        .merge_canon_branch_events(
            &project.project_id,
            &main_branch.id,
            &main_branch.id,
            &child_branch.id,
            "author:human",
            "incremental merge 3 (merge alias)",
        )
        .await
        .unwrap();
    assert!(plan_m3.conflicts.is_empty());
    assert_eq!(out_m3.unwrap().events.len(), 1);

    let main_after_m3 = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        main_after_m3.entities["character:veyra"].attributes["alias"].scalar_value,
        Some(ClaimScalar::Text {
            value: "Shadowblade".into()
        })
    );

    // Now append a revision-only invalidation on `child_branch` naming `rev:child_alias_shadow`
    // with an empty `invalidated_event_ids` list, and merge into `main_branch` again.
    svc.append_canon_decision(AppendCanonDecisionInputV1 {
        project_id: project.project_id.clone(),
        branch_id: child_branch.id.clone(),
        resolve_active_descendant_branch: false,
        action: CanonPolicyAction::InvalidateSource,
        tier: CanonTier::Ratified,
        actor: "author:human".into(),
        policy_version: CANON_POLICY_VERSION_V1.into(),
        source_revision_id: "rev:child_invalidate_alias_rev_only".into(),
        target_scene_id: None,
        replace_prior_scene_revisions: false,
        previous_decision_id: None,
        confidence_inputs: Vec::new(),
        reason: "revision-only invalidation of rev:child_alias_shadow".into(),
        events: vec![CanonEventDraftV1 {
            subject: resolved("character:veyra"),
            payload: CanonEventPayloadV1::SourceInvalidation {
                invalidation: SourceInvalidationV1 {
                    scene_id: None,
                    invalidated_revision_ids: vec!["rev:child_alias_shadow".into()],
                    invalidated_event_ids: Vec::new(),
                    replacement_revision_id: None,
                },
            },
            effective_position: Some(p3),
            within_scene_order: None,
            in_world_time: None,
            scene_id: None,
            source_claim_ids: Vec::new(),
            evidence_sha256: exact_text_sha256("invalidate_rev_child_alias_shadow"),
            provenance: CanonProvenanceKind::ManualAuthor,
            supersedes_event_id: None,
            withdraws_event_id: None,
            legacy_record_scope: None,
            merge_provenance: None,
        }],
    })
    .await
    .unwrap();

    let (plan_m4, out_m4) = svc
        .merge_canon_branch_events(
            &project.project_id,
            &main_branch.id,
            &main_branch.id,
            &child_branch.id,
            "author:human",
            "incremental merge 4 (merge revision-only invalidation)",
        )
        .await
        .unwrap();
    assert!(plan_m4.conflicts.is_empty());
    assert_eq!(out_m4.unwrap().events.len(), 1);

    let main_after_m4 = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(
        !main_after_m4.entities["character:veyra"]
            .attributes
            .contains_key("alias")
    );
}

#[tokio::test]
async fn backfill_all_18_stores_idempotency_progression_dedup_and_shadow_parity() {
    use crate::sqlite::repository::{
        AppendCharacterStateParams, AppendProgressionEventParams, AppendQuantityStateParams,
        CanonDeltaDecision, StageCanonDeltaParams, UpsertKnowsParams,
    };
    use spindle_core::canon::ALL_BACKFILL_STORES;
    use spindle_core::models::{
        ArchiveEntityInput, CharacterStatePatch, CreateFutureKnowledgeInput, CreateLocationInput,
        CreateNarrativePromiseInput, CreateRelationshipInput, CreateSystemOverlayInput,
        CreateTemporalInterventionInput, CreateTimelineEventInput, CreateWorldRuleInput,
        QuantityState, RecordKnowledgeInput, RegisterCanonicalFactInput, SecrecyScope, StoryClock,
        StoryPlacement, UpdatePromiseStatusInput, UpdateRelationshipInput, WorldStateInput,
    };

    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene1_id, _) = project_with_scene(&svc, "Backfill All 18").await;
    let main_branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let scene2_id = recap_scene(
        &svc,
        &project.project_id,
        1,
        1,
        2,
        "The second scene in the chapter.",
    )
    .await;

    let veyra_id = create_character_named(&svc, &project.project_id, "Veyra").await;
    let bram_id = create_character_named(&svc, &project.project_id, "Bram").await;
    let loc_out = svc
        .create_location(CreateLocationInput {
            project_id: project.project_id.clone(),
            name: "Obsidian Spire".into(),
            kind: "fortress".into(),
            realm: Some("North".into()),
            summary: "A tall spire of dark glass.".into(),
            initial_state: WorldStateInput {
                controlling_faction: Some("Wardens".into()),
                status: Some("fortified".into()),
                prosperity: Some("high".into()),
                stability: Some("stable".into()),
                threat_level: Some("elevated".into()),
                sensory_details: vec!["cold wind".into(), "ozone".into()],
            },
        })
        .await
        .unwrap();

    // 1. canonical_fact (active + superseded + secret)
    let old_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(scene1_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(veyra_id.clone()),
            predicate: Some("banner_color".into()),
            value_kind: Some("text".into()),
            value_text: Some("crimson".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: None,
        })
        .await
        .unwrap();
    let _new_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(scene2_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(veyra_id.clone()),
            predicate: Some("banner_color".into()),
            value_kind: Some("text".into()),
            value_text: Some("obsidian".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: Some(old_fact.canonical_fact_id.clone()),
            secrecy: None,
        })
        .await
        .unwrap();
    let secret_fact = svc
        .register_canonical_fact(RegisterCanonicalFactInput {
            project_id: project.project_id.clone(),
            scene_id: Some(scene1_id.clone()),
            book_number: 1,
            chapter_number: 1,
            fact_type: None,
            key: None,
            value: None,
            context: None,
            subject_table: Some("character".into()),
            subject_id: Some(veyra_id.clone()),
            predicate: Some("true_lineage".into()),
            value_kind: Some("text".into()),
            value_text: Some("House Solaris".into()),
            value_number: None,
            value_unit: None,
            value_json: None,
            aliases: Vec::new(),
            scope: None,
            valid_from: None,
            valid_until: None,
            legacy_untyped: None,
            supersedes_fact_id: None,
            secrecy: Some(SecrecyScope {
                holder_ids: vec![veyra_id.clone()],
                concealment_note: Some("hidden from the council".into()),
            }),
        })
        .await
        .unwrap();

    // 2. character_state (2 snapshots with status/injury transitions)
    svc.repository()
        .append_character_state(AppendCharacterStateParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            character_id: veyra_id.clone(),
            scene_id: Some(scene1_id.clone()),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
            patch: CharacterStatePatch {
                emotional_state: [("resolve".to_string(), serde_json::json!(0.8))]
                    .into_iter()
                    .collect(),
                goals: Some(vec!["hold the gate".into()]),
                status: Some(vec!["alive".into(), "injury:left_arm:minor".into()]),
                notes: Some(vec!["first watch".into()]),
                source_summary: Some("scene 1".into()),
            },
        })
        .await
        .unwrap();
    svc.repository()
        .append_character_state(AppendCharacterStateParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            character_id: veyra_id.clone(),
            scene_id: Some(scene2_id.clone()),
            book_number: 1,
            chapter_number: 1,
            scene_order: 2,
            patch: CharacterStatePatch {
                emotional_state: [("resolve".to_string(), serde_json::json!(0.95))]
                    .into_iter()
                    .collect(),
                goals: Some(vec!["hold the gate".into(), "seal the breach".into()]),
                status: Some(vec!["alive".into(), "focused".into()]),
                notes: Some(vec!["arm bandaged".into()]),
                source_summary: Some("scene 2".into()),
            },
        })
        .await
        .unwrap();

    // 3. world_state was created via `create_location` initial_state above (`loc_out.location_id`).

    // 4. world_rule
    let wr_out = svc
        .create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "Iron Ward".into(),
            rule_type: "magic".into(),
            description: "Cold iron nullifies spoken sigils within three paces.".into(),
            established_in: None,
            relevance_tags: vec!["warding".into(), "iron".into()],
            scan_pattern: None,
        })
        .await
        .unwrap();

    // 5. knowledge_fact + 6. knows
    let kn_out = svc
        .record_knowledge(RecordKnowledgeInput {
            project_id: project.project_id.clone(),
            branch_id: None,
            character_id: bram_id.clone(),
            fact: "The northern gate has a hidden postern.".into(),
            source_summary: "scout report".into(),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            }),
            confidence: Some(0.9),
            tags: vec!["gate".into()],
            reader_visible: true,
            secret_of_fact_id: None,
        })
        .await
        .unwrap();
    svc.repository()
        .upsert_knows(UpsertKnowsParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            character_id: bram_id.clone(),
            knowledge_fact_id: kn_out.fact.knowledge_fact_id.clone(),
            source_summary: Some("scout report".into()),
            learned_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            }),
            confidence: Some(0.9),
            reader_visible: true,
            source_import_session_id: None,
        })
        .await
        .unwrap();

    // 7. future_knowledge
    let fk_out = svc
        .create_future_knowledge(CreateFutureKnowledgeInput {
            project_id: project.project_id.clone(),
            character_id: veyra_id.clone(),
            knowledge_summary: "The eclipse occurs on the third night of the siege.".into(),
            source: "prophetic vision".into(),
            learned_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            expires_at: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 3,
                scene_order: Some(1),
                note: None,
            }),
            notes: vec!["time-displaced".into()],
        })
        .await
        .unwrap();

    // 8. system_overlay + 9. quantity_state + 10. progression_event
    let overlay_out = svc
        .create_system_overlay(CreateSystemOverlayInput {
            project_id: project.project_id.clone(),
            system_name: "Aether Conduit".into(),
            system_type: "cultivation".into(),
            rules: "Channels ambient aether through sigil rings.".into(),
            visibility: "explicit".into(),
            progression_currency: Some("aether".into()),
            stats: vec!["capacity".into()],
            advancement_tiers: vec!["Initiate".into(), "Adept".into()],
        })
        .await
        .unwrap();

    svc.repository()
        .append_quantity_state(AppendQuantityStateParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            subject_table: "character".into(),
            subject_id: veyra_id.clone(),
            measure: "gold".into(),
            state: QuantityState {
                amount: Some(100.0),
                unit: Some("crowns".into()),
                band: Some("comfortable".into()),
                change_reason: Some("opening".into()),
            },
            scene_id: Some(scene1_id.clone()),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
        })
        .await
        .unwrap();
    svc.repository()
        .append_quantity_state(AppendQuantityStateParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            subject_table: "character".into(),
            subject_id: veyra_id.clone(),
            measure: "gold".into(),
            state: QuantityState {
                amount: Some(125.0),
                unit: Some("crowns".into()),
                band: Some("comfortable".into()),
                change_reason: Some("bounty reward".into()),
            },
            scene_id: Some(scene2_id.clone()),
            book_number: 1,
            chapter_number: 1,
            scene_order: 2,
        })
        .await
        .unwrap();

    // progression_event #1: duplicates the quantity_state at scene 2 -> DuplicateSkipped!
    // progression_event #2: unique overlay tier_up transition -> Mapped!
    svc.repository()
        .append_progression_event(AppendProgressionEventParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            subject_table: "character".into(),
            subject_id: veyra_id.clone(),
            overlay_id: None,
            kind: "quantity_change".into(),
            delta_json: serde_json::json!({
                "measure": "gold",
                "delta": "25",
                "unit": "crowns"
            }),
            source_scene_id: Some(scene2_id.clone()),
            placement: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(2),
                note: None,
            }),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();
    svc.repository()
        .append_progression_event(AppendProgressionEventParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            subject_table: "character".into(),
            subject_id: veyra_id.clone(),
            overlay_id: Some(overlay_out.system_overlay_id.clone()),
            kind: "tier_up".into(),
            delta_json: serde_json::json!({
                "from_tier": "Initiate",
                "to_tier": "Adept"
            }),
            source_scene_id: Some(scene2_id.clone()),
            placement: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(2),
                note: None,
            }),
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    // 11. timeline_event + 12. temporal_intervention + 13. scene_clock + 14. timeline_event_clock
    let te_out = svc
        .create_timeline_event(CreateTimelineEventInput {
            project_id: project.project_id.clone(),
            title: "The Spire Bell Tolls".into(),
            event_type: "battle".into(),
            placement: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            summary: "The warning bell rings across the valley.".into(),
            related_entity_ids: vec![veyra_id.clone(), loc_out.location_id.clone()],
        })
        .await
        .unwrap();
    svc.create_temporal_intervention(CreateTemporalInterventionInput {
        project_id: project.project_id.clone(),
        title: "Echo Reversal".into(),
        intervention_type: "time_loop".into(),
        source_event_id: Some(te_out.timeline_event_id.clone()),
        target_event_id: Some(te_out.timeline_event_id.clone()),
        summary: "Rewinds the bell toll by ten minutes.".into(),
        consequences: vec!["bell rope frays".into()],
        status: Some("active".into()),
    })
    .await
    .unwrap();
    svc.repository()
        .upsert_scene_clock(
            &scene1_id,
            &project.project_id,
            &main_branch.id,
            &StoryClock {
                day_index: Some(1),
                time_of_day: Some(6),
                duration_days: Some(0.1),
                precision: Some("exact".into()),
            },
            Some("linear"),
            Some("main_timeline"),
        )
        .await
        .unwrap();
    svc.repository()
        .upsert_timeline_event_clock(
            &te_out.timeline_event_id,
            &project.project_id,
            &main_branch.id,
            &StoryClock {
                day_index: Some(1),
                time_of_day: Some(6),
                duration_days: Some(0.05),
                precision: Some("exact".into()),
            },
        )
        .await
        .unwrap();

    // 15. narrative_promise (one open + reinforced, one archived)
    let np_open = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project.project_id.clone(),
            promise_type: "chekhovs_gun".into(),
            description: "Veyra carries the glass key to the Spire vault.".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 4,
                scene_order: Some(1),
                note: None,
            }),
            notes: Vec::new(),
        })
        .await
        .unwrap();
    svc.update_promise_status(UpdatePromiseStatusInput {
        narrative_promise_id: np_open.narrative_promise_id.clone(),
        status: "in_progress".into(),
        at: Some(StoryPlacement {
            book_number: 1,
            chapter_number: 1,
            scene_order: Some(2),
            note: None,
        }),
        source_scene_id: Some(scene2_id.clone()),
        replaces_event_id: None,
        note: Some("Veyra inspects the vault lock".into()),
    })
    .await
    .unwrap();
    let np_archived = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project.project_id.clone(),
            promise_type: "red_herring".into(),
            description: "A dropped subplot about the cellar tunnel.".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: None,
            notes: Vec::new(),
        })
        .await
        .unwrap();
    svc.archive_entity(ArchiveEntityInput {
        entity_type: "narrative_promise".into(),
        entity_id: np_archived.narrative_promise_id.clone(),
    })
    .await
    .unwrap();

    // 16. relates_to + 17. relationship_scene_delta
    svc.create_relationship(CreateRelationshipInput {
        character_a_id: veyra_id.clone(),
        character_b_id: bram_id.clone(),
        relationship_type: "comrades".into(),
        initial_trust: 40,
        initial_tension: 15,
        dynamics: vec!["sworn shield".into()],
    })
    .await
    .unwrap();
    svc.update_relationship(UpdateRelationshipInput {
        character_a_id: veyra_id.clone(),
        character_b_id: bram_id.clone(),
        trust_delta: 10,
        tension_delta: -5,
        reason: "held the eastern parapet together".into(),
        scene_id: scene2_id.clone(),
    })
    .await
    .unwrap();

    // 18. canon_delta (applied, staged/pending, rejected)
    let cd_applied = svc
        .repository()
        .stage_canon_delta(StageCanonDeltaParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            scene_id: scene2_id.clone(),
            authoring_run_id: None,
            delta_class: "relationship_shift".into(),
            target_id: Some(veyra_id.clone()),
            payload: serde_json::json!({
                "from_character_id": veyra_id,
                "to_character_id": bram_id,
                "trust_delta": 10,
                "tension_delta": -5
            }),
            evidence: "They held the eastern parapet together.".into(),
            confidence: "high".into(),
        })
        .await
        .unwrap();
    svc.repository()
        .decide_canon_delta(
            &cd_applied.id,
            CanonDeltaDecision::Applied,
            "operator",
            None,
        )
        .await
        .unwrap();

    let _cd_staged = svc
        .repository()
        .stage_canon_delta(StageCanonDeltaParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            scene_id: scene2_id.clone(),
            authoring_run_id: None,
            delta_class: "character_state".into(),
            target_id: Some(veyra_id.clone()),
            payload: serde_json::json!({
                "status": ["vigilant"]
            }),
            evidence: "Veyra kept watch through the night.".into(),
            confidence: "medium".into(),
        })
        .await
        .unwrap();

    let cd_rejected = svc
        .repository()
        .stage_canon_delta(StageCanonDeltaParams {
            project_id: project.project_id.clone(),
            branch_id: main_branch.id.clone(),
            scene_id: scene2_id.clone(),
            authoring_run_id: None,
            delta_class: "character_state".into(),
            target_id: Some(bram_id.clone()),
            payload: serde_json::json!({
                "status": ["exhausted"]
            }),
            evidence: "A stray stone chipped the outer stair.".into(),
            confidence: "low".into(),
        })
        .await
        .unwrap();
    svc.repository()
        .decide_canon_delta(
            &cd_rejected.id,
            CanonDeltaDecision::Rejected,
            "operator",
            None,
        )
        .await
        .unwrap();

    // Run backfill!
    let report1 = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    assert_eq!(report1.total_quarantined_count, 0);
    assert_eq!(report1.total_reused_idempotent_count, 0);
    let total_scanned1 = report1.total_mapped_count
        + report1.total_duplicate_skipped_count
        + report1.total_quarantined_count
        + report1.total_reused_idempotent_count;
    assert!(total_scanned1 >= 18);

    // Every one of the 18 stores was scanned and had >= 1 row.
    for store in ALL_BACKFILL_STORES {
        let sc = &report1.by_store[store.as_source_table()];
        let store_scanned = sc.mapped_count
            + sc.duplicate_skipped_count
            + sc.quarantined_count
            + sc.reused_idempotent_count;
        assert!(
            store_scanned >= 1,
            "expected store {} to have at least 1 row scanned, got {:?}",
            store.as_source_table(),
            sc
        );
        assert_eq!(sc.reused_idempotent_count, 0);
        assert_eq!(sc.quarantined_count, 0);
    }

    // Verify progression_event deduplication: 1 DuplicateSkipped (gold delta already in quantity_state) + 1 Mapped (tier_up).
    let prog_counts = &report1.by_store["progression_event"];
    assert_eq!(prog_counts.duplicate_skipped_count, 1);
    assert_eq!(prog_counts.mapped_count, 1);

    // Verify canon_delta handling: 1 applied -> DuplicateSkipped, 1 staged + 1 rejected -> Mapped.
    let cd_counts = &report1.by_store["canon_delta"];
    assert_eq!(cd_counts.duplicate_skipped_count, 1);
    assert_eq!(cd_counts.mapped_count, 2);

    // Verify future_knowledge did NOT fold into ordinary unconditional view.knowledge,
    // and secret_circle_members matches legacy.
    let view = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        view.knowledge_status(
            &veyra_id,
            &format!("proposition:future:{}", fk_out.future_knowledge_id)
        ),
        KnowledgeStatus::Unknown
    );
    let proj_secret_members = crate::sqlite::repository::project_secret_circle_members_from_canon(
        &view,
        &secret_fact.canonical_fact_id,
    )
    .unwrap();
    assert_eq!(proj_secret_members.len(), 1);
    assert_eq!(proj_secret_members[0].0, veyra_id);

    // Verify archived promise is tombstoned in `view.open_threads` while active promise is open.
    assert!(
        view.open_threads
            .contains_key(&np_open.narrative_promise_id)
    );
    assert!(
        !view
            .open_threads
            .contains_key(&np_archived.narrative_promise_id)
    );

    // Verify relationship math in reducer (`40 + 10 = 50` trust, `15 - 5 = 10` tension) — NOT double-counted by applied canon_delta or progression_event!
    let rel_reduced = view.relationship(&veyra_id, &bram_id).unwrap();
    assert_eq!(rel_reduced.resolved_trust, Some(50));
    assert_eq!(rel_reduced.resolved_tension, Some(10));
    assert_eq!(view.quantity_balance(&veyra_id, "gold"), Some("125"));

    // Rerun backfill: 100% idempotent, zero new events or claims!
    let events_before_rerun = svc
        .list_canon_events_for_lineage(&project.project_id, &main_branch.id)
        .await
        .unwrap();
    let report2 = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    assert_eq!(report2.total_mapped_count, 0);
    assert_eq!(report2.total_duplicate_skipped_count, 0);
    assert_eq!(report2.total_quarantined_count, 0);
    assert_eq!(report2.total_reused_idempotent_count, total_scanned1);
    let events_after_rerun = svc
        .list_canon_events_for_lineage(&project.project_id, &main_branch.id)
        .await
        .unwrap();
    assert_eq!(events_before_rerun.len(), events_after_rerun.len());

    // Compare compatibility shadow reads & SubjectSnapshots on main_branch:
    let shadow = svc
        .compare_canon_compatibility_shadow_reads(&project.project_id, &main_branch.id, None)
        .await
        .unwrap();
    assert!(
        shadow.parity_clean,
        "expected clean shadow parity, got differences: {:#?}",
        shadow.differences
    );
    assert!(shadow.checked_store_count >= 16);
    assert!(shadow.checked_subject_snapshot_count >= 6);

    let readiness = svc
        .evaluate_project_canon_cutover_readiness(&project.project_id)
        .await
        .unwrap();
    assert!(
        readiness.cutover_allowed,
        "expected cutover_allowed, blocking_reasons: {:?}",
        readiness.blocking_reasons
    );

    // Metadata-only edit on an existing `world_rule` and `canonical_fact` where the
    // typed assertion (`ClaimAssertion::Rule` and `banner_color = "obsidian"`) is unchanged:
    // Reconciliation invalidates the old incarnation events, and the new incarnation
    // must mint fresh events so the typed fact and rule remain active in `CanonStateViewV1`!
    svc.update_world_rule(spindle_core::models::UpdateWorldRuleInput {
        world_rule_id: wr_out.world_rule_id.clone(),
        changes: serde_json::json!({
            "relevance_tags": ["warding", "iron", "UpdatedTag"],
            "scan_pattern": "cold iron"
        }),
    })
    .await
    .unwrap();
    svc.repository()
        .pool()
        .write({
            let fid = _new_fact.canonical_fact_id.clone();
            move |conn| {
                conn.execute(
                    "UPDATE canonical_fact SET aliases = '[\"dark-banner\"]' WHERE id = ?1",
                    [&fid],
                )?;
                Ok(())
            }
        })
        .await
        .unwrap();

    let report_after_meta_edit = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    assert!(report_after_meta_edit.total_mapped_count >= 2);
    assert_eq!(report_after_meta_edit.total_quarantined_count, 0);

    let view_after_meta_edit = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        view_after_meta_edit.entities[&veyra_id]
            .attributes
            .get("banner_color")
            .and_then(|a| a.scalar_value.as_ref()),
        Some(&ClaimScalar::Text {
            value: "obsidian".to_string()
        }),
        "typed attribute assertion must survive metadata-only edit reconciliation"
    );
    assert!(
        view_after_meta_edit
            .rules
            .get(&wr_out.world_rule_id)
            .is_some_and(|r| r.defined),
        "typed world_rule assertion must survive metadata-only edit reconciliation"
    );

    let shadow_after_meta_edit = svc
        .compare_canon_compatibility_shadow_reads(&project.project_id, &main_branch.id, None)
        .await
        .unwrap();
    assert!(
        shadow_after_meta_edit.parity_clean,
        "expected clean shadow parity after metadata-only edit, got: {:#?}",
        shadow_after_meta_edit.differences
    );

    // Unchanged rerun after metadata-only edit is 100% idempotent again:
    let report_idempotent_after_edit = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    assert_eq!(report_idempotent_after_edit.total_mapped_count, 0);
    assert_eq!(
        report_idempotent_after_edit.total_reused_idempotent_count,
        total_scanned1
    );

    // Removing a `DuplicateSkipped` `progression_event` row (and an `applied` `canon_delta` row)
    // while their authoritative rows (`quantity_state`, `relates_to`) remain unchanged must NOT
    // invalidate the authoritative rows' events!
    svc.repository()
        .pool()
        .write({
            let pid = project.project_id.clone();
            let cd_app_id = cd_applied.id.clone();
            move |conn| {
                conn.execute(
                    "DELETE FROM progression_event WHERE project_id = ?1 AND kind = 'quantity_change'",
                    [&pid],
                )?;
                conn.execute("DELETE FROM canon_delta WHERE id = ?1", [&cd_app_id])?;
                Ok(())
            }
        })
        .await
        .unwrap();

    let report_after_dup_delete = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    assert_eq!(report_after_dup_delete.total_mapped_count, 0);
    assert_eq!(report_after_dup_delete.total_quarantined_count, 0);

    let view_after_dup_delete = svc
        .canon_state_at(
            &project.project_id,
            &main_branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        view_after_dup_delete.quantity_balance(&veyra_id, "gold"),
        Some("125"),
        "removing duplicate progression_event must not invalidate authoritative quantity_state events"
    );
    let rel_after_dup_delete = view_after_dup_delete
        .relationship(&veyra_id, &bram_id)
        .expect("authoritative relationship must remain active after deleting applied canon_delta");
    assert_eq!(rel_after_dup_delete.resolved_trust, Some(50));
    assert_eq!(rel_after_dup_delete.resolved_tension, Some(10));

    let shadow_after_dup_delete = svc
        .compare_canon_compatibility_shadow_reads(&project.project_id, &main_branch.id, None)
        .await
        .unwrap();
    assert!(
        shadow_after_dup_delete.parity_clean,
        "expected clean shadow parity after deleting duplicate rows, got: {:#?}",
        shadow_after_dup_delete.differences
    );
}

#[tokio::test]
async fn backfill_and_shadow_parity_across_nested_branches_ancestor_edits_tombstones_merges_and_savepoints()
 {
    use spindle_core::models::{
        ArchiveEntityInput, CharacterStatePatch, CommitCharacterStateInput, CreateBranchInput,
        CreateLocationInput, CreateNarrativePromiseInput, CreateRelationshipInput,
        CreateSavePointInput, CreateWorldRuleInput, MergeBranchInput, RegisterCanonicalFactInput,
        RestoreSavePointInput, StoryPlacement, SwitchBranchInput, WorldStateInput,
    };

    let (_tmp, svc) = fresh_service_workspace().await;
    let (project, main_scene_id, _) = project_with_scene(&svc, "Nested Branch Parity").await;
    let main_branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();

    let veyra_id = create_character_named(&svc, &project.project_id, "Veyra").await;
    let bram_id = create_character_named(&svc, &project.project_id, "Bram").await;
    let _loc = svc
        .create_location(CreateLocationInput {
            project_id: project.project_id.clone(),
            name: "Sun Keep".into(),
            kind: "citadel".into(),
            realm: None,
            summary: "Fortress of the sun.".into(),
            initial_state: WorldStateInput {
                controlling_faction: Some("Sun Guard".into()),
                status: Some("standing".into()),
                prosperity: Some("high".into()),
                stability: Some("high".into()),
                threat_level: Some("low".into()),
                sensory_details: vec!["warm stone".into()],
            },
        })
        .await
        .unwrap();
    let main_promise = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project.project_id.clone(),
            promise_type: "mystery".into(),
            description: "Veyra hears footsteps behind the throne.".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: None,
            notes: Vec::new(),
        })
        .await
        .unwrap();
    svc.create_relationship(CreateRelationshipInput {
        character_a_id: veyra_id.clone(),
        character_b_id: bram_id.clone(),
        relationship_type: "allies".into(),
        initial_trust: 25,
        initial_tension: 10,
        dynamics: vec!["trusted".into()],
    })
    .await
    .unwrap();

    // Create `child` from `main`, and `grandchild` from `child`, plus `sibling` from `main`.
    let child = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "child-branch".into(),
            branch_type: "what_if".into(),
            description: Some("child".into()),
            parent_branch_id: Some(main_branch.id.clone()),
        })
        .await
        .unwrap();
    let grandchild = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "grandchild-branch".into(),
            branch_type: "what_if".into(),
            description: Some("grandchild".into()),
            parent_branch_id: Some(child.branch_id.clone()),
        })
        .await
        .unwrap();
    let sibling = svc
        .create_branch(CreateBranchInput {
            project_id: project.project_id.clone(),
            name: "sibling-branch".into(),
            branch_type: "what_if".into(),
            description: Some("sibling".into()),
            parent_branch_id: Some(main_branch.id.clone()),
        })
        .await
        .unwrap();

    // Switch to `child`: commit character state on inherited `main_scene_id` (routing to active descendant `child`)
    // and create + archive a child promise.
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: veyra_id.clone(),
        scene_id: main_scene_id.clone(),
        changes: CharacterStatePatch {
            emotional_state: [("courage".to_string(), serde_json::json!(0.9))]
                .into_iter()
                .collect(),
            goals: Some(vec!["defend the keep".into()]),
            status: Some(vec!["alive".into()]),
            notes: None,
            source_summary: Some("committed on child from inherited main scene".into()),
        },
    })
    .await
    .unwrap();
    let child_promise = svc
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project.project_id.clone(),
            promise_type: "foreshadowing".into(),
            description: "Bram notices the cracked seal.".into(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: None,
            notes: Vec::new(),
        })
        .await
        .unwrap();
    svc.archive_entity(ArchiveEntityInput {
        entity_type: "narrative_promise".into(),
        entity_id: child_promise.narrative_promise_id.clone(),
    })
    .await
    .unwrap();

    // Switch to `grandchild`: add a canonical fact override
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: grandchild.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.register_canonical_fact(RegisterCanonicalFactInput {
        project_id: project.project_id.clone(),
        scene_id: Some(main_scene_id.clone()),
        book_number: 1,
        chapter_number: 1,
        fact_type: None,
        key: None,
        value: None,
        context: None,
        subject_table: Some("character".into()),
        subject_id: Some(veyra_id.clone()),
        predicate: Some("title".into()),
        value_kind: Some("text".into()),
        value_text: Some("Grand Marshal".into()),
        value_number: None,
        value_unit: None,
        value_json: None,
        aliases: Vec::new(),
        scope: None,
        valid_from: None,
        valid_until: None,
        legacy_untyped: None,
        supersedes_fact_id: None,
        secrecy: None,
    })
    .await
    .unwrap();

    // Switch to `sibling`: add a world rule on `sibling`
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: sibling.branch_id.clone(),
    })
    .await
    .unwrap();
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project.project_id.clone(),
        rule_name: "Solar Resonance".into(),
        rule_type: "magic".into(),
        description: "Sunstones hum at noon.".into(),
        established_in: None,
        relevance_tags: vec!["sun".into()],
        scan_pattern: None,
    })
    .await
    .unwrap();

    // Live ancestor edit on `main` AFTER `child` and `grandchild` were forked!
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: main_branch.id.clone(),
    })
    .await
    .unwrap();
    svc.create_world_rule(CreateWorldRuleInput {
        project_id: project.project_id.clone(),
        rule_name: "Nightfall Curfew".into(),
        rule_type: "law".into(),
        description: "No lanterns on the outer wall after dusk.".into(),
        established_in: None,
        relevance_tags: vec!["curfew".into()],
        scan_pattern: None,
    })
    .await
    .unwrap();

    // Backfill and verify parity across all 4 branches (including sibling isolation and live ancestor edit inheritance)!
    let _ = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    for bid in [
        &main_branch.id,
        &child.branch_id,
        &grandchild.branch_id,
        &sibling.branch_id,
    ] {
        let rep = svc
            .compare_canon_compatibility_shadow_reads(&project.project_id, bid, None)
            .await
            .unwrap();
        assert!(
            rep.parity_clean,
            "expected clean parity on branch {bid}, got: {:#?}",
            rep.differences
        );
    }

    // Merge `sibling` into `main`, rerun backfill, and verify parity!
    svc.merge_branch(MergeBranchInput {
        project_id: project.project_id.clone(),
        source_branch_id: sibling.branch_id.clone(),
        target_branch_id: Some(main_branch.id.clone()),
        merge_type: "fast_forward".into(),
    })
    .await
    .unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    for bid in [&main_branch.id, &child.branch_id, &grandchild.branch_id] {
        let rep = svc
            .compare_canon_compatibility_shadow_reads(&project.project_id, bid, None)
            .await
            .unwrap();
        assert!(
            rep.parity_clean,
            "expected clean parity after merge on branch {bid}, got: {:#?}",
            rep.differences
        );
    }

    // Now test Save-Point Restore on `child`:
    // 1. Switch to `child` and create a save point.
    svc.switch_branch(SwitchBranchInput {
        project_id: project.project_id.clone(),
        branch_id: child.branch_id.clone(),
    })
    .await
    .unwrap();
    let sp = svc
        .create_save_point(CreateSavePointInput {
            project_id: project.project_id.clone(),
            name: "before-child-mutation".into(),
            description: Some("checkpoint before extra rule".into()),
        })
        .await
        .unwrap();

    // 2. Mutate `child` (add a new world rule), create a second savepoint `sp_with_rule`,
    //    and backfill so canon_event has the post-savepoint event (`@inc0`).
    let temp_rule = svc
        .create_world_rule(CreateWorldRuleInput {
            project_id: project.project_id.clone(),
            rule_name: "Temporary Child Rule".into(),
            rule_type: "custom".into(),
            description: "Should vanish when save point is restored.".into(),
            established_in: None,
            relevance_tags: Vec::new(),
            scan_pattern: None,
        })
        .await
        .unwrap();
    let sp_with_rule = svc
        .create_save_point(CreateSavePointInput {
            project_id: project.project_id.clone(),
            name: "with-child-rule".into(),
            description: Some("checkpoint with Temporary Child Rule".into()),
        })
        .await
        .unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();

    // 3. Restore `sp` (deleting `Temporary Child Rule`), rerun `backfill_project_canon`, and verify that
    //    stale post-savepoint canon events are invalidated and shadow parity is 100% clean!
    svc.restore_save_point(RestoreSavePointInput {
        project_id: project.project_id.clone(),
        save_point_id: sp.save_point_id.clone(),
    })
    .await
    .unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();

    let view_after_delete = svc
        .canon_state_at(
            &project.project_id,
            &child.branch_id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(
        !view_after_delete
            .rules
            .contains_key(&temp_rule.world_rule_id),
        "deleted rule must be invalidated in CanonStateViewV1"
    );

    let post_restore_shadow = svc
        .compare_canon_compatibility_shadow_reads(&project.project_id, &child.branch_id, None)
        .await
        .unwrap();
    assert!(
        post_restore_shadow.parity_clean,
        "expected clean shadow parity after savepoint restore, got: {:#?}",
        post_restore_shadow.differences
    );

    // 4. Now restore `sp_with_rule` (which brings back the exact same `Temporary Child Rule` row and snapshot!)
    //    and rerun `backfill_project_canon`. Verify that the restored row gets a fresh incarnation (`@inc1`),
    //    does NOT deduplicate to its suppressed `@inc0` events, is active in `view.rules`, and passes shadow parity!
    svc.restore_save_point(RestoreSavePointInput {
        project_id: project.project_id.clone(),
        save_point_id: sp_with_rule.save_point_id.clone(),
    })
    .await
    .unwrap();
    let re_restore_rep = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    assert!(re_restore_rep.total_mapped_count >= 1);

    let view_after_re_restore = svc
        .canon_state_at(
            &project.project_id,
            &child.branch_id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(
        view_after_re_restore
            .rules
            .get(&temp_rule.world_rule_id)
            .is_some_and(|r| r.defined),
        "restored rule must be re-emitted under a fresh incarnation and active in CanonStateViewV1"
    );

    let post_re_restore_shadow = svc
        .compare_canon_compatibility_shadow_reads(&project.project_id, &child.branch_id, None)
        .await
        .unwrap();
    assert!(
        post_re_restore_shadow.parity_clean,
        "expected clean shadow parity after delete/restore cycle, got: {:#?}",
        post_re_restore_shadow.differences
    );

    // Unchanged rerun after delete/restore cycle is 100% idempotent:
    let idem_after_re_restore = svc
        .backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    assert_eq!(idem_after_re_restore.total_mapped_count, 0);

    // 5. Insert a predecessor `character_state` snapshot after an uninjured successor at scene 10
    //    has already been backfilled. First backfill scene 10 (`status = ["alive"]`), then insert
    //    scene 5 (`status = ["alive", "injury:left_arm:severe"]`) and backfill again. Verify that
    //    reconciliation detects the newly inserted row, cascades to rebuild the scene-10 transition
    //    so it emits `InjuryOperation::Heal`, and leaves typed `injuries` empty at/after scene 10!
    svc.repository()
        .append_character_state(crate::sqlite::repository::AppendCharacterStateParams {
            project_id: project.project_id.clone(),
            branch_id: child.branch_id.clone(),
            character_id: veyra_id.clone(),
            scene_id: None,
            book_number: 1,
            chapter_number: 1,
            scene_order: 10,
            patch: CharacterStatePatch {
                emotional_state: Default::default(),
                goals: Some(vec!["reach the spire".into()]),
                status: Some(vec!["alive".into()]),
                notes: Some(vec!["uninjured at scene 10".into()]),
                source_summary: Some("scene 10 snapshot".into()),
            },
        })
        .await
        .unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();

    svc.repository()
        .append_character_state(crate::sqlite::repository::AppendCharacterStateParams {
            project_id: project.project_id.clone(),
            branch_id: child.branch_id.clone(),
            character_id: veyra_id.clone(),
            scene_id: None,
            book_number: 1,
            chapter_number: 1,
            scene_order: 5,
            patch: CharacterStatePatch {
                emotional_state: Default::default(),
                goals: Some(vec!["survive the ambush".into()]),
                status: Some(vec!["alive".into(), "injury:left_arm:severe".into()]),
                notes: Some(vec!["wounded at scene 5".into()]),
                source_summary: Some("scene 5 predecessor snapshot".into()),
            },
        })
        .await
        .unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();

    let left_arm_injury_id = format!("legacy_injury:{veyra_id}:left_arm");
    let view_at_scene_5 = svc
        .canon_state_at(
            &project.project_id,
            &child.branch_id,
            CanonCursorV1 {
                position: Some(CanonPositionV1 {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: 5,
                }),
                within_scene_order: None,
            },
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(
        view_at_scene_5
            .active_injuries(&veyra_id)
            .contains_key(left_arm_injury_id.as_str()),
        "expected left_arm injury to be active at scene 5"
    );

    let view_after_scene_10 = svc
        .canon_state_at(
            &project.project_id,
            &child.branch_id,
            CanonCursorV1 {
                position: Some(CanonPositionV1 {
                    book_number: 1,
                    chapter_number: 1,
                    scene_order: 10,
                }),
                within_scene_order: None,
            },
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(
        view_after_scene_10.active_injuries(&veyra_id).is_empty(),
        "expected scene 10 successor to emit Heal after predecessor insertion, got: {:?}",
        view_after_scene_10.active_injuries(&veyra_id)
    );
    let injury_at_10 = view_after_scene_10
        .entities
        .get(&veyra_id)
        .and_then(|e| e.injuries.get(&left_arm_injury_id))
        .expect("expected left_arm injury record at scene 10");
    assert_eq!(
        injury_at_10.healed_at,
        Some(CanonPositionV1 {
            book_number: 1,
            chapter_number: 1,
            scene_order: 10,
        })
    );

    let _ = main_promise;
}

#[tokio::test]
async fn per_project_cutover_readiness_gate_isolates_quarantined_and_unmapped_projects() {
    use crate::sqlite::repository::AppendProgressionEventParams;

    let (_tmp, svc) = fresh_service_local().await;
    let (clean_proj, _, _) = project_with_scene(&svc, "Clean Project").await;
    let (quarantined_proj, q_scene_id, _) = project_with_scene(&svc, "Quarantined Project").await;
    let q_branch = svc
        .repository()
        .get_active_branch(&quarantined_proj.project_id)
        .await
        .unwrap();
    let q_char = create_character_named(&svc, &quarantined_proj.project_id, "Kael").await;

    // Insert an opaque progression_event on `quarantined_proj`.
    svc.repository()
        .append_progression_event(AppendProgressionEventParams {
            project_id: quarantined_proj.project_id.clone(),
            branch_id: q_branch.id.clone(),
            subject_table: "character".into(),
            subject_id: q_char.clone(),
            overlay_id: None,
            kind: "opaque_legacy_mutation".into(),
            delta_json: serde_json::json!({
                "arbitrary_blob": [1, 2, 3]
            }),
            source_scene_id: Some(q_scene_id.clone()),
            placement: None,
            created_at: chrono::Utc::now(),
        })
        .await
        .unwrap();

    // Before backfill on `clean_proj` (if it had any canon rows) or `quarantined_proj`:
    // `quarantined_proj` has 2 unmapped legacy rows (`character_state` from create_character + `progression_event`), so cutover is blocked.
    let pre_q_gate = svc
        .evaluate_project_canon_cutover_readiness(&quarantined_proj.project_id)
        .await
        .unwrap();
    assert!(!pre_q_gate.cutover_allowed);
    assert_eq!(pre_q_gate.unmapped_legacy_row_count, 2);

    // Backfill both projects.
    let clean_rep = svc
        .backfill_project_canon(&clean_proj.project_id)
        .await
        .unwrap();
    assert_eq!(clean_rep.total_quarantined_count, 0);

    let q_rep = svc
        .backfill_project_canon(&quarantined_proj.project_id)
        .await
        .unwrap();
    assert_eq!(q_rep.total_quarantined_count, 1);
    assert_eq!(q_rep.quarantined_entries.len(), 1);
    assert_eq!(
        q_rep.quarantined_entries[0].source_table,
        "progression_event"
    );

    // Verify per-project isolation: `clean_proj` is ready for cutover, while `quarantined_proj` is blocked!
    let clean_gate = svc
        .evaluate_project_canon_cutover_readiness(&clean_proj.project_id)
        .await
        .unwrap();
    assert!(
        clean_gate.cutover_allowed,
        "clean project should allow cutover, blocking_reasons: {:?}",
        clean_gate.blocking_reasons
    );

    let post_q_gate = svc
        .evaluate_project_canon_cutover_readiness(&quarantined_proj.project_id)
        .await
        .unwrap();
    assert!(!post_q_gate.cutover_allowed);
    assert_eq!(post_q_gate.quarantined_count, 1);
    assert_eq!(post_q_gate.unmapped_legacy_row_count, 0);
}

// ── PR E: tier decisions, calibrated policy, revocation and cutover ─────────

#[allow(clippy::too_many_arguments)]
fn policy_claim(
    project_id: &str,
    branch_id: &str,
    scene_id: &str,
    revision_id: &str,
    text: &str,
    claim_id: &str,
    subject_id: &str,
    assertion: ClaimAssertion,
    confidence: f64,
) -> ClaimV1 {
    ClaimV1 {
        binding: ClaimBindingV1 {
            schema_version: CLAIM_SCHEMA_VERSION,
            project_id: project_id.to_string(),
            branch_id: branch_id.to_string(),
            scene_id: Some(scene_id.to_string()),
            revision_id: revision_id.to_string(),
            accepted_text_sha256: exact_text_sha256(text),
            position: spindle_core::models::StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            extraction_run_id: format!("run:{revision_id}"),
            model: "test-model".into(),
            agent_id: None,
            model_call_id: None,
            prompt_version: "canon-extract-v1".into(),
        },
        source: ClaimSource::Independent,
        proposed: ProposedClaimV1 {
            claim_id: claim_id.to_string(),
            subject: ClaimEntity::Resolved {
                id: subject_id.to_string(),
            },
            assertion,
            within_scene_order: Some(0),
            in_world_time: None,
            modality: ClaimModality::Narration,
            temporal_scope: TemporalScope::Current,
            speaker: None,
            experiencer: None,
            confidence: Some(confidence),
            evidence: vec![ClaimEvidence {
                start_byte: 0,
                end_byte: text.len(),
                quote: text.to_string(),
            }],
        },
    }
}

fn policy_context(text: &str) -> CanonPolicyContextV1 {
    let mut accepted_revisions = std::collections::BTreeMap::new();
    accepted_revisions.insert(
        "revision:policy:1".to_string(),
        AcceptedClaimRevisionV1 {
            text: text.to_string(),
            accepted_text_sha256: exact_text_sha256(text),
        },
    );
    CanonPolicyContextV1 {
        accepted_revisions,
        ratified_state: None,
        package_agreed_fingerprints: std::collections::BTreeSet::new(),
        secret_linked_propositions: std::collections::BTreeSet::new(),
        existing_promise_thread_ids: std::collections::BTreeSet::new(),
        calibrations: vec![
            CanonCalibrationProfileV1 {
                class: CanonClaimClassV1::ExplicitMovement,
                model: "test-model".into(),
                prompt_version: "canon-extract-v1".into(),
                accepted_claims: 400,
                correct_claims: 400,
            },
            CanonCalibrationProfileV1 {
                class: CanonClaimClassV1::DeathOrResurrection,
                model: "test-model".into(),
                prompt_version: "canon-extract-v1".into(),
                accepted_claims: 400,
                correct_claims: 400,
            },
            CanonCalibrationProfileV1 {
                class: CanonClaimClassV1::SecretDisclosure,
                model: "test-model".into(),
                prompt_version: "canon-extract-v1".into(),
                accepted_claims: 400,
                correct_claims: 400,
            },
        ],
        config: CanonPolicyConfigV1::default(),
    }
}

fn policy_input(
    project_id: &str,
    branch_id: &str,
    scene_id: &str,
    claim: ClaimV1,
    context: CanonPolicyContextV1,
    previous_decision_id: Option<String>,
) -> RecordCanonPolicyInputV1 {
    RecordCanonPolicyInputV1 {
        project_id: project_id.to_string(),
        branch_id: branch_id.to_string(),
        source_revision_id: claim.binding.revision_id.clone(),
        target_scene_id: Some(scene_id.to_string()),
        previous_decision_id,
        replace_prior_scene_revisions: false,
        requests: vec![CanonPolicyClaimRequestV1 {
            claim,
            provenance: CanonProvenanceKind::Extracted,
            secret_linked: false,
            supersedes_event_id: None,
        }],
        context,
    }
}

#[tokio::test]
async fn policy_audit_records_preconditions_and_stale_decision_fails_safely() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Policy Audit").await;
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let text = "Veyra walks into the market.";
    let claim = policy_claim(
        &project.project_id,
        &branch.id,
        &scene_id,
        "revision:policy:1",
        text,
        "claim:movement",
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: ClaimEntity::Resolved {
                id: "location:market".into(),
            },
        },
        1.0,
    );
    let mut context = policy_context(text);
    context
        .package_agreed_fingerprints
        .insert(semantic_claim_fingerprint(&claim));
    let input = policy_input(
        &project.project_id,
        &branch.id,
        &scene_id,
        claim,
        context,
        None,
    );
    let first = svc
        .record_canon_policy_decisions(input.clone())
        .await
        .unwrap();
    assert_eq!(
        first.evaluations[0].disposition,
        CanonPolicyDispositionV1::AutoRatify
    );
    assert_eq!(first.claims[0].status, CanonClaimStatus::Ratified);
    assert_eq!(
        first.decisions[0].decision.policy_version,
        CANON_POLICY_VERSION_V1
    );
    assert!(
        first.decisions[0]
            .decision
            .precondition_summary
            .contains("evidence=pass")
    );
    assert_eq!(first.decisions[0].decision.confidence_inputs.len(), 2);

    // Idempotent retry: one claim row, one deduplicated decision.
    let retry = svc.record_canon_policy_decisions(input).await.unwrap();
    assert!(retry.decisions[0].deduplicated);
    let claims = svc
        .list_canon_claims(
            &project.project_id,
            &branch.id,
            None,
            Some("revision:policy:1"),
        )
        .await
        .unwrap();
    assert_eq!(claims.len(), 1);

    // A non-duplicate decision against a bogus tip fails safely.
    let text2 = "Veyra walks into the tavern.";
    let mut stale_claim = policy_claim(
        &project.project_id,
        &branch.id,
        &scene_id,
        "revision:policy:2",
        text2,
        "claim:movement2",
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: ClaimEntity::Resolved {
                id: "location:tavern".into(),
            },
        },
        1.0,
    );
    stale_claim.binding.accepted_text_sha256 = exact_text_sha256(text2);
    let mut stale_context = policy_context(text2);
    stale_context.accepted_revisions.insert(
        "revision:policy:2".into(),
        AcceptedClaimRevisionV1 {
            text: text2.into(),
            accepted_text_sha256: exact_text_sha256(text2),
        },
    );
    stale_context
        .package_agreed_fingerprints
        .insert(semantic_claim_fingerprint(&stale_claim));
    let stale = policy_input(
        &project.project_id,
        &branch.id,
        &scene_id,
        stale_claim,
        stale_context,
        Some("canon_decision:does-not-exist".into()),
    );
    let error = svc.record_canon_policy_decisions(stale).await.unwrap_err();
    assert!(
        format!("{error}").contains("stale canon decision"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn hard_v1_high_impact_claims_stage_provisional_not_ratified() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Policy Safety").await;
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let text = "Veyra dies and a secret is spoken aloud.";
    let death = policy_claim(
        &project.project_id,
        &branch.id,
        &scene_id,
        "revision:policy:1",
        text,
        "claim:death",
        "character:veyra",
        ClaimAssertion::LifeStatus {
            status: LifeStatus::Dead,
        },
        1.0,
    );
    let secret = policy_claim(
        &project.project_id,
        &branch.id,
        &scene_id,
        "revision:policy:1",
        text,
        "claim:secret",
        "character:veyra",
        ClaimAssertion::Knowledge {
            operation: KnowledgeOperation::Disclose,
            proposition: ClaimEntity::Resolved {
                id: "proposition:secret".into(),
            },
            audience: vec![ClaimEntity::Resolved {
                id: "character:other".into(),
            }],
            source: None,
        },
        1.0,
    );
    let mut context = policy_context(text);
    context
        .package_agreed_fingerprints
        .insert(semantic_claim_fingerprint(&death));
    context
        .package_agreed_fingerprints
        .insert(semantic_claim_fingerprint(&secret));
    let input = RecordCanonPolicyInputV1 {
        project_id: project.project_id.clone(),
        branch_id: branch.id.clone(),
        source_revision_id: "revision:policy:1".into(),
        target_scene_id: Some(scene_id.clone()),
        previous_decision_id: None,
        replace_prior_scene_revisions: false,
        requests: vec![
            CanonPolicyClaimRequestV1 {
                claim: death,
                provenance: CanonProvenanceKind::Extracted,
                secret_linked: false,
                supersedes_event_id: None,
            },
            CanonPolicyClaimRequestV1 {
                claim: secret,
                provenance: CanonProvenanceKind::Extracted,
                secret_linked: true,
                supersedes_event_id: None,
            },
        ],
        context,
    };
    let output = svc.record_canon_policy_decisions(input).await.unwrap();
    assert!(output.evaluations.iter().all(|evaluation| {
        evaluation.disposition == CanonPolicyDispositionV1::StageProvisional
    }));
    assert!(
        output
            .claims
            .iter()
            .all(|claim| claim.status == CanonClaimStatus::Provisional)
    );
    let ratified = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(ratified.life_status("character:veyra"), LifeStatus::Unknown);
    let provisional = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Provisional,
            None,
        )
        .await
        .unwrap();
    assert_eq!(provisional.life_status("character:veyra"), LifeStatus::Dead);
}

#[tokio::test]
async fn revoked_policy_batch_is_withdrawn_and_dependents_are_invalidated() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Policy Revoke").await;
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let text = "Veyra walks into the market.";
    let claim = policy_claim(
        &project.project_id,
        &branch.id,
        &scene_id,
        "revision:policy:1",
        text,
        "claim:movement",
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: ClaimEntity::Resolved {
                id: "location:market".into(),
            },
        },
        1.0,
    );
    let mut context = policy_context(text);
    context
        .package_agreed_fingerprints
        .insert(semantic_claim_fingerprint(&claim));
    let output = svc
        .record_canon_policy_decisions(policy_input(
            &project.project_id,
            &branch.id,
            &scene_id,
            claim,
            context,
            None,
        ))
        .await
        .unwrap();
    let before = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(before.location("character:veyra"), Some("location:market"));

    let revoked = svc
        .revoke_canon_decision_batch(RevokeCanonDecisionInputV1 {
            project_id: project.project_id.clone(),
            branch_id: branch.id.clone(),
            decision_id: output.decisions[0].decision.decision_id.clone(),
            actor: "author:human".into(),
            reason: "wrong scene placement".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        revoked.decision.decision.action,
        CanonPolicyAction::Withdraw
    );
    assert!(!revoked.invalidated_event_ids.is_empty());
    let after = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert_eq!(after.location("character:veyra"), None);
    let claims = svc
        .list_canon_claims(
            &project.project_id,
            &branch.id,
            None,
            Some("revision:policy:1"),
        )
        .await
        .unwrap();
    assert_eq!(claims[0].status, CanonClaimStatus::Withdrawn);
}

#[tokio::test]
async fn cutover_gate_allows_ordered_transitions_and_rejects_shortcuts() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, _scene_id, _) = project_with_scene(&svc, "Cutover Gate").await;
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    let readiness = svc
        .evaluate_project_canon_cutover_readiness(&project.project_id)
        .await
        .unwrap();
    assert!(
        readiness.cutover_allowed,
        "empty project should be ready: {:?}",
        readiness.blocking_reasons
    );
    let dual = svc
        .set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    assert_eq!(dual.mode, CanonCutoverModeV1::DualWrite);
    assert_eq!(
        svc.canon_cutover_mode(&project.project_id).await.unwrap(),
        CanonCutoverModeV1::DualWrite
    );
    // Legacy -> CanonRead must pass through dual-write/shadow-read.
    let shortcut = svc
        .set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::CanonRead, "skip")
        .await;
    assert!(shortcut.is_err());
    let rollback = svc
        .set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::Legacy, "rollback")
        .await
        .unwrap();
    assert_eq!(rollback.mode, CanonCutoverModeV1::Legacy);
}

#[tokio::test]
async fn dual_write_commit_marks_legacy_rows_mapped_and_records_audit() {
    use spindle_core::models::{CanonicalFactEntry, CanonicalFactScope, CommitSceneChangesInput};
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Dual Write").await;
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    let output = svc
        .commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: vec![CanonicalFactEntry {
                fact_type: None,
                key: None,
                value: None,
                subject_table: Some("character".into()),
                subject_id: Some("character:veyra".into()),
                predicate: Some("eye_color".into()),
                value_kind: Some("string".into()),
                value_text: Some("blue".into()),
                value_number: None,
                value_unit: None,
                value_json: None,
                aliases: Vec::new(),
                scope: Some(CanonicalFactScope::Invariant),
                valid_from: None,
                valid_until: None,
                context: None,
                supersedes_fact_id: None,
            }],
            relationship_updates: Vec::new(),
            accept_world_rule_risks: true,
            accept_continuity_risks: true,
            continuity_gate: Some(spindle_core::models::CommitContinuityGate::Off),
        })
        .await
        .unwrap();
    assert_eq!(output.canonical_facts.len(), 1);
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let claims = svc
        .list_canon_claims(&project.project_id, &branch.id, Some(&scene_id), None)
        .await
        .unwrap();
    assert!(!claims.is_empty());
    assert!(
        claims
            .iter()
            .all(|claim| claim.provenance == CanonProvenanceKind::ManualAuthor)
    );
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    assert!(state.entity("character:veyra").is_some());
    // Exact live-write snapshots are mapped so a backfill rerun is idempotent.
    let readiness = svc
        .evaluate_project_canon_cutover_readiness(&project.project_id)
        .await
        .unwrap();
    assert_eq!(readiness.unmapped_legacy_row_count, 0);
}

#[tokio::test]
async fn legacy_unverified_provisional_findings_never_become_error_blocks() {
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Legacy Findings").await;
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let text = "Veyra walks into the market.";
    let claim = policy_claim(
        &project.project_id,
        &branch.id,
        &scene_id,
        "revision:policy:1",
        text,
        "claim:legacy",
        "character:veyra",
        ClaimAssertion::LocatedAt {
            location: ClaimEntity::Resolved {
                id: "location:market".into(),
            },
        },
        1.0,
    );
    svc.record_canon_claims(
        &project.project_id,
        &branch.id,
        &[claim],
        CanonClaimStatus::Provisional,
        CanonTier::Provisional,
        CanonProvenanceKind::LegacyUnverified,
    )
    .await
    .unwrap();
    let issues = svc
        .canon_consistency_issues(&project.project_id, &branch.id)
        .await
        .unwrap();
    assert!(!issues.is_empty());
    assert!(issues.iter().all(|issue| issue.severity != "error"));
    assert!(
        issues
            .iter()
            .all(|issue| issue.message.contains("provisional canon"))
    );
    let pending = svc
        .canon_pending_work(&project.project_id, &branch.id)
        .await
        .unwrap();
    assert!(!pending.continuity_clean());
    assert!(pending.next_scene_has_pending_work);
}

#[tokio::test]
async fn dual_write_character_state_emits_compatibility_snapshot_for_canon_read() {
    use spindle_core::models::{
        CharacterStatePatch, CommitCharacterStateInput, GetCharacterSnapshotInput,
    };
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "State Snapshot Parity").await;
    let character_id = create_character_named(&svc, &project.project_id, "Veyra").await;
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    let mut emotional_state = std::collections::BTreeMap::new();
    emotional_state.insert("fear".to_string(), serde_json::json!("tight"));
    svc.commit_character_state(CommitCharacterStateInput {
        character_id: character_id.clone(),
        scene_id: scene_id.clone(),
        changes: CharacterStatePatch {
            emotional_state,
            goals: Some(vec!["escape the docks".into()]),
            status: Some(vec!["alive".into()]),
            notes: Some(vec!["left hand bandaged".into()]),
            source_summary: Some("dual-write snapshot".into()),
        },
    })
    .await
    .unwrap();
    let readiness = svc
        .evaluate_project_canon_cutover_readiness(&project.project_id)
        .await
        .unwrap();
    assert_eq!(
        readiness.unmapped_legacy_row_count, 0,
        "blocking: {:?}",
        readiness.blocking_reasons
    );
    assert!(
        readiness.cutover_allowed,
        "blocking: {:?}",
        readiness.blocking_reasons
    );
    svc.set_canon_cutover_mode(
        &project.project_id,
        CanonCutoverModeV1::ShadowRead,
        "shadow",
    )
    .await
    .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::CanonRead, "read")
        .await
        .unwrap();
    let snapshot = svc
        .get_character_snapshot(GetCharacterSnapshotInput {
            project_id: project.project_id.clone(),
            character_id: character_id.clone(),
            branch_id: None,
        })
        .await
        .unwrap();
    let state = snapshot.current_state.expect("projected character state");
    assert_eq!(state.goals, vec!["escape the docks"]);
    assert_eq!(state.notes, vec!["left hand bandaged"]);
    assert_eq!(
        state.emotional_state.get("fear"),
        Some(&serde_json::json!("tight"))
    );
}

#[tokio::test]
async fn dual_write_relationship_replaces_prior_scene_delta_and_reverts() {
    use spindle_core::models::UpdateRelationshipInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Relationship Replace").await;
    let a = create_character_named(&svc, &project.project_id, "Veyra").await;
    let b = create_character_named(&svc, &project.project_id, "Kael").await;
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    for trust_delta in [2, 3, 2] {
        svc.update_relationship(UpdateRelationshipInput {
            character_a_id: a.clone(),
            character_b_id: b.clone(),
            trust_delta,
            tension_delta: 0,
            reason: "scene revision".into(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();
    }
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let relationship = state.relationship(&a, &b).expect("canon relationship");
    assert_eq!(relationship.accumulated_trust_delta, 2);
    assert_eq!(relationship.accumulated_tension_delta, 0);
    let legacy = svc
        .repository()
        .list_relationships_by_branch(&branch.id)
        .await
        .unwrap();
    let legacy_pair = legacy
        .iter()
        .find(|row| (row.in_id == a && row.out_id == b) || (row.in_id == b && row.out_id == a))
        .expect("legacy relationship");
    assert_eq!(legacy_pair.trust, 2);
}

#[tokio::test]
async fn dual_write_commit_scene_relationship_replaces_prior_scene_delta() {
    use spindle_core::models::{CommitSceneChangesInput, RelationshipUpdateEntry};
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Commit Relationship Replace").await;
    let a = create_character_named(&svc, &project.project_id, "Veyra").await;
    let b = create_character_named(&svc, &project.project_id, "Kael").await;
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    for trust_delta in [2, 3, 2] {
        svc.commit_scene_changes(CommitSceneChangesInput {
            project_id: project.project_id.clone(),
            scene_id: scene_id.clone(),
            character_states: Vec::new(),
            canonical_facts: Vec::new(),
            relationship_updates: vec![RelationshipUpdateEntry {
                character_a_id: a.clone(),
                character_b_id: b.clone(),
                trust_delta,
                tension_delta: 0,
                reason: "scene revision".into(),
            }],
            accept_world_rule_risks: true,
            accept_continuity_risks: true,
            continuity_gate: Some(spindle_core::models::CommitContinuityGate::Off),
        })
        .await
        .unwrap();
    }
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let relationship = state.relationship(&a, &b).expect("canon relationship");
    assert_eq!(relationship.accumulated_trust_delta, 2);
    let legacy = svc
        .repository()
        .list_relationships_by_branch(&branch.id)
        .await
        .unwrap();
    let legacy_pair = legacy
        .iter()
        .find(|row| (row.in_id == a && row.out_id == b) || (row.in_id == b && row.out_id == a))
        .expect("legacy relationship");
    assert_eq!(legacy_pair.trust, 2);
}

#[tokio::test]
async fn dual_write_relationship_replaces_backfilled_scene_delta_after_cutover() {
    use spindle_core::models::UpdateRelationshipInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Backfilled Relationship Replace").await;
    let a = create_character_named(&svc, &project.project_id, "Veyra").await;
    let b = create_character_named(&svc, &project.project_id, "Kael").await;
    // Seed a nonzero legacy scene contribution before canonical backfill.
    svc.update_relationship(UpdateRelationshipInput {
        character_a_id: a.clone(),
        character_b_id: b.clone(),
        trust_delta: 2,
        tension_delta: 0,
        reason: "pre-cutover".into(),
        scene_id: scene_id.clone(),
    })
    .await
    .unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    for trust_delta in [3, 2] {
        svc.update_relationship(UpdateRelationshipInput {
            character_a_id: a.clone(),
            character_b_id: b.clone(),
            trust_delta,
            tension_delta: 0,
            reason: "post-cutover revision".into(),
            scene_id: scene_id.clone(),
        })
        .await
        .unwrap();
    }
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let relationship = state.relationship(&a, &b).expect("canon relationship");
    assert_eq!(relationship.accumulated_trust_delta, 2);
    let legacy = svc
        .repository()
        .list_relationships_by_branch(&branch.id)
        .await
        .unwrap();
    let legacy_pair = legacy
        .iter()
        .find(|row| (row.in_id == a && row.out_id == b) || (row.in_id == b && row.out_id == a))
        .expect("legacy relationship");
    assert_eq!(legacy_pair.trust, 2);
}

#[tokio::test]
async fn dual_write_commit_scene_relationship_replaces_backfilled_scene_delta() {
    use spindle_core::models::{
        CommitContinuityGate, CommitSceneChangesInput, RelationshipUpdateEntry,
    };
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Backfilled Commit Replace").await;
    let a = create_character_named(&svc, &project.project_id, "Veyra").await;
    let b = create_character_named(&svc, &project.project_id, "Kael").await;
    let commit = |trust_delta: i32| CommitSceneChangesInput {
        project_id: project.project_id.clone(),
        scene_id: scene_id.clone(),
        character_states: Vec::new(),
        canonical_facts: Vec::new(),
        relationship_updates: vec![RelationshipUpdateEntry {
            character_a_id: a.clone(),
            character_b_id: b.clone(),
            trust_delta,
            tension_delta: 0,
            reason: "scene revision".into(),
        }],
        accept_world_rule_risks: true,
        accept_continuity_risks: true,
        continuity_gate: Some(CommitContinuityGate::Off),
    };
    svc.commit_scene_changes(commit(2)).await.unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    for trust_delta in [3, 2] {
        svc.commit_scene_changes(commit(trust_delta)).await.unwrap();
    }
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let relationship = state.relationship(&a, &b).expect("canon relationship");
    assert_eq!(relationship.accumulated_trust_delta, 2);
    let legacy = svc
        .repository()
        .list_relationships_by_branch(&branch.id)
        .await
        .unwrap();
    let legacy_pair = legacy
        .iter()
        .find(|row| (row.in_id == a && row.out_id == b) || (row.in_id == b && row.out_id == a))
        .expect("legacy relationship");
    assert_eq!(legacy_pair.trust, 2);
}

#[tokio::test]
async fn dual_write_relationship_replaces_live_delta_across_reversed_pair_arguments() {
    use spindle_core::models::UpdateRelationshipInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Reversed Pair Live").await;
    let a = create_character_named(&svc, &project.project_id, "Veyra").await;
    let b = create_character_named(&svc, &project.project_id, "Kael").await;
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    let update = |first: &str, second: &str, delta: i32| UpdateRelationshipInput {
        character_a_id: first.to_string(),
        character_b_id: second.to_string(),
        trust_delta: delta,
        tension_delta: 0,
        reason: "reversed revision".into(),
        scene_id: scene_id.clone(),
    };
    svc.update_relationship(update(&a, &b, 2)).await.unwrap();
    svc.update_relationship(update(&b, &a, 3)).await.unwrap();
    svc.update_relationship(update(&a, &b, 2)).await.unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let relationship = state
        .relationship(&a, &b)
        .or_else(|| state.relationship(&b, &a))
        .expect("canon relationship");
    assert_eq!(relationship.accumulated_trust_delta, 2);
    let legacy = svc
        .repository()
        .list_relationships_by_branch(&branch.id)
        .await
        .unwrap();
    let legacy_pair = legacy
        .iter()
        .find(|row| (row.in_id == a && row.out_id == b) || (row.in_id == b && row.out_id == a))
        .expect("legacy relationship");
    assert_eq!(legacy_pair.trust, 2);
}

#[tokio::test]
async fn dual_write_relationship_replaces_backfilled_delta_across_reversed_pair_arguments() {
    use spindle_core::models::UpdateRelationshipInput;
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Reversed Pair Backfill").await;
    let a = create_character_named(&svc, &project.project_id, "Veyra").await;
    let b = create_character_named(&svc, &project.project_id, "Kael").await;
    let update = |first: &str, second: &str, delta: i32| UpdateRelationshipInput {
        character_a_id: first.to_string(),
        character_b_id: second.to_string(),
        trust_delta: delta,
        tension_delta: 0,
        reason: "reversed revision".into(),
        scene_id: scene_id.clone(),
    };
    svc.update_relationship(update(&a, &b, 2)).await.unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    svc.update_relationship(update(&b, &a, 3)).await.unwrap();
    svc.update_relationship(update(&a, &b, 2)).await.unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let relationship = state
        .relationship(&a, &b)
        .or_else(|| state.relationship(&b, &a))
        .expect("canon relationship");
    assert_eq!(relationship.accumulated_trust_delta, 2);
    let legacy = svc
        .repository()
        .list_relationships_by_branch(&branch.id)
        .await
        .unwrap();
    let legacy_pair = legacy
        .iter()
        .find(|row| (row.in_id == a && row.out_id == b) || (row.in_id == b && row.out_id == a))
        .expect("legacy relationship");
    assert_eq!(legacy_pair.trust, 2);
}

#[tokio::test]
async fn dual_write_commit_scene_relationship_replaces_delta_across_reversed_pair_arguments() {
    use spindle_core::models::{
        CommitContinuityGate, CommitSceneChangesInput, RelationshipUpdateEntry,
    };
    let (_tmp, svc) = fresh_service_local().await;
    let (project, scene_id, _) = project_with_scene(&svc, "Reversed Pair Commit").await;
    let a = create_character_named(&svc, &project.project_id, "Veyra").await;
    let b = create_character_named(&svc, &project.project_id, "Kael").await;
    let commit = |first: &str, second: &str, delta: i32| CommitSceneChangesInput {
        project_id: project.project_id.clone(),
        scene_id: scene_id.clone(),
        character_states: Vec::new(),
        canonical_facts: Vec::new(),
        relationship_updates: vec![RelationshipUpdateEntry {
            character_a_id: first.to_string(),
            character_b_id: second.to_string(),
            trust_delta: delta,
            tension_delta: 0,
            reason: "reversed revision".into(),
        }],
        accept_world_rule_risks: true,
        accept_continuity_risks: true,
        continuity_gate: Some(CommitContinuityGate::Off),
    };
    svc.commit_scene_changes(commit(&a, &b, 2)).await.unwrap();
    svc.backfill_project_canon(&project.project_id)
        .await
        .unwrap();
    svc.set_canon_cutover_mode(&project.project_id, CanonCutoverModeV1::DualWrite, "start")
        .await
        .unwrap();
    svc.commit_scene_changes(commit(&b, &a, 3)).await.unwrap();
    svc.commit_scene_changes(commit(&a, &b, 2)).await.unwrap();
    let branch = svc
        .repository()
        .get_active_branch(&project.project_id)
        .await
        .unwrap();
    let state = svc
        .canon_state_at(
            &project.project_id,
            &branch.id,
            CanonCursorV1::unbounded(),
            CanonTier::Ratified,
            None,
        )
        .await
        .unwrap();
    let relationship = state
        .relationship(&a, &b)
        .or_else(|| state.relationship(&b, &a))
        .expect("canon relationship");
    assert_eq!(relationship.accumulated_trust_delta, 2);
    let legacy = svc
        .repository()
        .list_relationships_by_branch(&branch.id)
        .await
        .unwrap();
    let legacy_pair = legacy
        .iter()
        .find(|row| (row.in_id == a && row.out_id == b) || (row.in_id == b && row.out_id == a))
        .expect("legacy relationship");
    assert_eq!(legacy_pair.trust, 2);
}
