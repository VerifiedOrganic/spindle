use super::*;

#[tokio::test]
async fn mine_scene_canon_tool_stages_deltas_end_to_end() {
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Mine MCP".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "mined canon".to_string(),
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
    .expect("project output");

    // One committed scene carrying the mining sentinel.
    let save_args = serde_json::to_value(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "A grey tower loomed. MOCK_CANON_MINE stood at its gate.".to_string(),
        summary: "s".to_string(),
        content_rating: ContentRating::General,
        tone: None,
        source_path: None,
        generation_id: None,
        ..Default::default()
    })
    .expect("save args");
    let save_args = save_args.as_object().cloned().expect("save args object");
    let saved: SaveSceneDraftOutput = serde_json::from_value(structured_json(
        router
            .call_tool("save_scene_draft", Some(&save_args))
            .await
            .expect("save scene draft"),
    ))
    .expect("save output");

    let mine_args = serde_json::to_value(MineSceneCanonInput {
        project_id: project.project_id.clone(),
        scene_id: saved.scene_id.clone(),
    })
    .expect("mine args");
    let mine_args = mine_args.as_object().cloned().expect("mine args object");
    let out: MineSceneCanonOutput = serde_json::from_value(structured_json(
        router
            .call_tool("mine_scene_canon", Some(&mine_args))
            .await
            .expect("mine scene canon"),
    ))
    .expect("mine output");

    assert_eq!(out.status, "staged");
    assert_eq!(out.staged.len(), 1);
    assert_eq!(out.staged[0].delta_class, "canonical_fact");
}

#[tokio::test(flavor = "current_thread")]
async fn canon_delta_ratify_round_trip_via_mcp() {
    // End-to-end over the MCP surface: stage via mine (sentinel), list the
    // queue, apply the one delta, and confirm the fact is real canon.
    let router = router().await;

    let create_project_args = serde_json::to_value(CreateProjectInput {
        name: "Ratify MCP".to_string(),
        project_type: "novel".to_string(),
        genre: "fantasy".to_string(),
        reader_contract: ReaderContract {
            promise: "ratified canon".to_string(),
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
    .expect("project output");

    let save_args = serde_json::to_value(SaveSceneDraftInput {
        project_id: project.project_id.clone(),
        book_number: 1,
        chapter_number: 1,
        chapter_id: None,
        scene_order: 1,
        full_text: "A grey tower loomed. MOCK_CANON_MINE stood at its gate.".to_string(),
        summary: "s".to_string(),
        content_rating: ContentRating::General,
        tone: None,
        source_path: None,
        generation_id: None,
        ..Default::default()
    })
    .expect("save args");
    let save_args = save_args.as_object().cloned().expect("save args object");
    let saved: SaveSceneDraftOutput = serde_json::from_value(structured_json(
        router
            .call_tool("save_scene_draft", Some(&save_args))
            .await
            .expect("save scene draft"),
    ))
    .expect("save output");

    // Stage via mine.
    let mine_args = serde_json::to_value(MineSceneCanonInput {
        project_id: project.project_id.clone(),
        scene_id: saved.scene_id.clone(),
    })
    .expect("mine args");
    let mine_args = mine_args.as_object().cloned().expect("mine args object");
    router
        .call_tool("mine_scene_canon", Some(&mine_args))
        .await
        .expect("mine scene canon");

    // List the ratify queue.
    let list_args = serde_json::to_value(ListCanonDeltasInput {
        project_id: project.project_id.clone(),
        status: Some("staged".to_string()),
        scene_id: Some(saved.scene_id.clone()),
        chapter_range: None,
    })
    .expect("list args");
    let list_args = list_args.as_object().cloned().expect("list args object");
    let listed: ListCanonDeltasOutput = serde_json::from_value(structured_json(
        router
            .call_tool("list_canon_deltas", Some(&list_args))
            .await
            .expect("list canon deltas"),
    ))
    .expect("list output");
    assert_eq!(listed.deltas.len(), 1);
    let delta = &listed.deltas[0];
    assert_eq!(delta.delta_class, "canonical_fact");
    assert_eq!(delta.status, "staged");
    // Evidence is the verbatim sentinel from the prose.
    assert_eq!(delta.evidence, "MOCK_CANON_MINE");

    // Apply it.
    let decide_args = serde_json::to_value(DecideCanonDeltasInput {
        project_id: project.project_id.clone(),
        decisions: vec![CanonDeltaDecisionInput {
            delta_id: delta.id.clone(),
            action: "apply".to_string(),
            edit: None,
            note: Some("looks right".to_string()),
        }],
        decided_by: Some("mcp-operator".to_string()),
    })
    .expect("decide args");
    let decide_args = decide_args
        .as_object()
        .cloned()
        .expect("decide args object");
    let decided: DecideCanonDeltasOutput = serde_json::from_value(structured_json(
        router
            .call_tool("decide_canon_deltas", Some(&decide_args))
            .await
            .expect("decide canon deltas"),
    ))
    .expect("decide output");
    assert_eq!(decided.applied_count, 1);
    assert_eq!(decided.results[0].outcome, "applied");
    assert_eq!(decided.results[0].note.as_deref(), Some("looks right"));
    assert!(
        decided.results[0]
            .applied_record_id
            .as_deref()
            .is_some_and(|id| id.starts_with("canonical_fact:"))
    );

    // The row is now applied.
    let applied_list_args = serde_json::to_value(ListCanonDeltasInput {
        project_id: project.project_id.clone(),
        status: Some("applied".to_string()),
        scene_id: Some(saved.scene_id.clone()),
        chapter_range: None,
    })
    .expect("applied list args");
    let applied_list_args = applied_list_args
        .as_object()
        .cloned()
        .expect("applied list object");
    let applied: ListCanonDeltasOutput = serde_json::from_value(structured_json(
        router
            .call_tool("list_canon_deltas", Some(&applied_list_args))
            .await
            .expect("list applied"),
    ))
    .expect("applied output");
    assert_eq!(applied.deltas.len(), 1);
    assert_eq!(applied.deltas[0].status, "applied");
    assert_eq!(
        applied.deltas[0].decided_by.as_deref(),
        Some("mcp-operator")
    );
}
