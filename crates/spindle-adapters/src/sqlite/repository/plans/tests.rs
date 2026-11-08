use super::*;

#[tokio::test]
async fn stage_plan_amendment_round_trips() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    let staged = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "synopsis_update",
            Some(5),
            "chapter 3 resolved the siege early",
        ))
        .await
        .unwrap();
    assert!(staged.id.starts_with("plan_amendment:"));
    assert_eq!(staged.status, "staged");
    assert_eq!(staged.source_chapter, 3);
    assert_eq!(staged.target_chapter, Some(5));
    assert!(staged.decided_at.is_none());
    assert!(staged.prior_state.is_none());

    let all = repo
        .list_plan_amendments(&project.id, &branch.id, None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, staged.id);
}

#[tokio::test]
async fn stage_plan_amendment_rejects_unknown_class() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    let err = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "totally_made_up",
            Some(5),
            "reason",
        ))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown plan amendment class"));
}

#[tokio::test]
async fn stage_plan_amendment_rejects_empty_and_oversized_rationale() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    // Empty (whitespace-only) rationale is rejected.
    let empty = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "synopsis_update",
            Some(5),
            "   ",
        ))
        .await
        .unwrap_err();
    assert!(empty.to_string().contains("rationale is mandatory"));

    // Exactly 500 multibyte chars is accepted; 501 is rejected — the check
    // must count chars, not bytes (each `é` is 2 bytes).
    let ok_rationale: String = "é".repeat(500);
    let ok = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "synopsis_update",
            Some(5),
            &ok_rationale,
        ))
        .await;
    assert!(ok.is_ok(), "500 chars is the boundary and must pass");

    let over_rationale: String = "é".repeat(501);
    let over = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "synopsis_update",
            Some(5),
            &over_rationale,
        ))
        .await
        .unwrap_err();
    assert!(over.to_string().contains("≤500 chars"));
}

#[tokio::test]
async fn stage_plan_amendment_rejects_bad_confidence() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    let mut params = plan_stage_params(&project, &branch, "synopsis_update", Some(5), "reason");
    params.confidence = "certain".to_string();
    let err = repo.stage_plan_amendment(params).await.unwrap_err();
    assert!(err.to_string().contains("confidence must be one of"));
}

#[tokio::test]
async fn stage_plan_amendment_enforces_target_chapter_rules_per_class() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;

    // Every chapter-targeting class requires a target_chapter.
    for class in [
        "synopsis_update",
        "scene_add",
        "scene_drop",
        "scene_replace",
        "scene_reorder",
        "thread_promote",
        "thread_retire",
    ] {
        let err = repo
            .stage_plan_amendment(plan_stage_params(&project, &branch, class, None, "reason"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("requires a target_chapter"),
            "{class} without target must be rejected"
        );
        // And with a target it stages.
        let ok = repo
            .stage_plan_amendment(plan_stage_params(
                &project,
                &branch,
                class,
                Some(5),
                "reason",
            ))
            .await;
        assert!(ok.is_ok(), "{class} with target must stage");
    }

    // promise_followup must NOT carry a target_chapter.
    let followup_bad = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "promise_followup",
            Some(5),
            "reason",
        ))
        .await
        .unwrap_err();
    assert!(
        followup_bad
            .to_string()
            .contains("must NOT carry a target_chapter")
    );

    // promise_followup with no target stages.
    let followup_ok = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "promise_followup",
            None,
            "reason",
        ))
        .await;
    assert!(
        followup_ok.is_ok(),
        "promise_followup without target must stage"
    );
}

#[tokio::test]
async fn list_plan_amendments_filters_and_orders_deterministically() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    // Stage three (order by created_at, id — insertion order stable).
    for class in ["synopsis_update", "scene_drop", "thread_retire"] {
        repo.stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            class,
            Some(5),
            "reason",
        ))
        .await
        .unwrap();
    }
    let all = repo
        .list_plan_amendments(&project.id, &branch.id, None, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);
    // Deterministic (created_at, id): non-decreasing created_at.
    for w in all.windows(2) {
        assert!(w[0].created_at <= w[1].created_at);
    }

    // Decide one → status filter narrows.
    let first = all[0].id.clone();
    repo.decide_plan_amendment(&first, PlanAmendmentDecision::Applied, "op", None)
        .await
        .unwrap();
    let staged = repo
        .list_plan_amendments(&project.id, &branch.id, Some("staged"), None)
        .await
        .unwrap();
    assert_eq!(staged.len(), 2);
    let applied = repo
        .list_plan_amendments(&project.id, &branch.id, Some("applied"), None)
        .await
        .unwrap();
    assert_eq!(applied.len(), 1);

    // Book+source-chapter filter: matching pair returns all three, a
    // different chapter returns none.
    let scoped = repo
        .list_plan_amendments(&project.id, &branch.id, None, Some((1, 3)))
        .await
        .unwrap();
    assert_eq!(scoped.len(), 3);
    let other = repo
        .list_plan_amendments(&project.id, &branch.id, None, Some((1, 9)))
        .await
        .unwrap();
    assert!(other.is_empty());
}

