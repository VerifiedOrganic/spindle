//! Learned false-positive suppressions (store + apply).
//!
//! When a human edit keeps a scanner-flagged excerpt, that `(shelf_id,
//! normalized excerpt)` is a suppression. Later scans skip matching hits
//! and recount `hard_count` / `soft_count`. Learning is a pure function so
//! adapters can persist via V0031 style-edit capture.

use super::pack::{Severity, ShelfPack};
use super::scan::{AntiSlopHit, ScanInput, scan};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One learned "this excerpt is voice, not a finding" record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FalsePositiveSuppression {
    pub shelf_id: String,
    pub excerpt_normalized: String,
}

/// In-memory store used at scan time. Adapters persist the same records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SuppressionStore {
    entries: Vec<FalsePositiveSuppression>,
}

impl SuppressionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_entries(entries: Vec<FalsePositiveSuppression>) -> Self {
        Self { entries }
    }

    pub fn iter(&self) -> impl Iterator<Item = &FalsePositiveSuppression> {
        self.entries.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn suppresses(&self, hit: &AntiSlopHit) -> bool {
        let excerpt = normalize_excerpt(&hit.excerpt);
        if excerpt.is_empty() {
            return false;
        }
        self.entries.iter().any(|entry| {
            entry.shelf_id == hit.shelf_id
                && entry.excerpt_normalized.split_whitespace().count() >= 4
                && (entry.excerpt_normalized == excerpt
                    || entry.excerpt_normalized.contains(&excerpt))
        })
    }

    /// Check whether `hit` inside `prose` matches a narrow sentence-scoped suppression.
    pub fn suppresses_in_prose(&self, hit: &AntiSlopHit, prose: &str) -> bool {
        let span = surrounding_sentence_span(prose, hit.byte_range.start, hit.byte_range.end);
        let span_norm = normalize_excerpt(span);
        if span_norm.is_empty() {
            return false;
        }
        self.entries.iter().any(|entry| {
            entry.shelf_id == hit.shelf_id
                && entry.excerpt_normalized.split_whitespace().count() >= 4
                && entry.excerpt_normalized == span_norm
        })
    }
}

