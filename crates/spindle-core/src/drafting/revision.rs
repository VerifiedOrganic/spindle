//! Versioned prior-draft revision, deterministic candidate check, double-order
//! pairwise judge, stage transition idempotency, and revision-loop artifact contracts.
//! Pure data contracts only; no prompt rendering, tokenizer execution, or transport.

use crate::drafting::{
    AuthorDecisionItem, CriticFindingInput, CritiqueDirective, IngestedFinding, PacketSource,
    ShadowCritiqueOutcome, extraction::DraftReceiptBinding,
};
use crate::models::ContentRating;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const REVISION_LOOP_ARTIFACT_SCHEMA_VERSION: &str = "revision_loop_artifact_v1";
pub const PAIRWISE_JUDGE_SCHEMA_VERSION: &str = "pairwise_judge_v1";
pub const DEFAULT_MAX_REVISION_ROUNDS: u32 = 1;
pub const HARD_MAX_REVISION_ROUNDS: u32 = 2;
pub const DEFAULT_MAX_SCENE_CALLS: u32 = 6;
pub const HARD_MAX_SCENE_CALLS: u32 = 12;
pub const HARD_MAX_SCENE_TOKENS: usize = 210_000;

fn default_revision_loop_artifact_schema_version() -> String {
    REVISION_LOOP_ARTIFACT_SCHEMA_VERSION.into()
}

/// Ordered state progression for the critique/revision/judge loop
/// (`docs/drafting-and-revision-design.md`):
/// `packet_ready -> drafted -> deterministic_checked -> critiqued -> candidate_ready -> candidate_checked -> pairwise_judged -> accepted_original | accepted_revision`
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EditorialStage {
    PacketReady,
    Drafted,
    DeterministicChecked,
    Critiqued,
    CandidateReady,
    CandidateChecked,
    PairwiseJudged,
    AcceptedOriginal,
    AcceptedRevision,
}

impl EditorialStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PacketReady => "packet_ready",
            Self::Drafted => "drafted",
            Self::DeterministicChecked => "deterministic_checked",
            Self::Critiqued => "critiqued",
            Self::CandidateReady => "candidate_ready",
            Self::CandidateChecked => "candidate_checked",
            Self::PairwiseJudged => "pairwise_judged",
            Self::AcceptedOriginal => "accepted_original",
            Self::AcceptedRevision => "accepted_revision",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::AcceptedOriginal | Self::AcceptedRevision)
    }
}

/// Idempotency key for a stage transition:
/// `(run, branch, scene, base_text_hash, packet_hash, stage, attempt)`.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub struct StageTransitionKey {
    pub run_id: String,
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    pub base_text_hash: String,
    pub packet_hash: String,
    pub stage: EditorialStage,
    pub attempt: u32,
}

impl StageTransitionKey {
    pub fn idempotency_key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}:{}",
            self.run_id,
            self.branch_id,
            self.scene_id.as_deref().unwrap_or("unassigned"),
            self.base_text_hash,
            self.packet_hash,
            self.stage.as_str(),
            self.attempt
        )
    }
}

/// Durable stage transition record persisted before/after stage boundaries so a
/// restart never replays paid calls or commits twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StageTransitionRecord {
    pub key: StageTransitionKey,
    pub idempotency_key: String,
    pub output_hash: String,
    #[serde(default)]
    pub model_call_ids: Vec<String>,
    #[serde(default)]
    pub tokens_used: usize,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SpanChangeKind {
    Modified,
    Inserted,
    Deleted,
}

/// Paragraph-anchored UTF-8 byte span diff between incumbent and candidate prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedSpan {
    pub paragraph_index: usize,
    pub incumbent_start_byte: usize,
    pub incumbent_end_byte: usize,
    pub candidate_start_byte: usize,
    pub candidate_end_byte: usize,
    pub change_kind: SpanChangeKind,
}

