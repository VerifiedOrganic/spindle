//! Prior-draft revision prompt assembly, deterministic candidate checks,
//! paragraph/byte edit-span diffing, blinded double-order pairwise acceptance
//! judging, and revision-loop artifact persistence.

use super::{
    DraftMessages, DraftTokenizer,
    critique::{CritiqueError, compute_finding_fingerprint, ingest_and_dedup_findings, sha256_hex},
};
use spindle_core::drafting::*;
use spindle_core::models::ContentRating;
use spindle_core::style::antislop::{ScanInput, Severity as AntiSlopSeverity, ShelfPack, scan};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::Path;

pub const REVISION_SYSTEM: &str = "\
You are Spindle's scene-revision editor. Revise the supplied prior draft by applying ONLY the \
selected actionable directives while preserving unaffected passages and the listed strengths_to_preserve. \
Obey the scene card, hard constraints, POV knowledge boundaries, rating, and voice rules. \
Never invent new canon, never alter planned outcomes or value shifts, and never remove or contradict \
author locks. Return ONLY plain UTF-8 scene prose, without markdown fences, JSON, headings, or commentary.";

pub const PAIRWISE_JUDGE_SYSTEM: &str = "\
You are a senior fiction developmental editor evaluating two anonymous drafts of the same scene. \
Judge the prose strictly against the supplied scene card, hard constraints, POV knowledge boundaries, \
continuity context, and voice rules/exemplars. Score the comparative quality of the pair across all \
seven rubric dimensions (1-5 integer, or \"N/A\" only for dialogue in a silent scene), cite concrete \
 textual evidence in each rationale, and state a clear overall preference (\"passage_1\", \"passage_2\", \
or \"tie\"). Return ONLY a single valid JSON object matching PairwiseJudgeV1, with no markdown fences or commentary.";

/// Per-call token reservation for prior-draft revision (`docs/drafting-and-revision-design.md`:
/// 18,000 input / 3,000 output / 512 framing reserve).
#[derive(Debug, Clone)]
pub struct RevisionBudget {
    pub model_id: String,
    pub input_limit: usize,
    pub output_reserve: usize,
    pub framing_reserve: usize,
    pub context_window: usize,
    pub generation_seed: Option<i64>,
}

impl RevisionBudget {
    pub fn scene_default(model_id: String, context_window: usize) -> Self {
        Self {
            model_id,
            input_limit: 18_000,
            output_reserve: 3_000,
            framing_reserve: 512,
            context_window,
            generation_seed: None,
        }
    }
}

/// Per-call token reservation for the blinded pairwise acceptance judge
/// (`docs/drafting-and-revision-design.md`: 16,000 input / 1,000 output / 512 framing reserve).
#[derive(Debug, Clone)]
pub struct JudgeBudget {
    pub model_id: String,
    pub input_limit: usize,
    pub output_reserve: usize,
    pub framing_reserve: usize,
    pub context_window: usize,
    pub generation_seed: Option<i64>,
}

impl JudgeBudget {
    pub fn scene_default(model_id: String, context_window: usize) -> Self {
        Self {
            model_id,
            input_limit: 16_000,
            output_reserve: 1_000,
            framing_reserve: 512,
            context_window,
            generation_seed: None,
        }
    }
}

/// Optional boundary failpoint for deterministic crash/recovery testing at every
/// stage boundary of the revision loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryCrashPoint {
    Stage(EditorialStage, u32),
    JudgePass1(u32),
}

/// Configuration options for the scene revision and double-order judge loop.
#[derive(Debug, Clone)]
pub struct RevisionLoopOptions {
    pub max_rounds: u32,
    pub max_scene_calls: u32,
    pub max_scene_tokens: usize,
    pub max_critic_calls_per_round: u32,
    pub crash_after: Option<BoundaryCrashPoint>,
}