/// Collapse whitespace and case so keep/edit matching is stable.
pub fn normalize_excerpt(excerpt: &str) -> String {
    excerpt
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Learn suppressions: hits on the agent draft whose surrounding sentence span
/// was kept intact inside a substantively edited local region.
///
/// A typo fix or an edit in another paragraph does not suppress untouched hits,
/// and suppressions are scoped to the surrounding sentence span rather than a
/// bare marker substring.
pub fn learn_suppressions_from_edit(
    pack: &ShelfPack,
    agent_draft: &str,
    operator_edit: &str,
) -> Vec<FalsePositiveSuppression> {
    if normalize_excerpt(agent_draft) == normalize_excerpt(operator_edit) {
        return Vec::new();
    }
    let agent_report = scan(pack, &ScanInput::fiction(agent_draft));
    let mut out = Vec::new();
    for hit in agent_report.hits {
        let span = surrounding_sentence_span(agent_draft, hit.byte_range.start, hit.byte_range.end);
        let span_norm = normalize_excerpt(span);
        if span_norm.split_whitespace().count() < 4 {
            continue;
        }
        if !was_kept_in_edited_region(
            agent_draft,
            operator_edit,
            hit.byte_range.start,
            hit.byte_range.end,
            &span_norm,
        ) {
            continue;
        }
        if !out.iter().any(|existing: &FalsePositiveSuppression| {
            existing.shelf_id == hit.shelf_id && existing.excerpt_normalized == span_norm
        }) {
            out.push(FalsePositiveSuppression {
                shelf_id: hit.shelf_id,
                excerpt_normalized: span_norm,
            });
        }
    }
    out
}

fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Extract the sentence/line span surrounding `start..end` in `prose`.
pub(crate) fn surrounding_sentence_span(prose: &str, start: usize, end: usize) -> &str {
    let (s, e) = surrounding_sentence_bounds(prose, start, end);
    prose[s..e].trim()
}

fn surrounding_sentence_bounds(prose: &str, start: usize, end: usize) -> (usize, usize) {
    let len = prose.len();
    let start = floor_char_boundary(prose, start.min(len));
    let end = ceil_char_boundary(prose, end.min(len));

    let mut s = start;
    while s > 0 {
        let prev = floor_char_boundary(prose, s - 1);
        let ch = prose[prev..s].chars().next().unwrap_or(' ');
        if ch == '\n' || ch == '.' || ch == '!' || ch == '?' {
            break;
        }
        s = prev;
    }

    let mut e = end;
    while e < len {
        let ch = prose[e..].chars().next().unwrap_or('\n');
        e += ch.len_utf8();
        if ch == '\n' || ch == '.' || ch == '!' || ch == '?' {
            break;
        }
    }

    (s, e)
}

fn enclosing_paragraph_bounds(prose: &str, start: usize, end: usize) -> (usize, usize) {
    let len = prose.len();
    let start = start.min(len);
    let end = end.min(len);
    let para_start = prose[..start].rfind("\n\n").map(|idx| idx + 2).unwrap_or(0);
    let para_end = prose[end..]
        .find("\n\n")
        .map(|rel| end + rel)
        .unwrap_or(len);
    (para_start, para_end)
}

fn was_kept_in_edited_region(
    agent_draft: &str,
    operator_edit: &str,
    hit_start: usize,
    hit_end: usize,
    span_norm: &str,
) -> bool {
    let (sent_start, sent_end) = surrounding_sentence_bounds(agent_draft, hit_start, hit_end);
    let (para_start, para_end) = enclosing_paragraph_bounds(agent_draft, sent_start, sent_end);
    let agent_para_norm = normalize_excerpt(&agent_draft[para_start..para_end]);

    // Locate a paragraph in `operator_edit` that still contains `span_norm` intact.
    for op_para in operator_edit.split("\n\n") {
        let op_para_norm = normalize_excerpt(op_para);
        let Some(pos) = op_para_norm.find(span_norm) else {
            continue;
        };
        // If the entire paragraph was left unchanged, the edit happened elsewhere.
        if op_para_norm == agent_para_norm {
            continue;
        }

        // Compare local neighbor words around the kept sentence inside the paragraph
        // (±250 bytes) so distant edits or 1-word typo fixes do not suppress the hit.
        let win_before_start =
            floor_char_boundary(agent_draft, sent_start.saturating_sub(250).max(para_start));
        let win_after_end = ceil_char_boundary(agent_draft, (sent_end + 250).min(para_end));
        let agent_neighbors = format!(
            "{} {}",
            &agent_draft[win_before_start..sent_start],
            &agent_draft[sent_end..win_after_end]
        );

        let op_before_start = floor_char_boundary(&op_para_norm, pos.saturating_sub(250));
        let op_after_start = pos + span_norm.len();
        let op_after_end = ceil_char_boundary(
            &op_para_norm,
            (op_after_start + 250).min(op_para_norm.len()),
        );
        let op_neighbors = format!(
            "{} {}",
            &op_para_norm[op_before_start..pos],
            &op_para_norm[op_after_start..op_after_end]
        );

        if substantive_word_change(&agent_neighbors, &op_neighbors) {
            return true;
        }
    }
    false
}

fn substantive_word_change(before: &str, after: &str) -> bool {
    let before_norm = normalize_excerpt(before);
    let after_norm = normalize_excerpt(after);
    if before_norm == after_norm {
        return false;
    }
    let before_words: Vec<&str> = before_norm.split_whitespace().collect();
    let after_words: Vec<&str> = after_norm.split_whitespace().collect();
    let mut unmatched_after = after_words.clone();
    let mut removed = 0usize;
    for word in &before_words {
        if let Some(idx) = unmatched_after.iter().position(|w| w == word) {
            unmatched_after.swap_remove(idx);
        } else {
            removed += 1;
        }
    }
    let added = unmatched_after.len();
    // A 1-word typo replacement has `removed == 1 && added == 1` (sum 2).
    // Require at least 4 changed word tokens in the local neighborhood.
    removed + added >= 4
}

pub fn recount_after_suppress(hits: &[AntiSlopHit]) -> (u32, u32) {
    let mut hard = 0_u32;
    let mut soft = 0_u32;
    for hit in hits {
        match hit.severity {
            Severity::Hard => hard += 1,
            Severity::Soft => soft += 1,
        }
    }
    (hard, soft)
}
