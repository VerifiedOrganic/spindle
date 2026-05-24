//! Pure formatting helpers reinstated after the SurrealDB removal in Phase 6.
//!
//! These functions render `spindle_core::models::*` values into the Markdown
//! representations consumed by the MCP tools layer (`get_writer_state` and
//! `get_scene_context`, plus chapter-briefing and scene-impact summaries).
//! A small number of helpers operate over SQLite-side record types
//! (`crate::sqlite::records::*` / `crate::sqlite::json_records::*`) because
//! the original block used the equivalent SurrealDB records — IDs on those
//! records are already plain `String`s so the translation is mechanical.
//!
//! Nothing in this module touches the database; everything is pure.

use std::collections::BTreeSet;

use spindle_core::context_bundle::{estimate_json_tokens, estimate_text_tokens};
use spindle_core::models::{
    AgencyCheckSummary, BookOutline, BranchSummary, CanonicalFactReadModel,
    ChapterBriefingSceneSeed, ChapterOutline, ChapterPlanBriefing, ChapterSummaryBriefing,
    CharacterStateSummary, ConsistencyScope, ContextFormat, FutureKnowledgeSummary,
    GetSceneDeleteImpactOutput, GetSceneMoveImpactOutput, HardConstraint, KnowledgeBriefingItem,
    LocationSummary, NarrativePromiseDueSummary, PacingDirectiveSummary, ReaderContract,
    RecentSceneSummary, RelationshipSummary, SceneContextNovelLayer, SceneContextOutput,
    SceneContextSceneLayer, SceneDeleteImpactGroup, SceneMoveImpactGroup, SearchBibleResultItem,
    SystemOverlaySummary, TimelineEventSummary, WorldStateSummary, WriterIntent, WriterState,
};
use spindle_core::subject_snapshot::{RenderDepth, SubjectSnapshot as SnapshotSubject};

use crate::sqlite::json_records::StoredStoryPlacement;
use crate::sqlite::records::{
    BibleBranch, CanonicalFact, ChapterPlan, ChapterSummary, CharacterArc, FutureKnowledge,
    KnowledgeFact, Location, NarrativePromise, PacingTracker, Scene, SystemOverlay, TimelineEvent,
    WorldRule,
};

// =============================================================================
// Constants shared with the service layer.
// =============================================================================

/// Default markdown-render token budget for `get_writer_state`.
pub const DEFAULT_WRITER_STATE_BUDGET_TOKENS: usize = 8000;

/// Default cap on the recent_session_activity slice surfaced by `get_writer_state`.
pub const DEFAULT_WRITER_STATE_RECENT_ACTIVITY_LIMIT: usize = 20;

/// Default cap on the recent-scenes slice surfaced by `get_writer_state`.
pub const DEFAULT_WRITER_STATE_RECENT_SCENE_LIMIT: usize = 3;

/// Default cap on `next.suggested_subjects` produced by `get_writer_state`.
pub const DEFAULT_WRITER_STATE_SUGGESTED_SUBJECT_LIMIT: usize = 5;

/// Cap on `sample_record_ids` exposed in scene-delete/move impact groups.
pub const MAX_SCENE_DELETE_IMPACT_SAMPLE_IDS: usize = 5;

/// Order in which optional `WriterState` sections are dropped when the
/// caller-supplied `budget_tokens` is exceeded. Earlier entries are trimmed
/// first; sections listed in `writer_state_included_sections` but absent from
/// this slice are considered mandatory and never trimmed.
pub const WRITER_STATE_TRIMMING_ORDER: &[&str] = &[
    "book_outline",
    "chapter_outline",
    "recent_session_activity",
    "unsynced_local_files",
    "drift_warnings",
    "active_overlays",
    "subjects",
    "open_promises_due_now",
    "recent_scenes",
];

// =============================================================================
// Writer state markdown
// =============================================================================

pub fn format_writer_state_markdown(state: &WriterState) -> String {
    let mut lines = vec!["# Writer state".to_string()];

    if writer_state_includes_section(state, "current") {
        lines.push("\n## Current".to_string());
        lines.push(format!("- Project: {}", state.current.project.name));
        lines.push(format!("- Branch: {}", state.current.branch.name));
        lines.push(format!("- Intent: {:?}", state.current.intent));
        if let Some(book) = state.current.book.as_ref() {
            lines.push(format!(
                "- Book {}: {}",
                book.book_number,
                book.title.clone().unwrap_or_else(|| "Untitled".to_string())
            ));
        }
        if let Some(chapter) = state.current.chapter.as_ref() {
            lines.push(format!(
                "- Chapter {}: {}",
                chapter.chapter_number,
                chapter
                    .title
                    .clone()
                    .unwrap_or_else(|| "Untitled".to_string())
            ));
        }
        if let Some(scene) = state.current.scene.as_ref() {
            lines.push(format!("- Scene {}: {}", scene.scene_order, scene.summary));
        }
        if let Some(summary) = state.current.last_completed_scene_summary.as_deref() {
            lines.push(format!("- Last completed scene: {summary}"));
        }
    }

    if writer_state_includes_section(state, "next") {
        lines.push("\n## Next".to_string());
        lines.push(format!(
            "- Intended focus: {}",
            state
                .next
                .intended_focus
                .clone()
                .unwrap_or_else(|| "None".to_string())
        ));
        if !state.next.suggested_subjects.is_empty() {
            lines.push("- Suggested subjects:".to_string());
            for subject in &state.next.suggested_subjects {
                lines.push(format!("- {} ({})", subject.name, subject.kind));
            }
        }
    }

    if writer_state_includes_section(state, "hard_constraints") {
        lines.push("\n## Hard constraints".to_string());
        if state.hard_constraints.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for constraint in &state.hard_constraints {
                lines.push(format!("- **{}**: {}", constraint.id, constraint.statement));
            }
        }
    }

    if writer_state_includes_section(state, "subjects") {
        lines.push("\n## Subjects".to_string());
        if state.subjects.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for subject in &state.subjects {
                lines.push(format!("- {}: {}", subject.subject.name, subject.summary));
            }
        }
    }

    if writer_state_includes_section(state, "recent_scenes") {
        lines.push("\n## Recent scenes".to_string());
        if state.recent_scenes.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for scene in &state.recent_scenes {
                lines.push(format!(
                    "- Book {} Chapter {} Scene {}: {}",
                    scene.book_number, scene.chapter_number, scene.scene_order, scene.summary
                ));
            }
        }
    }

    if writer_state_includes_section(state, "open_promises_due_now") {
        lines.push("\n## Open promises due now".to_string());
        if state.open_promises_due_now.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for promise in &state.open_promises_due_now {
                lines.push(format!(
                    "- [{}] {}",
                    promise.promise_type, promise.description
                ));
            }
        }
    }

    if writer_state_includes_section(state, "active_overlays") {
        lines.push("\n## Active overlays".to_string());
        if state.active_overlays.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for overlay in &state.active_overlays {
                lines.push(format!("- {}", overlay.name));
            }
        }
    }

    if writer_state_includes_section(state, "drift_warnings") {
        lines.push("\n## Drift warnings".to_string());
        if state.drift_warnings.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for warning in &state.drift_warnings {
                lines.push(format!("- [{}] {}", warning.code, warning.message));
            }
        }
    }

    if writer_state_includes_section(state, "unsynced_local_files") {
        lines.push("\n## Unsynced local files".to_string());
        if state.unsynced_local_files.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for entry in &state.unsynced_local_files {
                lines.push(format!("- {} ({:?})", entry.source_path, entry.kind));
            }
        }
    }

    if writer_state_includes_section(state, "recent_session_activity") {
        lines.push("\n## Recent session activity".to_string());
        if state.recent_session_activity.is_empty() {
            lines.push("- None.".to_string());
        } else {
            for activity in &state.recent_session_activity {
                lines.push(format!("- [{}] {}", activity.kind, activity.summary));
            }
        }
    }

    if writer_state_includes_section(state, "chapter_outline")
        && let Some(chapter_outline) = state.chapter_outline.as_ref()
    {
        lines.push(format_chapter_outline_markdown(chapter_outline));
    }
    if writer_state_includes_section(state, "book_outline")
        && let Some(book_outline) = state.book_outline.as_ref()
    {
        lines.push(format_book_outline_markdown(book_outline));
    }

    lines.join("\n")
}

pub fn writer_state_includes_section(state: &WriterState, section_id: &str) -> bool {
    state
        .bundle_summary
        .included_sections
        .iter()
        .any(|candidate| candidate == section_id)
}

// =============================================================================
// Chapter briefing markdown
// =============================================================================

#[allow(clippy::too_many_arguments)]
pub fn format_chapter_briefing_markdown(
    book_number: i32,
    chapter_number: i32,
    scene_order: Option<i32>,
    hard_constraints: &[HardConstraint],
    continuity_sheets: &[SnapshotSubject],
    recent_chapter_summaries: &[ChapterSummaryBriefing],
    chapter_outline: Option<&ChapterOutline>,
    book_outline: Option<&BookOutline>,
    chapter_plan: Option<&ChapterPlanBriefing>,
    scene_context: Option<&SceneContextOutput>,
    scene_seed: &ChapterBriefingSceneSeed,
) -> String {
    let heading = match scene_order {
        Some(scene_order) => {
            format!(
                "Target scene: Book {book_number}, Chapter {chapter_number}, Scene {scene_order}"
            )
        }
        None => format!("Target chapter: Book {book_number}, Chapter {chapter_number}"),
    };
    let mut lines = vec![format!("# Chapter Briefing\n\n{heading}")];

    lines.push(format_chapter_briefing_hard_constraints_markdown(
        hard_constraints,
    ));
    lines.push(format_chapter_briefing_continuity_sheets_markdown(
        continuity_sheets,
    ));

    lines.push(format_recent_chapter_summaries_markdown(
        recent_chapter_summaries,
    ));
    if let Some(chapter_outline) = chapter_outline {
        lines.push(format_chapter_outline_markdown(chapter_outline));
    }
    if let Some(book_outline) = book_outline {
        lines.push(format_book_outline_markdown(book_outline));
    }
    if let Some(chapter_plan) = chapter_plan {
        lines.push(format_current_chapter_plan_markdown(chapter_plan));
    }
    lines.push(format_chapter_briefing_scene_context_markdown(
        scene_context,
        scene_seed,
    ));

    lines.join("\n")
}

pub fn format_chapter_briefing_hard_constraints_markdown(
    hard_constraints: &[HardConstraint],
) -> String {
    let mut lines = vec!["\n## Hard constraints".to_string()];
    if hard_constraints.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for constraint in hard_constraints {
            lines.push(format_chapter_briefing_hard_constraint_line(constraint));
        }
    }
    lines.join("\n")
}

pub fn format_chapter_briefing_hard_constraint_line(constraint: &HardConstraint) -> String {
    let statement = constraint.statement.trim();
    if statement.is_empty() {
        format!("- **{}**", constraint.id)
    } else {
        format!("- **{}**: {}", constraint.id, statement)
    }
}

