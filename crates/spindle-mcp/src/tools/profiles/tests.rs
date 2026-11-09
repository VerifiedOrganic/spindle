use super::*;

#[tokio::test(flavor = "current_thread")]
async fn generated_profiles_filter_listing_and_calling() {
    let mut router = router().await;
    for profile in spindle_adapters::guidance::list_profiles() {
        router.tool_profile = Some(profile.name.to_owned());
        let listed: std::collections::BTreeSet<_> = router
            .list_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(
            listed,
            profile.tools.iter().map(|s| (*s).to_owned()).collect()
        );
        for name in profile.tools {
            assert!(router.has_tool(name));
        }
        for name in [
            "configure_agents",
            "init_grok_skills",
            "test_agent",
            "rebuild_search_index",
            "backfill_scene_source_offsets",
            "operator_delete_scene",
        ] {
            assert!(!router.has_tool(name), "{}/{name}", profile.name);
            assert_eq!(
                router.call_tool(name, None).await.unwrap().is_error,
                Some(true)
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_profiles_fail_closed_and_legacy_all_preserves_catalog() {
    let mut router = router().await;
    router.tool_profile = Some("typo".to_owned());
    assert!(router.list_tools().is_empty());
    assert!(!router.has_tool("create_project"));
    assert_eq!(
        router
            .call_tool("create_project", None)
            .await
            .unwrap()
            .is_error,
        Some(true)
    );
    router.tool_profile = Some("legacy-all".to_owned());
    let expected: Vec<_> = router
        .all_tools()
        .into_iter()
        .filter(|tool| !is_operator_only(tool.name.as_ref()))
        .collect();
    assert_eq!(router.list_tools(), expected);
    for name in OPERATOR_ONLY_TOOLS {
        assert!(!router.has_tool(name), "legacy-all must deny {name}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn secondary_proxy_keeps_its_profile_and_denies_mutations_by_name() {
    use rmcp::ServiceExt as _;
    let registry = router().await;
    let service = registry.service().clone();
    let primary = crate::server::SpindleMcpServer::with_tool_profile(service.clone(), None);
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let primary_task = tokio::spawn(primary.serve(server_io));
    let primary_client = timeout(Duration::from_secs(20), ().serve(client_io))
        .await
        .unwrap()
        .unwrap();
    let primary_server = primary_task.await.unwrap().unwrap();
    let proxy = crate::proxy::ProxyHandler::new(
        primary_client.peer().clone(),
        tokio_util::sync::CancellationToken::new(),
        Some("research".to_owned()),
    );
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let proxy_task = tokio::spawn(proxy.serve(server_io));
    let client = timeout(Duration::from_secs(20), ().serve(client_io))
        .await
        .unwrap()
        .unwrap();
    let proxy_server = proxy_task.await.unwrap().unwrap();
    let actual = timeout(Duration::from_secs(20), client.list_all_tools())
        .await
        .unwrap()
        .unwrap();
    let expected = spindle_adapters::guidance::list_profiles()
        .iter()
        .find(|p| p.name == "research")
        .unwrap();
    let names: std::collections::BTreeSet<_> = actual.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, expected.tools.iter().copied().collect());
    let projected: Vec<_> = registry
        .list_tools()
        .into_iter()
        .filter(|t| expected.tools.contains(&t.name.as_ref()))
        .map(|t| crate::tools::project_tool_discovery(Some("research"), t))
        .collect();
    assert_eq!(
        actual, projected,
        "proxy must project output policy as well as names"
    );

    let args = serde_json::json!({"name":"Must not exist", "project_type":"novel", "genre":"fantasy", "reader_contract":{"promise":"A test", "style_notes":[], "boundaries":[]}});
    let result = timeout(
        Duration::from_secs(20),
        client.call_tool(
            rmcp::model::CallToolRequestParams::new("create_project")
                .with_arguments(args.as_object().unwrap().clone()),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(service.list_project_ids().await.unwrap().is_empty());
    client.cancel().await.unwrap();
    proxy_server.cancel().await.unwrap();
    primary_client.cancel().await.unwrap();
    primary_server.cancel().await.unwrap();
}
