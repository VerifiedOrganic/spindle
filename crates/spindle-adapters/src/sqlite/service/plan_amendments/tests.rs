use super::*;

// ── Replan differ (replan_chapter — evolution §3.5, ADR 0003) ────────────

/// Per-class payload validation units (ADR 0003 D1): one valid + one invalid
/// payload per class. This is the Part B contract — the payload shapes are
/// pinned here so a later apply dispatcher can rely on them.
#[test]
fn plan_amendment_payload_validation_per_class() {
    use serde_json::json;
    // (class, valid payload, invalid payload)
    let cases: [(&str, serde_json::Value, serde_json::Value); 8] = [
        (
            "synopsis_update",
            json!({"synopsis": "The gate has already fallen."}),
            json!({"synopsis": "   "}), // empty
        ),
        (
            "scene_add",
            json!({"scene_order": 2, "summary": "s", "purpose": "p", "character_ids": [], "insert_at": 1}),
            json!({"scene_order": 2, "summary": "s", "purpose": "p"}), // missing character_ids + insert_at
        ),
        (
            "scene_drop",
            json!({"scene_order": 3}),
            json!({"scene_order": "not-a-number"}),
        ),
        (
            "scene_replace",
            json!({"scene_order": 1, "summary": "s", "purpose": "p", "character_ids": ["character:x"]}),
            json!({"scene_order": 1, "purpose": "p", "character_ids": []}), // missing summary
        ),
        (
            "scene_reorder",
            json!({"new_order": [3, 1, 2]}),
            json!({"new_order": []}), // empty permutation
        ),
        (
            "thread_promote",
            json!({"kind": "theme", "id": "theme:x"}),
            json!({"kind": "invalid", "id": "theme:x"}), // bad kind
        ),
        (
            "thread_retire",
            json!({"kind": "conflict", "id": "conflict:x"}),
            json!({"kind": "conflict", "id": ""}), // empty id
        ),
        (
            "promise_followup",
            json!({"promise_type": "mystery", "description": "d", "planted_at_placement": {"book_number": 1, "chapter_number": 6}}),
            json!({"promise_type": "mystery", "description": "d"}), // missing placement
        ),
    ];
    for (class, valid, invalid) in cases {
        assert!(
            plan_amendment_payload_is_valid(class, &valid),
            "{class} valid payload must pass"
        );
        assert!(
            !plan_amendment_payload_is_valid(class, &invalid),
            "{class} invalid payload must be rejected"
        );
    }
    // An unknown class never validates regardless of payload.
    assert!(!plan_amendment_payload_is_valid(
        "made_up",
        &serde_json::json!({})
    ));
}

#[test]
fn replan_prompt_renders_realized_and_target_sections_deterministically() {
    // Pure render: the realized digest surfaces the source chapter's summary
    // + list fields + ids; the target section names only the future chapter
    // number and its plan_revision. No I/O.
    let realized = ReplanRealizedDigest {
        source_chapter: 3,
        summary: "The gate fell early.".to_string(),
        key_events: vec!["gate breached".to_string()],
        character_changes: Vec::new(),
        relationship_shifts: Vec::new(),
        arc_advances: Vec::new(),
        promise_events: Vec::new(),
        beats: vec![ReplanBeatDigest {
            scene_order: 1,
            motif_ids: vec!["motif:ash".to_string()],
            theme_ids: Vec::new(),
            conflict_ids: vec!["conflict:siege".to_string()],
            intensity: Some(0.8),
        }],
        promise_states: vec![(
            "narrative_promise:door".to_string(),
            "planted".to_string(),
            "soon".to_string(),
        )],
        arc_states: vec![("character_arc:mara".to_string(), "active".to_string(), 0.5)],
    };
    let realized_rendered = build_replan_realized_section(&realized);
    assert!(realized_rendered.contains("REALIZED REALITY of chapter 3"));
    assert!(realized_rendered.contains("The gate fell early."));
    assert!(realized_rendered.contains("gate breached"));
    assert!(realized_rendered.contains("motif:ash"));
    assert!(realized_rendered.contains("conflict:siege"));
    assert!(realized_rendered.contains("narrative_promise:door"));
    assert!(realized_rendered.contains("character_arc:mara"));

    let targets = vec![ReplanTargetDigest {
        chapter_number: 5,
        synopsis: "Chapter five as planned.".to_string(),
        scenes: vec![ReplanTargetSceneDigest {
            scene_order: 1,
            summary: "opening".to_string(),
            purpose: "establish".to_string(),
            character_ids: vec!["character:x".to_string()],
            location_id: Some("location:hall".to_string()),
            content_rating: Some("general".to_string()),
        }],
        target_theme_ids: vec!["theme:loss".to_string()],
        target_conflict_ids: Vec::new(),
        target_plot_line_ids: Vec::new(),
        plan_revision: 0,
    }];
    let targets_rendered = build_replan_targets_section(&targets);
    assert!(targets_rendered.contains("Chapter 5 (plan_revision 0)"));
    assert!(targets_rendered.contains("Chapter five as planned."));
    assert!(targets_rendered.contains("theme:loss"));
    assert!(targets_rendered.contains("character:x"));
    assert!(targets_rendered.contains("location:hall"));
}

