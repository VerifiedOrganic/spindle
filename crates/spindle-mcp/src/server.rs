use std::sync::Arc;

use rmcp::ErrorData as McpError;
use rmcp::RoleServer;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Implementation, InitializeResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResult, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use tokio_util::sync::CancellationToken;

use crate::resources::ResourceRouter;
use crate::tools::{ToolRouter, ToolSerializationState};

#[derive(Clone)]
pub struct SpindleMcpServer {
    tool_router: ToolRouter,
    resource_router: ResourceRouter,
}

impl SpindleMcpServer {
    pub fn new(service: SpindleService) -> Self {
        Self::with_serialization_state(service, Arc::new(ToolSerializationState::default()))
    }

    pub(crate) fn with_serialization_state(
        service: SpindleService,
        serialization_state: Arc<ToolSerializationState>,
    ) -> Self {
        Self {
            tool_router: ToolRouter::with_tool_profile_and_serialization(
                service.clone(),
                std::env::var("SPINDLE_TOOL_PROFILE").ok(),
                serialization_state,
            ),
            resource_router: ResourceRouter::new(service),
        }
    }

    #[cfg(test)]
    pub fn with_tool_profile(service: SpindleService, tool_profile: Option<String>) -> Self {
        Self {
            tool_router: ToolRouter::with_tool_profile_and_serialization(
                service.clone(),
                tool_profile,
                Arc::new(ToolSerializationState::default()),
            ),
            resource_router: ResourceRouter::new(service),
        }
    }

    /// Build a `StreamableHttpService` that spawns a new `SpindleMcpServer`
    /// per session, enabling multiple concurrent MCP clients over HTTP.
    pub fn streamable_http_service(
        service: SpindleService,
        cancellation_token: CancellationToken,
    ) -> StreamableHttpService<Self, LocalSessionManager> {
        Self::streamable_http_service_with_serialization_state(
            service,
            cancellation_token,
            Arc::new(ToolSerializationState::default()),
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
                "Read `bible://skills/character-creator` and `bible://skills/worldbuilder` for setup flows. Read `bible://skills/scene-writer` for drafting flows. Route manuscript import and continuation work through `bible://skills/manuscript-importer`. Use `bible://skills/revision-manager`, `bible://skills/continuity-editor`, and `bible://skills/bible-librarian` for revision, quality, and Bible lookup flows. Read `bible://skills/editor` for editorial review, developmental editing, and fact-checking workflows. Additional embedded skills and craft references are available under `bible://skills/*` and `bible://references/*`.

Grok users: `init_grok_skills` now defaults to installing into your global `~/.grok/skills/` (recommended). Pass `global=false` + `target_dir` only if you specifically want repo-scoped adapters. This gives you the full set of Spindle skills (spindle, spindle-scene-writer, spindle-character-creator, etc.) as first-class Grok skills.

Model routes are exposed as read-only resources under `bible://system/model-routes`. Direct entity resources can be read with the `bible://{table}:{id}` template, for example `bible://scene:xyz789`.

MCP clients without resource support (e.g. Kimi): use the `list_skills`, `get_skill`, and `get_reference` tools — they return the same content as the `bible://skills/*` and `bible://references/*` resources."
                    .to_string(),
            )
            .with_server_info(Implementation::new("spindle", env!("CARGO_PKG_VERSION")))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(
            self.tool_router.list_tools(),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router
            .list_tools()
            .into_iter()
            .find(|tool| tool.name == name)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
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
        let call = self
            .tool_router
            .call_tool(&request.name, request.arguments.as_ref());
        match AssertUnwindSafe(call).catch_unwind().await {
            Ok(result) => result.map_err(error_data),
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
        let data_dir = temp.keep();
        let service = SpindleService::new(SpindleRepository::with_model_router(
            db,
            data_dir,
            ModelRouter::local_only(),
        ));
        SpindleMcpServer::with_tool_profile(service, None)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn initialize_info_matches_current_public_surface() {
        let server = server().await;
        let info = server.get_info();

        assert_eq!(info.server_info.name, "spindle");
        assert!(info.capabilities.tools.is_some());
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
    }
}
