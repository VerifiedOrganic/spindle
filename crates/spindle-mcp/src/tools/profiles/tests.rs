use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use rmcp::model::CallToolRequestParams;
use spindle_core::models::UseProfileOutput;

#[derive(Clone)]
struct ToolListChangedClient {
    changes: Arc<AtomicUsize>,
    sender: tokio::sync::mpsc::UnboundedSender<()>,
}

impl rmcp::ClientHandler for ToolListChangedClient {
    async fn on_tool_list_changed(
        &self,
        _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
    ) {
        self.changes.fetch_add(1, Ordering::SeqCst);
        let _ = self.sender.send(());
    }
}

fn notification_client() -> (
    ToolListChangedClient,
    tokio::sync::mpsc::UnboundedReceiver<()>,
    Arc<AtomicUsize>,
) {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let changes = Arc::new(AtomicUsize::new(0));
    (
        ToolListChangedClient {
            changes: changes.clone(),
            sender,
        },
        receiver,
        changes,
    )
}

async fn tool_names(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ToolListChangedClient>,
) -> std::collections::BTreeSet<String> {
    timeout(Duration::from_secs(20), client.list_all_tools())
        .await
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

fn generated_profile(name: &str) -> &'static spindle_adapters::guidance::GeneratedProfile {
    spindle_adapters::guidance::list_profiles()
        .iter()
        .find(|profile| profile.name == name)
        .expect("generated profile")
}

fn profile_args(profile: &str) -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({"profile": profile})
        .as_object()
        .unwrap()
        .clone()
}

async fn switch_profile(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ToolListChangedClient>,
    profile: &str,
) -> CallToolResult {
    timeout(
        Duration::from_secs(20),
        client.call_tool(
            CallToolRequestParams::new("use_profile").with_arguments(profile_args(profile)),
        ),
    )
    .await
    .unwrap()
    .unwrap()
}

