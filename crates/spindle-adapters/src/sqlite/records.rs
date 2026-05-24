//! Record structs for the SQLite backend.
//!
//! Each record has a public `*_COLUMNS` constant listing the schema columns in
//! a fixed order, and a `TryFrom<&rusqlite::Row<'_>>` impl that reads them in
//! that same order. Repository functions always `SELECT` using the matching
//! `*_COLUMNS` constant, which keeps the column ↔ index mapping consistent and
//! catches drift at the call site.
//!
//! Phase 6: the JSON-only sub-structs are now defined natively in
//! `super::json_records` (no SurrealDB types). The SurrealDB-era `crate::records`
//! is on the deletion list.

use rusqlite::Row;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use super::row::{self, Timestamp};

// Re-export the JSON-only stored sub-structs from the SQLite-native module.
pub use super::json_records::{
    StoredAnnotatedBeat, StoredChapterOutlineBeat, StoredCharacterArcMilestone,
    StoredDualPersonaReviewRound, StoredEstablishedIn, StoredFlexRange, StoredNarratorVoice,
    StoredPersonaReviewNotes, StoredPlannedScene, StoredReaderContract, StoredStatedConsequence,
    StoredStoryPlacement, StoredTryFailCycleStep,
};

// =============================================================================
// Project
// =============================================================================

pub const PROJECT_COLUMNS: &str = "id, name, project_type, genre, reader_contract, active_branch_id, notes, created_at, updated_at, narrator_voice";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub project_type: String,
    pub genre: String,
    pub reader_contract: StoredReaderContract,
    pub active_branch_id: Option<String>,
    pub notes: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Prose-level narration directive (migration V0005). `None` when unset.
    #[serde(default)]
    pub narrator_voice: Option<StoredNarratorVoice>,
}

impl<'a> TryFrom<&Row<'a>> for Project {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            name: row::text(r, 1)?,
            project_type: row::text(r, 2)?,
            genre: row::text(r, 3)?,
            reader_contract: row::json(r, 4)?,
            active_branch_id: row::opt_text(r, 5)?,
            notes: row::opt_text(r, 6)?,
            created_at: row::time(r, 7)?,
            updated_at: row::time(r, 8)?,
            narrator_voice: row::opt_json(r, 9)?,
        })
    }
}

// =============================================================================
// BibleBranch
// =============================================================================

pub const BIBLE_BRANCH_COLUMNS: &str = "id, project_id, parent_branch_id, name, status, branch_type, description, \
     created_from_save_point_id, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BibleBranch {
    pub id: String,
    pub project_id: Option<String>,
    pub parent_branch_id: Option<String>,
    pub name: String,
    pub status: String,
    pub branch_type: Option<String>,
    pub description: Option<String>,
    pub created_from_save_point_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Option<Timestamp>,
}

impl<'a> TryFrom<&Row<'a>> for BibleBranch {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::opt_text(r, 1)?,
            parent_branch_id: row::opt_text(r, 2)?,
            name: row::text(r, 3)?,
            status: row::text(r, 4)?,
            branch_type: row::opt_text(r, 5)?,
            description: row::opt_text(r, 6)?,
            created_from_save_point_id: row::opt_text(r, 7)?,
            created_at: row::time(r, 8)?,
            updated_at: row::opt_time(r, 9)?,
        })
    }
}

// =============================================================================
// Book
// =============================================================================

pub const BOOK_COLUMNS: &str = "id, project_id, book_number, title, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub project_id: String,
    pub book_number: i32,
    pub title: Option<String>,
    pub created_at: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamp>,
}

impl<'a> TryFrom<&Row<'a>> for Book {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            book_number: row::int(r, 2)? as i32,
            title: row::opt_text(r, 3)?,
            created_at: row::time(r, 4)?,
            updated_at: row::opt_time(r, 5)?,
        })
    }
}

// =============================================================================
// Chapter
// =============================================================================

pub const CHAPTER_COLUMNS: &str =
    "id, project_id, book_id, book_number, chapter_number, title, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: String,
    pub project_id: String,
    pub book_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub title: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Option<Timestamp>,
}

impl<'a> TryFrom<&Row<'a>> for Chapter {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            book_id: row::text(r, 2)?,
            book_number: row::int(r, 3)? as i32,
            chapter_number: row::int(r, 4)? as i32,
            title: row::opt_text(r, 5)?,
            created_at: row::time(r, 6)?,
            updated_at: row::opt_time(r, 7)?,
        })
    }
}

// =============================================================================
// Scene
// =============================================================================

pub const SCENE_COLUMNS: &str = "id, project_id, branch_id, book_id, chapter_id, book_number, chapter_number, \
     scene_order, full_text, summary, content_rating, tone, draft_origin, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub book_id: String,
    pub chapter_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub full_text: String,
    pub summary: String,
    pub content_rating: String,
    pub tone: Option<String>,
    pub draft_origin: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Scene {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            book_id: row::text(r, 3)?,
            chapter_id: row::text(r, 4)?,
            book_number: row::int(r, 5)? as i32,
            chapter_number: row::int(r, 6)? as i32,
            scene_order: row::int(r, 7)? as i32,
            full_text: row::text(r, 8)?,
            summary: row::text(r, 9)?,
            content_rating: row::text(r, 10)?,
            tone: row::opt_text(r, 11)?,
            draft_origin: row::opt_text(r, 12)?,
            created_at: row::time(r, 13)?,
            updated_at: row::time(r, 14)?,
        })
    }
}

// =============================================================================
// Character
// =============================================================================

pub const CHARACTER_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, summary, role, realm, \
     appearance, notes, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub summary: String,
    pub role: String,
    pub realm: Option<String>,
    pub appearance: Option<String>,
    pub notes: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Character {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            summary: row::text(r, 5)?,
            role: row::text(r, 6)?,
            realm: row::opt_text(r, 7)?,
            appearance: row::opt_text(r, 8)?,
            notes: row::opt_text(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

// =============================================================================
// CharacterVoiceProfile
// =============================================================================

pub const CHARACTER_VOICE_PROFILE_COLUMNS: &str = "id, character_id, vocabulary, sentence_structure, tics, forbidden_words, \
     example_lines, tone, established_in_scene_id, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterVoiceProfile {
    pub id: String,
    pub character_id: String,
    pub vocabulary: Vec<String>,
    pub sentence_structure: Vec<String>,
    pub tics: Vec<String>,
    pub forbidden_words: Vec<String>,
    pub example_lines: Vec<String>,
    pub tone: Option<String>,
    pub established_in_scene_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Option<Timestamp>,
}

impl<'a> TryFrom<&Row<'a>> for CharacterVoiceProfile {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            character_id: row::text(r, 1)?,
            vocabulary: row::json(r, 2)?,
            sentence_structure: row::json(r, 3)?,
            tics: row::json(r, 4)?,
            forbidden_words: row::json(r, 5)?,
            example_lines: row::json(r, 6)?,
            tone: row::opt_text(r, 7)?,
            established_in_scene_id: row::opt_text(r, 8)?,
            created_at: row::time(r, 9)?,
            updated_at: row::opt_time(r, 10)?,
        })
    }
}

// =============================================================================
// CharacterEmotionalProfile
// =============================================================================

pub const CHARACTER_EMOTIONAL_PROFILE_COLUMNS: &str = "id, character_id, base_emotions, suppressed, triggers, defense_mechanisms, \
     flex_range, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterEmotionalProfile {
    pub id: String,
    pub character_id: String,
    pub base_emotions: BTreeMap<String, Value>,
    pub suppressed: Vec<String>,
    pub triggers: Vec<String>,
    pub defense_mechanisms: Vec<String>,
    pub flex_range: Option<StoredFlexRange>,
    pub created_at: Timestamp,
    pub updated_at: Option<Timestamp>,
}

