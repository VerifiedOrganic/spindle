//! MCP tool router for Spindle.
//!
//! ## Resource vs Tool Rule
//!
//! Spindle uses MCP resources and tools with a clear separation:
//!
//! **Resources (`bible://...`)** are for **stable, infrequently-changing reads** that benefit
//! from caching. They represent the canonical state of the Story Bible at a point in time.
//!
//! **Tools** are for **everything else**: state-changing operations, computations,
//! parameterized queries, and reads that require dynamic context or parameters.
//!
//! **Decision rule:** if an operation changes state, computes a result dynamically, or
//! requires parameters beyond simple project/entity IDs, use a tool. If it reads stable,
//! cacheable state, use a resource.
//!
//! ## Categorization
//!
//! ### Resource only
//!
//! These are available exclusively as `bible://` resources — no tool provides the same
//! read:
//!
//! | Resource URI | Description |
//! |---|---|
//! | `bible://skills/{name}` | Embedded skill markdown |
//! | `bible://references/{name}` | Craft reference markdown |
//! | `bible://system/model-routes` | Model route config |
//! | `bible://config/agents` | Sanitized agent config |
//! | `bible://config/routing` | Route assignment config |
//! | `bible://projects/{id}/chapters` | Chapters with scene spines |
//! | `bible://projects/{id}/characters` | Character list |
//! | `bible://projects/{id}/locations` | Location list |
//! | `bible://projects/{id}/world-rules` | World rule list |
//! | `bible://projects/{id}/factions` | Faction list |
//! | `bible://projects/{id}/plot-lines` | Plot line list |
//! | `bible://projects/{id}/conflicts` | Conflict list |
//! | `bible://projects/{id}/themes` | Theme list |
//! | `bible://projects/{id}/motifs` | Motif list |
//! | `bible://projects/{id}/narrative-promises` | Narrative promise list |
//! | `bible://projects/{id}/pacing/overview` | Pacing overview |
//! | `bible://projects/{id}/chapter-summaries` | Saved chapter summaries |
//! | `bible://projects/{id}/research-log` | Research log (paginated) |
//! | `bible://projects/{id}/reader-contract` | Reader contract |
//! | `bible://projects/{id}/branches` | Branch list (read-only; create_branch / switch_branch are write tools) |
//! | `bible://projects/{id}/continuity/health` | Active-branch continuity health summary |
//! | `bible://projects/{id}/future-knowledge` | Future knowledge list |
//! | `bible://projects/{id}/timeline-events` | Timeline event list |
//! | `bible://projects/{id}/timeline-graph/mermaid` | Branch/timeline Mermaid graph |
//! | `bible://projects/{id}/temporal-interventions` | Temporal intervention list |
//! | `bible://projects/{id}/system-overlays` | System overlay list |
//! | `bible://projects/{id}/dual-persona-reviews` | Dual persona review list |
//! | `bible://projects/{id}/relationships` | Relationship list |
//! | `bible://projects/{id}/character-arcs` | Character arc list |
//! | `bible://projects/{id}/religions` | Religion list |
//! | `bible://projects/{id}/economies` | Economy list |
//! | `bible://projects/{id}/terms` | Term list |
//! | `bible://projects/{id}/imports` | Import session list |
//! | `bible://projects/{id}/imports/{sid}/{path}` | Import session detail |
//! | `bible://{table}:{id}` | Direct entity lookup by record id |
//! | `bible://projects/{id}/scene-delete-impact/{b}/{c}/{s}` | Scene delete impact audit |
//! | `bible://projects/{id}/scene-move-impact/{from_book}/{from_chapter}/{from_scene}/{to_book}/{to_chapter}/{to_scene}` | Scene move impact audit |
//! | `bible://projects/{id}/research-log/{offset}/{limit}` | Research log page |
//!
//! ### Tool only
//!
//! Representative tool-only operations mutate state, compute dynamic results,
//! or require parameters that go beyond simple IDs:
//!
//! | Tool | Category |
//! |---|---|
//! | `create_project`, `create_book`, `create_chapter` | Write |
//! | `create_character`, `create_location`, `create_faction` | Write |
//! | `create_religion`, `create_economy`, `create_term` | Write |
//! | `create_relationship`, `create_world_rule` | Write |
//! | `create_plot_line`, `create_conflict`, `create_theme` | Write |
//! | `create_motif`, `create_narrative_promise` | Write |
//! | `create_character_arc`, `create_future_knowledge` | Write |
//! | `create_timeline_event`, `create_temporal_intervention` | Write |
//! | `create_system_overlay`, `create_pacing_config` | Write |
//! | `create_pacing_curve`, `set_arc_pacing_constraints` | Write |
//! | `update_entity`, `update_relationship`, `update_promise_status` | Write |
//! | `archive_entity` | Write |
//! | `save_scene_draft`, `commit_scene_changes` | Write |
//! | `commit_character_state`, `record_knowledge` | Write |
//! | `save_summary`, `register_canonical_fact`, `bind_canonical_fact_to_scene` | Write |
//! | `plan_chapter`, `annotate_scene_beats` | Write |
//! | `move_scene`, `delete_scene`, `operator_delete_scene` | Write |
//! | `create_branch`, `switch_branch` | Write |
//! | `create_save_point`, `restore_save_point` | Write |
//! | `diff_branches`, `merge_branch` | Write/Compute |
//! | `revise_scene`, `generate_alternatives`, `compare_alternatives`, `select_alternative` | Write/Compute |
//! | `list_revision_markers`, `resolve_revision_marker` | Read+Write |
//! | `import_manuscript`, `import_extract_entities` | Write |
//! | `import_consolidate_entities`, `import_analyze_character` | Write |
//! | `import_extract_world`, `import_analyze_narrative` | Write |
//! | `import_compute_final_state`, `import_hydrate_bible` | Write |
//! | `import_apply_review_decisions`, `import_status` | Read+Write |
//! | `run_dual_persona_review`, `check_consistency` | Compute |
//! | `search_bible`, `find_scenes_referencing` | Search (dynamic) |
//! | `continue_generation`, `revise_generation` | Compute |
//! | `rebuild_search_index`, `backfill_scene_source_offsets`, `configure_agents`, `test_agent` | Operator CLI only (hidden/denied on model transports) |
//! | `research_query` | Compute |
//! | `export_epub`, `preflight_book_export`, `export_bible` | Compute/Export |
//! | `list_scene_versions`, `restore_scene_version` | Read+Write |
//!
//! ### Both (resource + tool with different shapes)
//!
//! These exist as both a resource and a tool. The resource provides a cached, stable
//! read; the tool provides the same data (or a shaped subset) with explicit parameters.
//!
//! | Resource | Tool | Difference |
//! |---|---|---|
//! | `bible://projects` | `list_projects` | Same data; resource is cached, tool is dynamic |
//! | `bible://projects/{id}/books` | `list_book_chapters` | Resource lists all books; tool returns chapters for one book |
//! | `bible://projects/{id}/chapters/{b}/{c}/scenes` | `list_chapter_scenes` | Resource is cached, tool requires explicit project+chapter params |
//! | `bible://config/agents` | `list_agents` | Same data; resource is cached |
//! | `bible://skills/{name}` | `get_skill` | Same content; tool works for clients without resource support |
//! | `bible://references/{name}` | `get_reference` | Same content; tool works for clients without resource support |

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use rmcp::model::{CallToolResult, Content, Tool};
use rmcp::schemars;
use rmcp::schemars::generate::SchemaSettings;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Number, Value};
use spindle_adapters::sqlite::SqliteSpindleService as SpindleService;
use spindle_core::models::*;
use spindle_core::planning::*;
use spindle_core::style::*;
use spindle_core::subject_snapshot::SubjectSnapshot as EntitySubjectSnapshot;
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::json_utils::flatten_record_ids;
use crate::run_journal::{self, RunJournal};
use spindle_harness::artifacts::{
    ArtifactStore, ChapterSummaryArtifact, CheckpointReportArtifact,
    GeneratedChapterSummaryPackage, GeneratedScenePackage, SceneGenerationArtifact,
};
use spindle_harness::execution::{ExecutionResult, execute_one};
use spindle_harness::mcp::{McpHarnessClient, TransportConfig};
use spindle_harness::plan::{
    ChapterPlanSnapshot, ChapterSnapshot, NextAction, PersistedScene, PlannedSceneSnapshot,
    ProjectSnapshot, reconcile_state,
};
use spindle_harness::state::{CheckpointRecord, CheckpointStatus, HarnessState, ScenePhase};