async fn call_denied_tool(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ToolListChangedClient>,
) -> CallToolResult {
    timeout(
        Duration::from_secs(20),
        client.call_tool(
            CallToolRequestParams::new("create_branch").with_arguments(
                serde_json::json!({"project_id": "project:missing", "name": "switch"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        ),
    )
    .await
    .unwrap()
    .unwrap()
}

fn assert_unknown_tool(result: &CallToolResult) {
    assert_eq!(result.is_error, Some(true));
    assert!(
        format!("{:?}", result.content).contains("unknown tool"),
        "expected unknown-tool denial: {result:?}"
    );
}

fn assert_dispatchable(result: &CallToolResult) {
    assert_eq!(result.is_error, Some(true));
    assert!(
        !format!("{:?}", result.content).contains("unknown tool"),
        "expected dispatch beyond discovery filtering: {result:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn use_profile_switches_stdio_session_and_notifies() {
    use rmcp::ServiceExt as _;

    let registry = router().await;
    let server = crate::server::SpindleMcpServer::with_tool_profile(
        registry.service().clone(),
        Some("workflow-write".to_owned()),
    );
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server_task = tokio::spawn(server.serve(server_io));
    let (handler, mut changes, _) = notification_client();
    let client = timeout(Duration::from_secs(20), handler.serve(client_io))
        .await
        .unwrap()
        .unwrap();

    let workflow = generated_profile("workflow-write");
    assert_eq!(
        tool_names(&client).await,
        workflow
            .tools
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    );
    assert_unknown_tool(&call_denied_tool(&client).await);

    let switched = switch_profile(&client, "full").await;
    assert_eq!(switched.is_error, Some(false), "{switched:?}");
    let output: UseProfileOutput =
        serde_json::from_value(switched.structured_content.expect("structured content")).unwrap();
    timeout(Duration::from_secs(5), changes.recv())
        .await
        .expect("tools/list_changed notification")
        .expect("notification channel");
    let full = generated_profile("full");
    let names = tool_names(&client).await;
    assert_eq!(
        names,
        full.tools.iter().map(|name| (*name).to_owned()).collect()
    );
    assert_eq!(output.profile, "full");
    assert_eq!(output.tool_count, names.len());
    assert_eq!(
        output
            .tools
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        names
    );
    assert_dispatchable(&call_denied_tool(&client).await);

    let rejected = switch_profile(&client, "no-such-profile").await;
    assert_eq!(rejected.is_error, Some(true));
    let error = rejected.structured_content.expect("structured error");
    assert_eq!(error["error"]["code"], "unknown_profile");
    assert!(
        error["error"]["available_profiles"]
            .as_array()
            .unwrap()
            .iter()
            .any(|profile| profile == "full")
    );
    assert_eq!(
        tool_names(&client).await,
        full.tools.iter().map(|name| (*name).to_owned()).collect(),
        "a rejected switch must leave the active profile unchanged"
    );

    client.cancel().await.unwrap();
    server_task.await.unwrap().unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn use_profile_switches_http_session_without_leaking_to_other_sessions() {
    use rmcp::ServiceExt as _;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };

    struct ServerTask(
        tokio::task::JoinHandle<()>,
        tokio_util::sync::CancellationToken,
    );
    impl Drop for ServerTask {
        fn drop(&mut self) {
            self.1.cancel();
            self.0.abort();
        }
    }

    let registry = router().await;
    let service = registry.service().clone();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let http = StreamableHttpService::new(
        move || {
            Ok::<_, std::io::Error>(crate::server::SpindleMcpServer::with_tool_profile(
                service.clone(),
                Some("workflow-write".to_owned()),
            ))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default().with_cancellation_token(cancellation.clone()),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new().nest_service("/mcp", http);
    let _task = ServerTask(
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }),
        cancellation,
    );

    let (handler, mut changes, _) = notification_client();
    let client = timeout(
        Duration::from_secs(20),
        handler.serve(StreamableHttpClientTransport::from_uri(format!(
            "http://{address}/mcp"
        ))),
    )
    .await
    .unwrap()
    .unwrap();
    let other = timeout(
        Duration::from_secs(20),
        ().serve(StreamableHttpClientTransport::from_uri(format!(
            "http://{address}/mcp"
        ))),
    )
    .await
    .unwrap()
    .unwrap();

    let workflow = generated_profile("workflow-write");
    let expected: std::collections::BTreeSet<String> = workflow
        .tools
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(tool_names(&client).await, expected);
    let other_before = timeout(Duration::from_secs(20), other.list_all_tools())
        .await
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(other_before, expected);

    let switched = switch_profile(&client, "full").await;
    assert_eq!(switched.is_error, Some(false), "{switched:?}");
    timeout(Duration::from_secs(5), changes.recv())
        .await
        .expect("tools/list_changed notification")
        .expect("notification channel");
    assert_eq!(
        tool_names(&client).await,
        generated_profile("full")
            .tools
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    );
    let other_after = timeout(Duration::from_secs(20), other.list_all_tools())
        .await
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        other_after, expected,
        "profile switching must be scoped to one HTTP session"
    );

    client.cancel().await.unwrap();
    other.cancel().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn secondary_proxy_switches_within_but_not_beyond_primary_catalog() {
    use rmcp::ServiceExt as _;

    let registry = router().await;
    let primary = crate::server::SpindleMcpServer::with_tool_profile(
        registry.service().clone(),
        Some("workflow-write".to_owned()),
    );
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let primary_task = tokio::spawn(primary.serve(server_io));
    let primary_client = timeout(Duration::from_secs(20), ().serve(client_io))
        .await
        .unwrap()
        .unwrap();
    let proxy = crate::proxy::ProxyHandler::new(
        primary_client.peer().clone(),
        tokio_util::sync::CancellationToken::new(),
        Some("workflow-write".to_owned()),
    );
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let proxy_task = tokio::spawn(proxy.serve(server_io));
    let (handler, mut changes, _) = notification_client();
    let client = timeout(Duration::from_secs(20), handler.serve(client_io))
        .await
        .unwrap()
        .unwrap();

    let workflow = generated_profile("workflow-write");
    assert_eq!(
        tool_names(&client).await,
        workflow
            .tools
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    );

    let narrowed = switch_profile(&client, "setup").await;
    assert_eq!(narrowed.is_error, Some(false), "{narrowed:?}");
    timeout(Duration::from_secs(5), changes.recv())
        .await
        .expect("tools/list_changed notification")
        .expect("notification channel");
    assert_eq!(
        tool_names(&client).await,
        generated_profile("setup")
            .tools
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    );
    assert_unknown_tool(&call_denied_tool(&client).await);

    let expanded = switch_profile(&client, "full").await;
    assert_eq!(expanded.is_error, Some(true));
    let error = expanded.structured_content.expect("structured error");
    assert_eq!(error["error"]["code"], "profile_not_available");
    assert!(
        format!("{error}").contains("cannot expand beyond its primary"),
        "{error}"
    );
    assert_eq!(
        tool_names(&client).await,
        generated_profile("setup")
            .tools
            .iter()
            .map(|name| (*name).to_owned())
            .collect(),
        "a rejected secondary switch must keep the current profile"
    );

    client.cancel().await.unwrap();
    proxy_task.await.unwrap().unwrap();
    primary_client.cancel().await.unwrap();
    primary_task.await.unwrap().unwrap();
}

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
async fn unknown_and_removed_profiles_fail_closed() {
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
    for removed in ["legacy-all", "minimal", "write", "import", "authoring"] {
        router.tool_profile = Some(removed.to_owned());
        assert!(router.list_tools().is_empty(), "{removed} must fail closed");
        assert!(
            !router.has_tool("create_project"),
            "{removed} must deny tool calls"
        );
        assert_eq!(
            router
                .call_tool("create_project", None)
                .await
                .unwrap()
                .is_error,
            Some(true)
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn internal_listener_honors_secondary_profile_override() {
    use spindle_harness::mcp::{McpHarnessClient, TransportConfig};

    let registry = router().await;
    let listener = crate::internal_listener::spawn_internal_listener(registry.service().clone())
        .await
        .expect("internal listener");
    let url = format!("http://{}/mcp?profile=worldbuilding", listener.addr());
    let client = McpHarnessClient::connect(&TransportConfig::Http { url })
        .await
        .expect("connect secondary profile client");
    let tools = client.list_all_tools().await.expect("list tools");
    let names: std::collections::BTreeSet<_> = tools.iter().map(|t| t.name.as_ref()).collect();
    let worldbuilding = spindle_adapters::guidance::list_profiles()
        .iter()
        .find(|profile| profile.name == "worldbuilding")
        .unwrap();
    assert_eq!(
        names,
        worldbuilding.tools.iter().copied().collect(),
        "secondary profile override must not be narrowed to the primary's profile"
    );
    assert!(names.contains("create_faction"));

    let result = client
        .call_tool::<_, serde_json::Value>(
            "create_faction",
            &serde_json::json!({"project_id": "project:missing", "name": "Test Faction"}),
        )
        .await;
    let error = result.expect_err("missing project must fail validation");
    assert!(
        !format!("{error:#}").contains("unknown tool"),
        "create_faction must be dispatchable under the secondary profile: {error:#}"
    );

    drop(client);
    listener.shutdown();
}

#[tokio::test(flavor = "current_thread")]
async fn secondary_proxy_defaults_to_generated_workflow_write() {
    use rmcp::ServiceExt as _;
    let registry = router().await;
    let service = registry.service().clone();
    let primary = crate::server::SpindleMcpServer::with_tool_profile(
        service.clone(),
        Some("workflow-write".to_owned()),
    );
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
        Some("workflow-write".to_owned()),
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
    let names: std::collections::BTreeSet<_> = actual.iter().map(|t| t.name.as_ref()).collect();
    let workflow = spindle_adapters::guidance::list_profiles()
        .iter()
        .find(|p| p.name == "workflow-write")
        .unwrap();
    assert_eq!(names, workflow.tools.iter().copied().collect());
    assert!(names.contains("save_scene_draft"));
    assert!(!names.contains("create_branch"));

    let result = timeout(
        Duration::from_secs(20),
        client.call_tool(
            rmcp::model::CallToolRequestParams::new("create_branch").with_arguments(
                serde_json::json!({"project_id":"project:test","name":"out-of-profile"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result.is_error, Some(true));
    assert!(
        format!("{:?}", result.content).contains("unknown tool"),
        "secondary must deny an out-of-profile call: {result:?}"
    );

    client.cancel().await.unwrap();
    proxy_server.cancel().await.unwrap();
    primary_client.cancel().await.unwrap();
    primary_server.cancel().await.unwrap();
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