impl<'a> TryFrom<&Row<'a>> for CharacterEmotionalProfile {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            character_id: row::text(r, 1)?,
            base_emotions: row::json(r, 2)?,
            suppressed: row::json(r, 3)?,
            triggers: row::json(r, 4)?,
            defense_mechanisms: row::json(r, 5)?,
            flex_range: row::opt_json(r, 6)?,
            created_at: row::time(r, 7)?,
            updated_at: row::opt_time(r, 8)?,
        })
    }
}

// =============================================================================
// CharacterState
// =============================================================================

pub const CHARACTER_STATE_COLUMNS: &str = "id, project_id, branch_id, character_id, scene_id, book_number, chapter_number, \
     scene_order, emotional_state, goals, status, notes, source_summary, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterState {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub scene_id: Option<String>,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub emotional_state: BTreeMap<String, Value>,
    pub goals: Vec<String>,
    pub status: Vec<String>,
    pub notes: Vec<String>,
    pub source_summary: Option<String>,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for CharacterState {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            character_id: row::text(r, 3)?,
            scene_id: row::opt_text(r, 4)?,
            book_number: row::int(r, 5)? as i32,
            chapter_number: row::int(r, 6)? as i32,
            scene_order: row::int(r, 7)? as i32,
            emotional_state: row::json(r, 8)?,
            goals: row::json(r, 9)?,
            status: row::json(r, 10)?,
            notes: row::json(r, 11)?,
            source_summary: row::opt_text(r, 12)?,
            created_at: row::time(r, 13)?,
        })
    }
}

// =============================================================================
// CharacterArc
// =============================================================================

pub const CHARACTER_ARC_COLUMNS: &str = "id, project_id, branch_id, character_id, arc_type, starting_state, ending_state, \
     milestones, thematic_purpose, connected_theme_ids, status, progress, notes, \
     archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterArc {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub arc_type: String,
    pub starting_state: String,
    pub ending_state: String,
    pub milestones: Vec<StoredCharacterArcMilestone>,
    pub thematic_purpose: String,
    pub connected_theme_ids: Vec<String>,
    pub status: String,
    pub progress: f64,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for CharacterArc {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            character_id: row::text(r, 3)?,
            arc_type: row::text(r, 4)?,
            starting_state: row::text(r, 5)?,
            ending_state: row::text(r, 6)?,
            milestones: row::json(r, 7)?,
            thematic_purpose: row::text(r, 8)?,
            connected_theme_ids: row::json(r, 9)?,
            status: row::text(r, 10)?,
            progress: row::real(r, 11)?,
            notes: row::opt_text(r, 12)?,
            archived_at: row::opt_time(r, 13)?,
            created_at: row::time(r, 14)?,
            updated_at: row::time(r, 15)?,
        })
    }
}

// =============================================================================
// Location
// =============================================================================

pub const LOCATION_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, kind, realm, summary, notes, \
     created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub kind: String,
    pub realm: Option<String>,
    pub summary: String,
    pub notes: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Location {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            kind: row::text(r, 5)?,
            realm: row::opt_text(r, 6)?,
            summary: row::text(r, 7)?,
            notes: row::opt_text(r, 8)?,
            created_at: row::time(r, 9)?,
            updated_at: row::time(r, 10)?,
        })
    }
}

// =============================================================================
// WorldState
// =============================================================================

pub const WORLD_STATE_COLUMNS: &str = "id, project_id, branch_id, location_id, controlling_faction, status, prosperity, \
     stability, threat_level, sensory_details, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldState {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub location_id: String,
    pub controlling_faction: Option<String>,
    pub status: Option<String>,
    pub prosperity: Option<String>,
    pub stability: Option<String>,
    pub threat_level: Option<String>,
    pub sensory_details: Vec<String>,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for WorldState {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            location_id: row::text(r, 3)?,
            controlling_faction: row::opt_text(r, 4)?,
            status: row::opt_text(r, 5)?,
            prosperity: row::opt_text(r, 6)?,
            stability: row::opt_text(r, 7)?,
            threat_level: row::opt_text(r, 8)?,
            sensory_details: row::json(r, 9)?,
            updated_at: row::time(r, 10)?,
        })
    }
}

// =============================================================================
// WorldRule
// =============================================================================

pub const WORLD_RULE_COLUMNS: &str = "id, project_id, branch_id, rule_name, rule_type, description, established_in, \
     relevance_tags, scan_pattern, notes, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldRule {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub rule_name: String,
    pub rule_type: String,
    pub description: String,
    pub established_in: Option<StoredEstablishedIn>,
    pub relevance_tags: Option<Vec<String>>,
    pub scan_pattern: Option<String>,
    pub notes: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Option<Timestamp>,
}

impl WorldRule {
    pub fn relevance_tags_or_empty(&self) -> &[String] {
        self.relevance_tags.as_deref().unwrap_or(&[])
    }
}

impl<'a> TryFrom<&Row<'a>> for WorldRule {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            rule_name: row::text(r, 3)?,
            rule_type: row::text(r, 4)?,
            description: row::text(r, 5)?,
            established_in: row::opt_json(r, 6)?,
            relevance_tags: row::opt_json(r, 7)?,
            scan_pattern: row::opt_text(r, 8)?,
            notes: row::opt_text(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::opt_time(r, 11)?,
        })
    }
}

// =============================================================================
// Faction
// =============================================================================

pub const FACTION_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, faction_type, realm, summary, \
     tags, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Faction {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub faction_type: String,
    pub realm: Option<String>,
    pub summary: String,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Faction {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            faction_type: row::text(r, 5)?,
            realm: row::opt_text(r, 6)?,
            summary: row::text(r, 7)?,
            tags: row::json(r, 8)?,
            notes: row::opt_text(r, 9)?,
            archived_at: row::opt_time(r, 10)?,
            created_at: row::time(r, 11)?,
            updated_at: row::time(r, 12)?,
        })
    }
}

// =============================================================================
// Religion
// =============================================================================

pub const RELIGION_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, deity_or_principle, summary, \
     tags, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Religion {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub deity_or_principle: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Religion {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            deity_or_principle: row::text(r, 5)?,
            summary: row::text(r, 6)?,
            tags: row::json(r, 7)?,
            notes: row::opt_text(r, 8)?,
            archived_at: row::opt_time(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

// =============================================================================
// Economy
// =============================================================================

pub const ECONOMY_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, realm, summary, scarce_resources, \
     trade_goods, currency, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Economy {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub realm: Option<String>,
    pub summary: String,
    pub scarce_resources: Vec<String>,
    pub trade_goods: Vec<String>,
    pub currency: Option<String>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Economy {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            realm: row::opt_text(r, 5)?,
            summary: row::text(r, 6)?,
            scarce_resources: row::json(r, 7)?,
            trade_goods: row::json(r, 8)?,
            currency: row::opt_text(r, 9)?,
            notes: row::opt_text(r, 10)?,
            archived_at: row::opt_time(r, 11)?,
            created_at: row::time(r, 12)?,
            updated_at: row::time(r, 13)?,
        })
    }
}

// =============================================================================
// Term
// =============================================================================

pub const TERM_COLUMNS: &str = "id, project_id, branch_id, term_text, normalized_term, pronunciation, definition, \
     usage_context, origin, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Term {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub term_text: String,
    pub normalized_term: String,
    pub pronunciation: Option<String>,
    pub definition: String,
    pub usage_context: Option<String>,
    pub origin: Option<String>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Term {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            term_text: row::text(r, 3)?,
            normalized_term: row::text(r, 4)?,
            pronunciation: row::opt_text(r, 5)?,
            definition: row::text(r, 6)?,
            usage_context: row::opt_text(r, 7)?,
            origin: row::opt_text(r, 8)?,
            notes: row::opt_text(r, 9)?,
            archived_at: row::opt_time(r, 10)?,
            created_at: row::time(r, 11)?,
            updated_at: row::time(r, 12)?,
        })
    }
}

