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
    ActiveThreadSummary, AgencyCheckSummary, BookOutline, BranchSummary, CanonicalFactReadModel,
    ChapterBriefingSceneSeed, ChapterOutline, ChapterPlanBriefing, ChapterSummaryBriefing,
    CharacterStateSummary, ConsistencyScope, ContextFormat, EconomySummary, FutureKnowledgeSummary,
    GetSceneDeleteImpactOutput, GetSceneMoveImpactOutput, HardConstraint, KnowledgeBriefingItem,
    LocationSummary, NarrativePromiseDueSummary, PacingDirectiveSummary, PreviousSceneTail,
    ReaderContract, RecentSceneSummary, RelationshipSummary, SceneContextNovelLayer,
    SceneContextOutput, SceneContextSceneLayer, SceneDeleteImpactGroup, SceneMoveImpactGroup,
    SearchBibleResultItem, SystemOverlaySummary, TimelineEventSummary, WorldStateSummary,
    WriterIntent, WriterState,
};
use spindle_core::subject_snapshot::{RenderDepth, SubjectSnapshot as SnapshotSubject};

use crate::sqlite::json_records::StoredStoryPlacement;
use crate::sqlite::records::{
    BibleBranch, CanonicalFact, ChapterPlan, ChapterSummary, CharacterArc, Conflict, Economy,
    FutureKnowledge, KnowledgeFact, Location, NarrativePromise, PacingTracker, PlotLine, Scene,
    SceneBeatAnnotation, SystemOverlay, TimelineEvent, WorldRule,
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
    active_threads: &[ActiveThreadSummary],
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
    let active_threads_markdown = format_chapter_briefing_active_threads_markdown(active_threads);
    if !active_threads_markdown.is_empty() {
        lines.push(active_threads_markdown);
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
        .map(|placement| chapter_story_index(placement.book_number, placement.chapter_number))
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

/// Radix for packing a `(book, chapter, scene)` placement into a single, stable,
/// totally-ordered `i64` index. Each chapter gets `SCENE_RADIX` scene slots and
/// each book gets `CHAPTER_RADIX` chapter slots. Placement components are
/// validated at write time (`create_chapter` and scene persistence) to stay
/// strictly below these radixes, so the packing stays collision-free even for
/// books with hundreds of chapters or chapters with hundreds of scenes.
pub const SCENE_RADIX: i64 = 1_000;
pub const CHAPTER_RADIX: i64 = 1_000;
pub const BOOK_RADIX: i64 = CHAPTER_RADIX * SCENE_RADIX;

#[inline]
fn pack_story_index(book_number: i32, chapter_number: i32, scene_order: i32) -> i64 {
    book_number as i64 * BOOK_RADIX + chapter_number as i64 * SCENE_RADIX + scene_order as i64
}

pub fn story_index_from_placement(placement: &StoredStoryPlacement) -> i64 {
    pack_story_index(
        placement.book_number,
        placement.chapter_number,
        placement.scene_order.unwrap_or(0),
    )
}

pub fn end_scope_index(scope: &ConsistencyScope, scenes: &[Scene]) -> Option<i64> {
    scenes
        .last()
        .map(story_index_from_scene)
        .or_else(|| match scope {
            ConsistencyScope::Full => None,
            ConsistencyScope::Book { book_number } => Some((*book_number as i64) * BOOK_RADIX),
            ConsistencyScope::ChapterRange {
                end_book_number,
                end_chapter_number,
                ..
            } => Some(pack_story_index(*end_book_number, *end_chapter_number, 0)),
        })
}

pub fn story_index_from_scene(scene: &Scene) -> i64 {
    pack_story_index(scene.book_number, scene.chapter_number, scene.scene_order)
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
        novel.realized_intensity_trend.as_deref(),
    ));
    lines.push(format_scene_context_promises_markdown(
        &novel.narrative_promises_due,
    ));
    let active_threads_markdown =
        format_scene_context_active_threads_markdown(&novel.active_threads);
    if !active_threads_markdown.is_empty() {
        lines.push(active_threads_markdown);
    }
    let previous_scene_tail_markdown =
        format_scene_context_previous_scene_tail_markdown(novel.previous_scene_tail.as_ref());
    if !previous_scene_tail_markdown.is_empty() {
        lines.push(previous_scene_tail_markdown);
    }
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

pub fn format_scene_context_economy_markdown(items: &[EconomySummary]) -> String {
    let mut lines = vec!["\n## Economies in play".to_string()];
    if items.is_empty() {
        lines.push("- None.".to_string());
    } else {
        for economy in items {
            lines.push(format!(
                "- {} (currency: {})",
                economy.name,
                economy.currency.as_deref().unwrap_or("unspecified")
            ));
            lines.push(format!("  {}", economy.summary));
            if !economy.scarce_resources.is_empty() {
                lines.push(format!("  Scarce: {}", economy.scarce_resources.join(", ")));
            }
            if !economy.trade_goods.is_empty() {
                lines.push(format!("  Trade goods: {}", economy.trade_goods.join(", ")));
            }
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

pub fn format_scene_context_pacing_markdown(
    items: &[PacingDirectiveSummary],
    realized_intensity_trend: Option<&str>,
) -> String {
    let mut lines = vec!["\n## Pacing".to_string()];
    if let Some(trend) = realized_intensity_trend {
        lines.push(format!("- {trend}"));
    }
    if items.is_empty() {
        if realized_intensity_trend.is_none() {
            lines.push("- None.".to_string());
        }
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

/// Maximum length, in characters, of an [`ActiveThreadSummary`] statement.
/// Statements longer than this are truncated on a char boundary.
pub const ACTIVE_THREAD_STATEMENT_CHARS: usize = 240;

/// Char-boundary-safe truncation of a targeted-thread statement so the result
/// is at most [`ACTIVE_THREAD_STATEMENT_CHARS`] characters *including* the
/// trailing ellipsis. The cut always lands on a char boundary, so multibyte
/// text near the budget is never split.
pub fn truncate_active_thread_statement(text: &str) -> String {
    const ELLIPSIS: &str = "...";
    if text.chars().count() <= ACTIVE_THREAD_STATEMENT_CHARS {
        return text.trim().to_string();
    }
    // Reserve room for the ellipsis so the total stays within budget.
    let keep = ACTIVE_THREAD_STATEMENT_CHARS.saturating_sub(ELLIPSIS.chars().count());
    let end_byte = text
        .char_indices()
        .nth(keep)
        .map(|(idx, _)| idx)
        .unwrap_or(text.len());
    format!("{}{ELLIPSIS}", text[..end_byte].trim_end())
}

/// Render one active-thread line: `- [kind] name — statement (status)`, with an
/// appended ` | next: <expectation>` when present.
fn format_active_thread_line(thread: &ActiveThreadSummary) -> String {
    let mut line = format!("- [{}] {} — {}", thread.kind, thread.name, thread.statement);
    if !thread.status.is_empty() {
        line.push_str(&format!(" ({})", thread.status));
    }
    if let Some(next) = thread.next_expectation.as_deref() {
        line.push_str(&format!(" | next: {next}"));
    }
    line
}

/// Render the ACTIVE THREADS block for scene-context markdown. Returns an empty
/// string when there are no threads so the block is omitted entirely.
pub fn format_scene_context_active_threads_markdown(items: &[ActiveThreadSummary]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut lines = vec!["\n## ACTIVE THREADS".to_string()];
    for thread in items {
        lines.push(format_active_thread_line(thread));
    }
    lines.join("\n")
}

/// Maximum length, in characters, of a previous-scene closing excerpt.
pub const PREVIOUS_SCENE_TAIL_CHARS: usize = 1200;

/// Char-boundary-safe closing excerpt: the final at most
/// [`PREVIOUS_SCENE_TAIL_CHARS`] characters of `full_text`, taken from the END.
/// Returns `None` when the prose is empty or whitespace-only, so an empty
/// excerpt is never surfaced. The cut always lands on a char boundary, so
/// multibyte text near the budget is never split.
pub fn scene_closing_excerpt(full_text: &str) -> Option<String> {
    if full_text.trim().is_empty() {
        return None;
    }
    let char_count = full_text.chars().count();
    if char_count <= PREVIOUS_SCENE_TAIL_CHARS {
        return Some(full_text.to_string());
    }
    // Skip the leading chars beyond the budget and start the byte slice at the
    // boundary of the first char we keep, so the tail is always valid UTF-8.
    let skip = char_count - PREVIOUS_SCENE_TAIL_CHARS;
    let start_byte = full_text
        .char_indices()
        .nth(skip)
        .map(|(idx, _)| idx)
        .unwrap_or(full_text.len());
    Some(full_text[start_byte..].to_string())
}

/// Render the PREVIOUS SCENE (closing) block for scene-context markdown.
/// Returns an empty string when there is no preceding-scene tail so the block
/// is omitted entirely.
pub fn format_scene_context_previous_scene_tail_markdown(
    tail: Option<&PreviousSceneTail>,
) -> String {
    let Some(tail) = tail else {
        return String::new();
    };
    format!(
        "\n## PREVIOUS SCENE (closing)\nCh {}.{}: …{}",
        tail.chapter_number, tail.scene_order, tail.excerpt
    )
}

/// Render the ACTIVE THREADS block for chapter-briefing markdown. Returns an
/// empty string when there are no threads so the block is omitted entirely.
pub fn format_chapter_briefing_active_threads_markdown(items: &[ActiveThreadSummary]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let mut lines = vec!["\n## ACTIVE THREADS".to_string()];
    for thread in items {
        lines.push(format_active_thread_line(thread));
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

/// Stable, totally-ordered key for "place X within the book/chapter/scene grid".
/// Used to compare arbitrary placements (e.g., promise `planted_at` vs cursor
/// position). See [`SCENE_RADIX`]/[`CHAPTER_RADIX`]: placement components are
/// validated below their radixes at write time, so the packing is collision-free.
pub fn story_index(book_number: i32, chapter_number: i32, scene_order: i32) -> i64 {
    pack_story_index(book_number, chapter_number, scene_order)
}

/// Same as [`story_index`] but for chapter-level placements (scene order 0), so
/// it stays directly comparable with scene-level indices.
pub fn chapter_story_index(book_number: i32, chapter_number: i32) -> i64 {
    pack_story_index(book_number, chapter_number, 0)
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

pub const DEFAULT_SCENE_CONTEXT_BUDGET_TOKENS: usize = 24_000;
/// Extra room added when mandatory scene hard constraints alone exceed the
/// caller's preferred budget. Hard constraints are canon, not optional prompt
/// filler, so the service expands instead of dropping or rejecting them.
pub const SCENE_CONTEXT_HARD_CONSTRAINT_HEADROOM_TOKENS: usize = 4_000;
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

/// Mean realized intensity per `(book_number, chapter_number)`, computed from
/// scene-beat annotations. Only annotations with a non-NULL intensity attached
/// to one of `scenes` contribute; a chapter whose annotations all lack an
/// intensity value produces no entry (it is treated as unannotated). Shared by
/// the `pacing_drift` consistency check and the `get_scene_context` realized-
/// intensity feed-forward so the two surfaces never disagree about the same
/// per-chapter means. Deterministic (BTreeMap keys iterate in sorted order).
pub(crate) fn chapter_mean_intensities(
    scenes: &[Scene],
    annotations: &[SceneBeatAnnotation],
) -> std::collections::BTreeMap<(i32, i32), f64> {
    use std::collections::BTreeMap;

    let intensity_by_scene: BTreeMap<&str, f64> = annotations
        .iter()
        .filter_map(|annotation| {
            annotation
                .intensity
                .map(|value| (annotation.scene_id.as_str(), value))
        })
        .collect();

    let mut chapter_intensities: BTreeMap<(i32, i32), Vec<f64>> = BTreeMap::new();
    for scene in scenes {
        if let Some(intensity) = intensity_by_scene.get(scene.id.as_str()) {
            chapter_intensities
                .entry((scene.book_number, scene.chapter_number))
                .or_default()
                .push(*intensity);
        }
    }

    chapter_intensities
        .into_iter()
        .map(|(key, values)| {
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            (key, mean)
        })
        .collect()
}

/// Whether the trend across `values` (chronological chapter means) reads as
/// rising, falling, or flat. Flat when the max-min spread is within
/// `REALIZED_INTENSITY_FLAT_EPSILON`; otherwise the sign of last-minus-first
/// decides. Only meaningful for two or more values.
pub(crate) const REALIZED_INTENSITY_FLAT_EPSILON: f64 = 0.05;

/// Number of prior annotated chapters the realized-intensity feed-forward
/// summarizes. Fixed by T-109; not a configurable knob.
pub(crate) const REALIZED_INTENSITY_TREND_WINDOW: usize = 3;

fn realized_intensity_direction(values: &[f64]) -> &'static str {
    let min = values.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    if max - min <= REALIZED_INTENSITY_FLAT_EPSILON {
        "flat"
    } else if values.last() >= values.first() {
        "rising"
    } else {
        "falling"
    }
}

/// Linearly interpolate the expected intensity at `position` (a 0..1 book
/// fraction) from a curve's sampled `intensity_points`. Returns `None` when
/// fewer than two points are present. Points are sorted by position; a position
/// outside the sampled range clamps to the nearest endpoint. Deterministic.
pub(crate) fn interpolate_expected_intensity(
    intensity_points: &[super::sqlite::json_records::StoredIntensityPoint],
    position: f64,
) -> Option<f64> {
    if intensity_points.len() < 2 {
        return None;
    }
    let mut points: Vec<(f64, f64)> = intensity_points
        .iter()
        .map(|point| (point.position, point.intensity))
        .collect();
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    // Clamp outside the sampled range to the nearest endpoint.
    if position <= points[0].0 {
        return Some(points[0].1);
    }
    let last = points[points.len() - 1];
    if position >= last.0 {
        return Some(last.1);
    }
    // Find the surrounding pair and interpolate.
    for window in points.windows(2) {
        let (lo_pos, lo_int) = window[0];
        let (hi_pos, hi_int) = window[1];
        if position >= lo_pos && position <= hi_pos {
            let span = hi_pos - lo_pos;
            if span <= f64::EPSILON {
                return Some(lo_int);
            }
            let t = (position - lo_pos) / span;
            return Some(lo_int + t * (hi_int - lo_int));
        }
    }
    // Unreachable given the clamps above, but stay total.
    Some(last.1)
}

/// Build the realized-intensity feed-forward directive for a scene about to be
/// drafted in `(book_number, chapter_number)`. Summarizes the last up-to-3
/// annotated chapters strictly before `chapter_number` in the same book (a
/// chapter with no non-NULL intensity annotation is excluded). Returns `None`
/// when no prior chapter is annotated; a single-chapter window states the mean
/// with no direction claim; two or three chapters state the sequence and its
/// direction. Values are rounded to two decimals and the directive is capped at
/// 300 chars. Deterministic.
///
/// When the book's pacing curve carries ≥2 `intensity_points` AND the chapter
/// denominator (`max_chapter`) is derivable, the directive is extended with a
/// `; curve expects {e:.2} here` clause where `e` is the linearly interpolated
/// expected intensity at `chapter_number / max_chapter` (clamped to 0..1). With
/// fewer than two points or an underivable denominator, the without-expectation
/// form is emitted verbatim.
pub(crate) fn realized_intensity_trend_directive(
    chapter_means: &std::collections::BTreeMap<(i32, i32), f64>,
    book_number: i32,
    chapter_number: i32,
    intensity_points: &[super::sqlite::json_records::StoredIntensityPoint],
    max_chapter: Option<i32>,
) -> Option<String> {
    let mut prior: Vec<f64> = chapter_means
        .iter()
        .filter(|((book, chapter), _)| *book == book_number && *chapter < chapter_number)
        .map(|(_, mean)| (*mean * 100.0).round() / 100.0)
        .collect();
    if prior.is_empty() {
        return None;
    }
    // `chapter_means` iterates in ascending (book, chapter) order, so `prior` is
    // already chronological; keep only the last `WINDOW` chapters.
    if prior.len() > REALIZED_INTENSITY_TREND_WINDOW {
        prior.drain(0..prior.len() - REALIZED_INTENSITY_TREND_WINDOW);
    }

    let sequence = prior
        .iter()
        .map(|value| format!("{value:.2}"))
        .collect::<Vec<_>>()
        .join(" → ");
    let n = prior.len();

    let mut directive = if n == 1 {
        format!("Realized intensity last {n} chapter: {sequence}")
    } else {
        let direction = realized_intensity_direction(&prior);
        format!("Realized intensity last {n} chapters: {sequence} ({direction})")
    };

    // Expectation clause: only when a curve with ≥2 points and a derivable
    // chapter denominator lets us interpolate an expected intensity here.
    if let Some(denominator) = max_chapter.filter(|max| *max > 0) {
        let position = (chapter_number as f64 / denominator as f64).clamp(0.0, 1.0);
        if let Some(expected) = interpolate_expected_intensity(intensity_points, position) {
            directive.push_str(&format!("; curve expects {expected:.2} here"));
        }
    }

    Some(truncate_at_chars(&directive, 300))
}

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

/// How urgently an open narrative promise needs attention at a given story
/// position. Ordered least-to-most pressing. `Resolved` is returned for
/// promises that are already paid off or abandoned and must never be flagged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseUrgency {
    Resolved,
    Watch,
    Soon,
    Due,
    Overdue,
}

impl PromiseUrgency {
    pub fn as_str(self) -> &'static str {
        match self {
            PromiseUrgency::Resolved => "resolved",
            PromiseUrgency::Watch => "watch",
            PromiseUrgency::Soon => "soon",
            PromiseUrgency::Due => "due",
            PromiseUrgency::Overdue => "overdue",
        }
    }

    /// Least-to-most-pressing ordinal. Used to sort open threads so the most
    /// pressing promises surface first in the persisted digest.
    pub fn rank(self) -> u8 {
        match self {
            PromiseUrgency::Resolved => 0,
            PromiseUrgency::Watch => 1,
            PromiseUrgency::Soon => 2,
            PromiseUrgency::Due => 3,
            PromiseUrgency::Overdue => 4,
        }
    }
}

/// Verdict produced by [`promise_timing_verdict`]: the single source of truth
/// for promise timing, shared by the scene-context "promises due" summary and
/// the `narrative_promise_tracking` consistency check so the two surfaces never
/// disagree about the same promise.
#[derive(Debug, Clone, Copy)]
pub struct PromiseTimingVerdict {
    pub urgency: PromiseUrgency,
    /// Whole in-world chapters elapsed since the promise was planted.
    pub chapters_since_plant: i64,
    /// Whole chapters past the declared `planned_payoff` (0 unless overdue).
    pub overdue_by_chapters: i64,
}

/// Chapters of look-ahead within which an unpaid promise whose `planned_payoff`
/// is approaching is flagged "soon".
pub const PROMISE_DUE_SOON_CHAPTERS: i64 = 3;
/// Chapters past a declared payoff before "due" escalates to "overdue".
pub const PROMISE_OVERDUE_ESCALATE_CHAPTERS: i64 = 2;
/// Fallback aging thresholds (whole chapters) for promises with NO declared
/// `planned_payoff`. Chapter-scaled so deliberately long arcs are not flagged
/// after a handful of scenes; `reinforced` promises get more slack than freshly
/// `planted` ones, preserving the previous intent without the unit bug.
pub const PROMISE_PLANTED_SOON_CHAPTERS: i64 = 8;
pub const PROMISE_PLANTED_OVERDUE_CHAPTERS: i64 = 14;
pub const PROMISE_REINFORCED_SOON_CHAPTERS: i64 = 12;
pub const PROMISE_REINFORCED_OVERDUE_CHAPTERS: i64 = 20;

/// Compute the timing verdict for `promise` at `current_index` (a [`story_index`]
/// value). Honors the author's declared `planned_payoff` when present and falls
/// back to chapter-scaled aging otherwise. Resolved/abandoned promises always
/// return [`PromiseUrgency::Resolved`].
pub fn promise_timing_verdict(
    promise: &NarrativePromise,
    current_index: i64,
) -> PromiseTimingVerdict {
    let planted_index = story_index_from_placement(&promise.planted_at);
    let chapters_since_plant = (current_index - planted_index).max(0) / SCENE_RADIX;

    if promise.status == "paid_off" || promise.status == "abandoned" {
        return PromiseTimingVerdict {
            urgency: PromiseUrgency::Resolved,
            chapters_since_plant,
            overdue_by_chapters: 0,
        };
    }

    if let Some(payoff) = promise.planned_payoff.as_ref() {
        let payoff_index = story_index_from_placement(payoff);
        if current_index >= payoff_index {
            let overdue_by_chapters = (current_index - payoff_index) / SCENE_RADIX;
            let urgency = if overdue_by_chapters >= PROMISE_OVERDUE_ESCALATE_CHAPTERS {
                PromiseUrgency::Overdue
            } else {
                PromiseUrgency::Due
            };
            return PromiseTimingVerdict {
                urgency,
                chapters_since_plant,
                overdue_by_chapters,
            };
        }
        let chapters_until_payoff = (payoff_index - current_index) / SCENE_RADIX;
        let urgency = if chapters_until_payoff <= PROMISE_DUE_SOON_CHAPTERS {
            PromiseUrgency::Soon
        } else {
            PromiseUrgency::Watch
        };
        return PromiseTimingVerdict {
            urgency,
            chapters_since_plant,
            overdue_by_chapters: 0,
        };
    }

    let (soon, overdue) = if promise.status == "reinforced" {
        (
            PROMISE_REINFORCED_SOON_CHAPTERS,
            PROMISE_REINFORCED_OVERDUE_CHAPTERS,
        )
    } else {
        (
            PROMISE_PLANTED_SOON_CHAPTERS,
            PROMISE_PLANTED_OVERDUE_CHAPTERS,
        )
    };
    let urgency = if chapters_since_plant >= overdue {
        PromiseUrgency::Overdue
    } else if chapters_since_plant >= soon {
        PromiseUrgency::Soon
    } else {
        PromiseUrgency::Watch
    };
    PromiseTimingVerdict {
        urgency,
        chapters_since_plant,
        overdue_by_chapters: 0,
    }
}

pub fn narrative_promise_due_summary(
    promise: &NarrativePromise,
    book_number: i32,
    chapter_number: i32,
    scene_order: i32,
) -> NarrativePromiseDueSummary {
    let current_index = story_index(book_number, chapter_number, scene_order);
    let verdict = promise_timing_verdict(promise, current_index);

    let mut notes = promise.notes.clone();
    match verdict.urgency {
        PromiseUrgency::Overdue => notes.push(format!(
            "Promise is {} chapter(s) past its planned payoff and risks narrative drag.",
            verdict.overdue_by_chapters
        )),
        PromiseUrgency::Due => {
            notes.push("Planned payoff point has arrived or passed.".to_string())
        }
        _ => {}
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
        urgency: verdict.urgency.as_str().to_string(),
        chapters_since_plant: verdict.chapters_since_plant as i32,
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
        "active_threads",
        SectionKind::Supplementary(7),
        format_scene_context_active_threads_markdown(&novel.active_threads)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "active_threads": novel.active_threads } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "previous_scene_tail",
        SectionKind::Supplementary(8),
        format_scene_context_previous_scene_tail_markdown(novel.previous_scene_tail.as_ref())
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "previous_scene_tail": novel.previous_scene_tail } }),
    )));
    bundle.push_section(Box::new(SceneContextBundleSection::new(
        "pacing_directives",
        SectionKind::Supplementary(4),
        format_scene_context_pacing_markdown(
            &novel.pacing_directives,
            novel.realized_intensity_trend.as_deref(),
        )
        .trim_start_matches('\n')
        .to_string(),
        json!({
            "novel": {
                "pacing_directives": novel.pacing_directives,
                "realized_intensity_trend": novel.realized_intensity_trend,
            }
        }),
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
        "economy_briefing",
        SectionKind::Supplementary(2),
        format_scene_context_economy_markdown(&novel.economy_briefing)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "novel": { "economy_briefing": novel.economy_briefing } }),
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
            "economy_briefing" => novel.economy_briefing.clear(),
            "future_knowledge_briefing" => novel.future_knowledge_briefing.clear(),
            "pacing_directives" => {
                novel.pacing_directives.clear();
                novel.realized_intensity_trend = None;
            }
            "narrative_promises_due" => novel.narrative_promises_due.clear(),
            "active_threads" => novel.active_threads.clear(),
            "previous_scene_tail" => novel.previous_scene_tail = None,
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
    active_threads: &[ActiveThreadSummary],
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
        "active_threads",
        SectionKind::Supplementary(170),
        format_chapter_briefing_active_threads_markdown(active_threads)
            .trim_start_matches('\n')
            .to_string(),
        json!({ "active_threads": active_threads }),
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
    active_threads: &mut Vec<ActiveThreadSummary>,
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
            "active_threads" => active_threads.clear(),
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

/// Char-boundary-safe truncation to at most `max_chars` characters, appending
/// an ellipsis when the input is longer. The cut always lands on a char
/// boundary so multibyte text is never split. Shared by scene-context
/// compaction and the consistency-check message builders.
pub fn truncate_at_chars(text: &str, max_chars: usize) -> String {
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

/// Project a SQLite [`Economy`] record into the public [`EconomySummary`].
pub fn economy_summary(economy: Economy) -> EconomySummary {
    EconomySummary {
        name: economy.name,
        realm: economy.realm,
        currency: economy.currency,
        summary: economy.summary,
        scarce_resources: economy.scarce_resources,
        trade_goods: economy.trade_goods,
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

/// A canonical-fact assertion reduced to exactly what contradiction detection
/// needs: its `subject_table:subject_id:predicate` key, its canonicalized value,
/// its half-open validity window `[from, until)` as story indices (None =
/// unbounded), and the id of a fact it explicitly supersedes (if any).
#[derive(Debug, Clone)]
pub struct ContradictionCandidate {
    pub id: String,
    pub key: String,
    pub value: String,
    pub from_index: Option<i64>,
    pub until_index: Option<i64>,
    pub supersedes: Option<String>,
}

/// A detected contradiction: two or more candidates sharing a key that carry
/// different values over overlapping validity windows.
#[derive(Debug, Clone)]
pub struct FactContradiction {
    pub composite_key: String,
    pub conflicting_ids: Vec<String>,
    pub values: Vec<String>,
}

/// Build the canonical `subject_table:subject_id:predicate` grouping key. A
/// missing subject id collapses to the project-level `project` sentinel, matching
/// how facts are grouped elsewhere.
pub fn contradiction_subject_key(
    subject_table: &str,
    subject_id: Option<&str>,
    predicate: &str,
) -> String {
    format!(
        "{}:{}:{}",
        subject_table,
        subject_id.unwrap_or("project"),
        predicate
    )
}

/// Half-open `[from, until)` overlap test, treating `None` as unbounded. Adjacent
/// windows (`a.until == b.from`) do NOT overlap, so consecutive evolving states
/// are not flagged.
fn validity_windows_overlap(a: &ContradictionCandidate, b: &ContradictionCandidate) -> bool {
    let a_starts_before_b_ends = match (a.from_index, b.until_index) {
        (Some(a_from), Some(b_until)) => a_from < b_until,
        _ => true,
    };
    let b_starts_before_a_ends = match (b.from_index, a.until_index) {
        (Some(b_from), Some(a_until)) => b_from < a_until,
        _ => true,
    };
    a_starts_before_b_ends && b_starts_before_a_ends
}

/// Scope-aware canonical-fact contradiction detection. Two facts conflict only
/// when they share a key, carry different values, have OVERLAPPING (or unbounded)
/// validity windows, and neither explicitly supersedes the other. Facts whose
/// windows are disjoint (legitimate evolving state across story time) are not
/// reported — this replaces the previous behavior that treated any two differing
/// active facts as a contradiction regardless of their validity windows.
pub fn detect_fact_contradictions(candidates: &[ContradictionCandidate]) -> Vec<FactContradiction> {
    let mut by_key: std::collections::BTreeMap<&str, Vec<&ContradictionCandidate>> =
        std::collections::BTreeMap::new();
    for candidate in candidates {
        by_key
            .entry(candidate.key.as_str())
            .or_default()
            .push(candidate);
    }

    let mut contradictions = Vec::new();
    for (key, group) in by_key {
        if group.len() < 2 {
            continue;
        }
        let mut conflicting_ids: BTreeSet<String> = BTreeSet::new();
        let mut values: BTreeSet<String> = BTreeSet::new();
        for i in 0..group.len() {
            for j in (i + 1)..group.len() {
                let (a, b) = (group[i], group[j]);
                if a.value == b.value {
                    continue;
                }
                if a.supersedes.as_deref() == Some(b.id.as_str())
                    || b.supersedes.as_deref() == Some(a.id.as_str())
                {
                    continue;
                }
                if validity_windows_overlap(a, b) {
                    conflicting_ids.insert(a.id.clone());
                    conflicting_ids.insert(b.id.clone());
                    values.insert(a.value.clone());
                    values.insert(b.value.clone());
                }
            }
        }
        if values.len() >= 2 {
            contradictions.push(FactContradiction {
                composite_key: key.to_string(),
                conflicting_ids: conflicting_ids.into_iter().collect(),
                values: values.into_iter().collect(),
            });
        }
    }
    contradictions
}

/// Canonicalize a stored fact's value: prefer text, then number, then JSON, else
/// the `<unset>` sentinel.
pub fn canonical_fact_value(fact: &crate::sqlite::records::CanonicalFact) -> String {
    if let Some(value) = fact.value_text.clone().filter(|value| !value.is_empty()) {
        return value;
    }
    if let Some(value) = fact
        .value_number
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
    {
        return value;
    }
    if let Some(value) = fact
        .value_json
        .as_ref()
        .map(serde_json::Value::to_string)
        .filter(|value| !value.is_empty())
    {
        return value;
    }
    "<unset>".to_string()
}

/// Reduce a stored canonical fact to a [`ContradictionCandidate`]. Active facts
/// carry no supersedes link (they are the survivors), so `supersedes` is None.
pub fn candidate_from_canonical_fact(
    fact: &crate::sqlite::records::CanonicalFact,
) -> ContradictionCandidate {
    ContradictionCandidate {
        id: fact.id.clone(),
        key: contradiction_subject_key(
            &fact.subject_table,
            fact.subject_id.as_deref(),
            &fact.predicate,
        ),
        value: canonical_fact_value(fact),
        from_index: fact.valid_from.as_ref().map(story_index_from_placement),
        until_index: fact.valid_until.as_ref().map(story_index_from_placement),
        supersedes: None,
    }
}

/// Canonicalize an inbound commit fact entry's value (mirrors
/// [`canonical_fact_value`], with the legacy `value` string as a final fallback).
fn commit_entry_value(entry: &spindle_core::models::CanonicalFactEntry) -> String {
    if let Some(value) = entry.value_text.clone().filter(|value| !value.is_empty()) {
        return value;
    }
    if let Some(value) = entry
        .value_number
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
    {
        return value;
    }
    if let Some(value) = entry
        .value_json
        .as_ref()
        .map(serde_json::Value::to_string)
        .filter(|value| !value.is_empty())
    {
        return value;
    }
    if let Some(value) = entry.value.clone().filter(|value| !value.is_empty()) {
        return value;
    }
    "<unset>".to_string()
}

/// Reduce a prospective commit fact entry to a [`ContradictionCandidate`]. Only
/// typed entries (explicit `subject_table` plus `predicate`/`key`) can be keyed
/// the same way stored facts are; untyped/legacy entries return None and are not
/// checked for contradictions here.
pub fn candidate_from_commit_entry(
    entry: &spindle_core::models::CanonicalFactEntry,
    pending_id: String,
) -> Option<ContradictionCandidate> {
    let subject_table = entry.subject_table.as_deref()?;
    let predicate = entry.predicate.as_deref().or(entry.key.as_deref())?;
    let placement_index = |placement: &spindle_core::models::StoryPlacement| {
        story_index(
            placement.book_number,
            placement.chapter_number,
            placement.scene_order.unwrap_or(0),
        )
    };
    Some(ContradictionCandidate {
        id: pending_id,
        key: contradiction_subject_key(subject_table, entry.subject_id.as_deref(), predicate),
        value: commit_entry_value(entry),
        from_index: entry.valid_from.as_ref().map(placement_index),
        until_index: entry.valid_until.as_ref().map(placement_index),
        supersedes: entry.supersedes_fact_id.clone(),
    })
}

/// Deterministically build a per-book "story so far" synopsis from its ordered
/// `(chapter_number, summary)` pairs, capped to `char_cap`. Under the cap it
/// joins every chapter; over it, it keeps the most recent chapters that fit
/// behind a condensation marker (a model compaction pass can replace this
/// later). Returns `(synopsis, truncated)`.
pub fn build_book_synopsis(parts: &[(i32, String)], char_cap: usize) -> (String, bool) {
    let rendered: Vec<String> = parts
        .iter()
        .map(|(chapter, summary)| format!("Ch {chapter}: {summary}"))
        .collect();
    let full = rendered.join("\n");
    if full.len() <= char_cap {
        return (full, false);
    }
    const MARKER: &str = "[earlier chapters condensed]";
    let mut kept: Vec<&str> = Vec::new();
    let mut used = MARKER.len();
    for line in rendered.iter().rev() {
        let add = line.len() + 1; // newline
        if used + add > char_cap {
            break;
        }
        used += add;
        kept.push(line.as_str());
    }
    kept.reverse();
    let mut out = String::from(MARKER);
    for line in kept {
        out.push('\n');
        out.push_str(line);
    }
    (out, true)
}

/// Maximum number of open-thread entries persisted into a book digest.
pub const OPEN_THREADS_MAX_ENTRIES: usize = 12;
/// Maximum characters retained from a thread's name/description before the
/// status/urgency suffix. Char-boundary safe (never byte-slices).
const OPEN_THREAD_NAME_CHARS: usize = 80;

/// Deterministically collect the still-open narrative threads for a book's
/// "story so far" digest from its unresolved promises, conflicts, and plot
/// lines. Filtering:
///   * promises whose status is neither `paid_off` nor `abandoned`,
///   * conflicts with at least one `stated_consequences[].delivered == false`,
///   * plot lines whose status is not `complete`.
///
/// Each entry renders as `"<kind>: <name-or-description> (<status|urgency>)"`,
/// where the name/description is truncated to [`OPEN_THREAD_NAME_CHARS`] on a
/// char boundary. Ordering is stable: promises first by urgency rank descending
/// then id ascending, then conflicts by id ascending, then plot lines by id
/// ascending. Capped at [`OPEN_THREADS_MAX_ENTRIES`]. `current_index` is a
/// [`story_index`] value used only to derive each promise's urgency label.
pub fn build_open_threads(
    promises: &[NarrativePromise],
    conflicts: &[Conflict],
    plot_lines: &[PlotLine],
    current_index: i64,
) -> Vec<String> {
    let mut open_promises: Vec<(&NarrativePromise, PromiseUrgency)> = promises
        .iter()
        .filter(|p| p.status != "paid_off" && p.status != "abandoned")
        .map(|p| (p, promise_timing_verdict(p, current_index).urgency))
        .collect();
    // Most pressing first (urgency rank desc), ties broken by id ascending.
    open_promises
        .sort_by(|(a, ua), (b, ub)| ub.rank().cmp(&ua.rank()).then_with(|| a.id.cmp(&b.id)));

    let mut open_conflicts: Vec<&Conflict> = conflicts
        .iter()
        .filter(|c| c.stated_consequences.iter().any(|sc| !sc.delivered))
        .collect();
    open_conflicts.sort_by(|a, b| a.id.cmp(&b.id));

    let mut open_plots: Vec<&PlotLine> = plot_lines
        .iter()
        .filter(|pl| pl.status != "complete")
        .collect();
    open_plots.sort_by(|a, b| a.id.cmp(&b.id));

    let mut threads: Vec<String> = Vec::new();
    for (promise, urgency) in open_promises {
        threads.push(format!(
            "promise: {} ({})",
            truncate_open_thread_name(&promise.description),
            urgency.as_str()
        ));
        if threads.len() >= OPEN_THREADS_MAX_ENTRIES {
            return threads;
        }
    }
    for conflict in open_conflicts {
        threads.push(format!(
            "conflict: {} ({})",
            truncate_open_thread_name(&conflict.name),
            conflict.conflict_type
        ));
        if threads.len() >= OPEN_THREADS_MAX_ENTRIES {
            return threads;
        }
    }
    for plot in open_plots {
        threads.push(format!(
            "plot: {} ({})",
            truncate_open_thread_name(&plot.name),
            plot.status
        ));
        if threads.len() >= OPEN_THREADS_MAX_ENTRIES {
            return threads;
        }
    }
    threads
}

/// Char-boundary-safe truncation of a thread name/description to
/// [`OPEN_THREAD_NAME_CHARS`] characters.
fn truncate_open_thread_name(text: &str) -> String {
    text.chars().take(OPEN_THREAD_NAME_CHARS).collect()
}

/// Render the `Open threads: …` segment for one book's [STORY SO FAR] entry,
/// joining thread entries with `; ` and staying within `char_cap` BYTES. Threads
/// are dropped whole from the tail until the segment fits; the final surviving
/// entry may itself be truncated on a char boundary if a single entry exceeds
/// the cap. Returns an empty string when there are no threads or none fit.
pub fn render_open_threads_segment(threads: &[String], char_cap: usize) -> String {
    if threads.is_empty() {
        return String::new();
    }
    const LABEL: &str = "Open threads: ";
    if char_cap <= LABEL.len() {
        return String::new();
    }
    let body_cap = char_cap - LABEL.len();
    let mut body = String::new();
    for thread in threads {
        let candidate_extra = if body.is_empty() {
            thread.len()
        } else {
            "; ".len() + thread.len()
        };
        if body.len() + candidate_extra <= body_cap {
            if !body.is_empty() {
                body.push_str("; ");
            }
            body.push_str(thread);
            continue;
        }
        // This whole thread does not fit. If nothing has been kept yet, keep a
        // char-boundary-safe prefix of the first entry so the segment is not
        // empty; otherwise stop (drop remaining threads).
        if body.is_empty() {
            let remaining = body_cap;
            let mut end = 0usize;
            for (idx, ch) in thread.char_indices() {
                let next = idx + ch.len_utf8();
                if next > remaining {
                    break;
                }
                end = next;
            }
            body.push_str(&thread[..end]);
        }
        break;
    }
    if body.is_empty() {
        return String::new();
    }
    format!("{LABEL}{body}")
}

// =============================================================================
// Secret-knowledge gating: the pure `SecretVisibility` resolver.
//
// Part A of the secret-knowledge gating system (see
// `docs/secret-knowledge-gating-design.md` §2.2). This is the ONE place the
// per-scene context gate decides, for a single secret fact, whether the model
// sees the fact at all and — when it does — which characters are inside its
// circle of trust. The context-gate wiring in `get_scene_context` that calls
// this resolver is deliberately Part B; here we only encode the decision table.
//
// The resolver is fully pure: it takes the fact's circle (each member's
// `character_id` plus the story index at which they entered the circle, or
// `None` for always-known), the scene cast, the POV character, and the scene's
// story cursor, and returns a [`SecretDecision`].
// =============================================================================

/// The gating decision for a single secret fact in a single scene. See the
/// §2.2 decision table in the design doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretDecision {
    /// No circle member (and no POV insider) is present: strip the fact from
    /// every context carrier. The model cannot leak what it never saw (P-A).
    Withhold,
    /// At least one non-POV circle member is present: ship the fact with an
    /// envelope naming who is and is not in the know (P-B).
    Envelope {
        /// Circle members, known at the cursor, who are present in the scene.
        known_to: Vec<String>,
        /// Present cast members who are NOT in the circle — they must not act
        /// on the secret.
        unaware_present: Vec<String>,
        concealment_note: Option<String>,
    },
    /// The POV character is the only present circle member: the same envelope,
    /// but narration may carry the POV's private awareness while dialogue and
    /// other characters' behavior must not.
    PovEnvelope {
        known_to: Vec<String>,
        concealment_note: Option<String>,
    },
}

/// Resolve how a single secret fact should be gated for one scene.
///
/// `circle` is the fact's derived circle of trust: one entry per holder, the
/// story index at which they learned it (`None` = always known, i.e. known
/// from the start). `scene_cast` is the characters present in the scene;
/// `pov_character_id` is the POV, if any. `scene_cursor` is the scene's packed
/// story index. `concealment_note` is optional drafting guidance carried into
/// the envelope.
///
/// Normalization: the POV character always counts as *present* whether or not
/// its id also appears in `scene_cast` — the resolver treats the present set as
/// `scene_cast ∪ {pov}`. Circle membership is evaluated *at the cursor*: a
/// member with `learned_at = Some(idx)` is only in the circle when
/// `idx <= scene_cursor` (so a ch-12 reveal does not leak into a ch-9 flashback
/// drafted later); `learned_at = None` is always in the circle.
///
/// A secret with an empty circle (zero holders) resolves to [`SecretDecision::Withhold`]
/// everywhere — an orphaned secret can never be drafted around. This is
/// intentional and acceptable per the design: a secret nobody holds is inert.
pub fn resolve_secret_visibility(
    circle: &[(String, Option<i64>)],
    scene_cast: &[String],
    pov_character_id: Option<&str>,
    scene_cursor: i64,
    concealment_note: Option<&str>,
) -> SecretDecision {
    // Circle-at-cursor: members whose learned_at is None (always known) or
    // has been reached by this scene's story position.
    let circle_at_cursor: BTreeSet<&str> = circle
        .iter()
        .filter(|(_, learned_at)| learned_at.is_none_or(|idx| idx <= scene_cursor))
        .map(|(id, _)| id.as_str())
        .collect();

    // Present set = cast ∪ {pov}. The POV counts as present even when the cast
    // list omits its id (POV interiority is on-page regardless).
    let mut present: BTreeSet<&str> = scene_cast.iter().map(String::as_str).collect();
    if let Some(pov) = pov_character_id {
        present.insert(pov);
    }

    let pov_in_circle = pov_character_id.is_some_and(|pov| circle_at_cursor.contains(pov));

    // Circle members (at cursor) actually present in the scene.
    let present_circle: Vec<&str> = present
        .iter()
        .copied()
        .filter(|id| circle_at_cursor.contains(id))
        .collect();

    // Non-POV present circle members drive the plain Envelope variant.
    let has_other_present_circle_member = present_circle
        .iter()
        .any(|id| pov_character_id != Some(*id));

    let concealment_note = concealment_note.map(str::to_string);

    if has_other_present_circle_member {
        // ≥1 circle member present (someone other than the POV): full envelope.
        let known_to: Vec<String> = present_circle.iter().map(|id| id.to_string()).collect();
        let unaware_present: Vec<String> = present
            .iter()
            .filter(|id| !circle_at_cursor.contains(*id))
            .map(|id| id.to_string())
            .collect();
        SecretDecision::Envelope {
            known_to,
            unaware_present,
            concealment_note,
        }
    } else if pov_in_circle {
        // POV is the only present circle member: POV-only envelope.
        let known_to: Vec<String> = present_circle.iter().map(|id| id.to_string()).collect();
        SecretDecision::PovEnvelope {
            known_to,
            concealment_note,
        }
    } else {
        // No circle member present and POV not in circle: withhold entirely.
        SecretDecision::Withhold
    }
}

#[cfg(test)]
mod secret_visibility_tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    /// §2.2 row 1: no circle member present and POV not in circle → withhold.
    #[test]
    fn no_circle_member_present_withholds() {
        let circle = vec![("mara".to_string(), None)];
        let decision = resolve_secret_visibility(
            &circle,
            &ids(&["bran", "aldric"]),
            Some("bran"),
            1_000,
            None,
        );
        assert_eq!(decision, SecretDecision::Withhold);
    }

    /// §2.2 row 2: a non-POV circle member present → full envelope naming the
    /// insider (known_to) and the out-of-circle cast (unaware_present).
    #[test]
    fn circle_member_present_yields_envelope() {
        let circle = vec![("mara".to_string(), None)];
        let decision = resolve_secret_visibility(
            &circle,
            &ids(&["mara", "bran"]),
            Some("bran"),
            1_000,
            Some("she deflects with dry humor"),
        );
        match decision {
            SecretDecision::Envelope {
                known_to,
                unaware_present,
                concealment_note,
            } => {
                assert_eq!(known_to, ids(&["mara"]));
                assert_eq!(unaware_present, ids(&["bran"]));
                assert_eq!(
                    concealment_note.as_deref(),
                    Some("she deflects with dry humor")
                );
            }
            other => panic!("expected Envelope, got {other:?}"),
        }
    }

    /// §2.2 row 3: POV is in the circle and is the ONLY present circle member →
    /// POV-only envelope (narration may carry her private awareness).
    #[test]
    fn pov_only_circle_member_yields_pov_envelope() {
        let circle = vec![("mara".to_string(), None)];
        let decision =
            resolve_secret_visibility(&circle, &ids(&["mara", "bran"]), Some("mara"), 1_000, None);
        match decision {
            SecretDecision::PovEnvelope { known_to, .. } => {
                assert_eq!(known_to, ids(&["mara"]));
            }
            other => panic!("expected PovEnvelope, got {other:?}"),
        }
    }

    /// POV counts as present via cast∪{pov} normalization even when the cast
    /// list omits the POV id: a POV insider alone still yields PovEnvelope.
    #[test]
    fn pov_present_via_normalization_when_absent_from_cast() {
        let circle = vec![("mara".to_string(), None)];
        // Cast lists only bran; mara is POV but not in the cast slice.
        let decision =
            resolve_secret_visibility(&circle, &ids(&["bran"]), Some("mara"), 1_000, None);
        match decision {
            SecretDecision::PovEnvelope { known_to, .. } => {
                assert_eq!(known_to, ids(&["mara"]));
            }
            other => panic!("expected PovEnvelope via pov normalization, got {other:?}"),
        }
    }

    /// Flashback cursor: a member who learns the secret at index 500 is NOT in
    /// the circle-at-cursor for a scene at cursor 400 → withhold.
    #[test]
    fn future_reveal_not_in_circle_before_cursor() {
        let circle = vec![("mara".to_string(), Some(500))];
        let decision =
            resolve_secret_visibility(&circle, &ids(&["mara", "bran"]), Some("bran"), 400, None);
        assert_eq!(
            decision,
            SecretDecision::Withhold,
            "a reveal at index 500 must not leak into a scene at cursor 400"
        );
    }

    /// The same member at the same cursor is in-circle once the cursor reaches
    /// the reveal index (learned_at <= cursor).
    #[test]
    fn reveal_in_circle_at_or_after_cursor() {
        let circle = vec![("mara".to_string(), Some(500))];
        let decision =
            resolve_secret_visibility(&circle, &ids(&["mara", "bran"]), Some("bran"), 500, None);
        match decision {
            SecretDecision::Envelope { known_to, .. } => assert_eq!(known_to, ids(&["mara"])),
            other => panic!("expected Envelope at cursor 500, got {other:?}"),
        }
    }

    /// Always-known (`learned_at = None`) members are in the circle at any
    /// cursor, including cursor 0.
    #[test]
    fn always_known_member_in_circle_at_cursor_zero() {
        let circle = vec![("mara".to_string(), None)];
        let decision = resolve_secret_visibility(&circle, &ids(&["mara"]), Some("mara"), 0, None);
        match decision {
            SecretDecision::PovEnvelope { known_to, .. } => assert_eq!(known_to, ids(&["mara"])),
            other => panic!("expected PovEnvelope for always-known member, got {other:?}"),
        }
    }

    /// Degenerate: a secret with zero holders withholds everywhere — an
    /// orphaned secret can never be drafted around (design-acceptable).
    #[test]
    fn empty_circle_withholds_everywhere() {
        let circle: Vec<(String, Option<i64>)> = Vec::new();
        let decision =
            resolve_secret_visibility(&circle, &ids(&["mara", "bran"]), Some("mara"), 1_000, None);
        assert_eq!(decision, SecretDecision::Withhold);
    }

    /// Two present insiders plus an outsider: known_to lists both insiders,
    /// unaware_present lists only the outsider.
    #[test]
    fn multiple_insiders_and_one_outsider() {
        let circle = vec![("mara".to_string(), None), ("aldric".to_string(), None)];
        let decision = resolve_secret_visibility(
            &circle,
            &ids(&["mara", "aldric", "bran"]),
            Some("bran"),
            1_000,
            None,
        );
        match decision {
            SecretDecision::Envelope {
                known_to,
                unaware_present,
                ..
            } => {
                assert_eq!(known_to, ids(&["aldric", "mara"]));
                assert_eq!(unaware_present, ids(&["bran"]));
            }
            other => panic!("expected Envelope, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod promise_timing_tests {
    use super::*;

    fn placement(book: i32, chapter: i32, scene: i32) -> StoredStoryPlacement {
        StoredStoryPlacement {
            book_number: book,
            chapter_number: chapter,
            scene_order: Some(scene),
            note: None,
        }
    }

    fn promise(
        status: &str,
        planted: StoredStoryPlacement,
        payoff: Option<StoredStoryPlacement>,
    ) -> NarrativePromise {
        let now = chrono::Utc::now();
        NarrativePromise {
            id: "narrative_promise:test".to_string(),
            project_id: "project:test".to_string(),
            branch_id: "branch:test".to_string(),
            promise_type: "setup".to_string(),
            description: "a test promise".to_string(),
            status: status.to_string(),
            planted_at: planted,
            planned_payoff: payoff,
            notes: Vec::new(),
            archived_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn planted_promise_does_not_flag_after_a_few_scene_steps() {
        // Regression for the ~100x unit bug: planted b1/ch1, cursor b1/ch1/scene5
        // must read as 0 chapters elapsed and stay on "watch".
        let p = promise("planted", placement(1, 1, 0), None);
        let verdict = promise_timing_verdict(&p, story_index(1, 1, 5));
        assert_eq!(verdict.chapters_since_plant, 0);
        assert_eq!(verdict.urgency, PromiseUrgency::Watch);
    }

    #[test]
    fn long_arc_with_distant_payoff_progresses_watch_soon_due() {
        let p = promise("planted", placement(1, 1, 0), Some(placement(1, 50, 0)));
        assert_eq!(
            promise_timing_verdict(&p, story_index(1, 5, 0)).urgency,
            PromiseUrgency::Watch
        );
        assert_eq!(
            promise_timing_verdict(&p, story_index(1, 49, 0)).urgency,
            PromiseUrgency::Soon
        );
        assert_eq!(
            promise_timing_verdict(&p, story_index(1, 50, 0)).urgency,
            PromiseUrgency::Due
        );
    }

    #[test]
    fn promise_well_past_payoff_is_overdue_with_chapter_count() {
        let p = promise("reinforced", placement(1, 1, 0), Some(placement(1, 10, 0)));
        let verdict = promise_timing_verdict(&p, story_index(1, 14, 0));
        assert_eq!(verdict.urgency, PromiseUrgency::Overdue);
        assert_eq!(verdict.overdue_by_chapters, 4);
    }

    #[test]
    fn resolved_promise_never_flags() {
        let p = promise("paid_off", placement(1, 1, 0), Some(placement(1, 10, 0)));
        let verdict = promise_timing_verdict(&p, story_index(5, 0, 0));
        assert_eq!(verdict.urgency, PromiseUrgency::Resolved);
    }

    #[test]
    fn unscheduled_promise_uses_chapter_scaled_fallback() {
        // No declared payoff: stays "watch" through the early chapters and only
        // ages into "soon"/"overdue" on a chapter scale, not a scene-step scale.
        let p = promise("planted", placement(1, 1, 0), None);
        assert_eq!(
            promise_timing_verdict(&p, story_index(1, 4, 0)).urgency,
            PromiseUrgency::Watch
        );
        assert_eq!(
            promise_timing_verdict(
                &p,
                story_index(1, 1 + PROMISE_PLANTED_SOON_CHAPTERS as i32, 0)
            )
            .urgency,
            PromiseUrgency::Soon
        );
        assert_eq!(
            promise_timing_verdict(
                &p,
                story_index(1, 1 + PROMISE_PLANTED_OVERDUE_CHAPTERS as i32, 0)
            )
            .urgency,
            PromiseUrgency::Overdue
        );
    }

    #[test]
    fn story_index_no_longer_collides_across_book_boundary_within_radix() {
        // Old packing collided: story_index(1,100,0) == story_index(2,0,0).
        assert_ne!(story_index(1, 100, 0), story_index(2, 0, 0));
        // The whole in-radix range of book 1 stays strictly below book 2.
        assert!(
            story_index(1, (CHAPTER_RADIX - 1) as i32, (SCENE_RADIX - 1) as i32)
                < story_index(2, 0, 0)
        );
    }
}

#[cfg(test)]
mod contradiction_tests {
    use super::*;

    fn cand(
        id: &str,
        value: &str,
        from: Option<i64>,
        until: Option<i64>,
        supersedes: Option<&str>,
    ) -> ContradictionCandidate {
        ContradictionCandidate {
            id: id.to_string(),
            key: "character:c1:eye_color".to_string(),
            value: value.to_string(),
            from_index: from,
            until_index: until,
            supersedes: supersedes.map(|s| s.to_string()),
        }
    }

    #[test]
    fn unbounded_facts_with_different_values_conflict() {
        let candidates = vec![
            cand("a", "blue", None, None, None),
            cand("b", "brown", None, None, None),
        ];
        let found = detect_fact_contradictions(&candidates);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].values,
            vec!["blue".to_string(), "brown".to_string()]
        );
    }

    #[test]
    fn disjoint_validity_windows_do_not_conflict() {
        // A holds [day1, day10); B holds [day10, day20): adjacent, no overlap —
        // legitimate evolving state, must NOT be flagged.
        let candidates = vec![
            cand("a", "captain", Some(1), Some(10), None),
            cand("b", "major", Some(10), Some(20), None),
        ];
        assert!(detect_fact_contradictions(&candidates).is_empty());
    }

    #[test]
    fn overlapping_validity_windows_conflict() {
        let candidates = vec![
            cand("a", "captain", Some(1), Some(20), None),
            cand("b", "major", Some(10), Some(30), None),
        ];
        assert_eq!(detect_fact_contradictions(&candidates).len(), 1);
    }

    #[test]
    fn superseding_fact_does_not_conflict() {
        let candidates = vec![
            cand("old", "blue", None, None, None),
            cand("new", "brown", None, None, Some("old")),
        ];
        assert!(detect_fact_contradictions(&candidates).is_empty());
    }

    #[test]
    fn identical_values_never_conflict() {
        let candidates = vec![
            cand("a", "blue", None, None, None),
            cand("b", "blue", None, None, None),
        ];
        assert!(detect_fact_contradictions(&candidates).is_empty());
    }

    #[test]
    fn single_fact_does_not_conflict() {
        assert!(detect_fact_contradictions(&[cand("a", "blue", None, None, None)]).is_empty());
    }
}

#[cfg(test)]
mod book_digest_tests {
    use super::*;

    #[test]
    fn synopsis_joins_all_chapters_under_cap() {
        let parts = vec![(1, "setup".to_string()), (2, "rising".to_string())];
        let (synopsis, truncated) = build_book_synopsis(&parts, 1000);
        assert!(!truncated);
        assert!(synopsis.contains("Ch 1: setup"));
        assert!(synopsis.contains("Ch 2: rising"));
    }

    #[test]
    fn synopsis_keeps_recent_chapters_over_cap() {
        let parts: Vec<(i32, String)> = (1..=10)
            .map(|n| (n, format!("event number {n} occurs here")))
            .collect();
        let (synopsis, truncated) = build_book_synopsis(&parts, 90);
        assert!(truncated, "should report truncation");
        assert!(synopsis.contains("condensed"), "should mark condensation");
        assert!(synopsis.contains("Ch 10:"), "most recent chapter retained");
        assert!(
            !synopsis.contains("Ch 1:"),
            "oldest chapter dropped under cap"
        );
        assert!(synopsis.len() <= 90, "synopsis stays within the char cap");
    }
}

#[cfg(test)]
mod open_threads_tests {
    use super::*;

    fn placement(book: i32, chapter: i32, scene: i32) -> StoredStoryPlacement {
        StoredStoryPlacement {
            book_number: book,
            chapter_number: chapter,
            scene_order: Some(scene),
            note: None,
        }
    }

    fn promise(id: &str, status: &str, description: &str) -> NarrativePromise {
        let now = chrono::Utc::now();
        NarrativePromise {
            id: id.to_string(),
            project_id: "project:test".to_string(),
            branch_id: "branch:test".to_string(),
            promise_type: "setup".to_string(),
            description: description.to_string(),
            status: status.to_string(),
            planted_at: placement(1, 1, 0),
            planned_payoff: None,
            notes: Vec::new(),
            archived_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn conflict(id: &str, name: &str, deliver_done: bool) -> Conflict {
        let now = chrono::Utc::now();
        Conflict {
            id: id.to_string(),
            project_id: "project:test".to_string(),
            branch_id: "branch:test".to_string(),
            name: name.to_string(),
            normalized_name: name.to_ascii_lowercase(),
            conflict_type: "external".to_string(),
            stakes: "everything".to_string(),
            escalation_stages: Vec::new(),
            expected_total_cycles: None,
            try_fail_cycles: Vec::new(),
            stated_consequences: vec![crate::sqlite::json_records::StoredStatedConsequence {
                description: "a stated cost".to_string(),
                stated_at: None,
                must_demonstrate_by: None,
                delivered: deliver_done,
            }],
            resolution_summary: None,
            notes: None,
            archived_at: None,
            created_at: now,
            updated_at: now,
            escalation_demonstrated: Vec::new(),
        }
    }

    fn plot_line(id: &str, name: &str, status: &str) -> PlotLine {
        let now = chrono::Utc::now();
        PlotLine {
            id: id.to_string(),
            project_id: "project:test".to_string(),
            branch_id: "branch:test".to_string(),
            name: name.to_string(),
            normalized_name: name.to_ascii_lowercase(),
            plot_type: "main".to_string(),
            summary: "a plot".to_string(),
            status: status.to_string(),
            convergence_points: Vec::new(),
            notes: None,
            archived_at: None,
            created_at: now,
            updated_at: now,
            connected_conflict_ids: Vec::new(),
            connected_theme_ids: Vec::new(),
        }
    }

    #[test]
    fn collects_unresolved_threads_with_kind_prefixes() {
        let threads = build_open_threads(
            &[promise(
                "narrative_promise:p1",
                "active",
                "the missing heir returns",
            )],
            &[conflict("conflict:c1", "Siege of Ash", false)],
            &[plot_line("plot_line:pl1", "The Rebellion", "developing")],
            story_index(1, 5, 0),
        );
        assert_eq!(threads.len(), 3, "one entry per unresolved thread");
        assert!(
            threads
                .iter()
                .any(|t| t.starts_with("promise:") && t.contains("the missing heir returns")),
            "promise entry present: {threads:?}"
        );
        assert!(
            threads
                .iter()
                .any(|t| t.starts_with("conflict:") && t.contains("Siege of Ash")),
            "conflict entry present: {threads:?}"
        );
        assert!(
            threads
                .iter()
                .any(|t| t.starts_with("plot:") && t.contains("The Rebellion")),
            "plot entry present: {threads:?}"
        );
    }

    #[test]
    fn resolved_threads_are_excluded() {
        // Paid-off/abandoned promise, delivered consequence, complete plot line.
        let threads = build_open_threads(
            &[
                promise("narrative_promise:p1", "paid_off", "resolved promise"),
                promise("narrative_promise:p2", "abandoned", "dropped promise"),
            ],
            &[conflict("conflict:c1", "Done Conflict", true)],
            &[plot_line("plot_line:pl1", "Finished Line", "complete")],
            story_index(1, 5, 0),
        );
        assert!(threads.is_empty(), "no open threads: {threads:?}");
    }

    #[test]
    fn conflict_with_any_undelivered_consequence_is_open() {
        let now = chrono::Utc::now();
        let mut c = conflict("conflict:c1", "Mixed", true);
        c.stated_consequences
            .push(crate::sqlite::json_records::StoredStatedConsequence {
                description: "an undelivered cost".to_string(),
                stated_at: None,
                must_demonstrate_by: None,
                delivered: false,
            });
        c.updated_at = now;
        let threads = build_open_threads(&[], &[c], &[], story_index(1, 5, 0));
        assert_eq!(threads.len(), 1, "still open due to undelivered cost");
    }

    #[test]
    fn deterministic_order_promises_by_urgency_then_id_then_conflicts_then_plots() {
        // Two promises: p_overdue (overdue) should sort ahead of p_watch (watch).
        let mut overdue = promise("narrative_promise:z_overdue", "planted", "overdue one");
        overdue.planted_at = placement(1, 1, 0);
        overdue.planned_payoff = Some(placement(1, 2, 0));
        let mut watch = promise("narrative_promise:a_watch", "planted", "watch one");
        watch.planted_at = placement(1, 1, 0);
        watch.planned_payoff = Some(placement(1, 50, 0));
        let current = story_index(1, 10, 0);

        let a = build_open_threads(
            &[watch.clone(), overdue.clone()],
            &[conflict("conflict:c1", "Cee", false)],
            &[plot_line("plot_line:pl1", "Pee", "developing")],
            current,
        );
        // Reverse input order → identical output (sort is stable/deterministic).
        let b = build_open_threads(
            &[overdue, watch],
            &[conflict("conflict:c1", "Cee", false)],
            &[plot_line("plot_line:pl1", "Pee", "developing")],
            current,
        );
        assert_eq!(a, b, "byte-identical regardless of input order");
        // Overdue promise before watch promise; both before conflict; conflict before plot.
        let idx = |needle: &str| a.iter().position(|t| t.contains(needle)).unwrap();
        assert!(idx("overdue one") < idx("watch one"));
        assert!(idx("watch one") < idx("Cee"));
        assert!(idx("Cee") < idx("Pee"));
    }

    #[test]
    fn caps_at_twelve_entries() {
        let promises: Vec<NarrativePromise> = (0..20)
            .map(|n| promise(&format!("narrative_promise:p{n:02}"), "active", "a promise"))
            .collect();
        let threads = build_open_threads(&promises, &[], &[], story_index(1, 5, 0));
        assert_eq!(threads.len(), 12, "capped at 12 entries");
    }

    #[test]
    fn thread_description_truncated_to_eighty_chars_char_safe() {
        // 100 em-dashes (multibyte) — truncation must land on a char boundary.
        let long = "—".repeat(100);
        let threads = build_open_threads(
            &[promise("narrative_promise:p1", "active", &long)],
            &[],
            &[],
            story_index(1, 5, 0),
        );
        assert_eq!(threads.len(), 1);
        // The rendered name portion holds at most 80 chars of the description.
        let entry = &threads[0];
        assert!(entry.is_char_boundary(0));
        // Whole string is valid UTF-8 by construction; count em-dashes retained.
        let dashes = entry.chars().filter(|c| *c == '—').count();
        assert!(
            dashes <= 80,
            "at most 80 description chars retained: {dashes}"
        );
        assert!(dashes >= 1, "some description retained");
    }

    #[test]
    fn render_segment_stays_within_cap_and_truncates_threads_first() {
        let threads: Vec<String> = (0..15)
            .map(|n| {
                format!("promise: a fairly long open thread description number {n:02} (watch)")
            })
            .collect();
        let segment = render_open_threads_segment(&threads, 120);
        assert!(
            segment.len() <= 120,
            "segment within cap: {}",
            segment.len()
        );
        assert!(segment.starts_with("Open threads:"), "labelled: {segment}");
    }

    #[test]
    fn render_segment_empty_when_no_threads() {
        assert!(render_open_threads_segment(&[], 700).is_empty());
    }

    #[test]
    fn render_segment_char_boundary_safe_with_multibyte() {
        // Force a truncation boundary inside a run of em-dashes.
        let threads = vec![format!("promise: {} (watch)", "—".repeat(200))];
        let segment = render_open_threads_segment(&threads, 40);
        // Must be valid UTF-8 and within the cap.
        assert!(segment.chars().count() <= 40 || segment.len() <= 40 * 4);
        assert!(segment.len() <= 40, "byte cap respected: {}", segment.len());
    }
}

#[cfg(test)]
mod intensity_trend_expectation_tests {
    use super::*;
    use crate::sqlite::json_records::StoredIntensityPoint;

    fn means(entries: &[((i32, i32), f64)]) -> std::collections::BTreeMap<(i32, i32), f64> {
        entries.iter().copied().collect()
    }

    fn point(position: f64, intensity: f64) -> StoredIntensityPoint {
        StoredIntensityPoint {
            position,
            intensity,
        }
    }

    /// Interpolation correctness: points at 0.0→0.2 and 1.0→0.9, drafting
    /// chapter 5 of 10 (position 0.5) → expected 0.55, appended to the trend.
    #[test]
    fn interpolates_expectation_between_two_points() {
        let m = means(&[((1, 1), 0.4), ((1, 2), 0.5), ((1, 3), 0.6)]);
        let points = vec![point(0.0, 0.2), point(1.0, 0.9)];
        let directive =
            realized_intensity_trend_directive(&m, 1, 5, &points, Some(10)).expect("directive");
        assert!(
            directive.contains("curve expects 0.55 here"),
            "expected interpolated 0.55 clause: {directive}"
        );
    }

    /// A single intensity point yields no expectation clause (needs ≥2).
    #[test]
    fn single_point_yields_no_expectation_clause() {
        let m = means(&[((1, 1), 0.4), ((1, 2), 0.5)]);
        let points = vec![point(0.5, 0.5)];
        let directive =
            realized_intensity_trend_directive(&m, 1, 3, &points, Some(10)).expect("directive");
        assert!(
            !directive.contains("curve expects"),
            "single point must not emit an expectation clause: {directive}"
        );
    }

    /// No intensity points → the directive is exactly the without-expectation
    /// form (unchanged from the pre-V0022 behavior).
    #[test]
    fn no_points_leaves_directive_unchanged() {
        let m = means(&[((1, 1), 0.4), ((1, 2), 0.5)]);
        let with_none =
            realized_intensity_trend_directive(&m, 1, 3, &[], Some(10)).expect("directive");
        assert!(
            !with_none.contains("curve expects"),
            "no points must not emit an expectation clause: {with_none}"
        );
        assert!(with_none.starts_with("Realized intensity last"));
    }

    /// Denominator underivable (no max chapter) → no expectation clause even
    /// with ≥2 points.
    #[test]
    fn missing_denominator_yields_no_clause() {
        let m = means(&[((1, 1), 0.4), ((1, 2), 0.5)]);
        let points = vec![point(0.0, 0.2), point(1.0, 0.9)];
        let directive =
            realized_intensity_trend_directive(&m, 1, 3, &points, None).expect("directive");
        assert!(
            !directive.contains("curve expects"),
            "no denominator must not emit an expectation clause: {directive}"
        );
    }

    /// Position past the last point clamps to the nearest (last) point.
    #[test]
    fn position_clamps_to_nearest_point_outside_range() {
        let m = means(&[((1, 8), 0.4)]);
        let points = vec![point(0.0, 0.2), point(0.5, 0.6)];
        // chapter 9 of 10 → position 0.9, past the last point (0.5) → clamp 0.6.
        let directive =
            realized_intensity_trend_directive(&m, 1, 9, &points, Some(10)).expect("directive");
        assert!(
            directive.contains("curve expects 0.60 here"),
            "position past range clamps to nearest point: {directive}"
        );
    }
}