mod serialization;
#[cfg(test)]
use serialization::*;
mod catalog;
use catalog::*;
mod profiles;
#[cfg(test)]
pub(crate) use profiles::OPERATOR_ONLY_TOOLS;
pub(crate) use profiles::is_operator_only;
pub(crate) use profiles::profile_names;
pub(crate) use profiles::{
    print_profiles, profile_allowed_tool_names, profile_has_tool, profile_help,
    profile_switch_error, validate_tool_profile,
};
mod authoring;
use authoring::*;
mod session;
use session::*;
mod authoring_drafting;
mod commit_scene;
mod dispatch;
use authoring_drafting::*;
mod journal;
use journal::*;
mod authoring_checkpoint;
use authoring_checkpoint::*;
mod discovery;
use discovery::*;
pub(crate) use discovery::{project_tool_discovery, response_contract, response_contract_index};
mod schema;
mod skills;
use schema::*;
mod authoring_status;
mod planning;

pub use catalog::static_tool_names;
pub use skills::run_init_grok_skills;
pub(crate) use skills::{resolve_reference_output_for_profile, resolve_skill_output_for_profile};

#[derive(Debug, Clone, Default)]
struct SessionContext {
    active_project_id: Option<String>,
    active_branch_id: Option<String>,
}

#[derive(Clone)]
pub struct ToolRouter {
    service: SpindleService,
    session_context: Arc<RwLock<SessionContext>>,
    serialization_state: Arc<ToolSerializationState>,
    tool_profile: Option<String>,
    /// Session-scoped runtime profile state. `None` keeps the router's fixed
    /// `tool_profile` (tests and exact per-request views); server sessions
    /// attach shared state so `use_profile` can switch without reconnecting.
    runtime_profile: Option<Arc<std::sync::RwLock<Option<String>>>>,
    /// Test-only ownership guard for the fixture's temporary data directory.
    /// Keeping it here lets the router's Drop remove the directory without
    /// changing every fixture call site.
    #[cfg(test)]
    temp_dir: Option<Arc<tempfile::TempDir>>,
}

