use super::*;

/// A knowledge_learned reveal on its own SATISFIES the mandatory continuity
/// package (design §2.3 path 2): a scene whose only durable change is an
/// on-page reveal must not be rejected for an "empty" package.
#[test]
fn auto_strict_product_lock_requires_hard_count_zero() {
    use spindle_core::models::ConsistencyIssue;
    let soft_only: Vec<ConsistencyIssue> = Vec::new();
    assert!(
        !auto_strict_anti_slop_blocks(&soft_only),
        "soft-only (no anti_slop issues) must not invent a hard finding"
    );
    let hard = vec![ConsistencyIssue {
        severity: "warning".into(),
        check_type: "anti_slop".into(),
        message: "hard shelf `emotion_cocktail` over limit".into(),
        entity_ids: vec!["scene:1".into()],
        suggested_action: Some("rewrite from the beat".into()),
    }];
    assert!(
        auto_strict_anti_slop_blocks(&hard),
        "auto_strict must block when hard_count != 0"
    );
    let other = vec![ConsistencyIssue {
        severity: "info".into(),
        check_type: "scene_stub_text".into(),
        message: "stub".into(),
        entity_ids: vec![],
        suggested_action: None,
    }];
    assert!(
        !auto_strict_anti_slop_blocks(&other),
        "non-anti_slop findings are not this product lock"
    );
}

