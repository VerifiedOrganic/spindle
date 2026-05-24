use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use spindle_core::models::{
    AgentConfigStatus, AgentHealthSummary, AgentRoutingConfigOutput, AgentRoutingRuleSummary,
    AgentSummary, ConfigureAgentsOutput, ListAgentsOutput, TestAgentOutput,
};

use crate::agent_config::{
    AgentHealthStatus, AgentStatus, AgentStatusSummary, ConfiguredAgent, LoadedAgentConfig,
    RoutingRule, health_check_timeout, health_checks_enabled, load_agent_config,
};

pub const EMBEDDING_VERSION: &str = "token-hash-v1";
const EMBEDDING_DIMENSIONS: usize = 64;
const HTTP_RETRY_ATTEMPTS: usize = 3;
const HTTP_RETRY_BASE_DELAY_MS: u64 = if cfg!(test) { 10 } else { 1_000 };
const HEALTH_HEARTBEAT_INTERVAL_MS: u64 = 10 * 60 * 1_000;
const EXPLICIT_DRAFT_SYSTEM_APPENDIX: &str = "\
Explicit-rating drafting directive: this scene is intended for adult readers. \
When the prompt requests sexual material, keep it on page rather than fading to black \
or euphemizing the adult beats. Write with direct, concrete physical language at the \
requested explicit rating while preserving consent, adult age, continuity, character \
voice, and story tone.";

/// Default `--max-turns` for the grok-cli adapter. Long explicit scenes that
/// pull bible context via MCP and span multiple output continuations can
/// legitimately consume 50–150 messages; the headroom is intentional so that
/// drafts don't fail mid-scene under default config. Override per-agent via
/// `max_turns` in `spindle.toml` if you see truncation in practice.
const DEFAULT_GROK_MAX_TURNS: u32 = 200;

/// Appended to the system prompt for every grok-cli completion. Tells grok
/// the spindle MCP server is available for additional canon lookups, and that
/// it should not invoke write tools — spindle persists the returned text via
/// its own draft-receipt machinery.
///
/// IMPORTANT: this string includes the bootstrap instruction to activate the
/// project first. The grok-cli adapter runs in a fresh MCP session that does
/// not inherit the caller's `active_project_id`, so without this step the
/// spindle MCP read tools fail with "this MCP session has no active project".
const GROK_MCP_USAGE_HINT: &str = "\
You have access to the spindle MCP server. Bootstrap rule: BEFORE calling any \
other spindle tool, call `set_active_project` with the project_id given in \
the Spindle Context block below. Your MCP session starts with no active \
project — without this step, every other spindle call will error. After \
activation, call the read tools — search_bible, get_scene_context, \
get_character_snapshot, get_entity, get_chapter_briefing, \
find_scenes_referencing, get_writer_state — to pull only the canon you need \
beyond what the prompt already provides. Do not invoke any write tools; the \
spindle service persists your final output through its own draft pipeline. \
Return the prose only, with no preamble or commentary.";

#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub entity_table: String,
    pub title: String,
    pub excerpt: String,
    pub content: String,
}

#[derive(Clone)]
pub(crate) struct EmbeddingSession {
    backend: EmbeddingBackend,
    http_client: reqwest::Client,
}

#[derive(Clone)]
enum EmbeddingBackend {
    TokenHash,
    Http {
        version: String,
        agent: Box<AgentRuntime>,
    },
}

pub fn embed_text(input: &str) -> Vec<f64> {
    let tokens = tokenize(input);
    let mut vector = vec![0.0; EMBEDDING_DIMENSIONS];
    if tokens.is_empty() {
        return vector;
    }

    for (token, count) in token_counts(tokens) {
        let slot = hash_token(&token) % EMBEDDING_DIMENSIONS;
        vector[slot] += count as f64;
    }

    normalize(vector)
}

pub fn cosine_similarity(left: &[f64], right: &[f64]) -> f64 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    left.iter().zip(right).map(|(l, r)| l * r).sum()
}

pub fn tokenize(input: &str) -> Vec<String> {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_lowercase();
            if token.len() >= 2 { Some(token) } else { None }
        })
        .collect()
}

/// Structured pointers to the spindle work the request belongs to. Adapters
/// that run in their own MCP session (currently just `grok-cli`) use these to
/// tell the spawned agent which project / book / chapter / scene to activate
/// before pulling bible context. Adapters that talk directly to a stateless
/// HTTP endpoint (`openai-compatible`, etc.) ignore the context — their
/// prompt is fully pre-packed by the service layer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
}

impl RequestContext {
    pub fn is_empty(&self) -> bool {
        self.project_id.is_none()
            && self.book_id.is_none()
            && self.chapter_id.is_none()
            && self.scene_id.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRequest {
    pub route: String,
    pub prompt: String,
    /// Optional content rating (general/teen/mature/explicit). When set, the
    /// router picks a per-rating routing rule if one is configured for this
    /// route; otherwise it falls back to the default rule for the route.
    /// When None, the default rule is always used. Existing callers that
    /// construct a `ModelRequest` without this field through struct-update
    /// syntax `..Default::default()` get None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    /// Optional spindle context (project_id / book_id / chapter_id / scene_id)
    /// for adapters that need to bootstrap an external MCP session, such as
    /// `grok-cli`. Ignored by HTTP/local/CLI adapters whose prompts are
    /// already fully pre-packed by the service layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<RequestContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponse {
    pub adapter_kind: String,
    pub model_name: String,
    pub output: String,
    /// True when the model hit its token limit and the output is incomplete.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRoute {
    pub route_name: String,
    pub adapter_kind: String,
    pub model_name: String,
    pub purpose: String,
    pub system_prompt: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stop: Vec<String>,
}

/// A resolved route entry plus the rating it serves (if any). Returned by
/// `ModelRouter::list_route_bindings` so callers can disambiguate the two
/// `ModelRoute` entries that share a `route_name` when a per-rating override
/// is configured.
#[derive(Debug, Clone)]
pub struct RouteBinding {
    pub route: ModelRoute,
    pub rating: Option<String>,
}

#[derive(Debug, Clone)]
struct AgentRuntime {
    config: ConfiguredAgent,
    resolved_api_key: Option<String>,
    health: AgentHealthStatus,
}

#[derive(Debug, Clone)]
struct RuntimeConfig {
    source_path: Option<String>,
    /// Default route bindings keyed by route name. One entry per route.
    routes: BTreeMap<String, ModelRoute>,
    /// Rating-specific overrides keyed by `(route_name, normalized_rating)`.
    /// Overrides take precedence over the corresponding default route when a
    /// request specifies a matching rating; otherwise the default applies.
    rating_routes: BTreeMap<(String, String), ModelRoute>,
    routing_rules: Vec<RoutingRule>,
    agents: BTreeMap<String, AgentRuntime>,
    health_checks_enabled: bool,
    health_check_timeout: Duration,
}

#[derive(Debug, Clone)]
pub struct ModelRouter {
    runtime: Arc<RwLock<RuntimeConfig>>,
    http_client: reqwest::Client,
    health_generation: Arc<AtomicU64>,
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::from_loaded_config(
            load_agent_config(None).unwrap_or_else(|_| LoadedAgentConfig {
                source_path: None,
                agents: Vec::new(),
                routing: Vec::new(),
                health_check: crate::agent_config::default_health_check_config(),
            }),
        )
    }
}

impl ModelRouter {
    pub fn local_only() -> Self {
        Self::from_loaded_config(LoadedAgentConfig {
            source_path: None,
            agents: Vec::new(),
            routing: Vec::new(),
            health_check: crate::agent_config::default_health_check_config(),
        })
    }

    fn from_loaded_config(loaded: LoadedAgentConfig) -> Self {
        let http_client = reqwest::Client::new();
        let runtime = runtime_from_loaded_config(&http_client, loaded);
        let router = Self {
            runtime: Arc::new(RwLock::new(runtime)),
            http_client,
            health_generation: Arc::new(AtomicU64::new(0)),
        };
        router.refresh_health_heartbeat();
        router
    }

    pub fn list_routes(&self) -> Vec<ModelRoute> {
        let runtime = self.runtime.read().expect("model router read lock");
        let mut routes: Vec<ModelRoute> = runtime.routes.values().cloned().collect();
        routes.extend(runtime.rating_routes.values().cloned());
        routes
    }

    /// Like `list_routes` but each entry carries the optional `rating` the
    /// route was bound to. Default-rule bindings have `rating = None`;
    /// per-rating overrides carry the lowercase rating string. Use this when
    /// you need to disambiguate two ModelRoute entries with the same
    /// `route_name` (e.g. one default + one explicit override).
    pub fn list_route_bindings(&self) -> Vec<RouteBinding> {
        let runtime = self.runtime.read().expect("model router read lock");
        let mut out: Vec<RouteBinding> = runtime
            .routes
            .values()
            .cloned()
            .map(|route| RouteBinding {
                route,
                rating: None,
            })
            .collect();
        out.extend(
            runtime
                .rating_routes
                .iter()
                .map(|((_, rating), route)| RouteBinding {
                    route: route.clone(),
                    rating: Some(rating.clone()),
                }),
        );
        out
    }

    pub fn configure(&self, explicit_path: Option<&str>) -> anyhow::Result<ConfigureAgentsOutput> {
        let loaded = load_agent_config(explicit_path)?;
        let warnings = configuration_warnings(&loaded);
        let health_checks = health_checks_enabled(&loaded);
        let runtime = runtime_from_loaded_config(&self.http_client, loaded.clone());
        *self.runtime.write().expect("model router write lock") = runtime;
        self.refresh_health_heartbeat();
        Ok(ConfigureAgentsOutput {
            source_path: loaded.source_path,
            agents_loaded: loaded.agents.len(),
            routing_rules_loaded: loaded.routing.len(),
            health_checks_enabled: health_checks,
            warnings,
        })
    }