/// Summary of paragraph and UTF-8 byte changes produced by a revision candidate.
/// Changes affecting > 40% (`> 4000` bps) of paragraphs without an explicit
/// `DirectiveScope::Scene` structural directive are flagged for review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EditSpanSummary {
    pub incumbent_paragraph_count: usize,
    pub candidate_paragraph_count: usize,
    pub changed_paragraph_count: usize,
    pub unchanged_paragraph_count: usize,
    /// Basis points (`0..=10_000`) of incumbent paragraphs affected by edits.
    pub changed_paragraph_ratio_bps: u32,
    pub incumbent_byte_len: usize,
    pub candidate_byte_len: usize,
    pub changed_incumbent_bytes: usize,
    pub changed_candidate_bytes: usize,
    /// Basis points (`0..=10_000`) of incumbent UTF-8 bytes affected by edits.
    pub changed_byte_ratio_bps: u32,
    pub scene_structural_rewrite_authorized: bool,
    pub broad_change_flagged: bool,
    #[serde(default)]
    pub changed_spans: Vec<ChangedSpan>,
}

/// Deterministic check report for an incumbent or candidate draft.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeterministicCandidateReport {
    pub text_sha256: String,
    pub truncated: bool,
    #[serde(default)]
    pub hard_continuity_blockers: Vec<String>,
    #[serde(default)]
    pub hard_rating_violations: Vec<String>,
    #[serde(default)]
    pub hard_antislop_blockers: Vec<String>,
    #[serde(default)]
    pub soft_antislop_findings: Vec<CriticFindingInput>,
    #[serde(default)]
    pub ingested_findings: Vec<IngestedFinding>,
    pub normalized_finding_fingerprint: String,
}

/// Comparison between the incumbent draft's deterministic report and a revised
/// candidate's deterministic report. Any new hard continuity, rating, or
/// anti-slop blocker disqualifies the candidate before pairwise judging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CandidateCheckComparison {
    pub incumbent_report: DeterministicCandidateReport,
    pub candidate_report: DeterministicCandidateReport,
    #[serde(default)]
    pub new_hard_continuity_blockers: Vec<String>,
    #[serde(default)]
    pub new_hard_rating_violations: Vec<String>,
    #[serde(default)]
    pub new_hard_antislop_blockers: Vec<String>,
    #[serde(default)]
    pub remaining_hard_blockers: Vec<String>,
    pub resolved_finding_count: usize,
    pub disqualified: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disqualification_reason: Option<String>,
}

/// The seven canonical pairwise judge rubric dimensions shared with `evals/prose_judge.py`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum JudgeRubricDimension {
    ScenePurposeFulfilled,
    TensionTurn,
    PovDiscipline,
    VoiceDistinctiveness,
    Dialogue,
    ProseFreshness,
    ContinuityWithBrief,
}

impl JudgeRubricDimension {
    pub const ALL: [Self; 7] = [
        Self::ScenePurposeFulfilled,
        Self::TensionTurn,
        Self::PovDiscipline,
        Self::VoiceDistinctiveness,
        Self::Dialogue,
        Self::ProseFreshness,
        Self::ContinuityWithBrief,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ScenePurposeFulfilled => "scene_purpose_fulfilled",
            Self::TensionTurn => "tension_turn",
            Self::PovDiscipline => "pov_discipline",
            Self::VoiceDistinctiveness => "voice_distinctiveness",
            Self::Dialogue => "dialogue",
            Self::ProseFreshness => "prose_freshness",
            Self::ContinuityWithBrief => "continuity_with_brief",
        }
    }
}

/// Blinded preference slot returned by a single judge pass (`passage_1`, `passage_2`, or `tie`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PassageSlot {
    #[serde(rename = "passage_1", alias = "passage1")]
    Passage1,
    #[serde(rename = "passage_2", alias = "passage2")]
    Passage2,
    Tie,
}

