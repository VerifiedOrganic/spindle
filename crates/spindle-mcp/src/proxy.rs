use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, RwLock};
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
use spindle_core::models::{UseProfileInput, UseProfileOutput};
use tokio_util::sync::CancellationToken;

use crate::read_addr_file;

/// A `ServerHandler` that proxies every request to a remote primary MCP server
/// via an HTTP client connection.
pub struct ProxyHandler {
    client: Peer<RoleClient>,
    failover_signal: CancellationToken,
    /// Session-scoped active profile. A secondary may narrow within the
    /// primary's discovered catalog but can never expand beyond it.
    tool_profile: Arc<RwLock<Option<String>>>,
}

impl ProxyHandler {
    pub fn new(
        client: Peer<RoleClient>,
        failover_signal: CancellationToken,
        tool_profile: Option<String>,
    ) -> Self {
        Self {
            client,
            failover_signal,
            tool_profile: Arc::new(RwLock::new(tool_profile)),
        }
    }

    fn map_proxy_error<T>(&self, result: Result<T, ServiceError>) -> Result<T, McpError> {
        map_proxy_service_error(&self.failover_signal, result)
    }

    fn current_profile(&self) -> Option<String> {
        self.tool_profile
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Recompute skill/reference coverage against the secondary's local
    /// profile. Forwarding these calls would describe the primary's profile,
    /// which can differ after a local `use_profile` switch. Contract
    /// references and unknown units stay on the primary path.
    fn local_unit_result(&self, request: &CallToolRequestParams) -> Option<CallToolResult> {
        let name = request.arguments.as_ref()?.get("name")?.as_str()?;
        let profile = self.current_profile();
        let value = match request.name.as_ref() {
            "get_skill" => {
                let skill = spindle_adapters::get_skill(name)?;
                let output =
                    crate::tools::resolve_skill_output_for_profile(profile.as_deref(), skill);
                serde_json::to_value(output).ok()?
            }
            "get_reference" => {
                if name.starts_with("contracts/") {
                    return None;
                }
                let reference = spindle_adapters::get_reference(name)?;
                let output = crate::tools::resolve_reference_output_for_profile(
                    profile.as_deref(),
                    reference,
                );
                serde_json::to_value(output).ok()?
            }
            _ => return None,
        };
        Some(CallToolResult::structured(value))
    }

    /// Switch the secondary's local profile. The target must be fully covered
    /// by the primary's discovered catalog; otherwise the proxy returns a
    /// structured `profile_not_available` error and keeps the old profile.
    async fn switch_profile(
        &self,
        request: &CallToolRequestParams,
        context: &RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let Some(arguments) = request.arguments.clone() else {
            return Ok(crate::tools::profile_switch_error(
                "invalid_arguments",
                "",
                "use_profile requires a profile argument",
            ));
        };
        let input: UseProfileInput =
            match serde_json::from_value(serde_json::Value::Object(arguments)) {
                Ok(input) => input,
                Err(error) => {
                    return Ok(crate::tools::profile_switch_error(
                        "invalid_arguments",
                        "",
                        &format!("invalid use_profile arguments: {error}"),
                    ));
                }
            };
        if let Err(error) = crate::tools::validate_tool_profile(Some(&input.profile)) {
            return Ok(crate::tools::profile_switch_error(
                "unknown_profile",
                &input.profile,
                &format!("{error:#}"),
            ));
        }
        let Some(target) = crate::tools::profile_allowed_tool_names(&input.profile) else {
            return Ok(crate::tools::profile_switch_error(
                "unknown_profile",
                &input.profile,
                &format!("unknown tool profile '{}'", input.profile),
            ));
        };
        let primary = self.map_proxy_error(self.client.list_all_tools().await)?;
        let available: BTreeSet<&str> = primary.iter().map(|tool| tool.name.as_ref()).collect();
        let missing: Vec<&str> = target
            .iter()
            .copied()
            .filter(|name| !available.contains(name))
            .collect();
        if !missing.is_empty() {
            let preview = missing
                .iter()
                .take(10)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            let remainder = missing.len().saturating_sub(10);
            let suffix = if remainder > 0 {
                format!(" and {remainder} more")
            } else {
                String::new()
            };
            let message = format!(
                "profile '{}' is not available from this primary; a secondary proxy cannot expand beyond its primary (missing: {preview}{suffix})",
                input.profile,
            );
            return Ok(crate::tools::profile_switch_error(
                "profile_not_available",
                &input.profile,
                &message,
            ));
        }
        *self
            .tool_profile
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(input.profile.clone());
        let tools: Vec<String> = target.iter().map(|name| (*name).to_owned()).collect();
        let output = UseProfileOutput {
            profile: input.profile,
            tool_count: tools.len(),
            tools,
        };
        if let Err(error) = context.peer.notify_tool_list_changed().await {
            tracing::warn!(
                %error,
                "failed to send tools/list_changed after secondary profile switch"
            );
        }
        let value = serde_json::to_value(output)
            .map_err(|error| McpError::internal_error(error.to_string(), None))?;
        Ok(CallToolResult::structured(value))
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
        let profile = self.current_profile();
        let mut tools = self.map_proxy_error(self.client.list_all_tools().await)?;
        tools.retain(|tool| crate::tools::profile_has_tool(profile.as_deref(), &tool.name));
        let tools = tools
            .into_iter()
            .map(|tool| crate::tools::project_tool_discovery(profile.as_deref(), tool))
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    fn get_tool(&self, _name: &str) -> Option<Tool> {
        None
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if request.name == "use_profile" {
            return self.switch_profile(&request, &context).await;
        }
        if let Some(result) = self.local_unit_result(&request) {
            return Ok(result);
        }
        let profile = self.current_profile();
        if !crate::tools::profile_has_tool(profile.as_deref(), &request.name) {
            return Ok(CallToolResult::error(vec![rmcp::model::Content::text(
                format!("unknown tool: {}", request.name),
            )]));
        }
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
/// Connect a secondary to the primary and declare the secondary's model profile
/// so the internal listener returns that profile rather than the primary's.
pub async fn connect_to_primary_with_profile(
    addr: SocketAddr,
    tool_profile: Option<&str>,
) -> anyhow::Result<RunningService<RoleClient, ()>> {
    let mut uri = format!("http://{addr}/mcp");
    if let Some(profile) = tool_profile {
        uri.push_str("?profile=");
        uri.push_str(profile);
    }
    tracing::info!("secondary: connecting to primary at {uri}");

    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(uri),
    );
    ().serve(transport)
        .await
        .context("failed to connect to primary")
}

/// Try to acquire the primary election lock and verify the DB is reachable.
/// Returns `true` if the primary lock is available (primary is gone),
/// `false` if another process holds it.
/// Caller should exit cleanly — the MCP client will restart us as the new primary.
pub async fn try_promote(data_dir: &Path, db_path: &Path) -> anyhow::Result<bool> {
    let Some(_primary_lock) = crate::try_acquire_primary_lock(data_dir)? else {
        return Ok(false);
    };
    let _pool = spindle_adapters::SqlitePool::open(db_path).await?;
    Ok(true)
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
