//! Layered derived memory (scene -> chapter -> arc -> book) and retrieval
//! selection contracts (P2 PR G).
//!
//! Summaries are derived memory, never canon authority. Selection filters by
//! cursor before choosing a layer, applies reader/secret visibility, and falls
//! back to eligible children when a parent spans a later position. The module is
//! pure so the adapter can persist/rebuild summaries without changing these
//! rules.

use crate::models::StoryPlacement;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const LAYERED_MEMORY_VERSION: &str = "layered-memory-v1";

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLayerV1 {
    Scene,
    Chapter,
    Arc,
    Book,
}

impl MemoryLayerV1 {
    pub fn rank(self) -> u8 {
        match self {
            Self::Scene => 1,
            Self::Chapter => 2,
            Self::Arc => 3,
            Self::Book => 4,
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTierV1 {
    Ratified,
    Provisional,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum MemorySummaryStatusV1 {
    Fresh,
    Stale { reason: String },
    Recomputing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryPositionV1 {
    pub book_number: i32,
    pub chapter_number: i32,
    pub scene_order: i32,
    pub within_scene_order: Option<u32>,
}

impl MemoryPositionV1 {
    pub fn new(book_number: i32, chapter_number: i32, scene_order: i32) -> Self {
        Self {
            book_number,
            chapter_number,
            scene_order,
            within_scene_order: None,
        }
    }

    pub fn from_placement(placement: &StoryPlacement) -> Option<Self> {
        if placement.book_number < 1 || placement.chapter_number < 1 {
            return None;
        }
        Some(Self {
            book_number: placement.book_number,
            chapter_number: placement.chapter_number,
            scene_order: placement.scene_order.unwrap_or(0),
            within_scene_order: None,
        })
    }

    pub fn key(&self) -> (i32, i32, i32, u32) {
        (
            self.book_number,
            self.chapter_number,
            self.scene_order,
            self.within_scene_order.unwrap_or(0),
        )
    }

    pub fn is_before(&self, other: &Self) -> bool {
        self.key() < other.key()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryCoverageV1 {
    pub start: MemoryPositionV1,
    pub end: MemoryPositionV1,
}

impl MemoryCoverageV1 {
    pub fn covers(&self, other: &Self) -> bool {
        self.start.key() <= other.start.key() && self.end.key() >= other.end.key()
    }

    pub fn ends_before(&self, cursor: &MemoryPositionV1) -> bool {
        self.end.key() < cursor.key()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemorySummaryV1 {
    pub summary_id: String,
    pub project_id: String,
    pub branch_id: String,
    pub layer: MemoryLayerV1,
    /// Stable natural key for the layer scope (scene id, `chapter:b:c`,
    /// `arc:<id>` or `arc:unassigned:<book>`, `book:<b>`).
    pub scope_key: String,
    /// `None` is the explicit unassigned arc group, never an invented arc.
    pub arc_id: Option<String>,
    pub coverage: MemoryCoverageV1,
    pub text: String,
    pub child_summary_ids: Vec<String>,
    pub entity_ids: Vec<String>,
    pub thread_ids: Vec<String>,
    pub source_revision_hashes: Vec<String>,
    pub derived_event_ids: Vec<String>,
    pub secret_fact_ids: Vec<String>,
    pub tier: MemoryTierV1,
    pub status: MemorySummaryStatusV1,
    pub reader_visible: bool,
    pub model: String,
    pub prompt_version: String,
    pub reducer_version: u32,
    pub updated_at_micros: i64,
}

impl MemorySummaryV1 {
    pub fn is_fresh(&self) -> bool {
        matches!(self.status, MemorySummaryStatusV1::Fresh)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayeredMemoryBuildReportV1 {
    pub project_id: String,
    pub branch_id: String,
    pub scene_count: usize,
    pub chapter_count: usize,
    pub arc_count: usize,
    pub book_count: usize,
    pub dependency_count: usize,
    pub stale_count: usize,
    pub pending_job_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayeredMemoryInvalidationReportV1 {
    pub plan: MemoryInvalidationPlanV1,
    pub pending_job_count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryVisibilityPolicyV1 {
    /// When true, summaries not marked reader-visible are omitted.
    pub reader_visible_only: bool,
    /// Secret facts the intended reader/POV may see.
    pub allowed_secret_fact_ids: BTreeSet<String>,
    /// Explicitly hidden secret facts. Always wins over the allow list.
    pub hidden_secret_fact_ids: BTreeSet<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOmissionReasonV1 {
    Stale,
    Recomputing,
    AtOrAfterCursor,
    SecretHidden,
    NotReaderVisible,
    CoveredByParent,
    Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryOmissionV1 {
    pub layer: MemoryLayerV1,
    pub reason: MemoryOmissionReasonV1,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LayeredMemorySelectionV1 {
    pub cursor: MemoryPositionV1,
    pub items: Vec<MemorySummaryV1>,
    pub omissions: Vec<MemoryOmissionV1>,
    pub secret_filtered_count: usize,
    pub stale_count: usize,
    pub after_cursor_count: usize,
}

impl LayeredMemorySelectionV1 {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

fn add_omission(
    omissions: &mut BTreeMap<(MemoryLayerV1, MemoryOmissionReasonV1), usize>,
    layer: MemoryLayerV1,
    reason: MemoryOmissionReasonV1,
) {
    *omissions.entry((layer, reason)).or_insert(0) += 1;
}

/// Cursor filtering happens before layer selection: a parent summary that
/// spans the cursor is never used; eligible children are chosen instead.
pub fn select_layered_memory(
    summaries: &[MemorySummaryV1],
    cursor: &MemoryPositionV1,
    policy: &MemoryVisibilityPolicyV1,
) -> LayeredMemorySelectionV1 {
    let mut candidates = summaries.to_vec();
    candidates.sort_by(|a, b| {
        b.layer
            .rank()
            .cmp(&a.layer.rank())
            .then_with(|| a.coverage.start.key().cmp(&b.coverage.start.key()))
            .then_with(|| a.summary_id.cmp(&b.summary_id))
    });

    let mut selected: Vec<MemorySummaryV1> = Vec::new();
    let mut omissions: BTreeMap<(MemoryLayerV1, MemoryOmissionReasonV1), usize> = BTreeMap::new();
    let mut secret_filtered_count = 0usize;
    let mut stale_count = 0usize;
    let mut after_cursor_count = 0usize;

    for summary in candidates {
        if !summary.is_fresh() {
            stale_count += 1;
            add_omission(
                &mut omissions,
                summary.layer,
                if matches!(summary.status, MemorySummaryStatusV1::Recomputing) {
                    MemoryOmissionReasonV1::Recomputing
                } else {
                    MemoryOmissionReasonV1::Stale
                },
            );
            continue;
        }
        if !summary.coverage.ends_before(cursor) {
            after_cursor_count += 1;
            add_omission(
                &mut omissions,
                summary.layer,
                MemoryOmissionReasonV1::AtOrAfterCursor,
            );
            continue;
        }
        if policy.reader_visible_only && !summary.reader_visible {
            add_omission(
                &mut omissions,
                summary.layer,
                MemoryOmissionReasonV1::NotReaderVisible,
            );
            continue;
        }
        let hidden = summary
            .secret_fact_ids
            .iter()
            .any(|fact_id| policy.hidden_secret_fact_ids.contains(fact_id));
        let visible = summary.secret_fact_ids.iter().all(|fact_id| {
            !policy.hidden_secret_fact_ids.contains(fact_id)
                && (policy.allowed_secret_fact_ids.contains(fact_id)
                    || policy.allowed_secret_fact_ids.is_empty())
        });
        if hidden || !visible {
            secret_filtered_count += 1;
            add_omission(
                &mut omissions,
                summary.layer,
                MemoryOmissionReasonV1::SecretHidden,
            );
            continue;
        }
        if selected.iter().any(|parent| {
            parent.layer.rank() > summary.layer.rank() && parent.coverage.covers(&summary.coverage)
        }) {
            add_omission(
                &mut omissions,
                summary.layer,
                MemoryOmissionReasonV1::CoveredByParent,
            );
            continue;
        }
        selected.push(summary);
    }

    selected.sort_by(|a, b| {
        a.layer
            .rank()
            .cmp(&b.layer.rank())
            .then_with(|| a.coverage.end.key().cmp(&b.coverage.end.key()))
            .then_with(|| a.summary_id.cmp(&b.summary_id))
    });

    LayeredMemorySelectionV1 {
        cursor: cursor.clone(),
        items: selected,
        omissions: omissions
            .into_iter()
            .map(|((layer, reason), count)| MemoryOmissionV1 {
                layer,
                reason,
                count,
            })
            .collect(),
        secret_filtered_count,
        stale_count,
        after_cursor_count,
    }
}

// ── context budget ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryBudgetPolicyV1 {
    pub total_tokens: usize,
    pub hard_constraint_tokens: usize,
    /// Per-layer caps applied after the hard-constraint reserve.
    pub layer_caps: BTreeMap<MemoryLayerV1, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryBudgetItemV1 {
    pub summary_id: String,
    pub layer: MemoryLayerV1,
    pub text: String,
    pub tokens: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryBudgetedContextV1 {
    pub items: Vec<MemoryBudgetItemV1>,
    pub included_sections: Vec<String>,
    pub truncated_sections: Vec<String>,
    pub omitted_by_layer: BTreeMap<MemoryLayerV1, usize>,
    pub estimated_tokens: usize,
    pub hard_constraint_tokens: usize,
    pub budget_exceeded: bool,
    pub hard_constraints_preserved: bool,
}

fn estimate_tokens(text: &str) -> usize {
    // Conservative byte/4 estimate, rounded up.
    text.len().div_ceil(4).max(1)
}

fn truncate_to_tokens(text: &str, max_tokens: usize) -> (&str, bool) {
    let max_bytes = max_tokens.saturating_mul(4);
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Recent detail comes first (scene -> chapter -> arc -> book), each layer is
/// capped, and hard constraints are reserved before any summary token.
pub fn build_memory_budgeted_context(
    selection: &LayeredMemorySelectionV1,
    policy: &MemoryBudgetPolicyV1,
) -> MemoryBudgetedContextV1 {
    let available = policy
        .total_tokens
        .saturating_sub(policy.hard_constraint_tokens);
    let mut remaining = available;
    let mut items = Vec::new();
    let mut included_sections = Vec::new();
    let mut truncated_sections = Vec::new();
    let mut omitted_by_layer: BTreeMap<MemoryLayerV1, usize> = BTreeMap::new();

    for layer in [
        MemoryLayerV1::Scene,
        MemoryLayerV1::Chapter,
        MemoryLayerV1::Arc,
        MemoryLayerV1::Book,
    ] {
        let cap = policy
            .layer_caps
            .get(&layer)
            .copied()
            .unwrap_or(remaining)
            .min(remaining);
        let mut layer_items: Vec<&MemorySummaryV1> = selection
            .items
            .iter()
            .filter(|summary| summary.layer == layer)
            .collect();
        layer_items.sort_by_key(|summary| std::cmp::Reverse(summary.coverage.end.key()));
        let mut layer_used = 0usize;
        for summary in layer_items {
            let layer_left = cap.saturating_sub(layer_used);
            if layer_left == 0 {
                *omitted_by_layer.entry(layer).or_insert(0) += 1;
                continue;
            }
            let (text, truncated) = truncate_to_tokens(&summary.text, layer_left);
            if text.is_empty() {
                *omitted_by_layer.entry(layer).or_insert(0) += 1;
                continue;
            }
            let used = estimate_tokens(text);
            layer_used += used;
            remaining = remaining.saturating_sub(used);
            if truncated {
                truncated_sections.push(format!("{:?}", layer).to_ascii_lowercase());
            }
            included_sections.push(format!("{:?}", layer).to_ascii_lowercase());
            items.push(MemoryBudgetItemV1 {
                summary_id: summary.summary_id.clone(),
                layer,
                text: text.to_string(),
                tokens: used,
                truncated,
            });
        }
    }

    let estimated_tokens =
        items.iter().map(|item| item.tokens).sum::<usize>() + policy.hard_constraint_tokens;
    MemoryBudgetedContextV1 {
        items,
        included_sections,
        truncated_sections,
        omitted_by_layer,
        estimated_tokens,
        hard_constraint_tokens: policy.hard_constraint_tokens,
        budget_exceeded: policy.hard_constraint_tokens >= policy.total_tokens,
        hard_constraints_preserved: policy.hard_constraint_tokens <= policy.total_tokens,
    }
}

// ── invalidation DAG ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryDependencyEdgeV1 {
    pub summary_id: String,
    pub depends_on_summary_id: String,
    pub source_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryRecomputeJobV1 {
    pub summary_id: String,
    pub reason: String,
    pub dedup_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MemoryInvalidationPlanV1 {
    pub root_summary_ids: Vec<String>,
    pub stale_summary_ids: Vec<String>,
    pub jobs: Vec<MemoryRecomputeJobV1>,
    pub affected_entity_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}

/// Mark the root summaries and every transitive ancestor stale, returning
/// deterministic job rows for lazy recomputation.
pub fn plan_memory_invalidation(
    root_summary_ids: &[String],
    summaries: &[MemorySummaryV1],
    edges: &[MemoryDependencyEdgeV1],
    reason: &str,
) -> MemoryInvalidationPlanV1 {
    let by_id: BTreeMap<&str, &MemorySummaryV1> = summaries
        .iter()
        .map(|summary| (summary.summary_id.as_str(), summary))
        .collect();
    let mut parents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for edge in edges {
        parents
            .entry(edge.depends_on_summary_id.as_str())
            .or_default()
            .push(edge.summary_id.as_str());
    }
    let mut stale = BTreeSet::new();
    let mut queue = VecDeque::new();
    for root in root_summary_ids {
        if by_id.contains_key(root.as_str()) && stale.insert(root.clone()) {
            queue.push_back(root.clone());
        }
    }
    while let Some(current) = queue.pop_front() {
        if let Some(children) = parents.get(current.as_str()) {
            for child in children {
                if stale.insert((*child).to_string()) {
                    queue.push_back((*child).to_string());
                }
            }
        }
    }
    let mut entity_ids = BTreeSet::new();
    let mut thread_ids = BTreeSet::new();
    for summary_id in &stale {
        if let Some(summary) = by_id.get(summary_id.as_str()) {
            entity_ids.extend(summary.entity_ids.iter().cloned());
            thread_ids.extend(summary.thread_ids.iter().cloned());
        }
    }
    let jobs = stale
        .iter()
        .map(|summary_id| MemoryRecomputeJobV1 {
            summary_id: summary_id.clone(),
            reason: reason.to_string(),
            dedup_key: format!("memory-recompute:{summary_id}:{reason}"),
        })
        .collect();
    MemoryInvalidationPlanV1 {
        root_summary_ids: root_summary_ids.to_vec(),
        stale_summary_ids: stale.into_iter().collect(),
        jobs,
        affected_entity_ids: entity_ids.into_iter().collect(),
        affected_thread_ids: thread_ids.into_iter().collect(),
    }
}

// ── open-thread retrieval ───────────────────────────────────────────────

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThreadKindV1 {
    Promise,
    Conflict,
    PlotLine,
    Other,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ThreadUrgencyV1 {
    Overdue,
    DueSoon,
    BlockingContradiction,
    SceneRelevant,
    Open,
    Resolved,
}

impl ThreadUrgencyV1 {
    fn rank(self) -> u8 {
        match self {
            Self::Overdue => 0,
            Self::DueSoon => 1,
            Self::BlockingContradiction => 2,
            Self::SceneRelevant => 3,
            Self::Open => 4,
            Self::Resolved => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadDigestV1 {
    pub thread_id: String,
    pub kind: ThreadKindV1,
    pub label: String,
    pub urgency: ThreadUrgencyV1,
    pub due_position: Option<MemoryPositionV1>,
    /// Planned deadlines are labeled separately from realized events.
    pub planned: bool,
    pub blocked: bool,
    pub scene_relevant: bool,
    /// Age in scene steps; larger is older.
    pub age_scene_count: u32,
    pub source_event_ids: Vec<String>,
}

impl ThreadDigestV1 {
    fn effective_rank(&self) -> u8 {
        if self.urgency == ThreadUrgencyV1::BlockingContradiction {
            return ThreadUrgencyV1::BlockingContradiction.rank();
        }
        if self.blocked && self.urgency.rank() >= ThreadUrgencyV1::BlockingContradiction.rank() {
            return ThreadUrgencyV1::BlockingContradiction.rank();
        }
        if self.urgency == ThreadUrgencyV1::Open && self.scene_relevant {
            return ThreadUrgencyV1::SceneRelevant.rank();
        }
        self.urgency.rank()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadRetrievalPolicyV1 {
    pub max_total: usize,
    pub page_offset: usize,
    /// Reserved capacity per class before the remaining slots are filled.
    pub class_quotas: BTreeMap<ThreadKindV1, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ThreadRetrievalOutputV1 {
    pub selected: Vec<ThreadDigestV1>,
    pub omitted_by_class: BTreeMap<ThreadKindV1, usize>,
    pub overflow_total: usize,
    pub next_offset: Option<usize>,
}

pub fn retrieve_threads(
    threads: &[ThreadDigestV1],
    policy: &ThreadRetrievalPolicyV1,
) -> ThreadRetrievalOutputV1 {
    let mut ordered = threads.to_vec();
    ordered.sort_by(|a, b| {
        a.effective_rank()
            .cmp(&b.effective_rank())
            .then_with(|| b.scene_relevant.cmp(&a.scene_relevant))
            .then_with(|| b.age_scene_count.cmp(&a.age_scene_count))
            .then_with(|| a.thread_id.cmp(&b.thread_id))
    });

    // Reserve per-class capacity from the full ordered list before filling the
    // remaining slots, so a lower-ranked class cannot be crowded out by filler.
    let mut selected_all = Vec::new();
    let mut used_ids = BTreeSet::new();
    for (kind, quota) in &policy.class_quotas {
        let mut used = 0usize;
        for thread in ordered.iter().filter(|thread| thread.kind == *kind) {
            if used >= *quota {
                break;
            }
            if used_ids.insert(thread.thread_id.clone()) {
                selected_all.push(thread.clone());
                used += 1;
            }
        }
    }
    for thread in &ordered {
        if selected_all.len() >= policy.max_total {
            break;
        }
        if used_ids.insert(thread.thread_id.clone()) {
            selected_all.push(thread.clone());
        }
    }
    selected_all.sort_by(|a, b| {
        a.effective_rank()
            .cmp(&b.effective_rank())
            .then_with(|| b.scene_relevant.cmp(&a.scene_relevant))
            .then_with(|| b.age_scene_count.cmp(&a.age_scene_count))
            .then_with(|| a.thread_id.cmp(&b.thread_id))
    });
    let page: Vec<ThreadDigestV1> = selected_all
        .iter()
        .skip(policy.page_offset)
        .take(policy.max_total)
        .cloned()
        .collect();
    let selected = page;
    let selected_ids: BTreeSet<&str> = selected
        .iter()
        .map(|thread| thread.thread_id.as_str())
        .collect();
    let mut omitted_by_class = BTreeMap::new();
    for thread in threads {
        if !selected_ids.contains(thread.thread_id.as_str()) {
            *omitted_by_class.entry(thread.kind).or_insert(0) += 1;
        }
    }
    let next_offset = policy.page_offset + selected.len();
    let overflow_total = threads.len().saturating_sub(selected.len());
    ThreadRetrievalOutputV1 {
        selected,
        omitted_by_class,
        overflow_total,
        next_offset: (next_offset < ordered.len()).then_some(next_offset),
    }
}

// ── entity state retrieval ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EntityRetrievalProviderV1 {
    /// Stable-id direct state retrieval; no embedding provider required.
    DirectState,
    Embedding,
    HashFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntityRetrievalV1 {
    pub subject_id: String,
    pub names: Vec<String>,
    pub aliases: Vec<String>,
    pub location: Option<String>,
    pub possessions: Vec<String>,
    pub injuries: Vec<String>,
    pub knowledge: Vec<String>,
    pub neighbors: Vec<String>,
    pub source_event_ids: Vec<String>,
    pub provider: EntityRetrievalProviderV1,
}

impl EntityRetrievalV1 {
    pub fn is_available_without_embedding(&self) -> bool {
        self.provider == EntityRetrievalProviderV1::DirectState
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
