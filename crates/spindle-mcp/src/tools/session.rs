use super::*;

pub(super) fn required_string_argument<'a>(
    arguments: &'a rmcp::model::JsonObject,
    field: &str,
) -> anyhow::Result<&'a str> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("{field} is required"))
}

pub(super) fn tool_supports_session_project_default(name: &str) -> bool {
    !matches!(
        name,
        "set_active_project"
            | "update_entity"
            | "archive_entity"
            | "update_world_rule"
            | "resolve_revision_marker"
            | "pull_chapter_from_file"
            | "push_chapter_to_file"
            | "bind_canonical_fact_to_scene"
    )
}

pub(super) fn tool_requires_project_context(name: &str) -> bool {
    !matches!(
        name,
        "create_project"
            | "list_projects"
            | "get_model_usage"
            | "get_episode_release"
            | "set_active_project"
            | "import_manuscript"
            | "list_agents"
            | "list_skills"
            | "get_skill"
            | "get_reference"
            | "agent_routing_config"
            | "test_agent"
            | "continue_generation"
            | "revise_generation"
            | "configure_agents"
            | "init_grok_skills"
            | "update_entity"
            | "archive_entity"
            | "update_world_rule"
            | "resolve_revision_marker"
            | "pull_chapter_from_file"
            | "push_chapter_to_file"
            | "bind_canonical_fact_to_scene"
    )
}

pub(super) fn tool_supports_session_branch_default(name: &str) -> bool {
    matches!(
        name,
        "get_writer_state"
            | "get_entity"
            | "find_entity"
            | "get_character_snapshot"
            | "set_character_voice_profile"
            | "batch_set_character_voice_profiles"
            | "run_dual_persona_review"
            | "record_knowledge"
            | "record_note"
            | "update_writer_position"
    )
}

impl ToolRouter {
    pub(super) async fn set_session_context(&self, project_id: String, branch_id: Option<String>) {
        let mut session = self.session_context.write().await;
        session.active_project_id = Some(project_id);
        session.active_branch_id = branch_id;
    }

    pub(super) async fn default_branch_id_for_project(
        &self,
        project_id: &str,
    ) -> anyhow::Result<String> {
        // Per-project main branch (Phase 6): every project has its own
        // active branch row. Ask the service for it directly rather than
        // guessing or falling back to a global singleton id.
        self.service.active_branch_id_for_project(project_id).await
    }

    pub(super) async fn resolve_arguments(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
    ) -> anyhow::Result<rmcp::model::JsonObject> {
        let mut resolved = arguments.cloned().unwrap_or_default();
        let mut session = self.session_context.read().await.clone();

        if tool_supports_session_project_default(name) && !resolved.contains_key("project_id") {
            if let Some(project_id) = session.active_project_id.clone() {
                resolved.insert("project_id".to_string(), Value::String(project_id));
            } else if tool_requires_project_context(name) {
                let projects = self.service.list_projects().await?;
                match projects.projects.as_slice() {
                    [project] => {
                        let branch_id = self
                            .default_branch_id_for_project(&project.project_id)
                            .await?;
                        resolved.insert(
                            "project_id".to_string(),
                            Value::String(project.project_id.clone()),
                        );
                        session.active_project_id = Some(project.project_id.clone());
                        session.active_branch_id = Some(branch_id.clone());
                        self.set_session_context(project.project_id.clone(), Some(branch_id))
                            .await;
                    }
                    [] => anyhow::bail!(
                        "`{name}` requires a project, but none exist. Create a project first or pass `project_id` explicitly."
                    ),
                    _ => anyhow::bail!(
                        "`{name}` requires a project, but this MCP session has no active project. Call `set_active_project` or pass `project_id` explicitly."
                    ),
                }
            }
        }

        let resolved_project_id = resolved
            .get("project_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if tool_supports_session_branch_default(name)
            && !resolved.contains_key("branch_id")
            && resolved_project_id.as_deref() == session.active_project_id.as_deref()
            && let Some(branch_id) = session.active_branch_id
        {
            resolved.insert("branch_id".to_string(), Value::String(branch_id));
        }

        Ok(resolved)
    }

    pub(super) async fn set_active_project(
        &self,
        input: SetActiveProjectInput,
    ) -> anyhow::Result<SetActiveProjectOutput> {
        let project = self.service.read_entity_by_id(&input.project_id).await?;
        let project_id = project
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(input.project_id.as_str())
            .to_string();
        // Resolve branch_id: explicit input, else look up the project's
        // active branch. The per-project main branch design (Phase 6)
        // means there's no global singleton fallback.
        let branch_id = match input.branch_id.clone() {
            Some(id) => id,
            None => {
                self.service
                    .active_branch_id_for_project(&project_id)
                    .await?
            }
        };
        // Ownership check + branch_name fetch in one go.
        let (branch_id, branch_project_id, branch_name) =
            self.service.get_branch_info(&branch_id).await?;
        if branch_project_id != project_id {
            anyhow::bail!("invalid request: branch does not belong to the requested project");
        }
        self.set_session_context(project_id.clone(), Some(branch_id.clone()))
            .await;

        Ok(SetActiveProjectOutput {
            project_id,
            branch_id,
            branch_name,
            status: "ok".to_string(),
        })
    }
}
