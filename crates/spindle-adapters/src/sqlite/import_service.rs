//! Service-side helpers for the import pipeline.
//!
//! Sits between `crate::sqlite::service::SqliteSpindleService` and
//! `crate::sqlite::import::*`. Provides the small helpers the service
//! methods need to render record summaries, classify content, walk slicer
//! output back through the persisted segments, and pack hydration
//! progress envelopes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use spindle_core::models::{
    ContentRating, ImportChapterSlice, ImportCharacterDossierSummary, ImportConfidenceLevel,
    ImportEntityClusterSummary, ImportEntityConsolidationReport, ImportEntityExtractionReport,
    ImportEntityKind, ImportEntityMentionSummary, ImportHydrationMode, ImportManuscriptInput,
    ImportNarrativeDossierSummary, ImportPassName, ImportResumeSnapshotSummary,
    ImportReviewItemKind, ImportReviewItemSummary, ImportReviewSeverity, ImportReviewStatus,
    ImportSceneSlice, ImportSessionProgress, ImportSessionStatus, ImportSessionSummary,
    ImportSourceDocumentSummary, ImportSourceFormat, ImportStructuralAnalysisSummary,
    ImportWorldDossierSummary,
};

use crate::sqlite::records::{
    ImportCharacterDossier, ImportEntityCluster, ImportEntityMention, ImportNarrativeDossier,
    ImportResumeSnapshot, ImportReviewItem, ImportSegment, ImportSession, ImportSourceDocument,
    ImportWorldDossier,
};

// =============================================================================
// Pass-name + status + format string codecs (Surreal -> SQLite mechanical).
// =============================================================================

pub fn import_pass_name(value: &ImportPassName) -> String {
    match value {
        ImportPassName::StructuralAnalysis => "structural_analysis",
        ImportPassName::EntityExtraction => "entity_extraction",
        ImportPassName::EntityConsolidation => "entity_consolidation",
        ImportPassName::CharacterAnalysis => "character_analysis",
        ImportPassName::WorldExtraction => "world_extraction",
        ImportPassName::NarrativeAnalysis => "narrative_analysis",
        ImportPassName::FinalState => "final_state",
        ImportPassName::Hydration => "hydration",
    }
    .to_string()
}

pub fn parse_import_pass_name(raw: &str) -> Result<ImportPassName> {
    match raw {
        "structural_analysis" => Ok(ImportPassName::StructuralAnalysis),
        "entity_extraction" => Ok(ImportPassName::EntityExtraction),
        "entity_consolidation" => Ok(ImportPassName::EntityConsolidation),
        "character_analysis" => Ok(ImportPassName::CharacterAnalysis),
        "world_extraction" => Ok(ImportPassName::WorldExtraction),
        "narrative_analysis" => Ok(ImportPassName::NarrativeAnalysis),
        "final_state" => Ok(ImportPassName::FinalState),
        "hydration" => Ok(ImportPassName::Hydration),
        _ => anyhow::bail!("unknown import pass name: {raw}"),
    }
}

pub fn import_session_status_name(value: &ImportSessionStatus) -> String {
    match value {
        ImportSessionStatus::Pending => "pending",
        ImportSessionStatus::Running => "running",
        ImportSessionStatus::ReviewNeeded => "review_needed",
        ImportSessionStatus::ReadyToHydrate => "ready_to_hydrate",
        ImportSessionStatus::Hydrated => "hydrated",
        ImportSessionStatus::Failed => "failed",
    }
    .to_string()
}

pub fn parse_import_session_status(raw: &str) -> Result<ImportSessionStatus> {
    match raw {
        "pending" => Ok(ImportSessionStatus::Pending),
        "running" => Ok(ImportSessionStatus::Running),
        "review_needed" => Ok(ImportSessionStatus::ReviewNeeded),
        "ready_to_hydrate" => Ok(ImportSessionStatus::ReadyToHydrate),
        "hydrated" => Ok(ImportSessionStatus::Hydrated),
        "failed" => Ok(ImportSessionStatus::Failed),
        _ => anyhow::bail!("unknown import session status: {raw}"),
    }
}

