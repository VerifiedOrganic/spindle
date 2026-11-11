//! Typed contracts and pure policy for the 0.4 model-assisted semantic
//! continuity lane.
//!
//! This module is deliberately infrastructure-free: it owns candidate,
//! evidence, judgment, host, and error DTOs plus the validation, severity,
//! dedupe, and budget policy. It performs no I/O, model calls, persistence, or
//! finding emission. The design is `docs/semantic-continuity-design.md`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Version of the prompt/DTO contract. It participates in the semantic cache
/// key and is checked on every candidate set, report, and host request.
pub const SEMANTIC_CONTRACT_VERSION: &str = "semantic-continuity-v1";

/// Maximum number of candidates in one scene request.
pub const SEMANTIC_MAX_CANDIDATES_PER_SCENE: usize = 12;
/// Maximum characters in one candidate's prose excerpt.
pub const SEMANTIC_MAX_EXCERPT_CHARS: usize = 600;
/// Maximum prompt estimate for one judge call.
pub const SEMANTIC_MAX_PROMPT_TOKENS: usize = 8_000;
/// Maximum output tokens for one judge call.
pub const SEMANTIC_MAX_OUTPUT_TOKENS: usize = 1_200;
/// One judgment call plus at most one repair.
pub const SEMANTIC_MAX_CALLS_PER_SCENE: usize = 2;
/// Maximum scenes audited in one semantic run.
pub const SEMANTIC_MAX_SCENES_PER_RUN: usize = 80;
/// Maximum input tokens in one semantic run.
pub const SEMANTIC_MAX_RUN_INPUT_TOKENS: usize = 500_000;
/// Per-call wall-clock ceiling.
pub const SEMANTIC_CALL_TIMEOUT_SECONDS: u64 = 45;
/// Run wall-clock ceiling.
pub const SEMANTIC_RUN_WALL_CLOCK_SECONDS: u64 = 15 * 60;
/// Judge calls that may be in flight at once.
pub const SEMANTIC_JUDGE_CONCURRENCY: usize = 2;
/// Maximum length of a judgment's rationale.
pub const SEMANTIC_MAX_RATIONALE_CHARS: usize = 240;

/// Stable default limits for the 0.4 lane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticContinuityLimitsV1 {
    pub max_candidates_per_scene: usize,
    pub max_excerpt_chars: usize,
    pub max_prompt_tokens: usize,
    pub max_output_tokens: usize,
    pub max_calls_per_scene: usize,
    pub max_scenes_per_run: usize,
    pub max_run_input_tokens: usize,
    pub call_timeout_seconds: u64,
    pub run_wall_clock_seconds: u64,
    pub judge_concurrency: usize,
}

impl Default for SemanticContinuityLimitsV1 {
    fn default() -> Self {
        Self {
            max_candidates_per_scene: SEMANTIC_MAX_CANDIDATES_PER_SCENE,
            max_excerpt_chars: SEMANTIC_MAX_EXCERPT_CHARS,
            max_prompt_tokens: SEMANTIC_MAX_PROMPT_TOKENS,
            max_output_tokens: SEMANTIC_MAX_OUTPUT_TOKENS,
            max_calls_per_scene: SEMANTIC_MAX_CALLS_PER_SCENE,
            max_scenes_per_run: SEMANTIC_MAX_SCENES_PER_RUN,
            max_run_input_tokens: SEMANTIC_MAX_RUN_INPUT_TOKENS,
            call_timeout_seconds: SEMANTIC_CALL_TIMEOUT_SECONDS,
            run_wall_clock_seconds: SEMANTIC_RUN_WALL_CLOCK_SECONDS,
            judge_concurrency: SEMANTIC_JUDGE_CONCURRENCY,
        }
    }
}

/// Execution mode for the additive semantic lane. Off is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SemanticMode {
    #[default]
    Off,
    Shadow,
    On,
}