    pub fn list_agents(&self) -> ListAgentsOutput {
        let runtime = self.runtime.read().expect("model router read lock");
        ListAgentsOutput {
            source_path: runtime.source_path.clone(),
            health_checks_enabled: runtime.health_checks_enabled,
            agents: agent_statuses(&runtime)
                .into_iter()
                .map(|agent| AgentSummary {
                    id: agent.id,
                    name: agent.name,
                    provider: agent.provider,
                    endpoint: agent.endpoint,
                    model: agent.model,
                    max_context: agent.max_context,
                    ratings: agent.ratings,
                    quality_tier: agent.quality_tier,
                    capabilities: agent.capabilities,
                    notes: agent.notes,
                    status: match agent.status {
                        AgentStatus::Active => AgentConfigStatus::Active,
                        AgentStatus::MissingApiKey => AgentConfigStatus::MissingApiKey,
                        AgentStatus::Unreachable => AgentConfigStatus::Unreachable,
                    },
                    health: AgentHealthSummary {
                        checked: agent.health.checked,
                        reachable: agent.health.reachable,
                        status_code: agent.health.status_code,
                        message: agent.health.message,
                    },
                    route_names: agent.route_names,
                })
                .collect(),
        }
    }

    pub fn routing_config(&self) -> AgentRoutingConfigOutput {
        let runtime = self.runtime.read().expect("model router read lock");
        AgentRoutingConfigOutput {
            source_path: runtime.source_path.clone(),
            health_checks_enabled: runtime.health_checks_enabled,
            rules: runtime
                .routing_rules
                .iter()
                .map(|rule| {
                    let adapter_kind = runtime
                        .agents
                        .get(&rule.agent)
                        .map(|agent| adapter_kind_for_agent(&agent.config))
                        .unwrap_or_default();
                    let caller_should_send_brief = adapter_pulls_canon_via_mcp(&adapter_kind);
                    AgentRoutingRuleSummary {
                        route_name: rule.route.clone(),
                        agent_id: rule.agent.clone(),
                        fallback_agent_id: rule.fallback.clone(),
                        purpose: rule.purpose.clone(),
                        system_prompt: rule.system_prompt.clone(),
                        max_tokens: rule.max_tokens,
                        temperature: rule.temperature,
                        stop: rule.stop.clone(),
                        rating: rule.rating.clone(),
                        adapter_kind,
                        caller_should_send_brief,
                    }
                })
                .collect(),
        }
    }

    pub async fn test_agent(
        &self,
        agent_id: &str,
        prompt: Option<&str>,
    ) -> anyhow::Result<TestAgentOutput> {
        let runtime = self.runtime.read().expect("model router read lock").clone();
        let route_name = runtime
            .routing_rules
            .iter()
            .find(|rule| rule.agent == agent_id)
            .map(|rule| rule.route.clone())
            .ok_or_else(|| anyhow::anyhow!("unknown agent id: {agent_id}"))?;

        let response = self
            .complete(&ModelRequest {
                route: route_name.clone(),
                prompt: prompt
                    .unwrap_or("Write two short lines that confirm the model is reachable.")
                    .to_string(),
                rating: None,
                context: None,
            })
            .await?;
        Ok(TestAgentOutput {
            agent_id: agent_id.to_string(),
            route_name,
            adapter_kind: response.adapter_kind,
            model_name: response.model_name,
            health_checked: runtime.health_checks_enabled,
            output: response.output,
            truncated: response.truncated,
        })
    }

    pub async fn complete(&self, request: &ModelRequest) -> anyhow::Result<ModelResponse> {
        let runtime = self.runtime.read().expect("model router read lock").clone();
        let route = resolve_route(&runtime, &request.route, request.rating.as_deref())
            .ok_or_else(|| anyhow::anyhow!("unknown model route: {}", request.route))?;

        match route.adapter_kind.as_str() {
            "local" => Ok(ModelResponse {
                adapter_kind: route.adapter_kind.clone(),
                model_name: route.model_name.clone(),
                output: local_completion(route, &request.prompt),
                truncated: false,
            }),
            "cli" => self.run_cli(route, &request.prompt).await,
            "http" => {
                self.run_http(&runtime, route, request.rating.as_deref(), &request.prompt)
                    .await
            }
            "grok" => {
                self.run_grok(
                    &runtime,
                    route,
                    request.rating.as_deref(),
                    request.context.as_ref(),
                    &request.prompt,
                )
                .await
            }
            other => Err(anyhow::anyhow!("unsupported model adapter kind: {other}")),
        }
    }

    pub fn embedding_version(&self) -> String {
        self.embedding_session().version().to_string()
    }

    pub(crate) fn embedding_session(&self) -> EmbeddingSession {
        let runtime = self.runtime.read().expect("model router read lock").clone();
        EmbeddingSession::from_runtime(self.http_client.clone(), &runtime)
    }

    pub async fn embed_text(&self, input: &str) -> anyhow::Result<Vec<f64>> {
        self.embedding_session().embed_text(input).await
    }

    fn refresh_health_heartbeat(&self) {
        let generation = self.health_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let runtime = self.runtime.read().expect("model router read lock").clone();
        if !runtime.health_checks_enabled || runtime.agents.is_empty() {
            return;
        }
        let Some(interval) = health_heartbeat_interval() else {
            return;
        };
        spawn_health_heartbeat(
            Arc::downgrade(&self.runtime),
            self.health_generation.clone(),
            generation,
            self.http_client.clone(),
            interval,
        );
    }

    /// Continue a truncated completion by replaying the conversation with the
    /// partial assistant response and asking the model to pick up where it
    /// stopped. Returns the continuation fragment (not the full concatenated
    /// text — the caller is responsible for joining).
    ///
    /// `context` is honored by adapters that run in their own MCP session
    /// (grok-cli) so they can call `set_active_project` against the right
    /// project before pulling canon. HTTP adapters ignore it because the
    /// caller has already pre-packed the relevant context into the prompt.
    pub async fn complete_continuation(
        &self,
        route_name: &str,
        rating: Option<&str>,
        context: Option<&RequestContext>,
        original_prompt: &str,
        prior_output: &str,
    ) -> anyhow::Result<ModelResponse> {
        let runtime = self.runtime.read().expect("model router read lock").clone();
        let route = resolve_route(&runtime, route_name, rating)
            .ok_or_else(|| anyhow::anyhow!("unknown model route: {route_name}"))?;

        match route.adapter_kind.as_str() {
            "http" => {
                self.continue_http(&runtime, route, rating, original_prompt, prior_output)
                    .await
            }
            "grok" => {
                let prompt = build_grok_continuation_prompt(original_prompt, prior_output);
                self.run_grok(&runtime, route, rating, context, &prompt)
                    .await
            }
            other => Err(anyhow::anyhow!(
                "continuation not supported for adapter kind: {other}"
            )),
        }
    }

    async fn run_cli(&self, route: &ModelRoute, prompt: &str) -> anyhow::Result<ModelResponse> {
        let command = std::env::var("SPINDLE_MODEL_CLI_COMMAND")
            .map_err(|_| anyhow::anyhow!("SPINDLE_MODEL_CLI_COMMAND is not configured"))?;
        let output = tokio::process::Command::new(&command)
            .arg(&route.route_name)
            .arg(prompt)
            .output()
            .await?;
        if !output.status.success() {
            anyhow::bail!("cli model adapter failed with status {}", output.status);
        }
        Ok(ModelResponse {
            adapter_kind: route.adapter_kind.clone(),
            model_name: route.model_name.clone(),
            output: String::from_utf8(output.stdout)?.trim().to_string(),
            truncated: false,
        })
    }

    /// Drive the local `grok` CLI in single-turn headless mode. The agent's
    /// `endpoint` is the binary name or path (resolved via `PATH` when bare).
    /// The prompt is written to a tempfile and passed via `--prompt-file` to
    /// sidestep argv size limits and shell-escape edge cases. Grok inherits
    /// MCP server config from `~/.claude.json`, `~/.grok/config.toml`, and the
    /// project-scoped `.grok/config.toml` / `.mcp.json` discovered from the
    /// working directory, so it can call back into spindle's MCP server for
    /// canon lookups while drafting.
    async fn run_grok(
        &self,
        runtime: &RuntimeConfig,
        route: &ModelRoute,
        rating: Option<&str>,
        context: Option<&RequestContext>,
        prompt: &str,
    ) -> anyhow::Result<ModelResponse> {
        let agent = http_agent_for_route(runtime, route)?;
        let system_prompt = grok_system_prompt(route, rating, context);
        let max_turns = agent.config.max_turns.unwrap_or(DEFAULT_GROK_MAX_TURNS);

        let prompt_file = tempfile::Builder::new()
            .prefix("spindle-grok-")
            .suffix(".txt")
            .tempfile()
            .context("create grok prompt tempfile")?;
        std::fs::write(prompt_file.path(), prompt).context("write grok prompt tempfile")?;

        let binary = agent.config.endpoint.trim();
        if binary.is_empty() {
            anyhow::bail!(
                "grok-cli agent {} has empty endpoint; set it to the grok binary name or path",
                agent.config.id
            );
        }

        let mut cmd = tokio::process::Command::new(binary);
        cmd.arg("--prompt-file").arg(prompt_file.path());
        cmd.arg("--output-format").arg("json");
        cmd.arg("--always-approve");
        cmd.arg("--max-turns").arg(max_turns.to_string());
        if !system_prompt.is_empty() {
            cmd.arg("--system-prompt-override").arg(&system_prompt);
        }
        let model = agent.config.model.trim();
        if !model.is_empty() {
            cmd.arg("--model").arg(model);
        }
        if let Some(profile) = agent.config.agent_profile.as_deref() {
            cmd.arg("--agent").arg(profile);
        }
        if let Some(effort) = agent.config.effort.as_deref() {
            cmd.arg("--effort").arg(effort);
        }
        if let Some(cwd) = agent.config.working_directory.as_deref() {
            cmd.arg("--cwd").arg(cwd);
        }
        for rule in &agent.config.allow_tools {
            cmd.arg("--allow").arg(rule);
        }
        for rule in &agent.config.deny_tools {
            cmd.arg("--deny").arg(rule);
        }
        for extra in &agent.config.extra_args {
            cmd.arg(extra);
        }
        // Inherit stderr so grok's own tracing logs surface in real time during
        // long agentic loops. Capturing it would force us to buffer everything
        // until grok exits, which makes multi-minute drafts feel hung.
        cmd.stderr(Stdio::inherit());

        let output = cmd
            .output()
            .await
            .with_context(|| format!("failed to spawn grok CLI '{binary}'"))?;
        // Tempfile is held until after `output().await` returns; dropping
        // `prompt_file` here removes it from disk regardless of outcome.
        drop(prompt_file);

        let stdout = String::from_utf8(output.stdout).context("grok stdout was not valid UTF-8")?;
        let (text, truncated) = parse_grok_envelope(&stdout, output.status.success())
            .with_context(|| format!("grok-cli agent {}", agent.config.id))?;
        Ok(ModelResponse {
            adapter_kind: route.adapter_kind.clone(),
            model_name: route.model_name.clone(),
            output: text,
            truncated,
        })
    }

