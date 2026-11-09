//! Chapter-scoped advisory critique, deterministic scene windows, repetition
//! memory, checkpoint-feedback adapters, and the book advisory queue.
//!
//! Pure contracts only. Model dispatch, prompt rendering, and persistence live
//! in `spindle-adapters`; the chapter pass is advisory and never rewrites prose
//! without an explicit author selection that reuses the scene acceptance gate.

use crate::drafting::{
    CriticDimension, CriticFindingInput, CritiqueAnchor, DirectiveSeverity, EditorialMemory,
    FeedbackSourceKind, PacketItem, PacketSource,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CHAPTER_CRITIQUE_SCHEMA_VERSION: &str = "chapter_critique_v1";
pub const CHAPTER_COVERAGE_SCHEMA_VERSION: &str = "chapter_coverage_v1";
pub const BOOK_ADVISORY_QUEUE_SCHEMA_VERSION: &str = "book_advisory_queue_v1";
pub const MAX_CHAPTER_EDIT_PROPOSALS: usize = 2;
pub const DEFAULT_CHAPTER_WINDOW_SCENES: usize = 4;
pub const HARD_MAX_CHAPTER_WINDOW_SCENES: usize = 8;
pub const DEFAULT_RECENT_MEMORY_SCENES: usize = 8;

fn default_chapter_critique_schema_version() -> String {
    CHAPTER_CRITIQUE_SCHEMA_VERSION.into()
}

fn default_coverage_schema_version() -> String {
    CHAPTER_COVERAGE_SCHEMA_VERSION.into()
}

fn default_book_advisory_schema_version() -> String {
    BOOK_ADVISORY_QUEUE_SCHEMA_VERSION.into()
}

fn chapter_sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// One accepted scene considered by a chapter pass or memory build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterSceneRefV1 {
    pub scene_id: String,
    pub scene_order: u32,
    pub text_sha256: String,
    pub word_count: usize,
}

/// Deterministic chapter window. Long chapters are reviewed in whole windows,
/// never silently truncated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterWindowV1 {
    pub index: u32,
    pub scene_ids: Vec<String>,
    pub scene_orders: Vec<u32>,
    pub start_chapter_byte: usize,
    pub end_chapter_byte: usize,
    pub word_count: usize,
}

/// Explicit coverage report for a chapter pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterCoverageV1 {
    #[serde(default = "default_coverage_schema_version")]
    pub schema_version: String,
    pub chapter_number: u32,
    pub window_size: usize,
    pub total_scene_count: usize,
    pub covered_scene_count: usize,
    pub coverage_bps: u32,
    pub fully_covered: bool,
    #[serde(default)]
    pub chapter_index: Vec<ChapterSceneRefV1>,
    #[serde(default)]
    pub windows: Vec<ChapterWindowV1>,
}

impl ChapterCoverageV1 {
    /// All scene ids covered by at least one window, in chapter order.
    pub fn covered_scene_ids(&self) -> Vec<String> {
        let mut covered = BTreeSet::new();
        for window in &self.windows {
            covered.extend(window.scene_ids.iter().cloned());
        }
        self.chapter_index
            .iter()
            .filter(|scene| covered.contains(&scene.scene_id))
            .map(|scene| scene.scene_id.clone())
            .collect()
    }
}

/// Split a chapter into deterministic scene windows and report coverage.
pub fn plan_chapter_windows(
    chapter_number: u32,
    scenes: &[ChapterSceneRefV1],
    window_size: usize,
) -> ChapterCoverageV1 {
    let window_size = window_size.clamp(1, HARD_MAX_CHAPTER_WINDOW_SCENES);
    let mut by_order: BTreeMap<u32, ChapterSceneRefV1> = BTreeMap::new();
    for scene in scenes {
        by_order
            .entry(scene.scene_order)
            .or_insert_with(|| scene.clone());
    }
    let chapter_index: Vec<ChapterSceneRefV1> = by_order.into_values().collect();
    let total = chapter_index.len();

    let mut windows = Vec::new();
    let mut byte_cursor = 0usize;
    for (index, chunk) in chapter_index.chunks(window_size).enumerate() {
        let start = byte_cursor;
        let word_count = chunk.iter().map(|scene| scene.word_count).sum::<usize>();
        // Byte offsets are synthetic chapter-index offsets: each scene occupies
        // its own text length in the concatenated index. The caller can map a
        // scene window back to its exact per-scene UTF-8 spans.
        let bytes = chunk
            .iter()
            .map(|scene| scene.word_count.saturating_mul(6))
            .sum::<usize>();
        byte_cursor = start.saturating_add(bytes);
        windows.push(ChapterWindowV1 {
            index: u32::try_from(index + 1).unwrap_or(u32::MAX),
            scene_ids: chunk.iter().map(|scene| scene.scene_id.clone()).collect(),
            scene_orders: chunk.iter().map(|scene| scene.scene_order).collect(),
            start_chapter_byte: start,
            end_chapter_byte: byte_cursor,
            word_count,
        });
    }
    let covered = windows
        .iter()
        .map(|window| window.scene_ids.len())
        .sum::<usize>();
    let coverage_bps = if total == 0 {
        0
    } else {
        u32::try_from((covered as u64 * 10_000) / total as u64).unwrap_or(10_000)
    };
    ChapterCoverageV1 {
        schema_version: CHAPTER_COVERAGE_SCHEMA_VERSION.into(),
        chapter_number,
        window_size,
        total_scene_count: total,
        covered_scene_count: covered,
        coverage_bps,
        fully_covered: covered == total,
        chapter_index,
        windows,
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChapterConcernDimension {
    OpeningsEndings,
    Repetition,
    BeatProgression,
    SceneLengths,
    ReaderEngagement,
    Continuity,
    ChapterCoherence,
}

impl ChapterConcernDimension {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpeningsEndings => "openings_endings",
            Self::Repetition => "repetition",
            Self::BeatProgression => "beat_progression",
            Self::SceneLengths => "scene_lengths",
            Self::ReaderEngagement => "reader_engagement",
            Self::Continuity => "continuity",
            Self::ChapterCoherence => "chapter_coherence",
        }
    }
}

