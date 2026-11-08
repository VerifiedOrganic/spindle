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
