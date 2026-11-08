use super::*;

#[tokio::test(flavor = "current_thread")]
async fn invalid_tool_input_returns_structured_tool_error() {
    let router = router().await;

    let result = router
        .call_tool("create_project", Some(&serde_json::Map::new()))
        .await
        .expect("tool call should return result");

    assert_eq!(result.is_error, Some(true));
    assert!(
        result.structured_content.is_none(),
        "error should not use structured content"
    );
    let text = result.content.first().expect("error content");
    let text = format!("{text:?}");
    assert!(
        text.contains("missing field"),
        "expected 'missing field' in: {text}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_tool_returns_structured_tool_error() {
    let router = router().await;

    let result = router
        .call_tool("not_a_real_tool", None)
        .await
        .expect("tool call should return result");

    assert_eq!(result.is_error, Some(true));
    assert!(
        result.structured_content.is_none(),
        "error should not use structured content"
    );
    let text = result.content.first().expect("error content");
    let text = format!("{text:?}");
    assert!(
        text.contains("unknown tool: not_a_real_tool"),
        "expected error text in: {text}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn broken_config_toml_is_surfaced_in_list_agents_and_health() {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    let dir = tempdir().expect("tempdir");
    let db_path = dir.path().join("spindle.db");
    let bad_config_path = dir.path().join("config.toml");
    std::fs::write(&bad_config_path, "this is = [broken toml\n").expect("write broken config");

    let pool = SqlitePool::open(&db_path).await.expect("open sqlite pool");
    let service = crate::build_service(
        pool,
        dir.path(),
        Some(bad_config_path.display().to_string()),
    );

    let router = ToolRouter::with_tool_profile_and_serialization(
        service.clone(),
        None,
        ToolSerializationState::for_service(&service),
    );
    let list_agents = structured_json(
        router
            .call_tool("list_agents", None)
            .await
            .expect("list_agents tool call"),
    );
    let config_err = list_agents["config_error"]
        .as_str()
        .expect("list_agents must surface config_error when config.toml is broken");
    assert!(
        config_err.contains("failed to parse"),
        "unexpected config_error: {config_err}"
    );

    let app = crate::http::mcp_router(service, tokio_util::sync::CancellationToken::new());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("GET /health");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read health body");
    let health_json: Value = serde_json::from_slice(&bytes).expect("parse health json");
    assert_eq!(
        health_json["config_error"].as_str(),
        Some(config_err),
        "/health must surface config_error when config.toml is broken"
    );
}