pub fn import_hydration_mode_name(value: &ImportHydrationMode) -> String {
    match value {
        ImportHydrationMode::NewProject => "new_project",
        ImportHydrationMode::ExistingProject => "existing_project",
    }
    .to_string()
}

pub fn parse_import_hydration_mode(raw: &str) -> Result<ImportHydrationMode> {
    match raw {
        "new_project" => Ok(ImportHydrationMode::NewProject),
        "existing_project" => Ok(ImportHydrationMode::ExistingProject),
        _ => anyhow::bail!("unknown import hydration mode: {raw}"),
    }
}

pub fn import_source_format_name(value: &ImportSourceFormat) -> String {
    match value {
        ImportSourceFormat::Txt => "txt",
        ImportSourceFormat::Md => "md",
        ImportSourceFormat::Html => "html",
        ImportSourceFormat::Epub => "epub",
        ImportSourceFormat::Docx => "docx",
    }
    .to_string()
}

pub fn parse_import_source_format(raw: &str) -> ImportSourceFormat {
    match raw {
        "md" => ImportSourceFormat::Md,
        "html" => ImportSourceFormat::Html,
        "epub" => ImportSourceFormat::Epub,
        "docx" => ImportSourceFormat::Docx,
        _ => ImportSourceFormat::Txt,
    }
}

pub fn import_entity_kind_name(value: &ImportEntityKind) -> String {
    match value {
        ImportEntityKind::Character => "character",
        ImportEntityKind::Location => "location",
        ImportEntityKind::Faction => "faction",
        ImportEntityKind::Religion => "religion",
        ImportEntityKind::Economy => "economy",
        ImportEntityKind::Term => "term",
        ImportEntityKind::WorldRule => "world_rule",
        ImportEntityKind::PlotLine => "plot_line",
        ImportEntityKind::Conflict => "conflict",
        ImportEntityKind::NarrativePromise => "narrative_promise",
        ImportEntityKind::Theme => "theme",
        ImportEntityKind::Motif => "motif",
        ImportEntityKind::CharacterArc => "character_arc",
        ImportEntityKind::Knowledge => "knowledge",
        ImportEntityKind::Other => "other",
    }
    .to_string()
}

pub fn parse_import_entity_kind(raw: &str) -> ImportEntityKind {
    match raw {
        "character" => ImportEntityKind::Character,
        "location" => ImportEntityKind::Location,
        "faction" => ImportEntityKind::Faction,
        "religion" => ImportEntityKind::Religion,
        "economy" => ImportEntityKind::Economy,
        "term" => ImportEntityKind::Term,
        "world_rule" => ImportEntityKind::WorldRule,
        "plot_line" => ImportEntityKind::PlotLine,
        "conflict" => ImportEntityKind::Conflict,
        "narrative_promise" => ImportEntityKind::NarrativePromise,
        "theme" => ImportEntityKind::Theme,
        "motif" => ImportEntityKind::Motif,
        "character_arc" => ImportEntityKind::CharacterArc,
        "knowledge" => ImportEntityKind::Knowledge,
        _ => ImportEntityKind::Other,
    }
}

pub fn import_review_item_kind_name(value: &ImportReviewItemKind) -> String {
    match value {
        ImportReviewItemKind::Structure => "structure",
        ImportReviewItemKind::Entity => "entity",
        ImportReviewItemKind::Character => "character",
        ImportReviewItemKind::World => "world",
        ImportReviewItemKind::Narrative => "narrative",
        ImportReviewItemKind::FinalState => "final_state",
        ImportReviewItemKind::Knowledge => "knowledge",
        ImportReviewItemKind::ContentRating => "content_rating",
    }
    .to_string()
}

pub fn parse_import_review_item_kind(raw: &str) -> Result<ImportReviewItemKind> {
    match raw {
        "structure" => Ok(ImportReviewItemKind::Structure),
        "entity" => Ok(ImportReviewItemKind::Entity),
        "character" => Ok(ImportReviewItemKind::Character),
        "world" => Ok(ImportReviewItemKind::World),
        "narrative" => Ok(ImportReviewItemKind::Narrative),
        "final_state" => Ok(ImportReviewItemKind::FinalState),
        "knowledge" => Ok(ImportReviewItemKind::Knowledge),
        "content_rating" => Ok(ImportReviewItemKind::ContentRating),
        _ => anyhow::bail!("unknown import review item kind: {raw}"),
    }
}