// =============================================================================
// PlotLine
// =============================================================================

pub const PLOT_LINE_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, plot_type, summary, status, \
     convergence_points, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlotLine {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub plot_type: String,
    pub summary: String,
    pub status: String,
    pub convergence_points: Vec<StoredStoryPlacement>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for PlotLine {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            plot_type: row::text(r, 5)?,
            summary: row::text(r, 6)?,
            status: row::text(r, 7)?,
            convergence_points: row::json(r, 8)?,
            notes: row::opt_text(r, 9)?,
            archived_at: row::opt_time(r, 10)?,
            created_at: row::time(r, 11)?,
            updated_at: row::time(r, 12)?,
        })
    }
}

// =============================================================================
// Conflict
// =============================================================================

pub const CONFLICT_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, conflict_type, stakes, \
     escalation_stages, expected_total_cycles, try_fail_cycles, stated_consequences, \
     resolution_summary, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conflict {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub conflict_type: String,
    pub stakes: String,
    pub escalation_stages: Vec<String>,
    pub expected_total_cycles: Option<i32>,
    pub try_fail_cycles: Vec<StoredTryFailCycleStep>,
    pub stated_consequences: Vec<StoredStatedConsequence>,
    pub resolution_summary: Option<String>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Conflict {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            conflict_type: row::text(r, 5)?,
            stakes: row::text(r, 6)?,
            escalation_stages: row::json(r, 7)?,
            expected_total_cycles: row::opt_int(r, 8)?.map(|v| v as i32),
            try_fail_cycles: row::json(r, 9)?,
            stated_consequences: row::json(r, 10)?,
            resolution_summary: row::opt_text(r, 11)?,
            notes: row::opt_text(r, 12)?,
            archived_at: row::opt_time(r, 13)?,
            created_at: row::time(r, 14)?,
            updated_at: row::time(r, 15)?,
        })
    }
}

// =============================================================================
// Theme
// =============================================================================

pub const THEME_COLUMNS: &str = "id, project_id, branch_id, theme_statement, thesis_antithesis, introduction_point, \
     resolution_point, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub theme_statement: String,
    pub thesis_antithesis: String,
    pub introduction_point: Option<StoredStoryPlacement>,
    pub resolution_point: Option<StoredStoryPlacement>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Theme {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            theme_statement: row::text(r, 3)?,
            thesis_antithesis: row::text(r, 4)?,
            introduction_point: row::opt_json(r, 5)?,
            resolution_point: row::opt_json(r, 6)?,
            notes: row::opt_text(r, 7)?,
            archived_at: row::opt_time(r, 8)?,
            created_at: row::time(r, 9)?,
            updated_at: row::time(r, 10)?,
        })
    }
}

// =============================================================================
// Motif
// =============================================================================

pub const MOTIF_COLUMNS: &str = "id, project_id, branch_id, name, normalized_name, description, max_uses_per_chapter, \
     connected_theme_ids, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Motif {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub normalized_name: String,
    pub description: String,
    pub max_uses_per_chapter: Option<i32>,
    pub connected_theme_ids: Vec<String>,
    pub notes: Option<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Motif {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            description: row::text(r, 5)?,
            max_uses_per_chapter: row::opt_int(r, 6)?.map(|v| v as i32),
            connected_theme_ids: row::json(r, 7)?,
            notes: row::opt_text(r, 8)?,
            archived_at: row::opt_time(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

// =============================================================================
// NarrativePromise
// =============================================================================

pub const NARRATIVE_PROMISE_COLUMNS: &str = "id, project_id, branch_id, promise_type, description, status, planted_at, \
     planned_payoff, notes, archived_at, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativePromise {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub promise_type: String,
    pub description: String,
    pub status: String,
    pub planted_at: StoredStoryPlacement,
    pub planned_payoff: Option<StoredStoryPlacement>,
    pub notes: Vec<String>,
    pub archived_at: Option<Timestamp>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for NarrativePromise {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            promise_type: row::text(r, 3)?,
            description: row::text(r, 4)?,
            status: row::text(r, 5)?,
            planted_at: row::json(r, 6)?,
            planned_payoff: row::opt_json(r, 7)?,
            notes: row::json(r, 8)?,
            archived_at: row::opt_time(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

// =============================================================================
// SavePoint
// =============================================================================

pub const SAVE_POINT_COLUMNS: &str = "id, project_id, branch_id, name, description, snapshot_file_path, snapshot_format, \
     snapshot_record_count, snapshot_created_at, snapshot_sha256, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavePoint {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub name: String,
    pub description: Option<String>,
    pub snapshot_file_path: Option<String>,
    pub snapshot_format: Option<String>,
    pub snapshot_record_count: Option<i64>,
    pub snapshot_created_at: Option<Timestamp>,
    pub snapshot_sha256: Option<String>,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for SavePoint {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            name: row::text(r, 3)?,
            description: row::opt_text(r, 4)?,
            snapshot_file_path: row::opt_text(r, 5)?,
            snapshot_format: row::opt_text(r, 6)?,
            snapshot_record_count: row::opt_int(r, 7)?,
            snapshot_created_at: row::opt_time(r, 8)?,
            snapshot_sha256: row::opt_text(r, 9)?,
            created_at: row::time(r, 10)?,
        })
    }
}

// =============================================================================
// RelatesTo (junction table; composite PK = (branch_id, in_id, out_id))
// =============================================================================

pub const RELATES_TO_COLUMNS: &str = "in_id, out_id, branch_id, relationship_type, trust, tension, dynamics, reason, \
     last_scene_id, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatesTo {
    pub in_id: String,
    pub out_id: String,
    pub branch_id: String,
    pub relationship_type: String,
    pub trust: i32,
    pub tension: i32,
    pub dynamics: Vec<String>,
    pub reason: Option<String>,
    pub last_scene_id: Option<String>,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for RelatesTo {
    type Error = rusqlite::Error;

    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            in_id: row::text(r, 0)?,
            out_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            relationship_type: row::text(r, 3)?,
            trust: row::int(r, 4)? as i32,
            tension: row::int(r, 5)? as i32,
            dynamics: row::json(r, 6)?,
            reason: row::opt_text(r, 7)?,
            last_scene_id: row::opt_text(r, 8)?,
            updated_at: row::time(r, 9)?,
        })
    }
}

// =============================================================================
// PacingConfig / PacingCurve / PacingTracker
// =============================================================================

pub const PACING_CONFIG_COLUMNS: &str = "id, project_id, branch_id, total_planned_books, avg_chapters_per_book, \
     avg_scenes_per_chapter, tension_model, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacingConfig {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub total_planned_books: i32,
    pub avg_chapters_per_book: i32,
    pub avg_scenes_per_chapter: i32,
    pub tension_model: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for PacingConfig {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            total_planned_books: row::int(r, 3)? as i32,
            avg_chapters_per_book: row::int(r, 4)? as i32,
            avg_scenes_per_chapter: row::int(r, 5)? as i32,
            tension_model: row::text(r, 6)?,
            created_at: row::time(r, 7)?,
            updated_at: row::time(r, 8)?,
        })
    }
}