#[tokio::test]
async fn replan_chapter_stages_amendments_supersedes_on_rerun_and_counts_discards() {
    use spindle_core::models::ReplanChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    // Target chapter 5's synopsis carries the happy sentinel (chapter 5) and
    // the discard sentinel (MOCK_REPLAN_BAD) so the stub returns 2 valid + 2
    // discardable amendments in one pass.
    let project = project_with_target_plan(
        &svc,
        "Replan Happy",
        5,
        "MOCK_REPLAN_SYNOPSIS[5] MOCK_REPLAN_BAD chapter five synopsis",
    )
    .await;
    save_source_summary(&svc, &project.project_id, 3).await;

    let out = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(out.status, "staged");
    assert_eq!(out.superseded_count, 0);
    assert_eq!(out.staged.len(), 2, "synopsis_update + thread_retire stage");
    assert_eq!(
        out.discarded_count, 2,
        "unknown class + empty rationale discarded"
    );
    // Every staged amendment carries a non-empty rationale.
    assert!(out.staged.iter().all(|a| !a.rationale.trim().is_empty()));
    let classes: Vec<&str> = out
        .staged
        .iter()
        .map(|a| a.amendment_class.as_str())
        .collect();
    assert!(classes.contains(&"synopsis_update"));
    assert!(classes.contains(&"thread_retire"));
    assert!(out.staged.iter().all(|a| a.target_chapter == Some(5)));

    // Rerun supersedes the prior staged rows for the same source chapter.
    let rerun = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(rerun.status, "staged");
    assert_eq!(
        rerun.superseded_count, 2,
        "the prior pass's 2 staged rows superseded"
    );
    assert_eq!(rerun.staged.len(), 2);
}

#[tokio::test]
async fn replan_chapter_skips_when_source_has_no_summary() {
    use spindle_core::models::ReplanChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    let project =
        project_with_target_plan(&svc, "Replan NoSummary", 5, "MOCK_REPLAN_SYNOPSIS[5] x").await;
    // No summary saved for chapter 3.
    let out = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(out.status, "no_summary");
    assert!(out.staged.is_empty());
    assert_eq!(out.superseded_count, 0);
}

#[tokio::test]
async fn replan_chapter_skips_when_no_eligible_targets() {
    use spindle_core::models::ReplanChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    // Only a plan for chapter 2 (BEFORE the source chapter 3) — no target
    // strictly after chapter 3.
    let project = project_with_target_plan(&svc, "Replan NoTargets", 2, "earlier chapter").await;
    save_source_summary(&svc, &project.project_id, 3).await;
    let out = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(out.status, "no_targets");
    assert!(out.staged.is_empty());
}

#[tokio::test]
async fn replan_chapter_excludes_drafted_target_at_staging() {
    use spindle_core::models::ReplanChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    // Plan chapter 5 (the only future chapter) but then draft a scene in it —
    // it must drop out of the eligible set, leaving no targets.
    let project = project_with_target_plan(
        &svc,
        "Replan Drafted",
        5,
        "MOCK_REPLAN_SYNOPSIS[5] chapter five",
    )
    .await;
    save_source_summary(&svc, &project.project_id, 3).await;
    svc.save_scene_draft(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 5,
        chapter_id: None,
        scene_order: 1,
        full_text: "chapter five already has drafted prose".to_string(),
        summary: "drafted".to_string(),
        content_rating: spindle_core::models::ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        ..Default::default()
    })
    .await
    .unwrap();

    let out = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(
        out.status, "no_targets",
        "a drafted target is excluded at staging (ADR D3 immutability sanity)"
    );
    assert!(out.staged.is_empty());
}

#[tokio::test]
async fn replan_chapter_dispatch_falls_to_review_route() {
    // `replan` is absent from default_routes; the ladder must fall to
    // `review`. We prove it via the dispatch recorder: the dispatch went to
    // the `review` route and never to a `replan` route.
    use spindle_core::models::ReplanChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    let log = svc
        .repository
        .model_router()
        .install_dispatch_recorder()
        .unwrap();

    let project = project_with_target_plan(
        &svc,
        "Replan Ladder",
        5,
        "MOCK_REPLAN_SYNOPSIS[5] chapter five",
    )
    .await;
    save_source_summary(&svc, &project.project_id, 3).await;
    let out = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(out.status, "staged");

    let records = log.lock().unwrap().clone();
    let dispatched_routes: Vec<String> = records
        .iter()
        .filter_map(|r| match r {
            crate::ai::DispatchRecord::Dispatch { route, .. } => Some(route.clone()),
            _ => None,
        })
        .collect();
    assert!(
        dispatched_routes.iter().any(|r| r == "review"),
        "the replan dispatch fell to the review route: {dispatched_routes:?}"
    );
    assert!(
        !dispatched_routes.iter().any(|r| r == "replan"),
        "replan is not a real route and never dispatches: {dispatched_routes:?}"
    );
}

#[tokio::test]
async fn replan_chapter_skips_on_transport_error() {
    // Config a review agent whose endpoint fails so the ladder's review leg
    // errors → honest `skipped` status (never an error return, never blocks).
    use spindle_core::models::{ConfigureAgentsInput, ReplanChapterInput};
    let (tmp, svc) = fresh_service_local().await;
    let config_path = tmp.path().join("replan-routes.toml");
    std::fs::write(
        &config_path,
        r#"
[health_check]
enabled = false

[[agents]]
id = "review-dead"
name = "Review Dead"
provider = "openai_compatible"
endpoint = "http://127.0.0.1:1/unreachable"
model = "dead"
ratings = ["general", "teen", "mature", "explicit"]

[[routing]]
route = "review"
agent = "review-dead"
"#,
    )
    .unwrap();
    svc.configure_agents(ConfigureAgentsInput {
        config_path: Some(config_path.display().to_string()),
    })
    .unwrap();

    let project = project_with_target_plan(
        &svc,
        "Replan Dead",
        5,
        "MOCK_REPLAN_SYNOPSIS[5] chapter five",
    )
    .await;
    save_source_summary(&svc, &project.project_id, 3).await;
    let out = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(out.status, "skipped");
    assert!(out.staged.is_empty());
    let reason = out.skip_reason.expect("skip reason present");
    assert!(
        reason.contains("replan SKIPPED"),
        "honest skip reason: {reason}"
    );
}