impl Default for RevisionLoopOptions {
    fn default() -> Self {
        Self {
            max_rounds: DEFAULT_MAX_REVISION_ROUNDS,
            max_scene_calls: DEFAULT_MAX_SCENE_CALLS,
            max_scene_tokens: HARD_MAX_SCENE_TOKENS,
            max_critic_calls_per_round: 2,
            crash_after: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RevisionError {
    #[error("budget_unknown: {0}")]
    BudgetUnknown(String),
    #[error("mandatory_overflow: {0}")]
    MandatoryOverflow(String),
    #[error("malformed_json: {0}")]
    MalformedJson(String),
    #[error("invalid_judge_output: {0}")]
    InvalidJudgeOutput(String),
    #[error("stale_cas_conflict: {0}")]
    StaleCasConflict(String),
    #[error("invalid_revision_loop_artifact: {0}")]
    InvalidLoopArtifact(String),
    #[error(transparent)]
    Critique(#[from] CritiqueError),
}

fn fits_budget(
    input_tokens: usize,
    input_limit: usize,
    output_reserve: usize,
    framing_reserve: usize,
    context_window: usize,
) -> bool {
    input_tokens <= input_limit
        && input_tokens
            .checked_add(output_reserve)
            .and_then(|n| n.checked_add(framing_reserve))
            .is_some_and(|n| n <= context_window)
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

/// Extract quoted literal spans (`"..."`) of length >= 4 from a constraint statement.
fn extract_quoted_literals(statement: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = statement;
    while let Some(open) = rest.find('"') {
        let after_open = &rest[open + 1..];
        if let Some(close) = after_open.find('"') {
            let quoted = after_open[..close].trim();
            if quoted.len() >= 4 {
                out.push(quoted.to_string());
            }
            rest = &after_open[close + 1..];
        } else {
            break;
        }
    }
    out
}

/// Run deterministic continuity, rating, and anti-slop checks on an incumbent
/// or candidate draft, returning a `DeterministicCandidateReport` with hard
/// blockers, soft anti-slop findings, deduplicated `IngestedFinding` records,
/// and a canonical `normalized_finding_fingerprint`.
pub fn evaluate_deterministic_checks(
    packet: &DraftingPacketV1,
    text: &str,
    truncated: bool,
    external_findings: Vec<CriticFindingInput>,
) -> Result<DeterministicCandidateReport, RevisionError> {
    let text_sha256 = sha256_hex(text.as_bytes());
    let text_lower = text.to_lowercase();
    let mut hard_continuity_blockers = Vec::new();
    let mut hard_rating_violations = Vec::new();
    let mut hard_antislop_blockers = Vec::new();
    let mut soft_antislop_findings = Vec::new();
    let mut combined_findings = external_findings;

    // 1. Hard continuity checks:
    // 1a. AuthorLock literal quotes (`"..."`) or explicit `must include: <phrase>` requirements.
    for constraint in &packet.hard_constraints {
        if constraint.kind == ConstraintKind::AuthorLock {
            for literal in extract_quoted_literals(&constraint.statement) {
                if !text_lower.contains(&literal.to_lowercase()) {
                    let msg = format!(
                        "author_lock '{}' required literal {:?} missing from draft",
                        constraint.id, literal
                    );
                    hard_continuity_blockers
                        .push(format!("author_lock:{}:{literal}", constraint.id));
                    combined_findings.push(CriticFindingInput {
                        finding_id: format!("det:author_lock:{}", constraint.id),
                        source: FeedbackSourceKind::Continuity,
                        check_type: "author_lock_missing".into(),
                        dimension: CriticDimension::Continuity,
                        severity: DirectiveSeverity::Blocker,
                        message: msg,
                        anchor: None,
                        suggested_fix: Some(format!("Restore locked phrase {:?}.", literal)),
                    });
                }
            }
        }
    }

    // 1b. Prohibited outcomes from scene_card.prohibited_outcomes.
    for outcome in &packet.scene_card.prohibited_outcomes {
        let trimmed = outcome.trim();
        if trimmed.len() >= 4 && text_lower.contains(&trimmed.to_lowercase()) {
            let blocker_id = format!("prohibited_outcome:{}", trimmed.to_lowercase());
            hard_continuity_blockers.push(blocker_id.clone());
            combined_findings.push(CriticFindingInput {
                finding_id: format!("det:{blocker_id}"),
                source: FeedbackSourceKind::Continuity,
                check_type: "prohibited_outcome".into(),
                dimension: CriticDimension::Continuity,
                severity: DirectiveSeverity::Blocker,
                message: format!("Draft includes prohibited outcome {trimmed:?}."),
                anchor: None,
                suggested_fix: Some(format!("Remove prohibited outcome {trimmed:?}.")),
            });
        }
    }

    // 1c. Knowledge boundary forbidden leaks (`forbidden leak:` or `withheld:` quoted literals).
    for kb in &packet.continuity.knowledge_boundaries {
        let kb_lower = kb.text.to_lowercase();
        if kb_lower.contains("forbidden leak")
            || kb_lower.contains("must not know")
            || kb_lower.contains("withheld:")
        {
            for literal in extract_quoted_literals(&kb.text) {
                if text_lower.contains(&literal.to_lowercase()) {
                    let blocker_id = format!(
                        "knowledge_boundary_leak:{}:{}",
                        kb.proposition_id,
                        literal.to_lowercase()
                    );
                    hard_continuity_blockers.push(blocker_id.clone());
                    combined_findings.push(CriticFindingInput {
                        finding_id: format!("det:{blocker_id}"),
                        source: FeedbackSourceKind::Continuity,
                        check_type: "knowledge_boundary_leak".into(),
                        dimension: CriticDimension::Pov,
                        severity: DirectiveSeverity::Blocker,
                        message: format!(
                            "Draft leaks withheld knowledge {:?} (knowledge boundary {}).",
                            literal, kb.proposition_id
                        ),
                        anchor: None,
                        suggested_fix: Some(format!(
                            "Remove leaked knowledge {:?} outside POV boundary.",
                            literal
                        )),
                    });
                }
            }
        }
    }

    // 2. Hard rating compliance check for non-mature ratings (General, Teen).
    if matches!(
        packet.scene_card.rating,
        ContentRating::General | ContentRating::Teen
    ) {
        for marker in [
            "[explicit_nc17_violation]",
            "explicit sexual act",
            "graphic disembowelment",
        ] {
            if text_lower.contains(marker) {
                let violation = format!(
                    "rating_violation:{}:{marker}",
                    packet.scene_card.rating.as_str()
                );
                hard_rating_violations.push(violation.clone());
                combined_findings.push(CriticFindingInput {
                    finding_id: format!("det:{violation}"),
                    source: FeedbackSourceKind::Continuity,
                    check_type: "rating_violation".into(),
                    dimension: CriticDimension::Continuity,
                    severity: DirectiveSeverity::Blocker,
                    message: format!(
                        "Draft contains content marker {marker:?} exceeding scene rating {}.",
                        packet.scene_card.rating.as_str()
                    ),
                    anchor: None,
                    suggested_fix: Some("Remove content exceeding the scene rating.".into()),
                });
            }
        }
    }

    // 3. Anti-slop scanner: hard shelves over limit -> hard blockers; soft shelves -> critic findings.
    let pack =
        ShelfPack::load_default().map_err(|e| RevisionError::InvalidLoopArtifact(e.to_string()))?;
    let antislop_report = scan(&pack, &ScanInput::fiction(text));
    for (idx, hit) in antislop_report.hits.into_iter().enumerate() {
        let start = hit.byte_range.start;
        let end = hit.byte_range.end;
        let anchor = if start <= end
            && end <= text.len()
            && text.is_char_boundary(start)
            && text.is_char_boundary(end)
            && start < end
        {
            Some(CritiqueAnchor {
                scene_id: packet.scene_id.clone(),
                base_text_hash: text_sha256.clone(),
                start_byte: start,
                end_byte: end,
                exact_quote: text[start..end].to_string(),
                context_before: None,
                context_after: None,
            })
        } else {
            None
        };
        match hit.severity {
            AntiSlopSeverity::Hard => {
                let blocker_key = format!(
                    "antislop_hard:{}:{}",
                    hit.shelf_id,
                    hit.excerpt.trim().to_lowercase()
                );
                hard_antislop_blockers.push(blocker_key);
                combined_findings.push(CriticFindingInput {
                    finding_id: format!("det:antislop_hard:{}:{idx}", hit.shelf_id),
                    source: FeedbackSourceKind::AntiSlop,
                    check_type: hit.shelf_id,
                    dimension: CriticDimension::Freshness,
                    severity: DirectiveSeverity::Blocker,
                    message: format!("Hard anti-slop shelf exceeded on {:?}.", hit.excerpt),
                    anchor,
                    suggested_fix: Some(hit.rewrite_hint),
                });
            }
            AntiSlopSeverity::Soft => {
                let soft_finding = CriticFindingInput {
                    finding_id: format!("det:antislop_soft:{}:{idx}", hit.shelf_id),
                    source: FeedbackSourceKind::AntiSlop,
                    check_type: hit.shelf_id,
                    dimension: CriticDimension::Freshness,
                    severity: DirectiveSeverity::Minor,
                    message: format!("Soft anti-slop advisory on {:?}.", hit.excerpt),
                    anchor,
                    suggested_fix: Some(hit.rewrite_hint),
                };
                soft_antislop_findings.push(soft_finding.clone());
                combined_findings.push(soft_finding);
            }
        }
    }

    hard_continuity_blockers.sort();
    hard_continuity_blockers.dedup();
    hard_rating_violations.sort();
    hard_rating_violations.dedup();
    hard_antislop_blockers.sort();
    hard_antislop_blockers.dedup();

    let (ingested_findings, _) = ingest_and_dedup_findings(
        combined_findings,
        text,
        &text_sha256,
        packet.scene_id.as_deref(),
    )?;

    // Build canonical normalized finding fingerprint across issue types + normalized messages
    // (independent of shifting byte offsets so repeated unresolved issue sets across rounds match).
    let mut issue_keys: Vec<String> = ingested_findings
        .iter()
        .map(|f| compute_finding_fingerprint(f.dimension, None, &f.check_type, &f.message))
        .collect();
    for b in &hard_continuity_blockers {
        issue_keys.push(format!("cont:{b}"));
    }
    for b in &hard_rating_violations {
        issue_keys.push(format!("rating:{b}"));
    }
    for b in &hard_antislop_blockers {
        issue_keys.push(format!("slop:{b}"));
    }
    issue_keys.sort();
    issue_keys.dedup();
    let normalized_finding_fingerprint = sha256_hex(issue_keys.join("|").as_bytes());

    Ok(DeterministicCandidateReport {
        text_sha256,
        truncated,
        hard_continuity_blockers,
        hard_rating_violations,
        hard_antislop_blockers,
        soft_antislop_findings,
        ingested_findings,
        normalized_finding_fingerprint,
    })
}

/// Compare the candidate draft's deterministic report against the incumbent draft's
/// report (`docs/drafting-and-revision-design.md`):
/// - Disqualifies if `candidate.truncated` is true.
/// - Disqualifies if `candidate` introduces any new hard continuity, rating, or anti-slop blocker
///   (or increases the count of hard blockers).
/// - Also checks whether any `AuthorLock` constraint statement present verbatim in `incumbent_text`
///   was dropped from `candidate_text`.
pub fn compare_candidate_checks(
    packet: &DraftingPacketV1,
    incumbent_text: &str,
    incumbent_report: &DeterministicCandidateReport,
    candidate_text: &str,
    mut candidate_report: DeterministicCandidateReport,
) -> CandidateCheckComparison {
    let inc_lower = incumbent_text.to_lowercase();
    let cand_lower = candidate_text.to_lowercase();

    // Check if any AuthorLock statement that appeared in incumbent_text was removed in candidate_text.
    for constraint in &packet.hard_constraints {
        if constraint.kind == ConstraintKind::AuthorLock {
            let stmt_trimmed = constraint.statement.trim();
            if stmt_trimmed.len() >= 4
                && inc_lower.contains(&stmt_trimmed.to_lowercase())
                && !cand_lower.contains(&stmt_trimmed.to_lowercase())
            {
                let blocker = format!("author_lock_dropped:{}", constraint.id);
                if !candidate_report.hard_continuity_blockers.contains(&blocker) {
                    candidate_report.hard_continuity_blockers.push(blocker);
                    candidate_report.hard_continuity_blockers.sort();
                }
            }
        }
    }

    let inc_cont: BTreeSet<&String> = incumbent_report.hard_continuity_blockers.iter().collect();
    let new_hard_continuity_blockers: Vec<String> = candidate_report
        .hard_continuity_blockers
        .iter()
        .filter(|b| !inc_cont.contains(b))
        .cloned()
        .collect();

    let inc_rating: BTreeSet<&String> = incumbent_report.hard_rating_violations.iter().collect();
    let new_hard_rating_violations: Vec<String> = candidate_report
        .hard_rating_violations
        .iter()
        .filter(|b| !inc_rating.contains(b))
        .cloned()
        .collect();

    let inc_slop: BTreeSet<&String> = incumbent_report.hard_antislop_blockers.iter().collect();
    let mut new_hard_antislop_blockers: Vec<String> = candidate_report
        .hard_antislop_blockers
        .iter()
        .filter(|b| !inc_slop.contains(b))
        .cloned()
        .collect();
    if new_hard_antislop_blockers.is_empty()
        && candidate_report.hard_antislop_blockers.len()
            > incumbent_report.hard_antislop_blockers.len()
    {
        new_hard_antislop_blockers.push("hard_antislop_count_increased".into());
    }

    let mut remaining_hard_blockers = Vec::new();
    remaining_hard_blockers.extend(candidate_report.hard_continuity_blockers.iter().cloned());
    remaining_hard_blockers.extend(candidate_report.hard_rating_violations.iter().cloned());
    remaining_hard_blockers.extend(candidate_report.hard_antislop_blockers.iter().cloned());

    // Measure how many incumbent findings (by dimension + check_type + normalized message) were resolved.
    let cand_issue_set: BTreeSet<String> = candidate_report
        .ingested_findings
        .iter()
        .map(|f| compute_finding_fingerprint(f.dimension, None, &f.check_type, &f.message))
        .collect();
    let resolved_finding_count = incumbent_report
        .ingested_findings
        .iter()
        .filter(|f| {
            let key = compute_finding_fingerprint(f.dimension, None, &f.check_type, &f.message);
            !cand_issue_set.contains(&key)
        })
        .count();

    let mut reasons = Vec::new();
    if candidate_report.truncated {
        reasons.push("candidate_truncated".to_string());
    }
    if !new_hard_continuity_blockers.is_empty() {
        reasons.push(format!(
            "new_hard_continuity_blockers: {}",
            new_hard_continuity_blockers.join(", ")
        ));
    }
    if !new_hard_rating_violations.is_empty() {
        reasons.push(format!(
            "new_hard_rating_violations: {}",
            new_hard_rating_violations.join(", ")
        ));
    }
    if !new_hard_antislop_blockers.is_empty() {
        reasons.push(format!(
            "new_hard_antislop_blockers: {}",
            new_hard_antislop_blockers.join(", ")
        ));
    }
    let disqualified = !reasons.is_empty();
    let disqualification_reason = if disqualified {
        Some(reasons.join("; "))
    } else {
        None
    };

    CandidateCheckComparison {
        incumbent_report: incumbent_report.clone(),
        candidate_report,
        new_hard_continuity_blockers,
        new_hard_rating_violations,
        new_hard_antislop_blockers,
        remaining_hard_blockers,
        resolved_finding_count,
        disqualified,
        disqualification_reason,
    }
}

#[derive(Debug, Clone)]
struct ParagraphSlice<'a> {
    start_byte: usize,
    end_byte: usize,
    trimmed: &'a str,
}

fn split_paragraphs_with_spans(text: &str) -> Vec<ParagraphSlice<'_>> {
    let mut paragraphs = Vec::new();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut idx = 0;

    while idx < len {
        while idx < len && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= len {
            break;
        }
        let start = idx;
        while idx < len {
            if idx + 1 < len && bytes[idx] == b'\n' && bytes[idx + 1] == b'\n' {
                break;
            }
            if idx + 3 < len
                && bytes[idx] == b'\r'
                && bytes[idx + 1] == b'\n'
                && bytes[idx + 2] == b'\r'
                && bytes[idx + 3] == b'\n'
            {
                break;
            }
            idx += 1;
        }
        let mut end = idx;
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if end > start {
            paragraphs.push(ParagraphSlice {
                start_byte: start,
                end_byte: end,
                trimmed: &text[start..end],
            });
        }
    }
    paragraphs
}

/// Compute paragraph and UTF-8 byte edit-span summary between `incumbent_text` and
/// `candidate_text`. Flags changes affecting more than 40% (`> 4000` bps) of
/// paragraphs when no selected directive has `DirectiveScope::Scene`.
pub fn compute_edit_span_summary(
    incumbent_text: &str,
    candidate_text: &str,
    selected_directives: &[CritiqueDirective],
) -> EditSpanSummary {
    let inc_paras = split_paragraphs_with_spans(incumbent_text);
    let cand_paras = split_paragraphs_with_spans(candidate_text);
    let m = inc_paras.len();
    let n = cand_paras.len();

    // LCS dynamic programming table on exact trimmed paragraph equality.
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            if inc_paras[i].trimmed == cand_paras[j].trimmed {
                dp[i][j] = 1 + dp[i + 1][j + 1];
            } else {
                dp[i][j] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    let mut matched_inc = vec![false; m];
    let mut matched_cand = vec![false; n];
    let mut i = 0;
    let mut j = 0;
    let mut matched_pairs = Vec::new();
    while i < m && j < n {
        if inc_paras[i].trimmed == cand_paras[j].trimmed {
            matched_inc[i] = true;
            matched_cand[j] = true;
            matched_pairs.push((i, j));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }

    let unchanged_paragraph_count = matched_pairs.len();
    let changed_inc_paras = m.saturating_sub(unchanged_paragraph_count);
    let inserted_cand_paras = n.saturating_sub(unchanged_paragraph_count);
    let changed_paragraph_count = changed_inc_paras.max(inserted_cand_paras);

    let mut changed_spans = Vec::new();
    let mut prev_inc = 0usize;
    let mut prev_cand = 0usize;
    let mut anchors = matched_pairs;
    anchors.push((m, n));

    let mut changed_incumbent_bytes = 0usize;
    let mut changed_candidate_bytes = 0usize;

    for (anchor_inc, anchor_cand) in anchors {
        let inc_gap = anchor_inc.saturating_sub(prev_inc);
        let cand_gap = anchor_cand.saturating_sub(prev_cand);
        if inc_gap > 0 || cand_gap > 0 {
            let inc_start = if inc_gap > 0 || prev_inc < m {
                inc_paras[prev_inc].start_byte
            } else {
                incumbent_text.len()
            };
            let inc_end = if inc_gap > 0 {
                inc_paras[anchor_inc - 1].end_byte
            } else {
                inc_start
            };
            let cand_start = if cand_gap > 0 || prev_cand < n {
                cand_paras[prev_cand].start_byte
            } else {
                candidate_text.len()
            };
            let cand_end = if cand_gap > 0 {
                cand_paras[anchor_cand - 1].end_byte
            } else {
                cand_start
            };

            let change_kind = match (inc_gap > 0, cand_gap > 0) {
                (true, true) => SpanChangeKind::Modified,
                (false, true) => SpanChangeKind::Inserted,
                (true, false) => SpanChangeKind::Deleted,
                (false, false) => unreachable!(),
            };
            changed_incumbent_bytes =
                changed_incumbent_bytes.saturating_add(inc_end.saturating_sub(inc_start));
            changed_candidate_bytes =
                changed_candidate_bytes.saturating_add(cand_end.saturating_sub(cand_start));
            changed_spans.push(ChangedSpan {
                paragraph_index: prev_inc,
                incumbent_start_byte: inc_start,
                incumbent_end_byte: inc_end,
                candidate_start_byte: cand_start,
                candidate_end_byte: cand_end,
                change_kind,
            });
        }
        prev_inc = anchor_inc + 1;
        prev_cand = anchor_cand + 1;
    }

    let denom_paras = m.max(1);
    let changed_paragraph_ratio_bps =
        ((changed_paragraph_count as u64 * 10_000) / denom_paras as u64).min(10_000) as u32;
    let denom_bytes = incumbent_text.len().max(1);
    let changed_byte_ratio_bps =
        ((changed_incumbent_bytes as u64 * 10_000) / denom_bytes as u64).min(10_000) as u32;

    let scene_structural_rewrite_authorized = selected_directives
        .iter()
        .any(|d| d.scope == DirectiveScope::Scene);
    let broad_change_flagged =
        changed_paragraph_ratio_bps > 4_000 && !scene_structural_rewrite_authorized;

    EditSpanSummary {
        incumbent_paragraph_count: m,
        candidate_paragraph_count: n,
        changed_paragraph_count,
        unchanged_paragraph_count,
        changed_paragraph_ratio_bps,
        incumbent_byte_len: incumbent_text.len(),
        candidate_byte_len: candidate_text.len(),
        changed_incumbent_bytes,
        changed_candidate_bytes,
        changed_byte_ratio_bps,
        scene_structural_rewrite_authorized,
        broad_change_flagged,
        changed_spans,
    }
}

/// Assemble and token-verify the prior-draft revision `DraftMessages` (`18,000` input /
/// `3,000` output / `512` framing reserve).
///
/// The full prior draft (`incumbent_text`), `scene_card`, `hard_constraints`,
/// `knowledge_boundaries`, selected `directives`, and `strengths_to_preserve` are
/// mandatory and never truncated; if they exceed `RevisionBudget`, this fails with
/// `RevisionError::MandatoryOverflow`.
#[allow(clippy::too_many_arguments)]
pub fn assemble_revision_messages(
    packet: &DraftingPacketV1,
    incumbent_text: &str,
    incumbent_text_hash: &str,
    directives: &[CritiqueDirective],
    strengths_to_preserve: &[String],
    budget: &RevisionBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
    route_system: &str,
) -> Result<(DraftMessages, String, usize), RevisionError> {
    let t =
        tokenizer.ok_or_else(|| RevisionError::BudgetUnknown("no tokenizer configured".into()))?;
    if t.id().is_empty() || t.version().is_empty() || !t.supports_model(&budget.model_id) {
        return Err(RevisionError::BudgetUnknown(
            "tokenizer/model mismatch".into(),
        ));
    }
    let system = if route_system.is_empty() {
        REVISION_SYSTEM.to_string()
    } else {
        format!("{route_system}\n\n{REVISION_SYSTEM}")
    };

    let scene_card_json = serde_json::to_string(&packet.scene_card)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let constraints_json = serde_json::to_string(&packet.hard_constraints)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let knowledge_boundaries_json = serde_json::to_string(&packet.continuity.knowledge_boundaries)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let directives_json = serde_json::to_string(directives)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let strengths_json = serde_json::to_string(strengths_to_preserve)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;

    let render_revision_user = |continuity_sec: &str,
                                voice_sec: &str,
                                editorial_sec: &str|
     -> String {
        format!(
            "REVISION CONTRACT\n\
             incumbent_text_hash: {incumbent_text_hash}\n\
             incumbent_utf8_byte_len: {}\n\
             - Apply ONLY the selected actionable directives below.\n\
             - Preserve unaffected paragraphs verbatim whenever possible; changes affecting >40% of paragraphs without a scene-scoped directive are flagged.\n\
             - Preserve every item in STRENGTHS TO PRESERVE.\n\
             - Obey SCENE CARD, HARD CONSTRAINTS, and KNOWLEDGE BOUNDARIES.\n\
             - Return ONLY the revised UTF-8 scene prose (no markdown fences, no commentary, no JSON).\n\n\
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
             STRENGTHS TO PRESERVE\n\
             {strengths_json}\n\n\
             SELECTED ACTIONABLE DIRECTIVES\n\
             {directives_json}\n\n\
             FULL PRIOR DRAFT UTF-8 (NEVER TRUNCATED)\n\
             {incumbent_text}\n",
            incumbent_text.len()
        )
    };

    // 1. Verify mandatory context including the full prior draft fits within RevisionBudget.
    let mandatory_messages = DraftMessages {
        system: system.clone(),
        user: render_revision_user("", "", ""),
    };
    let mandatory_tokens = t
        .count_messages(&mandatory_messages)
        .map_err(RevisionError::BudgetUnknown)?;
    if !fits_budget(
        mandatory_tokens,
        budget.input_limit,
        budget.output_reserve,
        budget.framing_reserve,
        budget.context_window,
    ) {
        return Err(RevisionError::MandatoryOverflow(
            "mandatory revision context (full prior draft, scene card, hard constraints, knowledge boundaries, and directives) exceeds revision budget".into(),
        ));
    }

    // 2. Render compact packet sections under remaining revision token budget.
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
            .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
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
                .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
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
            let proposed = DraftMessages {
                system: system.clone(),
                user: render_revision_user(&prop_cont, &prop_voice, &prop_edit),
            };
            let tokens = t
                .count_messages(&proposed)
                .map_err(RevisionError::BudgetUnknown)?;
            if fits_budget(
                tokens,
                budget.input_limit,
                budget.output_reserve,
                budget.framing_reserve,
                budget.context_window,
            ) {
                seen_propositions.insert(prop_key);
                match tier_idx {
                    0 => continuity_sec = prop_cont,
                    1 => voice_sec = prop_voice,
                    _ => editorial_sec = prop_edit,
                }
            }
        }
    }

    let messages = DraftMessages {
        system,
        user: render_revision_user(&continuity_sec, &voice_sec, &editorial_sec),
    };
    let input_tokens = t
        .count_messages(&messages)
        .map_err(RevisionError::BudgetUnknown)?;
    if !fits_budget(
        input_tokens,
        budget.input_limit,
        budget.output_reserve,
        budget.framing_reserve,
        budget.context_window,
    ) {
        return Err(RevisionError::MandatoryOverflow(
            "revision prompt exceeds context window or input budget".into(),
        ));
    }
    let prompt_hash = sha256_hex(
        &serde_json::to_vec(&messages).map_err(|e| RevisionError::MalformedJson(e.to_string()))?,
    );
    Ok((messages, prompt_hash, input_tokens))
}

/// Build a complete `SinglePassJudgePayload` JSON template with all 7 rubric
/// dimensions so live judge models have the exact required JSON shape.
pub fn pairwise_judge_output_template() -> serde_json::Value {
    let mut rubric = serde_json::Map::new();
    for dim in JudgeRubricDimension::ALL {
        rubric.insert(
            dim.as_str().to_string(),
            serde_json::json!({
                "score": 4,
                "rationale": format!("Comparative assessment of {} citing Passage 1 and Passage 2.", dim.as_str())
            }),
        );
    }
    serde_json::json!({
        "rubric": rubric,
        "overall_preference": "passage_1",
        "deciding_factor": "Concrete craft and continuity reason why the chosen passage is stronger.",
        "strengths_passage_1": ["Specific strength of Passage 1"],
        "strengths_passage_2": ["Specific strength of Passage 2"]
    })
}

/// Deterministically select the randomized initial `(passage_1_arm, passage_2_arm)`
/// assignment for Pass 1 from `(packet_hash, incumbent_hash, candidate_hash, round, seed)`.
/// Pass 2 always uses the exact swapped order `(passage_2_arm, passage_1_arm)`.
pub fn select_initial_judge_order(
    packet_hash: &str,
    incumbent_hash: &str,
    candidate_hash: &str,
    round: u32,
    seed: Option<i64>,
) -> (DeblindedArm, DeblindedArm) {
    let seed_val = seed.unwrap_or(0);
    let material = format!("{packet_hash}:{incumbent_hash}:{candidate_hash}:{round}:{seed_val}");
    let digest = sha256_hex(material.as_bytes());
    let first_byte = u8::from_str_radix(&digest[..2], 16).unwrap_or(0);
    if first_byte & 1 == 0 {
        (DeblindedArm::Baseline, DeblindedArm::Candidate)
    } else {
        (DeblindedArm::Candidate, DeblindedArm::Baseline)
    }
}

/// Assemble and token-verify the blinded pairwise acceptance judge `DraftMessages`
/// (`16,000` input / `1,000` output / `512` framing reserve).
///
/// The judge sees:
/// - `PAIRWISE JUDGE CONTRACT`, JSON schema, and output template
/// - `SCENE CARD`, `HARD CONSTRAINTS`, `KNOWLEDGE BOUNDARIES (MANDATORY)`
/// - Budgeted `CONTINUITY CONTEXT` and `VOICE & STYLE CONTEXT` (including approved exemplars)
/// - Anonymous `PASSAGE 1` and `PASSAGE 2`
///
/// The judge NEVER sees critic scores, directives, revision labels (`incumbent`,
/// `candidate`, `baseline`, `revised`), or model identity.
pub fn assemble_pairwise_judge_messages(
    packet: &DraftingPacketV1,
    passage_1_text: &str,
    passage_2_text: &str,
    budget: &JudgeBudget,
    tokenizer: Option<&dyn DraftTokenizer>,
    route_system: &str,
) -> Result<(DraftMessages, String, usize), RevisionError> {
    let t =
        tokenizer.ok_or_else(|| RevisionError::BudgetUnknown("no tokenizer configured".into()))?;
    if t.id().is_empty() || t.version().is_empty() || !t.supports_model(&budget.model_id) {
        return Err(RevisionError::BudgetUnknown(
            "tokenizer/model mismatch".into(),
        ));
    }

    let system = if route_system.is_empty() {
        PAIRWISE_JUDGE_SYSTEM.to_string()
    } else {
        format!("{route_system}\n\n{PAIRWISE_JUDGE_SYSTEM}")
    };

    let scene_card_json = serde_json::to_string(&packet.scene_card)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let constraints_json = serde_json::to_string(&packet.hard_constraints)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let knowledge_boundaries_json = serde_json::to_string(&packet.continuity.knowledge_boundaries)
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let template_json = serde_json::to_string(&pairwise_judge_output_template())
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
    let schema_json = serde_json::to_string(&SinglePassJudgePayload::json_schema())
        .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;

    let render_judge_user = |continuity_sec: &str, voice_sec: &str| -> String {
        format!(
            "PAIRWISE JUDGE CONTRACT\n\
             schema_version: {PAIRWISE_JUDGE_SCHEMA_VERSION}\n\
             - Compare Passage 1 and Passage 2 blindly against the Scene Card, Hard Constraints, Knowledge Boundaries, Continuity Context, and Voice & Style Context.\n\
             - Top-level required fields: rubric (object with all 7 dimensions: scene_purpose_fulfilled, tension_turn, pov_discipline, voice_distinctiveness, dialogue, prose_freshness, continuity_with_brief), overall_preference (\"passage_1\" | \"passage_2\" | \"tie\"), deciding_factor (non-empty string), strengths_passage_1 (array of strings), strengths_passage_2 (array of strings).\n\
             - Each rubric.<dimension> object requires: score (integer 1..=5, or \"N/A\" ONLY for dialogue in a silent scene with no spoken lines), rationale (non-empty string citing concrete textual evidence).\n\
             - Any violation of HARD CONSTRAINTS or KNOWLEDGE BOUNDARIES is a severe continuity failure.\n\n\
             PAIRWISE_JUDGE_V1 OUTPUT TEMPLATE\n\
             {template_json}\n\n\
             PAIRWISE_JUDGE_V1 JSON SCHEMA\n\
             {schema_json}\n\n\
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
             PASSAGE 1\n\
             {passage_1_text}\n\n\
             PASSAGE 2\n\
             {passage_2_text}\n"
        )
    };

    // 1. Verify mandatory context + both full passages fit within JudgeBudget.
    let mandatory_messages = DraftMessages {
        system: system.clone(),
        user: render_judge_user("", ""),
    };
    let mandatory_tokens = t
        .count_messages(&mandatory_messages)
        .map_err(RevisionError::BudgetUnknown)?;
    if !fits_budget(
        mandatory_tokens,
        budget.input_limit,
        budget.output_reserve,
        budget.framing_reserve,
        budget.context_window,
    ) {
        return Err(RevisionError::MandatoryOverflow(
            "mandatory pairwise judge context (scene card, hard constraints, knowledge boundaries, schema, and both passages) exceeds judge budget".into(),
        ));
    }

    // 2. Render continuity and voice sections under remaining judge budget.
    let c = &packet.continuity;
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
            .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
        tier_voice.push(("approved prose example (not canon)", item));
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

    let mut seen_propositions: BTreeSet<(PacketSource, String)> = BTreeSet::new();
    for kb in &c.knowledge_boundaries {
        seen_propositions.insert((kb.source.clone(), kb.proposition_id.clone()));
    }

    let mut continuity_sec = String::new();
    let mut voice_sec = String::new();

    for (tier_idx, group) in [tier_continuity, tier_voice].into_iter().enumerate() {
        for (label, item) in group {
            let prop_key = (item.source.clone(), item.proposition_id.clone());
            if seen_propositions.contains(&prop_key) {
                continue;
            }
            let item_json = serde_json::to_string(&item)
                .map_err(|e| RevisionError::MalformedJson(e.to_string()))?;
            let line = format!("{label}: {item_json}\n");
            let (prop_cont, prop_voice) = if tier_idx == 0 {
                (format!("{continuity_sec}{line}"), voice_sec.clone())
            } else {
                (continuity_sec.clone(), format!("{voice_sec}{line}"))
            };
            let proposed = DraftMessages {
                system: system.clone(),
                user: render_judge_user(&prop_cont, &prop_voice),
            };
            let tokens = t
                .count_messages(&proposed)
                .map_err(RevisionError::BudgetUnknown)?;
            if fits_budget(
                tokens,
                budget.input_limit,
                budget.output_reserve,
                budget.framing_reserve,
                budget.context_window,
            ) {
                seen_propositions.insert(prop_key);
                if tier_idx == 0 {
                    continuity_sec = prop_cont;
                } else {
                    voice_sec = prop_voice;
                }
            }
        }
    }

    let messages = DraftMessages {
        system,
        user: render_judge_user(&continuity_sec, &voice_sec),
    };
    let input_tokens = t
        .count_messages(&messages)
        .map_err(RevisionError::BudgetUnknown)?;
    if !fits_budget(
        input_tokens,
        budget.input_limit,
        budget.output_reserve,
        budget.framing_reserve,
        budget.context_window,
    ) {
        return Err(RevisionError::MandatoryOverflow(
            "pairwise judge prompt exceeds context window or input budget".into(),
        ));
    }
    let prompt_hash = sha256_hex(
        &serde_json::to_vec(&messages).map_err(|e| RevisionError::MalformedJson(e.to_string()))?,
    );
    Ok((messages, prompt_hash, input_tokens))
}

/// Parse, validate, coerce (`dialogue: "N/A"` -> `3`, clamp `1..=5`), and deblind
/// a single pairwise judge pass using the exact `evals/prose_judge.py` rules.
pub fn parse_and_validate_single_judge_pass(
    raw_output: &str,
    order_index: u8,
    passage_1_arm: DeblindedArm,
    passage_2_arm: DeblindedArm,
    prompt_hash: String,
    model_call_id: Option<String>,
) -> Result<SinglePassJudgeRecord, RevisionError> {
    let stripped = strip_thinking_and_fences(raw_output);
    let payload: SinglePassJudgePayload = serde_json::from_str(stripped)
        .map_err(|e| RevisionError::InvalidJudgeOutput(format!("malformed JSON: {e}")))?;
    validate_single_judge_payload(
        payload,
        order_index,
        passage_1_arm,
        passage_2_arm,
        prompt_hash,
        model_call_id,
    )
}

pub fn validate_single_judge_payload(
    payload: SinglePassJudgePayload,
    order_index: u8,
    passage_1_arm: DeblindedArm,
    passage_2_arm: DeblindedArm,
    prompt_hash: String,
    model_call_id: Option<String>,
) -> Result<SinglePassJudgeRecord, RevisionError> {
    let mut normalized_rubric = BTreeMap::new();
    for dim in JudgeRubricDimension::ALL {
        let dim_name = dim.as_str();
        let entry = payload.rubric.get(dim_name).ok_or_else(|| {
            RevisionError::InvalidJudgeOutput(format!("missing rubric dimension: {dim_name}"))
        })?;
        let obj = entry.as_object().ok_or_else(|| {
            RevisionError::InvalidJudgeOutput(format!("rubric.{dim_name} must be an object"))
        })?;
        let raw_score = obj.get("score").ok_or_else(|| {
            RevisionError::InvalidJudgeOutput(format!("rubric.{dim_name}.score missing"))
        })?;
        let rationale = obj
            .get("rationale")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                RevisionError::InvalidJudgeOutput(format!(
                    "rubric.{dim_name}.rationale must be a non-empty string"
                ))
            })?
            .to_string();

        let (score, dialogue_na) = if dim == JudgeRubricDimension::Dialogue
            && raw_score
                .as_str()
                .is_some_and(|s| matches!(s.trim().to_uppercase().as_str(), "N/A" | "NA" | "NONE"))
        {
            (3u8, true)
        } else if let Some(n) = raw_score.as_i64() {
            (n.clamp(1, 5) as u8, false)
        } else if let Some(f) = raw_score.as_f64() {
            ((f.round() as i64).clamp(1, 5) as u8, false)
        } else {
            return Err(RevisionError::InvalidJudgeOutput(format!(
                "rubric.{dim_name}.score must be integer 1-5 (or N/A for dialogue), got {raw_score}"
            )));
        };

        normalized_rubric.insert(
            dim_name.to_string(),
            SinglePassDimensionScore {
                score,
                dialogue_na,
                rationale,
            },
        );
    }

    let raw_preference =
        PassageSlot::parse_normalized(&payload.overall_preference).ok_or_else(|| {
            RevisionError::InvalidJudgeOutput(format!(
                "overall_preference must be passage_1, passage_2, or tie, got {:?}",
                payload.overall_preference
            ))
        })?;
    let deciding_factor = payload.deciding_factor.trim().to_string();
    if deciding_factor.is_empty() {
        return Err(RevisionError::InvalidJudgeOutput(
            "deciding_factor must be a non-empty string".into(),
        ));
    }

    let deblinded_preference = match raw_preference {
        PassageSlot::Passage1 => passage_1_arm,
        PassageSlot::Passage2 => passage_2_arm,
        PassageSlot::Tie => DeblindedArm::Tie,
    };

    Ok(SinglePassJudgeRecord {
        order_index,
        passage_1_arm,
        passage_2_arm,
        prompt_hash,
        model_call_id,
        raw_preference,
        deblinded_preference,
        deciding_factor,
        strengths_passage_1: payload
            .strengths_passage_1
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        strengths_passage_2: payload
            .strengths_passage_2
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        rubric: normalized_rubric,
    })
}

fn mean_rubric_score(pass: &SinglePassJudgeRecord) -> f64 {
    if pass.rubric.is_empty() {
        return 3.0;
    }
    let sum: u32 = pass.rubric.values().map(|d| u32::from(d.score)).sum();
    f64::from(sum) / (pass.rubric.len() as f64)
}

/// Resolve two swapped-order judge passes (`pass_1` and `pass_2`) into a
/// `DoubleOrderJudgeEvaluation` and optional `RevisionRetentionReason`, matching
/// `evals/prose_judge.py` `_pair_outcome` and `docs/drafting-and-revision-design.md`:
/// - Both passes agree on `Candidate` -> `verdict = Candidate`, `accepted = true`.
/// - Both passes agree on `Baseline` -> `verdict = Baseline`, `accepted = false`, `JudgePreferredIncumbent`.
/// - Both passes `Tie` -> `verdict = Tie`, `accepted = false`, `JudgeTie`.
/// - Any order disagreement -> `verdict = Tie`, `accepted = false`, `JudgeOrderDisagreementTie`,
///   classifying `FirstPosition`, `SecondPosition`, or `TieFlip`.
pub fn resolve_double_order_judgment(
    pass_1: SinglePassJudgeRecord,
    pass_2: SinglePassJudgeRecord,
) -> (DoubleOrderJudgeEvaluation, Option<RevisionRetentionReason>) {
    let pref_a = pass_1.deblinded_preference;
    let pref_b = pass_2.deblinded_preference;
    let avg_1 = mean_rubric_score(&pass_1);
    let avg_2 = mean_rubric_score(&pass_2);
    let pair_avg = ((avg_1 + avg_2) / 2.0 * 1000.0).round() / 1000.0;

    if pref_a == pref_b {
        let (accepted, retention) = match pref_a {
            DeblindedArm::Candidate => (true, None),
            DeblindedArm::Baseline => (
                false,
                Some(RevisionRetentionReason::JudgePreferredIncumbent),
            ),
            DeblindedArm::Tie => (false, Some(RevisionRetentionReason::JudgeTie)),
        };
        return (
            DoubleOrderJudgeEvaluation {
                pass_1,
                pass_2,
                verdict: pref_a,
                accepted,
                position_bias: None,
                baseline_score_avg: pair_avg,
                candidate_score_avg: pair_avg,
            },
            retention,
        );
    }

    // Order disagreement -> always resolves to Tie and retains incumbent.
    let position_bias = match (pass_1.raw_preference, pass_2.raw_preference) {
        (PassageSlot::Passage1, PassageSlot::Passage1) => Some(PositionBiasKind::FirstPosition),
        (PassageSlot::Passage2, PassageSlot::Passage2) => Some(PositionBiasKind::SecondPosition),
        _ => Some(PositionBiasKind::TieFlip),
    };

    (
        DoubleOrderJudgeEvaluation {
            pass_1,
            pass_2,
            verdict: DeblindedArm::Tie,
            accepted: false,
            position_bias,
            baseline_score_avg: pair_avg,
            candidate_score_avg: pair_avg,
        },
        Some(RevisionRetentionReason::JudgeOrderDisagreementTie),
    )
}

/// Validate a `RevisionLoopArtifactV1` before saving or after loading from disk.
pub fn validate_revision_loop_artifact(artifact: &RevisionLoopArtifactV1) -> anyhow::Result<()> {
    anyhow::ensure!(
        artifact.schema_version == REVISION_LOOP_ARTIFACT_SCHEMA_VERSION,
        "unexpected revision loop artifact schema_version: {}",
        artifact.schema_version
    );
    anyhow::ensure!(
        !artifact.artifact_id.trim().is_empty()
            && !artifact.run_id.trim().is_empty()
            && !artifact.project_id.trim().is_empty()
            && !artifact.branch_id.trim().is_empty()
            && !artifact.packet_hash.trim().is_empty(),
        "revision loop artifact missing required identity fields"
    );
    anyhow::ensure!(
        (1..=HARD_MAX_REVISION_ROUNDS).contains(&artifact.max_rounds),
        "revision loop max_rounds ({}) outside 1..={HARD_MAX_REVISION_ROUNDS}",
        artifact.max_rounds
    );
    anyhow::ensure!(
        artifact.max_scene_calls <= HARD_MAX_SCENE_CALLS
            && artifact.max_scene_tokens <= HARD_MAX_SCENE_TOKENS,
        "revision loop call/token caps exceed scene hard caps"
    );
    anyhow::ensure!(
        sha256_hex(artifact.initial_text.as_bytes()) == artifact.initial_text_sha256,
        "initial_text_sha256 mismatch"
    );
    anyhow::ensure!(
        sha256_hex(artifact.accepted_text.as_bytes()) == artifact.accepted_text_sha256,
        "accepted_text_sha256 mismatch"
    );
    for t in &artifact.transitions {
        anyhow::ensure!(
            t.idempotency_key == t.key.idempotency_key(),
            "transition idempotency_key mismatch"
        );
    }
    Ok(())
}

/// Atomically persist a `RevisionLoopArtifactV1` to `path`.
pub fn save_revision_loop_artifact(
    path: &Path,
    artifact: &RevisionLoopArtifactV1,
) -> anyhow::Result<()> {
    validate_revision_loop_artifact(artifact)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(temp.as_file_mut(), artifact)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}

/// Load and validate a `RevisionLoopArtifactV1` from `path`.
pub fn load_revision_loop_artifact(path: &Path) -> anyhow::Result<RevisionLoopArtifactV1> {
    let artifact: RevisionLoopArtifactV1 = serde_json::from_reader(std::fs::File::open(path)?)?;
    validate_revision_loop_artifact(&artifact)?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestTokenizer;
    impl DraftTokenizer for TestTokenizer {
        fn id(&self) -> &str {
            "test-tok"
        }
        fn version(&self) -> &str {
            "v1"
        }
        fn supports_model(&self, model_id: &str) -> bool {
            model_id == "gemma4:31b"
        }
        fn count_text(&self, text: &str) -> Result<usize, String> {
            Ok(text.split_whitespace().count().max(1))
        }
        fn count_messages(&self, messages: &DraftMessages) -> Result<usize, String> {
            Ok(self.count_text(&messages.system)? + self.count_text(&messages.user)? + 8)
        }
    }

    fn sample_packet() -> DraftingPacketV1 {
        let source = PacketSource {
            branch_id: "branch-main".into(),
            source_id: "scene:1".into(),
            revision_hash: "rev-1".into(),
        };
        DraftingPacketV1 {
            schema_version: "drafting_packet_v1".into(),
            packet_id: "pkt-1".into(),
            project_id: "proj-1".into(),
            branch_id: "branch-main".into(),
            run_id: Some("run-1".into()),
            scene_id: Some("scene-1".into()),
            scene_revision: Some("rev-1".into()),
            canon_snapshot_hash: "canon-hash-1".into(),
            plan_revision: "plan-1".into(),
            source_versions: vec![source.clone()],
            scene_card: SceneCardV1 {
                source: source.clone(),
                pov: PacketPov {
                    character_id: "char-1".into(),
                    display_name: "Mara".into(),
                    person: NarrativePerson::Third,
                    tense: NarrativeTense::Past,
                    source: source.clone(),
                },
                goal: "Open the sealed sluice gate before the patrol returns.".into(),
                opposition: "Rusted chain and rising canal water.".into(),
                turn: "The key snaps inside the brass lock, revealing a false ward.".into(),
                intended_outcome: "Yes, but the iron counterweight drops and rings the alarm.".into(),
                value_shift: ValueShift {
                    axis: "safety".into(),
                    before: "concealed".into(),
                    after: "exposed".into(),
                },
                emotion: EmotionShift {
                    entry: "wary".into(),
                    exit: "resolute".into(),
                },
                chapter: PacketChapter {
                    book_number: 1,
                    number: 2,
                    scene_index: 1,
                    planned_scene_count: Some(3),
                    is_closer: false,
                },
                target_words: TargetWords {
                    min: 300,
                    preferred: 450,
                    max: 600,
                },
                location: PacketLocation {
                    id: "loc-1".into(),
                    name: "North Sluice".into(),
                },
                story_clock: "Day 2, 03:15".into(),
                rating: ContentRating::General,
                tone: Some("tense".into()),
                participants: vec![],
                required_beats: vec!["Key snaps in lock".into()],
                promises_due: vec![],
                prohibited_outcomes: vec!["the patrol surrenders peacefully".into()],
            },
            hard_constraints: vec![PacketConstraint {
                id: "lock-brass-key".into(),
                kind: ConstraintKind::AuthorLock,
                statement: "Mara must keep \"the notched brass key\" in her left glove.".into(),
                source: source.clone(),
                scope: "scene".into(),
            }],
            continuity: PacketContinuity {
                character_states: vec![PacketItem {
                    source: source.clone(),
                    proposition_id: "cs-1".into(),
                    text: "SENTINEL_CONT_STATE: Mara's left wrist is splinted.".into(),
                    relevance: 95,
                    recency: 10,
                }],
                knowledge_boundaries: vec![PacketItem {
                    source: source.clone(),
                    proposition_id: "kb-1".into(),
                    text: "SENTINEL_KB: Withheld: Mara must not know \"Vane is the warden's brother\".".into(),
                    relevance: 100,
                    recency: 10,
                }],
                facts: vec![PacketItem {
                    source: source.clone(),
                    proposition_id: "fact-1".into(),
                    text: "SENTINEL_FACT: The sluice wheel turns counter-clockwise.".into(),
                    relevance: 90,
                    recency: 9,
                }],
                prior_scene_tail: None,
                story_so_far: vec![],
                open_threads: vec![],
            },
            voice: PacketVoice {
                rules: vec![PacketItem {
                    source: source.clone(),
                    proposition_id: "vr-1".into(),
                    text: "SENTINEL_VOICE_RULE: Crisp tactile verbs; no abstract musing.".into(),
                    relevance: 85,
                    recency: 8,
                }],
                exemplars: vec![ApprovedExemplar {
                    item: PacketItem {
                        source: source.clone(),
                        proposition_id: "ex-1".into(),
                        text: "SENTINEL_EXEMPLAR: Cold iron bit through wet wool.".into(),
                        relevance: 80,
                        recency: 7,
                    },
                    speaker_id: Some("char-1".into()),
                    approved_revision_hash: "rev-1".into(),
                }],
            },
            editorial_memory: EditorialMemory::default(),
            craft_rubric: CraftRubric {
                version: "scene-craft-v1".into(),
                text: "Craft rubric.".into(),
            },
            research_evidence: vec![],
            assembly: None,
        }
    }

    #[test]
    fn shared_python_rust_verdict_fixtures_match_exactly() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../evals/prose_judge/shared_verdict_fixtures.json");
        let raw = std::fs::read_to_string(&fixture_path)
            .expect("read evals/prose_judge/shared_verdict_fixtures.json");
        let root: serde_json::Value = serde_json::from_str(&raw).expect("parse fixture JSON");

        // 1. Verify rubric dimensions list.
        let dims: Vec<String> = serde_json::from_value(root["rubric_dimensions"].clone()).unwrap();
        let rust_dims: Vec<String> = JudgeRubricDimension::ALL
            .iter()
            .map(|d| d.as_str().to_string())
            .collect();
        assert_eq!(dims, rust_dims);

        // 2. Verify preference normalization cases.
        for case in root["preference_normalization_cases"].as_array().unwrap() {
            let raw_token = case["raw"].as_str().unwrap();
            let expected = case["expected"].as_str();
            let actual = PassageSlot::parse_normalized(raw_token).map(|s| s.as_str());
            assert_eq!(
                actual, expected,
                "preference normalization mismatch for {raw_token:?}"
            );
        }

        // 3. Verify single-pass validation & coercion cases.
        for case in root["single_pass_validation_cases"].as_array().unwrap() {
            let case_id = case["id"].as_str().unwrap();
            let p1_arm: DeblindedArm =
                serde_json::from_value(case["passage_1_arm"].clone()).unwrap();
            let p2_arm: DeblindedArm =
                serde_json::from_value(case["passage_2_arm"].clone()).unwrap();
            let payload_str = serde_json::to_string(&case["payload"]).unwrap();
            let expect_valid = case["expect_valid"].as_bool().unwrap();

            let res = parse_and_validate_single_judge_pass(
                &payload_str,
                1,
                p1_arm,
                p2_arm,
                "prompt-hash-1".into(),
                Some("call-1".into()),
            );
            assert_eq!(
                res.is_ok(),
                expect_valid,
                "single-pass case {case_id} validity mismatch: {res:?}"
            );
            if let Ok(record) = res {
                let exp_raw = case["expected_raw_preference"].as_str().unwrap();
                let exp_deb = case["expected_deblinded_preference"].as_str().unwrap();
                assert_eq!(record.raw_preference.as_str(), exp_raw, "case {case_id}");
                assert_eq!(
                    record.deblinded_preference.as_str(),
                    exp_deb,
                    "case {case_id}"
                );
                if let Some(exp_diag) = case.get("expected_dialogue_score").and_then(|v| v.as_u64())
                {
                    assert_eq!(
                        u64::from(record.rubric["dialogue"].score),
                        exp_diag,
                        "case {case_id}"
                    );
                    assert_eq!(
                        record.rubric["dialogue"].dialogue_na,
                        case["expected_dialogue_na"].as_bool().unwrap(),
                        "case {case_id}"
                    );
                }
                if let Some(clamped) = case
                    .get("expected_clamped_scores")
                    .and_then(|v| v.as_object())
                {
                    for (dim_key, exp_val) in clamped {
                        assert_eq!(
                            u64::from(record.rubric[dim_key].score),
                            exp_val.as_u64().unwrap(),
                            "case {case_id} dim {dim_key}"
                        );
                    }
                }
            }
        }

        // 4. Verify double-order resolution cases.
        for case in root["double_order_resolution_cases"].as_array().unwrap() {
            let case_id = case["id"].as_str().unwrap();
            let make_pass = |order_idx: u8, node: &serde_json::Value| SinglePassJudgeRecord {
                order_index: order_idx,
                passage_1_arm: serde_json::from_value(node["passage_1_arm"].clone()).unwrap(),
                passage_2_arm: serde_json::from_value(node["passage_2_arm"].clone()).unwrap(),
                prompt_hash: format!("hash-{order_idx}"),
                model_call_id: Some(format!("call-{order_idx}")),
                raw_preference: serde_json::from_value(node["raw_preference"].clone()).unwrap(),
                deblinded_preference: serde_json::from_value(node["deblinded_preference"].clone())
                    .unwrap(),
                deciding_factor: "Test factor".into(),
                strengths_passage_1: vec![],
                strengths_passage_2: vec![],
                rubric: BTreeMap::new(),
            };
            let pass_1 = make_pass(1, &case["pass_1"]);
            let pass_2 = make_pass(2, &case["pass_2"]);
            let (eval, retention) = resolve_double_order_judgment(pass_1, pass_2);
            assert_eq!(
                eval.verdict.as_str(),
                case["expected_verdict"].as_str().unwrap(),
                "case {case_id} verdict"
            );
            assert_eq!(
                eval.accepted,
                case["expected_accepted"].as_bool().unwrap(),
                "case {case_id} accepted"
            );
            let exp_bias = case["expected_position_bias"].as_str();
            assert_eq!(
                eval.position_bias.map(|b| b.as_str()),
                exp_bias,
                "case {case_id} position_bias"
            );
            let exp_retention = case["expected_retention_reason"].as_str();
            assert_eq!(
                retention.map(|r| r.as_str()),
                exp_retention,
                "case {case_id} retention_reason"
            );
        }
    }