impl PassageSlot {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Passage1 => "passage_1",
            Self::Passage2 => "passage_2",
            Self::Tie => "tie",
        }
    }

    /// Normalize a raw judge preference token using the exact `evals/prose_judge.py`
    /// `_normalize_preference` rules.
    pub fn parse_normalized(raw: &str) -> Option<Self> {
        let cleaned: String = raw
            .trim()
            .to_lowercase()
            .chars()
            .map(|c| if c == '-' || c == ' ' { '_' } else { c })
            .collect();
        match cleaned.as_str() {
            "passage_1" | "passage1" | "1" | "first" => Some(Self::Passage1),
            "passage_2" | "passage2" | "2" | "second" => Some(Self::Passage2),
            "tie" | "draw" | "equal" | "neither" | "both" => Some(Self::Tie),
            _ => None,
        }
    }
}

/// Deblinded arm identifier (`baseline` = incumbent draft, `candidate` = revised draft, `tie`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum DeblindedArm {
    #[serde(alias = "incumbent")]
    Baseline,
    Candidate,
    Tie,
}

impl DeblindedArm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Candidate => "candidate",
            Self::Tie => "tie",
        }
    }
}

/// Position-bias classification when the two swapped-order judge passes disagree.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PositionBiasKind {
    FirstPosition,
    SecondPosition,
    TieFlip,
}

impl PositionBiasKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FirstPosition => "first_position",
            Self::SecondPosition => "second_position",
            Self::TieFlip => "tie_flip",
        }
    }
}

/// Normalized single-dimension score (`1..=5`, with `"N/A"` on `dialogue` coerced to `3`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinglePassDimensionScore {
    pub score: u8,
    #[serde(default)]
    pub dialogue_na: bool,
    pub rationale: String,
}

/// Structured single-pass judge output contract matching `evals/prose_judge.py`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinglePassJudgePayload {
    pub rubric: BTreeMap<String, serde_json::Value>,
    pub overall_preference: String,
    pub deciding_factor: String,
    #[serde(default)]
    pub strengths_passage_1: Vec<String>,
    #[serde(default)]
    pub strengths_passage_2: Vec<String>,
}

impl SinglePassJudgePayload {
    /// Return the generated JSON Schema for `SinglePassJudgePayload`.
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(Self)).expect("SinglePassJudgePayload schema")
    }
}

/// Validated and deblinded record for one pass of the swapped-order pairwise judge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinglePassJudgeRecord {
    pub order_index: u8,
    pub passage_1_arm: DeblindedArm,
    pub passage_2_arm: DeblindedArm,
    pub prompt_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_call_id: Option<String>,
    pub raw_preference: PassageSlot,
    pub deblinded_preference: DeblindedArm,
    pub deciding_factor: String,
    #[serde(default)]
    pub strengths_passage_1: Vec<String>,
    #[serde(default)]
    pub strengths_passage_2: Vec<String>,
    pub rubric: BTreeMap<String, SinglePassDimensionScore>,
}

/// Swapped-order pairwise judge evaluation. `accepted` is `true` if and only if
/// both passes independently prefer `DeblindedArm::Candidate` after deblinding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DoubleOrderJudgeEvaluation {
    pub pass_1: SinglePassJudgeRecord,
    pub pass_2: SinglePassJudgeRecord,
    pub verdict: DeblindedArm,
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_bias: Option<PositionBiasKind>,
    pub baseline_score_avg: f64,
    pub candidate_score_avg: f64,
}

/// Reason why the incumbent draft was retained or why a revision loop stopped.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RevisionRetentionReason {
    NoActionableDirectives,
    CriticInvalid,
    RepeatedFindingFingerprint,
    IdenticalCandidateHash,
    CandidateTruncated,
    CandidateDisqualifiedByDeterministicCheck,
    JudgePreferredIncumbent,
    JudgeTie,
    JudgeOrderDisagreementTie,
    JudgeInvalidOrUnavailable,
    NoDirectiveProgress,
    BudgetExhausted,
    StaleCasConflict,
}