pub const PACING_CURVE_COLUMNS: &str = "id, project_id, branch_id, book_number, act_breakpoints, scene_type_density, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacingCurve {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub book_number: i32,
    pub act_breakpoints: BTreeMap<String, f64>,
    pub scene_type_density: BTreeMap<String, f64>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for PacingCurve {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            book_number: row::int(r, 3)? as i32,
            act_breakpoints: row::json(r, 4)?,
            scene_type_density: row::json(r, 5)?,
            created_at: row::time(r, 6)?,
            updated_at: row::time(r, 7)?,
        })
    }
}

pub const PACING_TRACKER_COLUMNS: &str = "id, project_id, branch_id, character_arc_id, per_book_budget, max_progress_per_chapter, \
     milestone_spacing, sprint_allowance, regression_budget, current_progress, budget_remaining, \
     velocity, status, next_milestone, warnings, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacingTracker {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub character_arc_id: String,
    pub per_book_budget: BTreeMap<String, f64>,
    pub max_progress_per_chapter: Option<f64>,
    pub milestone_spacing: Option<i32>,
    pub sprint_allowance: Option<i32>,
    pub regression_budget: Option<f64>,
    pub current_progress: f64,
    pub budget_remaining: f64,
    pub velocity: String,
    pub status: String,
    pub next_milestone: Option<String>,
    pub warnings: Vec<String>,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for PacingTracker {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            character_arc_id: row::text(r, 3)?,
            per_book_budget: row::json(r, 4)?,
            max_progress_per_chapter: row::opt_real(r, 5)?,
            milestone_spacing: row::opt_int(r, 6)?.map(|v| v as i32),
            sprint_allowance: row::opt_int(r, 7)?.map(|v| v as i32),
            regression_budget: row::opt_real(r, 8)?,
            current_progress: row::real(r, 9)?,
            budget_remaining: row::real(r, 10)?,
            velocity: row::text(r, 11)?,
            status: row::text(r, 12)?,
            next_milestone: row::opt_text(r, 13)?,
            warnings: row::json(r, 14)?,
            updated_at: row::time(r, 15)?,
        })
    }
}

// =============================================================================
// ChapterPlan / ChapterSummary
// =============================================================================

pub const CHAPTER_PLAN_COLUMNS: &str = "id, project_id, branch_id, book_number, chapter_number, pov_character_id, synopsis, \
     target_theme_ids, target_conflict_ids, target_plot_line_ids, scenes, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterPlan {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub pov_character_id: Option<String>,
    pub synopsis: String,
    pub target_theme_ids: Vec<String>,
    pub target_conflict_ids: Vec<String>,
    pub target_plot_line_ids: Vec<String>,
    pub scenes: Vec<StoredPlannedScene>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ChapterPlan {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            book_number: row::int(r, 3)? as i32,
            chapter_number: row::int(r, 4)? as i32,
            pov_character_id: row::opt_text(r, 5)?,
            synopsis: row::text(r, 6)?,
            target_theme_ids: row::json(r, 7)?,
            target_conflict_ids: row::json(r, 8)?,
            target_plot_line_ids: row::json(r, 9)?,
            scenes: row::json(r, 10)?,
            created_at: row::time(r, 11)?,
            updated_at: row::time(r, 12)?,
        })
    }
}

pub const CHAPTER_SUMMARY_COLUMNS: &str = "id, project_id, branch_id, book_number, chapter_number, summary, key_events, \
     character_changes, relationship_shifts, arc_advances, promise_events, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterSummary {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub book_number: i32,
    pub chapter_number: i32,
    pub summary: String,
    pub key_events: Vec<String>,
    pub character_changes: Vec<String>,
    pub relationship_shifts: Vec<String>,
    pub arc_advances: Vec<String>,
    pub promise_events: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ChapterSummary {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            book_number: row::int(r, 3)? as i32,
            chapter_number: row::int(r, 4)? as i32,
            summary: row::text(r, 5)?,
            key_events: row::json(r, 6)?,
            character_changes: row::json(r, 7)?,
            relationship_shifts: row::json(r, 8)?,
            arc_advances: row::json(r, 9)?,
            promise_events: row::json(r, 10)?,
            created_at: row::time(r, 11)?,
            updated_at: row::time(r, 12)?,
        })
    }
}

// =============================================================================
// BookOutline / ChapterOutline / SceneBeatAnnotation
// =============================================================================

pub const BOOK_OUTLINE_COLUMNS: &str = "id, book_id, branch_id, format, content, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookOutline {
    pub id: String,
    pub book_id: String,
    pub branch_id: String,
    pub format: String,
    pub content: String,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for BookOutline {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            book_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            format: row::text(r, 3)?,
            content: row::text(r, 4)?,
            updated_at: row::time(r, 5)?,
        })
    }
}

pub const CHAPTER_OUTLINE_COLUMNS: &str =
    "id, chapter_id, branch_id, format, content, beats, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterOutline {
    pub id: String,
    pub chapter_id: String,
    pub branch_id: String,
    pub format: String,
    pub content: String,
    pub beats: Vec<StoredChapterOutlineBeat>,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ChapterOutline {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            chapter_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            format: row::text(r, 3)?,
            content: row::text(r, 4)?,
            beats: row::json(r, 5)?,
            updated_at: row::time(r, 6)?,
        })
    }
}

pub const SCENE_BEAT_ANNOTATION_COLUMNS: &str = "id, project_id, branch_id, scene_id, beats, motif_ids, theme_ids, conflict_ids, \
     created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneBeatAnnotation {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub beats: Vec<StoredAnnotatedBeat>,
    pub motif_ids: Vec<String>,
    pub theme_ids: Vec<String>,
    pub conflict_ids: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for SceneBeatAnnotation {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            scene_id: row::text(r, 3)?,
            beats: row::json(r, 4)?,
            motif_ids: row::json(r, 5)?,
            theme_ids: row::json(r, 6)?,
            conflict_ids: row::json(r, 7)?,
            created_at: row::time(r, 8)?,
            updated_at: row::time(r, 9)?,
        })
    }
}

// =============================================================================
// TimelineEvent / TemporalIntervention / SystemOverlay / ProgressionEvent
// =============================================================================

pub const TIMELINE_EVENT_COLUMNS: &str = "id, project_id, branch_id, title, event_type, placement, summary, related_entity_ids, \
     created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub title: String,
    pub event_type: String,
    pub placement: StoredStoryPlacement,
    pub summary: String,
    pub related_entity_ids: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for TimelineEvent {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            title: row::text(r, 3)?,
            event_type: row::text(r, 4)?,
            placement: row::json(r, 5)?,
            summary: row::text(r, 6)?,
            related_entity_ids: row::json(r, 7)?,
            created_at: row::time(r, 8)?,
            updated_at: row::time(r, 9)?,
        })
    }
}

pub const TEMPORAL_INTERVENTION_COLUMNS: &str = "id, project_id, branch_id, title, intervention_type, source_event_id, target_event_id, \
     summary, consequences, status, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemporalIntervention {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub title: String,
    pub intervention_type: String,
    pub source_event_id: Option<String>,
    pub target_event_id: Option<String>,
    pub summary: String,
    pub consequences: Vec<String>,
    pub status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for TemporalIntervention {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            title: row::text(r, 3)?,
            intervention_type: row::text(r, 4)?,
            source_event_id: row::opt_text(r, 5)?,
            target_event_id: row::opt_text(r, 6)?,
            summary: row::text(r, 7)?,
            consequences: row::json(r, 8)?,
            status: row::text(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

pub const SYSTEM_OVERLAY_COLUMNS: &str = "id, project_id, branch_id, system_name, normalized_name, system_type, rules, visibility, \
     progression_currency, stats, advancement_tiers, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemOverlay {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub system_name: String,
    pub normalized_name: String,
    pub system_type: String,
    pub rules: String,
    pub visibility: String,
    pub progression_currency: Option<String>,
    pub stats: Vec<String>,
    pub advancement_tiers: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for SystemOverlay {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            system_name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            system_type: row::text(r, 5)?,
            rules: row::text(r, 6)?,
            visibility: row::text(r, 7)?,
            progression_currency: row::opt_text(r, 8)?,
            stats: row::json(r, 9)?,
            advancement_tiers: row::json(r, 10)?,
            created_at: row::time(r, 11)?,
            updated_at: row::time(r, 12)?,
        })
    }
}