#[test]
fn knowledge_learned_counts_toward_package_satisfaction() {
    use spindle_core::models::{
        AuthoringSaveSceneDraftInput, ContentRating, KnowledgeLearnedEntry,
    };
    let base = AuthoringSaveSceneDraftInput {
        project_id: "project:x".into(),
        run_id: None,
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "prose".into(),
        authorship: Default::default(),
        summary: "s".into(),
        content_rating: ContentRating::General,
        tone: None,
        generation_id: None,
        source_path: None,
        location_id: None,
        research_source_ids: Vec::new(),
        research_note_ids: Vec::new(),
        research_claim_ids: Vec::new(),
        research_query_pack_input: None,
        research_context_hash: None,
        character_states: Vec::new(),
        canonical_facts: Vec::new(),
        relationship_updates: Vec::new(),
        beats: Vec::new(),
        continuity_notes: Vec::new(),
        knowledge_learned: Vec::new(),
    };
    assert_eq!(
        super::authoring_structured_update_count(&base),
        0,
        "an empty package is empty"
    );
    let with_reveal = AuthoringSaveSceneDraftInput {
        knowledge_learned: vec![KnowledgeLearnedEntry {
            character_id: "character:bran".into(),
            fact: "Mara is reincarnated.".into(),
            source_summary: None,
            secret_of_fact_id: Some("canonical_fact:reinc".into()),
            reader_visible: Some(true),
        }],
        ..base
    };
    assert_eq!(
        super::authoring_structured_update_count(&with_reveal),
        1,
        "a knowledge_learned reveal satisfies the mandatory package"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn update_promise_status_accepts_common_alias_fields() {
    let router = router().await;

    let project = router
        .service
        .create_project(CreateProjectInput {
            name: "Promise Alias Test".to_string(),
            project_type: "novel".to_string(),
            genre: "sports drama".to_string(),
            reader_contract: ReaderContract {
                promise: "Alias fields should deserialize for promise updates.".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .await
        .expect("create project directly");

    let promise = router
        .service
        .create_narrative_promise(CreateNarrativePromiseInput {
            project_id: project.project_id,
            promise_type: "callback".to_string(),
            description: "Hotel room details recur later.".to_string(),
            planted_at: StoryPlacement {
                book_number: 1,
                chapter_number: 1,
                scene_order: Some(1),
                note: None,
            },
            planned_payoff: Some(StoryPlacement {
                book_number: 1,
                chapter_number: 25,
                scene_order: Some(1),
                note: None,
            }),
            notes: vec![],
        })
        .await
        .expect("create promise directly");

    let args = serde_json::json!({
        "promise_id": promise.narrative_promise_id,
        "new_status": "planted",
        "scene_id": "scene:unused-alias-check",
        "note": "Specific hotel room details planted in scene 1."
    });
    let args = args.as_object().cloned().expect("alias args object");

    let updated: UpdatePromiseStatusOutput = serde_json::from_value(structured_json(
        router
            .call_tool("update_promise_status", Some(&args))
            .await
            .expect("update promise with alias fields"),
    ))
    .expect("decode updated promise");

    assert_eq!(updated.status, "planted");
}

#[tokio::test(flavor = "current_thread")]
async fn set_active_project_does_not_depend_on_writer_state_budget() {
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Budget Independent Session Defaults".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "Session defaults should not require a writer-state bundle.".to_string(),
            style_notes: vec![],
            boundaries: vec![],
        },
    })
    .expect("create project args");
    let create_project_args = create_project_args
        .as_object()
        .cloned()
        .expect("create project object");
    let project: CreateProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_project", Some(&create_project_args))
            .await
            .expect("create project"),
    ))
    .expect("decode project");

    let long_rule_body = r#"Magic "rewrites" cause a \ cascade of constraint debt across every revision boundary and must be treated as binding law in all future scenes. "#.repeat(40);
    for index in 0..12 {
        router
            .service
            .create_world_rule(CreateWorldRuleInput {
                project_id: project.project_id.clone(),
                rule_name: format!("Constraint {}", index + 1),
                rule_type: "law".to_string(),
                description: format!("{}{}", long_rule_body, index + 1),
                scan_pattern: None,
                relevance_tags: vec![],
                established_in: None,
            })
            .await
            .expect("create world rule");
    }

    let writer_state_args = serde_json::json!({
        "project_id": project.project_id.clone(),
        "format": "json",
        "budget_tokens": 1,
        "include_subjects": false,
        "include_recent_activity": false,
        "recent_activity_limit": 0
    });
    let writer_state_args = writer_state_args
        .as_object()
        .cloned()
        .expect("writer state object");
    let writer_state_error = router
        .call_tool("get_writer_state", Some(&writer_state_args))
        .await
        .expect("writer state should return a structured error result");
    assert_eq!(writer_state_error.is_error, Some(true));
    let writer_state_error = format!(
        "{:?}",
        writer_state_error.content.first().expect("error content")
    );
    assert!(
        writer_state_error.contains("mandatory writer-state sections"),
        "expected writer-state budget failure, got: {writer_state_error}"
    );

    let set_active_args = serde_json::to_value(SetActiveProjectInput {
        project_id: project.project_id.clone(),
        branch_id: None,
    })
    .expect("set active args");
    let set_active_args = set_active_args
        .as_object()
        .cloned()
        .expect("set active object");
    let active: SetActiveProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("set_active_project", Some(&set_active_args))
            .await
            .expect("set active project"),
    ))
    .expect("decode set active project");
    assert_eq!(active.project_id, project.project_id);
    // Per-project main branches (Phase 6 reconciliation, Risk #6): the
    // SurrealDB-era singleton `bible_branch:main` no longer exists.
    // Every project owns its own main branch with a ULID-flavoured id,
    // surfaced via `CreateProjectOutput.branch_id`. The session default
    // resolved through set_active_project must round-trip to that id.
    assert_eq!(active.branch_id, project.branch_id);
}

