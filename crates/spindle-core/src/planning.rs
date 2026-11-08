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
    UpdateReaderContract(PlanningReaderContract),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateProjectInput {
    pub name: String,
    pub project_type: String,
    pub genre: String,
    pub reader_contract: PlanningReaderContract,
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
    pub emotional_profile: PlanningCharacterEmotionalProfileData,
    pub initial_state: Option<PlanningCharacterStatePatch>,
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
    pub established_in: Option<PlanningEstablishedIn>,
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
    pub convergence_points: Vec<PlanningStoryPlacement>,
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
    pub try_fail_cycles: Vec<PlanningTryFailCycleStep>,
    #[serde(default)]
    pub stated_consequences: Vec<PlanningStatedConsequence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCreateThemeInput {
    pub theme_statement: String,
    pub thesis_antithesis: String,
    pub introduction_point: Option<PlanningStoryPlacement>,
    pub resolution_point: Option<PlanningStoryPlacement>,
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
    pub planted_at: PlanningStoryPlacement,
    pub planned_payoff: Option<PlanningStoryPlacement>,
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
    pub milestones: Vec<PlanningCharacterArcMilestone>,
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
    pub intensity_points: Vec<PlanningIntensityPoint>,
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
    /// Existing records/targets captured at staging or approved parent-ID resolution;
    /// checked again under the writer gate.
    #[serde(default)]
    pub base_state: BTreeMap<String, String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningReaderContract {
    pub promise: String,
    #[serde(default)]
    pub style_notes: Vec<String>,
    #[serde(default)]
    pub boundaries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCharacterStatePatch {
    #[serde(default)]
    pub emotional_state: BTreeMap<String, serde_json::Value>,
    pub goals: Option<Vec<String>>,
    pub status: Option<Vec<String>>,
    pub notes: Option<Vec<String>>,
    pub source_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct PlanningCharacterEmotionalProfileData {
    #[serde(default)]
    pub base_emotions: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub suppressed: Vec<String>,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub defense_mechanisms: Vec<String>,
    pub flex_range: Option<PlanningFlexRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningFlexRange {
    pub low: Option<String>,
    pub high: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningEstablishedIn {
    pub book_number: i32,
    pub chapter_number: i32,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningStoryPlacement {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: Option<i32>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningCharacterArcMilestone {
    pub label: String,
    pub placement: Option<PlanningStoryPlacement>,
    pub description: String,
    #[serde(default)]
    pub unlocks: Vec<String>,
    #[serde(default)]
    pub reached_at: Option<PlanningStoryPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningStatedConsequence {
    pub description: String,
    pub stated_at: Option<PlanningStoryPlacement>,
    pub must_demonstrate_by: Option<String>,
    pub delivered: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningTryFailCycleStep {
    pub attempt_order: i32,
    pub label: String,
    pub outcome: String,
    pub cost: Option<String>,
    pub revelation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningIntensityPoint {
    pub position: f64,
    pub intensity: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};

    fn rejects_extra<T: DeserializeOwned>(mut value: Value) {
        assert!(
            serde_json::from_value::<T>(value.clone()).is_ok(),
            "valid nested fixture must parse"
        );
        value
            .as_object_mut()
            .unwrap()
            .insert("unexpected_field".into(), json!(true));
        assert!(
            serde_json::from_value::<T>(value).is_err(),
            "nested unknown fields must fail closed"
        );
    }

    #[test]
    fn nested_planning_contracts_are_strict() {
        rejects_extra::<PlanningReaderContract>(json!({"promise":"Discovery"}));
        rejects_extra::<PlanningCharacterStatePatch>(json!({}));
        rejects_extra::<PlanningCharacterEmotionalProfileData>(json!({}));
        rejects_extra::<PlanningFlexRange>(json!({}));
        rejects_extra::<PlanningEstablishedIn>(json!({"book_number":1,"chapter_number":1}));
        rejects_extra::<PlanningStoryPlacement>(json!({"book_number":1,"chapter_number":1}));
        rejects_extra::<PlanningCharacterArcMilestone>(
            json!({"label":"Trust","description":"Accept help"}),
        );
        rejects_extra::<PlanningStatedConsequence>(
            json!({"description":"Exposure","delivered":false}),
        );
        rejects_extra::<PlanningTryFailCycleStep>(
            json!({"attempt_order":1,"label":"Ask","outcome":"Refused"}),
        );
        rejects_extra::<PlanningIntensityPoint>(json!({"position":0.0,"intensity":0.5}));
        rejects_extra::<PlanningCharacterVoiceProfileData>(json!({}));
        rejects_extra::<PlanningWorldStateInput>(json!({}));
        rejects_extra::<PlanningChapterOutlineBeat>(
            json!({"order":1,"summary":"Clue","status":"planned"}),
        );
        rejects_extra::<PlanningPlanChapterSceneInput>(
            json!({"scene_order":1,"summary":"Clue","purpose":"Discovery"}),
        );
    }

    fn rejects_at(mut operation: Value, path: &str) {
        assert!(
            serde_json::from_value::<PlanningOperation>(operation.clone()).is_ok(),
            "valid operation fixture must parse"
        );
        operation
            .pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unexpected_field".into(), json!(true));
        assert!(
            serde_json::from_value::<PlanningOperation>(operation).is_err(),
            "unknown nested operation input must fail closed"
        );
    }

    #[test]
    fn operation_tree_rejects_nested_unknowns_before_hashing() {
        rejects_at(
            json!({"operation":"update_reader_contract","input":{"promise":"Discovery"}}),
            "/input",
        );
        rejects_at(
            json!({"operation":"create_project","input":{"name":"Harbor","project_type":"novel","genre":"mystery","reader_contract":{"promise":"Discovery"}}}),
            "/input/reader_contract",
        );
        let character = json!({"operation":"create_character","input":{"name":"Ada","summary":"Seeker","role":"lead","voice_profile":{},"emotional_profile":{"flex_range":{}},"initial_state":{}}});
        for path in [
            "/input/voice_profile",
            "/input/emotional_profile",
            "/input/emotional_profile/flex_range",
            "/input/initial_state",
        ] {
            rejects_at(character.clone(), path);
        }
        let arc = json!({"operation":"create_character_arc","input":{"character_id":{"kind":"proposal_ref","value":"cast:ada"},"arc_type":"growth","starting_state":"Alone","ending_state":"Trusting","milestones":[{"label":"Ask","description":"Seek help","placement":{"book_number":1,"chapter_number":2}}],"thematic_purpose":"Trust"}});
        for path in ["/input/milestones/0", "/input/milestones/0/placement"] {
            rejects_at(arc.clone(), path);
        }
        let conflict = json!({"operation":"create_conflict","input":{"name":"Heir","conflict_type":"mystery","stakes":"Home","try_fail_cycles":[{"attempt_order":1,"label":"Ask","outcome":"Refused"}],"stated_consequences":[{"description":"Exposure","delivered":false,"stated_at":{"book_number":1,"chapter_number":1}}]}});
        for path in [
            "/input/try_fail_cycles/0",
            "/input/stated_consequences/0",
            "/input/stated_consequences/0/stated_at",
        ] {
            rejects_at(conflict.clone(), path);
        }
    }
}

/// A committed operation receipt. IDs include the operation's named output and
/// root aliases `<output>:book`, `<output>:chapter`, `<output>:branch`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanningOperationReceipt {
    pub proposal_id: String,
    pub operation_index: usize,
    pub input_hash: String,
    pub returned_ids: BTreeMap<String, String>,
    pub actor: String,
    pub decision_note: String,
    pub result: Option<PlanningAppliedOutput>,
    pub prior_state: serde_json::Value,
    pub outcome: PlanningOperationOutcome,
    pub error: Option<String>,
    pub recorded_at: i64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanningOperationOutcome {
    Applied,
    Failed,
    NotReached,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanningApplyReport {
    pub attempt_id: String,
    /// Completion of this application batch, never a drafting-readiness verdict.
    pub complete: bool,
    pub proposals: Vec<PlanningProposal>,
    pub operations: Vec<PlanningOperationReceipt>,
    pub error: Option<String>,
    pub pending_search_refreshes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "operation", content = "output", rename_all = "snake_case")]
pub enum PlanningAppliedOutput {
    CreateProject(Box<CreateProjectOutput>),
    CreateCharacter(Box<CreateCharacterOutput>),
    CreateLocation(Box<CreateLocationOutput>),
    CreateWorldRule(Box<CreateWorldRuleOutput>),
    CreateRelationship(Box<CreateRelationshipOutput>),
    CreatePlotLine(Box<CreatePlotLineOutput>),
    CreateConflict(Box<CreateConflictOutput>),
    CreateTheme(Box<CreateThemeOutput>),
    CreateMotif(Box<CreateMotifOutput>),
    CreateNarrativePromise(Box<CreateNarrativePromiseOutput>),
    CreateCharacterArc(Box<CreateCharacterArcOutput>),
    CreatePacingConfig(Box<CreatePacingConfigOutput>),
    CreatePacingCurve(Box<CreatePacingCurveOutput>),
    SetArcPacingConstraints(Box<SetArcPacingConstraintsOutput>),
    CreateBook(Box<CreateBookOutput>),
    CreateChapter(Box<CreateChapterOutput>),
    SetBookOutline(Box<SetBookOutlineOutput>),
    SetChapterOutline(Box<SetChapterOutlineOutput>),
    PlanChapter(Box<PlanChapterOutput>),
    UpdateReaderContract(Box<UpdateEntityOutput>),
}