#[derive(Clone, Default)]
pub struct ToolSerializationState {
    serialization_gate: Arc<RwLock<()>>,
    project_locks: Arc<Mutex<BTreeMap<String, Arc<Mutex<()>>>>>,
    internal_tokens: Arc<std::sync::Mutex<BTreeMap<String, InternalTokenGrant>>>,
}

#[derive(Clone)]
struct InternalTokenGrant {
    scope: ToolSerializationScope,
    reentrant_lock: Arc<Mutex<()>>,
}

pub struct InternalCallTokenGuard {
    state: Arc<ToolSerializationState>,
    token: String,
}

impl InternalCallTokenGuard {
    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for InternalCallTokenGuard {
    fn drop(&mut self) {
        if let Ok(mut tokens) = self.state.internal_tokens.lock() {
            tokens.remove(&self.token);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ToolSerializationScope {
    Global,
    Project(String),
}

enum ToolSerializationGuard {
    Global {
        _gate: OwnedRwLockWriteGuard<()>,
    },
    Project {
        _project: OwnedMutexGuard<()>,
        _gate: OwnedRwLockReadGuard<()>,
    },
    Reentrant {
        _reentrant: OwnedMutexGuard<()>,
    },
}

impl ToolRouter {
    pub fn with_tool_profile_and_serialization(
        service: SpindleService,
        tool_profile: Option<String>,
        serialization_state: Arc<ToolSerializationState>,
    ) -> Self {
        let serialization_state =
            ToolSerializationState::bind_or_share_for_service(&service, serialization_state);
        Self {
            service,
            session_context: Arc::new(RwLock::new(SessionContext::default())),
            serialization_state,
            tool_profile,
            runtime_profile: None,
            #[cfg(test)]
            temp_dir: None,
        }
    }

    /// A view of this router with a different fixed tool profile. The session
    /// context and serialization state are shared, so `set_active_project`
    /// and branch selection survive per-request profile selection. Runtime
    /// switching is intentionally disabled in this view: internal-listener
    /// profile overrides must stay exact.
    pub(crate) fn with_profile(&self, tool_profile: Option<String>) -> Self {
        Self {
            service: self.service.clone(),
            session_context: self.session_context.clone(),
            serialization_state: self.serialization_state.clone(),
            tool_profile,
            runtime_profile: None,
            #[cfg(test)]
            temp_dir: self.temp_dir.clone(),
        }
    }

    /// A view of this router that shares the session's runtime profile state.
    pub(crate) fn with_active_profile(&self) -> Self {
        Self {
            service: self.service.clone(),
            session_context: self.session_context.clone(),
            serialization_state: self.serialization_state.clone(),
            tool_profile: self.tool_profile.clone(),
            runtime_profile: self.runtime_profile.clone(),
            #[cfg(test)]
            temp_dir: self.temp_dir.clone(),
        }
    }

    /// Attach session-scoped runtime profile state. Server constructors call
    /// this once; per-request views share it through `with_active_profile`.
    pub(crate) fn with_runtime_profile(
        mut self,
        runtime_profile: Arc<std::sync::RwLock<Option<String>>>,
    ) -> Self {
        self.runtime_profile = Some(runtime_profile);
        self
    }

    /// The profile active for this router view: runtime state when attached,
    /// otherwise the fixed profile.
    pub(crate) fn current_profile(&self) -> Option<String> {
        self.runtime_profile.as_ref().map_or_else(
            || self.tool_profile.clone(),
            |state| {
                state
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone()
            },
        )
    }

    /// Switch this session's active profile and return the new discovery
    /// summary. Unknown names fail with a structured `unknown_profile` error;
    /// routers without session state reject the call.
    fn switch_profile(&self, profile: &str) -> Result<UseProfileOutput, CallToolResult> {
        let Some(state) = self.runtime_profile.as_ref() else {
            return Err(profile_switch_error(
                "profile_switch_unavailable",
                profile,
                "profile switching is only available in an MCP server session",
            ));
        };
        if let Err(error) = validate_tool_profile(Some(profile)) {
            return Err(profile_switch_error(
                "unknown_profile",
                profile,
                &format!("{error:#}"),
            ));
        }
        *state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(profile.to_owned());
        let tools: Vec<String> = self
            .list_tools()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        Ok(UseProfileOutput {
            profile: profile.to_owned(),
            tool_count: tools.len(),
            tools,
        })
    }

    #[cfg(test)]
    pub fn hold_temp_dir(&mut self, temp: tempfile::TempDir) {
        self.temp_dir = Some(Arc::new(temp));
    }

    pub(crate) fn serialization_state(&self) -> &Arc<ToolSerializationState> {
        &self.serialization_state
    }

    /// Borrow the underlying service (test-only accessor for integration tests
    /// that need to read persisted state directly through the repository).
    #[cfg(test)]
    pub fn service(&self) -> &SpindleService {
        &self.service
    }

    pub fn list_tools(&self) -> Vec<Tool> {
        let all = all_tool_schemas();
        let visible = |tools: &[CatalogTool]| -> Vec<Tool> {
            tools
                .iter()
                .filter(|tool| !is_operator_only(tool.name.as_ref()))
                .map(|tool| tool.tool.clone())
                .collect()
        };
        let Some(profile) = self.current_profile() else {
            return visible(all);
        };
        let Some(allowed) = profile_allowed_tool_names(&profile) else {
            return Vec::new();
        };
        generated_tool_schemas()
            .iter()
            .filter(|tool| {
                allowed.contains(&tool.name.as_ref()) && !is_operator_only(tool.name.as_ref())
            })
            .cloned()
            .collect()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        profile_has_tool(self.current_profile().as_deref(), name)
    }
}

/// The Grok skill adapter content for Spindle.
/// This makes the bible://skills/* resources usable as first-class skills in Grok.
const SPINDLE_GROK_SKILL_MD: &str = r#"---
name: spindle
description: Use for any work involving Spindle book projects (new books, scene writing, character creation, worldbuilding, revision, explicit scenes, etc.). This is the Grok adapter for Spindle's official skills.
---

# Spindle (Grok Adapter)

This skill bridges Spindle's canonical instructions (served as `bible://skills/*` MCP resources) into Grok's skill system.

## How to use

When the user asks to do anything related to writing, worldbuilding, or managing a book inside Spindle, activate this skill.

The authoritative, always-up-to-date instructions live at:
- `bible://skills/scene-writer`
- `bible://skills/character-creator`
- `bible://skills/worldbuilder`
- `bible://skills/revision-manager`
- `bible://skills/continuity-editor`
- `bible://skills/editor`
- `bible://skills/manuscript-importer`
- and others under `bible://skills/*`

**Always prefer reading the live `bible://` version** for the current detailed procedure rather than relying on stale embedded text.

## Host-Neutral Drafting Guidance

- Spindle already gives excellent structural guardrails (Bible, continuity sheets, hard constraints, voice profiles, explicit routing).
- A useful advantage is producing **natural, voicey, webnovel-style prose** (especially first-person, wry, comedic, internally-monologuing, or raw/explicit when the book calls for it).
- Do **not** over-polish into high-literary fiction unless the project explicitly wants that tone.
- For explicit scenes: respect the rating-aware routing. Never generate `Explicit` sexual prose client-side; use `continue_generation` with `rating: "explicit"`.

## Recommended First Actions on a Spindle Task

1. Ensure the correct project is active (`set_active_project` if needed).
2. Call `get_writer_state` to re-anchor.
3. Call the appropriate high-level briefing (`get_chapter_briefing`, `get_scene_context`, etc.).
4. Follow the detailed steps from the matching `bible://skills/*` resource.

This guidance is intentionally host-neutral and applies to any MCP host.
"#;

/// The result of the in-process auto-checkpoint automation (evolution §3.3).
/// Ids/counts/enums only, never prose (I8).
struct AutoCheckpointOutcome {
    /// True iff the automation self-cleared the checkpoint under its policy.
    approved: bool,
    /// Deep-consistency severity counts (`{severity: n}`) at approval; empty on
    /// a block (the block reason carries the prose-free summary).
    finding_counts: std::collections::BTreeMap<String, i64>,
    /// Prose-free block summary when not approved; `None` on approval.
    blocked_reason: Option<String>,
    /// Scene ids awaiting manual dual-persona review (rating not covered — I3).
    pending_manual_scene_ids: Vec<String>,
}

#[cfg(test)]
mod tests;