impl RevisionRetentionReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoActionableDirectives => "no_actionable_directives",
            Self::CriticInvalid => "critic_invalid",
            Self::RepeatedFindingFingerprint => "repeated_finding_fingerprint",
            Self::IdenticalCandidateHash => "identical_candidate_hash",
            Self::CandidateTruncated => "candidate_truncated",
            Self::CandidateDisqualifiedByDeterministicCheck => {
                "candidate_disqualified_by_deterministic_check"
            }
            Self::JudgePreferredIncumbent => "judge_preferred_incumbent",
            Self::JudgeTie => "judge_tie",
            Self::JudgeOrderDisagreementTie => "judge_order_disagreement_tie",
            Self::JudgeInvalidOrUnavailable => "judge_invalid_or_unavailable",
            Self::NoDirectiveProgress => "no_directive_progress",
            Self::BudgetExhausted => "budget_exhausted",
            Self::StaleCasConflict => "stale_cas_conflict",
        }
    }
}

/// Compare-and-swap (CAS) version expectation captured when the revision loop
/// starts. Before any candidate is promoted, active branch, canon snapshot,
/// plan revision, source versions, and incumbent artifact hash must match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CasVersionExpectation {
    pub branch_id: String,
    pub canon_snapshot_hash: String,
    pub plan_revision: String,
    pub source_versions: Vec<PacketSource>,
    pub incumbent_text_sha256: String,
}

/// Complete record of a single critique -> revision -> deterministic check -> double-order judge round.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RevisionRoundRecord {
    pub round: u32,
    pub incumbent_text_sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incumbent_deterministic_report: Option<DeterministicCandidateReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critique_outcome: Option<ShadowCritiqueOutcome>,
    #[serde(default)]
    pub selected_directives: Vec<CritiqueDirective>,
    #[serde(default)]
    pub strengths_to_preserve: Vec<String>,
    #[serde(default)]
    pub requires_author_decision: Vec<AuthorDecisionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_text_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_receipt: Option<DraftReceiptBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_span_summary: Option<EditSpanSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_check_comparison: Option<CandidateCheckComparison>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_pass_1: Option<SinglePassJudgeRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judge_pass_2: Option<SinglePassJudgeRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub double_order_judgment: Option<DoubleOrderJudgeEvaluation>,
    #[serde(default)]
    pub round_accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_stop_reason: Option<RevisionRetentionReason>,
    #[serde(default)]
    pub prompt_hashes: Vec<String>,
    #[serde(default)]
    pub model_call_ids: Vec<String>,
    #[serde(default)]
    pub tokens_used: usize,
}

/// Durable revision-loop artifact persisted across every stage boundary so a
/// crash at any boundary resumes idempotently without replaying paid calls or
/// losing the incumbent draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RevisionLoopArtifactV1 {
    #[serde(default = "default_revision_loop_artifact_schema_version")]
    pub schema_version: String,
    pub artifact_id: String,
    pub run_id: String,
    pub project_id: String,
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    pub packet_hash: String,
    pub source_packet_sha256: String,
    pub rating: ContentRating,
    pub max_rounds: u32,
    pub max_scene_calls: u32,
    pub max_scene_tokens: usize,
    pub total_calls_used: u32,
    pub total_tokens_used: usize,
    pub cas_expectation: CasVersionExpectation,
    pub initial_draft_artifact_id: String,
    pub initial_text: String,
    pub initial_text_sha256: String,
    pub initial_receipt: DraftReceiptBinding,
    pub current_stage: EditorialStage,
    #[serde(default)]
    pub seen_finding_fingerprints: Vec<String>,
    #[serde(default)]
    pub transitions: Vec<StageTransitionRecord>,
    #[serde(default)]
    pub rounds: Vec<RevisionRoundRecord>,
    pub accepted_text: String,
    pub accepted_text_sha256: String,
    pub accepted_receipt: DraftReceiptBinding,
    pub revision_promoted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention_reason: Option<RevisionRetentionReason>,
    pub parked_for_review: bool,
    pub broad_edit_flagged_for_review: bool,
    #[serde(default)]
    pub requires_author_decision: Vec<AuthorDecisionItem>,
}