pub fn import_review_severity_name(value: &ImportReviewSeverity) -> String {
    match value {
        ImportReviewSeverity::Info => "info",
        ImportReviewSeverity::Warning => "warning",
        ImportReviewSeverity::RequiresReview => "requires_review",
    }
    .to_string()
}

pub fn parse_import_review_severity(raw: &str) -> Result<ImportReviewSeverity> {
    match raw {
        "info" => Ok(ImportReviewSeverity::Info),
        "warning" => Ok(ImportReviewSeverity::Warning),
        "requires_review" => Ok(ImportReviewSeverity::RequiresReview),
        _ => anyhow::bail!("unknown import review severity: {raw}"),
    }
}

pub fn import_review_status_name(value: &ImportReviewStatus) -> String {
    match value {
        ImportReviewStatus::Open => "open",
        ImportReviewStatus::Applied => "applied",
        ImportReviewStatus::Resolved => "resolved",
        ImportReviewStatus::Skipped => "skipped",
    }
    .to_string()
}

pub fn parse_import_review_status(raw: &str) -> Result<ImportReviewStatus> {
    match raw {
        "open" => Ok(ImportReviewStatus::Open),
        "applied" => Ok(ImportReviewStatus::Applied),
        "resolved" => Ok(ImportReviewStatus::Resolved),
        "skipped" => Ok(ImportReviewStatus::Skipped),
        _ => anyhow::bail!("unknown import review status: {raw}"),
    }
}

pub fn import_segment_status_name() -> String {
    "ready".to_string()
}

pub fn import_progress(
    total_documents: usize,
    processed_documents: usize,
    total_segments: usize,
    processed_segments: usize,
    total_review_items: usize,
    open_review_items: usize,
) -> ImportSessionProgress {
    ImportSessionProgress {
        total_documents,
        processed_documents,
        total_segments,
        processed_segments,
        total_review_items,
        open_review_items,
    }
}

pub fn import_confidence_level(confidence: f64) -> ImportConfidenceLevel {
    if confidence >= 0.8 {
        ImportConfidenceLevel::High
    } else if confidence >= 0.55 {
        ImportConfidenceLevel::Medium
    } else {
        ImportConfidenceLevel::Low
    }
}

// =============================================================================
// Record -> summary projections.
// =============================================================================

pub fn import_session_summary(session: &ImportSession) -> Result<ImportSessionSummary> {
    Ok(ImportSessionSummary {
        session_id: session.id.clone(),
        project_id: session.project_id.clone(),
        target_branch_id: session.target_branch_id.clone(),
        status: parse_import_session_status(&session.session_status)?,
        active_pass: parse_import_pass_name(&session.active_pass)?,
        source_format: session
            .source_format
            .as_deref()
            .map(parse_import_source_format),
        hydrate_mode: parse_import_hydration_mode(&session.hydrate_mode)?,
        progress: serde_json::from_value(session.progress.clone()).unwrap_or(
            ImportSessionProgress {
                total_documents: 0,
                processed_documents: 0,
                total_segments: 0,
                processed_segments: 0,
                total_review_items: 0,
                open_review_items: 0,
            },
        ),
        started_at: Some(session.imported_at.to_rfc3339()),
        updated_at: Some(session.updated_at.to_rfc3339()),
    })
}

pub fn import_review_item_summary(
    review_item: ImportReviewItem,
) -> Result<ImportReviewItemSummary> {
    Ok(ImportReviewItemSummary {
        review_item_id: review_item.id.clone(),
        session_id: review_item.session_id.clone(),
        pass_name: parse_import_pass_name(&review_item.pass_name)?,
        kind: parse_import_review_item_kind(&review_item.item_kind)?,
        severity: parse_import_review_severity(&review_item.severity)?,
        status: parse_import_review_status(&review_item.status)?,
        title: review_item.title,
        description: review_item.description,
        related_segment_ids: review_item.related_segment_ids,
        related_entity_ids: review_item.related_entity_ids,
        confidence: review_item.confidence,
        proposed_correction: review_item
            .proposed_correction
            .map(serde_json::from_value)
            .transpose()?,
        resolver_notes: review_item.resolver_notes,
    })
}

