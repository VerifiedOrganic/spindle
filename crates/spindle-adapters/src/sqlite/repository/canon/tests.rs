use super::*;

#[tokio::test]
async fn stage_canon_delta_round_trips() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    let staged = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "relationship_shift",
            "She turned away without a word.",
        ))
        .await
        .unwrap();
    assert!(staged.id.starts_with("canon_delta:"));
    assert_eq!(staged.delta_class, "relationship_shift");
    assert_eq!(staged.status, "staged");
    assert!(staged.decided_at.is_none());
    assert!(staged.decided_by.is_none());

    let listed = repo
        .list_canon_deltas(&project.id, &branch.id, None, None)
        .await
        .unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, staged.id);
    assert_eq!(listed[0].evidence, "She turned away without a word.");
}

#[tokio::test]
async fn stage_canon_delta_rejects_unknown_class() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    let err = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "not_a_real_class",
            "evidence quote",
        ))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("not_a_real_class"),
        "error names the bad class: {err}"
    );
}

#[tokio::test]
async fn stage_canon_delta_rejects_empty_and_oversized_evidence() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;

    // Empty evidence rejected (ADR D2: not stageable without a quote).
    let err = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "beat_annotation",
            "   ",
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("evidence"));

    // Exactly 300 chars is allowed; 301 is not — using a multibyte char so
    // the boundary is measured in chars, not bytes.
    let ok_300 = "é".repeat(300);
    let staged = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "beat_annotation",
            &ok_300,
        ))
        .await
        .unwrap();
    assert_eq!(staged.evidence.chars().count(), 300);

    let too_long = "é".repeat(301);
    let err = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "beat_annotation",
            &too_long,
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("evidence"));
}

#[tokio::test]
async fn stage_canon_delta_rejects_bad_confidence() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    let mut params = stage_params(&project, &branch, &scene_id, "beat_annotation", "quote");
    params.confidence = "certain".to_string();
    let err = repo.stage_canon_delta(params).await.unwrap_err();
    assert!(err.to_string().to_lowercase().contains("confidence"));
}

#[tokio::test]
async fn list_canon_deltas_filters_and_orders_deterministically() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    // Stage three deltas; created_at then id is the deterministic order.
    for class in ["beat_annotation", "quantity_change", "character_state"] {
        repo.stage_canon_delta(stage_params(&project, &branch, &scene_id, class, "quote"))
            .await
            .unwrap();
    }
    let all = repo
        .list_canon_deltas(&project.id, &branch.id, None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    // Deterministic order: sort a clone by (created_at, id) and compare ids.
    let mut expected = all.clone();
    expected.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let got_ids: Vec<_> = all.iter().map(|d| &d.id).collect();
    let want_ids: Vec<_> = expected.iter().map(|d| &d.id).collect();
    assert_eq!(got_ids, want_ids);

    // Status filter: decide one, then filter by status.
    let first = all[0].id.clone();
    repo.decide_canon_delta(&first, CanonDeltaDecision::Applied, "op", None)
        .await
        .unwrap();
    let staged_only = repo
        .list_canon_deltas(&project.id, &branch.id, Some("staged"), None)
        .await
        .unwrap();
    assert_eq!(staged_only.len(), 2);
    let applied_only = repo
        .list_canon_deltas(&project.id, &branch.id, Some("applied"), None)
        .await
        .unwrap();
    assert_eq!(applied_only.len(), 1);
    assert_eq!(applied_only[0].id, first);

    // scene_id filter narrows to the provenance scene.
    let by_scene = repo
        .list_canon_deltas(&project.id, &branch.id, None, Some(&scene_id))
        .await
        .unwrap();
    assert_eq!(by_scene.len(), 3);
    let other_scene = repo
        .list_canon_deltas(&project.id, &branch.id, None, Some("scene:none"))
        .await
        .unwrap();
    assert!(other_scene.is_empty());
}

#[tokio::test]
async fn decide_canon_delta_records_decision_and_is_final() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    let staged = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "beat_annotation",
            "quote",
        ))
        .await
        .unwrap();

    let decided = repo
        .decide_canon_delta(&staged.id, CanonDeltaDecision::Applied, "operator-1", None)
        .await
        .unwrap();
    assert_eq!(decided.status, "applied");
    assert!(decided.decided_at.is_some());
    assert_eq!(decided.decided_by.as_deref(), Some("operator-1"));
    // Payload unchanged when no edit supplied.
    assert_eq!(decided.payload, serde_json::json!({ "note": "x" }));

    // Deciding a second time errors — decisions are final (ADR D3).
    let err = repo
        .decide_canon_delta(&staged.id, CanonDeltaDecision::Rejected, "op2", None)
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("staged"));
}

#[tokio::test]
async fn decide_canon_delta_stores_edited_payload() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    let staged = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "canonical_fact",
            "quote",
        ))
        .await
        .unwrap();
    let edited = serde_json::json!({ "note": "operator corrected" });
    let decided = repo
        .decide_canon_delta(
            &staged.id,
            CanonDeltaDecision::Applied,
            "op",
            Some(edited.clone()),
        )
        .await
        .unwrap();
    assert_eq!(decided.payload, edited);
}

#[tokio::test]
async fn decide_canon_delta_on_superseded_errors() {
    let (_tmp, repo, project, branch, scene_id) = repo_with_scene().await;
    let staged = repo
        .stage_canon_delta(stage_params(
            &project,
            &branch,
            &scene_id,
            "beat_annotation",
            "quote",
        ))
        .await
        .unwrap();
    repo.supersede_scene_deltas(&scene_id).await.unwrap();
    let err = repo
        .decide_canon_delta(&staged.id, CanonDeltaDecision::Applied, "op", None)
        .await
        .unwrap_err();
    assert!(err.to_string().to_lowercase().contains("staged"));
}