    #[test]
    fn deterministic_checks_disqualify_new_continuity_rating_or_antislop_blockers() {
        let packet = sample_packet();
        let incumbent_text = "Mara kept the notched brass key in her left glove as she gripped the wet iron chain.\n\nThe sluice wheel groaned counter-clockwise, and the key snapped inside the ward.";
        let incumbent_report =
            evaluate_deterministic_checks(&packet, incumbent_text, false, vec![]).unwrap();
        assert!(incumbent_report.hard_continuity_blockers.is_empty());
        assert!(incumbent_report.hard_antislop_blockers.is_empty());

        // Candidate 1: drops AuthorLock required literal ("the notched brass key") -> disqualified.
        let cand_missing_lock = "Mara dropped her glove and gripped the wet iron chain.\n\nThe sluice wheel groaned counter-clockwise.";
        let cand_lock_report =
            evaluate_deterministic_checks(&packet, cand_missing_lock, false, vec![]).unwrap();
        let cmp_lock = compare_candidate_checks(
            &packet,
            incumbent_text,
            &incumbent_report,
            cand_missing_lock,
            cand_lock_report,
        );
        assert!(cmp_lock.disqualified);
        assert!(!cmp_lock.new_hard_continuity_blockers.is_empty());

        // Candidate 2: leaks withheld knowledge ("Vane is the warden's brother") -> disqualified.
        let cand_leak = "Mara kept the notched brass key in her left glove. She knew Vane is the warden's brother.\n\nThe sluice wheel groaned.";
        let cand_leak_report =
            evaluate_deterministic_checks(&packet, cand_leak, false, vec![]).unwrap();
        let cmp_leak = compare_candidate_checks(
            &packet,
            incumbent_text,
            &incumbent_report,
            cand_leak,
            cand_leak_report,
        );
        assert!(cmp_leak.disqualified);
        assert!(
            cmp_leak
                .new_hard_continuity_blockers
                .iter()
                .any(|b| b.contains("knowledge_boundary_leak"))
        );

        // Candidate 3: introduces hard anti-slop blocker (emotion_cocktail / fishing_ending) -> disqualified.
        let cand_slop = "Mara kept the notched brass key in her left glove.\n\nShe felt a mix of relief and dread as the gate rose. She didn't know what tomorrow would bring.";
        let cand_slop_report =
            evaluate_deterministic_checks(&packet, cand_slop, false, vec![]).unwrap();
        let cmp_slop = compare_candidate_checks(
            &packet,
            incumbent_text,
            &incumbent_report,
            cand_slop,
            cand_slop_report,
        );
        assert!(cmp_slop.disqualified);
        assert!(!cmp_slop.new_hard_antislop_blockers.is_empty());
    }