pub fn format_chapter_briefing_canonical_facts_markdown(
    canonical_facts: &[CanonicalFactReadModel],
) -> String {
    if canonical_facts.is_empty() {
        return String::new();
    }

    let mut lines = vec!["\n## Canonical facts".to_string()];
    for fact in canonical_facts {
        lines.push(format!(
            "- **{}** [{}]: {}",
            fact.predicate,
            fact.value_kind,
            canonical_fact_read_model_value_display(fact)
        ));
    }
    lines.join("\n")
}

pub fn format_chapter_briefing_continuity_sheets_markdown(
    continuity_sheets: &[SnapshotSubject],
) -> String {
    if continuity_sheets.is_empty() {
        return String::new();
    }

    let mut lines = vec!["\n## Continuity sheets".to_string()];
    lines.push(
        "- Treat physical details, habits, voice profile, current state, relationships, and recent appearances here as authoritative for drafting.".to_string(),
    );
    for snapshot in continuity_sheets {
        lines.push(snapshot.render_markdown(RenderDepth::Standard));
    }
    lines.join("\n\n")
}

pub fn format_recent_chapter_summaries_markdown(
    recent_chapter_summaries: &[ChapterSummaryBriefing],
) -> String {
    let mut lines = Vec::new();
    if recent_chapter_summaries.is_empty() {
        lines.push("\n## Recent chapter summaries".to_string());
        lines.push("- None recorded before this chapter.".to_string());
    } else {
        lines.push("\n## Recent chapter summaries".to_string());
        for summary in recent_chapter_summaries {
            lines.push(format!(
                "- Book {} Chapter {}: {}",
                summary.book_number, summary.chapter_number, summary.summary
            ));
            push_briefing_list(&mut lines, "  key events", &summary.key_events);
            push_briefing_list(
                &mut lines,
                "  character changes",
                &summary.character_changes,
            );
            push_briefing_list(
                &mut lines,
                "  relationship shifts",
                &summary.relationship_shifts,
            );
            push_briefing_list(&mut lines, "  arc advances", &summary.arc_advances);
            push_briefing_list(&mut lines, "  promise events", &summary.promise_events);
        }
    }
    lines.join("\n")
}

pub fn format_chapter_outline_markdown(chapter_outline: &ChapterOutline) -> String {
    let mut lines = vec!["\n## Chapter outline".to_string()];
    lines.push(format!("- Format: {}", chapter_outline.format));
    if !chapter_outline.content.trim().is_empty() {
        lines.push(chapter_outline.content.clone());
    }
    if !chapter_outline.beats.is_empty() {
        lines.push("- Beats:".to_string());
        for beat in &chapter_outline.beats {
            lines.push(format!(
                "- {} [{}]: {}",
                beat.order, beat.status, beat.summary
            ));
            if let Some(scene_id) = beat.scene_id.as_deref() {
                lines.push(format!("  scene id: {scene_id}"));
            }
        }
    }
    lines.join("\n")
}

pub fn format_book_outline_markdown(book_outline: &BookOutline) -> String {
    let mut lines = vec!["\n## Book outline".to_string()];
    lines.push(format!("- Format: {}", book_outline.format));
    lines.push(book_outline.content.clone());
    lines.join("\n")
}

pub fn format_current_chapter_plan_markdown(chapter_plan: &ChapterPlanBriefing) -> String {
    let mut lines = vec!["\n## Current chapter plan".to_string()];
    lines.push(format!("- Synopsis: {}", chapter_plan.synopsis));
    if let Some(pov_character_id) = chapter_plan.pov_character_id.as_deref() {
        lines.push(format!("- POV character: {pov_character_id}"));
    }
    for scene in &chapter_plan.scenes {
        lines.push(format!(
            "- Planned scene {}: {}",
            scene.scene_order, scene.summary
        ));
        lines.push(format!("  purpose: {}", scene.purpose));
        push_briefing_list(&mut lines, "  beat structure", &scene.beat_structure);
    }
    lines.join("\n")
}

pub fn format_chapter_briefing_scene_context_markdown(
    scene_context: Option<&SceneContextOutput>,
    scene_seed: &ChapterBriefingSceneSeed,
) -> String {
    let mut lines = Vec::new();
    if let Some(scene_context) = scene_context {
        lines.push("\n## Scene context highlights".to_string());
        lines.push(format!(
            "- Reader contract: {}",
            scene_context.novel.reader_contract.promise
        ));
        lines.push(format!(
            "- Location: {} ({})",
            scene_context.scene.location.name, scene_context.scene.location.kind
        ));
        if let Some(status) = scene_context.scene.world_state.status.as_deref() {
            lines.push(format!("- World state: {status}"));
        }
        if !scene_context.scene.characters.is_empty() {
            lines.push("- Characters:".to_string());
            for character in &scene_context.scene.characters {
                let goals = if character.goals.is_empty() {
                    "no explicit goals".to_string()
                } else {
                    character.goals.join("; ")
                };
                let status = if character.status.is_empty() {
                    "no explicit status".to_string()
                } else {
                    character.status.join("; ")
                };
                lines.push(format!("- {} ({})", character.name, character.role));
                lines.push(format!("  goals: {goals}"));
                lines.push(format!("  status: {status}"));
            }
        }
        if let Some(warning) = scene_context.scene.agency_check.warning.as_deref() {
            lines.push(format!("- Agency warning: {warning}"));
        }
        push_briefing_list(
            &mut lines,
            "- Due promises",
            &scene_context
                .novel
                .narrative_promises_due
                .iter()
                .map(|promise| promise.description.clone())
                .collect::<Vec<_>>(),
        );
        push_briefing_list(
            &mut lines,
            "- Pacing warnings",
            &scene_context
                .novel
                .pacing_directives
                .iter()
                .flat_map(|directive| directive.warnings.clone())
                .collect::<Vec<_>>(),
        );
        push_briefing_list(
            &mut lines,
            "- Knowledge briefing",
            &scene_context
                .novel
                .knowledge_briefing
                .iter()
                .map(|item| item.fact.clone())
                .collect::<Vec<_>>(),
        );
        push_briefing_list(
            &mut lines,
            "- Semantic references",
            &scene_context
                .novel
                .semantic_references
                .iter()
                .map(|item| format!("{} ({})", item.title, item.entity_type))
                .collect::<Vec<_>>(),
        );
    } else {
        lines.push("\n## Scene context unavailable".to_string());
        if let Some(scene_order) = scene_seed.scene_order {
            lines.push(format!("- Resolved scene order: {scene_order}"));
        }
        push_briefing_list(
            &mut lines,
            "- Resolved character ids",
            &scene_seed.character_ids,
        );
        if let Some(location_id) = scene_seed.location_id.as_deref() {
            lines.push(format!("- Resolved location id: {location_id}"));
        }
        push_briefing_list(
            &mut lines,
            "- Missing fields for scene context",
            &scene_seed.missing_fields,
        );
    }
    lines.join("\n")
}

pub fn push_briefing_list(lines: &mut Vec<String>, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    lines.push(format!("{label}: {}", values.join("; ")));
}

// =============================================================================
// Scene delete / move impact helpers.
// =============================================================================

pub fn scene_delete_placement_matches(placement: &StoredStoryPlacement, scene: &Scene) -> bool {
    placement.book_number == scene.book_number
        && placement.chapter_number == scene.chapter_number
        && placement.scene_order == Some(scene.scene_order)
}

pub fn push_scene_delete_impact_group(
    groups: &mut Vec<SceneDeleteImpactGroup>,
    dependency_type: &str,
    record_ids: Vec<String>,
    reason: &str,
) {
    if record_ids.is_empty() {
        return;
    }

    groups.push(SceneDeleteImpactGroup {
        dependency_type: dependency_type.to_string(),
        count: record_ids.len(),
        sample_record_ids: record_ids
            .into_iter()
            .take(MAX_SCENE_DELETE_IMPACT_SAMPLE_IDS)
            .collect(),
        reason: reason.to_string(),
    });
}

pub fn push_scene_move_impact_group(
    groups: &mut Vec<SceneMoveImpactGroup>,
    dependency_type: &str,
    record_ids: Vec<String>,
    reason: &str,
) {
    if record_ids.is_empty() {
        return;
    }

    groups.push(SceneMoveImpactGroup {
        dependency_type: dependency_type.to_string(),
        count: record_ids.len(),
        sample_record_ids: record_ids
            .into_iter()
            .take(MAX_SCENE_DELETE_IMPACT_SAMPLE_IDS)
            .collect(),
        reason: reason.to_string(),
    });
}

pub fn scene_move_hard_blocker_from_delete_group(
    group: SceneDeleteImpactGroup,
) -> SceneMoveImpactGroup {
    let reason = match group.dependency_type.as_str() {
        "character_state" => {
            "Character states are committed against the scene id and current story position; a move would need coordinated remapping."
        }
        "revision_marker" => {
            "Revision markers point directly at this scene and would need to stay attached across the move."
        }
        "dual_persona_review" => {
            "Dual-persona reviews are keyed to the exact scene id and would need explicit move-time handling."
        }
        "scene_version" => {
            "Scene history snapshots remain attached to the scene id and need an explicit policy before moving positions."
        }
        "scene_beat_annotation" => {
            "Beat annotations point directly at this scene and would need validation after a move."
        }
        "canonical_fact" => {
            "Canonical facts cite this scene as their source, so a move needs explicit source-position handling."
        }
        "scene_source_link" => {
            "Source links point directly at this scene id and would need validation after a move."
        }
        "relationship_last_scene" => {
            "Relationship recency is anchored to this scene id and current story position; moving it requires repair."
        }
        _ => group.reason.as_str(),
    };

    SceneMoveImpactGroup {
        dependency_type: group.dependency_type,
        count: group.count,
        sample_record_ids: group.sample_record_ids,
        reason: reason.to_string(),
    }
}

pub fn scene_move_semantic_risk_from_delete_group(
    group: SceneDeleteImpactGroup,
) -> SceneMoveImpactGroup {
    let reason = match group.dependency_type.as_str() {
        "narrative_promise_planted_at" => {
            "Narrative promises planted at the source position would become stale after a move."
        }
        "narrative_promise_planned_payoff" => {
            "Planned promise payoffs scheduled at the source position would need manual repositioning after a move."
        }
        "future_knowledge_learned_at" => {
            "Future-knowledge acquisition tied to the source position would become stale after a move."
        }
        "future_knowledge_expires_at" => {
            "Future-knowledge expiry anchored to the source position would need manual repositioning after a move."
        }
        "timeline_event_placement" => {
            "Timeline events placed at the source position would become semantically wrong after a move."
        }
        "character_arc_milestone" => {
            "Character-arc milestones scheduled at the source position would need manual repositioning after a move."
        }
        "plot_line_convergence_point" => {
            "Plot-line convergence points anchored to the source position would become stale after a move."
        }
        "theme_introduction_point" => {
            "Theme introductions placed at the source position would need manual repositioning after a move."
        }
        "theme_resolution_point" => {
            "Theme resolutions placed at the source position would need manual repositioning after a move."
        }
        "conflict_stated_consequence" => {
            "Conflict consequences first stated at the source position would become semantically stale after a move."
        }
        _ => group.reason.as_str(),
    };

    SceneMoveImpactGroup {
        dependency_type: group.dependency_type,
        count: group.count,
        sample_record_ids: group.sample_record_ids,
        reason: reason.to_string(),
    }
}