#[tokio::test]
async fn replan_chapter_caps_staged_at_eight_and_reports_dropped() {
    use spindle_core::models::ReplanChapterInput;
    let (_tmp, svc) = fresh_service_local().await;
    // The cap sentinel makes the stub return 10 valid synopsis_update
    // amendments for chapter 5; the differ stages 8 and drops 2.
    let project =
        project_with_target_plan(&svc, "Replan Cap", 5, "MOCK_REPLAN_CAP[5] chapter five").await;
    save_source_summary(&svc, &project.project_id, 3).await;
    let out = svc
        .replan_chapter(ReplanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            source_chapter: 3,
        })
        .await
        .unwrap();
    assert_eq!(out.status, "staged");
    assert_eq!(out.staged.len(), 8, "per-pass cap");
    assert_eq!(
        out.discarded_count, 2,
        "overflow beyond the cap is dropped + counted"
    );
}

// ── Plan-amendment ratification (list_plan_amendments +
//    decide_plan_amendments) — ADR 0003 D3/D4/D5. Exercises the apply
//    dispatcher: pre-flight-all, the immutability guard, per-class replay
//    through the plan write path, prior_state capture + plan_revision
//    increment, and the honesty guarantees (atomicity, mid-apply failure).
mod plan_ratify {
    use super::*;
    use crate::sqlite::repository::StagePlanAmendmentParams;
    use spindle_core::models::{
        DecidePlanAmendmentsInput, ListPlanAmendmentsInput, PlanAmendmentDecisionInput,
        PlanChapterInput, PlanChapterSceneInput,
    };