impl SemanticMode {
    pub fn is_off(self) -> bool {
        self == Self::Off
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SemanticCandidateFamily {
    Attribute,
    Secret,
    Knowledge,
    Death,
    WorldRule,
    Quantity,
    Timeline,
}

impl SemanticCandidateFamily {
    /// Default deterministic cap for this family before global ranking.
    pub fn default_cap(self) -> usize {
        match self {
            Self::Attribute | Self::Knowledge | Self::WorldRule => 6,
            Self::Secret | Self::Death | Self::Quantity | Self::Timeline => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Attribute => "attribute",
            Self::Secret => "secret",
            Self::Knowledge => "knowledge",
            Self::Death => "death",
            Self::WorldRule => "world_rule",
            Self::Quantity => "quantity",
            Self::Timeline => "timeline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticCanonRefV1 {
    /// Canonical table name.
    pub table: String,
    /// Canonical record id.
    pub id: String,
    /// Row/version hash, not a display value.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticEvidenceHintV1 {
    /// Inclusive scene byte where the hint starts.
    pub start_byte: usize,
    /// Exclusive scene byte where the hint ends.
    pub end_byte: usize,
    /// Exact scene substring at the hint span.
    pub exact_quote: String,
    /// `resolver`, `world_rule_scan`, `price_scan`, `temporal_anchor`, ...
    pub origin: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticCandidateV1 {
    /// Contract version used to build this candidate.
    pub schema_version: String,
    /// Stable hash of family, canon refs, and excerpt.
    pub candidate_id: String,
    /// Candidate family that selected this item.
    pub family: SemanticCandidateFamily,
    /// Existing deterministic check type, for compatibility and grouping.
    pub check_type: String,
    /// Canon rows this candidate is judged against.
    pub canon_refs: Vec<SemanticCanonRefV1>,
    /// Subjects involved in the candidate.
    pub subject_ids: Vec<String>,
    /// Concise canonical constraint.
    pub constraint: String,
    /// Bounded scene prose window.
    pub excerpt: String,
    /// Inclusive byte where the excerpt starts.
    pub excerpt_start_byte: usize,
    /// Exclusive byte where the excerpt ends.
    pub excerpt_end_byte: usize,
    /// Scene-local anchors for the candidate.
    pub evidence_hints: Vec<SemanticEvidenceHintV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticCandidateSetV1 {
    /// Contract version used to build the set.
    pub schema_version: String,
    /// Scene the candidates belong to.
    pub scene_id: String,
    /// SHA-256 of the scene prose.
    pub scene_sha256: String,
    /// Stable hash of the relevant canon rows.
    pub canon_version: String,
    /// Ranked, capped candidates for this scene.
    pub candidates: Vec<SemanticCandidateV1>,
    /// Counts of family candidates omitted by a cap.
    #[serde(default)]
    pub omitted_by_family: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticEvidenceSpanV1 {
    /// Scene this evidence belongs to.
    pub scene_id: String,
    /// SHA-256 of the scene revision the evidence was anchored in.
    pub scene_sha256: String,
    /// Inclusive byte offset where the evidence starts.
    pub start_byte: usize,
    /// Exclusive byte offset where the evidence ends.
    pub end_byte: usize,
    /// Exact scene substring at the byte span.
    pub exact_quote: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticVerdict {
    Contradiction,
    Compatible,
    Insufficient,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticJudgmentV1 {
    /// Candidate being judged.
    pub candidate_id: String,
    /// Model verdict; the server owns severity.
    pub verdict: SemanticVerdict,
    /// Canon rows the judgment cites.
    #[serde(default)]
    pub canon_refs: Vec<SemanticCanonRefV1>,
    /// Exact supporting scene evidence, when the judge supplied it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<SemanticEvidenceSpanV1>,
    /// Judge confidence in 0.0..=1.0.
    pub confidence: f32,
    /// Short machine-checkable explanation.
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticJudgeReportV1 {
    /// Contract version of this report.
    pub schema_version: String,
    /// Scene the judgments target.
    pub scene_id: String,
    /// SHA-256 of the judged scene prose.
    pub scene_sha256: String,
    /// One judgment per candidate (missing means insufficient).
    pub judgments: Vec<SemanticJudgmentV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticHostRequestV1 {
    /// Stable hash of the whole request.
    pub request_id: String,
    /// Contract version for host/report compatibility.
    pub contract_version: String,
    /// Ranked candidate set for one scene.
    pub candidate_set: SemanticCandidateSetV1,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticHostReportV1 {
    /// Request this report answers.
    pub request_id: String,
    /// Hash of the candidate set in the request.
    pub candidate_set_hash: String,
    /// SHA-256 of the judged scene prose.
    pub scene_sha256: String,
    /// Canon version in the request.
    pub canon_version: String,
    /// Judgments returned by the host.
    pub judgments: Vec<SemanticJudgmentV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticErrorKind {
    RouteUnavailable,
    RatingDenied,
    Timeout,
    BudgetExceeded,
    MalformedOutput,
    Truncated,
    UnknownCandidate,
    UnknownCanonRef,
    InvalidEvidence,
    StaleHostRequest,
    CacheCorrupt,
}

/// Complete cache identity for one validated semantic judge report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticCacheKeyV1 {
    /// Scene the report describes.
    pub scene_id: String,
    /// Prose hash at judgment time.
    pub scene_sha256: String,
    /// Relevant canon hash at judgment time.
    pub canon_version: String,
    /// Hash of the exact candidate set.
    pub candidate_set_hash: String,
    /// Semantic contract version.
    pub contract_version: String,
    /// Resolved route (`continuity_semantic`, `review`, or `host`).
    pub route: String,
    /// Resolved model/agent identity.
    pub model_identity: String,
}

/// One actual judge dispatch receipt (initial or repair).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticCallSummaryV1 {
    /// Scene on whose behalf the call was made.
    pub scene_id: String,
    /// Prompt tokens attributable to this call.
    pub input_tokens: u64,
    /// Output tokens attributable to this call.
    pub output_tokens: u64,
    /// Wall time for this call.
    pub elapsed_ms: u64,
    /// True when this was the one allowed repair call.
    pub repaired: bool,
    /// `success`, `truncated`, `timeout`, `route_error`, or `local_stub`.
    pub stop_reason: String,
    /// Character length of this call's raw output.
    pub output_chars: usize,
}

/// Per-scene semantic accounting for the evaluator. Present only when the
/// semantic lane runs in shadow or on mode.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticSceneSummaryV1 {
    /// Scene the counters describe.
    pub scene_id: String,
    /// Candidates built for this scene.
    pub candidates: usize,
    /// Validated judgments accepted for this scene.
    pub judgments: usize,
    /// Would-be/emitted warning count after dedupe.
    pub warnings: usize,
    /// Would-be/emitted advisory count after dedupe.
    pub infos: usize,
    /// Whether a validated cache report satisfied this scene.
    pub cache_hit: bool,
    /// Model calls made for this scene (including one repair).
    pub model_calls: usize,
    /// Whether a repair call was made.
    pub repaired: bool,
    /// Prompt tokens attributable to this scene.
    pub input_tokens: u64,
    /// Output tokens attributable to this scene.
    pub output_tokens: u64,
    /// Wall time for this scene.
    pub elapsed_ms: u64,
    /// First typed error for this scene, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<SemanticJudgeErrorV1>,
}

/// Shadow/on run counters. This is observability, not a finding channel.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticRunSummaryV1 {
    /// Effective mode used for this run.
    pub mode: SemanticMode,
    /// Scenes considered in scope.
    pub scenes: usize,
    /// Scenes with at least one candidate.
    pub candidate_scenes: usize,
    /// Candidates built (before judgments).
    pub candidates: usize,
    /// Judgments accepted after validation.
    pub judgments: usize,
    /// Warnings emitted.
    pub warnings: usize,
    /// Advisory info findings emitted.
    pub infos: usize,
    /// Candidates whose judgment was dropped.
    pub dropped: usize,
    /// Validated cache hits.
    pub cache_hits: usize,
    /// Cache misses (including corrupt keys treated as misses).
    pub cache_misses: usize,
    /// Model calls made, excluding host reports.
    pub model_calls: usize,
    /// Prompt tokens reported by the router, when available.
    pub input_tokens: u64,
    /// Output tokens reported by the router, when available.
    pub output_tokens: u64,
    /// Wall time for the semantic stage.
    pub elapsed_ms: u64,
    /// Typed route/parse/host errors, never containing scene prose.
    #[serde(default)]
    pub errors: Vec<SemanticJudgeErrorV1>,
    /// Individual dispatch receipts for per-call percentiles.
    #[serde(default)]
    pub calls: Vec<SemanticCallSummaryV1>,
    /// Verdict counts across validated judgments.
    #[serde(default)]
    pub verdicts: BTreeMap<String, usize>,
    /// Confidence histogram: <0.5, 0.5-0.7, 0.7-0.8, 0.8-0.85, 0.85-1.
    #[serde(default)]
    pub confidence_buckets: Vec<usize>,
    /// Why a candidate was discarded before emission.
    #[serde(default)]
    pub discard_reasons: BTreeMap<String, usize>,
    /// Stop reasons across actual dispatches.
    #[serde(default)]
    pub stop_reasons: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticJudgeErrorV1 {
    /// Stable error category.
    pub kind: SemanticErrorKind,
    /// Whether the same request may be retried.
    pub retryable: bool,
    /// Route name when the failure was route-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Content rating when the failure was rating-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<String>,
    /// Machine-readable detail only; never scene prose or secret values.
    pub detail: String,
}

impl SemanticJudgeErrorV1 {
    pub fn new(kind: SemanticErrorKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            retryable: false,
            route: None,
            rating: None,
            detail: detail.into(),
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn with_route(mut self, route: impl Into<String>) -> Self {
        self.route = Some(route.into());
        self
    }

    pub fn with_rating(mut self, rating: impl Into<String>) -> Self {
        self.rating = Some(rating.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticIssueSeverity {
    Warning,
    Info,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SemanticDecisionReason {
    Verified,
    NotContradiction,
    MissingCanonRef,
    MissingEvidence,
    LowConfidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticDecisionV1 {
    /// `None` means the judgment produces no finding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<SemanticIssueSeverity>,
    pub reason: SemanticDecisionReason,
}

/// Confidence required before a contradiction may be emitted as a warning.
pub fn semantic_warning_threshold(family: SemanticCandidateFamily) -> f32 {
    match family {
        SemanticCandidateFamily::Secret => 0.85,
        SemanticCandidateFamily::Attribute
        | SemanticCandidateFamily::Knowledge
        | SemanticCandidateFamily::Death => 0.80,
        SemanticCandidateFamily::WorldRule
        | SemanticCandidateFamily::Quantity
        | SemanticCandidateFamily::Timeline => 0.75,
    }
}

/// Server-side severity policy. The caller must have validated evidence and
/// canon refs first; this function never trusts a model-provided severity.
///
/// A valid contradiction with a missing evidence span is advisory (`info`);
/// only a contradiction with verified evidence, a canon ref, and confidence at
/// or above the family threshold becomes a warning.
pub fn decide_semantic_severity(
    candidate: &SemanticCandidateV1,
    judgment: &SemanticJudgmentV1,
) -> SemanticDecisionV1 {
    if judgment.verdict != SemanticVerdict::Contradiction {
        return SemanticDecisionV1 {
            severity: None,
            reason: SemanticDecisionReason::NotContradiction,
        };
    }
    if judgment.canon_refs.is_empty() {
        return SemanticDecisionV1 {
            severity: None,
            reason: SemanticDecisionReason::MissingCanonRef,
        };
    }
    if judgment.evidence.is_none() {
        return SemanticDecisionV1 {
            severity: Some(SemanticIssueSeverity::Info),
            reason: SemanticDecisionReason::MissingEvidence,
        };
    }
    if !judgment.confidence.is_finite()
        || judgment.confidence < semantic_warning_threshold(candidate.family)
    {
        return SemanticDecisionV1 {
            severity: Some(SemanticIssueSeverity::Info),
            reason: SemanticDecisionReason::LowConfidence,
        };
    }
    SemanticDecisionV1 {
        severity: Some(SemanticIssueSeverity::Warning),
        reason: SemanticDecisionReason::Verified,
    }
}

/// Stable, ordered key used to deduplicate deterministic and semantic findings.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticDedupeKeyV1 {
    pub scene_id: String,
    pub check_type: String,
    pub canon_refs: Vec<SemanticCanonRefV1>,
    pub subject_ids: Vec<String>,
    pub evidence_start: usize,
    pub evidence_end: usize,
}

pub fn semantic_dedupe_key(
    scene_id: &str,
    check_type: &str,
    canon_refs: &[SemanticCanonRefV1],
    subject_ids: &[String],
    evidence: Option<&SemanticEvidenceSpanV1>,
) -> SemanticDedupeKeyV1 {
    let mut refs = canon_refs.to_vec();
    refs.sort();
    refs.dedup();
    let mut subjects = subject_ids.to_vec();
    subjects.sort();
    subjects.dedup();
    SemanticDedupeKeyV1 {
        scene_id: scene_id.to_string(),
        check_type: check_type.to_string(),
        canon_refs: refs,
        subject_ids: subjects,
        evidence_start: evidence.map_or(0, |span| span.start_byte),
        evidence_end: evidence.map_or(0, |span| span.end_byte),
    }
}

pub fn semantic_evidence_overlaps(
    left: &SemanticEvidenceSpanV1,
    right: &SemanticEvidenceSpanV1,
) -> bool {
    left.scene_id == right.scene_id
        && left.start_byte < right.end_byte
        && right.start_byte < left.end_byte
}

/// Deduplicate semantic findings by canon refs/subjects and overlapping
/// evidence. Deterministic findings have no evidence span; callers can pass the
/// same key with `evidence: None` to suppress a semantic copy by canon ref.
pub fn semantic_findings_are_duplicates(
    left: &SemanticDedupeKeyV1,
    right: &SemanticDedupeKeyV1,
) -> bool {
    if left.scene_id != right.scene_id
        || left.check_type != right.check_type
        || left.canon_refs != right.canon_refs
        || left.subject_ids != right.subject_ids
    {
        return false;
    }
    if left.evidence_start == 0 && left.evidence_end == 0 {
        return true;
    }
    if right.evidence_start == 0 && right.evidence_end == 0 {
        return true;
    }
    left.evidence_start < right.evidence_end && right.evidence_start < left.evidence_end
}

pub fn scene_sha256(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn candidate_set_hash_inner(candidate_set: &SemanticCandidateSetV1) -> String {
    // Field order is fixed by the DTO and vectors/BTreeMaps are canonicalized
    // by construction; JSON is used only as a stable byte serialization.
    let bytes = serde_json::to_vec(candidate_set).unwrap_or_default();
    let digest = Sha256::digest(&bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn semantic_candidate_set_hash(candidate_set: &SemanticCandidateSetV1) -> String {
    candidate_set_hash_inner(candidate_set)
}

pub fn semantic_host_request_id(candidate_set: &SemanticCandidateSetV1) -> String {
    let digest = Sha256::digest(
        format!(
            "{SEMANTIC_CONTRACT_VERSION}:{}",
            candidate_set_hash_inner(candidate_set)
        )
        .as_bytes(),
    );
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validation_error(kind: SemanticErrorKind, detail: impl Into<String>) -> SemanticJudgeErrorV1 {
    SemanticJudgeErrorV1::new(kind, detail)
}

fn validate_canon_ref(reference: &SemanticCanonRefV1) -> Result<(), SemanticJudgeErrorV1> {
    if reference.table.is_empty() || reference.id.is_empty() || reference.version.is_empty() {
        return Err(validation_error(
            SemanticErrorKind::UnknownCanonRef,
            "canon ref table, id, and version must be non-empty",
        ));
    }
    if !reference
        .table
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Err(validation_error(
            SemanticErrorKind::UnknownCanonRef,
            "canon ref table must be a canonical snake_case table name",
        ));
    }
    Ok(())
}

fn utf8_boundary(text: &str, start: usize, end: usize) -> bool {
    start <= end && end <= text.len() && text.is_char_boundary(start) && text.is_char_boundary(end)
}

fn validate_span_against_scene(
    span: &SemanticEvidenceSpanV1,
    expected_scene_id: &str,
    expected_scene_sha256: &str,
    scene_text: &str,
) -> Result<(), SemanticJudgeErrorV1> {
    if span.scene_id != expected_scene_id {
        return Err(validation_error(
            SemanticErrorKind::InvalidEvidence,
            "evidence scene_id does not match the candidate scene",
        ));
    }
    if span.scene_sha256 != expected_scene_sha256 {
        return Err(validation_error(
            SemanticErrorKind::InvalidEvidence,
            "evidence scene_sha256 does not match the scene",
        ));
    }
    if span.start_byte >= span.end_byte
        || span.exact_quote.is_empty()
        || !utf8_boundary(scene_text, span.start_byte, span.end_byte)
    {
        return Err(validation_error(
            SemanticErrorKind::InvalidEvidence,
            "evidence byte range is out of bounds or splits UTF-8",
        ));
    }
    if scene_text[span.start_byte..span.end_byte] != span.exact_quote {
        return Err(validation_error(
            SemanticErrorKind::InvalidEvidence,
            "evidence exact_quote does not match the scene span",
        ));
    }
    Ok(())
}

pub fn validate_semantic_evidence_span(
    span: &SemanticEvidenceSpanV1,
    expected_scene_id: &str,
    scene_text: &str,
) -> Result<(), SemanticJudgeErrorV1> {
    let expected_hash = scene_sha256(scene_text);
    validate_span_against_scene(span, expected_scene_id, &expected_hash, scene_text)
}

fn refs_are_subset(judgment: &[SemanticCanonRefV1], candidate: &[SemanticCanonRefV1]) -> bool {
    let candidate_keys: BTreeSet<(&str, &str, &str)> = candidate
        .iter()
        .map(|reference| {
            (
                reference.table.as_str(),
                reference.id.as_str(),
                reference.version.as_str(),
            )
        })
        .collect();
    judgment.iter().all(|reference| {
        candidate_keys.contains(&(
            reference.table.as_str(),
            reference.id.as_str(),
            reference.version.as_str(),
        ))
    })
}

fn span_overlaps_candidate(span: &SemanticEvidenceSpanV1, candidate: &SemanticCandidateV1) -> bool {
    let excerpt_overlap = span.start_byte < candidate.excerpt_end_byte
        && candidate.excerpt_start_byte < span.end_byte;
    let hint_overlap = candidate
        .evidence_hints
        .iter()
        .any(|hint| span.start_byte < hint.end_byte && hint.start_byte < span.end_byte);
    excerpt_overlap || hint_overlap
}

pub fn validate_semantic_candidate_set(
    candidate_set: &SemanticCandidateSetV1,
    scene_text: &str,
) -> Result<(), SemanticJudgeErrorV1> {
    if candidate_set.schema_version != SEMANTIC_CONTRACT_VERSION {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "candidate set schema_version is unsupported",
        ));
    }
    if candidate_set.scene_id.is_empty() {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "candidate set scene_id is empty",
        ));
    }
    if candidate_set.scene_sha256 != scene_sha256(scene_text) {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "candidate set scene_sha256 does not match the scene",
        ));
    }
    if candidate_set.canon_version.is_empty() {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "candidate set canon_version is empty",
        ));
    }
    if candidate_set.candidates.len() > SEMANTIC_MAX_CANDIDATES_PER_SCENE {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "candidate set exceeds the per-scene cap",
        ));
    }
    let mut ids = BTreeSet::new();
    for candidate in &candidate_set.candidates {
        if candidate.schema_version != SEMANTIC_CONTRACT_VERSION {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "candidate schema_version is unsupported",
            ));
        }
        if candidate.candidate_id.is_empty() || !ids.insert(candidate.candidate_id.clone()) {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "candidate ids must be non-empty and unique",
            ));
        }
        if candidate.check_type.is_empty() {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "candidate check_type is empty",
            ));
        }
        if candidate.canon_refs.is_empty() {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "candidate must carry at least one canon ref",
            ));
        }
        for reference in &candidate.canon_refs {
            validate_canon_ref(reference)?;
        }
        if candidate.constraint.is_empty() || candidate.evidence_hints.is_empty() {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "candidate must carry a constraint and an evidence hint",
            ));
        }
        if candidate.excerpt.is_empty()
            || !utf8_boundary(
                scene_text,
                candidate.excerpt_start_byte,
                candidate.excerpt_end_byte,
            )
            || scene_text[candidate.excerpt_start_byte..candidate.excerpt_end_byte]
                != candidate.excerpt
        {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "candidate excerpt does not match the scene span",
            ));
        }
        if candidate.excerpt.chars().count() > SEMANTIC_MAX_EXCERPT_CHARS {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "candidate excerpt exceeds the contract limit",
            ));
        }
        for hint in &candidate.evidence_hints {
            let hint_overlaps_excerpt = hint.start_byte < candidate.excerpt_end_byte
                && candidate.excerpt_start_byte < hint.end_byte;
            if hint.start_byte >= hint.end_byte
                || hint.exact_quote.is_empty()
                || hint.origin.is_empty()
                || !hint_overlaps_excerpt
                || !utf8_boundary(scene_text, hint.start_byte, hint.end_byte)
                || scene_text[hint.start_byte..hint.end_byte] != hint.exact_quote
            {
                return Err(validation_error(
                    SemanticErrorKind::InvalidEvidence,
                    "candidate evidence hint does not match the scene span",
                ));
            }
        }
    }
    Ok(())
}