    #[test]
    fn edit_span_summary_flags_over_forty_percent_paragraph_changes_without_scene_directive() {
        let incumbent = "Paragraph one stays intact.\n\nParagraph two stays intact.\n\nParagraph three stays intact.\n\nParagraph four changes.\n\nParagraph five stays intact.";
        let targeted_candidate = "Paragraph one stays intact.\n\nParagraph two stays intact.\n\nParagraph three stays intact.\n\nParagraph four is sharpened with concrete physical action.\n\nParagraph five stays intact.";
        let span_directive = CritiqueDirective {
            id: "dir-1".into(),
            dimension: CriticDimension::TensionTurn,
            severity: DirectiveSeverity::Major,
            priority: 1,
            anchor: CritiqueAnchor {
                scene_id: Some("scene-1".into()),
                base_text_hash: "h".into(),
                start_byte: 0,
                end_byte: 10,
                exact_quote: "Paragraph ".into(),
                context_before: None,
                context_after: None,
            },
            diagnosis: "Stalled beat".into(),
            requested_change: "Sharpen paragraph four".into(),
            preserve: vec![],
            success_test: "Beat lands".into(),
            source_finding_ids: vec![],
            scope: DirectiveScope::Span,
        };
        let targeted_summary = compute_edit_span_summary(
            incumbent,
            targeted_candidate,
            std::slice::from_ref(&span_directive),
        );
        assert_eq!(targeted_summary.incumbent_paragraph_count, 5);
        assert_eq!(targeted_summary.changed_paragraph_count, 1);
        assert_eq!(targeted_summary.changed_paragraph_ratio_bps, 2_000);
        assert!(!targeted_summary.broad_change_flagged);

        // Now change 3 of 5 paragraphs (60% > 40%) with only a Span directive -> flagged!
        let broad_candidate = "Paragraph one rewritten.\n\nParagraph two rewritten.\n\nParagraph three stays intact.\n\nParagraph four rewritten.\n\nParagraph five stays intact.";
        let broad_summary = compute_edit_span_summary(
            incumbent,
            broad_candidate,
            std::slice::from_ref(&span_directive),
        );
        assert_eq!(broad_summary.changed_paragraph_count, 3);
        assert_eq!(broad_summary.changed_paragraph_ratio_bps, 6_000);
        assert!(broad_summary.broad_change_flagged);

        // With an explicit DirectiveScope::Scene structural directive, broad_change_flagged is false.
        let mut scene_directive = span_directive;
        scene_directive.scope = DirectiveScope::Scene;
        let authorized_summary =
            compute_edit_span_summary(incumbent, broad_candidate, &[scene_directive]);
        assert!(authorized_summary.scene_structural_rewrite_authorized);
        assert!(!authorized_summary.broad_change_flagged);
    }

