use super::*;

#[test]
fn session_serialization_is_enabled_for_mutating_tools() {
    assert!(tool_requires_session_serialization("save_scene_draft"));
    assert!(tool_requires_session_serialization("commit_scene_changes"));
    assert!(!tool_requires_session_serialization("get_writer_state"));
    assert!(!tool_requires_session_serialization("get_scene_context"));
    assert!(!tool_requires_session_serialization("list_skills"));
    assert!(!tool_requires_session_serialization("get_skill"));
    assert!(!tool_requires_session_serialization("get_reference"));
}

#[test]
fn serialization_scope_uses_global_only_for_process_wide_tools() {
    assert!(tool_requires_global_serialization("create_project"));
    assert!(tool_requires_global_serialization("configure_agents"));
    assert!(tool_requires_global_serialization("init_grok_skills"));
    assert!(!tool_requires_global_serialization("test_agent"));
    assert!(!tool_requires_global_serialization("continue_generation"));
    assert!(!tool_requires_global_serialization("revise_generation"));
    assert!(!tool_requires_global_serialization("save_scene_draft"));
    assert!(!tool_requires_global_serialization("record_note"));

    for name in static_tool_names() {
        if tool_requires_global_serialization(name) {
            assert!(
                tool_requires_session_serialization(name),
                "global serialization tool `{name}` must not also appear in the no-serialization list"
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn project_scoped_serialization_allows_different_projects_to_lock_independently() {
    let router = router().await;

    let first = router
        .lock_tool_scope(ToolSerializationScope::Project("project:first".to_string()))
        .await;
    let second = timeout(
        Duration::from_millis(100),
        router.lock_tool_scope(ToolSerializationScope::Project(
            "project:second".to_string(),
        )),
    )
    .await;
    assert!(
        second.is_ok(),
        "different projects should not share a mutation lock"
    );

    drop(second);
    drop(first);
}

#[tokio::test(flavor = "current_thread")]
async fn project_scoped_serialization_queues_same_project_mutations() {
    let router = router().await;

    let first = router
        .lock_tool_scope(ToolSerializationScope::Project("project:same".to_string()))
        .await;
    let second = timeout(
        Duration::from_millis(50),
        router.lock_tool_scope(ToolSerializationScope::Project("project:same".to_string())),
    )
    .await;
    assert!(
        second.is_err(),
        "same-project mutations must still serialize"
    );

    drop(first);
    let second = timeout(
        Duration::from_millis(100),
        router.lock_tool_scope(ToolSerializationScope::Project("project:same".to_string())),
    )
    .await;
    assert!(second.is_ok(), "same-project lock should release");
}

#[tokio::test(flavor = "current_thread")]
async fn global_serialization_blocks_project_mutations() {
    let router = router().await;

    let global = router.lock_tool_scope(ToolSerializationScope::Global).await;
    let project = timeout(
        Duration::from_millis(50),
        router.lock_tool_scope(ToolSerializationScope::Project("project:first".to_string())),
    )
    .await;
    assert!(project.is_err(), "global lock must block project writes");

    drop(global);
    let project = timeout(
        Duration::from_millis(100),
        router.lock_tool_scope(ToolSerializationScope::Project("project:first".to_string())),
    )
    .await;
    assert!(
        project.is_ok(),
        "project lock should proceed after global release"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_save_scene_draft_calls_complete_without_hanging() {
    let router = router().await;
    let project = router
        .service
        .create_project(CreateProjectInput {
            name: "Concurrent Save Scene Draft".to_string(),
            project_type: "novel".to_string(),
            genre: "fantasy".to_string(),
            reader_contract: ReaderContract {
                promise: "Concurrent scene saves should queue, not hang.".to_string(),
                style_notes: vec![],
                boundaries: vec![],
            },
        })
        .await
        .expect("project");

    for scene_order in 1..=3 {
        router
            .service
            .save_scene_draft(SaveSceneDraftInput {
                project_id: project.project_id.clone(),
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order,
                full_text: format!("Scene {scene_order} baseline."),
                summary: format!("Baseline summary {scene_order}."),
                content_rating: ContentRating::Teen,
                tone: Some("grounded".to_string()),
                source_path: None,
                generation_id: None,
                ..Default::default()
            })
            .await
            .expect("seed scene");
    }

    let temp = tempdir().expect("temp dir");
    let chapter_path = temp.path().join("ch26_signal_and_noise.md");
    std::fs::write(&chapter_path, "placeholder chapter source").expect("write chapter file");
    let chapter_path = chapter_path.display().to_string();

    let save_call = |router: ToolRouter, scene_order: i32| {
        let project_id = project.project_id.clone();
        let chapter_path = chapter_path.clone();
        async move {
            let args = serde_json::to_value(SaveSceneDraftInput {
                project_id,
                book_number: 1,
                chapter_number: 1,
                chapter_id: None,
                scene_order,
                full_text: format!("Scene {scene_order} updated from concurrent tool call."),
                summary: format!("Concurrent summary {scene_order}."),
                content_rating: ContentRating::Teen,
                tone: Some("grounded".to_string()),
                source_path: Some(chapter_path),
                generation_id: None,
                ..Default::default()
            })
            .expect("save args");
            let args = args.as_object().cloned().expect("save args object");
            let result = router
                .call_tool("save_scene_draft", Some(&args))
                .await
                .expect("save scene draft");
            let payload = structured_json(result);
            payload["status"].as_str().expect("save status").to_string()
        }
    };

    let joined = timeout(Duration::from_secs(5), async {
        tokio::join!(
            save_call(router.clone(), 1),
            save_call(router.clone(), 2),
            save_call(router.clone(), 3)
        )
    })
    .await
    .expect("concurrent save_scene_draft calls should not hang");

    assert_eq!(joined.0, "updated");
    assert_eq!(joined.1, "updated");
    assert_eq!(joined.2, "updated");
}

#[tokio::test(flavor = "current_thread")]
async fn transports_share_one_serialization_state_per_workspace() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("spindle.db");
    let pool = SqlitePool::open(&db_path).await.expect("open sqlite pool");
    let service = crate::build_service(pool, dir.path(), None);

    let stdio_server = crate::server::SpindleMcpServer::new(service.clone());
    let http_state = crate::http::HttpState::new(service.clone());
    let console_router = ToolRouter::with_tool_profile_and_serialization(
        service.clone(),
        None,
        Arc::new(ToolSerializationState::default()),
    );

    assert!(
        Arc::ptr_eq(
            stdio_server.serialization_state(),
            http_state.serialization_state()
        ),
        "stdio server and HTTP state must share one ToolSerializationState"
    );
    assert!(
        Arc::ptr_eq(
            stdio_server.serialization_state(),
            console_router.serialization_state()
        ),
        "console router must share the workspace ToolSerializationState"
    );

    let guard = console_router
        .lock_tool_scope(ToolSerializationScope::Project(
            "project:shared".to_string(),
        ))
        .await;
    assert!(
        !console_router
            .scope_is_free(&ToolSerializationScope::Project(
                "project:shared".to_string()
            ))
            .await
    );
    let second_router = ToolRouter::with_tool_profile_and_serialization(
        service,
        None,
        http_state.serialization_state().clone(),
    );
    let blocked = timeout(
        Duration::from_millis(50),
        second_router.lock_tool_scope(ToolSerializationScope::Project(
            "project:shared".to_string(),
        )),
    )
    .await;
    assert!(
        blocked.is_err(),
        "mutation on HTTP transport must contend on lock held by stdio/console transport"
    );
    drop(guard);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn internal_call_token_allows_reentrant_dispatch_while_blocking_external_callers() {
    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("spindle.db");
    let pool = SqlitePool::open(&db_path).await.expect("open sqlite pool");
    let service = crate::build_service(pool, dir.path(), None);
    let router = ToolRouter::with_tool_profile_and_serialization(
        service.clone(),
        None,
        ToolSerializationState::for_service(&service),
    );

    let project_id = prepare_run_project(&router, "Reentrant Lock Test").await;

    let listener = crate::internal_listener::spawn_internal_listener(service.clone())
        .await
        .expect("spawn internal listener");
    let addr = listener.addr();

    // Hold the outer project lock (as `authoring_execute_next` does) and
    // issue an internal call token for `project_id`.
    let outer_guard = router
        .lock_tool_scope(ToolSerializationScope::Project(project_id.clone()))
        .await;
    let token_guard = router
        .serialization_state()
        .issue_internal_token(ToolSerializationScope::Project(project_id.clone()));

    // (1) Internal dispatch carrying `?internal_token=...` succeeds without deadlocking!
    let internal_url = format!("http://{addr}/mcp?internal_token={}", token_guard.token());
    let internal_client = McpHarnessClient::connect(&TransportConfig::Http { url: internal_url })
        .await
        .expect("connect internal client");
    let reentrant_res = timeout(
        Duration::from_secs(2),
        internal_client.create_save_point(&CreateSavePointInput {
            project_id: project_id.clone(),
            name: "reentrant-checkpoint".to_string(),
            description: Some(
                "Dispatched over internal listener while project lock held".to_string(),
            ),
        }),
    )
    .await
    .expect("internal dispatch with valid token must not deadlock")
    .expect("create_save_point via internal token must succeed");
    assert!(!reentrant_res.save_point_id.is_empty());

    // (2) An external HTTP caller without the token blocks on the held project lock.
    let external_url = format!("http://{addr}/mcp");
    let external_client = McpHarnessClient::connect(&TransportConfig::Http { url: external_url })
        .await
        .expect("connect external client");
    let blocked_external = timeout(
        Duration::from_millis(150),
        external_client.create_save_point(&CreateSavePointInput {
            project_id: project_id.clone(),
            name: "external-checkpoint".to_string(),
            description: Some("External call without token".to_string()),
        }),
    )
    .await;
    assert!(
        blocked_external.is_err(),
        "external HTTP caller without internal_token must block while project lock is held"
    );

    // (3) Dropping the token guard invalidates the token immediately.
    let expired_token = token_guard.token().to_string();
    drop(token_guard);
    let expired_call = timeout(
        Duration::from_millis(100),
        router.call_tool_with_internal_token(
            "create_save_point",
            Some(
                serde_json::json!({
                    "project_id": project_id,
                    "name": "expired-token-checkpoint",
                    "description": "Call with expired token"
                })
                .as_object()
                .unwrap(),
            ),
            Some(&expired_token),
        ),
    )
    .await;
    assert!(
        expired_call.is_err(),
        "dropped internal token must no longer bypass project lock"
    );

    drop(outer_guard);
    listener.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn update_entity_locks_resolved_project_id_when_project_id_omitted() {
    let router = router().await;
    let p1 = prepare_run_project(&router, "Project One").await;

    let character = structured_json(
        router
            .call_tool(
                "create_character",
                Some(
                    serde_json::json!({
                        "project_id": p1,
                        "name": "Mira",
                        "summary": "Protagonist of Project One",
                        "role": "protagonist"
                    })
                    .as_object()
                    .unwrap(),
                ),
            )
            .await
            .expect("create character"),
    );
    let character_id = character["character_id"].as_str().unwrap().to_string();

    // Create a second project so it becomes the active session project.
    let p2 = prepare_run_project(&router, "Project Two").await;
    assert_ne!(p1, p2);

    // Resolve arguments + serialization scope for `update_entity` with only `entity_id`.
    let raw_args = serde_json::json!({
        "entity_id": character_id,
        "updates": { "summary": "Updated bio" }
    });
    let resolved = router
        .resolve_arguments("update_entity", Some(raw_args.as_object().unwrap()))
        .await
        .expect("resolve_arguments for update_entity");
    let scope = router
        .tool_serialization_scope("update_entity", &resolved)
        .await
        .expect("tool_serialization_scope for update_entity");
    assert_eq!(
        scope,
        Some(ToolSerializationScope::Project(p1.clone())),
        "update_entity must lock the entity's owning project ({p1}), not the entity_id ({character_id}) or active session project ({p2})"
    );
}
