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
    CommitSceneChangesOutput, ContextFormat, ContinueGenerationInput, ContinueGenerationOutput,
    CreateSavePointInput, CreateSavePointOutput, GetChapterBriefingInput, GetChapterBriefingOutput,
    GetSceneContextInput, ListAgentsOutput, ModelRouteSummary, RunDualPersonaReviewInput,
    RunDualPersonaReviewOutput, SaveSceneDraftInput, SaveSceneDraftOutput, SaveSummaryInput,
    SaveSummaryOutput, SceneContextBudgetMeta, SceneContextNovelLayer, SceneContextSceneLayer,
    TestAgentInput, TestAgentOutput,
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
                let transport = TokioChildProcess::new(
                    tokio::process::Command::new("cargo").configure(|command| {
                        command
                            .args(["run", "-q", "-p", "spindle-mcp"])
                            .current_dir(workspace_root);
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
                        budget_tokens: Some(3500),
                        recent_chapter_limit: Some(1),
                        token_budget: Some(3500),
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

            let chapter_plan = briefing.chapter_plan.map(|plan| ChapterPlanSnapshot {
                synopsis: plan.synopsis,
                pov_character_id: plan.pov_character_id,
                scenes: plan
                    .scenes
                    .into_iter()
                    .map(|scene| PlannedSceneSnapshot {
                        scene_order: scene.scene_order,
                        character_ids: scene.character_ids,
                    })
                    .collect(),
            });

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

    pub async fn save_summary(&self, input: &SaveSummaryInput) -> Result<SaveSummaryOutput> {
        self.call_tool("save_summary", input).await
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

    pub async fn resolve_draft_route(&self) -> Result<DraftRouteBinding> {
        let routes: Vec<ModelRouteSummary> = self
            .read_json_resource("bible://system/model-routes".to_string())
            .await?;
        let routing: AgentRoutingConfigOutput = self
            .read_json_resource("bible://config/routing".to_string())
            .await?;
        let agents: ListAgentsOutput = self
            .read_json_resource("bible://config/agents".to_string())
            .await?;

        let route = routes
            .into_iter()
            .find(|route| route.route_name == "draft")
            .context("missing model route named 'draft'")?;
        if route.adapter_kind == "local" {
            anyhow::bail!(
                "draft route resolves to local adapter {}; configure a real draft model before running the harness",
                route.model_name
            );
        }

        let draft_rule = routing
            .rules
            .iter()
            .find(|rule| rule.route_name == "draft")
            .context("missing routing rule for route 'draft'")?;
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

        let rule_count_for_agent = routing
            .rules
            .iter()
            .filter(|rule| rule.agent_id == agent.id)
            .count();
        if rule_count_for_agent != 1 || agent.route_names != ["draft".to_string()] {
            anyhow::bail!(
                "draft agent {} is not dedicated to the draft route; test_agent would be ambiguous",
                agent.id
            );
        }

        Ok(DraftRouteBinding {
            route_name: route.route_name,
            agent_id: agent.id.clone(),
        })
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