pub fn entity_extraction_report_from_records(
    mentions: &[ImportEntityMention],
    review_items: &[ImportReviewItem],
) -> ImportEntityExtractionReport {
    use std::collections::BTreeSet;
    let completed_segment_ids = mentions
        .iter()
        .map(|mention| mention.segment_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mentions = mentions
        .iter()
        .cloned()
        .map(|mention| ImportEntityMentionSummary {
            mention_id: mention.id.clone(),
            segment_id: mention.segment_id.clone(),
            entity_kind: parse_import_entity_kind(&mention.entity_kind),
            surface_form: mention.surface_form,
            normalized_name: mention.normalized_name,
            alias_hint: mention.alias_hint,
            surrounding_text: mention.surrounding_text,
            confidence: mention.confidence,
            confidence_level: import_confidence_level(mention.confidence),
        })
        .collect();

    ImportEntityExtractionReport {
        mentions,
        completed_segment_ids,
        review_items_created: review_items
            .iter()
            .filter(|item| item.pass_name == import_pass_name(&ImportPassName::EntityExtraction))
            .count(),
    }
}

pub fn entity_consolidation_report_from_records(
    clusters: &[ImportEntityCluster],
    review_items: &[ImportReviewItem],
) -> ImportEntityConsolidationReport {
    let clusters = clusters
        .iter()
        .cloned()
        .map(|cluster| ImportEntityClusterSummary {
            cluster_id: cluster.id.clone(),
            entity_kind: parse_import_entity_kind(&cluster.entity_kind),
            canonical_name: cluster.canonical_name,
            aliases: cluster.aliases,
            first_segment_id: cluster.first_segment_id,
            last_segment_id: cluster.last_segment_id,
            mention_count: cluster.mention_ids.len(),
            importance_rank: cluster.importance_rank as i32,
            merge_confidence: cluster.merge_confidence,
            confidence_level: import_confidence_level(cluster.merge_confidence),
            review_required: cluster.review_required,
        })
        .collect();

    ImportEntityConsolidationReport {
        clusters,
        review_items_created: review_items
            .iter()
            .filter(|item| item.pass_name == import_pass_name(&ImportPassName::EntityConsolidation))
            .count(),
    }
}

pub fn character_dossier_summaries_from_records(
    dossiers: &[ImportCharacterDossier],
) -> Result<Vec<ImportCharacterDossierSummary>> {
    dossiers
        .iter()
        .cloned()
        .map(|dossier| {
            Ok(ImportCharacterDossierSummary {
                cluster_id: dossier.cluster_id.clone(),
                canonical_name: dossier.canonical_name,
                aliases: dossier.aliases,
                importance_rank: dossier.importance_rank as i32,
                voice_profile: serde_json::from_value(dossier.voice_profile)?,
                emotional_profile: serde_json::from_value(dossier.emotional_profile)?,
                state_trajectory: serde_json::from_value(dossier.state_trajectory)?,
                relationship_inferences: serde_json::from_value(dossier.relationship_inferences)?,
                decision_patterns: dossier.decision_patterns,
                dialogue_samples: dossier.dialogue_samples,
                confidence: dossier.confidence,
                confidence_level: import_confidence_level(dossier.confidence),
            })
        })
        .collect()
}

pub fn world_dossier_summary_from_record(
    dossier: ImportWorldDossier,
) -> Result<ImportWorldDossierSummary> {
    Ok(ImportWorldDossierSummary {
        world_rules: serde_json::from_value(dossier.world_rules)?,
        locations: serde_json::from_value(dossier.locations)?,
        entities: serde_json::from_value(dossier.entities)?,
        system_signals: serde_json::from_value(dossier.system_signals)?,
    })
}

pub fn narrative_dossier_summary_from_record(
    dossier: ImportNarrativeDossier,
) -> Result<ImportNarrativeDossierSummary> {
    Ok(ImportNarrativeDossierSummary {
        plot_lines: serde_json::from_value(dossier.plot_lines)?,
        conflicts: serde_json::from_value(dossier.conflicts)?,
        narrative_promises: serde_json::from_value(dossier.narrative_promises)?,
        arcs: serde_json::from_value(dossier.arcs)?,
        themes: serde_json::from_value(dossier.themes)?,
        motifs: serde_json::from_value(dossier.motifs)?,
        reader_contract: serde_json::from_value(dossier.reader_contract)?,
        pacing_hints: serde_json::from_value(dossier.pacing_hints)?,
    })
}

pub fn resume_snapshot_summary_from_record(
    snapshot: ImportResumeSnapshot,
) -> Result<ImportResumeSnapshotSummary> {
    Ok(ImportResumeSnapshotSummary {
        book_number: snapshot.book_number as i32,
        chapter_number: snapshot.chapter_number as i32,
        scene_order: snapshot.scene_order.map(|value| value as i32),
        summary: snapshot.summary,
        characters: serde_json::from_value(snapshot.characters)?,
        relationships: serde_json::from_value(snapshot.relationships)?,
        locations: serde_json::from_value(snapshot.locations)?,
        plot_threads: serde_json::from_value(snapshot.plot_threads)?,
    })
}

// =============================================================================
// Structural-summary projection from persisted records.
// =============================================================================

pub fn structural_summary_from_records(
    source_documents: &[ImportSourceDocument],
    segments: &[ImportSegment],
    review_item_count: usize,
) -> Result<ImportStructuralAnalysisSummary> {
    let source_documents = source_documents
        .iter()
        .cloned()
        .map(|document| ImportSourceDocumentSummary {
            document_id: document.id.clone(),
            display_name: document.display_name,
            source_path: document.source_path,
            copied_path: document.copied_path,
            source_format: parse_import_source_format(&document.source_format),
            original_sha256: document.original_sha256,
            normalized_sha256: document.normalized_sha256,
            word_count: document.word_count.max(0) as usize,
            chapter_hint: document.chapter_hint,
            source_order: document.source_order.max(0) as usize,
        })
        .collect::<Vec<_>>();

    let mut scenes_by_parent = BTreeMap::<String, Vec<ImportSceneSlice>>::new();
    for segment in segments {
        if segment.segment_type == "scene" {
            let scene = ImportSceneSlice {
                segment_id: segment.id.clone(),
                chapter_segment_id: segment.parent_segment_id.clone(),
                scene_index: segment.scene_order.unwrap_or_default().max(0) as usize,
                label: segment.label.clone(),
                start_offset: segment.start_offset.max(0) as usize,
                end_offset: segment.end_offset.max(0) as usize,
                word_count: segment.word_count.max(0) as usize,
                character_count: segment.character_count.max(0) as usize,
                pov_guess: segment
                    .pov_guess
                    .clone()
                    .map(serde_json::from_value)
                    .transpose()?,
                confidence: segment.confidence,
                confidence_level: import_confidence_level(segment.confidence),
            };
            if let Some(parent_id) = segment.parent_segment_id.clone() {
                scenes_by_parent.entry(parent_id).or_default().push(scene);
            }
        }
    }

    let mut chapters = Vec::new();
    for segment in segments {
        if segment.segment_type != "chapter" {
            continue;
        }
        let chapter_id = segment.id.clone();
        let mut scenes = scenes_by_parent.remove(&chapter_id).unwrap_or_default();
        scenes.sort_by_key(|scene| scene.scene_index);
        chapters.push(ImportChapterSlice {
            segment_id: chapter_id,
            book_number: segment.book_number.unwrap_or(1) as i32,
            chapter_number: segment.chapter_number.unwrap_or(1) as i32,
            title: segment.label.clone(),
            start_offset: segment.start_offset.max(0) as usize,
            end_offset: segment.end_offset.max(0) as usize,
            word_count: segment.word_count.max(0) as usize,
            confidence: segment.confidence,
            confidence_level: import_confidence_level(segment.confidence),
            scenes,
        });
    }

    chapters.sort_by_key(|chapter| (chapter.book_number, chapter.chapter_number));
    Ok(ImportStructuralAnalysisSummary {
        source_documents,
        chapters,
        review_items_created: review_item_count,
    })
}

// =============================================================================
// File-system + slicer helpers used by the service layer.
// =============================================================================

pub fn default_import_data_dir() -> PathBuf {
    std::env::var_os("SPINDLE_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::data_local_dir().map(|path| path.join("spindle")))
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".spindle-data")
        })
}