fn validate_judgments(
    judgments: &[SemanticJudgmentV1],
    candidate_set: &SemanticCandidateSetV1,
    scene_text: &str,
) -> Result<(), SemanticJudgeErrorV1> {
    let candidates: BTreeMap<&str, &SemanticCandidateV1> = candidate_set
        .candidates
        .iter()
        .map(|candidate| (candidate.candidate_id.as_str(), candidate))
        .collect();
    let mut seen = BTreeSet::new();
    for judgment in judgments {
        let Some(candidate) = candidates.get(judgment.candidate_id.as_str()) else {
            return Err(validation_error(
                SemanticErrorKind::UnknownCandidate,
                "judgment names an unknown candidate",
            ));
        };
        if !seen.insert(judgment.candidate_id.clone()) {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "judgment candidate ids must be unique",
            ));
        }
        if !judgment.confidence.is_finite() || !(0.0..=1.0).contains(&judgment.confidence) {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "judgment confidence must be finite and in 0..=1",
            ));
        }
        if judgment.rationale.chars().count() > SEMANTIC_MAX_RATIONALE_CHARS {
            return Err(validation_error(
                SemanticErrorKind::MalformedOutput,
                "judgment rationale exceeds the contract limit",
            ));
        }
        for reference in &judgment.canon_refs {
            validate_canon_ref(reference)?;
        }
        if !refs_are_subset(&judgment.canon_refs, &candidate.canon_refs) {
            return Err(validation_error(
                SemanticErrorKind::UnknownCanonRef,
                "judgment cites a canon ref outside the candidate",
            ));
        }
        if let Some(evidence) = judgment.evidence.as_ref() {
            validate_span_against_scene(
                evidence,
                &candidate_set.scene_id,
                &candidate_set.scene_sha256,
                scene_text,
            )?;
            if !span_overlaps_candidate(evidence, candidate) {
                return Err(validation_error(
                    SemanticErrorKind::InvalidEvidence,
                    "evidence does not overlap the candidate excerpt or hints",
                ));
            }
        }
    }
    Ok(())
}

