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
        self.entries
            .iter()
            .any(|entry| entry.shelf_id == hit.shelf_id && entry.excerpt_normalized == excerpt)
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

/// Learn suppressions: hits on the agent draft whose excerpt still appears
/// in the operator edit were kept on purpose.
pub fn learn_suppressions_from_edit(
    pack: &ShelfPack,
    agent_draft: &str,
    operator_edit: &str,
) -> Vec<FalsePositiveSuppression> {
    let agent_report = scan(pack, &ScanInput::fiction(agent_draft));
    let operator_norm = normalize_excerpt(operator_edit);
    let mut out = Vec::new();
    for hit in agent_report.hits {
        let excerpt = normalize_excerpt(&hit.excerpt);
        if excerpt.is_empty() {
            continue;
        }
        if operator_norm.contains(&excerpt)
            && !out.iter().any(|existing: &FalsePositiveSuppression| {
                existing.shelf_id == hit.shelf_id && existing.excerpt_normalized == excerpt
            })
        {
            out.push(FalsePositiveSuppression {
                shelf_id: hit.shelf_id,
                excerpt_normalized: excerpt,
            });
        }
    }
    out
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
