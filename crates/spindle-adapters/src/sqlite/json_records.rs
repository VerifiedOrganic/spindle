//! JSON-shaped record sub-structs used by the SQLite backend.
//!
//! These types are serialized into TEXT columns (`reader_contract`,
//! `voice_profile`, `chapter_outline.beats`, etc.) and deserialized on read.
//! They are pure data shapes — no SurrealDB types, no `RecordId` — so the
//! SQLite stack can compile standalone after the Phase 6 SurrealDB removal.
//!
//! Each struct provides `from(core)` and `into_core()` (or equivalent
//! conversions) to keep the boundary with `spindle_core::models` typed.

use serde::{Deserialize, Serialize};
use spindle_core::models as core;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredReaderContract {
    pub promise: String,
    #[serde(default)]
    pub style_notes: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<String>,
}

impl StoredReaderContract {
    pub fn into_core(self) -> core::ReaderContract {
        core::ReaderContract {
            promise: self.promise,
            style_notes: self.style_notes,
            boundaries: self.boundaries,
        }
    }
}

impl From<core::ReaderContract> for StoredReaderContract {
    fn from(value: core::ReaderContract) -> Self {
        Self {
            promise: value.promise,
            style_notes: value.style_notes,
            boundaries: value.boundaries,
        }
    }
}

/// Serialized form of [`spindle_core::style::NarratorVoice`], stored in the
/// nullable `project.narrator_voice` TEXT column (see migration V0005).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoredNarratorVoice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comedy_density: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing_feel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interiority_ratio: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emotional_register: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter_ending_style: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl StoredNarratorVoice {
    pub fn into_core(self) -> spindle_core::style::NarratorVoice {
        spindle_core::style::NarratorVoice {
            comedy_density: self.comedy_density,
            pacing_feel: self.pacing_feel,
            interiority_ratio: self.interiority_ratio,
            emotional_register: self.emotional_register,
            chapter_ending_style: self.chapter_ending_style,
            notes: self.notes,
        }
    }
}