    async fn run_http(
        &self,
        runtime: &RuntimeConfig,
        route: &ModelRoute,
        rating: Option<&str>,
        prompt: &str,
    ) -> anyhow::Result<ModelResponse> {
        let agent = http_agent_for_route(runtime, route)?;
        let endpoint = format!(
            "{}/chat/completions",
            agent.config.endpoint.trim_end_matches('/')
        );
        let system_prompt = system_prompt_for_request(route, rating);
        let mut body = serde_json::json!({
            "model": agent.config.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": prompt }
            ],
            "max_tokens": route.max_tokens,
            "temperature": route.temperature,
            "stream": false,
        });
        if !route.stop.is_empty() {
            body["stop"] = serde_json::json!(route.stop);
        }
        let response = send_request_with_retry(|| {
            let mut request = self.http_client.post(&endpoint).json(&body);
            request = with_provider_headers(request, agent);
            request
        })
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {status} from {endpoint}: {error_body}");
        }
        let body: serde_json::Value = response.json().await?;
        let first_choice = body
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .and_then(|choices| choices.first());
        let output = first_choice
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| body.get("output").and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string();
        let truncated = first_choice
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(serde_json::Value::as_str)
            == Some("length");
        Ok(ModelResponse {
            adapter_kind: route.adapter_kind.clone(),
            model_name: route.model_name.clone(),
            output,
            truncated,
        })
    }

    /// Continue a previous completion that was truncated. Sends the original
    /// system/user exchange plus the partial assistant response, then asks the
    /// model to continue.
    async fn continue_http(
        &self,
        runtime: &RuntimeConfig,
        route: &ModelRoute,
        rating: Option<&str>,
        original_prompt: &str,
        prior_output: &str,
    ) -> anyhow::Result<ModelResponse> {
        let agent = http_agent_for_route(runtime, route)?;
        let endpoint = format!(
            "{}/chat/completions",
            agent.config.endpoint.trim_end_matches('/')
        );
        let system_prompt = system_prompt_for_request(route, rating);
        let mut body = serde_json::json!({
            "model": agent.config.model,
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": original_prompt },
                { "role": "assistant", "content": prior_output },
                { "role": "user", "content": "Continue exactly where you left off. Do not repeat any prior text." }
            ],
            "max_tokens": route.max_tokens,
            "temperature": route.temperature,
            "stream": false,
        });
        if !route.stop.is_empty() {
            body["stop"] = serde_json::json!(route.stop);
        }
        let response = send_request_with_retry(|| {
            let mut request = self.http_client.post(&endpoint).json(&body);
            request = with_provider_headers(request, agent);
            request
        })
        .await?;
        if !response.status().is_success() {
            let status = response.status();
            let error_body = response.text().await.unwrap_or_default();
            anyhow::bail!("HTTP {status} from {endpoint}: {error_body}");
        }
        let resp_body: serde_json::Value = response.json().await?;
        let first_choice = resp_body
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .and_then(|choices| choices.first());
        let output = first_choice
            .and_then(|choice| choice.get("message"))
            .and_then(|message| message.get("content"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| resp_body.get("output").and_then(serde_json::Value::as_str))
            .unwrap_or_default()
            .to_string();
        let truncated = first_choice
            .and_then(|choice| choice.get("finish_reason"))
            .and_then(serde_json::Value::as_str)
            == Some("length");
        Ok(ModelResponse {
            adapter_kind: route.adapter_kind.clone(),
            model_name: route.model_name.clone(),
            output,
            truncated,
        })
    }
}

impl EmbeddingSession {
    fn from_runtime(http_client: reqwest::Client, runtime: &RuntimeConfig) -> Self {
        let backend = configured_embedding_backend(runtime).unwrap_or(EmbeddingBackend::TokenHash);
        Self {
            backend,
            http_client,
        }
    }

    pub(crate) fn version(&self) -> &str {
        match &self.backend {
            EmbeddingBackend::TokenHash => EMBEDDING_VERSION,
            EmbeddingBackend::Http { version, .. } => version,
        }
    }

    pub(crate) async fn embed_text(&self, input: &str) -> anyhow::Result<Vec<f64>> {
        match &self.backend {
            EmbeddingBackend::TokenHash => Ok(embed_text(input)),
            EmbeddingBackend::Http { agent, .. } => {
                run_http_embedding(&self.http_client, agent, input).await
            }
        }
    }
}

pub(crate) async fn send_request_with_retry<F>(
    mut build_request: F,
) -> anyhow::Result<reqwest::Response>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    for attempt in 0..HTTP_RETRY_ATTEMPTS {
        match build_request().send().await {
            Ok(response) => {
                if should_retry_status(response.status()) && attempt + 1 < HTTP_RETRY_ATTEMPTS {
                    tokio::time::sleep(http_retry_delay(attempt)).await;
                    continue;
                }
                return Ok(response);
            }
            Err(error) => {
                if should_retry_error(&error) && attempt + 1 < HTTP_RETRY_ATTEMPTS {
                    tokio::time::sleep(http_retry_delay(attempt)).await;
                    continue;
                }
                return Err(error.into());
            }
        }
    }

    unreachable!("retry loop always returns or retries");
}

/// Resolve a logical route name + optional rating to a concrete `ModelRoute`.
///
/// Resolution order:
/// 1. If `rating` is `Some`, try `rating_routes[(route, normalized_rating)]`.
/// 2. Fall back to `routes[route]` (the default rule for this route).
/// 3. Return `None` if neither is configured.
fn resolve_route<'r>(
    runtime: &'r RuntimeConfig,
    route_name: &str,
    rating: Option<&str>,
) -> Option<&'r ModelRoute> {
    if let Some(raw_rating) = rating {
        let normalized = raw_rating.trim().to_ascii_lowercase();
        if !normalized.is_empty()
            && let Some(route) = runtime
                .rating_routes
                .get(&(route_name.to_string(), normalized))
        {
            return Some(route);
        }
    }
    runtime.routes.get(route_name)
}

fn system_prompt_for_request(route: &ModelRoute, rating: Option<&str>) -> String {
    let mut system_prompt = route.system_prompt.clone();
    let is_explicit_draft = route.route_name == "draft"
        && rating
            .map(|value| value.trim().eq_ignore_ascii_case("explicit"))
            .unwrap_or(false);
    if is_explicit_draft && !system_prompt.contains(EXPLICIT_DRAFT_SYSTEM_APPENDIX) {
        if !system_prompt.trim().is_empty() {
            system_prompt.push_str("\n\n");
        }
        system_prompt.push_str(EXPLICIT_DRAFT_SYSTEM_APPENDIX);
    }
    system_prompt
}

/// Compose the system prompt grok receives via `--system-prompt-override`.
/// Layers the route's base system prompt, the explicit-rating appendix when
/// applicable, the MCP usage hint, and the structured Spindle Context block
/// so grok can call `set_active_project` against the right project before any
/// other spindle MCP tool runs.
fn grok_system_prompt(
    route: &ModelRoute,
    rating: Option<&str>,
    context: Option<&RequestContext>,
) -> String {
    let mut prompt = system_prompt_for_request(route, rating);
    if !prompt.is_empty() {
        prompt.push_str("\n\n");
    }
    prompt.push_str(GROK_MCP_USAGE_HINT);
    if let Some(context) = context
        && !context.is_empty()
    {
        prompt.push_str("\n\n");
        prompt.push_str(&format_spindle_context_block(context));
    }
    prompt
}

/// Render the structured Spindle Context block grok reads to bootstrap its
/// MCP session. Emitted as a labeled block of `key: value` lines so grok
/// extracts the IDs verbatim — no parsing of prose.
fn format_spindle_context_block(context: &RequestContext) -> String {
    let mut out = String::from("Spindle Context (required for MCP bootstrap):\n");
    if let Some(project_id) = context.project_id.as_deref() {
        out.push_str(&format!(
            "- project_id: {project_id} (call set_active_project with this id FIRST)\n"
        ));
    }
    if let Some(book_id) = context.book_id.as_deref() {
        out.push_str(&format!("- book_id: {book_id}\n"));
    }
    if let Some(chapter_id) = context.chapter_id.as_deref() {
        out.push_str(&format!("- chapter_id: {chapter_id}\n"));
    }
    if let Some(scene_id) = context.scene_id.as_deref() {
        out.push_str(&format!("- scene_id: {scene_id}\n"));
    }
    out
}

/// Build the prompt grok sees for a continuation. Embeds the original prompt,
/// the prior assistant output, and an explicit "continue where you left off"
/// instruction. Grok's headless single-turn mode has no native concept of
/// resuming a prior assistant message, so the resumption signal lives in the
/// prompt body instead of the OpenAI `messages` array.
fn build_grok_continuation_prompt(original_prompt: &str, prior_output: &str) -> String {
    format!(
        "{original_prompt}\n\n\
         === Prior partial output (resume from here) ===\n\
         {prior_output}\n\
         === End prior output ===\n\n\
         Continue exactly where the prior output stopped. Do not repeat any \
         prior text. Do not add commentary, headers, or section labels — emit \
         only the continuing prose."
    )
}

