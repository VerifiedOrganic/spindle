use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpindleConfigFile {
    #[serde(default)]
    pub agents: Vec<ConfiguredAgent>,
    #[serde(default)]
    pub routing: Vec<RoutingRule>,
    pub health_check: Option<HealthCheckConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfiguredAgent {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub api_key_env: Option<String>,
    pub max_context: Option<usize>,
    #[serde(default)]
    pub ratings: Vec<String>,
    pub quality_tier: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub notes: Option<String>,
    /// Grok CLI: reasoning/quality knob mapped to `--effort` (one of
    /// `low|medium|high|xhigh|max`). Ignored for non-grok providers.
    pub effort: Option<String>,
    /// Grok CLI: cap on agent loop turns mapped to `--max-turns`. Ignored for
    /// non-grok providers. Default when unset: 40.
    pub max_turns: Option<u32>,
    /// Grok CLI: agent profile mapped to `--agent`. Lets you anchor a grok
    /// session to a named skill such as `spindle-scene-writer`. Ignored for
    /// non-grok providers.
    pub agent_profile: Option<String>,
    /// Grok CLI: working directory mapped to `--cwd`. Controls which
    /// project-scoped `.grok/config.toml` and `.mcp.json` files grok loads in
    /// addition to the global config. When unset, grok uses spindle-mcp's
    /// current working directory. Ignored for non-grok providers.
    pub working_directory: Option<String>,
    /// Grok CLI: extra `--allow <RULE>` permission entries (additive — added
    /// per occurrence). Ignored for non-grok providers.
    #[serde(default)]
    pub allow_tools: Vec<String>,
    /// Grok CLI: extra `--deny <RULE>` permission entries (additive — added
    /// per occurrence). Ignored for non-grok providers.
    #[serde(default)]
    pub deny_tools: Vec<String>,
    /// Grok CLI: escape-hatch extra CLI args appended verbatim to the grok
    /// invocation. Use sparingly — flags modeled above should not be repeated
    /// here. Ignored for non-grok providers.
    #[serde(default)]
    pub extra_args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutingRule {
    pub route: String,
    pub agent: String,
    pub fallback: Option<String>,
    pub purpose: Option<String>,
    /// Optional system message sent as the first chat completion message.
    /// When omitted, the route purpose is used as the system message for
    /// backward compatibility.
    pub system_prompt: Option<String>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    #[serde(default)]
    pub stop: Vec<String>,
    /// Optional content rating this rule serves. When set, this rule only
    /// applies to requests whose content rating matches. When omitted, the
    /// rule acts as the default for the route — used when no rating-specific
    /// rule covers the request. Valid values: `general`, `teen`, `mature`,
    /// `explicit`.
    pub rating: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthCheckConfig {
    pub enabled: Option<bool>,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoadedAgentConfig {
    pub source_path: Option<String>,
    #[serde(default)]
    pub agents: Vec<ConfiguredAgent>,
    #[serde(default)]
    pub routing: Vec<RoutingRule>,
    pub health_check: HealthCheckConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentStatus {
    Active,
    MissingApiKey,
    Unreachable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentHealthStatus {
    pub checked: bool,
    pub reachable: bool,
    pub status_code: Option<u16>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentStatusSummary {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub max_context: Option<usize>,
    #[serde(default)]
    pub ratings: Vec<String>,
    pub quality_tier: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub notes: Option<String>,
    pub status: AgentStatus,
    pub health: AgentHealthStatus,
    #[serde(default)]
    pub route_names: Vec<String>,
}

pub fn default_config_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        paths.push(current_dir.join("spindle.toml"));
    }
    if let Some(home_dir) = dirs::home_dir() {
        paths.push(home_dir.join(".spindle").join("config.toml"));
    }
    paths
}

pub fn resolve_config_path(explicit: Option<&str>) -> anyhow::Result<Option<PathBuf>> {
    if let Some(explicit) = explicit {
        let path = PathBuf::from(explicit);
        if path.exists() {
            return Ok(Some(path));
        }
        anyhow::bail!("agent config file not found: {}", path.display());
    }

    Ok(default_config_candidates()
        .into_iter()
        .find(|path| path.exists()))
}

pub fn load_agent_config(explicit: Option<&str>) -> anyhow::Result<LoadedAgentConfig> {
    let Some(path) = resolve_config_path(explicit)? else {
        return Ok(LoadedAgentConfig {
            source_path: None,
            agents: Vec::new(),
            routing: Vec::new(),
            health_check: default_health_check_config(),
        });
    };

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read agent config {}", path.display()))?;
    let config: SpindleConfigFile =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_config(&config)?;
    Ok(LoadedAgentConfig {
        source_path: Some(path.display().to_string()),
        agents: config.agents,
        routing: config.routing,
        health_check: normalized_health_check_config(config.health_check),
    })
}

pub fn default_health_check_config() -> HealthCheckConfig {
    HealthCheckConfig {
        enabled: Some(false),
        timeout_ms: Some(1500),
    }
}

pub fn normalized_health_check_config(value: Option<HealthCheckConfig>) -> HealthCheckConfig {
    let default = default_health_check_config();
    let value = value.unwrap_or(default.clone());
    HealthCheckConfig {
        enabled: value.enabled.or(default.enabled),
        timeout_ms: value.timeout_ms.or(default.timeout_ms),
    }
}

pub fn health_checks_enabled(config: &LoadedAgentConfig) -> bool {
    config.health_check.enabled.unwrap_or(false)
}

pub fn health_check_timeout(config: &LoadedAgentConfig) -> Duration {
    Duration::from_millis(config.health_check.timeout_ms.unwrap_or(1500))
}

/// Grok CLI's documented `--effort` values. Kept in sync with `grok --help`
/// output so the loader rejects typos before grok itself does.
const GROK_EFFORT_VALUES: &[&str] = &["low", "medium", "high", "xhigh", "max"];

fn validate_config(config: &SpindleConfigFile) -> anyhow::Result<()> {
    let mut agent_ids = BTreeSet::new();
    for agent in &config.agents {
        if agent.id.trim().is_empty() {
            anyhow::bail!("agent id cannot be empty");
        }
        if !agent_ids.insert(agent.id.clone()) {
            anyhow::bail!("duplicate agent id: {}", agent.id);
        }
        if agent.endpoint.trim().is_empty() {
            anyhow::bail!("agent {} is missing endpoint", agent.id);
        }
        if agent.model.trim().is_empty() {
            anyhow::bail!("agent {} is missing model", agent.id);
        }
        if let Some(effort) = agent.effort.as_deref() {
            let normalized = effort.trim().to_ascii_lowercase();
            if !GROK_EFFORT_VALUES.contains(&normalized.as_str()) {
                anyhow::bail!(
                    "agent {} has unknown effort '{}'; expected one of {:?}",
                    agent.id,
                    effort,
                    GROK_EFFORT_VALUES
                );
            }
        }
        if let Some(max_turns) = agent.max_turns
            && max_turns == 0
        {
            anyhow::bail!("agent {} has max_turns = 0; must be >= 1", agent.id);
        }
        if agent.provider == "grok-cli" {
            // grok-specific knobs (effort, max_turns, agent_profile, allow/deny,
            // extra_args) only make sense for grok-cli agents. Flag them on
            // other providers so misconfiguration surfaces at load time rather
            // than silently being ignored at request time.
        } else {
            let mut stray = Vec::new();
            if agent.effort.is_some() {
                stray.push("effort");
            }
            if agent.max_turns.is_some() {
                stray.push("max_turns");
            }
            if agent.agent_profile.is_some() {
                stray.push("agent_profile");
            }
            if agent.working_directory.is_some() {
                stray.push("working_directory");
            }
            if !agent.allow_tools.is_empty() {
                stray.push("allow_tools");
            }
            if !agent.deny_tools.is_empty() {
                stray.push("deny_tools");
            }
            if !agent.extra_args.is_empty() {
                stray.push("extra_args");
            }
            if !stray.is_empty() {
                anyhow::bail!(
                    "agent {} sets grok-cli-only fields {:?} but provider is '{}'",
                    agent.id,
                    stray,
                    agent.provider
                );
            }
        }
    }

    let known_agents = config
        .agents
        .iter()
        .map(|agent| agent.id.as_str())
        .collect::<BTreeSet<_>>();
    // Routing rules are unique per (route, rating). A rule with `rating: None`
    // acts as the default for the route; at most one default per route. A
    // rule with `rating: Some(...)` overrides the default for that rating; at
    // most one such override per (route, rating) pair.
    let mut seen_routes: BTreeSet<(String, Option<String>)> = BTreeSet::new();
    let allowed_ratings = ["general", "teen", "mature", "explicit"];
    for rule in &config.routing {
        if rule.route.trim().is_empty() {
            anyhow::bail!("routing rule route cannot be empty");
        }
        if let Some(rating) = rule.rating.as_deref() {
            let normalized = rating.trim().to_ascii_lowercase();
            if !allowed_ratings.contains(&normalized.as_str()) {
                anyhow::bail!(
                    "routing rule for {} has unknown rating '{}'; expected one of {:?}",
                    rule.route,
                    rating,
                    allowed_ratings
                );
            }
        }
        let key = (
            rule.route.clone(),
            rule.rating
                .as_deref()
                .map(|r| r.trim().to_ascii_lowercase()),
        );
        if !seen_routes.insert(key) {
            match rule.rating.as_deref() {
                Some(rating) => anyhow::bail!(
                    "duplicate routing rule for route {} with rating {}",
                    rule.route,
                    rating
                ),
                None => anyhow::bail!(
                    "duplicate default routing rule for route {} (only one rule per route may omit `rating`)",
                    rule.route
                ),
            }
        }
        if !known_agents.contains(rule.agent.as_str()) {
            anyhow::bail!(
                "routing rule for {} references unknown agent {}",
                rule.route,
                rule.agent
            );
        }
        if let Some(fallback) = rule.fallback.as_deref()
            && !known_agents.contains(fallback)
        {
            anyhow::bail!(
                "routing rule for {} references unknown fallback agent {}",
                rule.route,
                fallback
            );
        }
        if let Some(temperature) = rule.temperature
            && !(0.0..=2.0).contains(&temperature)
        {
            anyhow::bail!(
                "routing rule for {} has invalid temperature {}",
                rule.route,
                temperature
            );
        }
    }

    Ok(())
}

pub fn default_config_template() -> &'static str {
    r#"# Spindle runtime config

[health_check]
enabled = false
timeout_ms = 1500

[[agents]]
id = "local-http"
name = "Local HTTP model"
provider = "openai-compatible"
endpoint = "http://localhost:11434/v1"
model = "mistral"
max_context = 32000
ratings = ["safe", "mature", "explicit"]
quality_tier = "primary"
capabilities = ["system_prompt"]

[[routing]]
route = "draft"
agent = "local-http"
system_prompt = "You are a fiction drafting agent."
max_tokens = 1800
temperature = 0.8

[[routing]]
route = "review"
agent = "local-http"
temperature = 0.2

[[routing]]
route = "embedding"
agent = "local-http"

[[routing]]
route = "import_extract"
agent = "local-http"
temperature = 0.1

[[routing]]
route = "import_synthesize"
agent = "local-http"
max_tokens = 2000
temperature = 0.3

[[routing]]
route = "import_validate"
agent = "local-http"
temperature = 0.1
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_duplicate_agent_ids() {
        let error = validate_config(&SpindleConfigFile {
            agents: vec![
                ConfiguredAgent {
                    id: "dup".to_string(),
                    name: "One".to_string(),
                    provider: "test".to_string(),
                    endpoint: "http://localhost:1/v1".to_string(),
                    model: "alpha".to_string(),
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
                },
                ConfiguredAgent {
                    id: "dup".to_string(),
                    name: "Two".to_string(),
                    provider: "test".to_string(),
                    endpoint: "http://localhost:2/v1".to_string(),
                    model: "beta".to_string(),
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
                },
            ],
            routing: Vec::new(),
            health_check: None,
        })
        .expect_err("duplicate agent ids should fail");

        assert!(error.to_string().contains("duplicate agent id"));
    }

    #[test]
    fn validates_unknown_routing_agent() {
        let error = validate_config(&SpindleConfigFile {
            agents: vec![ConfiguredAgent {
                id: "one".to_string(),
                name: "One".to_string(),
                provider: "test".to_string(),
                endpoint: "http://localhost:1/v1".to_string(),
                model: "alpha".to_string(),
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
            }],
            routing: vec![RoutingRule {
                route: "draft".to_string(),
                agent: "missing".to_string(),
                fallback: None,
                purpose: None,
                system_prompt: None,
                max_tokens: None,
                temperature: None,
                stop: Vec::new(),
                rating: None,
            }],
            health_check: None,
        })
        .expect_err("unknown routing agent should fail");

        assert!(error.to_string().contains("unknown agent"));
    }

    #[test]
    fn validates_temperature_bounds() {
        let error = validate_config(&SpindleConfigFile {
            agents: vec![ConfiguredAgent {
                id: "one".to_string(),
                name: "One".to_string(),
                provider: "test".to_string(),
                endpoint: "http://localhost:1/v1".to_string(),
                model: "alpha".to_string(),
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
            }],
            routing: vec![RoutingRule {
                route: "draft".to_string(),
                agent: "one".to_string(),
                fallback: None,
                purpose: None,
                system_prompt: None,
                max_tokens: None,
                temperature: Some(3.0),
                stop: Vec::new(),
                rating: None,
            }],
            health_check: None,
        })
        .expect_err("invalid temperature should fail");

        assert!(error.to_string().contains("invalid temperature"));
    }

    #[test]
    fn template_mentions_controls_and_health_checks() {
        let template = default_config_template();
        assert!(template.contains("[health_check]"));
        assert!(template.contains("max_tokens = 1800"));
        assert!(template.contains("temperature = 0.8"));
    }

    fn make_agent(id: &str) -> ConfiguredAgent {
        ConfiguredAgent {
            id: id.to_string(),
            name: id.to_string(),
            provider: "openai-compatible".to_string(),
            endpoint: "http://localhost:11434/v1".to_string(),
            model: "test".to_string(),
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
        }
    }

    fn make_routing(route: &str, agent: &str, rating: Option<&str>) -> RoutingRule {
        RoutingRule {
            route: route.to_string(),
            agent: agent.to_string(),
            fallback: None,
            purpose: None,
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
            rating: rating.map(|r| r.to_string()),
        }
    }

    #[test]
    fn allows_one_default_and_per_rating_overrides_for_same_route() {
        validate_config(&SpindleConfigFile {
            agents: vec![make_agent("default-agent"), make_agent("explicit-agent")],
            routing: vec![
                make_routing("draft", "default-agent", None),
                make_routing("draft", "explicit-agent", Some("explicit")),
            ],
            health_check: None,
        })
        .expect("default + per-rating overrides for the same route should validate");
    }

    #[test]
    fn rejects_two_defaults_for_the_same_route() {
        let error = validate_config(&SpindleConfigFile {
            agents: vec![make_agent("agent-a"), make_agent("agent-b")],
            routing: vec![
                make_routing("draft", "agent-a", None),
                make_routing("draft", "agent-b", None),
            ],
            health_check: None,
        })
        .expect_err("two default rules for the same route must fail");
        assert!(
            error.to_string().contains("duplicate default routing rule"),
            "unexpected error message: {error}"
        );
    }

    #[test]
    fn rejects_duplicate_rating_overrides_for_the_same_route() {
        let error = validate_config(&SpindleConfigFile {
            agents: vec![make_agent("agent-a"), make_agent("agent-b")],
            routing: vec![
                make_routing("draft", "agent-a", Some("explicit")),
                make_routing("draft", "agent-b", Some("explicit")),
            ],
            health_check: None,
        })
        .expect_err("two rules for the same (route, rating) must fail");
        assert!(
            error
                .to_string()
                .contains("duplicate routing rule for route draft with rating explicit"),
            "unexpected error message: {error}"
        );
    }

    #[test]
    fn rejects_unknown_rating_value() {
        let error = validate_config(&SpindleConfigFile {
            agents: vec![make_agent("agent-a")],
            routing: vec![make_routing("draft", "agent-a", Some("nc-17"))],
            health_check: None,
        })
        .expect_err("unknown rating values must be rejected");
        assert!(
            error.to_string().contains("unknown rating"),
            "unexpected error message: {error}"
        );
    }

    fn make_grok_agent(id: &str) -> ConfiguredAgent {
        ConfiguredAgent {
            id: id.to_string(),
            name: id.to_string(),
            provider: "grok-cli".to_string(),
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
            allow_tools: vec!["mcp__spindle__search_bible".to_string()],
            deny_tools: Vec::new(),
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn grok_cli_agent_with_all_optional_fields_validates() {
        validate_config(&SpindleConfigFile {
            agents: vec![make_grok_agent("grok-local")],
            routing: vec![make_routing("draft", "grok-local", Some("explicit"))],
            health_check: None,
        })
        .expect("grok-cli agent with full optional fields should validate");
    }

    #[test]
    fn validates_grok_effort_value_against_grok_help() {
        let mut agent = make_grok_agent("grok-local");
        agent.effort = Some("ultra".to_string()); // not a real grok level
        let err = validate_config(&SpindleConfigFile {
            agents: vec![agent],
            routing: Vec::new(),
            health_check: None,
        })
        .expect_err("invalid effort should fail");
        assert!(
            err.to_string().contains("unknown effort"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validates_grok_max_turns_zero_rejected() {
        let mut agent = make_grok_agent("grok-local");
        agent.max_turns = Some(0);
        let err = validate_config(&SpindleConfigFile {
            agents: vec![agent],
            routing: Vec::new(),
            health_check: None,
        })
        .expect_err("zero max_turns should fail");
        assert!(
            err.to_string().contains("max_turns = 0"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn validates_grok_only_fields_rejected_on_other_providers() {
        let mut agent = make_agent("openai-agent"); // provider = openai-compatible
        agent.agent_profile = Some("spindle-scene-writer".to_string());
        let err = validate_config(&SpindleConfigFile {
            agents: vec![agent],
            routing: Vec::new(),
            health_check: None,
        })
        .expect_err("grok-only fields on non-grok provider should fail");
        let msg = err.to_string();
        assert!(
            msg.contains("grok-cli-only fields") && msg.contains("agent_profile"),
            "unexpected error: {msg}"
        );
    }

    #[test]
    fn grok_cli_agent_parses_from_toml_with_extra_args() {
        // Round-trip a real TOML snippet to make sure all the new optional
        // fields land on ConfiguredAgent through serde defaults.
        let toml_src = r####"
[[agents]]
id = "grok-local"
name = "Local Grok"
provider = "grok-cli"
endpoint = "grok"
model = "grok-4"
ratings = ["explicit"]
effort = "high"
max_turns = 250
agent_profile = "spindle-scene-writer"
working_directory = "/tmp/spindle-work"
allow_tools = ["mcp__spindle__search_bible"]
deny_tools = ["mcp__spindle__delete_scene"]
extra_args = ["--check"]

[[routing]]
route = "draft"
agent = "grok-local"
rating = "explicit"
"####;
        let parsed: SpindleConfigFile = toml::from_str(toml_src).expect("parse toml");
        assert_eq!(parsed.agents.len(), 1);
        let agent = &parsed.agents[0];
        assert_eq!(agent.provider, "grok-cli");
        assert_eq!(agent.effort.as_deref(), Some("high"));
        assert_eq!(agent.max_turns, Some(250));
        assert_eq!(agent.agent_profile.as_deref(), Some("spindle-scene-writer"));
        assert_eq!(
            agent.working_directory.as_deref(),
            Some("/tmp/spindle-work")
        );
        assert_eq!(agent.allow_tools, vec!["mcp__spindle__search_bible"]);
        assert_eq!(agent.deny_tools, vec!["mcp__spindle__delete_scene"]);
        assert_eq!(agent.extra_args, vec!["--check"]);
        validate_config(&parsed).expect("validates");
    }

    #[test]
    fn default_candidates_include_project_and_global_paths() {
        let candidates = default_config_candidates();
        assert!(candidates.iter().any(|path| path.ends_with("spindle.toml")));
        assert!(
            candidates
                .iter()
                .any(|path| path.ends_with(".spindle/config.toml"))
        );
    }
}