pub fn validate_semantic_judge_report(
    report: &SemanticJudgeReportV1,
    candidate_set: &SemanticCandidateSetV1,
    scene_text: &str,
) -> Result<(), SemanticJudgeErrorV1> {
    validate_semantic_candidate_set(candidate_set, scene_text)?;
    if report.schema_version != SEMANTIC_CONTRACT_VERSION {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "report schema_version is unsupported",
        ));
    }
    if report.scene_id != candidate_set.scene_id
        || report.scene_sha256 != candidate_set.scene_sha256
    {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "report scene identity does not match the candidate set",
        ));
    }
    validate_judgments(&report.judgments, candidate_set, scene_text)
}

pub fn validate_semantic_host_request(
    request: &SemanticHostRequestV1,
    scene_text: &str,
) -> Result<(), SemanticJudgeErrorV1> {
    if request.contract_version != SEMANTIC_CONTRACT_VERSION {
        return Err(validation_error(
            SemanticErrorKind::MalformedOutput,
            "host request contract_version is unsupported",
        ));
    }
    validate_semantic_candidate_set(&request.candidate_set, scene_text)?;
    if request.request_id != semantic_host_request_id(&request.candidate_set) {
        return Err(validation_error(
            SemanticErrorKind::StaleHostRequest,
            "host request_id does not match its candidate set",
        ));
    }
    Ok(())
}