pub const PROGRESSION_EVENT_COLUMNS: &str = "id, project_id, branch_id, subject_table, subject_id, overlay_id, kind, delta_json, \
     source_scene_id, placement, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressionEvent {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub subject_table: String,
    pub subject_id: String,
    pub overlay_id: Option<String>,
    pub kind: String,
    pub delta_json: Value,
    pub source_scene_id: Option<String>,
    pub placement: Option<StoredStoryPlacement>,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ProgressionEvent {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            subject_table: row::text(r, 3)?,
            subject_id: row::text(r, 4)?,
            overlay_id: row::opt_text(r, 5)?,
            kind: row::text(r, 6)?,
            delta_json: row::json(r, 7)?,
            source_scene_id: row::opt_text(r, 8)?,
            placement: row::opt_json(r, 9)?,
            created_at: row::time(r, 10)?,
        })
    }
}

// =============================================================================
// FutureKnowledge / KnowledgeFact / Knows (edge)
// =============================================================================

pub const FUTURE_KNOWLEDGE_COLUMNS: &str = "id, project_id, branch_id, character_id, knowledge_summary, source, learned_at, \
     expires_at, notes, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FutureKnowledge {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub knowledge_summary: String,
    pub source: String,
    pub learned_at: StoredStoryPlacement,
    pub expires_at: Option<StoredStoryPlacement>,
    pub notes: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for FutureKnowledge {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            character_id: row::text(r, 3)?,
            knowledge_summary: row::text(r, 4)?,
            source: row::text(r, 5)?,
            learned_at: row::json(r, 6)?,
            expires_at: row::opt_json(r, 7)?,
            notes: row::json(r, 8)?,
            created_at: row::time(r, 9)?,
            updated_at: row::time(r, 10)?,
        })
    }
}

pub const KNOWLEDGE_FACT_COLUMNS: &str = "id, project_id, branch_id, character_id, fact, normalized_fact, source_summary, \
     learned_at, confidence, tags, reader_visible, source_import_session_id, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeFact {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub character_id: String,
    pub fact: String,
    pub normalized_fact: String,
    pub source_summary: String,
    pub learned_at: Option<StoredStoryPlacement>,
    pub confidence: Option<f64>,
    pub tags: Vec<String>,
    pub reader_visible: bool,
    pub source_import_session_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for KnowledgeFact {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            character_id: row::text(r, 3)?,
            fact: row::text(r, 4)?,
            normalized_fact: row::text(r, 5)?,
            source_summary: row::text(r, 6)?,
            learned_at: row::opt_json(r, 7)?,
            confidence: row::opt_real(r, 8)?,
            tags: row::json(r, 9)?,
            reader_visible: row::boolean(r, 10)?,
            source_import_session_id: row::opt_text(r, 11)?,
            created_at: row::time(r, 12)?,
            updated_at: row::time(r, 13)?,
        })
    }
}

/// `knows` edge table: composite PK (branch_id, in_id, out_id). Like RelatesTo,
/// the SurrealDB surrogate `id` is gone.
pub const KNOWS_COLUMNS: &str = "in_id, out_id, project_id, branch_id, source_summary, learned_at, confidence, \
     reader_visible, source_import_session_id, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Knows {
    pub in_id: String,
    pub out_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub source_summary: Option<String>,
    pub learned_at: Option<StoredStoryPlacement>,
    pub confidence: Option<f64>,
    pub reader_visible: bool,
    pub source_import_session_id: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for Knows {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            in_id: row::text(r, 0)?,
            out_id: row::text(r, 1)?,
            project_id: row::text(r, 2)?,
            branch_id: row::text(r, 3)?,
            source_summary: row::opt_text(r, 4)?,
            learned_at: row::opt_json(r, 5)?,
            confidence: row::opt_real(r, 6)?,
            reader_visible: row::boolean(r, 7)?,
            source_import_session_id: row::opt_text(r, 8)?,
            created_at: row::time(r, 9)?,
            updated_at: row::time(r, 10)?,
        })
    }
}

// =============================================================================
// DualPersonaReview / RevisionMarker / SceneVersion / ValidatorFinding
// =============================================================================

pub const DUAL_PERSONA_REVIEW_COLUMNS: &str = "id, project_id, branch_id, scene_id, scene_revision_fingerprint, rounds_completed, \
     status, review_rounds, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DualPersonaReview {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub scene_revision_fingerprint: String,
    pub rounds_completed: i64,
    pub status: String,
    pub review_rounds: Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for DualPersonaReview {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            scene_id: row::text(r, 3)?,
            scene_revision_fingerprint: row::text(r, 4)?,
            rounds_completed: row::int(r, 5)?,
            status: row::text(r, 6)?,
            review_rounds: row::json(r, 7)?,
            created_at: row::time(r, 8)?,
            updated_at: row::time(r, 9)?,
        })
    }
}

pub const REVISION_MARKER_COLUMNS: &str = "id, project_id, branch_id, scene_id, marker_type, target_record_id, position, note, \
     status, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionMarker {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub marker_type: String,
    pub target_record_id: Option<String>,
    pub position: String,
    pub note: String,
    pub status: String,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for RevisionMarker {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            scene_id: row::text(r, 3)?,
            marker_type: row::text(r, 4)?,
            target_record_id: row::opt_text(r, 5)?,
            position: row::text(r, 6)?,
            note: row::text(r, 7)?,
            status: row::text(r, 8)?,
            created_at: row::time(r, 9)?,
        })
    }
}

pub const SCENE_VERSION_COLUMNS: &str = "id, project_id, branch_id, scene_id, version_number, book_number, chapter_number, \
     scene_order, full_text, summary, content_rating, tone, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneVersion {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub version_number: i32,
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub full_text: String,
    pub summary: String,
    pub content_rating: String,
    pub tone: Option<String>,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for SceneVersion {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            scene_id: row::text(r, 3)?,
            version_number: row::int(r, 4)? as i32,
            book_number: row::int(r, 5)? as i32,
            chapter_number: row::int(r, 6)? as i32,
            scene_order: row::int(r, 7)? as i32,
            full_text: row::text(r, 8)?,
            summary: row::text(r, 9)?,
            content_rating: row::text(r, 10)?,
            tone: row::opt_text(r, 11)?,
            created_at: row::time(r, 12)?,
        })
    }
}

pub const VALIDATOR_FINDING_COLUMNS: &str = "id, project_id, branch_id, scene_id, scene_text_hash, validator_id, finding_id, \
     severity, message, byte_range, details_json, created_at, resolved_at, context_hash";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorFinding {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub scene_text_hash: String,
    pub validator_id: String,
    pub finding_id: String,
    pub severity: String,
    pub message: String,
    pub byte_range: Option<Value>,
    pub details_json: Option<Value>,
    pub created_at: Timestamp,
    pub resolved_at: Option<Timestamp>,
    pub context_hash: Option<String>,
}

impl<'a> TryFrom<&Row<'a>> for ValidatorFinding {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            scene_id: row::text(r, 3)?,
            scene_text_hash: row::text(r, 4)?,
            validator_id: row::text(r, 5)?,
            finding_id: row::text(r, 6)?,
            severity: row::text(r, 7)?,
            message: row::text(r, 8)?,
            byte_range: row::opt_json(r, 9)?,
            details_json: row::opt_json(r, 10)?,
            created_at: row::time(r, 11)?,
            resolved_at: row::opt_time(r, 12)?,
            context_hash: row::opt_text(r, 13)?,
        })
    }
}