pub fn default_import_project_name(input: &ImportManuscriptInput) -> String {
    input
        .source_paths
        .first()
        .and_then(|path| {
            PathBuf::from(path)
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("Imported {}", value.replace(['_', '-'], " ")))
        .unwrap_or_else(|| "Imported Manuscript".to_string())
}

pub fn load_segment_text(
    source_document: &ImportSourceDocument,
    segment: &ImportSegment,
) -> Result<String> {
    let normalized_text = std::fs::read_to_string(&source_document.normalized_text_ref)
        .with_context(|| {
            format!(
                "failed to read normalized text {}",
                source_document.normalized_text_ref
            )
        })?;
    let start = segment.start_offset.max(0) as usize;
    let end = segment.end_offset.max(segment.start_offset) as usize;
    normalized_text
        .get(start..end)
        .map(ToString::to_string)
        .context("segment offsets fell outside normalized source text")
}

pub fn scene_text_from_slice(
    source_documents: &[ImportSourceDocument],
    segments: &[ImportSegment],
    scene: &ImportSceneSlice,
) -> Result<String> {
    let segment = segments
        .iter()
        .find(|segment| segment.id == scene.segment_id)
        .context("scene segment not found")?;
    let source_document = source_documents
        .iter()
        .find(|document| document.id == segment.source_document_id)
        .context("source document missing for scene segment")?;
    load_segment_text(source_document, segment)
}

pub fn chapter_text_from_slice(
    source_documents: &[ImportSourceDocument],
    segments: &[ImportSegment],
    chapter: &ImportChapterSlice,
) -> Result<String> {
    let segment = segments
        .iter()
        .find(|segment| segment.id == chapter.segment_id)
        .context("chapter segment not found")?;
    let source_document = source_documents
        .iter()
        .find(|document| document.id == segment.source_document_id)
        .context("source document missing for chapter segment")?;
    load_segment_text(source_document, segment)
}

pub fn find_scene_text_summary(
    source_documents: &[ImportSourceDocument],
    segments: &[ImportSegment],
    scene: &ImportSceneSlice,
) -> Result<String> {
    Ok(summarize_text(
        &scene_text_from_slice(source_documents, segments, scene)?.replace('\n', " "),
        120,
    ))
}

pub fn summarize_text(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        input.to_string()
    } else {
        let summarized: String = input.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{summarized}...")
    }
}