pub fn validate_semantic_host_report(
    report: &SemanticHostReportV1,
    request: &SemanticHostRequestV1,
    scene_text: &str,
) -> Result<(), SemanticJudgeErrorV1> {
    validate_semantic_host_request(request, scene_text)?;
    if report.request_id != request.request_id
        || report.candidate_set_hash != semantic_candidate_set_hash(&request.candidate_set)
    {
        return Err(validation_error(
            SemanticErrorKind::StaleHostRequest,
            "host report does not match the current request",
        ));
    }
    if report.scene_sha256 != request.candidate_set.scene_sha256
        || report.canon_version != request.candidate_set.canon_version
    {
        return Err(validation_error(
            SemanticErrorKind::StaleHostRequest,
            "host report scene/canon version is stale",
        ));
    }
    validate_judgments(&report.judgments, &request.candidate_set, scene_text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> &'static str {
        "Mara crossed the yard at dawn. The bell did not ring."
    }

    fn canon_ref() -> SemanticCanonRefV1 {
        SemanticCanonRefV1 {
            table: "character_state".into(),
            id: "state-1".into(),
            version: "v1".into(),
        }
    }

    fn candidate() -> SemanticCandidateV1 {
        let text = scene();
        let start = text.find("The bell").unwrap();
        let end = text.len();
        SemanticCandidateV1 {
            schema_version: SEMANTIC_CONTRACT_VERSION.into(),
            candidate_id: "candidate-1".into(),
            family: SemanticCandidateFamily::WorldRule,
            check_type: "world_rule_semantic_drift".into(),
            canon_refs: vec![canon_ref()],
            subject_ids: vec!["character-1".into()],
            constraint: "the bell must ring at dawn".into(),
            excerpt: text[start..end].into(),
            excerpt_start_byte: start,
            excerpt_end_byte: end,
            evidence_hints: vec![SemanticEvidenceHintV1 {
                start_byte: start,
                end_byte: start + "The bell".len(),
                exact_quote: "The bell".into(),
                origin: "resolver".into(),
            }],
        }
    }

    fn candidate_set() -> SemanticCandidateSetV1 {
        SemanticCandidateSetV1 {
            schema_version: SEMANTIC_CONTRACT_VERSION.into(),
            scene_id: "scene-1".into(),
            scene_sha256: scene_sha256(scene()),
            canon_version: "canon-v1".into(),
            candidates: vec![candidate()],
            omitted_by_family: BTreeMap::new(),
        }
    }

    fn judgment() -> SemanticJudgmentV1 {
        let text = scene();
        let start = text.find("The bell").unwrap();
        let end = start + "The bell did not ring".len();
        SemanticJudgmentV1 {
            candidate_id: "candidate-1".into(),
            verdict: SemanticVerdict::Contradiction,
            canon_refs: vec![canon_ref()],
            evidence: Some(SemanticEvidenceSpanV1 {
                scene_id: "scene-1".into(),
                scene_sha256: scene_sha256(scene()),
                start_byte: start,
                end_byte: end,
                exact_quote: text[start..end].into(),
            }),
            confidence: 0.9,
            rationale: "the bell stayed silent".into(),
        }
    }

    #[test]
    fn validates_exact_evidence_against_scene_hash() {
        validate_semantic_evidence_span(&judgment().evidence.unwrap(), "scene-1", scene())
            .expect("valid evidence");
        let mut bad = judgment().evidence.unwrap();
        bad.exact_quote = "The bell rang".into();
        assert_eq!(
            validate_semantic_evidence_span(&bad, "scene-1", scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::InvalidEvidence
        );
        let mut wrong_hash = judgment().evidence.unwrap();
        wrong_hash.scene_sha256 = "not-the-hash".into();
        assert_eq!(
            validate_semantic_evidence_span(&wrong_hash, "scene-1", scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::InvalidEvidence
        );
    }

    #[test]
    fn validates_candidate_set_and_report() {
        let set = candidate_set();
        validate_semantic_candidate_set(&set, scene()).expect("valid candidate set");
        let report = SemanticJudgeReportV1 {
            schema_version: SEMANTIC_CONTRACT_VERSION.into(),
            scene_id: "scene-1".into(),
            scene_sha256: scene_sha256(scene()),
            judgments: vec![judgment()],
        };
        validate_semantic_judge_report(&report, &set, scene()).expect("valid report");
    }

    #[test]
    fn rejects_unknown_candidates_and_canon_refs() {
        let set = candidate_set();
        let mut report = SemanticJudgeReportV1 {
            schema_version: SEMANTIC_CONTRACT_VERSION.into(),
            scene_id: "scene-1".into(),
            scene_sha256: scene_sha256(scene()),
            judgments: vec![judgment()],
        };
        report.judgments[0].candidate_id = "invented".into();
        assert_eq!(
            validate_semantic_judge_report(&report, &set, scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::UnknownCandidate
        );

        report.judgments[0].candidate_id = "candidate-1".into();
        report.judgments[0].canon_refs = vec![SemanticCanonRefV1 {
            table: "character_state".into(),
            id: "state-1".into(),
            version: "invented".into(),
        }];
        assert_eq!(
            validate_semantic_judge_report(&report, &set, scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::UnknownCanonRef
        );
    }

    #[test]
    fn evidence_must_overlap_the_candidate() {
        let set = candidate_set();
        let mut report = SemanticJudgeReportV1 {
            schema_version: SEMANTIC_CONTRACT_VERSION.into(),
            scene_id: "scene-1".into(),
            scene_sha256: scene_sha256(scene()),
            judgments: vec![judgment()],
        };
        let text = scene();
        let start = text.find("Mara").unwrap();
        report.judgments[0].evidence = Some(SemanticEvidenceSpanV1 {
            scene_id: "scene-1".into(),
            scene_sha256: scene_sha256(scene()),
            start_byte: start,
            end_byte: start + "Mara".len(),
            exact_quote: "Mara".into(),
        });
        // The candidate excerpt starts at "The bell", so "Mara" is outside it,
        // but the evidence hint covers "The bell", not "Mara".
        assert_eq!(
            validate_semantic_judge_report(&report, &set, scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::InvalidEvidence
        );
    }

    #[test]
    fn severity_policy_requires_evidence_and_confidence() {
        let candidate = candidate();
        let mut judgment = judgment();
        assert_eq!(
            decide_semantic_severity(&candidate, &judgment),
            SemanticDecisionV1 {
                severity: Some(SemanticIssueSeverity::Warning),
                reason: SemanticDecisionReason::Verified,
            }
        );
        judgment.confidence = 0.5;
        assert_eq!(
            decide_semantic_severity(&candidate, &judgment).severity,
            Some(SemanticIssueSeverity::Info)
        );
        judgment.confidence = 0.9;
        judgment.evidence = None;
        assert_eq!(
            decide_semantic_severity(&candidate, &judgment).severity,
            Some(SemanticIssueSeverity::Info)
        );
        judgment.verdict = SemanticVerdict::Compatible;
        assert_eq!(
            decide_semantic_severity(&candidate, &judgment).severity,
            None
        );
    }

    #[test]
    fn dedupe_keys_are_ordered_and_overlap_aware() {
        let evidence = judgment().evidence.unwrap();
        let key_a = semantic_dedupe_key(
            "scene-1",
            "world_rule_semantic_drift",
            &[canon_ref(), canon_ref()],
            &["b".into(), "a".into()],
            Some(&evidence),
        );
        assert_eq!(key_a.canon_refs, vec![canon_ref()]);
        assert_eq!(key_a.subject_ids, vec!["a".to_string(), "b".to_string()]);

        let mut later = evidence.clone();
        later.start_byte += 4;
        later.end_byte += 4;
        let key_b = semantic_dedupe_key(
            "scene-1",
            "world_rule_semantic_drift",
            &[canon_ref()],
            &["a".into(), "b".into()],
            Some(&later),
        );
        assert!(semantic_findings_are_duplicates(&key_a, &key_b));

        let mut disjoint = evidence;
        disjoint.start_byte = 0;
        disjoint.end_byte = 1;
        let key_c = semantic_dedupe_key(
            "scene-1",
            "world_rule_semantic_drift",
            &[canon_ref()],
            &["a".into(), "b".into()],
            Some(&disjoint),
        );
        assert!(!semantic_findings_are_duplicates(&key_a, &key_c));
    }

    #[test]
    fn host_request_and_report_validate_and_reject_stale_hashes() {
        let request = SemanticHostRequestV1 {
            request_id: String::new(),
            contract_version: SEMANTIC_CONTRACT_VERSION.into(),
            candidate_set: candidate_set(),
        };
        let request = SemanticHostRequestV1 {
            request_id: semantic_host_request_id(&request.candidate_set),
            ..request
        };
        validate_semantic_host_request(&request, scene()).expect("valid host request");
        let report = SemanticHostReportV1 {
            request_id: request.request_id.clone(),
            candidate_set_hash: semantic_candidate_set_hash(&request.candidate_set),
            scene_sha256: request.candidate_set.scene_sha256.clone(),
            canon_version: request.candidate_set.canon_version.clone(),
            judgments: vec![judgment()],
        };
        validate_semantic_host_report(&report, &request, scene()).expect("valid host report");

        let mut stale = report;
        stale.candidate_set_hash = "stale".into();
        assert_eq!(
            validate_semantic_host_report(&stale, &request, scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::StaleHostRequest
        );
    }

    #[test]
    fn consistency_issue_evidence_round_trips_and_is_omitted_when_absent() {
        let evidence = judgment().evidence.unwrap();
        let issue = crate::models::ConsistencyIssue {
            severity: "warning".into(),
            check_type: "world_rule_semantic_drift".into(),
            message: "constraint contradicted".into(),
            entity_ids: Vec::new(),
            suggested_action: None,
            evidence: Some(evidence.clone()),
        };
        let value = serde_json::to_value(&issue).unwrap();
        assert_eq!(
            value["evidence"]["exact_quote"],
            serde_json::Value::String(evidence.exact_quote)
        );
        let legacy = crate::models::ConsistencyIssue {
            evidence: None,
            ..issue
        };
        let value = serde_json::to_value(&legacy).unwrap();
        assert!(value.get("evidence").is_none());
    }

    #[test]
    fn rejects_empty_canon_refs_and_missing_hints() {
        let mut set = candidate_set();
        set.candidates[0].canon_refs[0].id.clear();
        assert_eq!(
            validate_semantic_candidate_set(&set, scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::UnknownCanonRef
        );
        let mut set = candidate_set();
        set.candidates[0].evidence_hints.clear();
        assert_eq!(
            validate_semantic_candidate_set(&set, scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::MalformedOutput
        );
    }

    #[test]
    fn canon_ref_identity_is_not_colon_ambiguous() {
        let mut set = candidate_set();
        set.candidates[0].canon_refs = vec![SemanticCanonRefV1 {
            table: "canonical_fact".into(),
            id: "canonical_fact:abc".into(),
            version: "v1".into(),
        }];
        let mut report = SemanticJudgeReportV1 {
            schema_version: SEMANTIC_CONTRACT_VERSION.into(),
            scene_id: "scene-1".into(),
            scene_sha256: scene_sha256(scene()),
            judgments: vec![judgment()],
        };
        report.judgments[0].canon_refs = vec![SemanticCanonRefV1 {
            table: "canonical_fact".into(),
            id: "canonical_fact".into(),
            version: "abc:v1".into(),
        }];
        assert_eq!(
            validate_semantic_judge_report(&report, &set, scene())
                .unwrap_err()
                .kind,
            SemanticErrorKind::UnknownCanonRef
        );
    }

    #[test]
    fn dto_rejects_unknown_fields() {
        let raw = serde_json::json!({
            "candidate_id": "candidate-1",
            "verdict": "contradiction",
            "canon_refs": [],
            "confidence": 0.9,
            "rationale": "x",
            "unexpected": true
        });
        assert!(serde_json::from_value::<SemanticJudgmentV1>(raw).is_err());
    }

    #[test]
    fn family_thresholds_and_caps_match_the_design() {
        assert_eq!(
            semantic_warning_threshold(SemanticCandidateFamily::Secret),
            0.85
        );
        assert_eq!(
            semantic_warning_threshold(SemanticCandidateFamily::Attribute),
            0.80
        );
        assert_eq!(
            semantic_warning_threshold(SemanticCandidateFamily::Timeline),
            0.75
        );
        assert_eq!(SemanticCandidateFamily::Secret.default_cap(), 4);
        assert_eq!(SemanticCandidateFamily::WorldRule.default_cap(), 6);
    }

    #[test]
    fn limits_are_the_designed_ceilings() {
        let limits = SemanticContinuityLimitsV1::default();
        assert_eq!(limits.max_candidates_per_scene, 12);
        assert_eq!(limits.max_excerpt_chars, 600);
        assert_eq!(limits.max_calls_per_scene, 2);
    }
}