pub fn summarize_scene_delete_impact(impact: &GetSceneDeleteImpactOutput) -> String {
    let mut parts = Vec::new();
    append_scene_delete_group_summary(&mut parts, "blockers", &impact.hard_blockers);
    append_scene_delete_group_summary(&mut parts, "semantic risks", &impact.semantic_risks);
    append_scene_delete_group_summary(&mut parts, "chapter artifacts", &impact.chapter_artifacts);
    if parts.is_empty() {
        return format!(
            "scene {} on branch {} is not clear for deletion",
            impact.scene.scene_id, impact.active_branch_name
        );
    }
    format!(
        "scene {} on branch {} has {}",
        impact.scene.scene_id,
        impact.active_branch_name,
        parts.join("; ")
    )
}

pub fn summarize_operator_delete_scene_blockers(
    unsupported_hard_blockers: &[SceneDeleteImpactGroup],
    semantic_risks: &[SceneDeleteImpactGroup],
    unsupported_chapter_artifacts: &[SceneDeleteImpactGroup],
    inherited_chapter_artifacts: bool,
) -> String {
    let mut parts = Vec::new();
    append_scene_delete_group_summary(
        &mut parts,
        "unsupported blockers",
        unsupported_hard_blockers,
    );
    append_scene_delete_group_summary(&mut parts, "semantic risks", semantic_risks);
    append_scene_delete_group_summary(
        &mut parts,
        "unsupported chapter artifacts",
        unsupported_chapter_artifacts,
    );
    if inherited_chapter_artifacts {
        parts.push(
            "chapter artifacts are inherited from main on the active branch and cannot be invalidated safely"
                .to_string(),
        );
    }

    let base = "operator_delete_scene only supports cleanup of scene_source_link blockers and invalidation of chapter_plan_scene/chapter_summary artifacts";
    if parts.is_empty() {
        base.to_string()
    } else {
        format!("{base}; {}", parts.join("; "))
    }
}

pub fn summarize_scene_move_impact(impact: &GetSceneMoveImpactOutput) -> String {
    let mut parts = Vec::new();
    append_scene_move_group_summary(&mut parts, "blockers", &impact.hard_blockers);
    append_scene_move_group_summary(&mut parts, "semantic risks", &impact.semantic_risks);
    append_scene_move_group_summary(&mut parts, "chapter artifacts", &impact.chapter_artifacts);
    if parts.is_empty() {
        return format!(
            "scene {} on branch {} is not clear for movement",
            impact.scene.scene_id, impact.active_branch_name
        );
    }
    format!(
        "scene {} on branch {} has {}",
        impact.scene.scene_id,
        impact.active_branch_name,
        parts.join("; ")
    )
}

pub fn append_scene_delete_group_summary(
    parts: &mut Vec<String>,
    label: &str,
    groups: &[SceneDeleteImpactGroup],
) {
    if groups.is_empty() {
        return;
    }
    let detail = groups
        .iter()
        .map(|group| format!("{} ({})", group.dependency_type, group.count))
        .collect::<Vec<_>>()
        .join(", ");
    parts.push(format!("{label}: {detail}"));
}

pub fn append_scene_move_group_summary(
    parts: &mut Vec<String>,
    label: &str,
    groups: &[SceneMoveImpactGroup],
) {
    if groups.is_empty() {
        return;
    }
    let detail = groups
        .iter()
        .map(|group| format!("{} ({})", group.dependency_type, group.count))
        .collect::<Vec<_>>()
        .join(", ");
    parts.push(format!("{label}: {detail}"));
}

// =============================================================================
// Consistency-scope helpers (operate on SQLite records).
// =============================================================================

pub fn scoped_chapter_plans(plans: Vec<ChapterPlan>, scope: &ConsistencyScope) -> Vec<ChapterPlan> {
    plans
        .into_iter()
        .filter(|plan| scope_contains_chapter(scope, plan.book_number, plan.chapter_number))
        .collect()
}

/// Filter scenes by a `ConsistencyScope`. Mirrors `scoped_scenes` from
/// `services/mod.rs:18864` in 705b835^.
pub fn scoped_scenes(scenes: Vec<Scene>, scope: &ConsistencyScope) -> Vec<Scene> {
    scenes
        .into_iter()
        .filter(|scene| {
            scope_contains_position(
                scope,
                scene.book_number,
                scene.chapter_number,
                scene.scene_order,
            )
        })
        .collect()
}

/// Filter chapter summaries by a `ConsistencyScope`. Mirrors
/// `scoped_chapter_summaries` from `services/mod.rs:18878` in 705b835^.
pub fn scoped_chapter_summaries(
    summaries: Vec<ChapterSummary>,
    scope: &ConsistencyScope,
) -> Vec<ChapterSummary> {
    summaries
        .into_iter()
        .filter(|summary| {
            scope_contains_chapter(scope, summary.book_number, summary.chapter_number)
        })
        .collect()
}

pub fn scoped_narrative_promises(
    promises: Vec<NarrativePromise>,
    scope: &ConsistencyScope,
) -> Vec<NarrativePromise> {
    promises
        .into_iter()
        .filter(|promise| {
            scope_contains_position(
                scope,
                promise.planted_at.book_number,
                promise.planted_at.chapter_number,
                promise.planted_at.scene_order.unwrap_or(0),
            )
        })
        .collect()
}

pub fn scope_contains_chapter(
    scope: &ConsistencyScope,
    book_number: i32,
    chapter_number: i32,
) -> bool {
    match scope {
        ConsistencyScope::Full => true,
        ConsistencyScope::Book {
            book_number: scoped_book,
        } => book_number == *scoped_book,
        ConsistencyScope::ChapterRange {
            start_book_number,
            start_chapter_number,
            end_book_number,
            end_chapter_number,
        } => {
            let chapter_key = (book_number, chapter_number);
            chapter_key >= (*start_book_number, *start_chapter_number)
                && chapter_key <= (*end_book_number, *end_chapter_number)
        }
    }
}

pub fn scope_contains_position(
    scope: &ConsistencyScope,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> bool {
    match scope {
        ConsistencyScope::Full => true,
        ConsistencyScope::Book {
            book_number: scoped_book,
        } => book_number == *scoped_book,
        ConsistencyScope::ChapterRange {
            start_book_number,
            start_chapter_number,
            end_book_number,
            end_chapter_number,
        } => {
            let position = (book_number, chapter_number, scene_order);
            position >= (*start_book_number, *start_chapter_number, i32::MIN)
                && position <= (*end_book_number, *end_chapter_number, i32::MAX)
        }
    }
}

pub fn chapter_keys_from_scenes(scenes: &[Scene]) -> BTreeSet<(i32, i32)> {
    scenes
        .iter()
        .map(|scene| (scene.book_number, scene.chapter_number))
        .collect()
}

pub fn scene_mentions_rule(scene: &Scene, rule: &WorldRule) -> bool {
    let haystack = format!(
        "{} {} {}",
        scene.summary.to_lowercase(),
        scene.full_text.to_lowercase(),
        scene.tone.clone().unwrap_or_default().to_lowercase()
    );
    let keywords = keyword_tokens(&format!(
        "{} {} {}",
        rule.rule_name, rule.rule_type, rule.description
    ));

    if keywords.is_empty() {
        return true;
    }

    keywords.iter().any(|keyword| haystack.contains(keyword))
}

pub fn world_rule_established_before_scene(rule: &WorldRule, scene: &Scene) -> bool {
    let scene_index = story_index_from_scene(scene);
    rule.established_in
        .as_ref()
        .map(|placement| placement.book_number * 10_000 + placement.chapter_number * 100)
        .is_none_or(|rule_index| rule_index <= scene_index)
}

pub fn keyword_tokens(input: &str) -> BTreeSet<String> {
    input
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_lowercase();
            if token.len() >= 4 { Some(token) } else { None }
        })
        .collect()
}

pub fn story_index_from_placement(placement: &StoredStoryPlacement) -> i32 {
    placement.book_number * 10_000
        + placement.chapter_number * 100
        + placement.scene_order.unwrap_or(0)
}

pub fn end_scope_index(scope: &ConsistencyScope, scenes: &[Scene]) -> Option<i32> {
    scenes
        .last()
        .map(story_index_from_scene)
        .or_else(|| match scope {
            ConsistencyScope::Full => None,
            ConsistencyScope::Book { book_number } => Some(book_number * 10_000),
            ConsistencyScope::ChapterRange {
                end_book_number,
                end_chapter_number,
                ..
            } => Some(end_book_number * 10_000 + end_chapter_number * 100),
        })
}

pub fn story_index_from_scene(scene: &Scene) -> i32 {
    scene.book_number * 10_000 + scene.chapter_number * 100 + scene.scene_order
}

// =============================================================================
// Search bible markdown
// =============================================================================

pub fn format_search_bible_markdown(query: &str, results: &[SearchBibleResultItem]) -> String {
    let mut lines = vec![format!("# Search results\n\nQuery: {query}")];
    if results.is_empty() {
        lines.push("\n- No results.".to_string());
        return lines.join("\n");
    }
    lines.push("\n## Matches".to_string());
    for result in results {
        lines.push(format!(
            "- {} (`{}`): {}",
            result.title, result.entity_type, result.excerpt
        ));
    }
    lines.join("\n")
}

// =============================================================================
// Scene context markdown
// =============================================================================

pub fn format_scene_context_markdown(
    standards: Option<&str>,
    hard_constraints: &[HardConstraint],
    novel: &SceneContextNovelLayer,
    scene: &SceneContextSceneLayer,
) -> String {
    let mut lines = vec!["# Scene context".to_string()];

    lines.push(format_scene_context_hard_constraints_markdown(
        hard_constraints,
    ));

    // Forceful, consolidated style contract — rendered before the raw reader
    // contract so the genre-voice requirements are the first thing read after
    // the hard constraints. Never trimmed.
    if let Some(directive) = novel
        .style_directive
        .as_ref()
        .and_then(|directive| directive.render_markdown())
    {
        lines.push(directive);
    }

    lines.push(format_scene_context_reader_contract_markdown(
        &novel.reader_contract,
    ));

    if let Some(standards) = standards.map(str::trim).filter(|value| !value.is_empty()) {
        lines.push("\n## Standards".to_string());
        lines.push(standards.to_string());
    }

    lines.push(format_scene_context_location_markdown(&scene.location));
    lines.push(format_scene_context_agency_warning_markdown(
        &scene.agency_check,
    ));
    lines.push(format_scene_context_agency_check_markdown(
        &scene.agency_check,
    ));
    lines.push(format_scene_context_relationships_markdown(
        &scene.relationships,
    ));
    lines.push(format_scene_context_knowledge_markdown(
        &novel.knowledge_briefing,
    ));
    lines.push(format_scene_context_timeline_markdown(
        &novel.timeline_briefing,
    ));
    lines.push(format_scene_context_future_knowledge_markdown(
        &novel.future_knowledge_briefing,
    ));
    lines.push(format_scene_context_world_state_markdown(
        &scene.world_state,
    ));
    lines.push(format_scene_context_system_overlays_markdown(
        &novel.system_overlays,
    ));
    lines.push(format_scene_context_pacing_markdown(
        &novel.pacing_directives,
    ));
    lines.push(format_scene_context_promises_markdown(
        &novel.narrative_promises_due,
    ));
    lines.push(format_scene_context_characters_markdown(&scene.characters));
    lines.push(scene_context_subjects_markdown(&novel.subjects));
    lines.push(format_scene_context_semantic_references_markdown(
        &novel.semantic_references,
    ));

    lines.join("\n")
}