/// Parse the JSON envelope grok writes to stdout under `--output-format json`.
///
/// Discrimination rules:
/// - `{"type": "error", "message": "..."}` → error, surface `message`
/// - any other shape with `text` → success, `truncated = stopReason != "EndTurn"`
/// - missing `text` field → error (envelope shape changed)
/// - non-zero exit without an error envelope → error (defensive)
///
/// Pure function so it can be unit-tested without spawning grok.
fn parse_grok_envelope(stdout: &str, exit_success: bool) -> anyhow::Result<(String, bool)> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        anyhow::bail!("grok CLI returned empty stdout (exit_success={exit_success})");
    }
    let body: serde_json::Value = serde_json::from_str(trimmed)
        .with_context(|| format!("grok stdout was not valid JSON: {trimmed}"))?;
    if body.get("type").and_then(|v| v.as_str()) == Some("error") {
        let message = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown grok error");
        anyhow::bail!("grok CLI error: {message}");
    }
    if !exit_success {
        anyhow::bail!(
            "grok CLI exited with non-zero status but envelope had no error field: {trimmed}"
        );
    }
    let text = body
        .get("text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("grok JSON envelope missing 'text' field: {trimmed}"))?
        .to_string();
    let stop_reason = body
        .get("stopReason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Ok((text, stop_reason != "EndTurn"))
}

fn should_retry_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn should_retry_error(error: &reqwest::Error) -> bool {
    error.is_connect() || error.is_timeout() || error.is_request()
}

fn http_retry_delay(attempt: usize) -> Duration {
    Duration::from_millis(HTTP_RETRY_BASE_DELAY_MS * (1_u64 << attempt))
}

fn with_provider_headers(
    request: reqwest::RequestBuilder,
    agent: &AgentRuntime,
) -> reqwest::RequestBuilder {
    let Some(api_key) = agent.resolved_api_key.as_deref() else {
        return request;
    };
    match agent.config.provider.as_str() {
        "anthropic" => request
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01"),
        "openrouter" => request
            .header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}"))
            .header("X-Title", "Spindle"),
        _ => request.header(reqwest::header::AUTHORIZATION, format!("Bearer {api_key}")),
    }
}

fn http_agent_for_route<'r>(
    runtime: &'r RuntimeConfig,
    route: &ModelRoute,
) -> anyhow::Result<&'r AgentRuntime> {
    if let Some(agent) = runtime.agents.get(&route.model_name) {
        return Ok(agent);
    }

    let Some(rule) = runtime
        .routing_rules
        .iter()
        .find(|rule| rule.route == route.route_name)
    else {
        anyhow::bail!("no configured routing rule for route {}", route.route_name);
    };
    runtime
        .agents
        .get(&rule.agent)
        .ok_or_else(|| anyhow::anyhow!("unknown configured agent: {}", rule.agent))
}

fn configured_embedding_backend(runtime: &RuntimeConfig) -> Option<EmbeddingBackend> {
    let route = runtime.routes.get("embedding")?;
    if route.adapter_kind != "http" {
        return None;
    }

    let rule = runtime
        .routing_rules
        .iter()
        .find(|rule| rule.route == "embedding")?;
    let agent = runtime.agents.get(&rule.agent)?;
    Some(EmbeddingBackend::Http {
        version: embedding_version_for_agent(agent),
        agent: Box::new(agent.clone()),
    })
}

fn embedding_version_for_agent(agent: &AgentRuntime) -> String {
    format!(
        "model-embedding-v1:{}:{}:{}",
        agent.config.provider,
        agent.config.endpoint.trim_end_matches('/'),
        agent.config.model
    )
}

async fn run_http_embedding(
    http_client: &reqwest::Client,
    agent: &AgentRuntime,
    input: &str,
) -> anyhow::Result<Vec<f64>> {
    let endpoint = format!("{}/embeddings", agent.config.endpoint.trim_end_matches('/'));
    let body = serde_json::json!({
        "model": agent.config.model,
        "input": input,
        "encoding_format": "float",
    });
    let response = send_request_with_retry(|| {
        let mut request = http_client.post(&endpoint).json(&body);
        request = with_provider_headers(request, agent);
        request
    })
    .await?;
    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        anyhow::bail!("HTTP {status} from {endpoint}: {error_body}");
    }
    let body: serde_json::Value = response.json().await?;
    let embedding = parse_embedding_response(&body)
        .ok_or_else(|| anyhow::anyhow!("embedding response missing vector data"))?;
    if embedding.is_empty() {
        anyhow::bail!("embedding response returned an empty vector");
    }
    Ok(normalize(embedding))
}

fn parse_embedding_response(body: &serde_json::Value) -> Option<Vec<f64>> {
    if let Some(values) = body
        .get("data")
        .and_then(serde_json::Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| item.get("embedding"))
        .and_then(serde_json::Value::as_array)
        .and_then(|values| parse_embedding_array(values))
    {
        return Some(values);
    }

    body.get("embedding")
        .and_then(serde_json::Value::as_array)
        .and_then(|values| parse_embedding_array(values))
}

fn parse_embedding_array(values: &[serde_json::Value]) -> Option<Vec<f64>> {
    let mut embedding = Vec::with_capacity(values.len());
    for value in values {
        embedding.push(value.as_f64()?);
    }
    Some(embedding)
}

fn runtime_from_loaded_config(
    client: &reqwest::Client,
    config: LoadedAgentConfig,
) -> RuntimeConfig {
    let has_external_agents = !config.agents.is_empty() && !config.routing.is_empty();
    let health_checks_enabled = health_checks_enabled(&config);
    let timeout = health_check_timeout(&config);
    let agents = config
        .agents
        .iter()
        .map(|agent| {
            (
                agent.id.clone(),
                AgentRuntime {
                    config: agent.clone(),
                    resolved_api_key: agent
                        .api_key_env
                        .as_deref()
                        .and_then(|name| std::env::var(name).ok()),
                    health: if health_checks_enabled {
                        endpoint_health(client, agent, timeout)
                    } else {
                        AgentHealthStatus {
                            checked: false,
                            reachable: true,
                            status_code: None,
                            message: None,
                        }
                    },
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut routes = default_routes();
    let mut rating_routes: BTreeMap<(String, String), ModelRoute> = BTreeMap::new();
    if has_external_agents {
        for rule in &config.routing {
            let Some(agent) = agents.get(&rule.agent) else {
                continue;
            };
            let purpose = rule
                .purpose
                .clone()
                .unwrap_or_else(|| format!("configured route for {}", agent.config.name));
            let resolved = ModelRoute {
                route_name: rule.route.clone(),
                adapter_kind: adapter_kind_for_agent(&agent.config),
                model_name: agent.config.id.clone(),
                system_prompt: rule
                    .system_prompt
                    .clone()
                    .unwrap_or_else(|| purpose.clone()),
                purpose,
                max_tokens: rule.max_tokens,
                temperature: rule.temperature,
                stop: rule.stop.clone(),
            };
            match rule.rating.as_deref() {
                Some(rating) => {
                    let normalized = rating.trim().to_ascii_lowercase();
                    rating_routes.insert((rule.route.clone(), normalized), resolved);
                }
                None => {
                    routes.insert(rule.route.clone(), resolved);
                }
            }
        }
    }

    RuntimeConfig {
        source_path: config.source_path,
        routes,
        rating_routes,
        routing_rules: config.routing,
        agents,
        health_checks_enabled,
        health_check_timeout: timeout,
    }
}

fn health_heartbeat_interval() -> Option<Duration> {
    match std::env::var("SPINDLE_HEALTH_CHECK_INTERVAL_MS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(0) => None,
            Ok(value) => Some(Duration::from_millis(value)),
            Err(_) => Some(Duration::from_millis(HEALTH_HEARTBEAT_INTERVAL_MS)),
        },
        Err(_) => Some(Duration::from_millis(HEALTH_HEARTBEAT_INTERVAL_MS)),
    }
}

fn spawn_health_heartbeat(
    runtime: Weak<RwLock<RuntimeConfig>>,
    generation: Arc<AtomicU64>,
    generation_id: u64,
    client: reqwest::Client,
    interval: Duration,
) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(interval);
            if generation.load(Ordering::SeqCst) != generation_id {
                break;
            }
            let Some(runtime_handle) = runtime.upgrade() else {
                break;
            };
            refresh_agent_health_once(&runtime_handle, &client, &generation, generation_id);
        }
    });
}

fn refresh_agent_health_once(
    runtime: &Arc<RwLock<RuntimeConfig>>,
    client: &reqwest::Client,
    generation: &Arc<AtomicU64>,
    generation_id: u64,
) {
    let snapshot = runtime.read().expect("model router read lock").clone();
    if !snapshot.health_checks_enabled {
        return;
    }
    let updated_health = snapshot
        .agents
        .iter()
        .map(|(id, agent)| {
            (
                id.clone(),
                endpoint_health(client, &agent.config, snapshot.health_check_timeout),
            )
        })
        .collect::<Vec<_>>();
    if generation.load(Ordering::SeqCst) != generation_id {
        return;
    }
    let agents_checked = updated_health.len();
    let unhealthy_agents = updated_health
        .iter()
        .filter(|(_, health)| !health.reachable)
        .count();

    let mut runtime = runtime.write().expect("model router write lock");
    if !runtime.health_checks_enabled {
        return;
    }
    for (agent_id, health) in updated_health {
        if let Some(agent) = runtime.agents.get_mut(&agent_id) {
            let previous = agent.health.clone();
            if previous.reachable != health.reachable
                || previous.status_code != health.status_code
                || previous.message != health.message
            {
                if health.reachable {
                    tracing::info!(
                        agent_id = %agent_id,
                        status_code = health.status_code,
                        generation_id,
                        "model route heartbeat marked agent reachable"
                    );
                } else {
                    tracing::warn!(
                        agent_id = %agent_id,
                        status_code = health.status_code,
                        error = health.message.as_deref().unwrap_or(""),
                        generation_id,
                        "model route heartbeat marked agent unreachable"
                    );
                }
            }
            agent.health = health;
        }
    }
    tracing::info!(
        agents_checked,
        unhealthy_agents,
        generation_id,
        "refreshed model route heartbeat"
    );
}

