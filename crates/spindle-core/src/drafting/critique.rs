//! Versioned scene-critic DTOs, UTF-8 byte-span anchors, and shadow-critique contracts.
//! Pure data contracts only; no prompt rendering, tokenizer execution, or transport.

use crate::models::ContentRating;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const CRITIQUE_SCHEMA_VERSION: &str = "critique_v1";
pub const SHADOW_CRITIQUE_ARTIFACT_SCHEMA_VERSION: &str = "shadow_critique_artifact_v1";
pub const MAX_ACTIONABLE_DIRECTIVES: usize = 5;

fn default_critique_schema_version() -> String {
    CRITIQUE_SCHEMA_VERSION.into()
}

fn default_shadow_critique_artifact_schema_version() -> String {
    SHADOW_CRITIQUE_ARTIFACT_SCHEMA_VERSION.into()
}

/// Ten diagnostic dimensions assessed by `CritiqueV1`: the seven `prose_judge.py`
/// rubric dimensions plus scene length, entry-to-exit emotional shift, and chapter role.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CriticDimension {
    #[serde(alias = "scene_purpose_fulfilled")]
    ScenePurpose,
    TensionTurn,
    #[serde(alias = "pov_discipline")]
    Pov,
    #[serde(alias = "voice_distinctiveness")]
    Voice,
    Dialogue,
    #[serde(alias = "prose_freshness")]
    Freshness,
    #[serde(alias = "continuity_with_brief")]
    Continuity,
    Length,
    EntryExitEmotion,
    ChapterRole,
}

impl CriticDimension {
    pub const ALL: [Self; 10] = [
        Self::ScenePurpose,
        Self::TensionTurn,
        Self::Pov,
        Self::Voice,
        Self::Dialogue,
        Self::Freshness,
        Self::Continuity,
        Self::Length,
        Self::EntryExitEmotion,
        Self::ChapterRole,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ScenePurpose => "scene_purpose",
            Self::TensionTurn => "tension_turn",
            Self::Pov => "pov",
            Self::Voice => "voice",
            Self::Dialogue => "dialogue",
            Self::Freshness => "freshness",
            Self::Continuity => "continuity",
            Self::Length => "length",
            Self::EntryExitEmotion => "entry_exit_emotion",
            Self::ChapterRole => "chapter_role",
        }
    }

    /// Priority ordering tier for directive selection:
    /// Tier 0: continuity & hard constraints
    /// Tier 1: author intent & structural scene contract
    /// Tier 2: craft, voice & polish
    pub fn priority_tier(&self) -> u8 {
        match self {
            Self::Continuity => 0,
            Self::ScenePurpose
            | Self::TensionTurn
            | Self::Pov
            | Self::EntryExitEmotion
            | Self::ChapterRole => 1,
            Self::Voice | Self::Dialogue | Self::Freshness | Self::Length => 2,
        }
    }
}

/// Half-open UTF-8 byte range (`[start_byte, end_byte)`) anchored to a specific
/// `base_text_hash` and `scene_id`.
///
/// Non-empty spans must match `&base_text[start_byte..end_byte] == exact_quote`.
/// Zero-width insertion anchors (`start_byte == end_byte`) must have an empty
/// `exact_quote` and at least one non-empty surrounding context (`context_before`
/// or `context_after`) matching adjacent UTF-8 bytes in `base_text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CritiqueAnchor {
    pub scene_id: Option<String>,
    pub base_text_hash: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub exact_quote: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_after: Option<String>,
}

/// Diagnostic 1..=5 score and anchored evidence for a single `CriticDimension`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CriticDimensionScore {
    pub dimension: CriticDimension,
    pub score_1_to_5: u8,
    #[serde(default)]
    pub evidence_spans: Vec<CritiqueAnchor>,
    pub rationale: String,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveSeverity {
    Blocker,
    Major,
    Minor,
    Polish,
}

