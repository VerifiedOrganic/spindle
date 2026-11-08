//! Scene-critic rubric, UTF-8 byte-span anchor validation, feedback ingestion/deduplication,
//! author-lock conflict quarantine, tokenizer-budgeted prompt assembly, and shadow-only artifact persistence.

use super::{DraftMessages, DraftTokenizer, artifact};
use sha2::{Digest, Sha256};
use spindle_core::drafting::*;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

pub const CRITIC_RUBRIC_VERSION: &str = "scene-critic-v1";

/// Empirical scene-critic rubric calibrated against the 90-pair live G baseline
/// (`action = 0.3611` and `reveal = 0.4167` weakest; `chapter-closer = 0.5556` strongest).
pub const CRITIC_RUBRIC: &str = "\
Assess the draft across all ten dimensions (scene_purpose, tension_turn, pov, voice, dialogue, \
freshness, continuity, length, entry_exit_emotion, chapter_role). Scores (1..=5) are diagnostic \
only, never an automatic rewrite trigger. \
Pay special attention to empirically weak packet failure modes from the 90-pair G baseline: \
(1) Action scenes (G tie-half 0.36): flag front-loaded constraint inventories, static spatial \
cataloging, or procedural recap that stalls immediate physical causality, kinetic momentum, and \
sensory immediacy under pressure; \
(2) Reveal scenes (G tie-half 0.42): flag telegraphed turns, premature explanation before the \
dramatic beat lands, or burying the discovery in abstract summary instead of earned concrete \
perception and character reaction; \
(3) Chapter-closer scenes (G tie-half 0.56): preserve earned exit emotional resonance, forward \
propulsion, and closing image discipline without tacked-on cliffhanger clichés. \
Every directive and evidence span must anchor to exact half-open UTF-8 byte offsets \
[start_byte, end_byte) and verbatim exact_quote from the base draft (or a zero-width insertion \
with matching context_before/context_after). Select at most 5 actionable directives, prioritizing \
continuity and author intent over stylistic polish. Never propose new canon, alter planned \
outcomes/value shifts, or override author_lock constraints; place any such conflict in \
requires_author_decision.";

const CRITIC_SYSTEM: &str = "\
You are a precise developmental and line critic for long-form fiction. Evaluate the supplied \
base draft against the scene card, hard constraints, continuity packet, ingested findings, and \
critic rubric. Return ONLY a single valid JSON object matching CritiqueV1, with no markdown \
fences or commentary. All anchor offsets (start_byte, end_byte) must be exact half-open UTF-8 \
byte indices into the base draft such that base_text[start_byte..end_byte] == exact_quote. \
Never mutate or rewrite the draft itself; never invent canon or override author locks.";

/// Per-call token reservation for the scene critic (`docs/drafting-and-revision-design.md`:
/// 16,000 input / 1,500 output / 512 framing reserve).
#[derive(Debug, Clone)]
pub struct CriticBudget {
    pub model_id: String,
    pub input_limit: usize,
    pub output_reserve: usize,
    pub framing_reserve: usize,
    pub context_window: usize,
    pub generation_seed: Option<i64>,
}

