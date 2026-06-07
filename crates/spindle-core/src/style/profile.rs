use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleProfileStatus {
    Ready,
    NeedsReview,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleProfileApplyMode {
    Merge,
    ReplaceGeneratedStyleNotes,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StyleProfileModelReceipt {
    pub model_route: String,
    pub model_name: String,
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub latency_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleProfileCard {
    pub profile_id: String,
    pub project_id: String,
    pub name: String,
    pub status: StyleProfileStatus,
    pub created_at: String,
    pub updated_at: String,
    pub corpus: StyleCorpusSummary,
    pub metrics: StyleCorpusMetrics,
    pub guidance: StyleProfileGuidance,
    pub source_policy: StyleProfileSourcePolicy,
    pub model_receipt: Option<StyleProfileModelReceipt>,
    #[serde(default)]
    pub quality: StyleProfileQualityReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleCorpusSummary {
    pub source_count: usize,
    pub analyzed_source_count: usize,
    pub skipped_source_count: usize,
    pub total_words: usize,
    pub total_characters: usize,
    pub chunk_count: usize,
    pub source_refs: Vec<StyleSourceRef>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StyleSourceRef {
    pub display_name: String,
    pub canonical_path: String,
    pub sha256: String,
    pub word_count: usize,
    pub included: bool,
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StyleCorpusMetrics {
    pub average_sentence_words: f64,
    pub median_sentence_words: f64,
    pub p90_sentence_words: f64,
    pub average_paragraph_words: f64,
    pub median_paragraph_words: f64,
    pub dialogue_line_ratio: f64,
    pub dialogue_word_ratio: f64,
    pub question_mark_rate_per_1k_words: f64,
    pub exclamation_rate_per_1k_words: f64,
    pub semicolon_rate_per_1k_words: f64,
    pub em_dash_rate_per_1k_words: f64,
    pub ellipsis_rate_per_1k_words: f64,
    pub first_person_pronoun_rate_per_1k_words: f64,
    pub third_person_pronoun_rate_per_1k_words: f64,
    pub top_functional_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
pub struct StyleProfileGuidance {
    pub summary: String,
    pub pov: Option<String>,
    pub tense: Option<String>,
    pub narrator_distance: Option<String>,
    pub narrator_voice: crate::style::NarratorVoice,
    pub pacing: Vec<String>,
    pub paragraphing: Vec<String>,
    pub sentence_rhythm: Vec<String>,
    pub diction: Vec<String>,
    pub dialogue: Vec<String>,
    pub exposition: Vec<String>,
    pub interiority: Vec<String>,
    pub humor_or_tension: Vec<String>,
    pub scene_structure: Vec<String>,
    pub do_rules: Vec<String>,
    pub avoid_rules: Vec<String>,
    pub prompt_snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StyleProfileSourcePolicy {
    pub local_user_provided: bool,
    pub source_text_persisted: bool,
    pub max_excerpt_words: usize,
    pub allowed_roots: Vec<String>,
    #[serde(default)]
    pub metrics_only: bool,
    #[serde(default)]
    pub source_sample_word_budget: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateStyleProfileFromMarkdownInput {
    pub project_id: String,
    pub profile_name: String,
    pub source_paths: Vec<String>,
    pub recursive: Option<bool>,
    pub include_globs: Option<Vec<String>>,
    pub exclude_globs: Option<Vec<String>>,
    pub max_files: Option<usize>,
    pub max_bytes_per_file: Option<usize>,
    pub max_total_words: Option<usize>,
    pub apply: Option<bool>,
    pub application_mode: Option<StyleProfileApplyMode>,
    pub source_sample_word_budget: Option<usize>,
    pub metrics_only: Option<bool>,
    pub force_apply: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateStyleProfileFromMarkdownOutput {
    pub profile: StyleProfileCard,
    pub applied: bool,
    pub application: Option<ApplyStyleProfileOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListStyleProfilesInput {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListStyleProfilesOutput {
    pub profiles: Vec<StyleProfileCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetStyleProfileInput {
    pub project_id: String,
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetStyleProfileOutput {
    pub profile: StyleProfileCard,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyStyleProfileInput {
    pub project_id: String,
    pub profile_id: String,
    pub mode: StyleProfileApplyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyStyleProfileOutput {
    pub project_id: String,
    pub profile_id: String,
    pub narrator_voice: crate::style::NarratorVoice,
    pub reader_contract_style_notes: Vec<String>,
    pub style_rule_id: Option<String>,
    pub invalidated_validator_findings: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StyleChunk {
    pub text: String,
    pub word_count: usize,
    pub label: Option<String>,
    pub source_display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum StyleWorldRuleAction {
    Create {
        rule_name: String,
        description: String,
    },
    Update {
        rule_id: String,
        rule_name: String,
        previous_description: String,
        new_description: String,
    },
    NoOp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewApplyStyleProfileInput {
    pub project_id: String,
    pub profile_id: String,
    pub mode: StyleProfileApplyMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewApplyStyleProfileOutput {
    pub project_id: String,
    pub profile_id: String,
    pub before_narrator_voice: crate::style::NarratorVoice,
    pub after_narrator_voice: crate::style::NarratorVoice,
    pub added_style_notes: Vec<String>,
    pub removed_style_notes: Vec<String>,
    pub style_rule_action: StyleWorldRuleAction,
    pub invalidated_validator_cache_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListStyleProfileApplicationsInput {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleProfileApplicationRecord {
    pub id: String,
    pub project_id: String,
    pub profile_id: String,
    pub applied_at: String,
    pub apply_mode: StyleProfileApplyMode,
    pub before_narrator_voice: crate::style::NarratorVoice,
    pub after_narrator_voice: crate::style::NarratorVoice,
    pub before_style_notes: Vec<String>,
    pub after_style_notes: Vec<String>,
    pub added_style_notes: Vec<String>,
    pub removed_style_notes: Vec<String>,
    pub style_rule_id: Option<String>,
    pub style_rule_action: String,
    pub style_rule_previous_description: Option<String>,
    pub invalidated_validator_count: usize,
    pub rolled_back_at: Option<String>,
    pub rollback_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListStyleProfileApplicationsOutput {
    pub applications: Vec<StyleProfileApplicationRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackStyleProfileApplicationInput {
    pub project_id: String,
    pub application_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackStyleProfileApplicationOutput {
    pub project_id: String,
    pub application_id: String,
    pub rolled_back_at: String,
    pub narrator_voice: crate::style::NarratorVoice,
    pub reader_contract_style_notes: Vec<String>,
    pub style_rule_action: String, // "deleted", "restored", "no_op"
    pub invalidated_validator_findings: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleRevisionSeverity {
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleRevisionTargetScope {
    RawText,
    Scene,
    Chapter,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleRevisionConfidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanStyleRevisionInput {
    pub project_id: String,
    pub profile_id: Option<String>,
    pub raw_text: Option<String>,
    pub scene_id: Option<String>,
    pub chapter_id: Option<String>,
    pub max_suggestions: Option<usize>,
    pub metrics_only: Option<bool>,
    pub include_rewrite_examples: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPlanFinding {
    pub severity: StyleRevisionSeverity,
    pub category: String,
    pub evidence_summary: String,
    pub suggested_correction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric_delta: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPlanStep {
    pub order: usize,
    pub finding_category: String,
    pub instructions: String,
    pub target_scope: StyleRevisionTargetScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    pub confidence: StyleRevisionConfidence,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPlanExample {
    pub original_prose: String,
    pub revised_prose: String,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PlanStyleRevisionOutput {
    pub project_id: String,
    pub profile_id: String,
    pub target_summary: String,
    pub drift_summary_score: StyleDriftSummaryScore,
    pub findings: Vec<StyleRevisionPlanFinding>,
    pub steps: Vec<StyleRevisionPlanStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_examples: Option<Vec<StyleRevisionPlanExample>>,
    pub mutates_prose: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckStyleAgainstProfileInput {
    pub project_id: String,
    pub profile_id: Option<String>,
    pub scene_id: Option<String>,
    pub raw_text: Option<String>,
    pub chapter_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StyleDriftFinding {
    pub severity: String,
    pub category: String,
    pub evidence_summary: String,
    pub suggested_correction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric_delta: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleDriftSummaryScore {
    Aligned,
    MildDrift,
    StrongDrift,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckStyleAgainstProfileOutput {
    pub project_id: String,
    pub profile_id: String,
    pub findings: Vec<StyleDriftFinding>,
    pub summary_score: StyleDriftSummaryScore,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleProfileQualityClassification {
    Ready,
    Thin,
    Inconsistent,
}

impl Default for StyleProfileQualityClassification {
    fn default() -> Self {
        Self::Ready
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StyleProfileQualityReport {
    pub corpus_size_words: usize,
    pub dialogue_coverage: f64,
    pub pov_tense_confidence: f64,
    pub chunk_consistency: f64,
    pub file_count: usize,
    pub warnings: Vec<String>,
    pub confidence_score: f64,
    pub classification: StyleProfileQualityClassification,
}

impl Default for StyleProfileQualityReport {
    fn default() -> Self {
        Self {
            corpus_size_words: 0,
            dialogue_coverage: 0.0,
            pov_tense_confidence: 1.0,
            chunk_consistency: 1.0,
            file_count: 0,
            warnings: Vec::new(),
            confidence_score: 1.0,
            classification: StyleProfileQualityClassification::Ready,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompareStyleProfilesInput {
    pub project_id: String,
    pub profile_id_a: String,
    pub profile_id_b: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CompareStyleProfilesOutput {
    pub project_id: String,
    pub profile_id_a: String,
    pub profile_id_b: String,
    pub metric_deltas: StyleCorpusMetricsDeltas,
    pub guidance_differences: StyleProfileGuidanceDifferences,
    pub likely_material_change: bool,
    pub change_reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleCorpusMetricsDeltas {
    pub average_sentence_words_delta: f64,
    pub median_sentence_words_delta: f64,
    pub p90_sentence_words_delta: f64,
    pub average_paragraph_words_delta: f64,
    pub median_paragraph_words_delta: f64,
    pub dialogue_line_ratio_delta: f64,
    pub dialogue_word_ratio_delta: f64,
    pub question_mark_rate_delta: f64,
    pub exclamation_rate_delta: f64,
    pub semicolon_rate_delta: f64,
    pub em_dash_rate_delta: f64,
    pub ellipsis_rate_delta: f64,
    pub first_person_pronoun_rate_delta: f64,
    pub third_person_pronoun_rate_delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleProfileGuidanceDifferences {
    pub summary_changed: bool,
    pub pov_changed: bool,
    pub tense_changed: bool,
    pub narrator_distance_changed: bool,
    pub voice_changed: bool,
    pub do_rules_added: Vec<String>,
    pub do_rules_removed: Vec<String>,
    pub avoid_rules_added: Vec<String>,
    pub avoid_rules_removed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveStyleProfileInput {
    pub project_id: String,
    pub profile_id: String,
    pub force: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveStyleProfileOutput {
    pub project_id: String,
    pub profile_id: String,
    pub archived_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewStyleRevisionPatchInput {
    pub project_id: String,
    pub scene_id: Option<String>,
    pub chapter_id: Option<String>,
    pub profile_id: Option<String>,
    pub max_suggestions: Option<usize>,
    pub instructions: Option<String>,
    pub run_evaluation: Option<bool>,
    pub run_validator_preflight: Option<bool>,
    pub minimum_improvement_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchHunk {
    pub old_range: String,
    pub new_range: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchScene {
    pub scene_id: String,
    pub original_word_count: usize,
    pub revised_word_count: usize,
    pub before_hash: String,
    pub after_hash: String,
    pub unified_diff: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunks: Option<Vec<StyleRevisionPatchHunk>>,
    pub revised_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewStyleRevisionPatchOutput {
    pub project_id: String,
    pub profile_id: String,
    pub scenes: Vec<StyleRevisionPatchScene>,
    pub model_receipt: Option<StyleProfileModelReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation: Option<EvaluateStyleRevisionPatchOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyStyleRevisionPatchInput {
    pub project_id: String,
    pub profile_id: String,
    pub scenes: Vec<StyleRevisionPatchScene>,
    pub model_receipt: Option<StyleProfileModelReceipt>,
    pub require_positive_evaluation: Option<bool>,
    pub minimum_improvement_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyStyleRevisionPatchOutput {
    pub project_id: String,
    pub applied_scene_ids: Vec<String>,
    pub audit_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchAuditRecord {
    pub id: String,
    pub project_id: String,
    pub profile_id: String,
    pub applied_at: String,
    pub target_ids: Vec<String>,
    pub before_hashes: Vec<String>,
    pub after_hashes: Vec<String>,
    pub model_receipt: Option<StyleProfileModelReceipt>,
    pub rolled_back_at: Option<String>,
    pub rollback_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListStyleRevisionPatchAuditsInput {
    pub project_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListStyleRevisionPatchAuditsOutput {
    pub audits: Vec<StyleRevisionPatchAuditRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackStyleRevisionPatchInput {
    pub project_id: String,
    pub audit_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackStyleRevisionPatchOutput {
    pub project_id: String,
    pub audit_id: String,
    pub rolled_back_at: String,
    pub restored_scene_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvaluateStyleRevisionPatchInput {
    pub project_id: String,
    pub profile_id: String,
    pub scenes: Vec<StyleRevisionPatchScene>,
    pub run_validator_preflight: Option<bool>,
    pub minimum_improvement_score: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StyleRevisionPatchStatus {
    Improved,
    Neutral,
    Regressed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchScore {
    pub before_warnings: usize,
    pub after_warnings: usize,
    pub before_errors: usize,
    pub after_errors: usize,
    pub improvement_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StyleRevisionPatchRisk {
    pub risk_type: String,
    pub severity: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchEvaluation {
    pub scene_id: String,
    pub score: StyleRevisionPatchScore,
    pub status: StyleRevisionPatchStatus,
    pub risks: Vec<StyleRevisionPatchRisk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvaluateStyleRevisionPatchOutput {
    pub project_id: String,
    pub profile_id: String,
    pub scenes: Vec<StyleRevisionPatchEvaluation>,
    pub aggregate_score: StyleRevisionPatchScore,
    pub status: StyleRevisionPatchStatus,
    pub risks: Vec<StyleRevisionPatchRisk>,
}