fn endpoint_health(
    client: &reqwest::Client,
    agent: &ConfiguredAgent,
    timeout: std::time::Duration,
) -> AgentHealthStatus {
    let url = format!("{}/models", agent.endpoint.trim_end_matches('/'));
    let client = client.clone();
    let response = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("health check runtime")
            .block_on(async move { client.get(url).timeout(timeout).send().await })
    })
    .join()
    .map_err(|_| anyhow::anyhow!("health check thread panicked"));
    match response {
        Ok(Ok(response)) => AgentHealthStatus {
            checked: true,
            reachable: response.status().is_success(),
            status_code: Some(response.status().as_u16()),
            message: None,
        },
        Ok(Err(error)) => AgentHealthStatus {
            checked: true,
            reachable: false,
            status_code: None,
            message: Some(error.to_string()),
        },
        Err(error) => AgentHealthStatus {
            checked: true,
            reachable: false,
            status_code: None,
            message: Some(error.to_string()),
        },
    }
}

fn adapter_kind_for_agent(agent: &ConfiguredAgent) -> String {
    // `grok-cli` is recognized by provider, not endpoint. The endpoint string
    // for a grok agent holds the binary name or path (e.g. "grok") rather than
    // an HTTP URL.
    if agent.provider == "grok-cli" {
        return "grok".to_string();
    }
    if agent.endpoint.starts_with("http://") || agent.endpoint.starts_with("https://") {
        "http".to_string()
    } else {
        "local".to_string()
    }
}

/// True when this adapter kind runs a child process that has its own MCP
/// access back to spindle and therefore can pull bible canon on demand.
/// Callers that build prompts for such routes should send a SHORT brief
/// rather than pre-packing context — any inlined canon is wasted tokens.
///
/// This is the generic taxonomy axis the scene-writer skill checks, so a
/// future Codex-CLI / Claude-CLI adapter just needs to be added here once
/// and every caller's prompt-building automatically adapts.
pub fn adapter_pulls_canon_via_mcp(adapter_kind: &str) -> bool {
    matches!(adapter_kind, "grok")
}

fn configuration_warnings(config: &LoadedAgentConfig) -> Vec<String> {
    let mut warnings = config
        .agents
        .iter()
        .filter_map(|agent| {
            agent.api_key_env.as_deref().and_then(|name| {
                std::env::var(name)
                    .err()
                    .map(|_| format!("agent {} is missing API key env {}", agent.id, name))
            })
        })
        .collect::<Vec<_>>();
    if health_checks_enabled(config) {
        warnings.push("endpoint health checks are enabled during configuration reload".to_string());
    }
    warnings
}

fn agent_statuses(runtime: &RuntimeConfig) -> Vec<AgentStatusSummary> {
    runtime
        .agents
        .values()
        .map(|agent| AgentStatusSummary {
            id: agent.config.id.clone(),
            name: agent.config.name.clone(),
            provider: agent.config.provider.clone(),
            endpoint: agent.config.endpoint.clone(),
            model: agent.config.model.clone(),
            max_context: agent.config.max_context,
            ratings: agent.config.ratings.clone(),
            quality_tier: agent.config.quality_tier.clone(),
            capabilities: agent.config.capabilities.clone(),
            notes: agent.config.notes.clone(),
            status: if agent.config.api_key_env.is_some() && agent.resolved_api_key.is_none() {
                AgentStatus::MissingApiKey
            } else if agent.health.checked && !agent.health.reachable {
                AgentStatus::Unreachable
            } else {
                AgentStatus::Active
            },
            health: agent.health.clone(),
            route_names: runtime
                .routing_rules
                .iter()
                .filter(|rule| rule.agent == agent.config.id)
                .map(|rule| rule.route.clone())
                .collect(),
        })
        .collect()
}

fn default_routes() -> BTreeMap<String, ModelRoute> {
    [
        ModelRoute {
            route_name: "draft".to_string(),
            adapter_kind: "local".to_string(),
            model_name: "scene-writer-local".to_string(),
            purpose: "scene drafting and alternative synthesis".to_string(),
            system_prompt: "scene drafting and alternative synthesis".to_string(),
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
        },
        ModelRoute {
            route_name: "review".to_string(),
            adapter_kind: "local".to_string(),
            model_name: "dual-persona-local".to_string(),
            purpose: "revision review and quality gates".to_string(),
            system_prompt: "revision review and quality gates".to_string(),
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
        },
        ModelRoute {
            route_name: "embedding".to_string(),
            adapter_kind: "local".to_string(),
            model_name: EMBEDDING_VERSION.to_string(),
            purpose: "semantic search embeddings".to_string(),
            system_prompt: "semantic search embeddings".to_string(),
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
        },
        ModelRoute {
            route_name: "import_extract".to_string(),
            adapter_kind: "local".to_string(),
            model_name: "import-extract-local".to_string(),
            purpose: "segment-level import extraction and candidate harvesting".to_string(),
            system_prompt: "segment-level import extraction and candidate harvesting".to_string(),
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
        },
        ModelRoute {
            route_name: "import_synthesize".to_string(),
            adapter_kind: "local".to_string(),
            model_name: "import-synthesize-local".to_string(),
            purpose: "cross-segment import synthesis and dossier assembly".to_string(),
            system_prompt: "cross-segment import synthesis and dossier assembly".to_string(),
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
        },
        ModelRoute {
            route_name: "import_validate".to_string(),
            adapter_kind: "local".to_string(),
            model_name: "import-validate-local".to_string(),
            purpose: "import ambiguity validation and review-item triage".to_string(),
            system_prompt: "import ambiguity validation and review-item triage".to_string(),
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
        },
    ]
    .into_iter()
    .map(|route| (route.route_name.clone(), route))
    .collect()
}

fn local_completion(route: &ModelRoute, prompt: &str) -> String {
    let compact_prompt = prompt
        .split_whitespace()
        .take(48)
        .collect::<Vec<_>>()
        .join(" ");
    match route.route_name.as_str() {
        "review" => format!("Literary critic and craft technician both reviewed: {compact_prompt}"),
        "draft" => format!("Local drafting adapter synthesized: {compact_prompt}"),
        "import_extract" => format!("Local import extraction adapter harvested: {compact_prompt}"),
        "import_synthesize" => {
            format!("Local import synthesis adapter assembled: {compact_prompt}")
        }
        "import_validate" => format!("Local import validation adapter triaged: {compact_prompt}"),
        _ => compact_prompt,
    }
}

fn token_counts(tokens: Vec<String>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for token in tokens {
        *counts.entry(token).or_insert(0) += 1;
    }
    counts
}

fn hash_token(token: &str) -> usize {
    let mut value: usize = 2166136261;
    for byte in token.bytes() {
        value ^= usize::from(byte);
        value = value.wrapping_mul(16777619);
    }
    value
}