impl From<spindle_core::style::NarratorVoice> for StoredNarratorVoice {
    fn from(value: spindle_core::style::NarratorVoice) -> Self {
        Self {
            comedy_density: value.comedy_density,
            pacing_feel: value.pacing_feel,
            interiority_ratio: value.interiority_ratio,
            emotional_register: value.emotional_register,
            chapter_ending_style: value.chapter_ending_style,
            notes: value.notes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredEstablishedIn {
    pub book_number: i32,
    pub chapter_number: i32,
    pub note: Option<String>,
}

impl StoredEstablishedIn {
    pub fn into_core(self) -> core::EstablishedIn {
        core::EstablishedIn {
            book_number: self.book_number,
            chapter_number: self.chapter_number,
            note: self.note,
        }
    }
}

impl From<core::EstablishedIn> for StoredEstablishedIn {
    fn from(value: core::EstablishedIn) -> Self {
        Self {
            book_number: value.book_number,
            chapter_number: value.chapter_number,
            note: value.note,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredFlexRange {
    pub low: Option<String>,
    pub high: Option<String>,
}

impl StoredFlexRange {
    pub fn into_core(self) -> core::FlexRange {
        core::FlexRange {
            low: self.low,
            high: self.high,
        }
    }
}

impl From<core::FlexRange> for StoredFlexRange {
    fn from(value: core::FlexRange) -> Self {
        Self {
            low: value.low,
            high: value.high,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredStoryPlacement {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: Option<i32>,
    pub note: Option<String>,
}

impl StoredStoryPlacement {
    pub fn into_core(self) -> core::StoryPlacement {
        core::StoryPlacement {
            book_number: self.book_number,
            chapter_number: self.chapter_number,
            scene_order: self.scene_order,
            note: self.note,
        }
    }
}

impl From<core::StoryPlacement> for StoredStoryPlacement {
    fn from(value: core::StoryPlacement) -> Self {
        Self {
            book_number: value.book_number,
            chapter_number: value.chapter_number,
            scene_order: value.scene_order,
            note: value.note,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTryFailCycleStep {
    pub attempt_order: i32,
    pub label: String,
    pub outcome: String,
    pub cost: Option<String>,
    pub revelation: Option<String>,
}

impl StoredTryFailCycleStep {
    pub fn into_core(self) -> core::TryFailCycleStep {
        core::TryFailCycleStep {
            attempt_order: self.attempt_order,
            label: self.label,
            outcome: self.outcome,
            cost: self.cost,
            revelation: self.revelation,
        }
    }
}

impl From<core::TryFailCycleStep> for StoredTryFailCycleStep {
    fn from(value: core::TryFailCycleStep) -> Self {
        Self {
            attempt_order: value.attempt_order,
            label: value.label,
            outcome: value.outcome,
            cost: value.cost,
            revelation: value.revelation,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredStatedConsequence {
    pub description: String,
    pub stated_at: Option<StoredStoryPlacement>,
    pub must_demonstrate_by: Option<String>,
    pub delivered: bool,
}

impl StoredStatedConsequence {
    pub fn into_core(self) -> core::StatedConsequence {
        core::StatedConsequence {
            description: self.description,
            stated_at: self.stated_at.map(StoredStoryPlacement::into_core),
            must_demonstrate_by: self.must_demonstrate_by,
            delivered: self.delivered,
        }
    }
}

impl From<core::StatedConsequence> for StoredStatedConsequence {
    fn from(value: core::StatedConsequence) -> Self {
        Self {
            description: value.description,
            stated_at: value.stated_at.map(StoredStoryPlacement::from),
            must_demonstrate_by: value.must_demonstrate_by,
            delivered: value.delivered,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCharacterArcMilestone {
    pub label: String,
    pub placement: Option<StoredStoryPlacement>,
    pub description: String,
    #[serde(default)]
    pub unlocks: Vec<String>,
}

impl StoredCharacterArcMilestone {
    pub fn into_core(self) -> core::CharacterArcMilestone {
        core::CharacterArcMilestone {
            label: self.label,
            placement: self.placement.map(StoredStoryPlacement::into_core),
            description: self.description,
            unlocks: self.unlocks,
        }
    }
}

impl From<core::CharacterArcMilestone> for StoredCharacterArcMilestone {
    fn from(value: core::CharacterArcMilestone) -> Self {
        Self {
            label: value.label,
            placement: value.placement.map(StoredStoryPlacement::from),
            description: value.description,
            unlocks: value.unlocks,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPlannedScene {
    pub scene_order: i32,
    pub summary: String,
    #[serde(default)]
    pub beat_structure: Vec<String>,
    #[serde(default)]
    pub character_ids: Vec<String>,
    pub purpose: String,
}

impl StoredPlannedScene {
    pub fn into_core(self) -> core::PlannedScene {
        core::PlannedScene {
            scene_order: self.scene_order,
            summary: self.summary,
            beat_structure: self.beat_structure,
            character_ids: self.character_ids,
            purpose: self.purpose,
        }
    }
}

impl From<core::PlannedScene> for StoredPlannedScene {
    fn from(value: core::PlannedScene) -> Self {
        Self {
            scene_order: value.scene_order,
            summary: value.summary,
            beat_structure: value.beat_structure,
            character_ids: value.character_ids,
            purpose: value.purpose,
        }
    }
}

/// `scene_id` is `Option<String>` here (SQLite native), unlike the SurrealDB
/// era which used `Option<RecordId>` and a custom deserializer. The on-wire
/// JSON shape is identical: a `"table:id"` string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredChapterOutlineBeat {
    pub order: i32,
    pub summary: String,
    #[serde(default)]
    pub scene_id: Option<String>,
    pub status: String,
}

impl StoredChapterOutlineBeat {
    pub fn into_core(self) -> core::ChapterOutlineBeat {
        core::ChapterOutlineBeat {
            order: self.order,
            summary: self.summary,
            scene_id: self.scene_id,
            status: self.status,
        }
    }
}

impl From<core::ChapterOutlineBeat> for StoredChapterOutlineBeat {
    fn from(value: core::ChapterOutlineBeat) -> Self {
        Self {
            order: value.order,
            summary: value.summary,
            scene_id: value.scene_id,
            status: value.status,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAnnotatedBeat {
    pub beat_type: String,
    pub summary: String,
}

impl StoredAnnotatedBeat {
    pub fn into_core(self) -> core::AnnotatedBeat {
        core::AnnotatedBeat {
            beat_type: self.beat_type,
            summary: self.summary,
        }
    }
}

impl From<core::AnnotatedBeat> for StoredAnnotatedBeat {
    fn from(value: core::AnnotatedBeat) -> Self {
        Self {
            beat_type: value.beat_type,
            summary: value.summary,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredPersonaReviewNotes {
    pub persona: String,
    pub strengths: Vec<String>,
    pub concerns: Vec<String>,
}

impl StoredPersonaReviewNotes {
    pub fn into_core(self) -> core::PersonaReviewNotes {
        core::PersonaReviewNotes {
            persona: self.persona,
            strengths: self.strengths,
            concerns: self.concerns,
        }
    }
}

impl From<core::PersonaReviewNotes> for StoredPersonaReviewNotes {
    fn from(value: core::PersonaReviewNotes) -> Self {
        Self {
            persona: value.persona,
            strengths: value.strengths,
            concerns: value.concerns,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredDualPersonaReviewRound {
    pub round: i64,
    pub literary_critic: StoredPersonaReviewNotes,
    pub craft_technician: StoredPersonaReviewNotes,
    /// Defaulted so reviews persisted before the Target Reader persona existed
    /// still deserialize.
    #[serde(default)]
    pub genre_reader: Option<StoredPersonaReviewNotes>,
    pub priority_actions: Vec<String>,
}

impl StoredDualPersonaReviewRound {
    pub fn into_core(self) -> core::DualPersonaReviewRound {
        core::DualPersonaReviewRound {
            round: self.round as usize,
            literary_critic: self.literary_critic.into_core(),
            craft_technician: self.craft_technician.into_core(),
            genre_reader: self
                .genre_reader
                .map(StoredPersonaReviewNotes::into_core)
                .unwrap_or_default(),
            priority_actions: self.priority_actions,
        }
    }
}

impl From<core::DualPersonaReviewRound> for StoredDualPersonaReviewRound {
    fn from(value: core::DualPersonaReviewRound) -> Self {
        Self {
            round: value.round as i64,
            literary_critic: value.literary_critic.into(),
            craft_technician: value.craft_technician.into(),
            genre_reader: Some(value.genre_reader.into()),
            priority_actions: value.priority_actions,
        }
    }
}
