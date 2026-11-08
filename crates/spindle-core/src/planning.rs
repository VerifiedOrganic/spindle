//! Initial-planning contracts. These never authorize in-run amendments.
use crate::models::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PlanningRef {
    ExistingRecordId(String),
    ProposalRef(String),
}

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum PlanningLevel {
    Contract,
    CastWorld,
    SeriesArc,
    Book,
    Chapter,
    Scene,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningGate {
    #[default]
    Human,
    DelegateIfValid,
    Hold,
}

/// An empty map means human gates at every level. Non-human policy activation
/// is deliberately unavailable until the audited apply path ships.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningPolicy {
    pub gates: BTreeMap<PlanningLevel, PlanningGate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanningScope {
    pub project_id: String,
    pub branch_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginPlanningInput {
    pub premise: String,
    pub scope: Option<PlanningScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningSession {
    pub id: String,
    pub premise: String,
    pub intent_hash: String,
    pub scope: Option<PlanningScope>,
    pub policy: PlanningPolicy,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "operation",
    content = "input",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum PlanningOperation {
    CreateProject(PlanningCreateProjectInput),
    CreateCharacter(Box<PlanningCreateCharacterInput>),
    CreateLocation(PlanningCreateLocationInput),
    CreateWorldRule(PlanningCreateWorldRuleInput),
    CreateRelationship(PlanningCreateRelationshipInput),
    CreatePlotLine(PlanningCreatePlotLineInput),
    CreateConflict(PlanningCreateConflictInput),
    CreateTheme(PlanningCreateThemeInput),
    CreateMotif(PlanningCreateMotifInput),
    CreateNarrativePromise(PlanningCreateNarrativePromiseInput),
    CreateCharacterArc(PlanningCreateCharacterArcInput),
    CreatePacingConfig(PlanningCreatePacingConfigInput),
    CreatePacingCurve(PlanningCreatePacingCurveInput),
    SetArcPacingConstraints(PlanningSetArcPacingConstraintsInput),
    CreateBook(PlanningCreateBookInput),
    CreateChapter(PlanningCreateChapterInput),
    SetBookOutline(PlanningSetBookOutlineInput),
    SetChapterOutline(PlanningSetChapterOutlineInput),
    PlanChapter(PlanningPlanChapterInput),
    UpdateReaderContract(ReaderContract),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateProjectInput {
    pub name: String,
    pub project_type: String,
    pub genre: String,
    pub reader_contract: ReaderContract,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateCharacterInput {
    pub name: String,
    pub summary: String,
    pub role: String,
    pub realm: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub voice_profile: PlanningCharacterVoiceProfileData,
    #[serde(default)]
    pub emotional_profile: CharacterEmotionalProfileData,
    pub initial_state: Option<CharacterStatePatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateLocationInput {
    pub name: String,
    #[serde(default, alias = "type")]
    pub kind: String,
    pub realm: Option<String>,
    pub summary: String,
    #[serde(default)]
    pub initial_state: PlanningWorldStateInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateWorldRuleInput {
    pub rule_name: String,
    pub rule_type: String,
    pub description: String,
    #[serde(default)]
    pub scan_pattern: Option<String>,
    #[serde(default)]
    pub relevance_tags: Vec<String>,
    pub established_in: Option<EstablishedIn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateRelationshipInput {
    pub character_a_id: PlanningRef,
    pub character_b_id: PlanningRef,
    pub relationship_type: String,
    pub initial_trust: i32,
    pub initial_tension: i32,
    #[serde(default)]
    pub dynamics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreatePlotLineInput {
    pub name: String,
    pub plot_type: String,
    pub summary: String,
    pub status: Option<String>,
    #[serde(default)]
    pub convergence_points: Vec<StoryPlacement>,
    #[serde(default)]
    pub connected_conflict_ids: Vec<PlanningRef>,
    #[serde(default)]
    pub connected_theme_ids: Vec<PlanningRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateConflictInput {
    pub name: String,
    pub conflict_type: String,
    pub stakes: String,
    #[serde(default)]
    pub escalation_stages: Vec<String>,
    pub expected_total_cycles: Option<i32>,
    #[serde(default)]
    pub try_fail_cycles: Vec<TryFailCycleStep>,
    #[serde(default)]
    pub stated_consequences: Vec<StatedConsequence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateThemeInput {
    pub theme_statement: String,
    pub thesis_antithesis: String,
    pub introduction_point: Option<StoryPlacement>,
    pub resolution_point: Option<StoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateMotifInput {
    pub name: String,
    pub description: String,
    pub max_uses_per_chapter: Option<i32>,
    #[serde(default)]
    pub connected_theme_ids: Vec<PlanningRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateNarrativePromiseInput {
    pub promise_type: String,
    pub description: String,
    pub planted_at: StoryPlacement,
    pub planned_payoff: Option<StoryPlacement>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateCharacterArcInput {
    pub character_id: PlanningRef,
    pub arc_type: String,
    pub starting_state: String,
    pub ending_state: String,
    #[serde(default)]
    pub milestones: Vec<CharacterArcMilestone>,
    pub thematic_purpose: String,
    #[serde(default)]
    pub connected_theme_ids: Vec<PlanningRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreatePacingConfigInput {
    pub total_planned_books: i32,
    pub avg_chapters_per_book: i32,
    pub avg_scenes_per_chapter: i32,
    pub tension_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreatePacingCurveInput {
    pub book_number: i32,
    pub act_breakpoints: BTreeMap<String, f64>,
    pub scene_type_density: BTreeMap<String, f64>,
    #[serde(default)]
    pub intensity_points: Vec<IntensityPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningSetArcPacingConstraintsInput {
    pub character_arc_id: PlanningRef,
    pub per_book_budget: BTreeMap<String, f64>,
    pub max_progress_per_chapter: Option<f64>,
    pub milestone_spacing: Option<i32>,
    pub sprint_allowance: Option<i32>,
    pub regression_budget: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateBookInput {
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateChapterInput {
    pub book_number: Option<i32>,
    pub book_id: Option<PlanningRef>,
    pub chapter_number: Option<i32>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningSetBookOutlineInput {
    #[serde(default)]
    pub book_number: i32,
    #[serde(default)]
    pub book_id: Option<PlanningRef>,
    #[serde(default)]
    pub format: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningSetChapterOutlineInput {
    #[serde(default)]
    pub book_number: i32,
    #[serde(default)]
    pub chapter_number: i32,
    #[serde(default, alias = "chapter_id")]
    pub entity_id: Option<PlanningRef>,
    #[serde(default)]
    pub entity_type: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub beats: Vec<PlanningChapterOutlineBeat>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningPlanChapterInput {
    pub book_number: i32,
    pub chapter_number: i32,
    pub pov_character_id: Option<PlanningRef>,
    pub synopsis: String,
    #[serde(default)]
    pub target_theme_ids: Vec<PlanningRef>,
    #[serde(default)]
    pub target_conflict_ids: Vec<PlanningRef>,
    #[serde(default)]
    pub target_plot_line_ids: Vec<PlanningRef>,
    #[serde(default)]
    pub scenes: Vec<PlanningPlanChapterSceneInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct PlanningPlanChapterSceneInput {
    pub scene_order: i32,
    pub summary: String,
    #[serde(default)]
    pub beat_structure: Vec<String>,
    #[serde(default)]
    pub character_ids: Vec<PlanningRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_id: Option<PlanningRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_rating: Option<ContentRating>,
    pub purpose: String,
    #[serde(default)]
    pub research_required: Option<bool>,
    #[serde(default)]
    pub research_tags: Vec<String>,
    #[serde(default)]
    pub explicit_query: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct PlanningCharacterVoiceProfileData {
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub vocabulary: Vec<String>,
    #[serde(default)]
    pub sentence_structure: Vec<String>,
    #[serde(default)]
    pub tics: Vec<String>,
    #[serde(default)]
    pub forbidden_words: Vec<String>,
    #[serde(default)]
    pub example_lines: Vec<String>,
    #[serde(default)]
    pub established_in_scene_id: Option<PlanningRef>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct PlanningWorldStateInput {
    pub controlling_faction: Option<PlanningRef>,
    pub status: Option<String>,
    pub prosperity: Option<String>,
    pub stability: Option<String>,
    pub threat_level: Option<String>,
    #[serde(default)]
    pub sensory_details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct PlanningChapterOutlineBeat {
    pub order: i32,
    pub summary: String,
    pub scene_id: Option<PlanningRef>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningOperationItem {
    pub output_ref: String,
    pub operation: PlanningOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StagePlanningInput {
    pub session_id: String,
    pub scope: Option<PlanningScope>,
    pub intent_hash: String,
    pub schema_version: u32,
    pub level: PlanningLevel,
    /// Exact hashes of all external proposal dependencies.
    pub parents: BTreeMap<String, String>,
    pub operations: Vec<PlanningOperationItem>,
    pub rationale: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningProposalStatus {
    Staged,
    Applied,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanningProposal {
    pub id: String,
    pub artifact: StagePlanningInput,
    pub payload_hash: String,
    pub status: PlanningProposalStatus,
    pub policy_snapshot: PlanningPolicy,
    pub decided_by: Option<String>,
    pub decision_note: Option<String>,
    pub decided_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningDecision {
    Accept,
    Reject,
    Supersede,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecidePlanningInput {
    pub session_id: String,
    pub scope: Option<PlanningScope>,
    pub expected_hashes: BTreeMap<String, String>,
    pub decision: PlanningDecision,
    pub actor: String,
    pub note: String,
}