#[tokio::test]
async fn decide_plan_amendment_records_decision_prior_state_and_is_final() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    let staged = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "synopsis_update",
            Some(5),
            "reason",
        ))
        .await
        .unwrap();
    let prior = r#"{"synopsis":"old"}"#.to_string();
    let applied = repo
        .decide_plan_amendment(
            &staged.id,
            PlanAmendmentDecision::Applied,
            "operator-1",
            Some(prior.clone()),
        )
        .await
        .unwrap();
    assert_eq!(applied.status, "applied");
    assert_eq!(applied.decided_by.as_deref(), Some("operator-1"));
    assert!(applied.decided_at.is_some());
    assert_eq!(applied.prior_state.as_deref(), Some(prior.as_str()));

    // A second decision on the same (now-applied) row errors — decisions are
    // final history.
    let again = repo
        .decide_plan_amendment(&staged.id, PlanAmendmentDecision::Rejected, "op2", None)
        .await
        .unwrap_err();
    assert!(again.to_string().contains("not staged"));
}

#[tokio::test]
async fn decide_plan_amendment_on_superseded_errors() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    let staged = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "synopsis_update",
            Some(5),
            "reason",
        ))
        .await
        .unwrap();
    repo.supersede_source_chapter_amendments(&project.id, &branch.id, 1, 3)
        .await
        .unwrap();
    let err = repo
        .decide_plan_amendment(&staged.id, PlanAmendmentDecision::Applied, "op", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not staged"));
}

#[tokio::test]
async fn supersede_source_chapter_amendments_only_flips_staged_rows() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    let staged = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "synopsis_update",
            Some(5),
            "will be superseded",
        ))
        .await
        .unwrap();
    let applied = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "scene_drop",
            Some(5),
            "already applied",
        ))
        .await
        .unwrap();
    let rejected = repo
        .stage_plan_amendment(plan_stage_params(
            &project,
            &branch,
            "thread_retire",
            Some(5),
            "already rejected",
        ))
        .await
        .unwrap();
    repo.decide_plan_amendment(&applied.id, PlanAmendmentDecision::Applied, "op", None)
        .await
        .unwrap();
    repo.decide_plan_amendment(&rejected.id, PlanAmendmentDecision::Rejected, "op", None)
        .await
        .unwrap();

    let flipped = repo
        .supersede_source_chapter_amendments(&project.id, &branch.id, 1, 3)
        .await
        .unwrap();
    assert_eq!(flipped, 1, "only the one staged row is superseded");

    let by_id = |rows: &[crate::sqlite::records::StoredPlanAmendment], id: &str| {
        rows.iter().find(|r| r.id == id).unwrap().status.clone()
    };
    let all = repo
        .list_plan_amendments(&project.id, &branch.id, None, None)
        .await
        .unwrap();
    assert_eq!(by_id(&all, &staged.id), "superseded");
    assert_eq!(by_id(&all, &applied.id), "applied");
    assert_eq!(by_id(&all, &rejected.id), "rejected");
}

#[tokio::test]
async fn supersede_source_chapter_amendments_spares_other_source_chapters() {
    let (_tmp, repo, project, branch) = repo_with_project_branch().await;
    // Two staged amendments from different source chapters.
    let mut p3 = plan_stage_params(&project, &branch, "synopsis_update", Some(6), "from ch3");
    p3.source_chapter = 3;
    let a3 = repo.stage_plan_amendment(p3).await.unwrap();
    let mut p4 = plan_stage_params(&project, &branch, "synopsis_update", Some(6), "from ch4");
    p4.source_chapter = 4;
    let a4 = repo.stage_plan_amendment(p4).await.unwrap();

    let flipped = repo
        .supersede_source_chapter_amendments(&project.id, &branch.id, 1, 3)
        .await
        .unwrap();
    assert_eq!(flipped, 1);

    let all = repo
        .list_plan_amendments(&project.id, &branch.id, None, None)
        .await
        .unwrap();
    let status = |id: &str| all.iter().find(|r| r.id == id).unwrap().status.clone();
    assert_eq!(status(&a3.id), "superseded");
    assert_eq!(
        status(&a4.id),
        "staged",
        "chapter 4's amendment is untouched"
    );
}
