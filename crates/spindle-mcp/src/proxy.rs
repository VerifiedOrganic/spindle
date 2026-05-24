use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResult, ServerCapabilities,
    ServerInfo, Tool,
};
use rmcp::service::{Peer, RequestContext, RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ErrorData as McpError, RoleClient, RoleServer, ServerHandler, ServiceExt};
use tokio_util::sync::CancellationToken;

use crate::read_addr_file;

/// A `ServerHandler` that proxies every request to a remote primary MCP server
/// via an HTTP client connection.
pub struct ProxyHandler {
    client: Peer<RoleClient>,
    failover_signal: CancellationToken,
}

impl ProxyHandler {
    pub fn new(client: Peer<RoleClient>, failover_signal: CancellationToken) -> Self {
        Self {
            client,
            failover_signal,
        }
    }

    fn map_proxy_error<T>(&self, result: Result<T, ServiceError>) -> Result<T, McpError> {
        map_proxy_service_error(&self.failover_signal, result)
    }
}

impl ServerHandler for ProxyHandler {
    fn get_info(&self) -> ServerInfo {
        match self.client.peer_info() {
            Some(info) => info.clone(),
            None => rmcp::model::InitializeResult::new(
                ServerCapabilities::builder()
                    .enable_tools()
                    .enable_resources()
                    .build(),
            )
            .with_server_info(rmcp::model::Implementation::new(
                "spindle-proxy",
                env!("CARGO_PKG_VERSION"),
            )),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self.map_proxy_error(self.client.list_all_tools().await)?;
        Ok(ListToolsResult::with_all_items(tools))
    }

    fn get_tool(&self, _name: &str) -> Option<Tool> {
        None
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        self.map_proxy_error(self.client.call_tool(request).await)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let resources = self.map_proxy_error(self.client.list_all_resources().await)?;
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        self.map_proxy_error(self.client.read_resource(request).await)
    }
}

/// Connect to an existing primary as a secondary proxy.
pub async fn connect_to_primary(
    addr: SocketAddr,
) -> anyhow::Result<RunningService<RoleClient, ()>> {
    let uri = format!("http://{addr}/mcp");
    tracing::info!("secondary: connecting to primary at {uri}");

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri),
    );
    ().serve(transport)
        .await
        .context("failed to connect to primary")
}

/// Try to acquire the DB lock. Returns `true` if the lock is available
/// (primary is gone), `false` if another process grabbed it first.
/// Caller should exit cleanly — Claude Code will restart us as the new primary.
pub async fn try_promote(data_dir: &Path) -> anyhow::Result<bool> {
    // SQLite uses file locks rather than an embedded service. Opening the
    // pool implicitly opens a writer connection which holds the lock until
    // dropped. If another process holds it we get a SQLITE_BUSY-flavoured
    // error that is_lock_error matches.
    let db_path = data_dir.join("spindle.sqlite");
    match spindle_adapters::SqlitePool::open(&db_path).await {
        Ok(_pool) => Ok(true),
        Err(e) if crate::is_lock_error(&e) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Wait for a primary to appear by polling the addr file + health check.
pub async fn wait_for_primary(data_dir: &Path) -> anyhow::Result<SocketAddr> {
    for _ in 0..40 {
        if let Ok(addr) = read_addr_file(data_dir)
            && reqwest::Client::new()
                .get(format!("http://{addr}/health"))
                .timeout(Duration::from_secs(1))
                .send()
                .await
                .is_ok()
        {
            return Ok(addr);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("timed out waiting for a primary to appear")
}

fn map_proxy_service_error<T>(
    failover_signal: &CancellationToken,
    result: Result<T, ServiceError>,
) -> Result<T, McpError> {
    result.map_err(|error| match error {
        ServiceError::TransportSend(_) | ServiceError::TransportClosed => {
            failover_signal.cancel();
            McpError::internal_error(
                "spindle primary became unavailable; the secondary proxy is restarting, retry the request",
                None::<serde_json::Value>,
            )
        }
        other => McpError::internal_error(other.to_string(), None::<serde_json::Value>),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_errors_trigger_failover_signal() {
        let failover_signal = CancellationToken::new();

        let error =
            map_proxy_service_error::<()>(&failover_signal, Err(ServiceError::TransportClosed))
                .expect_err("transport failure should become an MCP error");

        assert!(failover_signal.is_cancelled());
        assert!(error.message.contains("spindle primary became unavailable"));
    }

    #[test]
    fn non_transport_errors_do_not_trigger_failover_signal() {
        let failover_signal = CancellationToken::new();

        let error =
            map_proxy_service_error::<()>(&failover_signal, Err(ServiceError::UnexpectedResponse))
                .expect_err("non-transport failures should still surface as MCP errors");

        assert!(!failover_signal.is_cancelled());
        assert!(error.message.contains("Unexpected response type"));
    }
}