pub fn hydration_mapped_book(map: &BTreeMap<i32, i32>, ms_book: i32) -> i32 {
    map.get(&ms_book).copied().unwrap_or(ms_book)
}

pub fn hydration_mapped_chapter(
    map: &BTreeMap<(i32, i32), i32>,
    ms_book: i32,
    ms_chapter: i32,
) -> i32 {
    map.get(&(ms_book, ms_chapter))
        .copied()
        .unwrap_or(ms_chapter)
}

pub fn imported_character_role(dossier: &ImportCharacterDossierSummary) -> String {
    use spindle_core::models::normalize_name;
    let normalized_patterns = dossier
        .decision_patterns
        .iter()
        .map(|pattern| normalize_name(pattern))
        .collect::<Vec<_>>();
    if normalized_patterns.iter().any(|pattern| {
        pattern.contains("warn") || pattern.contains("decide") || pattern.contains("lead")
    }) {
        "protagonist".to_string()
    } else {
        "supporting".to_string()
    }
}

pub fn summarize_character_summary(dossier: &ImportCharacterDossierSummary) -> String {
    dossier
        .state_trajectory
        .first()
        .map(|point| summarize_text(&point.summary, 140))
        .or_else(|| dossier.decision_patterns.first().cloned())
        .unwrap_or_else(|| format!("Imported character profile for {}.", dossier.canonical_name))
}

// =============================================================================
// Content-rating heuristic (carried over from the SurrealDB-era service).
// =============================================================================