fn normalize(mut vector: Vec<f64>) -> Vec<f64> {
    let magnitude = vector.iter().map(|value| value * value).sum::<f64>().sqrt();
    if magnitude == 0.0 {
        return vector;
    }
    for value in &mut vector {
        *value /= magnitude;
    }
    vector
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex, OnceLock};
    use std::thread;

    use super::*;

    fn health_env_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    #[test]
    fn embedding_prefers_related_queries() {
        let military = embed_text("loyal border regiment military garrison");
        let query = embed_text("military border soldiers");
        let romance = embed_text("forbidden romance ballroom dance");

        assert!(cosine_similarity(&military, &query) > cosine_similarity(&romance, &query));
    }

    #[tokio::test]
    async fn local_router_exposes_default_routes() {
        let router = ModelRouter::local_only();
        let output = router
            .complete(&ModelRequest {
                route: "review".to_string(),
                prompt: "Check this scene for prose drag".to_string(),
                rating: None,
                context: None,
            })
            .await
            .expect("local route works");

        assert_eq!(output.adapter_kind, "local");
        assert!(output.output.contains("reviewed"));
        assert_eq!(router.list_routes().len(), 6);
    }

    #[tokio::test]
    async fn embedding_route_defaults_to_token_hash_when_unconfigured() {
        let router = ModelRouter::local_only();

        assert_eq!(router.embedding_version(), EMBEDDING_VERSION);
        assert_eq!(
            router
                .embed_text("loyal border regiment")
                .await
                .expect("default embedding"),
            embed_text("loyal border regiment")
        );
    }

    #[test]
    fn configured_routes_override_default_routes() {
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            r####"
[health_check]
enabled = false

[[agents]]
id = "external-draft"
name = "External Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"

[[routing]]
route = "draft"
agent = "external-draft"
max_tokens = 1200
temperature = 0.7
stop = ["###"]
"####,
        )
        .expect("write config");

        let configured = router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");
        assert_eq!(configured.agents_loaded, 1);
        assert_eq!(configured.routing_rules_loaded, 1);
        assert!(!configured.health_checks_enabled);

        let routes = router.list_routes();
        let draft = routes
            .iter()
            .find(|route| route.route_name == "draft")
            .expect("draft route");
        assert_eq!(draft.adapter_kind, "http");
        assert_eq!(draft.model_name, "external-draft");
        assert_eq!(draft.max_tokens, Some(1200));
        assert_eq!(draft.temperature, Some(0.7));
        assert_eq!(draft.stop, vec!["###"]);
    }

    #[test]
    fn rating_aware_routing_resolves_explicit_to_override_agent() {
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            r####"
[health_check]
enabled = false

[[agents]]
id = "default-draft"
name = "Default Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"

[[agents]]
id = "uncensored"
name = "Uncensored Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11435/v1"
model = "uncensored-large"
ratings = ["safe", "mature", "explicit"]

[[routing]]
route = "draft"
agent = "default-draft"

[[routing]]
route = "draft"
agent = "uncensored"
rating = "explicit"
"####,
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let runtime = router
            .runtime
            .read()
            .expect("model router read lock")
            .clone();

        // Explicit rating must pick the override agent.
        let explicit_route =
            resolve_route(&runtime, "draft", Some("explicit")).expect("explicit route");
        assert_eq!(explicit_route.model_name, "uncensored");

        // Mature falls back to the default rule (no per-rating override).
        let mature_route =
            resolve_route(&runtime, "draft", Some("mature")).expect("mature falls back");
        assert_eq!(mature_route.model_name, "default-draft");

        // No rating context also falls back to the default rule.
        let none_route = resolve_route(&runtime, "draft", None).expect("default route");
        assert_eq!(none_route.model_name, "default-draft");
    }

    #[test]
    fn rating_aware_routing_is_backward_compatible_with_single_rule_configs() {
        // A spindle.toml that has only a default rule (no `rating` field on
        // any [[routing]] block) must continue to resolve as before.
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            r####"
[health_check]
enabled = false

[[agents]]
id = "default-draft"
name = "Default Draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"

[[routing]]
route = "draft"
agent = "default-draft"
"####,
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let runtime = router
            .runtime
            .read()
            .expect("model router read lock")
            .clone();

        // Even with a rating context, the default rule applies because no
        // override exists.
        let route =
            resolve_route(&runtime, "draft", Some("explicit")).expect("default still resolves");
        assert_eq!(route.model_name, "default-draft");
    }

    #[tokio::test]
    async fn http_router_sends_configured_system_prompt() {
        let (endpoint, captured, server) = spawn_http_capture_server(vec![(
            200,
            serde_json::json!({
                "choices": [{
                    "message": { "content": "ok" },
                    "finish_reason": "stop"
                }]
            })
            .to_string(),
        )]);
        let router = ModelRouter::local_only();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            format!(
                r####"
[health_check]
enabled = false

[[agents]]
id = "venice"
name = "Venice"
provider = "venice"
endpoint = "{endpoint}"
model = "venice-model"

[[routing]]
route = "draft"
agent = "venice"
purpose = "draft purpose"
system_prompt = "adult-route prompt: keep explicit-rated material on page."
max_tokens = 123
temperature = 0.7
"####
            ),
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let output = router
            .complete(&ModelRequest {
                route: "draft".to_string(),
                prompt: "Draft the scene.".to_string(),
                rating: None,
                context: None,
            })
            .await
            .expect("http route");

        assert_eq!(output.output, "ok");
        assert_eq!(server.join().expect("server join"), 1);
        let body = captured_json_body(&captured);
        assert_eq!(body["model"], "venice-model");
        assert_eq!(
            body["messages"][0]["content"],
            "adult-route prompt: keep explicit-rated material on page."
        );
        assert_eq!(body["messages"][1]["content"], "Draft the scene.");
    }

    #[tokio::test]
    async fn explicit_rating_http_request_uses_override_agent_and_prompt() {
        let (endpoint, captured, server) = spawn_http_capture_server(vec![(
            200,
            serde_json::json!({
                "choices": [{
                    "message": { "content": "explicit route ok" },
                    "finish_reason": "stop"
                }]
            })
            .to_string(),
        )]);
        let router = ModelRouter::local_only();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            format!(
                r####"
[health_check]
enabled = false

[[agents]]
id = "default-draft"
name = "Default Draft"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:9/v1"
model = "default-model"

[[agents]]
id = "venice"
name = "Venice Explicit"
provider = "venice"
endpoint = "{endpoint}"
model = "venice-explicit-model"

[[routing]]
route = "draft"
agent = "default-draft"
purpose = "default draft prompt"

[[routing]]
route = "draft"
agent = "venice"
rating = "explicit"
purpose = "explicit draft route"
system_prompt = "explicit-route prompt: do not fade out adult-rated material."
"####
            ),
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let output = router
            .complete(&ModelRequest {
                route: "draft".to_string(),
                prompt: "Continue the explicit-rated scene.".to_string(),
                rating: Some("explicit".to_string()),
                context: None,
            })
            .await
            .expect("explicit route");

        assert_eq!(output.output, "explicit route ok");
        assert_eq!(server.join().expect("server join"), 1);
        let body = captured_json_body(&captured);
        assert_eq!(body["model"], "venice-explicit-model");
        let system_prompt = body["messages"][0]["content"]
            .as_str()
            .expect("system prompt");
        assert!(system_prompt.contains("explicit-route prompt"));
        assert!(system_prompt.contains("Explicit-rating drafting directive"));
        assert!(system_prompt.contains("fading to black"));
        assert!(system_prompt.contains("consent"));
    }

    #[tokio::test]
    async fn explicit_rating_continuation_appends_default_drafting_directive() {
        let (endpoint, captured, server) = spawn_http_capture_server(vec![(
            200,
            serde_json::json!({
                "choices": [{
                    "message": { "content": "continued" },
                    "finish_reason": "stop"
                }]
            })
            .to_string(),
        )]);
        let router = ModelRouter::local_only();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            format!(
                r####"
[health_check]
enabled = false

[[agents]]
id = "venice"
name = "Venice Explicit"
provider = "venice"
endpoint = "{endpoint}"
model = "venice-explicit-model"

[[routing]]
route = "draft"
agent = "venice"
rating = "explicit"
system_prompt = "configured explicit route prompt."
"####
            ),
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let output = router
            .complete_continuation(
                "draft",
                Some("explicit"),
                None,
                "Original prompt.",
                "Prior output.",
            )
            .await
            .expect("explicit continuation");

        assert_eq!(output.output, "continued");
        assert_eq!(server.join().expect("server join"), 1);
        let body = captured_json_body(&captured);
        let system_prompt = body["messages"][0]["content"]
            .as_str()
            .expect("system prompt");
        assert!(system_prompt.contains("configured explicit route prompt."));
        assert!(system_prompt.contains("Explicit-rating drafting directive"));
        assert_eq!(body["messages"][2]["content"], "Prior output.");
    }

    #[tokio::test]
    async fn configured_embedding_route_uses_model_vectors() {
        let (endpoint, server) = spawn_http_test_server(vec![(
            200,
            serde_json::json!({
                "data": [{
                    "embedding": [3.0, 4.0]
                }]
            })
            .to_string(),
        )]);
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            format!(
                r####"
[health_check]
enabled = false

[[agents]]
id = "embed-http"
name = "Embedding HTTP"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "text-embedding-3-small"

[[routing]]
route = "embedding"
agent = "embed-http"
"####
            ),
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let embedding = router
            .embed_text("loyal border regiment")
            .await
            .expect("configured embedding");

        assert_eq!(
            router.embedding_version(),
            format!("model-embedding-v1:openai-compatible:{endpoint}:text-embedding-3-small")
        );
        assert_eq!(server.join().expect("server join"), 1);
        assert_eq!(embedding.len(), 2);
        assert!((embedding[0] - 0.6).abs() < 1e-6);
        assert!((embedding[1] - 0.8).abs() < 1e-6);
    }

    #[tokio::test]
    async fn configure_with_health_checks_does_not_require_current_runtime_block_on() {
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            r####"
[health_check]
enabled = true
timeout_ms = 25

[[agents]]
id = "external-draft"
name = "External Draft"
provider = "openai-compatible"
endpoint = "http://127.0.0.1:9/v1"
model = "mistral"

[[routing]]
route = "draft"
agent = "external-draft"
"####,
        )
        .expect("write config");

        let configured = router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");
        assert!(configured.health_checks_enabled);

        let agents = router.list_agents();
        let agent = agents.agents.first().expect("configured agent");
        assert_eq!(agent.status, AgentConfigStatus::Unreachable);
        assert!(agent.health.checked);
        assert!(!agent.health.reachable);
    }

    #[tokio::test]
    async fn health_heartbeat_refreshes_agent_status_between_checks() {
        let _guard = health_env_lock().lock().await;
        let (endpoint, server) = spawn_http_test_server(vec![
            (
                200,
                serde_json::json!({
                    "data": []
                })
                .to_string(),
            ),
            (
                503,
                serde_json::json!({
                    "error": { "message": "down" }
                })
                .to_string(),
            ),
        ]);
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        let disable_path = temp.path().join("spindle-disable.toml");
        std::fs::write(
            &config_path,
            format!(
                r####"
[health_check]
enabled = true
timeout_ms = 25

[[agents]]
id = "external-draft"
name = "External Draft"
provider = "openai-compatible"
endpoint = "{endpoint}"
model = "mistral"

[[routing]]
route = "draft"
agent = "external-draft"
"####
            ),
        )
        .expect("write config");
        std::fs::write(
            &disable_path,
            r####"
[health_check]
enabled = false
"####,
        )
        .expect("write disable config");

        unsafe {
            std::env::set_var("SPINDLE_HEALTH_CHECK_INTERVAL_MS", "25");
        }

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let initial_agents = router.list_agents();
        let initial_agent = initial_agents.agents.first().expect("configured agent");
        assert_eq!(initial_agent.status, AgentConfigStatus::Active);
        assert!(initial_agent.health.checked);
        assert!(initial_agent.health.reachable);

        tokio::time::sleep(Duration::from_millis(100)).await;

        let refreshed_agents = router.list_agents();
        let refreshed_agent = refreshed_agents.agents.first().expect("configured agent");
        assert_eq!(refreshed_agent.status, AgentConfigStatus::Unreachable);
        assert!(refreshed_agent.health.checked);
        assert!(!refreshed_agent.health.reachable);
        assert!(
            refreshed_agent.health.status_code == Some(503)
                || refreshed_agent.health.message.is_some()
        );

        router
            .configure(Some(&disable_path.display().to_string()))
            .expect("disable heartbeat");
        unsafe {
            std::env::remove_var("SPINDLE_HEALTH_CHECK_INTERVAL_MS");
        }

        assert_eq!(server.join().expect("server join"), 2);
    }

    #[tokio::test]
    async fn http_router_retries_after_rate_limit() {
        let (endpoint, server) = spawn_http_test_server(vec![
            (
                429,
                serde_json::json!({
                    "error": { "message": "rate limit" }
                })
                .to_string(),
            ),
            (
                200,
                serde_json::json!({
                    "choices": [{
                        "message": { "content": "retried successfully" },
                        "finish_reason": "stop"
                    }]
                })
                .to_string(),
            ),
        ]);
        let router = http_test_router(&endpoint);

        let output = router
            .complete(&ModelRequest {
                route: "review".to_string(),
                prompt: "Check this scene".to_string(),
                rating: None,
                context: None,
            })
            .await
            .expect("http route retries");

        assert_eq!(output.adapter_kind, "http");
        assert_eq!(output.output, "retried successfully");
        assert_eq!(server.join().expect("server join"), 2);
    }

    fn http_test_router(endpoint: &str) -> ModelRouter {
        let route = ModelRoute {
            route_name: "review".to_string(),
            adapter_kind: "http".to_string(),
            model_name: "test-model".to_string(),
            purpose: "revision review".to_string(),
            system_prompt: "revision review".to_string(),
            max_tokens: Some(256),
            temperature: Some(0.2),
            stop: Vec::new(),
        };
        let routing_rule = RoutingRule {
            route: "review".to_string(),
            agent: "test-agent".to_string(),
            fallback: None,
            purpose: Some("revision review".to_string()),
            system_prompt: None,
            max_tokens: Some(256),
            temperature: Some(0.2),
            stop: Vec::new(),
            rating: None,
        };
        let agent = ConfiguredAgent {
            id: "test-agent".to_string(),
            name: "Test Agent".to_string(),
            provider: "openai-compatible".to_string(),
            endpoint: endpoint.to_string(),
            model: "test-model".to_string(),
            api_key_env: None,
            max_context: None,
            ratings: Vec::new(),
            quality_tier: None,
            capabilities: Vec::new(),
            notes: None,
            effort: None,
            max_turns: None,
            agent_profile: None,
            working_directory: None,
            allow_tools: Vec::new(),
            deny_tools: Vec::new(),
            extra_args: Vec::new(),
        };
        let runtime = RuntimeConfig {
            source_path: None,
            routes: BTreeMap::from([(route.route_name.clone(), route)]),
            rating_routes: BTreeMap::new(),
            routing_rules: vec![routing_rule],
            agents: BTreeMap::from([(
                agent.id.clone(),
                AgentRuntime {
                    config: agent,
                    resolved_api_key: None,
                    health: AgentHealthStatus {
                        checked: false,
                        reachable: true,
                        status_code: None,
                        message: None,
                    },
                },
            )]),
            health_checks_enabled: false,
            health_check_timeout: Duration::from_millis(1500),
        };

        ModelRouter {
            runtime: Arc::new(RwLock::new(runtime)),
            http_client: reqwest::Client::new(),
            health_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    fn spawn_http_test_server(
        responses: Vec<(u16, String)>,
    ) -> (String, thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let address = listener.local_addr().expect("listener address");
        let handle = thread::spawn(move || {
            let mut handled = 0usize;
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = [0_u8; 8192];
                let _ = stream.read(&mut buffer);
                let status_text = match status {
                    200 => "OK",
                    429 => "Too Many Requests",
                    503 => "Service Unavailable",
                    _ => "Test Response",
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                stream.flush().expect("flush response");
                handled += 1;
            }
            handled
        });

        (format!("http://{address}/v1"), handle)
    }

    fn spawn_http_capture_server(
        responses: Vec<(u16, String)>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test listener");
        let address = listener.local_addr().expect("listener address");
        let captured = Arc::new(Mutex::new(Vec::new()));
        let captured_for_thread = Arc::clone(&captured);
        let handle = thread::spawn(move || {
            let mut handled = 0usize;
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().expect("accept request");
                let mut buffer = [0_u8; 8192];
                let count = stream.read(&mut buffer).expect("read request");
                let request = String::from_utf8_lossy(&buffer[..count]).to_string();
                let request_body = request
                    .split_once("\r\n\r\n")
                    .map(|(_, body)| body.to_string())
                    .unwrap_or_default();
                captured_for_thread
                    .lock()
                    .expect("captured requests lock")
                    .push(request_body);

                let status_text = match status {
                    200 => "OK",
                    429 => "Too Many Requests",
                    503 => "Service Unavailable",
                    _ => "Test Response",
                };
                let response = format!(
                    "HTTP/1.1 {status} {status_text}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
                stream.flush().expect("flush response");
                handled += 1;
            }
            handled
        });

        (format!("http://{address}/v1"), captured, handle)
    }

    fn captured_json_body(captured: &Arc<Mutex<Vec<String>>>) -> serde_json::Value {
        let requests = captured.lock().expect("captured requests lock");
        let body = requests.first().expect("captured request body");
        serde_json::from_str(body).expect("request body json")
    }

    fn grok_draft_route() -> ModelRoute {
        ModelRoute {
            route_name: "draft".to_string(),
            adapter_kind: "grok".to_string(),
            model_name: "grok-agent".to_string(),
            purpose: "scene drafting".to_string(),
            system_prompt: "You are the drafting model.".to_string(),
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
        }
    }

    #[test]
    fn parse_grok_envelope_extracts_text_and_endturn_is_not_truncated() {
        let stdout = r#"{
            "text": "pong",
            "stopReason": "EndTurn",
            "sessionId": "abc",
            "requestId": "def",
            "thought": "reasoning..."
        }"#;
        let (text, truncated) = parse_grok_envelope(stdout, true).expect("envelope");
        assert_eq!(text, "pong");
        assert!(!truncated);
    }

    #[test]
    fn parse_grok_envelope_marks_non_endturn_as_truncated() {
        let stdout = r#"{ "text": "partial", "stopReason": "MaxTokens" }"#;
        let (text, truncated) = parse_grok_envelope(stdout, true).expect("envelope");
        assert_eq!(text, "partial");
        assert!(truncated);
    }

    #[test]
    fn parse_grok_envelope_surfaces_error_envelope() {
        let stdout =
            r#"{"type":"error","message":"Internal error: \"max_turns exceeded: limit is 1\""}"#;
        let err = parse_grok_envelope(stdout, false).expect_err("error envelope");
        let msg = err.to_string();
        assert!(msg.contains("grok CLI error"), "got: {msg}");
        assert!(msg.contains("max_turns exceeded"), "got: {msg}");
    }

    #[test]
    fn parse_grok_envelope_rejects_non_zero_exit_without_error_field() {
        // Defensive: grok shouldn't ever do this, but if it does we don't want
        // to silently return whatever stub text the envelope had.
        let stdout = r#"{ "text": "incomplete", "stopReason": "EndTurn" }"#;
        let err = parse_grok_envelope(stdout, false).expect_err("non-zero exit");
        assert!(err.to_string().contains("non-zero status"), "got: {err}");
    }

    #[test]
    fn parse_grok_envelope_rejects_empty_stdout() {
        let err = parse_grok_envelope("   \n", true).expect_err("empty stdout");
        assert!(err.to_string().contains("empty stdout"), "got: {err}");
    }

    #[test]
    fn parse_grok_envelope_rejects_missing_text_field() {
        let stdout = r#"{ "stopReason": "EndTurn" }"#;
        let err = parse_grok_envelope(stdout, true).expect_err("missing text");
        assert!(err.to_string().contains("missing 'text'"), "got: {err}");
    }

    #[test]
    fn parse_grok_envelope_rejects_invalid_json() {
        let err = parse_grok_envelope("not json at all", true).expect_err("invalid json");
        assert!(err.to_string().contains("not valid JSON"), "got: {err}");
    }

    #[test]
    fn grok_system_prompt_layers_route_explicit_appendix_and_hint() {
        let route = grok_draft_route();
        let prompt = grok_system_prompt(&route, Some("explicit"), None);
        assert!(prompt.contains("You are the drafting model."));
        assert!(prompt.contains("Explicit-rating drafting directive"));
        assert!(prompt.contains("set_active_project"));
        assert!(prompt.contains("Do not invoke any write tools"));
    }

    #[test]
    fn grok_system_prompt_includes_spindle_context_block() {
        let route = grok_draft_route();
        let context = RequestContext {
            project_id: Some("proj-xyz".to_string()),
            book_id: Some("book-1".to_string()),
            chapter_id: Some("ch-04".to_string()),
            scene_id: Some("sc-021".to_string()),
        };
        let prompt = grok_system_prompt(&route, Some("explicit"), Some(&context));
        assert!(prompt.contains("Spindle Context"));
        assert!(prompt.contains("project_id: proj-xyz"));
        assert!(prompt.contains("book_id: book-1"));
        assert!(prompt.contains("chapter_id: ch-04"));
        assert!(prompt.contains("scene_id: sc-021"));
        assert!(prompt.contains("set_active_project with this id FIRST"));
    }

    #[test]
    fn grok_system_prompt_omits_context_block_when_empty() {
        let route = grok_draft_route();
        let empty = RequestContext::default();
        let prompt = grok_system_prompt(&route, None, Some(&empty));
        // The block header is what we look for: the hint itself mentions
        // "Spindle Context block below" so we can't search for that phrase.
        assert!(!prompt.contains("Spindle Context (required for MCP bootstrap)"));
        assert!(!prompt.contains("project_id:"));
    }

    #[test]
    fn grok_system_prompt_omits_explicit_appendix_for_non_draft_routes() {
        let mut route = grok_draft_route();
        route.route_name = "review".to_string();
        let prompt = grok_system_prompt(&route, Some("explicit"), None);
        assert!(!prompt.contains("Explicit-rating drafting directive"));
    }

    #[test]
    fn build_grok_continuation_prompt_embeds_prior_output_and_resume_instruction() {
        let prompt = build_grok_continuation_prompt(
            "Draft an explicit reunion scene.",
            "She crossed the room and reached for him,",
        );
        assert!(prompt.contains("Draft an explicit reunion scene."));
        assert!(prompt.contains("She crossed the room and reached for him,"));
        assert!(prompt.contains("Continue exactly where the prior output stopped"));
        assert!(prompt.contains("Do not repeat any prior text"));
    }

    #[test]
    fn adapter_kind_for_grok_cli_provider_overrides_endpoint_sniffing() {
        let agent = ConfiguredAgent {
            id: "grok-local".to_string(),
            name: "Local Grok".to_string(),
            provider: "grok-cli".to_string(),
            // Endpoint is a binary name, not an HTTP URL — verify provider wins.
            endpoint: "grok".to_string(),
            model: "grok-4".to_string(),
            api_key_env: None,
            max_context: None,
            ratings: vec!["explicit".to_string()],
            quality_tier: None,
            capabilities: Vec::new(),
            notes: None,
            effort: Some("high".to_string()),
            max_turns: Some(250),
            agent_profile: Some("spindle-scene-writer".to_string()),
            working_directory: None,
            allow_tools: Vec::new(),
            deny_tools: Vec::new(),
            extra_args: Vec::new(),
        };
        assert_eq!(adapter_kind_for_agent(&agent), "grok");
    }

    #[test]
    fn adapter_kind_for_non_grok_provider_still_sniffs_endpoint() {
        let mut agent = ConfiguredAgent {
            id: "x".to_string(),
            name: "x".to_string(),
            provider: "openai-compatible".to_string(),
            endpoint: "http://localhost:11434/v1".to_string(),
            model: "m".to_string(),
            api_key_env: None,
            max_context: None,
            ratings: Vec::new(),
            quality_tier: None,
            capabilities: Vec::new(),
            notes: None,
            effort: None,
            max_turns: None,
            agent_profile: None,
            working_directory: None,
            allow_tools: Vec::new(),
            deny_tools: Vec::new(),
            extra_args: Vec::new(),
        };
        assert_eq!(adapter_kind_for_agent(&agent), "http");
        agent.endpoint = "/usr/local/bin/local-model".to_string();
        assert_eq!(adapter_kind_for_agent(&agent), "local");
    }

    #[tokio::test]
    async fn grok_cli_routing_resolves_through_per_rating_rule() {
        // End-to-end-ish: parse a config file with an explicit grok-cli
        // override on `draft`, confirm the resolved route has adapter_kind
        // = "grok" so requests would dispatch to run_grok. We don't actually
        // spawn grok here.
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            r####"
[health_check]
enabled = false

[[agents]]
id = "default-draft"
name = "Default draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"

[[agents]]
id = "grok-local"
name = "Local Grok CLI"
provider = "grok-cli"
endpoint = "grok"
model = "grok-4"
ratings = ["safe", "mature", "explicit"]
effort = "high"
max_turns = 250
agent_profile = "spindle-scene-writer"

[[routing]]
route = "draft"
agent = "default-draft"

[[routing]]
route = "draft"
agent = "grok-local"
rating = "explicit"
"####,
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let routes = router.list_routes();
        let explicit_draft = routes
            .iter()
            .find(|r| r.route_name == "draft" && r.adapter_kind == "grok")
            .expect("explicit draft route bound to grok adapter");
        assert_eq!(explicit_draft.model_name, "grok-local");

        let default_draft = routes
            .iter()
            .find(|r| r.route_name == "draft" && r.adapter_kind == "http")
            .expect("default draft route stays on http adapter");
        assert_eq!(default_draft.model_name, "default-draft");

        // list_route_bindings disambiguates the two draft entries via rating.
        let bindings = router.list_route_bindings();
        let default_binding = bindings
            .iter()
            .find(|b| b.route.route_name == "draft" && b.rating.is_none())
            .expect("default draft binding");
        assert_eq!(default_binding.route.adapter_kind, "http");
        let explicit_binding = bindings
            .iter()
            .find(|b| b.route.route_name == "draft" && b.rating.as_deref() == Some("explicit"))
            .expect("explicit-rated draft binding");
        assert_eq!(explicit_binding.route.adapter_kind, "grok");

        // routing_config exposes adapter_kind + caller_should_send_brief so
        // skills can pick a prompt strategy without joining against list_agents.
        let routing = router.routing_config();
        let explicit_rule = routing
            .rules
            .iter()
            .find(|r| r.route_name == "draft" && r.rating.as_deref() == Some("explicit"))
            .expect("explicit rule surfaced");
        assert_eq!(explicit_rule.adapter_kind, "grok");
        assert!(explicit_rule.caller_should_send_brief);
        let default_rule = routing
            .rules
            .iter()
            .find(|r| r.route_name == "draft" && r.rating.is_none())
            .expect("default rule surfaced");
        assert_eq!(default_rule.adapter_kind, "http");
        assert!(!default_rule.caller_should_send_brief);
    }

    #[test]
    fn adapter_pulls_canon_via_mcp_classifies_grok_only() {
        // This is the generic taxonomy axis the scene-writer skill reads.
        // When future CLI-with-MCP adapters land, they should join this list
        // (and the skill's brief-vs-packed branching will adapt automatically).
        assert!(adapter_pulls_canon_via_mcp("grok"));
        assert!(!adapter_pulls_canon_via_mcp("http"));
        assert!(!adapter_pulls_canon_via_mcp("local"));
        assert!(!adapter_pulls_canon_via_mcp("cli"));
        assert!(!adapter_pulls_canon_via_mcp(""));
    }

    /// Wire-shape contract: `bible://config/routing` is rendered by
    /// `serde_json::to_string_pretty(&service.agent_routing_config())`, and
    /// the scene-writer skill reads the literal field names
    /// `caller_should_send_brief` and `adapter_kind` per rule. If anyone
    /// adds `#[serde(rename)]` to either field on `AgentRoutingRuleSummary`
    /// or removes a field, the skill's contract silently breaks. This test
    /// fails first, before that drift can ship.
    #[test]
    fn agent_routing_rule_summary_json_shape_carries_skill_contract_fields() {
        let router = ModelRouter::default();
        let temp = tempfile::tempdir().expect("temp dir");
        let config_path = temp.path().join("spindle.toml");
        std::fs::write(
            &config_path,
            r####"
[health_check]
enabled = false

[[agents]]
id = "default-draft"
name = "Default draft"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"

[[agents]]
id = "grok-local"
name = "Local Grok"
provider = "grok-cli"
endpoint = "grok"
model = "grok-build"
ratings = ["safe", "mature", "explicit"]

[[routing]]
route = "draft"
agent = "default-draft"

[[routing]]
route = "draft"
agent = "grok-local"
rating = "explicit"
"####,
        )
        .expect("write config");

        router
            .configure(Some(&config_path.display().to_string()))
            .expect("configure router");

        let routing = router.routing_config();
        let json = serde_json::to_string(&routing).expect("serialize routing config");

        // Field names the skill reads. A rename/removal breaks the contract.
        assert!(
            json.contains("\"caller_should_send_brief\""),
            "wire shape missing caller_should_send_brief: {json}"
        );
        assert!(
            json.contains("\"adapter_kind\""),
            "wire shape missing adapter_kind: {json}"
        );

        // Values must flip correctly per rule.
        let value: serde_json::Value = serde_json::from_str(&json).expect("re-parse routing json");
        let rules = value
            .get("rules")
            .and_then(|v| v.as_array())
            .expect("rules array");

        let explicit_rule = rules
            .iter()
            .find(|r| {
                r.get("route_name").and_then(|v| v.as_str()) == Some("draft")
                    && r.get("rating").and_then(|v| v.as_str()) == Some("explicit")
            })
            .expect("explicit rule on the wire");
        assert_eq!(
            explicit_rule.get("adapter_kind").and_then(|v| v.as_str()),
            Some("grok")
        );
        assert_eq!(
            explicit_rule
                .get("caller_should_send_brief")
                .and_then(|v| v.as_bool()),
            Some(true)
        );

        let default_rule = rules
            .iter()
            .find(|r| {
                r.get("route_name").and_then(|v| v.as_str()) == Some("draft")
                    && r.get("rating").is_none_or(|v| v.is_null())
            })
            .expect("default rule on the wire");
        assert_eq!(
            default_rule.get("adapter_kind").and_then(|v| v.as_str()),
            Some("http")
        );
        assert_eq!(
            default_rule
                .get("caller_should_send_brief")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    /// Wire-shape contract for `bible://system/model-routes` — same drift
    /// guard, this time on `ModelRouteSummary`. Constructs the summary
    /// directly (the service-level transformation is trivial) and asserts
    /// the JSON carries the fields and values the skill contract requires.
    #[test]
    fn model_route_summary_json_shape_carries_skill_contract_fields() {
        use spindle_core::models::ModelRouteSummary;

        let summary = ModelRouteSummary {
            route_name: "draft".to_string(),
            adapter_kind: "grok".to_string(),
            model_name: "grok-local".to_string(),
            purpose: "configured route for Local Grok".to_string(),
            rating: Some("explicit".to_string()),
            caller_should_send_brief: true,
        };
        let json = serde_json::to_string(&summary).expect("serialize route summary");

        assert!(
            json.contains("\"adapter_kind\":\"grok\""),
            "missing adapter_kind=grok: {json}"
        );
        assert!(
            json.contains("\"caller_should_send_brief\":true"),
            "missing caller_should_send_brief=true: {json}"
        );
        assert!(
            json.contains("\"rating\":\"explicit\""),
            "missing rating=explicit: {json}"
        );

        // And the false branch: stateless HTTP adapter, no rating override.
        let http_default = ModelRouteSummary {
            route_name: "draft".to_string(),
            adapter_kind: "http".to_string(),
            model_name: "default-draft".to_string(),
            purpose: "configured route for Default Draft".to_string(),
            rating: None,
            caller_should_send_brief: false,
        };
        let json = serde_json::to_string(&http_default).expect("serialize http route");
        assert!(
            json.contains("\"adapter_kind\":\"http\""),
            "missing http adapter_kind: {json}"
        );
        assert!(
            json.contains("\"caller_should_send_brief\":false"),
            "missing caller_should_send_brief=false: {json}"
        );
        // `rating: None` skips serialization via skip_serializing_if; should
        // NOT appear on the wire for default rules.
        assert!(
            !json.contains("\"rating\""),
            "rating=None should be omitted: {json}"
        );
    }

    /// Doc-drift guard for the scene-writer skill: the skill references
    /// `caller_should_send_brief` and `adapter_kind` by name. If anyone
    /// renames the field on the Rust side, the test above catches it; this
    /// one catches the reverse — someone editing the skill markdown and
    /// dropping the field reference (or using a stale name).
    #[test]
    fn scene_writer_skill_references_brief_contract_field_names() {
        // crates/spindle-adapters/src/ai.rs lives at
        // {workspace_root}/crates/spindle-adapters/src/ai.rs, so the skill
        // file is at ../../../skills/scene-writer/SKILL.md from CARGO_MANIFEST_DIR.
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let skill_path =
            std::path::Path::new(manifest_dir).join("../../skills/scene-writer/SKILL.md");
        let skill = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", skill_path.display()));
        assert!(
            skill.contains("caller_should_send_brief"),
            "scene-writer skill must reference caller_should_send_brief"
        );
        assert!(
            skill.contains("adapter_kind"),
            "scene-writer skill must reference adapter_kind"
        );
        assert!(
            skill.contains("bible://config/routing"),
            "scene-writer skill must point callers at bible://config/routing"
        );
    }
}
