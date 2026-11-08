use super::*;

/// Contract: for any (book, chapter), the scenes `list_chapter_scenes`
/// reports as the spine are EXACTLY the scenes `compile_manuscript`
/// renders, and every one is addressable by the position resolver. This is
/// the invariant whose failure let an operator position-delete the real
/// scene while cleaning up phantom duplicates.
#[tokio::test]
async fn list_chapter_scenes_matches_compiled_spine_and_position_resolver() {
    use spindle_core::models::{CompileManuscriptInput, ListChapterScenesInput};
    use std::collections::BTreeSet;

    let (_tmp, svc, proj, spine_ids, alt_ids) = spine_chapter_with_unresolved_alternatives().await;
    let active_branch = svc
        .repository()
        .get_active_branch(&proj.project_id)
        .await
        .unwrap();

    let listing = svc
        .list_chapter_scenes(ListChapterScenesInput {
            project_id: proj.project_id.clone(),
            chapter_id: None,
            book_number: 1,
            chapter_number: 4,
        })
        .await
        .unwrap();

    // 1. The listing is the three-scene spine, in order — no phantom rows.
    let listed_ids = listing
        .scenes
        .iter()
        .map(|entry| entry.scene_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        listed_ids, spine_ids,
        "listing must be exactly the active-branch spine"
    );
    for alt_id in &alt_ids {
        assert!(
            !listed_ids.contains(alt_id),
            "alternative-branch scene {alt_id} leaked into the spine listing"
        );
    }

    // 2. The listing agrees with the raw active-branch spine rows.
    let branch_scenes = svc
        .repository()
        .list_scenes_by_project_and_branch(&proj.project_id, &active_branch.id)
        .await
        .unwrap()
        .into_iter()
        .filter(|scene| scene.book_number == 1 && scene.chapter_number == 4)
        .map(|scene| scene.id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        listed_ids.iter().cloned().collect::<BTreeSet<_>>(),
        branch_scenes,
        "listing and branch-scoped spine query must return the same scene set"
    );

    // 3. compile_manuscript renders exactly the listed scenes.
    let compiled = svc
        .compile_manuscript(CompileManuscriptInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            start_chapter: Some(4),
            end_chapter: Some(4),
            write_to_workspace: false,
        })
        .await
        .unwrap();
    assert_eq!(
        compiled.scene_count,
        listing.scenes.len(),
        "compile_manuscript rendered {} scenes but the listing reports {}",
        compiled.scene_count,
        listing.scenes.len()
    );
    assert!(
        compiled.missing_scenes.is_empty(),
        "a healthy chapter must compile with no missing scenes, got {:?}",
        compiled.missing_scenes
    );

    // 4. Every listed scene is addressable by the position resolver.
    for entry in &listing.scenes {
        let resolved = svc
            .repository()
            .find_scene_by_natural_key(&proj.project_id, &active_branch.id, 1, 4, entry.scene_order)
            .await
            .unwrap()
            .unwrap_or_else(|| {
                panic!(
                    "position resolver lost listed scene at order {}",
                    entry.scene_order
                )
            });
        assert_eq!(
            resolved.id, entry.scene_id,
            "position resolver disagrees with the listing at order {}",
            entry.scene_order
        );
    }
}

/// Regression for the observed duplicate-`scene_order` shape: the listing
/// may never carry two rows at one position. Three rows at order 2 is what
/// made a healthy chapter look corrupted.
#[tokio::test]
async fn list_chapter_scenes_never_lists_two_scenes_at_one_position() {
    use spindle_core::models::ListChapterScenesInput;
    use std::collections::BTreeMap;

    let (_tmp, svc, proj, _spine_ids, _alt_ids) =
        spine_chapter_with_unresolved_alternatives().await;

    let listing = svc
        .list_chapter_scenes(ListChapterScenesInput {
            project_id: proj.project_id.clone(),
            chapter_id: None,
            book_number: 1,
            chapter_number: 4,
        })
        .await
        .unwrap();

    let mut by_order: BTreeMap<i32, Vec<String>> = BTreeMap::new();
    for entry in &listing.scenes {
        by_order
            .entry(entry.scene_order)
            .or_default()
            .push(entry.scene_id.clone());
    }
    for (order, ids) in &by_order {
        assert_eq!(
            ids.len(),
            1,
            "scene_order {order} listed {} rows: {ids:?}",
            ids.len()
        );
    }
    assert_eq!(
        listing.scenes.len(),
        3,
        "book 1 chapter 4 has a three-scene spine, listing shows {}",
        listing.scenes.len()
    );
}