pub fn detect_content_rating(full_text: &str) -> (ContentRating, f64) {
    let evidence = content_rating_evidence(full_text);

    if evidence.explicit_sex_hits > 0 || evidence.explicit_violence_hits >= 2 {
        let confidence = (0.75
            + evidence.explicit_sex_hits as f64 * 0.05
            + evidence.explicit_violence_hits as f64 * 0.04)
            .clamp(0.0, 0.95);
        (ContentRating::Explicit, confidence)
    } else if evidence.explicit_violence_hits == 1 || evidence.mature_hits >= 2 {
        let confidence = (0.65 + evidence.mature_hits as f64 * 0.04).clamp(0.0, 0.90);
        (ContentRating::Mature, confidence)
    } else if evidence.mature_hits == 1 || evidence.teen_hits >= 1 {
        let confidence = (0.60 + evidence.teen_hits as f64 * 0.05).clamp(0.0, 0.85);
        (ContentRating::Teen, confidence)
    } else {
        (ContentRating::General, 0.72)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ContentRatingEvidence {
    explicit_sex_hits: usize,
    explicit_violence_hits: usize,
    mature_hits: usize,
    teen_hits: usize,
}

fn content_rating_evidence(full_text: &str) -> ContentRatingEvidence {
    use spindle_core::models::normalize_name;
    let prose = normalize_name(full_text);

    ContentRatingEvidence {
        explicit_sex_hits: explicit_sex_markers()
            .iter()
            .filter(|marker| prose.contains(**marker))
            .count()
            + count_contextual_thrust_hits(&prose),
        explicit_violence_hits: explicit_violence_markers()
            .iter()
            .filter(|marker| prose.contains(**marker))
            .count(),
        mature_hits: mature_markers()
            .iter()
            .filter(|marker| prose.contains(**marker))
            .count(),
        teen_hits: teen_markers()
            .iter()
            .filter(|marker| prose.contains(**marker))
            .count(),
    }
}

fn normalize_prose_token(token: &str) -> String {
    token
        .trim_matches(|ch: char| !ch.is_ascii_alphabetic() && ch != '\'')
        .to_ascii_lowercase()
}

fn normalized_prose_tokens(prose: &str) -> Vec<String> {
    prose
        .split_whitespace()
        .map(normalize_prose_token)
        .filter(|token| !token.is_empty())
        .collect()
}

fn count_contextual_thrust_hits(prose: &str) -> usize {
    let phrase_hits = contextual_explicit_sex_phrases()
        .iter()
        .filter(|marker| prose.contains(**marker))
        .count();
    let tokens = normalized_prose_tokens(prose);
    let window_hits = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.starts_with("thrust"))
        .filter(|(index, _)| {
            let start = index.saturating_sub(4);
            let end = (index + 5).min(tokens.len());
            tokens[start..end].iter().any(|candidate| {
                explicit_sex_context_markers()
                    .iter()
                    .any(|marker| candidate.starts_with(marker))
            })
        })
        .count();

    phrase_hits.max(window_hits)
}

fn explicit_sex_markers() -> &'static [&'static str] {
    &[
        "cock",
        "clit",
        "cum",
        "orgasm",
        "penetrat",
        "fucked",
        "moaned as",
        "nipple",
        "erection",
    ]
}

fn contextual_explicit_sex_phrases() -> &'static [&'static str] {
    &[
        "thrust into her",
        "thrust into him",
        "thrust into them",
        "thrust inside her",
        "thrust inside him",
        "thrust between her thighs",
        "thrust between his thighs",
        "thrust between their thighs",
    ]
}

fn explicit_sex_context_markers() -> &'static [&'static str] {
    &[
        "cock", "clit", "cum", "orgasm", "penetrat", "fucked", "nipple", "erection", "thigh",
        "thighs", "breast", "breasts", "groin",
    ]
}

fn explicit_violence_markers() -> &'static [&'static str] {
    &[
        "gore",
        "entrails",
        "dismember",
        "viscera",
        "blood sprayed",
        "brain splatter",
        "intestines",
    ]
}

fn mature_markers() -> &'static [&'static str] {
    &[
        "blood", "wound", "corpse", "scream", "naked", "stripped", "torture", "severed",
    ]
}

fn teen_markers() -> &'static [&'static str] {
    &[
        "kiss", "punch", "fight", "blade", "damn", "hell", "drunk", "bruise",
    ]
}
