use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rmcp::{
    RoleClient, ServiceExt,
    model::{CallToolRequestParams, ReadResourceRequestParams, ResourceContents},
    service::RunningService,
    transport::{
        ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use spindle_core::models::{
    AgentRoutingConfigOutput, AnnotateSceneBeatsInput, AnnotateSceneBeatsOutput, BranchSummary,
    CheckConsistencyInput, CheckConsistencyOutput, CommitSceneChangesInput,
    CommitSceneChangesOutput, ContentRating, ContextFormat, ContinueGenerationInput,
    ContinueGenerationOutput, CreateSavePointInput, CreateSavePointOutput, GetChapterBriefingInput,
    GetChapterBriefingOutput, GetSceneContextInput, ListAgentsOutput, ModelRouteSummary,
    RunDualPersonaReviewInput, RunDualPersonaReviewOutput, SaveSceneDraftInput,
    SaveSceneDraftOutput, SaveSummaryInput, SaveSummaryOutput, SceneContextBudgetMeta,
    SceneContextNovelLayer, SceneContextSceneLayer, TestAgentInput, TestAgentOutput,
};

use crate::plan::{
    ChapterPlanSnapshot, ChapterSnapshot, PersistedScene, PlannedSceneSnapshot, ProjectSnapshot,
};
use crate::state::HarnessState;

#[derive(Debug, Clone)]
pub enum TransportConfig {
    Child {
        data_dir: Option<PathBuf>,
        config_path: Option<PathBuf>,
    },
    Http {
        url: String,
    },
}

pub struct McpHarnessClient {
    client: RunningService<RoleClient, ()>,
}

impl McpHarnessClient {
    pub async fn connect(config: &TransportConfig) -> Result<Self> {
        let client = match config {
            TransportConfig::Child {
                data_dir,
                config_path,
            } => {
                let workspace_root = workspace_root();
                let manifest_path = workspace_root.join("Cargo.toml");
                let transport = TokioChildProcess::new(
                    tokio::process::Command::new("cargo").configure(|command| {
                        command.args([
                            "run",
                            "-q",
                            "--manifest-path",
                            &manifest_path.to_string_lossy(),
                            "-p",
                            "spindle-mcp",
                        ]);
                        if let Some(data_dir) = data_dir {
                            command.env("SPINDLE_DATA_DIR", data_dir);
                        }
                        if let Some(config_path) = config_path {
                            command.env("SPINDLE_CONFIG", config_path);
                        }
                    }),
                )?;
                ().serve(transport)
                    .await
                    .context("failed to connect to child spindle-mcp process")?
            }
            TransportConfig::Http { url } => {
                let transport = StreamableHttpClientTransport::from_config(
                    StreamableHttpClientTransportConfig::with_uri(url.clone()),
                );
                ().serve(transport)
                    .await
                    .with_context(|| format!("failed to connect to spindle-mcp at {url}"))?
            }
        };

        Ok(Self { client })
    }

    pub async fn project_snapshot(&self, state: &HarnessState) -> Result<ProjectSnapshot> {
        let branches: Vec<BranchSummary> = self
            .read_json_resource(format!("bible://projects/{}/branches", state.project_id))
            .await?;
        let active_branch = branches
            .into_iter()
            .find(|branch| branch.is_active)
            .context("project has no active branch in branches resource")?;

        let summaries: Vec<ChapterSummaryResource> = self
            .read_json_resource(format!(
                "bible://projects/{}/chapter-summaries",
                state.project_id
            ))
            .await?;
        let summarized_chapters = summaries
            .into_iter()
            .filter(|summary| summary.book_number == state.book_number)
            .map(|summary| summary.chapter_number)
            .collect();

        let mut chapters = std::collections::BTreeMap::new();
        for chapter in &state.chapters {
            let resource: ChapterScenesResource = self
                .read_json_resource(format!(
                    "bible://projects/{}/chapters/{}/{}/scenes",
                    state.project_id, state.book_number, chapter.chapter_number
                ))
                .await
                .with_context(|| {
                    format!(
                        "failed to read scenes resource for chapter {}",
                        chapter.chapter_number
                    )
                })?;

            let first_scene = chapter
                .scenes
                .first()
                .context("chapter manifest must contain at least one scene")?;
            let briefing: GetChapterBriefingOutput = self
                .call_tool(
                    "get_chapter_briefing",
                    &GetChapterBriefingInput {
                        project_id: state.project_id.clone(),
                        book_number: state.book_number,
                        chapter_number: chapter.chapter_number,
                        scene_order: Some(first_scene.scene_order),
                        character_ids: first_scene.character_ids.clone(),
                        location_id: Some(first_scene.location_id.clone()),
                        format: Some(ContextFormat::Markdown),
                        budget_tokens: Some(12_000),
                        recent_chapter_limit: Some(1),
                        token_budget: Some(12_000),
                    },
                )
                .await
                .with_context(|| {
                    format!(
                        "failed to fetch chapter briefing for chapter {}",
                        chapter.chapter_number
                    )
                })?;

            let scenes = resource
                .scenes
                .into_iter()
                .map(|scene| {
                    (
                        scene.scene_order,
                        PersistedScene {
                            scene_id: scene.id,
                            scene_order: scene.scene_order,
                        },
                    )
                })
                .collect();

            let mut chapter_plan = None;
            if let Some(plan) = briefing.chapter_plan {
                let mut planned_scenes = Vec::new();
                for scene in plan.scenes {
                    let harness_scene = chapter
                        .scenes
                        .iter()
                        .find(|s| s.scene_order == scene.scene_order);
                    let scene_location = harness_scene.map(|s| s.location_id.clone());

                    let research_tags = scene.research_tags.clone();
                    let explicit_query = scene.explicit_query.clone();

                    let pack = self
                        .call_tool::<_, spindle_core::models::ResearchPackForSceneOutput>(
                            "research_pack_for_scene",
                            &spindle_core::models::ResearchPackForSceneInput {
                                project_id: state.project_id.clone(),
                                branch_id: Some(state.active_branch_id.clone()),
                                scene_summary: Some(scene.summary.clone()),
                                scene_location,
                                character_ids: scene.character_ids.clone(),
                                tags: research_tags.clone(),
                                explicit_query: explicit_query.clone(),
                                limit: Some(10),
                            },
                        )
                        .await
                        .unwrap_or(spindle_core::models::ResearchPackForSceneOutput {
                            sources: vec![],
                            notes: vec![],
                            claims: vec![],
                        });

                    let research_pack_empty =
                        pack.sources.is_empty() && pack.notes.is_empty() && pack.claims.is_empty();

                    let mut research_tags_matched = true;
                    if !research_tags.is_empty() {
                        let mut found_tag = false;
                        for t in &research_tags {
                            let t_lower = t.to_lowercase();
                            for s in &pack.sources {
                                if s.tags.iter().any(|st| st.to_lowercase() == t_lower) {
                                    found_tag = true;
                                    break;
                                }
                            }
                            if found_tag {
                                break;
                            }
                            for n in &pack.notes {
                                if n.tags.iter().any(|nt| nt.to_lowercase() == t_lower) {
                                    found_tag = true;
                                    break;
                                }
                            }
                            if found_tag {
                                break;
                            }
                            for c in &pack.claims {
                                if c.tags.iter().any(|ct| ct.to_lowercase() == t_lower) {
                                    found_tag = true;
                                    break;
                                }
                            }
                            if found_tag {
                                break;
                            }
                        }
                        research_tags_matched = found_tag;
                    }

                    planned_scenes.push(PlannedSceneSnapshot {
                        scene_order: scene.scene_order,
                        character_ids: scene.character_ids,
                        research_required: scene.research_required,
                        research_tags: scene.research_tags,
                        explicit_query,
                        research_pack_empty,
                        research_tags_matched,
                    });
                }

                chapter_plan = Some(ChapterPlanSnapshot {
                    synopsis: plan.synopsis,
                    pov_character_id: plan.pov_character_id,
                    scenes: planned_scenes,
                });
            }

            chapters.insert(
                chapter.chapter_number,
                ChapterSnapshot {
                    chapter_id: resource.chapter_id,
                    scenes,
                    chapter_plan,
                },
            );
        }

        Ok(ProjectSnapshot {
            active_branch_id: active_branch.branch_id,
            active_branch_name: active_branch.name,
            chapters,
            summarized_chapters,
        })
    }

    pub async fn get_chapter_briefing(
        &self,
        input: &GetChapterBriefingInput,
    ) -> Result<GetChapterBriefingOutput> {
        self.call_tool("get_chapter_briefing", input).await
    }

    pub async fn get_scene_context(
        &self,
        input: &GetSceneContextInput,
    ) -> Result<SceneContextEnvelope> {
        self.call_tool("get_scene_context", input).await
    }

    pub async fn save_scene_draft(
        &self,
        input: &SaveSceneDraftInput,
    ) -> Result<SaveSceneDraftOutput> {
        self.call_tool("save_scene_draft", input).await
    }

    pub async fn commit_scene_changes(
        &self,
        input: &CommitSceneChangesInput,
    ) -> Result<CommitSceneChangesOutput> {
        self.call_tool("commit_scene_changes", input).await
    }

    pub async fn annotate_scene_beats(
        &self,
        input: &AnnotateSceneBeatsInput,
    ) -> Result<AnnotateSceneBeatsOutput> {
        self.call_tool("annotate_scene_beats", input).await
    }

    pub async fn mine_scene_canon(
        &self,
        input: &spindle_core::models::MineSceneCanonInput,
    ) -> Result<spindle_core::models::MineSceneCanonOutput> {
        self.call_tool("mine_scene_canon", input).await
    }

    pub async fn replan_chapter(
        &self,
        input: &spindle_core::models::ReplanChapterInput,
    ) -> Result<spindle_core::models::ReplanChapterOutput> {
        self.call_tool("replan_chapter", input).await
    }

    pub async fn save_summary(&self, input: &SaveSummaryInput) -> Result<SaveSummaryOutput> {
        self.call_tool("save_summary", input).await
    }

    /// True when the chapter_summary row with `chapter_summary_id` still exists
    /// for (book, chapter) on the project's active branch. Used by the
    /// save-summary step's stale-artifact guard (defect item 2): a summary
    /// artifact's save_summary_output is idempotency proof only while the row
    /// it references is really persisted.
    pub async fn chapter_summary_row_exists(
        &self,
        project_id: &str,
        book_number: i32,
        chapter_number: i32,
        chapter_summary_id: &str,
    ) -> Result<bool> {
        let summaries: Vec<ChapterSummaryRowResource> = self
            .read_json_resource(format!("bible://projects/{project_id}/chapter-summaries"))
            .await?;
        Ok(summaries.iter().any(|summary| {
            summary.book_number == book_number
                && summary.chapter_number == chapter_number
                && summary.id == chapter_summary_id
        }))
    }

    pub async fn check_consistency(
        &self,
        input: &CheckConsistencyInput,
    ) -> Result<CheckConsistencyOutput> {
        self.call_tool("check_consistency", input).await
    }

    pub async fn run_dual_persona_review(
        &self,
        input: &RunDualPersonaReviewInput,
    ) -> Result<RunDualPersonaReviewOutput> {
        self.call_tool("run_dual_persona_review", input).await
    }

    pub async fn create_save_point(
        &self,
        input: &CreateSavePointInput,
    ) -> Result<CreateSavePointOutput> {
        self.call_tool("create_save_point", input).await
    }

    pub async fn test_agent(&self, input: &TestAgentInput) -> Result<TestAgentOutput> {
        self.call_tool("test_agent", input).await
    }

    pub async fn continue_generation(
        &self,
        input: &ContinueGenerationInput,
    ) -> Result<ContinueGenerationOutput> {
        self.call_tool("continue_generation", input).await
    }

    pub async fn research_pack_for_scene(
        &self,
        input: &spindle_core::models::ResearchPackForSceneInput,
    ) -> Result<spindle_core::models::ResearchPackForSceneOutput> {
        self.call_tool("research_pack_for_scene", input).await
    }

    pub async fn read_text_resource(&self, uri: String) -> Result<String> {
        let result = self
            .client
            .peer()
            .read_resource(ReadResourceRequestParams::new(uri.clone()))
            .await
            .with_context(|| format!("resource read failed: {uri}"))?;
        let text = first_resource_text(&result.contents)
            .with_context(|| format!("resource returned no text payload: {uri}"))?;
        Ok(text.to_string())
    }

    pub async fn resolve_draft_route(
        &self,
        rating: Option<ContentRating>,
    ) -> Result<DraftRouteBinding> {
        let routes: Vec<ModelRouteSummary> = self
            .read_json_resource("bible://system/model-routes".to_string())
            .await?;
        let routing: AgentRoutingConfigOutput = self
            .read_json_resource("bible://config/routing".to_string())
            .await?;
        let agents: ListAgentsOutput = self
            .read_json_resource("bible://config/agents".to_string())
            .await?;

        select_draft_route_binding(&routes, &routing, &agents, rating)
    }

    pub async fn call_tool<I, O>(&self, name: &str, input: &I) -> Result<O>
    where
        I: Serialize,
        O: DeserializeOwned,
    {
        let result = self
            .client
            .peer()
            .call_tool(
                CallToolRequestParams::new(name.to_string())
                    .with_arguments(rmcp::model::object(serde_json::to_value(input)?)),
            )
            .await
            .with_context(|| format!("tool call failed: {name}"))?;
        parse_call_tool_result(name, &result)
    }

    pub async fn read_json_resource<T>(&self, uri: String) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let result = self
            .client
            .peer()
            .read_resource(ReadResourceRequestParams::new(uri.clone()))
            .await
            .with_context(|| format!("resource read failed: {uri}"))?;
        let text = first_resource_text(&result.contents)
            .with_context(|| format!("resource returned no text payload: {uri}"))?;
        serde_json::from_str(text)
            .with_context(|| format!("resource payload was not valid JSON: {uri}"))
    }
}

fn select_draft_route_binding(
    routes: &[ModelRouteSummary],
    routing: &AgentRoutingConfigOutput,
    agents: &ListAgentsOutput,
    rating: Option<ContentRating>,
) -> Result<DraftRouteBinding> {
    let requested_rating = rating.map(|rating| rating.as_str().to_string());
    let draft_rule = requested_rating
        .as_deref()
        .and_then(|rating| {
            routing
                .rules
                .iter()
                .find(|rule| rule.route_name == "draft" && rule.rating.as_deref() == Some(rating))
        })
        .or_else(|| {
            routing
                .rules
                .iter()
                .find(|rule| rule.route_name == "draft" && rule.rating.is_none())
        })
        .context("missing routing rule for route 'draft'")?;

    let route = routes
        .iter()
        .find(|route| {
            route.route_name == "draft" && route.rating.as_deref() == draft_rule.rating.as_deref()
        })
        .or_else(|| {
            routes
                .iter()
                .find(|route| route.route_name == "draft" && route.rating.is_none())
        })
        .context("missing model route named 'draft'")?;
    if route.adapter_kind == "local" {
        anyhow::bail!(
            "draft route resolves to local adapter {}; configure a real draft model before running the harness",
            route.model_name
        );
    }

    let agent = agents
        .agents
        .iter()
        .find(|agent| agent.id == draft_rule.agent_id)
        .with_context(|| {
            format!(
                "routing rule for draft references unknown agent {}",
                draft_rule.agent_id
            )
        })?;
    if agent.status != spindle_core::models::AgentConfigStatus::Active {
        anyhow::bail!(
            "draft agent {} is not active ({:?})",
            agent.id,
            agent.status
        );
    }
    Ok(DraftRouteBinding {
        route_name: route.route_name.clone(),
        agent_id: agent.id.clone(),
        rating: draft_rule.rating.clone().or(requested_rating),
        caller_should_send_brief: route.caller_should_send_brief,
    })
}

fn parse_call_tool_result<T>(tool_name: &str, result: &rmcp::model::CallToolResult) -> Result<T>
where
    T: DeserializeOwned,
{
    let first_text = result
        .content
        .iter()
        .find_map(|content| content.as_text().map(|text| text.text.as_str()))
        .context("tool returned no text content")?;
    if let Some(error) = first_text.strip_prefix("Error: ") {
        anyhow::bail!("tool {tool_name} returned error: {error}");
    }
    serde_json::from_str(first_text)
        .with_context(|| format!("tool {tool_name} returned non-JSON payload: {first_text:?}"))
}

fn first_resource_text(contents: &[ResourceContents]) -> Option<&str> {
    contents.iter().find_map(|content| match content {
        ResourceContents::TextResourceContents { text, .. } => Some(text.as_str()),
        _ => None,
    })
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate is nested under workspace root")
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SceneContextEnvelope {
    pub standards: String,
    pub novel: SceneContextNovelLayer,
    pub scene: SceneContextSceneLayer,
    pub budget: SceneContextBudgetMeta,
}

#[derive(Debug, Clone)]
pub struct DraftRouteBinding {
    pub route_name: String,
    pub agent_id: String,
    pub rating: Option<String>,
    pub caller_should_send_brief: bool,
}

#[derive(Debug, serde::Deserialize)]
struct ChapterScenesResource {
    #[allow(dead_code)]
    active_branch_id: String,
    #[allow(dead_code)]
    book_number: i32,
    #[allow(dead_code)]
    chapter_number: i32,
    chapter_id: String,
    #[allow(dead_code)]
    title: String,
    scenes: Vec<ChapterSceneResource>,
}

#[derive(Debug, serde::Deserialize)]
struct ChapterSceneResource {
    id: String,
    scene_order: i32,
}

#[derive(Debug, serde::Deserialize)]
struct ChapterSummaryResource {
    book_number: i32,
    chapter_number: i32,
}

#[derive(serde::Deserialize)]
struct ChapterSummaryRowResource {
    id: String,
    book_number: i32,
    chapter_number: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use spindle_core::models::{
        AgentConfigStatus, AgentHealthSummary, AgentRoutingRuleSummary, AgentSummary,
    };

    fn route(name: &str, model: &str, rating: Option<&str>) -> ModelRouteSummary {
        ModelRouteSummary {
            route_name: name.to_string(),
            adapter_kind: "http".to_string(),
            model_name: model.to_string(),
            purpose: "drafting".to_string(),
            rating: rating.map(ToString::to_string),
            caller_should_send_brief: false,
        }
    }

    fn rule(agent_id: &str, rating: Option<&str>) -> AgentRoutingRuleSummary {
        rule_for_route("draft", agent_id, rating)
    }

    fn rule_for_route(
        route_name: &str,
        agent_id: &str,
        rating: Option<&str>,
    ) -> AgentRoutingRuleSummary {
        AgentRoutingRuleSummary {
            route_name: route_name.to_string(),
            agent_id: agent_id.to_string(),
            fallback_agent_id: None,
            purpose: Some("drafting".to_string()),
            system_prompt: None,
            max_tokens: None,
            temperature: None,
            stop: Vec::new(),
            rating: rating.map(ToString::to_string),
            adapter_kind: "http".to_string(),
            caller_should_send_brief: false,
        }
    }

    fn agent(id: &str) -> AgentSummary {
        AgentSummary {
            id: id.to_string(),
            name: id.to_string(),
            provider: "test".to_string(),
            endpoint: "http://localhost".to_string(),
            model: id.to_string(),
            max_context: None,
            ratings: Vec::new(),
            quality_tier: None,
            capabilities: Vec::new(),
            notes: None,
            status: AgentConfigStatus::Active,
            health: AgentHealthSummary {
                checked: false,
                reachable: true,
                status_code: None,
                message: None,
            },
            route_names: vec!["draft".to_string()],
        }
    }

    #[test]
    fn select_draft_route_binding_prefers_explicit_override() {
        let routes = vec![
            route("draft", "default-model", None),
            route("draft", "explicit-model", Some("explicit")),
        ];
        let routing = AgentRoutingConfigOutput {
            source_path: None,
            health_checks_enabled: false,
            rules: vec![
                rule("default-draft", None),
                rule("explicit-draft", Some("explicit")),
            ],
        };
        let agents = ListAgentsOutput {
            source_path: None,
            health_checks_enabled: false,
            agents: vec![agent("default-draft"), agent("explicit-draft")],
        };

        let binding =
            select_draft_route_binding(&routes, &routing, &agents, Some(ContentRating::Explicit))
                .unwrap();

        assert_eq!(binding.agent_id, "explicit-draft");
        assert_eq!(binding.rating.as_deref(), Some("explicit"));
    }

    #[test]
    fn select_draft_route_binding_falls_back_to_default_for_unmatched_rating() {
        let routes = vec![route("draft", "default-model", None)];
        let routing = AgentRoutingConfigOutput {
            source_path: None,
            health_checks_enabled: false,
            rules: vec![rule("default-draft", None)],
        };
        let agents = ListAgentsOutput {
            source_path: None,
            health_checks_enabled: false,
            agents: vec![agent("default-draft")],
        };

        let binding =
            select_draft_route_binding(&routes, &routing, &agents, Some(ContentRating::Mature))
                .unwrap();

        assert_eq!(binding.agent_id, "default-draft");
        assert_eq!(binding.rating.as_deref(), Some("mature"));
    }

    #[test]
    fn select_draft_route_binding_allows_shared_draft_research_agent() {
        let routes = vec![
            route("draft", "shared-model", None),
            route("research", "shared-model", None),
        ];
        let routing = AgentRoutingConfigOutput {
            source_path: None,
            health_checks_enabled: false,
            rules: vec![
                rule_for_route("draft", "grok-local", None),
                rule_for_route("research", "grok-local", None),
            ],
        };
        let mut shared_agent = agent("grok-local");
        shared_agent.route_names = vec!["draft".to_string(), "research".to_string()];
        let agents = ListAgentsOutput {
            source_path: None,
            health_checks_enabled: false,
            agents: vec![shared_agent],
        };

        let binding = select_draft_route_binding(&routes, &routing, &agents, None).unwrap();

        assert_eq!(binding.agent_id, "grok-local");
        assert_eq!(binding.route_name, "draft");
        assert_eq!(binding.rating, None);
    }
}