/// Unresolved `generate_alternatives` output must still be visible to the
/// operator — but under its own field, where it can never be mistaken for
/// the spine or deleted by position.
#[tokio::test]
async fn list_chapter_scenes_reports_unresolved_alternatives_separately() {
    use spindle_core::models::ListChapterScenesInput;

    let (_tmp, svc, proj, spine_ids, alt_ids) = spine_chapter_with_unresolved_alternatives().await;

    let listing = svc
        .list_chapter_scenes(ListChapterScenesInput {
            project_id: proj.project_id.clone(),
            chapter_id: None,
            book_number: 1,
            chapter_number: 4,
        })
        .await
        .unwrap();

    let reported_alt_ids = listing
        .unresolved_alternatives
        .iter()
        .map(|alt| alt.scene_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        reported_alt_ids,
        alt_ids
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        "every unselected alternative in the chapter must be reported"
    );
    for alt in &listing.unresolved_alternatives {
        assert_eq!(alt.scene_order, 2);
        assert!(
            alt.branch_name.starts_with("alt-approach-1-4-"),
            "alternative entry must name its branch, got {:?}",
            alt.branch_name
        );
        assert!(
            !spine_ids.contains(&alt.scene_id),
            "an alternative may never be reported as a spine scene"
        );
    }
}

#[tokio::test]
async fn delete_scene_resolves_downstream_validator_findings() {
    use crate::sqlite::repository::UpsertValidatorFindingParams;
    use spindle_core::models::{ContentRating, DeleteSceneInput, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "del".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "a".into(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();
    let scene_b = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "b".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let repo = svc.repository();
    let branch = repo.get_active_branch(&proj.project_id).await.unwrap();

    // An open finding on the downstream scene B.
    repo.upsert_validator_finding(UpsertValidatorFindingParams {
        project_id: proj.project_id.clone(),
        branch_id: branch.id.clone(),
        scene_id: scene_b.scene_id.clone(),
        scene_text_hash: "hashb".into(),
        context_hash: None,
        validator_id: "canonical_fact_prose_drift".into(),
        finding_id: "fb".into(),
        severity: "error".into(),
        message: "downstream finding".into(),
        byte_range: None,
        details_json: None,
    })
    .await
    .unwrap();

    // Deleting the scene before B shifts B's "scenes before" set.
    svc.delete_scene(DeleteSceneInput {
        project_id: proj.project_id.clone(),
        scene_id: None,
        book_number: 1,
        chapter_number: 1,
        scene_order: 1,
    })
    .await
    .unwrap();

    let after = repo
        .list_validator_findings_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap();
    let b_findings: Vec<_> = after
        .iter()
        .filter(|f| f.scene_id == scene_b.scene_id)
        .collect();
    assert!(!b_findings.is_empty(), "downstream finding still tracked");
    assert!(
        b_findings.iter().all(|f| f.resolved_at.is_some()),
        "downstream scene's stale findings must be resolved: {b_findings:?}"
    );
}

#[tokio::test]
async fn move_scene_resolves_stale_validator_findings() {
    use crate::sqlite::repository::UpsertValidatorFindingParams;
    use spindle_core::models::{ContentRating, MoveSceneInput, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "mv".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "x".into(),
            summary: "s".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();
    let repo = svc.repository();
    let branch = repo.get_active_branch(&proj.project_id).await.unwrap();

    // Cache an open, placement-dependent finding on the scene.
    repo.upsert_validator_finding(UpsertValidatorFindingParams {
        project_id: proj.project_id.clone(),
        branch_id: branch.id.clone(),
        scene_id: scene.scene_id.clone(),
        scene_text_hash: "hash1".into(),
        context_hash: None,
        validator_id: "canonical_fact_prose_drift".into(),
        finding_id: "f1".into(),
        severity: "error".into(),
        message: "stale finding".into(),
        byte_range: None,
        details_json: None,
    })
    .await
    .unwrap();
    let before = repo
        .list_validator_findings_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap();
    assert!(
        before
            .iter()
            .any(|f| f.scene_id == scene.scene_id && f.resolved_at.is_none()),
        "precondition: an open finding exists for the scene"
    );

    // Moving the scene changes its placement, so the cached finding is stale.
    svc.move_scene(MoveSceneInput {
        project_id: proj.project_id.clone(),
        scene_id: None,
        from_book_number: 1,
        from_chapter_number: 1,
        from_scene_order: 1,
        to_book_number: 1,
        to_chapter_number: 2,
        to_scene_order: 1,
    })
    .await
    .unwrap();

    let after = repo
        .list_validator_findings_by_project_and_branch(&proj.project_id, &branch.id)
        .await
        .unwrap();
    let scene_findings: Vec<_> = after
        .iter()
        .filter(|f| f.scene_id == scene.scene_id)
        .collect();
    assert!(!scene_findings.is_empty(), "finding still tracked");
    assert!(
        scene_findings.iter().all(|f| f.resolved_at.is_some()),
        "moved scene's stale findings must be resolved: {scene_findings:?}"
    );
}

/// Live-run bug: when duplicates or orphans appear there must be a SAFE
/// remediation path. Position-addressed delete cannot disambiguate rows
/// sharing a position and silently acts on whichever row the resolver
/// picks first. A `scene_id`-addressed delete removes exactly the named
/// row.
#[tokio::test]
async fn delete_scene_by_id_removes_the_exact_scene() {
    use spindle_core::models::DeleteSceneInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "IdDelete".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let (scene_one, scene_two) = two_scene_chapter(&svc, &proj.project_id).await;

    let out = svc
        .delete_scene(DeleteSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(scene_one.clone()),
            ..Default::default()
        })
        .await
        .expect("scene_id-addressed delete must succeed without position fields");
    assert_eq!(out.scene_id, scene_one);

    let branch_id = svc
        .repository
        .active_branch_id_public(&proj.project_id)
        .await
        .unwrap();
    assert!(
        svc.repository
            .find_scene_by_natural_key(&proj.project_id, &branch_id, 1, 1, 1)
            .await
            .unwrap()
            .is_none(),
        "the deleted scene's position must be empty"
    );
    let survivor = svc
        .repository
        .find_scene_by_natural_key(&proj.project_id, &branch_id, 1, 1, 2)
        .await
        .unwrap()
        .expect("the sibling scene must survive");
    assert_eq!(survivor.id, scene_two);
}

/// When both `scene_id` and position fields are supplied they must agree;
/// a mismatch is a caller bug and must be rejected loudly rather than
/// silently acting on the id.
#[tokio::test]
async fn delete_scene_by_id_rejects_mismatched_position_fields() {
    use spindle_core::models::DeleteSceneInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "IdMismatch".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let (scene_one, _scene_two) = two_scene_chapter(&svc, &proj.project_id).await;

    let err = svc
        .delete_scene(DeleteSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(scene_one.clone()),
            book_number: 1,
            chapter_number: 1,
            scene_order: 2,
        })
        .await
        .expect_err("position fields contradicting the scene_id must be rejected");
    assert!(
        err.to_string().contains("does not match"),
        "expected a mismatch error, got: {err}"
    );
    assert!(
        svc.repository.get_scene(&scene_one).await.is_ok(),
        "the rejected delete must not touch the row"
    );
}

/// `operator_delete_scene` shares the scene_id path.
#[tokio::test]
async fn operator_delete_scene_by_id_removes_the_exact_scene() {
    use spindle_core::models::OperatorDeleteSceneInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "OpIdDelete".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let (scene_one, _scene_two) = two_scene_chapter(&svc, &proj.project_id).await;

    let out = svc
        .operator_delete_scene(OperatorDeleteSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(scene_one.clone()),
            ..Default::default()
        })
        .await
        .expect("operator delete must accept scene_id");
    assert_eq!(out.scene_id, scene_one);
}