impl CriticBudget {
    pub fn scene_default(model_id: String, context_window: usize) -> Self {
        Self {
            model_id,
            input_limit: 16_000,
            output_reserve: 1_500,
            framing_reserve: 512,
            context_window,
            generation_seed: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CritiqueError {
    #[error("critic_schema_mismatch: {0}")]
    SchemaMismatch(String),
    #[error("stale_anchor_hash: expected {expected}, got {actual}")]
    StaleAnchorHash { expected: String, actual: String },
    #[error("anchor_scene_mismatch: expected {expected:?}, got {actual:?}")]
    AnchorSceneMismatch {
        expected: Option<String>,
        actual: Option<String>,
    },
    #[error("invalid_anchor_span: start_byte={start} end_byte={end} text_len={text_len}")]
    InvalidAnchorSpan {
        start: usize,
        end: usize,
        text_len: usize,
    },
    #[error("non_char_boundary_anchor: start_byte={start} end_byte={end}")]
    NonCharBoundaryAnchor { start: usize, end: usize },
    #[error("anchor_quote_mismatch: bytes {start}..{end} expected {expected:?}, got {actual:?}")]
    AnchorQuoteMismatch {
        start: usize,
        end: usize,
        expected: String,
        actual: String,
    },
    #[error(
        "unanchored_insertion: zero-width anchor at byte {offset} requires matching context_before or context_after"
    )]
    UnanchoredInsertion { offset: usize },
    #[error("invalid_dimension_coverage: {0}")]
    InvalidDimensionCoverage(String),
    #[error("invalid_score: dimension {dimension} score {score} outside 1..=5")]
    InvalidScore { dimension: String, score: u8 },
    #[error("invalid_directive: {0}")]
    InvalidDirective(String),
    #[error("budget_unknown: {0}")]
    BudgetUnknown(String),
    #[error("mandatory_overflow: {0}")]
    MandatoryOverflow(String),
    #[error("malformed_critic_json: {0}")]
    MalformedJson(String),
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Validate a `CritiqueAnchor` against the exact UTF-8 bytes of `base_text`,
/// `expected_base_text_hash`, and `expected_scene_id`.
pub fn validate_anchor(
    anchor: &CritiqueAnchor,
    base_text: &str,
    expected_base_text_hash: &str,
    expected_scene_id: Option<&str>,
) -> Result<(), CritiqueError> {
    if anchor.base_text_hash != expected_base_text_hash {
        return Err(CritiqueError::StaleAnchorHash {
            expected: expected_base_text_hash.into(),
            actual: anchor.base_text_hash.clone(),
        });
    }
    if let Some(exp_scene) = expected_scene_id
        && anchor.scene_id.as_deref() != Some(exp_scene)
    {
        return Err(CritiqueError::AnchorSceneMismatch {
            expected: Some(exp_scene.into()),
            actual: anchor.scene_id.clone(),
        });
    }
    let text_len = base_text.len();
    if anchor.start_byte > anchor.end_byte || anchor.end_byte > text_len {
        return Err(CritiqueError::InvalidAnchorSpan {
            start: anchor.start_byte,
            end: anchor.end_byte,
            text_len,
        });
    }
    if !base_text.is_char_boundary(anchor.start_byte)
        || !base_text.is_char_boundary(anchor.end_byte)
    {
        return Err(CritiqueError::NonCharBoundaryAnchor {
            start: anchor.start_byte,
            end: anchor.end_byte,
        });
    }

    if anchor.start_byte == anchor.end_byte {
        if !anchor.exact_quote.is_empty() {
            return Err(CritiqueError::AnchorQuoteMismatch {
                start: anchor.start_byte,
                end: anchor.end_byte,
                expected: String::new(),
                actual: anchor.exact_quote.clone(),
            });
        }
        let before = anchor.context_before.as_deref().filter(|s| !s.is_empty());
        let after = anchor.context_after.as_deref().filter(|s| !s.is_empty());
        if before.is_none() && after.is_none() {
            return Err(CritiqueError::UnanchoredInsertion {
                offset: anchor.start_byte,
            });
        }
        if let Some(b) = before
            && !base_text[..anchor.start_byte].ends_with(b)
        {
            return Err(CritiqueError::UnanchoredInsertion {
                offset: anchor.start_byte,
            });
        }
        if let Some(a) = after
            && !base_text[anchor.start_byte..].starts_with(a)
        {
            return Err(CritiqueError::UnanchoredInsertion {
                offset: anchor.start_byte,
            });
        }
    } else {
        let actual_slice = &base_text[anchor.start_byte..anchor.end_byte];
        if anchor.exact_quote.is_empty() || actual_slice != anchor.exact_quote {
            return Err(CritiqueError::AnchorQuoteMismatch {
                start: anchor.start_byte,
                end: anchor.end_byte,
                expected: actual_slice.into(),
                actual: anchor.exact_quote.clone(),
            });
        }
    }
    Ok(())
}

fn normalize_tokens(text: &str) -> String {
    let mut tokens: Vec<String> = text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(String::from)
        .collect();
    tokens.sort();
    tokens.dedup();
    tokens.join(" ")
}

/// Compute a deterministic normalized fingerprint for a finding or directive so
/// duplicate deterministic and reader/dual-persona concerns are collapsed.
pub fn compute_finding_fingerprint(
    dimension: CriticDimension,
    anchor: Option<&CritiqueAnchor>,
    check_type: &str,
    message: &str,
) -> String {
    let span_key = anchor
        .map(|a| format!("{}:{}", a.start_byte, a.end_byte))
        .unwrap_or_else(|| "unanchored".into());
    let norm_msg = normalize_tokens(message);
    let norm_check = check_type.trim().to_lowercase();
    let payload = format!(
        "{}|{}|{}|{}",
        dimension.as_str(),
        span_key,
        norm_check,
        norm_msg
    );
    sha256_hex(payload.as_bytes())
}

/// Validate and deduplicate deterministic continuity/anti-slop findings and
/// retained reader/dual-persona feedback against `base_text`.
///
/// Returns `(deduplicated_findings, omitted_duplicate_finding_ids)`.
pub fn ingest_and_dedup_findings(
    findings: Vec<CriticFindingInput>,
    base_text: &str,
    expected_base_text_hash: &str,
    expected_scene_id: Option<&str>,
) -> Result<(Vec<IngestedFinding>, Vec<String>), CritiqueError> {
    let mut by_fp: BTreeMap<String, IngestedFinding> = BTreeMap::new();
    let mut duplicate_ids: Vec<String> = Vec::new();

    for f in findings {
        if f.finding_id.trim().is_empty() || f.message.trim().is_empty() {
            return Err(CritiqueError::InvalidDirective(
                "ingested finding requires non-empty finding_id and message".into(),
            ));
        }
        if let Some(anchor) = &f.anchor {
            validate_anchor(
                anchor,
                base_text,
                expected_base_text_hash,
                expected_scene_id,
            )?;
        }
        // Group cross-source findings that share the same dimension, byte span, and normalized issue text.
        let cross_source_fp =
            compute_finding_fingerprint(f.dimension, f.anchor.as_ref(), "", &f.message);
        if let Some(existing) = by_fp.get_mut(&cross_source_fp) {
            if !existing.merged_finding_ids.contains(&f.finding_id) {
                existing.merged_finding_ids.push(f.finding_id.clone());
                duplicate_ids.push(f.finding_id);
            }
            if !existing.sources.contains(&f.source) {
                existing.sources.push(f.source);
                existing.sources.sort();
            }
            if f.severity.rank() < existing.severity.rank() {
                existing.severity = f.severity;
            }
            if existing.suggested_fix.is_none() && f.suggested_fix.is_some() {
                existing.suggested_fix = f.suggested_fix;
            }
        } else {
            by_fp.insert(
                cross_source_fp.clone(),
                IngestedFinding {
                    canonical_id: f.finding_id.clone(),
                    merged_finding_ids: vec![f.finding_id],
                    sources: vec![f.source],
                    check_type: f.check_type,
                    dimension: f.dimension,
                    severity: f.severity,
                    message: f.message,
                    anchor: f.anchor,
                    suggested_fix: f.suggested_fix,
                    fingerprint: cross_source_fp,
                },
            );
        }
    }

    let mut deduped: Vec<IngestedFinding> = by_fp.into_values().collect();
    deduped.sort_by(|a, b| {
        a.dimension
            .priority_tier()
            .cmp(&b.dimension.priority_tier())
            .then(a.severity.rank().cmp(&b.severity.rank()))
            .then(a.canonical_id.cmp(&b.canonical_id))
    });
    Ok((deduped, duplicate_ids))
}

/// Check whether a directive conflicts with an `AuthorLock`, `prohibited_outcomes`,
/// planned `intended_outcome`/`value_shift`, or attempts to invent new canon.
/// Returns `Some((blocking_constraint_id, reason))` if the directive must be
/// quarantined into `requires_author_decision`.
fn detect_author_lock_or_canon_conflict(
    directive: &CritiqueDirective,
    packet: &DraftingPacketV1,
) -> Option<(Option<String>, String)> {
    // 0. Chapter-scoped directives violate the scene-only revision contract and must be
    // quarantined before textual keyword matching.
    if directive.scope == DirectiveScope::Chapter {
        return Some((
            Some("directive.scope:chapter".into()),
            format!(
                "directive {} has chapter scope and cannot be executed in a scene-only revision pass",
                directive.id
            ),
        ));
    }

    let combined = format!(
        "{} {}",
        directive.diagnosis.to_lowercase(),
        directive.requested_change.to_lowercase()
    );

    // 1. Check explicit AuthorLock constraints on the packet.
    for constraint in &packet.hard_constraints {
        if constraint.kind == ConstraintKind::AuthorLock {
            let cid_lower = constraint.id.to_lowercase();
            let stmt_lower = constraint.statement.to_lowercase();
            let mentions_lock_id =
                !cid_lower.is_empty() && combined.contains(&format!("lock:{cid_lower}"));
            let proposes_override = [
                "override author lock",
                "ignore author lock",
                "remove lock",
                "change locked",
                "alter locked",
                "violate lock",
            ]
            .iter()
            .any(|phrase| combined.contains(phrase));

            // Check if the directive asks to negate or replace the locked statement.
            let lock_tokens: Vec<&str> = stmt_lower
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| w.len() >= 4)
                .collect();
            let shared_lock_terms = lock_tokens
                .iter()
                .filter(|&&tok| combined.contains(tok))
                .count();
            let has_negation_verb = [
                "instead of",
                "do not",
                "don't",
                "replace",
                "remove",
                "cut",
                "reverse",
                "change outcome",
                "contrary to",
            ]
            .iter()
            .any(|v| combined.contains(v));

            if mentions_lock_id
                || proposes_override
                || (!lock_tokens.is_empty()
                    && shared_lock_terms * 2 >= lock_tokens.len().max(2)
                    && has_negation_verb)
            {
                return Some((
                    Some(constraint.id.clone()),
                    format!(
                        "directive {} conflicts with author_lock constraint '{}': {}",
                        directive.id, constraint.id, constraint.statement
                    ),
                ));
            }
        }
    }

    // 2. Check prohibited_outcomes on the scene card.
    for prohibited in &packet.scene_card.prohibited_outcomes {
        let p_lower = prohibited.trim().to_lowercase();
        if !p_lower.is_empty() && directive.requested_change.to_lowercase().contains(&p_lower) {
            return Some((
                Some("scene_card.prohibited_outcomes".into()),
                format!(
                    "directive {} requests prohibited outcome '{}'",
                    directive.id, prohibited
                ),
            ));
        }
    }

    // 3. Check attempts to create new canon or override planned intended_outcome / value_shift.
    let canon_creation_markers = [
        "create new canon",
        "invent new canon",
        "add new canon",
        "change intended_outcome",
        "change the intended outcome",
        "reverse the value shift",
        "override planned outcome",
    ];
    for marker in canon_creation_markers {
        if combined.contains(marker) {
            return Some((
                Some("scene_card.intended_outcome".into()),
                format!(
                    "directive {} attempts to alter planned outcome or create canon ({marker})",
                    directive.id
                ),
            ));
        }
    }

    None
}

/// Validate a `CritiqueV1` against `packet`, `expected_packet_hash`, `base_text`, and
/// `expected_base_text_hash`:
/// - Enforces schema version, base_text_hash, packet_hash, and rubric_version.
/// - Verifies all 10 `CriticDimension` entries are present with scores in `1..=5` and valid evidence anchors.
/// - Validates every directive's UTF-8 byte-span anchor and exact quote (or zero-width insertion context).
/// - Quarantines any directive that conflicts with `AuthorLock`, `prohibited_outcomes`, or canon/outcome rules into `requires_author_decision`.
/// - Deduplicates directives, orders by continuity/author-intent priority tier and severity, and caps actionable directives at `MAX_ACTIONABLE_DIRECTIVES` (`5`).
pub fn validate_and_normalize_critique(
    mut critique: CritiqueV1,
    packet: &DraftingPacketV1,
    expected_packet_hash: &str,
    base_text: &str,
    expected_base_text_hash: &str,
) -> Result<CritiqueV1, CritiqueError> {
    if critique.schema_version != CRITIQUE_SCHEMA_VERSION {
        return Err(CritiqueError::SchemaMismatch(format!(
            "expected schema_version '{}', got '{}'",
            CRITIQUE_SCHEMA_VERSION, critique.schema_version
        )));
    }
    if critique.base_text_hash != expected_base_text_hash {
        return Err(CritiqueError::StaleAnchorHash {
            expected: expected_base_text_hash.into(),
            actual: critique.base_text_hash,
        });
    }
    if critique.packet_hash != expected_packet_hash {
        return Err(CritiqueError::SchemaMismatch(format!(
            "expected packet_hash '{}', got '{}'",
            expected_packet_hash, critique.packet_hash
        )));
    }
    if critique.rubric_version != CRITIC_RUBRIC_VERSION {
        return Err(CritiqueError::SchemaMismatch(format!(
            "expected rubric_version '{}', got '{}'",
            CRITIC_RUBRIC_VERSION, critique.rubric_version
        )));
    }

    let expected_scene_id = packet.scene_id.as_deref();

    // Validate all 10 diagnostic dimensions.
    let mut seen_dims = BTreeSet::new();
    for dim_score in &critique.dimensions {
        if !seen_dims.insert(dim_score.dimension) {
            return Err(CritiqueError::InvalidDimensionCoverage(format!(
                "duplicate dimension '{}'",
                dim_score.dimension.as_str()
            )));
        }
        if !(1..=5).contains(&dim_score.score_1_to_5) {
            return Err(CritiqueError::InvalidScore {
                dimension: dim_score.dimension.as_str().into(),
                score: dim_score.score_1_to_5,
            });
        }
        if dim_score.rationale.trim().is_empty() {
            return Err(CritiqueError::InvalidDimensionCoverage(format!(
                "dimension '{}' requires non-empty rationale",
                dim_score.dimension.as_str()
            )));
        }
        for span in &dim_score.evidence_spans {
            validate_anchor(span, base_text, expected_base_text_hash, expected_scene_id)?;
        }
    }
    for required_dim in CriticDimension::ALL {
        if !seen_dims.contains(&required_dim) {
            return Err(CritiqueError::InvalidDimensionCoverage(format!(
                "missing required dimension '{}'",
                required_dim.as_str()
            )));
        }
    }
    critique.dimensions.sort_by_key(|d| d.dimension);

    // Validate, quarantine author-lock conflicts, deduplicate, and prioritize directives.
    let mut deduped_directives: BTreeMap<String, CritiqueDirective> = BTreeMap::new();
    for mut directive in critique.directives {
        if directive.id.trim().is_empty()
            || directive.diagnosis.trim().is_empty()
            || directive.requested_change.trim().is_empty()
            || directive.success_test.trim().is_empty()
        {
            return Err(CritiqueError::InvalidDirective(format!(
                "directive '{}' has empty required text fields",
                directive.id
            )));
        }
        if !(1..=5).contains(&directive.priority) {
            return Err(CritiqueError::InvalidDirective(format!(
                "directive '{}' priority {} outside 1..=5",
                directive.id, directive.priority
            )));
        }
        validate_anchor(
            &directive.anchor,
            base_text,
            expected_base_text_hash,
            expected_scene_id,
        )?;
        directive.source_finding_ids.sort();
        directive.source_finding_ids.dedup();

        // Check AuthorLock / prohibited_outcomes / canon creation boundaries.
        if let Some((blocking_id, reason)) =
            detect_author_lock_or_canon_conflict(&directive, packet)
        {
            critique.requires_author_decision.push(AuthorDecisionItem {
                id: format!("author_decision:{}", directive.id),
                reason,
                blocking_constraint_id: blocking_id,
                proposed_change: directive.requested_change,
                source_finding_ids: directive.source_finding_ids,
            });
            continue;
        }

        let dir_fp = compute_finding_fingerprint(
            directive.dimension,
            Some(&directive.anchor),
            "",
            &format!("{} {}", directive.diagnosis, directive.requested_change),
        );
        if let Some(existing) = deduped_directives.get_mut(&dir_fp) {
            for fid in directive.source_finding_ids {
                if !existing.source_finding_ids.contains(&fid) {
                    existing.source_finding_ids.push(fid);
                }
            }
            existing.source_finding_ids.sort();
            for p in directive.preserve {
                if !existing.preserve.contains(&p) {
                    existing.preserve.push(p);
                }
            }
            if directive.severity.rank() < existing.severity.rank() {
                existing.severity = directive.severity;
            }
            if directive.priority < existing.priority {
                existing.priority = directive.priority;
            }
        } else {
            deduped_directives.insert(dir_fp, directive);
        }
    }

    let mut actionable: Vec<CritiqueDirective> = deduped_directives.into_values().collect();
    actionable.sort_by(|a, b| {
        a.dimension
            .priority_tier()
            .cmp(&b.dimension.priority_tier())
            .then(a.severity.rank().cmp(&b.severity.rank()))
            .then(a.priority.cmp(&b.priority))
            .then(a.anchor.start_byte.cmp(&b.anchor.start_byte))
            .then(a.anchor.end_byte.cmp(&b.anchor.end_byte))
            .then(a.id.cmp(&b.id))
    });
    if actionable.len() > MAX_ACTIONABLE_DIRECTIVES {
        actionable.truncate(MAX_ACTIONABLE_DIRECTIVES);
    }
    critique.directives = actionable;

    if critique.directives.is_empty() && critique.no_edit_reason.is_none() {
        critique.no_edit_reason = Some("no_actionable_directives".into());
    } else if !critique.directives.is_empty() {
        critique.no_edit_reason = None;
    }

    Ok(critique)
}

/// Extract a JSON object from raw model output (stripping `<think>...</think>` blocks or markdown fences if present)
/// and validate/normalize it as `CritiqueV1`.
pub fn parse_and_validate_critique_output(
    raw_output: &str,
    packet: &DraftingPacketV1,
    expected_packet_hash: &str,
    base_text: &str,
    expected_base_text_hash: &str,
) -> Result<CritiqueV1, CritiqueError> {
    let stripped = strip_thinking_and_fences(raw_output);
    let parsed: CritiqueV1 =
        serde_json::from_str(stripped).map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    validate_and_normalize_critique(
        parsed,
        packet,
        expected_packet_hash,
        base_text,
        expected_base_text_hash,
    )
}

fn strip_thinking_and_fences(raw: &str) -> &str {
    let mut s = raw.trim();
    if let Some(end_think) = s.find("</think>") {
        s = s[end_think + "</think>".len()..].trim();
    }
    if let Some(rest) = s.strip_prefix("```json") {
        s = rest.trim();
    } else if let Some(rest) = s.strip_prefix("```") {
        s = rest.trim();
    }
    if let Some(rest) = s.strip_suffix("```") {
        s = rest.trim();
    }
    if let (Some(first), Some(last)) = (s.find('{'), s.rfind('}'))
        && first <= last
    {
        return &s[first..=last];
    }
    s
}

fn fits_critic_context(input_tokens: usize, budget: &CriticBudget) -> bool {
    input_tokens <= budget.input_limit
        && input_tokens
            .checked_add(budget.output_reserve)
            .and_then(|n| n.checked_add(budget.framing_reserve))
            .is_some_and(|n| n <= budget.context_window)
}

/// Generate the canonical JSON Schema for `CritiqueV1` directly from the Rust DTO.
pub fn critique_v1_json_schema() -> serde_json::Value {
    CritiqueV1::json_schema()
}

/// Build a complete `CritiqueV1` JSON template populated with the required hashes,
/// rubric version, all 10 `CriticDimension` entries, and exemplar `CritiqueDirective`
/// and `AuthorDecisionItem` shapes so the model has the exact required structure.
pub fn critique_v1_output_template(
    expected_packet_hash: &str,
    expected_base_text_hash: &str,
    scene_id: Option<&str>,
) -> serde_json::Value {
    let dimensions: Vec<serde_json::Value> = CriticDimension::ALL
        .iter()
        .map(|dim| {
            serde_json::json!({
                "dimension": dim.as_str(),
                "score_1_to_5": 4,
                "evidence_spans": [],
                "rationale": format!("Assessment of {}.", dim.as_str()),
            })
        })
        .collect();
    serde_json::json!({
        "schema_version": CRITIQUE_SCHEMA_VERSION,
        "base_text_hash": expected_base_text_hash,
        "packet_hash": expected_packet_hash,
        "rubric_version": CRITIC_RUBRIC_VERSION,
        "dimensions": dimensions,
        "directives": [
            {
                "id": "dir-1",
                "dimension": "tension_turn",
                "severity": "major",
                "priority": 1,
                "anchor": {
                    "scene_id": scene_id,
                    "base_text_hash": expected_base_text_hash,
                    "start_byte": 0,
                    "end_byte": 1,
                    "exact_quote": "<exact UTF-8 slice base_text[start_byte..end_byte]>",
                    "context_before": null,
                    "context_after": null
                },
                "diagnosis": "Concrete diagnosis of the anchored span.",
                "requested_change": "Specific revision instruction for the anchored span.",
                "preserve": ["Element to keep intact"],
                "success_test": "Observable check that the revision solved the diagnosis.",
                "source_finding_ids": [],
                "scope": "span"
            }
        ],
        "strengths_to_preserve": [
            "Specific strength in the base draft to preserve."
        ],
        "requires_author_decision": [
            {
                "id": "decision-1",
                "reason": "Why this item requires author judgment (e.g. author_lock or canon conflict).",
                "blocking_constraint_id": "lock-id-if-applicable",
                "proposed_change": "Optional description of the quarantined suggestion.",
                "source_finding_ids": []
            }
        ],
        "no_edit_reason": null
    })
}

/// Assemble and token-verify the initial scene-critic `DraftMessages` for the `review` route.
#[allow(clippy::too_many_arguments)]
pub fn assemble_critic_messages(
    packet: &DraftingPacketV1,
    expected_packet_hash: &str,
    base_text: &str,
    expected_base_text_hash: &str,
    ingested_findings: &[IngestedFinding],
    budget: &CriticBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
    route_system: &str,
) -> Result<(DraftMessages, String, usize), CritiqueError> {
    let t =
        tokenizer.ok_or_else(|| CritiqueError::BudgetUnknown("no tokenizer configured".into()))?;
    if t.id().is_empty() || t.version().is_empty() || !t.supports_model(&budget.model_id) {
        return Err(CritiqueError::BudgetUnknown(
            "tokenizer/model mismatch".into(),
        ));
    }

    let system = if route_system.is_empty() {
        CRITIC_SYSTEM.to_string()
    } else {
        format!("{route_system}\n\n{CRITIC_SYSTEM}")
    };

    let scene_card_json = serde_json::to_string(&packet.scene_card)
        .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    let constraints_json = serde_json::to_string(&packet.hard_constraints)
        .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    let knowledge_boundaries_json = serde_json::to_string(&packet.continuity.knowledge_boundaries)
        .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    let findings_json = serde_json::to_string(ingested_findings)
        .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    let output_template_json = serde_json::to_string(&critique_v1_output_template(
        expected_packet_hash,
        expected_base_text_hash,
        packet.scene_id.as_deref(),
    ))
    .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    let schema_json = serde_json::to_string(&critique_v1_json_schema())
        .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;

    let render_critic_user = |continuity_sec: &str,
                              voice_sec: &str,
                              editorial_sec: &str|
     -> String {
        format!(
            "CRITIC CONTRACT\n\
                 schema_version: {CRITIQUE_SCHEMA_VERSION}\n\
                 rubric_version: {CRITIC_RUBRIC_VERSION}\n\
                 packet_hash: {expected_packet_hash}\n\
                 base_text_hash: {expected_base_text_hash}\n\
                 scene_id: {:?}\n\
                 base_text_utf8_byte_len: {}\n\n\
                 CRITIQUE_V1 FIELD & ENUM REQUIREMENTS\n\
                 - Top-level required fields: schema_version (\"critique_v1\"), base_text_hash, packet_hash, rubric_version (\"scene-critic-v1\"), dimensions (array of all 10 CriticDimensionScore objects), directives (0..=5 CritiqueDirective objects), strengths_to_preserve (array of strings), requires_author_decision (array of AuthorDecisionItem objects), no_edit_reason (string or null; required when directives is empty).\n\
                 - dimensions[] required fields: dimension (one of: scene_purpose | tension_turn | pov | voice | dialogue | freshness | continuity | length | entry_exit_emotion | chapter_role), score_1_to_5 (integer 1..=5), evidence_spans (array of CritiqueAnchor), rationale (non-empty string).\n\
                 - directives[] required fields: id (non-empty string), dimension (CriticDimension enum), severity (one of: blocker | major | minor | polish), priority (integer 1..=5), anchor (CritiqueAnchor object), diagnosis (non-empty string), requested_change (non-empty string), preserve (array of strings), success_test (non-empty string), source_finding_ids (array of strings), scope (one of: span | scene | chapter).\n\
                 - CritiqueAnchor required fields: scene_id (matching scene_id above), base_text_hash (matching base_text_hash above), start_byte (usize), end_byte (usize), exact_quote (verbatim UTF-8 substring base_text[start_byte..end_byte]; empty string only when start_byte == end_byte), context_before (optional string; required if start_byte == end_byte and context_after is null), context_after (optional string; required if start_byte == end_byte and context_before is null).\n\
                 - requires_author_decision[] fields: id, reason, blocking_constraint_id (string or null), proposed_change, source_finding_ids (array of strings).\n\n\
                 CRITIQUE_V1 OUTPUT TEMPLATE\n\
                 {output_template_json}\n\n\
                 CRITIQUE_V1 JSON SCHEMA\n\
                 {schema_json}\n\n\
                 CRITIC RUBRIC {CRITIC_RUBRIC_VERSION}\n\
                 {CRITIC_RUBRIC}\n\n\
                 SCENE CARD\n\
                 {scene_card_json}\n\n\
                 HARD CONSTRAINTS\n\
                 {constraints_json}\n\n\
                 KNOWLEDGE BOUNDARIES (MANDATORY)\n\
                 {knowledge_boundaries_json}\n\n\
                 CONTINUITY CONTEXT\n\
                 {continuity_sec}\n\
                 VOICE & STYLE CONTEXT\n\
                 {voice_sec}\n\
                 EDITORIAL MEMORY & STORY CONTEXT\n\
                 {editorial_sec}\n\
                 INGESTED FINDINGS (DEDUPLICATED)\n\
                 {findings_json}\n\n\
                 EXACT BASE DRAFT UTF-8\n\
                 {base_text}\n",
            packet.scene_id,
            base_text.len()
        )
    };

    // 1. Verify mandatory context (contract, schema, rubric, scene_card, hard_constraints,
    // knowledge_boundaries, ingested_findings, and base_text) fits within the critic budget.
    let mandatory_messages = DraftMessages {
        system: system.clone(),
        user: render_critic_user("", "", ""),
    };
    let mandatory_tokens = t
        .count_messages(&mandatory_messages)
        .map_err(CritiqueError::BudgetUnknown)?;
    if !fits_critic_context(mandatory_tokens, budget) {
        return Err(CritiqueError::MandatoryOverflow(
            "mandatory critic context (scene card, hard constraints, knowledge boundaries, findings, schema, and base draft) exceeds critic budget".into(),
        ));
    }

    // 2. Render compact packet sections (continuity, voice, and editorial memory) in priority
    // order under the remaining critic token budget.
    let c = &packet.continuity;
    let m = &packet.editorial_memory;
    let mut tier_continuity: Vec<(&str, PacketItem)> = Vec::new();
    for (label, items) in [
        ("character state", &c.character_states),
        ("fact", &c.facts),
        ("open thread", &c.open_threads),
    ] {
        for item in items {
            tier_continuity.push((label, item.clone()));
        }
    }
    if let Some(tail) = &c.prior_scene_tail {
        tier_continuity.push(("prior scene tail", tail.clone()));
    }

    let mut tier_voice: Vec<(&str, PacketItem)> = Vec::new();
    for item in &packet.voice.rules {
        tier_voice.push(("voice rule", item.clone()));
    }
    for exemplar in &packet.voice.exemplars {
        let mut item = exemplar.item.clone();
        item.text = serde_json::to_string(exemplar)
            .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
        tier_voice.push(("approved prose example (not canon)", item));
    }

    let mut tier_editorial: Vec<(&str, PacketItem)> = Vec::new();
    for (label, items) in [
        ("story so far", &c.story_so_far),
        ("recent opening", &m.recent_openings),
        ("recent ending", &m.recent_endings),
        ("recurring image", &m.recurring_images),
        ("repeated phrase", &m.repeated_phrases),
        ("pacing history", &m.pacing_history),
        ("research", &packet.research_evidence),
    ] {
        for item in items {
            tier_editorial.push((label, item.clone()));
        }
    }

    let sort_tier = |group: &mut Vec<(&str, PacketItem)>| {
        group.sort_by(|(la, a), (lb, b)| {
            b.relevance
                .cmp(&a.relevance)
                .then(b.recency.cmp(&a.recency))
                .then(a.source.cmp(&b.source))
                .then(a.proposition_id.cmp(&b.proposition_id))
                .then(la.cmp(lb))
                .then(a.text.cmp(&b.text))
        });
    };
    sort_tier(&mut tier_continuity);
    sort_tier(&mut tier_voice);
    sort_tier(&mut tier_editorial);

    let mut seen_propositions: BTreeSet<(PacketSource, String)> = BTreeSet::new();
    for kb in &c.knowledge_boundaries {
        seen_propositions.insert((kb.source.clone(), kb.proposition_id.clone()));
    }

    let mut continuity_sec = String::new();
    let mut voice_sec = String::new();
    let mut editorial_sec = String::new();

    for (tier_idx, group) in [tier_continuity, tier_voice, tier_editorial]
        .into_iter()
        .enumerate()
    {
        for (label, item) in group {
            let prop_key = (item.source.clone(), item.proposition_id.clone());
            if seen_propositions.contains(&prop_key) {
                continue;
            }
            let item_json = serde_json::to_string(&item)
                .map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
            let line = format!("{label}: {item_json}\n");
            let (prop_cont, prop_voice, prop_edit) = match tier_idx {
                0 => (
                    format!("{continuity_sec}{line}"),
                    voice_sec.clone(),
                    editorial_sec.clone(),
                ),
                1 => (
                    continuity_sec.clone(),
                    format!("{voice_sec}{line}"),
                    editorial_sec.clone(),
                ),
                _ => (
                    continuity_sec.clone(),
                    voice_sec.clone(),
                    format!("{editorial_sec}{line}"),
                ),
            };
            let proposed_messages = DraftMessages {
                system: system.clone(),
                user: render_critic_user(&prop_cont, &prop_voice, &prop_edit),
            };
            let proposed_tokens = t
                .count_messages(&proposed_messages)
                .map_err(CritiqueError::BudgetUnknown)?;
            if fits_critic_context(proposed_tokens, budget) {
                seen_propositions.insert(prop_key);
                match tier_idx {
                    0 => continuity_sec = prop_cont,
                    1 => voice_sec = prop_voice,
                    _ => editorial_sec = prop_edit,
                }
            }
        }
    }

    let user = render_critic_user(&continuity_sec, &voice_sec, &editorial_sec);
    let messages = DraftMessages { system, user };
    let input_tokens = t
        .count_messages(&messages)
        .map_err(CritiqueError::BudgetUnknown)?;
    if !fits_critic_context(input_tokens, budget) {
        return Err(CritiqueError::MandatoryOverflow(
            "critic prompt exceeds context window or input budget".into(),
        ));
    }
    let msg_bytes =
        serde_json::to_vec(&messages).map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    let messages_hash = sha256_hex(&msg_bytes);
    Ok((messages, messages_hash, input_tokens))
}

/// Assemble and token-verify the bounded one-shot schema/anchor repair prompt when
/// the initial critic output is malformed or fails anchor/schema validation.
pub fn assemble_critic_repair_messages(
    initial_messages: &DraftMessages,
    invalid_raw_output: &str,
    validation_error: &CritiqueError,
    budget: &CriticBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
) -> Result<(DraftMessages, String, usize), CritiqueError> {
    let t =
        tokenizer.ok_or_else(|| CritiqueError::BudgetUnknown("no tokenizer configured".into()))?;
    if t.id().is_empty() || t.version().is_empty() || !t.supports_model(&budget.model_id) {
        return Err(CritiqueError::BudgetUnknown(
            "tokenizer/model mismatch".into(),
        ));
    }

    let user = format!(
        "{}\n\n\
         SCHEMA / ANCHOR REPAIR REQUIRED\n\
         Your previous output failed CritiqueV1 validation with error:\n\
         {validation_error}\n\n\
         Previous invalid output:\n\
         {invalid_raw_output}\n\n\
         Return a corrected CritiqueV1 JSON object conforming to CRITIQUE_V1 FIELD & ENUM \
         REQUIREMENTS, CRITIQUE_V1 OUTPUT TEMPLATE, and CRITIQUE_V1 JSON SCHEMA above, where \
         every anchor [start_byte, end_byte) is an exact UTF-8 byte slice of the base draft \
         matching exact_quote, all 10 dimensions are present with score_1_to_5 in 1..=5 and \
         non-empty rationale, every directive includes id/dimension/severity/priority/anchor/\
         diagnosis/requested_change/preserve/success_test/source_finding_ids/scope, and \
         base_text_hash/packet_hash/rubric_version match the contract values above.",
        initial_messages.user
    );
    let messages = DraftMessages {
        system: initial_messages.system.clone(),
        user,
    };
    let input_tokens = t
        .count_messages(&messages)
        .map_err(CritiqueError::BudgetUnknown)?;
    if !fits_critic_context(input_tokens, budget) {
        return Err(CritiqueError::MandatoryOverflow(
            "critic repair prompt exceeds context window or input budget".into(),
        ));
    }
    let msg_bytes =
        serde_json::to_vec(&messages).map_err(|e| CritiqueError::MalformedJson(e.to_string()))?;
    let messages_hash = sha256_hex(&msg_bytes);
    Ok((messages, messages_hash, input_tokens))
}

/// Validate a `ShadowCritiqueArtifactV1` before saving or after loading.
pub fn validate_shadow_critique_artifact(
    artifact: &ShadowCritiqueArtifactV1,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        artifact.schema_version == SHADOW_CRITIQUE_ARTIFACT_SCHEMA_VERSION,
        "unknown shadow critique artifact version"
    );
    anyhow::ensure!(
        !artifact.base_text_hash.is_empty() && !artifact.packet_hash.is_empty(),
        "shadow critique requires base_text_hash and packet_hash"
    );
    anyhow::ensure!(
        artifact.rubric_version == CRITIC_RUBRIC_VERSION,
        "shadow critique rubric_version mismatch"
    );
    if let ShadowCritiqueOutcome::Critiqued { critique, .. } = &artifact.outcome {
        anyhow::ensure!(
            critique.base_text_hash == artifact.base_text_hash
                && critique.packet_hash == artifact.packet_hash
                && critique.rubric_version == artifact.rubric_version,
            "critique outcome hash/version mismatch"
        );
        anyhow::ensure!(
            critique.directives.len() <= MAX_ACTIONABLE_DIRECTIVES,
            "critique exceeds max actionable directives"
        );
    }
    Ok(())
}

/// Persist a `ShadowCritiqueArtifactV1` atomically in the local artifact layer.
pub fn save_shadow_critique_artifact(
    path: &Path,
    artifact: &ShadowCritiqueArtifactV1,
) -> anyhow::Result<()> {
    validate_shadow_critique_artifact(artifact)?;
    let _lock = artifact::lock_artifact(path)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temporary.as_file_mut(), artifact)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

/// Load and validate a `ShadowCritiqueArtifactV1` from disk.
pub fn load_shadow_critique_artifact(path: &Path) -> anyhow::Result<ShadowCritiqueArtifactV1> {
    let artifact: ShadowCritiqueArtifactV1 = serde_json::from_reader(std::fs::File::open(path)?)?;
    validate_shadow_critique_artifact(&artifact)?;
    Ok(artifact)
}
