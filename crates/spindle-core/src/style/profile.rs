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
    /// Configured model route used for the call (for example `style_analyze`).
    pub model_route: String,
    /// Model or local stub name that produced the result.
    pub model_name: String,
    /// Input token count, when the route reports it.
    pub input_tokens: Option<usize>,
    /// Output token count, when the route reports it.
    pub output_tokens: Option<usize>,
    /// End-to-end latency in milliseconds, when recorded.
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_from_profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_number: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob_policy_metadata: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captured_at: Option<String>,
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
    // Every field carries `#[serde(default)]`: model-synthesized guidance is
    // parsed leniently so a response that omits (or slightly misnames) a field
    // degrades to an empty value instead of failing the whole parse. An
    // all-empty result is then detected explicitly (`== default()`) and
    // surfaced loudly by the caller rather than silently persisting.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub pov: Option<String>,
    #[serde(default)]
    pub tense: Option<String>,
    #[serde(default)]
    pub narrator_distance: Option<String>,
    #[serde(default)]
    pub narrator_voice: crate::style::NarratorVoice,
    #[serde(default)]
    pub pacing: Vec<String>,
    #[serde(default)]
    pub paragraphing: Vec<String>,
    #[serde(default)]
    pub sentence_rhythm: Vec<String>,
    #[serde(default)]
    pub diction: Vec<String>,
    #[serde(default)]
    pub dialogue: Vec<String>,
    #[serde(default)]
    pub exposition: Vec<String>,
    #[serde(default)]
    pub interiority: Vec<String>,
    #[serde(default)]
    pub humor_or_tension: Vec<String>,
    #[serde(default)]
    pub scene_structure: Vec<String>,
    #[serde(default)]
    pub do_rules: Vec<String>,
    #[serde(default)]
    pub avoid_rules: Vec<String>,
    #[serde(default)]
    pub prompt_snippet: String,
}