#[tokio::test(flavor = "current_thread")]
async fn writer_state_default_budget_degrades_and_stays_within_budget() {
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Writer State Budget Regression".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "The writer state must fit its advertised budget.".to_string(),
            style_notes: vec![],
            boundaries: vec![],
        },
    })
    .expect("create project args");
    let create_project_args = create_project_args
        .as_object()
        .cloned()
        .expect("create project object");
    let project: CreateProjectOutput = serde_json::from_value(structured_json(
        router
            .call_tool("create_project", Some(&create_project_args))
            .await
            .expect("create project"),
    ))
    .expect("decode project");

    let long_rule_body = "Magic rewrites cause cascading constraint debt across every \
            revision boundary and must be treated as binding law in all future scenes. "
        .repeat(40);
    for index in 0..12 {
        router
            .service
            .create_world_rule(CreateWorldRuleInput {
                project_id: project.project_id.clone(),
                rule_name: format!("Constraint {}", index + 1),
                rule_type: "law".to_string(),
                description: format!("{}{}", long_rule_body, index + 1),
                scan_pattern: None,
                relevance_tags: vec![],
                established_in: None,
            })
            .await
            .expect("create world rule");
    }

    // The user-reported default is 8,000 tokens. A default call must now
    // succeed by compacting required constraints, not by returning an error.
    for (budget, format) in [
        (None, "markdown"),
        (Some(8_000), "markdown"),
        (Some(16_000), "markdown"),
        (Some(24_000), "markdown"),
        (Some(8_000), "json"),
        (Some(24_000), "json"),
    ] {
        let mut args = serde_json::json!({
            "project_id": project.project_id.clone(),
            "format": format,
        });
        if let Some(budget) = budget {
            args["budget_tokens"] = serde_json::json!(budget);
        }
        let args = args.as_object().cloned().expect("writer state object");
        let result = router
            .call_tool("get_writer_state", Some(&args))
            .await
            .expect("writer state");
        assert_ne!(
            result.is_error,
            Some(true),
            "writer state must degrade instead of failing at {format} budget {budget:?}: {:?}",
            result.content
        );
        let payload = structured_json(result.clone());
        assert_eq!(payload["bundle_summary"]["truncated"], true);
        let omissions = payload["omissions"]
            .as_array()
            .expect("typed writer-state omissions");
        assert!(!omissions.is_empty());
        for omission in omissions {
            assert!(omission["section"].is_string());
            assert!(omission["reason"].is_string());
            assert!(omission["omitted"].as_u64().is_some());
            assert!(omission["fetch_with"].is_string());
            assert!(omission["hint"].is_string());
        }
        if format == "markdown" {
            assert!(
                payload["writer_state_markdown"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("Omitted for budget")
            );
        }
        let serialized = serde_json::to_string(&result).expect("serialize writer state result");
        let effective_budget = budget.unwrap_or(8_000);
        assert!(
            serialized.chars().count() / 4 <= effective_budget,
            "writer state output for {format} budget {effective_budget} was {} tokens",
            serialized.chars().count() / 4
        );
    }
}

/// A plan scene with no location and no rating must be a descriptive
/// `Err` naming chapter + scene — never a handler panic, never a silent
/// default (silent defaults corrupt fiction canon).
#[test]
fn start_run_seed_rejects_scene_missing_location_and_rating() {
    let err = super::require_planned_scene_location_and_rating(
        3,
        2,
        None,
        None,
        "A quiet scene with no legacy markers.",
        "Advance the plot.",
    )
    .expect_err("missing location+rating must fail, not panic or default");
    let msg = format!("{err:?}");
    assert!(msg.contains("Chapter 3"), "error names the chapter: {msg}");
    assert!(msg.contains("scene 2"), "error names the scene: {msg}");
}

#[test]
fn start_run_seed_rejects_scene_missing_rating() {
    let err = super::require_planned_scene_location_and_rating(
        1,
        1,
        Some("location:hall"),
        None,
        "A quiet scene with no legacy markers.",
        "Advance the plot.",
    )
    .expect_err("missing rating must fail, not panic or default");
    let msg = format!("{err:?}");
    assert!(msg.contains("Chapter 1"), "error names the chapter: {msg}");
    assert!(msg.contains("scene 1"), "error names the scene: {msg}");
    assert!(
        msg.contains("rating"),
        "error names the missing field: {msg}"
    );
}

