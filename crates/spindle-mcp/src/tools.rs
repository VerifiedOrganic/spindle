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
//! | `rebuild_search_index`, `backfill_scene_source_offsets` | Compute |
//! | `configure_agents`, `test_agent`, `continue_generation`, `revise_generation` | Compute |
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
pub(crate) use profiles::profile_names;
use profiles::*;
pub(crate) use profiles::{print_profiles, profile_has_tool, profile_help, validate_tool_profile};
mod authoring;
use authoring::*;
mod session;
use session::*;
mod authoring_drafting;
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

pub use catalog::static_tool_names;
pub use skills::run_init_grok_skills;

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
        }
    }

    #[cfg(test)]
    pub fn serialization_state(&self) -> &Arc<ToolSerializationState> {
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
        let Some(profile) = self.tool_profile.as_deref() else {
            return all.to_vec();
        };
        if profile == "legacy-all" {
            return all.to_vec();
        }
        let Some(allowed) = profile_allowed_tool_names(profile) else {
            return Vec::new();
        };
        let view = if spindle_adapters::guidance::list_profiles()
            .iter()
            .any(|p| p.name == profile)
        {
            generated_tool_schemas()
        } else {
            all
        };
        view.iter()
            .filter(|t| allowed.contains(&t.name.as_ref()))
            .cloned()
            .collect()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        profile_has_tool(self.tool_profile.as_deref(), name)
    }

    #[cfg(test)]
    fn all_tools(&self) -> Vec<Tool> {
        all_tool_schemas().to_vec()
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

## Grok-Specific Guidance

- Spindle already gives excellent structural guardrails (Bible, continuity sheets, hard constraints, voice profiles, explicit routing).
- Your advantage in this environment is producing **natural, voicey, webnovel-style prose** (especially first-person, wry, comedic, internally-monologuing, or raw/explicit when the book calls for it).
- Do **not** over-polish into high-literary fiction unless the project explicitly wants that tone.
- For explicit scenes: respect the rating-aware routing. Never generate `Explicit` sexual prose client-side; use `continue_generation` with `rating: "explicit"`.

## Recommended First Actions on a Spindle Task

1. Ensure the correct project is active (`set_active_project` if needed).
2. Call `get_writer_state` to re-anchor.
3. Call the appropriate high-level briefing (`get_chapter_briefing`, `get_scene_context`, etc.).
4. Follow the detailed steps from the matching `bible://skills/*` resource.

This adapter ensures Spindle workflows feel as well-guided in Grok as they do in Claude.
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
