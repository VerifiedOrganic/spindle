use std::sync::Arc;

use rmcp::ErrorData as McpError;
use rmcp::RoleServer;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, ReadResourceRequestParams, ReadResourceResult, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use tokio_util::sync::CancellationToken;

use crate::resources::ResourceRouter;
use crate::tools::{ToolRouter, ToolSerializationState};

/// Resolve the model-facing startup profile. Unset uses the generated
/// `workflow-write` profile; legacy names are rejected by `validate_tool_profile`.
pub(crate) fn resolved_startup_tool_profile() -> Option<String> {
    std::env::var("SPINDLE_TOOL_PROFILE")
        .ok()
        .or_else(|| Some("workflow-write".to_string()))
}

#[derive(Clone)]
pub struct SpindleMcpServer {
    tool_router: ToolRouter,
    /// Internal listener sessions may select a generated model profile via
    /// `?profile=`; ordinary model HTTP/stdio sessions may not.
    allow_profile_override: bool,
    resource_router: ResourceRouter,
}

impl SpindleMcpServer {
    pub fn new(service: SpindleService) -> Self {
        let serialization_state = ToolSerializationState::for_service(&service);
        Self::with_serialization_state(service, serialization_state)
    }

    pub(crate) fn with_serialization_state(
        service: SpindleService,
        serialization_state: Arc<ToolSerializationState>,
    ) -> Self {
        let profile = resolved_startup_tool_profile();
        let runtime_profile = Arc::new(std::sync::RwLock::new(profile.clone()));
        let tool_router = ToolRouter::with_tool_profile_and_serialization(
            service.clone(),
            profile,
            serialization_state,
        )
        .with_runtime_profile(runtime_profile);
        Self {
            tool_router,
            allow_profile_override: false,
            resource_router: ResourceRouter::new(service),
        }
    }

    #[cfg(test)]
    pub fn serialization_state(&self) -> &Arc<ToolSerializationState> {
        self.tool_router.serialization_state()
    }

    #[cfg(test)]
    pub fn with_tool_profile(service: SpindleService, tool_profile: Option<String>) -> Self {
        let serialization_state = ToolSerializationState::for_service(&service);
        let runtime_profile = Arc::new(std::sync::RwLock::new(tool_profile.clone()));
        Self {
            tool_router: ToolRouter::with_tool_profile_and_serialization(
                service.clone(),
                tool_profile,
                serialization_state,
            )
            .with_runtime_profile(runtime_profile),
            allow_profile_override: false,
            resource_router: ResourceRouter::new(service),
        }
    }

    /// The internal listener is a local orchestration surface. It keeps the
    /// startup profile for tokenless clients but lets a secondary proxy select
    /// its own generated profile via `?profile=`; a valid internal token still
    /// upgrades a request to the full registry.
    pub(crate) fn with_internal_listener(service: SpindleService) -> Self {
        let serialization_state = ToolSerializationState::for_service(&service);
        let profile = resolved_startup_tool_profile();
        let runtime_profile = Arc::new(std::sync::RwLock::new(profile.clone()));
        let tool_router = ToolRouter::with_tool_profile_and_serialization(
            service.clone(),
            profile,
            serialization_state,
        )
        .with_runtime_profile(runtime_profile);
        Self {
            tool_router,
            allow_profile_override: true,
            resource_router: ResourceRouter::new(service),
        }
    }

    /// Build a router for one request. The base router owns the session
    /// context; profile views share it.
    fn router(&self, internal: bool, requested_profile: Option<&str>) -> ToolRouter {
        if internal {
            return self.tool_router.with_profile(None);
        }
        if let Some(profile) = requested_profile {
            return self.tool_router.with_profile(Some(profile.to_owned()));
        }
        self.tool_router.with_active_profile()
    }

    /// Build a `StreamableHttpService` that spawns a new `SpindleMcpServer`
    /// per session, enabling multiple concurrent MCP clients over HTTP.
    pub fn streamable_http_service(
        service: SpindleService,
        cancellation_token: CancellationToken,
    ) -> StreamableHttpService<Self, LocalSessionManager> {
        let serialization_state = ToolSerializationState::for_service(&service);
        Self::streamable_http_service_with_serialization_state(
            service,
            cancellation_token,
            serialization_state,
        )
    }

