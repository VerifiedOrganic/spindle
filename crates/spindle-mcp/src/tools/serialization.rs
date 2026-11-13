use super::*;

impl ToolSerializationState {
    /// Return the process-wide shared [`ToolSerializationState`] for `service`'s
    /// workspace (`data_dir`), ensuring stdio, the internal HTTP listener, and
    /// the operator console all coordinate on the same per-project locks.
    pub fn for_service(service: &SpindleService) -> Arc<Self> {
        Self::for_workspace(service.repository().data_dir())
    }

    pub fn for_workspace(data_dir: &Path) -> Arc<Self> {
        let key = workspace_serialization_key(data_dir);
        let mut map = workspace_serialization_states()
            .lock()
            .expect("workspace serialization registry mutex poisoned");
        map.entry(key)
            .or_insert_with(|| Arc::new(Self::default()))
            .clone()
    }

    pub(super) fn bind_or_share_for_service(
        service: &SpindleService,
        candidate: Arc<Self>,
    ) -> Arc<Self> {
        let key = workspace_serialization_key(service.repository().data_dir());
        let mut map = workspace_serialization_states()
            .lock()
            .expect("workspace serialization registry mutex poisoned");
        map.entry(key).or_insert(candidate).clone()
    }

    pub(crate) fn issue_internal_token(
        self: &Arc<Self>,
        scope: ToolSerializationScope,
    ) -> InternalCallTokenGuard {
        static NEXT_TOKEN_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let seq = NEXT_TOKEN_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let token = format!("spindle-internal-{}-{nanos}-{seq}", std::process::id());
        let grant = InternalTokenGrant {
            scope,
            reentrant_lock: Arc::new(Mutex::new(())),
        };
        self.internal_tokens
            .lock()
            .expect("internal token mutex poisoned")
            .insert(token.clone(), grant);
        InternalCallTokenGuard {
            state: Arc::clone(self),
            token,
        }
    }

    /// True when `token` is a currently issued internal dispatch token. Used to
    /// allow internal orchestration calls (for example the harness probe) to
    /// reach operator-only tools without exposing them to model transports.
    pub(crate) fn is_internal_token(&self, token: Option<&str>) -> bool {
        let Some(token) = token.map(str::trim).filter(|token| !token.is_empty()) else {
            return false;
        };
        self.internal_tokens
            .lock()
            .map(|tokens| tokens.contains_key(token))
            .unwrap_or(false)
    }

    pub(super) fn reentrant_lock_for_token(
        &self,
        token: Option<&str>,
        required_scope: &ToolSerializationScope,
    ) -> Option<Arc<Mutex<()>>> {
        let token = token?.trim();
        if token.is_empty() {
            return None;
        }
        let tokens = self.internal_tokens.lock().ok()?;
        let grant = tokens.get(token)?;
        let allowed = match (&grant.scope, required_scope) {
            (ToolSerializationScope::Global, _) => true,
            (
                ToolSerializationScope::Project(held_project_id),
                ToolSerializationScope::Project(req_project_id),
            ) => held_project_id == req_project_id,
            (ToolSerializationScope::Project(_), ToolSerializationScope::Global) => false,
        };
        allowed.then(|| Arc::clone(&grant.reentrant_lock))
    }
}

pub(super) fn workspace_serialization_states()
-> &'static std::sync::Mutex<BTreeMap<PathBuf, Arc<ToolSerializationState>>> {
    static STATES: std::sync::OnceLock<
        std::sync::Mutex<BTreeMap<PathBuf, Arc<ToolSerializationState>>>,
    > = std::sync::OnceLock::new();
    STATES.get_or_init(|| std::sync::Mutex::new(BTreeMap::new()))
}

pub(super) fn workspace_serialization_key(data_dir: &Path) -> PathBuf {
    std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf())
}

pub(super) fn tool_requires_session_serialization(name: &str) -> bool {
    !matches!(
        name,
        "list_projects"
            | "use_profile"
            | "planning_begin"
            | "planning_advance"
            | "planning_status"
            | "planning_decide"
            | "get_model_usage"
            | "get_series_status"
            | "get_editorial_queue"
            | "prepare_episode_release"
            | "get_episode_release"
            | "list_agents"
            | "list_skills"
            | "get_skill"
            | "get_reference"
            | "agent_routing_config"
            | "test_agent"
            | "continue_generation"
            | "revise_generation"
            | "get_writer_state"
            | "get_chapter_briefing"
            | "get_scene_context"
            | "prepare_scene"
            | "get_entity"
            | "find_entity"
            | "get_character_snapshot"
            | "list_book_chapters"
            | "list_chapter_scenes"
            | "list_revision_markers"
            | "list_scene_versions"
            | "check_consistency"
            | "search_bible"
            | "find_scenes_referencing"
            | "preflight_book_export"
            | "research_query"
            | "research_search"
            | "research_pack_for_scene"
            | "research_usage_for_scene"
            | "research_plan_for_scene"
    )
}

pub(super) fn tool_requires_global_serialization(name: &str) -> bool {
    matches!(
        name,
        "create_project" | "configure_agents" | "init_grok_skills"
    )
}