    #[test]
    fn pairwise_judge_prompt_includes_packet_context_and_schema_and_remains_strictly_blinded() {
        let packet = sample_packet();
        let budget = JudgeBudget::scene_default("gemma4:31b".into(), 32_768);
        let (messages, _, _) = assemble_pairwise_judge_messages(
            &packet,
            "Passage alpha text.",
            "Passage beta text.",
            &budget,
            Some(&TestTokenizer),
            "",
        )
        .unwrap();

        // Must include packet continuity, knowledge boundaries, voice rules, and approved exemplars.
        assert!(messages.user.contains("SENTINEL_KB"));
        assert!(messages.user.contains("SENTINEL_CONT_STATE"));
        assert!(messages.user.contains("SENTINEL_FACT"));
        assert!(messages.user.contains("SENTINEL_VOICE_RULE"));
        assert!(messages.user.contains("SENTINEL_EXEMPLAR"));
        assert!(messages.user.contains("PAIRWISE_JUDGE_V1 OUTPUT TEMPLATE"));
        assert!(messages.user.contains("PAIRWISE_JUDGE_V1 JSON SCHEMA"));

        // Must be strictly blinded: no revision/arm labels or critic directives.
        let lower = messages.user.to_lowercase();
        for forbidden in [
            "incumbent",
            "candidate",
            "baseline",
            "selected actionable directives",
        ] {
            assert!(
                !lower.contains(forbidden),
                "judge prompt leaked unblinded token {forbidden:?}"
            );
        }
    }
}