/// scene_id-addressed move relocates the exact named row and reports the
/// resolved source placement.
#[tokio::test]
async fn move_scene_by_id_relocates_the_exact_scene() {
    use spindle_core::models::MoveSceneInput;

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "IdMove".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();
    let (scene_one, _scene_two) = two_scene_chapter(&svc, &proj.project_id).await;

    let out = svc
        .move_scene(MoveSceneInput {
            project_id: proj.project_id.clone(),
            scene_id: Some(scene_one.clone()),
            to_book_number: 1,
            to_chapter_number: 3,
            to_scene_order: 1,
            ..Default::default()
        })
        .await
        .expect("scene_id-addressed move must succeed without from_* fields");
    assert_eq!(out.scene_id, scene_one);
    assert_eq!(out.from_book_number, 1);
    assert_eq!(out.from_chapter_number, 1);
    assert_eq!(out.from_scene_order, 1);
    assert_eq!(out.to_chapter_number, 3);

    let branch_id = svc
        .repository
        .active_branch_id_public(&proj.project_id)
        .await
        .unwrap();
    let moved = svc
        .repository
        .find_scene_by_natural_key(&proj.project_id, &branch_id, 1, 3, 1)
        .await
        .unwrap()
        .expect("the scene must land at the destination");
    assert_eq!(moved.id, scene_one);
}

