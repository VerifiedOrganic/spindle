//! Initial-planning contracts. These never authorize in-run amendments.
use crate::models::*;
use schemars::JsonSchema;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

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

impl PlanningLevel {
    pub const fn all() -> [Self; 6] {
        [
            Self::Contract,
            Self::CastWorld,
            Self::SeriesArc,
            Self::Book,
            Self::Chapter,
            Self::Scene,
        ]
    }

    pub const fn route_task(self) -> &'static str {
        match self {
            Self::Contract | Self::CastWorld | Self::SeriesArc => "plan",
            Self::Book | Self::Chapter | Self::Scene => "outline",
        }
    }

    pub const fn prompt_id(self) -> PlanningPromptId {
        match self {
            Self::Contract => PlanningPromptId::PlanningContractV1,
            Self::CastWorld => PlanningPromptId::PlanningSeedV1,
            Self::SeriesArc => PlanningPromptId::PlanningSeriesArcV1,
            Self::Book => PlanningPromptId::OutlineBookV1,
            Self::Chapter => PlanningPromptId::OutlineChaptersV1,
            Self::Scene => PlanningPromptId::OutlineManifestV1,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningPromptId {
    #[serde(rename = "planning_contract_v1")]
    PlanningContractV1,
    #[serde(rename = "planning_seed_v1")]
    PlanningSeedV1,
    #[serde(rename = "planning_series_arc_v1")]
    PlanningSeriesArcV1,
    #[serde(rename = "outline_book_v1")]
    OutlineBookV1,
    #[serde(rename = "outline_chapters_v1")]
    OutlineChaptersV1,
    #[serde(rename = "outline_manifest_v1")]
    OutlineManifestV1,
    #[serde(rename = "planning_repair_v1")]
    PlanningRepairV1,
}

impl PlanningPromptId {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlanningContractV1 => "planning_contract_v1",
            Self::PlanningSeedV1 => "planning_seed_v1",
            Self::PlanningSeriesArcV1 => "planning_series_arc_v1",
            Self::OutlineBookV1 => "outline_book_v1",
            Self::OutlineChaptersV1 => "outline_chapters_v1",
            Self::OutlineManifestV1 => "outline_manifest_v1",
            Self::PlanningRepairV1 => "planning_repair_v1",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanningTargetRange {
    /// Book number the detailed plan covers (1-based).
    pub book_number: i32,
    /// First chapter in the detailed planning horizon (1-based).
    pub chapter_start: i32,
    /// Last chapter in the detailed planning horizon, inclusive.
    pub chapter_end: i32,
    /// Planned scene count for each chapter in the horizon.
    pub scenes_per_chapter: i32,
}

impl Default for PlanningTargetRange {
    fn default() -> Self {
        Self {
            book_number: 1,
            chapter_start: 1,
            chapter_end: 3,
            scenes_per_chapter: 2,
        }
    }
}

impl PlanningTargetRange {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlannedPromiseAction {
    Plant,
    Advance,
    Payoff,
    Defer,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlannedPromiseEvent {
    pub promise_ref: PlanningRef,
    pub action: PlannedPromiseAction,
    pub placement: PlanningStoryPlacement,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlannedPov {
    pub book_number: i32,
    pub chapter_number: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_order: Option<i32>,
    pub pov_character_id: PlanningRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlannedIntensity {
    pub book_number: i32,
    pub chapter_number: i32,
    pub target_intensity: f64,
    #[serde(default)]
    pub recovery_beat: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningSchedule {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promise_events: Vec<PlannedPromiseEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pov_plan: Vec<PlannedPov>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub intensity_targets: Vec<PlannedIntensity>,
}

impl PlanningSchedule {
    pub fn is_empty(&self) -> bool {
        self.promise_events.is_empty()
            && self.pov_plan.is_empty()
            && self.intensity_targets.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanningAuthorQuestion {
    /// Stable identifier for this author question.
    pub id: String,
    /// The creative decision the author needs to make.
    pub question: String,
    /// Optional set of choices the author may select.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Optional recommended choice for the author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recommended: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningArtifactV1 {
    pub schema_version: u32,
    pub level: PlanningLevel,
    #[serde(default)]
    pub target_range: PlanningTargetRange,
    pub rationale: String,
    #[serde(default)]
    pub assumptions: Vec<String>,
    #[serde(default)]
    pub author_questions: Vec<PlanningAuthorQuestion>,
    pub operations: Vec<PlanningOperationItem>,
    #[serde(default)]
    pub schedule: PlanningSchedule,
}

impl PlanningArtifactV1 {
    pub fn canonical_json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(PlanningArtifactV1))
            .expect("PlanningArtifactV1 schema serializes")
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningFindingSeverity {
    Blocking,
    Advisory,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanningFinding {
    /// Stable machine-readable finding code.
    pub code: String,
    /// Whether this finding blocks acceptance or is advisory only.
    pub severity: PlanningFindingSeverity,
    /// JSON pointer or schema path the finding concerns.
    pub path: String,
    /// Related proposal references or record IDs.
    #[serde(default)]
    pub related_refs: Vec<String>,
    /// What the validator expected at this path.
    pub expected: String,
    /// What the staged artifact actually contained.
    pub actual: String,
    /// Short instruction for repairing the artifact.
    pub repair_hint: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningCostStatus {
    EstimatedFromConfiguredPrices,
    #[default]
    UnknownUnpriced,
    UnknownMissingProviderUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningBudget {
    /// Maximum number of route calls across the whole session.
    pub max_model_calls: u32,
    /// Maximum input tokens the session may spend.
    pub max_input_tokens: u64,
    /// Maximum output tokens the session may spend.
    pub max_output_tokens: u64,
    /// Maximum combined input plus output tokens.
    pub max_total_tokens: u64,
    /// Maximum output tokens requested from a single route call.
    pub max_output_tokens_per_call: u32,
    /// Optional hard cost ceiling in US dollars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    /// Optional configured input price in US dollars per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_cost_per_million_usd: Option<f64>,
    /// Optional configured output price in US dollars per million tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_cost_per_million_usd: Option<f64>,
}

impl Default for PlanningBudget {
    fn default() -> Self {
        Self {
            max_model_calls: 12,
            max_input_tokens: 48_000,
            max_output_tokens: 24_000,
            max_total_tokens: 72_000,
            max_output_tokens_per_call: 2_000,
            max_cost_usd: None,
            input_cost_per_million_usd: None,
            output_cost_per_million_usd: None,
        }
    }
}

impl PlanningBudget {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningBudgetUsage {
    /// Route calls already dispatched for this session.
    pub model_calls: u32,
    /// Input tokens reported or estimated so far.
    pub input_tokens: u64,
    /// Output tokens reported or estimated so far.
    pub output_tokens: u64,
    /// Combined input plus output tokens.
    pub total_tokens: u64,
    /// Estimated cost when configured prices are available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    /// Conservative cost reserved for the maximum output of pending calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_cost_usd: Option<f64>,
    /// How the cost figure was derived, or why it is unknown.
    #[serde(default)]
    pub cost_status: PlanningCostStatus,
}

impl PlanningBudgetUsage {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningOutputSource {
    Model,
    Host,
    Fixture,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningModelCallReceipt {
    pub attempt_number: u32,
    pub route_task: String,
    pub resolved_route: String,
    pub agent_name: String,
    pub adapter_kind: String,
    pub model: String,
    pub prompt_id: PlanningPromptId,
    pub prompt_hash: String,
    pub schema_hash: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub provider_reported_usage: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_cost_usd: Option<f64>,
    pub cost_status: PlanningCostStatus,
    pub latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningGenerationReceipt {
    pub source: PlanningOutputSource,
    pub prompt_id: PlanningPromptId,
    pub prompt_hash: String,
    pub schema_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_route: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub attempts_used: u32,
    pub repaired: bool,
    #[serde(default)]
    pub calls: Vec<PlanningModelCallReceipt>,
    pub recorded_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningSessionOutcome {
    Staged,
    NeedsAuthor,
    NeedsRoute,
    ModelOutputRejected,
    BudgetExhausted,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanningRouteProblem {
    MissingRoute {
        route_task: String,
        allow_review_backend: bool,
    },
    LocalStubRejected {
        route_task: String,
        resolved_route: String,
        agent_name: String,
        adapter_kind: String,
        builtin_default: bool,
    },
    MissingApiKey {
        route_task: String,
        resolved_route: String,
        agent_name: String,
        env_var: String,
    },
    RatingNotCovered {
        route_task: String,
        resolved_route: String,
        agent_name: String,
        rating_ceiling: ContentRating,
    },
    DispatchFailed {
        route_task: String,
        resolved_route: String,
        agent_name: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningHostRequest {
    pub session_id: String,
    pub level: PlanningLevel,
    pub route_task: String,
    pub prompt_id: PlanningPromptId,
    pub prompt_hash: String,
    pub schema_hash: String,
    pub system_prompt: String,
    pub user_prompt: String,
    pub artifact_schema: serde_json::Value,
    pub parent_proposals: BTreeMap<String, String>,
    pub target_range: PlanningTargetRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_problem: Option<PlanningRouteProblem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prior_findings: Vec<PlanningFinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningHostSubmission {
    pub source: PlanningOutputSource,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AdvancePlanningInput {
    pub session_id: String,
    pub scope: Option<PlanningScope>,
    pub level: Option<PlanningLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parents: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<PlanningTargetRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_review_backend: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_ceiling: Option<ContentRating>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<PlanningBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_submission: Option<PlanningHostSubmission>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningAdvanceReport {
    pub attempt_id: String,
    pub session_id: String,
    pub level: PlanningLevel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_key: Option<String>,
    pub outcome: PlanningSessionOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<PlanningProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_receipt: Option<PlanningGenerationReceipt>,
    #[serde(default)]
    pub findings: Vec<PlanningFinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author_questions: Vec<PlanningAuthorQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_request: Option<PlanningHostRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_problem: Option<PlanningRouteProblem>,
    pub usage: PlanningBudgetUsage,
    pub budget: PlanningBudget,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningGate {
    #[default]
    Human,
    DelegateIfValid,
    Hold,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningPovPolicy {
    #[serde(default)]
    pub single_pov: Option<PlanningRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_roster: Vec<PlanningRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub desired_shares: BTreeMap<String, f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_tolerance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_absence_chapters: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_consecutive_chapters: Option<i32>,
    #[serde(default)]
    pub hard_constraint: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanningPacingPolicy {
    #[serde(default)]
    pub hard_constraint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiver_reason: Option<String>,
}

/// An empty map means human gates at every level. Root levels remain human-gated;
/// Chapter and Scene levels may opt into `delegate_if_valid` or `hold` once upstream
/// roots are accepted and a human explicitly ratifies the policy version.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningPolicy {
    #[serde(default)]
    pub gates: BTreeMap<PlanningLevel, PlanningGate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_advisory_findings: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pov_policy: Option<PlanningPovPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pacing_policy: Option<PlanningPacingPolicy>,
}

impl PlanningPolicy {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn delegated_actor(&self) -> Option<String> {
        let policy_id = self.policy_id.as_deref()?.trim();
        let version = self.policy_version?;
        if policy_id.is_empty() || version == 0 {
            return None;
        }
        Some(format!("policy:{policy_id}:{version}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPlanningPolicyInput {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PlanningScope>,
    pub policy: PlanningPolicy,
    pub actor: String,
    pub note: String,
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
    #[serde(default, skip_serializing_if = "PlanningTargetRange::is_default")]
    pub horizon: PlanningTargetRange,
    #[serde(default, skip_serializing_if = "PlanningBudget::is_default")]
    pub budget: PlanningBudget,
    #[serde(default, skip_serializing_if = "PlanningBudgetUsage::is_default")]
    pub usage: PlanningBudgetUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_ceiling: Option<ContentRating>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_review_backend: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub projection_stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<PlanningSessionOutcome>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningPacingMetrics {
    /// Number of scene intensity targets evaluated.
    pub evaluated_items: usize,
    /// Mean absolute deviation from the accepted pacing curve.
    pub mean_absolute_deviation: f64,
    /// Longest run of scenes that stayed outside the advisory band.
    pub longest_sustained_deviation: usize,
    /// Chapter count for each POV character reference.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pov_chapter_counts: BTreeMap<String, usize>,
    /// Share of chapters for each POV character reference.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pov_shares: BTreeMap<String, f64>,
    /// Longest consecutive chapter run for each POV character.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pov_max_consecutive_chapters: BTreeMap<String, i32>,
    /// Longest chapter absence for each POV character.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pov_max_absence_chapters: BTreeMap<String, i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningReadinessInput {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PlanningScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<PlanningTargetRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mining_policy: Option<AuthoringMiningPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_policy: Option<AuthoringCheckpointPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningReadinessReport {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PlanningScope>,
    pub target_range: PlanningTargetRange,
    pub ready_to_draft: bool,
    pub planner_checks_passed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stale_projection: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<PlanningFinding>,
    pub pacing_metrics: PlanningPacingMetrics,
    pub prepare_run: AuthoringPrepareRunOutput,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<PlanningTargetRange>,
    #[serde(default, skip_serializing_if = "PlanningSchedule::is_empty")]
    pub schedule: PlanningSchedule,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assumptions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub author_questions: Vec<PlanningAuthorQuestion>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_receipt: Option<PlanningGenerationReceipt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<PlanningFinding>,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanningStoryPlacement {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: Option<i32>,
    pub note: Option<String>,
}

impl PlanningStoryPlacement {
    pub fn has_positive_coordinates(&self) -> bool {
        self.book_number >= 1
            && self.chapter_number >= 1
            && self.scene_order.is_none_or(|scene| scene >= 1)
    }

    /// Compares two story placements lexicographically by `(book_number, chapter_number, scene_order)`.
    /// When both placements are in the same `(book_number, chapter_number)` and one has `scene_order: None`
    /// (chapter-wide), they compare equal for window boundary checks unless strict scene ordering is requested.
    pub fn cmp_story_order(&self, other: &Self) -> std::cmp::Ordering {
        match (
            self.book_number.cmp(&other.book_number),
            self.chapter_number.cmp(&other.chapter_number),
        ) {
            (std::cmp::Ordering::Equal, std::cmp::Ordering::Equal) => {
                match (self.scene_order, other.scene_order) {
                    (Some(left), Some(right)) => left.cmp(&right),
                    _ => std::cmp::Ordering::Equal,
                }
            }
            (std::cmp::Ordering::Equal, chapter_ord) => chapter_ord,
            (book_ord, _) => book_ord,
        }
    }
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

/// Session settings that can be changed without generating a planning level.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigurePlanningSessionInput {
    /// Planning session to update.
    pub session_id: String,
    /// Project and branch the session is bound to; omit before the project root is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PlanningScope>,
    /// Detailed chapter horizon to store on the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<PlanningTargetRange>,
    /// Token and call ceiling to store on the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<PlanningBudget>,
    /// Highest content rating the stored plan may request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_ceiling: Option<ContentRating>,
    /// Whether a configured review agent may be used as a planning backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_review_backend: Option<bool>,
}

/// Gate selected for every planning level. Root levels cannot be delegated.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlanningGatePolicy {
    /// Gate for the Contract level; this root requires human review.
    #[serde(default)]
    pub contract: PlanningGate,
    /// Gate for the CastWorld level; this root requires human review.
    #[serde(default)]
    pub cast_world: PlanningGate,
    /// Gate for the SeriesArc level; this root requires human review.
    #[serde(default)]
    pub series_arc: PlanningGate,
    /// Gate for the Book level; this root requires human review.
    #[serde(default)]
    pub book: PlanningGate,
    /// Gate for Chapter-level proposals.
    #[serde(default)]
    pub chapter: PlanningGate,
    /// Gate for Scene-level proposals.
    #[serde(default)]
    pub scene: PlanningGate,
}

impl PlanningGatePolicy {
    /// Build a full gate policy from a sparse level map.
    pub fn from_gates(gates: &BTreeMap<PlanningLevel, PlanningGate>) -> Self {
        let gate = |level| gates.get(&level).copied().unwrap_or_default();
        Self {
            contract: gate(PlanningLevel::Contract),
            cast_world: gate(PlanningLevel::CastWorld),
            series_arc: gate(PlanningLevel::SeriesArc),
            book: gate(PlanningLevel::Book),
            chapter: gate(PlanningLevel::Chapter),
            scene: gate(PlanningLevel::Scene),
        }
    }

    /// Materialize the complete level map stored on the planning session.
    pub fn to_gates(self) -> BTreeMap<PlanningLevel, PlanningGate> {
        BTreeMap::from([
            (PlanningLevel::Contract, self.contract),
            (PlanningLevel::CastWorld, self.cast_world),
            (PlanningLevel::SeriesArc, self.series_arc),
            (PlanningLevel::Book, self.book),
            (PlanningLevel::Chapter, self.chapter),
            (PlanningLevel::Scene, self.scene),
        ])
    }

    /// Gate configured for one level.
    pub fn gate_for(self, level: PlanningLevel) -> PlanningGate {
        match level {
            PlanningLevel::Contract => self.contract,
            PlanningLevel::CastWorld => self.cast_world,
            PlanningLevel::SeriesArc => self.series_arc,
            PlanningLevel::Book => self.book,
            PlanningLevel::Chapter => self.chapter,
            PlanningLevel::Scene => self.scene,
        }
    }
}

/// Explicit gate and policy settings for a planning session. Every level stays
/// human-gated unless the author names a delegated gate here.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningPolicyOverride {
    /// Gate to apply at each planning level; omitted levels keep human review.
    #[serde(default)]
    pub gates: PlanningGatePolicy,
    /// Stable identifier for an explicitly delegated policy version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    /// Monotonic policy version recorded with delegated decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<u32>,
    /// Maximum advisory findings allowed for delegated chapter or scene acceptance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_advisory_findings: Option<usize>,
}

/// Start a new planning session or resume an existing one.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningBeginInput {
    /// One- to three-sentence premise or intent for a new planning session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub premise: Option<String>,
    /// Existing planning session to resume instead of starting a new one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Existing project to bind; omit for a brand-new book.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Branch to bind; defaults to the project's active branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// Detailed chapter horizon to generate; defaults to three chapters with two scenes each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizon: Option<PlanningTargetRange>,
    /// Token and call ceiling for route generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<PlanningBudget>,
    /// Highest content rating the plan may request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_ceiling: Option<ContentRating>,
    /// Allow a configured review agent as a planning backend.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_review_backend: Option<bool>,
    /// Gate policy override; omission keeps every level human-gated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PlanningPolicyOverride>,
    /// Human author recorded as ratifying the policy override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_actor: Option<String>,
    /// Note explaining why the policy override is authorized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_note: Option<String>,
}

/// Compact audit summary of a planning policy.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlanningPolicySummary {
    /// Gate configured for each planning level; the default is human review.
    #[serde(default)]
    pub gates: PlanningGatePolicy,
    /// Identifier of the explicitly delegated policy version, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_id: Option<String>,
    /// Version of the explicitly delegated policy, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_version: Option<u32>,
    /// Maximum advisory findings allowed for delegation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_advisory_findings: Option<usize>,
    /// Whether the session carries an explicit POV policy.
    #[serde(default)]
    pub has_pov_policy: bool,
    /// Whether the session carries an explicit pacing policy.
    #[serde(default)]
    pub has_pacing_policy: bool,
}

/// Compact planning session state suitable for reconnect and review.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningSessionSummary {
    /// Stable planning session identifier.
    pub session_id: String,
    /// Short preview of the author's premise.
    pub premise_preview: String,
    /// Hash of the exact premise intent used by staging.
    pub intent_hash: String,
    /// Bound project identifier, once a project root is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Bound branch identifier, once a project root is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// First planning level that has no staged or accepted proposal.
    pub next_level: PlanningLevel,
    /// Current gate and policy summary.
    pub policy: PlanningPolicySummary,
    /// Detailed chapter horizon stored on the session.
    pub horizon: PlanningTargetRange,
    /// Token and call ceiling stored on the session.
    pub budget: PlanningBudget,
    /// Usage recorded so far.
    pub usage: PlanningBudgetUsage,
    /// Highest content rating the plan may request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_ceiling: Option<ContentRating>,
    /// Whether a configured review agent may be used as a planning backend.
    #[serde(default)]
    pub allow_review_backend: bool,
    /// Whether an accepted amendment made the planning projection stale.
    #[serde(default)]
    pub projection_stale: bool,
    /// Explanation recorded when the projection became stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    /// Outcome of the most recent generation attempt, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<PlanningSessionOutcome>,
    /// Session creation time in microseconds since the Unix epoch.
    pub created_at: i64,
}

/// One typed operation inside a staged proposal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningOperationSummary {
    /// Proposal-local output reference used by dependent operations.
    pub output_ref: String,
    /// Typed operation payload exactly as staged.
    pub operation: serde_json::Value,
}

/// Review summary of one staged or decided proposal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningProposalSummary {
    /// Stable proposal identifier.
    pub proposal_id: String,
    /// Planning level this proposal covers.
    pub level: PlanningLevel,
    /// Current proposal status.
    pub status: PlanningProposalStatus,
    /// Exact hash required to record a decision for this proposal.
    pub payload_hash: String,
    /// Creative rationale supplied with the artifact.
    pub rationale: String,
    /// Typed operations included in the proposal.
    #[serde(default)]
    pub operations: Vec<PlanningOperationSummary>,
    /// Promise, POV and intensity schedule supplied with the artifact.
    #[serde(default)]
    pub schedule: serde_json::Value,
    /// Assumptions the plan relies on.
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// Questions the author still needs to answer.
    #[serde(default)]
    pub author_questions: Vec<PlanningAuthorQuestion>,
    /// Validation findings recorded at staging or decision time.
    #[serde(default)]
    pub findings: Vec<PlanningFinding>,
    /// Actor who recorded the decision, when decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_by: Option<String>,
    /// Note recorded with the decision, when decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_note: Option<String>,
    /// Proposal creation time in microseconds since the Unix epoch.
    pub created_at: i64,
}

/// Result of starting or resuming a planning session.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningBeginOutput {
    /// Compact session state after the call.
    pub session: PlanningSessionSummary,
    /// Whether an existing session was resumed instead of created.
    pub resumed: bool,
    /// The next safe action for the host or author.
    pub next_action: String,
    /// Short human-readable status message.
    pub message: String,
}

/// A fieldless planning action that is published as a JSON Schema string enum
/// and accepts the two wire forms clients already send.
trait PlanningAction: Sized + Copy + 'static {
    const WIRE: &'static [(&'static str, Self)];

    fn from_wire(value: &str) -> Option<Self> {
        Self::WIRE
            .iter()
            .find_map(|(name, action)| (*name == value).then_some(*action))
    }

    fn wire_names() -> String {
        Self::WIRE
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn deserialize_planning_action<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: PlanningAction,
{
    struct ActionVisitor<T>(PhantomData<T>);

    impl<'de, T: PlanningAction> Visitor<'de> for ActionVisitor<T> {
        type Value = T;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "a planning action string (one of: {}) or a legacy object with one variant key mapped to null",
                T::wire_names()
            )
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            T::from_wire(value).ok_or_else(|| {
                E::custom(format!(
                    "unknown planning action '{value}'; expected one of: {}",
                    T::wire_names()
                ))
            })
        }

        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: MapAccess<'de>,
        {
            let Some((key, value)) = map.next_entry::<String, Value>()? else {
                return Err(de::Error::custom(format!(
                    "planning action object must contain exactly one variant key mapped to null; expected one of: {}",
                    T::wire_names()
                )));
            };
            if map.next_key::<String>()?.is_some() {
                return Err(de::Error::custom(format!(
                    "planning action object must contain exactly one variant key; expected one of: {}",
                    T::wire_names()
                )));
            }
            if !value.is_null() {
                return Err(de::Error::custom(format!(
                    "legacy planning action object must map '{key}' to null, not a value; expected one of: {}",
                    T::wire_names()
                )));
            }
            T::from_wire(&key).ok_or_else(|| {
                de::Error::custom(format!(
                    "unknown planning action '{key}'; expected one of: {}",
                    T::wire_names()
                ))
            })
        }
    }

    deserializer.deserialize_any(ActionVisitor(PhantomData))
}

/// One planning action a host may request without silent drafting.
#[derive(Debug, Clone, Copy, Default, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningAdvanceAction {
    /// Generate the next or requested planning level through the configured route.
    #[default]
    Generate,
    /// Stage a host-authored structured artifact when no route is available.
    StageHostArtifact,
    /// Retry the most recent planning level after a rejected or malformed attempt.
    RepairLastProposal,
    /// Recompute planner checks and the authoring prepare report without generating.
    RecomputeReadiness,
    /// Update the session gate policy without generating.
    SetPolicy,
}

impl PlanningAction for PlanningAdvanceAction {
    const WIRE: &'static [(&'static str, Self)] = &[
        ("generate", Self::Generate),
        ("stage_host_artifact", Self::StageHostArtifact),
        ("repair_last_proposal", Self::RepairLastProposal),
        ("recompute_readiness", Self::RecomputeReadiness),
        ("set_policy", Self::SetPolicy),
    ];
}

impl<'de> Deserialize<'de> for PlanningAdvanceAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_planning_action(deserializer)
    }
}

/// Generate, submit, retry, recheck or repolicy one planning level.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningAdvanceInput {
    /// Planning session to advance.
    pub session_id: String,
    /// Project the session is bound to; omit to use the stored scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Branch the session is bound to; defaults to the project's active branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// Action to perform; defaults to generating the next level.
    #[serde(default)]
    pub action: PlanningAdvanceAction,
    /// Planning level to generate; defaults to the first unfilled level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<PlanningLevel>,
    /// Exact accepted or staged parent proposal hashes; defaults to active ancestors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parents: Option<BTreeMap<String, String>>,
    /// Detailed chapter horizon for this generation call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<PlanningTargetRange>,
    /// Allow a configured review agent as a planning backend for this call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_review_backend: Option<bool>,
    /// Highest content rating the generated plan may request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_ceiling: Option<ContentRating>,
    /// Token and call ceiling for this call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<PlanningBudget>,
    /// Host-authored artifact payload used by the stage_host_artifact action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_artifact: Option<serde_json::Value>,
    /// Caller-supplied idempotency key that prevents duplicate generation attempts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_key: Option<String>,
    /// Gate policy override used by the set_policy action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PlanningPolicyOverride>,
    /// Human author recorded as ratifying a policy change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_actor: Option<String>,
    /// Note explaining why a policy change is authorized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_note: Option<String>,
    /// Canon-mining policy applied when checking authoring readiness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mining_policy: Option<AuthoringMiningPolicy>,
    /// Checkpoint policy applied when checking authoring readiness.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_policy: Option<AuthoringCheckpointPolicy>,
}

/// Host generation request returned when no planning route can run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningHostRequestSummary {
    /// Planning level the host must generate.
    pub level: PlanningLevel,
    /// Logical route task the host is replacing.
    pub route_task: String,
    /// Versioned prompt identifier the host must use.
    pub prompt_id: PlanningPromptId,
    /// Hash of the exact prompt text.
    pub prompt_hash: String,
    /// Hash of the exact artifact schema.
    pub schema_hash: String,
    /// System instructions for the host model.
    pub system_prompt: String,
    /// User prompt containing intent, context and constraints.
    pub user_prompt: String,
    /// JSON Schema the host artifact must satisfy.
    pub artifact_schema: serde_json::Value,
    /// Exact parent proposal hashes the artifact must reference.
    #[serde(default)]
    pub parent_proposals: BTreeMap<String, String>,
    /// Detailed chapter horizon the artifact must cover.
    pub target_range: PlanningTargetRange,
    /// Short explanation of why the configured route could not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_problem: Option<String>,
    /// Findings from an earlier rejected attempt that must be repaired.
    #[serde(default)]
    pub prior_findings: Vec<PlanningFinding>,
}

/// Compact outcome of one generation or submission attempt.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningAdvanceSummary {
    /// Durable attempt identifier for retry and audit.
    pub attempt_id: String,
    /// Planning level attempted.
    pub level: PlanningLevel,
    /// Honest session outcome for the attempt.
    pub outcome: PlanningSessionOutcome,
    /// Staged proposal produced by the attempt, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<PlanningProposalSummary>,
    /// Where the artifact came from, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PlanningOutputSource>,
    /// Number of route attempts consumed, including repair.
    #[serde(default)]
    pub attempts_used: u32,
    /// Whether the single bounded repair pass was used.
    #[serde(default)]
    pub repaired: bool,
    /// Validation findings from the attempt.
    #[serde(default)]
    pub findings: Vec<PlanningFinding>,
    /// Questions the author must answer before acceptance.
    #[serde(default)]
    pub author_questions: Vec<PlanningAuthorQuestion>,
    /// Host generation request used when no route could run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_request: Option<PlanningHostRequestSummary>,
    /// Usage recorded by the attempt.
    pub usage: PlanningBudgetUsage,
    /// Ceiling in force for the attempt.
    pub budget: PlanningBudget,
    /// Optional human-readable status message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Result of advancing, rechecking or repolicing a planning session.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningAdvanceOutput {
    /// Compact session state after the call.
    pub session: PlanningSessionSummary,
    /// Action performed by the call.
    pub action: PlanningAdvanceAction,
    /// Generation result, when the action generated or submitted an artifact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advance: Option<PlanningAdvanceSummary>,
    /// Readiness result, when the action recomputed or checked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<PlanningReadinessSummary>,
    /// The next safe action for the host or author.
    pub next_action: String,
}

/// One chapter in the authoring prepare report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningPrepareChapterSummary {
    /// Chapter number checked.
    pub chapter_number: i32,
    /// Whether this chapter is ready for drafting.
    pub ready: bool,
    /// Items this chapter still needs.
    #[serde(default)]
    pub missing_items: Vec<String>,
}

/// Compact authoring prepare report for the planned range.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningPrepareRunSummary {
    /// Project whose chapters were checked.
    pub project_id: String,
    /// Book number checked.
    pub book_number: i32,
    /// First chapter checked.
    pub start_chapter: i32,
    /// Last chapter checked, inclusive.
    pub end_chapter: i32,
    /// Whether the whole range is ready for drafting.
    pub ready_to_draft: bool,
    /// Items still required before drafting can start.
    #[serde(default)]
    pub missing_requirements: Vec<String>,
    /// Per-chapter readiness.
    #[serde(default)]
    pub chapters: Vec<PlanningPrepareChapterSummary>,
}

/// Planner checks plus the real authoring prepare result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningReadinessSummary {
    /// Chapter range checked.
    pub target_range: PlanningTargetRange,
    /// Whether the accepted plan can start drafting.
    pub ready_to_draft: bool,
    /// Whether the deterministic planner checks passed.
    pub planner_checks_passed: bool,
    /// Whether a stale projection blocks readiness.
    pub stale_projection: bool,
    /// Blocking and advisory planner findings.
    #[serde(default)]
    pub findings: Vec<PlanningFinding>,
    /// POV and pacing measurements for the checked range.
    pub pacing_metrics: PlanningPacingMetrics,
    /// Real authoring prepare report for the same range.
    pub prepare_run: PlanningPrepareRunSummary,
}

/// Read compact planning status for one session or for reconnect discovery.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningStatusInput {
    /// Existing planning session to inspect; omit to discover active sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Existing project whose sessions should be listed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Branch whose sessions should be listed; defaults to the project's active branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// Chapter range to check instead of the session's stored horizon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<PlanningTargetRange>,
    /// Whether to include planner checks and the authoring prepare report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_readiness: Option<bool>,
}

/// Compact planning status, staged proposals and reconnect guidance.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningStatusOutput {
    /// Selected session detail, when a session was found or selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<PlanningSessionSummary>,
    /// Sessions matching the requested project, or recent sessions when no project is given.
    #[serde(default)]
    pub sessions: Vec<PlanningSessionSummary>,
    /// Effective gate for the selected session's next level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<PlanningGate>,
    /// Proposals staged or decided in the selected session.
    #[serde(default)]
    pub proposals: Vec<PlanningProposalSummary>,
    /// Planner findings currently visible for the selected session.
    #[serde(default)]
    pub findings: Vec<PlanningFinding>,
    /// Readiness report for the selected session, when its project is attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<PlanningReadinessSummary>,
    /// The next safe action for the host or author.
    pub next_action: String,
    /// How to reconnect to this work in a later session.
    pub reconnect_hint: String,
}

/// One decision action for staged planning proposals.
#[derive(Debug, Clone, Copy, Default, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanningDecideAction {
    /// Record an accept, reject or supersede decision for staged proposals.
    #[default]
    Decide,
    /// Supersede staged proposals so a replacement can be staged at the same level.
    Revise,
    /// Check readiness and prepare the authoring run without changing proposals.
    PrepareAuthoringRun,
}

impl PlanningAction for PlanningDecideAction {
    const WIRE: &'static [(&'static str, Self)] = &[
        ("decide", Self::Decide),
        ("revise", Self::Revise),
        ("prepare_authoring_run", Self::PrepareAuthoringRun),
    ];
}

impl<'de> Deserialize<'de> for PlanningDecideAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_planning_action(deserializer)
    }
}

/// Decide staged planning proposals or prepare the authoring run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningDecideInput {
    /// Planning session whose proposals are being decided.
    pub session_id: String,
    /// Project the session is bound to; omit to use the stored scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Branch the session is bound to; defaults to the project's active branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// Whether to decide proposals or prepare the authoring run.
    #[serde(default)]
    pub action: PlanningDecideAction,
    /// Decision to record when the action is decide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<PlanningDecision>,
    /// Exact proposal IDs and payload hashes the decision covers.
    #[serde(default)]
    pub expected_hashes: BTreeMap<String, String>,
    /// Human author recorded as making the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// Note explaining the decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Chapter range to prepare instead of the session's stored horizon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<PlanningTargetRange>,
    /// Canon-mining policy applied when preparing the authoring run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mining_policy: Option<AuthoringMiningPolicy>,
    /// Checkpoint policy applied when preparing the authoring run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_policy: Option<AuthoringCheckpointPolicy>,
}

/// Result of a planning decision or authoring preparation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningDecideOutput {
    /// Compact session state after the call.
    pub session: PlanningSessionSummary,
    /// Action performed by the call.
    pub action: PlanningDecideAction,
    /// Proposals affected by the decision.
    #[serde(default)]
    pub decided: Vec<PlanningProposalSummary>,
    /// Readiness report produced by the call, when checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<PlanningReadinessSummary>,
    /// The next safe action for the host or author.
    pub next_action: String,
    /// Short human-readable status message.
    pub message: String,
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
        rejects_extra::<PlanningTargetRange>(
            json!({"book_number":1,"chapter_start":1,"chapter_end":3,"scenes_per_chapter":2}),
        );
        rejects_extra::<PlannedPromiseEvent>(json!({
            "promise_ref":{"kind":"proposal_ref","value":"arc:promise"},
            "action":"plant",
            "placement":{"book_number":1,"chapter_number":1}
        }));
        rejects_extra::<PlannedPov>(json!({
            "book_number":1,
            "chapter_number":1,
            "pov_character_id":{"kind":"proposal_ref","value":"cast:ada"}
        }));
        rejects_extra::<PlannedIntensity>(
            json!({"book_number":1,"chapter_number":1,"target_intensity":0.5}),
        );
        rejects_extra::<PlanningSchedule>(json!({}));
        rejects_extra::<PlanningAuthorQuestion>(json!({"id":"q1","question":"Which tone?"}));
        rejects_extra::<PlanningArtifactV1>(json!({
            "schema_version":1,
            "level":"contract",
            "rationale":"Establish contract",
            "operations":[{"output_ref":"contract:project","operation":{"operation":"update_reader_contract","input":{"promise":"Discovery"}}}]
        }));
        rejects_extra::<PlanningFinding>(json!({
            "code":"ref_unresolved",
            "severity":"blocking",
            "path":"/operations/0",
            "expected":"character",
            "actual":"missing",
            "repair_hint":"Use a valid character reference"
        }));
        rejects_extra::<PlanningBudget>(json!({
            "max_model_calls":12,
            "max_input_tokens":48000,
            "max_output_tokens":24000,
            "max_total_tokens":72000,
            "max_output_tokens_per_call":2000
        }));
        rejects_extra::<PlanningBudgetUsage>(json!({
            "model_calls":0,
            "input_tokens":0,
            "output_tokens":0,
            "total_tokens":0
        }));
        rejects_extra::<PlanningHostSubmission>(json!({"source":"host","payload":{}}));
        rejects_extra::<PlanningPovPolicy>(json!({}));
        rejects_extra::<PlanningPacingPolicy>(json!({}));
        rejects_extra::<PlanningPolicy>(json!({"gates":{}}));
        rejects_extra::<SetPlanningPolicyInput>(json!({
            "session_id":"session-1",
            "policy":{"gates":{}},
            "actor":"author:alice",
            "note":"Accept policy"
        }));
        rejects_extra::<PlanningPacingMetrics>(json!({
            "evaluated_items":0,
            "mean_absolute_deviation":0.0,
            "longest_sustained_deviation":0
        }));
        rejects_extra::<PlanningReadinessInput>(json!({"session_id":"session-1"}));
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

    fn advance_action(value: Value) -> Result<PlanningAdvanceAction, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    fn decide_action(value: Value) -> Result<PlanningDecideAction, String> {
        serde_json::from_value(value).map_err(|error| error.to_string())
    }

    #[test]
    fn planning_actions_accept_strings_and_legacy_null_objects() {
        assert_eq!(
            advance_action(json!("stage_host_artifact")).unwrap(),
            PlanningAdvanceAction::StageHostArtifact
        );
        assert_eq!(
            advance_action(json!({"stage_host_artifact": null})).unwrap(),
            PlanningAdvanceAction::StageHostArtifact
        );
        assert_eq!(
            advance_action(json!({"repair_last_proposal": null})).unwrap(),
            PlanningAdvanceAction::RepairLastProposal
        );
        assert_eq!(
            decide_action(json!("prepare_authoring_run")).unwrap(),
            PlanningDecideAction::PrepareAuthoringRun
        );
        assert_eq!(
            decide_action(json!({"revise": null})).unwrap(),
            PlanningDecideAction::Revise
        );
    }

    #[test]
    fn planning_action_published_and_legacy_forms_share_one_path() {
        // The published schema form is a plain string. Serialization must
        // produce that form, and deserializing it must reach the same variant
        // as the legacy externally tagged unit-variant object.
        let published = serde_json::to_value(PlanningAdvanceAction::StageHostArtifact).unwrap();
        assert_eq!(published, json!("stage_host_artifact"));
        assert_eq!(
            advance_action(published).unwrap(),
            advance_action(json!({"stage_host_artifact": null})).unwrap()
        );
        assert_eq!(
            advance_action(json!("generate")).unwrap(),
            advance_action(json!({"generate": null})).unwrap()
        );

        let published = serde_json::to_value(PlanningDecideAction::PrepareAuthoringRun).unwrap();
        assert_eq!(published, json!("prepare_authoring_run"));
        assert_eq!(
            decide_action(published).unwrap(),
            decide_action(json!({"prepare_authoring_run": null})).unwrap()
        );
    }

    #[test]
    fn planning_actions_reject_other_shapes_with_actionable_errors() {
        let unknown = advance_action(json!("stage_artifact")).unwrap_err();
        assert!(
            unknown.contains("unknown planning action 'stage_artifact'"),
            "{unknown}"
        );
        assert!(unknown.contains("stage_host_artifact"), "{unknown}");

        let empty = advance_action(json!({})).unwrap_err();
        assert!(empty.contains("exactly one variant key"), "{empty}");
        assert!(empty.contains("stage_host_artifact"), "{empty}");

        let non_null = advance_action(json!({"stage_host_artifact": {}})).unwrap_err();
        assert!(
            non_null.contains("must map 'stage_host_artifact' to null"),
            "{non_null}"
        );

        let multiple = decide_action(json!({"decide": null, "revise": null})).unwrap_err();
        assert!(multiple.contains("exactly one variant key"), "{multiple}");

        let wrong_type = decide_action(json!(7)).unwrap_err();
        assert!(wrong_type.contains("expected"), "{wrong_type}");
    }

    #[test]
    fn planning_action_inputs_accept_both_wire_forms() {
        let string_form: PlanningAdvanceInput = serde_json::from_value(json!({
            "session_id": "session-1",
            "action": "stage_host_artifact",
            "host_artifact": {}
        }))
        .unwrap();
        assert_eq!(string_form.action, PlanningAdvanceAction::StageHostArtifact);

        let legacy_form: PlanningAdvanceInput = serde_json::from_value(json!({
            "session_id": "session-1",
            "action": {"stage_host_artifact": null},
            "host_artifact": {}
        }))
        .unwrap();
        assert_eq!(legacy_form.action, PlanningAdvanceAction::StageHostArtifact);

        let decide: PlanningDecideInput = serde_json::from_value(json!({
            "session_id": "session-1",
            "action": {"prepare_authoring_run": null}
        }))
        .unwrap();
        assert_eq!(decide.action, PlanningDecideAction::PrepareAuthoringRun);
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