#[test]
fn start_run_seed_rejects_scene_missing_location() {
    let err = super::require_planned_scene_location_and_rating(
        1,
        4,
        None,
        Some(&ContentRating::General),
        "A quiet scene with no legacy markers.",
        "Advance the plot.",
    )
    .expect_err("missing location must fail, not panic or default");
    let msg = format!("{err:?}");
    assert!(msg.contains("Chapter 1"), "error names the chapter: {msg}");
    assert!(msg.contains("scene 4"), "error names the scene: {msg}");
    assert!(
        msg.contains("location"),
        "error names the missing field: {msg}"
    );
}

#[test]
fn start_run_seed_accepts_complete_scene() {
    let (loc_id, rating) = super::require_planned_scene_location_and_rating(
        1,
        1,
        Some("location:hall"),
        Some(&ContentRating::Teen),
        "A quiet scene with no legacy markers.",
        "Advance the plot.",
    )
    .expect("complete scene resolves");
    assert_eq!(loc_id, "location:hall");
    assert_eq!(rating, ContentRating::Teen);
}

#[test]
fn start_run_seed_still_honors_legacy_markers() {
    let (loc_id, rating) = super::require_planned_scene_location_and_rating(
        1,
        1,
        None,
        None,
        "Confrontation. location:hall rating:teen",
        "Advance the plot.",
    )
    .expect("legacy markers still resolve");
    assert_eq!(loc_id, "location:hall");
    assert_eq!(rating, ContentRating::Teen);
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_run_chapter_count_max_single_accepted() {
    let router = router().await;
    let project_id = prepare_run_project(&router, "Overflow Max Single").await;
    // True end is exactly i32::MAX: representable, so no range message.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": i32::MAX,
        "chapter_count": 1,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result, not panic");
    assert_eq!(result.is_error, Some(false));
    let content = result.structured_content.expect("structured content");
    assert_eq!(content["end_chapter"].as_i64(), Some(i32::MAX as i64));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        !has_range_message(missing),
        "representable (MAX, 1) range must not report out of bounds: {missing:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_run_chapter_count_min_rejected() {
    let router = router().await;
    let args = serde_json::json!({
        "project_id": "project:overflow",
        "book_number": 1,
        "start_chapter": i32::MIN,
        "chapter_count": 0,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result, not panic");
    assert_eq!(result.is_error, Some(false));
    let content = result
        .structured_content
        .expect("validation error uses structured content");
    assert_eq!(content["ready_to_draft"].as_bool(), Some(false));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    let expected = format!(
        "chapter range out of bounds: start_chapter {} with chapter_count {} exceeds the supported chapter-number range",
        i32::MIN,
        0,
    );
    assert!(
        missing
            .iter()
            .any(|val| val.as_str() == Some(expected.as_str())),
        "expected range message in: {missing:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_run_chapter_count_boundary_overflow_and_fit() {
    let router = router().await;
    let project_id = prepare_run_project(&router, "Overflow Boundary").await;

    // True end i32::MAX + 1 is unrepresentable: validation error.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": i32::MAX - 1,
        "chapter_count": 3,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result, not panic");
    assert_eq!(result.is_error, Some(false));
    let content = result
        .structured_content
        .expect("validation error uses structured content");
    assert_eq!(content["ready_to_draft"].as_bool(), Some(false));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        has_range_message(missing),
        "expected out-of-bounds validation error in: {missing:?}"
    );

    // True end is exactly i32::MAX: representable, so accepted.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": i32::MAX - 1,
        "chapter_count": 2,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result, not panic");
    assert_eq!(result.is_error, Some(false));
    let content = result.structured_content.expect("structured content");
    assert_eq!(content["end_chapter"].as_i64(), Some(i32::MAX as i64));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        !has_range_message(missing),
        "representable (MAX-1, 2) range must not report out of bounds: {missing:?}"
    );

    // Small range near the boundary: proceeds with exact end.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": i32::MAX - 1,
        "chapter_count": 1,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result");
    assert_eq!(result.is_error, Some(false));
    let content = result.structured_content.expect("structured content");
    assert_eq!(content["end_chapter"].as_i64(), Some((i32::MAX - 1) as i64));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        !has_range_message(missing),
        "exactly-fitting range must not report out of bounds: {missing:?}"
    );
}

#[test]
fn chapter_range_end_accepts_representable_max_ranges() {
    // A full tool call for (1, MAX) is rejected by the span cap, so
    // arithmetic representability is covered here against the same
    // helper the handler uses.
    assert_eq!(
        super::chapter_range_end(1, i32::MAX),
        Some(i32::MAX),
        "(1, MAX) ends exactly at MAX and is representable"
    );
    assert_eq!(
        super::chapter_range_end(i32::MAX, 1),
        Some(i32::MAX),
        "(MAX, 1) ends exactly at MAX and is representable"
    );
    assert_eq!(
        super::chapter_range_end(i32::MAX - 1, 2),
        Some(i32::MAX),
        "(MAX-1, 2) ends exactly at MAX and is representable"
    );
    assert_eq!(
        super::chapter_range_end(i32::MAX - 1, 3),
        None,
        "true end MAX+1 is unrepresentable"
    );
    assert_eq!(
        super::chapter_range_end(i32::MIN, 0),
        None,
        "(MIN, 0) is out of bounds"
    );
    assert_eq!(
        super::chapter_range_end(i32::MIN, i32::MIN),
        None,
        "subtract-first stays panic-free for MIN count"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_run_chapter_span_over_max_rejected() {
    let router = router().await;
    let project_id = prepare_run_project(&router, "Span Over Max").await;

    // count=1001 → span 1001, rejected before the per-chapter loop.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "chapter_count": 1001,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result, not panic");
    assert_eq!(result.is_error, Some(false));
    let content = result
        .structured_content
        .expect("validation error uses structured content");
    assert_eq!(content["ready_to_draft"].as_bool(), Some(false));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        missing.iter().any(|val| val.as_str()
            == Some(
                "chapter range spans 1001 chapters, exceeding the per-call maximum of 1000 chapters"
            )),
        "expected span message in: {missing:?}"
    );

    // Explicit end = start + 1000 → span 1001, rejected.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1001,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result, not panic");
    assert_eq!(result.is_error, Some(false));
    let content = result
        .structured_content
        .expect("validation error uses structured content");
    assert_eq!(content["ready_to_draft"].as_bool(), Some(false));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        has_span_message(missing),
        "expected span message in: {missing:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_run_chapter_span_at_max_accepted() {
    let router = router().await;
    let project_id = prepare_run_project(&router, "Span At Max").await;

    // count=1000 → span 1000, no span message.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "chapter_count": 1000,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result");
    assert_eq!(result.is_error, Some(false));
    let content = result.structured_content.expect("structured content");
    assert_eq!(content["end_chapter"].as_i64(), Some(1000));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        !has_span_message(missing),
        "span of exactly 1000 must not report span message: {missing:?}"
    );

    // Explicit end = start + 999 → span 1000, no span message.
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "end_chapter": 1000,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result");
    assert_eq!(result.is_error, Some(false));
    let content = result.structured_content.expect("structured content");
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        !has_span_message(missing),
        "explicit span of exactly 1000 must not report span message: {missing:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn prepare_run_chapter_count_happy_path_computes_end_chapter() {
    let router = router().await;
    let project_id = prepare_run_project(&router, "Overflow Happy Path").await;
    let args = serde_json::json!({
        "project_id": project_id,
        "book_number": 1,
        "start_chapter": 1,
        "chapter_count": 2,
    });
    let result = router
        .call_tool("authoring_prepare_run", Some(args.as_object().unwrap()))
        .await
        .expect("tool call should return result");
    assert_eq!(result.is_error, Some(false));
    let content = result.structured_content.expect("structured content");
    assert_eq!(content["end_chapter"].as_i64(), Some(2));
    let missing = content["missing_requirements"]
        .as_array()
        .expect("missing_requirements array");
    assert!(
        !has_range_message(missing),
        "normal range must not report out of bounds: {missing:?}"
    );
}
