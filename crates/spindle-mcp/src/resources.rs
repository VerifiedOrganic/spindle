use anyhow::Context;
use rmcp::model::{
    AnnotateAble, ListResourceTemplatesResult, ListResourcesResult, RawResource,
    RawResourceTemplate, ReadResourceResult, Resource, ResourceContents,
};
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use spindle_adapters::{get_reference, get_skill, list_references, list_skills};

use crate::json_utils::flatten_record_ids;

#[derive(Clone)]
pub struct ResourceRouter {
    service: SpindleService,
}

impl ResourceRouter {
    pub fn new(service: SpindleService) -> Self {
        Self { service }
    }

    pub async fn list_resources(&self) -> anyhow::Result<ListResourcesResult> {
        let mut resources: Vec<Resource> = list_skills()
            .iter()
            .map(|skill| {
                RawResource::new(
                    format!("bible://skills/{}", skill.name),
                    skill.name.to_string(),
                )
                .with_description(format!("Embedded skill: {}", skill.name))
                .with_mime_type("text/markdown")
                .no_annotation()
            })
            .collect();

        resources.extend(list_references().iter().map(|reference| {
            RawResource::new(
                format!("bible://references/{}", reference.name),
                reference.name.to_string(),
            )
            .with_description(format!("Craft reference: {}", reference.name))
            .with_mime_type("text/markdown")
            .no_annotation()
        }));

        resources.push(
            RawResource::new("bible://system/model-routes", "model routes")
                .with_description("Configured model routes. Resource-only read; use configure_agents tool to reload from config.")
                .with_mime_type("application/json")
                .no_annotation(),
        );
        resources.push(
            RawResource::new("bible://config/agents", "agent config")
                .with_description(
                    "Configured model agents without secrets. Tool equivalent: list_agents.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
        );
        resources.push(
            RawResource::new("bible://config/routing", "route config")
                .with_description("Configured model route assignments. Resource-only read.")
                .with_mime_type("application/json")
                .no_annotation(),
        );

        resources.push(
            RawResource::new("bible://projects", "all projects")
                .with_description(
                    "List all projects. Tool equivalent: list_projects. Resource provides \
                     cached project list for stable browsing; tool is preferred for dynamic \
                     operations.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
        );

        for project_id in self.service.list_project_ids().await? {
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/books"),
                    format!("project {project_id} books"),
                )
                .with_description("Books with chapter/scene counts. Tool equivalent: list_book_chapters returns chapters for a specific book. Use create_book to add books.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/chapters"),
                    format!("project {project_id} chapters"),
                )
                .with_description("Chapters with nested ordered scene spines. Resource-only read; use create_chapter tool to add chapters.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/characters"),
                    format!("project {project_id} characters"),
                )
                .with_description("Character entity list. Resource-only read; use create_character or update_entity tools to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/locations"),
                    format!("project {project_id} locations"),
                )
                .with_description("Location entity list. Resource-only read; use create_location or update_entity tools to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/world-rules"),
                    format!("project {project_id} world rules"),
                )
                .with_description(
                    "World rule list. Resource-only read; use create_world_rule tool to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/factions"),
                    format!("project {project_id} factions"),
                )
                .with_description(
                    "Faction entity list. Resource-only read; use create_faction tool to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/plot-lines"),
                    format!("project {project_id} plot lines"),
                )
                .with_description(
                    "Plot line list. Resource-only read; use create_plot_line tool to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/conflicts"),
                    format!("project {project_id} conflicts"),
                )
                .with_description(
                    "Paginated conflict list (first page by default). Use the conflicts resource template for page navigation, or create_conflict to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/themes"),
                    format!("project {project_id} themes"),
                )
                .with_description("Theme list. Resource-only read; use create_theme tool to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/motifs"),
                    format!("project {project_id} motifs"),
                )
                .with_description("Motif list. Resource-only read; use create_motif tool to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/narrative-promises"),
                    format!("project {project_id} narrative promises"),
                )
                .with_description("Narrative promise list. Resource-only read; use create_narrative_promise or update_promise_status tools to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/pacing/overview"),
                    format!("project {project_id} pacing overview"),
                )
                .with_description("Pacing overview with arc progress and constraints. Resource-only read; use create_pacing_config or set_arc_pacing_constraints tools to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/chapter-summaries"),
                    format!("project {project_id} chapter summaries"),
                )
                .with_description(
                    "Saved chapter summaries. Resource-only read; use save_summary tool to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/research-log"),
                    format!("project {project_id} research log"),
                )
                .with_description("Paginated research log entries. Use the research-log resource template for page navigation, or the research_query tool for new queries.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/reader-contract"),
                    format!("project {project_id} reader contract"),
                )
                .with_description("The active-branch reader contract. Resource-only read.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/branches"),
                    format!("project {project_id} branches"),
                )
                .with_description("Branch list for the project. Resource-only read; write operations are create_branch and switch_branch tools.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/continuity/health"),
                    format!("project {project_id} continuity health"),
                )
                .with_description("Continuity health summary for the active branch, including validator cache state, branch lineage, orphaned temporal interventions, and duplicate canonical fact keys.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/future-knowledge"),
                    format!("project {project_id} future knowledge"),
                )
                .with_description("Paginated future knowledge records (first page by default). Use the future-knowledge resource template for page navigation, or create_future_knowledge to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/timeline-events"),
                    format!("project {project_id} timeline events"),
                )
                .with_description("Paginated timeline event list (first page by default). Use the timeline-events resource template for page navigation, or create_timeline_event to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/timeline-graph/mermaid"),
                    format!("project {project_id} timeline graph"),
                )
                .with_description("Markdown Mermaid graph of project branches, save points, timeline events, and temporal interventions.")
                .with_mime_type("text/markdown")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/temporal-interventions"),
                    format!("project {project_id} temporal interventions"),
                )
                .with_description("Paginated temporal intervention list (first page by default). Use the temporal-interventions resource template for page navigation, or create_temporal_intervention to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/system-overlays"),
                    format!("project {project_id} system overlays"),
                )
                .with_description("System overlay list. Resource-only read; use create_system_overlay tool to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/dual-persona-reviews"),
                    format!("project {project_id} dual persona reviews"),
                )
                .with_description("Paginated dual persona review list (first page by default). Use the dual-persona-reviews resource template for page navigation, or run_dual_persona_review to create reviews.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/relationships"),
                    format!("project {project_id} relationships"),
                )
                .with_description("Paginated relationship list for the active branch (first page by default). Use the relationships resource template for page navigation, or create_relationship/update_relationship to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/character-arcs"),
                    format!("project {project_id} character arcs"),
                )
                .with_description("Character arc list with pacing. Resource-only read; use create_character_arc tool to write.")
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/religions"),
                    format!("project {project_id} religions"),
                )
                .with_description(
                    "Religion entity list. Resource-only read; use create_religion tool to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/economies"),
                    format!("project {project_id} economies"),
                )
                .with_description(
                    "Economy entity list. Resource-only read; use create_economy tool to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );
            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/terms"),
                    format!("project {project_id} terms"),
                )
                .with_description(
                    "Glossary term list. Resource-only read; use create_term tool to write.",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            );

            resources.push(
                RawResource::new(
                    format!("bible://projects/{project_id}/imports"),
                    format!("project {project_id} import sessions"),
                )
                .with_description("Import session summaries. Resource-only read; use import_manuscript tool to create sessions and import_status tool for dynamic queries.")
                .with_mime_type("application/json")
                .no_annotation(),
            );

            let imports = self
                .service
                .read_project_resource(&project_id, "imports")
                .await?;
            let sessions: Vec<spindle_core::models::ImportSessionSummary> =
                serde_json::from_value(imports).unwrap_or_default();
            for session in sessions {
                let session_id = session.session_id;
                resources.push(
                    RawResource::new(
                        format!("bible://projects/{project_id}/imports/{session_id}"),
                        format!("import {session_id} summary"),
                    )
                    .with_description(format!(
                        "Import session {session_id} summary. Tool equivalent: import_status."
                    ))
                    .with_mime_type("application/json")
                    .no_annotation(),
                );
                for (suffix, label) in [
                    ("summary", "summary"),
                    ("structure", "structure"),
                    ("entity-extraction", "entity extraction"),
                    ("entity-consolidation", "entity consolidation"),
                    ("characters", "characters"),
                    ("world", "world"),
                    ("narrative", "narrative"),
                    ("resume-snapshot", "resume snapshot"),
                    ("review-items", "review items"),
                    ("hydration-report", "hydration report"),
                ] {
                    resources.push(
                        RawResource::new(
                            format!("bible://projects/{project_id}/imports/{session_id}/{suffix}"),
                            format!("import {session_id} {label}"),
                        )
                        .with_description(format!(
                            "Import session {session_id} {label} resource. Read-only detail view for the manuscript import pipeline."
                        ))
                        .with_mime_type("application/json")
                        .no_annotation(),
                    );
                }
            }
        }

        Ok(ListResourcesResult::with_all_items(resources))
    }

    pub fn list_resource_templates(&self) -> ListResourceTemplatesResult {
        ListResourceTemplatesResult::with_all_items(vec![
            RawResourceTemplate::new("bible://{table}:{id}", "direct entity record")
                .with_description(
                    "Read a single persisted entity by record id. Resource-only: stable cached \
                     lookup. Examples: bible://world_rule:abc123, bible://character:mara, \
                     bible://scene:xyz789",
                )
                .with_mime_type("application/json")
                .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/chapters/{book_number}/{chapter_number}/scenes",
                "chapter scenes",
            )
            .with_description(
                "Read the active-branch scenes for a specific chapter, including scene ids, scene_order, and summaries. \
                 Tool equivalent: list_chapter_scenes. Resource provides cached scene spines; tool requires explicit parameters.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/scene-delete-impact/{book_number}/{chapter_number}/{scene_order}",
                "scene delete impact",
            )
            .with_description(
                "Read a deletion-impact audit for the active-branch scene at a given book/chapter/scene position. Resource-only cached read.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/scene-move-impact/{from_book_number}/{from_chapter_number}/{from_scene_order}/{to_book_number}/{to_chapter_number}/{to_scene_order}",
                "scene move impact",
            )
            .with_description(
                "Read a move-impact audit for relocating an active-branch scene from one story position to another. Resource-only cached read.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/research-log/{offset}/{limit}",
                "research log page",
            )
            .with_description(
                "Read a paginated slice of persisted research_query entries for a project, newest first. Resource-only cached read; use research_query tool for new queries.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/conflicts/{offset}/{limit}",
                "conflicts page",
            )
            .with_description(
                "Read a paginated slice of persisted conflict records for a project. Resource-only cached read; use create_conflict to write.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/future-knowledge/{offset}/{limit}",
                "future knowledge page",
            )
            .with_description(
                "Read a paginated slice of persisted future knowledge records for a project. Resource-only cached read; use create_future_knowledge to write.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/timeline-events/{offset}/{limit}",
                "timeline events page",
            )
            .with_description(
                "Read a paginated slice of persisted timeline events for a project in story order. Resource-only cached read; use create_timeline_event to write.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/temporal-interventions/{offset}/{limit}",
                "temporal interventions page",
            )
            .with_description(
                "Read a paginated slice of persisted temporal intervention records for a project. Resource-only cached read; use create_temporal_intervention to write.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/dual-persona-reviews/{offset}/{limit}",
                "dual persona reviews page",
            )
            .with_description(
                "Read a paginated slice of persisted dual persona reviews for a project, newest first. Resource-only cached read; use run_dual_persona_review to create reviews.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
            RawResourceTemplate::new(
                "bible://projects/{project_id}/relationships/{offset}/{limit}",
                "relationships page",
            )
            .with_description(
                "Read a paginated slice of persisted active-branch relationships for a project. Resource-only cached read; use create_relationship or update_relationship to write.",
            )
            .with_mime_type("application/json")
            .no_annotation(),
        ])
    }

    pub async fn read_resource(&self, uri: &str) -> anyhow::Result<ReadResourceResult> {
        if let Some(name) = uri.strip_prefix("bible://skills/") {
            let skill = get_skill(name)
                .ok_or_else(|| anyhow::anyhow!("skill resource not found: {uri}"))?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(skill.markdown, uri).with_mime_type("text/markdown"),
            ]));
        }

        if let Some(name) = uri.strip_prefix("bible://references/") {
            let reference = get_reference(name)
                .ok_or_else(|| anyhow::anyhow!("reference resource not found: {uri}"))?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(reference.markdown, uri).with_mime_type("text/markdown"),
            ]));
        }

        if uri == "bible://system/model-routes" {
            let content = serde_json::to_string_pretty(&self.service.model_routes())?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        if uri == "bible://config/agents" {
            let content = serde_json::to_string_pretty(&self.service.list_agents())?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        if uri == "bible://config/routing" {
            let content = serde_json::to_string_pretty(&self.service.agent_routing_config())?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        if uri == "bible://projects" {
            let projects = self.service.list_projects().await?;
            let content = serde_json::to_string_pretty(&projects)?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        // Direct entity lookup: bible://{table}:{id}
        if let Some(record_id_str) = uri.strip_prefix("bible://")
            && record_id_str.contains(':')
            && !record_id_str.contains('/')
        {
            let mut value =
                serde_json::to_value(&self.service.read_entity_by_id(record_id_str).await?)?;
            flatten_record_ids(&mut value);
            let content = serde_json::to_string_pretty(&value)?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        if let Some(project_id) = uri
            .strip_prefix("bible://projects/")
            .and_then(|rest| rest.strip_suffix("/timeline-graph/mermaid"))
        {
            let content = self
                .service
                .timeline_graph_mermaid_resource(project_id)
                .await?;
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("text/markdown"),
            ]));
        }

        if let Some(rest) = uri.strip_prefix("bible://projects/") {
            let parts = rest
                .split_once('/')
                .context("project resource is missing a path")?;
            let project_part = parts.0;
            let resource_path = parts.1;
            if resource_path.is_empty() {
                anyhow::bail!("invalid project resource uri: {uri}");
            }

            let mut value = self
                .service
                .read_project_resource(project_part, resource_path)
                .await?;
            flatten_record_ids(&mut value);
            let content = serde_json::to_string_pretty(&value)?;

            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(content, uri).with_mime_type("application/json"),
            ]));
        }

        anyhow::bail!("resource not found: {uri}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spindle_adapters::sqlite::{Repository, SqlitePool};
    use spindle_core::models::{CreateProjectInput, ReaderContract};
    use tempfile::TempDir;

    async fn fresh_router() -> (TempDir, ResourceRouter, String) {
        let tmp = TempDir::new().unwrap();
        let pool = SqlitePool::open(&tmp.path().join("resources.db"))
            .await
            .unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let service = SpindleService::new(Repository::new(pool, data_dir));
        let project = service
            .create_project(CreateProjectInput {
                name: "Resource Router".to_string(),
                project_type: "novel".to_string(),
                genre: "fantasy".to_string(),
                reader_contract: ReaderContract {
                    promise: "resources expose continuity state".to_string(),
                    style_notes: Vec::new(),
                    boundaries: Vec::new(),
                },
            })
            .await
            .unwrap();
        (tmp, ResourceRouter::new(service), project.project_id)
    }

    #[tokio::test]
    async fn timeline_graph_resource_is_listed_and_read_as_markdown() {
        let (_tmp, router, project_id) = fresh_router().await;
        let uri = format!("bible://projects/{project_id}/timeline-graph/mermaid");

        let resources = router.list_resources().await.unwrap();
        let listed = resources
            .resources
            .iter()
            .find(|resource| resource.uri == uri)
            .expect("timeline graph resource should be listed");
        assert_eq!(listed.mime_type.as_deref(), Some("text/markdown"));

        let read = router.read_resource(&uri).await.unwrap();
        assert_eq!(read.contents.len(), 1);
        match &read.contents[0] {
            ResourceContents::TextResourceContents {
                mime_type, text, ..
            } => {
                assert_eq!(mime_type.as_deref(), Some("text/markdown"));
                assert!(text.starts_with("```mermaid\nflowchart LR\n"));
                assert!(text.ends_with("```\n"));
                assert_eq!(text.matches("```").count(), 2);
                assert!(!text.trim_start().starts_with('{'));
            }
            ResourceContents::BlobResourceContents { .. } => {
                panic!("timeline graph resource should be text")
            }
        }

        let branches_uri = format!("bible://projects/{project_id}/branches");
        let branches = router.read_resource(&branches_uri).await.unwrap();
        match &branches.contents[0] {
            ResourceContents::TextResourceContents {
                mime_type, text, ..
            } => {
                assert_eq!(mime_type.as_deref(), Some("application/json"));
                assert!(text.trim_start().starts_with('['));
            }
            ResourceContents::BlobResourceContents { .. } => {
                panic!("branches resource should be text")
            }
        }
    }
}
