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