/// Exact scene/hash/UTF-8 anchor for a chapter concern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterConcernAnchorV1 {
    pub scene_id: String,
    pub text_sha256: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub exact_quote: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterConcernV1 {
    pub id: String,
    pub dimension: ChapterConcernDimension,
    pub severity: DirectiveSeverity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<ChapterConcernAnchorV1>,
    pub diagnosis: String,
    /// `true` when the concern has no verifiable scene/hash anchor and therefore
    /// stays advisory rather than driving an edit.
    #[serde(default)]
    pub advisory: bool,
}

/// A bounded chapter-driven edit proposal for one scene. Applying it requires an
/// explicit author selection; it then flows through the scene check/judge gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterEditProposalV1 {
    pub id: String,
    pub scene_id: String,
    pub base_text_sha256: String,
    pub dimension: ChapterConcernDimension,
    pub anchor: ChapterConcernAnchorV1,
    pub diagnosis: String,
    pub requested_change: String,
    #[serde(default)]
    pub preserve: Vec<String>,
    pub success_test: String,
    #[serde(default)]
    pub source_concern_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterCritiqueV1 {
    #[serde(default = "default_chapter_critique_schema_version")]
    pub schema_version: String,
    pub chapter_number: u32,
    pub chapter_text_hash: String,
    #[serde(default)]
    pub concerns: Vec<ChapterConcernV1>,
    #[serde(default)]
    pub edit_proposals: Vec<ChapterEditProposalV1>,
    #[serde(default)]
    pub strengths_to_preserve: Vec<String>,
    #[serde(default)]
    pub requires_author_decision: Vec<crate::drafting::AuthorDecisionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<ChapterCoverageV1>,
    /// Windows actually reviewed by the model. A partial pass remains explicit.
    #[serde(default)]
    pub windows_reviewed: u32,
    #[serde(default)]
    pub windows_skipped: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_edit_reason: Option<String>,
}

impl ChapterCritiqueV1 {
    /// Generated JSON Schema for the chapter critic prompt.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Self)).expect("ChapterCritiqueV1 schema")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterCritiqueError {
    UnsupportedSchemaVersion(String),
    NoScenes,
    UnknownScene(String),
    StaleHash { scene_id: String },
    InvalidAnchor { scene_id: String, reason: String },
    DuplicateProposal(String),
}

impl std::fmt::Display for ChapterCritiqueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion(version) => {
                write!(f, "unsupported chapter critique schema_version {version:?}")
            }
            Self::NoScenes => write!(f, "chapter critique requires at least one scene"),
            Self::UnknownScene(scene) => write!(f, "unknown scene {scene}"),
            Self::StaleHash { scene_id } => {
                write!(f, "stale text hash for scene {scene_id}")
            }
            Self::InvalidAnchor { scene_id, reason } => {
                write!(f, "invalid anchor for scene {scene_id}: {reason}")
            }
            Self::DuplicateProposal(id) => write!(f, "duplicate chapter edit proposal {id}"),
        }
    }
}

impl std::error::Error for ChapterCritiqueError {}

/// Validate and normalize a chapter critique against exact scene text.
///
/// - Unverifiable or missing concern anchors become `advisory = true`; the
///   concern text is retained but never drives an edit.
/// - Invalid proposal anchors are dropped rather than guessed.
/// - At most `MAX_CHAPTER_EDIT_PROPOSALS` proposals survive, in input order.
pub fn validate_chapter_critique(
    critique: &mut ChapterCritiqueV1,
    scene_texts: &BTreeMap<String, String>,
) -> Result<(), ChapterCritiqueError> {
    if critique.schema_version != CHAPTER_CRITIQUE_SCHEMA_VERSION {
        return Err(ChapterCritiqueError::UnsupportedSchemaVersion(
            critique.schema_version.clone(),
        ));
    }
    if scene_texts.is_empty() {
        return Err(ChapterCritiqueError::NoScenes);
    }
    for concern in &mut critique.concerns {
        if let Some(anchor) = &concern.anchor {
            match validate_chapter_anchor(anchor, scene_texts) {
                Ok(()) => {
                    concern.advisory = false;
                }
                Err(_) => {
                    concern.anchor = None;
                    concern.advisory = true;
                }
            }
        } else {
            concern.advisory = true;
        }
    }
    let mut seen_proposals = BTreeSet::new();
    let mut proposals = Vec::new();
    for proposal in std::mem::take(&mut critique.edit_proposals) {
        if proposals.len() >= MAX_CHAPTER_EDIT_PROPOSALS {
            break;
        }
        if validate_chapter_anchor(&proposal.anchor, scene_texts).is_err() {
            continue;
        }
        if !seen_proposals.insert(proposal.id.clone()) {
            return Err(ChapterCritiqueError::DuplicateProposal(proposal.id));
        }
        proposals.push(proposal);
    }
    critique.edit_proposals = proposals;
    Ok(())
}