/// One-scene project + one chapter_summary attached to that chapter.
/// `get_scene_delete_impact` should classify the chapter_summary as a
/// chapter_artifact, leave hard_blockers empty, and resolve
/// `delete_readiness` to `needs_followup`. The reported scene fields
/// must point at the scene we just inserted.
#[tokio::test]
async fn get_scene_delete_impact_reports_chapter_artifacts() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput, SaveSummaryInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "delete-impact".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let scene = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    svc.save_summary(SaveSummaryInput {
        project_id: proj.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        entity_type: None,
        entity_id: None,
        summary: "ch1 close".into(),
        key_events: Vec::new(),
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
    })
    .await
    .unwrap();

    let impact = svc
        .get_scene_delete_impact(GetSceneDeleteImpactInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            scene_order: 1,
        })
        .await
        .unwrap();

    assert_eq!(impact.scene.scene_id, scene.scene_id);
    assert_eq!(impact.scene.book_number, 1);
    assert_eq!(impact.scene.chapter_number, 1);
    assert_eq!(impact.scene.scene_order, 1);
    assert!(impact.hard_blockers.is_empty(), "no hard blockers expected");
    assert_eq!(
        impact.delete_readiness,
        SceneDeleteReadiness::NeedsFollowup,
        "chapter_summary artifact must demote readiness from Clear"
    );

    // The chapter_summary artifact group must be present and reference
    // the freshly-saved summary.
    let summaries: Vec<_> = impact
        .chapter_artifacts
        .iter()
        .filter(|g| g.dependency_type == "chapter_summary")
        .collect();
    assert_eq!(summaries.len(), 1, "exactly one chapter_summary group");
    assert_eq!(summaries[0].count, 1);
    assert_eq!(summaries[0].sample_record_ids.len(), 1);
    assert!(
        summaries[0].sample_record_ids[0].starts_with("chapter_summary:"),
        "got {:?}",
        summaries[0].sample_record_ids
    );

    // The JSON projection should round-trip cleanly (this is what
    // `read_project_resource` will return downstream).
    let json = serde_json::to_value(&impact).unwrap();
    assert_eq!(json["delete_readiness"], "needs_followup");
    assert_eq!(json["scene"]["scene_id"], impact.scene.scene_id);
}

/// Two-scene project where both scenes live in book 1 chapter 1.
/// Asking `get_scene_move_impact` to move scene 1 onto scene 2's
/// position must flag a `destination_scene` hard blocker pointing
/// at the second scene id and resolve `move_readiness` to
/// `blocked`. The reported `scene` (source) and `destination`
/// fields must match the natural-key inputs.
#[tokio::test]
async fn get_scene_move_impact_flags_destination_conflict() {
    use spindle_core::models::{ContentRating, SaveSceneDraftInput};

    let (_tmp, svc) = fresh_service().await;
    let proj = svc
        .create_project(CreateProjectInput {
            name: "move-impact".into(),
            project_type: "novel".into(),
            genre: "fantasy".into(),
            reader_contract: ReaderContract {
                promise: "p".into(),
                style_notes: Vec::new(),
                boundaries: Vec::new(),
            },
        })
        .await
        .unwrap();

    let source = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 1,
            full_text: "Mara stood at the Ash Gate.".into(),
            summary: "stage".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let destination = svc
        .save_scene_draft(SaveSceneDraftInput {
            project_id: proj.project_id.clone(),
            book_number: 1,
            chapter_number: 1,
            chapter_id: None,
            scene_order: 2,
            full_text: "She crossed the threshold.".into(),
            summary: "cross".into(),
            content_rating: ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

    let impact = svc
        .get_scene_move_impact(GetSceneMoveImpactInput {
            project_id: proj.project_id.clone(),
            from_book_number: 1,
            from_chapter_number: 1,
            from_scene_order: 1,
            to_book_number: 1,
            to_chapter_number: 1,
            to_scene_order: 2,
        })
        .await
        .unwrap();

    assert_eq!(impact.scene.scene_id, source.scene_id);
    assert_eq!(impact.destination.book_number, 1);
    assert_eq!(impact.destination.chapter_number, 1);
    assert_eq!(impact.destination.scene_order, 2);
    assert_eq!(
        impact.destination.existing_scene_id.as_deref(),
        Some(destination.scene_id.as_str())
    );
    assert_eq!(
        impact.move_readiness,
        SceneMoveReadiness::Blocked,
        "destination occupancy must Block the move"
    );

    // Find the destination_scene hard blocker group and confirm it
    // points at the second scene.
    let dest_blockers: Vec<_> = impact
        .hard_blockers
        .iter()
        .filter(|g| g.dependency_type == "destination_scene")
        .collect();
    assert_eq!(
        dest_blockers.len(),
        1,
        "exactly one destination_scene group"
    );
    assert_eq!(dest_blockers[0].count, 1);
    assert_eq!(
        dest_blockers[0].sample_record_ids,
        vec![destination.scene_id.clone()]
    );

    // The JSON projection should round-trip cleanly.
    let json = serde_json::to_value(&impact).unwrap();
    assert_eq!(json["move_readiness"], "blocked");
    assert_eq!(json["scene"]["scene_id"], impact.scene.scene_id);
    assert_eq!(
        json["destination"]["existing_scene_id"],
        destination.scene_id
    );
}
