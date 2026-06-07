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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckStyleAgainstProfileInput {
    pub project_id: String,
    pub profile_id: Option<String>,
    pub scene_id: Option<String>,
    pub raw_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct StyleDriftFinding {
    pub severity: String,
    pub category: String,
    pub evidence_summary: String,
    pub suggested_correction: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CheckStyleAgainstProfileOutput {
    pub project_id: String,
    pub profile_id: String,
    pub findings: Vec<StyleDriftFinding>,
}