/// Validate one chapter anchor against exact scene text.
pub fn validate_chapter_anchor(
    anchor: &ChapterConcernAnchorV1,
    scene_texts: &BTreeMap<String, String>,
) -> Result<(), ChapterCritiqueError> {
    let text = scene_texts
        .get(&anchor.scene_id)
        .ok_or_else(|| ChapterCritiqueError::UnknownScene(anchor.scene_id.clone()))?;
    if chapter_sha256_hex(text.as_bytes()) != anchor.text_sha256 {
        return Err(ChapterCritiqueError::StaleHash {
            scene_id: anchor.scene_id.clone(),
        });
    }
    if anchor.start_byte > anchor.end_byte || anchor.end_byte > text.len() {
        return Err(ChapterCritiqueError::InvalidAnchor {
            scene_id: anchor.scene_id.clone(),
            reason: "byte range out of bounds".into(),
        });
    }
    if !text.is_char_boundary(anchor.start_byte) || !text.is_char_boundary(anchor.end_byte) {
        return Err(ChapterCritiqueError::InvalidAnchor {
            scene_id: anchor.scene_id.clone(),
            reason: "byte range splits UTF-8".into(),
        });
    }
    if text[anchor.start_byte..anchor.end_byte] != anchor.exact_quote {
        return Err(ChapterCritiqueError::InvalidAnchor {
            scene_id: anchor.scene_id.clone(),
            reason: "exact_quote does not match the base text span".into(),
        });
    }
    Ok(())
}

/// Convert a validated chapter proposal into a scene `CritiqueDirective`, keeping
/// the exact anchor so the existing scene gate can apply it.
pub fn chapter_proposal_to_directive(
    proposal: &ChapterEditProposalV1,
) -> crate::drafting::CritiqueDirective {
    crate::drafting::CritiqueDirective {
        id: proposal.id.clone(),
        dimension: CriticDimension::ChapterRole,
        severity: DirectiveSeverity::Major,
        priority: 1,
        anchor: CritiqueAnchor {
            scene_id: Some(proposal.scene_id.clone()),
            base_text_hash: proposal.base_text_sha256.clone(),
            start_byte: proposal.anchor.start_byte,
            end_byte: proposal.anchor.end_byte,
            exact_quote: proposal.anchor.exact_quote.clone(),
            context_before: None,
            context_after: None,
        },
        diagnosis: proposal.diagnosis.clone(),
        requested_change: proposal.requested_change.clone(),
        preserve: proposal.preserve.clone(),
        success_test: proposal.success_test.clone(),
        source_finding_ids: proposal.source_concern_ids.clone(),
        scope: crate::drafting::DirectiveScope::Span,
    }
}

/// Author selection of chapter proposals. Empty/absent selection means no edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterEditSelectionV1 {
    pub chapter_number: u32,
    #[serde(default)]
    pub selected_proposal_ids: Vec<String>,
    #[serde(default)]
    pub author_note: String,
}

impl ChapterEditSelectionV1 {
    pub fn selected_proposals<'a>(
        &self,
        critique: &'a ChapterCritiqueV1,
    ) -> Vec<&'a ChapterEditProposalV1> {
        critique
            .edit_proposals
            .iter()
            .filter(|proposal| self.selected_proposal_ids.contains(&proposal.id))
            .collect()
    }
}

/// Outcome of one chapter advisory critique pass. No prose is mutated here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ChapterCriticOutcomeV1 {
    Skipped {
        reason: String,
    },
    Critiqued {
        critique: Box<ChapterCritiqueV1>,
        critique_artifact_path: String,
        prompt_hash: String,
        #[serde(default)]
        windows_reviewed: u32,
        #[serde(default)]
        windows_skipped: u32,
        #[serde(default)]
        model_call_ids: Vec<String>,
    },
}

/// Per-proposal outcome after the author-selected scene gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterEditResultV1 {
    pub proposal_id: String,
    pub scene_id: String,
    pub accepted: bool,
    pub stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Result of applying an author-selected subset of chapter proposals. The
/// invalidation plan must be honored before the next checkpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterEditApplicationV1 {
    pub chapter_number: u32,
    #[serde(default)]
    pub edits: Vec<ChapterEditResultV1>,
    pub invalidation: ChapterInvalidationPlanV1,
}

/// Plan to invalidate derived artifacts after a chapter edit is accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChapterInvalidationPlanV1 {
    pub chapter_number: u32,
    pub affected_scene_ids: Vec<String>,
    pub invalidate_chapter_summary: bool,
    pub invalidate_scene_extraction: bool,
    pub invalidate_beat_annotations: bool,
    pub invalidate_reader_notes: bool,
    pub reextract_scene_ids: Vec<String>,
    pub revalidate_downstream: bool,
}

pub fn plan_chapter_invalidation(
    chapter_number: u32,
    affected_scene_ids: &[String],
) -> ChapterInvalidationPlanV1 {
    let mut unique = affected_scene_ids.to_vec();
    unique.sort();
    unique.dedup();
    ChapterInvalidationPlanV1 {
        chapter_number,
        affected_scene_ids: unique.clone(),
        invalidate_chapter_summary: true,
        invalidate_scene_extraction: !unique.is_empty(),
        invalidate_beat_annotations: !unique.is_empty(),
        invalidate_reader_notes: true,
        reextract_scene_ids: unique,
        revalidate_downstream: true,
    }
}