    pub(crate) fn streamable_http_service_internal(
        service: SpindleService,
        cancellation_token: CancellationToken,
    ) -> StreamableHttpService<Self, LocalSessionManager> {
        let config =
            StreamableHttpServerConfig::default().with_cancellation_token(cancellation_token);
        StreamableHttpService::new(
            move || Ok(SpindleMcpServer::with_internal_listener(service.clone())),
            Arc::new(LocalSessionManager::default()),
            config,
        )
    }

    pub(crate) fn streamable_http_service_with_serialization_state(
        service: SpindleService,
        cancellation_token: CancellationToken,
        serialization_state: Arc<ToolSerializationState>,
    ) -> StreamableHttpService<Self, LocalSessionManager> {
        let config =
            StreamableHttpServerConfig::default().with_cancellation_token(cancellation_token);

        StreamableHttpService::new(
            move || {
                Ok(SpindleMcpServer::with_serialization_state(
                    service.clone(),
                    serialization_state.clone(),
                ))
            },
            Arc::new(LocalSessionManager::default()),
            config,
        )
    }
}

impl ServerHandler for SpindleMcpServer {
    fn get_info(&self) -> ServerInfo {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_tool_list_changed()
                .build(),
        )
            .with_instructions(
                "Spindle is a persistent story bible for fiction. Start with get_skill(name=\"spindle\"). For a new book, or for any new-book / next-chapter / rolling-window planning request, load get_skill(name=\"new-book\"): its host-neutral flow runs intake, starts or resumes a durable planning session with planning_begin, generates levels with planning_advance, reviews them with planning_status, and records explicit human decisions with planning_decide. Every planning level stays staged and human-gated by default; never auto-accept a plan or an in-run amendment. Select the existing project with set_active_project, then get_writer_state before drafting. The connected host assistant writes prose by default; save with authorship=\"assistant\". Inside an authoring run use authoring_save_scene_draft and its continuity package. Load only the relevant workflow with get_skill or get_reference; MCP resources are optional. Follow the book's voice, canon, and reader contract.

To reconnect after an interruption, call authoring_status for the latest run and planning_status without a session ID for durable planning sessions. planning_status reports the active gate, staged proposals, findings, readiness, and the next safe action. Read `bible://skills/character-creator` and `bible://skills/worldbuilder` for setup flows. Read `bible://skills/scene-writer` for drafting and continuation, and `bible://skills/authoring-supervisor` for multi-chapter runs. Read `bible://skills/new-book` for premise-to-ready planning. Route manuscript import through `bible://skills/manuscript-importer`. Use `bible://skills/revision-manager`, `bible://skills/continuity-editor`, and `bible://skills/bible-librarian` for revision, quality, and Bible lookup flows. Read `bible://skills/editor` for editorial review, developmental editing, and fact-checking workflows. Additional embedded skills and craft references are available under `bible://skills/*` and `bible://references/*`.

Model routes are exposed as read-only resources under `bible://system/model-routes`. Direct entity resources can be read with the `bible://{table}:{id}` template, for example `bible://scene:xyz789`.

All hosts can use the `list_skills`, `get_skill`, and `get_reference` tools as equivalent loading routes for `bible://skills/*`, `bible://references/*`, and `bible://contracts/tools/v1`."
                    .to_string(),
            )
            .with_server_info(Implementation::new("spindle", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let internal_token = extract_internal_token(&context);
        let internal = self
            .tool_router
            .serialization_state()
            .is_internal_token(internal_token.as_deref());
        let requested = if self.allow_profile_override {
            extract_profile_override(&context)
        } else {
            None
        };
        Ok(ListToolsResult::with_all_items(
            self.router(internal, requested.as_deref()).list_tools(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.router(false, None)
            .list_tools()
            .into_iter()
            .find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        use futures_util::FutureExt;
        use std::panic::AssertUnwindSafe;

        // Panic guard. A panic inside a tool handler unwinds past the response
        // writer, so the caller receives NOTHING and blocks until its own
        // timeout — a ~60s hang with no diagnostic instead of an error, and an
        // authoring run parked with no state advance. That is the worst failure
        // mode this server has: it reads as a deadlock, while the panic message
        // goes only to a stderr nobody is tailing. (Lived example: the stale
        // addr-file reclamation path panicked on a nested `block_on`, which
        // presented as `authoring_execute_next` hanging forever.)
        //
        // Converting the panic into an MCP error is a DIAGNOSTIC net, not a
        // correctness mechanism: a handler that panicked may have left a
        // poisoned lock or half-applied state behind. Naming the panic still
        // beats hanging on it — the caller can log it, fail the step, and move
        // on instead of stalling the loop.
        let internal_token = extract_internal_token(&context);
        let internal = self
            .tool_router
            .serialization_state()
            .is_internal_token(internal_token.as_deref());
        let requested = if self.allow_profile_override {
            extract_profile_override(&context)
        } else {
            None
        };
        let router = self.router(internal, requested.as_deref());
        // A deep `check_consistency` can run for tens of minutes. When the
        // client supplied a progress token, forward one notification per
        // finished deep-tier check so the client sees liveness instead of
        // silence. Other tools ignore the sink.
        let progress_token = request
            .meta
            .as_ref()
            .and_then(|meta| meta.get_progress_token())
            .or_else(|| context.meta.get_progress_token());
        let progress = if request.name == "check_consistency" {
            progress_token.map(|token| {
                let (sink, mut events) = tokio::sync::mpsc::unbounded_channel::<
                    spindle_adapters::sqlite::service::ConsistencyProgress,
                >();
                let peer = context.peer.clone();
                tokio::spawn(async move {
                    while let Some(event) = events.recv().await {
                        let completed = event.checks_completed.len();
                        let total = completed + event.checks_pending.len();
                        let message = format!(
                            "check_consistency job {}: {completed} of {total} deep checks complete; {} finding(s) so far",
                            event.job_id, event.issue_count
                        );
                        if peer
                            .notify_progress(ProgressNotificationParam {
                                progress_token: token.clone(),
                                progress: completed as f64,
                                total: Some(total.max(1) as f64),
                                message: Some(message),
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                sink
            })
        } else {
            None
        };
        let call = router.call_tool_with_internal_token_and_progress(
            &request.name,
            request.arguments.as_ref(),
            internal_token.as_deref(),
            progress,
        );
        match AssertUnwindSafe(call).catch_unwind().await {
            Ok(result) => match result {
                Ok(value) => {
                    if request.name == "use_profile"
                        && value.is_error != Some(true)
                        && let Err(error) = context.peer.notify_tool_list_changed().await
                    {
                        tracing::warn!(
                            %error,
                            "failed to send tools/list_changed after profile switch"
                        );
                    }
                    Ok(value)
                }
                Err(error) => Err(error_data(error)),
            },
            Err(panic) => {
                let detail = panic_message(panic.as_ref());
                tracing::error!(
                    tool = %request.name,
                    panic = %detail,
                    "tool handler panicked; returning an error instead of hanging the caller"
                );
                Err(McpError::internal_error(
                    format!("tool {} panicked: {detail}", request.name),
                    None::<serde_json::Value>,
                ))
            }
        }
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        self.resource_router
            .list_resources()
            .await
            .map_err(error_data)
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(self.resource_router.list_resource_templates())
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        self.resource_router
            .read_resource(&request.uri)
            .await
            .map_err(error_data)
    }
}

fn error_data(error: anyhow::Error) -> McpError {
    McpError::internal_error(error.to_string(), None::<serde_json::Value>)
}

fn extract_internal_token(context: &RequestContext<RoleServer>) -> Option<String> {
    let parts = context.extensions.get::<axum::http::request::Parts>()?;
    if let Some(header) = parts
        .headers
        .get("x-spindle-internal-token")
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = header.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    let query = parts.uri.query()?;
    for pair in query.split('&') {
        if let Some((key, value)) = pair.split_once('=')
            && key == "internal_token"
        {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Optional generated-profile selection for an internal-listener secondary.
/// Unknown names are ignored (the base profile still applies); operator names
/// are never valid profiles.
fn extract_profile_override(context: &RequestContext<RoleServer>) -> Option<String> {
    let parts = context.extensions.get::<axum::http::request::Parts>()?;
    let raw = parts
        .headers
        .get("x-spindle-tool-profile")
        .and_then(|value| value.to_str().ok())
        .or_else(|| {
            parts.uri.query().and_then(|query| {
                query.split('&').find_map(|pair| {
                    let (key, value) = pair.split_once('=')?;
                    (key == "profile").then_some(value)
                })
            })
        })?;
    let profile = raw.trim();
    if profile.is_empty() || !crate::tools::profile_names().contains(&profile) {
        return None;
    }
    Some(profile.to_string())
}

/// Best-effort message out of a caught panic payload. `panic!` produces either a
/// `&'static str` or a formatted `String`; anything else is an opaque payload we
/// can only acknowledge.
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "panic with a non-string payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tempfile::tempdir;

    use spindle_adapters::ModelRouter;
    use spindle_adapters::SqlitePool;
    use spindle_adapters::sqlite::Repository as SpindleRepository;

    use super::*;

    /// The panic guard is only useful if the caller learns WHY. A guard that
    /// reported "panicked" with no detail would trade a silent hang for a silent
    /// error — the operator still could not tell a nested-runtime panic from an
    /// unwrap on missing config.
    #[test]
    fn panic_message_recovers_both_standard_payload_shapes() {
        // `panic!("literal")` yields a &'static str payload.
        let literal =
            std::panic::catch_unwind(|| panic!("static literal panic")).expect_err("must unwind");
        assert_eq!(panic_message(literal.as_ref()), "static literal panic");

        // A formatted `panic!` yields a String payload — this is the shape tokio
        // uses for "Cannot start a runtime from within a runtime".
        let formatted =
            std::panic::catch_unwind(|| panic!("formatted {} panic", 42)).expect_err("must unwind");
        assert_eq!(panic_message(formatted.as_ref()), "formatted 42 panic");
    }

    #[test]
    fn panic_message_acknowledges_opaque_payloads() {
        let opaque =
            std::panic::catch_unwind(|| std::panic::panic_any(7u8)).expect_err("must unwind");
        assert_eq!(
            panic_message(opaque.as_ref()),
            "panic with a non-string payload"
        );
    }

    async fn server() -> SpindleMcpServer {
        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("server.db"))
            .await
            .expect("db init");
        let data_dir = temp.path().to_path_buf();
        let service = SpindleService::new(SpindleRepository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ));
        let mut server = SpindleMcpServer::with_tool_profile(service, None);
        server.tool_router.hold_temp_dir(temp);
        server
    }

    #[tokio::test(flavor = "current_thread")]
    async fn session_context_survives_per_request_profile_views() {
        use rmcp::ServiceExt as _;

        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("server.db"))
            .await
            .expect("db init");
        let data_dir = temp.keep();
        let service = SpindleService::new(SpindleRepository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ));
        let server = SpindleMcpServer::with_tool_profile(service, None);
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let task = tokio::spawn(server.serve(server_io));
        let client = ().serve(client_io).await.unwrap();

        async fn call(
            client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
            name: &str,
            args: serde_json::Value,
        ) -> serde_json::Value {
            let result = client
                .call_tool(
                    rmcp::model::CallToolRequestParams::new(name.to_string())
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            assert_ne!(result.is_error, Some(true), "{name}: {result:?}");
            serde_json::from_value(result.structured_content.unwrap()).unwrap()
        }

        let project_args = |name: &str| {
            serde_json::json!({
                "name": name,
                "project_type": "novel",
                "genre": "fantasy",
                "reader_contract": {"promise": "A test", "style_notes": [], "boundaries": []}
            })
        };
        let p1 = call(&client, "create_project", project_args("First")).await;
        let p1_id = p1["project_id"].as_str().unwrap().to_string();
        let _p2 = call(&client, "create_project", project_args("Second")).await;
        call(
            &client,
            "set_active_project",
            serde_json::json!({"project_id": p1_id}),
        )
        .await;
        let branch = call(
            &client,
            "create_branch",
            serde_json::json!({
                "project_id": p1_id,
                "name": "alternate",
                "branch_type": "alternate"
            }),
        )
        .await;
        let branch_id = branch["branch_id"].as_str().unwrap().to_string();
        call(
            &client,
            "switch_branch",
            serde_json::json!({"project_id": p1_id, "branch_id": branch_id}),
        )
        .await;
        let state = call(&client, "get_writer_state", serde_json::json!({})).await;
        assert_eq!(state["current"]["project"]["project_id"], p1_id);
        assert_eq!(state["current"]["branch"]["branch_id"], branch_id);

        client.cancel().await.unwrap();
        task.await.unwrap().unwrap();
    }

    /// A deep check must not go silent when the client supplied a progress
    /// token. This drives the real stdio-style server loop and asserts the
    /// forwarded notification names the finished deep-tier check.
    #[derive(Clone, Default)]
    struct ProgressRecorder {
        events: std::sync::Arc<std::sync::Mutex<Vec<rmcp::model::ProgressNotificationParam>>>,
    }

    impl rmcp::ClientHandler for ProgressRecorder {
        async fn on_progress(
            &self,
            params: rmcp::model::ProgressNotificationParam,
            _context: rmcp::service::NotificationContext<rmcp::RoleClient>,
        ) {
            self.events.lock().expect("progress events").push(params);
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_check_forwards_mcp_progress_notifications() {
        use rmcp::ServiceExt as _;
        use rmcp::model::{CallToolRequestParams, ClientRequest, Request};
        use rmcp::service::PeerRequestOptions;

        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("server.db"))
            .await
            .expect("db init");
        let data_dir = temp.path().to_path_buf();
        let service = SpindleService::new(SpindleRepository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ));
        let server = SpindleMcpServer::with_tool_profile(service, None);
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(server.serve(server_io));
        let recorder = ProgressRecorder::default();
        let events = recorder.events.clone();
        let client = tokio::time::timeout(Duration::from_secs(20), recorder.serve(client_io))
            .await
            .expect("client init timed out")
            .expect("client init");

        let project = client
            .call_tool(
                CallToolRequestParams::new("create_project").with_arguments(
                    serde_json::json!({
                        "name": "Progress Test",
                        "project_type": "novel",
                        "genre": "fantasy",
                        "reader_contract": {
                            "promise": "A test",
                            "style_notes": [],
                            "boundaries": []
                        }
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .expect("create_project");
        let project_id = project.structured_content.unwrap()["project_id"]
            .as_str()
            .unwrap()
            .to_string();

        let saved = client
            .call_tool(
                CallToolRequestParams::new("save_scene_draft").with_arguments(
                    serde_json::json!({
                        "project_id": project_id,
                        "book_number": 1,
                        "chapter_number": 1,
                        "scene_order": 1,
                        "full_text": "He sat at the long table and turned the matter over. MOCK_TEMPORAL_JUMP. The lamps were lit and the room had gone cold.",
                        "summary": "a quiet accounting",
                        "content_rating": "General"
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .expect("save_scene_draft");
        assert_ne!(saved.is_error, Some(true), "save failed: {saved:?}");

        let params = CallToolRequestParams::new("check_consistency").with_arguments(
            serde_json::json!({
                "project_id": project_id,
                "scope": {"scope_type": "full"},
                "checks": ["temporal_coherence"],
                "severity_filter": [],
                "deep_check": true,
                "subjects": []
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        // `call_tool` does not expose the request handle, and the rmcp client
        // assigns the progress token itself; use the cancellable path so the
        // test can assert the token the client actually sent.
        let handle = client
            .send_cancellable_request(
                ClientRequest::CallToolRequest(Request::new(params)),
                PeerRequestOptions::no_options(),
            )
            .await
            .expect("send check_consistency");
        let token = handle.progress_token.clone();
        let result = handle
            .await_response()
            .await
            .expect("check_consistency response");
        let result = match result {
            rmcp::model::ServerResult::CallToolResult(result) => result,
            other => panic!("unexpected check_consistency response: {other:?}"),
        };
        assert_ne!(result.is_error, Some(true), "deep check failed: {result:?}");
        let output = result.structured_content.expect("structured output");
        assert_eq!(
            output["status"].as_str(),
            Some(spindle_core::models::ConsistencyStatus::Complete.as_str())
        );
        assert!(output["job_id"].as_str().is_some());

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if !events.lock().expect("progress events").is_empty() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "no progress notification arrived for a deep check"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let seen = events.lock().expect("progress events").clone();
        assert!(
            seen.iter().all(|event| event.progress_token == token),
            "progress token mismatch: {seen:?}"
        );
        assert!(
            seen.iter().any(|event| {
                event.total == Some(1.0)
                    && event
                        .message
                        .as_deref()
                        .is_some_and(|message| message.contains("deep checks complete"))
            }),
            "progress must report the finished deep check: {seen:?}"
        );

        client.cancel().await.unwrap();
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deep_check_tool_starts_joins_and_collects_a_job() {
        use rmcp::ServiceExt as _;
        use rmcp::model::CallToolRequestParams;

        let temp = tempdir().expect("temp dir");
        let db = SqlitePool::open(&temp.path().join("server.db"))
            .await
            .expect("db init");
        let data_dir = temp.path().to_path_buf();
        let mut service = SpindleService::new(SpindleRepository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ));
        service.set_consistency_job_join_wait(Duration::from_millis(50));
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        service.set_consistency_job_gate(Some(gate.clone()));
        let server = SpindleMcpServer::with_tool_profile(service, None);
        let (server_io, client_io) = tokio::io::duplex(64 * 1024);
        let server_task = tokio::spawn(server.serve(server_io));
        let client = tokio::time::timeout(Duration::from_secs(20), ().serve(client_io))
            .await
            .expect("client init timed out")
            .expect("client init");

        async fn call(
            client: &rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
            name: &str,
            args: serde_json::Value,
        ) -> serde_json::Value {
            let result = client
                .call_tool(
                    CallToolRequestParams::new(name.to_string())
                        .with_arguments(args.as_object().unwrap().clone()),
                )
                .await
                .unwrap();
            assert_ne!(result.is_error, Some(true), "{name}: {result:?}");
            serde_json::from_value(result.structured_content.unwrap()).unwrap()
        }

        let project = call(
            &client,
            "create_project",
            serde_json::json!({
                "name": "Deep Job Test",
                "project_type": "novel",
                "genre": "fantasy",
                "reader_contract": {
                    "promise": "A test",
                    "style_notes": [],
                    "boundaries": []
                }
            }),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap().to_string();
        call(
            &client,
            "save_scene_draft",
            serde_json::json!({
                "project_id": project_id,
                "book_number": 1,
                "chapter_number": 1,
                "scene_order": 1,
                "full_text": "He sat at the long table and turned the matter over. MOCK_TEMPORAL_JUMP. The lamps were lit and the room had gone cold.",
                "summary": "a quiet accounting",
                "content_rating": "General"
            }),
        )
        .await;

        let deep_args = || {
            serde_json::json!({
                "project_id": project_id,
                "scope": {"scope_type": "full"},
                "checks": ["temporal_coherence"],
                "severity_filter": [],
                "deep_check": true,
                "subjects": []
            })
        };

        let first = call(&client, "check_consistency", deep_args()).await;
        assert_eq!(
            first["status"].as_str(),
            Some(spindle_core::models::ConsistencyStatus::InProgress.as_str())
        );
        let job_id = first["job_id"].as_str().unwrap().to_string();
        assert_eq!(
            first["checks_pending"],
            serde_json::json!(["temporal_coherence.deep"])
        );
        let second = call(&client, "check_consistency", deep_args()).await;
        assert_eq!(
            second["status"].as_str(),
            Some(spindle_core::models::ConsistencyStatus::InProgress.as_str())
        );
        assert_eq!(second["job_id"].as_str(), Some(job_id.as_str()));

        gate.add_permits(1);
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let collected = loop {
            let output = call(&client, "check_consistency", deep_args()).await;
            if output["status"].as_str()
                == Some(spindle_core::models::ConsistencyStatus::Complete.as_str())
            {
                break output;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "deep-check job did not finish: {output:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert_eq!(collected["job_id"].as_str(), Some(job_id.as_str()));
        assert!(
            collected["issues"]
                .as_array()
                .unwrap()
                .iter()
                .any(|issue| issue["check_type"] == "temporal_coherence"),
            "collected audit must include the deep finding: {collected:?}"
        );

        client.cancel().await.unwrap();
        server_task.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn initialize_info_matches_current_public_surface() {
        let server = server().await;
        let info = server.get_info();

        assert_eq!(info.server_info.name, "spindle");
        assert!(info.capabilities.tools.is_some());
        assert_eq!(
            info.capabilities
                .tools
                .as_ref()
                .and_then(|tools| tools.list_changed),
            Some(true),
            "clients need tools.listChanged to refresh after use_profile"
        );
        assert!(info.capabilities.resources.is_some());
        assert!(
            info.instructions
                .as_deref()
                .expect("instructions")
                .contains("bible://skills/scene-writer")
        );
        assert!(
            info.instructions
                .as_deref()
                .expect("instructions")
                .contains("bible://skills/revision-manager")
        );
        assert!(
            info.instructions
                .as_deref()
                .expect("instructions")
                .contains("bible://skills/continuity-editor")
        );
        assert!(
            info.instructions
                .as_deref()
                .expect("instructions")
                .contains("bible://skills/bible-librarian")
        );
        assert!(
            info.instructions
                .as_deref()
                .expect("instructions")
                .contains("bible://references/*")
        );
        assert!(
            info.instructions
                .as_deref()
                .expect("instructions")
                .contains("bible://system/model-routes")
        );
        assert!(
            info.instructions
                .as_deref()
                .expect("instructions")
                .contains("bible://{table}:{id}")
        );
        for host_term in ["Codex", "GPT-6", "Grok"] {
            assert!(
                !info
                    .instructions
                    .as_deref()
                    .expect("instructions")
                    .contains(host_term),
                "initialize instructions must remain host-neutral (found {host_term})"
            );
        }
    }
}