// =============================================================================
// SessionActivity / ResearchLog / WriterPosition
// =============================================================================

pub const SESSION_ACTIVITY_COLUMNS: &str =
    "id, project_id, branch_id, kind, subject_table, subject_id, summary, details_json, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionActivity {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub kind: String,
    pub subject_table: Option<String>,
    pub subject_id: Option<String>,
    pub summary: String,
    pub details_json: Option<Value>,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for SessionActivity {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            kind: row::text(r, 3)?,
            subject_table: row::opt_text(r, 4)?,
            subject_id: row::opt_text(r, 5)?,
            summary: row::text(r, 6)?,
            details_json: row::opt_json(r, 7)?,
            created_at: row::time(r, 8)?,
        })
    }
}

pub const RESEARCH_LOG_COLUMNS: &str =
    "id, project_id, query, context_hint, model, response, context_summary, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchLog {
    pub id: String,
    pub project_id: String,
    pub query: String,
    pub context_hint: Option<String>,
    pub model: String,
    pub response: String,
    pub context_summary: String,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ResearchLog {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            query: row::text(r, 2)?,
            context_hint: row::opt_text(r, 3)?,
            model: row::text(r, 4)?,
            response: row::text(r, 5)?,
            context_summary: row::text(r, 6)?,
            created_at: row::time(r, 7)?,
        })
    }
}

pub const WRITER_POSITION_COLUMNS: &str = "id, project_id, branch_id, book_id, chapter_id, scene_id, intent, next_focus, \
     updated_by, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriterPosition {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub book_id: Option<String>,
    pub chapter_id: Option<String>,
    pub scene_id: Option<String>,
    pub intent: String,
    pub next_focus: Option<String>,
    pub updated_by: String,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for WriterPosition {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            book_id: row::opt_text(r, 3)?,
            chapter_id: row::opt_text(r, 4)?,
            scene_id: row::opt_text(r, 5)?,
            intent: row::text(r, 6)?,
            next_focus: row::opt_text(r, 7)?,
            updated_by: row::text(r, 8)?,
            updated_at: row::time(r, 9)?,
        })
    }
}

// =============================================================================
// CanonicalFact / SceneSourceLink
// =============================================================================

pub const CANONICAL_FACT_COLUMNS: &str = "id, project_id, branch_id, scene_id, source_scene_id, book_number, chapter_number, \
     subject_table, subject_id, predicate, value_kind, value_number, value_text, value_json, \
     unit, aliases, scope, valid_from, valid_until, superseded_by, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalFact {
    pub id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: String,
    pub source_scene_id: Option<String>,
    pub book_number: i32,
    pub chapter_number: i32,
    pub subject_table: String,
    pub subject_id: Option<String>,
    pub predicate: String,
    pub value_kind: String,
    pub value_number: Option<f64>,
    pub value_text: Option<String>,
    pub value_json: Option<Value>,
    pub unit: Option<String>,
    pub aliases: Vec<String>,
    pub scope: String,
    pub valid_from: Option<StoredStoryPlacement>,
    pub valid_until: Option<StoredStoryPlacement>,
    pub superseded_by: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for CanonicalFact {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::text(r, 2)?,
            scene_id: row::text(r, 3)?,
            source_scene_id: row::opt_text(r, 4)?,
            book_number: row::int(r, 5)? as i32,
            chapter_number: row::int(r, 6)? as i32,
            subject_table: row::text(r, 7)?,
            subject_id: row::opt_text(r, 8)?,
            predicate: row::text(r, 9)?,
            value_kind: row::text(r, 10)?,
            value_number: row::opt_real(r, 11)?,
            value_text: row::opt_text(r, 12)?,
            value_json: row::opt_json(r, 13)?,
            unit: row::opt_text(r, 14)?,
            aliases: row::json(r, 15)?,
            scope: row::text(r, 16)?,
            valid_from: row::opt_json(r, 17)?,
            valid_until: row::opt_json(r, 18)?,
            superseded_by: row::opt_text(r, 19)?,
            created_at: row::time(r, 20)?,
            updated_at: row::time(r, 21)?,
        })
    }
}

pub const SCENE_SOURCE_LINK_COLUMNS: &str = "id, project_id, scene_id, source_path, content_sha256, source_start_offset, \
     source_end_offset, linked_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneSourceLink {
    pub id: String,
    pub project_id: String,
    pub scene_id: String,
    pub source_path: String,
    pub content_sha256: String,
    pub source_start_offset: Option<i64>,
    pub source_end_offset: Option<i64>,
    pub linked_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for SceneSourceLink {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            scene_id: row::text(r, 2)?,
            source_path: row::text(r, 3)?,
            content_sha256: row::text(r, 4)?,
            source_start_offset: row::opt_int(r, 5)?,
            source_end_offset: row::opt_int(r, 6)?,
            linked_at: row::time(r, 7)?,
            updated_at: row::time(r, 8)?,
        })
    }
}

// =============================================================================
// SearchEmbedding (embedding stored as BLOB of packed f32, 64-dim)
// =============================================================================

pub const SEARCH_EMBEDDING_COLUMNS: &str = "id, project_id, branch_id, entity_table, entity_id, title, excerpt, content, \
     embedding_version, embedding, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEmbedding {
    pub id: String,
    pub project_id: String,
    pub branch_id: Option<String>,
    pub entity_table: String,
    pub entity_id: String,
    pub title: String,
    pub excerpt: String,
    pub content: String,
    pub embedding_version: String,
    pub embedding: Vec<f64>,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for SearchEmbedding {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::text(r, 1)?,
            branch_id: row::opt_text(r, 2)?,
            entity_table: row::text(r, 3)?,
            entity_id: row::text(r, 4)?,
            title: row::text(r, 5)?,
            excerpt: row::text(r, 6)?,
            content: row::text(r, 7)?,
            embedding_version: row::text(r, 8)?,
            embedding: row::blob_f32_as_f64(r, 9)?,
            updated_at: row::time(r, 10)?,
        })
    }
}

// =============================================================================
// Import pipeline records
// =============================================================================

pub const IMPORT_SESSION_COLUMNS: &str = "id, project_id, target_branch_id, source_format, active_pass, progress, session_status, \
     hydrate_mode, source_count, hydration_report, imported_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSession {
    pub id: String,
    pub project_id: Option<String>,
    pub target_branch_id: Option<String>,
    pub source_format: Option<String>,
    pub active_pass: String,
    pub progress: Value,
    pub session_status: String,
    pub hydrate_mode: String,
    pub source_count: i64,
    pub hydration_report: Option<Value>,
    pub imported_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportSession {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            project_id: row::opt_text(r, 1)?,
            target_branch_id: row::opt_text(r, 2)?,
            source_format: row::opt_text(r, 3)?,
            active_pass: row::text(r, 4)?,
            progress: row::json(r, 5)?,
            session_status: row::text(r, 6)?,
            hydrate_mode: row::text(r, 7)?,
            source_count: row::int(r, 8)?,
            hydration_report: row::opt_json(r, 9)?,
            imported_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

pub const IMPORT_SOURCE_DOCUMENT_COLUMNS: &str = "id, session_id, project_id, display_name, source_path, copied_path, source_format, \
     original_sha256, normalized_sha256, normalized_text_ref, word_count, chapter_hint, \
     source_order, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSourceDocument {
    pub id: String,
    pub session_id: String,
    pub project_id: Option<String>,
    pub display_name: String,
    pub source_path: String,
    pub copied_path: String,
    pub source_format: String,
    pub original_sha256: String,
    pub normalized_sha256: String,
    pub normalized_text_ref: String,
    pub word_count: i64,
    pub chapter_hint: Option<String>,
    pub source_order: i64,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportSourceDocument {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            project_id: row::opt_text(r, 2)?,
            display_name: row::text(r, 3)?,
            source_path: row::text(r, 4)?,
            copied_path: row::text(r, 5)?,
            source_format: row::text(r, 6)?,
            original_sha256: row::text(r, 7)?,
            normalized_sha256: row::text(r, 8)?,
            normalized_text_ref: row::text(r, 9)?,
            word_count: row::int(r, 10)?,
            chapter_hint: row::opt_text(r, 11)?,
            source_order: row::int(r, 12)?,
            created_at: row::time(r, 13)?,
            updated_at: row::time(r, 14)?,
        })
    }
}