// ---------------------------------------------------------------------------
// Repetition / editorial memory
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AcceptedSceneMemoryInputV1 {
    pub scene_id: String,
    pub chapter_number: u32,
    pub scene_order: u32,
    pub text: String,
    pub text_sha256: String,
    #[serde(default = "default_true")]
    pub accepted: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialMemoryBuildInputV1 {
    pub branch_id: String,
    pub current_chapter: u32,
    pub current_scene_order: u32,
    #[serde(default)]
    pub scenes: Vec<AcceptedSceneMemoryInputV1>,
    /// Rejected candidate hashes never contribute memory.
    #[serde(default)]
    pub rejected_text_hashes: Vec<String>,
    #[serde(default = "default_recent_memory_scenes")]
    pub recent_scene_limit: usize,
}

fn default_recent_memory_scenes() -> usize {
    DEFAULT_RECENT_MEMORY_SCENES
}

const MEMORY_STOPWORDS: &[&str] = &[
    "the", "and", "that", "with", "from", "this", "have", "were", "been", "they", "their", "there",
    "would", "could", "should", "into", "about", "after", "before", "over", "under", "then",
    "than", "when", "where", "what", "which", "while", "your", "yours", "she", "her", "his", "him",
    "was", "are", "for", "not", "but", "you", "all", "can", "had", "has", "one", "two", "its",
    "our", "out", "who", "why", "how", "did", "does", "just", "like", "very", "too", "also", "now",
];

fn paragraphs(text: &str) -> Vec<&str> {
    text.split("\n\n")
        .map(str::trim)
        .filter(|paragraph| !paragraph.is_empty())
        .collect()
}

fn memory_item(branch_id: &str, scene: &AcceptedSceneMemoryInputV1, text: String) -> PacketItem {
    PacketItem {
        source: PacketSource {
            branch_id: branch_id.to_string(),
            source_id: format!("scene:{}", scene.scene_id),
            revision_hash: scene.text_sha256.clone(),
        },
        proposition_id: scene.scene_id.clone(),
        text,
        relevance: 1,
        recency: (u64::from(scene.chapter_number) << 32) | u64::from(scene.scene_order),
    }
}

/// Build bounded repetition memory from accepted prose only.
///
/// Recent openings/endings and pacing cover the current chapter's earlier
/// scenes plus the previous `recent_scene_limit` accepted scenes. Rejected
/// candidate hashes are excluded even when their prose is present in `scenes`.
pub fn build_editorial_memory(input: &EditorialMemoryBuildInputV1) -> EditorialMemory {
    let rejected: BTreeSet<&str> = input
        .rejected_text_hashes
        .iter()
        .map(String::as_str)
        .collect();
    let mut selected: Vec<&AcceptedSceneMemoryInputV1> = input
        .scenes
        .iter()
        .filter(|scene| scene.accepted)
        .filter(|scene| !rejected.contains(scene.text_sha256.as_str()))
        .filter(|scene| {
            scene.chapter_number < input.current_chapter
                || (scene.chapter_number == input.current_chapter
                    && scene.scene_order < input.current_scene_order)
        })
        .collect();
    selected.sort_by_key(|scene| (scene.chapter_number, scene.scene_order));

    let current_chapter: Vec<&AcceptedSceneMemoryInputV1> = selected
        .iter()
        .copied()
        .filter(|scene| scene.chapter_number == input.current_chapter)
        .collect();
    let prior: Vec<&AcceptedSceneMemoryInputV1> = selected
        .iter()
        .copied()
        .filter(|scene| scene.chapter_number < input.current_chapter)
        .collect();
    let prior_start = prior.len().saturating_sub(input.recent_scene_limit.max(1));
    let mut memory_scenes = prior[prior_start..].to_vec();
    memory_scenes.extend(current_chapter);
    memory_scenes.sort_by_key(|scene| (scene.chapter_number, scene.scene_order));

    let mut recent_openings = Vec::new();
    let mut recent_endings = Vec::new();
    let mut pacing_history = Vec::new();
    let mut image_scenes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut phrase_scenes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for scene in &memory_scenes {
        let grouped = paragraphs(&scene.text);
        if let Some(first) = grouped.first() {
            recent_openings.push(memory_item(&input.branch_id, scene, (*first).to_string()));
        }
        if let Some(last) = grouped.last() {
            recent_endings.push(memory_item(&input.branch_id, scene, (*last).to_string()));
        }
        let word_count = scene.text.split_whitespace().count();
        pacing_history.push(memory_item(
            &input.branch_id,
            scene,
            format!(
                "chapter {} scene {}: {} words",
                scene.chapter_number, scene.scene_order, word_count
            ),
        ));

        let words: Vec<String> = scene
            .text
            .split(|ch: char| !ch.is_ascii_alphabetic() && ch != '\'')
            .map(|word| word.trim_matches('\'').to_lowercase())
            .filter(|word| word.len() >= 4 && !MEMORY_STOPWORDS.contains(&word.as_str()))
            .collect();
        for word in &words {
            image_scenes
                .entry(word.clone())
                .or_default()
                .insert(scene.scene_id.clone());
        }
        for window in words.windows(3) {
            let phrase = window.join(" ");
            phrase_scenes
                .entry(phrase)
                .or_default()
                .insert(scene.scene_id.clone());
        }
    }

    let mut recurring_images = Vec::new();
    for (word, scenes) in image_scenes {
        if scenes.len() >= 3 {
            let scene = memory_scenes
                .iter()
                .find(|scene| scenes.contains(&scene.scene_id));
            if let Some(scene) = scene {
                recurring_images.push(memory_item(&input.branch_id, scene, word));
            }
        }
    }
    recurring_images.sort_by(|a, b| a.text.cmp(&b.text));

    let mut repeated_phrases = Vec::new();
    for (phrase, scenes) in phrase_scenes {
        if scenes.len() >= 2 {
            let scene = memory_scenes
                .iter()
                .find(|scene| scenes.contains(&scene.scene_id));
            if let Some(scene) = scene {
                repeated_phrases.push(memory_item(&input.branch_id, scene, phrase));
            }
        }
    }
    repeated_phrases.sort_by(|a, b| a.text.cmp(&b.text));

    EditorialMemory {
        recent_openings,
        recent_endings,
        recurring_images,
        repeated_phrases,
        pacing_history,
    }
}

// ---------------------------------------------------------------------------
// Checkpoint feedback adapters
// ---------------------------------------------------------------------------

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointFeedbackSource {
    ReaderSim,
    DualPersona,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CheckpointFeedbackNoteV1 {
    pub source: CheckpointFeedbackSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(default)]
    pub severity: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
}

fn feedback_dimension(note: &CheckpointFeedbackNoteV1) -> CriticDimension {
    let lower = note.message.to_lowercase();
    if lower.contains("confus") || lower.contains("continuity") || lower.contains("contradict") {
        return CriticDimension::Continuity;
    }
    match note.source {
        CheckpointFeedbackSource::ReaderSim => {
            if lower.contains("slow") || lower.contains("pace") || lower.contains("tension") {
                CriticDimension::TensionTurn
            } else {
                CriticDimension::ScenePurpose
            }
        }
        CheckpointFeedbackSource::DualPersona => match note.persona.as_deref() {
            Some("literary_critic") => CriticDimension::Voice,
            Some("genre_reader") => CriticDimension::Freshness,
            _ => CriticDimension::TensionTurn,
        },
    }
}

fn feedback_severity(raw: &str) -> DirectiveSeverity {
    match raw.trim().to_lowercase().as_str() {
        "blocker" | "error" => DirectiveSeverity::Blocker,
        "major" | "warning" | "warn" => DirectiveSeverity::Major,
        "polish" | "info" | "advisory" => DirectiveSeverity::Polish,
        _ => DirectiveSeverity::Minor,
    }
}

/// Map retained reader-simulation and dual-persona notes into the existing
/// `CriticFindingInput` shape. These stay unanchored; the critic may anchor them
/// against exact prose.
pub fn adapt_checkpoint_feedback(notes: &[CheckpointFeedbackNoteV1]) -> Vec<CriticFindingInput> {
    notes
        .iter()
        .enumerate()
        .map(|(index, note)| {
            let source = match note.source {
                CheckpointFeedbackSource::ReaderSim => FeedbackSourceKind::ReaderPersona,
                CheckpointFeedbackSource::DualPersona => FeedbackSourceKind::DualPersona,
            };
            let check_type = match note.source {
                CheckpointFeedbackSource::ReaderSim => "checkpoint_reader_sim",
                CheckpointFeedbackSource::DualPersona => "checkpoint_dual_persona",
            };
            CriticFindingInput {
                finding_id: format!(
                    "checkpoint:{}:{}:{}",
                    check_type,
                    note.scene_id.as_deref().unwrap_or("chapter"),
                    index
                ),
                source,
                check_type: check_type.into(),
                dimension: feedback_dimension(note),
                severity: feedback_severity(&note.severity),
                message: note.message.clone(),
                anchor: None,
                suggested_fix: None,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Book advisory queue
// ---------------------------------------------------------------------------

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BookAdvisoryKind {
    ChapterSummaryGap,
    PacingTrajectory,
    RepeatedDevice,
    UnresolvedPromise,
    ChapterCoherence,
}

impl BookAdvisoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ChapterSummaryGap => "chapter_summary_gap",
            Self::PacingTrajectory => "pacing_trajectory",
            Self::RepeatedDevice => "repeated_device",
            Self::UnresolvedPromise => "unresolved_promise",
            Self::ChapterCoherence => "chapter_coherence",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BookAdvisoryItemV1 {
    pub id: String,
    pub kind: BookAdvisoryKind,
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_number: Option<u32>,
    #[serde(default)]
    pub scene_ids: Vec<String>,
    pub description: String,
    pub source_hash: String,
    /// Always advisory: the queue never rewrites prose or plots by itself.
    #[serde(default = "default_true")]
    pub advisory: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BookAdvisoryChapterV1 {
    pub chapter_number: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub scene_ids: Vec<String>,
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BookAdvisoryPacingPointV1 {
    pub chapter_number: u32,
    pub intensity: f64,
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BookAdvisoryPromiseV1 {
    pub promise_id: String,
    pub description: String,
    pub status: String,
    pub planted_chapter: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_payoff_chapter: Option<u32>,
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BookAdvisoryInputV1 {
    pub project_id: String,
    pub book_number: u32,
    pub source_snapshot_hash: String,
    #[serde(default)]
    pub chapters: Vec<BookAdvisoryChapterV1>,
    #[serde(default)]
    pub pacing: Vec<BookAdvisoryPacingPointV1>,
    #[serde(default)]
    pub promises: Vec<BookAdvisoryPromiseV1>,
    /// `(device, chapter/source labels)` pairs observed across the book.
    #[serde(default)]
    pub repeated_devices: Vec<RepeatedDeviceV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RepeatedDeviceV1 {
    pub device: String,
    pub occurrences: Vec<String>,
    pub source_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BookAdvisoryQueueV1 {
    #[serde(default = "default_book_advisory_schema_version")]
    pub schema_version: String,
    pub project_id: String,
    pub book_number: u32,
    pub source_snapshot_hash: String,
    #[serde(default)]
    pub items: Vec<BookAdvisoryItemV1>,
}

/// Build the advisory book queue. Every item is report-only; nothing here
/// rewrites a chapter or changes a plan.
pub fn build_book_advisory_queue(input: &BookAdvisoryInputV1) -> BookAdvisoryQueueV1 {
    let mut items = Vec::new();
    for chapter in &input.chapters {
        if chapter
            .summary
            .as_deref()
            .is_none_or(|summary| summary.trim().is_empty())
        {
            items.push(BookAdvisoryItemV1 {
                id: format!("book_advisory:summary_gap:{}", chapter.chapter_number),
                kind: BookAdvisoryKind::ChapterSummaryGap,
                severity: "warning".into(),
                chapter_number: Some(chapter.chapter_number),
                scene_ids: chapter.scene_ids.clone(),
                description: format!(
                    "Chapter {} has no usable summary; regenerate before checkpoint review.",
                    chapter.chapter_number
                ),
                source_hash: chapter.source_hash.clone(),
                advisory: true,
            });
        }
    }

    let mut sorted_pacing = input.pacing.clone();
    sorted_pacing.sort_by_key(|point| point.chapter_number);
    for pair in sorted_pacing.windows(2) {
        let drop = pair[0].intensity - pair[1].intensity;
        if drop > 0.25 {
            items.push(BookAdvisoryItemV1 {
                id: format!(
                    "book_advisory:pacing_drop:{}_{}",
                    pair[0].chapter_number, pair[1].chapter_number
                ),
                kind: BookAdvisoryKind::PacingTrajectory,
                severity: "info".into(),
                chapter_number: Some(pair[1].chapter_number),
                scene_ids: Vec::new(),
                description: format!(
                    "Intensity drops from {:.2} at chapter {} to {:.2} at chapter {}.",
                    pair[0].intensity,
                    pair[0].chapter_number,
                    pair[1].intensity,
                    pair[1].chapter_number
                ),
                source_hash: pair[1].source_hash.clone(),
                advisory: true,
            });
        }
    }

    for device in &input.repeated_devices {
        if device.occurrences.len() >= 2 {
            items.push(BookAdvisoryItemV1 {
                id: format!("book_advisory:repeated_device:{}", device.device),
                kind: BookAdvisoryKind::RepeatedDevice,
                severity: "info".into(),
                chapter_number: None,
                scene_ids: device.occurrences.clone(),
                description: format!(
                    "Opening/closing device '{}' repeats across {} chapters; consider varying it.",
                    device.device,
                    device.occurrences.len()
                ),
                source_hash: device.source_hash.clone(),
                advisory: true,
            });
        }
    }

    let last_chapter = input
        .chapters
        .iter()
        .map(|chapter| chapter.chapter_number)
        .max()
        .unwrap_or(0);
    for promise in &input.promises {
        let status = promise.status.to_lowercase();
        let resolved = matches!(status.as_str(), "paid_off" | "resolved" | "abandoned");
        if !resolved
            && promise
                .planned_payoff_chapter
                .is_some_and(|payoff| payoff <= last_chapter)
        {
            items.push(BookAdvisoryItemV1 {
                id: format!("book_advisory:promise:{}", promise.promise_id),
                kind: BookAdvisoryKind::UnresolvedPromise,
                severity: "warning".into(),
                chapter_number: Some(promise.planted_chapter),
                scene_ids: Vec::new(),
                description: format!(
                    "Promise '{}' was planned to pay off by chapter {:?} but remains {}.",
                    promise.description, promise.planned_payoff_chapter, promise.status
                ),
                source_hash: promise.source_hash.clone(),
                advisory: true,
            });
        }
    }

    items.sort_by(|a, b| a.kind.as_str().cmp(b.kind.as_str()).then(a.id.cmp(&b.id)));
    BookAdvisoryQueueV1 {
        schema_version: BOOK_ADVISORY_QUEUE_SCHEMA_VERSION.into(),
        project_id: input.project_id.clone(),
        book_number: input.book_number,
        source_snapshot_hash: input.source_snapshot_hash.clone(),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(id: &str, order: u32, text: &str) -> AcceptedSceneMemoryInputV1 {
        AcceptedSceneMemoryInputV1 {
            scene_id: id.into(),
            chapter_number: 1,
            scene_order: order,
            text: text.into(),
            text_sha256: chapter_sha256_hex(text.as_bytes()),
            accepted: true,
        }
    }

    fn ref_scene(id: &str, order: u32, words: usize) -> ChapterSceneRefV1 {
        ChapterSceneRefV1 {
            scene_id: id.into(),
            scene_order: order,
            text_sha256: format!("hash:{id}"),
            word_count: words,
        }
    }

    #[test]
    fn windows_cover_every_scene_without_truncation() {
        let scenes: Vec<_> = (1..=9)
            .rev()
            .map(|order| ref_scene(&format!("scene:{order}"), order, order as usize * 100))
            .collect();
        let coverage = plan_chapter_windows(3, &scenes, 4);
        assert_eq!(coverage.total_scene_count, 9);
        assert_eq!(coverage.covered_scene_count, 9);
        assert_eq!(coverage.coverage_bps, 10_000);
        assert!(coverage.fully_covered);
        assert_eq!(coverage.windows.len(), 3);
        assert_eq!(coverage.windows[0].scene_orders, vec![1, 2, 3, 4]);
        assert_eq!(coverage.windows[2].scene_orders, vec![9]);
        assert_eq!(coverage.covered_scene_ids().len(), 9);
        // window_size 0 is clamped rather than silently producing no windows.
        let clamped = plan_chapter_windows(3, &scenes, 0);
        assert_eq!(clamped.window_size, 1);
        assert_eq!(clamped.windows.len(), 9);
    }

    fn anchor(scene_id: &str, text: &str, start: usize, end: usize) -> ChapterConcernAnchorV1 {
        ChapterConcernAnchorV1 {
            scene_id: scene_id.into(),
            text_sha256: chapter_sha256_hex(text.as_bytes()),
            start_byte: start,
            end_byte: end,
            exact_quote: text[start..end].to_string(),
        }
    }

    #[test]
    fn chapter_critique_unanchored_concerns_stay_advisory_and_proposals_cap() {
        let text = "Cold water rose.";
        let mut scene_texts = BTreeMap::new();
        scene_texts.insert("scene:1".to_string(), text.to_string());
        let good = anchor("scene:1", text, 0, 4);
        let bad = ChapterConcernAnchorV1 {
            exact_quote: "wrong".into(),
            ..good.clone()
        };
        let proposal = |id: &str, anchor: ChapterConcernAnchorV1| ChapterEditProposalV1 {
            id: id.into(),
            scene_id: "scene:1".into(),
            base_text_sha256: good.text_sha256.clone(),
            dimension: ChapterConcernDimension::OpeningsEndings,
            anchor,
            diagnosis: "Opening needs a sharper physical image.".into(),
            requested_change: "Replace the abstract opening with concrete action.".into(),
            preserve: vec!["POV".into()],
            success_test: "The first sentence names physical action.".into(),
            source_concern_ids: vec!["concern:1".into()],
        };
        let mut critique = ChapterCritiqueV1 {
            schema_version: CHAPTER_CRITIQUE_SCHEMA_VERSION.into(),
            chapter_number: 1,
            chapter_text_hash: "chapter-hash".into(),
            concerns: vec![
                ChapterConcernV1 {
                    id: "concern:1".into(),
                    dimension: ChapterConcernDimension::OpeningsEndings,
                    severity: DirectiveSeverity::Major,
                    anchor: Some(good.clone()),
                    diagnosis: "Anchored concern.".into(),
                    advisory: false,
                },
                ChapterConcernV1 {
                    id: "concern:2".into(),
                    dimension: ChapterConcernDimension::Repetition,
                    severity: DirectiveSeverity::Minor,
                    anchor: Some(bad.clone()),
                    diagnosis: "Bad anchor becomes advisory.".into(),
                    advisory: false,
                },
                ChapterConcernV1 {
                    id: "concern:3".into(),
                    dimension: ChapterConcernDimension::SceneLengths,
                    severity: DirectiveSeverity::Polish,
                    anchor: None,
                    diagnosis: "Unanchored concern stays advisory.".into(),
                    advisory: false,
                },
            ],
            edit_proposals: vec![
                proposal("proposal:1", good.clone()),
                proposal("proposal:2", good.clone()),
                proposal("proposal:3", good.clone()),
                proposal("proposal:bad", bad),
            ],
            strengths_to_preserve: vec!["Cold image".into()],
            requires_author_decision: Vec::new(),
            coverage: None,
            windows_reviewed: 0,
            windows_skipped: 0,
            no_edit_reason: None,
        };
        validate_chapter_critique(&mut critique, &scene_texts).expect("validates");
        assert_eq!(critique.edit_proposals.len(), MAX_CHAPTER_EDIT_PROPOSALS);
        assert!(!critique.concerns[0].advisory);
        assert!(critique.concerns[1].advisory);
        assert!(critique.concerns[1].anchor.is_none());
        assert!(critique.concerns[2].advisory);
        let directive = chapter_proposal_to_directive(&critique.edit_proposals[0]);
        assert_eq!(directive.anchor.start_byte, 0);
        assert_eq!(directive.anchor.end_byte, 4);
        assert_eq!(directive.anchor.exact_quote, "Cold");
        assert_eq!(directive.dimension, CriticDimension::ChapterRole);
    }

    #[test]
    fn invalidation_plan_dedups_and_reextracts() {
        let plan =
            plan_chapter_invalidation(4, &["scene:2".into(), "scene:1".into(), "scene:2".into()]);
        assert_eq!(plan.affected_scene_ids, vec!["scene:1", "scene:2"]);
        assert_eq!(plan.reextract_scene_ids, vec!["scene:1", "scene:2"]);
        assert!(plan.invalidate_chapter_summary);
        assert!(plan.invalidate_beat_annotations);
        assert!(plan.invalidate_reader_notes);
        assert!(plan.revalidate_downstream);
    }

    #[test]
    fn memory_excludes_rejected_candidates_and_limits_recent_scenes() {
        let rejected_text = "Rejected candidate prose with water.";
        let rejected_hash = chapter_sha256_hex(rejected_text.as_bytes());
        let mut scenes = Vec::new();
        for order in 1..=10 {
            scenes.push(AcceptedSceneMemoryInputV1 {
                chapter_number: 1,
                ..scene(
                    &format!("scene:{order}"),
                    order,
                    &format!("Chapter one scene {order} opens on water and closes on stone."),
                )
            });
        }
        for order in 1..=2 {
            scenes.push(AcceptedSceneMemoryInputV1 {
                chapter_number: 2,
                ..scene(
                    &format!("scene:c2:{order}"),
                    order,
                    &format!("Chapter two scene {order} repeats the water image."),
                )
            });
        }
        scenes.push(AcceptedSceneMemoryInputV1 {
            scene_id: "scene:rejected".into(),
            chapter_number: 2,
            scene_order: 3,
            text: rejected_text.into(),
            text_sha256: rejected_hash.clone(),
            accepted: false,
        });
        let input = EditorialMemoryBuildInputV1 {
            branch_id: "branch:1".into(),
            current_chapter: 2,
            current_scene_order: 3,
            scenes,
            rejected_text_hashes: vec![rejected_hash],
            recent_scene_limit: 8,
        };
        let memory = build_editorial_memory(&input);
        // Previous eight chapter-one scenes + the two earlier chapter-two scenes.
        assert_eq!(memory.recent_openings.len(), 10);
        assert_eq!(memory.recent_endings.len(), 10);
        assert_eq!(memory.pacing_history.len(), 10);
        assert!(
            memory
                .recent_openings
                .iter()
                .all(|item| item.source.source_id != "scene:rejected")
        );
        assert!(
            memory
                .repeated_phrases
                .iter()
                .all(|item| !item.text.contains("Rejected"))
        );
        assert!(
            memory
                .recurring_images
                .iter()
                .any(|item| item.text == "water")
        );
        assert!(
            memory
                .repeated_phrases
                .iter()
                .any(|item| item.text == "water closes stone")
        );
    }

    #[test]
    fn feedback_adapter_maps_sources_and_severity() {
        let findings = adapt_checkpoint_feedback(&[
            CheckpointFeedbackNoteV1 {
                source: CheckpointFeedbackSource::ReaderSim,
                scene_id: Some("scene:1".into()),
                persona: None,
                severity: "warning".into(),
                message: "Reader was confused about the timeline.".into(),
                source_hash: Some("hash".into()),
            },
            CheckpointFeedbackNoteV1 {
                source: CheckpointFeedbackSource::DualPersona,
                scene_id: None,
                persona: Some("literary_critic".into()),
                severity: "major".into(),
                message: "The voice slips into generic phrasing.".into(),
                source_hash: None,
            },
        ]);
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].source, FeedbackSourceKind::ReaderPersona);
        assert_eq!(findings[0].dimension, CriticDimension::Continuity);
        assert_eq!(findings[0].severity, DirectiveSeverity::Major);
        assert_eq!(findings[1].source, FeedbackSourceKind::DualPersona);
        assert_eq!(findings[1].dimension, CriticDimension::Voice);
    }

    #[test]
    fn book_advisory_queue_reports_gaps_pacing_repeats_and_promises() {
        let queue = build_book_advisory_queue(&BookAdvisoryInputV1 {
            project_id: "project:1".into(),
            book_number: 1,
            source_snapshot_hash: "snapshot".into(),
            chapters: vec![
                BookAdvisoryChapterV1 {
                    chapter_number: 1,
                    summary: Some("A crossing.".into()),
                    scene_ids: vec!["scene:1".into()],
                    source_hash: "chapter:1".into(),
                },
                BookAdvisoryChapterV1 {
                    chapter_number: 2,
                    summary: Some("  ".into()),
                    scene_ids: vec!["scene:2".into()],
                    source_hash: "chapter:2".into(),
                },
            ],
            pacing: vec![
                BookAdvisoryPacingPointV1 {
                    chapter_number: 1,
                    intensity: 0.9,
                    source_hash: "p1".into(),
                },
                BookAdvisoryPacingPointV1 {
                    chapter_number: 2,
                    intensity: 0.4,
                    source_hash: "p2".into(),
                },
            ],
            promises: vec![BookAdvisoryPromiseV1 {
                promise_id: "promise:1".into(),
                description: "The locked door".into(),
                status: "planted".into(),
                planted_chapter: 1,
                planned_payoff_chapter: Some(2),
                source_hash: "promise-hash".into(),
            }],
            repeated_devices: vec![RepeatedDeviceV1 {
                device: "rain on the window".into(),
                occurrences: vec!["chapter:1".into(), "chapter:2".into()],
                source_hash: "device-hash".into(),
            }],
        });
        assert!(queue.items.iter().all(|item| item.advisory));
        for kind in [
            BookAdvisoryKind::ChapterSummaryGap,
            BookAdvisoryKind::PacingTrajectory,
            BookAdvisoryKind::RepeatedDevice,
            BookAdvisoryKind::UnresolvedPromise,
        ] {
            assert!(
                queue.items.iter().any(|item| item.kind == kind),
                "missing {kind:?}"
            );
        }
    }

    #[test]
    fn clean_book_queue_is_empty() {
        let queue = build_book_advisory_queue(&BookAdvisoryInputV1 {
            project_id: "project:1".into(),
            book_number: 1,
            source_snapshot_hash: "snapshot".into(),
            chapters: vec![BookAdvisoryChapterV1 {
                chapter_number: 1,
                summary: Some("Complete.".into()),
                scene_ids: Vec::new(),
                source_hash: "chapter:1".into(),
            }],
            pacing: vec![BookAdvisoryPacingPointV1 {
                chapter_number: 1,
                intensity: 0.5,
                source_hash: "p1".into(),
            }],
            promises: vec![BookAdvisoryPromiseV1 {
                promise_id: "promise:1".into(),
                description: "Paid".into(),
                status: "paid_off".into(),
                planted_chapter: 1,
                planned_payoff_chapter: Some(1),
                source_hash: "promise-hash".into(),
            }],
            repeated_devices: Vec::new(),
        });
        assert!(queue.items.is_empty());
    }
}