pub fn format_scene_context_hard_constraints_markdown(
    hard_constraints: &[HardConstraint],
) -> String {
    let mut lines = vec!["\n## Hard constraints".to_string()];
    if hard_constraints.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for constraint in hard_constraints {
            lines.push(format!("- **{}**: {}", constraint.id, constraint.statement));
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_reader_contract_markdown(reader_contract: &ReaderContract) -> String {
    let mut lines = vec!["\n## Reader contract".to_string()];
    lines.push(format!("- Promise: {}", reader_contract.promise));
    push_briefing_list(&mut lines, "- Style notes", &reader_contract.style_notes);
    push_briefing_list(&mut lines, "- Boundaries", &reader_contract.boundaries);
    lines.join("\n")
}

pub fn format_scene_context_location_markdown(location: &LocationSummary) -> String {
    let mut lines = vec!["\n## Location".to_string()];
    lines.push(format!("- Name: {}", location.name));
    lines.push(format!("- Kind: {}", location.kind));
    if let Some(realm) = location.realm.as_deref() {
        lines.push(format!("- Realm: {realm}"));
    }
    lines.push(format!("- Summary: {}", location.summary));
    lines.push(format!("- Location id: {}", location.location_id));
    lines.join("\n")
}

pub fn format_scene_context_agency_warning_markdown(agency_check: &AgencyCheckSummary) -> String {
    let mut lines = vec!["\n## Agency warning".to_string()];
    if let Some(warning) = agency_check.warning.as_deref() {
        lines.push(format!("- Warning: {warning}"));
    } else {
        lines.push("- Warning: none.".to_string());
    }
    lines.join("\n")
}

pub fn format_scene_context_agency_check_markdown(agency_check: &AgencyCheckSummary) -> String {
    let mut lines = vec!["\n## Scene context".to_string()];
    lines.push(format!(
        "- Protagonist character id: {}",
        agency_check
            .protagonist_character_id
            .as_deref()
            .unwrap_or("unknown")
    ));
    lines.push(format!(
        "- Scenes since active choice: {}",
        agency_check.scenes_since_active_choice
    ));
    lines.push(format!(
        "- Needs active choice: {}",
        agency_check.needs_active_choice
    ));
    lines.join("\n")
}

pub fn format_scene_context_relationships_markdown(
    relationships: &[RelationshipSummary],
) -> String {
    let mut lines = vec!["\n## Chapter arc".to_string()];
    if relationships.is_empty() {
        lines.push("- No relationship context resolved.".to_string());
    } else {
        for relationship in relationships {
            lines.push(format!(
                "- {} -> {} [{}], trust={}, tension={}",
                relationship.source_character_id,
                relationship.target_character_id,
                relationship.relationship_type,
                relationship.trust,
                relationship.tension
            ));
            push_briefing_list(&mut lines, "  dynamics", &relationship.dynamics);
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_knowledge_markdown(items: &[KnowledgeBriefingItem]) -> String {
    let mut lines = vec!["\n## Knowledge".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for item in items {
            lines.push(format!(
                "- {} [{}]: {}",
                item.character_id, item.scope, item.fact
            ));
            lines.push(format!("  source: {}", item.source));
            if let Some(learned_at) = item.learned_at.as_ref() {
                lines.push(format!(
                    "  learned at: {}:{}:{}",
                    learned_at.book_number,
                    learned_at.chapter_number,
                    learned_at.scene_order.unwrap_or_default()
                ));
            }
            if let Some(confidence) = item.confidence {
                lines.push(format!("  confidence: {confidence:.2}"));
            }
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_timeline_markdown(items: &[TimelineEventSummary]) -> String {
    let mut lines = vec!["\n## Timeline".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for event in items {
            lines.push(format!(
                "- {} ({}) @ {}:{}:{}",
                event.title,
                event.event_type,
                event.placement.book_number,
                event.placement.chapter_number,
                event.placement.scene_order.unwrap_or_default()
            ));
            lines.push(format!("  {}", event.summary));
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_future_knowledge_markdown(items: &[FutureKnowledgeSummary]) -> String {
    let mut lines = vec!["\n## Future knowledge".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for item in items {
            lines.push(format!(
                "- {}: {}",
                item.character_id, item.knowledge_summary
            ));
            lines.push(format!("  source: {}", item.source));
            lines.push(format!(
                "  learned at: {}:{}:{}",
                item.learned_at.book_number,
                item.learned_at.chapter_number,
                item.learned_at.scene_order.unwrap_or_default()
            ));
            if let Some(expires_at) = item.expires_at.as_ref() {
                lines.push(format!(
                    "  expires at: {}:{}:{}",
                    expires_at.book_number,
                    expires_at.chapter_number,
                    expires_at.scene_order.unwrap_or_default()
                ));
            }
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_world_state_markdown(world_state: &WorldStateSummary) -> String {
    let mut lines = vec!["\n## World state".to_string()];
    lines.push(format!(
        "- Controlling faction: {}",
        world_state
            .controlling_faction
            .as_deref()
            .unwrap_or("unknown")
    ));
    lines.push(format!(
        "- Status: {}",
        world_state.status.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "- Prosperity: {}",
        world_state.prosperity.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "- Stability: {}",
        world_state.stability.as_deref().unwrap_or("unknown")
    ));
    lines.push(format!(
        "- Threat level: {}",
        world_state.threat_level.as_deref().unwrap_or("unknown")
    ));
    push_briefing_list(
        &mut lines,
        "- Sensory details",
        &world_state.sensory_details,
    );
    lines.join("\n")
}

pub fn format_scene_context_system_overlays_markdown(items: &[SystemOverlaySummary]) -> String {
    let mut lines = vec!["\n## System overlays".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for overlay in items {
            lines.push(format!(
                "- {} [{}], visibility: {}",
                overlay.system_name, overlay.system_type, overlay.visibility
            ));
            lines.push(format!("  rules: {}", overlay.rules));
            push_briefing_list(&mut lines, "  stats", &overlay.stats);
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_pacing_markdown(items: &[PacingDirectiveSummary]) -> String {
    let mut lines = vec!["\n## Pacing".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for directive in items {
            lines.push(format!(
                "- Arc {} for character {}",
                directive.character_arc_id, directive.character_id
            ));
            lines.push(format!("  tracker: {}", directive.tracker_id));
            lines.push(format!("  status: {}", directive.status));
            lines.push(format!("  velocity: {}", directive.velocity));
            lines.push(format!(
                "  current progress: {:.2}",
                directive.current_progress
            ));
            push_briefing_list(&mut lines, "  warnings", &directive.warnings);
            if let Some(next_milestone) = directive.next_milestone.as_deref() {
                lines.push(format!("  next milestone: {next_milestone}"));
            }
            lines.push(format!(
                "  budget remaining: {:.2}",
                directive.budget_remaining
            ));
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_promises_markdown(items: &[NarrativePromiseDueSummary]) -> String {
    let mut lines = vec!["\n## Promises".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for promise in items {
            lines.push(format!(
                "- {} [{}] ({}) planted at {}:{}:{}",
                promise.description,
                promise.promise_type,
                promise.status,
                promise.planted_at.book_number,
                promise.planted_at.chapter_number,
                promise.planted_at.scene_order.unwrap_or_default()
            ));
            lines.push(format!("  urgency: {}", promise.urgency));
            lines.push(format!(
                "  chapters since plant: {}",
                promise.chapters_since_plant
            ));
            if let Some(payoff) = promise.planned_payoff.as_ref() {
                lines.push(format!(
                    "  planned payoff: {}:{}:{}",
                    payoff.book_number,
                    payoff.chapter_number,
                    payoff.scene_order.unwrap_or_default()
                ));
            }
            push_briefing_list(&mut lines, "  notes", &promise.notes);
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_characters_markdown(items: &[CharacterStateSummary]) -> String {
    let mut lines = vec!["\n## Characters".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for character in items {
            lines.push(format!(
                "- {} ({}) [{}]",
                character.name, character.role, character.character_id
            ));
            lines.push(format!("  summary: {}", character.summary));
            lines.push(format!(
                "  emotional state: {:?}",
                character.emotional_state
            ));
            push_briefing_list(&mut lines, "  goals", &character.goals);
            push_briefing_list(&mut lines, "  status", &character.status);
            push_briefing_list(&mut lines, "  notes", &character.notes);
        }
    }
    lines.join("\n")
}

pub fn format_scene_context_semantic_references_markdown(
    items: &[SearchBibleResultItem],
) -> String {
    let mut lines = vec!["\n## Semantic references".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for item in items {
            lines.push(format!(
                "- {} ({}) score={:.3}: {}",
                item.title, item.entity_type, item.score, item.excerpt
            ));
        }
    }
    lines.join("\n")
}

pub fn scene_context_subjects_markdown(subjects: &[SnapshotSubject]) -> String {
    if subjects.is_empty() {
        return String::new();
    }

    let mut lines = vec!["\n## Subjects".to_string()];
    for snapshot in subjects {
        lines.push(snapshot.render_markdown(RenderDepth::Standard));
    }
    lines.join("\n\n")
}

// =============================================================================
// Canonical fact rendering helpers (used by chapter-briefing facts section).
// =============================================================================

pub fn canonical_fact_read_model_value_display(fact: &CanonicalFactReadModel) -> String {
    if let Some(value_text) = fact.value_text.as_ref().filter(|value| !value.is_empty()) {
        return value_text.clone();
    }
    if let Some(value_number) = fact.value_number {
        let rendered_number = canonical_fact_float_string(value_number);
        if let Some(unit) = fact.value_unit.as_ref().filter(|unit| !unit.is_empty()) {
            return format!("{rendered_number} {unit}");
        }
        return rendered_number;
    }
    if let Some(value_json) = fact.value_json.as_ref() {
        return value_json.to_string();
    }
    String::new()
}

pub fn canonical_fact_float_string(value: f64) -> String {
    let mut rendered = value.to_string();
    if rendered.contains('.') {
        while rendered.ends_with('0') {
            rendered.pop();
        }
        if rendered.ends_with('.') {
            rendered.pop();
        }
    }
    rendered
}

// =============================================================================
// Writer-state shared helpers
//
// These were the writer-state-specific helpers in the SurrealDB-era
// `services/mod.rs` (lines 18917-19053 in 705b835^). They build the
// `WriterState` payload alongside `get_writer_state` and the
// `format_writer_state_markdown` renderer above. Kept here so the service
// layer stays a thin orchestrator over `Repository` calls plus these pure
// helpers.
// =============================================================================

/// Stable ordering key for "place X within the book/chapter/scene grid".
/// Used to compare arbitrary placements (e.g., promise planted_at vs cursor
/// position). Each book gets 10k slots, each chapter gets 100 — far more than
/// any realistic chapter holds, so collisions are impossible.
pub fn story_index(book_number: i32, chapter_number: i32, scene_order: i32) -> i32 {
    book_number * 10_000 + chapter_number * 100 + scene_order
}

/// Same as [`story_index`] but for chapter-level placements (no scene order).
pub fn chapter_story_index(book_number: i32, chapter_number: i32) -> i32 {
    book_number * 10_000 + chapter_number
}

/// Map the persisted intent string to a typed [`WriterIntent`]. Unknown
/// values fall back to `Drafting` because that's the value writers see most
/// often and the cheapest mismap.
pub fn parse_writer_intent(value: &str) -> WriterIntent {
    match value.trim().to_ascii_lowercase().as_str() {
        "planning" => WriterIntent::Planning,
        "revising" => WriterIntent::Revising,
        "idle" => WriterIntent::Idle,
        _ => WriterIntent::Drafting,
    }
}

/// Projection of a SQLite [`Scene`] record into the
/// [`RecentSceneSummary`] surfaced by `get_writer_state.recent_scenes`.
pub fn writer_state_recent_scene_summary(scene: Scene) -> RecentSceneSummary {
    RecentSceneSummary {
        scene_id: scene.id,
        book_number: scene.book_number,
        chapter_number: scene.chapter_number,
        scene_order: scene.scene_order,
        summary: scene.summary,
        updated_at: scene.updated_at.to_rfc3339(),
    }
}

/// List of section identifiers that the assembled writer state contains.
/// Drives both [`writer_state_includes_section`] and the inclusion metadata
/// exposed via `bundle_summary.included_sections`. Mandatory sections always
/// appear; optional ones are gated on caller flags or actual content.
pub fn writer_state_included_sections(
    include_subjects: bool,
    include_recent_activity: bool,
    include_chapter_outline: bool,
    include_book_outline: bool,
) -> Vec<String> {
    let mut sections = vec![
        "current".to_string(),
        "next".to_string(),
        "hard_constraints".to_string(),
        "recent_scenes".to_string(),
        "open_promises_due_now".to_string(),
        "active_overlays".to_string(),
        "drift_warnings".to_string(),
        "unsynced_local_files".to_string(),
    ];
    if include_subjects {
        sections.push("subjects".to_string());
    }
    if include_recent_activity {
        sections.push("recent_session_activity".to_string());
    }
    if include_chapter_outline {
        sections.push("chapter_outline".to_string());
    }
    if include_book_outline {
        sections.push("book_outline".to_string());
    }
    sections
}

/// Drop a writer-state section in place. Used by
/// [`enforce_writer_state_budget`] to shrink the payload to fit
/// `budget_tokens`. Mirrors the original SurrealDB service helper exactly so
/// JSON-mode trims and Markdown-mode trims agree on what survives.
pub fn trim_writer_state_section(state: &mut WriterState, section_id: &str) {
    match section_id {
        "subjects" => state.subjects.clear(),
        "recent_scenes" => state.recent_scenes.clear(),
        "open_promises_due_now" => state.open_promises_due_now.clear(),
        "active_overlays" => state.active_overlays.clear(),
        "drift_warnings" => state.drift_warnings.clear(),
        "unsynced_local_files" => state.unsynced_local_files.clear(),
        "recent_session_activity" => state.recent_session_activity.clear(),
        "chapter_outline" => state.chapter_outline = None,
        "book_outline" => state.book_outline = None,
        _ => {}
    }
    state
        .bundle_summary
        .included_sections
        .retain(|candidate| candidate != section_id);
}

/// Estimate the token count for a writer-state payload in the requested
/// format. Markdown uses the rendered string; JSON uses the serialized value.
pub fn estimate_writer_state_tokens(format: ContextFormat, state: &WriterState) -> usize {
    match format {
        ContextFormat::Markdown => estimate_text_tokens(&format_writer_state_markdown(state)),
        ContextFormat::Json => {
            estimate_json_tokens(&serde_json::to_value(state).expect("writer state to json"))
        }
    }
}

/// Lower bound on the writer-state token count once every optional section
/// has been trimmed. Used to detect budgets so tight that even mandatory
/// sections wouldn't fit, so we can return an actionable error.
pub fn minimum_writer_state_tokens(format: ContextFormat, state: &WriterState) -> usize {
    let mut minimum_state = state.clone();
    for section_id in WRITER_STATE_TRIMMING_ORDER {
        trim_writer_state_section(&mut minimum_state, section_id);
    }
    estimate_writer_state_tokens(format, &minimum_state)
}

/// Trim optional writer-state sections until the rendered payload fits inside
/// `token_budget`. Marks `bundle_summary.truncated` if anything was dropped
/// (or if the initial render was over budget even when nothing trimmable was
/// available). Errors with `anyhow::bail!` when even the minimum payload
/// exceeds the supplied budget — the original SurrealDB service raised
/// `DomainError::InvalidRequest`; the SQLite stack uses plain `anyhow` so the
/// error message is the user-facing contract here.
pub fn enforce_writer_state_budget(
    format: ContextFormat,
    token_budget: usize,
    state: &mut WriterState,
) -> anyhow::Result<()> {
    let minimum_tokens = minimum_writer_state_tokens(format, state);
    if minimum_tokens > token_budget {
        anyhow::bail!(
            "budget_tokens ({token_budget}) too small to fit mandatory writer-state sections \
             (estimated {minimum_tokens} tokens). Increase budget_tokens or request fewer \
             optional sections."
        );
    }

    let initial_tokens = estimate_writer_state_tokens(format, state);
    let mut estimated_tokens = initial_tokens;
    let mut truncated = false;

    if estimated_tokens > token_budget {
        for section_id in WRITER_STATE_TRIMMING_ORDER {
            if estimated_tokens <= token_budget {
                break;
            }
            if !writer_state_includes_section(state, section_id) {
                continue;
            }
            trim_writer_state_section(state, section_id);
            estimated_tokens = estimate_writer_state_tokens(format, state);
            truncated = true;
        }
    }

    state.bundle_summary.estimated_tokens = estimated_tokens;
    state.bundle_summary.truncated = truncated || initial_tokens > token_budget;
    Ok(())
}

// =============================================================================
// Scene-context / chapter-briefing constants + helpers (ported from
// services/mod.rs in 705b835^).
//
// These are the pure helpers the Tier 1.2 aggregators (`get_scene_context`
// and `get_chapter_briefing`) rely on. They were defined alongside the
// SurrealDB service in the reference; the SQLite stack keeps them here so
// the service stays a thin orchestrator over `Repository` calls plus pure
// projections.
// =============================================================================

pub const DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS: usize = 6000;
/// Default markdown-render token budget for `check_consistency`. Mirrors the
/// SurrealDB-era `DEFAULT_CHECK_CONSISTENCY_BUDGET_TOKENS = 4000`.
pub const DEFAULT_CHECK_CONSISTENCY_BUDGET_TOKENS: usize = 4000;
pub const DEFAULT_CHAPTER_BRIEFING_BUDGET_TOKENS: usize = 8000;
pub const DEFAULT_CHAPTER_BRIEFING_RECENT_LIMIT: usize = 3;
pub const MAX_CHAPTER_BRIEFING_RECENT_LIMIT: usize = 5;

/// Scene-context section ids embedded inside a chapter briefing. Mirrors the
/// reference constant; passed through to `get_scene_context` so the bundled
/// slice keeps a stable shape independent of caller flags.
pub const CHAPTER_BRIEFING_SCENE_CONTEXT_SECTIONS: &[&str] = &[
    "novel",
    "reader_contract",
    "world_rules",
    "system_overlays",
    "timeline_briefing",
    "future_knowledge_briefing",
    "pacing_directives",
    "narrative_promises_due",
    "knowledge_briefing",
    "semantic_references",
    "subjects",
    "scene",
    "location",
    "world_state",
    "characters",
    "relationships",
    "agency_check",
];

/// Empty `ReaderContract` used when the caller opts out of the
/// `reader_contract` section (or when the bundle trimmer drops it).
pub fn empty_reader_contract() -> ReaderContract {
    ReaderContract {
        promise: String::new(),
        style_notes: Vec::new(),
        boundaries: Vec::new(),
    }
}

pub fn empty_location_summary() -> LocationSummary {
    LocationSummary {
        location_id: String::new(),
        name: String::new(),
        kind: String::new(),
        realm: None,
        summary: String::new(),
    }
}

pub fn empty_world_state_summary() -> WorldStateSummary {
    WorldStateSummary {
        controlling_faction: None,
        status: None,
        prosperity: None,
        stability: None,
        threat_level: None,
        sensory_details: Vec::new(),
    }
}

pub fn empty_agency_check_summary() -> AgencyCheckSummary {
    AgencyCheckSummary {
        protagonist_character_id: None,
        scenes_since_active_choice: 0,
        needs_active_choice: false,
        warning: None,
    }
}

// -----------------------------------------------------------------------------
// World-rule relevance filtering. Identical algorithm to the reference: rules
// without `relevance_tags` are always included; tagged rules are kept only if
// at least one tag matches the rendered haystack (location + characters).
// When *every* tagged rule misses, the full set is returned (best-effort).
// -----------------------------------------------------------------------------

/// Lightweight projection of the character data used by the relevance filter.
/// Carries only the fields the haystack-builder reads so callers don't have
/// to plumb full Character records through to a pure helper.
#[derive(Debug, Clone)]
pub struct WorldRuleContextCharacter {
    pub name: String,
    pub role: String,
    pub summary: String,
}

pub fn filter_relevant_world_rules(
    rules: &[WorldRule],
    location: &Location,
    characters: &[WorldRuleContextCharacter],
) -> Vec<WorldRule> {
    if rules.is_empty() {
        return Vec::new();
    }

    let context_terms = world_rule_context_terms(location, characters);
    let context_haystack = context_terms.join(" ");
    let mut filtered = Vec::new();
    let mut saw_tagged_rule = false;
    let mut matched_tagged_rule = false;

    for rule in rules {
        if rule.relevance_tags_or_empty().is_empty() {
            filtered.push(rule.clone());
            continue;
        }

        saw_tagged_rule = true;
        if rule
            .relevance_tags_or_empty()
            .iter()
            .any(|tag| world_rule_tag_matches_context(tag, &context_haystack))
        {
            matched_tagged_rule = true;
            filtered.push(rule.clone());
        }
    }

    if saw_tagged_rule && !matched_tagged_rule {
        return rules.to_vec();
    }

    filtered
}

fn world_rule_context_terms(
    location: &Location,
    characters: &[WorldRuleContextCharacter],
) -> Vec<String> {
    let mut terms = BTreeSet::new();
    collect_relevance_terms(&mut terms, &location.name);
    collect_relevance_terms(&mut terms, &location.kind);
    if let Some(realm) = location.realm.as_deref() {
        collect_relevance_terms(&mut terms, realm);
    }
    collect_relevance_terms(&mut terms, &location.summary);

    for character in characters {
        collect_relevance_terms(&mut terms, &character.name);
        collect_relevance_terms(&mut terms, &character.role);
        collect_relevance_terms(&mut terms, &character.summary);
    }

    terms.into_iter().collect()
}

fn collect_relevance_terms(terms: &mut BTreeSet<String>, text: &str) {
    let normalized = normalize_relevance_text(text);
    if normalized.is_empty() {
        return;
    }

    terms.insert(normalized.clone());
    for token in normalized.split_whitespace() {
        if token.len() >= 4 {
            terms.insert(token.to_string());
        }
    }
}

fn world_rule_tag_matches_context(tag: &str, context_haystack: &str) -> bool {
    let normalized = normalize_relevance_text(tag);
    if normalized.is_empty() {
        return false;
    }
    if normalized == "always" || normalized == "core" {
        return true;
    }
    context_haystack.contains(&normalized)
}

fn normalize_relevance_text(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// -----------------------------------------------------------------------------
// Pacing / promise / agency helpers.
// -----------------------------------------------------------------------------

pub fn pacing_directives_for_characters(
    arcs: &[CharacterArc],
    trackers: &[PacingTracker],
    character_ids: &[String],
) -> Vec<PacingDirectiveSummary> {
    let tracker_by_arc = trackers
        .iter()
        .map(|tracker| (tracker.character_arc_id.clone(), tracker))
        .collect::<std::collections::BTreeMap<_, _>>();

    arcs.iter()
        .filter(|arc| character_ids.contains(&arc.character_id))
        .filter_map(|arc| {
            let tracker = tracker_by_arc.get(&arc.id)?;
            Some(PacingDirectiveSummary {
                character_arc_id: arc.id.clone(),
                tracker_id: tracker.id.clone(),
                character_id: arc.character_id.clone(),
                status: tracker.status.clone(),
                current_progress: tracker.current_progress,
                budget_remaining: tracker.budget_remaining,
                velocity: tracker.velocity.clone(),
                next_milestone: tracker.next_milestone.clone(),
                warnings: tracker.warnings.clone(),
            })
        })
        .collect()
}

pub fn narrative_promise_due_summary(
    promise: &NarrativePromise,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> NarrativePromiseDueSummary {
    let current_index = story_index(book_number, chapter_number, scene_order);
    let planted_index = story_index_from_placement(&promise.planted_at);
    let chapters_since_plant = ((current_index - planted_index).max(0)) / 100;

    let urgency = if let Some(payoff) = promise.planned_payoff.as_ref() {
        let payoff_index = story_index_from_placement(payoff);
        if current_index >= payoff_index {
            "due"
        } else if payoff_index - current_index <= 100 {
            "soon"
        } else {
            "watch"
        }
    } else if chapters_since_plant >= 5 {
        "overdue"
    } else if chapters_since_plant >= 3 {
        "soon"
    } else {
        "watch"
    };

    let mut notes = promise.notes.clone();
    if urgency == "overdue" {
        notes.push("Promise has stayed open long enough to risk narrative drag.".to_string());
    } else if urgency == "due" {
        notes.push("Planned payoff point has arrived or passed.".to_string());
    }

    NarrativePromiseDueSummary {
        narrative_promise_id: promise.id.clone(),
        promise_type: promise.promise_type.clone(),
        description: promise.description.clone(),
        status: promise.status.clone(),
        planted_at: promise.planted_at.clone().into_core(),
        planned_payoff: promise
            .planned_payoff
            .clone()
            .map(|placement| placement.into_core()),
        urgency: urgency.to_string(),
        chapters_since_plant,
        notes,
    }
}

pub fn agency_check_from_scene_history(
    scenes: &[Scene],
    characters: &[CharacterStateSummary],
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> AgencyCheckSummary {
    let protagonist = characters
        .iter()
        .find(|character| character.role.to_ascii_lowercase().contains("protagonist"))
        .or_else(|| characters.first());

    let Some(protagonist) = protagonist else {
        return empty_agency_check_summary();
    };

    let protagonist_name = normalize_relevance_text(&protagonist.name);
    let current_position = (book_number, chapter_number, scene_order);
    let mut scenes_since_active_choice = 0usize;

    for scene in scenes.iter().rev() {
        if (scene.book_number, scene.chapter_number, scene.scene_order) >= current_position {
            continue;
        }

        if scene_shows_active_choice(scene, &protagonist_name) {
            break;
        }

        scenes_since_active_choice += 1;
    }

    let needs_active_choice = scenes_since_active_choice >= 3;
    let warning = needs_active_choice.then(|| {
        format!(
            "{} has gone {} scenes without a clear active choice. Put a costly decision on-page.",
            protagonist.name, scenes_since_active_choice
        )
    });

    AgencyCheckSummary {
        protagonist_character_id: Some(protagonist.character_id.clone()),
        scenes_since_active_choice,
        needs_active_choice,
        warning,
    }
}

fn scene_shows_active_choice(scene: &Scene, protagonist_name: &str) -> bool {
    let summary = normalize_relevance_text(&scene.summary);
    let prose = normalize_relevance_text(&scene.full_text);
    let mentions_protagonist = protagonist_name.is_empty()
        || summary.contains(protagonist_name)
        || prose.contains(protagonist_name);

    if !mentions_protagonist {
        return false;
    }

    let active_verbs = [
        "decides",
        "decided",
        "chooses",
        "chose",
        "commits",
        "committed",
        "resolves",
        "resolved",
        "determines",
        "determined",
        "elects",
        "elected",
        "refuses",
        "refused",
        "rejects",
        "rejected",
        "declines",
        "declined",
        "denies",
        "denied",
        "demands",
        "demanded",
        "insists",
        "insisted",
        "orders",
        "ordered",
        "commands",
        "commanded",
        "declares",
        "declared",
        "announces",
        "announced",
        "confronts",
        "confronted",
        "challenges",
        "challenged",
        "attacks",
        "attacked",
        "defies",
        "defied",
        "resists",
        "resisted",
        "volunteers",
        "volunteered",
        "initiates",
        "initiated",
        "proposes",
        "proposed",
        "offers",
        "offered",
        "sacrifices",
        "sacrificed",
        "risks",
        "risked",
        "gambles",
        "gambled",
        "surrenders",
        "surrendered",
        "abandons",
        "abandoned",
        "leaves",
        "left",
        "departs",
        "departed",
        "retreats",
        "retreated",
        "flees",
        "fled",
        "pursues",
        "pursued",
        "charges",
        "charged",
        "admits",
        "admitted",
        "confesses",
        "confessed",
        "reveals",
        "revealed",
        "takes",
        "took",
        "seizes",
        "seized",
        "claims",
        "claimed",
        "grabs",
        "grabbed",
        "accepts",
        "accepted",
        "asks",
        "asked",
        "persuades",
        "persuaded",
        "convinces",
        "convinced",
        "bargains",
        "bargained",
        "negotiates",
        "negotiated",
        "vows",
        "vowed",
        "swears",
        "swore",
        "promises",
        "promised",
        "bets",
        "bet",
        "dares",
        "dared",
        "pleads",
        "pleaded",
        "forgives",
        "forgave",
        "betrays",
        "betrayed",
        "lies",
        "lied",
        "steals",
        "stole",
        "destroys",
        "destroyed",
        "breaks",
        "broke",
        "fights",
        "fought",
        "kills",
        "killed",
        "saves",
        "saved",
    ];
    let active_phrases = [
        "turns down",
        "turned down",
        "gives up",
        "gave up",
        "gives in",
        "gave in",
        "lets go",
        "let go",
        "passes on",
        "passed on",
        "passes over",
        "passed over",
        "backs down",
        "backed down",
        "stands firm",
        "stood firm",
        "stands up",
        "stood up",
        "steps forward",
        "stepped forward",
        "steps in",
        "stepped in",
        "walks away",
        "walked away",
        "holds back",
        "held back",
        "puts down",
        "put down",
        "takes charge",
        "took charge",
        "makes a choice",
        "made a choice",
        "makes a decision",
        "made a decision",
        "makes up",
        "made up",
        "cuts off",
        "cut off",
        "calls out",
        "called out",
        "owns up",
        "owned up",
        "draws a line",
        "drew a line",
        "takes a stand",
        "took a stand",
        "throws away",
        "threw away",
        "signs away",
        "signed away",
        "hands over",
        "handed over",
        "lays down",
        "laid down",
    ];
    let passive_markers = [
        "is dragged",
        "is forced",
        "is carried",
        "is compelled",
        "is pushed",
        "is pulled",
    ];

    let has_active_verb = active_verbs
        .iter()
        .any(|marker| summary.contains(marker) || prose.contains(marker));
    let has_active_phrase = active_phrases
        .iter()
        .any(|phrase| summary.contains(phrase) || prose.contains(phrase));
    let has_passive = passive_markers
        .iter()
        .any(|marker| summary.contains(marker) || prose.contains(marker));

    (has_active_verb || has_active_phrase) && !has_passive
}

// -----------------------------------------------------------------------------
// Semantic-search query builder + canonical-fact projections.
// -----------------------------------------------------------------------------

pub fn build_context_search_query(
    characters: &[CharacterStateSummary],
    location: &Location,
    world_rules: &[WorldRule],
    book_number: i32,
    chapter_number: i32,
) -> String {
    let character_terms = characters
        .iter()
        .flat_map(|character| {
            character
                .goals
                .iter()
                .chain(character.status.iter())
                .chain(std::iter::once(&character.role))
                .cloned()
        })
        .collect::<Vec<_>>()
        .join(" ");

    let rule_terms = world_rules
        .iter()
        .take(3)
        .map(|rule| format!("{} {}", rule.rule_name, rule.rule_type))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
        "book {} chapter {} {} {} {} {}",
        book_number,
        chapter_number,
        location.name,
        location.kind,
        location.summary,
        [character_terms, rule_terms].join(" ")
    )
}

/// Project a SQLite [`CanonicalFact`] into the `HardConstraint` row consumed
/// by the scene-context / chapter-briefing bundles.
pub fn canonical_fact_hard_constraint(fact: &CanonicalFact) -> HardConstraint {
    HardConstraint {
        id: fact.predicate.clone(),
        statement: canonical_fact_value_display(fact),
    }
}

/// Project a SQLite [`CanonicalFact`] into the public [`CanonicalFactReadModel`].
/// The SQLite record stores `value_number` as `Option<f64>` (the SurrealDB
/// version stored a `serde_json::Number`), so the mapping is mechanical.
pub fn canonical_fact_read_model(fact: &CanonicalFact) -> CanonicalFactReadModel {
    CanonicalFactReadModel {
        canonical_fact_id: fact.id.clone(),
        subject_table: fact.subject_table.clone(),
        subject_id: fact.subject_id.clone(),
        predicate: fact.predicate.clone(),
        value_kind: fact.value_kind.clone(),
        value_text: fact.value_text.clone(),
        value_number: fact.value_number,
        value_unit: fact.unit.clone(),
        value_json: fact.value_json.clone(),
        aliases: fact.aliases.clone(),
        scope: fact.scope.clone(),
        valid_from: fact.valid_from.as_ref().map(|sp| sp.clone().into_core()),
        valid_until: fact.valid_until.as_ref().map(|sp| sp.clone().into_core()),
    }
}

fn canonical_fact_value_display(fact: &CanonicalFact) -> String {
    if let Some(value_text) = fact.value_text.as_ref().filter(|value| !value.is_empty()) {
        return value_text.clone();
    }
    if let Some(value_number) = fact.value_number {
        let rendered_number = canonical_fact_float_string(value_number);
        if let Some(unit) = fact.unit.as_ref().filter(|unit| !unit.is_empty()) {
            return format!("{rendered_number} {unit}");
        }
        return rendered_number;
    }
    if let Some(value_json) = fact.value_json.as_ref() {
        return value_json.to_string();
    }
    String::new()
}

pub fn is_hard_constraint_budget_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("budget_tokens") && message.contains("hard constraints")
}

// -----------------------------------------------------------------------------
// Chapter-briefing recent summary slice.
// -----------------------------------------------------------------------------

pub fn recent_chapter_summaries_for_briefing(
    summaries: Vec<ChapterSummary>,
    book_number: i32,
    chapter_number: i32,
    limit: usize,
) -> Vec<ChapterSummary> {
    if limit == 0 {
        return Vec::new();
    }

    let target_index = chapter_story_index(book_number, chapter_number);
    let mut summaries = summaries
        .into_iter()
        .filter(|summary| {
            chapter_story_index(summary.book_number, summary.chapter_number) < target_index
        })
        .collect::<Vec<_>>();
    summaries.sort_by_key(|summary| {
        std::cmp::Reverse(chapter_story_index(
            summary.book_number,
            summary.chapter_number,
        ))
    });
    summaries.truncate(limit);
    summaries
}

// -----------------------------------------------------------------------------
// ContextBundle integration: a generic `Section` impl plus the per-aggregator
// build/apply helpers. Ported from services/mod.rs:19266..20570 in 705b835^.
// -----------------------------------------------------------------------------

use serde_json::{Value as JsonValue, json};
use spindle_core::context_bundle::{ContextBundle, Section, SectionKind};

pub struct SceneContextBundleSection {
    id: &'static str,
    kind: SectionKind,
    markdown: String,
    json: JsonValue,
}

impl SceneContextBundleSection {
    pub fn new(id: &'static str, kind: SectionKind, markdown: String, json: JsonValue) -> Self {
        Self {
            id,
            kind,
            markdown,
            json,
        }
    }
}

impl Section for SceneContextBundleSection {
    fn kind(&self) -> SectionKind {
        self.kind
    }

    fn id(&self) -> &str {
        self.id
    }

    fn is_empty(&self) -> bool {
        self.markdown.is_empty()
            && match &self.json {
                JsonValue::Null => true,
                JsonValue::Array(items) => items.is_empty(),
                JsonValue::Object(map) => map.is_empty(),
                _ => false,
            }
    }

    fn token_estimate(&self, format: ContextFormat) -> usize {
        match format {
            ContextFormat::Markdown => self.markdown.chars().count() / 4,
            ContextFormat::Json => self.json.to_string().chars().count() / 4,
        }
    }

    fn to_markdown(&self) -> String {
        self.markdown.clone()
    }

    fn to_json_value(&self) -> JsonValue {
        self.json.clone()
    }

    fn clear_content(&mut self) {
        self.markdown.clear();
        self.json = match self.json {
            JsonValue::Array(_) => JsonValue::Array(Vec::new()),
            JsonValue::Object(_) => JsonValue::Object(serde_json::Map::new()),
            _ => JsonValue::Null,
        };
    }
}

// -----------------------------------------------------------------------------
// Scene-context bundle assembly + budget enforcement.
// -----------------------------------------------------------------------------

pub fn build_scene_context_bundle(
    format: ContextFormat,
    budget_tokens: usize,
    hard_constraints: &[HardConstraint],
    subjects: &[SnapshotSubject],
    novel: &SceneContextNovelLayer,
    scene: &SceneContextSceneLayer,
) -> ContextBundle {
    let mut bundle = ContextBundle::new(format).with_budget(budget_tokens);
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "hard_constraints",
        SectionKind::HardConstraint,
        format_scene_context_hard_constraints_markdown(hard_constraints)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "hard_constraints": hard_constraints }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "subjects",
        SectionKind::Supplementary(120),
        scene_context_subjects_markdown(subjects)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "subjects": subjects } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "reader_contract",
        SectionKind::Supplementary(200),
        format_scene_context_reader_contract_markdown(&novel.reader_contract)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "reader_contract": novel.reader_contract } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "location",
        SectionKind::Supplementary(180),
        format_scene_context_location_markdown(&scene.location)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "scene": { "location": scene.location } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "world_state",
        SectionKind::Supplementary(170),
        format_scene_context_world_state_markdown(&scene.world_state)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "scene": { "world_state": scene.world_state } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "characters",
        SectionKind::Supplementary(160),
        format_scene_context_characters_markdown(&scene.characters)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "scene": { "characters": scene.characters } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "relationships",
        SectionKind::Supplementary(150),
        format_scene_context_relationships_markdown(&scene.relationships)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "scene": { "relationships": scene.relationships } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "agency_check",
        SectionKind::Supplementary(140),
        [
            format_scene_context_agency_warning_markdown(&scene.agency_check),
            format_scene_context_agency_check_markdown(&scene.agency_check),
        ]
        .join("\n")
        .trim_start_matches('\n')
        .to_string(),
        json!({ "scene": { "agency_check": scene.agency_check } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "system_overlays",
        SectionKind::Supplementary(6),
        format_scene_context_system_overlays_markdown(&novel.system_overlays)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "system_overlays": novel.system_overlays } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "narrative_promises_due",
        SectionKind::Supplementary(5),
        format_scene_context_promises_markdown(&novel.narrative_promises_due)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "narrative_promises_due": novel.narrative_promises_due } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "pacing_directives",
        SectionKind::Supplementary(4),
        format_scene_context_pacing_markdown(&novel.pacing_directives)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "pacing_directives": novel.pacing_directives } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "future_knowledge_briefing",
        SectionKind::Supplementary(3),
        format_scene_context_future_knowledge_markdown(&novel.future_knowledge_briefing)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "future_knowledge_briefing": novel.future_knowledge_briefing } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "timeline_briefing",
        SectionKind::Supplementary(2),
        format_scene_context_timeline_markdown(&novel.timeline_briefing)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "timeline_briefing": novel.timeline_briefing } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "knowledge_briefing",
        SectionKind::Supplementary(1),
        format_scene_context_knowledge_markdown(&novel.knowledge_briefing)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "knowledge_briefing": novel.knowledge_briefing } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "semantic_references",
        SectionKind::Supplementary(0),
        format_scene_context_semantic_references_markdown(&novel.semantic_references)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "semantic_references": novel.semantic_references } }),
    )));
    bundle
}

pub fn apply_scene_context_bundle_trims(
    truncated_section_ids: &[String],
    novel: &mut SceneContextNovelLayer,
    scene: &mut SceneContextSceneLayer,
) {
    for section_id in truncated_section_ids {
        match section_id.as_str() {
            "subjects" => novel.subjects.clear(),
            "reader_contract" => novel.reader_contract = empty_reader_contract(),
            "system_overlays" => novel.system_overlays.clear(),
            "timeline_briefing" => novel.timeline_briefing.clear(),
            "future_knowledge_briefing" => novel.future_knowledge_briefing.clear(),
            "pacing_directives" => novel.pacing_directives.clear(),
            "narrative_promises_due" => novel.narrative_promises_due.clear(),
            "knowledge_briefing" => novel.knowledge_briefing.clear(),
            "semantic_references" => novel.semantic_references.clear(),
            "location" => scene.location = empty_location_summary(),
            "world_state" => scene.world_state = empty_world_state_summary(),
            "characters" => scene.characters.clear(),
            "relationships" => scene.relationships.clear(),
            "agency_check" => scene.agency_check = empty_agency_check_summary(),
            _ => {}
        }
    }
}

pub fn truncate_markdown_at_line_boundary(markdown: &str, budget_tokens: usize) -> String {
    let max_chars = budget_tokens.saturating_mul(4);
    if markdown.chars().count() <= max_chars {
        return markdown.to_string();
    }

    let mut truncated = String::new();
    for line in markdown.lines() {
        let additional = if truncated.is_empty() {
            line.chars().count()
        } else {
            line.chars().count() + 1
        };
        if truncated.chars().count() + additional > max_chars {
            break;
        }
        if !truncated.is_empty() {
            truncated.push('\n');
        }
        truncated.push_str(line);
    }

    if truncated.is_empty() {
        markdown.chars().take(max_chars).collect()
    } else {
        truncated
    }
}

pub fn estimate_scene_context_tokens(
    format: ContextFormat,
    hard_constraints: &[HardConstraint],
    novel: &SceneContextNovelLayer,
    scene: &SceneContextSceneLayer,
) -> usize {
    match format {
        ContextFormat::Json => estimate_json_tokens(&json!({
            "hard_constraints": hard_constraints,
            "novel": novel,
            "scene": scene,
        })),
        ContextFormat::Markdown => estimate_text_tokens(&format_scene_context_markdown(
            None,
            hard_constraints,
            novel,
            scene,
        )),
    }
}

pub fn non_truncatable_prefix_tokens_scene_context(
    format: ContextFormat,
    hard_constraints: &[HardConstraint],
) -> usize {
    match format {
        ContextFormat::Json => {
            estimate_json_tokens(&serde_json::to_value(hard_constraints).unwrap_or_default())
        }
        ContextFormat::Markdown => {
            let mut prefix = "# Scene context\n\n## Hard constraints\n".to_string();
            if hard_constraints.is_empty() {
                prefix.push_str("- None.\n");
            } else {
                for constraint in hard_constraints {
                    prefix.push_str(&format!(
                        "- **{}**: {}\n",
                        constraint.id, constraint.statement
                    ));
                }
            }
            estimate_text_tokens(&prefix)
        }
    }
}

// -----------------------------------------------------------------------------
// Chapter-briefing bundle assembly + budget enforcement.
// -----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn build_chapter_briefing_bundle(
    format: ContextFormat,
    budget_tokens: usize,
    book_number: i32,
    chapter_number: i32,
    scene_order: Option<i32>,
    hard_constraints: &[HardConstraint],
    canonical_facts: &[CanonicalFactReadModel],
    continuity_sheets: &[SnapshotSubject],
    recent_chapter_summaries: &[ChapterSummaryBriefing],
    chapter_outline: Option<&ChapterOutline>,
    book_outline: Option<&BookOutline>,
    chapter_plan: Option<&ChapterPlanBriefing>,
    scene_context: Option<&SceneContextOutput>,
    scene_seed: &ChapterBriefingSceneSeed,
) -> ContextBundle {
    let mut bundle = ContextBundle::new(format).with_budget(budget_tokens);
    let heading = match scene_order {
        Some(scene_order) => {
            format!(
                "Target scene: Book {book_number}, Chapter {chapter_number}, Scene {scene_order}"
            )
        }
        None => format!("Target chapter: Book {book_number}, Chapter {chapter_number}"),
    };
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "heading",
        SectionKind::HardConstraint,
        format!("# Chapter Briefing\n\n{heading}"),
        json!({ "briefing_markdown": format!("# Chapter Briefing\n\n{heading}") }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "hard_constraints",
        SectionKind::HardConstraint,
        format_chapter_briefing_hard_constraints_markdown(hard_constraints)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "hard_constraints": hard_constraints }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "canonical_facts",
        SectionKind::Supplementary(40),
        format_chapter_briefing_canonical_facts_markdown(canonical_facts)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "canonical_facts": canonical_facts }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "continuity_sheets",
        SectionKind::Supplementary(210),
        format_chapter_briefing_continuity_sheets_markdown(continuity_sheets)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "continuity_sheets": continuity_sheets }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "chapter_outline",
        SectionKind::Supplementary(200),
        chapter_outline
            .map(format_chapter_outline_markdown)
            .unwrap_or_default()
            .trim_start_matches('\n')
            .to_string(),
        json!({ "chapter_outline": chapter_outline }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "chapter_plan",
        SectionKind::Supplementary(175),
        chapter_plan
            .map(format_current_chapter_plan_markdown)
            .unwrap_or_default()
            .trim_start_matches('\n')
            .to_string(),
        json!({ "chapter_plan": chapter_plan }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "book_outline",
        SectionKind::Supplementary(100),
        book_outline
            .map(format_book_outline_markdown)
            .unwrap_or_default()
            .trim_start_matches('\n')
            .to_string(),
        json!({ "book_outline": book_outline }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "recent_chapter_summaries",
        SectionKind::Supplementary(50),
        format_recent_chapter_summaries_markdown(recent_chapter_summaries)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "recent_chapter_summaries": recent_chapter_summaries }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "scene_context",
        SectionKind::Supplementary(25),
        format_chapter_briefing_scene_context_markdown(scene_context, scene_seed)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "scene_context": scene_context, "scene_seed": scene_seed }),
    )));
    bundle
}

#[allow(clippy::too_many_arguments)]
pub fn apply_chapter_briefing_bundle_trims(
    truncated_section_ids: &[String],
    canonical_facts: &mut Vec<CanonicalFactReadModel>,
    continuity_sheets: &mut Vec<SnapshotSubject>,
    recent_chapter_summaries: &mut Vec<ChapterSummaryBriefing>,
    chapter_outline: &mut Option<ChapterOutline>,
    book_outline: &mut Option<BookOutline>,
    chapter_plan: &mut Option<ChapterPlanBriefing>,
    scene_context: &mut Option<SceneContextOutput>,
) {
    for section_id in truncated_section_ids {
        match section_id.as_str() {
            "canonical_facts" => canonical_facts.clear(),
            "continuity_sheets" => continuity_sheets.clear(),
            "recent_chapter_summaries" => recent_chapter_summaries.clear(),
            "chapter_outline" => *chapter_outline = None,
            "book_outline" => *book_outline = None,
            "chapter_plan" => *chapter_plan = None,
            "scene_context" => *scene_context = None,
            _ => {}
        }
    }
}

fn compact_chapter_briefing_constraint_statement(statement: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let normalized = statement.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }

    truncate_at_chars(&normalized, max_chars)
}

fn truncate_at_chars(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.trim().to_string();
    }
    let end_byte = text
        .char_indices()
        .nth(max_chars)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    format!("{}...", text[..end_byte].trim())
}

pub fn fit_chapter_briefing_hard_constraints(
    format: ContextFormat,
    budget_tokens: usize,
    book_number: i32,
    chapter_number: i32,
    scene_order: Option<i32>,
    hard_constraints: &[HardConstraint],
) -> anyhow::Result<(Vec<HardConstraint>, bool)> {
    let compaction_note = |statement: &str| HardConstraint {
        id: "briefing_constraints_compacted".to_string(),
        statement: statement.to_string(),
    };
    let estimated = non_truncatable_prefix_tokens_chapter_briefing(
        format,
        hard_constraints,
        book_number,
        chapter_number,
        scene_order,
    );
    if estimated <= budget_tokens {
        return Ok((hard_constraints.to_vec(), false));
    }

    for max_chars in [160, 72, 0] {
        let compacted = hard_constraints
            .iter()
            .map(|constraint| HardConstraint {
                id: constraint.id.clone(),
                statement: compact_chapter_briefing_constraint_statement(
                    &constraint.statement,
                    max_chars,
                ),
            })
            .collect::<Vec<_>>();
        let compacted_fits = non_truncatable_prefix_tokens_chapter_briefing(
            format,
            &compacted,
            book_number,
            chapter_number,
            scene_order,
        ) <= budget_tokens;
        if !compacted_fits {
            continue;
        }

        let note_statement = if max_chars == 0 {
            "Constraint statements omitted; increase budget_tokens for full details."
        } else {
            "Constraint statements compacted; increase budget_tokens for full details."
        };
        let mut compacted_with_note = compacted.clone();
        compacted_with_note.push(compaction_note(note_statement));
        if non_truncatable_prefix_tokens_chapter_briefing(
            format,
            &compacted_with_note,
            book_number,
            chapter_number,
            scene_order,
        ) <= budget_tokens
        {
            return Ok((compacted_with_note, true));
        }
        return Ok((compacted, true));
    }

    let overflow_note = |omitted: usize| {
        compaction_note(&format!(
            "{omitted} more constraints omitted; increase budget_tokens for the full list."
        ))
    };
    let note_only = vec![overflow_note(hard_constraints.len())];
    if non_truncatable_prefix_tokens_chapter_briefing(
        format,
        &note_only,
        book_number,
        chapter_number,
        scene_order,
    ) > budget_tokens
    {
        anyhow::bail!(
            "budget_tokens ({budget_tokens}) too small to fit even a compact chapter briefing. Increase budget_tokens."
        );
    }

    let mut kept = Vec::new();
    for constraint in hard_constraints {
        let omitted_after = hard_constraints
            .len()
            .saturating_sub(kept.len().saturating_add(1));
        let mut candidate = kept.clone();
        candidate.push(HardConstraint {
            id: constraint.id.clone(),
            statement: String::new(),
        });
        if omitted_after > 0 {
            candidate.push(overflow_note(omitted_after));
        }
        if non_truncatable_prefix_tokens_chapter_briefing(
            format,
            &candidate,
            book_number,
            chapter_number,
            scene_order,
        ) <= budget_tokens
        {
            kept.push(HardConstraint {
                id: constraint.id.clone(),
                statement: String::new(),
            });
        } else {
            break;
        }
    }

    let omitted = hard_constraints.len().saturating_sub(kept.len());
    if omitted > 0 {
        kept.push(overflow_note(omitted));
    }
    Ok((kept, true))
}

fn non_truncatable_prefix_tokens_chapter_briefing(
    format: ContextFormat,
    hard_constraints: &[HardConstraint],
    book_number: i32,
    chapter_number: i32,
    scene_order: Option<i32>,
) -> usize {
    match format {
        ContextFormat::Json => {
            estimate_json_tokens(&serde_json::to_value(hard_constraints).unwrap_or_default())
        }
        ContextFormat::Markdown => {
            let heading = match scene_order {
                Some(so) => format!(
                    "Target scene: Book {book_number}, Chapter {chapter_number}, Scene {so}"
                ),
                None => format!("Target chapter: Book {book_number}, Chapter {chapter_number}"),
            };
            let mut prefix = format!("# Chapter Briefing\n\n{heading}\n\n## Hard constraints\n");
            if hard_constraints.is_empty() {
                prefix.push_str("- None.\n");
            } else {
                for constraint in hard_constraints {
                    prefix.push_str(&format!(
                        "{}\n",
                        format_chapter_briefing_hard_constraint_line(constraint)
                    ));
                }
            }
            estimate_text_tokens(&prefix)
        }
    }
}

// -----------------------------------------------------------------------------
// Projection helpers for the various per-section lists.
// -----------------------------------------------------------------------------

pub fn timeline_event_summary_at_or_before(
    event: TimelineEvent,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> Option<TimelineEventSummary> {
    let cursor = story_index(book_number, chapter_number, scene_order);
    if story_index_from_placement(&event.placement) > cursor {
        return None;
    }
    Some(TimelineEventSummary {
        title: event.title,
        event_type: event.event_type,
        placement: event.placement.into_core(),
        summary: event.summary,
    })
}

pub fn system_overlay_summary(overlay: SystemOverlay) -> SystemOverlaySummary {
    SystemOverlaySummary {
        system_name: overlay.system_name,
        system_type: overlay.system_type,
        visibility: overlay.visibility,
        rules: overlay.rules,
        stats: overlay.stats,
    }
}

pub fn future_knowledge_summary(knowledge: &FutureKnowledge) -> FutureKnowledgeSummary {
    FutureKnowledgeSummary {
        character_id: knowledge.character_id.clone(),
        knowledge_summary: knowledge.knowledge_summary.clone(),
        source: knowledge.source.clone(),
        learned_at: knowledge.learned_at.clone().into_core(),
        expires_at: knowledge
            .expires_at
            .clone()
            .map(|placement| placement.into_core()),
    }
}

pub fn future_knowledge_briefing_item(knowledge: &FutureKnowledge) -> KnowledgeBriefingItem {
    KnowledgeBriefingItem {
        character_id: knowledge.character_id.clone(),
        scope: "future_knowledge".to_string(),
        fact: knowledge.knowledge_summary.clone(),
        source: knowledge.source.clone(),
        learned_at: Some(knowledge.learned_at.clone().into_core()),
        confidence: Some(if knowledge.expires_at.is_some() {
            0.6
        } else {
            0.8
        }),
    }
}

pub fn knowledge_fact_briefing_item(fact: KnowledgeFact) -> KnowledgeBriefingItem {
    KnowledgeBriefingItem {
        character_id: fact.character_id,
        scope: "knowledge_fact".to_string(),
        fact: fact.fact,
        source: fact.source_summary,
        learned_at: fact.learned_at.map(|placement| placement.into_core()),
        confidence: fact.confidence,
    }
}

/// Project a SQLite [`BibleBranch`] record into the [`BranchSummary`]
/// surfaced by `get_writer_state.current.branch` (and several other service
/// orchestrations). `active_branch_id`, when supplied, drives the
/// `is_active` flag — pass the project's active branch id to populate it.
pub fn branch_summary(branch: &BibleBranch, active_branch_id: Option<&str>) -> BranchSummary {
    BranchSummary {
        branch_id: branch.id.clone(),
        name: branch.name.clone(),
        status: branch.status.clone(),
        branch_type: branch.branch_type.clone(),
        description: branch.description.clone(),
        parent_branch_id: branch.parent_branch_id.clone(),
        is_active: active_branch_id == Some(branch.id.as_str()),
    }
}