pub const IMPORT_SEGMENT_COLUMNS: &str = "id, session_id, source_document_id, parent_segment_id, segment_type, source_order, \
     book_number, chapter_number, scene_order, label, start_offset, end_offset, word_count, \
     character_count, pov_guess, confidence, segment_status, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportSegment {
    pub id: String,
    pub session_id: String,
    pub source_document_id: String,
    pub parent_segment_id: Option<String>,
    pub segment_type: String,
    pub source_order: i64,
    pub book_number: Option<i64>,
    pub chapter_number: Option<i64>,
    pub scene_order: Option<i64>,
    pub label: Option<String>,
    pub start_offset: i64,
    pub end_offset: i64,
    pub word_count: i64,
    pub character_count: i64,
    pub pov_guess: Option<Value>,
    pub confidence: f64,
    pub segment_status: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportSegment {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            source_document_id: row::text(r, 2)?,
            parent_segment_id: row::opt_text(r, 3)?,
            segment_type: row::text(r, 4)?,
            source_order: row::int(r, 5)?,
            book_number: row::opt_int(r, 6)?,
            chapter_number: row::opt_int(r, 7)?,
            scene_order: row::opt_int(r, 8)?,
            label: row::opt_text(r, 9)?,
            start_offset: row::int(r, 10)?,
            end_offset: row::int(r, 11)?,
            word_count: row::int(r, 12)?,
            character_count: row::int(r, 13)?,
            pov_guess: row::opt_json(r, 14)?,
            confidence: row::real(r, 15)?,
            segment_status: row::text(r, 16)?,
            created_at: row::time(r, 17)?,
            updated_at: row::time(r, 18)?,
        })
    }
}

pub const IMPORT_ENTITY_MENTION_COLUMNS: &str = "id, session_id, segment_id, entity_kind, surface_form, normalized_name, alias_hint, \
     surrounding_text, confidence, extraction_pass, created_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEntityMention {
    pub id: String,
    pub session_id: String,
    pub segment_id: String,
    pub entity_kind: String,
    pub surface_form: String,
    pub normalized_name: String,
    pub alias_hint: Option<String>,
    pub surrounding_text: Option<String>,
    pub confidence: f64,
    pub extraction_pass: String,
    pub created_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportEntityMention {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            segment_id: row::text(r, 2)?,
            entity_kind: row::text(r, 3)?,
            surface_form: row::text(r, 4)?,
            normalized_name: row::text(r, 5)?,
            alias_hint: row::opt_text(r, 6)?,
            surrounding_text: row::opt_text(r, 7)?,
            confidence: row::real(r, 8)?,
            extraction_pass: row::text(r, 9)?,
            created_at: row::time(r, 10)?,
        })
    }
}

pub const IMPORT_ENTITY_CLUSTER_COLUMNS: &str = "id, session_id, entity_kind, canonical_name, normalized_name, aliases, mention_ids, \
     first_segment_id, last_segment_id, importance_rank, merge_confidence, review_required, \
     notes, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportEntityCluster {
    pub id: String,
    pub session_id: String,
    pub entity_kind: String,
    pub canonical_name: String,
    pub normalized_name: String,
    pub aliases: Vec<String>,
    pub mention_ids: Vec<String>,
    pub first_segment_id: Option<String>,
    pub last_segment_id: Option<String>,
    pub importance_rank: i64,
    pub merge_confidence: f64,
    pub review_required: bool,
    pub notes: Vec<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportEntityCluster {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            entity_kind: row::text(r, 2)?,
            canonical_name: row::text(r, 3)?,
            normalized_name: row::text(r, 4)?,
            aliases: row::json(r, 5)?,
            mention_ids: row::json(r, 6)?,
            first_segment_id: row::opt_text(r, 7)?,
            last_segment_id: row::opt_text(r, 8)?,
            importance_rank: row::int(r, 9)?,
            merge_confidence: row::real(r, 10)?,
            review_required: row::boolean(r, 11)?,
            notes: row::json(r, 12)?,
            created_at: row::time(r, 13)?,
            updated_at: row::time(r, 14)?,
        })
    }
}

pub const IMPORT_CHARACTER_DOSSIER_COLUMNS: &str = "id, session_id, cluster_id, canonical_name, aliases, importance_rank, voice_profile, \
     emotional_profile, state_trajectory, relationship_inferences, decision_patterns, \
     dialogue_samples, confidence, review_required, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportCharacterDossier {
    pub id: String,
    pub session_id: String,
    pub cluster_id: String,
    pub canonical_name: String,
    pub aliases: Vec<String>,
    pub importance_rank: i64,
    pub voice_profile: Value,
    pub emotional_profile: Value,
    pub state_trajectory: Value,
    pub relationship_inferences: Value,
    pub decision_patterns: Vec<String>,
    pub dialogue_samples: Vec<String>,
    pub confidence: f64,
    pub review_required: bool,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportCharacterDossier {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            cluster_id: row::text(r, 2)?,
            canonical_name: row::text(r, 3)?,
            aliases: row::json(r, 4)?,
            importance_rank: row::int(r, 5)?,
            voice_profile: row::json(r, 6)?,
            emotional_profile: row::json(r, 7)?,
            state_trajectory: row::json(r, 8)?,
            relationship_inferences: row::json(r, 9)?,
            decision_patterns: row::json(r, 10)?,
            dialogue_samples: row::json(r, 11)?,
            confidence: row::real(r, 12)?,
            review_required: row::boolean(r, 13)?,
            created_at: row::time(r, 14)?,
            updated_at: row::time(r, 15)?,
        })
    }
}

pub const IMPORT_WORLD_DOSSIER_COLUMNS: &str =
    "id, session_id, world_rules, locations, entities, system_signals, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportWorldDossier {
    pub id: String,
    pub session_id: String,
    pub world_rules: Value,
    pub locations: Value,
    pub entities: Value,
    pub system_signals: Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportWorldDossier {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            world_rules: row::json(r, 2)?,
            locations: row::json(r, 3)?,
            entities: row::json(r, 4)?,
            system_signals: row::json(r, 5)?,
            created_at: row::time(r, 6)?,
            updated_at: row::time(r, 7)?,
        })
    }
}