impl ToolRouter {
    pub(super) async fn tool_serialization_scope(
        &self,
        name: &str,
        arguments: &rmcp::model::JsonObject,
    ) -> anyhow::Result<Option<ToolSerializationScope>> {
        if !tool_requires_session_serialization(name) {
            return Ok(None);
        }
        if tool_requires_global_serialization(name) {
            return Ok(Some(ToolSerializationScope::Global));
        }

        let Some(project_id) = arguments.get("project_id").and_then(Value::as_str) else {
            if let Some(target_project_id) =
                arguments.get("target_project_id").and_then(Value::as_str)
            {
                return Ok(Some(ToolSerializationScope::Project(
                    target_project_id.to_string(),
                )));
            }
            if name == "import_manuscript"
                && arguments
                    .get("target_project_id")
                    .and_then(Value::as_str)
                    .is_none()
                && arguments
                    .get("create_project_name")
                    .and_then(Value::as_str)
                    .is_some()
            {
                return Ok(Some(ToolSerializationScope::Global));
            }
            return self
                .resolve_project_id_for_scoped_tool(name, arguments)
                .await
                .map(ToolSerializationScope::Project)
                .map(Some);
        };
        if arguments
            .get("target_project_id")
            .and_then(Value::as_str)
            .is_none()
            && arguments
                .get("create_project_name")
                .and_then(Value::as_str)
                .is_some()
            && matches!(name, "import_hydrate_bible")
        {
            return Ok(Some(ToolSerializationScope::Global));
        }
        let project_id = match arguments.get("target_project_id").and_then(Value::as_str) {
            Some(project_id) => project_id.to_string(),
            None => project_id.to_string(),
        };
        Ok(Some(ToolSerializationScope::Project(project_id)))
    }

    async fn project_id_for_entity_id(&self, entity_id: &str) -> anyhow::Result<String> {
        Ok(self
            .service
            .read_entity_by_id(entity_id)
            .await?
            .get("project_id")
            .and_then(Value::as_str)
            .context("entity is missing project_id")?
            .to_string())
    }

    pub(super) async fn resolve_project_id_for_scoped_tool(
        &self,
        name: &str,
        arguments: &rmcp::model::JsonObject,
    ) -> anyhow::Result<String> {
        match name {
            "update_entity" | "archive_entity" => {
                let entity_id = required_string_argument(arguments, "entity_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(entity_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("entity is missing project_id")?
                    .to_string())
            }
            "update_character" => {
                let entity_id = required_string_argument(arguments, "character_id")?;
                self.project_id_for_entity_id(entity_id).await
            }
            "update_location" => {
                let entity_id = required_string_argument(arguments, "location_id")?;
                self.project_id_for_entity_id(entity_id).await
            }
            "update_conflict" => {
                let entity_id = required_string_argument(arguments, "conflict_id")?;
                self.project_id_for_entity_id(entity_id).await
            }
            "update_character_arc" => {
                let entity_id = required_string_argument(arguments, "character_arc_id")?;
                self.project_id_for_entity_id(entity_id).await
            }
            "update_world_rule" => {
                let entity_id = required_string_argument(arguments, "world_rule_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(entity_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("world rule is missing project_id")?
                    .to_string())
            }
            "resolve_revision_marker" => {
                let entity_id = required_string_argument(arguments, "marker_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(entity_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("revision marker is missing project_id")?
                    .to_string())
            }
            "pull_chapter_from_file" | "push_chapter_to_file" => {
                let chapter_id = required_string_argument(arguments, "chapter_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(chapter_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("chapter is missing project_id")?
                    .to_string())
            }
            "bind_canonical_fact_to_scene" => {
                let scene_id = required_string_argument(arguments, "scene_id")?;
                Ok(self
                    .service
                    .read_entity_by_id(scene_id)
                    .await?
                    .get("project_id")
                    .and_then(Value::as_str)
                    .context("scene is missing project_id")?
                    .to_string())
            }
            _ => anyhow::bail!("{name} requires project_id for mutation serialization"),
        }
    }

    pub(super) async fn lock_tool_scope(
        &self,
        scope: ToolSerializationScope,
    ) -> ToolSerializationGuard {
        match scope {
            ToolSerializationScope::Global => ToolSerializationGuard::Global {
                _gate: self
                    .serialization_state
                    .serialization_gate
                    .clone()
                    .write_owned()
                    .await,
            },
            ToolSerializationScope::Project(project_id) => {
                let gate = self
                    .serialization_state
                    .serialization_gate
                    .clone()
                    .read_owned()
                    .await;
                let project_lock = {
                    let mut locks = self.serialization_state.project_locks.lock().await;
                    locks
                        .entry(project_id)
                        .or_insert_with(|| Arc::new(Mutex::new(())))
                        .clone()
                };
                let project = project_lock.lock_owned().await;
                ToolSerializationGuard::Project {
                    _project: project,
                    _gate: gate,
                }
            }
        }
    }

    /// Is this serialization scope free right now? Used only to let
    /// `authoring_status` choose its degraded read-only path instead of queueing
    /// behind a minutes-long write. Advisory by nature — a scope free here can
    /// be taken a moment later, which is harmless: the caller either takes the
    /// lock normally or serves a non-persisting read.
    pub(super) async fn scope_is_free(&self, scope: &ToolSerializationScope) -> bool {
        match scope {
            ToolSerializationScope::Global => self
                .serialization_state
                .serialization_gate
                .try_write()
                .is_ok(),
            ToolSerializationScope::Project(project_id) => {
                let project_lock = {
                    let mut locks = self.serialization_state.project_locks.lock().await;
                    locks
                        .entry(project_id.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(())))
                        .clone()
                };
                project_lock.try_lock().is_ok()
            }
        }
    }
}
