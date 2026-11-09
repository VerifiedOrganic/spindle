//! Versioned prose-task data. No retrieval, tokenizer implementation or transport.
pub mod chapter;
pub mod critique;
pub mod editorial;
pub mod extraction;
pub mod revision;

pub use chapter::*;
pub use critique::*;
pub use editorial::*;
pub use revision::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditorialPolicy {
    #[default]
    Legacy,
    CritiqueV1,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
pub struct PacketSource {
    pub branch_id: String,
    pub source_id: String,
    pub revision_hash: String,
}

/// A whole record; relevance sorts descending, then recency, then stable source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketItem {
    pub source: PacketSource,
    /// Source-local identity; deduplication also includes the full PacketSource.
    pub proposition_id: String,
    pub text: String,
    pub relevance: u32,
    pub recency: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NarrativePerson {
    First,
    Third,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NarrativeTense {
    Past,
    Present,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketPov {
    pub character_id: String,
    pub display_name: String,
    pub person: NarrativePerson,
    pub tense: NarrativeTense,
    pub source: PacketSource,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ValueShift {
    pub axis: String,
    pub before: String,
    pub after: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EmotionShift {
    pub entry: String,
    pub exit: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketChapter {
    pub book_number: u32,
    pub number: u32,
    pub scene_index: u32,
    pub planned_scene_count: Option<u32>,
    pub is_closer: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TargetWords {
    pub min: u32,
    pub preferred: u32,
    pub max: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketLocation {
    pub id: String,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketParticipant {
    pub id: String,
    pub display_name: String,
    pub role: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SceneCardV1 {
    pub source: PacketSource,
    pub pov: PacketPov,
    pub goal: String,
    pub opposition: String,
    pub turn: String,
    pub intended_outcome: String,
    pub value_shift: ValueShift,
    pub emotion: EmotionShift,
    pub chapter: PacketChapter,
    pub target_words: TargetWords,
    pub location: PacketLocation,
    pub story_clock: String,
    pub rating: crate::models::ContentRating,
    pub tone: Option<String>,
    /// Optional scene classification carried from the scene's explicit tone/tag
    /// label when it names a recognized mode (action, quiet-interior,
    /// dialogue-heavy, reveal, chapter-closer). Kept separate from `value_shift`,
    /// which is the narrative value axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_type: Option<String>,
    pub participants: Vec<PacketParticipant>,
    pub required_beats: Vec<String>,
    pub promises_due: Vec<String>,
    pub prohibited_outcomes: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    Canon,
    Knowledge,
    Time,
    Location,
    AuthorLock,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketConstraint {
    pub id: String,
    pub kind: ConstraintKind,
    pub statement: String,
    pub source: PacketSource,
    pub scope: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketContinuity {
    pub character_states: Vec<PacketItem>,
    pub knowledge_boundaries: Vec<PacketItem>,
    pub facts: Vec<PacketItem>,
    pub prior_scene_tail: Option<PacketItem>,
    pub story_so_far: Vec<PacketItem>,
    pub open_threads: Vec<PacketItem>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovedExemplar {
    pub item: PacketItem,
    pub speaker_id: Option<String>,
    pub approved_revision_hash: String,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketVoice {
    pub rules: Vec<PacketItem>,
    pub exemplars: Vec<ApprovedExemplar>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditorialMemory {
    pub recent_openings: Vec<PacketItem>,
    pub recent_endings: Vec<PacketItem>,
    pub recurring_images: Vec<PacketItem>,
    pub repeated_phrases: Vec<PacketItem>,
    pub pacing_history: Vec<PacketItem>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CraftRubric {
    pub version: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SectionCount {
    pub section: String,
    pub tokens: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OmissionReason {
    Duplicate,
    SectionBudget,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketOmission {
    pub source: PacketSource,
    pub proposition_id: String,
    pub section: u8,
    pub reason: OmissionReason,
    pub tokens: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PacketAssembly {
    pub tokenizer_id: String,
    pub tokenizer_version: String,
    pub model_id: String,
    pub section_counts: Vec<SectionCount>,
    pub omissions: Vec<PacketOmission>,
    pub input_tokens: usize,
    pub output_reserve: usize,
    pub framing_reserve: usize,
    pub context_window: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_seed: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftingPacketV1 {
    pub schema_version: String,
    pub packet_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub run_id: Option<String>,
    pub scene_id: Option<String>,
    pub scene_revision: Option<String>,
    pub canon_snapshot_hash: String,
    pub plan_revision: String,
    pub source_versions: Vec<PacketSource>,
    pub scene_card: SceneCardV1,
    pub hard_constraints: Vec<PacketConstraint>,
    pub continuity: PacketContinuity,
    pub voice: PacketVoice,
    pub editorial_memory: EditorialMemory,
    pub craft_rubric: CraftRubric,
    pub research_evidence: Vec<PacketItem>,
    /// Set by the assembler; caller-supplied counts are never trusted.
    pub assembly: Option<PacketAssembly>,
}

/// Exact system/user messages; serialization order is part of prompt hashing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DraftMessages {
    pub system: String,
    pub user: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreparedDraftingPacket {
    /// Includes the source data and assembly/omission report, not only selected text.
    pub packet: DraftingPacketV1,
    pub messages: DraftMessages,
    pub messages_hash: String,
}