pub const IMPORT_NARRATIVE_DOSSIER_COLUMNS: &str = "id, session_id, plot_lines, conflicts, narrative_promises, arcs, themes, motifs, \
     reader_contract, pacing_hints, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportNarrativeDossier {
    pub id: String,
    pub session_id: String,
    pub plot_lines: Value,
    pub conflicts: Value,
    pub narrative_promises: Value,
    pub arcs: Value,
    pub themes: Value,
    pub motifs: Value,
    pub reader_contract: Value,
    pub pacing_hints: Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportNarrativeDossier {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            plot_lines: row::json(r, 2)?,
            conflicts: row::json(r, 3)?,
            narrative_promises: row::json(r, 4)?,
            arcs: row::json(r, 5)?,
            themes: row::json(r, 6)?,
            motifs: row::json(r, 7)?,
            reader_contract: row::json(r, 8)?,
            pacing_hints: row::json(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

pub const IMPORT_RESUME_SNAPSHOT_COLUMNS: &str = "id, session_id, book_number, chapter_number, scene_order, summary, characters, \
     relationships, locations, plot_threads, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResumeSnapshot {
    pub id: String,
    pub session_id: String,
    pub book_number: i64,
    pub chapter_number: i64,
    pub scene_order: Option<i64>,
    pub summary: String,
    pub characters: Value,
    pub relationships: Value,
    pub locations: Value,
    pub plot_threads: Value,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportResumeSnapshot {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            book_number: row::int(r, 2)?,
            chapter_number: row::int(r, 3)?,
            scene_order: row::opt_int(r, 4)?,
            summary: row::text(r, 5)?,
            characters: row::json(r, 6)?,
            relationships: row::json(r, 7)?,
            locations: row::json(r, 8)?,
            plot_threads: row::json(r, 9)?,
            created_at: row::time(r, 10)?,
            updated_at: row::time(r, 11)?,
        })
    }
}

pub const IMPORT_REVIEW_ITEM_COLUMNS: &str = "id, session_id, pass_name, item_kind, severity, status, title, description, \
     related_segment_ids, related_entity_ids, confidence, proposed_correction, \
     resolver_notes, created_at, updated_at";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportReviewItem {
    pub id: String,
    pub session_id: String,
    pub pass_name: String,
    pub item_kind: String,
    pub severity: String,
    pub status: String,
    pub title: String,
    pub description: String,
    pub related_segment_ids: Vec<String>,
    pub related_entity_ids: Vec<String>,
    pub confidence: Option<f64>,
    pub proposed_correction: Option<Value>,
    pub resolver_notes: Option<String>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl<'a> TryFrom<&Row<'a>> for ImportReviewItem {
    type Error = rusqlite::Error;
    fn try_from(r: &Row<'a>) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row::text(r, 0)?,
            session_id: row::text(r, 1)?,
            pass_name: row::text(r, 2)?,
            item_kind: row::text(r, 3)?,
            severity: row::text(r, 4)?,
            status: row::text(r, 5)?,
            title: row::text(r, 6)?,
            description: row::text(r, 7)?,
            related_segment_ids: row::json(r, 8)?,
            related_entity_ids: row::json(r, 9)?,
            confidence: row::opt_real(r, 10)?,
            proposed_correction: row::opt_json(r, 11)?,
            resolver_notes: row::opt_text(r, 12)?,
            created_at: row::time(r, 13)?,
            updated_at: row::time(r, 14)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::SqlitePool;
    use tempfile::TempDir;

    /// Round-trip the foundational records through INSERT and SELECT against
    /// the V0001 schema. Catches column-order drift between `*_COLUMNS` and
    /// the `TryFrom` impl.
    #[tokio::test]
    async fn foundational_records_round_trip() {
        let tmp = TempDir::new().unwrap();
        let pool = SqlitePool::open(&tmp.path().join("rec.db")).await.unwrap();

        // Insert minimum-viable graph: project -> branch -> book -> chapter -> scene + character.
        pool.write(|conn| {
            conn.execute_batch(
                "INSERT INTO project (id, name, project_type, genre, reader_contract, created_at, updated_at) \
                   VALUES ('project:01HRT', 'p', 'novel', 'fantasy', \
                           '{\"promise\":\"prom\",\"style_notes\":[],\"boundaries\":[]}', 1, 1);
                 INSERT INTO bible_branch (id, project_id, name, status, created_at) \
                   VALUES ('bible_branch:01HRT', 'project:01HRT', 'main', 'active', 1);
                 INSERT INTO book (id, project_id, book_number, created_at) \
                   VALUES ('book:01HRT', 'project:01HRT', 1, 1);
                 INSERT INTO chapter (id, project_id, book_id, book_number, chapter_number, created_at) \
                   VALUES ('chapter:01HRT', 'project:01HRT', 'book:01HRT', 1, 1, 1);
                 INSERT INTO scene (id, project_id, branch_id, book_id, chapter_id, book_number, chapter_number, scene_order, full_text, summary, content_rating, created_at, updated_at) \
                   VALUES ('scene:01HRT', 'project:01HRT', 'bible_branch:01HRT', 'book:01HRT', 'chapter:01HRT', 1, 1, 1, 'text', 'sum', 'General', 1, 1);
                 INSERT INTO character (id, project_id, branch_id, name, normalized_name, summary, role, created_at, updated_at) \
                   VALUES ('character:01HRT', 'project:01HRT', 'bible_branch:01HRT', 'C', 'c', 's', 'r', 1, 1);
                 INSERT INTO character_voice_profile (id, character_id, vocabulary, sentence_structure, tics, forbidden_words, example_lines, created_at) \
                   VALUES ('character_voice_profile:01HRT', 'character:01HRT', '[]', '[]', '[]', '[]', '[]', 1);",
            )?;
            Ok(())
        })
        .await
        .unwrap();

        // Read each one back and assert non-error decode.
        let project: Project = pool
            .read(|c| {
                c.query_row(
                    &format!(
                        "SELECT {} FROM project WHERE id = 'project:01HRT'",
                        PROJECT_COLUMNS
                    ),
                    [],
                    |r| Project::try_from(r),
                )
            })
            .await
            .unwrap();
        assert_eq!(project.id, "project:01HRT");

        let branch: BibleBranch = pool
            .read(|c| {
                c.query_row(
                    &format!(
                        "SELECT {} FROM bible_branch WHERE id = 'bible_branch:01HRT'",
                        BIBLE_BRANCH_COLUMNS
                    ),
                    [],
                    |r| BibleBranch::try_from(r),
                )
            })
            .await
            .unwrap();
        assert_eq!(branch.name, "main");

        let book: Book = pool
            .read(|c| {
                c.query_row(
                    &format!("SELECT {} FROM book WHERE id = 'book:01HRT'", BOOK_COLUMNS),
                    [],
                    |r| Book::try_from(r),
                )
            })
            .await
            .unwrap();
        assert_eq!(book.book_number, 1);

        let chapter: Chapter = pool
            .read(|c| {
                c.query_row(
                    &format!(
                        "SELECT {} FROM chapter WHERE id = 'chapter:01HRT'",
                        CHAPTER_COLUMNS
                    ),
                    [],
                    |r| Chapter::try_from(r),
                )
            })
            .await
            .unwrap();
        assert_eq!(chapter.chapter_number, 1);

        let scene: Scene = pool
            .read(|c| {
                c.query_row(
                    &format!(
                        "SELECT {} FROM scene WHERE id = 'scene:01HRT'",
                        SCENE_COLUMNS
                    ),
                    [],
                    |r| Scene::try_from(r),
                )
            })
            .await
            .unwrap();
        assert_eq!(scene.content_rating, "General");

        let character: Character = pool
            .read(|c| {
                c.query_row(
                    &format!(
                        "SELECT {} FROM character WHERE id = 'character:01HRT'",
                        CHARACTER_COLUMNS
                    ),
                    [],
                    |r| Character::try_from(r),
                )
            })
            .await
            .unwrap();
        assert_eq!(character.name, "C");

        let voice: CharacterVoiceProfile = pool
            .read(|c| {
                c.query_row(
                    &format!(
                        "SELECT {} FROM character_voice_profile WHERE id = 'character_voice_profile:01HRT'",
                        CHARACTER_VOICE_PROFILE_COLUMNS
                    ),
                    [],
                    |r| CharacterVoiceProfile::try_from(r),
                )
            })
            .await
            .unwrap();
        assert!(voice.vocabulary.is_empty());
    }
}