    /// Create a project, then plan a future chapter (`chapter_number`) with
    /// the given scenes, plus a source chapter summary so the outline has a
    /// stable base for amendments. Returns the project.
    async fn project_with_future_chapter(
        svc: &SqliteSpindleService,
        name: &str,
        chapter_number: i32,
        scenes: Vec<PlanChapterSceneInput>,
        target_theme_ids: Vec<String>,
    ) -> CreateProjectOutput {
        let project = svc
            .create_project(CreateProjectInput {
                name: name.to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "living outline".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        svc.plan_chapter(PlanChapterInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number,
            pov_character_id: None,
            synopsis: "the planned future".to_string(),
            target_theme_ids,
            target_conflict_ids: Vec::new(),
            target_plot_line_ids: Vec::new(),
            scenes,
        })
        .await
        .unwrap();
        project
    }

    fn scene(order: i32, summary: &str) -> PlanChapterSceneInput {
        PlanChapterSceneInput {
            scene_order: order,
            summary: summary.to_string(),
            beat_structure: Vec::new(),
            character_ids: Vec::new(),
            location_id: None,
            content_rating: None,
            purpose: format!("purpose {order}"),
            research_required: None,
            research_tags: Vec::new(),
            explicit_query: None,
        }
    }

    /// Stage one plan amendment directly on the active branch (bypassing the
    /// model) so the dispatcher can be driven deterministically per class.
    async fn stage(
        svc: &SqliteSpindleService,
        project_id: &str,
        class: &str,
        target_chapter: Option<i32>,
        payload: serde_json::Value,
    ) -> String {
        let branch = svc
            .repository()
            .get_active_branch(project_id)
            .await
            .unwrap();
        svc.repository()
            .stage_plan_amendment(StagePlanAmendmentParams {
                project_id: project_id.to_string(),
                branch_id: branch.id,
                source_chapter: 3,
                book_number: 1,
                authoring_run_id: None,
                amendment_class: class.to_string(),
                target_chapter,
                payload,
                rationale: format!("{class}: realized event id x moved the outline"),
                confidence: "high".to_string(),
            })
            .await
            .unwrap()
            .id
    }

    /// Read the current plan for a chapter (active branch) or panic.
    async fn plan_of(
        svc: &SqliteSpindleService,
        project_id: &str,
        chapter: i32,
    ) -> crate::sqlite::records::ChapterPlan {
        let plans = svc
            .repository()
            .list_chapter_plans_by_project(project_id)
            .await
            .unwrap();
        plans
            .into_iter()
            .find(|p| p.chapter_number == chapter)
            .expect("plan for chapter")
    }

    async fn apply_one(
        svc: &SqliteSpindleService,
        project_id: &str,
        amendment_id: &str,
    ) -> spindle_core::models::DecidePlanAmendmentsOutput {
        svc.decide_plan_amendments(DecidePlanAmendmentsInput {
            project_id: project_id.to_string(),
            decisions: vec![PlanAmendmentDecisionInput {
                amendment_id: amendment_id.to_string(),
                action: "apply".to_string(),
                edit: None,
                note: None,
            }],
            decided_by: None,
        })
        .await
        .unwrap()
    }

    // ── Class round-trips (Test 1) ───────────────────────────────────────

    #[tokio::test]
    async fn apply_synopsis_update_rewrites_synopsis_and_bumps_revision() {
        let (_tmp, svc) = fresh_service_local().await;
        let project =
            project_with_future_chapter(&svc, "PA synopsis", 5, vec![scene(1, "a")], Vec::new())
                .await;
        let before = plan_of(&svc, &project.project_id, 5).await;
        let id = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(5),
            serde_json::json!({"synopsis": "the gate has already fallen"}),
        )
        .await;

        let out = apply_one(&svc, &project.project_id, &id).await;
        assert_eq!(out.applied_count, 1);
        assert_eq!(out.results[0].outcome, "applied");

        let after = plan_of(&svc, &project.project_id, 5).await;
        assert_eq!(after.synopsis, "the gate has already fallen");
        // plan_revision NULL/0 → 1.
        assert_eq!(after.plan_revision, Some(1));
        // prior_state snapshot equals the pre-apply row.
        let amendments = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: Some("applied".to_string()),
                book_number: None,
                source_chapter: None,
            })
            .await
            .unwrap();
        let prior = amendments.amendments[0]
            .prior_state
            .as_deref()
            .expect("prior_state captured");
        let prior_plan: crate::sqlite::records::ChapterPlan = serde_json::from_str(prior).unwrap();
        assert_eq!(prior_plan.synopsis, before.synopsis);
        assert_eq!(prior_plan.plan_revision, before.plan_revision);
    }

    #[tokio::test]
    async fn apply_scene_add_inserts_and_renumbers() {
        let (_tmp, svc) = fresh_service_local().await;
        let project = project_with_future_chapter(
            &svc,
            "PA add",
            5,
            vec![scene(1, "first"), scene(2, "second")],
            Vec::new(),
        )
        .await;
        // Insert a new scene at position 2 (1-based insert_at), pushing the
        // old scene 2 to order 3.
        let id = stage(
            &svc,
            &project.project_id,
            "scene_add",
            Some(5),
            serde_json::json!({
                "scene_order": 99, "summary": "inserted", "purpose": "bridge",
                "character_ids": [], "insert_at": 2
            }),
        )
        .await;
        let out = apply_one(&svc, &project.project_id, &id).await;
        assert_eq!(out.results[0].outcome, "applied", "{out:?}");

        let after = plan_of(&svc, &project.project_id, 5).await;
        let summaries: Vec<(i32, String)> = after
            .scenes
            .iter()
            .map(|s| (s.scene_order, s.summary.clone()))
            .collect();
        assert_eq!(
            summaries,
            vec![
                (1, "first".to_string()),
                (2, "inserted".to_string()),
                (3, "second".to_string()),
            ],
            "insert_at renumbers the spine 1..n"
        );
        assert_eq!(after.plan_revision, Some(1));
    }

    #[tokio::test]
    async fn apply_scene_drop_removes_and_renumbers() {
        let (_tmp, svc) = fresh_service_local().await;
        let project = project_with_future_chapter(
            &svc,
            "PA drop",
            5,
            vec![scene(1, "keep-a"), scene(2, "drop-me"), scene(3, "keep-b")],
            Vec::new(),
        )
        .await;
        let id = stage(
            &svc,
            &project.project_id,
            "scene_drop",
            Some(5),
            serde_json::json!({"scene_order": 2}),
        )
        .await;
        let out = apply_one(&svc, &project.project_id, &id).await;
        assert_eq!(out.results[0].outcome, "applied", "{out:?}");

        let after = plan_of(&svc, &project.project_id, 5).await;
        let summaries: Vec<(i32, String)> = after
            .scenes
            .iter()
            .map(|s| (s.scene_order, s.summary.clone()))
            .collect();
        assert_eq!(
            summaries,
            vec![(1, "keep-a".to_string()), (2, "keep-b".to_string())],
            "drop renumbers the survivors 1..n"
        );
    }

    #[tokio::test]
    async fn apply_scene_replace_swaps_fields() {
        let (_tmp, svc) = fresh_service_local().await;
        let project = project_with_future_chapter(
            &svc,
            "PA replace",
            5,
            vec![scene(1, "old summary")],
            Vec::new(),
        )
        .await;
        let id = stage(
            &svc,
            &project.project_id,
            "scene_replace",
            Some(5),
            serde_json::json!({
                "scene_order": 1, "summary": "new summary", "purpose": "new purpose",
                "character_ids": ["character:x"]
            }),
        )
        .await;
        let out = apply_one(&svc, &project.project_id, &id).await;
        assert_eq!(out.results[0].outcome, "applied", "{out:?}");
        let after = plan_of(&svc, &project.project_id, 5).await;
        assert_eq!(after.scenes[0].summary, "new summary");
        assert_eq!(after.scenes[0].purpose, "new purpose");
        assert_eq!(
            after.scenes[0].character_ids,
            vec!["character:x".to_string()]
        );
    }

    #[tokio::test]
    async fn apply_scene_reorder_permutes_spine() {
        let (_tmp, svc) = fresh_service_local().await;
        let project = project_with_future_chapter(
            &svc,
            "PA reorder",
            5,
            vec![scene(1, "A"), scene(2, "B"), scene(3, "C")],
            Vec::new(),
        )
        .await;
        // Reverse: new_order [3,2,1] means scene currently at 3 becomes first.
        let id = stage(
            &svc,
            &project.project_id,
            "scene_reorder",
            Some(5),
            serde_json::json!({"new_order": [3, 2, 1]}),
        )
        .await;
        let out = apply_one(&svc, &project.project_id, &id).await;
        assert_eq!(out.results[0].outcome, "applied", "{out:?}");
        let after = plan_of(&svc, &project.project_id, 5).await;
        let summaries: Vec<(i32, String)> = after
            .scenes
            .iter()
            .map(|s| (s.scene_order, s.summary.clone()))
            .collect();
        assert_eq!(
            summaries,
            vec![
                (1, "C".to_string()),
                (2, "B".to_string()),
                (3, "A".to_string()),
            ],
            "reorder applies the permutation then renumbers 1..n"
        );
    }

    #[tokio::test]
    async fn apply_thread_promote_and_retire() {
        let (_tmp, svc) = fresh_service_local().await;
        let project = project_with_future_chapter(
            &svc,
            "PA thread",
            5,
            vec![scene(1, "a")],
            vec!["theme:existing".to_string()],
        )
        .await;
        // Promote adds a theme id.
        let promote = stage(
            &svc,
            &project.project_id,
            "thread_promote",
            Some(5),
            serde_json::json!({"kind": "theme", "id": "theme:added"}),
        )
        .await;
        apply_one(&svc, &project.project_id, &promote).await;
        let after = plan_of(&svc, &project.project_id, 5).await;
        assert!(
            after
                .target_theme_ids
                .contains(&"theme:existing".to_string())
        );
        assert!(after.target_theme_ids.contains(&"theme:added".to_string()));

        // Retire removes the existing theme id.
        let retire = stage(
            &svc,
            &project.project_id,
            "thread_retire",
            Some(5),
            serde_json::json!({"kind": "theme", "id": "theme:existing"}),
        )
        .await;
        apply_one(&svc, &project.project_id, &retire).await;
        let after = plan_of(&svc, &project.project_id, 5).await;
        assert!(
            !after
                .target_theme_ids
                .contains(&"theme:existing".to_string())
        );
        assert!(after.target_theme_ids.contains(&"theme:added".to_string()));
        // Two applies bumped the revision twice.
        assert_eq!(after.plan_revision, Some(2));
    }

    #[tokio::test]
    async fn apply_promise_followup_creates_promise_at_placement() {
        let (_tmp, svc) = fresh_service_local().await;
        let project =
            project_with_future_chapter(&svc, "PA promise", 5, vec![scene(1, "a")], Vec::new())
                .await;
        let id = stage(
            &svc,
            &project.project_id,
            "promise_followup",
            None, // promise_followup carries no target_chapter
            serde_json::json!({
                "promise_type": "mystery",
                "description": "the successor thread",
                "planted_at_placement": {"book_number": 1, "chapter_number": 6, "scene_order": 1}
            }),
        )
        .await;
        let out = apply_one(&svc, &project.project_id, &id).await;
        assert_eq!(out.results[0].outcome, "applied", "{out:?}");
        let promise_id = out.results[0]
            .applied_record_id
            .as_deref()
            .expect("promise id returned");
        assert!(promise_id.starts_with("narrative_promise:"));
        let promise = svc
            .repository()
            .get_narrative_promise(promise_id)
            .await
            .unwrap();
        assert_eq!(promise.description, "the successor thread");
        assert_eq!(promise.planted_at.chapter_number, 6);
        // No prior_state for promise_followup (it targets no chapter row).
        let applied = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: Some("applied".to_string()),
                book_number: None,
                source_chapter: None,
            })
            .await
            .unwrap();
        assert!(applied.amendments[0].prior_state.is_none());
    }

    // ── Immutability guard (Test 2) ──────────────────────────────────────

    #[tokio::test]
    async fn immutability_guard_rejects_drafted_target_at_apply() {
        let (_tmp, svc) = fresh_service_local().await;
        let project =
            project_with_future_chapter(&svc, "PA guard", 5, vec![scene(1, "planned")], Vec::new())
                .await;
        let id = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(5),
            serde_json::json!({"synopsis": "must not apply"}),
        )
        .await;
        // Draft a scene into chapter 5 AFTER staging — the guard is checked at
        // apply (ADR D3), so this makes the amendment fail at decision time.
        svc.save_scene_draft(SaveSceneDraftInput {
            project_id: project.project_id.clone(),
            book_number: 1,
            chapter_number: 5,
            chapter_id: None,
            scene_order: 1,
            full_text: "chapter five now has drafted prose".to_string(),
            summary: "drafted".to_string(),
            content_rating: spindle_core::models::ContentRating::General,
            tone: None,
            generation_id: None,
            source_path: None,
            ..Default::default()
        })
        .await
        .unwrap();

        let err = svc
            .decide_plan_amendments(DecidePlanAmendmentsInput {
                project_id: project.project_id.clone(),
                decisions: vec![PlanAmendmentDecisionInput {
                    amendment_id: id.clone(),
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .expect_err("guard must fail pre-flight");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("persisted scene") || msg.contains("immutab"),
            "error names the immutability guard: {msg}"
        );
        // Zero writes: the amendment is still staged, the synopsis unchanged.
        let staged = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: Some("staged".to_string()),
                book_number: None,
                source_chapter: None,
            })
            .await
            .unwrap();
        assert_eq!(staged.amendments.len(), 1);
        assert_eq!(staged.amendments[0].id, id);
        let after = plan_of(&svc, &project.project_id, 5).await;
        assert_eq!(after.synopsis, "the planned future");
    }

    // ── Pre-flight atomicity (Test 3) ────────────────────────────────────

    #[tokio::test]
    async fn preflight_atomicity_one_bad_decision_aborts_batch() {
        let (_tmp, svc) = fresh_service_local().await;
        let project =
            project_with_future_chapter(&svc, "PA atomic", 5, vec![scene(1, "a")], Vec::new())
                .await;
        let good1 = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(5),
            serde_json::json!({"synopsis": "good one"}),
        )
        .await;
        // A scene_drop naming an order that does not exist — dangling
        // reference the pre-flight catches.
        let bad = stage(
            &svc,
            &project.project_id,
            "scene_drop",
            Some(5),
            serde_json::json!({"scene_order": 99}),
        )
        .await;
        let good2 = stage(
            &svc,
            &project.project_id,
            "thread_promote",
            Some(5),
            serde_json::json!({"kind": "theme", "id": "theme:z"}),
        )
        .await;

        let err = svc
            .decide_plan_amendments(DecidePlanAmendmentsInput {
                project_id: project.project_id.clone(),
                decisions: vec![
                    PlanAmendmentDecisionInput {
                        amendment_id: good1.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                    PlanAmendmentDecisionInput {
                        amendment_id: bad.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                    PlanAmendmentDecisionInput {
                        amendment_id: good2.clone(),
                        action: "apply".to_string(),
                        edit: None,
                        note: None,
                    },
                ],
                decided_by: None,
            })
            .await
            .expect_err("one bad decision aborts");
        assert!(format!("{err:#}").contains("pre-flight"));
        // Zero writes: all three stay staged, the synopsis unchanged.
        let staged = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: Some("staged".to_string()),
                book_number: None,
                source_chapter: None,
            })
            .await
            .unwrap();
        assert_eq!(staged.amendments.len(), 3);
        let after = plan_of(&svc, &project.project_id, 5).await;
        assert_eq!(after.synopsis, "the planned future");
    }

    // ── Mid-apply honesty via the seam (Test 3, seam arm) ────────────────

    #[tokio::test]
    async fn mid_apply_failure_stops_and_reports_honestly() {
        // Pre-flight passes for two synopsis_update applies against two
        // distinct future chapters. Between pre-flight and apply the SECOND
        // chapter's plan row is deleted, so its apply fails at write time
        // (the current plan vanished). Earlier applies stay applied; the
        // failing row stays staged; the later row reads not_reached. The
        // deletion runs through the injectable after-pre-flight hook, so the
        // failure is driven through the genuine dispatch path, not a mock.
        let (_tmp, svc) = fresh_service_local().await;
        let project =
            project_with_future_chapter(&svc, "PA mid", 5, vec![scene(1, "a")], Vec::new()).await;
        // Add a second future chapter (6) and a third (7).
        for ch in [6, 7] {
            svc.plan_chapter(PlanChapterInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: ch,
                pov_character_id: None,
                synopsis: format!("chapter {ch} planned"),
                target_theme_ids: Vec::new(),
                target_conflict_ids: Vec::new(),
                target_plot_line_ids: Vec::new(),
                scenes: vec![scene(1, "s")],
            })
            .await
            .unwrap();
        }
        let first = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(5),
            serde_json::json!({"synopsis": "first applied"}),
        )
        .await;
        let second = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(6),
            serde_json::json!({"synopsis": "second fails"}),
        )
        .await;
        let third = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(7),
            serde_json::json!({"synopsis": "third not reached"}),
        )
        .await;

        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();
        let repo = svc.repository().clone();
        let project_id = project.project_id.clone();
        let branch_id = branch.id.clone();
        let out = svc
            .decide_plan_amendments_with_seam(
                DecidePlanAmendmentsInput {
                    project_id: project.project_id.clone(),
                    decisions: vec![
                        PlanAmendmentDecisionInput {
                            amendment_id: first.clone(),
                            action: "apply".to_string(),
                            edit: None,
                            note: None,
                        },
                        PlanAmendmentDecisionInput {
                            amendment_id: second.clone(),
                            action: "apply".to_string(),
                            edit: None,
                            note: None,
                        },
                        PlanAmendmentDecisionInput {
                            amendment_id: third.clone(),
                            action: "apply".to_string(),
                            edit: None,
                            note: None,
                        },
                    ],
                    decided_by: None,
                },
                move || {
                    let repo = repo.clone();
                    let project_id = project_id.clone();
                    let branch_id = branch_id.clone();
                    async move {
                        repo.delete_chapter_plans_for_chapter(&project_id, &branch_id, 1, 6)
                            .await
                            .unwrap();
                    }
                },
            )
            .await
            .unwrap();

        assert_eq!(out.applied_count, 1);
        assert_eq!(out.failed_count, 1);
        assert_eq!(out.results[0].outcome, "applied");
        assert_eq!(out.results[1].outcome, "failed");
        assert!(out.results[1].error.is_some());
        assert_eq!(out.results[2].outcome, "not_reached");

        // First apply is real (chapter 5 synopsis rewritten, not rolled back).
        let ch5 = plan_of(&svc, &project.project_id, 5).await;
        assert_eq!(ch5.synopsis, "first applied");
        // The failed + not-reached rows stay staged.
        let staged = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: Some("staged".to_string()),
                book_number: None,
                source_chapter: None,
            })
            .await
            .unwrap();
        let staged_ids: Vec<_> = staged.amendments.iter().map(|a| a.id.clone()).collect();
        assert!(staged_ids.contains(&second));
        assert!(staged_ids.contains(&third));
    }

    // ── Reorder / insert coherence (Test 4) ──────────────────────────────

    #[tokio::test]
    async fn incoherent_reorder_fails_preflight() {
        let (_tmp, svc) = fresh_service_local().await;
        let project = project_with_future_chapter(
            &svc,
            "PA badreorder",
            5,
            vec![scene(1, "A"), scene(2, "B"), scene(3, "C")],
            Vec::new(),
        )
        .await;
        // Wrong length (2 orders for a 3-scene chapter).
        let wrong_len = stage(
            &svc,
            &project.project_id,
            "scene_reorder",
            Some(5),
            serde_json::json!({"new_order": [1, 2]}),
        )
        .await;
        let err = svc
            .decide_plan_amendments(DecidePlanAmendmentsInput {
                project_id: project.project_id.clone(),
                decisions: vec![PlanAmendmentDecisionInput {
                    amendment_id: wrong_len.clone(),
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .expect_err("wrong-length permutation rejected");
        assert!(format!("{err:#}").contains("pre-flight"));

        // Duplicate order (dup 1, missing 3).
        let dup = stage(
            &svc,
            &project.project_id,
            "scene_reorder",
            Some(5),
            serde_json::json!({"new_order": [1, 1, 2]}),
        )
        .await;
        let err2 = svc
            .decide_plan_amendments(DecidePlanAmendmentsInput {
                project_id: project.project_id.clone(),
                decisions: vec![PlanAmendmentDecisionInput {
                    amendment_id: dup.clone(),
                    action: "apply".to_string(),
                    edit: None,
                    note: None,
                }],
                decided_by: None,
            })
            .await
            .expect_err("duplicate permutation rejected");
        assert!(format!("{err2:#}").contains("pre-flight"));
    }

    // ── List filtering (list_plan_amendments) ────────────────────────────

    #[tokio::test]
    async fn list_filters_by_status_and_provenance() {
        let (_tmp, svc) = fresh_service_local().await;
        let project =
            project_with_future_chapter(&svc, "PA list", 5, vec![scene(1, "a")], Vec::new()).await;
        let staged = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(5),
            serde_json::json!({"synopsis": "x"}),
        )
        .await;
        // All (no filter) returns it.
        let all = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: None,
                book_number: None,
                source_chapter: None,
            })
            .await
            .unwrap();
        assert_eq!(all.amendments.len(), 1);
        assert_eq!(all.amendments[0].id, staged);
        // Provenance filter (book 1, source chapter 3 — the stage helper's
        // provenance) returns it; a different source chapter returns none.
        let scoped = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: Some("staged".to_string()),
                book_number: Some(1),
                source_chapter: Some(3),
            })
            .await
            .unwrap();
        assert_eq!(scoped.amendments.len(), 1);
        let empty = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: None,
                book_number: Some(1),
                source_chapter: Some(99),
            })
            .await
            .unwrap();
        assert!(empty.amendments.is_empty());
        // A lone book_number (no source_chapter) is an input error.
        let err = svc
            .list_plan_amendments(ListPlanAmendmentsInput {
                project_id: project.project_id.clone(),
                status: None,
                book_number: Some(1),
                source_chapter: None,
            })
            .await;
        assert!(err.is_err(), "lone book_number is rejected");
    }

    #[tokio::test]
    async fn apply_amendments_reseeds_undrafted_chapter_in_active_authoring_run() {
        use crate::sqlite::records::{AuthoringRun, AuthoringRunChapter, AuthoringRunScene};
        use spindle_core::models::ContentRating;

        let (_tmp, svc) = fresh_service_local().await;
        let project = project_with_future_chapter(
            &svc,
            "PA active-run reseed",
            2,
            vec![
                PlanChapterSceneInput {
                    scene_order: 1,
                    summary: "scene one".to_string(),
                    beat_structure: Vec::new(),
                    character_ids: vec!["character:alice".to_string()],
                    location_id: Some("location:hall".to_string()),
                    content_rating: Some(ContentRating::General),
                    purpose: "open".to_string(),
                    research_required: None,
                    research_tags: Vec::new(),
                    explicit_query: None,
                },
                PlanChapterSceneInput {
                    scene_order: 2,
                    summary: "scene two".to_string(),
                    beat_structure: Vec::new(),
                    character_ids: vec!["character:bob".to_string()],
                    location_id: Some("location:courtyard".to_string()),
                    content_rating: Some(ContentRating::Teen),
                    purpose: "turn".to_string(),
                    research_required: None,
                    research_tags: Vec::new(),
                    explicit_query: None,
                },
            ],
            Vec::new(),
        )
        .await;
        let branch = svc
            .repository()
            .get_active_branch(&project.project_id)
            .await
            .unwrap();

        let run_id = "authoring_run:test-reseed".to_string();
        let now = chrono::Utc::now();
        let pre_amendment_rel = "scenes/chapter-0002/scene-001.json".to_string();
        let unowned_scene2_rel = "scenes/chapter-0002/scene-002.json".to_string();
        let pre_amendment_file = crate::workspace::runtime_dir(svc.repository().data_dir())
            .join("../artifacts")
            .join(&pre_amendment_rel);
        let unowned_scene2_file = crate::workspace::runtime_dir(svc.repository().data_dir())
            .join("../artifacts")
            .join(&unowned_scene2_rel);
        // Alternate root outside runtime_dir(data_dir) (e.g., data_dir/../artifacts)
        // that could belong to another workspace or CLI run.
        let alternate_root_file = svc
            .repository()
            .data_dir()
            .join("../artifacts")
            .join(&pre_amendment_rel);
        std::fs::create_dir_all(pre_amendment_file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(alternate_root_file.parent().unwrap()).unwrap();
        std::fs::write(
            &pre_amendment_file,
            r#"{"chapter_number":2,"scene_order":1,"prompt":"stale pre-amendment prompt"}"#,
        )
        .unwrap();
        std::fs::write(
            &unowned_scene2_file,
            r#"{"chapter_number":2,"scene_order":2,"prompt":"unrecorded artifact"}"#,
        )
        .unwrap();
        std::fs::write(
            &alternate_root_file,
            r#"{"chapter_number":2,"scene_order":1,"prompt":"other workspace artifact"}"#,
        )
        .unwrap();
        assert!(pre_amendment_file.exists());
        assert!(unowned_scene2_file.exists());
        assert!(alternate_root_file.exists());

        svc.repository()
            .save_authoring_run(
                AuthoringRun {
                    id: run_id.clone(),
                    project_id: project.project_id.clone(),
                    active_branch_id: branch.id.clone(),
                    book_number: 1,
                    start_chapter: 1,
                    end_chapter: 2,
                    checkpoint_interval: 2,
                    last_checkpoint_end_chapter: 0,
                    artifacts_dir: "../artifacts".to_string(),
                    editorial_directives: Vec::new(),
                    status: "active".to_string(),
                    created_at: now,
                    updated_at: now,
                    mining_policy: None,
                    max_revise_attempts: None,
                    checkpoint_policy: None,
                    replan_policy: Some("propose_after_chapter".to_string()),
                },
                vec![AuthoringRunChapter {
                    authoring_run_id: run_id.clone(),
                    chapter_number: 2,
                    planned: true,
                    synopsis: "the planned future".to_string(),
                    pov_character_id: None,
                    status: "pending".to_string(),
                    summary_saved: false,
                    summary_artifact_path: None,
                    replan_status: None,
                    replan_detail: None,
                }],
                vec![
                    AuthoringRunScene {
                        authoring_run_id: run_id.clone(),
                        chapter_number: 2,
                        scene_order: 1,
                        character_ids: vec!["character:alice".to_string()],
                        location_id: "location:hall".to_string(),
                        content_rating: "general".to_string(),
                        tone: Some("wry".to_string()),
                        source_path: None,
                        phase: "pending".to_string(),
                        scene_id: None,
                        scene_artifact_path: Some(pre_amendment_rel.clone()),
                        draft_diagnostics: None,
                        blocked_reason: None,
                        research_required: None,
                        research_tags: Vec::new(),
                        explicit_query: None,
                        mine_status: None,
                        mine_detail: None,
                        verify_status: None,
                        verify_detail: None,
                        revise_attempts: 0,
                        last_finding_fingerprint: None,
                    },
                    AuthoringRunScene {
                        authoring_run_id: run_id.clone(),
                        chapter_number: 2,
                        scene_order: 2,
                        character_ids: vec!["character:bob".to_string()],
                        location_id: "location:courtyard".to_string(),
                        content_rating: "teen".to_string(),
                        tone: Some("tense".to_string()),
                        source_path: None,
                        phase: "pending".to_string(),
                        scene_id: None,
                        scene_artifact_path: None,
                        draft_diagnostics: None,
                        blocked_reason: None,
                        research_required: None,
                        research_tags: Vec::new(),
                        explicit_query: None,
                        mine_status: None,
                        mine_detail: None,
                        verify_status: None,
                        verify_detail: None,
                        revise_attempts: 0,
                        last_finding_fingerprint: None,
                    },
                ],
                Vec::new(),
            )
            .await
            .unwrap();

        // 1. synopsis_update (revision 2): invalidates pre-amendment artifact and assigns -r2 paths
        let syn_id = stage(
            &svc,
            &project.project_id,
            "synopsis_update",
            Some(2),
            serde_json::json!({"synopsis": "the updated future"}),
        )
        .await;
        apply_one(&svc, &project.project_id, &syn_id).await;
        let (_, chs, scs, _) = svc
            .repository()
            .get_authoring_run(&run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(chs[0].synopsis, "the updated future");
        assert!(
            !pre_amendment_file.exists(),
            "run-owned pre-amendment scene artifact in resolved runtime root must be removed when reseeding"
        );
        assert!(
            alternate_root_file.exists(),
            "same-named artifact in alternate root (data_dir/../artifacts) must survive"
        );
        assert!(
            unowned_scene2_file.exists(),
            "unrecorded legacy filename for scene with scene_artifact_path=None must survive"
        );
        assert_eq!(
            scs[0].scene_artifact_path.as_deref(),
            Some("scenes/chapter-0002/scene-001-r1.json")
        );
        assert_eq!(
            scs[1].scene_artifact_path.as_deref(),
            Some("scenes/chapter-0002/scene-002-r1.json")
        );

        // 2. scene_add (omitting location_id and content_rating; inherits from neighbor)
        let add_id = stage(
            &svc,
            &project.project_id,
            "scene_add",
            Some(2),
            serde_json::json!({
                "scene_order": 2,
                "summary": "inserted bridge",
                "purpose": "bridge",
                "character_ids": ["character:carol"],
                "insert_at": 2
            }),
        )
        .await;
        apply_one(&svc, &project.project_id, &add_id).await;
        let (_, _, scs, _) = svc
            .repository()
            .get_authoring_run(&run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(scs.len(), 3);
        assert_eq!(scs[0].scene_order, 1);
        assert_eq!(scs[0].tone.as_deref(), Some("wry"));
        assert_eq!(scs[1].scene_order, 2);
        assert_eq!(scs[1].character_ids, vec!["character:carol".to_string()]);
        assert!(!scs[1].location_id.is_empty());
        assert_eq!(scs[2].scene_order, 3);
        assert_eq!(scs[2].character_ids, vec!["character:bob".to_string()]);
        assert_eq!(scs[2].tone.as_deref(), Some("tense"));

        // 3. scene_reorder ([3, 1, 2])
        let reorder_id = stage(
            &svc,
            &project.project_id,
            "scene_reorder",
            Some(2),
            serde_json::json!({"new_order": [3, 1, 2]}),
        )
        .await;
        apply_one(&svc, &project.project_id, &reorder_id).await;
        let (_, _, scs, _) = svc
            .repository()
            .get_authoring_run(&run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            scs.iter()
                .map(|s| (s.scene_order, s.character_ids.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, vec!["character:bob".to_string()]),
                (2, vec!["character:alice".to_string()]),
                (3, vec!["character:carol".to_string()]),
            ]
        );
        // Preserves lineage metadata (tone) across reorder
        assert_eq!(scs[0].tone.as_deref(), Some("tense"));
        assert_eq!(scs[1].tone.as_deref(), Some("wry"));

        // 4. scene_drop (drop scene_order 2)
        let drop_id = stage(
            &svc,
            &project.project_id,
            "scene_drop",
            Some(2),
            serde_json::json!({"scene_order": 2}),
        )
        .await;
        apply_one(&svc, &project.project_id, &drop_id).await;
        let (_, _, scs, _) = svc
            .repository()
            .get_authoring_run(&run_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            scs.iter()
                .map(|s| (s.scene_order, s.character_ids.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, vec!["character:bob".to_string()]),
                (2, vec!["character:carol".to_string()]),
            ]
        );
    }
}