impl StyleProfileGuidance {
    /// True when synthesis produced nothing usable — every field empty. Used
    /// to fail loudly instead of persisting an unappliable profile.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// JSON Schema for [`StyleProfileGuidance`], injected into the style-analysis
/// prompt so the routed model is told the exact output shape it must produce.
/// Without this the prompt only referenced "the StyleProfileGuidance schema"
/// by name, so real models could not conform and guidance synthesis silently
/// degraded to an empty `NeedsReview` profile. Subschemas (e.g. NarratorVoice)
/// are inlined so the emitted schema is self-contained.
pub fn style_profile_guidance_json_schema() -> String {
    let settings = schemars::generate::SchemaSettings::openapi3().with(|s| {
        s.inline_subschemas = true;
    });
    let generator = settings.into_generator();
    let schema = generator.into_root_schema_for::<StyleProfileGuidance>();
    serde_json::to_string(&schema).unwrap_or_else(|_| "{}".to_string())
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
    #[serde(default)]
    pub source_paths: Vec<String>,
    #[serde(default)]
    pub recursive: Option<bool>,
    #[serde(default)]
    pub include_globs: Option<Vec<String>>,
    #[serde(default)]
    pub exclude_globs: Option<Vec<String>>,
    #[serde(default)]
    pub max_files: Option<usize>,
    #[serde(default)]
    pub max_bytes_per_file: Option<usize>,
    #[serde(default)]
    pub max_total_words: Option<usize>,
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
    /// Project identifier.
    pub project_id: String,
    /// Style profile identifier to apply.
    pub profile_id: String,
    /// How generated style notes combine with existing reader-contract notes.
    pub mode: StyleProfileApplyMode,
    /// Override the Ready-status and application-guidance gates so a profile
    /// can be applied deliberately even when it is `NeedsReview` or carries no
    /// prose guidance (e.g. a metrics-only profile activated for drift
    /// detection). With no application guidance the prose settings (narrator
    /// voice, style notes, style world rule) are left untouched — only the
    /// profile is activated — so forcing never clobbers existing prose style.
    #[serde(default)]
    pub force: Option<bool>,
    /// Report the proposed narrator-voice, style-note, and style-rule changes
    /// without writing, activating the profile, invalidating caches, or
    /// creating an application audit record.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApplyStyleProfileOutput {
    /// Project identifier.
    pub project_id: String,
    /// Style profile identifier.
    pub profile_id: String,
    /// Narrator voice after the apply, or the proposed voice on a dry run.
    pub narrator_voice: crate::style::NarratorVoice,
    /// Reader-contract style notes after the apply, or the proposed notes on a dry run.
    pub reader_contract_style_notes: Vec<String>,
    /// Style world-rule id created or updated by a live apply; on a dry run,
    /// the id that would be updated (none when a rule would be created).
    pub style_rule_id: Option<String>,
    /// Number of validator findings invalidated by a live apply; always zero on a dry run.
    pub invalidated_validator_findings: usize,
    /// True when this call only previewed the proposed effects.
    #[serde(default)]
    pub dry_run: bool,
    /// Before/after diff and proposed world-rule action on a dry run. Absent on a live apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<PreviewApplyStyleProfileOutput>,
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
    /// Create a new style world rule.
    Create {
        /// Name of the rule to create.
        rule_name: String,
        /// Description of the rule to create.
        description: String,
    },
    /// Update an existing style world rule.
    Update {
        /// Id of the rule to update.
        rule_id: String,
        /// Current rule name.
        rule_name: String,
        /// Description before the update.
        previous_description: String,
        /// Description after the update.
        new_description: String,
    },
    /// The rule already matches and needs no change.
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
    /// Project identifier.
    pub project_id: String,
    /// Style profile identifier.
    pub profile_id: String,
    /// Narrator voice before the proposed apply.
    pub before_narrator_voice: crate::style::NarratorVoice,
    /// Narrator voice after the proposed apply.
    pub after_narrator_voice: crate::style::NarratorVoice,
    /// Style notes the apply would add.
    pub added_style_notes: Vec<String>,
    /// Style notes the apply would remove.
    pub removed_style_notes: Vec<String>,
    /// Style world-rule change the apply would make.
    pub style_rule_action: StyleWorldRuleAction,
    /// Validator caches a live apply would invalidate.
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
#[derive(Default)]
pub enum StyleProfileQualityClassification {
    #[default]
    Ready,
    Thin,
    Inconsistent,
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

/// Input for `create_style_revision_patch`, the canonical non-mutating patch
/// creation step. It produces proposed scene patches for later evaluation and
/// `apply_style_revision_patch`; it never writes prose.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateStyleRevisionPatchInput {
    /// Project identifier.
    pub project_id: String,
    /// Scene to revise; provide exactly one of `scene_id` or `chapter_id`.
    pub scene_id: Option<String>,
    /// Chapter whose scenes should be revised; provide exactly one target.
    pub chapter_id: Option<String>,
    /// Style profile to apply; defaults to the project's active style profile.
    pub profile_id: Option<String>,
    /// Maximum number of rewrite suggestions to request.
    pub max_suggestions: Option<usize>,
    /// Optional author instructions for the revision.
    pub instructions: Option<String>,
    /// Run the style evaluator on the created patches before returning.
    pub run_evaluation: Option<bool>,
    /// Run the deterministic validator preflight before returning.
    pub run_validator_preflight: Option<bool>,
    /// Minimum evaluator improvement score required to keep a patch.
    pub minimum_improvement_score: Option<f64>,
}

impl From<CreateStyleRevisionPatchInput> for PreviewStyleRevisionPatchInput {
    fn from(value: CreateStyleRevisionPatchInput) -> Self {
        Self {
            project_id: value.project_id,
            scene_id: value.scene_id,
            chapter_id: value.chapter_id,
            profile_id: value.profile_id,
            max_suggestions: value.max_suggestions,
            instructions: value.instructions,
            run_evaluation: value.run_evaluation,
            run_validator_preflight: value.run_validator_preflight,
            minimum_improvement_score: value.minimum_improvement_score,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewStyleRevisionPatchInput {
    /// Project identifier.
    pub project_id: String,
    /// Scene to revise; provide exactly one of `scene_id` or `chapter_id`.
    pub scene_id: Option<String>,
    /// Chapter whose scenes should be revised; provide exactly one target.
    pub chapter_id: Option<String>,
    /// Style profile to apply; defaults to the project's active style profile.
    pub profile_id: Option<String>,
    /// Maximum number of rewrite suggestions to request.
    pub max_suggestions: Option<usize>,
    /// Optional author instructions for the revision.
    pub instructions: Option<String>,
    /// Run the style evaluator on the created patches before returning.
    pub run_evaluation: Option<bool>,
    /// Run the deterministic validator preflight before returning.
    pub run_validator_preflight: Option<bool>,
    /// Minimum evaluator improvement score required to keep a patch.
    pub minimum_improvement_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchHunk {
    /// Original line range covered by the hunk.
    pub old_range: String,
    /// Revised line range covered by the hunk.
    pub new_range: String,
    /// Unified-diff lines including their leading markers.
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchScene {
    /// Scene identifier the patch targets.
    pub scene_id: String,
    /// Word count of the current scene prose.
    pub original_word_count: usize,
    /// Word count of the proposed revised prose.
    pub revised_word_count: usize,
    /// Content hash of the current scene prose.
    pub before_hash: String,
    /// Content hash of the proposed revised prose.
    pub after_hash: String,
    /// Unified diff between the current and proposed prose.
    pub unified_diff: String,
    /// Structured hunks when the caller requested them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hunks: Option<Vec<StyleRevisionPatchHunk>>,
    /// Proposed revised prose; `apply_style_revision_patch` writes it.
    pub revised_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewStyleRevisionPatchOutput {
    /// Project identifier.
    pub project_id: String,
    /// Style profile the patches were generated against.
    pub profile_id: String,
    /// Proposed scene patches; no prose has been written.
    pub scenes: Vec<StyleRevisionPatchScene>,
    /// Model provenance for the patch-generation call.
    pub model_receipt: Option<StyleProfileModelReceipt>,
    /// Evaluator result when `run_evaluation` was requested.
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
    /// Validator warnings before the patch.
    pub before_warnings: usize,
    /// Validator warnings after the patch.
    pub after_warnings: usize,
    /// Validator errors before the patch.
    pub before_errors: usize,
    /// Validator errors after the patch.
    pub after_errors: usize,
    /// Net improvement score; positive means better.
    pub improvement_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct StyleRevisionPatchRisk {
    /// Risk category (for example `voice_drift`).
    pub risk_type: String,
    /// Risk severity.
    pub severity: String,
    /// Human-readable risk description.
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleRevisionPatchEvaluation {
    /// Scene identifier the evaluation covers.
    pub scene_id: String,
    /// Before/after validator score.
    pub score: StyleRevisionPatchScore,
    /// Whether the patch improved, was neutral, or regressed.
    pub status: StyleRevisionPatchStatus,
    /// Risks the evaluator identified.
    pub risks: Vec<StyleRevisionPatchRisk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EvaluateStyleRevisionPatchOutput {
    /// Project identifier.
    pub project_id: String,
    /// Style profile the patches were evaluated against.
    pub profile_id: String,
    /// Per-scene evaluations.
    pub scenes: Vec<StyleRevisionPatchEvaluation>,
    /// Aggregate score across all scenes.
    pub aggregate_score: StyleRevisionPatchScore,
    /// Aggregate status across all scenes.
    pub status: StyleRevisionPatchStatus,
    /// Risks observed across all scenes.
    pub risks: Vec<StyleRevisionPatchRisk>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckStyleProfileSourcesInput {
    pub project_id: String,
    pub profile_id: String,
    pub include_archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckStyleProfileSourcesOutput {
    pub profile_id: String,
    pub stale: bool,
    pub added_files: Vec<String>,
    pub removed_files: Vec<String>,
    pub changed_files: Vec<String>,
    pub unchanged_count: usize,
    pub missing_source_roots: Vec<String>,
    pub can_refresh: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewRefreshStyleProfileInput {
    pub project_id: String,
    pub profile_id: String,
    pub metrics_only: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreviewRefreshStyleProfileOutput {
    pub old_profile_summary: StyleProfileCard,
    pub candidate_profile_summary: StyleProfileCard,
    pub quality_report: StyleProfileQualityReport,
    pub metric_deltas: StyleCorpusMetricsDeltas,
    pub material_change: bool,
    pub apply_safety: Vec<String>,
    // Operator style-edit candidates that will feed the next refresh
    // (evolution §3.9). Populated only when the project has pending candidates;
    // otherwise all zero/empty. The `included`/`withheld` split reflects the
    // source-side rating discipline (evolution §4): explicit candidates are
    // withheld unless the style route's resolved agent declares explicit.
    /// Operator style-edit candidates that will feed the next refresh.
    #[serde(default)]
    pub style_edit_candidates: StyleEditCandidatesPreview,
}

// Preview section describing the operator style-edit candidates a refresh will
// consume (evolution §3.9). Carries counts, per-candidate scene refs, and a
// BOUNDED diff summary — never the full prose (matching the metadata-only
// granularity the refresh preview already uses for file sources).
/// Preview section describing the operator style-edit candidates a refresh will consume.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct StyleEditCandidatesPreview {
    /// Number of pending candidates that WILL be fed into this refresh
    /// (rating-cleared or non-explicit).
    pub included_count: usize,
    /// Number of pending candidates WITHHELD by the rating discipline (explicit
    /// candidates when the style route is not explicit-cleared). These stay
    /// pending.
    pub withheld_count: usize,
    /// Per included/withheld candidate: a scene ref + bounded diff summary.
    #[serde(default)]
    pub candidates: Vec<StyleEditCandidateRef>,
    // Human-readable note when candidates are withheld (evolution §4), e.g.
    // "2 explicit candidates withheld: style route not explicit-cleared".
    /// Human-readable note when candidates are withheld by the rating discipline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withholding_note: Option<String>,
}

// One style-edit candidate as it appears in a refresh preview (evolution
// §3.9). Metadata + a bounded diff summary only — never the full prose.
/// One style-edit candidate as it appears in a refresh preview.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleEditCandidateRef {
    pub candidate_id: String,
    pub scene_id: String,
    /// `book.chapter.scene` placement of the edited scene.
    pub scene_ref: String,
    /// The candidate's content rating (lowercased).
    pub content_rating: String,
    /// Whether this candidate will be included in the refresh (true) or withheld
    /// by the rating discipline (false).
    pub included: bool,
    /// Bounded, prose-free diff summary: character counts of the agent draft and
    /// operator edit and their delta. NOT the prose itself.
    pub diff_summary: String,
}

/// Compact, fully described preview of a refresh candidate. The full candidate
/// card remains in `RefreshStyleProfileOutput::new_profile`; the release-R
/// `preview_refresh_style_profile` twin returns the extended preview shape.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RefreshStyleProfilePreview {
    /// Profile identifier the refresh starts from.
    pub old_profile_id: String,
    /// Identifier minted for the proposed candidate profile.
    pub candidate_profile_id: String,
    /// Candidate profile name.
    pub candidate_name: String,
    /// Candidate quality summary.
    pub candidate_quality: StyleProfileQualitySummary,
    /// True when the candidate materially changes the existing profile.
    pub material_change: bool,
    /// Safety notes a live apply would surface.
    #[serde(default)]
    pub apply_safety: Vec<String>,
    /// Key corpus-metric changes between the old profile and the candidate.
    #[serde(default)]
    pub metric_deltas: Vec<RefreshStyleMetricDelta>,
    /// Operator style-edit candidates that would feed this refresh.
    pub style_edit_candidates: StyleEditCandidatesPreview,
}

/// Compact quality summary for a refresh candidate.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StyleProfileQualitySummary {
    /// Quality classification (`ready`, `thin`, or `unusable`).
    pub classification: StyleProfileQualityClassification,
    /// Confidence score in `0.0..=1.0`.
    pub confidence_score: f64,
    /// Total corpus words backing the candidate.
    pub corpus_size_words: usize,
    /// Human-readable quality warnings.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// One changed corpus metric between the old profile and the refresh candidate.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RefreshStyleMetricDelta {
    /// Metric name (for example `average_sentence_words`).
    pub metric: String,
    /// Previous value.
    pub before: f64,
    /// Candidate value.
    pub after: f64,
    /// Candidate value minus the previous value.
    pub delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RefreshStyleProfileInput {
    /// Project identifier.
    pub project_id: String,
    /// Style profile identifier.
    pub profile_id: String,
    /// Apply the refreshed profile immediately.
    pub apply_after_refresh: Option<bool>,
    /// Force apply when quality warnings exist.
    pub force_apply: Option<bool>,
    /// Recompute corpus metrics only.
    pub metrics_only: Option<bool>,
    // Operator style-edit candidate ids to DISMISS instead of consuming
    // (evolution §3.9). Dismissed candidates are flipped to `dismissed` (they
    // never feed a profile) rather than fed as refresh examples. Additive — an
    // empty list behaves exactly as before.
    /// Candidate ids to dismiss instead of consuming.
    #[serde(default)]
    pub dismiss_candidate_ids: Vec<String>,
    /// Report the candidate profile and quality/apply-safety preview without
    /// inserting a new profile version, consuming or dismissing candidates, or
    /// applying anything. Conflicts with `apply_after_refresh` and
    /// `dismiss_candidate_ids`, which mutate.
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RefreshStyleProfileOutput {
    pub new_profile: StyleProfileCard,
    pub applied: bool,
    pub application: Option<ApplyStyleProfileOutput>,
    // Style-edit candidate ids consumed by this refresh (fed as examples and
    // flipped to `consumed`), evolution §3.9. Empty when the project has no
    // pending candidates.
    /// Style-edit candidate ids consumed by this refresh.
    #[serde(default)]
    pub consumed_candidate_ids: Vec<String>,
    // Style-edit candidate ids dismissed by this refresh (flipped to
    // `dismissed`, never fed), evolution §3.9.
    /// Style-edit candidate ids dismissed by this refresh.
    #[serde(default)]
    pub dismissed_candidate_ids: Vec<String>,
    /// True when this call only previewed the candidate profile.
    #[serde(default)]
    pub dry_run: bool,
    /// Compact refresh preview on a dry run. Absent on a live refresh.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<RefreshStyleProfilePreview>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guidance_json_schema_is_valid_and_names_every_field() {
        let schema = style_profile_guidance_json_schema();
        assert!(!schema.is_empty());
        let parsed: serde_json::Value =
            serde_json::from_str(&schema).expect("schema must be valid JSON");
        // The model must be able to see the concrete field names it has to
        // emit; the old prompt only referenced the schema by name.
        let schema_str = parsed.to_string();
        for field in [
            "summary",
            "pov",
            "tense",
            "narrator_distance",
            "narrator_voice",
            "pacing",
            "paragraphing",
            "sentence_rhythm",
            "diction",
            "dialogue",
            "exposition",
            "interiority",
            "humor_or_tension",
            "scene_structure",
            "do_rules",
            "avoid_rules",
            "prompt_snippet",
        ] {
            assert!(
                schema_str.contains(&format!("\"{field}\"")),
                "schema must name field {field}"
            );
        }
    }

    #[test]
    fn guidance_parses_leniently_and_reports_emptiness() {
        // An empty object parses to an all-default (empty) guidance — the
        // shape the production bug produced — and is detected as empty.
        let empty: StyleProfileGuidance =
            serde_json::from_str("{}").expect("empty object must parse leniently");
        assert!(empty.is_empty());

        // A partial response parses, filling omitted fields with defaults, and
        // is NOT empty.
        let partial: StyleProfileGuidance = serde_json::from_str(
            r#"{"summary":"Close third, past tense.","do_rules":["Keep POV tight"]}"#,
        )
        .expect("partial object must parse leniently");
        assert!(!partial.is_empty());
        assert_eq!(partial.summary, "Close third, past tense.");
        assert_eq!(partial.do_rules, vec!["Keep POV tight".to_string()]);
        assert!(partial.avoid_rules.is_empty());
        assert!(partial.pov.is_none());
        assert!(partial.narrator_voice.is_empty());

        // A fully-populated response parses and is not empty.
        let full_json = r#"{
            "summary":"s","pov":"third_person_close","tense":"past",
            "narrator_distance":"close",
            "narrator_voice":{"pacing_feel":"punchy","notes":["n"]},
            "pacing":["p"],"paragraphing":["pa"],"sentence_rhythm":["r"],
            "diction":["d"],"dialogue":["di"],"exposition":["e"],
            "interiority":["i"],"humor_or_tension":["h"],
            "scene_structure":["sc"],"do_rules":["do"],"avoid_rules":["av"],
            "prompt_snippet":"snippet"
        }"#;
        let full: StyleProfileGuidance =
            serde_json::from_str(full_json).expect("full object must parse");
        assert!(!full.is_empty());
        assert_eq!(full.prompt_snippet, "snippet");
        assert_eq!(full.narrator_voice.pacing_feel.as_deref(), Some("punchy"));
    }
}