impl DirectiveSeverity {
    pub fn rank(&self) -> u8 {
        match self {
            Self::Blocker => 0,
            Self::Major => 1,
            Self::Minor => 2,
            Self::Polish => 3,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DirectiveScope {
    Span,
    Scene,
    Chapter,
}

/// Actionable editorial directive anchored to exact UTF-8 bytes in the base draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CritiqueDirective {
    pub id: String,
    pub dimension: CriticDimension,
    pub severity: DirectiveSeverity,
    /// Priority within severity tier (`1..=5`, where `1` is highest priority).
    pub priority: u8,
    pub anchor: CritiqueAnchor,
    pub diagnosis: String,
    pub requested_change: String,
    #[serde(default)]
    pub preserve: Vec<String>,
    pub success_test: String,
    #[serde(default)]
    pub source_finding_ids: Vec<String>,
    pub scope: DirectiveScope,
}

/// Suggestion or conflict that cannot be executed automatically because it
/// touches an `AuthorLock`, `prohibited_outcomes`, planned `intended_outcome` /
/// `value_shift`, or new canon creation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuthorDecisionItem {
    pub id: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking_constraint_id: Option<String>,
    pub proposed_change: String,
    #[serde(default)]
    pub source_finding_ids: Vec<String>,
}

/// Typed reason for a no-change critic verdict. A generic "looks good" string is
/// not sufficient: the critic must choose one of these codes and explain it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum NoChangeReason {
    /// The draft has no material issue worth a revision.
    NoMaterialIssue,
    /// The draft already satisfies the scene card and hard constraints.
    AlreadySatisfiesBrief,
    /// The critic cannot establish a material issue from the supplied evidence.
    InsufficientEvidence,
    /// A potential change would require author judgment (locks, canon, planned outcome).
    AuthorIntentAmbiguous,
    /// Hard constraints prevent a safe revision.
    HardConstraintsPreventChange,
}

/// Explicit typed no-change verdict, required when `CritiqueV1.directives` is empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoChangeV1 {
    pub reason_code: NoChangeReason,
    pub explanation: String,
}

/// Versioned scene-critic output contract (`docs/drafting-and-revision-design.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CritiqueV1 {
    #[serde(default = "default_critique_schema_version")]
    pub schema_version: String,
    pub base_text_hash: String,
    pub packet_hash: String,
    pub rubric_version: String,
    pub dimensions: Vec<CriticDimensionScore>,
    pub directives: Vec<CritiqueDirective>,
    #[serde(default)]
    pub strengths_to_preserve: Vec<String>,
    #[serde(default)]
    pub requires_author_decision: Vec<AuthorDecisionItem>,
    /// Legacy human-readable mirror. New outputs must use `no_change`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_edit_reason: Option<String>,
    /// Required when `directives` is empty; must be absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_change: Option<NoChangeV1>,
}

impl CritiqueV1 {
    /// Return the generated JSON Schema for `CritiqueV1`.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Self)).expect("CritiqueV1 JSON schema")
    }
}

/// Source family of an ingested finding passed into the critic.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackSourceKind {
    Continuity,
    AntiSlop,
    ReaderPersona,
    DualPersona,
}

/// Deterministic or reader-persona finding ingested before critic execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CriticFindingInput {
    pub finding_id: String,
    pub source: FeedbackSourceKind,
    pub check_type: String,
    pub dimension: CriticDimension,
    pub severity: DirectiveSeverity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CritiqueAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<String>,
}

/// Deduplicated finding record after merging redundant deterministic and reader concerns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IngestedFinding {
    pub canonical_id: String,
    pub merged_finding_ids: Vec<String>,
    pub sources: Vec<FeedbackSourceKind>,
    pub check_type: String,
    pub dimension: CriticDimension,
    pub severity: DirectiveSeverity,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<CritiqueAnchor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<String>,
    pub fingerprint: String,
}

/// Outcome of a shadow-mode critic pass. Never mutates the base draft text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShadowCritiqueOutcome {
    Critiqued {
        critique: Box<CritiqueV1>,
        repair_used: bool,
    },
    CriticInvalid {
        reason: String,
        attempts: u32,
        repair_attempted: bool,
    },
}

/// Persisted shadow-critique artifact bound to an immutable `PlainTextDraftArtifactV1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ShadowCritiqueArtifactV1 {
    #[serde(default = "default_shadow_critique_artifact_schema_version")]
    pub schema_version: String,
    pub artifact_id: String,
    pub draft_artifact_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub scene_id: Option<String>,
    pub base_text_hash: String,
    pub packet_hash: String,
    pub rubric_version: String,
    pub rating: ContentRating,
    pub ingested_findings: Vec<IngestedFinding>,
    pub deduplicated_finding_ids: Vec<String>,
    pub prompt_hashes: Vec<String>,
    pub model_call_ids: Vec<String>,
    pub outcome: ShadowCritiqueOutcome,
}
